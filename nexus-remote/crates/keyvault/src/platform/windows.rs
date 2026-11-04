//! Almacen de claves de Windows, con DPAPI.
//!
//! # Estado de verificacion
//!
//! Este modulo **se compila para Windows pero no se ha ejecutado en Windows**.
//! La comprobacion que se le hace es la de nivel 2 del [ADR-0007]:
//! compilacion en cruce a `x86_64-pc-windows-gnu`.
//!
//! Lo que si esta probado es el formato de lo que se guarda y la migracion de
//! una clave antigua ([`crate::vault`]), que es donde esta el riesgo de perder
//! la identidad del equipo.
//!
//! [ADR-0007]: ../../../../docs/architecture/ADR-0007-verificacion-multiplataforma.md
//!
//! # Que protege DPAPI y que no
//!
//! `CryptProtectData` con ambito de usuario ata el descifrado a la **cuenta de
//! Windows**. El archivo copiado a otro equipo, o leido desde otra cuenta, no
//! se puede descifrar.
//!
//! Lo que **no** protege: un programa que corra como esa misma cuenta puede
//! pedirle a DPAPI que descifre, igual que hace el host. Sube el liston —ya no
//! basta con copiar el archivo— pero no lo hace imposible. Protegerse de eso
//! exigiria pedirle una contrasena a la persona en cada arranque, y eso es
//! incompatible con el acceso desatendido, que es el motivo de existir de este
//! producto.
//!
//! # La entropia adicional
//!
//! Se pasa una cadena fija como `poptionalentropy`. No es un secreto —esta en
//! el codigo— y no pretende serlo: lo que hace es atar el descifrado a esta
//! aplicacion, de modo que otro programa de la misma cuenta no pueda
//! descifrarlo por accidente pidiendoselo a DPAPI sin mas.

// Las llamadas a la API de Windows son intrinsecamente inseguras para Rust.
// Cada bloque `unsafe` de este archivo lleva su justificacion.
#![allow(unsafe_code)]

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

use zeroize::Zeroizing;

use crate::error::{VaultError, VaultResult};
use crate::vault::{SEALED_MAGIC, Vault, strip_magic};

/// Ata el descifrado a esta aplicacion. Ver la nota del modulo: no es un
/// secreto.
const ENTROPY: &[u8] = b"NEXUS-REMOTE/v1/host-identity";

/// Almacen de claves respaldado por DPAPI.
#[derive(Debug, Default)]
pub struct WindowsVault;

impl WindowsVault {
    /// Crea el almacen.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Bloque devuelto por DPAPI, que se libera solo.
///
/// DPAPI reserva la memoria del resultado y el llamante tiene que liberarla
/// con `LocalFree`. Hacerlo desde un `Drop` evita perderla si el camino de en
/// medio sale por un error.
struct DpapiBuffer(CRYPT_INTEGER_BLOB);

impl DpapiBuffer {
    /// Copia el contenido a un vector propio.
    ///
    /// El bloque original lo borra `drop`: DPAPI lo reservo, y al descifrar
    /// contiene la clave privada en claro.
    fn to_vec(&self) -> Vec<u8> {
        if self.0.pbData.is_null() || self.0.cbData == 0 {
            return Vec::new();
        }
        // SEGURIDAD: puntero y longitud los acaba de escribir DPAPI para este
        // bloque, y siguen validos hasta que se libere en `drop`.
        unsafe { std::slice::from_raw_parts(self.0.pbData, self.0.cbData as usize) }.to_vec()
    }
}

impl Drop for DpapiBuffer {
    fn drop(&mut self) {
        if self.0.pbData.is_null() {
            return;
        }
        // Al descifrar, este bloque tiene la clave privada en claro. Se borra
        // antes de devolverlo al sistema para no dejarla en memoria liberada.
        // SEGURIDAD: el puntero y el tamano son los que escribio DPAPI para
        // este bloque, que sigue siendo suyo hasta el `LocalFree` de abajo.
        unsafe {
            std::ptr::write_bytes(self.0.pbData, 0, self.0.cbData as usize);
        }
        // SEGURIDAD: libera exactamente el bloque que DPAPI reservo, una sola
        // vez: el puntero se pone a nulo despues.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.pbData.cast())));
        }
        self.0.pbData = std::ptr::null_mut();
    }
}

/// Construye el descriptor que DPAPI espera para unos bytes.
fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).unwrap_or(u32::MAX),
        // DPAPI no modifica la entrada, pero la API la declara mutable.
        pbData: data.as_ptr().cast_mut(),
    }
}

impl Vault for WindowsVault {
    fn name(&self) -> &'static str {
        "windows-dpapi"
    }

    fn protects_at_rest(&self) -> bool {
        true
    }

    fn seal(&self, plain: &[u8]) -> VaultResult<Vec<u8>> {
        let entrada = blob(plain);
        let entropia = blob(ENTROPY);
        let mut salida = CRYPT_INTEGER_BLOB::default();

        // SEGURIDAD: las tres estructuras viven durante toda la llamada y
        // describen memoria propia. `CRYPTPROTECT_UI_FORBIDDEN` garantiza que
        // no intente abrir ninguna ventana: el host puede estar corriendo sin
        // nadie delante.
        unsafe {
            CryptProtectData(
                &raw const entrada,
                None,
                Some(&raw const entropia),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &raw mut salida,
            )
        }
        .map_err(|e| VaultError::Sealing(e.to_string()))?;

        let cifrado = DpapiBuffer(salida).to_vec();
        let mut out = Vec::with_capacity(SEALED_MAGIC.len() + cifrado.len());
        out.extend_from_slice(SEALED_MAGIC);
        out.extend_from_slice(&cifrado);
        Ok(out)
    }

    fn unseal(&self, sealed: &[u8]) -> VaultResult<Zeroizing<Vec<u8>>> {
        let cuerpo = strip_magic(sealed)?;
        if cuerpo.is_empty() {
            return Err(VaultError::Corrupt {
                reason: "el archivo lleva la marca pero no tiene contenido",
            });
        }

        let entrada = blob(cuerpo);
        let entropia = blob(ENTROPY);
        let mut salida = CRYPT_INTEGER_BLOB::default();

        // SEGURIDAD: igual que al cifrar. Un fallo aqui significa que el
        // archivo viene de otra cuenta o de otro equipo, no que haya que
        // generar una identidad nueva.
        unsafe {
            CryptUnprotectData(
                &raw const entrada,
                None,
                Some(&raw const entropia),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &raw mut salida,
            )
        }
        .map_err(|e| VaultError::Unsealing(e.to_string()))?;

        Ok(Zeroizing::new(DpapiBuffer(salida).to_vec()))
    }
}
