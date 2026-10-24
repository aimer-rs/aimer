use std::time::Duration;

use aimer::animation::{
    AnimatedBuilder, curves, KeyframeTimeline, KeyframeTrack, TimedKeyframeAnimation,
    TweenAnimation,
};
use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::style::{
    BoxDecoration, FontWeight, LayoutSpacing, Spacing, TextStyle, Theme, ThemeData,
};
use aimer::{
    BuildContext, Column, Container, Dimension, SizedBox, State, StateUpdater, StatefulWidget,
    Text, Widget, ZeroSizedBox,
};

use crate::theme;

const BAR_HEIGHT: f32 = 40.0;
const TRACK_WIDTH: f32 = 360.0;

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// A bar that grows to the full track width and eases out.
fn growing_bar() -> TweenAnimation<f32> {
    TweenAnimation::new(24.0, TRACK_WIDTH, ms(900), curves::EASE_OUT)
}

/// A square that stretches wide, then tall, then settles. Each segment has its
/// own easing, which a single tween cannot express.
fn stretch_track() -> KeyframeTrack<(f32, f32)> {
    KeyframeTrack::builder()
        .at(ms(0), (48.0, 48.0))
        .at_with(ms(300), (200.0, 48.0), curves::EASE_OUT)
        .at_with(ms(700), (200.0, 120.0), curves::EASE_IN_OUT)
        .at_with(ms(1100), (48.0, 48.0), curves::EASE_IN)
        .build()
        .expect("the example keyframes are not empty")
}

/// Opacity fades in quickly while the width keeps growing for longer, so the
/// two tracks differ in length and share one clock.
fn fade_track() -> KeyframeTrack<f32> {
    KeyframeTrack::builder()
        .at(ms(0), 0.15)
        .at_with(ms(300), 1.0, curves::EASE_OUT)
        .build()
        .expect("the example keyframes are not empty")
}

fn width_track() -> KeyframeTrack<f32> {
    KeyframeTrack::builder()
        .at(ms(0), 24.0)
        .at_with(ms(600), 200.0, curves::EASE_IN_OUT)
        .at_with(ms(1200), TRACK_WIDTH, curves::EASE_OUT)
        .build()
        .expect("the example keyframes are not empty")
}

/// Builds the tween and keyframe example without starting an application.
pub fn tween_keyframe_example() -> impl Widget {
    TweenKeyframeExample
}

#[widget(Stateful)]
struct TweenKeyframeExample;

struct TweenKeyframeState {
    tween: TweenAnimation<f32>,
    stretch: TimedKeyframeAnimation<(f32, f32)>,
    fade: KeyframeTrack<f32>,
    width: KeyframeTrack<f32>,
    timeline: KeyframeTimeline,
}

impl StatefulWidget for TweenKeyframeExample {
    type State = TweenKeyframeState;

    fn create_state(self) -> Self::State {
        let fade = fade_track();
        let width = width_track();
        let timeline = KeyframeTimeline::new(Duration::ZERO)
            .including(&fade)
            .including(&width);
        let state = TweenKeyframeState {
            tween: growing_bar(),
            stretch: TimedKeyframeAnimation::new(stretch_track()),
            fade,
            width,
            timeline,
        };
        // Play once on first paint; the clock starts on the first frame so
        // building the page does not use up part of the animation.
        state.tween.forward_from_first_tick();
        state.stretch.forward_from_first_tick();
        state.timeline.forward_from_first_tick();
        state
    }
}

