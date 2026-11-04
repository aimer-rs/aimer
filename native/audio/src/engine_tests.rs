//! Engine behaviour against a fake backend: no audio hardware, no real time.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

use super::*;
use crate::PlaybackState;

#[derive(Default)]
struct VoiceState {
    duration: f64,
    position: f64,
    playing: bool,
    volume: f32,
    extra_loops: i64,
    plays: u32,
    from_bytes: bool,
    /// Opened from `VoiceData::Stream`.
    streamed: bool,
    buffering: bool,
    buffered: f64,
    error: Option<AudioError>,
    /// Opened like a player that reads its header asynchronously.
    opening: bool,
    /// Opened through `Backend::open_sound`.
    for_sound: bool,
    polls: u32,
    /// Every seek applied, in order.
    seeks: Vec<f64>,
    /// The engine dropped the voice.
    released: bool,
}

type VoiceRef = Rc<RefCell<VoiceState>>;

aimer_tls::thread_local! {
    static OPENED: RefCell<Vec<VoiceRef>> = const { RefCell::new(Vec::new()) };
    static FAIL_OPEN: Cell<bool> = const { Cell::new(false) };
    static FAIL_PLAY: Cell<bool> = const { Cell::new(false) };
    /// Voices open without knowing their length, like a player that opens
    /// asynchronously.
    static OPEN_ASYNC: Cell<bool> = const { Cell::new(false) };
    /// `prepare_now` handles every source, like the web backend.
    static PREPARE_NOW: Cell<bool> = const { Cell::new(false) };
}

fn voices() -> Vec<VoiceRef> {
    OPENED.with(|opened| opened.borrow().clone())
}

struct FakeVoice(VoiceRef);

impl Drop for FakeVoice {
    fn drop(&mut self) {
        self.0.borrow_mut().released = true;
    }
}

impl Voice for FakeVoice {
    fn duration(&self) -> f64 {
        self.0.borrow().duration
    }
    fn position(&self) -> f64 {
        self.0.borrow().position
    }
    fn is_playing(&self) -> bool {
        self.0.borrow().playing
    }
    fn play(&mut self) -> Result<(), AudioError> {
        if FAIL_PLAY.with(Cell::get) {
            return Err(AudioError::Playback("no output device".into()));
        }
        let mut state = self.0.borrow_mut();
        state.playing = true;
        state.plays += 1;
        Ok(())
    }
    fn pause(&mut self) {
        self.0.borrow_mut().playing = false;
    }
    fn stop(&mut self) {
        let mut state = self.0.borrow_mut();
        state.playing = false;
        state.position = 0.0;
    }
    fn seek(&mut self, seconds: f64) {
        let mut state = self.0.borrow_mut();
        state.position = seconds;
        state.seeks.push(seconds);
    }
    fn set_volume(&mut self, volume: f32) {
        self.0.borrow_mut().volume = volume;
    }
    fn set_extra_loops(&mut self, extra: i64) {
        self.0.borrow_mut().extra_loops = extra;
    }
    fn is_buffering(&self) -> bool {
        self.0.borrow().buffering
    }
    fn buffered(&self) -> f64 {
        let state = self.0.borrow();
        // Only a stream or a still-opening player has partial data.
        if state.streamed || state.opening { state.buffered } else { state.duration }
    }
    fn error(&self) -> Option<AudioError> {
        self.0.borrow().error.clone()
    }
    fn poll(&mut self) {
        self.0.borrow_mut().polls += 1;
    }
}

struct FakeBackend;

impl Backend for FakeBackend {
    fn new() -> Self {
        Self
    }

