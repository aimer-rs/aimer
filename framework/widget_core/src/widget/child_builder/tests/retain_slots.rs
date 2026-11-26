//! `BuildContext::retain` keeps a part of a `build` output alive between builds.
//!
//! A stateful widget's `build` runs again on every `set_state`, and everything
//! it returns is converted to elements again. A part that did not change can be
//! declared once under a key; later builds place the element the first build
//! made instead of describing the part anew.

use std::cell::RefCell;

use super::*;
use crate::BuildContextRetain;
use crate::{State, StateView, StateUpdater, StatefulElement};

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

impl State<Leaf> for LeafState {
    fn create(widget: Leaf) -> Self {
        LeafState {
            label: widget.label,
            builds: widget.builds,
            log: widget.log,
        }
    }

    fn build(this: StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        this.builds.set(this.builds.get() + 1);
        this.log.borrow_mut().push(this.label);
        ErrorWidget::new("leaf")
    }
}

impl Widget for Leaf {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget::<Self, LeafState>(self, ctx, "Leaf", None)
    }
}

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

impl State<Owner> for OwnerState {
    fn create(widget: Owner) -> Self {
        OwnerState {
            keys: widget.keys,
            leaf_builds: widget.leaf_builds,
            log: widget.log,
            declared: widget.declared,
            builds: widget.builds,
        }
    }

