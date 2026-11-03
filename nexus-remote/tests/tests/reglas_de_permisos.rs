//! El protocolo y el host deben coincidir en que permiso exige cada operacion.
//!
//! Es facil anadir una orden nueva al canal de control y olvidar comprobar su
//! permiso. Estas pruebas recorren el catalogo completo y verifican que un
//! dispositivo con un perfil concreto puede hacer exactamente lo que ese perfil
//! promete, ni mas ni menos.

use nexus_core::protocol::control::{ControlRequest, PowerAction};
use nexus_core::protocol::input::{InputEvent, KeyCode, KeyState, Modifiers, PointerButton};
use nexus_core::{Permission, PermissionSet, QualityProfile, TransferId};

/// Todas las operaciones del canal de control, para recorrerlas una a una.
fn todas_las_operaciones() -> Vec<ControlRequest> {
    vec![
        ControlRequest::Input {
            event: InputEvent::PointerMove { x: 0.5, y: 0.5 },
        },
        ControlRequest::Input {
            event: InputEvent::PointerButton {
                button: PointerButton::Left,
                state: KeyState::Down,
            },
        },
        ControlRequest::Input {
            event: InputEvent::Key {
                code: KeyCode("KeyA".into()),
                state: KeyState::Down,
                modifiers: Modifiers::NONE,
            },
        },
        ControlRequest::Input {
            event: InputEvent::Text {
                text: "hola".into(),
            },
        },
        ControlRequest::ListMonitors,
        ControlRequest::SelectMonitor { index: 0 },
        ControlRequest::SetQuality {
            profile: QualityProfile::default(),
        },
        ControlRequest::ClipboardGet,
        ControlRequest::ClipboardSet { text: "x".into() },
        ControlRequest::ListDir {
            path: "C:/Compartido".into(),
        },
        ControlRequest::DownloadStart {
            path: "C:/Compartido/a.txt".into(),
        },
        ControlRequest::UploadStart {
            path: "C:/Compartido/b.txt".into(),
            size_bytes: 10,
        },
        ControlRequest::TransferCancel {
            transfer_id: TransferId::new(),
        },
        ControlRequest::SystemStatus,
        ControlRequest::Power {
            action: PowerAction::Lock,
            confirmation_token: None,
        },
        ControlRequest::Power {
            action: PowerAction::Reboot,
            confirmation_token: None,
        },
        ControlRequest::Power {
            action: PowerAction::Shutdown,
            confirmation_token: None,
        },
        ControlRequest::SetAudioEnabled { enabled: true },
    ]
}

/// Lo que el host haria: comprobar el permiso antes de ejecutar.
fn permitido(permisos: &PermissionSet, operacion: &ControlRequest) -> bool {
    permisos.require(operacion.required_permission()).is_ok()
}

#[test]
fn el_perfil_de_solo_ver_no_puede_tocar_nada() {
    let permisos = PermissionSet::view_only();

    for operacion in todas_las_operaciones() {
        let autorizado = permitido(&permisos, &operacion);
        let esperado = matches!(
            operacion,
            ControlRequest::ListMonitors
                | ControlRequest::SelectMonitor { .. }
                | ControlRequest::SetQuality { .. }
                | ControlRequest::SystemStatus
        );
        assert_eq!(
            autorizado,
            esperado,
            "solo ver: {operacion:?} deberia estar {}",
            if esperado { "permitido" } else { "denegado" }
        );
    }
}

