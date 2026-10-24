//! The plumbing between IndexedDB's event callbacks and a [`StorageFuture`].
//!
//! # Why callbacks and not `await`
//!
//! An IndexedDB transaction stays open only while it has a request in flight
//! and only while control is inside one of its event handlers. A Venus task is
//! woken as a scheduled microtask, a later phase of the frame, by which time the
//! transaction has already committed and the next `put` would throw
//! `TransactionInactiveError`. So the work *inside* a transaction is a chain of
//! `onsuccess` closures, and only the final result crosses into a future.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use wasm_bindgen::prelude::*;
use web_sys::{DomException, IdbObjectStore, IdbRequest, IdbTransaction};

use crate::error::{
    StorageCapability, StorageError, StorageIoKind, StorageOperation, StorageResult,
};

struct Slot<T> {
    result: Option<StorageResult<T>>,
    waker: Option<Waker>,
}

/// The browser-side half of a result slot. Dropping it without completing
/// resolves the future with an error instead of leaving it pending forever.
pub(super) struct Completer<T> {
    slot: Rc<RefCell<Slot<T>>>,
    operation: StorageOperation,
    completed: bool,
}

/// The awaiting half of a result slot.
struct Receive<T> {
    slot: Rc<RefCell<Slot<T>>>,
}

/// Creates a result slot and the future that observes it.
pub(super) fn channel<T: 'static>(
    operation: StorageOperation,
) -> (Completer<T>, crate::StorageFuture<T>) {
    let slot = Rc::new(RefCell::new(Slot {
        result: None,
        waker: None,
    }));
    (
        Completer {
            slot: Rc::clone(&slot),
            operation,
            completed: false,
        },
        Box::pin(Receive { slot }),
    )
}

