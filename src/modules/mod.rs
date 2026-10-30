//! Los módulos de la barra. Cada uno sabe qué mostrar (una línea de ratatui,
//! con el look) y qué hacer con un clic o la rueda; para casi todo llama a los
//! scripts de discada, igual que lo hacía waybar.

pub mod audio;
pub mod battery;
pub mod clock;
pub mod workspaces;

use discada_kit::look::Look;
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

pub fn make(name: &str) -> Option<Box<dyn Module>> {
    Some(match name {
        "workspaces" => Box::new(workspaces::Workspaces::new()),
        "clock" => Box::new(clock::Clock::new()),
        "battery" => Box::new(battery::Battery::new()),
        "audio" => Box::new(audio::Audio::new()),
        _ => return None,
    })
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

/// Idioma de discada (hypr/discada.conf → LANGUAGE=es|en).
pub fn spanish() -> bool {
    let f = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/hypr/discada.conf");
    std::fs::read_to_string(f)
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("LANGUAGE=").map(|v| v.trim_start().starts_with("es"))))
        .unwrap_or(false)
}
