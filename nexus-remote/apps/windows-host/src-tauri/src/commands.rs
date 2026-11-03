//! Comandos que la interfaz puede invocar.
//!
//! Cada comando es una funcion fina: valida lo que llega de la interfaz,
//! delega en [`HostService`](crate::host::HostService) y traduce el error a
//! algo que el panel pueda mostrar. Las reglas no viven aqui, viven en el
//! servicio, que es donde estan probadas.
//!
//! Los tipos de retorno son estructuras propias y no los tipos internos: asi
//! cambiar una estructura interna no rompe la interfaz en silencio, y se decide
//! explicitamente que se expone. En particular, **nunca se expone la clave
//! privada del host**, solo su huella.
//!
//! ## Contrato de errores
//!
//! Todos los comandos devuelven [`UiError`], con un codigo estable y un
//! mensaje legible. Los codigos posibles estan enumerados en
//! [`crate::error::UiError`]: `database`, `io`, `security`, `capture`,
//! `unknown_device`, `no_pairing_open` e `invalid_input`.

// La firma de un comando de Tauri la impone su macro: el estado se recibe por
// valor y el resultado debe ser serializable. Ninguno de estos dos lints puede
// atenderse sin romper la integracion, y el contrato de errores esta descrito
// arriba en lugar de repetirlo en cada funcion.
#![allow(clippy::needless_pass_by_value, clippy::missing_errors_doc)]

use std::sync::Mutex;

use nexus_core::protocol::control::SystemStatus;
use nexus_core::{DeviceId, MonitorLayout, Permission, PermissionSet, Timestamp};
use serde::{Deserialize, Serialize};

use crate::error::{HostError, UiError};
use crate::host::{HostService, pairing_ttl};

/// Estado compartido de la aplicacion Tauri.
pub struct AppState {
    /// Servicio del host tras un cerrojo: los comandos llegan desde varios
    /// hilos y el servicio tiene estado mutable (el emparejamiento abierto).
    pub host: Mutex<HostService>,
}

/// Resumen del host para el panel.
#[derive(Debug, Serialize)]
pub struct HostSummary {
    /// Identificador del host.
    pub device_id: String,
    /// Huella corta de su clave publica, para comparar a ojo.
    pub fingerprint: String,
    /// Huella completa, para el registro de auditoria.
    pub fingerprint_full: String,
    /// Nombre del equipo.
    pub hostname: String,
    /// Sistema operativo.
    pub os_description: String,
    /// Version de la aplicacion.
    pub app_version: String,
    /// Version del protocolo.
    pub protocol_version: u16,
    /// URL del servidor de senalizacion, si esta configurada.
    pub signaling_url: Option<String>,
    /// Numero de dispositivos activos.
    pub active_devices: usize,
    /// Numero de dispositivos revocados.
    pub revoked_devices: usize,
}

/// Dispositivo tal como lo ve el panel.
#[derive(Debug, Serialize)]
pub struct DeviceSummary {
    /// Identificador.
    pub id: String,
    /// Nombre visible, ya normalizado.
    pub name: String,
    /// Plataforma legible.
    pub platform: String,
    /// Huella corta de su clave publica.
    pub fingerprint: String,
    /// Marca de emparejamiento en milisegundos.
    pub paired_at_ms: u64,
    /// Ultima conexion, si hubo.
    pub last_seen_ms: Option<u64>,
    /// Si esta revocado.
    pub revoked: bool,
    /// Si tiene acceso desatendido.
    pub unattended_access: bool,
    /// Permisos concedidos, por nombre.
    pub permissions: Vec<String>,
}

/// Emparejamiento abierto tal como lo ve el panel.
#[derive(Debug, Serialize)]
pub struct PairingView {
    /// Identificador del emparejamiento.
    pub pairing_id: String,
    /// Codigo agrupado para mostrar en pantalla.
    pub code: String,
    /// Contenido del QR en JSON.
    pub qr_payload: String,
    /// Caducidad en milisegundos.
    pub expires_at_ms: u64,
    /// Segundos de validez totales.
    pub ttl_secs: u64,
    /// Intentos que quedan.
    pub remaining_attempts: u32,
}

