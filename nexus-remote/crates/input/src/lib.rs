//! # nexus-input
//!
//! Traduccion de eventos de entrada y contrato de inyeccion en el sistema.
//!
//! El crate se divide en dos mitades con responsabilidades muy distintas:
//!
//! - [`resolve`]: convierte un [`InputEvent`](nexus_core::protocol::input::InputEvent)
//!   del protocolo en una orden absoluta y validada. Es codigo puro, sin
//!   plataforma, y esta completamente probado.
//! - [`InputInjector`]: el contrato que implementara el backend de Windows con
//!   `SendInput`. **Todavia no hay implementacion de Windows**: es trabajo de
//!   la etapa 2. Aqui solo vive la interfaz y un backend de registro que se usa
//!   en pruebas.
//!
//! La separacion es deliberada: toda la validacion que protege al host
//! (coordenadas acotadas, teclas con nombre valido, permisos) ocurre en la
//! mitad portable, que se puede probar de forma exhaustiva en cualquier
//! maquina, y el backend de plataforma queda reducido a traducir ordenes ya
//! verificadas a llamadas del sistema.

#![forbid(unsafe_code)]

pub mod error;
pub mod inject;
pub mod resolve;

pub use error::{InputError, InputResult};
pub use inject::{InputInjector, RecordingInjector};
pub use resolve::{ResolvedInput, resolve};
