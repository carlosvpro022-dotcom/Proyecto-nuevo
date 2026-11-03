//! Estado compartido del servidor.
//!
//! Todo lo mutable vive detras de un unico cerrojo. Es una eleccion
//! deliberada: las operaciones que lo toman son microsegundos de trabajo en
//! memoria (buscar un par en un mapa, encolar un mensaje con `try_send`) y
//! ninguna es `async`, de modo que no hay riesgo de retener el cerrojo
//! esperando a la red. A cambio se evita toda una familia de errores de
//! concurrencia entre el registro de pares, el de identidades y el de intentos.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use nexus_core::Timestamp;
use nexus_core::protocol::signaling::IceServer;
use nexus_security::{AttemptLimiter, Authenticator, RateLimitPolicy};

use crate::audit::AuditLog;
use crate::config::Config;
use crate::enrollment::EnrollmentStore;
use crate::hub::Hub;
use crate::turn::TurnCredential;

/// Estado mutable del servidor.
#[derive(Debug)]
pub struct Shared {
    /// Pares conectados y sesiones en curso.
    pub hub: Hub,
    /// Autenticacion de dispositivos contra el directorio de identidades.
    pub auth: Authenticator<EnrollmentStore>,
    /// Registro de conexiones.
    pub audit: AuditLog,
    /// Limite de intentos por direccion de origen.
    ///
    /// Es independiente del limite por dispositivo que lleva el autenticador:
    /// este frena a quien prueba identificadores distintos desde el mismo
    /// sitio, y aquel a quien insiste contra un dispositivo concreto.
    pub origins: AttemptLimiter,
}

impl Shared {
    /// Descarta estado caducado de todos los componentes.
    pub fn prune(&mut self, now: Timestamp) {
        self.auth.prune(now);
        self.origins.prune(now);
    }
}

/// Estado que comparten todas las peticiones.
#[derive(Debug)]
pub struct AppState {
    /// Configuracion cargada al arrancar.
    pub config: Config,
    started_at: Instant,
    shared: Mutex<Shared>,
}

impl AppState {
    /// Crea el estado a partir de la configuracion.
    #[must_use]
    pub fn new(config: Config) -> Self {
        let store = EnrollmentStore::new(config.host_enrollment_token.clone());
        Self {
            started_at: Instant::now(),
            shared: Mutex::new(Shared {
                hub: Hub::new(),
                auth: Authenticator::new(store),
                audit: AuditLog::new(),
                origins: AttemptLimiter::new(RateLimitPolicy::device_auth()),
            }),
            config,
        }
    }