    fn open(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError> {
        fake_open(data, false)
    }

    fn open_sound(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError> {
        fake_open(data, true)
    }

    fn prepare_now(source: &AudioSource, _want_bytes: bool) -> Option<Result<VoiceData, AudioError>> {
        if !PREPARE_NOW.with(Cell::get) {
            return None;
        }
        Some(match source {
            AudioSource::File(_) => Err(AudioError::Unsupported),
            AudioSource::Bundled(key) => Ok(VoiceData::Stream(format!("/{key}"))),
            AudioSource::Url(url) | AudioSource::Stream(url) => Ok(VoiceData::Stream(url.clone())),
        })
    }

    fn resolve_bundled(key: &str, want_bytes: bool) -> Option<Result<VoiceData, AudioError>> {
        let key = key.strip_prefix("apk/")?;
        Some(Ok(if want_bytes {
            VoiceData::Bytes(key.as_bytes().into())
        } else {
            VoiceData::Asset(key.to_string())
        }))
    }

    fn fetch(url: &str) -> Result<Vec<u8>, AudioError> {
        if url == "https://example.test/ok.mp3" {
            Ok(vec![1, 2, 3])
        } else {
            Err(AudioError::Network("offline".into()))
        }
    }
}

fn fake_open(data: &VoiceData, for_sound: bool) -> Result<Box<dyn Voice>, AudioError> {
    if FAIL_OPEN.with(Cell::get) {
        return Err(AudioError::Decode("not audio".into()));
    }
    let streamed = matches!(data, VoiceData::Stream(_));
    let opening = OPEN_ASYNC.with(Cell::get);
    let state = Rc::new(RefCell::new(VoiceState {
        // A stream does not know its length until the first bytes arrive.
        duration: if streamed || opening { f64::NAN } else { 10.0 },
        volume: 1.0,
        from_bytes: matches!(data, VoiceData::Bytes(_)),
        streamed,
        opening,
        for_sound,
        buffering: streamed,
        ..VoiceState::default()
    }));
    OPENED.with(|opened| opened.borrow_mut().push(state.clone()));
    Ok(Box::new(FakeVoice(state)))
}

struct Rig {
    engine: Engine<FakeBackend>,
    rx: Receiver<Command>,
}

impl Rig {
    fn new() -> Self {
        OPENED.with(|opened| opened.borrow_mut().clear());
        FAIL_OPEN.with(|flag| flag.set(false));
        FAIL_PLAY.with(|flag| flag.set(false));
        OPEN_ASYNC.with(|flag| flag.set(false));
        PREPARE_NOW.with(|flag| flag.set(false));
        let (tx, rx) = mpsc::channel();
        Self {
            engine: Engine::new(FakeBackend, tx),
            rx,
        }
    }

    /// Applies the next result a helper thread sends back.
    fn pump(&mut self) {
        let command = self
            .rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a helper thread should report back");
        self.engine.handle(command);
    }

    fn track(&mut self, id: u64) -> Arc<Shared> {
        let shared = Shared::new();
        self.engine.handle(Command::NewTrack {
            id,
            shared: shared.clone(),
        });
        shared
    }

    fn sound(&mut self, id: u64) -> Arc<Shared> {
        let shared = Shared::new();
        self.engine.handle(Command::NewSound {
            id,
            shared: shared.clone(),
        });
        shared
    }

    fn load(&mut self, id: u64, source: AudioSource) {
        self.engine.handle(Command::Load { id, source });
    }

    /// A track with a file loaded and open.
    fn loaded_track(&mut self, id: u64) -> Arc<Shared> {
        let shared = self.track(id);
        shared.lock().begin_load();
        self.load(id, AudioSource::File(audio_file("track.mp3")));
        self.pump();
        shared
    }

    fn loaded_sound(&mut self, id: u64) -> Arc<Shared> {
        let shared = self.sound(id);
        shared.lock().begin_load();
        self.load(id, AudioSource::File(audio_file("sound.wav")));
        self.pump();
        shared
    }
}

/// A file that exists; the fake backend never reads it.
fn audio_file(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("aimer_audio_engine_{}_{name}", std::process::id()));
    std::fs::write(&path, b"fake").expect("temp file should be writable");
    path
}

#[test]
fn loading_a_file_opens_a_voice_by_path_and_publishes_the_duration() {
    let mut rig = Rig::new();
    let shared = rig.loaded_track(1);

    let snapshot = shared.lock();
    assert_eq!(snapshot.phase, Phase::Loaded);
    assert_eq!(snapshot.duration, Some(10.0));
    drop(snapshot);
    let opened = voices();
    assert_eq!(opened.len(), 1);
    assert!(!opened[0].borrow().from_bytes, "a track streams from disk");
}

#[test]
fn a_missing_file_fails_with_not_found_before_any_voice_opens() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::file("/definitely/not/here.mp3"));
    rig.pump();

    let snapshot = shared.lock();
    assert_eq!(snapshot.state(), PlaybackState::Failed);
    assert!(matches!(snapshot.error, Some(AudioError::NotFound(_))));
    assert!(voices().is_empty());
}

#[test]
fn a_bundled_key_that_resolves_nowhere_is_not_found() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::bundled("assets/no-such-file.mp3"));
    rig.pump();

    assert!(matches!(shared.lock().error, Some(AudioError::NotFound(_))));
}

#[test]
fn a_decode_failure_marks_the_track_failed() {
    let mut rig = Rig::new();
    FAIL_OPEN.with(|flag| flag.set(true));
    let shared = rig.track(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::File(audio_file("bad.mp3")));
    rig.pump();

    let snapshot = shared.lock();
    assert_eq!(snapshot.state(), PlaybackState::Failed);
    assert!(matches!(snapshot.error, Some(AudioError::Decode(_))));
}

