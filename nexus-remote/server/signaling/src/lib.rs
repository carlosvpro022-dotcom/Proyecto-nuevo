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
//! - [`enrollment`]: directorio de identidades, con el testigo de alta de
//!   hosts y el rechazo de apropiaciones de identificador.
//! - [`hub`]: pares conectados, sesiones y enrutado entre extremos.
//! - [`ws`]: el WebSocket `/v1/signaling` completo: autenticacion del
//!   dispositivo, peticion y resolucion de sesion, reenvio de oferta,
//!   respuesta, candidatos ICE y pruebas de atadura.
//! - [`audit`]: registro de conexiones.
//!
//! Pendiente:
//!
//! - **Retransmision de codigos de emparejamiento.** El canal la rechaza a
//!   proposito; el motivo esta en
//!   `docs/architecture/ADR-0010-emparejamiento-por-red.md`.
//! - **Terminacion TLS**, que en el despliegue documentado la hace un proxy
//!   inverso delante de este proceso.
//! - **Verificacion contra Internet real**: todo lo de arriba esta probado
//!   contra sockets locales. Falta desplegarlo con un dominio, un certificado y
//!   un Coturn en marcha.

#![forbid(unsafe_code)]

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

pub mod audit;
pub mod config;
pub mod enrollment;
pub mod hub;
pub mod routes;
pub mod state;
pub mod turn;
pub mod ws;

pub use config::{Config, ConfigError};
pub use state::AppState;
pub use turn::TurnCredential;

/// Atiende peticiones en `listener` hasta que `shutdown` termine.
///
/// Es el unico sitio donde se monta el servicio, y lo usan tanto el binario
/// como las pruebas de integracion. Que sea el mismo importa: el servicio se
/// monta **con informacion de conexion**, que es de donde sale la direccion de
/// origen con la que se limita el numero de intentos. Si las pruebas montasen
/// el servicio por su cuenta, podrian pasar sin que ese limite funcionase en
/// produccion.
///
/// # Errors
///
/// El error de entrada y salida que devuelva el servidor.
pub async fn serve<F>(
    listener: tokio::net::TcpListener,
    state: Arc<AppState>,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let service = routes::router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, service)
        .with_graceful_shutdown(shutdown)
        .await
}
