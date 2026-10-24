//! The caret a `TextField` paints follows its insertion point as the text is edited.
//!
//! The caret is a sibling of the field's own paint, placed by the geometry the
//! field publishes while it paints. That geometry only exists after the frame's
//! paint, so a placement that is not carried to the render tree leaves the caret
//! where the previous edit put it, away from the text being typed.

use super::*;

use aimer_cupid::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderOp};
use winit::event::ElementState;

const CARET: (u8, u8, u8) = (255, 0, 255);

pub(super) fn click(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: winit::event::MouseButton::Left,
        });
    }
}

pub(super) fn type_text(app: &mut HeadlessAimerApp<impl Widget + 'static>, text: &str) {
    app.send_user_event(AimerNativePlatformEvent::InsertText(text.to_owned()));
}

/// The left edge of the painted caret, in window coordinates.
pub(super) fn caret_x(app: &HeadlessAimerApp<impl Widget + 'static>) -> f32 {
    let tree = app.app.render_tree();
    tree.render_all()
        .into_iter()
        .find_map(|op| {
            let RenderOp::Draw(item) = op else { return None };
            let paint = tree.draw_list_snapshot(item.element).ok()?;
            paint
                .commands
                .iter()
                .any(|command| {
                    matches!(command, V2DrawCommand::FillRect { color, .. }
                        if (color.r, color.g, color.b) == CARET)
                })
                .then_some(item.bounds.x)
        })
        .expect("a focused field paints its caret")
}

#[test]
fn the_caret_follows_text_typed_into_a_field() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless_with(
        Column::new().children([TextField::new()
            .controller(TextEditingController::with_text("Hello"))
            .cursor_color(aimer_widget::base::Colors::Magenta)
            .max_length(None)
            .boxed()]),
        HeadlessOptions {
            size: PhysicalSize::new(400, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    click(&mut app, 100.0, 12.0);
    app.pump_frames(3);
    let before = caret_x(&app);

    type_text(&mut app, " and a longer tail");
    app.pump_frames(3);
    let after = caret_x(&app);

    assert!(
        after > before + 40.0,
        "the caret stayed at x={before} while 18 characters were typed: now x={after}"
    );
}

/// A caret that moved is a render-tree matter: the layout of everything else is
/// where it was. Retiring every layout cache of the window for it made each frame
/// of a field that animates while focused measure the whole tree again.
#[test]
fn moving_the_caret_leaves_the_layout_caches_alone() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless_with(
        Column::new().children([TextField::new()
            .controller(TextEditingController::with_text("Hello"))
            .cursor_color(aimer_widget::base::Colors::Magenta)
            .max_length(None)
            .boxed()]),
        HeadlessOptions {
            size: PhysicalSize::new(400, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    click(&mut app, 100.0, 12.0);
    app.pump_frames(3);
    let before = caret_x(&app);
    let layout_generation = aimer_widget::layout_invalidation_generation();

    type_text(&mut app, " and a longer tail");
    app.pump_frames(3);

    assert!(caret_x(&app) > before + 40.0, "the caret did not follow the text");
    assert_eq!(
        aimer_widget::layout_invalidation_generation(),
        layout_generation,
        "moving the caret retired the layout caches of the whole window"
    );
}
