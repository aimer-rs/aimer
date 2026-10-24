//! Glass and Liquid take their corner shape from `BorderRadius`, or from the
//! decorations behind them, and never from a bare number.

use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_color::prelude::Color;
use aimer_container::{Glass, Liquid, SizedBox};
use aimer_cupid::draw_cmd_v2::DrawCommand;
use aimer_cupid::material::MaterialRequest;
use aimer_decoration::{BorderRadius, BoxDecoration, BoxShadow, Decoration};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{AnyElement, Widget};

fn context(inner: &InnerCanvas) -> BuildContext<'_> {
    let mut ctx = BuildContext::new(
        FrameCanvas::new(inner),
        ResolvedSize { width: 400.0, height: 300.0 },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 400.0,
        max_height: 300.0,
    };
    ctx
}

/// Radii of the fallback fill (fitted to the box) and of the material packet.
struct Shape {
    fill: [f32; 4],
    material: [f32; 4],
}

fn shape(element: &AnyElement, ctx: &BuildContext) -> Shape {
    let paints = aimer_widget::testing::record_retained_paint(element.as_ref(), ctx);
    let commands = &paints[0].commands;
    // The glass fill is the last rectangle recorded before the material packet.
    let fill = commands
        .iter()
        .rev()
        .find_map(|command| match command {
            DrawCommand::FillRect { border_radius, .. } => Some(*border_radius),
            _ => None,
        })
        .expect("a fallback fill");
    let material = commands
        .iter()
        .find_map(|command| match command {
            DrawCommand::DrawCustom { data, .. } => Some(MaterialRequest::decode(data).unwrap()),
            _ => None,
        })
        .expect("a material packet")
        .corner_radii;
    Shape { fill, material }
}

fn glass() -> Glass {
    Glass::new().shadow_blur(0.0)
}

fn child() -> SizedBox {
    SizedBox::new().width(80).height(40)
}

#[tokio::test]
async fn glass_and_liquid_keep_their_default_shapes() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let plain = glass().child(child()).to_element(&ctx);
    let Shape { fill, material } = shape(&plain, &ctx);
    assert_eq!((fill, material), ([16.0; 4], [16.0; 4]));
    let liquid = Liquid::new()
        .child(SizedBox::new().width(160).height(120))
        .to_element(&ctx);
    let Shape { fill, material } = shape(&liquid, &ctx);
    assert_eq!((fill, material), ([28.0; 4], [28.0; 4]));
}

#[tokio::test]
async fn border_radius_sets_per_corner_radii() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let uneven = BorderRadius::new().top_left(4.0).top_right(8.0).bottom_right(12.0).bottom_left(16.0);
    let element = glass().border_radius(uneven).child(child()).to_element(&ctx);
    let Shape { fill, material } = shape(&element, &ctx);
    assert_eq!((fill, material), ([4.0, 8.0, 12.0, 16.0], [4.0, 8.0, 12.0, 16.0]));
    let element = Liquid::new()
        .border_radius(7.0)
        .child(SizedBox::new().width(160).height(120))
        .to_element(&ctx);
    let Shape { fill, material } = shape(&element, &ctx);
    assert_eq!((fill, material), ([7.0; 4], [7.0; 4]));
}

#[tokio::test]
async fn border_radius_bounds_the_packet_and_fits_the_fallback() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let element = glass().border_radius(5000.0).child(child()).to_element(&ctx);
    let Shape { fill, material } = shape(&element, &ctx);
    assert_eq!(fill, [20.0; 4], "the fill fits half the shorter side");
    assert_eq!(material, [1024.0; 4]);
    for invalid in [-3.0, f32::NAN, f32::INFINITY] {
        let element = glass().border_radius(invalid).child(child()).to_element(&ctx);
        let Shape { fill, material } = shape(&element, &ctx);
        assert_eq!((fill, material), ([0.0; 4], [0.0; 4]), "{invalid}");
    }
}

#[tokio::test]
async fn a_decoration_declares_the_shape_when_no_radius_is_set() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let from_radius = glass().decoration(BorderRadius::from(10.0)).child(child()).to_element(&ctx);
    let from_box = glass()
        .decoration(BoxDecoration::new().border_radius(6.0))
        .child(child())
        .to_element(&ctx);
    let from_stack = glass()
        .decoration(vec![Color::RED.notype(), BorderRadius::from(9.0).notype()])
        .child(child())
        .to_element(&ctx);
    for (element, radius) in [(from_radius, 10.0), (from_box, 6.0), (from_stack, 9.0)] {
        let Shape { fill, material } = shape(&element, &ctx);
        assert_eq!((fill, material), ([radius; 4], [radius; 4]));
    }
    let liquid = Liquid::new()
        .decoration(BorderRadius::from(11.0))
        .child(SizedBox::new().width(160).height(120))
        .to_element(&ctx);
    let Shape { fill, material } = shape(&liquid, &ctx);
    assert_eq!((fill, material), ([11.0; 4], [11.0; 4]));
}

#[tokio::test]
async fn an_explicit_radius_wins_and_a_declared_zero_beats_the_default() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let explicit = glass()
        .border_radius(3.0)
        .decoration(BorderRadius::from(10.0))
        .child(child())
        .to_element(&ctx);
    assert_eq!(shape(&explicit, &ctx).material, [3.0; 4]);
    let square = glass().decoration(BorderRadius::from(0)).child(child()).to_element(&ctx);
    assert_eq!(shape(&square, &ctx).material, [0.0; 4], "square is a declaration, not an absence");
    // A decoration that declares no shape leaves the default alone.
    let unshaped = glass().decoration(Color::RED).child(child()).to_element(&ctx);
    assert_eq!(shape(&unshaped, &ctx).material, [16.0; 4]);
}

#[tokio::test]
async fn the_decoration_paints_behind_the_glass_and_widens_the_paint_bounds() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let element = glass().decoration(Color::Rgb(250, 0, 0)).child(child()).to_element(&ctx);
    let paints = aimer_widget::testing::record_retained_paint(element.as_ref(), &ctx);
    let commands = &paints[0].commands;
    assert!(matches!(commands[0], DrawCommand::FillRect { color, .. } if color.r == 250 && color.g == 0));
    assert!(matches!(commands.last(), Some(DrawCommand::DrawCustom { .. })));

    let bare = glass().child(child()).to_element(&ctx);
    assert_eq!(bare.retained_v2_paint_outsets(&ctx), None, "no shadow, no outsets");
    let shadowed = glass()
        .decoration(BoxShadow::new().blur(8.0))
        .child(child())
        .to_element(&ctx);
    let outsets = shadowed.retained_v2_paint_outsets(&ctx).expect("shadow outsets");
    assert!(outsets.iter().any(|outset| *outset > 0.0), "{outsets:?}");
    assert!(outsets.iter().all(|outset| outset.is_finite() && *outset >= 0.0));
}

#[tokio::test]
async fn glass_with_a_decoration_still_lays_out_as_its_child() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner);
    let element = glass().decoration(BorderRadius::from(10.0)).child(child()).to_element(&ctx);
    assert_eq!(element.computed_size(&ctx), ResolvedSize { width: 80.0, height: 40.0 });
}
