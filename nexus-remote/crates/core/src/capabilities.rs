//! Negociacion de capacidades entre host y controlador.
//!
//! Host y controlador se anuncian lo que saben hacer y la sesion usa la
//! interseccion. Asi una version nueva del host puede hablar con un cliente
//! antiguo sin romperse: simplemente no se activan las funciones que el otro
//! extremo no conoce.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Funcion opcional que un extremo puede soportar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Transmision de video de la pantalla.
    ScreenStream,
    /// Mas de un monitor seleccionable.
    MultiMonitor,
    /// Cambio de resolucion del monitor remoto.
    ResolutionChange,
    /// Inyeccion de raton y teclado.
    RemoteInput,
    /// Gestos tactiles traducidos a raton.
    TouchGestures,
    /// Audio del sistema.
    AudioStream,
    /// Explorador y transferencia de archivos.
    FileTransfer,
    /// Sincronizacion de portapapeles de texto.
    ClipboardText,
    /// Operaciones de energia (bloquear, reiniciar, apagar).
    PowerControl,
    /// Codec de video H.264.
    CodecH264,
    /// Codec de video VP8.
    CodecVp8,
    /// Codec de video AV1.
    CodecAv1,
    /// Capacidad anunciada por una version mas nueva del protocolo.
    #[serde(other)]
    Unknown,
}

impl Capability {
    /// Capacidades que el protocolo considera imprescindibles.
    ///
    /// Si la interseccion negociada no las contiene, la sesion no es util y se
    /// rechaza en lugar de abrir una pantalla negra.
    pub const REQUIRED: [Self; 1] = [Self::ScreenStream];
}

/// Conjunto de capacidades anunciado por un extremo.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    /// Conjunto vacio.
    #[must_use]
    pub const fn empty() -> Self {
        Self(BTreeSet::new())
    }

    /// Comprueba si la capacidad esta presente.
    #[must_use]
    pub fn has(&self, capability: Capability) -> bool {
        self.0.contains(&capability)
    }

    /// Anade una capacidad.
    pub fn insert(&mut self, capability: Capability) -> bool {
        self.0.insert(capability)
    }

    /// Numero de capacidades.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Indica si el conjunto esta vacio.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Itera en orden estable.
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }

    /// Calcula las capacidades efectivas de la sesion.
    ///
    /// `Unknown` se descarta siempre: representa algo que esta version no sabe
    /// manejar, asi que anunciarlo como activo seria mentir.
    #[must_use]
    pub fn negotiate(&self, other: &Self) -> Self {
        Self(
            self.0
                .intersection(&other.0)
                .copied()
                .filter(|c| *c != Capability::Unknown)
                .collect(),
        )
    }

    /// Indica si el conjunto cubre las capacidades imprescindibles.
    #[must_use]
    pub fn is_viable(&self) -> bool {
        Capability::REQUIRED.iter().all(|c| self.has(*c))
    }

    /// Primer codec de video comun, por orden de preferencia.
    ///
    /// H.264 va primero porque es el unico con decodificacion acelerada por
    /// hardware garantizada en iPhone y iPad.
    #[must_use]
    pub fn preferred_video_codec(&self) -> Option<Capability> {
        [
            Capability::CodecH264,
            Capability::CodecAv1,
            Capability::CodecVp8,
        ]
        .into_iter()
        .find(|c| self.has(*c))
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<T: IntoIterator<Item = Capability>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_is_the_intersection() {
        let host = CapabilitySet::from_iter([
            Capability::ScreenStream,
            Capability::AudioStream,
            Capability::FileTransfer,
            Capability::CodecH264,
        ]);
        let client = CapabilitySet::from_iter([
            Capability::ScreenStream,
            Capability::CodecH264,
            Capability::TouchGestures,
        ]);

        let effective = host.negotiate(&client);
        assert!(effective.has(Capability::ScreenStream));
        assert!(effective.has(Capability::CodecH264));
        assert!(
            !effective.has(Capability::AudioStream),
            "el cliente no lo anuncio"
        );
        assert!(
            !effective.has(Capability::TouchGestures),
            "el host no lo anuncio"
        );
        assert_eq!(effective.len(), 2);
    }

    #[test]
    fn negotiation_is_commutative() {
        let a = CapabilitySet::from_iter([Capability::ScreenStream, Capability::AudioStream]);
        let b = CapabilitySet::from_iter([Capability::AudioStream, Capability::FileTransfer]);
        assert_eq!(a.negotiate(&b), b.negotiate(&a));
    }

    #[test]
    fn unknown_capability_is_never_activated() {
        let a = CapabilitySet::from_iter([Capability::ScreenStream, Capability::Unknown]);
        let b = CapabilitySet::from_iter([Capability::ScreenStream, Capability::Unknown]);
        let effective = a.negotiate(&b);
        assert!(!effective.has(Capability::Unknown));
        assert_eq!(effective.len(), 1);
    }

    #[test]
    fn future_capability_names_deserialize_as_unknown() {
        let parsed: CapabilitySet =
            serde_json::from_str("[\"screen_stream\",\"holografia_3d\"]").expect("tolerante");
        assert!(parsed.has(Capability::ScreenStream));
        assert!(parsed.has(Capability::Unknown));
    }

    #[test]
    fn session_without_screen_stream_is_not_viable() {
        let effective = CapabilitySet::from_iter([Capability::AudioStream]);
        assert!(!effective.is_viable());
        assert!(CapabilitySet::from_iter([Capability::ScreenStream]).is_viable());
    }

    #[test]
    fn h264_wins_over_other_codecs() {
        let set = CapabilitySet::from_iter([
            Capability::CodecVp8,
            Capability::CodecH264,
            Capability::CodecAv1,
        ]);
        assert_eq!(set.preferred_video_codec(), Some(Capability::CodecH264));

        let set = CapabilitySet::from_iter([Capability::CodecVp8, Capability::CodecAv1]);
        assert_eq!(set.preferred_video_codec(), Some(Capability::CodecAv1));

        assert_eq!(CapabilitySet::empty().preferred_video_codec(), None);
    }
}
