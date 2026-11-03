//! Contabilidad de transferencias en curso.
//!
//! Lleva el progreso, permite cancelar y acota cuantas transferencias pueden
//! estar activas a la vez. No mueve bytes: eso corresponde al canal de datos y
//! es trabajo de la etapa 5. Lo que si esta aqui, y probado, es la maquina de
//! estados que evita progresos imposibles y transferencias zombis.

use std::collections::HashMap;
use std::path::PathBuf;

use nexus_core::protocol::control::TransferState;
use nexus_core::{Timestamp, TransferId};

use crate::error::{FileError, FileResult};

/// Tamano de trozo con el que se parte un archivo.
///
/// 64 KiB encaja holgadamente en el limite practico de un mensaje del canal de
/// datos de WebRTC y deja margen para la cabecera del propio protocolo.
pub const CHUNK_SIZE_BYTES: usize = 64 * 1024;

/// Tamano maximo de archivo admitido por omision (4 GiB).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Sentido de la transferencia visto desde el host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// El host envia: el dispositivo descarga.
    Download,
    /// El host recibe: el dispositivo sube.
    Upload,
}

/// Transferencia individual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    /// Identificador asignado por el host.
    pub id: TransferId,
    /// Sentido.
    pub direction: Direction,
    /// Ruta ya resuelta y comprobada contra la politica de acceso.
    pub path: PathBuf,
    /// Tamano total, si se conoce.
    pub total_bytes: Option<u64>,
    /// Bytes transferidos hasta ahora.
    pub transferred_bytes: u64,
    /// Estado actual.
    pub state: TransferState,
    /// Instante de inicio.
    pub started_at: Timestamp,
    /// Instante de la ultima actualizacion.
    pub updated_at: Timestamp,
}

impl Transfer {
    /// Porcentaje completado, si se conoce el total.
    #[must_use]
    pub fn progress_percent(&self) -> Option<u8> {
        let total = self.total_bytes?;
        if total == 0 {
            return Some(100);
        }
        let ratio = self.transferred_bytes.saturating_mul(100) / total;
        Some(ratio.min(100) as u8)
    }

    /// Indica si la transferencia sigue viva.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !self.state.is_final()
    }
}

/// Registro de las transferencias de una sesion.
#[derive(Debug)]
pub struct TransferTracker {
    transfers: HashMap<TransferId, Transfer>,
    max_concurrent: usize,
    max_file_bytes: u64,
}

impl TransferTracker {
    /// Transferencias simultaneas admitidas por omision.
    ///
    /// Acota el trabajo que un dispositivo puede pedir a la vez y mantiene
    /// previsible el reparto de ancho de banda con el video.
    pub const DEFAULT_MAX_CONCURRENT: usize = 4;

