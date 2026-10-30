//! Un texto fijo del look (`text#menu`, `text#blank`): el `custom/…` de waybar
//! que solo muestra su `format` y corre un comando con `on-click`,
//! `on-click-middle`, `on-click-right`, `on-scroll-up` y `on-scroll-down`.
//! Sin `format` no se ve; los separadores son uno con espacios.

use super::{Act, Ev, Module, show, spawn};
use crate::fmt::Conf;
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use std::process::Command;

pub struct Text {
    conf: Conf,
}

impl Text {
    pub fn new(conf: Conf) -> Text {
        Text { conf }
    }
}

impl Module for Text {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let tpl = self.conf.format("")?;
        // los espacios también ocupan (los separadores)
        if tpl.is_empty() {
            return None;
        }
        show(look, &tpl, &[], Style::new().fg(look.theme.fg)).or_else(|| Some(Line::raw(tpl)))
    }

    fn tooltip(&self, _: &Look) -> Option<String> {
        if self.conf.0["tooltip"].as_bool() != Some(true) && self.conf.str("tooltip-format").is_none() {
            return None;
        }
        self.conf.str("tooltip-format")
    }

    fn act(&mut self, act: Act, _: u16) {
        let key = match act {
            Act::Left => "on-click",
            Act::Middle => "on-click-middle",
            Act::Right => "on-click-right",
            Act::Up => "on-scroll-up",
            Act::Down => "on-scroll-down",
        };
        if let Some(cmd) = self.conf.str(key) {
            spawn(Command::new("sh").arg("-c").arg(cmd));
        }
    }

    fn update(&mut self, _: &Ev) -> bool {
        false
    }
}
