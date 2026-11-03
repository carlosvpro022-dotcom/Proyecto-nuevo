//! Contrato de inyeccion de entrada en el sistema operativo.
//!
//! # Estado de implementacion
//!
//! El backend de Windows vive en [`crate::platform::windows`] y usa
//! `SendInput`. Aqui viven el contrato y dos implementaciones auxiliares:
//! [`RecordingInjector`], que anota lo que recibe y permite verificar el flujo
//! completo (protocolo, permisos, resolucion de coordenadas) sin tocar ningun
//! sistema real, y [`UnsupportedInjector`], que falla de forma explicita en
//! las plataformas sin inyeccion.

use crate::error::{InputError, InputResult};
use crate::resolve::ResolvedInput;

/// Backend capaz de inyectar entrada en el sistema.
pub trait InputInjector {
    /// Nombre del backend, para registros y diagnostico.
    fn name(&self) -> &'static str;

    /// Inyecta una orden ya resuelta y validada.
    ///
    /// # Errors
    ///
    /// [`InputError::UnsupportedPlatform`] si el backend no puede actuar en
    /// esta plataforma, o [`InputError::SystemRejected`] si el sistema rechaza
    /// la llamada.
    fn inject(&mut self, input: &ResolvedInput) -> InputResult<()>;

    /// Libera todas las teclas y botones que pudieran haber quedado pulsados.
    ///
    /// Se llama al cerrar la sesion: si se corta la red con Ctrl pulsado, el PC
    /// no debe quedarse con la tecla hundida.
    ///
    /// # Errors
    ///
    /// Igual que [`Self::inject`].
    fn release_all(&mut self) -> InputResult<()>;
}

/// Backend que solo anota lo que recibe. Para pruebas y diagnostico.
#[derive(Debug, Default)]
pub struct RecordingInjector {
    injected: Vec<ResolvedInput>,
    releases: usize,
}

impl RecordingInjector {
    /// Crea un backend vacio.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ordenes recibidas, en orden.
    #[must_use]
    pub fn injected(&self) -> &[ResolvedInput] {
        &self.injected
    }

    /// Numero de veces que se pidio liberar todo.
    #[must_use]
    pub const fn releases(&self) -> usize {
        self.releases
    }
}

impl InputInjector for RecordingInjector {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn inject(&mut self, input: &ResolvedInput) -> InputResult<()> {
        self.injected.push(input.clone());
        Ok(())
    }

    fn release_all(&mut self) -> InputResult<()> {
        self.releases += 1;
        Ok(())
    }
}

/// Backend para plataformas sin soporte de inyeccion.
///
/// Existe para que el host compile y arranque en Linux durante el desarrollo:
/// falla de forma explicita en lugar de fingir que la orden se ejecuto.
#[derive(Debug, Default)]
pub struct UnsupportedInjector;

impl InputInjector for UnsupportedInjector {
    fn name(&self) -> &'static str {
        "unsupported"
    }

    fn inject(&mut self, _input: &ResolvedInput) -> InputResult<()> {
        Err(InputError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }

    fn release_all(&mut self) -> InputResult<()> {
        Err(InputError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::protocol::input::{KeyState, PointerButton};

    #[test]
    fn the_recording_backend_keeps_what_it_receives() {
        let mut injector = RecordingInjector::new();
        let orders = [
            ResolvedInput::MoveCursor { x: 10, y: 20 },
            ResolvedInput::Button {
                button: PointerButton::Left,
                state: KeyState::Down,
            },
            ResolvedInput::Button {
                button: PointerButton::Left,
                state: KeyState::Up,
            },
        ];
        for order in &orders {
            injector.inject(order).expect("anota");
        }
        assert_eq!(injector.injected(), orders);
        assert_eq!(injector.name(), "recording");
    }

    #[test]
    fn releasing_everything_is_counted() {
        let mut injector = RecordingInjector::new();
        assert_eq!(injector.releases(), 0);
        injector.release_all().expect("libera");
        injector.release_all().expect("libera");
        assert_eq!(injector.releases(), 2);
    }

    #[test]
    fn the_unsupported_backend_fails_loudly_instead_of_pretending() {
        let mut injector = UnsupportedInjector;
        let err = injector
            .inject(&ResolvedInput::MoveCursor { x: 0, y: 0 })
            .unwrap_err();
        assert!(
            matches!(err, InputError::UnsupportedPlatform { .. }),
            "{err:?}"
        );
        assert!(injector.release_all().is_err());
    }
}
