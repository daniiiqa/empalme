//! Las notificaciones de arre, como el custom/swaync de waybar: `format`
//! (`{}` es cuántas hay, `{icon}` el de `format-icons` según el estado:
//! notification, none, dnd-notification, dnd-none). Se entera por la señal
//! `Changed` de arre, sin otro proceso. Clic: vato en notificaciones; clic
//! derecho: no molestar.

use super::{Act, Ev, Module, l, script, show, spawn, template};
use crate::fmt::{Conf, Val};
use disco::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use std::process::Command;
use zbus::blocking::Connection;

const NAME: &str = "org.discada.Arre";
const PATH: &str = "/org/discada/Arre";
const IFACE: &str = "org.discada.Arre";

/// (cuántas, no molestar); None: arre no está corriendo.
pub type State = Option<(u32, bool)>;

fn read(c: &Connection) -> State {
    let ask = |m: &str| c.call_method(Some(NAME), PATH, Some(IFACE), m, &()).ok();
    let count: u32 = ask("Count")?.body().deserialize().ok()?;
    let dnd: bool = ask("Dnd")?.body().deserialize().ok()?;
    Some((count, dnd))
}

/// Lee el estado y lo vuelve a leer con cada `Changed` de arre (otro hilo).
pub fn watch(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let mut last: State = None;
        loop {
            let mut run = || -> zbus::Result<()> {
                let conn = Connection::session()?;
                let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(IFACE)?.member("Changed")?.build();
                let mut changes = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(16))?;
                loop {
                    let now = read(&conn);
                    if now != last {
                        last = now;
                        if tx.send(Ev::Notifications(now)).is_err() {
                            return Ok(());
                        }
                    }
                    // arre no está: preguntar otra vez en un rato
                    if now.is_none() {
                        return Ok(());
                    }
                    if changes.next().is_none() {
                        return Ok(());
                    }
                }
            };
            let _ = run();
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
    });
}

pub struct Notifications {
    conf: Conf,
    now: State,
}

impl Notifications {
    pub fn new(conf: Conf) -> Notifications {
        let now = Connection::session().ok().and_then(|c| read(&c));
        Notifications { conf, now }
    }
}

impl Module for Notifications {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let (count, dnd) = self.now?;
        let t = look.theme;
        let alt = match (dnd, count > 0) {
            (false, true) => "notification",
            (false, false) => "none",
            (true, true) => "dnd-notification",
            (true, false) => "dnd-none",
        };
        let c = &self.conf;
        let icon = c.icons(alt).into_iter().next();
        let (tpl, icon) = match icon {
            // el look trae sus íconos: su format (o "{icon}") con ellos
            Some(i) => (c.str("format").unwrap_or_else(|| "{icon}".into()), i),
            None => {
                let i = match alt {
                    "notification" => "<span color=\"red\">󰂚</span>",
                    "none" => "󰂜",
                    "dnd-notification" => "<span color=\"red\">󰂛</span>",
                    _ => "󰂛",
                };
                let text = match alt {
                    "notification" => "msg*",
                    "none" => "msg",
                    "dnd-notification" => "dnd*",
                    _ => "dnd",
                };
                (template(look, &Conf::default(), "", i, text), String::new())
            }
        };
        // el ícono puede traer marcado del look (<span foreground='red'>): va tal cual
        let tpl = tpl.replace("{icon}", &icon);
        let vals = [("", Val::Num(count as f64))];
        show(look, &tpl, &vals, Style::new().fg(if dnd { t.muted } else { t.fg }))
    }

    /// Lo mismo que dice `arre waybar`.
    fn tooltip(&self, _: &Look) -> Option<String> {
        let (count, dnd) = self.now?;
        let first = match (count, dnd) {
            (0, false) => l("Sin notificaciones", "No notifications").to_string(),
            (1, false) => format!("1 {}", l("notificación", "notification")),
            (n, false) => format!("{n} {}", l("notificaciones", "notifications")),
            (n, true) => format!("{n} · {}", l("no molestar", "do not disturb")),
        };
        let text = format!("{first}\n{}", l("Clic derecho: no molestar", "Right click: do not disturb"));
        self.conf.tooltip("", "").map(|t| if t.is_empty() { text } else { t })
    }

    fn act(&mut self, act: Act, _: u16) {
        match act {
            Act::Left => script("Vato.sh", &["notifications"]),
            Act::Right => {
                let arre = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/bin/arre");
                spawn(Command::new(arre).args(["dnd", "toggle"]));
            }
            _ => {}
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        let Ev::Notifications(s) = ev else { return false };
        let changed = *s != self.now;
        self.now = *s;
        changed
    }
}
