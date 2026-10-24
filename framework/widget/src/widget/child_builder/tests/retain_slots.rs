//! `BuildContext::retain` keeps a part of a `build` output alive between builds.
//!
//! A stateful widget's `build` runs again on every `set_state`, and everything
//! it returns is converted to elements again. A part that did not change can be
//! declared once under a key; later builds place the element the first build
//! made instead of describing the part anew.

use std::cell::RefCell;

use super::*;
use crate::{State, StateUpdater, StatefulElement, StatefulWidget};

/// A stateful leaf that counts how often it is built.
struct Leaf {
    label: &'static str,
    builds: Rc<Cell<usize>>,
    log: Rc<RefCell<Vec<&'static str>>>,
}

struct LeafState {
    label: &'static str,
    builds: Rc<Cell<usize>>,
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl StatefulWidget for Leaf {
    type State = LeafState;

    fn create_state(self) -> Self::State {
        LeafState {
            label: self.label,
            builds: self.builds,
            log: self.log,
        }
    }
}

impl State<Leaf> for LeafState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        self.builds.set(self.builds.get() + 1);
        self.log.borrow_mut().push(self.label);
        ErrorWidget::new("leaf")
    }
}

impl Widget for Leaf {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget(self, ctx, "Leaf", None)
    }
}

impl crate::widget::PortableWidget for Leaf {}

/// A stateful owner whose `build` retains one leaf per key in `keys`.
struct Owner {
    keys: Vec<&'static str>,
    leaf_builds: Rc<Cell<usize>>,
    log: Rc<RefCell<Vec<&'static str>>>,
    declared: Rc<Cell<usize>>,
    builds: Rc<Cell<usize>>,
}

struct OwnerState {
    keys: Vec<&'static str>,
    leaf_builds: Rc<Cell<usize>>,
    log: Rc<RefCell<Vec<&'static str>>>,
    declared: Rc<Cell<usize>>,
    builds: Rc<Cell<usize>>,
}

impl StatefulWidget for Owner {
    type State = OwnerState;

    fn create_state(self) -> Self::State {
        OwnerState {
            keys: self.keys,
            leaf_builds: self.leaf_builds,
            log: self.log,
            declared: self.declared,
            builds: self.builds,
        }
    }
}

impl State<Owner> for OwnerState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        self.builds.set(self.builds.get() + 1);
        let leaves = self
            .keys
            .iter()
            .map(|key| {
                ctx.retain(*key, || {
                    self.declared.set(self.declared.get() + 1);
                    Leaf {
                        label: *key,
                        builds: self.leaf_builds.clone(),
                        log: self.log.clone(),
                    }
                })
                .boxed()
            })
            .collect();
        Column { leaves }
    }
}

impl Widget for Owner {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget(self, ctx, "Owner", None)
    }
}

impl crate::widget::PortableWidget for Owner {}

/// Holds several children, so one build can retain more than one slot.
struct Column {
    leaves: Vec<AnyWidget>,
}

struct ColumnElement {
    leaves: Vec<AnyElement>,
}

impl Widget for Column {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        ColumnElement {
            leaves: self
                .leaves
                .into_iter()
                .map(|leaf| leaf.to_element(ctx))
                .collect(),
        }
        .boxed()
    }
}

impl crate::widget::PortableWidget for Column {}

impl VisitorElement for ColumnElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for leaf in &self.leaves {
            visitor(leaf.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "Column"
    }
}
impl Drawable for ColumnElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for ColumnElement {}
impl EventElement for ColumnElement {}
impl Rebuildable for ColumnElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        for leaf in &self.leaves {
            leaf.rebuild_if_dirty(ctx);
        }
    }
}

struct Fixture {
    ctx: BuildContext<'static>,
    element: StatefulElement,
    owner: StateUpdater<OwnerState>,
    leaf_builds: Rc<Cell<usize>>,
    log: Rc<RefCell<Vec<&'static str>>>,
    declared: Rc<Cell<usize>>,
    builds: Rc<Cell<usize>>,
}

