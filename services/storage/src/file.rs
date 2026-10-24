//! The native, disk-backed adapter.

mod app_dir;
mod commit;
mod journal;
mod layout;
mod worker;

#[cfg(test)]
mod recovery_tests;

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(target_os = "android")]
pub use self::app_dir::set_android_data_dir;
pub use self::app_dir::{app_data_dir, app_dir};

use aimer_venus::Lane;

use self::commit::{commit_batch, current_usage};
use self::layout::{
    namespace_entries, namespace_path, native_error, stat_value, value_path, write_atomically,
};
use self::worker::{Inner, lock, submit};
use crate::batch::StorageBatch;
use crate::entry::{StorageList, StorageMetadata};
use crate::error::{StorageError, StorageOperation, StorageResult};
use crate::key::{validate, validate_namespace};
use crate::limits::StorageLimits;
use crate::location::StorageLocation;
use crate::storage::{Storage, StorageFuture, ready};

/// Where, under an app's data directory, [`FileStorage::for_app`] keeps values.
///
/// A subdirectory of its own, because the adapter treats anything in its root
/// that it did not write as corruption, and an app directory is not exclusively
/// ours.
const APP_SUBDIRECTORY: &str = "storage";

/// A native file-backed storage adapter: persistent data on disk.
///
/// Values are stored below an encoded namespace/key path, and a write is a
/// temporary file plus a rename, so a reader — or a crash — never sees a
/// partially-written value, and a value that has been acknowledged survives
/// power loss. A store that is opened again, by a later run of the app or by a
/// second handle, finds every value that was written.
///
/// # Venus-driven
///
/// This adapter owns no threads. Each operation is queued on the store and run,
/// in the order it was issued, on the calling thread's Venus offload pool; the
/// future it returns is awaited by a Venus task and resolves on the UI thread.
/// Every method therefore needs a Venus runtime installed on the calling thread
/// (`Venus::install`) and otherwise resolves to
/// [`StorageError::Unavailable`] with [`StorageCapability::Runtime`].
///
/// Order holds across handles that are clones of one another: a `read` issued
/// after a `write` observes it without the `write` being awaited.
///
/// # Quotas
///
/// [`StorageLimits`] are enforced against totals computed by one scan of the
/// directory the first time a write needs them, then kept up to date. The
/// directory is assumed to belong to this store: another process writing into
/// it is not noticed until the totals are rebuilt.
///
/// # Examples
///
/// ```no_run
/// use aimer_storage::{FileStorage, Storage};
/// use aimer_venus::Venus;
///
/// let venus = Venus::new();
/// venus.install();
///
/// let storage = FileStorage::for_app("com.example.notes")?;
/// let task_venus = venus.clone();
/// venus.spawn(async move {
///     storage.write("settings", "theme", b"dark").await.unwrap();
///     let theme = storage.read("settings", "theme").await.unwrap();
///     assert_eq!(theme.as_deref(), Some(&b"dark"[..]));
///     # drop(task_venus);
/// });
/// # Ok::<(), aimer_storage::StorageError>(())
/// ```
///
/// [`StorageCapability::Runtime`]: crate::StorageCapability::Runtime
#[derive(Clone)]
pub struct FileStorage {
    inner: Lane<Inner>,
}

