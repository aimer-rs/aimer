use super::test_support::{stacked, stacked_in};
use crate::{BorderRadius, BoxBorder, BoxDecoration, Decoration, LightEdge};
use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_color::prelude::Color;
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use aimer_cupid::material::{MaterialKind, MaterialRequest};

fn recorded(edge: LightEdge, size: ResolvedSize) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
    let canvas = Canvas::of(&tree.context(root).unwrap());
    edge.paint(&canvas, Vec2d { x: 3.0, y: 4.0 }, size, 2.0);
    canvas.finish();
    tree.draw_list_snapshot(root).unwrap().commands.to_vec()
}

fn request(edge: LightEdge) -> MaterialRequest {
    let commands = recorded(
        edge,
        ResolvedSize {
            width: 80.0,
            height: 40.0,
        },
    );
    let [DrawCommand::DrawCustom { data, .. }] = commands.as_slice() else {
        panic!("one light-edge packet")
    };
    MaterialRequest::decode(data).unwrap()
}

#[test]
fn light_edge_defaults_and_controls_survive_the_canvas_bridge() {
    let edge = LightEdge::new();
    assert_eq!(edge, LightEdge::default());
    let defaults = request(edge);
    assert_eq!(defaults.kind, MaterialKind::LightEdge);
    assert_eq!(defaults.bounds, [3.0, 4.0, 80.0, 40.0]);
    assert_eq!(defaults.border_color, [1.0; 4]);
    assert_eq!(defaults.border_width, 8.0);
    assert_eq!(defaults.specular_highlight, 0.8);
    assert_eq!(defaults.edge_lighting, 0.35);
    assert_eq!(defaults.corner_radii, [0.0; 4]);
    assert_eq!(defaults.light_angle, 225.0);
    assert_eq!(defaults.blur_radius, 0.0);
    assert_eq!(defaults.tint, [0.0; 4]);
    assert_eq!(defaults.shadow_color, [0.0; 4]);
    assert_eq!(defaults.distortion_strength, 0.0);
    assert_eq!(defaults.animation_speed, 0.0);
    let configured = request(
        edge.color(Color::Rgba(255, 0, 0, 128))
            .width(3.0)
            .strength(0.5)
            .light_angle(90.0),
    );
    assert_eq!(configured.border_color, [1.0, 0.0, 0.0, 128.0 / 255.0]);
    assert_eq!(configured.border_width, 3.0);
    assert_eq!(configured.specular_highlight, 0.5);
    assert_eq!(configured.corner_radii, [0.0; 4]);
    assert_eq!(configured.light_angle, 90.0);
}

#[test]
fn light_edge_reflection_strength_is_independent_and_normalized() {
    for (value, expected) in [
        (0.0, 0.0),
        (0.65, 0.65),
        (1.0, 1.0),
        (-1.0, 0.0),
        (2.0, 1.0),
        (f32::NAN, 0.35),
        (f32::INFINITY, 0.35),
    ] {
        let edge = LightEdge::new().strength(0.8).reflection_strength(value);
        assert!(
            !edge.is_empty(),
            "zero reflection keeps the primary highlight"
        );
        let recorded = request(edge);
        assert_eq!(recorded.edge_lighting, expected);
        assert_eq!(recorded.specular_highlight, 0.8);
    }
}