/// Perfil de permisos que la persona elige al emparejar.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    /// Solo ver la pantalla.
    ViewOnly,
    /// Control de escritorio sin administracion.
    Standard,
    /// Todo, incluida administracion del equipo.
    Full,
}

impl From<PermissionProfile> for PermissionSet {
    fn from(profile: PermissionProfile) -> Self {
        match profile {
            PermissionProfile::ViewOnly => Self::view_only(),
            PermissionProfile::Standard => Self::standard(),
            PermissionProfile::Full => Self::full(),
        }
    }
}

/// Una funcion que el panel puede usar y en que etapa estara lista.
#[derive(Debug, Serialize)]
pub struct FeatureStatus {
    /// Clave estable de la funcion.
    pub key: &'static str,
    /// Nombre legible.
    pub label: &'static str,
    /// Si ya funciona.
    pub ready: bool,
    /// Etapa en la que se implementa.
    pub stage: u8,
    /// Que falta exactamente.
    pub detail: &'static str,
}

type CmdResult<T> = Result<T, UiError>;

/// Devuelve el resumen del host con datos reales del equipo.
#[tauri::command]
pub fn host_summary(state: tauri::State<'_, AppState>) -> CmdResult<HostSummary> {
    with_host(&state, build_host_summary)
}

/// Construye el resumen del host.
///
/// Esta separado del comando para que el volcado de verificacion
/// (`examples/volcado_ui.rs`) produzca exactamente los mismos datos que ve el
/// panel, sin posibilidad de que diverjan.
///
/// # Errors
///
/// [`HostError::Database`] si la lectura del almacen local falla.
pub fn build_host_summary(host: &mut HostService) -> crate::error::HostResult<HostSummary> {
    let status = crate::system::read_status();
    let devices = host.list_devices()?;
    let public_key = host.public_key();

    Ok(HostSummary {
        device_id: host.device_id().to_string(),
        fingerprint: public_key.short_fingerprint(),
        fingerprint_full: public_key.fingerprint(),
        hostname: status.hostname,
        os_description: status.os_description,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol_version: nexus_core::PROTOCOL_VERSION,
        signaling_url: host.signaling_url()?,
        active_devices: devices.iter().filter(|d| d.is_active()).count(),
        revoked_devices: devices.iter().filter(|d| !d.is_active()).count(),
    })
}

/// Devuelve el estado real del equipo.
#[tauri::command]
pub fn system_status() -> SystemStatus {
    crate::system::read_status()
}

/// Lista los dispositivos autorizados.
#[tauri::command]
pub fn list_devices(state: tauri::State<'_, AppState>) -> CmdResult<Vec<DeviceSummary>> {
    with_host(&state, |host| {
        Ok(host.list_devices()?.into_iter().map(to_summary).collect())
    })
}

/// Abre un emparejamiento y devuelve el codigo y el QR.
#[tauri::command]
pub fn open_pairing(
    state: tauri::State<'_, AppState>,
    profile: PermissionProfile,
    unattended_access: bool,
) -> CmdResult<PairingView> {
    with_host(&state, |host| {
        let now = Timestamp::now();
        let (session, code) = host.open_pairing(profile.into(), unattended_access, now)?;
        let invite = host.current_invite(now)?;

        Ok(PairingView {
            pairing_id: session.id.to_string(),
            code: code.grouped(),
            qr_payload: invite.to_json()?,
            expires_at_ms: session.expires_at.0,
            ttl_secs: pairing_ttl().as_secs(),
            remaining_attempts: session.remaining_attempts(),
        })
    })
}

