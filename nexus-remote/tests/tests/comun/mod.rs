//! Utilidades compartidas por las pruebas de integracion de red.
//!
//! Lo que monta aqui es el servidor **de verdad**: el mismo
//! `nexus_signaling::serve` que usa el binario, escuchando en un puerto libre
//! de la maquina local. No hay dobles ni simulaciones del servidor.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use nexus_core::device::{DeviceInfo, DevicePlatform, DeviceRole};
use nexus_core::{Capability, CapabilitySet, DeviceId};
use nexus_security::DeviceIdentity;
use nexus_signaling::{AppState, Config};
use nexus_transport::client::{Accepted, ConnectRequest, SignalingClient};

/// Testigo de alta del servidor de pruebas.
pub const TOKEN: &str = "testigo-de-alta-para-las-pruebas-de-integracion";

/// Tope de espera para cualquier paso de senalizacion.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// Un servidor de senalizacion en marcha, escuchando en la maquina local.
pub struct TestServer {
    /// URL del canal de senalizacion.
    pub url: String,
    /// Estado del servidor, para poder mirar su registro desde la prueba.
    pub state: Arc<AppState>,
    /// Direccion en la que escucha.
    pub address: SocketAddr,
}

impl TestServer {
    /// Arranca un servidor en un puerto libre.
    ///
    /// Los servidores STUN y TURN que anuncia apuntan a la propia maquina en un
    /// puerto cerrado. Es deliberado: la configuracion tiene que ser valida
    /// (sin ella el cliente se niega a conectar, y con razon), pero en una
    /// prueba no hay ningun Coturn en marcha. Lo que se comprueba aqui es la
    /// senalizacion y la conexion directa; que el relay funcione de verdad hace
    /// falta desplegarlo.
    pub async fn start() -> Self {
        let config = Config::from_lookup(|name| match name {
            "NEXUS_PUBLIC_URL" => Some("wss://pruebas.invalido/v1/signaling".to_owned()),
            "NEXUS_TURN_SHARED_SECRET" => Some("secreto-de-turn-para-las-pruebas".to_owned()),
            "NEXUS_TURN_URLS" => Some("turn:127.0.0.1:3478".to_owned()),
            "NEXUS_STUN_URLS" => Some("stun:127.0.0.1:3478".to_owned()),
            "NEXUS_HOST_ENROLLMENT_TOKEN" => Some(TOKEN.to_owned()),
            _ => None,
        })
        .expect("configuracion de prueba valida");

        let state = Arc::new(AppState::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("un puerto libre en la maquina local");
        let address = listener.local_addr().expect("direccion de escucha");

        let served = Arc::clone(&state);
        tokio::spawn(async move {
            let _ = nexus_signaling::serve(listener, served, std::future::pending()).await;
        });

        Self {
            url: format!("ws://{address}/v1/signaling"),
            state,
            address,
        }
    }

    /// Lineas del registro de conexiones del servidor.
    pub fn audit_lines(&self) -> Vec<String> {
        self.state
            .shared()
            .audit
            .entries()
            .map(ToString::to_string)
            .collect()
    }
}

/// Un dispositivo de prueba: su identidad y los datos que declara.
pub struct Device {
    /// Identidad criptografica.
    pub identity: DeviceIdentity,
    /// Datos declarados.
    pub info: DeviceInfo,
    /// Capacidades que anuncia.
    pub capabilities: CapabilitySet,
}

impl Device {
    /// Crea un host de prueba.
    pub fn host(name: &str) -> Self {
        Self::new(name, DeviceRole::Host, DevicePlatform::Windows)
    }

    /// Crea un controlador de prueba.
    pub fn controller(name: &str) -> Self {
        Self::new(name, DeviceRole::Controller, DevicePlatform::IpadOs)
    }

