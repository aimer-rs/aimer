//! The browser adapter, backed by IndexedDB.

mod request;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Array, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use web_sys::{IdbDatabase, IdbFactory, IdbKeyRange, IdbOpenDbRequest, IdbTransactionMode};

use self::request::{Finish, Txn, channel, thrown, unavailable};
use crate::batch::{StorageBatch, new_lens, plan};
use crate::entry::{StorageList, StorageMetadata};
use crate::error::{StorageError, StorageOperation, StorageResult};
use crate::key::{is_valid_component, validate, validate_namespace};
use crate::keyspace::{
    USAGE_KEY, decode_usage, encode_usage, key_of, namespace_bounds, record_key,
};
use crate::limits::{StorageLimits, Usage};
use crate::location::StorageLocation;
use crate::storage::{Storage, StorageFuture, ready};

/// The one object store, with out-of-line string keys.
const STORE: &str = "values";
const SCHEMA_VERSION: u32 = 1;

type DbJob = Box<dyn FnOnce(StorageResult<IdbDatabase>)>;

enum DbState {
    Closed,
    /// Operations issued while the database opens, in issue order.
    Opening(Vec<DbJob>),
    Open(IdbDatabase),
}

/// The lazily-opened connection shared by every clone of a store.
struct Connection {
    name: String,
    state: Rc<RefCell<DbState>>,
}

impl Connection {
    /// Runs `job` with the open database.
    ///
    /// Jobs run in the order they were issued: immediately when the database is
    /// open, and in queue order once it finishes opening. Each job creates its
    /// transaction synchronously, and IndexedDB runs overlapping read-write
    /// transactions in creation order, so that is what makes a `read` issued
    /// after a `write` see it.
    fn run(&self, job: DbJob) {
        let mut state = self.state.borrow_mut();
        match &mut *state {
            DbState::Open(database) => {
                let database = database.clone();
                drop(state);
                job(Ok(database));
            }
            DbState::Opening(pending) => pending.push(job),
            DbState::Closed => {
                *state = DbState::Opening(vec![job]);
                drop(state);
                self.open();
            }
        }
    }

    fn open(&self) {
        let request = match open_request(&self.name) {
            Ok(request) => request,
            Err(error) => return Self::settle(&self.state, Err(error)),
        };

        let on_upgrade = Closure::once(move |event: web_sys::Event| {
            let created = event
                .target()
                .and_then(|target| target.dyn_into::<IdbOpenDbRequest>().ok())
                .and_then(|request| request.result().ok())
                .and_then(|database| database.dyn_into::<IdbDatabase>().ok())
                .map(|database| database.create_object_store(STORE));
            // A failure here surfaces as the open request failing.
            drop(created);
        });
        request.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));
        on_upgrade.forget();

        let state = Rc::clone(&self.state);
        let target = request.clone();
        let on_end = Closure::once(move |event: web_sys::Event| {
            let opened = if event.type_() == "success" {
                target
                    .result()
                    .ok()
                    .and_then(|database| database.dyn_into::<IdbDatabase>().ok())
                    .ok_or_else(unavailable)
            } else {
                Err(unavailable())
            };
            Self::settle(&state, opened);
        });
        request.set_onsuccess(Some(on_end.as_ref().unchecked_ref()));
        request.set_onerror(Some(on_end.as_ref().unchecked_ref()));
        on_end.forget();
    }

    /// Finishes an open attempt, running everything that queued behind it.
    ///
    /// A failed attempt leaves the connection `Closed`, so the next operation
    /// tries again rather than failing forever.
    fn settle(state: &Rc<RefCell<DbState>>, opened: StorageResult<IdbDatabase>) {
        let pending = match std::mem::replace(
            &mut *state.borrow_mut(),
            match &opened {
                Ok(database) => DbState::Open(database.clone()),
                Err(_) => DbState::Closed,
            },
        ) {
            DbState::Opening(pending) => pending,
            DbState::Closed | DbState::Open(_) => Vec::new(),
        };
        for job in pending {
            job(opened.clone());
        }
    }
}

fn open_request(name: &str) -> StorageResult<IdbOpenDbRequest> {
    // `indexedDB` is a global in windows and workers alike.
    let factory = Reflect::get(&js_sys::global(), &JsValue::from_str("indexedDB"))
        .ok()
        .and_then(|factory| factory.dyn_into::<IdbFactory>().ok())
        .ok_or_else(unavailable)?;
    factory
        .open_with_u32(name, SCHEMA_VERSION)
        .map_err(|_| unavailable())
}

