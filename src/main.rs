//! empalme — la barra de discada. Se dibuja como las apps del rice (rola, vato,
//! arre, taco): celdas con la fuente de kitty y los colores y el estilo del
//! look; cómo es la barra en cada look (arriba o abajo, alto, islas o una sola
//! pieza, qué módulos van dónde) sale del `[bar]` de su look.toml.
//!
//!   empalme            la barra, en cada monitor
//!
//! Por ahora corre junto a waybar: `pkill -SIGUSR1 waybar` esconde waybar.

mod bar;
mod drawer;
mod fmt;
mod hypr;
mod modules;
mod signals;
mod style;
mod tooltip;

use bar::{Bar, Style};
use drawer::Drawer;
use discada_kit::look::Look;
use discada_kit::paint::{Painter, rounded};
use modules::{Act, Ctx, Ev, Module, Part};
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::layout::Rect;
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel::{Event, Sender, channel};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle, delegate_noop};
use smithay_client_toolkit::reexports::protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};
use std::cell::Cell;
use std::rc::Rc;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// Dónde quedó un módulo, en pixeles lógicos: lo que se ve (`width`) de su
/// ancho completo (`full` celdas), con `clip` pixeles escondidos a su
/// izquierda (un cajón a medio abrir).
struct Slot {
    module: usize,
    start: f32,
    width: f32,
    clip: f32,
    /// las celdas de su texto
    full: u16,
    /// el ancho de una celda de este módulo (su tamaño de letra)
    cw: f32,
    /// pixeles: margen izquierdo y relleno a cada lado del texto (el margen
    /// derecho ya va en el ancho)
    ml: f32,
    padx: f32,
}

impl Slot {
    /// Dónde empieza su texto (aunque esté escondido a la izquierda).
    fn text_x(&self) -> f32 {
        self.start - self.clip + self.ml + self.padx
    }

    /// La celda de su texto en x (pixeles); 0 si cae en el relleno izquierdo.
    fn col(&self, x: f32) -> u16 {
        ((x - self.text_x()) / self.cw).max(0.0) as u16
    }
}

/// Cómo se marca lo que está bajo el mouse: en qué lugar, de qué celda a qué
/// celda de su cuadro, y los colores.
#[derive(Clone, Copy)]
struct Hover {
    slot: usize,
    /// todo el módulo (su caja), o de qué celda a qué celda de su texto
    whole: bool,
    from: u16,
    to: u16,
    fg: Option<Color>,
    bg: Option<Color>,
}

/// Los módulos de la barra, y de qué cajón es cada uno.
struct Built {
    /// (sección 0 izquierda, 1 centro, 2 derecha; el módulo)
    modules: Vec<(u8, Box<dyn Module>)>,
    /// (cajón, ¿es el primero, el que siempre se ve?)
    member: Vec<Option<(usize, bool)>>,
    drawers: Vec<Drawer>,
    /// el tipo de cada módulo (`cpu` de `cpu#otro`), para `[bar.hover]`
    names: Vec<String>,
    /// su `font-size` propio, si tiene (los íconos de las apps, más grandes)
    sizes: Vec<Option<f32>>,
    /// cómo se ve cada uno (style.rs)
    pills: Vec<style::Pill>,
}

/// La barra en un monitor: lo suyo (superficie, tamaño, dónde quedó cada
/// módulo, sus cajones y el mouse); los módulos son los mismos en todas.
struct Out {
    output: wl_output::WlOutput,
    /// el nombre del monitor (DP-1…), para los workspaces de cada uno
    name: String,
    layer: LayerSurface,
    pool: Option<SlotPool>,
    configured: bool,
    size: (u32, u32),
    scale: i32,
    slots: Vec<Slot>,
    drawers: Vec<Drawer>,
    /// el mouse sobre esta barra (x en pixeles), y qué tiene debajo
    hover_x: Option<f32>,
    hovered: Option<(usize, Option<u16>)>,
    /// el último cuadro de los cajones (para saber cuánto avanzar)
    last_frame: Option<Instant>,
    /// ya se pidió al compositor que avise del siguiente cuadro
    frame_pending: bool,
}