    fn new(name: &str, role: DeviceRole, platform: DevicePlatform) -> Self {
        let id = DeviceId::new();
        let mut capabilities = CapabilitySet::empty();
        capabilities.insert(Capability::ScreenStream);
        capabilities.insert(Capability::RemoteInput);
        capabilities.insert(Capability::MultiMonitor);

        Self {
            identity: DeviceIdentity::generate(id),
            info: DeviceInfo {
                id,
                display_name: name.to_owned(),
                platform,
                role,
                os_version: "1.0".to_owned(),
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            capabilities,
        }
    }

    /// Identificador del dispositivo.
    pub fn id(&self) -> DeviceId {
        self.info.id
    }

    /// Copia de la identidad, para tareas que no pueden tomarla prestada.
    pub fn duplicate_identity(&self) -> DeviceIdentity {
        DeviceIdentity::from_secret_bytes(self.info.id, self.identity.export_secret().as_slice())
            .expect("la misma clave")
    }

    /// Se conecta al servidor y se autentica.
    pub async fn connect(
        &self,
        server: &TestServer,
        token: Option<&str>,
    ) -> Result<(SignalingClient, Accepted), nexus_transport::TransportError> {
        SignalingClient::connect(
            ConnectRequest {
                url: &server.url,
                device: &self.info,
                capabilities: &self.capabilities,
                enrollment_token: token,
            },
            &self.identity,
        )
        .await
    }

    /// Se conecta y falla la prueba si no lo consigue.
    pub async fn connect_ok(
        &self,
        server: &TestServer,
        token: Option<&str>,
    ) -> (SignalingClient, Accepted) {
        self.connect(server, token)
            .await
            .unwrap_or_else(|e| panic!("{} deberia autenticarse: {e}", self.info.display_name))
    }
}

/// Inyector de entrada cuyo registro vive fuera del manejador.
///
/// El `SessionHandler` del host se queda con el inyector que se le pasa, asi
/// que para poder comprobar que llego hace falta uno que comparta su registro.
#[derive(Clone, Default)]
pub struct InyectorCompartido(std::sync::Arc<std::sync::Mutex<Vec<nexus_input::ResolvedInput>>>);

impl InyectorCompartido {
    /// Ordenes que ha recibido, en orden.
    pub fn recibidas(&self) -> Vec<nexus_input::ResolvedInput> {
        self.0.lock().expect("sin envenenar").clone()
    }
}

impl nexus_input::InputInjector for InyectorCompartido {
    fn name(&self) -> &'static str {
        "compartido"
    }

    fn inject(&mut self, input: &nexus_input::ResolvedInput) -> nexus_input::InputResult<()> {
        self.0.lock().expect("sin envenenar").push(input.clone());
        Ok(())
    }

    fn release_all(&mut self) -> nexus_input::InputResult<()> {
        Ok(())
    }
}

/// Un host del producto en marcha, con su registro y su inyector.
///
/// Es el bucle de verdad (`HostRuntime`): se conecta solo, decide solo contra
/// su registro de dispositivos, negocia solo y sirve solo. Esta aqui, y no en
/// una sola prueba, porque varias lo necesitan y todas tienen que ejercitar el
/// mismo bucle, no una copia parecida.
pub struct HostDeProducto {
    eventos: tokio::sync::mpsc::Receiver<nexus_windows_host::runtime::RuntimeEvent>,
    /// Inyector del host, para comprobar que una orden llego al equipo.
    pub inyector: nexus_windows_host::runtime::RecordingInjectors,
    parar: Option<tokio::sync::oneshot::Sender<()>>,
    tarea: tokio::task::JoinHandle<()>,
    id: DeviceId,
}

impl HostDeProducto {
    /// Pantalla que el host declara en las pruebas.
    pub fn layout() -> nexus_core::monitor::MonitorLayout {
        nexus_core::monitor::MonitorLayout(vec![nexus_core::monitor::MonitorInfo {
            index: 0,
            name: "Principal".to_owned(),
            width: 1_920,
            height: 1_080,
            origin_x: 0,
            origin_y: 0,
            scale_percent: 100,
            is_primary: true,
            refresh_hz: Some(60),
        }])
    }

    /// Arranca el bucle con los dispositivos emparejados que se indiquen y sin
    /// compartir ninguna carpeta.
    pub async fn arrancar(
        server: &TestServer,
        emparejados: Vec<nexus_security::registry::DeviceRecord>,
    ) -> Self {
        Self::arrancar_con_carpetas(server, emparejados, Vec::new()).await
    }

