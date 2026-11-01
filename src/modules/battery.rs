//! La batería (solo en laptops: sin batería del equipo no aparece). La misma
//! cuenta que Battery.py: todas las baterías juntas, las de mouse y audífonos
//! no. Clic: el siguiente perfil de energía; clic derecho: el ahorro.
//! Con el `format` y los `format-icons` del look (por estado: discharging,
//! charging, full y saver, como el `alt` de Battery.py), y los colores de sus
//! clases `.low` y `.critical` (`low-color`, `critical-color`).

use super::{Act, Ev, Module, script, show};
use crate::fmt::{Conf, Val, color, mix, named, pick};
use disco::look::Look;
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
    conf: Conf,
    now: Option<(u8, State)>,
    last: Instant,
    /// el tooltip de `Battery.py status` (perfil, ahorro, tiempo que queda),
    /// que se lee en otro hilo
    tip: Arc<Mutex<(String, bool)>>,
    tip_last: Option<Instant>,
}

/// Pedirle el tooltip a Battery.py sin esperarlo.
/// También dice si está el ahorro (la clase `saver`).
fn ask_tooltip(tip: Arc<Mutex<(String, bool)>>) {
    std::thread::spawn(move || {
        let path = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/hypr/scripts/Battery.py");
        let out = crate::signals::clean(&mut std::process::Command::new(path)).arg("status").stderr(std::process::Stdio::null()).output();
        let v = out.ok().and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok()).unwrap_or_default();
        let text = v["tooltip"].as_str().unwrap_or_default().to_string();
        let saver = v["class"].as_array().is_some_and(|c| c.iter().any(|x| x == "saver"));
        if let Ok(mut t) = tip.lock() {
            *t = (text, saver);
        }
    });
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
    pub fn new(conf: Conf) -> Battery {
        Battery { conf, now: info(), last: Instant::now(), tip: Arc::default(), tip_last: None }
    }
}

impl Module for Battery {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let (pct, state) = self.now?;
        let t = look.theme;
        let saver = self.tip.lock().is_ok_and(|x| x.1);
        let alt = match state {
            State::Charging => "charging",
            _ if saver => "saver",
            State::Full => "full",
            State::Discharging => "discharging",
        };
        let mut icons = self.conf.icons(alt);
        if icons.is_empty() {
            icons = if !look.icons {
                vec![look.tx(match state {
                    State::Charging => "bat+",
                    State::Full => "ac",
                    State::Discharging => "bat",
                })]
            } else {
                // md-battery_10 … md-battery, y los de cargando
                let dis = ['\u{f007a}', '\u{f007b}', '\u{f007c}', '\u{f007d}', '\u{f007e}', '\u{f007f}', '\u{f0080}', '\u{f0081}', '\u{f0082}', '\u{f0079}'];
                let chg = ['\u{f089c}', '\u{f0086}', '\u{f0087}', '\u{f0088}', '\u{f089d}', '\u{f0089}', '\u{f089e}', '\u{f008a}', '\u{f008b}', '\u{f0085}'];
                match state {
                    State::Charging => chg.iter().map(|c| c.to_string()).collect(),
                    State::Full => vec!["\u{f06a5}".to_string()],
                    State::Discharging => dis.iter().map(|c| c.to_string()).collect(),
                }
            };
        }
        let tpl = self.conf.format("").unwrap_or_else(|| "{icon} {percentage}%".into());
        // el ícono puede traer marcado (las etiquetas chicas de fundidora)
        let tpl = tpl.replace("{icon}", &pick(&icons, pct as f64));
        // las clases de Battery.py: low (≤20) y critical (≤10) descargando
        let low = state == State::Discharging && pct <= 20;
        let critical = state == State::Discharging && pct <= 10;
        let class = |key: &str| self.conf.str(key).and_then(|c| color(look, &c));
        let mut fg = if critical {
            class("critical-color").or_else(|| named("critical")).unwrap_or(t.red)
        } else if low {
            class("low-color").or_else(|| named("critical")).unwrap_or(t.red)
        } else {
            t.fg
        };
        if saver && !low {
            // la opacidad de .saver (sobre el color propio del módulo, si tiene)
            let own = class("fg").unwrap_or(fg);
            fg = color(look, &mix(own, t.bg, 0.8)).unwrap_or(fg);
        }
        let mut st = Style::new().fg(fg);
        if critical {
            st = st.bold();
        }
        show(look, &tpl, &[("percentage", Val::Num(pct as f64))], st)
    }

    fn tooltip(&self, _: &Look) -> Option<String> {
        self.now?;
        let t = self.tip.lock().ok()?.0.clone();
        // el de Battery.py es texto, no marcado
        (!t.is_empty()).then(|| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"))
    }

    fn act(&mut self, act: Act, _: u16) {
        match act {
            Act::Left => script("Battery.py", &["profile"]),
            Act::Right => script("Battery.py", &["saver"]),
            // fierro (límites de carga, perfiles), si está
            Act::Middle => script("TuiApp.sh", &["fierro"]),
            _ => {}
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        // Battery.py avisa con RTMIN+11 al cambiar de perfil o de ahorro
        let now_please = matches!(ev, Ev::Signal(11));
        // su tooltip: cada minuto (como lo pedía waybar) y con la señal
        if self.now.is_some() && (now_please || self.tip_last.is_none_or(|t| t.elapsed().as_secs() >= 60)) {
            self.tip_last = Some(Instant::now());
            ask_tooltip(self.tip.clone());
        }
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
