//! El sistema: `cpu`, `memory`, `disk` y `temperature`, con los formatos del
//! look como los de waybar (`{usage}`, `{icon0}…` por núcleo, `{used:0.1f}`,
//! `{percentage_used}`, `{temperatureC}`; `format-alt` con un clic y
//! `format-critical` pasando `critical-threshold`). Clic derecho: vato en
//! sistema. Cada uno se relee a su ritmo (`interval`, en segundos).

use super::{Act, Ev, Module, l, script, show, template};
use crate::fmt::{Conf, Val, named, pick};
use disco::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Cpu,
    Memory,
    Disk,
    Temperature,
}

pub struct System {
    kind: Kind,
    conf: Conf,
    alt: bool,
    every: Duration,
    last: Instant,
    /// lo último leído, ya como valores para el formato
    vals: Vec<(&'static str, f64)>,
    texts: Vec<(String, String)>,
    critical: bool,
    /// cpu: (ocupado, total) de cada renglón de /proc/stat la vez pasada
    prev: Vec<(u64, u64)>,
    /// temperature: de dónde se lee
    sensor: Option<PathBuf>,
}

fn read(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// (ocupado, total) de "cpu" y de cada "cpuN".
fn cpu_times() -> Vec<(u64, u64)> {
    read("/proc/stat")
        .lines()
        .take_while(|l| l.starts_with("cpu"))
        .map(|l| {
            let n: Vec<u64> = l.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
            let total: u64 = n.iter().take(8).sum();
            let idle = n.get(3).unwrap_or(&0) + n.get(4).unwrap_or(&0);
            (total - idle, total)
        })
        .collect()
}

fn human(bytes: f64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", units[i])
}

/// El sensor: `hwmon-path` del look (el primero que exista), o el del CPU por
/// nombre (los números de hwmon cambian de un arranque a otro), o la zona 0.
fn sensor(conf: &Conf) -> Option<PathBuf> {
    let given: Vec<String> = match &conf.0["hwmon-path"] {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    };
    if let Some(p) = given.iter().map(PathBuf::from).find(|p| p.exists()) {
        return Some(p);
    }
    let want = ["k10temp", "zenpower", "coretemp", "cpu_thermal", "acpitz"];
    let mut found: Vec<(usize, PathBuf)> = std::fs::read_dir("/sys/class/hwmon")
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = read(&e.path().join("name").to_string_lossy()).trim().to_string();
            let rank = want.iter().position(|w| *w == name)?;
            Some((rank, e.path().join("temp1_input")))
        })
        .filter(|(_, p)| p.exists())
        .collect();
    found.sort();
    found.into_iter().next().map(|x| x.1).or_else(|| {
        let z = PathBuf::from(format!("/sys/class/thermal/thermal_zone{}/temp", conf.num("thermal-zone").unwrap_or(0.0) as u32));
        z.exists().then_some(z)
    })
}

impl System {
    pub fn new(kind: Kind, conf: Conf) -> System {
        let secs = conf.num("interval").unwrap_or(match kind {
            Kind::Cpu => 1.0,
            Kind::Memory | Kind::Temperature => 10.0,
            Kind::Disk => 30.0,
        });
        let sensor = if kind == Kind::Temperature { sensor(&conf) } else { None };
        let mut s = System {
            kind,
            conf,
            alt: false,
            every: Duration::from_secs_f64(secs.max(1.0)),
            last: Instant::now(),
            vals: Vec::new(),
            texts: Vec::new(),
            critical: false,
            prev: if kind == Kind::Cpu { cpu_times() } else { Vec::new() },
            sensor,
        };
        s.refresh();
        s
    }

