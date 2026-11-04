//! The web: one `HTMLVideoElement` per player.
//!
//! The browser fetches, streams, decodes and plays the sound in sync, so every
//! source becomes a URL and the element is prepared in place, with no helper
//! thread (there are none): a bundled asset is served from the site root, like
//! bundled images; a URL is used as it is; a host file cannot be reached and
//! fails with [`VideoError::Unsupported`].
//!
//! The element is never attached to the page. A frame names it by the id it is
//! registered under ([`aimer_cupid_gpu::web_video`]), and the renderer's
//! backend copies the element's current picture into a texture on the GPU
//! (see `NativeTexture::WebVideo`). A browser exposes no video texture to
//! share, so that copy is as close to zero-copy as the web gets: no pixel
//! reaches the CPU.
//!
//! Notes that follow from the browser:
//!
//! - Browsers refuse to start playback before the user has interacted with
//!   the page. The refusal arrives asynchronously and fails the player with
//!   [`VideoError::Playback`], so start playback from an input handler.
//! - Reading a video's pixels into a texture needs the page to be allowed to
//!   read them. Elements are created with `crossorigin="anonymous"`, so a
//!   cross-origin URL must be served with CORS headers, or it plays no
//!   picture.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use aimer_cupid::external_texture::NativeTexture;
use aimer_cupid_gpu::web_video::{self, WebVideoRegistration};
use js_sys::{Function, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlVideoElement;

use crate::backend::{FrameSlot, PlatformVideo, StoredFrame};
use crate::{VideoError, VideoSource};

/// `HAVE_METADATA`: the length and size are known.
const HAVE_METADATA: u16 = 1;
/// `HAVE_CURRENT_DATA`: the picture at the playhead is available.
const HAVE_CURRENT_DATA: u16 = 2;
/// `HAVE_FUTURE_DATA`: enough is buffered to play on from the playhead.
const HAVE_FUTURE_DATA: u16 = 3;
/// `MEDIA_ERR_NETWORK`.
const MEDIA_ERR_NETWORK: u16 = 2;
/// The last buffered range ending this close to the end counts as complete.
const COMPLETE_SLACK: f64 = 0.25;

/// Opens `source` as a paused player.
pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn PlatformVideo>, VideoError> {
    let url = match source {
        VideoSource::Bundled(key) => asset_url(key),
        VideoSource::Url(url) => url.clone(),
        VideoSource::File(_) => return Err(VideoError::Unsupported),
    };
    WebVideo::new(&url).map(|video| Box::new(video) as Box<dyn PlatformVideo>)
}

/// The URL a bundled asset is served from: the site root.
fn asset_url(key: &str) -> String {
    if key.starts_with('/') {
        key.to_string()
    } else {
        format!("/{key}")
    }
}

fn describe(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| Reflect::get(value, &"message".into()).ok()?.as_string())
        .unwrap_or_else(|| "the browser refused to play".into())
}

/// One `<video>` element and what tracks its pictures.
///
/// The browser has a single thread, so the `Send`/`Sync` the player handle
/// asks of its backends are vacuous: see the `unsafe impl`s.
struct WebVideo {
    element: Rc<HtmlVideoElement>,
    /// Keeps the element findable by the renderer's backend; frames share it,
    /// so the id stays valid for as long as a frame naming it can be drawn.
    registration: Arc<WebVideoRegistration>,
    /// Pictures the browser has presented, counted by
    /// `requestVideoFrameCallback`; `None` where the browser lacks it.
    pictures: Option<Rc<Cell<u64>>>,
    /// Ends the callback chain when the player is dropped.
    stopped: Rc<Cell<bool>>,
    /// A rejected `play()`, reported by its promise after the call returned.
    refused: Rc<RefCell<Option<String>>>,
    /// Whether the player last asked to play: `paused` flips back
    /// asynchronously when a start is refused, which is not the end.
    intent: Cell<bool>,
    /// What the last stored frame showed: the picture count, or the playhead
    /// where pictures are not counted.
    shown: Cell<Option<u64>>,
    /// Numbers the frames, so the backend copies each picture once.
    serial: Cell<u64>,
}

// SAFETY: a DOM element cannot leave the thread it was made on, and the web
// has exactly one thread: nothing here is ever touched from another.
unsafe impl Send for WebVideo {}
unsafe impl Sync for WebVideo {}

impl WebVideo {
    fn new(url: &str) -> Result<Self, VideoError> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or(VideoError::Unsupported)?;
        let element: HtmlVideoElement = document
            .create_element("video")
            .map_err(|error| VideoError::Playback(describe(&error)))?
            .dyn_into()
            .map_err(|_| VideoError::Unsupported)?;
        // Without these a cross-origin picture cannot be read into a texture,
        // and iOS Safari would take the video fullscreen.
        element.set_cross_origin(Some("anonymous"));
        element.set_attribute("playsinline", "").ok();
        element.set_preload("auto");
        element.set_src(url);
        let element = Rc::new(element);

