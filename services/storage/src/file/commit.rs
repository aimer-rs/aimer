//! Atomic batches on disk: commit, deferred sync, and crash recovery.
//!
//! The lifecycle of one batch, and what a crash at each point leaves behind:
//!
//! 1. **Journal.** The whole batch is written to the journal, synced, and
//!    renamed into place ([`journal::commit`]). *Crash before the rename: the
//!    store is untouched. After it: the batch is committed.* The caller's
//!    future resolves here, so a batch costs a fixed two syncs for the caller
//!    however many values it holds.
//! 2. **Apply.** Each value is written to its own file with no per-file sync.
//!    *Crash: recovery replays the journal over whatever landed.*
//! 3. **Checkpoint.** The files are synced and the journal is retired. This runs
//!    on the worker, after the caller has been woken, as soon as the queue is
//!    idle, and before any later *mutating* operation, so a stale journal can
//!    never be replayed over a newer write. *Crash: the journal is still there,
//!    and replaying it again is harmless.*
//!
//! Every operation first calls [`recover`], so a store never serves a
//! half-applied batch left by a crash, not even to a read.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use super::journal::{self, JournalOp};
use super::layout::{native_error, stat_value, value_path, write_atomically, write_unsynced};
use super::worker::{Inner, lock};
use crate::batch::{FinalOp, StorageBatch, new_lens, plan};
use crate::error::{StorageOperation, StorageResult};
use crate::limits::Usage;

const OPERATION: StorageOperation = StorageOperation::Write;

/// The usage a quota decision starts from, scanning the store the first time.
///
/// `slot` is the cache held in [`Inner::usage`]; leaving it `None` after an
/// error means the next operation scans again.
pub(super) fn current_usage<'a>(
    slot: &'a mut Option<Usage>,
    inner: &Inner,
    operation: StorageOperation,
) -> StorageResult<&'a mut Usage> {
    if slot.is_none() {
        *slot = Some(super::layout::scan_disk(&inner.root, operation)?);
    }
    slot.as_mut()
        .ok_or(crate::error::StorageError::Corrupt { operation })
}

/// Commits `batch`: when this returns `Ok` it is durable and will be completed
/// by recovery even if the process dies before its files are synced.
pub(super) fn commit_batch(inner: &Inner, batch: &StorageBatch) -> StorageResult<()> {
    let ops = batch.coalesce();
    if ops.is_empty() {
        return Ok(());
    }

    let mut usage_slot = lock(&inner.usage);
    let usage = *current_usage(&mut usage_slot, inner, OPERATION)?;
    let paths: Vec<PathBuf> = ops
        .iter()
        .map(|op| value_path(&inner.root, op.namespace, op.key))
        .collect();
    let old_lens = paths
        .iter()
        .map(|path| stat_value(path, OPERATION))
        .collect::<StorageResult<Vec<_>>>()?;
    // Every rejection happens here, before anything is written.
    let after = plan(inner.limits, usage, &new_lens(&ops), &old_lens)?;

    if let Err(error) = journal::commit(&inner.root, &journal::encode(&ops)) {
        // Not committed, or committed and unreported; recovery settles which.
        *usage_slot = None;
        inner.recovered.store(false, Ordering::Release);
        return Err(native_error(OPERATION, error));
    }

    if let Err(error) = apply_unsynced(&ops, &paths) {
        // Committed but not fully applied: the next operation completes it.
        *usage_slot = None;
        inner.recovered.store(false, Ordering::Release);
        return Err(native_error(OPERATION, error));
    }

    *usage_slot = Some(after);
    // Removals are included: their directories need the sync too.
    *lock(&inner.deferred) = Some(paths);
    Ok(())
}

/// Writes the batch's values and removals with no per-file sync; the journal
/// already holds them durably.
fn apply_unsynced(ops: &[FinalOp<'_>], paths: &[PathBuf]) -> io::Result<()> {
    for (op, path) in ops.iter().zip(paths) {
        match op.value {
            Some(value) => write_unsynced(path, value)?,
            None => match fs::remove_file(path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            },
        }
    }
    Ok(())
}

/// Syncs what the last batch wrote and retires its journal.
///
/// A no-op when nothing is outstanding. On failure the work stays outstanding
/// and the journal stays in place, so it is retried and, if the process dies
/// first, replayed.
pub(super) fn checkpoint(inner: &Inner) -> StorageResult<()> {
    let Some(paths) = lock(&inner.deferred).take() else {
        return Ok(());
    };
    let result = sync_and_retire(inner, &paths);
    if result.is_err() {
        *lock(&inner.deferred) = Some(paths);
    }
    result.map_err(|error| native_error(OPERATION, error))
}

/// Opens a value file so that `sync_all` on it can succeed.
///
/// Windows implements `sync_all` with `FlushFileBuffers`, which needs write
/// access: on a read-only handle it fails with `PermissionDenied`, and the
/// checkpoint would then never retire the journal. Opening for write neither
/// truncates nor changes the file.
fn open_for_sync(path: &std::path::Path) -> io::Result<fs::File> {
    #[cfg(windows)]
    {
        fs::OpenOptions::new().write(true).open(path)
    }
    #[cfg(not(windows))]
    {
        fs::File::open(path)
    }
}

fn sync_and_retire(inner: &Inner, paths: &[PathBuf]) -> io::Result<()> {
    let mut directories: Vec<&std::path::Path> = Vec::new();
    for path in paths {
        match open_for_sync(path) {
            Ok(file) => file.sync_all()?,
            // Removed by the batch (or replaced since): nothing to sync.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if let Some(parent) = path.parent()
            && !directories.contains(&parent)
        {
            directories.push(parent);
        }
    }
    for directory in directories {
        journal::sync_directory(directory);
    }
    journal::remove(&inner.root)
}

/// Completes a committed batch left behind by a crash, once per store handle.
///
/// # Errors
///
/// A journal that fails its own bounds checks is reported as corruption and
/// leaves the store refusing operations, rather than being guessed at or
/// silently dropped: it was committed, so discarding it would lose data.
pub(super) fn recover(inner: &Inner) -> StorageResult<()> {
    if inner.recovered.load(Ordering::Acquire) {
        return Ok(());
    }
    if let Some(bytes) = journal::read(&inner.root, OPERATION)? {
        let ops = journal::decode(&bytes, OPERATION)?;
        replay(inner, &ops).map_err(|error| native_error(OPERATION, error))?;
        *lock(&inner.usage) = None;
        *lock(&inner.deferred) = None;
    }
    inner.recovered.store(true, Ordering::Release);
    Ok(())
}

/// Applies a journal durably, then retires it. Idempotent.
fn replay(inner: &Inner, ops: &[JournalOp]) -> io::Result<()> {
    for op in ops {
        let path = value_path(&inner.root, &op.namespace, &op.key);
        match &op.value {
            Some(value) => write_atomically(&path, value)?,
            None => {
                match fs::remove_file(&path) {
                    Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                    _ => {}
                }
                if let Some(parent) = path.parent() {
                    journal::sync_directory(parent);
                }
            }
        }
    }
    journal::remove(&inner.root)
}
