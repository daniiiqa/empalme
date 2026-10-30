//! No dejar que la pantalla se apague, como el idle_inhibitor de waybar:
//! `format` con `{icon}` de `format-icons` (activated, deactivated). Clic:
//! prender o apagar. Quien pide al compositor que no se apague es la barra
//! (main.rs), con el protocolo idle-inhibit sobre su superficie; este módulo
//! solo dice si se quiere.

use super::{Act, Ev, Module, show, template};
use crate::fmt::{Conf, Val};
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use std::cell::Cell;
use std::rc::Rc;

pub struct Idle {
    conf: Conf,
    /// compartido con la barra: ¿que no se apague?
    on: Rc<Cell<bool>>,
}

impl Idle {
    pub fn new(conf: Conf, on: Rc<Cell<bool>>) -> Idle {
        Idle { conf, on }
    }
}

impl Module for Idle {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let t = look.theme;
        let on = self.on.get();
        let state = if on { "activated" } else { "deactivated" };
        let c = &self.conf;
        let icon = c.icons(state).into_iter().next();
        let tpl = match icon {
            Some(i) => c.str("format").unwrap_or_else(|| "{icon}".into()).replace("{icon}", &i),
            None if on => template(look, &Conf::default(), "", "<span color=\"primary\">󰅶</span>", "caf+"),
            None => template(look, &Conf::default(), "", "󰾪", "caf-"),
        };
        let vals: [(&str, Val); 0] = [];
        show(look, &tpl, &vals, Style::new().fg(if on { t.fg } else { t.muted }))
    }

    fn act(&mut self, act: Act, _: u16) {
        if act == Act::Left {
            self.on.set(!self.on.get());
        }
    }

    fn update(&mut self, _: &Ev) -> bool {
        false
    }
}
