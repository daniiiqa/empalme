//! Cómo se ve la barra y cada módulo, como el CSS de waybar de cada look:
//!   [bar.box]   la barra entera (window#waybar): bg, border, border-width,
//!               border-sides (all | bottom | top), dashed, radius,
//!               top-gradient [colores] y top-width (el border-image de
//!               ofrenda: una tira de colores arriba, de lado a lado)
//!   [bar.pill]  cada módulo (la regla común de sus #clock, #network…): fg,
//!               bg, border, border-width, radius, margin [arriba, derecha,
//!               abajo, izquierda], padding (a los lados), shadow [x, y,
//!               color], bold, button-radius (los fondos de sus botones),
//!               text-shadow (como en CSS; `<clase>-text-shadow` para un
//!               estado: active, recording…; ver glow.rs)
//!   [bar.modules.<nombre>] lo mismo, solo para ese módulo (encima de pill)
//! Los colores como en los formatos (`color12`, `acc`, `alpha(bg0, 0.9)`),
//! pixeles lógicos. Sin [bar.box] ni [bar.pill], `style = "solid"` es la
//! barra del color de fondo del look e `"islands"` una isla por módulo.

use crate::fmt;
use crate::glow::{self, Shadow};
use disco::look::Look;
use disco::paint::{rounded, stroke_rounded};
use ratatui::style::Color;
use serde_json::Value;
use tiny_skia::Pixmap;

/// Un fondo o borde ya resuelto: color y opacidad.
type Paint = Option<(Color, f32)>;

fn f(v: &Value, k: &str) -> Option<f32> {
    v[k].as_f64().map(|x| x as f32)
}

fn s(v: &Value, k: &str) -> Option<String> {
    v[k].as_str().map(String::from)
}

#[derive(Clone, Default)]
pub struct BarBox {
    pub bg: Option<String>,
    pub border: Option<String>,
    pub width: f32,
    /// "all", "bottom", "top"
    pub sides: String,
    pub dashed: bool,
    pub radius: f32,
    /// la tira de arriba en degradado (de izquierda a derecha) y su alto
    pub gradient: Vec<String>,
    pub gradient_width: f32,
}

impl BarBox {
    pub fn new(v: &Value, solid: bool, radius: f32) -> BarBox {
        if v.is_null() {
            return BarBox { bg: solid.then(|| "base".into()), radius, sides: "all".into(), ..Default::default() };
        }
        BarBox {
            bg: s(v, "bg"),
            border: s(v, "border"),
            width: f(v, "border-width").unwrap_or(1.0),
            sides: s(v, "border-sides").unwrap_or_else(|| "all".into()),
            dashed: v["dashed"].as_bool().unwrap_or(false),
            radius: f(v, "radius").unwrap_or(radius),
            gradient: v["top-gradient"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
            gradient_width: f(v, "top-width").unwrap_or(3.0),
        }
    }

    /// La barra: su fondo y su borde, en el cuadro entero (pixeles del monitor).
    pub fn paint(&self, pix: &mut Pixmap, look: &Look, sf: f32, alpha: f32) {
        let (w, mut h) = (pix.width() as f32, pix.height() as f32);
        let r = self.radius * sf;
        // con la tira de arriba, la caja empieza debajo de ella
        let top = if self.gradient.len() >= 2 { (self.gradient_width * sf).round() } else { 0.0 };
        h -= top;
        if let Some((c, a)) = self.bg.as_deref().and_then(|v| resolve(look, v)) {
            rounded(pix, 0.0, top, w, h, r, c, a * alpha);
        }
        if top > 0.0 {
            self.strip(pix, look, w, top, alpha);
        }
        let Some((c, a)) = self.border.as_deref().and_then(|v| resolve(look, v)) else { return };
        let bw = (self.width * sf).max(1.0);
        let dash = self.dashed.then_some(3.0 * sf);
        match self.sides.as_str() {
            "bottom" | "top" => {
                let y = if self.sides == "bottom" { top + h - bw } else { top };
                line(pix, y, w, bw, c, a, dash);
            }
            _ => stroke_rounded(pix, 0.0, top, w, h, r, bw, c, a, dash),
        }
    }

    /// La tira de colores de arriba, repartidos parejo de izquierda a derecha.
    fn strip(&self, pix: &mut Pixmap, look: &Look, w: f32, h: f32, alpha: f32) {
        let n = self.gradient.len();
        let stops: Vec<tiny_skia::GradientStop> = self
            .gradient
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                let (c, a) = resolve(look, v)?;
                let Color::Rgb(r, g, b) = c else { return None };
                let col = tiny_skia::Color::from_rgba8(r, g, b, (a * alpha * 255.0).round() as u8);
                Some(tiny_skia::GradientStop::new(i as f32 / (n - 1) as f32, col))
            })
            .collect();
        let shader = tiny_skia::LinearGradient::new(
            tiny_skia::Point::from_xy(0.0, 0.0),
            tiny_skia::Point::from_xy(w, 0.0),
            stops,
            tiny_skia::SpreadMode::Pad,
            tiny_skia::Transform::identity(),
        );
        let (Some(shader), Some(rect)) = (shader, tiny_skia::Rect::from_xywh(0.0, 0.0, w, h)) else { return };
        let paint = tiny_skia::Paint { shader, anti_alias: false, ..Default::default() };
        pix.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
    }
}

