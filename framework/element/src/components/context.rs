use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_canvas::{CanvasContext, FrameCanvas};
use aimer_color::prelude::Color;
use aimer_cupid::draw_cmd::CurrentBuildContext;
#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::Handle;

use crate::CALLED;
use crate::components::element::DirtySource;

mod current;
mod inherited_scope;
mod retain;
mod type_id_hasher;
mod window_handle;

pub use current::{ActiveBuildContext, NoActiveBuildContext};
pub use window_handle::{HeadlessWindowState, WindowHandle};

pub use inherited_scope::InheritedScope;
pub use type_id_hasher::{InheritedStates, TypeIdHasher};

/// An owned copy of a [`BuildContext`] taken by [`BuildContext::snapshot`].
///
/// It holds everything a context carries except the borrowed frame canvas, so
/// it can outlive the frame it was taken in. [`Self::restore`] binds it to a
/// later frame's canvas. A retained element that is revisited alone, without
/// its parents, uses it to see the same constraints, scale, window and
/// inherited state its parents gave it the last time the whole tree was walked.
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
#[derive(Clone)]
pub struct BuildContextSnapshot {
    parent_size: ResolvedSize,
    scale: f32,
    parent_pos: Vec2d,
    cursor_pos: Vec2d,
    box_constraint: BoxConstraint,
    visible_rect: Option<(f32, f32, f32, f32)>,
    paint_background: Color,
    window: WindowHandle,
    async_handle: Handle,
    inherited_states: InheritedStates,
}

#[cfg(not(target_arch = "wasm32"))]
impl BuildContextSnapshot {
    /// Rebuilds a context that draws into `canvas`.
    ///
    /// The restored context owns a private inherited-state map, so scoped
    /// changes made while it is in use cannot reach any other context.
    #[doc(hidden)]
    pub fn restore<'a>(&self, canvas: FrameCanvas<'a>) -> BuildContext<'a> {
        BuildContext {
            parent_size: self.parent_size,
            canvas,
            scale: self.scale,
            parent_pos: self.parent_pos,
            cursor_pos: self.cursor_pos,
            box_constraint: self.box_constraint,
            visible_rect: self.visible_rect,
            paint_background: self.paint_background,
            window: self.window.clone(),
            async_handle: self.async_handle.clone(),
            inherited_states: Rc::new(RefCell::new(self.inherited_states.clone())),
        }
    }
}

/// What an element is given while it builds, lays out and updates: its
/// constraints, scale, canvas and the state inherited from its ancestors.
///
/// The fields a widget lays itself out with are public: [`parent_size`],
/// [`scale`], [`box_constraint`] and [`canvas`]. The rest describe where the
/// frame is and which window it belongs to, or are the framework's own
/// plumbing, so they are read through methods and changed through setters: the
/// framework can then keep them consistent with each other.
///
/// The frame state is read through methods:
///
/// ```
/// # fn check(ctx: &aimer_element::base::BuildContext<'_>) {
/// let _ = (ctx.parent_pos(), ctx.cursor_pos(), ctx.visible_rect(), ctx.window());
/// # }
/// ```
///
/// The fields behind them are private, so reaching for one is an error:
///
/// ```compile_fail,E0616
/// # fn check(ctx: &aimer_element::base::BuildContext<'_>) {
/// let _ = &ctx.inherited_states;
/// # }
/// ```
///
/// [`parent_size`]: BuildContext::parent_size
/// [`scale`]: BuildContext::scale
/// [`box_constraint`]: BuildContext::box_constraint
/// [`canvas`]: BuildContext::canvas
#[derive(Clone)]
pub struct BuildContext<'a> {
    pub parent_size: ResolvedSize,
    pub canvas: FrameCanvas<'a>,
    pub scale: f32,
    parent_pos: Vec2d,
    cursor_pos: Vec2d,
    pub box_constraint: BoxConstraint,
    visible_rect: Option<(f32, f32, f32, f32)>, // (x, y, width, height)
    /// Declared solid surface behind this element; transparent means unknown.
    paint_background: Color,
    window: WindowHandle,
    #[cfg(not(target_arch = "wasm32"))]
    async_handle: Handle,
    inherited_states: Rc<RefCell<InheritedStates>>,
}


// impl Clone for BuildContext<'_>  {
//     fn clone(&self) -> Self {

//         CALLED.increase();
//         Self {
//             inherited_states: Rc::clone(&self.inherited_states),
//             async_handle: self.async_handle.clone(),
//             window: self.window.clone(),
//             canvas: self.canvas.clone(),
//             ..*self
//         }
//     }
// }

