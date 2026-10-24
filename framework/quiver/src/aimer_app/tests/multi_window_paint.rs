//! Paint state that waits between an event and the frame that answers it
//! belongs to one window.
//!
//! A window records what changed — a full repaint, a hover that has to be drawn
//! — and the next frame *of that window* acts on it. With several windows on one
//! thread, a frame of another window must neither spend what was recorded for
//! this one nor mistake this one's target for its own.

use super::*;

use aimer_cupid::damage_region::DamageSet;
use aimer_input::button::Button;

/// A button the cursor can hover, filling a corner of the window.
struct HoverButton;

impl Widget for HoverButton {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        Button::new()
            .decoration(aimer_style::BoxDecoration::new().background_color(Color::Rgb(30, 30, 90)))
            .hover_decoration(
                aimer_style::BoxDecoration::new().background_color(Color::Rgb(240, 100, 70)),
            )
            .child(
                SizedBox::new()
                    .width(Dimension::Px(60.0))
                    .height(Dimension::Px(40.0)),
            )
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for HoverButton {}

fn open(size: PhysicalSize<u32>) -> HeadlessAimerApp<impl Widget + 'static + use<>> {
    let mut app = AimerApp::start_headless_with(
        HoverButton,
        HeadlessOptions {
            size,
            scale_factor: 1.0,
        },
    );
    // Park the cursor outside the button, then let the window settle.
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(-10.0, -10.0),
    });
    app.pump_frames(8);
    app
}

fn damage(app: &HeadlessAimerApp<impl Widget + 'static>) -> DamageSet {
    app.last_frame_result
        .as_ref()
        .expect("a frame was drawn")
        .1
        .clone()
}

#[test]
fn a_settled_window_is_not_repainted_in_full_because_another_window_drew() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut a = open(PhysicalSize::new(400, 300));
    let mut b = open(PhysicalSize::new(800, 600));

    // Two frames each, so the first-frame repaint is behind both windows.
    for _ in 0..2 {
        a.render_frame();
        b.render_frame();
    }
    a.render_frame();

    assert!(
        !damage(&a).is_full(),
        "window A repainted in full only because window B has another size"
    );
}

#[test]
fn a_full_repaint_asked_of_one_window_is_not_spent_by_another() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut a = open(PhysicalSize::new(400, 300));
    let mut b = open(PhysicalSize::new(800, 600));
    for _ in 0..2 {
        a.render_frame();
        b.render_frame();
    }

    {
        let _active = a.activate();
        aimer_widget::mark_paint_damage_full();
    }
    b.render_frame();
    a.render_frame();

    assert!(
        !damage(&b).is_full(),
        "window B repainted in full for a request made of window A"
    );
    assert!(
        damage(&a).is_full(),
        "window A lost the full repaint that was asked of it"
    );
}

#[test]
fn a_dialog_shown_in_one_window_is_not_shown_in_another() {
    use aimer_modal::{Modal, ModalController};

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut a = open(PhysicalSize::new(400, 300));
    let mut b = open(PhysicalSize::new(800, 600));

    let handle = {
        let _active = a.activate();
        Modal::new().child(SizedBox::new()).show()
    };
    a.render_frame();
    b.render_frame();

    {
        let _active = a.activate();
        assert!(ModalController::is_showing(), "window A lost its dialog");
    }
    {
        let _active = b.activate();
        assert!(
            !ModalController::is_showing(),
            "window B shows a dialog that window A presented"
        );
    }

    {
        let _active = a.activate();
        assert!(handle.dismiss());
    }
    a.render_frame();
    let _active = a.activate();
    assert!(!ModalController::is_showing());
}

#[test]
fn a_dialog_traps_keyboard_focus_in_its_own_window_only() {
    use aimer_modal::Modal;
    use aimer_widget::focus::active_focus_trap;

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut a = open(PhysicalSize::new(400, 300));
    let mut b = open(PhysicalSize::new(800, 600));

    {
        let _active = a.activate();
        Modal::new().child(SizedBox::new()).show();
    }
    a.render_frame();
    b.render_frame();

    {
        let _active = a.activate();
        assert!(active_focus_trap().is_some(), "the dialog did not trap focus");
    }
    let _active = b.activate();
    assert_eq!(
        active_focus_trap(),
        None,
        "window B's keyboard is confined by window A's dialog"
    );
}