/// A browser storage adapter backed by IndexedDB.
///
/// The `wasm32` counterpart of [`FileStorage`](https://docs.rs/aimer_storage):
/// values persist in the browser for the page's origin, and
/// `IndexedDbStorage::for_app(app_id)` finds the same data on every visit.
///
/// All values live in one object store. The quota totals are kept in a record
/// of their own and updated in the *same transaction* as the write they
/// account for, so they stay exact across several tabs, and no write scans the
/// store.
///
/// IndexedDB is already asynchronous, so there is no worker: each operation
/// opens a transaction when it is issued and its future resolves when the
/// transaction commits. Operations take effect in the order they were issued.
///
/// # Browser support
///
/// This adapter has been compiled for `wasm32-unknown-unknown` but not run in a
/// browser yet. Its key-space and quota arithmetic are covered by native tests;
/// the IndexedDB calls themselves need a browser test pass.
#[derive(Clone)]
pub struct IndexedDbStorage {
    connection: Rc<Connection>,
    limits: StorageLimits,
}

impl IndexedDbStorage {
    /// Creates an adapter over the IndexedDB database named `database_name`
    /// with default limits. The database is opened on first use.
    pub fn new(database_name: impl Into<String>) -> Self {
        Self::with_limits(database_name, StorageLimits::default())
    }

    /// Creates an adapter with explicit limits.
    pub fn with_limits(database_name: impl Into<String>, limits: StorageLimits) -> Self {
        Self {
            connection: Rc::new(Connection {
                name: database_name.into(),
                state: Rc::new(RefCell::new(DbState::Closed)),
            }),
            limits,
        }
    }

    /// Opens the store an application keeps in the browser.
    ///
    /// # Errors
    ///
    /// [`StorageError::InvalidKey`] when `app_id` is not a valid component.
    pub fn for_app(app_id: &str) -> StorageResult<Self> {
        Self::for_location(app_id, StorageLocation::Data)
    }

    /// [`IndexedDbStorage::for_app`] with explicit limits.
    pub fn for_app_with_limits(app_id: &str, limits: StorageLimits) -> StorageResult<Self> {
        Self::for_location_with_limits(app_id, StorageLocation::Data, limits)
    }

    /// Opens the store an application keeps at `location`.
    ///
    /// `Data` and `Roaming` share one database, since the browser has no
    /// synchronization to tell them apart. `Cache` is a database of its own, so
    /// clearing it never touches data. The browser decides what it may evict
    /// either way.
    ///
    /// # Errors
    ///
    /// * [`StorageError::InvalidKey`] when `app_id` is not a valid component.
    /// * [`StorageError::Unsupported`] for `Temp`: scratch data on the web is
    ///   plain memory, so use [`MemoryStorage`](crate::MemoryStorage).
    pub fn for_location(app_id: &str, location: StorageLocation) -> StorageResult<Self> {
        Self::for_location_with_limits(app_id, location, StorageLimits::default())
    }

    /// [`IndexedDbStorage::for_location`] with explicit limits.
    pub fn for_location_with_limits(
        app_id: &str,
        location: StorageLocation,
        limits: StorageLimits,
    ) -> StorageResult<Self> {
        if !is_valid_component(app_id) {
            return Err(StorageError::InvalidKey);
        }
        let name = match location {
            StorageLocation::Data | StorageLocation::Roaming => format!("aimer-storage:{app_id}"),
            StorageLocation::Cache => format!("aimer-storage:{app_id}:cache"),
            StorageLocation::Temp => {
                return Err(StorageError::Unsupported {
                    operation: StorageOperation::Write,
                });
            }
        };
        Ok(Self::with_limits(name, limits))
    }

    /// Returns the limits enforced by this adapter.
    pub const fn limits(&self) -> StorageLimits {
        self.limits
    }

    /// Runs `body` inside a transaction created now, and resolves to the answer
    /// it records once that transaction has committed.
    fn transact<T, F>(
        &self,
        operation: StorageOperation,
        mode: IdbTransactionMode,
        body: F,
    ) -> StorageFuture<T>
    where
        T: 'static,
        F: FnOnce(Txn<T>) + 'static,
    {
        let (completer, future) = channel(operation);
        let finish = Finish::new(completer);
        self.connection.run(Box::new(move |opened| {
            let database = match opened {
                Ok(database) => database,
                Err(error) => return finish.done(Err(error)),
            };
            let transaction = match database.transaction_with_str_and_mode(STORE, mode) {
                Ok(transaction) => transaction,
                Err(error) => return finish.done(Err(thrown(operation, &error))),
            };
            let store = match transaction.object_store(STORE) {
                Ok(store) => store,
                Err(error) => return finish.done(Err(thrown(operation, &error))),
            };
            body(Txn::attach(&transaction, store, operation, finish));
        }));
        future
    }
}

