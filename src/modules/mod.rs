//! Los módulos de la barra. Cada uno sabe qué mostrar (una línea de ratatui,
//! con el look) y qué hacer con un clic o la rueda; para casi todo llama a los
//! scripts de discada, igual que lo hacía waybar.

pub mod audio;
pub mod battery;
pub mod bluetooth;
pub mod clock;
pub mod network;
pub mod power;
pub mod system;
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
    /// una vez por segundo, justo al cambiar el segundo
    Tick,
}

pub trait Module {
    /// Lo que se ve; None: el módulo no aparece (sin batería, por ejemplo).
    fn line(&self, look: &Look) -> Option<Line<'static>>;
    /// Un clic o la rueda en la celda `col` del módulo (desde su texto).
    fn act(&mut self, act: Act, col: u16);
    /// ¿Hay que redibujar?
    fn update(&mut self, ev: &Ev) -> bool;
}

/// El módulo `name` (`cpu`, o `cpu#otro` para tener dos con distinta
/// config) con lo que diga su `[bar.modules.<name>]`.
pub fn make(name: &str, conf: Conf) -> Option<Box<dyn Module>> {
    let kind = name.split('#').next().unwrap_or(name);
    Some(match kind {
        "workspaces" => Box::new(workspaces::Workspaces::new()),
        "clock" => Box::new(clock::Clock::new(conf)),
        "battery" => Box::new(battery::Battery::new()),
        "audio" => Box::new(audio::Audio::new(audio::Kind::Sink, conf)),
        "mic" => Box::new(audio::Audio::new(audio::Kind::Source, conf)),
        "cpu" => Box::new(system::System::new(system::Kind::Cpu, conf)),
        "memory" => Box::new(system::System::new(system::Kind::Memory, conf)),
        "disk" => Box::new(system::System::new(system::Kind::Disk, conf)),
        "temperature" => Box::new(system::System::new(system::Kind::Temperature, conf)),
        "network" => Box::new(network::Network::new(conf)),
        "bluetooth" => Box::new(bluetooth::Bluetooth::new(conf)),
        "power" => Box::new(power::Power::new(conf)),
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
    if let Ok(mut child) = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        // que no quede zombi
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}