impl<'a> CanvasContext for BuildContext<'a> {
    #[inline]
    fn paint_background(&self) -> Color { self.paint_background }

    fn current_canvas_context(&self) -> CurrentBuildContext {
        self.local_paint()
            .unwrap_or_else(|| panic!("Canvas::of(ctx) requires an active local-v2 paint callback"))
    }
}

#[doc(hidden)]
pub struct BuildConsumer {
    dirty_source: Rc<DirtySource>,
    cleanups: RefCell<Vec<Box<dyn FnOnce()>>>,
    dependencies: RefCell<HashSet<usize>>,
    /// Parts of this scope's output kept between its builds; see
    /// [`BuildContext::retain`].
    retained: RefCell<retain::Slots>,
}

impl BuildConsumer {
    pub fn new(dirty: Rc<Cell<bool>>) -> Rc<Self> {
        Rc::new(Self {
            dirty_source: DirtySource::new(dirty.clone()),
            cleanups: RefCell::new(Vec::new()),
            dependencies: RefCell::new(HashSet::new()),
            retained: RefCell::new(retain::Slots::default()),
        })
    }

    #[doc(hidden)]
    pub fn dirty_source(&self) -> Rc<DirtySource> {
        self.dirty_source.clone()
    }

    pub(crate) fn begin_build(&self) {
        self.dirty_source.begin_build();
        self.retained.borrow_mut().begin_build();
        self.dependencies.borrow_mut().clear();
        for cleanup in self.cleanups.borrow_mut().drain(..) {
            cleanup();
        }
    }

    pub fn add_cleanup(&self, cleanup: impl FnOnce() + 'static) {
        self.cleanups.borrow_mut().push(Box::new(cleanup));
    }

    pub fn register_dependency(&self, identity: usize) -> bool {
        self.dependencies.borrow_mut().insert(identity)
    }

    pub fn mark_needs_rebuild(&self) {
        self.dirty_source.mark();
    }
}

impl Drop for BuildConsumer {
    fn drop(&mut self) {
        for cleanup in self.cleanups.get_mut().drain(..) {
            cleanup();
        }
    }
}

#[derive(Clone)]
struct CurrentBuildConsumer(Rc<BuildConsumer>);

/// A retained render node that can report where it currently is.
///
/// It stays valid after the frame that produced it, so a registry can hold one
/// and answer "where is this element now?" when a navigation starts, however
/// the page has scrolled since.
#[derive(Clone)]
pub struct RetainedNodeHandle(CurrentBuildContext);

impl RetainedNodeHandle {
    /// Returns the node's `(x, y, width, height)` in logical pixels from the
    /// window's top-left corner, or `None` once the node has left the tree.
    pub fn world_bounds(&self) -> Option<(f32, f32, f32, f32)> {
        self.0
            .world_bounds()
            .map(|rect| (rect.x, rect.y, rect.width, rect.height))
    }
}

/// Puts the previous local paint scope back when the one entered ends, however
/// it ends.
struct LocalPaintGuard<'a> {
    states: &'a RefCell<InheritedStates>,
    previous: Option<CurrentBuildContext>,
}

impl Drop for LocalPaintGuard<'_> {
    fn drop(&mut self) {
        self.states
            .borrow_mut()
            .replace_local_paint(self.previous.take());
    }
}

struct StateScopeGuard {
    states: Rc<RefCell<InheritedStates>>,
    type_id: TypeId,
    previous: Option<Rc<dyn Any>>,
}

impl Drop for StateScopeGuard {
    fn drop(&mut self) {
        let mut states = self.states.borrow_mut();
        if let Some(previous) = self.previous.take() {
            states.insert(self.type_id, previous);
        } else {
            states.remove(&self.type_id);
        }
    }
}

impl<'a> std::fmt::Debug for BuildContext<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildContext")
            .field("parent_size", &self.parent_size)
            .field("scale", &self.scale)
            .field("parent_pos", &self.parent_pos)
            .field("cursor_pos", &self.cursor_pos)
            .field("box_constraint", &self.box_constraint)
            .finish()
    }
}

impl<'a> BuildContext<'a> {
    /// The position of the parent's origin, in the frame's coordinates.
    #[inline]
    pub fn parent_pos(&self) -> Vec2d {
        self.parent_pos
    }

    /// Sets the position of the parent's origin.
    #[inline]
    pub fn set_parent_pos(&mut self, parent_pos: Vec2d) {
        self.parent_pos = parent_pos;
    }

    /// The pointer position, in the frame's coordinates.
    #[inline]
    pub fn cursor_pos(&self) -> Vec2d {
        self.cursor_pos
    }

