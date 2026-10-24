//! The error vocabulary shared by every storage adapter.

/// The result type returned by storage operations.
pub type StorageResult<T> = Result<T, StorageError>;

/// A platform capability that an adapter needs and the host may not supply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageCapability {
    /// A native filesystem-backed adapter, or a directory to put it in.
    NativeFile,
    /// A browser IndexedDB-backed adapter.
    WebIndexedDb,
    /// The Venus runtime installed for the calling thread.
    ///
    /// Native disk I/O is driven by Venus, so a [`FileStorage`](crate::FileStorage)
    /// operation made before `Venus::install` has nowhere to run.
    Runtime,
}

/// An operation that can fail at the storage boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageOperation {
    /// Reading one value.
    Read,
    /// Replacing or creating one value.
    Write,
    /// Removing one value.
    Remove,
    /// Clearing one namespace.
    Clear,
    /// Listing one namespace.
    List,
    /// Reading one value's metadata.
    Metadata,
}

/// A coarse I/O failure that does not fit one of the typed storage errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageIoKind {
    /// The underlying operation failed for another operating-system reason.
    Other,
    /// The device or the browser's origin quota has no room left.
    StorageFull,
    /// The target is on a read-only filesystem.
    ReadOnly,
}

/// The quota dimension that rejected a write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaKind {
    /// One value exceeds the per-value byte limit.
    ValueBytes,
    /// The resulting adapter exceeds the total byte limit.
    TotalBytes,
    /// The resulting adapter exceeds the entry-count limit.
    Entries,
}

/// Errors returned by storage adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageError {
    /// A namespace or key is empty, too long, or contains an unsafe byte.
    InvalidKey,
    /// A limits configuration contains a zero bound.
    InvalidLimits,
    /// A configured quota would be exceeded.
    QuotaExceeded {
        /// The quota dimension that would be exceeded.
        kind: QuotaKind,
        /// The resulting amount requested by the operation.
        requested: usize,
        /// The configured maximum.
        limit: usize,
    },
    /// A bounded listing contains more entries than the requested bound.
    LimitExceeded {
        /// The number of entries needed to return the complete listing.
        requested: usize,
        /// The configured listing bound.
        limit: usize,
    },
    /// The requested platform capability is unavailable.
    Unavailable {
        /// The unavailable capability.
        capability: StorageCapability,
    },
    /// The operating system denied an operation.
    PermissionDenied {
        /// The denied operation.
        operation: StorageOperation,
    },
    /// Stored bytes or metadata are corrupt.
    Corrupt {
        /// The operation that encountered corrupt data.
        operation: StorageOperation,
    },
    /// The platform reported an unclassified I/O failure.
    Io {
        /// The operation that failed.
        operation: StorageOperation,
        /// The coarse error category.
        kind: StorageIoKind,
    },
    /// No exact next step exists for a requested schema migration.
    MigrationUnavailable {
        /// The version at which migration stopped.
        from_version: u32,
        /// The requested final version.
        to_version: u32,
    },
    /// A migration does not move monotonically toward its target version.
    InvalidMigration {
        /// The version accepted by the invalid migration.
        from_version: u32,
        /// The invalid version it produces.
        to_version: u32,
    },
    /// An adapter does not support the requested operation.
    Unsupported {
        /// The unsupported operation.
        operation: StorageOperation,
    },
}
