//! La hora. Clic: también el día y la fecha (otro clic, solo la hora).

use super::{Act, Ev, Module, spanish};
use discada_kit::look::Look;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};

#[repr(C)]
#[derive(Default)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    gmtoff: i64,
    zone: usize,
}

unsafe extern "C" {
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
}

fn now() -> Tm {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut tm = Tm::default();
    unsafe { localtime_r(&secs, &mut tm) };
    tm
}

pub struct Clock {
    long: bool,
    shown: (i32, i32),
}

impl Clock {
    pub fn new() -> Clock {
        Clock { long: false, shown: (-1, -1) }
    }
}

impl Module for Clock {
    fn line(&self, look: &Look) -> Option<Line<'static>> {
        let t = look.theme;
        let tm = now();
        let hm = format!("{:02}:{:02}", tm.hour, tm.min);
        if !self.long {
            return Some(Line::from(Span::styled(hm, Style::new().fg(t.fg).bold())));
        }
        let (days, months): (&[&str], &[&str]) = if spanish() {
            (&["dom", "lun", "mar", "mié", "jue", "vie", "sáb"], &["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"])
        } else {
            (&["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"], &["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"])
        };
        let date = format!("{} {} {}  ", days[tm.wday as usize % 7], tm.mday, months[tm.mon as usize % 12]);
        Some(Line::from(vec![
            Span::styled(look.tx(&date), Style::new().fg(t.muted)),
            Span::styled(hm, Style::new().fg(t.fg).bold()),
        ]))
    }

    fn act(&mut self, act: Act, _: u16) {
        if act == Act::Left {
            self.long = !self.long;
            self.shown = (-1, -1);
        }
    }

    fn update(&mut self, ev: &Ev) -> bool {
        if !matches!(ev, Ev::Tick) {
            return false;
        }
        let tm = now();
        let k = (tm.hour, tm.min);
        if k != self.shown {
            self.shown = k;
            return true;
        }
        false
    }
}
