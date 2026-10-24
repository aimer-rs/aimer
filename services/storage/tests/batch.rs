//! `Storage::commit`, run against every native adapter through the public API.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Instant;

use aimer_storage::{
    FileStorage, MemoryStorage, QuotaKind, Storage, StorageBatch, StorageError, StorageLimits,
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

/// A directory removed on drop, with a Venus runtime installed for the test.
struct Root(PathBuf);

impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Venus::new().install();
        Self(std::env::temp_dir().join(format!(
            "aimer-storage-batch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        Venus::uninstall();
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs `check` once per adapter with the same limits, so one body states the
/// contract and every adapter has to meet it.
fn for_each_adapter(limits: StorageLimits, check: impl Fn(&dyn Storage, &str)) {
    check(&MemoryStorage::with_limits(limits), "MemoryStorage");
    let root = Root::new();
    check(&FileStorage::with_limits(&root.0, limits), "FileStorage");
}

fn read(storage: &dyn Storage, namespace: &str, key: &str) -> Option<Vec<u8>> {
    block_on(storage.read(namespace, key)).unwrap()
}

#[test]
fn every_operation_of_a_batch_takes_effect_across_namespaces() {
    for_each_adapter(StorageLimits::default(), |storage, name| {
        block_on(storage.write("drafts", "stale", b"old")).unwrap();
        block_on(storage.write("settings", "theme", b"light")).unwrap();

        let mut batch = StorageBatch::new();
        batch
            .write("settings", "theme", b"dark")
            .write("settings", "language", b"km")
            .write("profile", "name", b"Ada")
            .remove("drafts", "stale")
            .remove("drafts", "never-existed");
        block_on(storage.commit(batch)).unwrap();

        assert_eq!(
            read(storage, "settings", "theme"),
            Some(b"dark".to_vec()),
            "{name}"
        );
        assert_eq!(
            read(storage, "settings", "language"),
            Some(b"km".to_vec()),
            "{name}"
        );
        assert_eq!(
            read(storage, "profile", "name"),
            Some(b"Ada".to_vec()),
            "{name}"
        );
        assert_eq!(read(storage, "drafts", "stale"), None, "{name}");
        assert_eq!(
            block_on(storage.list("drafts", 8)).unwrap().entries().len(),
            0,
            "{name}"
        );
    });
}

#[test]
fn the_last_operation_on_a_key_wins() {
    for_each_adapter(StorageLimits::default(), |storage, name| {
        let mut batch = StorageBatch::new();
        batch
            .write("ns", "a", b"first")
            .write("ns", "a", b"second")
            .write("ns", "b", b"kept")
            .remove("ns", "b")
            .remove("ns", "c")
            .write("ns", "c", b"revived");
        block_on(storage.commit(batch)).unwrap();

        assert_eq!(read(storage, "ns", "a"), Some(b"second".to_vec()), "{name}");
        assert_eq!(read(storage, "ns", "b"), None, "{name}");
        assert_eq!(
            read(storage, "ns", "c"),
            Some(b"revived".to_vec()),
            "{name}"
        );
    });
}

#[test]
fn a_rejected_batch_applies_nothing() {
    let limits = StorageLimits::new(8, 12, 4, 8).unwrap();
    for_each_adapter(limits, |storage, name| {
        block_on(storage.write("ns", "existing", b"123456")).unwrap();

        let mut invalid_key = StorageBatch::new();
        invalid_key
            .write("ns", "fine", b"1")
            .write("ns", "bad/key", b"2");
        assert_eq!(
            block_on(storage.commit(invalid_key)),
            Err(StorageError::InvalidKey),
            "{name}"
        );

        let mut oversized = StorageBatch::new();
        oversized
            .remove("ns", "existing")
            .write("ns", "huge", &[0; 9]);
        assert!(
            matches!(
                block_on(storage.commit(oversized)),
                Err(StorageError::QuotaExceeded {
                    kind: QuotaKind::ValueBytes,
                    ..
                })
            ),
            "{name}"
        );

        let mut over_total = StorageBatch::new();
        over_total
            .write("ns", "a", &[0; 5])
            .write("ns", "b", &[0; 5]);
        assert!(
            matches!(
                block_on(storage.commit(over_total)),
                Err(StorageError::QuotaExceeded {
                    kind: QuotaKind::TotalBytes,
                    ..
                })
            ),
            "{name}"
        );

        assert_eq!(
            read(storage, "ns", "existing"),
            Some(b"123456".to_vec()),
            "{name}"
        );
        for key in ["fine", "huge", "a", "b"] {
            assert_eq!(
                read(storage, "ns", key),
                None,
                "{name}: {key} leaked from a rejected batch"
            );
        }
        assert_eq!(
            block_on(storage.list("ns", 8)).unwrap().entries().len(),
            1,
            "{name}"
        );
    });
}

/// Quotas are judged on the net effect, so a batch can swap entries in a store
/// that is full, where the same operations one by one cannot.
#[test]
fn a_batch_can_swap_entries_in_a_full_store() {
    let limits = StorageLimits::new(8, 100, 2, 8).unwrap();
    for_each_adapter(limits, |storage, name| {
        block_on(storage.write("ns", "old-1", b"1")).unwrap();
        block_on(storage.write("ns", "old-2", b"2")).unwrap();
        assert!(
            matches!(
                block_on(storage.write("ns", "new-1", b"3")),
                Err(StorageError::QuotaExceeded {
                    kind: QuotaKind::Entries,
                    ..
                })
            ),
            "{name}: the store should be full"
        );

        let mut swap = StorageBatch::new();
        swap.write("ns", "new-1", b"3")
            .write("ns", "new-2", b"4")
            .remove("ns", "old-1")
            .remove("ns", "old-2");
        block_on(storage.commit(swap)).unwrap();

        let keys: Vec<String> = block_on(storage.list("ns", 8))
            .unwrap()
            .entries()
            .iter()
            .map(|entry| entry.key().to_owned())
            .collect();
        assert_eq!(keys, ["new-1", "new-2"], "{name}");

        // The accounting after the swap is exact: still full, and one removal
        // makes room for exactly one write.
        assert!(
            block_on(storage.write("ns", "overflow", b"5")).is_err(),
            "{name}"
        );
        assert!(block_on(storage.remove("ns", "new-1")).unwrap(), "{name}");
        block_on(storage.write("ns", "overflow", b"5")).unwrap();
    });
}

#[test]
fn an_empty_batch_succeeds_and_touches_nothing() {
    let root = Root::new();
    let file = FileStorage::new(&root.0);
    block_on(file.commit(StorageBatch::new())).unwrap();
    assert!(
        !root.0.exists(),
        "an empty batch must not create the store directory"
    );

    block_on(MemoryStorage::new().commit(StorageBatch::new())).unwrap();
}

/// Operations take effect in the order they were issued, batches included.
#[test]
fn a_batch_is_ordered_with_the_operations_around_it() {
    for_each_adapter(StorageLimits::default(), |storage, name| {
        let before = storage.write("ns", "key", b"before");
        let mut batch = StorageBatch::new();
        batch
            .write("ns", "key", b"batch")
            .write("ns", "other", b"batch");
        let commit = storage.commit(batch);
        let after = storage.write("ns", "key", b"after");
        let read_after = storage.read("ns", "key");
        let read_other = storage.read("ns", "other");

        assert_eq!(
            block_on(read_other).unwrap(),
            Some(b"batch".to_vec()),
            "{name}"
        );
        assert_eq!(
            block_on(read_after).unwrap(),
            Some(b"after".to_vec()),
            "{name}"
        );
        block_on(after).unwrap();
        block_on(commit).unwrap();
        block_on(before).unwrap();
    });
}

#[test]
fn a_committed_batch_survives_reopening_the_store() {
    let root = Root::new();

    let first = FileStorage::new(&root.0);
    let mut batch = StorageBatch::new();
    batch
        .write("settings", "theme", b"dark")
        .write("drafts", "note", b"hello");
    block_on(first.commit(batch)).unwrap();
    let mut second_batch = StorageBatch::new();
    second_batch
        .write("settings", "theme", b"light")
        .remove("drafts", "note");
    block_on(first.commit(second_batch)).unwrap();
    drop(first);

    let reopened = FileStorage::new(&root.0);
    assert_eq!(
        read(&reopened, "settings", "theme"),
        Some(b"light".to_vec())
    );
    assert_eq!(read(&reopened, "drafts", "note"), None);
}

#[test]
fn consecutive_batches_and_singles_keep_exact_quota_accounting() {
    let limits = StorageLimits::new(8, 12, 100, 100).unwrap();
    let root = Root::new();
    let storage = FileStorage::with_limits(&root.0, limits);

    for round in 0..5u8 {
        let mut batch = StorageBatch::new();
        batch
            .write("ns", "a", &[round; 6])
            .write("ns", "b", &[round; 6]);
        block_on(storage.commit(batch)).unwrap();
        block_on(storage.write("ns", "a", &[round; 4])).unwrap();
    }

    // 4 + 6 bytes are stored; a fresh scan must agree with the running totals.
    let fresh = FileStorage::with_limits(&root.0, limits);
    block_on(fresh.write("ns", "c", &[0; 2])).unwrap();
    assert!(block_on(fresh.write("ns", "d", &[0; 1])).is_err());
}

/// The point of a batch on disk: its cost does not grow with its size the way
/// one-by-one writes do. Printed for the record; the assertion is deliberately
/// loose so a loaded machine cannot fail it.
#[test]
fn a_batch_costs_fewer_syncs_than_the_same_writes_one_by_one() {
    const ENTRIES: usize = 40;
    let value = [7u8; 256];

    let singles_root = Root::new();
    let singles = FileStorage::new(&singles_root.0);
    let started = Instant::now();
    for index in 0..ENTRIES {
        block_on(singles.write("ns", &format!("key-{index}"), &value)).unwrap();
    }
    let one_by_one = started.elapsed();

    let batch_root = Root::new();
    let batched = FileStorage::new(&batch_root.0);
    let mut batch = StorageBatch::with_capacity(ENTRIES);
    for index in 0..ENTRIES {
        batch.write("ns", &format!("key-{index}"), &value);
    }
    let started = Instant::now();
    block_on(batched.commit(batch)).unwrap();
    let as_batch = started.elapsed();

    println!("{ENTRIES} entries: one by one {one_by_one:?}, as one batch {as_batch:?}");
    assert!(
        as_batch < one_by_one,
        "a batch of {ENTRIES} took {as_batch:?}, longer than {ENTRIES} single writes ({one_by_one:?})"
    );
    assert_eq!(read(&batched, "ns", "key-39"), Some(value.to_vec()));
}
