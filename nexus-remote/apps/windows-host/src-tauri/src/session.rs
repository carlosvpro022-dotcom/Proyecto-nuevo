//! Lo que el host hace con cada mensaje que llega por el canal de control.
//!
//! Es el punto donde una orden que viene del iPad se convierte en algo que
//! ocurre en el PC. Por eso tiene una regla que no admite excepciones: **el
//! permiso se comprueba aqui, antes de mirar que pide el mensaje**. Si esta
//! comprobacion se hiciera en el cliente, o mas adelante en la cadena, bastaria
//! un cliente modificado para saltarsela.
//!
//! El modulo es portable a proposito. Recibe el inyector de entrada como objeto
//! de rasgo, de modo que en Windows se le pasa el que llama a `SendInput` y en
//! una prueba el que anota lo que recibe. Asi la logica que decide **si** algo
//! se hace, y **en que** se traduce, se ejecuta y se comprueba en cualquier
//! sistema; lo unico especifico de Windows es la ultima llamada.

use nexus_core::monitor::MonitorLayout;
use nexus_core::protocol::Envelope;
use nexus_core::protocol::control::{ControlMessage, ControlRequest, ControlResponse, HostEvent};
use nexus_core::{CoreError, PermissionSet, Timestamp};
use nexus_file_transfer::AccessPolicy;
use nexus_file_transfer::error::FileError;
use nexus_input::{InputInjector, resolve};

use crate::error::{HostError, HostResult};
use crate::files::{Download, FileService, TransferReport, deny_code, deny_message};

/// Atiende el canal de control de una sesion establecida.
pub struct SessionHandler {
    permissions: PermissionSet,
    monitors: MonitorLayout,
    active_monitor: u8,
    injector: Box<dyn InputInjector + Send>,
    files: FileService,
    /// Descargas aceptadas que todavia no se han puesto en marcha.
    ///
    /// Esta aqui, y no se envia desde `handle`, porque enviar un archivo tarda
    /// y este manejador tambien atiende el raton. Quien llama recoge la
    /// descarga con [`SessionHandler::take_pending_download`] y la pone en su
    /// propia tarea.
    pending_downloads: Vec<Download>,
    applied: u64,
    refused: u64,
}

impl std::fmt::Debug for SessionHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandler")
            .field("permissions", &self.permissions)
            .field("active_monitor", &self.active_monitor)
            .field("transferencias", &self.files.active_count())
            .field("applied", &self.applied)
            .field("refused", &self.refused)
            .finish_non_exhaustive()
    }
}

impl SessionHandler {
    /// Crea el manejador con los permisos que concedio el host.
    ///
    /// Los permisos no se vuelven a negociar durante la sesion: son los que el
    /// host decidio al aceptarla, a partir de su registro de dispositivos.
    pub fn new(
        permissions: PermissionSet,
        monitors: MonitorLayout,
        injector: Box<dyn InputInjector + Send>,
    ) -> Self {
        let active_monitor = monitors
            .0
            .iter()
            .find(|m| m.is_primary)
            .map_or(0, |m| m.index);
        Self {
            permissions,
            monitors,
            active_monitor,
            injector,
            // Sin carpetas declaradas no se comparte nada. Es el valor por
            // omision a proposito: el acceso al disco se concede, no se hereda
            // de tener permiso de archivos.
            files: FileService::new(AccessPolicy::empty()),
            pending_downloads: Vec::new(),
            applied: 0,
            refused: 0,
        }
    }

    /// Declara las carpetas que esta sesion puede ver.
    ///
    /// Sin esto, el permiso de archivos no sirve de nada: la politica vacia
    /// rechaza todas las rutas. Son dos concesiones separadas a proposito, una
    /// del dispositivo y otra del disco.
    #[must_use]
    pub fn with_shared_folders(mut self, policy: AccessPolicy) -> Self {
        self.files = FileService::new(policy);
        self
    }

