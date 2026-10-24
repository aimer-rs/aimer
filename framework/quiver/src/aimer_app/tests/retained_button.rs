use super::*;

struct ButtonRows;

impl Widget for ButtonRows {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        Column::new()
            .children((0..3).map(|_| {
                aimer_input::button::Button::new()
                    .decoration(aimer_style::BoxDecoration::new().background_color(Color::WHITE))
                    .hover_decoration(
                        aimer_style::BoxDecoration::new().background_color(Color::BLUE),
                    )
                    .press_decoration(
                        aimer_style::BoxDecoration::new().background_color(Color::RED),
                    )
                    .child(aimer_container::SizedBox::new().width(128.0).height(40.0))
            }))
            .to_element(ctx)
    }
}
impl aimer_widget::PortableWidget for ButtonRows {}

fn decoration_nodes<W: Widget + 'static>(
    root: &dyn Element,
    app: &HeadlessAimerApp<W>,
) -> Vec<aimer_cupid::draw_cmd_v2::RenderNodeId> {
    let mut nodes = Vec::new();
    if root.debug_name() == "Container" {
        nodes.push(app.app.render_node_for_element(root.id()).unwrap());
    }
    root.visit_children(&mut |child| nodes.extend(decoration_nodes(child, app)));
    nodes
}

#[test]
fn button_hover_repaints_only_the_old_and_new_decoration_nodes() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless_with(
        ButtonRows,
        HeadlessOptions {
            size: PhysicalSize::new(128, 320),
            scale_factor: 1.0,
        },
    );
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(-10.0, -10.0),
    });
    app.pump_frames(2);
    let _ = direct_headless_frame_packet(&mut app);
    let root = app.app.widget_root.as_ref().unwrap();
    let mut nodes = decoration_nodes(root.as_ref(), &app);
    nodes.sort_by(|left, right| {
        app.app
            .render_tree()
            .element_bounds(*left)
            .unwrap()
            .y
            .total_cmp(&app.app.render_tree().element_bounds(*right).unwrap().y)
    });
    assert_eq!(nodes.len(), 3);
    let mut revisions = nodes
        .iter()
        .map(|node| app.app.render_tree().draw_list_revision(*node).unwrap())
        .collect::<Vec<_>>();

    for row in [0, 1] {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(10.0, 20.0 + row as f64 * 40.0),
        });
        assert!(
            app.window.take_redraw_request(),
            "row {row} must schedule hover paint"
        );
        let (_, packet) = direct_headless_frame_packet(&mut app);
        assert!(
            !packet.metadata().damage().is_full(),
            "hover paint damage should stay local"
        );
        for (index, node) in nodes.iter().enumerate() {
            let expected = revisions[index] + u64::from(index == row || row == 1 && index == 0);
            assert_eq!(
                app.app.render_tree().draw_list_revision(*node).unwrap(),
                expected,
                "hover should re-record only the old and new Button decoration"
            );
            revisions[index] = expected;
        }
        let _ = direct_headless_frame_packet(&mut app);
        for (index, node) in nodes.iter().enumerate() {
            assert_eq!(
                app.app.render_tree().draw_list_revision(*node).unwrap(),
                revisions[index]
            );
        }
    }
}
