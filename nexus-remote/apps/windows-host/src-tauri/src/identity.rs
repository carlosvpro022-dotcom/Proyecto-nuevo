//! Persistencia de la identidad criptografica del host.
//!
//! El host genera su par de claves Ed25519 la primera vez que arranca y lo
//! reutiliza siempre: su clave publica es lo que los dispositivos verifican
//! para saber que estan hablando con el PC correcto y no con un intermediario.
//!
//! # Como se protege la clave privada
//!
//! El archivo se guarda con los permisos mas restrictivos que permite el
//! sistema (`0600` en Unix) **y** cifrado por el almacen de claves de la
//! plataforma ([`nexus_keyvault`]). Los permisos protegen de otros usuarios;
//! el cifrado protege de otro programa del mismo usuario, que es el caso que
//! importa en un PC domestico de Windows.
//!
//! En Windows eso es DPAPI, que ata el descifrado a la cuenta: el archivo
//! copiado a otro equipo no sirve. Fuera de Windows no hay almacen del
//! sistema y la clave queda en claro, pero se **dice**: el panel lo muestra en
//! lugar de dejar la limitacion escondida en un comentario.
//!
//! # Migrar una clave antigua
//!
//! Hay PCs con una clave guardada desde antes de que existiera el cifrado. Al
//! arrancar se reconoce por no llevar la marca del formato, se carga y se
//! vuelve a guardar cifrada. La migracion es lo mas delicado de este modulo,
//! porque equivocarse pierde la identidad del equipo y con ella todos los
//! dispositivos emparejados. Por eso se escribe primero en un archivo
//! temporal y solo se sustituye el bueno cuando el nuevo esta entero, y por
//! eso una migracion que falle **no impide arrancar**: se avisa y se sigue con
//! la clave que ya funcionaba.

use std::path::Path;

use nexus_core::DeviceId;
use nexus_keyvault::{Vault, default_vault, is_sealed};
use nexus_security::DeviceIdentity;
use nexus_security::identity::SECRET_KEY_LEN;
use zeroize::Zeroizing;

use crate::error::{HostError, HostResult};

/// Nombre del archivo que guarda la clave privada.
pub const KEY_FILE_NAME: &str = "host-identity.key";

/// Nombre del archivo que guarda el identificador del dispositivo.
pub const ID_FILE_NAME: &str = "host-identity.id";

/// Como quedo la clave privada tras cargarla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyState {
    /// Si lo guardado esta cifrado por el almacen del sistema.
    pub protected: bool,
    /// Nombre del almacen en uso.
    pub vault: &'static str,
    /// Si en este arranque se migro una clave antigua que estaba en claro.
    pub migrated: bool,
    /// Por que no esta protegida, cuando no lo esta.
    pub detail: String,
}

/// Identidad del host junto con el estado de su clave.
#[derive(Debug)]
pub struct LoadedIdentity {
    /// La identidad.
    pub identity: DeviceIdentity,
    /// Como quedo la clave en disco.
    pub key: KeyState,
}

/// Carga la identidad del host, generandola la primera vez.
///
/// Si el archivo de clave existe pero no se puede leer o esta corrupto, **no**
/// se genera una identidad nueva: se devuelve error. Generar otra silenciosamente
/// invalidaria todos los dispositivos ya emparejados, que es justo lo contrario
/// de lo que la persona espera.
///
/// # Errors
///
/// [`HostError::Io`] si el directorio no se puede crear o el archivo no se
/// puede leer o escribir, y [`HostError::Security`] si el material guardado no
/// es una clave valida.
pub fn load_or_create(data_dir: &Path) -> HostResult<DeviceIdentity> {
    Ok(load_or_create_with(data_dir, default_vault().as_ref())?.identity)
}

