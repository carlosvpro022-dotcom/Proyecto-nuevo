//! Marca temporal del protocolo.
//!
//! Se usa un entero de milisegundos desde la epoca UNIX en lugar de un tipo de
//! fecha enriquecido: es estable en el cable, trivial de comparar y no arrastra
//! dependencias de zonas horarias al nucleo del protocolo. La ventana de
//! tolerancia de reloj vive en `nexus-security`, que es quien decide si un
//! mensaje llega demasiado tarde.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Milisegundos transcurridos desde la epoca UNIX.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

impl Timestamp {
    /// Instante cero. Util en pruebas y como valor centinela.
    pub const EPOCH: Self = Self(0);

    /// Lee el reloj del sistema.
    ///
    /// Si el reloj estuviera antes de la epoca UNIX (algo que solo ocurre con
    /// una configuracion corrupta) devuelve `EPOCH` en lugar de entrar en
    /// panico: un fallo de reloj no debe tumbar el host remoto.
    #[must_use]
    pub fn now() -> Self {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(Self::EPOCH, |d| {
                Self(d.as_millis().min(u128::from(u64::MAX)) as u64)
            })
    }

    /// Diferencia absoluta entre dos marcas.
    #[must_use]
    pub const fn abs_diff(self, other: Self) -> Duration {
        Duration::from_millis(self.0.abs_diff(other.0))
    }

    /// Avanza la marca una duracion dada.
    #[must_use]
    pub const fn saturating_add(self, delta: Duration) -> Self {
        Self(self.0.saturating_add(delta.as_millis() as u64))
    }

    /// Retrocede la marca una duracion dada.
    #[must_use]
    pub const fn saturating_sub(self, delta: Duration) -> Self {
        Self(self.0.saturating_sub(delta.as_millis() as u64))
    }

    /// Indica si `self` esta dentro de `tolerance` respecto a `reference`.
    #[must_use]
    pub const fn within(self, reference: Self, tolerance: Duration) -> bool {
        self.abs_diff(reference).as_millis() <= tolerance.as_millis()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_after_2024() {
        // 2024-01-01T00:00:00Z en milisegundos.
        assert!(Timestamp::now().0 > 1_704_067_200_000);
    }

    #[test]
    fn abs_diff_is_symmetric() {
        let a = Timestamp(1_000);
        let b = Timestamp(4_500);
        assert_eq!(a.abs_diff(b), Duration::from_millis(3_500));
        assert_eq!(b.abs_diff(a), Duration::from_millis(3_500));
    }

    #[test]
    fn within_tolerance_includes_boundary() {
        let reference = Timestamp(10_000);
        assert!(Timestamp(9_000).within(reference, Duration::from_secs(1)));
        assert!(Timestamp(11_000).within(reference, Duration::from_secs(1)));
        assert!(!Timestamp(11_001).within(reference, Duration::from_secs(1)));
    }

    #[test]
    fn saturating_arithmetic_does_not_wrap() {
        assert_eq!(
            Timestamp::EPOCH.saturating_sub(Duration::from_secs(5)),
            Timestamp::EPOCH
        );
        assert_eq!(
            Timestamp(u64::MAX).saturating_add(Duration::from_secs(5)),
            Timestamp(u64::MAX)
        );
    }
}
