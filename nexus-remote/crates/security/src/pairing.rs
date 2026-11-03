//! Emparejamiento inicial de un dispositivo.
//!
//! El host muestra un codigo temporal (o un QR que lo contiene) y el iPhone o
//! iPad lo canjea una sola vez. A partir de ahi el dispositivo queda en la
//! lista de autorizados con su propia clave publica y ya no necesita el codigo:
//! las conexiones posteriores se autentican por firma.
//!
//! ## Por que el codigo se guarda derivado y no en claro
//!
//! El codigo es un secreto de poca entropia (40 bits) y vive en la base de
//! datos del host mientras el emparejamiento esta abierto. Se guarda derivado
//! con Argon2id, de modo que leer el archivo no basta para obtenerlo. La
//! comparacion la hace la propia biblioteca en tiempo constante.
//!
//! ## Por que 40 bits bastan
//!
//! Un codigo de 8 caracteres sobre un alfabeto de 32 son 2^40 combinaciones,
//! pero lo que de verdad lo protege es que solo vive unos minutos y admite
//! cinco intentos ([`crate::ratelimit::RateLimitPolicy::pairing`]). Con ese
//! presupuesto, adivinarlo es inviable.

use std::fmt;
use std::time::Duration;

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use nexus_core::{DeviceId, PairingId, PermissionSet, Timestamp};
use rand::rngs::SysRng;
use rand_core::TryRng;
use serde::{Deserialize, Serialize};

use crate::error::{SecurityError, SecurityResult};
use crate::identity::DevicePublicKey;

/// Alfabeto del codigo, sin caracteres que se confundan al leerlos.
///
/// Se excluyen `I`, `O`, `0` y `1`. Son 32 simbolos, una potencia de dos, de
/// modo que tomar cinco bits de cada byte aleatorio da una distribucion
/// uniforme sin necesidad de descartar valores.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Numero de caracteres del codigo.
pub const CODE_LEN: usize = 8;

/// Validez por omision de un codigo de emparejamiento.
pub const DEFAULT_PAIRING_TTL: Duration = Duration::from_secs(300);

/// Intentos de canje permitidos antes de invalidar el codigo.
pub const MAX_PAIRING_ATTEMPTS: u32 = 5;

/// Codigo de emparejamiento en claro.
///
/// Solo existe en memoria del host mientras se muestra en pantalla. Su `Debug`
/// no lo imprime, para que no acabe en un registro por accidente.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingCode(String);

impl PairingCode {
    /// Genera un codigo nuevo con entropia del sistema operativo.
    ///
    /// # Panics
    ///
    /// Si el generador del sistema operativo falla.
    #[must_use]
    pub fn generate() -> Self {
        let mut raw = [0_u8; CODE_LEN];
        SysRng
            .try_fill_bytes(&mut raw)
            .expect("el generador del sistema operativo debe estar disponible");

        let code: String = raw
            .iter()
            .map(|b| char::from(CODE_ALPHABET[(*b & 0x1F) as usize]))
            .collect();
        Self(code)
    }

    /// Normaliza un codigo escrito por la persona.
    ///
    /// Acepta minusculas, espacios y guiones, porque es lo que se teclea de
    /// verdad desde un iPad. Rechaza cualquier otro caracter en lugar de
    /// ignorarlo.
    ///
    /// # Errors
    ///
    /// [`SecurityError::PairingCodeMismatch`] si, tras limpiar, el codigo no
    /// tiene la longitud exacta o contiene simbolos ajenos al alfabeto.
    pub fn parse(input: &str) -> SecurityResult<Self> {
        let cleaned: String = input
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .map(|c| c.to_ascii_uppercase())
            .collect();

        if cleaned.len() != CODE_LEN {
            return Err(SecurityError::PairingCodeMismatch);
        }
        if !cleaned.bytes().all(|b| CODE_ALPHABET.contains(&b)) {
            return Err(SecurityError::PairingCodeMismatch);
        }
        Ok(Self(cleaned))
    }

    /// Representacion canonica, sin separadores.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Representacion para mostrar en pantalla, agrupada en dos bloques.
    #[must_use]
    pub fn grouped(&self) -> String {
        format!("{}-{}", &self.0[..4], &self.0[4..])
    }
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingCode(oculto)")
    }
}

