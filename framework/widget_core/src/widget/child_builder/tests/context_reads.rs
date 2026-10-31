//! A retained child is rebuilt on a re-placement only when it can observe
//! something the placement changed.
//!
//! A parent that rebuilds itself places its retained child again. The child's
//! element survives, but its build closures are asked again so that they see
//! the new inherited context. That is only needed by a build that *read*
//! inherited context; a build that read none cannot differ from last time.

use super::*;
use crate::{State, StateUpdater, StatefulElement, StatefulWidget, StatelessElement};

/// An inherited value the host publishes and a subject may read.
struct Published(u32);

/// A stateful widget that counts its builds and optionally reads [`Published`].
struct Subject {
    builds: Rc<Cell<usize>>,
    seen: Rc<Cell<u32>>,
    reads_context: bool,
}

struct SubjectState {
    builds: Rc<Cell<usize>>,
    seen: Rc<Cell<u32>>,
    reads_context: bool,
}

impl StatefulWidget for Subject {
    type State = SubjectState;

    fn create_state(self) -> Self::State {
        SubjectState {
            builds: self.builds,
            seen: self.seen,
            reads_context: self.reads_context,
        }
    }
}

impl State<Subject> for SubjectState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        self.builds.set(self.builds.get() + 1);
        if self.reads_context {
            if let Some(published) = ctx.get_state::<Published>() {
                self.seen.set(published.0);
            }
        }
        ErrorWidget::new("subject")
    }
}

impl Widget for Subject {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget(self, ctx, "Subject", None)
    }
}

impl crate::widget::PortableWidget for Subject {}

/// A stateful parent that retains one child and publishes a [`Published`].
struct Host {
    child: ChildBuilder,
}

struct HostState {
    child: ChildBuilder,
    version: u32,
}

impl StatefulWidget for Host {
    type State = HostState;

    fn create_state(self) -> Self::State {
        HostState {
            child: self.child,
            version: 0,
        }
    }
}

impl State<Host> for HostState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        self.child = new.child;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        ctx.insert_state(Published(self.version));
        self.child.clone()
    }
}

/// A container of `Subject`s, so the retained subtree has many build scopes.
struct Row {
    subjects: Vec<AnyWidget>,
}

struct RowElement {
    subjects: Vec<AnyElement>,
}

impl Widget for Row {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RowElement {
            subjects: self
                .subjects
                .into_iter()
                .map(|subject| subject.to_element(ctx))
                .collect(),
        }
        .boxed()
    }
}

impl crate::widget::PortableWidget for Row {}

impl VisitorElement for RowElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for subject in &self.subjects {
            visitor(subject.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "Row"
    }
}
impl Drawable for RowElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for RowElement {}
impl EventElement for RowElement {}
impl Rebuildable for RowElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        for subject in &self.subjects {
            subject.rebuild_if_dirty(ctx);
        }
    }
}


struct Scene {
    ctx: BuildContext<'static>,
    element: StatefulElement,
    host: StateUpdater<HostState>,
    builds: Rc<Cell<usize>>,
    seen: Rc<Cell<u32>>,
}

fn scene(reads_context: bool) -> Scene {
    let ctx = context();
    let builds = Rc::new(Cell::new(0));
    let seen = Rc::new(Cell::new(0));
    let child = ChildBuilder::from_widget(Subject {
        builds: builds.clone(),
        seen: seen.clone(),
        reads_context,
    });
    let (element, host) =
        StatefulElement::new_with_name(Host { child }, &ctx, "Host", None);
    Scene {
        ctx,
        element,
        host,
        builds,
        seen,
    }
}

/// The host changes state the subject never reads, so the subject's build has
/// nothing new to observe.
#[tokio::test]
async fn a_child_that_read_no_context_is_not_rebuilt_by_its_parents_update() {
    let scene = scene(false);
    assert_eq!(scene.builds.get(), 1);

    scene.host.set_state(|state| state.version += 1);
    scene.element.rebuild_if_dirty(&scene.ctx);

    assert_eq!(
        scene.builds.get(),
        1,
        "a build that read no inherited context cannot differ from its last run"
    );
}

/// The guard for the test above: a child that did read inherited context must
/// still see the value its parent publishes now.
#[tokio::test]
async fn a_child_that_read_context_is_rebuilt_by_its_parents_update() {
    let scene = scene(true);
    assert_eq!(scene.seen.get(), 0);

    scene.host.set_state(|state| state.version += 1);
    scene.element.rebuild_if_dirty(&scene.ctx);

    assert_eq!(scene.seen.get(), 1, "the child must observe the new value");
    assert!(scene.builds.get() >= 2);
}

