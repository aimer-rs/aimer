mod snapshot_task;

pub use snapshot_task::{AsyncSnapshotError, FutureSnapshotExt, SnapshotTask};

use std::cell::{Cell, RefCell, UnsafeCell};
use std::future::Future;
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::{Rc, Weak};

use aimer_attribute::{ResolvedSize, Size, Vec2d};
use aimer_venus::{ScopeId, TaskScope, Venus};

use crate::base::BuildContext;
use crate::widget::stateful::{SyncChild, carry_child_state};
use crate::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, Key, LayoutElement, Rebuildable,
    PortableWidget, RequiredChild, State, StateUpdater, StatefulElement, StatefulWidget,
    VisitorElement, Widget,
};

/// The current state of an asynchronous operation.
///
/// Used by [`AsyncBuilder`] and by the live handles returned from
/// [`FutureSnapshotExt::waitme`].
pub enum AsyncSnapshot<T, E> {
    Waiting,
    Data(T),
    Error(E),
}

#[doc(hidden)]
pub struct FutureFactory<F, T, E> {
    factory: Rc<F>,
    marker: PhantomData<fn() -> Result<T, E>>,
}

#[doc(hidden)]
pub struct SnapshotBuilder<B, T, E> {
    builder: Rc<B>,
    marker: PhantomData<fn(&AsyncSnapshot<T, E>)>,
}

/// Builds one subtree from the eventual result of an asynchronous operation.
///
/// For a live snapshot without a widget builder, use [`FutureSnapshotExt::waitme`].
///
/// The future starts after the widget is mounted. Rebuilding a parent with the
/// same request key keeps the current operation; changing the request key
/// cancels it and starts a new one. The snapshot begins at
/// [`AsyncSnapshot::Waiting`], then changes exactly once to
/// [`AsyncSnapshot::Data`] or [`AsyncSnapshot::Error`] for the active request.
/// Cancelling destroys the request outright, so a cancelled one has no answer
/// left to discard, and an in-flight future is dropped when the widget is.
///
/// The future is driven by [`aimer_venus`] on the thread that owns the frame,
/// which has two consequences worth knowing:
///
/// - It does **not** have to be [`Send`], so it may capture a `StateUpdater`, a
///   controller, or anything else the element tree handed out.
/// - It shares the frame with build, layout and paint. Awaiting I/O is free,
///   but *blocking* work belongs on
///   [`Venus::offload`](aimer_venus::Venus::offload), which runs it on a worker
///   thread and resolves back here.
///
/// Futures from a runtime-backed ecosystem — `reqwest`, `tokio::fs`, a `sleep`
/// — keep working: the application installs a
/// [`PollContext`](aimer_venus::PollContext) on its runtime, so every poll of
/// every task happens inside the async runtime it owns.
///
/// ```rust
/// use aimer_widget::{AsyncBuilder, AsyncSnapshot, ErrorWidget, Widget};
///
/// let user_id = 7_u64;
/// let builder =
///     AsyncBuilder::new()
///         .request_key(user_id)
///         .future(move || async move { Ok::<_, &'static str>(user_id) })
///         .child(|snapshot| match snapshot {
///             AsyncSnapshot::Waiting => ErrorWidget::new("Loading user").boxed(),
///             AsyncSnapshot::Data(id) => {
///                 ErrorWidget::new(format!("User {id}")).boxed()
///              }
///             AsyncSnapshot::Error(error) => ErrorWidget::new(*error).boxed()
///         });
/// ```

#[derive(aimer_macro::PortableWidget)]
#[portable_widget(
    id = "aimer_widget::AsyncBuilder",
    schema_only,
    manual_lowering
)]
pub struct AsyncBuilder<K = (), F = RequiredChild, B = RequiredChild> {
    #[portable_skip]
    request_key: K,
    #[portable_skip]
    future_factory: F,
    #[portable_skip]
    snapshot_builder: B,
    #[portable_skip]
    widget_key: Option<Key>,
}

impl AsyncBuilder {
    /// Creates an incomplete async builder in the waiting state.
    ///
    /// The default request key is `()`. Call [`future`](Self::future), then
    /// [`child`](AsyncBuilder::child), to produce a valid [`Widget`].
    pub fn new() -> Self {
        Self {
            request_key: (),
            future_factory: RequiredChild,
            snapshot_builder: RequiredChild,
            widget_key: None,
        }
    }
}

