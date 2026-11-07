//! El bucle que mantiene vivo el host.
//!
//! Esto es lo que convierte "los dos extremos saben conectarse" en "este PC
//! atiende sesiones": se conecta al servidor propio, se declara disponible,
//! decide que hacer con cada peticion, levanta la conexion cifrada, comprueba
//! quien hay al otro lado y sirve el canal de control hasta que la sesion
//! termina. Si el enlace se cae, vuelve a intentarlo con espera creciente.
//!
//! # Lo que no se hace hasta estar seguro de con quien se habla
//!
//! El orden importa y es deliberado:
//!
//! 1. Se decide la sesion contra el registro local ([`crate::admission`]).
//! 2. Se negocia WebRTC.
//! 3. Se comprueba la atadura de sesion contra la clave que el registro local
//!    tiene para ese dispositivo, **no** contra la que diga el servidor.
//! 4. Solo entonces se crea el manejador que puede mover el raton.
//!
//! Mientras el paso 3 no se cumple, lo que llegue por el canal de datos se
//! descarta y se cuenta. No es una comprobacion que se pueda olvidar: el
//! manejador no existe todavia.
//!
//! # Una sesion a la vez
//!
//! El servidor ya lo impone, pero aqui tambien: este es un escritorio, y dos
//! personas moviendo el mismo raton no es una funcion, es un fallo.

use std::sync::Arc;
use std::time::Duration;

use nexus_core::monitor::MonitorLayout;
use nexus_core::protocol::control::HostEvent;
use nexus_core::protocol::signaling::{ClientToServer, ServerToClient};
use nexus_core::{
    CapabilitySet, CloseReason, DeviceId, DeviceInfo, Permission, PermissionSet, QualityProfile,
    SessionId, Timestamp,
};
use nexus_file_transfer::{AccessPolicy, Frame, SharedRoot};
use nexus_input::InputInjector;
use nexus_security::registry::DeviceRecord;
use nexus_security::{AuthRole, BindingProof, DeviceIdentity, DevicePublicKey, SessionBinding};
use nexus_transport::client::{ConnectRequest, SignalingClient};
use nexus_transport::peer::{
    CONTROL_CHANNEL, FILES_CHANNEL, INPUT_CHANNEL, LinkPhase, NetworkOptions, PeerEvent, PeerRole,
    PeerSender, PeerSession, ScreenSender, resolve_public_ip,
};
use nexus_transport::{LinkEvent, LinkState, SignalingLink, TransportError};
use tokio::sync::mpsc;

use crate::admission::{self, Decision};
use crate::confirmation::SharedConfirmations;
use crate::files::{Download, TransferOutcome, TransferReport};
use crate::screen::{CapturerSource, PlatformCapturers, ScreenCommand, ScreenOutput};
use crate::session::SessionHandler;

/// Datos con los que arranca el bucle.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// URL del canal de senalizacion.
    pub url: String,
    /// Datos que este host declara.
    pub device: DeviceInfo,
    /// Testigo de alta, solo en la primera conexion a un servidor.
    pub enrollment_token: Option<String>,
    /// Monitores del equipo, para resolver las coordenadas.
    pub monitors: MonitorLayout,
    /// Puerto UDP fijo para la conexion cifrada, o `0` para uno automatico.
    ///
    /// Hace falta fijarlo cuando este equipo se alcanza desde fuera por un
    /// puerto abierto en el router: un puerto al azar no se puede reenviar.
    pub udp_port: u16,
    /// Direccion publica que anunciar: una IP o el nombre de un DNS dinamico.
    ///
    /// Se resuelve **al abrir cada sesion**, de modo que un cambio de IP
    /// domestica se recoge solo en la siguiente conexion.
    pub public_endpoint: Option<String>,
    /// Carpetas que este host comparte, y con que acceso.
    ///
    /// Vacio por omision: el permiso de archivos de un dispositivo no sirve de
    /// nada hasta que aqui haya una carpeta. Son dos concesiones separadas a
    /// proposito, y la de aqui la da el dueno del PC.
    pub shared_roots: Vec<SharedRoot>,
}

/// Acceso al registro local de dispositivos emparejados.
///
/// Es un rasgo para que el host lo respalde con `SQLite` y las pruebas con
/// memoria, sin que el bucle cambie.
pub trait DeviceLookup: Send + Sync {
    /// Busca un dispositivo emparejado.
    fn get(&self, device_id: DeviceId) -> Option<DeviceRecord>;
}

/// De donde sale el backend que mueve el raton de verdad.
///
/// En Windows es el que llama a `SendInput`; en una prueba, uno que anota.
pub trait InjectorSource: Send + Sync {
    /// Crea un backend para una sesion.
    fn create(&self) -> Box<dyn InputInjector + Send>;
}

/// Lo que va ocurriendo, para la interfaz y el historial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    /// Conectado al servidor y disponible.
    Available {
        /// Servidores ICE recibidos.
        ice_servers: usize,
        /// Si entre ellos hay relay.
        has_relay: bool,
    },
    /// Se perdio el enlace con el servidor.
    LinkLost {
        /// Que paso, en lenguaje llano.
        detail: String,
        /// Cuanto se espera antes de reintentar.
        retry_in: Option<Duration>,
    },
    /// Se agotaron los reintentos.
    GaveUp {
        /// Intentos consumidos.
        attempts: u32,
    },
    /// Llego una peticion de sesion y se resolvio.
    SessionDecided {
        /// Quien la pidio.
        controller: DeviceId,
        /// Si se acepto.
        accepted: bool,
        /// Motivo, para el registro local.
        detail: String,
    },
    /// La sesion quedo establecida y autenticada.
    SessionStarted {
        /// Sesion.
        session_id: SessionId,
        /// Controlador.
        controller: DeviceId,
        /// Permisos efectivos.
        permissions: PermissionSet,
    },
    /// La sesion termino.
    SessionEnded {
        /// Sesion.
        session_id: SessionId,
        /// Motivo, para el historial.
        reason: CloseReason,
        /// Explicacion en lenguaje llano.
        detail: &'static str,
        /// Ordenes aplicadas durante la sesion.
        applied: u64,
        /// Ordenes rechazadas durante la sesion.
        refused: u64,
    },
}

