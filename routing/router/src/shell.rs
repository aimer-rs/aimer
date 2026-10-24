use std::cell::RefCell;
use std::rc::Rc;

use aimer_widget::base::{BuildContext, ResolvedSize, Size, Vec2d};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement,
    Rebuildable, State, StateUpdater, StatefulElement, StatefulWidget, VisitorElement, Widget,
};

use aimer_events::back::{self, BackGesture, BackGuard, BackHandler};

use crate::{Route, Router, NavigationAction, NavigationTransition, RoutedShell, SwipeBack, SwipeBackArea, TransitionChoice};
use crate::swipe_back::SwipeLink;
use crate::transition_host::SwipeDrive;
use crate::routed_shell::{RouteOutlet, route_outlet_builder};
use crate::outlet::{OutletChildBuilder, OutletSlot, RouteChildContext};

/// A persistent layout frame (nav bar, drawer, header, ...) that stays mounted
/// while only its inner [`crate::outlet::Outlet`] swaps between child routes.
///
/// Construct a `Shell` with the frame widget — which must contain an `Outlet`
/// somewhere in its tree — and a builder for the currently active child route.
/// When built, the shell injects an [`OutletSlot`] into the context so the
/// descendant outlet can render the child.
pub struct Shell {
    frame: AnyWidget,
    child_builder: OutletChildBuilder,
}

impl Shell {
    /// Supplies the active child route so this shell's outlet can animate.
    /// Configure an optional fallback with [`RoutedShell::transition`]. The
    /// existing child builder is used unless a redirect selects another route.
    #[inline]
    pub fn route<R: Route + Router>(self, route: R) -> RoutedShell<R> {
        RoutedShell::new(self.frame, self.child_builder, route)
    }
    /// Creates a shell from a `frame` widget (containing an [`crate::Outlet`])
    /// and a closure that builds the active child widget.
    ///
    /// The frame is the shell's child and remains the stable outer subtree;
    /// the closure supplies the content requested by its descendant outlet.
    pub fn new(
        frame: impl Widget + 'static,
        child_builder: impl Fn(&BuildContext) -> AnyWidget + 'static,
    ) -> Self {
        Self {
            frame: frame.boxed(),
            child_builder: Rc::new(child_builder),
        }
    }

    /// Creates the heap-allocated [`Widget`] form of [`Shell::new`].
    pub fn boxing(
        frame: impl Widget + 'static,
        child_builder: impl Fn(&BuildContext) -> AnyWidget + 'static,
    ) -> AnyWidget {
        Self::new(frame, child_builder).boxed()
    }

    /// Creates a shell whose active child is a fixed, cloneable widget value.
    ///
    /// The child is cloned each time the outlet requests it.
    pub fn with_child(frame: impl Widget + 'static, child: impl Widget + Clone + 'static) -> Self {
        Self::new(frame, move |_| child.clone().boxed())
    }
}

impl aimer_widget::PortableWidget for Shell {
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        ctx: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<aimer_widget::portable::PortableNodeId, aimer_widget::portable::PortableBuildError>
    {
        let slot = OutletSlot::new(self.child_builder);
        ctx.with_state(slot, |ctx| {
            self.frame.into_portable_node(ctx, source.child(0))
        })
    }
}

impl Widget for Shell {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let route_context = RouteChildContext::capture(ctx);
        let slot = OutletSlot::new(self.child_builder.clone());
        let child = route_context.with(ctx, |ctx| {
            ctx.with_state(slot.clone(), |ctx| self.frame.to_element(ctx))
        });
        ShellElement {
            route_context,
            slot,
            child,
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Shell"
    }
}

struct ShellElement {
    route_context: RouteChildContext,
    slot: OutletSlot,
    child: AnyElement,
}

impl ShellElement {
    fn scoped<R>(&self, ctx: &BuildContext, callback: impl FnOnce(&BuildContext) -> R) -> R {
        self.route_context
            .with(ctx, |ctx| ctx.with_state(self.slot.clone(), callback))
    }
}

impl VisitorElement for ShellElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "ShellScope"
    }
}

