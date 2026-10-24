use std::cell::Cell;
use std::rc::Rc;

use crate::{BorderSlice, BorderStyle, BoxBorder, BoxDecoration, BoxOutline, BoxShadow};
use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
use aimer_canvas::{Canvas, FrameCanvas, InnerCanvas};
use aimer_color::prelude::Color;
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderOp, RenderTree};
use aimer_style::LayoutSpacing;
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{Drawable, EventDispatcher, EventTreeRole, FocusNode, Focusable, Widget};

use crate::{Decoration, DecorationBox};
use aimer_container::{Padding, SizedBox, ZeroSizedBox};

#[test]
fn backdrop_filter_records_a_plain_filter_in_logical_coordinates() {
    use crate::BackdropFilter;
    use aimer_cupid::material::{MaterialKind, MaterialRequest};

    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 200.0, 160.0)).unwrap();
    let canvas = Canvas::of(&tree.context(root).unwrap());
    BackdropFilter::new()
        .blur_radius(12.0)
        .saturation(0.5)
        .brightness(1.25)
        .contrast(0.75)
        .paint(
            &canvas,
            Vec2d { x: 3.0, y: 4.0 },
            ResolvedSize {
                width: 80.0,
                height: 40.0,
            },
            2.0,
        );
    canvas.finish();
    let list = tree.draw_list_snapshot(root).unwrap();
    let [
        DrawCommand::DrawCustom {
            pipeline_name,
            data,
        },
    ] = list.commands.as_ref()
    else {
        panic!("a filter records one material command without a painted fallback");
    };
    assert_eq!(pipeline_name.as_ref(), "aimer.material");
    let request = MaterialRequest::decode(data).unwrap();
    assert_eq!(request.kind, MaterialKind::BackdropFilter);
    assert_eq!(request.bounds, [3.0, 4.0, 80.0, 40.0]);
    assert_eq!(request.blur_radius, 12.0);
    assert_eq!(
        [request.saturation, request.brightness, request.contrast],
        [0.5, 1.25, 0.75]
    );
    assert_eq!(request.corner_radii, [0.0; 4]);
    assert_eq!(request.tint, [0.0; 4]);
    assert_eq!(request.border_width, 0.0);
    assert_eq!(request.shadow_color, [0.0; 4]);
    assert_eq!(request.edge_lighting, 0.0);
    assert_eq!(request.specular_highlight, 0.0);
}

#[test]
fn backdrop_filter_bounds_inputs_and_skips_empty_or_identity_paint() {
    use crate::BackdropFilter;
    use aimer_cupid::material::MaterialRequest;

    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 20.0, 10.0)).unwrap();
    let record = |filter: BackdropFilter, size: ResolvedSize| {
        let canvas = Canvas::of(&tree.context(root).unwrap());
        filter.paint(&canvas, Vec2d::default(), size, 1.0);
        canvas.finish();
        tree.draw_list_snapshot(root).unwrap()
    };
    let size = ResolvedSize {
        width: 20.0,
        height: 10.0,
    };
    let filter = BackdropFilter::new()
        .blur_radius(200.0)
        .saturation(-1.0)
        .brightness(5.0)
        .contrast(f32::NAN);
    let list = record(filter, size);
    let DrawCommand::DrawCustom { data, .. } = &list.commands[0] else {
        panic!("material command")
    };
    let request = MaterialRequest::decode(data).unwrap();
    assert_eq!(
        [
            request.blur_radius,
            request.saturation,
            request.brightness,
            request.contrast
        ],
        [96.0, 0.0, 3.0, 1.0]
    );

    let filter = BackdropFilter::new()
        .blur_radius(f32::INFINITY)
        .saturation(f32::NAN)
        .brightness(f32::NEG_INFINITY)
        .contrast(f32::INFINITY);
    assert_eq!(filter, BackdropFilter::default());
    assert_eq!(filter.paint_outsets(size, 2.0), [0.0; 4]);
    assert_eq!(filter.revision(), 0);
    assert!(
        record(BackdropFilter::new().blur_radius(-1.0), size)
            .commands
            .is_empty()
    );
    assert!(
        record(
            filter,
            ResolvedSize {
                width: 0.0,
                height: 10.0
            }
        )
        .commands
        .is_empty()
    );
    assert!(
        record(
            filter,
            ResolvedSize {
                width: 20.0,
                height: 0.0
            }
        )
        .commands
        .is_empty()
    );
}

