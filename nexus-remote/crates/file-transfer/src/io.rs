//! Lectura y escritura de los bytes de un archivo.
//!
//! Aqui es donde la transferencia deja de ser contabilidad y toca el disco.
//! Dos piezas simetricas:
//!
//! - [`ChunkReader`] abre un archivo ya autorizado, lo entrega en trozos y va
//!   calculando su `SHA-256` al paso, sin leerlo entero en memoria.
//! - [`ChunkWriter`] recibe esos trozos y los escribe. **No** escribe en la
//!   ruta de destino: escribe en un archivo temporal de la misma carpeta y solo
//!   lo publica en su nombre definitivo cuando el resumen cuadra.
//!
//! ## Por que el archivo temporal
//!
//! Si se escribiera directamente en el destino, una transferencia cortada a
//! mitad dejaria ahi un archivo truncado con el nombre correcto, indistinguible
//! de uno completo. Peor aun con un archivo que ya existia: se habria destruido
//! para dejar un resto. Escribiendo aparte, el destino solo aparece cuando el
//! contenido esta entero y verificado, y si algo falla no queda nada.
//!
//! ## Que verifica quien recibe
//!
//! Quien envia cierra con el `SHA-256` de todo el archivo y cuantos trozos
//! mando. Quien recibe comprueba las tres cosas: que llegaron todos los trozos,
//! que la cuenta de bytes coincide con lo anunciado, y que el resumen del
//! contenido escrito coincide con el anunciado. Si cualquiera falla, el archivo
//! no llega a su destino.
//!
//! El resumen protege de la corrupcion, no del engano: lo calcula quien envia,
//! asi que un remitente malicioso puede mandar otro contenido con su resumen
//! correcto. De eso protege el cifrado del canal y la comprobacion de identidad
//! de la sesion, no esta capa.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use nexus_core::TransferId;
use sha2::{Digest, Sha256};

use crate::error::{FileError, FileResult};
use crate::frame::{DIGEST_BYTES, Frame};
use crate::transfer::CHUNK_SIZE_BYTES;

/// Lee un archivo del host y lo entrega en marcos listos para enviar.
#[derive(Debug)]
pub struct ChunkReader {
    transfer_id: TransferId,
    file: File,
    total_bytes: u64,
    sent_bytes: u64,
    next_sequence: u32,
    hasher: Sha256,
    closed: bool,
}

impl ChunkReader {
    /// Abre un archivo ya resuelto contra la politica de acceso.
    ///
    /// La ruta tiene que venir de
    /// [`AccessPolicy::resolve`](crate::access::AccessPolicy::resolve). Esta
    /// funcion no comprueba permisos: comprobarlos aqui tambien repartiria esa
    /// decision por dos sitios, y repartirla es como se pierde.
    ///
    /// # Errors
    ///
    /// - [`FileError::NotFound`] si la ruta no existe.
    /// - [`FileError::WrongKind`] si es un directorio.
    /// - [`FileError::Io`] ante cualquier otro fallo del sistema.
    pub fn open(transfer_id: TransferId, path: &Path) -> FileResult<Self> {
        let metadata = std::fs::metadata(path).map_err(|e| map_io(&e, path))?;
        if metadata.is_dir() {
            return Err(FileError::WrongKind {
                expected: "un archivo",
            });
        }
        let file = File::open(path).map_err(|e| map_io(&e, path))?;
        Ok(Self {
            transfer_id,
            file,
            total_bytes: metadata.len(),
            sent_bytes: 0,
            next_sequence: 0,
            hasher: Sha256::new(),
            closed: false,
        })
    }

    /// Tamano que tenia el archivo al abrirlo.
    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Bytes entregados hasta ahora.
    #[must_use]
    pub const fn sent_bytes(&self) -> u64 {
        self.sent_bytes
    }

