//! A deterministic in-memory adapter.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::batch::{StorageBatch, new_lens, plan};
use crate::entry::{StorageList, StorageMetadata};
use crate::key::{validate, validate_namespace};
use crate::limits::{StorageLimits, Usage};
use crate::storage::{Storage, StorageFuture, ready};

/// Namespaces nest their keys, so a lookup borrows `&str` instead of building
/// an owned `(String, String)` pair, and a listing or a clear touches one
/// namespace rather than filtering the whole store.
#[derive(Default)]
struct MemoryState {
    values: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
    usage: Usage,
}

/// A deterministic in-memory storage adapter.
///
/// Venus runs every task on the UI thread, so there is nothing to synchronize:
/// the state is a plain `Rc<RefCell<…>>` and each operation completes before
/// its future is returned. Clones share the same store. Nothing is persisted;
/// use [`FileStorage`](crate::FileStorage) or
/// [`IndexedDbStorage`](crate::IndexedDbStorage) for that.
///
/// # Examples
///
/// ```
/// use aimer_storage::{MemoryStorage, Storage};
/// # use std::task::{Context, Poll, Waker};
/// # let mut cx = Context::from_waker(Waker::noop());
///
/// let storage = MemoryStorage::new();
/// let mut write = storage.write("profile", "name", b"Ada");
/// assert!(matches!(write.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
/// ```
#[derive(Clone)]
pub struct MemoryStorage {
    state: Rc<RefCell<MemoryState>>,
    limits: StorageLimits,
}

impl MemoryStorage {
    /// Creates an empty memory adapter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty memory adapter with explicit resource limits.
    pub fn with_limits(limits: StorageLimits) -> Self {
        Self {
            state: Rc::new(RefCell::new(MemoryState::default())),
            limits,
        }
    }

    /// Returns the limits enforced by this adapter.
    pub const fn limits(&self) -> StorageLimits {
        self.limits
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::with_limits(StorageLimits::default())
    }
}

impl Storage for MemoryStorage {
    fn read(&self, namespace: &str, key: &str) -> StorageFuture<Option<Vec<u8>>> {
        ready(validate(namespace, key).map(|()| {
            self.state
                .borrow()
                .values
                .get(namespace)
                .and_then(|keys| keys.get(key))
                .cloned()
        }))
    }

    fn write(&self, namespace: &str, key: &str, value: &[u8]) -> StorageFuture<()> {
        ready(
            validate(namespace, key)
                .and_then(|()| self.limits.check_value(value.len()))
                .and_then(|()| {
                    let mut state = self.state.borrow_mut();
                    let old_len = state
                        .values
                        .get(namespace)
                        .and_then(|keys| keys.get(key))
                        .map(Vec::len);
                    state.usage = self.limits.admit_write(state.usage, old_len, value.len())?;
                    state
                        .values
                        .entry(namespace.to_owned())
                        .or_default()
                        .insert(key.to_owned(), value.to_vec());
                    Ok(())
                }),
        )
    }

    fn remove(&self, namespace: &str, key: &str) -> StorageFuture<bool> {
        ready(validate(namespace, key).map(|()| {
            let mut state = self.state.borrow_mut();
            let Some(keys) = state.values.get_mut(namespace) else {
                return false;
            };
            let removed = keys.remove(key);
            if keys.is_empty() {
                state.values.remove(namespace);
            }
            match removed {
                Some(value) => {
                    state.usage.entries -= 1;
                    state.usage.total_bytes -= value.len();
                    true
                }
                None => false,
            }
        }))
    }

    fn clear(&self, namespace: &str) -> StorageFuture<usize> {
        ready(validate_namespace(namespace).map(|()| {
            let mut state = self.state.borrow_mut();
            let Some(keys) = state.values.remove(namespace) else {
                return 0;
            };
            let bytes: usize = keys.values().map(Vec::len).sum();
            state.usage.entries -= keys.len();
            state.usage.total_bytes -= bytes;
            keys.len()
        }))
    }

