//! Arranque, parada y vigilancia del bucle del host.
//!
//! El bucle ([`crate::runtime`]) no sabe nada de la interfaz ni del almacen:
//! solo emite lo que va pasando. Este modulo es quien lo pone en marcha, va
//! leyendo esos avisos, los escribe en el historial y mantiene un resumen que
//! el panel consulta.
//!
//! La separacion es util: el bucle se puede probar sin almacen ni interfaz, y
//! aqui no hay nada de red que probar.

use std::sync::{Arc, Mutex};

use nexus_core::{CloseReason, DeviceId, PermissionSet, SessionId, Timestamp};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};

use crate::error::{HostError, HostResult};
use crate::host::HostService;
use crate::runtime::RuntimeEvent;

/// Sesion que se esta atendiendo ahora mismo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActiveSession {
    /// Identificador de la sesion.
    pub session_id: String,
    /// Dispositivo que la tiene.
    pub controller: String,
    /// Permisos efectivos, por nombre.
    pub permissions: Vec<String>,
    /// Cuando empezo.
    pub started_at_ms: u64,
}

/// Resumen del bucle, tal como lo lee el panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeStatus {
    /// Si el bucle esta en marcha.
    pub running: bool,
    /// Si esta conectado al servidor y declarado disponible.
    pub available: bool,
    /// Si el servidor entrego relay TURN.
    pub has_relay: bool,
    /// Que esta pasando, en lenguaje llano.
    pub detail: String,
    /// Sesion en curso, si la hay.
    pub active_session: Option<ActiveSession>,
    /// Sesiones atendidas desde que arranco el bucle.
    pub sessions_served: u64,
    /// Sesiones rechazadas desde que arranco el bucle.
    pub sessions_refused: u64,
}

impl RuntimeStatus {
    /// Estado de un bucle que no esta en marcha.
    #[must_use]
    pub fn stopped() -> Self {
        Self {
            running: false,
            available: false,
            has_relay: false,
            detail: "El host no esta atendiendo sesiones.".to_owned(),
            active_session: None,
            sessions_served: 0,
            sessions_refused: 0,
        }
    }
}

impl Default for RuntimeStatus {
    fn default() -> Self {
        Self::stopped()
    }
}

/// Un bucle en marcha, con lo necesario para pararlo.
pub struct RuntimeHandle {
    stop: Option<oneshot::Sender<()>>,
    status: Arc<Mutex<RuntimeStatus>>,
}

impl std::fmt::Debug for RuntimeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeHandle").finish_non_exhaustive()
    }
}