fn fixture(keys: &[&'static str]) -> Fixture {
    let ctx = context();
    let leaf_builds = Rc::new(Cell::new(0));
    let log = Rc::new(RefCell::new(Vec::new()));
    let declared = Rc::new(Cell::new(0));
    let builds = Rc::new(Cell::new(0));
    let (element, owner) = StatefulElement::new_with_name(
        Owner {
            keys: keys.to_vec(),
            leaf_builds: leaf_builds.clone(),
            log: log.clone(),
            declared: declared.clone(),
            builds: builds.clone(),
        },
        &ctx,
        "Owner",
        None,
    );
    Fixture {
        ctx,
        element,
        owner,
        leaf_builds,
        log,
        declared,
        builds,
    }
}

impl Fixture {
    /// Runs one `set_state` and the rebuild it schedules.
    fn update(&self, change: impl FnOnce(&mut OwnerState) + 'static) {
        self.owner.set_state(change);
        self.element.rebuild_if_dirty(&self.ctx);
    }
}

/// The point of the slot: the owner builds again, the declared part does not.
#[tokio::test]
async fn a_retained_part_is_declared_and_built_once_across_the_owners_rebuilds() {
    let fixture = fixture(&["row"]);
    assert_eq!(fixture.declared.get(), 1);
    assert_eq!(fixture.leaf_builds.get(), 1);

    for _ in 0..8 {
        fixture.update(|_| {});
    }

    assert_eq!(fixture.builds.get(), 9, "the owner itself builds every time");
    assert_eq!(
        fixture.declared.get(),
        1,
        "the closure that describes the part runs once"
    );
    assert_eq!(
        fixture.leaf_builds.get(),
        1,
        "the part's own state is not built again"
    );
}

/// A new key is a new part: the closure runs and the leaf is built afresh.
#[tokio::test]
async fn a_new_key_declares_and_builds_a_new_part() {
    let fixture = fixture(&["old"]);

    fixture.update(|state| state.keys = vec!["new"]);

    assert_eq!(fixture.declared.get(), 2);
    assert_eq!(
        *fixture.log.borrow(),
        ["old", "new"],
        "the old part's state must not be carried onto the new one"
    );
}

/// Several keys are independent slots and all of them survive a rebuild.
#[tokio::test]
async fn parts_under_different_keys_are_retained_independently() {
    let fixture = fixture(&["a", "b", "c"]);
    assert_eq!(fixture.declared.get(), 3);

    fixture.update(|_| {});
    fixture.update(|_| {});

    assert_eq!(fixture.declared.get(), 3);
    assert_eq!(fixture.leaf_builds.get(), 3);
}

/// Dropping one key rebuilds nothing else.
#[tokio::test]
async fn removing_one_key_keeps_the_others_retained() {
    let fixture = fixture(&["a", "b", "c"]);

    fixture.update(|state| state.keys = vec!["a", "c"]);

    assert_eq!(fixture.declared.get(), 3);
    // `c` moved up into the position `b` had. Two retained parts are two
    // subtrees, so `b`'s state must not be carried onto `c`: a leaf built
    // again under the label `b` would be exactly that.
    assert_eq!(*fixture.log.borrow(), ["a", "b", "c"]);
}

/// A slot the owner stopped asking for is released, so asking for it again
/// declares it afresh rather than resurrecting a stale part.
#[tokio::test]
async fn a_part_the_owner_stops_requesting_is_released() {
    let fixture = fixture(&["a", "b"]);

    fixture.update(|state| state.keys = vec!["a"]);
    fixture.update(|state| state.keys = vec!["a", "b"]);

    assert_eq!(
        fixture.declared.get(),
        3,
        "`b` was absent from one build, so its slot was released"
    );
}

/// Outside a stateful or stateless build there is nowhere to keep a slot; the
/// call still answers, it just cannot retain anything.
#[tokio::test]
async fn retain_outside_a_build_still_produces_the_part() {
    let ctx = context();
    let declared = Rc::new(Cell::new(0));

    let first = ctx.retain("row", || {
        declared.set(declared.get() + 1);
        ErrorWidget::new("row")
    });
    let second = ctx.retain("row", || {
        declared.set(declared.get() + 1);
        ErrorWidget::new("row")
    });

    assert_eq!(declared.get(), 2, "nothing was retained between the calls");
    let _ = (first.build(&ctx), second.build(&ctx));
}

/// A stateless widget's build is a scope too, so it can retain a part.
#[tokio::test]
async fn a_stateless_build_can_retain_a_part() {
    use crate::StatelessElement;

    let ctx = context();
    let declared = Rc::new(Cell::new(0));
    let leaf_builds = Rc::new(Cell::new(0));
    let log = Rc::new(RefCell::new(Vec::new()));
    let element = StatelessElement::from_builder(
        &ctx,
        {
            let (declared, leaf_builds, log) = (declared.clone(), leaf_builds.clone(), log.clone());
            move |ctx| {
                ctx.retain("row", || {
                    declared.set(declared.get() + 1);
                    Leaf {
                        label: "row",
                        builds: leaf_builds.clone(),
                        log: log.clone(),
                    }
                })
                .to_element(ctx)
            }
        },
        None,
        "StatelessOwner",
    );

    for _ in 0..4 {
        element.mark_needs_rebuild();
        element.rebuild_if_dirty(&ctx);
    }

    // An explicit mark reaches everything below, the retained part included,
    // so the leaf may well build again. What the slot saves is describing the
    // part and converting it to elements, which happened once.
    assert_eq!(declared.get(), 1);
    assert_eq!(log.borrow().first(), Some(&"row"));
}

/// Measures what a retained slot saves when its owner builds again.
///
/// Run with:
///
/// ```text
/// cargo nextest run -p aimer_widget retain_slots_profile --run-ignored only --no-capture
/// ```
mod retain_slots_profile {
    use std::time::{Duration, Instant};

    use super::*;

    /// An owner whose whole body is `size` stateful leaves, retained or not.
    struct Wide {
        size: usize,
        retained: bool,
        leaf_builds: Rc<Cell<usize>>,
        log: Rc<RefCell<Vec<&'static str>>>,
    }

    struct WideState {
        size: usize,
        retained: bool,
        leaf_builds: Rc<Cell<usize>>,
        log: Rc<RefCell<Vec<&'static str>>>,
    }

    impl StatefulWidget for Wide {
        type State = WideState;

        fn create_state(self) -> Self::State {
            WideState {
                size: self.size,
                retained: self.retained,
                leaf_builds: self.leaf_builds,
                log: self.log,
            }
        }
    }

    impl State<Wide> for WideState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, ctx: &BuildContext) -> impl Widget {
            let body = || Column {
                leaves: (0..self.size)
                    .map(|_| {
                        Leaf {
                            label: "leaf",
                            builds: self.leaf_builds.clone(),
                            log: Rc::new(RefCell::new(Vec::new())),
                        }
                        .boxed()
                    })
                    .collect(),
            };
            if self.retained {
                ctx.retain("body", body).boxed()
            } else {
                body().boxed()
            }
        }
    }

    impl Widget for Wide {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            StatefulElement::from_widget(self, ctx, "Wide", None)
        }
    }

    impl crate::widget::PortableWidget for Wide {}

    fn update_cost(size: usize, retained: bool) -> (Duration, usize) {
        let ctx = context();
        let leaf_builds = Rc::new(Cell::new(0));
        let (element, owner) = StatefulElement::new_with_name(
            Wide {
                size,
                retained,
                leaf_builds: leaf_builds.clone(),
                log: Rc::new(RefCell::new(Vec::new())),
            },
            &ctx,
            "Wide",
            None,
        );
        let iterations = (50_000 / size.max(1)).clamp(10, 500);
        let before = leaf_builds.get();
        let mut samples = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let start = Instant::now();
            owner.set_state(|_| {});
            element.rebuild_if_dirty(&ctx);
            samples.push(start.elapsed());
        }
        samples.sort();
        (samples[samples.len() / 2], (leaf_builds.get() - before) / iterations)
    }

    #[tokio::test]
    #[ignore = "profiling harness: run on demand"]
    async fn retain_slots_profile() {
        println!("\nowner set_state with N stateful leaves in its body (median)");
        println!(
            "{:>7} {:>14} {:>12} {:>14} {:>12}",
            "N", "not retained", "leaf builds", "retained", "leaf builds"
        );
        for size in [1, 10, 100, 1_000] {
            let (plain, plain_builds) = update_cost(size, false);
            let (kept, kept_builds) = update_cost(size, true);
            println!("{size:>7} {plain:>14.1?} {plain_builds:>12} {kept:>14.1?} {kept_builds:>12}");
        }
    }
}
