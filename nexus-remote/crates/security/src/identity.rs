//! Identidad criptografica de un dispositivo.
//!
//! Cada instalacion (el host Windows, el iPhone, el iPad) genera su propio par
//! de claves Ed25519 **una sola vez** y nunca lo comparte. La clave privada no
//! sale del dispositivo; lo que se intercambia al emparejar es solo la publica.
//!
//! No se implementa criptografia propia: se usa `ed25519-dalek`, y la
//! verificacion es siempre la variante estricta, que rechaza claves de orden
//! pequeno y firmas maleables.
//!
//! ## Dependencia a futuro
//!
//! El almacenamiento de la clave privada en el almacen seguro de cada
//! plataforma (DPAPI en Windows, Keychain en iOS) es trabajo de la etapa 2 y
//! vive en los crates de aplicacion. Este modulo solo sabe serializar la clave
//! a bytes; quien decida donde guardarlos es el llamante.

use std::fmt;

use ed25519_dalek::{
    SECRET_KEY_LENGTH, Signature, SigningKey, VerifyingKey, ed25519::signature::Signer,
};
use nexus_core::DeviceId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::{SecurityError, SecurityResult};

/// Longitud en bytes de una clave publica Ed25519.
pub const PUBLIC_KEY_LEN: usize = 32;
/// Longitud en bytes de una clave privada Ed25519.
pub const SECRET_KEY_LEN: usize = SECRET_KEY_LENGTH;
/// Longitud en bytes de una firma Ed25519.
pub const SIGNATURE_LEN: usize = 64;

/// Clave publica de un dispositivo, lista para almacenar y comparar.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DevicePublicKey {
    bytes: [u8; PUBLIC_KEY_LEN],
}

impl DevicePublicKey {
    /// Construye la clave a partir de sus bytes, validando que sea utilizable.
    ///
    /// Se rechazan dos cosas distintas:
    ///
    /// 1. Bytes que no decodifican a un punto de la curva.
    /// 2. Claves de **orden pequeno**, que decodifican bien pero con las que
    ///    `verify_strict` nunca aceptaria una firma. Rechazarlas aqui significa
    ///    que un dispositivo con una clave inutil no llega a registrarse, en
    ///    lugar de quedar emparejado y fallar despues en cada conexion.
    ///
    /// # Errors
    ///
    /// [`SecurityError::InvalidKeyMaterial`] en cualquiera de los dos casos.
    pub fn from_bytes(bytes: [u8; PUBLIC_KEY_LEN]) -> SecurityResult<Self> {
        let key = VerifyingKey::from_bytes(&bytes)
            .map_err(|e| SecurityError::InvalidKeyMaterial(e.to_string()))?;
        if key.is_weak() {
            return Err(SecurityError::InvalidKeyMaterial(
                "clave de orden pequeno: no sirve para autenticar".to_owned(),
            ));
        }
        Ok(Self { bytes })
    }

    /// Construye la clave desde su representacion hexadecimal.
    ///
    /// # Errors
    ///
    /// [`SecurityError::InvalidKeyMaterial`] si el texto no es hexadecimal de
    /// 64 caracteres o no decodifica a un punto valido.
    pub fn from_hex(hex_str: &str) -> SecurityResult<Self> {
        let raw = hex::decode(hex_str)
            .map_err(|e| SecurityError::InvalidKeyMaterial(format!("hex invalido: {e}")))?;
        let bytes: [u8; PUBLIC_KEY_LEN] = raw.try_into().map_err(|v: Vec<u8>| {
            SecurityError::InvalidKeyMaterial(format!(
                "se esperaban {PUBLIC_KEY_LEN} bytes, llegaron {}",
                v.len()
            ))
        })?;
        Self::from_bytes(bytes)
    }

