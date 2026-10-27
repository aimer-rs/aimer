//! An adjustable glass highlight beside the same surface without a rim.

use aimer::decoration::{BorderRadius, Decoration, DecorationBox, LightEdge};
use aimer::range::Slider;
use aimer::style::{BoxDecoration, Color, FontWeight, LayoutSpacing, Spacing, TextStyle};
use aimer::{
    AimerApp, AnyWidget, BuildContext, ChildBuilder, Column, Container, Row, State, StateUpdater,
    StatefulWidget, Text, Widget,
};

use crate::theme;

/// Starts the standalone light-edge example.
pub fn start_light_edge_example() {
    AimerApp::start(theme::provide(light_edge_example()));
}

/// Builds side-by-side surfaces with adjustable directional edge lighting.
pub fn light_edge_example() -> impl Widget {
    LightEdgeExample::new()
}

/// Demonstrates how a glass rim catches a configurable light source.
#[derive(StatefulWidget)]
pub struct LightEdgeExample {}

impl LightEdgeExample {
    /// Creates a soft white 8-pixel highlight lit from the upper left.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for LightEdgeExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LightEdgeExampleState {
    angle: f64,
    width: f64,
    strength: f64,
    radius: f64,
    reflection: f64,
    plain_content: ChildBuilder,
    lit_content: ChildBuilder,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for LightEdgeExample {
    type State = LightEdgeExampleState;

    fn create_state(self) -> Self::State {
        LightEdgeExampleState {
            angle: 225.0,
            width: 3.0,
            strength: 0.8,
            radius: 24.0,
            reflection: 0.35,
            plain_content: ChildBuilder::from_widget(panel_content(
                "Without highlight",
                "The same surface, without the bright rim.",
            )),
            lit_content: ChildBuilder::from_widget(panel_content(
                "Light edge",
                "Turn the light and watch the rim follow the corners.",
            )),
            updater: StateUpdater::empty(),
        }
    }
}

impl State<LightEdgeExample> for LightEdgeExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let updater = self.updater;
        Column::new().gaps(LayoutSpacing::all(Spacing::Px(16))).children([
            Text::new("Turn the light to move the highlight around the surface. Compare it with the unlit card beside it.")
                .text_style(TextStyle::new().font_size(15).color(theme::app_theme().on_background_color))
                .wrapped().boxed(),
            Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                control(format!("Light direction: {:.0}°", self.angle), self.angle, 0.0..360.0, 5.0,
                    move |value| updater.set_state(move |state| state.angle = value)),
                control(format!("Blend width: {:.1} px", self.width), self.width, 0.0..16.0, 0.5,
                    move |value| updater.set_state(move |state| state.width = value)),
            ]).boxed(),
            Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                control(format!("Highlight strength: {:.2}", self.strength), self.strength, 0.0..1.0, 0.05,
                    move |value| updater.set_state(move |state| state.strength = value)),
                control(format!("Corner radius: {:.0} px", self.radius), self.radius, 0.0..80.0, 1.0,
                    move |value| updater.set_state(move |state| state.radius = value)),
            ]).boxed(),
            control(format!("Reflection strength: {:.2}", self.reflection), self.reflection, 0.0..1.0, 0.05,
                move |value| updater.set_state(move |state| state.reflection = value)),
            Container::new().padding(LayoutSpacing::all(Spacing::Px(24)))
                .box_decoration(BoxDecoration::new().background_color(Color::Rgb(9, 16, 30)).border_radius(16))
                .child(Row::new().gaps(LayoutSpacing::all(Spacing::Px(24)))
                    .children([self.panel(false), self.panel(true)]))
                .boxed(),
        ])
    }
}

impl LightEdgeExampleState {
    fn panel(&self, highlighted: bool) -> AnyWidget {
        let content = if highlighted {
            &self.lit_content
        } else {
            &self.plain_content
        };
        // The surface paints before the decoration. Its retained text child
        // stays transparent so it cannot cover the highlight at the edges.
        Container::new()
            .height(176)
            .box_decoration(
                BoxDecoration::new()
                    .background_color(Color::Rgb(24, 42, 76))
                    .border_radius(self.radius as f32),
            )
            .child(
                DecorationBox::new()
                    .decoration(vec![
                        BorderRadius::from(self.radius as f32).notype(),
                        LightEdge::new()
                            .light_angle(self.angle as f32)
                            .width(self.width as f32)
                            .reflection_strength(self.reflection as f32)
                            .strength(if highlighted {
                                self.strength as f32
                            } else {
                                0.0
                            })
                            .notype(),
                    ])
                    .child(content.clone()),
            )
            .boxed()
    }
}

fn control(
    label: String,
    value: f64,
    range: std::ops::Range<f64>,
    step: f64,
    on_change: impl Fn(f64) + 'static,
) -> AnyWidget {
    Column::new()
        .gaps(LayoutSpacing::all(Spacing::Px(8)))
        .children([
            Text::new(label)
                .text_style(
                    TextStyle::new()
                        .font_size(14)
                        .color(theme::app_theme().on_background_color),
                )
                .boxed(),
            Slider::new()
                .range(range)
                .step(step)
                .value(value)
                .width(280.0)
                .on_change(on_change)
                .boxed(),
        ])
        .boxed()
}

fn panel_content(title: &'static str, description: &'static str) -> impl Widget {
    Container::new()
        .width(280)
        .height(176)
        .padding(LayoutSpacing::all(Spacing::Px(24)))
        .child(
            Column::new()
                .gaps(LayoutSpacing::all(Spacing::Px(12)))
                .children([
                    Text::new(title)
                        .text_style(
                            TextStyle::new()
                                .font_size(22)
                                .font_weight(FontWeight::Bold)
                                .color(Color::WHITE),
                        )
                        .boxed(),
                    Text::new(description)
                        .text_style(
                            TextStyle::new()
                                .font_size(14)
                                .color(Color::Rgb(190, 206, 232)),
                        )
                        .wrapped()
                        .boxed(),
                ]),
        )
}
