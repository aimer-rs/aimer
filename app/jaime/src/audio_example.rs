//! Audio playback: a bundled track with play, pause, stop, seek, volume, mute
//! and loop controls, plus a short effect that overlaps itself.
//!
//! The [`AudioPlayer`] and [`Sound`] live in the page's state, so rebuilding
//! the page never restarts them. Both are cheap handles: the buttons hold
//! clones, and the audio itself runs on the framework's audio thread. A
//! repeating animation controller drives frames, and the progress readout reads
//! the player's playhead on each one.
//!
//! The seek bar is a slider over the track. While it is dragged it shows the
//! dragged position instead of the playhead, so the thumb stays under the
//! finger, and the player seeks once, where the drag ends.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use aimer::animation::{AnimatedBuilder, AnimationController, curves};
use aimer::audio::{AudioLoops, AudioPlayer, AudioSource, PlaybackState, Sound};
use aimer::input::button::Button;
use aimer::range::{Slider, SliderThumb, SliderTrail};
use aimer::style::{
    BorderSlice, BorderStyle, BoxBorder, BoxDecoration, FontWeight, LayoutSpacing, TextAlign,
    TextStyle, Theme, ThemeData,
};
use aimer::{
    AnyWidget, BuildContext, Column, Container, Dimension, Row, ScrollAxis, Scrollable, SizedBox,
    State, StateUpdater, StatefulWidget, Text, Widget,
};

use crate::{Assets, theme};

/// How far the "-5 s" and "+5 s" buttons move the playhead.
const SKIP: Duration = Duration::from_secs(5);
/// Width of the seek bar in logical pixels.
const BAR_WIDTH: f32 = 480.0;
/// The seek bar's resolution, in seconds.
const SEEK_STEP: f64 = 0.05;

/// Builds the audio showcase page.
pub fn audio_example() -> impl Widget {
    Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .child(AudioExample::new())
}

#[derive(aimer::StatefulWidget)]
pub struct AudioExample;

impl AudioExample {
    pub fn new() -> Self {
        Self
    }
}

pub struct AudioExampleState {
    player: AudioPlayer,
    tap: Sound,
    /// Only drives frames: its value is never read.
    ticker: AnimationController,
    /// Where the seek bar is being dragged to, in seconds; `None` when it
    /// follows the playhead.
    scrub: Rc<Cell<Option<f64>>>,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for AudioExample {
    type State = AudioExampleState;

    fn create_state(self) -> AudioExampleState {
        let player = AudioPlayer::from_source(AudioSource::bundled(Assets::TRIMMED_MUSIC.path()));
        player.set_volume(0.6);
        let tap = Sound::from_source(AudioSource::bundled(Assets::TAP.path()));
        tap.set_volume(0.6);
        let ticker = AnimationController::new(Duration::from_secs(1), curves::LINEAR);
        ticker.set_repeat(true);
        ticker.forward_from_first_tick();
        AudioExampleState {
            player,
            tap,
            ticker,
            scrub: Rc::new(Cell::new(None)),
            updater: StateUpdater::empty(),
        }
    }
}

impl State<AudioExample> for AudioExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let updater = self.updater;
        let player = &self.player;

        let toggle = {
            let player = player.clone();
            control_button("Play / Pause", app_theme, true, move || player.toggle())
        };
        let stop = {
            let player = player.clone();
            control_button("Stop", app_theme, false, move || player.stop())
        };
        let back = {
            let player = player.clone();
            control_button("-5 s", app_theme, false, move || {
                player.seek(player.position().saturating_sub(SKIP));
            })
        };
        let forward = {
            let player = player.clone();
            control_button("+5 s", app_theme, false, move || {
                player.seek(player.position() + SKIP);
            })
        };
        let looping = {
            let player = player.clone();
            let label = if player.loops() == AudioLoops::Infinite {
                "Loop: on"
            } else {
                "Loop: off"
            };
            control_button(label, app_theme, false, move || {
                let next = if player.loops() == AudioLoops::Infinite {
                    AudioLoops::Once
                } else {
                    AudioLoops::Infinite
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
        let tap = {
            let tap = self.tap.clone();
            control_button("Tap (try it fast)", app_theme, true, move || tap.play())
        };

        let volume = {
            let player = player.clone();
            Slider::new()
                .range(0.0..1.0)
                .step(0.05)
                .value(f64::from(player.volume()))
                .width(320.0)
                .trail(
                    SliderTrail::new()
                        .height(6.0)
                        .radius(3.0)
                        .color(app_theme.primary_color),
                )
                .thumb(
                    SliderThumb::new()
                        .size(20.0)
                        .radius(10.0)
                        .color(app_theme.primary_color),
                )
                .on_change(move |next| {
                    player.set_volume(next as f32);
                    updater.set_state(|_| {});
                })
        };

        // The readout and bar read the player on every frame, so they follow
        // the playhead, and flip to "Finished" when the track ends by itself.
        let progress = {
            let player = player.clone();
            let scrub = self.scrub.clone();
            AnimatedBuilder::new(self.ticker.clone(), move |_| {
                progress_view(&player, &scrub, app_theme)
            })
        };

        Container::new()
            .padding(LayoutSpacing::all(28))
            .child(Column::new().children(vec![
                Text::new("Audio")
                    .text_style(
                        TextStyle::new()
                            .font_size(26)
                            .font_weight(FontWeight::Bold)
                            .color(app_theme.on_surface_color),
                    )
                    .boxed(),
                SizedBox::new().height(8).boxed(),
                Text::new(
                    "A bundled track plays through the system's audio player, driven from a \
                     dedicated audio thread. The tap sound below is a separate effect that \
                     can overlap itself.",
                )
                .text_style(
                    TextStyle::new()
                        .font_size(15)
                        .color(theme::muted_text(&app_theme)),
                )
                .boxed(),
                SizedBox::new().height(20.0).boxed(),
                progress.boxed(),
                SizedBox::new().height(16).boxed(),
                Row::new()
                    .children(vec![
                        toggle,
                        gap(),
                        stop,
                        gap(),
                        back,
                        gap(),
                        forward,
                        gap(),
                        looping,
                    ])
                    .boxed(),
                SizedBox::new().height(20).boxed(),
                Row::new()
                    .children(vec![
                        label("Volume", app_theme),
                        gap(),
                        volume.boxed(),
                        gap(),
                        mute,
                    ])
                    .boxed(),
                SizedBox::new().height(28).boxed(),
                label("Sound effect", app_theme),
                SizedBox::new().height(8).boxed(),
                tap,
            ]))
    }
}

/// The time readout, the seek bar, and the player's state.
fn progress_view(player: &AudioPlayer, scrub: &Rc<Cell<Option<f64>>>, app_theme: ThemeData) -> AnyWidget {
    let duration = player.duration();
    // While dragging, show where the thumb is, not where the audio is.
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
        PlaybackState::Loading => "Loading".to_string(),
        PlaybackState::Ready => "Ready".to_string(),
        PlaybackState::Playing => "Playing".to_string(),
        PlaybackState::Buffering => "Buffering".to_string(),
        PlaybackState::Paused => "Paused".to_string(),
        PlaybackState::Finished => "Finished".to_string(),
        PlaybackState::Failed => player
            .error()
            .map_or_else(|| "Failed".to_string(), |error| error.to_string()),
    };

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
            seek_bar(player, scrub, position, duration, app_theme),
            SizedBox::new().height(8).boxed(),
            Text::new(status)
                .text_style(
                    TextStyle::new()
                        .font_size(13)
                        .color(theme::muted_text(&app_theme)),
                )
                .boxed(),
        ])
        .boxed()
}

