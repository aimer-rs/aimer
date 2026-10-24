use std::rc::Rc;

use aimer_assets::{AssetImage, NetworkImage};
use aimer_color::prelude::Color;
use aimer_container::{Container, SizedBox};
use aimer_flex::{BoxAlignment, Column, Expanded, OverflowBehavior, Row};
use aimer_grid::{Grid, GridItem, GridTrack};
use aimer_input::button::Button;
use aimer_rubick::ShareRef;
use aimer_scroll::{ScrollAxis, Scrollable};
use aimer_std::case;
use aimer_style::{
    BorderSlice, BorderStyle, BoxBorder, BoxDecoration, FontStyle, FontWeight, LayoutSpacing,
    TextAlign, TextDecoration, TextDecorationLine, TextOverflow, TextStyle,
};
use aimer_svg::{Svg, SvgDocument, SvgStyle};
use aimer_text::{RichText, SelectionArea, SpanStyle, Text, TextSpan};
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyWidget, Widget};

use crate::custom::{TypedCustomBlockBuilder, TypedCustomInlineBuilder};
use crate::document::share_ref_range;
pub(crate) use crate::markdown_theme::MarkdownTheme;
use crate::syntax::highlight_cached;
use crate::{
    Alignment, Block, CaptureSpan, CustomBlockBuilder, CustomInlineBuilder, Document, Inline,
    TableRow,
};

const TICK_SVG_DATA: &[u8] = include_bytes!("../tick-checkbox-svgrepo-com.svg");
const COPY_SVG_DATA: &[u8] = include_bytes!("../copy-2-svgrepo-com.svg");

#[derive(Clone, Debug, PartialEq, Eq)]
/// Metadata passed to a [`ImageResolver`] for each image node.
pub struct MarkdownImage {
    pub source: ShareRef<str>,
    pub alt: ShareRef<str>,
    pub title: Option<ShareRef<str>>,
}

/// Shared callback invoked when a rendered link is activated.
pub type LinkHandler = Rc<dyn Fn(Rc<str>)>;
/// Shared resolver that maps image metadata to a native Aimer widget.
pub type ImageResolver = Rc<dyn Fn(&MarkdownImage) -> AnyWidget>;

fn build_tick_box(ticked: bool, theme: &MarkdownTheme) -> AnyWidget {
    match SvgDocument::from_svg(TICK_SVG_DATA) {
        Ok(doc) => Container::new()
            .margin(LayoutSpacing::new().top(2))
            .width(16)
            .height(18)
            .child(
                Svg::new(doc)
                    .style("#box", SvgStyle::new().fill(theme.icon_color))
                    .style(
                        "#tick",
                        SvgStyle::new().fill(case!(ticked, theme.icon_color, Color::Transparent)),
                    ),
            )
            .boxed(),
        Err(_) => Text::new(if ticked { "[✔]" } else { "[ ]" })
            .text_style(TextStyle::new().color(theme.icon_color))
            .boxed(),
    }
}

fn code_block_language_label(language: Option<&ShareRef<str>>) -> Option<&ShareRef<str>> {
    language.filter(|language| !language.trim().is_empty())
}

fn copy_code_with<E>(source: &str, copy: impl FnOnce(&str) -> Result<(), E>) -> Result<(), E> {
    copy(source)
}

fn build_code_header(
    value: &ShareRef<str>,
    language: Option<&ShareRef<str>>,
    theme: &MarkdownTheme,
) -> AnyWidget {
    let language = match code_block_language_label(language) {
        Some(language) => Text::new(language.clone()),
        None => Text::new("text"),
    }
    .text_style(
        theme
            .code_block
            .font_family(aimer_assets::FontFamily::SANS_SERIF)
            .color(theme.code_block.color.with_opacity(160)),
    )
    .boxed();
    let decoration = BoxDecoration::new().border_radius(6);
    let source = value.clone();
    let copy = match SvgDocument::from_svg(COPY_SVG_DATA) {
        Ok(document) => Container::new()
            .width(28)
            .height(28)
            .child(
                Button::new()
                    .decoration(decoration.clone())
                    .hover_decoration(decoration.background_color(theme.icon_color.with_alpha(0.3)))
                    .on_press(move || {
                        if let Err(error) =
                            copy_code_with(&source, aimer_native::clipboard::set_text)
                        {
                            eprintln!("Failed to copy Markdown code block: {error}");
                        }
                    })
                    .child(
                        Row::new()
                            .vertical_alignment(BoxAlignment::Center)
                            .horizontal_alignment(BoxAlignment::Center)
                            .children([Svg::new(document).width(20.0).height(20.0).style(
                                "path",
                                SvgStyle::new().stroke(theme.code_background.invert()),
                            )]),
                    ),
            )
            .boxed(),
        Err(_) => SizedBox::new().width(28).height(28).boxed(),
    };

    Container::new()
        .padding(LayoutSpacing::new().left(16).right(12).top(8).bottom(8))
        .child(
            Row::new()
                .vertical_alignment(BoxAlignment::Center)
                .children(vec![Expanded::new().box_child(language), copy]),
        )
        .boxed()
}

/// Resolves network URLs with `NetworkImage` and other sources with
/// `AssetImage`.
pub fn default_image_resolver(image: &MarkdownImage) -> AnyWidget {
    if image.source.starts_with("http://") || image.source.starts_with("https://") {
        // debug!("Loading network image {}", image.source);

        Container::new()
            // .height(400)
            .child(Row::new().children([NetworkImage::new(image.source.to_string()).height(400)]))
            .boxed()
    } else {
        // debug!("Loading asset image {}", image.source);

        Container::new()
            // .height(400)
            .child(Row::new().children([AssetImage::new(image.source.to_string()).height(400)]))
            .boxed()
    }
}

