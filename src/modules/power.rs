//! El botón de apagar. Clic: el menú de salir (Wlogout.sh); clic derecho:
//! cambiar el blur (ChangeBlur.sh).

use super::{Act, Ev, Module, l, script, show, template};
use crate::fmt::Conf;
use disco::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;

pub struct Power {
    conf: Conf,
}

impl Power {
    pub fn new(conf: Conf) -> Power {
        Power { conf }
    }
}

impl Module for Power {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let tpl = template(look, &self.conf, "", "<span color=\"red\">󰐥</span>", "[x]");
        show(look, &tpl, &[], Style::new().fg(look.theme.fg))
    }

    fn tooltip(&self, _: &Look) -> Option<String> {
        self.conf.tooltip("", l("Clic: salir\nClic derecho: cambiar el blur", "Click: logout menu\nRight click: change blur"))
    }

    fn act(&mut self, act: Act, _: u16) {
        match act {
            Act::Left => script("Wlogout.sh", &[]),
            Act::Right => script("ChangeBlur.sh", &[]),
            _ => {}
        }
    }

    fn update(&mut self, _: &Ev) -> bool {
        false
    }
}
