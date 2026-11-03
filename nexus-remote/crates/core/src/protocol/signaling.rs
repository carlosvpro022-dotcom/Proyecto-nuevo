//! Protocolo de senalizacion: dispositivo <-> servidor propio.
//!
//! Viaja como JSON sobre WebSocket con TLS. El servidor coordina pero no
//! participa en la sesion: no ve el contenido del escritorio ni las ordenes de
//! control, que van cifradas extremo a extremo por DTLS en el canal de datos.
//!
//! Secuencia normal de un controlador:
//!
//! 1. `Hello` con su clave publica -> el servidor responde `Challenge` con un
//!    nonce aleatorio.
//! 2. `AuthProof` con su propio nonce y la firma Ed25519 de la transcripcion ->
//!    `AuthAccepted` (con los servidores ICE) o `AuthRejected`.
//! 3. `RequestSession` contra el identificador del host. El host recibe
//!    `SessionRequested` y contesta `AcceptSession` o `RejectSession`.
//! 4. Intercambio de `Offer`, `Answer` y `IceCandidate` con el host.
//! 5. `PeerAuth` en ambas direcciones, cuando cada extremo ya conoce las dos
//!    huellas DTLS.
//!
//! El host sigue el mismo arranque y luego envia `RegisterHost` para quedar
//! localizable, que el servidor confirma con `HostRegistered`.
//!
//! # Por que hace falta el paso 5
//!
//! Los pasos 1 y 2 autentican el dispositivo **ante el servidor**. Eso no basta:
//! el servidor reenvia el SDP, y el SDP lleva la huella del certificado DTLS.
//! Un servidor comprometido podria sustituirla y leer la sesion completa sin que
//! ninguno de los dos extremos lo notase.
//!
//! El paso 5 lo cierra: cada extremo firma las dos huellas DTLS de la sesion y
//! comprueba la firma del otro contra la huella que negocio de verdad. La
//! transcripcion y su verificacion estan en
//! `nexus_security::binding::SessionBinding`. Si el servidor sustituye una
//! huella, la sesion se aborta.

use serde::{Deserialize, Serialize};

use crate::capabilities::CapabilitySet;
use crate::device::DeviceInfo;
use crate::ids::{DeviceId, PairingId, SessionId};
use crate::permissions::PermissionSet;
use crate::time::Timestamp;

/// Servidor ICE que el cliente debe usar.
///
/// Las credenciales TURN son efimeras: el servidor las genera por sesion con el
/// mecanismo de usuario temporal de Coturn, de modo que filtrar una no da
/// acceso permanente al relay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceServer {
    /// Lista de URL (`stun:`, `turn:`, `turns:`).
    pub urls: Vec<String>,
    /// Usuario temporal, solo para TURN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Credencial temporal, solo para TURN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// Instante en que la credencial deja de valer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
}

impl IceServer {
    /// Indica si la entrada exige credenciales (TURN) y las trae.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        if self.urls.is_empty() {
            return false;
        }
        let needs_credentials = self
            .urls
            .iter()
            .any(|u| u.starts_with("turn:") || u.starts_with("turns:"));
        !needs_credentials || (self.username.is_some() && self.credential.is_some())
    }
}

/// Motivo por el que el servidor rechaza una autenticacion.
///
/// Los motivos son deliberadamente poco especificos hacia el cliente: no se
/// distingue "dispositivo desconocido" de "firma invalida" para no confirmar a
/// un atacante que un identificador existe. El detalle queda en el registro
/// del servidor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthRejection {
    /// Credenciales o firma no validas.
    InvalidCredentials,
    /// El dispositivo fue revocado.
    Revoked,
    /// Demasiados intentos; hay que esperar.
    RateLimited,
    /// La version de protocolo no es compatible.
    ProtocolMismatch,
}

