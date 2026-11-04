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
    RTCConfiguration, RTCConfigurationBuilder, RTCIceCandidateInit, RTCIceCandidateType,
    RTCIceGatheringState, RTCIceServer, RTCIceTransportPolicy, RTCPeerConnectionIceEvent,
    RTCPeerConnectionState, RTCSessionDescription, Registry, SettingEngineBuilder,
    register_default_interceptors,
};

use crate::error::{TransportError, TransportResult};
use crate::ice::{IceConfiguration, TransportPolicy};

/// Canal de ordenes de sesion.
pub const CONTROL_CHANNEL: &str = "control";
/// Canal de raton y teclado.
pub const INPUT_CHANNEL: &str = "input";
/// Canal de los bytes de los archivos.
///
/// Va aparte del de control por dos razones. Los bytes van en binario, sin
/// codificar en JSON, que inflaria el tamano un tercio. Y el canal es ordenado
/// y fiable: una descarga de dos gigas en el canal de control dejaria el raton
/// congelado detras de ella hasta terminar.
pub const FILES_CHANNEL: &str = "files";

/// Tope de bytes que un canal puede tener pendientes de salir.
///
/// La biblioteca viene con este limite en `usize::MAX`, es decir sin limite: el
/// envio nunca espera y lo que no quepa en la red se acumula en memoria. Con
/// ordenes pequenas no se nota, pero al enviar un archivo de varios gigas por
/// el canal se acumularia el archivo entero en la memoria del host.
///
/// Con un tope, `send` espera a que haya sitio en lugar de acumular, lo que
/// convierte la velocidad de la red en la velocidad a la que se lee del disco.
/// Cuatro mebibytes son muchos trozos de 64 KiB en vuelo: de sobra para llenar
/// un enlace rapido sin que la memoria dependa del tamano del archivo.
pub const SEND_BUFFER_LIMIT_BYTES: usize = 4 * 1024 * 1024;

/// Eventos que caben en la cola antes de que el consumidor los lea.
const EVENT_QUEUE_DEPTH: usize = 256;

/// Como se expone este extremo a la red.
///
/// Los valores por omision sirven para un equipo que sale a Internet por un
/// servidor propio con STUN y TURN. Hacen falta otros cuando el equipo esta en
/// casa, detras de un router, y se quiere alcanzar desde fuera abriendo un
/// puerto: ahi el puerto no puede ser al azar, y el equipo tiene que anunciar
/// su direccion publica en lugar de la privada.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkOptions {
    /// Puerto UDP local, o `0` para que lo elija el sistema.
    ///
    /// Un puerto al azar no se puede abrir en un router: no se sabe cual sera.
    /// Fijarlo es lo que permite el reenvio de puertos.
    ///
    /// Con un puerto fijo solo cabe una conexion a la vez, que es justo lo que
    /// el host admite de todos modos: un escritorio, una sesion.
    pub udp_port: u16,

    /// Direccion publica que anunciar en lugar de la local.
    ///
    /// Cuando el equipo esta detras de un router con el puerto abierto, la
    /// direccion que ve de si mismo es privada y no le sirve de nada al otro
    /// extremo. Declarando aqui la publica se anuncia la correcta sin
    /// necesidad de preguntarsela a un servidor STUN.
    ///
    /// Si la direccion cambia (una IP domestica cambia), hay que volver a
    /// abrir la conexion con la nueva.
    pub public_ip: Option<String>,
}

impl NetworkOptions {
    /// Indica si hay algo que configurar mas alla de lo normal.
    #[must_use]
    pub const fn is_default(&self) -> bool {
        self.udp_port == 0 && self.public_ip.is_none()
    }

    /// Direccion de escucha que corresponde a estas opciones.
    #[must_use]
    pub fn bind_address(&self) -> String {
        format!("0.0.0.0:{}", self.udp_port)
    }
}

/// Averigua la direccion IP que hay que anunciar.
///
/// Admite tanto una IP como un nombre de dominio. Lo segundo es lo normal en
/// una casa: la IP que da el operador cambia, y lo que se mantiene es el nombre
/// de un DNS dinamico. Resolviendolo cada vez que se abre una sesion, un cambio
/// de IP se recoge solo en la siguiente conexion.
///
/// # Errors
///
/// [`TransportError::InvalidIceConfiguration`] si el nombre no resuelve a
/// ninguna direccion IPv4.
pub async fn resolve_public_ip(endpoint: &str) -> TransportResult<String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(TransportError::InvalidIceConfiguration(
            "la direccion publica esta vacia".to_owned(),
        ));
    }

    if let Ok(direccion) = endpoint.parse::<std::net::IpAddr>() {
        return Ok(direccion.to_string());
    }

    // El puerto da igual: solo se usa para poder resolver el nombre.
    let mut candidatos = tokio::net::lookup_host(format!("{endpoint}:443"))
        .await
        .map_err(|e| {
            TransportError::InvalidIceConfiguration(format!("no se pudo resolver {endpoint}: {e}"))
        })?;

    // Se busca IPv4 a proposito: es la que lleva el reenvio de puertos de un
    // router domestico. Una IPv6 no necesita reenvio y se descubre sola.
    candidatos
        .find_map(|socket| match socket.ip() {
            std::net::IpAddr::V4(v4) => Some(v4.to_string()),
            std::net::IpAddr::V6(_) => None,
        })
        .ok_or_else(|| {
            TransportError::InvalidIceConfiguration(format!(
                "{endpoint} no resuelve a ninguna direccion IPv4"
            ))
        })
}

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

