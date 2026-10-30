//! Los workspaces: del 1 al 5 siempre (como tenía waybar) y los que haya
//! además. El activo se marca como lo elegido en las apps del look; los que
//! tienen ventanas, con el color del texto; los vacíos, apagados. Clic va a
//! ese workspace, la rueda al siguiente o anterior.

use super::{Act, Ev, Module, script};
use crate::hypr;
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

const ALWAYS: i32 = 5;

pub struct Workspaces {
    /// (id, ¿tiene ventanas?)
    list: Vec<(i32, bool)>,
    active: i32,
}

impl Workspaces {
    pub fn new() -> Workspaces {
        let mut w = Workspaces { list: Vec::new(), active: 1 };
        w.refresh();
        w
    }

    fn refresh(&mut self) {
        let mut busy: Vec<(i32, bool)> = hypr::query("workspaces")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|w| Some((w["id"].as_i64()? as i32, w["windows"].as_i64().unwrap_or(0) > 0)))
            .filter(|(id, _)| *id > 0)
            .collect();
        busy.sort();
        let last = busy.last().map(|w| w.0).unwrap_or(0).max(ALWAYS);
        self.list = (1..=last).map(|id| (id, busy.iter().any(|w| w.0 == id && w.1))).collect();
        if let Some(id) = hypr::query("activeworkspace").and_then(|v| v["id"].as_i64()) {
            self.active = id as i32;
        }
    }

    /// Cada workspace ocupa " N " (3 celdas, más si el número es largo).
    fn label(id: i32) -> String {
        format!(" {id} ")
    }
}

impl Module for Workspaces {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let t = look.theme;
        let (sel, _) = look.sel(true);
        let spans = self
            .list
            .iter()
            .map(|&(id, busy)| {
                let st = if id == self.active {
                    sel
                } else if busy {
                    Style::new().fg(t.fg)
                } else {
                    Style::new().fg(t.muted)
                };
                Span::styled(Self::label(id), st)
            })
            .collect::<Vec<_>>();
        Some(Line::from(spans))
    }

    fn act(&mut self, act: Act, col: u16) {
        match act {
            Act::Left => {
                let mut x = 0u16;
                for &(id, _) in &self.list {
                    let w = Self::label(id).len() as u16;
                    if col >= x && col < x + w {
                        script("Hypr.sh", &["workspace", &id.to_string()]);
                        return;
                    }
                    x += w;
                }
            }
            // como en waybar: rueda arriba, el siguiente
            Act::Up => script("Hypr.sh", &["workspace", "e+1"]),
            Act::Down => script("Hypr.sh", &["workspace", "e-1"]),
            _ => {}
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        let Ev::Hypr(line) = ev else { return false };
        let kind = line.split(">>").next().unwrap_or("");
        if matches!(
            kind,
            "workspace" | "workspacev2" | "createworkspace" | "destroyworkspace" | "focusedmon" | "moveworkspace"
                | "openwindow" | "closewindow" | "movewindow" | "movewindowv2"
        ) {
            let before = (self.list.clone(), self.active);
            self.refresh();
            return (self.list.clone(), self.active) != before;
        }
        false
    }
}