impl Drawable for ShellElement {
    fn update(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.update(ctx));
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        _child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        self.route_context
            .with(ctx, |ctx| Some(ctx.fork_with_state(self.slot.clone())))
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.sync_paint_geometry(ctx));
    }
}

impl LayoutElement for ShellElement {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.layout(ctx))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.computed_size(ctx))
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.content_size(ctx))
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
}

impl EventElement for ShellElement {}

impl Rebuildable for ShellElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.rebuild_if_dirty(ctx));
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.scoped(ctx, callback);
    }

    fn mark_needs_rebuild(&self) {
        self.child.mark_needs_rebuild();
    }
}

// ---------------------------------------------------------------------------
// StatefulShell: per-branch history stacks (go_router's StatefulShellRoute).
// ---------------------------------------------------------------------------

/// Push `route` onto branch `index`'s stack (no-op if the index is out of
/// range).
pub fn branch_push<R>(branches: &mut [Vec<R>], index: usize, route: R) {
    if let Some(branch) = branches.get_mut(index) {
        branch.push(route);
    }
}

/// Pop branch `index`'s stack, guarded so a branch stack is never emptied.
pub fn branch_pop<R>(branches: &mut [Vec<R>], index: usize) {
    if let Some(branch) = branches.get_mut(index)
        && branch.len() > 1
    {
        branch.pop();
    }
}

/// The top (active) route of branch `active`, if any.
pub fn active_top<R: Clone>(branches: &[Vec<R>], active: usize) -> Option<R> {
    branches.get(active).and_then(|b| b.last().cloned())
}

/// A tabbed shell that keeps an independent navigation stack per branch, so
/// switching branches preserves each branch's history (StatefulShellRoute).
///
/// Only the active branch's top route is rendered into the shell's
/// [`crate::Outlet`]. Each branch starts with exactly one route and guarded
/// pops never empty a stack. Descendants navigate through
/// [`StatefulShellController`].
#[derive(aimer_widget::PortableWidget)]
#[portable_widget(id = "aimer_router::StatefulShell", schema_only)]
pub struct StatefulShell<R: Route> {
    #[portable_skip]
    pub branches: Vec<Vec<R>>,
    #[portable_skip]
    pub active: usize,
    #[portable_skip]
    pub frame: fn(&BuildContext) -> AnyWidget,
    #[portable_skip]
    pub routes: fn(R) -> AnyWidget,
    #[portable_skip]
    pub transition: Option<Rc<dyn NavigationTransition<R>>>,
    #[portable_skip]
    swipe_area: SwipeBackArea,
}

impl<R: Route> StatefulShell<R> {
    /// Creates a stateful shell from one initial route per branch.
    ///
    /// `frame` builds the persistent layout (which must contain an
    /// [`crate::Outlet`]) and `routes` builds the widget for a given child
    /// route.
    ///
    /// Branch zero is active initially. `initial_routes` must not be empty, and
    /// every branch created by this constructor has a non-empty history stack.
    pub fn new(
        initial_routes: Vec<R>,
        frame: fn(&BuildContext) -> AnyWidget,
        routes: fn(R) -> AnyWidget,
    ) -> Self {
        let branches = initial_routes.into_iter().map(|r| vec![r]).collect();
        Self {
            branches,
            active: 0,
            frame,
            routes,
            transition: None,
            swipe_area: SwipeBackArea::default(),
        }
    }

    /// Sets where on a page a swipe back may begin. The default is
    /// [`SwipeBackArea::Anywhere`].
    #[inline]
    pub fn swipe_back_area(mut self, area: SwipeBackArea) -> Self {
        self.swipe_area = area;
        self
    }

    /// Sets the fallback effect for visible branch and child-route changes.
    #[inline]
    pub fn transition(mut self, transition: impl NavigationTransition<R>) -> Self {
        self.transition = Some(Rc::new(transition));
        self
    }
}

