use std::sync::Arc;

use aimer_widget::Widget;

use crate::{StaticSvg, Svg, SvgColor, SvgStyle};

static ICON: StaticSvg = StaticSvg::new("test-icon", r#"<svg viewBox="0 0 24 24"
    fill="none" stroke="currentColor"><path d="M2 12h20"/></svg>"#);

#[test]
#[should_panic(expected = "invalid embedded SVG broken-icon")]
fn invalid_embedded_svg_reports_its_name() {
    StaticSvg::new("broken-icon", "<svg>").document();
}

#[test]
fn static_documents_are_shared_across_threads() {
    static SHARED: StaticSvg = StaticSvg::new("shared", "<svg viewBox='0 0 24 24'><path d='M0 0h24'/></svg>");
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let threads = (0..4).map(|_| {
        let barrier = barrier.clone();
        std::thread::spawn(move || { barrier.wait(); SHARED.document() })
    }).collect::<Vec<_>>();
    for thread in threads {
        assert!(Arc::ptr_eq(thread.join().unwrap().scene(), SHARED.document().scene()));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn styles(element: &dyn aimer_widget::Element, context: &aimer_widget::base::BuildContext)
    -> Arc<[aimer_cupid::svg::SvgNodeStyleOverride]>
{
    let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
    let node = tree.add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 48.0, 48.0)).unwrap();
    context.with_local_v2_paint_context(tree.context(node).unwrap(), |ctx| element.paint_local_v2(ctx));
    let drawing = tree.draw_list_snapshot(node).unwrap();
    drawing.commands.iter().find_map(|command| match command {
        aimer_cupid::draw_cmd_v2::DrawCommand::Svg { overrides, .. } => Some(overrides.clone()),
        _ => None,
    }).unwrap()
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn static_icon_documents_share_geometry_and_accept_svg_styles() {
    let first = ICON.document();
    let second = ICON.document();
    assert!(Arc::ptr_eq(first.scene(), second.scene()));
    let red = SvgColor::rgba8(255, 0, 0, 255);
    let styled = Svg::new(&ICON).width(48).height(48)
        .try_style("path", SvgStyle::new().stroke(red).fill(red).opacity(0.5)).unwrap()
        .try_hover_style("path", SvgStyle::new().opacity(0.8)).unwrap()
        .try_pressed_style("path", SvgStyle::new().opacity(0.3)).unwrap();
    // Observe sizing and styling through the retained drawing contract.
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let _guard = runtime.enter();
    let inner = aimer_canvas::InnerCanvas::new();
    let mut context = aimer_widget::base::BuildContext::new(
        aimer_canvas::FrameCanvas::new(&inner),
        aimer_attribute::ResolvedSize { width: 48.0, height: 48.0 },
        1.0, Default::default(), Default::default(),
        aimer_widget::base::WindowHandle::headless(Default::default(), 1.0),
        runtime.handle().clone(),
    );
    context.box_constraint = aimer_attribute::BoxConstraint {
        min_width: 0.0, min_height: 0.0, max_width: 48.0, max_height: 48.0,
    };
    let element = styled.to_element(&context);
    assert_eq!(element.computed_size(&context).width, 48.0);
    let overrides = styles(element.as_ref(), &context);
    assert!(overrides.iter().any(|style| style.stroke == Some(Some(red))
        && style.fill == Some(Some(red)) && style.opacity == Some(0.5)));
    element.sync_paint_geometry(&context);
    let pointer = aimer_events::pointer::PointerInfo::mouse(
        aimer_attribute::Vec2d { x: 24.0, y: 24.0 }, aimer_events::pointer::PointerButton::Primary,
    );
    use aimer_events::element::ElementEvent;
    for (event, opacity) in [
        (ElementEvent::PointerMove(pointer), 0.8),
        (ElementEvent::PointerDown(pointer), 0.3),
        (ElementEvent::PointerUp(pointer), 0.8),
    ] {
        let _ = element.on_event(&event);
        assert!(styles(element.as_ref(), &context).iter().any(|style| style.opacity == Some(opacity)));
    }
    assert_eq!(first.scene().nodes[0].stroke.as_ref().unwrap().color,
        SvgColor::rgba8(0, 0, 0, 255));
    assert!(first.scene().nodes[0].fill.is_none());
    assert_eq!(ICON.source(), r#"<svg viewBox="0 0 24 24"
    fill="none" stroke="currentColor"><path d="M2 12h20"/></svg>"#);
}