impl<T> Completer<T> {
    fn finish(&mut self, result: StorageResult<T>) {
        self.completed = true;
        let waker = {
            let mut slot = self.slot.borrow_mut();
            slot.result = Some(result);
            slot.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<T> Drop for Completer<T> {
    fn drop(&mut self) {
        if !self.completed {
            let error = StorageError::Io {
                operation: self.operation,
                kind: StorageIoKind::Other,
            };
            self.finish(Err(error));
        }
    }
}

impl<T> Future for Receive<T> {
    type Output = StorageResult<T>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = self.slot.borrow_mut();
        match slot.result.take() {
            Some(result) => Poll::Ready(result),
            None => {
                slot.waker = Some(context.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// A completer shared by the several closures that may end an operation.
///
/// The first to call [`Finish::done`] wins; later calls are ignored, which is
/// what lets both `oncomplete` and `onabort` hold one.
pub(super) struct Finish<T>(Rc<RefCell<Option<Completer<T>>>>);

impl<T> Clone for Finish<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T> Finish<T> {
    pub(super) fn new(completer: Completer<T>) -> Self {
        Self(Rc::new(RefCell::new(Some(completer))))
    }

    pub(super) fn done(&self, result: StorageResult<T>) {
        // Taken out first: waking the task must not happen under the borrow.
        let completer = self.0.borrow_mut().take();
        if let Some(mut completer) = completer {
            completer.finish(result);
        }
    }
}

/// Maps a browser exception onto the storage error vocabulary.
pub(super) fn dom_error(
    operation: StorageOperation,
    exception: Option<DomException>,
) -> StorageError {
    let name = exception.map(|exception| exception.name());
    match name.as_deref() {
        Some("QuotaExceededError") => StorageError::Io {
            operation,
            kind: StorageIoKind::StorageFull,
        },
        Some("ReadOnlyError") => StorageError::Io {
            operation,
            kind: StorageIoKind::ReadOnly,
        },
        Some("SecurityError" | "NotAllowedError") => StorageError::PermissionDenied { operation },
        Some("DataError" | "DataCloneError") => StorageError::Corrupt { operation },
        _ => StorageError::Io {
            operation,
            kind: StorageIoKind::Other,
        },
    }
}

/// The error for a call the browser refused outright, before any event.
pub(super) fn thrown(operation: StorageOperation, error: &JsValue) -> StorageError {
    dom_error(operation, error.clone().dyn_into::<DomException>().ok())
}

/// IndexedDB is missing, or refused to open (private browsing, blocked storage).
pub(super) fn unavailable() -> StorageError {
    StorageError::Unavailable {
        capability: StorageCapability::WebIndexedDb,
    }
}

/// One operation's transaction: the store to issue requests against, and the
/// slot the final answer is written to.
///
/// The answer is held until the transaction *completes*, not until the last
/// request succeeds, so an acknowledged write has been committed.
pub(super) struct Txn<T> {
    pub(super) store: IdbObjectStore,
    operation: StorageOperation,
    outcome: Rc<RefCell<Option<StorageResult<T>>>>,
}

impl<T> Clone for Txn<T> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            operation: self.operation,
            outcome: Rc::clone(&self.outcome),
        }
    }
}

impl<T: 'static> Txn<T> {
    /// Wires `transaction` to deliver this operation's outcome to `finish`.
    pub(super) fn attach(
        transaction: &IdbTransaction,
        store: IdbObjectStore,
        operation: StorageOperation,
        finish: Finish<T>,
    ) -> Self {
        let outcome: Rc<RefCell<Option<StorageResult<T>>>> = Rc::new(RefCell::new(None));

        // One closure for both events. A transaction ends with exactly one of
        // `complete` or `abort`, so the closure is called exactly once and a
        // `Closure::once` frees itself; two closures would leave the one that
        // never fires behind on every successful operation.
        //
        // An aborted transaction covers failed requests too: a request error
        // that nobody handles aborts its transaction.
        let on_end = {
            let (outcome, transaction) = (Rc::clone(&outcome), transaction.clone());
            Closure::once(move |event: web_sys::Event| {
                let result = if event.type_() == "complete" {
                    outcome.borrow_mut().take().unwrap_or(Err(StorageError::Io {
                        operation,
                        kind: StorageIoKind::Other,
                    }))
                } else {
                    Err(dom_error(operation, transaction.error()))
                };
                finish.done(result);
            })
        };
        transaction.set_oncomplete(Some(on_end.as_ref().unchecked_ref()));
        transaction.set_onabort(Some(on_end.as_ref().unchecked_ref()));
        on_end.forget();

        Self {
            store,
            operation,
            outcome,
        }
    }

    /// Records the operation's answer. The first answer wins.
    pub(super) fn set(&self, result: StorageResult<T>) {
        let mut outcome = self.outcome.borrow_mut();
        if outcome.is_none() {
            *outcome = Some(result);
        }
    }

    /// Whether an answer, including a failure, has already been recorded.
    ///
    /// Callbacks that fan out several requests check this before acting on the
    /// last one, so a failure in one of them stops the rest from writing.
    pub(super) fn has_outcome(&self) -> bool {
        self.outcome.borrow().is_some()
    }

    /// Calls `handler` with the result of `request` once it succeeds.
    ///
    /// A request the browser refuses to create fails the operation instead, and
    /// a request that fails after being created aborts the transaction.
    pub(super) fn then(
        &self,
        request: Result<IdbRequest, JsValue>,
        handler: impl FnOnce(JsValue) + 'static,
    ) {
        match request {
            Ok(request) => {
                let target = request.clone();
                // Registered for `error` as well so the closure is freed whichever
                // way the request ends; only a success runs the handler. The error
                // itself is not handled here, so it aborts the transaction.
                let on_end = Closure::once(move |event: web_sys::Event| {
                    if event.type_() == "success" {
                        handler(target.result().unwrap_or(JsValue::UNDEFINED));
                    }
                });
                request.set_onsuccess(Some(on_end.as_ref().unchecked_ref()));
                request.set_onerror(Some(on_end.as_ref().unchecked_ref()));
                on_end.forget();
            }
            Err(error) => self.set(Err(thrown(self.operation, &error))),
        }
    }

    /// Issues a request whose result does not matter, only that it is accepted.
    pub(super) fn fire(&self, request: Result<IdbRequest, JsValue>) {
        if let Err(error) = request {
            self.set(Err(thrown(self.operation, &error)));
        }
    }
}
