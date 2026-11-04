//! macOS and iOS: `AVPlayer`, for tracks played while they download.
//!
//! `AVAudioPlayer` needs the whole file, so a streamed [`AudioSource::Stream`]
//! opens an `AVPlayer` on the URL instead. The system fetches, buffers and
//! decodes on its own threads; this voice only reads its state.
//!
//! [`AudioSource::Stream`]: crate::AudioSource::Stream

use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_av_foundation::{
    AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVPlayerTimeControlStatus,
    NSValueAVFoundationExtensions,
};
use objc2_core_media::CMTime;
use objc2_foundation::{NSString, NSURL};

use crate::AudioError;
use crate::backend::Voice;

/// `CMTime` timescale for seeks: one millisecond.
const TIMESCALE: i32 = 1000;
/// A download this close to the end counts as complete, so a paused stream
/// stops being watched once the last bytes are in.
const COMPLETE_SLACK: f64 = 0.25;

/// Opens a paused player on `url`. The connection starts at once so the first
/// `play` has little left to wait for.
pub(crate) fn open(url: &str) -> Result<Box<dyn Voice>, AudioError> {
    let ns_url = NSURL::URLWithString(&NSString::from_str(url))
        .ok_or_else(|| AudioError::Network(format!("invalid URL: {url}")))?;
    // SAFETY: objc2 conservatively marks `AVPlayer` main-thread-only, but
    // AVFoundation documents the player's playback controls and state as safe
    // from any thread; only its layers and KVO callbacks have affinity, and
    // this voice uses neither. The marker is used for allocation only and the
    // player is then touched from the engine thread alone.
    let player = unsafe {
        let mtm = MainThreadMarker::new_unchecked();
        AVPlayer::initWithURL(AVPlayer::alloc(mtm), &ns_url)
    };
    Ok(Box::new(StreamVoice { player }))
}

struct StreamVoice {
    player: Retained<AVPlayer>,
}

impl StreamVoice {
    fn item(&self) -> Option<Retained<AVPlayerItem>> {
        // SAFETY: `player` is a valid `AVPlayer`; see `open` for threading.
        unsafe { self.player.currentItem() }
    }
}

/// Seconds of `time`, or `None` when it is invalid, indefinite or infinite.
fn finite_seconds(time: CMTime) -> Option<f64> {
    // SAFETY: `CMTimeGetSeconds` reads its by-value argument only.
    let seconds = unsafe { time.seconds() };
    seconds.is_finite().then_some(seconds)
}

// SAFETY (all `unsafe` blocks below): `player` and its item are valid objects
// and the engine thread is the only one that touches them.
impl Voice for StreamVoice {
    fn duration(&self) -> f64 {
        let Some(item) = self.item() else {
            return f64::NAN;
        };
        // The length is meaningless until the item has read its header.
        if unsafe { item.status() } != AVPlayerItemStatus::ReadyToPlay {
            return f64::NAN;
        }
        finite_seconds(unsafe { item.duration() }).unwrap_or(f64::NAN)
    }

    fn position(&self) -> f64 {
        finite_seconds(unsafe { self.player.currentTime() })
            .unwrap_or(0.0)
            .max(0.0)
    }

    fn is_playing(&self) -> bool {
        // The requested rate, which stays non-zero while the player stalls for
        // data and drops to zero when it is paused, ends, or fails.
        let rate = unsafe { self.player.rate() };
        rate != 0.0
    }

    fn play(&mut self) -> Result<(), AudioError> {
        unsafe { self.player.play() };
        Ok(())
    }

    fn pause(&mut self) {
        unsafe { self.player.pause() }
    }

    fn stop(&mut self) {
        unsafe { self.player.pause() };
        self.seek(0.0);
    }

    fn seek(&mut self, seconds: f64) {
        if !seconds.is_finite() {
            return;
        }
        // Not frame-exact: the default tolerance lets a compressed stream land
        // on the nearest cheap point instead of decoding up to the target.
        unsafe { self.player.seekToTime(CMTime::with_seconds(seconds, TIMESCALE)) }
    }

    fn set_volume(&mut self, volume: f32) {
        unsafe { self.player.setVolume(volume) }
    }

    /// Ignored: `AVPlayer` has no loop mode, so the engine rewinds instead.
    fn set_extra_loops(&mut self, _extra: i64) {}

    fn is_buffering(&self) -> bool {
        let status = unsafe { self.player.timeControlStatus() };
        status == AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate
    }

    fn buffered(&self) -> f64 {
        let Some(item) = self.item() else {
            return 0.0;
        };
        let position = self.position();
        let mut end = position;
        let ranges = unsafe { item.loadedTimeRanges() };
        for index in 0..ranges.len() {
            // SAFETY: the array holds `NSValue`s wrapping `CMTimeRange`s.
            let range = unsafe { ranges.objectAtIndex(index).CMTimeRangeValue() };
            let (Some(start), Some(stop)) = (
                finite_seconds(range.start),
                finite_seconds(unsafe { range.end() }),
            ) else {
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
        let item = self.item()?;
        if unsafe { item.status() } != AVPlayerItemStatus::Failed {
            return None;
        }
        let why = unsafe { item.error() }
            .map(|error| error.localizedDescription().to_string())
            .unwrap_or_else(|| "the stream failed".into());
        Some(AudioError::Network(why))
    }
}
