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
use crate::host::{EngineStatus, HostService, pairing_ttl};
use crate::signaling::SignalingProbe;

/// Estado compartido de la aplicacion Tauri.
pub struct AppState {
    /// Servicio del host tras un cerrojo: los comandos llegan desde varios
    /// hilos y el servicio tiene estado mutable (el emparejamiento abierto).
    ///
    /// Va en `Arc` porque la tarea que vigila el bucle del host tambien escribe
    /// en el historial, y no puede tomar prestado el estado de Tauri.
    pub host: std::sync::Arc<Mutex<HostService>>,
    /// Bucle del host, si esta en marcha.
    pub runtime: Mutex<Option<crate::supervisor::RuntimeHandle>>,
}

impl AppState {
    /// Crea el estado con el servicio ya abierto.
    #[must_use]
    pub fn new(host: HostService) -> Self {
        Self {
            host: std::sync::Arc::new(Mutex::new(host)),
            runtime: Mutex::new(None),
        }
    }
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

/// Hasta donde esta verificada una funcion.
///
/// Los tres valores corresponden a los niveles de verificacion del ADR-0007.
/// Un booleano no bastaria: hay codigo escrito y compilado para Windows que
/// nunca se ha ejecutado alli, y llamarlo "listo" seria mentir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureState {
    /// Implementada y probada ejecutandola. Nivel 1.
    Working,
    /// Implementada y compilada para Windows, pero **nunca ejecutada alli**.
    /// Nivel 2: el compilador verifico tipos y firmas, no el comportamiento.
    Unverified,
    /// Todavia no implementada.
    Pending,
}

