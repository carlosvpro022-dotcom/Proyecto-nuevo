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

use nexus_clipboard::{Clipboard, ClipboardError, check_text, default_clipboard};
use nexus_core::monitor::MonitorLayout;
use nexus_core::protocol::Envelope;
use nexus_core::protocol::control::PowerAction;
use nexus_core::protocol::control::{ControlMessage, ControlRequest, ControlResponse, HostEvent};
use nexus_core::protocol::input::InputEvent;
use nexus_core::{CoreError, PermissionSet, QualityPreset, QualityProfile, RequestId, Timestamp};
use nexus_file_transfer::AccessPolicy;
use nexus_file_transfer::error::FileError;
use nexus_input::{InputInjector, resolve};
use nexus_power::{PowerControl, PowerError, default_power};

use crate::confirmation::{Pending, Redeem, SharedConfirmations};
use crate::error::{HostError, HostResult};
use crate::files::{Download, FileService, TransferReport, deny_code, deny_message};

/// Lo que el host devuelve por el canal de control.
///
/// Una respuesta contesta a lo que se pregunto; un evento es algo que el host
/// cuenta sin que nadie lo pida. Cancelar una transferencia produce lo
/// segundo, y por eso las dos cosas comparten camino de salida.
enum Respuesta {
    /// Respuesta a una peticion concreta.
    Control(ControlResponse),
    /// Evento empujado por el host.
    Evento(HostEvent),
}

