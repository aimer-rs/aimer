//! Pins down what `StateUpdater::set_state` costs a `StatefulElement`.
//!
//! `set_state` re-runs `State::build` and converts the *entire* returned widget
//! tree into fresh elements; the old child subtree is then dropped and replaced.
//! These tests count that work so the behaviour is measured rather than assumed.

use super::*;
use crate::AnyWidget;

#[derive(Default)]
struct Counts {
    builds: Cell<usize>,
    leaf_to_elements: Cell<usize>,
    leaf_drops: Cell<usize>,
}

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
        dummy_async_handle(),
    )
}

/// A leaf whose conversion into an element and whose destruction are counted.
struct CountedLeaf {
    counts: Rc<Counts>,
}

struct CountedLeafElement {
    counts: Rc<Counts>,
}

impl Widget for CountedLeaf {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        self.counts
            .leaf_to_elements
            .set(self.counts.leaf_to_elements.get() + 1);
        CountedLeafElement { counts: self.counts }.boxed()
    }
}

impl crate::widget::PortableWidget for CountedLeaf {}

impl Drop for CountedLeafElement {
    fn drop(&mut self) {
        self.counts.leaf_drops.set(self.counts.leaf_drops.get() + 1);
    }
}

impl VisitorElement for CountedLeafElement {
    fn debug_name(&self) -> &'static str {
        "CountedLeaf"
    }
}
impl Drawable for CountedLeafElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for CountedLeafElement {}
impl EventElement for CountedLeafElement {}
impl Rebuildable for CountedLeafElement {}

/// A container so the built subtree has more than one node.
struct Group {
    children: Vec<AnyWidget>,
}

struct GroupElement {
    children: Vec<AnyElement>,
}

impl Widget for Group {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        GroupElement {
            children: self
                .children
                .into_iter()
                .map(|child| child.to_element(ctx))
                .collect(),
        }
        .boxed()
    }
}

impl crate::widget::PortableWidget for Group {}

impl VisitorElement for GroupElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "Group"
    }
}
impl Drawable for GroupElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for GroupElement {}
impl EventElement for GroupElement {}
impl Rebuildable for GroupElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        for child in &self.children {
            child.rebuild_if_dirty(ctx);
        }
    }
}

const LEAVES: usize = 5;

struct Counter {
    counts: Rc<Counts>,
    leaves: usize,
}

struct CounterState {
    counts: Rc<Counts>,
    leaves: usize,
    taps: usize,
}

impl StatefulWidget for Counter {
    type State = CounterState;

    fn create_state(self) -> Self::State {
        CounterState {
            counts: self.counts,
            leaves: self.leaves,
            taps: 0,
        }
    }
}

impl State<Counter> for CounterState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    /// Deliberately ignores `taps`: none of the leaves depend on the state that
    /// `set_state` mutates below.
    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        self.counts.builds.set(self.counts.builds.get() + 1);
        Group {
            children: (0..self.leaves)
                .map(|_| {
                    CountedLeaf {
                        counts: self.counts.clone(),
                    }
                    .boxed()
                })
                .collect(),
        }
    }
}

#[test]
fn set_state_rebuilds_and_replaces_the_whole_child_subtree() {
    let context = dummy_build_context();
    let counts = Rc::new(Counts::default());
    let (element, updater) = StatefulElement::new_with_name(
        Counter {
            counts: counts.clone(),
            leaves: LEAVES,
        },
        &context,
        "Counter",
        None,
    );

    // Initial build: `build` ran once and produced every leaf.
    assert_eq!(counts.builds.get(), 1);
    assert_eq!(counts.leaf_to_elements.get(), LEAVES);
    assert_eq!(counts.leaf_drops.get(), 0);

    // Nothing is dirty yet, so a rebuild pass is a no-op.
    element.rebuild_if_dirty(&context);
    assert_eq!(counts.builds.get(), 1);

    // `set_state` itself only queues the mutation; it does not build anything.
    updater.set_state(|state| state.taps += 1);
    assert_eq!(counts.builds.get(), 1, "set_state must not build inline");
    assert!(element.is_dirty());

    element.rebuild_if_dirty(&context);

    // One more `build`, and every leaf was converted to an element again even
    // though none of them depends on `taps`.
    assert_eq!(counts.builds.get(), 2);
    assert_eq!(counts.leaf_to_elements.get(), 2 * LEAVES);
    // The previous subtree was dropped and a new one installed, not patched.
    // (Comparing addresses would prove nothing: `AnyElement` stores small
    // elements inline, so the replacement lands in the same slot.)
    assert_eq!(counts.leaf_drops.get(), LEAVES);
    assert!(!element.is_dirty());
    // The mutation itself was applied exactly once.
    assert_eq!(updater.read(|state| state.taps), 1);
}

