use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, ChildBuilder, State, StateUpdater,
    StatefulElement, StatefulWidget, Widget};

use crate::outlet::OutletChildBuilder;
use crate::shell_swipe::{OutletSwipe, ShellPeek, SwipePort};
use crate::swipe_back::SwipeLink;
use crate::transition_host::{NavigationPages, NavigationRequest, RouteBoundary, SwipeDrive, SwipeSetup};
use crate::{MAX_REDIRECT_HOPS, NavigationAction, NavigationTransition, Route, Router, Shell,
    resolve_redirect_chain};

type RouteBuilder<R> = Rc<dyn Fn(R, &BuildContext) -> AnyWidget>;

pub(crate) struct RouteOutlet<R: Route> {
    route: Option<R>,
    action: NavigationAction,
    fallback: Option<Rc<dyn NavigationTransition<R>>>,
    builder: Option<RouteBuilder<R>>,
    pages: NavigationPages<R>,
    /// A swipe the navigator asked for, started by the next build of the outlet.
    pub(crate) swipe: Option<(R, Rc<SwipeDrive>)>,
    /// The edge swipe of the owner of this outlet, when the touch is the outlet's
    /// to take: a stateful shell has no navigator around it to own the touch.
    pub(crate) swipe_link: Option<Rc<SwipeLink>>,
}

impl<R: Route> Default for RouteOutlet<R> {
    fn default() -> Self {
        Self { route: None, action: NavigationAction::Push, fallback: None, builder: None, pages: NavigationPages::default(), swipe: None, swipe_link: None }
    }
}

impl<R: Route> RouteOutlet<R> {
    pub(crate) fn configure(&mut self, route: R, action: NavigationAction,
        fallback: Option<Rc<dyn NavigationTransition<R>>>, builder: RouteBuilder<R>) {
        self.route = Some(route);
        self.action = action;
        self.fallback = fallback;
        self.builder = Some(builder);
    }
}

pub(crate) fn route_outlet_builder<R: Route>(outlet: &Rc<RefCell<RouteOutlet<R>>>) -> OutletChildBuilder {
    // Pages capture their inherited OutletSlot. A strong reference here would
    // create slot -> pages -> page context -> slot ownership cycles.
    let outlet = Rc::downgrade(outlet);
    Rc::new(move |ctx| {
        let outlet = outlet.upgrade().expect("a routed outlet requires its owning shell");
        let mut outlet = outlet.borrow_mut();
        let route = outlet.route.clone().expect("the shell configured its active route");
        let effective = resolve_redirect_chain(route, |route| route.redirect(ctx), MAX_REDIRECT_HOPS);
        let action = outlet.action;
        let fallback = outlet.fallback.clone();
        let builder = outlet.builder.clone().expect("the shell configured its builder");
        // A routed shell has no edge swipe of its own: the touch belongs to the
        // navigator around it, which hands the swipe in through here. A stateful
        // shell has no navigator, so its outlet carries the link itself.
        let start = outlet.swipe.take();
        let swipe = (start.is_some() || outlet.swipe_link.is_some())
            .then(|| SwipeSetup { link: outlet.swipe_link.clone(), start });
        ctx.with_state(NavigationRequest(action), |ctx| {
            outlet.pages.build(effective, action, fallback.as_deref(), ctx, swipe, |route| builder(route, ctx)).boxed()
        })
    })
}

/// A persistent [`Shell`] with typed child-route transition metadata.
///
/// Created by [`Shell::route`]. Only its outlet animates; the frame is retained
/// through child changes. Its fallback is independent of an outer Navigator's
/// fallback, because the shell's child routes may have a different type.
pub struct RoutedShell<R: Route + Router> {
    frame: AnyWidget,
    builder: OutletChildBuilder,
    route: R,
    transition: Option<Rc<dyn NavigationTransition<R>>>,
}

impl<R: Route + Router> RoutedShell<R> {
    pub(crate) fn new(frame: AnyWidget, builder: OutletChildBuilder, route: R) -> Self {
        Self { frame, builder, route, transition: None }
    }

