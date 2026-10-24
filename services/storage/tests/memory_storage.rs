use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use aimer_storage::{
    MemoryStorage, QuotaKind, Storage, StorageError, StorageLimits, StorageMigration, migrate_bytes,
};

fn block_on<F: Future>(future: F) -> F::Output {
    struct Parker(std::thread::Thread);

    impl Wake for Parker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
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

#[test]
fn namespaces_do_not_leak_values_into_each_other() {
    let storage = MemoryStorage::new();

    block_on(storage.write("profile", "name", b"Ada"))
        .expect("writing a valid value should succeed");

    assert_eq!(
        block_on(storage.read("profile", "name")).expect("reading a valid value should succeed"),
        Some(b"Ada".to_vec())
    );
    assert_eq!(
        block_on(storage.read("session", "name")).expect("a different namespace is valid"),
        None
    );
}

#[test]
fn values_can_be_removed_cleared_and_listed_with_metadata() {
    let storage = MemoryStorage::new();

    block_on(storage.write("profile", "name", b"Ada")).expect("writing a value should succeed");
    block_on(storage.write("profile", "theme", b"dark")).expect("writing a value should succeed");

    let listing = block_on(storage.list("profile", 10)).expect("listing should succeed");
    assert!(!listing.is_truncated());
    assert_eq!(
        listing
            .entries()
            .iter()
            .map(|entry| (entry.key(), entry.size()))
            .collect::<Vec<_>>(),
        vec![("name", 3), ("theme", 4)]
    );
    assert_eq!(
        block_on(storage.metadata("profile", "name"))
            .expect("metadata should succeed")
            .expect("metadata should exist")
            .size(),
        3
    );

    assert!(block_on(storage.remove("profile", "name")).expect("removing should succeed"));
    assert!(!block_on(storage.remove("profile", "name")).expect("removing should be idempotent"));
    assert_eq!(
        block_on(storage.clear("profile")).expect("clearing should succeed"),
        1
    );
    assert!(
        block_on(storage.list("profile", 10))
            .expect("listing an empty namespace should succeed")
            .entries()
            .is_empty()
    );
}

#[test]
fn memory_storage_rejects_invalid_keys_and_preserves_quota_invariants() {
    let limits = StorageLimits::new(4, 6, 2, 1).expect("test limits should be valid");
    let storage = MemoryStorage::with_limits(limits);

    assert_eq!(
        block_on(storage.write("bad/name", "key", b"value")),
        Err(StorageError::InvalidKey)
    );
    assert_eq!(
        block_on(storage.write("profile", "bad\0key", b"value")),
        Err(StorageError::InvalidKey)
    );

    block_on(storage.write("profile", "one", b"1234")).expect("first value should fit");
    assert_eq!(
        block_on(storage.write("profile", "one", b"12345")),
        Err(StorageError::QuotaExceeded {
            kind: QuotaKind::ValueBytes,
            requested: 5,
            limit: 4,
        })
    );
    assert_eq!(
        block_on(storage.read("profile", "one")).expect("the rejected write must not mutate"),
        Some(b"1234".to_vec())
    );

    block_on(storage.write("profile", "two", b"12")).expect("total quota should allow 6 bytes");
    assert_eq!(
        block_on(storage.write("profile", "three", b"1")),
        Err(StorageError::QuotaExceeded {
            kind: QuotaKind::TotalBytes,
            requested: 7,
            limit: 6,
        })
    );
    assert_eq!(
        block_on(storage.list("profile", 3)),
        Err(StorageError::LimitExceeded {
            requested: 3,
            limit: 1,
        })
    );
    assert!(
        block_on(storage.list("profile", 1))
            .expect("the configured listing bound should be accepted")
            .is_truncated()
    );

    let entry_limited = MemoryStorage::with_limits(
        StorageLimits::new(4, 64, 2, 1).expect("entry test limits should be valid"),
    );
    block_on(entry_limited.write("profile", "one", b"1")).expect("first entry should fit");
    block_on(entry_limited.write("profile", "two", b"1")).expect("second entry should fit");
    assert_eq!(
        block_on(entry_limited.write("profile", "three", b"1")),
        Err(StorageError::QuotaExceeded {
            kind: QuotaKind::Entries,
            requested: 3,
            limit: 2,
        })
    );

    assert!(block_on(storage.remove("profile", "one")).expect("remove should succeed"));
    block_on(storage.write("profile", "three", b"1234"))
        .expect("removing a value should release its byte and entry quota");
}

struct AppendMigration {
    from: u32,
    to: u32,
    suffix: &'static [u8],
}

impl StorageMigration for AppendMigration {
    fn from_version(&self) -> u32 {
        self.from
    }

    fn to_version(&self) -> u32 {
        self.to
    }

    fn migrate(&self, value: &[u8]) -> aimer_storage::StorageResult<Vec<u8>> {
        let mut migrated = value.to_vec();
        migrated.extend_from_slice(self.suffix);
        Ok(migrated)
    }
}

#[test]
fn migrations_follow_exact_versions_and_report_gaps() {
    let first = AppendMigration {
        from: 1,
        to: 2,
        suffix: b"-two",
    };
    let second = AppendMigration {
        from: 2,
        to: 3,
        suffix: b"-three",
    };
    let migrations: [&dyn StorageMigration; 2] = [&first, &second];

    assert_eq!(
        migrate_bytes(b"value", 1, 3, &migrations).expect("both migrations should apply"),
        b"value-two-three".to_vec()
    );
    assert_eq!(
        migrate_bytes(b"value", 1, 4, &migrations),
        Err(StorageError::MigrationUnavailable {
            from_version: 3,
            to_version: 4,
        })
    );
    assert_eq!(
        migrate_bytes(b"value", 3, 2, &migrations),
        Err(StorageError::MigrationUnavailable {
            from_version: 3,
            to_version: 2,
        })
    );
}
