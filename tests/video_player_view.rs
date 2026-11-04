//! The standard video player UI in a headless application.
//!
//! This host has no video backend, so the player rests at 0:00 with an
//! unknown length: enough to check what the view draws (time, captions,
//! chapter title) and that a resting player view stops asking for frames.

use std::time::Duration;

use aimer::video::{CaptionTrack, Chapter, Chapters, Cue, VideoPlayer, VideoPlayerView, VideoSource};
use aimer::AimerApp;
use aimer_cupid::draw_cmd::{DrawCommand, RenderOp};

/// Every piece of text the render tree draws.
fn drawn_texts<W: aimer::Widget + 'static>(app: &aimer::quiver::aimer_app::HeadlessAimerApp<W>) -> Vec<String> {
    let mut texts = Vec::new();
    for op in app.render_ops() {
        if let RenderOp::Draw(item) = op {
            for command in item.snapshot().commands.iter() {
                if let DrawCommand::DrawText { text, .. } = command {
                    texts.push(text.to_string());
                }
            }
        }
    }
    texts
}

fn view() -> VideoPlayerView {
    let player = VideoPlayer::from_source(VideoSource::file("/no/video/here.mp4"));
    VideoPlayerView::new()
        .player(player)
        .captions(CaptionTrack::from_cues(vec![Cue::new(
            Duration::ZERO,
            Duration::from_secs(10),
            "Hello captions",
        )]))
        .chapters(Chapters::new(vec![
            Chapter::new(Duration::ZERO, "Intro"),
            Chapter::new(Duration::from_secs(60), "Main part"),
        ]))
}

#[test]
fn the_view_draws_the_time_the_caption_showing_and_the_chapter() {
    let mut app = AimerApp::start_headless(view());
    app.pump_frames(10);
    let texts = drawn_texts(&app);

    assert!(texts.iter().any(|text| text == "0:00 / --:--"), "{texts:?}");
    assert!(texts.iter().any(|text| text == "Hello captions"), "{texts:?}");
    assert!(texts.iter().any(|text| text == "Intro"), "{texts:?}");
}

#[test]
fn a_resting_player_view_stops_asking_for_frames() {
    let mut app = AimerApp::start_headless(view());
    app.pump_frames(20);
    assert_eq!(app.pump_frames(30), 0, "frames kept coming for a player that is not playing");
}
