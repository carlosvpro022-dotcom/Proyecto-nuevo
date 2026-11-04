//! Servicio del host: lo que la interfaz puede pedir y el host sabe hacer.
//!
//! Reune la identidad, el almacen local y el flujo de emparejamiento en un solo
//! objeto con estado. La interfaz no habla nunca directamente con `SQLite` ni con
//! la capa de criptografia: pasa por aqui, y aqui se aplican las reglas.
//!
//! # Lo que ya funciona
//!
//! - Identidad del host persistente entre arranques.
//! - Emparejamiento completo: abrir, mostrar codigo y QR, canjear, caducar,
//!   cancelar, con limite de intentos.
//! - Lista de dispositivos autorizados, con revocacion y borrado.
//! - Autenticacion de un dispositivo ya emparejado contra el almacen `SQLite`.
//! - Informacion real del equipo.
//!
//! # Lo que falta
//!
//! - Enumeracion de monitores y captura: etapa 2 (devuelve error explicito en
//!   las plataformas sin backend).
//! - Conexion con el servidor de senalizacion y sesiones remotas: etapa 3.

use std::path::Path;
use std::time::Duration;

use nexus_core::{
    Capability, CapabilitySet, DeviceId, DeviceInfo, DevicePlatform, DeviceRole, PermissionSet,
    Timestamp,
};
use nexus_security::identity::DevicePublicKey;
use nexus_security::pairing::{DEFAULT_PAIRING_TTL, PairingInvite};
use nexus_security::registry::DeviceRecord;
use nexus_security::{DeviceIdentity, PairingCode, PairingSession};

use crate::error::{HostError, HostResult};
use crate::store::Store;

/// Clave del ajuste que guarda la URL del servidor de senalizacion.
pub const SETTING_SIGNALING_URL: &str = "signaling_url";

/// Puerto UDP fijo para la conexion cifrada, o `0` para que lo elija el sistema.
pub const SETTING_UDP_PORT: &str = "udp_port";

/// Direccion publica que anunciar: una IP o el nombre de un DNS dinamico.
pub const SETTING_PUBLIC_ENDPOINT: &str = "public_endpoint";

/// Carpetas que el host comparte, guardadas como JSON.
pub const SETTING_SHARED_ROOTS: &str = "shared_roots";

/// Carpetas compartidas que se admiten como maximo.
///
/// No es un limite tecnico: es para que la lista siga siendo algo que una
/// persona pueda leer de un vistazo y saber que esta exponiendo.
pub const MAX_SHARED_ROOTS: usize = 8;

/// Emparejamiento abierto, con el codigo en claro para poder mostrarlo.
///
/// El codigo vive aqui, en memoria, solo mientras el emparejamiento esta
/// abierto. Al cerrarse se descarta.
#[derive(Debug)]
struct OpenPairing {
    session: PairingSession,
    code: PairingCode,
}

/// Resultado de comprobar una capacidad del motor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EngineCheck {
    /// Si la capacidad funciona en este equipo.
    pub working: bool,
    /// Explicacion legible: que se detecto, o por que no funciona.
    pub detail: String,
}

impl EngineCheck {
    fn working(detail: impl Into<String>) -> Self {
        Self {
            working: true,
            detail: detail.into(),
        }
    }

    fn failed(detail: impl Into<String>) -> Self {
        Self {
            working: false,
            detail: detail.into(),
        }
    }
}

/// Estado del motor de escritorio remoto en este equipo.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EngineStatus {
    /// Captura de pantalla.
    pub capture: EngineCheck,
    /// Inyeccion de raton y teclado.
    pub input: EngineCheck,
}

impl EngineStatus {
    /// Indica si el equipo puede sostener una sesion remota completa.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.capture.working && self.input.working
    }
}

/// Servicio del host.
pub struct HostService {
    identity: DeviceIdentity,
    store: Store,
    pairing: Option<OpenPairing>,
}

impl HostService {
    /// Arranca el servicio sobre un directorio de datos.
    ///
    /// # Errors
    ///
    /// [`HostError::Io`] si el directorio no es utilizable,
    /// [`HostError::Database`] si la base de datos no se puede abrir.
    pub fn open(data_dir: &Path) -> HostResult<Self> {
        let identity = crate::identity::load_or_create(data_dir)?;
        let store = Store::open(&data_dir.join("nexus-host.db"))?;
        Ok(Self {
            identity,
            store,
            pairing: None,
        })
    }

    /// Arranca el servicio con todo en memoria. Para pruebas.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la base en memoria no se puede crear.
    pub fn open_ephemeral(identity: DeviceIdentity) -> HostResult<Self> {
        Ok(Self {
            identity,
            store: Store::open_in_memory()?,
            pairing: None,
        })
    }