#[test]
fn a_url_is_downloaded_and_opened_from_bytes_and_a_failed_download_is_reported() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::url("https://example.test/ok.mp3"));
    rig.pump();
    assert_eq!(shared.lock().phase, Phase::Loaded);
    assert!(voices()[0].borrow().from_bytes);

    shared.lock().begin_load();
    rig.load(1, AudioSource::url("https://example.test/gone.mp3"));
    rig.pump();
    assert!(matches!(shared.lock().error, Some(AudioError::Network(_))));
}

#[test]
fn play_requested_while_loading_starts_when_the_source_arrives() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::File(audio_file("track.mp3")));
    rig.engine.handle(Command::Play { id: 1 });
    assert!(voices().is_empty(), "nothing is open yet");

    rig.pump();

    assert!(voices()[0].borrow().playing);
    assert_eq!(shared.lock().phase, Phase::Loaded);
}

#[test]
fn a_superseded_load_is_ignored_when_its_result_arrives_late() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::File(audio_file("first.mp3")));
    rig.load(1, AudioSource::File(audio_file("second.mp3")));
    rig.pump();
    rig.pump();

    assert_eq!(voices().len(), 1, "only the newest load opens a voice");
    assert_eq!(shared.lock().phase, Phase::Loaded);
}

#[test]
fn pause_keeps_the_playhead_and_stop_rewinds_it() {
    let mut rig = Rig::new();
    rig.loaded_track(1);
    rig.engine.handle(Command::Play { id: 1 });
    let voice = voices()[0].clone();
    voice.borrow_mut().position = 3.0;

    rig.engine.handle(Command::Pause { id: 1 });
    assert!(!voice.borrow().playing);
    assert_eq!(voice.borrow().position, 3.0);

    rig.engine.handle(Command::Stop { id: 1 });
    assert_eq!(voice.borrow().position, 0.0);
}

#[test]
fn tick_publishes_the_playhead_only_while_a_track_plays() {
    let mut rig = Rig::new();
    let shared = rig.loaded_track(1);
    voices()[0].borrow_mut().position = 4.5;
    rig.engine.tick();
    assert_eq!(shared.lock().position, 0.0, "a track that never played is not polled");
    assert!(!rig.engine.needs_tick());

    rig.engine.handle(Command::Play { id: 1 });
    assert!(rig.engine.needs_tick());
    rig.engine.tick();
    assert_eq!(shared.lock().position, 4.5);
}

#[test]
fn reaching_the_end_finishes_the_track_and_play_restarts_it() {
    let mut rig = Rig::new();
    let shared = rig.loaded_track(1);
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });
    let voice = voices()[0].clone();

    voice.borrow_mut().playing = false; // the platform ran out of samples
    rig.engine.tick();

    let snapshot = shared.lock().clone();
    assert!(snapshot.finished);
    assert!(!snapshot.want_play);
    assert_eq!(snapshot.position, 10.0);
    assert_eq!(snapshot.state(), PlaybackState::Finished);
    assert!(!rig.engine.needs_tick());

    rig.engine.handle(Command::Play { id: 1 });
    assert_eq!(voices().len(), 2, "a track that ran out restarts on a fresh voice");
    let fresh = voices()[1].clone();
    assert!(fresh.borrow().playing);
    assert_eq!(fresh.borrow().position, 0.0, "a finished track restarts from the top");
    assert!(voice.borrow().released, "the spent voice was dropped");
}

#[test]
fn a_pause_is_never_mistaken_for_the_end() {
    let mut rig = Rig::new();
    let shared = rig.loaded_track(1);
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });

    shared.lock().want_play = false;
    rig.engine.handle(Command::Pause { id: 1 });
    rig.engine.tick();

    assert!(!shared.lock().finished);
}

#[test]
fn seeks_are_clamped_to_the_track_and_nan_goes_to_the_start() {
    let mut rig = Rig::new();
    rig.loaded_track(1);
    let voice = voices()[0].clone();

    rig.engine.handle(Command::Seek { id: 1, seconds: 99.0 });
    assert_eq!(voice.borrow().position, 10.0);
    rig.engine.handle(Command::Seek { id: 1, seconds: -5.0 });
    assert_eq!(voice.borrow().position, 0.0);
    rig.engine.handle(Command::Seek { id: 1, seconds: 4.0 });
    rig.engine.handle(Command::Seek { id: 1, seconds: f64::NAN });
    assert_eq!(voice.borrow().position, 0.0);
}

#[test]
fn a_seek_before_the_source_opens_is_applied_after_it_does() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::File(audio_file("track.mp3")));
    rig.engine.handle(Command::Seek { id: 1, seconds: 4.0 });
    rig.pump();

    assert_eq!(voices()[0].borrow().position, 4.0);
}