/// Como termino el bucle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOutcome {
    /// Se pidio parar.
    Stopped,
    /// Se agotaron los reintentos de reconexion.
    GaveUp,
}

/// Tiempo maximo que se espera a que una sesion quede autenticada.
///
/// Si en ese tiempo no se ha comprobado la atadura, la sesion se cierra: un
/// extremo que no firma es un extremo con el que no hay que hablar.
pub const BINDING_TIMEOUT: Duration = Duration::from_secs(30);

/// Informes de progreso que caben en la cola antes de empezar a perderlos.
///
/// Los intermedios se pueden perder sin consecuencia porque llevan el total
/// enviado y no el incremento: lo unico que se nota es que la barra avanza a
/// saltos. El informe final no se pierde nunca, porque ese si espera sitio.
const REPORT_QUEUE_DEPTH: usize = 64;

/// Fotogramas que caben en la cola entre la captura y el envio.
///
/// Corta a proposito: una cola de fotogramas es retraso acumulado, y en una
/// pantalla remota el retraso se nota mas que un tiron. Con dos, el hilo de
/// captura se para en cuanto el envio se atrasa, que es exactamente lo que
/// tiene que pasar; ninguno se tira, porque tirar uno estropearia los
/// siguientes.
const FRAME_QUEUE_DEPTH: usize = 2;

/// El bucle del host.
pub struct HostRuntime {
    config: RuntimeConfig,
    identity: DeviceIdentity,
    lookup: Arc<dyn DeviceLookup>,
    injectors: Arc<dyn InjectorSource>,
    events: mpsc::Sender<RuntimeEvent>,
    /// Puerta de confirmaciones compartida con el panel del host.
    ///
    /// Sin compartirla, una sesion podria pedir confirmacion para apagar y
    /// nadie tendria donde aprobarla.
    confirmations: SharedConfirmations,
    /// De donde sale el capturador de pantalla de cada sesion.
    capturers: Arc<dyn CapturerSource>,
    link: SignalingLink,
}

impl std::fmt::Debug for HostRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostRuntime")
            .field("url", &self.config.url)
            .field("link", &self.link.state())
            .finish_non_exhaustive()
    }
}

impl HostRuntime {
    /// Crea el bucle.
    #[must_use]
    pub fn new(
        config: RuntimeConfig,
        identity: DeviceIdentity,
        lookup: Arc<dyn DeviceLookup>,
        injectors: Arc<dyn InjectorSource>,
        events: mpsc::Sender<RuntimeEvent>,
    ) -> Self {
        Self {
            config,
            identity,
            lookup,
            injectors,
            events,
            confirmations: SharedConfirmations::new(),
            capturers: Arc::new(PlatformCapturers),
            link: SignalingLink::new(),
        }
    }

    /// Comparte la puerta de confirmaciones con el panel del host.
    #[must_use]
    pub fn with_confirmations(mut self, confirmations: SharedConfirmations) -> Self {
        self.confirmations = confirmations;
        self
    }

    /// Cambia de donde sale el capturador de pantalla.
    ///
    /// Por omision es el de la plataforma. Las pruebas de extremo a extremo
    /// ponen aqui un capturador sintetico, que no es captura real pero recorre
    /// el mismo camino: codificar, trocear en RTP y llegar al otro lado.
    #[must_use]
    pub fn with_capturers(mut self, capturers: Arc<dyn CapturerSource>) -> Self {
        self.capturers = capturers;
        self
    }

    /// Estado del enlace con el servidor.
    #[must_use]
    pub const fn link_state(&self) -> LinkState {
        self.link.state()
    }

    /// Atiende sesiones hasta que `shutdown` termine o se agoten los intentos.
    pub async fn run<F>(mut self, shutdown: F) -> RuntimeOutcome
    where
        F: std::future::Future<Output = ()> + Send,
    {
        tokio::pin!(shutdown);

        loop {
            let _ = self.link.apply(LinkEvent::ConnectRequested);

            let resultado = tokio::select! {
                () = &mut shutdown => return RuntimeOutcome::Stopped,
                resultado = self.one_connection() => resultado,
            };

            let detalle = match resultado {
                Ok(()) => "el servidor cerro el enlace".to_owned(),
                Err(error) => error.to_string(),
            };

            match self.link.apply(LinkEvent::SocketClosed) {
                Ok(_) => {
                    let espera = self.link.current_backoff();
                    self.emit(RuntimeEvent::LinkLost {
                        detail: detalle,
                        retry_in: Some(espera),
                    })
                    .await;

                    tokio::select! {
                        () = &mut shutdown => return RuntimeOutcome::Stopped,
                        () = tokio::time::sleep(espera) => {}
                    }
                    let _ = self.link.apply(LinkEvent::RetryDue);
                }
                Err(TransportError::ReconnectExhausted { attempts }) => {
                    self.emit(RuntimeEvent::LinkLost {
                        detail: detalle,
                        retry_in: None,
                    })
                    .await;
                    self.emit(RuntimeEvent::GaveUp { attempts }).await;
                    return RuntimeOutcome::GaveUp;
                }
                Err(_) => return RuntimeOutcome::Stopped,
            }
        }
    }

