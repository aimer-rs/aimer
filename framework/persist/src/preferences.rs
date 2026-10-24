//! Small settings, read synchronously and saved in the background.
//!
//! The counterpart of `UserDefaults`, Android `SharedPreferences`, Windows
//! `ApplicationData.LocalSettings` and the browser's `localStorage`: a flat map
//! of named values that code reads and writes without awaiting, which survives
//! restarts.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use aimer_storage::Storage;

use crate::document::{Document, EncodedMap};
use crate::error::{PersistError, PersistFuture};
use crate::value::{StorageValue, from_bytes, to_bytes};
use crate::write_behind::WriteBehind;

const NAMESPACE: &str = "preferences";
const KEY: &str = "all";

type Callback = Rc<RefCell<dyn FnMut(&str)>>;

struct Listener {
    id: u64,
    /// `None` listens to every key.
    key: Option<String>,
    callback: Callback,
}

struct Shared {
    /// Shared with the saver, which owns its half outright: the saver must be
    /// able to write the map after every `Preferences` handle is gone, or a
    /// change made just before the last handle was dropped would be lost.
    values: Rc<RefCell<EncodedMap>>,
    listeners: RefCell<Vec<Listener>>,
    next_listener: Cell<u64>,
    saver: WriteBehind,
}

/// A persisted map of named settings.
///
/// Values are encoded with [`StorageValue`], so a preference can be any
/// supported type, not only text. The whole map is held in memory: reads
/// ([`get`](Self::get)) and writes ([`set`](Self::set)) are synchronous and
/// cheap. Writes are saved in the background, coalesced, as one atomic record,
/// so a burst of changes costs one save.
///
/// Unsaved changes are not lost when the app goes to the background, quits, or
/// is told to terminate: the event loop reports those moments and every pending
/// save is finished before it carries on (see
/// [`flush_all_blocking`](crate::flush_all_blocking)), and a Rust panic on any
/// thread saves them too ([`set_flush_on_panic`](crate::set_flush_on_panic)).
/// Only a kill the process cannot observe (`SIGKILL`, an out-of-memory kill, a
/// native crash) can lose the changes of the last few milliseconds. Call
/// [`flush`](Self::flush) yourself only to wait for a save, for example before
/// showing "saved".
///
/// The map is stored as a single record, which keeps a reload to one read and
/// keeps related settings consistent with each other, and bounds the total size
/// to the store's value limit (1 MiB by default). Large or numerous items belong
/// in a [`Document`] or directly in storage.
///
/// Cloning a `Preferences` shares the same map.
///
/// # Examples
///
/// ```
/// use std::rc::Rc;
/// use aimer_persist::Preferences;
/// use aimer_storage::MemoryStorage;
/// # use std::task::{Context, Poll, Waker};
/// # use std::future::Future;
///
/// # let mut load = Preferences::load(Rc::new(MemoryStorage::new()));
/// # let Poll::Ready(prefs) = load.as_mut().poll(&mut Context::from_waker(Waker::noop())) else { panic!() };
/// # let prefs = prefs?;
/// assert_eq!(prefs.get::<bool>("dark_mode")?, None);
/// prefs.set("dark_mode", &true);
/// assert_eq!(prefs.get::<bool>("dark_mode")?, Some(true));
/// assert!(prefs.get_or("volume", 0.8f32) > 0.79);
/// # Ok::<(), aimer_persist::PersistError>(())
/// ```
#[derive(Clone)]
pub struct Preferences {
    shared: Rc<Shared>,
}

impl Preferences {
    /// Loads the preferences stored in `storage`; empty if there are none yet.
    ///
    /// # Errors
    ///
    /// [`PersistError`] if the stored map cannot be read or decoded. A damaged
    /// map is reported rather than discarded, so the app can choose to start
    /// over (`Preferences::empty`) knowingly.
    pub fn load(storage: Rc<dyn Storage>) -> PersistFuture<Self> {
        let document = Self::document(storage);
        let load = document.load();
        Box::pin(async move { Ok(Self::with(document, load.await?.unwrap_or_default())) })
    }

    /// Starts with no preferences, ignoring whatever is stored. The first save
    /// replaces it.
    pub fn empty(storage: Rc<dyn Storage>) -> Self {
        Self::with(Self::document(storage), EncodedMap::new())
    }

    fn document(storage: Rc<dyn Storage>) -> Document<EncodedMap> {
        Document::in_namespace(storage, NAMESPACE, KEY)
    }

    fn with(document: Document<EncodedMap>, values: EncodedMap) -> Self {
        let values = Rc::new(RefCell::new(values));
        // The saver holds the map, not the other way round, so there is no cycle
        // and nothing for a dropped `Preferences` to take down with it.
        let saved = Rc::clone(&values);
        Self {
            shared: Rc::new(Shared {
                values,
                listeners: RefCell::new(Vec::new()),
                next_listener: Cell::new(0),
                saver: WriteBehind::new(Box::new(move || {
                    // `try_borrow`: a panic handler may run while a `set` holds the map.
                    let values = saved.try_borrow().ok()?;
                    Some(document.save(&values))
                })),
            }),
        }
    }

    /// Reads a preference. `None` if it was never set.
    ///
    /// # Errors
    ///
    /// [`PersistError::Decode`] if the stored value is not a `T`, such as after
    /// reading it as a different type than it was written.
    pub fn get<T: StorageValue>(&self, key: &str) -> Result<Option<T>, PersistError> {
        match self.shared.values.borrow().get(key) {
            Some(bytes) => Ok(Some(from_bytes(bytes)?)),
            None => Ok(None),
        }
    }

