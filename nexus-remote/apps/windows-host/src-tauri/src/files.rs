//! El servicio de archivos de una sesion establecida.
//!
//! Traduce las cuatro operaciones del protocolo (listar, descargar, subir y
//! cancelar) en trabajo real sobre el disco del host, y es el unico sitio donde
//! se decide que ruta se puede tocar.
//!
//! ## El orden de las comprobaciones
//!
//! Para cada operacion, en este orden y sin excepciones:
//!
//! 1. El permiso, que ya lo comprobo [`SessionHandler`](crate::session) antes
//!    de llegar aqui.
//! 2. La politica de acceso, que resuelve la ruta contra las carpetas
//!    compartidas y rechaza todo lo demas.
//! 3. Los limites de la sesion: cuantas transferencias a la vez y de que
//!    tamano.
//! 4. Y solo entonces, el disco.
//!
//! Un permiso de lectura de archivos **no** da acceso al disco: si no hay
//! ninguna carpeta compartida declarada, la politica rechaza todas las rutas.
//! Las dos cosas tienen que concederse por separado.
//!
//! ## Por que las descargas salen de aqui a medias
//!
//! Leer un archivo y enviarlo tarda. Hacerlo en el mismo bucle que atiende el
//! raton dejaria el raton congelado hasta que terminara la descarga, asi que
//! [`FileService::start_download`] no envia nada: entrega un [`Download`] que
//! el tiempo de ejecucion pone en una tarea aparte.
//!
//! Esa tarea informa de lo que va enviando ([`TransferReport`]) y es este
//! servicio quien lleva la cuenta. De esa forma la contabilidad sigue teniendo
//! un solo dueno, aunque los bytes salgan desde otro hilo.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nexus_core::protocol::control::{ControlResponse, HostEvent, TransferState};
use nexus_core::{Timestamp, TransferId};
use nexus_file_transfer::error::{FileError, FileResult};
use nexus_file_transfer::{
    Accepted, Access, AccessPolicy, ChunkReader, ChunkWriter, Direction, Frame, TransferTracker,
    list_dir_where,
};

/// Bytes que tienen que pasar antes de volver a informar del progreso.
///
/// Sin un minimo, una descarga de un giga enviaria dieciseis mil mensajes de
/// progreso por el canal de control, compitiendo con el raton por el mismo
/// canal para dibujar la misma barra.
const PROGRESS_EVERY_BYTES: u64 = 1024 * 1024;

/// Tiempo maximo sin informar, aunque no se hayan movido esos bytes.
///
/// Por una conexion lenta, el minimo de bytes dejaria la barra quieta minutos
/// enteros y pareceria que la transferencia se colgo.
const PROGRESS_EVERY_MS: u64 = 500;

/// Una descarga lista para ponerse en marcha en su propia tarea.
#[derive(Debug)]
pub struct Download {
    /// Transferencia a la que corresponde.
    pub transfer_id: TransferId,
    /// Tamano total del archivo.
    pub total_bytes: u64,
    /// Lector ya abierto sobre la ruta autorizada.
    pub reader: ChunkReader,
    /// Bandera que la cancela desde fuera.
    pub cancelled: Arc<AtomicBool>,
}

/// Lo que la tarea de envio cuenta de vuelta al servicio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferReport {
    /// Transferencia afectada.
    pub transfer_id: TransferId,
    /// Bytes enviados **en total**, no desde el informe anterior.
    ///
    /// Es acumulado a proposito. Si fuera un incremento, un informe que se
    /// perdiera porque la cola estaba llena descuadraria la cuenta para el
    /// resto de la transferencia. Siendo el total, perder informes solo hace
    /// que la barra avance a saltos, y el ultimo deja la cuenta correcta.
    pub sent_bytes: u64,
    /// Como termino, si termino.
    pub outcome: Option<TransferOutcome>,
}

/// Final de una transferencia vista por la tarea que la movia.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferOutcome {
    /// Se envio el archivo completo.
    Completed,
    /// Se corto porque alguien la cancelo.
    Cancelled,
    /// Fallo, con el motivo para el registro.
    Failed(String),
}

/// Servicio de archivos de una sesion.
#[derive(Debug)]
pub struct FileService {
    policy: AccessPolicy,
    tracker: TransferTracker,
    uploads: HashMap<TransferId, ChunkWriter>,
    downloads: HashMap<TransferId, Arc<AtomicBool>>,
    last_report: HashMap<TransferId, (u64, Timestamp)>,
}

impl FileService {
    /// Crea el servicio con las carpetas que el host comparte.
    ///
    /// Una politica vacia es una politica valida y es la que se usa por
    /// omision: no se comparte nada hasta que alguien lo declare.
    #[must_use]
    pub fn new(policy: AccessPolicy) -> Self {
        Self {
            policy,
            tracker: TransferTracker::new(),
            uploads: HashMap::new(),
            downloads: HashMap::new(),
            last_report: HashMap::new(),
        }
    }

    /// Indica si el host no comparte ninguna carpeta.
    #[must_use]
    pub fn shares_nothing(&self) -> bool {
        self.policy.is_empty()
    }

