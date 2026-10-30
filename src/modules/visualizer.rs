//! El visualizador de audio y los controles del reproductor, como
//! Visualizer.py: sigue a los reproductores con `playerctl -F` y solo corre
//! cava mientras alguno suena (no reacciona a cualquier sonido, como el cava
//! de waybar con las llamadas de Discord). En pausa quedan las barras planas y
//! tenues; sin reproductor no se ve. Del look (`[bar.modules.visualizer]`):
//!   style  braille (dos barras finas por letra) | peaks (solo el punto de
//!          arriba) | chars (una letra por barra, de `chars`)
//!   bars   cuántas barras; chars  de silencio a máximo
//! Clic: anterior; medio: pausa; derecho: siguiente; la rueda: el volumen.

use super::{Act, Ev, Module, l, script, show, spawn};
use crate::fmt::{Conf, color, mix};
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

const FPS: u32 = 24;
// puntos braille de abajo hacia arriba: columna izquierda y derecha
const LEFT: [u32; 4] = [0x40, 0x04, 0x02, 0x01];
const RIGHT: [u32; 4] = [0x80, 0x20, 0x10, 0x08];

#[derive(Clone, PartialEq)]
pub enum Player {
    None,
    Paused(String),
    Playing(String),
}

/// Lo que suena: el primero que esté en Playing, o el que esté en pausa.
fn player() -> Player {
    let out = Command::new("playerctl")
        .args(["--all-players", "metadata", "-f", "{{status}}\t{{artist}} - {{title}}"])
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let rows: Vec<(&str, &str)> = out.lines().filter_map(|l| l.split_once('\t')).collect();
    if let Some((_, t)) = rows.iter().find(|r| r.0 == "Playing") {
        return Player::Playing(t.to_string());
    }
    rows.iter().find(|r| r.0 == "Paused").map_or(Player::None, |(_, t)| Player::Paused(t.to_string()))
}

/// Sigue a los reproductores (otro hilo): una línea de playerctl por cada
/// cambio de estado o de canción.
pub fn watch(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        loop {
            let _ = tx.send(Ev::Player(player()));
            let child = crate::signals::clean(&mut Command::new("playerctl"))
                .args(["--all-players", "-F", "metadata", "-f", "{{status}}{{title}}"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(mut child) = child {
                if let Some(out) = child.stdout.take() {
                    for _ in BufReader::new(out).lines().map_while(Result::ok) {
                        if tx.send(Ev::Player(player())).is_err() {
                            let _ = child.kill();
                            return;
                        }
                    }
                }
                let _ = child.wait();
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
    });
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Braille,
    Peaks,
    Chars,
}

pub struct Visualizer {
    conf: Conf,
    tx: Sender<Ev>,
    kind: Kind,
    bars: usize,
    chars: Vec<char>,
    levels: u32,
    player: Player,
    /// el último cuadro de cava
    frame: Vec<u32>,
    cava: Arc<Mutex<Option<Child>>>,
}

impl Visualizer {
    pub fn new(conf: Conf, tx: Sender<Ev>) -> Visualizer {
        let chars: Vec<char> = conf.str("chars").unwrap_or_else(|| "▁▂▃▄▅▆▇█".into()).chars().collect();
        let mut kind = match conf.str("style").as_deref() {
            Some("peaks") => Kind::Peaks,
            Some("chars") => Kind::Chars,
            _ => Kind::Braille,
        };
        if kind == Kind::Chars && chars.len() < 2 {
            kind = Kind::Braille;
        }
        let mut bars = conf.num("bars").unwrap_or(16.0).clamp(2.0, 48.0) as usize;
        if kind != Kind::Chars {
            bars += bars % 2; // braille: dos barras por letra
        }
        let levels = if kind == Kind::Chars { chars.len() as u32 - 1 } else { 4 };
        let mut v = Visualizer { conf, tx, kind, bars, chars, levels, player: Player::None, frame: Vec::new(), cava: Arc::default() };
        // al armar la barra de nuevo (otro look) no esperar a la siguiente canción
        v.update(&Ev::Player(player()));
        v
    }

    /// Las barras como letras; en silencio queda una línea, no un hueco.
    fn render(&self, vals: &[u32]) -> String {
        let min = if self.kind == Kind::Chars { 0 } else { 1 };
        let v: Vec<u32> = (0..self.bars).map(|i| vals.get(i).copied().unwrap_or(0).clamp(min, self.levels)).collect();
        if self.kind == Kind::Chars {
            return v.iter().map(|&x| self.chars[x as usize]).collect();
        }
        v.chunks(2)
            .map(|p| {
                let (a, b) = (p[0] as usize, p.get(1).copied().unwrap_or(1) as usize);
                let code = if self.kind == Kind::Peaks {
                    0x2800 | LEFT[a - 1] | RIGHT[b - 1]
                } else {
                    0x2800 | LEFT[..a].iter().fold(0, |c, x| c | x) | RIGHT[..b].iter().fold(0, |c, x| c | x)
                };
                char::from_u32(code).unwrap_or(' ')
            })
            .collect()
    }

    fn start_cava(&mut self) {
        let mut slot = self.cava.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let conf = format!(
            "[general]\nbars = {}\nframerate = {FPS}\nsleep_timer = 0\n\n[input]\nmethod = pipewire\nsource = auto\n\n\
             [output]\nmethod = raw\nraw_target = /dev/stdout\ndata_format = ascii\nascii_max_range = {}\nbar_delimiter = 59\nframe_delimiter = 10\n\n\
             [smoothing]\nmonstercat = 1\nnoise_reduction = 77\n",
            self.bars, self.levels
        );
        let path = std::path::PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())).join("empalme-visualizer.cava");
        if std::fs::write(&path, conf).is_err() {
            return;
        }
        let mut cmd = Command::new("cava");
        crate::signals::clean(&mut cmd);
        // si empalme muere, cava con él
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let Ok(mut child) = cmd.arg("-p").arg(&path).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else { return };
        let out = child.stdout.take();
        *slot = Some(child);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let Some(out) = out else { return };
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let vals: Vec<u32> = line.split(';').filter_map(|v| v.trim().parse().ok()).collect();
                if !vals.is_empty() && tx.send(Ev::Viz(vals)).is_err() {
                    return;
                }
            }
        });
    }

    fn stop_cava(&mut self) {
        if let Some(mut c) = self.cava.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.frame.clear();
    }
}

