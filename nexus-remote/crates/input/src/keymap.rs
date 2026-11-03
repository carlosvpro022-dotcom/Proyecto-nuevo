//! Traduccion de nombres de tecla del protocolo a codigos virtuales de Windows.
//!
//! El protocolo identifica las teclas por su **posicion fisica**, con los
//! nombres de `KeyboardEvent.code` del navegador (`KeyA`, `Digit3`,
//! `ArrowLeft`). Windows las identifica por su **codigo virtual** (`VK_A`,
//! `VK_LEFT`). Esta tabla es el puente.
//!
//! ## Por que la posicion y no el caracter
//!
//! Si el protocolo enviara caracteres, una combinacion como Ctrl+Z dejaria de
//! funcionar al cambiar de distribucion: en un teclado AZERTY la tecla que
//! produce `z` esta donde en QWERTY esta la `w`. Enviando la posicion, Ctrl+Z
//! cae siempre en la misma tecla fisica, que es lo que la persona espera.
//!
//! Para escribir texto se usa otro camino,
//! [`InputEvent::Text`](nexus_core::protocol::input::InputEvent::Text), que si
//! respeta la distribucion del cliente.
//!
//! ## La marca de tecla extendida
//!
//! Algunas teclas comparten codigo virtual con las del teclado numerico y solo
//! se distinguen por el bit `KEYEVENTF_EXTENDEDKEY`. Sin el, la flecha
//! izquierda se confunde con el `4` del numerico cuando Bloq Num esta
//! apagado, y Ctrl derecho se comporta como el izquierdo. Cada entrada de la
//! tabla declara si es extendida.
//!
//! Este modulo **no depende de Windows**: es una tabla de datos y se prueba en
//! cualquier plataforma. El backend de Windows solo la consulta.

use nexus_core::protocol::input::{KeyCode, Modifiers};

/// Codigo virtual de Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VirtualKey(pub u16);

impl VirtualKey {
    /// Mayusculas izquierda. `VK_LSHIFT`.
    pub const LSHIFT: Self = Self(0xA0);
    /// Control izquierdo. `VK_LCONTROL`.
    pub const LCONTROL: Self = Self(0xA2);
    /// Alt izquierdo. `VK_LMENU`.
    pub const LMENU: Self = Self(0xA4);
    /// Tecla Windows izquierda. `VK_LWIN`.
    pub const LWIN: Self = Self(0x5B);
}

/// Una tecla traducida, lista para `SendInput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyMapping {
    /// Codigo virtual de Windows.
    pub vk: VirtualKey,
    /// Si necesita la marca de tecla extendida.
    pub extended: bool,
}

impl KeyMapping {
    const fn plain(vk: u16) -> Self {
        Self {
            vk: VirtualKey(vk),
            extended: false,
        }
    }

    const fn extended(vk: u16) -> Self {
        Self {
            vk: VirtualKey(vk),
            extended: true,
        }
    }
}

