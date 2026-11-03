//! Perfiles de calidad de video y su validacion.
//!
//! El controlador pide un perfil y el host lo acota a lo que puede sostener.
//! Los limites viven aqui, en codigo compartido, para que cliente y host no
//! discrepen sobre lo que es un valor valido.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// Preajuste pedido desde la interfaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityPreset {
    /// Prioriza fluidez sobre nitidez. Util en redes moviles lentas.
    Fluido,
    /// Reparto equilibrado entre nitidez y fluidez.
    Equilibrado,
    /// Prioriza nitidez. Pensado para Wi-Fi o fibra.
    Nitido,
    /// El host ajusta el perfil segun el ancho de banda medido.
    Automatico,
}

/// Parametros concretos de codificacion de video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityProfile {
    /// Fotogramas por segundo objetivo.
    pub target_fps: u16,
    /// Tasa de bits objetivo en kilobits por segundo.
    pub target_bitrate_kbps: u32,
    /// Escala aplicada a la resolucion nativa, en porcentaje (25..=100).
    pub scale_percent: u8,
    /// Si el host puede bajar la calidad sin que el cliente lo pida.
    pub allow_adaptive_downscale: bool,
}

impl QualityProfile {
    /// Minimo de fotogramas por segundo admitido.
    pub const MIN_FPS: u16 = 1;
    /// Maximo de fotogramas por segundo admitido.
    pub const MAX_FPS: u16 = 120;
    /// Minimo de tasa de bits admitido, en kbps.
    pub const MIN_BITRATE_KBPS: u32 = 200;
    /// Maximo de tasa de bits admitido, en kbps.
    pub const MAX_BITRATE_KBPS: u32 = 60_000;
    /// Escala minima admitida, en porcentaje.
    pub const MIN_SCALE: u8 = 25;
    /// Escala maxima admitida, en porcentaje.
    pub const MAX_SCALE: u8 = 100;

    /// Construye el perfil correspondiente a un preajuste.
    #[must_use]
    pub const fn from_preset(preset: QualityPreset) -> Self {
        match preset {
            QualityPreset::Fluido => Self {
                target_fps: 30,
                target_bitrate_kbps: 1_500,
                scale_percent: 60,
                allow_adaptive_downscale: true,
            },
            QualityPreset::Equilibrado => Self {
                target_fps: 30,
                target_bitrate_kbps: 5_000,
                scale_percent: 85,
                allow_adaptive_downscale: true,
            },
            QualityPreset::Nitido => Self {
                target_fps: 60,
                target_bitrate_kbps: 15_000,
                scale_percent: 100,
                allow_adaptive_downscale: false,
            },
            QualityPreset::Automatico => Self {
                target_fps: 60,
                target_bitrate_kbps: 8_000,
                scale_percent: 100,
                allow_adaptive_downscale: true,
            },
        }
    }

    /// Valida que los parametros esten dentro de los rangos admitidos.
    ///
    /// Se usa en el host sobre todo valor que llegue por la red: un cliente
    /// manipulado podria pedir 10.000 fps o una tasa absurda.
    ///
    /// # Errors
    ///
    /// [`CoreError::OutOfRange`] indicando el campo y los limites admitidos.
    pub fn validate(&self) -> CoreResult<()> {
        range(
            "target_fps",
            i64::from(self.target_fps),
            i64::from(Self::MIN_FPS),
            i64::from(Self::MAX_FPS),
        )?;
        range(
            "target_bitrate_kbps",
            i64::from(self.target_bitrate_kbps),
            i64::from(Self::MIN_BITRATE_KBPS),
            i64::from(Self::MAX_BITRATE_KBPS),
        )?;
        range(
            "scale_percent",
            i64::from(self.scale_percent),
            i64::from(Self::MIN_SCALE),
            i64::from(Self::MAX_SCALE),
        )?;
        Ok(())
    }

