use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::SizedBox;
use aimer_cupid::draw_cmd::DrawCommand;
use aimer_widget::base::WindowHandle;
use aimer_widget::{Drawable, EventElement, LayoutElement, Rebuildable, VisitorElement};

#[tokio::test]
async fn container_accepts_a_backdrop_decoration_without_changing_layout() {
    use aimer_cupid::material::{MaterialKind, MaterialRequest};
    use aimer_decoration::BackdropFilter;

    let ctx = context();
    let element = Container::new()
        .width(80)
        .height(40)
        .decoration(BackdropFilter::new().blur_radius(12.0))
        .padding(LayoutSpacing::all(3))
        .child(SizedBox::new())
        .to_element(&ctx);
    assert_eq!(
        element.computed_size(&ctx),
        ResolvedSize {
            width: 80.0,
            height: 40.0
        }
    );
    assert_eq!(
        element.content_size(&ctx),
        ResolvedSize {
            width: 74.0,
            height: 34.0
        }
    );
    let commands = record_local_v2(&element, &ctx);
    let [aimer_cupid::draw_cmd_v2::DrawCommand::DrawCustom { data, .. }] = commands.as_ref() else {
        panic!("the container must record one backdrop filter");
    };
    let request = MaterialRequest::decode(data).unwrap();
    assert_eq!(request.kind, MaterialKind::BackdropFilter);
    assert_eq!(request.bounds, [0.0, 0.0, 80.0, 40.0]);
    assert_eq!(request.blur_radius, 12.0);
    assert_eq!(
        element.event_tree_role(),
        aimer_widget::EventTreeRole::Transparent
    );
}

struct DrawProbe {
    draws: Rc<Cell<usize>>,
}

impl Drawable for DrawProbe {
    fn update(&self, _ctx: &BuildContext) {
        self.draws.set(self.draws.get() + 1);
    }
}

impl EventElement for DrawProbe {}
impl LayoutElement for DrawProbe {}
impl Rebuildable for DrawProbe {}

impl VisitorElement for DrawProbe {
    fn debug_name(&self) -> &'static str {
        "DrawProbe"
    }
}

struct PaintStableProbe;

impl Drawable for PaintStableProbe {
    fn update(&self, _ctx: &BuildContext) {}
}

impl EventElement for PaintStableProbe {}
impl LayoutElement for PaintStableProbe {}
impl Rebuildable for PaintStableProbe {}

impl VisitorElement for PaintStableProbe {
    fn debug_name(&self) -> &'static str {
        "PaintStableProbe"
    }
}

/// Records the space it is measured in, and answers with the width it was given.
struct ParentSizeProbe {
    seen: Rc<std::cell::RefCell<Vec<(f32, f32)>>>,
}

impl Drawable for ParentSizeProbe {
    fn update(&self, _ctx: &BuildContext) {}
}

impl EventElement for ParentSizeProbe {}
impl Rebuildable for ParentSizeProbe {}

impl LayoutElement for ParentSizeProbe {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.seen
            .borrow_mut()
            .push((ctx.box_constraint.max_width, ctx.parent_size.width));
        ResolvedSize {
            width: ctx.box_constraint.max_width,
            height: 10.0,
        }
    }
}

impl VisitorElement for ParentSizeProbe {
    fn debug_name(&self) -> &'static str {
        "ParentSizeProbe"
    }
}

/// A child that sizes itself from its parent's size (a `Row` with an automatic
/// width does) must see the same parent whether it is measured or laid out. The
/// layout caches below it are keyed by the constraint alone, so a measurement
/// made with the constraint of the inside and the parent size of the outside is
/// served back to the layout that has the inside's parent size.
#[tokio::test]
async fn a_container_measures_an_unbounded_child_in_the_space_it_lays_it_out_in() {
    for measure in [
        |container: &dyn LayoutElement, ctx: &BuildContext| {
            container.computed_size(ctx);
        },
        |container: &dyn LayoutElement, ctx: &BuildContext| {
            container.content_size(ctx);
        },
    ] {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut container = RawContainer::new(ParentSizeProbe { seen: seen.clone() });
        container.padding = LayoutSpacing::all(10);
        let mut ctx = context();
        // A scroll view offers an unbounded height, which makes the container
        // measure its child; the width is the container's own 200 and the
        // parent it sits in is 640 wide.
        ctx.parent_size = ResolvedSize {
            width: 640.0,
            height: 800.0,
        };
        ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 200.0,
            max_height: f32::MAX,
        };

        measure(&container, &ctx);

        let seen = seen.borrow();
        assert!(!seen.is_empty(), "the child was never measured");
        for (constraint_width, parent_width) in seen.iter() {
            assert_eq!(*constraint_width, 180.0);
            assert_eq!(
                *parent_width, 180.0,
                "the child was measured against a parent {parent_width} wide, but it is laid \
                 out in one {constraint_width} wide"
            );
        }
    }
}

