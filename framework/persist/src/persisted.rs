//! Provider state that survives restarts.

use std::cell::RefCell;
use std::rc::Rc;

use aimer_provider::{Observation, ProviderHandle, Snapshot};

use crate::document::Document;
use crate::error::{PersistError, PersistFuture};
use crate::value::StorageValue;
use crate::write_behind::WriteBehind;

/// A provider value that is loaded from storage at startup and saved whenever
/// it changes.
///
/// It wraps a [`ProviderHandle`]: give that handle to a provider and the whole
/// widget tree reads and updates the value as usual, with no persistence code at
/// the call sites.
///
/// ```no_run
/// use aimer_persist::{AppStorage, Persisted, impl_storage_value};
///
/// #[derive(Clone)]
/// struct Settings { volume: f32 }
/// impl_storage_value!(Settings { volume });
///
/// # async fn startup() -> Result<(), aimer_persist::PersistError> {
/// let app = AppStorage::new("com.example.player")?;
/// let settings = Persisted::load_or_default(app.document("settings"), || Settings { volume: 0.8 }).await?;
///
/// // Give this to `Provider::new().handle(handle)`; widgets below it read and
/// // update it through `ProviderHandle::<Settings>::of(context)`.
/// let handle = settings.handle();
/// handle.update(|settings| settings.volume = 0.5); // saved in the background
///
/// settings.flush().await?; // optional: wait until it is on disk
/// # Ok(())
/// # }
/// ```
///
/// Every update, from any clone of the handle, marks the value dirty. Saves run
/// in the background at idle time, one at a time, and updates that arrive
/// meanwhile collapse into the next save, so a slider dragged through a hundred
/// values costs a handful of writes. The value is encoded when a save begins, so
/// each save is a consistent snapshot.
///
/// Dropping a `Persisted` stops observing, but changes already made are still
/// saved. Unsaved changes are also finished when the app goes to the background,
/// quits, or is told to terminate (see
/// [`flush_all_blocking`](crate::flush_all_blocking)), so [`flush`](Self::flush)
/// is only needed to wait for a save.
pub struct Persisted<T: 'static> {
    handle: ProviderHandle<T>,
    saver: WriteBehind,
    _observation: Observation,
}

impl<T: StorageValue + Clone + 'static> Persisted<T> {
    /// Loads the stored value, or builds one with `default` if nothing is stored.
    ///
    /// # Errors
    ///
    /// Any [`PersistError`] from [`Document::load`], including a stored value
    /// that cannot be decoded. Use [`load_or_default`](Self::load_or_default) to
    /// start over instead in that case.
    pub fn load(
        document: Document<T>,
        default: impl FnOnce() -> T + 'static,
    ) -> PersistFuture<Self> {
        let load = document.load();
        Box::pin(async move {
            let value = load.await?.unwrap_or_else(default);
            Ok(Self::attach(document, value))
        })
    }

    /// Like [`load`](Self::load), but a stored value that cannot be decoded is
    /// replaced by `default` instead of failing.
    ///
    /// Only a damaged value is replaced. Data from a newer app
    /// ([`PersistError::NewerVersion`]) and storage failures are still errors,
    /// because overwriting them would destroy data that is merely unreadable
    /// *right now*.
    pub fn load_or_default(
        document: Document<T>,
        default: impl FnOnce() -> T + 'static,
    ) -> PersistFuture<Self> {
        let load = document.load();
        Box::pin(async move {
            let value = match load.await {
                Ok(stored) => stored.unwrap_or_else(default),
                Err(PersistError::Decode(_)) => default(),
                Err(error) => return Err(error),
            };
            Ok(Self::attach(document, value))
        })
    }

    /// Wraps `value` as the starting state, saving it to `document` from now on.
    /// The starting value is not saved until it changes.
    pub fn attach(document: Document<T>, value: T) -> Self {
        let handle = ProviderHandle::new(value);
        // The observer hands the newest snapshot to the saver rather than the
        // saver reading the handle, which would be a reference cycle through the
        // handle's own observer list.
        let latest: Rc<RefCell<Option<Snapshot<T>>>> = Rc::new(RefCell::new(None));

        // Cloned, not taken: a save started by a panic handler needs the state to
        // still be there even though a normal save already took its turn, and it
        // must not panic if the panic that brought it here left this borrowed.
        let read = Rc::clone(&latest);
        let saver = WriteBehind::new(Box::new(move || {
            let snapshot = read.try_borrow().ok()?.clone()?;
            Some(document.save(&snapshot))
        }));

        let marked = saver.clone();
        let observation = handle.observe(move |snapshot| {
            *latest.borrow_mut() = Some(snapshot.clone());
            marked.mark_dirty();
        });

        Self {
            handle,
            saver,
            _observation: observation,
        }
    }

    /// The handle to provide to the widget tree, or to update directly.
    pub fn handle(&self) -> ProviderHandle<T> {
        self.handle.clone()
    }

    /// Resolves when every update made so far has been saved.
    ///
    /// # Errors
    ///
    /// The error of the last failed save.
    pub fn flush(&self) -> PersistFuture<()> {
        self.saver.flush()
    }

    /// Whether an update has not yet been handed to storage.
    pub fn has_unsaved_changes(&self) -> bool {
        self.saver.is_dirty()
    }

    /// The error of the most recent background save, if it failed.
    pub fn last_save_error(&self) -> Option<PersistError> {
        self.saver.last_error()
    }
}
