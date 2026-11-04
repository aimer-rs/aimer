//! macOS and iOS: `AVPlayer`, with frames taken as Metal textures.
//!
//! `AVPlayer` opens, streams (progressive http(s) and HLS), decodes and keeps
//! the soundtrack in sync on its own threads. An `AVPlayerItemVideoOutput`
//! hands out the decoded frames as IOSurface-backed `CVPixelBuffer`s in BGRA,
//! the hardware decoder having converted from YUV, and a
//! `CVMetalTextureCache` wraps each one as an `MTLTexture` without copying it.
//! The renderer then samples that texture directly.
//!
//! Every object here is used through `&self` from the threads the player
//! handle lives on; see the safety notes on [`AppleVideo`].

use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{Arc, OnceLock};

use aimer_cupid::external_texture::NativeTexture;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_av_foundation::{
    AVPlayer, AVPlayerItem, AVPlayerItemOutput, AVPlayerItemStatus, AVPlayerItemVideoOutput,
    AVPlayerStatus, AVPlayerTimeControlStatus, NSValueAVFoundationExtensions,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::CMTime;
use objc2_core_video::{
    CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, kCVPixelBufferMetalCompatibilityKey,
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLPixelFormat, MTLTexture};
use objc2_quartz_core::CACurrentMediaTime;

use crate::backend::{FrameSlot, PlatformVideo, StoredFrame};
use crate::{VideoError, VideoSource};

/// `CMTime` timescale for seeks: one millisecond.
const TIMESCALE: i32 = 1000;
/// A download this close to the end counts as complete.
const COMPLETE_SLACK: f64 = 0.25;

/// Opens `source` as a paused player.
pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn PlatformVideo>, VideoError> {
    #[cfg(target_os = "ios")]
    configure_session();
    let url = source_url(source)?;
    let cache = texture_cache()?;
    // SAFETY: objc2 marks `AVPlayer` main-thread-only, but AVFoundation
    // documents its playback controls and state as usable from any thread;
    // only layers and KVO callbacks have affinity, and none is used here. The
    // marker is used for allocation only. The same holds for the audio crate.
    let player = unsafe {
        let mtm = MainThreadMarker::new_unchecked();
        AVPlayer::initWithURL(AVPlayer::alloc(mtm), &url)
    };
    // SAFETY: `player` is a valid, freshly initialised player.
    let item = unsafe { player.currentItem() }
        .ok_or_else(|| VideoError::Playback("AVPlayer has no item".into()))?;
    let output = video_output();
    // SAFETY: `item` and `output` are valid; an item accepts outputs until it
    // is released.
    unsafe { item.addOutput(&output) };
    Ok(Box::new(AppleVideo {
        player,
        item,
        output,
        cache,
    }))
}

/// The URL `AVPlayer` opens for `source`.
fn source_url(source: &VideoSource) -> Result<Retained<NSURL>, VideoError> {
    let file = |path: &Path| {
        if !path.is_file() {
            return Err(VideoError::NotFound(path.display().to_string()));
        }
        Ok(NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())))
    };
    match source {
        VideoSource::File(path) => file(path),
        VideoSource::Bundled(key) => {
            let exe = std::env::current_exe().ok();
            let path = bundled_candidates(key, exe.as_deref())
                .into_iter()
                .find(|candidate| candidate.is_file())
                .ok_or_else(|| VideoError::NotFound(key.clone()))?;
            file(&path)
        }
        VideoSource::Url(url) => {
            let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
            if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                return Err(VideoError::Network(format!("not an http(s) URL: {url}")));
            }
            NSURL::URLWithString(&NSString::from_str(url))
                .ok_or_else(|| VideoError::Network(format!("invalid URL: {url}")))
        }
    }
}

/// Where a bundled asset may live, in lookup order: the working directory
/// (development runs), the macOS bundle's `Resources`, then next to the
/// executable (iOS). The same lookup `aimer_assets` and `aimer_audio` use.
fn bundled_candidates(key: &str, exe: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(key)];
    if let Some(dir) = exe.and_then(Path::parent) {
        if let Some(contents) = dir.parent() {
            candidates.push(contents.join("Resources").join(key));
        }
        candidates.push(dir.join(key));
    }
    candidates
}

/// An output of Metal-compatible, IOSurface-backed BGRA pixel buffers.
fn video_output() -> Retained<AVPlayerItemVideoOutput> {
    // SAFETY: both keys are valid, immutable `CFString` constants, which are
    // toll-free bridged to `NSString`.
    let (format_key, metal_key): (&NSString, &NSString) = unsafe {
        (
            &*(std::ptr::from_ref(kCVPixelBufferPixelFormatTypeKey).cast::<NSString>()),
            &*(std::ptr::from_ref(kCVPixelBufferMetalCompatibilityKey).cast::<NSString>()),
        )
    };
    let format: Retained<AnyObject> = NSNumber::new_u32(kCVPixelFormatType_32BGRA).into();
    let metal: Retained<AnyObject> = NSNumber::new_bool(true).into();
    let attributes = NSDictionary::from_retained_objects(&[format_key, metal_key], &[format, metal]);
    // SAFETY: the dictionary holds only valid pixel-buffer attribute keys
    // with values of their documented types.
    unsafe {
        AVPlayerItemVideoOutput::initWithPixelBufferAttributes(
            AVPlayerItemVideoOutput::alloc(),
            Some(&attributes),
        )
    }
}

