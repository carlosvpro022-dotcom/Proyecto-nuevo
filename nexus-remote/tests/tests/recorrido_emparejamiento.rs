//! Recorrido completo: emparejar un dispositivo y conectarlo despues.
//!
//! Es la prueba que de verdad importa en la etapa 1, porque cruza todos los
//! crates: el host abre un emparejamiento, el dispositivo canjea el codigo con
//! su clave publica, queda en la lista blanca de `SQLite`, y a partir de ahi
//! se autentica por firma sin volver a usar el codigo.
//!
//! Si esta prueba pasa, el camino "dispositivo desconocido -> dispositivo
//! autorizado -> dispositivo autenticado" funciona de extremo a extremo.

use nexus_core::{DeviceId, DeviceInfo, DevicePlatform, DeviceRole, PermissionSet, Timestamp};
use nexus_security::challenge::{AuthRole, AuthTranscript, Nonce};
use nexus_security::registry::{AuthRequest, Authenticator, DeviceRegistry};
use nexus_security::{DeviceIdentity, SecurityError};
use nexus_windows_host::{HostService, Store};

const NOW: Timestamp = Timestamp(1_730_000_000_000);

/// Simula el dispositivo Apple: su identidad y lo que declara de si mismo.
struct Controller {
    identity: DeviceIdentity,
    info: DeviceInfo,
}

impl Controller {
    fn new(name: &str, platform: DevicePlatform) -> Self {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let info = DeviceInfo {
            id: identity.device_id(),
            display_name: name.to_owned(),
            platform,
            role: DeviceRole::Controller,
            os_version: "18.1".to_owned(),
            app_version: "0.1.0".to_owned(),
        };
        Self { identity, info }
    }

    /// Construye la prueba de identidad que el dispositivo enviaria.
    fn prove(&self, server_nonce: Nonce, now: Timestamp) -> AuthRequest {
        let client_nonce = Nonce::generate();
        let transcript = AuthTranscript::new(
            server_nonce,
            client_nonce,
            self.identity.device_id(),
            AuthRole::Controller,
            now,
        );
        AuthRequest {
            device_id: self.identity.device_id(),
            server_nonce,
            client_nonce,
            role: AuthRole::Controller,
            timestamp: now,
            signature_hex: self.identity.sign_hex(&transcript.to_bytes()),
        }
    }
}

fn host() -> HostService {
    HostService::open_ephemeral(DeviceIdentity::generate(DeviceId::new()))
        .expect("el host debe arrancar")
}

#[test]
fn un_dispositivo_emparejado_se_autentica_despues_sin_el_codigo() {
    let mut host = host();
    let ipad = Controller::new("iPad Pro", DevicePlatform::IpadOs);

    // 1. El host muestra un codigo.
    let (_, code) = host
        .open_pairing(PermissionSet::standard(), false, NOW)
        .expect("el host abre el emparejamiento");

    // 2. El dispositivo lo canjea con su clave publica.
    let record = host
        .redeem_pairing(
            code.as_str(),
            ipad.info.clone(),
            &ipad.identity.public_key().to_hex(),
            NOW,
        )
        .expect("el codigo correcto debe emparejar");
    assert_eq!(record.public_key, ipad.identity.public_key());

    // 3. A partir de aqui el codigo ya no interviene: se autentica por firma.
    let store = Store::open_in_memory().expect("almacen");
    let mut auth = Authenticator::new(store);
    auth.registry_mut()
        .upsert(record.info.id, record.clone())
        .expect("replica el registro");

    let challenge = auth
        .issue_challenge(NOW)
        .expect("el servidor emite un reto");
    let outcome = auth
        .verify(&ipad.prove(challenge.nonce, NOW), NOW)
        .expect("el dispositivo emparejado debe autenticarse");

    assert_eq!(outcome.permissions, PermissionSet::standard());
    assert_eq!(outcome.counter_transcript.role, AuthRole::Host);
}

#[test]
fn un_dispositivo_sin_emparejar_no_entra_aunque_firme_bien() {
    let host = host();
    let intruso = Controller::new("Portatil desconocido", DevicePlatform::Unknown);

    let store = Store::open_in_memory().expect("almacen");
    let mut auth = Authenticator::new(store);
    let challenge = auth.issue_challenge(NOW).expect("reto");

    // La firma es criptograficamente valida; lo que falla es que no esta en la
    // lista blanca. Es la diferencia entre "sabe firmar" y "esta autorizado".
    assert_eq!(
        auth.verify(&intruso.prove(challenge.nonce, NOW), NOW)
            .unwrap_err(),
        SecurityError::UnknownDevice
    );
    assert!(host.list_devices().expect("lista").is_empty());
}

