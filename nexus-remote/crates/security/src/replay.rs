//! Proteccion contra reenvio de mensajes.
//!
//! Dos comprobaciones complementarias, porque ninguna basta por si sola:
//!
//! 1. **Ventana temporal.** Un mensaje cuya marca se aleja demasiado del reloj
//!    local se rechaza. Esto acota cuanto tiempo puede guardarse un mensaje
//!    capturado antes de reenviarlo, y tambien cuanto hay que recordar.
//! 2. **Nonces vistos.** Dentro de esa ventana se recuerda cada nonce aceptado
//!    y se rechaza el segundo uso. Sin esto, un atacante podria reenviar el
//!    mismo mensaje tantas veces como quisiera mientras la marca siguiera
//!    siendo reciente.
//!
//! La memoria queda acotada por la propia ventana: lo que cae fuera se olvida.

use std::collections::HashMap;
use std::time::Duration;

use nexus_core::Timestamp;

use crate::challenge::Nonce;
use crate::error::{SecurityError, SecurityResult};

/// Tolerancia de reloj por omision entre dispositivos.
///
/// Treinta segundos cubren el desvio normal de un movil sincronizado por NTP
/// sin dar margen util a un reenvio.
pub const DEFAULT_CLOCK_TOLERANCE: Duration = Duration::from_secs(30);

/// Guardia que recuerda los nonces aceptados dentro de la ventana vigente.
#[derive(Debug)]
pub struct ReplayGuard {
    seen: HashMap<Nonce, Timestamp>,
    tolerance: Duration,
    max_entries: usize,
}

impl ReplayGuard {
    /// Numero maximo de nonces recordados por omision.
    ///
    /// Es un tope de seguridad frente a un pico de trafico: con la ventana por
    /// omision haria falta un volumen enorme para alcanzarlo.
    pub const DEFAULT_MAX_ENTRIES: usize = 16_384;

    /// Crea una guardia con la tolerancia indicada.
    #[must_use]
    pub fn new(tolerance: Duration) -> Self {
        Self {
            seen: HashMap::new(),
            tolerance,
            max_entries: Self::DEFAULT_MAX_ENTRIES,
        }
    }

    /// Crea una guardia indicando tambien el tope de nonces recordados.
    #[must_use]
    pub fn with_capacity(tolerance: Duration, max_entries: usize) -> Self {
        Self {
            seen: HashMap::new(),
            tolerance,
            max_entries,
        }
    }

    /// Tolerancia de reloj configurada.
    #[must_use]
    pub const fn tolerance(&self) -> Duration {
        self.tolerance
    }

    /// Comprueba y registra un mensaje.
    ///
    /// Si acepta, el nonce queda anotado y un segundo uso sera rechazado. Si
    /// rechaza, **no** se anota nada: un mensaje invalido no debe poder llenar
    /// la memoria de la guardia.
    ///
    /// # Errors
    ///
    /// - [`SecurityError::ClockSkew`] si la marca esta fuera de la ventana.
    /// - [`SecurityError::ReplayDetected`] si el nonce ya se habia aceptado.
    /// - [`SecurityError::RateLimited`] si se alcanzo el tope de nonces
    ///   recordados, lo que indica un volumen anomalo.
    pub fn check_and_record(
        &mut self,
        nonce: Nonce,
        message_time: Timestamp,
        now: Timestamp,
    ) -> SecurityResult<()> {
        self.check_clock(message_time, now)?;
        self.prune(now);

        if self.seen.contains_key(&nonce) {
            return Err(SecurityError::ReplayDetected);
        }

        if self.seen.len() >= self.max_entries {
            return Err(SecurityError::RateLimited {
                retry_after_secs: self.tolerance.as_secs().max(1),
            });
        }

        self.seen.insert(nonce, message_time);
        Ok(())
    }

    /// Comprueba solo la ventana temporal, sin registrar nada.
    ///
    /// Existe aparte para poder descartar un mensaje viejo antes de gastar
    /// trabajo en verificar su firma, y sin que un mensaje invalido deje rastro
    /// en la tabla de nonces.
    ///
    /// # Errors
    ///
    /// [`SecurityError::ClockSkew`] si la marca esta fuera de la ventana.
    pub fn check_clock(&self, message_time: Timestamp, now: Timestamp) -> SecurityResult<()> {
        let skew = message_time.abs_diff(now);
        if skew > self.tolerance {
            return Err(SecurityError::ClockSkew {
                skew_ms: skew.as_millis() as u64,
            });
        }
        Ok(())
    }

    /// Olvida los nonces que ya cayeron fuera de la ventana.
    ///
    /// Un nonce se puede olvidar en cuanto su marca esta tan lejos que
    /// `check_and_record` lo rechazaria por desvio de reloj: a partir de ese
    /// momento recordarlo no aporta nada.
    pub fn prune(&mut self, now: Timestamp) {
        let tolerance = self.tolerance;
        self.seen.retain(|_, ts| ts.abs_diff(now) <= tolerance);
    }

    /// Numero de nonces recordados.
    #[must_use]
    pub fn tracked_count(&self) -> usize {
        self.seen.len()
    }
}

impl Default for ReplayGuard {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_TOLERANCE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp(1_700_000_000_000);

    fn nonce(seed: u8) -> Nonce {
        Nonce::from_bytes([seed; 32])
    }

    #[test]
    fn a_fresh_message_is_accepted() {
        let mut guard = ReplayGuard::default();
        guard
            .check_and_record(nonce(1), NOW, NOW)
            .expect("mensaje fresco");
        assert_eq!(guard.tracked_count(), 1);
    }