/// Estado de un emparejamiento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingState {
    /// Esperando a que el dispositivo canjee el codigo.
    Active,
    /// Canjeado con exito.
    Completed,
    /// Caducado sin canjear.
    Expired,
    /// Cancelado desde el host.
    Cancelled,
    /// Invalidado por exceso de intentos fallidos.
    Blocked,
}

impl PairingState {
    /// Indica si todavia admite intentos de canje.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Emparejamiento abierto por el host, tal como se guarda.
///
/// No contiene el codigo en claro: solo su derivacion Argon2id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingSession {
    /// Identificador del intento.
    pub id: PairingId,
    /// Host que lo abrio.
    pub host_id: DeviceId,
    /// Derivacion del codigo en formato PHC.
    code_phc: String,
    /// Instante de creacion.
    pub created_at: Timestamp,
    /// Instante de caducidad.
    pub expires_at: Timestamp,
    /// Intentos fallidos consumidos.
    pub failed_attempts: u32,
    /// Estado actual.
    pub state: PairingState,
    /// Permisos que se concederan al completar.
    pub permissions: PermissionSet,
    /// Si el dispositivo podra conectarse sin que nadie acepte en el host.
    pub unattended_access: bool,
}

impl PairingSession {
    /// Abre un emparejamiento y devuelve el codigo que hay que mostrar.
    ///
    /// El codigo en claro se devuelve **una sola vez**: no queda guardado en la
    /// sesion, asi que si se pierde hay que abrir otro emparejamiento.
    ///
    /// # Errors
    ///
    /// [`SecurityError::HashFailure`] si la derivacion Argon2id falla.
    pub fn open(
        host_id: DeviceId,
        permissions: PermissionSet,
        unattended_access: bool,
        ttl: Duration,
        now: Timestamp,
    ) -> SecurityResult<(Self, PairingCode)> {
        let id = PairingId::new();
        let code = PairingCode::generate();
        let code_phc = derive(id, &code)?;

        let session = Self {
            id,
            host_id,
            code_phc,
            created_at: now,
            expires_at: now.saturating_add(ttl),
            failed_attempts: 0,
            state: PairingState::Active,
            permissions,
            unattended_access,
        };
        Ok((session, code))
    }

    /// Intentos que quedan antes de invalidar el codigo.
    #[must_use]
    pub const fn remaining_attempts(&self) -> u32 {
        MAX_PAIRING_ATTEMPTS.saturating_sub(self.failed_attempts)
    }

    /// Comprueba si el emparejamiento sigue utilizable en este instante.
    #[must_use]
    pub const fn is_usable_at(&self, now: Timestamp) -> bool {
        self.state.is_open() && now.0 < self.expires_at.0
    }

    /// Canjea el codigo y, si es correcto, cierra el emparejamiento.
    ///
    /// Un intento fallido consume presupuesto; al agotarlo, el emparejamiento
    /// pasa a [`PairingState::Blocked`] y ya no sirve ni con el codigo correcto.
    ///
    /// # Errors
    ///
    /// - [`SecurityError::PairingNotActive`] si ya se completo, se cancelo o se bloqueo.
    /// - [`SecurityError::PairingCodeExpired`] si paso la ventana de validez.
    /// - [`SecurityError::PairingCodeMismatch`] si el codigo no coincide.
    /// - [`SecurityError::HashFailure`] si la verificacion Argon2id falla por un
    ///   motivo distinto a no coincidir.
    pub fn redeem(&mut self, candidate: &PairingCode, now: Timestamp) -> SecurityResult<()> {
        if !self.state.is_open() {
            return Err(SecurityError::PairingNotActive);
        }
        if now.0 >= self.expires_at.0 {
            self.state = PairingState::Expired;
            return Err(SecurityError::PairingCodeExpired);
        }

        match Argon2::default().verify_password(
            transcript(self.id, candidate).as_bytes(),
            self.code_phc.as_str(),
        ) {
            Ok(()) => {
                self.state = PairingState::Completed;
                Ok(())
            }
            Err(argon2::password_hash::Error::PasswordInvalid) => {
                self.failed_attempts += 1;
                if self.failed_attempts >= MAX_PAIRING_ATTEMPTS {
                    self.state = PairingState::Blocked;
                }
                Err(SecurityError::PairingCodeMismatch)
            }
            Err(other) => Err(SecurityError::HashFailure(other.to_string())),
        }
    }

