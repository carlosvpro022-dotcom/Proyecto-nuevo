//! Inyeccion de entrada en Windows con `SendInput`.
//!
//! # Estado de verificacion
//!
//! Este modulo **se compila para Windows pero no se ha ejecutado en Windows**.
//! El desarrollo ocurre en Linux y la comprobacion que se hace es la de nivel 2
//! del [ADR-0007]: compilacion en cruce al objetivo
//! `x86_64-pc-windows-gnu`, que verifica tipos, nombres de API y firmas, pero
//! no el comportamiento.
//!
//! Lo que si esta probado de verdad es todo lo que decide **que** se inyecta:
//! la tabla de teclas ([`crate::keymap`]), la aritmetica de coordenadas
//! ([`crate::absolute`]) y la validacion de los eventos
//! ([`crate::resolve`]). Este archivo es la parte fina que traduce esas
//! decisiones ya tomadas a llamadas del sistema.
//!
//! Antes de dar la etapa 2 por buena hace falta ejecutarlo en un PC con
//! Windows.
//!
//! [ADR-0007]: ../../../../docs/architecture/ADR-0007-verificacion-multiplataforma.md
//!
//! # Por que hay `unsafe`
//!
//! `SendInput` es una llamada al sistema: recibe un puntero a un array de
//! estructuras y Rust no puede comprobar por si mismo que el tamano declarado
//! coincide. El `unsafe` esta acotado a las llamadas concretas, cada una con
//! su justificacion escrita.
//!
//! # Lo que no puede hacer
//!
//! Un proceso de usuario normal **no puede** inyectar entrada en ventanas que
//! corren con mas privilegios (el dialogo de Control de cuentas de usuario, la
//! pantalla de bloqueo, el escritorio seguro). Windows lo impide a proposito
//! con el aislamiento de privilegios de la interfaz. Para alcanzarlos hara
//! falta un servicio del sistema, que es trabajo de la etapa 6.

// Las llamadas a la API de Windows son intrinsecamente inseguras para Rust.
// Cada bloque `unsafe` de este archivo lleva su justificacion.
#![allow(unsafe_code)]

use std::collections::BTreeSet;

use nexus_core::protocol::input::{KeyState, PointerButton};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MOUSE_EVENT_FLAGS,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, MapVirtualKeyW,
    SendInput,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use crate::absolute::VirtualDesktop;
use crate::error::{InputError, InputResult};
use crate::inject::InputInjector;
use crate::keymap::{self, VirtualKey};
use crate::resolve::ResolvedInput;

/// Unidades que Windows considera un "clic" de rueda.
const WHEEL_DELTA: i32 = 120;

/// Backend real de Windows.
///
/// Recuerda que teclas y botones ha dejado pulsados para poder soltarlos todos
/// al cerrar la sesion. Sin eso, perder la red con Ctrl pulsado dejaria el PC
/// con la tecla hundida.
#[derive(Debug)]
pub struct WindowsInjector {
    desktop: VirtualDesktop,
    /// Teclas pulsadas, con su marca de extendida.
    ///
    /// La marca forma parte de la identidad de la tecla: `Enter` y el `Intro`
    /// del teclado numerico comparten codigo virtual y solo se distinguen por
    /// ella. Guardarla permite soltar exactamente la misma tecla que se pulso.
    pressed_keys: BTreeSet<(u16, bool)>,
    pressed_buttons: BTreeSet<u8>,
}

impl WindowsInjector {
    /// Crea el backend leyendo las metricas del escritorio virtual.
    ///
    /// # Errors
    ///
    /// [`InputError::SystemRejected`] si el sistema devuelve un escritorio sin
    /// tamano, lo que ocurriria en una sesion sin estacion de ventanas.
    pub fn new() -> InputResult<Self> {
        Ok(Self {
            desktop: read_virtual_desktop()?,
            pressed_keys: BTreeSet::new(),
            pressed_buttons: BTreeSet::new(),
        })
    }

    /// Vuelve a leer las metricas del escritorio virtual.
    ///
    /// Hay que llamarlo cuando cambia la configuracion de monitores: si no,
    /// las coordenadas se calcularian contra un escritorio que ya no existe y
    /// el cursor caeria en el sitio equivocado.
    ///
    /// # Errors
    ///
    /// Igual que [`Self::new`].
    pub fn refresh_desktop(&mut self) -> InputResult<()> {
        self.desktop = read_virtual_desktop()?;
        Ok(())
    }

    /// Escritorio virtual que esta usando ahora mismo.
    #[must_use]
    pub const fn desktop(&self) -> VirtualDesktop {
        self.desktop
    }

