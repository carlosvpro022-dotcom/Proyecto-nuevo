//! La cadena completa de entrada, del mensaje del protocolo a la orden final.
//!
//! Recorre lo mismo que recorreria un toque en el iPad: llega un evento del
//! canal de control, se comprueba el permiso, se valida, se traduce a
//! coordenadas absolutas del escritorio y se entrega al backend.
//!
//! El backend que se usa aqui es el de registro, que anota en lugar de
//! inyectar. Lo que se comprueba es que **lo que llega al backend es lo
//! correcto**; que Windows lo ejecute bien es lo que falta por verificar en un
//! PC real.

use nexus_core::monitor::{MonitorInfo, MonitorLayout};
use nexus_core::protocol::control::ControlRequest;
use nexus_core::protocol::input::{
    InputEvent, KeyCode, KeyState, Modifiers, PointerButton, ScrollDelta,
};
use nexus_core::{Permission, PermissionSet};
use nexus_input::keymap::{self, VirtualKey};
use nexus_input::{InputInjector, RecordingInjector, ResolvedInput, resolve};

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
            origin_y: 0,
            scale_percent: 150,
            is_primary: false,
            refresh_hz: Some(144),
        },
    ])
}

/// Hace lo que haria el host: comprobar permiso, validar, resolver, inyectar.
fn procesar(
    permisos: &PermissionSet,
    peticion: &ControlRequest,
    monitor: u8,
    injector: &mut RecordingInjector,
) -> Result<(), String> {
    permisos
        .require(peticion.required_permission())
        .map_err(|e| e.to_string())?;

    let ControlRequest::Input { event } = peticion else {
        return Err("no es un evento de entrada".to_owned());
    };

    let resolved = resolve(event, &layout(), monitor).map_err(|e| e.to_string())?;
    injector.inject(&resolved).map_err(|e| e.to_string())
}

#[test]
fn un_toque_en_el_ipad_acaba_siendo_una_orden_absoluta() {
    let permisos = PermissionSet::standard();
    let mut injector = RecordingInjector::new();

    // El cliente envia la posicion normalizada del toque, mas la pulsacion.
    let eventos = [
        InputEvent::PointerMove { x: 0.5, y: 0.5 },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: KeyState::Down,
        },
        InputEvent::PointerButton {
            button: PointerButton::Left,
            state: KeyState::Up,
        },
    ];

    for event in eventos {
        procesar(
            &permisos,
            &ControlRequest::Input { event },
            0,
            &mut injector,
        )
        .expect("deberia procesarse");
    }

    assert_eq!(
        injector.injected(),
        [
            ResolvedInput::MoveCursor { x: 960, y: 540 },
            ResolvedInput::Button {
                button: PointerButton::Left,
                state: KeyState::Down
            },
            ResolvedInput::Button {
                button: PointerButton::Left,
                state: KeyState::Up
            },
        ]
    );
}

#[test]
fn el_mismo_toque_cae_en_sitios_distintos_segun_el_monitor() {
    let permisos = PermissionSet::standard();
    let event = InputEvent::PointerMove { x: 0.5, y: 0.5 };

    let mut en_principal = RecordingInjector::new();
    procesar(
        &permisos,
        &ControlRequest::Input {
            event: event.clone(),
        },
        0,
        &mut en_principal,
    )
    .expect("monitor 0");

    let mut en_secundario = RecordingInjector::new();
    procesar(
        &permisos,
        &ControlRequest::Input { event },
        1,
        &mut en_secundario,
    )
    .expect("monitor 1");

    assert_eq!(
        en_principal.injected()[0],
        ResolvedInput::MoveCursor { x: 960, y: 540 }
    );
    assert_eq!(
        en_secundario.injected()[0],
        ResolvedInput::MoveCursor { x: -1_280, y: 720 },
        "el secundario tiene origen y tamano propios"
    );
}

#[test]
fn sin_permiso_de_teclado_no_se_escribe_nada() {
    // Un dispositivo con raton pero sin teclado: util para dejar que alguien
    // senale algo sin darle control del teclado.
    let permisos = PermissionSet::from_iter([Permission::ViewScreen, Permission::ControlPointer]);
    let mut injector = RecordingInjector::new();

    let raton = ControlRequest::Input {
        event: InputEvent::PointerMove { x: 0.1, y: 0.1 },
    };
    procesar(&permisos, &raton, 0, &mut injector).expect("el raton si");

    let teclado = ControlRequest::Input {
        event: InputEvent::Text {
            text: "rm -rf".into(),
        },
    };
    let err = procesar(&permisos, &teclado, 0, &mut injector).unwrap_err();
    assert!(err.contains("control_keyboard"), "{err}");

    assert_eq!(
        injector.injected().len(),
        1,
        "solo debe haber pasado el movimiento"
    );
}