#[tokio::test]
async fn retained_container_paint_includes_its_decoration() {
    let (ctx, inner) = recording_context();
    let mut element = RawContainer::new(PaintStableProbe);
    element.width = Dimension::Px(80.0);
    element.height = Dimension::Px(40.0);
    element.box_decoration = BoxDecoration::new()
        .background_color(Color::BLACK)
        .border_radius(8.0);

    let commands = record_local_v2(&element, &ctx);

    assert!(commands.iter().any(|command| matches!(
        command,
        aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { .. }
    )));
    assert!(
        inner.draw_list().commands().is_empty(),
        "the canvas itself is not painted"
    );
}

#[test]
fn containers_are_transparent_unless_they_occlude_scroll_and_never_cache_draw_bounds() {
    let child = DrawProbe {
        draws: Rc::new(Cell::new(0)),
    }
    .boxed();
    let mut container = RawContainer::new(child);

    assert_eq!(
        container.event_tree_role(),
        aimer_widget::EventTreeRole::Transparent
    );
    assert_eq!(container.event_tree_bounds(), None);

    container.box_decoration.update_color(Color::BLACK);
    assert_eq!(
        container.event_tree_role(),
        aimer_widget::EventTreeRole::IndexedTarget
    );
}

/// Counts every call that reaches the system allocator on this thread.
///
/// The count is what makes the migration's claim checkable: a widget hands
/// its fields to its element instead of copying them, so a warm rebuild of a
/// decorated tree must not reach the allocator at all.
struct RecordingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: Every method forwards to `System` unchanged; the counter only
// observes the call.
unsafe impl GlobalAlloc for RecordingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: RecordingAllocator = RecordingAllocator;

fn record() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

fn context() -> BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    BuildContext::new(
        canvas,
        ResolvedSize::default(),
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    )
}

fn recording_context() -> (BuildContext<'static>, &'static aimer_canvas::InnerCanvas) {
    let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
    let ctx = BuildContext::new(
        aimer_canvas::FrameCanvas::new(inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    (ctx, inner)
}

/// Records `element`'s own retained list, as the frame loop does for a node.
fn record_local_v2(
    element: &impl Drawable,
    ctx: &BuildContext<'_>,
) -> std::sync::Arc<[aimer_cupid::draw_cmd_v2::DrawCommand]> {
    let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
    let root = tree
        .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 400.0, 300.0))
        .unwrap();
    let node_context = tree.context(root).unwrap();
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        assert!(element.can_paint_local_v2(ctx));
        element.paint_local_v2(ctx);
    });
    tree.draw_list_snapshot(root).unwrap().commands
}

