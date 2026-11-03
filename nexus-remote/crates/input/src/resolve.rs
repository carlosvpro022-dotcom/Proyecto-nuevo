//! Resolucion de eventos del protocolo a ordenes absolutas del escritorio.
//!
//! El controlador envia coordenadas normalizadas (0.0 a 1.0) relativas al
//! monitor que esta viendo. Aqui se convierten a coordenadas del escritorio
//! virtual de Windows, que es lo que entiende `SendInput`.
//!
//! Todo evento que llega de la red pasa por aqui antes de tocar el sistema.
//! Las coordenadas se acotan y los nombres de tecla se validan: un cliente
//! manipulado no puede apuntar fuera del monitor compartido ni colar una cadena
//! arbitraria en la tabla de teclas.

use nexus_core::monitor::MonitorLayout;
use nexus_core::protocol::input::{
    InputEvent, KeyCode, KeyState, Modifiers, PointerButton, ScrollDelta,
};

use crate::error::{InputError, InputResult};

/// Orden de entrada ya validada y en coordenadas absolutas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedInput {
    /// Mover el cursor a un punto del escritorio virtual.
    MoveCursor {
        /// Coordenada X absoluta.
        x: i32,
        /// Coordenada Y absoluta.
        y: i32,
    },
    /// Pulsar o liberar un boton del raton.
    Button {
        /// Boton afectado.
        button: PointerButton,
        /// Estado al que pasa.
        state: KeyState,
    },
    /// Desplazar la rueda.
    Scroll {
        /// Magnitud del desplazamiento.
        delta: ScrollDelta,
    },
    /// Pulsar o liberar una tecla.
    Key {
        /// Tecla afectada.
        code: KeyCode,
        /// Estado al que pasa.
        state: KeyState,
        /// Modificadores activos.
        modifiers: Modifiers,
    },
    /// Escribir texto literal.
    Text {
        /// Texto a escribir.
        text: String,
    },
    /// Ejecutar una combinacion completa.
    Chord {
        /// Modificadores a mantener.
        modifiers: Modifiers,
        /// Teclas a pulsar.
        keys: Vec<KeyCode>,
    },
}

