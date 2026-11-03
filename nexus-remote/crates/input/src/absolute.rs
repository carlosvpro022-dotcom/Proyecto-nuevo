//! Conversion de coordenadas del escritorio virtual a las que espera `SendInput`.
//!
//! Con `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`, Windows no interpreta
//! `dx` y `dy` como pixeles: son una fraccion del escritorio virtual completo,
//! normalizada al rango 0..65535.
//!
//! ## Por que la formula no es la obvia
//!
//! Windows reconstruye el pixel haciendo, en esencia:
//!
//! ```text
//! pixel = origen + (valor * ancho) / 65536
//! ```
//!
//! Es una division entera, que trunca. Si se calcula el valor con la formula
//! directa `valor = pixel * 65536 / ancho`, el truncado de la ida y el de la
//! vuelta se suman y el cursor cae un pixel antes de lo pedido en buena parte
//! de la pantalla.
//!
//! La solucion es apuntar al **centro** de la banda de valores que
//! corresponden a ese pixel, no a su borde:
//!
//! ```text
//! valor = ((2 * pixel + 1) * 32768) / ancho
//! ```
//!
//! La prueba `cada_pixel_vuelve_a_si_mismo` recorre resoluciones reales pixel a
//! pixel y comprueba que la ida y la vuelta coinciden siempre.
//!
//! Este modulo **no depende de Windows**: es aritmetica y se prueba en
//! cualquier plataforma.

use crate::error::{InputError, InputResult};

/// Valor maximo que admite la coordenada normalizada.
pub const ABSOLUTE_MAX: i32 = 65_535;

/// Divisor que usa Windows al reconstruir el pixel.
const ABSOLUTE_RANGE: i64 = 65_536;

/// Rectangulo que ocupa el escritorio virtual completo.
///
/// Es la union de todos los monitores. El monitor principal empieza en (0,0),
/// asi que el origen puede ser negativo si hay pantallas a la izquierda o por
/// encima.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualDesktop {
    /// Borde izquierdo. `SM_XVIRTUALSCREEN`.
    pub left: i32,
    /// Borde superior. `SM_YVIRTUALSCREEN`.
    pub top: i32,
    /// Ancho total. `SM_CXVIRTUALSCREEN`.
    pub width: i32,
    /// Alto total. `SM_CYVIRTUALSCREEN`.
    pub height: i32,
}

impl VirtualDesktop {
    /// Construye el rectangulo comprobando que es utilizable.
    ///
    /// # Errors
    ///
    /// [`InputError::SystemRejected`] si el ancho o el alto no son positivos,
    /// lo que indica que el sistema devolvio metricas incoherentes.
    pub fn new(left: i32, top: i32, width: i32, height: i32) -> InputResult<Self> {
        if width <= 0 || height <= 0 {
            return Err(InputError::SystemRejected(format!(
                "el escritorio virtual mide {width}x{height}"
            )));
        }
        Ok(Self {
            left,
            top,
            width,
            height,
        })
    }

    /// Indica si un punto cae dentro del escritorio virtual.
    #[must_use]
    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left
            && y >= self.top
            && x < self.left.saturating_add(self.width)
            && y < self.top.saturating_add(self.height)
    }

    /// Convierte un punto del escritorio virtual a coordenadas de `SendInput`.
    ///
    /// El punto se acota al escritorio antes de convertir: un cliente
    /// manipulado no debe poder pedir una posicion fuera de la pantalla, y
    /// tampoco tiene sentido intentarlo.
    #[must_use]
    pub fn to_absolute(&self, x: i32, y: i32) -> (i32, i32) {
        (
            normalize(x - self.left, self.width),
            normalize(y - self.top, self.height),
        )
    }

    /// Operacion inversa: que pixel reconstruye Windows a partir de un valor.
    ///
    /// Existe para poder comprobar en las pruebas que la ida y la vuelta
    /// coinciden. Reproduce lo que hace el sistema.
    #[must_use]
    pub const fn from_absolute(&self, dx: i32, dy: i32) -> (i32, i32) {
        let px = (dx as i64 * self.width as i64) / ABSOLUTE_RANGE;
        let py = (dy as i64 * self.height as i64) / ABSOLUTE_RANGE;
        (self.left + px as i32, self.top + py as i32)
    }
}

