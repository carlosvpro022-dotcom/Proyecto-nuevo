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
use nexus_core::protocol::control::{ControlMessage, ControlRequest, ControlResponse};
use nexus_core::{CoreError, PermissionSet, Timestamp};
use nexus_input::{InputInjector, resolve};

use crate::error::{HostError, HostResult};

/// Atiende el canal de control de una sesion establecida.
pub struct SessionHandler {
    permissions: PermissionSet,
    monitors: MonitorLayout,
    active_monitor: u8,
    injector: Box<dyn InputInjector + Send>,
    applied: u64,
    refused: u64,
}

impl std::fmt::Debug for SessionHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandler")
            .field("permissions", &self.permissions)
            .field("active_monitor", &self.active_monitor)
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
            applied: 0,
            refused: 0,
        }
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
        ControlRequest::ListDir { .. } => "listar un directorio",
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
