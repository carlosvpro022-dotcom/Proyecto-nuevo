//! Punto de entrada del servidor de senalizacion.

use std::sync::Arc;

use nexus_signaling::{AppState, Config, routes};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("NEXUS_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            // Un error de configuracion se informa y se aborta: arrancar con una
            // configuracion incompleta es peor que no arrancar.
            tracing::error!(%error, "configuracion invalida; el servidor no arranca");
            eprintln!("Error de configuracion: {error}");
            eprintln!("Consulte infrastructure/docker/.env.example");
            return std::process::ExitCode::FAILURE;
        }
    };

    let bind_address = config.bind_address;
    tracing::info!(
        %bind_address,
        public_url = %config.public_url,
        turn_servers = config.turn_urls.len(),
        stun_servers = config.stun_urls.len(),
        "arrancando NEXUS REMOTE signaling"
    );
    tracing::warn!(
        "el canal /v1/signaling aun no esta implementado (etapa 3); solo responden /health y /v1/info"
    );

    let state = Arc::new(AppState::new(config));
    let app = routes::router(state);

    let listener = match tokio::net::TcpListener::bind(bind_address).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(%error, %bind_address, "no se pudo abrir el puerto");
            return std::process::ExitCode::FAILURE;
        }
    };

    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;

    match result {
        Ok(()) => {
            tracing::info!("servidor detenido de forma ordenada");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(%error, "el servidor termino con error");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Espera una senal de parada.
///
/// Se atienden `Ctrl+C` y `SIGTERM` para que Docker pueda detener el contenedor
/// sin matar conexiones a mitad.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "fallo al instalar el manejador de Ctrl+C");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(%error, "fallo al instalar el manejador de SIGTERM"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("recibido Ctrl+C"),
        () = terminate => tracing::info!("recibido SIGTERM"),
    }
}