    /// Sets the pointer position this context reports.
    #[inline]
    pub fn set_cursor_pos(&mut self, cursor_pos: Vec2d) {
        self.cursor_pos = cursor_pos;
    }

    /// The part of this context's local space that is on screen, as
    /// `(x, y, width, height)`, or `None` when the whole space may be visible.
    #[inline]
    pub fn visible_rect(&self) -> Option<(f32, f32, f32, f32)> {
        self.visible_rect
    }

    /// Sets the visible rectangle; see [`BuildContext::visible_rect`].
    #[inline]
    pub fn set_visible_rect(&mut self, visible_rect: Option<(f32, f32, f32, f32)>) {
        self.visible_rect = visible_rect;
    }

    /// The window this context draws into.
    #[inline]
    pub fn window(&self) -> &WindowHandle {
        &self.window
    }

    /// The declared solid surface behind this element; transparent means
    /// unknown.
    #[doc(hidden)]
    #[inline]
    pub fn paint_background(&self) -> Color {
        self.paint_background
    }

    /// Declares the solid surface behind this element.
    #[doc(hidden)]
    #[inline]
    pub fn set_paint_background(&mut self, paint_background: Color) {
        self.paint_background = paint_background;
    }

    /// The handle of the async runtime that owns this window's tasks.
    #[cfg(not(target_arch = "wasm32"))]
    #[doc(hidden)]
    #[inline]
    pub fn async_handle(&self) -> &Handle {
        &self.async_handle
    }

    /// The state this context inherits from its ancestors. Framework plumbing:
    /// widgets use [`BuildContext::insert_state`] and [`BuildContext::get_state`].
    #[doc(hidden)]
    #[inline]
    pub fn inherited_states(&self) -> &Rc<RefCell<InheritedStates>> {
        &self.inherited_states
    }
}

impl<'a> BuildContext<'a> {
    pub fn new(
        canvas: FrameCanvas<'a>,
        size: ResolvedSize,
        scale: f32,
        parent_pos: Vec2d,
        cursor_pos: Vec2d,
        window: WindowHandle,
        #[cfg(not(target_arch = "wasm32"))] async_handle: Handle,
    ) -> Self {
        Self {
            canvas,
            parent_size: size,
            scale,
            parent_pos,
            cursor_pos,
            box_constraint: BoxConstraint::default(),
            visible_rect: None,
            paint_background: Color::Transparent,
            window,
            #[cfg(not(target_arch = "wasm32"))]
            async_handle,
            inherited_states: Rc::new(RefCell::new(InheritedStates::default())),
        }
    }

    /// Returns the UI allocator active while this build context is in use.
    ///
    /// Aimer activates the owning application's allocator around widget
    /// construction and rebuilds. The returned handle can also be used for
    /// explicit `allocator-api2` collections or `boxed_in` conversions.
    #[inline]
    pub fn ui_allocator(&self) -> Option<aimer_rubick::UiAllocator> {
        aimer_rubick::UiAllocator::current()
    }

    /// Replaces the canvas used by this build context.
    ///
    /// This keeps callers that fork a recording context independent of how the
    /// canvas is stored.
    #[inline]
    pub fn replace_canvas(&mut self, canvas: FrameCanvas<'a>) {
        {
            self.canvas = canvas;
        }
    }

    /// Returns whether a known rectangle in this context can overlap the
    /// current viewport.
    ///
    /// The rectangle and [`BuildContext::visible_rect`] use the same local
    /// coordinates. A missing viewport, an invalid rectangle, or invalid
    /// viewport dimensions is treated conservatively as visible so unknown
    /// bounds are never under-culled. Rectangles that only touch at an edge
    /// are also visible, matching the canvas clipping convention.
    #[inline]
    pub fn is_rect_visible(&self, x: f32, y: f32, width: f32, height: f32) -> bool {
        let Some((vx, vy, vw, vh)) = self.visible_rect else {
            return true;
        };
        if !x.is_finite()
            || !y.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || width < 0.0
            || height < 0.0
            || !vx.is_finite()
            || !vy.is_finite()
            || !vw.is_finite()
            || !vh.is_finite()
            || vw < 0.0
            || vh < 0.0
        {
            return true;
        }

        x + width >= vx && x <= vx + vw && y + height >= vy && y <= vy + vh
    }

    pub fn insert_state<T: Any>(&self, state: T) {
        self.inherited_states
            .borrow_mut()
            .insert(TypeId::of::<T>(), Rc::new(state));
    }

