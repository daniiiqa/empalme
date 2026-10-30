//! El tooltip: lo que dice el módulo bajo el mouse, en un cuadro debajo de la
//! barra (encima si la barra va abajo). Sale tras medio segundo con el mouse
//! quieto encima, como en waybar, y se va al moverse a otro lado o con un
//! clic. Es otra superficie de layer-shell que no recibe el mouse, para no
//! quitárselo a la barra. Cómo se ve: `[bar.tooltip]` (fg, bg, border,
//! radius).

use crate::App;
use crate::fmt;
use discada_kit::paint::rounded;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use smithay_client_toolkit::compositor::Region;
use smithay_client_toolkit::reexports::client::protocol::wl_shm;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerSurface};
use smithay_client_toolkit::shm::slot::SlotPool;
use std::time::{Duration, Instant};

/// Cuánto hay que dejar el mouse quieto para que salga.
const DELAY: Duration = Duration::from_millis(500);
/// Separación de la barra, en pixeles.
const GAP: i32 = 4;

#[derive(Default)]
pub struct Tip {
    pub layer: Option<LayerSurface>,
    pool: Option<SlotPool>,
    /// el tamaño que le dio el compositor
    size: (u32, u32),
    ready: bool,
    /// lo que pidió: tamaño y orilla izquierda
    want: (u32, u32, i32),
    text: String,
    lines: Vec<Line<'static>>,
    /// desde cuándo está el mouse sobre lo mismo
    pub since: Option<Instant>,
    /// en la barra de qué monitor
    pub out: usize,
}

impl App {
    /// ¿Cuánto falta para que salga? None: no toca (ya salió, o no hay nada).
    pub fn tip_due(&self) -> Option<Duration> {
        if self.tip.layer.is_some() || self.hidden {
            return None;
        }
        Some(DELAY.saturating_sub(self.tip.since?.elapsed()))
    }

    /// Lo que está bajo el mouse cambió (o hubo clic): quitarlo, y contar de
    /// nuevo si hay algo debajo.
    pub fn tip_reset(&mut self, again: bool) {
        self.tip_hide();
        let hovered = self.outs.get(self.tip.out).is_some_and(|o| o.hovered.is_some());
        self.tip.since = (again && hovered).then(Instant::now);
    }

    pub fn tip_hide(&mut self) {
        self.tip.layer = None;
        self.tip.pool = None;
        self.tip.ready = false;
        self.tip.text.clear();
    }

    /// El texto y dónde va (el centro del módulo, en pixeles de la barra).
    fn tip_text(&self) -> Option<(String, f32)> {
        let o = self.outs.get(self.tip.out)?;
        let (module, _) = o.hovered?;
        let sl = o.slots.iter().find(|s| s.module == module)?;
        let text = self.modules[module].1.tooltip(&self.look)?;
        (!text.trim().is_empty()).then(|| (text, sl.start + sl.width / 2.0))
    }