pub(super) fn context(scale: f32) -> BuildContext<'static> {
    let inner = Box::leak(Box::new(InnerCanvas::new()));
    let mut ctx = BuildContext::new(
        FrameCanvas::new(inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        scale,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 200.0,
        max_height: 160.0,
    };
    ctx
}

struct MutableBackground {
    color: Rc<Cell<Color>>,
    revision: Rc<Cell<u64>>,
}

impl Decoration for MutableBackground {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        canvas.fill_color_rect(origin, size, self.color.get());
    }

    fn revision(&self) -> u64 {
        self.revision.get()
    }
}

#[tokio::test]
async fn decoration_box_revision_repaints_only_its_background() {
    let ctx = context(2.0);
    let color = Rc::new(Cell::new(Color::RED));
    let revision = Rc::new(Cell::new(0));
    let element = DecorationBox::new()
        .decoration(MutableBackground {
            color: color.clone(),
            revision: revision.clone(),
        })
        .child(SizedBox::new().width(24.0).height(12.0).color(Color::BLUE))
        .to_element(&ctx);
    let sibling = SizedBox::new()
        .width(16.0)
        .height(10.0)
        .color(Color::GREEN)
        .to_element(&ctx);
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 24.0, 12.0)).unwrap();
    let child_node = tree
        .add_child(root, Rect::new(0.0, 0.0, 24.0, 12.0))
        .unwrap();
    let sibling_node = tree.add_root(Rect::new(40.0, 0.0, 16.0, 10.0)).unwrap();
    let mut nodes =
        aimer_widget::ElementNodeMap::from([(element.id(), root), (sibling.id(), sibling_node)]);
    let mut child_id = None;
    element.visit_children(&mut |child| {
        nodes.insert(child.id(), child_node);
        child_id = Some(child.id());
    });
    let nodes = Rc::new(nodes);
    let frame = || {
        ctx.canvas.begin_frame();
        aimer_widget::begin_paint_frame(200, 160);
        aimer_widget::with_v2_render_tree_context(tree.clone(), nodes.clone(), None, || {
            element.update(&ctx);
            sibling.update(&ctx);
        });
    };
    frame();
    frame();
    assert_eq!(tree.draw_list_revision(root).unwrap(), 1);
    let order: Vec<_> = tree
        .render_all()
        .iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item.element),
            _ => None,
        })
        .collect();
    assert_eq!(order, vec![root, child_node, sibling_node]);
    tree.take_damage();
    color.set(Color::BLACK);
    revision.set(1);
    frame();

    assert_eq!(tree.draw_list_revision(root).unwrap(), 2);
    assert_eq!(tree.draw_list_revision(child_node).unwrap(), 1);
    assert_eq!(tree.draw_list_revision(sibling_node).unwrap(), 1);
    assert_eq!(tree.take_damage(), vec![Rect::new(0.0, 0.0, 24.0, 12.0)]);
    assert!(
        matches!(tree.draw_list_snapshot(root).unwrap().commands.as_ref(),
        [DrawCommand::FillRect { color, .. }] if color.a == 255 && color.r == 0)
    );
    element.visit_children(&mut |child| assert_eq!(Some(child.id()), child_id));
}

pub(super) fn record(element: &impl Drawable, ctx: &BuildContext) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 200.0, 160.0)).unwrap();
    ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
        assert!(element.can_paint_local_v2(ctx));
        element.paint_local_v2(ctx);
    });
    tree.draw_list_snapshot(root).unwrap().commands.to_vec()
}