#[tokio::test]
async fn margin_keeps_outline_and_border_inside_the_decorated_box() {
    let (mut ctx, inner) = recording_context();
    ctx.box_constraint = aimer_attribute::BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 200.0,
        max_height: 160.0,
    };
    let border = BorderSlice::new().style(BorderStyle::Solid).stroke(4.0);
    let outline = BorderSlice::new().style(BorderStyle::Solid).stroke(6.0);

    let element = Container::new()
        .margin(LayoutSpacing::all(30))
        .padding(LayoutSpacing::all(8))
        .box_decoration(
            BoxDecoration::new()
                .border(BoxBorder::all(border))
                .outline(BoxOutline::all(outline)),
        )
        .child(crate::ZeroSizedBox)
        .to_element(&ctx);

    assert_eq!(
        element.content_size(&ctx),
        ResolvedSize {
            width: 116.0,
            height: 76.0,
        }
    );
    element.update(&ctx);

    let commands = inner.draw_list();
    assert!(matches!(
        commands.commands().first(),
        Some(DrawCommand::PushTransform { .. })
    ));
    let margin_transform = commands
        .commands()
        .get(1)
        .and_then(|command| match command {
            DrawCommand::SetTransform { matrix } => Some(*matrix),
            _ => None,
        });
    assert_eq!(
        margin_transform.map(|matrix| matrix.cols[2]),
        Some([30.0, 30.0, 1.0])
    );

    // The decorated box is recorded in the node's own list, offset by the
    // margin, so the margin itself is never filled.
    let recorded = record_local_v2(&element, &ctx);
    let fill = recorded.iter().find_map(|command| match command {
        aimer_cupid::draw_cmd_v2::DrawCommand::FillRect {
            rect,
            border_width,
            outline_width,
            ..
        } => Some((*rect, *border_width, *outline_width)),
        _ => None,
    });
    let (fill, border_width, outline_width) =
        fill.expect("the decorated container should record one fill rectangle");
    // The list is relative to the node, which starts at the outline's outer edge
    // (6 px outside the box), so the box lies margin plus outline inside it.
    assert_eq!(
        (fill.x, fill.y),
        (36.0, 36.0),
        "the box starts after the margin and the outline's reach"
    );
    assert_eq!((fill.width, fill.height), (140.0, 100.0));
    assert_eq!(border_width, [4.0; 4]);
    assert_eq!(outline_width, [6.0; 4]);

    let transforms = commands
        .commands()
        .iter()
        .filter_map(|command| match command {
            DrawCommand::SetTransform { matrix } => Some(matrix.cols[2]),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(transforms, vec![[30.0, 30.0, 1.0], [42.0, 42.0, 1.0]]);
}

/// A decoration owning a shadow list, the field this migration is about.
fn shadowed() -> BoxDecoration {
    BoxDecoration {
        box_shadow: vec![BoxShadow::default(), BoxShadow::default()],
        ..BoxDecoration::default()
    }
}

/// A 100x60 container with `decoration`, plus the radii it reports for its
/// child's clip and whether it can paint locally.
fn rounded_child_clip(decoration: BoxDecoration, scale: f32) -> (bool, [f32; 4]) {
    let (mut ctx, _inner) = recording_context();
    ctx.scale = scale;
    ctx.box_constraint = aimer_attribute::BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 400.0,
        max_height: 300.0,
    };
    let element = Container::new()
        .width(Dimension::Px(100.0))
        .height(Dimension::Px(60.0))
        .box_decoration(decoration)
        .child(crate::ZeroSizedBox)
        .to_element(&ctx);
    // The render tree walker asks the inner node, not the `AnyElement`
    // wrapper, which does not forward `can_paint_local_v2`.
    let node: &dyn Element = element.as_ref();
    let mut child = None;
    node.visit_children(&mut |element| child = Some(element));
    let child = child.expect("the container exposes its child");
    (
        node.can_paint_local_v2(&ctx),
        node.retained_v2_child_clip_radius(&ctx, child),
    )
}

#[tokio::test]
async fn a_rounded_container_paints_locally_and_rounds_its_childs_clip() {
    let (local, radii) = rounded_child_clip(BoxDecoration::new().border_radius(12), 1.0);
    assert!(local, "a border radius must not force the legacy path");
    assert_eq!(radii, [12.0; 4]);
}

#[tokio::test]
async fn a_square_container_reports_a_square_child_clip() {
    let (local, radii) = rounded_child_clip(BoxDecoration::new(), 1.0);
    assert!(local);
    assert_eq!(radii, [0.0; 4]);
}

#[tokio::test]
async fn the_child_clip_radius_shrinks_by_the_border_like_the_live_path() {
    let border = BorderSlice::new().style(BorderStyle::Solid).stroke(4.0);
    let decoration = BoxDecoration::new()
        .border(BoxBorder::all(border))
        .border_radius(12);
    let (local, radii) = rounded_child_clip(decoration, 1.0);
    assert!(local);
    assert_eq!(radii, [8.0; 4]);
}

#[tokio::test]
async fn a_radius_smaller_than_the_border_clamps_the_child_clip_to_square() {
    let border = BorderSlice::new().style(BorderStyle::Solid).stroke(4.0);
    let decoration = BoxDecoration::new()
        .border(BoxBorder::all(border))
        .border_radius(3);
    let (_, radii) = rounded_child_clip(decoration, 1.0);
    assert_eq!(radii, [0.0; 4]);
}

