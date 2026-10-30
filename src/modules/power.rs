//! El botón de apagar. Clic: el menú de salir (Wlogout.sh); clic derecho:
//! cambiar el blur (ChangeBlur.sh).

use super::{Act, Ev, Module, script, show, template};
use crate::fmt::Conf;
use discada_kit::look::Look;
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
