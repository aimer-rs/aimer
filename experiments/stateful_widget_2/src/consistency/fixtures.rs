//! Widgets and specs the cases are built from. Nothing here names a backend.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_events::element::ElementEvent;
use aimer_widget_core::base::*;
use aimer_widget_core::{
    AnyElement, Drawable, Element, ErrorWidget, EventElement, EventResult, EventTreeRole,
    LayoutElement, Rebuildable, StatelessElement, VisitorElement, Widget,
};

use super::{Backend, Handle, Probe, Spec};

// ---------------------------------------------------------------------------
// Plain elements
// ---------------------------------------------------------------------------

pub struct TestLeaf;

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

/// The smallest widget: builds to a [`TestLeaf`].
pub struct LeafProbe;

impl Widget for LeafProbe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        TestLeaf.boxed()
    }
}

/// Counts the rebuild passes that reach it.
pub struct RebuildCountLeaf {
    pub rebuilds: Rc<Cell<usize>>,
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

/// A container that records the order in which `is_stateful_element` is asked.
pub struct TraversalElement {
    pub id: usize,
    pub children: Vec<AnyElement>,
    pub visits: Rc<RefCell<Vec<usize>>>,
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

pub fn traversal_element(
    id: usize,
    children: Vec<AnyElement>,
    visits: &Rc<RefCell<Vec<usize>>>,
) -> AnyElement {
    TraversalElement { id, children, visits: visits.clone() }.boxed()
}

/// Counts the events routed to it.
pub struct EventProbeChild {
    pub events: Rc<Cell<usize>>,
}

pub struct EventProbeElement {
    events: Rc<Cell<usize>>,
}

impl Widget for EventProbeChild {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        EventProbeElement { events: self.events }.boxed()
    }
}

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

/// Where in a child's life a [`Lifecycle`] widget should panic.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum PanicPhase {
    None,
    CreateState,
    InitState,
    Build,
    ToElement,
    AdoptConfig,
    ChildRuntimeAdoption,
}

pub struct LifecycleChild {
    pub panic_in_to_element: bool,
    pub panic_in_runtime_adoption: bool,
}

pub struct LifecycleChildElement {
    panic_in_runtime_adoption: bool,
}

impl Widget for LifecycleChild {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        if self.panic_in_to_element {
            panic!("child conversion failed");
        }
        LifecycleChildElement { panic_in_runtime_adoption: self.panic_in_runtime_adoption }.boxed()
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

/// Builds `Sp` as a child of whatever builds this widget.
///
/// A spec cannot name its backend, so the backend is baked in as a function
/// pointer by [`nest`].
pub struct Nest<Sp: Spec> {
    props: Sp::Props,
    name: &'static str,
    make: fn(Sp::Props, &BuildContext, &'static str) -> AnyElement,
}

impl<Sp: Spec> Widget for Nest<Sp> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        (self.make)(self.props, ctx, self.name)
    }

    fn debug_name(&self) -> &'static str {
        self.name
    }
}

pub fn nest<B: Backend, Sp: Spec>(props: Sp::Props, name: &'static str) -> Nest<Sp> {
    Nest { props, name, make: |props, ctx, name| B::element::<Sp>(props, ctx, name, None) }
}

// ---------------------------------------------------------------------------
// Specs
// ---------------------------------------------------------------------------

/// Counts builds; builds an error element so the child is trivially cheap.
pub struct Dirty;

pub struct DirtyProps {
    pub builds: Rc<Cell<usize>>,
}

pub struct DirtyState {
    builds: Rc<Cell<usize>>,
}

impl Spec for Dirty {
    type Props = DirtyProps;
    type State = DirtyState;

    fn create(props: &DirtyProps) -> DirtyState {
        DirtyState { builds: props.builds.clone() }
    }

    fn build<B: Backend>(state: &DirtyState, _this: Handle<DirtyState>, _ctx: &BuildContext) -> impl Widget {
        state.builds.set(state.builds.get() + 1);
        ErrorWidget::new("state").boxed()
    }
}

/// Counts builds and taps, and hands out its handle.
pub struct Tap;

pub struct TapProps {
    pub builds: Rc<Cell<usize>>,
    pub probe: Probe<TapState>,
}

pub struct TapState {
    pub taps: usize,
    builds: Rc<Cell<usize>>,
    probe: Probe<TapState>,
}

impl Spec for Tap {
    type Props = TapProps;
    type State = TapState;

    fn create(props: &TapProps) -> TapState {
        TapState { taps: 0, builds: props.builds.clone(), probe: props.probe.clone() }
    }

