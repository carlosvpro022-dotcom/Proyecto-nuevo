//! # nexus-file-transfer
//!
//! Restriccion de acceso al disco del host y seguimiento de transferencias.
//!
//! El modulo [`access`] es la pieza de seguridad: decide que rutas puede tocar
//! un dispositivo remoto y rechaza recorrido de directorios, enlaces que
//! escapan y nombres peligrosos de Windows. Esta completamente implementado y
//! probado.
//!
//! El modulo [`transfer`] lleva la contabilidad de cada transferencia
//! (progreso, cancelacion, limites). La lectura y escritura de bytes en si, y
//! su envio por el canal de datos de WebRTC, corresponden a la etapa 5.

#![forbid(unsafe_code)]

pub mod access;
pub mod error;
pub mod transfer;

pub use access::{Access, AccessPolicy, SharedRoot};
pub use error::{FileError, FileResult};
pub use transfer::{Direction, Transfer, TransferTracker};