    /// Transferencias activas en esta sesion.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.tracker.active_count()
    }

    /// Lista una pagina de un directorio.
    ///
    /// # Errors
    ///
    /// Lo que devuelva la politica de acceso, o
    /// [`FileError::WrongKind`] si la ruta no es un directorio.
    pub fn list(&self, path: &str, offset: u64) -> FileResult<ControlResponse> {
        let resolved = self.policy.resolve(Path::new(path), Access::Read)?;
        // El listado ensena solo lo que de verdad se puede abrir. Un acceso
        // directo que apunte fuera de las carpetas compartidas no aparece: si
        // apareciera, su tamano y su fecha contarian algo de fuera aunque
        // descargarlo se rechazara despues.
        let listing = list_dir_where(&resolved, offset, |candidato| {
            self.policy.resolve(candidato, Access::Read).is_ok()
        })?;
        Ok(ControlResponse::DirListing {
            // Se devuelve la ruta tal como la resolvio el host, no la que
            // llego: asi quien pregunta sabe sobre que se le contesto.
            path: resolved.to_string_lossy().into_owned(),
            entries: listing.entries,
            offset: listing.offset,
            total_entries: listing.total_entries,
            scan_truncated: listing.scan_truncated,
        })
    }

    /// Abre una descarga y la deja lista para enviarse.
    ///
    /// No envia nada: devuelve la respuesta para quien pregunto y el
    /// [`Download`] que hay que poner en marcha.
    ///
    /// # Errors
    ///
    /// Lo que devuelva la politica de acceso, mas
    /// [`FileError::TooManyTransfers`] si ya hay demasiadas en curso y
    /// [`FileError::WrongKind`] si la ruta es un directorio.
    pub fn start_download(
        &mut self,
        path: &str,
        now: Timestamp,
    ) -> FileResult<(ControlResponse, Download)> {
        let resolved = self.policy.resolve(Path::new(path), Access::Read)?;

        // El lector se abre antes de apuntar la transferencia: si el archivo no
        // existe o es una carpeta, no se ocupa una de las plazas. Y el tamano
        // que se anuncia es el del archivo que de verdad se va a leer.
        // El archivo se abre una sola vez. Abrirlo para mirar el tamano y
        // otra vez para leerlo permitiria que cambiara entre medias: se
        // anunciaria un tamano y se enviaria otro, y quien recibe lo veria
        // como un fallo suyo.
        let reader = ChunkReader::open(TransferId::new(), &resolved)?;
        let total_bytes = reader.total_bytes();

        let transfer = self.tracker.start(
            Direction::Download,
            resolved.clone(),
            Some(total_bytes),
            now,
        )?;
        let reader = reader.with_id(transfer.id);

        let cancelled = Arc::new(AtomicBool::new(false));
        self.downloads.insert(transfer.id, Arc::clone(&cancelled));

        Ok((
            ControlResponse::TransferAccepted {
                transfer_id: transfer.id,
                size_bytes: Some(total_bytes),
            },
            Download {
                transfer_id: transfer.id,
                total_bytes,
                reader,
                cancelled,
            },
        ))
    }

    /// Abre una subida y queda esperando sus trozos.
    ///
    /// # Errors
    ///
    /// Lo que devuelva la politica de acceso (incluido
    /// [`FileError::ReadOnlyRoot`] si la carpeta no admite escritura), mas
    /// [`FileError::TooLarge`] si el tamano anunciado pasa del limite,
    /// [`FileError::TooManyTransfers`] si no quedan plazas y
    /// [`FileError::AlreadyExists`] si el destino ya existe.
    pub fn start_upload(
        &mut self,
        path: &str,
        size_bytes: u64,
        now: Timestamp,
    ) -> FileResult<ControlResponse> {
        let resolved = self.policy.resolve(Path::new(path), Access::Write)?;

        // Primero los limites, que no tocan el disco, y luego el archivo
        // temporal: si la subida no cabe, no se ha creado nada.
        let transfer =
            self.tracker
                .start(Direction::Upload, resolved.clone(), Some(size_bytes), now)?;

        match ChunkWriter::create(transfer.id, &resolved, size_bytes) {
            Ok(writer) => {
                self.uploads.insert(transfer.id, writer);
                Ok(ControlResponse::TransferAccepted {
                    transfer_id: transfer.id,
                    size_bytes: Some(size_bytes),
                })
            }
            Err(error) => {
                // La plaza se devuelve: si no se puede escribir, la
                // transferencia no existe.
                let _ = self.tracker.fail(transfer.id, now);
                self.tracker.forget_finished();
                Err(error)
            }
        }
    }

    /// Cancela una transferencia de cualquiera de los dos sentidos.
    ///
    /// # Errors
    ///
    /// - [`FileError::UnknownTransfer`] si no existe.
    /// - [`FileError::TransferFinished`] si ya habia terminado.
    pub fn cancel(&mut self, transfer_id: TransferId, now: Timestamp) -> FileResult<HostEvent> {
        let transfer = self.tracker.cancel(transfer_id, now)?;

        if let Some(mut writer) = self.uploads.remove(&transfer_id) {
            writer.abort();
        }
        if let Some(flag) = self.downloads.remove(&transfer_id) {
            // La tarea que esta enviando lo vera en su siguiente vuelta y
            // parara. No se la interrumpe a la fuerza: dejarla terminar su
            // trozo evita enviar un marco a medias.
            flag.store(true, Ordering::Relaxed);
        }
        self.last_report.remove(&transfer_id);

        Ok(HostEvent::TransferProgress {
            transfer_id,
            transferred_bytes: transfer.transferred_bytes,
            total_bytes: transfer.total_bytes,
            state: TransferState::Cancelled,
        })
    }

    /// Atiende un marco recibido por el canal de archivos.
    ///
    /// Devuelve el evento de progreso que toque enviar, o `None` cuando no toca
    /// informar todavia.
    ///
    /// # Errors
    ///
    /// - [`FileError::MalformedFrame`] si el marco no respeta el formato.
    /// - [`FileError::UnknownTransfer`] si no corresponde a ninguna subida
    ///   abierta. Incluye el caso de un marco que pretenda alimentar una
    ///   **descarga**: por el canal de archivos solo entran subidas.
    /// - Lo que devuelva [`ChunkWriter::accept`] al verificar el contenido.
    pub fn on_frame(&mut self, raw: &[u8], now: Timestamp) -> FileResult<Option<HostEvent>> {
        let frame = Frame::parse(raw)?;
        let transfer_id = frame.transfer_id();

        let writer = self
            .uploads
            .get_mut(&transfer_id)
            .ok_or(FileError::UnknownTransfer)?;

        let accepted = match writer.accept(&frame) {
            Ok(accepted) => accepted,
            Err(error) => {
                // El escritor ya borro el temporal. Aqui se cierra la
                // contabilidad y se suelta la plaza.
                self.uploads.remove(&transfer_id);
                let _ = self.tracker.fail(transfer_id, now);
                self.last_report.remove(&transfer_id);
                return Err(error);
            }
        };

        match accepted {
            Accepted::Progress { written_bytes } => {
                let transfer = self.advance_to(transfer_id, written_bytes, now)?;
                Ok(self.maybe_report(transfer_id, written_bytes, transfer.total_bytes, now))
            }
            Accepted::Completed { path, total_bytes } => {
                self.uploads.remove(&transfer_id);
                self.last_report.remove(&transfer_id);
                self.advance_to(transfer_id, total_bytes, now)?;
                // Solo ahora: el contenido esta escrito y verificado. Hasta
                // esta linea la subida podia rechazarse.
                let transfer = self.tracker.complete(transfer_id, now)?;
                tracing::info!(
                    ruta = %path.display(),
                    bytes = total_bytes,
                    "subida completada y verificada"
                );
                Ok(Some(HostEvent::TransferProgress {
                    transfer_id,
                    transferred_bytes: total_bytes,
                    total_bytes: transfer.total_bytes,
                    state: TransferState::Completed,
                }))
            }
            Accepted::Aborted => {
                self.uploads.remove(&transfer_id);
                self.last_report.remove(&transfer_id);
                let transfer = self.tracker.cancel(transfer_id, now).ok();
                Ok(Some(HostEvent::TransferProgress {
                    transfer_id,
                    transferred_bytes: transfer.as_ref().map_or(0, |t| t.transferred_bytes),
                    total_bytes: transfer.and_then(|t| t.total_bytes),
                    state: TransferState::Cancelled,
                }))
            }
        }
    }

    /// Recoge lo que cuenta la tarea que esta enviando una descarga.
    ///
    /// Devuelve el evento de progreso que toque enviar, si toca. Un informe de
    /// una transferencia que ya se olvido se ignora sin ruido: pasa cuando se
    /// cancela mientras la tarea seguia enviando su ultimo trozo.
    pub fn on_report(&mut self, report: &TransferReport, now: Timestamp) -> Option<HostEvent> {
        let transfer_id = report.transfer_id;
        let total = self.tracker.get(transfer_id)?.total_bytes;

        // La cuenta se pone al total que dice la tarea, no se le suma: asi dos
        // informes del mismo byte, o uno perdido, dan lo mismo.
        let sent = self
            .advance_to(transfer_id, report.sent_bytes, now)
            .ok()
            .map_or(report.sent_bytes, |t| t.transferred_bytes);

        let Some(outcome) = report.outcome.as_ref() else {
            return self.maybe_report(transfer_id, sent, total, now);
        };

        let state = match outcome {
            TransferOutcome::Completed => {
                // Una descarga puede haberse cerrado sola al salir el ultimo
                // byte; completarla otra vez no es un error que importe.
                let _ = self.tracker.complete(transfer_id, now);
                TransferState::Completed
            }
            TransferOutcome::Cancelled => {
                let _ = self.tracker.cancel(transfer_id, now);
                TransferState::Cancelled
            }
            TransferOutcome::Failed(reason) => {
                tracing::warn!(
                    transferencia = %transfer_id.short(),
                    motivo = %reason,
                    "una descarga termino con error"
                );
                let _ = self.tracker.fail(transfer_id, now);
                TransferState::Failed
            }
        };
        self.finish(transfer_id);
        Some(HostEvent::TransferProgress {
            transfer_id,
            transferred_bytes: sent,
            total_bytes: total,
            state,
        })
    }

    /// Cierra todas las transferencias de la sesion.
    ///
    /// Se llama al terminar la sesion. Las subidas a medias se abandonan y sus
    /// temporales se borran: un archivo incompleto no se queda en la carpeta
    /// compartida porque el iPad se fuera de cobertura.
    pub fn end(&mut self, now: Timestamp) {
        for (_, mut writer) in self.uploads.drain() {
            writer.abort();
        }
        for (_, flag) in self.downloads.drain() {
            flag.store(true, Ordering::Relaxed);
        }
        for transfer in self.tracker.active() {
            let _ = self.tracker.cancel(transfer.id, now);
        }
        self.last_report.clear();
        self.tracker.forget_finished();
    }

    fn finish(&mut self, transfer_id: TransferId) {
        self.downloads.remove(&transfer_id);
        self.last_report.remove(&transfer_id);
    }

    /// Lleva la cuenta del tracker al total que dice la capa de disco.
    ///
    /// El escritor es la fuente de verdad de cuantos bytes hay en el archivo;
    /// el tracker solo sabe sumar. Se le pasa la diferencia.
    fn advance_to(
        &mut self,
        transfer_id: TransferId,
        written_bytes: u64,
        now: Timestamp,
    ) -> FileResult<nexus_file_transfer::Transfer> {
        let already = self
            .tracker
            .get(transfer_id)
            .map_or(0, |t| t.transferred_bytes);
        let delta = written_bytes.saturating_sub(already);
        self.tracker.advance(transfer_id, delta, now)
    }

    /// Decide si toca informar del progreso.
    fn maybe_report(
        &mut self,
        transfer_id: TransferId,
        transferred_bytes: u64,
        total_bytes: Option<u64>,
        now: Timestamp,
    ) -> Option<HostEvent> {
        let (last_bytes, last_at) = self
            .last_report
            .get(&transfer_id)
            .copied()
            .unwrap_or((0, Timestamp::EPOCH));

        let enough_bytes = transferred_bytes.saturating_sub(last_bytes) >= PROGRESS_EVERY_BYTES;
        let enough_time = now.0.saturating_sub(last_at.0) >= PROGRESS_EVERY_MS;
        if !enough_bytes && !enough_time {
            return None;
        }

        self.last_report
            .insert(transfer_id, (transferred_bytes, now));
        Some(HostEvent::TransferProgress {
            transfer_id,
            transferred_bytes,
            total_bytes,
            state: TransferState::Active,
        })
    }
}

