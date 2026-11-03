//! Autenticacion mutua por reto y respuesta.
//!
//! Ninguna de las dos partes revela su clave privada y ninguna se fia de la
//! palabra de la otra: cada extremo firma una **transcripcion** que incluye los
//! dos nonces, y verifica la firma del otro sobre la misma transcripcion.
//!
//! ## Por que la transcripcion lleva lo que lleva
//!
//! - **Prefijo de dominio**: una firma hecha para NEXUS no vale en otro
//!   protocolo que use la misma clave, ni al contrario.
//! - **Version de protocolo**: impide que alguien reutilice una firma de una
//!   version futura con reglas distintas.
//! - **Papel del firmante** (host o controlador): sin este byte, un atacante
//!   podria devolver al host su propia firma y hacerse pasar por el otro
//!   extremo. Es el ataque de reflexion clasico.
//! - **Los dos nonces**: el del servidor garantiza frescura para quien reta; el
//!   del cliente evita que el servidor elija toda la entrada que se firma.
//! - **Identificador de dispositivo**: ata la firma a un dispositivo concreto,
//!   de modo que no se puede reutilizar para otro.
//! - **Marca temporal**: la usa [`crate::replay`] para acotar la ventana.
//!
//! Los campos se concatenan con longitudes fijas, asi que no hay ambiguedad
//! posible entre dos transcripciones distintas.

use std::collections::HashMap;
use std::time::Duration;

use nexus_core::{DeviceId, Timestamp};
use rand::rngs::SysRng;
use rand_core::TryRng;
use serde::{Deserialize, Serialize};

use crate::error::{SecurityError, SecurityResult};

/// Prefijo de separacion de dominio de las firmas de autenticacion.
pub const AUTH_DOMAIN: &[u8] = b"NEXUS-REMOTE/v1/mutual-auth";

/// Longitud en bytes de un nonce de autenticacion.
pub const NONCE_LEN: usize = 32;

/// Validez por omision de un reto.
pub const DEFAULT_CHALLENGE_TTL: Duration = Duration::from_secs(30);

/// Valor aleatorio de un solo uso.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Nonce([u8; NONCE_LEN]);

impl Nonce {
    /// Genera un nonce con el generador del sistema operativo.
    ///
    /// Se pide la entropia directamente al sistema (`SysRng`) en lugar de a un
    /// generador de usuario: un nonce predecible rompe la frescura del reto. Si
    /// el sistema no puede darla, es preferible entrar en panico que emitir un
    /// reto adivinable.
    ///
    /// # Panics
    ///
    /// Si el generador del sistema operativo falla.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; NONCE_LEN];
        SysRng
            .try_fill_bytes(&mut bytes)
            .expect("el generador del sistema operativo debe estar disponible");
        Self(bytes)
    }

    /// Construye un nonce a partir de bytes conocidos. Pensado para pruebas.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; NONCE_LEN]) -> Self {
        Self(bytes)
    }

    /// Bytes del nonce.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; NONCE_LEN] {
        &self.0
    }

    /// Representacion hexadecimal, que es la que viaja por el protocolo.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Decodifica un nonce recibido en hexadecimal.
    ///
    /// # Errors
    ///
    /// [`SecurityError::InvalidKeyMaterial`] si el texto no es hexadecimal de
    /// la longitud exacta.
    pub fn from_hex(value: &str) -> SecurityResult<Self> {
        let raw = hex::decode(value)
            .map_err(|e| SecurityError::InvalidKeyMaterial(format!("nonce no hexadecimal: {e}")))?;
        let bytes: [u8; NONCE_LEN] = raw.try_into().map_err(|v: Vec<u8>| {
            SecurityError::InvalidKeyMaterial(format!(
                "se esperaban {NONCE_LEN} bytes de nonce, llegaron {}",
                v.len()
            ))
        })?;
        Ok(Self(bytes))
    }
}

impl std::fmt::Debug for Nonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Nonce({}...)", &self.to_hex()[..8])
    }
}

impl TryFrom<String> for Nonce {
    type Error = SecurityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_hex(&value)
    }
}

impl From<Nonce> for String {
    fn from(value: Nonce) -> Self {
        value.to_hex()
    }
}

/// Papel del firmante dentro del handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthRole {
    /// Firma el PC que comparte pantalla.
    Host,
    /// Firma el iPhone o iPad que controla.
    Controller,
}

