use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
use aimer_canvas::{Canvas, FrameCanvas, InnerCanvas};
use aimer_color::prelude::Color;
use aimer_container::{SizedBox, ZeroSizedBox};
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use aimer_decoration::{BoxDecoration, Decoration, LightEdge, MultiDecorationBox};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{
    Drawable, Element, EventDispatcher, EventTreeRole, FocusNode, Focusable, Widget,
};

fn context(inner: &InnerCanvas, scale: f32) -> BuildContext<'_> {
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

fn record(element: &dyn Element, ctx: &BuildContext) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 200.0, 160.0)).unwrap();
    ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
        element.paint_local_v2(ctx)
    });
    tree.draw_list_snapshot(root).unwrap().commands.to_vec()
}

// Deliberately non-Clone: the widget must move owned decorations.
struct Marker(u8);
impl Decoration for Marker {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        canvas.fill_color_rect(origin, size, Color::Rgb(self.0, 0, 0));
    }
}

#[tokio::test]
async fn multi_decoration_appends_typed_iterator_and_boxed_layers_before_child() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner, 1.0);
    let layers: Vec<Box<dyn Decoration>> = vec![Box::new(Marker(4)), Box::new(LightEdge::new())];
    let element = MultiDecorationBox::new()
        .decoration(Marker(1))
        .decorations([Marker(2), Marker(3)].into_iter())
        .decorations(layers)
        .decoration(Marker(5))
        .child(SizedBox::new().width(40).height(20).color(Color::BLUE))
        .to_element(&ctx);
    let paints = aimer_widget::testing::record_retained_paint(element.as_ref(), &ctx);
    assert_eq!(paints[0].element, "MultiDecorationBox");
    let commands = &paints[0].commands;
    assert_eq!(commands.len(), 6);
    for (index, red) in [(0, 1), (1, 2), (2, 3), (3, 4), (5, 5)] {
        assert!(matches!(commands[index], DrawCommand::FillRect { color, .. } if color.r == red));
    }
    assert!(matches!(commands[4], DrawCommand::DrawCustom { .. }));
    assert!(paints[1..].iter().flat_map(|paint| paint.commands.iter()).any(|command|
        matches!(command, DrawCommand::FillRect { color, .. } if color.b == 255 && color.r == 0)));
    let mut children = 0;
    element.visit_children(&mut |_| children += 1);
    assert_eq!(children, 1, "one wrapper, without a wrapper per decoration");
}

#[tokio::test]
async fn multi_decoration_empty_stacks_and_box_child_preserve_layout() {
    let inner = InnerCanvas::new();
    for scale in [1.0, 2.0] {
        let ctx = context(&inner, scale);
        let elements = [
            MultiDecorationBox::default()
                .child(SizedBox::new().width(40).height(20))
                .to_element(&ctx),
            MultiDecorationBox::new()
                .decorations(Vec::<Box<dyn Decoration>>::new())
                .box_child(SizedBox::new().width(40).height(20))
                .to_element(&ctx),
        ];
        for element in elements {
            assert_eq!(
                element.computed_size(&ctx),
                ResolvedSize {
                    width: 40.0 * scale,
                    height: 20.0 * scale
                }
            );
            assert!(record(element.as_ref(), &ctx).is_empty());
            assert!(!element.local_v2_paint_needs_recording(&ctx));
        }
        let zero = MultiDecorationBox::new()
            .decoration(LightEdge::new())
            .child(ZeroSizedBox)
            .to_element(&ctx);
        assert_eq!(zero.computed_size(&ctx), ResolvedSize::default());
        assert!(record(zero.as_ref(), &ctx).is_empty());
    }
}

type Observation = (Vec2d, ResolvedSize, f32);
struct OutsetMarker {
    outsets: [f32; 4],
    observed: Rc<RefCell<Vec<Observation>>>,
}
impl Decoration for OutsetMarker {
    fn paint(&self, _canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.observed.borrow_mut().push((origin, size, scale));
    }
    fn paint_outsets(&self, _size: ResolvedSize, _scale: f32) -> [f32; 4] {
        self.outsets
    }
    fn content_insets(&self, _size: ResolvedSize, _scale: f32) -> [f32; 4] {
        [10.0; 4]
    }
    fn child_clip_radius(&self, _size: ResolvedSize, _scale: f32) -> Option<[f32; 4]> {
        Some([8.0; 4])
    }
}

