use std::rc::Rc;

use aimer::input::button::Button;
use aimer::style::{
    BoxDecoration, FontWeight, LayoutSpacing, Spacing, TextAlign, TextStyle, Theme, ThemeData,
};
use aimer::*;

const JAIME_MARKDOWN: &str = include_str!("../assets/JAIME.md");
const CUSTOM_MARKDOWN: &str = r#"# Custom Markdown Widgets

Custom inline syntax can render a widget inline: {{button:Press me}}.

:::callout
This block is written in Markdown, but rendered as a custom Aimer widget.
:::
"#;

const TYPED_CUSTOM_MARKDOWN: &str = r#"# Strongly Typed Markdown Widgets

Typed inline syntax decodes into a model: @{alice}.

:::typed-callout
This block is parsed into typed properties and keeps its nested Markdown document.
:::
"#;

struct TypedCallout;

struct TypedCalloutProps {
    message: String,
    content: MarkdownDocument,
}

impl MarkdownCustomBlock for TypedCallout {
    const NAME: &'static str = "typed-callout";
    const OPENING: &'static str = ":::typed-callout";

    type Props = TypedCalloutProps;

    fn parse(input: MarkdownCustomBlockInput<'_>) -> Result<Self::Props, MarkdownError> {
        let message = input
            .raw
            .trim()
            .to_owned();
        if message.is_empty() {
            return Err(MarkdownError::new("typed callout cannot be empty"));
        }
        Ok(TypedCalloutProps {
            message,
            content: input
                .content
                .clone(),
        })
    }

    fn build(props: &Self::Props, ctx: &aimer::BuildContext) -> AnyWidget {
        let app_theme = ThemeData::copied(ctx);

        Container::new()
            .padding(LayoutSpacing::all(Spacing::Px(16)))
            .color(app_theme.surface_color)
            .child(
                Column::new().children([
                    Text::new("Typed block")
                        .text_style(TextStyle::new().font_weight(FontWeight::Bold))
                        .boxed(),
                    Text::new(
                        props
                            .message
                            .clone(),
                    )
                    .boxed(),
                    Text::new(format!(
                        "Nested blocks parsed: {}",
                        props
                            .content
                            .blocks
                            .len()
                    ))
                    .boxed(),
                ]),
            )
            .boxed()
    }
}

struct TypedMention;

impl MarkdownCustomInline for TypedMention {
    const NAME: &'static str = "mention";
    const OPENING: &'static str = "@{";
    const CLOSING: &'static str = "}";

    type Props = String;

    fn parse(raw: &str) -> Result<Self::Props, MarkdownError> {
        let name = raw.trim();
        if name.is_empty() {
            return Err(MarkdownError::new("mention cannot be empty"));
        }
        Ok(name.to_owned())
    }

    fn build(props: &Self::Props, _ctx: &aimer::BuildContext) -> AnyWidget {
        Text::new(format!("@{props}")).boxed()
    }
}

pub fn jaime_markdown_source() -> &'static str {
    JAIME_MARKDOWN
}

pub fn jaime_markdown_content() -> MarkdownViewer {
    MarkdownViewer::new()
        .padding(LayoutSpacing::all(Spacing::Px(16)))
        .theme(crate::theme::markdown_theme())
        .markdown(jaime_markdown_source())
}

pub fn jaime_markdown_viewer() -> impl Widget {
    let app_theme = crate::theme::app_theme();

    Container::new()
        .color(app_theme.background_color)
        .child(jaime_markdown_content())
}

