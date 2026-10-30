//! El bluetooth, con los formatos del look como el de waybar: `format-on`,
//! `format-off`, `format-disabled` (bloqueado con rfkill), `format-connected`
//! y `format-no-controller` (sin él, el módulo no se ve); `{num_connections}`,
//! `{device_alias}` (el primero conectado). El tooltip, igual que en waybar
//! (`{device_enumerate}`: un renglón por cada uno conectado, con
//! `tooltip-format-enumerate-connected`). Se entera por las señales de bluez,
//! sin preguntar a cada rato. Clic: vato en bluetooth.

use super::{Act, Ev, Module, l, script, show, template};
use crate::fmt::{Conf, Val};
use disco::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use std::collections::HashMap;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

type Props = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

#[derive(Clone, PartialEq, Default)]
pub struct State {
    /// no-controller, disabled, off, on, connected
    pub state: &'static str,
    /// lo conectado: (nombre, por ciento de su batería si lo dice)
    pub devices: Vec<(String, Option<u8>)>,
}

fn get<'a, T: TryFrom<&'a Value<'a>>>(p: &'a Props, k: &str) -> Option<T> {
    p.get(k).and_then(|v| T::try_from(v).ok())
}

fn read(c: &Connection) -> State {
    let objs: Objects = match c
        .call_method(Some("org.bluez"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
        .and_then(|m| m.body().deserialize::<Objects>())
    {
        Ok(o) => o,
        Err(_) => return State { state: "no-controller", devices: Vec::new() },
    };
    let mut state = "no-controller";
    let mut devices = Vec::new();
    let mut paths: Vec<_> = objs.keys().collect();
    paths.sort_by_key(|p| p.as_str());
    for path in paths {
        let ifaces = &objs[path];
        if let Some(a) = ifaces.get("org.bluez.Adapter1")
            && state == "no-controller" {
                let powered: bool = get(a, "Powered").unwrap_or(false);
                // PowerState (bluez 5.66+) dice si está apagado por rfkill
                let blocked = get::<&str>(a, "PowerState") == Some("off-blocked");
                state = if powered { "on" } else if blocked { "disabled" } else { "off" };
            }
        if let Some(d) = ifaces.get("org.bluez.Device1")
            && get(d, "Connected").unwrap_or(false) {
                let name = get::<&str>(d, "Alias").or(get::<&str>(d, "Name")).unwrap_or("?").to_string();
                let battery = ifaces.get("org.bluez.Battery1").and_then(|b| get::<u8>(b, "Percentage"));
                devices.push((name, battery));
            }
    }
    if state == "on" && !devices.is_empty() {
        state = "connected";
    }
    State { state, devices }
}

/// Lee el estado y lo vuelve a leer con cada señal de bluez (otro hilo).
pub fn watch(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let mut last = State::default();
        loop {
            let mut run = || -> zbus::Result<()> {
                let conn = Connection::system()?;
                let mut send = |s: State| -> bool {
                    if s != last {
                        last = s.clone();
                        return tx.send(Ev::Bluetooth(s)).is_ok();
                    }
                    true
                };
                if !send(read(&conn)) {
                    return Ok(());
                }
                let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).sender("org.bluez")?.build();
                for _ in zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(64))? {
                    if !send(read(&conn)) {
                        return Ok(());
                    }
                }
                Ok(())
            };
            let _ = run();
            // bluez se cayó o no ha arrancado: avisar y volver a intentar
            if last.state != "no-controller" {
                last = State { state: "no-controller", devices: Vec::new() };
                if tx.send(Ev::Bluetooth(last.clone())).is_err() {
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
}

pub struct Bluetooth {
    conf: Conf,
    now: State,
}

impl Bluetooth {
    pub fn new(conf: Conf) -> Bluetooth {
        let now = Connection::system().map(|c| read(&c)).unwrap_or_default();
        Bluetooth { conf, now }
    }
}

impl Module for Bluetooth {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let t = look.theme;
        let c = &self.conf;
        let tpl = match self.now.state {
            "connected" => template(look, c, "connected", "<span color=\"primary\">󰂱</span> {num_connections}", "bt {num_connections}"),
            "on" => template(look, c, "on", "<span color=\"primary\">󰂯</span>", "bt"),
            "off" => template(look, c, "off", "<span color=\"muted\">󰂲</span>", "bt --"),
            "disabled" => template(look, c, "disabled", "<span color=\"muted\">󰂲</span>", "bt --"),
            _ => c.str("format-no-controller")?,
        };
        let vals = [
            ("num_connections", Val::Num(self.now.devices.len() as f64)),
            ("device_alias", Val::from(self.now.devices.first().map(|d| d.0.as_str()).unwrap_or(""))),
        ];
        let off = matches!(self.now.state, "off" | "disabled");
        show(look, &tpl, &vals, Style::new().fg(if off { t.muted } else { t.fg }))
    }

    fn tooltip(&self, _: &Look) -> Option<String> {
        let c = &self.conf;
        let tpl = match self.now.state {
            "connected" => c.tooltip("connected", "{device_enumerate}"),
            "on" => c.tooltip("on", l("Bluetooth prendido", "Bluetooth on")),
            "off" => c.tooltip("off", l("Bluetooth apagado", "Bluetooth off")),
            "disabled" => c.tooltip("disabled", l("Bluetooth bloqueado", "Bluetooth blocked")),
            _ => return None,
        }?;
        // un renglón por cada uno conectado (con su batería si la dice)
        let each: Vec<String> = self
            .now
            .devices
            .iter()
            .map(|(name, bat)| {
                let f = match bat {
                    Some(_) => c
                        .str("tooltip-format-enumerate-connected-battery")
                        .or_else(|| c.str("tooltip-format-enumerate-connected"))
                        .unwrap_or_else(|| "{device_alias} {device_battery_percentage}%".into()),
                    None => c.str("tooltip-format-enumerate-connected").map(|f| f.replace("{device_battery_percentage}", "?")).unwrap_or_else(|| "{device_alias}".into()),
                };
                let vals = [("device_alias", Val::from(name.as_str())), ("device_battery_percentage", Val::Num(bat.unwrap_or(0) as f64))];
                crate::fmt::fill(&f, &vals)
            })
            .collect();
        let vals = [
            ("num_connections", Val::Num(self.now.devices.len() as f64)),
            ("device_alias", Val::from(self.now.devices.first().map(|d| d.0.as_str()).unwrap_or(""))),
        ];
        // {device_enumerate} ya va relleno (y escapado): se pone al final
        Some(crate::fmt::fill(&tpl, &vals).replace("{device_enumerate}", &each.join("\n")))
    }

    fn act(&mut self, act: Act, _: u16) {
        if act == Act::Left {
            script("Vato.sh", &["bluetooth"]);
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        let Ev::Bluetooth(s) = ev else { return false };
        let changed = *s != self.now;
        self.now = s.clone();
        changed
    }
}