/// Igual, pero diciendo ademas como quedo la clave y con que almacen.
///
/// El almacen se recibe para poder probar la migracion con uno que no dependa
/// de la plataforma: lo que hay que comprobar es que una clave antigua se
/// migra sin perder la identidad, y eso tiene que comprobarse en cualquier
/// sistema.
///
/// # Errors
///
/// Las mismas que [`load_or_create`].
pub fn load_or_create_with(data_dir: &Path, vault: &dyn Vault) -> HostResult<LoadedIdentity> {
    std::fs::create_dir_all(data_dir).map_err(|source| HostError::Io {
        path: data_dir.display().to_string(),
        source,
    })?;

    let key_path = data_dir.join(KEY_FILE_NAME);
    let id_path = data_dir.join(ID_FILE_NAME);

    if key_path.exists() {
        return load(&key_path, &id_path, vault);
    }

    let identity = DeviceIdentity::generate(DeviceId::new());
    let sellada = vault
        .seal(identity.export_secret().as_slice())
        .map_err(|e| HostError::InvalidInput(e.to_string()))?;
    write_secret(&key_path, &sellada)?;
    write_text(&id_path, &identity.device_id().to_string())?;

    tracing::info!(
        device_id = %identity.device_id(),
        fingerprint = %identity.public_key().short_fingerprint(),
        almacen = vault.name(),
        "identidad del host generada"
    );
    Ok(LoadedIdentity {
        identity,
        key: state_of(vault, false),
    })
}

fn load(key_path: &Path, id_path: &Path, vault: &dyn Vault) -> HostResult<LoadedIdentity> {
    let guardado = std::fs::read(key_path).map_err(|source| HostError::Io {
        path: key_path.display().to_string(),
        source,
    })?;

    // Una clave antigua no lleva marca: son los bytes crudos y nada mas. Se
    // reconoce asi y no por el tamano, para no confundirla con un archivo
    // cifrado que casualmente midiera lo mismo.
    // El contenido del archivo va envuelto para que se borre al soltarlo: si
    // la clave estaba en claro, estos son los bytes de la clave privada.
    let guardado = Zeroizing::new(guardado);

    let antigua = !is_sealed(&guardado);
    let secret: Zeroizing<Vec<u8>> = if antigua {
        guardado.clone()
    } else {
        vault.unseal(&guardado).map_err(|e| {
            HostError::InvalidInput(format!(
                "no se pudo descifrar la clave de {}: {e}. \
                 No se genera una identidad nueva para no invalidar los dispositivos emparejados; \
                 si el archivo viene de otro equipo o de otra cuenta de Windows, no sirve aqui",
                key_path.display()
            ))
        })?
    };

    if secret.len() != SECRET_KEY_LEN {
        return Err(HostError::InvalidInput(format!(
            "el archivo de clave {} mide {} bytes en lugar de {SECRET_KEY_LEN}; \
             no se genera una identidad nueva para no invalidar los dispositivos emparejados",
            key_path.display(),
            secret.len()
        )));
    }

    let device_id = std::fs::read_to_string(id_path)
        .map_err(|source| HostError::Io {
            path: id_path.display().to_string(),
            source,
        })?
        .trim()
        .parse::<DeviceId>()
        .map_err(|e| HostError::InvalidInput(format!("identificador del host corrupto: {e}")))?;

    let identity = DeviceIdentity::from_secret_bytes(device_id, &secret)?;

    // La clave ya esta cargada y la sesion puede arrancar. Si la migracion
    // falla, se avisa y se sigue: quedarse sin host porque el cifrado no pudo
    // hacerse seria peor que seguir con la clave que ya funcionaba.
    let migrated = antigua && migrate(key_path, &secret, vault);

    Ok(LoadedIdentity {
        identity,
        key: state_of(vault, migrated),
    })
}

/// Vuelve a guardar una clave antigua, ya cifrada.
///
/// Devuelve si lo consiguio. Se escribe primero aparte y solo se sustituye el
/// archivo bueno cuando el nuevo esta entero: un corte a mitad no puede dejar
/// un archivo de clave truncado, que seria perder la identidad del equipo.
fn migrate(key_path: &Path, secret: &[u8], vault: &dyn Vault) -> bool {
    let sellada = match vault.seal(secret) {
        Ok(sellada) => sellada,
        Err(error) => {
            tracing::warn!(%error, "no se pudo cifrar la clave del host; se deja como estaba");
            return false;
        }
    };

    let temporal = key_path.with_extension("key-nuevo");
    if let Err(error) = std::fs::write(&temporal, &sellada) {
        tracing::warn!(%error, "no se pudo escribir la clave cifrada; se deja como estaba");
        return false;
    }
    if let Err(error) = restrict_permissions(&temporal) {
        tracing::warn!(%error, "no se pudieron ajustar los permisos de la clave cifrada");
        let _ = std::fs::remove_file(&temporal);
        return false;
    }
    if let Err(error) = std::fs::rename(&temporal, key_path) {
        tracing::warn!(%error, "no se pudo sustituir la clave del host; se deja como estaba");
        let _ = std::fs::remove_file(&temporal);
        return false;
    }

    tracing::info!(
        almacen = vault.name(),
        "la clave del host estaba en claro y se ha cifrado"
    );
    true
}

