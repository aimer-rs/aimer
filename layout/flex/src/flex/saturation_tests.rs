//! A flex whose children each ask for the largest representable extent (a
//! percentage of an unbounded axis resolves to `f32::MAX`) must stay finite.
//! The render tree rejects non-finite rectangles, so an overflowed sum would
//! otherwise take the flex and every later sibling out of the tree.

use aimer_widget::base::{BuildContext, ResolvedSize};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement, Widget,
};

use crate::flex::test_support::dummy_build_context;
use crate::flex::{Column, Row};

/// A leaf that asks for `f32::MAX` along its main axis.
struct Huge(f32, f32);

impl Widget for Huge {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        HugeElement {
            width: self.0,
            height: self.1,
        }
        .boxed()
    }
}

impl aimer_widget::PortableWidget for Huge {}

struct HugeElement {
    width: f32,
    height: f32,
}

impl VisitorElement for HugeElement {
    fn debug_name(&self) -> &'static str {
        "Huge"
    }
}
impl EventElement for HugeElement {}
impl Rebuildable for HugeElement {}
impl Drawable for HugeElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for HugeElement {
    fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize {
            width: self.width,
            height: self.height,
        }
    }
}

#[test]
fn a_row_of_maximal_children_has_a_finite_size() {
    let ctx = dummy_build_context(100.0, 100.0, None);
    let row = Row::new()
        .children((0..13).map(|_| Huge(f32::MAX, 10.0)))
        .to_element(&ctx);

    let size = row.computed_size(&ctx);

    assert!(size.width.is_finite(), "width overflowed to {}", size.width);
    assert!(size.height.is_finite());
}

#[test]
fn every_child_of_a_maximal_column_has_a_finite_offset() {
    let ctx = dummy_build_context(100.0, 100.0, None);
    let column = Column::new()
        .children((0..13).map(|_| Huge(10.0, f32::MAX)))
        .to_element(&ctx);

    let mut children = Vec::new();
    column.visit_retained_v2_children(&mut |index, child| children.push((index, child)));
    assert_eq!(children.len(), 13);
    for (index, child) in children {
        let (rect, _) = column
            .retained_v2_child_geometry_at(&ctx, child, index)
            .expect("every child reports retained geometry");
        assert!(
            rect.x.is_finite() && rect.y.is_finite(),
            "child {index} was placed at ({}, {})",
            rect.x,
            rect.y
        );
    }
}