/// Repeated updates stay free for a child that reads nothing.
#[tokio::test]
async fn many_parent_updates_never_rebuild_a_child_that_read_no_context() {
    let scene = scene(false);

    for _ in 0..16 {
        scene.host.set_state(|state| state.version += 1);
        scene.element.rebuild_if_dirty(&scene.ctx);
    }

    assert_eq!(scene.builds.get(), 1);
}

/// A leaf that counts how often reconciliation looks at it.
struct Touched {
    touches: Rc<Cell<usize>>,
}

struct TouchedElement {
    touches: Rc<Cell<usize>>,
}

impl Widget for Touched {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        TouchedElement {
            touches: self.touches,
        }
        .boxed()
    }
}

impl crate::widget::PortableWidget for Touched {}

impl VisitorElement for TouchedElement {
    fn debug_name(&self) -> &'static str {
        "Touched"
    }

    fn reconciliation_key(&self) -> Option<&Key> {
        self.touches.set(self.touches.get() + 1);
        None
    }

    fn element_id(&self) -> Option<ElementId> {
        self.touches.set(self.touches.get() + 1);
        None
    }
}
impl Drawable for TouchedElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for TouchedElement {}
impl EventElement for TouchedElement {
    fn focus_node(&self) -> Option<&FocusNode> {
        self.touches.set(self.touches.get() + 1);
        None
    }
}
impl Rebuildable for TouchedElement {}

/// Placing the same retained child again has nothing to reconcile: both sides
/// are one element, so a pass must not walk what is inside it.
#[tokio::test]
async fn reconciling_two_placements_of_one_retained_child_does_not_walk_it() {
    let ctx = context();
    let touches = Rc::new(Cell::new(0));
    let child = ChildBuilder::from_widget(Row {
        subjects: (0..64)
            .map(|_| {
                Touched {
                    touches: touches.clone(),
                }
                .boxed()
            })
            .collect(),
    });
    let (element, host) = StatefulElement::new_with_name(Host { child }, &ctx, "Host", None);
    let before = touches.get();

    host.set_state(|state| state.version += 1);
    element.rebuild_if_dirty(&ctx);

    assert_eq!(
        touches.get() - before,
        0,
        "the retained child's interior was reconciled against itself"
    );
}

/// Cost of a parent update as a function of the retained subtree's size.
///
/// ```text
/// cargo nextest run -p aimer_widget context_reads_profile --run-ignored only --no-capture
/// ```
mod context_reads_profile {
    use aimer_time::{Duration, Instant};

    use super::*;
    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort();
        samples[samples.len() / 2]
    }

    fn update_cost(size: usize) -> (Duration, usize) {
        let ctx = context();
        let builds = Rc::new(Cell::new(0));
        let seen = Rc::new(Cell::new(0));
        let subjects = (0..size)
            .map(|_| {
                Subject {
                    builds: builds.clone(),
                    seen: seen.clone(),
                    reads_context: false,
                }
                .boxed()
            })
            .collect();
        let child = ChildBuilder::from_widget(Row { subjects });
        let (element, host) = StatefulElement::new_with_name(Host { child }, &ctx, "Host", None);
        let iterations = (100_000 / size.max(1)).clamp(20, 1_000);
        let before = builds.get();
        let mut samples = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let start = Instant::now();
            host.set_state(|state| state.version += 1);
            element.rebuild_if_dirty(&ctx);
            samples.push(start.elapsed());
        }
        ((median(samples)), (builds.get() - before) / iterations)
    }

    /// Splits one update into the placement mark, the whole update, and a
    /// pass over an already clean tree.
    fn phase_cost(size: usize) -> (Duration, Duration, Duration) {
        let ctx = context();
        let builds = Rc::new(Cell::new(0));
        let seen = Rc::new(Cell::new(0));
        let subjects = (0..size)
            .map(|_| {
                Subject {
                    builds: builds.clone(),
                    seen: seen.clone(),
                    reads_context: false,
                }
                .boxed()
            })
            .collect();
        let child = ChildBuilder::from_widget(Row { subjects });
        let probe = child.clone();
        let (element, host) = StatefulElement::new_with_name(Host { child }, &ctx, "Host", None);
        let iterations = (100_000 / size.max(1)).clamp(20, 1_000);
        let (mut mark, mut update, mut clean) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..iterations {
            // A second placement of the retained child is exactly the mark.
            let start = Instant::now();
            drop(probe.build(&ctx));
            mark.push(start.elapsed());

            host.set_state(|state| state.version += 1);
            let start = Instant::now();
            element.rebuild_if_dirty(&ctx);
            update.push(start.elapsed());

            let start = Instant::now();
            element.rebuild_if_dirty(&ctx);
            clean.push(start.elapsed());
        }
        (median(mark), median(update), median(clean))
    }

    #[tokio::test]
    #[ignore = "profiling harness; run explicitly with --run-ignored only"]
    async fn context_reads_profile() {
        println!("\nparent update with a retained child holding N stateful leaves (median)");
        println!("{:>7} {:>16} {:>18}", "N", "update+rebuild", "leaf builds/update");
        for size in [1, 10, 100, 1_000] {
            let (cost, builds) = update_cost(size);
            println!("{size:>7} {cost:>16.1?} {builds:>18}");
        }
        println!("\n{:>7} {:>12} {:>12} {:>12}", "N", "mark only", "update", "clean pass");
        for size in [1, 10, 100, 1_000] {
            let (mark, update, clean) = phase_cost(size);
            println!("{size:>7} {mark:>12.1?} {update:>12.1?} {clean:>12.1?}");
        }
    }
}

