//! Restriccion de acceso al sistema de archivos del host.
//!
//! El explorador remoto **no** da acceso a todo el disco. La persona declara
//! una o varias carpetas compartidas y cualquier ruta que llegue por la red se
//! resuelve contra esa lista. Lo que no cae dentro, se rechaza.
//!
//! ## Que se defiende aqui
//!
//! 1. **Recorrido de directorios.** Un `..` en la ruta se rechaza de entrada,
//!    sin intentar normalizarlo. Normalizar invita a errores sutiles; rechazar
//!    es inequivoco.
//! 2. **Enlaces simbolicos que escapan.** No basta comprobar el texto de la
//!    ruta: un enlace dentro de la carpeta compartida puede apuntar fuera. Se
//!    resuelve la ruta real con [`std::fs::canonicalize`] y se vuelve a
//!    comprobar que siga dentro.
//! 3. **Rutas que aun no existen.** Al subir un archivo el destino no existe
//!    todavia, asi que se canonicaliza el ancestro mas profundo que si existe y
//!    se comprueba ese. Sin este paso, las subidas se escaparian del control.
//! 4. **Nombres reservados de Windows y flujos alternativos de datos.**
//!    `CON`, `NUL`, `COM1`, `archivo.txt:oculto`... Se rechazan **en todas las
//!    plataformas**, no solo en Windows, para que lo que se prueba en Linux sea
//!    exactamente lo que se aplica en Windows.
//! 5. **Carpetas de solo lectura.** Una raiz puede permitir lectura sin
//!    permitir escritura.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{FileError, FileResult};

/// Tipo de acceso que una operacion necesita.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Listar y descargar.
    Read,
    /// Crear y sobrescribir.
    Write,
}

/// Nombres de dispositivo reservados por Windows.
///
/// Abrir uno de estos no toca el disco: habla con un dispositivo del sistema.
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Indica si el nombre coincide con un dispositivo reservado de Windows.
///
/// La comparacion ignora mayusculas y la extension, porque `con.txt` tambien
/// resuelve al dispositivo en Windows.
#[must_use]
pub fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    WINDOWS_RESERVED
        .iter()
        .any(|r| r.eq_ignore_ascii_case(stem))
}

/// Indica si el nombre intenta usar un flujo alternativo de datos de NTFS.
///
/// `archivo.txt:oculto:$DATA` guarda contenido invisible para quien mire el
/// archivo. No hay razon legitima para pedirlo desde el explorador remoto.
#[must_use]
pub fn has_alternate_data_stream(name: &str) -> bool {
    name.contains(':')
}

/// Indica si el nombre termina en punto o espacio.
///
/// Windows los recorta al abrir el archivo, de modo que `secreto.txt ` y
/// `secreto.txt` acaban siendo el mismo archivo: una ruta asi puede sortear una
/// comprobacion hecha sobre el texto.
#[must_use]
pub fn has_trailing_dot_or_space(name: &str) -> bool {
    name.ends_with('.') || name.ends_with(' ')
}

/// Carpeta compartida con el acceso que concede.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedRoot {
    /// Nombre que ve la persona en el explorador remoto.
    pub label: String,
    /// Ruta en el host.
    pub path: PathBuf,
    /// Si se permite escribir dentro.
    pub writable: bool,
}

impl SharedRoot {
    /// Declara una carpeta de solo lectura.
    #[must_use]
    pub fn read_only(label: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
            writable: false,
        }
    }

    /// Declara una carpeta con lectura y escritura.
    #[must_use]
    pub fn writable(label: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
            writable: true,
        }
    }
}

/// Politica de acceso del host: la lista de carpetas compartidas.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessPolicy {
    roots: Vec<SharedRoot>,
}

impl AccessPolicy {
    /// Politica sin ninguna carpeta compartida: no se puede acceder a nada.
    ///
    /// Es el valor por omision a proposito: el acceso a archivos se concede
    /// explicitamente, nunca por descuido.
    #[must_use]
    pub const fn empty() -> Self {
        Self { roots: Vec::new() }
    }

    /// Construye la politica a partir de una lista de carpetas.
    #[must_use]
    pub fn new(roots: Vec<SharedRoot>) -> Self {
        Self { roots }
    }

    /// Anade una carpeta compartida.
    pub fn add_root(&mut self, root: SharedRoot) {
        self.roots.push(root);
    }

