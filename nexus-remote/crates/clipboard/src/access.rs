//! El rasgo del portapapeles y los limites que se aplican a todo texto.
//!
//! ## Por que hay un limite de tamano
//!
//! El texto del portapapeles viaja por el canal de control, dentro de un
//! mensaje JSON que no puede pasar de 64 KiB. Un portapapeles puede tener
//! mucho mas: copiar un archivo de codigo entero lo supera sin esfuerzo.
//!
//! Ante eso hay dos salidas, y solo una es aceptable. **Recortar el texto** es
//! peligroso de una forma concreta: quien pega recibe algo que parece
//! completo y no lo es. Media contrasena, medio comando, media direccion. Asi
//! que se rechaza y se dice el tamano, que es informacion con la que la
//! persona puede hacer algo.

use crate::error::{ClipboardError, ClipboardResult};

/// Tamano maximo de texto que se admite, en bytes de UTF-8.
///
/// Sale del tope de 64 KiB del canal de control. Se queda en la mitad porque
/// el texto viaja dentro de un JSON y el escapado puede inflarlo: una comilla
/// ocupa dos bytes, y un caracter de control seis. Con 32 KiB no hay texto
/// que, ya escapado, se salga del mensaje.
pub const MAX_TEXT_BYTES: usize = 32 * 1024;

/// Acceso al portapapeles del equipo.
///
/// Es un rasgo para que el host use el del sistema y las pruebas uno en
/// memoria, sin que la logica que decide **si** se puede leer o escribir
/// cambie entre uno y otro.
pub trait Clipboard: Send {
    /// Nombre estable del backend, para registros y diagnostico.
    fn name(&self) -> &'static str;

    /// Lee el texto del portapapeles.
    ///
    /// # Errors
    ///
    /// - [`ClipboardError::Empty`] si no hay texto.
    /// - [`ClipboardError::Busy`] si otro programa lo tiene abierto.
    /// - [`ClipboardError::UnsupportedPlatform`] fuera de Windows.
    fn read_text(&mut self) -> ClipboardResult<String>;

    /// Escribe texto en el portapapeles.
    ///
    /// # Errors
    ///
    /// Las mismas que [`Clipboard::read_text`], mas los fallos del sistema.
    fn write_text(&mut self, text: &str) -> ClipboardResult<()>;

    /// Comprueba si hay portapapeles, **sin abrirlo y sin leer su contenido**.
    ///
    /// Devuelve si ahora mismo hay texto dentro.
    ///
    /// Existe porque el panel comprueba el estado del equipo cada pocos
    /// segundos, y hacerlo con [`Clipboard::read_text`] tendria dos problemas
    /// serios. El primero: abrir el portapapeles toma un cerrojo global del
    /// sistema, y tomarlo cada pocos segundos hace fallar los copiar y pegar
    /// de otros programas. El segundo: leeria el contenido del portapapeles de
    /// la persona continuamente, sin ninguna necesidad.
    ///
    /// # Errors
    ///
    /// [`ClipboardError::UnsupportedPlatform`] donde no hay backend.
    fn probe(&mut self) -> ClipboardResult<bool>;
}

/// Comprueba que un texto se puede mover por el canal de control.
///
/// Se aplica en las dos direcciones: a lo que llega del dispositivo antes de
/// escribirlo en el equipo, y a lo que se lee del equipo antes de enviarlo.
///
/// # Errors
///
/// - [`ClipboardError::TooLarge`] si pasa de [`MAX_TEXT_BYTES`].
/// - [`ClipboardError::InteriorNul`] si lleva un byte cero.
pub fn check_text(text: &str) -> ClipboardResult<()> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(ClipboardError::TooLarge {
            size_bytes: text.len(),
            max_bytes: MAX_TEXT_BYTES,
        });
    }
    if text.contains('\0') {
        return Err(ClipboardError::InteriorNul);
    }
    Ok(())
}

/// Portapapeles en memoria, para pruebas.
///
/// No toca el sistema. Existe para poder comprobar toda la logica que decide
/// si una operacion se hace y con que limites, en cualquier plataforma.
#[derive(Debug, Default)]
pub struct MemoryClipboard {
    content: Option<String>,
    reads: u32,
    writes: u32,
}

impl MemoryClipboard {
    /// Crea uno vacio.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            content: None,
            reads: 0,
            writes: 0,
        }
    }

    /// Crea uno con contenido inicial.
    #[must_use]
    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            content: Some(text.into()),
            reads: 0,
            writes: 0,
        }
    }

    /// Lo que contiene ahora mismo.
    #[must_use]
    pub fn content(&self) -> Option<&str> {
        self.content.as_deref()
    }

    /// Cuantas veces se leyo.
    #[must_use]
    pub const fn reads(&self) -> u32 {
        self.reads
    }

    /// Cuantas veces se escribio.
    #[must_use]
    pub const fn writes(&self) -> u32 {
        self.writes
    }
}