fn js_key(name: &str) -> JsValue {
    JsValue::from_str(name)
}

/// The bytes of a stored value, or `Corrupt` for a record this adapter did not
/// write.
fn bytes_of(value: &JsValue, operation: StorageOperation) -> StorageResult<Uint8Array> {
    value
        .clone()
        .dyn_into::<Uint8Array>()
        .map_err(|_| StorageError::Corrupt { operation })
}

/// The quota totals record; absent in a database nothing has been written to.
fn read_usage(value: &JsValue, operation: StorageOperation) -> StorageResult<Usage> {
    if value.is_undefined() {
        return Ok(Usage::default());
    }
    decode_usage(&bytes_of(value, operation)?.to_vec()).ok_or(StorageError::Corrupt { operation })
}

fn usage_value(usage: Usage) -> Uint8Array {
    Uint8Array::from(&encode_usage(usage)[..])
}

fn namespace_range(namespace: &str) -> Result<IdbKeyRange, JsValue> {
    let (lower, upper) = namespace_bounds(namespace);
    IdbKeyRange::bound_with_lower_open_and_upper_open(&js_key(&lower), &js_key(&upper), false, true)
}

/// Persists `usage`, or records why it could not be.
fn put_usage<T: 'static>(txn: &Txn<T>, usage: Usage) {
    txn.fire(
        txn.store
            .put_with_key(&usage_value(usage), &js_key(USAGE_KEY)),
    );
}