#[test]
fn light_edge_bounds_inputs_and_skips_invisible_paint() {
    let fallback = request(
        LightEdge::new()
            .width(f32::INFINITY)
            .strength(f32::NAN)
            .light_angle(f32::NAN),
    );
    assert_eq!(fallback.border_width, 8.0);
    assert_eq!(fallback.specular_highlight, 0.8);
    assert_eq!(fallback.corner_radii, [0.0; 4]);
    assert_eq!(fallback.light_angle, 225.0);
    let bounded = request(LightEdge::new().width(100.0).strength(10.0));
    assert_eq!(bounded.border_width, 16.0);
    assert_eq!(bounded.specular_highlight, 1.0);
    for (angle, expected) in [(-135.0, 225.0), (720.0, 0.0), (450.0, 90.0)] {
        assert_eq!(
            request(LightEdge::new().light_angle(angle)).light_angle,
            expected
        );
    }
    let size = ResolvedSize {
        width: 80.0,
        height: 40.0,
    };
    for edge in [
        LightEdge::new().width(0.0),
        LightEdge::new().width(-1.0),
        LightEdge::new().strength(0.0),
        LightEdge::new().strength(-1.0),
        LightEdge::new().color(Color::Transparent),
        LightEdge::new().color(Color::Rgba(255, 0, 0, 0)),
    ] {
        assert!(edge.is_empty());
        assert!(recorded(edge, size).is_empty());
    }
    for size in [
        ResolvedSize {
            width: 0.0,
            height: 40.0,
        },
        ResolvedSize {
            width: 80.0,
            height: 0.0,
        },
    ] {
        assert!(recorded(LightEdge::new(), size).is_empty());
    }
    let tiny = stacked_in(
        ResolvedSize { width: 0.5, height: 1.0 },
        &[&BorderRadius::from(1024.0), &LightEdge::new().width(16.0)],
    );
    assert_eq!(tiny.border_width, 0.25);
    assert_eq!(tiny.corner_radii, [0.25; 4]);
}

#[tokio::test]
async fn light_edge_preserves_child_layout_and_clipping() {
    use crate::DecorationBox;
    use aimer_canvas::{FrameCanvas, InnerCanvas};
    use aimer_container::SizedBox;
    use aimer_widget::{
        Drawable, Widget,
        base::{BuildContext, WindowHandle},
    };
    let inner = InnerCanvas::new();
    let ctx = BuildContext::new(
        FrameCanvas::new(&inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        2.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    let edge = LightEdge::new().width(16.0);
    assert_eq!(edge.content_insets(ResolvedSize::default(), 2.0), [0.0; 4]);
    assert_eq!(edge.child_clip_radius(ResolvedSize::default(), 2.0), None);
    assert_eq!(edge.background_color(), None);
    let element = DecorationBox::new()
        .decoration(edge)
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    assert_eq!(
        element.computed_size(&ctx),
        ResolvedSize {
            width: 48.0,
            height: 24.0
        }
    );
    assert_eq!(element.retained_clip(&ctx), None);
    assert_eq!(element.retained_v2_paint_outsets(&ctx), Some([0.0; 4]));
}

#[test]
fn light_edge_without_a_shape_layer_has_square_corners() {
    assert_eq!(stacked(&[&LightEdge::new()]).corner_radii, [0.0; 4]);
    // A shape declared after the highlight cannot reach back and reshape it.
    let late = stacked(&[&LightEdge::new(), &BorderRadius::from(12.0)]);
    assert_eq!(late.corner_radii, [0.0; 4]);
    // Layers that declare no shape leave the inherited one alone, including
    // border-only and shadow-only decorations that delegate to a default box.
    let edge = stacked(&[
        &BorderRadius::from(12.0),
        &Color::BLUE,
        &BoxDecoration::new(),
        &BoxBorder::default(),
        &LightEdge::new(),
    ]);
    assert_eq!(edge.corner_radii, [12.0; 4]);
}

#[test]
fn light_edge_takes_its_corners_from_preceding_shape_layers() {
    let lens = stacked(&[&BorderRadius::from(12.0), &LightEdge::new()]);
    assert_eq!(lens.corner_radii, [12.0; 4]);
    let uneven = BorderRadius::new()
        .top_left(4.0)
        .top_right(8.0)
        .bottom_right(12.0)
        .bottom_left(16.0);
    assert_eq!(stacked(&[&uneven, &LightEdge::new()]).corner_radii, [4.0, 8.0, 12.0, 16.0]);
    let shape = BoxDecoration::new().border_radius(10.0);
    assert_eq!(stacked(&[&shape, &LightEdge::new()]).corner_radii, [10.0; 4]);
}

#[test]
fn light_edge_fits_and_sanitizes_the_declared_corners() {
    // The 80x40 box has a 20px half-height; larger radii fit within it.
    let fit = stacked(&[&BorderRadius::from(2000.0), &LightEdge::new()]);
    assert_eq!(fit.corner_radii, [20.0; 4], "radii fit the shorter side");
    for invalid in [-4.0, f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
        let edge = stacked(&[&BorderRadius::from(invalid), &LightEdge::new()]);
        assert_eq!(edge.corner_radii, [0.0; 4], "{invalid}");
    }
}