    /// Returns the nearest inherited `T`, if one is in scope.
    ///
    /// Inside a build this records that the widget building observed inherited
    /// context, which is what makes it rebuild when a parent places it again.
    /// A value that announces its own changes (a provider read through
    /// `watch` or `select`) does not need that and uses
    /// [`BuildContext::get_state_untracked`].
    pub fn get_state<T: Any>(&self) -> Option<Rc<T>> {
        if let Some(consumer) = self.current_build_consumer() {
            consumer.dirty_source.note_context_read();
        }
        self.get_state_untracked::<T>()
    }

    /// Returns the nearest inherited `T` without recording a context read.
    ///
    /// Only for a caller that subscribes to the value's changes some other way:
    /// skipping the record means a parent's rebuild will not rebuild the widget
    /// building, so nothing but that subscription would.
    #[doc(hidden)]
    pub fn get_state_untracked<T: Any>(&self) -> Option<Rc<T>> {
        self.inherited_states
            .borrow()
            .get(&TypeId::of::<T>())
            .and_then(|arc| arc.clone().downcast::<T>().ok())
    }

    pub fn with_state<T: Any, R>(&self, state: T, callback: impl FnOnce(&Self) -> R) -> R {
        let type_id = TypeId::of::<T>();
        let previous = self
            .inherited_states
            .borrow_mut()
            .insert(type_id, Rc::new(state));
        let _guard = StateScopeGuard {
            states: self.inherited_states.clone(),
            type_id,
            previous,
        };
        callback(self)
    }

    /// Creates a child context with a snapshot of inherited state and `state`
    /// installed for its full lifetime.
    ///
    /// Unlike [`BuildContext::with_state`], the new value remains available
    /// after this method returns. The copied map is isolated from later scoped
    /// changes to either context, while its stored values continue to share
    /// their own `Rc`-backed state.
    #[doc(hidden)]
    pub fn fork_with_state<T: Any>(&self, state: T) -> Self {
        let mut context = self.clone();
        let mut inherited_states = self.inherited_states.borrow().clone();
        inherited_states.insert(TypeId::of::<T>(), Rc::new(state));
        context.inherited_states = Rc::new(RefCell::new(inherited_states));
        context
    }

    /// Replaces the inherited-state map this context shares with its
    /// descendants. A window opened to share its opener's providers installs
    /// the opener's map here, so scoped changes are made on one map.
    #[doc(hidden)]
    #[inline]
    pub fn set_inherited_states(&mut self, states: Rc<RefCell<InheritedStates>>) {
        self.inherited_states = states;
    }

    /// Returns a copy of this context whose inherited state is a private copy
    /// of the map, so scoped changes made through it cannot reach this context
    /// or any other. The stored values still share their own `Rc` state.
    #[doc(hidden)]
    pub fn with_private_inherited_states(&self) -> Self {
        let mut context = self.clone();
        context.inherited_states = Rc::new(RefCell::new(self.inherited_states.borrow().clone()));
        context
    }

    /// Copies this context into a value that does not borrow the frame canvas.
    ///
    /// The inherited-state map is copied rather than shared: [`Self::with_state`]
    /// installs and removes entries on the shared map as a traversal enters and
    /// leaves a scope, so a handle to that map only describes the scope it was
    /// taken in while that scope is still open. The stored values keep sharing
    /// their own `Rc` state, so a provider handle still reaches live data.
    #[cfg(not(target_arch = "wasm32"))]
    #[doc(hidden)]
    pub fn snapshot(&self) -> BuildContextSnapshot {
        BuildContextSnapshot {
            parent_size: self.parent_size,
            scale: self.scale,
            parent_pos: self.parent_pos,
            cursor_pos: self.cursor_pos,
            box_constraint: self.box_constraint,
            visible_rect: self.visible_rect,
            paint_background: self.paint_background,
            window: self.window.clone(),
            async_handle: self.async_handle.clone(),
            inherited_states: self.inherited_states.borrow().clone(),
        }
    }

    /// Provides the retained element context to `Canvas::of` for one local
    /// paint callback.
    #[doc(hidden)]
    pub fn with_local_v2_paint_context<R>(
        &self,
        context: CurrentBuildContext,
        callback: impl FnOnce(&Self) -> R,
    ) -> R {
        let previous = self
            .inherited_states
            .borrow_mut()
            .replace_local_paint(Some(context));
        let _guard = LocalPaintGuard {
            states: &self.inherited_states,
            previous,
        };
        callback(self)
    }

