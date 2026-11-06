//! The clock behind a playing animation: a position that advances with elapsed
//! time, loops or stops, and says whether another frame is needed.

use std::time::Duration;

/// What one [`Playback::advance`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// The frame to show is different from before.
    pub changed: bool,
    /// Playback is still running, so the next frame should be requested.
    pub wants_frame: bool,
}

/// A playhead over a frame range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Playback {
    start_frame: f32,
    end_frame: f32,
    frame_rate: f32,
    looping: bool,
    speed: f32,
    playing: bool,
    /// Seconds from the start, within `0..=duration`.
    position: f32,
}

impl Playback {
    /// A paused playhead at `start_frame` over `start_frame..end_frame`, which
    /// plays through once at normal speed once [`playing`](Self::playing) is set.
    ///
    /// A range that is empty, reversed or not finite, or a frame rate that is not
    /// positive, gives a still animation that never asks for frames.
    pub fn new(start_frame: f32, end_frame: f32, frame_rate: f32) -> Self {
        Self {
            start_frame,
            end_frame,
            frame_rate,
            looping: false,
            speed: 1.0,
            playing: false,
            position: 0.0,
        }
    }

    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Playback rate: `1.0` is real time, negative plays backwards, `0.0` or a
    /// non-finite value holds the playhead.
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    pub fn playing(mut self, playing: bool) -> Self {
        self.playing = playing;
        self
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Length of the range in seconds; `0.0` when the range cannot be played.
    pub fn duration(&self) -> f32 {
        let seconds = (self.end_frame - self.start_frame) / self.frame_rate;
        if seconds.is_finite() && seconds > 0.0 { seconds } else { 0.0 }
    }

    /// The frame to show now. The out point is exclusive in Lottie, so the
    /// playhead never reports it: the end of the range is the frame before.
    pub fn frame(&self) -> f32 {
        if self.duration() == 0.0 {
            return if self.start_frame.is_finite() { self.start_frame } else { 0.0 };
        }
        (self.start_frame + self.position * self.frame_rate).min(self.end_frame.next_down())
    }

    /// Moves the playhead to `progress` of the range, clamped to `0..=1`; a NaN
    /// is ignored. Does not start or stop playback.
    pub fn set_progress(&mut self, progress: f32) {
        if !progress.is_nan() {
            self.position = progress.clamp(0.0, 1.0) * self.duration();
        }
    }

    /// Moves the playhead forward by `dt` of real time.
    pub fn advance(&mut self, dt: Duration) -> Tick {
        let duration = self.duration();
        let still = Tick { changed: false, wants_frame: false };
        if !self.playing || duration == 0.0 || !self.speed.is_finite() || self.speed == 0.0 {
            return still;
        }
        let mut position = self.position + dt.as_secs_f32() * self.speed;
        if !position.is_finite() {
            return still;
        }
        if self.looping {
            position = position.rem_euclid(duration);
        } else if position >= duration && self.speed > 0.0 {
            position = duration;
            self.playing = false;
        } else if position <= 0.0 && self.speed < 0.0 {
            position = 0.0;
            self.playing = false;
        }
        let before = self.frame();
        self.position = position;
        Tick { changed: self.frame() != before, wants_frame: self.playing }
    }

    /// Takes over `other`'s playhead position, keeping this one's settings. The
    /// position is clamped to this range.
    pub fn adopt_position(&mut self, other: &Playback) {
        self.position = other.position.min(self.duration());
    }
}
