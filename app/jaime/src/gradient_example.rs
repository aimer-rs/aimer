//! Linear, radial and conic gradients, plus decorations painted over and through a child.

use aimer::decoration::{
    BackdropFilter, BorderRadius, Decoration, DecorationBox, ForegroundDecorationBox, Gradient,
    LightEdge, MaskedDecorationBox,
};
use aimer::range::Slider;
use aimer::style::{BoxDecoration, Color, FontWeight, LayoutSpacing, Spacing, TextStyle};
use aimer::{
    AimerApp, AnyWidget, BuildContext, ChildBuilder, Column, Container, Row, ScrollAxis,
    Scrollable, SizedBox, State, StateUpdater, StatefulWidget, Text, Widget,
};

use crate::theme;

const CARD_WIDTH: u32 = 220;
const CARD_HEIGHT: u32 = 120;

/// Starts the standalone gradient example.
pub fn start_gradient_example() {
    AimerApp::start(theme::provide(gradient_example()));
}

/// Builds adjustable gradients and the behind/over/through decoration comparison.
///
/// The gallery is taller than a small window, so it scrolls vertically.
pub fn gradient_example() -> impl Widget {
    Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .child(GradientExample::new())
}

/// Demonstrates GPU-evaluated gradients and where a decoration can paint.
#[derive(StatefulWidget)]
pub struct GradientExample {}

impl GradientExample {
    /// Creates a gradient gallery with a 135° sunset, a spotlight and a color wheel.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for GradientExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct GradientExampleState {
    angle: f64,
    radius: f64,
    start: f64,
    corners: f64,
    // The cards' content never changes, so it is built once and shared.
    blanks: [ChildBuilder; 3],
    artworks: [ChildBuilder; 2],
    caption: ChildBuilder,
    glass_caption: ChildBuilder,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for GradientExample {
    type State = GradientExampleState;

    fn create_state(self) -> Self::State {
        GradientExampleState {
            angle: 135.0,
            radius: 1.0,
            start: 0.0,
            corners: 20.0,
            blanks: std::array::from_fn(|_| {
                ChildBuilder::from_widget(
                    SizedBox::new()
                        .width(CARD_WIDTH)
                        .height(CARD_HEIGHT),
                )
            }),
            artworks: std::array::from_fn(|_| {
                ChildBuilder::from_widget(
                    Container::new()
                        .box_decoration(
                            BoxDecoration::new()
                                .background_color(Color::Rgb(24, 42, 76))
                                .border_radius(16),
                        )
                        .child(
                            SizedBox::new()
                                .width(CARD_WIDTH)
                                .height(CARD_HEIGHT),
                        ),
                )
            }),
            caption: ChildBuilder::from_widget(
                Text::new("Rainbow text").text_style(
                    TextStyle::new()
                        .font_size(44)
                        .font_weight(FontWeight::Bold)
                        .color(Color::WHITE),
                ),
            ),
            glass_caption: ChildBuilder::from_widget(
                Text::new("Glass text").text_style(
                    TextStyle::new()
                        .font_size(56)
                        .font_weight(FontWeight::Bold)
                        .color(Color::WHITE),
                ),
            ),
            updater: StateUpdater::empty(),
        }
    }
}

impl State<GradientExample> for GradientExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let updater = self.updater;
        Column::new().gaps(LayoutSpacing::all(Spacing::Px(16))).children([
            note("Gradients are evaluated per pixel on the GPU, so they stay smooth at any size and scale."),
            Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                control(format!("Linear angle: {:.0}°", self.angle), self.angle, 0.0..360.0, 5.0,
                    move |value| updater.set_state(move |state| state.angle = value)),
                control(format!("Radial radius: {:.2}", self.radius), self.radius, 0.1..1.5, 0.05,
                    move |value| updater.set_state(move |state| state.radius = value)),
            ]).boxed(),
            Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                control(format!("Conic start: {:.0}°", self.start), self.start, 0.0..360.0, 5.0,
                    move |value| updater.set_state(move |state| state.start = value)),
                control(format!("Corner radius: {:.0} px", self.corners), self.corners, 0.0..60.0, 1.0,
                    move |value| updater.set_state(move |state| state.corners = value)),
            ]).boxed(),
            Row::new().gaps(LayoutSpacing::all(Spacing::Px(16))).children([
                self.card(0, Gradient::linear()
                    .angle(self.angle as f32)
                    .stop(0.0, Color::Rgb(255, 94, 98))
                    .stop(1.0, Color::Rgb(255, 195, 113))),
                self.card(1, Gradient::radial()
                    .radius(self.radius as f32)
                    .colors([Color::Rgba(255, 255, 255, 230), Color::Rgb(60, 120, 255)])),
                self.card(2, Gradient::conic()
                    .from_angle(self.start as f32)
                    .colors([Color::RED, Color::Rgb(255, 220, 0), Color::Rgb(0, 200, 90),
                        Color::Rgb(0, 140, 255), Color::RED])),
            ]).boxed(),
            note("Behind or over: the same light edge on an opaque card. Behind it, the card covers the rim; over it, the rim stays visible."),
            Container::new().padding(LayoutSpacing::all(Spacing::Px(24)))
                .box_decoration(BoxDecoration::new().background_color(Color::Rgb(9, 16, 30)).border_radius(16))
                .child(Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                    DecorationBox::new()
                        .decoration(vec![
                            BorderRadius::from(16).notype(),
                            LightEdge::new().width(10.0).notype(),
                        ])
                        .child(self.artworks[0].clone())
                        .boxed(),
                    ForegroundDecorationBox::new()
                        .decoration(vec![
                            BorderRadius::from(16).notype(),
                            LightEdge::new().width(10.0).notype(),
                        ])
                        .child(self.artworks[1].clone())
                        .boxed(),
                ]))
                .boxed(),
            note("Through: a decoration shown only where the child has coverage, so a rainbow becomes rainbow text."),
            MaskedDecorationBox::new()
                .decoration(Gradient::rainbow())
                .child(self.caption.clone())
                .boxed(),
            note("Glass text: a backdrop filter masked by text blurs and brightens what is behind only inside the glyphs."),
            DecorationBox::new()
                .decoration(vec![
                    BorderRadius::from(16).notype(),
                    Gradient::conic()
                        .colors([Color::RED, Color::Rgb(255, 220, 0), Color::Rgb(0, 200, 90),
                            Color::Rgb(0, 140, 255), Color::RED])
                        .notype(),
                ])
                .child(Container::new().padding(LayoutSpacing::all(Spacing::Px(24)))
                    .child(MaskedDecorationBox::new()
                        .decoration(BackdropFilter::new().blur_radius(10.0).brightness(1.8))
                        .child(self.glass_caption.clone())))
                .boxed(),
        ])
    }
}

impl GradientExampleState {
    /// A gradient behind an empty card, rounded by the shared corner radius.
    fn card(&self, index: usize, gradient: Gradient) -> AnyWidget {
        DecorationBox::new()
            .decoration(vec![
                BorderRadius::from(self.corners as f32).notype(),
                gradient.notype(),
            ])
            .child(self.blanks[index].clone())
            .boxed()
    }
}

fn note(text: &'static str) -> AnyWidget {
    Text::new(text)
        .text_style(
            TextStyle::new()
                .font_size(14)
                .color(theme::app_theme().on_background_color),
        )
        .wrapped()
        .boxed()
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
