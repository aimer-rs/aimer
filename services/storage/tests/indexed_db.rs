//! `IndexedDbStorage` against a real browser's IndexedDB.
//!
//! Run with `scripts/web_tests.sh`. These exist because the adapter's risky
//! parts are browser behavior no native test can reach: how IndexedDB orders and
//! ranges string keys, how transactions interleave, and what a committed
//! transaction leaves behind.

#![cfg(target_arch = "wasm32")]

use std::cell::Cell;

use aimer_storage::{
    IndexedDbStorage, QuotaKind, Storage, StorageBatch, StorageError, StorageLimits,
    StorageLocation,
};
use js_sys::{Function, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::{IdbDatabase, IdbFactory, IdbRequest, IdbTransactionMode};

wasm_bindgen_test_configure!(run_in_browser);

thread_local! {
    static NEXT: Cell<u32> = const { Cell::new(0) };
}

/// A database name no other test (or earlier run) has used.
fn unique(test: &str) -> String {
    let n = NEXT.with(|next| {
        next.set(next.get() + 1);
        next.get()
    });
    format!("aimer-test-{test}-{}-{n}", js_sys::Date::now() as u64)
}

fn store(test: &str) -> (IndexedDbStorage, String) {
    let name = unique(test);
    (IndexedDbStorage::new(name.clone()), name)
}

fn limited(test: &str, value: usize, total: usize, entries: usize) -> IndexedDbStorage {
    let limits = StorageLimits::new(value, total, entries, 64).expect("limits");
    IndexedDbStorage::with_limits(unique(test), limits)
}

async fn read(storage: &IndexedDbStorage, namespace: &str, key: &str) -> Option<Vec<u8>> {
    storage.read(namespace, key).await.expect("read")
}

async fn keys(storage: &IndexedDbStorage, namespace: &str) -> Vec<String> {
    storage
        .list(namespace, 64)
        .await
        .expect("list")
        .entries()
        .iter()
        .map(|entry| entry.key().to_owned())
        .collect()
}

// ───────────────────────── the basic contract ─────────────────────────

#[wasm_bindgen_test]
async fn round_trip_read_write_metadata_list_remove() {
    let (storage, _) = store("round-trip");

    assert_eq!(read(&storage, "profile", "mode").await, None);
    storage.write("profile", "mode", b"dark").await.expect("write");
    assert_eq!(read(&storage, "profile", "mode").await, Some(b"dark".to_vec()));
    assert_eq!(
        storage.metadata("profile", "mode").await.expect("metadata").map(|m| m.size()),
        Some(4)
    );
    assert_eq!(keys(&storage, "profile").await, ["mode"]);

    assert!(storage.remove("profile", "mode").await.expect("remove"));
    assert!(!storage.remove("profile", "mode").await.expect("remove again"));
    assert_eq!(read(&storage, "profile", "mode").await, None);
    assert_eq!(storage.metadata("profile", "mode").await.expect("metadata"), None);
}

#[wasm_bindgen_test]
async fn binary_values_survive_exactly() {
    let (storage, _) = store("binary");
    let all_bytes: Vec<u8> = (0..=255).collect();
    storage.write("ns", "bytes", &all_bytes).await.expect("write");
    storage.write("ns", "empty", b"").await.expect("write empty");

    assert_eq!(read(&storage, "ns", "bytes").await, Some(all_bytes));
    assert_eq!(read(&storage, "ns", "empty").await, Some(Vec::new()));
}

#[wasm_bindgen_test]
async fn replacing_a_value_keeps_one_entry_with_the_new_size() {
    let (storage, _) = store("replace");
    storage.write("ns", "k", b"short").await.expect("write");
    let longer = b"a much longer value";
    storage.write("ns", "k", longer).await.expect("replace");

    let listing = storage.list("ns", 8).await.expect("list");
    assert_eq!(listing.entries().len(), 1);
    assert_eq!(listing.entries()[0].size(), longer.len());
}

#[wasm_bindgen_test]
async fn invalid_names_are_rejected_before_the_browser_is_asked() {
    let (storage, _) = store("invalid");
    for (namespace, key) in [("", "k"), ("ns", ""), ("a/b", "k"), ("ns", "a\0b"), ("ns", "..")] {
        assert_eq!(
            storage.write(namespace, key, b"x").await,
            Err(StorageError::InvalidKey),
            "{namespace:?}/{key:?}"
        );
    }
}

// ───────────────────── key ranges and ordering (browser-specific) ─────────────────────

/// A namespace is a key *range*. Namespaces that are prefixes of one another,
/// or share leading characters with a separator-like byte, must not leak.
#[wasm_bindgen_test]
async fn namespaces_that_share_a_prefix_do_not_leak_into_each_other() {
    let (storage, _) = store("prefixes");
    let namespaces = ["a", "ab", "a b", "a-", "a~", "a\u{ffff}", "a😀", "z"];
    for namespace in namespaces {
        storage.write(namespace, "key", namespace.as_bytes()).await.expect("write");
        storage.write(namespace, "other", b"x").await.expect("write");
    }

    for namespace in namespaces {
        assert_eq!(keys(&storage, namespace).await, ["key", "other"], "{namespace:?}");
        assert_eq!(read(&storage, namespace, "key").await, Some(namespace.as_bytes().to_vec()));
    }

    assert_eq!(storage.clear("a").await.expect("clear"), 2);
    for namespace in namespaces.into_iter().filter(|namespace| *namespace != "a") {
        assert_eq!(keys(&storage, namespace).await.len(), 2, "clearing `a` hit {namespace:?}");
    }
}

/// IndexedDB orders strings by UTF-16 code unit; the other adapters order by
/// code point. A listing must come back in the same order from all of them.
#[wasm_bindgen_test]
async fn listings_are_in_code_point_order_not_utf16_order() {
    let (storage, _) = store("order");
    // In UTF-16 the emoji (surrogate pair D83D…) sorts *before* U+FFFF.
    for key in ["😀", "\u{ffff}", "z", "a", "é"] {
        storage.write("ns", key, b"x").await.expect("write");
    }

    assert_eq!(keys(&storage, "ns").await, ["a", "z", "é", "\u{ffff}", "😀"]);
}

#[wasm_bindgen_test]
async fn listings_are_bounded_and_say_when_they_are_truncated() {
    let (storage, _) = store("bounded");
    for index in 0..5 {
        storage.write("ns", &format!("k{index}"), b"x").await.expect("write");
    }

    let listing = storage.list("ns", 3).await.expect("list");
    assert_eq!(listing.entries().len(), 3);
    assert!(listing.is_truncated());
    assert!(!storage.list("ns", 5).await.expect("list").is_truncated());

    let narrow = limited("narrow", 8, 100, 10);
    assert!(matches!(
        narrow.list("ns", 1_000).await,
        Err(StorageError::LimitExceeded { .. })
    ));
}

/// The bookkeeping record lives in the same object store as the data. No
/// namespace, however named, may ever list it or read it.
#[wasm_bindgen_test]
async fn the_accounting_record_never_appears_as_data() {
    let (storage, _) = store("hidden");
    storage.write("ns", "k", b"x").await.expect("write");

    assert_eq!(keys(&storage, "ns").await, ["k"]);
    for namespace in ["usage", "u", "v", " ", "~"] {
        assert!(keys(&storage, namespace).await.is_empty(), "{namespace:?}");
        assert_eq!(read(&storage, namespace, "usage").await, None, "{namespace:?}");
    }
}

// ───────────────────────────────── quotas ─────────────────────────────────

#[wasm_bindgen_test]
async fn quotas_are_enforced_and_accounted_exactly() {
    let storage = limited("quota", 8, 12, 3);

    assert!(matches!(
        storage.write("ns", "big", &[0; 9]).await,
        Err(StorageError::QuotaExceeded { kind: QuotaKind::ValueBytes, .. })
    ));

    storage.write("ns", "a", &[0; 6]).await.expect("a");
    storage.write("ns", "b", &[0; 6]).await.expect("b");
    assert!(matches!(
        storage.write("ns", "c", &[0; 1]).await,
        Err(StorageError::QuotaExceeded { kind: QuotaKind::TotalBytes, requested: 13, limit: 12 })
    ));

    // Replacing counts the difference, and freeing makes room.
    storage.write("ns", "a", &[0; 2]).await.expect("shrink");
    storage.write("ns", "c", &[0; 4]).await.expect("c fits after shrinking a");
    assert!(storage.remove("ns", "b").await.expect("remove"));
    storage.write("ns", "d", &[0; 6]).await.expect("d fits after removing b");

    // Entries: a, c, d is three; a fourth is refused whatever its size.
    assert!(matches!(
        storage.write("ns", "e", b"").await,
        Err(StorageError::QuotaExceeded { kind: QuotaKind::Entries, .. })
    ));
}

#[wasm_bindgen_test]
async fn clearing_a_namespace_frees_its_quota() {
    let storage = limited("clear-quota", 8, 100, 2);
    storage.write("one", "a", b"1").await.expect("a");
    storage.write("one", "b", b"2").await.expect("b");
    assert!(storage.write("two", "x", b"3").await.is_err(), "the store is full");

    assert_eq!(storage.clear("one").await.expect("clear"), 2);
    assert_eq!(storage.clear("one").await.expect("clear again"), 0);
    storage.write("two", "x", b"3").await.expect("room after clear");
    storage.write("two", "y", b"4").await.expect("room after clear");
    assert!(storage.write("two", "z", b"5").await.is_err());
}

/// The usage record is updated in the same transaction as the write it accounts
/// for. Writes issued together must therefore be admitted one at a time, not all
/// checked against the same stale total.
#[wasm_bindgen_test]
async fn writes_issued_together_cannot_overshoot_the_quota() {
    let storage = limited("race", 8, 10, 10);

    let first = storage.write("ns", "a", &[0; 4]);
    let second = storage.write("ns", "b", &[0; 4]);
    let third = storage.write("ns", "c", &[0; 4]);
    let fourth = storage.write("ns", "d", &[0; 2]);

    assert!(first.await.is_ok());
    assert!(second.await.is_ok());
    assert!(matches!(
        third.await,
        Err(StorageError::QuotaExceeded { kind: QuotaKind::TotalBytes, requested: 12, .. })
    ));
    assert!(fourth.await.is_ok(), "a later write that fits is still admitted");
    assert_eq!(keys(&storage, "ns").await, ["a", "b", "d"]);
}

#[wasm_bindgen_test]
async fn two_connections_to_one_database_share_one_quota() {
    let name = unique("two-connections");
    let limits = StorageLimits::new(8, 10, 10, 64).expect("limits");
    let first = IndexedDbStorage::with_limits(name.clone(), limits);
    let second = IndexedDbStorage::with_limits(name, limits);

    first.write("ns", "a", &[0; 6]).await.expect("a");
    // `second` has never seen `a`, but the usage lives in the database.
    assert!(matches!(
        second.write("ns", "b", &[0; 6]).await,
        Err(StorageError::QuotaExceeded { kind: QuotaKind::TotalBytes, .. })
    ));
    second.write("ns", "b", &[0; 4]).await.expect("b fits");
}

// ───────────────────────── persistence and ordering ─────────────────────────

#[wasm_bindgen_test]
async fn data_survives_opening_the_database_again() {
    let (first, name) = store("reopen");
    first.write("settings", "theme", b"dark").await.expect("write");
    first.write("settings", "language", b"km").await.expect("write");
    drop(first);

    let second = IndexedDbStorage::new(name);
    assert_eq!(read(&second, "settings", "theme").await, Some(b"dark".to_vec()));
    assert_eq!(keys(&second, "settings").await, ["language", "theme"]);
}

#[wasm_bindgen_test]
async fn operations_take_effect_in_the_order_they_were_issued() {
    let (storage, _) = store("ordering");

    // None awaited until all are issued, on a database that is not open yet.
    let write_a = storage.write("ns", "k", b"a");
    let read_a = storage.read("ns", "k");
    let write_b = storage.write("ns", "k", b"b");
    let read_b = storage.read("ns", "k");
    let remove = storage.remove("ns", "k");
    let read_gone = storage.read("ns", "k");

    write_a.await.expect("write a");
    assert_eq!(read_a.await.expect("read a"), Some(b"a".to_vec()));
    write_b.await.expect("write b");
    assert_eq!(read_b.await.expect("read b"), Some(b"b".to_vec()));
    assert!(remove.await.expect("remove"));
    assert_eq!(read_gone.await.expect("read gone"), None);
}

#[wasm_bindgen_test]
async fn many_unawaited_writes_to_one_key_leave_the_last() {
    let (storage, _) = store("many");
    let writes: Vec<_> = (0..50u32).map(|n| storage.write("ns", "counter", &n.to_le_bytes())).collect();
    for write in writes {
        write.await.expect("write");
    }
    assert_eq!(read(&storage, "ns", "counter").await, Some(49u32.to_le_bytes().to_vec()));
}

// ───────────────────────────────── batches ─────────────────────────────────

#[wasm_bindgen_test]
async fn a_batch_applies_every_operation_across_namespaces() {
    let (storage, _) = store("batch");
    storage.write("drafts", "stale", b"old").await.expect("seed");
    storage.write("settings", "theme", b"light").await.expect("seed");

    let mut batch = StorageBatch::new();
    batch
        .write("settings", "theme", b"dark")
        .write("settings", "language", b"km")
        .write("profile", "name", b"Ada")
        .remove("drafts", "stale")
        .remove("drafts", "never-existed");
    storage.commit(batch).await.expect("commit");

    assert_eq!(read(&storage, "settings", "theme").await, Some(b"dark".to_vec()));
    assert_eq!(read(&storage, "settings", "language").await, Some(b"km".to_vec()));
    assert_eq!(read(&storage, "profile", "name").await, Some(b"Ada".to_vec()));
    assert_eq!(read(&storage, "drafts", "stale").await, None);
}

#[wasm_bindgen_test]
async fn in_a_batch_the_last_operation_on_a_key_wins() {
    let (storage, _) = store("batch-last");
    let mut batch = StorageBatch::new();
    batch
        .write("ns", "a", b"first")
        .write("ns", "a", b"second")
        .write("ns", "b", b"kept")
        .remove("ns", "b")
        .remove("ns", "c")
        .write("ns", "c", b"revived");
    storage.commit(batch).await.expect("commit");

    assert_eq!(read(&storage, "ns", "a").await, Some(b"second".to_vec()));
    assert_eq!(read(&storage, "ns", "b").await, None);
    assert_eq!(read(&storage, "ns", "c").await, Some(b"revived".to_vec()));
}

#[wasm_bindgen_test]
async fn a_rejected_batch_applies_nothing() {
    let storage = limited("batch-reject", 8, 12, 4);
    storage.write("ns", "existing", b"123456").await.expect("seed");

    let mut invalid_key = StorageBatch::new();
    invalid_key.write("ns", "fine", b"1").write("ns", "bad/key", b"2");
    assert_eq!(storage.commit(invalid_key).await, Err(StorageError::InvalidKey));

    let mut oversized = StorageBatch::new();
    oversized.remove("ns", "existing").write("ns", "huge", &[0; 9]);
    assert!(matches!(
        storage.commit(oversized).await,
        Err(StorageError::QuotaExceeded { kind: QuotaKind::ValueBytes, .. })
    ));

    let mut over_total = StorageBatch::new();
    over_total.write("ns", "a", &[0; 5]).write("ns", "b", &[0; 5]);
    assert!(matches!(
        storage.commit(over_total).await,
        Err(StorageError::QuotaExceeded { kind: QuotaKind::TotalBytes, .. })
    ));

    assert_eq!(keys(&storage, "ns").await, ["existing"]);
    assert_eq!(read(&storage, "ns", "existing").await, Some(b"123456".to_vec()));
}

#[wasm_bindgen_test]
async fn a_batch_can_swap_entries_in_a_full_store_and_accounting_stays_exact() {
    let storage = limited("batch-swap", 8, 100, 2);
    storage.write("ns", "old-1", b"1").await.expect("seed");
    storage.write("ns", "old-2", b"2").await.expect("seed");
    assert!(storage.write("ns", "new-1", b"3").await.is_err(), "the store is full");

    let mut swap = StorageBatch::new();
    swap.write("ns", "new-1", b"3")
        .write("ns", "new-2", b"4")
        .remove("ns", "old-1")
        .remove("ns", "old-2");
    storage.commit(swap).await.expect("swap");

    assert_eq!(keys(&storage, "ns").await, ["new-1", "new-2"]);
    assert!(storage.write("ns", "overflow", b"5").await.is_err(), "still full");
    assert!(storage.remove("ns", "new-1").await.expect("remove"));
    storage.write("ns", "overflow", b"5").await.expect("one removal, one write");
}

#[wasm_bindgen_test]
async fn an_empty_batch_succeeds_and_a_batch_is_ordered_with_its_neighbours() {
    let (storage, _) = store("batch-order");
    storage.commit(StorageBatch::new()).await.expect("empty batch");

    let before = storage.write("ns", "key", b"before");
    let mut batch = StorageBatch::new();
    batch.write("ns", "key", b"batch").write("ns", "other", b"batch");
    let commit = storage.commit(batch);
    let after = storage.write("ns", "key", b"after");
    let read_key = storage.read("ns", "key");
    let read_other = storage.read("ns", "other");

    before.await.expect("before");
    commit.await.expect("commit");
    after.await.expect("after");
    assert_eq!(read_key.await.expect("read"), Some(b"after".to_vec()));
    assert_eq!(read_other.await.expect("read"), Some(b"batch".to_vec()));
}

#[wasm_bindgen_test]
async fn a_committed_batch_survives_reopening() {
    let (first, name) = store("batch-reopen");
    let mut batch = StorageBatch::new();
    batch.write("settings", "theme", b"dark").write("drafts", "note", b"hello");
    first.commit(batch).await.expect("commit");
    drop(first);

    let second = IndexedDbStorage::new(name);
    assert_eq!(read(&second, "settings", "theme").await, Some(b"dark".to_vec()));
    assert_eq!(read(&second, "drafts", "note").await, Some(b"hello".to_vec()));
}

// ───────────────────────────────── locations ─────────────────────────────────

#[wasm_bindgen_test]
async fn data_roaming_and_cache_locations_map_to_the_right_databases() {
    let app = unique("locations").replace(['-', ' '], "");
    let data = IndexedDbStorage::for_location(&app, StorageLocation::Data).expect("data");
    let roaming = IndexedDbStorage::for_location(&app, StorageLocation::Roaming).expect("roaming");
    let cache = IndexedDbStorage::for_location(&app, StorageLocation::Cache).expect("cache");

    data.write("ns", "k", b"data").await.expect("write");
    cache.write("ns", "k", b"cache").await.expect("write");

    assert_eq!(read(&roaming, "ns", "k").await, Some(b"data".to_vec()), "roaming is data");
    assert_eq!(read(&cache, "ns", "k").await, Some(b"cache".to_vec()), "cache is separate");
    assert_eq!(read(&data, "ns", "k").await, Some(b"data".to_vec()));
}

#[wasm_bindgen_test]
fn temp_is_not_a_database_and_bad_app_ids_are_rejected() {
    assert!(matches!(
        IndexedDbStorage::for_location("app", StorageLocation::Temp),
        Err(StorageError::Unsupported { .. })
    ));
    assert!(matches!(IndexedDbStorage::for_app("../escape"), Err(StorageError::InvalidKey)));
    assert!(matches!(IndexedDbStorage::for_app(""), Err(StorageError::InvalidKey)));
}

// ─────────────────────────── a database that is not ours ───────────────────────────

/// Resolves when `request` succeeds.
async fn done(request: &IdbRequest) {
    let promise = Promise::new(&mut |resolve: Function, reject: Function| {
        request.set_onsuccess(Some(&resolve));
        request.set_onerror(Some(&reject));
    });
    JsFuture::from(promise).await.expect("request");
}

/// Writes `value` straight into a store's object store, bypassing the adapter.
async fn put_raw(database: &str, record: &str, value: JsValue) {
    let factory: IdbFactory = Reflect::get(&js_sys::global(), &JsValue::from_str("indexedDB"))
        .expect("indexedDB")
        .dyn_into()
        .expect("factory");
    let open = factory.open(database).expect("open");
    done(&open).await;
    let db: IdbDatabase = open.result().expect("db").dyn_into().expect("database");

    let transaction = db
        .transaction_with_str_and_mode("values", IdbTransactionMode::Readwrite)
        .expect("transaction");
    let put = transaction
        .object_store("values")
        .expect("object store")
        .put_with_key(&value, &JsValue::from_str(record))
        .expect("put");
    done(&put).await;
    let finished = Promise::new(&mut |resolve: Function, _| transaction.set_oncomplete(Some(&resolve)));
    JsFuture::from(finished).await.expect("commit");
    db.close();
}

/// Something that is not a byte array under a key this adapter owns is not
/// ours to interpret: it is reported, not misread.
#[wasm_bindgen_test]
async fn a_foreign_value_is_reported_as_corruption_not_misread() {
    let (storage, name) = store("foreign");
    storage.write("ns", "ours", b"fine").await.expect("write");

    put_raw(&name, "ns\0theirs", JsValue::from_str("a string, not bytes")).await;

    assert_eq!(read(&storage, "ns", "ours").await, Some(b"fine".to_vec()));
    assert!(matches!(storage.read("ns", "theirs").await, Err(StorageError::Corrupt { .. })));
    assert!(matches!(storage.metadata("ns", "theirs").await, Err(StorageError::Corrupt { .. })));
    assert!(matches!(storage.list("ns", 8).await, Err(StorageError::Corrupt { .. })));
}
