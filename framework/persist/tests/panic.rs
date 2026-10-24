//! A panic must not take unsaved changes with it.
//!
//! A release build aborts on panic: the process ends the instant the panic hook
//! returns. These tests panic for real (inside `catch_unwind`) and look at the
//! disk immediately afterwards, with no flushing, pumping or awaiting in
//! between, so anything found there was written by the panic handler itself.

#![cfg(not(target_arch = "wasm32"))]

use std::panic;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use aimer_persist::{
    Document, Persisted, Preferences, set_flush_on_panic, set_lifecycle_flush_timeout,
};
use aimer_storage::{
    FileStorage, MemoryStorage, Storage, StorageBatch, StorageFuture, StorageList, StorageMetadata,
};
use aimer_venus::{FrameBudget, Venus, install_panic_hook};

/// A runtime with the panic hook installed, and a quiet one underneath it.
struct Runtime(Rc<Venus>);

impl Runtime {
    fn new() -> Self {
        // Keep the log readable; the chained hook still runs.
        panic::set_hook(Box::new(|_| {}));
        let venus = Venus::new();
        venus.install();
        install_panic_hook();
        set_flush_on_panic(true);
        Self(venus)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        Venus::uninstall();
        set_flush_on_panic(true);
        set_lifecycle_flush_timeout(Duration::from_secs(2));
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("aimer-panic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Self(root)
    }

    fn storage(&self) -> Rc<dyn Storage> {
        Rc::new(FileStorage::new(&self.0))
    }

    /// Value files on disk (not staging files).
    fn value_files(&self) -> usize {
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
        walk(&self.0, &mut count);
        count
    }

    /// What a fresh process would read, bypassing every in-memory copy.
    fn reload_preferences(&self) -> Preferences {
        let mut load = Preferences::load(self.storage());
        let mut context = Context::from_waker(Waker::noop());
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Poll::Ready(result) = load.as_mut().poll(&mut context) {
                return result.expect("preferences load");
            }
            assert!(Instant::now() < deadline);
            if let Some(venus) = Venus::current() {
                venus.run_microtasks();
            }
        }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn panic_now() {
    let _ = panic::catch_unwind(|| panic!("the app failed"));
}

// ───────────────────────────── on the UI thread ─────────────────────────────

#[test]
fn a_panic_saves_changes_that_had_not_been_written_yet() {
    let _runtime = Runtime::new();
    let root = TempRoot::new("ui");
    let prefs = Preferences::empty(root.storage());
    prefs.set("draft", &"typed a moment ago".to_owned());
    assert!(prefs.has_unsaved_changes());
    assert_eq!(root.value_files(), 0, "nothing has been written yet");

    panic_now();

    assert_eq!(root.value_files(), 1, "the panic handler did not save");
    assert_eq!(
        root.reload_preferences().get::<String>("draft"),
        Ok(Some("typed a moment ago".to_owned()))
    );
}

#[test]
fn a_panic_saves_persisted_state_even_while_a_normal_save_is_in_flight() {
    let runtime = Runtime::new();
    let root = TempRoot::new("in-flight");
    let document = Document::<u32>::new(root.storage(), "counter");
    let mut load = Persisted::load(document, || 0);
    let persisted = loop {
        if let Poll::Ready(result) = load.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            break result.expect("load");
        }
        runtime.0.run_microtasks();
    };

    let handle = persisted.handle();
    handle.update(|count| *count = 1);
    // The normal background save starts and takes its turn...
    runtime.0.run_idle(&FrameBudget::from_now(Duration::from_millis(8)));
    // ...and the app changes again and then panics before it settles.
    handle.update(|count| *count = 2);

    panic_now();

    let reloaded = Document::<u32>::new(root.storage(), "counter");
    let mut load = reloaded.load();
    let value = loop {
        if let Poll::Ready(result) = load.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            break result;
        }
        runtime.0.run_microtasks();
    };
    assert_eq!(value, Ok(Some(2)), "the last change was lost");
}

#[test]
fn a_panic_with_nothing_pending_costs_nothing() {
    let _runtime = Runtime::new();
    let root = TempRoot::new("clean");
    let _prefs = Preferences::empty(root.storage());

    let started = Instant::now();
    panic_now();

    assert!(started.elapsed() < Duration::from_millis(250), "a clean panic was slowed down");
    assert_eq!(root.value_files(), 0);
}

#[test]
fn saving_on_panic_can_be_turned_off() {
    let _runtime = Runtime::new();
    let root = TempRoot::new("opt-out");
    let prefs = Preferences::empty(root.storage());
    prefs.set("a", &1u8);

    set_flush_on_panic(false);
    panic_now();

    assert_eq!(root.value_files(), 0, "saved despite being turned off");
}

// ───────────────────────────── on another thread ─────────────────────────────

/// The case a release build makes fatal: a worker panics, the process aborts as
/// soon as the hook returns, and the state lives on the UI thread.
#[test]
fn a_panic_on_a_worker_thread_saves_state_held_on_the_ui_thread() {
    let runtime = Runtime::new();
    let root = TempRoot::new("worker");
    let prefs = Preferences::empty(root.storage());
    prefs.set("draft", &"unsaved".to_owned());
    assert_eq!(root.value_files(), 0);

    let released = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&released);
    let worker = std::thread::spawn(move || {
        panic_now();
        // Reached only after the hook returned, which is when the process would
        // abort: everything must already be saved.
        flag.store(true, Ordering::SeqCst);
    });

    // The UI thread only services the runtime's microtasks (which is how the
    // request reaches it); it never runs idle work or flushes by itself.
    let deadline = Instant::now() + Duration::from_secs(20);
    while !released.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the worker was never released");
        runtime.0.run_microtasks();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(root.value_files(), 1, "the worker was released before the save finished");
    worker.join().expect("worker");

    assert_eq!(
        root.reload_preferences().get::<String>("draft"),
        Ok(Some("unsaved".to_owned()))
    );
}

// ─────────────────────────────── when saving cannot finish ───────────────────────────────

/// A store whose writes never complete.
struct Stuck(MemoryStorage);

impl Storage for Stuck {
    fn read(&self, n: &str, k: &str) -> StorageFuture<Option<Vec<u8>>> {
        self.0.read(n, k)
    }
    fn write(&self, _: &str, _: &str, _: &[u8]) -> StorageFuture<()> {
        Box::pin(std::future::pending())
    }
    fn remove(&self, n: &str, k: &str) -> StorageFuture<bool> {
        self.0.remove(n, k)
    }
    fn clear(&self, n: &str) -> StorageFuture<usize> {
        self.0.clear(n)
    }
    fn list(&self, n: &str, l: usize) -> StorageFuture<StorageList> {
        self.0.list(n, l)
    }
    fn metadata(&self, n: &str, k: &str) -> StorageFuture<Option<StorageMetadata>> {
        self.0.metadata(n, k)
    }
    fn commit(&self, b: StorageBatch) -> StorageFuture<()> {
        self.0.commit(b)
    }
}

#[test]
fn a_save_that_never_finishes_cannot_hold_a_panic_open() {
    let _runtime = Runtime::new();
    let prefs = Preferences::empty(Rc::new(Stuck(MemoryStorage::new())));
    prefs.set("a", &1u8);

    set_lifecycle_flush_timeout(Duration::from_millis(100));
    let started = Instant::now();
    panic_now();
    let waited = started.elapsed();

    assert!(waited >= Duration::from_millis(100), "gave up after {waited:?}");
    assert!(waited < Duration::from_secs(2), "overran: {waited:?}");
}