    /// Una conexion completa: autenticarse, declararse y atender sesiones.
    async fn one_connection(&mut self) -> Result<(), TransportError> {
        let (mut sig, accepted) = SignalingClient::connect(
            ConnectRequest {
                url: &self.config.url,
                device: &self.config.device,
                capabilities: &admission::host_capabilities(),
                enrollment_token: self.config.enrollment_token.as_deref(),
            },
            &self.identity,
        )
        .await?;
        let _ = self.link.apply(LinkEvent::SocketOpened);

        let monitores = u8::try_from(self.config.monitors.0.len())
            .unwrap_or(u8::MAX)
            .max(1);
        sig.register_host(monitores).await?;
        let _ = self.link.apply(LinkEvent::Authenticated);

        self.emit(RuntimeEvent::Available {
            ice_servers: accepted.ice.servers.len(),
            has_relay: accepted.ice.has_relay(),
        })
        .await;

        loop {
            match sig.recv().await? {
                ServerToClient::SessionRequested {
                    session_id,
                    controller,
                    controller_capabilities,
                    requested_permissions,
                } => {
                    self.on_session_requested(
                        &mut sig,
                        &accepted.ice,
                        session_id,
                        controller,
                        &controller_capabilities,
                        &requested_permissions,
                    )
                    .await?;
                }
                // Fuera de una sesion no hay nada que hacer con lo demas.
                ServerToClient::Error { code, message } => {
                    tracing::warn!(%code, %message, "el servidor devolvio un error");
                }
                otro => {
                    tracing::debug!("mensaje inesperado fuera de sesion: {otro:?}");
                }
            }
        }
    }

    /// Como se expone este equipo, con la direccion publica ya resuelta.
    ///
    /// Se resuelve en cada sesion y no una vez al arrancar: una IP domestica
    /// cambia, y lo que no cambia es el nombre del DNS dinamico.
    async fn network(&self) -> Result<NetworkOptions, TransportError> {
        let public_ip = match &self.config.public_endpoint {
            Some(endpoint) => Some(resolve_public_ip(endpoint).await?),
            None => None,
        };
        Ok(NetworkOptions {
            udp_port: self.config.udp_port,
            public_ip,
        })
    }

    /// Decide la peticion y, si procede, sirve la sesion.
    async fn on_session_requested(
        &mut self,
        sig: &mut SignalingClient,
        ice: &nexus_transport::IceConfiguration,
        session_id: SessionId,
        controller: DeviceInfo,
        controller_capabilities: &CapabilitySet,
        requested_permissions: &PermissionSet,
    ) -> Result<(), TransportError> {
        let record = self.lookup.get(controller.id);
        let decision = admission::decide(
            record.as_ref(),
            &controller,
            requested_permissions,
            controller_capabilities,
            &admission::host_capabilities(),
        );

        let (permissions, capabilities, peer_key) = match decision {
            Decision::Accept {
                permissions,
                capabilities,
                peer_key,
            } => {
                self.emit(RuntimeEvent::SessionDecided {
                    controller: controller.id,
                    accepted: true,
                    detail: "acceso desatendido autorizado".to_owned(),
                })
                .await;
                (permissions, capabilities, *peer_key)
            }
            Decision::NeedsConfirmation { .. } => {
                // El dispositivo esta autorizado, pero no para entrar solo, y
                // todavia no hay forma de aceptar la sesion en el PC. Se dice
                // tal cual en lugar de dejar al controlador esperando.
                let detalle = "hace falta aceptar la sesion en el PC, y esa \
                               confirmacion todavia no esta implementada";
                self.reject(sig, session_id, controller.id, detalle).await?;
                return Ok(());
            }
            Decision::Reject { reason, detail } => {
                tracing::warn!(%detail, "sesion rechazada");
                self.reject(sig, session_id, controller.id, reason).await?;
                return Ok(());
            }
        };

        sig.send(ClientToServer::AcceptSession {
            session_id,
            negotiated_capabilities: capabilities,
            effective_permissions: permissions.clone(),
        })
        .await?;

        self.serve_session(sig, ice, session_id, &controller, permissions, &peer_key)
            .await
    }

    async fn reject(
        &mut self,
        sig: &mut SignalingClient,
        session_id: SessionId,
        controller: DeviceId,
        motivo: &str,
    ) -> Result<(), TransportError> {
        self.emit(RuntimeEvent::SessionDecided {
            controller,
            accepted: false,
            detail: motivo.to_owned(),
        })
        .await;
        sig.send(ClientToServer::RejectSession {
            session_id,
            reason: motivo.to_owned(),
        })
        .await
    }