impl Default for AsyncBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, F, B> AsyncBuilder<K, F, B> {
    /// Sets the logical identity of the asynchronous request.
    ///
    /// An equal key preserves the existing future and snapshot across parent
    /// rebuilds. A changed key aborts the old request, returns the snapshot to
    /// [`AsyncSnapshot::Waiting`], and starts the new future after mounting.
    pub fn request_key<NK>(self, request_key: NK) -> AsyncBuilder<NK, F, B> {
        AsyncBuilder {
            request_key,
            future_factory: self.future_factory,
            snapshot_builder: self.snapshot_builder,
            widget_key: self.widget_key,
        }
    }

    /// Sets this widget's reconciliation key.
    ///
    /// This is distinct from [`request_key`](Self::request_key): the widget key
    /// identifies the element, while the request key controls future reuse.
    #[track_caller]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        let caller = Location::caller();
        self.widget_key = Some(key.into().with_location(caller));
        self
    }
}

impl<K, B> AsyncBuilder<K, RequiredChild, B> {
    /// Supplies a factory for the asynchronous operation.
    ///
    /// The factory is retained and invoked at most once for each request key.
    /// It must return a future whose output maps to the snapshot's data and
    /// error variants.
    pub fn future<F, Fut, T, E>(self, factory: F) -> AsyncBuilder<K, FutureFactory<F, T, E>, B>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        AsyncBuilder {
            request_key: self.request_key,
            future_factory: FutureFactory {
                factory: Rc::new(factory),
                marker: PhantomData,
            },
            snapshot_builder: self.snapshot_builder,
            widget_key: self.widget_key,
        }
    }
}

impl<K, F, T, E> AsyncBuilder<K, FutureFactory<F, T, E>, RequiredChild> {
    /// Supplies the final snapshot builder and makes the value a valid widget.
    ///
    /// The closure is called for waiting, successful, and failed snapshots and
    /// must return a type-erased child widget for every variant.
    pub fn child<B>(
        self,
        builder: B,
    ) -> AsyncBuilder<K, FutureFactory<F, T, E>, SnapshotBuilder<B, T, E>>
    where
        B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget,
    {
        AsyncBuilder {
            request_key: self.request_key,
            future_factory: self.future_factory,
            snapshot_builder: SnapshotBuilder {
                builder: Rc::new(builder),
                marker: PhantomData,
            },
            widget_key: self.widget_key,
        }
    }
}

struct AsyncRuntimeInner<T, E> {
    snapshot: AsyncSnapshot<T, E>,
    revision: u64,
    /// The scope owning the in-flight task, and therefore the only thing that
    /// has to be dropped to cancel it.
    ///
    /// `Some` from the moment the request is launched. That doubles as the
    /// "already started" flag the launcher reads, so there is one fact about
    /// whether a request is live rather than two that can disagree.
    scope: Option<TaskScope>,
}

/// The state one [`AsyncBuilder`] request is tracked through.
///
/// Held behind an [`Rc`] shared by the state object and the element it builds,
/// and reached from the running task through a [`Weak`] — the task must not keep
/// this alive, or an unmounted widget's request would outlive the tree that
/// asked for it.
struct AsyncRuntime<T, E> {
    inner: RefCell<AsyncRuntimeInner<T, E>>,
}

impl<T, E> AsyncRuntime<T, E> {
    fn new() -> Self {
        Self {
            inner: RefCell::new(AsyncRuntimeInner {
                snapshot: AsyncSnapshot::Waiting,
                revision: 0,
                scope: None,
            }),
        }
    }

    fn revision(&self) -> u64 {
        self.inner.borrow().revision
    }

    /// Abandons the current request and returns to the waiting state.
    ///
    /// Dropping the scope *is* the cancellation: the task and the future inside
    /// it are gone, so a reply from the old request cannot arrive late. This is
    /// what replaced a generation counter that existed only to recognise and
    /// discard such replies.
    fn reset(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.scope = None;
        inner.revision = inner.revision.wrapping_add(1);
        inner.snapshot = AsyncSnapshot::Waiting;
    }

    /// Claims the right to launch, returning the scope to spawn into.
    ///
    /// `None` when a request is already live, which is what keeps the factory
    /// invoked at most once per request key however many times the tree is
    /// rebuilt or redrawn.
    fn begin(&self, venus: &Venus) -> Option<ScopeId> {
        let mut inner = self.inner.borrow_mut();
        if inner.scope.is_some() {
            return None;
        }
        let scope = venus.scope();
        let id = scope.id();
        inner.scope = Some(scope);
        Some(id)
    }

    /// Records the result of the request that is currently live.
    fn complete(&self, result: Result<T, E>) {
        let mut inner = self.inner.borrow_mut();
        inner.snapshot = match result {
            Ok(data) => AsyncSnapshot::Data(data),
            Err(error) => AsyncSnapshot::Error(error),
        };
        inner.revision = inner.revision.wrapping_add(1);
        crate::components::element::advance_rebuild_invalidation_generation();
    }
}