impl Drop for Visualizer {
    fn drop(&mut self) {
        self.stop_cava();
    }
}

impl Module for Visualizer {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let base = self.conf.str("color").and_then(|c| color(look, &c)).unwrap_or(look.theme.fg);
        let (text, fg) = match &self.player {
            Player::None => return None,
            // en pausa: planas y tenues (opacity 0.45 del CSS de los looks)
            Player::Paused(_) => (self.render(&[]), color(look, &mix(base, look.theme.base, 0.45)).unwrap_or(base)),
            Player::Playing(_) => (self.render(&self.frame), base),
        };
        show(look, &text, &[], Style::new().fg(fg))
    }

    fn tooltip(&self, _: &Look) -> Option<String> {
        let (icon, title) = match &self.player {
            Player::None => return None,
            Player::Paused(t) => ("\u{f04c}", t),
            Player::Playing(t) => ("\u{f04b}", t),
        };
        let title = title.replace('&', "&amp;").replace('<', "&lt;");
        Some(format!("{icon} {title}\n{}", l("click: anterior · medio: pausa · derecho: siguiente", "click: previous · middle: pause · right: next")))
    }

    fn act(&mut self, act: Act, _: u16) {
        let ctl = |what: &str| spawn(Command::new("playerctl").arg(what));
        match act {
            Act::Left => ctl("previous"),
            Act::Middle => ctl("play-pause"),
            Act::Right => ctl("next"),
            Act::Up => script("Volume.sh", &["--inc"]),
            Act::Down => script("Volume.sh", &["--dec"]),
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        match ev {
            Ev::Player(p) => {
                let changed = *p != self.player;
                self.player = p.clone();
                if matches!(self.player, Player::Playing(_)) {
                    self.start_cava();
                } else {
                    self.stop_cava();
                }
                changed
            }
            Ev::Viz(vals) if matches!(self.player, Player::Playing(_)) => {
                let changed = *vals != self.frame;
                self.frame = vals.clone();
                changed
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay_client_toolkit::reexports::calloop::channel::channel;

    fn viz(conf: serde_json::Value) -> Visualizer {
        let (tx, _rx) = channel();
        let mut v = Visualizer::new(Conf(conf), tx);
        v.stop_cava();
        v
    }

    #[test]
    fn barras() {
        let v = viz(serde_json::json!({"style": "chars", "bars": 4, "chars": "▁▂▃▄▅▆▇█"}));
        assert_eq!(v.render(&[0, 3, 7, 9]), "▁▄██");
        assert_eq!(v.render(&[]), "▁▁▁▁");
        let v = viz(serde_json::json!({"style": "braille", "bars": 4}));
        // 1 y 4 puntos a la izquierda/derecha; en silencio, el de abajo
        assert_eq!(v.render(&[1, 4, 0, 2]), "⣸⣠");
        let v = viz(serde_json::json!({"style": "peaks", "bars": 2}));
        assert_eq!(v.render(&[4, 1]), "\u{2881}");
    }
}