pub(crate) fn inline_spans<'a>(
    inlines: impl IntoIterator<Item = &'a Inline>,
    theme: &MarkdownTheme,
) -> TextSpan {
    TextSpan::root(inlines.into_iter().map(|inline| inline_span(inline, theme)))
}

fn inline_span(inline: &Inline, theme: &MarkdownTheme) -> TextSpan {
    match inline {
        Inline::Text(text) => TextSpan::new_shared(text.clone()),
        Inline::SoftBreak => TextSpan::new(" "),
        Inline::HardBreak => TextSpan::new("\n"),
        Inline::Emphasis(children) => TextSpan::new("")
            .style(SpanStyle::new().font_style(FontStyle::Italic))
            .children(children.iter().map(|inline| inline_span(inline, theme))),
        Inline::Strong(children) => TextSpan::new("")
            .style(SpanStyle::new().font_weight(FontWeight::Bold))
            .children(children.iter().map(|inline| inline_span(inline, theme))),
        Inline::Delete(children) => TextSpan::new("")
            .style(
                SpanStyle::new()
                    .text_decoration(TextDecoration::new().line(TextDecorationLine::LINE_THROUGH)),
            )
            .children(children.iter().map(|inline| inline_span(inline, theme))),
        Inline::Code(code) => TextSpan::new_shared(code.clone()).style(theme.inline_code),
        Inline::Link { url, content, .. } => TextSpan::new("")
            .style(theme.link)
            .children(content.iter().map(|inline| inline_span(inline, theme)))
            .link_shared(url.clone()),
        Inline::Image { alt, .. } => TextSpan::new("")
            .style(SpanStyle::new().font_style(FontStyle::Italic))
            .children([
                TextSpan::new("["),
                TextSpan::new_shared(alt.clone()),
                TextSpan::new("]"),
            ]),
        Inline::FootnoteReference { identifier } => TextSpan::new("")
            .style(theme.link)
            .children([
                TextSpan::new("["),
                TextSpan::new_shared(identifier.clone()),
                TextSpan::new("]"),
            ])
            .link(format!("#footnote-{identifier}")),
        Inline::Custom(data) => TextSpan::new_shared(data.text.clone()),
    }
}

#[cfg(test)]
pub(crate) fn render_document(
    document: &Document,
    theme: &MarkdownTheme,
    link_handler: Option<&LinkHandler>,
    image_resolver: &ImageResolver,
    custom_blocks: &[(crate::BlockRule, CustomBlockBuilder)],
    custom_inlines: &[(crate::InlineRule, CustomInlineBuilder)],
) -> AnyWidget {
    render_document_with_context(
        document,
        theme,
        link_handler,
        image_resolver,
        custom_blocks,
        custom_inlines,
        &[],
        &[],
        None,
    )
}

pub(crate) fn render_document_with_context(
    document: &Document,
    theme: &MarkdownTheme,
    link_handler: Option<&LinkHandler>,
    image_resolver: &ImageResolver,
    custom_blocks: &[(crate::BlockRule, CustomBlockBuilder)],
    custom_inlines: &[(crate::InlineRule, CustomInlineBuilder)],
    typed_custom_blocks: &[(crate::BlockRule, TypedCustomBlockBuilder)],
    typed_custom_inlines: &[(crate::InlineRule, TypedCustomInlineBuilder)],
    ctx: Option<&BuildContext>,
) -> AnyWidget {
    render_blocks(
        &document.blocks,
        theme,
        link_handler,
        image_resolver,
        custom_blocks,
        custom_inlines,
        typed_custom_blocks,
        typed_custom_inlines,
        ctx,
    )
}

fn render_blocks(
    blocks: &[Block],
    theme: &MarkdownTheme,
    link_handler: Option<&LinkHandler>,
    image_resolver: &ImageResolver,
    custom_blocks: &[(crate::BlockRule, CustomBlockBuilder)],
    custom_inlines: &[(crate::InlineRule, CustomInlineBuilder)],
    typed_custom_blocks: &[(crate::BlockRule, TypedCustomBlockBuilder)],
    typed_custom_inlines: &[(crate::InlineRule, TypedCustomInlineBuilder)],
    ctx: Option<&BuildContext>,
) -> AnyWidget {
    let children = blocks
        .iter()
        .map(|block| {
            render_block(
                block,
                theme,
                link_handler,
                image_resolver,
                custom_blocks,
                custom_inlines,
                typed_custom_blocks,
                typed_custom_inlines,
                ctx,
            )
        })
        .collect::<Vec<_>>();
    Column::new()
        .horizontal_alignment(BoxAlignment::Start)
        .gaps(LayoutSpacing::all(theme.block_spacing))
        .children(children)
        .boxed()
}

fn table_text_align(alignment: Alignment) -> TextAlign {
    match alignment {
        Alignment::Left | Alignment::None => TextAlign::TopLeft,
        Alignment::Center => TextAlign::TopCenter,
        Alignment::Right => TextAlign::TopRight,
    }
}

fn table_tracks(column_count: usize) -> Vec<GridTrack> {
    std::iter::repeat_n(GridTrack::Fr(1.0), column_count).collect()
}