/// Devuelve el emparejamiento abierto, si sigue vigente.
#[tauri::command]
pub fn current_pairing(state: tauri::State<'_, AppState>) -> CmdResult<Option<PairingView>> {
    with_host(&state, |host| {
        let now = Timestamp::now();
        let Some(code) = host.current_code(now) else {
            return Ok(None);
        };
        let invite = host.current_invite(now)?;
        let session = host.current_pairing(now).ok_or(HostError::NoPairingOpen)?;

        Ok(Some(PairingView {
            pairing_id: session.id.to_string(),
            code: code.grouped(),
            qr_payload: invite.to_json()?,
            expires_at_ms: session.expires_at.0,
            ttl_secs: pairing_ttl().as_secs(),
            remaining_attempts: session.remaining_attempts(),
        }))
    })
}

/// Cancela el emparejamiento abierto.
#[tauri::command]
pub fn cancel_pairing(state: tauri::State<'_, AppState>) -> CmdResult<bool> {
    with_host(&state, |host| Ok(host.cancel_pairing()))
}

/// Revoca un dispositivo.
#[tauri::command]
pub fn revoke_device(state: tauri::State<'_, AppState>, device_id: String) -> CmdResult<()> {
    with_host(&state, |host| {
        host.revoke_device(parse_device_id(&device_id)?, Timestamp::now())
    })
}

/// Borra un dispositivo y su historial.
#[tauri::command]
pub fn delete_device(state: tauri::State<'_, AppState>, device_id: String) -> CmdResult<()> {
    with_host(&state, |host| {
        host.delete_device(parse_device_id(&device_id)?)
    })
}

/// Cambia el perfil de permisos de un dispositivo.
#[tauri::command]
pub fn set_device_profile(
    state: tauri::State<'_, AppState>,
    device_id: String,
    profile: PermissionProfile,
) -> CmdResult<DeviceSummary> {
    with_host(&state, |host| {
        let record = host.set_device_permissions(parse_device_id(&device_id)?, profile.into())?;
        Ok(to_summary(record))
    })
}

/// Guarda la URL del servidor de senalizacion.
#[tauri::command]
pub fn set_signaling_url(state: tauri::State<'_, AppState>, url: String) -> CmdResult<()> {
    with_host(&state, |host| host.set_signaling_url(&url))
}

/// Enumera los monitores del host.
///
/// Mientras no exista el backend de Windows devuelve un error explicito en
/// lugar de una lista vacia, que la interfaz no podria distinguir de "este PC
/// no tiene pantallas".
#[tauri::command]
pub fn list_monitors(state: tauri::State<'_, AppState>) -> CmdResult<MonitorLayout> {
    with_host(&state, |host| host.list_monitors())
}

