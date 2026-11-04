//! El rasgo de energia y los backends que no tocan el sistema.

use nexus_core::protocol::control::PowerAction;

use crate::error::{PowerError, PowerResult};

/// Ejecuta operaciones de energia sobre el equipo.
///
/// Es un rasgo para que el host use el del sistema y las pruebas uno que
/// anota. Asi toda la logica que decide **si** una operacion se ejecuta —el
/// permiso y la confirmacion de una persona— se prueba sin apagar nada.
pub trait PowerControl: Send {
    /// Nombre estable del backend, para registros y diagnostico.
    fn name(&self) -> &'static str;

    /// Ejecuta la operacion.
    ///
    /// # Errors
    ///
    /// - [`PowerError::UnsupportedPlatform`] fuera de Windows.
    /// - [`PowerError::PrivilegeDenied`] si la cuenta no puede apagar.
    /// - [`PowerError::System`] ante cualquier otro fallo del sistema.
    fn execute(&mut self, action: PowerAction) -> PowerResult<()>;

    /// Comprueba si este equipo podria apagarse, **sin apagarlo**.
    ///
    /// Sirve para avisar antes y no cuando ya haga falta: en Windows, apagar
    /// exige un privilegio que una cuenta puede no tener, y descubrirlo el
    /// dia que se necesita es tarde.
    ///
    /// # Errors
    ///
    /// Las mismas que [`PowerControl::execute`], salvo que aqui no se ejecuta
    /// ninguna operacion.
    fn probe(&mut self) -> PowerResult<()>;
}

/// Backend que anota lo que se le pide y no toca el equipo.
///
/// Es el que usan las pruebas: comprobar que una confirmacion funciona no
/// puede exigir apagar la maquina donde corren las pruebas.
#[derive(Debug, Default, Clone)]
pub struct RecordingPower {
    executed: Vec<PowerAction>,
}

impl RecordingPower {
    /// Crea uno vacio.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            executed: Vec::new(),
        }
    }

    /// Operaciones que se le han pedido, en orden.
    #[must_use]
    pub fn executed(&self) -> &[PowerAction] {
        &self.executed
    }
}

impl PowerControl for RecordingPower {
    fn name(&self) -> &'static str {
        "anotador"
    }

    fn execute(&mut self, action: PowerAction) -> PowerResult<()> {
        self.executed.push(action);
        Ok(())
    }

    fn probe(&mut self) -> PowerResult<()> {
        Ok(())
    }
}

/// Backend de las plataformas que no tienen energia implementada.
///
/// Falla de forma explicita en lugar de devolver exito sin hacer nada. Un
/// «apagado correcto» que no apaga seria peor que un error.
#[derive(Debug, Default)]
pub struct UnsupportedPower;

impl PowerControl for UnsupportedPower {
    fn name(&self) -> &'static str {
        "no-disponible"
    }

    fn execute(&mut self, _action: PowerAction) -> PowerResult<()> {
        Err(PowerError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }

    fn probe(&mut self) -> PowerResult<()> {
        Err(PowerError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_anotador_guarda_lo_que_se_le_pide_sin_tocar_nada() {
        let mut power = RecordingPower::new();
        assert!(power.executed().is_empty());

        power.execute(PowerAction::Lock).expect("anota");
        power.execute(PowerAction::Reboot).expect("anota");
        assert_eq!(power.executed(), [PowerAction::Lock, PowerAction::Reboot]);
    }

    #[test]
    fn la_comprobacion_no_ejecuta_nada() {
        // Si `probe` apagara algo, esta prueba apagaria la maquina donde
        // corre. Que no lo haga es justamente lo que se comprueba.
        let mut power = RecordingPower::new();
        power.probe().expect("la comprobacion deberia salir bien");
        assert!(
            power.executed().is_empty(),
            "comprobar no puede ejecutar nada"
        );
    }

    #[test]
    fn fuera_de_windows_falla_diciendo_por_que() {
        let mut power = UnsupportedPower;
        assert_eq!(power.name(), "no-disponible");
        for action in [
            PowerAction::Lock,
            PowerAction::Reboot,
            PowerAction::Shutdown,
        ] {
            assert!(
                matches!(
                    power.execute(action).unwrap_err(),
                    PowerError::UnsupportedPlatform { .. }
                ),
                "{action:?} deberia fallar de forma explicita"
            );
        }
    }
}