#[tokio::test]
async fn decoration_box_paints_the_childs_size_without_border_insets() {
    let ctx = context(2.0);
    let decoration = BoxDecoration::new()
        .background_color(Color::RED)
        .border(BoxBorder::all(
            BorderSlice::new().style(BorderStyle::Solid).stroke(3.0),
        ))
        .border_radius(4.0);
    let element = DecorationBox::new()
        .decoration(decoration)
        .child(SizedBox::new().width(24.0).height(12.0).color(Color::BLUE))
        .to_element(&ctx);

    assert_eq!(
        element.computed_size(&ctx),
        ResolvedSize {
            width: 48.0,
            height: 24.0
        }
    );
    assert_eq!(
        element.content_size(&ctx),
        ResolvedSize {
            width: 48.0,
            height: 24.0
        }
    );
    element.visit_children(&mut |child| {
        assert_eq!(child.computed_size(&ctx), element.computed_size(&ctx));
        assert_eq!(element.retained_v2_child_clip_radius(&ctx, child), [0.0; 4]);
    });
    assert_eq!(element.retained_clip(&ctx), None);
    let commands = record(&element, &ctx);
    assert!(matches!(commands.as_slice(), [DrawCommand::FillRect {
        rect, color, border_width, border_radius, ..
    }] if *rect == Rect::new(0.0, 0.0, 24.0, 12.0)
        && (color.r, color.g, color.b, color.a) == (255, 0, 0, 255)
        && *border_width == [3.0; 4] && *border_radius == [4.0; 4]));
    assert!(
        ctx.canvas
            .get_inner_canvas()
            .draw_list()
            .commands()
            .is_empty()
    );
}

#[tokio::test]
async fn decoration_box_shadow_and_outline_bounds_keep_the_box_at_its_layout_origin() {
    let ctx = context(2.0);
    let element = DecorationBox::new()
        .decoration(
            BoxDecoration::new()
                .background_color(Color::RED)
                .outline(BoxOutline::all(
                    BorderSlice::new().style(BorderStyle::Solid).stroke(3.0),
                ))
                .add_shadow(
                    BoxShadow::new()
                        .blur(2.0)
                        .spread(2.0)
                        .offset_x(4.0)
                        .offset_y(-2.0),
                ),
        )
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);

    assert_eq!(
        element.retained_v2_paint_outsets(&ctx),
        Some([3.0, 5.0, 6.0, 3.0])
    );
    assert_eq!(
        element.retained_v2_bounds(&ctx),
        Some(ResolvedSize {
            width: 48.0,
            height: 24.0
        })
    );
    assert!(matches!(record(&element, &ctx).as_slice(),
        [DrawCommand::DrawShadowRect { rect: shadow, .. }, DrawCommand::FillRect { rect, .. }]
        if *shadow == Rect::new(3.0, 5.0, 24.0, 12.0) && *rect == *shadow));
    element.visit_children(&mut |child| {
        let child_ctx = element.retained_v2_child_context(&ctx, child).unwrap();
        assert_eq!(child_ctx.box_constraint, ctx.box_constraint);
        assert_eq!(child_ctx.parent_size, ctx.parent_size);
        assert_eq!(element.retained_v2_child_geometry(&ctx, child), None);
    });
}

#[tokio::test]
async fn decoration_box_accepts_owned_and_shared_trait_objects() {
    let ctx = context(1.0);
    let owned: Box<dyn Decoration> = Box::new(MutableBackground {
        color: Rc::new(Cell::new(Color::RED)),
        revision: Rc::new(Cell::new(0)),
    });
    let owned = DecorationBox::new()
        .decoration(owned)
        .box_child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    assert!(matches!(record(&owned, &ctx).as_slice(),
        [DrawCommand::FillRect { color, .. }] if color.r == 255 && color.a == 255));

    let source = Rc::new(BoxDecoration::new().background_color(Color::BLUE));
    let shared: Rc<dyn Decoration> = source.clone();
    let element = DecorationBox::new()
        .decoration(shared)
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    record(&element, &ctx);
    assert!(!element.local_v2_paint_needs_recording(&ctx));
    source.update_color(Color::GREEN);
    assert!(element.local_v2_paint_needs_recording(&ctx));
    assert!(matches!(record(&element, &ctx).as_slice(),
        [DrawCommand::FillRect { color, .. }] if color.g == 255 && color.a == 255));
    assert!(!element.local_v2_paint_needs_recording(&ctx));
}