#[doc(hidden)]
pub struct AsyncBuilderState<K, F, B, T, E> {
    request_key: K,
    future_factory: Rc<F>,
    snapshot_builder: Rc<B>,
    runtime: Rc<AsyncRuntime<T, E>>,
}

impl<K, F, Fut, B, T, E> StatefulWidget
    for AsyncBuilder<K, FutureFactory<F, T, E>, SnapshotBuilder<B, T, E>>
where
    K: Clone + Eq + 'static,
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Result<T, E>> + 'static,
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget + 'static,
    T: 'static,
    E: 'static,
{
    type State = AsyncBuilderState<K, F, B, T, E>;

    fn create_state(self) -> Self::State {
        AsyncBuilderState {
            request_key: self.request_key,
            future_factory: self.future_factory.factory,
            snapshot_builder: self.snapshot_builder.builder,
            runtime: Rc::new(AsyncRuntime::new()),
        }
    }
}

impl<K, F, Fut, B, T, E> State<AsyncBuilder<K, FutureFactory<F, T, E>, SnapshotBuilder<B, T, E>>>
    for AsyncBuilderState<K, F, B, T, E>
where
    K: Clone + Eq + 'static,
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Result<T, E>> + 'static,
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget + 'static,
    T: 'static,
    E: 'static,
{
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        self.future_factory = new.future_factory;
        self.snapshot_builder = new.snapshot_builder;
        if self.request_key != new.request_key {
            self.request_key = new.request_key;
            self.runtime.reset();
        }
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        AsyncFrame {
            future_factory: self.future_factory.clone(),
            snapshot_builder: self.snapshot_builder.clone(),
            runtime: self.runtime.clone(),
            marker: PhantomData::<fn() -> Fut>,
        }
    }
}

impl<K, F, Fut, B, T, E> Widget
    for AsyncBuilder<K, FutureFactory<F, T, E>, SnapshotBuilder<B, T, E>>
where
    K: Clone + Eq + 'static,
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Result<T, E>> + 'static,
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget + 'static,
    T: 'static,
    E: 'static,
{
    fn key(&self) -> Option<Key> {
        self.widget_key.clone()
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let key = Widget::key(&self);
        StatefulElement::new_with_name(self, ctx, "AsyncBuilder", key)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "AsyncBuilder"
    }
}

impl<K, F, Fut, B, T, E> PortableWidget
    for AsyncBuilder<K, FutureFactory<F, T, E>, SnapshotBuilder<B, T, E>>
where
    K: Clone + Eq + 'static,
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Result<T, E>> + 'static,
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget + 'static,
    T: 'static,
    E: 'static,
{
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        ctx: &mut crate::portable::PortableBuildContext,
        source: crate::portable::SourceFingerprint,
    ) -> Result<crate::portable::PortableNodeId, crate::portable::PortableBuildError> {
        let widget_key = Widget::key(&self);
        let request_key = self.request_key;
        let future_factory = self.future_factory.factory;
        let snapshot_builder = self.snapshot_builder.builder;
        let slot = ctx.slot_for(widget_key.as_ref(), source);

        // AsyncBuilder is a structural widget in the portable tree: only its
        // current snapshot child crosses AWIR. The future and the typed
        // snapshot stay inside the generation-local guest runtime, just as
        // they do inside the native retained element.
        ctx.with_animation_state(
            slot,
            || {
                Rc::new(RefCell::new(PortableAsyncBuilderState {
                    request_key: request_key.clone(),
                    snapshot: AsyncSnapshot::Waiting,
                    task_id: None,
                }))
            },
            |shared_state, ctx| {
                let callback_id = source.identity();
                let mut state = shared_state.borrow_mut();
                if state.request_key != request_key {
                    if let Some(task_id) = state.task_id.take() {
                        ctx.cancel_async_task(task_id);
                    }
                    state.request_key = request_key.clone();
                    state.snapshot = AsyncSnapshot::Waiting;
                }

                if state.task_id.is_none()
                    && matches!(state.snapshot, AsyncSnapshot::Waiting)
                {
                    let future = (future_factory)();
                    let task_state = Rc::clone(shared_state);
                    let task = Box::pin(async move {
                        let result = future.await;
                        let mut state = task_state.borrow_mut();
                        state.snapshot = match result {
                            Ok(data) => AsyncSnapshot::Data(data),
                            Err(error) => AsyncSnapshot::Error(error),
                        };
                        state.task_id = None;
                    });
                    state.task_id = Some(
                        ctx.start_async_task(
                            callback_id,
                            crate::portable::__anteros::AsyncCallbackSchemaMetadata::new(
                                crate::portable::__anteros::Version::new(1, 0),
                                1,
                                0,
                            ),
                            task,
                        )
                        .map_err(|error| {
                            crate::portable::PortableBuildError::Callback(
                                crate::portable::PortableCallbackError::CallbackFailed {
                                    id: callback_id,
                                    message: error.to_string(),
                                },
                            )
                        })?,
                    );
                }

                let child = (snapshot_builder)(&state.snapshot);
                drop(state);
                child.into_portable_node(ctx, source.child(0))
            },
        )
    }
}