    /// Igual, declarando ademas las carpetas que el host comparte.
    pub async fn arrancar_con_carpetas(
        server: &TestServer,
        emparejados: Vec<nexus_security::registry::DeviceRecord>,
        carpetas: Vec<nexus_file_transfer::SharedRoot>,
    ) -> Self {
        Self::arrancar_con(server, emparejados, carpetas, None).await
    }

    /// Igual, con una pantalla que el host pueda capturar y enviar.
    ///
    /// Sin esto el host arranca con el capturador de la plataforma, que fuera
    /// de Windows no captura nada y lo dice. Con esto recorre el camino
    /// completo: escalar, codificar, trocear en RTP y entregar.
    ///
    /// La pantalla se pasa a proposito en lugar de fijarla: codificar
    /// 1920x1080 cuesta bastante CPU, y una prueba que no este midiendo eso
    /// deberia pedir una pantalla pequena para no ahogar la maquina donde
    /// corre.
    pub async fn arrancar_con_pantalla(
        server: &TestServer,
        emparejados: Vec<nexus_security::registry::DeviceRecord>,
        pantalla: nexus_core::monitor::MonitorLayout,
    ) -> Self {
        Self::arrancar_con(server, emparejados, Vec::new(), Some(pantalla)).await
    }

    /// Una pantalla pequena, para pruebas que no miden la imagen.
    pub fn pantalla_pequena() -> nexus_core::monitor::MonitorLayout {
        nexus_core::monitor::MonitorLayout(vec![nexus_core::monitor::MonitorInfo {
            index: 0,
            name: "Pequena".to_owned(),
            width: 320,
            height: 240,
            origin_x: 0,
            origin_y: 0,
            scale_percent: 100,
            is_primary: true,
            refresh_hz: Some(60),
        }])
    }

    async fn arrancar_con(
        server: &TestServer,
        emparejados: Vec<nexus_security::registry::DeviceRecord>,
        carpetas: Vec<nexus_file_transfer::SharedRoot>,
        pantalla: Option<nexus_core::monitor::MonitorLayout>,
    ) -> Self {
        let host = Device::host("PC del salon");
        let mut registro = nexus_windows_host::runtime::InMemoryLookup::new();
        for record in emparejados {
            registro.insert(record);
        }

        let inyector = nexus_windows_host::runtime::RecordingInjectors::default();
        let (tx, eventos) = tokio::sync::mpsc::channel(64);
        let (parar, apagado) = tokio::sync::oneshot::channel();

        let runtime = nexus_windows_host::runtime::HostRuntime::new(
            nexus_windows_host::runtime::RuntimeConfig {
                url: server.url.clone(),
                device: host.info.clone(),
                enrollment_token: Some(TOKEN.to_owned()),
                monitors: pantalla.clone().unwrap_or_else(Self::layout),
                udp_port: 0,
                public_endpoint: None,
                shared_roots: carpetas,
            },
            host.duplicate_identity(),
            Arc::new(registro),
            Arc::new(inyector.clone()),
            tx,
        );

        let runtime = match pantalla {
            Some(layout) => runtime.with_capturers(Arc::new(PantallaSintetica(layout))),
            None => runtime,
        };

        let tarea = tokio::spawn(async move {
            runtime
                .run(async {
                    let _ = apagado.await;
                })
                .await;
        });

        let mut yo = Self {
            eventos,
            inyector,
            parar: Some(parar),
            tarea,
            id: host.id(),
        };

        // Esperar a que se declare disponible antes de devolverlo: si no, el
        // controlador pediria sesion contra un host que aun no existe.
        match yo.siguiente_evento().await {
            nexus_windows_host::runtime::RuntimeEvent::Available { has_relay, .. } => {
                assert!(has_relay);
            }
            otro => panic!("el host deberia declararse disponible, llego {otro:?}"),
        }
        yo
    }

    /// Identificador del host.
    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// Siguiente cosa que el host cuenta de si mismo.
    pub async fn siguiente_evento(&mut self) -> nexus_windows_host::runtime::RuntimeEvent {
        tokio::time::timeout(STEP_TIMEOUT, self.eventos.recv())
            .await
            .expect("el host deberia dar senales de vida")
            .expect("el canal de eventos sigue abierto")
    }

