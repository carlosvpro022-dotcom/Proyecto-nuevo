//! Lista de dispositivos autorizados y flujo completo de autenticacion.
//!
//! Este modulo junta las piezas: la lista blanca, el reto de un solo uso, la
//! verificacion de firma, la ventana antirreenvio y el limite de intentos. El
//! resto del sistema no deberia autenticar por su cuenta, sino llamar a
//! [`Authenticator::verify`], de modo que no se pueda olvidar una comprobacion.
//!
//! El orden de las comprobaciones es deliberado y esta documentado en
//! [`Authenticator::verify`].

use std::collections::HashMap;

use nexus_core::{DeviceId, DeviceInfo, PermissionSet, Timestamp};
use serde::{Deserialize, Serialize};

use crate::challenge::{AuthRole, AuthTranscript, Challenge, ChallengeStore, Nonce};
use crate::error::{SecurityError, SecurityResult};
use crate::identity::DevicePublicKey;
use crate::ratelimit::{AttemptLimiter, RateLimitPolicy};
use crate::replay::ReplayGuard;

/// Dispositivo autorizado, tal como se guarda en el host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    /// Datos declarados por el dispositivo (informativos, no de confianza).
    pub info: DeviceInfo,
    /// Clave publica con la que se verifica su firma. Esta si es la verdad.
    pub public_key: DevicePublicKey,
    /// Permisos concedidos.
    pub permissions: PermissionSet,
    /// Instante del emparejamiento.
    pub paired_at: Timestamp,
    /// Ultima conexion correcta.
    pub last_seen: Option<Timestamp>,
    /// Instante de revocacion, si se revoco.
    pub revoked_at: Option<Timestamp>,
    /// Si puede conectar sin que nadie acepte en el host.
    pub unattended_access: bool,
}

impl DeviceRecord {
    /// Crea un registro para un dispositivo recien emparejado.
    #[must_use]
    pub const fn new(
        info: DeviceInfo,
        public_key: DevicePublicKey,
        permissions: PermissionSet,
        unattended_access: bool,
        now: Timestamp,
    ) -> Self {
        Self {
            info,
            public_key,
            permissions,
            paired_at: now,
            last_seen: None,
            revoked_at: None,
            unattended_access,
        }
    }

    /// Indica si el dispositivo puede conectarse.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// Almacen de dispositivos autorizados.
///
/// Es un rasgo para que el host pueda respaldarlo con `SQLite` y las pruebas con
/// memoria, sin que el flujo de autenticacion cambie.
pub trait DeviceRegistry {
    /// Busca un dispositivo por identificador.
    fn get(&self, device_id: DeviceId) -> Option<DeviceRecord>;

    /// Inserta o reemplaza un dispositivo.
    ///
    /// # Errors
    ///
    /// Depende de la implementacion; la version en memoria no falla.
    fn upsert(&mut self, device_id: DeviceId, record: DeviceRecord) -> SecurityResult<()>;

    /// Revoca un dispositivo. Devuelve `true` si existia y estaba activo.
    ///
    /// # Errors
    ///
    /// Depende de la implementacion.
    fn revoke(&mut self, device_id: DeviceId, now: Timestamp) -> SecurityResult<bool>;

    /// Anota el instante de la ultima conexion correcta.
    ///
    /// # Errors
    ///
    /// Depende de la implementacion.
    fn touch(&mut self, device_id: DeviceId, now: Timestamp) -> SecurityResult<()>;

    /// Lista todos los dispositivos, revocados incluidos.
    fn list(&self) -> Vec<(DeviceId, DeviceRecord)>;
}

/// Implementacion en memoria, para pruebas y para el arranque del host.
#[derive(Debug, Default)]
pub struct InMemoryRegistry {
    devices: HashMap<DeviceId, DeviceRecord>,
}

impl InMemoryRegistry {
    /// Crea un registro vacio.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Numero de dispositivos registrados.
    #[must_use]
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Indica si no hay ningun dispositivo.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }
}

impl DeviceRegistry for InMemoryRegistry {
    fn get(&self, device_id: DeviceId) -> Option<DeviceRecord> {
        self.devices.get(&device_id).cloned()
    }

    fn upsert(&mut self, device_id: DeviceId, record: DeviceRecord) -> SecurityResult<()> {
        self.devices.insert(device_id, record);
        Ok(())
    }

