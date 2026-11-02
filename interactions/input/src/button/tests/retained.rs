use super::*;

pub(super) fn mounted_elements(root: &dyn Element) -> Vec<(aimer_widget::ElementId, *const ())> {
    let mut elements = vec![(root.id(), root as *const dyn Element as *const ())];
    root.visit_children(&mut |child| elements.extend(mounted_elements(child)));
    elements
}

#[tokio::test]
async fn hover_and_press_keep_the_mounted_button_wrappers() {
    let mut ctx = context();
    ctx.parent_size = ResolvedSize {
        width: 100.0,
        height: 40.0,
    };
    ctx.box_constraint.max_width = 100.0;
    ctx.box_constraint.max_height = 40.0;
    ctx.cursor_pos = (-10.0, -10.0).into();
    let builds = Rc::new(Cell::new(0));
    let button = Button::new()
        .decoration(BoxDecoration::new().background_color(Color::WHITE))
        .hover_decoration(BoxDecoration::new().background_color(Color::BLUE))
        .press_decoration(BoxDecoration::new().background_color(Color::RED))
        .child(Probe {
            builds: builds.clone(),
        });
    let root = button.to_element(&ctx);
    root.update(&ctx);
    let mut dispatcher = aimer_widget::EventDispatcher::new();
    let outside = PointerInfo::mouse((-10.0, -10.0).into(), PointerButton::Primary);
    let _ = dispatcher.dispatch(
        root.as_ref(),
        outside.pos,
        &ElementEvent::PointerMove(outside),
    );
    root.rebuild_if_dirty(&ctx);
    let mounted = mounted_elements(root.as_ref());
    let pointer = PointerInfo::mouse((1.0, 1.0).into(), PointerButton::Primary);

    let key = aimer_widget::PointerKey::new(aimer_events::pointer::PointerSource::Mouse, 0);
    for (event, color) in [
        (ElementEvent::PointerMove(pointer), Color::BLUE),
        (ElementEvent::PointerDown(pointer), Color::RED),
        (ElementEvent::PointerUp(pointer), Color::BLUE),
        (ElementEvent::PointerMove(outside), Color::WHITE),
    ] {
        let position = match event {
            ElementEvent::PointerMove(info) => info.pos,
            _ => pointer.pos,
        };
        let _ = dispatcher.dispatch(root.as_ref(), position, &event);
        root.rebuild_if_dirty(&ctx);
        if matches!(event, ElementEvent::PointerDown(_)) {
            assert!(dispatcher.captured_owner(key).is_some());
        } else if matches!(event, ElementEvent::PointerUp(_)) {
            assert!(dispatcher.captured_owner(key).is_none());
        }
        assert_eq!(decoration_paint(root.as_ref(), &ctx), color.into());
        assert_eq!(
            mounted_elements(root.as_ref()),
            mounted,
            "hover and press must update paint without replacing retained wrappers"
        );
    }
    assert_eq!(builds.get(), 1);
}

pub(super) fn find_element<'a>(root: &'a dyn Element, name: &str) -> Option<&'a dyn Element> {
    if root.debug_name() == name {
        return Some(root);
    }
    let mut found = None;
    root.visit_children(&mut |child| {
        if found.is_none() {
            found = find_element(child, name);
        }
    });
    found
}

pub(super) fn decoration_paint(root: &dyn Element, ctx: &BuildContext<'_>) -> aimer_cupid::draw_cmd_v2::Color {
    let element = find_element(root, "Container").expect("Button has a decoration owner");
    let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
    let node = tree
        .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 100.0, 40.0))
        .unwrap();
    ctx.with_local_v2_paint_context(tree.context(node).unwrap(), |ctx| {
        assert!(element.can_paint_local_v2(ctx));
        element.paint_local_v2(ctx);
    });
    assert!(!element.local_v2_paint_needs_recording(ctx));
    let commands = tree.draw_list_snapshot(node).unwrap().commands;
    match &commands[0] {
        aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { color, .. } => *color,
        command => panic!("expected a Button background, got {command:?}"),
    }
}

pub(super) fn button_context() -> BuildContext<'static> {
    let mut ctx = context();
    ctx.parent_size = ResolvedSize {
        width: 100.0,
        height: 40.0,
    };
    ctx.box_constraint.max_width = 100.0;
    ctx.box_constraint.max_height = 40.0;
    ctx.cursor_pos = (-10.0, -10.0).into();
    ctx
}

