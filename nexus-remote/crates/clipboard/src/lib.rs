//! # nexus-clipboard
//!
//! Portapapeles compartido entre el equipo y el dispositivo remoto.
//!
//! Son dos piezas con responsabilidades separadas:
//!
//! - [`access`] decide **si** una operacion se puede hacer y con que limites.
//!   Es logica pura, probada en cualquier plataforma.
//! - [`platform`] traduce esa decision a llamadas del sistema. En Windows
//!   habla con la API de Win32; en el resto falla de forma explicita en lugar
//!   de fingir un portapapeles vacio.
//!
//! El limite de tamano no es un detalle: el texto viaja por el canal de
//! control, y un texto que no quepa se **rechaza** en lugar de recortarse.
//! Recortar entregaria media contrasena o medio comando con aspecto de estar
//! completo.

// `deny` y no `forbid`: el backend de Windows necesita llamar a la API del
// sistema, y `forbid` no se puede levantar ni con una justificacion escrita.
// El unico modulo que lo levanta es `platform::windows`, y alli cada bloque
// lleva su motivo.
#![deny(unsafe_code)]

pub mod access;
pub mod error;
pub mod platform;

pub use access::{Clipboard, MAX_TEXT_BYTES, MemoryClipboard, UnsupportedClipboard, check_text};
pub use error::{ClipboardError, ClipboardResult};
pub use platform::default_clipboard;
