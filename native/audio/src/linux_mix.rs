//! Linux: the platform-free half of the software player.
//!
//! Every voice ends up as stereo `f32` frames at [`OUT_RATE`]; the output
//! thread adds the playing ones together. A sound's whole clip is decoded once
//! into shared [`Pcm`]; a track is decoded progressively by its own thread into
//! a bounded queue, so a long file never sits in memory decoded.
//!
//! The mixer runs once per output period and must not allocate or block for
//! long: it takes each playing voice's lock for one short copy.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::AudioError;

/// The mixer's sample rate; every voice is resampled to it once, when decoded.
pub(crate) const OUT_RATE: u32 = 48_000;
/// Interleaved output channels.
pub(crate) const CHANNELS: usize = 2;
/// Decoded audio a track keeps ahead of the playhead: half a second.
pub(crate) const QUEUE_FRAMES: usize = OUT_RATE as usize / 2;

/// A sound's whole clip, decoded and resampled: stereo frames at [`OUT_RATE`].
pub(crate) type Pcm = Arc<[f32]>;

/// Where a voice's frames come from.
pub(crate) enum Feed {
    /// A decoded clip, read in place.
    Clip { pcm: Pcm, cursor: usize },
    /// Frames a decoder thread pushes ahead of the playhead.
    Queue {
        queue: VecDeque<f32>,
        /// The decoder reached the end of the source.
        ended: bool,
        /// A seek for the decoder thread, in seconds.
        seek: Option<f64>,
    },
}

/// What the engine, the mixer and a decoder thread share about one voice.
pub(crate) struct Body {
    pub feed: Feed,
    /// Frames played since the start of the source.
    pub frame: u64,
    /// Seconds; `NaN` until a stream's header has been read.
    pub duration: f64,
    /// The mixer found the queue empty while it should play.
    pub starved: bool,
    /// Wrap around at the end instead of stopping.
    pub looping: bool,
    pub error: Option<AudioError>,
    /// The engine dropped the voice; its decoder thread exits.
    pub released: bool,
}

/// One voice, shared between threads.
pub(crate) struct VoiceShared {
    pub playing: AtomicBool,
    /// `f32` bits of the linear gain.
    volume: AtomicU32,
    body: Mutex<Body>,
    /// Wakes a decoder thread waiting for room, a seek, or release.
    pub wake: Condvar,
}

impl VoiceShared {
    pub fn new(feed: Feed, duration: f64) -> Arc<Self> {
        Arc::new(Self {
            playing: AtomicBool::new(false),
            volume: AtomicU32::new(1.0_f32.to_bits()),
            body: Mutex::new(Body {
                feed,
                frame: 0,
                duration,
                starved: false,
                looping: false,
                error: None,
                released: false,
            }),
            wake: Condvar::new(),
        })
    }

    /// Locks the body. A poisoned lock is recovered: it holds plain data, and
    /// a panicking decoder must not silence the mixer.
    pub fn lock(&self) -> MutexGuard<'_, Body> {
        self.body.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
    }

    /// Moves the playhead to `seconds`, dropping audio decoded for the old
    /// position at once so none of it plays.
    pub fn seek(&self, seconds: f64) {
        let mut body = self.lock();
        let frame = (seconds.max(0.0) * f64::from(OUT_RATE)) as u64;
        body.starved = false;
        match &mut body.feed {
            Feed::Clip { pcm, cursor } => {
                *cursor = (frame as usize * CHANNELS).min(pcm.len());
                body.frame = (*cursor / CHANNELS) as u64;
            }
            Feed::Queue { queue, ended, seek } => {
                queue.clear();
                *ended = false;
                *seek = Some(seconds.max(0.0));
                body.frame = frame;
            }
        }
        drop(body);
        self.wake.notify_all();
    }
}

