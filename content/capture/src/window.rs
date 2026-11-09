//! A picture of the whole window.

use std::time::Duration;

use aimer_platform::capture;

use crate::Image;

/// How long to wait for a frame when the caller has no opinion: long enough for a
/// busy app to draw one, short enough that a hidden window is reported before a
/// client gives up.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

/// A picture of the app's window as it is drawn, in full, whatever has or has not
/// changed since the last frame.
///
/// The renderer copies a finished frame out in order, so the picture is what was
/// really presented — nothing else on the screen, and no permission asked of the
/// operating system. An idle app draws no frames, so this wakes it to draw one.
/// The picture is in physical pixels; [`Image::scale`] says how many make a
/// logical pixel.
///
/// Call it from any thread but the one drawing: it waits for the frame, and so
/// would wait for itself. Nothing here encodes the picture — do that where you
/// can afford it, which is not the UI thread.
///
/// # Errors
///
/// [`capture::Error::NoWindow`] if no event loop is running,
/// [`capture::Error::TimedOut`] if no frame was drawn in `timeout` — the window
/// may be hidden or minimised — and [`capture::Error::Unsupported`] if the
/// renderer in use cannot copy its frames out yet.
///
/// # Examples
///
/// ```no_run
/// use aimer_capture::{Format, screenshot, DEFAULT_TIMEOUT};
///
/// // From a worker thread of a running app:
/// let picture = screenshot(DEFAULT_TIMEOUT)?;
/// let png = picture.scaled_to_width(1280).encode(Format::Png)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn screenshot(timeout: Duration) -> Result<Image, capture::Error> {
    let reply = capture::request_and_wake();
    let frame = reply.recv_timeout(timeout).map_err(|_| capture::Error::TimedOut)??;
    Ok(Image::from_frame(&frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_window_drawing_it_says_so_at_once() {
        let started = std::time::Instant::now();
        // No event loop runs in a unit test.
        assert_eq!(screenshot(Duration::from_secs(5)).unwrap_err(), capture::Error::NoWindow);
        assert!(started.elapsed() < Duration::from_secs(1), "it did not wait for the timeout");
    }

    #[test]
    fn a_frame_nobody_draws_times_out() {
        // Filed without waking anything: no renderer will ever answer.
        let _unanswered = capture::request();
        let started = std::time::Instant::now();
        let reply = capture::request();
        assert!(reply.recv_timeout(Duration::from_millis(50)).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        capture::take_pending();
    }
}
