//! Persistent byte storage for Aimer applications.
//!
//! The core is a small [`Storage`] trait over `(namespace, key) -> bytes`, with
//! bounded [`StorageLimits`], typed [`StorageError`]s, and a pure
//! [byte-migration](migrate_bytes) contract. Adapters:
//!
//! | Adapter | Persists | Where |
//! |---|---|---|
//! | [`FileStorage`] | yes, on disk | native targets |
//! | `IndexedDbStorage` | yes, in the browser | `wasm32` only |
//! | [`MemoryStorage`] | no | everywhere; tests and caches |
//!
//! [`PlatformStorage`] names the persistent one for the current target, and
//! `PlatformStorage::for_app("com.example.app")` opens an application's own
//! store, so the same code loads the same data on every launch.
//!
//! # Batches
//!
//! [`Storage::commit`] applies a [`StorageBatch`] of writes and removes
//! atomically, and on disk for a fixed number of syncs however many entries it
//! holds. Use it to save related values together.
//!
//! # Venus-driven
//!
//! Operations return futures that are awaited by a Venus task. Native disk I/O
//! leaves the UI thread through Venus's offload pool and comes back as a wake in
//! the task's frame phase; the crate has no threads, channels, or runtime of its
//! own. The browser needs no pool: IndexedDB is already asynchronous.
//!
//! Typed serialization helpers are intentionally absent: this crate stays free
//! of serializer dependencies and stores bytes.

#![deny(missing_docs)]

mod batch;
mod entry;
mod error;
mod key;
mod keyspace;
mod limits;
mod location;
mod migration;
mod storage;

mod memory;

#[cfg(not(target_arch = "wasm32"))]
mod file;
#[cfg(target_arch = "wasm32")]
mod indexed_db;

pub use crate::batch::StorageBatch;
pub use crate::entry::{StorageEntry, StorageList, StorageMetadata};
pub use crate::error::{
    QuotaKind, StorageCapability, StorageError, StorageIoKind, StorageOperation, StorageResult,
};
pub use crate::limits::StorageLimits;
pub use crate::location::StorageLocation;
pub use crate::memory::MemoryStorage;
pub use crate::migration::{StorageMigration, migrate_bytes};
pub use crate::storage::{Storage, StorageFuture};

#[cfg(target_os = "android")]
pub use crate::file::set_android_data_dir;
#[cfg(not(target_arch = "wasm32"))]
pub use crate::file::{FileStorage, app_data_dir, app_dir};
#[cfg(target_arch = "wasm32")]
pub use crate::indexed_db::IndexedDbStorage;

/// The persistent adapter for the current target: [`FileStorage`] on native
/// targets, `IndexedDbStorage` in the browser.
///
/// Both open an application's store with `for_app(app_id)`.
#[cfg(not(target_arch = "wasm32"))]
pub type PlatformStorage = FileStorage;

/// The persistent adapter for the current target: [`FileStorage`] on native
/// targets, `IndexedDbStorage` in the browser.
///
/// Both open an application's store with `for_app(app_id)`.
#[cfg(target_arch = "wasm32")]
pub type PlatformStorage = IndexedDbStorage;
