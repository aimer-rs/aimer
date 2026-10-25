//! A `Text` that aligns itself inside its parent follows the parent when the
//! window is resized.
//!
//! The text records its alignment against the size of the box it sits in, which
//! is not the size of the text: a centered `Hello` stays the same few pixels
//! wide while the window around it grows. Nothing about the text's own bounds
//! changes, so the recorded paint has to be refreshed on the parent's account.

use super::*;

use aimer_container::Container;
use aimer_cupid::draw_cmd_v2::{DrawCommand, RenderOp};
use aimer_style::TextAlign;
use aimer_text::Text;

/// The box the text run aligns itself in: `(width, height)`.
fn aligned_in<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> (f32, f32) {
    app.app
        .render_tree()
        .render_all()
        .into_iter()
        .find_map(|op| match op {
            RenderOp::Draw(item) => item.snapshot().commands.iter().find_map(|cmd| match cmd {
                DrawCommand::DrawText { bounds_width, bounds_height, .. } => {
                    Some((bounds_width.unwrap_or(-1.0), bounds_height.unwrap_or(-1.0)))
                }
                _ => None,
            }),
            _ => None,
        })
        .expect("the text must be drawn")
}

fn centered_hello(size: (u32, u32)) -> HeadlessAimerApp<impl Widget + 'static> {
    AimerApp::start_headless_with(
        Container::new()
            .color(Color::WHITE)
            .child(Text::new("Hello Aimer").text_align(TextAlign::MidCenter)),
        HeadlessOptions { size: PhysicalSize::new(size.0, size.1), scale_factor: 1.0 },
    )
}

#[test]
fn centered_text_realigns_when_the_window_resizes() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = centered_hello((800, 600));
    app.pump_frames(4);
    assert_eq!(aligned_in(&app), (800.0, 600.0));

    for (width, height) in [(400, 300), (1000, 700), (400, 300)] {
        app.send_window_event(WindowEvent::Resized(PhysicalSize::new(width, height)));
        app.pump_frames(4);
        assert_eq!(
            aligned_in(&app),
            (width as f32, height as f32),
            "the text must center in a {width}x{height} window"
        );
    }
}

#[test]
fn unchanged_window_leaves_the_aligned_text_alone() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = centered_hello((800, 600));
    app.pump_frames(4);

    // The staleness check runs for every text node on every frame, so frames
    // that change nothing must leave the alignment and the tree as they were.
    let before = app.app.render_tree().render_all().len();
    app.pump_frames(4);
    assert_eq!(app.app.render_tree().render_all().len(), before);
    assert_eq!(aligned_in(&app), (800.0, 600.0));
}
