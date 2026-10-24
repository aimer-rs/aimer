use std::time::Duration;

use aimer::input::button::Button;
use aimer::modal::*;
use aimer::style::*;
use aimer::*;

/// Builds a framework-level animated modal showcase.
pub fn modal_example() -> impl Widget {
    let app_theme = crate::theme::app_theme();
    let page = Column::new()
        .horizontal_alignment(BoxAlignment::Center)
        .vertical_alignment(BoxAlignment::Center)
        .children([
            Text::new("Content behind the modal")
                .text_style(
                    TextStyle::new()
                        .font_size(24)
                        .color(app_theme.on_background_color),
                )
                .boxed(),
            SizedBox::new()
                .height(20)
                .boxed(),
            Container::new()
                .width(180)
                .height(48)
                .box_child(
                    Button::new()
                        .on_press(show_modal)
                        .decoration(
                            BoxDecoration::new()
                                .background_color(app_theme.primary_color)
                                .border_radius(10),
                        )
                        .child(
                            Text::new("Open modal")
                                .text_align(TextAlign::MidCenter)
                                .text_style(
                                    TextStyle::new()
                                        .font_size(16)
                                        .color(app_theme.on_primary_color),
                                ),
                        ),
                ),
        ]);

    Container::new()
        .color(app_theme.background_color)
        .child(
            Align::new()
                .alignment(Alignment::MidCenter)
                .child(page),
        )
}

fn show_modal() {
    let app_theme = crate::theme::app_theme();
    let dialog = Container::new()
        .width(420)
        .height(220)
        .padding(LayoutSpacing::all(Spacing::Px(28)))
        .box_decoration(
            BoxDecoration::new()
                .background_color(app_theme.surface_color)
                .border_radius(18),
        )
        .child(
            Text::new("Modal\n\nThis content is centered above a full-window barrier.")
                .text_align(TextAlign::MidCenter)
                .text_style(
                    TextStyle::new()
                        .font_size(20)
                        .color(app_theme.on_surface_color),
                ),
        );

    Modal::new()
        .barrier_color(Color::BLACK.with_opacity(60))
        .animation(
            ModalAnimation::new()
                .enter_duration(Duration::from_millis(240))
                .exit_duration(Duration::from_millis(160)),
        )
        .child(dialog)
        .show();
}

pub fn start_modal_example() {
    show_modal();
    AimerApp::start(crate::theme::provide(modal_example()));
}
