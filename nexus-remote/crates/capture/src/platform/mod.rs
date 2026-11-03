//! Backends de plataforma para la captura de pantalla.
//!
//! Cada plataforma tiene su modulo, activado en tiempo de compilacion. El
//! resto del crate solo conoce los contratos de [`crate::capturer`].

#[cfg(windows)]
pub mod windows_capture;
#[cfg(windows)]
pub mod windows_monitors;

use crate::capturer::ScreenCapturer;
use crate::error::CaptureResult;

/// Enumera los monitores de esta plataforma.
///
/// # Errors
///
/// [`CaptureError::UnsupportedPlatform`](crate::error::CaptureError::UnsupportedPlatform)
/// fuera de Windows, o el error del sistema si la enumeracion falla.
pub fn enumerate_monitors() -> CaptureResult<nexus_core::MonitorLayout> {
    #[cfg(windows)]
    {
        windows_monitors::enumerate()
    }
    #[cfg(not(windows))]
    {
        Err(crate::error::CaptureError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }
}

/// Crea el capturador que corresponde a esta plataforma.
///
/// # Errors
///
/// Fuera de Windows devuelve un capturador que falla de forma explicita en
/// cada operacion, no un error: asi el host arranca durante el desarrollo. En
/// Windows, el error de inicializacion de Direct3D si lo hubiera.
pub fn default_capturer() -> CaptureResult<Box<dyn ScreenCapturer>> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows_capture::WindowsCapturer::new()?))
    }
    #[cfg(not(windows))]
    {
        Ok(Box::new(crate::capturer::UnsupportedCapturer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuera_de_windows_la_enumeracion_lo_dice_claramente() {
        let result = enumerate_monitors();
        if cfg!(windows) {
            // En Windows deberia funcionar; aqui solo se comprueba que no
            // entra en panico.
            let _ = result;
        } else {
            let err = result.unwrap_err();
            assert!(
                matches!(err, crate::error::CaptureError::UnsupportedPlatform { .. }),
                "{err:?}"
            );
        }
    }

    #[test]
    fn el_capturador_por_omision_se_crea_en_cualquier_plataforma() {
        let capturer = default_capturer().expect("deberia construirse");
        if cfg!(windows) {
            assert_eq!(capturer.name(), "windows-graphics-capture");
        } else {
            assert_eq!(capturer.name(), "unsupported");
        }
    }
}
