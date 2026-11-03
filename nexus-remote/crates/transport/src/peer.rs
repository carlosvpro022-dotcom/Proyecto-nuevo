//! La conexion WebRTC entre los dos dispositivos.
//!
//! Aqui viaja la sesion de verdad: el escritorio, el teclado, el raton, el
//! audio y los archivos. El servidor de senalizacion solo sirvio para que los
//! dos extremos se encontrasen; **el contenido no pasa por el**, y cuando hace
//! falta un relay TURN, este reenvia paquetes ya cifrados por DTLS sin poder
//! leerlos.
//!
//! # Quien ofrece
//!
//! Ofrece el **host**. Es el extremo que tiene algo que transmitir (la
//! pantalla, y en la etapa 5 el audio), de modo que es el que crea los canales
//! de datos y la oferta. El controlador contesta.
//!
//! # Canales de datos
//!
//! - `control`: ordenes de sesion, portapapeles, seleccion de monitor.
//! - `input`: raton y teclado.
//!
//! Los dos van **ordenados y fiables**. Para el raton se podria ganar algo de
//! latencia renunciando al orden, pero una pulsacion de teclado perdida o dos
//! clics intercambiados son errores que el usuario nota y no puede deshacer, de
//! modo que no es un intercambio que convenga. Si en la etapa 5 el movimiento
//! del raton resulta ser el cuello de botella, se puede anadir un canal aparte
//! sin orden solo para las coordenadas.
//!
//! El canal de archivos llega en la etapa 5.
//!
//! # Atadura al canal cifrado
//!
//! Esta capa expone las huellas DTLS local y remota
//! ([`PeerSession::local_fingerprint`] y [`PeerSession::remote_fingerprint`])
//! precisamente para que la capa de sesion pueda firmarlas y comprobarlas con
//! `nexus_security::binding`. Sin ese paso, un servidor de senalizacion
//! comprometido podria sustituir una huella y colocarse en medio.

use std::collections::HashMap;
use std::sync::Arc;

use bytes::BytesMut;
use nexus_security::DtlsFingerprint;
use tokio::sync::{Mutex, mpsc};
use webrtc::data_channel::{DataChannel, DataChannelEvent, RTCDataChannelInit};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCConfiguration, RTCConfigurationBuilder, RTCIceCandidateInit, RTCIceGatheringState,
    RTCIceServer, RTCIceTransportPolicy, RTCPeerConnectionIceEvent, RTCPeerConnectionState,
    RTCSessionDescription, Registry, register_default_interceptors,
};

use crate::error::{TransportError, TransportResult};
use crate::ice::{IceConfiguration, TransportPolicy};

/// Canal de ordenes de sesion.
pub const CONTROL_CHANNEL: &str = "control";
/// Canal de raton y teclado.
pub const INPUT_CHANNEL: &str = "input";

/// Eventos que caben en la cola antes de que el consumidor los lea.
const EVENT_QUEUE_DEPTH: usize = 256;

/// Papel del extremo en la negociacion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerRole {
    /// Crea los canales de datos y la oferta. Es el host.
    Offerer,
    /// Contesta a la oferta. Es el controlador.
    Answerer,
}

/// Estado del enlace cifrado entre los dos dispositivos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkPhase {
    /// Probando caminos.
    Connecting,
    /// Hay un camino y el canal cifrado esta en pie.
    Connected,
    /// Se perdio el camino; WebRTC sigue intentandolo.
    Disconnected,
    /// No hay forma de conectar.
    Failed,
    /// Cerrado.
    Closed,
}

/// Algo que ocurre en la conexion y que la capa de sesion tiene que atender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerEvent {
    /// Se descubrio un camino propio que hay que enviar al otro extremo.
    LocalCandidate {
        /// Candidato en formato SDP.
        candidate: String,
        /// Identificador del medio.
        sdp_mid: Option<String>,
    },
    /// Ya no habra mas candidatos propios.
    GatheringComplete,
    /// Cambio el estado del enlace.
    Phase(LinkPhase),
    /// Se abrio un canal de datos.
    ChannelOpen {
        /// Etiqueta del canal.
        label: String,
    },
    /// Se cerro un canal de datos.
    ChannelClosed {
        /// Etiqueta del canal.
        label: String,
    },
    /// Llegaron datos por un canal.
    Message {
        /// Etiqueta del canal.
        label: String,
        /// Contenido.
        data: Vec<u8>,
    },
}

