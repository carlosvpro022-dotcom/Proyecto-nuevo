//! Listado de un directorio del host.
//!
//! ## El limite que decide el diseno
//!
//! El listado viaja por el canal de control, que tiene un tope de 64 KiB por
//! mensaje. Una carpeta de descargas con tres mil archivos no cabe. Hay dos
//! salidas: truncar el listado, o paginarlo.
//!
//! Truncar significa que la persona ve doscientos archivos de tres mil y no
//! tiene forma de llegar al resto. Es un limite que se notaria el primer dia.
//! Asi que se pagina: cada peticion trae una ventana y el total, y quien
//! pregunta sabe si falta mas y puede pedirlo.
//!
//! Paginar obliga a un orden estable entre paginas, y un orden estable obliga a
//! leer la carpeta entera y ordenarla en cada peticion. Es lo que cuesta
//! hacerlo bien: `read_dir` no garantiza ningun orden, asi que sin ordenar
//! completo un archivo podria salir en dos paginas, o en ninguna.
//!
//! El recorrido completo tambien se acota ([`MAX_SCANNED_ENTRIES`]): un
//! directorio con millones de entradas no debe poder agotar la memoria del
//! host. Lo que pase de ahi se dice, no se esconde.

use std::path::Path;

use nexus_core::Timestamp;
use nexus_core::protocol::control::DirEntry;

use crate::error::{FileError, FileResult};

/// Entradas que se devuelven como maximo en una pagina.
///
/// Sale del tope de 64 KiB del canal de control: con nombres normales, una
/// pagina de este tamano ronda los 20 KiB y deja margen de sobra.
pub const MAX_PAGE_ENTRIES: usize = 200;

/// Entradas que se recorren como maximo en una carpeta.
///
/// Un directorio mas grande que esto se lista parcialmente antes que agotar la
/// memoria. [`Listing::scan_truncated`] lo avisa.
pub const MAX_SCANNED_ENTRIES: usize = 50_000;

/// Presupuesto de bytes para las entradas de una pagina.
///
/// Se queda holgadamente por debajo del tope del canal de control para dejar
/// sitio al sobre, a la ruta y al resto de la respuesta.
pub const MAX_PAGE_BYTES: usize = 40 * 1024;

/// Resultado de listar un directorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// Entradas de esta pagina, ya ordenadas.
    pub entries: Vec<DirEntry>,
    /// Posicion de la primera entrada dentro del orden completo.
    pub offset: u64,
    /// Entradas que tiene la carpeta en total.
    pub total_entries: u64,
    /// Si la carpeta tiene mas entradas de las que se pueden recorrer.
    pub scan_truncated: bool,
}

impl Listing {
    /// Indica si quedan entradas despues de esta pagina.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.offset.saturating_add(self.entries.len() as u64) < self.total_entries
    }
}

/// Lista una pagina de un directorio ya resuelto contra la politica de acceso.
///
/// La ruta tiene que venir de
/// [`AccessPolicy::resolve`](crate::access::AccessPolicy::resolve): esta
/// funcion no decide si se puede mirar, solo mira.
///
/// El orden es: carpetas antes que archivos, y dentro de cada grupo por nombre
/// ignorando mayusculas, con el nombre exacto como desempate para que no
/// dependa del orden en que el sistema devuelva las entradas.
///
/// # Errors
///
/// - [`FileError::NotFound`] si la ruta no existe.
/// - [`FileError::WrongKind`] si no es un directorio.
/// - [`FileError::Io`] si la carpeta no se puede leer.
pub fn list_dir(path: &Path, offset: u64) -> FileResult<Listing> {
    list_dir_where(path, offset, |_| true)
}

