//! Limpieza de texto que llega de la red.
//!
//! Cualquier cadena que venga de otro dispositivo (el nombre que declara, el
//! motivo con el que rechaza una sesion) se muestra en una interfaz y se
//! escribe en registros. Sin limpiarla, quien la envia puede insertar saltos de
//! linea para falsificar entradas en un registro, caracteres de control que
//! descolocan un terminal, o un texto de longitud arbitraria.

/// Convierte una cadena recibida en una sola linea acotada y sin controles.
///
/// Los caracteres de control pasan a espacios, los espacios consecutivos se
/// colapsan en uno y el resultado se recorta a `max_len` caracteres. Nunca
/// devuelve texto con saltos de linea.
///
/// Si no queda nada utilizable, devuelve `fallback`.
#[must_use]
pub fn sanitize_line(raw: &str, max_len: usize, fallback: &str) -> String {
    let cleaned = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if cleaned.is_empty() {
        return fallback.to_owned();
    }
    cleaned.chars().take(max_len).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_text_passes_through() {
        assert_eq!(sanitize_line("PC del salon", 64, "-"), "PC del salon");
    }

    #[test]
    fn control_characters_cannot_forge_log_lines() {
        let attack = "normal\n[2026-01-01] ADMIN: acceso concedido";
        let cleaned = sanitize_line(attack, 200, "-");
        assert!(!cleaned.contains('\n'), "{cleaned:?}");
        assert!(!cleaned.contains('\r'), "{cleaned:?}");
    }

    #[test]
    fn tabs_and_nulls_become_spaces() {
        assert_eq!(sanitize_line("a\tb\0c", 64, "-"), "a b c");
    }

    #[test]
    fn runs_of_whitespace_collapse() {
        assert_eq!(sanitize_line("  a     b  ", 64, "-"), "a b");
    }

    #[test]
    fn the_length_is_capped() {
        let long = "x".repeat(500);
        assert_eq!(sanitize_line(&long, 10, "-").chars().count(), 10);
    }

    #[test]
    fn an_empty_result_falls_back() {
        for raw in ["", "   ", "\n\r\t", "\0"] {
            assert_eq!(sanitize_line(raw, 64, "sin datos"), "sin datos");
        }
    }

    #[test]
    fn multibyte_text_is_counted_in_characters_not_bytes() {
        // Recortar por bytes partiria un caracter por la mitad.
        let cleaned = sanitize_line("ñññññ", 3, "-");
        assert_eq!(cleaned.chars().count(), 3);
        assert_eq!(cleaned, "ñññ");
    }
}
