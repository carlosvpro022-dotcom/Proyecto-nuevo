//! Descripcion de los monitores del host.
//!
//! El host enumera sus pantallas y el controlador elige una. Las coordenadas
//! son las del escritorio virtual de Windows, donde el monitor principal
//! empieza en (0,0) y los secundarios pueden tener origen negativo.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// Una pantalla conectada al host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInfo {
    /// Indice estable dentro de la enumeracion actual.
    pub index: u8,
    /// Nombre que reporta el sistema operativo.
    pub name: String,
    /// Ancho en pixeles fisicos.
    pub width: u32,
    /// Alto en pixeles fisicos.
    pub height: u32,
    /// Origen X en el escritorio virtual.
    pub origin_x: i32,
    /// Origen Y en el escritorio virtual.
    pub origin_y: i32,
    /// Factor de escalado del sistema en porcentaje (100 = sin escalar).
    pub scale_percent: u16,
    /// Si es el monitor principal.
    pub is_primary: bool,
    /// Frecuencia de refresco en hercios, si el sistema la reporta.
    pub refresh_hz: Option<u16>,
}

impl MonitorInfo {
    /// Relacion de aspecto como numero decimal.
    ///
    /// Devuelve `0.0` si el alto es cero, en lugar de `NaN`: un monitor sin
    /// dimensiones se rechaza en [`Self::validate`], y aqui conviene un valor
    /// comparable.
    #[must_use]
    pub fn aspect_ratio(&self) -> f32 {
        if self.height == 0 {
            return 0.0;
        }
        self.width as f32 / self.height as f32
    }

    /// Comprueba que las dimensiones son utilizables.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] si el ancho o el alto son cero, lo que indica
    /// que el sistema reporto un monitor que no puede capturarse.
    pub fn validate(&self) -> CoreResult<()> {
        if self.width == 0 || self.height == 0 {
            return Err(CoreError::Malformed(format!(
                "monitor {} con dimensiones nulas ({}x{})",
                self.index, self.width, self.height
            )));
        }
        Ok(())
    }

    /// Traduce un punto normalizado (0.0..=1.0) a coordenadas del escritorio virtual.
    ///
    /// Es la operacion que convierte un toque en la pantalla del iPad en una
    /// posicion de cursor en Windows. Los valores fuera de rango se acotan para
    /// que un cliente manipulado no pueda apuntar fuera del monitor elegido.
    #[must_use]
    pub fn normalized_to_virtual(&self, nx: f32, ny: f32) -> (i32, i32) {
        let nx = nx.clamp(0.0, 1.0);
        let ny = ny.clamp(0.0, 1.0);
        let x = self.origin_x + (nx * self.width.saturating_sub(1) as f32).round() as i32;
        let y = self.origin_y + (ny * self.height.saturating_sub(1) as f32).round() as i32;
        (x, y)
    }
}

/// Conjunto de monitores enumerados por el host.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MonitorLayout(pub Vec<MonitorInfo>);

impl MonitorLayout {
    /// Devuelve el monitor principal, o el primero si ninguno esta marcado.
    #[must_use]
    pub fn primary(&self) -> Option<&MonitorInfo> {
        self.0
            .iter()
            .find(|m| m.is_primary)
            .or_else(|| self.0.first())
    }

    /// Busca un monitor por indice.
    #[must_use]
    pub fn get(&self, index: u8) -> Option<&MonitorInfo> {
        self.0.iter().find(|m| m.index == index)
    }

    /// Exige que el indice exista y lo devuelve.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] si no hay ningun monitor con ese indice.
    pub fn require(&self, index: u8) -> CoreResult<&MonitorInfo> {
        self.get(index).ok_or_else(|| {
            CoreError::Malformed(format!("el monitor {index} no existe en este host"))
        })
    }

    /// Numero de monitores.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Indica si no se detecto ningun monitor.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Valida todos los monitores de la lista.
    ///
    /// # Errors
    ///
    /// El primer error devuelto por [`MonitorInfo::validate`].
    pub fn validate(&self) -> CoreResult<()> {
        for monitor in &self.0 {
            monitor.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(index: u8, w: u32, h: u32, x: i32, y: i32, primary: bool) -> MonitorInfo {
        MonitorInfo {
            index,
            name: format!("Pantalla {index}"),
            width: w,
            height: h,
            origin_x: x,
            origin_y: y,
            scale_percent: 100,
            is_primary: primary,
            refresh_hz: Some(60),
        }
    }

    #[test]
    fn primary_is_preferred_over_first() {
        let layout = MonitorLayout(vec![
            monitor(0, 1_920, 1_080, -1_920, 0, false),
            monitor(1, 2_560, 1_440, 0, 0, true),
        ]);
        assert_eq!(layout.primary().expect("hay principal").index, 1);
    }

    #[test]
    fn primary_falls_back_to_first_when_none_marked() {
        let layout = MonitorLayout(vec![monitor(7, 1_280, 720, 0, 0, false)]);
        assert_eq!(layout.primary().expect("hay uno").index, 7);
    }

    #[test]
    fn empty_layout_has_no_primary() {
        assert!(MonitorLayout::default().primary().is_none());
        assert!(MonitorLayout::default().is_empty());
    }

    #[test]
    fn requesting_a_missing_monitor_fails() {
        let layout = MonitorLayout(vec![monitor(0, 800, 600, 0, 0, true)]);
        assert!(layout.require(0).is_ok());
        assert!(layout.require(3).is_err());
    }

    #[test]
    fn zero_sized_monitor_is_rejected() {
        let layout = MonitorLayout(vec![monitor(0, 0, 600, 0, 0, true)]);
        assert!(layout.validate().is_err());
    }

    #[test]
    fn normalized_points_map_to_monitor_corners() {
        let m = monitor(0, 1_920, 1_080, 0, 0, true);
        assert_eq!(m.normalized_to_virtual(0.0, 0.0), (0, 0));
        assert_eq!(m.normalized_to_virtual(1.0, 1.0), (1_919, 1_079));
        assert_eq!(m.normalized_to_virtual(0.5, 0.5), (960, 540));
    }

    #[test]
    fn secondary_monitor_offset_is_applied() {
        let m = monitor(1, 1_920, 1_080, -1_920, 120, false);
        assert_eq!(m.normalized_to_virtual(0.0, 0.0), (-1_920, 120));
        assert_eq!(m.normalized_to_virtual(1.0, 0.0), (-1, 120));
    }

    #[test]
    fn out_of_range_normalized_input_is_clamped() {
        let m = monitor(0, 1_000, 1_000, 0, 0, true);
        // Un cliente manipulado no puede apuntar fuera de la pantalla elegida.
        assert_eq!(m.normalized_to_virtual(-5.0, 9.0), (0, 999));
    }

    #[test]
    fn aspect_ratio_is_computed() {
        assert!((monitor(0, 1_920, 1_080, 0, 0, true).aspect_ratio() - 1.777_8).abs() < 0.001);
        // El contrato es exactamente 0.0, no "proximo a cero": se compara asi.
        assert!(monitor(0, 100, 0, 0, 0, true).aspect_ratio().abs() < f32::EPSILON);
    }
}