/// Lista una pagina quedandose solo con las entradas que `alcanzable` acepte.
///
/// Existe por una fuga pequena pero real. Un acceso directo dentro de una
/// carpeta compartida puede apuntar fuera de ella, y describirlo siguiendo el
/// enlace contaria el tamano y la fecha de **lo de fuera**. Descargarlo si se
/// rechaza, pero el dato ya habria salido.
///
/// El filtro se aplica al recorrer, antes de ordenar y contar, para que el
/// total y las paginas sigan cuadrando: filtrar despues devolveria paginas mas
/// cortas de lo que dice el total y quien pagina se saltaria entradas.
///
/// # Errors
///
/// Las mismas que [`list_dir`].
pub fn list_dir_where(
    path: &Path,
    offset: u64,
    alcanzable: impl Fn(&Path) -> bool,
) -> FileResult<Listing> {
    let metadata = std::fs::metadata(path).map_err(|e| map_io(&e, path))?;
    if !metadata.is_dir() {
        return Err(FileError::WrongKind {
            expected: "un directorio",
        });
    }

    let reader = std::fs::read_dir(path).map_err(|e| map_io(&e, path))?;
    let mut all = Vec::new();
    let mut scan_truncated = false;

    for item in reader {
        if all.len() >= MAX_SCANNED_ENTRIES {
            scan_truncated = true;
            break;
        }
        // Una entrada ilegible (permisos, enlace roto, se borro justo ahora) no
        // invalida el listado: se omite y se sigue. Un directorio con un
        // archivo problematico tiene que poder mirarse.
        let Ok(entry) = item else { continue };
        if !alcanzable(&entry.path()) {
            continue;
        }
        let Some(described) = describe(&entry) else {
            continue;
        };
        all.push(described);
    }

    all.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });

    let total_entries = all.len() as u64;
    let start = usize::try_from(offset).unwrap_or(usize::MAX).min(all.len());
    let page = take_page(all.split_off(start));

    Ok(Listing {
        entries: page,
        offset,
        total_entries,
        scan_truncated,
    })
}

/// Recorta la pagina al numero de entradas y al presupuesto de bytes.
///
/// El tope de bytes importa porque el de entradas no basta: doscientos nombres
/// de 255 caracteres si cabrian en la cuenta y no en el mensaje. La estimacion
/// va holgada a proposito (seis bytes por caracter del nombre, que es el peor
/// caso del escapado JSON) para que nunca se pase.
fn take_page(mut entries: Vec<DirEntry>) -> Vec<DirEntry> {
    entries.truncate(MAX_PAGE_ENTRIES);

    let mut budget = MAX_PAGE_BYTES;
    let mut kept = 0;
    for entry in &entries {
        let estimate = 110 + entry.name.len() * 6;
        if estimate > budget {
            break;
        }
        budget -= estimate;
        kept += 1;
    }
    // Al menos una entrada, aunque su nombre se coma el presupuesto: una
    // pagina vacia haria creer que la carpeta esta vacia.
    entries.truncate(kept.max(1).min(entries.len()));
    entries
}

/// Traduce una entrada del sistema a lo que viaja por el protocolo.
///
/// Devuelve `None` si no se puede describir, para omitirla sin romper nada.
fn describe(entry: &std::fs::DirEntry) -> Option<DirEntry> {
    let name = entry.file_name().to_string_lossy().into_owned();
    if name.is_empty() {
        return None;
    }

    // `metadata` de `DirEntry` no sigue enlaces en Unix; se pide la que si los
    // sigue para que una carpeta enlazada se vea como carpeta. Si el enlace
    // esta roto, se cae a la del propio enlace en lugar de omitir la entrada:
    // verla y no poder abrirla informa mas que no verla.
    let metadata = std::fs::metadata(entry.path())
        .ok()
        .or_else(|| entry.metadata().ok())?;

    let is_dir = metadata.is_dir();
    Some(DirEntry {
        name: name.clone(),
        is_dir,
        size_bytes: if is_dir { 0 } else { metadata.len() },
        modified_at: modified_at(&metadata),
        hidden: is_hidden(&name, &metadata),
    })
}

fn modified_at(metadata: &std::fs::Metadata) -> Option<Timestamp> {
    let modified = metadata.modified().ok()?;
    let since = modified
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .ok()?;
    Some(Timestamp(since.as_millis().min(u128::from(u64::MAX)) as u64))
}

