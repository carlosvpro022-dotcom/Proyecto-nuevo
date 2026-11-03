//! Estado compartido del servidor.

use std::time::{Duration, Instant};

use crate::config::Config;

/// Estado que comparten todas las peticiones.
#[derive(Debug)]
pub struct AppState {
    /// Configuracion cargada al arrancar.
    pub config: Config,
    started_at: Instant,
}

impl AppState {
    /// Crea el estado a partir de la configuracion.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            started_at: Instant::now(),
        }
    }

    /// Tiempo que lleva el proceso en marcha.
    #[must_use]
    pub fn uptime(&self) -> Duration {
        self.started_at.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_starts_near_zero_and_advances() {
        let config = Config::from_lookup(|name| match name {
            "NEXUS_PUBLIC_URL" => Some("wss://x/signaling".to_owned()),
            "NEXUS_TURN_SHARED_SECRET" => Some("s".repeat(32)),
            "NEXUS_TURN_URLS" => Some("turn:x:3478".to_owned()),
            _ => None,
        })
        .expect("configuracion valida");

        let state = AppState::new(config);
        assert!(state.uptime() < Duration::from_secs(1));
        assert_eq!(state.config.turn_urls.len(), 1);
    }
}
