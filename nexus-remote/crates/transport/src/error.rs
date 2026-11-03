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

    /// La URL del servidor no va cifrada.
    #[error("la URL del servidor debe ser wss:// fuera de la maquina local: {0}")]
    InsecureUrl(String),

    /// Fallo del socket o del enlace con el servidor.
    #[error("fallo de red con el servidor de senalizacion: {0}")]
    Network(String),

    /// El servidor rechazo la autenticacion.
    #[error("el servidor rechazo la autenticacion: {reason:?}")]
    AuthRejected {
        /// Motivo que dio el servidor.
        reason: nexus_core::protocol::signaling::AuthRejection,
        /// Segundos que conviene esperar antes de reintentar.
        retry_after_secs: Option<u32>,
    },

    /// El servidor contesto algo que no corresponde a este punto del protocolo.
    #[error("el servidor contesto {received} cuando se esperaba {expected}")]
    UnexpectedMessage {
        /// Lo que se esperaba.
        expected: &'static str,
        /// Lo que llego.
        received: &'static str,
    },

    /// El servidor devolvio un error de aplicacion.
    #[error("el servidor devolvio el error {code}: {message}")]
    ServerError {
        /// Codigo estable.
        code: String,
        /// Texto legible.
        message: String,
    },

    /// Fallo de la pila WebRTC.
    #[error("fallo en la conexion WebRTC: {0}")]
    Peer(String),

    /// Fallo de seguridad al atar la sesion al canal cifrado.
    #[error("fallo de seguridad en la sesion: {0}")]
    Security(String),

    /// Se intento usar un canal de datos que no esta abierto.
    #[error("el canal de datos {0} no esta abierto")]
    ChannelUnavailable(String),
}

/// Resultado corto del crate.
pub type TransportResult<T> = Result<T, TransportError>;