/// Lleva un desplazamiento en pixeles al rango 0..65535.
fn normalize(offset: i32, extent: i32) -> i32 {
    debug_assert!(extent > 0, "el constructor garantiza que es positivo");

    let clamped = i64::from(offset.clamp(0, extent - 1));
    // Centro de la banda de valores que Windows traduce a este pixel.
    let value = ((2 * clamped + 1) * (ABSOLUTE_RANGE / 2)) / i64::from(extent);
    value.clamp(0, i64::from(ABSOLUTE_MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resoluciones reales, incluida una configuracion de dos monitores con
    /// origen negativo.
    fn escritorios() -> Vec<VirtualDesktop> {
        vec![
            VirtualDesktop::new(0, 0, 1_920, 1_080).expect("fhd"),
            VirtualDesktop::new(0, 0, 2_560, 1_440).expect("qhd"),
            VirtualDesktop::new(0, 0, 3_840, 2_160).expect("4k"),
            VirtualDesktop::new(0, 0, 1_366, 768).expect("portatil"),
            // Monitor secundario a la izquierda: el origen es negativo.
            VirtualDesktop::new(-1_920, 0, 3_840, 1_080).expect("dos monitores"),
            // Y uno por encima.
            VirtualDesktop::new(0, -1_080, 1_920, 2_160).expect("apilados"),
            // Tamanos raros, para que la aritmetica no dependa de potencias de dos.
            VirtualDesktop::new(0, 0, 1_001, 777).expect("impar"),
        ]
    }

    #[test]
    fn cada_pixel_vuelve_a_si_mismo() {
        // Es la prueba que justifica la formula: si la ida y la vuelta no
        // coincidieran, el cursor caeria un pixel desplazado.
        for escritorio in escritorios() {
            for px in 0..escritorio.width {
                let x = escritorio.left + px;
                let (dx, _) = escritorio.to_absolute(x, escritorio.top);
                let (vuelta, _) = escritorio.from_absolute(dx, 0);
                assert_eq!(
                    vuelta, x,
                    "en {}x{} el pixel x={x} volvio como {vuelta} (dx={dx})",
                    escritorio.width, escritorio.height
                );
            }
            for py in 0..escritorio.height {
                let y = escritorio.top + py;
                let (_, dy) = escritorio.to_absolute(escritorio.left, y);
                let (_, vuelta) = escritorio.from_absolute(0, dy);
                assert_eq!(
                    vuelta, y,
                    "en {}x{} el pixel y={y} volvio como {vuelta} (dy={dy})",
                    escritorio.width, escritorio.height
                );
            }
        }
    }

    #[test]
    fn las_esquinas_caen_donde_deben() {
        let d = VirtualDesktop::new(0, 0, 1_920, 1_080).expect("fhd");

        let (dx, dy) = d.to_absolute(0, 0);
        assert_eq!(d.from_absolute(dx, dy), (0, 0));

        let (dx, dy) = d.to_absolute(1_919, 1_079);
        assert_eq!(d.from_absolute(dx, dy), (1_919, 1_079));
    }

    #[test]
    fn el_resultado_siempre_cabe_en_el_rango() {
        for escritorio in escritorios() {
            for (x, y) in [
                (escritorio.left, escritorio.top),
                (
                    escritorio.left + escritorio.width - 1,
                    escritorio.top + escritorio.height - 1,
                ),
                (i32::MIN / 2, i32::MIN / 2),
                (i32::MAX / 2, i32::MAX / 2),
            ] {
                let (dx, dy) = escritorio.to_absolute(x, y);
                assert!((0..=ABSOLUTE_MAX).contains(&dx), "dx fuera de rango: {dx}");
                assert!((0..=ABSOLUTE_MAX).contains(&dy), "dy fuera de rango: {dy}");
            }
        }
    }

    #[test]
    fn los_puntos_fuera_del_escritorio_se_acotan() {
        let d = VirtualDesktop::new(0, 0, 1_920, 1_080).expect("fhd");

        // Muy a la izquierda y muy arriba: queda en la esquina superior izquierda.
        let (dx, dy) = d.to_absolute(-5_000, -5_000);
        assert_eq!(d.from_absolute(dx, dy), (0, 0));

        // Muy a la derecha y muy abajo: ultima fila y columna.
        let (dx, dy) = d.to_absolute(99_999, 99_999);
        assert_eq!(d.from_absolute(dx, dy), (1_919, 1_079));
    }

    #[test]
    fn el_origen_negativo_se_tiene_en_cuenta() {
        // Monitor secundario a la izquierda del principal.
        let d = VirtualDesktop::new(-1_920, 0, 3_840, 1_080).expect("dos monitores");

        // La esquina del secundario es el origen del escritorio virtual.
        let (dx, _) = d.to_absolute(-1_920, 0);
        assert_eq!(d.from_absolute(dx, 0).0, -1_920);

        // El origen del principal cae a la mitad.
        let (dx, _) = d.to_absolute(0, 0);
        assert_eq!(d.from_absolute(dx, 0).0, 0);
        assert!(
            (32_000..34_000).contains(&dx),
            "deberia estar cerca de la mitad: {dx}"
        );
    }

    #[test]
    fn contiene_distingue_dentro_de_fuera() {
        let d = VirtualDesktop::new(-1_920, 0, 3_840, 1_080).expect("dos monitores");
        assert!(d.contains(-1_920, 0));
        assert!(d.contains(0, 0));
        assert!(d.contains(1_919, 1_079));
        assert!(!d.contains(-1_921, 0));
        assert!(!d.contains(1_920, 0));
        assert!(!d.contains(0, 1_080));
        assert!(!d.contains(0, -1));
    }

    #[test]
    fn un_escritorio_sin_tamano_se_rechaza() {
        assert!(VirtualDesktop::new(0, 0, 0, 1_080).is_err());
        assert!(VirtualDesktop::new(0, 0, 1_920, 0).is_err());
        assert!(VirtualDesktop::new(0, 0, -1, -1).is_err());
    }

    #[test]
    fn un_escritorio_de_un_solo_pixel_no_divide_por_cero() {
        let d = VirtualDesktop::new(0, 0, 1, 1).expect("un pixel");
        let (dx, dy) = d.to_absolute(0, 0);
        assert_eq!(d.from_absolute(dx, dy), (0, 0));
    }

    #[test]
    fn la_conversion_es_monotona() {
        // Moverse a la derecha nunca debe producir un valor menor.
        let d = VirtualDesktop::new(0, 0, 2_560, 1_440).expect("qhd");
        let mut anterior = -1;
        for x in 0..d.width {
            let (dx, _) = d.to_absolute(x, 0);
            assert!(
                dx >= anterior,
                "en x={x} el valor bajo de {anterior} a {dx}"
            );
            anterior = dx;
        }
    }
}