    /// Reads a preference, or `default` if it is unset or is not a `T`.
    pub fn get_or<T: StorageValue>(&self, key: &str, default: T) -> T {
        self.get(key).ok().flatten().unwrap_or(default)
    }

    /// Sets a preference and schedules it to be saved.
    ///
    /// Listeners run only when the value actually changed. Returns whether it did.
    pub fn set<T: StorageValue>(&self, key: &str, value: &T) -> bool {
        let bytes = to_bytes(value);
        {
            let mut values = self.shared.values.borrow_mut();
            match values.get_mut(key) {
                Some(existing) if *existing == bytes => return false,
                Some(existing) => *existing = bytes,
                None => {
                    values.insert(key.to_owned(), bytes);
                }
            }
        }
        self.changed(key);
        true
    }

    /// Removes a preference and schedules the removal to be saved. Reports
    /// whether it existed.
    pub fn remove(&self, key: &str) -> bool {
        let existed = self.shared.values.borrow_mut().remove(key).is_some();
        if existed {
            self.changed(key);
        }
        existed
    }

    /// Whether `key` is set.
    pub fn contains(&self, key: &str) -> bool {
        self.shared.values.borrow().contains_key(key)
    }

    /// The names of every set preference, in order.
    pub fn keys(&self) -> Vec<String> {
        self.shared.values.borrow().keys().cloned().collect()
    }

    /// The number of set preferences.
    pub fn len(&self) -> usize {
        self.shared.values.borrow().len()
    }

    /// Whether no preference is set.
    pub fn is_empty(&self) -> bool {
        self.shared.values.borrow().is_empty()
    }

    fn changed(&self, key: &str) {
        self.shared.saver.mark_dirty();
        // Collected first: a callback is free to set or watch preferences.
        let callbacks: Vec<Callback> = self
            .shared
            .listeners
            .borrow()
            .iter()
            .filter(|listener| listener.key.as_deref().is_none_or(|watched| watched == key))
            .map(|listener| Rc::clone(&listener.callback))
            .collect();
        for callback in callbacks {
            // A callback that is already running and caused this change is not
            // re-entered; it has seen the new value by definition.
            if let Ok(mut callback) = callback.try_borrow_mut() {
                callback(key);
            }
        }
    }

    /// Calls `callback` whenever `key` changes, until the returned
    /// [`Subscription`] is dropped.
    pub fn watch(&self, key: &str, mut callback: impl FnMut() + 'static) -> Subscription {
        self.listen(Some(key.to_owned()), move |_| callback())
    }

    /// Calls `callback` with the name of every preference that changes, until
    /// the returned [`Subscription`] is dropped.
    pub fn watch_all(&self, callback: impl FnMut(&str) + 'static) -> Subscription {
        self.listen(None, callback)
    }

    fn listen(&self, key: Option<String>, callback: impl FnMut(&str) + 'static) -> Subscription {
        let id = self.shared.next_listener.get();
        self.shared.next_listener.set(id + 1);
        self.shared.listeners.borrow_mut().push(Listener {
            id,
            key,
            callback: Rc::new(RefCell::new(callback)),
        });
        Subscription {
            shared: Rc::downgrade(&self.shared),
            id,
        }
    }

    /// A typed handle to one preference with a default.
    pub fn preference<T: StorageValue + Clone>(&self, key: &str, default: T) -> Preference<T> {
        Preference {
            preferences: self.clone(),
            key: key.to_owned(),
            default,
        }
    }

    /// Resolves when every change made so far has been saved.
    ///
    /// # Errors
    ///
    /// The error of the last failed save, such as a map larger than the store's
    /// value limit or a disk error.
    pub fn flush(&self) -> PersistFuture<()> {
        self.shared.saver.flush()
    }

    /// Whether a change has not yet been handed to storage.
    pub fn has_unsaved_changes(&self) -> bool {
        self.shared.saver.is_dirty()
    }

    /// The error of the most recent background save, if it failed.
    pub fn last_save_error(&self) -> Option<PersistError> {
        self.shared.saver.last_error()
    }
}

/// Keeps a [`Preferences::watch`] callback registered; dropping it unregisters.
#[must_use = "the callback is removed as soon as the subscription is dropped"]
pub struct Subscription {
    shared: Weak<Shared>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            shared
                .listeners
                .borrow_mut()
                .retain(|listener| listener.id != self.id);
        }
    }
}

/// One preference with a default, read and written like a variable.
///
/// Created with [`Preferences::preference`]. Reading never fails: an unset or
/// undecodable value reads as the default.
#[derive(Clone)]
pub struct Preference<T> {
    preferences: Preferences,
    key: String,
    default: T,
}

impl<T: StorageValue + Clone> Preference<T> {
    /// The current value, or the default.
    pub fn get(&self) -> T {
        self.preferences.get_or(&self.key, self.default.clone())
    }

    /// Sets the value. Reports whether it changed.
    pub fn set(&self, value: &T) -> bool {
        self.preferences.set(&self.key, value)
    }

    /// Returns to the default by removing the stored value.
    pub fn reset(&self) -> bool {
        self.preferences.remove(&self.key)
    }

    /// Calls `callback` whenever this preference changes.
    pub fn watch(&self, callback: impl FnMut() + 'static) -> Subscription {
        self.preferences.watch(&self.key, callback)
    }
}