    /// Bytes de la clave.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; PUBLIC_KEY_LEN] {
        &self.bytes
    }

    /// Representacion hexadecimal, que es la que viaja por el protocolo.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.bytes)
    }

    /// Huella SHA-256 completa de la clave, en hexadecimal.
    ///
    /// Se usa en los registros de auditoria: identifica la clave sin exponerla.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        hex::encode(Sha256::digest(self.bytes))
    }

    /// Huella corta y legible, agrupada en bloques de cuatro.
    ///
    /// Pensada para que la persona la compare a ojo entre el host y el iPad al
    /// emparejar. No sustituye a la verificacion criptografica.
    #[must_use]
    pub fn short_fingerprint(&self) -> String {
        self.fingerprint()
            .chars()
            .take(16)
            .collect::<Vec<_>>()
            .chunks(4)
            .map(|c| c.iter().collect::<String>().to_uppercase())
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Verifica una firma sobre un mensaje.
    ///
    /// Usa `verify_strict`, que rechaza claves de orden pequeno y firmas
    /// maleables. Es mas restrictivo que la verificacion basica y es lo que
    /// corresponde a un protocolo de autenticacion.
    ///
    /// # Errors
    ///
    /// [`SecurityError::InvalidSignature`] si la firma no corresponde, y
    /// [`SecurityError::InvalidKeyMaterial`] si la firma no mide 64 bytes.
    pub fn verify(&self, message: &[u8], signature_bytes: &[u8]) -> SecurityResult<()> {
        let signature: [u8; SIGNATURE_LEN] = signature_bytes.try_into().map_err(|_| {
            SecurityError::InvalidKeyMaterial(format!(
                "se esperaban {SIGNATURE_LEN} bytes de firma, llegaron {}",
                signature_bytes.len()
            ))
        })?;

        let key = VerifyingKey::from_bytes(&self.bytes)
            .map_err(|e| SecurityError::InvalidKeyMaterial(e.to_string()))?;

        key.verify_strict(message, &Signature::from_bytes(&signature))
            .map_err(|_| SecurityError::InvalidSignature)
    }

    /// Verifica una firma recibida en hexadecimal.
    ///
    /// # Errors
    ///
    /// Igual que [`Self::verify`], mas [`SecurityError::InvalidKeyMaterial`] si
    /// el texto no es hexadecimal.
    pub fn verify_hex(&self, message: &[u8], signature_hex: &str) -> SecurityResult<()> {
        let raw = hex::decode(signature_hex)
            .map_err(|e| SecurityError::InvalidKeyMaterial(format!("hex invalido: {e}")))?;
        self.verify(message, &raw)
    }
}

impl fmt::Debug for DevicePublicKey {
    /// No imprime la clave completa: en un registro basta la huella corta.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DevicePublicKey({})", self.short_fingerprint())
    }
}

impl fmt::Display for DevicePublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl TryFrom<String> for DevicePublicKey {
    type Error = SecurityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_hex(&value)
    }
}

impl From<DevicePublicKey> for String {
    fn from(value: DevicePublicKey) -> Self {
        value.to_hex()
    }
}

/// Par de claves de este dispositivo.
///
/// `Debug` no revela la clave privada y los bytes se borran de memoria al
/// exportarlos (via [`Zeroizing`]) para reducir la ventana en que quedan en
/// memoria sin cifrar.
pub struct DeviceIdentity {
    device_id: DeviceId,
    signing_key: SigningKey,
}

impl DeviceIdentity {
    /// Genera una identidad nueva con aleatoriedad del sistema.
    #[must_use]
    pub fn generate(device_id: DeviceId) -> Self {
        Self {
            device_id,
            signing_key: SigningKey::generate(&mut rand::rng()),
        }
    }

    /// Restaura una identidad guardada previamente.
    ///
    /// # Errors
    ///
    /// [`SecurityError::InvalidKeyMaterial`] si el material no mide
    /// exactamente [`SECRET_KEY_LEN`] bytes.
    pub fn from_secret_bytes(device_id: DeviceId, secret: &[u8]) -> SecurityResult<Self> {
        let bytes: [u8; SECRET_KEY_LEN] = secret.try_into().map_err(|_| {
            SecurityError::InvalidKeyMaterial(format!(
                "se esperaban {SECRET_KEY_LEN} bytes de clave privada, llegaron {}",
                secret.len()
            ))
        })?;
        Ok(Self {
            device_id,
            signing_key: SigningKey::from_bytes(&bytes),
        })
    }