pub struct StatefulShellState<R: Route> {
    pub branches: Vec<Vec<R>>,
    pub active: usize,
    pub updater: StateUpdater<Self>,
    pub frame: fn(&BuildContext) -> AnyWidget,
    pub routes: fn(R) -> AnyWidget,
    transition: Option<Rc<dyn NavigationTransition<R>>>,
    retained_frame: RefCell<Option<aimer_widget::ChildBuilder>>,
    outlet: Rc<RefCell<RouteOutlet<R>>>,
    action: NavigationAction,
    swipe_area: SwipeBackArea,
    /// A swipe asked for since the last build, started by the next one.
    swipe_request: RefCell<Option<Rc<SwipeDrive>>>,
    /// The edge swipe of the active branch, carried by the outlet's host.
    swipe_link: Rc<SwipeLink>,
    /// Keeps this shell answering the system's back request while it lives.
    back_guard: Option<BackGuard>,
}

impl<R: Route> StatefulShellState<R> {
    /// Whether the active branch has a page to go back to and an effect for a
    /// finger to drive.
    fn can_swipe_back(&self) -> bool {
        let Some([.., _, top]) = self.branches.get(self.active).map(Vec::as_slice) else { return false };
        match top.navigation_transition() {
            TransitionChoice::Disabled => false,
            TransitionChoice::Custom(_) => true,
            TransitionChoice::Inherit => self.transition.is_some(),
        }
    }

    /// Asks for a swipe back in the active branch, which the next build starts.
    fn begin_swipe_back(&self) -> Option<SwipeBack> {
        if !self.can_swipe_back() { return None; }
        let updater = self.updater;
        let drive = SwipeDrive::new(move || {
            updater.read(|state| state.pop_active());
        });
        let request = drive.clone();
        self.updater.set_state(move |state| *state.swipe_request.borrow_mut() = Some(request));
        Some(SwipeBack(drive))
    }

    /// Pops the active branch, as a committed swipe or a back request does.
    fn pop_active(&self) -> bool {
        let can_pop = self.branches.get(self.active).is_some_and(|branch| branch.len() > 1);
        if can_pop {
            self.updater.set_state(|state| {
                branch_pop(&mut state.branches, state.active);
                state.action = NavigationAction::Pop;
            });
        }
        can_pop
    }
}

/// Answers the system's back request by popping the active branch, and declines
/// at its root so an enclosing navigator, and then the platform, may answer.
struct StatefulShellBack<R: Route> {
    updater: StateUpdater<StatefulShellState<R>>,
}

impl<R: Route> BackHandler for StatefulShellBack<R> {
    fn on_back(&self) -> bool {
        self.updater.read(|state| state.pop_active())
    }

    fn on_back_started(&self) -> Option<Box<dyn BackGesture>> {
        self.updater.read(|state| state.begin_swipe_back()).map(|swipe| Box::new(swipe) as Box<dyn BackGesture>)
    }
}

impl<R: Route> State<StatefulShell<R>> for StatefulShellState<R> {
    fn adopt_config_from(&mut self, new: Self) {
        self.routes = new.routes;
        self.transition = new.transition;
        self.swipe_area = new.swipe_area;
    }

    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
        self.swipe_link.install(move || updater.read(|state| state.begin_swipe_back()));
        self.back_guard = Some(back::register(StatefulShellBack { updater }));
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        // Inject the imperative controller for descendants.
        ctx.insert_state(StatefulShellController::<R> {
            go_branch_fn: {
                let updater = self.updater;
                Rc::new(move |index: usize| {
                    updater.set_state(move |state| {
                        if index < state.branches.len() && index != state.active {
                            state.active = index;
                            state.action = NavigationAction::BranchSwitch;
                        }
                    });
                })
            },
            push_in_branch_fn: {
                let updater = self.updater;
                Rc::new(move |index: usize, route: R| {
                    updater.set_state(move |state| {
                        branch_push(&mut state.branches, index, route);
                        if index == state.active { state.action = NavigationAction::Push; }
                    });
                })
            },
            pop_in_branch_fn: {
                let updater = self.updater;
                Rc::new(move |index: usize| {
                    updater.set_state(move |state| {
                        branch_pop(&mut state.branches, index);
                        if index == state.active { state.action = NavigationAction::Pop; }
                    });
                })
            },
            active_branch_fn: {
                let updater = self.updater;
                Rc::new(move || updater.read(|state| state.active))
            },
            branch_len_fn: {
                let updater = self.updater;
                Rc::new(move |index: usize| updater.read(|state| state.branches.get(index).map(|b| b.len()).unwrap_or(0)))
            },
            begin_swipe_back_fn: {
                let updater = self.updater;
                Rc::new(move || updater.read(|state| state.begin_swipe_back()))
            },
        });

