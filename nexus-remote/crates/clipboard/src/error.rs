//! Errores del portapapeles.

use thiserror::Error;

/// Fallos al leer o escribir el portapapeles.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClipboardError {
    /// El texto supera el tamano maximo que cabe en un mensaje de control.
    #[error("el texto no cabe: {size_bytes} bytes (maximo {max_bytes})")]
    TooLarge {
        /// Tamano del texto.
        size_bytes: usize,
        /// Maximo admitido.
        max_bytes: usize,
    },

    /// El texto lleva un byte cero en medio.
    ///
    /// El portapapeles de Windows guarda cadenas terminadas en cero, asi que
    /// un cero intermedio cortaria el texto sin avisar. Se rechaza en lugar de
    /// entregar algo distinto de lo que se pidio.
    #[error("el texto contiene un byte cero y el portapapeles lo cortaria")]
    InteriorNul,

    /// El portapapeles esta vacio o no tiene texto.
    #[error("el portapapeles no tiene texto")]
    Empty,

    /// Otro programa tiene el portapapeles abierto.
    #[error("el portapapeles esta ocupado por otro programa")]
    Busy,

    /// Esta plataforma no tiene backend de portapapeles.
    #[error("el portapapeles no esta implementado en {platform}")]
    UnsupportedPlatform {
        /// Plataforma en la que se intento.
        platform: &'static str,
    },

    /// Fallo del sistema al acceder al portapapeles.
    #[error("el sistema rechazo la operacion: {0}")]
    System(String),
}

/// Resultado corto del crate.
pub type ClipboardResult<T> = Result<T, ClipboardError>;
