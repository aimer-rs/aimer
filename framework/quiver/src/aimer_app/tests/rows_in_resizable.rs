//! Rows in a scrollable inside a resizable panel stop being hovered when the
//! pointer slides off them into the panel's grab band.
//!
//! The shape is a sidebar: a resizable panel whose content is a scrollable
//! list of buttons, next to a content pane.

use super::*;

use aimer_container::{Direction, Resizable};
use aimer_input::button::Button;

const WIDTH: f32 = 250.0;
const ROWS: usize = 8;

struct Layout;

impl Widget for Layout {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let rows: Vec<aimer_widget::AnyWidget> = (0..ROWS)
            .map(|_| {
                Button::new()
                    .hover_decoration(
                        aimer_style::BoxDecoration::new()
                            .border_radius(8)
                            .background_color(Color::Rgb(200, 100, 80)),
                    )
                    .decoration(
                        aimer_style::BoxDecoration::new()
                            .border_radius(8)
                            .background_color(Color::Transparent),
                    )
                    .on_press(|| {})
                    .child(SizedBox::new().height(Dimension::Px(35.0)))
                    .boxed()
            })
            .collect();
        Row::new()
            .children(vec![
                Resizable::new()
                    .height(f32::MAX)
                    .width(WIDTH)
                    .min_width(100.0)
                    .max_width(400.0)
                    .handle_thickness(14.0)
                    .handle_outset(12.0)
                    .direction(Direction::RIGHT)
                    .child(
                        // The panel pads its content, so the scrollable's viewport ends
                        // short of the panel's edge, where the grab band begins.
                        aimer_container::Container::new()
                            .width(Dimension::Px(WIDTH))
                            .padding(aimer_style::LayoutSpacing::horizontal(12.into()))
                            .child(
                                Scrollable::new().child(
                                    Column::new()
                                        .gaps(aimer_style::LayoutSpacing::all(4))
                                        .children(rows),
                                ),
                            ),
                    )
                    .boxed(),
                SizedBox::new()
                    .width(Dimension::Px(300.0))
                    .height(Dimension::Px(400.0))
                    .boxed(),
            ])
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for Layout {}

fn hover_fills<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> usize {
    fn walk<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        element: &dyn Element,
        hover: &str,
        out: &mut usize,
    ) {
        if let Some(node) = app.app.render_node_for_element(element.id())
            && let Ok(snapshot) = app.app.render_tree().draw_list_snapshot(node)
        {
            for command in snapshot.commands.iter() {
                if let aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { color, .. } = command
                    && format!("{color:?}") == hover
                {
                    *out += 1;
                }
            }
        }
        element.visit_children(&mut |child| walk(app, child, hover, out));
    }
    let hover: aimer_cupid::utilities::Color = Color::Rgb(200, 100, 80).into();
    let mut out = 0;
    walk(
        app,
        app.app.widget_root.as_ref().unwrap().as_ref(),
        &format!("{hover:?}"),
        &mut out,
    );
    out
}

fn move_to<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    app.pump_frames(2);
}

fn run(exit_path: &[(f64, f64)]) -> usize {
    let mut app = AimerApp::start_headless_with(
        Layout,
        HeadlessOptions {
            size: PhysicalSize::new(600, 400),
            scale_factor: 1.0,
        },
    );
    move_to(&mut app, 500.0, 200.0);
    app.pump_frames(3);
    // Down the list on the left of the panel.
    let mut hovered = 0;
    for step in 0..9 {
        move_to(&mut app, 100.0, 20.0 + f64::from(step) * 40.0);
        hovered = hovered.max(hover_fills(&app));
    }
    assert!(hovered > 0, "the pointer never hovered a row");
    for (x, y) in exit_path {
        move_to(&mut app, *x, *y);
    }
    app.pump_frames(5);
    hover_fills(&app)
}

#[test]
fn rows_are_not_left_hovered_after_sliding_into_the_grab_band() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let left = run(&[
        (200.0, 300.0),
        (240.0, 300.0),
        (246.0, 300.0),
        (252.0, 300.0),
        (258.0, 300.0),
        (400.0, 300.0),
    ]);
    assert_eq!(
        left, 0,
        "rows are still painted hovered after the pointer crossed the grab band into the pane"
    );
}

#[test]
fn rows_are_not_left_hovered_after_stopping_in_the_grab_band() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let left = run(&[(200.0, 300.0), (240.0, 300.0), (246.0, 300.0)]);
    assert_eq!(
        left, 0,
        "rows are still painted hovered while the pointer rests in the grab band"
    );
}

#[test]
fn rows_are_not_left_hovered_after_leaving_the_panel_far_away() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let left = run(&[(500.0, 300.0)]);
    assert_eq!(
        left, 0,
        "rows are still painted hovered after the pointer left the panel"
    );
}
