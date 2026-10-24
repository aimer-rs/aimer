//! A deterministic in-memory adapter.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::error::VaultResult;
use crate::key::{check_secret, validate, validate_component};
use crate::secret::SecretBytes;
use crate::vault::{Vault, VaultFuture, ready};

type Namespaces = BTreeMap<String, BTreeMap<String, SecretBytes>>;

/// A vault that keeps secrets in process memory.
///
/// For tests, and for code that wants a vault it can throw away. Nothing is
/// persisted and nothing is protected beyond what [`SecretBytes`] does: this is
/// **not** a secure store, and the crate never selects it on its own. A target
/// with no platform backend reports an error from
/// [`PlatformVault`](crate::PlatformVault) instead of falling back to it.
///
/// Venus runs every task on the UI thread, so the state is a plain
/// `Rc<RefCell<…>>` and each operation completes before its future is returned.
/// Clones share the same store.
///
/// # Examples
///
/// ```
/// use aimer_vault::{MemoryVault, Vault};
/// # use std::task::{Context, Poll, Waker};
/// # let mut cx = Context::from_waker(Waker::noop());
///
/// let vault = MemoryVault::new();
/// let mut write = vault.write("accounts", "api-token", b"s3cr3t");
/// assert!(matches!(write.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
/// ```
#[derive(Clone, Default)]
pub struct MemoryVault {
    state: Rc<RefCell<Namespaces>>,
}

impl MemoryVault {
    /// Creates an empty in-memory vault.
    pub fn new() -> Self {
        Self::default()
    }

    fn read_now(&self, namespace: &str, key: &str) -> VaultResult<Option<SecretBytes>> {
        validate(namespace, key)?;
        Ok(self
            .state
            .borrow()
            .get(namespace)
            .and_then(|keys| keys.get(key))
            .map(|secret| SecretBytes::from_slice(secret.expose())))
    }

    fn write_now(&self, namespace: &str, key: &str, secret: &[u8]) -> VaultResult<()> {
        validate(namespace, key)?;
        check_secret(secret.len())?;
        self.state
            .borrow_mut()
            .entry(namespace.to_owned())
            .or_default()
            .insert(key.to_owned(), SecretBytes::from_slice(secret));
        Ok(())
    }

    fn remove_now(&self, namespace: &str, key: &str) -> VaultResult<bool> {
        validate(namespace, key)?;
        let mut state = self.state.borrow_mut();
        let Some(keys) = state.get_mut(namespace) else {
            return Ok(false);
        };
        let removed = keys.remove(key).is_some();
        if keys.is_empty() {
            state.remove(namespace);
        }
        Ok(removed)
    }

    fn clear_now(&self, namespace: &str) -> VaultResult<()> {
        validate_component(namespace)?;
        self.state.borrow_mut().remove(namespace);
        Ok(())
    }
}

impl Vault for MemoryVault {
    fn read(&self, namespace: &str, key: &str) -> VaultFuture<Option<SecretBytes>> {
        ready(self.read_now(namespace, key))
    }

    fn write(&self, namespace: &str, key: &str, secret: &[u8]) -> VaultFuture<()> {
        ready(self.write_now(namespace, key, secret))
    }

    fn remove(&self, namespace: &str, key: &str) -> VaultFuture<bool> {
        ready(self.remove_now(namespace, key))
    }

    fn clear(&self, namespace: &str) -> VaultFuture<()> {
        ready(self.clear_now(namespace))
    }
}
