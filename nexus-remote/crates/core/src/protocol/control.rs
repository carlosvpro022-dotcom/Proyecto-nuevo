//! Protocolo del canal de control: controlador <-> host, extremo a extremo.
//!
//! Viaja por el canal de datos de WebRTC, cifrado con DTLS. Ni el servidor de
//! senalizacion ni el TURN pueden leerlo ni modificarlo.
//!
//! El canal es de peticion/respuesta con correlacion por
//! [`RequestId`](crate::ids::RequestId), mas un flujo de eventos que el host
//! empuja sin que nadie los pida (cambio de monitores, progreso de
//! transferencias, portapapeles).
//!
//! La correlacion no es un adorno. El orden de llegada no sirve para
//! emparejar: hay peticiones que no llevan respuesta —un movimiento de raton
//! no contesta nada— y el host mete eventos por el mismo canal sin que nadie
//! los pida. Con una sola de esas cosas en medio, quien pregunta empezaria a
//! atribuir cada respuesta a la peticion equivocada y no habria forma de
//! notarlo.
//!
//! Cada peticion declara en [`ControlRequest::required_permission`] el permiso
//! que necesita. El host comprueba ese permiso antes de ejecutar nada, en un
//! solo punto, lo que evita que una funcion nueva se cuele sin control.

use serde::{Deserialize, Serialize};

use crate::ids::{RequestId, TransferId};
use crate::monitor::MonitorLayout;
use crate::permissions::Permission;
use crate::protocol::input::InputEvent;
use crate::quality::QualityProfile;
use crate::time::Timestamp;

/// Operacion de energia sobre el equipo host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerAction {
    /// Bloquea la sesion de Windows.
    Lock,
    /// Reinicia el equipo.
    Reboot,
    /// Apaga el equipo.
    Shutdown,
}

impl PowerAction {
    /// Permiso que exige la operacion.
    #[must_use]
    pub const fn required_permission(self) -> Permission {
        match self {
            Self::Lock => Permission::LockWorkstation,
            Self::Reboot => Permission::Reboot,
            Self::Shutdown => Permission::Shutdown,
        }
    }

    /// Indica si hace falta confirmacion interactiva en el host.
    #[must_use]
    pub const fn requires_confirmation(self) -> bool {
        self.required_permission().requires_host_confirmation()
    }
}

/// Entrada de un listado de directorio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// Nombre del elemento, sin ruta.
    pub name: String,
    /// Si es un directorio.
    pub is_dir: bool,
    /// Tamano en bytes. Cero para directorios.
    pub size_bytes: u64,
    /// Ultima modificacion, si el sistema la reporta.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<Timestamp>,
    /// Si el host lo considera oculto.
    #[serde(default)]
    pub hidden: bool,
}

/// Estado de una transferencia en curso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferState {
    /// Aceptada, aun sin datos.
    Pending,
    /// Transmitiendo.
    Active,
    /// Terminada con exito.
    Completed,
    /// Cancelada por una de las partes.
    Cancelled,
    /// Terminada por error.
    Failed,
}

