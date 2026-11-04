use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use aimer_cupid::external_texture::{ExternalTexture, ExternalTextureSource};

use crate::backend::{FrameSlot, PlatformVideo};
use crate::{PlaybackState, VideoError, VideoLoops, VideoSource};

/// UI frames a paused player keeps asking for the picture a seek, load or
/// stop produces: about two seconds at 60 Hz. A frame that never arrives (a
/// seek past the end of a stream, say) must not keep the app busy.
const PENDING_FRAME_POLLS: u32 = 120;

/// Plays one video, with its sound, and supplies its frames to a
/// [`Video`](crate::Video) widget.
///
/// A `VideoPlayer` is a cheap handle; clones control the same video, so a
/// widget can keep one in its state and hand copies to its buttons. Dropping
/// the last clone stops the video.
///
/// Every control is safe to call from any thread and never blocks: the
/// platform opens, buffers and decodes on its own threads. `play` before the
/// source has opened starts it the moment it can.
///
/// Decoded frames stay on the GPU: the [`Video`](crate::Video) widget draws the
/// decoder's own texture, with no copy through the CPU.
///
/// # Examples
///
/// ```no_run
/// use aimer_video::{VideoLoops, VideoPlayer, VideoSource};
///
/// let player = VideoPlayer::from_source(VideoSource::bundled("assets/intro.mp4"));
/// player.set_loops(VideoLoops::Infinite);
/// player.set_muted(true);
/// player.play();
/// ```
#[derive(Clone)]
pub struct VideoPlayer {
    inner: Arc<Inner>,
}

struct Inner {
    core: Mutex<Core>,
    /// Shared with the renderer; outlives every platform player it is given.
    frames: Arc<FrameSlot>,
    open: fn(&VideoSource) -> Result<Box<dyn PlatformVideo>, VideoError>,
}

struct Core {
    platform: Option<Box<dyn PlatformVideo>>,
    error: Option<VideoError>,
    /// The caller wants the video playing: set by `play`, cleared by `pause`,
    /// `stop`, the end, and failures.
    want_play: bool,
    /// Played at least once since the last load or stop.
    started: bool,
    finished: bool,
    loops: VideoLoops,
    /// Repeats still to come in the current run.
    repeats_left: u64,
    volume: f32,
    muted: bool,
    /// Bumped for every new frame, so the render tree repaints the video
    /// exactly when its picture changes.
    generation: u64,
    /// UI frames left to wait for a picture that changed while not playing
    /// (after a load, seek or stop). Zero while idle.
    pending_polls: u32,
    /// The picture size, kept so painting never asks the platform.
    size: Option<(u32, u32)>,
}

impl Default for VideoPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoPlayer {
    /// Creates a player with nothing loaded.
    pub fn new() -> Self {
        Self::with_platform(crate::platform_open)
    }

    pub(crate) fn with_platform(open: fn(&VideoSource) -> Result<Box<dyn PlatformVideo>, VideoError>) -> Self {
        Self {
            inner: Arc::new(Inner {
                core: Mutex::new(Core {
                    platform: None,
                    error: None,
                    want_play: false,
                    started: false,
                    finished: false,
                    loops: VideoLoops::Once,
                    repeats_left: 0,
                    volume: 1.0,
                    muted: false,
                    generation: 0,
                    pending_polls: 0,
                    size: None,
                }),
                frames: Arc::new(FrameSlot::default()),
                open,
            }),
        }
    }

    /// Creates a player and starts opening `source`.
    pub fn from_source(source: VideoSource) -> Self {
        let player = Self::new();
        player.load(source);
        player
    }

    fn lock(&self) -> MutexGuard<'_, Core> {
        // Plain data: a panic elsewhere must not silence the controls forever.
        self.inner.core.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the current video with `source`.
    ///
    /// Playback stops and the playhead resets; volume, mute and loops are kept.
    pub fn load(&self, source: VideoSource) {
        let mut core = self.lock();
        // Drop the old player first, so its sound stops before the new opens.
        core.platform = None;
        self.inner.frames.store(None);
        core.error = None;
        core.want_play = false;
        core.started = false;
        core.finished = false;
        core.repeats_left = core.loops.repeats();
        core.generation = core.generation.wrapping_add(1);
        core.size = None;
        match (self.inner.open)(&source) {
            Ok(platform) => {
                platform.set_volume(core.effective_volume());
                core.platform = Some(platform);
                // The first picture arrives without `play`.
                core.pending_polls = PENDING_FRAME_POLLS;
            }
            Err(error) => core.error = Some(error),
        }
    }

