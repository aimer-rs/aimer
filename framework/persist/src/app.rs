//! The entry point: an app's storage, by location, on every platform.

use std::cell::RefCell;
use std::rc::Rc;

use aimer_storage::{MemoryStorage, Storage, StorageLocation};

use crate::document::Document;
use crate::error::{PersistError, PersistFuture};
use crate::preferences::Preferences;
use crate::value::StorageValue;

/// How the stores behind an [`AppStorage`] are made.
#[derive(Clone, Copy)]
enum Backend {
    /// Files on native platforms, IndexedDB in the browser.
    Platform,
    /// Memory only, for tests and previews.
    Memory,
}

/// An application's persistent storage, the same API on every platform.
///
/// One `AppStorage` per app (it is cheap to clone; clones share their stores).
/// It resolves where each [`StorageLocation`] lives on the current platform and
/// hands out the pieces apps are built from:
///
/// | Need | Use | Backed by |
/// |---|---|---|
/// | A few settings, read without awaiting | [`preferences`](Self::preferences) | one record |
/// | An app model, versioned | [`document`](Self::document) | one record |
/// | State widgets read, restored at launch | [`Persisted`](crate::Persisted) over a document | one record |
/// | Anything else | [`data`](Self::data) / [`at`](Self::at) | the raw [`Storage`] |
///
/// | Platform | Persistent data | Cache |
/// |---|---|---|
/// | Windows | `%LOCALAPPDATA%` (roaming: `%APPDATA%`) | `%LOCALAPPDATA%\<id>\Cache` |
/// | macOS, iOS | `~/Library/Application Support` | `~/Library/Caches` |
/// | Android | the app's `files` directory | its `cache` directory |
/// | Linux | `$XDG_DATA_HOME` | `$XDG_CACHE_HOME` |
/// | Web | IndexedDB | a second IndexedDB database |
///
/// Every operation is asynchronous and must be awaited by a Venus task, with a
/// Venus runtime installed (see `aimer_storage::FileStorage`); preferences are
/// the exception, which are read and written synchronously once loaded.
///
/// # Examples
///
/// ```no_run
/// use aimer_persist::AppStorage;
///
/// let app = AppStorage::new("com.example.notes")?;
/// let settings = app.preferences();          // a future: load once at startup
/// let notes = app.document::<Vec<String>>("notes");
/// # let _ = (settings, notes);
/// # Ok::<(), aimer_persist::PersistError>(())
/// ```
#[derive(Clone)]
pub struct AppStorage {
    inner: Rc<Inner>,
}

struct Inner {
    app_id: String,
    backend: Backend,
    data: Rc<dyn Storage>,
    others: RefCell<Vec<(StorageLocation, Rc<dyn Storage>)>>,
}

impl AppStorage {
    /// Opens the storage of the app named `app_id`, such as `com.example.notes`.
    ///
    /// # Errors
    ///
    /// [`PersistError::Storage`] if `app_id` is not a valid name, or the platform
    /// cannot say where the app's data goes: an Android host that has not given
    /// its data directory to `aimer_storage`, or an unset `HOME`.
    pub fn new(app_id: &str) -> Result<Self, PersistError> {
        let data = create(Backend::Platform, app_id, StorageLocation::Data)?;
        Ok(Self::with(app_id, Backend::Platform, data))
    }

    /// Storage that lives in memory only, for tests and design previews. It
    /// behaves like the real thing, including quotas, and forgets everything when
    /// dropped.
    pub fn in_memory(app_id: &str) -> Self {
        Self::with(app_id, Backend::Memory, Rc::new(MemoryStorage::new()))
    }

    fn with(app_id: &str, backend: Backend, data: Rc<dyn Storage>) -> Self {
        Self {
            inner: Rc::new(Inner {
                app_id: app_id.to_owned(),
                backend,
                data,
                others: RefCell::new(Vec::new()),
            }),
        }
    }

    /// The app's identifier.
    pub fn app_id(&self) -> &str {
        &self.inner.app_id
    }

