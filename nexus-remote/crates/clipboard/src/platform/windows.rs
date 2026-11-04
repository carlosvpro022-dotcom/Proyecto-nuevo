//! Portapapeles de Windows.
//!
//! # Estado de verificacion
//!
//! Este modulo **se compila para Windows pero no se ha ejecutado en Windows**.
//! La comprobacion que se le hace es la de nivel 2 del [ADR-0007]:
//! compilacion en cruce a `x86_64-pc-windows-gnu`, que verifica tipos, nombres
//! de API y firmas, pero no el comportamiento.
//!
//! Lo que si esta probado de verdad es todo lo que decide **si** una operacion
//! se hace y con que limites ([`crate::access`]). Este archivo es la parte fina
//! que traduce esas decisiones ya tomadas a llamadas del sistema.
//!
//! [ADR-0007]: ../../../../docs/architecture/ADR-0007-verificacion-multiplataforma.md
//!
//! # Las tres trampas de esta API
//!
//! El portapapeles de Win32 es una API con estado global y tres detalles que
//! se hacen mal con facilidad. Los tres estan resueltos aqui, y por eso cada
//! uno lleva su comentario:
//!
//! 1. **Hay que cerrarlo siempre.** El portapapeles es un recurso global: si
//!    se abre y no se cierra, ningun programa del sistema puede usarlo hasta
//!    que este proceso muera. Se cierra desde un `Drop`, de modo que tambien
//!    se cierra si algo entra en panico a mitad.
//! 2. **`GlobalUnlock` devuelve falso cuando acierta.** Devuelve falso tanto
//!    si falla como cuando el contador de bloqueos llega a cero, que es el caso
//!    normal. Tratar ese falso como error haria fallar toda lectura correcta.
//! 3. **Quien escribe deja de ser el dueno de la memoria.** Si
//!    `SetClipboardData` tiene exito, el sistema se queda con el bloque y
//!    liberarlo seria un uso despues de liberar. Si falla, el dueno sigue
//!    siendo este codigo y hay que liberarlo o se pierde.

// Las llamadas a la API de Windows son intrinsecamente inseguras para Rust.
// Cada bloque `unsafe` de este archivo lleva su justificacion.
#![allow(unsafe_code)]

use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::access::{Clipboard, check_text};
use crate::error::{ClipboardError, ClipboardResult};

/// Intentos de abrir el portapapeles antes de darse por vencido.
///
/// Otro programa puede tenerlo abierto un instante. Reintentar unas pocas
/// veces evita fallar por un cruce de milisegundos; insistir mas seria dejar
/// la sesion colgada por algo que no se va a arreglar solo.
const OPEN_ATTEMPTS: u32 = 5;

/// Espera entre intentos de abrir el portapapeles.
const OPEN_RETRY_WAIT: Duration = Duration::from_millis(20);

/// Portapapeles del sistema en Windows.
#[derive(Debug, Default)]
pub struct WindowsClipboard;

impl WindowsClipboard {
    /// Crea el backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Portapapeles abierto que se cierra solo.
///
/// Existe por la trampa 1: el portapapeles es global y dejarlo abierto lo
/// bloquea para todo el sistema. Con esto se cierra tambien si el codigo de en
/// medio sale por un camino inesperado.
struct OpenGuard;

impl OpenGuard {
    /// Abre el portapapeles, reintentando si otro programa lo tiene.
    fn acquire() -> ClipboardResult<Self> {
        for intento in 0..OPEN_ATTEMPTS {
            // SEGURIDAD: `OpenClipboard` con `None` asocia el portapapeles a
            // la tarea actual sin ventana propia, que es lo que corresponde a
            // un proceso sin interfaz. No recibe punteros.
            let abierto = unsafe { OpenClipboard(None) };
            if abierto.is_ok() {
                return Ok(Self);
            }
            if intento + 1 < OPEN_ATTEMPTS {
                std::thread::sleep(OPEN_RETRY_WAIT);
            }
        }
        Err(ClipboardError::Busy)
    }
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        // SEGURIDAD: cierra el portapapeles que esta misma tarea abrio. No
        // recibe punteros. Si fallara no hay nada que hacer salvo dejar
        // constancia: propagar el error desde un `Drop` no es posible.
        if let Err(error) = unsafe { CloseClipboard() } {
            tracing::warn!(%error, "no se pudo cerrar el portapapeles de Windows");
        }
    }
}

