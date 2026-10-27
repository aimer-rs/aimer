use std::sync::Arc;

use aimer::input::button::Button;
use aimer::input::callback::VoidCallback;
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
#[derive(StatefulWidget)]
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
                        self.click_button("Increase", theme, move || {
                            updater.set_state(|state| state.count += 1)
                        }),
                        SizedBox::new()
                            .height(20)
                            .boxed(),
                        self.click_button("Decrease", theme, move || {
                            updater.set_state(|state| state.count -= 1)
                        }),
                    ]),
            )
    }
}

impl CounterState {
    fn click_button(
        &self,
        label: &'static str,
        theme: ThemeData,
        onclick: impl Into<VoidCallback>,
    ) -> AnyWidget {
        Button::new()
            .disabled(self.on_loading)
            .on_press(onclick)
            .decoration(BoxDecoration::new().background_color(theme.primary_color))
            .box_child(
                Container::new()
                    .width(Dimension::Px(200.0))
                    .height(Dimension::Px(50.0))
                    .child(
                        Text::new(case!(self.on_loading, "Loading", label))
                            .text_align(TextAlign::MidCenter)
                            .text_style(TextStyle::new().color(theme.on_primary_color)),
                    ),
            )
    }
}