/// Codigo estable con el que se rechaza una operacion de archivos.
///
/// Dos rechazos comparten codigo a proposito: una ruta fuera de las carpetas
/// compartidas y una que no existe dan lo mismo (`access_denied`), porque
/// distinguirlas permitiria averiguar que hay en el disco del host preguntando
/// una ruta tras otra.
#[must_use]
pub const fn deny_code(error: &FileError) -> &'static str {
    match error {
        FileError::OutsideRoots
        | FileError::PathTraversal
        | FileError::NotFound
        | FileError::ForbiddenName { .. } => "access_denied",
        FileError::ReadOnlyRoot => "read_only",
        FileError::TooLarge { .. } => "too_large",
        FileError::TooManyTransfers { .. } => "too_many_transfers",
        FileError::UnknownTransfer => "unknown_transfer",
        FileError::TransferFinished => "transfer_finished",
        FileError::AlreadyExists => "already_exists",
        FileError::WrongKind { .. } => "wrong_kind",
        FileError::MalformedFrame { .. }
        | FileError::OutOfOrder { .. }
        | FileError::FrameCountMismatch { .. }
        | FileError::Incomplete { .. } => "bad_transfer",
        FileError::IntegrityMismatch => "integrity_mismatch",
        FileError::Io(_) => "io",
    }
}

