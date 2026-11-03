//! Directorio de identidades del servidor de senalizacion.
//!
//! # Que es y que no es
//!
//! Este directorio guarda una sola cosa: que clave publica corresponde a que
//! identificador de dispositivo. **No decide permisos y no decide confianza.**
//! Quien decide si un iPad puede controlar un PC es el host, en su propio
//! registro, y lo hace cada vez que llega una peticion de sesion.
//!
//! La separacion es deliberada. Si el servidor concediese permisos, un servidor
//! comprometido podria autorizarse dispositivos a si mismo. Tal como esta, lo
//! peor que puede hacer un servidor comprometido es negar el servicio o
//! intentar colocarse en medio, y eso ultimo lo detecta la atadura de sesion
//! (`nexus_security::binding`).
//!
//! # Por que hace falta entonces
//!
//! Por tres motivos concretos:
//!
//! 1. Para poder verificar la firma de quien se conecta hace falta su clave.
//! 2. Para que **nadie pueda apropiarse de un identificador ya conocido**: si
//!    el identificador existe y la clave no coincide, se rechaza la conexion.
//! 3. Para poder cortar a un dispositivo en el servidor, sin depender de que el
//!    host este encendido.
//!
//! # Primera conexion
//!
//! Un identificador que el servidor no ha visto nunca se trata segun el papel
//! que declara:
//!
//! - **Host**: exige el testigo de alta del servidor. Sin el, cualquiera podria
//!   registrar el identificador de un host antes que el propio host y
//!   quedarse con el. El testigo se compara en tiempo constante y no se
//!   escribe en ningun registro.
//! - **Controlador**: se acepta la clave que presente y queda atada a ese
//!   identificador. No hace falta testigo porque la decision que importa no es
//!   esta: el host comprobara despues si esa clave esta en su lista de
//!   dispositivos emparejados, y si no lo esta, la sesion no ocurre. Pedir
//!   testigo aqui obligaria a repartir un secreto del servidor a cada iPad
//!   para no ganar nada.

use std::collections::HashMap;

use nexus_core::device::DeviceRole;
use nexus_core::{DeviceId, DeviceInfo, PermissionSet, Timestamp};
use nexus_security::registry::{DeviceRecord, DeviceRegistry};
use nexus_security::{DevicePublicKey, SecurityResult, secret_matches};

/// Numero maximo de identidades que el directorio guarda.
///
/// Acota la memoria que puede hacer reservar quien se conecte repetidamente con
/// identificadores nuevos.
pub const DEFAULT_MAX_DEVICES: usize = 4_096;

/// Resultado de admitir una identidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// El identificador ya estaba y la clave coincide.
    Known,
    /// Identificador nuevo: se acaba de registrar la clave.
    Enrolled,
}

/// Motivo por el que el servidor no admite una identidad.
///
/// Hacia el cliente todos se traducen al mismo rechazo generico: distinguirlos
/// confirmaria a un atacante que un identificador existe. El detalle se queda
/// en el registro del servidor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// El identificador existe pero con otra clave: intento de apropiacion.
    KeyMismatch,
    /// Es un host desconocido y no presento testigo de alta.
    EnrollmentTokenRequired,
    /// Presento un testigo de alta que no es el configurado.
    EnrollmentTokenInvalid,
    /// El dispositivo esta revocado en el servidor.
    Revoked,
    /// El directorio esta lleno.
    DirectoryFull,
}

impl Refusal {
    /// Texto estable para el registro del servidor.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::KeyMismatch => "clave distinta de la registrada",
            Self::EnrollmentTokenRequired => "host desconocido sin testigo de alta",
            Self::EnrollmentTokenInvalid => "testigo de alta incorrecto",
            Self::Revoked => "dispositivo revocado en el servidor",
            Self::DirectoryFull => "directorio de identidades lleno",
        }
    }
}

/// Directorio de identidades conocidas por el servidor.
#[derive(Debug)]
pub struct EnrollmentStore {
    devices: HashMap<DeviceId, DeviceRecord>,
    host_enrollment_token: String,
    max_devices: usize,
}