#[test]
fn set_state_calls_between_frames_coalesce_into_one_rebuild() {
    let context = dummy_build_context();
    let counts = Rc::new(Counts::default());
    let (element, updater) = StatefulElement::new_with_name(
        Counter {
            counts: counts.clone(),
            leaves: LEAVES,
        },
        &context,
        "Counter",
        None,
    );

    for _ in 0..10 {
        updater.set_state(|state| state.taps += 1);
    }
    element.rebuild_if_dirty(&context);

    assert_eq!(counts.builds.get(), 2, "ten set_state calls, one build");
    assert_eq!(counts.leaf_to_elements.get(), 2 * LEAVES);
    assert_eq!(updater.read(|state| state.taps), 10);
}

#[derive(Default)]
struct NestedCounts {
    create_states: Cell<usize>,
    init_states: Cell<usize>,
    builds: Cell<usize>,
    /// The `value` the most recent `build` saw.
    last_built_value: Cell<usize>,
}

type NestedUpdater = Rc<RefCell<Option<StateUpdater<NestedState>>>>;

/// A stateful widget living inside another stateful widget's build output.
struct Nested {
    counts: Rc<NestedCounts>,
    updater: NestedUpdater,
}

struct NestedState {
    counts: Rc<NestedCounts>,
    updater: NestedUpdater,
    value: usize,
}

impl StatefulWidget for Nested {
    type State = NestedState;

    fn create_state(self) -> Self::State {
        self.counts
            .create_states
            .set(self.counts.create_states.get() + 1);
        NestedState {
            counts: self.counts,
            updater: self.updater,
            value: 0,
        }
    }
}

impl State<Nested> for NestedState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.counts.init_states.set(self.counts.init_states.get() + 1);
        *self.updater.borrow_mut() = Some(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        self.counts.builds.set(self.counts.builds.get() + 1);
        self.counts.last_built_value.set(self.value);
        CountedLeaf {
            counts: Rc::new(Counts::default()),
        }
    }
}

impl Widget for Nested {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget(self, ctx, "Nested", None)
    }
}

impl crate::widget::PortableWidget for Nested {}

struct NestingParent {
    nested: Rc<NestedCounts>,
    updater: NestedUpdater,
}

struct NestingParentState {
    nested: Rc<NestedCounts>,
    updater: NestedUpdater,
    taps: usize,
}

impl StatefulWidget for NestingParent {
    type State = NestingParentState;

    fn create_state(self) -> Self::State {
        NestingParentState {
            nested: self.nested,
            updater: self.updater,
            taps: 0,
        }
    }
}

impl State<NestingParent> for NestingParentState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    /// The nested child does not depend on `taps`.
    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        Group {
            children: vec![
                Nested {
                    counts: self.nested.clone(),
                    updater: self.updater.clone(),
                }
                .boxed(),
            ],
        }
    }
}