    /// Carpetas compartidas declaradas.
    #[must_use]
    pub fn roots(&self) -> &[SharedRoot] {
        &self.roots
    }

    /// Indica si no hay ninguna carpeta compartida.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Resuelve una ruta recibida por la red y comprueba que el acceso vale.
    ///
    /// Devuelve la ruta real (ya canonicalizada en su parte existente) lista
    /// para usar. Toda operacion de archivos del host debe pasar por aqui.
    ///
    /// # Errors
    ///
    /// - [`FileError::PathTraversal`] si la ruta lleva `..`.
    /// - [`FileError::ForbiddenName`] si algun componente tiene un nombre
    ///   reservado, un flujo alternativo de datos, o termina en punto o espacio.
    /// - [`FileError::OutsideRoots`] si la ruta real no cae en ninguna carpeta
    ///   compartida, incluido el caso de un enlace simbolico que escapa.
    /// - [`FileError::ReadOnlyRoot`] si se pide escritura en una carpeta de
    ///   solo lectura.
    pub fn resolve(&self, requested: &Path, access: Access) -> FileResult<PathBuf> {
        reject_dangerous_components(requested)?;

        // La parte existente se canonicaliza: asi un enlace simbolico que
        // apunte fuera queda al descubierto antes de comprobar las raices.
        let (existing, remainder) = split_at_existing_ancestor(requested);
        let canonical_existing =
            std::fs::canonicalize(&existing).map_err(|e| map_io(&e, &existing))?;
        let resolved = canonical_existing.join(&remainder);

        let matched = self
            .roots
            .iter()
            .find(|candidate| {
                std::fs::canonicalize(&candidate.path)
                    .is_ok_and(|real_root| resolved.starts_with(&real_root))
            })
            .ok_or(FileError::OutsideRoots)?;

        if access == Access::Write && !matched.writable {
            return Err(FileError::ReadOnlyRoot);
        }

        Ok(resolved)
    }
}

