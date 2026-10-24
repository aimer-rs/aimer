//! A whole app model stored as one versioned, atomically-replaced record.

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::rc::Rc;

use aimer_storage::{
    Storage, StorageBatch, StorageError, StorageMigration, StorageOperation, StorageResult,
    migrate_bytes,
};

use crate::error::{PersistError, PersistFuture};
use crate::value::{DecodeError, Encoder, StorageValue, from_bytes};

const NAMESPACE: &str = "documents";

/// A migration written as a closure.
struct FnMigration<F> {
    from: u32,
    to: u32,
    migrate: F,
}

impl<F: Fn(&[u8]) -> Result<Vec<u8>, DecodeError>> StorageMigration for FnMigration<F> {
    fn from_version(&self) -> u32 {
        self.from
    }

    fn to_version(&self) -> u32 {
        self.to
    }

    fn migrate(&self, value: &[u8]) -> StorageResult<Vec<u8>> {
        (self.migrate)(value).map_err(|_| StorageError::Corrupt {
            operation: StorageOperation::Read,
        })
    }
}

/// One named, versioned value of type `T` in a store.
///
/// A document is saved and loaded whole: `save` replaces the stored record
/// atomically (a crash leaves the old document or the new one), and on a
/// persistent store a saved document is durable once the future resolves. Several
/// documents that must change together are saved with one
/// [`stage`](Self::stage)d [`StorageBatch`].
///
/// # Versions and migration
///
/// The stored bytes begin with the document's schema version. Declare the
/// current one with [`version`](Self::version) and, whenever the layout of `T`
/// changes, bump it and add a [`migrate`](Self::migrate) step from the previous
/// version, which rewrites the *old* payload bytes into the new layout. `load`
/// applies the steps it needs, in order. Data written by a newer app is refused
/// with [`PersistError::NewerVersion`] rather than guessed at.
///
/// Loading does not write the migrated data back; the next `save` does.
///
/// # Examples
///
/// ```
/// use std::rc::Rc;
/// use aimer_persist::{Document, impl_storage_value};
/// use aimer_storage::MemoryStorage;
///
/// #[derive(Clone, Debug, PartialEq)]
/// struct Notes { titles: Vec<String> }
/// impl_storage_value!(Notes { titles });
///
/// let store = Rc::new(MemoryStorage::new());
/// let notes = Document::<Notes>::new(store, "notes");
/// // `notes.save(&Notes { .. })` and `notes.load()` return futures; await them
/// // from a Venus task.
/// # let _ = notes;
/// ```
pub struct Document<T> {
    storage: Rc<dyn Storage>,
    namespace: String,
    key: String,
    version: u32,
    migrations: Vec<Rc<dyn StorageMigration>>,
    marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Document<T> {
    fn clone(&self) -> Self {
        Self {
            storage: Rc::clone(&self.storage),
            namespace: self.namespace.clone(),
            key: self.key.clone(),
            version: self.version,
            migrations: self.migrations.clone(),
            marker: PhantomData,
        }
    }
}

impl<T: StorageValue> Document<T> {
    /// Creates the document called `name` in `storage`, at schema version 1.
    ///
    /// `name` follows the storage key rules: non-empty, at most 127 bytes, no
    /// path separators or control characters. It is checked when the document is
    /// first used, and reported as an error then.
    pub fn new(storage: Rc<dyn Storage>, name: &str) -> Self {
        Self::in_namespace(storage, NAMESPACE, name)
    }

    pub(crate) fn in_namespace(storage: Rc<dyn Storage>, namespace: &str, key: &str) -> Self {
        Self {
            storage,
            namespace: namespace.to_owned(),
            key: key.to_owned(),
            version: 1,
            migrations: Vec::new(),
            marker: PhantomData,
        }
    }

    /// Declares the current schema version of `T`. The default is 1.
    pub fn version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }

    /// Adds the step that rewrites a version `from` payload into version `to`.
    ///
    /// `migrate` receives the old payload bytes, without the version header, and
    /// returns the new payload bytes. Steps are chained by version: with steps
    /// 1→2 and 2→3, a version 1 document is migrated twice. `to` must be greater
    /// than `from` and no greater than the current version.
    pub fn migrate(
        mut self,
        from: u32,
        to: u32,
        migrate: impl Fn(&[u8]) -> Result<Vec<u8>, DecodeError> + 'static,
    ) -> Self {
        self.migrations
            .push(Rc::new(FnMigration { from, to, migrate }));
        self
    }

    /// Adds a [`StorageMigration`] implementation as a step.
    pub fn migration(mut self, migration: impl StorageMigration + 'static) -> Self {
        self.migrations.push(Rc::new(migration));
        self
    }

    fn encode(&self, value: &T) -> Vec<u8> {
        let mut encoder = Encoder::new();
        encoder.value(&self.version).value(value);
        encoder.into_bytes()
    }

    /// Loads the document, or `None` if it was never saved.
    ///
    /// # Errors
    ///
    /// * [`PersistError::NewerVersion`] when it was saved by a newer app.
    /// * [`PersistError::Decode`] when the bytes are not a valid `T`.
    /// * [`PersistError::Storage`] on I/O failure or a missing migration step.
    pub fn load(&self) -> PersistFuture<Option<T>>
    where
        T: 'static,
    {
        // Issued now, not on first poll, so it is ordered after earlier saves.
        let read = self.storage.read(&self.namespace, &self.key);
        let (current, migrations) = (self.version, self.migrations.clone());
        Box::pin(async move {
            let Some(bytes) = read.await? else {
                return Ok(None);
            };
            let (header, payload) = bytes
                .split_first_chunk::<4>()
                .ok_or(DecodeError::UnexpectedEnd)?;
            let found = u32::from_le_bytes(*header);
            if found > current {
                return Err(PersistError::NewerVersion {
                    found,
                    supported: current,
                });
            }
            if found == current {
                return Ok(Some(from_bytes(payload)?));
            }
            let steps: Vec<&dyn StorageMigration> = migrations.iter().map(|step| &**step).collect();
            let migrated = migrate_bytes(payload, found, current, &steps)?;
            Ok(Some(from_bytes(&migrated)?))
        })
    }

    /// Replaces the stored document with `value`.
    ///
    /// The value is encoded when this is called, so changing it afterwards does
    /// not change what is saved.
    pub fn save(&self, value: &T) -> PersistFuture<()> {
        let write = self
            .storage
            .write(&self.namespace, &self.key, &self.encode(value));
        Box::pin(async move { Ok(write.await?) })
    }

    /// Adds saving `value` to `batch`, so it commits atomically with the other
    /// entries, whether other documents or anything else in the same store.
    pub fn stage(&self, batch: &mut StorageBatch, value: &T) {
        batch.write(&self.namespace, &self.key, &self.encode(value));
    }

    /// Deletes the stored document. Reports whether there was one.
    pub fn remove(&self) -> PersistFuture<bool> {
        let remove = self.storage.remove(&self.namespace, &self.key);
        Box::pin(async move { Ok(remove.await?) })
    }
}

/// The map type preferences are stored as.
pub(crate) type EncodedMap = BTreeMap<String, Vec<u8>>;
