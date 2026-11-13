use std::time::Duration;

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
    const fn new(
        background: (u8, u8, u8),
        foreground_tone: u8,
        accent: (u8, u8, u8),
        panel_width: f32,
        button_radius: i32,
    ) -> Self {
        Self {
            background_red: background.0,
            background_green: background.1,
            background_blue: background.2,
            foreground_tone,
            accent_red: accent.0,
            accent_green: accent.1,
            accent_blue: accent.2,
            panel_width,
            button_radius,
        }
    }

    fn background(&self) -> Color {
        Color::Rgba(
            self.background_red,
            self.background_green,
            self.background_blue,
            255,
        )
    }

    fn foreground(&self) -> Color {
        Color::Rgba(
            self.foreground_tone,
            self.foreground_tone,
            self.foreground_tone,
            255,
        )
    }

    fn accent(&self) -> Color {
        Color::Rgba(self.accent_red, self.accent_green, self.accent_blue, 255)
    }
}

/// One color scheme, in both appearances.
///
/// A palette is a *pair* of themes, which is what `AnimatedTheme::adaptive`
/// takes. Which half is showing is the light/dark decision the theme's
/// controller owns; which pair is in use is an application choice, held by the
/// example's state.
struct Palette {
    name: &'static str,
    light: MyTheme,
    dark: MyTheme,
}

const PALETTES: [Palette; 4] = [
    Palette {
        name: "Indigo",
        light: MyTheme::new((244, 247, 255), 28, (42, 99, 210), 280.0, 12),
        dark: MyTheme::new((18, 24, 38), 240, (120, 174, 255), 320.0, 28),
    },
    Palette {
        name: "Forest",
        light: MyTheme::new((240, 248, 242), 24, (34, 139, 84), 300.0, 8),
        dark: MyTheme::new((14, 30, 22), 232, (96, 200, 140), 300.0, 20),
    },
    Palette {
        name: "Sunset",
        light: MyTheme::new((255, 246, 238), 40, (226, 102, 40), 260.0, 20),
        dark: MyTheme::new((38, 20, 16), 244, (255, 150, 96), 340.0, 6),
    },
    Palette {
        name: "Rose",
        light: MyTheme::new((255, 242, 246), 36, (214, 48, 110), 320.0, 28),
        dark: MyTheme::new((34, 14, 24), 242, (255, 128, 172), 280.0, 10),
    },
];

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