    /// Toma el cerrojo del estado mutable.
    ///
    /// Si un hilo entro en panico mientras lo tenia, se recupera el contenido
    /// en lugar de propagar el envenenamiento: un fallo al atender una conexion
    /// no debe dejar el servidor entero inservible. El panico ya quedo anotado
    /// por el gestor de panicos.
    ///
    /// # Panics
    ///
    /// No entra en panico.
    pub fn shared(&self) -> MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Tiempo que lleva el proceso en marcha.
    #[must_use]
    pub fn uptime(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Construye la lista de servidores ICE para un dispositivo.
    ///
    /// La credencial TURN se deriva por dispositivo y caduca sola, de modo que
    /// filtrar una no da acceso permanente al relay. `identity` solo sirve para
    /// que los registros de Coturn sean legibles.
    #[must_use]
    pub fn ice_servers(&self, identity: &str, now: Timestamp) -> Vec<IceServer> {
        let mut servers = Vec::with_capacity(2);

        if !self.config.stun_urls.is_empty() {
            servers.push(IceServer {
                urls: self.config.stun_urls.clone(),
                username: None,
                credential: None,
                expires_at: None,
            });
        }

        if !self.config.turn_urls.is_empty() {
            let credential = TurnCredential::derive(
                self.config.turn_shared_secret.as_bytes(),
                identity,
                now,
                self.config.credential_ttl,
            );
            servers.push(credential.to_ice_server(self.config.turn_urls.clone()));
        }

        servers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MIN_SECRET_LEN;

    const NOW: Timestamp = Timestamp(1_730_000_000_000);

    fn config_with(stun: Option<&'static str>) -> Config {
        Config::from_lookup(|name| match name {
            "NEXUS_PUBLIC_URL" => Some("wss://nexus.example/signaling".to_owned()),
            "NEXUS_TURN_SHARED_SECRET" => Some("s".repeat(MIN_SECRET_LEN)),
            "NEXUS_TURN_URLS" => Some("turn:turn.nexus.example:3478".to_owned()),
            "NEXUS_HOST_ENROLLMENT_TOKEN" => Some("t".repeat(MIN_SECRET_LEN)),
            "NEXUS_STUN_URLS" => stun.map(str::to_owned),
            _ => None,
        })
        .expect("configuracion de prueba valida")
    }

    fn state() -> AppState {
        AppState::new(config_with(Some("stun:stun.nexus.example:3478")))
    }

    #[test]
    fn uptime_starts_near_zero() {
        assert!(state().uptime() < Duration::from_secs(1));
    }

    #[test]
    fn the_shared_state_starts_empty() {
        let state = state();
        let shared = state.shared();
        assert_eq!(shared.hub.peer_count(), 0);
        assert!(shared.auth.registry().is_empty());
        assert!(shared.audit.is_empty());
    }

    #[test]
    fn the_enrollment_token_reaches_the_directory() {
        // Si no llegase, el servidor aceptaria hosts nuevos sin testigo.
        use nexus_core::DeviceId;
        use nexus_core::device::{DeviceInfo, DevicePlatform, DeviceRole};
        use nexus_security::DeviceIdentity;

        let state = state();
        let mut shared = state.shared();
        let host = DeviceInfo {
            id: DeviceId::new(),
            display_name: "PC".to_owned(),
            platform: DevicePlatform::Windows,
            role: DeviceRole::Host,
            os_version: "11".to_owned(),
            app_version: "0.1.0".to_owned(),
        };
        let key = DeviceIdentity::generate(host.id).public_key();

        assert!(
            shared
                .auth
                .registry_mut()
                .admit(&host, &key, Some("incorrecto"), NOW)
                .is_err()
        );
        assert!(
            shared
                .auth
                .registry_mut()
                .admit(&host, &key, Some(&"t".repeat(MIN_SECRET_LEN)), NOW)
                .is_ok()
        );
    }

    #[test]
    fn ice_servers_include_stun_and_turn_with_credentials() {
        let state = state();
        let servers = state.ice_servers("ipad", NOW);
        assert_eq!(servers.len(), 2);
        assert!(servers.iter().all(IceServer::is_usable));

        let turn = servers
            .iter()
            .find(|s| s.urls.iter().any(|u| u.starts_with("turn:")))
            .expect("hay TURN");
        assert!(turn.username.is_some(), "TURN necesita credencial");
        assert!(turn.expires_at.is_some(), "y debe caducar");
    }

    #[test]
    fn without_stun_configured_only_turn_is_offered() {
        let state = AppState::new(config_with(None));
        let servers = state.ice_servers("ipad", NOW);
        assert_eq!(servers.len(), 1);
        assert!(servers[0].urls[0].starts_with("turn:"));
    }

    #[test]
    fn each_device_gets_its_own_turn_credential() {
        let state = state();
        let first = state.ice_servers("ipad", NOW);
        let second = state.ice_servers("iphone", NOW);
        assert_ne!(
            first.last().expect("turn").credential,
            second.last().expect("turn").credential
        );
    }

    #[test]
    fn turn_credentials_expire_within_the_configured_window() {
        let state = state();
        let servers = state.ice_servers("ipad", NOW);
        let expires = servers.last().expect("turn").expires_at.expect("caduca");
        let expected = NOW.0 + state.config.credential_ttl.as_millis() as u64;
        assert_eq!(expires.0, expected);
    }
}