/// Traduce un nombre de tecla del protocolo.
///
/// Devuelve `None` si el nombre no corresponde a ninguna tecla conocida. El
/// llamante debe tratarlo como error, no ignorarlo en silencio: una tecla sin
/// traducir es una orden que la persona dio y no se ejecuto.
#[must_use]
#[allow(clippy::too_many_lines)] // Es una tabla de datos; partirla la haria peor.
pub fn lookup(code: &KeyCode) -> Option<KeyMapping> {
    // Las letras y los digitos de la fila superior siguen el codigo ASCII de
    // su mayuscula, que es como los define Windows.
    if let Some(letter) = code.0.strip_prefix("Key")
        && letter.len() == 1
        && let Some(c) = letter.chars().next()
        && c.is_ascii_uppercase()
    {
        return Some(KeyMapping::plain(c as u16));
    }
    if let Some(digit) = code.0.strip_prefix("Digit")
        && digit.len() == 1
        && let Some(c) = digit.chars().next()
        && c.is_ascii_digit()
    {
        return Some(KeyMapping::plain(c as u16));
    }
    if let Some(number) = code.0.strip_prefix("Numpad")
        && number.len() == 1
        && let Some(c) = number.chars().next()
        && c.is_ascii_digit()
    {
        // VK_NUMPAD0 = 0x60.
        return Some(KeyMapping::plain(0x60 + (c as u16 - u16::from(b'0'))));
    }
    if let Some(number) = code.0.strip_prefix('F')
        && let Ok(n) = number.parse::<u16>()
        && (1..=24).contains(&n)
    {
        // VK_F1 = 0x70.
        return Some(KeyMapping::plain(0x6F + n));
    }

    let mapping = match code.0.as_str() {
        // --- control basico ---
        "Escape" => KeyMapping::plain(0x1B),
        "Tab" => KeyMapping::plain(0x09),
        "CapsLock" => KeyMapping::plain(0x14),
        "Space" => KeyMapping::plain(0x20),
        "Enter" => KeyMapping::plain(0x0D),
        "Backspace" => KeyMapping::plain(0x08),

        // --- modificadores ---
        // Izquierdos y derechos tienen codigo propio, y los derechos son
        // extendidos: sin la marca, AltGr no funcionaria.
        "ShiftLeft" => KeyMapping::plain(0xA0),
        "ShiftRight" => KeyMapping::extended(0xA1),
        "ControlLeft" => KeyMapping::plain(0xA2),
        "ControlRight" => KeyMapping::extended(0xA3),
        "AltLeft" => KeyMapping::plain(0xA4),
        "AltRight" => KeyMapping::extended(0xA5),
        "MetaLeft" => KeyMapping::extended(0x5B),
        "MetaRight" => KeyMapping::extended(0x5C),
        "ContextMenu" => KeyMapping::extended(0x5D),

        // --- puntuacion, por posicion fisica (VK_OEM_*) ---
        "Minus" => KeyMapping::plain(0xBD),
        "Equal" => KeyMapping::plain(0xBB),
        "BracketLeft" => KeyMapping::plain(0xDB),
        "BracketRight" => KeyMapping::plain(0xDD),
        "Backslash" => KeyMapping::plain(0xDC),
        "Semicolon" => KeyMapping::plain(0xBA),
        "Quote" => KeyMapping::plain(0xDE),
        "Backquote" => KeyMapping::plain(0xC0),
        "Comma" => KeyMapping::plain(0xBC),
        "Period" => KeyMapping::plain(0xBE),
        "Slash" => KeyMapping::plain(0xBF),
        // Tecla extra de los teclados ISO (la de al lado del Shift izquierdo).
        "IntlBackslash" => KeyMapping::plain(0xE2),

        // --- navegacion: todas extendidas ---
        "Insert" => KeyMapping::extended(0x2D),
        "Delete" => KeyMapping::extended(0x2E),
        "Home" => KeyMapping::extended(0x24),
        "End" => KeyMapping::extended(0x23),
        "PageUp" => KeyMapping::extended(0x21),
        "PageDown" => KeyMapping::extended(0x22),
        "ArrowLeft" => KeyMapping::extended(0x25),
        "ArrowUp" => KeyMapping::extended(0x26),
        "ArrowRight" => KeyMapping::extended(0x27),
        "ArrowDown" => KeyMapping::extended(0x28),

        // --- teclado numerico ---
        // NumLock y la division son extendidas; el resto no.
        "NumLock" => KeyMapping::extended(0x90),
        "NumpadDivide" => KeyMapping::extended(0x6F),
        "NumpadMultiply" => KeyMapping::plain(0x6A),
        "NumpadSubtract" => KeyMapping::plain(0x6D),
        "NumpadAdd" => KeyMapping::plain(0x6B),
        "NumpadDecimal" => KeyMapping::plain(0x6E),
        // El Intro del numerico comparte codigo con el normal y solo se
        // distingue por la marca de extendida.
        "NumpadEnter" => KeyMapping::extended(0x0D),

        // --- sistema ---
        "PrintScreen" => KeyMapping::extended(0x2C),
        "ScrollLock" => KeyMapping::plain(0x91),
        "Pause" => KeyMapping::plain(0x13),

        _ => return None,
    };

    Some(mapping)
}