    /// Entrega los eventos al sistema.
    ///
    /// No depende del estado del inyector: es la llamada pura a la API.
    fn send(inputs: &[INPUT]) -> InputResult<()> {
        if inputs.is_empty() {
            return Ok(());
        }

        let size = i32::try_from(size_of::<INPUT>()).map_err(|_| {
            InputError::SystemRejected("el tamano de INPUT no cabe en i32".to_owned())
        })?;

        // SEGURIDAD: `inputs` es un slice vivo de `INPUT` correctamente
        // alineados, y `size` es el tamano real de la estructura que exige la
        // API. `SendInput` solo lee del slice durante la llamada.
        let sent = unsafe { SendInput(inputs, size) };

        let expected = u32::try_from(inputs.len()).unwrap_or(u32::MAX);
        if sent == expected {
            Ok(())
        } else {
            // Windows corta en el primer evento rechazado y devuelve cuantos
            // aceptó. Suele significar que otra ventana con mas privilegios
            // tiene el foco, o que lo bloqueo un filtro de entrada.
            Err(InputError::SystemRejected(format!(
                "SendInput acepto {sent} de {expected} eventos; \
                 lo habitual es que la ventana activa tenga mas privilegios"
            )))
        }
    }

    fn key_input(vk: u16, extended: bool, state: KeyState) -> INPUT {
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if state == KeyState::Up {
            flags |= KEYEVENTF_KEYUP;
        }

        // Se rellena tambien el codigo de rastreo. Con `wVk` bastaria para la
        // mayoria de programas, pero los que leen la entrada a bajo nivel
        // (juegos, maquinas virtuales) esperan encontrarlo.
        //
        // SEGURIDAD: `MapVirtualKeyW` solo consulta la distribucion de teclado
        // activa. No recibe punteros y no puede fallar de forma insegura.
        let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) };

        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(vk),
                    wScan: u16::try_from(scan).unwrap_or(0),
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// Evento que escribe un caracter literal, sin pasar por ninguna tecla.
    ///
    /// Es como se respeta la distribucion del cliente: el iPad envia el texto
    /// que la persona escribio y aqui se inserta tal cual, sin depender de que
    /// el teclado de Windows tenga esa tecla.
    fn unicode_input(unit: u16, state: KeyState) -> INPUT {
        let mut flags = KEYEVENTF_UNICODE;
        if state == KeyState::Up {
            flags |= KEYEVENTF_KEYUP;
        }
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn mouse_input(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn button_flags(button: PointerButton, state: KeyState) -> (MOUSE_EVENT_FLAGS, u8) {
        match (button, state) {
            (PointerButton::Left, KeyState::Down) => (MOUSEEVENTF_LEFTDOWN, 0),
            (PointerButton::Left, KeyState::Up) => (MOUSEEVENTF_LEFTUP, 0),
            (PointerButton::Right, KeyState::Down) => (MOUSEEVENTF_RIGHTDOWN, 1),
            (PointerButton::Right, KeyState::Up) => (MOUSEEVENTF_RIGHTUP, 1),
            (PointerButton::Middle, KeyState::Down) => (MOUSEEVENTF_MIDDLEDOWN, 2),
            (PointerButton::Middle, KeyState::Up) => (MOUSEEVENTF_MIDDLEUP, 2),
        }
    }

    fn press_key(&mut self, vk: VirtualKey, extended: bool) -> InputResult<()> {
        Self::send(&[Self::key_input(vk.0, extended, KeyState::Down)])?;
        self.pressed_keys.insert((vk.0, extended));
        Ok(())
    }

    fn release_key(&mut self, vk: VirtualKey, extended: bool) -> InputResult<()> {
        Self::send(&[Self::key_input(vk.0, extended, KeyState::Up)])?;
        self.pressed_keys.remove(&(vk.0, extended));
        Ok(())
    }

    /// Teclas que siguen pulsadas, con su marca de extendida.
    ///
    /// Expuesto para poder comprobar desde fuera que la sesion no deja teclas
    /// colgadas, que es el fallo que mas molesta a quien esta delante del PC.
    #[must_use]
    pub fn pressed_keys(&self) -> Vec<(u16, bool)> {
        self.pressed_keys.iter().copied().collect()
    }
}

impl InputInjector for WindowsInjector {
    fn name(&self) -> &'static str {
        "windows-sendinput"
    }

    fn inject(&mut self, input: &ResolvedInput) -> InputResult<()> {
        match input {
            ResolvedInput::MoveCursor { x, y } => {
                let (dx, dy) = self.desktop.to_absolute(*x, *y);
                Self::send(&[Self::mouse_input(
                    dx,
                    dy,
                    0,
                    MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                )])
            }

            ResolvedInput::Button { button, state } => {
                let (flags, id) = Self::button_flags(*button, *state);
                Self::send(&[Self::mouse_input(0, 0, 0, flags)])?;
                if *state == KeyState::Down {
                    self.pressed_buttons.insert(id);
                } else {
                    self.pressed_buttons.remove(&id);
                }
                Ok(())
            }

            ResolvedInput::Scroll { delta } => {
                let mut events = Vec::with_capacity(2);
                if delta.y != 0 {
                    // Windows cuenta hacia arriba como positivo, igual que el
                    // protocolo, asi que no hay que invertir nada.
                    let amount = i32::from(delta.y) * WHEEL_DELTA;
                    events.push(Self::mouse_input(0, 0, amount as u32, MOUSEEVENTF_WHEEL));
                }
                if delta.x != 0 {
                    let amount = i32::from(delta.x) * WHEEL_DELTA;
                    events.push(Self::mouse_input(0, 0, amount as u32, MOUSEEVENTF_HWHEEL));
                }
                Self::send(&events)
            }

            ResolvedInput::Key {
                code,
                state,
                modifiers,
            } => {
                let mapping = keymap::lookup(code).ok_or_else(|| InputError::UnmappedKey {
                    code: code.0.clone(),
                })?;

                // Los modificadores se aplican alrededor de la tecla y se
                // sueltan despues, para no dejarlos colgados.
                let mods = keymap::modifier_keys(*modifiers);
                if *state == KeyState::Down {
                    for m in &mods {
                        self.press_key(*m, false)?;
                    }
                }

                let result = if *state == KeyState::Down {
                    self.press_key(mapping.vk, mapping.extended)
                } else {
                    self.release_key(mapping.vk, mapping.extended)
                };

                if *state == KeyState::Up {
                    for m in mods.iter().rev() {
                        // Se intentan soltar todos aunque uno falle: dejar un
                        // modificador pulsado es peor que perder un error.
                        let _ = self.release_key(*m, false);
                    }
                }
                result
            }

            ResolvedInput::Text { text } => {
                // Cada caracter necesita pulsacion y liberacion, y los que
                // estan fuera del plano basico ocupan dos unidades UTF-16 que
                // deben ir seguidas.
                let mut events = Vec::with_capacity(text.len() * 2);
                for unit in text.encode_utf16() {
                    events.push(Self::unicode_input(unit, KeyState::Down));
                    events.push(Self::unicode_input(unit, KeyState::Up));
                }
                Self::send(&events)
            }

            ResolvedInput::Chord { modifiers, keys } => {
                let mods = keymap::modifier_keys(*modifiers);
                let mapped: Vec<_> = keys
                    .iter()
                    .map(|code| {
                        keymap::lookup(code).ok_or_else(|| InputError::UnmappedKey {
                            code: code.0.clone(),
                        })
                    })
                    .collect::<InputResult<_>>()?;

                // Se traduce todo antes de pulsar nada: si una tecla de la
                // combinacion no existe, no se pulsa ninguna.
                for m in &mods {
                    self.press_key(*m, false)?;
                }
                let result = (|| {
                    for k in &mapped {
                        self.press_key(k.vk, k.extended)?;
                    }
                    Ok(())
                })();

                // Se suelta siempre, en orden inverso, pase lo que pase.
                for k in mapped.iter().rev() {
                    let _ = self.release_key(k.vk, k.extended);
                }
                for m in mods.iter().rev() {
                    let _ = self.release_key(*m, false);
                }
                result
            }
        }
    }

    fn release_all(&mut self) -> InputResult<()> {
        let keys: Vec<(u16, bool)> = self.pressed_keys.iter().copied().collect();
        let buttons: Vec<u8> = self.pressed_buttons.iter().copied().collect();

        let mut events = Vec::with_capacity(keys.len() + buttons.len());
        for (vk, extended) in keys {
            // Se suelta con la misma marca con la que se pulso, de modo que la
            // tecla que se libera es exactamente la misma.
            events.push(Self::key_input(vk, extended, KeyState::Up));
        }
        for id in buttons {
            let flags = match id {
                0 => MOUSEEVENTF_LEFTUP,
                1 => MOUSEEVENTF_RIGHTUP,
                _ => MOUSEEVENTF_MIDDLEUP,
            };
            events.push(Self::mouse_input(0, 0, 0, flags));
        }

        // Se vacia el registro antes de enviar: si el envio falla, no conviene
        // reintentar indefinidamente al cerrar.
        self.pressed_keys.clear();
        self.pressed_buttons.clear();
        Self::send(&events)
    }
}

/// Lee el rectangulo que ocupan todos los monitores juntos.
fn read_virtual_desktop() -> InputResult<VirtualDesktop> {
    // SEGURIDAD: `GetSystemMetrics` solo consulta un valor del sistema. No
    // recibe punteros ni reserva memoria.
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    VirtualDesktop::new(left, top, width, height)
}
