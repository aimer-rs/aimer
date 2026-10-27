use std::any::Any;
use std::cell::{Cell, RefCell, UnsafeCell};
#[cfg(feature = "portable-guest")]
use std::cell::Ref;
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::{Rc, Weak};

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
#[cfg(not(aimer_portable_guest))]
use aimer_events::window::{request_animation_frame, request_scoped_frame};
use aimer_utils::{debug, error};

use crate::base::*;
use crate::components::element::{
    DirtySource, identities_are_compatible, structural_children,
};
use super::state_slots::{
    StateSlotReadGuard, StateSlotResolutionError, StateStorage, StateUpdaterGeneration,
};
use crate::widget::recovery::{BuildPhase, PanicDiagnostic, recover_operation};
use crate::{
    AnyElement, Drawable, Element, EventElement, EventResult, EventTreeRole, LayoutElement,
    Rebuildable, VisitorElement, Widget,
};

trait FetchAdd {
    fn fetch_add(&self, val: u64) -> u64;
}

impl FetchAdd for Cell<u64> {
    fn fetch_add(&self, val: u64) -> u64 {
        let previous = self.get();
        self.set(previous.wrapping_add(val));
        previous
    }
}

/// A `Send + Sync` wrapper around `UnsafeCell<AnyElement>`.
/// Safety: the rendering pipeline is single-threaded, so concurrent access does
/// not occur.
///
/// `pub(crate)` so `StatelessElement` can reuse the same swappable-child slot
/// (needed so `visit_children<'a>` can hand out `&'a` refs to a child that may
/// be replaced on rebuild).
pub(crate) struct SyncChild(pub(crate) UnsafeCell<AnyElement>);
unsafe impl Send for SyncChild {}
unsafe impl Sync for SyncChild {}

/// A `Send + Sync` wrapper around an optionally-owned state value.
///
/// The value is stored in an option so reconciliation can move a freshly-built
/// state into the preserved live state without leaving `StateUpdater`s that
/// still reference the source cell pointing at invalid memory.
///
/// Safety: the rendering pipeline is single-threaded. Mutations are applied
/// exclusively during `rebuild_if_dirty` on the render thread, and reads
/// happen only on the render thread (event handlers, build).
struct SyncState<S>(UnsafeCell<Option<S>>);
unsafe impl<S: Send> Send for SyncState<S> {}
unsafe impl<S: Send> Sync for SyncState<S> {}

/// A `Sync` wrapper for the rebuild closure so `StatefulElement` can replace
/// it during `adopt_state_from` (reconciliation) without requiring `&mut self`.
/// Safety: the rendering pipeline is single-threaded; the closure is only
/// invoked from `rebuild_if_dirty` on the render thread.
struct SyncRebuildFn(UnsafeCell<Rc<RebuildCallBack>>);
unsafe impl Send for SyncRebuildFn {}
unsafe impl Sync for SyncRebuildFn {}

/// A `Send + Sync` wrapper around the type-erased state cell (`Rc<dyn Any>`,
/// concretely `Rc<SyncState<W::State>>`). Kept so a reconciling element can
/// hand its freshly-built state to the live element for a config refresh,
/// without `StatefulElement` being generic over `W`.
///
/// Wrapped in `UnsafeCell` so `adopt_state_from` can *repoint* it to the OLD
/// element's state cell alongside `rebuild_fn`: after adoption the live element
/// reads the OLD cell, so its config-refresh machinery (`state_any` +
/// `adopt_config_fn`) must reference that SAME cell, otherwise a later
/// reconcile that uses this element as the `old` side would refresh an
/// orphaned cell while the live `rebuild_fn` keeps reading a stale one.
/// Safety: the rendering pipeline is single-threaded.
struct SyncStateAny(UnsafeCell<Rc<dyn Any>>);
unsafe impl Send for SyncStateAny {}
unsafe impl Sync for SyncStateAny {}

/// Type-erased "adopt the widget configuration from another element's state
/// into mine" hook. Captures this element's state cell (typed as `W::State`);
/// downcasts the supplied `&dyn Any` (another element's `SyncState<W::State>`),
/// takes its state, and calls `State::adopt_config_from`. No-op when the
/// concrete types differ or the source state was already consumed.
type AdoptConfigCallBack = dyn Fn(&dyn Any);

/// A `Send + Sync` wrapper around the config-adoption closure.
///
/// Wrapped in `UnsafeCell` for the same reason as [`SyncStateAny`]: it must be
/// repointed to the OLD element's cell during `adopt_state_from` so it stays in
/// sync with the adopted `rebuild_fn`.
/// Safety: invoked only during single-threaded reconciliation.
struct SyncAdoptConfigFn(UnsafeCell<Rc<AdoptConfigCallBack>>);
unsafe impl Send for SyncAdoptConfigFn {}
unsafe impl Sync for SyncAdoptConfigFn {}

/// Type-erased state mutation applied during the next rebuild.
type StateMutation<S> = Box<dyn FnOnce(&mut S)>;

struct StateMutationQueue<S> {
    mutations: RefCell<VecDeque<StateMutation<S>>>,
}

struct FailureState {
    message: RefCell<Option<String>>,
}

impl FailureState {
    fn record(&self, diagnostic: &PanicDiagnostic) -> bool {
        let mut message = self.message.borrow_mut();
        if message.is_none() {
            *message = Some(diagnostic.to_string());
            true
        } else {
            false
        }
    }

    fn record_message(&self, message: String) {
        let mut current = self.message.borrow_mut();
        if current.is_none() {
            *current = Some(message);
        }
    }

    fn message(&self) -> Option<String> {
        self.message.borrow().clone()
    }

    fn error_element(&self) -> Option<AnyElement> {
        self.message
            .borrow()
            .as_ref()
            .map(|message| crate::ErrorElement::new(message.clone()).boxed())
    }

    fn is_failed(&self) -> bool {
        self.message.borrow().is_some()
    }
}

fn recover_failure(
    failure: &FailureState,
    failed: &Cell<bool>,
    dirty_source: &DirtySource,
    diagnostic: PanicDiagnostic,
) -> AnyElement {
    failed.set(true);
    dirty_source.clear();
    if failure.record(&diagnostic) {
        diagnostic.into_error_element()
    } else {
        failure
            .error_element()
            .expect("a recorded failure should have an error element")
    }
}

#[track_caller]
fn panic_state_updater_resolution(error: StateSlotResolutionError) -> ! {
    let message = match error {
        StateSlotResolutionError::Missing => {
            "StateUpdater refers to a missing state slot"
        }
        StateSlotResolutionError::Stale => {
            "StateUpdater refers to a dropped or stale state slot"
        }
        StateSlotResolutionError::Consumed => {
            "StateUpdater refers to a state consumed during reconciliation"
        }
        StateSlotResolutionError::WrongType => {
            "StateUpdater refers to a state slot with the wrong type"
        }
    };
    error!("{message}: {:?}", error);
    panic!("{message} (see error above)");
}

#[track_caller]
fn panic_consumed_state() -> ! {
    panic!("StateUpdater state was consumed during reconciliation");
}

impl Default for FailureState {
    fn default() -> Self {
        Self {
            message: RefCell::new(None),
        }
    }
}

impl<S> Default for StateMutationQueue<S> {
    fn default() -> Self {
        Self {
            mutations: RefCell::new(VecDeque::new()),
        }
    }
}

impl<S> StateMutationQueue<S> {
    #[inline]
    fn push(&self, mutation: StateMutation<S>) {
        self.mutations.borrow_mut().push_back(mutation);
    }

    #[inline]
    fn pop_front(&self) -> Option<StateMutation<S>> {
        self.mutations.borrow_mut().pop_front()
    }

    fn drain_into(&self, state: &mut S, mut on_applied: impl FnMut()) -> usize {
        let mut applied = 0;
        while let Some(mutation) = self.pop_front() {
            mutation(state);
            on_applied();
            applied += 1;
        }
        applied
    }
}

/// A handle that allows StatefulWidgets to trigger state mutations and
/// rebuilds.
///
/// Mutations are queued as closures and applied on the render thread during
/// the next rebuild. The queue is local to the UI thread, so it needs neither
/// locking nor cross-thread channel storage.
///
/// `StateUpdater` is intentionally confined to the UI thread:
///
/// ```compile_fail
/// fn require_send<T: Send>() {}
/// fn require_sync<T: Sync>() {}
///
/// require_send::<aimer_widget::StateUpdater<()>>();
/// require_sync::<aimer_widget::StateUpdater<()>>();
/// ```
pub struct StateUpdater<S> {
    slot: Option<StateUpdaterGeneration<UpdaterSlot<S>>>,
    _ui_thread_only: PhantomData<Rc<()>>,
}

enum UpdaterSlot<S> {
    Native(StateUpdaterInner<S>),
    #[cfg(feature = "portable-guest")]
    Portable(crate::portable::state::PortableStateHandle<S>),
}

impl<S> Copy for StateUpdater<S> {}

impl<S> Clone for StateUpdater<S> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

/// A synchronous borrow returned by a state updater.
///
/// The guard retains an arena read lease. Native state uses a pointer valid for
/// that lease, while portable state additionally holds its `RefCell` borrow.
/// Both variants dereference to `S`, preserving ordinary `read_state().field`
/// use.
#[doc(hidden)]
pub struct StateReadGuard<'a, S: 'static> {
    kind: StateReadGuardKind<S>,
    _marker: PhantomData<&'a S>,
}

enum StateReadGuardKind<S: 'static> {
    Native {
        state: *const S,
        _slot: StateSlotReadGuard<UpdaterSlot<S>>,
    },
    #[cfg(feature = "portable-guest")]
    Portable {
        state: Ref<'static, S>,
        _slot: StateSlotReadGuard<UpdaterSlot<S>>,
    },
}

impl<S: 'static> std::ops::Deref for StateReadGuard<'_, S> {
    type Target = S;

    #[inline]
    fn deref(&self) -> &Self::Target {
        match &self.kind {
            StateReadGuardKind::Native { state, .. } => {
                // SAFETY: the native variant owns `slot`, whose read lease
                // keeps the erased updater slot and its state alive.
                unsafe { &**state }
            }
            #[cfg(feature = "portable-guest")]
            StateReadGuardKind::Portable { state, .. } => state,
        }
    }
}

struct StateUpdaterInner<S> {
    /// Shared UI-thread queue for pending state mutations.
    tx: Rc<StateMutationQueue<S>>,
    /// Shared state for synchronous reads on the render thread.
    state: Rc<SyncState<S>>,
    dirty_source: Rc<DirtySource>,
    failure: Rc<FailureState>,
    /// The window the element was built for, so a state update made later,
    /// by a timer, by a finished request, asks that window for its frame no
    /// matter which window is active when it happens.
    window: aimer_events::window::WindowKey,
}

