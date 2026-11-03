//! # nexus-audio
//!
//! Contrato de captura del audio del sistema y control de volumen.
//!
//! # Estado de implementacion
//!
//! **No hay backend de Windows todavia.** La captura del audio de salida con
//! WASAPI en modo bucle (`loopback`) corresponde a la etapa 5.
//!
//! Lo que esta implementado y probado es el formato de audio con su validacion,
//! el estado de volumen y silencio (que debe comportarse exactamente igual en
//! el iPad y en el host), y el contrato [`AudioCapturer`].

#![forbid(unsafe_code)]

use std::fmt;

use nexus_core::Timestamp;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fallos de captura de audio.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AudioError {
    /// La plataforma actual no tiene backend de audio.
    #[error("captura de audio no soportada en esta plataforma ({platform})")]
    UnsupportedPlatform {
        /// Plataforma detectada en tiempo de compilacion.
        platform: &'static str,
    },

    /// No hay dispositivo de salida que capturar.
    #[error("no hay dispositivo de audio de salida disponible")]
    NoOutputDevice,

    /// El formato pedido no es valido.
    #[error("formato de audio invalido: {0}")]
    InvalidFormat(String),

    /// Todavia no hay muestras nuevas.
    #[error("sin muestras nuevas disponibles")]
    WouldBlock,

    /// Error del sistema de audio.
    #[error("error del sistema de audio: {0}")]
    System(String),
}

/// Resultado corto del crate.
pub type AudioResult<T> = Result<T, AudioError>;

/// Formato de las muestras de audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    /// Muestras por segundo.
    pub sample_rate_hz: u32,
    /// Numero de canales (1 mono, 2 estereo).
    pub channels: u8,
    /// Bits por muestra.
    pub bits_per_sample: u8,
}

impl AudioFormat {
    /// Formato de referencia: 48 kHz, estereo, 16 bits.
    ///
    /// Es la frecuencia nativa de Windows y la que espera Opus, el codec de
    /// audio de WebRTC, asi que evita remuestrear en el camino.
    pub const STEREO_48K: Self = Self {
        sample_rate_hz: 48_000,
        channels: 2,
        bits_per_sample: 16,
    };

    /// Frecuencias de muestreo admitidas.
    pub const VALID_SAMPLE_RATES: [u32; 5] = [8_000, 16_000, 24_000, 44_100, 48_000];

    /// Comprueba que el formato es utilizable.
    ///
    /// # Errors
    ///
    /// [`AudioError::InvalidFormat`] describiendo el campo fuera de rango.
    pub fn validate(&self) -> AudioResult<()> {
        if !Self::VALID_SAMPLE_RATES.contains(&self.sample_rate_hz) {
            return Err(AudioError::InvalidFormat(format!(
                "frecuencia {} Hz no admitida",
                self.sample_rate_hz
            )));
        }
        if self.channels == 0 || self.channels > 2 {
            return Err(AudioError::InvalidFormat(format!(
                "{} canales",
                self.channels
            )));
        }
        if !matches!(self.bits_per_sample, 16 | 24 | 32) {
            return Err(AudioError::InvalidFormat(format!(
                "{} bits por muestra",
                self.bits_per_sample
            )));
        }
        Ok(())
    }

    /// Bytes que ocupa un fotograma de audio (una muestra de todos los canales).
    #[must_use]
    pub const fn frame_bytes(&self) -> usize {
        self.channels as usize * (self.bits_per_sample as usize / 8)
    }

    /// Bytes por segundo de audio sin comprimir.
    #[must_use]
    pub const fn bytes_per_second(&self) -> usize {
        self.sample_rate_hz as usize * self.frame_bytes()
    }
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self::STEREO_48K
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} Hz, {} canal(es), {} bits",
            self.sample_rate_hz, self.channels, self.bits_per_sample
        )
    }
}

/// Estado de volumen y silencio de la sesion.
///
/// Silenciar y bajar el volumen son cosas distintas a proposito: al quitar el
/// silencio se recupera el nivel que habia, como en cualquier reproductor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeState {
    /// Nivel de 0 a 100.
    level: u8,
    /// Si esta silenciado.
    muted: bool,
}

impl VolumeState {
    /// Nivel maximo.
    pub const MAX_LEVEL: u8 = 100;

    /// Crea el estado con un nivel dado, acotandolo al rango valido.
    #[must_use]
    pub const fn new(level: u8) -> Self {
        Self {
            level: if level > Self::MAX_LEVEL {
                Self::MAX_LEVEL
            } else {
                level
            },
            muted: false,
        }
    }