/// Una línea de lado a lado (el borde de abajo de retro, punteado).
fn line(pix: &mut Pixmap, y: f32, w: f32, bw: f32, c: Color, a: f32, dash: Option<f32>) {
    let step = dash.unwrap_or(w);
    let mut x = 0.0;
    while x < w {
        rounded(pix, x, y, step.min(w - x), bw, 0.0, c, a);
        x += step * 2.0;
        if dash.is_none() {
            break;
        }
    }
}

/// `base` es el fondo del look; lo demás, como en fmt.
fn resolve(look: &Look, v: &str) -> Paint {
    match v {
        "base" | "bg" => Some((look.theme.base, 1.0)),
        _ => fmt::rgba(look, v),
    }
}

#[derive(Clone, Default)]
pub struct Pill {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub border: Option<String>,
    pub width: f32,
    /// el borde solo a la izquierda (los separadores de brutalist)
    pub border_left: bool,
    pub radius: f32,
    /// arriba, derecha, abajo, izquierda
    pub margin: [f32; 4],
    /// a cada lado del texto; None: las celdas `pad` de la barra
    pub padding: Option<f32>,
    pub shadow: Option<(f32, f32, String)>,
    pub bold: bool,
    /// los fondos de sus botones (el workspace activo), redondeados y de todo
    /// el alto, como el border-radius de `#workspaces button`
    pub button_radius: Option<f32>,
    /// el relleno de cada botón y el del activo, en pixeles (los workspaces)
    pub button_padding: Option<f32>,
    pub active_padding: Option<f32>,
    /// lo activo (la app enfocada, el workspace activo): una línea arriba o
    /// abajo (`active-line`, de `active-color` o del color del módulo), o un
    /// borde de otro color en su caja (`active-border`)
    pub line_top: bool,
    pub active_color: Option<String>,
    pub active_border: Option<String>,
    /// el fondo de su caja cuando está activo (la app enfocada en loteria)
    pub active_bg: Option<String>,
    /// el color de sus letras cuando está activo
    pub active_fg: Option<String>,
    /// el brillo de sus letras (text-shadow), por estado ("" el de siempre)
    pub glow: Vec<(String, Vec<Shadow>)>,
}

impl Pill {
    /// La de todos los módulos: `[bar.pill]`, o la isla de `style = "islands"`.
    pub fn base(v: &Value, islands: bool, radius: f32) -> Pill {
        let mut p = Pill { radius, ..Default::default() };
        if islands {
            p.bg = Some("base".into());
        }
        p.with(v)
    }