/// Mensajes que el dispositivo envia al servidor de senalizacion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ClientToServer {
    /// Primer mensaje: anuncia quien dice ser y con que clave.
    Hello {
        /// Datos declarados del dispositivo. Informativos: no son de confianza.
        device: DeviceInfo,
        /// Capacidades que soporta.
        capabilities: CapabilitySet,
        /// Clave publica Ed25519 del dispositivo, en hexadecimal.
        ///
        /// El servidor la compara con la que tenga registrada para ese
        /// identificador. Si no coincide, rechaza la conexion: eso impide que
        /// otro dispositivo se apropie de un identificador ya conocido.
        public_key_hex: String,
        /// Testigo de alta, solo en la primera conexion de un host.
        ///
        /// Sirve para que nadie pueda registrar el identificador de un host
        /// antes que el propio host. No se guarda en ningun sitio: se usa una
        /// vez y se descarta.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enrollment_token: Option<String>,
    },
    /// Respuesta al reto del servidor.
    AuthProof {
        /// Identificador del dispositivo.
        device_id: DeviceId,
        /// Nonce que aporta el dispositivo, en hexadecimal.
        ///
        /// Que ambos extremos aporten nonce es lo que impide que el servidor
        /// fije por completo lo que el dispositivo firma.
        client_nonce_hex: String,
        /// Firma Ed25519 sobre la transcripcion de autenticacion, en hexadecimal.
        signature_hex: String,
    },
    /// El host se declara disponible para recibir sesiones.
    RegisterHost {
        /// Monitores disponibles, para que el cliente pueda elegir antes de conectar.
        monitor_count: u8,
    },
    /// El controlador pide sesion contra un host concreto.
    RequestSession {
        /// Host objetivo.
        host_id: DeviceId,
        /// Permisos que el controlador pretende ejercer.
        requested_permissions: PermissionSet,
    },
    /// El host acepta la sesion propuesta (mensaje del host al servidor).
    AcceptSession {
        /// Sesion que se acepta.
        session_id: SessionId,
        /// Capacidades que quedan tras negociar con el controlador.
        negotiated_capabilities: CapabilitySet,
        /// Permisos que el host concede de verdad a esta sesion.
        ///
        /// Los decide el host a partir de su registro de dispositivos, no el
        /// servidor y no lo que pidiera el controlador.
        effective_permissions: PermissionSet,
    },
    /// El host rechaza la sesion propuesta.
    RejectSession {
        /// Sesion que se rechaza.
        session_id: SessionId,
        /// Texto informativo para el controlador.
        reason: String,
    },
    /// Prueba de atadura de la sesion al canal cifrado.
    ///
    /// Se envia despues de intercambiar oferta y respuesta, cuando cada extremo
    /// ya conoce las dos huellas DTLS. El servidor solo la reenvia: no puede
    /// forjarla ni alterarla sin que el otro extremo lo detecte.
    PeerAuth {
        /// Sesion a la que pertenece.
        session_id: SessionId,
        /// Huella DTLS propia, en formato canonico de RFC 4572.
        fingerprint: String,
        /// Firma Ed25519 sobre la transcripcion de atadura, en hexadecimal.
        signature_hex: String,
    },
    /// Descripcion SDP de oferta.
    Offer {
        /// Sesion a la que pertenece.
        session_id: SessionId,
        /// SDP en texto.
        sdp: String,
    },
    /// Descripcion SDP de respuesta.
    Answer {
        /// Sesion a la que pertenece.
        session_id: SessionId,
        /// SDP en texto.
        sdp: String,
    },
    /// Candidato ICE descubierto.
    IceCandidate {
        /// Sesion a la que pertenece.
        session_id: SessionId,
        /// Candidato en formato SDP.
        candidate: String,
        /// Identificador del medio.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sdp_mid: Option<String>,
    },
    /// Canje de un codigo de emparejamiento temporal.
    ///
    /// **El servidor rechaza este mensaje a dia de hoy.** Ver
    /// `docs/architecture/ADR-0010-emparejamiento-por-red.md`: el codigo tiene
    /// unos 40 bits de entropia y, tal como esta definido este mensaje,
    /// viajaria en claro por el servidor, que es justo a quien el diseno no
    /// concede confianza. El mensaje se conserva porque el canje si ocurre en
    /// la red local, y para no cambiar el protocolo cuando se implemente.
    RedeemPairingCode {
        /// Host contra el que se canjea, tomado de la invitacion.
        host_id: DeviceId,
        /// Intento de emparejamiento al que corresponde.
        pairing_id: PairingId,
        /// Codigo mostrado por el host.
        code: String,
        /// Clave publica Ed25519 del nuevo dispositivo, en hexadecimal.
        public_key_hex: String,
    },
    /// Cierre ordenado de una sesion.
    Bye {
        /// Sesion que se cierra.
        session_id: SessionId,
    },
    /// Latido para mantener el WebSocket vivo a traves de NAT y proxies.
    Ping,
}

