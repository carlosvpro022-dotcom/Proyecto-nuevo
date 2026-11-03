//! Errores de la capa de seguridad.
//!
//! Los mensajes son deliberadamente poco informativos de cara al exterior: lo
//! que el host registra internamente es detallado, pero lo que viaja al cliente
//! se reduce a una de las variantes de rechazo del protocolo de senalizacion,
//! para no confirmar a un atacante si un identificador existe.

use thiserror::Error;

/// Fallos de identidad, emparejamiento y verificacion.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SecurityError {
    /// La clave no tiene la longitud o la codificacion esperadas.
    #[error("material de clave invalido: {0}")]
    InvalidKeyMaterial(String),

    /// La firma no corresponde al mensaje y a la clave indicados.
    #[error("firma invalida")]
    InvalidSignature,

    /// El dispositivo no figura en la lista de autorizados.
    #[error("dispositivo no autorizado")]
    UnknownDevice,

    /// El dispositivo figura pero esta revocado.
    #[error("dispositivo revocado")]
    DeviceRevoked,

    /// El reto no existe, ya se consumio o expiro.
    #[error("reto de autenticacion no valido o caducado")]
    ChallengeNotValid,

    /// La marca temporal del mensaje esta fuera de la ventana admitida.
    #[error("marca temporal fuera de la ventana admitida ({skew_ms} ms de desvio)")]
    ClockSkew {
        /// Desvio medido en milisegundos.
        skew_ms: u64,
    },

    /// El nonce ya se habia visto: posible reenvio de un mensaje capturado.
    #[error("mensaje repetido: el nonce ya se habia usado")]
    ReplayDetected,

    /// Se agotaron los intentos permitidos.
    #[error("demasiados intentos fallidos; reintentar en {retry_after_secs} s")]
    RateLimited {
        /// Segundos que faltan para poder reintentar.
        retry_after_secs: u64,
    },

    /// El codigo de emparejamiento no coincide.
    #[error("codigo de emparejamiento incorrecto")]
    PairingCodeMismatch,

    /// El codigo de emparejamiento caduco.
    #[error("codigo de emparejamiento caducado")]
    PairingCodeExpired,

    /// El emparejamiento ya se habia completado o cancelado.
    #[error("el emparejamiento ya no esta activo")]
    PairingNotActive,

    /// Fallo al derivar o verificar un hash de contrasena.
    #[error("fallo en la funcion de derivacion: {0}")]
    HashFailure(String),

    /// La huella del certificado DTLS no tiene un formato admitido.
    #[error("huella DTLS invalida: {0}")]
    InvalidFingerprint(String),

    /// La firma del otro extremo cubre una huella DTLS distinta de la que se
    /// negocio de verdad.
    ///
    /// Es la senal de que alguien intercepto la senalizacion y sustituyo la
    /// huella para colocarse en medio de la sesion. La sesion se aborta.
    #[error("la huella DTLS firmada no coincide con la negociada: posible interceptacion")]
    FingerprintMismatch,
}

/// Resultado corto del crate.
pub type SecurityResult<T> = Result<T, SecurityError>;
