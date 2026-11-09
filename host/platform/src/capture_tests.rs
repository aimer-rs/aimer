use std::time::Duration;

use super::*;

/// The slot is process-wide and `cargo test` runs tests in one process.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn frame(width: u32, height: u32) -> Frame {
    Frame { width, height, scale: 2.0, bgra: true, pixels: vec![7; (width * height * 4) as usize] }
}

fn drain() {
    if let Some(pending) = take_pending() {
        pending.fail(Error::Unsupported);
    }
}

#[test]
fn nothing_is_pending_until_someone_asks() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    drain();
    assert!(!is_pending());
    assert!(take_pending().is_none());
}

#[test]
fn a_request_is_pending_and_the_frame_that_fulfils_it_reaches_the_asker() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    drain();
    let reply = request();
    assert!(is_pending(), "the renderer is told to look");
    let pending = take_pending().expect("the renderer takes it");
    assert!(!is_pending(), "and it is taken once");
    pending.complete(frame(4, 3));
    let got = reply.recv_timeout(Duration::from_secs(1)).expect("an answer").expect("a frame");
    assert_eq!((got.width, got.height, got.scale, got.bgra), (4, 3, 2.0, true));
    assert_eq!(got.pixels.len(), 4 * 3 * 4);
}

#[test]
fn everyone_waiting_gets_the_same_frame_without_a_second_render() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    drain();
    let (a, b) = (request(), request());
    take_pending().expect("pending").complete(frame(2, 2));
    let (a, b) = (a.recv().unwrap().unwrap(), b.recv().unwrap().unwrap());
    assert!(std::sync::Arc::ptr_eq(&a, &b), "one frame, shared");
}

#[test]
fn a_renderer_that_cannot_capture_says_so_instead_of_leaving_the_asker_waiting() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    drain();
    let reply = request();
    take_pending().expect("pending").fail(Error::Unsupported);
    assert_eq!(reply.recv().unwrap().unwrap_err(), Error::Unsupported);
}

#[test]
fn an_asker_that_gave_up_does_not_break_the_renderer() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    drain();
    drop(request());
    take_pending().expect("pending").complete(frame(1, 1));
}

#[test]
fn rows_the_gpu_padded_are_packed_tight() {
    // 3 pixels wide, rows padded to 16 bytes, 2 rows: the padding is dropped.
    let mut padded = vec![0u8; 16 * 2];
    for (i, b) in padded.iter_mut().enumerate() {
        *b = if i % 16 < 12 { (i % 16) as u8 + 1 } else { 99 };
    }
    let tight = unpad_rows(&padded, 3, 2, 16);
    assert_eq!(tight.len(), 3 * 4 * 2);
    assert!(!tight.contains(&99), "no padding left");
    assert_eq!(&tight[..12], &(1..=12).collect::<Vec<u8>>()[..]);
    assert_eq!(&tight[12..], &(1..=12).collect::<Vec<u8>>()[..], "the second row starts where its own row does");
}

#[test]
fn already_tight_rows_are_returned_as_they_are() {
    let tight: Vec<u8> = (0..24).collect();
    assert_eq!(unpad_rows(&tight, 3, 2, 12), tight);
}

#[test]
fn a_renderer_that_cannot_capture_refuses_at_once_and_only_when_someone_is_waiting() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    drain();
    refuse(Error::Unsupported); // nobody asked: nothing happens
    assert!(!is_pending());
    let reply = request();
    refuse(Error::Unsupported);
    assert_eq!(reply.recv_timeout(Duration::from_secs(1)).unwrap().unwrap_err(), Error::Unsupported);
    assert!(!is_pending(), "and the request is spent");
}