    /// Entrega el siguiente marco, o `None` cuando ya no queda nada.
    ///
    /// Devuelve primero los marcos de datos y, al terminar, un unico
    /// [`Frame::End`] con el resumen y el numero de trozos. Despues, `None`.
    ///
    /// Nunca entrega mas bytes de los que tenia el archivo al abrirlo: si algo
    /// lo hace crecer mientras se lee, el resto no se envia. Asi lo que llega
    /// siempre cuadra con el tamano que se anuncio, en lugar de depender de
    /// cuanto haya crecido el archivo.
    ///
    /// # Errors
    ///
    /// [`FileError::Io`] si la lectura falla.
    pub fn next_frame(&mut self) -> FileResult<Option<Frame>> {
        if self.closed {
            return Ok(None);
        }

        let remaining = self.total_bytes.saturating_sub(self.sent_bytes);
        if remaining == 0 {
            return Ok(Some(self.close()));
        }

        let wanted = usize::try_from(remaining.min(CHUNK_SIZE_BYTES as u64))
            .unwrap_or(CHUNK_SIZE_BYTES)
            .min(CHUNK_SIZE_BYTES);
        let mut buffer = vec![0_u8; wanted];
        let read = self.fill(&mut buffer)?;

        if read == 0 {
            // El archivo se acorto mientras se leia. Se cierra con lo que de
            // verdad se envio: quien recibe vera que falta y lo rechazara.
            return Ok(Some(self.close()));
        }

        buffer.truncate(read);
        self.hasher.update(&buffer);
        self.sent_bytes = self.sent_bytes.saturating_add(read as u64);

        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        Ok(Some(Frame::Data {
            transfer_id: self.transfer_id,
            sequence,
            payload: buffer,
        }))
    }

    /// Rellena el buffer leyendo tantas veces como haga falta.
    ///
    /// `Read::read` puede devolver menos de lo pedido sin que sea el final del
    /// archivo. Dar por hecho lo contrario produciria trozos cortos y un
    /// archivo valido pero fragmentado de forma imprevisible.
    fn fill(&mut self, buffer: &mut [u8]) -> FileResult<usize> {
        let mut filled = 0;
        while filled < buffer.len() {
            match self.file.read(&mut buffer[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(FileError::Io(e.to_string())),
            }
        }
        Ok(filled)
    }

    fn close(&mut self) -> Frame {
        self.closed = true;
        let digest: [u8; DIGEST_BYTES] = self.hasher.clone().finalize().into();
        Frame::End {
            transfer_id: self.transfer_id,
            frames: self.next_sequence,
            digest,
        }
    }
}

/// Lo que ocurrio al aceptar un marco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted {
    /// Se escribieron bytes y la transferencia sigue.
    Progress {
        /// Bytes escritos en total hasta ahora.
        written_bytes: u64,
    },
    /// Termino bien: el archivo ya esta en su ruta definitiva.
    Completed {
        /// Ruta final.
        path: PathBuf,
        /// Tamano del archivo.
        total_bytes: u64,
    },
    /// Quien envia la abandono. No queda nada en el disco.
    Aborted,
}

/// Recibe los trozos de una subida y publica el archivo si todo cuadra.
#[derive(Debug)]
pub struct ChunkWriter {
    transfer_id: TransferId,
    file: Option<File>,
    temp_path: PathBuf,
    final_path: PathBuf,
    expected_bytes: u64,
    written_bytes: u64,
    next_sequence: u32,
    hasher: Sha256,
    settled: bool,
}

