//! Enumeracion de monitores en Windows.
//!
//! # Estado de verificacion
//!
//! Compilado para Windows, **no ejecutado en Windows**: verificacion de nivel 2
//! del [ADR-0007]. Lo que si esta probado de verdad es la conversion de los
//! datos crudos a [`MonitorInfo`](nexus_core::MonitorInfo), en
//! [`crate::enumeration`], que es donde estan los casos raros.
//!
//! [ADR-0007]: ../../../../docs/architecture/ADR-0007-verificacion-multiplataforma.md
//!
//! # Que lee de cada pantalla
//!
//! | Dato | API |
//! | --- | --- |
//! | Rectangulo y si es principal | `GetMonitorInfoW` |
//! | Nombre de dispositivo | `MONITORINFOEXW::szDevice` |
//! | Puntos por pulgada | `GetDpiForMonitor` |
//! | Frecuencia de refresco | `EnumDisplaySettingsW` |
//!
//! Si alguna de las tres ultimas falla, la pantalla **no se descarta**: se usa
//! un valor por omision razonable. Perder la frecuencia de refresco es mucho
//! menos grave que perder el monitor entero.

#![allow(unsafe_code)] // Llamadas a la API de Windows, justificadas una a una.

use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW,
    HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows::core::PCWSTR;

use crate::enumeration::{self, RawMonitor};
use crate::error::{CaptureError, CaptureResult};

/// Enumera todas las pantallas conectadas.
///
/// # Errors
///
/// [`CaptureError::System`] si `EnumDisplayMonitors` falla, que en la practica
/// solo ocurre en una sesion sin estacion de ventanas (un servicio sin
/// escritorio interactivo).
pub fn enumerate() -> CaptureResult<nexus_core::MonitorLayout> {
    let mut handles: Vec<HMONITOR> = Vec::new();

    // SEGURIDAD: se pasa la direccion de `handles` como dato opaco y la
    // devolucion de llamada la recupera con el mismo tipo. El vector vive
    // durante toda la llamada, que es sincrona, asi que el puntero es valido
    // mientras `EnumDisplayMonitors` lo use.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_handle),
            LPARAM(std::ptr::from_mut(&mut handles) as isize),
        )
    };

    if ok != TRUE {
        return Err(CaptureError::System(
            "EnumDisplayMonitors fallo; no hay escritorio interactivo".to_owned(),
        ));
    }

    let raws: Vec<RawMonitor> = handles.iter().filter_map(|h| describe(*h)).collect();
    Ok(enumeration::to_layout(&raws))
}

/// Enumera las pantallas devolviendo tambien sus manejadores.
///
/// Los manejadores van en el **mismo orden** que la disposicion, de modo que
/// el indice de un monitor sirve para ambas listas. Es lo que necesita el
/// capturador para pedirle a Windows el monitor correcto.
///
/// Una pantalla cuyos datos no se pueden leer se descarta de las dos listas a
/// la vez, que es lo que mantiene la correspondencia.
///
/// # Errors
///
/// Igual que [`enumerate`].
pub fn enumerate_with_handles() -> CaptureResult<(Vec<HMONITOR>, nexus_core::MonitorLayout)> {
    let mut handles: Vec<HMONITOR> = Vec::new();

    // SEGURIDAD: identica a la de `enumerate`.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_handle),
            LPARAM(std::ptr::from_mut(&mut handles) as isize),
        )
    };

    if ok != TRUE {
        return Err(CaptureError::System(
            "EnumDisplayMonitors fallo; no hay escritorio interactivo".to_owned(),
        ));
    }

    // Se descartan a la vez los manejadores y los datos de las pantallas que no
    // se pueden describir, para que ambas listas sigan alineadas.
    let mut kept_handles = Vec::with_capacity(handles.len());
    let mut raws = Vec::with_capacity(handles.len());
    for handle in handles {
        if let Some(raw) = describe(handle) {
            // `to_layout` tambien descarta las pantallas con dimensiones
            // imposibles, asi que se filtra aqui con el mismo criterio.
            if enumeration::to_monitor_info(&raw, 0).is_some() {
                kept_handles.push(handle);
                raws.push(raw);
            }
        }
    }

    Ok((kept_handles, enumeration::to_layout(&raws)))
}

