//! La batería (solo en laptops: sin batería del equipo no aparece). La misma
//! cuenta que Battery.py: todas las baterías juntas, las de mouse y audífonos
//! no. Clic: el siguiente perfil de energía; clic derecho: el ahorro.

use super::{Act, Ev, Module, script};
use discada_kit::look::Look;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use std::path::{Path, PathBuf};
use std::time::Instant;

const PS: &str = "/sys/class/power_supply";

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn num(p: PathBuf) -> u64 {
    read(&p).parse().unwrap_or(0)
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Charging,
    Full,
    Discharging,
}

pub struct Battery {
    now: Option<(u8, State)>,
    last: Instant,
}

/// (por ciento, estado) juntando las baterías del equipo, o None si no hay.
fn info() -> Option<(u8, State)> {
    let mut now = 0u64;
    let mut full = 0u64;
    let mut statuses = Vec::new();
    let mut ac = false;
    for e in std::fs::read_dir(PS).ok()?.flatten() {
        let d = e.path();
        match read(&d.join("type")).as_str() {
            "Mains" | "USB" => ac |= read(&d.join("online")) == "1",
            "Battery" if read(&d.join("scope")) != "Device" => {
                let kind = if d.join("energy_now").exists() { "energy" } else { "charge" };
                now += num(d.join(format!("{kind}_now")));
                full += num(d.join(format!("{kind}_full")));
                statuses.push(read(&d.join("status")));
            }
            _ => {}
        }
    }
    if statuses.is_empty() {
        return None;
    }
    let pct = (now * 100).checked_div(full).map_or(0, |p| p.min(100) as u8);
    let state = if statuses.iter().any(|s| s == "Charging") {
        State::Charging
    } else if statuses.iter().any(|s| s == "Discharging") && !ac {
        State::Discharging
    } else {
        State::Full
    };
    Some((pct, state))
}

impl Battery {
    pub fn new() -> Battery {
        Battery { now: info(), last: Instant::now() }
    }
}

impl Module for Battery {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let (pct, state) = self.now?;
        let t = look.theme;
        let color = match state {
            State::Discharging if pct <= 20 => t.red,
            State::Charging => t.green,
            _ => t.fg,
        };
        let label = if look.icons {
            // md-battery_10 … md-battery, y los de cargando
            let dis = ['\u{f007a}', '\u{f007b}', '\u{f007c}', '\u{f007d}', '\u{f007e}', '\u{f007f}', '\u{f0080}', '\u{f0081}', '\u{f0082}', '\u{f0079}'];
            let chg = ['\u{f089c}', '\u{f0086}', '\u{f0087}', '\u{f0088}', '\u{f089d}', '\u{f0089}', '\u{f089e}', '\u{f008a}', '\u{f008b}', '\u{f0085}'];
            let i = ((pct as usize).saturating_sub(1) / 10).min(9);
            match state {
                State::Charging => chg[i].to_string(),
                State::Full => "\u{f06a5}".to_string(),
                State::Discharging => dis[i].to_string(),
            }
        } else {
            look.tx(match state {
                State::Charging => "bat+",
                State::Full => "ac",
                State::Discharging => "bat",
            })
        };
        let mut st = Style::new().fg(color);
        if state == State::Discharging && pct <= 10 {
            st = st.bold();
        }
        Some(Line::from(vec![Span::styled(label, st), Span::styled(format!(" {pct}%"), Style::new().fg(t.fg))]))
    }

    fn act(&mut self, act: Act, _: u16) {
        match act {
            Act::Left => script("Battery.py", &["profile"]),
            Act::Right => script("Battery.py", &["saver"]),
            _ => {}
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        // Battery.py avisa con RTMIN+11 al cambiar de perfil o de ahorro
        let now_please = matches!(ev, Ev::Signal(11));
        if !now_please && (!matches!(ev, Ev::Tick) || self.last.elapsed().as_secs() < 20) {
            return false;
        }
        self.last = Instant::now();
        let now = info();
        let changed = now != self.now;
        self.now = now;
        changed
    }
}
