//! `FileStorage` against a real directory, driven the way an app drives it:
//! from Venus tasks, on a Venus offload pool.

#![cfg(not(target_arch = "wasm32"))]

use std::cell::{Cell, RefCell};
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use aimer_storage::{
    FileStorage, QuotaKind, Storage, StorageCapability, StorageError, StorageLimits,
};
use aimer_venus::Venus;

fn block_on<F: Future>(future: F) -> F::Output {
    struct Parker(std::thread::Thread);

    impl Wake for Parker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(Parker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match Pin::new(&mut future).poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// A directory removed on drop, so a failing test does not leave it behind.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(std::env::temp_dir().join(format!(
            "aimer-storage-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Installs a Venus runtime for the calling thread for the test's duration.
struct InstalledVenus(Rc<Venus>);

impl InstalledVenus {
    fn new() -> Self {
        let venus = Venus::new();
        venus.install();
        Self(venus)
    }
}

impl Drop for InstalledVenus {
    fn drop(&mut self) {
        Venus::uninstall();
    }
}

#[test]
fn round_trip_uses_the_offload_pool() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();
    let limits = StorageLimits::new(64, 256, 8, 8).expect("test limits should be valid");
    let storage = FileStorage::with_limits(root.path(), limits);

    block_on(storage.write("profile", "display-mode", b"dark"))
        .expect("encoded file paths should accept safe logical keys");
    assert_eq!(
        block_on(storage.read("profile", "display-mode")).expect("reading should succeed"),
        Some(b"dark".to_vec())
    );
    assert_eq!(
        block_on(storage.metadata("profile", "display-mode"))
            .expect("metadata should succeed")
            .expect("metadata should exist")
            .size(),
        4
    );
    assert_eq!(
        block_on(storage.list("profile", 8))
            .expect("listing should succeed")
            .entries()[0]
            .key(),
        "display-mode"
    );
    assert!(block_on(storage.remove("profile", "display-mode")).expect("remove should succeed"));
    assert_eq!(
        block_on(storage.read("profile", "display-mode")).expect("reading should succeed"),
        None
    );
}

/// The point of a disk adapter: a later run of the app finds what an earlier
/// one stored.
#[test]
fn values_survive_reopening_the_store() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();

    let first = FileStorage::new(root.path());
    block_on(first.write("settings", "theme", b"dark")).unwrap();
    block_on(first.write("settings", "language", b"km")).unwrap();
    block_on(first.write("drafts", "note-1", b"hello")).unwrap();
    drop(first);

    let second = FileStorage::new(root.path());
    assert_eq!(
        block_on(second.read("settings", "theme")).unwrap(),
        Some(b"dark".to_vec())
    );
    let settings = block_on(second.list("settings", 8)).unwrap();
    let keys: Vec<_> = settings.entries().iter().map(|entry| entry.key()).collect();
    assert_eq!(keys, ["language", "theme"]);
    assert_eq!(
        block_on(second.read("drafts", "note-1")).unwrap(),
        Some(b"hello".to_vec())
    );
}

#[test]
fn a_replaced_value_is_the_value_a_later_run_reads() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();

    let first = FileStorage::new(root.path());
    block_on(first.write("settings", "theme", b"dark")).unwrap();
    block_on(first.write("settings", "theme", b"light, and longer")).unwrap();
    drop(first);

    let second = FileStorage::new(root.path());
    assert_eq!(
        block_on(second.read("settings", "theme")).unwrap(),
        Some(b"light, and longer".to_vec())
    );
    assert_eq!(
        block_on(second.list("settings", 8))
            .unwrap()
            .entries()
            .len(),
        1
    );
}

#[test]
fn an_operation_without_a_venus_runtime_says_so() {
    Venus::uninstall();
    let root = TempRoot::new();
    let storage = FileStorage::new(root.path());

    assert_eq!(
        block_on(storage.write("settings", "theme", b"dark")),
        Err(StorageError::Unavailable {
            capability: StorageCapability::Runtime,
        })
    );
    assert!(
        !root.path().exists(),
        "nothing may be written without a runtime"
    );
}

/// Operations take effect in the order they were issued, even though they are
/// awaited out of order and the pool has several workers.
#[test]
fn operations_take_effect_in_issue_order() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();
    let storage = FileStorage::new(root.path());

    let write_a = storage.write("ns", "key", b"a");
    let read_a = storage.read("ns", "key");
    let write_b = storage.write("ns", "key", b"b");
    let read_b = storage.read("ns", "key");
    let remove = storage.remove("ns", "key");
    let read_gone = storage.read("ns", "key");

    assert_eq!(block_on(read_gone).unwrap(), None);
    assert!(block_on(remove).unwrap());
    assert_eq!(block_on(read_b).unwrap(), Some(b"b".to_vec()));
    block_on(write_b).unwrap();
    assert_eq!(block_on(read_a).unwrap(), Some(b"a".to_vec()));
    block_on(write_a).unwrap();
}

#[test]
fn many_unawaited_writes_to_one_key_leave_the_last() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();
    let storage = FileStorage::new(root.path());

    let writes: Vec<_> = (0..200u32)
        .map(|n| storage.write("ns", "counter", &n.to_le_bytes()))
        .collect();
    for write in writes {
        block_on(write).unwrap();
    }

    assert_eq!(
        block_on(storage.read("ns", "counter")).unwrap(),
        Some(199u32.to_le_bytes().to_vec())
    );
}