    /// Starts, resumes, or (after the end) restarts playback.
    ///
    /// Does nothing before a source is loaded or after a failure.
    pub fn play(&self) {
        let mut core = self.lock();
        if core.error.is_some() {
            return;
        }
        let Some(platform) = core.platform.as_ref() else {
            return;
        };
        if core.finished {
            platform.seek(0.0);
        }
        platform.play();
        if std::mem::take(&mut core.finished) {
            core.repeats_left = core.loops.repeats();
        }
        core.want_play = true;
        core.started = true;
    }

    /// Pauses, keeping the playhead.
    pub fn pause(&self) {
        let mut core = self.lock();
        core.want_play = false;
        if let Some(platform) = core.platform.as_ref() {
            platform.pause();
        }
    }

    /// Pauses when playing and plays otherwise.
    pub fn toggle(&self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Stops and rewinds to the start.
    pub fn stop(&self) {
        let mut core = self.lock();
        core.want_play = false;
        core.started = false;
        core.finished = false;
        core.repeats_left = core.loops.repeats();
        if let Some(platform) = core.platform.as_ref() {
            platform.pause();
            platform.seek(0.0);
            core.pending_polls = PENDING_FRAME_POLLS;
        }
    }

    /// Moves the playhead, clamped to the video's length once that is known.
    pub fn seek(&self, position: Duration) {
        let mut core = self.lock();
        let Some(platform) = core.platform.as_ref() else {
            return;
        };
        let duration = platform.duration();
        let mut seconds = position.as_secs_f64();
        if duration.is_finite() {
            seconds = seconds.min(duration.max(0.0));
        }
        platform.seek(seconds);
        core.finished = false;
        // A paused player shows the new position's frame once it decodes.
        core.pending_polls = PENDING_FRAME_POLLS;
    }

    /// Sets the volume, `0.0` (silent) to `1.0` (full). Out-of-range values are
    /// clamped and `NaN` is ignored.
    pub fn set_volume(&self, volume: f32) {
        if volume.is_nan() {
            return;
        }
        let mut core = self.lock();
        core.volume = volume.clamp(0.0, 1.0);
        core.apply_volume();
    }

    /// Mutes or unmutes without forgetting the volume.
    pub fn set_muted(&self, muted: bool) {
        let mut core = self.lock();
        core.muted = muted;
        core.apply_volume();
    }

    /// Sets how many times the video plays.
    pub fn set_loops(&self, loops: VideoLoops) {
        let mut core = self.lock();
        core.loops = loops;
        core.repeats_left = loops.repeats();
    }

    /// What the player is doing.
    pub fn state(&self) -> PlaybackState {
        let mut core = self.lock();
        core.settle();
        core.state()
    }

    /// Whether the video is, or is about to start, playing. A video that is
    /// [buffering](PlaybackState::Buffering) still counts.
    pub fn is_playing(&self) -> bool {
        matches!(self.state(), PlaybackState::Playing | PlaybackState::Buffering)
    }

    /// The playhead.
    pub fn position(&self) -> Duration {
        let core = self.lock();
        let seconds = core.platform.as_ref().map_or(0.0, |platform| platform.position());
        Duration::try_from_secs_f64(seconds).unwrap_or_default()
    }

    /// The video's length, once known. A live stream has none.
    pub fn duration(&self) -> Option<Duration> {
        let core = self.lock();
        let seconds = core.platform.as_ref()?.duration();
        Duration::try_from_secs_f64(seconds).ok()
    }

    /// How far into the video data is available without waiting.
    pub fn buffered(&self) -> Duration {
        let core = self.lock();
        let seconds = core.platform.as_ref().map_or(0.0, |platform| platform.buffered());
        Duration::try_from_secs_f64(seconds).unwrap_or_default()
    }

    /// The picture size in pixels, once the source has opened.
    pub fn size(&self) -> Option<(u32, u32)> {
        let mut core = self.lock();
        if core.size.is_none() {
            core.size = core.platform.as_ref()?.size();
        }
        core.size
    }

    /// Why loading or playback failed.
    pub fn error(&self) -> Option<VideoError> {
        let mut core = self.lock();
        core.settle();
        core.error.clone()
    }

    /// The volume set with [`set_volume`](Self::set_volume).
    pub fn volume(&self) -> f32 {
        self.lock().volume
    }

    /// Whether the player is muted.
    pub fn is_muted(&self) -> bool {
        self.lock().muted
    }

    /// The repeat setting.
    pub fn loops(&self) -> VideoLoops {
        self.lock().loops
    }

    /// Advances the player for one UI frame: notices the end, starts the next
    /// loop, and takes the frame due now. Returns the frame generation, which
    /// changes exactly when the picture does.
    ///
    /// The [`Video`](crate::Video) widget calls this every frame it is
    /// visible; call it yourself only when drawing the frames another way.
    ///
    /// A paused player with no picture on its way returns at once without
    /// calling the platform, so a paused video costs nothing per frame.
    pub fn poll(&self) -> u64 {
        let mut core = self.lock();
        if !core.wants_frames() {
            return core.generation;
        }
        core.pending_polls = core.pending_polls.saturating_sub(1);
        core.settle();
        let Some(platform) = core.platform.as_ref() else {
            return core.generation;
        };
        if !platform.take_frame(&self.inner.frames) {
            return core.generation;
        }
        let size = if core.size.is_none() { platform.size() } else { core.size };
        core.size = size;
        core.generation = core.generation.wrapping_add(1);
        // The awaited picture arrived; a paused player can rest now.
        core.pending_polls = 0;
        core.generation
    }

    /// Whether the player needs UI frames: it is playing, still opening, or
    /// fetching the picture a seek, load or stop produced while paused. The
    /// [`Video`](crate::Video) widget requests frames only while this holds.
    /// Answers from the player's own state, without calling the platform.
    pub fn wants_frames(&self) -> bool {
        self.lock().wants_frames()
    }

    /// The frame generation and picture size to paint with, without calling
    /// the platform.
    pub(crate) fn paint_state(&self) -> (u64, Option<(u32, u32)>) {
        let core = self.lock();
        (core.generation, core.size)
    }

    /// The current frame as a draw-command payload; see
    /// `aimer_canvas::Canvas::draw_external_texture`.
    pub fn texture(&self) -> ExternalTexture {
        let generation = self.lock().generation;
        let source: Arc<dyn ExternalTextureSource> = self.inner.frames.clone();
        ExternalTexture::new(source, generation)
    }

    /// Whether a frame has been decoded since the last load.
    pub fn has_frame(&self) -> bool {
        self.inner.frames.has_frame()
    }
}

impl Core {
    fn wants_frames(&self) -> bool {
        self.platform.is_some() && self.error.is_none() && (self.want_play || self.pending_polls > 0)
    }

    fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }

    fn apply_volume(&self) {
        if let Some(platform) = self.platform.as_ref() {
            platform.set_volume(self.effective_volume());
        }
    }

    /// Notices a failure or the end of a pass, and loops.
    fn settle(&mut self) {
        let Some(platform) = self.platform.as_ref() else {
            return;
        };
        if self.error.is_none()
            && let Some(error) = platform.error()
        {
            // A failed stream stops, which must not read as the end.
            self.error = Some(error);
            self.want_play = false;
            self.pending_polls = 0;
            return;
        }
        if !self.want_play || platform.is_playing() {
            return;
        }
        if self.repeats_left > 0 {
            if self.repeats_left != u64::MAX {
                self.repeats_left -= 1;
            }
            platform.seek(0.0);
            platform.play();
        } else {
            self.want_play = false;
            self.finished = true;
            // The last frame is on screen; nothing more will come.
            self.pending_polls = 0;
        }
    }

    fn state(&self) -> PlaybackState {
        let Some(platform) = self.platform.as_ref() else {
            return if self.error.is_some() { PlaybackState::Failed } else { PlaybackState::Idle };
        };
        if self.error.is_some() {
            return PlaybackState::Failed;
        }
        if self.want_play {
            return if platform.is_buffering() || !platform.is_ready() {
                PlaybackState::Buffering
            } else {
                PlaybackState::Playing
            };
        }
        if self.finished {
            PlaybackState::Finished
        } else if !platform.is_ready() {
            PlaybackState::Loading
        } else if self.started {
            PlaybackState::Paused
        } else {
            PlaybackState::Ready
        }
    }
}

#[cfg(test)]
#[path = "player_tests.rs"]
mod tests;