/// Atiende el canal de control de una sesion establecida.
pub struct SessionHandler {
    permissions: PermissionSet,
    monitors: MonitorLayout,
    active_monitor: u8,
    injector: Box<dyn InputInjector + Send>,
    quality: QualityProfile,
    /// Cuantas veces se ha pedido la pantalla en esta sesion.
    ///
    /// Es una cuenta y no un `bool` porque la segunda peticion tambien
    /// significa algo: quien la hace perdio el hilo y necesita un fotograma
    /// completo.
    screen_requests: u64,
    clipboard: Box<dyn Clipboard>,
    power: Box<dyn PowerControl>,
    confirmations: SharedConfirmations,
    /// Identificador de la peticion que se esta atendiendo.
    ///
    /// Lo pone [`SessionHandler::handle`] al decodificar, y lo usa
    /// [`SessionHandler::respond`] para marcar la respuesta. Vive aqui, y no
    /// como parametro de cada paso, porque el despacho tiene muchas ramas y
    /// pasarlo por todas invitaria a olvidarlo en una.
    current_request: RequestId,
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
            quality: QualityProfile::from_preset(QualityPreset::Equilibrado),
            screen_requests: 0,
            clipboard: default_clipboard(),
            power: default_power(),
            confirmations: SharedConfirmations::default(),
            current_request: RequestId::FIRST,
            // Sin carpetas declaradas no se comparte nada. Es el valor por
            // omision a proposito: el acceso al disco se concede, no se hereda
            // de tener permiso de archivos.
            files: FileService::new(AccessPolicy::empty()),
            pending_downloads: Vec::new(),
            applied: 0,
            refused: 0,
        }
    }

    /// Usa otro portapapeles en lugar del del sistema.
    ///
    /// Sirve para probar toda la logica del portapapeles en cualquier
    /// plataforma: el de Windows solo existe en Windows.
    #[must_use]
    pub fn with_clipboard(mut self, clipboard: Box<dyn Clipboard>) -> Self {
        self.clipboard = clipboard;
        self
    }

    /// Usa otro backend de energia en lugar del del sistema.
    ///
    /// Sirve para probar que la confirmacion funciona sin apagar la maquina
    /// donde corren las pruebas.
    #[must_use]
    pub fn with_power(mut self, power: Box<dyn PowerControl>) -> Self {
        self.power = power;
        self
    }

    /// Comparte la puerta de confirmaciones con el panel del host.
    ///
    /// Sin esto, la confirmacion la pide la sesion y **nadie puede
    /// aprobarla**: la persona del PC no tendria donde decidir. Es la pieza
    /// que conecta las dos mitades.
    #[must_use]
    pub fn with_confirmations(mut self, confirmations: SharedConfirmations) -> Self {
        self.confirmations = confirmations;
        self
    }

    /// La confirmacion que el panel del host tiene que mostrar, si la hay.
    ///
    /// Devuelve el identificador publico y el texto, nunca el token: el token
    /// es de quien pidio la operacion, y si el panel lo manejara se podria
    /// filtrar de vuelta a quien no debe tenerlo.
    #[must_use]
    pub fn pending_confirmation(&self) -> Option<Pending> {
        self.confirmations.pending(Timestamp::now())
    }

    /// Una persona del PC aprueba la confirmacion pendiente.
    ///
    /// Devuelve `false` si el identificador no corresponde a ninguna viva.
    pub fn approve_confirmation(&mut self, id: &str) -> bool {
        self.confirmations.approve(id, Timestamp::now())
    }

    /// Una persona del PC rechaza la confirmacion pendiente.
    pub fn refuse_confirmation(&mut self, id: &str) -> bool {
        self.confirmations.refuse(id, Timestamp::now())
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

    /// Perfil de calidad en uso.
    ///
    /// Lo pide el dispositivo y el host lo valida. El bucle de la sesion lo
    /// consulta para poner al dia al hilo que captura.
    #[must_use]
    pub const fn quality(&self) -> QualityProfile {
        self.quality
    }

    /// Cuantas veces ha pedido esta sesion la pantalla.
    ///
    /// Cero significa que todavia no la ha pedido, y por tanto que no se
    /// captura nada.
    #[must_use]
    pub const fn screen_requests(&self) -> u64 {
        self.screen_requests
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
        let (id, request) = match Self::decode(raw) {
            Ok(decodificado) => decodificado,
            // Si ni siquiera se pudo leer la peticion, no hay identificador
            // que devolver. Se contesta con el de la ultima que si se leyo,
            // que es lo unico que hay; quien pregunta vera que no cuadra con
            // nada pendiente y sabra que algo envio mal.
            Err(reason) => return self.deny("bad_request", &reason).map(Some),
        };
        self.current_request = id;

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

        self.dispatch(request)
    }

    /// Ejecuta una orden cuyo permiso ya se comprobo.
    ///
    /// Esta separada de [`SessionHandler::handle`] para que la comprobacion
    /// de permiso sea lo unico que haya antes del despacho: una orden solo
    /// puede llegar aqui habiendo pasado por ella.
    fn dispatch(&mut self, request: ControlRequest) -> HostResult<Option<String>> {
        match request {
            ControlRequest::Input { event } => self.handle_input(&event),
            ControlRequest::ListMonitors => {
                self.applied = self.applied.saturating_add(1);
                self.respond(ControlResponse::Monitors {
                    layout: self.monitors.clone(),
                })
                .map(Some)
            }
            // El permiso ya se comprobo arriba, para todas por igual.
            ControlRequest::StartScreen => {
                self.screen_requests = self.screen_requests.saturating_add(1);
                self.applied = self.applied.saturating_add(1);
                self.respond(ControlResponse::Ack).map(Some)
            }
            ControlRequest::SelectMonitor { index } => {
                if self.monitors.0.iter().any(|m| m.index == index) {
                    self.active_monitor = index;
                    self.applied = self.applied.saturating_add(1);
                    self.respond(ControlResponse::Ack).map(Some)
                } else {
                    self.deny("unknown_monitor", &format!("el monitor {index} no existe"))
                        .map(Some)
                }
            }
            ControlRequest::ListDir { path, offset } => {
                let hecho = self.files.list(&path, offset).map(Respuesta::Control);
                self.answer_file(hecho)
            }
            ControlRequest::DownloadStart { path } => {
                let hecho = self.files.start_download(&path, Timestamp::now()).map(
                    |(response, download)| {
                        // No se envia nada aqui: la descarga sale en su propia
                        // tarea para no dejar el raton esperando detras.
                        self.pending_downloads.push(download);
                        Respuesta::Control(response)
                    },
                );
                self.answer_file(hecho)
            }
            ControlRequest::UploadStart { path, size_bytes } => {
                let hecho = self
                    .files
                    .start_upload(&path, size_bytes, Timestamp::now())
                    .map(Respuesta::Control);
                self.answer_file(hecho)
            }
            ControlRequest::TransferCancel { transfer_id } => {
                let hecho = self
                    .files
                    .cancel(transfer_id, Timestamp::now())
                    .map(Respuesta::Evento);
                self.answer_file(hecho)
            }
            ControlRequest::Power {
                action,
                confirmation_token,
            } => self.handle_power(action, confirmation_token.as_deref()),
            ControlRequest::ClipboardGet => {
                // El portapapeles del PC puede tener mucho mas de lo que cabe
                // en un mensaje de control, asi que lo leido se comprueba
                // igual que lo que llega de fuera.
                let hecho = self
                    .clipboard
                    .read_text()
                    .and_then(|texto| check_text(&texto).map(|()| texto))
                    .map(|text| ControlResponse::ClipboardText { text });
                self.answer_clipboard(hecho)
            }
            ControlRequest::ClipboardSet { text } => {
                let hecho = self
                    .clipboard
                    .write_text(&text)
                    .map(|()| ControlResponse::Ack);
                self.answer_clipboard(hecho)
            }
            ControlRequest::SystemStatus => {
                self.applied = self.applied.saturating_add(1);
                self.respond(ControlResponse::SystemStatus {
                    status: crate::system::read_status(),
                })
                .map(Some)
            }
            ControlRequest::SetQuality { profile } => {
                // Un cliente manipulado podria pedir 10 000 fotogramas o una
                // tasa de bits absurda. Lo que llega por la red se valida
                // siempre, aunque el cliente propio nunca enviaria eso.
                match profile.validate() {
                    Ok(()) => {
                        self.quality = profile;
                        self.applied = self.applied.saturating_add(1);
                        self.respond(ControlResponse::Ack).map(Some)
                    }
                    Err(error) => self.deny("invalid_quality", &error.to_string()).map(Some),
                }
            }
            // La ultima que queda del protocolo de control. Se contesta que
            // falta, que es la verdad, en lugar de aceptar una preferencia
            // sobre un audio que todavia no se transmite.
            ControlRequest::SetAudioEnabled { .. } => self
                .deny(
                    "not_implemented",
                    "el host todavia no captura audio, asi que activarlo no haria nada",
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
                self.refused = self.refused.saturating_add(1);
                // Un evento y no una respuesta: esto no contesta a ninguna
                // peticion del canal de control, asi que no hay identificador
                // con el que marcarla. Lleva el de la transferencia cuando el
                // marco permite saber de cual era.
                Self::event(HostEvent::TransferFailed {
                    transfer_id: nexus_file_transfer::Frame::parse(raw)
                        .ok()
                        .map(|f| f.transfer_id()),
                    code: deny_code(&error).to_owned(),
                    message: deny_message(&error),
                })
                .map(Some)
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
        // Una aprobacion no sobrevive a la sesion que la pidio: si la sesion
        // se cae y vuelve, hay que volver a preguntar.
        self.confirmations.clear();
    }

    /// Atiende una operacion de energia.
    ///
    /// Bloquear la sesion se hace sin mas: es reversible y no pierde nada.
    /// Reiniciar y apagar exigen que una persona delante del PC lo apruebe, y
    /// el token que se entrega aqui **no vale** hasta que eso ocurra.
    fn handle_power(
        &mut self,
        action: PowerAction,
        token: Option<&str>,
    ) -> HostResult<Option<String>> {
        if !action.requires_confirmation() {
            return self.run_power(action);
        }

        let now = Timestamp::now();
        let Some(token) = token else {
            // Primera vez: se pide la confirmacion y se entrega el token. No
            // se ejecuta nada, y la cuenta de ordenes aplicadas no sube:
            // todavia no ha pasado nada en el equipo.
            let pedido = self.confirmations.request(action, now);
            return self
                .respond(ControlResponse::ConfirmationRequired {
                    token: pedido.token,
                    prompt: pedido.prompt,
                })
                .map(Some);
        };

        match self.confirmations.redeem(action, token, now) {
            Redeem::Execute => self.run_power(action),
            Redeem::Waiting => self
                .deny(
                    "awaiting_confirmation",
                    "falta que alguien lo apruebe en el equipo",
                )
                .map(Some),
            Redeem::Refused => self
                .deny("refused_at_host", "lo han rechazado en el equipo")
                .map(Some),
            Redeem::Invalid(motivo) => self.deny("invalid_confirmation", motivo).map(Some),
        }
    }

    /// Ejecuta la operacion de energia de verdad.
    fn run_power(&mut self, action: PowerAction) -> HostResult<Option<String>> {
        match self.power.execute(action) {
            Ok(()) => {
                tracing::warn!(?action, "operacion de energia ejecutada");
                self.applied = self.applied.saturating_add(1);
                self.respond(ControlResponse::Ack).map(Some)
            }
            Err(error) => {
                let code = match error {
                    PowerError::UnsupportedPlatform { .. } => "not_implemented",
                    PowerError::PrivilegeDenied => "privilege_denied",
                    PowerError::System(_) => "power_failed",
                };
                let message = match &error {
                    PowerError::System(detalle) => {
                        tracing::warn!(detalle, "la operacion de energia fallo");
                        "el equipo no pudo completar la operacion".to_owned()
                    }
                    otro => otro.to_string(),
                };
                self.deny(code, &message).map(Some)
            }
        }
    }

    /// Traduce un evento de entrada a coordenadas reales y lo inyecta.
    ///
    /// Un evento valido no lleva respuesta: la pantalla ya es la respuesta.
    /// Solo se contesta cuando algo falla, para que el otro extremo sepa por
    /// que no se movio nada.
    fn handle_input(&mut self, event: &InputEvent) -> HostResult<Option<String>> {
        let resolved = match resolve(event, &self.monitors, self.active_monitor) {
            Ok(resolved) => resolved,
            Err(error) => return self.deny("invalid_input", &error.to_string()).map(Some),
        };
        match self.injector.inject(&resolved) {
            Ok(()) => {
                self.applied = self.applied.saturating_add(1);
                Ok(None)
            }
            Err(error) => self.deny("injection_failed", &error.to_string()).map(Some),
        }
    }

    /// Contesta a una operacion de archivos, sea respuesta o evento.
    fn answer_file(
        &mut self,
        hecho: Result<Respuesta, nexus_file_transfer::error::FileError>,
    ) -> HostResult<Option<String>> {
        match hecho {
            Ok(respuesta) => {
                self.applied = self.applied.saturating_add(1);
                match respuesta {
                    Respuesta::Control(response) => self.respond(response).map(Some),
                    Respuesta::Evento(event) => Self::event(event).map(Some),
                }
            }
            Err(error) => self.refuse_file(&error).map(Some),
        }
    }

    /// Contesta a una operacion de portapapeles.
    fn answer_clipboard(
        &mut self,
        hecho: nexus_clipboard::ClipboardResult<ControlResponse>,
    ) -> HostResult<Option<String>> {
        match hecho {
            Ok(response) => {
                self.applied = self.applied.saturating_add(1);
                self.respond(response).map(Some)
            }
            Err(error) => self.refuse_clipboard(&error).map(Some),
        }
    }

    /// Rechaza una operacion de portapapeles con su codigo estable.
    fn refuse_clipboard(&mut self, error: &ClipboardError) -> HostResult<String> {
        let code = match error {
            ClipboardError::TooLarge { .. } => "too_large",
            ClipboardError::InteriorNul => "invalid_text",
            ClipboardError::Empty => "clipboard_empty",
            ClipboardError::Busy => "clipboard_busy",
            ClipboardError::UnsupportedPlatform { .. } => "not_implemented",
            ClipboardError::System(_) => "clipboard_failed",
        };
        // El mensaje del sistema se queda en el registro del host, no viaja:
        // puede llevar detalles del equipo que quien pregunta no necesita.
        let message = match error {
            ClipboardError::System(detalle) => {
                tracing::warn!(detalle, "el portapapeles del sistema fallo");
                "el portapapeles del equipo no respondio".to_owned()
            }
            otro => otro.to_string(),
        };
        self.deny(code, &message)
    }

    /// Rechaza una operacion de archivos con su codigo estable.
    fn refuse_file(&mut self, error: &FileError) -> HostResult<String> {
        self.deny(deny_code(error), &deny_message(error))
    }

    /// Codifica un aviso del host para el canal de control.
    ///
    /// Es publica porque hay avisos que no nacen de una peticion ni los
    /// descubre el manejador: la captura de pantalla se rompe en su propio
    /// hilo, y quien se entera es el bucle de la sesion.
    ///
    /// # Errors
    ///
    /// [`HostError::Protocol`] si el aviso no se puede codificar.
    pub fn event(event: HostEvent) -> HostResult<String> {
        Envelope::at(Timestamp::now(), ControlMessage::Event(event))
            .encode()
            .map_err(|e| HostError::Protocol(e.to_string()))
    }

    fn decode(raw: &str) -> Result<(RequestId, ControlRequest), String> {
        let envelope: Envelope<ControlMessage> =
            Envelope::decode(raw).map_err(|e: CoreError| e.to_string())?;
        match envelope.payload {
            ControlMessage::Request { id, request } => Ok((id, request)),
            ControlMessage::Response { .. } | ControlMessage::Event(_) => {
                Err("por el canal de control solo se admiten peticiones".to_owned())
            }
        }
    }

    fn deny(&mut self, code: &str, message: &str) -> HostResult<String> {
        self.refused = self.refused.saturating_add(1);
        self.respond(ControlResponse::Denied {
            code: code.to_owned(),
            message: message.to_owned(),
        })
    }

    fn respond(&self, response: ControlResponse) -> HostResult<String> {
        Envelope::at(
            Timestamp::now(),
            ControlMessage::response(self.current_request, response),
        )
        .encode()
        .map_err(|e| HostError::Protocol(e.to_string()))
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
        /// Ultimo identificador enviado, para comprobar la correlacion.
        enviado: RequestId,
    }

    impl Probe {
        fn with(permissions: PermissionSet) -> Self {
            let injector = SharedInjector::default();
            Self {
                handler: SessionHandler::new(permissions, monitors(), Box::new(injector.clone())),
                injector,
                enviado: RequestId::FIRST,
            }
        }

        /// Envia una peticion y devuelve la respuesta, comprobando de paso
        /// que viene marcada con el identificador que se mando.
        fn send(&mut self, request: ControlRequest) -> Option<ControlResponse> {
            self.enviado = self.enviado.next();
            let enviado = self.enviado;
            let raw = Envelope::at(
                Timestamp(1_730_000_000_000),
                ControlMessage::request(enviado, request),
            )
            .encode()
            .expect("codifica");
            let answer = self.handler.handle(&raw).expect("no debe fallar");
            answer.map(|raw| {
                let envelope: Envelope<ControlMessage> =
                    Envelope::decode(&raw).expect("la respuesta cumple el protocolo");
                match envelope.payload {
                    ControlMessage::Response { id, response } => {
                        assert_eq!(
                            id, enviado,
                            "la respuesta tiene que llevar el identificador de su peticion"
                        );
                        response
                    }
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
        (
            Probe {
                handler,
                injector,
                enviado: RequestId::FIRST,
            },
            temp,
            compartida,
        )
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

    /// Manejador con un portapapeles en memoria que la prueba puede mirar.
    fn probe_con_portapapeles(
        permissions: PermissionSet,
        inicial: Option<&str>,
    ) -> (Probe, std::sync::Arc<std::sync::Mutex<Option<String>>>) {
        let compartido = std::sync::Arc::new(std::sync::Mutex::new(
            inicial.map(std::borrow::ToOwned::to_owned),
        ));
        let injector = SharedInjector::default();
        let handler = SessionHandler::new(permissions, monitors(), Box::new(injector.clone()))
            .with_clipboard(Box::new(PortapapelesCompartido(std::sync::Arc::clone(
                &compartido,
            ))));
        (
            Probe {
                handler,
                injector,
                enviado: RequestId::FIRST,
            },
            compartido,
        )
    }

    /// Portapapeles de prueba cuyo contenido vive fuera del manejador.
    struct PortapapelesCompartido(std::sync::Arc<std::sync::Mutex<Option<String>>>);

    impl nexus_clipboard::Clipboard for PortapapelesCompartido {
        fn name(&self) -> &'static str {
            "compartido"
        }

        fn read_text(&mut self) -> nexus_clipboard::ClipboardResult<String> {
            self.0
                .lock()
                .expect("sin envenenar")
                .clone()
                .ok_or(ClipboardError::Empty)
        }

        fn write_text(&mut self, text: &str) -> nexus_clipboard::ClipboardResult<()> {
            check_text(text)?;
            *self.0.lock().expect("sin envenenar") = Some(text.to_owned());
            Ok(())
        }

        fn probe(&mut self) -> nexus_clipboard::ClipboardResult<bool> {
            Ok(self.0.lock().expect("sin envenenar").is_some())
        }
    }

    #[test]
    fn lo_copiado_en_el_pc_llega_al_dispositivo() {
        let (mut probe, _) =
            probe_con_portapapeles(PermissionSet::standard(), Some("texto copiado en el PC"));
        match probe.send(ControlRequest::ClipboardGet) {
            Some(ControlResponse::ClipboardText { text }) => {
                assert_eq!(text, "texto copiado en el PC");
            }
            otro => panic!("se esperaba el texto, llego {otro:?}"),
        }
    }

    #[test]
    fn lo_copiado_en_el_dispositivo_llega_al_pc() {
        let (mut probe, contenido) = probe_con_portapapeles(PermissionSet::standard(), None);
        assert_eq!(
            probe.send(ControlRequest::ClipboardSet {
                text: "escrito desde el iPad".to_owned(),
            }),
            Some(ControlResponse::Ack)
        );
        assert_eq!(
            contenido.lock().expect("sin envenenar").as_deref(),
            Some("escrito desde el iPad")
        );
    }

    #[test]
    fn un_portapapeles_vacio_se_dice_en_lugar_de_devolver_cadena_vacia() {
        // Una cadena vacia pareceria un portapapeles con un texto en blanco.
        let (mut probe, _) = probe_con_portapapeles(PermissionSet::standard(), None);
        match probe.send(ControlRequest::ClipboardGet) {
            Some(ControlResponse::Denied { code, .. }) => {
                assert_eq!(code, "clipboard_empty");
            }
            otro => panic!("deberia decir que esta vacio, llego {otro:?}"),
        }
    }

    #[test]
    fn un_texto_demasiado_grande_se_rechaza_en_las_dos_direcciones() {
        let enorme = "x".repeat(nexus_clipboard::MAX_TEXT_BYTES + 1);

        // Al escribir desde el dispositivo.
        let (mut probe, contenido) = probe_con_portapapeles(PermissionSet::standard(), None);
        match probe.send(ControlRequest::ClipboardSet {
            text: enorme.clone(),
        }) {
            Some(ControlResponse::Denied { code, .. }) => assert_eq!(code, "too_large"),
            otro => panic!("deberia rechazarse, llego {otro:?}"),
        }
        assert!(
            contenido.lock().expect("sin envenenar").is_none(),
            "un rechazo no debe tocar el portapapeles del PC"
        );

        // Y al leer del PC, que es el caso que de verdad sorprende: el
        // portapapeles del equipo puede tener mucho mas de lo que cabe.
        let (mut probe, _) = probe_con_portapapeles(PermissionSet::standard(), Some(&enorme));
        match probe.send(ControlRequest::ClipboardGet) {
            Some(ControlResponse::Denied { code, .. }) => assert_eq!(code, "too_large"),
            otro => panic!("deberia rechazarse al leer, llego {otro:?}"),
        }
    }

    #[test]
    fn un_texto_con_byte_cero_no_llega_al_portapapeles_del_pc() {
        let (mut probe, contenido) = probe_con_portapapeles(PermissionSet::standard(), None);
        match probe.send(ControlRequest::ClipboardSet {
            text: "contrase\0na".to_owned(),
        }) {
            Some(ControlResponse::Denied { code, .. }) => assert_eq!(code, "invalid_text"),
            otro => panic!("deberia rechazarse, llego {otro:?}"),
        }
        assert!(contenido.lock().expect("sin envenenar").is_none());
    }

    #[test]
    fn sin_permiso_de_lectura_no_se_lee_el_portapapeles() {
        let (mut probe, _) = probe_con_portapapeles(
            PermissionSet::from_iter([Permission::ClipboardWrite]),
            Some("secreto"),
        );
        match probe.send(ControlRequest::ClipboardGet) {
            Some(ControlResponse::Denied { code, .. }) => {
                assert_eq!(code, "permission_denied");
            }
            otro => panic!("deberia rechazarse, llego {otro:?}"),
        }
    }

    #[test]
    fn sin_permiso_de_escritura_no_se_toca_el_portapapeles_del_pc() {
        // El limite que importa: poder leer no da derecho a escribir.
        let (mut probe, contenido) = probe_con_portapapeles(
            PermissionSet::from_iter([Permission::ClipboardRead]),
            Some("lo que habia"),
        );
        match probe.send(ControlRequest::ClipboardSet {
            text: "intruso".to_owned(),
        }) {
            Some(ControlResponse::Denied { code, .. }) => {
                assert_eq!(code, "permission_denied");
            }
            otro => panic!("deberia rechazarse, llego {otro:?}"),
        }
        assert_eq!(
            contenido.lock().expect("sin envenenar").as_deref(),
            Some("lo que habia"),
            "lo que ya estaba no debe cambiar"
        );
    }

    #[test]
    fn sin_portapapeles_en_esta_plataforma_se_dice_claramente() {
        // El manejador por omision usa el del sistema, que en Linux no existe.
        let mut probe = Probe::with(PermissionSet::standard());
        match probe.send(ControlRequest::ClipboardGet) {
            Some(ControlResponse::Denied { code, message }) => {
                if cfg!(windows) {
                    // En Windows si hay backend: el portapapeles puede estar
                    // vacio, pero no «no implementado».
                    assert_ne!(code, "not_implemented", "{message}");
                } else {
                    assert_eq!(code, "not_implemented");
                }
            }
            otro if cfg!(windows) => {
                // En Windows puede contestar con el texto de verdad.
                assert!(matches!(otro, Some(ControlResponse::ClipboardText { .. })));
            }
            otro => panic!("deberia rechazarse fuera de Windows, llego {otro:?}"),
        }
    }

    /// Manejador con un backend de energia que anota en lugar de apagar.
    fn probe_con_energia(permissions: PermissionSet) -> (Probe, EnergiaCompartida) {
        let energia = EnergiaCompartida::default();
        let injector = SharedInjector::default();
        let handler = SessionHandler::new(permissions, monitors(), Box::new(injector.clone()))
            .with_power(Box::new(energia.clone()));
        (
            Probe {
                handler,
                injector,
                enviado: RequestId::FIRST,
            },
            energia,
        )
    }

    /// Backend de energia cuyo registro vive fuera del manejador.
    #[derive(Clone, Default)]
    struct EnergiaCompartida(Arc<Mutex<Vec<PowerAction>>>);

    impl EnergiaCompartida {
        fn ejecutadas(&self) -> Vec<PowerAction> {
            self.0.lock().expect("sin envenenar").clone()
        }
    }

    impl nexus_power::PowerControl for EnergiaCompartida {
        fn name(&self) -> &'static str {
            "compartido"
        }

        fn execute(&mut self, action: PowerAction) -> nexus_power::PowerResult<()> {
            self.0.lock().expect("sin envenenar").push(action);
            Ok(())
        }

        fn probe(&mut self) -> nexus_power::PowerResult<()> {
            Ok(())
        }
    }

    /// Pide una operacion de energia y devuelve lo que conteste el host.
    fn pedir_energia(
        probe: &mut Probe,
        action: PowerAction,
        token: Option<String>,
    ) -> Option<ControlResponse> {
        probe.send(ControlRequest::Power {
            action,
            confirmation_token: token,
        })
    }

    #[test]
    fn bloquear_la_sesion_no_necesita_confirmacion() {
        // Es reversible y no pierde nada: pedir confirmacion para esto solo
        // haria que la gente aprobara sin leer.
        let (mut probe, energia) = probe_con_energia(PermissionSet::full());
        assert_eq!(
            pedir_energia(&mut probe, PowerAction::Lock, None),
            Some(ControlResponse::Ack)
        );
        assert_eq!(energia.ejecutadas(), [PowerAction::Lock]);
    }

    #[test]
    fn con_el_token_en_la_mano_pero_sin_aprobar_no_se_apaga_nada() {
        // ESTA es la prueba que justifica todo el modulo de confirmacion.
        // Quien pide recibe el token al instante; si eso bastara, la
        // confirmacion no confirmaria nada.
        let (mut probe, energia) = probe_con_energia(PermissionSet::full());

        let token = match pedir_energia(&mut probe, PowerAction::Shutdown, None) {
            Some(ControlResponse::ConfirmationRequired { token, prompt }) => {
                assert!(prompt.contains("Apagar"), "{prompt}");
                token
            }
            otro => panic!("se esperaba una confirmacion, llego {otro:?}"),
        };

        match pedir_energia(&mut probe, PowerAction::Shutdown, Some(token)) {
            Some(ControlResponse::Denied { code, .. }) => {
                assert_eq!(code, "awaiting_confirmation");
            }
            otro => panic!("deberia seguir esperando, llego {otro:?}"),
        }
        assert!(
            energia.ejecutadas().is_empty(),
            "no debe haberse ejecutado nada: nadie lo aprobo"
        );
    }

    #[test]
    fn tras_aprobarlo_en_el_pc_el_apagado_se_ejecuta() {
        let (mut probe, energia) = probe_con_energia(PermissionSet::full());

        let token = match pedir_energia(&mut probe, PowerAction::Reboot, None) {
            Some(ControlResponse::ConfirmationRequired { token, .. }) => token,
            otro => panic!("se esperaba una confirmacion, llego {otro:?}"),
        };

        let id = probe
            .handler
            .pending_confirmation()
            .expect("el panel deberia ver una pendiente")
            .id
            .clone();
        assert!(probe.handler.approve_confirmation(&id));

        assert_eq!(
            pedir_energia(&mut probe, PowerAction::Reboot, Some(token)),
            Some(ControlResponse::Ack)
        );
        assert_eq!(energia.ejecutadas(), [PowerAction::Reboot]);
    }

    #[test]
    fn si_lo_rechazan_en_el_pc_no_se_apaga() {
        let (mut probe, energia) = probe_con_energia(PermissionSet::full());
        let token = match pedir_energia(&mut probe, PowerAction::Shutdown, None) {
            Some(ControlResponse::ConfirmationRequired { token, .. }) => token,
            otro => panic!("se esperaba una confirmacion, llego {otro:?}"),
        };

        let id = probe
            .handler
            .pending_confirmation()
            .expect("hay una pendiente")
            .id
            .clone();
        assert!(probe.handler.refuse_confirmation(&id));

        match pedir_energia(&mut probe, PowerAction::Shutdown, Some(token)) {
            Some(ControlResponse::Denied { code, .. }) => assert_eq!(code, "refused_at_host"),
            otro => panic!("deberia rechazarse, llego {otro:?}"),
        }
        assert!(energia.ejecutadas().is_empty());
    }

    #[test]
    fn un_token_inventado_no_apaga_el_equipo() {
        let (mut probe, energia) = probe_con_energia(PermissionSet::full());
        pedir_energia(&mut probe, PowerAction::Shutdown, None);
        let id = probe
            .handler
            .pending_confirmation()
            .expect("hay una")
            .id
            .clone();
        probe.handler.approve_confirmation(&id);

        for falso in ["", "0", &"f".repeat(64)] {
            match pedir_energia(&mut probe, PowerAction::Shutdown, Some(falso.to_owned())) {
                Some(ControlResponse::Denied { code, .. }) => {
                    assert_eq!(code, "invalid_confirmation", "{falso:?}");
                }
                otro => panic!("{falso:?} no deberia valer, llego {otro:?}"),
            }
        }
        assert!(energia.ejecutadas().is_empty());
    }

    #[test]
    fn la_confirmacion_de_un_reinicio_no_sirve_para_apagar() {
        let (mut probe, energia) = probe_con_energia(PermissionSet::full());
        let token = match pedir_energia(&mut probe, PowerAction::Reboot, None) {
            Some(ControlResponse::ConfirmationRequired { token, .. }) => token,
            otro => panic!("se esperaba una confirmacion, llego {otro:?}"),
        };
        let id = probe
            .handler
            .pending_confirmation()
            .expect("hay una")
            .id
            .clone();
        probe.handler.approve_confirmation(&id);

        match pedir_energia(&mut probe, PowerAction::Shutdown, Some(token)) {
            Some(ControlResponse::Denied { code, .. }) => assert_eq!(code, "invalid_confirmation"),
            otro => panic!("deberia rechazarse, llego {otro:?}"),
        }
        assert!(energia.ejecutadas().is_empty());
    }

    #[test]
    fn sin_permiso_no_se_apaga_aunque_se_apruebe() {
        // El permiso se comprueba antes que nada: sin el, ni siquiera se
        // llega a crear una confirmacion que alguien pudiera aprobar.
        let (mut probe, energia) = probe_con_energia(PermissionSet::standard());
        match pedir_energia(&mut probe, PowerAction::Shutdown, None) {
            Some(ControlResponse::Denied { code, .. }) => {
                assert_eq!(code, "permission_denied");
            }
            otro => panic!("deberia rechazarse, llego {otro:?}"),
        }
        assert!(
            probe.handler.pending_confirmation().is_none(),
            "no debe aparecer nada en el panel del host"
        );
        assert!(energia.ejecutadas().is_empty());
    }

    #[test]
    fn cerrar_la_sesion_invalida_una_aprobacion_pendiente() {
        let (mut probe, energia) = probe_con_energia(PermissionSet::full());
        let token = match pedir_energia(&mut probe, PowerAction::Shutdown, None) {
            Some(ControlResponse::ConfirmationRequired { token, .. }) => token,
            otro => panic!("se esperaba una confirmacion, llego {otro:?}"),
        };
        let id = probe
            .handler
            .pending_confirmation()
            .expect("hay una")
            .id
            .clone();
        probe.handler.approve_confirmation(&id);

        probe.handler.end();

        match pedir_energia(&mut probe, PowerAction::Shutdown, Some(token)) {
            Some(ControlResponse::Denied { code, .. }) => assert_eq!(code, "invalid_confirmation"),
            otro => panic!("deberia rechazarse, llego {otro:?}"),
        }
        assert!(energia.ejecutadas().is_empty());
    }

    #[test]
    fn insistir_no_llena_el_panel_de_avisos() {
        let (mut probe, _) = probe_con_energia(PermissionSet::full());
        let primero = match pedir_energia(&mut probe, PowerAction::Reboot, None) {
            Some(ControlResponse::ConfirmationRequired { token, .. }) => token,
            otro => panic!("llego {otro:?}"),
        };
        let segundo = match pedir_energia(&mut probe, PowerAction::Reboot, None) {
            Some(ControlResponse::ConfirmationRequired { token, .. }) => token,
            otro => panic!("llego {otro:?}"),
        };
        assert_eq!(primero, segundo, "deberia ser la misma confirmacion");
    }

    #[test]
    fn el_estado_del_equipo_son_datos_de_verdad() {
        let mut probe = Probe::with(PermissionSet::standard());
        match probe.send(ControlRequest::SystemStatus) {
            Some(ControlResponse::SystemStatus { status }) => {
                assert!(!status.hostname.is_empty());
                assert!(!status.os_description.is_empty());
                assert!(status.cpu_cores >= 1);
                assert!(status.memory_total_mib > 0, "la memoria no puede ser cero");
                assert!(status.memory_available_mib <= status.memory_total_mib);
                assert!(
                    status.session_locked.is_none(),
                    "mientras no se detecte, debe decir que no se sabe"
                );
            }
            otro => panic!("se esperaba el estado del equipo, llego {otro:?}"),
        }
    }

    #[test]
    fn sin_permiso_de_informacion_no_se_da_el_estado_del_equipo() {
        let mut probe = Probe::with(PermissionSet::from_iter([Permission::ViewScreen]));
        match probe.send(ControlRequest::SystemStatus) {
            Some(ControlResponse::Denied { code, .. }) => {
                assert_eq!(code, "permission_denied");
            }
            otro => panic!("deberia rechazarse, llego {otro:?}"),
        }
    }

    #[test]
    fn un_perfil_de_calidad_valido_se_acepta_y_se_guarda() {
        let mut probe = Probe::with(PermissionSet::standard());
        let nitido = QualityProfile::from_preset(QualityPreset::Nitido);
        assert_eq!(
            probe.send(ControlRequest::SetQuality { profile: nitido }),
            Some(ControlResponse::Ack)
        );
        assert_eq!(probe.handler.quality(), nitido);
    }

    #[test]
    fn un_perfil_de_calidad_imposible_se_rechaza_y_no_cambia_nada() {
        // El caso que importa: un cliente manipulado pidiendo algo absurdo.
        let mut probe = Probe::with(PermissionSet::standard());
        let antes = probe.handler.quality();

        for profile in [
            QualityProfile {
                target_fps: 10_000,
                ..QualityProfile::from_preset(QualityPreset::Equilibrado)
            },
            QualityProfile {
                target_bitrate_kbps: u32::MAX,
                ..QualityProfile::from_preset(QualityPreset::Equilibrado)
            },
            QualityProfile {
                scale_percent: 0,
                ..QualityProfile::from_preset(QualityPreset::Equilibrado)
            },
            QualityProfile {
                target_fps: 0,
                ..QualityProfile::from_preset(QualityPreset::Equilibrado)
            },
        ] {
            match probe.send(ControlRequest::SetQuality { profile }) {
                Some(ControlResponse::Denied { code, message }) => {
                    assert_eq!(code, "invalid_quality");
                    assert!(!message.is_empty(), "deberia decir que campo falla");
                }
                otro => panic!("{profile:?} deberia rechazarse, llego {otro:?}"),
            }
        }
        assert_eq!(
            probe.handler.quality(),
            antes,
            "un perfil rechazado no debe cambiar el que estaba en uso"
        );
    }

    #[test]
    fn sin_permiso_de_pantalla_no_se_cambia_la_calidad() {
        let mut probe = Probe::with(PermissionSet::from_iter([Permission::SystemInfo]));
        match probe.send(ControlRequest::SetQuality {
            profile: QualityProfile::from_preset(QualityPreset::Fluido),
        }) {
            Some(ControlResponse::Denied { code, .. }) => {
                assert_eq!(code, "permission_denied");
            }
            otro => panic!("deberia rechazarse, llego {otro:?}"),
        }
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
    fn una_operacion_que_falta_se_declara_pendiente_en_lugar_de_callar() {
        // El audio sigue sin motor, asi que se dice. Cuando se implemente,
        // esta prueba tendra que apuntar a otra cosa, o desaparecer porque ya
        // no quede ninguna.
        let mut probe = Probe::with(PermissionSet::full());
        match probe.send(ControlRequest::SetAudioEnabled { enabled: true }) {
            Some(ControlResponse::Denied { code, message }) => {
                assert_eq!(code, "not_implemented");
                assert!(
                    message.contains("audio"),
                    "el cliente tiene que saber que falta y por que: {message}"
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
        let raw = Envelope::at(
            Timestamp(1),
            ControlMessage::response(RequestId::FIRST, ControlResponse::Ack),
        )
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