        let stopped = Rc::new(Cell::new(false));
        let pictures = supports_frame_callback(&element).then(|| {
            let pictures = Rc::new(Cell::new(0));
            count_pictures(element.clone(), pictures.clone(), stopped.clone());
            pictures
        });
        let registration = Arc::new(web_video::register(element.clone()));
        Ok(Self {
            element,
            registration,
            pictures,
            stopped,
            refused: Rc::new(RefCell::new(None)),
            intent: Cell::new(false),
            shown: Cell::new(None),
            serial: Cell::new(0),
        })
    }
}

/// Whether the browser has `requestVideoFrameCallback`.
fn supports_frame_callback(element: &HtmlVideoElement) -> bool {
    Reflect::has(element, &"requestVideoFrameCallback".into()).unwrap_or(false)
}

/// Counts the pictures `element` presents, until `stopped`.
///
/// Each callback is single use and registers the next, so no closure outlives
/// its call.
fn count_pictures(element: Rc<HtmlVideoElement>, pictures: Rc<Cell<u64>>, stopped: Rc<Cell<bool>>) {
    let next = element.clone();
    let callback = Closure::once_into_js(move |_now: JsValue, _metadata: JsValue| {
        if stopped.get() {
            return;
        }
        pictures.set(pictures.get() + 1);
        count_pictures(next, pictures, stopped);
    });
    let request = Reflect::get(&element, &"requestVideoFrameCallback".into())
        .ok()
        .and_then(|request| request.dyn_into::<Function>().ok());
    if let Some(request) = request {
        let _ = request.call1(&element, &callback);
    }
}

impl Drop for WebVideo {
    fn drop(&mut self) {
        self.stopped.set(true);
        let _ = self.element.pause();
        // Detaching the source stops the download.
        self.element.remove_attribute("src").ok();
        self.element.load();
    }
}

impl PlatformVideo for WebVideo {
    fn play(&self) {
        self.intent.set(true);
        *self.refused.borrow_mut() = None;
        let Ok(promise) = self.element.play() else {
            return;
        };
        let refused = self.refused.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = JsFuture::from(promise).await {
                *refused.borrow_mut() = Some(describe(&error));
            }
        });
    }

    fn pause(&self) {
        self.intent.set(false);
        let _ = self.element.pause();
    }

    fn seek(&self, seconds: f64) {
        self.element.set_current_time(seconds.max(0.0));
    }

    fn set_volume(&self, volume: f32) {
        self.element.set_volume(f64::from(volume.clamp(0.0, 1.0)));
    }

    fn is_ready(&self) -> bool {
        self.element.ready_state() >= HAVE_METADATA
    }

    fn position(&self) -> f64 {
        self.element.current_time().max(0.0)
    }

    fn duration(&self) -> f64 {
        let duration = self.element.duration();
        // `NaN` until the metadata arrives; `Infinity` for a live stream.
        if duration.is_finite() { duration } else { f64::NAN }
    }

    fn buffered(&self) -> f64 {
        let position = self.position();
        let ranges = self.element.buffered();
        let mut end = position;
        for index in 0..ranges.length() {
            let (Ok(start), Ok(stop)) = (ranges.start(index), ranges.end(index)) else {
                continue;
            };
            if start <= position + COMPLETE_SLACK && stop > end {
                end = stop;
            }
        }
        let duration = self.duration();
        if duration.is_finite() && end >= duration - COMPLETE_SLACK {
            return duration;
        }
        end
    }

    fn is_playing(&self) -> bool {
        self.intent.get() && !self.element.ended()
    }

    fn is_buffering(&self) -> bool {
        self.intent.get() && !self.element.ended() && self.element.ready_state() < HAVE_FUTURE_DATA
    }

    fn error(&self) -> Option<VideoError> {
        if let Some(why) = self.refused.borrow().clone() {
            return Some(VideoError::Playback(why));
        }
        let error = self.element.error()?;
        let message = error.message();
        let message = if message.is_empty() { format!("media error {}", error.code()) } else { message };
        Some(if error.code() == MEDIA_ERR_NETWORK {
            VideoError::Network(message)
        } else {
            VideoError::Decode(message)
        })
    }

    fn size(&self) -> Option<(u32, u32)> {
        let (width, height) = (self.element.video_width(), self.element.video_height());
        (width > 0 && height > 0).then_some((width, height))
    }

    fn take_frame(&self, frames: &FrameSlot) -> bool {
        let Some((width, height)) = self.size() else {
            return false;
        };
        // No picture at the playhead yet (loading, or in the middle of a seek).
        if self.element.ready_state() < HAVE_CURRENT_DATA {
            return false;
        }
        // Where pictures are counted a new one is a new count; elsewhere the
        // playhead stands in, which also moves for every picture of a video
        // that plays.
        let token = match &self.pictures {
            Some(pictures) => pictures.get(),
            None => self.element.current_time().to_bits(),
        };
        if self.shown.get() == Some(token) && frames.has_frame() {
            return false;
        }
        self.shown.set(Some(token));
        let serial = self.serial.get() + 1;
        self.serial.set(serial);
        let texture = NativeTexture::WebVideo {
            id: self.registration.id(),
            serial,
        };
        // SAFETY: the registration, which the frame owns, keeps the id naming
        // the element for as long as the frame lives; the backend copies the
        // picture when it draws, and a width x height picture is what the
        // element reports.
        let frame = unsafe { StoredFrame::new(texture, width, height, self.registration.clone()) };
        frames.store(Some(frame));
        true
    }
}
