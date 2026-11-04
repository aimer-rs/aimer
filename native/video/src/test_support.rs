//! A fake platform player for tests: no decoder, no GPU, no real time.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use aimer_cupid::external_texture::NativeTexture;

use crate::backend::{FrameSlot, PlatformVideo, StoredFrame};
use crate::{VideoError, VideoPlayer, VideoSource};

#[derive(Default)]
pub(crate) struct FakeState {
    pub(crate) ready: bool,
    pub(crate) playing: bool,
    pub(crate) buffering: bool,
    pub(crate) position: f64,
    pub(crate) duration: f64,
    pub(crate) volume: f32,
    pub(crate) error: Option<VideoError>,
    pub(crate) seeks: Vec<f64>,
    pub(crate) plays: u32,
    /// Frames waiting to be taken, by id.
    pub(crate) pending_frames: Vec<u32>,
    pub(crate) shown_frame: Option<u32>,
    /// Every call into the platform, to prove an idle player makes none.
    pub(crate) calls: u32,
}

pub(crate) type FakeRef = Arc<Mutex<FakeState>>;

aimer_tls::thread_local! {
    static OPENED: RefCell<Vec<FakeRef>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn opened() -> Vec<FakeRef> {
    OPENED.with(|opened| opened.borrow().clone())
}

pub(crate) fn last() -> FakeRef {
    opened().last().cloned().expect("a fake was opened")
}

struct Fake(FakeRef);

impl PlatformVideo for Fake {
    fn play(&self) {
        self.0.lock().unwrap().calls += 1;
        let mut state = self.0.lock().unwrap();
        state.playing = true;
        state.plays += 1;
    }
    fn pause(&self) {
        self.0.lock().unwrap().calls += 1;
        self.0.lock().unwrap().playing = false;
    }
    fn seek(&self, seconds: f64) {
        self.0.lock().unwrap().calls += 1;
        let mut state = self.0.lock().unwrap();
        state.position = seconds;
        state.seeks.push(seconds);
    }
    fn set_volume(&self, volume: f32) {
        self.0.lock().unwrap().calls += 1;
        self.0.lock().unwrap().volume = volume;
    }
    fn is_ready(&self) -> bool {
        self.0.lock().unwrap().calls += 1;
        self.0.lock().unwrap().ready
    }
    fn position(&self) -> f64 {
        self.0.lock().unwrap().calls += 1;
        self.0.lock().unwrap().position
    }
    fn duration(&self) -> f64 {
        self.0.lock().unwrap().calls += 1;
        self.0.lock().unwrap().duration
    }
    fn buffered(&self) -> f64 {
        self.0.lock().unwrap().calls += 1;
        self.0.lock().unwrap().duration
    }
    fn is_playing(&self) -> bool {
        self.0.lock().unwrap().calls += 1;
        self.0.lock().unwrap().playing
    }
    fn is_buffering(&self) -> bool {
        self.0.lock().unwrap().calls += 1;
        self.0.lock().unwrap().buffering
    }
    fn error(&self) -> Option<VideoError> {
        self.0.lock().unwrap().calls += 1;
        self.0.lock().unwrap().error.clone()
    }
    fn size(&self) -> Option<(u32, u32)> {
        self.0.lock().unwrap().calls += 1;
        self.0.lock().unwrap().ready.then_some((1920, 1080))
    }
    fn take_frame(&self, frames: &FrameSlot) -> bool {
        self.0.lock().unwrap().calls += 1;
        let mut state = self.0.lock().unwrap();
        if state.pending_frames.is_empty() {
            return false;
        }
        let id = state.pending_frames.remove(0);
        state.shown_frame = Some(id);
        // SAFETY: the tests never dereference the texture handle.
        let frame = unsafe {
            StoredFrame::new(NativeTexture::Metal(NonNull::dangling()), 1920, 1080, Arc::new(id))
        };
        frames.store(Some(frame));
        true
    }
}

pub(crate) fn fake_open(source: &VideoSource) -> Result<Box<dyn PlatformVideo>, VideoError> {
    if matches!(source, VideoSource::File(path) if path.ends_with("missing.mp4")) {
        return Err(VideoError::NotFound("missing.mp4".into()));
    }
    let state = Arc::new(Mutex::new(FakeState {
        duration: f64::NAN,
        volume: 1.0,
        ..FakeState::default()
    }));
    OPENED.with(|opened| opened.borrow_mut().push(state.clone()));
    Ok(Box::new(Fake(state)))
}

pub(crate) fn player() -> VideoPlayer {
    OPENED.with(|opened| opened.borrow_mut().clear());
    VideoPlayer::with_platform(fake_open)
}

/// A player with a ten-second clip open.
pub(crate) fn ready_player() -> VideoPlayer {
    let player = player();
    player.load(VideoSource::file("clip.mp4"));
    let fake = last();
    let mut state = fake.lock().unwrap();
    state.ready = true;
    state.duration = 10.0;
    drop(state);
    player
}

