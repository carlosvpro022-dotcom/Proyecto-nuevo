//! Errores de codificacion de video.

use thiserror::Error;

/// Fallos al preparar o usar el codificador.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VideoError {
    /// El fotograma no tiene un formato que se pueda codificar.
    #[error("no se puede codificar un fotograma en formato {format}")]
    UnsupportedFormat {
        /// Formato recibido.
        format: &'static str,
    },

    /// Las dimensiones no sirven.
    #[error("dimensiones no utilizables: {reason}")]
    BadDimensions {
        /// Motivo concreto.
        reason: String,
    },

    /// El fotograma recibido no es coherente con sus metadatos.
    #[error("el fotograma no es coherente: {0}")]
    MalformedFrame(String),

    /// La biblioteca de codificacion rechazo la operacion.
    #[error("el codificador fallo: {0}")]
    Codec(String),
}

/// Resultado corto del crate.
pub type VideoResult<T> = Result<T, VideoError>;
