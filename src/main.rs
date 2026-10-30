//! empalme — la barra de discada. Se dibuja como las apps del rice (rola, vato,
//! arre, taco): celdas con la fuente de kitty y los colores y el estilo del
//! look; cómo es la barra en cada look (arriba o abajo, alto, islas o una sola
//! pieza, qué módulos van dónde) sale del `[bar]` de su look.toml.
//!
//!   empalme            la barra (en el monitor que diga Hyprland)
//!
//! Por ahora corre junto a waybar: `pkill -SIGUSR1 waybar` esconde waybar.

mod bar;
mod drawer;
mod fmt;
mod hypr;
mod modules;
mod signals;

use bar::{Bar, Style};
use drawer::Drawer;
use discada_kit::look::Look;
use discada_kit::paint::{Painter, rounded};
use modules::{Act, Ctx, Ev, Module};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel::{Event, channel};
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
    full: u16,
}

/// Los módulos de la barra, y de qué cajón es cada uno.
struct Built {
    /// (sección 0 izquierda, 1 centro, 2 derecha; el módulo)
    modules: Vec<(u8, Box<dyn Module>)>,
    /// (cajón, ¿es el primero, el que siempre se ve?)
    member: Vec<Option<(usize, bool)>>,
    drawers: Vec<Drawer>,
}

struct App {
    registry: RegistryState,
    seat: SeatState,
    output: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: Option<SlotPool>,
    layer: Option<LayerSurface>,
    configured: bool,
    size: (u32, u32),
    scale: i32,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_x: f64,
    scroll: f64,
    look: Look,
    painter: Painter,
    bar: Bar,
    /// (sección 0 izquierda, 1 centro, 2 derecha; el módulo)
    modules: Vec<(u8, Box<dyn Module>)>,
    member: Vec<Option<(usize, bool)>>,
    drawers: Vec<Drawer>,
    /// el último cuadro de los cajones (para saber cuánto avanzar)
    last_frame: Option<Instant>,
    /// ya se pidió al compositor que avise del siguiente cuadro
    frame_pending: bool,
    /// el último cuadro del pulso del grabador
    last_pulse: Instant,
    slots: Vec<Slot>,
    /// el módulo idle dice si se quiere; la barra lo pide al compositor
    idle: Rc<Cell<bool>>,
    /// escondida con SIGUSR1
    hidden: bool,
    idle_mgr: Option<ZwpIdleInhibitManagerV1>,
    inhibitor: Option<ZwpIdleInhibitorV1>,
    qh: QueueHandle<App>,
}

