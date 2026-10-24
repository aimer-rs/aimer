//! Several writes and removes applied as one unit.

use std::collections::HashMap;

use crate::error::StorageResult;
use crate::key::validate;
use crate::limits::{StorageLimits, Usage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BatchOp {
    Write {
        namespace: String,
        key: String,
        value: Vec<u8>,
    },
    Remove {
        namespace: String,
        key: String,
    },
}

impl BatchOp {
    fn target(&self) -> (&str, &str) {
        match self {
            Self::Write { namespace, key, .. } | Self::Remove { namespace, key } => {
                (namespace, key)
            }
        }
    }
}

/// An ordered set of writes and removes that [`Storage::commit`] applies
/// atomically: afterwards every operation has taken effect, or none has.
///
/// Operations may span namespaces. If the batch touches one key more than once,
/// the last operation on it wins, exactly as if they had been issued one at a
/// time.
///
/// Quotas are judged on the batch's *net* effect, so a batch that removes old
/// entries to make room for new ones succeeds where the same operations issued
/// one by one could be refused midway.
///
/// [`Storage::commit`]: crate::Storage::commit
///
/// # Examples
///
/// ```
/// use aimer_storage::StorageBatch;
///
/// let mut batch = StorageBatch::new();
/// batch
///     .write("settings", "theme", b"dark")
///     .write("settings", "language", b"km")
///     .remove("drafts", "stale");
/// assert_eq!(batch.len(), 3);
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StorageBatch {
    ops: Vec<BatchOp>,
}

impl StorageBatch {
    /// Creates an empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty batch with room for `capacity` operations.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ops: Vec::with_capacity(capacity),
        }
    }

    /// Adds a write, replacing or creating `key` in `namespace`.
    ///
    /// The value is copied. Keys and sizes are validated when the batch is
    /// committed, not here, so a bad entry rejects the whole batch.
    pub fn write(&mut self, namespace: &str, key: &str, value: &[u8]) -> &mut Self {
        self.ops.push(BatchOp::Write {
            namespace: namespace.to_owned(),
            key: key.to_owned(),
            value: value.to_vec(),
        });
        self
    }

    /// Adds a removal. Removing a key that does not exist is not an error.
    pub fn remove(&mut self, namespace: &str, key: &str) -> &mut Self {
        self.ops.push(BatchOp::Remove {
            namespace: namespace.to_owned(),
            key: key.to_owned(),
        });
        self
    }

    /// The number of operations added so far, duplicates included.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether no operation has been added.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Checks every key and value size, so one bad entry rejects the batch
    /// before any adapter does work.
    pub(crate) fn validate(&self, limits: StorageLimits) -> StorageResult<()> {
        for op in &self.ops {
            let (namespace, key) = op.target();
            validate(namespace, key)?;
            if let BatchOp::Write { value, .. } = op {
                limits.check_value(value.len())?;
            }
        }
        Ok(())
    }

    /// Reduces the batch to one operation per key, the last one issued, in the
    /// order each key first appeared.
    pub(crate) fn coalesce(&self) -> Vec<FinalOp<'_>> {
        let mut index: HashMap<(&str, &str), usize> = HashMap::with_capacity(self.ops.len());
        let mut finals: Vec<FinalOp<'_>> = Vec::with_capacity(self.ops.len());
        for op in &self.ops {
            let (namespace, key) = op.target();
            let value = match op {
                BatchOp::Write { value, .. } => Some(value.as_slice()),
                BatchOp::Remove { .. } => None,
            };
            match index.get(&(namespace, key)) {
                Some(&at) => finals[at].value = value,
                None => {
                    index.insert((namespace, key), finals.len());
                    finals.push(FinalOp {
                        namespace,
                        key,
                        value,
                    });
                }
            }
        }
        finals
    }
}

/// What a batch finally does to one key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FinalOp<'a> {
    pub(crate) namespace: &'a str,
    pub(crate) key: &'a str,
    /// The bytes to store, or `None` to remove.
    pub(crate) value: Option<&'a [u8]>,
}

/// Admits a coalesced batch against `limits` and returns the usage after it.
///
/// `old_lens[i]` is the current size of `ops[i]`'s key, `None` when absent.
/// Removals are accounted before writes so they can make room for them.
pub(crate) fn plan(
    limits: StorageLimits,
    mut usage: Usage,
    new_lens: &[Option<usize>],
    old_lens: &[Option<usize>],
) -> StorageResult<Usage> {
    debug_assert_eq!(new_lens.len(), old_lens.len());
    for (new, old) in new_lens.iter().zip(old_lens) {
        if let (None, Some(old)) = (new, old) {
            usage.entries = usage.entries.saturating_sub(1);
            usage.total_bytes = usage.total_bytes.saturating_sub(*old);
        }
    }
    for (new, old) in new_lens.iter().zip(old_lens) {
        if let Some(new) = new {
            usage = limits.admit_write(usage, *old, *new)?;
        }
    }
    Ok(usage)
}