        // Feed only the active branch's top route into the `Outlet`.
        let top = active_top(&self.branches, self.active).expect("StatefulShell branch stack must not be empty");
        let routes = self.routes;
        self.outlet.borrow_mut().configure(top, self.action, self.transition.clone(),
            Rc::new(move |route, _| routes(route)));
        // A swipe back to the page beneath the active branch's top, if one was
        // asked for and there still is one.
        self.swipe_link.set_enabled(self.can_swipe_back());
        self.swipe_link.set_anywhere(self.swipe_area == SwipeBackArea::Anywhere);
        if let Some(drive) = self.swipe_request.borrow_mut().take() {
            match self.branches.get(self.active).and_then(|branch| branch.len().checked_sub(2).map(|index| branch[index].clone())) {
                Some(previous) => self.outlet.borrow_mut().swipe = Some((previous, drive)),
                None => drive.cancel_if_unsettled(),
            }
        }
        let child_builder = route_outlet_builder(&self.outlet);

        // Reflect the active branch's top route in the browser address bar.
        #[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
        if let Some(route) = active_top(&self.branches, self.active) {
            crate::navigator::browser_replace_state(&route.format());
        }

        let frame = self.retained_frame.borrow_mut().get_or_insert_with(||
            aimer_widget::ChildBuilder::from_widget((self.frame)(ctx))).clone();
        Shell::new(frame, move |ctx| child_builder(ctx))
    }
}

pub struct StatefulShellController<R> {
    go_branch_fn: Rc<dyn Fn(usize)>,
    push_in_branch_fn: Rc<dyn Fn(usize, R)>,
    pop_in_branch_fn: Rc<dyn Fn(usize)>,
    active_branch_fn: Rc<dyn Fn() -> usize>,
    branch_len_fn: Rc<dyn Fn(usize) -> usize>,
    begin_swipe_back_fn: Rc<dyn Fn() -> Option<SwipeBack>>,
}

impl<R> Clone for StatefulShellController<R> {
    fn clone(&self) -> Self {
        Self {
            go_branch_fn: self.go_branch_fn.clone(),
            push_in_branch_fn: self.push_in_branch_fn.clone(),
            pop_in_branch_fn: self.pop_in_branch_fn.clone(),
            active_branch_fn: self.active_branch_fn.clone(),
            branch_len_fn: self.branch_len_fn.clone(),
            begin_swipe_back_fn: self.begin_swipe_back_fn.clone(),
        }
    }
}

pub type StatefulShellInstance<R> = Rc<StatefulShellController<R>>;

impl<R: 'static> StatefulShellController<R> {
    /// Obtain the controller from the context:
    /// `StatefulShellController::<R>::of(ctx)`.
    #[track_caller]
    pub fn of(ctx: &BuildContext) -> StatefulShellInstance<R> {
        ctx.get_state::<StatefulShellController<R>>()
            .expect("No StatefulShell found in context. Make sure a StatefulShell widget is an ancestor.")
            .clone()
    }

    /// Switch the active branch.
    pub fn go_branch(&self, index: usize) {
        (self.go_branch_fn)(index);
    }

    /// Push a route onto a specific branch's stack.
    pub fn push_in_branch(&self, index: usize, route: R) {
        (self.push_in_branch_fn)(index, route);
    }

    /// Pop a specific branch's stack (guarded so it never empties).
    pub fn pop_in_branch(&self, index: usize) {
        (self.pop_in_branch_fn)(index);
    }

    /// The currently active branch index.
    pub fn active_branch(&self) -> usize {
        (self.active_branch_fn)()
    }

    /// The stack depth of a given branch.
    pub fn branch_len(&self, index: usize) -> usize {
        (self.branch_len_fn)(index)
    }

    /// Starts popping the active branch under a finger's control.
    ///
    /// The page beneath is built and shown, and the returned [`SwipeBack`] moves
    /// the current page across it with the effect the departing route selects.
    /// Returns `None`, and does nothing, when the active branch is at its root or
    /// has no effect to play. An edge swipe on the shell's outlet calls this
    /// itself. See [`crate::NavigatorController::begin_swipe_back`].
    pub fn begin_swipe_back(&self) -> Option<SwipeBack> {
        (self.begin_swipe_back_fn)()
    }
}

