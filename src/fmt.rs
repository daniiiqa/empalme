//! Los formatos de los módulos, con las mismas reglas que waybar para que
//! pasar un look sea copiar: `{usage}`, `{used:0.1f}`, `{usage:>2}`, `{icon}`,
//! y el marcado de pango que usan los looks (`<span foreground="#…">`, `<b>`).
//! Además de colores `#rrggbb`, el marcado entiende los del look: primary,
//! accent, fg, muted, red, green.

use discada_kit::look::Look;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

/// Un valor para una plantilla.
pub enum Val {
    Num(f64),
    Text(String),
}

impl From<f64> for Val {
    fn from(v: f64) -> Val {
        Val::Num(v)
    }
}

impl From<String> for Val {
    fn from(v: String) -> Val {
        Val::Text(v)
    }
}

impl From<&str> for Val {
    fn from(v: &str) -> Val {
        Val::Text(v.to_string())
    }
}

/// Lo que dice el `[bar.modules.<nombre>]` del look de un módulo.
#[derive(Clone, Default)]
pub struct Conf(pub Value);

impl Conf {
    pub fn str(&self, key: &str) -> Option<String> {
        self.0[key].as_str().map(String::from)
    }

    /// `format-<estado>`, o `format` si el look no lo distingue.
    pub fn format(&self, state: &str) -> Option<String> {
        let own = if state.is_empty() { None } else { self.str(&format!("format-{state}")) };
        own.or_else(|| self.str("format"))
    }

    pub fn num(&self, key: &str) -> Option<f64> {
        self.0[key].as_f64()
    }

    /// `format-icons`: una lista, o un objeto con `default` (y otras listas o
    /// íconos por nombre, como los de audífonos de pulseaudio).
    pub fn icons(&self, key: &str) -> Vec<String> {
        let v = &self.0["format-icons"];
        let v = if v.is_object() { &v[key] } else if key == "default" { v } else { &Value::Null };
        match v {
            Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
            Value::String(s) => vec![s.clone()],
            _ => Vec::new(),
        }
    }
}

/// El ícono que toca para un por ciento (0 a 100) de una lista.
pub fn pick(icons: &[String], pct: f64) -> String {
    if icons.is_empty() {
        return String::new();
    }
    let i = ((pct.clamp(0.0, 100.0) / 100.0) * icons.len() as f64) as usize;
    icons[i.min(icons.len() - 1)].clone()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Un número con la especificación de waybar (de fmt de C++): `0.1f`, `>2`,
/// `<3`, `2`; sin ella, entero si lo es y si no con un decimal.
fn number(v: f64, spec: &str) -> String {
    let (align, rest) = match spec.chars().next() {
        Some(c @ ('<' | '>' | '^')) => (c, &spec[1..]),
        _ => ('>', spec),
    };
    let rest = rest.trim_end_matches('f');
    let (width, prec) = match rest.split_once('.') {
        Some((w, p)) => (w.parse().unwrap_or(0), p.parse::<usize>().ok()),
        None => (rest.parse().unwrap_or(0), None),
    };
    let s = match prec {
        Some(p) => format!("{v:.p$}"),
        None if spec.is_empty() && v.fract().abs() > 1e-9 => format!("{v:.1}"),
        None => format!("{}", v.round() as i64),
    };
    match align {
        '<' => format!("{s:<width$}"),
        '^' => format!("{s:^width$}"),
        _ => format!("{s:>width$}"),
    }
}

/// Rellena `{nombre}` y `{nombre:spec}`; lo que no conoce lo deja como está
/// (el `{:%H:%M}` del reloj, por ejemplo). Los textos van escapados para el
/// marcado.
pub fn fill(tpl: &str, vals: &[(&str, Val)]) -> String {
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let inner = &rest[i + 1..i + j];
        let (name, spec) = inner.split_once(':').unwrap_or((inner, ""));
        match vals.iter().find(|(n, _)| *n == name) {
            Some((_, Val::Num(v))) => out.push_str(&number(*v, spec)),
            Some((_, Val::Text(t))) => out.push_str(&escape(t)),
            None => out.push_str(&rest[i..=i + j]),
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

fn color(look: &Look, v: &str) -> Option<Color> {
    let t = look.theme;
    Some(match v {
        "primary" => t.primary,
        "accent" => t.accent,
        "fg" => t.fg,
        "muted" => t.muted,
        "red" => t.red,
        "green" => t.green,
        _ => {
            let h = v.strip_prefix('#')?;
            let n = u32::from_str_radix(h.get(..6)?, 16).ok()?;
            Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8)
        }
    })
}

/// `name="valor"` de una etiqueta.
fn attrs(tag: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = tag;
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let q = after.chars().next().unwrap_or('"');
        let Some(end) = after[1..].find(q) else { break };
        out.push((name, &after[1..1 + end]));
        rest = &after[end + 2..];
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// El marcado ya relleno, como renglón de la barra con `base` de estilo.
pub fn line(markup: &str, base: Style, look: &Look) -> Line<'static> {
    let mut stack = vec![base];
    let mut spans = Vec::new();
    let mut rest = markup;
    loop {
        let i = rest.find('<').unwrap_or(rest.len());
        if i > 0 {
            spans.push(Span::styled(unescape(&rest[..i]), *stack.last().unwrap()));
        }
        if i == rest.len() {
            break;
        }
        let Some(j) = rest[i..].find('>') else {
            spans.push(Span::styled(unescape(&rest[i..]), *stack.last().unwrap()));
            break;
        };
        let tag = &rest[i + 1..i + j];
        rest = &rest[i + j + 1..];
        if tag.starts_with('/') {
            if stack.len() > 1 {
                stack.pop();
            }
            continue;
        }
        let mut st = *stack.last().unwrap();
        let (name, body) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        match name {
            "b" => st = st.add_modifier(Modifier::BOLD),
            "i" => st = st.add_modifier(Modifier::ITALIC),
            "u" => st = st.add_modifier(Modifier::UNDERLINED),
            _ => {}
        }
        for (k, v) in attrs(body) {
            match k {
                "foreground" | "fgcolor" | "color" => {
                    if let Some(c) = color(look, v) {
                        st = st.fg(c);
                    }
                }
                "background" | "bgcolor" => {
                    if let Some(c) = color(look, v) {
                        st = st.bg(c);
                    }
                }
                "weight" | "font_weight" if v == "bold" => st = st.add_modifier(Modifier::BOLD),
                _ => {}
            }
        }
        stack.push(st);
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plantillas() {
        let v = [("used", Val::Num(3.456)), ("usage", Val::Num(7.0)), ("ifname", Val::from("a<b"))];
        assert_eq!(fill("mem {used:0.1f}g", &v), "mem 3.5g");
        assert_eq!(fill("cpu {usage:>2}%", &v), "cpu  7%");
        assert_eq!(fill("{usage}% {ifname}", &v), "7% a&lt;b");
        assert_eq!(fill("[ {:%H:%M} ] {otro}", &v), "[ {:%H:%M} ] {otro}");
        assert_eq!(fill("{used}", &v), "3.5");
    }

    #[test]
    fn iconos() {
        let i: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(pick(&i, 0.0), "a");
        assert_eq!(pick(&i, 49.0), "b");
        assert_eq!(pick(&i, 100.0), "d");
    }
}
