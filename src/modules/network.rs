//! La red, con los formatos del look como el network de waybar:
//! `format-wifi`, `format-ethernet`, `format-linked` (con cable o wifi pero
//! sin IP), `format-disconnected`; `{ifname}`, `{ipaddr}`, `{signalStrength}`,
//! `{icon}` (por la señal). Lo lee de /proc y /sys, sin lanzar nada.
//! Clic: vato en red; clic derecho: vato en wifi.

use super::{Act, Ev, Module, script, show, template};
use crate::fmt::{Conf, Val, pick};
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, PartialEq, Default)]
struct Now {
    /// wifi, ethernet, linked, disconnected
    state: &'static str,
    ifname: String,
    ip: String,
    signal: u32,
}

pub struct Network {
    conf: Conf,
    every: Duration,
    last: Instant,
    now: Now,
}

/// La interfaz de la ruta por defecto (la de menor métrica).
fn default_iface() -> Option<String> {
    std::fs::read_to_string("/proc/net/route")
        .ok()?
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.get(1) == Some(&"00000000")).then(|| (f.get(6).and_then(|m| m.parse::<u32>().ok()).unwrap_or(0), f[0].to_string()))
        })
        .min()
        .map(|x| x.1)
}

/// La IPv4 de `iface`, si tiene.
fn ipv4(iface: &str) -> Option<String> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut out = None;
    let mut p = list;
    while !p.is_null() {
        let a = unsafe { &*p };
        let name = unsafe { std::ffi::CStr::from_ptr(a.ifa_name) }.to_string_lossy();
        if name == iface && !a.ifa_addr.is_null() && unsafe { (*a.ifa_addr).sa_family } as i32 == libc::AF_INET {
            let sin = unsafe { &*(a.ifa_addr as *const libc::sockaddr_in) };
            out = Some(std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)).to_string());
            break;
        }
        p = a.ifa_next;
    }
    unsafe { libc::freeifaddrs(list) };
    out
}

fn wireless(iface: &str) -> bool {
    let d = Path::new("/sys/class/net").join(iface);
    d.join("wireless").exists() || d.join("phy80211").exists()
}

/// La calidad del enlace en por ciento (de 70, como la cuenta el kernel).
fn signal(iface: &str) -> u32 {
    std::fs::read_to_string("/proc/net/wireless")
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            let (name, rest) = l.split_once(':')?;
            (name.trim() == iface).then(|| rest.split_whitespace().nth(1)?.trim_end_matches('.').parse::<f64>().ok())?
        })
        .map(|q| (q * 100.0 / 70.0).round().min(100.0) as u32)
        .unwrap_or(0)
}

/// Una interfaz física prendida (con cable conectado o asociada al wifi).
fn linked_iface() -> Option<String> {
    std::fs::read_dir("/sys/class/net").ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        let up = std::fs::read_to_string(e.path().join("operstate")).is_ok_and(|s| s.trim() == "up");
        (up && e.path().join("device").exists()).then_some(name)
    })
}

fn read() -> Now {
    if let Some(ifname) = default_iface()
        && let Some(ip) = ipv4(&ifname) {
            let wifi = wireless(&ifname);
            let signal = if wifi { signal(&ifname) } else { 0 };
            return Now { state: if wifi { "wifi" } else { "ethernet" }, ifname, ip, signal };
        }
    match linked_iface() {
        Some(ifname) => Now { state: "linked", ifname, ..Default::default() },
        None => Now { state: "disconnected", ..Default::default() },
    }
}

impl Network {
    pub fn new(conf: Conf) -> Network {
        let secs = conf.num("interval").unwrap_or(5.0).max(1.0);
        Network { conf, every: Duration::from_secs_f64(secs), last: Instant::now(), now: read() }
    }
}

impl Module for Network {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let t = look.theme;
        let c = &self.conf;
        let n = &self.now;
        let tpl = match n.state {
            "wifi" => template(look, c, "wifi", "<span color=\"primary\">{icon}</span>", "net {signalStrength}%"),
            "ethernet" => template(look, c, "ethernet", "<span color=\"primary\">󰌘</span>", "net up"),
            "linked" => template(look, c, "linked", "<span color=\"muted\">󰈁</span> {ifname}", "net ??"),
            _ => template(look, c, "disconnected", "<span color=\"muted\">󰌙</span>", "net --"),
        };
        let mut icons = c.icons("default");
        if icons.is_empty() {
            icons = ["󰤯", "󰤟", "󰤢", "󰤥", "󰤨"].map(String::from).to_vec();
        }
        let vals = [
            ("ifname", Val::from(n.ifname.as_str())),
            ("ipaddr", Val::from(n.ip.as_str())),
            ("signalStrength", Val::Num(n.signal as f64)),
            ("icon", Val::Text(pick(&icons, n.signal as f64))),
        ];
        let off = matches!(n.state, "linked" | "disconnected");
        show(look, &tpl, &vals, Style::new().fg(if off { t.muted } else { t.fg }))
    }

    fn act(&mut self, act: Act, _: u16) {
        match act {
            Act::Left => script("Vato.sh", &["network"]),
            Act::Right => script("Vato.sh", &["wifi"]),
            _ => {}
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        if !matches!(ev, Ev::Tick) || self.last.elapsed() + Duration::from_millis(100) < self.every {
            return false;
        }
        self.last = Instant::now();
        let now = read();
        let changed = now != self.now;
        self.now = now;
        changed
    }
}