/// Devuelve que funciones estan listas y que falta para las demas.
///
/// El panel lo usa para no mostrar botones que no hacen nada: cada pantalla
/// pendiente dice en que etapa llega y que le falta.
#[tauri::command]
pub fn feature_matrix() -> Vec<FeatureStatus> {
    vec![
        FeatureStatus {
            key: "identity",
            label: "Identidad criptografica del host",
            ready: true,
            stage: 1,
            detail: "Par Ed25519 propio, persistente entre arranques.",
        },
        FeatureStatus {
            key: "pairing",
            label: "Emparejamiento por codigo y QR",
            ready: true,
            stage: 1,
            detail: "Codigo temporal derivado con Argon2id, 5 intentos y 5 minutos de validez.",
        },
        FeatureStatus {
            key: "devices",
            label: "Dispositivos autorizados y revocacion",
            ready: true,
            stage: 1,
            detail: "Lista blanca en SQLite con permisos por dispositivo.",
        },
        FeatureStatus {
            key: "system_info",
            label: "Informacion del equipo",
            ready: true,
            stage: 1,
            detail: "Datos reales del sistema. Falta detectar si la sesion esta bloqueada.",
        },
        FeatureStatus {
            key: "capture",
            label: "Captura de pantalla",
            ready: false,
            stage: 2,
            detail: "Falta el backend de Windows Graphics Capture y la enumeracion de monitores.",
        },
        FeatureStatus {
            key: "input",
            label: "Control de raton y teclado",
            ready: false,
            stage: 2,
            detail: "La validacion y traduccion de eventos esta lista; falta inyectar con SendInput.",
        },
        FeatureStatus {
            key: "signaling",
            label: "Conexion con el servidor propio",
            ready: false,
            stage: 3,
            detail: "Falta el WebSocket /v1/signaling en el servidor y el cliente en el host.",
        },
        FeatureStatus {
            key: "webrtc",
            label: "Sesion remota por WebRTC",
            ready: false,
            stage: 3,
            detail: "Falta la negociacion ICE y los canales de medios y datos.",
        },
        FeatureStatus {
            key: "audio",
            label: "Audio del sistema",
            ready: false,
            stage: 5,
            detail: "Formato y control de volumen listos; falta la captura WASAPI en modo bucle.",
        },
        FeatureStatus {
            key: "files",
            label: "Transferencia de archivos",
            ready: false,
            stage: 5,
            detail: "Restriccion de acceso y contabilidad listas; falta mover bytes por el canal de datos.",
        },
        FeatureStatus {
            key: "clipboard",
            label: "Portapapeles compartido",
            ready: false,
            stage: 5,
            detail: "Protocolo y permisos definidos; falta leer y escribir el portapapeles de Windows.",
        },
        FeatureStatus {
            key: "power",
            label: "Bloquear, reiniciar y apagar",
            ready: false,
            stage: 5,
            detail: "Permisos y confirmacion definidos; faltan las llamadas al sistema.",
        },
    ]
}

/// Devuelve el catalogo de permisos para la pantalla de seguridad.
#[tauri::command]
pub fn permission_catalog() -> Vec<serde_json::Value> {
    Permission::ALL
        .iter()
        .map(|permission| {
            serde_json::json!({
                "name": permission.name(),
                "sensitive": permission.is_sensitive(),
                "needs_confirmation": permission.requires_host_confirmation(),
            })
        })
        .collect()
}

fn with_host<T, F>(state: &tauri::State<'_, AppState>, action: F) -> CmdResult<T>
where
    F: FnOnce(&mut HostService) -> crate::error::HostResult<T>,
{
    // Un cerrojo envenenado significa que un comando anterior entro en panico.
    // Se recupera el estado en lugar de propagar el panico: el panel debe seguir
    // respondiendo, y el servicio no guarda invariantes que un panico rompa.
    let mut host = match state.host.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::error!("el cerrojo del host estaba envenenado; se recupera el estado");
            poisoned.into_inner()
        }
    };
    action(&mut host).map_err(UiError::from)
}

fn parse_device_id(raw: &str) -> crate::error::HostResult<DeviceId> {
    raw.parse::<DeviceId>().map_err(|_| {
        HostError::InvalidInput(format!("identificador de dispositivo invalido: {raw}"))
    })
}

