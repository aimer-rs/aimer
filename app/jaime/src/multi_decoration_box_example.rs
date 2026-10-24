//! Typed and runtime decoration stacks sharing one retained child.

use aimer::decoration::{Decoration, LightEdge, MultiDecorationBox};
use aimer::macros::widget;
use aimer::selection::{Checkbox, CheckboxValue};
use aimer::style::{BoxDecoration, Color, FontWeight, LayoutSpacing, Spacing, TextStyle};
use aimer::{
    AimerApp, BuildContext, ChildBuilder, Column, Container, Row, State, StateUpdater,
    StatefulWidget, Text, Widget,
};

use crate::theme;

/// Starts the standalone multiple-decoration example.
pub fn start_multi_decoration_box_example() {
    AimerApp::start(theme::provide(multi_decoration_box_example()));
}

/// Builds typed and runtime stacks with controls for runtime layer composition.
pub fn multi_decoration_box_example() -> impl Widget {
    MultiDecorationBoxExample::new()
}

/// Demonstrates decorations painting in insertion order before a retained child.
#[widget(Stateful)]
pub struct MultiDecorationBoxExample {}

impl MultiDecorationBoxExample {
    /// Creates a background followed by a light edge in both preview cards.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for MultiDecorationBoxExample {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MultiDecorationBoxExampleState {
    include_edge: bool,
    reverse: bool,
    typed_content: ChildBuilder,
    runtime_content: ChildBuilder,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for MultiDecorationBoxExample {
    type State = MultiDecorationBoxExampleState;

    fn create_state(self) -> Self::State {
        MultiDecorationBoxExampleState {
            include_edge: true,
            reverse: false,
            typed_content: ChildBuilder::from_widget(panel_content(
                "Typed chain",
                "A background and light edge appended with decoration().",
            )),
            runtime_content: ChildBuilder::from_widget(panel_content(
                "Runtime list",
                "An owned collection of decorations, in the selected order.",
            )),
            updater: StateUpdater::empty(),
        }
    }
}

impl State<MultiDecorationBoxExample> for MultiDecorationBoxExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let updater = self.updater;
        let mut layers: Vec<Box<dyn Decoration>> = Vec::with_capacity(2);
        layers.push(Box::new(background()));
        if self.include_edge {
            layers.push(Box::new(edge()));
        }
        if self.reverse {
            layers.reverse();
        }
        Column::new().gaps(LayoutSpacing::all(Spacing::Px(16))).children([
            Text::new("Each card stacks decorations behind its text. Change the runtime list: reversing it lets the opaque background cover the light edge.")
                .text_style(TextStyle::new().font_size(15).color(theme::app_theme().on_background_color))
                .wrapped().boxed(),
            Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                Checkbox::new().label("Include light edge")
                    .value(if self.include_edge { CheckboxValue::Checked } else { CheckboxValue::Unchecked })
                    .on_change(move |value| updater.set_state(move |state| state.include_edge = value == CheckboxValue::Checked))
                    .boxed(),
                Checkbox::new().label("Reverse layer order")
                    .value(if self.reverse { CheckboxValue::Checked } else { CheckboxValue::Unchecked })
                    .on_change(move |value| updater.set_state(move |state| state.reverse = value == CheckboxValue::Checked))
                    .boxed(),
            ]).boxed(),
            Container::new().padding(LayoutSpacing::all(Spacing::Px(24)))
                .box_decoration(BoxDecoration::new().background_color(Color::Rgb(9, 16, 30)).border_radius(16))
                .child(Row::new().gaps(LayoutSpacing::all(Spacing::Px(24))).children([
                    MultiDecorationBox::new().decoration(background()).decoration(edge())
                        .box_child(self.typed_content.clone()),
                    MultiDecorationBox::new().decorations(layers).box_child(self.runtime_content.clone()),
                ])).boxed(),
        ])
    }
}

fn background() -> BoxDecoration {
    BoxDecoration::new()
        .background_color(Color::Rgb(24, 42, 76))
        .border_radius(24)
}

fn edge() -> LightEdge {
    LightEdge::new()
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