impl ChunkWriter {
    /// Prepara la escritura de una subida.
    ///
    /// La ruta tiene que venir de
    /// [`AccessPolicy::resolve`](crate::access::AccessPolicy::resolve) con
    /// acceso de escritura.
    ///
    /// # Errors
    ///
    /// - [`FileError::AlreadyExists`] si el destino ya existe. Sobrescribir no
    ///   se hace a la ligera: haria falta una confirmacion en el host, que
    ///   todavia no existe, asi que por ahora se rechaza y se dice por que.
    /// - [`FileError::WrongKind`] si el destino es un directorio o la ruta no
    ///   tiene nombre de archivo.
    /// - [`FileError::Io`] si no se puede crear la carpeta o el temporal.
    pub fn create(
        transfer_id: TransferId,
        final_path: &Path,
        expected_bytes: u64,
    ) -> FileResult<Self> {
        if final_path.exists() {
            return Err(FileError::AlreadyExists);
        }
        let parent = final_path.parent().ok_or(FileError::WrongKind {
            expected: "una ruta con carpeta contenedora",
        })?;
        if final_path.file_name().is_none() {
            return Err(FileError::WrongKind {
                expected: "una ruta con nombre de archivo",
            });
        }
        std::fs::create_dir_all(parent).map_err(|e| map_io(&e, parent))?;

        // El temporal va en la misma carpeta para que publicarlo sea un cambio
        // de nombre dentro del mismo sistema de archivos, y no una copia.
        // El identificador va completo, no abreviado: ocho digitos hexadecimales
        // son 32 bits, y dos transferencias que coincidieran darian un error de
        // disco confuso en lugar de nada. El nombre completo lo descarta.
        let temp_path = parent.join(format!(".{transfer_id}.nexus-part"));
        let file = OpenOptions::new()
            .write(true)
            // Falla si ya existe, en lugar de reutilizar un temporal ajeno.
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| map_io(&e, &temp_path))?;

