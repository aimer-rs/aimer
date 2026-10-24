//! What a crash at each point of a batch leaves behind, and what the store does
//! about it. The journal is built by hand so each state can be set up exactly.

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use aimer_venus::Venus;

use super::FileStorage;
use super::journal::{self, JOURNAL, JOURNAL_STAGING};
use super::layout::{value_path, write_atomically};
use crate::batch::{FinalOp, StorageBatch};
use crate::error::StorageError;
use crate::storage::Storage;

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

/// A fresh store directory with a Venus runtime installed, removed on drop.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Venus::new().install();
        Self {
            root: std::env::temp_dir().join(format!(
                "aimer-storage-recovery-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
        }
    }

    fn store(&self) -> FileStorage {
        FileStorage::new(&self.root)
    }

    fn journal(&self) -> PathBuf {
        self.root.join(JOURNAL)
    }

    /// Writes a journal as a crash after the commit point would leave it.
    fn commit_journal(&self, ops: &[FinalOp<'_>]) {
        fs::create_dir_all(&self.root).unwrap();
        fs::write(self.journal(), journal::encode(ops)).unwrap();
    }

    /// Puts a value on disk without going through the store.
    fn put(&self, namespace: &str, key: &str, value: &[u8]) {
        write_atomically(&value_path(&self.root, namespace, key), value).unwrap();
    }

    fn has(&self, namespace: &str, key: &str) -> bool {
        value_path(&self.root, namespace, key).exists()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        Venus::uninstall();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn write_op<'a>(namespace: &'a str, key: &'a str, value: &'a [u8]) -> FinalOp<'a> {
    FinalOp {
        namespace,
        key,
        value: Some(value),
    }
}

fn remove_op<'a>(namespace: &'a str, key: &'a str) -> FinalOp<'a> {
    FinalOp {
        namespace,
        key,
        value: None,
    }
}

#[test]
fn a_committed_journal_is_completed_before_the_first_operation() {
    let fixture = Fixture::new();
    fixture.put("drafts", "stale", b"to be removed");
    // The crash came after the commit point and before any file was applied.
    fixture.commit_journal(&[
        write_op("settings", "theme", b"dark"),
        write_op("settings", "language", b"km"),
        remove_op("drafts", "stale"),
    ]);

    let store = fixture.store();
    // Even a read must not see the store as it was before the batch.
    assert_eq!(
        block_on(store.read("settings", "theme")).unwrap(),
        Some(b"dark".to_vec())
    );
    assert_eq!(
        block_on(store.read("settings", "language")).unwrap(),
        Some(b"km".to_vec())
    );
    assert_eq!(block_on(store.read("drafts", "stale")).unwrap(), None);
    assert!(
        !fixture.journal().exists(),
        "a replayed journal must be retired"
    );
}

#[test]
fn replaying_over_a_partly_applied_batch_converges() {
    let fixture = Fixture::new();
    fixture.put("a", "applied", b"new");
    fixture.put("a", "other", b"old");
    fixture.commit_journal(&[
        write_op("a", "applied", b"new"),
        write_op("a", "other", b"new"),
    ]);

    let store = fixture.store();
    assert_eq!(
        block_on(store.read("a", "applied")).unwrap(),
        Some(b"new".to_vec())
    );
    assert_eq!(
        block_on(store.read("a", "other")).unwrap(),
        Some(b"new".to_vec())
    );
}

#[test]
fn recovered_data_counts_against_the_quota() {
    let fixture = Fixture::new();
    fixture.commit_journal(&[write_op("a", "big", &[7; 8])]);

    let limits = crate::StorageLimits::new(8, 10, 4, 4).unwrap();
    let store = FileStorage::with_limits(&fixture.root, limits);
    assert!(matches!(
        block_on(store.write("a", "more", &[1; 4])),
        Err(StorageError::QuotaExceeded {
            kind: crate::QuotaKind::TotalBytes,
            requested: 12,
            ..
        })
    ));
}

#[test]
fn a_damaged_journal_is_reported_and_nothing_is_applied() {
    let fixture = Fixture::new();
    fixture.put("a", "kept", b"original");
    let mut bytes = journal::encode(&[write_op("a", "kept", b"from the journal")]);
    bytes.truncate(bytes.len() - 3);
    fs::write(fixture.journal(), bytes).unwrap();

    let store = fixture.store();
    for _ in 0..2 {
        assert!(matches!(
            block_on(store.read("a", "kept")),
            Err(StorageError::Corrupt { .. })
        ));
    }
    assert_eq!(
        fs::read(value_path(&fixture.root, "a", "kept")).unwrap(),
        b"original"
    );
    assert!(
        fixture.journal().exists(),
        "a committed journal must not be discarded"
    );
}

#[test]
fn an_uncommitted_staging_file_is_ignored() {
    let fixture = Fixture::new();
    fixture.put("a", "existing", b"1");
    // The crash came before the rename: the staging file is not a commit.
    fs::write(fixture.root.join(JOURNAL_STAGING), b"half a journal").unwrap();

    let store = fixture.store();
    assert_eq!(
        block_on(store.read("a", "existing")).unwrap(),
        Some(b"1".to_vec())
    );

    let mut batch = StorageBatch::new();
    batch.write("a", "next", b"2");
    block_on(store.commit(batch)).unwrap();
    assert_eq!(
        block_on(store.read("a", "next")).unwrap(),
        Some(b"2".to_vec())
    );
}

#[test]
fn a_committed_batch_is_synced_and_its_journal_retired_when_the_store_goes_idle() {
    let fixture = Fixture::new();
    let store = fixture.store();

    let mut batch = StorageBatch::new();
    batch
        .write("a", "1", b"x")
        .write("b", "2", b"y")
        .remove("a", "gone");
    block_on(store.commit(batch)).unwrap();

    wait_until("the idle checkpoint to retire the journal", || {
        !fixture.journal().exists()
    });
    assert!(fixture.has("a", "1") && fixture.has("b", "2"));
}

/// A stale journal must never be replayed over a newer write, so a mutating
/// operation settles the previous batch before it runs.
#[test]
fn a_later_write_settles_the_previous_batch_first() {
    let fixture = Fixture::new();
    let store = fixture.store();

    let mut batch = StorageBatch::new();
    batch.write("a", "key", b"from batch");
    let committed = store.commit(batch);
    let overwrite = store.write("a", "key", b"newer");
    block_on(committed).unwrap();
    block_on(overwrite).unwrap();

    assert!(
        !fixture.journal().exists(),
        "the journal outlived a write that follows it"
    );
    assert_eq!(
        fs::read(value_path(&fixture.root, "a", "key")).unwrap(),
        b"newer"
    );
    assert_eq!(
        block_on(fixture.store().read("a", "key")).unwrap(),
        Some(b"newer".to_vec())
    );
}

#[test]
fn a_batch_that_cannot_be_planned_writes_no_journal_and_changes_nothing() {
    let fixture = Fixture::new();
    let store = fixture.store();
    block_on(store.write("ok", "existing", b"1")).unwrap();

    // A plain file where a namespace directory belongs makes that key unreadable.
    fs::write(
        super::layout::namespace_path(&fixture.root, "blocked"),
        b"in the way",
    )
    .unwrap();

    let mut batch = StorageBatch::new();
    batch.write("ok", "new", b"2").write("blocked", "key", b"3");
    assert!(block_on(store.commit(batch)).is_err());

    assert!(!fixture.journal().exists());
    assert!(
        !fixture.has("ok", "new"),
        "an entry of a failed batch was applied"
    );
}

#[test]
fn the_journal_is_not_mistaken_for_a_namespace() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.root).unwrap();
    fs::write(fixture.root.join(JOURNAL_STAGING), b"x").unwrap();
    fs::write(fixture.root.join(JOURNAL), journal::encode(&[])).unwrap();

    let usage = super::layout::scan_disk(Path::new(&fixture.root), crate::StorageOperation::Write);
    assert_eq!(usage, Ok(crate::limits::Usage::default()));
}