    /// Identificador del host.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.identity.device_id()
    }

    /// Clave publica del host.
    #[must_use]
    pub fn public_key(&self) -> DevicePublicKey {
        self.identity.public_key()
    }

    /// Acceso de lectura al almacen.
    #[must_use]
    pub const fn store(&self) -> &Store {
        &self.store
    }

    /// Datos que el host declara sobre si mismo.
    #[must_use]
    pub fn describe_self(&self) -> DeviceInfo {
        let status = crate::system::read_status();
        DeviceInfo {
            id: self.identity.device_id(),
            display_name: status.hostname,
            platform: host_platform(),
            role: DeviceRole::Host,
            os_version: status.os_description,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    /// Abre un emparejamiento y devuelve el codigo a mostrar.
    ///
    /// Si ya habia uno abierto, se cancela: no puede haber dos codigos validos
    /// a la vez, porque entonces no quedaria claro cual se esta canjeando.
    ///
    /// # Errors
    ///
    /// [`HostError::Security`] si la derivacion del codigo falla.
    pub fn open_pairing(
        &mut self,
        permissions: PermissionSet,
        unattended_access: bool,
        now: Timestamp,
    ) -> HostResult<(PairingSession, PairingCode)> {
        self.cancel_pairing();

        let (session, code) = PairingSession::open(
            self.identity.device_id(),
            permissions,
            unattended_access,
            DEFAULT_PAIRING_TTL,
            now,
        )?;

        self.pairing = Some(OpenPairing {
            session: session.clone(),
            code: code.clone(),
        });
        tracing::info!(pairing_id = %session.id, "emparejamiento abierto");
        Ok((session, code))
    }

    /// Emparejamiento abierto, si lo hay y sigue vigente.
    pub fn current_pairing(&mut self, now: Timestamp) -> Option<&PairingSession> {
        if let Some(open) = self.pairing.as_mut()
            && open.session.expire_if_due(now)
        {
            tracing::info!(pairing_id = %open.session.id, "el codigo de emparejamiento caduco");
        }
        self.pairing.as_ref().map(|open| &open.session)
    }

    /// Codigo del emparejamiento abierto, para mostrarlo en pantalla.
    pub fn current_code(&mut self, now: Timestamp) -> Option<PairingCode> {
        let valid = self
            .current_pairing(now)
            .is_some_and(|s| s.is_usable_at(now));
        if !valid {
            return None;
        }
        self.pairing.as_ref().map(|open| open.code.clone())
    }

    /// Invitacion lista para pintarla en un QR.
    ///
    /// # Errors
    ///
    /// [`HostError::NoPairingOpen`] si no hay emparejamiento vigente.
    pub fn current_invite(&mut self, now: Timestamp) -> HostResult<PairingInvite> {
        let signaling_url = self
            .store
            .get_setting(SETTING_SIGNALING_URL)?
            .unwrap_or_else(|| "wss://sin-configurar/signaling".to_owned());

        let code = self.current_code(now).ok_or(HostError::NoPairingOpen)?;
        let session = self.pairing.as_ref().ok_or(HostError::NoPairingOpen)?;
        let host_key = self.identity.public_key();

        Ok(PairingInvite::new(
            signaling_url,
            &session.session,
            &code,
            &host_key,
        ))
    }

    /// Cancela el emparejamiento abierto. Devuelve `true` si habia uno.
    pub fn cancel_pairing(&mut self) -> bool {
        match self.pairing.take() {
            Some(mut open) => {
                open.session.cancel();
                tracing::info!(pairing_id = %open.session.id, "emparejamiento cancelado");
                true
            }
            None => false,
        }
    }

    /// Canjea el codigo y registra el dispositivo nuevo.
    ///
    /// Es la operacion que convierte un dispositivo desconocido en autorizado.
    /// El codigo se comprueba con Argon2id y se consume: aunque sea correcto, no
    /// sirve dos veces.
    ///
    /// # Errors
    ///
    /// - [`HostError::NoPairingOpen`] si no hay emparejamiento abierto.
    /// - [`HostError::Security`] si el codigo no coincide, caduco, o se agotaron
    ///   los intentos.
    /// - [`HostError::InvalidInput`] si la clave publica recibida no es valida.
    pub fn redeem_pairing(
        &mut self,
        typed_code: &str,
        info: DeviceInfo,
        public_key_hex: &str,
        now: Timestamp,
    ) -> HostResult<DeviceRecord> {
        let public_key = DevicePublicKey::from_hex(public_key_hex)?;

        // No se pre-caduca el emparejamiento: `PairingSession::redeem` ya
        // comprueba la ventana antes de gastar un intento y devuelve
        // `PairingCodeExpired`, que es mas util que un `PairingNotActive`.
        let open = self.pairing.as_mut().ok_or(HostError::NoPairingOpen)?;

        let candidate = PairingCode::parse(typed_code)?;
        open.session.redeem(&candidate, now)?;

        let record = DeviceRecord::new(
            info,
            public_key,
            open.session.permissions.clone(),
            open.session.unattended_access,
            now,
        );
        let device_id = record.info.id;
        self.store.save_device(device_id, &record)?;
        self.pairing = None;

        tracing::info!(
            %device_id,
            fingerprint = %record.public_key.short_fingerprint(),
            "dispositivo emparejado"
        );
        Ok(record)
    }

    /// Lista los dispositivos autorizados.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la lectura falla.
    pub fn list_devices(&self) -> HostResult<Vec<DeviceRecord>> {
        Ok(self
            .store
            .list_devices()?
            .into_iter()
            .map(|(_, record)| record)
            .collect())
    }

    /// Revoca un dispositivo.
    ///
    /// # Errors
    ///
    /// [`HostError::UnknownDevice`] si no existe.
    pub fn revoke_device(&mut self, device_id: DeviceId, now: Timestamp) -> HostResult<()> {
        if self.store.load_device(device_id)?.is_none() {
            return Err(HostError::UnknownDevice {
                id: device_id.to_string(),
            });
        }
        if self.store.revoke_device(device_id, now)? {
            tracing::warn!(%device_id, "dispositivo revocado");
        }
        Ok(())
    }

    /// Borra un dispositivo y su historial.
    ///
    /// # Errors
    ///
    /// [`HostError::UnknownDevice`] si no existe.
    pub fn delete_device(&mut self, device_id: DeviceId) -> HostResult<()> {
        if self.store.delete_device(device_id)? {
            tracing::warn!(%device_id, "dispositivo borrado");
            Ok(())
        } else {
            Err(HostError::UnknownDevice {
                id: device_id.to_string(),
            })
        }
    }

    /// Cambia los permisos de un dispositivo.
    ///
    /// # Errors
    ///
    /// [`HostError::UnknownDevice`] si no existe.
    pub fn set_device_permissions(
        &mut self,
        device_id: DeviceId,
        permissions: PermissionSet,
    ) -> HostResult<DeviceRecord> {
        let mut record =
            self.store
                .load_device(device_id)?
                .ok_or_else(|| HostError::UnknownDevice {
                    id: device_id.to_string(),
                })?;
        record.permissions = permissions;
        self.store.save_device(device_id, &record)?;
        Ok(record)
    }

    /// Guarda la URL del servidor de senalizacion.
    ///
    /// # Errors
    ///
    /// [`HostError::InvalidInput`] si la URL no usa `wss://`, y
    /// [`HostError::Database`] si la escritura falla.
    pub fn set_signaling_url(&self, url: &str) -> HostResult<()> {
        let trimmed = url.trim();
        // Se exige `wss://`: senalizacion en claro expondria los
        // identificadores de los dispositivos y los candidatos ICE.
        if !trimmed.starts_with("wss://") {
            return Err(HostError::InvalidInput(
                "la URL de senalizacion debe empezar por wss:// (TLS obligatorio)".to_owned(),
            ));
        }
        self.store.set_setting(SETTING_SIGNALING_URL, trimmed)
    }

    /// Guarda como se expone este equipo a la red.
    ///
    /// Hace falta cuando el servidor esta en este mismo PC y se alcanza desde
    /// fuera abriendo un puerto en el router: ahi el puerto no puede ser al
    /// azar y la direccion que el equipo ve de si mismo es privada.
    ///
    /// `public_endpoint` admite una IP o un nombre de dominio. Lo segundo es
    /// lo normal en una casa, donde la IP cambia y el nombre de un DNS
    /// dinamico no.
    ///
    /// # Errors
    ///
    /// - [`HostError::InvalidInput`] si el puerto esta reservado al sistema.
    /// - [`HostError::Database`] si la escritura falla.
    pub fn set_network_exposure(
        &self,
        udp_port: u16,
        public_endpoint: Option<&str>,
    ) -> HostResult<()> {
        // Por debajo de 1024 hacen falta permisos de administrador en la
        // mayoria de sistemas, y no hay motivo para pedirlos.
        if udp_port != 0 && udp_port < 1024 {
            return Err(HostError::InvalidInput(
                "el puerto debe ser 0 (automatico) o estar entre 1024 y 65535".to_owned(),
            ));
        }

        self.store
            .set_setting(SETTING_UDP_PORT, &udp_port.to_string())?;

        let endpoint = public_endpoint.map(str::trim).unwrap_or_default();
        self.store.set_setting(SETTING_PUBLIC_ENDPOINT, endpoint)
    }

    /// Como se expone este equipo a la red.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la lectura falla.
    pub fn network_exposure(&self) -> HostResult<(u16, Option<String>)> {
        let puerto = self
            .store
            .get_setting(SETTING_UDP_PORT)?
            .and_then(|valor| valor.parse::<u16>().ok())
            .unwrap_or(0);

        let endpoint = self
            .store
            .get_setting(SETTING_PUBLIC_ENDPOINT)?
            .filter(|valor| !valor.trim().is_empty());

        Ok((puerto, endpoint))
    }

    /// Declara las carpetas que este equipo comparte.
    ///
    /// Reemplaza la lista entera. Guardar una lista vacia deja el equipo sin
    /// compartir nada, que es el estado por omision.
    ///
    /// Se comprueba cada carpeta antes de guardarla, porque un error aqui se
    /// paga exponiendo algo que no se queria:
    ///
    /// - La ruta tiene que ser absoluta. Una relativa dependeria de desde donde
    ///   se arranco el programa.
    /// - Tiene que existir y ser una carpeta.
    /// - No puede ser la raiz de un disco. Compartir `C:\` entero anula el
    ///   sentido de tener una lista, y nadie lo elige a proposito.
    /// - La etiqueta no puede estar vacia: es lo que la persona vera y
    ///   reconocera en el explorador remoto.
    ///
    /// # Errors
    ///
    /// - [`HostError::InvalidInput`] si alguna carpeta no cumple lo anterior o
    ///   si hay mas de [`MAX_SHARED_ROOTS`].
    /// - [`HostError::Database`] si no se puede guardar.
    pub fn set_shared_roots(&self, roots: &[nexus_file_transfer::SharedRoot]) -> HostResult<()> {
        if roots.len() > MAX_SHARED_ROOTS {
            return Err(HostError::InvalidInput(format!(
                "no se pueden compartir mas de {MAX_SHARED_ROOTS} carpetas"
            )));
        }

        let mut limpias = Vec::with_capacity(roots.len());
        for root in roots {
            let etiqueta = nexus_core::text::sanitize_line(&root.label, 64, "");
            if etiqueta.is_empty() {
                return Err(HostError::InvalidInput(
                    "cada carpeta compartida necesita un nombre".to_owned(),
                ));
            }
            if !root.path.is_absolute() {
                return Err(HostError::InvalidInput(format!(
                    "la ruta de «{etiqueta}» tiene que ser absoluta"
                )));
            }
            let metadata = std::fs::metadata(&root.path).map_err(|error| {
                HostError::InvalidInput(format!(
                    "la carpeta de «{etiqueta}» no se puede leer: {error}"
                ))
            })?;
            if !metadata.is_dir() {
                return Err(HostError::InvalidInput(format!(
                    "«{etiqueta}» no es una carpeta"
                )));
            }
            if root.path.parent().is_none() {
                return Err(HostError::InvalidInput(
                    "no se puede compartir la raiz de un disco: elige una carpeta concreta"
                        .to_owned(),
                ));
            }

            limpias.push(nexus_file_transfer::SharedRoot {
                label: etiqueta,
                path: root.path.clone(),
                writable: root.writable,
            });
        }

        let json = serde_json::to_string(&limpias)
            .map_err(|error| HostError::InvalidInput(error.to_string()))?;
        self.store.set_setting(SETTING_SHARED_ROOTS, &json)
    }

    /// Carpetas que este equipo comparte.
    ///
    /// Devuelve una lista vacia cuando no hay nada configurado, y tambien
    /// cuando lo guardado no se puede interpretar: ante la duda, no se comparte
    /// nada. Un ajuste corrupto no debe poder abrir el disco.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la lectura falla.
    pub fn shared_roots(&self) -> HostResult<Vec<nexus_file_transfer::SharedRoot>> {
        let Some(json) = self.store.get_setting(SETTING_SHARED_ROOTS)? else {
            return Ok(Vec::new());
        };
        match serde_json::from_str(&json) {
            Ok(roots) => Ok(roots),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "el ajuste de carpetas compartidas no se pudo interpretar; no se comparte nada"
                );
                Ok(Vec::new())
            }
        }
    }

    /// URL del servidor de senalizacion, si esta configurada.
    ///
    /// # Errors
    ///
    /// [`HostError::Database`] si la lectura falla.
    pub fn signaling_url(&self) -> HostResult<Option<String>> {
        self.store.get_setting(SETTING_SIGNALING_URL)
    }

    /// Capacidades que este host puede ofrecer de verdad hoy.
    ///
    /// No se anuncia lo que no esta implementado: si el host dijese que sabe
    /// transmitir audio, el controlador lo ofreceria en su interfaz y luego no
    /// funcionaria. Se van anadiendo a medida que se implementan.
    #[must_use]
    pub fn capabilities(&self) -> CapabilitySet {
        let mut set = CapabilitySet::empty();
        set.insert(Capability::ScreenStream);
        set.insert(Capability::MultiMonitor);
        set.insert(Capability::RemoteInput);
        set.insert(Capability::TouchGestures);
        set
    }

    /// Copia de la identidad criptografica del host.
    ///
    /// Existe porque una tarea de red no puede retener el cerrojo del servicio
    /// mientras espera: el panel se quedaria bloqueado. La clave privada no
    /// sale del proceso; es la misma que ya estaba en memoria.
    ///
    /// # Errors
    ///
    /// [`HostError::Security`] si la clave guardada no se puede reconstruir.
    pub fn duplicate_identity(&self) -> HostResult<DeviceIdentity> {
        Ok(DeviceIdentity::from_secret_bytes(
            self.identity.device_id(),
            self.identity.export_secret().as_slice(),
        )?)
    }

    /// Enumera los monitores del host.
    ///
    /// En Windows consulta el sistema de verdad. En cualquier otra plataforma
    /// devuelve [`CaptureError::UnsupportedPlatform`](nexus_capture::CaptureError::UnsupportedPlatform),
    /// que la interfaz muestra como tal en lugar de una lista vacia.
    ///
    /// # Errors
    ///
    /// [`HostError::Capture`] si la plataforma no tiene backend o si la
    /// enumeracion falla.
    pub fn list_monitors(&self) -> HostResult<nexus_core::MonitorLayout> {
        Ok(nexus_capture::enumerate_monitors()?)
    }

    /// Comprueba si este host puede capturar la pantalla e inyectar entrada.
    ///
    /// Devuelve lo que ocurre **de verdad al intentarlo**, no una suposicion a
    /// partir del sistema operativo: en Windows, un equipo demasiado antiguo o
    /// sin Direct3D fallaria aqui, y conviene que la persona lo vea en el
    /// panel antes de intentar una sesion.
    #[must_use]
    pub fn engine_status(&self) -> EngineStatus {
        let capture = match nexus_capture::enumerate_monitors() {
            Ok(layout) if layout.is_empty() => {
                EngineCheck::failed("el sistema no reporto ninguna pantalla")
            }
            Ok(layout) => {
                EngineCheck::working(format!("{} pantalla(s) detectada(s)", layout.len()))
            }
            Err(error) => EngineCheck::failed(error.to_string()),
        };

        let input = match nexus_input::default_injector() {
            Ok(injector) => {
                let name = injector.name();
                if name == "unsupported" {
                    EngineCheck::failed(format!(
                        "sin backend de inyeccion en {}",
                        std::env::consts::OS
                    ))
                } else {
                    EngineCheck::working(format!("backend {name} listo"))
                }
            }
            Err(error) => EngineCheck::failed(error.to_string()),
        };

        EngineStatus { capture, input }
    }
}