struct App {
    registry: RegistryState,
    seat: SeatState,
    output: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    /// una barra por monitor
    outs: Vec<Out>,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_x: f64,
    scroll: f64,
    look: Look,
    painter: Painter,
    /// los de otro tamaño de letra: (tamaño, painter), y cuál usa cada módulo
    painters: Vec<(f32, Painter)>,
    painter_of: Vec<Option<usize>>,
    bar: Bar,
    /// (sección 0 izquierda, 1 centro, 2 derecha; el módulo)
    modules: Vec<(u8, Box<dyn Module>)>,
    member: Vec<Option<(usize, bool)>>,
    /// los cajones como salen al armar (cada barra tiene su copia)
    drawers: Vec<Drawer>,
    names: Vec<String>,
    pills: Vec<style::Pill>,
    /// cómo se ve la barra entera, y sus secciones (izquierda, centro, derecha)
    barbox: style::BarBox,
    sections: [Option<style::Section>; 3],
    tip: tooltip::Tip,
    /// el último cuadro del pulso del grabador
    last_pulse: Instant,
    /// el módulo idle dice si se quiere; la barra lo pide al compositor
    idle: Rc<Cell<bool>>,
    /// para armar de nuevo los módulos (los que lanzan hilos lo necesitan)
    tx: Sender<Ev>,
    /// escondida con SIGUSR1
    hidden: bool,
    idle_mgr: Option<ZwpIdleInhibitManagerV1>,
    inhibitor: Option<ZwpIdleInhibitorV1>,
    qh: QueueHandle<App>,
}

fn sections(bar: &Bar) -> [Option<style::Section>; 3] {
    ["left", "center", "right"].map(|k| style::Section::new(&bar.sections[k]))
}

