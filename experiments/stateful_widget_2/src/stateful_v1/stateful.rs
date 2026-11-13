use std::any::Any;
use std::cell::{Cell, RefCell, UnsafeCell};
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::{Rc, Weak};

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_events::window::{request_animation_frame, request_scoped_frame};
use aimer_utils::{debug, error};

use aimer_widget_core::base::*;
use aimer_widget_core::components::element::{
    DirtySource, identities_are_compatible, structural_children,
};
use super::state_slots::{
    StateSlotReadGuard, StateSlotResolutionError, StateStorage, StateUpdaterGeneration,
};
use super::recovery::{BuildPhase, PanicDiagnostic, recover_operation};
use aimer_widget_core::{
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
#[doc(hidden)]
pub struct SyncChild(pub UnsafeCell<AnyElement>);
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
            .map(|message| aimer_widget_core::ErrorElement::new(message.clone()).boxed())
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
/// require_send::<aimer_widget_core::StateUpdater<()>>();
/// require_sync::<aimer_widget_core::StateUpdater<()>>();
/// ```
pub struct StateUpdater<S> {
    slot: Option<StateUpdaterGeneration<UpdaterSlot<S>>>,
    _ui_thread_only: PhantomData<Rc<()>>,
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
/// The guard retains an arena read lease and dereferences to `S`, preserving
/// ordinary `read_state().field` use.
#[doc(hidden)]
pub struct StateReadGuard<'a, S: 'static> {
    state: *const S,
    _slot: StateSlotReadGuard<UpdaterSlot<S>>,
    _marker: PhantomData<&'a S>,
}

impl<S: 'static> std::ops::Deref for StateReadGuard<'_, S> {
    type Target = S;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: the guard owns `_slot`, whose read lease keeps the erased
        // updater slot and its state alive.
        unsafe { &*self.state }
    }
}

struct UpdaterSlot<S> {
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
        let (storage, slot) = StateStorage::insert(UpdaterSlot {
            tx,
            state,
            dirty_source,
            failure,
            window,
        });
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
        let state = unsafe {
            (&*slot.state.0.get())
                .as_ref()
                .unwrap_or_else(|| panic_consumed_state())
                as *const S
        };
        StateReadGuard {
            state,
            _slot: slot,
            _marker: PhantomData,
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
    /// element is still live, its slot contains the expected state, and the
    /// state has not been consumed or failed. The call must remain on the UI
    /// thread. Violating these requirements is undefined behavior. Prefer [`Self::try_read_state`]
    /// when the updater may outlive its state.
    #[track_caller]
    pub unsafe fn read_state_unchecked(&self) -> StateReadGuard<'_, S> {
        // SAFETY: the caller guarantees that this updater is initialized.
        let slot = unsafe { self.slot.unwrap_unchecked() };
        // SAFETY: the caller guarantees a live, correctly typed slot and a
        // live owner lease while the read guard is acquired.
        let slot = unsafe { slot.read_guard_unchecked() };
        // SAFETY: the caller guarantees that the state exists and has not
        // been consumed or failed.
        let state = unsafe { (&*slot.state.0.get()).as_ref().unwrap_unchecked() as *const S };
        StateReadGuard {
            state,
            _slot: slot,
            _marker: PhantomData,
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
        slot.try_resolve(|inner| {
            if inner.failure.is_failed() {
                return false;
            }
            // Safety: the native rendering pipeline is single-threaded.
            unsafe { (&*inner.state.0.get()).is_some() }
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
        slot.try_resolve(|inner| {
            if inner.failure.is_failed() {
                return None;
            }
            // Safety: the native rendering pipeline is single-threaded.
            let state = unsafe { (&*inner.state.0.get()).as_ref()? };
            Some(read(state))
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
        aimer_widget_core::frame_work_stats::record_state_update();
        let slot = match self.slot {
            Some(slot) => slot,
            None => {
                let loc = Location::caller();
                self.beautiful_error(loc);
                panic!("State is not initialized (see error above)");
            }
        };
        let result = slot.try_resolve(|inner| {
            if inner.failure.is_failed() {
                return;
            }
            inner.tx.push(Box::new(f));
            // Only request a redraw if this is the first set_state since the last rebuild.
            // This coalesces multiple set_state calls into a single redraw request.
            if inner.dirty_source.mark() {
                {
                    aimer_widget_core::components::element::note_state_update_marked();
                    request_state_frame(inner.window);
                }
            }
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
        slot.try_resolve(|inner| {
            if inner.failure.is_failed() {
                return None;
            }
            // Safety: the native rendering pipeline is single-threaded.
            unsafe { (&*inner.state.0.get()).as_ref() }?;
            inner.tx.push(Box::new(mutation));
            if inner.dirty_source.mark() {
                {
                    aimer_widget_core::components::element::note_state_update_marked();
                    request_state_frame(inner.window);
                }
            }
            Some(())
        })
        .ok()
        .flatten()
    }

    /// Queues a state mutation without performing updater validity checks.
    ///
    /// # Safety
    ///
    /// The caller must ensure that this updater is initialized, its owning
    /// element is still live, the updater's state slot has
    /// not been consumed or failed, and the call is made on the UI thread.
    /// Violating those requirements is undefined behavior. Prefer
    /// [`Self::try_set_state`] when the updater may outlive its state.
    #[track_caller]
    pub unsafe fn set_state_unchecked(&self, mutation: impl FnOnce(&mut S) + 'static) {
        aimer_widget_core::frame_work_stats::record_state_update();
        // SAFETY: the caller guarantees that this updater is initialized.
        let slot = unsafe { self.slot.unwrap_unchecked() };
        // SAFETY: the caller guarantees a live, correctly typed slot and a
        // live owner lease for the duration of this lookup.
        unsafe {
            slot.resolve_unchecked(|inner| {
                inner.tx.push(Box::new(mutation));
                if inner.dirty_source.mark() {
                    {
                        aimer_widget_core::components::element::note_state_update_marked();
                        request_state_frame(inner.window);
                    }
                }
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
        slot.try_resolve(|inner| {
            // Safety: single-threaded rendering pipeline, no concurrent mutation.
            let state = unsafe {
                (&*inner.state.0.get())
                    .as_ref()
                    .unwrap_or_else(|| panic_consumed_state())
            };
            f(state)
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
        if slot.failure.is_failed() {
            return None;
        }
        let state = unsafe { (&*slot.state.0.get()).as_ref()? as *const S };
        Some(StateReadGuard {
            state,
            _slot: slot,
            _marker: PhantomData,
        })
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
/// [`StatelessWidget`](aimer_widget_core::StatelessWidget).
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

fn consumed_state_panic_message(debug_name: &'static str, key: Option<&aimer_widget_core::Key>) -> String {
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

aimer_tls::thread_local! {
    static KEYED_STATE_REGISTRY: RefCell<HashMap<(aimer_widget_core::key::Key, &'static str), KeyedStateEntry>> =
        RefCell::new(HashMap::new());
    static KEYED_STATE_SCOPE_DEPTH: Cell<usize> = const { Cell::new(0) };
}

pub(super) struct KeyedStateScope {
    owns_registry: bool,
}

impl KeyedStateScope {
    pub(super) fn enter() -> Self {
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
    pub key: Option<aimer_widget_core::key::Key>,
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
        key: Option<aimer_widget_core::key::Key>,
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
        key: Option<aimer_widget_core::key::Key>,
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
        key: Option<aimer_widget_core::key::Key>,
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
            ctx.window().key(),
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
        aimer_widget_core::rebuild_stats::record_stateful_check();
        let invalidation_generation =
            aimer_widget_core::components::element::rebuild_invalidation_generation();
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
            aimer_widget_core::rebuild_stats::record_stateful_build();
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
                aimer_widget_core::components::element::reconcile_carried_tree(
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

#[doc(hidden)]
pub use aimer_widget_core::components::element::{carry_child_state, carry_stateful};

fn element_children(element: &dyn Element) -> smallvec::SmallVec<[&dyn Element; 8]> {
    aimer_widget_core::components::element::structural_children(element)
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
        // A name and a key say which element a state *was*, not what it holds.
        // Generic widgets share one name across their type parameters (every
        // `Provider<T, W>` is "Provider"), so a replaced subtree can pair an
        // element with one whose state is another type entirely. Taking over
        // that rebuild closure would make this element build from a state that
        // is not its own — here, with the old provider's handle under the new
        // subtree — so a state of another kind stays where it is, as the
        // configuration hook below already assumes.
        if Any::type_id(&**self_state) != Any::type_id(&**old_state) {
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
            aimer_widget_core::components::element::without_placement_skip(|| rf(ctx))
        };
        // Carry live state from nested StatefulElements into the new tree.
        let child_reconciliation = recover_operation(
            self.debug_name.get(),
            BuildPhase::ReconcileChildren,
            || {
                let old_child = unsafe { &*self.child.0.get() };
                aimer_widget_core::components::element::reconcile_carried_tree(
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

fn lookup_keyed_state(key: &aimer_widget_core::key::Key, debug_name: &'static str) -> Option<LiveKeyedState> {
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
    // Walking a state owner again rebuilds its child only if it is dirty, which
    // is a change a recorded subtree notices through the generations; what else
    // it holds that moves has to ask for frames, and a frame request counts
    // against the subtree.
    #[inline]
    fn reusable_walk(&self) -> bool {
        true
    }

    fn update(&self, ctx: &BuildContext) {
        self.rebuild_if_dirty(ctx);
        // Safety: single-threaded rendering pipeline
        let child = unsafe { &*self.child.0.get() };

        // The retained tree draws the generated child through its own render
        // nodes; this element only keeps the state owner's subtree current.
        // `child.update` follows, so the child may be spared the forward when
        // it will sync its own geometry there.
        {
            let _promise = aimer_element::ChildUpdateFollows::enter(true);
            child.forward_paint_geometry(ctx);
        }
        child.update(ctx);
    }

    #[inline]
    fn paints_nothing(&self) -> bool {
        true
    }

    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        // This wrapper owns state and rebuilds its child, but paints nothing.
        true
    }

    fn paint(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        canvas.finish();
    }

    #[inline]
    fn paint_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        // A state owner is visually transparent. Preserve a child's
        // viewport-sized render bounds instead of expanding the wrapper to
        // the child's scrollable content extent.
        unsafe { &*self.child.0.get() }.paint_bounds(ctx)
    }

    #[inline]
    fn paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        unsafe { &*self.child.0.get() }.paint_outsets(ctx)
    }

    #[inline]
    fn child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn aimer_widget_core::Element,
    ) -> Option<(
        aimer_cupid::draw_cmd::Rect,
        Option<aimer_cupid::draw_cmd::Rect>,
    )> {
        unsafe { &*self.child.0.get() }.child_geometry(ctx, child)
    }

    #[inline]
    fn child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn aimer_widget_core::Element,
        child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd::Rect,
        Option<aimer_cupid::draw_cmd::Rect>,
    )> {
        unsafe { &*self.child.0.get() }
            .child_geometry_at(ctx, child, child_index)
    }

    #[inline]
    fn child_context_at<'a>(
        &self,
        ctx: &aimer_widget_core::base::BuildContext<'a>,
        child: &dyn aimer_widget_core::Element,
        child_index: usize,
    ) -> Option<aimer_widget_core::base::BuildContext<'a>> {
        unsafe { &*self.child.0.get() }
            .child_context_at(ctx, child, child_index)
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let child = unsafe { &*self.child.0.get() };
        child.forward_paint_geometry(ctx);
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

    fn reconciliation_key(&self) -> Option<&aimer_widget_core::Key> {
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

impl aimer_widget_core::components::element::StateCarrier for StatefulElement {
    #[inline]
    fn carrier_key(&self) -> Option<&aimer_widget_core::key::Key> {
        self.key.as_ref()
    }

    #[inline]
    fn carrier_name(&self) -> &'static str {
        self.debug_name.get()
    }

    #[inline]
    fn carrier_revision(&self) -> u64 {
        self.state_revision.borrow().get()
    }

    fn adopt_state_from(&self, old: &dyn aimer_widget_core::components::element::StateCarrier, ctx: &BuildContext) {
        // A carrier is only ever paired with one of its own kind.
        if let Some(old) = old.as_any().downcast_ref::<StatefulElement>() {
            StatefulElement::adopt_state_from(self, old, ctx);
        }
    }

    #[inline]
    fn as_element(&self) -> &dyn Element {
        self
    }

    #[inline]
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl aimer_widget_core::components::rebuildable::SelfRebuilding for StatefulElement {
    #[inline]
    fn is_dirty(&self) -> bool {
        self.dirty.borrow().get()
    }

    #[inline]
    fn set_rebuild_path(&self, path: &[aimer_widget_core::ElementId]) {
        self.dirty_source.borrow().set_path(path);
    }

    #[inline]
    fn set_rebuild_path_if_missing(&self, path: &[aimer_widget_core::ElementId]) {
        self.dirty_source.borrow().set_path_if_missing(path);
    }

    #[inline]
    fn reads_inherited_context(&self) -> bool {
        self.dirty_source.borrow().reads_context()
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

    #[inline]
    fn self_rebuilding(&self) -> Option<&dyn aimer_widget_core::components::rebuildable::SelfRebuilding> {
        Some(self)
    }

    #[inline]
    fn state_carrier(&self) -> Option<&dyn aimer_widget_core::components::element::StateCarrier> {
        Some(self)
    }

    fn mark_needs_rebuild(&self) {
        let _mark = aimer_widget_core::components::element::begin_rebuild_mark();
        aimer_widget_core::components::element::with_rebuild_invalidation(|| {
            // Safety: single-threaded rendering pipeline.
            let child = unsafe { &*self.child.0.get() };
            let carries_child_state = child.is_carry_state();
            if !carries_child_state {
                let source = self.dirty_source.borrow();
                // A placement only needs to reach builds that observed inherited
                // context; this one read none, so it cannot differ from last time.
                if !(aimer_widget_core::components::element::placement_mark_active()
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
                aimer_widget_core::components::element::invalidate_dirty_paths();
            }
        });
    }
}

#[cfg(test)]
mod rebuild_scope_tests;

#[cfg(test)]
mod tests;

/// Asks for the frame that applies a state update.
///
/// A state update made by a task (a timer firing, a request finishing) belongs
/// to a frame that exists for nothing else, and it rebuilds exactly one
/// subtree, so that frame may be scoped. Any other state update (from input
/// handling, from the middle of a draw, from code that then goes on to change
/// something else) has no such guarantee about what the rest of the frame must
/// do, and keeps the ordinary full frame it always had.
#[inline]
#[doc(hidden)]
pub fn request_state_frame(window: aimer_events::window::WindowKey) {
    aimer_events::window::with_window_scope(window, || {
        if aimer_venus::is_polling_task() {
            request_scoped_frame();
        } else {
            request_animation_frame();
        }
    });
}