    /// The raw store for `location`, created on first use and shared afterwards.
    ///
    /// Sharing matters: one store orders its operations and accounts its quota,
    /// which two separate handles on the same place could not.
    ///
    /// # Errors
    ///
    /// [`PersistError::Storage`] if the platform cannot provide that location.
    pub fn at(&self, location: StorageLocation) -> Result<Rc<dyn Storage>, PersistError> {
        let location = self.canonical(location);
        if location == StorageLocation::Data {
            return Ok(Rc::clone(&self.inner.data));
        }
        if let Some((_, store)) = self
            .inner
            .others
            .borrow()
            .iter()
            .find(|(existing, _)| *existing == location)
        {
            return Ok(Rc::clone(store));
        }
        let store = create(self.inner.backend, &self.inner.app_id, location)?;
        self.inner
            .others
            .borrow_mut()
            .push((location, Rc::clone(&store)));
        Ok(store)
    }

    /// Where `Roaming` is the same place as `Data`, it is the same store.
    fn canonical(&self, location: StorageLocation) -> StorageLocation {
        if location == StorageLocation::Roaming && !roams_separately(&self.inner.app_id) {
            StorageLocation::Data
        } else {
            location
        }
    }

    /// The persistent data store: for things the app cannot recreate.
    pub fn data(&self) -> Rc<dyn Storage> {
        Rc::clone(&self.inner.data)
    }

    /// The cache store: for things the app can rebuild. The system may delete it.
    ///
    /// # Errors
    ///
    /// See [`at`](Self::at).
    pub fn cache(&self) -> Result<Rc<dyn Storage>, PersistError> {
        self.at(StorageLocation::Cache)
    }

    /// The scratch store: gone by the next launch at the latest.
    ///
    /// # Errors
    ///
    /// See [`at`](Self::at).
    pub fn temp(&self) -> Result<Rc<dyn Storage>, PersistError> {
        self.at(StorageLocation::Temp)
    }

    /// Loads the app's preferences from persistent data.
    ///
    /// Do this once at startup and keep the result: it holds the settings in
    /// memory. Each call loads a separate copy.
    pub fn preferences(&self) -> PersistFuture<Preferences> {
        Preferences::load(self.data())
    }

    /// The document called `name` in persistent data.
    ///
    /// Documents with different names are independent.
    pub fn document<T: StorageValue>(&self, name: &str) -> Document<T> {
        Document::new(self.data(), name)
    }

    /// The document called `name` at `location`.
    ///
    /// # Errors
    ///
    /// See [`at`](Self::at).
    pub fn document_at<T: StorageValue>(
        &self,
        location: StorageLocation,
        name: &str,
    ) -> Result<Document<T>, PersistError> {
        Ok(Document::new(self.at(location)?, name))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn roams_separately(app_id: &str) -> bool {
    use aimer_storage::app_dir;

    match (
        app_dir(app_id, StorageLocation::Roaming),
        app_dir(app_id, StorageLocation::Data),
    ) {
        (Ok(roaming), Ok(data)) => roaming != data,
        _ => false,
    }
}

#[cfg(target_arch = "wasm32")]
fn roams_separately(_app_id: &str) -> bool {
    false
}

fn create(
    backend: Backend,
    app_id: &str,
    location: StorageLocation,
) -> Result<Rc<dyn Storage>, PersistError> {
    if let Backend::Memory = backend {
        return Ok(Rc::new(MemoryStorage::new()));
    }
    platform_store(app_id, location)
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_store(
    app_id: &str,
    location: StorageLocation,
) -> Result<Rc<dyn Storage>, PersistError> {
    Ok(Rc::new(aimer_storage::FileStorage::for_location(
        app_id, location,
    )?))
}

#[cfg(target_arch = "wasm32")]
fn platform_store(
    app_id: &str,
    location: StorageLocation,
) -> Result<Rc<dyn Storage>, PersistError> {
    Ok(match location {
        // Scratch data on the web is plain memory.
        StorageLocation::Temp => Rc::new(MemoryStorage::new()),
        _ => Rc::new(aimer_storage::IndexedDbStorage::for_location(
            app_id, location,
        )?),
    })
}