#[tokio::test]
async fn the_child_clip_radius_is_reported_in_logical_pixels_at_any_scale() {
    let border = BorderSlice::new().style(BorderStyle::Solid).stroke(4.0);
    let decoration = BoxDecoration::new()
        .border(BoxBorder::all(border))
        .border_radius(12);
    let (_, radii) = rounded_child_clip(decoration, 2.0);
    assert_eq!(radii, [8.0; 4]);
}

/// A container with `decoration` whose parent leaves both axes unbounded in
/// the given way, plus what its v2 geometry reports for its child.
struct UnboundedProbe {
    can_paint: bool,
    clip: Option<aimer_cupid::draw_cmd_v2::Rect>,
    child_max: Option<(f32, f32)>,
    outsets: Option<[f32; 4]>,
}

fn unbounded_container(max_width: f32, max_height: f32) -> UnboundedProbe {
    let (mut ctx, _inner) = recording_context();
    ctx.box_constraint = aimer_attribute::BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width,
        max_height,
    };
    let element = Container::new()
        .box_decoration(BoxDecoration::new().background_color(Color::BLUE))
        .child(crate::ZeroSizedBox)
        .to_element(&ctx);
    let node: &dyn Element = element.as_ref();
    let mut child = None;
    node.visit_children(&mut |element| child = Some(element));
    let child = child.expect("the container exposes its child");
    UnboundedProbe {
        can_paint: node.can_paint_local_v2(&ctx),
        clip: node
            .retained_v2_child_geometry(&ctx, child)
            .and_then(|(_, clip)| clip),
        child_max: node
            .retained_v2_child_context(&ctx, child)
            .map(|ctx| (ctx.box_constraint.max_width, ctx.box_constraint.max_height)),
        outsets: node.retained_v2_paint_outsets(&ctx),
    }
}

#[tokio::test]
async fn an_auto_height_container_in_a_scroll_view_paints_locally() {
    // A scrollable offers an unbounded height as `f32::MAX`.
    let probe = unbounded_container(400.0, f32::MAX);
    assert!(
        probe.can_paint,
        "an unbounded axis must not force the legacy path"
    );

    // The render tree gets finite, sane rectangles: `f32::MAX` doubles to
    // infinity when scaled to device pixels.
    let clip = probe.clip.expect("the child is clipped");
    assert!(clip.width.is_finite() && clip.height.is_finite());
    assert!(clip.height <= LOCAL_V2_CONTAINER_EXTENT_LIMIT, "{clip:?}");
    assert!(
        probe
            .outsets
            .is_some_and(|outsets| outsets.iter().all(|v| v.is_finite()))
    );
}

#[tokio::test]
async fn the_child_still_sees_an_unbounded_axis_as_unbounded() {
    // Clamping the *render tree's* rectangles must not change layout: a
    // child sizes itself to its content only while its constraint stays
    // above the unbounded threshold.
    let probe = unbounded_container(400.0, f32::MAX);
    let (_, max_height) = probe.child_max.expect("child context");
    assert!(
        max_height > LOCAL_V2_CONTAINER_EXTENT_LIMIT,
        "child max height was clamped to {max_height}"
    );
}

#[tokio::test]
async fn an_auto_width_container_under_an_unbounded_width_paints_locally() {
    // Row-like parents offer a quarter of `f32::MAX` per child.
    let probe = unbounded_container(f32::MAX / 4.0, 120.0);
    assert!(probe.can_paint);
    let clip = probe.clip.expect("the child is clipped");
    assert!(clip.width.is_finite() && clip.width <= LOCAL_V2_CONTAINER_EXTENT_LIMIT);
    let (max_width, _) = probe.child_max.expect("child context");
    assert!(max_width > LOCAL_V2_CONTAINER_EXTENT_LIMIT);
}

fn fit_probe(fit: bool, max_width: f32, max_height: f32) -> (ResolvedSize, ResolvedSize) {
    let mut ctx = context();
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width,
        max_height,
    };
    let mut container = Container::new()
        .padding(LayoutSpacing::all(5))
        .margin(LayoutSpacing::all(2));
    if fit {
        container = container.fit_content();
    }
    let element = container
        .child(SizedBox::new().width(30).height(20))
        .to_element(&ctx);
    (element.computed_size(&ctx), element.content_size(&ctx))
}

#[tokio::test]
async fn an_auto_container_fills_a_bounded_constraint_unless_it_fits_its_content() {
    let (filled, _) = fit_probe(false, 200.0, 100.0);
    assert_eq!((filled.width, filled.height), (200.0, 100.0));

    // 30 x 20 child + 5 padding and 2 margin on every side.
    let (fitted, content) = fit_probe(true, 200.0, 100.0);
    assert_eq!((fitted.width, fitted.height), (44.0, 34.0));
    assert_eq!((content.width, content.height), (30.0, 20.0));
}

