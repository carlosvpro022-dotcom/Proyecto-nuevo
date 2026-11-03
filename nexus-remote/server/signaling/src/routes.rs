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
//! - `GET /v1/signaling`: el WebSocket de senalizacion, con autenticacion de
//!   dispositivo, enrutado entre pares y reenvio de la negociacion WebRTC. Ver
//!   [`crate::ws`].
//!
//! No hay ninguna ruta de administracion. Es deliberado: una interfaz de
//! administracion alcanzable desde Internet es justo lo que este servidor no
//! debe tener. El registro de conexiones se consulta en la maquina.

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
    /// Se publican a proposito: un cliente puede comprobar que le falta algo
    /// antes de intentarlo, en lugar de descubrirlo con un `404` opaco.
    pub pending_endpoints: Vec<&'static str>,
    /// Si el servidor retransmite codigos de emparejamiento.
    ///
    /// Hoy es `false`, y el canal rechaza esos mensajes. El motivo esta en
    /// `docs/architecture/ADR-0010-emparejamiento-por-red.md`.
    pub pairing_relay: bool,
}

/// Construye el enrutador del servicio.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/info", get(info))
        .route("/v1/signaling", get(crate::ws::handler))
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
            pending_endpoints: vec![],
            pairing_relay: false,
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
            "NEXUS_HOST_ENROLLMENT_TOKEN" => Some("t".repeat(32)),
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
        // Un cuerpo que no es JSON (el rechazo del canal de senalizacion es
        // texto plano) se devuelve como nulo en lugar de hacer fallar la
        // prueba: lo que se comprueba en esos casos es el codigo de estado.
        let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
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
    async fn info_no_longer_declares_the_signaling_endpoint_as_pending() {
        let (_, body) = get_json("/v1/info").await;
        let pending = body["pending_endpoints"].as_array().expect("lista");
        assert!(
            !pending.iter().any(|v| v == "/v1/signaling"),
            "el canal ya existe: no debe figurar como pendiente: {pending:?}"
        );
    }

    #[tokio::test]
    async fn info_declares_that_pairing_is_not_relayed() {
        // El canal rechaza los codigos de emparejamiento, y lo dice de
        // antemano en lugar de dejar que el cliente lo descubra al fallar.
        let (_, body) = get_json("/v1/info").await;
        assert_eq!(body["pairing_relay"], false);
    }

    #[tokio::test]
    async fn the_signaling_endpoint_exists_and_demands_a_websocket() {
        // Una peticion HTTP normal contra el canal no sirve: hace falta la
        // mejora a WebSocket. Lo que importa es que ya no es un 404.
        let (status, _) = get_json("/v1/signaling").await;
        assert_ne!(status, StatusCode::NOT_FOUND);
        assert!(
            status.is_client_error(),
            "sin cabeceras de WebSocket debe rechazarse: {status}"
        );
    }

    #[tokio::test]
    async fn there_is_no_administration_route() {
        for path in [
            "/admin",
            "/v1/admin",
            "/v1/audit",
            "/v1/devices",
            "/metrics",
        ] {
            let (status, _) = get_json(path).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{path} no debe existir: una interfaz de administracion no debe                  quedar expuesta a Internet"
            );
        }
    }

    #[tokio::test]
    async fn unknown_routes_return_not_found() {
        let (status, _) = get_json("/admin").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
