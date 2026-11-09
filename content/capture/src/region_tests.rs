use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use aimer_platform::capture::{Error as WindowError, Frame};

use super::*;

/// A frame whose pixel `(x, y)` is `[x, y, 9, 255]` (in red, green, blue, alpha
/// order), at 2 pixels to the logical pixel.
fn frame() -> Frame {
    let mut pixels = Vec::new();
    for y in 0..16u32 {
        for x in 0..20u32 {
            pixels.extend_from_slice(&[9, y as u8, x as u8, 255]); // blue, green, red, alpha
        }
    }
    Frame { width: 20, height: 16, scale: 2.0, bgra: true, pixels }
}

fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * image.width() + x) * 4) as usize;
    image.rgba()[at..at + 4].try_into().unwrap()
}

fn answered(reply: capture::Reply) -> CaptureRequest {
    let (tx, rx) = mpsc::channel();
    tx.send(reply).unwrap();
    CaptureRequest::new(Region { x: 2.0, y: 1.0, width: 3.0, height: 2.0 }, rx)
}

#[test]
fn the_picture_is_the_part_of_the_window_where_the_content_was() {
    let image = answered(Ok(Arc::new(frame()))).wait(Duration::from_secs(1)).expect("a picture");
    // Logical (2, 1) 3 by 2 at 2x is pixels (4, 2), 6 by 4.
    assert_eq!((image.width(), image.height()), (6, 4));
    assert_eq!(pixel(&image, 0, 0), [4, 2, 9, 255], "in red, green, blue, alpha order");
    assert_eq!(pixel(&image, 5, 3), [9, 5, 9, 255]);
    assert_eq!(image.scale(), 2.0, "still a 2x picture");
}

#[test]
fn content_hanging_over_the_edge_of_the_window_is_trimmed_to_what_is_in_it() {
    let (tx, rx) = mpsc::channel();
    tx.send(Ok(Arc::new(frame()))).unwrap();
    // The window is 10 by 8 logical; this reaches 4 past the right and bottom edges.
    let request = CaptureRequest::new(Region { x: 8.0, y: 6.0, width: 6.0, height: 6.0 }, rx);
    let image = request.wait(Duration::from_secs(1)).expect("what is inside");
    assert_eq!((image.width(), image.height()), (4, 4), "2 logical pixels left, at 2x");
}

#[test]
fn content_wholly_outside_the_window_is_reported_not_returned_as_nothing() {
    let (tx, rx) = mpsc::channel();
    tx.send(Ok(Arc::new(frame()))).unwrap();
    let request = CaptureRequest::new(Region { x: 50.0, y: 50.0, width: 5.0, height: 5.0 }, rx);
    assert_eq!(request.wait(Duration::from_secs(1)).unwrap_err(), ShotError::OffScreen);
}

#[test]
fn a_window_that_could_not_be_captured_says_why() {
    let err = answered(Err(WindowError::Unsupported)).wait(Duration::from_secs(1)).unwrap_err();
    assert_eq!(err, ShotError::Window(WindowError::Unsupported));
    assert!(err.to_string().contains("cannot capture"), "{err}");
}

#[test]
fn no_frame_in_time_is_a_timeout() {
    let (_keep, rx) = mpsc::channel();
    let request = CaptureRequest::new(Region { x: 0.0, y: 0.0, width: 1.0, height: 1.0 }, rx);
    assert_eq!(request.wait(Duration::from_millis(30)).unwrap_err(), ShotError::Window(WindowError::TimedOut));
}

#[test]
fn polling_does_not_block_and_gives_the_answer_once_it_is_there() {
    let (tx, rx) = mpsc::channel();
    let mut request = CaptureRequest::new(Region { x: 2.0, y: 1.0, width: 3.0, height: 2.0 }, rx);
    assert!(request.poll().is_none(), "no frame yet");
    tx.send(Ok(Arc::new(frame()))).unwrap();
    let got = request.poll().expect("now it has arrived").expect("a picture");
    assert_eq!(got.width(), 6);
    assert!(request.poll().is_none(), "and it is given once");
}

#[test]
fn a_region_with_no_area_is_refused_before_anything_is_asked_of_the_renderer() {
    for (w, h) in [(0.0, 5.0), (5.0, 0.0), (-1.0, 5.0), (f32::NAN, 5.0)] {
        assert_eq!(Region::new(1.0, 1.0, w, h), None, "{w} x {h}");
    }
    assert!(Region::new(1.0, 1.0, 5.0, 5.0).is_some());
    assert_eq!(Region::new(f32::INFINITY, 0.0, 5.0, 5.0), None);
}
