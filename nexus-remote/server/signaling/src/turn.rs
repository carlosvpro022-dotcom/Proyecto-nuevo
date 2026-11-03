//! Credenciales TURN temporales.
//!
//! El servidor TURN propio no lleva una lista de usuarios: usa el mecanismo de
//! credenciales de tiempo limitado de Coturn (`use-auth-secret`). El servidor de
//! senalizacion y Coturn comparten un secreto, y de ahi se derivan credenciales
//! que caducan solas:
//!
//! ```text
//! usuario    = <caducidad en segundos UNIX>:<identificador>
//! contrasena = base64( HMAC-SHA1( usuario, secreto_compartido ) )
//! ```
//!
//! La ventaja es que filtrar una credencial no da acceso permanente al relay:
//! deja de valer al pasar su caducidad. Coturn recalcula el mismo HMAC y
//! comprueba la marca, de modo que no hace falta ninguna comunicacion entre los
//! dos servicios.
//!
//! HMAC-SHA1 no es una eleccion de diseno propia: es lo que especifica el
//! borrador de la API REST de TURN y lo que Coturn implementa.

use std::time::Duration;

use base64::prelude::{BASE64_STANDARD, Engine};
use hmac::{Hmac, KeyInit, Mac};
use nexus_core::Timestamp;
use nexus_core::protocol::signaling::IceServer;
use sha1::Sha1;

/// Validez por omision de una credencial TURN.
///
/// Doce horas cubren de sobra una sesion larga sin dejar la credencial viva
/// indefinidamente.
pub const DEFAULT_CREDENTIAL_TTL: Duration = Duration::from_secs(12 * 3_600);

/// Credencial TURN temporal ya derivada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCredential {
    /// Usuario con la caducidad incorporada.
    pub username: String,
    /// Contrasena derivada, en base64.
    pub password: String,
    /// Instante de caducidad.
    pub expires_at: Timestamp,
}

impl TurnCredential {
    /// Deriva una credencial valida hasta `now + ttl`.
    ///
    /// `identity` solo sirve para que los registros de Coturn sean legibles: no
    /// aporta seguridad por si misma, porque quien valida es el HMAC.
    ///
    /// # Panics
    ///
    /// No entra en panico: HMAC admite claves de cualquier longitud, de modo
    /// que el `expect` interno es inalcanzable.
    #[must_use]
    pub fn derive(shared_secret: &[u8], identity: &str, now: Timestamp, ttl: Duration) -> Self {
        let expires_at = now.saturating_add(ttl);
        let expiry_secs = expires_at.0 / 1_000;
        let username = format!("{expiry_secs}:{identity}");

        // `new_from_slice` con HMAC acepta claves de cualquier longitud, asi que
        // este camino no puede fallar.
        let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(shared_secret)
            .expect("HMAC admite claves de cualquier longitud");
        mac.update(username.as_bytes());
        let password = BASE64_STANDARD.encode(mac.finalize().into_bytes());

        Self {
            username,
            password,
            expires_at,
        }
    }

    /// Indica si la credencial sigue vigente.
    #[must_use]
    pub const fn is_valid_at(&self, now: Timestamp) -> bool {
        now.0 < self.expires_at.0
    }

