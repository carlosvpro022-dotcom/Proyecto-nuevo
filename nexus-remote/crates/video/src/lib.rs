//! # nexus-video
//!
//! Convierte los fotogramas capturados en flujo H.264 para la pista de video.
//!
//! Dos piezas:
//!
//! - [`scale`] adapta el fotograma al tamano que pide el perfil de calidad,
//!   promediando en lugar de descartar pixeles. Lo que se transmite es texto,
//!   y descartar pixeles lo borra.
//! - [`encoder`] codifica. El codificador por software funciona en cualquier
//!   equipo; en Windows habra ademas uno por hardware, detras del mismo rasgo.
//!
//! ## Como se comprueba que esto funciona
//!
//! Codificando una imagen y **decodificandola de vuelta**, para comparar lo
//! que sale con lo que entro. H.264 pierde informacion a proposito, asi que
//! no puede salir identico: lo que se comprueba es que la imagen que vuelve se
//! parece a la que entro, y que una imagen distinta no cuela.

#![forbid(unsafe_code)]

pub mod encoder;
pub mod error;
pub mod scale;

pub use encoder::{EncodedFrame, Encoder, OpenH264Encoder};
pub use error::{VideoError, VideoResult};
pub use scale::target_size;
