//! Control del ritmo de captura.
//!
//! Capturar mas rapido de lo que la red puede enviar solo gasta CPU y bateria.
//! El marcapasos decide cuando toca el siguiente fotograma a partir del objetivo
//! de fotogramas por segundo, y permite bajar el ritmo cuando el transporte va
//! justo.
//!
//! Es logica pura y la comparten todos los backends de captura, asi que se
//! prueba de forma exhaustiva aqui en lugar de repetirla por plataforma.

use std::time::Duration;

use nexus_core::{QualityProfile, Timestamp};

/// Decide cuando capturar el siguiente fotograma.
#[derive(Debug, Clone)]
pub struct FramePacer {
    target_fps: u16,
    last_capture: Option<Timestamp>,
    dropped: u64,
    captured: u64,
}

impl FramePacer {
    /// Crea el marcapasos para un objetivo de fotogramas por segundo.
    ///
    /// El objetivo se acota al rango que admite el protocolo, de modo que un
    /// valor absurdo no se convierte en una espera de cero.
    #[must_use]
    pub fn new(target_fps: u16) -> Self {
        Self {
            target_fps: target_fps.clamp(QualityProfile::MIN_FPS, QualityProfile::MAX_FPS),
            last_capture: None,
            dropped: 0,
            captured: 0,
        }
    }

    /// Crea el marcapasos a partir de un perfil de calidad.
    #[must_use]
    pub fn from_profile(profile: &QualityProfile) -> Self {
        Self::new(profile.clamped().target_fps)
    }

    /// Objetivo actual de fotogramas por segundo.
    #[must_use]
    pub const fn target_fps(&self) -> u16 {
        self.target_fps
    }

    /// Intervalo entre fotogramas para el objetivo actual.
    #[must_use]
    pub const fn frame_interval(&self) -> Duration {
        Duration::from_nanos(1_000_000_000 / self.target_fps as u64)
    }

    /// Cambia el objetivo en caliente, acotandolo al rango admitido.
    pub fn set_target_fps(&mut self, target_fps: u16) {
        self.target_fps = target_fps.clamp(QualityProfile::MIN_FPS, QualityProfile::MAX_FPS);
    }

    /// Indica si toca capturar en este instante.
    #[must_use]
    pub fn should_capture(&self, now: Timestamp) -> bool {
        match self.last_capture {
            None => true,
            Some(last) => now.abs_diff(last) >= self.frame_interval(),
        }
    }

    /// Anota que se capturo un fotograma.
    pub fn record_capture(&mut self, now: Timestamp) {
        self.last_capture = Some(now);
        self.captured += 1;
    }

    /// Anota que se descarto un fotograma por ir con retraso.
    pub fn record_drop(&mut self) {
        self.dropped += 1;
    }

    /// Espera recomendada hasta el siguiente fotograma.
    #[must_use]
    pub fn time_until_next(&self, now: Timestamp) -> Duration {
        let Some(last) = self.last_capture else {
            return Duration::ZERO;
        };
        self.frame_interval().saturating_sub(now.abs_diff(last))
    }

    /// Fotogramas capturados.
    #[must_use]
    pub const fn captured(&self) -> u64 {
        self.captured
    }

    /// Fotogramas descartados.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Proporcion de fotogramas descartados, de 0,0 a 1,0.
    ///
    /// Es la senal que usa el host para decidir si conviene bajar la calidad.
    #[must_use]
    pub fn drop_ratio(&self) -> f32 {
        let total = self.captured + self.dropped;
        if total == 0 {
            return 0.0;
        }
        self.dropped as f32 / total as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_frame_is_captured_immediately() {
        let pacer = FramePacer::new(30);
        assert!(pacer.should_capture(Timestamp(0)));
        assert_eq!(pacer.time_until_next(Timestamp(0)), Duration::ZERO);
    }

    #[test]
    fn the_interval_matches_the_target() {
        assert_eq!(
            FramePacer::new(30).frame_interval(),
            Duration::from_nanos(33_333_333)
        );
        assert_eq!(
            FramePacer::new(60).frame_interval(),
            Duration::from_nanos(16_666_666)
        );
        assert_eq!(FramePacer::new(1).frame_interval(), Duration::from_secs(1));
    }

    #[test]
    fn capturing_too_soon_is_refused() {
        let mut pacer = FramePacer::new(30);
        pacer.record_capture(Timestamp(1_000));
        assert!(
            !pacer.should_capture(Timestamp(1_010)),
            "solo han pasado 10 ms de los 33"
        );
        assert!(pacer.should_capture(Timestamp(1_034)));
    }

    #[test]
    fn remaining_time_counts_down() {
        let mut pacer = FramePacer::new(30);
        pacer.record_capture(Timestamp(1_000));
        assert!(pacer.time_until_next(Timestamp(1_010)) > Duration::ZERO);
        assert_eq!(pacer.time_until_next(Timestamp(2_000)), Duration::ZERO);
    }

    #[test]
    fn absurd_targets_are_clamped() {
        assert_eq!(FramePacer::new(0).target_fps(), QualityProfile::MIN_FPS);
        assert_eq!(
            FramePacer::new(u16::MAX).target_fps(),
            QualityProfile::MAX_FPS
        );
    }

    #[test]
    fn the_target_can_change_while_running() {
        let mut pacer = FramePacer::new(60);
        pacer.set_target_fps(15);
        assert_eq!(pacer.target_fps(), 15);
        pacer.set_target_fps(10_000);
        assert_eq!(pacer.target_fps(), QualityProfile::MAX_FPS);
    }

    #[test]
    fn it_is_built_from_a_quality_profile() {
        let profile = QualityProfile {
            target_fps: 24,
            ..QualityProfile::default()
        };
        assert_eq!(FramePacer::from_profile(&profile).target_fps(), 24);

        // Un perfil manipulado se acota, no se acepta tal cual.
        let bad = QualityProfile {
            target_fps: 9_999,
            ..QualityProfile::default()
        };
        assert_eq!(
            FramePacer::from_profile(&bad).target_fps(),
            QualityProfile::MAX_FPS
        );
    }

    #[test]
    fn counters_and_drop_ratio_track_reality() {
        let mut pacer = FramePacer::new(30);
        assert!(
            pacer.drop_ratio().abs() < f32::EPSILON,
            "sin datos no hay descarte"
        );

        for i in 0..9 {
            pacer.record_capture(Timestamp(i * 100));
        }
        pacer.record_drop();

        assert_eq!(pacer.captured(), 9);
        assert_eq!(pacer.dropped(), 1);
        assert!(
            (pacer.drop_ratio() - 0.1).abs() < 0.001,
            "ratio: {}",
            pacer.drop_ratio()
        );
    }
}
