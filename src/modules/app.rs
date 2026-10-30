//! Las apps abiertas, de AppStrip.py: su daemon escribe un archivo por lugar
//! en $XDG_RUNTIME_DIR/appstrip (i1…i8 en la barra, more y d1…d8 en el
//! cajón) y avisa con RTMIN+9; cada `app#<lugar>` lee el suyo. La enfocada
//! lleva una línea abajo, la fijada y cerrada va tenue. Clic: enfocarla (o
//! abrirla); medio: cerrarla. `color` del look: el de los íconos.

use super::{Act, Ev, Module, script, show};
use crate::fmt::{Conf, color, mix};
use disco::look::Look;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, PartialEq, Default)]
struct Now {
    text: String,
    /// el de AppStrip.py (ya con marcado)
    tooltip: String,
    active: bool,
    closed: bool,
}

pub struct App {
    slot: String,
    conf: Conf,
    now: Now,
}

fn read(slot: &str) -> Now {
    let path = PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())).join("appstrip").join(slot);
    let Some(v) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
        return Now::default();
    };
    let has = |c: &str| v["class"].as_array().is_some_and(|a| a.iter().any(|x| x.as_str() == Some(c)));
    Now {
        text: v["text"].as_str().unwrap_or("").to_string(),
        tooltip: v["tooltip"].as_str().unwrap_or("").to_string(),
        active: has("active"),
        closed: has("closed"),
    }
}

impl App {
    pub fn new(slot: &str, conf: Conf) -> App {
        App { slot: slot.to_string(), conf, now: read(slot) }
    }

    fn fg(&self, look: &Look) -> Color {
        self.conf.str("color").and_then(|c| color(look, &c)).unwrap_or(look.theme.fg)
    }
}

impl Module for App {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        if self.now.text.is_empty() {
            return None;
        }
        let mut fg = self.fg(look);
        if self.now.closed {
            // opacity: 0.45 del CSS de los looks
            fg = color(look, &mix(fg, look.theme.base, 0.45)).unwrap_or(fg);
        }
        let text = self.now.text.replace('&', "&amp;").replace('<', "&lt;");
        show(look, &text, &[], Style::new().fg(fg))
    }

    fn opacity(&self) -> f32 {
        if self.now.closed { 0.45 } else { 1.0 }
    }

    fn tooltip(&self, _: &Look) -> Option<String> {
        (self.conf.0["tooltip"].as_bool() != Some(false)).then(|| self.now.tooltip.clone())
    }

    fn underline(&self, look: &Look) -> Option<Color> {
        self.now.active.then(|| self.fg(look))
    }

    fn act(&mut self, act: Act, _: u16) {
        if self.slot == "more" {
            return;
        }
        match act {
            Act::Left => script("AppStrip.py", &["focus", &self.slot]),
            Act::Middle => script("AppStrip.py", &["close", &self.slot]),
            _ => {}
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        // RTMIN+9 de AppStrip.py; cada segundo por si no llegó (un daemon
        // viejo que solo le avisa a waybar)
        if !matches!(ev, Ev::Signal(9) | Ev::Tick) {
            return false;
        }
        let now = read(&self.slot);
        let changed = now != self.now;
        self.now = now;
        changed
    }
}
