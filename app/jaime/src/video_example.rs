//! Video playback: a bundled clip or an HLS stream, drawn straight from the
//! decoder's GPU memory, with play, pause, a draggable seek bar, volume, mute,
//! loop and fit controls.
//!
//! The [`VideoPlayer`] lives in the page's state, so rebuilding the page never
//! restarts it. The [`Video`] widget asks for frames only while the video
//! plays. The readout is driven by an animation controller that runs only
//! while the player wants frames, so a paused page draws nothing at all.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use aimer::animation::{AnimatedBuilder, AnimationController, curves};
use aimer::range::{Slider, SliderThumb, SliderTrail};
use aimer::style::{FontWeight, LayoutSpacing, TextStyle, Theme, ThemeData};
use aimer::video::{PlaybackState, Video, VideoFit, VideoLoops, VideoPlayer, VideoSource};
use aimer::{
    AnyWidget, BuildContext, Column, Container, Row, ScrollAxis, Scrollable, SizedBox, State,
    StateUpdater, StatefulWidget, Text, Widget,
};

use crate::audio_example::{control_button, format_time, gap, label};
use crate::{Assets, theme};

/// Apple's public HLS test stream: adaptive bitrate, with sound.
const STREAM_URL: &str =
    "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/master.m3u8";
/// The size the picture is shown at, in logical pixels.
const PICTURE: (f32, f32) = (480.0, 270.0);
/// The seek bar's resolution, in seconds.
const SEEK_STEP: f64 = 0.05;

/// Builds the video showcase page.
pub fn video_example() -> impl Widget {
    Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .child(VideoExample::new())
}

#[derive(aimer::StatefulWidget)]
pub struct VideoExample;

impl VideoExample {
    pub fn new() -> Self {
        Self
    }
}

pub struct VideoExampleState {
    player: VideoPlayer,
    streaming: bool,
    fit: VideoFit,
    /// Only drives the readout's frames: its value is never read. It runs
    /// while the player wants frames and is reset as soon as it does not.
    ticker: AnimationController,
    /// Where the seek bar is being dragged to, in seconds; `None` when it
    /// follows the playhead.
    scrub: Rc<Cell<Option<f64>>>,
    updater: StateUpdater<Self>,
}

fn bundled_clip() -> VideoSource {
    VideoSource::bundled(Assets::TEST_PATTERN.path())
}

impl StatefulWidget for VideoExample {
    type State = VideoExampleState;

    fn create_state(self) -> VideoExampleState {
        let player = VideoPlayer::from_source(bundled_clip());
        player.set_volume(0.6);
        let ticker = AnimationController::new(Duration::from_secs(1), curves::LINEAR);
        ticker.set_repeat(true);
        ticker.forward_from_first_tick();
        VideoExampleState {
            player,
            streaming: false,
            fit: VideoFit::Contain,
            ticker,
            scrub: Rc::new(Cell::new(None)),
            updater: StateUpdater::empty(),
        }
    }
}

impl State<VideoExample> for VideoExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let updater = self.updater;
        let player = &self.player;

        let picture = Container::new()
            .width(PICTURE.0)
            .height(PICTURE.1)
            .box_decoration(aimer::style::BoxDecoration::new().background_color(aimer::Color::Rgb(0, 0, 0)))
            .child(
                Video::new()
                    .player(player.clone())
                    .fit(self.fit)
                    .width(PICTURE.0)
                    .height(PICTURE.1),
            );

        let toggle = {
            let player = player.clone();
            let ticker = self.ticker.clone();
            control_button("Play / Pause", app_theme, true, move || {
                player.toggle();
                wake(&ticker);
            })
        };
        let stop = {
            let player = player.clone();
            let ticker = self.ticker.clone();
            control_button("Stop", app_theme, false, move || {
                player.stop();
                wake(&ticker);
            })
        };
        let looping = {
            let player = player.clone();
            let label = if player.loops() == VideoLoops::Infinite { "Loop: on" } else { "Loop: off" };
            control_button(label, app_theme, false, move || {
                let next = if player.loops() == VideoLoops::Infinite {
                    VideoLoops::Once
                } else {
                    VideoLoops::Infinite
                };
                player.set_loops(next);
                updater.set_state(|_| {});
            })
        };
        let mute = {
            let player = player.clone();
            let label = if player.is_muted() { "Unmute" } else { "Mute" };
            control_button(label, app_theme, false, move || {
                player.set_muted(!player.is_muted());
                updater.set_state(|_| {});
            })
        };
        let fit = {
            let label = match self.fit {
                VideoFit::Contain => "Fit: contain",
                VideoFit::Cover => "Fit: cover",
                VideoFit::Fill => "Fit: fill",
            };
            control_button(label, app_theme, false, move || {
                updater.set_state(|state| {
                    state.fit = match state.fit {
                        VideoFit::Contain => VideoFit::Cover,
                        VideoFit::Cover => VideoFit::Fill,
                        VideoFit::Fill => VideoFit::Contain,
                    };
                });
            })
        };
        let source = {
            let player = player.clone();
            let label = if self.streaming { "Source: HLS stream" } else { "Source: bundled clip" };
            control_button(label, app_theme, false, move || {
                let player = player.clone();
                updater.set_state(move |state| {
                    state.streaming = !state.streaming;
                    let source = if state.streaming {
                        VideoSource::url(STREAM_URL)
                    } else {
                        bundled_clip()
                    };
                    player.load(source);
                    player.play();
                    wake(&state.ticker);
                });
            })
        };

        // The readout and seek bar read the player on every frame, so they
        // follow the playhead and flip to "Finished" by themselves.
        let progress = {
            let player = player.clone();
            let scrub = self.scrub.clone();
            let ticker = self.ticker.clone();
            AnimatedBuilder::new(self.ticker.clone(), move |_| {
                let view = progress_view(&player, &scrub, &ticker, app_theme);
                // Once the video settles (paused, finished, failed) the
                // readout has shown its last change: stop asking for frames.
                if !player.wants_frames() && ticker.is_animating() {
                    ticker.reset();
                }
                view
            })
        };

        Container::new()
            .padding(LayoutSpacing::all(28))
            .child(Column::new().children(vec![
                Text::new("Video")
                    .text_style(
                        TextStyle::new()
                            .font_size(26)
                            .font_weight(FontWeight::Bold)
                            .color(app_theme.on_surface_color),
                    )
                    .boxed(),
                SizedBox::new().height(8).boxed(),
                Text::new(
                    "The picture is the decoder's own GPU texture, drawn without a copy. The \
                     widget repaints only when a new frame arrives, and asks for none while \
                     paused.",
                )
                .text_style(TextStyle::new().font_size(15).color(theme::muted_text(&app_theme)))
                .boxed(),
                SizedBox::new().height(20).boxed(),
                picture.boxed(),
                SizedBox::new().height(16).boxed(),
                progress.boxed(),
                SizedBox::new().height(16).boxed(),
                Row::new()
                    .children(vec![toggle, gap(), stop, gap(), looping, gap(), mute])
                    .boxed(),
                SizedBox::new().height(12).boxed(),
                Row::new().children(vec![fit, gap(), source]).boxed(),
                SizedBox::new().height(12).boxed(),
                label("Not supported here yet? Video plays on macOS and iOS.", app_theme),
            ]))
    }
}

