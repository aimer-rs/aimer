//! The persistence layer through its public API, against in-memory storage and a
//! real Venus runtime, the way an app drives it.

#![cfg(not(target_arch = "wasm32"))]

use std::cell::Cell;
use std::future::Future;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use aimer_time::{Duration, Instant};

use aimer_persist::{
    AppStorage, DecodeError, Document, PersistError, Persisted, Preferences, StorageLocation,
    from_bytes, impl_storage_value, to_bytes,
};
use aimer_storage::{
    MemoryStorage, QuotaKind, Storage, StorageBatch, StorageError, StorageFuture, StorageLimits,
    StorageList, StorageMetadata,
};
use aimer_venus::{FrameBudget, Venus};

/// Runs `future` to completion, giving Venus its turns in between, the way the
/// event loop does between awaits.
fn run<F: Future>(venus: Option<&Rc<Venus>>, future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = Box::pin(future);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        assert!(Instant::now() < deadline, "the future never completed");
        pump(venus);
    }
}

fn pump(venus: Option<&Rc<Venus>>) {
    if let Some(venus) = venus {
        venus.run_microtasks();
        venus.run_idle(&FrameBudget::from_now(Duration::from_millis(20)));
    }
}

fn pump_until(venus: &Rc<Venus>, what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        pump(Some(venus));
    }
}

/// Installs a Venus runtime for the test and removes it afterwards.
struct Runtime(Rc<Venus>);

impl Runtime {
    fn new() -> Self {
        let venus = Venus::new();
        venus.install();
        Self(venus)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        Venus::uninstall();
    }
}

/// A store that counts the writes that reach it.
#[derive(Clone)]
struct Counting {
    inner: MemoryStorage,
    writes: Rc<Cell<usize>>,
}

impl Counting {
    fn new() -> Self {
        Self::with_limits(StorageLimits::default())
    }

    fn with_limits(limits: StorageLimits) -> Self {
        Self {
            inner: MemoryStorage::with_limits(limits),
            writes: Rc::new(Cell::new(0)),
        }
    }

    fn writes(&self) -> usize {
        self.writes.get()
    }
}

impl Storage for Counting {
    fn read(&self, namespace: &str, key: &str) -> StorageFuture<Option<Vec<u8>>> {
        self.inner.read(namespace, key)
    }

    fn write(&self, namespace: &str, key: &str, value: &[u8]) -> StorageFuture<()> {
        self.writes.set(self.writes.get() + 1);
        self.inner.write(namespace, key, value)
    }

    fn remove(&self, namespace: &str, key: &str) -> StorageFuture<bool> {
        self.inner.remove(namespace, key)
    }

    fn clear(&self, namespace: &str) -> StorageFuture<usize> {
        self.inner.clear(namespace)
    }

    fn list(&self, namespace: &str, limit: usize) -> StorageFuture<StorageList> {
        self.inner.list(namespace, limit)
    }

    fn metadata(&self, namespace: &str, key: &str) -> StorageFuture<Option<StorageMetadata>> {
        self.inner.metadata(namespace, key)
    }

