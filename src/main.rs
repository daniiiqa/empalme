//! empalme — la barra de discada. Se dibuja como las apps del rice (rola, vato,
//! arre, taco): celdas con la fuente de kitty y los colores y el estilo del
//! look; cómo es la barra en cada look (arriba o abajo, alto, islas o una sola
//! pieza, qué módulos van dónde) sale del `[bar]` de su look.toml.
//!
//!   empalme            la barra (en el monitor que diga Hyprland)
//!
//! Por ahora corre junto a waybar: `pkill -SIGUSR1 waybar` esconde waybar.

mod bar;
mod fmt;
mod hypr;
mod modules;

use bar::{Bar, Style};
use discada_kit::look::Look;
use discada_kit::paint::{Painter, rounded};
use modules::{Act, Ev, Module};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel::{Event, channel};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// Dónde quedó un módulo, en celdas.
struct Slot {
    module: usize,
    start: u16,
    width: u16,
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
    slots: Vec<Slot>,
    qh: QueueHandle<App>,
}

fn build(bar: &Bar) -> Vec<(u8, Box<dyn Module>)> {
    let mut out = Vec::new();
    for (sect, names) in [&bar.left, &bar.center, &bar.right].into_iter().enumerate() {
        for name in names {
            if let Some(m) = modules::make(name, fmt::Conf(bar.modules[name.as_str()].clone())) {
                out.push((sect as u8, m));
            }
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
        let cols = (self.size.0 as f32 / cw).floor() as u16;
        let (pad, gap) = (self.bar.pad, self.bar.gap);

        // qué muestra cada módulo y cuánto mide
        let lines: Vec<_> = self.modules.iter().map(|(sect, m)| (*sect, m.line(&self.look))).collect();
        self.slots.clear();
        for sect in 0..3u8 {
            let items: Vec<(usize, u16)> = lines
                .iter()
                .enumerate()
                .filter(|(_, (s, l))| *s == sect && l.is_some())
                .map(|(i, (_, l))| (i, l.as_ref().unwrap().width() as u16 + 2 * pad))
                .collect();
            let total: u16 = items.iter().map(|x| x.1).sum::<u16>() + gap * items.len().saturating_sub(1) as u16;
            let mut x = match sect {
                0 => 0,
                1 => cols.saturating_sub(total) / 2,
                _ => cols.saturating_sub(total),
            };
            for (i, width) in items {
                self.slots.push(Slot { module: i, start: x, width });
                x += width + gap;
            }
        }

        // los fondos: una isla por módulo, o una sola pieza
        let t = self.look.theme;
        let alpha = self.look.alpha();
        let r = self.bar.radius * s as f32;
        let (sw, sh) = (cw * s as f32, h as f32);
        if self.bar.style == Style::Solid {
            rounded(&mut pix, 0.0, 0.0, w as f32, sh, r, t.base, alpha);
        } else {
            for sl in &self.slots {
                rounded(&mut pix, sl.start as f32 * sw, 0.0, sl.width as f32 * sw, sh, r, t.base, alpha);
            }
        }

        // el texto, centrado de alto en la barra
        let mut buf = Buffer::empty(Rect::new(0, 0, cols, 1));
        for sl in &self.slots {
            if let Some(line) = &lines[sl.module].1 {
                buf.set_line(sl.start + pad, 0, line, sl.width.saturating_sub(2 * pad));
            }
        }
        let oy = ((h as f32 - ch * s as f32) / 2.0).floor();
        self.painter.cells(&mut pix, &buf, &self.look, 0.0, oy, s as f32);

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
        if buffer.attach_to(surface).is_ok() {
            layer.commit();
        }
    }

    /// Un clic o la rueda en x (pixeles): al módulo que esté ahí.
    fn act(&mut self, act: Act) {
        let col = (self.pointer_x / self.painter.cw as f64) as u16;
        let Some(sl) = self.slots.iter().find(|s| col >= s.start && col < s.start + s.width) else { return };
        let rel = col.saturating_sub(sl.start + self.bar.pad);
        let i = sl.module;
        self.modules[i].1.act(act, rel);
        // lo que cambia solo con el clic (el reloj largo): de una vez
        self.draw();
    }

    fn on_ev(&mut self, ev: Ev) {
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
            self.modules = build(&self.bar);
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
    modules::bluetooth::watch(tx);
    // los nombres de días y meses del reloj, en el idioma del sistema
    unsafe { libc::setlocale(libc::LC_TIME, c"".as_ptr()) };

    let bar = bar::load();
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
        modules: build(&bar),
        bar,
        slots: Vec::new(),
        qh,
    };
    app.create_layer();

    let mut last_sec = 0;
    let mut bar_stamp = bar::stamp();
    loop {
        // despertar justo al cambiar el segundo, para que el reloj no se salte uno
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let wait = Duration::from_millis(1000 - now.subsec_millis() as u64 + 2);
        event_loop.dispatch(wait, &mut app).map_err(|e| e.to_string())?;
        let sec = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        if sec != last_sec {
            last_sec = sec;
            app.on_ev(Ev::Tick);
            let look_changed = app.look.refresh();
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
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
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
                PointerEventKind::Leave { .. } => self.scroll = 0.0,
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