    fn build<B: Backend>(state: &TapState, this: Handle<TapState>, _ctx: &BuildContext) -> impl Widget {
        state.builds.set(state.builds.get() + 1);
        state.probe.capture(&this);
        LeafProbe
    }
}

/// Records every hook in the order the backend runs it.
pub struct Init;

pub struct InitProps {
    pub log: Rc<RefCell<Vec<&'static str>>>,
    pub probe: Probe<InitState>,
}

pub struct InitState {
    log: Rc<RefCell<Vec<&'static str>>>,
    probe: Probe<InitState>,
}

impl Spec for Init {
    type Props = InitProps;
    type State = InitState;

    fn create(props: &InitProps) -> InitState {
        props.log.borrow_mut().push("create");
        InitState { log: props.log.clone(), probe: props.probe.clone() }
    }

    fn init(state: &mut InitState) {
        state.log.borrow_mut().push("init");
    }

    fn build<B: Backend>(state: &InitState, this: Handle<InitState>, _ctx: &BuildContext) -> impl Widget {
        state.log.borrow_mut().push("build");
        state.probe.capture(&this);
        LeafProbe
    }
}

/// Shows which props and which taps each build saw.
pub struct Carry;

pub struct CarryProps {
    pub label: &'static str,
    pub seen: Rc<RefCell<Vec<(&'static str, usize)>>>,
    pub probe: Probe<CarryState>,
}

pub struct CarryState {
    pub label: &'static str,
    pub taps: usize,
    seen: Rc<RefCell<Vec<(&'static str, usize)>>>,
    probe: Probe<CarryState>,
}

impl Spec for Carry {
    type Props = CarryProps;
    type State = CarryState;

    fn create(props: &CarryProps) -> CarryState {
        CarryState { label: props.label, taps: 0, seen: props.seen.clone(), probe: props.probe.clone() }
    }

    fn adopt(state: &mut CarryState, props: &CarryProps) {
        state.label = props.label;
    }

    fn build<B: Backend>(state: &CarryState, this: Handle<CarryState>, _ctx: &BuildContext) -> impl Widget {
        state.seen.borrow_mut().push((state.label, state.taps));
        state.probe.capture(&this);
        LeafProbe
    }
}

/// Routes events to a child that counts them.
pub struct Event;

pub struct EventProps {
    pub events: Rc<Cell<usize>>,
}

pub struct EventState {
    events: Rc<Cell<usize>>,
}

impl Spec for Event {
    type Props = EventProps;
    type State = EventState;

    fn create(props: &EventProps) -> EventState {
        EventState { events: props.events.clone() }
    }

    fn build<B: Backend>(state: &EventState, _this: Handle<EventState>, _ctx: &BuildContext) -> impl Widget {
        EventProbeChild { events: state.events.clone() }
    }
}

/// Panics where asked to, so each phase's recovery can be observed.
pub struct Lifecycle;

pub struct LifecycleProps {
    pub phase: PanicPhase,
    pub probe: Probe<LifecycleState>,
    pub builds: Rc<Cell<usize>>,
}

pub struct LifecycleState {
    pub phase: PanicPhase,
    probe: Probe<LifecycleState>,
    builds: Rc<Cell<usize>>,
}

/// Fresh props with their own probe and build counter.
pub fn lifecycle(phase: PanicPhase) -> LifecycleProps {
    LifecycleProps { phase, probe: Probe::default(), builds: Rc::default() }
}

impl Spec for Lifecycle {
    type Props = LifecycleProps;
    type State = LifecycleState;

    fn create(props: &LifecycleProps) -> LifecycleState {
        if props.phase == PanicPhase::CreateState {
            panic!("state creation failed");
        }
        LifecycleState { phase: props.phase, probe: props.probe.clone(), builds: props.builds.clone() }
    }

    fn init(state: &mut LifecycleState) {
        if state.phase == PanicPhase::InitState {
            panic!("state initialization failed");
        }
    }

    fn adopt(state: &mut LifecycleState, props: &LifecycleProps) {
        if props.phase == PanicPhase::AdoptConfig {
            panic!("keyed configuration failed");
        }
        state.phase = props.phase;
    }

