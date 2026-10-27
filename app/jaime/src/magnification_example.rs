//! A retained text backdrop viewed through an adjustable glass lens.

use crate::theme;
use aimer::decoration::{
    BorderRadius, Decoration, DecorationBox, Magnification, MagnificationStyle as Style,
};
use aimer::range::Slider;
use aimer::selection::{Checkbox, CheckboxValue, ChoiceOption, Select};
use aimer::style::{
    BorderSlice, BorderStyle, BoxBorder, BoxDecoration, Color, FontWeight, LayoutSpacing, Spacing,
    Stroke, TextStyle,
};
use aimer::{
    AimerApp, AnyWidget, BuildContext, ChildBuilder, Column, Container, Positioned, Row, Stack,
    State, StateUpdater, StatefulWidget, Text, Widget,
};

/// Starts the standalone magnification example.
pub fn start_magnification_example() {
    AimerApp::start(theme::provide(magnification_example()));
}

/// Builds an interactive lens with text behind it and a sharp foreground label.
pub fn magnification_example() -> impl Widget {
    MagnificationExample::new()
}

/// Demonstrates composable zoom, bulge, and edge refraction.
#[derive(StatefulWidget)]
pub struct MagnificationExample {}

impl MagnificationExample {
    /// Creates the example with a 1.5× zoom and 16-pixel glass thickness.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for MagnificationExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MagnificationExampleState {
    zoom: f64,
    thickness: f64,
    bulge: f64,
    style: Style,
    backdrop: ChildBuilder,
    foreground: ChildBuilder,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for MagnificationExample {
    type State = MagnificationExampleState;
    fn create_state(self) -> Self::State {
        MagnificationExampleState {
            zoom: 1.5,
            thickness: 16.0,
            bulge: 0.25,
            style: Style::DEFAULT,
            backdrop: ChildBuilder::from_widget(text_backdrop()),
            foreground: ChildBuilder::from_widget(foreground()),
            updater: StateUpdater::empty(),
        }
    }
}

impl State<MagnificationExample> for MagnificationExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let zoom_updater = self.updater;
        let depth_updater = self.updater;
        let bulge_updater = self.updater;
        let text_style = TextStyle::new()
            .font_size(15)
            .color(theme::app_theme().on_background_color);
        Column::new()
            .gaps(LayoutSpacing::all(Spacing::Px(12)))
            .children([
                Text::new(
                    "Combine optical effects, then choose the axes and the lens's enlargement anchor.",
                )
                .text_style(text_style)
                .wrapped()
                .boxed(),
                Row::new()
                    .gaps(LayoutSpacing::all(Spacing::Px(16)))
                    .children([
                        effect_checkbox("Uniform zoom", Style::UNIFORM, self.style, self.updater),
                        effect_checkbox("Bulge", Style::BULGE, self.style, self.updater),
                        effect_checkbox("Edge refraction", Style::REFRACTION, self.style, self.updater),
                    ])
                    .boxed(),
                Row::new()
                    .gaps(LayoutSpacing::all(Spacing::Px(12)))
                    .children([
                        style_select("Axes", Style::HORIZONTAL | Style::VERTICAL,
                            [("Both axes", Style::NONE), ("Horizontal", Style::HORIZONTAL), ("Vertical", Style::VERTICAL)],
                            self.style, self.updater),
                        style_select("Horizontal anchor", Style::LEFT | Style::RIGHT,
                            [("Center X", Style::NONE), ("Left", Style::LEFT), ("Right", Style::RIGHT)],
                            self.style, self.updater),
                        style_select("Vertical anchor", Style::TOP | Style::BOTTOM,
                            [("Center Y", Style::NONE), ("Top", Style::TOP), ("Bottom", Style::BOTTOM)],
                            self.style, self.updater),
                    ])
                    .boxed(),
                Text::new(format!("Zoom: {:.1}×", self.zoom)).text_style(text_style).boxed(),
                Slider::new()
                    .range(1.0..4.0)
                    .step(0.05)
                    .value(self.zoom)
                    .width(320.0)
                    .on_change(move |value| zoom_updater.set_state(move |state| state.zoom = value))
                    .boxed(),
                Text::new(format!("Thickness: {:.0} px", self.thickness)).text_style(text_style).boxed(),
                Slider::new()
                    .range(0.0..64.0)
                    .step(1.0)
                    .value(self.thickness)
                    .width(320.0)
                    .on_change(move |value| {
                        depth_updater.set_state(move |state| state.thickness = value)
                    })
                    .boxed(),
                Text::new(format!("Bulge strength: {:.2}", self.bulge)).text_style(text_style).boxed(),
                Slider::new()
                    .range(0.0..1.0)
                    .step(0.05)
                    .value(self.bulge)
                    .width(320.0)
                    .on_change(move |value| {
                        bulge_updater.set_state(move |state| state.bulge = value)
                    })
                    .boxed(),
                Container::new()
                    .width(640)
                    .height(400)
                    .color(Color::Rgb(218, 237, 247))
                    .child(
                        Stack::new().children([
                            self.backdrop
                                .clone()
                                .boxed(),
                            Positioned::new()
                                .left(160)
                                .top(92)
                                .child(
                                    DecorationBox::new()
                                        .decoration(vec![
                                            BorderRadius::from(16.0).notype(),
                                            Magnification::new()
                                                .style(self.style)
                                                .bulge(self.bulge as f32)
                                                .zoom(self.zoom as f32)
                                                .thickness(self.thickness as f32)
                                                .notype(),
                                        ])
                                        .child(
                                            self.foreground
                                                .clone(),
                                        ),
                                )
                                .boxed(),
                        ]),
                    )
                    .boxed(),
            ])
    }
}

