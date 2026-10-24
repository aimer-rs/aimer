//! What the render-tree sync measures in order to place a node's children.
//!
//! A node's size is never enough to place its children: the sync also has to
//! know the constraint they were laid out under. Measuring a subtree is the
//! expensive part of the first frame of a page, because nothing below it is
//! cached yet, so a measurement nobody uses is paid for in full.

use std::cell::RefCell;
use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_container::Container;
use aimer_style::LayoutSpacing;
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{
    AnyElement, Drawable, EventElement, LayoutElement, Rebuildable, VisitorElement, Widget,
};

use super::WindowRenderTree;

/// Records the widest space it was measured in, each time it is measured.
struct WidthProbe {
    widths: Rc<RefCell<Vec<f32>>>,
}

impl VisitorElement for WidthProbe {
    fn debug_name(&self) -> &'static str {
        "WidthProbe"
    }
}

impl EventElement for WidthProbe {}
impl Rebuildable for WidthProbe {}

impl LayoutElement for WidthProbe {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.widths.borrow_mut().push(ctx.box_constraint.max_width);
        ResolvedSize {
            width: ctx.box_constraint.max_width,
            height: 10.0,
        }
    }
}

impl Drawable for WidthProbe {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        aimer_canvas::Canvas::of(ctx).finish();
    }
}

impl Widget for WidthProbe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        aimer_widget::Element::boxed(self)
    }
}

impl aimer_widget::PortableWidget for WidthProbe {}

fn context(canvas: &aimer_canvas::InnerCanvas) -> (tokio::runtime::Runtime, BuildContext<'_>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("create render-tree sync runtime");
    let mut context = BuildContext::new(
        aimer_canvas::FrameCanvas::new(canvas),
        ResolvedSize {
            width: 100.0,
            height: 80.0,
        },
        1.0,
        Vec2d::ZERO,
        Vec2d::ZERO,
        WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 80), 1.0),
        runtime.handle().clone(),
    );
    context.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 100.0,
        max_height: 80.0,
    };
    (runtime, context)
}

#[test]
fn a_container_that_gives_its_child_a_context_does_not_measure_it_in_its_own() {
    let canvas = aimer_canvas::InnerCanvas::new();
    let (_runtime, context) = context(&canvas);
    let widths = Rc::new(RefCell::new(Vec::new()));
    let root = Container::new()
        .padding(LayoutSpacing::all(12))
        .child(WidthProbe {
            widths: widths.clone(),
        })
        .to_element(&context);

    WindowRenderTree::default()
        .sync(Some(&root), &context, (100, 80), 1.0)
        .expect("synchronize the render tree");

    // The 100px context minus 12px of padding on both sides is what the child
    // is laid out in. Measuring it in the full 100px as well is a second
    // measurement of everything below it, for an answer nobody reads.
    let widths = widths.borrow();
    assert!(!widths.is_empty(), "the child was never measured");
    assert!(
        widths.iter().all(|width| (*width - 76.0).abs() < 0.01),
        "the child was measured in {widths:?} instead of only the 76px it is laid out in"
    );
}
