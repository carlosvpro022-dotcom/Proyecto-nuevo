//! Bloquear, reiniciar y apagar en Windows.
//!
//! # Estado de verificacion
//!
//! Este modulo **se compila para Windows pero no se ha ejecutado en Windows**.
//! La comprobacion que se le hace es la de nivel 2 del [ADR-0007]:
//! compilacion en cruce a `x86_64-pc-windows-gnu`.
//!
//! Es, ademas, el modulo que menos se puede probar de todo el proyecto: una
//! prueba automatica de «apagar el equipo» apagaria el equipo donde corre. Por
//! eso lo que si se prueba, y se prueba a fondo, es todo lo que decide **si**
//! se llega hasta aqui: el permiso y la confirmacion de una persona.
//!
//! [ADR-0007]: ../../../../docs/architecture/ADR-0007-verificacion-multiplataforma.md
//!
//! # El privilegio de apagado
//!
//! Reiniciar o apagar exige `SeShutdownPrivilege`. Windows se lo da a las
//! cuentas normales, pero **desactivado**: hay que activarlo en el testigo del
//! proceso antes de cada intento. Y hay una trampa conocida:
//! `AdjustTokenPrivileges` **devuelve exito aunque no haya podido activarlo**,
//! dejando el fallo solo en `GetLastError`. Comprobar unicamente el valor de
//! retorno es el error clasico de este codigo, y por eso aqui se mira tambien
//! el ultimo error.
//!
//! # Por que no se fuerza el cierre de los programas
//!
//! Se usa `EWX_FORCEIFHUNG`, que cierra los programas que no responden, y no
//! `EWX_FORCE`, que cierra todos sin preguntar. La diferencia es el trabajo
//! sin guardar de la persona que use el PC: apagar en remoto no deberia poder
//! tirar a la basura un documento abierto. Si un programa pide confirmacion,
//! el apagado se detiene, que es el comportamiento correcto.

// Las llamadas a la API de Windows son intrinsecamente inseguras para Rust.
// Cada bloque `unsafe` de este archivo lleva su justificacion.
#![allow(unsafe_code)]

use nexus_core::protocol::control::PowerAction;
use windows::Win32::Foundation::{CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
    SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Shutdown::{
    EWX_FORCEIFHUNG, EWX_REBOOT, EWX_SHUTDOWN, EXIT_WINDOWS_FLAGS, ExitWindowsEx, LockWorkStation,
    SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER, SHTDN_REASON_MINOR_OTHER,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::control::PowerControl;
use crate::error::{PowerError, PowerResult};

/// Operaciones de energia en Windows.
#[derive(Debug, Default)]
pub struct WindowsPower;

impl WindowsPower {
    /// Crea el backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl PowerControl for WindowsPower {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn execute(&mut self, action: PowerAction) -> PowerResult<()> {
        match action {
            PowerAction::Lock => {
                // SEGURIDAD: no recibe punteros ni devuelve recursos que
                // haya que liberar.
                unsafe { LockWorkStation() }.map_err(|e| PowerError::System(e.to_string()))
            }
            PowerAction::Reboot => exit_windows(EWX_REBOOT | EWX_FORCEIFHUNG),
            PowerAction::Shutdown => exit_windows(EWX_SHUTDOWN | EWX_FORCEIFHUNG),
        }
    }
}

/// Activa el privilegio de apagado y ejecuta la operacion.
fn exit_windows(flags: EXIT_WINDOWS_FLAGS) -> PowerResult<()> {
    enable_shutdown_privilege()?;

    let reason = SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_MINOR_OTHER | SHTDN_REASON_FLAG_PLANNED;
    // SEGURIDAD: solo recibe valores por copia. El apagado es asincrono: esta
    // llamada devuelve en cuanto el sistema lo acepta.
    unsafe { ExitWindowsEx(flags, reason) }.map_err(|e| PowerError::System(e.to_string()))
}

/// Activa `SeShutdownPrivilege` en el testigo de este proceso.
fn enable_shutdown_privilege() -> PowerResult<()> {
    let mut token = HANDLE::default();

    // SEGURIDAD: `GetCurrentProcess` devuelve un pseudo-identificador que no
    // se cierra. `OpenProcessToken` escribe en `token`, que es una variable
    // local viva durante toda la llamada.
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &raw mut token,
        )
    }
    .map_err(|e| PowerError::System(e.to_string()))?;

    let resultado = adjust_privilege(token);

    // SEGURIDAD: cierra el testigo que esta funcion abrio, una sola vez.
    if let Err(error) = unsafe { CloseHandle(token) } {
        tracing::warn!(%error, "no se pudo cerrar el testigo del proceso");
    }

    resultado
}

/// Pide al sistema que active el privilegio en el testigo recibido.
fn adjust_privilege(token: HANDLE) -> PowerResult<()> {
    let mut luid = LUID::default();
    // SEGURIDAD: escribe en `luid`, variable local viva durante la llamada.
    unsafe { LookupPrivilegeValueW(None, SE_SHUTDOWN_NAME, &raw mut luid) }
        .map_err(|e| PowerError::System(e.to_string()))?;

    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };

    // SEGURIDAD: `privileges` vive durante toda la llamada y declara un unico
    // privilegio, que es justo lo que dice `PrivilegeCount`.
    unsafe { AdjustTokenPrivileges(token, false, Some(&raw const privileges), 0, None, None) }
        .map_err(|e| PowerError::System(e.to_string()))?;

    // La trampa descrita arriba: la llamada anterior sale «bien» aunque no
    // haya podido activar el privilegio.
    // SEGURIDAD: no recibe punteros.
    if unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED {
        return Err(PowerError::PrivilegeDenied);
    }

    Ok(())
}