    /// Identificador del dispositivo al que pertenece la identidad.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Clave publica correspondiente.
    ///
    /// # Panics
    ///
    /// No entra en panico: la clave publica derivada de una privada valida es
    /// siempre un punto valido, de modo que el `expect` es inalcanzable.
    #[must_use]
    pub fn public_key(&self) -> DevicePublicKey {
        DevicePublicKey::from_bytes(self.signing_key.verifying_key().to_bytes())
            .expect("la clave publica derivada de una privada valida siempre es valida")
    }

    /// Exporta la clave privada para guardarla en el almacen seguro del sistema.
    ///
    /// El buffer devuelto se borra al destruirse. Quien lo reciba debe
    /// escribirlo cifrado (DPAPI en Windows, Keychain en iOS) y no dejarlo en
    /// un archivo en claro.
    #[must_use]
    pub fn export_secret(&self) -> Zeroizing<[u8; SECRET_KEY_LEN]> {
        Zeroizing::new(self.signing_key.to_bytes())
    }

    /// Firma un mensaje.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> [u8; SIGNATURE_LEN] {
        self.signing_key.sign(message).to_bytes()
    }

    /// Firma un mensaje y devuelve la firma en hexadecimal, lista para el protocolo.
    #[must_use]
    pub fn sign_hex(&self, message: &[u8]) -> String {
        hex::encode(self.sign(message))
    }
}

impl fmt::Debug for DeviceIdentity {
    /// Nunca imprime la clave privada.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("device_id", &self.device_id)
            .field("public_key", &self.public_key())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_verifies_with_the_matching_public_key() {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let message = b"transcripcion de autenticacion";
        let signature = identity.sign(message);
        identity
            .public_key()
            .verify(message, &signature)
            .expect("debe verificar");
    }

    #[test]
    fn signature_fails_with_a_different_key() {
        let a = DeviceIdentity::generate(DeviceId::new());
        let b = DeviceIdentity::generate(DeviceId::new());
        let message = b"mensaje";
        let signature = a.sign(message);
        assert_eq!(
            b.public_key().verify(message, &signature).unwrap_err(),
            SecurityError::InvalidSignature
        );
    }

    #[test]
    fn signature_fails_if_the_message_changes() {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let signature = identity.sign(b"abrir sesion de solo lectura");
        assert_eq!(
            identity
                .public_key()
                .verify(b"abrir sesion de administrador", &signature)
                .unwrap_err(),
            SecurityError::InvalidSignature
        );
    }

    #[test]
    fn flipping_one_bit_of_the_signature_is_rejected() {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let message = b"mensaje";
        let mut signature = identity.sign(message);
        signature[0] ^= 0x01;
        assert_eq!(
            identity
                .public_key()
                .verify(message, &signature)
                .unwrap_err(),
            SecurityError::InvalidSignature
        );
    }

    #[test]
    fn signature_of_wrong_length_is_rejected_as_key_material() {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let err = identity.public_key().verify(b"m", &[0_u8; 10]).unwrap_err();
        assert!(matches!(err, SecurityError::InvalidKeyMaterial(_)));
    }

    #[test]
    fn identity_survives_an_export_import_cycle() {
        let device_id = DeviceId::new();
        let original = DeviceIdentity::generate(device_id);
        let exported = original.export_secret();

        let restored = DeviceIdentity::from_secret_bytes(device_id, exported.as_slice())
            .expect("debe restaurar");

        assert_eq!(restored.public_key(), original.public_key());
        assert_eq!(restored.device_id(), device_id);

        // Una firma hecha con la identidad restaurada la valida la clave original.
        let signature = restored.sign(b"prueba");
        original
            .public_key()
            .verify(b"prueba", &signature)
            .expect("misma clave");
    }

    #[test]
    fn importing_truncated_secret_material_fails() {
        let err = DeviceIdentity::from_secret_bytes(DeviceId::new(), &[0_u8; 5]).unwrap_err();
        assert!(matches!(err, SecurityError::InvalidKeyMaterial(_)));
    }

    #[test]
    fn two_identities_are_never_equal() {
        let a = DeviceIdentity::generate(DeviceId::new());
        let b = DeviceIdentity::generate(DeviceId::new());
        assert_ne!(a.public_key(), b.public_key());
    }