    /// Crea un registro con los limites por omision.
    #[must_use]
    pub fn new() -> Self {
        Self {
            transfers: HashMap::new(),
            max_concurrent: Self::DEFAULT_MAX_CONCURRENT,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }

    /// Crea un registro con limites a medida.
    #[must_use]
    pub fn with_limits(max_concurrent: usize, max_file_bytes: u64) -> Self {
        Self {
            transfers: HashMap::new(),
            max_concurrent,
            max_file_bytes,
        }
    }

    /// Abre una transferencia.
    ///
    /// # Errors
    ///
    /// - [`FileError::TooManyTransfers`] si ya hay el maximo de activas.
    /// - [`FileError::TooLarge`] si el tamano anunciado supera el limite.
    pub fn start(
        &mut self,
        direction: Direction,
        path: PathBuf,
        total_bytes: Option<u64>,
        now: Timestamp,
    ) -> FileResult<Transfer> {
        if let Some(size) = total_bytes
            && size > self.max_file_bytes
        {
            return Err(FileError::TooLarge {
                size_bytes: size,
                max_bytes: self.max_file_bytes,
            });
        }

        self.forget_finished();
        if self.active_count() >= self.max_concurrent {
            return Err(FileError::TooManyTransfers {
                max: self.max_concurrent,
            });
        }

        let transfer = Transfer {
            id: TransferId::new(),
            direction,
            path,
            total_bytes,
            transferred_bytes: 0,
            state: TransferState::Pending,
            started_at: now,
            updated_at: now,
        };
        self.transfers.insert(transfer.id, transfer.clone());
        Ok(transfer)
    }

    /// Anota bytes transferidos y devuelve el estado actualizado.
    ///
    /// Si con estos bytes se alcanza el total conocido, la transferencia pasa a
    /// [`TransferState::Completed`] automaticamente.
    ///
    /// # Errors
    ///
    /// - [`FileError::UnknownTransfer`] si el identificador no existe.
    /// - [`FileError::TransferFinished`] si ya habia terminado.
    /// - [`FileError::TooLarge`] si los bytes recibidos superan el total
    ///   anunciado, lo que indica un cliente que no respeta su propio anuncio.
    pub fn advance(&mut self, id: TransferId, bytes: u64, now: Timestamp) -> FileResult<Transfer> {
        let max_file_bytes = self.max_file_bytes;
        let transfer = self
            .transfers
            .get_mut(&id)
            .ok_or(FileError::UnknownTransfer)?;
        if !transfer.is_active() {
            return Err(FileError::TransferFinished);
        }

        let next = transfer.transferred_bytes.saturating_add(bytes);
        if let Some(total) = transfer.total_bytes
            && next > total
        {
            transfer.state = TransferState::Failed;
            transfer.updated_at = now;
            return Err(FileError::TooLarge {
                size_bytes: next,
                max_bytes: total,
            });
        }
        if next > max_file_bytes {
            transfer.state = TransferState::Failed;
            transfer.updated_at = now;
            return Err(FileError::TooLarge {
                size_bytes: next,
                max_bytes: max_file_bytes,
            });
        }

        transfer.transferred_bytes = next;
        transfer.updated_at = now;
        transfer.state = match transfer.total_bytes {
            Some(total) if next >= total => TransferState::Completed,
            _ => TransferState::Active,
        };
        Ok(transfer.clone())
    }

    /// Cancela una transferencia activa.
    ///
    /// # Errors
    ///
    /// - [`FileError::UnknownTransfer`] si el identificador no existe.
    /// - [`FileError::TransferFinished`] si ya habia terminado.
    pub fn cancel(&mut self, id: TransferId, now: Timestamp) -> FileResult<Transfer> {
        let transfer = self
            .transfers
            .get_mut(&id)
            .ok_or(FileError::UnknownTransfer)?;
        if !transfer.is_active() {
            return Err(FileError::TransferFinished);
        }
        transfer.state = TransferState::Cancelled;
        transfer.updated_at = now;
        Ok(transfer.clone())
    }

    /// Marca una transferencia como fallida.
    ///
    /// # Errors
    ///
    /// [`FileError::UnknownTransfer`] si el identificador no existe.
    pub fn fail(&mut self, id: TransferId, now: Timestamp) -> FileResult<Transfer> {
        let transfer = self
            .transfers
            .get_mut(&id)
            .ok_or(FileError::UnknownTransfer)?;
        transfer.state = TransferState::Failed;
        transfer.updated_at = now;
        Ok(transfer.clone())
    }

    /// Consulta una transferencia.
    #[must_use]
    pub fn get(&self, id: TransferId) -> Option<&Transfer> {
        self.transfers.get(&id)
    }

    /// Numero de transferencias activas.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.transfers.values().filter(|t| t.is_active()).count()
    }

    /// Transferencias activas, en orden estable por identificador.
    #[must_use]
    pub fn active(&self) -> Vec<Transfer> {
        let mut out: Vec<_> = self
            .transfers
            .values()
            .filter(|t| t.is_active())
            .cloned()
            .collect();
        out.sort_by_key(|t| t.id.to_string());
        out
    }

