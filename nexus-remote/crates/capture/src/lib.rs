//! # nexus-capture
//!
//! Contrato de captura de pantalla y control del ritmo de fotogramas.
//!
//! # Estado de implementacion
//!
//! ## Portable, ejecutado y probado aqui
//!
//! - [`frame`]: descripcion de un fotograma, formatos de pixel y validacion de
//!   coherencia entre metadatos y datos.
//! - [`pacing`]: el control de ritmo que decide cuando toca capturar.
//! - [`enumeration`]: conversion de los datos crudos del sistema a
//!   [`MonitorInfo`](nexus_core::MonitorInfo), con los casos raros (densidades
//!   absurdas, rectangulos invertidos, nombres vacios).
//! - [`SyntheticCapturer`]: fotogramas de color plano para verificar el resto
//!   de la cadena sin pantalla real. **No es captura**, y su nombre lo dice.
//!
//! ## De plataforma, compilado para Windows pero no ejecutado alli
//!
//! - [`platform::windows_monitors`]: enumeracion real de pantallas.
//! - [`platform::windows_capture`]: captura con Windows Graphics Capture y
//!   Direct3D 11.
//!
//! Ambos son verificacion de nivel 2 del ADR-0007: el compilador comprobo
//! tipos y firmas en cruce, **nadie ha comprobado el comportamiento en un PC
//! con Windows**. No deben darse por buenos hasta ejecutarlos alli.

// El codigo portable de este crate no usa `unsafe`. Los backends de Windows si
// lo necesitan para llamar a Direct3D y a WinRT, y lo permiten de forma
// acotada en sus propios modulos, con una justificacion escrita en cada
// llamada. `forbid` no se puede levantar por modulo, de ahi `deny`.
#![deny(unsafe_code)]

pub mod capturer;
pub mod enumeration;
pub mod error;
pub mod frame;
pub mod pacing;
pub mod platform;

pub use capturer::{ScreenCapturer, SyntheticCapturer, UnsupportedCapturer};
pub use enumeration::{RawMonitor, to_layout};
pub use error::{CaptureError, CaptureResult};
pub use frame::{Frame, PixelFormat};
pub use pacing::FramePacer;
pub use platform::{default_capturer, enumerate_monitors};