impl AuthRole {
    /// Byte que representa el papel dentro de la transcripcion.
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Self::Host => 0x01,
            Self::Controller => 0x02,
        }
    }

    /// El papel contrario.
    #[must_use]
    pub const fn counterpart(self) -> Self {
        match self {
            Self::Host => Self::Controller,
            Self::Controller => Self::Host,
        }
    }
}

/// Datos que ambos extremos firman.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthTranscript {
    /// Nonce emitido por el servidor de senalizacion.
    pub server_nonce: Nonce,
    /// Nonce aportado por el dispositivo.
    pub client_nonce: Nonce,
    /// Dispositivo que firma.
    pub device_id: DeviceId,
    /// Papel con el que firma.
    pub role: AuthRole,
    /// Marca temporal del firmante.
    pub timestamp: Timestamp,
    /// Version del protocolo en uso.
    pub protocol_version: u16,
}

impl AuthTranscript {
    /// Longitud exacta en bytes de la transcripcion serializada.
    pub const LEN: usize = AUTH_DOMAIN.len() + 1 + 1 + 2 + NONCE_LEN + NONCE_LEN + 16 + 8;

    /// Construye la transcripcion con la version de protocolo actual.
    #[must_use]
    pub fn new(
        server_nonce: Nonce,
        client_nonce: Nonce,
        device_id: DeviceId,
        role: AuthRole,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            server_nonce,
            client_nonce,
            device_id,
            role,
            timestamp,
            protocol_version: nexus_core::PROTOCOL_VERSION,
        }
    }

    /// Transcripcion que deberia haber firmado el otro extremo.
    ///
    /// Solo cambia el papel: todo lo demas es compartido. Es lo que se verifica
    /// contra la firma recibida.
    #[must_use]
    pub fn counterpart(&self) -> Self {
        Self {
            role: self.role.counterpart(),
            ..self.clone()
        }
    }

    /// Serializa la transcripcion a los bytes que se firman.
    ///
    /// Todos los campos tienen longitud fija, asi que dos transcripciones
    /// distintas nunca producen los mismos bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::LEN);
        out.extend_from_slice(AUTH_DOMAIN);
        out.push(0x00); // separador entre dominio y campos
        out.push(self.role.tag());
        out.extend_from_slice(&self.protocol_version.to_le_bytes());
        out.extend_from_slice(self.server_nonce.as_bytes());
        out.extend_from_slice(self.client_nonce.as_bytes());
        out.extend_from_slice(self.device_id.as_uuid().as_bytes());
        out.extend_from_slice(&self.timestamp.0.to_le_bytes());
        debug_assert_eq!(out.len(), Self::LEN);
        out
    }
}

/// Reto emitido por el servidor, valido una sola vez y por tiempo limitado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    /// Nonce a firmar.
    pub nonce: Nonce,
    /// Instante de emision.
    pub issued_at: Timestamp,
    /// Instante a partir del cual deja de valer.
    pub expires_at: Timestamp,
}

impl Challenge {
    /// Indica si el reto sigue vigente.
    #[must_use]
    pub const fn is_valid_at(&self, now: Timestamp) -> bool {
        now.0 < self.expires_at.0
    }
}

/// Retos pendientes de respuesta.
///
/// Cada reto se consume como maximo una vez: cuando se usa, se elimina. Eso
/// hace que una respuesta capturada no sirva una segunda vez aunque el reloj no
/// haya avanzado.
#[derive(Debug)]
pub struct ChallengeStore {
    pending: HashMap<Nonce, Challenge>,
    ttl: Duration,
    max_pending: usize,
}

impl ChallengeStore {
    /// Numero maximo de retos pendientes por omision.
    ///
    /// Acota la memoria que un atacante puede hacer reservar pidiendo retos sin
    /// contestarlos.
    pub const DEFAULT_MAX_PENDING: usize = 1_024;

