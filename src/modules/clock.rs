//! La hora, con el `format` del look (`[ {:%H:%M:%S} ]`, como en waybar).
//! Clic: el `format-alt` (el día y la fecha); otro clic, de vuelta. El
//! tooltip, `tooltip-format` con `{calendar}`: el mes, con hoy marcado como
//! diga `calendar.format.today` (`<b><u>{}</u></b>`).

use super::{Act, Ev, Module, show};
use crate::fmt::Conf;
use discada_kit::look::Look;
use ratatui::style::Style;
use ratatui::text::Line;
use serde_json::Value;

fn now_tm() -> libc::tm {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as libc::time_t).unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    tm
}

/// Un formato de strftime para `tm`.
fn ftime(spec: &str, tm: &libc::tm) -> String {
    let Ok(c) = std::ffi::CString::new(spec) else { return String::new() };
    let mut buf = [0u8; 256];
    let n = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), c.as_ptr(), tm) };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// El día con que empieza la semana en el idioma del sistema (0 domingo,
/// 1 lunes), de `locale first_weekday`; se pregunta una vez.
fn first_weekday() -> i32 {
    static FIRST: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *FIRST.get_or_init(|| {
        std::process::Command::new("locale")
            .arg("first_weekday")
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<i32>().ok())
            .map_or(1, |d| (d - 1).rem_euclid(7))
    })
}

/// El mes como en waybar: el nombre, los días de la semana y las semanas;
/// cada parte con su `calendar.format` (months, weekdays, days, today).
fn calendar(conf: &Value) -> String {
    let tm = now_tm();
    let f = &conf["format"];
    let wrap = |key: &str, text: &str| f[key].as_str().map_or_else(|| text.to_string(), |w| w.replace("{}", text));
    let mut first = tm;
    first.tm_mday = 1;
    unsafe { libc::mktime(&mut first) };
    let mut last = tm;
    last.tm_mon += 1;
    last.tm_mday = 0;
    unsafe { libc::mktime(&mut last) };
    let start = first_weekday();
    let title = ftime("%B %Y", &tm);
    let pad = 20usize.saturating_sub(title.chars().count()) / 2;
    let mut out = vec![format!("{}{}", " ".repeat(pad), wrap("months", &title))];
    let heads: Vec<String> = (0..7)
        .map(|i| {
            let mut d = tm;
            d.tm_wday = (start + i) % 7;
            let name: String = ftime("%a", &d).chars().take(2).collect();
            format!("{name:>2}")
        })
        .collect();
    out.push(wrap("weekdays", &heads.join(" ")));
    let lead = (first.tm_wday - start).rem_euclid(7);
    let mut row: Vec<String> = vec!["  ".to_string(); lead as usize];
    for day in 1..=last.tm_mday {
        let n = format!("{day:>2}");
        row.push(if day == tm.tm_mday {
            // el espacio de relleno queda afuera del marcado
            let (sp, num) = n.split_at(n.len() - day.to_string().len());
            format!("{sp}{}", wrap("today", num))
        } else {
            wrap("days", &n)
        });
        if row.len() == 7 {
            out.push(row.join(" "));
            row.clear();
        }
    }
    if !row.is_empty() {
        out.push(row.join(" "));
    }
    out.join("\n")
}

/// `{:%H:%M}` y `{:L%A %d}` con strftime y el idioma del sistema.
fn strftime(tpl: &str) -> String {
    let tm = now_tm();
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(i) = rest.find("{:") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else { break };
        out.push_str(&ftime(rest[i + 2..i + j].trim_start_matches('L'), &tm));
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

    fn tooltip(&self, _: &Look) -> Option<String> {
        let tpl = self.conf.tooltip("", "{calendar}")?;
        Some(strftime(&tpl).replace("{calendar}", &calendar(&self.conf.0["calendar"])))
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