/// Describe como quedo la clave.
fn state_of(vault: &dyn Vault, migrated: bool) -> KeyState {
    let protected = vault.protects_at_rest();
    KeyState {
        protected,
        vault: vault.name(),
        migrated,
        detail: if protected {
            "cifrada con el almacen de claves del sistema".to_owned()
        } else {
            format!(
                "sin cifrar: {} no tiene almacen de claves en este proyecto",
                std::env::consts::OS
            )
        },
    }
}

/// Escribe el material secreto con los permisos mas restrictivos posibles.
fn write_secret(path: &Path, bytes: &[u8]) -> HostResult<()> {
    write_bytes(path, bytes)?;
    restrict_permissions(path)?;
    Ok(())
}

fn write_text(path: &Path, text: &str) -> HostResult<()> {
    write_bytes(path, text.as_bytes())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> HostResult<()> {
    std::fs::write(path, bytes).map_err(|source| HostError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> HostResult<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|source| {
        HostError::Io {
            path: path.display().to_string(),
            source,
        }
    })
}

// Devuelve `Result` aunque aqui nunca falle: la version de Unix si puede
// fallar, y las dos tienen que tener la misma firma para que quien llama no
// sepa en que plataforma esta.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn restrict_permissions(_path: &Path) -> HostResult<()> {
    // En Windows los permisos de archivo no aislan de otro proceso del mismo
    // usuario, asi que ahi no hay nada que ajustar: la proteccion que vale es
    // el cifrado con DPAPI, que ya se aplica al guardar.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Almacen de prueba que si transforma los bytes.
    ///
    /// No es criptografia ni lo pretende: invierte cada byte. Existe para que
    /// las pruebas puedan distinguir «lo guardado es el secreto tal cual» de
    /// «lo guardado paso por el almacen», que es lo que de verdad hay que
    /// comprobar y lo que un almacen que no tocara nada dejaria pasar.
    #[derive(Debug, Default)]
    struct AlmacenDePrueba {
        falla_al_cifrar: bool,
    }

    impl Vault for AlmacenDePrueba {
        fn name(&self) -> &'static str {
            "prueba"
        }

        fn protects_at_rest(&self) -> bool {
            true
        }

        fn seal(&self, plain: &[u8]) -> nexus_keyvault::VaultResult<Vec<u8>> {
            if self.falla_al_cifrar {
                return Err(nexus_keyvault::VaultError::Sealing("de prueba".to_owned()));
            }
            let mut out = nexus_keyvault::SEALED_MAGIC.to_vec();
            out.extend(plain.iter().map(|b| !b));
            Ok(out)
        }

        fn unseal(&self, sealed: &[u8]) -> nexus_keyvault::VaultResult<Zeroizing<Vec<u8>>> {
            let cuerpo = sealed.strip_prefix(nexus_keyvault::SEALED_MAGIC).ok_or(
                nexus_keyvault::VaultError::Corrupt {
                    reason: "sin marca",
                },
            )?;
            Ok(Zeroizing::new(cuerpo.iter().map(|b| !b).collect()))
        }
    }

    /// Almacen que nunca consigue descifrar.
    ///
    /// Es el caso de un archivo cifrado por otra cuenta de Windows o en otro
    /// equipo: DPAPI contesta que no puede.
    struct NoDescifra;

    impl Vault for NoDescifra {
        fn name(&self) -> &'static str {
            "no-descifra"
        }

        fn protects_at_rest(&self) -> bool {
            true
        }

        fn seal(&self, _plain: &[u8]) -> nexus_keyvault::VaultResult<Vec<u8>> {
            unreachable!("no deberia llegar a cifrar")
        }

        fn unseal(&self, _sealed: &[u8]) -> nexus_keyvault::VaultResult<Zeroizing<Vec<u8>>> {
            Err(nexus_keyvault::VaultError::Unsealing(
                "viene de otra cuenta".to_owned(),
            ))
        }
    }

    /// Escribe una clave antigua: los bytes crudos, sin marca, como se
    /// guardaban antes de que existiera el cifrado.
    fn escribir_clave_antigua(dir: &Path) -> DeviceIdentity {
        let identity = DeviceIdentity::generate(DeviceId::new());
        std::fs::write(dir.join(KEY_FILE_NAME), identity.export_secret().as_slice())
            .expect("escribe la clave");
        std::fs::write(dir.join(ID_FILE_NAME), identity.device_id().to_string())
            .expect("escribe el identificador");
        identity
    }

    #[test]
    fn una_clave_nueva_se_guarda_cifrada() {
        let temp = tempfile::tempdir().expect("temporal");
        let vault = AlmacenDePrueba::default();
        let cargada = load_or_create_with(temp.path(), &vault).expect("crea");

        let en_disco = std::fs::read(temp.path().join(KEY_FILE_NAME)).expect("lee");
        assert!(nexus_keyvault::is_sealed(&en_disco), "deberia llevar marca");
        assert_ne!(
            en_disco,
            cargada.identity.export_secret().as_slice(),
            "la clave no puede estar en el archivo tal cual"
        );
        assert!(cargada.key.protected);
        assert!(!cargada.key.migrated);
    }

    #[test]
    fn una_clave_antigua_se_migra_sin_cambiar_la_identidad() {
        // La prueba que importa: migrar no puede cambiar quien es este equipo,
        // porque eso invalidaria todos los dispositivos emparejados.
        let temp = tempfile::tempdir().expect("temporal");
        let antigua = escribir_clave_antigua(temp.path());
        let vault = AlmacenDePrueba::default();

        let cargada = load_or_create_with(temp.path(), &vault).expect("carga y migra");

        assert_eq!(
            cargada.identity.public_key(),
            antigua.public_key(),
            "la identidad tiene que ser exactamente la misma"
        );
        assert_eq!(cargada.identity.device_id(), antigua.device_id());
        assert!(cargada.key.migrated, "deberia haberse migrado");

        let en_disco = std::fs::read(temp.path().join(KEY_FILE_NAME)).expect("lee");
        assert!(
            nexus_keyvault::is_sealed(&en_disco),
            "ya deberia estar cifrada"
        );
    }

    #[test]
    fn migrar_dos_veces_no_vuelve_a_migrar() {
        let temp = tempfile::tempdir().expect("temporal");
        escribir_clave_antigua(temp.path());
        let vault = AlmacenDePrueba::default();

        let primera = load_or_create_with(temp.path(), &vault).expect("migra");
        assert!(primera.key.migrated);

        let segunda = load_or_create_with(temp.path(), &vault).expect("carga");
        assert!(!segunda.key.migrated, "ya estaba cifrada");
        assert_eq!(segunda.identity.public_key(), primera.identity.public_key());
    }

    #[test]
    fn si_el_cifrado_falla_el_host_sigue_arrancando_con_la_clave_de_antes() {
        // Quedarse sin host porque el cifrado no pudo hacerse seria peor que
        // seguir con la clave que ya funcionaba. Se avisa y se sigue.
        let temp = tempfile::tempdir().expect("temporal");
        let antigua = escribir_clave_antigua(temp.path());
        let vault = AlmacenDePrueba {
            falla_al_cifrar: true,
        };

        let cargada = load_or_create_with(temp.path(), &vault).expect("deberia arrancar igual");
        assert_eq!(cargada.identity.public_key(), antigua.public_key());
        assert!(!cargada.key.migrated);

        let en_disco = std::fs::read(temp.path().join(KEY_FILE_NAME)).expect("lee");
        assert_eq!(
            en_disco,
            antigua.export_secret().as_slice(),
            "la clave que ya funcionaba tiene que quedar intacta"
        );
        let restos: Vec<_> = std::fs::read_dir(temp.path())
            .expect("lee")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("nuevo"))
            .collect();
        assert!(
            restos.is_empty(),
            "no deberian quedar temporales: {restos:?}"
        );
    }

    #[test]
    fn una_clave_de_otra_cuenta_no_genera_una_identidad_nueva() {
        // En Windows pasa al copiar el archivo de otro equipo: DPAPI no puede
        // descifrarlo. Generar otra identidad en silencio dejaria a la persona
        // sin sus dispositivos emparejados y sin saber por que.
        let temp = tempfile::tempdir().expect("temporal");
        let mut ajena = nexus_keyvault::SEALED_MAGIC.to_vec();
        ajena.extend_from_slice(&[9_u8; 48]);
        std::fs::write(temp.path().join(KEY_FILE_NAME), &ajena).expect("escribe");
        std::fs::write(temp.path().join(ID_FILE_NAME), DeviceId::new().to_string())
            .expect("escribe");

        let error = load_or_create_with(temp.path(), &NoDescifra).unwrap_err();
        let mensaje = error.to_string();
        assert!(
            mensaje.contains("otro equipo") || mensaje.contains("otra cuenta"),
            "el mensaje tiene que explicar que pasa: {mensaje}"
        );
        assert_eq!(
            std::fs::read(temp.path().join(KEY_FILE_NAME)).expect("lee"),
            ajena,
            "no debe tocarse el archivo"
        );
    }

    #[test]
    fn el_estado_dice_la_verdad_sobre_si_esta_protegida() {
        let temp = tempfile::tempdir().expect("temporal");
        let cargada = load_or_create_with(temp.path(), &nexus_keyvault::PlainVault).expect("crea");
        assert!(!cargada.key.protected);
        assert_eq!(cargada.key.vault, "sin-cifrar");
        assert!(
            cargada.key.detail.contains("sin cifrar"),
            "{}",
            cargada.key.detail
        );
    }

    #[test]
    fn the_identity_is_created_on_first_run() {
        let temp = tempfile::tempdir().expect("temporal");
        let identity = load_or_create(temp.path()).expect("crea identidad");

        assert!(temp.path().join(KEY_FILE_NAME).exists());
        assert!(temp.path().join(ID_FILE_NAME).exists());
        assert_eq!(identity.public_key().fingerprint().len(), 64);
    }

    #[test]
    fn the_same_identity_is_reused_on_later_runs() {
        let temp = tempfile::tempdir().expect("temporal");
        let first = load_or_create(temp.path()).expect("crea");
        let second = load_or_create(temp.path()).expect("reutiliza");

        assert_eq!(
            first.public_key(),
            second.public_key(),
            "cambiar de clave invalidaria todos los emparejamientos"
        );
        assert_eq!(first.device_id(), second.device_id());
    }

    #[test]
    fn the_reloaded_identity_can_still_sign() {
        let temp = tempfile::tempdir().expect("temporal");
        let first = load_or_create(temp.path()).expect("crea");
        let message = b"transcripcion";
        let signature = first.sign(message);

        let second = load_or_create(temp.path()).expect("reutiliza");
        second
            .public_key()
            .verify(message, &signature)
            .expect("la clave es la misma");
    }

    #[test]
    fn a_missing_directory_is_created() {
        let temp = tempfile::tempdir().expect("temporal");
        let nested = temp.path().join("datos").join("nexus");
        load_or_create(&nested).expect("crea el arbol de directorios");
        assert!(nested.join(KEY_FILE_NAME).exists());
    }

    #[test]
    fn a_corrupt_key_file_fails_instead_of_regenerating() {
        let temp = tempfile::tempdir().expect("temporal");
        load_or_create(temp.path()).expect("crea");

        std::fs::write(temp.path().join(KEY_FILE_NAME), b"esto no es una clave").expect("corrompe");

        let err = load_or_create(temp.path()).unwrap_err();
        assert!(
            matches!(err, HostError::InvalidInput(_)),
            "debe fallar, no generar otra identidad: {err:?}"
        );
    }

    #[test]
    fn a_corrupt_id_file_fails_too() {
        let temp = tempfile::tempdir().expect("temporal");
        load_or_create(temp.path()).expect("crea");
        std::fs::write(temp.path().join(ID_FILE_NAME), b"no-es-un-uuid").expect("corrompe");
        assert!(load_or_create(temp.path()).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn the_key_file_is_not_readable_by_others() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temporal");
        load_or_create(temp.path()).expect("crea");

        let mode = std::fs::metadata(temp.path().join(KEY_FILE_NAME))
            .expect("metadatos")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "permisos reales: {:o}", mode & 0o777);
    }
}
