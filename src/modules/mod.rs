//! Los módulos de la barra. Cada uno sabe qué mostrar (una línea de ratatui,
//! con el look) y qué hacer con un clic o la rueda; para casi todo llama a los
//! scripts de discada, igual que lo hacía waybar.

pub mod app;
pub mod audio;
pub mod battery;
pub mod bluetooth;
pub mod clock;
pub mod idle;
pub mod network;
pub mod notifications;
pub mod power;
pub mod recorder;
pub mod system;
pub mod text;
pub mod visualizer;
pub mod workspaces;

use crate::fmt::{Conf, Val, fill, line};
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, PartialEq)]
pub enum Act {
    Left,
    Middle,
    Right,
    Up,
    Down,
}

/// Algo cambió afuera: a quién le toca releer.
pub enum Ev {
    Hypr(String),
    Audio,
    Bluetooth(bluetooth::State),
    Notifications(notifications::State),
    /// cambió lo que suena (playerctl)
    Player(visualizer::Player),
    /// un cuadro de cava
    Viz(Vec<u32>),
    /// `pkill -RTMIN+N -x empalme`: N
    Signal(i32),
    /// SIGUSR1: esconder o mostrar (la barra, no los módulos)
    Toggle,
    /// SIGUSR2: releer el look (la barra)
    Reload,
    /// una vez por segundo, justo al cambiar el segundo
    Tick,
}

/// Un botón dentro de un módulo: (inicio, ancho, ¿es el activo?), en celdas.
pub type Part = (u16, u16, bool);

pub trait Module {
    /// Lo que se ve; None: el módulo no aparece (sin batería, por ejemplo).
    fn line(&self, look: &Look) -> Option<Line<'static>>;
    /// Lo que se ve en la barra del monitor `out` (los workspaces marcan el
    /// activo de cada monitor); los demás se ven igual en todos.
    fn line_on(&self, look: &Look, _out: &str) -> Option<Line<'static>> {
        self.line(look)
    }
    /// Un clic o la rueda en la celda `col` del módulo (desde su texto).
    fn act(&mut self, act: Act, col: u16);
    /// El clic en la barra del monitor `out`.
    fn act_on(&mut self, act: Act, col: u16, _out: &str) {
        self.act(act, col)
    }
    /// ¿Hay que redibujar?
    fn update(&mut self, ev: &Ev) -> bool;
    /// Si el módulo tiene botones por dentro (los workspaces): el que está en
    /// la celda `col` (desde su texto), como (inicio, ancho, ¿es el activo?).
    /// None: el hover es de todo el módulo.
    fn part(&self, _col: u16) -> Option<Part> {
        None
    }
    fn part_on(&self, col: u16, _out: &str) -> Option<Part> {
        self.part(col)
    }
    /// El tooltip: marcado como el de los formatos, un renglón por `\n`.
    /// None: sin tooltip.
    fn tooltip(&self, _look: &Look) -> Option<String> {
        None
    }
    /// Una línea abajo del módulo, de este color (la app enfocada).
    fn underline(&self, _look: &Look) -> Option<ratatui::style::Color> {
        None
    }
    /// Los botones de adentro, cada uno con su texto y si es el activo (los
    /// workspaces): la barra los acomoda con su propio relleno en pixeles
    /// (`button-padding`, `active-padding` del look).
    fn buttons(&self, _look: &Look, _out: &str) -> Option<Vec<(Line<'static>, bool)>> {
        None
    }
    /// Un clic o la rueda en el botón `n` de `buttons` (None: entre botones).
    fn act_button(&mut self, act: Act, _n: Option<usize>, out: &str) {
        self.act_on(act, 0, out)
    }
    /// El botón activo de adentro en el monitor `out` (el workspace activo),
    /// para marcarlo con la línea del look.
    fn active_part(&self, _out: &str) -> Option<Part> {
        None
    }
    /// ¿Se está moviendo (el grabador pulsando)? La barra lo redibuja seguido.
    fn animating(&self) -> bool {
        false
    }
}

