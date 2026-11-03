//! Errores de la capa de entrada.

use thiserror::Error;

/// Fallos al traducir o inyectar entrada.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    /// El evento no paso la validacion estructural del protocolo.
    #[error("evento de entrada invalido: {0}")]
    InvalidEvent(String),

    /// El monitor indicado no existe en el host.
    #[error("el monitor {index} no existe")]
    UnknownMonitor {
        /// Indice pedido.
        index: u8,
    },

    /// La tecla no tiene equivalente en la plataforma actual.
    #[error("tecla sin equivalente en esta plataforma: {code}")]
    UnmappedKey {
        /// Nombre de la tecla.
        code: String,
    },

    /// La plataforma actual no puede inyectar entrada.
    ///
    /// Es lo que devuelve cualquier backend fuera de Windows: el envio de
    /// entrada sintetica es especifico del sistema y la implementacion real
    /// corresponde a la etapa 2.
    #[error("inyeccion de entrada no soportada en esta plataforma ({platform})")]
    UnsupportedPlatform {
        /// Plataforma detectada en tiempo de compilacion.
        platform: &'static str,
    },

    /// El sistema rechazo la llamada de inyeccion.
    #[error("el sistema rechazo la inyeccion: {0}")]
    SystemRejected(String),
}

/// Resultado corto del crate.
pub type InputResult<T> = Result<T, InputError>;
