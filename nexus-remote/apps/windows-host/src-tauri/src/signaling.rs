//! Comprobacion del servidor de senalizacion desde el host.
//!
//! Esta no describe el proyecto: **intenta conectarse de verdad** al servidor
//! que el usuario tenga configurado, se autentica y se declara disponible. Es
//! lo que permite saber si un despliegue funciona antes de que haya un iPad al
//! otro lado, y distinguir entre "el servidor no responde", "el servidor me
//! rechaza" y "el servidor no tiene relay configurado", que son tres problemas
//! con tres soluciones distintas.
//!
//! El testigo de alta se recibe por parametro y **no se guarda**: hace falta
//! una sola vez, en la primera conexion del host, y guardarlo seria dejar un
//! secreto en disco sin motivo.

use nexus_core::{CapabilitySet, DeviceInfo};
use nexus_security::DeviceIdentity;
use nexus_transport::client::{ConnectRequest, SignalingClient};
use serde::Serialize;

/// Resultado de la comprobacion, tal como lo lee el panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignalingProbe {
    /// Si se llego a autenticar y a quedar disponible.
    pub connected: bool,
    /// Servidores ICE que entrego el servidor.
    pub ice_servers: usize,
    /// Si entre ellos hay un relay TURN utilizable.
    ///
    /// Sin relay, una conexion desde una red movil con NAT simetrico no se
    /// establece. Que el servidor responda no basta: hay que mirar esto.
    pub has_relay: bool,
    /// Si hay descubrimiento (STUN) o retransmision (TURN).
    pub supports_remote_networks: bool,
    /// Que paso, en lenguaje llano.
    pub detail: String,
}

impl SignalingProbe {
    /// Construye el resultado de un fallo.
    fn failed(detail: String) -> Self {
        Self {
            connected: false,
            ice_servers: 0,
            has_relay: false,
            supports_remote_networks: false,
            detail,
        }
    }
}

/// Intenta conectarse al servidor y declararse disponible.
///
/// No devuelve error: cualquier fallo se convierte en un
/// [`SignalingProbe`] con `connected` en `false` y el motivo en `detail`,
/// porque el valor de esta funcion esta justamente en explicar lo que fallo.
pub async fn probe(
    url: &str,
    device: &DeviceInfo,
    capabilities: &CapabilitySet,
    identity: &DeviceIdentity,
    enrollment_token: Option<&str>,
    monitor_count: u8,
) -> SignalingProbe {
    let request = ConnectRequest {
        url,
        device,
        capabilities,
        enrollment_token,
    };

    let (mut client, accepted) = match SignalingClient::connect(request, identity).await {
        Ok(pair) => pair,
        Err(error) => return SignalingProbe::failed(explain(&error)),
    };

    let ice_servers = accepted.ice.servers.len();
    let has_relay = accepted.ice.has_relay();
    let supports_remote_networks = accepted.ice.supports_remote_networks();

    // Declararse disponible es la segunda mitad de la comprobacion: autenticarse
    // no garantiza que el servidor acepte a este dispositivo como host.
    let detail = match client.register_host(monitor_count.max(1)).await {
        Ok(()) => {
            if has_relay {
                "Conectado y disponible. El servidor entrego relay TURN.".to_owned()
            } else {
                "Conectado y disponible, pero el servidor no entrego relay TURN: \
                 una conexion desde una red movil puede no establecerse."
                    .to_owned()
            }
        }
        Err(error) => {
            let _ = client.close().await;
            return SignalingProbe {
                connected: false,
                ice_servers,
                has_relay,
                supports_remote_networks,
                detail: format!(
                    "Autenticado, pero no se pudo declarar disponible: {}",
                    explain(&error)
                ),
            };
        }
    };

    // La comprobacion no deja la conexion abierta: sirve para diagnosticar, no
    // para quedarse a la escucha.
    let _ = client.close().await;

    SignalingProbe {
        connected: true,
        ice_servers,
        has_relay,
        supports_remote_networks,
        detail,
    }
}