    /// Esta, con lo que diga `v` encima (el `[bar.modules.<nombre>]`).
    pub fn with(&self, v: &Value) -> Pill {
        let mut p = self.clone();
        if v.is_null() {
            return p;
        }
        if let Some(x) = s(v, "fg").or_else(|| s(v, "color")) {
            p.fg = Some(x);
        }
        if let Some(x) = s(v, "bg") {
            p.bg = Some(x);
        }
        if let Some(x) = s(v, "border") {
            p.border = Some(x);
        }
        if let Some(x) = s(v, "border-sides") {
            p.border_left = x == "left";
        }
        if let Some(x) = f(v, "border-width") {
            p.width = x;
        } else if p.border.is_some() && p.width == 0.0 {
            p.width = 1.0;
        }
        if let Some(x) = f(v, "radius") {
            p.radius = x;
        }
        if let Some(m) = v["margin"].as_array() {
            for (i, x) in m.iter().take(4).enumerate() {
                p.margin[i] = x.as_f64().unwrap_or(0.0) as f32;
            }
        }
        if let Some(x) = f(v, "padding") {
            p.padding = Some(x);
        }
        if let Some(sh) = v["shadow"].as_array()
            && let (Some(x), Some(y), Some(c)) = (sh.first().and_then(Value::as_f64), sh.get(1).and_then(Value::as_f64), sh.get(2).and_then(Value::as_str))
        {
            p.shadow = Some((x as f32, y as f32, c.to_string()));
        }
        if let Some(b) = v["bold"].as_bool() {
            p.bold = b;
        }
        if let Some(r) = f(v, "button-radius") {
            p.button_radius = Some(r);
        }
        if let Some(x) = f(v, "button-padding") {
            p.button_padding = Some(x);
        }
        if let Some(x) = f(v, "active-padding") {
            p.active_padding = Some(x);
        }
        if let Some(side) = s(v, "active-line") {
            p.line_top = side == "top";
        }
        if let Some(c) = s(v, "active-color") {
            p.active_color = Some(c);
        }
        if let Some(c) = s(v, "active-border") {
            p.active_border = Some(c);
        }
        if let Some(c) = s(v, "active-bg") {
            p.active_bg = Some(c);
        }
        if let Some(c) = s(v, "active-fg") {
            p.active_fg = Some(c);
        }
        for (k, x) in v.as_object().into_iter().flatten() {
            let Some(class) = k.strip_suffix("text-shadow").map(|c| c.trim_end_matches('-')) else { continue };
            let Some(x) = x.as_str() else { continue };
            p.glow.retain(|(c, _)| c != class);
            p.glow.push((class.to_string(), glow::parse(x)));
        }
        p
    }

    /// El brillo de sus letras en el estado `class` (o el de siempre).
    pub fn glow(&self, class: Option<&str>) -> &[Shadow] {
        let find = |c: &str| self.glow.iter().find(|(k, _)| k == c).map(|(_, g)| g.as_slice());
        class.and_then(find).or_else(|| find("")).unwrap_or(&[])
    }

    /// El color de las letras, si el look lo cambia para este módulo.
    pub fn text(&self, look: &Look) -> Option<Color> {
        self.fg.as_deref().and_then(|v| fmt::color(look, v))
    }

    /// Su caja: sombra, fondo y borde, en (x, y, w, h) pixeles del monitor.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&self, pix: &mut Pixmap, look: &Look, x: f32, y: f32, w: f32, h: f32, sf: f32, alpha: f32) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = self.radius * sf;
        if let Some((dx, dy, c)) = &self.shadow
            && let Some((c, a)) = resolve(look, c)
        {
            rounded(pix, x + dx * sf, y + dy * sf, w, h, r, c, a * alpha);
        }
        if let Some((c, a)) = self.bg.as_deref().and_then(|v| resolve(look, v)) {
            rounded(pix, x, y, w, h, r, c, a * alpha);
        }
        if let Some((c, a)) = self.border.as_deref().and_then(|v| resolve(look, v)) {
            let bw = (self.width * sf).max(1.0);
            if self.border_left {
                rounded(pix, x, y, bw, h, 0.0, c, a);
            } else {
                stroke_rounded(pix, x, y, w, h, r, bw, c, a, None);
            }
        }
    }
}

/// La caja de una sección entera (izquierda, centro, derecha), como las
/// `.modules-left` de waybar: su caja (como un módulo), relleno a cada lado
/// (`padding`, o `[izquierda, derecha]`), `dots` (un puntito de ese color en
/// cada orilla) y `underline` (una línea de ese color abajo). Con `slant`
/// (`left` o `right`) esa orilla va cortada en diagonal, de `slant-width`
/// pixeles adentro arriba a la esquina de abajo, con un trazo del color de
/// `underline` encima (las losas de neon).
#[derive(Clone, Default)]
pub struct Section {
    pub pill: Pill,
    pub pad: (f32, f32),
    pub dots: Option<String>,
    pub underline: Option<String>,
    /// la orilla cortada (true: la derecha) y cuánto entra arriba
    pub slant: Option<(bool, f32)>,
}

