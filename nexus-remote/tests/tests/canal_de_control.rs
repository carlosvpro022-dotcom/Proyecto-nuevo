//! Todo el canal de control, sobre una conexion de verdad.
//!
//! Las demas pruebas de integracion comprueban a fondo una operacion cada una.
//! Esta comprueba **todas**, y lo que busca es distinto: que ninguna se quede
//! sin contestar, que ninguna tire la sesion, y que la respuesta que llega sea
//! la que el protocolo dice, ya serializada, enviada por un canal SCTP real y
//! vuelta a leer al otro lado.
//!
//! Es el tipo de fallo que las pruebas unitarias del manejador no pueden ver:
//! alli la respuesta se construye y se mira directamente, sin pasar por el
//! sobre, por el canal, ni por el limite de tamano del mensaje.
//!
//! Lo que **no** comprueba es el comportamiento de cada operacion a fondo; eso
//! esta en las pruebas de cada modulo. Aqui lo que importa es que el camino
//! completo funcione para todas.

mod comun;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use comun::{Device, STEP_TIMEOUT, TestServer};
use nexus_core::protocol::Envelope;
use nexus_core::protocol::control::{ControlMessage, ControlRequest, ControlResponse, PowerAction};
use nexus_core::protocol::signaling::{ClientToServer, ServerToClient};
use nexus_core::{
    DeviceId, PermissionSet, QualityPreset, QualityProfile, RequestId, SessionId, TransferId,
};
use nexus_file_transfer::SharedRoot;
use nexus_security::DeviceIdentity;
use nexus_security::registry::DeviceRecord;
use nexus_transport::client::SignalingClient;
use nexus_transport::peer::{CONTROL_CHANNEL, PeerEvent, PeerRole, PeerSession};

const TIMEOUT: Duration = Duration::from_secs(60);

/// Lo que se espera de cada respuesta.
///
/// No se compara la respuesta entera: varias llevan datos del equipo que
/// cambian entre maquinas. Lo que se comprueba es que sea **la forma
/// correcta**, y en los rechazos, el codigo exacto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Forma {
    /// Aceptada sin datos.
    Ack,
    /// Lista de monitores.
    Monitores,
    /// Estado del equipo.
    Estado,
    /// Listado de un directorio.
    Listado,
    /// Transferencia aceptada.
    Transferencia,
    /// Hace falta confirmacion en el host.
    Confirmacion,
    /// Rechazada, con este codigo.
    Rechazo(&'static str),
    /// Rechazada por cualquiera de estos dos codigos.
    ///
    /// Hace falta donde la respuesta correcta depende de la plataforma: en
    /// Linux no hay portapapeles ni energia, y en Windows si.
    RechazoDe(&'static str, &'static str),
}

impl Forma {
    /// Comprueba una respuesta y devuelve que falla, si falla.
    fn comprobar(self, response: &ControlResponse) -> Result<(), String> {
        let cuadra = match (self, response) {
            (Self::Ack, ControlResponse::Ack)
            | (Self::Monitores, ControlResponse::Monitors { .. })
            | (Self::Estado, ControlResponse::SystemStatus { .. })
            | (Self::Listado, ControlResponse::DirListing { .. })
            | (Self::Transferencia, ControlResponse::TransferAccepted { .. })
            | (Self::Confirmacion, ControlResponse::ConfirmationRequired { .. }) => true,
            (Self::Rechazo(esperado), ControlResponse::Denied { code, .. }) => code == esperado,
            (Self::RechazoDe(a, b), ControlResponse::Denied { code, .. }) => code == a || code == b,
            _ => false,
        };
        if cuadra {
            Ok(())
        } else {
            Err(format!("se esperaba {self:?} y llego {response:?}"))
        }
    }
}

/// Una carpeta compartida con un archivo dentro, y otra que queda fuera.
struct Carpetas {
    _temp: tempfile::TempDir,
    compartida: PathBuf,
    fuera: PathBuf,
}

impl Carpetas {
    fn preparar() -> Self {
        let temp = tempfile::tempdir().expect("temporal");
        let compartida = temp.path().join("compartida");
        let fuera = temp.path().join("privado");
        for dir in [&compartida, &fuera] {
            std::fs::create_dir_all(dir).expect("crea");
        }
        std::fs::write(compartida.join("dato.txt"), b"contenido").expect("crea");
        Self {
            _temp: temp,
            compartida,
            fuera,
        }
    }
}

