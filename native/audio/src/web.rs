//! The web: one `HTMLAudioElement` per voice.
//!
//! The browser fetches, buffers, streams and decodes, so every source becomes
//! a URL and is prepared in place, with no helper thread (there are none):
//! a bundled asset is served from the site root, like bundled images; a URL
//! is used as it is, so `AudioSource::url` streams here too; a host file
//! cannot be reached and fails with [`AudioError::Unsupported`].
//!
//! Browsers refuse to start audio before the user has interacted with the
//! page. Such a refusal arrives asynchronously and fails the track with
//! [`AudioError::Playback`], so start playback from an input handler.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlAudioElement;

use crate::backend::{Backend, Voice, VoiceData};
use crate::{AudioError, AudioSource};

/// `HAVE_FUTURE_DATA`: enough is buffered to play on from the playhead.
const HAVE_FUTURE_DATA: u16 = 3;
/// `MEDIA_ERR_NETWORK`.
const MEDIA_ERR_NETWORK: u16 = 2;
/// The last buffered range ending this close to the end counts as complete.
const COMPLETE_SLACK: f64 = 0.25;

pub(crate) struct WebBackend;

/// The URL a bundled asset is served from: the site root.
fn asset_url(key: &str) -> String {
    if key.starts_with('/') {
        key.to_string()
    } else {
        format!("/{key}")
    }
}

impl Backend for WebBackend {
    fn new() -> Self {
        Self
    }

    fn open(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError> {
        match data {
            VoiceData::Stream(url) => WebVoice::new(url).map(|voice| Box::new(voice) as Box<dyn Voice>),
            VoiceData::Asset(key) => WebVoice::new(&asset_url(key)).map(|voice| Box::new(voice) as Box<dyn Voice>),
            // `prepare_now` turns every source into a URL first.
            VoiceData::Path(_) | VoiceData::Bytes(_) => Err(AudioError::Unsupported),
        }
    }

    fn fetch(_url: &str) -> Result<Vec<u8>, AudioError> {
        // Never called: `prepare_now` handles every source.
        Err(AudioError::Unsupported)
    }

    fn prepare_now(source: &AudioSource, _want_bytes: bool) -> Option<Result<VoiceData, AudioError>> {
        Some(match source {
            AudioSource::Bundled(key) => Ok(VoiceData::Stream(asset_url(key))),
            AudioSource::Url(url) | AudioSource::Stream(url) => Ok(VoiceData::Stream(url.clone())),
            AudioSource::File(_) => Err(AudioError::Unsupported),
        })
    }
}

fn describe(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| js_sys::Reflect::get(value, &"message".into()).ok()?.as_string())
        .unwrap_or_else(|| "the browser refused to play".into())
}

struct WebVoice {
    element: HtmlAudioElement,
    /// A rejected `play()`, reported by its promise after the call returned.
    refused: Rc<RefCell<Option<String>>>,
    /// Whether the engine last asked this voice to play: `paused` flips back
    /// asynchronously when a start is refused, which is not the end.
    intent: bool,
}

impl WebVoice {
    fn new(url: &str) -> Result<Self, AudioError> {
        let element = HtmlAudioElement::new_with_src(url)
            .map_err(|error| AudioError::Playback(describe(&error)))?;
        // Start fetching at once, so the first `play` has less to wait for.
        element.set_preload("auto");
        Ok(Self {
            element,
            refused: Rc::new(RefCell::new(None)),
            intent: false,
        })
    }
}

impl Drop for WebVoice {
    fn drop(&mut self) {
        let _ = self.element.pause();
        // Detaching the source stops the download.
        self.element.remove_attribute("src").ok();
        self.element.load();
    }
}

impl Voice for WebVoice {
    fn duration(&self) -> f64 {
        let duration = self.element.duration();
        // `NaN` until the metadata arrives; `Infinity` for a live stream.
        if duration.is_finite() { duration } else { f64::NAN }
    }

    fn position(&self) -> f64 {
        self.element.current_time().max(0.0)
    }

    fn is_playing(&self) -> bool {
        self.intent && !self.element.ended()
    }

    fn play(&mut self) -> Result<(), AudioError> {
        self.intent = true;
        *self.refused.borrow_mut() = None;
        let promise = self
            .element
            .play()
            .map_err(|error| AudioError::Playback(describe(&error)))?;
        let refused = self.refused.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = JsFuture::from(promise).await {
                *refused.borrow_mut() = Some(describe(&error));
            }
        });
        Ok(())
    }

    fn pause(&mut self) {
        self.intent = false;
        let _ = self.element.pause();
    }

    fn stop(&mut self) {
        self.pause();
        self.element.set_current_time(0.0);
    }

    fn seek(&mut self, seconds: f64) {
        self.element.set_current_time(seconds.max(0.0));
    }

    fn set_volume(&mut self, volume: f32) {
        self.element.set_volume(f64::from(volume));
    }

    fn set_extra_loops(&mut self, extra: i64) {
        self.element.set_loop(extra < 0);
    }

    fn is_buffering(&self) -> bool {
        self.intent && !self.element.ended() && self.element.ready_state() < HAVE_FUTURE_DATA
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

    fn error(&self) -> Option<AudioError> {
        if let Some(why) = self.refused.borrow().clone() {
            return Some(AudioError::Playback(why));
        }
        let error = self.element.error()?;
        let message = error.message();
        let message = if message.is_empty() { format!("media error {}", error.code()) } else { message };
        Some(if error.code() == MEDIA_ERR_NETWORK {
            AudioError::Network(message)
        } else {
            AudioError::Decode(message)
        })
    }
}
