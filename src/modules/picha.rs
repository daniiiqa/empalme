//! picha (Quick Share de discada) mientras pasa algo entre el cel y la laptop:
//! flecha abajo si llega, arriba si sale, y el porcentaje. Lo lee de
//! $XDG_RUNTIME_DIR/picha, donde picha deja un archivo `<pid>-<n>` por envío,
//! con un renglón `in|out <porcentaje> <quién>`, y lo borra al terminar. Sin
//! envíos no se ve. Clic: el menú de picha. Etiquetas del look: `in`, `out`.

use super::{Act, Ev, Module, l, show, spawn};
use crate::fmt::Conf;
use disco::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, PartialEq)]
struct Transfer {
    incoming: bool,
    pct: u8,
    who: String,
}

pub struct Picha {
    conf: Conf,
    now: Vec<Transfer>,
}

fn dir() -> PathBuf {
    PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())).join("picha")
}

fn parse(line: &str) -> Option<Transfer> {
    let mut f = line.trim_end().splitn(3, ' ');
    let incoming = match f.next()? {
        "in" => true,
        "out" => false,
        _ => return None,
    };
    let pct = f.next()?.parse::<u8>().ok()?.min(100);
    Some(Transfer { incoming, pct, who: f.next().unwrap_or("").to_string() })
}

/// Los envíos de procesos de picha que siguen vivos (uno que se cayó no deja
/// la flecha pegada).
fn read() -> Vec<Transfer> {
    let Ok(entries) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut out: Vec<(String, Transfer)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let pid: i32 = name.split_once('-')?.0.parse().ok()?;
            if unsafe { libc::kill(pid, 0) } != 0 {
                return None;
            }
            Some((name, parse(&std::fs::read_to_string(e.path()).ok()?)?))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().map(|(_, t)| t).collect()
}

impl Picha {
    pub fn new(conf: Conf) -> Picha {
        Picha { conf, now: read() }
    }

    fn label(&self, look: &Look, key: &str, icon: &str, text: &str) -> String {
        self.conf.str(key).unwrap_or_else(|| if look.icons { icon.to_string() } else { look.tx(text) })
    }
}

impl Module for Picha {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let parts: Vec<String> = self
            .now
            .iter()
            .map(|t| {
                let label = if t.incoming {
                    self.label(look, "in", "\u{f01da}", "[in]")
                } else {
                    self.label(look, "out", "\u{f0552}", "[out]")
                };
                format!("{label} {}%", t.pct)
            })
            .collect();
        show(look, &parts.join("  "), &[], Style::new().fg(look.theme.fg))
    }

    fn tooltip(&self, _: &Look) -> Option<String> {
        if self.now.is_empty() || self.conf.0["tooltip"].as_bool() == Some(false) {
            return None;
        }
        let mut tip: Vec<String> = self
            .now
            .iter()
            .map(|t| {
                let (dir, who) = if t.incoming { (l("De", "From"), &t.who) } else { (l("A", "To"), &t.who) };
                format!("{dir} {who} · {}%", t.pct)
            })
            .collect();
        tip.push(l("click: picha", "click: picha").to_string());
        Some(tip.join("\n"))
    }

    fn act(&mut self, act: Act, _: u16) {
        if act == Act::Left {
            spawn(Command::new("kitty").args(["--class", "picha", "-e", "picha"]));
        }
    }

    fn class(&self) -> Option<&'static str> {
        (!self.now.is_empty()).then_some("active")
    }

    fn update(&mut self, ev: &Ev) -> bool {
        if !matches!(ev, Ev::Tick) {
            return false;
        }
        let now = read();
        let changed = now != self.now;
        self.now = now;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_picha_lines() {
        let t = parse("in 42 Daniel's S21\n").unwrap();
        assert!(t.incoming && t.pct == 42 && t.who == "Daniel's S21");
        assert!(!parse("out 100 hachikuji").unwrap().incoming);
        assert!(parse("sideways 5 x").is_none());
    }
}