impl FileStorage {
    /// Creates a file-backed adapter rooted at `root` with default limits.
    ///
    /// `root` is created by the first write. It should be a directory this
    /// store owns outright; see [`FileStorage::for_app`] for the usual choice.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_limits(root, StorageLimits::default())
    }

    /// Creates a file-backed adapter rooted at `root` with explicit limits.
    pub fn with_limits(root: impl Into<PathBuf>, limits: StorageLimits) -> Self {
        Self {
            inner: Inner::lane(root.into(), limits),
        }
    }

    /// Opens the store an application keeps in its own data directory.
    ///
    /// The root is `storage` inside [`app_data_dir`]`(app_id)`, so the same
    /// `app_id` finds the same data on every launch.
    ///
    /// # Errors
    ///
    /// See [`app_data_dir`].
    pub fn for_app(app_id: &str) -> StorageResult<Self> {
        Self::for_location(app_id, StorageLocation::Data)
    }

    /// [`FileStorage::for_app`] with explicit limits.
    pub fn for_app_with_limits(app_id: &str, limits: StorageLimits) -> StorageResult<Self> {
        Self::for_location_with_limits(app_id, StorageLocation::Data, limits)
    }

    /// Opens the store an application keeps at `location`: persistent data,
    /// roaming data, cache, or scratch space. See [`app_dir`] for where each is.
    ///
    /// # Errors
    ///
    /// See [`app_dir`].
    pub fn for_location(app_id: &str, location: StorageLocation) -> StorageResult<Self> {
        Self::for_location_with_limits(app_id, location, StorageLimits::default())
    }

    /// [`FileStorage::for_location`] with explicit limits.
    pub fn for_location_with_limits(
        app_id: &str,
        location: StorageLocation,
        limits: StorageLimits,
    ) -> StorageResult<Self> {
        Ok(Self::with_limits(
            app_dir(app_id, location)?.join(APP_SUBDIRECTORY),
            limits,
        ))
    }

    /// Returns the directory used by this adapter.
    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// Returns the limits enforced by this adapter.
    pub fn limits(&self) -> StorageLimits {
        self.inner.limits
    }
}

impl fmt::Debug for FileStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileStorage")
            .field("root", &self.inner.root)
            .field("limits", &self.inner.limits)
            .finish()
    }
}

/// What removing one namespace's directory took with it.
struct Cleared {
    count: usize,
    bytes: usize,
}

/// Deletes every value in `directory`, then the directory itself.
///
/// Stray staging files from an interrupted write go too but are not counted:
/// they were never part of the store's usage.
fn clear_directory(directory: &Path) -> StorageResult<Cleared> {
    let operation = StorageOperation::Clear;
    let values = match fs::read_dir(directory) {
        Ok(values) => values,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Cleared { count: 0, bytes: 0 });
        }
        Err(error) => return Err(native_error(operation, error)),
    };

    let corrupt = || StorageError::Corrupt { operation };
    let mut cleared = Cleared { count: 0, bytes: 0 };
    for value_entry in values {
        let value_entry = value_entry.map_err(|error| native_error(operation, error))?;
        let name = value_entry
            .file_name()
            .into_string()
            .map_err(|_| corrupt())?;
        let is_temporary = name.starts_with(layout::TEMP_PREFIX);
        let metadata = value_entry
            .metadata()
            .map_err(|error| native_error(operation, error))?;
        if !metadata.is_file()
            || (!is_temporary && layout::decode_component(&name, operation).is_err())
        {
            return Err(corrupt());
        }
        fs::remove_file(value_entry.path()).map_err(|error| native_error(operation, error))?;
        if !is_temporary {
            cleared.count += 1;
            cleared.bytes += usize::try_from(metadata.len()).map_err(|_| corrupt())?;
        }
    }
    // Not an error if something raced a new value into it.
    let _ = fs::remove_dir(directory);
    Ok(cleared)
}

