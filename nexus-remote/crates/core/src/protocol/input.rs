//! Tipos de entrada tal como viajan por el cable.
//!
//! Son independientes de la plataforma a proposito: el cliente iOS no conoce
//! los codigos de Windows y el host no conoce los gestos de iPadOS. La
//! traduccion a `SendInput` vive en `nexus-input`.

use serde::{Deserialize, Serialize};

/// Boton del raton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    /// Boton principal.
    Left,
    /// Boton secundario.
    Right,
    /// Boton central o rueda pulsada.
    Middle,
}

/// Estado de una tecla o boton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyState {
    /// Pulsado.
    Down,
    /// Liberado.
    Up,
}

/// Modificadores activos en el momento del evento.
// Los cuatro modificadores son el formato de cable, calcado de los campos de
// `KeyboardEvent` del navegador: un mapa de bits seria mas compacto pero
// ilegible al depurar la sesion.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Modifiers {
    /// Mayusculas.
    #[serde(default)]
    pub shift: bool,
    /// Control.
    #[serde(default)]
    pub ctrl: bool,
    /// Alt.
    #[serde(default)]
    pub alt: bool,
    /// Tecla Windows (equivale a Command en el teclado Apple).
    #[serde(default)]
    pub meta: bool,
}

impl Modifiers {
    /// Sin ningun modificador.
    pub const NONE: Self = Self {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    };

    /// Indica si hay algun modificador activo.
    #[must_use]
    pub const fn any(self) -> bool {
        self.shift || self.ctrl || self.alt || self.meta
    }
}

/// Tecla identificada de forma independiente de la distribucion.
///
/// Se usan nombres estilo `KeyboardEvent.code` del navegador: identifican la
/// posicion fisica, no el caracter. Para texto se usa
/// [`InputEvent::Text`], que respeta la distribucion del teclado del cliente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyCode(pub String);

impl KeyCode {
    /// Longitud maxima admitida para un nombre de tecla.
    pub const MAX_LEN: usize = 24;

    /// Comprueba que el nombre tiene una forma razonable.
    ///
    /// Solo se aceptan caracteres alfanumericos ASCII: es lo que usan los
    /// nombres tipo `KeyA`, `F11`, `ArrowLeft` o `Digit3`. Validar aqui evita
    /// que una cadena arbitraria llegue a la tabla de traduccion del host.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        !self.0.is_empty()
            && self.0.len() <= Self::MAX_LEN
            && self.0.chars().all(|c| c.is_ascii_alphanumeric())
    }
}

/// Direccion y magnitud de un desplazamiento de rueda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrollDelta {
    /// Desplazamiento horizontal en "clics" de rueda.
    pub x: i16,
    /// Desplazamiento vertical en "clics" de rueda.
    pub y: i16,
}

/// Evento de entrada enviado del controlador al host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum InputEvent {
    /// Mueve el cursor a una posicion normalizada del monitor activo.
    ///
    /// Se usan coordenadas de 0.0 a 1.0 en lugar de pixeles para que el mismo
    /// evento valga sea cual sea la resolucion del monitor y el tamano de la
    /// pantalla del iPad.
    PointerMove {
        /// Posicion horizontal normalizada.
        x: f32,
        /// Posicion vertical normalizada.
        y: f32,
    },
    /// Pulsa o libera un boton del raton.
    PointerButton {
        /// Boton afectado.
        button: PointerButton,
        /// Estado al que pasa.
        state: KeyState,
    },
    /// Desplaza la rueda.
    Scroll {
        /// Magnitud del desplazamiento.
        delta: ScrollDelta,
    },
    /// Pulsa o libera una tecla concreta.
    Key {
        /// Tecla afectada.
        code: KeyCode,
        /// Estado al que pasa.
        state: KeyState,
        /// Modificadores activos.
        #[serde(default)]
        modifiers: Modifiers,
    },
    /// Escribe texto literal, respetando la distribucion del cliente.
    Text {
        /// Texto a escribir.
        text: String,
    },
    /// Ejecuta una combinacion completa (pulsar todo, liberar todo).
    ///
    /// Existe como evento propio porque una combinacion partida en eventos
    /// sueltos puede perder sincronia si la red reordena o descarta: aqui el
    /// host garantiza que las teclas se liberan aunque falle algo.
    Chord {
        /// Modificadores a mantener pulsados.
        modifiers: Modifiers,
        /// Teclas a pulsar mientras los modificadores estan activos.
        keys: Vec<KeyCode>,
    },
}

impl InputEvent {
    /// Longitud maxima de un bloque de texto en un solo evento.
    pub const MAX_TEXT_LEN: usize = 4_096;
    /// Numero maximo de teclas en una combinacion.
    pub const MAX_CHORD_KEYS: usize = 6;

