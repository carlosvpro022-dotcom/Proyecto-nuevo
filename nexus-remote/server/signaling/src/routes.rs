//! Rutas HTTP del servidor de senalizacion.
//!
//! # Estado de implementacion
//!
//! Lo que esta implementado:
//!
//! - `GET /health`: sonda para el orquestador y el balanceador.
//! - `GET /v1/info`: version del servicio y del protocolo, para que un cliente
//!   sepa si puede hablar con este servidor antes de intentar autenticarse.
//!
//! Lo que **falta** y corresponde a la etapa 3:
//!
//! - `GET /v1/signaling`: el WebSocket donde ocurre el emparejamiento, la
//!   autenticacion mutua y el intercambio de oferta, respuesta y candidatos
//!   ICE. La maquinaria que necesita ya existe y esta probada
//!   (`nexus_security::Authenticator`, `nexus_core::protocol::signaling`,
//!   `nexus_signaling::turn`); lo que queda es conectarla a un WebSocket y
//!   mantener el registro de pares conectados.
//!
//! Esa ruta no esta declarada a proposito: un `404` es una respuesta honesta.
//! Declararla para que devuelva algo vacio daria la impresion de que el canal
//! funciona.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use serde::Serialize;

use crate::state::AppState;

/// Respuesta de la sonda de salud.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Health {
    /// Siempre `"ok"` si el proceso responde.
    pub status: &'static str,
    /// Segundos que lleva el proceso en marcha.
    pub uptime_secs: u64,
}

/// Respuesta con la informacion del servicio.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ServiceInfo {
    /// Nombre del producto.
    pub product: &'static str,
    /// Version del servidor.
    pub version: &'static str,
    /// Version del protocolo que implementa.
    pub protocol_version: u16,
    /// Version minima del protocolo con la que sabe hablar.
    pub min_protocol_version: u16,
    /// Si hay al menos un servidor TURN configurado.
    pub relay_configured: bool,
    /// Rutas que todavia no estan implementadas.
    ///
    /// Se publican a proposito: un cliente puede comprobar que el canal de
    /// senalizacion aun no existe, en lugar de descubrirlo con un `404` opaco.
    pub pending_endpoints: Vec<&'static str>,
}

/// Construye el enrutador del servicio.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/info", get(info))
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(Health {
            status: "ok",
            uptime_secs: state.uptime().as_secs(),
        }),
    )
}

async fn info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(ServiceInfo {
            product: nexus_core::PRODUCT_NAME,
            version: env!("CARGO_PKG_VERSION"),
            protocol_version: nexus_core::PROTOCOL_VERSION,
            min_protocol_version: nexus_core::MIN_SUPPORTED_PROTOCOL,
            relay_configured: !state.config.turn_urls.is_empty(),
            pending_endpoints: vec!["/v1/signaling"],
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn state() -> Arc<AppState> {
        let config = Config::from_lookup(|name| match name {
            "NEXUS_PUBLIC_URL" => Some("wss://nexus.example/signaling".to_owned()),
            "NEXUS_TURN_SHARED_SECRET" => Some("s".repeat(32)),
            "NEXUS_TURN_URLS" => Some("turn:turn.nexus.example:3478".to_owned()),
            _ => None,
        })
        .expect("configuracion de prueba valida");
        Arc::new(AppState::new(config))
    }

    async fn get_json(path: &str) -> (StatusCode, serde_json::Value) {
        let response = router(state())
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("peticion"),
            )
            .await
            .expect("respuesta");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("cuerpo legible");
        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("json valido")
        };
        (status, value)
    }

    #[tokio::test]
    async fn health_reports_ok() {
        let (status, body) = get_json("/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert!(body["uptime_secs"].is_u64());
    }

    #[tokio::test]
    async fn info_reports_the_protocol_version() {
        let (status, body) = get_json("/v1/info").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["product"], nexus_core::PRODUCT_NAME);
        assert_eq!(body["protocol_version"], nexus_core::PROTOCOL_VERSION);
        assert_eq!(body["relay_configured"], true);
    }

    #[tokio::test]
    async fn info_declares_the_signaling_endpoint_as_pending() {
        let (_, body) = get_json("/v1/info").await;
        let pending = body["pending_endpoints"].as_array().expect("lista");
        assert!(
            pending.iter().any(|v| v == "/v1/signaling"),
            "el canal de senalizacion debe figurar como pendiente: {pending:?}"
        );
    }

    #[tokio::test]
    async fn the_signaling_endpoint_is_not_declared_yet() {
        // Es un 404 a proposito: no existe todavia y no se finge que exista.
        let (status, _) = get_json("/v1/signaling").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_routes_return_not_found() {
        let (status, _) = get_json("/admin").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