type ChannelMap = Arc<Mutex<HashMap<String, Arc<dyn DataChannel>>>>;

/// Traduce la configuracion ICE propia a la de la biblioteca WebRTC.
///
/// Es una funcion aparte y pura para poder comprobar la traduccion sin abrir
/// ningun socket: es donde se decide si la conexion podra salir de la red local.
fn rtc_configuration(ice: &IceConfiguration) -> RTCConfiguration {
    RTCConfigurationBuilder::new()
        .with_ice_servers(rtc_ice_servers(ice))
        .with_ice_transport_policy(rtc_transport_policy(ice.policy))
        .build()
}

/// Traduce la lista de servidores ICE.
///
/// Esta separada de [`rtc_configuration`] porque `RTCConfiguration` no deja
/// leer sus campos: asi la traduccion, que es donde se decide si la conexion
/// podra salir de la red local, si se puede comprobar en una prueba.
fn rtc_ice_servers(ice: &IceConfiguration) -> Vec<RTCIceServer> {
    ice.servers
        .iter()
        .map(|server| RTCIceServer {
            urls: server.urls.clone(),
            // La biblioteca espera cadenas vacias, no opciones: un servidor
            // STUN no lleva credenciales.
            username: server.username.clone().unwrap_or_default(),
            credential: server.credential.clone().unwrap_or_default(),
        })
        .collect()
}

/// Traduce la politica de transporte.
const fn rtc_transport_policy(policy: TransportPolicy) -> RTCIceTransportPolicy {
    match policy {
        TransportPolicy::All => RTCIceTransportPolicy::All,
        TransportPolicy::RelayOnly => RTCIceTransportPolicy::Relay,
    }
}

/// Canal ordenado y fiable, que es lo que quieren `control` e `input`.
fn reliable_channel() -> RTCDataChannelInit {
    RTCDataChannelInit {
        ordered: true,
        max_packet_life_time: None,
        max_retransmits: None,
        ..Default::default()
    }
}

/// Recibe los eventos de la biblioteca y los pasa a nuestra cola.
#[derive(Clone)]
struct Bridge {
    events: mpsc::Sender<PeerEvent>,
    channels: ChannelMap,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Bridge {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        let Ok(init) = event.candidate.to_json() else {
            tracing::warn!("un candidato ICE propio no se pudo serializar");
            return;
        };
        let _ = self
            .events
            .send(PeerEvent::LocalCandidate {
                candidate: init.candidate,
                sdp_mid: init.sdp_mid,
            })
            .await;
    }

    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.events.send(PeerEvent::GatheringComplete).await;
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        let phase = match state {
            RTCPeerConnectionState::Connected => LinkPhase::Connected,
            RTCPeerConnectionState::Disconnected => LinkPhase::Disconnected,
            RTCPeerConnectionState::Failed => LinkPhase::Failed,
            RTCPeerConnectionState::Closed => LinkPhase::Closed,
            _ => LinkPhase::Connecting,
        };
        let _ = self.events.send(PeerEvent::Phase(phase)).await;
    }

    async fn on_data_channel(&self, channel: Arc<dyn DataChannel>) {
        register_channel(&self.channels, &self.events, channel).await;
    }
}