    /// Crea un almacen con la validez indicada.
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            pending: HashMap::new(),
            ttl,
            max_pending: Self::DEFAULT_MAX_PENDING,
        }
    }

    /// Crea un almacen indicando tambien el tope de retos pendientes.
    #[must_use]
    pub fn with_capacity(ttl: Duration, max_pending: usize) -> Self {
        Self {
            pending: HashMap::new(),
            ttl,
            max_pending,
        }
    }

    /// Emite un reto nuevo.
    ///
    /// Antes de insertarlo descarta los caducados; si aun asi se alcanza el
    /// tope, rechaza la peticion en lugar de crecer sin limite.
    ///
    /// # Errors
    ///
    /// [`SecurityError::RateLimited`] si hay demasiados retos sin contestar.
    pub fn issue(&mut self, now: Timestamp) -> SecurityResult<Challenge> {
        self.prune(now);
        if self.pending.len() >= self.max_pending {
            return Err(SecurityError::RateLimited {
                retry_after_secs: self.ttl.as_secs().max(1),
            });
        }

        let challenge = Challenge {
            nonce: Nonce::generate(),
            issued_at: now,
            expires_at: now.saturating_add(self.ttl),
        };
        self.pending.insert(challenge.nonce, challenge.clone());
        Ok(challenge)
    }

    /// Consume un reto. Solo tiene exito la primera vez y antes de caducar.
    ///
    /// # Errors
    ///
    /// [`SecurityError::ChallengeNotValid`] si el nonce no existe, ya se
    /// consumio o el reto expiro.
    pub fn consume(&mut self, nonce: &Nonce, now: Timestamp) -> SecurityResult<Challenge> {
        let challenge = self
            .pending
            .remove(nonce)
            .ok_or(SecurityError::ChallengeNotValid)?;
        if challenge.is_valid_at(now) {
            Ok(challenge)
        } else {
            Err(SecurityError::ChallengeNotValid)
        }
    }

    /// Elimina los retos caducados.
    pub fn prune(&mut self, now: Timestamp) {
        self.pending.retain(|_, c| c.is_valid_at(now));
    }

    /// Numero de retos pendientes.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

impl Default for ChallengeStore {
    fn default() -> Self {
        Self::new(DEFAULT_CHALLENGE_TTL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;

    fn transcript(role: AuthRole) -> AuthTranscript {
        AuthTranscript::new(
            Nonce::from_bytes([1_u8; NONCE_LEN]),
            Nonce::from_bytes([2_u8; NONCE_LEN]),
            DeviceId::from_uuid(uuid_from_byte(7)),
            role,
            Timestamp(1_700_000_000_000),
        )
    }

    fn uuid_from_byte(b: u8) -> uuid::Uuid {
        uuid::Uuid::from_bytes([b; 16])
    }

    #[test]
    fn nonces_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            assert!(seen.insert(Nonce::generate()), "nonce repetido");
        }
    }

    #[test]
    fn nonce_roundtrips_through_hex() {
        let nonce = Nonce::generate();
        assert_eq!(nonce.to_hex().len(), NONCE_LEN * 2);
        assert_eq!(Nonce::from_hex(&nonce.to_hex()).expect("parsea"), nonce);
    }

    #[test]
    fn malformed_nonces_are_rejected() {
        for bad in ["", "zz", &"ab".repeat(31), &"ab".repeat(33)] {
            assert!(Nonce::from_hex(bad).is_err(), "{bad:?} no deberia valer");
        }
    }

    #[test]
    fn transcript_has_the_declared_length() {
        assert_eq!(
            transcript(AuthRole::Host).to_bytes().len(),
            AuthTranscript::LEN
        );
    }

    #[test]
    fn transcript_starts_with_the_domain_prefix() {
        let bytes = transcript(AuthRole::Controller).to_bytes();
        assert!(
            bytes.starts_with(AUTH_DOMAIN),
            "falta la separacion de dominio"
        );
    }

    #[test]
    fn role_changes_the_transcript() {
        // Si no cambiara, una firma del host valdria como firma del controlador:
        // es la defensa contra el ataque de reflexion.
        assert_ne!(
            transcript(AuthRole::Host).to_bytes(),
            transcript(AuthRole::Controller).to_bytes()
        );
    }

    #[test]
    fn counterpart_only_flips_the_role() {
        let host = transcript(AuthRole::Host);
        let controller = host.counterpart();
        assert_eq!(controller.role, AuthRole::Controller);
        assert_eq!(controller.server_nonce, host.server_nonce);
        assert_eq!(controller.client_nonce, host.client_nonce);
        assert_eq!(controller.device_id, host.device_id);
        assert_eq!(controller.timestamp, host.timestamp);
        assert_eq!(controller.counterpart().to_bytes(), host.to_bytes());
    }

