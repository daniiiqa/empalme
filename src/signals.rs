//! Las señales que los scripts de discada le mandan a la barra, como a
//! waybar: `pkill -RTMIN+10 -x empalme` (el grabador cambió), +9 (AppStrip),
//! +11 (la batería), +12 (los colores cambiaron: releerlos ya); SIGUSR1 esconde o muestra la barra y SIGUSR2 relee el
//! look. Se bloquean en todos los hilos (si no, una señal sin atender cierra
//! empalme) y un hilo las espera con sigwait.

use crate::modules::Ev;
use smithay_client_toolkit::reexports::calloop::channel::Sender;

/// Las RTMIN+N que se atienden.
pub const RT: [i32; 4] = [9, 10, 11, 12];

fn set() -> libc::sigset_t {
    unsafe {
        let mut s: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut s);
        for n in RT {
            libc::sigaddset(&mut s, libc::SIGRTMIN() + n);
        }
        libc::sigaddset(&mut s, libc::SIGUSR1);
        libc::sigaddset(&mut s, libc::SIGUSR2);
        s
    }
}

/// Bloquearlas: antes de abrir cualquier otro hilo, para que lo hereden.
pub fn block() {
    let s = set();
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &s, std::ptr::null_mut()) };
}

/// Que un programa que lanza empalme nazca con las señales sin bloquear: la
/// máscara se hereda, y lo que lancen los scripts (gpu-screen-recorder guarda
/// el replay con SIGUSR1 y pausa con SIGUSR2) no las recibiría.
pub fn clean(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            let mut none: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut none);
            libc::pthread_sigmask(libc::SIG_SETMASK, &none, std::ptr::null_mut());
            Ok(())
        })
    }
}

pub fn watch(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let s = set();
        loop {
            let mut sig = 0;
            if unsafe { libc::sigwait(&s, &mut sig) } != 0 {
                continue;
            }
            let ev = match sig {
                libc::SIGUSR1 => Ev::Toggle,
                libc::SIGUSR2 => Ev::Reload,
                n => Ev::Signal(n - libc::SIGRTMIN()),
            };
            if tx.send(ev).is_err() {
                return;
            }
        }
    });
}