    /// Nivel configurado, independientemente del silencio.
    #[must_use]
    pub const fn level(&self) -> u8 {
        self.level
    }

    /// Nivel efectivo: cero si esta silenciado.
    #[must_use]
    pub const fn effective_level(&self) -> u8 {
        if self.muted { 0 } else { self.level }
    }

    /// Si esta silenciado.
    #[must_use]
    pub const fn is_muted(&self) -> bool {
        self.muted
    }

    /// Indica si saldria algun sonido.
    #[must_use]
    pub const fn is_audible(&self) -> bool {
        self.effective_level() > 0
    }

    /// Cambia el nivel, acotandolo. No toca el silencio.
    pub const fn set_level(&mut self, level: u8) {
        self.level = if level > Self::MAX_LEVEL {
            Self::MAX_LEVEL
        } else {
            level
        };
    }

    /// Activa o desactiva el silencio sin perder el nivel.
    pub const fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Invierte el silencio y devuelve el estado nuevo.
    pub const fn toggle_mute(&mut self) -> bool {
        self.muted = !self.muted;
        self.muted
    }

    /// Ganancia lineal de 0,0 a 1,0, lista para aplicar a las muestras.
    #[must_use]
    pub fn gain(&self) -> f32 {
        f32::from(self.effective_level()) / f32::from(Self::MAX_LEVEL)
    }
}

impl Default for VolumeState {
    fn default() -> Self {
        Self::new(80)
    }
}

/// Bloque de muestras capturado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBuffer {
    /// Formato de las muestras.
    pub format: AudioFormat,
    /// Datos intercalados por canal.
    pub data: Vec<u8>,
    /// Instante de captura.
    pub captured_at: Timestamp,
}

impl AudioBuffer {
    /// Numero de fotogramas de audio que contiene.
    #[must_use]
    pub const fn frame_count(&self) -> usize {
        match self.data.len().checked_div(self.format.frame_bytes()) {
            Some(count) => count,
            None => 0,
        }
    }

    /// Duracion del bloque.
    #[must_use]
    pub fn duration(&self) -> std::time::Duration {
        if self.format.sample_rate_hz == 0 {
            return std::time::Duration::ZERO;
        }
        std::time::Duration::from_micros(
            (self.frame_count() as u64 * 1_000_000) / u64::from(self.format.sample_rate_hz),
        )
    }
}

/// Backend capaz de capturar el audio de salida del sistema.
pub trait AudioCapturer {
    /// Nombre del backend, para registros y diagnostico.
    fn name(&self) -> &'static str;

    /// Formato en el que entrega las muestras.
    ///
    /// # Errors
    ///
    /// [`AudioError::UnsupportedPlatform`] o [`AudioError::NoOutputDevice`].
    fn format(&self) -> AudioResult<AudioFormat>;

    /// Arranca la captura.
    ///
    /// # Errors
    ///
    /// Cualquier variante de [`AudioError`] segun el fallo del sistema.
    fn start(&mut self) -> AudioResult<()>;

    /// Detiene la captura.
    ///
    /// # Errors
    ///
    /// Cualquier variante de [`AudioError`] segun el fallo del sistema.
    fn stop(&mut self) -> AudioResult<()>;

    /// Lee el siguiente bloque disponible.
    ///
    /// # Errors
    ///
    /// [`AudioError::WouldBlock`] si aun no hay muestras (no es un fallo).
    fn read(&mut self, now: Timestamp) -> AudioResult<AudioBuffer>;
}

/// Backend para plataformas sin captura de audio.
///
/// Falla de forma explicita en lugar de entregar silencio, que seria
/// indistinguible de un fallo real de audio.
#[derive(Debug, Default)]
pub struct UnsupportedCapturer;

