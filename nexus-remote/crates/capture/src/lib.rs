//! # nexus-capture
//!
//! Contrato de captura de pantalla y control del ritmo de fotogramas.
//!
//! # Estado de implementacion
//!
//! **No hay backend de Windows todavia.** La captura con Windows Graphics
//! Capture corresponde a la etapa 2. Lo que esta implementado y probado aqui
//! es:
//!
//! - La descripcion de un fotograma y sus formatos de pixel ([`frame`]).
//! - El control de ritmo que decide cuando toca capturar, que es logica pura y
//!   la comparten todos los backends ([`pacing`]).
//! - El contrato [`ScreenCapturer`] y un backend de prueba que genera
//!   fotogramas sinteticos, usado para verificar el resto de la cadena sin
//!   necesidad de una pantalla real.
//!
//! El backend de prueba se llama [`SyntheticCapturer`] y esta marcado como tal
//! justamente para que nadie lo confunda con captura de verdad.

#![forbid(unsafe_code)]

pub mod capturer;
pub mod error;
pub mod frame;
pub mod pacing;

pub use capturer::{ScreenCapturer, SyntheticCapturer, UnsupportedCapturer};
pub use error::{CaptureError, CaptureResult};
pub use frame::{Frame, PixelFormat};
pub use pacing::FramePacer;
