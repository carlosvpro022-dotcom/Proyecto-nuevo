//! Contrato de los backends de captura.
//!
//! # Estado de implementacion
//!
//! El backend de Windows vive en [`crate::platform::windows_capture`]. Aqui
//! viven el contrato y dos implementaciones auxiliares:
//!
//! - [`UnsupportedCapturer`]: falla de forma explicita. Es lo que usa el host
//!   cuando se compila para una plataforma sin captura, para que el fallo sea
//!   visible en lugar de silencioso.
//! - [`SyntheticCapturer`]: genera fotogramas de color plano. Sirve para
//!   verificar el resto de la cadena (ritmo, codificacion, protocolo) sin
//!   pantalla real. **No es captura**; su nombre lo dice para que no se
//!   confunda con una implementacion terminada.

use nexus_core::Timestamp;
use nexus_core::monitor::MonitorLayout;

use crate::error::{CaptureError, CaptureResult};
use crate::frame::{Frame, PixelFormat};

/// Backend capaz de enumerar monitores y capturar su contenido.
pub trait ScreenCapturer {
    /// Nombre del backend, para registros y diagnostico.
    fn name(&self) -> &'static str;

    /// Enumera los monitores conectados.
    ///
    /// # Errors
    ///
    /// [`CaptureError::UnsupportedPlatform`] si el backend no funciona aqui, o
    /// [`CaptureError::System`] si el sistema falla al enumerar.
    fn enumerate_monitors(&self) -> CaptureResult<MonitorLayout>;

    /// Selecciona el monitor a capturar.
    ///
    /// # Errors
    ///
    /// [`CaptureError::UnknownMonitor`] si el indice no existe.
    fn select_monitor(&mut self, index: u8) -> CaptureResult<()>;

    /// Captura el siguiente fotograma disponible.
    ///
    /// # Errors
    ///
    /// [`CaptureError::WouldBlock`] si aun no hay fotograma nuevo (no es un
    /// fallo), o cualquier otra variante si la captura se rompe.
    fn capture_frame(&mut self, now: Timestamp) -> CaptureResult<Frame>;
}

/// Backend para plataformas sin captura.
#[derive(Debug, Default)]
pub struct UnsupportedCapturer;