        Ok(Self {
            transfer_id,
            file: Some(file),
            temp_path,
            final_path: final_path.to_path_buf(),
            expected_bytes,
            written_bytes: 0,
            next_sequence: 0,
            hasher: Sha256::new(),
            settled: false,
        })
    }

    /// Bytes escritos hasta ahora.
    #[must_use]
    pub const fn written_bytes(&self) -> u64 {
        self.written_bytes
    }

    /// Bytes anunciados por quien envia.
    #[must_use]
    pub const fn expected_bytes(&self) -> u64 {
        self.expected_bytes
    }

    /// Ruta del archivo temporal, mientras la transferencia esta en curso.
    #[must_use]
    pub fn temp_path(&self) -> &Path {
        &self.temp_path
    }

    /// Acepta un marco recibido por el canal de archivos.
    ///
    /// # Errors
    ///
    /// - [`FileError::UnknownTransfer`] si el marco es de otra transferencia.
    /// - [`FileError::TransferFinished`] si esta ya habia terminado.
    /// - [`FileError::OutOfOrder`] si el numero de secuencia no es el que toca.
    /// - [`FileError::TooLarge`] si los bytes superan lo anunciado.
    /// - [`FileError::Incomplete`] si al cerrar faltan bytes.
    /// - [`FileError::FrameCountMismatch`] si al cerrar faltan trozos.
    /// - [`FileError::IntegrityMismatch`] si el resumen no coincide.
    /// - [`FileError::AlreadyExists`] si algo ocupo el destino mientras tanto.
    /// - [`FileError::Io`] ante un fallo del disco.
    ///
    /// Cualquier error distinto de [`FileError::OutOfOrder`] deja la
    /// transferencia cerrada y borra el temporal: no se guarda un archivo a
    /// medias ni se deja basura.
    pub fn accept(&mut self, frame: &Frame) -> FileResult<Accepted> {
        if frame.transfer_id() != self.transfer_id {
            return Err(FileError::UnknownTransfer);
        }
        if self.settled {
            return Err(FileError::TransferFinished);
        }

        match frame {
            Frame::Data {
                sequence, payload, ..
            } => self.accept_data(*sequence, payload),
            Frame::End { frames, digest, .. } => {
                let outcome = self.publish(*frames, digest);
                if outcome.is_err() {
                    self.discard();
                }
                outcome
            }
            Frame::Abort { .. } => {
                self.discard();
                Ok(Accepted::Aborted)
            }
        }
    }

    fn accept_data(&mut self, sequence: u32, payload: &[u8]) -> FileResult<Accepted> {
        if sequence != self.next_sequence {
            // El canal es ordenado y fiable: un salto no es la red, es un
            // cliente que no respeta el protocolo. Se corta.
            let error = FileError::OutOfOrder {
                expected: self.next_sequence,
                received: sequence,
            };
            self.discard();
            return Err(error);
        }

        let next_total = self.written_bytes.saturating_add(payload.len() as u64);
        if next_total > self.expected_bytes {
            let error = FileError::TooLarge {
                size_bytes: next_total,
                max_bytes: self.expected_bytes,
            };
            self.discard();
            return Err(error);
        }

        let Some(file) = self.file.as_mut() else {
            return Err(FileError::TransferFinished);
        };
        if let Err(e) = file.write_all(payload) {
            let error = FileError::Io(e.to_string());
            self.discard();
            return Err(error);
        }

        self.hasher.update(payload);
        self.written_bytes = next_total;
        self.next_sequence = self.next_sequence.saturating_add(1);
        Ok(Accepted::Progress {
            written_bytes: self.written_bytes,
        })
    }

    /// Comprueba las tres cosas y, solo si las tres cuadran, publica.
    fn publish(&mut self, frames: u32, digest: &[u8; DIGEST_BYTES]) -> FileResult<Accepted> {
        if frames != self.next_sequence {
            return Err(FileError::FrameCountMismatch {
                expected: frames,
                received: self.next_sequence,
            });
        }
        if self.written_bytes != self.expected_bytes {
            return Err(FileError::Incomplete {
                expected: self.expected_bytes,
                received: self.written_bytes,
            });
        }

        let computed: [u8; DIGEST_BYTES] = self.hasher.clone().finalize().into();
        // Comparacion normal: ninguno de los dos lados es un secreto, asi que
        // no hay nada que filtrar por el tiempo que tarde.
        if &computed != digest {
            return Err(FileError::IntegrityMismatch);
        }

        // Los bytes al disco antes de que el archivo tenga su nombre final: si
        // el equipo se apaga justo ahora, lo que falte quedara en el temporal y
        // no en un destino que parece completo.
        let file = self.file.take().ok_or(FileError::TransferFinished)?;
        file.sync_all().map_err(|e| FileError::Io(e.to_string()))?;
        drop(file);

        // En Windows renombrar sobre un archivo existente falla, y en Unix lo
        // sustituye sin avisar. Se comprueba antes para que las dos plataformas
        // se comporten igual. Queda una ventana minima entre la comprobacion y
        // el cambio de nombre; cerrarla de verdad exigiria llamadas propias de
        // cada sistema, y no lo vale para lo que protege.
        if self.final_path.exists() {
            return Err(FileError::AlreadyExists);
        }
        std::fs::rename(&self.temp_path, &self.final_path)
            .map_err(|e| map_io(&e, &self.temp_path))?;

        self.settled = true;
        Ok(Accepted::Completed {
            path: self.final_path.clone(),
            total_bytes: self.written_bytes,
        })
    }

    /// Abandona la transferencia y borra el temporal.
    pub fn abort(&mut self) {
        self.discard();
    }

    fn discard(&mut self) {
        if self.settled {
            return;
        }
        self.settled = true;
        drop(self.file.take());
        if let Err(error) = std::fs::remove_file(&self.temp_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                ruta = %self.temp_path.display(),
                %error,
                "no se pudo borrar el archivo temporal de una transferencia"
            );
        }
    }
}

impl Drop for ChunkWriter {
    /// Si nadie cerro la transferencia, el temporal no se queda en el disco.
    ///
    /// Una sesion que se cae a mitad de una subida es lo normal, no la
    /// excepcion: sin esto, la carpeta compartida se iria llenando de restos.
    fn drop(&mut self) {
        self.discard();
    }
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

    fn digest_of(bytes: &[u8]) -> [u8; DIGEST_BYTES] {
        Sha256::digest(bytes).into()
    }

    /// Lee un archivo entero a marcos.
    fn drain(reader: &mut ChunkReader) -> Vec<Frame> {
        let mut frames = Vec::new();
        while let Some(frame) = reader.next_frame().expect("lee") {
            frames.push(frame);
        }
        frames
    }