/// Apunta el canal en el mapa y lanza la tarea que escucha sus eventos.
async fn register_channel(
    channels: &ChannelMap,
    events: &mpsc::Sender<PeerEvent>,
    channel: Arc<dyn DataChannel>,
) {
    let label = match channel.label().await {
        Ok(label) => label,
        Err(error) => {
            tracing::warn!("un canal de datos sin etiqueta legible: {error}");
            return;
        }
    };

    channels
        .lock()
        .await
        .insert(label.clone(), Arc::clone(&channel));

    let events = events.clone();
    let channels = Arc::clone(channels);
    tokio::spawn(async move {
        while let Some(event) = channel.poll().await {
            let forwarded = match event {
                DataChannelEvent::OnOpen => PeerEvent::ChannelOpen {
                    label: label.clone(),
                },
                DataChannelEvent::OnClose => PeerEvent::ChannelClosed {
                    label: label.clone(),
                },
                DataChannelEvent::OnMessage(message) => PeerEvent::Message {
                    label: label.clone(),
                    data: message.data.to_vec(),
                },
                _ => continue,
            };

            let closing = matches!(forwarded, PeerEvent::ChannelClosed { .. });
            if events.send(forwarded).await.is_err() || closing {
                break;
            }
        }
        channels.lock().await.remove(&label);
    });
}

/// Una conexion WebRTC con el otro dispositivo.
pub struct PeerSession {
    connection: Arc<dyn PeerConnection>,
    channels: ChannelMap,
    events: mpsc::Receiver<PeerEvent>,
}

impl std::fmt::Debug for PeerSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerSession").finish_non_exhaustive()
    }
}

impl PeerSession {
    /// Abre una conexion con la configuracion ICE recibida del servidor.
    ///
    /// La configuracion se valida antes de usarla: una sin STUN ni TURN solo
    /// funcionaria dentro de la misma red, que es exactamente el fallo que este
    /// proyecto no debe tener.
    ///
    /// # Errors
    ///
    /// - [`TransportError::InvalidIceConfiguration`] si la configuracion no
    ///   sirve para conectar.
    /// - [`TransportError::Peer`] si la pila WebRTC no arranca.
    pub async fn open(ice: &IceConfiguration, role: PeerRole) -> TransportResult<Self> {
        // El futuro de la apertura mide unos 20 KB: la pila de WebRTC arrastra
        // el motor de medios, el registro de interceptores y la configuracion
        // completa. Se reserva en el monton una vez, al abrir, para que ese
        // tamano no se propague a quien llame desde un `select!` o lo meta en
        // una tarea.
        Box::pin(Self::open_inner(ice, role)).await
    }

