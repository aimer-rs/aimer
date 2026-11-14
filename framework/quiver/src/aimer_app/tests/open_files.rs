//! Files the application is opened with reach the handlers the app registered,
//! through the same event the platform wakes the loop with.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use aimer_events::file_open;
use aimer_platform::file_inbox;

use super::*;

fn app() -> HeadlessAimerApp<ModalHost<RecordingWidget>> {
    let widget = RecordingWidget { builds: Arc::new(AtomicUsize::new(0)), cancels: Arc::new(AtomicUsize::new(0)) };
    AimerApp::start_headless_with(widget, HeadlessOptions { size: PhysicalSize::new(400, 300), scale_factor: 1.0 })
}

#[test]
fn a_file_posted_from_the_platform_reaches_the_registered_handler_when_the_loop_wakes() {
    let mut app = app();
    let opened = Rc::new(RefCell::new(Vec::new()));
    let sink = opened.clone();
    let _handler = file_open::register(move |paths: &[PathBuf]| {
        sink.borrow_mut().extend_from_slice(paths);
        true
    });

    // The platform posts from its own thread and wakes the loop; the loop drains.
    std::thread::spawn(|| file_inbox::post_files([PathBuf::from("/work/main.rs"), PathBuf::from("/work/lib.rs")]))
        .join()
        .unwrap();
    assert!(opened.borrow().is_empty(), "nothing is delivered on the posting thread");
    app.send_user_event(AimerNativePlatformEvent::OpenFiles);

    assert_eq!(*opened.borrow(), [PathBuf::from("/work/main.rs"), PathBuf::from("/work/lib.rs")]);
}

#[test]
fn a_file_that_launched_the_app_waits_for_the_first_handler_to_claim_it() {
    let mut app = app();
    std::thread::spawn(|| file_inbox::post_files([PathBuf::from("/work/launch.rs")])).join().unwrap();
    app.send_user_event(AimerNativePlatformEvent::OpenFiles);

    let mut claimed = Vec::new();
    assert!(file_open::claim_waiting_files(|paths| {
        claimed.extend_from_slice(paths);
        true
    }));
    assert_eq!(claimed, [PathBuf::from("/work/launch.rs")]);
}
