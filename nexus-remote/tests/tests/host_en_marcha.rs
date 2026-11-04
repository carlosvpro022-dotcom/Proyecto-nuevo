//! El host atendiendo sesiones de verdad.
//!
//! La prueba anterior (`sesion_extremo_a_extremo`) demostraba que las piezas
//! encajan, pero el lado del host lo conducia la propia prueba. Aqui el host es
//! el bucle del producto: `HostRuntime` se conecta solo, decide solo contra su
//! registro de dispositivos, negocia solo y sirve solo.
//!
//! Lo que se comprueba es lo que de verdad importa de ese bucle:
//!
//! - Que un dispositivo emparejado con acceso desatendido entra y le funciona
//!   el raton.
//! - Que uno que no esta emparejado no entra, y que el motivo no le dice nada.
//! - Que uno revocado se rechaza igual que uno desconocido.
//! - Que uno sin acceso desatendido no entra solo.
//! - Que no se concede mas de lo concedido, aunque lo pida.
//! - Y lo mas importante: que **nada toca el equipo antes de comprobar quien
//!   hay al otro lado**.

mod comun;

use std::time::Duration;

use comun::{Device, HostDeProducto, STEP_TIMEOUT, TestServer};
use nexus_core::protocol::Envelope;
use nexus_core::protocol::control::{ControlMessage, ControlRequest};
use nexus_core::protocol::input::{InputEvent, KeyState, PointerButton};
use nexus_core::protocol::signaling::{ClientToServer, ServerToClient};
use nexus_core::{DeviceId, Permission, PermissionSet, RequestId, SessionId};
use nexus_input::ResolvedInput;
use nexus_security::registry::DeviceRecord;
use nexus_security::{AuthRole, DeviceIdentity, SessionBinding};
use nexus_transport::client::SignalingClient;
use nexus_transport::peer::{INPUT_CHANNEL, PeerEvent, PeerRole, PeerSession};
use nexus_windows_host::runtime::RuntimeEvent;

const TIMEOUT: Duration = Duration::from_secs(45);

/// Un controlador que conduce su lado de la sesion.
struct Controlador {
    device: Device,
    sig: SignalingClient,
    /// La configuracion ICE de **esta** conexion.
    ///
    /// Se guarda aqui y no se vuelve a pedir: abrir una segunda conexion del
    /// mismo dispositivo desplazaria a la primera, que es justo lo que el
    /// servidor hace a proposito cuando un movil reconecta.
    ice: nexus_transport::IceConfiguration,
}

impl Controlador {
    async fn conectar(server: &TestServer, device: Device) -> Self {
        let (sig, aceptado) = device.connect_ok(server, None).await;
        Self {
            device,
            sig,
            ice: aceptado.ice,
        }
    }

    /// Pide sesion y devuelve lo que conteste el host.
    async fn pedir(&mut self, host: DeviceId, permisos: PermissionSet) -> ServerToClient {
        self.sig
            .send(ClientToServer::RequestSession {
                host_id: host,
                requested_permissions: permisos,
            })
            .await
            .expect("pide sesion");
        self.sig
            .recv_timeout(STEP_TIMEOUT)
            .await
            .expect("el host deberia contestar")
    }