impl<S: 'static> StateUpdater<S> {
    /// Create a new `StateUpdater` from a channel sender, shared state, and a
    /// tracked dirty source.
    #[inline]
    fn with(
        tx: Rc<StateMutationQueue<S>>,
        state: Rc<SyncState<S>>,
        dirty_source: Rc<DirtySource>,
        failure: Rc<FailureState>,
        window: aimer_events::window::WindowKey,
    ) -> (StateStorage, Self) {
        let (storage, slot) = StateStorage::insert(UpdaterSlot::Native(StateUpdaterInner {
                tx,
                state,
                dirty_source,
                failure,
                window,
            }));
        (
            storage,
            Self {
                slot: Some(slot),
                _ui_thread_only: PhantomData,
            },
        )
    }

    /// Creates the hidden updater backend used by portable generated wrappers.
    #[cfg(feature = "portable-guest")]
    #[doc(hidden)]
    #[inline]
    pub(crate) fn from_portable(
        handle: crate::portable::state::PortableStateHandle<S>,
    ) -> (StateStorage, Self) {
        let (storage, slot) = StateStorage::insert(UpdaterSlot::Portable(handle));
        (
            storage,
            Self {
                slot: Some(slot),
                _ui_thread_only: PhantomData,
            },
        )
    }

    #[track_caller]
    pub fn read_state(&self) -> StateReadGuard<'_, S> {
        let slot = match self.slot {
            Some(slot) => slot,
            None => {
                let loc = Location::caller();
                error!("Attempted to read state from an uninitialized StateUpdater");
                self.beautiful_error(loc);
                panic!("State is not initialized (see error above)");
            }
        };
        let slot = slot
            .try_read_guard()
            .unwrap_or_else(|error| panic_state_updater_resolution(error));
        match &*slot {
            UpdaterSlot::Native(inner) => {
                let state = unsafe {
                    (&*inner.state.0.get())
                        .as_ref()
                        .unwrap_or_else(|| panic_consumed_state())
                        as *const S
                };
                StateReadGuard {
                    kind: StateReadGuardKind::Native {
                        state,
                        _slot: slot,
                    },
                    _marker: PhantomData,
                }
            }
            #[cfg(feature = "portable-guest")]
            UpdaterSlot::Portable(handle) => {
                let state = handle.borrow();
                // The slot read guard owns the `PortableStateHandle` until the
                // `RefCell` borrow is dropped, so extending this borrow to the
                // guard's owned lifetime is sound.
                let state = unsafe {
                    std::mem::transmute::<Ref<'_, S>, Ref<'static, S>>(state)
                };
                StateReadGuard {
                    kind: StateReadGuardKind::Portable {
                        state,
                        _slot: slot,
                    },
                    _marker: PhantomData,
                }
            }
        }
    }

    /// Reads the current state without checking updater validity.
    ///
    /// The returned guard retains the state slot until it is dropped, just
    /// like [`Self::read_state`]. This method is intended for hot paths that
    /// already establish the updater's lifetime and type invariants.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this updater is initialized, its owning
    /// element or portable context is still live, its slot contains the
    /// expected state, and the state has not been consumed or failed. The
    /// call must remain on the UI thread. Portable callers must also ensure
    /// that no conflicting `RefCell` borrow is active. Violating these
    /// requirements is undefined behavior. Prefer [`Self::try_read_state`]
    /// when the updater may outlive its state.
    #[track_caller]
    pub unsafe fn read_state_unchecked(&self) -> StateReadGuard<'_, S> {
        // SAFETY: the caller guarantees that this updater is initialized.
        let slot = unsafe { self.slot.unwrap_unchecked() };
        // SAFETY: the caller guarantees a live, correctly typed slot and a
        // live owner lease while the read guard is acquired.
        let slot = unsafe { slot.read_guard_unchecked() };
        match &*slot {
            UpdaterSlot::Native(inner) => {
                // SAFETY: the caller guarantees that the native state exists
                // and has not been consumed or failed.
                let state = unsafe {
                    (&*inner.state.0.get()).as_ref().unwrap_unchecked() as *const S
                };
                StateReadGuard {
                    kind: StateReadGuardKind::Native {
                        state,
                        _slot: slot,
                    },
                    _marker: PhantomData,
                }
            }
            #[cfg(feature = "portable-guest")]
            UpdaterSlot::Portable(handle) => {
                let state = handle.borrow();
                // The slot read guard owns the `PortableStateHandle` until the
                // `RefCell` borrow is dropped, so extending this borrow to the
                // guard's owned lifetime is sound.
                let state = unsafe {
                    std::mem::transmute::<Ref<'_, S>, Ref<'static, S>>(state)
                };
                StateReadGuard {
                    kind: StateReadGuardKind::Portable {
                        state,
                        _slot: slot,
                    },
                    _marker: PhantomData,
                }
            }
        }
    }

    /// Returns whether this updater still resolves to a usable live state.
    ///
    /// This is a quiet check: it returns `false` for an empty, stale,
    /// consumed, failed, or wrong-type slot and never emits a diagnostic.
    #[inline]
    pub fn has_state(&self) -> bool {
        let Some(slot) = self.slot else {
            return false;
        };
        slot.try_resolve(|slot| match slot {
            UpdaterSlot::Native(inner) => {
                if inner.failure.is_failed() {
                    return false;
                }
                // Safety: the native rendering pipeline is single-threaded.
                unsafe { (&*inner.state.0.get()).is_some() }
            }
            #[cfg(feature = "portable-guest")]
            UpdaterSlot::Portable(_) => true,
        })
        .unwrap_or(false)
    }

    /// Attempts to borrow the current state without panicking when this
    /// updater is empty or no longer resolves to a live slot.
    ///
    /// The callback is not invoked for an empty, stale, consumed, failed, or
    /// wrong-type slot. Use this form for callbacks that may outlive the
    /// element which created the updater.
    #[inline]
    pub fn try_read<R>(&self, read: impl FnOnce(&S) -> R) -> Option<R> {
        let slot = self.slot?;
        slot.try_resolve(|slot| match slot {
            UpdaterSlot::Native(inner) => {
                if inner.failure.is_failed() {
                    return None;
                }
                // Safety: the native rendering pipeline is single-threaded.
                let state = unsafe { (&*inner.state.0.get()).as_ref()? };
                Some(read(state))
            }
            #[cfg(feature = "portable-guest")]
            UpdaterSlot::Portable(handle) => {
                let state = handle.try_borrow()?;
                Some(read(&state))
            }
        })
        .ok()
        .flatten()
    }

    /// Create an empty `StateUpdater` that is not yet initialized.
    /// Calling `set_state` or `read` on an empty updater will panic.
    ///
    /// It has the same functionality as `StateUpdater<S>::empty`
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::empty()
    }

    /// Create an empty `StateUpdater` that is not yet initialized.
    /// Calling `set_state` or `read` on an empty updater will panic.
    ///
    /// It has the same functionality as `StateUpdater<S>::new`
    #[inline]
    pub fn empty() -> Self {
        Self {
            slot: None,
            _ui_thread_only: PhantomData,
        }
    }

    /// Mutate the state using a value that is cloned once and moved into the
    /// mutation closure. This avoids the double-clone that would otherwise be
    /// needed when calling `set_state` from inside an `Fn` closure:
    ///
    /// ```ignore
    /// // Before (two clones):
    /// let id = item.id.clone();          // clone 1 – for the Fn capture
    /// move || {
    ///     let id = id.clone();           // clone 2 – for the 'static FnOnce
    ///     updater.set_state(move |s| { /* use id */ });
    /// }
    ///
    /// // After (one clone):
    /// let id = item.id.clone();          // clone 1 – for the Fn capture
    /// move || {
    ///     updater.set_state_with(id.clone(), |s, id| { /* use id */ });
    /// }
    /// ```
    ///
    /// Wait, that's still `id.clone()`. The real win is that `set_state_with`
    /// accepts a *reference* and clones internally, so from an `Fn` closure you
    /// can write:
    ///
    /// ```ignore
    /// let id = item.id.clone();          // clone 1 – captured by the Fn
    /// move || {
    ///     updater.set_state_with(&id, |s, id| { /* use owned id */ });
    /// }
    /// ```
    #[track_caller]
    pub fn set_state_with<V: Clone + Send + 'static>(
        &self,
        value: &V,
        f: impl FnOnce(&mut S, V) + Send + 'static,
    ) {
        let owned = value.clone();
        self.set_state(move |s| f(s, owned));
    }

    /// Mutate the state by sending a closure through the channel.
    /// The mutation will be applied on the render thread during the next
    /// rebuild. This is deadlock-free: it never acquires a lock.
    ///
    /// Multiple calls between frames are coalesced: the dirty flag is set once,
    /// and only a single rebuild happens during the next `draw`.
    #[track_caller]
    pub fn set_state(&self, f: impl FnOnce(&mut S) + 'static) {
        crate::frame_work_stats::record_state_update();
        let slot = match self.slot {
            Some(slot) => slot,
            None => {
                let loc = Location::caller();
                self.beautiful_error(loc);
                panic!("State is not initialized (see error above)");
            }
        };
        let result = slot.try_resolve(|slot| match slot {
            UpdaterSlot::Native(inner) => {
                if inner.failure.is_failed() {
                    return;
                }
                inner.tx.push(Box::new(f));
                // Only request a redraw if this is the first set_state since the last rebuild.
                // This coalesces multiple set_state calls into a single redraw request.
                if inner.dirty_source.mark() {
                    #[cfg(not(aimer_portable_guest))]
                    {
                        crate::components::element::note_state_update_marked();
                        request_state_frame(inner.window);
                    }
                }
            }
            #[cfg(feature = "portable-guest")]
            UpdaterSlot::Portable(handle) => handle.queue(f),
        });
        if let Err(error) = result {
            panic_state_updater_resolution(error);
        }
    }

    /// Attempts to queue a state mutation without panicking when this updater
    /// is empty or no longer resolves to a live slot.
    ///
    /// Returns `Some(())` only when the mutation was accepted. A rejected
    /// mutation is dropped without invoking it, marking the state dirty, or
    /// requesting a frame.
    #[inline]
    pub fn try_set_state(&self, mutation: impl FnOnce(&mut S) + 'static) -> Option<()> {
        let Some(slot) = self.slot else {
            return None;
        };
        slot.try_resolve(|slot| match slot {
            UpdaterSlot::Native(inner) => {
                if inner.failure.is_failed() {
                    return None;
                }
                // Safety: the native rendering pipeline is single-threaded.
                unsafe { (&*inner.state.0.get()).as_ref() }?;
                inner.tx.push(Box::new(mutation));
                if inner.dirty_source.mark() {
                    #[cfg(not(aimer_portable_guest))]
                    {
                        crate::components::element::note_state_update_marked();
                        request_state_frame(inner.window);
                    }
                }
                Some(())
            }
            #[cfg(feature = "portable-guest")]
            UpdaterSlot::Portable(handle) => {
                handle.queue(mutation);
                Some(())
            }
        })
        .ok()
        .flatten()
    }

    /// Queues a state mutation without performing updater validity checks.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this updater is initialized, its owning
    /// element or portable context is still live, the updater's state slot has
    /// not been consumed or failed, and the call is made on the UI thread.
    /// Violating those requirements is undefined behavior. Prefer
    /// [`Self::try_set_state`] when the updater may outlive its state.
    #[track_caller]
    pub unsafe fn set_state_unchecked(&self, mutation: impl FnOnce(&mut S) + 'static) {
        crate::frame_work_stats::record_state_update();
        // SAFETY: the caller guarantees that this updater is initialized.
        let slot = unsafe { self.slot.unwrap_unchecked() };
        // SAFETY: the caller guarantees a live, correctly typed slot and a
        // live owner lease for the duration of this lookup.
        unsafe {
            slot.resolve_unchecked(|slot| match slot {
                UpdaterSlot::Native(inner) => {
                    inner.tx.push(Box::new(mutation));
                    if inner.dirty_source.mark() {
                        #[cfg(not(aimer_portable_guest))]
                        {
                            crate::components::element::note_state_update_marked();
                            request_state_frame(inner.window);
                        }
                    }
                }
                #[cfg(feature = "portable-guest")]
                UpdaterSlot::Portable(handle) => handle.queue(mutation),
            });
        }
    }

    /// Read the current state without marking dirty.
    ///
    /// Safety: this reads from the `UnsafeCell` directly. It is safe because
    /// reads only happen on the render thread (event handlers, build methods),
    /// and mutations are also applied exclusively on the render thread during
    /// `rebuild_if_dirty`.
    #[track_caller]
    pub fn read<R>(&self, f: impl FnOnce(&S) -> R) -> R {
        let slot = match self.slot {
            Some(slot) => slot,
            None => {
                let loc = Location::caller();
                #[cfg(not(target_os = "ios"))]
                self.beautiful_error(loc);
                panic!("State is not initialized (see error above)");
            }
        };
        slot.try_resolve(|slot| match slot {
            UpdaterSlot::Native(inner) => {
                // Safety: single-threaded rendering pipeline, no concurrent mutation.
                let state = unsafe {
                    (&*inner.state.0.get())
                        .as_ref()
                        .unwrap_or_else(|| panic_consumed_state())
                };
                f(state)
            }
            #[cfg(feature = "portable-guest")]
            UpdaterSlot::Portable(handle) => handle.read(f),
        })
        .unwrap_or_else(|error| panic_state_updater_resolution(error))
    }

    /// Attempts to obtain the checked guard used by [`Self::read_state`].
    ///
    /// The returned guard keeps the resolved slot live for as long as its
    /// borrowed state is used. It returns `None` for empty, stale, consumed,
    /// failed, or wrong-type slots and never emits a stale-callback diagnostic.
    #[inline]
    pub fn try_read_state(&self) -> Option<StateReadGuard<'_, S>> {
        let slot = self.slot?;
        let slot = slot.try_read_guard().ok()?;
        match &*slot {
            UpdaterSlot::Native(inner) => {
                if inner.failure.is_failed() {
                    return None;
                }
                let state = unsafe { (&*inner.state.0.get()).as_ref()? as *const S };
                Some(StateReadGuard {
                    kind: StateReadGuardKind::Native {
                        state,
                        _slot: slot,
                    },
                    _marker: PhantomData,
                })
            }
            #[cfg(feature = "portable-guest")]
            UpdaterSlot::Portable(handle) => {
                let state = handle.try_borrow()?;
                // The slot read guard owns the portable handle until the
                // RefCell borrow is dropped, so extending this borrow to the
                // returned guard's owned lifetime is sound.
                let state = unsafe {
                    std::mem::transmute::<Ref<'_, S>, Ref<'static, S>>(state)
                };
                Some(StateReadGuard {
                    kind: StateReadGuardKind::Portable {
                        state,
                        _slot: slot,
                    },
                    _marker: PhantomData,
                })
            }
        }
    }

    #[inline]
    fn beautiful_error(&self, loc: &Location) {
        {
            const BRACE: &str = "{";
            error!(
                "State is not initialized and trying to read or update at {}:{}
   {}
   {} impl State<YourStatefulWidget> for YourWidgetState {BRACE}
   {}
   {}     fn init_state(&mut self, _updater: StateUpdater<Self>)
   {}         where
   {}             Self: Sized,
   {}         {{
   {}             self.updater = _updater;
   {}             {}
   {}         }}
   {}
   {}: call `self.updater = _updater` inside `init_state`
",
                loc.file(),
                loc.line(),
                "|",
                "|",
                "|",
                "|",
                "|",
                "|",
                "|",
                "|",
                "|",
                "^^^^^^^^^^^^^^^^^^^^^^^^^ add this line to prevent panic",
                "|",
                "|",
                "help",
            );
        }
    }
}

/// A widget whose UI depends on data that changes while it is on screen.
///
/// The split is deliberate. The widget is only *configuration*: the immutable
/// props a parent passes down, rebuilt freely and thrown away. Everything that
/// must survive a rebuild, a counter, hover or focus flags, scroll offsets,
/// animation progress, lives in the [`State`](StatefulWidget::State) that
/// [`create_state`](StatefulWidget::create_state) returns. The element owns that
/// state and keeps it alive for as long as the widget stays in the tree.
///
/// When a parent rebuilds and emits this widget again, the new widget creates a
/// fresh state, but the framework keeps the *live* one so runtime data is not
/// lost, and hands the fresh one to [`State::adopt_config_from`] so the live
/// state picks up the new props. `create_state` can therefore run on every
/// parent rebuild: keep it cheap and move the props into the state instead of
/// cloning them.
///
/// Put `#[derive(StatefulWidget)]` on the struct and implement [`State`] for
/// the type returned by `create_state`; that is all a stateful widget needs. The
/// derive writes the [`Widget`] impl for you, so there is no `to_element` to
/// implement by hand, prefer it over implementing `Widget` directly. A named
/// struct with a field `key: Option<Key>` has that key forwarded to the
/// element, which keeps the state attached to the same widget when siblings are
/// reordered. A widget with nothing to remember should be a
/// [`StatelessWidget`](crate::StatelessWidget).
///
/// # Examples
///
/// See [`State`] for a complete widget with a state that changes.
pub trait StatefulWidget: Sized {
    /// The state this widget creates, which owns every field that changes.
    type State: State<Self>;

    /// Returns this widget.
    ///
    /// Lets generic code that only knows `W: StatefulWidget` reach the widget
    /// through a method call.
    fn widget(&self) -> &Self
    where
        Self: Sized,
    {
        self
    }

    /// Consumes this widget and creates the state it configures.
    ///
    /// The widget is gone once this returns, its element keeps the state
    /// instead, so an implementation moves its props into the state rather than
    /// cloning them.
    fn create_state(self) -> Self::State;
}