    #[test]
    fn the_same_nonce_twice_is_rejected() {
        let mut guard = ReplayGuard::default();
        guard
            .check_and_record(nonce(1), NOW, NOW)
            .expect("primera vez");
        assert_eq!(
            guard.check_and_record(nonce(1), NOW, NOW).unwrap_err(),
            SecurityError::ReplayDetected
        );
    }

    #[test]
    fn replay_with_a_fresher_timestamp_is_still_rejected() {
        // El atacante reenvia el mensaje capturado actualizando la marca: el
        // nonce sigue siendo el mismo y la firma solo vale para su marca
        // original, pero la guardia no depende de eso para cortarlo.
        let mut guard = ReplayGuard::default();
        guard
            .check_and_record(nonce(1), NOW, NOW)
            .expect("primera vez");
        let later = Timestamp(NOW.0 + 1_000);
        assert_eq!(
            guard.check_and_record(nonce(1), later, later).unwrap_err(),
            SecurityError::ReplayDetected
        );
    }

    #[test]
    fn different_nonces_are_independent() {
        let mut guard = ReplayGuard::default();
        for seed in 0..32 {
            guard
                .check_and_record(nonce(seed), NOW, NOW)
                .expect("nonce distinto");
        }
        assert_eq!(guard.tracked_count(), 32);
    }

    #[test]
    fn a_message_from_the_past_outside_the_window_is_rejected() {
        let mut guard = ReplayGuard::new(Duration::from_secs(30));
        let too_old = Timestamp(NOW.0 - 31_000);
        let err = guard.check_and_record(nonce(1), too_old, NOW).unwrap_err();
        assert_eq!(err, SecurityError::ClockSkew { skew_ms: 31_000 });
    }

    #[test]
    fn a_message_from_the_future_outside_the_window_is_rejected() {
        let mut guard = ReplayGuard::new(Duration::from_secs(30));
        let too_new = Timestamp(NOW.0 + 31_000);
        let err = guard.check_and_record(nonce(1), too_new, NOW).unwrap_err();
        assert_eq!(err, SecurityError::ClockSkew { skew_ms: 31_000 });
    }

    #[test]
    fn the_window_boundary_is_inclusive() {
        let mut guard = ReplayGuard::new(Duration::from_secs(30));
        guard
            .check_and_record(nonce(1), Timestamp(NOW.0 - 30_000), NOW)
            .expect("el limite exacto se acepta");
        guard
            .check_and_record(nonce(2), Timestamp(NOW.0 + 30_000), NOW)
            .expect("y por arriba");
    }

    #[test]
    fn a_rejected_message_is_not_recorded() {
        let mut guard = ReplayGuard::new(Duration::from_secs(30));
        let too_old = Timestamp(NOW.0 - 60_000);
        assert!(guard.check_and_record(nonce(1), too_old, NOW).is_err());
        assert_eq!(
            guard.tracked_count(),
            0,
            "un rechazo no debe consumir memoria"
        );
    }

    #[test]
    fn nonces_are_forgotten_once_outside_the_window() {
        let mut guard = ReplayGuard::new(Duration::from_secs(30));
        guard.check_and_record(nonce(1), NOW, NOW).expect("fresco");
        assert_eq!(guard.tracked_count(), 1);

        // Mucho despues, el nonce antiguo ya no hace falta.
        let much_later = Timestamp(NOW.0 + 120_000);
        guard.prune(much_later);
        assert_eq!(guard.tracked_count(), 0);
    }

    #[test]
    fn forgetting_is_safe_because_the_window_already_rejects_the_message() {
        // Tras olvidar el nonce, reenviar el mensaje original no cuela: su
        // marca temporal ya esta fuera de la ventana.
        let mut guard = ReplayGuard::new(Duration::from_secs(30));
        guard.check_and_record(nonce(1), NOW, NOW).expect("fresco");

        let much_later = Timestamp(NOW.0 + 120_000);
        guard.prune(much_later);

        let err = guard
            .check_and_record(nonce(1), NOW, much_later)
            .unwrap_err();
        assert!(matches!(err, SecurityError::ClockSkew { .. }));
    }

    #[test]
    fn tracked_nonces_are_capped() {
        let mut guard = ReplayGuard::with_capacity(Duration::from_secs(300), 4);
        for seed in 0..4 {
            guard.check_and_record(nonce(seed), NOW, NOW).expect("cabe");
        }
        let err = guard.check_and_record(nonce(200), NOW, NOW).unwrap_err();
        assert!(matches!(err, SecurityError::RateLimited { .. }));
    }

    #[test]
    fn check_clock_does_not_record_anything() {
        let mut guard = ReplayGuard::new(Duration::from_secs(30));
        guard.check_clock(NOW, NOW).expect("marca fresca");
        assert_eq!(guard.tracked_count(), 0);
        // Y por tanto el mensaje sigue pudiendose aceptar luego.
        guard
            .check_and_record(nonce(1), NOW, NOW)
            .expect("se acepta");
    }

    #[test]
    fn clock_skew_error_reports_the_measured_deviation() {
        let mut guard = ReplayGuard::new(Duration::from_secs(5));
        let err = guard
            .check_and_record(nonce(1), Timestamp(NOW.0 - 12_345), NOW)
            .unwrap_err();
        assert_eq!(err, SecurityError::ClockSkew { skew_ms: 12_345 });
    }
}
