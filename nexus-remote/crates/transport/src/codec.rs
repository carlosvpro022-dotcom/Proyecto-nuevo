//! Codificacion de los mensajes de senalizacion.
//!
//! Todo mensaje viaja dentro de un [`Envelope`], que anade version de protocolo
//! y marca temporal. Aqui se anade lo unico que falta: un limite de tamano.
//!
//! El limite importa porque el mensaje llega de la red antes de estar
//! autenticado. Sin un tope, cualquiera que alcance el puerto podria hacer que
//! el servidor reserve memoria arbitraria. Se comprueba el tamano **antes** de
//! intentar deserializar.

use nexus_core::protocol::Envelope;
use nexus_core::protocol::signaling::{ClientToServer, ServerToClient};

use crate::error::{TransportError, TransportResult};

/// Tamano maximo de un mensaje de senalizacion.
///
/// 64 KiB deja sitio de sobra para el mensaje mas grande del protocolo (una
/// descripcion SDP con muchos candidatos ronda unos pocos kilobytes) sin dar
/// margen a un abuso.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// Codifica un mensaje del dispositivo hacia el servidor.
///
/// # Errors
///
/// - [`TransportError::MessageTooLarge`] si el resultado supera el limite.
/// - [`TransportError::Malformed`] si la serializacion falla.
pub fn encode_client_message(envelope: &Envelope<ClientToServer>) -> TransportResult<String> {
    let raw =
        serde_json::to_string(envelope).map_err(|e| TransportError::Malformed(e.to_string()))?;
    check_size(raw.len())?;
    Ok(raw)
}

/// Codifica un mensaje del servidor hacia el dispositivo.
///
/// # Errors
///
/// Igual que [`encode_client_message`].
pub fn encode_server_message(envelope: &Envelope<ServerToClient>) -> TransportResult<String> {
    let raw =
        serde_json::to_string(envelope).map_err(|e| TransportError::Malformed(e.to_string()))?;
    check_size(raw.len())?;
    Ok(raw)
}

/// Decodifica un mensaje recibido del servidor.
///
/// El tamano se comprueba antes de deserializar y la version de protocolo la
/// valida [`Envelope::decode`].
///
/// # Errors
///
/// - [`TransportError::MessageTooLarge`] si el texto supera el limite.
/// - [`TransportError::Malformed`] si no corresponde al esquema o la version no
///   es compatible.
pub fn decode_server_message(raw: &str) -> TransportResult<Envelope<ServerToClient>> {
    check_size(raw.len())?;
    Envelope::decode(raw).map_err(|e| TransportError::Malformed(e.to_string()))
}

/// Decodifica un mensaje recibido de un dispositivo.
///
/// # Errors
///
/// Igual que [`decode_server_message`].
pub fn decode_client_message(raw: &str) -> TransportResult<Envelope<ClientToServer>> {
    check_size(raw.len())?;
    Envelope::decode(raw).map_err(|e| TransportError::Malformed(e.to_string()))
}

const fn check_size(size: usize) -> TransportResult<()> {
    if size > MAX_MESSAGE_BYTES {
        return Err(TransportError::MessageTooLarge {
            size,
            max: MAX_MESSAGE_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::Timestamp;

    #[test]
    fn a_client_message_roundtrips() {
        let envelope = Envelope::at(Timestamp(1_700_000_000_000), ClientToServer::Ping);
        let raw = encode_client_message(&envelope).expect("codifica");
        let back = decode_client_message(&raw).expect("decodifica");
        assert_eq!(back, envelope);
    }

    #[test]
    fn a_server_message_roundtrips() {
        let envelope = Envelope::at(Timestamp(1), ServerToClient::Pong);
        let raw = encode_server_message(&envelope).expect("codifica");
        let back = decode_server_message(&raw).expect("decodifica");
        assert_eq!(back, envelope);
    }

    #[test]
    fn an_oversized_message_is_refused_before_parsing() {
        let huge = "x".repeat(MAX_MESSAGE_BYTES + 1);
        let err = decode_server_message(&huge).unwrap_err();
        assert_eq!(
            err,
            TransportError::MessageTooLarge {
                size: MAX_MESSAGE_BYTES + 1,
                max: MAX_MESSAGE_BYTES
            }
        );
    }

    #[test]
    fn a_message_at_the_limit_is_accepted_by_the_size_check() {
        // Llega al parser (y falla ahi por no ser JSON valido), no al limite.
        let at_limit = "x".repeat(MAX_MESSAGE_BYTES);
        let err = decode_server_message(&at_limit).unwrap_err();
        assert!(matches!(err, TransportError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn encoding_an_oversized_payload_is_refused() {
        let envelope = Envelope::at(
            Timestamp(1),
            ClientToServer::Offer {
                session_id: nexus_core::SessionId::new(),
                sdp: "v".repeat(MAX_MESSAGE_BYTES),
            },
        );
        assert!(matches!(
            encode_client_message(&envelope).unwrap_err(),
            TransportError::MessageTooLarge { .. }
        ));
    }

    #[test]
    fn garbage_is_reported_as_malformed() {
        for bad in ["", "{", "null", "[]", "{\"v\":1}"] {
            assert!(
                matches!(
                    decode_server_message(bad).unwrap_err(),
                    TransportError::Malformed(_)
                ),
                "{bad:?} deberia ser malformado"
            );
        }
    }

    #[test]
    fn an_incompatible_protocol_version_is_rejected() {
        let raw = format!(
            "{{\"v\":{},\"ts\":1,\"payload\":{{\"type\":\"pong\"}}}}",
            nexus_core::PROTOCOL_VERSION + 1
        );
        assert!(matches!(
            decode_server_message(&raw).unwrap_err(),
            TransportError::Malformed(_)
        ));
    }
}