/// The process's Metal device and the cache that wraps pixel buffers on it.
///
/// It must be the device the renderer draws with: the Metal backend uses the
/// system default device too.
fn texture_cache() -> Result<SharedCache, VideoError> {
    static CACHE: OnceLock<Option<SharedCache>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let device = MTLCreateSystemDefaultDevice()?;
            let mut cache: *mut CVMetalTextureCache = std::ptr::null_mut();
            // SAFETY: `cache` receives a +1 reference on success; the device
            // is valid for the call.
            let status = unsafe {
                CVMetalTextureCache::create(None, None, &device, None, NonNull::from(&mut cache))
            };
            if status != kCVReturnSuccess {
                return None;
            }
            // SAFETY: on success `cache` is a valid, owned, +1 reference.
            let cache = unsafe { CFRetained::from_raw(NonNull::new(cache)?) };
            Some(SharedCache(cache))
        })
        .clone()
        .ok_or_else(|| VideoError::Playback("Metal is not available".into()))
}

/// The shared texture cache. It holds its own reference to the device that
/// created it, so the device is not kept here.
#[derive(Clone)]
struct SharedCache(CFRetained<CVMetalTextureCache>);

// SAFETY: `CVMetalTextureCache` is documented as thread-safe, and this
// wrapper only shares it.
unsafe impl Send for SharedCache {}
unsafe impl Sync for SharedCache {}

/// What keeps a frame's texture valid: the pixel buffer and its Metal wrapper.
struct FrameOwner {
    _texture: Retained<ProtocolObject<dyn MTLTexture>>,
    _metal: CFRetained<CVMetalTexture>,
}

// SAFETY: retained Core Video and Metal objects may be released from any
// thread, and the owner never touches them otherwise.
unsafe impl Send for FrameOwner {}
unsafe impl Sync for FrameOwner {}

/// One `AVPlayer` with its item and frame output.
///
/// The player handle calls these objects from whichever thread it is used on.
/// `AVPlayer`'s controls and state, `AVPlayerItem`'s state and
/// `AVPlayerItemVideoOutput`'s pixel-buffer methods are documented as safe to
/// use from any thread, so the handle's own lock is the only synchronisation
/// this needs.
struct AppleVideo {
    player: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    output: Retained<AVPlayerItemVideoOutput>,
    cache: SharedCache,
}

// SAFETY: see the type's documentation.
unsafe impl Send for AppleVideo {}
unsafe impl Sync for AppleVideo {}

/// Seconds of `time`, or `None` when it is invalid, indefinite or infinite.
fn finite_seconds(time: CMTime) -> Option<f64> {
    // SAFETY: `CMTimeGetSeconds` only reads its by-value argument.
    let seconds = unsafe { time.seconds() };
    seconds.is_finite().then_some(seconds)
}

// SAFETY (every `unsafe` block below unless noted): the objects are valid and
// used from any thread as the type's documentation allows.
impl PlatformVideo for AppleVideo {
    fn play(&self) {
        unsafe { self.player.play() }
    }

    fn pause(&self) {
        unsafe { self.player.pause() }
    }

    fn seek(&self, seconds: f64) {
        if seconds.is_finite() {
            unsafe { self.player.seekToTime(CMTime::with_seconds(seconds, TIMESCALE)) }
        }
    }

    fn set_volume(&self, volume: f32) {
        unsafe { self.player.setVolume(volume) }
    }

    fn is_ready(&self) -> bool {
        let status = unsafe { self.item.status() };
        status == AVPlayerItemStatus::ReadyToPlay
    }

    fn position(&self) -> f64 {
        finite_seconds(unsafe { self.player.currentTime() })
            .unwrap_or(0.0)
            .max(0.0)
    }

    fn duration(&self) -> f64 {
        if !self.is_ready() {
            return f64::NAN;
        }
        finite_seconds(unsafe { self.item.duration() }).unwrap_or(f64::NAN)
    }