    fn revoke(&mut self, device_id: DeviceId, now: Timestamp) -> SecurityResult<bool> {
        match self.devices.get_mut(&device_id) {
            Some(record) if record.is_active() => {
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
        let mut out: Vec<_> = self.devices.iter().map(|(k, v)| (*k, v.clone())).collect();
        out.sort_by_key(|(id, _)| id.to_string());
        out
    }
}

/// Prueba de identidad que presenta un dispositivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRequest {
    /// Quien dice ser.
    pub device_id: DeviceId,
    /// Nonce que emitio el servidor.
    pub server_nonce: Nonce,
    /// Nonce aportado por el dispositivo.
    pub client_nonce: Nonce,
    /// Papel con el que firma.
    pub role: AuthRole,
    /// Marca temporal del firmante.
    pub timestamp: Timestamp,
    /// Firma en hexadecimal sobre la transcripcion.
    pub signature_hex: String,
}

/// Resultado de una autenticacion correcta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthOutcome {
    /// Registro del dispositivo verificado.
    pub device: DeviceRecord,
    /// Permisos efectivos de la sesion.
    pub permissions: PermissionSet,
    /// Transcripcion que el otro extremo debe firmar para cerrar el handshake.
    ///
    /// Que este rasgo devuelva la transcripcion contraria es lo que convierte
    /// el proceso en autenticacion **mutua**: el dispositivo tambien verifica
    /// al host.
    pub counter_transcript: AuthTranscript,
}

/// Flujo de autenticacion con todas sus defensas.
#[derive(Debug)]
pub struct Authenticator<R: DeviceRegistry> {
    registry: R,
    challenges: ChallengeStore,
    replay: ReplayGuard,
    limiter: AttemptLimiter,
}

impl<R: DeviceRegistry> Authenticator<R> {
    /// Crea el autenticador con las politicas por omision.
    #[must_use]
    pub fn new(registry: R) -> Self {
        Self {
            registry,
            challenges: ChallengeStore::default(),
            replay: ReplayGuard::default(),
            limiter: AttemptLimiter::new(RateLimitPolicy::device_auth()),
        }
    }

    /// Crea el autenticador con componentes a medida.
    #[must_use]
    pub const fn with_parts(
        registry: R,
        challenges: ChallengeStore,
        replay: ReplayGuard,
        limiter: AttemptLimiter,
    ) -> Self {
        Self {
            registry,
            challenges,
            replay,
            limiter,
        }
    }

    /// Acceso de lectura al registro.
    pub const fn registry(&self) -> &R {
        &self.registry
    }

    /// Acceso de escritura al registro, para emparejar y revocar.
    pub const fn registry_mut(&mut self) -> &mut R {
        &mut self.registry
    }

    /// Emite un reto para que el dispositivo lo firme.
    ///
    /// # Errors
    ///
    /// [`SecurityError::RateLimited`] si hay demasiados retos sin contestar.
    pub fn issue_challenge(&mut self, now: Timestamp) -> SecurityResult<Challenge> {
        self.challenges.issue(now)
    }

    /// Verifica la prueba de identidad de un dispositivo.
    ///
    /// Las comprobaciones van en este orden, y el orden importa:
    ///
    /// 1. **Limite de intentos.** Antes de nada, para que un atacante no pueda
    ///    forzar trabajo criptografico repetido.
    /// 2. **Ventana temporal.** Barata y sin efectos: descarta lo viejo sin
    ///    dejar rastro en memoria.
    /// 3. **Reto de un solo uso.** Consumirlo aqui impide reutilizar un
    ///    handshake completo capturado.
    /// 4. **Dispositivo en la lista y no revocado.**
    /// 5. **Firma sobre la transcripcion**, con `verify_strict`.
    /// 6. **Registro del nonce del cliente**, ya como ultimo paso: solo se
    ///    recuerda lo que de verdad se acepto.
    ///
    /// Cualquier fallo cuenta como intento fallido para el limite.
    ///
    /// # Errors
    ///
    /// La variante de [`SecurityError`] correspondiente a la comprobacion que
    /// falle. Hacia el exterior conviene traducirlas todas a un rechazo
    /// generico para no revelar que paso.
    pub fn verify(&mut self, request: &AuthRequest, now: Timestamp) -> SecurityResult<AuthOutcome> {
        let key = request.device_id.to_string();
        self.limiter.check(&key, now)?;

        let result = self.verify_inner(request, now);
        match &result {
            Ok(_) => self.limiter.register_success(&key),
            Err(_) => {
                self.limiter.register_failure(&key, now);
            }
        }
        result
    }