#[cfg(feature = "portable-guest")]
struct PortableAsyncBuilderState<K, T, E> {
    request_key: K,
    snapshot: AsyncSnapshot<T, E>,
    task_id: Option<crate::portable::PortableTaskId>,
}

struct AsyncFrame<F, Fut, B, T, E> {
    future_factory: Rc<F>,
    snapshot_builder: Rc<B>,
    runtime: Rc<AsyncRuntime<T, E>>,
    marker: PhantomData<fn() -> Fut>,
}

impl<F, Fut, B, T, E> AsyncFrame<F, Fut, B, T, E>
where
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget,
{
    fn child_element(&self, ctx: &BuildContext) -> AnyElement {
        let inner = self.runtime.inner.borrow();
        (self.snapshot_builder)(&inner.snapshot).into_element(ctx)
    }
}

impl<F, Fut, B, T, E> Widget for AsyncFrame<F, Fut, B, T, E>
where
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Result<T, E>> + 'static,
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget + 'static,
    T: 'static,
    E: 'static,
{
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        AsyncFrameElement {
            child: SyncChild(UnsafeCell::new(self.child_element(ctx))),
            rendered_revision: Cell::new(self.runtime.revision()),
            future_factory: self.future_factory,
            snapshot_builder: self.snapshot_builder,
            runtime: self.runtime,
            marker: PhantomData::<fn() -> Fut>,
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "AsyncFrame"
    }
}

impl<F, Fut, B, T, E> PortableWidget for AsyncFrame<F, Fut, B, T, E>
where
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Result<T, E>> + 'static,
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget + 'static,
    T: 'static,
    E: 'static,
{
}

struct AsyncFrameElement<F, Fut, B, T, E> {
    child: SyncChild,
    future_factory: Rc<F>,
    snapshot_builder: Rc<B>,
    runtime: Rc<AsyncRuntime<T, E>>,
    rendered_revision: Cell<u64>,
    marker: PhantomData<fn() -> Fut>,
}

impl<F, Fut, B, T, E> AsyncFrameElement<F, Fut, B, T, E> {
    fn current_child(&self) -> &dyn Element {
        // Safety: Aimer's rendering pipeline is single-threaded. Child replacement
        // happens only while processing this element on that render thread.
        unsafe { (&*self.child.0.get()).as_ref() }
    }

    fn replace_child(&self, child: AnyElement) {
        // Safety: see `current_child`; no child reference is retained across this
        // replacement.
        unsafe {
            *self.child.0.get() = child;
        }
    }
}

impl<F, Fut, B, T, E> AsyncFrameElement<F, Fut, B, T, E>
where
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget,
{
    fn update_child(&self, ctx: &BuildContext) {
        if self.rendered_revision.get() == self.runtime.revision() {
            return;
        }
        let new_child = {
            let inner = self.runtime.inner.borrow();
            (self.snapshot_builder)(&inner.snapshot).to_element(ctx)
        };
        carry_child_state(self.current_child(), new_child.as_ref(), ctx);
        crate::components::element::reconcile_generated_tree(
            self.current_child(),
            new_child.as_ref(),
        );
        self.replace_child(new_child);
        self.rendered_revision.set(self.runtime.revision());
    }
}

