//! Linux: FFmpeg decodes in a child process, frames are uploaded by the
//! renderer.
//!
//! Linux has no system video player to hand a file to, so this backend runs
//! `ffmpeg` (see [`linux_ffmpeg`](crate::linux_ffmpeg)) to decode and stream
//! the source into raw RGBA pictures, and plays the soundtrack through
//! [`aimer_audio`]. The soundtrack is the clock when it is running: pictures
//! are shown when the audio reaches them. A video without sound, or one whose
//! sound cannot play (no audio device), runs on the wall clock instead.
//!
//! This is the software path: pictures are decoded on the CPU and uploaded to
//! the GPU once each (`NativeTexture::Rgba8`). Both programs, `ffmpeg` and
//! `ffprobe`, must be installed; a missing one fails the load with
//! [`VideoError::Playback`] saying so.

use std::any::Any;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use aimer_audio::{AudioPlayer, AudioSource};
use aimer_cupid::external_texture::{NativeTexture, new_source_id};

use crate::backend::{FrameSlot, PlatformVideo, StoredFrame};
use crate::linux_ffmpeg::{Info, Programs, Session, hardware_decoding_allowed, probe};
use crate::{VideoError, VideoSource};

/// How far the soundtrack may be from the wall clock for it to be believed
/// after a start or a seek (the engine reports the old position for a moment).
const AUDIO_TRUST: f64 = 0.75;

/// Opens `source` as a paused player.
pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn PlatformVideo>, VideoError> {
    open_with(source, Programs::from_environment())
}

/// [`open`], running the given FFmpeg programs.
pub(crate) fn open_with(source: &VideoSource, programs: Programs) -> Result<Box<dyn PlatformVideo>, VideoError> {
    let (input, audio_source, local) = match source {
        VideoSource::File(path) => {
            let path = existing(path.clone())?;
            (path.to_string_lossy().into_owned(), AudioSource::File(path), true)
        }
        VideoSource::Bundled(key) => {
            let exe = std::env::current_exe().ok();
            let path = crate::assets::bundled_candidates(key, exe.as_deref())
                .into_iter()
                .find(|candidate| candidate.is_file())
                .ok_or_else(|| VideoError::NotFound(key.clone()))?;
            (path.to_string_lossy().into_owned(), AudioSource::File(path), true)
        }
        VideoSource::Url(url) => {
            let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
            if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                return Err(VideoError::Network(format!("not an http(s) URL: {url}")));
            }
            (url.clone(), AudioSource::Stream(url.clone()), false)
        }
    };
    if !programs.available() {
        return Err(VideoError::Playback(
            "video on Linux needs FFmpeg: install the ffmpeg package (it provides ffmpeg and ffprobe)".into(),
        ));
    }
    let state = Arc::new(Mutex::new(State::new(input.clone(), programs)));
    // Probing a remote source waits on the network, so it does not run here.
    let worker = state.clone();
    std::thread::Builder::new()
        .name("aimer-video-probe".into())
        .spawn(move || {
            let info = probe(&worker.lock().unwrap_or_else(PoisonError::into_inner).programs.clone(), &input);
            worker
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .opened(info, audio_source);
        })
        .map_err(|error| VideoError::Playback(error.to_string()))?;
    Ok(Box::new(LinuxVideo {
        state,
        source: new_source_id(),
        local,
    }))
}

fn existing(path: PathBuf) -> Result<PathBuf, VideoError> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(VideoError::NotFound(path.display().to_string()))
    }
}

struct LinuxVideo {
    state: Arc<Mutex<State>>,
    /// Names this player's pictures to the renderer.
    source: u32,
    /// Whether the input is a file, which is all there once it is probed.
    local: bool,
}

