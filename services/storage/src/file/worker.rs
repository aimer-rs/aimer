//! Runs file operations, in order, on Venus's offload pool.
//!
//! A [`FileStorage`](super::FileStorage) owns no thread. Its operations go
//! through an `aimer_venus::Lane`: a per-store FIFO queue drained by one
//! `Venus::spawn_blocking` job at a time. That gives two things the old
//! dedicated writer thread gave, without the thread:
//!
//! * **Order.** Jobs run one at a time in the order they were issued, so a
//!   `read` issued after a `write` sees it, and two writes to one key land in
//!   call order, even though the pool has several workers.
//! * **Cheap quota decisions.** Only the drain job touches `usage`, so the
//!   store is scanned once and then kept up to date incrementally.
//!
//! The awaiting task is woken through the same thread-safe waker Venus gives
//! `Offloaded`, so the result is applied in the frame phase the task belongs to.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, PoisonError};

use aimer_venus::{Lane, LaneState};

use super::commit;
use crate::error::{
    StorageCapability, StorageError, StorageIoKind, StorageOperation, StorageResult,
};
use crate::limits::{StorageLimits, Usage};
use crate::storage::{StorageFuture, ready};

/// The state a store's lane shares with its jobs.
pub(super) struct Inner {
    pub(super) root: PathBuf,
    pub(super) limits: StorageLimits,
    /// `None` until the first operation that needs a quota decision, and again
    /// after any operation that may have left it wrong.
    pub(super) usage: Mutex<Option<Usage>>,
    /// Whether a committed batch left behind by a crash has been completed. Reset
    /// when a batch fails midway, so the next operation finishes it first.
    pub(super) recovered: AtomicBool,
    /// The files of the last committed batch, still waiting for their sync and
    /// for the journal to be retired. `None` when nothing is outstanding.
    pub(super) deferred: Mutex<Option<Vec<std::path::PathBuf>>>,
}

impl Inner {
    /// Creates the lane a store runs on.
    pub(super) fn lane(root: PathBuf, limits: StorageLimits) -> Lane<Inner> {
        Lane::new(Self {
            root,
            limits,
            usage: Mutex::new(None),
            recovered: AtomicBool::new(false),
            deferred: Mutex::new(None),
        })
    }
}

impl LaneState for Inner {
    /// `usage` may be half-updated, so it is rebuilt on next use.
    fn job_panicked(&self) {
        *lock(&self.usage) = None;
    }

    /// Finishes the deferred syncs of the last batch now, where no caller is
    /// waiting on them. A failure leaves them outstanding for the next mutating
    /// operation to retry and report.
    fn idle(&self) {
        let _ = commit::checkpoint(self);
    }
}

/// Locks `mutex`, ignoring poison: every job is run under `catch_unwind`, and a
/// poisoned `usage` is reset rather than trusted.
pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Queues `work` behind everything already issued to `lane` and returns its
/// result.
///
/// Resolves to `Unavailable { capability: Runtime }` when no Venus runtime is
/// installed on the calling thread, because there is no pool to run on.
///
/// Before `work` runs, a batch left half-applied by a crash is completed. A
/// `mutates` operation also waits for the last batch to be synced and its
/// journal retired, so a stale journal is never replayed over a newer write;
/// reads do not wait, because they see applied data either way.
pub(super) fn submit<T, F>(
    lane: &Lane<Inner>,
    operation: StorageOperation,
    mutates: bool,
    work: F,
) -> StorageFuture<T>
where
    T: Send + 'static,
    F: FnOnce(&Inner) -> StorageResult<T> + Send + 'static,
{
    let job = move |inner: &Inner| {
        commit::recover(inner)
            .and_then(|()| {
                if mutates {
                    commit::checkpoint(inner)
                } else {
                    Ok(())
                }
            })
            .and_then(|()| work(inner))
    };
    match lane.submit(job) {
        // A job that never completed (it panicked) resolves with an error rather
        // than leaving its future pending forever.
        Some(completion) => Box::pin(async move {
            completion.await.unwrap_or(Err(StorageError::Io {
                operation,
                kind: StorageIoKind::Other,
            }))
        }),
        None => ready(Err(StorageError::Unavailable {
            capability: StorageCapability::Runtime,
        })),
    }
}