    /// The render node of the local paint scope that is open, if any.
    ///
    /// Reading it counts as observing inherited context, as every read of an
    /// inherited state does while a widget is building.
    fn local_paint(&self) -> Option<CurrentBuildContext> {
        if let Some(consumer) = self.current_build_consumer() {
            consumer.dirty_source.note_context_read();
        }
        self.inherited_states.borrow().local_paint().cloned()
    }

    /// Updates one direct retained child's transform and opacity during local
    /// v2 state synchronization.
    #[doc(hidden)]
    pub fn set_local_v2_child_presentation_at(
        &self,
        child_index: usize,
        transform: aimer_cupid::utilities::Mat3,
        opacity: f32,
    ) -> bool {
        self.local_paint()
            .is_some_and(|context| context.set_child_presentation_at(child_index, transform, opacity))
    }

    /// Updates one direct retained child's own clip and corner radii during
    /// local v2 state synchronization.
    ///
    /// `clip` is `(x, y, width, height)` local to the child, so it scales with
    /// the child's presentation transform. Unchanged values record no damage.
    #[doc(hidden)]
    pub fn set_local_v2_child_clip_at(
        &self,
        child_index: usize,
        clip: Option<(f32, f32, f32, f32)>,
        radius: [f32; 4],
    ) -> bool {
        let clip = clip.map(|(x, y, width, height)| aimer_cupid::draw_cmd::Rect::new(x, y, width, height));
        self.local_paint()
            .is_some_and(|context| context.set_child_clip_at(child_index, clip, radius))
    }

    /// Returns a handle to the render node being synchronized, from which its
    /// on-screen bounds can be read later.
    ///
    /// Only available during local v2 state synchronization of an element that
    /// paints locally.
    #[doc(hidden)]
    pub fn local_v2_node_handle(&self) -> Option<RetainedNodeHandle> {
        self.local_paint().map(RetainedNodeHandle)
    }

    /// Allows the current render node to escape all ancestor widget clips.
    /// Descendant-owned clips remain local to those descendants.
    #[doc(hidden)]
    pub fn set_local_v2_clip_escape(&self, enabled: bool) -> bool {
        self.local_paint()
            .is_some_and(|context| context.set_clip_escape(enabled))
    }

    #[doc(hidden)]
    pub fn with_build_consumer<R>(
        &self,
        consumer: Rc<BuildConsumer>,
        callback: impl FnOnce(&Self) -> R,
    ) -> R {
        consumer.begin_build();
        let _scope = current::BuildScope::enter(self);
        let result = self.with_state(CurrentBuildConsumer(consumer.clone()), callback);
        let released = consumer.retained.borrow_mut().end_build();
        drop(released);
        result
    }

    #[doc(hidden)]
    pub fn current_build_consumer(&self) -> Option<Rc<BuildConsumer>> {
        self.get_state_untracked::<CurrentBuildConsumer>()
            .map(|consumer| consumer.0.clone())
    }

    /// Reads the window metrics and rebuilds the widget currently building
    /// whenever they change.
    ///
    /// This is the only way to observe the window from a `build` and stay
    /// correct: a resize rebuilds the widgets that registered here and nobody
    /// else, because rebuilding a tree that cannot have changed is what makes a
    /// window drag stutter. Reading [`BuildContext::window`] directly registers
    /// nothing, so a widget that lays itself out differently on a phone-sized
    /// window would keep the layout it was first built with.
    ///
    /// Outside a build — from layout, drawing or an event handler — there is
    /// nothing to register and the metrics are simply returned; such code reads
    /// them again on the next frame anyway.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// fn build(&self, ctx: &BuildContext) -> impl Widget {
    ///     let compact = ctx.watch_window_metrics().logical_size().width < 600.0;
    ///     // ... this widget is rebuilt when that answer can change
    /// }
    /// ```
    pub fn watch_window_metrics(&self) -> crate::WindowMetrics {
        if let Some(consumer) = self.current_build_consumer() {
            crate::window_metrics::subscribe(&consumer, &self.window);
        }
        crate::WindowMetrics::of(&self.window)
    }

    /// Reads the appearance the platform asks for and rebuilds the widget
    /// currently building whenever the user switches it.
    ///
    /// Desktop and mobile systems switch between light and dark appearance
    /// while an application is running, so a widget that chooses colors from
    /// the system appearance has to be told when the choice changes; reading
    /// [`crate::platform_brightness`] directly registers nothing and leaves the
    /// widget with the appearance it was first built with.
    ///
    /// Outside a build there is nothing to register and the appearance is
    /// simply returned.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// fn build(&self, ctx: &BuildContext) -> impl Widget {
    ///     let brightness = ctx.watch_platform_brightness();
    ///     // ... this widget is rebuilt when the system appearance changes
    /// }
    /// ```
    pub fn watch_platform_brightness(&self) -> crate::Brightness {
        if let Some(consumer) = self.current_build_consumer() {
            crate::platform_brightness::subscribe(&consumer);
        }
        crate::platform_brightness()
    }

