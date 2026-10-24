//! Losing no data when the app is backgrounded, quits, or is about to be killed:
//! the event loop reports the moment through `aimer_venus::notify_lifecycle`, and
//! every pending change has to be in storage by the time that call returns.

#![cfg(not(target_arch = "wasm32"))]

use std::cell::Cell;
use std::future::Future;
use std::path::PathBuf;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use aimer_persist::{
    Document, FlushOutcome, PersistError, Persisted, Preferences, flush_all_blocking,
    has_unsaved_changes, set_lifecycle_flush_timeout,
};
use aimer_storage::{
    FileStorage, MemoryStorage, Storage, StorageBatch, StorageError, StorageFuture, StorageLimits,
    StorageList, StorageMetadata,
};
use aimer_venus::{Lifecycle, Venus, notify_lifecycle};

fn run<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = Box::pin(future);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        assert!(Instant::now() < deadline, "the future never completed");
        if let Some(venus) = Venus::current() {
            venus.run_microtasks();
        }
    }
}

/// A Venus runtime installed for the test.
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

/// A directory removed on drop.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("aimer-lifecycle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Self(root)
    }

    fn storage(&self) -> Rc<dyn Storage> {
        Rc::new(FileStorage::new(&self.0))
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a fresh process would find in `root`, bypassing every in-memory copy.
fn stored_preferences(root: &TempRoot) -> Preferences {
    run(Preferences::load(root.storage())).unwrap()
}

// ───────────────────────────── background and exit ─────────────────────────────

#[test]
fn going_to_the_background_saves_everything_before_it_returns() {
    let _runtime = Runtime::new();
    let root = TempRoot::new("background");
    let prefs = Preferences::empty(root.storage());

    prefs.set("draft", &"unsaved words".to_owned());
    assert!(prefs.has_unsaved_changes());
    assert!(has_unsaved_changes());

    // No pumping, no awaiting: the notification alone has to do it.
    notify_lifecycle(Lifecycle::Background);

    assert!(!prefs.has_unsaved_changes());
    assert!(!has_unsaved_changes());
    assert_eq!(
        stored_preferences(&root).get::<String>("draft"),
        Ok(Some("unsaved words".to_owned()))
    );
}

#[test]
fn quitting_saves_preferences_and_persisted_state_together() {
    let _runtime = Runtime::new();
    let root = TempRoot::new("exit");
    let storage = root.storage();

    let prefs = Preferences::empty(Rc::clone(&storage));
    let document = Document::<Vec<String>>::new(Rc::clone(&storage), "notes");
    let persisted = run(Persisted::load(document.clone(), Vec::new)).unwrap();

    prefs.set("dark", &true);
    persisted.handle().update(|notes| notes.push("typed just before quitting".into()));
    notify_lifecycle(Lifecycle::Exit);

    assert!(!has_unsaved_changes());
    assert_eq!(stored_preferences(&root).get::<bool>("dark"), Ok(Some(true)));
    assert_eq!(
        run(Document::<Vec<String>>::new(root.storage(), "notes").load()),
        Ok(Some(vec!["typed just before quitting".to_owned()]))
    );
}

/// The saver's task owns what it needs, so dropping the handle that made the
/// change does not lose it.
#[test]
fn a_change_is_saved_even_if_its_owner_was_dropped_first() {
    let _runtime = Runtime::new();
    let root = TempRoot::new("dropped");
    let prefs = Preferences::empty(root.storage());
    prefs.set("kept", &1u32);
    drop(prefs);

    notify_lifecycle(Lifecycle::Exit);

    assert_eq!(stored_preferences(&root).get::<u32>("kept"), Ok(Some(1)));
}

#[test]
fn nothing_pending_means_nothing_to_wait_for() {
    let _runtime = Runtime::new();
    let root = TempRoot::new("idle");
    let prefs = Preferences::empty(root.storage());
    prefs.set("a", &1u8);
    notify_lifecycle(Lifecycle::Background);

    let started = Instant::now();
    notify_lifecycle(Lifecycle::Exit);
    assert!(started.elapsed() < Duration::from_millis(100), "a clean exit blocked");
    assert_eq!(flush_all_blocking(Duration::from_secs(5)), FlushOutcome::Complete);
}

#[test]
fn the_notification_works_without_a_runtime_by_saving_inline() {
    Venus::uninstall();
    // Memory storage: the file adapter, by design, needs a runtime to do I/O.
    let storage: Rc<dyn Storage> = Rc::new(MemoryStorage::new());
    let prefs = Preferences::empty(Rc::clone(&storage));
    prefs.set("a", &1u8);

    notify_lifecycle(Lifecycle::Exit);

    assert!(!prefs.has_unsaved_changes());
    assert_eq!(run(Preferences::load(storage)).unwrap().get::<u8>("a"), Ok(Some(1)));
}

#[test]
fn each_of_several_changes_is_saved_by_one_notification() {
    let _runtime = Runtime::new();
    let root = TempRoot::new("several");
    let first = Preferences::empty(root.storage());
    first.set("k", &1u8);
    // A second, independent store with its own pending change.
    let other_root = TempRoot::new("several-other");
    let second = Preferences::empty(other_root.storage());
    second.set("k", &2u8);

    notify_lifecycle(Lifecycle::Background);

    assert_eq!(stored_preferences(&root).get::<u8>("k"), Ok(Some(1)));
    assert_eq!(stored_preferences(&other_root).get::<u8>("k"), Ok(Some(2)));
}

// ─────────────────────────────── becoming inactive ───────────────────────────────

/// Whether any value file (not a staging file) exists under `root`.
fn value_files(root: &std::path::Path) -> usize {
    fn walk(path: &std::path::Path, count: &mut usize) {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, count);
            } else if !entry.file_name().to_string_lossy().starts_with(".aimer-") {
                *count += 1;
            }
        }
    }
    let mut count = 0;
    walk(root, &mut count);
    count
}