    fn commit(&self, batch: StorageBatch) -> StorageFuture<()> {
        self.writes.set(self.writes.get() + 1);
        self.inner.commit(batch)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Notes {
    titles: Vec<String>,
    pinned: Option<String>,
}
impl_storage_value!(Notes { titles, pinned });

fn notes(titles: &[&str]) -> Notes {
    Notes {
        titles: titles.iter().map(|title| (*title).to_owned()).collect(),
        pinned: None,
    }
}

// ───────────────────────────── documents ─────────────────────────────

#[test]
fn a_document_round_trips_and_is_absent_until_saved() {
    let store = Rc::new(MemoryStorage::new());
    let document = Document::<Notes>::new(store, "notes");

    assert_eq!(run(None, document.load()), Ok(None));
    run(None, document.save(&notes(&["a", "b"]))).unwrap();
    assert_eq!(run(None, document.load()), Ok(Some(notes(&["a", "b"]))));

    assert_eq!(run(None, document.remove()), Ok(true));
    assert_eq!(run(None, document.load()), Ok(None));
    assert_eq!(run(None, document.remove()), Ok(false));
}

#[test]
fn documents_with_different_names_are_independent() {
    let store: Rc<dyn Storage> = Rc::new(MemoryStorage::new());
    let first = Document::<u32>::new(Rc::clone(&store), "first");
    let second = Document::<u32>::new(store, "second");

    run(None, first.save(&1)).unwrap();
    assert_eq!(run(None, second.load()), Ok(None));
}

#[test]
fn a_save_captures_the_value_when_it_is_called() {
    let store = Rc::new(MemoryStorage::new());
    let document = Document::<Notes>::new(store, "notes");

    let mut value = notes(&["before"]);
    let save = document.save(&value);
    value.titles.push("after".into());
    run(None, save).unwrap();

    assert_eq!(run(None, document.load()), Ok(Some(notes(&["before"]))));
}

#[test]
fn a_load_issued_after_an_unawaited_save_sees_it() {
    let store = Rc::new(MemoryStorage::new());
    let document = Document::<u32>::new(store, "counter");

    let save = document.save(&7);
    let load = document.load();
    assert_eq!(run(None, load), Ok(Some(7)));
    run(None, save).unwrap();
}

// A document's layout over time. Version 1 stored only titles.
#[derive(Debug, PartialEq)]
struct NotesV1 {
    titles: Vec<String>,
}
impl_storage_value!(NotesV1 { titles });

#[test]
fn old_documents_are_migrated_through_every_step() {
    let store: Rc<dyn Storage> = Rc::new(MemoryStorage::new());

    // Written by the first release.
    let v1 = Document::<NotesV1>::new(Rc::clone(&store), "notes");
    run(
        None,
        v1.save(&NotesV1 {
            titles: vec!["a".into()],
        }),
    )
    .unwrap();

    // The current release is version 3, via an intermediate version 2 that added
    // a count, which version 3 then dropped in favour of `pinned`.
    let current = Document::<Notes>::new(Rc::clone(&store), "notes")
        .version(3)
        .migrate(1, 2, |old| {
            let titles: Vec<String> = from_bytes(old)?;
            Ok(to_bytes(&(titles.len() as u32, titles)))
        })
        .migrate(2, 3, |old| {
            let (_count, titles): (u32, Vec<String>) = from_bytes(old)?;
            Ok(to_bytes(&(titles, None::<String>)))
        });

    assert_eq!(run(None, current.load()), Ok(Some(notes(&["a"]))));

    // Saving writes the current version, which then loads with no migration.
    run(None, current.save(&notes(&["a", "b"]))).unwrap();
    let no_migrations = Document::<Notes>::new(store, "notes").version(3);
    assert_eq!(
        run(None, no_migrations.load()),
        Ok(Some(notes(&["a", "b"])))
    );
}

#[test]
fn a_missing_migration_step_is_an_error_not_a_guess() {
    let store: Rc<dyn Storage> = Rc::new(MemoryStorage::new());
    run(None, Document::<u32>::new(Rc::clone(&store), "n").save(&1)).unwrap();

    let current = Document::<u32>::new(store, "n").version(3);
    assert!(matches!(
        run(None, current.load()),
        Err(PersistError::Storage(StorageError::MigrationUnavailable {
            from_version: 1,
            to_version: 3
        }))
    ));
}

#[test]
fn a_document_from_a_newer_app_is_refused_and_left_alone() {
    let store: Rc<dyn Storage> = Rc::new(MemoryStorage::new());
    let newer = Document::<u32>::new(Rc::clone(&store), "n").version(9);
    run(None, newer.save(&42)).unwrap();

    let older = Document::<u32>::new(Rc::clone(&store), "n").version(2);
    assert_eq!(
        run(None, older.load()),
        Err(PersistError::NewerVersion {
            found: 9,
            supported: 2
        })
    );
    // Still there for the newer app.
    assert_eq!(run(None, newer.load()), Ok(Some(42)));
}

#[test]
fn damaged_bytes_are_a_decode_error() {
    let store: Rc<dyn Storage> = Rc::new(MemoryStorage::new());
    // Version 1 header, then a payload too short for a u32.
    run(None, store.write("documents", "n", &[1, 0, 0, 0, 7])).unwrap();
    let document = Document::<u32>::new(Rc::clone(&store), "n");
    assert_eq!(
        run(None, document.load()),
        Err(PersistError::Decode(DecodeError::UnexpectedEnd))
    );

    // Shorter than the header itself.
    run(None, store.write("documents", "n", &[1, 0])).unwrap();
    assert_eq!(
        run(None, document.load()),
        Err(PersistError::Decode(DecodeError::UnexpectedEnd))
    );

    // Trailing bytes after a valid value.
    run(
        None,
        store.write("documents", "n", &[1, 0, 0, 0, 7, 0, 0, 0, 9]),
    )
    .unwrap();
    assert_eq!(
        run(None, document.load()),
        Err(PersistError::Decode(DecodeError::TrailingBytes))
    );
}

#[test]
fn staged_documents_are_saved_atomically_together() {
    let store: Rc<dyn Storage> = Rc::new(MemoryStorage::new());
    let first = Document::<u32>::new(Rc::clone(&store), "first");
    let second = Document::<String>::new(Rc::clone(&store), "second");

    let mut batch = StorageBatch::new();
    first.stage(&mut batch, &1);
    second.stage(&mut batch, &"two".to_owned());
    run(None, store.commit(batch)).unwrap();
    assert_eq!(run(None, first.load()), Ok(Some(1)));
    assert_eq!(run(None, second.load()), Ok(Some("two".to_owned())));

    // A batch that cannot be applied applies neither document.
    let mut rejected = StorageBatch::new();
    first.stage(&mut rejected, &100);
    rejected.write("documents", "bad/key", b"x");
    assert!(run(None, store.commit(rejected)).is_err());
    assert_eq!(run(None, first.load()), Ok(Some(1)));
}

#[test]
fn an_invalid_document_name_is_reported_when_used() {
    let store = Rc::new(MemoryStorage::new());
    let document = Document::<u32>::new(store, "a/b");
    assert_eq!(
        run(None, document.save(&1)),
        Err(PersistError::Storage(StorageError::InvalidKey))
    );
}

// ──────────────────────────── preferences ────────────────────────────

fn load_preferences(storage: Rc<dyn Storage>, venus: Option<&Rc<Venus>>) -> Preferences {
    run(venus, Preferences::load(storage)).unwrap()
}

#[test]
fn preferences_hold_typed_values_synchronously() {
    let prefs = Preferences::empty(Rc::new(MemoryStorage::new()));

    assert_eq!(prefs.get::<bool>("dark"), Ok(None));
    assert!(prefs.set("dark", &true));
    assert!(prefs.set("volume", &0.5f32));
    assert!(prefs.set("name", &"Ada".to_owned()));
    assert!(prefs.set("recent", &vec!["a".to_owned(), "b".to_owned()]));

    assert_eq!(prefs.get::<bool>("dark"), Ok(Some(true)));
    assert_eq!(prefs.get::<f32>("volume"), Ok(Some(0.5)));
    assert_eq!(prefs.get::<String>("name"), Ok(Some("Ada".to_owned())));
    assert_eq!(
        prefs.get::<Vec<String>>("recent").unwrap().unwrap().len(),
        2
    );
    assert_eq!(prefs.keys(), ["dark", "name", "recent", "volume"]);
    assert_eq!(prefs.len(), 4);
    assert!(prefs.contains("dark") && !prefs.contains("other"));
}

#[test]
fn reading_a_preference_as_the_wrong_type_is_an_error_or_the_default() {
    let prefs = Preferences::empty(Rc::new(MemoryStorage::new()));
    prefs.set("flag", &true);

    assert!(matches!(
        prefs.get::<u32>("flag"),
        Err(PersistError::Decode(_))
    ));
    assert_eq!(prefs.get_or("flag", 5u32), 5);
    assert_eq!(prefs.get_or("missing", 9u32), 9);
    assert!(prefs.get_or("flag", false));
}

#[test]
fn setting_an_unchanged_value_does_nothing() {
    let storage = Counting::new();
    let runtime = Runtime::new();
    let prefs = Preferences::empty(Rc::new(storage.clone()));

    assert!(prefs.set("a", &1u32));
    run(Some(&runtime.0), prefs.flush()).unwrap();
    assert_eq!(storage.writes(), 1);

    assert!(!prefs.set("a", &1u32), "the same value is not a change");
    assert!(!prefs.has_unsaved_changes());
    run(Some(&runtime.0), prefs.flush()).unwrap();
    assert_eq!(storage.writes(), 1);
}

#[test]
fn removing_reports_whether_it_existed() {
    let prefs = Preferences::empty(Rc::new(MemoryStorage::new()));
    prefs.set("a", &1u8);
    assert!(prefs.remove("a"));
    assert!(!prefs.remove("a"));
    assert_eq!(prefs.get::<u8>("a"), Ok(None));
    assert!(prefs.is_empty());
}

#[test]
fn watchers_hear_changes_until_dropped() {
    let prefs = Preferences::empty(Rc::new(MemoryStorage::new()));
    let (theme_calls, all_calls) = (
        Rc::new(Cell::new(0)),
        Rc::new(std::cell::RefCell::new(Vec::new())),
    );

    let counted = theme_calls.clone();
    let theme = prefs.watch("theme", move || counted.set(counted.get() + 1));
    let seen = all_calls.clone();
    let _all = prefs.watch_all(move |key| seen.borrow_mut().push(key.to_owned()));

    prefs.set("theme", &"dark".to_owned());
    prefs.set("volume", &1u8);
    prefs.set("theme", &"dark".to_owned()); // unchanged: silent
    prefs.remove("theme");
    assert_eq!(theme_calls.get(), 2);
    assert_eq!(*all_calls.borrow(), ["theme", "volume", "theme"]);

    drop(theme);
    prefs.set("theme", &"light".to_owned());
    assert_eq!(
        theme_calls.get(),
        2,
        "a dropped subscription must stay silent"
    );
}

#[test]
fn a_watcher_may_change_other_preferences() {
    let prefs = Preferences::empty(Rc::new(MemoryStorage::new()));
    let mirror = prefs.clone();
    let _watch = prefs.watch("source", move || {
        let value = mirror.get_or("source", 0u32);
        mirror.set("copy", &value);
    });

    prefs.set("source", &5u32);
    assert_eq!(prefs.get::<u32>("copy"), Ok(Some(5)));
}

#[test]
fn a_preference_handle_reads_its_default_until_set() {
    let prefs = Preferences::empty(Rc::new(MemoryStorage::new()));
    let volume = prefs.preference("volume", 0.8f32);
    let changes = Rc::new(Cell::new(0));
    let counted = changes.clone();
    let _watch = volume.watch(move || counted.set(counted.get() + 1));

    assert_eq!(volume.get(), 0.8);
    assert!(volume.set(&0.2));
    assert_eq!(volume.get(), 0.2);
    assert!(volume.reset());
    assert_eq!(volume.get(), 0.8);
    assert_eq!(changes.get(), 2);
}

#[test]
fn preferences_persist_across_a_restart() {
    let runtime = Runtime::new();
    let storage: Rc<dyn Storage> = Rc::new(MemoryStorage::new());

    let prefs = load_preferences(Rc::clone(&storage), Some(&runtime.0));
    assert!(prefs.is_empty());
    prefs.set("dark", &true);
    prefs.set("volume", &0.25f32);
    prefs.remove("volume");
    prefs.set("name", &"Ada".to_owned());
    run(Some(&runtime.0), prefs.flush()).unwrap();
    drop(prefs);

    let after_restart = load_preferences(storage, Some(&runtime.0));
    assert_eq!(after_restart.get::<bool>("dark"), Ok(Some(true)));
    assert_eq!(after_restart.get::<f32>("volume"), Ok(None));
    assert_eq!(
        after_restart.get::<String>("name"),
        Ok(Some("Ada".to_owned()))
    );
}

#[test]
fn changes_are_saved_in_the_background_without_a_flush() {
    let runtime = Runtime::new();
    let storage = Counting::new();
    let prefs = Preferences::empty(Rc::new(storage.clone()));

    prefs.set("a", &1u32);
    assert!(prefs.has_unsaved_changes());
    assert_eq!(
        storage.writes(),
        0,
        "nothing is written on the caller's stack"
    );

    pump_until(&runtime.0, "the background save", || {
        !prefs.has_unsaved_changes()
    });
    assert_eq!(storage.writes(), 1);
    assert_eq!(prefs.last_save_error(), None);
}

/// A slider dragged through a thousand values must not cost a thousand writes.
#[test]
fn a_burst_of_changes_collapses_into_few_saves() {
    let runtime = Runtime::new();
    let storage = Counting::new();
    let prefs = Preferences::empty(Rc::new(storage.clone()));

    for step in 0..1_000u32 {
        prefs.set("volume", &step);
    }
    run(Some(&runtime.0), prefs.flush()).unwrap();

    assert_eq!(
        storage.writes(),
        1,
        "1000 changes in one frame are one save"
    );
    let reloaded = load_preferences(Rc::new(storage.inner.clone()), Some(&runtime.0));
    assert_eq!(
        reloaded.get::<u32>("volume"),
        Ok(Some(999)),
        "the last value won"
    );
}

#[test]
fn changes_made_while_a_save_runs_are_saved_next() {
    let runtime = Runtime::new();
    let storage = Counting::new();
    let prefs = Preferences::empty(Rc::new(storage.clone()));

    prefs.set("a", &1u32);
    pump(Some(&runtime.0)); // the save starts
    prefs.set("a", &2u32);
    run(Some(&runtime.0), prefs.flush()).unwrap();

    let reloaded = load_preferences(Rc::new(storage.inner.clone()), Some(&runtime.0));
    assert_eq!(reloaded.get::<u32>("a"), Ok(Some(2)));
}

#[test]
fn flush_works_without_a_runtime_by_saving_inline() {
    let storage = Counting::new();
    let prefs = Preferences::empty(Rc::new(storage.clone()));

    prefs.set("a", &1u32);
    assert!(prefs.has_unsaved_changes());
    run(None, prefs.flush()).unwrap();
    assert!(!prefs.has_unsaved_changes());
    assert_eq!(storage.writes(), 1);
}

#[test]
fn a_failed_save_is_reported_by_flush_and_recovers() {
    let limits = StorageLimits::new(64, 1024, 8, 8).unwrap();
    let storage = Rc::new(Counting::with_limits(limits));
    let prefs = Preferences::empty(storage.clone());

    prefs.set("big", &vec![0u8; 200]);
    let error = run(None, prefs.flush()).unwrap_err();
    assert!(
        matches!(
            error,
            PersistError::Storage(StorageError::QuotaExceeded {
                kind: QuotaKind::ValueBytes,
                ..
            })
        ),
        "{error:?}"
    );
    assert_eq!(prefs.last_save_error(), Some(error));

    // Removing the offender lets the next save succeed, and clears the error.
    prefs.remove("big");
    prefs.set("small", &1u8);
    run(None, prefs.flush()).unwrap();
    assert_eq!(prefs.last_save_error(), None);
}

#[test]
fn damaged_preferences_are_reported_and_can_be_started_over() {
    let storage: Rc<dyn Storage> = Rc::new(MemoryStorage::new());
    run(
        None,
        storage.write("preferences", "all", &[1, 0, 0, 0, 0xff]),
    )
    .unwrap();

    assert!(matches!(
        run(None, Preferences::load(Rc::clone(&storage))),
        Err(PersistError::Decode(_))
    ));

    let fresh = Preferences::empty(Rc::clone(&storage));
    fresh.set("a", &1u8);
    run(None, fresh.flush()).unwrap();
    assert_eq!(load_preferences(storage, None).get::<u8>("a"), Ok(Some(1)));
}

// ───────────────────────────── persisted ─────────────────────────────

#[test]
fn persisted_state_starts_from_the_default_and_saves_nothing_until_changed() {
    let runtime = Runtime::new();
    let storage = Counting::new();
    let document = Document::<Notes>::new(Rc::new(storage.clone()), "notes");

    let persisted = run(
        Some(&runtime.0),
        Persisted::load(document, || notes(&["default"])),
    )
    .unwrap();
    assert_eq!(*persisted.handle().read(), notes(&["default"]));
    pump(Some(&runtime.0));
    assert_eq!(storage.writes(), 0, "loading must not write");
    assert!(!persisted.has_unsaved_changes());
}

#[test]
fn updates_from_any_handle_clone_are_saved_and_restored() {
    let runtime = Runtime::new();
    let storage: Rc<dyn Storage> = Rc::new(MemoryStorage::new());
    let document = || Document::<Notes>::new(Rc::clone(&storage), "notes");

    let persisted = run(Some(&runtime.0), Persisted::load(document(), || notes(&[]))).unwrap();
    // A widget reaches the provider through a clone of the handle, not through
    // the `Persisted` it came from.
    let from_a_widget = persisted.handle();
    from_a_widget.update(|value| value.titles.push("one".into()));
    from_a_widget.update(|value| value.pinned = Some("one".into()));
    run(Some(&runtime.0), persisted.flush()).unwrap();
    drop(persisted);

    let restarted = run(
        Some(&runtime.0),
        Persisted::load(document(), || notes(&["unused"])),
    )
    .unwrap();
    assert_eq!(
        *restarted.handle().read(),
        Notes {
            titles: vec!["one".into()],
            pinned: Some("one".into())
        }
    );
}

#[test]
fn a_burst_of_updates_collapses_into_few_saves() {
    let runtime = Runtime::new();
    let storage = Counting::new();
    let document = Document::<u32>::new(Rc::new(storage.clone()), "counter");
    let persisted = run(Some(&runtime.0), Persisted::load(document, || 0)).unwrap();

    let handle = persisted.handle();
    for _ in 0..500 {
        handle.update(|count| *count += 1);
    }
    run(Some(&runtime.0), persisted.flush()).unwrap();

    assert_eq!(storage.writes(), 1);
    let reloaded = Document::<u32>::new(Rc::new(storage.inner.clone()), "counter");
    assert_eq!(run(None, reloaded.load()), Ok(Some(500)));
}

#[test]
fn dropping_persisted_stops_saving_further_updates() {
    let runtime = Runtime::new();
    let storage = Counting::new();
    let document = Document::<u32>::new(Rc::new(storage.clone()), "counter");
    let persisted = run(Some(&runtime.0), Persisted::load(document, || 0)).unwrap();
    let handle = persisted.handle();
    drop(persisted);

    handle.update(|count| *count = 5);
    pump(Some(&runtime.0));
    assert_eq!(storage.writes(), 0);
}

#[test]
fn a_damaged_document_fails_load_but_load_or_default_starts_over() {
    let runtime = Runtime::new();
    let store: Rc<dyn Storage> = Rc::new(MemoryStorage::new());
    run(None, store.write("documents", "n", &[1, 0, 0, 0, 0xff])).unwrap();
    let document = || Document::<String>::new(Rc::clone(&store), "n");

    assert!(matches!(
        run(Some(&runtime.0), Persisted::load(document(), String::new)),
        Err(PersistError::Decode(_))
    ));

    let recovered = run(
        Some(&runtime.0),
        Persisted::load_or_default(document(), || "fresh".to_owned()),
    )
    .unwrap();
    assert_eq!(*recovered.handle().read(), "fresh");
}

/// Overwriting data that is merely newer than this build would destroy it.
#[test]
fn load_or_default_never_replaces_data_from_a_newer_app() {
    let runtime = Runtime::new();
    let store: Rc<dyn Storage> = Rc::new(MemoryStorage::new());
    run(
        None,
        Document::<u32>::new(Rc::clone(&store), "n")
            .version(5)
            .save(&1),
    )
    .unwrap();

    let document = Document::<u32>::new(store, "n").version(2);
    assert!(matches!(
        run(Some(&runtime.0), Persisted::load_or_default(document, || 0)),
        Err(PersistError::NewerVersion {
            found: 5,
            supported: 2
        })
    ));
}

// ───────────────────────────── app storage ─────────────────────────────

#[test]
fn locations_are_separate_stores_and_stable() {
    let app = AppStorage::in_memory("com.example.notes");
    assert_eq!(app.app_id(), "com.example.notes");

    run(None, app.data().write("ns", "key", b"data")).unwrap();
    run(None, app.cache().unwrap().write("ns", "key", b"cache")).unwrap();

    assert_eq!(
        run(None, app.data().read("ns", "key")).unwrap(),
        Some(b"data".to_vec())
    );
    assert_eq!(
        run(None, app.cache().unwrap().read("ns", "key")).unwrap(),
        Some(b"cache".to_vec())
    );
    assert_eq!(
        run(None, app.temp().unwrap().read("ns", "key")).unwrap(),
        None
    );

    // The same location is the same store, every time and from every clone.
    assert!(Rc::ptr_eq(
        &app.cache().unwrap(),
        &app.clone().cache().unwrap()
    ));
    assert!(Rc::ptr_eq(
        &app.data(),
        &app.at(StorageLocation::Data).unwrap()
    ));
}

#[test]
fn app_storage_hands_out_working_preferences_and_documents() {
    let runtime = Runtime::new();
    let app = AppStorage::in_memory("com.example.notes");

    let prefs = run(Some(&runtime.0), app.preferences()).unwrap();
    prefs.set("dark", &true);
    run(Some(&runtime.0), prefs.flush()).unwrap();
    assert_eq!(
        run(Some(&runtime.0), app.preferences())
            .unwrap()
            .get::<bool>("dark"),
        Ok(Some(true))
    );

    let document = app.document::<Notes>("notes");
    run(None, document.save(&notes(&["x"]))).unwrap();
    assert_eq!(
        run(None, app.document::<Notes>("notes").load()),
        Ok(Some(notes(&["x"])))
    );

    let cached = app
        .document_at::<u32>(StorageLocation::Cache, "thumbnail-count")
        .unwrap();
    run(None, cached.save(&3)).unwrap();
    assert_eq!(
        run(None, app.document::<u32>("thumbnail-count").load()),
        Ok(None)
    );
}

#[test]
fn preferences_do_not_collide_with_a_document_of_the_same_name() {
    let runtime = Runtime::new();
    let app = AppStorage::in_memory("com.example.notes");

    run(None, app.document::<u32>("preferences").save(&7)).unwrap();
    let prefs = run(Some(&runtime.0), app.preferences()).unwrap();
    assert!(prefs.is_empty());
    prefs.set("a", &1u8);
    run(Some(&runtime.0), prefs.flush()).unwrap();

    assert_eq!(
        run(None, app.document::<u32>("preferences").load()),
        Ok(Some(7))
    );
}

// ─────────────────────── the whole stack, on real files ───────────────────────

/// Preferences, a document and persisted provider state over `FileStorage`, with
/// disk I/O on Venus's offload pool and a restart in the middle.
#[test]
fn everything_survives_a_restart_on_real_files() {
    use aimer_storage::FileStorage;

    let runtime = Runtime::new();
    let root = std::env::temp_dir().join(format!("aimer-persist-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());

    // First run.
    {
        let storage: Rc<dyn Storage> = Rc::new(FileStorage::new(&root));
        let prefs = load_preferences(Rc::clone(&storage), Some(&runtime.0));
        prefs.set("dark", &true);
        prefs.set("name", &"Ada".to_owned());

        let model = Document::<Notes>::new(Rc::clone(&storage), "notes");
        let persisted = run(Some(&runtime.0), Persisted::load(model, || notes(&[]))).unwrap();
        persisted
            .handle()
            .update(|value| value.titles.push("first".into()));

        run(Some(&runtime.0), prefs.flush()).unwrap();
        run(Some(&runtime.0), persisted.flush()).unwrap();
    }

    // Second run: new store handles over the same directory.
    let storage: Rc<dyn Storage> = Rc::new(FileStorage::new(&root));
    let prefs = load_preferences(Rc::clone(&storage), Some(&runtime.0));
    assert_eq!(prefs.get::<bool>("dark"), Ok(Some(true)));
    assert_eq!(prefs.get::<String>("name"), Ok(Some("Ada".to_owned())));

    let model = Document::<Notes>::new(storage, "notes");
    let persisted = run(
        Some(&runtime.0),
        Persisted::load(model, || notes(&["unused"])),
    )
    .unwrap();
    assert_eq!(*persisted.handle().read(), notes(&["first"]));
}
