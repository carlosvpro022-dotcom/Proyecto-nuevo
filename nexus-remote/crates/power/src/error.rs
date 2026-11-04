//! Errores de las operaciones de energia.

use thiserror::Error;

/// Fallos al bloquear, reiniciar o apagar.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PowerError {
    /// Esta plataforma no tiene backend de energia.
    #[error("las operaciones de energia no estan implementadas en {platform}")]
    UnsupportedPlatform {
        /// Plataforma en la que se intento.
        platform: &'static str,
    },

    /// El sistema no concede el privilegio necesario para apagar.
    ///
    /// Windows exige `SeShutdownPrivilege` para reiniciar o apagar. Una
    /// cuenta sin ese privilegio no puede hacerlo, y decirlo asi es mas util
    /// que un fallo generico del sistema.
    #[error("esta cuenta de Windows no tiene permiso para apagar el equipo")]
    PrivilegeDenied,

    /// Fallo del sistema al ejecutar la operacion.
    #[error("el sistema rechazo la operacion: {0}")]
    System(String),
}

/// Resultado corto del crate.
pub type PowerResult<T> = Result<T, PowerError>;
