//! Identificadores opacos del dominio.
//!
//! Se usan tipos nuevos (newtypes) en lugar de `String` o `Uuid` desnudos para
//! que el compilador impida mezclar un identificador de dispositivo con uno de
//! sesion. Todos serializan como cadena, lo que mantiene el protocolo legible.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::CoreError;

macro_rules! uuid_newtype {
    ($name:ident, $doc:literal, $prefix:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Genera un identificador nuevo con aleatoriedad del sistema.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Construye el identificador a partir de un UUID ya existente.
            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Devuelve el UUID subyacente.
            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// Prefijo legible usado en registros y en la interfaz.
            #[must_use]
            pub fn short(&self) -> String {
                // `{:.8}` no recorta el `Display` de Uuid (no usa `Formatter::pad`),
                // asi que el truncado se hace explicitamente sobre los caracteres.
                let hex: String = self.0.simple().to_string().chars().take(8).collect();
                format!("{}-{hex}", $prefix)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = CoreError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value)
                    .map(Self)
                    .map_err(|_| CoreError::InvalidId {
                        kind: stringify!($name),
                        value: value.to_owned(),
                    })
            }
        }
    };
}

uuid_newtype!(
    DeviceId,
    "Identidad estable de una instalacion concreta.",
    "dev"
);
uuid_newtype!(SessionId, "Identifica una sesion remota activa.", "ses");
uuid_newtype!(
    TransferId,
    "Identifica una transferencia de archivos.",
    "tx"
);
uuid_newtype!(
    PairingId,
    "Identifica un intento de emparejamiento.",
    "pair"
);

/// Correlaciona peticion y respuesta dentro del canal de control.
///
/// Es un contador monotono por sesion: barato de serializar y suficiente para
/// emparejar respuestas sin necesidad de un UUID por mensaje.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub u64);

impl RequestId {
    /// Primer identificador valido de una sesion.
    pub const FIRST: Self = Self(1);

    /// Devuelve el siguiente identificador, saturando en el maximo.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_roundtrips_through_string() {
        let id = DeviceId::new();
        let parsed: DeviceId = id.to_string().parse().expect("debe parsear");
        assert_eq!(id, parsed);
    }

    #[test]
    fn invalid_id_is_rejected() {
        let err = "no-es-un-uuid".parse::<DeviceId>().unwrap_err();
        assert!(matches!(err, CoreError::InvalidId { .. }));
    }

    #[test]
    fn ids_of_different_kinds_are_distinct_types() {
        // Compila solo porque son tipos distintos; la prueba documenta la intencion.
        let device = DeviceId::new();
        let session = SessionId::new();
        assert_ne!(device.to_string(), session.to_string());
    }

    #[test]
    fn short_form_is_prefixed() {
        let id = DeviceId::new();
        assert!(id.short().starts_with("dev-"));
        assert_eq!(id.short().len(), "dev-".len() + 8);
    }

    #[test]
    fn request_id_advances_and_saturates() {
        assert_eq!(RequestId::FIRST.next(), RequestId(2));
        assert_eq!(RequestId(u64::MAX).next(), RequestId(u64::MAX));
    }

    #[test]
    fn ids_serialize_as_plain_strings() {
        let id = SessionId::new();
        let json = serde_json::to_string(&id).expect("serializa");
        assert_eq!(json, format!("\"{id}\""));
    }
}
