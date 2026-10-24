use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
use aimer_canvas::{Canvas, FrameCanvas, InnerCanvas};
use aimer_color::prelude::Color;
use aimer_container::{Container, SizedBox};
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use aimer_decoration::{BorderRadius, Decoration, DecorationBox, MultiDecorationBox};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{Drawable, Widget};

#[test]
fn color_decoration_paints_its_background_without_fallback_in_logical_coordinates() {
    let size = ResolvedSize { width: 80.0, height: 40.0 };
    for scale in [1.0, 2.0] {
        for color in [Color::BLUE, Color::Rgba(10, 20, 30, 128), Color::Transparent] {
            assert_eq!(color.background_color(), Some(color));
            assert_eq!(color.content_insets(size, scale), [0.0; 4]);
            assert_eq!(color.paint_outsets(size, scale), [0.0; 4]);
            assert!(color.child_clip_radius(size, scale).is_none());
            let tree = RenderTree::new();
            let node = tree.add_root(Rect::new(0.0, 0.0, 200.0, 160.0)).unwrap();
            let canvas = Canvas::of(&tree.context(node).unwrap());
            color.paint_with_background(&canvas, (3.0, 5.0).into(), size, scale, Some(Color::RED));
            canvas.finish();
            let list = tree.draw_list_snapshot(node).unwrap();
            assert!(matches!(list.commands.as_ref(), [DrawCommand::FillRect {
                rect, color: painted, border_width, outline_width, border_radius, ..
            }] if *rect == Rect::new(3.0, 5.0, 80.0, 40.0) && *painted == color.into()
                && *border_width == [0.0; 4] && *outline_width == [0.0; 4]
                && *border_radius == [0.0; 4]));
        }
    }
    assert_ne!(Color::RED.revision(), Color::BLUE.revision());
    let erased = Color::BLUE.notype();
    assert!(erased.is_inline());
    assert_eq!(erased.background_color(), Some(Color::BLUE));
}

#[test]
fn color_decoration_ignores_invalid_scale_and_accepts_zero_size() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 80.0, 40.0)).unwrap();
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let canvas = Canvas::of(&tree.context(node).unwrap());
        Color::BLUE.paint(&canvas, Vec2d::default(), ResolvedSize::default(), scale);
        canvas.finish();
        assert!(tree.draw_list_snapshot(node).unwrap().commands.is_empty());
    }
    let canvas = Canvas::of(&tree.context(node).unwrap());
    Color::BLUE.paint(&canvas, Vec2d::default(), ResolvedSize::default(), 1.0);
    canvas.finish();
    assert!(matches!(tree.draw_list_snapshot(node).unwrap().commands.as_ref(),
        [DrawCommand::FillRect { rect, .. }] if *rect == Rect::new(0.0, 0.0, 0.0, 0.0)));
}

/// Paints `Color::BLUE` onto a canvas whose earlier layers declared `radii`, and
/// returns the fill it recorded.
fn painted_after_declaring(radii: Option<[f32; 4]>, size: ResolvedSize, scale: f32) -> DrawCommand {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 200.0, 160.0)).unwrap();
    let canvas = Canvas::of(&tree.context(node).unwrap());
    if let Some(radii) = radii {
        canvas.set_corner_radii(radii);
    }
    Color::BLUE.paint(&canvas, (3.0, 5.0).into(), size, scale);
    canvas.finish();
    let list = tree.draw_list_snapshot(node).unwrap();
    assert_eq!(list.commands.len(), 1);
    list.commands[0].clone()
}

#[test]
fn color_decoration_follows_the_corner_radii_an_earlier_layer_declared() {
    let size = ResolvedSize { width: 80.0, height: 40.0 };
    for scale in [1.0, 2.0] {
        // Radii are declared in logical pixels, the same space the rect is in.
        let fill = painted_after_declaring(Some([8.0, 4.0, 2.0, 0.0]), size, scale);
        assert!(matches!(fill, DrawCommand::FillRect { rect, border_radius, border_width, outline_width, .. }
            if rect == Rect::new(3.0, 5.0, 80.0, 40.0) && border_radius == [8.0, 4.0, 2.0, 0.0]
                && border_width == [0.0; 4] && outline_width == [0.0; 4]), "scale {scale}: {fill:?}");
    }
}