impl Storage for FileStorage {
    fn read(&self, namespace: &str, key: &str) -> StorageFuture<Option<Vec<u8>>> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let path = value_path(&self.inner.root, namespace, key);
        submit(&self.inner, StorageOperation::Read, false, move |inner| {
            let operation = StorageOperation::Read;
            // Stat first so an oversized file is refused without reading it.
            let Some(size) = stat_value(&path, operation)? else {
                return Ok(None);
            };
            inner.limits.check_value(size)?;
            match fs::read(&path) {
                Ok(value) => Ok(Some(value)),
                // Removed between the stat and the read.
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(native_error(operation, error)),
            }
        })
    }

    fn write(&self, namespace: &str, key: &str, value: &[u8]) -> StorageFuture<()> {
        if let Err(error) =
            validate(namespace, key).and_then(|()| self.inner.limits.check_value(value.len()))
        {
            return ready(Err(error));
        }
        let path = value_path(&self.inner.root, namespace, key);
        let value = value.to_vec();
        submit(&self.inner, StorageOperation::Write, true, move |inner| {
            let operation = StorageOperation::Write;
            let mut guard = lock(&inner.usage);
            let usage = *current_usage(&mut guard, inner, operation)?;
            let old_len = stat_value(&path, operation)?;
            let after = inner.limits.admit_write(usage, old_len, value.len())?;
            if let Err(error) = write_atomically(&path, &value) {
                // The file may or may not have been replaced.
                *guard = None;
                return Err(native_error(operation, error));
            }
            *guard = Some(after);
            Ok(())
        })
    }

    fn remove(&self, namespace: &str, key: &str) -> StorageFuture<bool> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let path = value_path(&self.inner.root, namespace, key);
        submit(&self.inner, StorageOperation::Remove, true, move |inner| {
            let operation = StorageOperation::Remove;
            let Some(size) = stat_value(&path, operation)? else {
                return Ok(false);
            };
            match fs::remove_file(&path) {
                Ok(()) => {
                    if let Some(usage) = lock(&inner.usage).as_mut() {
                        usage.entries = usage.entries.saturating_sub(1);
                        usage.total_bytes = usage.total_bytes.saturating_sub(size);
                    }
                    Ok(true)
                }
                // Someone else removed it first: still gone, but not by us.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    *lock(&inner.usage) = None;
                    Ok(false)
                }
                Err(error) => Err(native_error(operation, error)),
            }
        })
    }

    fn clear(&self, namespace: &str) -> StorageFuture<usize> {
        if let Err(error) = validate_namespace(namespace) {
            return ready(Err(error));
        }
        let directory = namespace_path(&self.inner.root, namespace);
        submit(&self.inner, StorageOperation::Clear, true, move |inner| {
            match clear_directory(&directory) {
                Ok(Cleared { count, bytes }) => {
                    if let Some(usage) = lock(&inner.usage).as_mut() {
                        usage.entries = usage.entries.saturating_sub(count);
                        usage.total_bytes = usage.total_bytes.saturating_sub(bytes);
                    }
                    Ok(count)
                }
                Err(error) => {
                    // Some files may already be gone.
                    *lock(&inner.usage) = None;
                    Err(error)
                }
            }
        })
    }

    fn list(&self, namespace: &str, limit: usize) -> StorageFuture<StorageList> {
        if let Err(error) =
            validate_namespace(namespace).and_then(|()| self.inner.limits.check_list(limit))
        {
            return ready(Err(error));
        }
        let namespace = namespace.to_owned();
        submit(&self.inner, StorageOperation::List, false, move |inner| {
            let entries = namespace_entries(&inner.root, &namespace, StorageOperation::List)?;
            Ok(StorageList::from_sorted(entries, limit))
        })
    }

    fn metadata(&self, namespace: &str, key: &str) -> StorageFuture<Option<StorageMetadata>> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let path = value_path(&self.inner.root, namespace, key);
        submit(&self.inner, StorageOperation::Metadata, false, move |_| {
            Ok(stat_value(&path, StorageOperation::Metadata)?.map(StorageMetadata::new))
        })
    }

    fn commit(&self, batch: StorageBatch) -> StorageFuture<()> {
        if let Err(error) = batch.validate(self.inner.limits) {
            return ready(Err(error));
        }
        if batch.is_empty() {
            return ready(Ok(()));
        }
        submit(&self.inner, StorageOperation::Write, true, move |inner| {
            commit_batch(inner, &batch)
        })
    }
}