/// The time readout, the seek bar, and the player's state.
fn progress_view(
    player: &VideoPlayer,
    scrub: &Rc<Cell<Option<f64>>>,
    ticker: &AnimationController,
    app_theme: ThemeData,
) -> AnyWidget {
    let duration = player.duration();
    // While dragging, show where the thumb is, not where the video is.
    let position = scrub
        .get()
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .unwrap_or_else(|| player.position());
    let time = format!(
        "{} / {}",
        format_time(position),
        duration.map_or_else(|| "--:--".to_string(), format_time)
    );
    let status = match player.state() {
        PlaybackState::Idle => "Nothing loaded".to_string(),
        PlaybackState::Loading => "Opening".to_string(),
        PlaybackState::Ready => "Ready".to_string(),
        PlaybackState::Playing => "Playing".to_string(),
        PlaybackState::Buffering => "Buffering".to_string(),
        PlaybackState::Paused => "Paused".to_string(),
        PlaybackState::Finished => "Finished".to_string(),
        PlaybackState::Failed => player
            .error()
            .map_or_else(|| "Failed".to_string(), |error| error.to_string()),
    };
    let size = player
        .size()
        .map_or_else(String::new, |(width, height)| format!("  ·  {width} x {height}"));

    Column::new()
        .children(vec![
            Text::new(time)
                .text_style(
                    TextStyle::new()
                        .font_size(20)
                        .font_weight(FontWeight::Bold)
                        .color(app_theme.on_surface_color),
                )
                .boxed(),
            SizedBox::new().height(8).boxed(),
            seek_bar(player, scrub, ticker, position, duration, app_theme),
            SizedBox::new().height(8).boxed(),
            Text::new(format!("{status}{size}"))
                .text_style(TextStyle::new().font_size(13).color(theme::muted_text(&app_theme)))
                .boxed(),
        ])
        .boxed()
}

/// A slider over the whole video. It shows a drag while the finger is down
/// and seeks once, where the drag ends.
fn seek_bar(
    player: &VideoPlayer,
    scrub: &Rc<Cell<Option<f64>>>,
    ticker: &AnimationController,
    position: Duration,
    duration: Option<Duration>,
    app_theme: ThemeData,
) -> AnyWidget {
    let total = duration.map_or(0.0, |total| total.as_secs_f64());
    let dragged = scrub.clone();
    let settled = scrub.clone();
    let player = player.clone();
    let ticker = ticker.clone();
    Slider::new()
        // The range must not be empty while the length is still unknown.
        .range(0.0..total.max(SEEK_STEP))
        .step(SEEK_STEP)
        .value(position.as_secs_f64())
        .disabled(duration.is_none())
        .width(PICTURE.0)
        .trail(SliderTrail::new().height(6.0).radius(3.0).color(app_theme.primary_color))
        .thumb(SliderThumb::new().size(16.0).radius(8.0).color(app_theme.primary_color))
        .on_change({
            let ticker = ticker.clone();
            move |seconds| {
                dragged.set(Some(seconds));
                // The readout follows the thumb, also while paused.
                wake(&ticker);
            }
        })
        .on_change_end(move |seconds| {
            player.seek(Duration::from_secs_f64(seconds.max(0.0)));
            settled.set(None);
            // Show the new position, and the frame it decodes, while paused.
            wake(&ticker);
        })
        .boxed()
}

/// Runs the readout again after a control changed the player.
fn wake(ticker: &AnimationController) {
    if !ticker.is_animating() {
        ticker.forward_from_first_tick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_builds_as_a_widget_without_starting_the_application() {
        fn assert_widget(_widget: impl Widget) {}
        assert_widget(video_example());
    }
}