    #[test]
    fn public_key_roundtrips_through_hex() {
        let key = DeviceIdentity::generate(DeviceId::new()).public_key();
        let hex_form = key.to_hex();
        assert_eq!(hex_form.len(), PUBLIC_KEY_LEN * 2);
        assert_eq!(DevicePublicKey::from_hex(&hex_form).expect("parsea"), key);
    }

    #[test]
    fn malformed_hex_public_keys_are_rejected() {
        for bad in ["", "zz", "abcd", &"ab".repeat(31), &"ab".repeat(33)] {
            assert!(
                DevicePublicKey::from_hex(bad).is_err(),
                "{bad:?} no deberia aceptarse como clave"
            );
        }
    }

    #[test]
    fn small_order_public_keys_are_rejected() {
        // El punto todo-ceros decodifica correctamente (es el elemento neutro
        // comprimido) pero es de orden pequeno: `verify_strict` nunca aceptaria
        // una firma suya, asi que se rechaza ya al construir la clave.
        let err = DevicePublicKey::from_bytes([0_u8; PUBLIC_KEY_LEN]).unwrap_err();
        assert!(
            matches!(err, SecurityError::InvalidKeyMaterial(_)),
            "{err:?}"
        );

        // El mismo punto por la via hexadecimal, que es la que usa el protocolo.
        assert!(DevicePublicKey::from_hex(&"00".repeat(PUBLIC_KEY_LEN)).is_err());
    }

    #[test]
    fn a_freshly_generated_key_is_not_weak() {
        for _ in 0..16 {
            let identity = DeviceIdentity::generate(DeviceId::new());
            // `public_key()` entraria en panico si la clave derivada fuese debil.
            let _ = identity.public_key();
        }
    }

    #[test]
    fn hex_signature_path_matches_the_byte_path() {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let message = b"mensaje";
        let signature_hex = identity.sign_hex(message);
        assert_eq!(signature_hex.len(), SIGNATURE_LEN * 2);
        identity
            .public_key()
            .verify_hex(message, &signature_hex)
            .expect("verifica");
        assert!(
            identity
                .public_key()
                .verify_hex(message, "no-es-hex")
                .is_err()
        );
    }

    #[test]
    fn fingerprints_are_stable_and_distinct() {
        let a = DeviceIdentity::generate(DeviceId::new()).public_key();
        let b = DeviceIdentity::generate(DeviceId::new()).public_key();

        assert_eq!(
            a.fingerprint(),
            a.fingerprint(),
            "la huella debe ser estable"
        );
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_eq!(
            a.fingerprint().len(),
            64,
            "SHA-256 en hex son 64 caracteres"
        );

        // AAAA-BBBB-CCCC-DDDD
        assert_eq!(a.short_fingerprint().len(), 19);
        assert_eq!(a.short_fingerprint().matches('-').count(), 3);
    }

    #[test]
    fn debug_output_never_leaks_the_secret_key() {
        let identity = DeviceIdentity::generate(DeviceId::new());
        let secret_hex = hex::encode(identity.export_secret().as_slice());
        let rendered = format!("{identity:?}");

        assert!(
            !rendered.contains(&secret_hex),
            "el Debug filtra la clave privada"
        );
        assert!(rendered.contains("DeviceIdentity"));

        // El Debug de la clave publica tampoco imprime la clave entera.
        let public = identity.public_key();
        let public_debug = format!("{public:?}");
        assert!(!public_debug.contains(&public.to_hex()));
        assert!(public_debug.contains(&public.short_fingerprint()));
    }

    #[test]
    fn public_key_serializes_as_a_hex_string() {
        let key = DeviceIdentity::generate(DeviceId::new()).public_key();
        let json = serde_json::to_string(&key).expect("serializa");
        assert_eq!(json, format!("\"{}\"", key.to_hex()));
        let back: DevicePublicKey = serde_json::from_str(&json).expect("deserializa");
        assert_eq!(back, key);
    }

    #[test]
    fn deserializing_an_invalid_key_fails_instead_of_panicking() {
        let result: Result<DevicePublicKey, _> = serde_json::from_str("\"00\"");
        assert!(result.is_err());
    }
}