#[test]
fn volume_and_loops_set_before_a_load_reach_the_voice_that_opens() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    rig.engine.handle(Command::SetVolume { id: 1, volume: 0.3 });
    rig.engine.handle(Command::SetLoops {
        id: 1,
        loops: AudioLoops::Infinite,
    });
    shared.lock().begin_load();
    rig.load(1, AudioSource::File(audio_file("track.mp3")));
    rig.pump();

    let voice = voices()[0].clone();
    assert_eq!(voice.borrow().volume, 0.3);
    assert_eq!(voice.borrow().extra_loops, -1);

    rig.engine.handle(Command::SetVolume { id: 1, volume: 0.9 });
    rig.engine.handle(Command::SetLoops {
        id: 1,
        loops: AudioLoops::Count(3),
    });
    assert_eq!(voice.borrow().volume, 0.9);
    assert_eq!(
        voice.borrow().extra_loops, 0,
        "finite repeats are counted by the engine, not delegated"
    );
    rig.engine.handle(Command::SetLoops {
        id: 1,
        loops: AudioLoops::Infinite,
    });
    assert_eq!(voice.borrow().extra_loops, -1, "forever stays native and gapless");
}

#[test]
fn a_finite_loop_restarts_on_a_fresh_voice_at_each_end_and_finishes_after_the_last_pass() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    rig.engine.handle(Command::SetLoops {
        id: 1,
        loops: AudioLoops::Count(3),
    });
    shared.lock().begin_load();
    rig.load(1, AudioSource::File(audio_file("track.mp3")));
    rig.pump();
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });
    voices()[0].borrow_mut().volume = 0.5; // a volume set after opening

    for pass in 2..=3 {
        voices().last().unwrap().borrow_mut().playing = false; // a pass ran out
        rig.engine.tick();
        let opened = voices();
        assert_eq!(opened.len(), pass, "pass {pass} opened a fresh voice");
        let voice = opened.last().unwrap().borrow();
        assert!(voice.playing, "pass {pass} started");
        assert_eq!(voice.position, 0.0, "pass {pass} starts from the top");
        assert!(!shared.lock().finished);
        assert!(rig.engine.needs_tick());
    }
    assert!(voices()[0].borrow().released && voices()[1].borrow().released);

    voices().last().unwrap().borrow_mut().playing = false; // the last pass
    rig.engine.tick();
    assert_eq!(voices().len(), 3, "no fourth pass");
    assert!(shared.lock().finished);

    // A fresh run after the end gets the full count again.
    shared.lock().want_play = true;
    shared.lock().finished = false;
    rig.engine.handle(Command::Play { id: 1 });
    assert_eq!(voices().len(), 4, "play after the end opens a fresh voice");
    voices().last().unwrap().borrow_mut().playing = false;
    rig.engine.tick();
    assert_eq!(voices().len(), 5, "and the repeat count was restored");
}

#[test]
fn pausing_midway_keeps_the_remaining_repeats_and_stop_restores_them() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    rig.engine.handle(Command::SetLoops {
        id: 1,
        loops: AudioLoops::Count(2),
    });
    shared.lock().begin_load();
    rig.load(1, AudioSource::File(audio_file("track.mp3")));
    rig.pump();
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });

    voices().last().unwrap().borrow_mut().playing = false;
    rig.engine.tick(); // pass 2 starts; no repeats left
    assert_eq!(voices().len(), 2);
    rig.engine.handle(Command::Pause { id: 1 });
    rig.engine.handle(Command::Play { id: 1 });
    assert_eq!(voices().len(), 2, "resuming a pause reuses the voice");
    voices().last().unwrap().borrow_mut().playing = false;
    rig.engine.tick();
    assert!(shared.lock().finished, "pausing did not grant extra repeats");

    rig.engine.handle(Command::Stop { id: 1 });
    shared.lock().want_play = true;
    shared.lock().finished = false;
    rig.engine.handle(Command::Play { id: 1 });
    voices().last().unwrap().borrow_mut().playing = false;
    rig.engine.tick();
    assert!(
        voices().last().unwrap().borrow().playing,
        "stop restored the repeat"
    );
}

#[test]
fn a_fresh_voice_after_the_end_keeps_the_volume_and_native_loop_setting() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    rig.engine.handle(Command::SetVolume { id: 1, volume: 0.4 });
    shared.lock().begin_load();
    rig.load(1, AudioSource::File(audio_file("track.mp3")));
    rig.pump();
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });
    voices()[0].borrow_mut().playing = false;
    rig.engine.tick(); // finished
    rig.engine.handle(Command::Play { id: 1 });

    let fresh = voices()[1].clone();
    assert_eq!(fresh.borrow().volume, 0.4);
    assert_eq!(fresh.borrow().extra_loops, 0);
}

