//! Errores del host.

use thiserror::Error;

/// Fallos del servicio del host.
#[derive(Debug, Error)]
pub enum HostError {
    /// Fallo de la base de datos local.
    #[error("error de base de datos: {0}")]
    Database(String),

    /// Fallo al leer o escribir en disco.
    #[error("error de archivo en {path}: {source}")]
    Io {
        /// Ruta implicada.
        path: String,
        /// Error original.
        source: std::io::Error,
    },

    /// Fallo de la capa de seguridad.
    #[error(transparent)]
    Security(#[from] nexus_security::SecurityError),

    /// Fallo de la capa de captura.
    #[error(transparent)]
    Capture(#[from] nexus_capture::CaptureError),

    /// El dispositivo indicado no existe.
    #[error("el dispositivo {id} no figura en la lista de autorizados")]
    UnknownDevice {
        /// Identificador buscado.
        id: String,
    },

    /// No hay ningun emparejamiento abierto.
    #[error("no hay ningun emparejamiento abierto")]
    NoPairingOpen,

    /// Un valor recibido de la interfaz no es valido.
    #[error("valor invalido: {0}")]
    InvalidInput(String),

    /// Un mensaje del protocolo no se pudo codificar o decodificar.
    #[error("fallo de protocolo: {0}")]
    Protocol(String),
}

impl From<rusqlite::Error> for HostError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}

/// Resultado corto del crate.
pub type HostResult<T> = Result<T, HostError>;

/// Error tal como se envia a la interfaz.
///
/// Tauri necesita un error serializable. Se envia un codigo estable, para que
/// la interfaz pueda decidir, y un mensaje legible para mostrar.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UiError {
    /// Codigo estable.
    pub code: String,
    /// Mensaje para la persona.
    pub message: String,
}

impl From<HostError> for UiError {
    fn from(error: HostError) -> Self {
        let code = match &error {
            HostError::Database(_) => "database",
            HostError::Io { .. } => "io",
            HostError::Security(_) => "security",
            HostError::Capture(_) => "capture",
            HostError::UnknownDevice { .. } => "unknown_device",
            HostError::NoPairingOpen => "no_pairing_open",
            HostError::InvalidInput(_) => "invalid_input",
            HostError::Protocol(_) => "protocol",
        };
        Self {
            code: code.to_owned(),
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_a_stable_code_to_the_ui() {
        let cases = [
            (HostError::Database("x".into()), "database"),
            (HostError::NoPairingOpen, "no_pairing_open"),
            (
                HostError::UnknownDevice { id: "d".into() },
                "unknown_device",
            ),
            (HostError::InvalidInput("y".into()), "invalid_input"),
            (
                HostError::Security(nexus_security::SecurityError::InvalidSignature),
                "security",
            ),
        ];
        for (error, expected) in cases {
            let ui: UiError = error.into();
            assert_eq!(ui.code, expected);
            assert!(!ui.message.is_empty());
        }
    }

    #[test]
    fn sqlite_errors_become_database_errors() {
        let error: HostError = rusqlite::Error::QueryReturnedNoRows.into();
        assert!(matches!(error, HostError::Database(_)));
    }
}
