//! Cliente del canal de senalizacion.
//!
//! Abre el WebSocket contra el servidor propio, demuestra la identidad del
//! dispositivo firmando el reto que emite el servidor, y despues deja pasar los
//! mensajes de senalizacion en las dos direcciones.
//!
//! Lo usa el host de Windows. La aplicacion de Apple hablara el mismo protocolo
//! con su propia implementacion en Swift: lo que define el contrato es
//! `nexus_core::protocol::signaling`, no este codigo.
//!
//! # Lo que este cliente comprueba por su cuenta
//!
//! - **Que la conexion va cifrada.** Se exige `wss://` salvo contra la propia
//!   maquina, donde `ws://` sirve para pruebas sin montar certificados.
//! - **Que el reloj esta en hora.** El servidor rechaza firmas con marca
//!   temporal desviada, y ese rechazo llega como "credenciales no validas", que
//!   es un mensaje desconcertante. Si el desvio es la causa, el cliente lo dice
//!   con claridad antes de intentarlo.
//!
//! # Lo que no hace
//!
//! **No se reconecta solo.** Una conexion caida se nota en
//! [`SignalingClient::recv`], que devuelve [`TransportError::Network`], y
//! reabrirla es trabajo de quien lo use. La politica de espera creciente que
//! conviene aplicar esta en [`SignalingLink`](crate::link::SignalingLink), ya
//! escrita y probada, pero nada la pone en marcha todavia: el bucle que
//! mantiene una sesion viva llega con la transmision de pantalla.
//!
//! # Lo que no autentica
//!
//! Al servidor lo autentica TLS, no una firma. La autenticacion mutua que
//! importa es la del **otro dispositivo**, y ocurre despues, sobre las huellas
//! DTLS de la sesion (`nexus_security::binding`).

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nexus_core::device::DeviceRole;
use nexus_core::protocol::Envelope;
use nexus_core::protocol::signaling::{ClientToServer, ServerToClient};
use nexus_core::{CapabilitySet, DeviceInfo, SessionId, Timestamp};
use nexus_security::replay::DEFAULT_CLOCK_TOLERANCE;
use nexus_security::{AuthRole, AuthTranscript, DeviceIdentity, Nonce};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::codec::{decode_server_message, encode_client_message};
use crate::error::{TransportError, TransportResult};
use crate::ice::IceConfiguration;

/// Tiempo maximo que se espera una respuesta del servidor durante el saludo.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Datos necesarios para abrir el canal.
#[derive(Debug, Clone)]
pub struct ConnectRequest<'a> {
    /// URL del servidor (`wss://host/v1/signaling`).
    pub url: &'a str,
    /// Datos declarados del dispositivo.
    pub device: &'a DeviceInfo,
    /// Capacidades que anuncia.
    pub capabilities: &'a CapabilitySet,
    /// Testigo de alta, solo en la primera conexion de un host.
    pub enrollment_token: Option<&'a str>,
}

/// Resultado de una autenticacion correcta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    /// Identificador que el servidor asigna a esta conexion.
    pub signaling_session: SessionId,
    /// Configuracion ICE con la que abrir la conexion WebRTC.
    pub ice: IceConfiguration,
}

/// Conexion autenticada con el servidor de senalizacion.
pub struct SignalingClient {
    socket: Socket,
}

impl std::fmt::Debug for SignalingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalingClient").finish_non_exhaustive()
    }
}

