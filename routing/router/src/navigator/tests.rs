use std::cell::{Cell, RefCell};

use aimer_widget::base::{ResolvedSize, WindowHandle};
use aimer_widget::{Drawable, EventElement, LayoutElement, Rebuildable, VisitorElement};

use crate::Router;

use super::*;

thread_local! {
    static NAVIGATOR_OBSERVED: Cell<bool> = const { Cell::new(false) };
    static CURRENT_ROUTE_OBSERVED: Cell<Option<TestRoute>> = const { Cell::new(None) };
    static HISTORY_LENGTH_OBSERVED: Cell<usize> = const { Cell::new(0) };
    static NAVIGATOR_OPERATION_STEP: Cell<u8> = const { Cell::new(0) };
    static ROUTE_CONTEXT_MISSES: Cell<usize> = const { Cell::new(0) };
    static ROUTE_CONTEXT_NAVIGATOR: RefCell<Option<NavigatorController<TestRoute>>> =
        const { RefCell::new(None) };
    #[cfg(feature = "portable-guest")]
    static PORTABLE_ROUTE_BUILDS: RefCell<Vec<TestRoute>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestRoute {
    Home,
    Settings,
    Profile,
}

impl Route for TestRoute {
    fn parse(path: &str) -> Option<Self> {
        (path == "/").then_some(Self::Home)
    }

    fn format(&self) -> String {
        match self {
            Self::Home => "/".to_owned(),
            Self::Settings => "/settings".to_owned(),
            Self::Profile => "/profile".to_owned(),
        }
    }
}

struct NavigatorLookupWidget;

impl Widget for NavigatorLookupWidget {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        NavigatorLookupElement.boxed()
    }
}

impl aimer_widget::PortableWidget for NavigatorLookupWidget {}

struct NavigatorLookupElement;

impl VisitorElement for NavigatorLookupElement {
    fn debug_name(&self) -> &'static str {
        "NavigatorLookupElement"
    }
}
impl EventElement for NavigatorLookupElement {}
impl LayoutElement for NavigatorLookupElement {}
impl Rebuildable for NavigatorLookupElement {}
impl Drawable for NavigatorLookupElement {
    fn update(&self, ctx: &BuildContext) {
        let _ = NavigatorController::<TestRoute>::of(ctx);
        NAVIGATOR_OBSERVED.set(true);
    }
}

#[derive(Clone, Copy)]
struct AncestorRouteProvider;

struct RouteContextProbe;

impl Widget for RouteContextProbe {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        if ctx.get_state::<AncestorRouteProvider>().is_none() {
            ROUTE_CONTEXT_MISSES.set(ROUTE_CONTEXT_MISSES.get() + 1);
        }
        ROUTE_CONTEXT_NAVIGATOR.with(|controller| {
            *controller.borrow_mut() = Some(NavigatorController::<TestRoute>::of(ctx));
        });
        NavigatorLookupElement.boxed()
    }
}

impl aimer_widget::PortableWidget for RouteContextProbe {}

fn lookup_route_context(_: TestRoute) -> AnyWidget {
    RouteContextProbe.boxed()
}

struct NavigatorControllerOperationsWidget;

impl Widget for NavigatorControllerOperationsWidget {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        NavigatorControllerOperationsElement.boxed()
    }
}

impl aimer_widget::PortableWidget for NavigatorControllerOperationsWidget {}

struct NavigatorControllerOperationsElement;

