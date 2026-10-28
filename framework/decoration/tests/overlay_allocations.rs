//! Unchanged-paint checks of the foreground, masked and gradient additions run
//! every frame, so they must not allocate.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_color::prelude::Color;
use aimer_container::SizedBox;
use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};
use aimer_decoration::{
    DecorationBox, ForegroundDecorationBox, Gradient, MaskedDecorationBox, OpacityGradient,
};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{Drawable, Element, Widget};

struct RecordingAllocator;

aimer_tls::thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn record_allocation() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

// SAFETY: All allocation operations forward unchanged to System. The counter
// only observes allocations made on the current test thread.
unsafe impl GlobalAlloc for RecordingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation();
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: RecordingAllocator = RecordingAllocator;

fn context(inner: &InnerCanvas) -> BuildContext<'_> {
    BuildContext::new(
        FrameCanvas::new(inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    )
}

/// Records `element` once, then proves 1,024 unchanged checks allocate nothing
/// and leave the recording alone.
fn assert_unchanged_checks_allocate_nothing(element: &dyn Element, ctx: &BuildContext, what: &str) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 24.0, 12.0)).unwrap();
    ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
        element.paint_local_v2(ctx)
    });
    let revision = tree.draw_list_revision(root).unwrap();
    let before = ALLOCATIONS.with(Cell::get);
    let unchanged = (0..1_024).all(|_| !element.local_v2_paint_needs_recording(ctx));
    let checking = ALLOCATIONS.with(Cell::get) - before;
    assert!(unchanged, "{what}");
    assert_eq!(tree.draw_list_revision(root).unwrap(), revision);
    assert_eq!(checking, 0, "{what}: unchanged traversal must not allocate");
}

fn rainbow_child() -> SizedBox {
    SizedBox::new().width(24.0).height(12.0).color(Color::BLUE)
}

#[tokio::test]
async fn a_gradient_reuses_its_recording_without_allocating() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let element = DecorationBox::new()
        .decoration(Gradient::rainbow())
        .child(rainbow_child())
        .to_element(&ctx);
    assert_unchanged_checks_allocate_nothing(element.as_ref(), &ctx, "gradient");
}

#[tokio::test]
async fn a_masked_box_reuses_its_recording_without_allocating() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let element = MaskedDecorationBox::new()
        .decoration(Gradient::rainbow())
        .child(rainbow_child())
        .to_element(&ctx);
    assert_unchanged_checks_allocate_nothing(element.as_ref(), &ctx, "masked box");
}

#[tokio::test]
async fn an_opacity_mask_box_reuses_its_recording_without_allocating() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let element = OpacityGradient::linear()
        .opacities([1.0, 0.0])
        .child(rainbow_child())
        .to_element(&ctx);
    assert_unchanged_checks_allocate_nothing(element.as_ref(), &ctx, "opacity mask box");
}

#[tokio::test]
async fn a_foreground_layer_reuses_its_recording_without_allocating() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let element = ForegroundDecorationBox::new()
        .decoration(Gradient::rainbow())
        .child(rainbow_child())
        .to_element(&ctx);
    // The decoration lives in the wrapper's second retained child.
    let mut layer = None;
    element.visit_children(&mut |child| layer = Some(child));
    let layer = layer.expect("the foreground layer");
    let layer_ctx = element
        .retained_v2_child_context_at(&ctx, layer, 1)
        .expect("the wrapper sizes its layer");
    assert_eq!(layer.debug_name(), "ForegroundDecorationLayer");
    assert_unchanged_checks_allocate_nothing(layer, &layer_ctx, "foreground layer");
}
