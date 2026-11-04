//! # nexus-file-transfer
//!
//! Transferencia de archivos entre el host y el dispositivo, por el canal de
//! datos cifrado.
//!
//! Son cuatro piezas, cada una con una responsabilidad que no comparte:
//!
//! - [`access`] decide **que rutas** puede tocar un dispositivo remoto. Es la
//!   pieza de seguridad: rechaza recorrido de directorios, enlaces que escapan
//!   y nombres peligrosos de Windows.
//! - [`listing`] lee un directorio ya autorizado y lo pagina para que quepa en
//!   el canal de control.
//! - [`frame`] define el formato binario con el que viajan los bytes.
//! - [`io`] mueve los bytes de verdad: los lee del disco, los escribe en un
//!   archivo temporal y solo publica el destino cuando el contenido esta
//!   entero y verificado.
//! - [`transfer`] lleva la contabilidad: progreso, cancelacion y limites.
//!
//! ## El reparto de responsabilidades que importa
//!
//! Ninguna funcion de este crate comprueba permisos, y ninguna abre una ruta
//! que no venga ya resuelta por [`AccessPolicy::resolve`]. El permiso se
//! comprueba en un solo sitio, en el host, antes de llegar aqui. Repartir esa
//! decision entre varias capas es como se acaba perdiendo.

#![forbid(unsafe_code)]

pub mod access;
pub mod error;
pub mod frame;
pub mod io;
pub mod listing;
pub mod transfer;

pub use access::{Access, AccessPolicy, SharedRoot};
pub use error::{FileError, FileResult};
pub use frame::{Frame, MAX_FRAME_BYTES};
pub use io::{Accepted, ChunkReader, ChunkWriter};
pub use listing::{Listing, list_dir};
pub use transfer::{CHUNK_SIZE_BYTES, Direction, Transfer, TransferTracker};