    /// Comprueba que el evento es estructuralmente valido.
    ///
    /// Esta validacion corre en el host sobre todo evento recibido, antes de
    /// tocar la API del sistema.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self {
            Self::PointerMove { x, y } => {
                x.is_finite() && y.is_finite() && (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)
            }
            Self::PointerButton { .. } | Self::Scroll { .. } => true,
            Self::Key { code, .. } => code.is_well_formed(),
            Self::Text { text } => !text.is_empty() && text.len() <= Self::MAX_TEXT_LEN,
            Self::Chord { keys, .. } => {
                !keys.is_empty()
                    && keys.len() <= Self::MAX_CHORD_KEYS
                    && keys.iter().all(KeyCode::is_well_formed)
            }
        }
    }

    /// Indica si el evento necesita el permiso de teclado en lugar de raton.
    #[must_use]
    pub const fn needs_keyboard_permission(&self) -> bool {
        matches!(
            self,
            Self::Key { .. } | Self::Text { .. } | Self::Chord { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_move_inside_range_is_valid() {
        assert!(InputEvent::PointerMove { x: 0.0, y: 0.0 }.is_valid());
        assert!(InputEvent::PointerMove { x: 1.0, y: 1.0 }.is_valid());
        assert!(InputEvent::PointerMove { x: 0.5, y: 0.25 }.is_valid());
    }

    #[test]
    fn pointer_move_outside_range_is_rejected() {
        assert!(!InputEvent::PointerMove { x: -0.1, y: 0.5 }.is_valid());
        assert!(!InputEvent::PointerMove { x: 1.5, y: 0.5 }.is_valid());
    }

    #[test]
    fn non_finite_coordinates_are_rejected() {
        assert!(
            !InputEvent::PointerMove {
                x: f32::NAN,
                y: 0.0
            }
            .is_valid()
        );
        assert!(
            !InputEvent::PointerMove {
                x: f32::INFINITY,
                y: 0.0
            }
            .is_valid()
        );
    }

    #[test]
    fn key_codes_are_restricted_to_ascii_alphanumeric() {
        assert!(KeyCode("KeyA".into()).is_well_formed());
        assert!(KeyCode("F11".into()).is_well_formed());
        assert!(KeyCode("ArrowLeft".into()).is_well_formed());
        assert!(!KeyCode(String::new()).is_well_formed());
        assert!(!KeyCode("Key A".into()).is_well_formed());
        assert!(!KeyCode("rm -rf /".into()).is_well_formed());
        assert!(!KeyCode("a".repeat(100)).is_well_formed());
    }

    #[test]
    fn empty_text_is_rejected_and_long_text_is_capped() {
        assert!(
            !InputEvent::Text {
                text: String::new()
            }
            .is_valid()
        );
        assert!(
            InputEvent::Text {
                text: "hola".into()
            }
            .is_valid()
        );
        assert!(
            !InputEvent::Text {
                text: "a".repeat(InputEvent::MAX_TEXT_LEN + 1)
            }
            .is_valid()
        );
    }

    #[test]
    fn chord_must_have_between_one_and_six_keys() {
        let ok = InputEvent::Chord {
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::NONE
            },
            keys: vec![KeyCode("KeyC".into())],
        };
        assert!(ok.is_valid());

        let empty = InputEvent::Chord {
            modifiers: Modifiers::NONE,
            keys: vec![],
        };
        assert!(!empty.is_valid());

        let too_many = InputEvent::Chord {
            modifiers: Modifiers::NONE,
            keys: (0..9).map(|i| KeyCode(format!("Key{i}"))).collect(),
        };
        assert!(!too_many.is_valid());
    }

    #[test]
    fn keyboard_events_are_flagged_for_the_right_permission() {
        assert!(InputEvent::Text { text: "x".into() }.needs_keyboard_permission());
        assert!(
            InputEvent::Key {
                code: KeyCode("KeyA".into()),
                state: KeyState::Down,
                modifiers: Modifiers::NONE,
            }
            .needs_keyboard_permission()
        );
        assert!(!InputEvent::PointerMove { x: 0.1, y: 0.1 }.needs_keyboard_permission());
        assert!(
            !InputEvent::Scroll {
                delta: ScrollDelta { x: 0, y: -3 }
            }
            .needs_keyboard_permission()
        );
    }

    #[test]
    fn modifiers_default_to_none_when_absent_in_json() {
        let raw = r#"{"kind":"key","code":"KeyA","state":"down"}"#;
        let event: InputEvent = serde_json::from_str(raw).expect("deserializa");
        assert_eq!(
            event,
            InputEvent::Key {
                code: KeyCode("KeyA".into()),
                state: KeyState::Down,
                modifiers: Modifiers::NONE,
            }
        );
    }

    #[test]
    fn events_roundtrip_through_json() {
        let events = vec![
            InputEvent::PointerMove { x: 0.25, y: 0.75 },
            InputEvent::PointerButton {
                button: PointerButton::Right,
                state: KeyState::Down,
            },
            InputEvent::Scroll {
                delta: ScrollDelta { x: 0, y: 3 },
            },
            InputEvent::Text {
                text: "NEXUS".into(),
            },
            InputEvent::Chord {
                modifiers: Modifiers {
                    ctrl: true,
                    alt: true,
                    ..Modifiers::NONE
                },
                keys: vec![KeyCode("Delete".into())],
            },
        ];
        for event in events {
            let raw = serde_json::to_string(&event).expect("serializa");
            let back: InputEvent = serde_json::from_str(&raw).expect("deserializa");
            assert_eq!(event, back, "fallo con {raw}");
        }
    }
}