#[tokio::test]
async fn cancelling_a_retained_press_restores_hover_paint() {
    let ctx = button_context();
    let presses = Rc::new(Cell::new(0));
    let root = Button::new()
        .decoration(BoxDecoration::new().background_color(Color::WHITE))
        .hover_decoration(BoxDecoration::new().background_color(Color::BLUE))
        .press_decoration(BoxDecoration::new().background_color(Color::RED))
        .on_press({
            let presses = presses.clone();
            move || presses.set(presses.get() + 1)
        })
        .child(Probe {
            builds: Rc::new(Cell::new(0)),
        })
        .to_element(&ctx);
    root.update(&ctx);
    let mounted = mounted_elements(root.as_ref());
    let mut dispatcher = aimer_widget::EventDispatcher::new();
    let pointer = PointerInfo::mouse((1.0, 1.0).into(), PointerButton::Primary);
    let _ = dispatcher.dispatch(
        root.as_ref(),
        pointer.pos,
        &ElementEvent::PointerDown(pointer),
    );
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::RED.into());
    let _ = dispatcher.broadcast(root.as_ref(), &ElementEvent::Cancel);
    root.rebuild_if_dirty(&ctx);
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::BLUE.into());
    assert_eq!(mounted_elements(root.as_ref()), mounted);
    assert_eq!(presses.get(), 0);
}

#[tokio::test]
async fn retained_hover_observes_parent_decoration_and_disabled_updates() {
    let ctx = button_context();
    let button = Button::new()
        .decoration(BoxDecoration::new().background_color(Color::WHITE))
        .hover_decoration(BoxDecoration::new().background_color(Color::BLUE))
        .disable_decoration(BoxDecoration::new().background_color(Color::GRAY))
        .child(Probe {
            builds: Rc::new(Cell::new(0)),
        });
    let (root, updater) =
        aimer_widget::StatefulElement::new_with_name(button, &ctx, "Button", None);
    let root = root.boxed();
    root.update(&ctx);
    let mut dispatcher = aimer_widget::EventDispatcher::new();
    let pointer = PointerInfo::mouse((1.0, 1.0).into(), PointerButton::Primary);
    let _ = dispatcher.dispatch(
        root.as_ref(),
        pointer.pos,
        &ElementEvent::PointerMove(pointer),
    );
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::BLUE.into());
    updater.set_state(|state| {
        state.hover_decoration = Some(BoxDecoration::new().background_color(Color::RED));
    });
    root.rebuild_if_dirty(&ctx);
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::RED.into());
    updater.set_state(|state| state.is_disabled = true);
    root.rebuild_if_dirty(&ctx);
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::GRAY.into());
    assert!(find_element(root.as_ref(), "GestureDetector").is_none());
    updater.set_state(|state| state.is_disabled = false);
    root.rebuild_if_dirty(&ctx);
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::RED.into());
    assert!(find_element(root.as_ref(), "GestureDetector").is_some());
}

#[tokio::test]
async fn retained_feedback_keeps_default_lightening_and_darkening() {
    let ctx = button_context();
    let root = Button::new()
        .decoration(BoxDecoration::new().background_color(Color::BLACK))
        .child(Probe {
            builds: Rc::new(Cell::new(0)),
        })
        .to_element(&ctx);
    root.update(&ctx);
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::BLACK.into());
    let mut dispatcher = aimer_widget::EventDispatcher::new();
    let pointer = PointerInfo::mouse((1.0, 1.0).into(), PointerButton::Primary);
    for (event, channel) in [
        (ElementEvent::PointerMove(pointer), 51),
        (ElementEvent::PointerDown(pointer), 43),
        (ElementEvent::PointerUp(pointer), 51),
    ] {
        let _ = dispatcher.dispatch(root.as_ref(), pointer.pos, &event);
        root.rebuild_if_dirty(&ctx);
        assert_eq!(
            decoration_paint(root.as_ref(), &ctx),
            Color::Rgba(channel, channel, channel, 255).into()
        );
    }
}

pub(super) fn child_geometry(root: &dyn Element, ctx: &BuildContext<'_>) -> (f32, f32, [f32; 4]) {
    let decoration = find_element(root, "Container").unwrap();
    let mut child = None;
    decoration.visit_children(&mut |element| child = Some(element));
    let child = child.unwrap();
    let child_ctx = decoration.retained_v2_child_context(ctx, child).unwrap();
    (
        child_ctx.box_constraint.max_width,
        child_ctx.box_constraint.max_height,
        decoration.retained_v2_child_clip_radius(ctx, child),
    )
}