    /// Reads the region the system reserves along the window edges and rebuilds
    /// the widget currently building whenever it changes.
    ///
    /// The status bar, the notch and the home indicator sit *over* the
    /// application's surface, and a rotation moves them; a widget that keeps
    /// something interactive out of that region has to be told when the region
    /// moves. Reading [`crate::safe_area_insets`] directly registers nothing.
    ///
    /// Outside a build there is nothing to register and the insets are simply
    /// returned.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// fn build(&self, ctx: &BuildContext) -> impl Widget {
    ///     let insets = ctx.watch_safe_area_insets();
    ///     // ... this widget is rebuilt when the status bar moves
    /// }
    /// ```
    pub fn watch_safe_area_insets(&self) -> crate::SafeAreaInsets {
        if let Some(consumer) = self.current_build_consumer() {
            crate::safe_area::subscribe_for(&consumer, &self.window);
        }
        crate::safe_area::safe_area_insets_for(&self.window)
    }

    /// Answers one question about the window and rebuilds the widget currently
    /// building only when that answer changes.
    ///
    /// Almost no widget depends on the window itself; it depends on something
    /// derived from it — a breakpoint, a column count — whose answer changes
    /// once in a whole drag, if at all. Registering the question instead of the
    /// metrics is the difference between rebuilding on every pixel and
    /// rebuilding when the layout genuinely differs.
    ///
    /// Each call registers separately, so a build may ask several independent
    /// questions.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// fn build(&self, ctx: &BuildContext) -> impl Widget {
    ///     // Rebuilt when the window crosses 600, and at no other width.
    ///     let compact = ctx.select_window_metrics(|window| {
    ///         window.logical_size().width < 600.0
    ///     });
    /// }
    /// ```
    pub fn select_window_metrics<T: Clone + PartialEq + 'static>(
        &self,
        selector: impl Fn(&crate::WindowMetrics) -> T + 'static,
    ) -> T {
        match self.current_build_consumer() {
            Some(consumer) => {
                crate::window_metrics::subscribe_selected(&consumer, &self.window, selector)
            }
            None => selector(&crate::WindowMetrics::of(&self.window)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;

    #[test]
    fn headless_redraw_requests_coalesce_until_the_next_display_tick() {
        let window = WindowHandle::headless(Default::default(), 1.0);

        window.request_redraw();
        window.request_redraw();
        window.request_redraw();

        assert_eq!(
            window.headless_redraw_request_counts(),
            Some((1, 2, 0))
        );
        assert!(window.take_redraw_request());
        assert_eq!(
            window.headless_redraw_request_counts(),
            Some((1, 2, 1))
        );

        window.request_redraw();
        assert_eq!(
            window.headless_redraw_request_counts(),
            Some((2, 2, 1))
        );
    }

    #[test]
    fn animation_frame_requests_are_recorded_for_headless_windows() {
        let window = WindowHandle::headless(Default::default(), 1.0);

        window.request_animation_frame();

        assert_eq!(
            window.headless_redraw_request_counts(),
            Some((1, 0, 0))
        );
        assert!(window.take_redraw_request());
    }

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

    fn context() -> BuildContext<'static> {
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

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn paint_background_survives_cloning_and_animation_context_snapshots() {
        let mut ctx = context();
        assert_eq!(ctx.paint_background, Color::Transparent);
        ctx.paint_background = Color::WHITE;
        assert_eq!(ctx.clone().paint_background, Color::WHITE);
        let snapshot = ctx.snapshot();
        ctx.paint_background = Color::BLACK;
        assert_eq!(snapshot.restore(ctx.canvas.clone()).paint_background, Color::WHITE);
    }

    /// A render node to hand to `with_local_v2_paint_context`, and its handle.
    fn paint_scope() -> (aimer_cupid::draw_cmd::RenderTree, [CurrentBuildContext; 2]) {
        use aimer_cupid::draw_cmd::{Rect, RenderTree};

        let tree = RenderTree::new();
        let first = tree.add_root(Rect::new(1.0, 2.0, 10.0, 10.0)).unwrap();
        let second = tree.add_root(Rect::new(30.0, 40.0, 10.0, 10.0)).unwrap();
        let contexts = [tree.context(first).unwrap(), tree.context(second).unwrap()];
        (tree, contexts)
    }

    fn node_origin(context: &BuildContext) -> Option<(f32, f32)> {
        context
            .local_v2_node_handle()
            .and_then(|handle| handle.world_bounds())
            .map(|(x, y, _, _)| (x, y))
    }

    #[test]
    fn the_local_paint_context_is_available_only_inside_its_scope() {
        let (_tree, [first, _]) = paint_scope();
        let context = context();
        assert_eq!(node_origin(&context), None);

        context.with_local_v2_paint_context(first, |context| {
            assert_eq!(node_origin(context), Some((1.0, 2.0)));
        });

        assert_eq!(node_origin(&context), None);
    }

    #[test]
    fn a_nested_local_paint_scope_restores_the_outer_one() {
        let (_tree, [first, second]) = paint_scope();
        let context = context();

        context.with_local_v2_paint_context(first, |context| {
            context.with_local_v2_paint_context(second, |context| {
                assert_eq!(node_origin(context), Some((30.0, 40.0)));
            });
            assert_eq!(node_origin(context), Some((1.0, 2.0)));
        });
    }

    #[test]
    fn the_local_paint_context_is_restored_after_a_panic() {
        let (_tree, [first, second]) = paint_scope();
        let context = context();

        context.with_local_v2_paint_context(first, |context| {
            let result = catch_unwind(AssertUnwindSafe(|| {
                context.with_local_v2_paint_context(second, |_| panic!("stop"));
            }));
            assert!(result.is_err());
            assert_eq!(node_origin(context), Some((1.0, 2.0)));
        });
        assert_eq!(node_origin(&context), None);
    }

    #[test]
    fn a_clone_shares_the_scope_a_snapshot_copies_it() {
        let (_tree, [first, second]) = paint_scope();
        let context = context();

        context.with_local_v2_paint_context(first, |context| {
            // A clone is the same scope: it sees what the original enters.
            let shared = context.clone();
            context.with_local_v2_paint_context(second.clone(), |_| {
                assert_eq!(node_origin(&shared), Some((30.0, 40.0)));
            });
            assert_eq!(node_origin(&shared), Some((1.0, 2.0)));

            // A forked context starts from the scope as it is and keeps it.
            let forked = context.fork_with_state(0_u8);
            assert_eq!(node_origin(&forked), Some((1.0, 2.0)));
            context.with_local_v2_paint_context(second, |_| {
                assert_eq!(node_origin(&forked), Some((1.0, 2.0)));
            });
        });
    }

    #[test]
    fn scoped_state_and_the_local_paint_context_do_not_disturb_each_other() {
        let (_tree, [first, _]) = paint_scope();
        let context = context();
        context.insert_state(1_u32);

        context.with_local_v2_paint_context(first, |context| {
            context.with_state(2_u32, |context| {
                assert_eq!(*context.get_state::<u32>().unwrap(), 2);
                assert_eq!(node_origin(context), Some((1.0, 2.0)));
            });
            assert_eq!(*context.get_state::<u32>().unwrap(), 1);
        });
    }

    /// Timing probe for `cargo nextest run -p aimer_element --run-ignored only --no-capture`.
    ///
    /// The retained element update enters this scope once per element per frame.
    #[test]
    #[ignore = "timing probe, prints results"]
    fn profile_entering_the_local_paint_scope() {
        let (_tree, [first, _]) = paint_scope();
        let context = context();
        const CALLS: u32 = 200_000;
        let mut best = f64::MAX;
        for _ in 0..7 {
            let start = std::time::Instant::now();
            for _ in 0..CALLS {
                context.with_local_v2_paint_context(first.clone(), |context| {
                    std::hint::black_box(context);
                });
            }
            best = best.min(start.elapsed().as_secs_f64() * 1e9 / f64::from(CALLS));
        }
        println!("with_local_v2_paint_context: {best:.1} ns per scope");
    }

    #[test]
    fn scoped_state_shadows_and_restores_the_outer_value() {
        let context = context();
        context.insert_state(1_u32);

        context.with_state(2_u32, |context| {
            assert_eq!(*context.get_state::<u32>().unwrap(), 2);
        });

        assert_eq!(*context.get_state::<u32>().unwrap(), 1);
    }

    #[test]
    fn scoped_state_is_restored_after_a_panic() {
        let context = context();
        context.insert_state(1_u32);

        let result = catch_unwind(AssertUnwindSafe(|| {
            context.with_state(2_u32, |_| panic!("stop"));
        }));

        assert!(result.is_err());
        assert_eq!(*context.get_state::<u32>().unwrap(), 1);
    }

    #[test]
    fn rect_visibility_is_conservative_and_edge_inclusive() {
        let mut context = context();
        assert!(context.is_rect_visible(10.0, 10.0, 10.0, 10.0));

        context.visible_rect = Some((0.0, 0.0, 100.0, 100.0));
        assert!(context.is_rect_visible(100.0, 20.0, 10.0, 10.0));
        assert!(!context.is_rect_visible(101.0, 20.0, 10.0, 10.0));
        assert!(!context.is_rect_visible(20.0, 101.0, 10.0, 10.0));

        assert!(context.is_rect_visible(f32::NAN, 0.0, 10.0, 10.0));
        context.visible_rect = Some((0.0, 0.0, f32::NAN, 100.0));
        assert!(context.is_rect_visible(200.0, 200.0, 10.0, 10.0));
    }

    #[test]
    fn build_consumer_cleans_previous_dependencies_before_rebuild() {
        let context = context();
        let dirty = Rc::new(Cell::new(false));
        let consumer = BuildConsumer::new(dirty.clone());
        let cleanup_count = Rc::new(Cell::new(0));

        context.with_build_consumer(consumer.clone(), |context| {
            let current = context.current_build_consumer().unwrap();
            let cleanup_count = cleanup_count.clone();
            current.add_cleanup(move || cleanup_count.set(cleanup_count.get() + 1));
        });
        assert_eq!(cleanup_count.get(), 0);

        context.with_build_consumer(consumer.clone(), |_| {});

        assert_eq!(cleanup_count.get(), 1);
        consumer.mark_needs_rebuild();
        assert!(dirty.get());
    }

    #[test]
    fn current_is_the_context_being_built() {
        let mut context = context();
        context.scale = 2.5;
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));

        context.with_build_consumer(consumer, |built| {
            let current = BuildContext::current();
            assert!(std::ptr::eq(current.get(), built));
            assert_eq!(current.get().scale, 2.5);
            assert!(current.is_live());
            // SAFETY: a build scope is active and `built` outlives this block.
            let unchecked = unsafe { BuildContext::unchecked_current() };
            assert!(std::ptr::eq(unchecked, built));
        });
    }

    #[test]
    fn there_is_no_current_context_outside_a_build() {
        assert_eq!(BuildContext::try_current().err(), Some(NoActiveBuildContext));

        let context = context();
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));
        context.with_build_consumer(consumer, |_| {});

        assert!(BuildContext::try_current().is_err());
    }

    #[test]
    #[should_panic(expected = "no widget build is running on this thread")]
    fn current_panics_outside_a_build() {
        let _ = BuildContext::current();
    }

    #[test]
    fn nested_builds_see_their_own_context_and_restore_the_outer_one() {
        let mut outer_context = context();
        outer_context.scale = 1.5;
        let mut inner_context = context();
        inner_context.scale = 3.0;
        let consumer = || BuildConsumer::new(Rc::new(Cell::new(false)));

        outer_context.with_build_consumer(consumer(), |_| {
            let outer = BuildContext::current();
            inner_context.with_build_consumer(consumer(), |_| {
                assert_eq!(BuildContext::current().get().scale, 3.0);
                // The outer guard is still valid while a nested build runs.
                assert!(outer.is_live());
                assert_eq!(outer.get().scale, 1.5);
            });
            assert_eq!(BuildContext::current().get().scale, 1.5);
        });
    }

    #[test]
    fn a_guard_is_not_live_once_its_build_has_ended() {
        let context = context();
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));

        let escaped = context.with_build_consumer(consumer, |_| BuildContext::current());

        assert!(!escaped.is_live());
    }

    #[test]
    #[should_panic(expected = "build that created it has ended")]
    fn using_a_guard_after_its_build_ended_panics() {
        let context = context();
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));

        let escaped = context.with_build_consumer(consumer, |_| BuildContext::current());

        let _ = escaped.get();
    }

    #[test]
    fn a_guard_does_not_match_a_later_build_at_the_same_depth() {
        let context = context();
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));
        let escaped = context.with_build_consumer(consumer.clone(), |_| BuildContext::current());

        context.with_build_consumer(consumer, |_| {
            assert!(BuildContext::try_current().is_ok());
            assert!(!escaped.is_live());
        });
    }

    #[test]
    fn a_panicking_build_leaves_no_current_context_behind() {
        let context = context();
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            context.with_build_consumer(consumer, |_| panic!("build failed"));
        }));

        assert!(result.is_err());
        assert!(BuildContext::try_current().is_err());
    }
}