impl AudioCapturer for UnsupportedCapturer {
    fn name(&self) -> &'static str {
        "unsupported"
    }

    fn format(&self) -> AudioResult<AudioFormat> {
        Err(AudioError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }

    fn start(&mut self) -> AudioResult<()> {
        Err(AudioError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }

    fn stop(&mut self) -> AudioResult<()> {
        Err(AudioError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }

    fn read(&mut self, _now: Timestamp) -> AudioResult<AudioBuffer> {
        Err(AudioError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_format_is_valid() {
        AudioFormat::STEREO_48K
            .validate()
            .expect("48 kHz estereo 16 bits debe valer");
        assert_eq!(AudioFormat::default(), AudioFormat::STEREO_48K);
    }

    #[test]
    fn unsupported_sample_rates_are_rejected() {
        for rate in [0, 1, 22_050, 96_000, 192_000] {
            let format = AudioFormat {
                sample_rate_hz: rate,
                ..AudioFormat::STEREO_48K
            };
            assert!(format.validate().is_err(), "{rate} Hz no deberia aceptarse");
        }
    }

    #[test]
    fn channel_and_depth_limits_are_enforced() {
        for channels in [0, 3, 8] {
            let format = AudioFormat {
                channels,
                ..AudioFormat::STEREO_48K
            };
            assert!(format.validate().is_err(), "{channels} canales");
        }
        for bits in [0, 8, 12, 64] {
            let format = AudioFormat {
                bits_per_sample: bits,
                ..AudioFormat::STEREO_48K
            };
            assert!(format.validate().is_err(), "{bits} bits");
        }
    }

    #[test]
    fn frame_and_throughput_sizes_are_computed() {
        let format = AudioFormat::STEREO_48K;
        assert_eq!(format.frame_bytes(), 4);
        assert_eq!(format.bytes_per_second(), 192_000);
    }

    #[test]
    fn volume_starts_at_a_sensible_level() {
        let volume = VolumeState::default();
        assert_eq!(volume.level(), 80);
        assert!(!volume.is_muted());
        assert!(volume.is_audible());
    }

    #[test]
    fn levels_above_the_maximum_are_clamped() {
        assert_eq!(VolumeState::new(250).level(), VolumeState::MAX_LEVEL);
        let mut volume = VolumeState::new(50);
        volume.set_level(200);
        assert_eq!(volume.level(), VolumeState::MAX_LEVEL);
    }

    #[test]
    fn muting_preserves_the_level() {
        let mut volume = VolumeState::new(65);
        volume.set_muted(true);
        assert_eq!(volume.effective_level(), 0);
        assert_eq!(volume.level(), 65, "el nivel no se pierde al silenciar");
        assert!(!volume.is_audible());

        volume.set_muted(false);
        assert_eq!(volume.effective_level(), 65);
    }

    #[test]
    fn toggling_mute_reports_the_new_state() {
        let mut volume = VolumeState::new(50);
        assert!(volume.toggle_mute());
        assert!(!volume.toggle_mute());
    }

    #[test]
    fn zero_level_is_inaudible_without_muting() {
        let volume = VolumeState::new(0);
        assert!(!volume.is_audible());
        assert!(!volume.is_muted());
        assert!(volume.gain().abs() < f32::EPSILON);
    }

    #[test]
    fn gain_maps_the_level_to_a_linear_factor() {
        assert!((VolumeState::new(100).gain() - 1.0).abs() < f32::EPSILON);
        assert!((VolumeState::new(50).gain() - 0.5).abs() < f32::EPSILON);

        let mut muted = VolumeState::new(100);
        muted.set_muted(true);
        assert!(muted.gain().abs() < f32::EPSILON);
    }

    #[test]
    fn buffer_duration_follows_the_sample_count() {
        let format = AudioFormat::STEREO_48K;
        // 48.000 fotogramas de 4 bytes equivalen a un segundo.
        let buffer = AudioBuffer {
            format,
            data: vec![0; 48_000 * format.frame_bytes()],
            captured_at: Timestamp(1),
        };
        assert_eq!(buffer.frame_count(), 48_000);
        assert_eq!(buffer.duration(), std::time::Duration::from_secs(1));
    }

    #[test]
    fn an_empty_buffer_has_no_duration() {
        let buffer = AudioBuffer {
            format: AudioFormat::STEREO_48K,
            data: vec![],
            captured_at: Timestamp(1),
        };
        assert_eq!(buffer.frame_count(), 0);
        assert_eq!(buffer.duration(), std::time::Duration::ZERO);
    }

    #[test]
    fn the_unsupported_backend_fails_instead_of_delivering_silence() {
        let mut capturer = UnsupportedCapturer;
        assert!(capturer.format().is_err());
        assert!(capturer.start().is_err());
        assert!(capturer.stop().is_err());
        assert!(matches!(
            capturer.read(Timestamp(0)).unwrap_err(),
            AudioError::UnsupportedPlatform { .. }
        ));
        assert_eq!(capturer.name(), "unsupported");
    }

    #[test]
    fn volume_state_roundtrips_through_json() {
        let mut volume = VolumeState::new(42);
        volume.set_muted(true);
        let raw = serde_json::to_string(&volume).expect("serializa");
        let back: VolumeState = serde_json::from_str(&raw).expect("deserializa");
        assert_eq!(back, volume);
    }
}
