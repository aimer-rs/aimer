use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_color::prelude::Color;
use aimer_container::SizedBox;
use aimer_decoration::{BackdropFilter, DecorationBox};
use aimer_style::{BoxDecoration, BoxShadow};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{Drawable, Widget};

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

#[tokio::test]
async fn decoration_box_moves_shadow_storage_without_allocating_after_warmup() {
    let inner = InnerCanvas::new();
    let ctx = BuildContext::new(
        FrameCanvas::new(&inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    let shadowed = || {
        BoxDecoration::new()
            .background_color(Color::RED)
            .add_shadow(BoxShadow::new().blur(4.0))
    };

    for _ in 0..4 {
        drop(
            DecorationBox::new()
                .decoration(shadowed())
                .child(SizedBox::new())
                .to_element(&ctx),
        );
    }
    let before_description = ALLOCATIONS.with(Cell::get);
    let decoration = shadowed();
    let describing = ALLOCATIONS.with(Cell::get) - before_description;
    let before_build = ALLOCATIONS.with(Cell::get);
    drop(
        DecorationBox::new()
            .decoration(decoration)
            .child(SizedBox::new())
            .to_element(&ctx),
    );
    let building = ALLOCATIONS.with(Cell::get) - before_build;

    assert_eq!(
        describing, 1,
        "the counter must observe the shadow vector allocation"
    );
    assert_eq!(
        building, 0,
        "warm conversion must move the decoration into pooled element storage"
    );
}

#[tokio::test]
async fn decoration_box_unchanged_paint_checks_allocate_nothing() {
    let inner = InnerCanvas::new();
    let ctx = BuildContext::new(
        FrameCanvas::new(&inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    let element = DecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::RED))
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
    let root = tree
        .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 24.0, 12.0))
        .unwrap();
    ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
        element.paint_local_v2(ctx)
    });
    assert!(!element.local_v2_paint_needs_recording(&ctx));

    let before = ALLOCATIONS.with(Cell::get);
    let unchanged = (0..1_024).all(|_| !element.local_v2_paint_needs_recording(&ctx));
    let checking = ALLOCATIONS.with(Cell::get) - before;
    assert!(unchanged);
    assert_eq!(checking, 0);
}

#[tokio::test]
async fn backdrop_filter_reuses_its_recording_without_allocating() {
    let inner = InnerCanvas::new();
    let ctx = BuildContext::new(
        FrameCanvas::new(&inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    let element = DecorationBox::new()
        .decoration(BackdropFilter::new().saturation(0.5))
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
    let root = tree
        .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 24.0, 12.0))
        .unwrap();
    ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
        element.paint_local_v2(ctx)
    });
    let revision = tree.draw_list_revision(root).unwrap();
    let before = ALLOCATIONS.with(Cell::get);
    let unchanged = (0..1_024).all(|_| !element.local_v2_paint_needs_recording(&ctx));
    let checking = ALLOCATIONS.with(Cell::get) - before;
    assert!(unchanged);
    assert_eq!(tree.draw_list_revision(root).unwrap(), revision);
    assert_eq!(
        checking, 0,
        "unchanged filter traversal must reuse the encoded request"
    );
}

#[tokio::test]
async fn magnification_reuses_its_recording_without_allocating() {
    let inner = InnerCanvas::new();
    let ctx = BuildContext::new(
        FrameCanvas::new(&inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    let element = DecorationBox::new()
        .decoration(aimer_decoration::Magnification::new().zoom(2.0))
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
    let root = tree
        .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 24.0, 12.0))
        .unwrap();
    ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
        element.paint_local_v2(ctx)
    });
    let revision = tree.draw_list_revision(root).unwrap();
    let before = ALLOCATIONS.with(Cell::get);
    let unchanged = (0..1_024).all(|_| !element.local_v2_paint_needs_recording(&ctx));
    let checking = ALLOCATIONS.with(Cell::get) - before;
    assert!(unchanged);
    assert_eq!(tree.draw_list_revision(root).unwrap(), revision);
    assert_eq!(
        checking, 0,
        "unchanged lens traversal must reuse the encoded request"
    );
}

#[tokio::test]
async fn light_edge_reuses_its_recording_without_allocating() {
    let inner = InnerCanvas::new();
    let ctx = BuildContext::new(
        FrameCanvas::new(&inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    let element = DecorationBox::new()
        .decoration(aimer_decoration::LightEdge::new())
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
    let root = tree
        .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 24.0, 12.0))
        .unwrap();
    ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
        element.paint_local_v2(ctx)
    });
    let revision = tree.draw_list_revision(root).unwrap();
    let before = ALLOCATIONS.with(Cell::get);
    let unchanged = (0..1_024).all(|_| !element.local_v2_paint_needs_recording(&ctx));
    let checking = ALLOCATIONS.with(Cell::get) - before;
    assert!(unchanged);
    assert_eq!(tree.draw_list_revision(root).unwrap(), revision);
    assert_eq!(
        checking, 0,
        "unchanged light-edge traversal must reuse the encoded request"
    );
}

