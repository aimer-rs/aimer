//! Dragging a panel's edge resizes it, and the pane beside it, on every frame.

use super::*;

use aimer_container::{Direction, Resizable};
use aimer_style::BoxDecoration;
use winit::event::{ElementState, MouseButton};

const WIDTH: f32 = 250.0;

struct Layout;

impl Widget for Layout {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        Row::new()
            .children(vec![
                // A panel decorated like a sidebar: the resizable sits inside a container.
                aimer_container::Container::new()
                    .box_decoration(BoxDecoration::new().background_color(Color::Rgb(30, 22, 20)))
                    .child(
                        Resizable::new()
                            .height(f32::MAX)
                            .width(WIDTH)
                            .min_width(100.0)
                            .max_width(400.0)
                            .handle_thickness(14.0)
                            .handle_outset(12.0)
                            .direction(Direction::RIGHT)
                            .child(
                                SizedBox::new()
                                    .width(Dimension::Px(WIDTH))
                                    .height(Dimension::Px(400.0))
                                    .color(Color::Rgb(40, 30, 30)),
                            ),
                    )
                    .boxed(),
                aimer_flex::Expanded::new()
                    .child(
                        Scrollable::new().child(
                            Column::new().children(
                                (0..8)
                                    .map(|_| {
                                        aimer_container::Container::new()
                                            .padding(aimer_style::LayoutSpacing::all(4))
                                            .child(
                                                SizedBox::new()
                                                    .height(Dimension::Px(60.0))
                                                    .color(Color::Rgb(20, 20, 20)),
                                            )
                                            .boxed()
                                    })
                                    .collect::<Vec<_>>(),
                            ),
                        ),
                    )
                    .boxed(),
            ])
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for Layout {}

fn find<'a>(element: &'a dyn Element, name: &str) -> Option<&'a dyn Element> {
    if element.debug_name() == name {
        return Some(element);
    }
    let mut found = None;
    element.structural_children(&mut |child| {
        if found.is_none() {
            found = find(child, name);
        }
    });
    found
}

/// The width the resizable itself reports.
fn panel_width(app: &HeadlessAimerApp<impl Widget + 'static>) -> f32 {
    let root = app.app.widget_root.as_ref().unwrap().as_ref();
    match find(root, "Resizable")
        .and_then(|resizable| resizable.size())
        .map(|size| size.width)
    {
        Some(Dimension::Px(width)) => width,
        _ => f32::NAN,
    }
}

/// The left edge of the first card in the pane.
fn card_left_edge(app: &HeadlessAimerApp<impl Widget + 'static>) -> Option<f32> {
    let root = app.app.widget_root.as_ref()?.as_ref();
    let scroll = find(root, "RawScrollableContainer")?;
    let mut edge = None;
    fn first_bounds(element: &dyn Element, edge: &mut Option<f32>) {
        if edge.is_some() {
            return;
        }
        if element.debug_name() == "Container"
            && let Some((start, _)) = element.pos_start_end()
        {
            *edge = Some(start.x);
            return;
        }
        element.structural_children(&mut |child| first_bounds(child, edge));
    }
    first_bounds(scroll, &mut edge);
    edge
}

#[test]
fn the_pane_beside_a_dragged_panel_follows_it_on_every_frame() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless_with(
        Layout,
        HeadlessOptions {
            size: PhysicalSize::new(800, 400),
            scale_factor: 1.0,
        },
    );
    let at = |app: &mut HeadlessAimerApp<_>, x: f64| {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, 200.0),
        });
        app.pump_frames(1);
    };
    at(&mut app, 600.0);
    app.pump_frames(3);
    at(&mut app, f64::from(WIDTH) - 2.0);
    app.send_window_event(WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Pressed,
        button: MouseButton::Left,
    });
    app.pump_frames(1);

    let mut wrong = Vec::new();
    for step in 1..=24 {
        let x = f64::from(WIDTH) - 2.0 + f64::from(step) * 5.0;
        at(&mut app, x);
        // The panel is as wide as the pointer has dragged it, up to its maximum,
        // and the first card of the pane starts where the panel ends.
        let expected = (x + 2.0).min(400.0) as f32;
        let card = card_left_edge(&app).expect("the pane has a card");
        let panel = panel_width(&app);
        if (card - expected).abs() > 2.0 || (panel - expected).abs() > 2.0 {
            wrong.push((step, x, panel, card, expected));
        }
    }
    assert!(
        wrong.is_empty(),
        "(step, pointer, panel's width, pane's left edge, expected): the pane did not follow the dragged edge on these frames: {wrong:?}"
    );

    app.send_window_event(WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Released,
        button: MouseButton::Left,
    });
    app.pump_frames(2);
}