    /// Levanta la conexion cifrada y sirve la sesion hasta que termina.
    async fn serve_session(
        &mut self,
        sig: &mut SignalingClient,
        ice: &nexus_transport::IceConfiguration,
        session_id: SessionId,
        controller: &DeviceInfo,
        permissions: PermissionSet,
        peer_key: &DevicePublicKey,
    ) -> Result<(), TransportError> {
        let mut peer =
            PeerSession::open_with(ice, PeerRole::Offerer, &self.network().await?).await?;
        let offer = peer.create_offer().await?;
        sig.send(ClientToServer::Offer {
            session_id,
            sdp: offer,
        })
        .await?;

        let mut estado = SessionState::new(session_id, controller.id, self.identity.device_id());
        let mut handler: Option<SessionHandler> = None;
        // Por aqui cuentan las tareas de envio lo que van sacando. Tiene cola
        // porque son varias y el bucle puede estar ocupado; si se llenara, el
        // informe se pierde y la barra de progreso se queda corta, que es un
        // precio aceptable por no frenar el envio.
        let (reportes, mut recibe_reportes) = mpsc::channel::<TransferReport>(REPORT_QUEUE_DEPTH);
        let mut envios = tokio::task::JoinSet::new();
        // La pista de video existe desde que se abre la conexion, pero no sale
        // nada por ella hasta que el bombeo arranca, y el bombeo no arranca
        // hasta que se sabe quien hay al otro lado.
        let mut bombeo: Option<Bombeo> = None;
        let limite = tokio::time::sleep(BINDING_TIMEOUT);
        tokio::pin!(limite);

        let motivo: Motivo = loop {
            tokio::select! {
                () = &mut limite, if handler.is_none() => {
                    break (
                        CloseReason::AuthenticationFailed,
                        "el otro extremo no se autentico a tiempo",
                    );
                }
                mensaje = sig.recv() => {
                    match mensaje {
                        Ok(mensaje) => {
                            if let Some(fin) = self
                                .on_signaling(sig, &mut peer, &mut estado, mensaje)
                                .await?
                            {
                                break fin;
                            }
                        }
                        Err(error) => {
                            // El enlace con el servidor se cayo: la sesion no
                            // puede continuar porque no habria por donde
                            // cerrarla ni renegociarla.
                            let counts = recuento(handler.as_ref());
                            self.end_session(
                                session_id,
                                estado.dropped,
                                counts,
                                (CloseReason::NetworkLost, "se perdio el enlace"),
                            )
                            .await;
                            return Err(error);
                        }
                    }
                }
                evento = peer.next_event() => {
                    let Some(evento) = evento else {
                        break (CloseReason::PeerClosed, "la conexion cifrada se cerro");
                    };
                    if let Some(fin) = self
                        .on_peer(sig, &peer, &mut estado, evento, &mut handler)
                        .await?
                    {
                        break fin;
                    }
                }
                Some(reporte) = recibe_reportes.recv() => {
                    anunciar_progreso(&peer, handler.as_mut(), &reporte).await;
                }
            }

            // Las descargas que el manejador acepto salen en su propia tarea:
            // enviar un archivo grande desde este bucle dejaria el raton
            // congelado hasta terminar.
            arrancar_descargas(&peer, handler.as_mut(), &reportes, &mut envios);

            // La pantalla arranca cuando la sesion la pide, no antes (ver
            // `ControlRequest::StartScreen`). Y solo se puede pedir por el
            // canal de control, que solo se atiende con la atadura comprobada:
            // por eso esto no es una segunda puerta, es la misma.
            if bombeo.is_none()
                && let Some(handler) = handler.as_ref()
                && handler.screen_requests() > 0
            {
                bombeo = self.start_screen(handler, &permissions, &peer);
            }
            sincronizar_pantalla(bombeo.as_mut(), handler.as_ref());

            // La atadura se comprueba en cuanto se tienen las dos piezas, y es
            // lo unico que da paso al manejador.
            if handler.is_none() {
                match self.autorizar(&estado, peer_key, &permissions).await {
                    Ok(Some(nuevo)) => handler = Some(nuevo),
                    Ok(None) => {}
                    Err(motivo) => break motivo,
                }
            }
        };

        cerrar_trabajos(handler.as_mut(), &mut envios, bombeo.take()).await;

        let counts = recuento(handler.as_ref());
        self.end_session(session_id, estado.dropped, counts, motivo)
            .await;
        let _ = sig.send(ClientToServer::Bye { session_id }).await;
        let _ = peer.close().await;
        Ok(())
    }

    /// Comprueba la atadura y, si cuadra, monta lo que la sesion necesita.
    ///
    /// Devuelve `Ok(None)` mientras falte alguna de las dos piezas (la huella
    /// del canal cifrado y la firma del otro extremo). Devuelve `Err` cuando
    /// no cuadran: eso no es un retraso, es alguien en medio.
    ///
    /// Es el unico sitio donde nace el manejador, y por tanto el unico sitio
    /// desde el que se puede empezar a mover el raton. La pantalla necesita
    /// ademas que la sesion la pida, asi que arranca aparte, en el bucle.
    async fn autorizar(
        &self,
        estado: &SessionState,
        peer_key: &DevicePublicKey,
        permissions: &PermissionSet,
    ) -> Result<Option<SessionHandler>, Motivo> {
        let Some(verificada) = estado.verify(peer_key) else {
            return Ok(None);
        };
        if !verificada {
            return Err((
                CloseReason::AuthenticationFailed,
                "la atadura de sesion no cuadra: posible interceptacion",
            ));
        }

        let handler = SessionHandler::new(
            permissions.clone(),
            self.config.monitors.clone(),
            self.injectors.create(),
        )
        .with_shared_folders(AccessPolicy::new(self.config.shared_roots.clone()))
        .with_confirmations(self.confirmations.clone());

        self.emit(RuntimeEvent::SessionStarted {
            session_id: estado.session_id,
            controller: estado.controller,
            permissions: permissions.clone(),
        })
        .await;

        Ok(Some(handler))
    }