fn custom_markdown_content_with_press(on_press: impl Fn() + 'static) -> MarkdownViewer {
    let on_press = Rc::new(on_press);

    MarkdownViewer::new()
        .padding(LayoutSpacing::all(Spacing::Px(16)))
        .theme(crate::theme::markdown_theme())
        .markdown(CUSTOM_MARKDOWN)
        .custom_block(
            MarkdownBlockRule::new(
                "callout",
                MarkdownBlockSyntax::Paired {
                    opening: ":::callout",
                    closing: ":::",
                },
            ),
            |data| {
                Container::new()
                    .padding(LayoutSpacing::all(Spacing::Px(16)))
                    .color(crate::theme::recessed_surface(&crate::theme::app_theme()))
                    .child(
                        Column::new().children([
                            Text::new("Custom block")
                                .text_style(
                                    TextStyle::new()
                                        .font_weight(FontWeight::Bold)
                                        .color(crate::theme::app_theme().primary_color),
                                )
                                .boxed(),
                            Text::new(
                                data.text
                                    .trim()
                                    .to_owned(),
                            )
                            .text_style(
                                TextStyle::new().color(crate::theme::app_theme().on_surface_color),
                            )
                            .boxed(),
                        ]),
                    )
                    .boxed()
            },
        )
        .custom_inline(
            MarkdownInlineRule::new(
                "button",
                MarkdownInlineSyntax::Paired {
                    opening: "{{button:",
                    closing: "}}",
                },
            ),
            move |data| {
                let on_press = on_press.clone();
                Button::new()
                    .decoration(
                        BoxDecoration::new()
                            .background_color(crate::theme::app_theme().primary_color)
                            .border_radius(6),
                    )
                    .hover_decoration(
                        BoxDecoration::new()
                            .background_color(
                                crate::theme::app_theme()
                                    .primary_color
                                    .darken(0.08),
                            )
                            .border_radius(6),
                    )
                    .press_decoration(
                        BoxDecoration::new()
                            .background_color(
                                crate::theme::app_theme()
                                    .primary_color
                                    .darken(0.18),
                            )
                            .border_radius(6),
                    )
                    .on_press(move || on_press())
                    .child(
                        Container::new()
                            .width(120)
                            .height(20)
                            .child(
                                Text::new(data.label.clone())
                                    .text_align(TextAlign::MidCenter)
                                    .text_style(
                                        TextStyle::new()
                                            .font_weight(FontWeight::Bold)
                                            .color(crate::theme::app_theme().on_primary_color),
                                    ),
                            ),
                    )
                    .boxed()
            },
        )
}

pub fn custom_markdown_content() -> MarkdownViewer {
    custom_markdown_content_with_press(|| {})
}

pub fn typed_custom_markdown_content() -> MarkdownViewer {
    MarkdownViewer::new()
        .padding(LayoutSpacing::all(Spacing::Px(16)))
        .theme(crate::theme::markdown_theme())
        .markdown(TYPED_CUSTOM_MARKDOWN)
        .typed_block::<TypedCallout>()
        .typed_inline::<TypedMention>()
}

#[derive(StatefulWidget)]
pub struct CustomMarkdownExample {}

impl CustomMarkdownExample {
    pub fn new() -> Self {
        Self {}
    }
}

pub struct CustomMarkdownExampleState {
    presses: u32,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for CustomMarkdownExample {
    type State = CustomMarkdownExampleState;

    fn create_state(self) -> Self::State {
        CustomMarkdownExampleState {
            presses: 0,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<CustomMarkdownExample> for CustomMarkdownExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, ctx: &aimer::BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let updater = self.updater;
        Column::new().children([
            Text::new(format!("Button presses: {}", self.presses))
                .text_style(TextStyle::new().color(app_theme.on_background_color))
                .boxed(),
            custom_markdown_content_with_press(move || {
                println!("Button pressed");
                updater.set_state(|state| state.presses += 1);
            })
            .boxed(),
        ])
    }
}

pub fn custom_markdown_viewer() -> impl Widget {
    let app_theme = crate::theme::app_theme();

    Container::new()
        .color(app_theme.background_color)
        .child(Row::new().children([
            Expanded::new().box_child(typed_custom_markdown_content()),
            Expanded::new().box_child(CustomMarkdownExample::new()),
        ]))
}

pub fn start_custom_markdown_example() {
    AimerApp::start(crate::theme::provide(custom_markdown_viewer()));
}

pub fn start_markdown_example() {
    AimerApp::start(crate::theme::provide(jaime_markdown_viewer()));
}

#[cfg(test)]
mod tests {
    use aimer::{MarkdownDocument, Widget};

    use super::{
        CUSTOM_MARKDOWN, TYPED_CUSTOM_MARKDOWN, custom_markdown_content, jaime_markdown_content,
        jaime_markdown_source, typed_custom_markdown_content,
    };

    #[test]
    fn bundled_jaime_markdown_is_loaded_and_parseable() {
        let source = jaime_markdown_source();

        assert!(source.starts_with("# AimerMarkdown"));
        assert!(MarkdownDocument::parse(source).is_ok());
        assert_eq!(jaime_markdown_content().text_content(), Some(source));
    }

    #[test]
    fn custom_markdown_source_contains_registered_syntax() {
        assert!(CUSTOM_MARKDOWN.contains("{{button:Press me}}"));
        assert!(CUSTOM_MARKDOWN.contains(":::callout"));
        assert!(
            custom_markdown_content()
                .text_content()
                .is_some()
        );
    }

    #[test]
    fn typed_custom_markdown_registers_strongly_typed_rules() {
        assert!(TYPED_CUSTOM_MARKDOWN.contains("@{alice}"));
        assert!(TYPED_CUSTOM_MARKDOWN.contains(":::typed-callout"));
        assert!(
            typed_custom_markdown_content()
                .text_content()
                .is_some()
        );
    }
}