/// Traduce un dispositivo del almacen a lo que ve el panel.
///
/// Expone la huella de la clave publica y nunca la clave privada, que el host
/// no entrega a nadie.
#[must_use]
pub fn to_summary(record: nexus_security::registry::DeviceRecord) -> DeviceSummary {
    DeviceSummary {
        id: record.info.id.to_string(),
        name: record.info.sanitized_name(),
        platform: record.info.platform.label().to_owned(),
        fingerprint: record.public_key.short_fingerprint(),
        paired_at_ms: record.paired_at.0,
        last_seen_ms: record.last_seen.map(|t| t.0),
        revoked: !record.is_active(),
        unattended_access: record.unattended_access,
        permissions: record
            .permissions
            .iter()
            .map(|p| p.name().to_owned())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_profiles_map_to_the_expected_sets() {
        assert_eq!(
            PermissionSet::from(PermissionProfile::ViewOnly),
            PermissionSet::view_only()
        );
        assert_eq!(
            PermissionSet::from(PermissionProfile::Standard),
            PermissionSet::standard()
        );
        assert_eq!(
            PermissionSet::from(PermissionProfile::Full),
            PermissionSet::full()
        );
    }

    #[test]
    fn profiles_deserialize_from_what_the_ui_sends() {
        for (raw, expected) in [
            ("\"view_only\"", PermissionSet::view_only()),
            ("\"standard\"", PermissionSet::standard()),
            ("\"full\"", PermissionSet::full()),
        ] {
            let profile: PermissionProfile = serde_json::from_str(raw).expect(raw);
            assert_eq!(PermissionSet::from(profile), expected);
        }
        assert!(serde_json::from_str::<PermissionProfile>("\"root\"").is_err());
    }

    #[test]
    fn the_feature_matrix_is_consistent_with_the_stage_plan() {
        let matrix = feature_matrix();
        assert!(!matrix.is_empty());

        for feature in &matrix {
            assert!(
                !feature.detail.is_empty(),
                "{} sin explicacion",
                feature.key
            );
            assert!(
                (1..=6).contains(&feature.stage),
                "{} en etapa {}",
                feature.key,
                feature.stage
            );
            // Lo que ya funciona pertenece a la etapa 1; si algo de una etapa
            // posterior se marcara listo, esta prueba lo detecta.
            if feature.ready {
                assert_eq!(
                    feature.stage, 1,
                    "{} dice estar listo en etapa {}",
                    feature.key, feature.stage
                );
            }
        }

        let ready: Vec<_> = matrix.iter().filter(|f| f.ready).map(|f| f.key).collect();
        assert_eq!(ready, vec!["identity", "pairing", "devices", "system_info"]);
    }

    #[test]
    fn the_permission_catalog_covers_every_permission() {
        let catalog = permission_catalog();
        assert_eq!(catalog.len(), Permission::ALL.len());

        let shutdown = catalog
            .iter()
            .find(|entry| entry["name"] == "shutdown")
            .expect("apagar debe figurar");
        assert_eq!(shutdown["sensitive"], true);
        assert_eq!(shutdown["needs_confirmation"], true);

        let view = catalog
            .iter()
            .find(|entry| entry["name"] == "view_screen")
            .expect("ver pantalla debe figurar");
        assert_eq!(view["needs_confirmation"], false);
    }

    #[test]
    fn device_ids_are_validated_before_use() {
        assert!(parse_device_id("no-es-un-uuid").is_err());
        assert!(parse_device_id(&DeviceId::new().to_string()).is_ok());
    }

    #[test]
    fn the_summary_exposes_the_fingerprint_and_never_the_private_key() {
        use nexus_core::{DeviceInfo, DevicePlatform, DeviceRole};
        use nexus_security::DeviceIdentity;

        let identity = DeviceIdentity::generate(DeviceId::new());
        let secret_hex = hex_of(identity.export_secret().as_slice());

        let record = nexus_security::registry::DeviceRecord::new(
            DeviceInfo {
                id: identity.device_id(),
                display_name: "iPad".to_owned(),
                platform: DevicePlatform::IpadOs,
                role: DeviceRole::Controller,
                os_version: "18.1".to_owned(),
                app_version: "0.1.0".to_owned(),
            },
            identity.public_key(),
            PermissionSet::standard(),
            false,
            Timestamp(1),
        );

        let summary = to_summary(record);
        let json = serde_json::to_string(&summary).expect("serializa");
        assert!(
            !json.contains(&secret_hex),
            "la clave privada no debe salir a la interfaz"
        );
        assert!(json.contains(&identity.public_key().short_fingerprint()));
        assert_eq!(summary.platform, "iPad");
        assert!(summary.permissions.contains(&"control_pointer".to_owned()));
    }

    fn hex_of(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
    }
}