    /// Pone en marcha el envio de la pantalla de una sesion ya autenticada.
    ///
    /// Devuelve `None` cuando no hay nada que enviar: el dispositivo no tiene
    /// permiso para ver la pantalla, no hay pista de video por la que enviarla,
    /// o el sistema no deja crear el hilo. En ninguno de los tres casos se
    /// corta la sesion: el teclado, el raton y las transferencias no dependen
    /// de esto.
    fn start_screen(
        &self,
        handler: &SessionHandler,
        permissions: &PermissionSet,
        peer: &PeerSession,
    ) -> Option<Bombeo> {
        if !permissions.has(Permission::ViewScreen) {
            tracing::info!("la sesion no puede ver la pantalla: no se captura nada");
            return None;
        }
        let Some(pista) = peer.screen() else {
            // Solo el que abre la conexion tiene pista, y en este producto el
            // que abre es siempre el host. Sin pista, capturar seria gastar
            // CPU para tirar el resultado.
            tracing::warn!("no hay pista de video: no se captura nada");
            return None;
        };

        let visto = (handler.quality(), handler.active_monitor());
        let (ordenes, recibe_ordenes) = std::sync::mpsc::channel();
        let (fotogramas, recibe_fotogramas) = mpsc::channel::<ScreenOutput>(FRAME_QUEUE_DEPTH);
        let fuente = Arc::clone(&self.capturers);
        let envia = fotogramas;
        let (perfil, monitor) = visto;

        // El reenvio va en su propia tarea y no en el bucle de la sesion. Si
        // estuviera en el bucle, un envio lento por la pista de video dejaria
        // de atender la senalizacion, el teclado y las transferencias: la
        // pantalla no puede congelar el raton.
        let reenvio = tokio::spawn(reenviar_pantalla(peer.sender(), pista, recibe_fotogramas));

        // Un hilo propio y no el grupo de bloqueo de tokio: esto no es una
        // tarea que acabe, es un trabajo que dura toda la sesion y tendria
        // ocupado un hilo del grupo de todas formas.
        let hilo = std::thread::Builder::new()
            .name("nexus-pantalla".to_owned())
            .spawn(move || {
                crate::screen::serve(fuente.as_ref(), monitor, &perfil, &recibe_ordenes, &envia);
            });

        match hilo {
            Ok(hilo) => Some(Bombeo {
                ordenes,
                hilo,
                reenvio,
                peticiones: handler.screen_requests(),
                visto,
            }),
            Err(error) => {
                tracing::error!(%error, "no se pudo crear el hilo de captura");
                reenvio.abort();
                None
            }
        }
    }

    /// Un mensaje de senalizacion dentro de una sesion.
    async fn on_signaling(
        &mut self,
        sig: &mut SignalingClient,
        peer: &mut PeerSession,
        estado: &mut SessionState,
        mensaje: ServerToClient,
    ) -> Result<Option<Motivo>, TransportError> {
        match mensaje {
            ServerToClient::Answer { sdp, .. } => {
                peer.accept_answer(&sdp).await?;
                estado.local = Some(peer.local_fingerprint().await?);
                estado.remote = Some(peer.remote_fingerprint().await?);
                if let Some(vista) = estado.binding() {
                    let prueba = vista.prove(&self.identity);
                    sig.send(ClientToServer::PeerAuth {
                        session_id: estado.session_id,
                        fingerprint: prueba.fingerprint,
                        signature_hex: prueba.signature_hex,
                    })
                    .await?;
                }
            }
            ServerToClient::IceCandidate {
                candidate, sdp_mid, ..
            } => {
                // Un camino que no se puede anadir **no** tumba la sesion. El
                // otro extremo puede mandar caminos antes de que su respuesta
                // llegue —los navegadores lo hacen— y entonces aqui todavia no
                // hay descripcion remota. Propagar ese error cerraba la
                // conexion con el servidor y obligaba a reconectar, por un
                // mensaje que solo habia que ignorar.
                if let Err(error) = peer.add_remote_candidate(candidate, sdp_mid).await {
                    tracing::debug!(%error, "camino descartado");
                }
            }
            ServerToClient::PeerAuth {
                device_id,
                fingerprint,
                signature_hex,
                ..
            } => {
                estado.proof = Some(BindingProof {
                    device_id,
                    fingerprint,
                    signature_hex,
                });
            }
            ServerToClient::PeerGone { .. } => {
                return Ok(Some((
                    CloseReason::PeerClosed,
                    "el otro extremo se desconecto",
                )));
            }
            otro => tracing::debug!("mensaje no esperado en sesion: {otro:?}"),
        }
        Ok(None)
    }

    /// Un evento de la conexion cifrada.
    async fn on_peer(
        &mut self,
        sig: &mut SignalingClient,
        peer: &PeerSession,
        estado: &mut SessionState,
        evento: PeerEvent,
        handler: &mut Option<SessionHandler>,
    ) -> Result<Option<Motivo>, TransportError> {
        match evento {
            PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                sig.send(ClientToServer::IceCandidate {
                    session_id: estado.session_id,
                    candidate,
                    sdp_mid,
                })
                .await?;
            }
            PeerEvent::Message { label, data } => {
                let Some(handler) = handler.as_mut() else {
                    // Todavia no se sabe quien hay al otro lado. Nada de lo que
                    // llegue puede tocar este equipo.
                    estado.dropped = estado.dropped.saturating_add(1);
                    tracing::warn!(
                        canal = %label,
                        "mensaje descartado: la sesion aun no esta autenticada"
                    );
                    return Ok(None);
                };

                // El canal de archivos lleva binario, no texto: se atiende
                // antes de intentar interpretarlo como UTF-8.
                if label == FILES_CHANNEL {
                    match handler.handle_file_frame(&data) {
                        Ok(Some(respuesta)) => {
                            peer.send(CONTROL_CHANNEL, respuesta.as_bytes()).await?;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            tracing::error!(%error, "fallo al atender un marco de archivo");
                        }
                    }
                    return Ok(None);
                }

                if label != INPUT_CHANNEL && label != CONTROL_CHANNEL {
                    tracing::warn!(canal = %label, "canal desconocido");
                    return Ok(None);
                }

                let Ok(texto) = std::str::from_utf8(&data) else {
                    tracing::warn!("el canal de control recibio algo que no es texto");
                    return Ok(None);
                };

                match handler.handle(texto) {
                    Ok(Some(respuesta)) => {
                        peer.send(CONTROL_CHANNEL, respuesta.as_bytes()).await?;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::error!(%error, "fallo al atender una orden");
                    }
                }
            }
            PeerEvent::Phase(LinkPhase::Failed) => {
                return Ok(Some((
                    CloseReason::NetworkLost,
                    "no se encontro ningun camino entre los dos extremos",
                )));
            }
            PeerEvent::Phase(LinkPhase::Closed) => {
                return Ok(Some((
                    CloseReason::PeerClosed,
                    "la conexion cifrada se cerro",
                )));
            }
            _ => {}
        }
        Ok(None)
    }

    /// Anota el fin de la sesion.
    ///
    /// Toma el recuento ya hecho y no el manejador: sostener una referencia al
    /// manejador mientras se espera obligaria a que fuese `Sync`, y el backend
    /// de entrada no tiene por que serlo.
    async fn end_session(
        &self,
        session_id: SessionId,
        dropped: u64,
        counts: (u64, u64),
        motivo: Motivo,
    ) {
        if dropped > 0 {
            tracing::warn!(
                descartados = dropped,
                "la sesion recibio datos antes de autenticarse"
            );
        }
        let (applied, refused) = counts;
        self.emit(RuntimeEvent::SessionEnded {
            session_id,
            reason: motivo.0,
            detail: motivo.1,
            applied,
            refused,
        })
        .await;
    }

    async fn emit(&self, event: RuntimeEvent) {
        // Si nadie escucha, el bucle sigue: los eventos son para la interfaz y
        // el historial, no para el funcionamiento.
        let _ = self.events.send(event).await;
    }
}

