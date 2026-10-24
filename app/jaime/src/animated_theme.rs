use std::time::Duration;

use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::style::*;
use aimer::*;

pub fn start_animated_theme_example() {
    AimerApp::start(AnimatedThemeExample::new())
}

#[widget(Stateful)]
pub struct AnimatedThemeExample {}

impl AnimatedThemeExample {
    pub fn new() -> Self {
        Self {}
    }
}

pub struct AnimatedThemeExampleState {
    is_dark: bool,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for AnimatedThemeExample {
    type State = AnimatedThemeExampleState;

    fn create_state(self) -> Self::State {
        AnimatedThemeExampleState {
            is_dark: false,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<AnimatedThemeExample> for AnimatedThemeExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        AnimatedTheme::new()
            .data(if self.is_dark {
                ThemeData::dark()
            } else {
                ThemeData::light()
            })
            .duration(Duration::from_millis(400))
            .curve(curves::EASE_IN_OUT)
            .child(ThemedPanel::new(self.is_dark, self.updater))
    }
}

#[derive(Clone)]
#[widget(Stateless)]
struct ThemedPanel {
    is_dark: bool,
    updater: StateUpdater<AnimatedThemeExampleState>,
}

impl ThemedPanel {
    fn new(is_dark: bool, updater: StateUpdater<AnimatedThemeExampleState>) -> Self {
        Self { is_dark, updater }
    }
}

impl StatelessWidget for ThemedPanel {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::of(ctx);
        let updater = self.updater;
        #[cfg(test)]
        tests::BUILT_BACKGROUNDS.with(|seen| {
            seen.borrow_mut()
                .push(theme.background_color)
        });

        Container::new()
            .color(theme.background_color)
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Center)
                    .vertical_alignment(BoxAlignment::Center)
                    .children([
                        Text::new("AnimatedTheme")
                            .text_style(
                                TextStyle::new()
                                    .font_size(32)
                                    .color(theme.on_background_color),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(24)
                            .boxed(),
                        Text::new("Colors interpolate while the widget tree keeps its state.")
                            .text_style(
                                TextStyle::new()
                                    .font_size(18)
                                    .color(theme.on_background_color),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(32)
                            .boxed(),
                        Container::new()
                            .width(Dimension::Px(220.0))
                            .height(Dimension::Px(56.0))
                            .child(
                                Button::new()
                                    .on_press(move || {
                                        updater.set_state(|state| state.is_dark = !state.is_dark);
                                    })
                                    .decoration(
                                        BoxDecoration::new()
                                            .background_color(theme.primary_color)
                                            .border_radius(12),
                                    )
                                    .child(
                                        Text::new(if self.is_dark {
                                            "Switch to light theme"
                                        } else {
                                            "Switch to dark theme"
                                        })
                                        .text_align(TextAlign::MidCenter)
                                        .text_style(
                                            TextStyle::new()
                                                .font_size(16)
                                                .color(theme.on_primary_color),
                                        ),
                                    ),
                            )
                            .boxed(),
                    ]),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use aimer::Color;
    use aimer::quiver::winit::dpi::PhysicalPosition;
    use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};

    use super::*;

    thread_local! {
        pub(super) static BUILT_BACKGROUNDS: RefCell<Vec<Color>> = const { RefCell::new(Vec::new()) };
    }

    fn click(
        app: &mut aimer::quiver::aimer_app::HeadlessAimerApp<impl Widget + 'static>,
        x: f64,
        y: f64,
    ) {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        });
        app.render_frame();
        for state in [ElementState::Pressed, ElementState::Released] {
            app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: MouseButton::Left,
            });
            app.render_frame();
        }
    }

    #[test]
    fn switching_the_theme_animates_to_the_target_and_stops_asking_for_frames() {
        let mut app = AimerApp::start_headless(AnimatedThemeExample::new());
        app.pump_frames(8);
        BUILT_BACKGROUNDS.with(|seen| {
            seen.borrow_mut()
                .clear()
        });
        let size = app.logical_size();

        click(
            &mut app,
            (size.width / 2.0) as f64,
            // The button sits below the centre by half the two captions' line
            // heights, which depends on the host's fonts. Aim at the middle of
            // the range it can land in, not at the edge of one font's layout.
            (size.height / 2.0 + 58.0) as f64,
        );

        let mut frames = 0;
        while app.take_redraw_request() && frames < 200 {
            app.render_frame();
            frames += 1;
            if let Some(census) = app.paint_source_census() {
                assert!(
                    census
                        .unresolved_roots
                        .is_empty(),
                    "frame {frames}: {census:?}"
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let seen = BUILT_BACKGROUNDS.with(|seen| {
            seen.borrow()
                .clone()
        });
        assert!(
            frames < 200,
            "the 400ms transition never finished ({frames} frames)"
        );
        assert!(seen.len() > 3, "the panel was rebuilt {} times", seen.len());
        let (light, dark) = (
            ThemeData::light().background_color,
            ThemeData::dark().background_color,
        );
        assert_eq!(*seen.last().unwrap(), dark);
        assert!(
            seen.iter()
                .any(|color| *color != light && *color != dark),
            "the panel never saw an in-between theme: {seen:?}"
        );
    }
}
