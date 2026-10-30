//! Los workspaces: solo los primeros `persistent` (5 si el look no dice),
//! los mismos en la barra de cada monitor; uno más allá solo si es el activo
//! de ese monitor, para no perder dónde se está. El activo de ese monitor se marca como lo elegido
//! en las apps del look; los que tienen ventanas, con el color del texto; los
//! vacíos, apagados. Cada botón como el `format` del look (`{name}`, `{icon}`
//! de `format-icons`: active, el número, empty, default, como en waybar) con
//! `button-pad` celdas a cada lado. Clic va a ese workspace, la rueda al
//! siguiente o anterior.

use super::{Act, Ev, Module, Part, script};
use crate::fmt::Conf;
use crate::hypr;
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;

pub struct Workspaces {
    conf: Conf,
    /// los que siempre se ven
    always: i32,
    /// (id, ¿tiene ventanas?): los que siempre se ven
    list: Vec<(i32, bool)>,
    /// los que existen (para el activo de más allá)
    busy: Vec<(i32, bool)>,
    /// el activo del monitor enfocado
    active: i32,
    /// el activo de cada monitor: (nombre, id)
    actives: Vec<(String, i32)>,
}

impl Workspaces {
    pub fn new(conf: Conf) -> Workspaces {
        let always = conf.num("persistent").unwrap_or(5.0).clamp(0.0, 20.0) as i32;
        let mut w = Workspaces { conf, always, list: Vec::new(), busy: Vec::new(), active: 1, actives: Vec::new() };
        w.refresh();
        w
    }