/// The mutable half of a [`StatefulWidget`]: the data that changes, and the
/// UI built from it.
///
/// # Lifecycle
///
/// 1. [`StatefulWidget::create_state`] builds the state from the widget.
/// 2. [`init_state`](State::init_state) runs once, handing the state a
///    [`StateUpdater`].
/// 3. [`build`](State::build) describes the UI, and runs again after every
///    mutation applied through the updater.
/// 4. When a parent re-emits the widget, the live state is kept and
///    [`adopt_config_from`](State::adopt_config_from) copies the new props
///    into it.
///
/// # Changing state
///
/// `build` and the other hooks take `&self` or `&mut self` only while the
/// framework is calling them, so a callback cannot hold the state. It holds
/// the [`StateUpdater`] instead, store it in `init_state`, and describes the
/// change as a closure with [`StateUpdater::set_state`]. The closure is queued
/// and applied on the render thread at the next rebuild, and several calls in
/// one frame cause a single rebuild. The updater is `Copy` but belongs to the
/// UI thread: it is neither `Send` nor `Sync`.
///
/// Calling `set_state` on an updater that `init_state` never stored panics
/// with an error that points at the missing line.
///
/// # Examples
///
/// ```rust,ignore
/// use aimer::*;
///
/// #[derive(StatefulWidget)]
/// struct Counter {
///     start: i32,
/// }
///
/// struct CounterState {
///     updater: StateUpdater<Self>,
///     count: i32,
/// }
///
/// impl StatefulWidget for Counter {
///     type State = CounterState;
///
///     fn create_state(self) -> CounterState {
///         CounterState {
///             updater: StateUpdater::empty(),
///             count: self.start,
///         }
///     }
/// }
///
/// impl State<Counter> for CounterState {
///     fn init_state(&mut self, updater: StateUpdater<Self>) {
///         self.updater = updater;
///     }
///
///     fn build(&self, _ctx: &BuildContext) -> impl Widget {
///         // Hand this to a button's press handler: it rebuilds with `count + 1`.
///         let updater = self.updater;
///         let _increment = move || updater.set_state(|state| state.count += 1);
///         Text::new(format!("Count: {}", self.count))
///     }
/// }
/// ```
pub trait State<W: StatefulWidget> {
    /// Called once after the state is created, providing a [`StateUpdater`]
    /// handle. Store the updater in your state struct to later call
    /// `set_state()` from event handlers or callbacks, similar to
    /// Flutter's `setState`.
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized;

    /// Called during reconciliation when a parent rebuild produces a freshly
    /// built element for the *same* stateful widget (e.g. a window resize, or a
    /// parent `set_state` that re-emits this widget with new props).
    ///
    /// The framework preserves this (the *live*) state object, keeping runtime
    /// fields such as hover/focus/scroll/animation progress, but the freshly
    /// built `new` state carries the up-to-date widget *configuration* (the
    /// props passed down from the parent, e.g. a `TextButton`'s `style` /
    /// `hover_style` / `on_press`, or a selected/disabled flag). Move those
    /// configuration fields out of the freshly-built `new` state into
    /// `self` so the widget renders with the current configuration while
    /// retaining its runtime state.
    fn adopt_config_from(&mut self, _new: Self)
    where
        Self: Sized,
    {
    }

    /// Override this method to build the widget
    fn build(&self, ctx: &BuildContext) -> impl Widget;
}
pub type RebuildCallBack = dyn Fn(&BuildContext) -> AnyElement;

struct KeyedStateEntry {
    rebuild_fn: Weak<RebuildCallBack>,
    dirty: Weak<Cell<bool>>,
    dirty_source: Weak<DirtySource>,
    failed: Weak<Cell<bool>>,
    failure: Weak<FailureState>,
    state_revision: Weak<Cell<u64>>,
    state_any: Weak<dyn Any>,
    state_sender: Weak<dyn Any>,
    state_owner: Weak<super::state_slots::SlotOwner>,
    adopt_config_fn: Weak<AdoptConfigCallBack>,
}

struct LiveKeyedState {
    rebuild_fn: Rc<RebuildCallBack>,
    dirty: Rc<Cell<bool>>,
    dirty_source: Rc<DirtySource>,
    failed: Rc<Cell<bool>>,
    failure: Rc<FailureState>,
    state_revision: Rc<Cell<u64>>,
    state_any: Rc<dyn Any>,
    state_sender: Rc<dyn Any>,
    state_storage: StateStorage,
    adopt_config_fn: Rc<AdoptConfigCallBack>,
}

fn consumed_state_panic_message(debug_name: &'static str, key: Option<&crate::Key>) -> String {
    match key {
        Some(key) => format!(
            "Live state was consumed during reconciliation for widget `{debug_name}` with key {}. \
             Check where this key is created and used: keys for stateful widgets must be stable \
             and unique; do not reuse one key for multiple widgets.",
            key.diagnostic_description()
        ),
        None => format!(
            "Live state was consumed during reconciliation for widget `{debug_name}`. Check \
             where this widget's key is created and used: keys for stateful widgets must be stable \
             and unique; do not reuse one key for multiple widgets."
        ),
    }
}

thread_local! {
    static KEYED_STATE_REGISTRY: RefCell<HashMap<(crate::key::Key, &'static str), KeyedStateEntry>> =
        RefCell::new(HashMap::new());
    static KEYED_STATE_SCOPE_DEPTH: Cell<usize> = const { Cell::new(0) };
}

struct KeyedStateScope {
    owns_registry: bool,
}

impl KeyedStateScope {
    fn enter() -> Self {
        let owns_registry = KEYED_STATE_SCOPE_DEPTH.with(|depth| {
            let owns_registry = depth.get() == 0;
            if owns_registry {
                KEYED_STATE_REGISTRY.with(|registry| registry.borrow_mut().clear());
            }
            depth.set(depth.get() + 1);
            owns_registry
        });
        Self { owns_registry }
    }

    fn is_active() -> bool {
        KEYED_STATE_SCOPE_DEPTH.with(|depth| depth.get() > 0)
    }
}

impl Drop for KeyedStateScope {
    fn drop(&mut self) {
        KEYED_STATE_SCOPE_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

pub struct StatefulElement {
    child: SyncChild,
    /// Marked when this element (or its state's own `set_state`) requests a
    /// rebuild. Wrapped in `RefCell` so `adopt_state_from` can *repoint* it to
    /// the OLD element's flag during reconciliation, see `adopt_state_from`
    /// for why the live element must share the flag the preserved state's
    /// captured updater flips.
    pub dirty: RefCell<Rc<Cell<bool>>>,
    pub(crate) dirty_source: RefCell<Rc<DirtySource>>,
    rebuild_fn: SyncRebuildFn,
    /// Monotonically increasing generation counter. Incremented on each rebuild
    /// so that multiple `set_state` calls between frames only trigger one
    /// rebuild.
    rebuild_generation: Cell<u64>,
    /// The generation at which the last rebuild was performed.
    last_rebuilt_generation: Cell<u64>,
    /// Last invalidation generation whose descendant rebuild work was visited.
    /// Clean stateful elements can skip their retained subtree until a state,
    /// dependency, or explicit dirty mark advances the generation.
    rebuild_invalidation_generation: Cell<u64>,
    /// Shared revision incremented whenever a queued state mutation is applied.
    /// Used to disambiguate duplicate keyed copies retained by transitions.
    state_revision: RefCell<Rc<Cell<u64>>>,
    // #[cfg(debug_assertions)]
    debug_name: Cell<&'static str>,
    pub key: Option<crate::key::Key>,
    /// This element's own state cell, type-erased, so a reconciling element can
    /// hand it to the live element's `adopt_config_fn` for a config refresh.
    state_any: SyncStateAny,
    state_sender: SyncStateAny,
    /// Copies widget configuration from another element's state (passed as
    /// `&dyn Any`) into this element's live state via
    /// `State::adopt_config_from`.
    ///
    /// This element-side lease is the only strong owner of the updater slot
    /// during ordinary native operation; copied updaters do not keep it alive.
    state_storage: RefCell<StateStorage>,
    adopt_config_fn: SyncAdoptConfigFn,
    /// Set after a recovered reconciliation failure so later queued updates do
    /// not touch a state cell that reconciliation has already consumed.
    failed: Rc<Cell<bool>>,
    failure: Rc<FailureState>,
}

impl StatefulElement {
    #[inline]
    pub fn boxed(self) -> AnyElement {
        Element::boxed(self)
    }
}

impl StatefulElement {
    /// Converts a stateful widget into an element while containing lifecycle
    /// panics to this widget subtree in unwind-enabled builds.
    #[doc(hidden)]
    pub fn from_widget<W: StatefulWidget + 'static>(
        widget: W,
        ctx: &BuildContext,
        debug_name: &'static str,
        key: Option<crate::key::Key>,
    ) -> AnyElement
    where
        W::State: 'static,
    {
        match recover_operation(debug_name, BuildPhase::KeyedState, || {
            Self::try_new_with_identity(widget, ctx, debug_name, key)
        }) {
            Ok(Ok((element, _updater))) => element.boxed(),
            Ok(Err(diagnostic)) | Err(diagnostic) => diagnostic.into_error_element(),
        }
    }

    /// Create a new StatefulElement from a StatefulWidget.
    /// Returns the element and a StateUpdater that can be used in callbacks.
    pub fn new_with_name<W: StatefulWidget + 'static>(
        widget: W,
        ctx: &BuildContext,
        debug_name: &'static str,
        key: Option<crate::key::Key>,
    ) -> (Self, StateUpdater<W::State>)
    where
        W::State: 'static,
    {
        Self::try_new_with_identity(widget, ctx, debug_name, key)
            .unwrap_or_else(|diagnostic| panic!("{diagnostic}"))
    }

    pub fn new<W: StatefulWidget + 'static>(
        widget: W,
        ctx: &BuildContext,
    ) -> (Self, StateUpdater<W::State>)
    where
        W::State: 'static,
    {
        Self::try_new_with_identity(widget, ctx, "Unknown", None)
            .unwrap_or_else(|diagnostic| panic!("{diagnostic}"))
    }

    fn try_new_with_identity<W: StatefulWidget + 'static>(
        widget: W,
        ctx: &BuildContext,
        debug_name: &'static str,
        key: Option<crate::key::Key>,
    ) -> Result<(Self, StateUpdater<W::State>), PanicDiagnostic>
    where
        W::State: 'static,
    {
        let state = recover_operation(debug_name, BuildPhase::CreateState, || {
            widget.create_state()
        })?;

        if KeyedStateScope::is_active()
            && let Some(key_ref) = key.as_ref()
            && let Some(live) = lookup_keyed_state(key_ref, debug_name)
        {
            let fresh_state = Rc::new(SyncState(UnsafeCell::new(Some(state))));
            let fresh_state_any: Rc<dyn Any> = fresh_state;
            if let Err(diagnostic) = recover_operation(debug_name, BuildPhase::AdoptConfig, || {
                (live.adopt_config_fn)(fresh_state_any.as_ref())
            }) {
                live.failed.set(true);
                live.dirty_source.clear();
                let diagnostic = diagnostic.with_site(key_ref.diagnostic_site());
                live.failure.record(&diagnostic);
                return Err(diagnostic);
            }

            let child = (live.rebuild_fn)(ctx);
            return recover_operation(debug_name, BuildPhase::KeyedState, || {
                let element = StatefulElement {
                    child: SyncChild(UnsafeCell::new(child)),
                    dirty: RefCell::new(live.dirty),
                    dirty_source: RefCell::new(live.dirty_source),
                    rebuild_fn: SyncRebuildFn(UnsafeCell::new(live.rebuild_fn)),
                    rebuild_generation: Cell::new(0),
                    last_rebuilt_generation: Cell::new(0),
                    rebuild_invalidation_generation: Cell::new(u64::MAX),
                    state_revision: RefCell::new(live.state_revision),
                    debug_name: Cell::new(debug_name),
                    key,
                    state_any: SyncStateAny(UnsafeCell::new(live.state_any)),
                    state_sender: SyncStateAny(UnsafeCell::new(live.state_sender)),
                    state_storage: RefCell::new(live.state_storage),
                    adopt_config_fn: SyncAdoptConfigFn(UnsafeCell::new(live.adopt_config_fn)),
                    failed: live.failed,
                    failure: live.failure,
                };
                let updater = element
                    .state_updater()
                    .expect("keyed state registry contained a state of the wrong type");
                register_keyed_state(&element);
                (element, updater)
            });
        }

        let dirty = Rc::new(Cell::new(false));
        let build_consumer = BuildConsumer::new(dirty.clone());
        let dirty_source = build_consumer.dirty_source();

        let tx = Rc::new(StateMutationQueue::default());

        let state_cell = Rc::new(SyncState(UnsafeCell::new(Some(state))));
        let state_revision = Rc::new(Cell::new(0));
        let failure = Rc::new(FailureState::default());
        let failed = Rc::new(Cell::new(false));

        // Create the updater and pass it into init_state.
        let (state_storage, init_updater) = StateUpdater::with(
            tx.clone(),
            state_cell.clone(),
            dirty_source.clone(),
            failure.clone(),
            ctx.window.key(),
        );

        {
            // Safety: single-threaded, we are the only accessor during construction.
            let s = unsafe {
                (&mut *state_cell.0.get())
                    .as_mut()
                    .expect("State was consumed before initialization")
            };
            recover_operation(debug_name, BuildPhase::InitState, || {
                s.init_state(init_updater)
            })?;
        }

        let state_for_build = state_cell.clone();
        let revision_for_rebuild = state_revision.clone();
        let mutations_for_rebuild = tx.clone();
        let consumer_for_rebuild = build_consumer.clone();
        let failure_for_rebuild = failure.clone();
        let failed_for_rebuild = failed.clone();
        let dirty_source_for_rebuild = dirty_source.clone();
        let rebuild_fn: Rc<RebuildCallBack> = Rc::new(move |ctx| {
            if let Some(error) = failure_for_rebuild.error_element() {
                return error;
            }
            // Drain all pending mutations before rebuilding.
            let mutation_result =
                recover_operation(debug_name, BuildPhase::ApplyStateMutation, || {
                    let s = unsafe {
                        (&mut *state_for_build.0.get())
                            .as_mut()
                            .expect("State has already been consumed during reconciliation")
                    };
                    mutations_for_rebuild.drain_into(s, || {
                        revision_for_rebuild.fetch_add(1);
                    });
                });
            if let Err(diagnostic) = mutation_result {
                return recover_failure(
                    &failure_for_rebuild,
                    &failed_for_rebuild,
                    &dirty_source_for_rebuild,
                    diagnostic,
                );
            }
            ctx.with_build_consumer(consumer_for_rebuild.clone(), |ctx| {
                let s = unsafe {
                    (&*state_for_build.0.get())
                        .as_ref()
                        .expect("State has already been consumed during reconciliation")
                };
                let child_widget =
                    match recover_operation(debug_name, BuildPhase::Build, || s.build(ctx)) {
                        Ok(widget) => widget,
                        Err(diagnostic) => {
                            return recover_failure(
                                &failure_for_rebuild,
                                &failed_for_rebuild,
                                &dirty_source_for_rebuild,
                                diagnostic,
                            )
                        }
                    };
                match recover_operation(debug_name, BuildPhase::ToElement, || {
                    Widget::to_element(child_widget, ctx)
                })
                {
                    Ok(element) => element,
                    Err(diagnostic) => recover_failure(
                        &failure_for_rebuild,
                        &failed_for_rebuild,
                        &dirty_source_for_rebuild,
                        diagnostic,
                    ),
                }
            })
        });

        let child = {
            // Safety: single-threaded, initial build during construction.
            let s = unsafe {
                (&*state_cell.0.get())
                    .as_ref()
                    .expect("State was consumed before the initial build")
            };
            ctx.with_build_consumer(build_consumer, |ctx| {
                let child_widget =
                    recover_operation(debug_name, BuildPhase::Build, || s.build(ctx))?;
                recover_operation(debug_name, BuildPhase::ToElement, || {
                    Widget::to_element(child_widget, ctx)
                })
            })?
        };

        // Type-erased handle to this element's state, plus a closure that can
        // pull configuration out of *another* element's state (of the same
        // `W::State` type) into this one. Together these let reconciliation
        // refresh a preserved live state's widget props without
        // `StatefulElement` being generic over `W`.
        let state_any: Rc<dyn Any> = state_cell.clone();
        let state_sender: Rc<dyn Any> = Rc::new(init_updater);
        let state_for_config = state_cell.clone();
        let key_for_config = key.clone();
        let adopt_config_fn: Rc<AdoptConfigCallBack> = Rc::new(move |new_any: &dyn Any| {
            if let Some(new_cell) = new_any.downcast_ref::<SyncState<W::State>>() {
                // Safety: single-threaded reconciliation; the live state is not
                // otherwise borrowed while we move the fresh config into it.
                let old_state = unsafe {
                    (&mut *state_for_config.0.get())
                        .as_mut()
                        .unwrap_or_else(|| {
                            panic!(
                                "{}",
                                consumed_state_panic_message(debug_name, key_for_config.as_ref())
                            )
                        })
                };
                let new_state = unsafe { (&mut *new_cell.0.get()).take() };
                if let Some(new_state) = new_state {
                    old_state.adopt_config_from(new_state);
                }
            }
        });
        let updater = init_updater;

        let element = StatefulElement {
            child: SyncChild(UnsafeCell::new(child)),
            dirty: RefCell::new(dirty),
            dirty_source: RefCell::new(dirty_source),
            rebuild_fn: SyncRebuildFn(UnsafeCell::new(rebuild_fn)),
            rebuild_generation: Cell::new(0),
            last_rebuilt_generation: Cell::new(0),
            rebuild_invalidation_generation: Cell::new(u64::MAX),
            state_revision: RefCell::new(state_revision),
            debug_name: Cell::new(debug_name),
            key,
            state_any: SyncStateAny(UnsafeCell::new(state_any)),
            state_sender: SyncStateAny(UnsafeCell::new(state_sender)),
            state_storage: RefCell::new(state_storage),
            adopt_config_fn: SyncAdoptConfigFn(UnsafeCell::new(adopt_config_fn)),
            failed,
            failure,
        };

        let element = if KeyedStateScope::is_active() {
            recover_operation(debug_name, BuildPhase::KeyedState, || {
                register_keyed_state(&element);
                element
            })?
        } else {
            element
        };

        Ok((element, updater))
    }

    /// Check if this element needs a rebuild and perform it if so.
    ///
    /// Uses element reconciliation: before replacing the child, tries to update
    /// it in-place via `try_update_element`. If the child's type and key
    /// match the new element's, the child is updated without replacement,
    /// preserving nested `StatefulElement` state, GPU resources, and
    /// reducing allocations.
    ///
    /// Before rebuilding itself, this method first walks the existing child
    /// tree to let any nested `StatefulElement`s rebuild independently.
    /// This avoids destroying and recreating the entire subtree when only a
    /// deeply-nested element's state has changed.
    pub fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        #[cfg(feature = "frame-stats")]
        crate::rebuild_stats::record_stateful_check();
        let invalidation_generation =
            crate::components::element::rebuild_invalidation_generation();
        let dirty = self.dirty.borrow().get();
        if !dirty
            && self.rebuild_invalidation_generation.get() == invalidation_generation
        {
            return;
        }
        self.rebuild_invalidation_generation
            .set(invalidation_generation);

        if self.failed.get() {
            self.dirty_source.borrow().clear();
            if let Some(error) = self.failure.error_element() {
                unsafe {
                    *self.child.0.get() = error;
                }
            }
            return;
        }
        if !self.dirty.borrow().get() {
            // A clean ancestor does not participate in reconciliation, so avoid
            // scanning its entire subtree solely to populate the keyed-state
            // registry. Any dirty descendant enters its own scope when reached.
            let child = unsafe { &*self.child.0.get() };
            Self::propagate_rebuild(child.as_ref(), ctx);
            return;
        }

        let keyed_state_scope = KeyedStateScope::enter();
        if keyed_state_scope.owns_registry {
            register_keyed_state(self);
            let existing_child = unsafe { &*self.child.0.get() };
            register_keyed_subtree(existing_child.as_ref());
        }

        // Coalesce: only rebuild once per generation bump.
        let current_gen = self.rebuild_generation.get();
        let last = self.last_rebuilt_generation.get();
        if current_gen == last && !self.dirty.borrow().get() {
            return;
        }

        // Build the new child element FIRST. Running our own `build` before
        // propagating the rebuild downward ensures any inherited state this
        // element provides via `ctx.insert_state` (e.g. a `Navigator` inserting
        // its `NavigatorController`) is re-published into the *current* frame's
        // context before descendants rebuild and look it up.
        //
        // Otherwise a nested consumer rebuilt during `propagate_rebuild`, such
        // as a header calling `NavigatorController::of` on a window resize,
        // where `mark_needs_rebuild` dirties the whole tree and the frame's
        // `BuildContext` starts with an empty `inherited_states` map, would
        // look up state the provider has not re-inserted yet this frame and
        // panic ("No Navigator found in context").
        let new_child = {
            #[cfg(feature = "frame-stats")]
            crate::rebuild_stats::record_stateful_build();
            let rf = unsafe { &*self.rebuild_fn.0.get() };
            rf(ctx)
        };

        // Then let nested dirty StatefulElements in the existing subtree rebuild
        // in-place, now that the parent-provided context is populated.
        {
            let child = unsafe { &*self.child.0.get() };
            Self::propagate_rebuild(child.as_ref(), ctx);
        }

        // Carry live state from nested StatefulElements in the old tree into the
        // freshly-built new tree before replacing. This preserves runtime state
        // (e.g. selected tab index, scroll position) across the rebuild.
        let child_reconciliation = recover_operation(
            self.debug_name.get(),
            BuildPhase::ReconcileChildren,
            || {
                let old_child = unsafe { &*self.child.0.get() };
                crate::components::element::reconcile_carried_tree(
                    old_child.as_ref(),
                    new_child.as_ref(),
                    ctx,
                );
            },
        );
        if let Err(diagnostic) = child_reconciliation {
            let error = {
                let dirty_source = self.dirty_source.borrow();
                recover_failure(
                    &self.failure,
                    &self.failed,
                    &dirty_source,
                    diagnostic,
                )
            };
            unsafe {
                *self.child.0.get() = error;
            }
            return;
        }

        // Install the newly-built child, replacing the old subtree.
        // Safety: single-threaded rendering pipeline; old_child is not used past this
        // point.
        unsafe {
            *self.child.0.get() = new_child;
        }

        self.dirty_source.borrow().clear();
        self.rebuild_generation.fetch_add(1);
        self.last_rebuilt_generation
            .set(self.rebuild_generation.get());
    }

    /// Walk the element tree and rebuild any nested dirty `StatefulElement`s.
    /// This is called on the *existing* child tree so that inner stateful
    /// widgets can update in-place without the parent having to reconstruct
    /// the whole subtree.
    fn propagate_rebuild(element: &dyn Element, ctx: &BuildContext) {
        element.rebuild_if_dirty(ctx);
    }
}