fn build(bar: &Bar, ctx: &Ctx) -> Built {
    let mut out = Built { modules: Vec::new(), member: Vec::new(), drawers: Vec::new(), names: Vec::new(), sizes: Vec::new(), pills: Vec::new() };
    let pill = style::Pill::base(&bar.pill, bar.style == Style::Islands, bar.radius);
    let add = |out: &mut Built, sect: u8, name: &str, member: Option<(usize, bool)>| {
        // `[bar.modules.app]` vale para todos los `app#…`; lo de cada uno encima
        let kind = name.split('#').next().unwrap_or(name);
        let mut v = bar.modules[kind].clone();
        if kind != name
            && let (Some(base), Some(own)) = (v.as_object_mut(), bar.modules[name].as_object())
        {
            base.extend(own.clone());
        } else if kind != name && !bar.modules[name].is_null() {
            v = bar.modules[name].clone();
        }
        let module_pill = pill.with(&v);
        let conf = fmt::Conf(v);
        let size = conf.num("font-size").map(|f| f.clamp(6.0, 60.0) as f32);
        if let Some(m) = modules::make(name, conf, ctx) {
            out.sizes.push(size);
            out.pills.push(module_pill);
            out.modules.push((sect, m));
            out.member.push(member);
            out.names.push(name.split('#').next().unwrap_or(name).to_string());
        }
    };
    for (sect, names) in [&bar.left, &bar.center, &bar.right].into_iter().enumerate() {
        let sect = sect as u8;
        for name in names {
            let Some(group) = name.strip_prefix("group/") else {
                add(&mut out, sect, name, None);
                continue;
            };
            let g = &bar.groups[group];
            let names: Vec<&str> = g["modules"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            if g["drawer"].is_null() || names.len() < 2 {
                for n in names {
                    add(&mut out, sect, n, None);
                }
                continue;
            }
            let d = Drawer::new(&g["drawer"]);
            let idx = out.drawers.len();
            // el primero junto a los que salen: antes si salen a la derecha
            let (lead, rest) = names.split_first().unwrap();
            if !d.right {
                for n in rest {
                    add(&mut out, sect, n, Some((idx, false)));
                }
            }
            add(&mut out, sect, lead, Some((idx, true)));
            if d.right {
                for n in rest {
                    add(&mut out, sect, n, Some((idx, false)));
                }
            }
            out.drawers.push(d);
        }
    }
    out
}

impl App {
    /// La barra en el monitor `output`.
    fn add_out(&mut self, output: wl_output::WlOutput) {
        if self.hidden || self.outs.iter().any(|o| o.output == output) {
            return;
        }
        let name = self.output.info(&output).and_then(|i| i.name).unwrap_or_default();
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Top, Some("empalme"), Some(&output));
        let b = &self.bar;
        let edge = if b.bottom { Anchor::BOTTOM } else { Anchor::TOP };
        layer.set_anchor(edge | Anchor::LEFT | Anchor::RIGHT);
        let [t, r, bo, l] = b.margin;
        layer.set_margin(t, r, bo, l);
        // el espacio que reserva: su alto y el margen de su orilla
        layer.set_exclusive_zone(b.height as i32 + if b.bottom { bo } else { t });
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, b.height);
        layer.commit();
        self.outs.push(Out {
            output,
            name,
            layer,
            pool: None,
            configured: false,
            size: (0, 0),
            scale: 1,
            slots: Vec::new(),
            drawers: self.drawers.clone(),
            hover_x: None,
            hovered: None,
            last_frame: None,
            frame_pending: false,
        });
        self.sync_idle();
    }

    /// Las barras de todos los monitores (de nuevo, si ya había).
    fn make_outs(&mut self) {
        self.drop_outs();
        for o in self.output.outputs() {
            self.add_out(o);
        }
    }

    fn drop_outs(&mut self) {
        self.tip_hide();
        if let Some(i) = self.inhibitor.take() {
            i.destroy();
        }
        self.outs.clear();
    }

    /// Que el compositor no apague la pantalla mientras el módulo idle lo
    /// pida (sobre la superficie de la primera barra).
    fn sync_idle(&mut self) {
        let want = self.idle.get() && !self.outs.is_empty();
        match (want, &self.inhibitor, &self.idle_mgr) {
            (true, None, Some(mgr)) => {
                let layer = &self.outs[0].layer;
                self.inhibitor = Some(mgr.create_inhibitor(layer.wl_surface(), &self.qh, ()));
                layer.commit();
            }
            (false, Some(_), _) => {
                if let Some(i) = self.inhibitor.take() {
                    i.destroy();
                }
            }
            _ => {}
        }
    }

    /// La barra de esa superficie.
    fn out_of(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.outs.iter().position(|o| o.layer.wl_surface() == surface)
    }

    fn redraw_all(&mut self) {
        for i in 0..self.outs.len() {
            self.redraw(i);
        }
    }

    fn redraw(&mut self, i: usize) {
        if self.outs.get(i).is_some_and(|o| o.configured) {
            self.draw(i);
        }
    }

    fn draw(&mut self, i: usize) {
        let o = &self.outs[i];
        if o.size.0 == 0 {
            return;
        }
        let layer = o.layer.clone();
        let s = o.scale.max(1);
        let (w, h) = (o.size.0 as i32 * s, o.size.1 as i32 * s);
        let Some(mut pix) = tiny_skia::Pixmap::new(w as u32, h as u32) else { return };
        let cw = self.painter.cw;
        let gap = self.bar.spacing.unwrap_or(self.bar.gap as f32 * cw);
        let look = &self.look;
        let t = look.theme;

        // qué muestra cada módulo en este monitor y cuánto mide (su texto, su
        // relleno y sus márgenes, en pixeles)
        let lines: Vec<_> = self.modules.iter().map(|(sect, m)| (*sect, m.line_on(look, &o.name))).collect();
        let full: Vec<Option<u16>> = lines.iter().map(|(_, l)| l.as_ref().map(|l| l.width() as u16)).collect();
        let cws: Vec<f32> = self.painter_of.iter().map(|p| p.map_or(cw, |j| self.painters[j].1.cw)).collect();
        let padx: Vec<f32> = self.pills.iter().zip(&cws).map(|(p, cw)| p.padding.unwrap_or(self.bar.pad as f32 * cw)).collect();
        let outer = |i: usize| full[i].map_or(0.0, |c| c as f32 * cws[i] + 2.0 * padx[i] + self.pills[i].margin[1] + self.pills[i].margin[3]);
        // lo que se ve de cada uno (los que salen de un cajón, a medio abrir):
        // (visible, escondido a la izquierda), en pixeles
        let mut shown: Vec<(f32, f32)> = (0..full.len()).map(|i| (outer(i), 0.0)).collect();
        for (g, d) in o.drawers.iter().enumerate() {
            let rest: Vec<usize> = (0..full.len()).filter(|&i| self.member[i] == Some((g, false)) && full[i].is_some()).collect();
            let widths: Vec<f32> = rest.iter().map(|&i| shown[i].0).collect();
            for (k, v) in drawer::reveal(d, &widths, gap).into_iter().enumerate() {
                shown[rest[k]] = v;
            }
        }
        let bw = o.size.0 as f32;
        let mut slots = Vec::new();
        // dónde quedó la caja de cada sección (pixeles lógicos)
        let mut spans: Vec<(usize, f32, f32)> = Vec::new();
        for sect in 0..3u8 {
            let items: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].0 == sect && full[i].is_some() && shown[i].0 > 0.0).collect();
            let (pl, pr) = self.sections[sect as usize].as_ref().map_or((0.0, 0.0), |s| s.pad);
            let total: f32 = items.iter().map(|&i| shown[i].0).sum::<f32>() + gap * items.len().saturating_sub(1) as f32 + pl + pr;
            let mut x = match sect {
                0 => 0.0,
                1 => ((bw - total) / 2.0).max(0.0),
                _ => (bw - total).max(0.0),
            };
            if !items.is_empty() {
                spans.push((sect as usize, x, x + total));
            }
            x += pl;
            for i in items {
                let (width, clip) = shown[i];
                let p = &self.pills[i];
                slots.push(Slot { module: i, start: x, width, clip, full: full[i].unwrap(), cw: cws[i], ml: p.margin[3], padx: padx[i] });
                x += width + gap;
            }
        }
        self.outs[i].slots = slots;
        let o = &self.outs[i];

        // la barra, en pixeles del monitor (redondeados para que el texto quede nítido)
        let alpha = look.alpha();
        let sf = s as f32;
        let px = |v: f32| (v * sf).round();
        self.barbox.paint(&mut pix, look, sf, alpha);
        for &(sect, x0, x1) in &spans {
            if let Some(sec) = &self.sections[sect] {
                sec.paint(&mut pix, look, px(x0), px(x1), h as f32, sf, alpha);
            }
        }

        // la caja de cada módulo: de su margen izquierdo al derecho, recortada a
        // lo que se ve; lo que está bajo el mouse, con el fondo del hover
        let hov = self.hover_style(i);
        let boxes: Vec<(f32, f32)> = o
            .slots
            .iter()
            .map(|sl| {
                let x0 = (sl.start - sl.clip + sl.ml).max(sl.start);
                let x1 = (sl.start - sl.clip + sl.ml + sl.full as f32 * sl.cw + 2.0 * sl.padx).min(sl.start + sl.width);
                (x0, x1)
            })
            .collect();
        // lo activo de cada uno: la app enfocada (todo el módulo) o el botón
        // activo de adentro (el workspace), en celdas de su texto
        let active: Vec<Option<(Color, Option<Part>)>> = o
            .slots
            .iter()
            .map(|sl| {
                let m = &self.modules[sl.module].1;
                m.underline(look).map(|c| (c, None)).or_else(|| {
                    let p = &self.pills[sl.module];
                    let c = p.active_color.as_deref().and_then(|v| fmt::color(look, v))?;
                    m.active_part(&o.name).map(|part| (c, Some(part)))
                })
            })
            .collect();
        for (k, sl) in o.slots.iter().enumerate() {
            let p = &self.pills[sl.module];
            let (x0, x1) = boxes[k];
            let (mt, mb) = (p.margin[0], p.margin[2]);
            let mut p = p.clone();
            if let Some((_, None)) = active[k] {
                if let Some(b) = &p.active_border {
                    p.border = Some(b.clone());
                }
                if let Some(b) = &p.active_bg {
                    p.bg = Some(b.clone());
                }
            }
            if let Some(Hover { slot, whole: true, bg: Some(bg), .. }) = hov
                && slot == k
            {
                p.bg = Some(fmt::hexa(bg));
            }
            p.paint(&mut pix, look, px(x0), px(mt), px(x1) - px(x0), h as f32 - px(mt) - px(mb), sf, alpha);
        }
        if let Some(Hover { slot, whole: false, from, to, bg: Some(bg), .. }) = hov {
            let sl = &o.slots[slot];
            let x0 = (sl.text_x() + from as f32 * sl.cw).max(sl.start);
            let x1 = (sl.text_x() + to as f32 * sl.cw).min(sl.start + sl.width);
            if x1 > x0 {
                rounded(&mut pix, px(x0), 0.0, px(x1) - px(x0), h as f32, self.bar.radius * sf, bg, alpha);
            }
        }

        // el texto, centrado de alto en la barra; a medio abrir, en su propio
        // cuadro y solo lo que se ve
        for (k, sl) in self.outs[i].slots.iter().enumerate() {
            let painter = match self.painter_of[sl.module] {
                Some(j) => &mut self.painters[j].1,
                None => &mut self.painter,
            };
            let oy = ((h as f32 - painter.ch * sf) / 2.0).floor();
            let (x0, x1) = boxes[k];
            // la línea de lo activo: 2 pixeles abajo (o arriba), de todo el
            // módulo o de su botón activo, como el border del CSS
            let pill = &self.pills[sl.module];
            let line = active[k].filter(|_| pill.active_border.is_none() && pill.active_bg.is_none()).map(|(c, part)| {
                let c = pill.active_color.as_deref().and_then(|v| fmt::color(look, v)).unwrap_or(c);
                match part {
                    Some((a, w, _)) => (c, sl.text_x() + a as f32 * sl.cw, sl.text_x() + (a + w) as f32 * sl.cw),
                    None => (c, x0, x1),
                }
            });
            let ly = if pill.line_top { pill.margin[0] * sf } else { h as f32 - (pill.margin[2] + 2.0) * sf };
            if let Some((c, lx0, lx1)) = line
                && let Some(r) = tiny_skia::Rect::from_xywh(px(lx0), ly.round(), px(lx1) - px(lx0), 2.0 * sf)
            {
                let mut paint = tiny_skia::Paint::default();
                if let Color::Rgb(r_, g, b) = c {
                    paint.set_color_rgba8(r_, g, b, 255);
                }
                pix.fill_rect(r, &paint, tiny_skia::Transform::identity(), None);
            }
            let Some(line) = &lines[sl.module].1 else { continue };
            let mut one = Buffer::empty(Rect::new(0, 0, sl.full.max(1), 1));
            one.set_line(0, 0, line, sl.full);
            // el color y el grueso de las letras del módulo, si el look los cambia
            let pfg = pill.text(look);
            // activo con su color de letras (loteria): todas, aunque el módulo pinte las suyas
            let afg = matches!(active[k], Some((_, None))).then(|| pill.active_fg.as_deref().and_then(|v| fmt::color(look, v))).flatten();
            for x in 0..sl.full {
                let cell = &mut one[(x, 0)];
                if let Some(c) = afg {
                    cell.set_fg(c);
                } else if let Some(c) = pfg
                    && (cell.fg == t.fg || cell.fg == Color::Reset)
                {
                    cell.set_fg(c);
                }
                if pill.bold {
                    cell.modifier.insert(ratatui::style::Modifier::BOLD);
                }
            }
            if let Some(Hover { slot, from, to, fg, bg, whole }) = hov
                && slot == k
            {
                let (from, to) = if whole { (0, sl.full) } else { (from, to.min(sl.full)) };
                for x in from..to {
                    let cell = &mut one[(x, 0)];
                    if let Some(fg) = fg {
                        cell.set_fg(fg);
                    }
                    // las celdas con fondo propio (el workspace activo) también
                    if let Some(bg) = bg
                        && cell.bg != Color::Reset
                    {
                        cell.set_bg(bg);
                    }
                }
            }
            let tx = sl.text_x();
            // los fondos de los botones (el workspace activo): redondeados y de
            // todo el alto de la caja, en vez del cuadro de cada celda
            if let Some(br) = pill.button_radius
                && sl.clip <= 0.01
            {
                let (top, bot) = (pill.margin[0] + pill.width, pill.margin[2] + pill.width);
                let mut x = 0;
                while x < sl.full {
                    let bg = one[(x, 0)].bg;
                    if bg == Color::Reset {
                        x += 1;
                        continue;
                    }
                    let start = x;
                    while x < sl.full && one[(x, 0)].bg == bg {
                        one[(x, 0)].set_bg(Color::Reset);
                        x += 1;
                    }
                    let (bx0, bx1) = (px(tx + start as f32 * sl.cw), px(tx + x as f32 * sl.cw));
                    rounded(&mut pix, bx0, px(top), bx1 - bx0, h as f32 - px(top) - px(bot), br * sf, bg, 1.0);
                }
            }
            if sl.clip <= 0.01 && sl.width >= x1 - sl.start {
                painter.cells(&mut pix, &one, look, px(tx), oy, sf);
                continue;
            }
            let vis = (px(sl.start + sl.width) - px(sl.start)) as u32;
            let Some(mut part) = tiny_skia::Pixmap::new(vis.max(1), h as u32) else { continue };
            painter.cells(&mut part, &one, look, px(tx) - px(sl.start), oy, sf);
            pix.draw_pixmap(px(sl.start) as i32, 0, part.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        }

        let o = &mut self.outs[i];
        let pool = match &mut o.pool {
            Some(p) => p,
            None => o.pool.insert(SlotPool::new((w * h * 4) as usize, &self.shm).expect("memoria compartida")),
        };
        let Ok((buffer, canvas)) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else { return };
        // tiny-skia: RGBA premultiplicado; Wayland ARGB8888 en little endian: BGRA
        for (dst, src) in canvas.as_chunks_mut::<4>().0.iter_mut().zip(pix.data().chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        let surface = layer.wl_surface();
        surface.set_buffer_scale(s);
        surface.damage_buffer(0, 0, w, h);
        if self.drawers_moving(i) && !self.outs[i].frame_pending {
            surface.frame(&self.qh, FrameCallbackData(surface.clone()));
            self.outs[i].frame_pending = true;
        }
        if buffer.attach_to(surface).is_ok() {
            layer.commit();
        }
        // lo que dice el tooltip puede haber cambiado (el cpu, la hora…)
        if self.tip.layer.is_some() && self.tip.out == i {
            self.tip_show();
        }
    }

    /// El lugar de la barra `i` en x (pixeles).
    fn slot_at(&self, i: usize, x: f32) -> Option<usize> {
        self.outs[i].slots.iter().position(|s| x >= s.start && x < s.start + s.width)
    }

    /// Un clic o la rueda en la barra `i`, en x (pixeles): al módulo que esté ahí.
    fn act(&mut self, i: usize, act: Act) {
        let x = self.pointer_x as f32;
        let Some(k) = self.slot_at(i, x) else { return };
        let sl = &self.outs[i].slots[k];
        let rel = sl.col(x);
        let m = sl.module;
        let name = self.outs[i].name.clone();
        self.modules[m].1.act_on(act, rel, &name);
        self.sync_idle();
        self.tip_reset(false);
        // lo que cambia solo con el clic (el reloj largo): de una vez
        self.redraw_all();
    }

    /// El mouse en la barra `i`, en x (pixeles; None: se fue): abrir el cajón
    /// que tenga debajo y cerrar los demás.
    fn hover(&mut self, i: usize, x: Option<f64>) {
        self.outs[i].hover_x = x.map(|x| x as f32);
        let now = self.under_mouse(i).map(|(k, part)| (self.outs[i].slots[k].module, part.map(|p| p.0)));
        if now != self.outs[i].hovered {
            self.outs[i].hovered = now;
            if now.is_some() || self.tip.out == i {
                self.tip.out = i;
                self.tip_reset(true);
            }
            if !self.drawers_moving(i) {
                self.redraw(i);
            }
        }
        let g = x.and_then(|x| self.slot_at(i, x as f32)).and_then(|k| self.member[self.outs[i].slots[k].module].map(|m| m.0));
        let was = self.drawers_moving(i);
        for (n, d) in self.outs[i].drawers.iter_mut().enumerate() {
            d.open = g == Some(n);
        }
        // empezó a moverse: el primer cuadro ya, los demás con el compositor
        if !was && self.drawers_moving(i) && !self.outs[i].frame_pending {
            self.frame(i);
        }
    }

    /// Un painter por cada tamaño de letra propio que pidan los módulos.
    fn make_painters(&mut self, sizes: &[Option<f32>]) {
        self.painters.retain(|(sz, _)| sizes.contains(&Some(*sz)));
        self.painter_of = sizes
            .iter()
            .map(|sz| {
                let sz = (*sz)?;
                Some(self.painters.iter().position(|(p, _)| *p == sz).unwrap_or_else(|| {
                    self.painters.push((sz, Painter::sized(Some(sz))));
                    self.painters.len() - 1
                }))
            })
            .collect();
    }

    /// Lo que está bajo el mouse en la barra `i`: (el lugar, y el botón de
    /// adentro si tiene).
    fn under_mouse(&self, i: usize) -> Option<(usize, Option<Part>)> {
        let o = &self.outs[i];
        let x = o.hover_x?;
        let k = self.slot_at(i, x)?;
        let sl = &o.slots[k];
        Some((k, self.modules[sl.module].1.part_on(sl.col(x), &o.name)))
    }

    /// Cómo se marca lo que está bajo el mouse (según `[bar.hover]`).
    fn hover_style(&self, i: usize) -> Option<Hover> {
        let (k, part) = self.under_mouse(i)?;
        let sl = &self.outs[i].slots[k];
        let c = |conf: &serde_json::Value, key: &str| conf[key].as_str().and_then(|v| fmt::color(&self.look, v));
        match part {
            Some((a, w, active)) => {
                let h = &self.bar.hover_ws;
                if h.is_null() {
                    return None;
                }
                let (fg, bg) = if active { ("active-fg", "active-bg") } else { ("fg", "bg") };
                Some(Hover { slot: k, whole: false, from: a, to: a + w, fg: c(h, fg), bg: c(h, bg) })
            }
            None => {
                let h = &self.bar.hover;
                let name = &self.names[sl.module];
                let listed = h["modules"].as_array().is_none_or(|l| l.iter().any(|m| m.as_str() == Some(name)));
                if h.is_null() || !listed || name == "workspaces" {
                    return None;
                }
                Some(Hover { slot: k, whole: true, from: 0, to: sl.full, fg: c(h, "fg"), bg: c(h, "bg") })
            }
        }
    }

    /// ¿Algún cajón de la barra `i` se está abriendo o cerrando? Esos van al
    /// ritmo de la pantalla, con lo que avisa el compositor en cada cuadro.
    fn drawers_moving(&self, i: usize) -> bool {
        !self.hidden && self.outs.get(i).is_some_and(|o| o.drawers.iter().any(Drawer::moving))
    }

    /// ¿Algún módulo se mueve solo (el grabador pulsando)? Ese con reloj.
    fn pulsing(&self) -> bool {
        !self.hidden && self.outs.iter().any(|o| o.configured) && self.modules.iter().any(|(_, m)| m.animating())
    }

    /// Un cuadro de los cajones de la barra `i`: avanzar lo que pasó desde el
    /// anterior y redibujar (draw pide el siguiente si siguen moviéndose).
    fn frame(&mut self, i: usize) {
        let now = Instant::now();
        let o = &mut self.outs[i];
        let dt = o.last_frame.map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(0.1));
        o.last_frame = Some(now);
        for d in &mut o.drawers {
            d.step(dt);
        }
        if !self.drawers_moving(i) {
            self.outs[i].last_frame = None;
        }
        self.redraw(i);
    }

    fn on_ev(&mut self, ev: Ev) {
        match ev {
            Ev::Toggle => {
                self.hidden = !self.hidden;
                if self.hidden {
                    self.drop_outs();
                } else {
                    self.make_outs();
                }
                return;
            }
            Ev::Reload => {
                self.look.refresh();
                self.reload(true);
                return;
            }
            _ => {}
        }
        let mut dirty = false;
        for (_, m) in &mut self.modules {
            dirty |= m.update(&ev);
        }
        if dirty {
            self.redraw_all();
        }
    }

    /// Otro look u otra barra: releer y volver a armar.
    fn reload(&mut self, new_bar: bool) {
        if new_bar {
            self.bar = bar::load();
            self.painter = Painter::sized(self.bar.font_size);
            let b = build(&self.bar, &Ctx { idle: self.idle.clone(), tx: self.tx.clone() });
            (self.modules, self.member, self.drawers, self.names, self.pills) = (b.modules, b.member, b.drawers, b.names, b.pills);
            self.barbox = style::BarBox::new(&self.bar.boxed, self.bar.style == Style::Solid, self.bar.radius);
            self.sections = sections(&self.bar);
            self.painters.clear();
            self.make_painters(&b.sizes);
            if !self.hidden {
                self.make_outs();
            }
        } else {
            self.painter = Painter::sized(self.bar.font_size);
            for (sz, p) in &mut self.painters {
                *p = Painter::sized(Some(*sz));
            }
            self.redraw_all();
        }
    }
}