    /// Olvida las transferencias terminadas.
    pub fn forget_finished(&mut self) {
        self.transfers.retain(|_, t| t.is_active());
    }

    /// Numero de trozos necesarios para un tamano dado.
    #[must_use]
    pub const fn chunk_count(size_bytes: u64) -> u64 {
        size_bytes.div_ceil(CHUNK_SIZE_BYTES as u64)
    }
}

impl Default for TransferTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp(1_000);

    fn tracker() -> TransferTracker {
        TransferTracker::with_limits(2, 1_000)
    }

    fn start(tracker: &mut TransferTracker, total: Option<u64>) -> Transfer {
        tracker
            .start(
                Direction::Download,
                PathBuf::from("/compartido/a.bin"),
                total,
                NOW,
            )
            .expect("abre transferencia")
    }

    #[test]
    fn a_new_transfer_starts_pending_with_no_progress() {
        let mut t = tracker();
        let transfer = start(&mut t, Some(500));
        assert_eq!(transfer.state, TransferState::Pending);
        assert_eq!(transfer.transferred_bytes, 0);
        assert_eq!(transfer.progress_percent(), Some(0));
        assert_eq!(t.active_count(), 1);
    }

    #[test]
    fn progress_moves_the_transfer_to_active() {
        let mut t = tracker();
        let transfer = start(&mut t, Some(500));
        let updated = t
            .advance(transfer.id, 250, Timestamp(2_000))
            .expect("avanza");
        assert_eq!(updated.state, TransferState::Active);
        assert_eq!(updated.progress_percent(), Some(50));
        assert_eq!(updated.updated_at, Timestamp(2_000));
    }

    #[test]
    fn reaching_the_total_completes_the_transfer() {
        let mut t = tracker();
        let transfer = start(&mut t, Some(500));
        t.advance(transfer.id, 250, NOW).expect("avanza");
        let done = t.advance(transfer.id, 250, NOW).expect("completa");
        assert_eq!(done.state, TransferState::Completed);
        assert_eq!(done.progress_percent(), Some(100));
        assert!(!done.is_active());
        assert_eq!(t.active_count(), 0);
    }

    #[test]
    fn sending_more_bytes_than_announced_fails_the_transfer() {
        let mut t = tracker();
        let transfer = start(&mut t, Some(500));
        let err = t.advance(transfer.id, 501, NOW).unwrap_err();
        assert_eq!(
            err,
            FileError::TooLarge {
                size_bytes: 501,
                max_bytes: 500
            }
        );
        assert_eq!(
            t.get(transfer.id).expect("existe").state,
            TransferState::Failed
        );
    }

    #[test]
    fn a_transfer_without_a_known_total_never_auto_completes() {
        let mut t = tracker();
        let transfer = start(&mut t, None);
        let updated = t.advance(transfer.id, 999, NOW).expect("avanza");
        assert_eq!(updated.state, TransferState::Active);
        assert_eq!(updated.progress_percent(), None);
    }

    #[test]
    fn an_unbounded_transfer_still_respects_the_global_limit() {
        let mut t = tracker();
        let transfer = start(&mut t, None);
        let err = t.advance(transfer.id, 1_001, NOW).unwrap_err();
        assert_eq!(
            err,
            FileError::TooLarge {
                size_bytes: 1_001,
                max_bytes: 1_000
            }
        );
    }

    #[test]
    fn an_oversized_file_is_refused_before_starting() {
        let mut t = tracker();
        let err = t
            .start(
                Direction::Upload,
                PathBuf::from("/compartido/grande.iso"),
                Some(2_000),
                NOW,
            )
            .unwrap_err();
        assert_eq!(
            err,
            FileError::TooLarge {
                size_bytes: 2_000,
                max_bytes: 1_000
            }
        );
        assert_eq!(t.active_count(), 0);
    }

    #[test]
    fn cancelling_stops_the_transfer() {
        let mut t = tracker();
        let transfer = start(&mut t, Some(500));
        let cancelled = t.cancel(transfer.id, Timestamp(5_000)).expect("cancela");
        assert_eq!(cancelled.state, TransferState::Cancelled);
        assert_eq!(cancelled.updated_at, Timestamp(5_000));
    }

    #[test]
    fn a_cancelled_transfer_accepts_no_more_bytes() {
        let mut t = tracker();
        let transfer = start(&mut t, Some(500));
        t.cancel(transfer.id, NOW).expect("cancela");
        assert_eq!(
            t.advance(transfer.id, 1, NOW).unwrap_err(),
            FileError::TransferFinished
        );
    }

    #[test]
    fn cancelling_twice_is_reported() {
        let mut t = tracker();
        let transfer = start(&mut t, Some(500));
        t.cancel(transfer.id, NOW).expect("cancela");
        assert_eq!(
            t.cancel(transfer.id, NOW).unwrap_err(),
            FileError::TransferFinished
        );
    }

    #[test]
    fn unknown_transfers_are_reported() {
        let mut t = tracker();
        let ghost = TransferId::new();
        assert_eq!(
            t.advance(ghost, 1, NOW).unwrap_err(),
            FileError::UnknownTransfer
        );
        assert_eq!(
            t.cancel(ghost, NOW).unwrap_err(),
            FileError::UnknownTransfer
        );
        assert_eq!(t.fail(ghost, NOW).unwrap_err(), FileError::UnknownTransfer);
        assert!(t.get(ghost).is_none());
    }

    #[test]
    fn concurrent_transfers_are_capped() {
        let mut t = tracker();
        start(&mut t, Some(100));
        start(&mut t, Some(100));
        let err = t
            .start(
                Direction::Download,
                PathBuf::from("/compartido/c.bin"),
                Some(100),
                NOW,
            )
            .unwrap_err();
        assert_eq!(err, FileError::TooManyTransfers { max: 2 });
    }

    #[test]
    fn finishing_a_transfer_frees_a_slot() {
        let mut t = tracker();
        let first = start(&mut t, Some(100));
        start(&mut t, Some(100));

        t.cancel(first.id, NOW).expect("cancela");
        t.start(
            Direction::Download,
            PathBuf::from("/compartido/c.bin"),
            Some(100),
            NOW,
        )
        .expect("ahora cabe");
        assert_eq!(t.active_count(), 2);
    }

    #[test]
    fn active_listing_is_deterministic() {
        let mut t = TransferTracker::with_limits(8, 10_000);
        for _ in 0..5 {
            start(&mut t, Some(100));
        }
        assert_eq!(t.active().len(), 5);
        assert_eq!(t.active(), t.active());
    }

    #[test]
    fn zero_byte_files_report_full_progress() {
        let mut t = tracker();
        let transfer = start(&mut t, Some(0));
        assert_eq!(transfer.progress_percent(), Some(100));
    }

    #[test]
    fn chunk_count_rounds_up() {
        assert_eq!(TransferTracker::chunk_count(0), 0);
        assert_eq!(TransferTracker::chunk_count(1), 1);
        assert_eq!(TransferTracker::chunk_count(CHUNK_SIZE_BYTES as u64), 1);
        assert_eq!(TransferTracker::chunk_count(CHUNK_SIZE_BYTES as u64 + 1), 2);
    }

    #[test]
    fn default_limits_are_sane() {
        let t = TransferTracker::new();
        assert_eq!(t.active_count(), 0);
        assert_eq!(TransferTracker::DEFAULT_MAX_CONCURRENT, 4);
        assert_eq!(DEFAULT_MAX_FILE_BYTES, 4 * 1024 * 1024 * 1024);
    }
}