/// Por que termino una sesion: motivo para el historial y texto para la persona.
type Motivo = (CloseReason, &'static str);

/// Pide al hilo de captura lo que la sesion haya cambiado por el control.
///
/// Cambiar de calidad o de monitor lo tiene que aplicar el hilo que captura,
/// porque es el unico que puede tocar el capturador y el codificador.
fn sincronizar_pantalla(bombeo: Option<&mut Bombeo>, handler: Option<&SessionHandler>) {
    if let Some(bombeo) = bombeo
        && let Some(handler) = handler
    {
        bombeo.sincronizar(handler);
    }
}

/// Para todo lo que la sesion tenia en marcha, en el orden que importa.
///
/// El orden no es decorativo:
///
/// 1. Las subidas a medias tienen que borrar su archivo temporal antes de que
///    la sesion desaparezca.
/// 2. Las tareas de envio tienen que parar antes de cerrar la conexion.
/// 3. El bombeo se para al final, con su propio orden interno (ver
///    [`Bombeo::parar`]).
async fn cerrar_trabajos(
    handler: Option<&mut SessionHandler>,
    envios: &mut tokio::task::JoinSet<()>,
    bombeo: Option<Bombeo>,
) {
    if let Some(handler) = handler {
        handler.end();
    }
    envios.shutdown().await;
    if let Some(bombeo) = bombeo {
        bombeo.parar().await;
    }
}

/// El envio de la pantalla de una sesion, visto desde el bucle.
struct Bombeo {
    /// Por donde se le piden cambios al hilo que captura.
    ordenes: std::sync::mpsc::Sender<ScreenCommand>,
    /// El hilo que captura.
    hilo: std::thread::JoinHandle<()>,
    /// La tarea que saca los fotogramas por la pista de video.
    reenvio: tokio::task::JoinHandle<()>,
    /// Peticiones de pantalla ya atendidas.
    ///
    /// Pedirla otra vez con la pantalla en marcha significa "he perdido el
    /// hilo": lo que hace falta es un fotograma completo.
    peticiones: u64,
    /// Calidad y monitor que el bombeo ya conoce.
    ///
    /// Sin esto habria que repetirle la orden en cada vuelta del bucle, y cada
    /// cambio de calidad rehace el codificador y obliga a un fotograma
    /// completo: repetirlo sin motivo se veria como un parpadeo.
    visto: (QualityProfile, u8),
}

impl Bombeo {
    /// Pone al hilo de captura al dia con lo que la sesion pidio.
    fn sincronizar(&mut self, handler: &SessionHandler) {
        if handler.screen_requests() > self.peticiones {
            self.peticiones = handler.screen_requests();
            let _ = self.ordenes.send(ScreenCommand::Keyframe);
        }

        let ahora = (handler.quality(), handler.active_monitor());
        if ahora.0 != self.visto.0 {
            let _ = self.ordenes.send(ScreenCommand::Quality(ahora.0));
        }
        if ahora.1 != self.visto.1 {
            let _ = self.ordenes.send(ScreenCommand::Monitor(ahora.1));
        }
        self.visto = ahora;
    }

    /// Para el bombeo y espera a que todo lo suyo termine.
    ///
    /// El orden importa: primero se corta el reenvio, porque eso suelta la
    /// cola de fotogramas y desbloquea al hilo de captura si estuviera
    /// esperando sitio; si se hiciera al contrario, esperar al hilo podria no
    /// terminar nunca. Soltar el extremo de ordenes es la senal de parada, y
    /// la espera por el hilo va al grupo de bloqueo porque puede estar a mitad
    /// de codificar un fotograma.
    async fn parar(self) {
        self.reenvio.abort();
        let _ = self.reenvio.await;
        drop(self.ordenes);
        let _ = tokio::task::spawn_blocking(move || self.hilo.join()).await;
    }
}

/// Saca por la pista de video lo que va produciendo el hilo de captura.
///
/// Vive en su propia tarea a proposito: `send_frame` puede tardar lo que tarde
/// la red, y esa espera no puede caer sobre el bucle de la sesion, que es
/// quien atiende la senalizacion, el teclado y las transferencias.
///
/// Termina cuando el hilo de captura suelta su extremo, que es lo que ocurre
/// tanto al acabar la sesion como al romperse la captura.
async fn reenviar_pantalla(
    salida: PeerSender,
    pista: Arc<ScreenSender>,
    mut fotogramas: mpsc::Receiver<ScreenOutput>,
) {
    while let Some(cosa) = fotogramas.recv().await {
        match cosa {
            ScreenOutput::Frame {
                data,
                duration,
                keyframe,
            } => {
                if let Err(error) = pista.send_frame(&data, duration).await {
                    tracing::warn!(%error, keyframe, "no se pudo enviar un fotograma");
                }
            }
            ScreenOutput::Refused { detail } => {
                tracing::warn!(detail, "el hilo de captura no pudo aplicar una orden");
            }
            ScreenOutput::Broken { detail } => {
                tracing::error!(detail, "se dejo de enviar la pantalla");
                avisar_pantalla_parada(&salida, detail).await;
            }
        }
    }
}

/// Dice al otro extremo que la pantalla dejo de llegar, y por que.
///
/// Sin esto, la imagen se quedaria congelada y pareceria un problema de red.
async fn avisar_pantalla_parada(salida: &PeerSender, reason: String) {
    match SessionHandler::event(HostEvent::ScreenStopped { reason }) {
        Ok(aviso) => {
            if let Err(error) = salida.send(CONTROL_CHANNEL, aviso.as_bytes()).await {
                tracing::warn!(%error, "no se pudo avisar de que la pantalla paro");
            }
        }
        Err(error) => tracing::error!(%error, "no se pudo codificar el aviso de pantalla"),
    }
}

/// Ordenes aplicadas y rechazadas, o ceros si la sesion nunca llego a abrirse.
/// Envia por el canal de control el progreso que cuente una tarea de envio.
async fn anunciar_progreso(
    peer: &PeerSession,
    handler: Option<&mut SessionHandler>,
    reporte: &TransferReport,
) {
    let Some(handler) = handler else { return };
    let evento = match handler.handle_transfer_report(reporte) {
        Ok(Some(evento)) => evento,
        Ok(None) => return,
        Err(error) => {
            tracing::error!(%error, "no se pudo codificar un progreso");
            return;
        }
    };
    if let Err(error) = peer.send(CONTROL_CHANNEL, evento.as_bytes()).await {
        tracing::warn!(%error, "no se pudo enviar el progreso de una transferencia");
    }
}

/// Pone en marcha las descargas que el manejador dejo aceptadas.
fn arrancar_descargas(
    peer: &PeerSession,
    handler: Option<&mut SessionHandler>,
    reportes: &mpsc::Sender<TransferReport>,
    envios: &mut tokio::task::JoinSet<()>,
) {
    let Some(handler) = handler else { return };
    while let Some(descarga) = handler.take_pending_download() {
        envios.spawn(enviar_archivo(descarga, peer.sender(), reportes.clone()));
    }
}

/// Envia un archivo por el canal de archivos, trozo a trozo.
///
/// Corre en su propia tarea por una razon concreta: enviar dos gigabytes desde
/// el bucle que atiende el raton dejaria el raton congelado hasta terminar.
///
/// La velocidad la marca la red, no este bucle. `PeerSender::send` espera
/// cuando el canal tiene demasiados bytes pendientes de salir, asi que leer del
/// disco avanza al ritmo al que la red es capaz de vaciar: la memoria que usa
/// una descarga no depende del tamano del archivo.
async fn enviar_archivo(
    descarga: Download,
    envia: PeerSender,
    reportes: mpsc::Sender<TransferReport>,
) {
    let Download {
        transfer_id,
        reader,
        cancelled,
        ..
    } = descarga;
    let mut lector = reader;
    let mut enviados = 0_u64;

    loop {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = reportes
                .send(TransferReport {
                    transfer_id,
                    sent_bytes: enviados,
                    outcome: Some(TransferOutcome::Cancelled),
                })
                .await;
            return;
        }

        // Leer del disco bloquea el hilo. En un ejecutor asincrono eso frena
        // todo lo demas que ese hilo tuviera entre manos, asi que la lectura se
        // hace en el grupo de hilos que existe para eso. El lector va y vuelve
        // porque no se puede prestar a otro hilo.
        let resultado = tokio::task::spawn_blocking(move || {
            let siguiente = lector.next_frame();
            (siguiente, lector)
        })
        .await;

        let (siguiente, devuelto) = match resultado {
            Ok(par) => par,
            Err(error) => {
                let _ = reportes
                    .send(TransferReport {
                        transfer_id,
                        sent_bytes: enviados,
                        outcome: Some(TransferOutcome::Failed(error.to_string())),
                    })
                    .await;
                return;
            }
        };
        lector = devuelto;

        let marco = match siguiente {
            Ok(Some(marco)) => marco,
            // El marco de fin ya salio en la vuelta anterior, junto con su
            // informe: aqui no queda nada que decir.
            Ok(None) => return,
            Err(error) => {
                let _ = reportes
                    .send(TransferReport {
                        transfer_id,
                        sent_bytes: enviados,
                        outcome: Some(TransferOutcome::Failed(error.to_string())),
                    })
                    .await;
                return;
            }
        };

        let (es_final, bytes) = match &marco {
            Frame::Data { payload, .. } => (false, payload.len() as u64),
            Frame::End { .. } | Frame::Abort { .. } => (true, 0),
        };

        if let Err(error) = envia.send(FILES_CHANNEL, &marco.encode()).await {
            let _ = reportes
                .send(TransferReport {
                    transfer_id,
                    sent_bytes: enviados,
                    outcome: Some(TransferOutcome::Failed(error.to_string())),
                })
                .await;
            return;
        }
        enviados = enviados.saturating_add(bytes);

        if es_final {
            // El final si espera sitio en la cola: perderlo dejaria la
            // transferencia marcada como activa para siempre.
            let _ = reportes
                .send(TransferReport {
                    transfer_id,
                    sent_bytes: enviados,
                    outcome: Some(TransferOutcome::Completed),
                })
                .await;
            return;
        }

        // El progreso intermedio no espera: si la cola esta llena, mas vale
        // perder una actualizacion de la barra que frenar el envio. Por eso el
        // informe lleva el total y no el incremento.
        let _ = reportes.try_send(TransferReport {
            transfer_id,
            sent_bytes: enviados,
            outcome: None,
        });
    }
}

