//! El volumen de la salida. Clic: silencio; clic derecho: vato en audio; la
//! rueda: subir y bajar (Volume.sh, con su aviso de arre). Se entera de los
//! cambios con `pactl subscribe`, no preguntando a cada rato.

use super::{Act, Ev, Module, script};
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use smithay_client_toolkit::reexports::calloop::channel::Sender;

pub struct Audio {
    vol: Option<u32>,
    muted: bool,
}

fn pactl(args: &[&str]) -> String {
    Command::new("pactl").args(args).stderr(Stdio::null()).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn read() -> (Option<u32>, bool) {
    let v = pactl(&["get-sink-volume", "@DEFAULT_SINK@"]);
    let vol = v.split('/').nth(1).and_then(|p| p.trim().trim_end_matches('%').trim().parse().ok());
    let muted = pactl(&["get-sink-mute", "@DEFAULT_SINK@"]).contains("yes");
    (vol, muted)
}

/// Avisa cuando cambia algo de las salidas (otro hilo).
pub fn watch(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        loop {
            let child = Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
            if let Ok(mut child) = child {
                if let Some(out) = child.stdout.take() {
                    for line in BufReader::new(out).lines().map_while(Result::ok) {
                        if (line.contains("sink") || line.contains("server")) && tx.send(Ev::Audio).is_err() {
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
    pub fn new() -> Audio {
        let (vol, muted) = read();
        Audio { vol, muted }
    }
}

impl Module for Audio {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let vol = self.vol?;
        let t = look.theme;
        let icon = if look.icons {
            match (self.muted, vol) {
                (true, _) => "\u{f075f}".to_string(),
                (_, 0..=30) => "\u{f057f}".to_string(),
                (_, 31..=60) => "\u{f0580}".to_string(),
                _ => "\u{f057e}".to_string(),
            }
        } else {
            look.tx("vol")
        };
        let text = if self.muted { " --".to_string() } else { format!(" {vol}%") };
        let st = Style::new().fg(if self.muted { t.muted } else { t.fg });
        Some(Line::from(vec![Span::styled(icon, Style::new().fg(if self.muted { t.muted } else { t.primary })), Span::styled(text, st)]))
    }

    fn act(&mut self, act: Act, _: u16) {
        match act {
            Act::Left => script("Volume.sh", &["--toggle"]),
            Act::Right => script("Vato.sh", &["audio"]),
            Act::Up => script("Volume.sh", &["--inc"]),
            Act::Down => script("Volume.sh", &["--dec"]),
            Act::Middle => {}
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        if !matches!(ev, Ev::Audio) {
            return false;
        }
        let now = read();
        let changed = now != (self.vol, self.muted);
        (self.vol, self.muted) = now;
        changed
    }
}