fn effect_checkbox(
    label: &'static str,
    flag: Style,
    style: Style,
    updater: StateUpdater<MagnificationExampleState>,
) -> AnyWidget {
    Checkbox::new()
        .label(label)
        .value(if style.contains(flag) {
            CheckboxValue::Checked
        } else {
            CheckboxValue::Unchecked
        })
        .on_change(move |value| {
            updater.set_state(move |state| {
                if value == CheckboxValue::Checked {
                    state.style |= flag;
                } else {
                    state.style &= !flag;
                }
            })
        })
        .boxed()
}

fn style_select(
    label: &'static str,
    mask: Style,
    choices: [(&'static str, Style); 3],
    style: Style,
    updater: StateUpdater<MagnificationExampleState>,
) -> AnyWidget {
    Container::new()
        .width(200)
        .child(
            Select::new()
                .options(choices.map(|(label, value)| ChoiceOption::new(label, label, value)))
                .label(label)
                .selected(Some(style & mask))
                .on_change(move |value| {
                    updater.set_state(move |state| state.style = (state.style & !mask) | value)
                }),
        )
        .boxed()
}

const TEXT_ITEMS: &[&str] = &[
    "AIMER / TEXT / AIMER / TEXT",
    "WORDS BEHIND THE LENS",
    "AIMER / ZOOM / AIMER",
    "MAGNIFICATION / AIMER",
    "AIMER / TEXT / AIMER / TEXT",
    "WORDS BEHIND THE LENS",
    "AIMER / ZOOM / AIMER",
];

fn text_backdrop() -> impl Widget {
    Container::new()
        .width(640)
        .height(400)
        .child(
            Stack::new().children(
                TEXT_ITEMS
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| {
                        Positioned::new()
                            .left(24)
                            .top(24 + index as i32 * 52)
                            .child(
                                Text::new(*text).text_style(
                                    TextStyle::new()
                                        .font_size(28)
                                        .font_weight(FontWeight::Bold)
                                        .color(if index % 2 == 0 {
                                            Color::Rgb(28, 81, 126)
                                        } else {
                                            Color::Rgb(119, 65, 145)
                                        }),
                                ),
                            )
                    }),
            ),
        )
}

fn foreground() -> impl Widget {
    Container::new()
        .width(320)
        .height(208)
        .padding(LayoutSpacing::all(Spacing::Px(24)))
        .box_decoration(
            BoxDecoration::new()
                .border_radius(16)
                .border(BoxBorder::all(
                    BorderSlice::new()
                        .style(BorderStyle::Solid)
                        .stroke(Stroke::Px(1.0))
                        .color(Color::Rgba(255, 255, 255, 220)),
                )),
        )
        .child(
            Text::new("This text stays sharp").text_style(
                TextStyle::new()
                    .font_size(20)
                    .font_weight(FontWeight::Bold)
                    .color(Color::Rgb(16, 39, 65)),
            ),
        )
}
