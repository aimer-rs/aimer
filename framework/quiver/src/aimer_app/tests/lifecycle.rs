//! The event loop reports the moments an app can lose unsaved state, so the
//! services that hold some (see `aimer_persist`) can finish their work.

use std::cell::RefCell;

use aimer_venus::{Lifecycle, on_lifecycle};

use super::*;

/// Records every lifecycle notification until dropped.
struct Recorder {
    seen: Rc<RefCell<Vec<Lifecycle>>>,
    _subscription: aimer_venus::LifecycleSubscription,
}

impl Recorder {
    fn new() -> Self {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        Self {
            seen,
            _subscription: on_lifecycle(move |event| log.borrow_mut().push(event)),
        }
    }

    fn events(&self) -> Vec<Lifecycle> {
        self.seen.borrow().clone()
    }
}

fn started() -> HeadlessAimerApp<ModalHost<SizedBox>> {
    let mut app = AimerApp::start_headless(SizedBox::new());
    app.render_frame();
    app
}

#[test]
fn losing_focus_reports_that_attention_left() {
    let recorder = Recorder::new();
    let mut app = started();

    app.send_window_event(WindowEvent::Focused(true));
    assert_eq!(recorder.events(), [], "gaining focus is not a warning");

    app.send_window_event(WindowEvent::Focused(false));
    // Mobile platforms treat it as the start of backgrounding, which may end in
    // the process being killed; everywhere else it only asks to start saving.
    let expected = if cfg!(any(target_os = "android", target_os = "ios")) {
        Lifecycle::Background
    } else {
        Lifecycle::Inactive
    };
    assert_eq!(recorder.events(), [expected]);
}

#[test]
fn a_window_that_becomes_hidden_reports_it_and_one_that_reappears_does_not() {
    let recorder = Recorder::new();
    let mut app = started();

    app.send_window_event(WindowEvent::Occluded(false));
    assert_eq!(recorder.events(), []);

    app.send_window_event(WindowEvent::Occluded(true));
    assert_eq!(recorder.events().len(), 1);
}

/// Before any widget tree exists the focus handler returns early; the warning
/// must still go out, because that is exactly when nothing else is looking.
#[test]
fn focus_loss_is_reported_even_without_an_active_root() {
    let recorder = Recorder::new();
    let mut app = AimerApp::start_headless(SizedBox::new());

    app.send_window_event(WindowEvent::Focused(false));

    assert_eq!(recorder.events().len(), 1);
}