    /// Lleva la sesion hasta enviar un toque. `firmante` es quien firma la
    /// atadura: normalmente el propio controlador.
    ///
    /// Devuelve si llego a enviarlo. Puede que no: si el host no reconoce la
    /// firma, corta la sesion antes de que el canal de entrada llegue a
    /// abrirse, que es exactamente lo que debe pasar.
    async fn sesionar(
        &mut self,
        session_id: SessionId,
        host: DeviceId,
        firmante: &DeviceIdentity,
    ) -> bool {
        let ice = self.ice.clone();
        let mut peer = PeerSession::open(&ice, PeerRole::Answerer)
            .await
            .expect("abre la conexion");
        let mut enviado = false;

        loop {
            tokio::select! {
                mensaje = self.sig.recv() => {
                    match mensaje.expect("el canal sigue abierto") {
                        ServerToClient::Offer { sdp, .. } => {
                            let answer = peer.accept_offer(&sdp).await.expect("contesta");
                            self.sig
                                .send(ClientToServer::Answer { session_id, sdp: answer })
                                .await
                                .expect("envia la respuesta");

                            let vista = SessionBinding {
                                session_id,
                                role: AuthRole::Controller,
                                local_device: self.device.id(),
                                remote_device: host,
                                local_fingerprint: peer
                                    .local_fingerprint()
                                    .await
                                    .expect("huella local"),
                                remote_fingerprint: peer
                                    .remote_fingerprint()
                                    .await
                                    .expect("huella remota"),
                            };
                            let prueba = vista.prove(firmante);
                            self.sig
                                .send(ClientToServer::PeerAuth {
                                    session_id,
                                    fingerprint: prueba.fingerprint,
                                    signature_hex: prueba.signature_hex,
                                })
                                .await
                                .expect("envia la atadura");
                        }
                        ServerToClient::IceCandidate { candidate, sdp_mid, .. } => {
                            peer.add_remote_candidate(candidate, sdp_mid)
                                .await
                                .expect("anade camino");
                        }
                        ServerToClient::PeerGone { .. } => break,
                        _ => {}
                    }
                }
                evento = peer.next_event() => {
                    match evento.expect("la conexion sigue viva") {
                        PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                            self.sig
                                .send(ClientToServer::IceCandidate {
                                    session_id,
                                    candidate,
                                    sdp_mid,
                                })
                                .await
                                .expect("envia camino");
                        }
                        PeerEvent::ChannelOpen { label } if label == INPUT_CHANNEL => {
                            for event in toque() {
                                let sobre = Envelope::new(ControlMessage::request(
                                    RequestId::FIRST,
                                    ControlRequest::Input { event },
                                ));
                                let raw = sobre.encode().expect("codifica");
                                peer.send(INPUT_CHANNEL, raw.as_bytes())
                                    .await
                                    .expect("envia el toque");
                            }
                            enviado = true;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Deja al host recibir antes de cerrar.
        tokio::time::sleep(Duration::from_millis(600)).await;
        let _ = peer.close().await;
        enviado
    }
}

fn toque() -> [InputEvent; 2] {
    [
        InputEvent::PointerMove { x: 0.5, y: 0.5 },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: KeyState::Down,
        },
    ]
}

/// Emparejado: lo que el host guardaria tras un emparejamiento correcto.
fn emparejado(device: &Device, permisos: PermissionSet, desatendido: bool) -> DeviceRecord {
    DeviceRecord::new(
        device.info.clone(),
        device.identity.public_key(),
        permisos,
        desatendido,
        nexus_core::Timestamp::now(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn un_dispositivo_emparejado_entra_y_mueve_el_raton() {
    let server = TestServer::start().await;
    let ipad = Device::controller("iPad Pro");
    let mut host = HostDeProducto::arrancar(
        &server,
        vec![emparejado(&ipad, PermissionSet::standard(), true)],
    )
    .await;

    let mut controlador = Controlador::conectar(&server, ipad).await;
    let respuesta = controlador
        .pedir(host.id(), PermissionSet::standard())
        .await;

    let (session_id, permisos) = match respuesta {
        ServerToClient::SessionAccepted {
            session_id,
            effective_permissions,
            ..
        } => (session_id, effective_permissions),
        otro => panic!("deberia aceptarse, llego {otro:?}"),
    };
    assert!(permisos.has(Permission::ControlPointer));

    match host.siguiente_evento().await {
        RuntimeEvent::SessionDecided { accepted, .. } => assert!(accepted),
        otro => panic!("se esperaba la decision, llego {otro:?}"),
    }

    let identidad = controlador.device.duplicate_identity();
    let enviado = tokio::time::timeout(TIMEOUT, async {
        controlador
            .sesionar(session_id, host.id(), &identidad)
            .await
    })
    .await
    .expect("la sesion deberia completarse");
    assert!(enviado, "el canal de entrada deberia haberse abierto");

    match host.siguiente_evento().await {
        RuntimeEvent::SessionStarted { controller, .. } => {
            assert_eq!(controller, controlador.device.id());
        }
        otro => panic!("la sesion deberia quedar autenticada, llego {otro:?}"),
    }

    assert_eq!(
        host.inyector.recorded(),
        [
            ResolvedInput::MoveCursor { x: 960, y: 540 },
            ResolvedInput::Button {
                button: PointerButton::Left,
                state: KeyState::Down
            },
        ],
        "el toque del iPad debe llegar al escritorio"
    );

    host.parar().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn un_dispositivo_sin_emparejar_no_entra() {
    let server = TestServer::start().await;
    let mut host = HostDeProducto::arrancar(&server, vec![]).await;

    let desconocido = Device::controller("iPhone de un desconocido");
    let mut controlador = Controlador::conectar(&server, desconocido).await;

    match controlador.pedir(host.id(), PermissionSet::full()).await {
        ServerToClient::SessionRejected { reason, .. } => {
            assert!(
                !reason.contains("emparejad") && !reason.contains("revocad"),
                "el motivo no debe decirle nada del registro: {reason}"
            );
        }
        otro => panic!("deberia rechazarse, llego {otro:?}"),
    }

    match host.siguiente_evento().await {
        RuntimeEvent::SessionDecided { accepted, .. } => assert!(!accepted),
        otro => panic!("se esperaba la decision, llego {otro:?}"),
    }
    assert!(host.inyector.recorded().is_empty());
    host.parar().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn un_dispositivo_revocado_se_rechaza_igual_que_uno_desconocido() {
    let server = TestServer::start().await;
    let ipad = Device::controller("iPad revocado");
    let mut revocado = emparejado(&ipad, PermissionSet::full(), true);
    revocado.revoked_at = Some(nexus_core::Timestamp::now());

    let host = HostDeProducto::arrancar(&server, vec![revocado]).await;
    let mut controlador = Controlador::conectar(&server, ipad).await;

    match controlador.pedir(host.id(), PermissionSet::full()).await {
        ServerToClient::SessionRejected { .. } => {}
        otro => panic!("deberia rechazarse, llego {otro:?}"),
    }
    host.parar().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sin_acceso_desatendido_el_host_lo_dice_en_lugar_de_dejar_esperando() {
    let server = TestServer::start().await;
    let ipad = Device::controller("iPad sin desatendido");
    let host = HostDeProducto::arrancar(
        &server,
        vec![emparejado(&ipad, PermissionSet::standard(), false)],
    )
    .await;

    let mut controlador = Controlador::conectar(&server, ipad).await;
    match controlador
        .pedir(host.id(), PermissionSet::standard())
        .await
    {
        ServerToClient::SessionRejected { reason, .. } => {
            assert!(
                reason.contains("aceptar la sesion en el PC"),
                "el usuario tiene que entender que le falta: {reason}"
            );
        }
        otro => panic!("deberia rechazarse con explicacion, llego {otro:?}"),
    }
    host.parar().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_se_concede_mas_de_lo_concedido_aunque_se_pida() {
    let server = TestServer::start().await;
    let ipad = Device::controller("iPad solo mirar");
    let host = HostDeProducto::arrancar(
        &server,
        vec![emparejado(&ipad, PermissionSet::view_only(), true)],
    )
    .await;

    let mut controlador = Controlador::conectar(&server, ipad).await;
    match controlador.pedir(host.id(), PermissionSet::full()).await {
        ServerToClient::SessionAccepted {
            effective_permissions,
            ..
        } => {
            assert!(effective_permissions.has(Permission::ViewScreen));
            assert!(
                !effective_permissions.has(Permission::ControlPointer),
                "pidio todo y tiene concedido solo ver"
            );
            assert!(!effective_permissions.has(Permission::Shutdown));
        }
        otro => panic!("deberia aceptarse con menos permisos, llego {otro:?}"),
    }
    host.parar().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn nada_toca_el_equipo_si_la_atadura_no_cuadra() {
    // El escenario: el dispositivo esta emparejado, pero quien negocia la
    // sesion firma con otra clave. Es lo que pasaria si alguien se hubiese
    // colocado en medio. La sesion llega a establecerse por abajo, pero el host
    // no debe aplicar ni una sola orden.
    let server = TestServer::start().await;
    let ipad = Device::controller("iPad Pro");
    let mut host = HostDeProducto::arrancar(
        &server,
        vec![emparejado(&ipad, PermissionSet::full(), true)],
    )
    .await;

    let mut controlador = Controlador::conectar(&server, ipad).await;
    let session_id = match controlador.pedir(host.id(), PermissionSet::full()).await {
        ServerToClient::SessionAccepted { session_id, .. } => session_id,
        otro => panic!("deberia aceptarse, llego {otro:?}"),
    };
    match host.siguiente_evento().await {
        RuntimeEvent::SessionDecided { accepted, .. } => assert!(accepted),
        otro => panic!("se esperaba la decision, llego {otro:?}"),
    }

    // Firma un impostor, no el dispositivo emparejado.
    // Puede que ni llegue a abrirse el canal: el host corta en cuanto ve que la
    // firma no cuadra, y eso es lo deseable.
    let impostor = DeviceIdentity::generate(controlador.device.id());
    let _ = tokio::time::timeout(TIMEOUT, async {
        controlador.sesionar(session_id, host.id(), &impostor).await
    })
    .await;

    // El host tiene que haber cerrado la sesion sin empezarla.
    let fin = host.siguiente_evento().await;
    match fin {
        RuntimeEvent::SessionEnded {
            reason,
            detail,
            applied,
            ..
        } => {
            assert!(
                matches!(
                    reason,
                    nexus_core::CloseReason::AuthenticationFailed
                        | nexus_core::CloseReason::PeerClosed
                ),
                "motivo inesperado: {detail}"
            );
            assert_eq!(applied, 0, "no debe aplicarse ni una orden");
        }
        RuntimeEvent::SessionStarted { .. } => {
            panic!("la sesion no debe declararse iniciada con una firma que no cuadra")
        }
        otro => panic!("se esperaba el fin de la sesion, llego {otro:?}"),
    }

    assert!(
        host.inyector.recorded().is_empty(),
        "nada debe haber tocado el equipo"
    );
    host.parar().await;
}