impl Storage for IndexedDbStorage {
    fn read(&self, namespace: &str, key: &str) -> StorageFuture<Option<Vec<u8>>> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let name = js_key(&record_key(namespace, key));
        let limits = self.limits;
        self.transact(
            StorageOperation::Read,
            IdbTransactionMode::Readonly,
            move |txn| {
                let found = txn.clone();
                txn.then(txn.store.get(&name), move |value| {
                    found.set(if value.is_undefined() {
                        Ok(None)
                    } else {
                        bytes_of(&value, StorageOperation::Read).and_then(|bytes| {
                            limits.check_value(bytes.length() as usize)?;
                            Ok(Some(bytes.to_vec()))
                        })
                    });
                });
            },
        )
    }

    fn write(&self, namespace: &str, key: &str, value: &[u8]) -> StorageFuture<()> {
        if let Err(error) =
            validate(namespace, key).and_then(|()| self.limits.check_value(value.len()))
        {
            return ready(Err(error));
        }
        let name = js_key(&record_key(namespace, key));
        // Copied now: the caller's slice is gone by the time callbacks run.
        let value = Uint8Array::from(value);
        let limits = self.limits;
        self.transact(
            StorageOperation::Write,
            IdbTransactionMode::Readwrite,
            move |txn| {
                let operation = StorageOperation::Write;
                let after_usage = txn.clone();
                txn.then(txn.store.get(&js_key(USAGE_KEY)), move |usage| {
                    let usage = match read_usage(&usage, operation) {
                        Ok(usage) => usage,
                        Err(error) => return after_usage.set(Err(error)),
                    };
                    let after_old = after_usage.clone();
                    after_usage.then(after_usage.store.get(&name), move |old| {
                        let old_len = if old.is_undefined() {
                            None
                        } else {
                            match bytes_of(&old, operation) {
                                Ok(bytes) => Some(bytes.length() as usize),
                                Err(error) => return after_old.set(Err(error)),
                            }
                        };
                        match limits.admit_write(usage, old_len, value.length() as usize) {
                            Err(error) => after_old.set(Err(error)),
                            Ok(after) => {
                                after_old.fire(after_old.store.put_with_key(&value, &name));
                                put_usage(&after_old, after);
                                after_old.set(Ok(()));
                            }
                        }
                    });
                });
            },
        )
    }

    fn remove(&self, namespace: &str, key: &str) -> StorageFuture<bool> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let name = js_key(&record_key(namespace, key));
        self.transact(
            StorageOperation::Remove,
            IdbTransactionMode::Readwrite,
            move |txn| {
                let operation = StorageOperation::Remove;
                let after_usage = txn.clone();
                txn.then(txn.store.get(&js_key(USAGE_KEY)), move |usage| {
                    let usage = match read_usage(&usage, operation) {
                        Ok(usage) => usage,
                        Err(error) => return after_usage.set(Err(error)),
                    };
                    let after_old = after_usage.clone();
                    after_usage.then(after_usage.store.get(&name), move |old| {
                        if old.is_undefined() {
                            return after_old.set(Ok(false));
                        }
                        let len = match bytes_of(&old, operation) {
                            Ok(bytes) => bytes.length() as usize,
                            Err(error) => return after_old.set(Err(error)),
                        };
                        after_old.fire(after_old.store.delete(&name));
                        put_usage(
                            &after_old,
                            Usage {
                                entries: usage.entries.saturating_sub(1),
                                total_bytes: usage.total_bytes.saturating_sub(len),
                            },
                        );
                        after_old.set(Ok(true));
                    });
                });
            },
        )
    }

    fn clear(&self, namespace: &str) -> StorageFuture<usize> {
        if let Err(error) = validate_namespace(namespace) {
            return ready(Err(error));
        }
        let namespace = namespace.to_owned();
        self.transact(
            StorageOperation::Clear,
            IdbTransactionMode::Readwrite,
            move |txn| {
                let operation = StorageOperation::Clear;
                let range = match namespace_range(&namespace) {
                    Ok(range) => range,
                    Err(error) => return txn.set(Err(thrown(operation, &error))),
                };
                let after_usage = txn.clone();
                txn.then(txn.store.get(&js_key(USAGE_KEY)), move |usage| {
                    let usage = match read_usage(&usage, operation) {
                        Ok(usage) => usage,
                        Err(error) => return after_usage.set(Err(error)),
                    };
                    let after_values = after_usage.clone();
                    after_usage.then(after_usage.store.get_all_with_key(&range), move |values| {
                        let values = Array::from(&values);
                        let mut bytes = 0usize;
                        for value in values.iter() {
                            match bytes_of(&value, operation) {
                                Ok(value) => bytes += value.length() as usize,
                                Err(error) => return after_values.set(Err(error)),
                            }
                        }
                        let count = values.length() as usize;
                        if count == 0 {
                            return after_values.set(Ok(0));
                        }
                        after_values.fire(after_values.store.delete(&range));
                        put_usage(
                            &after_values,
                            Usage {
                                entries: usage.entries.saturating_sub(count),
                                total_bytes: usage.total_bytes.saturating_sub(bytes),
                            },
                        );
                        after_values.set(Ok(count));
                    });
                });
            },
        )
    }

    fn list(&self, namespace: &str, limit: usize) -> StorageFuture<StorageList> {
        if let Err(error) =
            validate_namespace(namespace).and_then(|()| self.limits.check_list(limit))
        {
            return ready(Err(error));
        }
        let namespace = namespace.to_owned();
        self.transact(
            StorageOperation::List,
            IdbTransactionMode::Readonly,
            move |txn| {
                let operation = StorageOperation::List;
                let range = match namespace_range(&namespace) {
                    Ok(range) => range,
                    Err(error) => return txn.set(Err(thrown(operation, &error))),
                };
                let after_keys = txn.clone();
                txn.then(txn.store.get_all_keys_with_key(&range), move |keys| {
                    let keys = Array::from(&keys);
                    let after_values = after_keys.clone();
                    after_keys.then(after_keys.store.get_all_with_key(&range), move |values| {
                        let values = Array::from(&values);
                        after_values.set(list_from(&namespace, &keys, &values, limit, operation));
                    });
                });
            },
        )
    }

    fn commit(&self, batch: StorageBatch) -> StorageFuture<()> {
        if let Err(error) = batch.validate(self.limits) {
            return ready(Err(error));
        }
        if batch.is_empty() {
            return ready(Ok(()));
        }
        let limits = self.limits;
        // One read-write transaction is the atomic unit: it commits as a whole or
        // not at all, and the usage record moves with it.
        self.transact(
            StorageOperation::Write,
            IdbTransactionMode::Readwrite,
            move |txn| {
                let operation = StorageOperation::Write;
                let ops = batch.coalesce();
                let new_lens = new_lens(&ops);
                // Copied now: the batch is gone by the time callbacks run.
                let staged: Rc<Vec<Staged>> = Rc::new(
                    ops.iter()
                        .map(|op| Staged {
                            name: js_key(&record_key(op.namespace, op.key)),
                            value: op.value.map(Uint8Array::from),
                        })
                        .collect(),
                );
                let new_lens = Rc::new(new_lens);

                let after_usage = txn.clone();
                txn.then(txn.store.get(&js_key(USAGE_KEY)), move |usage| {
                    let usage = match read_usage(&usage, operation) {
                        Ok(usage) => usage,
                        Err(error) => return after_usage.set(Err(error)),
                    };
                    // Every key's current size is requested up front, in parallel;
                    // the batch is planned when the last answer arrives.
                    let old_lens = Rc::new(RefCell::new(vec![None; staged.len()]));
                    let remaining = Rc::new(Cell::new(staged.len()));
                    for index in 0..staged.len() {
                        let (txn, staged, new_lens) =
                            (after_usage.clone(), staged.clone(), new_lens.clone());
                        let (old_lens, remaining) = (old_lens.clone(), remaining.clone());
                        after_usage.then(after_usage.store.get(&staged[index].name), move |old| {
                            if !old.is_undefined() {
                                match bytes_of(&old, operation) {
                                    Ok(bytes) => {
                                        old_lens.borrow_mut()[index] = Some(bytes.length() as usize)
                                    }
                                    Err(error) => txn.set(Err(error)),
                                }
                            }
                            remaining.set(remaining.get() - 1);
                            if remaining.get() == 0 && !txn.has_outcome() {
                                finish_batch(
                                    &txn,
                                    limits,
                                    usage,
                                    &staged,
                                    &new_lens,
                                    &old_lens.borrow(),
                                );
                            }
                        });
                    }
                });
            },
        )
    }

    fn metadata(&self, namespace: &str, key: &str) -> StorageFuture<Option<StorageMetadata>> {
        if let Err(error) = validate(namespace, key) {
            return ready(Err(error));
        }
        let name = js_key(&record_key(namespace, key));
        self.transact(
            StorageOperation::Metadata,
            IdbTransactionMode::Readonly,
            move |txn| {
                let found = txn.clone();
                txn.then(txn.store.get(&name), move |value| {
                    found.set(if value.is_undefined() {
                        Ok(None)
                    } else {
                        bytes_of(&value, StorageOperation::Metadata)
                            .map(|bytes| Some(StorageMetadata::new(bytes.length() as usize)))
                    });
                });
            },
        )
    }
}

