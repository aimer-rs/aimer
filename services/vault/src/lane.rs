//! Runs blocking secret-store calls, in order, on Venus's offload pool.
//!
//! The queue is `aimer_venus::Lane`; this wrapper only speaks the vault's error
//! vocabulary: no runtime installed is `Unavailable`, and a job that panicked is
//! `Interrupted`.

use crate::error::{VaultCapability, VaultError, VaultResult};
use crate::vault::{VaultFuture, ready};

/// An ordered queue of blocking jobs. Clones share the queue.
#[derive(Clone, Default)]
pub(crate) struct Lane(aimer_venus::Lane);

impl Lane {
    /// Queues `work` behind everything already issued to this lane and returns
    /// its result.
    ///
    /// Resolves to `Unavailable { capability: Runtime }` when no Venus runtime is
    /// installed on the calling thread, because there is no pool to run on.
    pub(crate) fn submit<T, F>(&self, work: F) -> VaultFuture<T>
    where
        T: Send + 'static,
        F: FnOnce() -> VaultResult<T> + Send + 'static,
    {
        match self.0.submit(move |_| work()) {
            Some(completion) => Box::pin(async move {
                completion.await.unwrap_or(Err(VaultError::Interrupted))
            }),
            None => ready(Err(VaultError::Unavailable {
                capability: VaultCapability::Runtime,
            })),
        }
    }
}