    /// Cancela el emparejamiento desde el host.
    pub const fn cancel(&mut self) {
        if self.state.is_open() {
            self.state = PairingState::Cancelled;
        }
    }

    /// Marca como caducado si procede. Devuelve `true` si cambio el estado.
    pub const fn expire_if_due(&mut self, now: Timestamp) -> bool {
        if self.state.is_open() && now.0 >= self.expires_at.0 {
            self.state = PairingState::Expired;
            return true;
        }
        false
    }
}

/// Datos que el host codifica en el QR para ahorrar teclear.
///
/// Contiene el codigo, asi que el QR es tan sensible como el codigo: se muestra
/// en la pantalla del host y caduca con el.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingInvite {
    /// URL del servidor de senalizacion propio (`wss://...`).
    pub signaling_url: String,
    /// Host que abre el emparejamiento.
    pub host_id: DeviceId,
    /// Intento de emparejamiento.
    pub pairing_id: PairingId,
    /// Codigo en claro.
    pub code: String,
    /// Clave publica Ed25519 del host, en hexadecimal.
    ///
    /// Viaja en la invitacion, que llega al dispositivo fuera de banda (un QR
    /// en la pantalla del host). Es la unica forma de que el controlador
    /// obtenga la clave del host **sin preguntarsela al servidor**: si la
    /// pidiese al servidor, un servidor comprometido podria darle la suya y la
    /// autenticacion mutua posterior no demostraria nada.
    pub host_public_key_hex: String,
    /// Huella corta de la misma clave, para comparar a ojo.
    pub host_fingerprint: String,
    /// Caducidad.
    pub expires_at: Timestamp,
}

impl PairingInvite {
    /// Construye la invitacion a partir de un emparejamiento abierto.
    #[must_use]
    pub fn new(
        signaling_url: impl Into<String>,
        session: &PairingSession,
        code: &PairingCode,
        host_key: &DevicePublicKey,
    ) -> Self {
        Self {
            signaling_url: signaling_url.into(),
            host_id: session.host_id,
            pairing_id: session.id,
            code: code.as_str().to_owned(),
            host_public_key_hex: host_key.to_hex(),
            host_fingerprint: host_key.short_fingerprint(),
            expires_at: session.expires_at,
        }
    }

    /// Serializa la invitacion a JSON, que es lo que se pinta en el QR.
    ///
    /// # Errors
    ///
    /// [`SecurityError::HashFailure`] si la serializacion falla, lo que solo
    /// ocurriria por un error interno.
    pub fn to_json(&self) -> SecurityResult<String> {
        serde_json::to_string(self).map_err(|e| SecurityError::HashFailure(e.to_string()))
    }

    /// Reconstruye la invitacion leida de un QR.
    ///
    /// # Errors
    ///
    /// [`SecurityError::InvalidKeyMaterial`] si el contenido no es la
    /// invitacion esperada.
    pub fn from_json(raw: &str) -> SecurityResult<Self> {
        serde_json::from_str(raw)
            .map_err(|e| SecurityError::InvalidKeyMaterial(format!("invitacion ilegible: {e}")))
    }

    /// Codigo ya normalizado, listo para canjear.
    ///
    /// # Errors
    ///
    /// [`SecurityError::PairingCodeMismatch`] si el codigo del QR no tiene la
    /// forma esperada.
    pub fn parsed_code(&self) -> SecurityResult<PairingCode> {
        PairingCode::parse(&self.code)
    }
}

/// Texto que se deriva: ata el codigo a su emparejamiento concreto.
///
/// Sin el identificador, un codigo observado en un emparejamiento podria
/// probarse contra otro abierto a la vez.
fn transcript(id: PairingId, code: &PairingCode) -> String {
    format!("NEXUS-REMOTE/v1/pairing|{id}|{}", code.as_str())
}

