//! Resource bounds and the one place a write is admitted against them.

use crate::error::{QuotaKind, StorageError, StorageResult};

/// Bounded resource limits applied by a storage adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageLimits {
    max_value_bytes: usize,
    max_total_bytes: usize,
    max_entries: usize,
    max_list_entries: usize,
}

impl StorageLimits {
    /// The maximum UTF-8 byte length of a namespace or key component.
    ///
    /// Native adapters spell a component as lowercase hex in a file name, which
    /// doubles its length, and common filesystems cap a name at 255 bytes. 127
    /// is the longest component whose encoding still fits.
    pub const MAX_COMPONENT_BYTES: usize = 127;
    /// The default maximum size of one stored value.
    pub const DEFAULT_MAX_VALUE_BYTES: usize = 1024 * 1024;
    /// The default total size of one adapter.
    pub const DEFAULT_MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
    /// The default number of values in one adapter.
    pub const DEFAULT_MAX_ENTRIES: usize = 10_000;
    /// The default number of entries returned by one listing.
    pub const DEFAULT_MAX_LIST_ENTRIES: usize = 1_024;

    /// Creates validated storage limits.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidLimits`] when any bound is zero.
    pub fn new(
        max_value_bytes: usize,
        max_total_bytes: usize,
        max_entries: usize,
        max_list_entries: usize,
    ) -> StorageResult<Self> {
        if max_value_bytes == 0 || max_total_bytes == 0 || max_entries == 0 || max_list_entries == 0
        {
            return Err(StorageError::InvalidLimits);
        }
        Ok(Self {
            max_value_bytes,
            max_total_bytes,
            max_entries,
            max_list_entries,
        })
    }

    /// Returns the maximum size of one value.
    pub const fn max_value_bytes(self) -> usize {
        self.max_value_bytes
    }

    /// Returns the maximum total size of the adapter.
    pub const fn max_total_bytes(self) -> usize {
        self.max_total_bytes
    }

    /// Returns the maximum number of values in the adapter.
    pub const fn max_entries(self) -> usize {
        self.max_entries
    }

    /// Returns the maximum number of entries in one listing response.
    pub const fn max_list_entries(self) -> usize {
        self.max_list_entries
    }

    /// Rejects a value that alone exceeds the per-value bound.
    ///
    /// Checked before an operation is queued, so an oversized write never
    /// reaches a worker.
    pub(crate) fn check_value(self, len: usize) -> StorageResult<()> {
        if len > self.max_value_bytes {
            Err(StorageError::QuotaExceeded {
                kind: QuotaKind::ValueBytes,
                requested: len,
                limit: self.max_value_bytes,
            })
        } else {
            Ok(())
        }
    }

    /// Rejects a listing bound larger than this adapter will answer.
    pub(crate) fn check_list(self, requested: usize) -> StorageResult<()> {
        if requested > self.max_list_entries {
            Err(StorageError::LimitExceeded {
                requested,
                limit: self.max_list_entries,
            })
        } else {
            Ok(())
        }
    }

    /// Admits replacing `old_len` (absent when `None`) with `new_len` bytes.
    ///
    /// Returns the usage after the write. Every adapter funnels through here,
    /// so they reject the same writes in the same order: total bytes first,
    /// then entry count.
    pub(crate) fn admit_write(
        self,
        usage: Usage,
        old_len: Option<usize>,
        new_len: usize,
    ) -> StorageResult<Usage> {
        let total_bytes = usage
            .total_bytes
            .checked_sub(old_len.unwrap_or(0))
            .and_then(|total| total.checked_add(new_len))
            .unwrap_or(usize::MAX);
        if total_bytes > self.max_total_bytes {
            return Err(StorageError::QuotaExceeded {
                kind: QuotaKind::TotalBytes,
                requested: total_bytes,
                limit: self.max_total_bytes,
            });
        }
        if old_len.is_none() && usage.entries >= self.max_entries {
            return Err(StorageError::QuotaExceeded {
                kind: QuotaKind::Entries,
                requested: usage.entries.saturating_add(1),
                limit: self.max_entries,
            });
        }
        Ok(Usage {
            entries: usage.entries + usize::from(old_len.is_none()),
            total_bytes,
        })
    }
}

impl Default for StorageLimits {
    fn default() -> Self {
        Self {
            max_value_bytes: Self::DEFAULT_MAX_VALUE_BYTES,
            max_total_bytes: Self::DEFAULT_MAX_TOTAL_BYTES,
            max_entries: Self::DEFAULT_MAX_ENTRIES,
            max_list_entries: Self::DEFAULT_MAX_LIST_ENTRIES,
        }
    }
}

/// What an adapter currently holds, as far as quotas are concerned.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Usage {
    pub(crate) entries: usize,
    pub(crate) total_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> StorageLimits {
        StorageLimits::new(8, 16, 2, 4).expect("valid limits")
    }

    #[test]
    fn a_new_entry_counts_and_a_replacement_does_not() {
        let usage = Usage {
            entries: 1,
            total_bytes: 6,
        };
        assert_eq!(
            limits().admit_write(usage, None, 4),
            Ok(Usage {
                entries: 2,
                total_bytes: 10
            })
        );
        assert_eq!(
            limits().admit_write(usage, Some(6), 2),
            Ok(Usage {
                entries: 1,
                total_bytes: 2
            })
        );
    }

    #[test]
    fn total_bytes_are_rejected_before_entries() {
        let usage = Usage {
            entries: 2,
            total_bytes: 16,
        };
        assert_eq!(
            limits().admit_write(usage, None, 1),
            Err(StorageError::QuotaExceeded {
                kind: QuotaKind::TotalBytes,
                requested: 17,
                limit: 16,
            })
        );
        let usage = Usage {
            entries: 2,
            total_bytes: 4,
        };
        assert_eq!(
            limits().admit_write(usage, None, 1),
            Err(StorageError::QuotaExceeded {
                kind: QuotaKind::Entries,
                requested: 3,
                limit: 2,
            })
        );
    }

    #[test]
    fn inconsistent_usage_saturates_instead_of_wrapping() {
        let usage = Usage {
            entries: 1,
            total_bytes: 1,
        };
        assert!(matches!(
            limits().admit_write(usage, Some(5), 1),
            Err(StorageError::QuotaExceeded {
                kind: QuotaKind::TotalBytes,
                ..
            })
        ));
    }
}
