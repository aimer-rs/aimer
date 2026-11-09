//! `Capture` finds where its child is in the window, and a picture of it is that
//! part of a frame.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use aimer_capture::{Capture, CaptureHandle, ShotError};
use aimer_container::SizedBox;
use aimer_platform::capture::Frame;

use super::*;

/// A 400 x 300 window with a 100 x 60 child 30 across and 20 down.
fn page(handle: &CaptureHandle) -> HeadlessAimerApp<ModalHost<aimer_space::Stack>> {
    let widget = aimer_space::Stack::new().children(vec![
        aimer_space::Positioned::new()
            .left(30)
            .top(20)
            .box_child(Capture::new().handle(handle).child(SizedBox::new().width(100.0).height(60.0)))
            .boxed(),
    ]);
    let mut app = AimerApp::start_headless_with(
        widget,
        HeadlessOptions { size: PhysicalSize::new(400, 300), scale_factor: 1.0 },
    );
    app.pump_frames(4);
    app
}

/// What a renderer would hand back: pixel `(x, y)` is red `x`, green `y`.
fn window_frame() -> Frame {
    let mut pixels = Vec::new();
    for y in 0..300u32 {
        for x in 0..400u32 {
            pixels.extend_from_slice(&[x as u8, y as u8, 0, 255]);
        }
    }
    Frame { width: 400, height: 300, scale: 1.0, bgra: false, pixels }
}

#[test]
fn before_anything_is_drawn_there_is_nowhere_to_take_a_picture_from() {
    let handle = CaptureHandle::new();
    assert!(handle.bounds().is_none());
    assert_eq!(handle.request().err(), Some(ShotError::NotShown));
}

#[test]
fn the_handle_reports_exactly_where_the_child_is_and_the_wrapper_adds_nothing() {
    let handle = CaptureHandle::new();
    let _app = page(&handle);
    let region = handle.bounds().expect("drawn, so it has a place");
    assert_eq!((region.x, region.y, region.width, region.height), (30.0, 20.0, 100.0, 60.0));
}

#[test]
fn the_picture_is_that_part_of_the_frame() {
    let handle = CaptureHandle::new();
    let _app = page(&handle);
    let (tx, rx) = mpsc::channel();
    let request = handle.request_from(rx).expect("it is on screen");
    tx.send(Ok(Arc::new(window_frame()))).unwrap();
    let image = request.wait(Duration::from_secs(1)).expect("a picture");
    assert_eq!((image.width(), image.height()), (100, 60));
    let at = |x: u32, y: u32| -> [u8; 4] { image.rgba()[((y * 100 + x) * 4) as usize..][..4].try_into().unwrap() };
    assert_eq!(at(0, 0), [30, 20, 0, 255], "its top-left corner is the window's (30, 20)");
    assert_eq!(at(99, 59), [129, 79, 0, 255], "and its bottom-right the window's (129, 79)");
}