fn main() {
    if let Some(a) = std::env::args().nth(1)
        && (a == "-h" || a == "--help") {
            println!("empalme — la barra de discada (cómo es: el [bar] del look.toml del look)");
            return;
        }
    if let Err(e) = run() {
        eprintln!("empalme: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    // antes de abrir cualquier hilo, para que todos las tengan bloqueadas
    signals::block();
    let conn = Connection::connect_to_env().map_err(|e| format!("Wayland: {e}"))?;
    let (globals, queue) = registry_queue_init(&conn).map_err(|e| format!("Wayland: {e}"))?;
    let qh = queue.handle();
    let mut event_loop: EventLoop<App> = EventLoop::try_new().map_err(|e| e.to_string())?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), queue).insert(handle.clone()).map_err(|e| e.to_string())?;
    let (tx, rx) = channel::<Ev>();
    handle
        .insert_source(rx, |ev, _, app: &mut App| {
            if let Event::Msg(ev) = ev {
                app.on_ev(ev);
            }
        })
        .map_err(|e| e.to_string())?;
    hypr::events(tx.clone());
    modules::audio::watch(tx.clone());
    modules::bluetooth::watch(tx.clone());
    modules::notifications::watch(tx.clone());
    modules::visualizer::watch(tx.clone());
    signals::watch(tx.clone());
    // los nombres de días y meses del reloj, en el idioma del sistema
    unsafe { libc::setlocale(libc::LC_TIME, c"".as_ptr()) };

    let bar = bar::load();
    let idle = Rc::new(Cell::new(false));
    let ctx = Ctx { idle: idle.clone(), tx: tx.clone() };
    let built = build(&bar, &ctx);
    let built_sizes = built.sizes.clone();
    let mut app = App {
        registry: RegistryState::new(&globals),
        seat: SeatState::new(&globals, &qh),
        output: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh).map_err(|_| "el compositor no tiene wl_compositor")?,
        layer_shell: LayerShell::bind(&globals, &qh).map_err(|_| "el compositor no tiene layer-shell")?,
        shm: Shm::bind(&globals, &qh).map_err(|_| "el compositor no tiene wl_shm")?,
        outs: Vec::new(),
        pointer: None,
        pointer_x: 0.0,
        scroll: 0.0,
        look: Look::load("empalme"),
        painter: Painter::sized(bar.font_size),
        painters: Vec::new(),
        painter_of: Vec::new(),
        modules: built.modules,
        member: built.member,
        drawers: built.drawers,
        names: built.names,
        pills: built.pills,
        barbox: style::BarBox::default(),
        sections: [None, None, None],
        tip: tooltip::Tip::default(),
        last_pulse: Instant::now(),
        bar,
        idle,
        tx,
        hidden: false,
        idle_mgr: globals.bind::<ZwpIdleInhibitManagerV1, _, _>(&qh, 1..=1, ()).ok(),
        inhibitor: None,
        qh,
    };
    app.make_painters(&built_sizes);
    app.barbox = style::BarBox::new(&app.bar.boxed, app.bar.style == Style::Solid, app.bar.radius);
    app.sections = sections(&app.bar);
    // las barras salen con cada monitor (new_output)

    let mut last_sec = 0;
    let mut bar_stamp = bar::stamp();
    fmt::load_palette();
    let mut palette_stamp = fmt::palette_stamp();
    loop {
        // despertar justo al cambiar el segundo, para que el reloj no se salte uno
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let mut wait = Duration::from_millis(1000 - now.subsec_millis() as u64 + 2);
        if let Some(d) = app.tip_due() {
            wait = wait.min(d);
        }
        const PULSE: Duration = Duration::from_millis(33);
        if app.pulsing() {
            wait = wait.min(PULSE.saturating_sub(app.last_pulse.elapsed()));
        }
        event_loop.dispatch(wait, &mut app).map_err(|e| e.to_string())?;
        if app.tip_due() == Some(Duration::ZERO) {
            app.tip_show();
        }
        // el pulso del grabador (si los cajones se mueven, ya se redibuja con ellos)
        if app.pulsing() && app.last_pulse.elapsed() >= PULSE {
            app.last_pulse = Instant::now();
            for i in 0..app.outs.len() {
                if !app.outs[i].frame_pending {
                    app.redraw(i);
                }
            }
        }
        let sec = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        if sec != last_sec {
            last_sec = sec;
            app.on_ev(Ev::Tick);
            let mut look_changed = app.look.refresh();
            let ps = fmt::palette_stamp();
            if ps != palette_stamp {
                palette_stamp = ps;
                fmt::load_palette();
                look_changed = true;
            }
            let stamp = bar::stamp();
            if stamp != bar_stamp {
                bar_stamp = stamp;
                app.reload(true);
            } else if look_changed {
                app.reload(false);
            }
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        if let Some(i) = self.out_of(surface)
            && factor != self.outs[i].scale
        {
            self.outs[i].scale = factor;
            self.outs[i].pool = None;
            self.redraw(i);
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        let Some(i) = self.out_of(surface) else { return };
        self.outs[i].frame_pending = false;
        if self.drawers_moving(i) {
            self.frame(i);
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_out(output);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        // otro nombre (o llegó tarde): ponerlo al día
        let name = self.output.info(&output).and_then(|i| i.name).unwrap_or_default();
        if let Some(o) = self.outs.iter_mut().find(|o| o.output == output) {
            o.name = name;
        }
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let Some(i) = self.outs.iter().position(|o| o.output == output) else { return };
        if self.tip.out == i {
            self.tip_hide();
        }
        if i == 0
            && let Some(inh) = self.inhibitor.take()
        {
            inh.destroy();
        }
        self.outs.remove(i);
        self.tip.out = 0;
        self.sync_idle();
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.tip.layer.as_ref() == Some(layer) {
            self.tip_hide();
            return;
        }
        self.outs.retain(|o| &o.layer != layer);
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, cfg: LayerSurfaceConfigure, _: u32) {
        if self.tip.layer.as_ref() == Some(layer) {
            self.tip_configure(cfg.new_size);
            return;
        }
        let Some(i) = self.outs.iter().position(|o| &o.layer == layer) else { return };
        let o = &mut self.outs[i];
        if cfg.new_size.0 > 0 && cfg.new_size.1 > 0 {
            if cfg.new_size != o.size {
                o.pool = None;
            }
            o.size = cfg.new_size;
        }
        o.configured = true;
        self.draw(i);
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat.get_pointer(qh, &seat).ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Pointer
            && let Some(p) = self.pointer.take() {
                p.release();
            }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for e in events {
            let Some(i) = self.out_of(&e.surface) else { continue };
            self.pointer_x = e.position.0;
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => self.hover(i, Some(e.position.0)),
                PointerEventKind::Press { button, .. } => match button {
                    BTN_LEFT => self.act(i, Act::Left),
                    BTN_RIGHT => self.act(i, Act::Right),
                    BTN_MIDDLE => self.act(i, Act::Middle),
                    _ => {}
                },
                PointerEventKind::Axis { vertical, .. } => {
                    // un paso de rueda (o 15 pixeles de touchpad) = una vez
                    let step = if vertical.discrete != 0 { vertical.discrete as f64 * 15.0 } else { vertical.absolute };
                    self.scroll += step;
                    while self.scroll.abs() >= 15.0 {
                        let down = self.scroll > 0.0;
                        self.scroll -= 15.0f64.copysign(self.scroll);
                        self.act(i, if down { Act::Down } else { Act::Up });
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.scroll = 0.0;
                    self.hover(i, None);
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(App);
delegate_noop!(App: ZwpIdleInhibitManagerV1);
delegate_noop!(App: ZwpIdleInhibitorV1);