fn render_table(
    alignments: &[Alignment],
    rows: &[TableRow],
    theme: &MarkdownTheme,
    link_handler: Option<&LinkHandler>,
    image_resolver: &ImageResolver,
    custom_inlines: &[(crate::InlineRule, CustomInlineBuilder)],
    typed_custom_inlines: &[(crate::InlineRule, TypedCustomInlineBuilder)],
    ctx: Option<&BuildContext>,
) -> AnyWidget {
    let column_count = rows
        .iter()
        .map(|row| row.cells.len())
        .chain(std::iter::once(alignments.len()))
        .max()
        .unwrap_or(1)
        .max(1);
    let cells = rows
        .iter()
        .enumerate()
        .flat_map(|(row_index, row)| {
            (0..column_count).map(move |column_index| {
                let content = row
                    .cells
                    .get(column_index)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let alignment = alignments
                    .get(column_index)
                    .copied()
                    .unwrap_or(Alignment::None);
                let content = render_inline_flow(
                    content,
                    theme.body,
                    Some(table_text_align(alignment)),
                    theme,
                    link_handler,
                    image_resolver,
                    custom_inlines,
                    typed_custom_inlines,
                    ctx,
                );
                let background = if row_index == 0 {
                    theme.table_header_background
                } else {
                    theme.table_cell_background
                };
                GridItem::new(
                    Container::new()
                        .padding(LayoutSpacing::all(8_u32))
                        .color(background)
                        .child(content)
                        .boxed(),
                )
                .at(row_index, column_index)
            })
        })
        .collect::<Vec<_>>();
    Container::new()
        .padding(LayoutSpacing::all(1_u32))
        .color(theme.rule_color)
        .child(
            Grid::new()
                .columns(table_tracks(column_count))
                .gap(1.0)
                .children(cells),
        )
        .boxed()
}

fn render_block(
    block: &Block,
    theme: &MarkdownTheme,
    link_handler: Option<&LinkHandler>,
    image_resolver: &ImageResolver,
    custom_blocks: &[(crate::BlockRule, CustomBlockBuilder)],
    custom_inlines: &[(crate::InlineRule, CustomInlineBuilder)],
    typed_custom_blocks: &[(crate::BlockRule, TypedCustomBlockBuilder)],
    typed_custom_inlines: &[(crate::InlineRule, TypedCustomInlineBuilder)],
    ctx: Option<&BuildContext>,
) -> AnyWidget {
    match block {
        Block::Heading { depth, content } => render_inline_flow(
            content,
            theme.headings[usize::from(*depth).saturating_sub(1).min(5)],
            None,
            theme,
            link_handler,
            image_resolver,
            custom_inlines,
            typed_custom_inlines,
            ctx,
        ),
        Block::Paragraph(content) => render_inline_flow(
            content,
            theme.body,
            None,
            theme,
            link_handler,
            image_resolver,
            custom_inlines,
            typed_custom_inlines,
            ctx,
        ),
        Block::Blockquote(blocks) => Container::new()
            .padding(LayoutSpacing::all(8_u32))
            .box_decoration(
                BoxDecoration::new()
                    .background_color(theme.quote_background)
                    .border(
                        BoxBorder::new().left(
                            BorderSlice::new()
                                .stroke(4)
                                .color(theme.rule_color)
                                .style(BorderStyle::Solid),
                        ),
                    ),
            )
            .box_child(render_blocks_with_style(
                blocks,
                theme.blockquote,
                theme,
                link_handler,
                image_resolver,
                custom_blocks,
                custom_inlines,
                typed_custom_blocks,
                typed_custom_inlines,
                ctx,
            )),
        Block::List {
            ordered,
            start,
            items,
        } => {
            let start = start.unwrap_or(1);
            let rows = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let mut is_complete = false;
                    let marker = match item.checked {
                        Some(true) => {
                            is_complete = true;
                            None
                        }
                        Some(false) => None,
                        None if *ordered => Some(format!("{}.", start + index as u32)),
                        None => Some("•".to_string()),
                    };

                    Row::new()
                        .gaps(LayoutSpacing::all(8_u32))
                        .vertical_alignment(BoxAlignment::Start)
                        .children(vec![
                            match marker {
                                Some(marker) => Text::new(marker)
                                    .text_style(theme.body.font_weight(FontWeight::Bold))
                                    .boxed(),
                                None => build_tick_box(is_complete, theme),
                            },
                            Expanded::new().box_child(render_blocks(
                                &item.blocks,
                                theme,
                                link_handler,
                                image_resolver,
                                custom_blocks,
                                custom_inlines,
                                typed_custom_blocks,
                                typed_custom_inlines,
                                ctx,
                            )),
                        ])
                        .boxed()
                })
                .collect::<Vec<_>>();
            Column::new()
                .horizontal_alignment(BoxAlignment::Start)
                .gaps(LayoutSpacing::all(6_u32))
                .children(rows)
                .boxed()
        }
        Block::Code {
            value, language, ..
        } => {
            let spans = highlighted_code_spans(value, language.as_deref());
            Container::new()
                .box_decoration(
                    BoxDecoration::new()
                        .background_color(theme.code_background)
                        .border_radius(8),
                )
                .box_child(
                    Column::new()
                        .horizontal_alignment(BoxAlignment::Start)
                        .children([
                            build_code_header(value, language.as_ref(), theme),
                            Container::new()
                                .padding(LayoutSpacing::new().left(16).right(16).bottom(16))
                                .box_child(
                                    Scrollable::new()
                                        .axis(ScrollAxis::Horizontal)
                                        .vertical_scroll_bar(None)
                                        .horizontal_scroll_bar(None)
                                        .child(
                                            SizedBox::new().child(
                                                SelectionArea::new().child(
                                                    RichText::new(TextSpan::root(spans))
                                                        .text_style(theme.code_block),
                                                ),
                                            ),
                                        ),
                                ),
                        ]),
                )
        }
        Block::ThematicBreak => SizedBox::new().height(1.0).color(theme.rule_color).boxed(),
        Block::Table { alignments, rows } => render_table(
            alignments,
            rows,
            theme,
            link_handler,
            image_resolver,
            custom_inlines,
            typed_custom_inlines,
            ctx,
        ),
        Block::FootnoteDefinition { identifier, blocks } => Row::new()
            .gaps(LayoutSpacing::all(6_u32))
            .children(vec![
                RichText::new(
                    TextSpan::new("")
                        .style(SpanStyle::new().font_weight(FontWeight::Bold))
                        .children([
                            TextSpan::new("["),
                            TextSpan::new_shared(identifier.clone()),
                            TextSpan::new("]"),
                        ]),
                )
                .text_style(theme.body)
                .boxed(),
                render_blocks(
                    blocks,
                    theme,
                    link_handler,
                    image_resolver,
                    custom_blocks,
                    custom_inlines,
                    typed_custom_blocks,
                    typed_custom_inlines,
                    ctx,
                ),
            ])
            .boxed(),
        Block::Custom(data) => typed_custom_blocks
            .iter()
            .find(|(rule, _)| rule.name() == data.name.as_ref())
            .and_then(|(_, builder)| ctx.map(|ctx| builder(data, ctx)))
            .or_else(|| {
                custom_blocks
                    .iter()
                    .find(|(rule, _)| rule.name() == data.name.as_ref())
                    .map(|(_, builder)| builder(data))
            })
            .unwrap_or_else(|| Text::new(data.text.clone()).text_style(theme.body).boxed()),
    }
}

