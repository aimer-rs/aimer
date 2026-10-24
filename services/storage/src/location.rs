//! The kinds of place an application keeps data.

/// Where on a device a store lives, and how long its contents may be expected
/// to last.
///
/// The variants are the common ground of the platform conventions: Windows
/// `ApplicationData` (Local, Roaming, LocalCache, Temporary), Apple's
/// Application Support, Caches and temporary directories, Android's `filesDir`
/// and `cacheDir`, and the browser's persistent and evictable storage. Each
/// platform maps them to its own directories; see [`app_dir`](crate::app_dir).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StorageLocation {
    /// Data the app cannot recreate and the system will not remove: documents,
    /// settings, user-created content.
    Data,
    /// Like [`Data`](Self::Data), but on platforms that synchronize app data
    /// between a user's devices (Windows roaming data) it is the part that may
    /// be. Everywhere else it is the same place as `Data`, so code that stores
    /// roaming data works unchanged on a platform without the feature.
    Roaming,
    /// Data the app can rebuild, kept to save work. The system may delete it
    /// when space runs short, so it must never be the only copy of anything.
    Cache,
    /// Scratch data for the current run. It may be deleted between runs, and on
    /// the web it lives in memory and does not outlive the page.
    Temp,
}

impl StorageLocation {
    /// Whether the system may discard what is stored here without being asked.
    pub const fn is_disposable(self) -> bool {
        matches!(self, Self::Cache | Self::Temp)
    }
}