    /// Recoge una descarga aceptada para ponerla en marcha.
    ///
    /// Quien llama tiene que vaciar esta cola despues de cada
    /// [`SessionHandler::handle`]: una descarga que no se recoge no envia nada
    /// y se queda ocupando una plaza.
    pub fn take_pending_download(&mut self) -> Option<Download> {
        self.pending_downloads.pop()
    }

    /// Indica si el host comparte alguna carpeta en esta sesion.
    #[must_use]
    pub fn shares_no_folders(&self) -> bool {
        self.files.shares_nothing()
    }

    /// Transferencias de archivos activas.
    #[must_use]
    pub fn active_transfers(&self) -> usize {
        self.files.active_count()
    }

    /// Monitor que se esta transmitiendo.
    #[must_use]
    pub const fn active_monitor(&self) -> u8 {
        self.active_monitor
    }

    /// Ordenes aplicadas en esta sesion.
    #[must_use]
    pub const fn applied(&self) -> u64 {
        self.applied
    }

    /// Ordenes rechazadas por falta de permiso o por no ser validas.
    #[must_use]
    pub const fn refused(&self) -> u64 {
        self.refused
    }

    /// Procesa un mensaje recibido por el canal de control.
    ///
    /// Devuelve el mensaje de respuesta ya codificado, listo para enviar por el
    /// mismo canal, o `None` cuando la orden no lleva respuesta.
    ///
    /// Nunca entra en panico ni propaga un error por una orden mal formada: un
    /// cliente que envie basura no debe poder tirar la sesion. Lo que devuelve
    /// en ese caso es un [`ControlResponse::Refused`] con el motivo.
    ///
    /// # Errors
    ///
    /// [`HostError::Protocol`] solo si la **respuesta** no se puede codificar,
    /// que seria un error interno.
    pub fn handle(&mut self, raw: &str) -> HostResult<Option<String>> {
        let request = match Self::decode(raw) {
            Ok(request) => request,
            Err(reason) => return self.deny("bad_request", &reason).map(Some),
        };

        // Primero el permiso. Antes de mirar que pide.
        if self
            .permissions
            .require(request.required_permission())
            .is_err()
        {
            tracing::warn!(
                permission = ?request.required_permission(),
                "orden rechazada por falta de permiso"
            );
            // El mensaje no describe la orden ni la pantalla: solo que no hay
            // permiso. Un cliente no autorizado no debe aprender nada del
            // rechazo.
            return self
                .deny("permission_denied", "el host no concedio ese permiso")
                .map(Some);
        }

        match request {
            ControlRequest::Input { event } => {
                match resolve(&event, &self.monitors, self.active_monitor) {
                    Ok(resolved) => match self.injector.inject(&resolved) {
                        Ok(()) => {
                            self.applied = self.applied.saturating_add(1);
                            Ok(None)
                        }
                        Err(error) => self.deny("injection_failed", &error.to_string()).map(Some),
                    },
                    Err(error) => self.deny("invalid_input", &error.to_string()).map(Some),
                }
            }
            ControlRequest::ListMonitors => {
                self.applied = self.applied.saturating_add(1);
                Self::respond(ControlResponse::Monitors {
                    layout: self.monitors.clone(),
                })
                .map(Some)
            }
            ControlRequest::SelectMonitor { index } => {
                if self.monitors.0.iter().any(|m| m.index == index) {
                    self.active_monitor = index;
                    self.applied = self.applied.saturating_add(1);
                    Self::respond(ControlResponse::Ack).map(Some)
                } else {
                    self.deny("unknown_monitor", &format!("el monitor {index} no existe"))
                        .map(Some)
                }
            }
            ControlRequest::ListDir { path, offset } => match self.files.list(&path, offset) {
                Ok(response) => {
                    self.applied = self.applied.saturating_add(1);
                    Self::respond(response).map(Some)
                }
                Err(error) => self.refuse_file(&error).map(Some),
            },
            ControlRequest::DownloadStart { path } => {
                match self.files.start_download(&path, Timestamp::now()) {
                    Ok((response, download)) => {
                        self.applied = self.applied.saturating_add(1);
                        // No se envia nada aqui: la descarga sale en su propia
                        // tarea para no dejar el raton esperando detras.
                        self.pending_downloads.push(download);
                        Self::respond(response).map(Some)
                    }
                    Err(error) => self.refuse_file(&error).map(Some),
                }
            }
            ControlRequest::UploadStart { path, size_bytes } => {
                match self.files.start_upload(&path, size_bytes, Timestamp::now()) {
                    Ok(response) => {
                        self.applied = self.applied.saturating_add(1);
                        Self::respond(response).map(Some)
                    }
                    Err(error) => self.refuse_file(&error).map(Some),
                }
            }
            ControlRequest::TransferCancel { transfer_id } => {
                match self.files.cancel(transfer_id, Timestamp::now()) {
                    Ok(event) => {
                        self.applied = self.applied.saturating_add(1);
                        Self::event(event).map(Some)
                    }
                    Err(error) => self.refuse_file(&error).map(Some),
                }
            }
            // El resto del protocolo de control corresponde a la etapa 5. Se
            // contesta que no esta disponible, que es la verdad, en lugar de
            // callar y dejar al cliente esperando.
            other => self
                .deny(
                    "not_implemented",
                    &format!(
                        "la operacion de {} todavia no esta implementada en el host",
                        operation_name(&other)
                    ),
                )
                .map(Some),
        }
    }