    fn build(this: StateView<Self>, ctx: &BuildContext) -> impl Widget {
        this.builds.set(this.builds.get() + 1);
        let leaves = this.keys
            .iter()
            .map(|key| {
                ctx.retain(*key, || {
                    this.declared.set(this.declared.get() + 1);
                    Leaf {
                        label: *key,
                        builds: this.leaf_builds.clone(),
                        log: this.log.clone(),
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
        StatefulElement::from_widget::<Self, OwnerState>(self, ctx, "Owner", None)
    }
}

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

impl VisitorElement for ColumnElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for leaf in &self.leaves {
            visitor(leaf.as_ref());
        }
    }
}

impl Element for ColumnElement {
    fn name(&self) -> &'static str {
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
    let (element, owner) = StatefulElement::new_with_name::<Owner, OwnerState>(
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
    use aimer_time::{Duration, Instant};

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

    impl State<Wide> for WideState {
        fn create(widget: Wide) -> Self {
            WideState {
                size: widget.size,
                retained: widget.retained,
                leaf_builds: widget.leaf_builds,
                log: widget.log,
            }
        }

        fn build(this: StateView<Self>, ctx: &BuildContext) -> impl Widget {
            let body = || Column {
                leaves: (0..this.size)
                    .map(|_| {
                        Leaf {
                            label: "leaf",
                            builds: this.leaf_builds.clone(),
                            log: Rc::new(RefCell::new(Vec::new())),
                        }
                        .boxed()
                    })
                    .collect(),
            };
            if this.retained {
                ctx.retain("body", body).boxed()
            } else {
                body().boxed()
            }
        }
    }

    impl Widget for Wide {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            StatefulElement::from_widget::<Self, WideState>(self, ctx, "Wide", None)
        }
    }

    fn update_cost(size: usize, retained: bool) -> (Duration, usize) {
        let ctx = context();
        let leaf_builds = Rc::new(Cell::new(0));
        let (element, owner) = StatefulElement::new_with_name::<Wide, WideState>(
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

/// `BuildContext::retain_hashed` keeps a part alive for as long as the widget
/// that describes it keeps the same content hash.
mod retain_hashed {
    use super::*;

    /// A stateful leaf that takes the configuration of a rebuilt twin into its live
    /// state, as real widgets do through `adopt_config_from`.
    struct AdoptingLeaf {
        label: &'static str,
        builds: Rc<Cell<usize>>,
        log: Rc<RefCell<Vec<&'static str>>>,
        adoptions: Rc<Cell<usize>>,
    }

    struct AdoptingLeafState {
        label: &'static str,
        builds: Rc<Cell<usize>>,
        log: Rc<RefCell<Vec<&'static str>>>,
        adoptions: Rc<Cell<usize>>,
    }

    impl State<AdoptingLeaf> for AdoptingLeafState {
        fn create(widget: AdoptingLeaf) -> Self {
            AdoptingLeafState {
                label: widget.label,
                builds: widget.builds,
                log: widget.log,
                adoptions: widget.adoptions,
            }
        }

        fn adopt_config_from(&mut self, new: Self) {
            self.label = new.label;
            self.adoptions.set(self.adoptions.get() + 1);
        }

        fn build(this: StateView<Self>, _ctx: &BuildContext) -> impl Widget {
            this.builds.set(this.builds.get() + 1);
            this.log.borrow_mut().push(this.label);
            ErrorWidget::new("leaf")
        }
    }

    impl Widget for AdoptingLeaf {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            StatefulElement::from_widget::<Self, AdoptingLeafState>(self, ctx, "AdoptingLeaf", None)
        }
    }

    /// A leaf that reports a hash the test chooses; `0` stands for no hash.
    struct HashedLeaf {
        leaf: AdoptingLeaf,
        hash: usize,
    }

    impl Widget for HashedLeaf {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            self.leaf.to_element(ctx)
        }

        fn hash(&self) -> Option<crate::ContentHash> {
            use std::hash::Hash;
            if self.hash == 0 {
                return None;
            }
            let mut hasher = crate::ContentHasher::new::<Self>();
            self.hash.hash(&mut hasher);
            hasher.finish_hash()
        }
    }

    /// A stateful owner that describes one hashed part on every build.
    struct HashOwner {
        label: &'static str,
        hash: usize,
        leaf_builds: Rc<Cell<usize>>,
        log: Rc<RefCell<Vec<&'static str>>>,
        described: Rc<Cell<usize>>,
        adoptions: Rc<Cell<usize>>,
    }

    struct HashOwnerState {
        label: &'static str,
        hash: usize,
        leaf_builds: Rc<Cell<usize>>,
        log: Rc<RefCell<Vec<&'static str>>>,
        described: Rc<Cell<usize>>,
        adoptions: Rc<Cell<usize>>,
    }

    impl State<HashOwner> for HashOwnerState {
        fn create(widget: HashOwner) -> Self {
            HashOwnerState {
                label: widget.label,
                hash: widget.hash,
                leaf_builds: widget.leaf_builds,
                log: widget.log,
                described: widget.described,
                adoptions: widget.adoptions,
            }
        }

        fn build(this: StateView<Self>, ctx: &BuildContext) -> impl Widget {
            // The owner always describes the part: that is the cheap half. What
            // `retain_hashed` decides is whether it becomes elements again.
            this.described.set(this.described.get() + 1);
            ctx.retain_hashed(
                "part",
                HashedLeaf {
                    leaf: AdoptingLeaf {
                        label: this.label,
                        builds: this.leaf_builds.clone(),
                        log: this.log.clone(),
                        adoptions: this.adoptions.clone(),
                    },
                    hash: this.hash,
                },
            )
        }
    }

    impl Widget for HashOwner {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            StatefulElement::from_widget::<Self, HashOwnerState>(self, ctx, "HashOwner", None)
        }
    }

    struct Hashed {
        ctx: BuildContext<'static>,
        element: StatefulElement,
        owner: StateUpdater<HashOwnerState>,
        leaf_builds: Rc<Cell<usize>>,
        log: Rc<RefCell<Vec<&'static str>>>,
        described: Rc<Cell<usize>>,
        adoptions: Rc<Cell<usize>>,
    }

    fn hashed(label: &'static str, hash: usize) -> Hashed {
        let ctx = context();
        let leaf_builds = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Vec::new()));
        let described = Rc::new(Cell::new(0));
        let adoptions = Rc::new(Cell::new(0));
        let (element, owner) = StatefulElement::new_with_name::<HashOwner, HashOwnerState>(
            HashOwner {
                label,
                hash,
                leaf_builds: leaf_builds.clone(),
                log: log.clone(),
                described: described.clone(),
                adoptions: adoptions.clone(),
            },
            &ctx,
            "HashOwner",
            None,
        );
        Hashed {
            ctx,
            element,
            owner,
            leaf_builds,
            log,
            described,
            adoptions,
        }
    }

    impl Hashed {
        fn update(&self, change: impl FnOnce(&mut HashOwnerState) + 'static) {
            self.owner.set_state(change);
            self.element.rebuild_if_dirty(&self.ctx);
        }
    }

    /// The point of the hash: the owner describes the part again, the part is
    /// not turned into elements again because nothing about it changed.
    #[tokio::test]
    async fn an_unchanged_hash_keeps_the_part_across_the_owners_rebuilds() {
        let fixture = hashed("part", 7);
        assert_eq!(fixture.leaf_builds.get(), 1);

        for _ in 0..8 {
            fixture.update(|_| {});
        }

        assert_eq!(fixture.described.get(), 9, "the owner describes the part every time");
        assert_eq!(
            fixture.leaf_builds.get(),
            1,
            "an unchanged part was built into elements again"
        );
    }

    /// A different hash means the configuration changed, so the part is built
    /// from the new widget.
    #[tokio::test]
    async fn a_changed_hash_builds_the_part_from_the_new_widget() {
        let fixture = hashed("before", 1);

        fixture.update(|state| {
            state.label = "after";
            state.hash = 2;
        });

        assert_eq!(*fixture.log.borrow(), ["before", "after"]);
    }

    /// A changed hash is the same part with new configuration, so it is rebuilt the
    /// way any rebuild is: the state below it lives on and is handed the new
    /// configuration, rather than being thrown away. A slider being dragged
    /// changes its hash on every tick and must not lose the drag.
    #[tokio::test]
    async fn a_changed_hash_keeps_the_live_state_and_hands_it_the_new_configuration() {
        let fixture = hashed("before", 1);
        assert_eq!(fixture.adoptions.get(), 0);

        fixture.update(|state| {
            state.label = "after";
            state.hash = 2;
        });

        assert_eq!(
            fixture.adoptions.get(),
            1,
            "the part was built afresh instead of adopting into the live state"
        );
        assert_eq!(*fixture.log.borrow(), ["before", "after"]);
    }

    /// The slot follows the latest hash: after a change the new part is the one
    /// kept, and it is kept again rather than rebuilt on every later build.
    #[tokio::test]
    async fn the_part_built_after_a_change_is_the_one_retained() {
        let fixture = hashed("before", 1);

        fixture.update(|state| {
            state.label = "after";
            state.hash = 2;
        });
        for _ in 0..4 {
            fixture.update(|_| {});
        }

        assert_eq!(fixture.leaf_builds.get(), 2, "one build per distinct hash");
        assert_eq!(*fixture.log.borrow(), ["before", "after"]);
    }

    /// No hash is never taken to be equal to another widget's, so the part is
    /// not retained at all.
    #[tokio::test]
    async fn a_widget_without_a_hash_is_never_retained() {
        let retained = hashed("part", 5);
        let unhashed = hashed("part", 0);

        for _ in 0..4 {
            retained.update(|_| {});
            unhashed.update(|_| {});
        }

        assert_eq!(retained.leaf_builds.get(), 1);
        assert!(
            unhashed.leaf_builds.get() > retained.leaf_builds.get(),
            "a widget without a hash was treated as unchanged"
        );
    }

    /// A hash that comes back after a different one is not resurrected: the slot
    /// holds the latest part only.
    #[tokio::test]
    async fn an_earlier_hash_is_not_resurrected() {
        let fixture = hashed("first", 1);

        fixture.update(|state| {
            state.label = "second";
            state.hash = 2;
        });
        fixture.update(|state| {
            state.label = "first";
            state.hash = 1;
        });

        assert_eq!(*fixture.log.borrow(), ["first", "second", "first"]);
    }

    /// Outside a build there is no scope to keep a slot in; the call still
    /// answers with the part.
    #[tokio::test]
    async fn retain_hashed_outside_a_build_still_produces_the_part() {
        let ctx = context();
        let part = ctx.retain_hashed("row", ErrorWidget::new("row"));

        let _ = part.build(&ctx);
    }
}
