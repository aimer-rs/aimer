//! Public handles against the real process-wide engine.
//!
//! Every file is silent and played at volume zero, so the suite makes no sound.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{AudioError, AudioLoops, AudioPlayer, AudioSource, PlaybackState, Sound};

/// Mono, 16-bit, 8 kHz PCM silence of the given length.
fn silent_wav(seconds: f32) -> Vec<u8> {
    const RATE: u32 = 8000;
    let samples = (RATE as f32 * seconds) as u32;
    let data_len = samples * 2;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1_u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2_u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16_u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(44 + data_len as usize, 0);
    wav
}

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("aimer_audio_{}_{name}", std::process::id()));
    std::fs::write(&path, bytes).expect("temp file should be writable");
    path
}

/// How long a test waits for the engine. Generous because the first player of
/// a process starts Core Audio, which takes seconds when a test runner starts
/// many processes at once.
const WAIT: Duration = Duration::from_secs(30);

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_loaded(player: &AudioPlayer) {
    wait_until("the track to load", || player.state() != PlaybackState::Loading);
}

#[test]
fn a_missing_file_fails_with_not_found_on_every_platform() {
    let player = AudioPlayer::from_source(AudioSource::file("/definitely/not/here.mp3"));
    wait_loaded(&player);

    assert_eq!(player.state(), PlaybackState::Failed);
    assert!(matches!(player.error(), Some(AudioError::NotFound(_))));
    player.play();
    assert_eq!(player.state(), PlaybackState::Failed, "play after failure is a no-op");
}

#[test]
fn controls_before_a_load_do_nothing() {
    let player = AudioPlayer::new();
    player.play();
    player.seek(Duration::from_secs(3));
    assert_eq!(player.state(), PlaybackState::Idle);
    assert_eq!(player.duration(), None);

    let sound = Sound::new();
    sound.play();
    assert_eq!(sound.state(), PlaybackState::Idle);
}

#[test]
fn volume_is_clamped_nan_is_ignored_and_mute_keeps_the_volume() {
    let player = AudioPlayer::new();
    player.set_volume(3.0);
    assert_eq!(player.volume(), 1.0);
    player.set_volume(-1.0);
    assert_eq!(player.volume(), 0.0);
    player.set_volume(0.25);
    player.set_volume(f32::NAN);
    assert_eq!(player.volume(), 0.25);
    player.set_muted(true);
    assert!(player.is_muted());
    assert_eq!(player.volume(), 0.25);

    let sound = Sound::new();
    sound.set_volume(7.0);
    assert_eq!(sound.volume(), 1.0);
}