/// Losing focus is not an emergency, so the UI thread does not wait, but the
/// write is started at once and reaches the disk on a worker thread whether or
/// not the UI thread ever looks at it again.
#[test]
fn becoming_inactive_starts_the_write_without_waiting_for_it() {
    let runtime = Runtime::new();
    let root = TempRoot::new("inactive");
    let prefs = Preferences::empty(root.storage());
    prefs.set("a", &1u8);
    assert_eq!(value_files(&root.0), 0, "nothing is written until the save starts");

    let started = Instant::now();
    notify_lifecycle(Lifecycle::Inactive);
    assert!(started.elapsed() < Duration::from_secs(1), "an inactive notification must not wait");

    // Not touching the runtime at all: the worker alone puts the value on disk.
    let deadline = Instant::now() + Duration::from_secs(30);
    while value_files(&root.0) == 0 {
        assert!(Instant::now() < deadline, "the started write never reached the disk");
        std::thread::sleep(Duration::from_millis(1));
    }

    // Letting the runtime observe the completion settles the saver.
    while prefs.has_unsaved_changes() {
        assert!(Instant::now() < deadline, "the saver never settled");
        runtime.0.run_microtasks();
        runtime.0.run_idle(&aimer_venus::FrameBudget::from_now(Duration::from_millis(8)));
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(stored_preferences(&root).get::<u8>("a"), Ok(Some(1)));
}

/// Pending state outlives every handle to it, for persisted provider state too.
#[test]
fn persisted_state_changed_just_before_it_is_dropped_is_still_saved() {
    let _runtime = Runtime::new();
    let root = TempRoot::new("persisted-dropped");
    let document = Document::<u32>::new(root.storage(), "counter");
    let persisted = run(Persisted::load(document, || 0)).unwrap();
    let handle = persisted.handle();
    handle.update(|count| *count = 41);
    handle.update(|count| *count += 1);
    drop(persisted);
    drop(handle);

    notify_lifecycle(Lifecycle::Exit);

    assert_eq!(
        run(Document::<u32>::new(root.storage(), "counter").load()),
        Ok(Some(42))
    );
}

// ───────────────────────────── when saving cannot finish ─────────────────────────────

/// A store whose writes never complete.
struct Stuck(MemoryStorage);

impl Storage for Stuck {
    fn read(&self, namespace: &str, key: &str) -> StorageFuture<Option<Vec<u8>>> {
        self.0.read(namespace, key)
    }
    fn write(&self, _: &str, _: &str, _: &[u8]) -> StorageFuture<()> {
        Box::pin(std::future::pending())
    }
    fn remove(&self, namespace: &str, key: &str) -> StorageFuture<bool> {
        self.0.remove(namespace, key)
    }
    fn clear(&self, namespace: &str) -> StorageFuture<usize> {
        self.0.clear(namespace)
    }
    fn list(&self, namespace: &str, limit: usize) -> StorageFuture<StorageList> {
        self.0.list(namespace, limit)
    }
    fn metadata(&self, namespace: &str, key: &str) -> StorageFuture<Option<StorageMetadata>> {
        self.0.metadata(namespace, key)
    }
    fn commit(&self, batch: StorageBatch) -> StorageFuture<()> {
        self.0.commit(batch)
    }
}

#[test]
fn a_save_that_never_finishes_cannot_hold_the_app_past_the_timeout() {
    let _runtime = Runtime::new();
    let prefs = Preferences::empty(Rc::new(Stuck(MemoryStorage::new())));
    prefs.set("a", &1u8);

    let started = Instant::now();
    let outcome = flush_all_blocking(Duration::from_millis(100));
    let waited = started.elapsed();

    assert_eq!(outcome, FlushOutcome::TimedOut { unsaved: 1 });
    assert!(waited >= Duration::from_millis(100), "gave up after only {waited:?}");
    assert!(waited < Duration::from_secs(2), "overran the deadline: {waited:?}");
    assert!(prefs.has_unsaved_changes(), "an unsaved change must not be reported as saved");
}

#[test]
fn the_notification_honours_the_configured_timeout() {
    let _runtime = Runtime::new();
    let prefs = Preferences::empty(Rc::new(Stuck(MemoryStorage::new())));
    prefs.set("a", &1u8);

    set_lifecycle_flush_timeout(Duration::from_millis(50));
    let started = Instant::now();
    notify_lifecycle(Lifecycle::Background);
    let waited = started.elapsed();
    set_lifecycle_flush_timeout(Duration::from_secs(2));

    assert!(waited >= Duration::from_millis(50) && waited < Duration::from_secs(1), "{waited:?}");
}

#[test]
fn a_failed_save_is_reported() {
    let _runtime = Runtime::new();
    let limits = StorageLimits::new(16, 1024, 8, 8).unwrap();
    let prefs = Preferences::empty(Rc::new(MemoryStorage::with_limits(limits)));
    prefs.set("too big", &vec![0u8; 100]);

    let outcome = flush_all_blocking(Duration::from_secs(5));
    assert!(
        matches!(outcome, FlushOutcome::Failed(PersistError::Storage(StorageError::QuotaExceeded { .. }))),
        "{outcome:?}"
    );
    assert!(prefs.last_save_error().is_some());
}

/// The blocking flush pumps the runtime, so it must never be entered from inside
/// a task it would be pumping.
#[test]
fn inside_a_venus_task_the_flush_starts_saves_but_does_not_block() {
    let runtime = Runtime::new();
    let prefs = Preferences::empty(Rc::new(Stuck(MemoryStorage::new())));
    prefs.set("a", &1u8);

    let outcome = Rc::new(Cell::new(None));
    let seen = outcome.clone();
    let started = Instant::now();
    runtime.0.spawn(async move {
        seen.set(Some(flush_all_blocking(Duration::from_secs(10))));
    });
    runtime.0.run_microtasks();

    assert_eq!(outcome.take(), Some(FlushOutcome::TimedOut { unsaved: 1 }));
    assert!(started.elapsed() < Duration::from_secs(2), "blocked inside a task");
}