    fn build<B: Backend>(
        state: &LifecycleState,
        this: Handle<LifecycleState>,
        _ctx: &BuildContext,
    ) -> impl Widget {
        state.probe.capture(&this);
        state.builds.set(state.builds.get() + 1);
        if state.phase == PanicPhase::Build {
            panic!("state build failed");
        }
        LifecycleChild {
            panic_in_to_element: state.phase == PanicPhase::ToElement,
            panic_in_runtime_adoption: state.phase == PanicPhase::ChildRuntimeAdoption,
        }
    }
}

/// Two stateful specs that share a name but not a state type.
pub struct Alpha;
pub struct Beta;

pub struct PlainProps {
    pub probe: Probe<PlainState>,
}

pub struct PlainState {
    probe: Probe<PlainState>,
}

impl Spec for Alpha {
    type Props = PlainProps;
    type State = PlainState;

    fn create(props: &PlainProps) -> PlainState {
        PlainState { probe: props.probe.clone() }
    }

    fn build<B: Backend>(state: &PlainState, this: Handle<PlainState>, _ctx: &BuildContext) -> impl Widget {
        state.probe.capture(&this);
        LeafProbe
    }
}

pub struct BetaProps {
    pub probe: Probe<BetaState>,
}

pub struct BetaState {
    probe: Probe<BetaState>,
}

impl Spec for Beta {
    type Props = BetaProps;
    type State = BetaState;

    fn create(props: &BetaProps) -> BetaState {
        BetaState { probe: props.probe.clone() }
    }

    fn build<B: Backend>(state: &BetaState, this: Handle<BetaState>, _ctx: &BuildContext) -> impl Widget {
        state.probe.capture(&this);
        LeafProbe
    }
}

/// A state whose value tests read and mutate across an unmount.
pub struct Stale;

pub struct StaleProps {
    pub probe: Probe<StaleState>,
}

pub struct StaleState {
    pub value: String,
    probe: Probe<StaleState>,
}

impl Spec for Stale {
    type Props = StaleProps;
    type State = StaleState;

    fn create(props: &StaleProps) -> StaleState {
        StaleState { value: String::from("live"), probe: props.probe.clone() }
    }

    fn build<B: Backend>(state: &StaleState, this: Handle<StaleState>, _ctx: &BuildContext) -> impl Widget {
        state.probe.capture(&this);
        LeafProbe
    }
}

/// An unkeyed stateful widget that records which of its states built the
/// subtree that is live. Each state takes the next identity from `next_id`, so
/// a state created by a rebuild is distinguishable from the one it replaces.
pub struct Inner;

#[derive(Clone)]
pub struct InnerProps {
    pub next_id: Rc<Cell<usize>>,
    pub built_by: Rc<Cell<usize>>,
}

pub struct InnerState {
    id: usize,
    built_by: Rc<Cell<usize>>,
}

impl Spec for Inner {
    type Props = InnerProps;
    type State = InnerState;

    fn create(props: &InnerProps) -> InnerState {
        let id = props.next_id.get();
        props.next_id.set(id + 1);
        InnerState { id, built_by: props.built_by.clone() }
    }

    fn build<B: Backend>(state: &InnerState, _this: Handle<InnerState>, _ctx: &BuildContext) -> impl Widget {
        state.built_by.set(state.id);
        LeafProbe
    }
}

/// One of two interchangeable "pages" below a keyed element, distinguished only
/// by the name it builds under, the shape a route's pages have.
#[derive(Clone)]
pub struct ProbePage {
    name: &'static str,
    inner: InnerProps,
    make: fn(InnerProps, &BuildContext) -> AnyElement,
}

pub fn probe_page<B: Backend>(name: &'static str, inner: InnerProps) -> ProbePage {
    ProbePage {
        name,
        inner,
        make: |props, ctx| B::element::<Inner>(props, ctx, "InnerProbe", None),
    }
}

impl Widget for ProbePage {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let (inner, make) = (self.inner, self.make);
        StatelessElement::from_builder(
            ctx,
            // The builder may run again on every rebuild, so each build needs
            // its own copy of the props the conversion consumes.
            move |ctx| make(inner.clone(), ctx),
            None,
            self.name,
        )
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        self.name
    }
}

/// A keyed stateful widget standing where a route transition stands: it keeps
/// its own state by key, and everything below it does not.
pub struct KeyedOuter;

pub struct KeyedOuterProps {
    pub page: ProbePage,
}

pub struct KeyedOuterState {
    page: ProbePage,
}

impl Spec for KeyedOuter {
    type Props = KeyedOuterProps;
    type State = KeyedOuterState;

    fn create(props: &KeyedOuterProps) -> KeyedOuterState {
        KeyedOuterState { page: props.page.clone() }
    }