/// Only the palette lives here. Light or dark is not state of this widget: the
/// theme owns it, and the panel below changes it through the controller.
struct CustomAnimatedThemeExampleState {
    palette: usize,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for CustomAnimatedThemeExample {
    type State = CustomAnimatedThemeExampleState;

    fn create_state(self) -> Self::State {
        CustomAnimatedThemeExampleState {
            palette: 0,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<CustomAnimatedThemeExample> for CustomAnimatedThemeExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let palette = &PALETTES[self.palette];
        AnimatedTheme::new()
            .adaptive(palette.light, palette.dark)
            // Starts light; repeating this on every rebuild is not a new
            // decision, so it does not undo what the toggle chose.
            .mode(ThemeMode::Light)
            .duration(Duration::from_millis(500))
            .curve(curves::EASE_IN_OUT)
            .child(CustomThemedPanel::new(self.palette, self.updater))
    }
}

#[derive(Clone)]
#[derive(StatelessWidget)]
struct CustomThemedPanel {
    palette: usize,
    updater: StateUpdater<CustomAnimatedThemeExampleState>,
}

impl CustomThemedPanel {
    fn new(palette: usize, updater: StateUpdater<CustomAnimatedThemeExampleState>) -> Self {
        Self { palette, updater }
    }
}

impl StatelessWidget for CustomThemedPanel {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = MyTheme::of(ctx);
        let controller = theme.controller().clone();
        let is_dark = controller.brightness() == Brightness::Dark;
        let (background, foreground, accent) = (theme.background(), theme.foreground(), theme.accent());
        #[cfg(test)]
        tests::BUILT.with(|seen| seen.borrow_mut().push((background, accent)));

        let swatches: Vec<AnyWidget> = PALETTES
            .iter()
            .enumerate()
            .map(|(index, palette)| {
                // A palette button previews the variant that is showing now, so
                // flipping light/dark re-tints all four.
                let variant = if is_dark { &palette.dark } else { &palette.light };
                let selected = index == self.palette;
                let updater = self.updater;
                Button::new()
                    .on_press(move || {
                        updater.set_state(move |state| state.palette = index);
                    })
                    .decoration(
                        BoxDecoration::new()
                            .background_color(variant.accent())
                            .border_radius(theme.button_radius),
                    )
                    .child(
                        Container::new()
                            .width(Dimension::Px(84.0))
                            .height(Dimension::Px(44.0))
                            .child(
                                Text::new(if selected {
                                    format!("● {}", palette.name)
                                } else {
                                    palette.name.to_string()
                                })
                                .text_align(TextAlign::MidCenter)
                                .text_style(
                                    TextStyle::new()
                                        .font_size(14)
                                        .color(variant.background()),
                                ),
                            ),
                    )
                    .boxed()
            })
            .collect();

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
                        Row::new()
                            .gaps(LayoutSpacing::all(Spacing::Px(8)))
                            .children(swatches)
                            .boxed(),
                        SizedBox::new()
                            .height(24)
                            .boxed(),
                        // The size belongs inside the button: a button is as big as
                        // its child, so a container around it would leave only the
                        // label clickable.
                        Button::new()
                            // The button asks the theme itself to cross over;
                            // nothing above it holds the decision.
                            .on_press(move || controller.toggle())
                            .decoration(
                                BoxDecoration::new()
                                    .background_color(accent)
                                    .border_radius(theme.button_radius),
                            )
                            .child(
                                Container::new()
                                    .width(Dimension::Px(theme.panel_width))
                                    .height(Dimension::Px(56.0))
                                    .child(
                                        Text::new(if is_dark {
                                            "Switch to light variant"
                                        } else {
                                            "Switch to dark variant"
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use aimer::quiver::aimer_app::HeadlessAimerApp;
    use aimer::quiver::winit::dpi::PhysicalPosition;
    use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};

    use super::*;

    thread_local! {
        /// The background and accent the panel last built with.
        pub(super) static BUILT: RefCell<Vec<(Color, Color)>> = const { RefCell::new(Vec::new()) };
    }

    fn last_built() -> (Color, Color) {
        BUILT.with(|seen| *seen.borrow().last().expect("the panel built"))
    }

    fn click(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
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

    /// Renders until the transition has settled and the window stops asking for
    /// frames.
    fn settle(app: &mut HeadlessAimerApp<impl Widget + 'static>) {
        let mut frames = 0;
        while app.take_redraw_request() && frames < 300 {
            app.render_frame();
            frames += 1;
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(frames < 300, "the 500ms transition never finished");
    }

    // The column is centred, so the controls sit at fixed offsets from the
    // middle of the window. They are aimed at the middle of the band each button
    // can land in, because the captions' line heights depend on the host's fonts.
    const PALETTE_ROW_OFFSET: f64 = 12.0;
    const TOGGLE_OFFSET: f64 = 86.0;

    fn start() -> (HeadlessAimerApp<impl Widget + 'static>, f64, f64) {
        BUILT.with(|seen| seen.borrow_mut().clear());
        let mut app = AimerApp::start_headless(CustomAnimatedThemeExample::new());
        app.pump_frames(8);
        let size = app.logical_size();
        (app, (size.width / 2.0) as f64, (size.height / 2.0) as f64)
    }

    #[test]
    fn the_toggle_button_crosses_to_the_dark_variant_through_the_controller() {
        let (mut app, mid_x, mid_y) = start();
        assert_eq!(last_built().0, PALETTES[0].light.background());

        click(&mut app, mid_x, mid_y + TOGGLE_OFFSET);
        settle(&mut app);

        assert_eq!(last_built().0, PALETTES[0].dark.background());
        assert_eq!(last_built().1, PALETTES[0].dark.accent());
    }

    #[test]
    fn a_palette_button_switches_the_pair_and_keeps_the_chosen_appearance() {
        let (mut app, mid_x, mid_y) = start();

        // Dark first, then a palette: the new palette must show its dark half.
        click(&mut app, mid_x, mid_y + TOGGLE_OFFSET);
        settle(&mut app);
        // Rose is the fourth button, right of centre in a row of four.
        click(&mut app, mid_x + 138.0, mid_y + PALETTE_ROW_OFFSET);
        settle(&mut app);

        assert_eq!(last_built().0, PALETTES[3].dark.background());
        assert_eq!(last_built().1, PALETTES[3].dark.accent());
    }

    #[test]
    fn every_palette_has_a_distinct_light_and_dark_variant() {
        for palette in &PALETTES {
            assert_ne!(
                palette.light, palette.dark,
                "{} has identical variants",
                palette.name
            );
            assert!(
                palette.light.background_red > palette.dark.background_red,
                "{}: the light variant should be the brighter background",
                palette.name
            );
        }
    }

    #[test]
    fn palettes_are_distinguishable_by_name_and_accent() {
        for (i, a) in PALETTES.iter().enumerate() {
            for b in &PALETTES[i + 1..] {
                assert_ne!(a.name, b.name);
                assert_ne!(a.light.accent(), b.light.accent(), "{} vs {}", a.name, b.name);
            }
        }
    }
}