/// Mensaje que se envia al otro extremo por un fallo de archivos.
///
/// Las rutas que no se pueden tocar dan todas el mismo texto, y un error de
/// disco no envia el mensaje del sistema: ese queda en el registro del host,
/// donde lo lee su dueno y no quien pregunta.
#[must_use]
pub fn deny_message(error: &FileError) -> String {
    match error {
        FileError::OutsideRoots
        | FileError::PathTraversal
        | FileError::NotFound
        | FileError::ForbiddenName { .. } => {
            "esa ruta no esta entre las carpetas compartidas".to_owned()
        }
        FileError::Io(_) => "el host no pudo completar la operacion en el disco".to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_file_transfer::SharedRoot;
    use std::path::PathBuf;

    const NOW: Timestamp = Timestamp(1_000);

    struct Fixture {
        _temp: tempfile::TempDir,
        lectura: PathBuf,
        escritura: PathBuf,
        fuera: PathBuf,
        service: FileService,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().expect("temporal");
        let lectura = temp.path().join("documentos");
        let escritura = temp.path().join("intercambio");
        let fuera = temp.path().join("privado");
        for dir in [&lectura, &escritura, &fuera] {
            std::fs::create_dir_all(dir).expect("crea");
        }
        std::fs::write(lectura.join("informe.txt"), b"contenido del informe").expect("crea");
        std::fs::write(fuera.join("claves.txt"), b"secreto").expect("crea");

        let policy = AccessPolicy::new(vec![
            SharedRoot::read_only("Documentos", &lectura),
            SharedRoot::writable("Intercambio", &escritura),
        ]);
        Fixture {
            _temp: temp,
            lectura,
            escritura,
            fuera,
            service: FileService::new(policy),
        }
    }

    fn ruta(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    // --- Lo que no se puede tocar ---------------------------------------

    #[test]
    fn with_no_shared_folders_nothing_can_be_reached() {
        let temp = tempfile::tempdir().expect("temporal");
        std::fs::write(temp.path().join("a.txt"), b"x").expect("crea");
        let mut service = FileService::new(AccessPolicy::empty());
        assert!(service.shares_nothing());

        assert_eq!(
            service.list(&ruta(temp.path()), 0).unwrap_err(),
            FileError::OutsideRoots
        );
        assert_eq!(
            service
                .start_download(&ruta(&temp.path().join("a.txt")), NOW)
                .unwrap_err(),
            FileError::OutsideRoots
        );
        assert_eq!(
            service
                .start_upload(&ruta(&temp.path().join("b.txt")), 1, NOW)
                .unwrap_err(),
            FileError::OutsideRoots
        );
    }

    #[test]
    fn a_file_outside_the_shared_folders_cannot_be_downloaded() {
        let mut f = fixture();
        assert_eq!(
            f.service
                .start_download(&ruta(&f.fuera.join("claves.txt")), NOW)
                .unwrap_err(),
            FileError::OutsideRoots
        );
    }

    #[test]
    fn traversal_out_of_a_shared_folder_is_refused() {
        let mut f = fixture();
        let ataque = ruta(&f.lectura.join("..").join("privado").join("claves.txt"));
        assert_eq!(
            f.service.start_download(&ataque, NOW).unwrap_err(),
            FileError::PathTraversal
        );
    }

    #[test]
    fn a_read_only_folder_refuses_uploads() {
        let mut f = fixture();
        assert_eq!(
            f.service
                .start_upload(&ruta(&f.lectura.join("nuevo.txt")), 10, NOW)
                .unwrap_err(),
            FileError::ReadOnlyRoot
        );
        assert!(!f.lectura.join("nuevo.txt").exists());
    }

    #[test]
    fn a_denied_path_and_a_missing_one_are_indistinguishable() {
        let f = fixture();
        let fuera = f.service.list(&ruta(&f.fuera), 0).unwrap_err();
        let inexistente = f
            .service
            .list(&ruta(&f.lectura.join("no-existe")), 0)
            .unwrap_err();
        assert_eq!(deny_code(&fuera), deny_code(&inexistente));
        assert_eq!(deny_message(&fuera), deny_message(&inexistente));
    }

    #[test]
    fn a_disk_error_does_not_travel_to_the_other_end() {
        let error = FileError::Io("/home/usuario/privado/x: permiso denegado".to_owned());
        let mensaje = deny_message(&error);
        assert!(
            !mensaje.contains("privado"),
            "el mensaje no debe llevar la ruta: {mensaje}"
        );
        assert_eq!(deny_code(&error), "io");
    }

    // --- Listado ---------------------------------------------------------

    #[test]
    fn listing_a_shared_folder_works_and_reports_the_resolved_path() {
        let f = fixture();
        let response = f.service.list(&ruta(&f.lectura), 0).expect("lista");
        match response {
            ControlResponse::DirListing {
                entries,
                total_entries,
                offset,
                scan_truncated,
                ..
            } => {
                assert_eq!(total_entries, 1);
                assert_eq!(offset, 0);
                assert!(!scan_truncated);
                assert_eq!(entries[0].name, "informe.txt");
            }
            other => panic!("se esperaba un listado: {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn un_acceso_directo_que_apunta_fuera_no_sale_en_el_listado() {
        // Lo que se protege: el tamano y la fecha de lo que hay fuera de las
        // carpetas compartidas. Descargarlo ya se rechazaba; lo que faltaba
        // era que no se contara al listar.
        let f = fixture();
        std::os::unix::fs::symlink(&f.fuera, f.lectura.join("atajo")).expect("crea enlace");

        let response = f.service.list(&ruta(&f.lectura), 0).expect("lista");
        match response {
            ControlResponse::DirListing {
                entries,
                total_entries,
                ..
            } => {
                let nombres: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
                assert!(
                    !nombres.contains(&"atajo"),
                    "el enlace que escapa no debe aparecer: {nombres:?}"
                );
                assert_eq!(nombres, ["informe.txt"]);
                assert_eq!(total_entries, 1, "el total tampoco debe contarlo");
            }
            otro => panic!("se esperaba un listado: {otro:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn un_acceso_directo_que_se_queda_dentro_si_sale() {
        // El reverso: un enlace que no escapa es util y tiene que verse.
        let f = fixture();
        std::fs::create_dir_all(f.lectura.join("real")).expect("crea");
        std::os::unix::fs::symlink(f.lectura.join("real"), f.lectura.join("alias"))
            .expect("crea enlace");

        let response = f.service.list(&ruta(&f.lectura), 0).expect("lista");
        match response {
            ControlResponse::DirListing { entries, .. } => {
                let alias = entries
                    .iter()
                    .find(|e| e.name == "alias")
                    .expect("deberia verse");
                assert!(
                    alias.is_dir,
                    "un enlace interno a carpeta se ve como carpeta"
                );
            }
            otro => panic!("se esperaba un listado: {otro:?}"),
        }
    }

    #[test]
    fn listing_a_file_instead_of_a_folder_is_refused() {
        let f = fixture();
        assert_eq!(
            f.service
                .list(&ruta(&f.lectura.join("informe.txt")), 0)
                .unwrap_err(),
            FileError::WrongKind {
                expected: "un directorio"
            }
        );
    }

    // --- Descarga --------------------------------------------------------

    #[test]
    fn a_download_is_accepted_with_the_real_size() {
        let mut f = fixture();
        let (response, download) = f
            .service
            .start_download(&ruta(&f.lectura.join("informe.txt")), NOW)
            .expect("acepta");

        let esperado = b"contenido del informe".len() as u64;
        match response {
            ControlResponse::TransferAccepted {
                transfer_id,
                size_bytes,
            } => {
                assert_eq!(transfer_id, download.transfer_id);
                assert_eq!(size_bytes, Some(esperado));
            }
            other => panic!("se esperaba aceptacion: {other:?}"),
        }
        assert_eq!(download.total_bytes, esperado);
        assert_eq!(download.reader.total_bytes(), esperado);
        assert_eq!(f.service.active_count(), 1);
    }

    #[test]
    fn a_directory_cannot_be_downloaded_and_does_not_take_a_slot() {
        let mut f = fixture();
        assert_eq!(
            f.service
                .start_download(&ruta(&f.lectura), NOW)
                .unwrap_err(),
            FileError::WrongKind {
                expected: "un archivo"
            }
        );
        assert_eq!(
            f.service.active_count(),
            0,
            "un fallo no debe gastar una plaza"
        );
    }

    #[test]
    fn concurrent_transfers_are_capped_and_the_cap_frees_up() {
        let mut f = fixture();
        for i in 0..TransferTracker::DEFAULT_MAX_CONCURRENT {
            std::fs::write(f.lectura.join(format!("a{i}.txt")), b"x").expect("crea");
            f.service
                .start_download(&ruta(&f.lectura.join(format!("a{i}.txt"))), NOW)
                .expect("acepta");
        }
        let err = f
            .service
            .start_download(&ruta(&f.lectura.join("informe.txt")), NOW)
            .unwrap_err();
        assert_eq!(
            err,
            FileError::TooManyTransfers {
                max: TransferTracker::DEFAULT_MAX_CONCURRENT
            }
        );
    }

    #[test]
    fn cancelling_a_download_raises_its_flag() {
        let mut f = fixture();
        let (_, download) = f
            .service
            .start_download(&ruta(&f.lectura.join("informe.txt")), NOW)
            .expect("acepta");
        assert!(!download.cancelled.load(Ordering::Relaxed));

        let event = f
            .service
            .cancel(download.transfer_id, Timestamp(2_000))
            .expect("cancela");
        assert!(
            download.cancelled.load(Ordering::Relaxed),
            "la tarea que envia tiene que enterarse"
        );
        assert!(matches!(
            event,
            HostEvent::TransferProgress {
                state: TransferState::Cancelled,
                ..
            }
        ));
    }

    #[test]
    fn cancelling_something_that_does_not_exist_is_reported() {
        let mut f = fixture();
        assert_eq!(
            f.service.cancel(TransferId::new(), NOW).unwrap_err(),
            FileError::UnknownTransfer
        );
    }

    // --- Subida ----------------------------------------------------------

    /// Sube `content` entero y devuelve el ultimo evento.
    fn subir(service: &mut FileService, destino: &Path, content: &[u8]) -> Option<HostEvent> {
        let response = service
            .start_upload(&ruta(destino), content.len() as u64, NOW)
            .expect("acepta la subida");
        let ControlResponse::TransferAccepted { transfer_id, .. } = response else {
            panic!("se esperaba aceptacion");
        };

        let mut sequence = 0;
        for trozo in content.chunks(nexus_file_transfer::CHUNK_SIZE_BYTES) {
            let frame = Frame::Data {
                transfer_id,
                sequence,
                payload: trozo.to_vec(),
            };
            service
                .on_frame(&frame.encode(), NOW)
                .expect("acepta el trozo");
            sequence += 1;
        }
        let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(content).into();
        let end = Frame::End {
            transfer_id,
            frames: sequence,
            digest,
        };
        service.on_frame(&end.encode(), NOW).expect("cierra")
    }

    #[test]
    fn an_upload_lands_in_the_shared_folder_byte_for_byte() {
        let mut f = fixture();
        let destino = f.escritura.join("subido.txt");
        let content = b"lo que sube el iPad".to_vec();

        let event = subir(&mut f.service, &destino, &content);
        assert!(matches!(
            event,
            Some(HostEvent::TransferProgress {
                state: TransferState::Completed,
                ..
            })
        ));
        assert_eq!(std::fs::read(&destino).expect("lee"), content);
        assert_eq!(f.service.active_count(), 0, "la plaza queda libre");
    }

    #[test]
    fn an_upload_larger_than_one_chunk_lands_intact() {
        let mut f = fixture();
        let destino = f.escritura.join("grande.bin");
        let content: Vec<u8> = (0..nexus_file_transfer::CHUNK_SIZE_BYTES * 2 + 7)
            .map(|i| (i % 253) as u8)
            .collect();

        subir(&mut f.service, &destino, &content);
        assert_eq!(std::fs::read(&destino).expect("lee"), content);
    }

    #[test]
    fn an_upload_with_a_tampered_digest_never_reaches_the_folder() {
        let mut f = fixture();
        let destino = f.escritura.join("falso.txt");
        let response = f
            .service
            .start_upload(&ruta(&destino), 5, NOW)
            .expect("acepta");
        let ControlResponse::TransferAccepted { transfer_id, .. } = response else {
            panic!("se esperaba aceptacion");
        };

        f.service
            .on_frame(
                &Frame::Data {
                    transfer_id,
                    sequence: 0,
                    payload: b"hola!".to_vec(),
                }
                .encode(),
                NOW,
            )
            .expect("acepta el trozo");

        let err = f
            .service
            .on_frame(
                &Frame::End {
                    transfer_id,
                    frames: 1,
                    digest: [0_u8; 32],
                }
                .encode(),
                NOW,
            )
            .unwrap_err();

        assert_eq!(err, FileError::IntegrityMismatch);
        assert!(!destino.exists());
        assert_eq!(deny_code(&err), "integrity_mismatch");
        // Y la plaza se devuelve, no se queda ocupada por una subida muerta.
        assert_eq!(f.service.active_count(), 0);
    }

    #[test]
    fn a_frame_for_an_unknown_transfer_is_refused() {
        let mut f = fixture();
        let frame = Frame::Data {
            transfer_id: TransferId::new(),
            sequence: 0,
            payload: b"x".to_vec(),
        };
        assert_eq!(
            f.service.on_frame(&frame.encode(), NOW).unwrap_err(),
            FileError::UnknownTransfer
        );
    }

    #[test]
    fn a_download_cannot_be_fed_through_the_files_channel() {
        // Un cliente podria intentar escribir en el archivo que esta
        // descargando enviando marcos con su identificador.
        let mut f = fixture();
        let (_, download) = f
            .service
            .start_download(&ruta(&f.lectura.join("informe.txt")), NOW)
            .expect("acepta");
        let frame = Frame::Data {
            transfer_id: download.transfer_id,
            sequence: 0,
            payload: b"contenido inyectado".to_vec(),
        };
        assert_eq!(
            f.service.on_frame(&frame.encode(), NOW).unwrap_err(),
            FileError::UnknownTransfer
        );
        assert_eq!(
            std::fs::read(f.lectura.join("informe.txt")).expect("lee"),
            b"contenido del informe",
            "el archivo de origen queda intacto"
        );
    }

    #[test]
    fn garbage_on_the_files_channel_is_refused_without_killing_anything() {
        let mut f = fixture();
        for basura in [
            vec![],
            vec![0_u8; 5],
            vec![9_u8; 40],
            vec![0_u8; nexus_file_transfer::MAX_FRAME_BYTES + 1],
        ] {
            let err = f.service.on_frame(&basura, NOW).unwrap_err();
            assert!(
                matches!(err, FileError::MalformedFrame { .. }),
                "{err:?} deberia ser un marco mal formado"
            );
        }
        // Y despues de la basura, una subida legitima sigue funcionando.
        let destino = f.escritura.join("despues.txt");
        subir(&mut f.service, &destino, b"sigue funcionando");
        assert_eq!(std::fs::read(&destino).expect("lee"), b"sigue funcionando");
    }

    #[test]
    fn an_upload_that_exceeds_what_it_announced_is_cut_off() {
        let mut f = fixture();
        let destino = f.escritura.join("mentiroso.bin");
        let response = f
            .service
            .start_upload(&ruta(&destino), 4, NOW)
            .expect("acepta");
        let ControlResponse::TransferAccepted { transfer_id, .. } = response else {
            panic!("se esperaba aceptacion");
        };

        let err = f
            .service
            .on_frame(
                &Frame::Data {
                    transfer_id,
                    sequence: 0,
                    payload: vec![0_u8; 100],
                }
                .encode(),
                NOW,
            )
            .unwrap_err();
        assert!(matches!(err, FileError::TooLarge { .. }), "{err:?}");
        assert!(!destino.exists());
        assert_eq!(f.service.active_count(), 0);
    }

    #[test]
    fn an_upload_over_the_size_limit_is_refused_before_touching_the_disk() {
        let mut f = fixture();
        let destino = f.escritura.join("enorme.iso");
        let err = f
            .service
            .start_upload(&ruta(&destino), u64::MAX, NOW)
            .unwrap_err();
        assert!(matches!(err, FileError::TooLarge { .. }), "{err:?}");
        let restos: Vec<_> = std::fs::read_dir(&f.escritura)
            .expect("lee")
            .filter_map(Result::ok)
            .collect();
        assert!(restos.is_empty(), "no deberia haberse creado nada");
    }

    #[test]
    fn an_existing_destination_is_not_overwritten_and_frees_the_slot() {
        let mut f = fixture();
        let destino = f.escritura.join("ya-esta.txt");
        std::fs::write(&destino, b"lo de antes").expect("crea");

        assert_eq!(
            f.service.start_upload(&ruta(&destino), 5, NOW).unwrap_err(),
            FileError::AlreadyExists
        );
        assert_eq!(std::fs::read(&destino).expect("lee"), b"lo de antes");
        assert_eq!(
            f.service.active_count(),
            0,
            "un destino ocupado no deja la plaza tomada"
        );
    }

    // --- Progreso --------------------------------------------------------

    #[test]
    fn progress_is_reported_once_at_the_start_and_not_per_chunk() {
        let mut f = fixture();
        let destino = f.escritura.join("silencioso.bin");
        let total = nexus_file_transfer::CHUNK_SIZE_BYTES * 4;
        let response = f
            .service
            .start_upload(&ruta(&destino), total as u64, NOW)
            .expect("acepta");
        let ControlResponse::TransferAccepted { transfer_id, .. } = response else {
            panic!("se esperaba aceptacion");
        };

        // Mismo instante en todos, asi que solo el minimo de bytes podria
        // disparar un informe, y cuatro trozos de 63 KiB no llegan al
        // mebibyte. El primero si informa, porque la barra tiene que aparecer
        // en cuanto empieza y no medio segundo despues.
        let mut informes = 0;
        for sequence in 0..4 {
            let frame = Frame::Data {
                transfer_id,
                sequence,
                payload: vec![0_u8; nexus_file_transfer::CHUNK_SIZE_BYTES],
            };
            if f.service
                .on_frame(&frame.encode(), NOW)
                .expect("acepta")
                .is_some()
            {
                informes += 1;
            }
        }
        assert_eq!(
            informes, 1,
            "deberia informar al empezar y callarse despues, no informar por cada trozo"
        );
    }

    #[test]
    fn progress_is_reported_once_enough_bytes_have_passed() {
        let mut f = fixture();
        let destino = f.escritura.join("ruidoso.bin");
        let total = PROGRESS_EVERY_BYTES * 3;
        let response = f
            .service
            .start_upload(&ruta(&destino), total, NOW)
            .expect("acepta");
        let ControlResponse::TransferAccepted { transfer_id, .. } = response else {
            panic!("se esperaba aceptacion");
        };

        let trozo = nexus_file_transfer::CHUNK_SIZE_BYTES;
        let mut informes = 0;
        for sequence in 0..(total as usize / trozo) {
            let frame = Frame::Data {
                transfer_id,
                sequence: sequence as u32,
                payload: vec![0_u8; trozo],
            };
            if f.service
                .on_frame(&frame.encode(), NOW)
                .expect("acepta")
                .is_some()
            {
                informes += 1;
            }
        }
        assert!(
            (2..=3).contains(&informes),
            "con tres mebibytes esperaba dos o tres informes, hubo {informes}"
        );
    }

    #[test]
    fn progress_is_reported_on_a_slow_link_even_without_enough_bytes() {
        let mut f = fixture();
        let destino = f.escritura.join("lento.bin");
        let response = f
            .service
            .start_upload(&ruta(&destino), 1_000, NOW)
            .expect("acepta");
        let ControlResponse::TransferAccepted { transfer_id, .. } = response else {
            panic!("se esperaba aceptacion");
        };

        // Primer trozo: informa, porque nunca se informo.
        f.service
            .on_frame(
                &Frame::Data {
                    transfer_id,
                    sequence: 0,
                    payload: vec![0_u8; 10],
                }
                .encode(),
                NOW,
            )
            .expect("acepta")
            .expect("el primero informa");

        // Enseguida, otro trozo pequeno: no informa.
        assert!(
            f.service
                .on_frame(
                    &Frame::Data {
                        transfer_id,
                        sequence: 1,
                        payload: vec![0_u8; 10],
                    }
                    .encode(),
                    NOW,
                )
                .expect("acepta")
                .is_none()
        );

        // Medio segundo despues, el mismo trozo pequeno si informa.
        let luego = Timestamp(NOW.0 + PROGRESS_EVERY_MS);
        assert!(
            f.service
                .on_frame(
                    &Frame::Data {
                        transfer_id,
                        sequence: 2,
                        payload: vec![0_u8; 10],
                    }
                    .encode(),
                    luego,
                )
                .expect("acepta")
                .is_some(),
            "por una conexion lenta la barra no puede quedarse quieta"
        );
    }

    // --- Informes de la tarea que envia ---------------------------------

    #[test]
    fn a_completed_download_report_closes_the_transfer() {
        let mut f = fixture();
        let (_, download) = f
            .service
            .start_download(&ruta(&f.lectura.join("informe.txt")), NOW)
            .expect("acepta");

        let event = f
            .service
            .on_report(
                &TransferReport {
                    transfer_id: download.transfer_id,
                    sent_bytes: download.total_bytes,
                    outcome: Some(TransferOutcome::Completed),
                },
                Timestamp(2_000),
            )
            .expect("informa");

        match event {
            HostEvent::TransferProgress {
                transferred_bytes,
                state,
                ..
            } => {
                assert_eq!(transferred_bytes, download.total_bytes);
                assert_eq!(state, TransferState::Completed);
            }
            other => panic!("se esperaba progreso: {other:?}"),
        }
        assert_eq!(f.service.active_count(), 0, "la plaza queda libre");
    }

    #[test]
    fn a_failed_download_report_frees_the_slot() {
        let mut f = fixture();
        let (_, download) = f
            .service
            .start_download(&ruta(&f.lectura.join("informe.txt")), NOW)
            .expect("acepta");

        let event = f
            .service
            .on_report(
                &TransferReport {
                    transfer_id: download.transfer_id,
                    sent_bytes: 0,
                    outcome: Some(TransferOutcome::Failed("el disco fallo".to_owned())),
                },
                NOW,
            )
            .expect("informa");
        assert!(matches!(
            event,
            HostEvent::TransferProgress {
                state: TransferState::Failed,
                ..
            }
        ));
        assert_eq!(f.service.active_count(), 0);
    }

    #[test]
    fn a_lost_progress_report_does_not_leave_the_count_short() {
        // El informe lleva el total, no el incremento: por eso saltarse uno
        // intermedio deja la cuenta correcta igualmente.
        let mut f = fixture();
        std::fs::write(f.lectura.join("mil.bin"), vec![0_u8; 1_000]).expect("crea");
        let (_, download) = f
            .service
            .start_download(&ruta(&f.lectura.join("mil.bin")), NOW)
            .expect("acepta");

        // Llega el informe de 300 bytes, se pierde el de 600, y llega el final.
        f.service.on_report(
            &TransferReport {
                transfer_id: download.transfer_id,
                sent_bytes: 300,
                outcome: None,
            },
            NOW,
        );
        let event = f
            .service
            .on_report(
                &TransferReport {
                    transfer_id: download.transfer_id,
                    sent_bytes: 1_000,
                    outcome: Some(TransferOutcome::Completed),
                },
                Timestamp(2_000),
            )
            .expect("informa");

        match event {
            HostEvent::TransferProgress {
                transferred_bytes,
                total_bytes,
                state,
                ..
            } => {
                assert_eq!(
                    transferred_bytes, 1_000,
                    "la cuenta final tiene que cuadrar"
                );
                assert_eq!(total_bytes, Some(1_000));
                assert_eq!(state, TransferState::Completed);
            }
            other => panic!("se esperaba progreso: {other:?}"),
        }
    }

    #[test]
    fn a_repeated_report_does_not_double_the_count() {
        let mut f = fixture();
        std::fs::write(f.lectura.join("mil.bin"), vec![0_u8; 1_000]).expect("crea");
        let (_, download) = f
            .service
            .start_download(&ruta(&f.lectura.join("mil.bin")), NOW)
            .expect("acepta");

        for _ in 0..3 {
            f.service.on_report(
                &TransferReport {
                    transfer_id: download.transfer_id,
                    sent_bytes: 400,
                    outcome: None,
                },
                NOW,
            );
        }
        let event = f
            .service
            .on_report(
                &TransferReport {
                    transfer_id: download.transfer_id,
                    sent_bytes: 400,
                    outcome: Some(TransferOutcome::Completed),
                },
                Timestamp(2_000),
            )
            .expect("informa");
        match event {
            HostEvent::TransferProgress {
                transferred_bytes, ..
            } => assert_eq!(transferred_bytes, 400, "no debe sumar cuatro veces"),
            other => panic!("se esperaba progreso: {other:?}"),
        }
    }

    #[test]
    fn a_report_for_a_transfer_already_forgotten_is_ignored() {
        let mut f = fixture();
        assert!(
            f.service
                .on_report(
                    &TransferReport {
                        transfer_id: TransferId::new(),
                        sent_bytes: 10,
                        outcome: None,
                    },
                    NOW,
                )
                .is_none(),
            "no deberia entrar en panico ni inventar un evento"
        );
    }

    // --- Fin de sesion ---------------------------------------------------

    #[test]
    fn ending_the_session_leaves_no_partial_files_behind() {
        let mut f = fixture();
        let destino = f.escritura.join("a-medias.bin");
        let response = f
            .service
            .start_upload(&ruta(&destino), 1_000, NOW)
            .expect("acepta");
        let ControlResponse::TransferAccepted { transfer_id, .. } = response else {
            panic!("se esperaba aceptacion");
        };
        f.service
            .on_frame(
                &Frame::Data {
                    transfer_id,
                    sequence: 0,
                    payload: vec![1_u8; 100],
                }
                .encode(),
                NOW,
            )
            .expect("acepta");

        f.service.end(Timestamp(5_000));

        let restos: Vec<_> = std::fs::read_dir(&f.escritura)
            .expect("lee")
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert!(
            restos.is_empty(),
            "la carpeta compartida no debe quedar con restos: {restos:?}"
        );
        assert_eq!(f.service.active_count(), 0);
    }

    #[test]
    fn ending_the_session_stops_the_downloads_in_flight() {
        let mut f = fixture();
        let (_, download) = f
            .service
            .start_download(&ruta(&f.lectura.join("informe.txt")), NOW)
            .expect("acepta");
        f.service.end(Timestamp(5_000));
        assert!(download.cancelled.load(Ordering::Relaxed));
    }

    #[test]
    fn every_error_has_a_distinct_stable_code_where_it_matters() {
        // Los codigos que la interfaz necesita distinguir no deben colapsarse.
        let distintos = [
            deny_code(&FileError::ReadOnlyRoot),
            deny_code(&FileError::AlreadyExists),
            deny_code(&FileError::IntegrityMismatch),
            deny_code(&FileError::TooManyTransfers { max: 4 }),
            deny_code(&FileError::OutsideRoots),
        ];
        let mut unicos = distintos.to_vec();
        unicos.sort_unstable();
        unicos.dedup();
        assert_eq!(unicos.len(), distintos.len(), "codigos repetidos");
    }
}
