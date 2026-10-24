//! A decoration painted over its child, toggled by a button.
//!
//! The card's own text and background paint first. The foreground decoration
//! paints after them: a magnifying lens, a blur, a dark fade and a light rim.
//! Because it paints last it can sample the card, so the lens enlarges and the
//! blur softens the text beneath it, while the fade and rim sit on top. The
//! button and the two sliders rebuild the box with a different decoration stack;
//! the card itself is retained and never rebuilt.
//! 
use aimer::decoration::{
    AnyDecor, BackdropFilter, BorderRadius, Decoration, DecorationBox, ForegroundDecorationBox,
    Gradient, LightEdge, Magnification,
};
use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::range::Slider;
use aimer::style::{
    BoxDecoration, Color, FontWeight, LayoutSpacing, Spacing, TextAlign, TextStyle,
};
use aimer::{
    AimerApp, AnyWidget, BuildContext, ChildBuilder, Column, Container, Row, State, StateUpdater,
    StatefulWidget, Text, Widget,
};

use crate::theme;

const CARD_WIDTH: u32 = 320;
const CARD_HEIGHT: u32 = 180;
const CARD_RADIUS: f32 = 20.0;

/// Starts the standalone foreground decoration example.
pub fn start_foreground_decoration_example() {
    AimerApp::start(theme::provide(foreground_decoration_example()));
}

/// Builds a card whose foreground decoration a button shows and hides.
pub fn foreground_decoration_example() -> impl Widget {
    ForegroundDecorationExample::new()
}

/// Demonstrates a decoration that paints over its child.
#[widget(Stateful)]
pub struct ForegroundDecorationExample {}

impl ForegroundDecorationExample {
    /// Creates the example with its decoration shown.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for ForegroundDecorationExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ForegroundDecorationExampleState {
    shown: bool,
    /// Backdrop blur radius in logical pixels; zero adds no blur layer.
    blur: f64,
    /// Lens zoom; one adds no lens layer.
    zoom: f64,
    // One position in the tree, so one builder: the card is placed only once.
    card: ChildBuilder,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for ForegroundDecorationExample {
    type State = ForegroundDecorationExampleState;

    fn create_state(self) -> Self::State {
        ForegroundDecorationExampleState {
            shown: true,
            blur: 3.0,
            zoom: 1.25,
            card: ChildBuilder::from_widget(card()),
            updater: StateUpdater::empty(),
        }
    }
}

impl State<ForegroundDecorationExample> for ForegroundDecorationExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let updater = self.updater;
        Column::new().gaps(LayoutSpacing::all(Spacing::Px(16))).children([
            note("The card paints first. The foreground decoration paints after it, so the fade darkens the card's text and the rim stays on top of its edge."),
            toggle_button(self.shown, move || {
                updater.set_state(|state| state.shown = !state.shown)
            }),
            Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                control(format!("Blur: {:.0} px", self.blur), self.blur, 0.0..24.0, 1.0,
                    move |value| updater.set_state(move |state| state.blur = value)),
                control(format!("Magnification: {:.2}x", self.zoom), self.zoom, 1.0..2.5, 0.05,
                    move |value| updater.set_state(move |state| state.zoom = value)),
            ]).boxed(),
            note(if self.shown {
                "Foreground decoration: shown"
            } else {
                "Foreground decoration: hidden"
            }),
            Container::new()
                .padding(LayoutSpacing::all(Spacing::Px(24)))
                .box_decoration(
                    BoxDecoration::new()
                        .background_color(Color::Rgb(9, 16, 30))
                        .border_radius(16),
                )
                .child(
                    ForegroundDecorationBox::new()
                        .decoration(self.layers())
                        .child(self.card.clone()),
                )
                .boxed(),
        ])
    }
}

impl ForegroundDecorationExampleState {
    /// The decoration stack: empty when hidden, so the box paints nothing over the card.
    fn layers(&self) -> Vec<AnyDecor> {
        if !self.shown {
            return Vec::new();
        }
        let mut layers = vec![BorderRadius::from(CARD_RADIUS).notype()];
        // Both sample what the card already painted, so the lens goes first and
        // the blur softens its enlargement.
        if self.zoom > 1.0 {
            layers.push(Magnification::new().zoom(self.zoom as f32).thickness(12.0).notype());
        }
        if self.blur > 0.0 {
            layers.push(BackdropFilter::new().blur_radius(self.blur as f32).notype());
        }
        // A fade from clear at the top to dark at the bottom.
        layers.push(
            Gradient::linear()
                .angle(180.0)
                .stop(0.0, Color::Rgba(0, 0, 0, 0))
                .stop(1.0, Color::Rgba(0, 0, 0, 190))
                .notype(),
        );
        layers.push(LightEdge::new().width(10.0).strength(0.9).notype());
        layers
    }
}

/// The card: a gradient surface with a title and a line of text near its bottom.
fn card() -> impl Widget {
    DecorationBox::new()
        .decoration(vec![
            BorderRadius::from(CARD_RADIUS).notype(),
            Gradient::linear()
                .angle(135.0)
                .colors([Color::Rgb(80, 120, 255), Color::Rgb(190, 90, 255)])
                .notype(),
        ])
        .child(
            Container::new()
                .width(CARD_WIDTH)
                .height(CARD_HEIGHT)
                .padding(LayoutSpacing::all(Spacing::Px(24)))
                .child(
                    Column::new()
                        .gaps(LayoutSpacing::all(Spacing::Px(12)))
                        .children([
                            Text::new("Over, not behind")
                                .text_style(
                                    TextStyle::new()
                                        .font_size(26)
                                        .font_weight(FontWeight::Bold)
                                        .color(Color::WHITE),
                                )
                                .boxed(),
                            Text::new("This text is drawn by the card. The fade covers it.")
                                .text_style(TextStyle::new().font_size(15).color(Color::WHITE))
                                .wrapped()
                                .boxed(),
                        ]),
                ),
        )
}

fn toggle_button(shown: bool, on_press: impl Fn() + 'static) -> AnyWidget {
    let app_theme = theme::app_theme();
    Container::new()
        .width(200)
        .height(44)
        .box_child(
            Button::new()
                .on_press(on_press)
                .decoration(
                    BoxDecoration::new()
                        .background_color(app_theme.primary_color)
                        .border_radius((8, 8, 8, 8)),
                )
                .child(
                    Text::new(if shown { "Hide decoration" } else { "Show decoration" })
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(15)
                                .color(app_theme.on_primary_color),
                        ),
                ),
        )
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
