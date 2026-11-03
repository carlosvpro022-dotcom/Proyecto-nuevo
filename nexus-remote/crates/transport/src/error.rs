//! Errores de transporte.

use thiserror::Error;

/// Fallos de codificacion, enlace y configuracion de red.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransportError {
    /// El mensaje supera el tamano admitido.
    #[error("mensaje demasiado grande: {size} bytes (maximo {max})")]
    MessageTooLarge {
        /// Tamano recibido.
        size: usize,
        /// Maximo admitido.
        max: usize,
    },

    /// El mensaje no pudo decodificarse.
    #[error("mensaje mal formado: {0}")]
    Malformed(String),

    /// La configuracion ICE no sirve para conectar.
    #[error("configuracion ICE invalida: {0}")]
    InvalidIceConfiguration(String),

    /// La operacion no es valida en el estado actual del enlace.
    #[error("operacion invalida con el enlace en estado {state}")]
    InvalidLinkState {
        /// Estado actual.
        state: &'static str,
    },

    /// Se agotaron los reintentos de reconexion.
    #[error("se agotaron los reintentos de reconexion ({attempts})")]
    ReconnectExhausted {
        /// Intentos consumidos.
        attempts: u32,
    },
}

/// Resultado corto del crate.
pub type TransportResult<T> = Result<T, TransportError>;