    async fn open_inner(ice: &IceConfiguration, role: PeerRole) -> TransportResult<Self> {
        ice.validate()?;

        let (sender, events) = mpsc::channel(EVENT_QUEUE_DEPTH);
        let channels: ChannelMap = Arc::new(Mutex::new(HashMap::new()));
        let bridge = Bridge {
            events: sender.clone(),
            channels: Arc::clone(&channels),
        };

        // El motor de medios no hace falta para los canales de datos, pero se
        // registra ya para que la etapa 5 pueda anadir video y audio sin tocar
        // esta funcion.
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .map_err(|e| TransportError::Peer(e.to_string()))?;
        let registry = register_default_interceptors(Registry::new(), &mut media_engine)
            .map_err(|e| TransportError::Peer(e.to_string()))?;

        let builder = PeerConnectionBuilder::new()
            .with_configuration(rtc_configuration(ice))
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_handler(Arc::new(bridge))
            // Puerto efimero en todas las interfaces: los caminos utiles los
            // descubre ICE.
            .with_udp_addrs(vec!["0.0.0.0:0".to_owned()]);

        let connection = Box::pin(builder.build())
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))?;
        let connection: Arc<dyn PeerConnection> = Arc::new(connection);

        if role == PeerRole::Offerer {
            for label in [CONTROL_CHANNEL, INPUT_CHANNEL] {
                let channel = connection
                    .create_data_channel(label, Some(reliable_channel()))
                    .await
                    .map_err(|e| TransportError::Peer(e.to_string()))?;
                register_channel(&channels, &sender, channel).await;
            }
        }

        Ok(Self {
            connection,
            channels,
            events,
        })
    }

    /// Crea la oferta y la fija como descripcion local.
    ///
    /// # Errors
    ///
    /// [`TransportError::Peer`] si la pila WebRTC falla.
    pub async fn create_offer(&self) -> TransportResult<String> {
        let offer = self
            .connection
            .create_offer(None)
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))?;
        let sdp = offer.sdp.clone();
        self.connection
            .set_local_description(offer)
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))?;
        Ok(sdp)
    }

    /// Acepta la oferta del otro extremo y devuelve la respuesta.
    ///
    /// # Errors
    ///
    /// [`TransportError::Peer`] si el SDP no es valido o la pila falla.
    pub async fn accept_offer(&self, sdp: &str) -> TransportResult<String> {
        let offer = RTCSessionDescription::offer(sdp.to_owned())
            .map_err(|e| TransportError::Peer(e.to_string()))?;
        self.connection
            .set_remote_description(offer)
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))?;

        let answer = self
            .connection
            .create_answer(None)
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))?;
        let sdp = answer.sdp.clone();
        self.connection
            .set_local_description(answer)
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))?;
        Ok(sdp)
    }

    /// Aplica la respuesta del otro extremo.
    ///
    /// # Errors
    ///
    /// [`TransportError::Peer`] si el SDP no es valido o la pila falla.
    pub async fn accept_answer(&self, sdp: &str) -> TransportResult<()> {
        let answer = RTCSessionDescription::answer(sdp.to_owned())
            .map_err(|e| TransportError::Peer(e.to_string()))?;
        self.connection
            .set_remote_description(answer)
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))
    }

    /// Anade un camino que descubrio el otro extremo.
    ///
    /// # Errors
    ///
    /// [`TransportError::Peer`] si el candidato no es valido.
    pub async fn add_remote_candidate(
        &self,
        candidate: String,
        sdp_mid: Option<String>,
    ) -> TransportResult<()> {
        self.connection
            .add_ice_candidate(RTCIceCandidateInit {
                candidate,
                sdp_mid,
                ..Default::default()
            })
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))
    }

    /// Huella del certificado DTLS propio, leida de la descripcion local.
    ///
    /// # Errors
    ///
    /// [`TransportError::Peer`] si todavia no hay descripcion local, o
    /// [`TransportError::Security`] si la descripcion no declara una huella
    /// utilizable.
    pub async fn local_fingerprint(&self) -> TransportResult<DtlsFingerprint> {
        let description =
            self.connection.local_description().await.ok_or_else(|| {
                TransportError::Peer("todavia no hay descripcion local".to_owned())
            })?;
        DtlsFingerprint::from_sdp(&description.sdp)
            .map_err(|e| TransportError::Security(e.to_string()))
    }

    /// Huella del certificado DTLS del otro extremo.
    ///
    /// Es la que hay que comprobar contra la que el otro extremo firmo: si no
    /// coincide, alguien sustituyo la huella en la senalizacion.
    ///
    /// # Errors
    ///
    /// [`TransportError::Peer`] si todavia no hay descripcion remota, o
    /// [`TransportError::Security`] si la descripcion no declara una huella
    /// utilizable o declara varias distintas.
    pub async fn remote_fingerprint(&self) -> TransportResult<DtlsFingerprint> {
        let description =
            self.connection.remote_description().await.ok_or_else(|| {
                TransportError::Peer("todavia no hay descripcion remota".to_owned())
            })?;
        DtlsFingerprint::from_sdp(&description.sdp)
            .map_err(|e| TransportError::Security(e.to_string()))
    }

    /// Envia datos por un canal.
    ///
    /// # Errors
    ///
    /// - [`TransportError::ChannelUnavailable`] si el canal no esta abierto.
    /// - [`TransportError::Peer`] si el envio falla.
    pub async fn send(&self, label: &str, data: &[u8]) -> TransportResult<()> {
        let channel = self
            .channels
            .lock()
            .await
            .get(label)
            .map(Arc::clone)
            .ok_or_else(|| TransportError::ChannelUnavailable(label.to_owned()))?;

        channel
            .send(BytesMut::from(data))
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))
    }

    /// Etiquetas de los canales de datos registrados.
    ///
    /// Registrado no es lo mismo que abierto: un canal aparece aqui en cuanto
    /// se crea, y queda utilizable cuando llega
    /// [`PeerEvent::ChannelOpen`]. Enviar antes de eso falla.
    pub async fn channel_labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = self.channels.lock().await.keys().cloned().collect();
        labels.sort();
        labels
    }

    /// Espera el siguiente evento de la conexion.
    ///
    /// Devuelve `None` cuando la conexion se cerro y no habra mas eventos.
    pub async fn next_event(&mut self) -> Option<PeerEvent> {
        self.events.recv().await
    }

    /// Cierra la conexion.
    ///
    /// # Errors
    ///
    /// [`TransportError::Peer`] si la pila WebRTC falla al cerrar.
    pub async fn close(&self) -> TransportResult<()> {
        self.connection
            .close()
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::Timestamp;
    use nexus_core::protocol::signaling::IceServer;

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
            urls: vec!["turn:turn.nexus.example:3478?transport=udp".to_owned()],
            username: Some("1730000000:nexus".to_owned()),
            credential: Some("credencial-temporal".to_owned()),
            expires_at: Some(Timestamp(1_730_000_000_000)),
        }
    }

    #[test]
    fn the_ice_servers_reach_the_webrtc_configuration() {
        // Si esta traduccion se perdiese, la conexion funcionaria en la red
        // local y fallaria desde Internet, que es el fallo mas dificil de ver.
        let servers = rtc_ice_servers(&IceConfiguration::new(vec![stun(), turn()]));
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].urls, stun().urls);
        assert_eq!(servers[1].username, "1730000000:nexus");
        assert_eq!(servers[1].credential, "credencial-temporal");
    }

    #[test]
    fn a_stun_server_carries_empty_credentials() {
        let servers = rtc_ice_servers(&IceConfiguration::new(vec![stun()]));
        assert!(servers[0].username.is_empty());
        assert!(servers[0].credential.is_empty());
    }

    #[test]
    fn the_default_policy_allows_direct_connections() {
        assert_eq!(
            rtc_transport_policy(TransportPolicy::All),
            RTCIceTransportPolicy::All
        );
    }

    #[test]
    fn forcing_relay_reaches_the_webrtc_configuration() {
        assert_eq!(
            rtc_transport_policy(TransportPolicy::RelayOnly),
            RTCIceTransportPolicy::Relay
        );
    }

    #[test]
    fn the_configuration_is_built_without_panicking() {
        // `RTCConfiguration` no deja leer sus campos, asi que lo que se
        // comprueba aqui es solo que la construccion completa no falla.
        let _ = rtc_configuration(&IceConfiguration::new(vec![stun(), turn()]));
    }

    #[test]
    fn the_data_channels_are_ordered_and_reliable() {
        let init = reliable_channel();
        assert!(init.ordered, "dos clics no pueden llegar al reves");
        assert_eq!(
            init.max_retransmits, None,
            "una pulsacion de teclado no puede perderse"
        );
        assert_eq!(init.max_packet_life_time, None);
    }

    #[tokio::test]
    async fn an_unusable_ice_configuration_is_refused_before_opening_anything() {
        let local_only = IceServer {
            urls: vec!["udp:192.168.1.10:9000".to_owned()],
            username: None,
            credential: None,
            expires_at: None,
        };
        let error = PeerSession::open(
            &IceConfiguration {
                servers: vec![local_only],
                policy: TransportPolicy::RelayOnly,
            },
            PeerRole::Offerer,
        )
        .await
        .expect_err("una configuracion sin relay no debe abrir nada");
        assert!(
            matches!(error, TransportError::InvalidIceConfiguration(_)),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn an_empty_ice_configuration_is_refused() {
        let error = PeerSession::open(&IceConfiguration::new(vec![]), PeerRole::Offerer)
            .await
            .expect_err("sin servidores ICE no se puede conectar");
        assert!(
            matches!(error, TransportError::InvalidIceConfiguration(_)),
            "{error:?}"
        );
    }
}