fn highlighted_code_spans(value: &ShareRef<str>, language: Option<&str>) -> Vec<TextSpan> {
    let mut offset = 0;
    let mut spans = Vec::new();
    for capture in highlight_cached(value.as_ref(), language).iter() {
        let (start, end) = capture.range();
        let (start, end) = (start as usize, end as usize);
        if start < offset
            || end <= start
            || end > value.len()
            || !value.is_char_boundary(start)
            || !value.is_char_boundary(end)
        {
            continue;
        }
        if offset < start {
            spans.push(TextSpan::new_shared(share_ref_range(value, offset, start)));
        }
        let mut style = SpanStyle::new().color(capture.color());
        if matches!(capture, CaptureSpan::Keyword { .. }) {
            style = style.font_weight(FontWeight::Bold);
        } else if matches!(capture, CaptureSpan::Comment { .. }) {
            style = style.font_style(FontStyle::Italic);
        }
        spans.push(TextSpan::new_shared(share_ref_range(value, start, end)).style(style));
        offset = end;
    }
    if offset < value.len() {
        spans.push(TextSpan::new_shared(share_ref_range(
            value,
            offset,
            value.len(),
        )));
    }
    spans
}

fn render_blocks_with_style(
    blocks: &[Block],
    style: TextStyle,
    theme: &MarkdownTheme,
    link_handler: Option<&LinkHandler>,
    image_resolver: &ImageResolver,
    custom_blocks: &[(crate::BlockRule, CustomBlockBuilder)],
    custom_inlines: &[(crate::InlineRule, CustomInlineBuilder)],
    typed_custom_blocks: &[(crate::BlockRule, TypedCustomBlockBuilder)],
    typed_custom_inlines: &[(crate::InlineRule, TypedCustomInlineBuilder)],
    ctx: Option<&BuildContext>,
) -> AnyWidget {
    let children = blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph(inlines) => render_inline_flow(
                inlines,
                style,
                None,
                theme,
                link_handler,
                image_resolver,
                custom_inlines,
                typed_custom_inlines,
                ctx,
            ),
            _ => render_block(
                block,
                theme,
                link_handler,
                image_resolver,
                custom_blocks,
                custom_inlines,
                typed_custom_blocks,
                typed_custom_inlines,
                ctx,
            ),
        })
        .collect::<Vec<_>>();
    Column::new()
        .horizontal_alignment(BoxAlignment::Start)
        .gaps(LayoutSpacing::all(8_u32))
        .box_children(children)
}

fn render_inline_flow(
    inlines: &[Inline],
    style: TextStyle,
    alignment: Option<TextAlign>,
    theme: &MarkdownTheme,
    link_handler: Option<&LinkHandler>,
    image_resolver: &ImageResolver,
    custom_inlines: &[(crate::InlineRule, CustomInlineBuilder)],
    typed_custom_inlines: &[(crate::InlineRule, TypedCustomInlineBuilder)],
    ctx: Option<&BuildContext>,
) -> AnyWidget {
    let mut has_plain_content = true;
    let mut has_image = false;
    let mut has_custom_inline = false;
    for inline in inlines {
        match inline {
            Inline::Text(_) | Inline::SoftBreak | Inline::HardBreak => {}
            Inline::Image { .. } => {
                has_image = true;
                has_plain_content = false;
            }
            Inline::Custom(_) => {
                has_custom_inline = true;
                has_plain_content = false;
            }
            _ => has_plain_content = false,
        }
    }
    if has_plain_content {
        let text = match inlines {
            [] => Some(Text::new("")),
            [Inline::Text(text)] => Some(Text::new(text.clone())),
            _ => None,
        };
        if let Some(text) = text {
            // Syntax-free flows can use RawTextWidget, which is safe for retained
            // paint reuse. Rich syntax still follows the span-based path below.
            let text = text.text_style(style.text_overflow(TextOverflow::Wrap));
            return match alignment {
                Some(alignment) => text.text_align(alignment).boxed(),
                None => text.boxed(),
            };
        }
        return rich_text_aligned(inlines, style, alignment, theme, link_handler);
    }
    if !has_image && !has_custom_inline {
        return rich_text_aligned(inlines, style, alignment, theme, link_handler);
    }
    let mut children = Vec::new();
    let mut text: Vec<&Inline> = Vec::new();
    for inline in inlines {
        if let Inline::Image { url, title, alt } = inline {
            if !text.is_empty() {
                children.push(rich_text_for_inline_flow(
                    &text,
                    style,
                    alignment,
                    theme,
                    link_handler,
                    has_custom_inline,
                ));
                text.clear();
            }
            children.push(image_resolver(&MarkdownImage {
                source: url.clone(),
                alt: alt.clone(),
                title: title.clone(),
            }));
        } else if let Inline::Custom(data) = inline {
            if !text.is_empty() {
                children.push(rich_text_for_inline_flow(
                    &text,
                    style,
                    alignment,
                    theme,
                    link_handler,
                    true,
                ));
                text.clear();
            }
            let widget = custom_inlines
                .iter()
                .find(|(rule, _)| rule.name() == data.name.as_ref())
                .map(|(_, builder)| builder(data))
                .or_else(|| {
                    typed_custom_inlines
                        .iter()
                        .find(|(rule, _)| rule.name() == data.name.as_ref())
                        .and_then(|(_, builder)| ctx.map(|ctx| builder(data, ctx)))
                })
                .unwrap_or_else(|| Text::new(data.text.clone()).text_style(style).boxed());
            children.push(widget);
        } else {
            text.push(inline);
        }
    }
    if !text.is_empty() {
        children.push(rich_text_for_inline_flow(
            &text,
            style,
            alignment,
            theme,
            link_handler,
            has_custom_inline,
        ));
    }
    if children.len() == 1 {
        return children.remove(0);
    }
    if has_custom_inline && !has_image {
        Row::new()
            .vertical_alignment(BoxAlignment::Start)
            .overflow(OverflowBehavior::Wrap)
            .children(children)
            .boxed()
    } else {
        Column::new()
            .horizontal_alignment(BoxAlignment::Start)
            .gaps(LayoutSpacing::all(8_u32))
            .children(children)
            .boxed()
    }
}

