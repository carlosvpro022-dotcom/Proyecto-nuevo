//! # nexus-core
//!
//! Contratos compartidos de NEXUS REMOTE: identificadores, descripcion de
//! dispositivos, permisos, capacidades, calidad de video, monitores, maquina de
//! estados de sesion y los dos protocolos de mensajes.
//!
//! Este crate no hace entrada/salida ni depende de ninguna plataforma. Eso es
//! deliberado: es el unico codigo que compilan a la vez el host Windows, el
//! servidor de senalizacion y (via FFI, en una etapa posterior) el cliente
//! Apple, asi que debe poder compilarse y probarse en cualquier sitio.
//!
//! ## Donde estan las decisiones de seguridad
//!
//! Aqui viven las **reglas**: que permiso exige cada operacion, que
//! transiciones de sesion son legitimas, que valores estan dentro de rango.
//! La **criptografia** vive en `nexus-security`. La separacion permite probar
//! las reglas de forma exhaustiva sin generar claves.

#![forbid(unsafe_code)]

pub mod capabilities;
pub mod device;
pub mod error;
pub mod ids;
pub mod monitor;
pub mod permissions;
pub mod protocol;
pub mod quality;
pub mod session;
pub mod time;

pub use capabilities::{Capability, CapabilitySet};
pub use device::{AuthorizedDevice, DeviceInfo, DevicePlatform, DeviceRole};
pub use error::{CoreError, CoreResult};
pub use ids::{DeviceId, PairingId, RequestId, SessionId, TransferId};
pub use monitor::{MonitorInfo, MonitorLayout};
pub use permissions::{Permission, PermissionSet};
pub use protocol::{Envelope, MIN_SUPPORTED_PROTOCOL, PROTOCOL_VERSION};
pub use quality::{QualityPreset, QualityProfile};
pub use session::{CloseReason, ReconnectPolicy, Session, SessionEvent, SessionState};
pub use time::Timestamp;

/// Nombre del producto, usado en registros y cabeceras.
pub const PRODUCT_NAME: &str = "NEXUS REMOTE";

/// Version del paquete, tomada de `Cargo.toml` en tiempo de compilacion.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_exposed() {
        assert!(!VERSION.is_empty());
        assert_eq!(PRODUCT_NAME, "NEXUS REMOTE");
    }

    #[test]
    fn protocol_version_is_at_least_the_minimum() {
        // Comprobacion de coherencia de las constantes: si alguien sube
        // MIN_SUPPORTED_PROTOCOL por encima de PROTOCOL_VERSION, nada podria
        // conectarse nunca.
        const _: () = assert!(PROTOCOL_VERSION >= MIN_SUPPORTED_PROTOCOL);
    }
}
