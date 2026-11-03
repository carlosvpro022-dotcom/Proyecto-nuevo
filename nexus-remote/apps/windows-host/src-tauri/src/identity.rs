//! Persistencia de la identidad criptografica del host.
//!
//! El host genera su par de claves Ed25519 la primera vez que arranca y lo
//! reutiliza siempre: su clave publica es lo que los dispositivos verifican
//! para saber que estan hablando con el PC correcto y no con un intermediario.
//!
//! # Proteccion de la clave privada: pendiente
//!
//! Ahora mismo la clave se guarda en un archivo con los permisos mas
//! restrictivos que permite el sistema (`0600` en Unix). **Eso no es
//! suficiente para produccion en Windows**, donde los permisos de archivo no
//! protegen frente a otro proceso del mismo usuario.
//!
//! Antes de considerar el host listo para produccion hay que cifrar este
//! archivo con DPAPI (`CryptProtectData` con ambito de usuario), que ata el
//! descifrado a la cuenta de Windows. Esta recogido como tarea de la etapa 6 en
//! `docs/architecture/ADR-0004-identidad-criptografica.md`.
//!
//! El modulo esta escrito para que ese cambio afecte solo a [`load_or_create`]:
//! el resto del host no sabe de donde sale la clave.

use std::path::{Path, PathBuf};

use nexus_core::DeviceId;
use nexus_security::DeviceIdentity;
use nexus_security::identity::SECRET_KEY_LEN;

use crate::error::{HostError, HostResult};

/// Nombre del archivo que guarda la clave privada.
pub const KEY_FILE_NAME: &str = "host-identity.key";

/// Nombre del archivo que guarda el identificador del dispositivo.
pub const ID_FILE_NAME: &str = "host-identity.id";

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
    std::fs::create_dir_all(data_dir).map_err(|source| HostError::Io {
        path: data_dir.display().to_string(),
        source,
    })?;

    let key_path = data_dir.join(KEY_FILE_NAME);
    let id_path = data_dir.join(ID_FILE_NAME);

    if key_path.exists() {
        return load(&key_path, &id_path);
    }

    let identity = DeviceIdentity::generate(DeviceId::new());
    write_secret(&key_path, identity.export_secret().as_slice())?;
    write_text(&id_path, &identity.device_id().to_string())?;

    tracing::info!(
        device_id = %identity.device_id(),
        fingerprint = %identity.public_key().short_fingerprint(),
        "identidad del host generada"
    );
    Ok(identity)
}

fn load(key_path: &Path, id_path: &Path) -> HostResult<DeviceIdentity> {
    let secret = std::fs::read(key_path).map_err(|source| HostError::Io {
        path: key_path.display().to_string(),
        source,
    })?;

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

    Ok(DeviceIdentity::from_secret_bytes(device_id, &secret)?)
}

/// Escribe el material secreto con los permisos mas restrictivos posibles.
fn write_secret(path: &PathBuf, bytes: &[u8]) -> HostResult<()> {
    write_bytes(path, bytes)?;
    restrict_permissions(path)?;
    Ok(())
}

fn write_text(path: &PathBuf, text: &str) -> HostResult<()> {
    write_bytes(path, text.as_bytes())
}

fn write_bytes(path: &PathBuf, bytes: &[u8]) -> HostResult<()> {
    std::fs::write(path, bytes).map_err(|source| HostError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(unix)]
fn restrict_permissions(path: &PathBuf) -> HostResult<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|source| {
        HostError::Io {
            path: path.display().to_string(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &PathBuf) -> HostResult<()> {
    // En Windows los permisos de archivo no aislan de otro proceso del mismo
    // usuario. La proteccion real es DPAPI y esta pendiente (etapa 6).
    tracing::warn!(
        "la clave privada del host se guarda sin cifrar: falta el cifrado con DPAPI (etapa 6)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