    fn refresh(&mut self) {
        self.vals.clear();
        self.texts.clear();
        match self.kind {
            Kind::Cpu => {
                let now = cpu_times();
                let icons = self.conf.icons("default");
                let usage: Vec<f64> = now
                    .iter()
                    .zip(&self.prev)
                    .map(|(n, p)| {
                        let total = n.1.saturating_sub(p.1);
                        if total == 0 { 0.0 } else { n.0.saturating_sub(p.0) as f64 * 100.0 / total as f64 }
                    })
                    .collect();
                self.prev = now;
                let all = usage.first().copied().unwrap_or(0.0);
                self.vals.push(("usage", all.round()));
                let load = read("/proc/loadavg").split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                self.vals.push(("load", load));
                self.texts.push(("icon".into(), pick(&icons, all)));
                for (i, u) in usage.iter().skip(1).take(64).enumerate() {
                    self.texts.push((format!("icon{i}"), pick(&icons, *u)));
                }
                self.critical = false;
            }
            Kind::Memory => {
                let info = read("/proc/meminfo");
                let kb = |k: &str| {
                    info.lines().find_map(|l| l.strip_prefix(k)?.split_whitespace().next()?.parse::<f64>().ok()).unwrap_or(0.0)
                };
                let (total, avail) = (kb("MemTotal:"), kb("MemAvailable:"));
                let gib = 1024.0 * 1024.0;
                let used = total - avail;
                self.vals.push(("used", used / gib));
                self.vals.push(("total", total / gib));
                self.vals.push(("avail", avail / gib));
                self.vals.push(("percentage", if total > 0.0 { (used * 100.0 / total).round() } else { 0.0 }));
                self.texts.push(("icon".into(), pick(&self.conf.icons("default"), used * 100.0 / total.max(1.0))));
            }
            Kind::Disk => {
                let path = self.conf.str("path").unwrap_or_else(|| "/".into());
                let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
                let ok = std::ffi::CString::new(path.clone()).is_ok_and(|c| unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0);
                if ok {
                    let f = st.f_frsize as f64;
                    let (total, free, avail) = (st.f_blocks as f64 * f, st.f_bfree as f64 * f, st.f_bavail as f64 * f);
                    let used = total - free;
                    let pct = if total > 0.0 { used * 100.0 / (used + avail) } else { 0.0 };
                    self.vals.push(("percentage_used", pct.round()));
                    self.vals.push(("percentage_free", (100.0 - pct).round()));
                    self.texts.push(("used".into(), human(used)));
                    self.texts.push(("free".into(), human(avail)));
                    self.texts.push(("total".into(), human(total)));
                }
                self.texts.push(("path".into(), path));
            }
            Kind::Temperature => {
                let milli: f64 = self.sensor.as_ref().and_then(|p| std::fs::read_to_string(p).ok()?.trim().parse().ok()).unwrap_or(0.0);
                let c = (milli / 1000.0).round();
                self.vals.push(("temperatureC", c));
                self.vals.push(("temperatureF", (c * 9.0 / 5.0 + 32.0).round()));
                self.vals.push(("temperatureK", (c + 273.15).round()));
                let crit = self.conf.num("critical-threshold").unwrap_or(80.0);
                self.critical = c >= crit;
                self.texts.push(("icon".into(), pick(&self.conf.icons("default"), c * 100.0 / crit.max(1.0))));
            }
        }
    }
}

impl System {
    fn vals(&self) -> Vec<(&str, Val)> {
        let mut vals: Vec<(&str, Val)> = self.vals.iter().map(|(k, v)| (*k, Val::Num(*v))).collect();
        vals.extend(self.texts.iter().map(|(k, v)| (k.as_str(), Val::Text(v.clone()))));
        vals
    }
}

impl Module for System {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        if self.kind == Kind::Temperature && self.sensor.is_none() {
            return None;
        }
        let t = look.theme;
        let c = &self.conf;
        let tpl = if self.alt && c.str("format-alt").is_some() {
            c.str("format-alt").unwrap()
        } else {
            match self.kind {
                Kind::Cpu => template(look, c, "", "<span color=\"primary\">󰍛</span> {usage}%", "cpu {usage}%"),
                Kind::Memory => template(look, c, "", "<span color=\"primary\">󰘚</span> {used:0.1f}G", "mem {used:0.1f}g"),
                Kind::Disk => template(look, c, "", "<span color=\"primary\">󰋊</span> {percentage_used}%", "dsk {percentage_used}%"),
                Kind::Temperature if self.critical => template(look, c, "critical", "<span color=\"red\">󰈸</span> {temperatureC}°", "tmp {temperatureC}c!"),
                Kind::Temperature => template(look, c, "", "<span color=\"primary\">󰔏</span> {temperatureC}°", "tmp {temperatureC}c"),
            }
        };
        let critical = named("critical").unwrap_or(t.red);
        show(look, &tpl, &self.vals(), Style::new().fg(if self.critical { critical } else { t.fg }))
    }

    fn tooltip(&self, _: &Look) -> Option<String> {
        if self.kind == Kind::Temperature && self.sensor.is_none() {
            return None;
        }
        let tpl = match self.kind {
            Kind::Cpu => self.conf.tooltip("", l("Uso: {usage}%\nCarga: {load:0.2f}", "Usage: {usage}%\nLoad: {load:0.2f}")),
            Kind::Memory => self.conf.tooltip("", "{used:0.1f}GB/{total:0.1f}G"),
            Kind::Disk => self.conf.tooltip("", l("{used} de {total} en {path} ({percentage_used}%)", "{used} used out of {total} on {path} ({percentage_used}%)")),
            Kind::Temperature if self.critical => self.conf.tooltip("critical", "{temperatureC}°C"),
            Kind::Temperature => self.conf.tooltip("", "{temperatureC}°C"),
        }?;
        Some(crate::fmt::fill(&tpl, &self.vals()))
    }

    fn act(&mut self, act: Act, _: u16) {
        match act {
            Act::Left if self.conf.str("format-alt").is_some() => self.alt = !self.alt,
            Act::Right if self.kind != Kind::Disk => script("Vato.sh", &["system"]),
            _ => {}
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        if !matches!(ev, Ev::Tick) || self.last.elapsed() + Duration::from_millis(100) < self.every {
            return false;
        }
        self.last = Instant::now();
        let before = (self.vals.clone(), self.texts.clone(), self.critical);
        self.refresh();
        (self.vals.clone(), self.texts.clone(), self.critical) != before
    }
}