#[tokio::test]
async fn multi_decoration_unions_sanitized_outsets_and_uses_one_logical_origin() {
    let inner = InnerCanvas::new();
    for scale in [1.0, 2.0] {
        for runtime in [false, true] {
            let ctx = context(&inner, scale);
            let observed = Rc::new(RefCell::new(Vec::new()));
            let a = OutsetMarker {
                outsets: [2.0, f32::NAN, f32::INFINITY, 4.0],
                observed: observed.clone(),
            };
            let b = OutsetMarker {
                outsets: [-1.0, 3.0, 5.0, 1.0],
                observed: observed.clone(),
            };
            let element = if runtime {
                MultiDecorationBox::new()
                    .decorations([a, b])
                    .child(SizedBox::new().width(40).height(20))
                    .to_element(&ctx)
            } else {
                MultiDecorationBox::new()
                    .decoration(a)
                    .decoration(b)
                    .child(SizedBox::new().width(40).height(20))
                    .to_element(&ctx)
            };
            assert_eq!(
                element.retained_v2_paint_outsets(&ctx),
                Some([2.0, 3.0, 5.0, 4.0])
            );
            assert_eq!(
                element.computed_size(&ctx),
                ResolvedSize {
                    width: 40.0 * scale,
                    height: 20.0 * scale
                }
            );
            assert_eq!(element.retained_clip(&ctx), None);
            record(element.as_ref(), &ctx);
            assert_eq!(
                *observed.borrow(),
                vec![
                    (
                        Vec2d { x: 2.0, y: 3.0 },
                        ResolvedSize {
                            width: 40.0,
                            height: 20.0
                        },
                        scale
                    );
                    2
                ]
            );
            element.visit_children(&mut |child| {
                let child_ctx = element.retained_v2_child_context(&ctx, child).unwrap();
                assert_eq!(child_ctx.box_constraint, ctx.box_constraint);
                assert_eq!(child.computed_size(&child_ctx), element.computed_size(&ctx));
                assert_eq!(element.retained_v2_child_clip_radius(&ctx, child), [0.0; 4]);
            });
        }
    }
}

struct MutableMarker(Rc<Cell<u64>>);
impl Decoration for MutableMarker {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        canvas.fill_color_rect(origin, size, Color::Rgb(self.0.get() as u8, 0, 0));
    }
    fn revision(&self) -> u64 {
        self.0.get()
    }
}

#[tokio::test]
async fn multi_decoration_tracks_each_revision_without_sum_or_xor_collisions() {
    let inner = InnerCanvas::new();
    for runtime in [false, true] {
        let ctx = context(&inner, 1.0);
        let a = Rc::new(Cell::new(1));
        let b = Rc::new(Cell::new(2));
        let element = if runtime {
            MultiDecorationBox::new()
                .decorations([MutableMarker(a.clone()), MutableMarker(b.clone())])
                .child(SizedBox::new().width(40).height(20))
                .to_element(&ctx)
        } else {
            MultiDecorationBox::new()
                .decoration(MutableMarker(a.clone()))
                .decoration(MutableMarker(b.clone()))
                .child(SizedBox::new().width(40).height(20))
                .to_element(&ctx)
        };
        for (first, second) in [(1, 2), (2, 1), (3, 1), (3, 4), (1, 2)] {
            a.set(first);
            b.set(second);
            assert!(element.local_v2_paint_needs_recording(&ctx));
            let commands = record(element.as_ref(), &ctx);
            assert!(
                matches!(commands.as_slice(), [DrawCommand::FillRect { color: a, .. }, DrawCommand::FillRect { color: b, .. }]
                if a.r == first as u8 && b.r == second as u8)
            );
            assert!(!element.local_v2_paint_needs_recording(&ctx));
        }
    }
}

#[tokio::test]
async fn multi_decoration_preserves_child_events_and_focus() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner, 1.0);
    let focus = FocusNode::new();
    let element = MultiDecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::RED))
        .decoration(LightEdge::new())
        .child(Focusable::new().node(focus.clone()).child(ZeroSizedBox))
        .to_element(&ctx);
    assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);
    assert!(element.focus_node().is_none());
    let mut events = Vec::new();
    element.event_children(&mut |child| events.push(child.id()));
    let mut structural = Vec::new();
    element.structural_children(&mut |child| structural.push(child.id()));
    assert_eq!(events, structural);
    assert_eq!(events.len(), 1);
    focus.request_focus();
    let result = EventDispatcher::new().dispatch(
        element.as_ref(),
        Vec2d::default(),
        &aimer_events::element::ElementEvent::Cancel,
    );
    assert!(!result.is_consumed());
    assert!(focus.has_focus());
}

#[cfg(feature = "portable-guest")]
#[test]
fn multi_decoration_reports_unsupported_portable_lowering() {
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
    let source = SourceFingerprint::new(StableId128::from_path("test", "multi-decoration-box"));
    let result = MultiDecorationBox::new()
        .child(ZeroSizedBox)
        .to_portable_node(&mut ctx, source);
    assert!(matches!(result, Err(PortableBuildError::UnsupportedWidget {
        widget: "MultiDecorationBox", source: actual,
    }) if actual == source));
}