    /// El activo en el monitor `out` (o el del enfocado, si no se sabe).
    fn active_on(&self, out: &str) -> i32 {
        self.actives.iter().find(|a| a.0 == out).map_or(self.active, |a| a.1)
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
        self.busy = busy;
        self.list = (1..=self.always).map(|id| (id, self.busy.iter().any(|w| w.0 == id && w.1))).collect();
        if let Some(id) = hypr::query("activeworkspace").and_then(|v| v["id"].as_i64()) {
            self.active = id as i32;
        }
        self.actives = hypr::query("monitors")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|m| Some((m["name"].as_str()?.to_string(), m["activeWorkspace"]["id"].as_i64()? as i32)))
            .collect();
    }

    fn act_for(&mut self, act: Act, col: u16, active: i32) {
        match act {
            Act::Left => {
                let mut x = 0u16;
                for &(id, busy) in &self.shown(active) {
                    let w = self.width(id, id == active, busy);
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

    /// Los que se ven con `active` de activo: los de siempre, y él si está más allá.
    fn shown(&self, active: i32) -> Vec<(i32, bool)> {
        let mut l = self.list.clone();
        if active > self.always {
            l.push((active, self.busy.iter().any(|w| w.0 == active && w.1)));
        }
        l
    }

    fn line_for(&self, look: &Look, active: i32) -> Option<Line<'static>> {
        let t = look.theme;
        // los colores del look (#workspaces button, .empty, .active), o los de siempre
        let c = |k: &str| self.conf.str(k).and_then(|v| crate::fmt::color(look, &v));
        let sel = match (c("active-fg"), c("active-bg")) {
            (None, None) => look.sel(true).0,
            (fg, bg) => {
                let mut st = Style::new().fg(fg.unwrap_or(t.fg));
                if let Some(bg) = bg {
                    st = st.bg(bg);
                }
                st
            }
        };
        let (busy_fg, empty_fg) = (c("fg").unwrap_or(t.fg), c("empty-fg").unwrap_or(t.muted));
        let spans = self
            .shown(active)
            .iter()
            .flat_map(|&(id, busy)| {
                let st = if id == active {
                    sel
                } else if busy {
                    Style::new().fg(busy_fg)
                } else {
                    Style::new().fg(empty_fg)
                };
                crate::fmt::line(&self.label(id, id == active, busy), st, look).spans
            })
            .collect::<Vec<_>>();
        Some(Line::from(spans))
    }

    fn part_for(&self, col: u16, active: i32) -> Option<Part> {
        let mut x = 0u16;
        for &(id, busy) in &self.shown(active) {
            let w = self.width(id, id == active, busy);
            if col >= x && col < x + w {
                return Some((x, w, id == active));
            }
            x += w;
        }
        None
    }

    /// El botón del workspace `id` (marcado), como el `format` del look.
    fn label(&self, id: i32, active: bool, busy: bool) -> String {
        let fmt = self.conf.str("format").unwrap_or_else(|| " {name} ".into());
        let icons = &self.conf.0["format-icons"];
        let n = id.to_string();
        // el orden de waybar: activo, su número, vacío, default; si no, el número
        let keys: Vec<&str> = [active.then_some("active"), Some(n.as_str()), (!busy).then_some("empty"), Some("default")].into_iter().flatten().collect();
        let icon = keys.iter().find_map(|k| icons[*k].as_str()).unwrap_or(&n).to_string();
        let pad = " ".repeat(self.conf.num("button-pad").unwrap_or(0.0).max(0.0) as usize);
        format!("{pad}{}{pad}", fmt.replace("{name}", &n).replace("{id}", &n).replace("{icon}", &icon))
    }

    /// Las celdas que ocupa un botón.
    fn width(&self, id: i32, active: bool, busy: bool) -> u16 {
        Line::from(crate::fmt::plain(&self.label(id, active, busy))).width() as u16
    }
}

impl Module for Workspaces {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        self.line_for(look, self.active)
    }

    fn line_on(&self, look: &Look, out: &str) -> Option<Line<'static>> {
        self.line_for(look, self.active_on(out))
    }

    fn part(&self, col: u16) -> Option<Part> {
        self.part_for(col, self.active)
    }

    fn part_on(&self, col: u16, out: &str) -> Option<Part> {
        self.part_for(col, self.active_on(out))
    }

    fn buttons(&self, look: &Look, out: &str) -> Option<Vec<(Line<'static>, bool)>> {
        let active = self.active_on(out);
        let t = look.theme;
        let c = |k: &str| self.conf.str(k).and_then(|v| crate::fmt::color(look, &v));
        let (busy_fg, empty_fg, active_fg) = (c("fg").unwrap_or(t.fg), c("empty-fg").unwrap_or(t.muted), c("active-fg"));
        Some(
            self.shown(active)
                .iter()
                .map(|&(id, busy)| {
                    let on = id == active;
                    let fg = if on { active_fg.unwrap_or(t.fg) } else if busy { busy_fg } else { empty_fg };
                    // sin el relleno de celdas: la barra pone el suyo en pixeles
                    let markup = self.label(id, on, busy);
                    let pad = " ".repeat(self.conf.num("button-pad").unwrap_or(0.0).max(0.0) as usize);
                    let markup = markup.strip_prefix(pad.as_str()).and_then(|m| m.strip_suffix(pad.as_str())).unwrap_or(&markup).to_string();
                    (crate::fmt::line(&markup, Style::new().fg(fg), look), on)
                })
                .collect(),
        )
    }

    fn act_button(&mut self, act: Act, n: Option<usize>, out: &str) {
        let shown = self.shown(self.active_on(out));
        match act {
            Act::Left => {
                if let Some((id, _)) = n.and_then(|n| shown.get(n)) {
                    script("Hypr.sh", &["workspace", &id.to_string()]);
                }
            }
            _ => self.act_for(act, u16::MAX, 0),
        }
    }

    fn active_part(&self, out: &str) -> Option<Part> {
        let active = self.active_on(out);
        let mut x = 0u16;
        for &(id, busy) in &self.shown(active) {
            let w = self.width(id, id == active, busy);
            if id == active {
                return Some((x, w, true));
            }
            x += w;
        }
        None
    }

    fn act(&mut self, act: Act, col: u16) {
        self.act_for(act, col, self.active);
    }

    fn act_on(&mut self, act: Act, col: u16, out: &str) {
        self.act_for(act, col, self.active_on(out));
    }
    fn update(&mut self, ev: &Ev) -> bool {
        let Ev::Hypr(line) = ev else { return false };
        let kind = line.split(">>").next().unwrap_or("");
        if matches!(
            kind,
            "workspace" | "workspacev2" | "createworkspace" | "destroyworkspace" | "focusedmon" | "moveworkspace"
                | "openwindow" | "closewindow" | "movewindow" | "movewindowv2"
        ) {
            let before = (self.list.clone(), self.active, self.actives.clone());
            self.refresh();
            return (self.list.clone(), self.active, self.actives.clone()) != before;
        }
        false
    }
}
