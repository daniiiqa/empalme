//! Hyprland por su socket: preguntas (`j/workspaces`…) y los eventos de la
//! sesión. Para *mandar* algo (cambiar de workspace) empalme usa Hypr.sh de
//! discada, que habla igual con la config .conf que con la de Lua.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use crate::modules::Ev;
use smithay_client_toolkit::reexports::calloop::channel::Sender;

fn dir() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let run = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    Some(PathBuf::from(run).join("hypr").join(sig))
}

/// Una pregunta en JSON (`workspaces`, `activeworkspace`…).
pub fn query(what: &str) -> Option<Value> {
    let mut s = UnixStream::connect(dir()?.join(".socket.sock")).ok()?;
    s.write_all(format!("j/{what}").as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    serde_json::from_str(&out).ok()
}

/// Los eventos de Hyprland (`workspace>>2`…), una línea por mensaje, en otro
/// hilo; si se cae el socket, vuelve a intentar.
pub fn events(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        loop {
            if let Some(path) = dir().map(|d| d.join(".socket2.sock"))
                && let Ok(s) = UnixStream::connect(path) {
                    for line in BufReader::new(s).lines().map_while(Result::ok) {
                        if tx.send(Ev::Hypr(line)).is_err() {
                            return;
                        }
                    }
                }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}