impl SignalingClient {
    /// Abre el canal y completa la autenticacion.
    ///
    /// # Errors
    ///
    /// - [`TransportError::InsecureUrl`] si la URL no va cifrada y no apunta a
    ///   la propia maquina.
    /// - [`TransportError::Network`] si el socket no se puede abrir o se cae.
    /// - [`TransportError::Security`] si el reloj local esta demasiado
    ///   desviado del servidor.
    /// - [`TransportError::AuthRejected`] si el servidor rechaza la identidad.
    /// - [`TransportError::UnexpectedMessage`] si el servidor contesta algo
    ///   fuera de orden.
    pub async fn connect(
        request: ConnectRequest<'_>,
        identity: &DeviceIdentity,
    ) -> TransportResult<(Self, Accepted)> {
        check_transport_security(request.url)?;

        let (socket, _response) = connect_async(request.url)
            .await
            .map_err(|e| TransportError::Network(e.to_string()))?;
        let mut client = Self { socket };

        client
            .send(ClientToServer::Hello {
                device: request.device.clone(),
                capabilities: request.capabilities.clone(),
                public_key_hex: identity.public_key().to_hex(),
                enrollment_token: request.enrollment_token.map(str::to_owned),
            })
            .await?;

        let (nonce_hex, server_time) = match client.recv_handshake().await? {
            ServerToClient::Challenge {
                nonce_hex,
                server_time,
            } => (nonce_hex, server_time),
            other => return Err(unexpected("challenge", &other)),
        };

        let now = Timestamp::now();
        check_clock(server_time, now)?;

        let server_nonce =
            Nonce::from_hex(&nonce_hex).map_err(|e| TransportError::Security(e.to_string()))?;
        let client_nonce = Nonce::generate();
        let transcript = AuthTranscript::new(
            server_nonce,
            client_nonce,
            request.device.id,
            auth_role(request.device.role),
            now,
        );

        // La marca temporal del sobre tiene que ser la misma que se firmo: el
        // servidor verifica la firma sobre la marca del sobre.
        client
            .send_at(
                ClientToServer::AuthProof {
                    device_id: request.device.id,
                    client_nonce_hex: client_nonce.to_hex(),
                    signature_hex: identity.sign_hex(&transcript.to_bytes()),
                },
                now,
            )
            .await?;

        match client.recv_handshake().await? {
            ServerToClient::AuthAccepted {
                session_id,
                ice_servers,
            } => {
                let ice = IceConfiguration::new(ice_servers);
                // Una configuracion ICE inservible se detecta aqui y no al
                // intentar conectar: el mensaje es mucho mas claro.
                ice.validate()?;
                Ok((
                    client,
                    Accepted {
                        signaling_session: session_id,
                        ice,
                    },
                ))
            }
            other => Err(unexpected("auth_accepted", &other)),
        }
    }

    /// Declara el dispositivo como host disponible y espera la confirmacion.
    ///
    /// # Errors
    ///
    /// Igual que [`Self::send`], mas [`TransportError::ServerError`] si el
    /// servidor se niega y [`TransportError::UnexpectedMessage`] si contesta
    /// otra cosa.
    pub async fn register_host(&mut self, monitor_count: u8) -> TransportResult<()> {
        self.send(ClientToServer::RegisterHost { monitor_count })
            .await?;
        match self.recv_handshake().await? {
            ServerToClient::HostRegistered { .. } => Ok(()),
            other => Err(unexpected("host_registered", &other)),
        }
    }

    /// Envia un mensaje con la marca temporal actual.
    ///
    /// # Errors
    ///
    /// - [`TransportError::MessageTooLarge`] o [`TransportError::Malformed`] si
    ///   el mensaje no se puede codificar.
    /// - [`TransportError::Network`] si el socket falla.
    pub async fn send(&mut self, payload: ClientToServer) -> TransportResult<()> {
        self.send_at(payload, Timestamp::now()).await
    }

    /// Envia un mensaje con una marca temporal concreta.
    ///
    /// # Errors
    ///
    /// Igual que [`Self::send`].
    pub async fn send_at(&mut self, payload: ClientToServer, at: Timestamp) -> TransportResult<()> {
        let raw = encode_client_message(&Envelope::at(at, payload))?;
        self.socket
            .send(WsMessage::Text(raw.into()))
            .await
            .map_err(|e| TransportError::Network(e.to_string()))
    }

    /// Espera el siguiente mensaje del servidor.
    ///
    /// Los marcos de control del WebSocket (`ping`, `pong`) se atienden sin
    /// llegar al llamante.
    ///
    /// # Errors
    ///
    /// - [`TransportError::Network`] si el socket falla o se cierra.
    /// - [`TransportError::Malformed`] si el mensaje no cumple el protocolo.
    pub async fn recv(&mut self) -> TransportResult<ServerToClient> {
        loop {
            let frame = self
                .socket
                .next()
                .await
                .ok_or_else(|| TransportError::Network("el servidor cerro el canal".to_owned()))?
                .map_err(|e| TransportError::Network(e.to_string()))?;

            match frame {
                WsMessage::Text(text) => {
                    return Ok(decode_server_message(&text)?.payload);
                }
                WsMessage::Close(_) => {
                    return Err(TransportError::Network(
                        "el servidor cerro el canal".to_owned(),
                    ));
                }
                WsMessage::Binary(_) => {
                    return Err(TransportError::Malformed(
                        "el servidor envio datos binarios: el protocolo es texto".to_owned(),
                    ));
                }
                WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_) => {}
            }
        }
    }

    /// Espera el siguiente mensaje con un tope de tiempo.
    ///
    /// # Errors
    ///
    /// Igual que [`Self::recv`], y [`TransportError::Network`] si se agota la
    /// espera.
    pub async fn recv_timeout(&mut self, limit: Duration) -> TransportResult<ServerToClient> {
        tokio::time::timeout(limit, self.recv())
            .await
            .map_err(|_| TransportError::Network("el servidor no contesto a tiempo".to_owned()))?
    }

    /// Cierra el canal con cortesia.
    ///
    /// # Errors
    ///
    /// [`TransportError::Network`] si el cierre falla.
    pub async fn close(mut self) -> TransportResult<()> {
        self.socket
            .close(None)
            .await
            .map_err(|e| TransportError::Network(e.to_string()))
    }

    /// Espera un mensaje durante el saludo, traduciendo los finales.
    ///
    /// Un rechazo de autenticacion puede llegar en cualquier punto del saludo:
    /// el servidor lo envia en cuanto decide que no, sin esperar a la firma. Se
    /// traduce aqui para que el llamante reciba
    /// [`TransportError::AuthRejected`] y no un confuso "mensaje inesperado".
    async fn recv_handshake(&mut self) -> TransportResult<ServerToClient> {
        match self.recv_timeout(HANDSHAKE_TIMEOUT).await? {
            ServerToClient::AuthRejected {
                reason,
                retry_after_secs,
            } => Err(TransportError::AuthRejected {
                reason,
                retry_after_secs,
            }),
            ServerToClient::Error { code, message } => {
                Err(TransportError::ServerError { code, message })
            }
            other => Ok(other),
        }
    }
}