#[test]
fn un_evento_manipulado_no_llega_al_backend() {
    let permisos = PermissionSet::full();
    let mut injector = RecordingInjector::new();

    let manipulados = [
        // Fuera del rango que declara el protocolo.
        InputEvent::PointerMove { x: 50.0, y: 0.5 },
        InputEvent::PointerMove {
            x: f32::NAN,
            y: 0.0,
        },
        // Nombre de tecla que no es un nombre de tecla.
        InputEvent::Key {
            code: KeyCode("'; DROP TABLE devices; --".into()),
            state: KeyState::Down,
            modifiers: Modifiers::NONE,
        },
        // Texto vacio y texto desmesurado.
        InputEvent::Text {
            text: String::new(),
        },
        InputEvent::Text {
            text: "a".repeat(InputEvent::MAX_TEXT_LEN + 1),
        },
        // Combinacion con mas teclas de las admitidas.
        InputEvent::Chord {
            modifiers: Modifiers::NONE,
            keys: (0..30).map(|i| KeyCode(format!("Key{i}"))).collect(),
        },
    ];

    for event in manipulados {
        let peticion = ControlRequest::Input { event };
        assert!(
            procesar(&permisos, &peticion, 0, &mut injector).is_err(),
            "deberia rechazarse: {peticion:?}"
        );
    }

    assert!(
        injector.injected().is_empty(),
        "nada manipulado debe llegar al backend"
    );
}

#[test]
fn una_combinacion_conocida_se_traduce_entera_a_codigos_de_windows() {
    // Ctrl+Alt+Supr, que es la combinacion que mas se pide en un escritorio
    // remoto y la que mas partes tiene que encajar.
    let permisos = PermissionSet::full();
    let mut injector = RecordingInjector::new();

    let event = InputEvent::Chord {
        modifiers: Modifiers {
            ctrl: true,
            alt: true,
            ..Modifiers::NONE
        },
        keys: vec![KeyCode("Delete".into())],
    };
    procesar(
        &permisos,
        &ControlRequest::Input { event },
        0,
        &mut injector,
    )
    .expect("procesa");

    let ResolvedInput::Chord { modifiers, keys } = &injector.injected()[0] else {
        panic!("deberia ser una combinacion");
    };

    // Los modificadores se traducen a sus codigos virtuales, en orden estable.
    assert_eq!(
        keymap::modifier_keys(*modifiers),
        vec![VirtualKey::LCONTROL, VirtualKey::LMENU]
    );

    // Y la tecla tambien, con su marca de extendida.
    let supr = keymap::lookup(&keys[0]).expect("Delete deberia traducirse");
    assert_eq!(supr.vk, VirtualKey(0x2E));
    assert!(supr.extended, "Supr es extendida");
}

#[test]
fn cada_tecla_que_acepta_el_protocolo_tiene_traduccion_o_se_rechaza() {
    // No debe haber teclas que pasen la validacion del protocolo y luego el
    // backend no sepa traducir en silencio: o se traducen, o se rechazan.
    let conocidas = [
        "KeyA",
        "KeyZ",
        "Digit0",
        "Digit9",
        "F1",
        "F24",
        "Escape",
        "Tab",
        "Enter",
        "Backspace",
        "Space",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "Insert",
        "Delete",
        "Numpad0",
        "NumpadEnter",
        "ControlLeft",
        "ControlRight",
        "AltLeft",
        "AltRight",
        "ShiftLeft",
        "MetaLeft",
    ];
    for nombre in conocidas {
        let code = KeyCode(nombre.to_owned());
        assert!(
            code.is_well_formed(),
            "{nombre} deberia pasar la validacion"
        );
        assert!(
            keymap::lookup(&code).is_some(),
            "{nombre} deberia traducirse"
        );
    }

    // Y una que pasa la validacion pero no existe en ningun teclado: el
    // backend debe decir que no la conoce, no ignorarla.
    let inventada = KeyCode("TeclaInventada".to_owned());
    assert!(inventada.is_well_formed());
    assert!(keymap::lookup(&inventada).is_none());
}

#[test]
fn el_scroll_no_necesita_monitor_ni_permiso_de_teclado() {
    let permisos = PermissionSet::from_iter([Permission::ViewScreen, Permission::ControlPointer]);
    let mut injector = RecordingInjector::new();

    let event = InputEvent::Scroll {
        delta: ScrollDelta { x: 0, y: -3 },
    };
    procesar(
        &permisos,
        &ControlRequest::Input { event },
        0,
        &mut injector,
    )
    .expect("procesa");

    assert_eq!(
        injector.injected()[0],
        ResolvedInput::Scroll {
            delta: ScrollDelta { x: 0, y: -3 }
        }
    );
}

#[test]
fn al_cerrar_la_sesion_se_sueltan_las_teclas() {
    // Si se corta la red con Ctrl pulsado, el PC no debe quedarse con la tecla
    // hundida. El host llama a `release_all` al cerrar.
    let mut injector = RecordingInjector::new();
    assert_eq!(injector.releases(), 0);
    injector.release_all().expect("suelta todo");
    assert_eq!(injector.releases(), 1);
}

#[test]
fn fuera_de_windows_el_motor_lo_dice_en_lugar_de_fingir() {
    // En Linux no hay captura ni inyeccion. Lo importante es que falle de
    // forma visible y no devuelva un resultado vacio que parezca valido.
    if cfg!(windows) {
        return;
    }

    let err = nexus_capture::enumerate_monitors().unwrap_err();
    assert!(
        matches!(err, nexus_capture::CaptureError::UnsupportedPlatform { .. }),
        "{err:?}"
    );

    let mut injector = nexus_input::default_injector().expect("se construye igualmente");
    let err = injector
        .inject(&ResolvedInput::MoveCursor { x: 0, y: 0 })
        .unwrap_err();
    assert!(
        matches!(err, nexus_input::InputError::UnsupportedPlatform { .. }),
        "{err:?}"
    );
}