#[test]
fn a_refused_play_marks_the_track_failed() {
    let mut rig = Rig::new();
    let shared = rig.loaded_track(1);
    FAIL_PLAY.with(|flag| flag.set(true));
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });

    let snapshot = shared.lock();
    assert_eq!(snapshot.state(), PlaybackState::Failed);
    assert!(matches!(snapshot.error, Some(AudioError::Playback(_))));
    drop(snapshot);
    assert!(!rig.engine.needs_tick());
}

#[test]
fn releasing_a_track_drops_its_voice_and_later_commands_are_ignored() {
    let mut rig = Rig::new();
    rig.loaded_track(1);
    let voice = voices()[0].clone();
    assert!(!voice.borrow().released);

    rig.engine.handle(Command::Release { id: 1 });
    assert!(voice.borrow().released, "the engine let go of the voice");

    rig.engine.handle(Command::Play { id: 1 });
    rig.engine.handle(Command::Seek { id: 1, seconds: 1.0 });
    rig.engine.tick();
    assert!(!rig.engine.needs_tick());
}

#[test]
fn a_sound_loaded_for_overlap_is_read_into_memory() {
    let mut rig = Rig::new();
    let shared = rig.loaded_sound(1);

    assert_eq!(shared.lock().phase, Phase::Loaded);
    assert!(voices()[0].borrow().from_bytes);
}

#[test]
fn overlapping_triggers_open_more_voices_and_play_them_together() {
    let mut rig = Rig::new();
    rig.loaded_sound(1);
    for _ in 0..3 {
        rig.engine.handle(Command::Play { id: 1 });
    }

    let opened = voices();
    assert_eq!(opened.len(), 3);
    assert!(opened.iter().all(|voice| voice.borrow().playing));
}

#[test]
fn a_finished_voice_is_reused_instead_of_opening_another() {
    let mut rig = Rig::new();
    rig.loaded_sound(1);
    rig.engine.handle(Command::Play { id: 1 });
    voices()[0].borrow_mut().playing = false;
    rig.engine.handle(Command::Play { id: 1 });

    assert_eq!(voices().len(), 1);
    assert_eq!(voices()[0].borrow().plays, 2);
}

#[test]
fn the_pool_is_capped_and_a_surplus_trigger_restarts_the_oldest_voice() {
    let mut rig = Rig::new();
    rig.loaded_sound(1);
    for _ in 0..MAX_VOICES + 2 {
        rig.engine.handle(Command::Play { id: 1 });
    }

    let opened = voices();
    assert_eq!(opened.len(), MAX_VOICES);
    assert_eq!(opened[0].borrow().plays, 2, "the first surplus trigger took voice 0");
    assert_eq!(opened[1].borrow().plays, 2, "the second took voice 1");
    assert_eq!(opened[2].borrow().plays, 1);
}

#[test]
fn a_trigger_before_the_data_arrives_plays_exactly_once_on_arrival() {
    let mut rig = Rig::new();
    let shared = rig.sound(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::File(audio_file("sound.wav")));
    rig.engine.handle(Command::Play { id: 1 });
    rig.engine.handle(Command::Play { id: 1 });
    rig.pump();

    let opened = voices();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].borrow().plays, 1);
}

#[test]
fn stopping_a_sound_silences_every_instance_and_cancels_a_pending_trigger() {
    let mut rig = Rig::new();
    rig.loaded_sound(1);
    for _ in 0..3 {
        rig.engine.handle(Command::Play { id: 1 });
    }
    rig.engine.handle(Command::Stop { id: 1 });
    assert!(voices().iter().all(|voice| !voice.borrow().playing));

    let mut rig = Rig::new();
    let shared = rig.sound(2);
    shared.lock().begin_load();
    rig.load(2, AudioSource::File(audio_file("sound.wav")));
    rig.engine.handle(Command::Play { id: 2 });
    rig.engine.handle(Command::Stop { id: 2 });
    rig.pump();
    assert_eq!(voices()[0].borrow().plays, 0);
}

#[test]
fn reloading_a_sound_discards_its_old_voices() {
    let mut rig = Rig::new();
    let shared = rig.loaded_sound(1);
    let old = voices()[0].clone();
    shared.lock().begin_load();
    rig.load(1, AudioSource::File(audio_file("other.wav")));
    assert!(old.borrow().released, "the old pool was dropped");
    rig.pump();
    assert_eq!(voices().len(), 2);
}

#[test]
fn bundled_assets_are_searched_in_the_working_directory_then_the_bundle() {
    let exe = std::path::Path::new("/Apps/Jaime.app/Contents/MacOS/jaime");
    assert_eq!(
        bundled_candidates("assets/a.mp3", Some(exe)),
        [
            PathBuf::from("assets/a.mp3"),
            PathBuf::from("/Apps/Jaime.app/Contents/Resources/assets/a.mp3"),
            PathBuf::from("/Apps/Jaime.app/Contents/MacOS/assets/a.mp3"),
        ]
    );
    assert_eq!(
        bundled_candidates("assets/a.mp3", None),
        [PathBuf::from("assets/a.mp3")]
    );
}