const fn auth_role(role: DeviceRole) -> AuthRole {
    match role {
        DeviceRole::Host => AuthRole::Host,
        DeviceRole::Controller => AuthRole::Controller,
    }
}

/// Nombre del mensaje recibido, para poder explicar el desajuste.
const fn message_name(message: &ServerToClient) -> &'static str {
    match message {
        ServerToClient::Challenge { .. } => "challenge",
        ServerToClient::AuthAccepted { .. } => "auth_accepted",
        ServerToClient::AuthRejected { .. } => "auth_rejected",
        ServerToClient::HostRegistered { .. } => "host_registered",
        ServerToClient::SessionRequested { .. } => "session_requested",
        ServerToClient::SessionAccepted { .. } => "session_accepted",
        ServerToClient::SessionRejected { .. } => "session_rejected",
        ServerToClient::Offer { .. } => "offer",
        ServerToClient::Answer { .. } => "answer",
        ServerToClient::IceCandidate { .. } => "ice_candidate",
        ServerToClient::PeerAuth { .. } => "peer_auth",
        ServerToClient::PeerGone { .. } => "peer_gone",
        ServerToClient::PairingCompleted { .. } => "pairing_completed",
        ServerToClient::Error { .. } => "error",
        ServerToClient::Pong => "pong",
    }
}

fn unexpected(expected: &'static str, received: &ServerToClient) -> TransportError {
    TransportError::UnexpectedMessage {
        expected,
        received: message_name(received),
    }
}

/// Comprueba que la conexion ira cifrada.
///
/// `wss://` siempre vale. `ws://` solo contra la propia maquina, donde no hay
/// red que escuchar y montar un certificado para una prueba no aporta nada.
/// Cualquier otro caso se rechaza: una conexion de control remoto en claro
/// expondria el teclado y la pantalla a quien comparta la red.
///
/// # Errors
///
/// [`TransportError::InsecureUrl`] si la URL no cumple lo anterior.
pub fn check_transport_security(url: &str) -> TransportResult<()> {
    if url.starts_with("wss://") {
        return Ok(());
    }

    let Some(rest) = url.strip_prefix("ws://") else {
        return Err(TransportError::InsecureUrl(format!(
            "esquema no admitido en {url}"
        )));
    };

    let host = host_of(rest);
    if matches!(host, "localhost" | "127.0.0.1" | "::1") {
        Ok(())
    } else {
        Err(TransportError::InsecureUrl(format!(
            "ws:// solo se admite contra la propia maquina, y {host} no lo es"
        )))
    }
}

/// Extrae el nombre de maquina de lo que sigue a `ws://`.
///
/// Se tratan aparte las direcciones IPv6 entre corchetes, porque llevan dos
/// puntos dentro y separar por el ultimo dejaria `[:` como nombre.
fn host_of(rest: &str) -> &str {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();

    if let Some(end) = authority.strip_prefix('[').and_then(|a| a.find(']')) {
        return &authority[1..=end];
    }
    authority
        .rsplit_once(':')
        .map_or(authority, |(host, _port)| host)
}