#[tokio::test]
async fn decoration_box_invalid_geometry_records_an_empty_stable_list() {
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let ctx = context(scale);
        let element = DecorationBox::new()
            .decoration(MutableBackground {
                color: Rc::new(Cell::new(Color::RED)),
                revision: Rc::new(Cell::new(0)),
            })
            .child(SizedBox::new().width(24.0).height(12.0))
            .to_element(&ctx);
        assert!(record(&element, &ctx).is_empty(), "invalid scale {scale}");
        assert!(!element.local_v2_paint_needs_recording(&ctx));
    }
    let ctx = context(1.0);
    for width in [-1.0, f32::NAN, f32::INFINITY] {
        let element = DecorationBox::new()
            .decoration(BoxDecoration::new().background_color(Color::RED))
            .child(SizedBox::new().width(width).height(12.0))
            .to_element(&ctx);
        assert!(record(&element, &ctx).is_empty(), "invalid width {width}");
        assert!(!element.local_v2_paint_needs_recording(&ctx));
    }
}

struct OutsetBackground([f32; 4]);

impl Decoration for OutsetBackground {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        canvas.fill_color_rect(origin, size, Color::RED);
    }

    fn paint_outsets(&self, _size: ResolvedSize, _scale: f32) -> [f32; 4] {
        self.0
    }
}

#[tokio::test]
async fn decoration_box_invalid_outsets_do_not_poison_the_paint_origin() {
    let ctx = context(1.0);
    let decoration: Rc<dyn Decoration> =
        Rc::new(OutsetBackground([f32::NAN, -1.0, f32::INFINITY, 3.0]));
    let element = DecorationBox::new()
        .decoration(decoration)
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);

    assert_eq!(
        element.retained_v2_paint_outsets(&ctx),
        Some([0.0, 0.0, 0.0, 3.0])
    );
    assert!(matches!(record(&element, &ctx).as_slice(),
        [DrawCommand::FillRect { rect, .. }] if *rect == Rect::new(0.0, 0.0, 24.0, 12.0)));
}

#[tokio::test]
async fn decoration_box_nested_wrappers_preserve_spacing_and_flex_layout() {
    let mut ctx = context(2.0);
    ctx.visible_rect = Some((10.0, 20.0, 80.0, 60.0));
    let content = || {
        Padding::new()
            .padding(LayoutSpacing::all(3))
            .child(SizedBox::new().width(24.0).height(12.0))
    };
    let plain = content().to_element(&ctx);
    let wrapped = DecorationBox::new()
        .child(DecorationBox::new().child(content()))
        .to_element(&ctx);
    for max_height in [160.0, f32::MAX] {
        ctx.box_constraint.max_height = max_height;
        plain.invalidate_layout();
        wrapped.invalidate_layout();
        assert_eq!(wrapped.layout(&ctx), plain.layout(&ctx));
        assert_eq!(wrapped.computed_size(&ctx), plain.computed_size(&ctx));
        assert_eq!(wrapped.content_size(&ctx), plain.content_size(&ctx));
        assert_eq!(wrapped.get_size_from_child(), plain.get_size_from_child());
        assert_eq!(wrapped.is_layout_stable(), plain.is_layout_stable());
        wrapped.visit_children(&mut |child| {
            let child_ctx = wrapped.retained_v2_child_context(&ctx, child).unwrap();
            assert_eq!(child_ctx.box_constraint, ctx.box_constraint);
            assert_eq!(child_ctx.visible_rect, ctx.visible_rect);
        });
    }
    let expanded = DecorationBox::new()
        .child(aimer_flex::Expanded::new().flex(2.0).child(ZeroSizedBox))
        .to_element(&ctx);
    assert_eq!(expanded.flex(), Some(2.0));
    assert_eq!(expanded.get_size_from_child(), None);
}