    /// Atiende un marco binario llegado por el canal de archivos.
    ///
    /// Devuelve el mensaje a enviar por el canal de control (un progreso, o el
    /// rechazo si el marco no vale), o `None` cuando no toca decir nada.
    ///
    /// El permiso ya se comprobo al abrir la transferencia: un marco solo puede
    /// alimentar una subida que este manejador acepto, y
    /// [`FileService::on_frame`](crate::files::FileService::on_frame) rechaza
    /// cualquier identificador que no sea uno de los suyos.
    ///
    /// # Errors
    ///
    /// [`HostError::Protocol`] solo si la respuesta no se puede codificar.
    pub fn handle_file_frame(&mut self, raw: &[u8]) -> HostResult<Option<String>> {
        match self.files.on_frame(raw, Timestamp::now()) {
            Ok(Some(event)) => Self::event(event).map(Some),
            Ok(None) => Ok(None),
            Err(error) => {
                tracing::warn!(%error, "se rechazo un marco del canal de archivos");
                self.refuse_file(&error).map(Some)
            }
        }
    }

    /// Recoge lo que cuenta la tarea que esta enviando una descarga.
    ///
    /// # Errors
    ///
    /// [`HostError::Protocol`] solo si el evento no se puede codificar.
    pub fn handle_transfer_report(
        &mut self,
        report: &TransferReport,
    ) -> HostResult<Option<String>> {
        match self.files.on_report(report, Timestamp::now()) {
            Some(event) => Self::event(event).map(Some),
            None => Ok(None),
        }
    }

    /// Cierra las transferencias al terminar la sesion.
    ///
    /// Las subidas a medias se abandonan y sus temporales se borran: un archivo
    /// incompleto no se queda en la carpeta compartida porque el iPad se fuera
    /// de cobertura.
    pub fn end(&mut self) {
        self.files.end(Timestamp::now());
        self.pending_downloads.clear();
    }

    /// Rechaza una operacion de archivos con su codigo estable.
    fn refuse_file(&mut self, error: &FileError) -> HostResult<String> {
        self.deny(deny_code(error), &deny_message(error))
    }

    fn event(event: HostEvent) -> HostResult<String> {
        Envelope::at(Timestamp::now(), ControlMessage::Event(event))
            .encode()
            .map_err(|e| HostError::Protocol(e.to_string()))
    }

