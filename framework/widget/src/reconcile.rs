use crate::base::BuildContext;
use crate::components::element::Element;

/// Collect an element's children as exposed for event dispatch. This is the
/// traversal `carry_child_state` walks to reach nested state-owning elements
/// (e.g. a `Scrollable` inside a `Container`), so single-child wrappers must
/// surface their child here for scroll state to survive an ancestor rebuild.
#[allow(clippy::needless_lifetimes, unused)]
fn event_children_of<'a>(element: &'a dyn Element) -> Vec<&'a dyn Element> {
    let mut children: Vec<&dyn Element> = Vec::new();
    element.event_children(&mut |c| children.push(c));
    children
}

/// Wrapper element that mimics `RawScrollableContainer`: its `event_children`
/// is intentionally empty (events handled via `on_event`) but its
/// `update_from_widget` recurses into the child via `try_update_element` —
/// the shape used by the framework's only built-in scroll container.
#[allow(unused)]
struct ScrollableLikeWrapper {
    child: Box<dyn Element>,
    key: Option<crate::Key>,
}

impl crate::VisitorElement for ScrollableLikeWrapper {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
    fn debug_name(&self) -> &'static str {
        "ScrollableLikeWrapper"
    }
}

impl crate::EventElement for ScrollableLikeWrapper {
    fn event_children<'a>(&'a self, _: &mut dyn FnMut(&'a dyn Element)) {}
}

impl crate::Drawable for ScrollableLikeWrapper {
    fn update(&self, _ctx: &BuildContext) {}
}

impl crate::LayoutElement for ScrollableLikeWrapper {}

impl crate::Rebuildable for ScrollableLikeWrapper {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::event_element::EventElement;
    use crate::components::layout_element::LayoutElement;
    use crate::components::rebuildable::Rebuildable;
    use crate::components::visitor_element::VisitorElement;
    use crate::{AnyElement, Drawable};

