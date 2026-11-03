//! # nexus-signaling
//!
//! Servidor de senalizacion propio de NEXUS REMOTE.
//!
//! Su trabajo es que dos dispositivos autorizados se encuentren y negocien una
//! conexion WebRTC. **No participa en la sesion**: el video, el audio y las
//! ordenes de control viajan cifrados extremo a extremo entre los dispositivos,
//! y este servidor no tiene las claves para leerlos.
//!
//! # Estado de implementacion
//!
//! Terminado y probado:
//!
//! - [`config`]: carga y validacion de la configuracion, sin valores por
//!   omision peligrosos.
//! - [`turn`]: derivacion de credenciales TURN temporales compatibles con
//!   Coturn, verificada contra un vector de prueba publico de HMAC-SHA1.
//! - [`routes`]: sondas de salud e informacion del servicio.
//!
//! Pendiente (etapa 3):
//!
//! - El WebSocket `/v1/signaling` con el emparejamiento, la autenticacion mutua
//!   y el reenvio de oferta, respuesta y candidatos ICE.
//! - El registro de pares conectados y el enrutado de mensajes entre ellos.
//! - La terminacion TLS, que en el despliegue documentado la hace un proxy
//!   inverso delante de este proceso.

#![forbid(unsafe_code)]

pub mod config;
pub mod routes;
pub mod state;
pub mod turn;

pub use config::{Config, ConfigError};
pub use state::AppState;
pub use turn::TurnCredential;