/// Comprueba que el reloj local concuerda con el del servidor.
///
/// # Errors
///
/// [`TransportError::Security`] si el desvio supera la tolerancia con que el
/// servidor acepta firmas.
pub fn check_clock(server_time: Timestamp, now: Timestamp) -> TransportResult<()> {
    let skew_ms = server_time.0.abs_diff(now.0);
    if skew_ms > DEFAULT_CLOCK_TOLERANCE.as_millis() as u64 {
        return Err(TransportError::Security(format!(
            "el reloj de este dispositivo esta {} s desviado del servidor; \
             el servidor rechazara la firma hasta que se ponga en hora",
            skew_ms / 1_000
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_encrypted_url_is_accepted() {
        for url in [
            "wss://nexus.example/v1/signaling",
            "wss://nexus.example:8443/v1/signaling",
            "wss://198.51.100.10/v1/signaling",
        ] {
            check_transport_security(url).unwrap_or_else(|e| panic!("{url} deberia valer: {e}"));
        }
    }

    #[test]
    fn plain_websocket_to_the_local_machine_is_accepted() {
        // Es lo que usan las pruebas: no hay red donde escuchar.
        for url in [
            "ws://127.0.0.1:9000/v1/signaling",
            "ws://localhost:9000/v1/signaling",
            "ws://[::1]:9000/v1/signaling",
            "ws://[::1]/v1/signaling",
            "ws://127.0.0.1/v1/signaling",
        ] {
            check_transport_security(url).unwrap_or_else(|e| panic!("{url} deberia valer: {e}"));
        }
    }

    #[test]
    fn plain_websocket_to_anywhere_else_is_refused() {
        // Control remoto en claro expondria el teclado a toda la red.
        for url in [
            "ws://nexus.example/v1/signaling",
            "ws://192.168.1.50:9000/v1/signaling",
            "ws://198.51.100.10/v1/signaling",
            // El truco del nombre que contiene "localhost" pero no lo es.
            "ws://localhost.atacante.example/v1/signaling",
            "ws://127.0.0.1.atacante.example/v1/signaling",
            "ws://[2001:db8::1]:9000/v1/signaling",
        ] {
            let error =
                check_transport_security(url).expect_err(&format!("{url} no deberia valer"));
            assert!(matches!(error, TransportError::InsecureUrl(_)), "{error:?}");
        }
    }

    #[test]
    fn other_schemes_are_refused() {
        for url in [
            "http://nexus.example",
            "https://nexus.example",
            "nexus.example",
            "",
            "file:///etc/passwd",
        ] {
            assert!(
                check_transport_security(url).is_err(),
                "{url} no deberia valer"
            );
        }
    }

    #[test]
    fn a_clock_in_agreement_is_accepted() {
        let now = Timestamp(1_730_000_000_000);
        check_clock(now, now).expect("en hora");
        check_clock(Timestamp(now.0 + 5_000), now).expect("cinco segundos cabe");
        check_clock(Timestamp(now.0 - 5_000), now).expect("hacia atras tambien");
    }

    #[test]
    fn a_skewed_clock_is_reported_clearly() {
        // Sin esta comprobacion el usuario veria "credenciales no validas" y no
        // tendria forma de saber que su reloj esta mal.
        let now = Timestamp(1_730_000_000_000);
        let error = check_clock(Timestamp(now.0 + 10 * 60 * 1_000), now)
            .expect_err("diez minutos es demasiado");
        match error {
            TransportError::Security(message) => {
                assert!(message.contains("reloj"), "{message}");
                assert!(message.contains("600"), "{message}");
            }
            other => panic!("se esperaba un fallo de reloj, llego {other:?}"),
        }
    }

    #[test]
    fn the_clock_tolerance_matches_what_the_server_accepts() {
        // Si este cliente fuese mas permisivo que el servidor, dejaria pasar
        // firmas que el servidor va a rechazar.
        let now = Timestamp(1_730_000_000_000);
        let limit = DEFAULT_CLOCK_TOLERANCE.as_millis() as u64;
        check_clock(Timestamp(now.0 + limit), now).expect("el limite cabe");
        assert!(check_clock(Timestamp(now.0 + limit + 1), now).is_err());
    }

    #[test]
    fn every_server_message_has_a_name() {
        // El nombre se usa para explicar un desajuste de protocolo; si alguna
        // variante se quedase sin nombre, el mensaje de error seria inutil.
        use nexus_core::protocol::signaling::AuthRejection;
        let names = [
            message_name(&ServerToClient::Pong),
            message_name(&ServerToClient::AuthRejected {
                reason: AuthRejection::Revoked,
                retry_after_secs: None,
            }),
            message_name(&ServerToClient::Error {
                code: "x".to_owned(),
                message: "y".to_owned(),
            }),
        ];
        assert_eq!(names, ["pong", "auth_rejected", "error"]);
    }
}