/// Una funcion que el panel puede usar y hasta donde esta verificada.
#[derive(Debug, Serialize)]
pub struct FeatureStatus {
    /// Clave estable de la funcion.
    pub key: &'static str,
    /// Nombre legible.
    pub label: &'static str,
    /// Hasta donde esta verificada.
    pub state: FeatureState,
    /// Etapa en la que se implementa.
    pub stage: u8,
    /// Que falta exactamente, o que queda por comprobar.
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
/// Esta en el codigo y no en la documentacion a proposito: el panel la usa para
/// no mostrar como terminado lo que no lo esta, y asi no puede quedarse
/// desfasada sin que se note. Cada entrada pendiente dice en que etapa llega y
/// que le falta.
///
/// Esta partida por etapas porque es como se lee y como se revisa.
#[tauri::command]
pub fn feature_matrix() -> Vec<FeatureStatus> {
    let mut todas = Vec::new();
    todas.extend(fundacion());
    todas.extend(motor_de_windows());
    todas.extend(comunicacion());
    todas.extend(funciones_avanzadas());
    todas
}

/// Etapa 1: fundacion, identidad y emparejamiento.
fn fundacion() -> Vec<FeatureStatus> {
    use FeatureState::Working;

    vec![
        FeatureStatus {
            key: "identity",
            label: "Identidad criptografica del host",
            state: Working,
            stage: 1,
            detail: "Par Ed25519 propio, persistente entre arranques.",
        },
        FeatureStatus {
            key: "pairing",
            label: "Emparejamiento por codigo y QR",
            state: Working,
            stage: 1,
            detail: "Codigo temporal derivado con Argon2id, 5 intentos y 5 minutos de validez.",
        },
        FeatureStatus {
            key: "devices",
            label: "Dispositivos autorizados y revocacion",
            state: Working,
            stage: 1,
            detail: "Lista blanca en SQLite con permisos por dispositivo.",
        },
        FeatureStatus {
            key: "system_info",
            label: "Informacion del equipo",
            state: Working,
            stage: 1,
            detail: "Datos reales del sistema. Falta detectar si la sesion esta bloqueada.",
        },
    ]
}

/// Etapa 2: el motor de captura y control de Windows.
fn motor_de_windows() -> Vec<FeatureStatus> {
    use FeatureState::Unverified;

    vec![
        FeatureStatus {
            key: "monitors",
            label: "Deteccion de monitores",
            state: Unverified,
            stage: 2,
            detail: "Implementada con EnumDisplayMonitors, GetDpiForMonitor y                      EnumDisplaySettingsW. Compila para Windows; falta ejecutarla en un PC real.",
        },
        FeatureStatus {
            key: "capture",
            label: "Captura de pantalla",
            state: Unverified,
            stage: 2,
            detail: "Implementada con Windows Graphics Capture y Direct3D 11. Compila para                      Windows; falta comprobar los fotogramas en un PC real.",
        },
        FeatureStatus {
            key: "input",
            label: "Control de raton y teclado",
            state: Unverified,
            stage: 2,
            detail: "Implementada con SendInput, con tabla de teclas y coordenadas absolutas                      probadas aparte. Compila para Windows; falta comprobarla en un PC real.",
        },
    ]
}

/// Etapa 3: senalizacion y conexion entre dispositivos.
fn comunicacion() -> Vec<FeatureStatus> {
    use FeatureState::{Pending, Unverified, Working};

    vec![
        FeatureStatus {
            key: "signaling",
            label: "Canal de senalizacion propio",
            state: Working,
            stage: 3,
            detail: "Servidor y cliente probados contra sockets reales: autenticacion Ed25519,                      limite de intentos, enrutado entre los dos extremos y credenciales TURN                      temporales.",
        },
        FeatureStatus {
            key: "webrtc",
            label: "Conexion cifrada entre dispositivos",
            state: Working,
            stage: 3,
            detail: "ICE, DTLS y canal de datos comprobados de extremo a extremo: una orden                      enviada por un par llega cifrada al otro y se aplica.",
        },
        FeatureStatus {
            key: "sessions",
            label: "Atender sesiones",
            state: Working,
            stage: 3,
            detail: "El host se conecta solo, decide cada peticion contra su registro de \
                     dispositivos, comprueba quien hay al otro lado y sirve el canal de \
                     control. Probado con un controlador real contra el bucle del producto.",
        },
        FeatureStatus {
            key: "session_binding",
            label: "Autenticacion mutua atada al canal cifrado",
            state: Working,
            stage: 3,
            detail: "Cada extremo firma las dos huellas DTLS de la sesion. Si el servidor                      sustituye una para interceptar, la sesion se aborta.",
        },
        FeatureStatus {
            key: "reconnect",
            label: "Reconexion automatica tras un corte",
            state: Working,
            stage: 3,
            detail: "El bucle del host reintenta con espera creciente y vuelve a declararse \
                     disponible solo. Tras un corte, la sesion hay que volver a abrirla desde \
                     el dispositivo.",
        },
        FeatureStatus {
            key: "internet",
            label: "Conexion entre redes distintas con relay",
            state: Unverified,
            stage: 3,
            detail: "El codigo esta escrito y probado en bucle local, donde ICE encuentra camino                      directo. Falta desplegar el servidor con dominio y certificado, y un Coturn                      en marcha, para comprobarlo entre dos redes de verdad.",
        },
        FeatureStatus {
            key: "pairing_relay",
            label: "Emparejar un dispositivo desde fuera de casa",
            state: Pending,
            stage: 3,
            detail: "El servidor lo rechaza a proposito: el codigo tiene unos 40 bits y viajaria                      en claro por el servidor. Falta enviar un verificador Argon2id y confirmar                      la huella en el host. Mientras, se empareja en la red local.",
        },
    ]
}

/// Etapa 5: funciones que faltan por implementar.
fn funciones_avanzadas() -> Vec<FeatureStatus> {
    use FeatureState::Pending;

    vec![
        FeatureStatus {
            key: "video",
            label: "Transmision de la pantalla al dispositivo",
            state: Pending,
            stage: 5,
            detail: "La captura existe y la conexion existe; falta codificar los fotogramas y                      enviarlos por una pista de medios.",
        },
        FeatureStatus {
            key: "audio",
            label: "Audio del sistema",
            state: Pending,
            stage: 5,
            detail: "Formato y control de volumen listos; falta la captura WASAPI en modo bucle.",
        },
        FeatureStatus {
            key: "files",
            label: "Transferencia de archivos",
            state: Pending,
            stage: 5,
            detail: "Restriccion de acceso y contabilidad listas; falta mover bytes por el canal de datos.",
        },
        FeatureStatus {
            key: "clipboard",
            label: "Portapapeles compartido",
            state: Pending,
            stage: 5,
            detail: "Protocolo y permisos definidos; falta leer y escribir el portapapeles de Windows.",
        },
        FeatureStatus {
            key: "power",
            label: "Bloquear, reiniciar y apagar",
            state: Pending,
            stage: 5,
            detail: "Permisos y confirmacion definidos; faltan las llamadas al sistema.",
        },
    ]
}

/// Comprueba si este equipo puede capturar la pantalla e inyectar entrada.
///
/// A diferencia de [`feature_matrix`], que describe el estado del proyecto,
/// esto comprueba **este equipo concreto ahora mismo**: intenta enumerar las
/// pantallas y crear el inyector, y devuelve lo que ocurrio.
#[tauri::command]
pub fn engine_status(state: tauri::State<'_, AppState>) -> CmdResult<EngineStatus> {
    with_host(&state, |host| Ok(host.engine_status()))
}

/// Intenta conectarse al servidor de senalizacion configurado.
///
/// A diferencia de [`feature_matrix`], esto **no describe el proyecto**: se
/// conecta de verdad, se autentica y se declara disponible, y devuelve lo que
/// ocurrio. Es la forma de comprobar un despliegue sin tener todavia un iPad al
/// otro lado.
///
/// El testigo de alta se pasa desde la interfaz y no se guarda en ningun sitio:
/// hace falta una sola vez, en la primera conexion de este host.
///
/// # Errors
///
/// [`HostError::InvalidInput`] si no hay direccion configurada. Un fallo de red
/// o un rechazo del servidor **no** son errores: vienen dentro del resultado
/// con su explicacion, que es justamente lo que se quiere ver.
#[tauri::command]
pub async fn check_signaling(
    state: tauri::State<'_, AppState>,
    enrollment_token: Option<String>,
) -> CmdResult<SignalingProbe> {
    // Todo lo que hace falta se saca con el cerrojo tomado y se suelta antes de
    // tocar la red: retenerlo durante una espera dejaria el panel bloqueado.
    let prepared = with_host(&state, |host| {
        let url = host.signaling_url()?.ok_or_else(|| {
            HostError::InvalidInput(
                "falta configurar la direccion del servidor de senalizacion".to_owned(),
            )
        })?;
        let monitors = host.list_monitors().map_or(1, |layout| {
            u8::try_from(layout.0.len()).unwrap_or(u8::MAX).max(1)
        });
        Ok((
            url,
            host.describe_self(),
            host.capabilities(),
            host.duplicate_identity()?,
            monitors,
        ))
    })?;

    let (url, device, capabilities, identity, monitors) = prepared;
    Ok(crate::signaling::probe(
        &url,
        &device,
        &capabilities,
        &identity,
        enrollment_token.as_deref(),
        monitors,
    )
    .await)
}

/// Arranca el bucle que atiende sesiones.
///
/// A partir de aqui el host esta localizable y un dispositivo emparejado puede
/// abrir sesion. El bucle decide cada peticion contra el registro local, no
/// contra lo que diga el servidor.
///
/// # Errors
///
/// - [`HostError::InvalidInput`] si ya esta en marcha o falta la direccion del
///   servidor.
/// - [`HostError::Security`] si la identidad del host no se puede reconstruir.
#[tauri::command]
pub fn start_host(
    state: tauri::State<'_, AppState>,
    enrollment_token: Option<String>,
) -> CmdResult<crate::supervisor::RuntimeStatus> {
    {
        let marcha = lock(&state.runtime);
        if marcha.as_ref().is_some_and(|h| h.status().running) {
            return Err(crate::supervisor::already_running().into());
        }
    }

    let (config, identity) = with_host(&state, |host| {
        let url = host.signaling_url()?.ok_or_else(|| {
            HostError::InvalidInput(
                "falta configurar la direccion del servidor de senalizacion".to_owned(),
            )
        })?;
        // Los monitores se leen al arrancar: una sesion no puede resolver
        // coordenadas sin saber como esta la pantalla.
        let monitors = host.list_monitors().unwrap_or_else(|error| {
            tracing::warn!(%error, "sin monitores detectados; se usara una pantalla ficticia");
            nexus_core::MonitorLayout(Vec::new())
        });
        Ok((
            crate::runtime::RuntimeConfig {
                url,
                device: host.describe_self(),
                enrollment_token,
                monitors,
            },
            host.duplicate_identity()?,
        ))
    })?;

    let (shutdown, status, handle) = crate::supervisor::prepare();
    let (events, inbox) = tokio::sync::mpsc::channel(64);

    let runtime = crate::runtime::HostRuntime::new(
        config,
        identity,
        std::sync::Arc::new(crate::supervisor::StoreLookup::new(std::sync::Arc::clone(
            &state.host,
        ))),
        std::sync::Arc::new(crate::runtime::PlatformInjectors),
        events,
    );

    let compartido = std::sync::Arc::clone(&state.host);
    let estado = std::sync::Arc::clone(&status);
    tauri::async_runtime::spawn(async move {
        crate::supervisor::consume(inbox, estado, compartido).await;
    });
    tauri::async_runtime::spawn(async move {
        runtime
            .run(async {
                let _ = shutdown.await;
            })
            .await;
    });

    let resumen = handle.status();
    *lock(&state.runtime) = Some(handle);
    Ok(resumen)
}

/// Deja de atender sesiones.
///
/// La sesion en curso, si la hay, se cierra de forma ordenada.
///
/// # Errors
///
/// No falla.
#[tauri::command]
pub fn stop_host(state: tauri::State<'_, AppState>) -> CmdResult<crate::supervisor::RuntimeStatus> {
    let mut marcha = lock(&state.runtime);
    if let Some(handle) = marcha.as_mut() {
        handle.stop();
    }
    *marcha = None;
    Ok(crate::supervisor::RuntimeStatus::stopped())
}

/// Devuelve que esta haciendo el bucle del host.
///
/// # Errors
///
/// No falla.
#[tauri::command]
pub fn runtime_status(
    state: tauri::State<'_, AppState>,
) -> CmdResult<crate::supervisor::RuntimeStatus> {
    Ok(lock(&state.runtime).as_ref().map_or_else(
        crate::supervisor::RuntimeStatus::stopped,
        crate::supervisor::RuntimeHandle::status,
    ))
}

/// Devuelve las sesiones mas recientes del historial.
///
/// # Errors
///
/// [`HostError::Database`] si la lectura falla.
#[tauri::command]
pub fn session_history(
    state: tauri::State<'_, AppState>,
) -> CmdResult<Vec<crate::store::SessionRow>> {
    with_host(&state, |host| host.store().list_sessions(100))
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

/// Toma un cerrojo recuperando el estado si estaba envenenado.
fn lock<T>(cerrojo: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    cerrojo
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    fn la_matriz_de_funciones_es_coherente_con_el_plan_de_etapas() {
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

            match feature.state {
                // "Probado" solo vale para lo que se puede ejecutar y comprobar
                // donde se escribe. El codigo de Windows (etapa 2) y el de
                // Apple (etapa 4) necesitan hardware que no esta aqui, de modo
                // que nunca pueden declararse probados por esta via.
                FeatureState::Working => assert!(
                    feature.stage != 2 && feature.stage != 4,
                    "{} dice estar probado, pero la etapa {} necesita hardware                      que no esta en el entorno de desarrollo",
                    feature.key,
                    feature.stage
                ),
                // "Sin verificar" y "pendiente" tienen que decir que falta.
                FeatureState::Unverified | FeatureState::Pending => assert!(
                    feature.detail.contains("falta") || feature.detail.contains("Falta"),
                    "{} no esta terminado y debe decir que falta: {}",
                    feature.key,
                    feature.detail
                ),
            }
        }
    }

    #[test]
    fn nada_de_la_etapa_2_se_declara_probado() {
        // Es la salvaguarda contra el error mas facil de cometer: marcar como
        // terminado codigo de Windows que nunca se ha ejecutado en Windows.
        for feature in feature_matrix().iter().filter(|f| f.stage == 2) {
            assert_eq!(
                feature.state,
                FeatureState::Unverified,
                "{} no puede declararse probado sin ejecutarlo en Windows",
                feature.key
            );
            assert!(
                feature.detail.contains("falta") || feature.detail.contains("Falta"),
                "{} deberia decir que falta comprobarlo: {}",
                feature.key,
                feature.detail
            );
        }
    }

    #[test]
    fn lo_que_depende_de_un_despliegue_no_se_declara_probado() {
        // La trampa de esta etapa: todo funciona en bucle local y es tentador
        // declarar que funciona desde Internet. No es lo mismo, y la diferencia
        // la nota el usuario cuando lo intenta desde fuera de casa.
        let internet = feature_matrix()
            .into_iter()
            .find(|f| f.key == "internet")
            .expect("debe figurar la conexion entre redes distintas");
        assert_eq!(internet.state, FeatureState::Unverified);
        assert!(
            internet.detail.contains("Falta") || internet.detail.contains("falta"),
            "debe decir que falta: {}",
            internet.detail
        );
    }

    #[test]
    fn lo_que_el_servidor_rechaza_figura_como_pendiente() {
        // Si el relevo de emparejamiento apareciese como terminado, alguien
        // intentaria emparejar desde fuera y no entenderia el rechazo.
        let pairing = feature_matrix()
            .into_iter()
            .find(|f| f.key == "pairing_relay")
            .expect("debe figurar el emparejamiento por red");
        assert_eq!(pairing.state, FeatureState::Pending);
        assert!(
            pairing.detail.contains("red local"),
            "debe decir que alternativa hay: {}",
            pairing.detail
        );
    }

    #[test]
    fn el_host_no_anuncia_capacidades_que_no_tiene() {
        // Anunciar audio o archivos haria que el controlador los ofreciese en
        // su interfaz para despues no funcionar.
        use nexus_core::Capability;
        let host = HostService::open_ephemeral(nexus_security::DeviceIdentity::generate(
            nexus_core::DeviceId::new(),
        ))
        .expect("host en memoria");
        let capabilities = host.capabilities();

        assert!(capabilities.has(Capability::ScreenStream));
        assert!(capabilities.has(Capability::RemoteInput));
        for todavia_no in [
            Capability::AudioStream,
            Capability::FileTransfer,
            Capability::ClipboardText,
            Capability::PowerControl,
        ] {
            assert!(
                !capabilities.has(todavia_no),
                "{todavia_no:?} no esta implementada y no debe anunciarse"
            );
        }
    }

    #[test]
    fn los_estados_se_serializan_como_los_espera_el_panel() {
        assert_eq!(
            serde_json::to_string(&FeatureState::Working).expect("ok"),
            "\"working\""
        );
        assert_eq!(
            serde_json::to_string(&FeatureState::Unverified).expect("ok"),
            "\"unverified\""
        );
        assert_eq!(
            serde_json::to_string(&FeatureState::Pending).expect("ok"),
            "\"pending\""
        );
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
