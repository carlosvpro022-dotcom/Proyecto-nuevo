//! # nexus-power
//!
//! Bloquear, reiniciar y apagar el equipo.
//!
//! Son las tres operaciones mas destructivas del protocolo, y por eso estan
//! aqui separadas: quien decide **si** se hacen es el host, tras comprobar el
//! permiso y, para reiniciar y apagar, tras una confirmacion que una persona
//! tiene que dar en el propio PC. Este crate solo las ejecuta.
//!
//! En Windows habla con la API de Win32. En el resto falla de forma explicita
//! en lugar de fingir que apago algo.

// `deny` y no `forbid`: el backend de Windows necesita llamar a la API del
// sistema. El unico modulo que lo levanta es `platform::windows`, y alli cada
// bloque lleva su motivo.
#![deny(unsafe_code)]

pub mod control;
pub mod error;
pub mod platform;

pub use control::{PowerControl, RecordingPower, UnsupportedPower};
pub use error::{PowerError, PowerResult};
pub use platform::default_power;
