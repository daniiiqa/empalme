//! El grabador (Recorder.sh): cuánto lleva grabando, si está en pausa y si
//! el replay está prendido. Lo lee del estado que deja Recorder.sh en
//! $XDG_RUNTIME_DIR/recorder (sin correr `Recorder.sh status` cada segundo,
//! como hacía waybar). Las etiquetas son del look: `rec`, `paused`, `replay`.
//! Clic: detener (o guardar el replay); medio: pausa; derecho: el menú.

use super::{Act, Ev, Module, script, show};
use crate::fmt::Conf;
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, PartialEq, Default)]
struct Now {
    /// segundos grabados (sin las pausas), si está grabando
    rec: Option<u64>,
    paused: bool,
    replay: bool,
}

pub struct Recorder {
    conf: Conf,
    now: Now,
}

fn dir() -> PathBuf {
    PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())).join("recorder")
}

fn num(name: &str) -> Option<u64> {
    std::fs::read_to_string(dir().join(name)).ok()?.trim().parse().ok()
}

/// Como `alive` de Recorder.sh: hay pid y el proceso vive.
fn alive(kind: &str) -> bool {
    num(&format!("{kind}.pid")).is_some_and(|pid| unsafe { libc::kill(pid as i32, 0) } == 0)
}

fn read() -> Now {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut out = Now { replay: alive("replay"), ..Default::default() };
    if alive("rec") {
        let start = num("rec.start").unwrap_or(now);
        let mut t = now.saturating_sub(start).saturating_sub(num("rec.pausedtotal").unwrap_or(0));
        if let Some(p) = num("rec.paused") {
            t = t.saturating_sub(now.saturating_sub(p));
            out.paused = true;
        }
        out.rec = Some(t);
    }
    out
}

impl Recorder {
    pub fn new(conf: Conf) -> Recorder {
        Recorder { conf, now: read() }
    }

    fn label(&self, look: &Look, key: &str, icon: &str, text: &str) -> String {
        self.conf.str(key).unwrap_or_else(|| if look.icons { icon.to_string() } else { look.tx(text) })
    }
}

impl Module for Recorder {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let t = look.theme;
        let n = &self.now;
        let mut parts = Vec::new();
        if let Some(secs) = n.rec {
            let (key, icon, text, color) = if n.paused {
                ("paused", "\u{f03e5}", "[||]", "muted")
            } else {
                ("rec", "\u{f044b}", "[rec]", "red")
            };
            let label = self.label(look, key, icon, text);
            parts.push(format!("<span color=\"{color}\">{label}</span> {:02}:{:02}", secs / 60, secs % 60));
        }
        if n.replay {
            parts.push(self.label(look, "replay", "\u{f0459}", "[rpl]"));
        }
        show(look, &parts.join("  "), &[], Style::new().fg(t.fg))
    }

    fn act(&mut self, act: Act, _: u16) {
        match act {
            Act::Left => script("Recorder.sh", &["click"]),
            Act::Middle => script("Recorder.sh", &["pause"]),
            Act::Right => script("Recorder.sh", &["menu"]),
            _ => {}
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        if !matches!(ev, Ev::Tick | Ev::Signal(10)) {
            return false;
        }
        let now = read();
        let changed = now != self.now;
        self.now = now;
        changed
    }
}