impl VisitorElement for NavigatorControllerOperationsElement {
    fn debug_name(&self) -> &'static str {
        "NavigatorControllerOperationsElement"
    }
}
impl EventElement for NavigatorControllerOperationsElement {}
impl LayoutElement for NavigatorControllerOperationsElement {}
impl Rebuildable for NavigatorControllerOperationsElement {}
impl Drawable for NavigatorControllerOperationsElement {
    fn update(&self, ctx: &BuildContext) {
        let navigator = NavigatorController::<TestRoute>::of(ctx);

        match NAVIGATOR_OPERATION_STEP.get() {
            0 => {
                assert_eq!(navigator.current_route(), TestRoute::Home);
                navigator.push(TestRoute::Settings);
            }
            1 => {
                assert_eq!(navigator.current_route(), TestRoute::Settings);
                navigator.push(TestRoute::Profile);
            }
            2 => {
                assert_eq!(navigator.current_route(), TestRoute::Profile);
                navigator.set_route(TestRoute::Settings);
            }
            3 => {
                assert_eq!(navigator.current_route(), TestRoute::Settings);
                assert_eq!(navigator.history_len(), 1);
                navigator.push(TestRoute::Profile);
            }
            4 => {
                assert_eq!(navigator.current_route(), TestRoute::Profile);
                navigator.clear();
            }
            5 => {
                CURRENT_ROUTE_OBSERVED.set(Some(navigator.current_route()));
                HISTORY_LENGTH_OBSERVED.set(navigator.history_len());
                assert_eq!(navigator.routes(), vec![TestRoute::Home]);
                assert!(navigator.contains_route(&TestRoute::Home));
                let routes: Vec<_> = (&navigator).into_iter().collect();
                assert_eq!(routes, vec![TestRoute::Home]);
            }
            _ => return,
        }
        NAVIGATOR_OPERATION_STEP.set(NAVIGATOR_OPERATION_STEP.get() + 1);
    }
}

fn lookup_route(_: TestRoute) -> AnyWidget {
    NavigatorLookupWidget.boxed()
}

fn lookup_controller_operations(_: TestRoute) -> AnyWidget {
    NavigatorControllerOperationsWidget.boxed()
}

#[cfg(not(feature = "portable-guest"))]
impl Router for TestRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        lookup_route(*self)
    }
}

#[cfg(feature = "portable-guest")]
struct PortableRouteWidget;

#[cfg(feature = "portable-guest")]
impl aimer_widget::PortableWidget for PortableRouteWidget {
    fn to_portable_node(
        self,
        ctx: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<
        aimer_widget::portable::PortableNodeId,
        aimer_widget::portable::PortableBuildError,
    > {
        let navigator = NavigatorController::<TestRoute>::of(&ctx.build_context());
        let event_kind = aimer_widget::portable::__anteros::EventId::new(1);
        let callback_id = ctx.callback_id_for(None, source, event_kind);
        let callback = aimer_widget::portable::PortableCallback::new(
            event_kind,
            aimer_widget::portable::__anteros::Version::new(1, 0),
            callback_id,
            move || {
                navigator.push(TestRoute::Settings);
                Ok(())
            },
        );
        ctx.push_node_with_callbacks(
            aimer_widget::portable::__anteros::WIDGET_TEXT,
            aimer_widget::portable::__anteros::Version::new(1, 0),
            None,
            source,
            &[],
            vec![callback],
            &[],
        )
    }
}

#[cfg(feature = "portable-guest")]
impl Widget for PortableRouteWidget {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("portable route widget must not build natively")
    }
}

#[cfg(feature = "portable-guest")]
impl Router for TestRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        PORTABLE_ROUTE_BUILDS.with(|routes| routes.borrow_mut().push(*self));
        PortableRouteWidget.boxed()
    }
}

#[cfg(feature = "portable-guest")]
#[test]
fn navigator_portable_lowering_uses_the_active_router_tree() {
    use aimer_widget::PortableWidget as _;
    use aimer_widget::portable::__anteros::{WIDGET_TEXT, WidgetDocumentView};
    use aimer_widget::portable::{
        PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        StableId128,
    };

    let mut context = PortableBuildContext::new(
        1,
        1,
        PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
        PortableLimits::new(8, 16, 64, 128, 1_024),
    )
    .unwrap();
    let root = Navigator::new(TestRoute::Home, lookup_route)
        .to_portable_node(
            &mut context,
            SourceFingerprint::new(StableId128::from_bytes([7; 16])),
        )
        .unwrap();
    let document = context.finish_document(root).unwrap();
    let bytes = document.encode().unwrap();
    let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();

    assert_eq!(
        view.node(view.root_node()).unwrap().widget_type(),
        WIDGET_TEXT
    );
}