fn rich_text_for_inline_flow(
    inlines: &[&Inline],
    style: TextStyle,
    alignment: Option<TextAlign>,
    theme: &MarkdownTheme,
    link_handler: Option<&LinkHandler>,
    has_custom_inline: bool,
) -> AnyWidget {
    if has_custom_inline {
        rich_text_aligned_wrappable(
            inlines.iter().copied(),
            style,
            alignment,
            theme,
            link_handler,
        )
    } else {
        rich_text_aligned(
            inlines.iter().copied(),
            style,
            alignment,
            theme,
            link_handler,
        )
    }
}

fn rich_text_aligned_wrappable<'a>(
    inlines: impl IntoIterator<Item = &'a Inline>,
    style: TextStyle,
    alignment: Option<TextAlign>,
    theme: &MarkdownTheme,
    link_handler: Option<&LinkHandler>,
) -> AnyWidget {
    let rich = RichText::new(inline_spans(inlines, theme))
        .text_style(style.text_overflow(TextOverflow::Wrap));
    let rich = match alignment {
        Some(alignment) => rich.text_align(alignment),
        None => rich,
    };
    match link_handler {
        Some(handler) => {
            let handler = (*handler).clone();
            rich.on_link(move |target: Rc<str>| handler(target)).boxed()
        }
        None => rich.boxed(),
    }
}