    /// Sacarlo, o ponerlo al día si ya está (el texto cambió).
    pub fn tip_show(&mut self) {
        let Some((text, cx)) = self.tip_text() else {
            self.tip_hide();
            return;
        };
        if self.tip.layer.is_some() && text == self.tip.text {
            return;
        }
        let t = self.look.theme;
        let fg = self.tip_color("fg").unwrap_or(t.fg);
        self.tip.lines = text.lines().map(|l| fmt::line(l, Style::new().fg(fg), &self.look)).collect();
        self.tip.text = text;
        let (cw, ch) = (self.painter.cw, self.painter.ch);
        let cols = self.tip.lines.iter().map(Line::width).max().unwrap_or(0) as f32 + 2.0;
        let w = (cols * cw).ceil() as u32 + 2;
        let h = (self.tip.lines.len() as f32 * ch).ceil() as u32 + 8;
        let bw = self.outs[self.tip.out].size.0 as i32;
        let left = self.bar.margin[3] + ((cx - w as f32 / 2.0).round() as i32).clamp(0, (bw - w as i32).max(0));
        let want = (w, h, left);
        if let Some(layer) = &self.tip.layer {
            if want != self.tip.want {
                self.tip.want = want;
                layer.set_size(w, h);
                layer.set_margin(GAP, 0, GAP, left);
                layer.commit();
            } else {
                self.tip_draw();
            }
            return;
        }
        let surface = self.compositor.create_surface(&self.qh);
        // que no reciba el mouse: la barra lo sigue teniendo
        if let Ok(r) = Region::new(&self.compositor) {
            surface.set_input_region(Some(r.wl_region()));
        }
        let output = self.outs[self.tip.out].output.clone();
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Overlay, Some("empalme-tooltip"), Some(&output));
        let edge = if self.bar.bottom { Anchor::BOTTOM } else { Anchor::TOP };
        layer.set_anchor(edge | Anchor::LEFT);
        layer.set_margin(GAP, 0, GAP, left);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(w, h);
        layer.commit();
        self.tip.want = want;
        self.tip.ready = false;
        self.tip.layer = Some(layer);
    }

    fn tip_color(&self, key: &str) -> Option<Color> {
        self.bar.tooltip[key].as_str().and_then(|v| fmt::color(&self.look, v))
    }

    /// Le dio tamaño el compositor: dibujarlo.
    pub fn tip_configure(&mut self, size: (u32, u32)) {
        if size.0 > 0 && size.1 > 0 {
            if size != self.tip.size {
                self.tip.pool = None;
            }
            self.tip.size = size;
        }
        self.tip.ready = true;
        self.tip_draw();
    }

    fn tip_draw(&mut self) {
        let Some(layer) = self.tip.layer.clone() else { return };
        if !self.tip.ready || self.tip.size.0 == 0 {
            return;
        }
        let s = self.outs.get(self.tip.out).map_or(1, |o| o.scale.max(1));
        let sf = s as f32;
        let (w, h) = (self.tip.size.0 as i32 * s, self.tip.size.1 as i32 * s);
        let Some(mut pix) = tiny_skia::Pixmap::new(w as u32, h as u32) else { return };
        let t = self.look.theme;
        let r = self.bar.tooltip["radius"].as_f64().unwrap_or(self.bar.radius as f64) as f32 * sf;
        let border = self.tip_color("border").unwrap_or(t.border);
        let bg = self.tip_color("bg").unwrap_or(t.base);
        // el borde: el cuadro entero de su color y encima el fondo, un pixel adentro
        rounded(&mut pix, 0.0, 0.0, w as f32, h as f32, r, border, 1.0);
        rounded(&mut pix, sf, sf, w as f32 - 2.0 * sf, h as f32 - 2.0 * sf, (r - sf).max(0.0), bg, 1.0);
        let cols = ((self.tip.size.0 as f32 - 2.0) / self.painter.cw) as u16;
        let rows = self.tip.lines.len() as u16;
        let mut buf = Buffer::empty(Rect::new(0, 0, cols.max(1), rows.max(1)));
        for (i, line) in self.tip.lines.iter().enumerate() {
            buf.set_line(1, i as u16, line, cols.saturating_sub(2));
        }
        self.painter.cells(&mut pix, &buf, &self.look, sf, 4.0 * sf, sf);

        let pool = match &mut self.tip.pool {
            Some(p) => p,
            None => self.tip.pool.insert(SlotPool::new((w * h * 4) as usize, &self.shm).expect("memoria compartida")),
        };
        let Ok((buffer, canvas)) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else { return };
        for (dst, src) in canvas.as_chunks_mut::<4>().0.iter_mut().zip(pix.data().chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        let surface = layer.wl_surface();
        surface.set_buffer_scale(s);
        surface.damage_buffer(0, 0, w, h);
        if buffer.attach_to(surface).is_ok() {
            layer.commit();
        }
    }
}
