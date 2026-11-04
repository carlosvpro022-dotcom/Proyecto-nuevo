//! Backends de plataforma para el portapapeles.
//!
//! Cada plataforma tiene su modulo, activado en tiempo de compilacion. El
//! resto del crate no sabe cual esta en uso: todos implementan
//! [`Clipboard`](crate::access::Clipboard).

#[cfg(windows)]
pub mod windows;

use crate::access::Clipboard;
#[cfg(not(windows))]
use crate::access::UnsupportedClipboard;

/// Crea el backend que corresponde a esta plataforma.
///
/// En Windows devuelve el real. En cualquier otra plataforma devuelve uno que
/// falla de forma explicita, para que el host pueda arrancar durante el
/// desarrollo sin fingir que el portapapeles funciona.
#[must_use]
pub fn default_clipboard() -> Box<dyn Clipboard> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsClipboard::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(UnsupportedClipboard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_backend_por_omision_se_crea_en_cualquier_plataforma() {
        let clipboard = default_clipboard();
        if cfg!(windows) {
            assert_eq!(clipboard.name(), "windows");
        } else {
            assert_eq!(clipboard.name(), "no-disponible");
        }
    }

    #[test]
    #[cfg(not(windows))]
    fn fuera_de_windows_el_backend_falla_de_forma_explicita() {
        let mut clipboard = default_clipboard();
        assert!(matches!(
            clipboard.read_text().unwrap_err(),
            crate::error::ClipboardError::UnsupportedPlatform { .. }
        ));
    }
}