/// Extremo de envio de una conexion, clonable y independiente del bucle de
/// eventos.
///
/// Se obtiene con [`PeerSession::sender`]. Enviar por un canal no necesita
/// acceso exclusivo a la conexion, asi que varias tareas pueden tener su propio
/// remitente: el bucle de control por un lado y el envio de archivos por otro.
#[derive(Clone)]
pub struct PeerSender {
    channels: ChannelMap,
}

impl std::fmt::Debug for PeerSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerSender").finish_non_exhaustive()
    }
}

impl PeerSender {
    /// Envia datos por un canal.
    ///
    /// Espera si el canal ya tiene demasiados bytes pendientes de salir (ver
    /// [`SEND_BUFFER_LIMIT_BYTES`]). Esa espera es la que evita que un archivo
    /// grande se acumule en memoria: quien lee del disco avanza al ritmo al que
    /// la red es capaz de vaciar.
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

    /// Indica si un canal esta registrado.
    pub async fn has_channel(&self, label: &str) -> bool {
        self.channels.lock().await.contains_key(label)
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
        Self::open_with(ice, role, &NetworkOptions::default()).await
    }

    /// Abre la conexion indicando como se expone este extremo a la red.
    ///
    /// Es lo que hace falta cuando el equipo esta en casa detras de un router:
    /// un puerto fijo que se pueda abrir y la direccion publica que anunciar.
    ///
    /// # Errors
    ///
    /// Igual que [`Self::open`].
    pub async fn open_with(
        ice: &IceConfiguration,
        role: PeerRole,
        network: &NetworkOptions,
    ) -> TransportResult<Self> {
        // El futuro de la apertura mide unos 20 KB: la pila de WebRTC arrastra
        // el motor de medios, el registro de interceptores y la configuracion
        // completa. Se reserva en el monton una vez, al abrir, para que ese
        // tamano no se propague a quien llame desde un `select!` o lo meta en
        // una tarea.
        Box::pin(Self::open_inner(ice, role, network)).await
    }

