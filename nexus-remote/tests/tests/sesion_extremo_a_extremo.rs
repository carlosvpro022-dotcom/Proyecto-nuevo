//! La sesion completa, de principio a fin, con todo real.
//!
//! Esta prueba es la que demuestra que la etapa 3 funciona. No hay dobles de
//! prueba en ninguna capa:
//!
//! - El servidor de senalizacion es el de verdad, escuchando en un puerto de la
//!   maquina local, montado con la misma funcion que usa el binario.
//! - La autenticacion usa claves Ed25519 de verdad y se firma la transcripcion
//!   de verdad.
//! - La conexion entre los dos extremos es WebRTC de verdad: ICE descubre un
//!   camino, DTLS cifra, y los datos van por un canal SCTP.
//! - El manejador que recibe la orden al final es el del producto
//!   (`nexus_windows_host::session::SessionHandler`), no una copia escrita para
//!   la prueba: comprueba el permiso, resuelve a coordenadas absolutas y
//!   entrega al inyector.
//!
//! Lo unico que no es real es el inyector final, que anota en lugar de mover el
//! raton, porque esto corre en Linux. Y el relay TURN, que no esta desplegado:
//! aqui los dos extremos estan en la misma maquina y ICE encuentra un camino
//! directo. Esas dos cosas son lo que falta por comprobar en un PC con Windows
//! y con el servidor desplegado.

mod comun;

use std::time::Duration;

use comun::{Device, InyectorCompartido, TOKEN, TestServer};
use nexus_core::monitor::{MonitorInfo, MonitorLayout};
use nexus_core::protocol::Envelope;
use nexus_core::protocol::control::{ControlMessage, ControlRequest};
use nexus_core::protocol::input::{InputEvent, KeyState, PointerButton};
use nexus_core::protocol::signaling::{ClientToServer, ServerToClient};
use nexus_core::{DeviceId, PermissionSet, RequestId, SessionId};
use nexus_input::ResolvedInput;
use nexus_security::{
    AuthRole, BindingProof, DeviceIdentity, DevicePublicKey, SecurityError, SessionBinding,
};
use nexus_transport::client::SignalingClient;
use nexus_transport::peer::{
    CONTROL_CHANNEL, INPUT_CHANNEL, LinkPhase, PeerEvent, PeerRole, PeerSession,
};
use nexus_windows_host::session::SessionHandler;

/// Tope total de la prueba. ICE y DTLS tardan, pero no tanto.
const TIMEOUT: Duration = Duration::from_secs(45);

/// Huella con la que se simula la sustitucion de un servidor comprometido.
const HUELLA_DEL_ATACANTE: &str = "sha-256 \
    DE:AD:BE:EF:DE:AD:BE:EF:DE:AD:BE:EF:DE:AD:BE:EF:\
    DE:AD:BE:EF:DE:AD:BE:EF:DE:AD:BE:EF:DE:AD:BE:EF";