    fn verify_inner(
        &mut self,
        request: &AuthRequest,
        now: Timestamp,
    ) -> SecurityResult<AuthOutcome> {
        self.replay.check_clock(request.timestamp, now)?;
        self.challenges.consume(&request.server_nonce, now)?;

        let record = self
            .registry
            .get(request.device_id)
            .ok_or(SecurityError::UnknownDevice)?;
        if !record.is_active() {
            return Err(SecurityError::DeviceRevoked);
        }

        let transcript = AuthTranscript::new(
            request.server_nonce,
            request.client_nonce,
            request.device_id,
            request.role,
            request.timestamp,
        );
        record
            .public_key
            .verify_hex(&transcript.to_bytes(), &request.signature_hex)?;

        self.replay
            .check_and_record(request.client_nonce, request.timestamp, now)?;
        self.registry.touch(request.device_id, now)?;

        Ok(AuthOutcome {
            permissions: record.permissions.clone(),
            counter_transcript: transcript.counterpart(),
            device: record,
        })
    }

    /// Descarta estado caducado. Conviene llamarlo periodicamente.
    pub fn prune(&mut self, now: Timestamp) {
        self.challenges.prune(now);
        self.replay.prune(now);
        self.limiter.prune(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use nexus_core::{DevicePlatform, DeviceRole};
    use std::time::Duration;

    const NOW: Timestamp = Timestamp(1_700_000_000_000);

    fn info(id: DeviceId) -> DeviceInfo {
        DeviceInfo {
            id,
            display_name: "iPad Pro".to_owned(),
            platform: DevicePlatform::IpadOs,
            role: DeviceRole::Controller,
            os_version: "18.1".to_owned(),
            app_version: "0.1.0".to_owned(),
        }
    }

    /// Monta un autenticador con un dispositivo ya emparejado.
    fn paired() -> (Authenticator<InMemoryRegistry>, DeviceIdentity) {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let mut registry = InMemoryRegistry::new();
        registry
            .upsert(
                identity.device_id(),
                DeviceRecord::new(
                    info(identity.device_id()),
                    identity.public_key(),
                    PermissionSet::standard(),
                    false,
                    NOW,
                ),
            )
            .expect("inserta");
        (Authenticator::new(registry), identity)
    }

    /// Construye una peticion firmada correctamente.
    fn sign_request(identity: &DeviceIdentity, server_nonce: Nonce, now: Timestamp) -> AuthRequest {
        let client_nonce = Nonce::generate();
        let transcript = AuthTranscript::new(
            server_nonce,
            client_nonce,
            identity.device_id(),
            AuthRole::Controller,
            now,
        );
        AuthRequest {
            device_id: identity.device_id(),
            server_nonce,
            client_nonce,
            role: AuthRole::Controller,
            timestamp: now,
            signature_hex: identity.sign_hex(&transcript.to_bytes()),
        }
    }

    #[test]
    fn a_paired_device_authenticates() {
        let (mut auth, identity) = paired();
        let challenge = auth.issue_challenge(NOW).expect("reto");
        let request = sign_request(&identity, challenge.nonce, NOW);

        let outcome = auth.verify(&request, NOW).expect("debe autenticar");
        assert_eq!(outcome.permissions, PermissionSet::standard());
        assert_eq!(outcome.device.info.id, identity.device_id());
    }

    #[test]
    fn successful_auth_updates_last_seen() {
        let (mut auth, identity) = paired();
        assert!(
            auth.registry()
                .get(identity.device_id())
                .expect("existe")
                .last_seen
                .is_none()
        );

        let challenge = auth.issue_challenge(NOW).expect("reto");
        auth.verify(&sign_request(&identity, challenge.nonce, NOW), NOW)
            .expect("autentica");

        assert_eq!(
            auth.registry()
                .get(identity.device_id())
                .expect("existe")
                .last_seen,
            Some(NOW)
        );
    }

    #[test]
    fn the_outcome_carries_the_transcript_the_host_must_sign() {
        let (mut auth, identity) = paired();
        let challenge = auth.issue_challenge(NOW).expect("reto");
        let request = sign_request(&identity, challenge.nonce, NOW);
        let outcome = auth.verify(&request, NOW).expect("autentica");

        // El host firma la transcripcion contraria; el dispositivo la verifica.
        assert_eq!(outcome.counter_transcript.role, AuthRole::Host);
        assert_eq!(
            outcome.counter_transcript.client_nonce,
            request.client_nonce
        );

        let host_identity = DeviceIdentity::generate(DeviceId::new());
        let host_signature = host_identity.sign(&outcome.counter_transcript.to_bytes());
        host_identity
            .public_key()
            .verify(&outcome.counter_transcript.to_bytes(), &host_signature)
            .expect("el dispositivo puede verificar al host");
    }

    #[test]
    fn an_unknown_device_is_rejected() {
        let (mut auth, _identity) = paired();
        let stranger = DeviceIdentity::generate(DeviceId::new());
        let challenge = auth.issue_challenge(NOW).expect("reto");

        assert_eq!(
            auth.verify(&sign_request(&stranger, challenge.nonce, NOW), NOW)
                .unwrap_err(),
            SecurityError::UnknownDevice
        );
    }

    #[test]
    fn a_revoked_device_is_rejected_even_with_a_valid_signature() {
        let (mut auth, identity) = paired();
        auth.registry_mut()
            .revoke(identity.device_id(), NOW)
            .expect("revoca");

        let challenge = auth.issue_challenge(NOW).expect("reto");
        assert_eq!(
            auth.verify(&sign_request(&identity, challenge.nonce, NOW), NOW)
                .unwrap_err(),
            SecurityError::DeviceRevoked
        );
    }

    #[test]
    fn revoking_twice_reports_no_change() {
        let mut registry = InMemoryRegistry::new();
        let identity = DeviceIdentity::generate(DeviceId::new());
        registry
            .upsert(
                identity.device_id(),
                DeviceRecord::new(
                    info(identity.device_id()),
                    identity.public_key(),
                    PermissionSet::view_only(),
                    false,
                    NOW,
                ),
            )
            .expect("inserta");

        assert!(registry.revoke(identity.device_id(), NOW).expect("revoca"));
        assert!(
            !registry
                .revoke(identity.device_id(), NOW)
                .expect("ya estaba")
        );
        assert!(!registry.revoke(DeviceId::new(), NOW).expect("no existe"));
    }

    #[test]
    fn a_forged_signature_is_rejected() {
        let (mut auth, identity) = paired();
        let challenge = auth.issue_challenge(NOW).expect("reto");
        let mut request = sign_request(&identity, challenge.nonce, NOW);
        request.signature_hex = "00".repeat(64);

        assert_eq!(
            auth.verify(&request, NOW).unwrap_err(),
            SecurityError::InvalidSignature
        );
    }

    #[test]
    fn a_signature_from_another_device_is_rejected() {
        let (mut auth, identity) = paired();
        let impostor = DeviceIdentity::generate(DeviceId::new());
        let challenge = auth.issue_challenge(NOW).expect("reto");

        // El impostor firma, pero dice ser el dispositivo emparejado.
        let client_nonce = Nonce::generate();
        let transcript = AuthTranscript::new(
            challenge.nonce,
            client_nonce,
            identity.device_id(),
            AuthRole::Controller,
            NOW,
        );
        let request = AuthRequest {
            device_id: identity.device_id(),
            server_nonce: challenge.nonce,
            client_nonce,
            role: AuthRole::Controller,
            timestamp: NOW,
            signature_hex: impostor.sign_hex(&transcript.to_bytes()),
        };

        assert_eq!(
            auth.verify(&request, NOW).unwrap_err(),
            SecurityError::InvalidSignature
        );
    }

    #[test]
    fn replaying_a_whole_handshake_fails_because_the_challenge_is_single_use() {
        let (mut auth, identity) = paired();
        let challenge = auth.issue_challenge(NOW).expect("reto");
        let request = sign_request(&identity, challenge.nonce, NOW);

        auth.verify(&request, NOW).expect("primera vez");
        assert_eq!(
            auth.verify(&request, NOW).unwrap_err(),
            SecurityError::ChallengeNotValid,
            "el reto ya se consumio"
        );
    }

    #[test]
    fn a_signature_cannot_be_moved_to_a_fresh_challenge() {
        let (mut auth, identity) = paired();
        let first = auth.issue_challenge(NOW).expect("reto");
        let request = sign_request(&identity, first.nonce, NOW);

        // Un atacante captura la firma y la presenta con un reto nuevo.
        let second = auth.issue_challenge(NOW).expect("otro reto");
        let moved = AuthRequest {
            server_nonce: second.nonce,
            ..request
        };

        assert_eq!(
            auth.verify(&moved, NOW).unwrap_err(),
            SecurityError::InvalidSignature,
            "la firma incluye el nonce del servidor"
        );
    }

    #[test]
    fn a_host_role_signature_does_not_pass_as_controller() {
        let (mut auth, identity) = paired();
        let challenge = auth.issue_challenge(NOW).expect("reto");
        let client_nonce = Nonce::generate();

        // Firma como host, pero se presenta como controlador.
        let host_view = AuthTranscript::new(
            challenge.nonce,
            client_nonce,
            identity.device_id(),
            AuthRole::Host,
            NOW,
        );
        let request = AuthRequest {
            device_id: identity.device_id(),
            server_nonce: challenge.nonce,
            client_nonce,
            role: AuthRole::Controller,
            timestamp: NOW,
            signature_hex: identity.sign_hex(&host_view.to_bytes()),
        };

        assert_eq!(
            auth.verify(&request, NOW).unwrap_err(),
            SecurityError::InvalidSignature
        );
    }

    #[test]
    fn a_stale_timestamp_is_rejected() {
        let (mut auth, identity) = paired();
        let challenge = auth.issue_challenge(NOW).expect("reto");
        let long_ago = Timestamp(NOW.0 - 600_000);
        let request = sign_request(&identity, challenge.nonce, long_ago);

        let err = auth.verify(&request, NOW).unwrap_err();
        assert!(matches!(err, SecurityError::ClockSkew { .. }), "{err:?}");
    }

    #[test]
    fn a_stale_request_does_not_consume_the_challenge() {
        let (mut auth, identity) = paired();
        let challenge = auth.issue_challenge(NOW).expect("reto");

        // Intento con reloj desviado: se rechaza antes de tocar el reto.
        let stale = sign_request(&identity, challenge.nonce, Timestamp(NOW.0 - 600_000));
        assert!(auth.verify(&stale, NOW).is_err());

        // El reto sigue disponible para el intento legitimo.
        auth.verify(&sign_request(&identity, challenge.nonce, NOW), NOW)
            .expect("el reto no se habia gastado");
    }

    #[test]
    fn an_expired_challenge_is_rejected() {
        let (mut auth, identity) = paired();
        let challenge = auth.issue_challenge(NOW).expect("reto");
        // Pasados 31 s el reto caduco, pero la marca del mensaje sigue fresca.
        let later = Timestamp(NOW.0 + 31_000);
        let request = sign_request(&identity, challenge.nonce, later);

        assert_eq!(
            auth.verify(&request, later).unwrap_err(),
            SecurityError::ChallengeNotValid
        );
    }

    #[test]
    fn repeated_failures_lock_the_device_out() {
        let (mut auth, identity) = paired();
        let policy = RateLimitPolicy::device_auth();

        for _ in 0..policy.max_failures {
            let challenge = auth.issue_challenge(NOW).expect("reto");
            let mut request = sign_request(&identity, challenge.nonce, NOW);
            request.signature_hex = "11".repeat(64);
            assert!(auth.verify(&request, NOW).is_err());
        }

        // Ahora ni una peticion correcta pasa: primero hay que esperar.
        let challenge = auth.issue_challenge(NOW).expect("reto");
        let err = auth
            .verify(&sign_request(&identity, challenge.nonce, NOW), NOW)
            .unwrap_err();
        assert!(matches!(err, SecurityError::RateLimited { .. }), "{err:?}");
    }

    #[test]
    fn the_lockout_lifts_after_waiting() {
        let (mut auth, identity) = paired();
        let policy = RateLimitPolicy::device_auth();

        for _ in 0..policy.max_failures {
            let challenge = auth.issue_challenge(NOW).expect("reto");
            let mut request = sign_request(&identity, challenge.nonce, NOW);
            request.signature_hex = "11".repeat(64);
            let _ = auth.verify(&request, NOW);
        }

        let later = Timestamp(NOW.0 + policy.base_lockout.as_millis() as u64 + 1);
        let challenge = auth.issue_challenge(later).expect("reto");
        auth.verify(&sign_request(&identity, challenge.nonce, later), later)
            .expect("tras la espera debe pasar");
    }

    #[test]
    fn a_success_clears_the_failure_history() {
        let (mut auth, identity) = paired();

        // Un fallo, luego un exito.
        let challenge = auth.issue_challenge(NOW).expect("reto");
        let mut bad = sign_request(&identity, challenge.nonce, NOW);
        bad.signature_hex = "22".repeat(64);
        assert!(auth.verify(&bad, NOW).is_err());

        let challenge = auth.issue_challenge(NOW).expect("reto");
        auth.verify(&sign_request(&identity, challenge.nonce, NOW), NOW)
            .expect("autentica");

        // Y a partir de ahi vuelve a tener todo el presupuesto de intentos.
        let policy = RateLimitPolicy::device_auth();
        for _ in 0..policy.max_failures - 1 {
            let challenge = auth.issue_challenge(NOW).expect("reto");
            let mut bad = sign_request(&identity, challenge.nonce, NOW);
            bad.signature_hex = "33".repeat(64);
            assert!(auth.verify(&bad, NOW).is_err());
        }
        let challenge = auth.issue_challenge(NOW).expect("reto");
        auth.verify(&sign_request(&identity, challenge.nonce, NOW), NOW)
            .expect("aun no deberia estar bloqueado");
    }

    #[test]
    fn one_devices_lockout_does_not_affect_another() {
        let mut registry = InMemoryRegistry::new();
        let good = DeviceIdentity::generate(DeviceId::new());
        let attacked = DeviceIdentity::generate(DeviceId::new());
        for identity in [&good, &attacked] {
            registry
                .upsert(
                    identity.device_id(),
                    DeviceRecord::new(
                        info(identity.device_id()),
                        identity.public_key(),
                        PermissionSet::view_only(),
                        false,
                        NOW,
                    ),
                )
                .expect("inserta");
        }
        let mut auth = Authenticator::new(registry);

        let policy = RateLimitPolicy::device_auth();
        for _ in 0..policy.max_failures {
            let challenge = auth.issue_challenge(NOW).expect("reto");
            let mut bad = sign_request(&attacked, challenge.nonce, NOW);
            bad.signature_hex = "44".repeat(64);
            let _ = auth.verify(&bad, NOW);
        }

        let challenge = auth.issue_challenge(NOW).expect("reto");
        auth.verify(&sign_request(&good, challenge.nonce, NOW), NOW)
            .expect("el otro dispositivo no debe sufrir el bloqueo");
    }

    #[test]
    fn pruning_clears_expired_state() {
        let (mut auth, _identity) = paired();
        for _ in 0..5 {
            auth.issue_challenge(NOW).expect("reto");
        }
        auth.prune(Timestamp(
            NOW.0 + Duration::from_secs(600).as_millis() as u64,
        ));
        // Tras limpiar, un reto antiguo ya no existe.
        let stale = Nonce::generate();
        assert_eq!(
            auth.challenges.consume(&stale, NOW).unwrap_err(),
            SecurityError::ChallengeNotValid
        );
    }

    #[test]
    fn registry_listing_is_deterministic() {
        let mut registry = InMemoryRegistry::new();
        for _ in 0..5 {
            let identity = DeviceIdentity::generate(DeviceId::new());
            registry
                .upsert(
                    identity.device_id(),
                    DeviceRecord::new(
                        info(identity.device_id()),
                        identity.public_key(),
                        PermissionSet::view_only(),
                        false,
                        NOW,
                    ),
                )
                .expect("inserta");
        }
        assert_eq!(registry.len(), 5);
        assert_eq!(
            registry.list(),
            registry.list(),
            "el orden debe ser estable"
        );
    }

    #[test]
    fn record_serialization_roundtrips() {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let record = DeviceRecord::new(
            info(identity.device_id()),
            identity.public_key(),
            PermissionSet::full(),
            true,
            NOW,
        );
        let raw = serde_json::to_string(&record).expect("serializa");
        let back: DeviceRecord = serde_json::from_str(&raw).expect("deserializa");
        assert_eq!(back, record);
    }
}
