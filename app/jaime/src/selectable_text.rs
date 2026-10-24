use aimer::style::{LayoutSpacing, Spacing, TextDecoration, TextOverflow, TextStyle};
use aimer::{
    AimerApp, Container, Flex, FlexDirection, FontStyle, FontWeight, RichText, SelectionArea,
    SizedBox, SpanStyle, Text, TextSpan, Widget,
};

use crate::theme;

pub fn selectable_text_example() -> impl Widget {
    let app_theme = theme::app_theme();

    Container::new()
        .color(app_theme.background_color)
        .padding(LayoutSpacing::all(Spacing::Px(32)))
        .child(
            Flex::new()
                .direction(FlexDirection::Column)
                .children([
                    Text::new("Selectable text region")
                        .text_style(TextStyle::new().font_size(30).color(app_theme.on_background_color))
                        .boxed(),
                    SizedBox::new().height(16).boxed(),
                    Text::new(
                        "Drag from the heading through the paragraph and into the caption: one gesture selects across every widget. Press Cmd/Ctrl+A to select the whole region and Cmd/Ctrl+C to copy it. Right-click a word to open the desktop menu where you clicked; hold a finger on one instead and the same verbs arrive as a pill floating above the selection, with a blue handle at each end that you can drag to adjust it.",
                    )
                    .text_style(
                        TextStyle::new()
                        .font_size(16)
                        .color(theme::muted_text(&app_theme))
                        .text_overflow(TextOverflow::Wrap)
                    )
                    .boxed(),
                    SizedBox::new().height(24).boxed(),
                    Container::new()
                        // .width(Percent(50.0))
                        .padding(LayoutSpacing::all(Spacing::Px(20)))
                        .color(theme::raised_surface(&app_theme))
                        .child(SelectionArea::new().selection_color(app_theme.primary_color.with_alpha(0.38)).child(
                            Flex::new().direction(FlexDirection::Column).children([
                                Text::new("A heading that joins the selection")
                                    .text_style(TextStyle::new().font_size(24).color(app_theme.on_surface_color))
                                    .boxed(),
                                SizedBox::new().height(12).boxed(),
                                RichText::new(TextSpan::root([
                                    TextSpan::new("Selection works across "),
                                    TextSpan::new("bold text")
                                        .style(SpanStyle::new().font_weight(FontWeight::Bolder)),
                                    TextSpan::new(", "),
                                    TextSpan::new("italic text").style(
                                        SpanStyle::new()
                                            .font_style(FontStyle::Italic)
                                            .text_decoration(TextDecoration::Underline),
                                    ),
                                    TextSpan::new(", colors, wrapped lines, and Unicode: "),
                                    TextSpan::new("Aimer • 你好 • សួស្តី • 👩‍💻")
                                        .style(SpanStyle::new().color(app_theme.primary_color)),
                                    TextSpan::new(
                                        "\n\nThe copied value is plain text, without style metadata.",
                                    ),
                                ]))
                                .text_overflow(TextOverflow::Wrap)
                                .text_style(TextStyle::new().font_size(20).color(app_theme.on_surface_color))
                                .boxed(),
                                SizedBox::new().height(12).boxed(),
                                Text::new("A caption, selectable because it sits inside the region.")
                                    .text_style(TextStyle::new().font_size(14).color(theme::muted_text(&app_theme)))
                                    .wrapped()
                                    .boxed(),
                            ]),
                        ))
                        .boxed(),
                ]),
        )
}

pub fn start_selectable_text_example() {
    AimerApp::start(crate::theme::provide(selectable_text_example()));
}

#[cfg(test)]
mod tests {
    use aimer::Widget;

    use super::selectable_text_example;

    #[test]
    fn selectable_text_example_builds_a_demo_screen() {
        fn assert_widget(_: impl Widget) {}

        assert_widget(selectable_text_example());
    }
}