fn recuento(handler: Option<&SessionHandler>) -> (u64, u64) {
    handler.map_or((0, 0), |h| (h.applied(), h.refused()))
}

/// Lo que se va sabiendo de una sesion mientras se negocia.
struct SessionState {
    session_id: SessionId,
    controller: DeviceId,
    host: DeviceId,
    local: Option<nexus_security::DtlsFingerprint>,
    remote: Option<nexus_security::DtlsFingerprint>,
    proof: Option<BindingProof>,
    dropped: u64,
}

impl SessionState {
    const fn new(session_id: SessionId, controller: DeviceId, host: DeviceId) -> Self {
        Self {
            session_id,
            controller,
            host,
            local: None,
            remote: None,
            proof: None,
            dropped: 0,
        }
    }

    /// La vista de la atadura, si ya se conocen las dos huellas.
    fn binding(&self) -> Option<SessionBinding> {
        Some(SessionBinding {
            session_id: self.session_id,
            role: AuthRole::Host,
            local_device: self.host,
            remote_device: self.controller,
            local_fingerprint: self.local.clone()?,
            remote_fingerprint: self.remote.clone()?,
        })
    }

    /// Comprueba la prueba recibida, si ya estan todas las piezas.
    ///
    /// `None` mientras falte algo; `Some(false)` si no cuadra.
    fn verify(&self, peer_key: &DevicePublicKey) -> Option<bool> {
        let vista = self.binding()?;
        let prueba = self.proof.as_ref()?;
        match vista.verify_peer(prueba, peer_key) {
            Ok(()) => Some(true),
            Err(error) => {
                tracing::error!(%error, "la atadura de sesion no cuadra");
                Some(false)
            }
        }
    }
}