impl TransferState {
    /// Indica si el estado es final.
    #[must_use]
    pub const fn is_final(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

/// Informacion basica del equipo host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemStatus {
    /// Nombre del equipo.
    pub hostname: String,
    /// Sistema operativo y version.
    pub os_description: String,
    /// Nucleos logicos.
    pub cpu_cores: u16,
    /// Memoria total en mebibytes.
    pub memory_total_mib: u64,
    /// Memoria disponible en mebibytes.
    pub memory_available_mib: u64,
    /// Tiempo encendido en segundos.
    pub uptime_secs: u64,
    /// Si la sesion de Windows esta bloqueada, cuando se puede saber.
    ///
    /// Es opcional a proposito. Un `bool` obligaria a elegir entre `true` y
    /// `false` tambien cuando el host no tiene forma de averiguarlo, y
    /// contestar «no esta bloqueada» sin saberlo seria inventarse el dato.
    /// `None` significa que no se sabe.
    #[serde(default)]
    pub session_locked: Option<bool>,
}

/// Peticion del controlador al host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum ControlRequest {
    /// Evento de raton o teclado.
    Input {
        /// Evento concreto.
        event: InputEvent,
    },
    /// Pide la lista de monitores.
    ListMonitors,
    /// Cambia el monitor transmitido.
    SelectMonitor {
        /// Indice del monitor.
        index: u8,
    },
    /// Cambia los parametros de calidad.
    SetQuality {
        /// Perfil pedido.
        profile: QualityProfile,
    },
    /// Lee el portapapeles del host.
    ClipboardGet,
    /// Escribe en el portapapeles del host.
    ClipboardSet {
        /// Texto a colocar.
        text: String,
    },
    /// Lista un directorio del host.
    ///
    /// El listado se pagina: una carpeta de descargas con miles de archivos no
    /// cabe en un mensaje del canal de control, y truncarla dejaria el resto
    /// inalcanzable. Quien pregunta avanza con `offset` hasta que la respuesta
    /// diga que no queda mas.
    ListDir {
        /// Ruta absoluta dentro de las raices permitidas.
        path: String,
        /// Entradas a saltar, dentro del orden completo de la carpeta.
        #[serde(default)]
        offset: u64,
    },
    /// Empieza una descarga desde el host.
    DownloadStart {
        /// Ruta del archivo.
        path: String,
    },
    /// Empieza una subida hacia el host.
    UploadStart {
        /// Ruta de destino.
        path: String,
        /// Tamano total anunciado.
        size_bytes: u64,
    },
    /// Cancela una transferencia.
    TransferCancel {
        /// Transferencia a cancelar.
        transfer_id: TransferId,
    },
    /// Pide informacion del sistema.
    SystemStatus,
    /// Ejecuta una operacion de energia.
    Power {
        /// Operacion pedida.
        action: PowerAction,
        /// Token de confirmacion entregado por el host, si la operacion lo exige.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirmation_token: Option<String>,
    },
    /// Activa o desactiva el audio.
    SetAudioEnabled {
        /// Si el audio debe transmitirse.
        enabled: bool,
    },
}

impl ControlRequest {
    /// Permiso necesario para atender la peticion.
    ///
    /// Es el unico sitio donde se decide: el host llama a esta funcion antes de
    /// despachar, de modo que anadir una operacion obliga a declarar su permiso.
    // Varias operaciones comparten permiso a proposito (listar y descargar
    // exigen lectura, cancelar lo propio no amplia nada): fusionar los brazos
    // esconderia la intencion de cada una.
    #[allow(clippy::match_same_arms)]
    #[must_use]
    pub const fn required_permission(&self) -> Permission {
        match self {
            Self::Input { event } => {
                if event.needs_keyboard_permission() {
                    Permission::ControlKeyboard
                } else {
                    Permission::ControlPointer
                }
            }
            Self::ListMonitors | Self::SelectMonitor { .. } | Self::SetQuality { .. } => {
                Permission::ViewScreen
            }
            Self::ClipboardGet => Permission::ClipboardRead,
            Self::ClipboardSet { .. } => Permission::ClipboardWrite,
            Self::ListDir { .. } | Self::DownloadStart { .. } => Permission::FileRead,
            Self::UploadStart { .. } => Permission::FileWrite,
            // Cancelar una transferencia propia no amplia el acceso: basta con
            // poder leer para poder abortar lo que uno mismo inicio.
            Self::TransferCancel { .. } => Permission::FileRead,
            Self::SystemStatus => Permission::SystemInfo,
            Self::Power { action, .. } => action.required_permission(),
            Self::SetAudioEnabled { .. } => Permission::ReceiveAudio,
        }
    }

    /// Indica si el host debe pedir confirmacion antes de ejecutar.
    #[must_use]
    pub const fn requires_host_confirmation(&self) -> bool {
        match self {
            Self::Power { action, .. } => action.requires_confirmation(),
            _ => false,
        }
    }
}