/// One key of a batch, ready to send.
struct Staged {
    name: JsValue,
    /// The bytes to store, or `None` to remove.
    value: Option<Uint8Array>,
}

/// Plans and applies a batch once every key's current size is known.
fn finish_batch(
    txn: &Txn<()>,
    limits: StorageLimits,
    usage: Usage,
    staged: &[Staged],
    new_lens: &[Option<usize>],
    old_lens: &[Option<usize>],
) {
    match plan(limits, usage, new_lens, old_lens) {
        Err(error) => txn.set(Err(error)),
        Ok(after) => {
            for entry in staged {
                txn.fire(match &entry.value {
                    Some(value) => txn.store.put_with_key(value, &entry.name),
                    None => txn.store.delete(&entry.name),
                });
            }
            put_usage(txn, after);
            txn.set(Ok(()));
        }
    }
}

/// Pairs a namespace's keys with its values and sorts them as Rust sorts
/// strings. IndexedDB orders by UTF-16 code unit, so characters outside the
/// Basic Multilingual Plane would otherwise list in a different order here than
/// in [`MemoryStorage`](crate::MemoryStorage) and `FileStorage`.
fn list_from(
    namespace: &str,
    keys: &Array,
    values: &Array,
    limit: usize,
    operation: StorageOperation,
) -> StorageResult<StorageList> {
    let corrupt = || StorageError::Corrupt { operation };
    if keys.length() != values.length() {
        return Err(corrupt());
    }
    let mut entries = Vec::with_capacity(keys.length() as usize);
    for (record, value) in keys.iter().zip(values.iter()) {
        let record = record.as_string().ok_or_else(corrupt)?;
        let key = key_of(&record, namespace).ok_or_else(corrupt)?;
        entries.push((
            key.to_owned(),
            bytes_of(&value, operation)?.length() as usize,
        ));
    }
    entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    Ok(StorageList::from_sorted(entries, limit))
}