#[test]
fn revocar_corta_el_acceso_de_inmediato() {
    let mut host = host();
    let iphone = Controller::new("iPhone", DevicePlatform::Ios);

    let (_, code) = host
        .open_pairing(PermissionSet::standard(), false, NOW)
        .expect("abre emparejamiento");
    let record = host
        .redeem_pairing(
            code.as_str(),
            iphone.info.clone(),
            &iphone.identity.public_key().to_hex(),
            NOW,
        )
        .expect("empareja");

    let store = Store::open_in_memory().expect("almacen");
    let mut auth = Authenticator::new(store);
    auth.registry_mut()
        .upsert(record.info.id, record.clone())
        .expect("registra");

    // Antes de revocar entra.
    let challenge = auth.issue_challenge(NOW).expect("reto");
    auth.verify(&iphone.prove(challenge.nonce, NOW), NOW)
        .expect("entra");

    // Despues de revocar, no.
    auth.registry_mut()
        .revoke(record.info.id, NOW)
        .expect("revoca");
    let challenge = auth.issue_challenge(NOW).expect("reto");
    assert_eq!(
        auth.verify(&iphone.prove(challenge.nonce, NOW), NOW)
            .unwrap_err(),
        SecurityError::DeviceRevoked
    );
}

#[test]
fn el_codigo_de_un_emparejamiento_no_vale_en_otro_host() {
    // Dos hosts distintos abren emparejamientos a la vez. El codigo de uno no
    // puede canjearse en el otro, porque la derivacion incluye el
    // identificador del emparejamiento.
    let mut primero = host();
    let mut segundo = host();

    let (_, code) = primero
        .open_pairing(PermissionSet::standard(), false, NOW)
        .expect("abre");
    segundo
        .open_pairing(PermissionSet::standard(), false, NOW)
        .expect("abre");

    let ipad = Controller::new("iPad", DevicePlatform::IpadOs);
    let err = segundo
        .redeem_pairing(
            code.as_str(),
            ipad.info,
            &ipad.identity.public_key().to_hex(),
            NOW,
        )
        .unwrap_err();

    assert!(
        format!("{err}").contains("codigo de emparejamiento incorrecto"),
        "error inesperado: {err}"
    );
    assert!(segundo.list_devices().expect("lista").is_empty());
}

#[test]
fn varios_dispositivos_conviven_con_permisos_distintos() {
    let mut host = host();

    let ipad = Controller::new("iPad Pro", DevicePlatform::IpadOs);
    let (_, code) = host
        .open_pairing(PermissionSet::full(), true, NOW)
        .expect("abre");
    host.redeem_pairing(
        code.as_str(),
        ipad.info,
        &ipad.identity.public_key().to_hex(),
        NOW,
    )
    .expect("empareja el iPad con todos los permisos");

    let iphone = Controller::new("iPhone", DevicePlatform::Ios);
    let (_, code) = host
        .open_pairing(PermissionSet::view_only(), false, NOW)
        .expect("abre");
    host.redeem_pairing(
        code.as_str(),
        iphone.info,
        &iphone.identity.public_key().to_hex(),
        NOW,
    )
    .expect("empareja el iPhone solo para ver");

    let devices = host.list_devices().expect("lista");
    assert_eq!(devices.len(), 2);

    let full = devices
        .iter()
        .find(|d| d.info.display_name == "iPad Pro")
        .expect("iPad");
    let view = devices
        .iter()
        .find(|d| d.info.display_name == "iPhone")
        .expect("iPhone");

    assert!(full.permissions.has(nexus_core::Permission::Shutdown));
    assert!(full.unattended_access);
    assert!(!view.permissions.has(nexus_core::Permission::ControlPointer));
    assert!(!view.unattended_access);
}

#[test]
fn el_emparejamiento_sobrevive_a_un_reinicio_del_host() {
    let temp = tempfile::tempdir().expect("carpeta temporal");
    let ipad = Controller::new("iPad Pro", DevicePlatform::IpadOs);

    let (host_key, device_id) = {
        let mut host = HostService::open(temp.path()).expect("arranca");
        let (_, code) = host
            .open_pairing(PermissionSet::standard(), false, NOW)
            .expect("abre");
        let record = host
            .redeem_pairing(
                code.as_str(),
                ipad.info.clone(),
                &ipad.identity.public_key().to_hex(),
                NOW,
            )
            .expect("empareja");
        (host.public_key(), record.info.id)
    };

    // Se cierra la aplicacion y se vuelve a abrir.
    let host = HostService::open(temp.path()).expect("rearranca");

    assert_eq!(
        host.public_key(),
        host_key,
        "la identidad del host no debe cambiar"
    );
    let devices = host.list_devices().expect("lista");
    assert_eq!(
        devices.len(),
        1,
        "el dispositivo emparejado debe seguir ahi"
    );
    assert_eq!(devices[0].info.id, device_id);
    assert_eq!(devices[0].public_key, ipad.identity.public_key());
}
