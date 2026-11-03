//! Errores de captura de pantalla.

use thiserror::Error;

/// Fallos al capturar la pantalla.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CaptureError {
    /// La plataforma actual no tiene backend de captura.
    ///
    /// Es lo que devuelve el host al compilarse para Linux durante el
    /// desarrollo: la captura real es especifica de Windows.
    #[error("captura de pantalla no soportada en esta plataforma ({platform})")]
    UnsupportedPlatform {
        /// Plataforma detectada en tiempo de compilacion.
        platform: &'static str,
    },

    /// El monitor pedido no existe.
    #[error("el monitor {index} no existe")]
    UnknownMonitor {
        /// Indice pedido.
        index: u8,
    },

    /// El sistema denego el permiso de captura.
    #[error("el sistema denego la captura: {0}")]
    PermissionDenied(String),

    /// La sesion de captura se perdio (cambio de resolucion, bloqueo, etc.).
    #[error("la sesion de captura se perdio: {0}")]
    SessionLost(String),

    /// Todavia no hay un fotograma nuevo disponible.
    ///
    /// No es un fallo: indica al llamante que vuelva mas tarde.
    #[error("sin fotograma nuevo disponible")]
    WouldBlock,

    /// Error del sistema al capturar.
    #[error("error del sistema de captura: {0}")]
    System(String),
}

/// Resultado corto del crate.
pub type CaptureResult<T> = Result<T, CaptureError>;