/// If both elements are `StatefulElement`s with the same `debug_name`, adopt
/// the live state from `old` into `new`. This is the rescue path that runs
/// even when the element tree shape changed, it ensures nested stateful
/// widgets (e.g. tab buttons, form inputs) keep their runtime state across
/// a parent rebuild.
///
/// Safe to call on any pair: when both sides aren't matching
/// `StatefulElement`s, it's a no-op.
pub(crate) fn carry_stateful(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    if !old.is_stateful_element() && !new.is_stateful_element() {
        return;
    }

    let Some(old_ele) = old
        .option_any()
        .and_then(|o| o.downcast_ref::<StatefulElement>())
    else {
        return;
    };

    let Some(new_ele) = new
        .option_any()
        .and_then(|o| o.downcast_ref::<StatefulElement>())
    else {
        return;
    };
    if old_ele.debug_name.get() != new_ele.debug_name.get() || old_ele.key != new_ele.key {
        return;
    }
    new_ele.adopt_state_from(old_ele, ctx);
}

struct KeyedStateIndex<'a> {
    states: HashMap<(&'a crate::key::Key, &'static str), &'a StatefulElement>,
}

impl<'a> KeyedStateIndex<'a> {
    fn from_subtree(root: &'a dyn Element) -> Self {
        let mut states = HashMap::new();
        let mut pending = vec![root];

        while let Some(current) = pending.pop() {
            if let Some(stateful) = current
                .option_any()
                .and_then(|value| value.downcast_ref::<StatefulElement>())
                && let Some(key) = stateful.key.as_ref()
            {
                let identity = (key, stateful.debug_name.get());
                let should_replace = states
                    .get(&identity)
                    .is_none_or(|existing: &&StatefulElement| {
                        existing.state_revision.borrow().get()
                            < stateful.state_revision.borrow().get()
                    });
                if should_replace {
                    states.insert(identity, stateful);
                }
            }

            pending.extend(element_children(current).into_iter().rev());
        }

        Self { states }
    }

    #[inline]
    fn get(
        &self,
        key: &crate::key::Key,
        debug_name: &'static str,
    ) -> Option<&'a StatefulElement> {
        self.states.get(&(key, debug_name)).copied()
    }
}

/// Reports whether two element references name the same element.
///
/// Compares the data addresses only: a retained child is reached through two
/// different proxies, so the vtable halves of the two fat pointers are the ones
/// that may differ while the element behind them is one and the same.
#[inline]
fn same_element(left: &dyn Element, right: &dyn Element) -> bool {
    std::ptr::eq(
        left as *const dyn Element as *const (),
        right as *const dyn Element as *const (),
    ) || crate::components::element::same_retained_child(left, right)
}

fn element_children(element: &dyn Element) -> smallvec::SmallVec<[&dyn Element; 8]> {
    structural_children(element)
}

/// Recurse into the matched children of an old and new element tree, letting
/// each nested `StatefulElement` carry its runtime state from the old subtree
/// into the new one.
///
/// Children are enumerated through the canonical structural-child accessor so
/// elements such as scrollable containers can expose their layout children
/// even when those children are hidden from event dispatch.
pub(crate) fn carry_child_state(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    if same_element(old, new) {
        return;
    }
    carry_keyed_child_state(old, new, ctx);
    carry_unkeyed_child_state(old, new, ctx);
}

fn carry_keyed_child_state(old_root: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    new.with_rebuild_context(ctx, &mut |ctx| {
        let mut keyed_states = None;
        carry_keyed_child_state_in_context(old_root, new, ctx, &mut keyed_states)
    });
}

fn carry_keyed_child_state_in_context<'a>(
    old_root: &'a dyn Element,
    new: &dyn Element,
    ctx: &BuildContext,
    keyed_states: &mut Option<KeyedStateIndex<'a>>,
) {
    if new.is_stateful_element()
        && let Some(new_stateful) = new
            .option_any()
            .and_then(|value| value.downcast_ref::<StatefulElement>())
        && let Some(key) = new_stateful.key.as_ref()
    {
        let old_stateful = keyed_states
            .get_or_insert_with(|| KeyedStateIndex::from_subtree(old_root))
            .get(key, new_stateful.debug_name.get());
        if let Some(old_stateful) = old_stateful
            && old_stateful.state_revision.borrow().get()
                >= new_stateful.state_revision.borrow().get()
        {
            new_stateful.adopt_state_from(old_stateful, ctx);
            carry_state_below_keyed(old_stateful, new_stateful, ctx);
        }
        return;
    }

    for child in element_children(new) {
        // A descendant may publish inherited state (for example, a provider).
        // Keep entering each element's rebuild scope while sharing the keyed
        // index, otherwise a keyed state rebuild can see the bare frame context.
        child.with_rebuild_context(ctx, &mut |ctx| {
            carry_keyed_child_state_in_context(old_root, child, ctx, keyed_states);
        });
    }
}

/// Hands the subtree below a keyed stateful element over to the element that
/// replaced it.
///
/// A keyed element keeps its own state wherever it reappears, it is looked up
/// by key, not by position, but the subtree it rebuilds from that state is
/// constructed anew, and everything below it is ordinary unkeyed state: a
/// request an `AsyncBuilder` already completed, a scroll offset, an animation
/// halfway through. Without this hand-over all of it starts over whenever an
/// ancestor rebuilds, which for a route transition wrapping a page means every
/// tick of an unrelated animation above it.
///
/// Unlike the walk below an unkeyed element, this one cannot assume the two
/// subtrees line up. A keyed element deliberately outlives changes to what it
/// contains, one route transition keeps its identity while the page inside it
/// is replaced, so a pair is only descended into once both sides agree on what
/// they are. Everything else keeps the state it was built with, which is how a
/// navigation still gets its new page.
///
/// Keys nested below this one are resolved first, and by key rather than by
/// position: the walk that follows deliberately steps over keyed elements, and
/// the pass that would otherwise claim them stopped at this element. A section
/// that names itself inside a page inside a keyed route transition, the shape
/// `website/src/screen/home_screen.rs` builds, would otherwise be the one
/// thing an ancestor's rebuild resets.
fn carry_state_below_keyed(old: &StatefulElement, new: &StatefulElement, ctx: &BuildContext) {
    if std::ptr::eq(old, new) {
        return;
    }

    let mut keyed_states = None;
    for child in element_children(new) {
        child.with_rebuild_context(ctx, &mut |ctx| {
            carry_keyed_child_state_in_context(old, child, ctx, &mut keyed_states);
        });
    }

    carry_matching_child_state(old, new, ctx);
}

