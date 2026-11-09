//! A request for a picture of the window, and how the renderer answers it.
//!
//! Anything may ask, from any thread — a debugging server, a test, a widget —
//! and the answer comes from the one place that has the pixels: the renderer,
//! which copies its finished frame out in order, between drawing it and
//! presenting it. The request does not touch the GPU, which is why it can be
//! made from a thread that owns no part of the frame.
//!
//! [`request_and_wake`] files a request and wakes the event loop, because an app
//! that is not changing draws nothing and so has no frame to copy. The renderer
//! checks [`is_pending`] — one atomic load — on every frame, and when it is set
//! takes the request with [`take_pending`] and answers it with
//! [`Pending::complete`].

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// A finished frame, copied out of the renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
    /// Physical pixels per logical pixel, so a point in the window's own
    /// coordinates can be found in the image.
    pub scale: f32,
    /// Whether each pixel is stored blue, green, red, alpha (Apple's surfaces
    /// are) rather than red, green, blue, alpha.
    pub bgra: bool,
    /// `width * height * 4` bytes, rows packed with no padding between them.
    pub pixels: Vec<u8>,
}

/// Why a frame could not be had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The renderer in use cannot copy its frames out.
    Unsupported,
    /// No event loop is running to draw a frame.
    NoWindow,
    /// No frame arrived in time: the window may be hidden or minimised, so
    /// nothing is being drawn.
    TimedOut,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "this renderer cannot capture its frames yet",
            Self::NoWindow => "the app has no window drawing yet",
            Self::TimedOut => "no frame was drawn in time: is the window visible?",
        })
    }
}

impl std::error::Error for Error {}

/// What the asker receives: a frame shared with everyone who asked at once.
pub type Reply = Result<Arc<Frame>, Error>;

static WAITING: Mutex<Vec<Sender<Reply>>> = Mutex::new(Vec::new());
static PENDING: AtomicBool = AtomicBool::new(false);

/// Files a request for the next frame, and returns where the answer will arrive.
///
/// This does not wake anything: use [`request_and_wake`] unless a frame is known
/// to be coming.
pub fn request() -> Receiver<Reply> {
    let (tx, rx) = mpsc::channel();
    WAITING.lock().expect("capture mutex poisoned").push(tx);
    PENDING.store(true, Ordering::Release);
    rx
}

/// Files a request and wakes the event loop to draw a frame to answer it.
///
/// With no event loop running there is no frame to wait for, and the answer is
/// [`Error::NoWindow`] at once.
pub fn request_and_wake() -> Receiver<Reply> {
    let reply = request();
    match crate::EVENT_PROXY.get() {
        Some(proxy) if proxy.send_event(crate::AimerNativePlatformEvent::CaptureRequested).is_ok() => {}
        _ => {
            if let Some(pending) = take_pending() {
                pending.fail(Error::NoWindow);
            }
        }
    }
    reply
}

/// Whether someone is waiting for a frame. One atomic load, cheap enough for
/// every frame.
#[inline]
pub fn is_pending() -> bool {
    PENDING.load(Ordering::Acquire)
}

/// The requests waiting for a frame, taken so that exactly one frame answers them.
pub struct Pending {
    senders: Vec<Sender<Reply>>,
}

/// Takes every waiting request, or `None` if nobody is waiting.
pub fn take_pending() -> Option<Pending> {
    let mut waiting = WAITING.lock().expect("capture mutex poisoned");
    if waiting.is_empty() {
        return None;
    }
    PENDING.store(false, Ordering::Release);
    Some(Pending { senders: std::mem::take(&mut *waiting) })
}

impl Pending {
    /// Answers everyone waiting with `frame`, shared between them.
    pub fn complete(self, frame: Frame) {
        let shared = Arc::new(frame);
        for sender in self.senders {
            // A requester that gave up has dropped its end: nothing to tell it.
            let _ = sender.send(Ok(Arc::clone(&shared)));
        }
    }

    /// Answers everyone waiting that there will be no frame, and why.
    pub fn fail(self, error: Error) {
        for sender in self.senders {
            let _ = sender.send(Err(error));
        }
    }
}

/// For a renderer that cannot copy its frames out: answers anyone waiting with
/// `error` at once, and does nothing if nobody is. One atomic load per frame.
pub fn refuse(error: Error) {
    if is_pending()
        && let Some(pending) = take_pending()
    {
        pending.fail(error);
    }
}

/// Packs rows that the GPU padded out to `bytes_per_row` into `width * 4` bytes
/// each. Rows already that wide are returned as they are.
pub fn unpad_rows(bytes: &[u8], width: u32, height: u32, bytes_per_row: u32) -> Vec<u8> {
    let tight = width as usize * 4;
    let stride = bytes_per_row as usize;
    if stride == tight {
        return bytes[..tight * height as usize].to_vec();
    }
    let mut out = Vec::with_capacity(tight * height as usize);
    for row in 0..height as usize {
        out.extend_from_slice(&bytes[row * stride..row * stride + tight]);
    }
    out
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