/// Rechaza los componentes de ruta que no deben llegar nunca al disco.
fn reject_dangerous_components(path: &Path) -> FileResult<()> {
    if path.as_os_str().is_empty() {
        return Err(FileError::OutsideRoots);
    }

    for component in path.components() {
        match component {
            Component::ParentDir => return Err(FileError::PathTraversal),
            Component::Normal(raw) => {
                let name = raw.to_string_lossy();
                if has_alternate_data_stream(&name) {
                    return Err(FileError::ForbiddenName {
                        reason: "flujo alternativo de datos de NTFS",
                    });
                }
                if is_windows_reserved_name(&name) {
                    return Err(FileError::ForbiddenName {
                        reason: "nombre de dispositivo reservado de Windows",
                    });
                }
                if has_trailing_dot_or_space(&name) {
                    return Err(FileError::ForbiddenName {
                        reason: "el nombre termina en punto o espacio",
                    });
                }
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Ok(())
}

/// Parte la ruta en el ancestro mas profundo que existe y el resto.
///
/// Hace falta porque al subir un archivo el destino todavia no existe y
/// `canonicalize` fallaria sobre la ruta completa.
fn split_at_existing_ancestor(path: &Path) -> (PathBuf, PathBuf) {
    let mut existing = path.to_path_buf();
    let mut remainder = Vec::new();

    while !existing.exists() {
        match (
            existing.file_name().map(std::ffi::OsString::from),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) if !parent.as_os_str().is_empty() => {
                remainder.push(name);
                existing = parent.to_path_buf();
            }
            _ => break,
        }
    }

    remainder.reverse();
    (existing, remainder.into_iter().collect())
}

fn map_io(error: &std::io::Error, path: &Path) -> FileError {
    if error.kind() == std::io::ErrorKind::NotFound {
        FileError::NotFound
    } else {
        FileError::Io(format!("{}: {error}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Monta dos carpetas compartidas y una tercera que queda fuera.
    struct Fixture {
        _temp: tempfile::TempDir,
        read_root: PathBuf,
        write_root: PathBuf,
        outside: PathBuf,
        policy: AccessPolicy,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().expect("carpeta temporal");
        let base = temp.path();

        let read_root = base.join("documentos");
        let write_root = base.join("intercambio");
        let outside = base.join("privado");
        for dir in [&read_root, &write_root, &outside] {
            fs::create_dir_all(dir).expect("crea carpeta");
        }
        fs::write(read_root.join("informe.txt"), b"contenido").expect("crea archivo");
        fs::write(outside.join("claves.txt"), b"secreto").expect("crea archivo");

        let policy = AccessPolicy::new(vec![
            SharedRoot::read_only("Documentos", &read_root),
            SharedRoot::writable("Intercambio", &write_root),
        ]);

        Fixture {
            _temp: temp,
            read_root,
            write_root,
            outside,
            policy,
        }
    }

    #[test]
    fn the_default_policy_shares_nothing() {
        let policy = AccessPolicy::empty();
        assert!(policy.is_empty());
        assert_eq!(
            policy.resolve(Path::new("/tmp"), Access::Read).unwrap_err(),
            FileError::OutsideRoots
        );
    }

    #[test]
    fn a_file_inside_a_shared_root_is_allowed() {
        let f = fixture();
        let resolved = f
            .policy
            .resolve(&f.read_root.join("informe.txt"), Access::Read)
            .expect("deberia permitirse");
        assert!(resolved.ends_with("informe.txt"));
    }

    #[test]
    fn a_file_outside_every_root_is_denied() {
        let f = fixture();
        assert_eq!(
            f.policy
                .resolve(&f.outside.join("claves.txt"), Access::Read)
                .unwrap_err(),
            FileError::OutsideRoots
        );
    }

    #[test]
    fn directory_traversal_is_rejected_outright() {
        let f = fixture();
        let attack = f.read_root.join("..").join("privado").join("claves.txt");
        assert_eq!(
            f.policy.resolve(&attack, Access::Read).unwrap_err(),
            FileError::PathTraversal
        );
    }

    #[test]
    fn deep_traversal_is_also_rejected() {
        let f = fixture();
        let attack = f.read_root.join("../../../../etc/passwd");
        assert_eq!(
            f.policy.resolve(&attack, Access::Read).unwrap_err(),
            FileError::PathTraversal
        );
    }

    #[test]
    fn traversal_that_would_cancel_out_is_still_rejected() {
        // `documentos/sub/../informe.txt` acabaria dentro de la carpeta, pero
        // se rechaza igual: normalizar rutas con `..` es fuente de errores.
        let f = fixture();
        fs::create_dir_all(f.read_root.join("sub")).expect("crea");
        let path = f.read_root.join("sub").join("..").join("informe.txt");
        assert_eq!(
            f.policy.resolve(&path, Access::Read).unwrap_err(),
            FileError::PathTraversal
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_escaping_the_root_is_denied() {
        let f = fixture();
        let link = f.read_root.join("atajo");
        std::os::unix::fs::symlink(&f.outside, &link).expect("crea enlace");

        // El texto de la ruta no tiene nada sospechoso...
        let attack = link.join("claves.txt");
        assert!(!attack.to_string_lossy().contains(".."));

        // ...pero la ruta real sale de la carpeta compartida.
        assert_eq!(
            f.policy.resolve(&attack, Access::Read).unwrap_err(),
            FileError::OutsideRoots,
            "el enlace simbolico no debe permitir escapar"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_staying_inside_the_root_is_allowed() {
        let f = fixture();
        fs::create_dir_all(f.read_root.join("real")).expect("crea");
        fs::write(f.read_root.join("real/dato.txt"), b"x").expect("crea");
        std::os::unix::fs::symlink(f.read_root.join("real"), f.read_root.join("alias"))
            .expect("crea enlace");

        f.policy
            .resolve(&f.read_root.join("alias/dato.txt"), Access::Read)
            .expect("un enlace interno si vale");
    }

    #[test]
    fn writing_to_a_read_only_root_is_denied() {
        let f = fixture();
        assert_eq!(
            f.policy
                .resolve(&f.read_root.join("informe.txt"), Access::Write)
                .unwrap_err(),
            FileError::ReadOnlyRoot
        );
    }

    #[test]
    fn writing_to_a_writable_root_is_allowed() {
        let f = fixture();
        f.policy
            .resolve(&f.write_root.join("nuevo.bin"), Access::Write)
            .expect("la carpeta de intercambio admite escritura");
    }

    #[test]
    fn a_new_file_in_a_writable_root_resolves_even_though_it_does_not_exist() {
        let f = fixture();
        let target = f.write_root.join("sub").join("aun-no-existe.txt");
        let resolved = f.policy.resolve(&target, Access::Write).expect("resuelve");
        assert!(resolved.ends_with("sub/aun-no-existe.txt"));
    }

    #[test]
    #[cfg(unix)]
    fn a_new_file_behind_an_escaping_symlink_is_denied() {
        // El caso peligroso de las subidas: el destino no existe, pero su
        // carpeta padre es un enlace que apunta fuera.
        let f = fixture();
        std::os::unix::fs::symlink(&f.outside, f.write_root.join("fuga")).expect("enlace");
        assert_eq!(
            f.policy
                .resolve(&f.write_root.join("fuga/nuevo.txt"), Access::Write)
                .unwrap_err(),
            FileError::OutsideRoots
        );
    }

    #[test]
    fn windows_device_names_are_rejected_on_every_platform() {
        let f = fixture();
        for name in ["CON", "con", "NUL", "nul.txt", "COM1", "lpt9.dat", "AUX"] {
            let err = f
                .policy
                .resolve(&f.write_root.join(name), Access::Write)
                .unwrap_err();
            assert_eq!(
                err,
                FileError::ForbiddenName {
                    reason: "nombre de dispositivo reservado de Windows"
                },
                "{name} deberia rechazarse"
            );
        }
    }

    #[test]
    fn ordinary_names_that_merely_start_like_a_device_are_allowed() {
        let f = fixture();
        for name in ["contrato.pdf", "console.log", "auxiliar.txt", "com10.txt"] {
            f.policy
                .resolve(&f.write_root.join(name), Access::Write)
                .unwrap_or_else(|e| panic!("{name} deberia permitirse: {e}"));
        }
    }

    #[test]
    fn alternate_data_streams_are_rejected() {
        let f = fixture();
        let err = f
            .policy
            .resolve(&f.write_root.join("informe.txt:oculto"), Access::Write)
            .unwrap_err();
        assert_eq!(
            err,
            FileError::ForbiddenName {
                reason: "flujo alternativo de datos de NTFS"
            }
        );
    }

    #[test]
    fn trailing_dots_and_spaces_are_rejected() {
        let f = fixture();
        for name in ["informe.txt.", "informe.txt "] {
            let err = f
                .policy
                .resolve(&f.write_root.join(name), Access::Write)
                .unwrap_err();
            assert_eq!(
                err,
                FileError::ForbiddenName {
                    reason: "el nombre termina en punto o espacio"
                },
                "{name:?} deberia rechazarse"
            );
        }
    }

    #[test]
    fn an_empty_path_is_rejected() {
        let f = fixture();
        assert_eq!(
            f.policy.resolve(Path::new(""), Access::Read).unwrap_err(),
            FileError::OutsideRoots
        );
    }

    #[test]
    fn a_missing_file_in_a_shared_root_reports_not_found_only_for_reads() {
        let f = fixture();
        // Para lectura, la ruta inexistente se resuelve igual: es el llamante
        // quien al abrirla recibira el error del sistema. Lo importante es que
        // la comprobacion de raices se aplique antes.
        let resolved = f
            .policy
            .resolve(&f.read_root.join("no-existe.txt"), Access::Read);
        assert!(
            resolved.is_ok(),
            "la ruta esta dentro de la carpeta compartida"
        );
    }

    #[test]
    fn reserved_name_helpers_behave_as_documented() {
        assert!(is_windows_reserved_name("CON"));
        assert!(is_windows_reserved_name("nul.txt"));
        assert!(is_windows_reserved_name("COM9.log"));
        assert!(!is_windows_reserved_name("COM10"));
        assert!(!is_windows_reserved_name("contrato"));

        assert!(has_alternate_data_stream("a:b"));
        assert!(!has_alternate_data_stream("a-b"));

        assert!(has_trailing_dot_or_space("a."));
        assert!(has_trailing_dot_or_space("a "));
        assert!(!has_trailing_dot_or_space("a"));
    }

    #[test]
    fn roots_can_be_added_after_construction() {
        let mut policy = AccessPolicy::empty();
        assert!(policy.is_empty());
        policy.add_root(SharedRoot::read_only("Temporal", "/tmp"));
        assert_eq!(policy.roots().len(), 1);
        assert!(!policy.roots()[0].writable);
    }
}
