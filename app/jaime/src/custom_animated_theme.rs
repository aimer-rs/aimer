use std::time::Duration;

use aimer::console::debug;
use aimer::input::button::Button;
use aimer::style::*;
use aimer::*;

#[derive(Clone, Copy, Debug, PartialEq, Theme)]
struct MyTheme {
    background_red: u8,
    background_green: u8,
    background_blue: u8,
    foreground_tone: u8,
    accent_red: u8,
    accent_green: u8,
    accent_blue: u8,
    panel_width: f32,
    button_radius: i32,
}

impl MyTheme {
    fn light() -> Self {
        Self {
            background_red: 244,
            background_green: 247,
            background_blue: 255,
            foreground_tone: 28,
            accent_red: 42,
            accent_green: 99,
            accent_blue: 210,
            panel_width: 280.0,
            button_radius: 12,
        }
    }

    fn dark() -> Self {
        Self {
            background_red: 18,
            background_green: 24,
            background_blue: 38,
            foreground_tone: 240,
            accent_red: 120,
            accent_green: 174,
            accent_blue: 255,
            panel_width: 320.0,
            button_radius: 28,
        }
    }
}

pub fn start_custom_animated_theme_example() {
    AimerApp::start(CustomAnimatedThemeExample::new())
}

#[derive(StatefulWidget)]
struct CustomAnimatedThemeExample {}

impl CustomAnimatedThemeExample {
    /// Creates the custom animated-theme showcase.
    #[inline]
    fn new() -> Self {
        Self {}
    }
}

/// Builds the custom animated-theme showcase without starting an application.
pub fn custom_animated_theme_example() -> impl Widget {
    CustomAnimatedThemeExample::new()
}

struct CustomAnimatedThemeExampleState {
    is_dark: bool,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for CustomAnimatedThemeExample {
    type State = CustomAnimatedThemeExampleState;

    fn create_state(self) -> Self::State {
        CustomAnimatedThemeExampleState {
            is_dark: false,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<CustomAnimatedThemeExample> for CustomAnimatedThemeExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        AnimatedTheme::new()
            .data(if self.is_dark {
                MyTheme::dark()
            } else {
                MyTheme::light()
            })
            .duration(Duration::from_millis(500))
            .curve(curves::EASE_IN_OUT)
            .child(CustomThemedPanel::new(self.is_dark, self.updater))
    }
}

#[derive(Clone)]
#[derive(StatelessWidget)]
struct CustomThemedPanel {
    is_dark: bool,
    updater: StateUpdater<CustomAnimatedThemeExampleState>,
}

impl CustomThemedPanel {
    fn new(is_dark: bool, updater: StateUpdater<CustomAnimatedThemeExampleState>) -> Self {
        Self { is_dark, updater }
    }
}

impl StatelessWidget for CustomThemedPanel {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = MyTheme::of(ctx);
        let panel_width = ctx
            .copied::<MyTheme>()
            .panel_width;
        let updater = self.updater;
        let background = Color::Rgba(
            theme.background_red,
            theme.background_green,
            theme.background_blue,
            255,
        );
        let foreground = Color::Rgba(
            theme.foreground_tone,
            theme.foreground_tone,
            theme.foreground_tone,
            255,
        );
        let accent = Color::Rgba(theme.accent_red, theme.accent_green, theme.accent_blue, 255);

        Container::new()
            .color(background)
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Center)
                    .vertical_alignment(BoxAlignment::Center)
                    .children([
                        Text::new("Derived custom AnimatedTheme")
                            .text_style(
                                TextStyle::new()
                                    .font_size(26)
                                    .color(foreground),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(24)
                            .boxed(),
                        Text::new("Color channels, width, and radius interpolate together.")
                            .text_style(
                                TextStyle::new()
                                    .font_size(14)
                                    .color(foreground),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(32)
                            .boxed(),
                        Container::new()
                            .width(Dimension::Px(panel_width))
                            .height(Dimension::Px(56.0))
                            .child(
                                Button::new()
                                    .on_press(move || {
                                        debug!("custom theme button pressed");
                                        updater.set_state(|state| state.is_dark = !state.is_dark);
                                    })
                                    .decoration(
                                        BoxDecoration::new()
                                            .background_color(accent)
                                            .border_radius(theme.button_radius),
                                    )
                                    .child(
                                        Text::new(if self.is_dark {
                                            "Switch to custom light theme"
                                        } else {
                                            "Switch to custom dark theme"
                                        })
                                        .text_align(TextAlign::MidCenter)
                                        .text_style(
                                            TextStyle::new()
                                                .font_size(16)
                                                .color(background),
                                        ),
                                    ),
                            )
                            .boxed(),
                    ]),
            )
    }
}
