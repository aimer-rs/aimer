//! A live, interactive showcase for Aimer's controlled range controls.
//!
//! Each slider publishes pointer and keyboard proposals through `on_change`.
//! The page stores those proposals in its own retained state and supplies the
//! values back on the next rebuild, which is the controlled-component loop an
//! application would use.

use aimer::macros::widget;
use aimer::style::{
    BoxDecoration, FontWeight, LayoutSpacing, Spacing, TextStyle, Theme, ThemeData,
};
use aimer::{
    AimerApp, AnyWidget, BuildContext, Column, Container, State, StateUpdater, StatefulWidget,
    Text, Widget,
};

use aimer::range::{RangeSlider, Slider, SliderKey, SliderThumb, SliderTrail};

use crate::theme;

/// Builds the range-controls showcase without starting an application.
pub fn range_controls_example() -> impl Widget {
    RangeControlsExample::new()
}

/// Starts the standalone range-controls showcase.
pub fn start_range_controls_example() {
    AimerApp::start(theme::provide(range_controls_example()));
}

/// Demonstrates single-value and two-thumb range controls, including their
/// composable visuals, keyboard, pointer-conversion, and disabled states.
#[widget(Stateful)]
pub struct RangeControlsExample {}

impl RangeControlsExample {
    /// Creates a deterministic range-controls showcase.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for RangeControlsExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RangeControlsExampleState {
    slider_value: f64,
    range_lower: f64,
    range_upper: f64,
    updater: StateUpdater<Self>,
}

impl RangeControlsExampleState {
    fn initial() -> Self {
        Self {
            // Start at the same stepped value the original static showcase
            // reached with one ArrowRight action from 45.
            slider_value: 55.0,
            range_lower: 20.0,
            range_upper: 70.0,
            updater: StateUpdater::empty(),
        }
    }
}

impl StatefulWidget for RangeControlsExample {
    type State = RangeControlsExampleState;

    fn create_state(self) -> Self::State {
        RangeControlsExampleState::initial()
    }
}

impl State<RangeControlsExample> for RangeControlsExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);

        let slider_trail = SliderTrail::new()
            .height(6.0)
            .radius(3.0)
            .color(app_theme.primary_color);
        let slider_thumb = SliderThumb::new()
            .size(20.0)
            .radius(10.0)
            .color(app_theme.primary_color);

        let slider_updater = self.updater;
        let slider = Slider::new()
            .range(0.0..100.0)
            .step(1.0)
            .value(self.slider_value)
            .width(320.0)
            .trail(slider_trail.clone())
            .thumb(slider_thumb.clone())
            .on_change(move |value| {
                slider_updater.set_state(move |state| state.slider_value = value);
            });

        let range_updater = self.updater;
        let range_slider = RangeSlider::new()
            .range(0.0..100.0)
            .step(10.0)
            .values(self.range_lower..self.range_upper)
            .width(320.0)
            .trail(slider_trail)
            .thumbs(slider_thumb.clone(), slider_thumb)
            .on_change(move |(lower, upper)| {
                range_updater.set_state(move |state| {
                    state.range_lower = lower;
                    state.range_upper = upper;
                });
            });

        let disabled_slider = Slider::new()
            .range(0.0..100.0)
            .step(10.0)
            .value(50.0)
            .disabled(true)
            .width(320.0);

        let keyboard_sample = keyboard_sample(self.slider_value);
        let (invalid_config, invalid_semantics) = invalid_sample();

        let pointer_value = slider
            .value_at_position(75.0, 100.0)
            .expect("showcase track is valid");
        let lower_semantics = range_slider.semantics();
        let disabled_semantics = disabled_slider.semantics();

        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .gaps(LayoutSpacing::all(Spacing::Px(16)))
                    .children(vec![
                        Text::new("Range controls")
                            .text_style(
                                TextStyle::new()
                                    .font_size(28)
                                    .font_weight(FontWeight::Bold)
                                    .color(app_theme.on_background_color),
                            )
                            .boxed(),
                        Text::new(
                            "Stepped values stay controlled by the page while the widgets \
                             handle pointer, keyboard, semantics, disabled behavior, and \
                             theme-driven trail/thumb composition.",
                        )
                        .wrapped()
                        .text_style(
                            TextStyle::new()
                                .font_size(15)
                                .color(theme::muted_text(&app_theme)),
                        )
                        .boxed(),
                        range_card(
                            "Single value",
                            format!(
                                "Value: {} · pointer 75%: {}\nArrowRight sample: {}",
                                self.slider_value, pointer_value, keyboard_sample
                            ),
                            slider.boxed(),
                            app_theme,
                        ),
                        range_card(
                            "Two values",
                            format!(
                                "Lower: {} · upper: {} · semantic role: {:?}",
                                self.range_lower,
                                self.range_upper,
                                lower_semantics.role()
                            ),
                            range_slider.boxed(),
                            app_theme,
                        ),
                        range_card(
                            "Disabled state",
                            format!(
                                "Enabled: {} · invalid range: {}\nInvalid sample: validate={} · semantics={}",
                                disabled_semantics.is_enabled(),
                                disabled_semantics.invalid_range(),
                                invalid_config,
                                invalid_semantics,
                            ),
                            disabled_slider.boxed(),
                            app_theme,
                        ),
                    ]),
            )
    }
}

/// Calculates the value produced by one keyboard increment in the showcase.
pub(crate) fn keyboard_sample(value: f64) -> f64 {
    let mut slider = Slider::new()
        .range(0.0..100.0)
        .step(1.0)
        .value(value);
    slider
        .handle_key(SliderKey::ArrowRight)
        .expect("showcase keyboard sample is valid");
    slider.current_value()
}

/// Returns validation and semantic flags for an intentionally invalid slider.
pub(crate) fn invalid_sample() -> (bool, bool) {
    let slider = Slider::new()
        .range(100.0..0.0)
        .step(0.0)
        .value(50.0);
    (
        slider
            .validate()
            .is_err(),
        slider
            .semantics()
            .invalid_range(),
    )
}

fn range_card(
    title: &'static str,
    status: String,
    control: AnyWidget,
    app_theme: ThemeData,
) -> AnyWidget {
    Container::new()
        .padding(LayoutSpacing::all(Spacing::Px(16)))
        .box_decoration(
            BoxDecoration::new()
                .background_color(app_theme.surface_color)
                .border_radius(12),
        )
        .child(
            Column::new()
                .gaps(LayoutSpacing::all(Spacing::Px(8)))
                .children([
                    Text::new(title)
                        .text_style(
                            TextStyle::new()
                                .font_size(15)
                                .font_weight(FontWeight::Bold)
                                .color(app_theme.on_surface_color),
                        )
                        .boxed(),
                    Text::new(status)
                        .wrapped()
                        .text_style(
                            TextStyle::new()
                                .font_size(13)
                                .color(theme::muted_text(&app_theme)),
                        )
                        .boxed(),
                    control,
                ]),
        )
        .boxed()
}