/// Plataforma en la que corre este binario.
const fn host_platform() -> DevicePlatform {
    if cfg!(windows) {
        DevicePlatform::Windows
    } else if cfg!(target_os = "linux") {
        DevicePlatform::Linux
    } else {
        DevicePlatform::Unknown
    }
}

/// Validez del codigo de emparejamiento, expuesta para la interfaz.
#[must_use]
pub const fn pairing_ttl() -> Duration {
    DEFAULT_PAIRING_TTL
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_security::SecurityError;
    use nexus_security::pairing::PairingState;

    const NOW: Timestamp = Timestamp(1_730_000_000_000);

    fn service() -> HostService {
        HostService::open_ephemeral(DeviceIdentity::generate(DeviceId::new()))
            .expect("servicio en memoria")
    }

    #[test]
    fn por_omision_el_equipo_no_declara_nada_especial() {
        let host = service();
        assert_eq!(host.network_exposure().expect("lee"), (0, None));
    }

    #[test]
    fn se_puede_fijar_el_puerto_y_el_nombre_publico() {
        // Es la configuracion de quien aloja el servidor en su propio PC.
        let host = service();
        host.set_network_exposure(51_820, Some("casa.midominio.com"))
            .expect("guarda");
        assert_eq!(
            host.network_exposure().expect("lee"),
            (51_820, Some("casa.midominio.com".to_owned()))
        );
    }

    #[test]
    fn un_puerto_reservado_al_sistema_se_rechaza() {
        let host = service();
        let error = host
            .set_network_exposure(443, None)
            .expect_err("443 necesita permisos de administrador");
        assert!(matches!(error, HostError::InvalidInput(_)));
    }

    #[test]
    fn el_puerto_automatico_sigue_valiendo() {
        let host = service();
        host.set_network_exposure(0, None).expect("vale");
        assert_eq!(host.network_exposure().expect("lee").0, 0);
    }

    #[test]
    fn un_nombre_publico_en_blanco_cuenta_como_ninguno() {
        let host = service();
        host.set_network_exposure(1_024, Some("   "))
            .expect("guarda");
        assert_eq!(host.network_exposure().expect("lee").1, None);
    }

    fn controller() -> (DeviceIdentity, DeviceInfo) {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let info = DeviceInfo {
            id: identity.device_id(),
            display_name: "iPad Pro".to_owned(),
            platform: DevicePlatform::IpadOs,
            role: DeviceRole::Controller,
            os_version: "18.1".to_owned(),
            app_version: "0.1.0".to_owned(),
        };
        (identity, info)
    }

    #[test]
    fn a_fresh_host_has_no_devices_and_no_pairing() {
        let mut host = service();
        assert!(host.list_devices().expect("lista").is_empty());
        assert!(host.current_pairing(NOW).is_none());
        assert!(host.current_code(NOW).is_none());
    }

    #[test]
    fn the_host_describes_itself_with_real_data() {
        let host = service();
        let info = host.describe_self();
        assert_eq!(info.role, DeviceRole::Host);
        assert_eq!(info.id, host.device_id());
        assert!(!info.display_name.is_empty());
        assert!(!info.os_version.is_empty());
    }

    #[test]
    fn an_expired_pairing_reports_expiry_not_a_generic_refusal() {
        // El mensaje importa: "el codigo caduco" le dice a la persona que
        // genere otro; "no esta activo" no le dice nada.
        let mut host = service();
        let (_, code) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        let (identity, info) = controller();
        let too_late = Timestamp(NOW.0 + DEFAULT_PAIRING_TTL.as_millis() as u64);

        let err = host
            .redeem_pairing(
                code.as_str(),
                info,
                &identity.public_key().to_hex(),
                too_late,
            )
            .unwrap_err();
        assert!(
            matches!(err, HostError::Security(SecurityError::PairingCodeExpired)),
            "{err:?}"
        );
    }

    #[test]
    fn opening_a_pairing_produces_a_showable_code() {
        let mut host = service();
        let (session, code) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");

        assert_eq!(session.state, PairingState::Active);
        assert_eq!(code.as_str().len(), 8);
        assert_eq!(code.grouped().len(), 9, "formato XXXX-XXXX");
        assert_eq!(
            host.current_code(NOW).map(|c| c.as_str().to_owned()),
            Some(code.as_str().to_owned())
        );
    }

    #[test]
    fn opening_a_second_pairing_replaces_the_first() {
        let mut host = service();
        let (_, first) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        let (_, second) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        assert_ne!(first.as_str(), second.as_str());

        // El primer codigo ya no sirve: solo hay un emparejamiento vigente.
        let (_, info) = controller();
        let identity = DeviceIdentity::generate(info.id);
        let err = host
            .redeem_pairing(first.as_str(), info, &identity.public_key().to_hex(), NOW)
            .unwrap_err();
        assert!(
            matches!(err, HostError::Security(SecurityError::PairingCodeMismatch)),
            "{err:?}"
        );
    }

    #[test]
    fn the_right_code_registers_the_device() {
        let mut host = service();
        let (_, code) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        let (identity, info) = controller();

        let record = host
            .redeem_pairing(code.as_str(), info, &identity.public_key().to_hex(), NOW)
            .expect("empareja");

        assert_eq!(record.public_key, identity.public_key());
        assert_eq!(record.permissions, PermissionSet::standard());
        assert_eq!(host.list_devices().expect("lista").len(), 1);
        assert!(
            host.current_pairing(NOW).is_none(),
            "el emparejamiento se consume"
        );
    }

    #[test]
    fn a_grouped_code_typed_by_hand_also_works() {
        let mut host = service();
        let (_, code) = host
            .open_pairing(PermissionSet::view_only(), false, NOW)
            .expect("abre");
        let (identity, info) = controller();

        // Tal como lo teclearia la persona en el iPad.
        let typed = code.grouped().to_lowercase();
        host.redeem_pairing(&typed, info, &identity.public_key().to_hex(), NOW)
            .expect("debe aceptar el formato con guion y en minusculas");
    }

    #[test]
    fn the_granted_permissions_are_the_ones_chosen_when_opening() {
        let mut host = service();
        let (_, code) = host
            .open_pairing(PermissionSet::view_only(), false, NOW)
            .expect("abre");
        let (identity, info) = controller();

        let record = host
            .redeem_pairing(code.as_str(), info, &identity.public_key().to_hex(), NOW)
            .expect("empareja");
        assert_eq!(record.permissions, PermissionSet::view_only());
        assert!(!record.unattended_access);
    }

    #[test]
    fn a_wrong_code_does_not_register_anything() {
        let mut host = service();
        host.open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        let (identity, info) = controller();

        let err = host
            .redeem_pairing("ZZZZ9999", info, &identity.public_key().to_hex(), NOW)
            .unwrap_err();
        assert!(matches!(
            err,
            HostError::Security(SecurityError::PairingCodeMismatch)
        ));
        assert!(host.list_devices().expect("lista").is_empty());
    }

    #[test]
    fn redeeming_without_an_open_pairing_fails() {
        let mut host = service();
        let (identity, info) = controller();
        let err = host
            .redeem_pairing("ABCD2345", info, &identity.public_key().to_hex(), NOW)
            .unwrap_err();
        assert!(matches!(err, HostError::NoPairingOpen), "{err:?}");
    }

    #[test]
    fn an_expired_code_is_refused_and_registers_nothing() {
        let mut host = service();
        let (_, code) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        let (identity, info) = controller();

        let too_late = Timestamp(NOW.0 + DEFAULT_PAIRING_TTL.as_millis() as u64 + 1);
        let err = host
            .redeem_pairing(
                code.as_str(),
                info,
                &identity.public_key().to_hex(),
                too_late,
            )
            .unwrap_err();
        assert!(
            matches!(err, HostError::Security(SecurityError::PairingCodeExpired)),
            "{err:?}"
        );
        assert!(host.list_devices().expect("lista").is_empty());
        assert!(host.current_code(too_late).is_none());
    }

    #[test]
    fn an_invalid_public_key_is_refused_before_spending_an_attempt() {
        let mut host = service();
        let (_, code) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        let (_, info) = controller();

        assert!(
            host.redeem_pairing(code.as_str(), info.clone(), "no-es-hex", NOW)
                .is_err()
        );
        assert!(
            host.redeem_pairing(code.as_str(), info, &"00".repeat(32), NOW)
                .is_err(),
            "una clave de orden pequeno no debe registrarse"
        );

        // El codigo correcto sigue valiendo: no se consumio intento.
        let (identity, info) = controller();
        host.redeem_pairing(code.as_str(), info, &identity.public_key().to_hex(), NOW)
            .expect("el emparejamiento seguia vivo");
    }

    #[test]
    fn cancelling_a_pairing_invalidates_the_code() {
        let mut host = service();
        let (_, code) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        assert!(host.cancel_pairing());
        assert!(!host.cancel_pairing(), "ya no habia ninguno");

        let (identity, info) = controller();
        let err = host
            .redeem_pairing(code.as_str(), info, &identity.public_key().to_hex(), NOW)
            .unwrap_err();
        assert!(matches!(err, HostError::NoPairingOpen));
    }

    #[test]
    fn the_invite_carries_everything_the_qr_needs() {
        let mut host = service();
        host.set_signaling_url("wss://nexus.example/signaling")
            .expect("guarda URL");
        let (_, code) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");

        let invite = host.current_invite(NOW).expect("invitacion");
        assert_eq!(invite.signaling_url, "wss://nexus.example/signaling");
        assert_eq!(invite.host_id, host.device_id());
        assert_eq!(invite.code, code.as_str());
        assert_eq!(
            invite.host_fingerprint,
            host.public_key().short_fingerprint()
        );

        // Y sobrevive al viaje por el QR.
        let json = invite.to_json().expect("serializa");
        let back = PairingInvite::from_json(&json).expect("deserializa");
        assert_eq!(back, invite);
    }

    #[test]
    fn there_is_no_invite_without_an_open_pairing() {
        let mut host = service();
        assert!(matches!(
            host.current_invite(NOW).unwrap_err(),
            HostError::NoPairingOpen
        ));
    }

    #[test]
    fn revoking_a_device_keeps_it_listed_but_inactive() {
        let mut host = service();
        let (_, code) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        let (identity, info) = controller();
        let record = host
            .redeem_pairing(code.as_str(), info, &identity.public_key().to_hex(), NOW)
            .expect("empareja");

        host.revoke_device(record.info.id, Timestamp(NOW.0 + 10))
            .expect("revoca");
        let devices = host.list_devices().expect("lista");
        assert_eq!(
            devices.len(),
            1,
            "queda en la lista para que se vea el historial"
        );
        assert!(!devices[0].is_active());
    }

    #[test]
    fn revoking_or_deleting_an_unknown_device_is_reported() {
        let mut host = service();
        let ghost = DeviceId::new();
        assert!(matches!(
            host.revoke_device(ghost, NOW).unwrap_err(),
            HostError::UnknownDevice { .. }
        ));
        assert!(matches!(
            host.delete_device(ghost).unwrap_err(),
            HostError::UnknownDevice { .. }
        ));
    }

    #[test]
    fn deleting_a_device_removes_it_from_the_list() {
        let mut host = service();
        let (_, code) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        let (identity, info) = controller();
        let record = host
            .redeem_pairing(code.as_str(), info, &identity.public_key().to_hex(), NOW)
            .expect("empareja");

        host.delete_device(record.info.id).expect("borra");
        assert!(host.list_devices().expect("lista").is_empty());
    }

    #[test]
    fn permissions_can_be_changed_after_pairing() {
        let mut host = service();
        let (_, code) = host
            .open_pairing(PermissionSet::view_only(), false, NOW)
            .expect("abre");
        let (identity, info) = controller();
        let record = host
            .redeem_pairing(code.as_str(), info, &identity.public_key().to_hex(), NOW)
            .expect("empareja");

        let updated = host
            .set_device_permissions(record.info.id, PermissionSet::standard())
            .expect("cambia permisos");
        assert_eq!(updated.permissions, PermissionSet::standard());

        let reloaded = &host.list_devices().expect("lista")[0];
        assert_eq!(reloaded.permissions, PermissionSet::standard());
    }

    #[test]
    fn the_signaling_url_must_use_tls() {
        let host = service();
        assert!(host.signaling_url().expect("lee").is_none());

        for bad in [
            "ws://nexus.example",
            "http://nexus.example",
            "nexus.example",
        ] {
            assert!(
                host.set_signaling_url(bad).is_err(),
                "{bad} no deberia aceptarse sin TLS"
            );
        }

        host.set_signaling_url("  wss://nexus.example/signaling  ")
            .expect("acepta wss");
        assert_eq!(
            host.signaling_url().expect("lee").as_deref(),
            Some("wss://nexus.example/signaling"),
            "debe guardarse recortada"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn fuera_de_windows_la_enumeracion_dice_que_no_hay_backend() {
        let host = service();
        let err = host.list_monitors().unwrap_err();
        assert!(
            matches!(
                err,
                HostError::Capture(nexus_capture::CaptureError::UnsupportedPlatform { .. })
            ),
            "debe decirlo claramente en lugar de devolver una lista vacia: {err:?}"
        );
    }

    #[test]
    #[cfg(windows)]
    fn en_windows_la_enumeracion_encuentra_al_menos_una_pantalla() {
        let host = service();
        let layout = host.list_monitors().expect("Windows deberia enumerar");
        assert!(
            !layout.is_empty(),
            "un equipo con escritorio tiene al menos una pantalla"
        );
        layout
            .validate()
            .expect("todas las pantallas deben ser validas");
        assert!(
            layout.primary().is_some(),
            "deberia haber una pantalla principal"
        );
    }

    #[test]
    fn el_estado_del_motor_refleja_la_plataforma_real() {
        let host = service();
        let status = host.engine_status();

        // Cada comprobacion explica lo que encontro, funcione o no.
        assert!(!status.capture.detail.is_empty());
        assert!(!status.input.detail.is_empty());

        if cfg!(windows) {
            assert!(
                status.input.working,
                "en Windows deberia haber inyeccion de entrada"
            );
        } else {
            assert!(!status.capture.working, "fuera de Windows no hay captura");
            assert!(!status.input.working, "fuera de Windows no hay inyeccion");
            assert!(!status.is_ready());
        }
    }

    #[test]
    fn a_host_opened_on_disk_keeps_its_devices_across_restarts() {
        let temp = tempfile::tempdir().expect("temporal");

        let device_id = {
            let mut host = HostService::open(temp.path()).expect("abre");
            let (_, code) = host
                .open_pairing(PermissionSet::standard(), false, NOW)
                .expect("abre");
            let (identity, info) = controller();
            host.redeem_pairing(code.as_str(), info, &identity.public_key().to_hex(), NOW)
                .expect("empareja")
                .info
                .id
        };

        let host = HostService::open(temp.path()).expect("reabre");
        let devices = host.list_devices().expect("lista");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].info.id, device_id);
    }

    #[test]
    fn the_host_identity_is_stable_across_restarts() {
        let temp = tempfile::tempdir().expect("temporal");
        let first = HostService::open(temp.path()).expect("abre").public_key();
        let second = HostService::open(temp.path()).expect("reabre").public_key();
        assert_eq!(first, second);
    }
}