#[test]
fn a_declared_radius_never_exceeds_half_the_box() {
    let size = ResolvedSize { width: 80.0, height: 40.0 };
    let fill = painted_after_declaring(Some([100.0; 4]), size, 1.0);
    assert!(matches!(fill, DrawCommand::FillRect { border_radius, .. } if border_radius == [20.0; 4]), "{fill:?}");
}

#[test]
fn invalid_declared_radii_leave_square_corners() {
    let size = ResolvedSize { width: 80.0, height: 40.0 };
    let fill = painted_after_declaring(Some([-4.0, f32::NAN, f32::INFINITY, 6.0]), size, 1.0);
    assert!(matches!(fill, DrawCommand::FillRect { border_radius, .. } if border_radius == [0.0, 0.0, 0.0, 6.0]), "{fill:?}");
}

#[test]
fn with_no_declared_shape_the_fill_stays_square() {
    let fill = painted_after_declaring(None, ResolvedSize { width: 80.0, height: 40.0 }, 1.0);
    assert!(matches!(fill, DrawCommand::FillRect { border_radius, .. } if border_radius == [0.0; 4]), "{fill:?}");
}

/// The shape of an avatar: a radius, then the fill it applies to.
#[tokio::test]
async fn a_color_after_a_border_radius_in_a_list_is_rounded() {
    let inner = InnerCanvas::new();
    let mut ctx = BuildContext::new(
        FrameCanvas::new(&inner), ResolvedSize { width: 200.0, height: 160.0 }, 2.0,
        Vec2d::default(), Vec2d::default(), WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0, min_height: 0.0, max_width: 200.0, max_height: 160.0,
    };
    let rounded = MultiDecorationBox::new().decoration(BorderRadius::all(12.0)).decoration(Color::BLUE)
        .child(SizedBox::new().width(80).height(40)).to_element(&ctx);
    let paints = aimer_widget::testing::record_retained_paint(rounded.as_ref(), &ctx);
    let fill = paints.iter().flat_map(|paint| paint.commands.iter())
        .find(|command| matches!(command, DrawCommand::FillRect { color, .. } if color.b == 255 && color.r == 0))
        .expect("the blue fill is recorded");
    assert!(matches!(fill, DrawCommand::FillRect { border_radius, .. } if *border_radius == [12.0; 4]), "{fill:?}");
}

struct BackgroundProbe;

impl Decoration for BackgroundProbe {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        assert_eq!(canvas.background_color(), Color::BLUE);
        canvas.fill_color_rect(origin, size, Color::RED);
    }
}

#[tokio::test]
async fn color_decoration_works_in_containers_and_propagates_to_later_layers() {
    let inner = InnerCanvas::new();
    let mut ctx = BuildContext::new(
        FrameCanvas::new(&inner), ResolvedSize { width: 200.0, height: 160.0 }, 2.0,
        Vec2d::default(), Vec2d::default(), WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0, min_height: 0.0, max_width: 200.0, max_height: 160.0,
    };
    let container = Container::new().width(80).height(40).color(Color::RED)
        .decoration(Color::BLUE).child(SizedBox::new()).to_element(&ctx);
    assert_eq!(container.content_size(&ctx), ResolvedSize { width: 160.0, height: 80.0 });
    let decorated = DecorationBox::new().decoration(Color::BLUE)
        .child(SizedBox::new().width(80).height(40)).to_element(&ctx);
    let layered = MultiDecorationBox::new().decoration(Color::BLUE).decoration(BackgroundProbe)
        .child(SizedBox::new().width(80).height(40)).to_element(&ctx);
    for element in [container, decorated, layered] {
        let paints = aimer_widget::testing::record_retained_paint(element.as_ref(), &ctx);
        let background = paints.iter().flat_map(|paint| paint.commands.iter()).next().unwrap();
        assert!(matches!(background, DrawCommand::FillRect { color, .. } if color.b == 255 && color.r == 0));
        assert!(!element.local_v2_paint_needs_recording(&ctx));
    }
}