/// Pairs the children of `old` and `new` by position and hands over the state of
/// every pair that describes the same widget.
fn carry_matching_child_state(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    let old_children = element_children(old);
    if old_children.is_empty() {
        return;
    }

    let new_children = element_children(new);
    for (old_child, new_child) in old_children.iter().zip(new_children.iter()) {
        if !identities_are_compatible(*old_child, *new_child) {
            continue;
        }

        carry_matching_state(*old_child, *new_child, ctx);
    }
}

/// Hands one compatible pair over, then continues into their children.
fn carry_matching_state(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    // A retained child appears in both trees, so a pair can be one element
    // twice over; it has nothing to hand to itself.
    if same_element(old, new) {
        return;
    }
    new.with_rebuild_context(ctx, &mut |ctx| {
        adopt_runtime_state(old, new);

        if old.is_stateful_element()
            && new.is_stateful_element()
            && let Some(new_stateful) = new
                .option_any()
                .and_then(|value| value.downcast_ref::<StatefulElement>())
        {
            // A keyed element is the keyed pass's business, not this walk's.
            if new_stateful.key.is_some() {
                return;
            }

            carry_stateful(old, new, ctx);
        }

        carry_matching_child_state(old, new, ctx);
    });
}

/// Lets `new` take over the runtime state `old` was holding beside its children.
///
/// The positional walk below reaches every child an element owns, which covers
/// almost everything. What it cannot reach is a container that materializes its
/// children on demand, a freshly built one has none, so there is no pair to walk
///, or state a container keeps beside them, such as a measurement of a list too
/// long to measure again. Both are handed over here instead; see
/// [`Rebuildable::adopt_runtime_state_from`].
///
/// Restricted to elements of the same concrete type, so an element is never
/// offered state it cannot interpret.
fn adopt_runtime_state(old: &dyn Element, new: &dyn Element) {
    if old.element_type_id() != new.element_type_id() || old.debug_name() != new.debug_name() {
        return;
    }

    new.adopt_runtime_state_from(old);
}

fn carry_unkeyed_child_state(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    // A retained child is placed into the old and the new tree alike, so the
    // pairing walk can arrive with the same element on both sides. Nothing can
    // be carried from an element into itself, and descending would carry
    // everything below it twice.
    if same_element(old, new) || crate::components::element::different_retained_children(old, new)
    {
        return;
    }
    new.with_rebuild_context(ctx, &mut |ctx| {
        carry_unkeyed_child_state_in_context(old, new, ctx)
    });
}

fn carry_unkeyed_child_state_in_context(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    adopt_runtime_state(old, new);

    if old.is_stateful_element() && new.is_stateful_element() {
        if new
            .option_any()
            .and_then(|value| value.downcast_ref::<StatefulElement>())
            .is_some_and(|stateful| stateful.key.is_some())
        {
            return;
        }

        carry_stateful(old, new, ctx);
    }

    let old_children = element_children(old);
    if old_children.is_empty() {
        return;
    }

    let new_children = element_children(new);

    for (old_child, new_child) in old_children.iter().zip(new_children.iter()) {
        carry_unkeyed_child_state(*old_child, *new_child, ctx);
    }
}

impl StatefulElement {
    fn state_updater<S: 'static>(&self) -> Option<StateUpdater<S>> {
        let sender_any = unsafe { (&*self.state_sender.0.get()).clone() };
        let sender = sender_any.downcast::<StateUpdater<S>>().ok()?;
        Some(*sender)
    }

    /// Returns true if this element is marked dirty.
    pub fn is_dirty(&self) -> bool {
        self.dirty.borrow().get()
    }

    /// Adopt the live state from another `StatefulElement` of the same widget
    /// type.
    ///
    /// Transfers the `rebuild_fn` (which captures the state cell and mutation
    /// channel), inherits the `debug_name`, and marks this element dirty so
    /// `rebuild_if_dirty` re-generates the child tree from the preserved state
    /// on the next frame.
    ///
    /// Called by `update_from_widget` when a parent's reconciliation replaces
    /// an entire subtree, without this, a freshly-constructed
    /// `StatefulElement` (with `current_index: 0`) would shadow the live
    /// one (with `current_index: 2`).
    pub(crate) fn adopt_state_from(&self, old: &StatefulElement, ctx: &BuildContext) {
        // A retained child reaches the walk from both trees, so the element it
        // is asked to adopt from can be itself. Its state never left, and the
        // borrows below would collide with themselves.
        if std::ptr::eq(self, old) {
            return;
        }
        if self.failed.get()
            || old.failed.get()
            || self.failure.is_failed()
            || old.failure.is_failed()
        {
            if let Some(message) = self.failure.message().or_else(|| old.failure.message()) {
                self.failure.record_message(message.clone());
                old.failure.record_message(message);
                self.failed.set(true);
                self.dirty_source.borrow().clear();
                if let Some(error) = self.failure.error_element() {
                    unsafe {
                        *self.child.0.get() = error;
                    }
                }
            }
            return;
        }
        // Keyed construction can already reuse the live element's state cell
        // before the replacement enters the general tree walk. In that case
        // the fresh element has already adopted its configuration, and taking
        // the shared cell here would alias the live mutable state.
        let self_state = unsafe { &*self.state_any.0.get() };
        let old_state = unsafe { &*old.state_any.0.get() };
        if Rc::ptr_eq(self_state, old_state) {
            return;
        }
        // Safety: called only from `update_from_widget` during single-threaded
        // reconciliation, before the new element is visible to any other code.
        unsafe {
            // The rebuild closure captures the state cell and mutation channel.
            // Replacing it makes this element's build() read from the live state.
            // println!("adopt_state_from casting raw ptr");
            *self.rebuild_fn.0.get() = (*old.rebuild_fn.0.get()).clone();
        }
        // Inherit the name so future reconciliation still matches.
        self.debug_name.set(old.debug_name.get());

        // Adopt the OLD element's dirty flag so the *live* element
        *self.dirty.borrow_mut() = old.dirty.borrow().clone();
        *self.dirty_source.borrow_mut() = old.dirty_source.borrow().clone();
        *self.state_revision.borrow_mut() = old.state_revision.borrow().clone();

        // Refresh the *configuration* stored in the preserved live state from
        // the freshly-built element. We keep `old`'s state cell (its runtime
        // state, hover, scroll offset, selected tab, animation progress, …),
        // but that same cell also holds whatever props the widget copied from
        // its parent at `create_state` time (e.g. a `TextButton`'s `style` /
        // `hover_style` / `on_press`, a selected/disabled flag). Without this
        // refresh a widget re-emitted with different props after a parent
        // rebuild (a window resize, a parent `set_state`) would keep rendering
        // its *stale* props, the classic symptom being a tab whose highlight
        // stays stuck on the initially-selected button even though the live
        // selection moved on. `self` is the fresh element and carries the
        // up-to-date config in its state; hand it to `old`'s config hook.
        //
        // NOTE: this MUST run before we repoint `self.state_any` below, because
        // it uses `self`'s own (freshly-built) state as the *source* of the new
        // config.
        {
            // Safety: single-threaded reconciliation.
            let fresh_state: &dyn Any = unsafe { &*self.state_any.0.get() }.as_ref();
            let old_adopt = unsafe { &*old.adopt_config_fn.0.get() };
            if let Err(diagnostic) = recover_operation(
                self.debug_name.get(),
                BuildPhase::AdoptConfig,
                || old_adopt(fresh_state),
            ) {
                let diagnostic = match self.key.as_ref().or(old.key.as_ref()) {
                    Some(key) => diagnostic.with_site(key.diagnostic_site()),
                    None => diagnostic,
                };
                self.failure.record(&diagnostic);
                old.failure.record(&diagnostic);
                unsafe {
                    *self.child.0.get() = diagnostic.into_error_element();
                }
                old.failed.set(true);
                self.failed.set(true);
                self.dirty_source.borrow().clear();
                let cur_gen = self.rebuild_generation.get();
                self.last_rebuilt_generation.set(cur_gen);
                return;
            }
        }

        // Keep the live slot owner before dropping this element's candidate
        // storage. The candidate updater is only a temporary configuration
        // value; the preserved state and updater must continue to resolve the
        // old live slot after adoption.
        let live_storage = old.state_storage.borrow().clone_for_reconciliation();

        // Repoint this element's config-refresh machinery at the OLD state cell,
        // matching the `rebuild_fn` we just adopted. `rebuild_fn` now reads
        // `old`'s cell, but `self` was constructed with `state_any` /
        // `adopt_config_fn` bound to its OWN (now-orphaned) fresh cell. If we
        // left them pointing there, a *subsequent* reconcile that uses `self` as
        // the `old` side, which a single window resize does trigger (the eager
        // rebuild below reconciles this subtree, and the follow-up
        // `carry_child_state` pass reconciles it again), would refresh the
        // ORPHANED cell while the live `rebuild_fn` keeps reading `old`'s cell,
        // so the freshly-built config would never reach what actually renders
        // and the selected/highlight styling would freeze on a stale value.
        // Safety: single-threaded reconciliation; not otherwise borrowed here.
        unsafe {
            *self.state_any.0.get() = (*old.state_any.0.get()).clone();
            *self.state_sender.0.get() = (*old.state_sender.0.get()).clone();
            *self.adopt_config_fn.0.get() = (*old.adopt_config_fn.0.get()).clone();
        }
        *self.state_storage.borrow_mut() = live_storage;

        // Materialize the adopted state *immediately*, during reconciliation,
        // do not defer to the next `draw`.
        //
        // The child we were constructed with was built from this widget's
        // *initial* state (e.g. `current_index: 0`). Merely flagging `dirty` and
        // waiting for `draw` → `rebuild_if_dirty` to regenerate it is not
        // enough: on a window resize the rebuilt element is frequently *culled*
        // by a scroll viewport (its `draw`, and hence `rebuild_if_dirty`, never
        // runs) or sits behind a wrapper whose rebuild cascade, which walks
        // `visit_children`, never reaches it (containers such as `Container`
        // and `Row`/`Column` expose their children only through
        // `event_children`). In those cases the adopted `rebuild_fn` would never
        // execute and the user's state would silently snap back to the initial
        // value. Regenerating the child here, against the current
        // `BuildContext`, guarantees the live state is reflected regardless of
        // whether this element is ever drawn.
        let new_child = {
            let rf = unsafe { &*self.rebuild_fn.0.get() };
            // The child placed here is new content, so no placement may skip it;
            // see `without_placement_skip`.
            crate::components::element::without_placement_skip(|| rf(ctx))
        };
        // Carry live state from nested StatefulElements into the new tree.
        let child_reconciliation = recover_operation(
            self.debug_name.get(),
            BuildPhase::ReconcileChildren,
            || {
                let old_child = unsafe { &*self.child.0.get() };
                crate::components::element::reconcile_carried_tree(
                    old_child.as_ref(),
                    new_child.as_ref(),
                    ctx,
                );
            },
        );
        if let Err(diagnostic) = child_reconciliation {
            let diagnostic = match self.key.as_ref().or(old.key.as_ref()) {
                Some(key) => diagnostic.with_site(key.diagnostic_site()),
                None => diagnostic,
            };
            // The old child and old state owner remain installed. The candidate
            // subtree is discarded below, while the diagnostic makes the
            // failed handoff visible and prevents either element from being
            // treated as a healthy future source of state.
            self.failure.record(&diagnostic);
            old.failure.record(&diagnostic);
            self.failed.set(true);
            old.failed.set(true);
            self.dirty_source.borrow().clear();
            old.dirty_source.borrow().clear();
            unsafe {
                *self.child.0.get() = diagnostic.into_error_element();
            }
            let cur_gen = self.rebuild_generation.get();
            self.last_rebuilt_generation.set(cur_gen);
            return;
        }
        // Install the newly-built child, replacing the old subtree.
        // Safety: single-threaded reconciliation; old child is not used past this
        // point.
        unsafe {
            *self.child.0.get() = new_child;
        }

        // The adopted state has already been materialized with the current
        // context. Leaving it dirty would rebuild the replacement again while
        // its ancestors are still reconciling, producing fresh nested state
        // that can overwrite the live subtree.
        self.dirty_source.borrow().clear();
        let cur_gen = self.rebuild_generation.get();
        self.last_rebuilt_generation.set(cur_gen);
    }
}

fn lookup_keyed_state(key: &crate::key::Key, debug_name: &'static str) -> Option<LiveKeyedState> {
    KEYED_STATE_REGISTRY.with(|registry| {
        let registry = registry.borrow();
        let entry = registry.get(&(key.clone(), debug_name))?;
        let failed = entry.failed.upgrade()?;
        if failed.get() {
            return None;
        }
        let failure = entry.failure.upgrade()?;
        if failure.is_failed() {
            return None;
        }
        Some(LiveKeyedState {
            rebuild_fn: entry.rebuild_fn.upgrade()?,
            dirty: entry.dirty.upgrade()?,
            dirty_source: entry.dirty_source.upgrade()?,
            failed,
            failure,
            state_revision: entry.state_revision.upgrade()?,
            state_any: entry.state_any.upgrade()?,
            state_sender: entry.state_sender.upgrade()?,
            state_storage: StateStorage::from_owner(entry.state_owner.upgrade()?),
            adopt_config_fn: entry.adopt_config_fn.upgrade()?,
        })
    })
}

fn register_keyed_state(element: &StatefulElement) {
    let Some(key) = element.key.clone() else {
        return;
    };
    let rebuild_fn = unsafe { (&*element.rebuild_fn.0.get()).clone() };
    let state_any = unsafe { (&*element.state_any.0.get()).clone() };
    let state_sender = unsafe { (&*element.state_sender.0.get()).clone() };
    let state_owner = element.state_storage.borrow().downgrade();
    let adopt_config_fn = unsafe { (&*element.adopt_config_fn.0.get()).clone() };
    let failed = element.failed.clone();
    let failure = element.failure.clone();
    KEYED_STATE_REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let existing_failed = registry
            .get(&(key.clone(), element.debug_name.get()))
            .and_then(|entry| entry.failed.upgrade())
            .is_some_and(|failed| failed.get());
        if !existing_failed
            && registry
            .get(&(key.clone(), element.debug_name.get()))
            .and_then(|entry| entry.state_revision.upgrade())
            .is_some_and(|revision| revision.get() >= element.state_revision.borrow().get())
        {
            return;
        }
        registry.insert(
            (key, element.debug_name.get()),
            KeyedStateEntry {
                rebuild_fn: Rc::downgrade(&rebuild_fn),
                dirty: Rc::downgrade(&element.dirty.borrow()),
                dirty_source: Rc::downgrade(&element.dirty_source.borrow()),
                failed: Rc::downgrade(&failed),
                failure: Rc::downgrade(&failure),
                state_revision: Rc::downgrade(&element.state_revision.borrow()),
                state_any: Rc::downgrade(&state_any),
                state_sender: Rc::downgrade(&state_sender),
                state_owner,
                adopt_config_fn: Rc::downgrade(&adopt_config_fn),
            },
        );
    });
}

