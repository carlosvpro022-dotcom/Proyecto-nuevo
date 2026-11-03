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

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

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
