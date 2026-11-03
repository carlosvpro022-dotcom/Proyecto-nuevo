//! Errores compartidos por todos los componentes.

use thiserror::Error;

/// Errores de validacion y de protocolo que no dependen de la plataforma.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    /// Un identificador recibido no tiene el formato esperado.
    #[error("identificador {kind} invalido: {value}")]
    InvalidId {
        /// Tipo de identificador esperado.
        kind: &'static str,
        /// Valor recibido.
        value: String,
    },

    /// La version de protocolo del par no es compatible con la nuestra.
    #[error("version de protocolo no soportada: recibida {received}, soportada {supported}")]
    UnsupportedProtocol {
        /// Version anunciada por el par.
        received: u16,
        /// Version que implementa este binario.
        supported: u16,
    },

    /// Transicion de estado no permitida por la maquina de estados de sesion.
    #[error("transicion de sesion invalida: {from} --{event}--> rechazada")]
    InvalidTransition {
        /// Estado de partida.
        from: &'static str,
        /// Evento recibido.
        event: &'static str,
    },

    /// Un valor de configuracion esta fuera del rango admitido.
    #[error("valor fuera de rango para {field}: {value} (permitido {min}..={max})")]
    OutOfRange {
        /// Campo afectado.
        field: &'static str,
        /// Valor recibido.
        value: i64,
        /// Minimo admitido.
        min: i64,
        /// Maximo admitido.
        max: i64,
    },

    /// Falta un permiso para ejecutar la operacion solicitada.
    #[error("permiso denegado: la operacion requiere {required}")]
    PermissionDenied {
        /// Permiso necesario.
        required: &'static str,
    },

    /// El mensaje no pudo decodificarse.
    #[error("mensaje mal formado: {0}")]
    Malformed(String),
}

/// Resultado corto para el crate.
pub type CoreResult<T> = Result<T, CoreError>;