    /// Mueve un archivo de origen a destino por el camino completo: marcos
    /// serializados, analizados y aceptados. Es la prueba de que las dos
    /// mitades encajan de verdad.
    fn transfer(source: &Path, destination: &Path) -> FileResult<Accepted> {
        let id = TransferId::new();
        let mut reader = ChunkReader::open(id, source).expect("abre origen");
        let mut writer = ChunkWriter::create(id, destination, reader.total_bytes())?;

        let mut last = Accepted::Progress { written_bytes: 0 };
        for frame in drain(&mut reader) {
            let raw = frame.encode();
            let parsed = Frame::parse(&raw).expect("el marco propio se analiza");
            last = writer.accept(&parsed)?;
        }
        Ok(last)
    }

    #[test]
    fn a_small_file_arrives_byte_for_byte() {
        let temp = tempfile::tempdir().expect("temporal");
        let source = temp.path().join("origen.txt");
        let destination = temp.path().join("destino.txt");
        let content = b"NEXUS REMOTE: contenido de prueba.".to_vec();
        std::fs::write(&source, &content).expect("escribe");

        let outcome = transfer(&source, &destination).expect("transfiere");
        assert_eq!(
            outcome,
            Accepted::Completed {
                path: destination.clone(),
                total_bytes: content.len() as u64
            }
        );
        assert_eq!(std::fs::read(&destination).expect("lee"), content);
    }

    #[test]
    fn a_file_larger_than_one_chunk_arrives_byte_for_byte() {
        let temp = tempfile::tempdir().expect("temporal");
        let source = temp.path().join("grande.bin");
        let destination = temp.path().join("copia.bin");

        // Dos trozos y medio, con un patron que delataria bytes mal colocados.
        let content: Vec<u8> = (0..CHUNK_SIZE_BYTES * 2 + 1_234)
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(&source, &content).expect("escribe");

        transfer(&source, &destination).expect("transfiere");
        assert_eq!(std::fs::read(&destination).expect("lee"), content);
    }

    #[test]
    fn an_empty_file_arrives_as_an_empty_file() {
        let temp = tempfile::tempdir().expect("temporal");
        let source = temp.path().join("vacio.txt");
        let destination = temp.path().join("vacio-copia.txt");
        std::fs::write(&source, b"").expect("escribe");

        let outcome = transfer(&source, &destination).expect("transfiere");
        assert_eq!(
            outcome,
            Accepted::Completed {
                path: destination.clone(),
                total_bytes: 0
            }
        );
        assert!(destination.exists());
        assert_eq!(std::fs::read(&destination).expect("lee"), Vec::<u8>::new());
    }

    #[test]
    fn the_reader_splits_the_file_at_the_chunk_size() {
        let temp = tempfile::tempdir().expect("temporal");
        let source = temp.path().join("a.bin");
        std::fs::write(&source, vec![1_u8; CHUNK_SIZE_BYTES + 10]).expect("escribe");

        let mut reader = ChunkReader::open(TransferId::new(), &source).expect("abre");
        assert_eq!(reader.total_bytes(), CHUNK_SIZE_BYTES as u64 + 10);
        let frames = drain(&mut reader);

        assert_eq!(frames.len(), 3, "dos de datos y uno de fin");
        match &frames[0] {
            Frame::Data {
                sequence, payload, ..
            } => {
                assert_eq!(*sequence, 0);
                assert_eq!(payload.len(), CHUNK_SIZE_BYTES);
            }
            other => panic!("se esperaba datos: {other:?}"),
        }
        match &frames[1] {
            Frame::Data {
                sequence, payload, ..
            } => {
                assert_eq!(*sequence, 1);
                assert_eq!(payload.len(), 10);
            }
            other => panic!("se esperaba datos: {other:?}"),
        }
        match &frames[2] {
            Frame::End { frames: n, .. } => assert_eq!(*n, 2),
            other => panic!("se esperaba fin: {other:?}"),
        }
    }