#[test]
fn seconds_clamp_into_the_track() {
    assert_eq!(clamp_seconds(3.0, 10.0), 3.0);
    assert_eq!(clamp_seconds(11.0, 10.0), 10.0);
    assert_eq!(clamp_seconds(-1.0, 10.0), 0.0);
    assert_eq!(clamp_seconds(f64::NAN, 10.0), 0.0);
    assert_eq!(clamp_seconds(f64::INFINITY, 10.0), 10.0);
    assert_eq!(clamp_seconds(5.0, 0.0), 0.0);
}

const STREAM_URL: &str = "https://example.test/live.mp3";

impl Rig {
    /// A track opened on a stream whose length is not known yet.
    fn streaming_track(&mut self, id: u64) -> Arc<Shared> {
        let shared = self.track(id);
        shared.lock().begin_load();
        self.load(id, AudioSource::stream(STREAM_URL));
        self.pump();
        shared
    }
}

/// The stream reports its first bytes: a length and some buffered audio.
fn stream_becomes_playable(voice: &VoiceRef, duration: f64, buffered: f64) {
    let mut voice = voice.borrow_mut();
    voice.duration = duration;
    voice.buffered = buffered;
    voice.buffering = false;
}

#[test]
fn a_stream_opens_without_a_download_and_its_length_is_not_invented() {
    let mut rig = Rig::new();
    let shared = rig.streaming_track(1);

    let snapshot = shared.lock();
    assert_eq!(snapshot.phase, Phase::Loaded);
    assert_eq!(snapshot.duration, None, "an unknown length stays unknown");
    assert!(snapshot.buffering);
    drop(snapshot);
    let opened = voices();
    assert_eq!(opened.len(), 1);
    assert!(opened[0].borrow().streamed && !opened[0].borrow().from_bytes);
}

#[test]
fn a_stream_url_that_is_not_http_fails_before_any_voice_opens() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::stream("ftp://example.test/a.mp3"));
    rig.pump();

    assert!(matches!(shared.lock().error, Some(AudioError::Network(_))));
    assert!(voices().is_empty());
}

#[test]
fn a_playing_stream_reads_buffering_until_the_first_bytes_arrive() {
    let mut rig = Rig::new();
    let shared = rig.streaming_track(1);
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });
    assert_eq!(shared.lock().state(), PlaybackState::Buffering);
    assert!(rig.engine.needs_tick(), "buffering has to be watched");

    rig.engine.tick();
    assert_eq!(shared.lock().state(), PlaybackState::Buffering);

    stream_becomes_playable(&voices()[0], 30.0, 4.0);
    rig.engine.tick();
    let snapshot = shared.lock();
    assert_eq!(snapshot.state(), PlaybackState::Playing);
    assert_eq!(snapshot.duration, Some(30.0));
    assert_eq!(snapshot.buffered, 4.0);
}

#[test]
fn a_paused_stream_keeps_ticking_until_it_is_fully_buffered() {
    let mut rig = Rig::new();
    let shared = rig.streaming_track(1);
    assert!(rig.engine.needs_tick(), "the length is still unknown");

    stream_becomes_playable(&voices()[0], 30.0, 10.0);
    rig.engine.tick();
    assert_eq!(shared.lock().duration, Some(30.0));
    assert!(rig.engine.needs_tick(), "20 s are still to download");

    voices()[0].borrow_mut().buffered = 30.0;
    rig.engine.tick();
    assert_eq!(shared.lock().buffered, 30.0);
    assert!(!rig.engine.needs_tick(), "nothing left to watch while paused");
}

#[test]
fn a_stream_that_fails_midway_reports_the_error_instead_of_finishing() {
    let mut rig = Rig::new();
    let shared = rig.streaming_track(1);
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });

    {
        let voice = &voices()[0];
        let mut voice = voice.borrow_mut();
        voice.playing = false; // the player stops when the connection dies
        voice.error = Some(AudioError::Network("connection lost".into()));
    }
    rig.engine.tick();

    let snapshot = shared.lock();
    assert_eq!(snapshot.state(), PlaybackState::Failed);
    assert_eq!(snapshot.error, Some(AudioError::Network("connection lost".into())));
    assert!(!snapshot.finished);
}

#[test]
fn a_paused_stream_that_fails_is_reported_too() {
    let mut rig = Rig::new();
    let shared = rig.streaming_track(1);
    voices()[0].borrow_mut().error = Some(AudioError::Network("404".into()));
    rig.engine.tick();

    assert_eq!(shared.lock().state(), PlaybackState::Failed);
}