impl EnrollmentStore {
    /// Crea el directorio con el testigo de alta de hosts.
    #[must_use]
    pub fn new(host_enrollment_token: String) -> Self {
        Self {
            devices: HashMap::new(),
            host_enrollment_token,
            max_devices: DEFAULT_MAX_DEVICES,
        }
    }

    /// Crea el directorio indicando tambien el tope de identidades.
    #[must_use]
    pub fn with_capacity(host_enrollment_token: String, max_devices: usize) -> Self {
        Self {
            devices: HashMap::new(),
            host_enrollment_token,
            max_devices,
        }
    }

    /// Numero de identidades registradas.
    #[must_use]
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Indica si no hay ninguna identidad registrada.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// Comprueba y registra la identidad que presenta una conexion.
    ///
    /// # Errors
    ///
    /// La variante de [`Refusal`] que corresponda. El llamante debe traducirla
    /// a un rechazo generico antes de contestar al cliente.
    pub fn admit(
        &mut self,
        info: &DeviceInfo,
        public_key: &DevicePublicKey,
        enrollment_token: Option<&str>,
        now: Timestamp,
    ) -> Result<Admission, Refusal> {
        if let Some(existing) = self.devices.get(&info.id) {
            if existing.revoked_at.is_some() {
                return Err(Refusal::Revoked);
            }
            if existing.public_key != *public_key {
                return Err(Refusal::KeyMismatch);
            }
            return Ok(Admission::Known);
        }

        if info.role == DeviceRole::Host {
            match enrollment_token {
                None => return Err(Refusal::EnrollmentTokenRequired),
                Some(token) if !secret_matches(&self.host_enrollment_token, token) => {
                    return Err(Refusal::EnrollmentTokenInvalid);
                }
                Some(_) => {}
            }
        }

        if self.devices.len() >= self.max_devices {
            return Err(Refusal::DirectoryFull);
        }

        self.devices.insert(
            info.id,
            DeviceRecord::new(
                info.clone(),
                public_key.clone(),
                // El servidor no concede permisos. El conjunto vacio deja claro
                // que esta decision no se toma aqui.
                PermissionSet::empty(),
                false,
                now,
            ),
        );
        Ok(Admission::Enrolled)
    }

    /// Olvida una identidad registrada.
    ///
    /// Se usa cuando un dispositivo se dio de alta al presentarse pero no llego
    /// a demostrar que tiene la clave privada. Sin esto, cualquiera podria
    /// llenar el directorio presentandose con identificadores nuevos y sin
    /// firmar nunca nada.
    ///
    /// Devuelve `true` si existia.
    pub fn forget(&mut self, device_id: DeviceId) -> bool {
        self.devices.remove(&device_id).is_some()
    }

    /// Papel declarado por un dispositivo registrado.
    #[must_use]
    pub fn role_of(&self, device_id: DeviceId) -> Option<DeviceRole> {
        self.devices.get(&device_id).map(|d| d.info.role)
    }
}

impl DeviceRegistry for EnrollmentStore {
    fn get(&self, device_id: DeviceId) -> Option<DeviceRecord> {
        self.devices.get(&device_id).cloned()
    }

    fn upsert(&mut self, device_id: DeviceId, record: DeviceRecord) -> SecurityResult<()> {
        self.devices.insert(device_id, record);
        Ok(())
    }

