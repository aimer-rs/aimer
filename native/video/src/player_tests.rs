//! The player against a fake platform: no decoder, no GPU, no real time.

use aimer_cupid::external_texture::ExternalTextureSource;

use super::*;
use crate::test_support::{last, opened, player, ready_player};

#[test]
fn a_fresh_player_is_idle_and_ignores_controls() {
    let player = player();
    player.play();
    player.seek(Duration::from_secs(3));
    assert_eq!(player.state(), PlaybackState::Idle);
    assert_eq!(player.duration(), None);
    assert!(opened().is_empty());
}

#[test]
fn a_source_that_cannot_open_fails_and_play_does_nothing() {
    let player = player();
    player.load(VideoSource::file("missing.mp4"));
    assert_eq!(player.state(), PlaybackState::Failed);
    assert!(matches!(player.error(), Some(VideoError::NotFound(_))));
    player.play();
    assert_eq!(player.state(), PlaybackState::Failed);
}

#[test]
fn opening_reads_as_loading_and_a_play_meanwhile_reads_as_buffering() {
    let player = player();
    player.load(VideoSource::url("https://example.test/live.m3u8"));
    assert_eq!(player.state(), PlaybackState::Loading);
    player.play();
    assert_eq!(player.state(), PlaybackState::Buffering);
    assert!(player.is_playing());

    last().lock().unwrap().ready = true;
    assert_eq!(player.state(), PlaybackState::Playing);
    assert_eq!(player.size(), Some((1920, 1080)));
}

#[test]
fn a_stall_while_playing_reads_as_buffering_and_resumes_by_itself() {
    let player = ready_player();
    player.play();
    last().lock().unwrap().buffering = true;
    assert_eq!(player.state(), PlaybackState::Buffering);
    last().lock().unwrap().buffering = false;
    assert_eq!(player.state(), PlaybackState::Playing);
}

#[test]
fn ready_playing_paused_stopped() {
    let player = ready_player();
    assert_eq!(player.state(), PlaybackState::Ready);
    player.play();
    assert_eq!(player.state(), PlaybackState::Playing);
    player.pause();
    assert_eq!(player.state(), PlaybackState::Paused);
    player.toggle();
    assert_eq!(player.state(), PlaybackState::Playing);
    player.stop();
    assert_eq!(player.state(), PlaybackState::Ready);
    assert_eq!(last().lock().unwrap().position, 0.0);
}

#[test]
fn reaching_the_end_finishes_and_play_starts_again_from_the_top() {
    let player = ready_player();
    player.play();
    {
        let fake = last();
        let mut state = fake.lock().unwrap();
        state.position = 10.0;
        state.playing = false; // the end
    }
    assert_eq!(player.state(), PlaybackState::Finished);
    assert!(!player.is_playing());

    player.play();
    assert_eq!(player.state(), PlaybackState::Playing);
    let fake = last();
    let state = fake.lock().unwrap();
    assert_eq!(state.seeks.last(), Some(&0.0), "restarted from the top");
    assert_eq!(state.plays, 2);
}

#[test]
fn a_loop_count_plays_that_many_passes_then_finishes() {
    let player = ready_player();
    player.set_loops(VideoLoops::Count(3));
    player.play();
    for pass in 2..=3 {
        last().lock().unwrap().playing = false; // a pass ended
        assert_eq!(player.state(), PlaybackState::Playing, "pass {pass} started");
        assert_eq!(last().lock().unwrap().plays, pass);
    }
    last().lock().unwrap().playing = false;
    assert_eq!(player.state(), PlaybackState::Finished, "no fourth pass");

    // A fresh run gets the full count again.
    player.play();
    last().lock().unwrap().playing = false;
    assert_eq!(player.state(), PlaybackState::Playing);
}

