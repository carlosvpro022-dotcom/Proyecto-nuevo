//! Backends de plataforma para la inyeccion de entrada.
//!
//! Cada plataforma tiene su modulo, activado en tiempo de compilacion. El
//! resto del crate no sabe cual esta en uso: todos implementan
//! [`InputInjector`](crate::inject::InputInjector).
//!
//! [`default_injector`] devuelve el que corresponde a esta compilacion.

#[cfg(windows)]
pub mod windows;

use crate::inject::InputInjector;
#[cfg(not(windows))]
use crate::inject::UnsupportedInjector;

/// Crea el backend que corresponde a esta plataforma.
///
/// En Windows devuelve el backend real. En cualquier otra plataforma devuelve
/// uno que falla de forma explicita, para que el host pueda arrancar durante
/// el desarrollo sin fingir que la inyeccion funciona.
///
/// # Errors
///
/// En Windows, si no se pueden leer las metricas del escritorio virtual.
pub fn default_injector() -> crate::error::InputResult<Box<dyn InputInjector + Send>> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::WindowsInjector::new()?))
    }
    #[cfg(not(windows))]
    {
        Ok(Box::new(UnsupportedInjector))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(windows))]
    use crate::resolve::ResolvedInput;

    #[test]
    fn el_backend_por_omision_se_crea_en_cualquier_plataforma() {
        let injector = default_injector().expect("deberia construirse");
        if cfg!(windows) {
            assert_eq!(injector.name(), "windows-sendinput");
        } else {
            assert_eq!(injector.name(), "unsupported");
        }
    }

    #[test]
    #[cfg(not(windows))]
    fn fuera_de_windows_el_backend_falla_de_forma_explicita() {
        let mut injector = default_injector().expect("se construye");
        let err = injector
            .inject(&ResolvedInput::MoveCursor { x: 10, y: 10 })
            .unwrap_err();
        assert!(
            matches!(err, crate::error::InputError::UnsupportedPlatform { .. }),
            "{err:?}"
        );
    }
}
