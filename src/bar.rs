//! Cómo es la barra en el look actual: `~/.config/empalme/bar.json`, que
//! LookBuild.py escribe desde el `[bar]` del look.toml. Sin archivo, una barra
//! sencilla arriba.

use serde_json::Value;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, PartialEq)]
pub enum Style {
    /// cada módulo en su isla
    Islands,
    /// una sola pieza de orilla a orilla
    Solid,
}

#[derive(Clone)]
pub struct Bar {
    pub bottom: bool,
    /// alto en pixeles lógicos
    pub height: u32,
    /// arriba, derecha, abajo, izquierda
    pub margin: [i32; 4],
    pub style: Style,
    pub radius: f32,
    /// tamaño de la letra en pixeles; None: el de kitty
    pub font_size: Option<f32>,
    /// celdas de aire a cada lado del contenido de un módulo
    pub pad: u16,
    /// celdas entre módulos
    pub gap: u16,
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
    /// `[bar.modules.<nombre>]`: los formatos y ajustes de cada módulo
    pub modules: Value,
    /// `[bar.groups.<nombre>]`: los `group/<nombre>` (con `drawer`, cajones)
    pub groups: Value,
    /// `[bar.hover]`: los colores (fg, bg) del módulo bajo el mouse, y en
    /// cuáles (`modules`; sin lista, en todos)
    pub hover: Value,
    /// `[bar.hover-workspaces]`: los del botón de workspace bajo el mouse
    /// (fg, bg, y active-fg, active-bg si es el activo)
    pub hover_ws: Value,
}

impl Default for Bar {
    fn default() -> Bar {
        Bar {
            bottom: false,
            height: 31,
            margin: [8, 8, 0, 8],
            style: Style::Islands,
            radius: 6.0,
            font_size: None,
            pad: 1,
            gap: 1,
            left: vec!["workspaces".into()],
            center: vec!["clock".into()],
            right: ["temperature", "cpu", "memory", "disk", "audio", "mic", "network", "bluetooth", "battery", "power"]
                .map(String::from)
                .to_vec(),
            modules: Value::Null,
            groups: Value::Null,
            hover: Value::Null,
            hover_ws: Value::Null,
        }
    }
}

pub fn path() -> PathBuf {
    let cfg = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    cfg.join("empalme/bar.json")
}

pub fn stamp() -> Option<SystemTime> {
    std::fs::metadata(path()).and_then(|m| m.modified()).ok()
}

pub fn load() -> Bar {
    let mut b = Bar::default();
    let Ok(text) = std::fs::read_to_string(path()) else { return b };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return b };
    let list = |k: &str| v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect());
    if let Some(p) = v["position"].as_str() {
        b.bottom = p == "bottom";
    }
    if let Some(h) = v["height"].as_u64() {
        b.height = h.clamp(16, 120) as u32;
    }
    if let Some(m) = v["margin"].as_array() {
        for (i, x) in m.iter().take(4).enumerate() {
            b.margin[i] = x.as_i64().unwrap_or(0) as i32;
        }
    }
    if v["style"].as_str() == Some("solid") {
        b.style = Style::Solid;
    }
    if let Some(r) = v["radius"].as_f64() {
        b.radius = r as f32;
    }
    b.font_size = v["font-size"].as_f64().map(|f| f.clamp(6.0, 60.0) as f32);
    if let Some(p) = v["pad"].as_u64() {
        b.pad = p as u16;
    }
    if let Some(g) = v["gap"].as_u64() {
        b.gap = g as u16;
    }
    b.left = list("left").unwrap_or(b.left);
    b.center = list("center").unwrap_or(b.center);
    b.right = list("right").unwrap_or(b.right);
    b.modules = v["modules"].clone();
    b.groups = v["groups"].clone();
    b.hover = v["hover"].clone();
    b.hover_ws = v["hover-workspaces"].clone();
    b
}