fn rich_text_aligned<'a>(
    inlines: impl IntoIterator<Item = &'a Inline>,
    style: TextStyle,
    alignment: Option<TextAlign>,
    theme: &MarkdownTheme,
    link_handler: Option<&LinkHandler>,
) -> AnyWidget {
    let rich = RichText::new(inline_spans(inlines, theme))
        .text_style(style)
        .wrapped()
        .link_hover_color(theme.link_hover_color);
    let rich = match alignment {
        Some(alignment) => rich.text_align(alignment),
        None => rich,
    };
    match link_handler {
        Some(handler) => {
            let handler = (*handler).clone();
            rich.on_link(move |target: Rc<str>| handler(target)).boxed()
        }
        None => rich.boxed(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::sync::OnceLock;

    use aimer_attribute::{BoxConstraint, ResolvedSize};
    use aimer_canvas::{FrameCanvas, InnerCanvas};
    use aimer_cupid::draw_cmd::DrawCommand;
    use aimer_style::{TextAlign, TextDecorationLine, TextStyle};
    use aimer_widget::VisitorElement;
    use aimer_widget::base::{BuildContext, WindowHandle};

    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    fn dummy_async_handle() -> tokio::runtime::Handle {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let _guard = runtime.enter();
        tokio::runtime::Handle::current()
    }

    fn layout_context(width: f32, height: f32) -> BuildContext<'static> {
        layout_context_with_canvas(width, height).0
    }

    fn layout_context_with_canvas(width: f32, height: f32) -> (BuildContext<'static>, InnerCanvas) {
        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(Box::leak(Box::new(inner.clone())));
        let size = ResolvedSize { width, height };
        let mut ctx = BuildContext::new(
            canvas,
            size,
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(
                winit::dpi::PhysicalSize::new(width as u32, height as u32),
                1.0,
            ),
            #[cfg(not(target_arch = "wasm32"))]
            dummy_async_handle(),
        );
        ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: width,
            max_height: height,
        };
        (ctx, inner)
    }

    #[test]
    fn single_line_blockquote_uses_bounded_width_and_natural_height() {
        let resolver: ImageResolver = Rc::new(default_image_resolver);
        let quote = Document::parse("> one line").unwrap();
        let quote_widget =
            render_document(&quote, &MarkdownTheme::default(), None, &resolver, &[], &[]);
        let quote_ctx = layout_context(320.0, 200.0);
        let quote_size = quote_widget
            .to_element(&quote_ctx)
            .computed_size(&quote_ctx);

        assert!(
            quote_size.height < 100.0,
            "single-line blockquote height was {}",
            quote_size.height
        );

        let document = Document::parse("> one line\n\nafter").unwrap();
        let widget = render_document(
            &document,
            &MarkdownTheme::default(),
            None,
            &resolver,
            &[],
            &[],
        );
        let ctx = layout_context(320.0, 200.0);

        let size = widget.to_element(&ctx).computed_size(&ctx);

        assert!(size.width.is_finite());
        assert!(size.height.is_finite());
        assert!(
            size.width <= 320.0,
            "blockquote document width was {}",
            size.width
        );
        assert!(
            size.height < 100.0,
            "blockquote document height was {}",
            size.height
        );
    }

    #[test]
    fn nested_blockquote_increases_the_outer_quote_height() {
        let resolver: ImageResolver = Rc::new(default_image_resolver);
        let ctx = layout_context(320.0, 200.0);
        let single_quote = Document::parse("> outer").unwrap();
        let nested_quote = Document::parse("> outer\n>> inner").unwrap();

        let single_height = render_document(
            &single_quote,
            &MarkdownTheme::default(),
            None,
            &resolver,
            &[],
            &[],
        )
        .to_element(&ctx)
        .computed_size(&ctx)
        .height;
        let nested_height = render_document(
            &nested_quote,
            &MarkdownTheme::default(),
            None,
            &resolver,
            &[],
            &[],
        )
        .to_element(&ctx)
        .computed_size(&ctx)
        .height;

        assert!(
            nested_height > single_height + 30.0,
            "nested blockquote height {nested_height} did not reserve space for its inner text; single quote height was {single_height}"
        );
    }

    #[test]
    fn list_content_wraps_within_the_space_remaining_after_its_marker() {
        let resolver: ImageResolver = Rc::new(default_image_resolver);
        let theme = MarkdownTheme::default();
        let text = "Use CodeGraph to understand code safely now";
        let paragraph = Document::parse(text).unwrap();
        let list = Document::parse(&format!("- {text}")).unwrap();
        let height_at = |document: &Document, width: f32| {
            let ctx = layout_context(width, 200.0);
            render_document(document, &theme, None, &resolver, &[], &[])
                .to_element(&ctx)
                .computed_size(&ctx)
                .height
        };

        // The host's font decides how wide the sentence is, so a fixed viewport
        // can already wrap the bare paragraph and leave nothing to compare.
        // Find the narrowest viewport that still fits the paragraph on one
        // line instead: any width the marker takes away from it must wrap.
        let one_line = height_at(&paragraph, 2000.0);
        let (mut narrow, mut fits) = (1.0_f32, 2000.0_f32);
        while fits - narrow > 0.5 {
            let middle = (narrow + fits) / 2.0;
            if height_at(&paragraph, middle) <= one_line {
                fits = middle;
            } else {
                narrow = middle;
            }
        }
        let width = fits.ceil();
        let paragraph_height = height_at(&paragraph, width);
        let list_height = height_at(&list, width);

        assert_eq!(
            paragraph_height, one_line,
            "the paragraph must fit on one line at {width}"
        );
        assert!(
            list_height < 200.0,
            "a list row expanded to the viewport height: {list_height}"
        );
        assert!(
            list_height > paragraph_height,
            "list content did not wrap in its reduced width: list height {list_height}, paragraph height {paragraph_height}"
        );
    }

    #[test]
    fn table_columns_share_the_available_width() {
        assert_eq!(
            table_tracks(2),
            vec![GridTrack::Fr(1.0), GridTrack::Fr(1.0)]
        );
    }

    #[test]
    fn default_link_hover_color_is_brighter_than_the_link_color() {
        let theme = MarkdownTheme::default();
        let link_color = theme.link.color.expect("default links have a color");

        assert_eq!(theme.link_hover_color, link_color.lighten(0.48));
        assert_ne!(theme.link_hover_color, link_color);
    }

    #[test]
    fn table_alignment_maps_to_rich_text_alignment() {
        assert!(matches!(
            table_text_align(Alignment::Left),
            TextAlign::TopLeft
        ));
        assert!(matches!(
            table_text_align(Alignment::Center),
            TextAlign::TopCenter
        ));
        assert!(matches!(
            table_text_align(Alignment::Right),
            TextAlign::TopRight
        ));
        assert!(matches!(
            table_text_align(Alignment::None),
            TextAlign::TopLeft
        ));
    }

    #[test]
    fn highlighted_code_spans_preserve_all_source_text() {
        for (source, language) in [
            ("fn café() { \"你好\" } // done\n", Some("rust")),
            ("anything <goes>\n", Some("unknown")),
            ("unlabelled\n", None),
        ] {
            let source = ShareRef::from(source.to_owned());
            let text = TextSpan::root(highlighted_code_spans(&source, language))
                .flatten(&TextStyle::default())
                .into_iter()
                .map(|span| span.text.to_string())
                .collect::<String>();
            assert_eq!(text, source.as_ref());
        }
    }

    #[test]
    fn code_block_header_preserves_the_fence_language() {
        let rust = ShareRef::from_static("rust");
        assert_eq!(
            code_block_language_label(Some(&rust)).map(|language| language.as_ref()),
            Some("rust")
        );
        assert_eq!(code_block_language_label(None), None);
    }

    #[test]
    fn code_block_copy_forwards_the_exact_source() {
        let copied = RefCell::new(String::new());

        copy_code_with("fn main() {}\n", |source| {
            copied.replace(source.to_owned());
            Ok::<(), ()>(())
        })
        .unwrap();

        assert_eq!(copied.into_inner(), "fn main() {}\n");
    }

    #[test]
    fn code_block_copy_icon_is_valid_svg() {
        assert!(SvgDocument::from_svg(COPY_SVG_DATA).is_ok());
    }

    #[test]
    fn rendered_code_block_exposes_its_language_and_source_text() {
        let resolver: ImageResolver = Rc::new(default_image_resolver);
        let document = Document::parse("```rust\nfn main() {}\n```").unwrap();
        let (ctx, _canvas) = layout_context_with_canvas(320.0, 200.0);
        let element = render_document(
            &document,
            &MarkdownTheme::default(),
            None,
            &resolver,
            &[],
            &[],
        )
        .to_element(&ctx);
        element.layout(&ctx);
        element.update(&ctx);
        let mut commands = Vec::new();
        recorded_commands(element.as_ref(), &ctx, &mut commands);
        let text = commands
            .iter()
            .flat_map(|command| match command {
                aimer_cupid::draw_cmd_v2::DrawCommand::DrawText { text, .. } => vec![text.as_ref()],
                aimer_cupid::draw_cmd_v2::DrawCommand::DrawRichText { spans, .. } => {
                    spans.iter().map(|span| span.text.as_ref()).collect()
                }
                _ => Vec::new(),
            })
            .collect::<String>();

        assert!(text.contains("rust"));
        assert!(text.contains("fn main() {}"));
    }

    /// Every command the elements under `element` record in their own retained
    /// lists, each recorded the way the frame loop does for a node.
    fn recorded_commands(
        element: &dyn aimer_widget::Element,
        ctx: &BuildContext<'_>,
        commands: &mut Vec<aimer_cupid::draw_cmd_v2::DrawCommand>,
    ) {
        let mut node_ctx = ctx.clone();
        node_ctx.parent_size = element.computed_size(ctx);
        if element.can_paint_local_v2(&node_ctx) {
            let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
            let root = tree
                .add_root(aimer_cupid::draw_cmd_v2::Rect::new(
                    0.0, 0.0, 1_000.0, 1_000.0,
                ))
                .unwrap();
            let retained = tree.context(root).unwrap();
            node_ctx.with_local_v2_paint_context(retained, |node_ctx| {
                element.paint_local_v2(node_ctx);
            });
            commands.extend(
                tree.draw_list_snapshot(root)
                    .unwrap()
                    .commands
                    .iter()
                    .cloned(),
            );
        }
        element.visit_children(&mut |child| recorded_commands(child, ctx, commands));
    }

    #[test]
    fn plain_markdown_paragraph_uses_plain_text_painting() {
        let resolver: ImageResolver = Rc::new(default_image_resolver);
        let document = Document::parse("A plain Markdown paragraph").unwrap();
        let (ctx, _canvas) = layout_context_with_canvas(320.0, 200.0);
        let element = render_document(
            &document,
            &MarkdownTheme::default(),
            None,
            &resolver,
            &[],
            &[],
        )
        .to_element(&ctx);

        element.layout(&ctx);

        let mut commands = Vec::new();
        recorded_commands(element.as_ref(), &ctx, &mut commands);
        assert!(commands.iter().any(|command| {
            matches!(
                command,
                aimer_cupid::draw_cmd_v2::DrawCommand::DrawText { text, .. }
                    if text.as_ref() == "A plain Markdown paragraph"
            )
        }));
        assert!(!commands.iter().any(|command| matches!(
            command,
            aimer_cupid::draw_cmd_v2::DrawCommand::DrawRichText { .. }
        )));
    }

    #[test]
    fn formatted_markdown_paragraph_keeps_rich_text_painting() {
        let resolver: ImageResolver = Rc::new(default_image_resolver);
        let theme = MarkdownTheme::default();
        let ctx = layout_context(320.0, 200.0);
        let element = render_inline_flow(
            &[Inline::Strong(vec![Inline::Text("Rich Markdown".into())])],
            theme.body,
            None,
            &theme,
            None,
            &resolver,
            &[],
            &[],
            None,
        )
        .to_element(&ctx);

        assert_eq!(element.debug_name(), "RawRichText");
    }

    #[test]
    fn custom_inline_widgets_wrap_with_their_surrounding_text() {
        use crate::{CustomInlineData, InlineRule, InlineSyntax};

        let rule = InlineRule::new(
            "button",
            InlineSyntax::Paired {
                opening: "{{button:",
                closing: "}}",
            },
        );
        let document = Document::parse_with_rules(
            "This sentence is deliberately long before {{button:continue}} and after it.",
            &[],
            std::slice::from_ref(&rule),
        )
        .unwrap();
        let Block::Paragraph(inlines) = &document.blocks[0] else {
            panic!("expected a paragraph with inline content");
        };
        let custom_inlines = vec![(
            rule,
            Rc::new(|data: &CustomInlineData| {
                assert_eq!(data.label.as_ref(), "continue");
                SizedBox::new().width(72).height(24).boxed()
            }) as CustomInlineBuilder,
        )];
        let resolver: ImageResolver = Rc::new(default_image_resolver);
        let theme = MarkdownTheme::default();
        let ctx = layout_context(180.0, 160.0);
        let element = render_inline_flow(
            inlines,
            theme.body,
            None,
            &theme,
            None,
            &resolver,
            &custom_inlines,
            &[],
            None,
        )
        .to_element(&ctx);

        let size = element.computed_size(&ctx);
        assert!(size.width <= ctx.parent_size.width);
        assert!(
            size.height > 24.0,
            "the text and custom widget stayed on one clipped row: {size:?}"
        );
    }

    #[test]
    fn code_block_copy_control_fits_its_header_track() {
        let resolver: ImageResolver = Rc::new(default_image_resolver);
        let document = Document::parse("```rust\nfn main() {}\n```").unwrap();
        let (mut ctx, canvas) = layout_context_with_canvas(1700.0, 200.0);
        ctx.scale = 2.0;
        let element = render_document(
            &document,
            &MarkdownTheme::default(),
            None,
            &resolver,
            &[],
            &[],
        )
        .to_element(&ctx);

        element.layout(&ctx);
        element.update(&ctx);

        assert!(canvas.draw_list().commands().iter().all(|command| {
            !matches!(
                command,
                DrawCommand::DrawText { text, .. } if text.contains("overflowed by")
            )
        }));
    }

    #[test]
    fn code_blocks_scroll_horizontally_without_wrapping_long_lines() {
        let resolver: ImageResolver = Rc::new(default_image_resolver);
        let document = Document::parse(
            "```rust\nlet message = \"this line is deliberately wider than the narrow viewport\";\n```",
        )
            .unwrap();

        let narrow_ctx = layout_context(180.0, 200.0);
        let narrow_size = render_document(
            &document,
            &MarkdownTheme::default(),
            None,
            &resolver,
            &[],
            &[],
        )
        .to_element(&narrow_ctx)
        .computed_size(&narrow_ctx);
        let wide_ctx = layout_context(800.0, 200.0);
        let wide_size = render_document(
            &document,
            &MarkdownTheme::default(),
            None,
            &resolver,
            &[],
            &[],
        )
        .to_element(&wide_ctx)
        .computed_size(&wide_ctx);

        assert!(
            narrow_size.width <= 180.0,
            "code block width was {}",
            narrow_size.width
        );
        assert!(
            narrow_size.height < 100.0,
            "code block height should follow its content, but was {}",
            narrow_size.height
        );
        assert_eq!(narrow_size.height, wide_size.height);
    }

    #[test]
    fn maps_nested_inline_styles_links_breaks_and_footnotes() {
        let theme = MarkdownTheme::default();
        let inlines = vec![
            Inline::Text("plain ".into()),
            Inline::Emphasis(vec![Inline::Strong(vec![Inline::Text("both".into())])]),
            Inline::Text(" ".into()),
            Inline::Delete(vec![Inline::Text("gone".into())]),
            Inline::SoftBreak,
            Inline::Code("code".into()),
            Inline::Link {
                url: "https://example.com".into(),
                title: None,
                content: vec![Inline::Text("link".into())],
            },
            Inline::FootnoteReference {
                identifier: "note".into(),
            },
        ];
        let resolved = inline_spans(&inlines, &theme).flatten(&TextStyle::default());

        assert_eq!(
            resolved
                .iter()
                .map(|span| span.text.as_ref())
                .collect::<String>(),
            "plain both gone codelink[note]"
        );
        let both = resolved
            .iter()
            .find(|span| span.text.as_ref() == "both")
            .unwrap();
        assert_eq!(both.style.font_style, FontStyle::Italic);
        assert_eq!(both.style.font_weight, FontWeight::Bold);
        let gone = resolved
            .iter()
            .find(|span| span.text.as_ref() == "gone")
            .unwrap();
        assert!(
            gone.style
                .text_decoration
                .line
                .contains(TextDecorationLine::LINE_THROUGH)
        );
        assert!(
            resolved
                .iter()
                .any(|span| span.link.as_deref() == Some("https://example.com"))
        );
        assert!(
            resolved
                .iter()
                .any(|span| span.link.as_deref() == Some("#footnote-note"))
        );
        let code_font_family = theme
            .inline_code
            .font_family
            .expect("inline code should select the Markdown code font");
        assert_eq!(
            resolved
                .iter()
                .find(|span| span.text.as_ref() == "code")
                .unwrap()
                .style
                .font_family,
            code_font_family
        );
    }

    #[test]
    fn soft_breaks_flow_as_spaces_while_hard_breaks_remain_newlines() {
        let theme = MarkdownTheme::default();
        let document = Document::parse("first\nsecond\n\nthird  \nfourth").unwrap();
        let rendered_paragraphs = document
            .blocks
            .iter()
            .map(|block| {
                let Block::Paragraph(inlines) = block else {
                    panic!("expected paragraph")
                };
                inline_spans(inlines, &theme)
                    .flatten(&TextStyle::default())
                    .into_iter()
                    .map(|span| span.text.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(rendered_paragraphs, ["first second", "third\nfourth"]);
    }

    #[test]
    fn document_rendering_invokes_the_configured_image_resolver() {
        let document = Document::parse("before ![alt](asset.png) after").unwrap();
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let resolver: ImageResolver = Rc::new(move |image| {
            assert_eq!(image.source.as_ref(), "asset.png");
            assert_eq!(image.alt.as_ref(), "alt");
            observed.set(observed.get() + 1);
            SizedBox::new().boxed()
        });

        let _widget = render_document(
            &document,
            &MarkdownTheme::default(),
            None,
            &resolver,
            &[],
            &[],
        );
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn document_rendering_invokes_custom_block_and_inline_builders() {
        use crate::{
            BlockRule, BlockSyntax, CustomBlockData, CustomInlineData, InlineRule, InlineSyntax,
        };

        let document = Document::parse_with_rules(
            ":::alert\nblock body\n:::\n\nClick {{button:continue}}.",
            &[BlockRule::new(
                "alert",
                BlockSyntax::Paired {
                    opening: ":::alert",
                    closing: ":::",
                },
            )],
            &[InlineRule::new(
                "button",
                InlineSyntax::Paired {
                    opening: "{{button:",
                    closing: "}}",
                },
            )],
        )
        .unwrap();
        let block_calls = Rc::new(Cell::new(0));
        let inline_calls = Rc::new(Cell::new(0));
        let observed_block = block_calls.clone();
        let observed_inline = inline_calls.clone();
        let blocks = vec![(
            BlockRule::new(
                "alert",
                BlockSyntax::Paired {
                    opening: ":::alert",
                    closing: ":::",
                },
            ),
            Rc::new(move |data: &CustomBlockData| {
                assert_eq!(data.text.as_ref(), "block body");
                observed_block.set(observed_block.get() + 1);
                Text::new(data.text.clone()).boxed()
            }) as CustomBlockBuilder,
        )];
        let inlines = vec![(
            InlineRule::new(
                "button",
                InlineSyntax::Paired {
                    opening: "{{button:",
                    closing: "}}",
                },
            ),
            Rc::new(move |data: &CustomInlineData| {
                assert_eq!(data.label.as_ref(), "continue");
                observed_inline.set(observed_inline.get() + 1);
                Text::new(data.label.clone()).boxed()
            }) as CustomInlineBuilder,
        )];
        let resolver: ImageResolver = Rc::new(default_image_resolver);

        let _widget = render_document(
            &document,
            &MarkdownTheme::default(),
            None,
            &resolver,
            &blocks,
            &inlines,
        );

        assert_eq!(block_calls.get(), 1);
        assert_eq!(inline_calls.get(), 1);
    }
}