struct State {
    input: String,
    programs: Programs,
    info: Option<Info>,
    error: Option<VideoError>,
    session: Option<Session>,
    audio: Option<AudioPlayer>,
    volume: f32,
    /// Whether the player last asked to play.
    playing: bool,
    ended: bool,
    /// The playhead at `since`, or while paused.
    base: f64,
    /// When the wall clock last started; `None` while paused or not begun.
    since: Option<Instant>,
    /// The soundtrack is running in step with the playhead, and is the clock.
    audio_started: bool,
    /// A seek asked for before the source was probed.
    pending_seek: Option<f64>,
    /// The next picture is shown whatever its time: the first one, and the
    /// one a seek lands on.
    show_first: bool,
    serial: u64,
    /// FFmpeg may decode on the GPU. Turned off for good once a source fails
    /// with it and works without.
    hardware: bool,
    /// The current session started with the GPU decoder, and has shown nothing
    /// yet: if it fails, decoding in software is worth a try.
    hardware_unproven: bool,
}

impl State {
    fn new(input: String, programs: Programs) -> Self {
        Self {
            input,
            programs,
            info: None,
            error: None,
            session: None,
            audio: None,
            volume: 1.0,
            playing: false,
            ended: false,
            base: 0.0,
            since: None,
            audio_started: false,
            pending_seek: None,
            show_first: true,
            serial: 0,
            hardware: hardware_decoding_allowed(),
            hardware_unproven: false,
        }
    }

    /// The probe finished.
    fn opened(&mut self, info: Result<Info, String>, audio: AudioSource) {
        let info = match info {
            Ok(info) => info,
            Err(why) => {
                self.error = Some(VideoError::Decode(why));
                return;
            }
        };
        if info.has_audio {
            let player = AudioPlayer::from_source(audio);
            player.set_volume(self.volume);
            self.audio = Some(player);
        }
        let start = self.pending_seek.take().unwrap_or(0.0);
        self.base = start;
        self.start_session(&info, start);
        self.info = Some(info);
        if start > 0.0
            && let Some(audio) = &self.audio
        {
            audio.seek(Duration::from_secs_f64(start));
        }
        if self.playing {
            self.since = Some(Instant::now());
            if let Some(audio) = &self.audio {
                audio.play();
            }
        }
    }

    /// The playhead in seconds.
    fn position(&mut self) -> f64 {
        let wall = self.base + self.since.map_or(0.0, |since| since.elapsed().as_secs_f64());
        if self.playing
            && let Some(audio) = &self.audio
            && audio.error().is_none()
        {
            let heard = audio.position().as_secs_f64();
            // Believe the soundtrack once it is running and agrees with the
            // clock; before that (loading, or just sought) the clock leads.
            if self.audio_started || (audio.is_playing() && heard > 0.0 && (heard - wall).abs() < AUDIO_TRUST) {
                self.audio_started = true;
                return heard;
            }
        }
        wall
    }

    fn duration(&self) -> Option<f64> {
        self.info.as_ref().and_then(|info| info.duration)
    }

    /// Moves the playhead and restarts decoding there.
    fn seek(&mut self, seconds: f64) {
        self.ended = false;
        let Some(info) = self.info.clone() else {
            self.pending_seek = Some(seconds);
            return;
        };
        let seconds = self.duration().map_or(seconds, |length| seconds.min(length)).max(0.0);
        self.base = seconds;
        self.since = self.playing.then(Instant::now);
        self.audio_started = false;
        self.show_first = true;
        if let Some(audio) = &self.audio {
            audio.seek(Duration::from_secs_f64(seconds));
        }
        // Dropping the old session kills its FFmpeg.
        self.session = None;
        self.start_session(&info, seconds);
    }

    /// Starts FFmpeg decoding from `start`.
    fn start_session(&mut self, info: &Info, start: f64) {
        match Session::start(&self.programs, &self.input, info, start, self.hardware) {
            Ok(session) => {
                self.session = Some(session);
                self.hardware_unproven = self.hardware;
            }
            Err(why) => self.error = Some(VideoError::Playback(why)),
        }
    }

    /// A session that used the GPU decoder failed before showing anything:
    /// decodes in software from now on, from the playhead. Returns whether it
    /// did, which is the case once only.
    fn retry_in_software(&mut self, info: &Info) -> bool {
        if !self.hardware || !self.hardware_unproven {
            return false;
        }
        self.hardware = false;
        self.hardware_unproven = false;
        self.session = None;
        let at = self.base + self.since.map_or(0.0, |since| since.elapsed().as_secs_f64());
        self.start_session(info, at.max(0.0));
        true
    }
}