fn register_keyed_subtree(element: &dyn Element) {
    let mut pending = vec![element];

    while let Some(current) = pending.pop() {
        if current.is_stateful_element()
            && let Some(stateful) = current
                .option_any()
                .and_then(|value| value.downcast_ref::<StatefulElement>())
        {
            register_keyed_state(stateful);
        }

        pending.extend(element_children(current).into_iter().rev());
    }
}

impl Drawable for StatefulElement {
    fn update(&self, ctx: &BuildContext) {
        self.rebuild_if_dirty(ctx);
        // Safety: single-threaded rendering pipeline
        let child = unsafe { &*self.child.0.get() };

        // The retained tree draws the generated child through its own render
        // nodes; this element only keeps the state owner's subtree current.
        child.sync_paint_geometry(ctx);
        child.update(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        // This wrapper owns state and rebuilds its child, but paints nothing.
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        canvas.finish();
    }

    #[inline]
    fn retained_v2_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        // A state owner is visually transparent. Preserve a child's
        // viewport-sized render bounds instead of expanding the wrapper to
        // the child's scrollable content extent.
        unsafe { &*self.child.0.get() }.retained_v2_bounds(ctx)
    }

    #[inline]
    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        unsafe { &*self.child.0.get() }.retained_v2_paint_outsets(ctx)
    }

    #[inline]
    fn retained_v2_child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn aimer_widget::Element,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        unsafe { &*self.child.0.get() }.retained_v2_child_geometry(ctx, child)
    }

    #[inline]
    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn aimer_widget::Element,
        child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        unsafe { &*self.child.0.get() }
            .retained_v2_child_geometry_at(ctx, child, child_index)
    }

    #[inline]
    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &aimer_widget::base::BuildContext<'a>,
        child: &dyn aimer_widget::Element,
        child_index: usize,
    ) -> Option<aimer_widget::base::BuildContext<'a>> {
        unsafe { &*self.child.0.get() }
            .retained_v2_child_context_at(ctx, child, child_index)
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let child = unsafe { &*self.child.0.get() };
        child.sync_paint_geometry(ctx);
    }

}

impl VisitorElement for StatefulElement {
    fn visit_children<'a>(&self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // Safety: single-threaded rendering pipeline
        let child = unsafe { &*self.child.0.get() };
        visitor(child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        self.debug_name.get()
    }

    fn element_type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<StatefulElement>()
    }

    fn reconciliation_key(&self) -> Option<&crate::Key> {
        self.key.as_ref()
    }
}

impl EventElement for StatefulElement {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }

    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored()
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // The generated child is the ordinary event child. Stateful elements
        // are retained wrappers, not forwarding boundaries; routing it again
        // from `on_event_with_context` would deliver every pointer event twice.
        let child = unsafe { &*self.child.0.get() };
        visitor(child.as_ref());
    }
}

impl LayoutElement for StatefulElement {
    fn pos(&self) -> Option<Vec2d> {
        unsafe { &*self.child.0.get() }.pos()
    }

    fn size(&self) -> Option<Size> {
        unsafe { &*self.child.0.get() }.size()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        unsafe { &*self.child.0.get() }.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        unsafe { &*self.child.0.get() }.content_size(ctx)
    }
    fn flex(&self) -> Option<f32> {
        unsafe { &*self.child.0.get() }.flex()
    }
    fn get_size_from_child(&self) -> Option<Size> {
        unsafe { &*self.child.0.get() }.get_size_from_child()
    }
    fn invalidate_layout(&self) {
        unsafe { &*self.child.0.get() }.invalidate_layout();
    }
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        unsafe { &*self.child.0.get() }.pos_start_end()
    }
}

impl Rebuildable for StatefulElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        StatefulElement::rebuild_if_dirty(self, ctx);
    }

    fn option_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn is_stateful_element(&self) -> bool {
        true
    }

    fn mark_needs_rebuild(&self) {
        let _mark = crate::components::element::begin_rebuild_mark();
        crate::components::element::with_rebuild_invalidation(|| {
            // Safety: single-threaded rendering pipeline.
            let child = unsafe { &*self.child.0.get() };
            let carries_child_state = child.is_carry_state();
            if !carries_child_state {
                let source = self.dirty_source.borrow();
                // A placement only needs to reach builds that observed inherited
                // context; this one read none, so it cannot differ from last time.
                if !(crate::components::element::placement_mark_active()
                    && !source.reads_context())
                {
                    source.mark();
                }
            }
            child.mark_needs_rebuild();
            if carries_child_state {
                // The child owns the rebuildable state, but custom carrying
                // elements do not publish a path for their own dirty work.
                // Explicit recursive marks therefore use the safe fallback.
                crate::components::element::invalidate_dirty_paths();
            }
        });
    }
}

