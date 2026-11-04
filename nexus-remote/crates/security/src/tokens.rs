//! Comparacion de secretos de despliegue.
//!
//! El servidor de senalizacion compara un testigo de alta recibido por la red
//! con el que tiene configurado. Hacerlo con `==` filtraria informacion por el
//! tiempo de ejecucion: la comparacion de cadenas se detiene en el primer byte
//! distinto, de modo que medir cuanto tarda permite adivinar el secreto byte a
//! byte.
//!
//! Aqui se comparan los resumenes `SHA-256` de los dos valores en tiempo
//! constante. Resumir primero resuelve ademas el otro escape: comparar
//! longitudes distintas no revela la longitud del secreto.
//!
//! No hay criptografia propia en esto: son `SHA-256` de `sha2` y la comparacion
//! en tiempo constante de `subtle`.

use rand::rngs::SysRng;
use rand_core::TryRng;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Bytes de entropia de un secreto generado aqui.
///
/// 32 bytes son 256 bits: muy por encima de lo que se puede adivinar probando,
/// y es lo mismo que usan las claves de este proyecto.
pub const SECRET_BYTES: usize = 32;

/// Genera un secreto nuevo con entropia del sistema operativo.
///
/// Se devuelve en hexadecimal para que viaje por un protocolo de texto sin
/// codificaciones intermedias.
///
/// No hay generador propio: la entropia la da el sistema, a traves de `rand`.
///
/// # Panics
///
/// Si el generador del sistema operativo falla. Sin entropia no se puede
/// emitir un secreto, y seguir adelante con uno predecible seria peor que
/// parar.
#[must_use]
pub fn generate_secret() -> String {
    let mut raw = [0_u8; SECRET_BYTES];
    SysRng
        .try_fill_bytes(&mut raw)
        .expect("el generador del sistema operativo debe estar disponible");
    hex::encode(raw)
}

/// Indica si los dos secretos coinciden, sin filtrar por tiempo cual era.
///
/// Un secreto esperado vacio devuelve siempre `false`: una configuracion sin
/// secreto no debe autorizar a nadie, ni siquiera a quien presente una cadena
/// vacia.
#[must_use]
pub fn secret_matches(expected: &str, presented: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    let expected_digest = Sha256::digest(expected.as_bytes());
    let presented_digest = Sha256::digest(presented.as_bytes());
    expected_digest.ct_eq(&presented_digest).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_generated_secrets_are_different() {
        let a = generate_secret();
        let b = generate_secret();
        assert_ne!(a, b, "dos secretos seguidos no pueden coincidir");
        assert_eq!(
            a.len(),
            SECRET_BYTES * 2,
            "hexadecimal: dos letras por byte"
        );
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_generated_secret_matches_itself_and_nothing_else() {
        let secret = generate_secret();
        assert!(secret_matches(&secret, &secret));
        assert!(!secret_matches(&secret, &generate_secret()));
    }

    #[test]
    fn the_same_secret_matches() {
        assert!(secret_matches("testigo-de-alta-123", "testigo-de-alta-123"));
    }

    #[test]
    fn a_different_secret_does_not_match() {
        assert!(!secret_matches(
            "testigo-de-alta-123",
            "testigo-de-alta-124"
        ));
    }

    #[test]
    fn a_prefix_does_not_match() {
        assert!(!secret_matches("testigo-de-alta-123", "testigo"));
        assert!(!secret_matches("testigo", "testigo-de-alta-123"));
    }

    #[test]
    fn an_empty_expected_secret_authorises_nobody() {
        // Un servidor mal configurado no debe dejar entrar a quien no presente
        // nada.
        assert!(!secret_matches("", ""));
        assert!(!secret_matches("", "cualquier-cosa"));
    }

    #[test]
    fn an_empty_presented_secret_does_not_match() {
        assert!(!secret_matches("testigo", ""));
    }

    #[test]
    fn comparison_is_byte_exact_including_case_and_spaces() {
        assert!(!secret_matches("Testigo", "testigo"));
        assert!(!secret_matches("testigo", "testigo "));
    }
}
