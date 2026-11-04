//! El rasgo del almacen y el formato de lo que se guarda.
//!
//! ## Por que el archivo lleva una marca
//!
//! Hay PCs con una clave guardada desde antes de que existiera el cifrado. Al
//! leer el archivo hay que saber si lo que hay esta cifrado o no, y hay que
//! saberlo **sin equivocarse**: tratar una clave en claro como cifrada da
//! basura, y tratar una cifrada como si estuviera en claro da una identidad
//! que no es la del equipo. Las dos cosas acabarian invalidando los
//! dispositivos ya emparejados.
//!
//! Por eso lo cifrado empieza por una marca que lo identifica. Lo que no la
//! lleva es una clave antigua en claro, y se migra al cifrarla la primera vez
//! que el host arranca.

use crate::error::{VaultError, VaultResult};

/// Marca con la que empieza todo archivo cifrado por este crate.
///
/// Lleva version para que un formato futuro se pueda rechazar en lugar de
/// malinterpretarse.
pub const SEALED_MAGIC: &[u8] = b"NEXUS-KEY-v1\n";

/// Indica si lo guardado esta cifrado por este crate.
///
/// Lo que no lleva la marca es una clave antigua guardada en claro.
#[must_use]
pub fn is_sealed(stored: &[u8]) -> bool {
    stored.starts_with(SEALED_MAGIC)
}

/// Protege secretos para guardarlos en disco.
pub trait Vault: Send + Sync {
    /// Nombre estable del almacen, para registros y para el panel.
    fn name(&self) -> &'static str;

    /// Indica si lo guardado queda protegido de otro programa del mismo
    /// usuario.
    ///
    /// Es lo que de verdad distingue a un almacen del sistema de guardar el
    /// archivo tal cual. El panel lo muestra: una limitacion que nadie ve es
    /// una limitacion que nadie arregla.
    fn protects_at_rest(&self) -> bool;

    /// Cifra el secreto y devuelve lo que hay que guardar, marca incluida.
    ///
    /// # Errors
    ///
    /// [`VaultError::Sealing`] si el sistema rechaza la operacion.
    fn seal(&self, plain: &[u8]) -> VaultResult<Vec<u8>>;

    /// Recupera el secreto de lo guardado.
    ///
    /// # Errors
    ///
    /// - [`VaultError::Corrupt`] si no lleva la marca o esta truncado.
    /// - [`VaultError::Unsealing`] si el sistema no puede descifrarlo.
    fn unseal(&self, sealed: &[u8]) -> VaultResult<Vec<u8>>;
}

/// Separa la marca de la carga util.
///
/// # Errors
///
/// [`VaultError::Corrupt`] si no empieza por la marca.
pub fn strip_magic(sealed: &[u8]) -> VaultResult<&[u8]> {
    sealed
        .strip_prefix(SEALED_MAGIC)
        .ok_or(VaultError::Corrupt {
            reason: "no empieza por la marca de este formato",
        })
}

/// Almacen que **no** cifra: guarda el secreto tal cual.
///
/// Es lo que se usa fuera de Windows, donde este proyecto no tiene almacen del
/// sistema. No finge: [`Vault::protects_at_rest`] devuelve `false`, y el host
/// lo muestra en el panel.
#[derive(Debug, Default)]
pub struct PlainVault;

impl Vault for PlainVault {
    fn name(&self) -> &'static str {
        "sin-cifrar"
    }

    fn protects_at_rest(&self) -> bool {
        false
    }

    fn seal(&self, plain: &[u8]) -> VaultResult<Vec<u8>> {
        // Lleva la marca igualmente: asi el archivo se lee del mismo modo en
        // las dos plataformas y migrar entre ellas no depende del tamano.
        let mut out = Vec::with_capacity(SEALED_MAGIC.len() + plain.len());
        out.extend_from_slice(SEALED_MAGIC);
        out.extend_from_slice(plain);
        Ok(out)
    }

    fn unseal(&self, sealed: &[u8]) -> VaultResult<Vec<u8>> {
        Ok(strip_magic(sealed)?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lo_cifrado_se_reconoce_por_la_marca() {
        let vault = PlainVault;
        let guardado = vault.seal(b"clave secreta").expect("cifra");
        assert!(is_sealed(&guardado));
    }

    #[test]
    fn una_clave_antigua_en_claro_no_lleva_marca() {
        // El caso que hace falta distinguir: 32 bytes sueltos de un PC que ya
        // tenia identidad antes de que existiera el cifrado.
        let antigua = [7_u8; 32];
        assert!(!is_sealed(&antigua));
    }

    #[test]
    fn el_almacen_sin_cifrar_devuelve_lo_mismo_que_guardo() {
        let vault = PlainVault;
        let secreto = b"treinta y dos bytes de clave....";
        let guardado = vault.seal(secreto).expect("cifra");
        assert_eq!(vault.unseal(&guardado).expect("descifra"), secreto);
    }

    #[test]
    fn el_almacen_sin_cifrar_no_dice_que_protege() {
        // Lo importante de este almacen: que no mienta.
        let vault = PlainVault;
        assert!(!vault.protects_at_rest());
        assert_eq!(vault.name(), "sin-cifrar");
    }

    #[test]
    fn sin_la_marca_no_se_intenta_descifrar() {
        let vault = PlainVault;
        for basura in [
            &b""[..],
            &b"NEXUS"[..],
            &[0_u8; 32][..],
            &b"NEXUS-KEY-v2\n datos"[..],
        ] {
            assert_eq!(
                vault.unseal(basura).unwrap_err(),
                VaultError::Corrupt {
                    reason: "no empieza por la marca de este formato"
                },
                "{basura:?} deberia rechazarse"
            );
        }
    }

    #[test]
    fn una_version_futura_del_formato_se_rechaza_en_lugar_de_leerse_mal() {
        // Si un formato futuro se leyera como el actual, la clave saldria
        // distinta y el equipo perderia su identidad sin avisar.
        let mut futuro = b"NEXUS-KEY-v9\n".to_vec();
        futuro.extend_from_slice(&[1_u8; 32]);
        assert!(!is_sealed(&futuro));
        assert!(PlainVault.unseal(&futuro).is_err());
    }

    #[test]
    fn un_secreto_vacio_sigue_dando_un_archivo_con_marca() {
        let guardado = PlainVault.seal(b"").expect("cifra");
        assert_eq!(guardado, SEALED_MAGIC);
        assert!(PlainVault.unseal(&guardado).expect("descifra").is_empty());
    }
}