/// The new sizes of coalesced operations, for [`plan`].
pub(crate) fn new_lens(ops: &[FinalOp<'_>]) -> Vec<Option<usize>> {
    ops.iter().map(|op| op.value.map(<[u8]>::len)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{QuotaKind, StorageError};

    fn limits(total: usize, entries: usize) -> StorageLimits {
        StorageLimits::new(16, total, entries, 8).unwrap()
    }

    #[test]
    fn the_last_operation_on_a_key_wins_and_order_is_first_appearance() {
        let mut batch = StorageBatch::new();
        batch
            .write("a", "1", b"old")
            .write("b", "1", b"x")
            .remove("a", "1")
            .write("a", "1", b"new")
            .write("b", "1", b"y")
            .remove("b", "2");

        let finals = batch.coalesce();
        assert_eq!(
            finals,
            [
                FinalOp {
                    namespace: "a",
                    key: "1",
                    value: Some(b"new")
                },
                FinalOp {
                    namespace: "b",
                    key: "1",
                    value: Some(b"y")
                },
                FinalOp {
                    namespace: "b",
                    key: "2",
                    value: None
                },
            ]
        );
    }

    #[test]
    fn the_same_key_in_different_namespaces_is_not_a_duplicate() {
        let mut batch = StorageBatch::new();
        batch.write("a", "k", b"1").write("b", "k", b"2");
        assert_eq!(batch.coalesce().len(), 2);
    }

    #[test]
    fn one_bad_entry_rejects_the_whole_batch() {
        let limits = limits(100, 10);
        let mut batch = StorageBatch::new();
        batch.write("ok", "key", b"1");
        assert_eq!(batch.validate(limits), Ok(()));

        batch.write("ok", "bad/key", b"1");
        assert_eq!(batch.validate(limits), Err(StorageError::InvalidKey));

        let mut oversized = StorageBatch::new();
        oversized.write("ok", "key", &[0; 17]);
        assert!(matches!(
            oversized.validate(limits),
            Err(StorageError::QuotaExceeded {
                kind: QuotaKind::ValueBytes,
                ..
            })
        ));

        let mut bad_remove = StorageBatch::new();
        bad_remove.remove("", "key");
        assert_eq!(bad_remove.validate(limits), Err(StorageError::InvalidKey));
    }

    #[test]
    fn removals_make_room_for_writes_in_the_same_batch() {
        // Full: two entries, 10 of 10 bytes.
        let full = Usage {
            entries: 2,
            total_bytes: 10,
        };
        let limits = limits(10, 2);

        // Write-then-remove one at a time would be refused at the first write.
        assert!(limits.admit_write(full, None, 5).is_err());

        // As a batch, the net effect is one entry swapped for another.
        let after = plan(limits, full, &[Some(5), None], &[None, Some(5)]).unwrap();
        assert_eq!(
            after,
            Usage {
                entries: 2,
                total_bytes: 10
            }
        );
    }

    #[test]
    fn removing_an_absent_key_changes_nothing() {
        let usage = Usage {
            entries: 1,
            total_bytes: 3,
        };
        assert_eq!(plan(limits(10, 2), usage, &[None], &[None]), Ok(usage));
    }

    #[test]
    fn replacing_counts_the_difference_and_new_keys_count_entries() {
        let usage = Usage {
            entries: 1,
            total_bytes: 6,
        };
        let after = plan(limits(20, 3), usage, &[Some(2), Some(4)], &[Some(6), None]).unwrap();
        assert_eq!(
            after,
            Usage {
                entries: 2,
                total_bytes: 6
            }
        );
    }

    #[test]
    fn a_batch_over_quota_is_refused_with_the_net_amounts() {
        let usage = Usage {
            entries: 1,
            total_bytes: 8,
        };
        assert_eq!(
            plan(limits(10, 5), usage, &[Some(4)], &[None]),
            Err(StorageError::QuotaExceeded {
                kind: QuotaKind::TotalBytes,
                requested: 12,
                limit: 10,
            })
        );
        assert!(matches!(
            plan(
                limits(100, 2),
                Usage {
                    entries: 2,
                    total_bytes: 0
                },
                &[Some(1)],
                &[None]
            ),
            Err(StorageError::QuotaExceeded {
                kind: QuotaKind::Entries,
                ..
            })
        ));
    }
}