/// El backend de entrada de esta plataforma.
///
/// En Windows es el que llama a `SendInput`. En cualquier otra plataforma el
/// propio backend contesta que no esta admitida, de modo que la sesion se abre
/// y cada orden se rechaza con un motivo, en lugar de aceptarlas y no hacer
/// nada.
#[derive(Debug, Clone, Copy)]
pub struct PlatformInjectors;

impl InjectorSource for PlatformInjectors {
    fn create(&self) -> Box<dyn InputInjector + Send> {
        nexus_input::default_injector().unwrap_or_else(|error| {
            // Crear el backend fallo de verdad: no es que la plataforma no
            // este admitida. Se dice el motivo en cada orden en lugar de
            // fingir que la sesion controla algo.
            tracing::error!(%error, "no se pudo crear el backend de entrada");
            Box::new(UnavailableInjector {
                reason: error.to_string(),
            })
        })
    }
}

/// Backend que rechaza todo y explica por que.
#[derive(Debug)]
struct UnavailableInjector {
    reason: String,
}

impl InputInjector for UnavailableInjector {
    fn name(&self) -> &'static str {
        "no disponible"
    }

    fn inject(&mut self, _input: &nexus_input::ResolvedInput) -> nexus_input::InputResult<()> {
        Err(nexus_input::InputError::SystemRejected(self.reason.clone()))
    }

    fn release_all(&mut self) -> nexus_input::InputResult<()> {
        Ok(())
    }
}

/// Registro en memoria, para pruebas y para el arranque.
#[derive(Debug, Default)]
pub struct InMemoryLookup {
    devices: std::collections::HashMap<DeviceId, DeviceRecord>,
}

impl InMemoryLookup {
    /// Registro vacio.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Anade un dispositivo emparejado.
    pub fn insert(&mut self, record: DeviceRecord) {
        self.devices.insert(record.info.id, record);
    }
}

impl DeviceLookup for InMemoryLookup {
    fn get(&self, device_id: DeviceId) -> Option<DeviceRecord> {
        self.devices.get(&device_id).cloned()
    }
}

/// Inyector que anota, para pruebas y diagnostico.
#[derive(Debug, Clone, Default)]
pub struct RecordingInjectors(Arc<std::sync::Mutex<Vec<nexus_input::ResolvedInput>>>);

impl RecordingInjectors {
    /// Ordenes que han llegado, en orden.
    ///
    /// # Panics
    ///
    /// No entra en panico: un cerrojo envenenado se recupera.
    #[must_use]
    pub fn recorded(&self) -> Vec<nexus_input::ResolvedInput> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl InjectorSource for RecordingInjectors {
    fn create(&self) -> Box<dyn InputInjector + Send> {
        Box::new(Self(Arc::clone(&self.0)))
    }
}

impl InputInjector for RecordingInjectors {
    fn name(&self) -> &'static str {
        "registro"
    }

    fn inject(&mut self, input: &nexus_input::ResolvedInput) -> nexus_input::InputResult<()> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(input.clone());
        Ok(())
    }

    fn release_all(&mut self) -> nexus_input::InputResult<()> {
        Ok(())
    }
}

/// Marca temporal actual, para el historial.
#[must_use]
pub fn now() -> Timestamp {
    Timestamp::now()
}
