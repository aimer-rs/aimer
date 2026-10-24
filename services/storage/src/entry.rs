//! The values a listing or a metadata query hands back.

/// Metadata for one stored value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageMetadata {
    size: usize,
}

impl StorageMetadata {
    #[inline]
    pub(crate) const fn new(size: usize) -> Self {
        Self { size }
    }

    /// Returns the value's byte length.
    pub const fn size(self) -> usize {
        self.size
    }
}

/// A key and its metadata returned by a namespace listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageEntry {
    key: String,
    metadata: StorageMetadata,
}

impl StorageEntry {
    #[inline]
    pub(crate) fn new(key: String, size: usize) -> Self {
        Self {
            key,
            metadata: StorageMetadata::new(size),
        }
    }

    /// Returns the stored key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns the stored value's byte length.
    pub const fn size(&self) -> usize {
        self.metadata.size()
    }
}

/// A bounded listing response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageList {
    entries: Vec<StorageEntry>,
    truncated: bool,
}

impl StorageList {
    /// Builds a listing from entries already in key order, keeping at most
    /// `limit` of them.
    pub(crate) fn from_sorted(
        sorted: impl IntoIterator<Item = (String, usize)>,
        limit: usize,
    ) -> Self {
        let mut sorted = sorted.into_iter();
        let entries: Vec<StorageEntry> = sorted
            .by_ref()
            .take(limit)
            .map(|(key, size)| StorageEntry::new(key, size))
            .collect();
        Self {
            truncated: sorted.next().is_some(),
            entries,
        }
    }

    /// Returns the entries in deterministic key order.
    pub fn entries(&self) -> &[StorageEntry] {
        &self.entries
    }

    /// Returns whether additional entries exist beyond the bound.
    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }
}