#[tokio::test]
async fn a_fitted_container_never_outgrows_its_constraint() {
    let (fitted, _) = fit_probe(true, 30.0, 20.0);
    assert_eq!((fitted.width, fitted.height), (30.0, 20.0));
}

#[tokio::test]
async fn a_fitted_container_falls_back_when_the_child_reports_no_extent() {
    for (max_width, max_height, expected) in [
        // A row offers an unbounded width and a bounded height.
        (f32::MAX / 4.0, 100.0, (200.0 + 14.0, 60.0 + 14.0)),
        (400.0, 300.0, (200.0 + 14.0, 60.0 + 14.0)),
        // The fallback is still capped by what the constraint allows.
        (100.0, 40.0, (100.0, 40.0)),
    ] {
        let mut ctx = context();
        ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width,
            max_height,
        };
        let element = Container::new()
            .padding(LayoutSpacing::all(5))
            .margin(LayoutSpacing::all(2))
            .fit_content()
            .child(SizedBox::new())
            .to_element(&ctx);
        let size = element.computed_size(&ctx);
        assert_eq!((size.width, size.height), expected, "{max_width} x {max_height}");
    }
}

/// A button's label usually sits in an automatic container of its own for
/// padding. That inner container fills whatever it is offered, so the fitted
/// one has to measure it without a bound or it would hug nothing.
#[tokio::test]
async fn a_fitted_container_hugs_an_automatic_container_child() {
    let mut ctx = context();
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 400.0,
        max_height: 300.0,
    };
    let element = Container::new()
        .fit_content()
        .child(
            Container::new()
                .padding(LayoutSpacing::new().left(24).right(24).top(12).bottom(12))
                .child(SizedBox::new().width(50).height(10)),
        )
        .to_element(&ctx);
    let size = element.computed_size(&ctx);
    assert_eq!((size.width, size.height), (98.0, 34.0));
    let content = element.content_size(&ctx);
    assert_eq!((content.width, content.height), (98.0, 34.0));

    // Content wider than the room is capped, and is measured against that room.
    ctx.box_constraint.max_width = 60.0;
    let size = element.computed_size(&ctx);
    assert_eq!((size.width, size.height), (60.0, 34.0));
}

#[tokio::test]
async fn a_fitted_container_keeps_an_explicit_size_and_hugs_the_other_axis() {
    let mut ctx = context();
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 200.0,
        max_height: 100.0,
    };
    let element = Container::new()
        .width(120)
        .fit_content()
        .child(SizedBox::new().width(30).height(20))
        .to_element(&ctx);
    let size = element.computed_size(&ctx);
    assert_eq!((size.width, size.height), (120.0, 20.0));
}

#[tokio::test]
async fn a_bounded_container_is_unchanged_by_the_unbounded_handling() {
    let probe = unbounded_container(400.0, 300.0);
    assert!(probe.can_paint);
    let clip = probe.clip.expect("the child is clipped");
    assert_eq!((clip.width, clip.height), (400.0, 300.0));
    assert_eq!(probe.child_max, Some((400.0, 300.0)));
}

#[tokio::test]
async fn offscreen_container_skips_unknown_child_draw_but_keeps_traversal_views() {
    let draws = Rc::new(Cell::new(0));
    let container = RawContainer::new(DrawProbe {
        draws: draws.clone(),
    });
    let mut ctx = context();
    ctx.parent_size = ResolvedSize {
        width: 100.0,
        height: 100.0,
    };
    ctx.box_constraint = aimer_attribute::BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 100.0,
        max_height: 100.0,
    };
    ctx.visible_rect = Some((0.0, 101.0, 100.0, 20.0));

    container.update(&ctx);

    assert_eq!(
        draws.get(),
        0,
        "a clipped container must not dynamically dispatch an offscreen child"
    );

    let mut structural = 0;
    container.structural_children(&mut |_| structural += 1);
    assert_eq!(structural, 1);

    let mut events = 0;
    container.event_children(&mut |_| events += 1);
    assert_eq!(events, 1);

    let mut hit_test = 0;
    container.hit_test_children(&mut |_| hit_test += 1);
    assert_eq!(hit_test, 1);

    ctx.visible_rect = Some((0.0, 0.0, 100.0, 100.0));
    container.update(&ctx);
    assert_eq!(
        draws.get(),
        1,
        "an unknown-bounds child remains drawable when its clipping parent is visible"
    );
}