    #[test]
    fn every_field_changes_the_transcript() {
        let base = transcript(AuthRole::Host);
        let baseline = base.to_bytes();

        let variants = [
            AuthTranscript {
                server_nonce: Nonce::from_bytes([9_u8; NONCE_LEN]),
                ..base.clone()
            },
            AuthTranscript {
                client_nonce: Nonce::from_bytes([9_u8; NONCE_LEN]),
                ..base.clone()
            },
            AuthTranscript {
                device_id: DeviceId::from_uuid(uuid_from_byte(8)),
                ..base.clone()
            },
            AuthTranscript {
                timestamp: Timestamp(1_700_000_000_001),
                ..base.clone()
            },
            AuthTranscript {
                protocol_version: 2,
                ..base.clone()
            },
        ];
        for (i, variant) in variants.iter().enumerate() {
            assert_ne!(
                variant.to_bytes(),
                baseline,
                "la variante {i} no cambio los bytes"
            );
        }
    }

    #[test]
    fn swapping_the_two_nonces_changes_the_transcript() {
        let base = transcript(AuthRole::Host);
        let swapped = AuthTranscript {
            server_nonce: base.client_nonce,
            client_nonce: base.server_nonce,
            ..base.clone()
        };
        assert_ne!(swapped.to_bytes(), base.to_bytes());
    }

    #[test]
    fn a_host_signature_does_not_validate_as_a_controller_signature() {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let host_view = transcript(AuthRole::Host);
        let signature = identity.sign(&host_view.to_bytes());

        // La firma vale para su propia transcripcion...
        identity
            .public_key()
            .verify(&host_view.to_bytes(), &signature)
            .expect("vale");

        // ...y no vale para la del papel contrario.
        assert_eq!(
            identity
                .public_key()
                .verify(&host_view.counterpart().to_bytes(), &signature)
                .unwrap_err(),
            SecurityError::InvalidSignature
        );
    }

    #[test]
    fn challenge_can_be_consumed_once() {
        let mut store = ChallengeStore::new(Duration::from_secs(30));
        let now = Timestamp(1_000);
        let challenge = store.issue(now).expect("emite");
        assert_eq!(store.pending_count(), 1);

        store
            .consume(&challenge.nonce, now)
            .expect("primera vez vale");
        assert_eq!(store.pending_count(), 0);

        assert_eq!(
            store.consume(&challenge.nonce, now).unwrap_err(),
            SecurityError::ChallengeNotValid,
            "un reto consumido no se puede reutilizar"
        );
    }

    #[test]
    fn expired_challenge_is_refused() {
        let mut store = ChallengeStore::new(Duration::from_secs(30));
        let challenge = store.issue(Timestamp(1_000)).expect("emite");
        let too_late = Timestamp(1_000 + 30_001);
        assert_eq!(
            store.consume(&challenge.nonce, too_late).unwrap_err(),
            SecurityError::ChallengeNotValid
        );
    }

    #[test]
    fn challenge_valid_until_the_last_millisecond() {
        let mut store = ChallengeStore::new(Duration::from_secs(10));
        let challenge = store.issue(Timestamp(0)).expect("emite");
        assert!(store.consume(&challenge.nonce, Timestamp(9_999)).is_ok());
    }

    #[test]
    fn unknown_nonce_is_refused() {
        let mut store = ChallengeStore::default();
        let err = store.consume(&Nonce::generate(), Timestamp(1)).unwrap_err();
        assert_eq!(err, SecurityError::ChallengeNotValid);
    }

    #[test]
    fn pruning_drops_expired_challenges() {
        let mut store = ChallengeStore::new(Duration::from_secs(5));
        for _ in 0..10 {
            store.issue(Timestamp(0)).expect("emite");
        }
        assert_eq!(store.pending_count(), 10);
        store.prune(Timestamp(6_000));
        assert_eq!(store.pending_count(), 0);
    }

    #[test]
    fn pending_challenges_are_capped() {
        let mut store = ChallengeStore::with_capacity(Duration::from_secs(60), 3);
        let now = Timestamp(0);
        for _ in 0..3 {
            store.issue(now).expect("cabe");
        }
        let err = store.issue(now).unwrap_err();
        assert!(matches!(err, SecurityError::RateLimited { .. }));

        // Cuando caducan, se puede volver a emitir.
        let later = Timestamp(61_000);
        store.issue(later).expect("tras caducar hay sitio");
    }

    #[test]
    fn nonce_debug_does_not_print_the_whole_value() {
        let nonce = Nonce::generate();
        let rendered = format!("{nonce:?}");
        assert!(!rendered.contains(&nonce.to_hex()));
    }
}