/// Traduce un evento del protocolo a una orden absoluta.
///
/// # Errors
///
/// - [`InputError::InvalidEvent`] si el evento no pasa la validacion
///   estructural declarada en el protocolo.
/// - [`InputError::UnknownMonitor`] si el monitor activo no existe en la
///   disposicion actual, lo que ocurre si el usuario desconecta una pantalla
///   mientras hay una sesion abierta.
pub fn resolve(
    event: &InputEvent,
    layout: &MonitorLayout,
    active_monitor: u8,
) -> InputResult<ResolvedInput> {
    if !event.is_valid() {
        return Err(InputError::InvalidEvent(format!("{event:?}")));
    }

    match event {
        InputEvent::PointerMove { x, y } => {
            let monitor = layout
                .get(active_monitor)
                .ok_or(InputError::UnknownMonitor {
                    index: active_monitor,
                })?;
            let (vx, vy) = monitor.normalized_to_virtual(*x, *y);
            Ok(ResolvedInput::MoveCursor { x: vx, y: vy })
        }
        InputEvent::PointerButton { button, state } => Ok(ResolvedInput::Button {
            button: *button,
            state: *state,
        }),
        InputEvent::Scroll { delta } => Ok(ResolvedInput::Scroll { delta: *delta }),
        InputEvent::Key {
            code,
            state,
            modifiers,
        } => Ok(ResolvedInput::Key {
            code: code.clone(),
            state: *state,
            modifiers: *modifiers,
        }),
        InputEvent::Text { text } => Ok(ResolvedInput::Text { text: text.clone() }),
        InputEvent::Chord { modifiers, keys } => Ok(ResolvedInput::Chord {
            modifiers: *modifiers,
            keys: keys.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::monitor::MonitorInfo;

    fn layout() -> MonitorLayout {
        MonitorLayout(vec![
            MonitorInfo {
                index: 0,
                name: "Principal".to_owned(),
                width: 1_920,
                height: 1_080,
                origin_x: 0,
                origin_y: 0,
                scale_percent: 100,
                is_primary: true,
                refresh_hz: Some(60),
            },
            MonitorInfo {
                index: 1,
                name: "Secundario".to_owned(),
                width: 2_560,
                height: 1_440,
                origin_x: -2_560,
                origin_y: 100,
                scale_percent: 100,
                is_primary: false,
                refresh_hz: Some(144),
            },
        ])
    }

    #[test]
    fn pointer_move_lands_on_the_active_monitor() {
        let resolved =
            resolve(&InputEvent::PointerMove { x: 0.5, y: 0.5 }, &layout(), 0).expect("resuelve");
        assert_eq!(resolved, ResolvedInput::MoveCursor { x: 960, y: 540 });
    }

    #[test]
    fn the_same_event_maps_differently_per_monitor() {
        let event = InputEvent::PointerMove { x: 0.0, y: 0.0 };
        assert_eq!(
            resolve(&event, &layout(), 0).expect("monitor 0"),
            ResolvedInput::MoveCursor { x: 0, y: 0 }
        );
        assert_eq!(
            resolve(&event, &layout(), 1).expect("monitor 1"),
            ResolvedInput::MoveCursor { x: -2_560, y: 100 },
            "el monitor secundario tiene origen propio"
        );
    }

    #[test]
    fn the_far_corner_stays_inside_the_monitor() {
        let resolved =
            resolve(&InputEvent::PointerMove { x: 1.0, y: 1.0 }, &layout(), 1).expect("resuelve");
        assert_eq!(resolved, ResolvedInput::MoveCursor { x: -1, y: 1_539 });
    }

    #[test]
    fn an_out_of_range_coordinate_is_rejected_not_clamped() {
        // El protocolo declara 0.0..=1.0; algo fuera de rango es un cliente
        // mal implementado o manipulado, y se rechaza en lugar de adivinar.
        let err = resolve(&InputEvent::PointerMove { x: 5.0, y: 0.5 }, &layout(), 0).unwrap_err();
        assert!(matches!(err, InputError::InvalidEvent(_)), "{err:?}");
    }

    #[test]
    fn non_finite_coordinates_are_rejected() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(resolve(&InputEvent::PointerMove { x: bad, y: 0.0 }, &layout(), 0).is_err());
        }
    }

    #[test]
    fn a_missing_monitor_is_reported() {
        let err = resolve(&InputEvent::PointerMove { x: 0.5, y: 0.5 }, &layout(), 7).unwrap_err();
        assert_eq!(err, InputError::UnknownMonitor { index: 7 });
    }

    #[test]
    fn an_empty_layout_cannot_resolve_pointer_moves() {
        let err = resolve(
            &InputEvent::PointerMove { x: 0.5, y: 0.5 },
            &MonitorLayout::default(),
            0,
        )
        .unwrap_err();
        assert_eq!(err, InputError::UnknownMonitor { index: 0 });
    }

    #[test]
    fn button_and_scroll_do_not_need_a_monitor() {
        let empty = MonitorLayout::default();
        resolve(
            &InputEvent::PointerButton {
                button: PointerButton::Left,
                state: KeyState::Down,
            },
            &empty,
            0,
        )
        .expect("los botones no dependen del monitor");
        resolve(
            &InputEvent::Scroll {
                delta: ScrollDelta { x: 0, y: -3 },
            },
            &empty,
            0,
        )
        .expect("el scroll tampoco");
    }

    #[test]
    fn keys_keep_their_modifiers() {
        let event = InputEvent::Key {
            code: KeyCode("KeyS".into()),
            state: KeyState::Down,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::NONE
            },
        };
        let resolved = resolve(&event, &layout(), 0).expect("resuelve");
        assert_eq!(
            resolved,
            ResolvedInput::Key {
                code: KeyCode("KeyS".into()),
                state: KeyState::Down,
                modifiers: Modifiers {
                    ctrl: true,
                    ..Modifiers::NONE
                },
            }
        );
    }

    #[test]
    fn a_malformed_key_name_never_reaches_the_platform() {
        let event = InputEvent::Key {
            code: KeyCode("'; DROP TABLE".into()),
            state: KeyState::Down,
            modifiers: Modifiers::NONE,
        };
        assert!(matches!(
            resolve(&event, &layout(), 0).unwrap_err(),
            InputError::InvalidEvent(_)
        ));
    }

    #[test]
    fn empty_text_is_rejected() {
        assert!(
            resolve(
                &InputEvent::Text {
                    text: String::new()
                },
                &layout(),
                0
            )
            .is_err()
        );
    }

    #[test]
    fn oversized_text_is_rejected() {
        let text = "a".repeat(InputEvent::MAX_TEXT_LEN + 1);
        assert!(resolve(&InputEvent::Text { text }, &layout(), 0).is_err());
    }

    #[test]
    fn chords_are_passed_through_intact() {
        let event = InputEvent::Chord {
            modifiers: Modifiers {
                ctrl: true,
                alt: true,
                ..Modifiers::NONE
            },
            keys: vec![KeyCode("Delete".into())],
        };
        let resolved = resolve(&event, &layout(), 0).expect("resuelve");
        assert_eq!(
            resolved,
            ResolvedInput::Chord {
                modifiers: Modifiers {
                    ctrl: true,
                    alt: true,
                    ..Modifiers::NONE
                },
                keys: vec![KeyCode("Delete".into())],
            }
        );
    }

    #[test]
    fn an_oversized_chord_is_rejected() {
        let event = InputEvent::Chord {
            modifiers: Modifiers::NONE,
            keys: (0..20).map(|i| KeyCode(format!("Key{i}"))).collect(),
        };
        assert!(resolve(&event, &layout(), 0).is_err());
    }
}
