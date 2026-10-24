//! The `Icon` widget: its size, its color, and its builder contract.

use aimer_attribute::BoxConstraint;
use aimer_attribute::size::ResolvedSize;
use aimer_cupid::draw_cmd_v2::DrawCommand;
use aimer_cupid::svg::SvgColor;
use aimer_lucide::{Icon, Icons};
use aimer_svg::{SvgDocument, SvgStyle};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::testing::{all_commands, record_retained_paint};
use aimer_widget::{AnyElement, Drawable, LayoutElement, PortableWidget, Widget};

fn context() -> BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    let mut ctx = BuildContext::new(
        canvas,
        ResolvedSize {
            width: 400.0,
            height: 400.0,
        },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 400.0,
        max_height: 400.0,
    };
    ctx
}

fn size_of(icon: impl Widget + 'static, ctx: &BuildContext) -> (f32, f32) {
    let size = icon.to_element(ctx).layout(ctx);
    (size.width, size.height)
}

/// The per-node overrides of the single SVG an icon paints.
fn overrides(element: &AnyElement, ctx: &BuildContext) -> Vec<(Option<Option<SvgColor>>, Option<Option<SvgColor>>, Option<f32>)> {
    let _ = element.layout(ctx);
    element.update(ctx);
    let recorded = record_retained_paint(element.as_ref(), ctx);
    let commands = all_commands(&recorded);
    let mut svgs = commands.iter().filter_map(|command| match command {
        DrawCommand::Svg { overrides, .. } => Some(overrides.clone()),
        _ => None,
    });
    let overrides = svgs.next().expect("an icon paints one SVG");
    assert!(svgs.next().is_none(), "an icon paints exactly one SVG");
    overrides
        .iter()
        .map(|node| (node.fill, node.stroke, node.opacity))
        .collect()
}

const BLUE: SvgColor = SvgColor::rgba8(0, 128, 255, 255);

#[tokio::test]
async fn an_icon_is_a_named_widget_with_the_default_portable_contract() {
    fn assert_portable<T: PortableWidget>() {}
    assert_portable::<Icon<SvgDocument>>();
    assert_eq!(Widget::debug_name(&Icon::of(Icons::HOUSE)), "Icon");
}

#[tokio::test]
async fn an_icon_without_a_size_uses_its_intrinsic_viewport() {
    let ctx = context();
    assert_eq!(size_of(Icon::of(Icons::HOUSE), &ctx), (24.0, 24.0));
}

#[tokio::test]
async fn size_makes_a_square_and_one_dimension_derives_the_other() {
    let ctx = context();
    assert_eq!(size_of(Icon::of(Icons::HOUSE).size(48), &ctx), (48.0, 48.0));
    assert_eq!(size_of(Icon::of(Icons::HOUSE).width(32), &ctx), (32.0, 32.0));
    assert_eq!(size_of(Icon::of(Icons::HOUSE).height(16), &ctx), (16.0, 16.0));
    assert_eq!(
        size_of(Icon::of(Icons::HOUSE).width(30).height(10), &ctx),
        (30.0, 10.0),
        "both dimensions set are used as given"
    );
}

#[tokio::test]
async fn setting_the_icon_after_the_size_keeps_the_size() {
    let ctx = context();
    let icon = Icon::new().size(40).color(BLUE).data(Icons::SEARCH);
    assert_eq!(size_of(icon, &ctx), (40.0, 40.0));
}

#[tokio::test]
async fn data_replaces_the_icon_and_keeps_the_styling() {
    let ctx = context();
    let icon = Icon::of(Icons::HOUSE).color(BLUE).data(Icons::CIRCLE_CHECK);
    let styled = overrides(&icon.to_element(&ctx), &ctx);
    assert!(!styled.is_empty());
    assert!(styled.iter().all(|(_, stroke, _)| *stroke == Some(Some(BLUE))));
}

#[tokio::test]
async fn an_unstyled_icon_overrides_no_property() {
    let ctx = context();
    let element = Icon::of(Icons::HOUSE).size(24).to_element(&ctx);
    // `Svg` records one entry per node; none of them may override a property.
    assert!(
        overrides(&element, &ctx)
            .iter()
            .all(|entry| *entry == (None, None, None))
    );
}

#[tokio::test]
async fn color_strokes_every_shape_of_the_icon() {
    // `circle-check` is a circle plus a path: both must take the color.
    let ctx = context();
    let element = Icon::of(Icons::CIRCLE_CHECK).color(BLUE).to_element(&ctx);
    let styled = overrides(&element, &ctx);
    assert!(styled.len() >= 2, "expected the circle and the check: {styled:?}");
    assert!(
        styled
            .iter()
            .all(|(fill, stroke, opacity)| *stroke == Some(Some(BLUE)) && fill.is_none() && opacity.is_none()),
        "color touches only the stroke: {styled:?}"
    );
}

#[tokio::test]
async fn fill_and_opacity_combine_with_color() {
    let ctx = context();
    let gold = SvgColor::rgba8(245, 180, 0, 255);
    let element = Icon::of(Icons::STAR)
        .color(gold)
        .fill(gold)
        .opacity(0.5)
        .to_element(&ctx);
    let styled = overrides(&element, &ctx);
    assert!(!styled.is_empty());
    assert!(
        styled
            .iter()
            .all(|entry| *entry == (Some(Some(gold)), Some(Some(gold)), Some(0.5))),
        "{styled:?}"
    );
}

#[tokio::test]
async fn opacity_is_clamped_to_the_unit_range() {
    let ctx = context();
    let element = Icon::of(Icons::HOUSE).opacity(3.0).to_element(&ctx);
    assert!(overrides(&element, &ctx).iter().all(|entry| entry.2 == Some(1.0)));
}

#[tokio::test]
async fn style_replaces_what_color_set_earlier() {
    let ctx = context();
    let element = Icon::of(Icons::HOUSE)
        .color(BLUE)
        .style(SvgStyle::new().opacity(0.25))
        .to_element(&ctx);
    let styled = overrides(&element, &ctx);
    assert!(!styled.is_empty());
    assert!(styled.iter().all(|entry| entry.1.is_none() && entry.2 == Some(0.25)), "{styled:?}");
}

#[tokio::test]
async fn any_parsed_document_works_not_only_the_bundled_icons() {
    let ctx = context();
    let document = SvgDocument::from_svg(
        br#"<svg viewBox="0 0 10 10"><path d="M1 1h8v8z"/></svg>"#,
    )
    .unwrap();
    let element = Icon::new().data(document).size(20).color(BLUE).to_element(&ctx);
    let size = element.layout(&ctx);
    assert_eq!((size.width, size.height), (20.0, 20.0));
    let styled = overrides(&element, &ctx);
    assert_eq!(styled.len(), 1);
    assert_eq!(styled[0].1, Some(Some(BLUE)));
}

#[tokio::test]
async fn every_bundled_icon_can_be_built_as_a_widget() {
    let ctx = context();
    for (name, icon) in Icons::ALL.iter().step_by(40) {
        let size = Icon::of(*icon).size(24).to_element(&ctx).layout(&ctx);
        assert_eq!((size.width, size.height), (24.0, 24.0), "{name}");
    }
}