    /// Para el bucle y espera a que termine.
    pub async fn parar(mut self) {
        if let Some(parar) = self.parar.take() {
            let _ = parar.send(());
        }
        let _ = tokio::time::timeout(STEP_TIMEOUT, self.tarea).await;
    }
}

/// Quien conduce el lado del controlador en una sesion.
pub struct LadoControlador<'a> {
    /// Sesion negociada.
    pub session_id: nexus_core::SessionId,
    /// Identificador de este dispositivo.
    pub yo: DeviceId,
    /// Identificador del host.
    pub host: DeviceId,
    /// Quien firma la atadura de sesion.
    pub firmante: &'a DeviceIdentity,
}

/// Atiende un mensaje de senalizacion del lado del controlador.
///
/// Es el mismo baile en todas las pruebas: contestar la oferta, firmar la
/// atadura con las dos huellas DTLS y pasar los candidatos ICE. Esta aqui una
/// vez para que cada prueba escriba solo lo suyo, y para que todas ejerciten
/// exactamente la misma negociacion.
///
/// Devuelve `false` cuando el otro extremo se fue y la prueba debe parar.
pub async fn atender_senalizacion(
    sig: &mut SignalingClient,
    peer: &nexus_transport::peer::PeerSession,
    mensaje: nexus_core::protocol::signaling::ServerToClient,
    lado: &LadoControlador<'_>,
) -> bool {
    use nexus_core::protocol::signaling::{ClientToServer, ServerToClient};

    match mensaje {
        ServerToClient::Offer { sdp, .. } => {
            let answer = peer.accept_offer(&sdp).await.expect("contesta la oferta");
            sig.send(ClientToServer::Answer {
                session_id: lado.session_id,
                sdp: answer,
            })
            .await
            .expect("envia la respuesta");

            // Las dos huellas DTLS ya se conocen: aqui es donde se ata la
            // sesion al canal cifrado concreto.
            let vista = nexus_security::SessionBinding {
                session_id: lado.session_id,
                role: nexus_security::AuthRole::Controller,
                local_device: lado.yo,
                remote_device: lado.host,
                local_fingerprint: peer.local_fingerprint().await.expect("huella local"),
                remote_fingerprint: peer.remote_fingerprint().await.expect("huella remota"),
            };
            let prueba = vista.prove(lado.firmante);
            sig.send(ClientToServer::PeerAuth {
                session_id: lado.session_id,
                fingerprint: prueba.fingerprint,
                signature_hex: prueba.signature_hex,
            })
            .await
            .expect("envia la atadura");
            true
        }
        ServerToClient::IceCandidate {
            candidate, sdp_mid, ..
        } => {
            peer.add_remote_candidate(candidate, sdp_mid)
                .await
                .expect("anade camino");
            true
        }
        ServerToClient::PeerGone { .. } => false,
        _ => true,
    }
}

/// Pasa un candidato ICE propio al otro extremo.
pub async fn enviar_candidato(
    sig: &mut SignalingClient,
    session_id: nexus_core::SessionId,
    candidate: String,
    sdp_mid: Option<String>,
) {
    sig.send(
        nexus_core::protocol::signaling::ClientToServer::IceCandidate {
            session_id,
            candidate,
            sdp_mid,
        },
    )
    .await
    .expect("envia camino");
}

/// Pantalla de prueba para las sesiones completas.
///
/// **No es captura.** Entrega fotogramas de color plano del tamano que el host
/// declara, porque capturar de verdad solo funciona en Windows. Lo que si
/// ejercita es todo lo que viene despues —escalar, codificar en H.264, trocear
/// en paquetes RTP, cifrar y entregar—, que es exactamente la parte que no se
/// podria probar de ninguna otra forma en esta maquina.
pub struct PantallaSintetica(pub nexus_core::monitor::MonitorLayout);

impl nexus_windows_host::screen::CapturerSource for PantallaSintetica {
    fn create(
        &self,
    ) -> Result<Box<dyn nexus_capture::ScreenCapturer>, nexus_capture::error::CaptureError> {
        Ok(Box::new(nexus_capture::SyntheticCapturer::new(
            self.0.clone(),
        )))
    }
}