    /// Devuelve una copia acotada a los rangos admitidos.
    ///
    /// Alternativa a `validate` cuando se prefiere degradar en lugar de
    /// rechazar: el host la usa al aplicar un cambio en caliente.
    #[must_use]
    pub const fn clamped(mut self) -> Self {
        if self.target_fps < Self::MIN_FPS {
            self.target_fps = Self::MIN_FPS;
        } else if self.target_fps > Self::MAX_FPS {
            self.target_fps = Self::MAX_FPS;
        }
        if self.target_bitrate_kbps < Self::MIN_BITRATE_KBPS {
            self.target_bitrate_kbps = Self::MIN_BITRATE_KBPS;
        } else if self.target_bitrate_kbps > Self::MAX_BITRATE_KBPS {
            self.target_bitrate_kbps = Self::MAX_BITRATE_KBPS;
        }
        if self.scale_percent < Self::MIN_SCALE {
            self.scale_percent = Self::MIN_SCALE;
        } else if self.scale_percent > Self::MAX_SCALE {
            self.scale_percent = Self::MAX_SCALE;
        }
        self
    }

    /// Resolucion de salida al aplicar la escala sobre un tamano nativo.
    ///
    /// Las dimensiones se redondean a par porque los codificadores de video con
    /// submuestreo de croma 4:2:0 (H.264, VP8, AV1) lo exigen.
    #[must_use]
    pub const fn scaled_resolution(&self, native_width: u32, native_height: u32) -> (u32, u32) {
        let w = native_width * self.scale_percent as u32 / 100;
        let h = native_height * self.scale_percent as u32 / 100;
        (even_at_least_two(w), even_at_least_two(h))
    }
}

const fn even_at_least_two(value: u32) -> u32 {
    if value < 2 { 2 } else { value & !1 }
}

fn range(field: &'static str, value: i64, min: i64, max: i64) -> CoreResult<()> {
    if value < min || value > max {
        return Err(CoreError::OutOfRange {
            field,
            value,
            min,
            max,
        });
    }
    Ok(())
}

impl Default for QualityProfile {
    fn default() -> Self {
        Self::from_preset(QualityPreset::Automatico)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_valid() {
        for preset in [
            QualityPreset::Fluido,
            QualityPreset::Equilibrado,
            QualityPreset::Nitido,
            QualityPreset::Automatico,
        ] {
            QualityProfile::from_preset(preset)
                .validate()
                .unwrap_or_else(|e| panic!("{preset:?} deberia ser valido: {e}"));
        }
    }

    #[test]
    fn absurd_fps_is_rejected() {
        let profile = QualityProfile {
            target_fps: 10_000,
            ..QualityProfile::default()
        };
        let err = profile.validate().unwrap_err();
        assert!(matches!(
            err,
            CoreError::OutOfRange {
                field: "target_fps",
                ..
            }
        ));
    }

    #[test]
    fn zero_values_are_rejected() {
        let profile = QualityProfile {
            target_fps: 0,
            target_bitrate_kbps: 0,
            scale_percent: 0,
            allow_adaptive_downscale: false,
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn clamping_pulls_values_into_range() {
        let profile = QualityProfile {
            target_fps: 10_000,
            target_bitrate_kbps: 1,
            scale_percent: 250,
            allow_adaptive_downscale: true,
        }
        .clamped();
        assert_eq!(profile.target_fps, QualityProfile::MAX_FPS);
        assert_eq!(
            profile.target_bitrate_kbps,
            QualityProfile::MIN_BITRATE_KBPS
        );
        assert_eq!(profile.scale_percent, QualityProfile::MAX_SCALE);
        profile.validate().expect("tras acotar debe ser valido");
    }

    #[test]
    fn scaled_resolution_is_even() {
        let profile = QualityProfile {
            scale_percent: 85,
            ..QualityProfile::default()
        };
        let (w, h) = profile.scaled_resolution(1_920, 1_080);
        assert_eq!(w % 2, 0, "ancho impar: {w}");
        assert_eq!(h % 2, 0, "alto impar: {h}");
        assert_eq!((w, h), (1_632, 918));
    }

    #[test]
    fn full_scale_keeps_native_resolution() {
        let profile = QualityProfile {
            scale_percent: 100,
            ..QualityProfile::default()
        };
        assert_eq!(profile.scaled_resolution(2_560, 1_440), (2_560, 1_440));
    }

    #[test]
    fn tiny_sources_never_collapse_to_zero() {
        let profile = QualityProfile {
            scale_percent: 25,
            ..QualityProfile::default()
        };
        assert_eq!(profile.scaled_resolution(4, 4), (2, 2));
    }
}
