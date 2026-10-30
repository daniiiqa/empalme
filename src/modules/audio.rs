//! El volumen de la salida (`audio`) o del micrófono (`mic`), con los formatos
//! del look como el pulseaudio de waybar: `format`, `format-muted`,
//! `format-bluetooth`, `{volume}`, `{icon}` (de `format-icons`: la lista
//! `default` por volumen, o la del tipo de salida: `headphone`, `headset`…).
//! Clic: silencio; clic derecho: vato en audio o mic; la rueda: subir y bajar
//! (Volume.sh, con su aviso de arre). Se entera de los cambios con
//! `pactl subscribe`, no preguntando a cada rato.

use super::{Act, Ev, Module, script, show, template};
use crate::fmt::{Conf, Val, pick};
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use serde_json::Value;
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Sink,
    Source,
}

#[derive(Clone, PartialEq, Default)]
struct Now {
    vol: u32,
    muted: bool,
    bluetooth: bool,
    /// el tipo de la salida como en format-icons de waybar (headphone…)
    port: String,
    /// el nombre del dispositivo (el tooltip)
    desc: String,
}

pub struct Audio {
    kind: Kind,
    conf: Conf,
    now: Option<Now>,
}

fn pactl(args: &[&str]) -> String {
    Command::new("pactl").args(args).stderr(Stdio::null()).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn read(kind: Kind) -> Option<Now> {
    let (default, list) = match kind {
        Kind::Sink => ("get-default-sink", "sinks"),
        Kind::Source => ("get-default-source", "sources"),
    };
    let name = pactl(&[default]).trim().to_string();
    let all: Value = serde_json::from_str(&pactl(&["-f", "json", "list", list])).ok()?;
    let dev = all.as_array()?.iter().find(|d| d["name"].as_str() == Some(name.as_str()))?;
    let vol = dev["volume"]
        .as_object()
        .and_then(|ch| ch.values().next())
        .and_then(|c| c["value_percent"].as_str())
        .and_then(|p| p.trim_end_matches('%').parse().ok())
        .unwrap_or(0);
    let port = dev["active_port"].as_str().and_then(|a| {
        dev["ports"].as_array()?.iter().find(|p| p["name"].as_str() == Some(a))?["type"].as_str().map(String::from)
    });
    let port = match port.as_deref().unwrap_or("") {
        "Headphones" => "headphone",
        "Headset" => "headset",
        "Handsfree" => "hands-free",
        "Phone" => "phone",
        "Portable" => "portable",
        "Car" => "car",
        _ => "",
    };
    Some(Now {
        vol,
        muted: dev["mute"].as_bool().unwrap_or(false),
        bluetooth: name.starts_with("bluez"),
        port: port.into(),
        desc: dev["description"].as_str().unwrap_or("").to_string(),
    })
}

/// Avisa cuando cambia algo de las salidas o entradas (otro hilo).
pub fn watch(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        loop {
            let child = Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
            if let Ok(mut child) = child {
                if let Some(out) = child.stdout.take() {
                    for line in BufReader::new(out).lines().map_while(Result::ok) {
                        if (line.contains("sink") || line.contains("source") || line.contains("server")) && tx.send(Ev::Audio).is_err() {
                            let _ = child.kill();
                            return;
                        }
                    }
                }
                let _ = child.wait();
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

impl Audio {
    pub fn new(kind: Kind, conf: Conf) -> Audio {
        Audio { kind, conf, now: read(kind) }
    }
}

impl Audio {
    /// Los valores de los formatos: con los nombres de waybar para la
    /// salida ({volume}, {desc}) y para el micrófono ({source_volume}…).
    fn vals(&self) -> Vec<(&str, Val)> {
        let Some(n) = &self.now else { return Vec::new() };
        let c = &self.conf;
        let mut icons = if n.port.is_empty() { Vec::new() } else { c.icons(&n.port) };
        if icons.is_empty() {
            icons = c.icons("default");
        }
        if icons.is_empty() && self.kind == Kind::Sink {
            icons = ["󰕿", "󰖀", "󰕾"].map(String::from).to_vec();
        }
        let v = n.vol as f64;
        vec![
            ("volume", Val::Num(v)),
            ("source_volume", Val::Num(v)),
            ("icon", Val::Text(pick(&icons, v))),
            ("desc", Val::from(n.desc.as_str())),
            ("source_desc", Val::from(n.desc.as_str())),
        ]
    }
}

impl Module for Audio {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let n = self.now.as_ref()?;
        let t = look.theme;
        let c = &self.conf;
        let state = if n.muted { "muted" } else if n.bluetooth { "bluetooth" } else { "" };
        let tpl = match (self.kind, state) {
            (Kind::Sink, "muted") => template(look, c, "muted", "<span color=\"muted\">󰖁</span>", "vol --"),
            (Kind::Sink, "bluetooth") => template(look, c, "bluetooth", "<span color=\"primary\">{icon} 󰂰</span> {volume}%", "vol(bt) {volume}%"),
            (Kind::Sink, _) => template(look, c, "", "<span color=\"primary\">{icon}</span> {volume}%", "vol {volume}%"),
            (Kind::Source, "muted") => template(look, c, "muted", "<span color=\"muted\">󰍭</span>", "mic --"),
            (Kind::Source, _) => template(look, c, "", "<span color=\"primary\">󰍬</span> {volume}%", "mic {volume}%"),
        };
        show(look, &tpl, &self.vals(), Style::new().fg(if n.muted { t.muted } else { t.fg }))
    }

    fn tooltip(&self, _: &Look) -> Option<String> {
        self.now.as_ref()?;
        let tpl = match self.kind {
            Kind::Sink => self.conf.tooltip("", "{desc} | {volume}%"),
            Kind::Source => self.conf.tooltip("", "{source_desc} | {source_volume}%"),
        }?;
        Some(crate::fmt::fill(&tpl, &self.vals()))
    }

    fn act(&mut self, act: Act, _: u16) {
        let mic = self.kind == Kind::Source;
        match act {
            Act::Left => script("Volume.sh", &[if mic { "--toggle-mic" } else { "--toggle" }]),
            Act::Right => script("Vato.sh", &[if mic { "mic" } else { "audio" }]),
            Act::Up => script("Volume.sh", &[if mic { "--mic-inc" } else { "--inc" }]),
            Act::Down => script("Volume.sh", &[if mic { "--mic-dec" } else { "--dec" }]),
            Act::Middle => {}
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        if !matches!(ev, Ev::Audio) {
            return false;
        }
        let now = read(self.kind);
        let changed = now != self.now;
        self.now = now;
        changed
    }
}
