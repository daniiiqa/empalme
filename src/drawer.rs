//! Los cajones, como los `group/…` con `drawer` de waybar: el primer módulo
//! del grupo siempre se ve y los demás salen al pasar el mouse por encima,
//! deslizándose en `transition-duration` milisegundos; con
//! `transition-left-to-right = false` salen hacia la izquierda.

pub struct Drawer {
    /// los demás salen a la derecha del primero
    pub right: bool,
    /// cuánto tarda en abrir o cerrar, en milisegundos
    pub ms: f32,
    /// qué tanto está abierto, de 0 a 1
    pub p: f32,
    /// ¿el mouse está encima?
    pub open: bool,
}

impl Drawer {
    pub fn new(conf: &serde_json::Value) -> Drawer {
        Drawer {
            right: conf["transition-left-to-right"].as_bool().unwrap_or(true),
            ms: conf["transition-duration"].as_f64().unwrap_or(500.0).max(1.0) as f32,
            p: 0.0,
            open: false,
        }
    }

    pub fn moving(&self) -> bool {
        self.p != if self.open { 1.0 } else { 0.0 }
    }

    /// Avanza `dt` segundos hacia abierto o cerrado.
    pub fn step(&mut self, dt: f32) {
        let d = dt * 1000.0 / self.ms;
        self.p = if self.open { (self.p + d).min(1.0) } else { (self.p - d).max(0.0) };
    }

    /// Qué tanto se ve, con aceleración suave al principio y al final.
    pub fn shown(&self) -> f32 {
        let p = self.p;
        p * p * (3.0 - 2.0 * p)
    }
}

/// Lo que se ve de cada módulo de un cajón que se está abriendo, en pixeles
/// (para que se deslice suave, no a saltos de una letra): de los que salen
/// (en orden de la barra, con su ancho completo) cuánto se ve y cuánto queda
/// escondido a su izquierda. El espacio `gap` antes de cada uno también se va
/// abriendo.
pub fn reveal(d: &Drawer, widths: &[f32], gap: f32) -> Vec<(f32, f32)> {
    let total: f32 = widths.iter().map(|w| w + gap).sum();
    let mut left = total * d.shown();
    let mut out = vec![(0.0, 0.0); widths.len()];
    // se van abriendo desde el que está junto al primero del grupo
    let order: Vec<usize> = if d.right { (0..widths.len()).collect() } else { (0..widths.len()).rev().collect() };
    for i in order {
        let take = left.min(widths[i] + gap);
        left -= take;
        let vis = (take - gap).max(0.0);
        // a la derecha se ve su principio; a la izquierda, su final
        out[i] = (vis, if d.right { 0.0 } else { widths[i] - vis });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abrir() {
        let mut d = Drawer { right: true, ms: 500.0, p: 0.0, open: true };
        assert_eq!(reveal(&d, &[50.0, 40.0], 10.0), vec![(0.0, 0.0), (0.0, 0.0)]);
        d.p = 1.0;
        assert_eq!(reveal(&d, &[50.0, 40.0], 10.0), vec![(50.0, 0.0), (40.0, 0.0)]);
        d.p = 0.5; // la mitad de 110: el primero completo
        assert_eq!(reveal(&d, &[50.0, 40.0], 10.0), vec![(45.0, 0.0), (0.0, 0.0)]);
        d.right = false; // a la izquierda: primero el de junto (el último); lo que sobra es el espacio
        assert_eq!(reveal(&d, &[50.0, 40.0], 10.0), vec![(0.0, 50.0), (40.0, 0.0)]);
    }
}