#[test]
fn an_infinite_loop_never_finishes() {
    let player = ready_player();
    player.set_loops(VideoLoops::Infinite);
    player.play();
    for _ in 0..50 {
        last().lock().unwrap().playing = false;
        assert_eq!(player.state(), PlaybackState::Playing);
    }
}

#[test]
fn a_pause_is_never_mistaken_for_the_end() {
    let player = ready_player();
    player.set_loops(VideoLoops::Count(2));
    player.play();
    player.pause();
    assert_eq!(player.state(), PlaybackState::Paused);
    assert_eq!(last().lock().unwrap().plays, 1, "pausing did not start a pass");
}

#[test]
fn a_stream_that_fails_reports_the_error_instead_of_finishing() {
    let player = ready_player();
    player.play();
    {
        let fake = last();
        let mut state = fake.lock().unwrap();
        state.playing = false;
        state.error = Some(VideoError::Network("connection lost".into()));
    }
    assert_eq!(player.state(), PlaybackState::Failed);
    assert_eq!(player.error(), Some(VideoError::Network("connection lost".into())));
}

#[test]
fn seeks_are_clamped_once_the_length_is_known() {
    let player = player();
    player.load(VideoSource::url("https://example.test/a.mp4"));
    player.seek(Duration::from_secs(42));
    assert_eq!(last().lock().unwrap().position, 42.0, "unknown length: not clamped");

    let player = ready_player();
    player.seek(Duration::from_secs(42));
    assert_eq!(last().lock().unwrap().position, 10.0);
}

#[test]
fn volume_is_clamped_nan_is_ignored_and_mute_keeps_the_volume() {
    let player = ready_player();
    player.set_volume(2.0);
    assert_eq!(player.volume(), 1.0);
    player.set_volume(f32::NAN);
    assert_eq!(player.volume(), 1.0);
    player.set_volume(0.4);
    player.set_muted(true);
    assert_eq!(last().lock().unwrap().volume, 0.0);
    player.set_muted(false);
    assert_eq!(last().lock().unwrap().volume, 0.4);

    // A newly loaded source starts with the same settings.
    player.set_muted(true);
    player.load(VideoSource::file("other.mp4"));
    assert_eq!(last().lock().unwrap().volume, 0.0);
}

#[test]
fn the_generation_changes_exactly_when_a_new_frame_arrives() {
    let player = ready_player();
    let first = player.poll();
    assert_eq!(player.poll(), first, "no new frame, same generation");
    assert!(!player.has_frame());

    last().lock().unwrap().pending_frames.push(1);
    let second = player.poll();
    assert_ne!(second, first);
    assert_eq!(player.poll(), second);
    assert!(player.has_frame());

    // The renderer reads the stored frame through the texture.
    let texture = player.texture();
    assert_eq!(texture.generation(), second);
    let frame = texture.current_frame().expect("a frame was stored");
    assert_eq!((frame.width(), frame.height()), (1920, 1080));
}

#[test]
fn loading_drops_the_old_frame_and_changes_the_generation() {
    let player = ready_player();
    last().lock().unwrap().pending_frames.push(1);
    let before = player.poll();
    assert!(player.has_frame());

    player.load(VideoSource::file("next.mp4"));
    assert!(!player.has_frame(), "the old video's frame is gone");
    assert!(player.texture().current_frame().is_none());
    assert_ne!(player.poll(), before);
}

#[test]
fn clones_control_the_same_video() {
    let player = ready_player();
    let copy = player.clone();
    copy.play();
    assert_eq!(player.state(), PlaybackState::Playing);
    copy.set_loops(VideoLoops::Count(4));
    assert_eq!(player.loops(), VideoLoops::Count(4));
}

#[test]
fn the_frame_source_is_the_same_across_textures() {
    let player = ready_player();
    let a = player.texture();
    let b = player.texture();
    assert!(Arc::ptr_eq(a.source(), b.source()));
    assert_eq!(a, b);
    let _ = <FrameSlot as ExternalTextureSource>::current_frame;
}