/// Pins today's behaviour for a stateful widget inside another stateful
/// widget's build output: the user-visible state is kept, but the child is
/// constructed from scratch first, so its `create_state`, `init_state` and
/// `build` all run again, and `build` runs a second time on the carried state.
#[test]
fn a_nested_stateful_child_keeps_its_state_but_builds_again_when_the_parent_updates() {
    let context = dummy_build_context();
    let nested = Rc::new(NestedCounts::default());
    let updater: NestedUpdater = Rc::default();
    let (element, parent) = StatefulElement::new_with_name(
        NestingParent {
            nested: nested.clone(),
            updater: updater.clone(),
        },
        &context,
        "NestingParent",
        None,
    );
    assert_eq!(nested.create_states.get(), 1);
    assert_eq!(nested.init_states.get(), 1);
    assert_eq!(nested.builds.get(), 1);

    // Give the nested state something a rebuild must not lose, and let its own
    // update settle so only the parent's update is measured below.
    let inner = updater.borrow().expect("nested state initialised");
    inner.set_state(|state| state.value = 7);
    element.rebuild_if_dirty(&context);
    assert_eq!(nested.builds.get(), 2);
    assert_eq!(nested.last_built_value.get(), 7);

    parent.set_state(|state| state.taps += 1);
    element.rebuild_if_dirty(&context);

    // The state the user can see survives the parent's update...
    assert_eq!(nested.last_built_value.get(), 7, "state must be carried over");
    // ...but a throwaway state was created and initialised for the new element,
    // and the nested `build` ran twice: once in the new element's constructor,
    // and again on the carried state.
    assert_eq!(nested.create_states.get(), 2);
    assert_eq!(nested.init_states.get(), 2);
    assert_eq!(nested.builds.get(), 4);
}

/// Cost of `set_state` as a function of subtree size. Debug-mode numbers are the
/// ones that matter for this repository, so run it unoptimised:
///
/// ```text
/// cargo nextest run -p aimer_widget rebuild_scope_profile --run-ignored only --no-capture
/// ```
mod rebuild_scope_profile {
    use aimer_time::{Duration, Instant};

    use super::*;

    const SIZES: [usize; 5] = [1, 10, 100, 1_000, 5_000];