    fn list(&self, namespace: &str, limit: usize) -> StorageFuture<StorageList> {
        ready(
            validate_namespace(namespace)
                .and_then(|()| self.limits.check_list(limit))
                .map(|()| {
                    let state = self.state.borrow();
                    StorageList::from_sorted(
                        state
                            .values
                            .get(namespace)
                            .into_iter()
                            .flatten()
                            .map(|(key, value)| (key.clone(), value.len())),
                        limit,
                    )
                }),
        )
    }

    fn metadata(&self, namespace: &str, key: &str) -> StorageFuture<Option<StorageMetadata>> {
        ready(validate(namespace, key).map(|()| {
            self.state
                .borrow()
                .values
                .get(namespace)
                .and_then(|keys| keys.get(key))
                .map(|value| StorageMetadata::new(value.len()))
        }))
    }

    fn commit(&self, batch: StorageBatch) -> StorageFuture<()> {
        ready(batch.validate(self.limits).and_then(|()| {
            let ops = batch.coalesce();
            let mut state = self.state.borrow_mut();
            let old_lens: Vec<Option<usize>> = ops
                .iter()
                .map(|op| {
                    state
                        .values
                        .get(op.namespace)
                        .and_then(|keys| keys.get(op.key))
                        .map(Vec::len)
                })
                .collect();
            // Everything that can fail has been decided before the first change,
            // so nothing below has to be undone.
            state.usage = plan(self.limits, state.usage, &new_lens(&ops), &old_lens)?;
            for op in ops {
                match op.value {
                    Some(value) => {
                        state
                            .values
                            .entry(op.namespace.to_owned())
                            .or_default()
                            .insert(op.key.to_owned(), value.to_vec());
                    }
                    None => {
                        if let Some(keys) = state.values.get_mut(op.namespace) {
                            keys.remove(op.key);
                            if keys.is_empty() {
                                state.values.remove(op.namespace);
                            }
                        }
                    }
                }
            }
            Ok(())
        }))
    }
}

