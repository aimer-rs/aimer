//! Following the system appearance, and taking the decision back.
//!
//! `AnimatedTheme` follows the platform out of the box: switching appearance in
//! the system settings while this example runs animates it into the other theme.
//! The two buttons cover the other half of the story — an application may also
//! decide the appearance itself, and say so with [`ThemeMode`].

use std::time::Duration;

use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::style::*;
use aimer::*;

pub fn start_system_theme_example() {
    AimerApp::start(SystemThemeExample::new())
}

#[widget(Stateful)]
pub struct SystemThemeExample {}

impl SystemThemeExample {
    pub fn new() -> Self {
        Self {}
    }
}

pub struct SystemThemeExampleState {
    mode: ThemeMode,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for SystemThemeExample {
    type State = SystemThemeExampleState;

    fn create_state(self) -> Self::State {
        SystemThemeExampleState {
            // The default: whatever the user told the operating system.
            mode: ThemeMode::System,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<SystemThemeExample> for SystemThemeExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        // `AnimatedTheme::new()` already carries the built-in light and dark
        // themes; the mode decides which of them is supplied, and whether the
        // platform is consulted at all.
        AnimatedTheme::new()
            .mode(self.mode)
            .duration(Duration::from_millis(400))
            .curve(curves::EASE_IN_OUT)
            .child(SystemThemePanel::new(self.mode, self.updater))
    }
}

#[derive(Clone)]
#[widget(Stateless)]
struct SystemThemePanel {
    mode: ThemeMode,
    updater: StateUpdater<SystemThemeExampleState>,
}

impl SystemThemePanel {
    fn new(mode: ThemeMode, updater: StateUpdater<SystemThemeExampleState>) -> Self {
        Self { mode, updater }
    }
}

impl StatelessWidget for SystemThemePanel {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::of(ctx);
        // Reading the appearance here rebuilds this panel when the user switches
        // it, which is what keeps the label below truthful.
        let system = ctx.watch_platform_brightness();
        let showing = self
            .mode
            .resolve(system);
        let following = self
            .mode
            .follows_system();

        let toggle_theme = self.updater;
        let toggle_following = self.updater;

        Container::new()
            .color(theme.background_color)
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Center)
                    .vertical_alignment(BoxAlignment::Center)
                    .children([
                        Text::new("System theme sync")
                            .text_style(
                                TextStyle::new()
                                    .font_size(32)
                                    .color(theme.on_background_color),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(24)
                            .boxed(),
                        Text::new(format!("The system asks for: {system:?}"))
                            .text_style(
                                TextStyle::new()
                                    .font_size(18)
                                    .color(theme.on_background_color),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(8)
                            .boxed(),
                        Text::new(if following {
                            format!("Following the system — showing {showing:?}")
                        } else {
                            format!("Ignoring the system — showing {showing:?}")
                        })
                        .text_style(
                            TextStyle::new()
                                .font_size(18)
                                .color(theme.on_background_color),
                        )
                        .boxed(),
                        SizedBox::new()
                            .height(32)
                            .boxed(),
                        // Deciding the appearance is deciding to ignore the system,
                        // so this button pins the opposite of what is on screen.
                        theme_button(
                            if showing.is_dark() {
                                "Switch to light theme"
                            } else {
                                "Switch to dark theme"
                            },
                            &theme,
                            move || {
                                toggle_theme.set_state(move |state| {
                                    state.mode = if showing.is_dark() {
                                        ThemeMode::Light
                                    } else {
                                        ThemeMode::Dark
                                    };
                                });
                            },
                        ),
                        SizedBox::new()
                            .height(16)
                            .boxed(),
                        // Handing the decision back, or taking it away without
                        // changing what is currently on screen.
                        theme_button(
                            if following {
                                "Ignore the system theme"
                            } else {
                                "Follow the system theme"
                            },
                            &theme,
                            move || {
                                toggle_following.set_state(move |state| {
                                    state.mode = if state
                                        .mode
                                        .follows_system()
                                    {
                                        match showing {
                                            Brightness::Light => ThemeMode::Light,
                                            Brightness::Dark => ThemeMode::Dark,
                                        }
                                    } else {
                                        ThemeMode::System
                                    };
                                });
                            },
                        ),
                    ]),
            )
    }
}

/// One themed button, so the two differ only in what they say and what they do.
fn theme_button(
    label: &'static str,
    theme: &ThemeData,
    on_press: impl Fn() + 'static,
) -> AnyWidget {
    Container::new()
        .width(Dimension::Px(260.0))
        .height(Dimension::Px(56.0))
        .child(
            Button::new()
                .on_press(on_press)
                .decoration(
                    BoxDecoration::new()
                        .background_color(theme.primary_color)
                        .border_radius(12),
                )
                .child(
                    Text::new(label)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(16)
                                .color(theme.on_primary_color),
                        ),
                ),
        )
        .boxed()
}