/// The shape an app actually uses: a Venus task holding non-`Send` state awaits
/// the store, and the result lands on the UI thread.
#[test]
fn a_venus_task_awaits_storage_and_keeps_its_non_send_state() {
    let installed = InstalledVenus::new();
    let root = TempRoot::new();
    let storage = FileStorage::new(root.path());

    let loaded = Rc::new(RefCell::new(None));
    let finished = Rc::new(Cell::new(false));

    let (task_loaded, task_finished) = (loaded.clone(), finished.clone());
    installed.0.spawn(async move {
        storage.write("settings", "theme", b"dark").await.unwrap();
        *task_loaded.borrow_mut() = storage.read("settings", "theme").await.unwrap();
        task_finished.set(true);
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !finished.get() {
        assert!(
            std::time::Instant::now() < deadline,
            "the task never finished"
        );
        installed.0.run_microtasks();
        std::thread::yield_now();
    }

    assert_eq!(loaded.borrow().as_deref(), Some(&b"dark"[..]));
    assert_eq!(installed.0.task_count(), 0);
}

/// Regression: a component's hex spelling used to exceed the 255-byte file-name
/// limit from 128 bytes up, so keys the crate accepted failed to write.
#[test]
fn the_longest_allowed_key_is_written_and_a_longer_one_is_refused() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();
    let storage = FileStorage::new(root.path());

    let longest = "k".repeat(StorageLimits::MAX_COMPONENT_BYTES);
    block_on(storage.write(&longest, &longest, b"x")).expect("the longest key must fit on disk");
    assert_eq!(
        block_on(storage.read(&longest, &longest)).unwrap(),
        Some(b"x".to_vec())
    );

    let too_long = "k".repeat(StorageLimits::MAX_COMPONENT_BYTES + 1);
    assert_eq!(
        block_on(storage.write("ns", &too_long, b"x")),
        Err(StorageError::InvalidKey)
    );
}

#[test]
fn quotas_are_enforced_against_what_is_already_on_disk() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();
    let limits = StorageLimits::new(8, 10, 3, 8).unwrap();

    let first = FileStorage::with_limits(root.path(), limits);
    block_on(first.write("ns", "a", b"123456")).unwrap();
    drop(first);

    let reopened = FileStorage::with_limits(root.path(), limits);
    assert_eq!(
        block_on(reopened.write("ns", "b", b"12345")),
        Err(StorageError::QuotaExceeded {
            kind: QuotaKind::TotalBytes,
            requested: 11,
            limit: 10,
        })
    );
    block_on(reopened.write("ns", "b", b"1234")).unwrap();
}

#[test]
fn removing_and_clearing_release_quota() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();
    let limits = StorageLimits::new(8, 100, 2, 8).unwrap();
    let storage = FileStorage::with_limits(root.path(), limits);

    block_on(storage.write("ns", "a", b"1")).unwrap();
    block_on(storage.write("ns", "b", b"2")).unwrap();
    assert!(matches!(
        block_on(storage.write("ns", "c", b"3")),
        Err(StorageError::QuotaExceeded {
            kind: QuotaKind::Entries,
            ..
        })
    ));

    assert!(block_on(storage.remove("ns", "a")).unwrap());
    block_on(storage.write("ns", "c", b"3")).unwrap();

    assert_eq!(block_on(storage.clear("ns")).unwrap(), 2);
    block_on(storage.write("other", "x", b"1")).unwrap();
    block_on(storage.write("other", "y", b"2")).unwrap();
    assert!(matches!(
        block_on(storage.write("other", "z", b"3")),
        Err(StorageError::QuotaExceeded {
            kind: QuotaKind::Entries,
            ..
        })
    ));
}

/// The quota totals come from one scan, not one scan per write: a write to a
/// store of N entries must not cost N directory reads.
#[test]
fn the_store_is_scanned_once_not_once_per_write() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();
    let storage = FileStorage::new(root.path());

    block_on(storage.write("ns", "first", b"1")).unwrap();
    // A stray file in the root is corruption to a scan, so a rescan would fail.
    fs::write(root.path().join("stray"), b"not ours").unwrap();
    block_on(storage.write("ns", "second", b"2")).expect("the second write must not rescan");

    let fresh = FileStorage::new(root.path());
    assert!(matches!(
        block_on(fresh.write("ns", "third", b"3")),
        Err(StorageError::Corrupt { .. })
    ));
}

#[test]
fn foreign_files_in_a_namespace_are_reported_as_corruption() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();
    let storage = FileStorage::new(root.path());

    block_on(storage.write("ns", "key", b"1")).unwrap();
    let namespace_dir = fs::read_dir(root.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(namespace_dir.join(".DS_Store"), b"junk").unwrap();

    assert!(matches!(
        block_on(storage.list("ns", 8)),
        Err(StorageError::Corrupt { .. })
    ));
}

#[test]
fn an_interrupted_write_leaves_no_visible_entry() {
    let _venus = InstalledVenus::new();
    let root = TempRoot::new();
    let storage = FileStorage::new(root.path());

    block_on(storage.write("ns", "key", b"1")).unwrap();
    let namespace_dir = fs::read_dir(root.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(namespace_dir.join(".aimer-tmp-1-1"), b"half a write").unwrap();

    let fresh = FileStorage::new(root.path());
    let listing = block_on(fresh.list("ns", 8)).unwrap();
    assert_eq!(listing.entries().len(), 1);
    assert_eq!(block_on(fresh.clear("ns")).unwrap(), 1);
    assert!(!namespace_dir.exists());
}

#[test]
fn for_app_rejects_an_app_id_that_could_escape_its_directory() {
    assert_eq!(
        FileStorage::for_app("../escape").unwrap_err(),
        StorageError::InvalidKey
    );
    assert_eq!(
        FileStorage::for_app("").unwrap_err(),
        StorageError::InvalidKey
    );
}