/// Resolves a future that is known to be ready, for tests that have no runtime.
#[cfg(test)]
pub(crate) fn now<T>(mut future: StorageFuture<T>) -> crate::error::StorageResult<T> {
    use std::task::{Context, Poll, Waker};

    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("a memory operation completes before it is returned"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{QuotaKind, StorageError};

    #[test]
    fn usage_tracks_writes_replacements_removals_and_clears() {
        let storage = MemoryStorage::new();
        now(storage.write("a", "1", b"xxxx")).unwrap();
        now(storage.write("a", "2", b"yy")).unwrap();
        now(storage.write("b", "1", b"z")).unwrap();
        now(storage.write("a", "1", b"x")).unwrap();
        assert_eq!(
            storage.state.borrow().usage,
            Usage {
                entries: 3,
                total_bytes: 4
            }
        );

        assert!(now(storage.remove("a", "1")).unwrap());
        assert!(!now(storage.remove("a", "1")).unwrap());
        assert_eq!(now(storage.clear("a")).unwrap(), 1);
        assert_eq!(now(storage.clear("a")).unwrap(), 0);
        assert_eq!(
            storage.state.borrow().usage,
            Usage {
                entries: 1,
                total_bytes: 1
            }
        );
    }

    #[test]
    fn emptied_namespaces_are_forgotten() {
        let storage = MemoryStorage::new();
        now(storage.write("a", "1", b"x")).unwrap();
        now(storage.remove("a", "1")).unwrap();
        assert!(storage.state.borrow().values.is_empty());
    }

    #[test]
    fn a_rejected_write_changes_nothing() {
        let limits = StorageLimits::new(4, 6, 2, 4).unwrap();
        let storage = MemoryStorage::with_limits(limits);
        now(storage.write("a", "1", b"xxxx")).unwrap();

        let error = now(storage.write("a", "2", b"yyy")).unwrap_err();
        assert_eq!(
            error,
            StorageError::QuotaExceeded {
                kind: QuotaKind::TotalBytes,
                requested: 7,
                limit: 6,
            }
        );
        assert_eq!(now(storage.read("a", "2")).unwrap(), None);
        assert_eq!(
            storage.state.borrow().usage,
            Usage {
                entries: 1,
                total_bytes: 4
            }
        );
    }

    /// The incrementally-maintained usage must equal what is actually stored.
    fn recomputed(storage: &MemoryStorage) -> Usage {
        let state = storage.state.borrow();
        Usage {
            entries: state.values.values().map(BTreeMap::len).sum(),
            total_bytes: state
                .values
                .values()
                .flat_map(BTreeMap::values)
                .map(Vec::len)
                .sum(),
        }
    }

    #[test]
    fn a_batch_applies_every_operation_and_keeps_usage_exact() {
        let storage = MemoryStorage::new();
        now(storage.write("a", "keep", b"1234")).unwrap();
        now(storage.write("a", "drop", b"12")).unwrap();

        let mut batch = StorageBatch::new();
        batch
            .write("a", "keep", b"x")
            .remove("a", "drop")
            .remove("a", "never-existed")
            .write("b", "new", b"yz")
            .write("b", "new", b"final");
        now(storage.commit(batch)).unwrap();

        assert_eq!(now(storage.read("a", "keep")).unwrap(), Some(b"x".to_vec()));
        assert_eq!(now(storage.read("a", "drop")).unwrap(), None);
        assert_eq!(
            now(storage.read("b", "new")).unwrap(),
            Some(b"final".to_vec())
        );
        assert_eq!(storage.state.borrow().usage, recomputed(&storage));
        assert_eq!(
            storage.state.borrow().usage,
            Usage {
                entries: 2,
                total_bytes: 6
            }
        );
    }

    #[test]
    fn a_rejected_batch_leaves_the_store_untouched() {
        let limits = StorageLimits::new(8, 10, 5, 4).unwrap();
        let storage = MemoryStorage::with_limits(limits);
        now(storage.write("a", "1", b"123456")).unwrap();

        let mut over_quota = StorageBatch::new();
        over_quota
            .remove("a", "1")
            .write("a", "2", b"12345")
            .write("a", "3", b"123456");
        assert!(matches!(
            now(storage.commit(over_quota)).unwrap_err(),
            StorageError::QuotaExceeded {
                kind: QuotaKind::TotalBytes,
                ..
            }
        ));

        let mut invalid = StorageBatch::new();
        invalid.remove("a", "1").write("a", "bad/key", b"1");
        assert_eq!(
            now(storage.commit(invalid)).unwrap_err(),
            StorageError::InvalidKey
        );

        assert_eq!(
            now(storage.read("a", "1")).unwrap(),
            Some(b"123456".to_vec())
        );
        assert_eq!(storage.state.borrow().usage, recomputed(&storage));
    }

    #[test]
    fn a_batch_may_swap_entries_in_a_full_store() {
        let limits = StorageLimits::new(8, 100, 1, 4).unwrap();
        let storage = MemoryStorage::with_limits(limits);
        now(storage.write("a", "old", b"1")).unwrap();

        let mut swap = StorageBatch::new();
        swap.write("a", "new", b"2").remove("a", "old");
        now(storage.commit(swap)).unwrap();

        assert_eq!(now(storage.read("a", "old")).unwrap(), None);
        assert_eq!(now(storage.read("a", "new")).unwrap(), Some(b"2".to_vec()));
    }

    #[test]
    fn an_empty_batch_is_a_no_op() {
        let storage = MemoryStorage::new();
        now(storage.commit(StorageBatch::new())).unwrap();
        assert!(storage.state.borrow().values.is_empty());
    }

    #[test]
    fn clones_share_one_store() {
        let storage = MemoryStorage::new();
        let clone = storage.clone();
        now(storage.write("a", "1", b"x")).unwrap();
        assert_eq!(now(clone.read("a", "1")).unwrap(), Some(b"x".to_vec()));
    }
}