    /// Sets the fallback effect for child routes which inherit their selection.
    #[inline]
    pub fn transition(mut self, transition: impl NavigationTransition<R>) -> Self {
        self.transition = Some(Rc::new(transition));
        self
    }
}

impl<R: Route + Router> aimer_widget::PortableWidget for RoutedShell<R> {
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(self, ctx: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<aimer_widget::portable::PortableNodeId, aimer_widget::portable::PortableBuildError> {
        // Keep the existing portable shell lowering and immediate child swap.
        let builder = self.builder;
        aimer_widget::PortableWidget::to_portable_node(Shell::new(self.frame, move |ctx| builder(ctx)), ctx, source)
    }
}

/// The shell's outlet as a navigator's swipe port sees it.
struct OutletHandle<R: Route + Router> {
    outlet: Rc<RefCell<RouteOutlet<R>>>,
    updater: StateUpdater<RoutedShellState<R>>,
}

impl<R: Route + Router> OutletSwipe for OutletHandle<R> {
    fn begin(&self, previous: &dyn Any, drive: Rc<SwipeDrive>) -> bool {
        let Some(previous) = previous.downcast_ref::<R>().cloned() else { return false };
        self.outlet.borrow_mut().swipe = Some((previous, drive));
        // The outlet rebuilds exactly as it does for a route change, and starts
        // the swipe while it does.
        self.updater.set_state(|_| {});
        true
    }
}

impl<R: Route + Router> Widget for RoutedShell<R> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        // A navigator is only asking which child this shell shows, to start a
        // swipe back to it. Report the child instead of mounting a second shell.
        if let Some(peek) = ctx.get_state::<ShellPeek>() {
            peek.deposit(Rc::new(self.route));
            return aimer_container::SizedBox::new().to_element(ctx);
        }
        let key = ctx.get_state::<RouteBoundary>().map(|boundary| boundary.0.clone());
        StatefulElement::new_with_name(self, ctx, "RoutedShell", key).0.boxed()
    }
}

impl<R: Route + Router> StatefulWidget for RoutedShell<R> {
    type State = RoutedShellState<R>;
    fn create_state(self) -> Self::State {
        RoutedShellState {
            frame: ChildBuilder::from_widget(self.frame), builder: self.builder,
            route: self.route, transition: self.transition, outlet: Rc::new(RefCell::new(RouteOutlet::default())),
            handle: None,
        }
    }
}

#[doc(hidden)]
pub struct RoutedShellState<R: Route + Router> {
    frame: ChildBuilder,
    builder: OutletChildBuilder,
    route: R,
    transition: Option<Rc<dyn NavigationTransition<R>>>,
    outlet: Rc<RefCell<RouteOutlet<R>>>,
    /// What the navigator's swipe port reaches this outlet through.
    handle: Option<Rc<dyn OutletSwipe>>,
}

impl<R: Route + Router> State<RoutedShell<R>> for RoutedShellState<R> {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.handle = Some(Rc::new(OutletHandle { outlet: self.outlet.clone(), updater }));
    }
    fn adopt_config_from(&mut self, new: Self) {
        self.builder = new.builder;
        self.route = new.route;
        self.transition = new.transition;
    }
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let route = self.route.clone();
        let path = route.format();
        let builder = self.builder.clone();
        if let (Some(port), Some(handle)) = (ctx.get_state::<SwipePort>(), &self.handle) {
            port.attach(handle);
            // The outlet's host owns the touch, so a drag that begins on the
            // frame around it is not a swipe.
            self.outlet.borrow_mut().swipe_link = Some(port.link());
        }
        let action = ctx.get_state::<NavigationRequest>().map(|request| request.0).unwrap_or(NavigationAction::History);
        self.outlet.borrow_mut().configure(route, action, self.transition.clone(), Rc::new(move |route, ctx| {
            if route.format() == path { builder(ctx) } else { Router::build(&route, ctx) }
        }));
        let builder = route_outlet_builder(&self.outlet);
        Shell::new(self.frame.clone(), move |ctx| builder(ctx))
    }
}