impl RuntimeHandle {
    /// Resumen actual.
    ///
    /// # Panics
    ///
    /// No entra en panico: un cerrojo envenenado se recupera.
    #[must_use]
    pub fn status(&self) -> RuntimeStatus {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Pide al bucle que pare. No espera a que termine.
    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Canal de parada y resumen compartido de un bucle recien montado.
#[must_use]
pub fn prepare() -> (
    oneshot::Receiver<()>,
    Arc<Mutex<RuntimeStatus>>,
    RuntimeHandle,
) {
    let (stop, shutdown) = oneshot::channel();
    let status = Arc::new(Mutex::new(RuntimeStatus {
        running: true,
        detail: "Conectando con el servidor...".to_owned(),
        ..RuntimeStatus::stopped()
    }));
    let handle = RuntimeHandle {
        stop: Some(stop),
        status: Arc::clone(&status),
    };
    (shutdown, status, handle)
}

/// Va leyendo los avisos del bucle: actualiza el resumen y escribe el historial.
///
/// Termina cuando el bucle deja de emitir, es decir cuando para.
pub async fn consume(
    mut events: mpsc::Receiver<RuntimeEvent>,
    status: Arc<Mutex<RuntimeStatus>>,
    host: Arc<Mutex<HostService>>,
) {
    while let Some(event) = events.recv().await {
        if let Err(error) = apply(&event, &status, &host) {
            // Que falle el historial no puede tumbar la sesion en curso.
            tracing::error!(%error, "no se pudo anotar un hecho del bucle");
        }
    }

    let mut status = status
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    status.running = false;
    status.available = false;
    status.active_session = None;
    "El host dejo de atender sesiones.".clone_into(&mut status.detail);
}

/// Aplica un aviso al resumen y al historial.
///
/// # Errors
///
/// [`HostError::Database`] si el historial no se puede escribir.
fn apply(
    event: &RuntimeEvent,
    status: &Arc<Mutex<RuntimeStatus>>,
    host: &Arc<Mutex<HostService>>,
) -> HostResult<()> {
    let mut estado = status
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    match event {
        RuntimeEvent::Available {
            ice_servers,
            has_relay,
        } => {
            estado.available = true;
            estado.has_relay = *has_relay;
            estado.detail = if *has_relay {
                format!("Disponible. {ice_servers} servidores de red, con relay.")
            } else {
                format!(
                    "Disponible, pero sin relay TURN ({ice_servers} servidores): \
                     una conexion desde una red movil puede no establecerse."
                )
            };
        }
        RuntimeEvent::LinkLost { detail, retry_in } => {
            estado.available = false;
            estado.active_session = None;
            estado.detail = match retry_in {
                Some(espera) => format!(
                    "Sin conexion con el servidor ({detail}). Reintentando en {} s.",
                    espera.as_secs().max(1)
                ),
                None => format!("Sin conexion con el servidor ({detail})."),
            };
        }
        RuntimeEvent::GaveUp { attempts } => {
            estado.running = false;
            estado.available = false;
            estado.detail = format!(
                "No se pudo conectar con el servidor tras {attempts} intentos. \
                 Compruebe la direccion y que el servicio este en marcha."
            );
        }
        RuntimeEvent::SessionDecided {
            accepted, detail, ..
        } => {
            if !accepted {
                estado.sessions_refused = estado.sessions_refused.saturating_add(1);
                estado.detail = format!("Sesion rechazada: {detail}");
            }
        }
        RuntimeEvent::SessionStarted {
            session_id,
            controller,
            permissions,
        } => {
            let now = Timestamp::now();
            estado.active_session = Some(ActiveSession {
                session_id: session_id.to_string(),
                controller: controller.to_string(),
                permissions: permissions.iter().map(|p| p.name().to_owned()).collect(),
                started_at_ms: now.0,
            });
            estado.sessions_served = estado.sessions_served.saturating_add(1);
            "Sesion en curso.".clone_into(&mut estado.detail);
            drop(estado);

            return log_start(host, *session_id, *controller, now);
        }
        RuntimeEvent::SessionEnded {
            session_id,
            reason,
            detail,
            applied,
            refused,
        } => {
            estado.active_session = None;
            estado.detail =
                format!("Sesion terminada: {detail}. {applied} ordenes, {refused} rechazadas.");
            drop(estado);

            return log_end(host, *session_id, *reason);
        }
    }

    Ok(())
}

fn log_start(
    host: &Arc<Mutex<HostService>>,
    session_id: SessionId,
    controller: DeviceId,
    now: Timestamp,
) -> HostResult<()> {
    let host = host
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    host.store()
        .log_session_start(session_id, controller, now, None)
}

fn log_end(
    host: &Arc<Mutex<HostService>>,
    session_id: SessionId,
    reason: CloseReason,
) -> HostResult<()> {
    let host = host
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    host.store()
        .log_session_end(session_id, Timestamp::now(), reason)
}

/// Consulta el registro de dispositivos del host, en vivo.
///
/// Es en vivo a proposito y no una foto tomada al arrancar: si se revoca un
/// dispositivo mientras el host esta atendiendo, la siguiente peticion suya
/// tiene que encontrarse ya con la revocacion.
pub struct StoreLookup(Arc<Mutex<HostService>>);

impl StoreLookup {
    /// Crea la consulta sobre el servicio compartido.
    #[must_use]
    pub const fn new(host: Arc<Mutex<HostService>>) -> Self {
        Self(host)
    }
}

impl std::fmt::Debug for StoreLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreLookup").finish_non_exhaustive()
    }
}

impl crate::runtime::DeviceLookup for StoreLookup {
    fn get(&self, device_id: DeviceId) -> Option<nexus_security::registry::DeviceRecord> {
        let host = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match host.store().load_device(device_id) {
            Ok(record) => record,
            Err(error) => {
                // No poder leer el registro no es "no esta autorizado": es un
                // fallo. Se anota y se niega la sesion, que es lo prudente.
                tracing::error!(%error, "no se pudo leer el registro de dispositivos");
                None
            }
        }
    }
}

/// Permisos de una sesion en el formato que espera el panel.
#[must_use]
pub fn permission_names(permissions: &PermissionSet) -> Vec<String> {
    permissions.iter().map(|p| p.name().to_owned()).collect()
}

