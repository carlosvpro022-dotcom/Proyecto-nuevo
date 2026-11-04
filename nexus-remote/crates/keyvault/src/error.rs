//! Errores al proteger o recuperar la clave.

use thiserror::Error;

/// Fallos del almacen de claves.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VaultError {
    /// El material guardado no tiene la forma esperada.
    #[error("el material guardado esta corrupto: {reason}")]
    Corrupt {
        /// Motivo concreto.
        reason: &'static str,
    },

    /// El sistema no pudo descifrar lo guardado.
    ///
    /// En Windows pasa cuando el archivo viene de otra cuenta o de otro
    /// equipo: DPAPI ata el descifrado a la cuenta que cifro. **No** es motivo
    /// para generar una identidad nueva, porque eso invalidaria todos los
    /// dispositivos ya emparejados.
    #[error("el sistema no pudo descifrar la clave: {0}")]
    Unsealing(String),

    /// El sistema no pudo cifrar.
    #[error("el sistema no pudo cifrar la clave: {0}")]
    Sealing(String),
}

/// Resultado corto del crate.
pub type VaultResult<T> = Result<T, VaultError>;
