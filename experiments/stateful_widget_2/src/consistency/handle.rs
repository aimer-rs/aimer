//! The updater as the consistency suite sees it.
//!
//! Every backend has its own native updater (`StateUpdater<S>` today, a
//! different shape in the redesigns). Test fixtures must not name any of them,
//! so they receive a [`Handle`]: a type-erased wrapper the backend's adapter
//! builds around its native updater. Every call still goes to the native
//! updater, the handle only hides which one it is.

use std::cell::RefCell;
use std::rc::Rc;

/// What a backend's adapter implements to expose its native updater.
///
/// Object-safe on purpose: generic callbacks are wrapped by [`Handle`].
pub trait Ops<S: 'static> {
    fn set_state(&self, mutation: Box<dyn FnOnce(&mut S)>);

    /// Returns whether the mutation was accepted (a live, healthy state).
    fn try_set_state(&self, mutation: Box<dyn FnOnce(&mut S)>) -> bool;

    /// The native `read`: panics when the state cannot be resolved at all.
    ///
    /// Kept apart from `try_read` because the two differ on purpose: `read`
    /// still reaches the state of an element that has failed, `try_read` does not.
    fn read(&self, read: &mut dyn FnMut(&S));

    /// Returns whether the callback ran.
    fn try_read(&self, read: &mut dyn FnMut(&S)) -> bool;

    fn has_state(&self) -> bool;
}

/// Backend-neutral handle to a live state.
pub struct Handle<S: 'static> {
    ops: Rc<dyn Ops<S>>,
}

impl<S: 'static> Clone for Handle<S> {
    fn clone(&self) -> Self {
        Self { ops: self.ops.clone() }
    }
}

impl<S: 'static> Handle<S> {
    pub fn new(ops: impl Ops<S> + 'static) -> Self {
        Self { ops: Rc::new(ops) }
    }

    /// Queues `mutation`; applied at the next rebuild.
    pub fn set_state(&self, mutation: impl FnOnce(&mut S) + 'static) {
        self.ops.set_state(Box::new(mutation));
    }

    /// Like [`Handle::set_state`], but `None` (and no queueing) when the state is gone.
    pub fn try_set_state(&self, mutation: impl FnOnce(&mut S) + 'static) -> Option<()> {
        self.ops.try_set_state(Box::new(mutation)).then_some(())
    }

    pub fn read<R>(&self, read: impl FnOnce(&S) -> R) -> R {
        let mut read = Some(read);
        let mut out = None;
        self.ops.read(&mut |state| {
            out = Some((read.take().expect("the callback runs at most once"))(state));
        });
        out.expect("a native read either runs the callback or panics")
    }

    pub fn try_read<R>(&self, read: impl FnOnce(&S) -> R) -> Option<R> {
        let mut read = Some(read);
        let mut out = None;
        self.ops.try_read(&mut |state| {
            out = Some((read.take().expect("the callback runs at most once"))(state));
        });
        out
    }

    pub fn has_state(&self) -> bool {
        self.ops.has_state()
    }
}

/// A slot a fixture fills with its handle, so a test can reach the state a
/// fixture's widget created without any backend-specific constructor.
pub struct Probe<S: 'static>(Rc<RefCell<Option<Handle<S>>>>);

impl<S: 'static> Clone for Probe<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S: 'static> Default for Probe<S> {
    fn default() -> Self {
        Self(Rc::default())
    }
}

impl<S: 'static> Probe<S> {
    /// Called by the fixture's `build`; the latest build wins.
    pub fn capture(&self, handle: &Handle<S>) {
        *self.0.borrow_mut() = Some(handle.clone());
    }

    /// The handle captured by the most recent build.
    pub fn handle(&self) -> Handle<S> {
        self.0.borrow().clone().expect("the fixture has built at least once")
    }
}