/// Devolucion de llamada de `EnumDisplayMonitors`: solo recoge los manejadores.
///
/// Se limita a apuntar el manejador y salir. Leer los detalles aqui dentro
/// significaria hacer llamadas al sistema dentro de una enumeracion, que es
/// mejor evitar.
unsafe extern "system" fn collect_handle(
    monitor: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> windows::core::BOOL {
    // SEGURIDAD: `data` es el puntero que se paso en `EnumDisplayMonitors`,
    // que apunta a un `Vec<HMONITOR>` vivo durante toda la enumeracion.
    let handles = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    handles.push(monitor);
    TRUE
}

/// Lee todo lo que se sabe de una pantalla.
///
/// Devuelve `None` solo si no se puede obtener ni su rectangulo, que es el
/// unico dato sin el cual la pantalla no sirve de nada.
fn describe(monitor: HMONITOR) -> Option<RawMonitor> {
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: u32::try_from(size_of::<MONITORINFOEXW>()).ok()?,
            ..Default::default()
        },
        szDevice: [0; 32],
    };

    // SEGURIDAD: `GetMonitorInfoW` espera un puntero a `MONITORINFO` cuyo
    // `cbSize` declare el tamano real de la estructura. Al declarar el tamano
    // de `MONITORINFOEXW` la API rellena tambien `szDevice`, que es el motivo
    // de usar la variante extendida. La conversion de puntero es la que la
    // propia API documenta.
    let ok =
        unsafe { GetMonitorInfoW(monitor, std::ptr::from_mut(&mut info).cast::<MONITORINFO>()) };
    if ok != TRUE {
        return None;
    }

    let rect = info.monitorInfo.rcMonitor;
    let device_name = String::from_utf16_lossy(&info.szDevice);

    Some(RawMonitor {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
        is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        dpi: read_dpi(monitor),
        refresh_hz: read_refresh_hz(&info.szDevice),
        device_name,
    })
}

/// Puntos por pulgada efectivos de la pantalla.
///
/// Si la llamada falla se devuelve la densidad base, que
/// [`enumeration::dpi_to_scale_percent`] traduce a 100 %.
fn read_dpi(monitor: HMONITOR) -> u32 {
    let mut dpi_x = 0_u32;
    let mut dpi_y = 0_u32;

    // SEGURIDAD: ambos punteros apuntan a variables locales vivas durante la
    // llamada, que es sincrona.
    let result = unsafe {
        GetDpiForMonitor(
            monitor,
            MDT_EFFECTIVE_DPI,
            std::ptr::from_mut(&mut dpi_x),
            std::ptr::from_mut(&mut dpi_y),
        )
    };

    if result.is_ok() && dpi_x > 0 {
        dpi_x
    } else {
        enumeration::BASE_DPI
    }
}

/// Frecuencia de refresco actual de la pantalla.
///
/// Devuelve `None` si el sistema no la reporta. No es motivo para descartar la
/// pantalla: es informacion para la interfaz.
fn read_refresh_hz(device_name: &[u16; 32]) -> Option<u16> {
    let mut mode = DEVMODEW {
        dmSize: u16::try_from(size_of::<DEVMODEW>()).ok()?,
        ..Default::default()
    };

    // SEGURIDAD: `device_name` es un array de 32 unidades terminado en cero que
    // vive en el llamante, y `mode` es una estructura local con su `dmSize`
    // correctamente declarado, que es lo que la API exige.
    let ok = unsafe {
        EnumDisplaySettingsW(
            PCWSTR(device_name.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            std::ptr::from_mut(&mut mode),
        )
    };

    if ok == TRUE {
        u16::try_from(mode.dmDisplayFrequency).ok()
    } else {
        None
    }
}
