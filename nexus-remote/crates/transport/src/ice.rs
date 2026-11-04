//! Configuracion ICE: descubrimiento y retransmision.
//!
//! WebRTC intenta primero una conexion directa entre los dos dispositivos. Para
//! lograrlo necesita saber que direccion publica tiene cada uno, y eso lo
//! resuelve un servidor STUN. Cuando el NAT de alguno de los extremos no admite
//! conexion directa (tipicamente NAT simetrico o CGNAT de operador movil), el
//! trafico pasa por un servidor TURN propio.
//!
//! El TURN solo reenvia paquetes ya cifrados por DTLS-SRTP: ve el trafico pasar
//! pero no puede leer el escritorio ni las ordenes de control.

use nexus_core::protocol::signaling::IceServer;
use serde::{Deserialize, Serialize};

use crate::error::{TransportError, TransportResult};

/// Cuanto se permite intentar antes de recurrir al relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportPolicy {
    /// Intentar conexion directa y recurrir a TURN solo si hace falta.
    ///
    /// Es lo normal: la conexion directa tiene menos latencia y no consume
    /// ancho de banda del servidor propio.
    #[default]
    All,
    /// Forzar siempre el paso por TURN.
    ///
    /// Util cuando no se quiere revelar la direccion IP local al otro extremo,
    /// a cambio de mas latencia y de consumo en el servidor.
    RelayOnly,
}

/// Configuracion ICE completa que recibe el cliente al autenticarse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceConfiguration {
    /// Servidores STUN y TURN a usar.
    pub servers: Vec<IceServer>,
    /// Politica de transporte.
    #[serde(default)]
    pub policy: TransportPolicy,
}

impl IceConfiguration {
    /// Crea la configuracion con la politica por omision.
    #[must_use]
    pub fn new(servers: Vec<IceServer>) -> Self {
        Self {
            servers,
            policy: TransportPolicy::All,
        }
    }

    /// Comprueba que los servidores declarados sirven para algo.
    ///
    /// **No comprueba si la conexion podra salir de la red local**: eso
    /// depende tambien de si el equipo anuncia una direccion publica propia,
    /// que esta capa no conoce. Lo decide quien abre la conexion, que si tiene
    /// las dos piezas.
    ///
    /// Una lista vacia es valida: es la configuracion de un equipo que se
    /// alcanza directamente porque tiene un puerto abierto en su router.
    ///
    /// # Errors
    ///
    /// [`TransportError::InvalidIceConfiguration`] si algun servidor no es
    /// utilizable, o si se exige relay y no hay ningun TURN con credenciales.
    pub fn validate(&self) -> TransportResult<()> {
        for (index, server) in self.servers.iter().enumerate() {
            if !server.is_usable() {
                return Err(TransportError::InvalidIceConfiguration(format!(
                    "el servidor {index} no es utilizable (sin URL, o TURN sin credenciales)"
                )));
            }
        }

        if self.policy == TransportPolicy::RelayOnly && !self.has_relay() {
            return Err(TransportError::InvalidIceConfiguration(
                "se exige relay pero no hay ningun servidor TURN utilizable".to_owned(),
            ));
        }

        Ok(())
    }

    /// Indica si hay algun TURN utilizable.
    #[must_use]
    pub fn has_relay(&self) -> bool {
        self.servers.iter().any(|s| {
            s.is_usable()
                && s.urls
                    .iter()
                    .any(|u| u.starts_with("turn:") || u.starts_with("turns:"))
        })
    }

    /// Indica si hay algun STUN.
    #[must_use]
    pub fn has_stun(&self) -> bool {
        self.servers
            .iter()
            .any(|s| s.urls.iter().any(|u| u.starts_with("stun:")))
    }