/// Indica si la entrada se considera oculta.
///
/// El nombre con punto delante se respeta en todas las plataformas, porque es
/// la convencion de Unix y de muchas herramientas. En Windows cuenta ademas el
/// atributo del archivo, que es lo que mira el explorador.
fn is_hidden(name: &str, metadata: &std::fs::Metadata) -> bool {
    name.starts_with('.') || has_hidden_attribute(metadata)
}

/// Lee el atributo de oculto de Windows.
#[cfg(windows)]
fn has_hidden_attribute(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    /// `FILE_ATTRIBUTE_HIDDEN` de la API de Windows.
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;

    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

/// Fuera de Windows no hay tal atributo.
#[cfg(not(windows))]
const fn has_hidden_attribute(_metadata: &std::fs::Metadata) -> bool {
    false
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
    use std::path::PathBuf;

    struct Dir {
        _temp: tempfile::TempDir,
        path: PathBuf,
    }

    fn dir_with(names: &[&str]) -> Dir {
        let temp = tempfile::tempdir().expect("temporal");
        let path = temp.path().to_path_buf();
        for name in names {
            if let Some(folder) = name.strip_suffix('/') {
                fs::create_dir_all(path.join(folder)).expect("crea carpeta");
            } else {
                fs::write(path.join(name), b"contenido").expect("crea archivo");
            }
        }
        Dir { _temp: temp, path }
    }

    fn names(listing: &Listing) -> Vec<String> {
        listing.entries.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn an_empty_directory_lists_nothing() {
        let d = dir_with(&[]);
        let listing = list_dir(&d.path, 0).expect("lista");
        assert!(listing.entries.is_empty());
        assert_eq!(listing.total_entries, 0);
        assert!(!listing.has_more());
        assert!(!listing.scan_truncated);
    }

    #[test]
    fn folders_come_before_files() {
        let d = dir_with(&["zeta.txt", "alfa/", "beta.txt", "omega/"]);
        let listing = list_dir(&d.path, 0).expect("lista");
        assert_eq!(names(&listing), ["alfa", "omega", "beta.txt", "zeta.txt"]);
    }

    #[test]
    fn the_order_ignores_case_but_stays_deterministic() {
        let d = dir_with(&["Banana.txt", "ananas.txt", "Cereza.txt"]);
        let listing = list_dir(&d.path, 0).expect("lista");
        assert_eq!(names(&listing), ["ananas.txt", "Banana.txt", "Cereza.txt"]);
        // Repetirlo da lo mismo: el orden no depende del sistema de archivos.
        assert_eq!(list_dir(&d.path, 0).expect("lista"), listing);
    }

    #[test]
    fn sizes_and_kinds_are_reported() {
        let d = dir_with(&["carpeta/", "dato.bin"]);
        let listing = list_dir(&d.path, 0).expect("lista");

        let carpeta = &listing.entries[0];
        assert!(carpeta.is_dir);
        assert_eq!(carpeta.size_bytes, 0, "una carpeta no declara tamano");

        let dato = &listing.entries[1];
        assert!(!dato.is_dir);
        assert_eq!(dato.size_bytes, b"contenido".len() as u64);
        assert!(dato.modified_at.is_some(), "deberia traer fecha");
    }

    #[test]
    fn dot_prefixed_names_are_marked_hidden() {
        let d = dir_with(&[".oculto", "visible.txt"]);
        let listing = list_dir(&d.path, 0).expect("lista");
        let oculto = listing
            .entries
            .iter()
            .find(|e| e.name == ".oculto")
            .expect("existe");
        let visible = listing
            .entries
            .iter()
            .find(|e| e.name == "visible.txt")
            .expect("existe");
        assert!(oculto.hidden);
        assert!(!visible.hidden);
    }

    #[test]
    fn a_file_is_not_a_directory() {
        let d = dir_with(&["dato.bin"]);
        assert_eq!(
            list_dir(&d.path.join("dato.bin"), 0).unwrap_err(),
            FileError::WrongKind {
                expected: "un directorio"
            }
        );
    }

    #[test]
    fn a_missing_directory_is_reported_as_not_found() {
        let d = dir_with(&[]);
        assert_eq!(
            list_dir(&d.path.join("no-existe"), 0).unwrap_err(),
            FileError::NotFound
        );
    }

    // --- Paginacion ------------------------------------------------------

    #[test]
    fn a_large_directory_is_paged_and_every_entry_appears_exactly_once() {
        let listed: Vec<String> = (0..MAX_PAGE_ENTRIES * 2 + 37)
            .map(|i| format!("archivo-{i:05}.txt"))
            .collect();
        let refs: Vec<&str> = listed.iter().map(String::as_str).collect();
        let d = dir_with(&refs);

        let mut seen = Vec::new();
        let mut offset = 0;
        loop {
            let listing = list_dir(&d.path, offset).expect("lista");
            assert_eq!(listing.total_entries, listed.len() as u64);
            assert!(listing.entries.len() <= MAX_PAGE_ENTRIES);
            if listing.entries.is_empty() {
                break;
            }
            offset += listing.entries.len() as u64;
            seen.extend(names(&listing));
            if !listing.has_more() {
                break;
            }
        }

        assert_eq!(seen.len(), listed.len(), "ni falta ni sobra nada");
        let mut unique = seen.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), seen.len(), "ninguna entrada se repite");

        let mut expected = listed.clone();
        expected.sort();
        assert_eq!(seen, expected);
    }

    #[test]
    fn the_first_page_reports_that_more_remain() {
        let listed: Vec<String> = (0..MAX_PAGE_ENTRIES + 5)
            .map(|i| format!("a-{i:04}.txt"))
            .collect();
        let refs: Vec<&str> = listed.iter().map(String::as_str).collect();
        let d = dir_with(&refs);

        let first = list_dir(&d.path, 0).expect("lista");
        assert_eq!(first.entries.len(), MAX_PAGE_ENTRIES);
        assert!(first.has_more());

        let last = list_dir(&d.path, MAX_PAGE_ENTRIES as u64).expect("lista");
        assert_eq!(last.entries.len(), 5);
        assert!(!last.has_more());
        assert_eq!(last.offset, MAX_PAGE_ENTRIES as u64);
    }

    #[test]
    fn an_offset_past_the_end_gives_an_empty_page_not_an_error() {
        let d = dir_with(&["uno.txt"]);
        let listing = list_dir(&d.path, 9_999).expect("lista");
        assert!(listing.entries.is_empty());
        assert_eq!(listing.total_entries, 1);
        assert!(!listing.has_more());
    }

    #[test]
    fn an_absurd_offset_does_not_panic() {
        let d = dir_with(&["uno.txt"]);
        let listing = list_dir(&d.path, u64::MAX).expect("lista");
        assert!(listing.entries.is_empty());
    }

    #[test]
    fn long_names_are_capped_by_the_byte_budget_not_the_entry_count() {
        // Nombres largos: la cuenta de entradas permitiria 200, el presupuesto
        // de bytes tiene que cortar antes.
        let listed: Vec<String> = (0..MAX_PAGE_ENTRIES)
            .map(|i| format!("{}-{i:03}.txt", "n".repeat(200)))
            .collect();
        let refs: Vec<&str> = listed.iter().map(String::as_str).collect();
        let d = dir_with(&refs);

        let listing = list_dir(&d.path, 0).expect("lista");
        assert!(
            listing.entries.len() < MAX_PAGE_ENTRIES,
            "el presupuesto de bytes deberia recortar la pagina, quedaron {}",
            listing.entries.len()
        );
        assert!(listing.has_more());
    }

    #[test]
    fn a_page_of_long_names_fits_in_the_control_message() {
        // La razon de ser del presupuesto: la respuesta tiene que caber.
        let listed: Vec<String> = (0..MAX_PAGE_ENTRIES)
            .map(|i| format!("{}-{i:03}.txt", "n".repeat(240)))
            .collect();
        let refs: Vec<&str> = listed.iter().map(String::as_str).collect();
        let d = dir_with(&refs);

        let listing = list_dir(&d.path, 0).expect("lista");
        let encoded = serde_json::to_string(&listing.entries).expect("serializa");
        assert!(
            encoded.len() <= MAX_PAGE_BYTES,
            "la pagina ocupa {} bytes, por encima del presupuesto",
            encoded.len()
        );
    }

    #[test]
    fn a_single_entry_is_returned_even_if_it_exceeds_the_budget() {
        let huge = "x".repeat(250);
        let d = dir_with(&[huge.as_str()]);
        let listing = list_dir(&d.path, 0).expect("lista");
        assert_eq!(listing.entries.len(), 1);
    }

    #[test]
    #[cfg(unix)]
    fn a_broken_symlink_is_listed_instead_of_breaking_the_listing() {
        let d = dir_with(&["real.txt"]);
        std::os::unix::fs::symlink(d.path.join("no-existe"), d.path.join("roto"))
            .expect("crea enlace");

        let listing = list_dir(&d.path, 0).expect("lista");
        assert_eq!(listing.total_entries, 2);
        assert!(names(&listing).contains(&"roto".to_owned()));
        assert!(names(&listing).contains(&"real.txt".to_owned()));
    }

    #[test]
    #[cfg(unix)]
    fn an_escaping_symlink_is_not_listed_at_all() {
        // La fuga que cierra el filtro: describir el enlace siguiendolo
        // contaria el tamano y la fecha de lo que hay fuera de la carpeta
        // compartida, aunque descargarlo se rechace despues.
        let d = dir_with(&["dentro.txt"]);
        let fuera = tempfile::tempdir().expect("temporal");
        fs::write(fuera.path().join("secreto.txt"), vec![0_u8; 4_096]).expect("crea");
        std::os::unix::fs::symlink(fuera.path(), d.path.join("atajo")).expect("crea enlace");

        // Sin filtro aparece, con su tamano de fuera.
        let todo = list_dir(&d.path, 0).expect("lista");
        assert!(names(&todo).contains(&"atajo".to_owned()));

        // Con el filtro de la politica de acceso, no.
        let raiz = d.path.clone();
        let listing = list_dir_where(&d.path, 0, |p| {
            std::fs::canonicalize(p).is_ok_and(|real| real.starts_with(&raiz))
        })
        .expect("lista");

        assert_eq!(names(&listing), ["dentro.txt"]);
        assert_eq!(
            listing.total_entries, 1,
            "el total tambien tiene que contar solo lo que se puede ver"
        );
    }

    #[test]
    #[cfg(unix)]
    fn the_filter_keeps_pages_and_totals_consistent() {
        // Filtrar despues de paginar daria paginas mas cortas que el total y
        // quien pagina se saltaria entradas. Por eso se filtra al recorrer.
        let listed: Vec<String> = (0..50).map(|i| format!("ok-{i:03}.txt")).collect();
        let refs: Vec<&str> = listed.iter().map(String::as_str).collect();
        let d = dir_with(&refs);
        for i in 0..50 {
            fs::write(d.path.join(format!("no-{i:03}.txt")), b"x").expect("crea");
        }

        let listing = list_dir_where(&d.path, 0, |p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("ok-"))
        })
        .expect("lista");

        assert_eq!(listing.total_entries, 50);
        assert_eq!(listing.entries.len(), 50);
        assert!(!listing.has_more());
        assert!(listing.entries.iter().all(|e| e.name.starts_with("ok-")));
    }

    #[test]
    #[cfg(unix)]
    fn a_symlinked_folder_is_listed_as_a_folder() {
        let d = dir_with(&["real/", "dato.txt"]);
        std::os::unix::fs::symlink(d.path.join("real"), d.path.join("alias")).expect("crea enlace");

        let listing = list_dir(&d.path, 0).expect("lista");
        let alias = listing
            .entries
            .iter()
            .find(|e| e.name == "alias")
            .expect("existe");
        assert!(alias.is_dir, "un enlace a carpeta se ve como carpeta");
    }
}
