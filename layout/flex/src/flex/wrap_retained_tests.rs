//! A wrapping flex paints through the retained v2 tree like any other flex:
//! it records no paint of its own, and tells the tree where every wrapped child
//! sits so the children keep their own render nodes.

use std::cell::Cell;
use std::rc::Rc;

use aimer_cupid::draw_cmd_v2::Rect;
use aimer_widget::base::{BuildContext, ResolvedSize};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement, Widget,
};

use crate::flex::test_support::dummy_build_context;
use crate::flex::{OverflowBehavior, Row};

/// A leaf with a fixed size.
struct Fixed(f32, f32);

impl Widget for Fixed {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        FixedElement { size: self }.boxed()
    }
}

impl aimer_widget::PortableWidget for Fixed {}

struct FixedElement {
    size: Fixed,
}

impl VisitorElement for FixedElement {
    fn debug_name(&self) -> &'static str {
        "Fixed"
    }
}
impl EventElement for FixedElement {}
impl Rebuildable for FixedElement {}
impl Drawable for FixedElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for FixedElement {
    fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize {
            width: self.size.0,
            height: self.size.1,
        }
    }
}

fn wrapped_row(ctx: &BuildContext) -> AnyElement {
    Row::new()
        .overflow(OverflowBehavior::Wrap)
        .children([Fixed(60.0, 10.0), Fixed(50.0, 20.0), Fixed(40.0, 15.0)])
        .to_element(ctx)
}

#[test]
fn a_wrapping_row_paints_locally() {
    let ctx = dummy_build_context(100.0, 100.0, None);
    let row = wrapped_row(&ctx);

    assert!(
        row.as_ref().can_paint_local_v2(&ctx),
        "a wrapping flex must not fall back to a legacy island"
    );
}

#[test]
fn wrapped_children_are_placed_on_their_lines() {
    let ctx = dummy_build_context(100.0, 100.0, None);
    let row = wrapped_row(&ctx);

    let mut children = Vec::new();
    row.visit_retained_v2_children(&mut |index, child| children.push((index, child)));
    assert_eq!(children.len(), 3);

    let rects: Vec<Rect> = children
        .iter()
        .map(|(index, child)| {
            row.retained_v2_child_geometry_at(&ctx, *child, *index)
                .expect("a wrapped child reports retained geometry")
                .0
        })
        .collect();
    // 60 + 50 does not fit in 100, so the second line holds the last two.
    assert_eq!(
        rects,
        vec![
            Rect::new(0.0, 0.0, 60.0, 10.0),
            Rect::new(0.0, 10.0, 50.0, 20.0),
            Rect::new(50.0, 10.0, 40.0, 15.0),
        ]
    );
}

#[test]
fn a_wrapped_child_is_given_exactly_its_own_size() {
    let ctx = dummy_build_context(100.0, 100.0, None);
    let row = wrapped_row(&ctx);

    let mut children = Vec::new();
    row.visit_retained_v2_children(&mut |index, child| children.push((index, child)));
    let child_ctx = row
        .retained_v2_child_context_at(&ctx, children[1].1, 1)
        .expect("a wrapped child gets a retained context");

    assert_eq!(child_ctx.box_constraint.max_width, 50.0);
    assert_eq!(child_ctx.box_constraint.max_height, 20.0);
}

#[test]
fn a_stranger_is_not_given_wrapped_geometry() {
    let ctx = dummy_build_context(100.0, 100.0, None);
    let row = wrapped_row(&ctx);
    let stranger = Fixed(1.0, 1.0).to_element(&ctx);

    assert!(
        row.retained_v2_child_geometry_at(&ctx, stranger.as_ref(), 0)
            .is_none()
    );
}

/// A leaf that counts how often it is measured.
struct Counted(Rc<Cell<usize>>);

impl Widget for Counted {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        CountedElement { measured: self.0 }.boxed()
    }
}

impl aimer_widget::PortableWidget for Counted {}

struct CountedElement {
    measured: Rc<Cell<usize>>,
}

impl VisitorElement for CountedElement {
    fn debug_name(&self) -> &'static str {
        "Counted"
    }
}
impl EventElement for CountedElement {}
impl Rebuildable for CountedElement {}
impl Drawable for CountedElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for CountedElement {
    fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        self.measured.set(self.measured.get() + 1);
        ResolvedSize {
            width: 30.0,
            height: 10.0,
        }
    }
}

/// The retained tree asks for every child's slot in turn. Re-wrapping the whole
/// list per child would measure `O(n²)` times; one layout pass serves them all.
#[test]
fn syncing_every_wrapped_child_measures_the_list_once() {
    const CHILDREN: usize = 300;
    let ctx = dummy_build_context(1000.0, 1000.0, None);
    let measured = Rc::new(Cell::new(0));
    let row = Row::new()
        .overflow(OverflowBehavior::Wrap)
        .children((0..CHILDREN).map(|_| Counted(measured.clone())))
        .to_element(&ctx);

    let mut children = Vec::new();
    row.visit_retained_v2_children(&mut |index, child| children.push((index, child)));
    for (index, child) in &children {
        row.retained_v2_child_geometry_at(&ctx, *child, *index)
            .unwrap();
        row.retained_v2_child_context_at(&ctx, *child, *index)
            .unwrap();
    }

    assert!(
        measured.get() < 4 * CHILDREN,
        "{} measurements for {CHILDREN} children is not linear",
        measured.get()
    );
}
