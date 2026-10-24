use std::sync::Arc;

use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::style::*;
use aimer::{AimerApp, *};

use crate::theme;

// this is the entry point of the app
pub fn start_counter() {
    // simply start the app with AimerApp::start
    AimerApp::start(theme::provide(CounterWidget::new(1).boxed()))
}

// creating a widget with state
#[allow(non_snake_case)]
#[widget(Stateful)]
pub struct CounterWidget {
    pub initial_count: i32,
}

impl CounterWidget {
    pub fn new(initial_count: i32) -> Self {
        Self { initial_count }
    }
}
// create a state for the CounterWidget
pub struct CounterState {
    count: i32,
    on_loading: bool,
    shared: Arc<u32>,
    updater: StateUpdater<Self>,
}

// implement the StatefulWidget trait for CounterWidget
impl StatefulWidget for CounterWidget {
    type State = CounterState;

    fn create_state(self) -> CounterState {
        CounterState {
            count: self.initial_count,
            updater: StateUpdater::empty(),
            on_loading: false,
            shared: Arc::new(0),
        }
    }
}
// implement the State trait for CounterState
impl State<CounterWidget> for CounterState {
    fn init_state(&mut self, _updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = _updater
    }

    // build the widget with state
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        // debug!("self.count: {}", self.count);
        let updater = self.updater;
        let theme = ThemeData::copied(ctx);
        Container::new()
            .color(theme.background_color)
            .padding(LayoutSpacing {
                top: Spacing::Px(20),
                ..Default::default()
            })
            .child(
                Flex::new()
                    .direction(FlexDirection::Column)
                    .vertical_alignment(BoxAlignment::Center)
                    .horizontal_alignment(BoxAlignment::Center)
                    .children([
                        Text::new("Widget with State")
                            .text_style(
                                TextStyle::new()
                                    .font_size(25)
                                    .color(theme.on_background_color),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(50)
                            .boxed(),
                        Text::new(format!("Clicked: {}", self.count,))
                            .text_style(
                                TextStyle::new()
                                    .font_size(25)
                                    .color(theme.on_background_color),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(50)
                            .boxed(),
                        Container::new()
                            .width(Dimension::Px(200.0))
                            .height(Dimension::Px(50.0))
                            .child(
                                Button::new()
                                    .disabled(self.on_loading)
                                    .on_press(move || {
                                        // updater.set_state(|state| state.on_loading = true);

                                        // updater.set_state(|state| state.on_loading = false);

                                        println!(
                                            "Button pressed with state : {}",
                                            updater
                                                .read_state()
                                                .count
                                        );
                                        updater.set_state(|state| {
                                            state.count += 1;
                                        });
                                    })
                                    .decoration(
                                        BoxDecoration::new().background_color(theme.primary_color),
                                    )
                                    .child(
                                        Text::new(if self.on_loading {
                                            "Loading..."
                                        } else {
                                            "Click Me"
                                        })
                                        .text_align(TextAlign::MidCenter)
                                        .text_style(TextStyle::new().color(theme.on_primary_color)),
                                    )
                                    .boxed(),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(20)
                            .boxed(),
                        Container::new()
                            .width(Dimension::Px(200.0))
                            .height(Dimension::Px(50.0))
                            .child(
                                Button::new()
                                    .disabled(self.on_loading)
                                    .on_press({
                                        let updater = self.updater;
                                        move || {
                                            println!(
                                                "Button pressed with state : {}",
                                                updater
                                                    .read_state()
                                                    .count
                                            );
                                            updater.set_state(|state| {
                                                state.count -= 1;
                                            });
                                        }
                                    })
                                    .decoration(
                                        BoxDecoration::new().background_color(theme.primary_color),
                                    )
                                    .child(
                                        Text::new(if self.on_loading {
                                            "Loading..."
                                        } else {
                                            "Click Me"
                                        })
                                        .text_align(TextAlign::MidCenter)
                                        .text_style(TextStyle::new().color(theme.on_primary_color)),
                                    )
                                    .boxed(),
                            )
                            .boxed(),
                    ]),
            )
    }
}
