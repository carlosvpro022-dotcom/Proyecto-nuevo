//! Backends de plataforma para las operaciones de energia.

#[cfg(windows)]
pub mod windows;

use crate::control::PowerControl;
#[cfg(not(windows))]
use crate::control::UnsupportedPower;

/// Crea el backend que corresponde a esta plataforma.
///
/// En Windows devuelve el real. En cualquier otra plataforma devuelve uno que
/// falla de forma explicita: un apagado que contesta «hecho» sin apagar seria
/// peor que un error.
#[must_use]
pub fn default_power() -> Box<dyn PowerControl> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsPower::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(UnsupportedPower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_backend_por_omision_se_crea_en_cualquier_plataforma() {
        let power = default_power();
        if cfg!(windows) {
            assert_eq!(power.name(), "windows");
        } else {
            assert_eq!(power.name(), "no-disponible");
        }
    }

    #[test]
    #[cfg(not(windows))]
    fn fuera_de_windows_apagar_falla_en_lugar_de_mentir() {
        let mut power = default_power();
        assert!(matches!(
            power
                .execute(nexus_core::protocol::control::PowerAction::Shutdown)
                .unwrap_err(),
            crate::error::PowerError::UnsupportedPlatform { .. }
        ));
    }
}
