//! Contratos de comunicacion de NEXUS REMOTE.
//!
//! Hay dos protocolos distintos y deliberadamente separados:
//!
//! - [`signaling`]: JSON sobre WebSocket con TLS contra el servidor propio.
//!   Sirve para que los dispositivos se encuentren y negocien WebRTC. El
//!   servidor ve estos mensajes.
//! - [`control`]: JSON sobre el canal de datos de WebRTC, cifrado extremo a
//!   extremo con DTLS. Transporta raton, teclado, portapapeles y archivos. Ni
//!   el servidor de senalizacion ni el TURN pueden leerlo.
//!
//! Ambos viajan dentro de un [`Envelope`], que anade version de protocolo y
//! marca temporal. La marca la necesita `nexus-security` para rechazar
//! mensajes repetidos.

pub mod control;
pub mod input;
pub mod signaling;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::ids::RequestId;
use crate::time::Timestamp;

/// Version del protocolo que implementa este binario.
///
/// Se incrementa cuando un cambio rompe la compatibilidad. Las adiciones
/// compatibles (un campo opcional nuevo, una capacidad nueva) no la cambian:
/// para eso esta la negociacion de capacidades.
pub const PROTOCOL_VERSION: u16 = 1;

/// Version minima con la que este binario sabe hablar.
pub const MIN_SUPPORTED_PROTOCOL: u16 = 1;

/// Sobre comun a todos los mensajes del sistema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// Version del protocolo del emisor.
    pub v: u16,
    /// Marca temporal del emisor, en milisegundos desde la epoca UNIX.
    pub ts: Timestamp,
    /// Correlacion peticion/respuesta. Ausente en mensajes no solicitados.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rid: Option<RequestId>,
    /// Contenido concreto.
    pub payload: T,
}

impl<T> Envelope<T> {
    /// Crea un sobre con la version actual y el reloj del sistema.
    #[must_use]
    pub fn new(payload: T) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ts: Timestamp::now(),
            rid: None,
            payload,
        }
    }

    /// Crea un sobre correlacionado con una peticion.
    #[must_use]
    pub fn reply_to(rid: RequestId, payload: T) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ts: Timestamp::now(),
            rid: Some(rid),
            payload,
        }
    }

    /// Crea un sobre con marca temporal explicita. Pensado para pruebas.
    #[must_use]
    pub const fn at(ts: Timestamp, payload: T) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ts,
            rid: None,
            payload,
        }
    }

    /// Comprueba que la version del emisor es compatible.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnsupportedProtocol`] si la version esta fuera del rango
    /// que este binario implementa.
    pub const fn check_version(&self) -> CoreResult<()> {
        if self.v < MIN_SUPPORTED_PROTOCOL || self.v > PROTOCOL_VERSION {
            return Err(CoreError::UnsupportedProtocol {
                received: self.v,
                supported: PROTOCOL_VERSION,
            });
        }
        Ok(())
    }
}

impl<T: Serialize> Envelope<T> {
    /// Codifica el sobre a JSON.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] si el contenido no es serializable.
    pub fn encode(&self) -> CoreResult<String> {
        serde_json::to_string(self).map_err(|e| CoreError::Malformed(e.to_string()))
    }
}

impl<T: DeserializeOwned> Envelope<T> {
    /// Decodifica un sobre y valida su version en el mismo paso.
    ///
    /// Se valida aqui y no en el llamante para que sea imposible olvidarlo: un
    /// mensaje de una version futura se rechaza antes de llegar a la logica.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] si el JSON no corresponde al esquema, o
    /// [`CoreError::UnsupportedProtocol`] si la version no es compatible.
    pub fn decode(raw: &str) -> CoreResult<Self> {
        let envelope: Self =
            serde_json::from_str(raw).map_err(|e| CoreError::Malformed(e.to_string()))?;
        envelope.check_version()?;
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::signaling::ClientToServer;

    #[test]
    fn envelope_roundtrips() {
        let envelope = Envelope::at(Timestamp(1_700_000_000_000), ClientToServer::Ping);
        let raw = envelope.encode().expect("codifica");
        let back: Envelope<ClientToServer> = Envelope::decode(&raw).expect("decodifica");
        assert_eq!(envelope, back);
    }

    #[test]
    fn request_id_is_omitted_when_absent() {
        let raw = Envelope::at(Timestamp(1), ClientToServer::Ping)
            .encode()
            .expect("codifica");
        assert!(!raw.contains("rid"), "no debe aparecer rid vacio: {raw}");
    }

    #[test]
    fn reply_carries_the_request_id() {
        let envelope = Envelope::reply_to(RequestId(42), ClientToServer::Ping);
        let raw = envelope.encode().expect("codifica");
        assert!(raw.contains("\"rid\":42"), "{raw}");
    }

    #[test]
    fn future_protocol_version_is_rejected() {
        let raw = format!(
            "{{\"v\":{},\"ts\":1,\"payload\":{{\"type\":\"ping\"}}}}",
            PROTOCOL_VERSION + 1
        );
        let err = Envelope::<ClientToServer>::decode(&raw).unwrap_err();
        assert!(matches!(err, CoreError::UnsupportedProtocol { .. }));
    }

    #[test]
    fn version_zero_is_rejected() {
        let raw = "{\"v\":0,\"ts\":1,\"payload\":{\"type\":\"ping\"}}";
        assert!(Envelope::<ClientToServer>::decode(raw).is_err());
    }

    #[test]
    fn garbage_is_reported_as_malformed() {
        let err = Envelope::<ClientToServer>::decode("{no es json").unwrap_err();
        assert!(matches!(err, CoreError::Malformed(_)));
    }

    #[test]
    fn unknown_payload_type_is_rejected() {
        // El protocolo de senalizacion no tolera variantes desconocidas: un
        // mensaje que no entendemos es un error, no algo que ignorar en silencio.
        let raw = "{\"v\":1,\"ts\":1,\"payload\":{\"type\":\"tomar_el_control\"}}";
        assert!(Envelope::<ClientToServer>::decode(raw).is_err());
    }
}