/// Mensajes que el servidor envia al dispositivo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ServerToClient {
    /// Reto a firmar para demostrar la posesion de la clave privada.
    Challenge {
        /// Nonce aleatorio en hexadecimal.
        nonce_hex: String,
        /// Hora del servidor, para detectar relojes desviados.
        server_time: Timestamp,
    },
    /// Autenticacion correcta.
    ///
    /// Deliberadamente no lleva permisos: el servidor de senalizacion no
    /// concede ninguno. Los permisos de una sesion los decide el host y llegan
    /// en [`ServerToClient::SessionAccepted`].
    AuthAccepted {
        /// Sesion de senalizacion asignada.
        session_id: SessionId,
        /// Servidores ICE a usar.
        ice_servers: Vec<IceServer>,
    },
    /// El host quedo registrado y localizable.
    HostRegistered {
        /// Identificador con el que quedo registrado.
        host_id: DeviceId,
    },
    /// Autenticacion rechazada.
    AuthRejected {
        /// Motivo, deliberadamente generico.
        reason: AuthRejection,
        /// Segundos que conviene esperar antes de reintentar.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_secs: Option<u32>,
    },
    /// Un controlador autorizado quiere abrir sesion (mensaje hacia el host).
    SessionRequested {
        /// Sesion propuesta.
        session_id: SessionId,
        /// Controlador que la pide.
        controller: DeviceInfo,
        /// Capacidades que anuncio el controlador al conectarse.
        ///
        /// El host las necesita para calcular la interseccion con las suyas y
        /// devolverla en [`ServerToClient::SessionAccepted`]. Sin ellas no
        /// podria negociar nada.
        controller_capabilities: CapabilitySet,
        /// Permisos que pretende ejercer.
        requested_permissions: PermissionSet,
    },
    /// El host acepto; empieza la negociacion WebRTC.
    SessionAccepted {
        /// Sesion aceptada.
        session_id: SessionId,
        /// Capacidades negociadas entre ambos extremos.
        negotiated_capabilities: CapabilitySet,
        /// Permisos efectivos de la sesion.
        effective_permissions: PermissionSet,
    },
    /// El host rechazo la sesion.
    SessionRejected {
        /// Sesion rechazada.
        session_id: SessionId,
        /// Texto informativo.
        reason: String,
    },
    /// Reenvio de una oferta del otro extremo.
    Offer {
        /// Sesion a la que pertenece.
        session_id: SessionId,
        /// SDP en texto.
        sdp: String,
    },
    /// Reenvio de una respuesta del otro extremo.
    Answer {
        /// Sesion a la que pertenece.
        session_id: SessionId,
        /// SDP en texto.
        sdp: String,
    },
    /// Reenvio de un candidato ICE del otro extremo.
    IceCandidate {
        /// Sesion a la que pertenece.
        session_id: SessionId,
        /// Candidato en formato SDP.
        candidate: String,
        /// Identificador del medio.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sdp_mid: Option<String>,
    },
    /// Reenvio de la prueba de atadura del otro extremo.
    PeerAuth {
        /// Sesion a la que pertenece.
        session_id: SessionId,
        /// Dispositivo que firmo, segun el servidor.
        ///
        /// Es un dato del servidor, no una afirmacion verificada: lo que vale
        /// es la firma, que cubre los dos identificadores. Si el servidor
        /// mintiese aqui, la verificacion fallaria.
        device_id: DeviceId,
        /// Huella DTLS que declara el otro extremo.
        fingerprint: String,
        /// Firma Ed25519 sobre la transcripcion de atadura.
        signature_hex: String,
    },
    /// El otro extremo desaparecio.
    PeerGone {
        /// Sesion afectada.
        session_id: SessionId,
    },
    /// Emparejamiento completado: el dispositivo ya esta autorizado.
    PairingCompleted {
        /// Intento de emparejamiento.
        pairing_id: PairingId,
        /// Host con el que quedo emparejado.
        host_id: DeviceId,
        /// Permisos concedidos.
        permissions: PermissionSet,
    },
    /// Error de protocolo o de aplicacion.
    Error {
        /// Codigo estable para que el cliente decida.
        code: String,
        /// Mensaje legible.
        message: String,
    },
    /// Respuesta al latido.
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DevicePlatform, DeviceRole};

    fn device() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId::new(),
            display_name: "iPad Pro".to_owned(),
            platform: DevicePlatform::IpadOs,
            role: DeviceRole::Controller,
            os_version: "18.1".to_owned(),
            app_version: "0.1.0".to_owned(),
        }
    }

    fn roundtrip_c2s(message: &ClientToServer) {
        let raw = serde_json::to_string(message).expect("serializa");
        let back: ClientToServer = serde_json::from_str(&raw).expect("deserializa");
        assert_eq!(message, &back, "fallo con {raw}");
    }

    fn roundtrip_s2c(message: &ServerToClient) {
        let raw = serde_json::to_string(message).expect("serializa");
        let back: ServerToClient = serde_json::from_str(&raw).expect("deserializa");
        assert_eq!(message, &back, "fallo con {raw}");
    }

    #[test]
    fn client_messages_roundtrip() {
        let session_id = SessionId::new();
        for message in [
            ClientToServer::Hello {
                device: device(),
                capabilities: CapabilitySet::empty(),
                public_key_hex: "ab".repeat(32),
                enrollment_token: None,
            },
            ClientToServer::Hello {
                device: device(),
                capabilities: CapabilitySet::empty(),
                public_key_hex: "ab".repeat(32),
                enrollment_token: Some("testigo-de-alta".to_owned()),
            },
            ClientToServer::AuthProof {
                device_id: DeviceId::new(),
                client_nonce_hex: "cd".repeat(32),
                signature_hex: "ab".repeat(64),
            },
            ClientToServer::AcceptSession {
                session_id,
                negotiated_capabilities: CapabilitySet::empty(),
                effective_permissions: PermissionSet::standard(),
            },
            ClientToServer::RejectSession {
                session_id,
                reason: "dispositivo no autorizado".to_owned(),
            },
            ClientToServer::PeerAuth {
                session_id,
                fingerprint: "sha-256 AA:BB".to_owned(),
                signature_hex: "ef".repeat(64),
            },
            ClientToServer::RegisterHost { monitor_count: 2 },
            ClientToServer::RequestSession {
                host_id: DeviceId::new(),
                requested_permissions: PermissionSet::standard(),
            },
            ClientToServer::Offer {
                session_id,
                sdp: "v=0\r\n".to_owned(),
            },
            ClientToServer::Answer {
                session_id,
                sdp: "v=0\r\n".to_owned(),
            },
            ClientToServer::IceCandidate {
                session_id,
                candidate: "candidate:1 1 UDP 1 192.0.2.1 3478 typ host".to_owned(),
                sdp_mid: Some("0".to_owned()),
            },
            ClientToServer::RedeemPairingCode {
                host_id: DeviceId::new(),
                pairing_id: PairingId::new(),
                code: "483921".to_owned(),
                public_key_hex: "cd".repeat(32),
            },
            ClientToServer::Bye { session_id },
            ClientToServer::Ping,
        ] {
            roundtrip_c2s(&message);
        }
    }

    #[test]
    fn server_messages_roundtrip() {
        let session_id = SessionId::new();
        for message in [
            ServerToClient::Challenge {
                nonce_hex: "ff".repeat(32),
                server_time: Timestamp(1_700_000_000_000),
            },
            ServerToClient::AuthAccepted {
                session_id,
                ice_servers: vec![IceServer {
                    urls: vec!["stun:stun.nexus.example:3478".to_owned()],
                    username: None,
                    credential: None,
                    expires_at: None,
                }],
            },
            ServerToClient::HostRegistered {
                host_id: DeviceId::new(),
            },
            ServerToClient::PeerAuth {
                session_id,
                device_id: DeviceId::new(),
                fingerprint: "sha-256 AA:BB".to_owned(),
                signature_hex: "ef".repeat(64),
            },
            ServerToClient::AuthRejected {
                reason: AuthRejection::RateLimited,
                retry_after_secs: Some(60),
            },
            ServerToClient::SessionRequested {
                session_id,
                controller: device(),
                controller_capabilities: CapabilitySet::empty(),
                requested_permissions: PermissionSet::view_only(),
            },
            ServerToClient::SessionAccepted {
                session_id,
                negotiated_capabilities: CapabilitySet::empty(),
                effective_permissions: PermissionSet::view_only(),
            },
            ServerToClient::SessionRejected {
                session_id,
                reason: "ocupado".to_owned(),
            },
            ServerToClient::PeerGone { session_id },
            ServerToClient::PairingCompleted {
                pairing_id: PairingId::new(),
                host_id: DeviceId::new(),
                permissions: PermissionSet::standard(),
            },
            ServerToClient::Error {
                code: "bad_request".to_owned(),
                message: "faltan datos".to_owned(),
            },
            ServerToClient::Pong,
        ] {
            roundtrip_s2c(&message);
        }
    }

    #[test]
    fn messages_are_tagged_with_snake_case_type() {
        let raw = serde_json::to_string(&ClientToServer::Ping).expect("serializa");
        assert_eq!(raw, r#"{"type":"ping"}"#);

        let raw =
            serde_json::to_string(&ClientToServer::RegisterHost { monitor_count: 1 }).expect("ok");
        assert!(raw.contains(r#""type":"register_host""#), "{raw}");
    }

    #[test]
    fn the_enrollment_token_is_omitted_when_absent() {
        let raw = serde_json::to_string(&ClientToServer::Hello {
            device: device(),
            capabilities: CapabilitySet::empty(),
            public_key_hex: "ab".repeat(32),
            enrollment_token: None,
        })
        .expect("serializa");
        assert!(
            !raw.contains("enrollment_token"),
            "no debe viajar un campo vacio: {raw}"
        );
    }

    #[test]
    fn pairing_completion_does_not_carry_a_server_supplied_host_key() {
        // La clave publica del host llega al controlador en la invitacion, que
        // viaja fuera de banda. Si llegase en este mensaje, el controlador
        // estaria confiando en el servidor para saber con quien habla, y la
        // autenticacion mutua posterior no demostraria nada.
        let raw = serde_json::to_string(&ServerToClient::PairingCompleted {
            pairing_id: PairingId::new(),
            host_id: DeviceId::new(),
            permissions: PermissionSet::standard(),
        })
        .expect("serializa");
        assert!(
            !raw.contains("public_key"),
            "el servidor no debe decirle al controlador cual es la clave del host: {raw}"
        );
    }

    #[test]
    fn authentication_acceptance_grants_no_permissions() {
        // El servidor de senalizacion no concede permisos. Si este mensaje
        // volviese a llevarlos, habria dos fuentes de verdad.
        let raw = serde_json::to_string(&ServerToClient::AuthAccepted {
            session_id: SessionId::new(),
            ice_servers: vec![],
        })
        .expect("serializa");
        assert!(
            !raw.contains("permission"),
            "el servidor no debe hablar de permisos: {raw}"
        );
    }

    #[test]
    fn unknown_message_type_is_an_error() {
        let result: Result<ClientToServer, _> =
            serde_json::from_str(r#"{"type":"escalar_privilegios"}"#);
        assert!(
            result.is_err(),
            "un tipo desconocido no debe aceptarse en silencio"
        );
    }

    #[test]
    fn stun_only_entry_needs_no_credentials() {
        let server = IceServer {
            urls: vec!["stun:stun.nexus.example:3478".to_owned()],
            username: None,
            credential: None,
            expires_at: None,
        };
        assert!(server.is_usable());
    }

    #[test]
    fn turn_entry_without_credentials_is_unusable() {
        let server = IceServer {
            urls: vec!["turns:turn.nexus.example:5349?transport=tcp".to_owned()],
            username: None,
            credential: None,
            expires_at: None,
        };
        assert!(!server.is_usable(), "TURN sin credenciales no sirve");

        let server = IceServer {
            username: Some("u".into()),
            credential: Some("c".into()),
            ..server
        };
        assert!(server.is_usable());
    }

    #[test]
    fn empty_url_list_is_unusable() {
        let server = IceServer {
            urls: vec![],
            username: None,
            credential: None,
            expires_at: None,
        };
        assert!(!server.is_usable());
    }
}
