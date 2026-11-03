//! # nexus-transport
//!
//! Capa de transporte: codificacion de los mensajes de senalizacion, estado del
//! enlace con el servidor y configuracion ICE.
//!
//! # Estado de implementacion
//!
//! **Todavia no se abre ninguna conexion WebRTC.** Eso es la etapa 3. Lo que
//! esta implementado y probado aqui es todo lo que rodea a esa conexion y que
//! conviene tener resuelto antes:
//!
//! - [`codec`]: serializacion de los mensajes con limite de tamano, para que un
//!   mensaje enorme no pueda agotar la memoria del host.
//! - [`link`]: la maquina de estados del enlace de senalizacion con su politica
//!   de reconexion.
//! - [`ice`]: la configuracion de servidores STUN y TURN, con validacion y la
//!   politica de forzar relay.

#![forbid(unsafe_code)]

#[cfg(feature = "peer")]
pub mod client;
pub mod codec;
pub mod error;
pub mod ice;
pub mod link;
#[cfg(feature = "peer")]
pub mod peer;

#[cfg(feature = "peer")]
pub use client::{Accepted, ConnectRequest, SignalingClient};
pub use codec::{MAX_MESSAGE_BYTES, decode_server_message, encode_client_message};
pub use error::{TransportError, TransportResult};
pub use ice::{IceConfiguration, TransportPolicy};
pub use link::{LinkEvent, LinkState, SignalingLink};
#[cfg(feature = "peer")]
pub use peer::{LinkPhase, PeerEvent, PeerRole, PeerSession};