impl<R: Route> StatefulWidget for StatefulShell<R> {
    type State = StatefulShellState<R>;
    fn create_state(self) -> Self::State {
        let swipe_link = SwipeLink::detached();
        let outlet = Rc::new(RefCell::new(RouteOutlet::default()));
        outlet.borrow_mut().swipe_link = Some(swipe_link.clone());
        StatefulShellState::<R> {
            branches: self.branches.clone(),
            active: self.active,
            updater: StateUpdater::empty(),
            frame: self.frame,
            routes: self.routes,
            transition: self.transition,
            retained_frame: RefCell::new(None),
            outlet,
            action: NavigationAction::Push,
            swipe_area: self.swipe_area,
            swipe_request: RefCell::new(None),
            swipe_link,
            back_guard: None,
        }
    }
}

impl<R: Route> Widget for StatefulShell<R> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let (el, _) = StatefulElement::new(self, ctx);
        el.boxed()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use aimer_widget::base::{BuildContext, ResolvedSize, WindowHandle};
    use aimer_widget::{Drawable, EventElement, LayoutElement, Rebuildable, VisitorElement};

    use crate::Outlet;

    use super::*;

    struct DeferredOutletWidget {
        child_built: Rc<Cell<bool>>,
    }

    #[derive(Clone, Copy)]
    struct AncestorShellProvider;

    thread_local! {
        static SHELL_ROUTE_CONTEXT_MISSES: Cell<usize> = const { Cell::new(0) };
    }

    struct ShellRouteContextProbe;

    impl Widget for ShellRouteContextProbe {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            if ctx.get_state::<AncestorShellProvider>().is_none() {
                SHELL_ROUTE_CONTEXT_MISSES.set(SHELL_ROUTE_CONTEXT_MISSES.get() + 1);
            }
            DeferredOutletElement {
                child_built: Rc::new(Cell::new(false)),
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for ShellRouteContextProbe {}

    impl Widget for DeferredOutletWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            DeferredOutletElement {
                child_built: self.child_built.clone(),
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for DeferredOutletWidget {}

    struct DeferredOutletElement {
        child_built: Rc<Cell<bool>>,
    }

    impl VisitorElement for DeferredOutletElement {
        fn debug_name(&self) -> &'static str {
            "DeferredOutletElement"
        }
    }

    impl EventElement for DeferredOutletElement {}
    impl LayoutElement for DeferredOutletElement {}
    impl Drawable for DeferredOutletElement {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl Rebuildable for DeferredOutletElement {
        fn rebuild_if_dirty(&self, ctx: &BuildContext) {
            let _ = crate::Outlet.to_element(ctx);
            self.child_built.set(true);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn context() -> BuildContext<'static> {
        use std::sync::OnceLock;

        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let _guard = runtime.enter();
        BuildContext::new(
            canvas,
            ResolvedSize::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn outlet_slot_remains_scoped_during_a_delayed_descendant_rebuild() {
        let child_built = Rc::new(Cell::new(false));
        let shell = Shell::new(
            DeferredOutletWidget {
                child_built: child_built.clone(),
            },
            |_| {
                DeferredOutletWidget {
                    child_built: Rc::new(Cell::new(false)),
                }
                .boxed()
            },
        );
        let element = shell.to_element(&context());

        element.rebuild_if_dirty(&context());

        assert!(child_built.get());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn shell_route_child_retains_ancestor_provider_context() {
        SHELL_ROUTE_CONTEXT_MISSES.set(0);
        let initial_context = context();
        let shell = Shell::new(
            DeferredOutletWidget {
                child_built: Rc::new(Cell::new(false)),
            },
            |_| ShellRouteContextProbe.boxed(),
        );
        let element =
            initial_context.with_state(AncestorShellProvider, |ctx| shell.to_element(ctx));

        let rebuild_context = context();
        element.rebuild_if_dirty(&rebuild_context);

        assert_eq!(
            SHELL_ROUTE_CONTEXT_MISSES.get(),
            0,
            "a shell route child lost an ancestor provider during delayed Outlet construction"
        );
        assert!(
            rebuild_context
                .get_state::<AncestorShellProvider>()
                .is_none(),
            "the captured provider must not leak outside the shell boundary"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn shell_scope_does_not_claim_parent_state_ownership() {
        let shell = Shell::new(
            DeferredOutletWidget {
                child_built: Rc::new(Cell::new(false)),
            },
            |_| {
                DeferredOutletWidget {
                    child_built: Rc::new(Cell::new(false)),
                }
                .boxed()
            },
        );

        assert!(!shell.to_element(&context()).is_carry_state());
    }

    #[test]
    fn branch_push_updates_active_top_and_depth() {
        let mut branches = vec![vec!["home"], vec!["reports"]];
        branch_push(&mut branches, 0, "detail");
        assert_eq!(active_top(&branches, 0), Some("detail"));
        assert_eq!(branches[0].len(), 2);
    }

    #[test]
    fn switching_branch_preserves_each_stack() {
        let mut branches = vec![vec!["a_home"], vec!["b_home"]];
        // Push into branch A.
        branch_push(&mut branches, 0, "a_detail");
        // Switch to branch B (only the active index changes): B keeps its own top.
        assert_eq!(active_top(&branches, 1), Some("b_home"));
        // Switch back to branch A: its top route and depth are restored.
        assert_eq!(active_top(&branches, 0), Some("a_detail"));
        assert_eq!(branches[0].len(), 2);
        assert_eq!(branches[1].len(), 1);
    }

    #[test]
    fn pop_is_guarded_and_never_empties_a_branch() {
        let mut branches = vec![vec!["only"]];
        branch_pop(&mut branches, 0);
        assert_eq!(branches[0].len(), 1);

        branch_push(&mut branches, 0, "second");
        branch_pop(&mut branches, 0);
        assert_eq!(active_top(&branches, 0), Some("only"));
    }

    #[test]
    fn out_of_range_branch_ops_are_noops() {
        let mut branches: Vec<Vec<&str>> = vec![vec!["x"]];
        branch_push(&mut branches, 5, "y");
        branch_pop(&mut branches, 5);
        assert_eq!(active_top(&branches, 5), None);
        assert_eq!(branches[0].len(), 1);
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn shell_portable_lowering_replaces_outlet_with_the_active_child() {
        use aimer_widget::portable::__anteros::{Version, WIDGET_TEXT, WidgetDocumentView};
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
            StableId128,
        };
        use aimer_widget::{ErrorWidget, PortableWidget};

        struct PortableLeaf;

        impl Widget for PortableLeaf {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                ErrorWidget::new("portable leaf").to_element(ctx)
            }
        }

        impl PortableWidget for PortableLeaf {
            fn to_portable_node(
                self,
                ctx: &mut PortableBuildContext,
                source: SourceFingerprint,
            ) -> Result<
                aimer_widget::portable::PortableNodeId,
                aimer_widget::portable::PortableBuildError,
            > {
                ctx.push_node(WIDGET_TEXT, Version::new(1, 0), None, source, &[], &[])
            }
        }

        let mut context = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let root = Shell::new(Outlet, |_| PortableLeaf.boxed())
            .to_portable_node(
                &mut context,
                SourceFingerprint::new(StableId128::from_bytes([8; 16])),
            )
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();

        assert_eq!(
            view.node(view.root_node()).unwrap().widget_type(),
            WIDGET_TEXT
        );
        assert_eq!(view.node_count(), 1);
    }
}
