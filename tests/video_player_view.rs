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

/// Each text drawn, with its world baseline origin and its layout bounds.
fn placed_texts<W: aimer::Widget + 'static>(
    app: &aimer::quiver::aimer_app::HeadlessAimerApp<W>,
) -> Vec<(String, f32, f32, Option<f32>, Option<f32>)> {
    let mut texts = Vec::new();
    for op in app.render_ops() {
        if let RenderOp::Draw(item) = op {
            for command in item.snapshot().commands.iter() {
                if let DrawCommand::DrawText { text, position, bounds_width, bounds_height, .. } = command {
                    texts.push((
                        text.to_string(),
                        item.origin.0 + position.x,
                        item.origin.1 + position.y,
                        *bounds_width,
                        *bounds_height,
                    ));
                }
            }
        }
    }
    texts
}

/// Each filled rectangle, in world coordinates, with its colour.
fn placed_fills<W: aimer::Widget + 'static>(
    app: &aimer::quiver::aimer_app::HeadlessAimerApp<W>,
) -> Vec<((u8, u8, u8, u8), aimer_cupid::draw_cmd::Rect)> {
    let mut fills = Vec::new();
    for op in app.render_ops() {
        if let RenderOp::Draw(item) = op {
            for command in item.snapshot().commands.iter() {
                if let DrawCommand::FillRect { rect, color, .. } = command {
                    let mut world = *rect;
                    world.x += item.origin.0;
                    world.y += item.origin.1;
                    fills.push(((color.r, color.g, color.b, color.a), world));
                }
            }
        }
    }
    fills
}

/// The standard view at a fixed size, laid out.
fn laid_out() -> aimer::quiver::aimer_app::HeadlessAimerApp<impl aimer::Widget + 'static> {
    let mut app = AimerApp::start_headless(view().width(WIDTH).height(HEIGHT));
    app.pump_frames(10);
    app
}

const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 360.0;
/// The control bar's translucent background.
const BAR: (u8, u8, u8, u8) = (0, 0, 0, 140);
/// The caption box's default background.
const CAPTION_BOX: (u8, u8, u8, u8) = (0, 0, 0, 190);

fn text<'a>(
    texts: &'a [(String, f32, f32, Option<f32>, Option<f32>)],
    wanted: &str,
) -> &'a (String, f32, f32, Option<f32>, Option<f32>) {
    texts.iter().find(|(text, ..)| text == wanted).unwrap_or_else(|| panic!("{wanted:?} not drawn: {texts:?}"))
}

#[test]
fn the_control_bar_sits_along_the_bottom_and_only_as_tall_as_its_content() {
    let app = laid_out();
    let fills = placed_fills(&app);
    let (_, bar) = fills.iter().find(|(color, _)| *color == BAR).expect("no bar background");
    assert!((bar.y + bar.height - HEIGHT).abs() < 0.5, "bar does not reach the bottom: {bar:?}");
    assert!(bar.height < HEIGHT / 3.0, "bar covers the picture: {bar:?}");
    assert!((bar.width - WIDTH).abs() < 0.5, "bar does not span the width: {bar:?}");

    // The seek bar's unplayed rail runs across the bar inside its padding.
    let (_, rail) = fills.iter().find(|(color, _)| *color == (255, 255, 255, 70)).expect("no seek rail");
    assert!(rail.y > bar.y && rail.x >= 12.0 && rail.x + rail.width <= WIDTH - 12.0, "rail {rail:?}");
    assert!(rail.width > WIDTH - 40.0, "rail is short: {rail:?}");

    let texts = placed_texts(&app);
    let (_, _, time_y, ..) = text(&texts, "0:00 / --:--");
    assert!(*time_y > bar.y && *time_y < HEIGHT, "time {time_y} is outside the bar {bar:?}");
}

#[test]
fn the_caption_box_hugs_its_text_and_is_centred() {
    let app = laid_out();
    let fills = placed_fills(&app);
    let (_, caption) = fills.iter().find(|(color, _)| *color == CAPTION_BOX).expect("no caption box");
    assert!(caption.width < WIDTH / 2.0, "caption box spans the picture: {caption:?}");
    let centre = caption.x + caption.width / 2.0;
    assert!((centre - WIDTH / 2.0).abs() < 1.0, "caption box is not centred: {caption:?}");

    let bar = fills.iter().find(|(color, _)| *color == BAR).expect("no bar background").1;
    assert!(caption.y + caption.height <= bar.y, "caption {caption:?} overlaps the bar {bar:?}");
}

#[test]
fn the_chapter_pill_hugs_its_title() {
    let app = laid_out();
    let fills = placed_fills(&app);
    let (_, pill) = fills
        .iter()
        .find(|(color, _)| *color == (255, 255, 255, 40))
        .expect("no chapter pill");
    let texts = placed_texts(&app);
    let (_, title_x, _, title_width, title_height) = text(&texts, "Intro");
    let (title_width, title_height) = (title_width.unwrap(), title_height.unwrap());
    assert!(pill.height < 30.0, "pill is stretched to the row: {pill:?}");
    assert!(pill.height >= title_height, "pill {pill:?} is shorter than its title ({title_height})");
    assert!(*title_x >= pill.x && title_x + title_width <= pill.x + pill.width, "title outside {pill:?}");
}