#[test]
fn el_perfil_estandar_controla_pero_no_administra() {
    let permisos = PermissionSet::standard();

    // Puede controlar el escritorio.
    for operacion in [
        ControlRequest::Input {
            event: InputEvent::PointerMove { x: 0.1, y: 0.1 },
        },
        ControlRequest::Input {
            event: InputEvent::Text { text: "a".into() },
        },
        ControlRequest::ClipboardGet,
        ControlRequest::ClipboardSet { text: "a".into() },
        ControlRequest::ListDir { path: "C:/".into() },
        ControlRequest::SetAudioEnabled { enabled: true },
    ] {
        assert!(
            permitido(&permisos, &operacion),
            "deberia poder: {operacion:?}"
        );
    }

    // No puede administrar ni escribir archivos.
    for operacion in [
        ControlRequest::UploadStart {
            path: "C:/a".into(),
            size_bytes: 1,
        },
        ControlRequest::Power {
            action: PowerAction::Lock,
            confirmation_token: None,
        },
        ControlRequest::Power {
            action: PowerAction::Reboot,
            confirmation_token: None,
        },
        ControlRequest::Power {
            action: PowerAction::Shutdown,
            confirmation_token: None,
        },
    ] {
        assert!(
            !permitido(&permisos, &operacion),
            "no deberia poder: {operacion:?}"
        );
    }
}

#[test]
fn el_perfil_completo_puede_todo_pero_sigue_pidiendo_confirmacion() {
    let permisos = PermissionSet::full();

    for operacion in todas_las_operaciones() {
        assert!(
            permitido(&permisos, &operacion),
            "perfil completo: {operacion:?}"
        );
    }

    // Tener el permiso no basta: reiniciar y apagar exigen confirmacion en el host.
    assert!(
        ControlRequest::Power {
            action: PowerAction::Reboot,
            confirmation_token: None
        }
        .requires_host_confirmation()
    );
    assert!(
        ControlRequest::Power {
            action: PowerAction::Shutdown,
            confirmation_token: None
        }
        .requires_host_confirmation()
    );
    assert!(
        !ControlRequest::Power {
            action: PowerAction::Lock,
            confirmation_token: None
        }
        .requires_host_confirmation()
    );
}

#[test]
fn raton_y_teclado_son_permisos_separados() {
    // Un dispositivo puede mover el raton sin poder escribir, que es util para
    // dejar que alguien senale algo en la pantalla sin darle el teclado.
    let solo_raton = PermissionSet::from_iter([Permission::ViewScreen, Permission::ControlPointer]);

    assert!(permitido(
        &solo_raton,
        &ControlRequest::Input {
            event: InputEvent::PointerMove { x: 0.5, y: 0.5 }
        }
    ));
    assert!(!permitido(
        &solo_raton,
        &ControlRequest::Input {
            event: InputEvent::Text {
                text: "rm -rf".into()
            }
        }
    ));
}

#[test]
fn leer_y_escribir_archivos_son_permisos_separados() {
    let solo_lectura = PermissionSet::from_iter([Permission::ViewScreen, Permission::FileRead]);

    assert!(permitido(
        &solo_lectura,
        &ControlRequest::ListDir { path: "C:/".into() }
    ));
    assert!(permitido(
        &solo_lectura,
        &ControlRequest::DownloadStart {
            path: "C:/a.txt".into()
        }
    ));
    assert!(!permitido(
        &solo_lectura,
        &ControlRequest::UploadStart {
            path: "C:/a.txt".into(),
            size_bytes: 1
        }
    ));
}

#[test]
fn un_conjunto_vacio_deniega_absolutamente_todo() {
    let ninguno = PermissionSet::empty();
    for operacion in todas_las_operaciones() {
        assert!(
            !permitido(&ninguno, &operacion),
            "sin permisos: {operacion:?}"
        );
    }
}

#[test]
fn la_interseccion_de_permisos_nunca_concede_de_mas() {
    // Lo que un dispositivo pide se cruza con lo que tiene concedido: pedir mas
    // no da mas.
    let concedidos = PermissionSet::view_only();
    let pedidos = PermissionSet::full();
    let efectivos = concedidos.intersect(&pedidos);

    assert_eq!(efectivos, concedidos);
    for operacion in todas_las_operaciones() {
        assert_eq!(
            permitido(&efectivos, &operacion),
            permitido(&concedidos, &operacion),
            "{operacion:?}"
        );
    }
}