    #[test]
    fn the_reader_announces_the_digest_of_what_it_sent() {
        let temp = tempfile::tempdir().expect("temporal");
        let source = temp.path().join("a.bin");
        let content = b"abcdefghij".to_vec();
        std::fs::write(&source, &content).expect("escribe");

        let mut reader = ChunkReader::open(TransferId::new(), &source).expect("abre");
        let frames = drain(&mut reader);
        let Some(Frame::End { digest, .. }) = frames.last() else {
            panic!("el ultimo marco deberia ser el de fin");
        };
        assert_eq!(*digest, digest_of(&content));
        assert_eq!(reader.sent_bytes(), content.len() as u64);
    }

    #[test]
    fn the_reader_stops_after_the_end_frame() {
        let temp = tempfile::tempdir().expect("temporal");
        let source = temp.path().join("a.bin");
        std::fs::write(&source, b"x").expect("escribe");

        let mut reader = ChunkReader::open(TransferId::new(), &source).expect("abre");
        drain(&mut reader);
        assert!(reader.next_frame().expect("no falla").is_none());
        assert!(reader.next_frame().expect("sigue sin fallar").is_none());
    }

    #[test]
    fn a_directory_cannot_be_read_as_a_file() {
        let temp = tempfile::tempdir().expect("temporal");
        assert_eq!(
            ChunkReader::open(TransferId::new(), temp.path()).unwrap_err(),
            FileError::WrongKind {
                expected: "un archivo"
            }
        );
    }

    #[test]
    fn a_missing_file_is_reported_as_not_found() {
        let temp = tempfile::tempdir().expect("temporal");
        assert_eq!(
            ChunkReader::open(TransferId::new(), &temp.path().join("no-existe")).unwrap_err(),
            FileError::NotFound
        );
    }

    // --- Lo que hace el receptor cuando algo no cuadra -------------------

    /// Prepara una subida de `content` lista para manipular los marcos.
    struct Upload {
        _temp: tempfile::TempDir,
        destination: PathBuf,
        id: TransferId,
        writer: ChunkWriter,
        content: Vec<u8>,
    }

    fn upload(content: &[u8]) -> Upload {
        let temp = tempfile::tempdir().expect("temporal");
        let destination = temp.path().join("subido.bin");
        let id = TransferId::new();
        let writer =
            ChunkWriter::create(id, &destination, content.len() as u64).expect("prepara escritura");
        Upload {
            _temp: temp,
            destination,
            id,
            writer,
            content: content.to_vec(),
        }
    }

