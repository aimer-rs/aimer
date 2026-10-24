//! The adapter contract.

use std::future::Future;
use std::pin::Pin;

use crate::error::VaultResult;
use crate::secret::SecretBytes;

/// A boxed asynchronous vault operation.
///
/// Deliberately not `Send`: it is awaited by a Venus task on the UI thread,
/// which keeps its `Rc` captures. Only the blocking work behind it crosses to a
/// worker.
pub type VaultFuture<T> = Pin<Box<dyn Future<Output = VaultResult<T>> + 'static>>;

/// A future that is already resolved.
pub(crate) fn ready<T: 'static>(result: VaultResult<T>) -> VaultFuture<T> {
    Box::pin(std::future::ready(result))
}

/// A byte-oriented store for secrets.
///
/// Secrets live in a `(namespace, key)` space, like
/// [`aimer_storage::Storage`](https://docs.rs/aimer_storage) values, and the
/// same rules decide which names are valid. The differences are the point:
/// values come back as [`SecretBytes`], which wipe themselves and never print,
/// and a persistent adapter keeps them in the platform's secret store rather
/// than in a file.
///
/// Operations are issued in call order and take effect in that order: a `read`
/// issued after a `write` sees it, even if the `write` has not been awaited yet.
///
/// A vault has no listing, no metadata, and no batch. Enumerating secrets is an
/// attack surface the platform stores deliberately make awkward, and an
/// application that needs to know what it holds should keep an index in
/// ordinary storage and keep only the secrets here.
pub trait Vault {
    /// Reads a secret, returning `None` when it is absent.
    ///
    /// # Errors
    ///
    /// * [`VaultError::InvalidKey`](crate::VaultError::InvalidKey) for an invalid name.
    /// * [`VaultError::Locked`](crate::VaultError::Locked) while the store is locked.
    /// * Any platform error from the adapter.
    fn read(&self, namespace: &str, key: &str) -> VaultFuture<Option<SecretBytes>>;

    /// Replaces or creates a secret.
    ///
    /// # Errors
    ///
    /// * [`VaultError::InvalidKey`](crate::VaultError::InvalidKey) for an invalid name.
    /// * [`VaultError::EmptySecret`](crate::VaultError::EmptySecret) for an empty secret.
    /// * [`VaultError::SecretTooLarge`](crate::VaultError::SecretTooLarge) above
    ///   [`MAX_SECRET_BYTES`](crate::MAX_SECRET_BYTES).
    /// * Any platform error from the adapter.
    fn write(&self, namespace: &str, key: &str, secret: &[u8]) -> VaultFuture<()>;

    /// Removes one secret and reports whether it existed.
    fn remove(&self, namespace: &str, key: &str) -> VaultFuture<bool>;

    /// Removes every secret in one namespace. Removing an empty namespace
    /// succeeds.
    fn clear(&self, namespace: &str) -> VaultFuture<()>;
}