impl Clipboard for WindowsClipboard {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn read_text(&mut self) -> ClipboardResult<String> {
        let _guard = OpenGuard::acquire()?;

        // SEGURIDAD: consulta de formato disponible, sin punteros.
        let hay_texto = unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)) };
        if hay_texto.is_err() {
            return Err(ClipboardError::Empty);
        }

        // SEGURIDAD: devuelve un identificador cuyo dueno es el portapapeles.
        // No se libera aqui: hacerlo seria liberar memoria ajena.
        let handle: HANDLE = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT.0)) }
            .map_err(|e| ClipboardError::System(e.to_string()))?;
        if handle.is_invalid() {
            return Err(ClipboardError::Empty);
        }
        let bloque = HGLOBAL(handle.0);

        // SEGURIDAD: bloquea el bloque recien obtenido y devuelve un puntero
        // valido mientras no se desbloquee. El tamano se pregunta al sistema
        // en lugar de recorrer hasta encontrar un cero, para no salirse del
        // bloque si el contenido estuviera mal formado.
        let bytes = unsafe { GlobalSize(bloque) };
        let puntero = unsafe { GlobalLock(bloque) }.cast::<u16>();
        if puntero.is_null() || bytes < 2 {
            // Trampa 2: el falso de `GlobalUnlock` tambien significa exito.
            unsafe { drop(GlobalUnlock(bloque)) };
            return Err(ClipboardError::Empty);
        }

        // SEGURIDAD: `bytes` lo dio el propio sistema para este bloque, asi
        // que `bytes / 2` unidades de 16 bits caben dentro de el.
        let unidades = unsafe { std::slice::from_raw_parts(puntero, bytes / 2) };
        let fin = unidades
            .iter()
            .position(|u| *u == 0)
            .unwrap_or(unidades.len());
        let texto = String::from_utf16_lossy(&unidades[..fin]);

        // Trampa 2 otra vez: se ignora el resultado a proposito.
        unsafe { drop(GlobalUnlock(bloque)) };

        Ok(texto)
    }

    fn write_text(&mut self, text: &str) -> ClipboardResult<()> {
        check_text(text)?;

        // UTF-16 terminado en cero, que es lo que espera `CF_UNICODETEXT`.
        let mut unidades: Vec<u16> = text.encode_utf16().collect();
        unidades.push(0);
        let bytes = unidades.len() * std::mem::size_of::<u16>();

        let _guard = OpenGuard::acquire()?;

        // SEGURIDAD: vacia el portapapeles de esta tarea, que ya esta abierto.
        unsafe { EmptyClipboard() }.map_err(|e| ClipboardError::System(e.to_string()))?;

        // SEGURIDAD: reserva un bloque movible del tamano calculado arriba.
        let bloque = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }
            .map_err(|e| ClipboardError::System(e.to_string()))?;

        // SEGURIDAD: bloquea el bloque recien reservado. Si no se pudiera, se
        // libera: su dueno sigue siendo este codigo (trampa 3).
        let destino = unsafe { GlobalLock(bloque) }.cast::<u16>();
        if destino.is_null() {
            unsafe { drop(GlobalFree(Some(bloque))) };
            return Err(ClipboardError::System(
                "no se pudo bloquear la memoria del portapapeles".to_owned(),
            ));
        }

        // SEGURIDAD: se copian exactamente las unidades reservadas, en un
        // bloque recien pedido para ellas y que nadie mas tiene todavia.
        unsafe { std::ptr::copy_nonoverlapping(unidades.as_ptr(), destino, unidades.len()) };
        unsafe { drop(GlobalUnlock(bloque)) };

        // SEGURIDAD: entrega el bloque al portapapeles. Trampa 3: si esto sale
        // bien, el sistema pasa a ser el dueno y liberarlo seria un uso
        // despues de liberar; si falla, el dueno sigue siendo este codigo.
        match unsafe { SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(bloque.0))) } {
            Ok(_) => Ok(()),
            Err(error) => {
                unsafe { drop(GlobalFree(Some(bloque))) };
                Err(ClipboardError::System(error.to_string()))
            }
        }
    }
}