    /// Fewer iterations for bigger trees so every row takes a similar time.
    fn iterations(size: usize) -> usize {
        (200_000 / size.max(1)).clamp(20, 2_000)
    }

    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort();
        samples[samples.len() / 2]
    }

    fn per_leaf_ns(total: Duration, leaves: usize) -> f64 {
        total.as_nanos() as f64 / leaves.max(1) as f64
    }

    /// Experiment A: the dirty element itself returns `size` leaves.
    fn set_state_cost(size: usize) -> (Duration, Duration) {
        let context = dummy_build_context();
        let counts = Rc::new(Counts::default());
        let (element, updater) = StatefulElement::new_with_name(
            Counter {
                counts,
                leaves: size,
            },
            &context,
            "Counter",
            None,
        );
        let mut dirty_pass = Vec::new();
        let mut clean_pass = Vec::new();
        for _ in 0..iterations(size) {
            let start = Instant::now();
            updater.set_state(|state| state.taps += 1);
            element.rebuild_if_dirty(&context);
            dirty_pass.push(start.elapsed());

            // Nothing is dirty now: the pass should be (nearly) free.
            let start = Instant::now();
            element.rebuild_if_dirty(&context);
            clean_pass.push(start.elapsed());
        }
        (median(dirty_pass), median(clean_pass))
    }

    /// A small stateful widget that hands its updater to the test.
    struct Inner {
        counts: Rc<Counts>,
        updater: Rc<RefCell<Option<StateUpdater<InnerState>>>>,
    }

    struct InnerState {
        counts: Rc<Counts>,
        updater: Rc<RefCell<Option<StateUpdater<InnerState>>>>,
        taps: usize,
    }

    impl StatefulWidget for Inner {
        type State = InnerState;

        fn create_state(self) -> Self::State {
            InnerState {
                counts: self.counts,
                updater: self.updater,
                taps: 0,
            }
        }
    }

    impl State<Inner> for InnerState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            *self.updater.borrow_mut() = Some(updater);
        }

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            Group {
                children: (0..LEAVES)
                    .map(|_| {
                        CountedLeaf {
                            counts: self.counts.clone(),
                        }
                        .boxed()
                    })
                    .collect(),
            }
        }
    }

    impl Widget for Inner {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            StatefulElement::from_widget(self, ctx, "Inner", None)
        }
    }

    impl crate::widget::PortableWidget for Inner {}

    /// A clean outer stateful holding `size` leaves next to one small dirty
    /// stateful child.
    struct Outer {
        counts: Rc<Counts>,
        inner_counts: Rc<Counts>,
        updater: Rc<RefCell<Option<StateUpdater<InnerState>>>>,
        size: usize,
    }

    struct OuterState {
        counts: Rc<Counts>,
        inner_counts: Rc<Counts>,
        updater: Rc<RefCell<Option<StateUpdater<InnerState>>>>,
        size: usize,
    }

    impl StatefulWidget for Outer {
        type State = OuterState;

        fn create_state(self) -> Self::State {
            OuterState {
                counts: self.counts,
                inner_counts: self.inner_counts,
                updater: self.updater,
                size: self.size,
            }
        }
    }

    impl State<Outer> for OuterState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            let mut children: Vec<AnyWidget> = (0..self.size)
                .map(|_| {
                    CountedLeaf {
                        counts: self.counts.clone(),
                    }
                    .boxed()
                })
                .collect();
            children.push(
                Inner {
                    counts: self.inner_counts.clone(),
                    updater: self.updater.clone(),
                }
                .boxed(),
            );
            Group { children }
        }
    }

    /// Experiment B: only the small inner element is dirty, the outer one and its
    /// `size` leaves are not.
    fn scoped_cost(size: usize) -> (Duration, usize, usize) {
        let context = dummy_build_context();
        let outer_counts = Rc::new(Counts::default());
        let inner_counts = Rc::new(Counts::default());
        let updater = Rc::new(RefCell::new(None));
        let (element, _) = StatefulElement::new_with_name(
            Outer {
                counts: outer_counts.clone(),
                inner_counts: inner_counts.clone(),
                updater: updater.clone(),
                size,
            },
            &context,
            "Outer",
            None,
        );
        let inner = updater.borrow().expect("inner state initialised");
        let outer_before = outer_counts.leaf_to_elements.get();
        let inner_before = inner_counts.leaf_to_elements.get();
        let mut samples = Vec::new();
        for _ in 0..iterations(size) {
            let start = Instant::now();
            inner.set_state(|state| state.taps += 1);
            element.rebuild_if_dirty(&context);
            samples.push(start.elapsed());
        }
        let rebuilt = iterations(size);
        // The outer leaves were never rebuilt; the inner ones were, every time.
        assert_eq!(outer_counts.leaf_to_elements.get(), outer_before);
        assert_eq!(
            inner_counts.leaf_to_elements.get() - inner_before,
            rebuilt * LEAVES
        );
        (median(samples), outer_counts.builds.get(), rebuilt)
    }

    #[test]
    #[ignore = "profiling harness; run explicitly with --run-ignored only"]
    fn set_state_cost_by_subtree_size() {
        println!("\nA. set_state on an element whose build() returns N leaves (median)");
        println!("{:>7} {:>14} {:>12} {:>16}", "N", "set_state+rebuild", "ns/leaf", "clean pass");
        for size in SIZES {
            let (dirty, clean) = set_state_cost(size);
            println!(
                "{size:>7} {:>14.1?} {:>12.0} {:>16.1?}",
                dirty,
                per_leaf_ns(dirty, size),
                clean
            );
        }

        println!("\nB. one small dirty element (5 leaves) inside a clean parent holding N leaves");
        println!("{:>7} {:>14} {:>14}", "N", "set_state+rebuild", "outer builds");
        for size in SIZES {
            let (cost, outer_builds, _) = scoped_cost(size);
            println!("{size:>7} {:>14.1?} {outer_builds:>14}", cost);
        }
    }
}
