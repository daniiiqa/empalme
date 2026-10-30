//! Los formatos de los módulos, con las mismas reglas que waybar para que
//! pasar un look sea copiar: `{usage}`, `{used:0.1f}`, `{usage:>2}`, `{icon}`,
//! y el marcado de pango que usan los looks (`<span foreground="#…">`, `<b>`).
//! Además de colores `#rrggbb`, el marcado entiende los del look: primary,
//! accent, fg, muted, red, green, y los de `~/.config/empalme/colors.json`
//! (que escribe LookBuild.py: color0…color15 y los nombres propios del look,
//! como rec o critical). Y los tamaños de pango (`size="small"`, `font='12'`,
//! `<small>`, `<sup>`): ver `Size`.

use discada_kit::look::Look;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::SystemTime;

static PALETTE: RwLock<Option<HashMap<String, Color>>> = RwLock::new(None);

fn palette_path() -> PathBuf {
    crate::bar::path().with_file_name("colors.json")
}

pub fn palette_stamp() -> Option<SystemTime> {
    std::fs::metadata(palette_path()).and_then(|m| m.modified()).ok()
}

/// (Re)leer los colores del look.
pub fn load_palette() {
    let map = std::fs::read_to_string(palette_path())
        .ok()
        .and_then(|t| serde_json::from_str::<HashMap<String, String>>(&t).ok())
        .map(|m| m.into_iter().filter_map(|(k, v)| Some((k, hex(&v)?))).collect());
    if let Ok(mut p) = PALETTE.write() {
        *p = map;
    }
}

/// Un color con nombre del look (`rec`, `critical`, `color4`…).
pub fn named(name: &str) -> Option<Color> {
    PALETTE.read().ok()?.as_ref()?.get(name).copied()
}