impl<F, Fut, B, T, E> AsyncFrameElement<F, Fut, B, T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>> + 'static,
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget,
    T: 'static,
    E: 'static,
{
    fn refresh(&self, ctx: &BuildContext) {
        self.launch_if_idle(ctx);
        self.update_child(ctx);
    }

    /// Starts the request, unless one is already in flight.
    ///
    /// The task is a microtask on the runtime that owns this thread's frames,
    /// so a future that is already resolved reports its result to *this*
    /// frame's build rather than the next one. It holds the request state only
    /// weakly: the element and its state own that, and a task outliving them is
    /// a task whose answer nobody is waiting for.
    ///
    /// Nothing is launched when no runtime is installed — a widget exercised in
    /// isolation simply stays in [`AsyncSnapshot::Waiting`] instead of taking the
    /// process down.
    fn launch_if_idle(&self, ctx: &BuildContext) {
        let Some(venus) = Venus::current() else {
            return;
        };
        let Some(scope) = self.runtime.begin(&venus) else {
            return;
        };

        let future = (self.future_factory)();
        let runtime = Rc::downgrade(&self.runtime);
        let window = ctx.window.clone();
        venus.spawn_in(scope, async move {
            let result = future.await;
            let Some(runtime) = Weak::upgrade(&runtime) else {
                return;
            };
            runtime.complete(result);
            window.request_animation_frame();
        });
    }
}

impl<F, Fut, B, T, E> VisitorElement for AsyncFrameElement<F, Fut, B, T, E> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.current_child());
    }

    fn debug_name(&self) -> &'static str {
        "AsyncFrame"
    }
}

impl<F, Fut, B, T, E> EventElement for AsyncFrameElement<F, Fut, B, T, E> {
}

impl<F, Fut, B, T, E> Rebuildable for AsyncFrameElement<F, Fut, B, T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>> + 'static,
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget,
    T: 'static,
    E: 'static,
{
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.refresh(ctx);
        self.current_child().rebuild_if_dirty(ctx);
    }

    fn mark_needs_rebuild(&self) {
        self.current_child().mark_needs_rebuild();
    }
}

impl<F, Fut, B, T, E> Drawable for AsyncFrameElement<F, Fut, B, T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>> + 'static,
    B: Fn(&AsyncSnapshot<T, E>) -> AnyWidget,
    T: 'static,
    E: 'static,
{
    fn update(&self, ctx: &BuildContext) {
        self.refresh(ctx);
        self.current_child().update(ctx);
    }

    /// The frame records nothing itself: its current child owns the render
    /// nodes and resolves its own paint source.
    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    /// Launches the request and adopts a completed one before the retained
    /// tree records, so the frame that observes a result renders it.
    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        self.refresh(ctx);
        true
    }
}

impl<F, Fut, B, T, E> LayoutElement for AsyncFrameElement<F, Fut, B, T, E> {
    fn pos(&self) -> Option<Vec2d> {
        self.current_child().pos()
    }

    fn size(&self) -> Option<Size> {
        self.current_child().size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.current_child().layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.current_child().computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.current_child().content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.current_child().layer()
    }