/// Respuesta del host a una peticion concreta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "result")]
pub enum ControlResponse {
    /// Operacion aceptada sin datos de vuelta.
    Ack,
    /// Operacion rechazada.
    Denied {
        /// Codigo estable.
        code: String,
        /// Mensaje legible.
        message: String,
    },
    /// Hace falta confirmacion en el host; se adjunta el token a reenviar.
    ConfirmationRequired {
        /// Token de un solo uso que acompana al reintento.
        token: String,
        /// Texto que el host mostrara a la persona.
        prompt: String,
    },
    /// Lista de monitores.
    Monitors {
        /// Disposicion actual.
        layout: MonitorLayout,
    },
    /// Contenido del portapapeles.
    ClipboardText {
        /// Texto leido.
        text: String,
    },
    /// Listado de un directorio, una pagina a la vez.
    DirListing {
        /// Ruta consultada, ya normalizada por el host.
        path: String,
        /// Entradas de esta pagina, ordenadas: carpetas primero, luego nombre.
        entries: Vec<DirEntry>,
        /// Posicion de la primera entrada dentro del orden completo.
        #[serde(default)]
        offset: u64,
        /// Entradas que tiene la carpeta en total.
        ///
        /// Quien pregunta sabe que falta mas cuando `offset + entries.len()`
        /// se queda por debajo de este numero.
        total_entries: u64,
        /// Si la carpeta es tan grande que el host no la recorrio entera.
        ///
        /// Se dice en lugar de esconderlo: un listado incompleto que parece
        /// completo es peor que uno que avisa.
        #[serde(default)]
        scan_truncated: bool,
    },
    /// Transferencia aceptada.
    TransferAccepted {
        /// Identificador asignado.
        transfer_id: TransferId,
        /// Tamano total conocido, si lo hay.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size_bytes: Option<u64>,
    },
    /// Informacion del sistema.
    SystemStatus {
        /// Datos del equipo.
        status: SystemStatus,
    },
}

/// Evento que el host empuja sin peticion previa.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum HostEvent {
    /// Cambio la configuracion de monitores.
    MonitorsChanged {
        /// Nueva disposicion.
        layout: MonitorLayout,
    },
    /// El portapapeles del host cambio.
    ClipboardChanged {
        /// Texto nuevo.
        text: String,
    },
    /// Progreso de una transferencia.
    TransferProgress {
        /// Transferencia afectada.
        transfer_id: TransferId,
        /// Bytes transferidos.
        transferred_bytes: u64,
        /// Bytes totales, si se conocen.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_bytes: Option<u64>,
        /// Estado actual.
        state: TransferState,
    },
    /// Una transferencia se corto por un error.
    ///
    /// Es un evento y no una respuesta porque no contesta a ninguna peticion:
    /// los bytes de una transferencia llegan por el canal binario, y lo que
    /// falla ahi no tiene una peticion del canal de control a la que
    /// responder.
    TransferFailed {
        /// Transferencia afectada, cuando se puede saber cual.
        ///
        /// Un marco tan mal formado que ni siquiera se puede leer de que
        /// transferencia era deja esto vacio.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transfer_id: Option<TransferId>,
        /// Codigo estable del motivo.
        code: String,
        /// Motivo legible.
        message: String,
    },
    /// El perfil de calidad en uso cambio (por adaptacion automatica).
    QualityChanged {
        /// Perfil efectivo.
        profile: QualityProfile,
    },
    /// La sesion de Windows se bloqueo o desbloqueo.
    SessionLockChanged {
        /// Si quedo bloqueada.
        locked: bool,
    },
    /// El host va a cerrar la sesion.
    Closing {
        /// Motivo legible.
        reason: String,
    },
}

/// Cualquier mensaje del canal de control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "channel")]
pub enum ControlMessage {
    /// Peticion del controlador.
    Request {
        /// Identificador con el que el host marcara su respuesta.
        ///
        /// Lo elige quien pregunta y el host lo devuelve tal cual. Sin el, la
        /// unica forma de emparejar una respuesta con su peticion seria el
        /// orden de llegada, y el orden no basta: hay peticiones que **no
        /// llevan respuesta** (un movimiento de raton no contesta nada), y el
        /// host empuja eventos por el mismo canal sin que nadie los pida. Con
        /// una sola de esas cosas en medio, quien pregunta empezaria a
        /// atribuir cada respuesta a la peticion equivocada.
        id: RequestId,
        /// Lo que se pide.
        request: ControlRequest,
    },
    /// Respuesta del host.
    Response {
        /// El mismo identificador que traia la peticion.
        id: RequestId,
        /// Lo que contesta.
        response: ControlResponse,
    },
    /// Evento empujado por el host.
    ///
    /// No lleva identificador porque no contesta a nada: es el host contando
    /// algo por su cuenta.
    Event(HostEvent),
}

impl ControlMessage {
    /// Empaqueta una peticion con su identificador.
    #[must_use]
    pub const fn request(id: RequestId, request: ControlRequest) -> Self {
        Self::Request { id, request }
    }