/// A slider over the whole track. It reports drags to `scrub` and seeks the
/// player only when a drag (or a key press) ends, so a drag costs one seek.
fn seek_bar(
    player: &AudioPlayer,
    scrub: &Rc<Cell<Option<f64>>>,
    position: Duration,
    duration: Option<Duration>,
    app_theme: ThemeData,
) -> AnyWidget {
    let total = duration.map_or(0.0, |total| total.as_secs_f64());
    let dragged = scrub.clone();
    let settled = scrub.clone();
    let player = player.clone();
    Slider::new()
        // The range must not be empty while the length is still unknown.
        .range(0.0..total.max(SEEK_STEP))
        .step(SEEK_STEP)
        .value(position.as_secs_f64())
        .disabled(duration.is_none())
        .width(BAR_WIDTH)
        .trail(
            SliderTrail::new()
                .height(6.0)
                .radius(3.0)
                .color(app_theme.primary_color),
        )
        .thumb(
            SliderThumb::new()
                .size(16.0)
                .radius(8.0)
                .color(app_theme.primary_color),
        )
        .on_change(move |seconds| dragged.set(Some(seconds)))
        .on_change_end(move |seconds| {
            player.seek(Duration::from_secs_f64(seconds.max(0.0)));
            // `seek` publishes the new playhead at once, so following it again
            // does not flash the old position.
            settled.set(None);
        })
        .boxed()
}

fn format_time(time: Duration) -> String {
    let seconds = time.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn gap() -> AnyWidget {
    SizedBox::new().width(12).boxed()
}

fn label(text: &'static str, app_theme: ThemeData) -> AnyWidget {
    Text::new(text)
        .text_style(
            TextStyle::new()
                .font_size(14)
                .font_weight(FontWeight::Bold)
                .color(app_theme.on_surface_color),
        )
        .boxed()
}

fn control_button(
    label: &'static str,
    app_theme: ThemeData,
    primary: bool,
    on_press: impl Fn() + 'static,
) -> AnyWidget {
    let (background, foreground) = if primary {
        (app_theme.primary_color, app_theme.on_primary_color)
    } else {
        (aimer::Color::Transparent, app_theme.primary_color)
    };
    Button::new()
        .on_press(on_press)
        .decoration(
            BoxDecoration::new()
                .background_color(background)
                .border(if primary {
                    BoxBorder::default()
                } else {
                    BoxBorder::all(
                        BorderSlice::new()
                            .style(BorderStyle::Solid)
                            .stroke(1.0)
                            .color(app_theme.primary_color),
                    )
                })
                .border_radius(8),
        )
        .child(
            Container::new()
                .height(Dimension::Px(32.0))
                .padding(LayoutSpacing::new().left(14).right(14))
                .child(
                    Text::new(label)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(13)
                                .font_weight(FontWeight::Bold)
                                .color(foreground),
                        ),
                ),
        )
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_builds_as_a_widget_without_starting_the_application() {
        fn assert_widget(_widget: impl Widget) {}
        assert_widget(audio_example());
    }

    #[test]
    fn times_read_as_minutes_and_two_digit_seconds() {
        assert_eq!(format_time(Duration::ZERO), "0:00");
        assert_eq!(format_time(Duration::from_millis(5_999)), "0:05");
        assert_eq!(format_time(Duration::from_secs(65)), "1:05");
        assert_eq!(format_time(Duration::from_secs(600)), "10:00");
    }
}