/// Adds `voice`'s next frames into `out` (interleaved stereo) and advances it.
///
/// Clears `playing` when a clip or a finished queue runs out, which is how the
/// engine sees the end.
pub(crate) fn mix_voice(voice: &VoiceShared, out: &mut [f32]) {
    if !voice.playing.load(Ordering::Acquire) {
        return;
    }
    let gain = voice.volume();
    let mut body = voice.lock();
    let wanted = out.len() / CHANNELS;
    let looping = body.looping;
    let mut written = 0;
    let mut finished = false;
    let mut starved = false;
    match &mut body.feed {
        Feed::Clip { pcm, cursor } => {
            while written < wanted {
                if *cursor >= pcm.len() {
                    if looping && !pcm.is_empty() {
                        *cursor = 0;
                    } else {
                        finished = true;
                        break;
                    }
                }
                let take = ((pcm.len() - *cursor) / CHANNELS).min(wanted - written);
                let src = &pcm[*cursor..*cursor + take * CHANNELS];
                let dst = &mut out[written * CHANNELS..(written + take) * CHANNELS];
                add_scaled(dst, src, gain);
                *cursor += take * CHANNELS;
                written += take;
            }
        }
        Feed::Queue { queue, ended, .. } => {
            let take = (queue.len() / CHANNELS).min(wanted);
            let samples = take * CHANNELS;
            let (front, back) = queue.as_slices();
            let first = front.len().min(samples);
            add_scaled(&mut out[..first], &front[..first], gain);
            add_scaled(&mut out[first..samples], &back[..samples - first], gain);
            queue.drain(..samples);
            written = take;
            // Running dry with more to come is buffering, not the end.
            finished = take < wanted && *ended;
            starved = take < wanted && !*ended;
        }
    }
    body.frame += written as u64;
    body.starved = starved;
    let popped = written > 0 && matches!(body.feed, Feed::Queue { .. });
    drop(body);
    if finished {
        voice.playing.store(false, Ordering::Release);
    }
    if popped {
        // A decoder waiting for room can refill now.
        voice.wake.notify_all();
    }
}

#[inline]
fn add_scaled(dst: &mut [f32], src: &[f32], gain: f32) {
    for (out, sample) in dst.iter_mut().zip(src) {
        *out += sample * gain;
    }
}

/// Converts mixed `f32` frames to the device's 16-bit samples, clipping.
pub(crate) fn to_i16(src: &[f32], dst: &mut [i16]) {
    for (out, sample) in dst.iter_mut().zip(src) {
        *out = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
    }
}

/// Converts interleaved audio of any rate and channel count to stereo at
/// [`OUT_RATE`], by linear interpolation. It keeps its phase across calls, so
/// a stream converted in chunks has no seams.
pub(crate) struct Resampler {
    in_rate: u32,
    channels: usize,
    /// Position of the next output frame, in input frames, relative to the
    /// first frame of the next chunk; `-1.0..0.0` means between `last` and it.
    phase: f64,
    /// The last stereo input frame of the previous chunk.
    last: [f32; 2],
    primed: bool,
}

impl Resampler {
    pub fn new(in_rate: u32, channels: usize) -> Self {
        Self {
            in_rate: in_rate.max(1),
            channels: channels.max(1),
            phase: 0.0,
            last: [0.0; 2],
            primed: false,
        }
    }

    /// Forgets the previous chunk, after a seek.
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.primed = false;
    }

    /// Appends `input` (interleaved, `channels` per frame) to `out` as stereo
    /// frames at [`OUT_RATE`].
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let frames = input.len() / self.channels;
        if frames == 0 {
            return;
        }
        let stereo = |index: usize| -> [f32; 2] {
            let base = index * self.channels;
            let left = input[base];
            let right = if self.channels > 1 { input[base + 1] } else { left };
            [left, right]
        };
        if self.in_rate == OUT_RATE {
            out.reserve(frames * CHANNELS);
            for index in 0..frames {
                out.extend_from_slice(&stereo(index));
            }
            self.last = stereo(frames - 1);
            self.primed = true;
            return;
        }
        let step = f64::from(self.in_rate) / f64::from(OUT_RATE);
        if !self.primed {
            // The very first output frame is the first input frame.
            self.last = stereo(0);
            self.phase = 0.0;
            self.primed = true;
        }
        out.reserve((frames as f64 / step) as usize * CHANNELS + CHANNELS);
        // `phase` is measured from input frame 0; frame -1 is `last`.
        while self.phase < (frames - 1) as f64 {
            let (a, b, t) = if self.phase < 0.0 {
                (self.last, stereo(0), (self.phase + 1.0) as f32)
            } else {
                let index = self.phase as usize;
                (stereo(index), stereo(index + 1), (self.phase - index as f64) as f32)
            };
            out.push(a[0] + (b[0] - a[0]) * t);
            out.push(a[1] + (b[1] - a[1]) * t);
            self.phase += step;
        }
        self.last = stereo(frames - 1);
        self.phase -= frames as f64;
    }
}

#[cfg(test)]
#[path = "linux_mix_tests.rs"]
mod tests;