impl Clipboard for MemoryClipboard {
    fn name(&self) -> &'static str {
        "memoria"
    }

    fn read_text(&mut self) -> ClipboardResult<String> {
        self.reads = self.reads.saturating_add(1);
        self.content.clone().ok_or(ClipboardError::Empty)
    }

    fn write_text(&mut self, text: &str) -> ClipboardResult<()> {
        check_text(text)?;
        self.writes = self.writes.saturating_add(1);
        self.content = Some(text.to_owned());
        Ok(())
    }

    fn probe(&mut self) -> ClipboardResult<bool> {
        Ok(self.content.is_some())
    }
}

/// Portapapeles de las plataformas que no tienen backend.
///
/// Falla de forma explicita en lugar de fingir que funciona. Asi el host puede
/// arrancar en Linux durante el desarrollo sin que nadie confunda un
/// portapapeles vacio con uno que no existe.
#[derive(Debug, Default)]
pub struct UnsupportedClipboard;

impl Clipboard for UnsupportedClipboard {
    fn name(&self) -> &'static str {
        "no-disponible"
    }

    fn read_text(&mut self) -> ClipboardResult<String> {
        Err(ClipboardError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }

    fn write_text(&mut self, _text: &str) -> ClipboardResult<()> {
        Err(ClipboardError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }

    fn probe(&mut self) -> ClipboardResult<bool> {
        Err(ClipboardError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_texto_normal_pasa_la_comprobacion() {
        for texto in ["", "hola", "con\ttabulador\ny salto", "acentos: ñáé", "🙂"] {
            check_text(texto).unwrap_or_else(|e| panic!("{texto:?} deberia valer: {e}"));
        }
    }

    #[test]
    fn un_texto_justo_en_el_limite_pasa() {
        let texto = "x".repeat(MAX_TEXT_BYTES);
        check_text(&texto).expect("el limite exacto deberia valer");
    }

    #[test]
    fn un_texto_que_pasa_del_limite_se_rechaza_en_lugar_de_recortarse() {
        let texto = "x".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(
            check_text(&texto).unwrap_err(),
            ClipboardError::TooLarge {
                size_bytes: MAX_TEXT_BYTES + 1,
                max_bytes: MAX_TEXT_BYTES
            }
        );
    }

    #[test]
    fn el_limite_cuenta_bytes_y_no_caracteres() {
        // Un caracter de cuatro bytes: por caracteres cabria, por bytes no.
        let texto = "🙂".repeat(MAX_TEXT_BYTES / 4 + 1);
        assert!(texto.chars().count() < MAX_TEXT_BYTES);
        assert!(matches!(
            check_text(&texto).unwrap_err(),
            ClipboardError::TooLarge { .. }
        ));
    }

    #[test]
    fn un_byte_cero_intermedio_se_rechaza() {
        // El caso peligroso: el portapapeles de Windows cortaria en el cero y
        // entregaria "contrase" en lugar del texto completo.
        assert_eq!(
            check_text("contrase\0na secreta").unwrap_err(),
            ClipboardError::InteriorNul
        );
    }

    #[test]
    fn el_portapapeles_de_memoria_guarda_y_devuelve() {
        let mut c = MemoryClipboard::new();
        assert_eq!(c.read_text().unwrap_err(), ClipboardError::Empty);

        c.write_text("copiado").expect("escribe");
        assert_eq!(c.read_text().expect("lee"), "copiado");
        assert_eq!(c.content(), Some("copiado"));
        assert_eq!(c.writes(), 1);
        assert_eq!(c.reads(), 2);
    }

    #[test]
    fn el_portapapeles_de_memoria_tambien_aplica_el_limite() {
        let mut c = MemoryClipboard::new();
        let enorme = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(matches!(
            c.write_text(&enorme).unwrap_err(),
            ClipboardError::TooLarge { .. }
        ));
        assert_eq!(c.content(), None, "un rechazo no debe cambiar nada");
    }

    #[test]
    fn se_puede_arrancar_con_contenido() {
        let mut c = MemoryClipboard::with_text("de antes");
        assert_eq!(c.read_text().expect("lee"), "de antes");
    }

    #[test]
    fn comprobar_no_lee_ni_cuenta_como_lectura() {
        // El panel comprueba el estado cada pocos segundos. Si eso contara
        // como leer, estaria leyendo el portapapeles de la persona todo el
        // rato.
        let mut c = MemoryClipboard::with_text("algo copiado");
        assert!(c.probe().expect("comprueba"));
        assert_eq!(c.reads(), 0, "comprobar no es leer");

        let mut vacio = MemoryClipboard::new();
        assert!(!vacio.probe().expect("comprueba"));
        assert_eq!(vacio.reads(), 0);
    }

    #[test]
    fn fuera_de_windows_falla_diciendo_por_que() {
        let mut c = UnsupportedClipboard;
        assert_eq!(c.name(), "no-disponible");
        assert!(matches!(
            c.read_text().unwrap_err(),
            ClipboardError::UnsupportedPlatform { .. }
        ));
        assert!(matches!(
            c.write_text("x").unwrap_err(),
            ClipboardError::UnsupportedPlatform { .. }
        ));
    }
}
