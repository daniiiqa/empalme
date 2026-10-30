//! La hora, con el `format` del look (`[ {:%H:%M:%S} ]`, como en waybar).
//! Clic: el `format-alt` (el día y la fecha); otro clic, de vuelta.

use super::{Act, Ev, Module, show};
use crate::fmt::Conf;
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;

/// `{:%H:%M}` y `{:L%A %d}` con strftime y el idioma del sistema.
fn strftime(tpl: &str) -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as libc::time_t).unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(i) = rest.find("{:") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else { break };
        let spec = rest[i + 2..i + j].trim_start_matches('L');
        if let Ok(c) = std::ffi::CString::new(spec) {
            let mut buf = [0u8; 256];
            let n = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), c.as_ptr(), &tm) };
            out.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

pub struct Clock {
    conf: Conf,
    alt: bool,
    shown: String,
}

impl Clock {
    pub fn new(conf: Conf) -> Clock {
        Clock { conf, alt: false, shown: String::new() }
    }

    fn text(&self) -> String {
        let f = if self.alt {
            self.conf.str("format-alt").unwrap_or_else(|| "{:%a %d %b  %H:%M}".into())
        } else {
            self.conf.str("format").unwrap_or_else(|| "{:%H:%M}".into())
        };
        strftime(&f)
    }
}

impl Module for Clock {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        show(look, &self.text(), &[], Style::new().fg(look.theme.fg))
    }

    fn act(&mut self, act: Act, _: u16) {
        if act == Act::Left {
            self.alt = !self.alt;
            self.shown = self.text();
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        if !matches!(ev, Ev::Tick) {
            return false;
        }
        let now = self.text();
        if now != self.shown {
            self.shown = now;
            return true;
        }
        false
    }
}