fn ruta(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// El guion: cada operacion del protocolo con la forma que debe contestar.
///
/// `Input` no esta porque no lleva respuesta —la pantalla es la respuesta— y
/// ya se comprueba en `host_en_marcha`.
fn guion(c: &Carpetas) -> Vec<(&'static str, ControlRequest, Forma)> {
    let mut todo = pantalla_y_calidad();
    todo.extend(archivos(c));
    todo.extend(sistema_y_energia());
    todo
}

/// Lo que se puede pedir sobre la pantalla.
fn pantalla_y_calidad() -> Vec<(&'static str, ControlRequest, Forma)> {
    vec![
        (
            "listar monitores",
            ControlRequest::ListMonitors,
            Forma::Monitores,
        ),
        (
            "elegir monitor",
            ControlRequest::SelectMonitor { index: 0 },
            Forma::Ack,
        ),
        (
            "elegir un monitor que no existe",
            ControlRequest::SelectMonitor { index: 9 },
            Forma::Rechazo("unknown_monitor"),
        ),
        (
            "calidad valida",
            ControlRequest::SetQuality {
                profile: QualityProfile::from_preset(QualityPreset::Fluido),
            },
            Forma::Ack,
        ),
        (
            "calidad imposible",
            ControlRequest::SetQuality {
                profile: QualityProfile {
                    target_fps: 10_000,
                    ..QualityProfile::from_preset(QualityPreset::Fluido)
                },
            },
            Forma::Rechazo("invalid_quality"),
        ),
    ]
}

/// Lo que se puede pedir sobre los archivos compartidos.
fn archivos(c: &Carpetas) -> Vec<(&'static str, ControlRequest, Forma)> {
    vec![
        (
            "listar la carpeta compartida",
            ControlRequest::ListDir {
                path: ruta(&c.compartida),
                offset: 0,
            },
            Forma::Listado,
        ),
        (
            "listar una carpeta de fuera",
            ControlRequest::ListDir {
                path: ruta(&c.fuera),
                offset: 0,
            },
            Forma::Rechazo("access_denied"),
        ),
        (
            "descargar un archivo compartido",
            ControlRequest::DownloadStart {
                path: ruta(&c.compartida.join("dato.txt")),
            },
            Forma::Transferencia,
        ),
        (
            "descargar algo de fuera",
            ControlRequest::DownloadStart {
                path: ruta(&c.fuera.join("secreto.txt")),
            },
            Forma::Rechazo("access_denied"),
        ),
        (
            "subir a la carpeta compartida",
            ControlRequest::UploadStart {
                path: ruta(&c.compartida.join("nuevo.bin")),
                size_bytes: 10,
            },
            Forma::Transferencia,
        ),
        (
            "cancelar algo que no existe",
            ControlRequest::TransferCancel {
                transfer_id: TransferId::new(),
            },
            Forma::Rechazo("unknown_transfer"),
        ),
    ]
}

/// Lo que se puede pedir sobre el equipo: estado, portapapeles y energia.
fn sistema_y_energia() -> Vec<(&'static str, ControlRequest, Forma)> {
    vec![
        (
            "estado del equipo",
            ControlRequest::SystemStatus,
            Forma::Estado,
        ),
        (
            "leer el portapapeles",
            ControlRequest::ClipboardGet,
            // En Linux no hay backend; en Windows lo normal es que responda,
            // y si esta vacio lo dice.
            Forma::RechazoDe("not_implemented", "clipboard_empty"),
        ),
        (
            "escribir el portapapeles",
            ControlRequest::ClipboardSet {
                text: "desde la prueba".to_owned(),
            },
            Forma::RechazoDe("not_implemented", "clipboard_failed"),
        ),
        (
            "bloquear la sesion",
            ControlRequest::Power {
                action: PowerAction::Lock,
                confirmation_token: None,
            },
            // Bloquear no pide confirmacion, asi que intenta ejecutarse; en
            // Linux no hay backend de energia.
            Forma::Rechazo("not_implemented"),
        ),
        (
            "apagar sin token",
            ControlRequest::Power {
                action: PowerAction::Shutdown,
                confirmation_token: None,
            },
            // Lo importante: pide confirmacion, no apaga.
            Forma::Confirmacion,
        ),
        (
            "apagar con un token inventado",
            ControlRequest::Power {
                action: PowerAction::Shutdown,
                confirmation_token: Some("token-inventado".to_owned()),
            },
            Forma::Rechazo("invalid_confirmation"),
        ),
        (
            "activar el audio",
            ControlRequest::SetAudioEnabled { enabled: true },
            Forma::Rechazo("not_implemented"),
        ),
    ]
}

/// Un controlador que recorre el guion entero.
struct Controlador {
    device: Device,
    sig: SignalingClient,
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

    /// Envia el guion entero y devuelve las respuestas, en orden.
    async fn recorrer(
        &mut self,
        session_id: SessionId,
        host: DeviceId,
        firmante: &DeviceIdentity,
        guion: &[(&'static str, ControlRequest, Forma)],
    ) -> BTreeMap<RequestId, ControlResponse> {
        let ice = self.ice.clone();
        let mut peer = PeerSession::open(&ice, PeerRole::Answerer)
            .await
            .expect("abre la conexion");

        let lado = comun::LadoControlador {
            session_id,
            yo: self.device.id(),
            host,
            firmante,
        };
        // Por identificador, no por orden de llegada: es justo lo que la
        // correlacion permite, y sin ella una operacion que no contestara
        // haria que todas las siguientes se atribuyeran a la peticion
        // equivocada.
        let mut respuestas: BTreeMap<RequestId, ControlResponse> = BTreeMap::new();
        let mut enviado = false;

        // Deja de esperar cuando dejan de llegar respuestas, en lugar de
        // agotar el tope de la prueba entera. Asi, si una operacion se queda
        // callada, el fallo dice cual y no «se acabo el tiempo».
        let silencio = tokio::time::sleep(Duration::from_secs(20));
        tokio::pin!(silencio);

        loop {
            tokio::select! {
                () = &mut silencio => break,
                mensaje = self.sig.recv() => {
                    let mensaje = mensaje.expect("el canal sigue abierto");
                    if !comun::atender_senalizacion(&mut self.sig, &peer, mensaje, &lado).await {
                        break;
                    }
                }
                evento = peer.next_event() => {
                    match evento.expect("la conexion sigue viva") {
                        PeerEvent::LocalCandidate { candidate, sdp_mid } => {
                            comun::enviar_candidato(
                                &mut self.sig,
                                session_id,
                                candidate,
                                sdp_mid,
                            )
                            .await;
                        }
                        PeerEvent::ChannelOpen { label } if label == CONTROL_CHANNEL => {
                            if !enviado {
                                enviado = true;
                                // Todas de una vez: el canal es ordenado, asi
                                // que las respuestas llegan en el mismo orden
                                // y se pueden emparejar por posicion.
                                // Cada una con su identificador: la posicion
                                // en el guion mas uno. Asi la respuesta se
                                // empareja con su peticion aunque alguna no
                                // conteste o se cuele un evento en medio.
                                for (i, (_, request, _)) in guion.iter().enumerate() {
                                    let raw = Envelope::new(ControlMessage::request(
                                        RequestId(i as u64 + 1),
                                        request.clone(),
                                    ))
                                    .encode()
                                    .expect("codifica");
                                    peer.send(CONTROL_CHANNEL, raw.as_bytes())
                                        .await
                                        .expect("envia la peticion");
                                }
                            }
                        }
                        PeerEvent::Message { label, data } if label == CONTROL_CHANNEL => {
                            let texto = std::str::from_utf8(&data)
                                .expect("el canal de control es texto");
                            let sobre: Envelope<ControlMessage> = Envelope::decode(texto)
                                .expect("la respuesta cumple el protocolo");
                            // Los eventos que empuja el host (el progreso de
                            // la descarga) no son respuestas a nada: no
                            // cuentan.
                            if let ControlMessage::Response { id, response } = sobre.payload {
                                respuestas.insert(id, response);
                                if respuestas.len() == guion.len() {
                                    break;
                                }
                                // Mientras sigan llegando, se sigue esperando.
                                silencio
                                    .as_mut()
                                    .reset(tokio::time::Instant::now() + Duration::from_secs(20));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = peer.close().await;
        respuestas
    }
}

fn emparejado(device: &Device) -> DeviceRecord {
    DeviceRecord::new(
        device.info.clone(),
        device.identity.public_key(),
        PermissionSet::full(),
        true,
        nexus_core::Timestamp::now(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn todas_las_operaciones_del_protocolo_contestan_lo_que_deben() {
    let prueba = async {
        let carpetas = Carpetas::preparar();
        let server = TestServer::start().await;

        let ipad = Device::controller("iPad completo");
        let host = comun::HostDeProducto::arrancar_con_carpetas(
            &server,
            vec![emparejado(&ipad)],
            vec![SharedRoot::writable("Compartida", &carpetas.compartida)],
        )
        .await;

        let mut control = Controlador::conectar(&server, ipad).await;
        let session_id = match control.pedir(host.id(), PermissionSet::full()).await {
            ServerToClient::SessionAccepted { session_id, .. } => session_id,
            otro => panic!("el host deberia aceptar la sesion, llego {otro:?}"),
        };

        let firmante = control.device.duplicate_identity();
        let guion = guion(&carpetas);
        let respuestas = control
            .recorrer(session_id, host.id(), &firmante, &guion)
            .await;

        let mut fallos = Vec::new();
        for (i, (nombre, _, forma)) in guion.iter().enumerate() {
            let id = RequestId(i as u64 + 1);
            match respuestas.get(&id) {
                None => fallos.push(format!("  {nombre}: se quedo sin contestar")),
                Some(respuesta) => {
                    if let Err(detalle) = forma.comprobar(respuesta) {
                        fallos.push(format!("  {nombre}: {detalle}"));
                    }
                }
            }
        }
        assert!(
            fallos.is_empty(),
            "operaciones que no contestaron lo que debian:\n{}",
            fallos.join("\n")
        );

        host.parar().await;
    };

    tokio::time::timeout(TIMEOUT, prueba)
        .await
        .expect("la prueba no deberia agotar el tiempo");
}