#[test]
fn clones_control_the_same_track() {
    let player = AudioPlayer::new();
    let copy = player.clone();
    copy.set_loops(AudioLoops::Count(4));
    assert_eq!(player.loops(), AudioLoops::Count(4));
}

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
#[test]
fn platforms_without_a_backend_report_unsupported() {
    assert!(!crate::is_supported());
    let path = temp_file("unsupported.wav", &silent_wav(0.2));
    let player = AudioPlayer::from_source(AudioSource::File(path));
    wait_loaded(&player);
    assert_eq!(player.error(), Some(AudioError::Unsupported));
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple {
    use super::*;

    fn silent_player(name: &str, seconds: f32) -> AudioPlayer {
        let path = temp_file(name, &silent_wav(seconds));
        let player = AudioPlayer::new();
        player.set_volume(0.0);
        player.load(AudioSource::File(path));
        wait_loaded(&player);
        player
    }

    #[test]
    fn a_wav_file_loads_with_its_real_duration() {
        assert!(crate::is_supported());
        let player = silent_player("duration.wav", 1.0);

        assert_eq!(player.state(), PlaybackState::Ready, "{:?}", player.error());
        let duration = player.duration().expect("a loaded track has a duration");
        assert!((duration.as_secs_f64() - 1.0).abs() < 0.05, "{duration:?}");
    }

    #[test]
    fn play_pause_seek_and_stop_move_the_playhead() {
        let player = silent_player("controls.wav", 2.0);

        player.play();
        assert_eq!(player.state(), PlaybackState::Playing);
        wait_until("the playhead to advance", || {
            player.position() > Duration::from_millis(80)
        });

        player.pause();
        assert_eq!(player.state(), PlaybackState::Paused);
        std::thread::sleep(Duration::from_millis(100));
        let paused_at = player.position();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(player.position(), paused_at, "a paused track does not move");

        player.seek(Duration::from_millis(1500));
        assert_eq!(player.state(), PlaybackState::Paused, "seeking does not resume");
        assert_eq!(player.position(), Duration::from_millis(1500));

        player.stop();
        assert_eq!(player.state(), PlaybackState::Ready);
        assert_eq!(player.position(), Duration::ZERO);
    }

    #[test]
    fn a_seek_past_the_end_is_clamped_to_the_duration() {
        let player = silent_player("clamp.wav", 1.0);
        let duration = player.duration().unwrap();
        player.seek(Duration::from_secs(60));
        assert_eq!(player.position(), duration);
    }

    #[test]
    fn a_track_plays_to_the_end_and_play_restarts_it() {
        let player = silent_player("end.wav", 0.3);
        player.play();
        wait_until("the track to finish", || {
            player.state() == PlaybackState::Finished
        });
        assert!(!player.is_playing());
        assert_eq!(Some(player.position()), player.duration());

        player.play();
        assert_eq!(player.state(), PlaybackState::Playing);
        // It really plays again: the playhead moves, rather than the track
        // falling straight back to `Finished`.
        wait_until("the restarted playhead to move", || {
            player.position() > Duration::from_millis(60)
        });
        assert_eq!(player.state(), PlaybackState::Playing);
        wait_until("the restarted track to finish", || {
            player.state() == PlaybackState::Finished
        });
    }

    #[test]
    fn a_loop_count_plays_the_track_that_many_times() {
        let player = silent_player("loops.wav", 0.3);
        player.set_loops(AudioLoops::Count(2));
        let started = Instant::now();
        player.play();
        wait_until("both plays to finish", || {
            player.state() == PlaybackState::Finished
        });
        assert!(
            started.elapsed() >= Duration::from_millis(500),
            "two 0.3 s plays took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn play_called_while_loading_starts_once_the_file_is_ready() {
        let path = temp_file("autoplay.wav", &silent_wav(0.3));
        let player = AudioPlayer::new();
        player.set_volume(0.0);
        player.load(AudioSource::File(path));
        player.play(); // still loading
        assert_eq!(player.state(), PlaybackState::Playing);
        wait_until("the track to finish", || {
            player.state() == PlaybackState::Finished
        });
    }

    #[test]
    fn a_file_that_is_not_audio_fails_with_a_decode_error() {
        let path = temp_file("garbage.wav", b"this is not audio at all");
        let player = AudioPlayer::from_source(AudioSource::File(path));
        wait_loaded(&player);

        assert_eq!(player.state(), PlaybackState::Failed);
        assert!(matches!(player.error(), Some(AudioError::Decode(_))), "{:?}", player.error());
    }

    #[test]
    fn loading_again_replaces_the_track_and_resets_the_playhead() {
        let player = silent_player("first.wav", 2.0);
        player.seek(Duration::from_secs(1));

        let second = temp_file("second.wav", &silent_wav(0.5));
        player.load(AudioSource::File(second));
        assert_eq!(player.state(), PlaybackState::Loading);
        wait_loaded(&player);

        assert_eq!(player.position(), Duration::ZERO);
        let duration = player.duration().unwrap().as_secs_f64();
        assert!((duration - 0.5).abs() < 0.05, "{duration}");
    }

    #[test]
    fn a_sound_loads_and_overlapping_triggers_do_not_disturb_it() {
        let path = temp_file("effect.wav", &silent_wav(0.4));
        let sound = Sound::new();
        sound.set_volume(0.0);
        sound.load(AudioSource::File(path));
        sound.play(); // before it is ready: plays once on arrival
        wait_until("the sound to load", || sound.state() != PlaybackState::Loading);

        assert_eq!(sound.state(), PlaybackState::Ready, "{:?}", sound.error());
        let duration = sound.duration().unwrap().as_secs_f64();
        assert!((duration - 0.4).abs() < 0.05, "{duration}");
        for _ in 0..12 {
            sound.play();
        }
        sound.stop();
        assert_eq!(sound.state(), PlaybackState::Ready);
    }

    #[test]
    fn dropping_the_last_handle_releases_the_track() {
        let player = silent_player("drop.wav", 2.0);
        player.play();
        drop(player);
        // The engine keeps running and serves the next handle.
        let next = silent_player("after_drop.wav", 0.3);
        assert_eq!(next.state(), PlaybackState::Ready);
    }
}