    fn buffered(&self) -> f64 {
        let position = self.position();
        let mut end = position;
        let ranges = unsafe { self.item.loadedTimeRanges() };
        for index in 0..ranges.len() {
            // SAFETY: the array holds `NSValue`s wrapping `CMTimeRange`s.
            let range = unsafe { ranges.objectAtIndex(index).CMTimeRangeValue() };
            let (Some(start), Some(stop)) =
                (finite_seconds(range.start), finite_seconds(unsafe { range.end() }))
            else {
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
        // The requested rate: non-zero while playing or stalled for data,
        // zero once paused, ended or failed.
        let rate = unsafe { self.player.rate() };
        rate != 0.0
    }

    fn is_buffering(&self) -> bool {
        let status = unsafe { self.player.timeControlStatus() };
        status == AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate
    }

    fn error(&self) -> Option<VideoError> {
        let item_failed = unsafe { self.item.status() } == AVPlayerItemStatus::Failed;
        let player_failed = unsafe { self.player.status() } == AVPlayerStatus::Failed;
        if !item_failed && !player_failed {
            return None;
        }
        let error = if item_failed { unsafe { self.item.error() } } else { unsafe { self.player.error() } };
        let why = error
            .map(|error| error.localizedDescription().to_string())
            .unwrap_or_else(|| "the video could not be played".into());
        // Before the item opened, a failure is almost always the source.
        Some(if self.duration().is_finite() {
            VideoError::Playback(why)
        } else {
            VideoError::Decode(why)
        })
    }

    fn size(&self) -> Option<(u32, u32)> {
        if !self.is_ready() {
            return None;
        }
        let size = unsafe { self.item.presentationSize() };
        let (width, height) = (size.width.round(), size.height.round());
        (width >= 1.0 && height >= 1.0).then_some((width as u32, height as u32))
    }

    fn take_frame(&self, frames: &FrameSlot) -> bool {
        let now = CACurrentMediaTime();
        let output: &AVPlayerItemOutput = &self.output;
        let item_time = unsafe { output.itemTimeForHostTime(now) };
        if !unsafe { self.output.hasNewPixelBufferForItemTime(item_time) } {
            return false;
        }
        let Some(pixels) = (unsafe {
            self.output
                .copyPixelBufferForItemTime_itemTimeForDisplay(item_time, std::ptr::null_mut())
        }) else {
            return false;
        };
        let width = CVPixelBufferGetWidth(&pixels);
        let height = CVPixelBufferGetHeight(&pixels);
        let mut metal: *mut CVMetalTexture = std::ptr::null_mut();
        // SAFETY: `pixels` is a Metal-compatible BGRA buffer (the output was
        // asked for one), plane 0 is its only plane, and `metal` receives a
        // +1 reference on success.
        let status = unsafe {
            CVMetalTextureCache::create_texture_from_image(
                None,
                &self.cache.0,
                &pixels,
                None,
                MTLPixelFormat::BGRA8Unorm,
                width,
                height,
                0,
                NonNull::from(&mut metal),
            )
        };
        let Some(metal) = NonNull::new(metal).filter(|_| status == kCVReturnSuccess) else {
            return false;
        };
        // SAFETY: on success `metal` is a valid, owned, +1 reference.
        let metal = unsafe { CFRetained::from_raw(metal) };
        let Some(texture) = CVMetalTextureGetTexture(&metal) else {
            return false;
        };
        let handle = NativeTexture::Metal(NonNull::from(&*texture).cast());
        // The texture views the pixel buffer's IOSurface, which `metal` keeps
        // alive; `metal` and the texture together are the frame's owner.
        let owner = Arc::new(FrameOwner {
            _texture: texture,
            _metal: metal,
        });
        // SAFETY: `owner` keeps the texture valid, and a decoded frame's
        // pixels never change.
        let frame = unsafe { StoredFrame::new(handle, width as u32, height as u32, owner) };
        frames.store(Some(frame));
        true
    }
}

/// Lets playback start with the silent switch on, as a video player should.
#[cfg(target_os = "ios")]
fn configure_session() {
    static CONFIGURED: OnceLock<()> = OnceLock::new();
    CONFIGURED.get_or_init(|| {
        use objc2_avf_audio::{AVAudioSession, AVAudioSessionCategoryPlayback};
        // SAFETY: the shared session is a process-wide singleton and the
        // category is a valid, immutable constant.
        unsafe {
            let session = AVAudioSession::sharedInstance();
            if let Some(category) = AVAudioSessionCategoryPlayback {
                let _ = session.setCategory_error(category);
            }
            let _ = session.setActive_error(true);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::bundled_candidates;
    use std::path::{Path, PathBuf};

    #[test]
    fn bundled_assets_are_searched_in_the_working_directory_then_the_bundle() {
        let exe = Path::new("/Apps/Demo.app/Contents/MacOS/demo");
        assert_eq!(
            bundled_candidates("assets/a.mp4", Some(exe)),
            vec![
                PathBuf::from("assets/a.mp4"),
                PathBuf::from("/Apps/Demo.app/Contents/Resources/assets/a.mp4"),
                PathBuf::from("/Apps/Demo.app/Contents/MacOS/assets/a.mp4"),
            ]
        );
    }
}