/// Codigos virtuales de los modificadores activos, en orden de pulsacion.
///
/// El orden importa: los modificadores se pulsan antes que la tecla y se
/// liberan en orden inverso, de modo que el sistema vea una combinacion bien
/// formada.
#[must_use]
pub fn modifier_keys(modifiers: Modifiers) -> Vec<VirtualKey> {
    let mut keys = Vec::with_capacity(4);
    if modifiers.ctrl {
        keys.push(VirtualKey::LCONTROL);
    }
    if modifiers.alt {
        keys.push(VirtualKey::LMENU);
    }
    if modifiers.shift {
        keys.push(VirtualKey::LSHIFT);
    }
    if modifiers.meta {
        keys.push(VirtualKey::LWIN);
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(name: &str) -> Option<KeyMapping> {
        lookup(&KeyCode(name.to_owned()))
    }

    fn vk(name: &str) -> u16 {
        map(name)
            .unwrap_or_else(|| panic!("{name} deberia traducirse"))
            .vk
            .0
    }

    #[test]
    fn las_letras_siguen_su_codigo_ascii() {
        assert_eq!(vk("KeyA"), 0x41);
        assert_eq!(vk("KeyM"), 0x4D);
        assert_eq!(vk("KeyZ"), 0x5A);
    }

    #[test]
    fn los_digitos_de_la_fila_superior_siguen_su_codigo_ascii() {
        assert_eq!(vk("Digit0"), 0x30);
        assert_eq!(vk("Digit5"), 0x35);
        assert_eq!(vk("Digit9"), 0x39);
    }

    #[test]
    fn el_teclado_numerico_tiene_codigos_propios() {
        assert_eq!(vk("Numpad0"), 0x60);
        assert_eq!(vk("Numpad9"), 0x69);
        // Y no se confunde con la fila superior.
        assert_ne!(vk("Numpad5"), vk("Digit5"));
    }

    #[test]
    fn las_teclas_de_funcion_van_de_f1_a_f24() {
        assert_eq!(vk("F1"), 0x70);
        assert_eq!(vk("F12"), 0x7B);
        assert_eq!(vk("F24"), 0x87);
        assert!(map("F0").is_none(), "F0 no existe");
        assert!(map("F25").is_none(), "F25 no existe");
    }

    #[test]
    fn los_nombres_mal_formados_no_se_traducen() {
        for malo in [
            "",
            "Key",
            "KeyAB",
            "Keya",
            "Digit",
            "Digit99",
            "Numpad",
            "F",
            "Fx",
            "KEYA",
            "teclaA",
            "rm",
            "ArrowLeftt",
        ] {
            assert!(map(malo).is_none(), "{malo:?} no deberia traducirse");
        }
    }

    #[test]
    fn las_flechas_son_extendidas() {
        // Sin la marca de extendida, la flecha izquierda se confunde con el 4
        // del teclado numerico cuando Bloq Num esta apagado.
        for flecha in ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"] {
            let m = map(flecha).expect(flecha);
            assert!(m.extended, "{flecha} deberia ser extendida");
        }
    }

    #[test]
    fn el_bloque_de_navegacion_es_extendido() {
        for tecla in ["Insert", "Delete", "Home", "End", "PageUp", "PageDown"] {
            assert!(
                map(tecla).expect(tecla).extended,
                "{tecla} deberia ser extendida"
            );
        }
    }

    #[test]
    fn los_modificadores_derechos_son_extendidos_y_los_izquierdos_no() {
        assert!(!map("ControlLeft").expect("ok").extended);
        assert!(map("ControlRight").expect("ok").extended);
        assert!(!map("AltLeft").expect("ok").extended);
        assert!(
            map("AltRight").expect("ok").extended,
            "AltGr necesita la marca"
        );
        assert!(!map("ShiftLeft").expect("ok").extended);
        assert!(map("ShiftRight").expect("ok").extended);
    }

    #[test]
    fn izquierdo_y_derecho_tienen_codigos_distintos() {
        assert_ne!(vk("ControlLeft"), vk("ControlRight"));
        assert_ne!(vk("AltLeft"), vk("AltRight"));
        assert_ne!(vk("ShiftLeft"), vk("ShiftRight"));
        assert_ne!(vk("MetaLeft"), vk("MetaRight"));
    }

    #[test]
    fn los_dos_intro_comparten_codigo_y_se_distinguen_por_la_marca() {
        let normal = map("Enter").expect("ok");
        let numerico = map("NumpadEnter").expect("ok");
        assert_eq!(
            normal.vk, numerico.vk,
            "Windows les da el mismo codigo virtual"
        );
        assert!(!normal.extended);
        assert!(numerico.extended, "solo la marca los distingue");
    }

    #[test]
    fn la_division_del_numerico_es_extendida_pero_la_multiplicacion_no() {
        assert!(map("NumpadDivide").expect("ok").extended);
        assert!(!map("NumpadMultiply").expect("ok").extended);
        assert!(!map("NumpadAdd").expect("ok").extended);
        assert!(!map("NumpadSubtract").expect("ok").extended);
    }

    #[test]
    fn la_puntuacion_usa_los_codigos_oem_por_posicion() {
        assert_eq!(vk("Minus"), 0xBD);
        assert_eq!(vk("Equal"), 0xBB);
        assert_eq!(vk("Slash"), 0xBF);
        assert_eq!(vk("Backquote"), 0xC0);
        assert_eq!(vk("IntlBackslash"), 0xE2);
    }

    #[test]
    fn no_hay_dos_teclas_con_el_mismo_codigo_y_la_misma_marca() {
        // Un duplicado accidental haria que dos teclas distintas produjeran
        // exactamente lo mismo, y es dificil de detectar a ojo en una tabla.
        let nombres = [
            "Escape",
            "Tab",
            "CapsLock",
            "Space",
            "Enter",
            "Backspace",
            "ShiftLeft",
            "ShiftRight",
            "ControlLeft",
            "ControlRight",
            "AltLeft",
            "AltRight",
            "MetaLeft",
            "MetaRight",
            "ContextMenu",
            "Minus",
            "Equal",
            "BracketLeft",
            "BracketRight",
            "Backslash",
            "Semicolon",
            "Quote",
            "Backquote",
            "Comma",
            "Period",
            "Slash",
            "IntlBackslash",
            "Insert",
            "Delete",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "ArrowLeft",
            "ArrowUp",
            "ArrowRight",
            "ArrowDown",
            "NumLock",
            "NumpadDivide",
            "NumpadMultiply",
            "NumpadSubtract",
            "NumpadAdd",
            "NumpadDecimal",
            "NumpadEnter",
            "PrintScreen",
            "ScrollLock",
            "Pause",
        ];

        let mut vistos = std::collections::HashMap::new();
        for nombre in nombres {
            let m = map(nombre).unwrap_or_else(|| panic!("{nombre} deberia estar en la tabla"));
            let clave = (m.vk, m.extended);
            if let Some(anterior) = vistos.insert(clave, nombre) {
                panic!(
                    "{nombre} y {anterior} comparten codigo {:#04x} y marca",
                    m.vk.0
                );
            }
        }
    }

    #[test]
    fn toda_tecla_del_protocolo_bien_formada_que_se_traduce_tiene_nombre_valido() {
        // La tabla no debe contener nombres que el propio protocolo rechazaria.
        for nombre in ["KeyA", "F11", "ArrowLeft", "NumpadAdd", "IntlBackslash"] {
            let code = KeyCode(nombre.to_owned());
            assert!(
                code.is_well_formed(),
                "{nombre} no pasa la validacion del protocolo"
            );
            assert!(lookup(&code).is_some());
        }
    }

    #[test]
    fn los_modificadores_se_devuelven_en_orden_estable() {
        assert!(modifier_keys(Modifiers::NONE).is_empty());

        let todos = modifier_keys(Modifiers {
            shift: true,
            ctrl: true,
            alt: true,
            meta: true,
        });
        assert_eq!(
            todos,
            vec![
                VirtualKey::LCONTROL,
                VirtualKey::LMENU,
                VirtualKey::LSHIFT,
                VirtualKey::LWIN
            ]
        );

        // Y el mismo conjunto siempre da el mismo orden.
        assert_eq!(
            modifier_keys(Modifiers {
                ctrl: true,
                ..Modifiers::NONE
            }),
            vec![VirtualKey::LCONTROL]
        );
    }

    #[test]
    fn las_combinaciones_habituales_se_traducen_enteras() {
        // Ctrl+Alt+Supr, Alt+Tab, Win+L, Ctrl+Shift+Escape.
        let casos: [(Modifiers, &str); 4] = [
            (
                Modifiers {
                    ctrl: true,
                    alt: true,
                    ..Modifiers::NONE
                },
                "Delete",
            ),
            (
                Modifiers {
                    alt: true,
                    ..Modifiers::NONE
                },
                "Tab",
            ),
            (
                Modifiers {
                    meta: true,
                    ..Modifiers::NONE
                },
                "KeyL",
            ),
            (
                Modifiers {
                    ctrl: true,
                    shift: true,
                    ..Modifiers::NONE
                },
                "Escape",
            ),
        ];
        for (mods, tecla) in casos {
            assert!(
                !modifier_keys(mods).is_empty(),
                "faltan modificadores para {tecla}"
            );
            assert!(map(tecla).is_some(), "{tecla} no se traduce");
        }
    }
}