    fn adopt(state: &mut KeyedOuterState, props: &KeyedOuterProps) {
        state.page = props.page.clone();
    }

    fn build<B: Backend>(
        state: &KeyedOuterState,
        _this: Handle<KeyedOuterState>,
        _ctx: &BuildContext,
    ) -> impl Widget {
        state.page.clone()
    }
}

/// A stateful widget living inside another stateful widget's build output.
pub struct Nested;

pub struct NestedProps {
    pub last_built_value: Rc<Cell<usize>>,
    pub probe: Probe<NestedState>,
}

pub struct NestedState {
    pub value: usize,
    last_built_value: Rc<Cell<usize>>,
    probe: Probe<NestedState>,
}

impl Spec for Nested {
    type Props = NestedProps;
    type State = NestedState;

    fn create(props: &NestedProps) -> NestedState {
        NestedState { value: 0, last_built_value: props.last_built_value.clone(), probe: props.probe.clone() }
    }

    fn build<B: Backend>(state: &NestedState, this: Handle<NestedState>, _ctx: &BuildContext) -> impl Widget {
        state.last_built_value.set(state.value);
        state.probe.capture(&this);
        LeafProbe
    }
}

pub struct NestingParent;

pub struct NestingParentProps {
    pub nested_value: Rc<Cell<usize>>,
    pub nested_probe: Probe<NestedState>,
    pub probe: Probe<NestingParentState>,
}

pub struct NestingParentState {
    pub taps: usize,
    nested_value: Rc<Cell<usize>>,
    nested_probe: Probe<NestedState>,
    probe: Probe<NestingParentState>,
}

impl Spec for NestingParent {
    type Props = NestingParentProps;
    type State = NestingParentState;

    fn create(props: &NestingParentProps) -> NestingParentState {
        NestingParentState {
            taps: 0,
            nested_value: props.nested_value.clone(),
            nested_probe: props.nested_probe.clone(),
            probe: props.probe.clone(),
        }
    }

    fn build<B: Backend>(
        state: &NestingParentState,
        this: Handle<NestingParentState>,
        _ctx: &BuildContext,
    ) -> impl Widget {
        state.probe.capture(&this);
        // The nested child does not depend on `taps`.
        nest::<B, Nested>(
            NestedProps { last_built_value: state.nested_value.clone(), probe: state.nested_probe.clone() },
            "Nested",
        )
    }
}

/// A chain of nested stateful widgets, `depth` levels deep, counting every
/// create / init / build so a backend's re-emission cost can be read off.
pub struct Chain;

#[derive(Default)]
pub struct ChainCounts {
    pub creates: Cell<u32>,
    pub inits: Cell<u32>,
    pub builds: Cell<u32>,
    /// The latest handle of the level at each depth (index 0 is depth 1).
    pub handles: RefCell<Vec<Option<Handle<ChainState>>>>,
}

impl ChainCounts {
    pub fn take(&self) -> (u32, u32, u32) {
        (self.creates.take(), self.inits.take(), self.builds.take())
    }

    /// The handle of the level `depth` deep (1 is the innermost).
    pub fn handle(&self, depth: usize) -> Handle<ChainState> {
        self.handles.borrow()[depth - 1].clone().expect("that level has built")
    }
}

pub struct ChainProps {
    pub depth: usize,
    pub counts: Rc<ChainCounts>,
}

pub struct ChainState {
    depth: usize,
    counts: Rc<ChainCounts>,
}

impl Spec for Chain {
    type Props = ChainProps;
    type State = ChainState;

    fn create(props: &ChainProps) -> ChainState {
        props.counts.creates.set(props.counts.creates.get() + 1);
        ChainState { depth: props.depth, counts: props.counts.clone() }
    }

    fn init(state: &mut ChainState) {
        state.counts.inits.set(state.counts.inits.get() + 1);
    }

    fn adopt(state: &mut ChainState, props: &ChainProps) {
        state.depth = props.depth;
    }

    fn build<B: Backend>(state: &ChainState, this: Handle<ChainState>, _ctx: &BuildContext) -> impl Widget {
        state.counts.builds.set(state.counts.builds.get() + 1);
        {
            let mut handles = state.counts.handles.borrow_mut();
            if handles.len() < state.depth {
                handles.resize(state.depth, None);
            }
            handles[state.depth - 1] = Some(this);
        }
        if state.depth > 1 {
            nest::<B, Chain>(ChainProps { depth: state.depth - 1, counts: state.counts.clone() }, "Chain").boxed()
        } else {
            LeafProbe.boxed()
        }
    }
}