    /// Indica si la conexion podria establecerse desde fuera de la red local.
    ///
    /// Hace falta al menos descubrimiento (STUN) o retransmision (TURN): sin
    /// ninguno de los dos, solo funcionaria dentro de la misma red, que es
    /// exactamente el fallo que este proyecto no debe tener.
    #[must_use]
    pub fn supports_remote_networks(&self) -> bool {
        self.has_stun() || self.has_relay()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::Timestamp;

    fn stun() -> IceServer {
        IceServer {
            urls: vec!["stun:stun.nexus.example:3478".to_owned()],
            username: None,
            credential: None,
            expires_at: None,
        }
    }

    fn turn() -> IceServer {
        IceServer {
            urls: vec![
                "turn:turn.nexus.example:3478?transport=udp".to_owned(),
                "turns:turn.nexus.example:5349?transport=tcp".to_owned(),
            ],
            username: Some("1730000000:nexus".to_owned()),
            credential: Some("credencial-temporal".to_owned()),
            expires_at: Some(Timestamp(1_730_000_000_000)),
        }
    }

    #[test]
    fn a_stun_plus_turn_configuration_is_valid() {
        let config = IceConfiguration::new(vec![stun(), turn()]);
        config.validate().expect("deberia valer");
        assert!(config.has_stun());
        assert!(config.has_relay());
        assert!(config.supports_remote_networks());
    }

    #[test]
    fn an_empty_configuration_is_structurally_valid() {
        // Es la configuracion de un equipo que se alcanza directamente porque
        // tiene un puerto abierto en su router. Si eso basta o no lo decide
        // quien abre la conexion, que sabe si hay direccion publica declarada.
        let config = IceConfiguration::new(vec![]);
        config.validate().expect("no hay nada mal formado");
        assert!(
            !config.supports_remote_networks(),
            "pero por si sola no llega a otra red"
        );
    }

    #[test]
    fn a_turn_server_without_credentials_is_rejected() {
        let broken = IceServer {
            username: None,
            credential: None,
            ..turn()
        };
        let err = IceConfiguration::new(vec![broken]).validate().unwrap_err();
        assert!(matches!(err, TransportError::InvalidIceConfiguration(_)));
    }

    #[test]
    fn relay_only_without_turn_is_rejected() {
        let config = IceConfiguration {
            servers: vec![stun()],
            policy: TransportPolicy::RelayOnly,
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, TransportError::InvalidIceConfiguration(_)),
            "{err:?}"
        );
    }

    #[test]
    fn relay_only_with_turn_is_valid() {
        let config = IceConfiguration {
            servers: vec![turn()],
            policy: TransportPolicy::RelayOnly,
        };
        config.validate().expect("con TURN si vale");
    }

    #[test]
    fn a_stun_only_configuration_still_reaches_remote_networks() {
        // Sin TURN muchas conexiones funcionan igual; lo que no funcionaria es
        // una configuracion sin STUN ni TURN.
        let config = IceConfiguration::new(vec![stun()]);
        config.validate().expect("valida");
        assert!(config.supports_remote_networks());
        assert!(!config.has_relay());
    }

    #[test]
    fn a_configuration_with_no_discovery_cannot_work_from_the_internet() {
        let local_only = IceServer {
            urls: vec!["udp:192.168.1.10:9000".to_owned()],
            username: None,
            credential: None,
            expires_at: None,
        };
        let config = IceConfiguration::new(vec![local_only]);
        assert!(
            !config.supports_remote_networks(),
            "sin STUN ni TURN solo funcionaria en la red local"
        );
    }

    #[test]
    fn the_default_policy_tries_direct_first() {
        assert_eq!(TransportPolicy::default(), TransportPolicy::All);
        assert_eq!(
            IceConfiguration::new(vec![stun()]).policy,
            TransportPolicy::All
        );
    }

    #[test]
    fn configuration_roundtrips_through_json() {
        let config = IceConfiguration {
            servers: vec![stun(), turn()],
            policy: TransportPolicy::RelayOnly,
        };
        let raw = serde_json::to_string(&config).expect("serializa");
        let back: IceConfiguration = serde_json::from_str(&raw).expect("deserializa");
        assert_eq!(back, config);
    }

    #[test]
    fn the_policy_defaults_when_missing_from_json() {
        let raw = r#"{"servers":[{"urls":["stun:a:3478"]}]}"#;
        let config: IceConfiguration = serde_json::from_str(raw).expect("deserializa");
        assert_eq!(config.policy, TransportPolicy::All);
    }
}