/// Error cuando se pide arrancar un bucle que ya esta en marcha.
#[must_use]
pub fn already_running() -> HostError {
    HostError::InvalidInput("el host ya esta atendiendo sesiones".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::Permission;

    fn estado_compartido() -> Arc<Mutex<RuntimeStatus>> {
        Arc::new(Mutex::new(RuntimeStatus {
            running: true,
            ..RuntimeStatus::stopped()
        }))
    }

    fn servicio() -> Arc<Mutex<HostService>> {
        Arc::new(Mutex::new(
            HostService::open_ephemeral(nexus_security::DeviceIdentity::generate(DeviceId::new()))
                .expect("host en memoria"),
        ))
    }

    /// Un servicio con un dispositivo emparejado.
    ///
    /// El historial tiene clave foranea contra la tabla de dispositivos, que es
    /// lo correcto: no puede haber una sesion de alguien que no esta
    /// emparejado.
    fn servicio_con_dispositivo(controller: DeviceId) -> Arc<Mutex<HostService>> {
        use nexus_core::device::{DeviceInfo, DevicePlatform, DeviceRole};
        use nexus_security::registry::DeviceRecord;

        let servicio = servicio();
        let info = DeviceInfo {
            id: controller,
            display_name: "iPad Pro".to_owned(),
            platform: DevicePlatform::IpadOs,
            role: DeviceRole::Controller,
            os_version: "18.1".to_owned(),
            app_version: "0.1.0".to_owned(),
        };
        let record = DeviceRecord::new(
            info,
            nexus_security::DeviceIdentity::generate(controller).public_key(),
            PermissionSet::standard(),
            true,
            Timestamp::now(),
        );
        servicio
            .lock()
            .expect("sin envenenar")
            .store()
            .save_device(controller, &record)
            .expect("guarda el dispositivo");
        servicio
    }

    fn aplicar(event: &RuntimeEvent, status: &Arc<Mutex<RuntimeStatus>>) -> RuntimeStatus {
        aplicar_con(event, status, &servicio())
    }

    fn aplicar_con(
        event: &RuntimeEvent,
        status: &Arc<Mutex<RuntimeStatus>>,
        host: &Arc<Mutex<HostService>>,
    ) -> RuntimeStatus {
        apply(event, status, host).expect("no deberia fallar");
        status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    #[test]
    fn un_bucle_parado_no_dice_estar_disponible() {
        let estado = RuntimeStatus::stopped();
        assert!(!estado.running);
        assert!(!estado.available);
        assert!(estado.active_session.is_none());
    }

    #[test]
    fn estar_disponible_sin_relay_se_avisa() {
        // Un host sin relay parece funcionar y luego falla desde una red movil.
        let compartido = estado_compartido();
        let estado = aplicar(
            &RuntimeEvent::Available {
                ice_servers: 1,
                has_relay: false,
            },
            &compartido,
        );
        assert!(estado.available);
        assert!(!estado.has_relay);
        assert!(estado.detail.contains("sin relay"), "{}", estado.detail);
    }

    #[test]
    fn estar_disponible_con_relay_lo_dice_sin_alarmar() {
        let compartido = estado_compartido();
        let estado = aplicar(
            &RuntimeEvent::Available {
                ice_servers: 2,
                has_relay: true,
            },
            &compartido,
        );
        assert!(estado.has_relay);
        assert!(!estado.detail.contains("puede no"), "{}", estado.detail);
    }

    #[test]
    fn perder_el_enlace_dice_cuanto_falta_para_reintentar() {
        let compartido = estado_compartido();
        let estado = aplicar(
            &RuntimeEvent::LinkLost {
                detail: "conexion rechazada".to_owned(),
                retry_in: Some(std::time::Duration::from_secs(4)),
            },
            &compartido,
        );
        assert!(!estado.available);
        assert!(estado.detail.contains("4 s"), "{}", estado.detail);
    }

    #[test]
    fn rendirse_explica_que_mirar() {
        let compartido = estado_compartido();
        let estado = aplicar(&RuntimeEvent::GaveUp { attempts: 5 }, &compartido);
        assert!(!estado.running);
        assert!(estado.detail.contains("direccion"), "{}", estado.detail);
    }

    #[test]
    fn una_sesion_rechazada_se_cuenta_y_se_explica() {
        let compartido = estado_compartido();
        let estado = aplicar(
            &RuntimeEvent::SessionDecided {
                controller: DeviceId::new(),
                accepted: false,
                detail: "no esta emparejado".to_owned(),
            },
            &compartido,
        );
        assert_eq!(estado.sessions_refused, 1);
        assert_eq!(estado.sessions_served, 0);
        assert!(estado.detail.contains("no esta emparejado"));
    }

    #[test]
    fn una_sesion_aceptada_no_cuenta_hasta_que_empieza() {
        // Aceptarla no es lo mismo que establecerla: entre medias esta la
        // comprobacion de quien hay al otro lado, que puede fallar.
        let compartido = estado_compartido();
        let estado = aplicar(
            &RuntimeEvent::SessionDecided {
                controller: DeviceId::new(),
                accepted: true,
                detail: "autorizado".to_owned(),
            },
            &compartido,
        );
        assert_eq!(estado.sessions_served, 0);
        assert!(estado.active_session.is_none());
    }

    #[test]
    fn una_sesion_en_curso_aparece_con_sus_permisos() {
        let compartido = estado_compartido();
        let controller = DeviceId::new();
        let estado = aplicar_con(
            &RuntimeEvent::SessionStarted {
                session_id: SessionId::new(),
                controller,
                permissions: PermissionSet::view_only(),
            },
            &compartido,
            &servicio_con_dispositivo(controller),
        );

        let activa = estado.active_session.expect("hay sesion");
        assert_eq!(activa.controller, controller.to_string());
        assert!(
            activa
                .permissions
                .contains(&Permission::ViewScreen.name().to_owned())
        );
        assert_eq!(estado.sessions_served, 1);
    }

    #[test]
    fn al_terminar_la_sesion_deja_de_figurar_como_activa() {
        let compartido = estado_compartido();
        let controller = DeviceId::new();
        let servicio = servicio_con_dispositivo(controller);
        let session_id = SessionId::new();

        apply(
            &RuntimeEvent::SessionStarted {
                session_id,
                controller,
                permissions: PermissionSet::standard(),
            },
            &compartido,
            &servicio,
        )
        .expect("anota el inicio");
        apply(
            &RuntimeEvent::SessionEnded {
                session_id,
                reason: CloseReason::PeerClosed,
                detail: "el otro extremo se desconecto",
                applied: 12,
                refused: 1,
            },
            &compartido,
            &servicio,
        )
        .expect("anota el fin");

        let estado = compartido
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert!(estado.active_session.is_none());
        assert!(estado.detail.contains("12 ordenes"), "{}", estado.detail);
    }

    #[test]
    fn la_sesion_queda_en_el_historial() {
        let compartido = estado_compartido();
        let session_id = SessionId::new();
        let controller = DeviceId::new();
        let servicio = servicio_con_dispositivo(controller);

        apply(
            &RuntimeEvent::SessionStarted {
                session_id,
                controller,
                permissions: PermissionSet::standard(),
            },
            &compartido,
            &servicio,
        )
        .expect("anota el inicio");

        let anotadas = servicio
            .lock()
            .expect("sin envenenar")
            .store()
            .session_count(controller)
            .expect("cuenta");
        assert_eq!(anotadas, 1, "la sesion debe quedar en el historial");
    }

    #[test]
    fn si_el_historial_falla_el_resumen_se_actualiza_igual() {
        // Si el dispositivo se borro entre que se decidio la sesion y se anoto,
        // la escritura falla. Eso no puede tumbar la sesion en curso.
        let compartido = estado_compartido();
        let resultado = apply(
            &RuntimeEvent::SessionStarted {
                session_id: SessionId::new(),
                controller: DeviceId::new(),
                permissions: PermissionSet::standard(),
            },
            &compartido,
            &servicio(),
        );

        assert!(resultado.is_err(), "la escritura deberia fallar");
        let estado = compartido
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert!(
            estado.active_session.is_some(),
            "el resumen debe reflejar la sesion aunque el historial falle"
        );
    }

    #[test]
    fn el_resumen_se_serializa_como_lo_espera_el_panel() {
        let json = serde_json::to_value(RuntimeStatus::stopped()).expect("serializa");
        assert_eq!(json["running"], false);
        assert_eq!(json["active_session"], serde_json::Value::Null);
        assert_eq!(json["sessions_served"], 0);
    }
}