fn derive(id: PairingId, code: &PairingCode) -> SecurityResult<String> {
    Argon2::default()
        .hash_password(transcript(id, code).as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|e| SecurityError::HashFailure(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;

    const NOW: Timestamp = Timestamp(1_700_000_000_000);

    fn open() -> (PairingSession, PairingCode) {
        PairingSession::open(
            DeviceId::new(),
            PermissionSet::standard(),
            false,
            DEFAULT_PAIRING_TTL,
            NOW,
        )
        .expect("abre emparejamiento")
    }

    #[test]
    fn generated_codes_have_the_expected_shape() {
        for _ in 0..32 {
            let code = PairingCode::generate();
            assert_eq!(code.as_str().len(), CODE_LEN);
            assert!(
                code.as_str().bytes().all(|b| CODE_ALPHABET.contains(&b)),
                "caracter fuera del alfabeto: {}",
                code.as_str()
            );
        }
    }

    #[test]
    fn codes_exclude_confusable_characters() {
        for _ in 0..64 {
            let code = PairingCode::generate();
            for bad in ['I', 'O', '0', '1'] {
                assert!(
                    !code.as_str().contains(bad),
                    "{} contiene {bad}",
                    code.as_str()
                );
            }
        }
    }

    #[test]
    fn generated_codes_are_not_repeated() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            seen.insert(PairingCode::generate().as_str().to_owned());
        }
        // Con 2^40 combinaciones, 200 extracciones no deberian colisionar.
        assert_eq!(seen.len(), 200);
    }

    #[test]
    fn grouped_form_is_readable() {
        let code = PairingCode::parse("ABCD2345").expect("valido");
        assert_eq!(code.grouped(), "ABCD-2345");
    }

    #[test]
    fn parsing_tolerates_how_people_actually_type() {
        let expected = PairingCode::parse("ABCD2345").expect("valido");
        for input in [
            "ABCD2345",
            "abcd2345",
            "ABCD-2345",
            " abcd 2345 ",
            "AB-CD-23-45",
        ] {
            assert_eq!(
                PairingCode::parse(input).expect(input),
                expected,
                "fallo con {input:?}"
            );
        }
    }

    #[test]
    fn parsing_rejects_wrong_length_and_bad_characters() {
        for bad in ["", "ABC", "ABCD23456", "ABCD234!", "ABCDI234", "ABCD0234"] {
            assert_eq!(
                PairingCode::parse(bad).unwrap_err(),
                SecurityError::PairingCodeMismatch,
                "{bad:?} no deberia aceptarse"
            );
        }
    }

    #[test]
    fn the_right_code_completes_the_pairing() {
        let (mut session, code) = open();
        assert!(session.is_usable_at(NOW));
        session
            .redeem(&code, NOW)
            .expect("el codigo correcto debe valer");
        assert_eq!(session.state, PairingState::Completed);
    }

    #[test]
    fn a_completed_pairing_cannot_be_reused() {
        let (mut session, code) = open();
        session.redeem(&code, NOW).expect("primera vez");
        assert_eq!(
            session.redeem(&code, NOW).unwrap_err(),
            SecurityError::PairingNotActive
        );
    }

    #[test]
    fn a_wrong_code_is_rejected_and_consumes_an_attempt() {
        let (mut session, _code) = open();
        let wrong = PairingCode::parse("ZZZZ9999").expect("valido");
        assert_eq!(
            session.redeem(&wrong, NOW).unwrap_err(),
            SecurityError::PairingCodeMismatch
        );
        assert_eq!(session.failed_attempts, 1);
        assert_eq!(session.remaining_attempts(), MAX_PAIRING_ATTEMPTS - 1);
        assert_eq!(session.state, PairingState::Active, "aun quedan intentos");
    }

    #[test]
    fn exhausting_the_attempts_blocks_the_pairing() {
        let (mut session, code) = open();
        let wrong = PairingCode::parse("ZZZZ9999").expect("valido");

        for _ in 0..MAX_PAIRING_ATTEMPTS {
            assert!(session.redeem(&wrong, NOW).is_err());
        }
        assert_eq!(session.state, PairingState::Blocked);
        assert_eq!(session.remaining_attempts(), 0);

        // Ni siquiera el codigo correcto sirve ya.
        assert_eq!(
            session.redeem(&code, NOW).unwrap_err(),
            SecurityError::PairingNotActive
        );
    }

    #[test]
    fn an_expired_code_is_rejected() {
        let (mut session, code) = open();
        let too_late = Timestamp(NOW.0 + DEFAULT_PAIRING_TTL.as_millis() as u64);
        assert_eq!(
            session.redeem(&code, too_late).unwrap_err(),
            SecurityError::PairingCodeExpired
        );
        assert_eq!(session.state, PairingState::Expired);
        assert!(!session.is_usable_at(too_late));
    }

    #[test]
    fn the_code_is_valid_until_the_last_millisecond() {
        let (mut session, code) = open();
        let just_in_time = Timestamp(NOW.0 + DEFAULT_PAIRING_TTL.as_millis() as u64 - 1);
        session
            .redeem(&code, just_in_time)
            .expect("dentro de la ventana");
    }

    #[test]
    fn cancelling_closes_the_pairing() {
        let (mut session, code) = open();
        session.cancel();
        assert_eq!(session.state, PairingState::Cancelled);
        assert_eq!(
            session.redeem(&code, NOW).unwrap_err(),
            SecurityError::PairingNotActive
        );
    }

    #[test]
    fn cancelling_a_completed_pairing_does_not_change_it() {
        let (mut session, code) = open();
        session.redeem(&code, NOW).expect("completa");
        session.cancel();
        assert_eq!(session.state, PairingState::Completed);
    }

    #[test]
    fn expire_if_due_reports_the_change_only_once() {
        let (mut session, _code) = open();
        let too_late = Timestamp(session.expires_at.0 + 1);
        assert!(session.expire_if_due(too_late));
        assert!(!session.expire_if_due(too_late), "ya estaba caducado");
    }

    #[test]
    fn the_stored_session_never_contains_the_plaintext_code() {
        let (session, code) = open();
        let serialized = serde_json::to_string(&session).expect("serializa");
        assert!(
            !serialized.contains(code.as_str()),
            "el codigo en claro aparece en el estado guardado"
        );
        assert!(
            serialized.contains("$argon2"),
            "deberia guardarse la derivacion PHC"
        );
    }

    #[test]
    fn the_same_code_does_not_work_on_a_different_pairing() {
        // Dos emparejamientos abiertos a la vez: el codigo de uno no debe
        // servir en el otro aunque por azar fuese el mismo texto.
        let (_first, code) = open();
        let (mut second, _other_code) = open();
        assert_eq!(
            second.redeem(&code, NOW).unwrap_err(),
            SecurityError::PairingCodeMismatch
        );
    }

    #[test]
    fn code_debug_does_not_leak_the_value() {
        let code = PairingCode::generate();
        let rendered = format!("{code:?}");
        assert!(!rendered.contains(code.as_str()));
    }

    #[test]
    fn invite_roundtrips_through_json() {
        let (session, code) = open();
        let host_key = DeviceIdentity::generate(DeviceId::new()).public_key();
        let invite =
            PairingInvite::new("wss://nexus.example/signaling", &session, &code, &host_key);

        let json = invite.to_json().expect("serializa");
        let back = PairingInvite::from_json(&json).expect("deserializa");
        assert_eq!(back, invite);

        // El codigo del QR canjea el emparejamiento.
        let mut session = session;
        session
            .redeem(&back.parsed_code().expect("codigo valido"), NOW)
            .expect("canjea");
        assert_eq!(session.state, PairingState::Completed);
    }

    #[test]
    fn invite_carries_the_full_host_key_not_just_its_fingerprint() {
        // La huella corta sirve para que una persona compare a ojo. Para que el
        // controlador pueda autenticar al host de verdad necesita la clave
        // completa, y tiene que llegarle fuera de banda: en la invitacion.
        let (session, code) = open();
        let host_key = DeviceIdentity::generate(session.host_id).public_key();
        let invite = PairingInvite::new("wss://x", &session, &code, &host_key);

        assert_eq!(invite.host_public_key_hex, host_key.to_hex());
        let recovered =
            DevicePublicKey::from_hex(&invite.host_public_key_hex).expect("clave utilizable");
        assert_eq!(recovered, host_key);
    }

    #[test]
    fn invite_carries_the_host_fingerprint_for_visual_check() {
        let (session, code) = open();
        let host_key = DeviceIdentity::generate(DeviceId::new()).public_key();
        let invite = PairingInvite::new("wss://x", &session, &code, &host_key);
        assert_eq!(invite.host_fingerprint, host_key.short_fingerprint());
    }

    #[test]
    fn malformed_invites_are_rejected() {
        assert!(PairingInvite::from_json("{}").is_err());
        assert!(PairingInvite::from_json("no es json").is_err());
    }
}