impl LinuxVideo {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PlatformVideo for LinuxVideo {
    fn play(&self) {
        let mut state = self.lock();
        if state.ended {
            state.seek(0.0);
        }
        state.playing = true;
        if state.info.is_some() {
            if state.since.is_none() {
                state.since = Some(Instant::now());
            }
            if let Some(audio) = &state.audio {
                audio.play();
            }
        }
    }

    fn pause(&self) {
        let mut state = self.lock();
        if !state.playing {
            return;
        }
        // Freeze the playhead where it is.
        state.base = state.position();
        state.since = None;
        state.playing = false;
        state.audio_started = false;
        if let Some(audio) = &state.audio {
            audio.pause();
        }
    }

    fn seek(&self, seconds: f64) {
        self.lock().seek(seconds);
    }

    fn set_volume(&self, volume: f32) {
        let mut state = self.lock();
        state.volume = volume;
        if let Some(audio) = &state.audio {
            audio.set_volume(volume);
        }
    }

    fn is_ready(&self) -> bool {
        self.lock().info.is_some()
    }

    fn position(&self) -> f64 {
        let mut state = self.lock();
        let position = state.position();
        state.duration().map_or(position, |length| position.min(length))
    }

    fn duration(&self) -> f64 {
        self.lock().duration().unwrap_or(f64::NAN)
    }

    fn buffered(&self) -> f64 {
        let mut state = self.lock();
        let position = state.position();
        if self.local {
            return state.duration().unwrap_or(position);
        }
        state
            .session
            .as_ref()
            .and_then(Session::decoded_to)
            .unwrap_or(position)
            .max(position)
    }

    fn is_playing(&self) -> bool {
        let state = self.lock();
        state.playing && !state.ended
    }

    fn is_buffering(&self) -> bool {
        let state = self.lock();
        state.playing
            && !state.ended
            && state.info.is_some()
            && state.session.as_ref().is_some_and(|session| session.queued() == 0 && !session.drained())
    }

    fn error(&self) -> Option<VideoError> {
        self.lock().error.clone()
    }

    fn size(&self) -> Option<(u32, u32)> {
        self.lock().info.as_ref().map(|info| info.size)
    }

    fn take_frame(&self, frames: &FrameSlot) -> bool {
        let mut state = self.lock();
        let Some(info) = state.info.clone() else {
            return false;
        };
        let position = state.position();
        let Some(session) = state.session.as_ref() else {
            return false;
        };
        if let Some(why) = session.failure() {
            // The GPU decoder may be what failed: try the CPU before giving up.
            if state.retry_in_software(&info) {
                return false;
            }
            state.error = Some(VideoError::Decode(why));
            return false;
        }
        let picture = if state.show_first {
            session.take_first()
        } else if state.playing {
            session.take_due(position, 0.5 / info.fps)
        } else {
            None
        };
        let Some(picture) = picture else {
            // Nothing more to come: the end, once the playhead has caught up.
            if state.playing && session.drained() {
                state.ended = true;
                state.playing = false;
                state.since = None;
                state.base = state.duration().unwrap_or(position);
            }
            return false;
        };
        state.show_first = false;
        state.hardware_unproven = false;
        state.serial += 1;
        let Some(data) = NonNull::new(picture.pixels.as_ptr().cast_mut()) else {
            return false;
        };
        let texture = NativeTexture::Rgba8 {
            source: self.source,
            data,
            serial: state.serial,
        };
        let owner: Arc<dyn Any + Send + Sync> = picture;
        // SAFETY: `data` points at `size.0 * size.1 * 4` bytes of the picture,
        // which `owner` keeps alive and nothing ever writes again.
        let frame = unsafe { StoredFrame::new(texture, info.size.0, info.size.1, owner) };
        frames.store(Some(frame));
        true
    }
}