/// Only the leaf that read context is rebuilt; its sibling is left alone.
#[tokio::test]
async fn only_the_leaf_that_read_context_is_rebuilt_in_a_mixed_subtree() {
    let ctx = context();
    let (reader_builds, reader_seen) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (quiet_builds, quiet_seen) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let child = ChildBuilder::from_widget(Row {
        subjects: vec![
            Subject {
                builds: reader_builds.clone(),
                seen: reader_seen.clone(),
                reads_context: true,
            }
            .boxed(),
            Subject {
                builds: quiet_builds.clone(),
                seen: quiet_seen.clone(),
                reads_context: false,
            }
            .boxed(),
        ],
    });
    let (element, host) = StatefulElement::new_with_name(Host { child }, &ctx, "Host", None);

    host.set_state(|state| state.version = 7);
    element.rebuild_if_dirty(&ctx);

    assert_eq!(reader_seen.get(), 7, "the reader must see the new value");
    assert_eq!(reader_builds.get(), 2);
    assert_eq!(quiet_builds.get(), 1, "the quiet leaf has nothing new to observe");
}

/// A build closure may read anything it captured, so a placement always
/// reaches a stateless child even when it read no inherited context.
#[tokio::test]
async fn a_stateless_child_is_still_rebuilt_by_its_parents_update() {
    let ctx = context();
    let builds = Rc::new(Cell::new(0));
    let counted = builds.clone();
    let child = ChildBuilder::from_widget(ClosureChild { builds: counted });
    let (element, host) = StatefulElement::new_with_name(Host { child }, &ctx, "Host", None);
    assert_eq!(builds.get(), 1);

    host.set_state(|state| state.version += 1);
    element.rebuild_if_dirty(&ctx);

    assert_eq!(builds.get(), 2);
}

/// A child built from a closure, as `StatelessElement::from_builder` is.
struct ClosureChild {
    builds: Rc<Cell<usize>>,
}

impl Widget for ClosureChild {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let builds = self.builds;
        StatelessElement::from_builder(
            ctx,
            move |ctx| {
                builds.set(builds.get() + 1);
                ErrorWidget::new("closure").to_element(ctx)
            },
            None,
            "ClosureChild",
        )
        .boxed()
    }
}

impl crate::widget::PortableWidget for ClosureChild {}

/// New content a host re-places while adopting live state.
///
/// A fresh host is built with its own retained child. When reconciliation then
/// hands it a live host's state, it places the child again at once, and carry
/// cannot pair that child with the old one because they are not the same widget.
/// What it places is new content that was only ever built at construction, so its
/// stateful elements are rebuilt like any freshly placed content, whether or not
/// their last build read context.
struct Other(Subject);

impl Widget for Other {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget(self.0, ctx, "Other", None)
    }
}

impl crate::widget::PortableWidget for Other {}

#[tokio::test]
async fn content_placed_while_adopting_live_state_is_rebuilt() {
    let ctx = context();
    let seen = Rc::new(Cell::new(0));
    let subject = |builds: &Rc<Cell<usize>>| Subject {
        builds: builds.clone(),
        seen: seen.clone(),
        reads_context: false,
    };
    let (old_builds, new_builds) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (old, _old_updater) = StatefulElement::new_with_name(
        Host {
            child: ChildBuilder::from_widget(subject(&old_builds)),
        },
        &ctx,
        "Host",
        None,
    );
    let new = StatefulElement::from_widget(
        Host {
            child: ChildBuilder::from_widget(Other(subject(&new_builds))),
        },
        &ctx,
        "Host",
        None,
    );
    assert_eq!(new_builds.get(), 1, "built once, at construction");

    crate::components::element::reconcile_carried_tree(&old, new.as_ref(), &ctx);
    new.rebuild_if_dirty(&ctx);

    assert_eq!(
        new_builds.get(),
        2,
        "content placed during adoption is new, so its stateful elements rebuild"
    );
}
