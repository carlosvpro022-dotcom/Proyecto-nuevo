//! Errores de transferencia de archivos.

use thiserror::Error;

/// Fallos de acceso y de transferencia.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FileError {
    /// La ruta queda fuera de las raices permitidas.
    #[error("acceso denegado: la ruta queda fuera de las carpetas compartidas")]
    OutsideRoots,

    /// La ruta contiene componentes de recorrido (`..`).
    #[error("acceso denegado: la ruta contiene recorrido de directorios")]
    PathTraversal,

    /// El nombre esta reservado por el sistema o tiene una forma peligrosa.
    #[error("nombre de archivo no permitido: {reason}")]
    ForbiddenName {
        /// Motivo concreto del rechazo.
        reason: &'static str,
    },

    /// La raiz concede acceso, pero no del tipo solicitado.
    #[error("acceso denegado: la carpeta compartida es de solo lectura")]
    ReadOnlyRoot,

    /// La ruta no existe.
    #[error("la ruta no existe")]
    NotFound,

    /// El archivo supera el tamano maximo admitido.
    #[error("archivo demasiado grande: {size_bytes} bytes (maximo {max_bytes})")]
    TooLarge {
        /// Tamano anunciado.
        size_bytes: u64,
        /// Maximo admitido.
        max_bytes: u64,
    },

    /// Se alcanzo el limite de transferencias simultaneas.
    #[error("demasiadas transferencias simultaneas (maximo {max})")]
    TooManyTransfers {
        /// Maximo admitido.
        max: usize,
    },

    /// La transferencia indicada no existe.
    #[error("la transferencia indicada no existe")]
    UnknownTransfer,

    /// La transferencia ya habia terminado.
    #[error("la transferencia ya habia terminado")]
    TransferFinished,

    /// Error de entrada/salida del sistema de archivos.
    #[error("error de entrada/salida: {0}")]
    Io(String),
}

/// Resultado corto del crate.
pub type FileResult<T> = Result<T, FileError>;
