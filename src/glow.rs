//! El text-shadow del CSS de waybar (el brillo de neon, ofrenda y retro): el
//! texto en el color de la sombra, desenfocado y debajo del texto. Se
//! escribe igual que en CSS: `0 0 6px alpha(color12, 0.7), 0 1px 3px black`,
//! o `none`; los colores como en los formatos.

use crate::fmt;
use disco::look::Look;
use ratatui::style::Color;
use tiny_skia::{Pixmap, PixmapPaint, PremultipliedColorU8, Transform};

#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    pub color: String,
}

/// Lo de adentro de `text-shadow`; `none` (o nada que entender): ninguna.
pub fn parse(v: &str) -> Vec<Shadow> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in v.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&v[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&v[start..]);
    parts
        .into_iter()
        .filter_map(|p| {
            let toks: Vec<&str> = p.split_whitespace().collect();
            let lens: Vec<f32> = toks.iter().map_while(|t| t.trim_end_matches("px").parse().ok()).collect();
            let color = toks[lens.len()..].join(" ");
            (lens.len() >= 2 && !color.is_empty()).then(|| Shadow { dx: lens[0], dy: lens[1], blur: lens.get(2).copied().unwrap_or(0.0), color })
        })
        .collect()
}

/// Cuánto se sale de su texto la sombra más grande (pixeles del monitor).
pub fn reach(shadows: &[Shadow], sf: f32) -> f32 {
    shadows.iter().map(|s| (s.blur * 1.5 + s.dx.abs().max(s.dy.abs())) * sf).fold(0.0, f32::max).ceil() + 2.0
}

/// Una pasada de caja de radio `r` a lo largo (`step` entre vecinos, `n`
/// de largo) de cada una de las `lines` líneas (`stride` entre ellas).
fn pass(src: &[f32], dst: &mut [f32], n: usize, lines: usize, step: usize, stride: usize, r: usize) {
    let k = 1.0 / (2 * r + 1) as f32;
    for l in 0..lines {
        let at = |i: usize| l * stride + i * step;
        let mut sum: f32 = (0..=r.min(n - 1)).map(|i| src[at(i)]).sum();
        for i in 0..n {
            dst[at(i)] = sum * k;
            if i + r + 1 < n {
                sum += src[at(i + r + 1)];
            }
            if i >= r {
                sum -= src[at(i - r)];
            }
        }
    }
}

/// Desenfoca `a` (w×h) como una gaussiana de `sigma`: tres pasadas de caja.
fn blur(a: &mut [f32], w: usize, h: usize, sigma: f32) {
    if sigma < 0.5 || w == 0 || h == 0 {
        return;
    }
    let r = ((((4.0 * sigma * sigma + 1.0).sqrt() - 1.0) / 2.0).round() as usize).max(1);
    let mut tmp = vec![0.0; a.len()];
    for _ in 0..3 {
        pass(a, &mut tmp, w, h, 1, w, r);
        pass(&tmp, a, h, w, w, 1, r);
    }
}

/// Las sombras de un texto: `cover` es el texto dibujado aparte (solo cuenta
/// cuánto cubre cada pixel), y va a dar a `pix` en (x, 0). La primera
/// sombra queda arriba, como en CSS. `opacity`: la del módulo.
pub fn paint(pix: &mut Pixmap, cover: &Pixmap, shadows: &[Shadow], look: &Look, x: f32, sf: f32, opacity: f32) {
    let (w, h) = (cover.width() as usize, cover.height() as usize);
    let base: Vec<f32> = cover.pixels().iter().map(|p| p.alpha() as f32 / 255.0).collect();
    for s in shadows.iter().rev() {
        let Some((Color::Rgb(r, g, b), ca)) = fmt::rgba(look, &s.color) else { continue };
        let mut a = base.clone();
        blur(&mut a, w, h, s.blur * sf / 2.0);
        let Some(mut layer) = Pixmap::new(w as u32, h as u32) else { continue };
        for (px, v) in layer.pixels_mut().iter_mut().zip(&a) {
            let k = (v * ca * opacity).clamp(0.0, 1.0);
            if k > 0.0 {
                let m = |c: u8| (c as f32 * k).round() as u8;
                *px = PremultipliedColorU8::from_rgba(m(r), m(g), m(b), (k * 255.0).round() as u8).unwrap_or(*px);
            }
        }
        pix.draw_pixmap((x + s.dx * sf).round() as i32, (s.dy * sf).round() as i32, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sombras() {
        let s = parse("0 0 2px alpha(@background, 0.95), 0 1px 3px black");
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].blur, s[0].color.as_str()), (2.0, "alpha(@background, 0.95)"));
        assert_eq!((s[1].dy, s[1].color.as_str()), (1.0, "black"));
        assert!(parse("none").is_empty());
    }

    #[test]
    fn desenfoque() {
        let mut a = vec![0.0; 21 * 21];
        a[10 * 21 + 10] = 1.0;
        blur(&mut a, 21, 21, 2.0);
        let total: f32 = a.iter().sum();
        assert!((total - 1.0).abs() < 0.01);
        assert!(a[10 * 21 + 10] > a[10 * 21 + 13]);
    }
}