impl Section {
    pub fn new(v: &Value) -> Option<Section> {
        if v.is_null() {
            return None;
        }
        let pill = Pill::default().with(v);
        let pad = match &v["padding"] {
            Value::Array(a) => (
                a.first().and_then(Value::as_f64).unwrap_or(0.0) as f32,
                a.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32,
            ),
            x => {
                let p = x.as_f64().unwrap_or(0.0) as f32;
                (p, p)
            }
        };
        let slant = s(v, "slant").filter(|x| x == "left" || x == "right").map(|x| (x == "right", f(v, "slant-width").unwrap_or(22.0)));
        Some(Section { pill, pad, dots: s(v, "dots"), underline: s(v, "underline"), slant })
    }

    /// La caja de x0 a x1 (pixeles del monitor) en una barra de alto `h`.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&self, pix: &mut Pixmap, look: &Look, x0: f32, x1: f32, h: f32, sf: f32, alpha: f32) {
        let p = &self.pill;
        let (top, bot) = ((p.margin[0] * sf).round(), (p.margin[2] * sf).round());
        match self.slant {
            Some((right, w)) => self.slab(pix, look, x0, x1, top, h - bot, right, w * sf, sf, alpha),
            None => p.paint(pix, look, x0, top, x1 - x0, h - top - bot, sf, alpha),
        }
        if let Some((c, a)) = self.underline.as_deref().and_then(|v| resolve(look, v)) {
            rounded(pix, x0, h - bot - 2.0 * sf, x1 - x0, 2.0 * sf, 0.0, c, a);
        }
        if let Some((c, a)) = self.dots.as_deref().and_then(|v| resolve(look, v)) {
            let (r, cy) = (2.0 * sf, (top + h - bot) / 2.0);
            for cx in [x0 + 5.0 * sf, x1 - 5.0 * sf] {
                rounded(pix, cx - r, cy - r, 2.0 * r, 2.0 * r, r, c, a);
            }
        }
    }

    /// La losa con una orilla en diagonal: el fondo, un brillo tenue hacia
    /// adentro del corte y el trazo encima (como el degradado de paradas
    /// duras del CSS de neon).
    #[allow(clippy::too_many_arguments)]
    fn slab(&self, pix: &mut Pixmap, look: &Look, x0: f32, x1: f32, y0: f32, y1: f32, right: bool, w: f32, sf: f32, alpha: f32) {
        use tiny_skia::{FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Point, SpreadMode, Stroke, Transform};
        // el corte: de (arriba, w adentro) a (abajo, en la esquina)
        let (a, b) = if right { ((x1 - w, y0), (x1, y1)) } else { ((x0 + w, y0), (x0, y1)) };
        let poly = |pts: &[(f32, f32)]| {
            let mut pb = PathBuilder::new();
            pb.move_to(pts[0].0, pts[0].1);
            for p in &pts[1..] {
                pb.line_to(p.0, p.1);
            }
            pb.close();
            pb.finish()
        };
        let shape = if right { [(x0, y0), a, b, (x0, y1)] } else { [a, (x1, y0), (x1, y1), b] };
        let rgba = |c: Color, k: f32| match c {
            Color::Rgb(r, g, b) => tiny_skia::Color::from_rgba8(r, g, b, (k.clamp(0.0, 1.0) * 255.0) as u8),
            _ => tiny_skia::Color::TRANSPARENT,
        };
        let Some(path) = poly(&shape) else { return };
        if let Some((c, k)) = self.pill.bg.as_deref().and_then(|v| resolve(look, v)) {
            let mut paint = Paint::default();
            paint.set_color(rgba(c, k * alpha));
            pix.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
        let Some((c, k)) = self.underline.as_deref().and_then(|v| resolve(look, v)) else { return };
        // hacia adentro, perpendicular al corte
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let n = if right { (-dy / len, dx / len) } else { (dy / len, -dx / len) };
        let glow = 4.0 * sf;
        let stops = vec![GradientStop::new(0.0, rgba(c, 0.25 * k * alpha)), GradientStop::new(1.0, rgba(c, 0.0))];
        if let Some(shader) = LinearGradient::new(Point::from_xy(a.0, a.1), Point::from_xy(a.0 + n.0 * glow, a.1 + n.1 * glow), stops, SpreadMode::Pad, Transform::identity()) {
            let mut paint = Paint::default();
            paint.shader = shader;
            pix.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
        let mut pb = PathBuilder::new();
        pb.move_to(a.0, a.1);
        pb.line_to(b.0, b.1);
        if let Some(line) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(rgba(c, k * alpha));
            paint.anti_alias = true;
            pix.stroke_path(&line, &paint, &Stroke { width: 1.7 * sf, ..Default::default() }, Transform::identity(), None);
        }
    }
}
