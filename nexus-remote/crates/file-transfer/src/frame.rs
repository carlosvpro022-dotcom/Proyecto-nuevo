//! Formato de los marcos binarios que llevan los bytes de un archivo.
//!
//! ## Por que un formato binario y un canal aparte
//!
//! El canal de control lleva JSON. Meter los bytes de un archivo ahi obligaria
//! a codificarlos en base64, que infla el tamano un 33 %, y ademas los pondria
//! en la misma cola que las ordenes: el canal es ordenado y fiable, asi que una
//! descarga de 2 GB dejaria el raton congelado hasta terminar. Por eso los
//! archivos viajan en binario por su propio canal.
//!
//! ## La cabecera
//!
//! ```text
//! posicion  bytes  campo
//! 0         1      version del formato
//! 1         1      tipo de marco
//! 2         16     identificador de la transferencia
//! 18        4      numero de secuencia (big endian)
//! 22        ...    carga
//! ```
//!
//! La cabecera es de tamano fijo a proposito: no hay longitudes declaradas por
//! quien envia, de modo que no existe el error clasico de creer una longitud
//! que no se corresponde con los bytes recibidos. El canal de datos de WebRTC
//! conserva los limites de cada mensaje, asi que el tamano real del marco es el
//! del mensaje y no hay que delimitar nada.
//!
//! ## Que se comprueba al analizar
//!
//! Estos marcos llegan de la red. Se rechaza, **antes** de reservar nada:
//! un marco mas corto que la cabecera, una version que no se conoce, un tipo
//! que no se conoce, una carga mayor que el trozo maximo, una carga vacia donde
//! tiene que haber datos, y un digest que no mide 32 bytes.

use nexus_core::TransferId;
use uuid::Uuid;

use crate::error::{FileError, FileResult};
use crate::transfer::{CHUNK_SIZE_BYTES, SCTP_MAX_MESSAGE_BYTES};

/// Version del formato de marco que entiende esta version del programa.
pub const FRAME_VERSION: u8 = 1;

/// Tamano de la cabecera de todo marco.
pub const HEADER_BYTES: usize = 22;

/// Tamano del digest que cierra una transferencia (SHA-256).
pub const DIGEST_BYTES: usize = 32;

/// Tamano maximo de un marco completo, cabecera incluida.
///
/// Es el tope que el receptor aplica al mensaje antes de mirarlo, y el que debe
/// configurarse en el canal de datos.
pub const MAX_FRAME_BYTES: usize = HEADER_BYTES + CHUNK_SIZE_BYTES;

// Un marco que no quepa en un mensaje del canal de datos no se podria enviar,
// y el fallo solo aparecerian al mover un archivo grande por una conexion de
// verdad. Que no compile es mucho mas barato.
const _: () = assert!(
    MAX_FRAME_BYTES <= SCTP_MAX_MESSAGE_BYTES,
    "el marco maximo no cabe en un mensaje del canal de datos"
);

/// Codigo del marco que lleva datos.
const KIND_DATA: u8 = 1;
/// Codigo del marco que cierra la transferencia.
const KIND_END: u8 = 2;
/// Codigo del marco que la aborta.
const KIND_ABORT: u8 = 3;

/// Un marco del canal de archivos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Un trozo de archivo.
    Data {
        /// Transferencia a la que pertenece.
        transfer_id: TransferId,
        /// Posicion del trozo, empezando en cero.
        sequence: u32,
        /// Bytes del archivo.
        payload: Vec<u8>,
    },
    /// Fin de la transferencia, con lo necesario para verificarla.
    End {
        /// Transferencia que termina.
        transfer_id: TransferId,
        /// Cuantos marcos de datos se enviaron.
        ///
        /// Lo comprueba el receptor: si no cuadra con lo que recibio, falta
        /// algo y la transferencia no se da por buena.
        frames: u32,
        /// SHA-256 del archivo completo, calculado por quien envia.
        digest: [u8; DIGEST_BYTES],
    },
    /// La transferencia se corta sin completarse.
    Abort {
        /// Transferencia que se abandona.
        transfer_id: TransferId,
    },
}

impl Frame {
    /// Transferencia a la que se refiere el marco.
    #[must_use]
    pub const fn transfer_id(&self) -> TransferId {
        match self {
            Self::Data { transfer_id, .. }
            | Self::End { transfer_id, .. }
            | Self::Abort { transfer_id } => *transfer_id,
        }
    }