#[tokio::test]
async fn decoration_box_empty_and_zero_sized_children_are_supported() {
    let ctx = context(1.0);
    let empty = DecorationBox::default()
        .child(ZeroSizedBox)
        .to_element(&ctx);
    assert_eq!(empty.layout(&ctx), ResolvedSize::default());
    assert!(record(&empty, &ctx).is_empty());
    assert!(!empty.local_v2_paint_needs_recording(&ctx));

    let decorated = DecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::RED))
        .child(ZeroSizedBox)
        .to_element(&ctx);
    assert!(matches!(record(&decorated, &ctx).as_slice(),
        [DrawCommand::FillRect { rect, .. }] if *rect == Rect::new(0.0, 0.0, 0.0, 0.0)));
}

#[tokio::test]
async fn decoration_box_leaves_events_and_focus_on_its_child() {
    let ctx = context(1.0);
    let focus = FocusNode::new();
    let element = DecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::RED))
        .child(Focusable::new().node(focus.clone()).child(ZeroSizedBox))
        .to_element(&ctx);
    assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);
    assert!(element.focus_node().is_none());
    let mut event_ids = Vec::new();
    element.event_children(&mut |child| {
        event_ids.push(child.id());
    });
    let mut structural_ids = Vec::new();
    element.structural_children(&mut |child| structural_ids.push(child.id()));
    assert_eq!(structural_ids, event_ids);
    assert_eq!(event_ids.len(), 1);
    focus.request_focus();
    let result = EventDispatcher::new().dispatch(
        element.as_ref(),
        Vec2d::default(),
        &aimer_events::element::ElementEvent::Cancel,
    );
    assert!(!result.is_consumed());
    assert!(focus.has_focus());
}

#[tokio::test]
async fn decoration_box_scale_changes_refresh_its_local_paint() {
    let mut ctx = context(1.0);
    let element = DecorationBox::new()
        .decoration(
            BoxDecoration::new()
                .background_color(Color::RED)
                .add_shadow(BoxShadow::new().blur(4.0)),
        )
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    record(&element, &ctx);
    assert!(!element.local_v2_paint_needs_recording(&ctx));
    ctx.scale = 2.0;
    assert!(element.local_v2_paint_needs_recording(&ctx));
    assert!(matches!(record(&element, &ctx).as_slice(),
        [DrawCommand::DrawShadowRect { shadow_params, rect, .. }, DrawCommand::FillRect { .. }]
        if shadow_params[2] == 2.0 && *rect == Rect::new(6.0, 6.0, 24.0, 12.0)));
    assert!(!element.local_v2_paint_needs_recording(&ctx));
}

#[cfg(feature = "portable-guest")]
#[test]
fn decoration_box_reports_unsupported_portable_lowering() {
    use aimer_widget::PortableWidget;
    use aimer_widget::portable::{
        PortableBuildContext, PortableBuildError, PortableLimits, PortableWidgetLimits,
        SourceFingerprint, StableId128,
    };
    let mut ctx = PortableBuildContext::new(
        7,
        11,
        PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
        PortableLimits::new(8, 16, 64, 128, 1_024),
    )
    .unwrap();
    let source = SourceFingerprint::new(StableId128::from_path("test", "decoration-box"));
    let result = DecorationBox::new()
        .child(ZeroSizedBox)
        .to_portable_node(&mut ctx, source);
    assert!(matches!(result, Err(PortableBuildError::UnsupportedWidget {
        widget: "DecorationBox", source: actual,
    }) if actual == source));
}

#[tokio::test]
async fn decoration_box_constraint_changes_refresh_its_local_paint() {
    let mut ctx = context(1.0);
    let element = DecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::RED))
        .child(
            SizedBox::new()
                .width(aimer_attribute::Dimension::Percent(50.0))
                .height(12.0),
        )
        .to_element(&ctx);
    assert!(matches!(record(&element, &ctx).as_slice(),
        [DrawCommand::FillRect { rect, .. }] if *rect == Rect::new(0.0, 0.0, 100.0, 12.0)));
    ctx.box_constraint.max_width = 100.0;
    assert!(element.local_v2_paint_needs_recording(&ctx));
    assert!(matches!(record(&element, &ctx).as_slice(),
        [DrawCommand::FillRect { rect, .. }] if *rect == Rect::new(0.0, 0.0, 50.0, 12.0)));
    assert!(!element.local_v2_paint_needs_recording(&ctx));
}