#[test]
fn the_counter_counts() {
    let before = allocations();
    let allocated = vec![0u8; 64];

    assert!(
        allocations() > before,
        "a test asserting zero allocations is worthless if nothing is counted"
    );
    drop(allocated);
}

#[tokio::test]
async fn converting_a_decorated_container_adds_no_allocation() {
    let ctx = context();
    let memory = aimer_rubick::UiMemory::new(2 * 1024 * 1024);
    let allocator = memory.allocator();

    allocator.scope(|| {
        // App builds use the UI heap, including Rubick's provenance header.
        // Warm its storage before measuring the steady-state conversion.
        for _ in 0..4 {
            drop(
                Container::new()
                    .box_decoration(shadowed())
                    .child(SizedBox::new())
                    .to_element(&ctx),
            );
        }

        let before_decoration = allocations();
        let decoration = shadowed();
        let describing = allocations() - before_decoration;

        let committed_before_build = memory.committed_bytes();
        let before_build = allocations();
        drop(
            Container::new()
                .box_decoration(decoration)
                .child(SizedBox::new())
                .to_element(&ctx),
        );
        let building = allocations() - before_build;
        assert_eq!(memory.committed_bytes(), committed_before_build);

        assert_eq!(
            describing, 1,
            "the shadow list is the one allocation a decorated container costs, \
             and it is paid when the decoration is described"
        );
        assert_eq!(
            building, 0,
            "the conversion must hand the shadow list to the element rather than \
             copy it, and must reuse the UI heap: a warm build of a decorated \
             container reaches the global allocator zero times"
        );
    });
}

/// Keep the container payload within its 512-byte size budget. Rubick's
/// provenance header is additional storage; the app-owned UI heap serves
/// the combined allocation, as checked by the conversion test above.
///
/// The element is dominated by its [`BoxDecoration`], which is dominated by
/// its eight border and outline colors. That is why a color is stored as one
/// packed word: with a color wide enough to carry HSLA components this
/// element measured 576 bytes and spilled out of the pool.
#[test]
fn a_container_element_stays_within_its_size_budget() {
    eprintln!("Container Size : {}", size_of::<Container>());
    eprintln!(
        "Container element size: {}",
        size_of::<RawContainer<AnyElement>>()
    );

    assert!(
        size_of::<RawContainer<AnyElement>>() <= 512,
        "a container element grew past its payload size budget: {} bytes",
        size_of::<RawContainer<AnyElement>>()
    );
}

struct CustomDecoration {
    insets: [f32; 4],
    clip: Option<[f32; 4]>,
    revision: Rc<Cell<u64>>,
}

impl Decoration for CustomDecoration {
    fn paint(&self, canvas: &aimer_canvas::Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        let color = if self.revision.get() == 0 {
            Color::RED
        } else {
            Color::BLUE
        };
        canvas.fill_color_rect(origin, size, color);
    }

    fn content_insets(&self, _size: ResolvedSize, _scale: f32) -> [f32; 4] {
        self.insets
    }

    fn child_clip_radius(&self, _size: ResolvedSize, _scale: f32) -> Option<[f32; 4]> {
        self.clip
    }

    fn revision(&self) -> u64 {
        self.revision.get()
    }
}

#[tokio::test]
async fn custom_decoration_geometry_is_applied_in_logical_pixels_through_trait_objects() {
    use aimer_cupid::draw_cmd_v2::Rect;
    let (mut ctx, _) = recording_context();
    ctx.scale = 2.0;
    let decoration: Rc<dyn Decoration> = Rc::new(CustomDecoration {
        insets: [2.0, 4.0, 6.0, 8.0],
        clip: Some([5.0, 6.0, 7.0, 8.0]),
        revision: Rc::new(Cell::new(0)),
    });
    let element = Container::new()
        .decoration(decoration)
        .width(80)
        .height(40)
        .margin(LayoutSpacing::all(5))
        .padding(LayoutSpacing::all(3))
        .child(SizedBox::new().width(24).height(12))
        .to_element(&ctx);
    assert_eq!(
        element.computed_size(&ctx),
        ResolvedSize {
            width: 180.0,
            height: 100.0
        }
    );
    assert_eq!(
        element.content_size(&ctx),
        ResolvedSize {
            width: 132.0,
            height: 44.0
        }
    );
    element.visit_children(&mut |child| {
        assert_eq!(
            element.retained_v2_child_clip_radius(&ctx, child),
            [5.0, 6.0, 7.0, 8.0]
        );
        let (bounds, clip) = element.retained_v2_child_geometry(&ctx, child).unwrap();
        assert_eq!(bounds, Rect::new(10.0, 12.0, 24.0, 12.0));
        assert_eq!(clip, Some(Rect::new(-3.0, -3.0, 72.0, 28.0)));
    });
}