    fn flex(&self) -> Option<f32> {
        self.current_child().flex()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.current_child().get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.current_child().invalidate_layout();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.current_child().pos_start_end()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_attribute::ResolvedSize;
    use aimer_venus::Venus;

    use crate::base::{BuildContext, WindowHandle};
    use crate::{
        AnyElement, AnyWidget, AsyncBuilder, AsyncSnapshot, Drawable, Element, EventElement,
        LayoutCache, LayoutElement, Rebuildable, VisitorElement, Widget,
    };

    /// A leaf of a stated height, so a snapshot can be told apart by what it
    /// measures as well as by its name.
    struct MarkerWidget(&'static str, f32);

    impl Widget for MarkerWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            MarkerElement(self.0, self.1).boxed()
        }

        fn debug_name(&self) -> &'static str {
            self.0
        }
    }

    impl crate::widget::PortableWidget for MarkerWidget {}

    struct MarkerElement(&'static str, f32);

    impl VisitorElement for MarkerElement {
        fn debug_name(&self) -> &'static str {
            self.0
        }
    }

    impl Drawable for MarkerElement {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl EventElement for MarkerElement {}

    impl LayoutElement for MarkerElement {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: 0.0,
                height: self.1,
            }
        }
    }

    impl Rebuildable for MarkerElement {}

    /// An ancestor that memoizes what it measured, exactly like `Container`,
    /// `Row` and `Column` do.
    ///
    /// This is the shape a `Scrollable` reads its content extent through, so it
    /// is the shape that decides whether a completed request can be scrolled.
    struct CachingParent {
        child: AnyElement,
        cache: LayoutCache,
    }

    impl CachingParent {
        fn new(child: AnyElement) -> Self {
            Self {
                child,
                cache: LayoutCache::new(),
            }
        }
    }

    impl VisitorElement for CachingParent {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }

        fn debug_name(&self) -> &'static str {
            "CachingParent"
        }
    }

    impl Drawable for CachingParent {
        fn update(&self, ctx: &BuildContext) {
            self.child.update(ctx);
        }
    }

    impl EventElement for CachingParent {}

    impl LayoutElement for CachingParent {
        fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
            let scale_bits = ctx.scale.to_bits();
            if let Some(cached) = self.cache.get_content(ctx.box_constraint, scale_bits) {
                return cached;
            }
            let size = self.child.content_size(ctx);
            self.cache.set_content(ctx.box_constraint, scale_bits, size);
            size
        }
    }

    impl Rebuildable for CachingParent {
        fn rebuild_if_dirty(&self, ctx: &BuildContext) {
            self.child.rebuild_if_dirty(ctx);
        }
    }

    /// A runtime installed for this test's thread, and the context a frame is
    /// built with — the pair an event loop hands the tree.
    ///
    /// Every test runs on a thread of its own, so the installed runtime belongs
    /// to that test alone and nothing has to be torn down between them.
    fn context() -> (Rc<Venus>, BuildContext<'static>) {
        let venus = Venus::new();
        venus.install();

        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let ctx = BuildContext::new(
            canvas,
            ResolvedSize::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );

        (venus, ctx)
    }

    fn contains(element: &dyn Element, name: &'static str) -> bool {
        if element.debug_name() == name {
            return true;
        }
        let found = Rc::new(Cell::new(false));
        let found_in_child = found.clone();
        element.visit_children(&mut |child| {
            if contains(child, name) {
                found_in_child.set(true);
            }
        });
        found.get()
    }

    /// Returns the first element in the tree whose debug name is `name`.
    fn find<'a>(element: &'a dyn Element, name: &'static str) -> Option<&'a dyn Element> {
        if element.debug_name() == name {
            return Some(element);
        }
        let mut found = None;
        element.visit_children(&mut |child| {
            if found.is_none() {
                found = find(child, name);
            }
        });
        found
    }

    /// The frame records nothing itself: its current child owns the render
    /// nodes. Declining local paint would put the whole async subtree on the
    /// legacy path.
    #[tokio::test]
    async fn an_async_frame_paints_locally_and_settles_its_state_first() {
        let (venus, ctx) = context();
        let widget = AsyncBuilder::new()
            .future(|| async { Ok::<_, &'static str>(42_usize) })
            .child(marker);
        let element = widget.to_element(&ctx);
        let frame = find(element.as_ref(), "AsyncFrame").expect("the builder mounts a frame");

        assert!(frame.can_paint_local_v2(&ctx));
        assert!(frame.sync_local_v2_state(&ctx), "the request is launched while syncing");
        venus.run_microtasks();
        assert!(frame.sync_local_v2_state(&ctx));
        assert!(
            contains(frame, "Data"),
            "syncing adopts the completed request before paint is recorded"
        );
    }

    fn marker(snapshot: &AsyncSnapshot<usize, &'static str>) -> AnyWidget {
        match snapshot {
            AsyncSnapshot::Waiting => MarkerWidget("Waiting", WAITING_HEIGHT).boxed(),
            AsyncSnapshot::Data(_) => MarkerWidget("Data", DATA_HEIGHT).boxed(),
            AsyncSnapshot::Error(_) => MarkerWidget("Error", WAITING_HEIGHT).boxed(),
        }
    }

    /// Height of the loading state: a spinner is short.
    const WAITING_HEIGHT: f32 = 40.0;

    /// Height of the loaded state: the archive it was waiting for is long.
    const DATA_HEIGHT: f32 = 4_000.0;

    /// A completed request grows the content, and every ancestor that already
    /// measured the loading state has to report the new extent.
    ///
    /// A `Scrollable` derives its scroll range from exactly this measurement, so
    /// an ancestor answering with the height of the spinner is a page that
    /// renders its data and refuses to scroll.
    #[tokio::test]
    async fn a_completed_request_grows_the_measurement_of_a_caching_ancestor() {
        let (venus, ctx) = context();
        let widget = AsyncBuilder::new()
            .future(|| async { Ok::<_, &'static str>(42_usize) })
            .child(marker);
        let parent = CachingParent::new(widget.to_element(&ctx));

        parent.rebuild_if_dirty(&ctx);
        assert_eq!(parent.content_size(&ctx).height, WAITING_HEIGHT);

        venus.run_microtasks();
        parent.rebuild_if_dirty(&ctx);

        assert_eq!(parent.content_size(&ctx).height, DATA_HEIGHT);
    }

    /// The property the migration to Venus was for: a request that has already
    /// resolved is drained *before* the build phase, so the frame that observes
    /// the completion is the frame that renders it — not the one after it.
    #[tokio::test]
    async fn a_resolved_request_reaches_the_build_of_the_same_frame() {
        let (venus, ctx) = context();
        let widget = AsyncBuilder::new()
            .future(|| async { Ok::<_, &'static str>(42_usize) })
            .child(marker);
        let element = widget.to_element(&ctx);

        // The launch happens inside a build, so the earliest a result can be
        // seen is the next frame — and it must be seen by that frame's build.
        element.rebuild_if_dirty(&ctx);

        let rendered_data = venus.drive_frame(|| {
            element.rebuild_if_dirty(&ctx);
            contains(element.as_ref(), "Data")
        });

        assert!(rendered_data);
    }

    /// A future may hold an [`Rc`] taken from the element tree.
    ///
    /// Under the Tokio path this did not compile: the future had to be `Send`,
    /// which ruled out every handle the tree actually hands out.
    #[tokio::test]
    async fn a_future_may_capture_a_handle_from_the_element_tree() {
        let (venus, ctx) = context();
        let shared = Rc::new(Cell::new(41_usize));
        let widget = AsyncBuilder::new()
            .future(move || {
                let shared = shared.clone();
                async move {
                    aimer_venus::yield_now().await;
                    Ok::<_, &'static str>(shared.get() + 1)
                }
            })
            .child(marker);
        let element = widget.to_element(&ctx);

        element.rebuild_if_dirty(&ctx);
        while venus.task_count() > 0 {
            venus.run_microtasks();
        }
        element.rebuild_if_dirty(&ctx);

        assert!(contains(element.as_ref(), "Data"));
    }

    #[tokio::test]
    async fn launches_once_and_rebuilds_from_waiting_to_data_after_redraw() {
        let (venus, ctx) = context();
        let launches = Rc::new(Cell::new(0));
        let factory_launches = launches.clone();
        let widget = AsyncBuilder::new()
            .request_key(7_u64)
            .future(move || {
                factory_launches.set(factory_launches.get() + 1);
                async { Ok::<_, &'static str>(42_usize) }
            })
            .child(marker);
        let element = widget.to_element(&ctx);

        assert!(contains(element.as_ref(), "Waiting"));
        element.rebuild_if_dirty(&ctx);
        assert_eq!(launches.get(), 1);

        venus.run_microtasks();
        assert!(ctx.window.take_redraw_request());
        element.rebuild_if_dirty(&ctx);

        assert!(contains(element.as_ref(), "Data"));
        assert_eq!(launches.get(), 1);
    }

    #[tokio::test]
    async fn renders_typed_errors_with_a_different_widget_type() {
        let (venus, ctx) = context();
        let widget = AsyncBuilder::new()
            .future(|| async { Err::<usize, _>("failed") })
            .child(marker);
        let element = widget.to_element(&ctx);

        element.rebuild_if_dirty(&ctx);
        venus.run_microtasks();
        element.rebuild_if_dirty(&ctx);

        assert!(contains(element.as_ref(), "Error"));
    }

    #[tokio::test]
    async fn unchanged_request_identity_does_not_launch_again_during_reconciliation() {
        let (venus, ctx) = context();
        let launches = Rc::new(Cell::new(0));
        let make_widget = || {
            let launches = launches.clone();
            AsyncBuilder::new()
                .request_key("same")
                .future(move || {
                    launches.set(launches.get() + 1);
                    async { Ok::<_, &'static str>(1_usize) }
                })
                .child(marker)
        };
        let old = make_widget().to_element(&ctx);
        old.rebuild_if_dirty(&ctx);
        venus.run_microtasks();
        old.rebuild_if_dirty(&ctx);
        let new = make_widget().to_element(&ctx);

        crate::widget::stateful::carry_child_state(old.as_ref(), new.as_ref(), &ctx);

        assert_eq!(launches.get(), 1);
    }

    #[tokio::test]
    async fn changed_request_identity_restarts_once() {
        let (venus, ctx) = context();
        let launches = Rc::new(Cell::new(0));
        let make_widget = |request_key| {
            let launches = launches.clone();
            AsyncBuilder::new()
                .request_key(request_key)
                .future(move || {
                    launches.set(launches.get() + 1);
                    async { Ok::<_, &'static str>(1_usize) }
                })
                .child(marker)
        };
        let old = make_widget(1_u64).to_element(&ctx);
        old.rebuild_if_dirty(&ctx);
        venus.run_microtasks();
        old.rebuild_if_dirty(&ctx);
        let new = make_widget(2_u64).to_element(&ctx);

        crate::widget::stateful::carry_child_state(old.as_ref(), new.as_ref(), &ctx);
        assert!(contains(new.as_ref(), "Waiting"));
        new.rebuild_if_dirty(&ctx);

        assert_eq!(launches.get(), 2);
    }

    /// Abandoning a request destroys the task that would have answered it, so
    /// there is no late reply left to recognise and discard.
    ///
    /// This is what a generation counter used to be for: the old design let a
    /// cancelled request run to completion and filtered its answer out on
    /// arrival. Cancelling the scope removes the question instead.
    #[test]
    fn a_reset_leaves_no_task_that_could_answer_late() {
        let venus = Venus::new();
        let runtime = Rc::new(super::AsyncRuntime::<usize, &'static str>::new());
        let scope = runtime.begin(&venus).expect("an idle request may launch");
        let answering = Rc::downgrade(&runtime);
        venus.spawn_in(scope, async move {
            aimer_venus::yield_now().await;
            if let Some(runtime) = answering.upgrade() {
                runtime.complete(Ok(1));
            }
        });
        assert_eq!(venus.task_count(), 1);

        runtime.reset();
        venus.run_microtasks();

        assert_eq!(venus.task_count(), 0, "the request itself is gone");
        assert!(matches!(
            runtime.inner.borrow().snapshot,
            AsyncSnapshot::Waiting
        ));
    }

    /// An unmounted widget's request is dropped with it, which is the only
    /// thing keeping a scrolled-away page from finishing work nobody wants.
    #[tokio::test]
    async fn dropping_the_element_cancels_a_pending_future() {
        struct DropGuard(Rc<Cell<usize>>);
        impl Drop for DropGuard {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let (venus, ctx) = context();
        let drops = Rc::new(Cell::new(0));
        let future_drops = drops.clone();
        let widget = AsyncBuilder::new()
            .future(move || {
                let guard = DropGuard(future_drops.clone());
                async move {
                    let _guard = guard;
                    std::future::pending::<Result<usize, &'static str>>().await
                }
            })
            .child(marker);
        let element = widget.to_element(&ctx);

        element.rebuild_if_dirty(&ctx);
        venus.run_microtasks();
        assert_eq!(drops.get(), 0, "a pending request is still wanted");

        drop(element);

        assert_eq!(drops.get(), 1);
        assert_eq!(venus.task_count(), 0);
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_anteros::{
        PROPERTY_TEXT_CONTENT, Version, WIDGET_TEXT, WidgetDocumentView, WidgetProperty,
    };

    use super::{AsyncBuilder, AsyncSnapshot};
    use crate::base::BuildContext;
    use crate::portable::{
        PortableBuildContext, PortableBuildError, PortableLimits, PortableNodeId,
        PortableWidgetLimits, SourceFingerprint, StableId128,
    };
    use crate::{AnyElement, AnyWidget, PortableWidget, Widget};

    struct PortableText(&'static str);

    impl Widget for PortableText {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            panic!("portable async test must not enter native element construction")
        }
    }

    impl PortableWidget for PortableText {
        fn to_portable_node(
            self,
            ctx: &mut PortableBuildContext,
            source: SourceFingerprint,
        ) -> Result<PortableNodeId, PortableBuildError> {
            let content = ctx.push_string(self.0)?;
            ctx.push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source,
                &[WidgetProperty::new(PROPERTY_TEXT_CONTENT, content)],
                &[],
            )
        }
    }

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(
            7,
            0,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap()
    }

    fn source() -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_bytes([0xAC; 16]))
    }

    #[test]
    fn portable_async_builder_rebuilds_its_snapshot_without_restarting_the_request() {
        let launches = Rc::new(Cell::new(0));
        let make_widget = || {
            let launches = Rc::clone(&launches);
            AsyncBuilder::new()
                .future(move || {
                    launches.set(launches.get() + 1);
                    async { Ok::<_, &'static str>("ready") }
                })
                .child(|snapshot| {
                    let label = match snapshot {
                        AsyncSnapshot::Waiting => "waiting",
                        AsyncSnapshot::Data(value) => value,
                        AsyncSnapshot::Error(_) => "error",
                    };
                    AnyWidget::erase(PortableText(label))
                })
        };

        let mut context = context();
        let first = make_widget()
            .to_portable_node(&mut context, source())
            .unwrap();
        context.finish_document(first).unwrap();
        assert_eq!(launches.get(), 1);
        assert_eq!(context.async_task_count(), 1);

        context.run_async_microtasks();
        assert!(context.take_rebuild_request());
        assert_eq!(context.async_task_count(), 0);

        let second = make_widget()
            .to_portable_node(&mut context, source())
            .unwrap();
        let document = context.finish_document(second).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        assert_eq!(view.string(0), Some("ready"));
        assert_eq!(launches.get(), 1);
    }
}
