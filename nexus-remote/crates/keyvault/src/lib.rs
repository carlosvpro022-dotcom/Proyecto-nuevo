//! # nexus-keyvault
//!
//! Protege en disco la clave privada del host.
//!
//! ## Por que no bastan los permisos del archivo
//!
//! La clave privada del host es lo que demuestra a los dispositivos que estan
//! hablando con el PC correcto. Quien la copie puede hacerse pasar por el
//! equipo.
//!
//! Guardarla en un archivo con permisos restrictivos protege de **otros
//! usuarios** del sistema. No protege de **otro programa del mismo usuario**,
//! que es el caso que importa en un PC domestico de Windows: cualquier cosa
//! que la persona ejecute puede leer sus propios archivos.
//!
//! En Windows eso se resuelve con DPAPI, que ata el descifrado a la cuenta de
//! Windows: el archivo copiado a otro equipo, o leido desde otra cuenta, no se
//! puede descifrar. No es criptografia propia; es el almacen del sistema.
//!
//! ## Fuera de Windows
//!
//! [`PlainVault`] guarda la clave tal cual, que es lo que ya se hacia. La
//! diferencia es que ahora lo **dice**: [`Vault::protects_at_rest`] devuelve
//! `false` y el panel lo muestra, en lugar de que la limitacion viva solo en
//! un comentario del codigo.

// `deny` y no `forbid`: el backend de Windows necesita llamar a la API del
// sistema. El unico modulo que lo levanta es `platform::windows`.
#![deny(unsafe_code)]

pub mod error;
pub mod platform;
pub mod vault;

pub use error::{VaultError, VaultResult};
pub use platform::default_vault;
pub use vault::{PlainVault, SEALED_MAGIC, Vault, is_sealed};