#[test]
fn a_seek_on_a_stream_of_unknown_length_is_not_clamped_to_zero() {
    let mut rig = Rig::new();
    let _shared = rig.streaming_track(1);
    rig.engine.handle(Command::Seek { id: 1, seconds: 42.0 });

    assert_eq!(voices()[0].borrow().position, 42.0);
}

#[test]
fn a_looping_stream_rewinds_the_same_voice_instead_of_downloading_again() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    rig.engine.handle(Command::SetLoops {
        id: 1,
        loops: AudioLoops::Count(2),
    });
    shared.lock().begin_load();
    rig.load(1, AudioSource::stream(STREAM_URL));
    rig.pump();
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });
    stream_becomes_playable(&voices()[0], 10.0, 10.0);

    voices()[0].borrow_mut().position = 10.0;
    voices()[0].borrow_mut().playing = false; // the first pass ran out
    rig.engine.tick();

    assert_eq!(voices().len(), 1, "no second connection");
    let voice = voices()[0].clone();
    assert!(voice.borrow().playing);
    assert_eq!(voice.borrow().position, 0.0);
    assert_eq!(voice.borrow().plays, 2);
    assert!(!shared.lock().finished);

    voices()[0].borrow_mut().playing = false; // the last pass
    rig.engine.tick();
    assert!(shared.lock().finished);
}

#[test]
fn an_infinite_stream_loop_is_driven_by_the_engine_not_the_platform() {
    let mut rig = Rig::new();
    let shared = rig.track(1);
    rig.engine.handle(Command::SetLoops {
        id: 1,
        loops: AudioLoops::Infinite,
    });
    shared.lock().begin_load();
    rig.load(1, AudioSource::stream(STREAM_URL));
    rig.pump();
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });
    stream_becomes_playable(&voices()[0], 10.0, 10.0);

    assert_eq!(voices()[0].borrow().extra_loops, 0, "a player has no native loop for a stream");
    for round in 2..=4 {
        voices()[0].borrow_mut().playing = false;
        rig.engine.tick();
        assert_eq!(voices()[0].borrow().plays, round);
    }
    assert!(!shared.lock().finished);
}

#[test]
fn a_sound_cannot_stream_so_a_stream_source_is_downloaded_for_it() {
    let mut rig = Rig::new();
    let shared = rig.sound(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::stream("https://example.test/ok.mp3"));
    rig.pump();

    assert_eq!(shared.lock().phase, Phase::Loaded);
    assert!(voices()[0].borrow().from_bytes);
}

#[test]
fn a_track_that_opens_asynchronously_is_watched_until_its_length_is_known() {
    let mut rig = Rig::new();
    OPEN_ASYNC.with(|flag| flag.set(true));
    let shared = rig.loaded_track(1);
    assert_eq!(shared.lock().duration, None);
    assert!(rig.engine.needs_tick(), "a paused track still has to learn its length");

    stream_becomes_playable(&voices()[0], 12.0, 12.0);
    rig.engine.tick();
    assert_eq!(shared.lock().duration, Some(12.0));
    assert!(!rig.engine.needs_tick());
}

#[test]
fn every_tick_polls_the_voice_before_reading_it() {
    let mut rig = Rig::new();
    OPEN_ASYNC.with(|flag| flag.set(true));
    let _shared = rig.loaded_track(1);
    rig.engine.tick();
    rig.engine.tick();
    assert_eq!(voices()[0].borrow().polls, 2);
}

#[test]
fn a_local_track_that_fails_while_playing_reports_the_error_instead_of_finishing() {
    let mut rig = Rig::new();
    let shared = rig.loaded_track(1);
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });
    {
        let voice = &voices()[0];
        let mut voice = voice.borrow_mut();
        voice.playing = false;
        voice.error = Some(AudioError::Decode("corrupt frame".into()));
    }
    rig.engine.tick();

    let snapshot = shared.lock();
    assert_eq!(snapshot.state(), PlaybackState::Failed);
    assert!(!snapshot.finished);
    drop(snapshot);
    assert!(voices()[0].borrow().released);
    assert!(!rig.engine.needs_tick());
}

#[test]
fn the_end_of_a_track_of_unknown_length_keeps_the_playhead_where_it_stopped() {
    let mut rig = Rig::new();
    let shared = rig.streaming_track(1);
    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });
    {
        let voice = &voices()[0];
        let mut voice = voice.borrow_mut();
        voice.buffering = false;
        voice.position = 7.5;
        voice.playing = false; // a live stream that ended
    }
    rig.engine.tick();

    let snapshot = shared.lock();
    assert!(snapshot.finished);
    assert_eq!(snapshot.position, 7.5);
}

