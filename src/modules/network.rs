//! La red, con los formatos del look como el network de waybar:
//! `format-wifi`, `format-ethernet`, `format-linked` (con cable o wifi pero
//! sin IP), `format-disconnected`; `{ifname}`, `{ipaddr}`, `{signalStrength}`,
//! `{icon}` (por la señal). Lo lee de /proc y /sys, sin lanzar nada.
//! Clic: vato en red; clic derecho: vato en wifi.

use super::{Act, Ev, Module, l, script, show, template};
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
    /// el nombre de la red wifi
    essid: String,
    /// bytes recibidos y enviados hasta ahora (para la velocidad)
    bytes: (u64, u64),
}

pub struct Network {
    conf: Conf,
    every: Duration,
    last: Instant,
    now: Now,
    /// bajada y subida, en bits por segundo
    rate: (f64, f64),
}

/// Bytes recibidos y enviados por `iface`.
fn bytes(iface: &str) -> (u64, u64) {
    let n = |f: &str| {
        std::fs::read_to_string(Path::new("/sys/class/net").join(iface).join("statistics").join(f))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    };
    (n("rx_bytes"), n("tx_bytes"))
}

/// El nombre de la red wifi (`iw dev <iface> link`).
fn essid(iface: &str) -> String {
    std::process::Command::new("iw")
        .args(["dev", iface, "link"])
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().find_map(|l| l.trim().strip_prefix("SSID:").map(|s| s.trim().to_string())).unwrap_or_default())
        .unwrap_or_default()
}

/// Bits por segundo como waybar: `1.27Mb/s`.
fn bits(v: f64) -> String {
    let mut v = v;
    let mut unit = "";
    for u in ["k", "M", "G"] {
        if v < 1000.0 {
            break;
        }
        v /= 1000.0;
        unit = u;
    }
    let n = if unit.is_empty() || v >= 100.0 { format!("{v:.0}") } else if v >= 10.0 { format!("{v:.1}") } else { format!("{v:.2}") };
    format!("{n}{unit}b/s")
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
            let essid = if wifi { essid(&ifname) } else { String::new() };
            let bytes = bytes(&ifname);
            return Now { state: if wifi { "wifi" } else { "ethernet" }, ifname, ip, signal, essid, bytes };
        }
    match linked_iface() {
        Some(ifname) => Now { state: "linked", ifname, ..Default::default() },
        None => Now { state: "disconnected", ..Default::default() },
    }
}

impl Network {
    pub fn new(conf: Conf) -> Network {
        let secs = conf.num("interval").unwrap_or(5.0).max(1.0);
        Network { conf, every: Duration::from_secs_f64(secs), last: Instant::now(), now: read(), rate: (0.0, 0.0) }
    }
}

impl Network {
    fn vals(&self) -> Vec<(&str, Val)> {
        let n = &self.now;
        let mut icons = self.conf.icons("default");
        if icons.is_empty() {
            icons = ["󰤯", "󰤟", "󰤢", "󰤥", "󰤨"].map(String::from).to_vec();
        }
        vec![
            ("ifname", Val::from(n.ifname.as_str())),
            ("ipaddr", Val::from(n.ip.as_str())),
            ("essid", Val::from(n.essid.as_str())),
            ("signalStrength", Val::Num(n.signal as f64)),
            ("icon", Val::Text(pick(&icons, n.signal as f64))),
            ("bandwidthDownBits", Val::Text(bits(self.rate.0))),
            ("bandwidthUpBits", Val::Text(bits(self.rate.1))),
            ("bandwidthTotalBits", Val::Text(bits(self.rate.0 + self.rate.1))),
        ]
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
        let off = matches!(n.state, "linked" | "disconnected");
        show(look, &tpl, &self.vals(), Style::new().fg(if off { t.muted } else { t.fg }))
    }

    fn tooltip(&self, _: &Look) -> Option<String> {
        let c = &self.conf;
        let tpl = match self.now.state {
            "wifi" => c.tooltip("wifi", "{essid} {signalStrength}%\n{ipaddr}  {bandwidthDownBits}  {bandwidthUpBits}"),
            "ethernet" => c.tooltip("ethernet", "{ifname} {ipaddr}\n{bandwidthDownBits}  {bandwidthUpBits}"),
            "linked" => c.tooltip("linked", l("{ifname} (sin IP)", "{ifname} (no IP)")),
            _ => c.tooltip("disconnected", l("Sin red", "Disconnected")),
        }?;
        Some(crate::fmt::fill(&tpl, &self.vals()))
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
        let secs = self.last.elapsed().as_secs_f64().max(0.001);
        self.last = Instant::now();
        let now = read();
        // la velocidad: lo que pasó desde la vez anterior, en la misma interfaz
        self.rate = if now.ifname == self.now.ifname {
            let d = |a: u64, b: u64| a.saturating_sub(b) as f64 * 8.0 / secs;
            (d(now.bytes.0, self.now.bytes.0), d(now.bytes.1, self.now.bytes.1))
        } else {
            (0.0, 0.0)
        };
        // los bytes siempre cambian: redibujar solo si cambia lo que se ve
        let changed = (now.state, &now.ifname, &now.ip, now.signal) != (self.now.state, &self.now.ifname, &self.now.ip, self.now.signal);
        self.now = now;
        changed
    }
}
