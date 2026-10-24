//! The adapter contract.

use std::future::Future;
use std::pin::Pin;

use crate::batch::StorageBatch;
use crate::entry::{StorageList, StorageMetadata};
use crate::error::{StorageError, StorageOperation, StorageResult};

/// A boxed asynchronous storage operation.
///
/// Deliberately not `Send`: it is awaited by a Venus task on the UI thread,
/// which keeps its `Rc` captures. Only the blocking work behind it crosses to a
/// worker.
pub type StorageFuture<T> = Pin<Box<dyn Future<Output = StorageResult<T>> + 'static>>;

/// A future that is already resolved.
pub(crate) fn ready<T: 'static>(result: StorageResult<T>) -> StorageFuture<T> {
    Box::pin(std::future::ready(result))
}

/// A byte-oriented storage backend.
///
/// Values live in a `(namespace, key)` space. Operations are issued in call
/// order and take effect in that order: a `read` issued after a `write` sees
/// it, even if the `write` has not been awaited yet.
pub trait Storage {
    /// Reads a value from a namespace, returning `None` when it is absent.
    fn read(&self, namespace: &str, key: &str) -> StorageFuture<Option<Vec<u8>>>;

    /// Replaces or creates a value in a namespace.
    fn write(&self, namespace: &str, key: &str, value: &[u8]) -> StorageFuture<()>;

    /// Removes one value and reports whether it existed.
    fn remove(&self, namespace: &str, key: &str) -> StorageFuture<bool>;

    /// Removes all values in one namespace and reports the number removed.
    fn clear(&self, namespace: &str) -> StorageFuture<usize>;

    /// Lists up to the requested number of values in a namespace.
    fn list(&self, namespace: &str, limit: usize) -> StorageFuture<StorageList>;

    /// Returns metadata for one value, if it exists.
    fn metadata(&self, namespace: &str, key: &str) -> StorageFuture<Option<StorageMetadata>>;

    /// Applies every operation in `batch` atomically: when the future resolves
    /// `Ok`, all of them have taken effect, and a batch that is rejected (an
    /// invalid key, a quota) has taken none. No outcome is ever a partial batch.
    ///
    /// The one ambiguous case is an I/O failure on a persistent adapter: the
    /// batch is then still all-or-nothing, but whether it was applied is decided
    /// by whether it reached its commit point, and the adapter finishes a
    /// committed batch before it serves the next operation. Re-reading is
    /// therefore always safe.
    ///
    /// On a persistent adapter, `Ok` also means the batch is durable, and it
    /// costs a fixed number of syncs however many entries it holds, where the
    /// same entries written one by one cost one sync each. A batch is the right
    /// way to save several related values together.
    ///
    /// An empty batch succeeds and touches nothing. See [`StorageBatch`] for how
    /// duplicate keys and quotas are treated.
    ///
    /// The default implementation reports [`StorageError::Unsupported`], so an
    /// adapter written before batches existed keeps compiling.
    ///
    /// # Errors
    ///
    /// * [`StorageError::InvalidKey`] if any key in the batch is invalid.
    /// * [`StorageError::QuotaExceeded`] if a value is too large, or the batch's
    ///   net effect would exceed a total or entry limit.
    /// * Any I/O error from the adapter.
    fn commit(&self, batch: StorageBatch) -> StorageFuture<()> {
        drop(batch);
        ready(Err(StorageError::Unsupported {
            operation: StorageOperation::Write,
        }))
    }
}