#[test]
fn sounds_open_through_open_sound_and_tracks_through_open() {
    let mut rig = Rig::new();
    let track = rig.loaded_track(1);
    let sound = rig.loaded_sound(2);
    rig.engine.handle(Command::Play { id: 2 });
    rig.engine.handle(Command::Play { id: 2 });
    drop((track, sound));

    let opened = voices();
    assert!(!opened[0].borrow().for_sound);
    assert!(opened[1..].iter().all(|voice| voice.borrow().for_sound));
    assert_eq!(opened.len(), 3, "the overlapping trigger opened a second sound voice");
}

#[test]
fn a_backend_that_prepares_in_place_needs_no_helper_thread() {
    let mut rig = Rig::new();
    PREPARE_NOW.with(|flag| flag.set(true));
    let shared = rig.track(1);
    shared.lock().begin_load();
    rig.load(1, AudioSource::bundled("assets/theme.mp3"));

    // No `pump`: the load finished inside `handle`.
    assert_eq!(shared.lock().phase, Phase::Loaded);
    assert!(rig.rx.try_recv().is_err(), "nothing was sent back by a helper thread");
    assert!(voices()[0].borrow().streamed);

    rig.load(1, AudioSource::file("/a.mp3"));
    assert_eq!(shared.lock().error, Some(AudioError::Unsupported));
}

#[test]
fn a_backend_can_resolve_bundled_assets_that_are_not_files() {
    let mut rig = Rig::new();
    let track = rig.track(1);
    track.lock().begin_load();
    rig.load(1, AudioSource::bundled("apk/theme.mp3"));
    rig.pump();
    let sound = rig.sound(2);
    sound.lock().begin_load();
    rig.load(2, AudioSource::bundled("apk/tap.wav"));
    rig.pump();

    assert_eq!(track.lock().phase, Phase::Loaded);
    assert_eq!(sound.lock().phase, Phase::Loaded);
    let opened = voices();
    assert!(!opened[0].borrow().from_bytes, "a track opens the asset by key");
    assert!(opened[1].borrow().from_bytes, "a sound reads the asset into memory");
}

#[test]
fn a_stream_that_cannot_report_its_buffer_settles_once_its_length_is_known() {
    let mut rig = Rig::new();
    let shared = rig.streaming_track(1);
    stream_becomes_playable(&voices()[0], 30.0, f64::NAN);
    voices()[0].borrow_mut().position = 3.0;
    rig.engine.tick();

    assert_eq!(shared.lock().duration, Some(30.0));
    assert_eq!(shared.lock().buffered, 3.0, "unknown: only what has played is certain");
    assert!(!rig.engine.needs_tick(), "nothing more will be learned while paused");

    shared.lock().want_play = true;
    rig.engine.handle(Command::Play { id: 1 });
    voices()[0].borrow_mut().position = 5.0;
    rig.engine.tick();
    assert_eq!(shared.lock().buffered, 5.0, "a playing track keeps publishing it");
}

#[test]
fn queued_seeks_of_one_track_collapse_into_the_latest() {
    let mut rig = Rig::new();
    let _first = rig.loaded_track(1);
    let _second = rig.loaded_track(2);
    let (tx, rx) = mpsc::channel();
    // A scrub: many seeks of track 1 queue up while the engine is busy.
    for seconds in [1.0, 2.0, 3.0, 4.0, 5.0] {
        tx.send(Command::Seek { id: 1, seconds }).unwrap();
    }
    tx.send(Command::Seek { id: 2, seconds: 7.0 }).unwrap();
    tx.send(Command::Play { id: 1 }).unwrap();
    tx.send(Command::Seek { id: 1, seconds: 6.0 }).unwrap();
    tx.send(Command::Seek { id: 1, seconds: 8.0 }).unwrap();

    rig.engine.handle_pending(None, &rx);

    let opened = voices();
    assert_eq!(opened[0].borrow().seeks, vec![5.0, 8.0], "one seek per run of seeks");
    assert_eq!(opened[1].borrow().seeks, vec![7.0]);
    assert!(opened[0].borrow().playing, "the play between them still applied");
    assert_eq!(opened[0].borrow().plays, 1);
}

#[test]
fn a_seek_is_never_moved_past_another_command_of_its_track() {
    let mut rig = Rig::new();
    let _shared = rig.loaded_track(1);
    let (tx, rx) = mpsc::channel();
    tx.send(Command::Seek { id: 1, seconds: 2.0 }).unwrap();
    tx.send(Command::Stop { id: 1 }).unwrap();

    rig.engine.handle_pending(Some(Command::Seek { id: 1, seconds: 1.0 }), &rx);

    let voice = voices()[0].clone();
    assert_eq!(voice.borrow().seeks, vec![2.0], "the stop came after the seek");
    assert_eq!(voice.borrow().position, 0.0, "and the stop rewound it");
}