    async fn open_inner(
        ice: &IceConfiguration,
        role: PeerRole,
        network: &NetworkOptions,
    ) -> TransportResult<Self> {
        ice.validate()?;

        // Aqui si se puede decidir: una conexion llega a otra red si hay
        // descubrimiento o retransmision, **o** si este equipo anuncia una
        // direccion publica propia porque tiene un puerto abierto. Sin
        // ninguna de las dos cosas solo funcionaria dentro de la misma red, y
        // eso es justo el fallo que este proyecto no debe tener.
        if !ice.supports_remote_networks() && network.public_ip.is_none() {
            return Err(TransportError::InvalidIceConfiguration(
                "sin servidores STUN o TURN, y sin una direccion publica propia, \
                 la conexion solo funcionaria dentro de la misma red"
                    .to_owned(),
            ));
        }

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

        let mut settings = SettingEngineBuilder::new();
        if let Some(ip) = &network.public_ip {
            // Se anuncia como candidato "host" y no "srflx" porque desde fuera
            // esa direccion es alcanzable directamente: hay un puerto abierto
            // que lleva hasta aqui.
            settings = settings.with_nat_1to1_ips(vec![ip.clone()], RTCIceCandidateType::Host);
        }

        let builder = PeerConnectionBuilder::new()
            .with_configuration(rtc_configuration(ice))
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_handler(Arc::new(bridge))
            .with_setting_engine(settings.build())
            // Sin esto el envio no espera nunca y un archivo grande se
            // acumularia entero en memoria. Ver SEND_BUFFER_LIMIT_BYTES.
            .with_data_channel_send_buffer_limit(SEND_BUFFER_LIMIT_BYTES)
            // Escucha en todas las interfaces. El puerto es el que digan las
            // opciones: al azar si sale por un servidor propio, fijo si hay que
            // abrirlo en un router.
            .with_udp_addrs(vec![network.bind_address()]);

        let connection = Box::pin(builder.build())
            .await
            .map_err(|e| TransportError::Peer(e.to_string()))?;
        let connection: Arc<dyn PeerConnection> = Arc::new(connection);

        if role == PeerRole::Offerer {
            for label in [CONTROL_CHANNEL, INPUT_CHANNEL, FILES_CHANNEL] {
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
        self.sender().send(label, data).await
    }

    /// Devuelve un remitente independiente de esta conexion.
    ///
    /// Hace falta porque leer eventos necesita `&mut self` y enviar no: sin
    /// esto, una tarea que envie archivos y el bucle que atiende las ordenes no
    /// podrian tener la conexion a la vez.
    ///
    /// Y tienen que poder: enviar un archivo de dos gigas desde el mismo bucle
    /// que mueve el raton significa un raton congelado durante la descarga. El
    /// remitente se clona y se lleva a su propia tarea.
    #[must_use]
    pub fn sender(&self) -> PeerSender {
        PeerSender {
            channels: Arc::clone(&self.channels),
        }
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

    #[test]
    fn por_omision_el_puerto_lo_elige_el_sistema() {
        let opciones = NetworkOptions::default();
        assert!(opciones.is_default());
        assert_eq!(opciones.bind_address(), "0.0.0.0:0");
        assert!(opciones.public_ip.is_none());
    }

    #[test]
    fn un_puerto_fijo_se_refleja_en_la_direccion_de_escucha() {
        // Es lo que permite abrirlo en el router: un puerto al azar no se
        // puede reenviar porque no se sabe cual sera.
        let opciones = NetworkOptions {
            udp_port: 51_820,
            public_ip: None,
        };
        assert_eq!(opciones.bind_address(), "0.0.0.0:51820");
        assert!(!opciones.is_default());
    }

    #[test]
    fn declarar_una_ip_publica_no_es_la_configuracion_normal() {
        let opciones = NetworkOptions {
            udp_port: 0,
            public_ip: Some("198.51.100.7".to_owned()),
        };
        assert!(!opciones.is_default());
    }

    #[tokio::test]
    async fn una_conexion_con_puerto_fijo_escucha_donde_se_le_dice() {
        // Se abren dos a la vez con el mismo puerto: la segunda debe fallar.
        // No es un defecto, es la consecuencia de fijar el puerto, y conviene
        // que quede comprobada para que nadie la descubra en produccion.
        let opciones = NetworkOptions {
            udp_port: 45_671,
            public_ip: None,
        };
        let primera = PeerSession::open_with(
            &IceConfiguration::new(vec![stun()]),
            PeerRole::Offerer,
            &opciones,
        )
        .await
        .expect("la primera deberia abrir");

        let segunda = PeerSession::open_with(
            &IceConfiguration::new(vec![stun()]),
            PeerRole::Offerer,
            &opciones,
        )
        .await;

        assert!(
            segunda.is_err(),
            "dos conexiones no pueden compartir un puerto fijo"
        );
        let _ = primera.close().await;
    }

    #[tokio::test]
    async fn una_ip_se_devuelve_tal_cual() {
        assert_eq!(
            resolve_public_ip("198.51.100.7").await.expect("vale"),
            "198.51.100.7"
        );
        assert_eq!(
            resolve_public_ip("  198.51.100.7  ").await.expect("vale"),
            "198.51.100.7"
        );
    }

    #[tokio::test]
    async fn un_nombre_se_resuelve() {
        // Es el caso de un DNS dinamico domestico, donde la IP cambia y el
        // nombre no.
        let resuelto = resolve_public_ip("localhost").await.expect("resuelve");
        assert_eq!(resuelto, "127.0.0.1");
    }

    #[tokio::test]
    async fn una_direccion_vacia_o_imposible_se_rechaza() {
        for malo in ["", "   "] {
            assert!(
                resolve_public_ip(malo).await.is_err(),
                "{malo:?} deberia rechazarse"
            );
        }
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
    async fn sin_servidores_ni_direccion_publica_se_rechaza() {
        let error = PeerSession::open(&IceConfiguration::new(vec![]), PeerRole::Offerer)
            .await
            .expect_err("eso solo funcionaria en la red local");
        match error {
            TransportError::InvalidIceConfiguration(detalle) => {
                assert!(detalle.contains("misma red"), "{detalle}");
            }
            otro => panic!("se esperaba un fallo de configuracion, llego {otro:?}"),
        }
    }

    #[tokio::test]
    async fn sin_servidores_pero_con_direccion_publica_si_vale() {
        // Es el montaje de quien aloja el servidor en su propio PC: no hay
        // STUN ni TURN, pero hay un puerto abierto que lleva hasta aqui.
        let sesion = PeerSession::open_with(
            &IceConfiguration::new(vec![]),
            PeerRole::Offerer,
            &NetworkOptions {
                udp_port: 0,
                public_ip: Some("198.51.100.7".to_owned()),
            },
        )
        .await
        .expect("deberia abrir");
        let _ = sesion.close().await;
    }
}