#[tokio::test]
async fn retained_feedback_refreshes_border_insets_and_rounded_clipping() {
    use aimer_style::{BorderSlice, BorderStyle, BoxBorder};

    let ctx = button_context();
    let border = |width| {
        BoxBorder::all(
            BorderSlice::new()
                .style(BorderStyle::Solid)
                .stroke(width)
                .color(Color::BLACK),
        )
    };
    let root = Button::new()
        .decoration(
            BoxDecoration::new()
                .background_color(Color::WHITE)
                .border(border(1.0))
                .border_radius(8.0),
        )
        .hover_decoration(
            BoxDecoration::new()
                .background_color(Color::BLUE)
                .border(border(3.0))
                .border_radius(12.0),
        )
        .press_decoration(
            BoxDecoration::new()
                .background_color(Color::RED)
                .border(border(3.0))
                .border_radius(16.0),
        )
        .child(Probe {
            builds: Rc::new(Cell::new(0)),
        })
        .to_element(&ctx);
    root.update(&ctx);
    let mounted = mounted_elements(root.as_ref());
    assert_eq!(child_geometry(root.as_ref(), &ctx), (98.0, 38.0, [7.0; 4]));
    let mut dispatcher = aimer_widget::EventDispatcher::new();
    let pointer = PointerInfo::mouse((10.0, 10.0).into(), PointerButton::Primary);
    let _ = dispatcher.dispatch(
        root.as_ref(),
        pointer.pos,
        &ElementEvent::PointerMove(pointer),
    );
    root.rebuild_if_dirty(&ctx);
    assert_eq!(child_geometry(root.as_ref(), &ctx), (94.0, 34.0, [9.0; 4]));
    let _ = dispatcher.dispatch(
        root.as_ref(),
        pointer.pos,
        &ElementEvent::PointerDown(pointer),
    );
    root.rebuild_if_dirty(&ctx);
    assert_eq!(child_geometry(root.as_ref(), &ctx), (94.0, 34.0, [13.0; 4]));
    assert_eq!(decoration_paint(root.as_ref(), &ctx), Color::RED.into());
    assert_eq!(mounted_elements(root.as_ref()), mounted);
}

/// A button is as big as its label, not as big as the flex line it sits in.
/// `Row` offers a child the whole cross axis and `Column` the whole main one,
/// so a container that fills its constraint made the button, and the area that
/// answers presses, as tall as the row.
#[tokio::test]
async fn a_button_in_a_flex_hugs_its_child_on_both_axes() {
    use aimer_container::SizedBox;
    use aimer_flex::{Column, Row};

    let mut ctx = button_context();
    ctx.parent_size = ResolvedSize { width: 300.0, height: 200.0 };
    ctx.box_constraint.max_width = 300.0;
    ctx.box_constraint.max_height = 200.0;
    let button = || {
        Button::new()
            .decoration(BoxDecoration::new().background_color(Color::WHITE))
            .child(SizedBox::new().width(30).height(20))
    };

    let row = Row::new().children([button()]).to_element(&ctx);
    row.update(&ctx);
    let size = row.computed_size(&ctx);
    assert_eq!((size.width, size.height), (30.0, 20.0), "row");

    let column = Column::new().children([button()]).to_element(&ctx);
    column.update(&ctx);
    let size = column.computed_size(&ctx);
    assert_eq!((size.width, size.height), (30.0, 20.0), "column");
}

/// The usual label button wraps its text in an automatic, padded container.
#[tokio::test]
async fn a_button_with_a_padded_label_container_does_not_fill_the_flex() {
    use aimer_container::{Container, SizedBox};
    use aimer_flex::{Column, Row};
    use aimer_style::LayoutSpacing;

    let mut ctx = button_context();
    ctx.parent_size = ResolvedSize { width: 300.0, height: 200.0 };
    ctx.box_constraint.max_width = 300.0;
    ctx.box_constraint.max_height = 200.0;
    let button = || {
        Button::new()
            .decoration(BoxDecoration::new().background_color(Color::WHITE).border_radius(12))
            .child(
                Container::new()
                    .padding(LayoutSpacing::new().left(24).right(24).top(12).bottom(12))
                    .child(SizedBox::new().width(50).height(10)),
            )
    };

    let row = Row::new().children([button()]).to_element(&ctx);
    row.update(&ctx);
    let size = row.computed_size(&ctx);
    assert_eq!((size.width, size.height), (98.0, 34.0), "row");

    let column = Column::new().children([button()]).to_element(&ctx);
    column.update(&ctx);
    let size = column.computed_size(&ctx);
    assert_eq!((size.width, size.height), (98.0, 34.0), "column");
}