    /// Construye la entrada ICE que se envia al dispositivo.
    #[must_use]
    pub fn to_ice_server(&self, urls: Vec<String>) -> IceServer {
        IceServer {
            urls,
            username: Some(self.username.clone()),
            credential: Some(self.password.clone()),
            expires_at: Some(self.expires_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp(1_730_000_000_000);
    const SECRET: &[u8] = b"secreto-compartido-con-coturn";

    #[test]
    fn hmac_sha1_matches_the_known_answer() {
        // Vector de prueba publico de HMAC-SHA1: comprueba que la primitiva se
        // usa correctamente, antes de confiar en el resto de la derivacion.
        let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(b"key").expect("clave valida");
        mac.update(b"The quick brown fox jumps over the lazy dog");
        assert_eq!(
            hex_encode(&mac.finalize().into_bytes()),
            "de7c9b85b8b78aa6bc8a7a36f70a90701c9db4d9"
        );
    }

    fn hex_encode(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
    }

    #[test]
    fn the_username_carries_the_expiry_in_unix_seconds() {
        let credential = TurnCredential::derive(SECRET, "ipad", NOW, Duration::from_secs(3_600));
        let expected_secs = NOW.0 / 1_000 + 3_600;
        assert_eq!(credential.username, format!("{expected_secs}:ipad"));
    }

    #[test]
    fn the_derivation_is_deterministic() {
        let first = TurnCredential::derive(SECRET, "ipad", NOW, DEFAULT_CREDENTIAL_TTL);
        let second = TurnCredential::derive(SECRET, "ipad", NOW, DEFAULT_CREDENTIAL_TTL);
        assert_eq!(first, second, "Coturn debe poder recalcular lo mismo");
    }

    #[test]
    fn a_different_secret_gives_a_different_password() {
        let good = TurnCredential::derive(SECRET, "ipad", NOW, DEFAULT_CREDENTIAL_TTL);
        let forged = TurnCredential::derive(b"otro-secreto", "ipad", NOW, DEFAULT_CREDENTIAL_TTL);
        assert_eq!(good.username, forged.username, "el usuario es publico");
        assert_ne!(
            good.password, forged.password,
            "sin el secreto no se puede forjar"
        );
    }

    #[test]
    fn a_different_identity_or_expiry_changes_the_password() {
        let base = TurnCredential::derive(SECRET, "ipad", NOW, DEFAULT_CREDENTIAL_TTL);
        let other_identity = TurnCredential::derive(SECRET, "iphone", NOW, DEFAULT_CREDENTIAL_TTL);
        let other_expiry = TurnCredential::derive(SECRET, "ipad", NOW, Duration::from_secs(60));

        assert_ne!(base.password, other_identity.password);
        assert_ne!(base.password, other_expiry.password);
    }

    #[test]
    fn the_password_is_valid_base64() {
        let credential = TurnCredential::derive(SECRET, "ipad", NOW, DEFAULT_CREDENTIAL_TTL);
        let decoded = BASE64_STANDARD
            .decode(&credential.password)
            .expect("base64 valido");
        assert_eq!(decoded.len(), 20, "HMAC-SHA1 produce 20 bytes");
    }

    #[test]
    fn validity_follows_the_ttl() {
        let ttl = Duration::from_secs(600);
        let credential = TurnCredential::derive(SECRET, "ipad", NOW, ttl);
        assert!(credential.is_valid_at(NOW));
        assert!(credential.is_valid_at(Timestamp(NOW.0 + 599_000)));
        assert!(!credential.is_valid_at(Timestamp(NOW.0 + 600_000)));
    }

    #[test]
    fn an_empty_secret_still_derives_without_panicking() {
        // Una configuracion sin secreto es un error de despliegue que detecta
        // `Config::from_env`, pero la derivacion no debe entrar en panico.
        let credential = TurnCredential::derive(b"", "ipad", NOW, DEFAULT_CREDENTIAL_TTL);
        assert!(!credential.password.is_empty());
    }

    #[test]
    fn the_ice_entry_is_usable_by_the_client() {
        let credential = TurnCredential::derive(SECRET, "ipad", NOW, DEFAULT_CREDENTIAL_TTL);
        let server = credential.to_ice_server(vec![
            "turn:turn.nexus.example:3478?transport=udp".to_owned(),
            "turns:turn.nexus.example:5349?transport=tcp".to_owned(),
        ]);
        assert!(
            server.is_usable(),
            "una entrada TURN necesita credenciales y las tiene"
        );
        assert_eq!(
            server.username.as_deref(),
            Some(credential.username.as_str())
        );
        assert_eq!(server.expires_at, Some(credential.expires_at));
    }
}
