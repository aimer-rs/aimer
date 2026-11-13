//! Animated GIF playback: a bundled GIF with play, pause, restart and seek
//! controls driven through a [`GifController`].
//!
//! The image widget decodes the GIF a few frames at a time, so the page costs
//! the same however long the animation is. The controller is created once in
//! the page's state, so rebuilding the page never restarts the animation.

use aimer::input::button::Button;
use aimer::style::{
    BorderSlice, BorderStyle, BoxBorder, BoxDecoration, BoxFit, FontWeight, LayoutSpacing,
    TextAlign, TextStyle, Theme, ThemeData,
};
use aimer::{
    AnyWidget, AssetImage, BuildContext, Column, Container, Dimension, GifController, GifLoops,
    Row, SizedBox, State, StateView, StateUpdater, StatefulWidget, Text, Widget,
};

use crate::{Assets, theme};

/// The frame a "Jump" press seeks to. The bundled GIF has more frames than
/// this; a shorter GIF would simply land on its last frame.
const JUMP_FRAME: usize = 10;

/// Builds the GIF showcase page.
pub fn gif_example() -> impl Widget {
    GifExample::new()
}

#[derive(aimer::StatefulWidget)]
pub struct GifExample;

impl GifExample {
    pub fn new() -> Self {
        Self
    }
}

pub struct GifExampleState {
    controller: GifController,
}

impl State<GifExample> for GifExampleState {
    fn create(widget: GifExample) -> Self {
        GifExampleState {
            controller: GifController::new(),
        }
    }

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let updater = this.updater();
        let playing = this.controller.is_playing();

        let toggle = {
            let controller = this.controller.clone();
            control_button(
                if playing { "Pause" } else { "Play" },
                app_theme,
                true,
                move || {
                    controller.toggle();
                    // Rebuild so the label follows the controller.
                    updater.set_state(|_| {});
                },
            )
        };
        let restart = {
            let controller = this.controller.clone();
            control_button("Restart", app_theme, false, move || {
                controller.restart();
                updater.set_state(|_| {});
            })
        };
        let jump = {
            let controller = this.controller.clone();
            control_button("Jump to frame 10", app_theme, false, move || {
                controller.seek_to_frame(JUMP_FRAME);
            })
        };

        Container::new()
            .padding(LayoutSpacing::all(28))
            .child(Column::new().children(vec![
                Text::new("Animated GIF")
                    .text_style(
                        TextStyle::new()
                            .font_size(26)
                            .font_weight(FontWeight::Bold)
                            .color(app_theme.on_surface_color),
                    )
                    .boxed(),
                SizedBox::new().height(8).boxed(),
                Text::new(
                    "A bundled GIF plays on its own timing and loops forever. Frames are \
                     decoded lazily, a few at a time, and a GifController plays, pauses and \
                     seeks it.",
                )
                .text_style(
                    TextStyle::new()
                        .font_size(15)
                        .color(theme::muted_text(&app_theme)),
                )
                .boxed(),
                SizedBox::new().height(24).boxed(),
                Container::new()
                    .width(Dimension::Px(480.0))
                    .child(
                        AssetImage::new(Assets::GIPHY)
                            .fit(BoxFit::FitWidth)
                            .gif_loops(GifLoops::Infinite)
                            .gif_controller(this.controller.clone()),
                    )
                    .boxed(),
                SizedBox::new().height(20).boxed(),
                Row::new()
                    .children(vec![
                        toggle,
                        SizedBox::new().width(12).boxed(),
                        restart,
                        SizedBox::new().width(12).boxed(),
                        jump,
                    ])
                    .boxed(),
            ]))
    }
}

fn control_button(
    label: &'static str,
    app_theme: ThemeData,
    primary: bool,
    on_press: impl Fn() + 'static,
) -> AnyWidget {
    let (background, foreground) = if primary {
        (app_theme.primary_color, app_theme.on_primary_color)
    } else {
        (aimer::Color::Transparent, app_theme.primary_color)
    };
    Button::new()
        .on_press(on_press)
        .decoration(
            BoxDecoration::new()
                .background_color(background)
                .border(if primary {
                    BoxBorder::default()
                } else {
                    BoxBorder::all(
                        BorderSlice::new()
                            .style(BorderStyle::Solid)
                            .stroke(1.0)
                            .color(app_theme.primary_color),
                    )
                })
                .border_radius(8),
        )
        .child(
            Container::new()
                .height(Dimension::Px(32.0))
                .padding(LayoutSpacing::new().left(14).right(14))
                .child(
                    Text::new(label)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(13)
                                .font_weight(FontWeight::Bold)
                                .color(foreground),
                        ),
                ),
        )
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_builds_as_a_widget_without_starting_the_application() {
        fn assert_widget(_widget: impl Widget) {}
        assert_widget(gif_example());
    }
}