    fn revoke(&mut self, device_id: DeviceId, now: Timestamp) -> SecurityResult<bool> {
        match self.devices.get_mut(&device_id) {
            Some(record) if record.revoked_at.is_none() => {
                record.revoked_at = Some(now);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn touch(&mut self, device_id: DeviceId, now: Timestamp) -> SecurityResult<()> {
        if let Some(record) = self.devices.get_mut(&device_id) {
            record.last_seen = Some(now);
        }
        Ok(())
    }

    fn list(&self) -> Vec<(DeviceId, DeviceRecord)> {
        self.devices
            .iter()
            .map(|(id, record)| (*id, record.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::device::DevicePlatform;
    use nexus_security::DeviceIdentity;

    const NOW: Timestamp = Timestamp(1_730_000_000_000);
    const TOKEN: &str = "testigo-de-alta-del-servidor";

    fn info(role: DeviceRole) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId::new(),
            display_name: match role {
                DeviceRole::Host => "PC del salon".to_owned(),
                DeviceRole::Controller => "iPad Pro".to_owned(),
            },
            platform: match role {
                DeviceRole::Host => DevicePlatform::Windows,
                DeviceRole::Controller => DevicePlatform::IpadOs,
            },
            role,
            os_version: "1.0".to_owned(),
            app_version: "0.1.0".to_owned(),
        }
    }

    fn store() -> EnrollmentStore {
        EnrollmentStore::new(TOKEN.to_owned())
    }

    fn key() -> DevicePublicKey {
        DeviceIdentity::generate(DeviceId::new()).public_key()
    }

    #[test]
    fn a_host_with_the_right_token_is_enrolled() {
        let mut store = store();
        let host = info(DeviceRole::Host);
        assert_eq!(
            store.admit(&host, &key(), Some(TOKEN), NOW),
            Ok(Admission::Enrolled)
        );
        assert_eq!(store.len(), 1);
        assert_eq!(store.role_of(host.id), Some(DeviceRole::Host));
    }

    #[test]
    fn a_new_host_without_a_token_is_refused() {
        let mut store = store();
        assert_eq!(
            store.admit(&info(DeviceRole::Host), &key(), None, NOW),
            Err(Refusal::EnrollmentTokenRequired)
        );
        assert!(store.is_empty(), "no debe quedar registrado");
    }

    #[test]
    fn a_new_host_with_a_wrong_token_is_refused() {
        let mut store = store();
        assert_eq!(
            store.admit(&info(DeviceRole::Host), &key(), Some("otro"), NOW),
            Err(Refusal::EnrollmentTokenInvalid)
        );
        assert!(store.is_empty());
    }

    #[test]
    fn a_controller_needs_no_token() {
        // La decision que importa no es esta: la toma el host al comprobar su
        // lista de dispositivos emparejados.
        let mut store = store();
        assert_eq!(
            store.admit(&info(DeviceRole::Controller), &key(), None, NOW),
            Ok(Admission::Enrolled)
        );
    }

    #[test]
    fn a_known_device_with_the_same_key_is_recognised() {
        let mut store = store();
        let device = info(DeviceRole::Controller);
        let device_key = key();
        store
            .admit(&device, &device_key, None, NOW)
            .expect("primera vez");
        assert_eq!(
            store.admit(&device, &device_key, None, NOW),
            Ok(Admission::Known)
        );
        assert_eq!(store.len(), 1, "no debe duplicarse");
    }

    #[test]
    fn taking_over_a_known_identifier_is_refused() {
        // El ataque: alguien conoce el identificador de un dispositivo (que es
        // publico, aparece en una invitacion) e intenta conectarse con una
        // clave propia.
        let mut store = store();
        let device = info(DeviceRole::Controller);
        store
            .admit(&device, &key(), None, NOW)
            .expect("primera vez");

        assert_eq!(
            store.admit(&device, &key(), None, NOW),
            Err(Refusal::KeyMismatch)
        );
    }

    #[test]
    fn a_token_does_not_let_you_take_over_an_identifier() {
        // Ni siquiera con el testigo correcto: el testigo sirve para dar de
        // alta, no para desplazar a un dispositivo ya registrado.
        let mut store = store();
        let host = info(DeviceRole::Host);
        store
            .admit(&host, &key(), Some(TOKEN), NOW)
            .expect("primera vez");

        assert_eq!(
            store.admit(&host, &key(), Some(TOKEN), NOW),
            Err(Refusal::KeyMismatch)
        );
    }

    #[test]
    fn an_unproven_enrolment_can_be_forgotten() {
        // El caso: alguien se presenta con un identificador nuevo y no firma el
        // reto. Esa identidad no debe quedarse ocupando sitio.
        let mut store = store();
        let device = info(DeviceRole::Controller);
        store.admit(&device, &key(), None, NOW).expect("alta");
        assert_eq!(store.len(), 1);

        assert!(store.forget(device.id));
        assert!(store.is_empty());
        assert!(!store.forget(device.id), "la segunda vez ya no existe");
    }

    #[test]
    fn a_revoked_device_cannot_come_back() {
        let mut store = store();
        let device = info(DeviceRole::Controller);
        let device_key = key();
        store
            .admit(&device, &device_key, None, NOW)
            .expect("primera vez");

        assert!(store.revoke(device.id, NOW).expect("revoca"));
        assert_eq!(
            store.admit(&device, &device_key, None, NOW),
            Err(Refusal::Revoked)
        );
    }

    #[test]
    fn revoking_twice_reports_no_change() {
        let mut store = store();
        let device = info(DeviceRole::Controller);
        store.admit(&device, &key(), None, NOW).expect("alta");
        assert!(store.revoke(device.id, NOW).expect("revoca"));
        assert!(!store.revoke(device.id, NOW).expect("ya estaba"));
    }

    #[test]
    fn revoking_an_unknown_device_reports_no_change() {
        let mut store = store();
        assert!(!store.revoke(DeviceId::new(), NOW).expect("no existe"));
    }

    #[test]
    fn the_directory_has_a_size_limit() {
        let mut store = EnrollmentStore::with_capacity(TOKEN.to_owned(), 2);
        for _ in 0..2 {
            store
                .admit(&info(DeviceRole::Controller), &key(), None, NOW)
                .expect("cabe");
        }
        assert_eq!(
            store.admit(&info(DeviceRole::Controller), &key(), None, NOW),
            Err(Refusal::DirectoryFull)
        );
    }

    #[test]
    fn enrolled_devices_carry_no_permissions() {
        // El servidor no concede permisos; si algun dia este registro empieza a
        // llevarlos, esta prueba falla y obliga a revisarlo.
        let mut store = store();
        let device = info(DeviceRole::Controller);
        store.admit(&device, &key(), None, NOW).expect("alta");
        let record = store.get(device.id).expect("registrado");
        assert_eq!(record.permissions, PermissionSet::empty());
        assert!(!record.unattended_access);
    }

    #[test]
    fn a_server_without_a_token_configured_enrolls_no_hosts() {
        // Una configuracion sin testigo no debe dejar pasar a un host nuevo ni
        // presentando una cadena vacia.
        let mut store = EnrollmentStore::new(String::new());
        assert_eq!(
            store.admit(&info(DeviceRole::Host), &key(), Some(""), NOW),
            Err(Refusal::EnrollmentTokenInvalid)
        );
    }

    #[test]
    fn touch_records_the_last_connection() {
        let mut store = store();
        let device = info(DeviceRole::Controller);
        store.admit(&device, &key(), None, NOW).expect("alta");
        store.touch(device.id, NOW).expect("anota");
        assert_eq!(store.get(device.id).expect("existe").last_seen, Some(NOW));
    }

    #[test]
    fn the_authenticator_can_use_this_store() {
        // El flujo de autenticacion completo de la etapa 1 se reutiliza tal
        // cual: este directorio solo cambia de donde salen las claves.
        use nexus_security::registry::AuthRequest;
        use nexus_security::{AuthRole, AuthTranscript, Authenticator, Nonce};

        let mut store = store();
        let device = info(DeviceRole::Controller);
        let identity = DeviceIdentity::generate(device.id);
        store
            .admit(&device, &identity.public_key(), None, NOW)
            .expect("alta");

        let mut authenticator = Authenticator::new(store);
        let challenge = authenticator.issue_challenge(NOW).expect("reto");
        let client_nonce = Nonce::generate();
        let transcript = AuthTranscript::new(
            challenge.nonce,
            client_nonce,
            device.id,
            AuthRole::Controller,
            NOW,
        );

        let outcome = authenticator
            .verify(
                &AuthRequest {
                    device_id: device.id,
                    server_nonce: challenge.nonce,
                    client_nonce,
                    role: AuthRole::Controller,
                    timestamp: NOW,
                    signature_hex: identity.sign_hex(&transcript.to_bytes()),
                },
                NOW,
            )
            .expect("deberia autenticar");
        assert_eq!(outcome.device.info.id, device.id);
    }
}
