use super::test_support::stacked;
use crate::{BorderRadius, BoxDecoration, Decoration, Magnification, MagnificationStyle as S};
use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use aimer_cupid::material::{MaterialKind, MaterialRequest};

fn recorded(lens: Magnification, size: ResolvedSize) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
    let canvas = Canvas::of(&tree.context(root).unwrap());
    lens.paint(&canvas, Vec2d { x: 3.0, y: 4.0 }, size, 2.0);
    canvas.finish();
    tree.draw_list_snapshot(root).unwrap().commands.to_vec()
}

fn request(lens: Magnification) -> MaterialRequest {
    let commands = recorded(
        lens,
        ResolvedSize {
            width: 80.0,
            height: 40.0,
        },
    );
    let [DrawCommand::DrawCustom { data, .. }] = commands.as_slice() else {
        panic!("one lens packet")
    };
    assert_eq!(data.len(), 256);
    MaterialRequest::decode(data).unwrap()
}

#[test]
fn magnification_defaults_and_packet_preserve_zoom_and_depth() {
    let lens = Magnification::new();
    assert_eq!(lens, Magnification::default());
    let default = request(lens);
    assert_eq!(default.kind, MaterialKind::Magnification);
    assert_eq!(default.bounds, [3.0, 4.0, 80.0, 40.0]);
    assert_eq!(default.magnification, 1.4);
    assert_eq!(default.bevel_radius, 16.0);
    assert_eq!(default.corner_radii, [0.0; 4]);
    assert_eq!(default.magnification_style, S::DEFAULT);
    assert_eq!(default.bulge_strength, 0.25);
    let configured = request(lens.zoom(2.0).thickness(24.0));
    assert_eq!(configured.magnification, 2.0);
    assert_eq!(configured.bevel_radius, 24.0);
    assert_eq!(configured.corner_radii, [0.0; 4]);
    assert_eq!(configured.blur_radius, 0.0);
    assert_eq!(configured.tint, [0.0; 4]);
    assert_eq!(configured.border_width, 0.0);
    assert_eq!(configured.shadow_color, [0.0; 4]);
    assert_eq!(configured.chromatic_aberration, 0.0);
    assert_eq!(lens.paint_outsets(ResolvedSize::default(), 2.0), [0.0; 4]);
    assert_eq!(lens.revision(), 0);
}

#[test]
fn magnification_normalizes_inputs_and_skips_identity_and_empty_bounds() {
    let fallback = request(
        Magnification::new()
            .zoom(f32::NAN)
            .thickness(f32::INFINITY),
    );
    assert_eq!(fallback.magnification, 1.5);
    assert_eq!(fallback.bevel_radius, 16.0);
    let bounded = request(Magnification::new().zoom(99.0).thickness(999.0));
    assert_eq!(bounded.magnification, 8.0);
    assert_eq!(bounded.bevel_radius, 512.0);
    let size = ResolvedSize {
        width: 80.0,
        height: 40.0,
    };
    assert!(recorded(Magnification::new().zoom(-1.0).thickness(-1.0), size).is_empty());
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
        assert!(recorded(Magnification::new(), size).is_empty());
    }
}

#[test]
fn magnification_styles_preserve_flags_and_skip_inactive_effects() {
    let style = S::UNIFORM | S::BULGE | S::REFRACTION | S::HORIZONTAL | S::LEFT;
    let configured = request(Magnification::new().style(style).bulge(0.75));
    assert_eq!(configured.magnification_style, style);
    assert_eq!(configured.bulge_strength, 0.75);
    for (value, expected) in [
        (f32::NAN, 0.25),
        (f32::INFINITY, 0.25),
        (-1.0, 0.0),
        (2.0, 1.0),
    ] {
        assert_eq!(
            request(Magnification::new().bulge(value)).bulge_strength,
            expected
        );
    }
    let size = ResolvedSize {
        width: 80.0,
        height: 40.0,
    };
    for lens in [
        Magnification::new().style(S::NONE),
        Magnification::new().style(S::HORIZONTAL | S::LEFT),
        Magnification::new().style(S::UNIFORM).zoom(1.0),
        Magnification::new().style(S::BULGE).bulge(0.0),
        Magnification::new().style(S::REFRACTION).thickness(0.0),
    ] {
        assert!(recorded(lens, size).is_empty());
    }
    assert!(
        !recorded(
            Magnification::new()
                .style(S::BULGE)
                .zoom(1.0)
                .thickness(0.0),
            size
        )
        .is_empty()
    );
}

#[tokio::test]
async fn magnification_preserves_child_layout_and_does_not_clip_it() {
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
    let element = DecorationBox::new()
        .decoration(Magnification::new().zoom(8.0).thickness(512.0))
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
    element.visit_children(&mut |child| {
        assert_eq!(child.computed_size(&ctx), element.computed_size(&ctx));
        assert_eq!(element.retained_v2_child_clip_radius(&ctx, child), [0.0; 4]);
    });
}

#[test]
fn magnification_without_a_shape_layer_has_square_corners() {
    assert_eq!(stacked(&[&Magnification::new()]).corner_radii, [0.0; 4]);
    // A shape declared after the lens cannot reach back and reshape it.
    let late = stacked(&[&Magnification::new(), &BorderRadius::from(12.0)]);
    assert_eq!(late.corner_radii, [0.0; 4]);
}

#[test]
fn magnification_takes_its_corners_from_the_preceding_border_radius() {
    for scale_independent in [BorderRadius::from(12.0), BorderRadius::from(12)] {
        let lens = stacked(&[&scale_independent, &Magnification::new()]);
        assert_eq!(lens.corner_radii, [12.0; 4]);
    }
    let uneven = BorderRadius::new()
        .top_left(4.0)
        .top_right(8.0)
        .bottom_right(12.0)
        .bottom_left(16.0);
    let lens = stacked(&[&uneven, &Magnification::new()]);
    assert_eq!(lens.corner_radii, [4.0, 8.0, 12.0, 16.0]);
    let lens = stacked(&[&BorderRadius::from(4.0), &BorderRadius::from(20.0), &Magnification::new()]);
    assert_eq!(lens.corner_radii, [20.0; 4], "the nearest preceding shape wins");
}

#[test]
fn magnification_takes_its_corners_from_a_preceding_box_decoration() {
    let shape = BoxDecoration::new().border_radius(10.0);
    assert_eq!(stacked(&[&shape, &Magnification::new()]).corner_radii, [10.0; 4]);
}

#[test]
fn magnification_bounds_and_sanitizes_the_declared_corners() {
    let huge = stacked(&[&BorderRadius::from(9999.0), &Magnification::new()]);
    assert_eq!(huge.corner_radii, [1024.0; 4]);
    for invalid in [-4.0, f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
        let lens = stacked(&[&BorderRadius::from(invalid), &Magnification::new()]);
        assert_eq!(lens.corner_radii, [0.0; 4], "{invalid}");
    }
}