    // Minimal fake elements: all trait methods except the listed ones use defaults.
    struct Leaf(&'static str);
    impl VisitorElement for Leaf {
        fn debug_name(&self) -> &'static str {
            self.0
        }
    }
    impl Drawable for Leaf {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl EventElement for Leaf {}
    impl LayoutElement for Leaf {}
    impl Rebuildable for Leaf {}
    // A single-child wrapper (like `Container`) that surfaces its child only
    // through `visit_children` — exactly the shape that previously hid the
    // nested scrollable from reconciliation.
    struct Wrapper(AnyElement);
    impl VisitorElement for Wrapper {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.0.as_ref());
        }
        fn debug_name(&self) -> &'static str {
            "Wrapper"
        }
    }
    impl Drawable for Wrapper {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl EventElement for Wrapper {}
    impl LayoutElement for Wrapper {}
    impl Rebuildable for Wrapper {}

    struct Branches(Vec<AnyElement>);
    impl VisitorElement for Branches {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            for child in &self.0 {
                visitor(child.as_ref());
            }
        }
        fn debug_name(&self) -> &'static str {
            "Branches"
        }
    }
    impl Drawable for Branches {
        fn update(&self, _: &BuildContext) {}
    }
    impl EventElement for Branches {}
    impl LayoutElement for Branches {}
    impl Rebuildable for Branches {}

    struct SplitTraversal {
        event_child: AnyElement,
        visual_child: AnyElement,
    }
    impl VisitorElement for SplitTraversal {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.visual_child.as_ref());
        }
        fn debug_name(&self) -> &'static str {
            "SplitTraversal"
        }
    }
    impl Drawable for SplitTraversal {
        fn update(&self, _: &BuildContext) {}
    }
    impl EventElement for SplitTraversal {
        fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.event_child.as_ref());
        }
    }
    impl LayoutElement for SplitTraversal {}
    impl Rebuildable for SplitTraversal {}
    // Contract the carry-over relies on: a wrapper that only implements
    // `visit_children` still surfaces its child through `event_children` (via the
    // default delegation), so `carry_child_state` can descend to a nested
    // state-owning element and hand its scroll offset to the rebuilt one.
    #[test]
    fn event_children_reach_wrapper_child() {
        let wrapper = Wrapper(Leaf("Scrollable").boxed());
        let children = event_children_of(&wrapper);
        assert_eq!(children.len(), 1, "wrapper must expose its single child");
        assert_eq!(children[0].debug_name(), "Scrollable");
    }

    // ─── StatefulElement state carry on reconcile ─────────────────────────
    //
    // The reconcile chain `StatelessElement::rebuild_if_dirty → try_update_element`
    // is the only thing that preserves a `StatefulElement`'s live state cell
    // across a window-resize rebuild. The current `try_update_element` early-
    // returns on key mismatch (`reconcile.rs:23`) BEFORE calling
    // `update_from_widget` → `adopt_state_from`, so a resize that produces a
    // freshly-built `StatefulElement` with a different key silently drops the
    // user's state.

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::Widget;
    use crate::key::Key;
    use crate::widget::stateful::{
        State, StateUpdater, StatefulElement, StatefulWidget, carry_child_state,
    };

    /// A no-op widget used as the inner child of a `StatefulElement` for tests.
    /// Its `to_element` does not touch `ctx` fields beyond borrowing them, so
    /// the dummy `BuildContext` is safe.
    struct EmptyWidget;
    impl Widget for EmptyWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            EmptyLeaf.boxed()
        }
        fn debug_name(&self) -> &'static str {
            "EmptyWidget"
        }
    }

    impl crate::widget::PortableWidget for EmptyWidget {}

    struct EmptyLeaf;
    impl VisitorElement for EmptyLeaf {
        fn debug_name(&self) -> &'static str {
            "EmptyLeaf"
        }
    }
    impl Drawable for EmptyLeaf {
        fn update(&self, _: &BuildContext) {}
    }
    impl EventElement for EmptyLeaf {}
    impl LayoutElement for EmptyLeaf {}
    impl Rebuildable for EmptyLeaf {}

    /// Adapts an already-constructed `AnyElement` into a `Widget` so a
    /// `State::build` (which must return `impl Widget`) can hand back a subtree
    /// that was assembled directly from elements.
    ///
    /// The element is simply moved out on conversion: `Widget::to_element`
    /// consumes the widget, so the type system already guarantees the single
    /// hand-over this adapter needs — no cell and no run-time guard.
    struct ElementWidget {
        element: AnyElement,
    }
    impl ElementWidget {
        fn new(element: AnyElement) -> Self {
            Self { element }
        }
    }
    impl Widget for ElementWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            self.element
        }
        fn debug_name(&self) -> &'static str {
            "ElementWidget"
        }
    }

    impl crate::widget::PortableWidget for ElementWidget {}

    /// A minimal `StatefulWidget` for reconcile tests. The state struct
    /// holds a counter and a shared observer that records the most recent
    /// counter value `build()` saw — this is the only way to observe what
    /// `rebuild_fn` actually reads from the state cell after `adopt_state_from`
    /// swaps it.
    #[allow(unused)]
    struct CounterWidget {
        observer: Rc<Cell<usize>>,
    }
    #[allow(unused)]
    struct CounterState {
        counter: usize,
        observer: Rc<Cell<usize>>,
        #[allow(dead_code)]
        updater: StateUpdater<Self>,
    }
    impl StatefulWidget for CounterWidget {
        type State = CounterState;
        fn create_state(self) -> Self::State {
            CounterState {
                counter: 1,
                observer: self.observer.clone(),
                updater: StateUpdater::new(),
            }
        }
    }
    impl State<CounterWidget> for CounterState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }
        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            // Record the counter value the rebuild closure actually saw. This
            // is the only observable proof of which state cell a `StatefulElement`
            // is reading from after `adopt_state_from` rewires its `rebuild_fn`.
            self.observer.set(self.counter);
            EmptyWidget
        }
    }

    struct CounterParentWidget {
        observer: Rc<Cell<usize>>,
    }

    struct CounterParentState {
        observer: Rc<Cell<usize>>,
        updater: StateUpdater<Self>,
    }

    impl StatefulWidget for CounterParentWidget {
        type State = CounterParentState;

        fn create_state(self) -> Self::State {
            CounterParentState {
                observer: self.observer.clone(),
                updater: StateUpdater::new(),
            }
        }
    }

    impl State<CounterParentWidget> for CounterParentState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }

        fn build(&self, ctx: &BuildContext) -> impl Widget {
            let widget = CounterWidget {
                observer: self.observer.clone(),
            };
            let (child, _) = StatefulElement::new_with_name(
                widget,
                ctx,
                "Counter",
                Some(Key::Static("nested-counter")),
            );
            ElementWidget::new(child.boxed())
        }
    }

    struct BuildRecordingCounterWidget {
        builds: Rc<RefCell<Vec<usize>>>,
        updater: Rc<RefCell<Option<StateUpdater<BuildRecordingCounterState>>>>,
    }

    struct BuildRecordingCounterState {
        counter: usize,
        builds: Rc<RefCell<Vec<usize>>>,
        published_updater: Rc<RefCell<Option<StateUpdater<Self>>>>,
        updater: StateUpdater<Self>,
    }

    impl StatefulWidget for BuildRecordingCounterWidget {
        type State = BuildRecordingCounterState;

        fn create_state(self) -> Self::State {
            BuildRecordingCounterState {
                counter: 0,
                builds: self.builds.clone(),
                published_updater: self.updater.clone(),
                updater: StateUpdater::new(),
            }
        }
    }

    impl State<BuildRecordingCounterWidget> for BuildRecordingCounterState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
            *self.published_updater.borrow_mut() = Some(updater);
        }

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            self.builds.borrow_mut().push(self.counter);
            EmptyWidget
        }
    }

    struct BuildRecordingParentWidget {
        builds: Rc<RefCell<Vec<usize>>>,
        child_updater: Rc<RefCell<Option<StateUpdater<BuildRecordingCounterState>>>>,
    }

    struct BuildRecordingParentState {
        builds: Rc<RefCell<Vec<usize>>>,
        child_updater: Rc<RefCell<Option<StateUpdater<BuildRecordingCounterState>>>>,
        updater: StateUpdater<Self>,
    }

    impl StatefulWidget for BuildRecordingParentWidget {
        type State = BuildRecordingParentState;

        fn create_state(self) -> Self::State {
            BuildRecordingParentState {
                builds: self.builds.clone(),
                child_updater: self.child_updater.clone(),
                updater: StateUpdater::new(),
            }
        }
    }

    impl State<BuildRecordingParentWidget> for BuildRecordingParentState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }

        fn build(&self, ctx: &BuildContext) -> impl Widget {
            let child = BuildRecordingCounterWidget {
                builds: self.builds.clone(),
                updater: self.child_updater.clone(),
            };
            let (element, _) = StatefulElement::new_with_name(
                child,
                ctx,
                "BuildRecordingCounter",
                Some(Key::Static("build-recording-counter")),
            );
            ElementWidget::new(element.boxed())
        }
    }
    #[allow(unused)]
    struct ConfigWidget {
        label: usize,
        observed_label: Rc<Cell<usize>>,
        observed_runtime: Rc<Cell<usize>>,
        config_adoptions: Rc<Cell<usize>>,
        live_updater: Rc<RefCell<Option<StateUpdater<ConfigState>>>>,
    }

    struct ConfigState {
        config_label: usize,
        runtime: usize,
        observed_label: Rc<Cell<usize>>,
        observed_runtime: Rc<Cell<usize>>,
        config_adoptions: Rc<Cell<usize>>,
        live_updater: Rc<RefCell<Option<StateUpdater<Self>>>>,
        updater: StateUpdater<Self>,
    }

    impl StatefulWidget for ConfigWidget {
        type State = ConfigState;

        fn create_state(self) -> Self::State {
            ConfigState {
                config_label: self.label,
                runtime: 0,
                observed_label: self.observed_label.clone(),
                observed_runtime: self.observed_runtime.clone(),
                config_adoptions: self.config_adoptions.clone(),
                live_updater: self.live_updater.clone(),
                updater: StateUpdater::new(),
            }
        }
    }

    impl State<ConfigWidget> for ConfigState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }

        // Config = `config_label`; runtime = `runtime`. On reconcile the
        // framework preserves this live state but must refresh the config from
        // the freshly-built widget.
        fn adopt_config_from(&mut self, new: Self) {
            self.config_adoptions.set(self.config_adoptions.get() + 1);
            self.config_label = new.config_label;
        }

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            self.observed_label.set(self.config_label);
            self.observed_runtime.set(self.runtime);
            *self.live_updater.borrow_mut() = Some(self.updater);
            EmptyWidget
        }
    }
    #[allow(unused)]
    fn current_config_updater(
        live_updater: &Rc<RefCell<Option<StateUpdater<ConfigState>>>>,
    ) -> StateUpdater<ConfigState> {
        live_updater
            .borrow()
            .as_ref()
            .copied()
            .expect("live updater should be published from build()")
    }

    /// A tokio runtime handle for the non-wasm `async_handle` field. Built
    /// once via `OnceLock` so the leaked runtime outlives every test that
    /// constructs a `BuildContext`.
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
        // SAFETY: see `dummy_window`. `InnerCanvas::new()` requires no GPU;
        // leaked so the canvas reference has the `'static` lifetime
        // `BuildContext` demands.
        let canvas = {
            let leaked: &'static aimer_canvas::InnerCanvas =
                Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(leaked)
        };
        BuildContext::new(
            canvas,
            Default::default(),
            1.0,
            Default::default(),
            Default::default(),
            crate::base::WindowHandle::headless(Default::default(), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            dummy_async_handle(),
        )
    }

    // ─── Draw-path regression: the DRAWN subtree must reflect set_state ────
    //
    // Every other stateful test asserts on a BUILD observer (what `build()`
    // saw). That leaves a gap: none of them check that the element actually
    // *drawn* after a `set_state` is the freshly-rebuilt one. The reported
    // "counter freezes on screen while `self.count` keeps incrementing in the
    // console" symptom lives precisely in that gap — `build()` runs with the
    // new value, yet if the stale child subtree were drawn the screen would
    // stay frozen.
    //
    // This mirrors the real app: a stateful counter whose `build()` returns a
    // container -> row -> [text-leaf, nested stateful button] tree, then fires
    // `set_state` through the state's own updater exactly like the button's
    // `on_press`, and asserts the DRAWN leaf shows the new value.

    /// Leaf that records the value it renders when DRAWN (not when built),
    /// so the test can prove which element actually reaches the screen.
    struct RecordingLeaf {
        value: usize,
        drawn: Rc<Cell<usize>>,
    }
    impl VisitorElement for RecordingLeaf {
        fn debug_name(&self) -> &'static str {
            "RecordingLeaf"
        }
    }
    impl Drawable for RecordingLeaf {
        fn update(&self, _ctx: &BuildContext) {
            self.drawn.set(self.value);
        }
    }
    impl EventElement for RecordingLeaf {}
    impl LayoutElement for RecordingLeaf {}
    impl Rebuildable for RecordingLeaf {}

    /// Single-child wrapper that DRAWS its child (like `Container`) and is
    /// always replaced on reconcile (`update_from_widget` -> false, default).
    struct DrawWrapper(AnyElement);
    impl VisitorElement for DrawWrapper {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.0.as_ref());
        }
        fn debug_name(&self) -> &'static str {
            "DrawWrapper"
        }
    }
    impl Drawable for DrawWrapper {
        fn update(&self, ctx: &BuildContext) {
            self.0.update(ctx);
        }
    }
    impl EventElement for DrawWrapper {
        fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.0.as_ref());
        }
    }
    impl LayoutElement for DrawWrapper {}
    impl Rebuildable for DrawWrapper {}
    /// Multi-child wrapper that DRAWS every child in order (like `Flex`).
    struct DrawRow(Vec<AnyElement>);
    impl VisitorElement for DrawRow {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            for c in &self.0 {
                visitor(c.as_ref());
            }
        }
        fn debug_name(&self) -> &'static str {
            "DrawRow"
        }
    }
    impl Drawable for DrawRow {
        fn update(&self, ctx: &BuildContext) {
            for c in &self.0 {
                c.update(ctx);
            }
        }
    }
    impl EventElement for DrawRow {
        fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            for c in &self.0 {
                visitor(c.as_ref());
            }
        }
    }
    impl LayoutElement for DrawRow {}
    impl Rebuildable for DrawRow {}

    /// A nested stateful widget standing in for the `Button` sibling, so the
    /// counter's rebuild has to reconcile a real `StatefulElement` sibling
    /// alongside the text leaf (matching jaime's flex children).
    struct NestedButtonWidget;
    struct NestedButtonState {
        #[allow(dead_code)]
        updater: StateUpdater<Self>,
    }
    impl StatefulWidget for NestedButtonWidget {
        type State = NestedButtonState;
        fn create_state(self) -> Self::State {
            NestedButtonState {
                updater: StateUpdater::new(),
            }
        }
    }
    impl State<NestedButtonWidget> for NestedButtonState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }
        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            EmptyWidget
        }
    }

    struct DrawCounterWidget {
        drawn: Rc<Cell<usize>>,
        live_updater: Rc<RefCell<Option<StateUpdater<DrawCounterState>>>>,
    }
    struct DrawCounterState {
        counter: usize,
        drawn: Rc<Cell<usize>>,
        live_updater: Rc<RefCell<Option<StateUpdater<Self>>>>,
        updater: StateUpdater<Self>,
    }
    impl StatefulWidget for DrawCounterWidget {
        type State = DrawCounterState;
        fn create_state(self) -> Self::State {
            DrawCounterState {
                counter: 1,
                drawn: self.drawn.clone(),
                live_updater: self.live_updater.clone(),
                updater: StateUpdater::new(),
            }
        }
    }
    impl State<DrawCounterWidget> for DrawCounterState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }
        fn build(&self, ctx: &BuildContext) -> impl Widget {
            *self.live_updater.borrow_mut() = Some(self.updater);

            // container -> row -> [ text-leaf(counter), nested stateful button ]
            let leaf: AnyElement = RecordingLeaf {
                value: self.counter,
                drawn: self.drawn.clone(),
            }
            .boxed();
            let (button, _ctor) =
                StatefulElement::new_with_name(NestedButtonWidget, ctx, "NestedButton", None);
            let row: AnyElement = DrawRow(vec![leaf, button.boxed()]).boxed();
            ElementWidget::new(DrawWrapper(row).boxed())
        }
    }

    #[test]
    fn set_state_updates_the_drawn_subtree() {
        let ctx = dummy_build_context();
        let drawn = Rc::new(Cell::new(0usize));
        let live_updater: Rc<RefCell<Option<StateUpdater<DrawCounterState>>>> =
            Rc::new(RefCell::new(None));

        let widget = DrawCounterWidget {
            drawn: drawn.clone(),
            live_updater: live_updater.clone(),
        };
        let (root, _ctor) = StatefulElement::new_with_name(widget, &ctx, "DrawCounter", None);

        // First frame: the initial state (counter = 1) is drawn.
        root.update(&ctx);
        assert_eq!(drawn.get(), 1, "initial draw must render counter = 1");

        // Fire a mutation through the state's OWN updater — exactly what the
        // `Increase` button's `on_press` does.
        live_updater
            .borrow()
            .as_ref()
            .expect("build() publishes the live updater")
            .set_state(|s| s.counter = 2);

        // Next frame: the DRAWN subtree must reflect the new value. If the old
        // child were kept/drawn, `drawn` would stay 1 — the on-screen freeze.
        root.update(&ctx);
        assert_eq!(
            drawn.get(),
            2,
            "after set_state the DRAWN subtree must render counter = 2"
        );
    }

    #[test]
    fn keyed_state_survives_a_responsive_wrapper_change() {
        let ctx = dummy_build_context();
        let observer = Rc::new(Cell::new(0usize));

        let old_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (old_stateful, old_updater) = StatefulElement::new_with_name(
            old_widget,
            &ctx,
            "Counter",
            Some(Key::Static("responsive-counter")),
        );
        old_updater.set_state(|state| state.counter = 7);
        old_stateful.rebuild_if_dirty(&ctx);
        assert_eq!(observer.get(), 7);

        let new_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (new_stateful, _) = StatefulElement::new_with_name(
            new_widget,
            &ctx,
            "Counter",
            Some(Key::Static("responsive-counter")),
        );
        assert_eq!(observer.get(), 1, "the replacement starts with fresh state");

        let old_tree = Wrapper(Wrapper(old_stateful.boxed()).boxed());
        let new_tree = Wrapper(new_stateful.boxed());
        carry_child_state(&old_tree, &new_tree, &ctx);

        assert_eq!(
            observer.get(),
            7,
            "a stable key must preserve state across wrapper changes"
        );
    }

    #[test]
    fn unkeyed_adoption_discards_the_candidate_updater_slot() {
        let ctx = dummy_build_context();
        let observer = Rc::new(Cell::new(0usize));

        let (old_stateful, old_updater) = StatefulElement::new_with_name(
            CounterWidget {
                observer: observer.clone(),
            },
            &ctx,
            "Counter",
            None,
        );
        old_updater.set_state(|state| state.counter = 7);
        old_stateful.rebuild_if_dirty(&ctx);

        let (replacement, candidate_updater) = StatefulElement::new_with_name(
            CounterWidget {
                observer: observer.clone(),
            },
            &ctx,
            "Counter",
            None,
        );
        assert_eq!(candidate_updater.try_read(|state| state.counter), Some(1));

        carry_child_state(&old_stateful, &replacement, &ctx);

        assert_eq!(observer.get(), 7);
        assert_eq!(old_updater.try_read(|state| state.counter), Some(7));
        assert_eq!(candidate_updater.try_read(|state| state.counter), None);
    }

    #[test]
    fn keyed_state_prefers_the_copy_with_the_latest_mutation() {
        let ctx = dummy_build_context();
        let observer = Rc::new(Cell::new(0usize));
        let key = Some(Key::Static("transition-retained-counter"));

        // Each element is built from its own widget: the conversion consumes
        // the configuration, and the three copies are interchangeable anyway.
        let widget = || CounterWidget {
            observer: observer.clone(),
        };
        let (stale_stateful, _) =
            StatefulElement::new_with_name(widget(), &ctx, "Counter", key.clone());
        let (live_stateful, live_updater) =
            StatefulElement::new_with_name(widget(), &ctx, "Counter", key.clone());
        live_updater.set_state(|state| state.counter = 7);
        live_stateful.rebuild_if_dirty(&ctx);

        let (replacement, _) = StatefulElement::new_with_name(widget(), &ctx, "Counter", key);
        let old_tree = Branches(vec![stale_stateful.boxed(), live_stateful.boxed()]);
        let new_tree = Wrapper(replacement.boxed());
        carry_child_state(&old_tree, &new_tree, &ctx);

        assert_eq!(
            observer.get(),
            7,
            "a retained stale branch must not overwrite more recently mutated keyed state"
        );
    }

    #[test]
    fn keyed_replacement_never_builds_fresh_state_before_adoption() {
        let ctx = dummy_build_context();
        let builds = Rc::new(RefCell::new(Vec::new()));
        let child_updater = Rc::new(RefCell::new(None));
        let parent_widget = BuildRecordingParentWidget {
            builds: builds.clone(),
            child_updater: child_updater.clone(),
        };
        let (parent, parent_updater) =
            StatefulElement::new_with_name(parent_widget, &ctx, "BuildRecordingParent", None);

        child_updater
            .borrow()
            .as_ref()
            .expect("child updater should be published during the initial build")
            .set_state(|state| state.counter = 2);
        parent_updater.set_state(|_| {});
        builds.borrow_mut().clear();

        parent.rebuild_if_dirty(&ctx);

        assert!(
            builds.borrow().iter().all(|value| *value == 2),
            "a keyed replacement must adopt live state before its first build: {:?}",
            builds.borrow().as_slice()
        );
    }

    #[test]
    fn keyed_state_rejects_an_older_copy_after_adopting_newer_state() {
        let ctx = dummy_build_context();
        let observer = Rc::new(Cell::new(0usize));
        let key = Some(Key::Static("transition-retained-counter"));
        // Each element is built from its own widget: the conversion consumes
        // the configuration, and the three copies are interchangeable anyway.
        let widget = || CounterWidget {
            observer: observer.clone(),
        };

        let (stale_stateful, _) =
            StatefulElement::new_with_name(widget(), &ctx, "Counter", key.clone());
        let (live_stateful, live_updater) =
            StatefulElement::new_with_name(widget(), &ctx, "Counter", key.clone());
        live_updater.set_state(|state| state.counter = 7);
        live_stateful.rebuild_if_dirty(&ctx);

        let (replacement, _) = StatefulElement::new_with_name(widget(), &ctx, "Counter", key);
        carry_child_state(&live_stateful, &replacement, &ctx);
        assert_eq!(observer.get(), 7);
        assert!(
            !replacement.is_dirty(),
            "eagerly materialized adopted state must not schedule a redundant rebuild"
        );

        carry_child_state(&stale_stateful, &replacement, &ctx);
        assert_eq!(
            observer.get(),
            7,
            "an older retained branch must not roll back state already adopted by the replacement"
        );
    }

    #[test]
    fn keyed_state_is_restored_after_a_stale_parent_is_adopted() {
        let ctx = dummy_build_context();
        let observer = Rc::new(Cell::new(0usize));
        // The stale and the replacement parent describe the same configuration,
        // but each conversion consumes the widget it is given.
        let parent_widget = || CounterParentWidget {
            observer: observer.clone(),
        };
        let (stale_parent, _) =
            StatefulElement::new_with_name(parent_widget(), &ctx, "CounterParent", None);

        let counter_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (live_counter, live_updater) = StatefulElement::new_with_name(
            counter_widget,
            &ctx,
            "Counter",
            Some(Key::Static("nested-counter")),
        );
        live_updater.set_state(|state| state.counter = 7);
        live_counter.rebuild_if_dirty(&ctx);

        let (replacement_parent, _) =
            StatefulElement::new_with_name(parent_widget(), &ctx, "CounterParent", None);
        let old_tree = Branches(vec![stale_parent.boxed(), live_counter.boxed()]);
        let new_tree = Wrapper(replacement_parent.boxed());
        carry_child_state(&old_tree, &new_tree, &ctx);

        assert_eq!(
            observer.get(),
            7,
            "keyed descendants must be restored after positional parent adoption"
        );
    }

    #[test]
    fn keyed_state_survives_moving_between_responsive_sibling_branches() {
        let ctx = dummy_build_context();
        let observer = Rc::new(Cell::new(0usize));

        let old_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (old_stateful, old_updater) = StatefulElement::new_with_name(
            old_widget,
            &ctx,
            "Counter",
            Some(Key::Static("responsive-counter")),
        );
        old_updater.set_state(|state| state.counter = 7);
        old_stateful.rebuild_if_dirty(&ctx);

        let new_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (new_stateful, _) = StatefulElement::new_with_name(
            new_widget,
            &ctx,
            "Counter",
            Some(Key::Static("responsive-counter")),
        );

        let old_tree = Branches(vec![
            Wrapper(EmptyLeaf.boxed()).boxed(),
            Wrapper(old_stateful.boxed()).boxed(),
        ]);
        let new_tree = Branches(vec![
            Wrapper(new_stateful.boxed()).boxed(),
            Wrapper(EmptyLeaf.boxed()).boxed(),
        ]);
        carry_child_state(&old_tree, &new_tree, &ctx);

        assert_eq!(
            observer.get(),
            7,
            "a stable key must survive responsive sibling movement"
        );
    }

    #[test]
    fn keyed_state_survives_when_a_new_wrapper_replaces_a_leaf() {
        let ctx = dummy_build_context();
        let observer = Rc::new(Cell::new(0usize));

        let old_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (old_stateful, old_updater) = StatefulElement::new_with_name(
            old_widget,
            &ctx,
            "Counter",
            Some(Key::Static("responsive-counter")),
        );
        old_updater.set_state(|state| state.counter = 7);
        old_stateful.rebuild_if_dirty(&ctx);

        let new_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (new_stateful, _) = StatefulElement::new_with_name(
            new_widget,
            &ctx,
            "Counter",
            Some(Key::Static("responsive-counter")),
        );

        let old_tree = Branches(vec![EmptyLeaf.boxed(), old_stateful.boxed()]);
        let new_tree = Branches(vec![
            Wrapper(new_stateful.boxed()).boxed(),
            EmptyLeaf.boxed(),
        ]);
        carry_child_state(&old_tree, &new_tree, &ctx);

        assert_eq!(
            observer.get(),
            7,
            "a keyed descendant must not depend on positional traversal"
        );
    }

    #[test]
    fn keyed_state_receives_changed_config_after_moving_between_sibling_branches() {
        let ctx = dummy_build_context();
        let observed_label = Rc::new(Cell::new(0));
        let observed_runtime = Rc::new(Cell::new(0));
        let config_adoptions = Rc::new(Cell::new(0));
        let live_updater = Rc::new(RefCell::new(None));

        let old_widget = ConfigWidget {
            label: 1,
            observed_label: observed_label.clone(),
            observed_runtime: observed_runtime.clone(),
            config_adoptions: config_adoptions.clone(),
            live_updater: live_updater.clone(),
        };
        let (old_stateful, old_updater) = StatefulElement::new_with_name(
            old_widget,
            &ctx,
            "Config",
            Some(Key::Static("moving-config")),
        );
        old_updater.set_state(|state| state.runtime = 9);
        old_stateful.rebuild_if_dirty(&ctx);

        let new_widget = ConfigWidget {
            label: 2,
            observed_label: observed_label.clone(),
            observed_runtime: observed_runtime.clone(),
            config_adoptions: config_adoptions.clone(),
            live_updater,
        };
        let (new_stateful, _) = StatefulElement::new_with_name(
            new_widget,
            &ctx,
            "Config",
            Some(Key::Static("moving-config")),
        );

        let old_tree = Branches(vec![
            Wrapper(EmptyLeaf.boxed()).boxed(),
            Wrapper(old_stateful.boxed()).boxed(),
        ]);
        let new_tree = Branches(vec![
            Wrapper(new_stateful.boxed()).boxed(),
            Wrapper(EmptyLeaf.boxed()).boxed(),
        ]);
        carry_child_state(&old_tree, &new_tree, &ctx);

        assert_eq!(
            config_adoptions.get(),
            1,
            "the moved live state must receive new config once"
        );
        assert_eq!(
            observed_label.get(),
            2,
            "the replacement config must reach the live state"
        );
        assert_eq!(
            observed_runtime.get(),
            9,
            "runtime state must survive config replacement"
        );
    }

    #[test]
    fn keyed_state_search_includes_visual_children_hidden_from_events() {
        let ctx = dummy_build_context();
        let observer = Rc::new(Cell::new(0usize));

        let old_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (old_stateful, old_updater) = StatefulElement::new_with_name(
            old_widget,
            &ctx,
            "Counter",
            Some(Key::Static("visual-counter")),
        );
        old_updater.set_state(|state| state.counter = 7);
        old_stateful.rebuild_if_dirty(&ctx);

        let new_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (new_stateful, _) = StatefulElement::new_with_name(
            new_widget,
            &ctx,
            "Counter",
            Some(Key::Static("visual-counter")),
        );

        let old_tree = SplitTraversal {
            event_child: EmptyLeaf.boxed(),
            visual_child: old_stateful.boxed(),
        };
        let new_tree = Wrapper(new_stateful.boxed());
        carry_child_state(&old_tree, &new_tree, &ctx);

        assert_eq!(
            observer.get(),
            7,
            "keyed state in the visual tree must be preserved"
        );
    }

    #[test]
    fn responsive_wrapper_change_does_not_adopt_a_different_key() {
        let ctx = dummy_build_context();
        let observer = Rc::new(Cell::new(0usize));

        let old_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (old_stateful, old_updater) = StatefulElement::new_with_name(
            old_widget,
            &ctx,
            "Counter",
            Some(Key::Static("old-counter")),
        );
        old_updater.set_state(|state| state.counter = 7);
        old_stateful.rebuild_if_dirty(&ctx);

        let new_widget = CounterWidget {
            observer: observer.clone(),
        };
        let (new_stateful, _) = StatefulElement::new_with_name(
            new_widget,
            &ctx,
            "Counter",
            Some(Key::Static("new-counter")),
        );

        let old_tree = Wrapper(Wrapper(old_stateful.boxed()).boxed());
        let new_tree = Wrapper(new_stateful.boxed());
        carry_child_state(&old_tree, &new_tree, &ctx);

        assert_eq!(
            observer.get(),
            1,
            "different keys must keep independent state"
        );
    }

    // ─── Router-shaped reconcile: keyed switcher behind a context Outlet ──
    //
    // Reproduces `website/src/router.rs`: an `AnimatedSwitcher` keyed
    // "route-switcher" is built by an `Outlet` that reads the active route's
    // transition key from a slot the `Shell` inserts into the `BuildContext`.
    // Navigation fires `set_state` on the top-level `Navigator`, whose rebuild
    // re-provides the slot with the new route key and reconciles the whole
    // shell subtree. The keyed switcher's `State` must be carried across that
    // rebuild so its `adopt_config_from` observes the changed child key and
    // starts the transition (here counted in `transitions`).

    #[derive(Clone)]
    struct RouteKeySlot(&'static str);

    struct SwitcherMock {
        child_key: &'static str,
        transitions: Rc<Cell<usize>>,
    }
    struct SwitcherMockState {
        child_key: &'static str,
        transitions: Rc<Cell<usize>>,
        #[allow(dead_code)]
        updater: StateUpdater<Self>,
    }
    impl StatefulWidget for SwitcherMock {
        type State = SwitcherMockState;
        fn create_state(self) -> Self::State {
            SwitcherMockState {
                child_key: self.child_key,
                transitions: self.transitions.clone(),
                updater: StateUpdater::new(),
            }
        }
    }
    impl State<SwitcherMock> for SwitcherMockState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }
        fn adopt_config_from(&mut self, new: Self) {
            if self.child_key != new.child_key {
                self.transitions.set(self.transitions.get() + 1);
                self.child_key = new.child_key;
            }
        }
        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            EmptyWidget
        }
    }
    impl Widget for SwitcherMock {
        fn key(&self) -> Option<Key> {
            Some(Key::Static("route-switcher"))
        }
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let __key = Widget::key(&self);
            StatefulElement::new_with_name(self, ctx, "AnimatedSwitcher", __key)
                .0
                .boxed()
        }
    }

    impl crate::widget::PortableWidget for SwitcherMock {}

    /// Reads the active route key from context (like `Outlet` reading its
    /// `OutletSlot`) and builds the keyed switcher from it.
    struct OutletMock {
        transitions: Rc<Cell<usize>>,
    }
    impl Widget for OutletMock {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let slot = ctx
                .get_state::<RouteKeySlot>()
                .expect("Shell must insert RouteKeySlot");
            SwitcherMock {
                child_key: slot.0,
                transitions: self.transitions.clone(),
            }
            .to_element(ctx)
        }
        fn debug_name(&self) -> &'static str {
            "Outlet"
        }
    }

    impl crate::widget::PortableWidget for OutletMock {}

    fn route_key(route: usize) -> &'static str {
        match route {
            0 => "home",
            1 => "docs",
            _ => "learn",
        }
    }

    struct NavMock {
        transitions: Rc<Cell<usize>>,
        header_observer: Rc<Cell<usize>>,
    }
    struct NavMockState {
        route: usize,
        transitions: Rc<Cell<usize>>,
        header_observer: Rc<Cell<usize>>,
        updater: StateUpdater<Self>,
    }
    impl StatefulWidget for NavMock {
        type State = NavMockState;
        fn create_state(self) -> Self::State {
            NavMockState {
                route: 0,
                transitions: self.transitions.clone(),
                header_observer: self.header_observer.clone(),
                updater: StateUpdater::new(),
            }
        }
    }
    impl State<NavMock> for NavMockState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }
        fn build(&self, ctx: &BuildContext) -> impl Widget {
            // Shell: publish the active route's transition key for the Outlet.
            ctx.insert_state(RouteKeySlot(route_key(self.route)));
            // Frame: a stateful header sibling + a content area holding the
            // Outlet, wrapped in container-like elements as in `AppShell`.
            let header = StatefulElement::new_with_name(
                CounterWidget {
                    observer: self.header_observer.clone(),
                },
                ctx,
                "Counter",
                None,
            )
            .0
            .boxed();
            let outlet = OutletMock {
                transitions: self.transitions.clone(),
            }
            .to_element(ctx);
            let content: AnyElement = DrawWrapper(DrawWrapper(outlet).boxed()).boxed();
            let frame: AnyElement = DrawRow(vec![header, content]).boxed();
            ElementWidget::new(frame)
        }
    }

    #[test]
    fn route_navigation_carries_keyed_switcher_and_starts_transition() {
        let ctx = dummy_build_context();
        let transitions = Rc::new(Cell::new(0usize));
        let header_observer = Rc::new(Cell::new(0usize));

        let (nav, updater) = StatefulElement::new_with_name(
            NavMock {
                transitions: transitions.clone(),
                header_observer: header_observer.clone(),
            },
            &ctx,
            "Navigator",
            None,
        );

        // Initial frame: switcher mounts on route "home"; no transition yet.
        nav.update(&ctx);
        assert_eq!(
            transitions.get(),
            0,
            "initial mount must not start a transition"
        );

        // Navigate home -> docs (like `Navigator::push`): the switcher's live
        // state must survive the shell rebuild and observe the changed key.
        updater.set_state(|s| s.route = 1);
        nav.rebuild_if_dirty(&ctx);

        assert_eq!(
            transitions.get(),
            1,
            "switching the route must carry the keyed switcher state and start its transition"
        );
    }

    /// A marker inherited state a provider inserts into the `BuildContext`
    /// during its own `build` — the test analogue of the `Navigator` inserting
    /// its `NavigatorController`.
    #[derive(Clone, Copy)]
    struct ProvidedValue;

    /// A `StatefulWidget` that provides an inherited value during `build`
    /// (mirroring `NavigatorState::build` inserting its controller) and renders
    /// a nested `ConsumerWidget` below it.
    struct ProviderWidget;
    struct ProviderState {
        #[allow(dead_code)]
        updater: StateUpdater<Self>,
    }
    impl StatefulWidget for ProviderWidget {
        type State = ProviderState;
        fn create_state(self) -> Self::State {
            ProviderState {
                updater: StateUpdater::new(),
            }
        }
    }
    impl State<ProviderWidget> for ProviderState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }
        fn build(&self, ctx: &BuildContext) -> impl Widget {
            // Provide the inherited value *before* building the child subtree —
            // exactly what `NavigatorState::build` does with its controller.
            ctx.insert_state(ProvidedValue);
            let (consumer, _u) = StatefulElement::new(ConsumerWidget, ctx);
            ElementWidget::new(consumer.boxed())
        }
    }

    /// A nested `StatefulWidget` that reads the ancestor-provided inherited
    /// value during `build`, panicking if it is absent — mirroring
    /// `NavigatorController::of`, which panics with "No Navigator found in
    /// context" when the controller has not been re-provided this frame.
    struct ConsumerWidget;
    struct ConsumerState {
        #[allow(dead_code)]
        updater: StateUpdater<Self>,
    }
    impl StatefulWidget for ConsumerWidget {
        type State = ConsumerState;
        fn create_state(self) -> Self::State {
            ConsumerState {
                updater: StateUpdater::new(),
            }
        }
    }
    impl State<ConsumerWidget> for ConsumerState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }
        fn build(&self, ctx: &BuildContext) -> impl Widget {
            ctx.get_state::<ProvidedValue>().expect(
                "No provided value in context. Ancestor provider must build before descendant.",
            );
            EmptyWidget
        }
    }

    /// Regression for the reported "app panics when resizing the window: No
    /// Navigator found in context" bug.
    ///
    /// On a window resize the whole element tree is marked dirty
    /// (`mark_needs_rebuild`) and rebuilt in a single draw pass against a FRESH
    /// `BuildContext` whose `inherited_states` map starts empty. A dirty
    /// `StatefulElement` that provides inherited state in its own `build` (e.g.
    /// `Navigator` inserting its `NavigatorController`) must re-provide that
    /// state BEFORE its descendants rebuild.
    ///
    /// The bug: `rebuild_if_dirty` propagated the rebuild down to the existing
    /// child subtree *before* running its own `build`, so a nested consumer (a
    /// header calling `NavigatorController::of`) rebuilt against the empty
    /// fresh context and panicked. This test drives that exact ordering and
    /// must not panic.
    #[test]
    fn provider_reprovides_inherited_state_before_children_rebuild_on_resize() {
        // Initial build: provider inserts the value, the consumer reads it.
        let ctx = dummy_build_context();
        let (provider, _u) = StatefulElement::new(ProviderWidget, &ctx);

        // Simulate a resize: dirty the whole subtree, then rebuild it against a
        // brand-new context whose inherited_states map is empty (a new frame).
        Rebuildable::mark_needs_rebuild(&provider);
        let fresh_ctx = dummy_build_context();

        // Must not panic: the provider's own build (which re-inserts the value)
        // must run before the consumer subtree rebuilds and looks it up.
        provider.rebuild_if_dirty(&fresh_ctx);
    }

    mod resize_repro {
        use std::cell::{Cell, RefCell};

        use aimer_attribute::size::ResolvedSize;

        use super::*;
        use crate::widget::stateless::StatelessElement;

        struct ResizeCounterWidget {
            observer: Rc<Cell<usize>>,
            live_updater: Rc<RefCell<Option<StateUpdater<ResizeCounterState>>>>,
        }

        struct ResizeCounterState {
            counter: usize,
            observer: Rc<Cell<usize>>,
            live_updater: Rc<RefCell<Option<StateUpdater<Self>>>>,
            updater: StateUpdater<Self>,
        }

        impl StatefulWidget for ResizeCounterWidget {
            type State = ResizeCounterState;

            fn create_state(self) -> Self::State {
                ResizeCounterState {
                    counter: 1,
                    observer: self.observer.clone(),
                    live_updater: self.live_updater.clone(),
                    updater: StateUpdater::new(),
                }
            }
        }

        impl State<ResizeCounterWidget> for ResizeCounterState {
            fn init_state(&mut self, updater: StateUpdater<Self>) {
                self.updater = updater;
            }

            fn build(&self, _ctx: &BuildContext) -> impl Widget {
                self.observer.set(self.counter);
                *self.live_updater.borrow_mut() = Some(self.updater);
                EmptyWidget
            }
        }

        struct FakeLeaf {
            name: &'static str,
            size: ResolvedSize,
        }

        impl FakeLeaf {
            fn new(name: &'static str, width: f32, height: f32) -> Self {
                Self {
                    name,
                    size: ResolvedSize { width, height },
                }
            }
        }

        impl VisitorElement for FakeLeaf {
            fn debug_name(&self) -> &'static str {
                self.name
            }
        }

        impl Drawable for FakeLeaf {
            fn update(&self, _ctx: &BuildContext) {}
        }

        impl EventElement for FakeLeaf {}

        impl LayoutElement for FakeLeaf {
            fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
                self.size
            }
        }

        impl Rebuildable for FakeLeaf {}

        struct FakeContainer {
            child: AnyElement,
            size: ResolvedSize,
        }

        impl FakeContainer {
            fn new(child: AnyElement, width: f32, height: f32) -> Self {
                Self {
                    child,
                    size: ResolvedSize { width, height },
                }
            }
        }

        impl VisitorElement for FakeContainer {
            fn debug_name(&self) -> &'static str {
                "FakeContainer"
            }
        }

        impl Drawable for FakeContainer {
            fn update(&self, ctx: &BuildContext) {
                self.child.update(ctx);
            }
        }

        impl EventElement for FakeContainer {
            fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                visitor(self.child.as_ref());
            }
        }

        impl LayoutElement for FakeContainer {
            fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
                self.size
            }
        }

        impl Rebuildable for FakeContainer {}

        struct FakeFlex {
            children: Vec<AnyElement>,
        }

        impl FakeFlex {
            fn new(children: Vec<AnyElement>) -> Self {
                Self { children }
            }
        }

        impl VisitorElement for FakeFlex {
            fn debug_name(&self) -> &'static str {
                "FakeFlex"
            }
        }

        impl Drawable for FakeFlex {
            fn update(&self, ctx: &BuildContext) {
                let mut current_y = 0.0;
                for child in &self.children {
                    let child_size = child.computed_size(ctx);
                    let c_w = child_size.width;
                    let c_h = child_size.height;

                    let mut is_visible = true;
                    if let Some((vx, vy, vw, vh)) = ctx.visible_rect
                        && (c_w < vx
                            || 0.0 > vx + vw
                            || current_y + c_h < vy
                            || current_y > vy + vh)
                    {
                        is_visible = false;
                    }

                    if is_visible {
                        let mut child_ctx = ctx.clone();
                        child_ctx.parent_size = child_size;
                        child_ctx.visible_rect = ctx
                            .visible_rect
                            .map(|(vx, vy, vw, vh)| (vx, vy - current_y, vw, vh));
                        child.update(&child_ctx);
                    }

                    current_y += c_h;
                }
            }
        }

        impl EventElement for FakeFlex {
            fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                for child in &self.children {
                    visitor(child.as_ref());
                }
            }
        }

        impl LayoutElement for FakeFlex {
            fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
                let mut width: f32 = 0.0;
                let mut height: f32 = 0.0;
                for child in &self.children {
                    let child_size = child.computed_size(ctx);
                    width = width.max(child_size.width);
                    height += child_size.height;
                }
                ResolvedSize { width, height }
            }
        }

        impl Rebuildable for FakeFlex {}

        struct FakeStack {
            children: Vec<AnyElement>,
        }

        impl FakeStack {
            fn new(children: Vec<AnyElement>) -> Self {
                Self { children }
            }
        }

        impl VisitorElement for FakeStack {
            fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                for child in &self.children {
                    visitor(child.as_ref());
                }
            }

            fn debug_name(&self) -> &'static str {
                "FakeStack"
            }
        }

        impl Drawable for FakeStack {
            fn update(&self, ctx: &BuildContext) {
                for child in &self.children {
                    child.update(ctx);
                }
            }
        }

        impl EventElement for FakeStack {}

        impl LayoutElement for FakeStack {
            fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
                let mut width: f32 = 0.0;
                let mut height: f32 = 0.0;
                for child in &self.children {
                    let child_size = child.computed_size(ctx);
                    width = width.max(child_size.width);
                    height = height.max(child_size.height);
                }
                ResolvedSize { width, height }
            }
        }

        impl Rebuildable for FakeStack {}

        struct FakePositioned {
            child: AnyElement,
        }

        impl FakePositioned {
            fn new(child: AnyElement) -> Self {
                Self { child }
            }
        }

        impl VisitorElement for FakePositioned {
            fn visit_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

            fn debug_name(&self) -> &'static str {
                "FakePositioned"
            }
        }

        impl Drawable for FakePositioned {
            fn update(&self, ctx: &BuildContext) {
                self.child.update(ctx);
            }
        }

        impl EventElement for FakePositioned {
            fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                visitor(self.child.as_ref());
            }
        }

        impl LayoutElement for FakePositioned {
            fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
                self.child.computed_size(ctx)
            }
        }

        impl Rebuildable for FakePositioned {
            fn rebuild_if_dirty(&self, ctx: &BuildContext) {
                self.child.rebuild_if_dirty(ctx);
            }

            fn mark_needs_rebuild(&self) {
                self.child.mark_needs_rebuild();
            }
        }

        struct FakeScrollable {
            child: AnyElement,
            #[allow(unused)]
            key: Option<Key>,
        }

        impl FakeScrollable {
            fn new(child: AnyElement) -> Self {
                Self {
                    child,
                    key: Some(Key::Static("scrollable-default")),
                }
            }
        }

        impl VisitorElement for FakeScrollable {
            fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                visitor(self.child.as_ref());
            }

            fn debug_name(&self) -> &'static str {
                "FakeScrollable"
            }
        }

        impl Drawable for FakeScrollable {
            fn update(&self, ctx: &BuildContext) {
                self.child.update(ctx);
            }
        }

        impl EventElement for FakeScrollable {
            fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}
        }

        impl LayoutElement for FakeScrollable {
            fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
                self.child.computed_size(ctx)
            }
        }

        impl Rebuildable for FakeScrollable {}

        #[derive(Debug)]
        struct VariantResult {
            label: &'static str,
            observer_after_resize: usize,
            live_counter_after_resize: usize,
        }

        fn build_home_page(
            ctx: &BuildContext,
            observer: Rc<Cell<usize>>,
            live_updater: Rc<RefCell<Option<StateUpdater<ResizeCounterState>>>>,
        ) -> AnyElement {
            let counter_widget = ResizeCounterWidget {
                observer,
                live_updater,
            };
            let (stateful, _updater) =
                StatefulElement::new_with_name(counter_widget, ctx, "Counter", None);

            FakeContainer::new(
                FakeStack::new(vec![
                    FakePositioned::new(FakeLeaf::new("HeaderLeaf", 200.0, 40.0).boxed()).boxed(),
                    FakePositioned::new(
                        FakeScrollable::new(
                            FakeFlex::new(vec![
                                FakeContainer::new(
                                    FakeLeaf::new("LeafA", 200.0, 100.0).boxed(),
                                    200.0,
                                    100.0,
                                )
                                .boxed(),
                                FakeContainer::new(
                                    FakeLeaf::new("LeafB", 200.0, 100.0).boxed(),
                                    200.0,
                                    100.0,
                                )
                                .boxed(),
                                FakeContainer::new(
                                    FakeLeaf::new("LeafC", 200.0, 100.0).boxed(),
                                    200.0,
                                    100.0,
                                )
                                .boxed(),
                                stateful.boxed(),
                            ])
                            .boxed(),
                        )
                        .boxed(),
                    )
                    .boxed(),
                ])
                .boxed(),
                200.0,
                400.0,
            )
            .boxed()
        }

        fn current_live_updater(
            live_updater: &Rc<RefCell<Option<StateUpdater<ResizeCounterState>>>>,
        ) -> StateUpdater<ResizeCounterState> {
            live_updater
                .borrow()
                .as_ref()
                .copied()
                .expect("current live updater should be published from build()")
        }

        fn run_variant(culled: bool, resize_count: usize) -> VariantResult {
            let initial_ctx = dummy_build_context();
            let observer = Rc::new(Cell::new(0usize));
            let live_updater = Rc::new(RefCell::new(None));

            let initial_child =
                build_home_page(&initial_ctx, observer.clone(), live_updater.clone());
            let rebuild_observer = observer.clone();
            let rebuild_live_updater = live_updater.clone();
            let driver = StatelessElement::new(
                initial_child,
                move |ctx| {
                    build_home_page(ctx, rebuild_observer.clone(), rebuild_live_updater.clone())
                },
                None,
                "HomePage",
            );

            driver.update(&initial_ctx);
            let current = current_live_updater(&live_updater);
            current.set_state(|state| state.counter = 2);
            driver.update(&initial_ctx);

            assert_eq!(
                observer.get(),
                2,
                "setup failed: stateful draw should observe counter=2 before resize"
            );

            let mut resize_ctx = initial_ctx.clone();
            resize_ctx.visible_rect = if culled {
                Some((0.0, 0.0, 500.0, 250.0))
            } else {
                None
            };

            for _ in 0..resize_count {
                driver.mark_needs_rebuild();
                driver.update(&resize_ctx);
            }

            VariantResult {
                label: match (culled, resize_count) {
                    (false, 1) => "on-screen / one resize",
                    (false, 2) => "on-screen / two resizes",
                    (true, 1) => "culled / one resize",
                    (true, 2) => "culled / two resizes",
                    _ => "unexpected",
                },
                observer_after_resize: observer.get(),
                live_counter_after_resize: current_live_updater(&live_updater)
                    .read(|state| state.counter),
            }
        }

        fn diagnosis(result: &VariantResult) -> &'static str {
            if result.observer_after_resize == 2 && result.live_counter_after_resize == 2 {
                "PASS: state survived; `StatefulElement::update_from_widget` adopted state at crates/aimer_widget/src/widget/stateful.rs:597-606 and `StatefulElement::draw` consumed the dirty flag at crates/aimer_widget/src/widget/stateful.rs:500-503."
            } else if result.observer_after_resize == 1 && result.live_counter_after_resize == 1 {
                "FAIL: `StatefulElement::update_from_widget` can adopt only by copying the old `rebuild_fn` and marking the new element dirty at crates/aimer_widget/src/widget/stateful.rs:457-472 and 597-606; it does NOT swap the freshly-created state cell or updater from `StatefulElement::new` at crates/aimer_widget/src/widget/stateful.rs:321-361. When the resize pre-pass in crates/aimer_widget/src/widget/stateless.rs:125-129 cannot reach the nested stateful and the later draw is skipped by flex-style culling mirroring crates/aimer_container/src/flex/raw_flex.rs:285-319, the adopted rebuild never runs at crates/aimer_widget/src/widget/stateful.rs:500-503, so both the observer and the latest updater remain on the fresh state value 1."
            } else if result.observer_after_resize == 1 && result.live_counter_after_resize == 2 {
                "FAIL: adoption happened, but the rebuilt state was never rendered; the fresh element records 1 during construction, then stays dirty until draw. The resize pre-pass in crates/aimer_widget/src/widget/stateless.rs:125-129 cannot reach through container-like wrappers that only expose `event_children`, and the adopted state is only materialized when `StatefulElement::draw` runs at crates/aimer_widget/src/widget/stateful.rs:500-503. In this reproduction that draw is skipped by flex-style culling mirroring crates/aimer_container/src/flex/raw_flex.rs:285-319."
            } else if result.live_counter_after_resize != 2 {
                "FAIL: state adoption itself appears to have been lost before draw; inspect crates/aimer_widget/src/widget/stateful.rs:597-606 and crates/aimer_widget/src/reconcile.rs:91-107."
            } else {
                "FAIL: unexpected mixed result; inspect the printed counters."
            }
        }

        #[test]
        fn resize_repro_state_survival_across_window_resize() {
            let results = [
                run_variant(false, 1),
                run_variant(false, 2),
                run_variant(true, 1),
                run_variant(true, 2),
            ];

            let mut failures = Vec::new();
            for result in &results {
                println!(
                    "[resize-repro] {} => observer={}, live_counter={} :: {}",
                    result.label,
                    result.observer_after_resize,
                    result.live_counter_after_resize,
                    diagnosis(result)
                );
                if result.observer_after_resize != 2 || result.live_counter_after_resize != 2 {
                    failures.push(format!(
                        "{} produced observer={} and live_counter={} (expected both 2)",
                        result.label,
                        result.observer_after_resize,
                        result.live_counter_after_resize
                    ));
                }
            }

            assert!(
                failures.is_empty(),
                "window-resize state reproduction failed:\n{}",
                failures.join("\n")
            );
        }

        // ─── Multi-button selection reproduction ──────────────────────────
        //
        // Reproduces `website/src/same_looking.rs`: a Row of platform
        // `TextButton`s where only the selected index is highlighted. Tapping a
        // button triggers the section's `set_state`, which rebuilds the whole
        // row of freshly-built buttons; those reconcile positionally against the
        // live ones. Each button is a `StatefulWidget` whose `State` mirrors the
        // parent-provided `selected` prop (exactly like `ButtonState` mirrors
        // the `TextButton` config), so it relies on `adopt_config_from` to
        // refresh that prop on reconcile.
        //
        // The reported regression: after picking "Android" (index 3) the other
        // buttons ("iOS"/"Web") ALSO render as active. This test asserts that
        // after switching the selection exactly ONE button (the newly selected
        // one) reports `selected == true`.
        #[allow(unused)]
        struct TabButtonWidget {
            index: usize,
            selected: bool,
            observer: Rc<Cell<i32>>,
        }
        #[allow(unused)]
        struct TabButtonState {
            index: usize,
            selected: bool,
            // Present to mirror `ButtonState`'s runtime field that
            // `adopt_config_from` must NOT clobber; not read by this test.
            #[allow(dead_code)]
            hovered: bool,
            observer: Rc<Cell<i32>>,
            updater: StateUpdater<Self>,
        }

        impl StatefulWidget for TabButtonWidget {
            type State = TabButtonState;

            fn create_state(self) -> Self::State {
                TabButtonState {
                    index: self.index,
                    selected: self.selected,
                    hovered: false,
                    observer: self.observer.clone(),
                    updater: StateUpdater::new(),
                }
            }
        }

        impl State<TabButtonWidget> for TabButtonState {
            fn init_state(&mut self, updater: StateUpdater<Self>) {
                self.updater = updater;
            }

            // Mirrors `ButtonState::adopt_config_from`: refresh the parent-
            // provided config (`index`, `selected`) while keeping runtime
            // (`hovered`).
            fn adopt_config_from(&mut self, new: Self) {
                self.index = new.index;
                self.selected = new.selected;
            }

            fn build(&self, _ctx: &BuildContext) -> impl Widget {
                // Record what this button believes its selection state is —
                // this is what drives its visible highlight in the real app.
                self.observer.set(if self.selected { 1 } else { 0 });
                EmptyWidget
            }
        }
        #[allow(unused)]
        const TAB_COUNT: usize = 4;
        #[allow(unused)]
        fn build_tab_row(
            ctx: &BuildContext,
            selected_index: Rc<Cell<usize>>,
            observers: Rc<Vec<Rc<Cell<i32>>>>,
        ) -> AnyElement {
            let selected = selected_index.get();
            let mut children: Vec<AnyElement> = Vec::with_capacity(TAB_COUNT);
            for index in 0..TAB_COUNT {
                let widget = TabButtonWidget {
                    index,
                    selected: index == selected,
                    observer: observers[index].clone(),
                };
                // Mirror the real app: `TextButton::to_element` yields a
                // `StatefulElement` whose `debug_name` is "Unknown", and the
                // `#[derive(WidgetConstructor)]`-generated `NamedWidget` then
                // wraps it in a `StatelessElement` named after the widget
                // ("TextButton") because the names differ. Reproduce that exact
                // wrapper so reconciliation takes the same path.
                let (stateful, _updater) =
                    StatefulElement::new_with_name(widget, ctx, "Unknown", None);
                let wrapped = StatelessElement::wrapper(stateful.boxed(), None, "TextButton");
                children.push(wrapped.boxed());
            }
            FakeFlex::new(children).boxed()
        }
    }
}
