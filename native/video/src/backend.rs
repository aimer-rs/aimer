//! The seam between the platform-neutral player and a platform's decoder.

use std::any::Any;
use std::sync::{Arc, Mutex, PoisonError};

use aimer_cupid::external_texture::{ExternalTextureSource, NativeFrame, NativeTexture};

use crate::VideoError;

/// A platform video player. Implementations synchronise internally: the
/// handle calls them from the UI thread and any thread its clones live on.
pub(crate) trait PlatformVideo: Send + Sync {
    fn play(&self);
    fn pause(&self);
    /// Moves the playhead; `seconds` is finite and not negative.
    fn seek(&self, seconds: f64);
    /// Linear gain in `0.0..=1.0`, already muted when muted.
    fn set_volume(&self, volume: f32);
    /// Whether the source has opened far enough to know its length or size.
    fn is_ready(&self) -> bool;
    /// Playhead in seconds.
    fn position(&self) -> f64;
    /// Length in seconds, or `NaN` while unknown (and for live streams).
    fn duration(&self) -> f64;
    /// Seconds from the start that can play without waiting.
    fn buffered(&self) -> f64;
    /// The requested rate is not zero: playing, or stalled while asked to play.
    fn is_playing(&self) -> bool;
    /// Asked to play, but waiting for data.
    fn is_buffering(&self) -> bool;
    fn error(&self) -> Option<VideoError>;
    /// The picture size in pixels, once known.
    fn size(&self) -> Option<(u32, u32)>;
    /// Stores the frame due now in `frames` when it differs from the stored
    /// one. Returns whether it stored a new frame.
    fn take_frame(&self, frames: &FrameSlot) -> bool;
}

/// The newest decoded frame, shared with the renderer.
///
/// The UI thread stores frames ([`PlatformVideo::take_frame`]); the renderer
/// reads the current one while it encodes, possibly on the raster thread.
#[derive(Default)]
pub(crate) struct FrameSlot {
    frame: Mutex<Option<StoredFrame>>,
}

/// A frame as the slot keeps it; cheap to hand out again.
#[derive(Clone)]
pub(crate) struct StoredFrame {
    texture: NativeTexture,
    width: u32,
    height: u32,
    owner: Arc<dyn Any + Send + Sync>,
}

// SAFETY: `texture` is a handle to a GPU texture object that any thread may
// use, and `StoredFrame::new`'s contract keeps it valid while `owner` lives.
unsafe impl Send for StoredFrame {}
unsafe impl Sync for StoredFrame {}

impl StoredFrame {
    /// Only the platform backends (and tests) produce frames.
    #[cfg_attr(not(any(target_os = "ios", target_os = "macos", target_arch = "wasm32", target_os = "windows", test)), allow(dead_code))]
    /// # Safety
    ///
    /// `texture` must be a valid `width` x `height` texture whose contents stay
    /// unchanged for as long as `owner` lives.
    pub(crate) unsafe fn new(
        texture: NativeTexture,
        width: u32,
        height: u32,
        owner: Arc<dyn Any + Send + Sync>,
    ) -> Self {
        Self {
            texture,
            width,
            height,
            owner,
        }
    }
}

impl FrameSlot {
    pub(crate) fn store(&self, frame: Option<StoredFrame>) {
        *self.frame.lock().unwrap_or_else(PoisonError::into_inner) = frame;
    }

    pub(crate) fn has_frame(&self) -> bool {
        self.frame.lock().unwrap_or_else(PoisonError::into_inner).is_some()
    }
}

impl ExternalTextureSource for FrameSlot {
    fn current_frame(&self) -> Option<NativeFrame> {
        let stored = self.frame.lock().unwrap_or_else(PoisonError::into_inner).clone()?;
        // SAFETY: `StoredFrame::new`'s contract is `NativeFrame::new`'s.
        Some(unsafe { NativeFrame::new(stored.texture, stored.width, stored.height, stored.owner) })
    }
}