#[tokio::test]
async fn light_edge_auto_color_reuses_inherited_background_without_allocating() {
    let inner = InnerCanvas::new();
    let mut ctx = BuildContext::new(
        FrameCanvas::new(&inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    ctx.paint_background = Color::WHITE;
    let element = DecorationBox::new()
        .decoration(aimer_decoration::LightEdge::new())
        .child(SizedBox::new().width(40).height(20))
        .to_element(&ctx);
    let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
    let root = tree
        .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 40.0, 20.0))
        .unwrap();
    ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
        element.paint_local_v2(ctx)
    });
    let before = ALLOCATIONS.with(Cell::get);
    let unchanged = (0..1_024).all(|_| !element.local_v2_paint_needs_recording(&ctx));
    let checking = ALLOCATIONS.with(Cell::get) - before;
    assert!(unchanged);
    assert_eq!(
        checking, 0,
        "automatic contrast must not allocate during retained checks"
    );
    ctx.paint_background = Color::BLACK;
    assert!(element.local_v2_paint_needs_recording(&ctx));
}

#[tokio::test]
async fn multi_decoration_typed_and_runtime_stacks_reuse_recording_without_allocating() {
    use aimer_decoration::{Decoration, LightEdge, MultiDecorationBox};
    let inner = InnerCanvas::new();
    let ctx = BuildContext::new(
        FrameCanvas::new(&inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    let typed = MultiDecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::BLUE))
        .decoration(LightEdge::new())
        .child(SizedBox::new().width(40).height(20))
        .to_element(&ctx);
    let layers: Vec<Box<dyn Decoration>> = vec![
        Box::new(BoxDecoration::new().background_color(Color::BLUE)),
        Box::new(LightEdge::new()),
    ];
    let runtime = MultiDecorationBox::new()
        .decorations(layers)
        .child(SizedBox::new().width(40).height(20))
        .to_element(&ctx);
    for element in [typed, runtime] {
        let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
        let root = tree
            .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 40.0, 20.0))
            .unwrap();
        ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
            element.paint_local_v2(ctx)
        });
        let revision = tree.draw_list_revision(root).unwrap();
        let before = ALLOCATIONS.with(Cell::get);
        let unchanged = (0..1_024).all(|_| !element.local_v2_paint_needs_recording(&ctx));
        let checking = ALLOCATIONS.with(Cell::get) - before;
        assert!(unchanged);
        assert_eq!(tree.draw_list_revision(root).unwrap(), revision);
        assert_eq!(checking, 0, "retained layer checks must not allocate");
    }
}

#[tokio::test]
async fn standalone_style_components_reuse_recording_and_geometry_without_allocating() {
    use aimer_decoration::{BorderRadius, BorderSlice, BorderStyle, BoxBorder, BoxOutline, Decoration};
    let inner = InnerCanvas::new();
    let size = ResolvedSize { width: 80.0, height: 40.0 };
    let ctx = BuildContext::new(
        FrameCanvas::new(&inner), size, 2.0, Vec2d::default(), Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0), tokio::runtime::Handle::current(),
    );
    let stroke = BorderSlice::new().style(BorderStyle::Solid).stroke(2).color(Color::BLUE);
    let decorations: [std::rc::Rc<dyn Decoration>; 5] = [
        std::rc::Rc::new(BoxBorder::all(stroke)), std::rc::Rc::new(BoxOutline::all(stroke)),
        std::rc::Rc::new(BoxShadow::new().blur(3.0)), std::rc::Rc::new(BorderRadius::from(8)),
        std::rc::Rc::new(Color::BLUE),
    ];
    for decoration in decorations {
        let element = DecorationBox::new().decoration(decoration.clone())
            .child(SizedBox::new().width(40).height(20)).to_element(&ctx);
        let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
        let node = tree.add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 80.0, 40.0)).unwrap();
        ctx.with_local_v2_paint_context(tree.context(node).unwrap(), |ctx| element.paint_local_v2(ctx));
        let before = ALLOCATIONS.with(Cell::get);
        for _ in 0..1_024 {
            assert!(!element.local_v2_paint_needs_recording(&ctx));
            std::hint::black_box(decoration.content_insets(size, 2.0));
            std::hint::black_box(decoration.child_clip_radius(size, 2.0));
            std::hint::black_box(decoration.paint_outsets(size, 2.0));
        }
        assert_eq!(ALLOCATIONS.with(Cell::get) - before, 0);
    }
}

#[test]
fn notype_keeps_small_decorations_inline_without_allocating_or_rewrapping() {
    use aimer_decoration::Decoration;
    let before = ALLOCATIONS.with(Cell::get);
    let mut decoration = BoxShadow::new().blur(3.0).notype();
    assert!(decoration.is_inline());
    for _ in 0..1_024 {
        decoration = decoration.notype();
        std::hint::black_box(&decoration);
    }
    assert!(decoration.is_inline());
    drop(decoration);
    assert_eq!(ALLOCATIONS.with(Cell::get) - before, 0);
}
