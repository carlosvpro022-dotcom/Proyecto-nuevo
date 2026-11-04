//! Backends de plataforma para el almacen de claves.

#[cfg(windows)]
pub mod windows;

#[cfg(not(windows))]
use crate::vault::PlainVault;
use crate::vault::Vault;

/// Crea el almacen que corresponde a esta plataforma.
///
/// En Windows, DPAPI. En el resto, uno que guarda la clave tal cual y **lo
/// dice**: no se finge una proteccion que no existe.
#[must_use]
pub fn default_vault() -> Box<dyn Vault> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsVault::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(PlainVault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_almacen_por_omision_dice_la_verdad_sobre_si_protege() {
        let vault = default_vault();
        if cfg!(windows) {
            assert_eq!(vault.name(), "windows-dpapi");
            assert!(vault.protects_at_rest());
        } else {
            assert_eq!(vault.name(), "sin-cifrar");
            assert!(
                !vault.protects_at_rest(),
                "fuera de Windows no hay almacen del sistema y no se debe fingir que si"
            );
        }
    }

    #[test]
    fn el_almacen_por_omision_recupera_lo_que_guarda() {
        let vault = default_vault();
        let secreto = [42_u8; 32];
        let guardado = vault.seal(&secreto).expect("cifra");
        assert_ne!(guardado, secreto, "lo guardado lleva al menos la marca");
        assert_eq!(
            vault.unseal(&guardado).expect("descifra").as_slice(),
            secreto
        );
    }
}