    /// Empaqueta una respuesta con el identificador de su peticion.
    #[must_use]
    pub const fn response(id: RequestId, response: ControlResponse) -> Self {
        Self::Response { id, response }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::input::{KeyCode, KeyState, Modifiers, PointerButton};

    #[test]
    fn pointer_input_needs_pointer_permission() {
        let request = ControlRequest::Input {
            event: InputEvent::PointerMove { x: 0.5, y: 0.5 },
        };
        assert_eq!(request.required_permission(), Permission::ControlPointer);

        let request = ControlRequest::Input {
            event: InputEvent::PointerButton {
                button: PointerButton::Left,
                state: KeyState::Down,
            },
        };
        assert_eq!(request.required_permission(), Permission::ControlPointer);
    }

    #[test]
    fn keyboard_input_needs_keyboard_permission() {
        let request = ControlRequest::Input {
            event: InputEvent::Key {
                code: KeyCode("KeyA".into()),
                state: KeyState::Down,
                modifiers: Modifiers::NONE,
            },
        };
        assert_eq!(request.required_permission(), Permission::ControlKeyboard);

        let request = ControlRequest::Input {
            event: InputEvent::Text {
                text: "hola".into(),
            },
        };
        assert_eq!(request.required_permission(), Permission::ControlKeyboard);
    }

    #[test]
    fn clipboard_read_and_write_are_separate_permissions() {
        assert_eq!(
            ControlRequest::ClipboardGet.required_permission(),
            Permission::ClipboardRead
        );
        assert_eq!(
            ControlRequest::ClipboardSet { text: "x".into() }.required_permission(),
            Permission::ClipboardWrite
        );
    }

    #[test]
    fn upload_needs_write_while_listing_needs_read() {
        assert_eq!(
            ControlRequest::ListDir {
                path: "C:/".into(),
                offset: 0,
            }
            .required_permission(),
            Permission::FileRead
        );
        assert_eq!(
            ControlRequest::DownloadStart {
                path: "C:/a.txt".into()
            }
            .required_permission(),
            Permission::FileRead
        );
        assert_eq!(
            ControlRequest::UploadStart {
                path: "C:/a.txt".into(),
                size_bytes: 10
            }
            .required_permission(),
            Permission::FileWrite
        );
    }

    #[test]
    fn power_actions_map_to_their_own_permissions() {
        for (action, permission) in [
            (PowerAction::Lock, Permission::LockWorkstation),
            (PowerAction::Reboot, Permission::Reboot),
            (PowerAction::Shutdown, Permission::Shutdown),
        ] {
            let request = ControlRequest::Power {
                action,
                confirmation_token: None,
            };
            assert_eq!(request.required_permission(), permission);
        }
    }

    #[test]
    fn reboot_and_shutdown_require_confirmation_but_lock_does_not() {
        assert!(
            ControlRequest::Power {
                action: PowerAction::Reboot,
                confirmation_token: None
            }
            .requires_host_confirmation()
        );
        assert!(
            ControlRequest::Power {
                action: PowerAction::Shutdown,
                confirmation_token: None
            }
            .requires_host_confirmation()
        );
        assert!(
            !ControlRequest::Power {
                action: PowerAction::Lock,
                confirmation_token: None
            }
            .requires_host_confirmation()
        );
    }

    #[test]
    fn no_other_request_asks_for_confirmation() {
        let requests = [
            ControlRequest::ListMonitors,
            ControlRequest::SelectMonitor { index: 0 },
            ControlRequest::ClipboardGet,
            ControlRequest::SystemStatus,
            ControlRequest::SetAudioEnabled { enabled: true },
            ControlRequest::UploadStart {
                path: "C:/x".into(),
                size_bytes: 1,
            },
        ];
        for request in requests {
            assert!(!request.requires_host_confirmation(), "{request:?}");
        }
    }

    #[test]
    fn every_request_variant_declares_a_permission() {
        // Si se anade una variante sin declarar permiso, el `match` exhaustivo
        // de `required_permission` no compila: esta prueba documenta el contrato.
        let requests = vec![
            ControlRequest::Input {
                event: InputEvent::PointerMove { x: 0.0, y: 0.0 },
            },
            ControlRequest::ListMonitors,
            ControlRequest::SelectMonitor { index: 1 },
            ControlRequest::SetQuality {
                profile: QualityProfile::default(),
            },
            ControlRequest::ClipboardGet,
            ControlRequest::ClipboardSet { text: "a".into() },
            ControlRequest::ListDir {
                path: "C:/".into(),
                offset: 0,
            },
            ControlRequest::DownloadStart {
                path: "C:/a".into(),
            },
            ControlRequest::UploadStart {
                path: "C:/a".into(),
                size_bytes: 1,
            },
            ControlRequest::TransferCancel {
                transfer_id: TransferId::new(),
            },
            ControlRequest::SystemStatus,
            ControlRequest::Power {
                action: PowerAction::Lock,
                confirmation_token: None,
            },
            ControlRequest::SetAudioEnabled { enabled: false },
        ];
        for request in requests {
            // Basta con que la llamada no entre en panico y devuelva un permiso.
            let _ = request.required_permission().name();
        }
    }

    #[test]
    fn una_peticion_lleva_su_identificador_y_la_respuesta_lo_devuelve() {
        let id = RequestId(7);
        let peticion = ControlMessage::request(id, ControlRequest::ListMonitors);
        let raw = crate::protocol::Envelope::new(peticion)
            .encode()
            .expect("codifica");

        let vuelta: crate::protocol::Envelope<ControlMessage> =
            crate::protocol::Envelope::decode(&raw).expect("decodifica");
        match vuelta.payload {
            ControlMessage::Request { id: leido, .. } => assert_eq!(leido, id),
            otro => panic!("se esperaba una peticion: {otro:?}"),
        }

        let respuesta = ControlMessage::response(id, ControlResponse::Ack);
        match respuesta {
            ControlMessage::Response { id: devuelto, .. } => assert_eq!(devuelto, id),
            otro => panic!("se esperaba una respuesta: {otro:?}"),
        }
    }

    #[test]
    fn un_evento_no_lleva_identificador_porque_no_contesta_a_nada() {
        // Es la distincion que hace util la correlacion: lo que el host empuja
        // por su cuenta no se puede confundir con la respuesta a una peticion.
        let evento = ControlMessage::Event(HostEvent::SessionLockChanged { locked: true });
        let raw = crate::protocol::Envelope::new(evento)
            .encode()
            .expect("codifica");
        assert!(
            !raw.contains("\"id\""),
            "un evento no deberia llevar identificador: {raw}"
        );
    }

    #[test]
    fn dos_peticiones_distintas_no_comparten_identificador_por_descuido() {
        // Lo que protege: si el orden bastara, una peticion sin respuesta
        // —como un movimiento de raton— desplazaria todas las siguientes.
        let a = ControlMessage::request(RequestId::FIRST, ControlRequest::ListMonitors);
        let b = ControlMessage::request(
            RequestId::FIRST.next(),
            ControlRequest::SelectMonitor { index: 1 },
        );
        match (a, b) {
            (ControlMessage::Request { id: uno, .. }, ControlMessage::Request { id: dos, .. }) => {
                assert_ne!(uno, dos);
            }
            otro => panic!("se esperaban dos peticiones: {otro:?}"),
        }
    }

    #[test]
    fn control_messages_roundtrip() {
        let messages = vec![
            ControlMessage::request(RequestId::FIRST, ControlRequest::ListMonitors),
            ControlMessage::response(RequestId::FIRST, ControlResponse::Ack),
            ControlMessage::response(
                RequestId::FIRST,
                ControlResponse::ConfirmationRequired {
                    token: "t-1".into(),
                    prompt: "Reiniciar el equipo?".into(),
                },
            ),
            ControlMessage::response(
                RequestId::FIRST,
                ControlResponse::DirListing {
                    path: "C:/Users".into(),
                    entries: vec![DirEntry {
                        name: "Documentos".into(),
                        is_dir: true,
                        size_bytes: 0,
                        modified_at: Some(Timestamp(1)),
                        hidden: false,
                    }],
                    offset: 0,
                    total_entries: 1,
                    scan_truncated: false,
                },
            ),
            ControlMessage::Event(HostEvent::TransferProgress {
                transfer_id: TransferId::new(),
                transferred_bytes: 512,
                total_bytes: Some(2_048),
                state: TransferState::Active,
            }),
            ControlMessage::Event(HostEvent::SessionLockChanged { locked: true }),
        ];
        for message in messages {
            let raw = serde_json::to_string(&message).expect("serializa");
            let back: ControlMessage = serde_json::from_str(&raw).expect("deserializa");
            assert_eq!(message, back, "fallo con {raw}");
        }
    }

    #[test]
    fn transfer_final_states_are_identified() {
        assert!(!TransferState::Pending.is_final());
        assert!(!TransferState::Active.is_final());
        assert!(TransferState::Completed.is_final());
        assert!(TransferState::Cancelled.is_final());
        assert!(TransferState::Failed.is_final());
    }
}