impl ScreenCapturer for UnsupportedCapturer {
    fn name(&self) -> &'static str {
        "unsupported"
    }

    fn enumerate_monitors(&self) -> CaptureResult<MonitorLayout> {
        Err(CaptureError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }

    fn select_monitor(&mut self, _index: u8) -> CaptureResult<()> {
        Err(CaptureError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }

    fn capture_frame(&mut self, _now: Timestamp) -> CaptureResult<Frame> {
        Err(CaptureError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }
}

/// Backend de fotogramas sinteticos. **No es captura real.**
#[derive(Debug)]
pub struct SyntheticCapturer {
    layout: MonitorLayout,
    selected: u8,
    counter: u8,
}

impl SyntheticCapturer {
    /// Crea el backend sobre una disposicion de monitores dada.
    ///
    /// # Panics
    ///
    /// Si la disposicion esta vacia: un backend sin monitores no tiene uso y es
    /// mejor detectarlo al construirlo que al capturar.
    #[must_use]
    pub fn new(layout: MonitorLayout) -> Self {
        assert!(
            !layout.is_empty(),
            "el backend sintetico necesita al menos un monitor"
        );
        let selected = layout.primary().map_or(0, |m| m.index);
        Self {
            layout,
            selected,
            counter: 0,
        }
    }

    /// Monitor seleccionado ahora mismo.
    #[must_use]
    pub const fn selected_monitor(&self) -> u8 {
        self.selected
    }
}

impl ScreenCapturer for SyntheticCapturer {
    fn name(&self) -> &'static str {
        "synthetic"
    }

    fn enumerate_monitors(&self) -> CaptureResult<MonitorLayout> {
        Ok(self.layout.clone())
    }

    fn select_monitor(&mut self, index: u8) -> CaptureResult<()> {
        if self.layout.get(index).is_none() {
            return Err(CaptureError::UnknownMonitor { index });
        }
        self.selected = index;
        Ok(())
    }

    fn capture_frame(&mut self, now: Timestamp) -> CaptureResult<Frame> {
        let monitor = self
            .layout
            .get(self.selected)
            .ok_or(CaptureError::UnknownMonitor {
                index: self.selected,
            })?;

        // Color plano que cambia en cada fotograma: suficiente para comprobar
        // que los fotogramas llegan y se distinguen entre si.
        self.counter = self.counter.wrapping_add(17);
        let stride = monitor.width as usize * 4;
        let frame = Frame {
            width: monitor.width,
            height: monitor.height,
            format: PixelFormat::Bgra8,
            stride,
            data: vec![self.counter; stride * monitor.height as usize],
            captured_at: now,
            monitor_index: self.selected,
        };
        frame.validate()?;
        Ok(frame)
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
                name: "A".to_owned(),
                width: 64,
                height: 32,
                origin_x: 0,
                origin_y: 0,
                scale_percent: 100,
                is_primary: true,
                refresh_hz: Some(60),
            },
            MonitorInfo {
                index: 1,
                name: "B".to_owned(),
                width: 32,
                height: 16,
                origin_x: 64,
                origin_y: 0,
                scale_percent: 100,
                is_primary: false,
                refresh_hz: None,
            },
        ])
    }

    #[test]
    fn the_unsupported_backend_never_pretends_to_work() {
        let mut capturer = UnsupportedCapturer;
        assert!(matches!(
            capturer.enumerate_monitors().unwrap_err(),
            CaptureError::UnsupportedPlatform { .. }
        ));
        assert!(capturer.select_monitor(0).is_err());
        assert!(capturer.capture_frame(Timestamp(0)).is_err());
    }

    #[test]
    fn the_synthetic_backend_starts_on_the_primary_monitor() {
        let capturer = SyntheticCapturer::new(layout());
        assert_eq!(capturer.selected_monitor(), 0);
        assert_eq!(capturer.enumerate_monitors().expect("enumera").len(), 2);
    }

    #[test]
    fn selecting_a_valid_monitor_changes_the_frame_size() {
        let mut capturer = SyntheticCapturer::new(layout());
        let first = capturer.capture_frame(Timestamp(1)).expect("captura");
        assert_eq!((first.width, first.height), (64, 32));

        capturer.select_monitor(1).expect("existe");
        let second = capturer.capture_frame(Timestamp(2)).expect("captura");
        assert_eq!((second.width, second.height), (32, 16));
        assert_eq!(second.monitor_index, 1);
    }

    #[test]
    fn selecting_a_missing_monitor_fails_and_keeps_the_previous_one() {
        let mut capturer = SyntheticCapturer::new(layout());
        assert_eq!(
            capturer.select_monitor(9).unwrap_err(),
            CaptureError::UnknownMonitor { index: 9 }
        );
        assert_eq!(
            capturer.selected_monitor(),
            0,
            "la seleccion no debe cambiar al fallar"
        );
    }

    #[test]
    fn synthetic_frames_are_always_valid_and_distinguishable() {
        let mut capturer = SyntheticCapturer::new(layout());
        let a = capturer.capture_frame(Timestamp(1)).expect("captura");
        let b = capturer.capture_frame(Timestamp(2)).expect("captura");
        a.validate().expect("valido");
        b.validate().expect("valido");
        assert_ne!(
            a.data, b.data,
            "cada fotograma debe diferenciarse del anterior"
        );
        assert_eq!(a.captured_at, Timestamp(1));
    }

    #[test]
    #[should_panic(expected = "al menos un monitor")]
    fn a_synthetic_backend_without_monitors_is_rejected() {
        let _ = SyntheticCapturer::new(MonitorLayout::default());
    }
}