#[tokio::test]
async fn custom_decoration_revisions_refresh_paint_and_invalid_insets_are_sanitized() {
    let (ctx, _) = recording_context();
    let revision = Rc::new(Cell::new(0));
    let decoration: Box<dyn Decoration> = Box::new(CustomDecoration {
        insets: [f32::NAN, -1.0, f32::INFINITY, 3.0],
        clip: None,
        revision: revision.clone(),
    });
    let element = Container::new()
        .decoration(decoration)
        .width(80)
        .height(40)
        .child(SizedBox::new())
        .to_element(&ctx);
    assert_eq!(
        element.content_size(&ctx),
        ResolvedSize {
            width: 80.0,
            height: 37.0
        }
    );
    element.visit_children(&mut |child| {
        assert_eq!(
            element.retained_v2_child_geometry(&ctx, child).unwrap().1,
            None
        );
    });
    record_local_v2(&element, &ctx);
    assert!(!element.local_v2_paint_needs_recording(&ctx));
    for next in [1, 1 << 32, u64::MAX, 0] {
        revision.set(next);
        assert!(element.local_v2_paint_needs_recording(&ctx));
        let commands = record_local_v2(&element, &ctx);
        let expected = if next == 0 { (255, 0) } else { (0, 255) };
        assert!(
            matches!(commands.as_ref(), [aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { color, .. }]
            if (color.r, color.b) == expected)
        );
        assert!(!element.local_v2_paint_needs_recording(&ctx));
    }
}

#[tokio::test]
async fn decoration_method_preserves_box_borders_clipping_and_fallback_color() {
    let (mut ctx, _) = recording_context();
    ctx.scale = 2.0;
    let element = Container::new()
        .width(80)
        .height(40)
        .color(Color::BLUE)
        .decoration(
            BoxDecoration::new()
                .border_radius(12)
                .border(BoxBorder::all(
                    BorderSlice::new().style(BorderStyle::Solid).stroke(4),
                )),
        )
        .child(SizedBox::new())
        .to_element(&ctx);
    assert_eq!(
        element.content_size(&ctx),
        ResolvedSize {
            width: 144.0,
            height: 64.0
        }
    );
    element.visit_children(&mut |child| {
        assert_eq!(element.retained_v2_child_clip_radius(&ctx, child), [8.0; 4]);
    });
    let commands = record_local_v2(&element, &ctx);
    assert!(
        matches!(commands.as_ref(), [aimer_cupid::draw_cmd_v2::DrawCommand::FillRect {
        color, border_radius, border_width, ..
    }] if color.b == 255 && color.a == 255 && *border_radius == [12.0; 4] && *border_width == [4.0; 4])
    );
    assert_eq!(element.event_tree_role(), EventTreeRole::IndexedTarget);
}

#[tokio::test]
async fn generic_decoration_conversion_and_retained_checks_allocate_nothing_after_warmup() {
    use aimer_decoration::Magnification;
    let (ctx, _) = recording_context();
    for _ in 0..4 {
        drop(
            Container::new()
                .decoration(Magnification::new())
                .width(80)
                .height(40)
                .child(SizedBox::new())
                .to_element(&ctx),
        );
    }
    let before = allocations();
    let element = Container::new()
        .decoration(Magnification::new())
        .width(80)
        .height(40)
        .child(SizedBox::new())
        .to_element(&ctx);
    let building = allocations() - before;
    assert_eq!(
        building, 0,
        "concrete decorations must remain in pooled element storage"
    );
    record_local_v2(&element, &ctx);
    let before = allocations();
    let unchanged = (0..1_024).all(|_| !element.local_v2_paint_needs_recording(&ctx));
    let checking = allocations() - before;
    assert!(unchanged);
    assert_eq!(checking, 0);
}