fn build(bar: &Bar, ctx: &Ctx) -> Built {
    let mut out = Built { modules: Vec::new(), member: Vec::new(), drawers: Vec::new() };
    let add = |out: &mut Built, sect: u8, name: &str, member: Option<(usize, bool)>| {
        if let Some(m) = modules::make(name, fmt::Conf(bar.modules[name].clone()), ctx) {
            out.modules.push((sect, m));
            out.member.push(member);
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
    fn create_layer(&mut self) {
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, Layer::Top, Some("empalme"), None);
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
        self.layer = Some(layer);
        self.configured = false;
        self.frame_pending = false;
        // el inhibidor iba con la superficie de antes
        if let Some(i) = self.inhibitor.take() {
            i.destroy();
        }
        self.sync_idle();
    }

    /// Que el compositor no apague la pantalla mientras el módulo idle lo pida.
    fn sync_idle(&mut self) {
        match (self.idle.get(), &self.inhibitor, &self.idle_mgr, &self.layer) {
            (true, None, Some(mgr), Some(layer)) => {
                self.inhibitor = Some(mgr.create_inhibitor(layer.wl_surface(), &self.qh, ()));
                layer.commit();
            }
            (false, Some(_), ..) => {
                if let Some(i) = self.inhibitor.take() {
                    i.destroy();
                }
            }
            _ => {}
        }
    }

    fn redraw(&mut self) {
        if self.configured {
            self.draw();
        }
    }

    fn draw(&mut self) {
        let Some(layer) = self.layer.clone() else { return };
        if self.size.0 == 0 {
            return;
        }
        let s = self.scale.max(1);
        let (w, h) = (self.size.0 as i32 * s, self.size.1 as i32 * s);
        let Some(mut pix) = tiny_skia::Pixmap::new(w as u32, h as u32) else { return };
        let (cw, ch) = (self.painter.cw, self.painter.ch);
        let (pad, gap) = (self.bar.pad, self.bar.gap as f32 * cw);

        // qué muestra cada módulo y cuánto mide
        let lines: Vec<_> = self.modules.iter().map(|(sect, m)| (*sect, m.line(&self.look))).collect();
        let full: Vec<Option<u16>> = lines.iter().map(|(_, l)| l.as_ref().map(|l| l.width() as u16 + 2 * pad)).collect();
        // lo que se ve de cada uno (los que salen de un cajón, a medio abrir):
        // (visible, escondido a la izquierda), en pixeles
        let mut shown: Vec<(f32, f32)> = full.iter().map(|w| (w.unwrap_or(0) as f32 * cw, 0.0)).collect();
        for (g, d) in self.drawers.iter().enumerate() {
            let rest: Vec<usize> = (0..full.len()).filter(|&i| self.member[i] == Some((g, false)) && full[i].is_some()).collect();
            let widths: Vec<f32> = rest.iter().map(|&i| shown[i].0).collect();
            for (k, v) in drawer::reveal(d, &widths, gap).into_iter().enumerate() {
                shown[rest[k]] = v;
            }
        }
        let bw = self.size.0 as f32;
        self.slots.clear();
        for sect in 0..3u8 {
            let items: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].0 == sect && full[i].is_some() && shown[i].0 > 0.0).collect();
            let total: f32 = items.iter().map(|&i| shown[i].0).sum::<f32>() + gap * items.len().saturating_sub(1) as f32;
            let mut x = match sect {
                0 => 0.0,
                1 => ((bw - total) / 2.0).max(0.0),
                _ => (bw - total).max(0.0),
            };
            for i in items {
                let (width, clip) = shown[i];
                self.slots.push(Slot { module: i, start: x, width, clip, full: full[i].unwrap() });
                x += width + gap;
            }
        }

        // los fondos: una isla por módulo, o una sola pieza (en pixeles del
        // monitor, redondeados para que el texto quede nítido)
        let t = self.look.theme;
        let alpha = self.look.alpha();
        let sf = s as f32;
        let r = self.bar.radius * sf;
        let px = |v: f32| (v * sf).round();
        if self.bar.style == Style::Solid {
            rounded(&mut pix, 0.0, 0.0, w as f32, h as f32, r, t.base, alpha);
        } else {
            for sl in &self.slots {
                rounded(&mut pix, px(sl.start), 0.0, px(sl.start + sl.width) - px(sl.start), h as f32, r, t.base, alpha);
            }
        }

        // el texto, centrado de alto en la barra; a medio abrir, en su propio
        // cuadro corrido a la izquierda lo escondido, y solo lo que se ve
        let oy = ((h as f32 - ch * sf) / 2.0).floor();
        for sl in &self.slots {
            let Some(line) = &lines[sl.module].1 else { continue };
            let mut one = Buffer::empty(Rect::new(0, 0, sl.full, 1));
            one.set_line(pad, 0, line, sl.full.saturating_sub(2 * pad));
            if sl.width >= sl.full as f32 * cw - 0.01 {
                self.painter.cells(&mut pix, &one, &self.look, px(sl.start), oy, sf);
                continue;
            }
            let vis = (px(sl.start + sl.width) - px(sl.start)) as u32;
            let Some(mut part) = tiny_skia::Pixmap::new(vis.max(1), h as u32) else { continue };
            self.painter.cells(&mut part, &one, &self.look, -px(sl.clip), oy, sf);
            pix.draw_pixmap(px(sl.start) as i32, 0, part.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        }

        let pool = match &mut self.pool {
            Some(p) => p,
            None => self.pool.insert(SlotPool::new((w * h * 4) as usize, &self.shm).expect("memoria compartida")),
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
        if self.drawers_moving() && !self.frame_pending {
            surface.frame(&self.qh, FrameCallbackData(surface.clone()));
            self.frame_pending = true;
        }
        if buffer.attach_to(surface).is_ok() {
            layer.commit();
        }
    }

    /// Un clic o la rueda en x (pixeles): al módulo que esté ahí.
    fn act(&mut self, act: Act) {
        let x = self.pointer_x as f32;
        let Some(sl) = self.slots.iter().find(|s| x >= s.start && x < s.start + s.width) else { return };
        let rel = ((x - sl.start + sl.clip) / self.painter.cw) as u16;
        let rel = rel.saturating_sub(self.bar.pad);
        let i = sl.module;
        self.modules[i].1.act(act, rel);
        self.sync_idle();
        // lo que cambia solo con el clic (el reloj largo): de una vez
        self.draw();
    }

    /// El mouse en x (pixeles; None: se fue de la barra): abrir el cajón que
    /// tenga debajo y cerrar los demás.
    fn hover(&mut self, x: Option<f64>) {
        let g = x.and_then(|x| {
            let x = x as f32;
            let sl = self.slots.iter().find(|s| x >= s.start && x < s.start + s.width)?;
            self.member[sl.module].map(|m| m.0)
        });
        let was = self.drawers_moving();
        for (i, d) in self.drawers.iter_mut().enumerate() {
            d.open = g == Some(i);
        }
        // empezó a moverse: el primer cuadro ya, los demás con el compositor
        if !was && self.drawers_moving() && !self.frame_pending {
            self.frame();
        }
    }

    /// ¿Algún cajón se está abriendo o cerrando? Esos van al ritmo de la
    /// pantalla, con lo que avisa el compositor en cada cuadro.
    fn drawers_moving(&self) -> bool {
        !self.hidden && self.drawers.iter().any(Drawer::moving)
    }

    /// ¿Algún módulo se mueve solo (el grabador pulsando)? Ese con reloj.
    fn pulsing(&self) -> bool {
        !self.hidden && self.configured && self.modules.iter().any(|(_, m)| m.animating())
    }

    /// Un cuadro de los cajones: avanzar lo que pasó desde el anterior y
    /// redibujar (draw pide el siguiente si siguen moviéndose).
    fn frame(&mut self) {
        let now = Instant::now();
        let dt = self.last_frame.map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(0.1));
        self.last_frame = Some(now);
        for d in &mut self.drawers {
            d.step(dt);
        }
        if !self.drawers_moving() {
            self.last_frame = None;
        }
        self.redraw();
    }

    fn on_ev(&mut self, ev: Ev) {
        match ev {
            Ev::Toggle => {
                self.hidden = !self.hidden;
                if self.hidden {
                    if let Some(i) = self.inhibitor.take() {
                        i.destroy();
                    }
                    self.layer = None;
                    self.pool = None;
                } else {
                    self.create_layer();
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
            self.redraw();
        }
    }

    /// Otro look u otra barra: releer y volver a armar.
    fn reload(&mut self, new_bar: bool) {
        if new_bar {
            self.bar = bar::load();
            self.painter = Painter::sized(self.bar.font_size);
            let b = build(&self.bar, &Ctx { idle: self.idle.clone() });
            (self.modules, self.member, self.drawers) = (b.modules, b.member, b.drawers);
            if self.hidden {
                return;
            }
            self.layer = None;
            self.pool = None;
            self.create_layer();
        } else {
            self.painter = Painter::sized(self.bar.font_size);
            self.redraw();
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
    signals::watch(tx);
    // los nombres de días y meses del reloj, en el idioma del sistema
    unsafe { libc::setlocale(libc::LC_TIME, c"".as_ptr()) };

    let bar = bar::load();
    let idle = Rc::new(Cell::new(false));
    let ctx = Ctx { idle: idle.clone() };
    let built = build(&bar, &ctx);
    let mut app = App {
        registry: RegistryState::new(&globals),
        seat: SeatState::new(&globals, &qh),
        output: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh).map_err(|_| "el compositor no tiene wl_compositor")?,
        layer_shell: LayerShell::bind(&globals, &qh).map_err(|_| "el compositor no tiene layer-shell")?,
        shm: Shm::bind(&globals, &qh).map_err(|_| "el compositor no tiene wl_shm")?,
        pool: None,
        layer: None,
        configured: false,
        size: (0, 0),
        scale: 1,
        pointer: None,
        pointer_x: 0.0,
        scroll: 0.0,
        look: Look::load("empalme"),
        painter: Painter::sized(bar.font_size),
        modules: built.modules,
        member: built.member,
        drawers: built.drawers,
        last_frame: None,
        frame_pending: false,
        last_pulse: Instant::now(),
        bar,
        slots: Vec::new(),
        idle,
        hidden: false,
        idle_mgr: globals.bind::<ZwpIdleInhibitManagerV1, _, _>(&qh, 1..=1, ()).ok(),
        inhibitor: None,
        qh,
    };
    app.create_layer();

    let mut last_sec = 0;
    let mut bar_stamp = bar::stamp();
    fmt::load_palette();
    let mut palette_stamp = fmt::palette_stamp();
    loop {
        // despertar justo al cambiar el segundo, para que el reloj no se salte uno
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let mut wait = Duration::from_millis(1000 - now.subsec_millis() as u64 + 2);
        const PULSE: Duration = Duration::from_millis(33);
        if app.pulsing() {
            wait = wait.min(PULSE.saturating_sub(app.last_pulse.elapsed()));
        }
        event_loop.dispatch(wait, &mut app).map_err(|e| e.to_string())?;
        // el pulso del grabador (si los cajones se mueven, ya se redibuja con ellos)
        if app.pulsing() && app.last_pulse.elapsed() >= PULSE {
            app.last_pulse = Instant::now();
            if !app.frame_pending {
                app.redraw();
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
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, factor: i32) {
        if factor != self.scale {
            self.scale = factor;
            self.pool = None;
            self.redraw();
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_pending = false;
        if self.drawers_moving() {
            self.frame();
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.layer = None;
        self.configured = false;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, cfg: LayerSurfaceConfigure, _: u32) {
        if self.layer.as_ref() != Some(layer) {
            return;
        }
        if cfg.new_size.0 > 0 && cfg.new_size.1 > 0 {
            if cfg.new_size != self.size {
                self.pool = None;
            }
            self.size = cfg.new_size;
        }
        self.configured = true;
        self.draw();
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
            if self.layer.as_ref().is_none_or(|l| &e.surface != l.wl_surface()) {
                continue;
            }
            self.pointer_x = e.position.0;
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => self.hover(Some(e.position.0)),
                PointerEventKind::Press { button, .. } => match button {
                    BTN_LEFT => self.act(Act::Left),
                    BTN_RIGHT => self.act(Act::Right),
                    BTN_MIDDLE => self.act(Act::Middle),
                    _ => {}
                },
                PointerEventKind::Axis { vertical, .. } => {
                    // un paso de rueda (o 15 pixeles de touchpad) = una vez
                    let step = if vertical.discrete != 0 { vertical.discrete as f64 * 15.0 } else { vertical.absolute };
                    self.scroll += step;
                    while self.scroll.abs() >= 15.0 {
                        let down = self.scroll > 0.0;
                        self.scroll -= 15.0f64.copysign(self.scroll);
                        self.act(if down { Act::Down } else { Act::Up });
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.scroll = 0.0;
                    self.hover(None);
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