    fn data_frame(id: TransferId, sequence: u32, payload: &[u8]) -> Frame {
        Frame::Data {
            transfer_id: id,
            sequence,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn a_tampered_digest_keeps_the_file_out_of_its_destination() {
        let mut u = upload(b"contenido autentico");
        u.writer
            .accept(&data_frame(u.id, 0, &u.content))
            .expect("acepta datos");

        let err = u
            .writer
            .accept(&Frame::End {
                transfer_id: u.id,
                frames: 1,
                digest: [0_u8; DIGEST_BYTES],
            })
            .unwrap_err();

        assert_eq!(err, FileError::IntegrityMismatch);
        assert!(
            !u.destination.exists(),
            "el destino no debe existir si el resumen no cuadra"
        );
        assert!(
            !u.writer.temp_path().exists(),
            "el temporal tampoco debe quedarse"
        );
    }

    #[test]
    fn a_missing_chunk_is_caught_by_the_frame_count() {
        let mut u = upload(b"0123456789");
        u.writer
            .accept(&data_frame(u.id, 0, &u.content))
            .expect("acepta");

        // Quien envia dice haber mandado dos trozos, pero solo mando uno.
        let err = u
            .writer
            .accept(&Frame::End {
                transfer_id: u.id,
                frames: 2,
                digest: digest_of(&u.content),
            })
            .unwrap_err();

        assert_eq!(
            err,
            FileError::FrameCountMismatch {
                expected: 2,
                received: 1
            }
        );
        assert!(!u.destination.exists());
    }

    #[test]
    fn a_short_upload_is_caught_even_with_a_matching_digest() {
        let mut u = upload(b"diez bytes");
        let half = &u.content[..4];
        // Se anuncio el total completo pero solo se mandan cuatro bytes, con
        // su resumen correcto: el engano no pasa la cuenta de bytes.
        u.writer.accept(&data_frame(u.id, 0, half)).expect("acepta");
        let err = u
            .writer
            .accept(&Frame::End {
                transfer_id: u.id,
                frames: 1,
                digest: digest_of(half),
            })
            .unwrap_err();

        assert_eq!(
            err,
            FileError::Incomplete {
                expected: u.content.len() as u64,
                received: 4
            }
        );
        assert!(!u.destination.exists());
    }

    #[test]
    fn a_chunk_out_of_order_ends_the_transfer() {
        let mut u = upload(b"0123456789");
        let err = u.writer.accept(&data_frame(u.id, 1, b"xx")).unwrap_err();
        assert_eq!(
            err,
            FileError::OutOfOrder {
                expected: 0,
                received: 1
            }
        );
        assert!(!u.writer.temp_path().exists());
        // Y no se puede seguir como si nada.
        assert_eq!(
            u.writer.accept(&data_frame(u.id, 0, b"xx")).unwrap_err(),
            FileError::TransferFinished
        );
    }

    #[test]
    fn a_repeated_chunk_is_refused() {
        let mut u = upload(b"0123456789");
        u.writer
            .accept(&data_frame(u.id, 0, b"01234"))
            .expect("acepta");
        let err = u.writer.accept(&data_frame(u.id, 0, b"01234")).unwrap_err();
        assert_eq!(
            err,
            FileError::OutOfOrder {
                expected: 1,
                received: 0
            }
        );
    }

    #[test]
    fn more_bytes_than_announced_are_refused() {
        let mut u = upload(b"corto");
        let err = u
            .writer
            .accept(&data_frame(u.id, 0, b"mucho mas largo de lo anunciado"))
            .unwrap_err();
        assert_eq!(
            err,
            FileError::TooLarge {
                size_bytes: 31,
                max_bytes: 5
            }
        );
        assert!(!u.writer.temp_path().exists());
    }

    #[test]
    fn a_frame_from_another_transfer_is_refused() {
        let mut u = upload(b"x");
        let err = u
            .writer
            .accept(&data_frame(TransferId::new(), 0, b"x"))
            .unwrap_err();
        assert_eq!(err, FileError::UnknownTransfer);
        // Y no estropea la transferencia legitima.
        u.writer
            .accept(&data_frame(u.id, 0, b"x"))
            .expect("la propia sigue valiendo");
    }

    #[test]
    fn an_abort_removes_the_partial_file() {
        let mut u = upload(b"0123456789");
        u.writer
            .accept(&data_frame(u.id, 0, b"01234"))
            .expect("acepta");
        assert!(u.writer.temp_path().exists());

        let outcome = u
            .writer
            .accept(&Frame::Abort { transfer_id: u.id })
            .expect("acepta el aborto");
        assert_eq!(outcome, Accepted::Aborted);
        assert!(!u.writer.temp_path().exists());
        assert!(!u.destination.exists());
    }

    #[test]
    fn dropping_a_writer_mid_transfer_leaves_nothing_behind() {
        let temp = tempfile::tempdir().expect("temporal");
        let destination = temp.path().join("a-medias.bin");
        let id = TransferId::new();
        let temp_path;
        {
            let mut writer = ChunkWriter::create(id, &destination, 100).expect("prepara");
            temp_path = writer.temp_path().to_path_buf();
            writer
                .accept(&data_frame(id, 0, b"inicio"))
                .expect("acepta");
            assert!(temp_path.exists());
        }
        assert!(
            !temp_path.exists(),
            "una sesion cortada no debe dejar restos en la carpeta compartida"
        );
        assert!(!destination.exists());
        // La carpeta compartida queda como estaba.
        let remaining: Vec<_> = std::fs::read_dir(temp.path())
            .expect("lee")
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert!(remaining.is_empty(), "quedaron restos: {remaining:?}");
    }

    #[test]
    fn the_destination_is_never_written_before_it_is_verified() {
        let mut u = upload(b"0123456789");
        u.writer
            .accept(&data_frame(u.id, 0, b"01234"))
            .expect("acepta");
        assert!(
            !u.destination.exists(),
            "a mitad de la subida el destino no existe todavia"
        );
        assert!(u.writer.temp_path().exists());
        assert_eq!(u.writer.written_bytes(), 5);
        assert_eq!(u.writer.expected_bytes(), 10);
    }

    #[test]
    fn an_existing_destination_is_not_overwritten() {
        let temp = tempfile::tempdir().expect("temporal");
        let destination = temp.path().join("ya-esta.txt");
        std::fs::write(&destination, b"lo que ya habia").expect("escribe");

        assert_eq!(
            ChunkWriter::create(TransferId::new(), &destination, 5).unwrap_err(),
            FileError::AlreadyExists
        );
        assert_eq!(
            std::fs::read(&destination).expect("lee"),
            b"lo que ya habia",
            "el archivo existente queda intacto"
        );
    }

    #[test]
    fn a_destination_that_appears_mid_transfer_does_not_get_clobbered() {
        let mut u = upload(b"nuevo contenido");
        u.writer
            .accept(&data_frame(u.id, 0, &u.content))
            .expect("acepta");

        // Alguien crea el archivo mientras la subida estaba en marcha.
        std::fs::write(&u.destination, b"creado mientras tanto").expect("escribe");

        let err = u
            .writer
            .accept(&Frame::End {
                transfer_id: u.id,
                frames: 1,
                digest: digest_of(&u.content),
            })
            .unwrap_err();
        assert_eq!(err, FileError::AlreadyExists);
        assert_eq!(
            std::fs::read(&u.destination).expect("lee"),
            b"creado mientras tanto"
        );
        assert!(!u.writer.temp_path().exists());
    }

    #[test]
    fn missing_parent_directories_are_created() {
        let temp = tempfile::tempdir().expect("temporal");
        let destination = temp.path().join("uno/dos/tres.txt");
        let id = TransferId::new();
        let mut writer = ChunkWriter::create(id, &destination, 3).expect("prepara");
        writer.accept(&data_frame(id, 0, b"abc")).expect("acepta");
        writer
            .accept(&Frame::End {
                transfer_id: id,
                frames: 1,
                digest: digest_of(b"abc"),
            })
            .expect("completa");
        assert_eq!(std::fs::read(&destination).expect("lee"), b"abc");
    }

    #[test]
    fn two_concurrent_uploads_to_the_same_folder_use_different_temporaries() {
        let temp = tempfile::tempdir().expect("temporal");
        let a = ChunkWriter::create(TransferId::new(), &temp.path().join("a.bin"), 1)
            .expect("prepara a");
        let b = ChunkWriter::create(TransferId::new(), &temp.path().join("b.bin"), 1)
            .expect("prepara b");
        assert_ne!(a.temp_path(), b.temp_path());
    }

    #[test]
    fn nothing_can_be_accepted_after_the_transfer_completed() {
        let mut u = upload(b"abc");
        u.writer
            .accept(&data_frame(u.id, 0, b"abc"))
            .expect("acepta");
        u.writer
            .accept(&Frame::End {
                transfer_id: u.id,
                frames: 1,
                digest: digest_of(b"abc"),
            })
            .expect("completa");
        assert_eq!(
            u.writer.accept(&data_frame(u.id, 1, b"mas")).unwrap_err(),
            FileError::TransferFinished
        );
        assert_eq!(std::fs::read(&u.destination).expect("lee"), b"abc");
    }

    #[test]
    fn aborting_twice_is_harmless() {
        let mut u = upload(b"abc");
        u.writer.abort();
        u.writer.abort();
        assert!(!u.writer.temp_path().exists());
    }
}