    fn decode(raw: &str) -> Result<ControlRequest, String> {
        let envelope: Envelope<ControlMessage> =
            Envelope::decode(raw).map_err(|e: CoreError| e.to_string())?;
        match envelope.payload {
            ControlMessage::Request(request) => Ok(request),
            ControlMessage::Response(_) | ControlMessage::Event(_) => {
                Err("por el canal de control solo se admiten peticiones".to_owned())
            }
        }
    }

    fn deny(&mut self, code: &str, message: &str) -> HostResult<String> {
        self.refused = self.refused.saturating_add(1);
        Self::respond(ControlResponse::Denied {
            code: code.to_owned(),
            message: message.to_owned(),
        })
    }

    fn respond(response: ControlResponse) -> HostResult<String> {
        Envelope::at(Timestamp::now(), ControlMessage::Response(response))
            .encode()
            .map_err(|e| HostError::Protocol(e.to_string()))
    }
}

/// Nombre estable de una operacion, para poder explicar que falta.
const fn operation_name(request: &ControlRequest) -> &'static str {
    match request {
        ControlRequest::Input { .. } => "entrada",
        ControlRequest::ListMonitors => "listar monitores",
        ControlRequest::SelectMonitor { .. } => "seleccionar monitor",
        ControlRequest::SetQuality { .. } => "cambiar calidad",
        ControlRequest::ClipboardGet => "leer el portapapeles",
        ControlRequest::ClipboardSet { .. } => "escribir el portapapeles",

        _ => "esa",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::monitor::MonitorInfo;
    use nexus_core::protocol::input::{InputEvent, KeyState, PointerButton};
    use nexus_core::{Permission, QualityPreset, QualityProfile};
    use nexus_input::{RecordingInjector, ResolvedInput};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    fn monitors() -> MonitorLayout {
        MonitorLayout(vec![
            MonitorInfo {
                index: 0,
                name: "Principal".to_owned(),
                width: 1_920,
                height: 1_080,
                origin_x: 0,
                origin_y: 0,
                scale_percent: 100,
                is_primary: true,
                refresh_hz: Some(60),
            },
            MonitorInfo {
                index: 1,
                name: "Secundario".to_owned(),
                width: 1_280,
                height: 720,
                origin_x: 1_920,
                origin_y: 0,
                scale_percent: 100,
                is_primary: false,
                refresh_hz: Some(60),
            },
        ])
    }

    /// Inyector que comparte su registro con la prueba.
    ///
    /// El manejador se queda con el inyector, asi que para poder comprobar que
    /// llego hace falta uno cuyo registro viva fuera.
    #[derive(Clone, Default)]
    struct SharedInjector(Arc<Mutex<Vec<ResolvedInput>>>);

    impl SharedInjector {
        fn injected(&self) -> Vec<ResolvedInput> {
            self.0.lock().expect("sin envenenar").clone()
        }
    }

    impl InputInjector for SharedInjector {
        fn name(&self) -> &'static str {
            "compartido"
        }

        fn inject(&mut self, input: &ResolvedInput) -> nexus_input::InputResult<()> {
            self.0.lock().expect("sin envenenar").push(input.clone());
            Ok(())
        }

        fn release_all(&mut self) -> nexus_input::InputResult<()> {
            Ok(())
        }
    }

    /// Manejador con un inyector cuyo registro se puede inspeccionar.
    struct Probe {
        handler: SessionHandler,
        injector: SharedInjector,
    }

    impl Probe {
        fn with(permissions: PermissionSet) -> Self {
            let injector = SharedInjector::default();
            Self {
                handler: SessionHandler::new(permissions, monitors(), Box::new(injector.clone())),
                injector,
            }
        }

        fn send(&mut self, request: ControlRequest) -> Option<ControlResponse> {
            let raw = Envelope::at(
                Timestamp(1_730_000_000_000),
                ControlMessage::Request(request),
            )
            .encode()
            .expect("codifica");
            let answer = self.handler.handle(&raw).expect("no debe fallar");
            answer.map(|raw| {
                let envelope: Envelope<ControlMessage> =
                    Envelope::decode(&raw).expect("la respuesta cumple el protocolo");
                match envelope.payload {
                    ControlMessage::Response(response) => response,
                    other => panic!("se esperaba una respuesta, llego {other:?}"),
                }
            })
        }
    }

    #[test]
    fn un_toque_autorizado_llega_al_inyector() {
        let mut probe = Probe::with(PermissionSet::standard());
        assert_eq!(
            probe.send(ControlRequest::Input {
                event: InputEvent::PointerMove { x: 0.5, y: 0.5 },
            }),
            None,
            "un evento de entrada no lleva respuesta"
        );
        assert_eq!(
            probe.injector.injected(),
            [ResolvedInput::MoveCursor { x: 960, y: 540 }]
        );
        assert_eq!(probe.handler.applied(), 1);
    }

    #[test]
    fn sin_permiso_de_raton_no_se_mueve_nada() {
        let mut probe = Probe::with(PermissionSet::view_only());
        match probe.send(ControlRequest::Input {
            event: InputEvent::PointerMove { x: 0.5, y: 0.5 },
        }) {
            Some(ControlResponse::Denied { code, message }) => {
                assert_eq!(code, "permission_denied");
                assert!(message.contains("permiso"), "{message}");
            }
            other => panic!("deberia rechazarse, llego {other:?}"),
        }
        assert!(
            probe.injector.injected().is_empty(),
            "no debe tocar el inyector"
        );
        assert_eq!(probe.handler.refused(), 1);
    }

    /// Un manejador con una carpeta compartida y los permisos que se digan.
    fn probe_con_carpetas(permissions: PermissionSet) -> (Probe, tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("carpeta temporal");
        let compartida = temp.path().join("intercambio");
        std::fs::create_dir_all(&compartida).expect("crea carpeta");
        std::fs::write(compartida.join("dato.txt"), b"contenido").expect("crea archivo");

        let injector = SharedInjector::default();
        let handler = SessionHandler::new(permissions, monitors(), Box::new(injector.clone()))
            .with_shared_folders(nexus_file_transfer::AccessPolicy::new(vec![
                nexus_file_transfer::SharedRoot::writable("Intercambio", &compartida),
            ]));
        (Probe { handler, injector }, temp, compartida)
    }

    #[test]
    fn sin_permiso_de_archivos_no_se_lista_ni_se_descarga() {
        let (mut probe, _temp, compartida) =
            probe_con_carpetas(PermissionSet::from_iter([Permission::ViewScreen]));

        for request in [
            ControlRequest::ListDir {
                path: compartida.to_string_lossy().into_owned(),
                offset: 0,
            },
            ControlRequest::DownloadStart {
                path: compartida.join("dato.txt").to_string_lossy().into_owned(),
            },
        ] {
            match probe.send(request) {
                Some(ControlResponse::Denied { code, .. }) => {
                    assert_eq!(code, "permission_denied");
                }
                otro => panic!("deberia rechazarse por permiso, llego {otro:?}"),
            }
        }
    }

    #[test]
    fn con_permiso_de_solo_lectura_no_se_puede_subir_nada() {
        // El limite que importa: la carpeta compartida admite escritura, pero
        // el dispositivo no tiene el permiso. Lo uno no suple lo otro.
        let (mut probe, _temp, compartida) = probe_con_carpetas(PermissionSet::from_iter([
            Permission::ViewScreen,
            Permission::FileRead,
        ]));

        // Leer si puede.
        probe
            .send(ControlRequest::ListDir {
                path: compartida.to_string_lossy().into_owned(),
                offset: 0,
            })
            .expect("deberia contestar");

        // Escribir no.
        let destino = compartida.join("nuevo.bin");
        match probe.send(ControlRequest::UploadStart {
            path: destino.to_string_lossy().into_owned(),
            size_bytes: 4,
        }) {
            Some(ControlResponse::Denied { code, .. }) => {
                assert_eq!(code, "permission_denied");
            }
            otro => panic!("deberia rechazarse por permiso, llego {otro:?}"),
        }
        assert!(!destino.exists(), "no debe crearse nada");

        // Y los marcos que envie por el canal de archivos no tienen donde ir:
        // sin subida abierta, no hay nada que alimentar.
        let frame = nexus_file_transfer::Frame::Data {
            transfer_id: nexus_core::TransferId::new(),
            sequence: 0,
            payload: b"cola".to_vec(),
        };
        let respuesta = probe
            .handler
            .handle_file_frame(&frame.encode())
            .expect("no debe fallar")
            .expect("deberia contestar");
        assert!(
            respuesta.contains("unknown_transfer"),
            "el marco no debe encontrar transferencia: {respuesta}"
        );
        assert!(!destino.exists());
        let restos: Vec<_> = std::fs::read_dir(&compartida)
            .expect("lee")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "dato.txt")
            .collect();
        assert!(restos.is_empty(), "quedaron restos: {restos:?}");
    }

    #[test]
    fn un_manejador_recien_creado_no_comparte_ninguna_carpeta() {
        // El valor por omision: tener el permiso no abre el disco.
        let mut probe = Probe::with(PermissionSet::full());
        assert!(probe.handler.shares_no_folders());
        match probe.send(ControlRequest::ListDir {
            path: "/tmp".to_owned(),
            offset: 0,
        }) {
            Some(ControlResponse::Denied { code, .. }) => assert_eq!(code, "access_denied"),
            otro => panic!("deberia rechazarse por no haber carpetas, llego {otro:?}"),
        }
        assert_eq!(probe.handler.active_transfers(), 0);
    }

    #[test]
    fn sin_permiso_de_teclado_tampoco_pasa_una_pulsacion() {
        use nexus_core::protocol::input::{KeyCode, Modifiers};
        let mut permisos = PermissionSet::standard();
        permisos.revoke(Permission::ControlKeyboard);
        let mut probe = Probe::with(permisos);

        assert!(matches!(
            probe.send(ControlRequest::Input {
                event: InputEvent::Key {
                    code: KeyCode("KeyA".to_owned()),
                    state: KeyState::Down,
                    modifiers: Modifiers::default(),
                },
            }),
            Some(ControlResponse::Denied { .. })
        ));
        assert!(probe.injector.injected().is_empty());
    }

    #[test]
    fn el_permiso_se_comprueba_antes_de_validar_la_orden() {
        // Una orden invalida y sin permiso debe rechazarse por el permiso: el
        // motivo que se devuelve no debe revelar nada de la pantalla del host.
        let mut probe = Probe::with(PermissionSet::view_only());
        match probe.send(ControlRequest::Input {
            event: InputEvent::PointerMove { x: 50.0, y: 50.0 },
        }) {
            Some(ControlResponse::Denied { code, message }) => {
                assert_eq!(code, "permission_denied");
                assert!(
                    !message.contains("1920") && !message.contains("monitor"),
                    "no debe filtrar la configuracion de pantalla: {message}"
                );
            }
            other => panic!("deberia rechazarse, llego {other:?}"),
        }
    }

    #[test]
    fn una_coordenada_fuera_de_rango_se_rechaza_con_permiso_incluido() {
        let mut probe = Probe::with(PermissionSet::standard());
        assert!(matches!(
            probe.send(ControlRequest::Input {
                event: InputEvent::PointerMove { x: 7.5, y: 0.5 },
            }),
            Some(ControlResponse::Denied { .. })
        ));
        assert!(probe.injector.injected().is_empty());
    }

    #[test]
    fn se_puede_listar_y_cambiar_de_monitor() {
        let mut probe = Probe::with(PermissionSet::standard());

        match probe.send(ControlRequest::ListMonitors) {
            Some(ControlResponse::Monitors { layout }) => {
                assert_eq!(layout.0.len(), 2);
            }
            other => panic!("se esperaba la lista de monitores, llego {other:?}"),
        }

        assert_eq!(probe.handler.active_monitor(), 0, "arranca en el principal");
        assert!(matches!(
            probe.send(ControlRequest::SelectMonitor { index: 1 }),
            Some(ControlResponse::Ack)
        ));
        assert_eq!(probe.handler.active_monitor(), 1);

        // Y el mismo toque cae ahora en el otro monitor.
        probe.send(ControlRequest::Input {
            event: InputEvent::PointerMove { x: 0.5, y: 0.5 },
        });
        assert_eq!(
            probe.injector.injected().last(),
            Some(&ResolvedInput::MoveCursor {
                x: 1_920 + 640,
                y: 360
            })
        );
    }

    #[test]
    fn un_monitor_que_no_existe_se_rechaza() {
        let mut probe = Probe::with(PermissionSet::standard());
        assert!(matches!(
            probe.send(ControlRequest::SelectMonitor { index: 9 }),
            Some(ControlResponse::Denied { .. })
        ));
        assert_eq!(probe.handler.active_monitor(), 0, "no debe cambiar");
    }

    #[test]
    fn una_operacion_de_la_etapa_5_se_declara_pendiente_en_lugar_de_callar() {
        let mut probe = Probe::with(PermissionSet::full());
        match probe.send(ControlRequest::SetQuality {
            profile: QualityProfile::from_preset(QualityPreset::Equilibrado),
        }) {
            Some(ControlResponse::Denied { code, message }) => {
                assert_eq!(code, "not_implemented");
                assert!(
                    message.contains("todavia no esta implementada"),
                    "el cliente tiene que saber que falta: {message}"
                );
            }
            other => panic!("deberia contestar que falta, llego {other:?}"),
        }
    }

    #[test]
    fn basura_no_tira_la_sesion() {
        let mut probe = Probe::with(PermissionSet::full());
        for raw in [
            "",
            "{",
            "null",
            r#"{"v":1,"ts":1,"payload":{"type":"tomar_el_control"}}"#,
            r#"{"v":99,"ts":1,"payload":{"type":"list_monitors"}}"#,
        ] {
            let answer = probe
                .handler
                .handle(raw)
                .unwrap_or_else(|e| panic!("{raw:?} no debe fallar: {e}"));
            assert!(answer.is_some(), "{raw:?} deberia contestar algo");
        }
        assert!(probe.injector.injected().is_empty());
        assert_eq!(probe.handler.refused(), 5);
    }

    #[test]
    fn una_respuesta_no_se_admite_como_peticion() {
        // El canal es bidireccional; un cliente no debe poder colar respuestas.
        let mut probe = Probe::with(PermissionSet::full());
        let raw = Envelope::at(Timestamp(1), ControlMessage::Response(ControlResponse::Ack))
            .encode()
            .expect("codifica");
        assert!(probe.handler.handle(&raw).expect("no falla").is_some());
        assert_eq!(probe.handler.refused(), 1);
    }

    #[test]
    fn el_manejador_arranca_en_el_monitor_principal() {
        let layout = MonitorLayout(vec![MonitorInfo {
            index: 3,
            name: "Unico".to_owned(),
            width: 800,
            height: 600,
            origin_x: 0,
            origin_y: 0,
            scale_percent: 100,
            is_primary: true,
            refresh_hz: None,
        }]);
        let handler = SessionHandler::new(
            PermissionSet::standard(),
            layout,
            Box::new(RecordingInjector::new()),
        );
        assert_eq!(handler.active_monitor(), 3);
    }

    #[test]
    fn un_boton_completo_llega_en_orden() {
        let mut probe = Probe::with(PermissionSet::standard());
        for state in [KeyState::Down, KeyState::Up] {
            probe.send(ControlRequest::Input {
                event: InputEvent::PointerButton {
                    button: PointerButton::Right,
                    state,
                },
            });
        }
        assert_eq!(
            probe.injector.injected(),
            [
                ResolvedInput::Button {
                    button: PointerButton::Right,
                    state: KeyState::Down
                },
                ResolvedInput::Button {
                    button: PointerButton::Right,
                    state: KeyState::Up
                },
            ]
        );
    }
}