/// Traduce un fallo de transporte a algo que se pueda leer y actuar.
fn explain(error: &nexus_transport::TransportError) -> String {
    use nexus_core::protocol::signaling::AuthRejection;
    use nexus_transport::TransportError as E;

    match error {
        E::InsecureUrl(detail) => {
            format!("La direccion no va cifrada: {detail}. Debe empezar por wss://.")
        }
        E::Network(detail) => format!(
            "No se pudo hablar con el servidor: {detail}. \
             Compruebe la direccion, que el servicio este en marcha y que el \
             certificado sea valido."
        ),
        E::AuthRejected {
            reason: AuthRejection::RateLimited,
            retry_after_secs,
        } => format!(
            "El servidor esta limitando los intentos. Reintente en {} s.",
            retry_after_secs.unwrap_or(60)
        ),
        E::AuthRejected {
            reason: AuthRejection::Revoked,
            ..
        } => "Este host esta revocado en el servidor.".to_owned(),
        E::AuthRejected { .. } => "El servidor rechazo la identidad de este host. \
             Si es la primera conexion, hace falta el testigo de alta del servidor."
            .to_owned(),
        E::Security(detail) => detail.clone(),
        E::InvalidIceConfiguration(detail) => {
            format!("El servidor no entrego una configuracion de red utilizable: {detail}.")
        }
        E::ServerError { code, message } => format!("El servidor respondio {code}: {message}"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::protocol::signaling::AuthRejection;
    use nexus_transport::TransportError;

    #[test]
    fn un_fallo_de_red_explica_que_mirar() {
        let detail = explain(&TransportError::Network("conexion rechazada".to_owned()));
        assert!(detail.contains("conexion rechazada"), "{detail}");
        assert!(detail.contains("certificado"), "{detail}");
    }

    #[test]
    fn una_direccion_sin_cifrar_dice_lo_que_hace_falta() {
        let detail = explain(&TransportError::InsecureUrl("ws://x".to_owned()));
        assert!(detail.contains("wss://"), "{detail}");
    }

    #[test]
    fn un_rechazo_de_identidad_apunta_al_testigo_de_alta() {
        // Es la causa mas probable en la primera conexion, y sin esta pista el
        // usuario no tiene forma de adivinarlo.
        let detail = explain(&TransportError::AuthRejected {
            reason: AuthRejection::InvalidCredentials,
            retry_after_secs: None,
        });
        assert!(detail.contains("testigo de alta"), "{detail}");
    }

    #[test]
    fn un_bloqueo_por_intentos_dice_cuanto_esperar() {
        let detail = explain(&TransportError::AuthRejected {
            reason: AuthRejection::RateLimited,
            retry_after_secs: Some(120),
        });
        assert!(detail.contains("120"), "{detail}");
    }

    #[test]
    fn un_reloj_desviado_se_cuenta_tal_cual() {
        let detail = explain(&TransportError::Security(
            "el reloj de este dispositivo esta 600 s desviado".to_owned(),
        ));
        assert!(detail.contains("reloj"), "{detail}");
    }

    #[test]
    fn un_fallo_sin_conexion_no_declara_relay() {
        let probe = SignalingProbe::failed("lo que sea".to_owned());
        assert!(!probe.connected);
        assert!(!probe.has_relay);
        assert!(!probe.supports_remote_networks);
        assert_eq!(probe.ice_servers, 0);
    }

    #[test]
    fn el_resultado_se_serializa_como_lo_espera_el_panel() {
        let json = serde_json::to_value(SignalingProbe {
            connected: true,
            ice_servers: 2,
            has_relay: true,
            supports_remote_networks: true,
            detail: "listo".to_owned(),
        })
        .expect("serializa");
        assert_eq!(json["connected"], true);
        assert_eq!(json["ice_servers"], 2);
        assert_eq!(json["has_relay"], true);
    }
}