#[cfg(feature = "portable-guest")]
#[test]
fn navigator_portable_push_survives_the_rebuild_triggered_by_a_callback() {
    use aimer_widget::PortableWidget as _;
    use aimer_widget::portable::{
        PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        StableId128,
    };

    PORTABLE_ROUTE_BUILDS.with(|routes| routes.borrow_mut().clear());

    let mut context = PortableBuildContext::new(
        1,
        1,
        PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
        PortableLimits::new(8, 16, 64, 128, 1_024),
    )
    .unwrap();
    let source = SourceFingerprint::new(StableId128::from_bytes([7; 16]));
    let root = Navigator::new(TestRoute::Home, lookup_route)
        .to_portable_node(&mut context, source)
        .unwrap();
    context.finish_document(root).unwrap();

    let event_kind = aimer_widget::portable::__anteros::EventId::new(1);
    let callback_id = context.callback_id_for(None, source, event_kind);
    let callbacks = context.take_callback_registry();
    callbacks.dispatch(callback_id, &mut context).unwrap();
    assert!(context.take_rebuild_request());

    let root = Navigator::new(TestRoute::Home, lookup_route)
        .to_portable_node(&mut context, source)
        .unwrap();
    context.finish_document(root).unwrap();

    assert_eq!(
        PORTABLE_ROUTE_BUILDS.with(|routes| routes.borrow().clone()),
        vec![TestRoute::Home, TestRoute::Settings],
        "a callback-triggered portable rebuild must render the pushed route"
    );
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
fn navigator_controller_remains_scoped_on_a_fresh_frame_context() {
    NAVIGATOR_OBSERVED.set(false);
    let navigator = Navigator::new(TestRoute::Home, lookup_route);
    let initial_context = context();
    let element = navigator.to_element(&initial_context);

    element.update(&context());

    assert!(NAVIGATOR_OBSERVED.get());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn direct_route_replacement_retains_ancestor_provider_context() {
    ROUTE_CONTEXT_MISSES.set(0);
    ROUTE_CONTEXT_NAVIGATOR.with(|controller| *controller.borrow_mut() = None);

    let initial_context = context();
    let element = initial_context.with_state(AncestorRouteProvider, |ctx| {
        Navigator::new(TestRoute::Home, lookup_route_context).to_element(ctx)
    });

    assert_eq!(ROUTE_CONTEXT_MISSES.get(), 0);
    let controller = ROUTE_CONTEXT_NAVIGATOR
        .with(|controller| controller.borrow().clone())
        .expect("the route child should be able to read its Navigator");
    controller.push(TestRoute::Settings);

    let rebuild_context = context();
    element.rebuild_if_dirty(&rebuild_context);

    assert_eq!(
        ROUTE_CONTEXT_MISSES.get(),
        0,
        "a direct route child lost an ancestor provider during replacement"
    );
    assert!(
        rebuild_context
            .get_state::<AncestorRouteProvider>()
            .is_none(),
        "the captured provider must not leak outside the route boundary"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn navigator_controller_can_read_and_replace_route_history() {
    CURRENT_ROUTE_OBSERVED.set(None);
    HISTORY_LENGTH_OBSERVED.set(0);
    NAVIGATOR_OPERATION_STEP.set(0);
    let navigator = Navigator::new(TestRoute::Home, lookup_controller_operations);
    let initial_context = context();
    let element = navigator.to_element(&initial_context);

    for _ in 0..6 {
        element.update(&context());
        element.rebuild_if_dirty(&context());
    }

    assert_eq!(CURRENT_ROUTE_OBSERVED.get(), Some(TestRoute::Home));
    assert_eq!(HISTORY_LENGTH_OBSERVED.get(), 1);
}

#[test]
fn redirect_reroutes_once_then_settles() {
    // "guarded" redirects to "login"; "login" does not redirect.
    let result = resolve_redirect_chain(
        "guarded",
        |r| if *r == "guarded" { Some("login") } else { None },
        MAX_REDIRECT_HOPS,
    );
    assert_eq!(result, "login");
}

#[test]
fn redirect_none_passes_through() {
    let result = resolve_redirect_chain("home", |_| None, MAX_REDIRECT_HOPS);
    assert_eq!(result, "home");
}

#[test]
fn redirect_chain_follows_multiple_hops() {
    let result = resolve_redirect_chain(
        0,
        |n| if *n < 3 { Some(n + 1) } else { None },
        MAX_REDIRECT_HOPS,
    );
    assert_eq!(result, 3);
}

#[test]
fn redirect_loop_is_bounded_and_terminates() {
    // Always redirects: must terminate at the hop limit without hanging/panicking.
    let result = resolve_redirect_chain(0u32, |n| Some(n.wrapping_add(1)), 4);
    assert_eq!(result, 4);
}
