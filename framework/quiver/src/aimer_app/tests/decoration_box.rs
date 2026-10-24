use super::*;

use aimer_container::Padding;
use aimer_cupid::draw_cmd_v2::{DrawCommand as LocalCommand, RenderOp};
use aimer_decoration::{BackdropFilter, DecorationBox};
use aimer_space::{Positioned, Stack};
use aimer_style::{BorderSlice, BorderStyle, BoxDecoration, BoxOutline, BoxShadow, LayoutSpacing};

fn decoration_element(root: &dyn Element) -> Option<ElementId> {
    if root.debug_name() == "DecorationBox" {
        return Some(root.id());
    }
    let mut found = None;
    root.visit_retained_v2_children(&mut |_, child| {
        if found.is_none() {
            found = decoration_element(child);
        }
    });
    found
}

#[test]
fn decoration_box_headless_preserves_placement_and_retained_child_paint() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let source = Rc::new(
        BoxDecoration::new()
            .background_color(Color::RED)
            .border_radius(8.0)
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
    );
    let widget = Stack::new().add_child(
        Positioned::new().left(20).top(30).child(
            SizedBox::new().width(40).height(30).child(
                DecorationBox::new().decoration(source.clone()).child(
                    Padding::new()
                        .padding(LayoutSpacing::all(3))
                        .child(SizedBox::new().width(24).height(12).color(Color::BLUE)),
                ),
            ),
        ),
    );
    let mut app = AimerApp::start_headless_with(
        widget,
        HeadlessOptions {
            size: PhysicalSize::new(400, 320),
            scale_factor: 2.0,
        },
    );
    app.pump_frames(3);
    let root = app
        .app
        .widget_root
        .as_ref()
        .expect("headless frame mounted the root");
    let element = decoration_element(root.as_ref()).unwrap_or_else(|| {
        fn names(root: &dyn Element, out: &mut Vec<&'static str>) {
            out.push(root.debug_name());
            root.visit_retained_v2_children(&mut |_, child| names(child, out));
        }
        let mut tree_names = Vec::new();
        names(root.as_ref(), &mut tree_names);
        panic!("DecorationBox must remain mounted: {tree_names:?}");
    });
    let decorated_node = app.app.render_node_for_element(element).unwrap();
    let tree = app.app.render_tree().clone();
    let draws: Vec<_> = tree
        .render_all()
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item),
            _ => None,
        })
        .collect();
    let decorated_index = draws
        .iter()
        .position(|item| item.element == decorated_node)
        .unwrap();
    let decorated = &draws[decorated_index];
    let blue_index = draws
        .iter()
        .position(|item| {
            item.snapshot().commands.iter().any(|command| {
                matches!(command, LocalCommand::FillRect { color, .. }
            if (color.r, color.g, color.b, color.a) == (0, 0, 255, 255))
            })
        })
        .unwrap();
    let blue = &draws[blue_index];

    assert!(
        decorated_index < blue_index,
        "decoration paints behind its child"
    );
    assert_eq!(decorated.bounds, Rect::new(17.0, 25.0, 49.0, 38.0));
    let fill = decorated
        .snapshot()
        .commands
        .iter()
        .find_map(|command| match command {
            LocalCommand::FillRect { rect, .. } => Some(*rect),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        Rect::new(
            decorated.origin.0 + fill.x,
            decorated.origin.1 + fill.y,
            fill.width,
            fill.height
        ),
        Rect::new(20.0, 30.0, 40.0, 30.0)
    );
    assert_eq!(blue.bounds, Rect::new(23.0, 33.0, 24.0, 12.0));
    assert_eq!(
        blue.clip_radius, [0.0; 4],
        "decoration corners do not clip the child"
    );

    let decoration_revision = decorated.snapshot().revision;
    let child_revision = blue.snapshot().revision;
    source.update_color(Color::BLACK);
    app.window.request_redraw();
    app.render_frame();
    let updated = tree.draw_list_snapshot(decorated_node).unwrap();
    assert_eq!(updated.revision, decoration_revision + 1);
    assert!(updated.commands.iter().any(|command| matches!(command,
        LocalCommand::FillRect { color, .. }
        if (color.r, color.g, color.b, color.a) == (0, 0, 0, 255))));
    assert_eq!(
        tree.draw_list_revision(blue.element).unwrap(),
        child_revision
    );
    let damage = &app.last_frame_result.as_ref().unwrap().1;
    assert!(
        damage.is_full()
            || damage.regions().iter().any(|region| {
                region.x <= 34
                    && region.y <= 50
                    && region.x + region.width >= 132
                    && region.y + region.height >= 126
            }),
        "repaint damage must include the shadow and outline"
    );
}

#[test]
fn backdrop_filter_headless_preserves_logical_bounds_and_retained_paint() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let widget = Stack::new().add_child(
        Positioned::new().left(20).top(30).child(
            DecorationBox::new()
                .decoration(BackdropFilter::new().blur_radius(12.0))
                .child(SizedBox::new().width(40).height(30).color(Color::BLUE)),
        ),
    );
    let mut app = AimerApp::start_headless_with(
        widget,
        HeadlessOptions {
            size: PhysicalSize::new(400, 320),
            scale_factor: 2.0,
        },
    );
    app.pump_frames(3);
    let root = app.app.widget_root.as_ref().unwrap();
    let element = decoration_element(root.as_ref()).unwrap();
    let node = app.app.render_node_for_element(element).unwrap();
    let tree = app.app.render_tree().clone();
    let draws: Vec<_> = tree
        .render_all()
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item),
            _ => None,
        })
        .collect();
    let filtered_index = draws.iter().position(|item| item.element == node).unwrap();
    let filtered = &draws[filtered_index];
    assert_eq!(filtered.bounds, Rect::new(20.0, 30.0, 40.0, 30.0));
    let list = filtered.snapshot();
    let [LocalCommand::DrawCustom { data, .. }] = list.commands.as_ref() else {
        panic!("one retained filter request");
    };
    let request = aimer_cupid::material::MaterialRequest::decode(data).unwrap();
    assert_eq!(request.bounds, [0.0, 0.0, 40.0, 30.0]);
    assert_eq!(request.blur_radius, 12.0);
    let child_index = draws
        .iter()
        .position(|item| {
            item.snapshot().commands.iter().any(|command| {
                matches!(command, LocalCommand::FillRect { color, .. }
            if (color.r, color.g, color.b, color.a) == (0, 0, 255, 255))
            })
        })
        .unwrap();
    assert!(filtered_index < child_index);
    app.window.request_redraw();
    app.render_frame();
    assert_eq!(tree.draw_list_revision(node).unwrap(), list.revision);
}