#[cfg(test)]
mod rebuild_scope_tests;

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;

    use aimer_events::pointer::{PointerButton, PointerInfo};

    use super::*;
    use crate::{EventDispatcher, StatelessElement};

    #[cfg(not(target_arch = "wasm32"))]
    fn dummy_async_handle() -> tokio::runtime::Handle {
        use std::sync::OnceLock;

        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let _guard = runtime.enter();
        tokio::runtime::Handle::current()
    }

    fn dummy_build_context() -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        BuildContext::new(
            canvas,
            Default::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            dummy_async_handle(),
        )
    }

    struct TestLeaf;
    impl VisitorElement for TestLeaf {
        fn debug_name(&self) -> &'static str {
            "TestLeaf"
        }
    }
    impl Drawable for TestLeaf {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl LayoutElement for TestLeaf {}
    impl EventElement for TestLeaf {}
    impl Rebuildable for TestLeaf {}

    #[allow(dead_code)]
    struct PaintCountWidget {
        draws: Rc<Cell<usize>>,
        paints: Rc<Cell<usize>>,
    }

    #[allow(dead_code)]
    struct PaintCountElement {
        draws: Rc<Cell<usize>>,
        paints: Rc<Cell<usize>>,
    }

    impl Widget for PaintCountWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            PaintCountElement {
                draws: self.draws,
                paints: self.paints,
            }
            .boxed()
        }
    }

    impl crate::widget::PortableWidget for PaintCountWidget {}

    impl VisitorElement for PaintCountElement {
        fn debug_name(&self) -> &'static str {
            "PaintCountElement"
        }
    }

    impl Drawable for PaintCountElement {
        fn update(&self, _ctx: &BuildContext) {
            self.draws.set(self.draws.get() + 1);
        }

    }

    impl LayoutElement for PaintCountElement {
        fn is_layout_stable(&self) -> bool {
            true
        }
    }

    impl EventElement for PaintCountElement {}
    impl Rebuildable for PaintCountElement {}

    #[allow(dead_code)]
    struct PaintCountState {
        draws: Rc<Cell<usize>>,
        paints: Rc<Cell<usize>>,
    }

    #[allow(dead_code)]
    struct PaintCountStateful {
        draws: Rc<Cell<usize>>,
        paints: Rc<Cell<usize>>,
    }

    impl StatefulWidget for PaintCountStateful {
        type State = PaintCountState;

        fn create_state(self) -> Self::State {
            PaintCountState {
                draws: self.draws,
                paints: self.paints,
            }
        }
    }

    impl State<PaintCountStateful> for PaintCountState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            PaintCountWidget {
                draws: self.draws.clone(),
                paints: self.paints.clone(),
            }
        }
    }

    struct RebuildCountLeaf {
        rebuilds: Rc<Cell<usize>>,
    }

    impl VisitorElement for RebuildCountLeaf {
        fn debug_name(&self) -> &'static str {
            "RebuildCountLeaf"
        }
    }

    impl Drawable for RebuildCountLeaf {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl LayoutElement for RebuildCountLeaf {}
    impl EventElement for RebuildCountLeaf {}

    impl Rebuildable for RebuildCountLeaf {
        fn rebuild_if_dirty(&self, _ctx: &BuildContext) {
            self.rebuilds.set(self.rebuilds.get() + 1);
        }
    }

    struct DirtyStateWidget {
        builds: Rc<Cell<usize>>,
    }

    struct DirtyState {
        builds: Rc<Cell<usize>>,
    }

    impl StatefulWidget for DirtyStateWidget {
        type State = DirtyState;

        fn create_state(self) -> Self::State {
            DirtyState {
                builds: self.builds,
            }
        }
    }

    impl State<DirtyStateWidget> for DirtyState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            self.builds.set(self.builds.get() + 1);
            crate::ErrorWidget::new("state").boxed()
        }
    }

    /// An unkeyed stateful widget that records which of its states built the
    /// subtree that is live.
    ///
    /// Each state takes the next identity from `next_id`, so a state created by
    /// a rebuild is distinguishable from the one it was meant to replace.
    #[derive(Clone)]
    struct InnerProbe {
        next_id: Rc<Cell<usize>>,
        built_by: Rc<Cell<usize>>,
    }

    struct InnerProbeState {
        id: usize,
        built_by: Rc<Cell<usize>>,
    }

    impl StatefulWidget for InnerProbe {
        type State = InnerProbeState;

        fn create_state(self) -> Self::State {
            let id = self.next_id.get();
            self.next_id.set(id + 1);
            InnerProbeState {
                id,
                built_by: self.built_by.clone(),
            }
        }
    }

    impl State<InnerProbe> for InnerProbeState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            self.built_by.set(self.id);
            LeafProbe
        }
    }

    impl Widget for InnerProbe {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            StatefulElement::from_widget(self, ctx, "InnerProbe", None)
        }
    }

    impl crate::widget::PortableWidget for InnerProbe {}

    struct LeafProbe;

    impl Widget for LeafProbe {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            TestLeaf.boxed()
        }
    }

    impl crate::widget::PortableWidget for LeafProbe {}

    /// One of two interchangeable "pages" below the keyed element, distinguished
    /// only by the name it builds under, the shape a route's pages have.
    #[derive(Clone)]
    struct ProbePage {
        name: &'static str,
        inner: InnerProbe,
    }

    impl Widget for ProbePage {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let inner = self.inner;
            StatelessElement::from_builder(
                ctx,
                // The builder may run again on every rebuild, so each build
                // needs its own copy of the widget the conversion consumes.
                move |ctx| inner.clone().to_element(ctx),
                None,
                self.name,
            )
            .boxed()
        }

        fn debug_name(&self) -> &'static str {
            self.name
        }
    }

    impl crate::widget::PortableWidget for ProbePage {}

    /// A keyed stateful widget standing where a route transition stands: it
    /// keeps its own state by key, and everything below it does not.
    struct KeyedOuter {
        page: ProbePage,
    }

    struct KeyedOuterState {
        page: ProbePage,
    }

    impl StatefulWidget for KeyedOuter {
        type State = KeyedOuterState;

        fn create_state(self) -> Self::State {
            KeyedOuterState {
                page: self.page.clone(),
            }
        }
    }

    impl State<KeyedOuter> for KeyedOuterState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn adopt_config_from(&mut self, new: Self) {
            self.page = new.page;
        }

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            self.page.clone()
        }
    }

    impl Widget for KeyedOuter {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            StatefulElement::from_widget(
                self,
                ctx,
                "KeyedOuter",
                Some(crate::key::Key::from("keyed-outer")),
            )
        }
    }

    impl crate::widget::PortableWidget for KeyedOuter {}

    struct EventProbeWidget {
        events: Rc<Cell<usize>>,
    }

    struct EventProbeState {
        events: Rc<Cell<usize>>,
    }

    struct EventProbeChild {
        events: Rc<Cell<usize>>,
    }

    struct EventProbeElement {
        events: Rc<Cell<usize>>,
    }

    impl StatefulWidget for EventProbeWidget {
        type State = EventProbeState;

        fn create_state(self) -> Self::State {
            EventProbeState {
                events: self.events.clone(),
            }
        }
    }

    impl State<EventProbeWidget> for EventProbeState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            EventProbeChild {
                events: self.events.clone(),
            }
        }
    }

    impl Widget for EventProbeChild {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            EventProbeElement {
                events: self.events.clone(),
            }
            .boxed()
        }
    }

    impl crate::widget::PortableWidget for EventProbeChild {}

    impl VisitorElement for EventProbeElement {
        fn debug_name(&self) -> &'static str {
            "EventProbeElement"
        }
    }

    impl EventElement for EventProbeElement {
        fn event_tree_role(&self) -> EventTreeRole {
            EventTreeRole::IndexedTarget
        }

        fn on_event(&self, _event: &ElementEvent) -> EventResult {
            self.events.set(self.events.get() + 1);
            EventResult::ignored()
        }
    }

    impl LayoutElement for EventProbeElement {}
    impl Drawable for EventProbeElement {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl Rebuildable for EventProbeElement {}

    #[test]
    fn stateful_generated_child_receives_each_routed_event_once() {
        let events = Rc::new(Cell::new(0));
        let element = StatefulElement::from_widget(
            EventProbeWidget {
                events: events.clone(),
            },
            &dummy_build_context(),
            "EventProbeWidget",
            None,
        );
        assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);

        let _ = EventDispatcher::new().dispatch(
            &element,
            Vec2d::default(),
            &ElementEvent::PointerMove(PointerInfo::mouse(
                Vec2d::default(),
                PointerButton::Primary,
            )),
        );

        assert_eq!(events.get(), 1);
    }

    struct TraversalElement {
        id: usize,
        children: Vec<AnyElement>,
        visits: Rc<RefCell<Vec<usize>>>,
    }

    impl VisitorElement for TraversalElement {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            for child in &self.children {
                visitor(child.as_ref());
            }
        }

        fn debug_name(&self) -> &'static str {
            "TraversalElement"
        }
    }

    impl Drawable for TraversalElement {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl LayoutElement for TraversalElement {}
    impl EventElement for TraversalElement {}

    impl Rebuildable for TraversalElement {
        fn rebuild_if_dirty(&self, ctx: &BuildContext) {
            for child in &self.children {
                child.rebuild_if_dirty(ctx);
            }
        }

        fn is_stateful_element(&self) -> bool {
            self.visits.borrow_mut().push(self.id);
            false
        }
    }

    fn traversal_element(
        id: usize,
        children: Vec<AnyElement>,
        visits: &Rc<RefCell<Vec<usize>>>,
    ) -> AnyElement {
        TraversalElement {
            id,
            children,
            visits: visits.clone(),
        }
        .boxed()
    }

    #[test]
    fn state_updater_rebuilds_only_its_dirty_subtree() {
        let context = dummy_build_context();
        let builds = Rc::new(Cell::new(0));
        let (stateful, updater) = StatefulElement::new_with_name(
            DirtyStateWidget {
                builds: builds.clone(),
            },
            &context,
            "DirtyStateWidget",
            None,
        );
        let clean_rebuilds = Rc::new(Cell::new(0));
        let visits = Rc::new(RefCell::new(Vec::new()));
        let root = TraversalElement {
            id: 0,
            children: vec![
                StatelessElement::wrapper(
                    RebuildCountLeaf {
                        rebuilds: clean_rebuilds.clone(),
                    }
                    .boxed(),
                    None,
                    "CleanSibling",
                )
                .boxed(),
                stateful.boxed(),
            ],
            visits,
        }
        .boxed();

        root.rebuild_if_dirty(&context);
        clean_rebuilds.set(0);
        builds.set(0);

        updater.set_state(|_| {});
        root.rebuild_if_dirty(&context);

        assert_eq!(clean_rebuilds.get(), 0);
        assert_eq!(builds.get(), 1);
    }

    #[derive(Clone, Copy, Eq, PartialEq)]
    enum PanicPhase {
        None,
        CreateState,
        InitState,
        Build,
        ToElement,
        AdoptConfig,
        ChildRuntimeAdoption,
    }

    struct LifecycleWidget {
        phase: PanicPhase,
        updater: Rc<RefCell<Option<StateUpdater<LifecycleState>>>>,
        builds: Rc<Cell<usize>>,
    }

    struct LifecycleState {
        phase: PanicPhase,
        updater: Rc<RefCell<Option<StateUpdater<LifecycleState>>>>,
        builds: Rc<Cell<usize>>,
    }

    struct LifecycleChild {
        panic_in_to_element: bool,
        panic_in_runtime_adoption: bool,
    }

    struct LifecycleChildElement {
        panic_in_runtime_adoption: bool,
    }

    impl Widget for LifecycleChild {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            if self.panic_in_to_element {
                panic!("child conversion failed");
            }
            LifecycleChildElement {
                panic_in_runtime_adoption: self.panic_in_runtime_adoption,
            }
            .boxed()
        }
    }

    impl VisitorElement for LifecycleChildElement {
        fn debug_name(&self) -> &'static str {
            "LifecycleChildElement"
        }
    }

    impl Drawable for LifecycleChildElement {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl LayoutElement for LifecycleChildElement {}
    impl EventElement for LifecycleChildElement {}

    impl Rebuildable for LifecycleChildElement {
        fn adopt_runtime_state_from(&self, _old: &dyn Element) {
            if self.panic_in_runtime_adoption {
                panic!("child runtime-state adoption failed");
            }
        }
    }

    impl crate::widget::PortableWidget for LifecycleChild {}

    impl StatefulWidget for LifecycleWidget {
        type State = LifecycleState;

        fn create_state(self) -> Self::State {
            if self.phase == PanicPhase::CreateState {
                panic!("state creation failed");
            }
            LifecycleState {
                phase: self.phase,
                updater: self.updater.clone(),
                builds: self.builds.clone(),
            }
        }
    }

    impl State<LifecycleWidget> for LifecycleState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            if self.phase == PanicPhase::InitState {
                panic!("state initialization failed");
            }
            self.updater.replace(Some(updater));
        }

        fn adopt_config_from(&mut self, new: Self) {
            if new.phase == PanicPhase::AdoptConfig {
                panic!("keyed configuration failed");
            }
            self.phase = new.phase;
        }

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            self.builds.set(self.builds.get() + 1);
            if self.phase == PanicPhase::Build {
                panic!("state build failed");
            }
            LifecycleChild {
                panic_in_to_element: self.phase == PanicPhase::ToElement,
                panic_in_runtime_adoption: self.phase == PanicPhase::ChildRuntimeAdoption,
            }
        }
    }

    fn lifecycle_widget(phase: PanicPhase) -> LifecycleWidget {
        LifecycleWidget {
            phase,
            updater: Rc::new(RefCell::new(None)),
            builds: Rc::new(Cell::new(0)),
        }
    }

    fn assert_initial_phase_recovers(phase: PanicPhase) {
        let context = dummy_build_context();
        let element = StatefulElement::from_widget(
            lifecycle_widget(phase),
            &context,
            "LifecycleWidget",
            None,
        );
        assert_eq!(element.debug_name(), "ErrorWidget");
    }

    fn has_error_child(element: &dyn Element) -> bool {
        let mut found = false;
        element.visit_children(&mut |child| found |= child.debug_name() == "ErrorWidget");
        found
    }

    #[test]
    fn create_state_panic_becomes_error_element() {
        assert_initial_phase_recovers(PanicPhase::CreateState);
    }

    #[test]
    fn init_state_panic_becomes_error_element() {
        assert_initial_phase_recovers(PanicPhase::InitState);
    }

    #[test]
    fn initial_state_build_panic_becomes_error_element() {
        assert_initial_phase_recovers(PanicPhase::Build);
    }

    #[test]
    fn initial_child_conversion_panic_becomes_error_element() {
        assert_initial_phase_recovers(PanicPhase::ToElement);
    }

    #[test]
    fn child_reconciliation_failure_keeps_the_old_owner_and_rejects_the_candidate() {
        let context = dummy_build_context();
        let (old, old_updater) = StatefulElement::new_with_name(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            None,
        );
        let (new, candidate_updater) = StatefulElement::new_with_name(
            lifecycle_widget(PanicPhase::ChildRuntimeAdoption),
            &context,
            "LifecycleWidget",
            None,
        );

        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            carry_stateful(&old, &new, &context);
        }));

        assert!(result.is_ok(), "child reconciliation failure must be recovered");
        assert!(has_error_child(&new));
        assert_eq!(candidate_updater.try_read(|_| ()), None);
        drop(new);

        assert!(old_updater.read(|state| state.phase == PanicPhase::ChildRuntimeAdoption));
        assert_eq!(old_updater.try_read(|_| ()), None);
    }

    #[test]
    fn rebuild_child_reconciliation_failure_becomes_a_stable_error_element() {
        let context = dummy_build_context();
        let widget = lifecycle_widget(PanicPhase::None);
        let updater_slot = widget.updater.clone();
        let element = StatefulElement::from_widget(widget, &context, "LifecycleWidget", None);
        let updater = updater_slot.borrow().as_ref().unwrap().clone();
        updater.set_state(|state| state.phase = PanicPhase::ChildRuntimeAdoption);

        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            element.rebuild_if_dirty(&context);
        }));

        assert!(result.is_ok(), "child reconciliation failure must be recovered");
        assert!(has_error_child(element.as_ref()));
        assert_eq!(updater.try_read(|_| ()), None);
        element.rebuild_if_dirty(&context);
        assert!(has_error_child(element.as_ref()));
    }

    #[test]
    fn queued_mutation_panic_installs_stable_error_child() {
        let context = dummy_build_context();
        let widget = lifecycle_widget(PanicPhase::None);
        let updater_slot = widget.updater.clone();
        let mutation_attempts = Rc::new(Cell::new(0));
        let observed_attempts = mutation_attempts.clone();
        let element = StatefulElement::from_widget(widget, &context, "LifecycleWidget", None);
        let updater = updater_slot.borrow().as_ref().unwrap().clone();
        updater.set_state(move |_| {
            observed_attempts.set(observed_attempts.get() + 1);
            panic!("queued mutation failed");
        });

        element.rebuild_if_dirty(&context);
        assert!(has_error_child(element.as_ref()));
        element.rebuild_if_dirty(&context);
        assert_eq!(
            mutation_attempts.get(),
            1,
            "recovered mutation must not be retried"
        );
    }

    #[test]
    fn stale_rebuild_after_queued_panic_reuses_the_recovery_element() {
        let context = dummy_build_context();
        let widget = lifecycle_widget(PanicPhase::None);
        let updater_slot = widget.updater.clone();
        let element = StatefulElement::from_widget(widget, &context, "LifecycleWidget", None);
        let stateful = element
            .option_any()
            .and_then(|value| value.downcast_ref::<StatefulElement>())
            .expect("element should be stateful");
        let updater = updater_slot.borrow().as_ref().unwrap().clone();
        updater.set_state(|_| panic!("queued mutation failed"));

        let rebuild = unsafe { (&*stateful.rebuild_fn.0.get()).clone() };
        let first = rebuild(&context);
        assert_eq!(first.debug_name(), "ErrorWidget");

        let second = rebuild(&context);
        assert_eq!(
            second.debug_name(),
            "ErrorWidget",
            "stale callbacks must not rebuild the failed state again"
        );
    }

    #[test]
    fn dirty_state_build_panic_installs_stable_error_child() {
        let context = dummy_build_context();
        let widget = lifecycle_widget(PanicPhase::None);
        let updater_slot = widget.updater.clone();
        let builds = widget.builds.clone();
        let element = StatefulElement::from_widget(widget, &context, "LifecycleWidget", None);
        updater_slot
            .borrow()
            .as_ref()
            .unwrap()
            .set_state(|state| state.phase = PanicPhase::Build);

        element.rebuild_if_dirty(&context);
        assert!(has_error_child(element.as_ref()));
        element.rebuild_if_dirty(&context);
        assert_eq!(
            builds.get(),
            2,
            "recovered build must not be retried while clean"
        );
    }

    #[test]
    fn keyed_config_panic_becomes_error_element() {
        let context = dummy_build_context();
        let _scope = KeyedStateScope::enter();
        let key = crate::Key::Value("panic-recovery-key".to_owned());
        let live = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            Some(key.clone()),
        );
        assert_eq!(live.debug_name(), "LifecycleWidget");

        let recovered = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::AdoptConfig),
            &context,
            "LifecycleWidget",
            Some(key),
        );
        assert_eq!(recovered.debug_name(), "ErrorWidget");
    }

    #[test]
    fn failed_keyed_reconciliation_does_not_retry_the_live_state() {
        let context = dummy_build_context();
        let _scope = KeyedStateScope::enter();
        let key = crate::Key::Value("consumed-live-state-key".to_owned());
        let live = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            Some(key.clone()),
        );
        let live_stateful = live
            .option_any()
            .and_then(|value| value.downcast_ref::<StatefulElement>())
            .expect("live element should be stateful");
        let updater = live_stateful
            .state_updater::<LifecycleState>()
            .expect("live element should expose its state updater");
        let live_state = unsafe { &*live_stateful.state_any.0.get() }
            .downcast_ref::<SyncState<LifecycleState>>()
            .expect("live state should have the expected type");
        unsafe {
            (&mut *live_state.0.get()).take();
        }

        let recovered = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            Some(key),
        );

        assert_eq!(recovered.debug_name(), "ErrorWidget");
        updater.set_state(|_| {});
        live_stateful.rebuild_if_dirty(&context);
        assert!(has_error_child(live.as_ref()));
    }

    #[test]
    fn carried_config_panic_becomes_error_child_with_key_hint() {
        let context = dummy_build_context();
        let key = crate::Key::Value("carry-panic-key".to_owned());
        let old = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            Some(key.clone()),
        );
        let old_stateful = old
            .option_any()
            .and_then(|value| value.downcast_ref::<StatefulElement>())
            .expect("old element should be stateful");
        let old_state = unsafe { &*old_stateful.state_any.0.get() }
            .downcast_ref::<SyncState<LifecycleState>>()
            .expect("old state should have the expected type");
        unsafe {
            (&mut *old_state.0.get()).take();
        }

        let new = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            Some(key),
        );

        carry_stateful(old.as_ref(), new.as_ref(), &context);

        assert!(has_error_child(new.as_ref()));
    }

    #[test]
    fn failed_unkeyed_adoption_keeps_the_old_owner_and_rejects_the_candidate() {
        let context = dummy_build_context();
        let (old, old_updater) = StatefulElement::new_with_name(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            None,
        );
        let (new, candidate_updater) = StatefulElement::new_with_name(
            lifecycle_widget(PanicPhase::AdoptConfig),
            &context,
            "LifecycleWidget",
            None,
        );

        carry_stateful(&old, &new, &context);

        assert!(has_error_child(&new));
        assert_eq!(candidate_updater.try_read(|_| ()), None);
        drop(new);

        assert!(old_updater.read(|state| state.phase) == PanicPhase::None);
        assert_eq!(old_updater.try_read(|_| ()), None);
    }

    #[test]
    fn failed_state_reconciliation_does_not_report_a_second_internal_panic() {
        let context = dummy_build_context();
        let key = crate::Key::Value("carry-panic-key".to_owned());
        let old = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            Some(key.clone()),
        );
        let old_stateful = old
            .option_any()
            .and_then(|value| value.downcast_ref::<StatefulElement>())
            .expect("old element should be stateful");
        let updater = old_stateful
            .state_updater::<LifecycleState>()
            .expect("old element should expose its state updater");
        updater.set_state(|_| {});
        let old_state = unsafe { &*old_stateful.state_any.0.get() }
            .downcast_ref::<SyncState<LifecycleState>>()
            .expect("old state should have the expected type");
        unsafe {
            (&mut *old_state.0.get()).take();
        }

        let new = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            Some(key),
        );

        carry_stateful(old.as_ref(), new.as_ref(), &context);

        assert!(has_error_child(new.as_ref()));
        updater.set_state(|_| {});
        old_stateful.rebuild_if_dirty(&context);
        assert!(has_error_child(old.as_ref()));
    }

    #[test]
    fn stale_keyed_rebuild_reuses_the_original_recovery_diagnostic() {
        let context = dummy_build_context();
        let _scope = KeyedStateScope::enter();
        let key = crate::Key::Value("stale-rebuild-key".to_owned());
        let live = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            Some(key.clone()),
        );
        let live_stateful = live
            .option_any()
            .and_then(|value| value.downcast_ref::<StatefulElement>())
            .expect("live element should be stateful");
        let live_state = unsafe { &*live_stateful.state_any.0.get() }
            .downcast_ref::<SyncState<LifecycleState>>()
            .expect("live state should have the expected type");
        unsafe {
            (&mut *live_state.0.get()).take();
        }

        let recovered = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "LifecycleWidget",
            Some(key),
        );
        assert_eq!(recovered.debug_name(), "ErrorWidget");

        let rebuild = unsafe { (&*live_stateful.rebuild_fn.0.get()).clone() };
        let stale_child = rebuild(&context);
        let stale_debug = stale_child
            .option_any()
            .and_then(|value| value.downcast_ref::<crate::ErrorElement>())
            .map(|error| format!("{error:?}"))
            .expect("stale rebuild should return an error element");

        assert!(stale_debug.contains("adopt_config_from"), "{stale_debug}");
        assert!(!stale_debug.contains("queued state mutation"), "{stale_debug}");
        assert!(stale_debug.contains("stale-rebuild-key"), "{stale_debug}");
    }

    #[test]
    fn consumed_keyed_state_panic_explains_where_to_check_the_key() {
        let key = crate::Key::Value("duplicate-key".to_owned());

        let message = consumed_state_panic_message("LifecycleWidget", Some(&key));

        assert!(message.contains("LifecycleWidget"));
        assert!(message.contains("duplicate-key"));
        assert!(message.contains("value Value(\"duplicate-key\")"));
        assert!(!message.contains("create_location"));
        assert!(!message.contains(file!()));
        assert!(message.contains("key is created and used"));
        assert!(message.contains("stable and unique"));
    }

    #[test]
    fn consumed_keyed_state_diagnostic_points_to_key_creation() {
        let key = crate::Key::Value("creation-site-key".to_owned());
        let diagnostic = recover_operation("LifecycleWidget", BuildPhase::AdoptConfig, || {
            panic!("{}", consumed_state_panic_message("LifecycleWidget", Some(&key)))
        })
        .expect_err("the consumed-state diagnostic should be recovered");

        let message = diagnostic.with_site(key.diagnostic_site()).to_string();

        assert!(message.contains("at "), "{message}");
        assert!(message.contains(file!()), "{message}");
        assert!(message.contains("let key = crate::Key::Value"), "{message}");
        assert!(message.contains("^"), "{message}");
        assert!(!message.contains("create_location"), "{message}");
    }

    #[test]
    fn keyed_subtree_registration_visits_depth_first_in_child_order() {
        let visits = Rc::new(RefCell::new(Vec::new()));
        let first = traversal_element(
            1,
            vec![
                traversal_element(3, vec![], &visits),
                traversal_element(4, vec![], &visits),
            ],
            &visits,
        );
        let second = traversal_element(2, vec![traversal_element(5, vec![], &visits)], &visits);
        let root = traversal_element(0, vec![first, second], &visits);

        register_keyed_subtree(root.as_ref());

        assert_eq!(*visits.borrow(), vec![0, 1, 3, 4, 2, 5]);
    }

    #[test]
    fn nested_rebuilds_share_one_keyed_subtree_scan() {
        let context = dummy_build_context();
        let visits = Rc::new(RefCell::new(Vec::new()));
        let nested = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "NestedLifecycleWidget",
            None,
        );
        let nested_stateful = nested
            .option_any()
            .and_then(|element| element.downcast_ref::<StatefulElement>())
            .unwrap();
        unsafe {
            *nested_stateful.child.0.get() = traversal_element(2, vec![], &visits);
        }

        let outer = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "OuterLifecycleWidget",
            None,
        );
        let outer_stateful = outer
            .option_any()
            .and_then(|element| element.downcast_ref::<StatefulElement>())
            .unwrap();
        unsafe {
            *outer_stateful.child.0.get() = traversal_element(1, vec![nested], &visits);
        }
        outer_stateful.dirty.borrow().set(true);

        outer_stateful.rebuild_if_dirty(&context);

        assert_eq!(*visits.borrow(), vec![1, 2, 1]);
    }

    #[test]
    fn clean_stateful_rebuild_skips_keyed_subtree_registration() {
        let context = dummy_build_context();
        let visits = Rc::new(RefCell::new(Vec::new()));
        let outer = StatefulElement::from_widget(
            lifecycle_widget(PanicPhase::None),
            &context,
            "OuterLifecycleWidget",
            None,
        );
        let outer_stateful = outer
            .option_any()
            .and_then(|element| element.downcast_ref::<StatefulElement>())
            .unwrap();
        unsafe {
            *outer_stateful.child.0.get() = traversal_element(1, vec![], &visits);
        }

        outer_stateful.rebuild_if_dirty(&context);

        assert!(visits.borrow().is_empty());
    }

    #[test]
    fn local_mutation_queue_preserves_order_and_accepts_reentrant_updates() {
        let queue: Rc<StateMutationQueue<Vec<i32>>> = Rc::new(StateMutationQueue::default());
        let nested_queue = queue.clone();
        let mut state = Vec::new();

        queue.push(Box::new(move |state| {
            state.push(1);
            nested_queue.push(Box::new(|state| state.push(3)));
        }));
        queue.push(Box::new(|state| state.push(2)));

        assert_eq!(queue.drain_into(&mut state, || {}), 3);
        assert_eq!(state, vec![1, 2, 3]);
        assert_eq!(queue.drain_into(&mut state, || {}), 0);
    }

    /// A keyed element is found by key, so it keeps its state wherever it
    /// reappears, but the subtree it rebuilds from that state is built anew,
    /// and what lives below it is unkeyed.
    ///
    /// This is the route transition an application wraps every page in: a theme
    /// change rebuilds the shell above it, and the request the page completed,
    /// an `AsyncBuilder`'s snapshot, must not start over because the switcher
    /// between them was rebuilt.
    #[test]
    fn state_below_a_keyed_element_outlives_a_rebuild_of_its_ancestor() {
        let context = dummy_build_context();
        let built_by = Rc::new(Cell::new(usize::MAX));
        let page = ProbePage {
            name: "ProbePageOne",
            inner: InnerProbe {
                next_id: Rc::new(Cell::new(0)),
                built_by: built_by.clone(),
            },
        };

        let old = KeyedOuter { page: page.clone() }.to_element(&context);
        assert_eq!(built_by.get(), 0, "the first subtree builds from its state");

        let new = KeyedOuter { page: page.clone() }.to_element(&context);
        assert_eq!(built_by.get(), 1, "a fresh subtree builds from fresh state");

        carry_child_state(old.as_ref(), new.as_ref(), &context);

        assert_eq!(
            built_by.get(),
            0,
            "the state below the keyed element was replaced by a fresh one"
        );
    }

    /// The other half of the same rule: a keyed element outlives changes to what
    /// it contains, so what it contains must be free to change.
    ///
    /// One route transition keeps its identity across every route. When the page
    /// inside it is replaced, the outgoing page's state must stay with the
    /// outgoing page, handing it to the page that replaced it would render the
    /// route that was navigated away from.
    #[test]
    fn a_replaced_page_below_a_keyed_element_keeps_its_own_state() {
        let context = dummy_build_context();
        let built_by = Rc::new(Cell::new(usize::MAX));
        let next_id = Rc::new(Cell::new(0));

        let old = KeyedOuter {
            page: ProbePage {
                name: "ProbePageOne",
                inner: InnerProbe {
                    next_id: next_id.clone(),
                    built_by: built_by.clone(),
                },
            },
        }
        .to_element(&context);
        assert_eq!(built_by.get(), 0);

        let new = KeyedOuter {
            page: ProbePage {
                name: "ProbePageTwo",
                inner: InnerProbe {
                    next_id: next_id.clone(),
                    built_by: built_by.clone(),
                },
            },
        }
        .to_element(&context);
        let page_two_state = built_by.get();
        assert_ne!(page_two_state, 0, "the second page builds from its own state");

        carry_child_state(old.as_ref(), new.as_ref(), &context);

        assert_ne!(
            built_by.get(),
            0,
            "the replaced page's state was handed to the page that replaced it"
        );
    }

    #[test]
    fn test_state_updater_empty_panic() {
        let updater: StateUpdater<i32> = StateUpdater::empty();
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            updater.set_state(|_s| {});
        }));
        assert!(result.is_err());

        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            updater.read(|_s| {});
        }));
        assert!(result.is_err());
    }

    #[test]
    fn try_operations_on_an_empty_updater_are_quiet_noops() {
        let updater: StateUpdater<i32> = StateUpdater::empty();
        let callback_called = Rc::new(Cell::new(false));
        let callback_called_by_read = callback_called.clone();
        assert_eq!(
            updater.try_read(move |_| {
                callback_called_by_read.set(true);
                1
            }),
            None,
        );
        let callback_called_by_mutation = callback_called.clone();
        assert_eq!(
            updater.try_set_state(move |_| callback_called_by_mutation.set(true)),
            None,
        );
        assert!(updater.try_read_state().is_none());
        assert!(!callback_called.get());
    }

    struct StaleUpdaterWidget;

    struct StaleUpdaterState {
        value: String,
    }

    impl StatefulWidget for StaleUpdaterWidget {
        type State = StaleUpdaterState;

        fn create_state(self) -> Self::State {
            Self::State {
                value: String::from("live"),
            }
        }
    }

    impl State<StaleUpdaterWidget> for StaleUpdaterState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            LeafProbe
        }
    }

    #[test]
    fn try_operations_reject_a_copied_updater_after_unmount() {
        let context = dummy_build_context();
        let (element, updater) = StatefulElement::new_with_name(
            StaleUpdaterWidget,
            &context,
            "StaleUpdaterWidget",
            None,
        );
        let copied = updater;

        assert_eq!(copied.try_read(|state| state.value.clone()), Some(String::from("live")));
        assert_eq!(
            copied.try_set_state(|state| state.value.push_str("-updated")),
            Some(()),
        );
        assert_eq!(
            copied.try_set_state(|state| state.value.push_str("-again")),
            Some(()),
        );
        element.rebuild_if_dirty(&context);
        assert_eq!(updater.try_read(|state| state.value.clone()), Some(String::from("live-updated-again")));

        let guard = copied.try_read_state().expect("live state should be readable");

        drop(element);

        assert_eq!(guard.value, "live-updated-again");
        drop(guard);

        let callback_called = Rc::new(Cell::new(false));
        let callback_called_by_mutation = callback_called.clone();
        assert_eq!(
            copied.try_set_state(move |_| callback_called_by_mutation.set(true)),
            None,
        );
        assert!(!callback_called.get());
        assert_eq!(copied.try_read(|_| String::from("unexpected")), None);
        assert!(copied.try_read_state().is_none());
    }

    #[test]
    fn delayed_callback_rejects_all_state_operations_after_native_unmount() {
        let context = dummy_build_context();
        let (element, updater) = StatefulElement::new_with_name(
            StaleUpdaterWidget,
            &context,
            "StaleUpdaterWidget",
            None,
        );
        let read_called = Rc::new(Cell::new(false));
        let mutation_called = Rc::new(Cell::new(false));
        let delayed: Rc<RefCell<Option<Box<dyn FnOnce()>>>> = Rc::new(RefCell::new(None));
        let delayed_callback = delayed.clone();
        let read_called_by_callback = read_called.clone();
        let mutation_called_by_callback = mutation_called.clone();
        delayed_callback.borrow_mut().replace(Box::new(move || {
            assert!(!updater.has_state());
            assert_eq!(
                updater.try_read(move |_| {
                    read_called_by_callback.set(true);
                }),
                None,
            );
            assert_eq!(
                updater.try_set_state(move |_| mutation_called_by_callback.set(true)),
                None,
            );
            assert!(updater.try_read_state().is_none());
        }));

        drop(element);
        delayed
            .borrow_mut()
            .take()
            .expect("delayed callback should be retained")();

        assert!(!read_called.get());
        assert!(!mutation_called.get());
    }

    #[test]
    fn has_state_and_unchecked_updates_require_a_live_updater() {
        let context = dummy_build_context();
        let (element, updater) = StatefulElement::new_with_name(
            StaleUpdaterWidget,
            &context,
            "StaleUpdaterWidget",
            None,
        );
        assert!(updater.has_state());

        unsafe {
            updater.set_state_unchecked(|state| state.value.push_str("-unchecked"));
        }
        element.rebuild_if_dirty(&context);
        assert_eq!(updater.try_read(|state| state.value.clone()), Some(String::from("live-unchecked")));
        let unchecked_guard = unsafe { updater.read_state_unchecked() };
        assert_eq!(unchecked_guard.value, "live-unchecked");

        drop(element);
        assert!(!updater.has_state());
        assert_eq!(unchecked_guard.value, "live-unchecked");
        drop(unchecked_guard);
    }

    struct OwnedConfigWidget;

    struct OwnedConfigState {
        config: String,
    }

    impl StatefulWidget for OwnedConfigWidget {
        type State = OwnedConfigState;

        fn create_state(self) -> Self::State {
            OwnedConfigState {
                config: String::from("new"),
            }
        }
    }

    impl State<OwnedConfigWidget> for OwnedConfigState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn adopt_config_from(&mut self, new: Self) {
            self.config = new.config;
        }

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            LeafProbe
        }
    }

    impl Widget for OwnedConfigWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            StatefulElement::from_widget(self, ctx, "OwnedConfigWidget", None)
        }
    }

    impl crate::widget::PortableWidget for OwnedConfigWidget {}

    #[test]
    fn adopt_config_from_consumes_the_new_state() {
        let mut state = OwnedConfigState {
            config: String::from("old"),
        };

        state.adopt_config_from(OwnedConfigState {
            config: String::from("new"),
        });

        assert_eq!(state.config, "new");
    }
}

/// Asks for the frame that applies a state update.
///
/// A state update made by a task (a timer firing, a request finishing) belongs
/// to a frame that exists for nothing else, and it rebuilds exactly one
/// subtree, so that frame may be scoped. Any other state update (from input
/// handling, from the middle of a draw, from code that then goes on to change
/// something else) has no such guarantee about what the rest of the frame must
/// do, and keeps the ordinary full frame it always had.
#[inline]
pub(crate) fn request_state_frame(window: aimer_events::window::WindowKey) {
    aimer_events::window::with_window_scope(window, || {
        if aimer_venus::is_polling_task() {
            request_scoped_frame();
        } else {
            request_animation_frame();
        }
    });
}