/// El módulo `name` (`cpu`, o `cpu#otro` para tener dos con distinta
/// config) con lo que diga su `[bar.modules.<name>]`.
/// Lo que algunos módulos comparten con la barra.
pub struct Ctx {
    /// ¿que la pantalla no se apague? (idle)
    pub idle: std::rc::Rc<std::cell::Cell<bool>>,
    /// para los módulos que lanzan sus propios hilos (el visualizador)
    pub tx: smithay_client_toolkit::reexports::calloop::channel::Sender<Ev>,
}

pub fn make(name: &str, conf: Conf, ctx: &Ctx) -> Option<Box<dyn Module>> {
    let (kind, tag) = name.split_once('#').unwrap_or((name, ""));
    Some(match kind {
        "app" => Box::new(app::App::new(tag, conf)),
        "workspaces" => Box::new(workspaces::Workspaces::new(conf)),
        "clock" => Box::new(clock::Clock::new(conf)),
        "battery" => Box::new(battery::Battery::new(conf)),
        "audio" => Box::new(audio::Audio::new(audio::Kind::Sink, conf)),
        "mic" => Box::new(audio::Audio::new(audio::Kind::Source, conf)),
        "cpu" => Box::new(system::System::new(system::Kind::Cpu, conf)),
        "memory" => Box::new(system::System::new(system::Kind::Memory, conf)),
        "disk" => Box::new(system::System::new(system::Kind::Disk, conf)),
        "temperature" => Box::new(system::System::new(system::Kind::Temperature, conf)),
        "network" => Box::new(network::Network::new(conf)),
        "bluetooth" => Box::new(bluetooth::Bluetooth::new(conf)),
        "power" => Box::new(power::Power::new(conf)),
        "text" => Box::new(text::Text::new(conf)),
        "visualizer" => Box::new(visualizer::Visualizer::new(conf, ctx.tx.clone())),
        "recorder" => Box::new(recorder::Recorder::new(conf)),
        "notifications" => Box::new(notifications::Notifications::new(conf)),
        "idle" | "idle_inhibitor" => Box::new(idle::Idle::new(conf, ctx.idle.clone())),
        _ => return None,
    })
}

/// El formato de un módulo: el del look, o el de empalme (con íconos o con
/// texto, según el look; al texto le toca el caso del look, no a los `{…}`).
pub fn template(look: &Look, conf: &Conf, state: &str, icons: &str, text: &str) -> String {
    if let Some(f) = conf.format(state) {
        return f;
    }
    if look.icons {
        return icons.to_string();
    }
    let mut out = String::new();
    let mut inside = false;
    for c in text.chars() {
        match c {
            '{' => inside = true,
            '}' => inside = false,
            _ => {}
        }
        if inside || c == '}' { out.push(c) } else { out.push_str(&look.tx(&c.to_string())) }
    }
    out
}

/// Relleno y como renglón; vacío: el módulo no se ve (como en waybar).
pub fn show(look: &Look, tpl: &str, vals: &[(&str, Val)], base: Style) -> Option<Line<'static>> {
    let s = fill(tpl, vals);
    if s.trim().is_empty() { None } else { Some(line(&s, base, look)) }
}

/// Un script de discada (hypr/scripts) sin esperarlo.
pub fn script(name: &str, args: &[&str]) {
    let path = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/hypr/scripts").join(name);
    spawn(Command::new(path).args(args));
}

pub fn spawn(cmd: &mut Command) {
    if let Ok(mut child) = crate::signals::clean(cmd).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        // que no quede zombi
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Idioma de discada (hypr/discada.conf → LANGUAGE=es|en): el texto que toca.
pub fn l(es: &'static str, en: &'static str) -> &'static str {
    let f = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/hypr/discada.conf");
    let spanish = std::fs::read_to_string(f)
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("LANGUAGE=").map(|v| v.trim_start().starts_with("es"))))
        .unwrap_or(false);
    if spanish { es } else { en }
}