fn hex(v: &str) -> Option<Color> {
    let h = v.strip_prefix('#')?;
    let n = u32::from_str_radix(h.get(..6)?, 16).ok()?;
    Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

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

    /// `tooltip-format-<estado>`, o `tooltip-format`, o `default`; None si el
    /// look dice `tooltip = false`.
    pub fn tooltip(&self, state: &str, default: &str) -> Option<String> {
        if self.0["tooltip"].as_bool() == Some(false) {
            return None;
        }
        let own = if state.is_empty() { None } else { self.str(&format!("tooltip-format-{state}")) };
        Some(own.or_else(|| self.str("tooltip-format")).unwrap_or_else(|| default.to_string()))
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

/// Un color de la config del look: los del tema (primary, fg…), los de
/// colors.json (`color12`, `background`, `rec`…), `#rrggbb`, o
/// `alpha(<color>, 0.2)` como en el CSS de waybar (mezclado con el fondo de la
/// barra).
pub fn color(look: &Look, v: &str) -> Option<Color> {
    let v = v.trim();
    if let Some(inner) = v.strip_prefix("alpha(").and_then(|r| r.strip_suffix(')')) {
        let (c, a) = inner.rsplit_once(',')?;
        let a: f32 = a.trim().parse().ok()?;
        return hex(&mix(color(look, c)?, look.theme.base, a.clamp(0.0, 1.0)));
    }
    let t = look.theme;
    Some(match v {
        "primary" => t.primary,
        "accent" => t.accent,
        "fg" => t.fg,
        "muted" => t.muted,
        "red" => t.red,
        "green" => t.green,
        "white" => Color::Rgb(255, 255, 255),
        "black" => Color::Rgb(0, 0, 0),
        _ => return hex(v).or_else(|| named(v)),
    })
}

/// Un color con su opacidad, para lo que se pinta sobre el wallpaper (los
/// fondos y bordes de la barra): `alpha(<color>, 0.9)` deja pasar lo de atrás,
/// en vez de mezclarse con el fondo como en `color`. `transparent`: None.
pub fn rgba(look: &Look, v: &str) -> Option<(Color, f32)> {
    let v = v.trim();
    if v == "transparent" || v == "none" {
        return None;
    }
    if let Some(inner) = v.strip_prefix("alpha(").and_then(|r| r.strip_suffix(')')) {
        let (c, a) = inner.rsplit_once(',')?;
        let (c, a0) = rgba(look, c)?;
        return Some((c, a0 * a.trim().parse::<f32>().ok()?.clamp(0.0, 1.0)));
    }
    color(look, v).map(|c| (c, 1.0))
}

/// `a` sobre `b` con opacidad `k` (0 a 1), en `#rrggbb` para el marcado: así
/// se hace la opacidad del CSS de waybar (el pulso del grabador).
pub fn mix(a: Color, b: Color, k: f32) -> String {
    let rgb = |c: Color| match c {
        Color::Rgb(r, g, b) => (r as f32, g as f32, b as f32),
        _ => (128.0, 128.0, 128.0),
    };
    let (a, b) = (rgb(a), rgb(b));
    let m = |x: f32, y: f32| (x * k + y * (1.0 - k)).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// Un color como `#rrggbb` (para guardarlo donde va un nombre de color).
pub fn hexa(c: Color) -> String {
    mix(c, c, 1.0)
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

/// El texto de un marcado, sin etiquetas (para medirlo).
pub fn plain(markup: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in markup.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    unescape(&out)
}

/// El tamaño de un pedazo del marcado. Va escondido en el color de
/// subrayado de sus celdas (la barra no lo usa): `Rgb(1, %, rise)` relativo
/// al del módulo, `Rgb(2, px, rise)` absoluto; rise 1 arriba (`<sup>`), 2
/// abajo (`<sub>`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Size {
    /// None: el del módulo
    px: Option<f32>,
    k: f32,
    pub rise: u8,
}

impl Size {
    const NORMAL: Size = Size { px: None, k: 1.0, rise: 0 };

    fn scaled(self, k: f32) -> Size {
        Size { k: self.k * k, ..self }
    }

    fn encode(self) -> Option<Color> {
        if self == Size::NORMAL {
            return None;
        }
        Some(match self.px {
            Some(px) => Color::Rgb(2, (px * self.k).round().clamp(1.0, 255.0) as u8, self.rise),
            None => Color::Rgb(1, (self.k * 100.0).round().clamp(1.0, 255.0) as u8, self.rise),
        })
    }

    /// El de una celda (su color de subrayado).
    pub fn of(c: Color) -> Size {
        match c {
            Color::Rgb(1, k, rise) => Size { px: None, k: k as f32 / 100.0, rise },
            Color::Rgb(2, px, rise) => Size { px: Some(px as f32), k: 1.0, rise },
            _ => Size::NORMAL,
        }
    }

    /// En pixeles, para un módulo de letra `base`; None si es la misma.
    pub fn px(self, base: f32) -> Option<f32> {
        let px = (self.px.unwrap_or(base) * self.k).round().max(6.0);
        (px != base.round()).then_some(px)
    }
}

/// `size` de pango: `small`, `x-large`, `smaller`, `120%`, `12pt`, o en
/// milésimas de punto (`10240`). Un tamaño absoluto en puntos va a pixeles.
fn pango_size(cur: Size, v: &str) -> Size {
    let v = v.trim();
    let k = |n: i32| cur.scaled(1.2f32.powi(n));
    match v {
        "xx-small" => Size { px: None, k: 1.2f32.powi(-3), ..cur },
        "x-small" => Size { px: None, k: 1.2f32.powi(-2), ..cur },
        "small" => Size { px: None, k: 1.0 / 1.2, ..cur },
        "medium" => Size { px: None, k: 1.0, ..cur },
        "large" => Size { px: None, k: 1.2, ..cur },
        "x-large" => Size { px: None, k: 1.2f32.powi(2), ..cur },
        "xx-large" => Size { px: None, k: 1.2f32.powi(3), ..cur },
        "smaller" => k(-1),
        "larger" => k(1),
        _ => {
            if let Some(p) = v.strip_suffix('%').and_then(|p| p.parse::<f32>().ok()) {
                cur.scaled(p / 100.0)
            } else if let Some(pt) = v.strip_suffix("pt").and_then(|p| p.trim().parse::<f32>().ok()) {
                Size { px: Some(pt * 4.0 / 3.0), k: 1.0, ..cur }
            } else if let Ok(n) = v.parse::<f32>() {
                Size { px: Some(n / 1024.0 * 4.0 / 3.0), k: 1.0, ..cur }
            } else {
                cur
            }
        }
    }
}

/// `font` de pango (`12`, `Sans Bold 12`, `12px`): solo el tamaño.
fn pango_font(cur: Size, v: &str) -> Size {
    let Some(last) = v.split_whitespace().last() else { return cur };
    if let Some(px) = last.strip_suffix("px").and_then(|p| p.parse::<f32>().ok()) {
        return Size { px: Some(px), k: 1.0, ..cur };
    }
    match last.parse::<f32>() {
        Ok(pt) => Size { px: Some(pt * 4.0 / 3.0), k: 1.0, ..cur },
        Err(_) => cur,
    }
}

/// El marcado ya relleno, como renglón de la barra con `base` de estilo.
pub fn line(markup: &str, base: Style, look: &Look) -> Line<'static> {
    let mut stack = vec![(base, Size::NORMAL)];
    let mut spans = Vec::new();
    let mut rest = markup;
    loop {
        let i = rest.find('<').unwrap_or(rest.len());
        if i > 0 {
            spans.push(Span::styled(unescape(&rest[..i]), stack.last().unwrap().0));
        }
        if i == rest.len() {
            break;
        }
        let Some(j) = rest[i..].find('>') else {
            spans.push(Span::styled(unescape(&rest[i..]), stack.last().unwrap().0));
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
        let (mut st, mut size) = *stack.last().unwrap();
        let (name, body) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        match name {
            "b" => st = st.add_modifier(Modifier::BOLD),
            "i" => st = st.add_modifier(Modifier::ITALIC),
            "u" => st = st.add_modifier(Modifier::UNDERLINED),
            "small" => size = size.scaled(1.0 / 1.2),
            "big" => size = size.scaled(1.2),
            // como pango 1.50: más chico y arriba (o abajo)
            "sup" => size = Size { rise: 1, ..size.scaled(0.7) },
            "sub" => size = Size { rise: 2, ..size.scaled(0.7) },
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
                "size" | "font_size" => size = pango_size(size, v),
                "font" | "font_desc" => size = pango_font(size, v),
                _ => {}
            }
        }
        st.underline_color = size.encode().or(stack[0].0.underline_color);
        stack.push((st, size));
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

    #[test]
    fn tamanos() {
        let look = Look::load("empalme");
        let size = |m: &str, k: usize| Size::of(line(m, Style::default(), &look).spans[k].style.underline_color.unwrap_or(Color::Reset));
        assert_eq!(size("<span size='small'>cpu</span> 3%", 0).px(13.0), Some(11.0));
        assert_eq!(size("<span size='small'>cpu</span> 3%", 1).px(13.0), None);
        assert_eq!(size("<span font='12'>x</span>", 0).px(13.0), Some(16.0));
        assert_eq!(size("<span font='8'>x</span>", 0).px(20.0), Some(11.0));
        let sup = size("a<span foreground='red'><sup>b</sup></span>", 1);
        assert_eq!((sup.px(20.0), sup.rise), (Some(14.0), 1));
    }
}