    /// Nombre estable del tipo de marco, para registros y mensajes de error.
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Data { .. } => "datos",
            Self::End { .. } => "fin",
            Self::Abort { .. } => "aborto",
        }
    }

    /// Serializa el marco listo para enviar por el canal de datos.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let payload_len = match self {
            Self::Data { payload, .. } => payload.len(),
            Self::End { .. } => DIGEST_BYTES,
            Self::Abort { .. } => 0,
        };
        let mut out = Vec::with_capacity(HEADER_BYTES + payload_len);

        out.push(FRAME_VERSION);
        out.push(match self {
            Self::Data { .. } => KIND_DATA,
            Self::End { .. } => KIND_END,
            Self::Abort { .. } => KIND_ABORT,
        });
        out.extend_from_slice(self.transfer_id().as_uuid().as_bytes());

        let sequence = match self {
            Self::Data { sequence, .. } => *sequence,
            Self::End { frames, .. } => *frames,
            Self::Abort { .. } => 0,
        };
        out.extend_from_slice(&sequence.to_be_bytes());

        match self {
            Self::Data { payload, .. } => out.extend_from_slice(payload),
            Self::End { digest, .. } => out.extend_from_slice(digest),
            Self::Abort { .. } => {}
        }

        out
    }

    /// Analiza un marco recibido por el canal de datos.
    ///
    /// # Errors
    ///
    /// [`FileError::MalformedFrame`] con el motivo concreto. Nunca entra en
    /// panico ni reserva memoria en funcion de un valor recibido.
    pub fn parse(raw: &[u8]) -> FileResult<Self> {
        if raw.len() > MAX_FRAME_BYTES {
            return Err(FileError::MalformedFrame {
                reason: "el marco supera el tamano maximo",
            });
        }
        if raw.len() < HEADER_BYTES {
            return Err(FileError::MalformedFrame {
                reason: "el marco es mas corto que la cabecera",
            });
        }
        if raw[0] != FRAME_VERSION {
            return Err(FileError::MalformedFrame {
                reason: "version de formato desconocida",
            });
        }

        // Los indices son constantes y ya se comprobo la longitud minima.
        let mut id_bytes = [0_u8; 16];
        id_bytes.copy_from_slice(&raw[2..18]);
        let transfer_id = TransferId::from_uuid(Uuid::from_bytes(id_bytes));

        let mut seq_bytes = [0_u8; 4];
        seq_bytes.copy_from_slice(&raw[18..22]);
        let sequence = u32::from_be_bytes(seq_bytes);

        let payload = &raw[HEADER_BYTES..];

        match raw[1] {
            KIND_DATA => {
                if payload.is_empty() {
                    // Un trozo vacio no dice nada y permitiria enviar marcos
                    // sin fin sin avanzar la transferencia.
                    return Err(FileError::MalformedFrame {
                        reason: "un marco de datos no puede venir vacio",
                    });
                }
                Ok(Self::Data {
                    transfer_id,
                    sequence,
                    payload: payload.to_vec(),
                })
            }
            KIND_END => {
                let digest: [u8; DIGEST_BYTES] =
                    payload.try_into().map_err(|_| FileError::MalformedFrame {
                        reason: "el marco de fin no lleva un SHA-256 de 32 bytes",
                    })?;
                Ok(Self::End {
                    transfer_id,
                    frames: sequence,
                    digest,
                })
            }
            KIND_ABORT => {
                if !payload.is_empty() {
                    return Err(FileError::MalformedFrame {
                        reason: "el marco de aborto no lleva carga",
                    });
                }
                Ok(Self::Abort { transfer_id })
            }
            _ => Err(FileError::MalformedFrame {
                reason: "tipo de marco desconocido",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> TransferId {
        TransferId::from_uuid(Uuid::from_bytes([7_u8; 16]))
    }

    fn data(payload: Vec<u8>) -> Frame {
        Frame::Data {
            transfer_id: id(),
            sequence: 3,
            payload,
        }
    }

    #[test]
    fn a_data_frame_roundtrips() {
        let frame = data(b"hola".to_vec());
        let back = Frame::parse(&frame.encode()).expect("analiza");
        assert_eq!(back, frame);
        assert_eq!(back.transfer_id(), id());
        assert_eq!(back.kind_name(), "datos");
    }

    #[test]
    fn an_end_frame_roundtrips() {
        let frame = Frame::End {
            transfer_id: id(),
            frames: 1_234,
            digest: [9_u8; DIGEST_BYTES],
        };
        let back = Frame::parse(&frame.encode()).expect("analiza");
        assert_eq!(back, frame);
        assert_eq!(back.kind_name(), "fin");
    }

    #[test]
    fn an_abort_frame_roundtrips() {
        let frame = Frame::Abort { transfer_id: id() };
        let back = Frame::parse(&frame.encode()).expect("analiza");
        assert_eq!(back, frame);
        assert_eq!(back.kind_name(), "aborto");
    }

    #[test]
    fn the_header_has_the_documented_layout() {
        let raw = data(b"xy".to_vec()).encode();
        assert_eq!(raw.len(), HEADER_BYTES + 2);
        assert_eq!(raw[0], FRAME_VERSION);
        assert_eq!(raw[1], KIND_DATA);
        assert_eq!(&raw[2..18], &[7_u8; 16]);
        assert_eq!(&raw[18..22], &3_u32.to_be_bytes());
        assert_eq!(&raw[22..], b"xy");
    }

    #[test]
    fn a_full_frame_fits_in_a_data_channel_message() {
        // Que quepa ya lo impide la comprobacion en tiempo de compilacion.
        // Esto deja escrito el margen que queda, que es lo que se perderia de
        // vista si alguien subiera el tamano del trozo sin mirar.
        assert_eq!(SCTP_MAX_MESSAGE_BYTES - MAX_FRAME_BYTES, 1_002);
    }

    #[test]
    fn a_full_sized_chunk_fits_exactly_in_the_maximum_frame() {
        let frame = data(vec![0_u8; CHUNK_SIZE_BYTES]);
        let raw = frame.encode();
        assert_eq!(raw.len(), MAX_FRAME_BYTES);
        assert_eq!(Frame::parse(&raw).expect("analiza"), frame);
    }

    #[test]
    fn a_chunk_over_the_maximum_is_refused_without_being_copied() {
        let raw = data(vec![0_u8; CHUNK_SIZE_BYTES + 1]).encode();
        assert_eq!(
            Frame::parse(&raw).unwrap_err(),
            FileError::MalformedFrame {
                reason: "el marco supera el tamano maximo"
            }
        );
    }

    #[test]
    fn frames_shorter_than_the_header_are_refused() {
        for len in 0..HEADER_BYTES {
            let raw = vec![FRAME_VERSION; len];
            assert_eq!(
                Frame::parse(&raw).unwrap_err(),
                FileError::MalformedFrame {
                    reason: "el marco es mas corto que la cabecera"
                },
                "longitud {len} deberia rechazarse"
            );
        }
    }

    #[test]
    fn an_unknown_version_is_refused() {
        let mut raw = data(b"x".to_vec()).encode();
        raw[0] = FRAME_VERSION + 1;
        assert_eq!(
            Frame::parse(&raw).unwrap_err(),
            FileError::MalformedFrame {
                reason: "version de formato desconocida"
            }
        );
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        let mut raw = data(b"x".to_vec()).encode();
        raw[1] = 99;
        assert_eq!(
            Frame::parse(&raw).unwrap_err(),
            FileError::MalformedFrame {
                reason: "tipo de marco desconocido"
            }
        );
    }

    #[test]
    fn an_empty_data_frame_is_refused() {
        let raw = data(Vec::new()).encode();
        assert_eq!(
            Frame::parse(&raw).unwrap_err(),
            FileError::MalformedFrame {
                reason: "un marco de datos no puede venir vacio"
            }
        );
    }

    #[test]
    fn an_end_frame_with_the_wrong_digest_length_is_refused() {
        for len in [0_usize, 1, 31, 33, 64] {
            let mut raw = Frame::Abort { transfer_id: id() }.encode();
            raw[1] = KIND_END;
            raw.extend(std::iter::repeat_n(0_u8, len));
            assert_eq!(
                Frame::parse(&raw).unwrap_err(),
                FileError::MalformedFrame {
                    reason: "el marco de fin no lleva un SHA-256 de 32 bytes"
                },
                "un digest de {len} bytes deberia rechazarse"
            );
        }
    }

    #[test]
    fn an_abort_frame_carrying_payload_is_refused() {
        let mut raw = Frame::Abort { transfer_id: id() }.encode();
        raw.push(0);
        assert_eq!(
            Frame::parse(&raw).unwrap_err(),
            FileError::MalformedFrame {
                reason: "el marco de aborto no lleva carga"
            }
        );
    }

    #[test]
    fn every_byte_pattern_of_the_header_is_handled_without_panicking() {
        // Barrido del tipo y de la version: lo que importa es que ninguna
        // combinacion entre en panico, no que se acepte.
        for version in 0_u8..=3 {
            for kind in 0_u8..=5 {
                let mut raw = vec![0_u8; HEADER_BYTES + 4];
                raw[0] = version;
                raw[1] = kind;
                let _ = Frame::parse(&raw);
            }
        }
    }

    #[test]
    fn the_sequence_number_survives_its_full_range() {
        for sequence in [0_u32, 1, u32::MAX / 2, u32::MAX] {
            let frame = Frame::Data {
                transfer_id: id(),
                sequence,
                payload: b"z".to_vec(),
            };
            assert_eq!(Frame::parse(&frame.encode()).expect("analiza"), frame);
        }
    }
}