impl State<TweenKeyframeExample> for TweenKeyframeState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        let accent = theme.primary_color;
        let heading = move |text: &'static str| {
            Text::new(text)
                .text_style(
                    TextStyle::new()
                        .font_size(18)
                        .font_weight(FontWeight::Bold)
                        .color(theme.on_background_color),
                )
                .boxed()
        };
        let note = move |text: &'static str| {
            Text::new(text)
                .text_style(
                    TextStyle::new()
                        .font_size(14)
                        .color(theme::muted_text(&theme)),
                )
                .boxed()
        };

        let tween = AnimatedBuilder::tween(self.tween.clone(), move |width| {
            Container::new()
                .width(Dimension::Px(width))
                .height(Dimension::Px(BAR_HEIGHT))
                .color(accent)
                .child(ZeroSizedBox)
        });

        let stretch = AnimatedBuilder::keyframes(self.stretch.clone(), move |(width, height)| {
            Container::new()
                .width(Dimension::Px(width))
                .height(Dimension::Px(height))
                .color(accent)
                .child(ZeroSizedBox)
        });

        let (fade, width) = (self.fade.clone(), self.width.clone());
        let timeline = AnimatedBuilder::timeline(self.timeline.clone(), move |frame| {
            Container::new()
                .width(Dimension::Px(frame.sample(&width)))
                .height(Dimension::Px(BAR_HEIGHT))
                .color(accent.with_alpha(frame.sample(&fade)))
                .child(ZeroSizedBox)
        });

        let replay = {
            let this = (
                self.tween.clone(),
                self.stretch.clone(),
                self.timeline.clone(),
            );
            Button::new()
                .on_press(move || {
                    let (tween, stretch, timeline) = &this;
                    tween.reset();
                    tween.forward();
                    stretch.reset();
                    stretch.forward();
                    timeline.reset();
                    timeline.forward();
                })
                .decoration(
                    BoxDecoration::new()
                        .background_color(theme.primary_color)
                        .border_radius(8),
                )
                .child(
                    Container::new()
                        .padding(LayoutSpacing::all(Spacing::Px(10)))
                        .child(
                            Text::new("Replay")
                                .text_style(
                                    TextStyle::new()
                                        .font_size(15)
                                        .color(theme.on_primary_color),
                                ),
                        ),
                )
        };

        Container::new()
            .color(theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(28)))
            .child(Column::new().children(vec![
                Text::new("Tween and keyframe animations")
                    .text_style(
                        TextStyle::new()
                            .font_size(26)
                            .font_weight(FontWeight::Bold)
                            .color(theme.on_background_color),
                    )
                    .boxed(),
                SizedBox::new().height(8).boxed(),
                note("Typed values that own their timing. No controller and tween to pair by hand."),
                SizedBox::new().height(16).boxed(),
                replay.boxed(),
                SizedBox::new().height(24).boxed(),
                heading("TweenAnimation"),
                note("0 \u{2192} full width with one ease-out curve."),
                SizedBox::new().height(8).boxed(),
                tween.boxed(),
                SizedBox::new().height(24).boxed(),
                heading("TimedKeyframeAnimation"),
                note("A (width, height) pair with a different curve on each segment."),
                SizedBox::new().height(8).boxed(),
                stretch.boxed(),
                SizedBox::new().height(24).boxed(),
                heading("KeyframeTimeline"),
                note("Width and opacity tracks of different lengths, sampled from one clock."),
                SizedBox::new().height(8).boxed(),
                timeline.boxed(),
            ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stretch_track_visits_each_pose_at_its_keyframe() {
        let track = stretch_track();

        assert_eq!(track.at(ms(0)), (48.0, 48.0));
        assert_eq!(track.at(ms(300)), (200.0, 48.0));
        assert_eq!(track.at(ms(700)), (200.0, 120.0));
        assert_eq!(track.at(ms(1100)), (48.0, 48.0));
    }

    #[test]
    fn the_timeline_tracks_share_a_clock_but_finish_at_different_times() {
        let (fade, width) = (fade_track(), width_track());
        let timeline = KeyframeTimeline::new(Duration::ZERO)
            .including(&fade)
            .including(&width);

        assert_eq!(timeline.duration(), width.duration());
        let frame = timeline.frame_at_progress(0.5);
        // Fade has finished by the midpoint; width is still growing.
        assert_eq!(frame.sample(&fade), 1.0);
        assert!(frame.sample(&width) < TRACK_WIDTH);
    }

    #[test]
    fn the_tween_ends_at_the_full_track_width() {
        assert_eq!(growing_bar().tween().end, TRACK_WIDTH);
    }
}