/// La pantalla que el host declara.
fn layout() -> MonitorLayout {
    MonitorLayout(vec![MonitorInfo {
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

/// El toque que el controlador envia: el centro exacto de la pantalla.
fn toque() -> [InputEvent; 3] {
    [
        InputEvent::PointerMove { x: 0.5, y: 0.5 },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: KeyState::Down,
        },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: KeyState::Up,
        },
    ]
}

/// Un extremo de la sesion con lo que necesita para sostenerla.
///
/// Los dos extremos hacen casi lo mismo —reenviar candidatos, firmar las
/// huellas, comprobar la del otro—, de modo que eso vive aqui una sola vez. Lo
/// que cambia es quien ofrece y que hace con los datos que recibe.
struct Extremo {
    sig: SignalingClient,
    peer: PeerSession,
    identity: DeviceIdentity,
    session_id: SessionId,
    role: AuthRole,
    yo: DeviceId,
    otro: DeviceId,
    clave_del_otro: DevicePublicKey,
    atadura: Option<SessionBinding>,
    prueba_recibida: Option<BindingProof>,
    autenticado: bool,
}

impl Extremo {
    /// Firma las dos huellas DTLS de la sesion y envia la prueba al otro lado.
    ///
    /// Solo se puede llamar cuando las dos descripciones estan puestas: antes
    /// no se conoce la huella del otro extremo.
    async fn enviar_atadura(&mut self) {
        let vista = SessionBinding {
            session_id: self.session_id,
            role: self.role,
            local_device: self.yo,
            remote_device: self.otro,
            local_fingerprint: self
                .peer
                .local_fingerprint()
                .await
                .expect("la descripcion local declara su huella DTLS"),
            remote_fingerprint: self
                .peer
                .remote_fingerprint()
                .await
                .expect("la descripcion remota declara su huella DTLS"),
        };
        let prueba = vista.prove(&self.identity);
        self.atadura = Some(vista);

        self.sig
            .send(ClientToServer::PeerAuth {
                session_id: self.session_id,
                fingerprint: prueba.fingerprint,
                signature_hex: prueba.signature_hex,
            })
            .await
            .expect("envia su atadura");
    }

    /// Reenvia al otro extremo un camino propio que descubrio ICE.
    async fn reenviar_candidato(&mut self, candidate: String, sdp_mid: Option<String>) {
        self.sig
            .send(ClientToServer::IceCandidate {
                session_id: self.session_id,
                candidate,
                sdp_mid,
            })
            .await
            .expect("envia su camino");
    }

    /// Guarda la prueba que llego del otro extremo.
    fn anotar_prueba(&mut self, device_id: DeviceId, fingerprint: String, signature_hex: String) {
        self.prueba_recibida = Some(BindingProof {
            device_id,
            fingerprint,
            signature_hex,
        });
    }

    /// Comprueba la atadura en cuanto se tienen las dos piezas.
    ///
    /// Y, con las huellas reales de esta sesion, comprueba tambien que una
    /// huella sustituida se rechaza: es lo que haria un servidor de
    /// senalizacion comprometido que intentase colocarse en medio.
    fn comprobar_atadura(&mut self) {
        if self.autenticado {
            return;
        }
        let (Some(vista), Some(prueba)) = (self.atadura.as_ref(), self.prueba_recibida.as_ref())
        else {
            return;
        };

        vista
            .verify_peer(prueba, &self.clave_del_otro)
            .expect("el otro extremo deberia quedar autenticado");

        let manipulada = BindingProof {
            fingerprint: HUELLA_DEL_ATACANTE.to_owned(),
            ..prueba.clone()
        };
        assert_eq!(
            vista.verify_peer(&manipulada, &self.clave_del_otro),
            Err(SecurityError::FingerprintMismatch),
            "sustituir la huella DTLS debe abortar la sesion"
        );

        self.autenticado = true;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn una_sesion_completa_del_ipad_al_escritorio() {
    let server = TestServer::start().await;
    let host = Device::host("PC del salon");
    let controller = Device::controller("iPad Pro");

    // Cada extremo conoce de antemano la clave publica del otro. En el producto
    // el host la tiene en su registro de dispositivos emparejados, y el
    // controlador la recibio en la invitacion, que viaja fuera de banda. Lo que
    // importa es que **ninguno la obtiene del servidor**.
    let clave_del_host = host.identity.public_key();
    let clave_del_controlador = controller.identity.public_key();

    let trabajo = async {
        tokio::join!(
            lado_host(&server, &host, controller.id(), clave_del_controlador),
            lado_controlador(&server, &controller, host.id(), clave_del_host),
        )
    };

    let Ok((inyectado, canal_abierto)) = tokio::time::timeout(TIMEOUT, Box::pin(trabajo)).await
    else {
        panic!(
            "la sesion no se completo en {} s; si ICE no encuentra camino en \
             bucle local, el problema esta en la configuracion de red del entorno",
            TIMEOUT.as_secs()
        );
    };

    assert!(
        canal_abierto,
        "el canal de datos cifrado deberia haberse abierto"
    );

    // La prueba de que la sesion sirve para algo: el toque llego entero y en
    // orden, con las coordenadas correctas.
    assert_eq!(
        inyectado,
        [
            ResolvedInput::MoveCursor { x: 960, y: 540 },
            ResolvedInput::Button {
                button: PointerButton::Left,
                state: KeyState::Down
            },
            ResolvedInput::Button {
                button: PointerButton::Left,
                state: KeyState::Up
            },
        ],
        "el centro de la pantalla del iPad debe caer en el centro del escritorio"
    );

    // El servidor supo quien hablaba con quien, y nada mas.
    let lineas = server.audit_lines();
    assert!(
        lineas.iter().any(|l| l.contains("atadura")),
        "la atadura de sesion deberia constar en el registro: {lineas:?}"
    );
    assert!(
        lineas.iter().all(|l| !l.contains("v=0")),
        "el servidor no debe volcar descripciones SDP en su registro: {lineas:?}"
    );
}

/// El lado del host: ofrece, y obedece lo que sus permisos permitan.
///
/// Devuelve lo que acabo llegando al inyector.
async fn lado_host(
    server: &TestServer,
    me: &Device,
    controller_id: DeviceId,
    clave_del_controlador: DevicePublicKey,
) -> Vec<ResolvedInput> {
    let (mut sig, accepted) = me.connect_ok(server, Some(TOKEN)).await;
    sig.register_host(1).await.expect("queda disponible");

    let session_id = match sig.recv().await.expect("el canal sigue abierto") {
        ServerToClient::SessionRequested { session_id, .. } => session_id,
        otro => panic!("el host esperaba una peticion de sesion y llego {otro:?}"),
    };

    // El host decide los permisos. Da control de raton, no da administracion.
    sig.send(ClientToServer::AcceptSession {
        session_id,
        negotiated_capabilities: me.capabilities.clone(),
        effective_permissions: PermissionSet::standard(),
    })
    .await
    .expect("acepta la sesion");

    let peer = PeerSession::open(&accepted.ice, PeerRole::Offerer)
        .await
        .expect("abre la conexion WebRTC");
    let offer = peer.create_offer().await.expect("crea la oferta");
    sig.send(ClientToServer::Offer {
        session_id,
        sdp: offer,
    })
    .await
    .expect("envia la oferta");

    let mut extremo = Extremo {
        sig,
        peer,
        identity: me.duplicate_identity(),
        session_id,
        role: AuthRole::Host,
        yo: me.id(),
        otro: controller_id,
        clave_del_otro: clave_del_controlador,
        atadura: None,
        prueba_recibida: None,
        autenticado: false,
    };

    // El manejador de sesion es el del producto.
    let injector = InyectorCompartido::default();
    let mut handler = SessionHandler::new(
        PermissionSet::standard(),
        layout(),
        Box::new(injector.clone()),
    );

    loop {
        tokio::select! {
            mensaje = extremo.sig.recv() => {
                let mensaje = mensaje.expect("el canal de senalizacion sigue abierto");
                if atender_senalizacion_host(&mut extremo, mensaje).await {
                    break;
                }
            }
            evento = extremo.peer.next_event() => {
                let evento = evento.expect("la conexion sigue viva");
                atender_conexion(&mut extremo, evento, &mut handler).await;
            }
        }

        extremo.comprobar_atadura();
        if extremo.autenticado && injector.recibidas().len() == toque().len() {
            break;
        }
    }

    let _ = extremo.sig.send(ClientToServer::Bye { session_id }).await;
    let _ = extremo.peer.close().await;
    injector.recibidas()
}

/// Atiende un mensaje de senalizacion en el host. Devuelve `true` si hay que parar.
async fn atender_senalizacion_host(extremo: &mut Extremo, mensaje: ServerToClient) -> bool {
    match mensaje {
        ServerToClient::Answer { sdp, .. } => {
            extremo
                .peer
                .accept_answer(&sdp)
                .await
                .expect("acepta la respuesta");
            extremo.enviar_atadura().await;
        }
        ServerToClient::IceCandidate {
            candidate, sdp_mid, ..
        } => extremo
            .peer
            .add_remote_candidate(candidate, sdp_mid)
            .await
            .expect("anade el camino del otro extremo"),
        ServerToClient::PeerAuth {
            device_id,
            fingerprint,
            signature_hex,
            ..
        } => extremo.anotar_prueba(device_id, fingerprint, signature_hex),
        ServerToClient::PeerGone { .. } => return true,
        _ => {}
    }
    false
}

/// Atiende un evento de la conexion cifrada en el host.
async fn atender_conexion(extremo: &mut Extremo, evento: PeerEvent, handler: &mut SessionHandler) {
    match evento {
        PeerEvent::LocalCandidate { candidate, sdp_mid } => {
            extremo.reenviar_candidato(candidate, sdp_mid).await;
        }
        PeerEvent::Message { label, data } => {
            assert_eq!(label, INPUT_CHANNEL, "solo se espera entrada");
            let texto = std::str::from_utf8(&data).expect("el canal transporta texto");
            if let Some(respuesta) = handler.handle(texto).expect("el manejador no debe fallar") {
                extremo
                    .peer
                    .send(CONTROL_CHANNEL, respuesta.as_bytes())
                    .await
                    .expect("contesta por el canal de control");
            }
        }
        PeerEvent::Phase(LinkPhase::Failed) => {
            panic!("ICE no encontro ningun camino entre los dos extremos");
        }
        _ => {}
    }
}

/// El lado del controlador: pide sesion, contesta la oferta y envia el toque.
///
/// Devuelve si el canal de datos llego a abrirse.
async fn lado_controlador(
    server: &TestServer,
    me: &Device,
    host_id: DeviceId,
    clave_del_host: DevicePublicKey,
) -> bool {
    let (mut sig, accepted) = me.connect_ok(server, None).await;
    let session_id = pedir_sesion(&mut sig, host_id).await;

    let peer = PeerSession::open(&accepted.ice, PeerRole::Answerer)
        .await
        .expect("abre la conexion WebRTC");

    let mut extremo = Extremo {
        sig,
        peer,
        identity: me.duplicate_identity(),
        session_id,
        role: AuthRole::Controller,
        yo: me.id(),
        otro: host_id,
        clave_del_otro: clave_del_host,
        atadura: None,
        prueba_recibida: None,
        autenticado: false,
    };

    let mut canal_abierto = false;

    loop {
        tokio::select! {
            mensaje = extremo.sig.recv() => {
                let mensaje = mensaje.expect("el canal de senalizacion sigue abierto");
                if atender_senalizacion_controlador(&mut extremo, mensaje).await {
                    break;
                }
            }
            evento = extremo.peer.next_event() => {
                match evento.expect("la conexion sigue viva") {
                    PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                        extremo.reenviar_candidato(candidate, sdp_mid).await;
                    }
                    PeerEvent::ChannelOpen { label } if label == INPUT_CHANNEL => {
                        canal_abierto = true;
                    }
                    PeerEvent::Phase(LinkPhase::Failed) => {
                        panic!("ICE no encontro ningun camino entre los dos extremos");
                    }
                    _ => {}
                }
            }
        }

        extremo.comprobar_atadura();

        // El toque se envia solo cuando el canal esta abierto y el host quedo
        // autenticado: enviar pulsaciones antes de saber con quien se habla
        // seria exactamente el fallo que esta etapa cierra.
        if canal_abierto && extremo.autenticado {
            enviar_toque(&extremo).await;
            break;
        }
    }

    // Deja que el host termine de recibir antes de cerrar el canal.
    tokio::time::sleep(Duration::from_millis(500)).await;
    canal_abierto
}

/// Atiende un mensaje de senalizacion en el controlador.
async fn atender_senalizacion_controlador(extremo: &mut Extremo, mensaje: ServerToClient) -> bool {
    match mensaje {
        ServerToClient::Offer { sdp, .. } => {
            let answer = extremo
                .peer
                .accept_offer(&sdp)
                .await
                .expect("contesta la oferta");
            let session_id = extremo.session_id;
            extremo
                .sig
                .send(ClientToServer::Answer {
                    session_id,
                    sdp: answer,
                })
                .await
                .expect("envia la respuesta");
            extremo.enviar_atadura().await;
        }
        ServerToClient::IceCandidate {
            candidate, sdp_mid, ..
        } => extremo
            .peer
            .add_remote_candidate(candidate, sdp_mid)
            .await
            .expect("anade el camino del otro extremo"),
        ServerToClient::PeerAuth {
            device_id,
            fingerprint,
            signature_hex,
            ..
        } => extremo.anotar_prueba(device_id, fingerprint, signature_hex),
        ServerToClient::SessionRejected { reason, .. } => {
            panic!("el host rechazo la sesion: {reason}")
        }
        ServerToClient::PeerGone { .. } => return true,
        _ => {}
    }
    false
}

/// Envia el toque completo por el canal de entrada.
async fn enviar_toque(extremo: &Extremo) {
    for event in toque() {
        let mensaje = Envelope::new(ControlMessage::request(
            RequestId::FIRST,
            ControlRequest::Input { event },
        ));
        let raw = mensaje.encode().expect("codifica el evento");
        extremo
            .peer
            .send(INPUT_CHANNEL, raw.as_bytes())
            .await
            .expect("envia el evento por el canal cifrado");
    }
}

/// Pide sesion, reintentando mientras el host todavia no este disponible.
async fn pedir_sesion(sig: &mut SignalingClient, host_id: DeviceId) -> SessionId {
    for _ in 0..100 {
        sig.send(ClientToServer::RequestSession {
            host_id,
            requested_permissions: PermissionSet::full(),
        })
        .await
        .expect("pide sesion");

        match sig.recv().await.expect("el canal sigue abierto") {
            ServerToClient::SessionAccepted { session_id, .. } => return session_id,
            ServerToClient::Error { code, .. } if code == "host_unavailable" => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            ServerToClient::Error { code, message } => {
                panic!("el servidor rechazo la peticion: {code} ({message})");
            }
            otro => panic!("se esperaba la resolucion de la sesion y llego {otro:?}"),
        }
    }
    panic!("el host no llego a estar disponible");
}
