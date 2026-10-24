use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};

use aimer_rubick::ShareRef;
use aimer_utils::error;
use markdown::mdast::{AlignKind, Node};
use markdown::{Constructs, ParseOptions};

use crate::custom::{BlockRule, CustomBlockData, CustomInlineData, InlineRule};

#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading {
        depth: u8,
        content: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Blockquote(Vec<Block>),
    List {
        ordered: bool,
        start: Option<u32>,
        items: Vec<ListItem>,
    },
    Code {
        value: ShareRef<str>,
        language: Option<ShareRef<str>>,
        meta: Option<ShareRef<str>>,
    },
    ThematicBreak,
    Table {
        alignments: Vec<Alignment>,
        rows: Vec<TableRow>,
    },
    FootnoteDefinition {
        identifier: ShareRef<str>,
        blocks: Vec<Block>,
    },
    Custom(CustomBlockData),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableRow {
    pub cells: Vec<Vec<Inline>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(ShareRef<str>),
    SoftBreak,
    HardBreak,
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Delete(Vec<Inline>),
    Code(ShareRef<str>),
    Link {
        url: ShareRef<str>,
        title: Option<ShareRef<str>>,
        content: Vec<Inline>,
    },
    Image {
        url: ShareRef<str>,
        title: Option<ShareRef<str>>,
        alt: ShareRef<str>,
    },
    FootnoteReference {
        identifier: ShareRef<str>,
    },
    Custom(CustomInlineData),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownError {
    message: String,
}

impl MarkdownError {
    /// Creates an error describing an invalid Markdown document or extension.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for MarkdownError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MarkdownError {}

struct PreparedSource<'a> {
    source: Cow<'a, str>,
    blocks: HashMap<String, CustomBlockData>,
}

pub(crate) fn share_ref_range(source: &ShareRef<str>, start: usize, end: usize) -> ShareRef<str> {
    source.clone().project(move |source| &source[start..end])
}

fn validate_rules(
    block_rules: &[BlockRule],
    inline_rules: &[InlineRule],
) -> Result<(), MarkdownError> {
    for (index, rule) in block_rules.iter().enumerate() {
        let (opening, closing) = rule.delimiters();
        if rule.name().is_empty() || opening.is_empty() || closing.is_empty() || opening == closing
        {
            return Err(MarkdownError::new(format!(
                "Custom block rule '{}' has invalid delimiters",
                rule.name()
            )));
        }
        if block_rules[..index]
            .iter()
            .any(|other| other.name() == rule.name() || other.delimiters() == (opening, closing))
        {
            return Err(MarkdownError::new(format!(
                "Duplicate custom block rule '{}'",
                rule.name()
            )));
        }
    }
    for (index, rule) in inline_rules.iter().enumerate() {
        let (opening, closing) = rule.delimiters();
        if rule.name().is_empty() || opening.is_empty() || closing.is_empty() || opening == closing
        {
            return Err(MarkdownError::new(format!(
                "Custom inline rule '{}' has invalid delimiters",
                rule.name()
            )));
        }
        if inline_rules[..index]
            .iter()
            .any(|other| other.name() == rule.name() || other.delimiters() == (opening, closing))
        {
            return Err(MarkdownError::new(format!(
                "Duplicate custom inline rule '{}'",
                rule.name()
            )));
        }
    }
    Ok(())
}

fn prepare_source<'a>(
    source: &'a str,
    source_owner: Option<&ShareRef<str>>,
    block_rules: &[BlockRule],
    inline_rules: &[InlineRule],
) -> Result<PreparedSource<'a>, MarkdownError> {
    let mut prepared: Option<String> = None;
    let mut blocks = HashMap::new();
    let mut lines = source.split_inclusive('\n');
    let mut source_offset = 0;
    let mut token_index = 0;

    while let Some(line) = lines.next() {
        let line_offset = source_offset;
        source_offset += line.len();
        let line_content = line.trim_matches(['\r', '\n']);
        let Some(rule) = block_rules
            .iter()
            .find(|rule| rule.delimiters().0 == line_content.trim())
        else {
            if let Some(prepared) = &mut prepared {
                prepared.push_str(line);
            }
            continue;
        };

        if prepared.is_none() {
            let mut prepared_source = String::with_capacity(source.len());
            prepared_source.push_str(&source[..line_offset]);
            prepared = Some(prepared_source);
        }

        let (_, closing) = rule.delimiters();
        let body_start = source_offset;
        let mut nesting = 1_usize;
        let mut closed = false;
        let mut body_end = 0;
        while let Some(inner_line) = lines.next() {
            let inner_line_offset = source_offset;
            source_offset += inner_line.len();
            let inner_content = inner_line.trim_matches(['\r', '\n']);
            if inner_content.trim() == rule.delimiters().0 {
                nesting += 1;
            } else if inner_content.trim() == closing {
                nesting -= 1;
                if nesting == 0 {
                    closed = true;
                    body_end = inner_line_offset;
                    break;
                }
            }
        }
        if !closed {
            return Err(MarkdownError::new(format!(
                "Unclosed custom block '{}'",
                rule.name()
            )));
        }

        let raw_body = &source[body_start..body_end];
        let body = raw_body.trim_end_matches(['\r', '\n']);
        let body_end = body_start + body.len();
        let text = source_owner
            .map(|source| share_ref_range(source, body_start, body_end))
            .unwrap_or_else(|| ShareRef::from(body.to_owned()));
        let content = Document::parse_shared_with_rules(text.clone(), block_rules, inline_rules)?;
        let token = loop {
            let token = format!("AIMER_CUSTOM_BLOCK_{token_index}");
            token_index += 1;
            if !source.contains(&token) {
                break token;
            }
        };
        let prepared = prepared
            .as_mut()
            .expect("custom block should have an owned prepared source");
        prepared.push_str(&token);
        prepared.push('\n');
        blocks.insert(
            token,
            CustomBlockData {
                name: rule.shared_name(),
                text,
                content,
            },
        );
    }

    Ok(PreparedSource {
        source: prepared.map_or(Cow::Borrowed(source), Cow::Owned),
        blocks,
    })
}

fn replace_custom_blocks(blocks: &mut [Block], captures: &mut HashMap<String, CustomBlockData>) {
    for block in blocks {
        match block {
            Block::Paragraph(inlines) if inlines.len() == 1 => {
                let Some(Inline::Text(token)) = inlines.first() else {
                    continue;
                };
                if let Some(data) = captures.remove(token.as_ref()) {
                    *block = Block::Custom(data);
                }
            }
            Block::Blockquote(children) => replace_custom_blocks(children, captures),
            Block::List { items, .. } => items
                .iter_mut()
                .for_each(|item| replace_custom_blocks(&mut item.blocks, captures)),
            Block::FootnoteDefinition { blocks, .. } => replace_custom_blocks(blocks, captures),
            _ => {}
        }
    }
}

fn split_custom_inlines(
    result: &mut Vec<Inline>,
    value: ShareRef<str>,
    rules: &[InlineRule],
) -> Result<(), MarkdownError> {
    let source = value.as_ref();
    let Some((mut start, mut rule)) = find_custom_opening(source, rules) else {
        if !source.is_empty() {
            result.push(Inline::Text(value));
        }
        return Ok(());
    };

    let mut offset = 0;
    loop {
        let remaining = &source[offset..];
        if start > 0 {
            result.push(Inline::Text(share_ref_range(
                &value,
                offset,
                offset + start,
            )));
        }
        let (opening, closing) = rule.delimiters();
        let value_start = start + opening.len();
        let Some(relative_end) = find_unescaped(remaining[value_start..].as_bytes(), closing)
        else {
            return Err(MarkdownError::new(format!(
                "Unclosed custom inline '{}'",
                rule.name()
            )));
        };
        let value_end = value_start + relative_end;
        let custom_value = &remaining[value_start..value_end];
        if rules
            .iter()
            .any(|nested| find_unescaped(custom_value.as_bytes(), nested.delimiters().0).is_some())
        {
            return Err(MarkdownError::new(format!(
                "Nested custom inline '{}' is not supported",
                rule.name()
            )));
        }
        let custom_value = share_ref_range(&value, offset + value_start, offset + value_end);
        result.push(Inline::Custom(CustomInlineData {
            name: rule.shared_name(),
            text: custom_value.clone(),
            label: custom_value,
        }));
        offset += value_end + closing.len();
        let remaining = &source[offset..];
        let Some((next_start, next_rule)) = find_custom_opening(remaining, rules) else {
            break;
        };
        start = next_start;
        rule = next_rule;
    }
    if offset < source.len() {
        result.push(Inline::Text(share_ref_range(&value, offset, source.len())));
    }
    Ok(())
}

fn find_custom_opening<'a>(
    value: &str,
    rules: &'a [InlineRule],
) -> Option<(usize, &'a InlineRule)> {
    rules
        .iter()
        .filter_map(|rule| {
            find_unescaped(value.as_bytes(), rule.delimiters().0).map(|start| (start, rule))
        })
        .min_by_key(|(start, _)| *start)
}

fn find_unescaped(value: &[u8], needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    let mut offset = 0;
    while offset + needle.len() <= value.len() {
        let Some(relative) = value[offset..]
            .windows(needle.len())
            .position(|window| window == needle)
        else {
            return None;
        };
        let position = offset + relative;
        let mut slash_count = 0;
        let mut slash = position;
        while slash > 0 && value[slash - 1] == b'\\' {
            slash_count += 1;
            slash -= 1;
        }
        if slash_count % 2 == 0 {
            return Some(position);
        }
        offset = position + needle.len();
    }
    None
}

impl Document {
    pub fn parse(source: impl AsRef<str>) -> Result<Self, MarkdownError> {
        Self::parse_with_rules(source.as_ref(), &[], &[])
    }

    pub(crate) fn parse_shared(source: ShareRef<str>) -> Result<Self, MarkdownError> {
        Self::parse_shared_with_rules(source, &[], &[])
    }

    pub(crate) fn parse_shared_with_rules(
        source: ShareRef<str>,
        block_rules: &[BlockRule],
        inline_rules: &[InlineRule],
    ) -> Result<Self, MarkdownError> {
        Self::parse_with_source(&source, Some(&source), block_rules, inline_rules)
    }

    pub(crate) fn parse_with_rules(
        source: &str,
        block_rules: &[BlockRule],
        inline_rules: &[InlineRule],
    ) -> Result<Self, MarkdownError> {
        Self::parse_with_source(source, None, block_rules, inline_rules)
    }

    fn parse_with_source(
        source: &str,
        source_owner: Option<&ShareRef<str>>,
        block_rules: &[BlockRule],
        inline_rules: &[InlineRule],
    ) -> Result<Self, MarkdownError> {
        validate_rules(block_rules, inline_rules)?;
        let mut prepared = prepare_source(source, source_owner, block_rules, inline_rules)?;
        let options = ParseOptions {
            constructs: Constructs::gfm(),
            gfm_strikethrough_single_tilde: false,
            ..ParseOptions::default()
        };
        let root = markdown::to_mdast(prepared.source.as_ref(), &options)
            .map_err(|error| MarkdownError::new(error.to_string()))?;
        let Node::Root(root) = root else {
            return Err(MarkdownError::new(
                "Markdown parser did not produce a document root",
            ));
        };
        let mut blocks = convert_blocks(root.children, inline_rules)?;
        replace_custom_blocks(&mut blocks, &mut prepared.blocks);
        Ok(Self { blocks })
    }
}

fn convert_blocks(
    nodes: Vec<Node>,
    inline_rules: &[InlineRule],
) -> Result<Vec<Block>, MarkdownError> {
    nodes
        .into_iter()
        .map(|node| convert_block(node, inline_rules))
        .collect()
}

fn convert_block(node: Node, inline_rules: &[InlineRule]) -> Result<Block, MarkdownError> {
    match node {
        Node::Heading(heading) => Ok(Block::Heading {
            depth: heading.depth,
            content: convert_inlines(heading.children, inline_rules)?,
        }),
        Node::Paragraph(paragraph) => Ok(Block::Paragraph(convert_inlines(
            paragraph.children,
            inline_rules,
        )?)),
        Node::Blockquote(quote) => Ok(Block::Blockquote(convert_blocks(
            quote.children,
            inline_rules,
        )?)),
        Node::List(list) => {
            let ordered = list.ordered;
            let start = list.start;
            let items = list
                .children
                .into_iter()
                .map(|node| {
                    let Node::ListItem(item) = node else {
                        return Err(MarkdownError::new(
                            "Markdown list contains a non-list-item node",
                        ));
                    };
                    Ok(ListItem {
                        checked: item.checked,
                        blocks: convert_blocks(item.children, inline_rules)?,
                    })
                })
                .collect::<Result<_, MarkdownError>>()?;
            Ok(Block::List {
                ordered,
                start,
                items,
            })
        }
        Node::Code(code) => Ok(Block::Code {
            value: code.value.into(),
            language: code.lang.map(ShareRef::from),
            meta: code.meta.map(ShareRef::from),
        }),
        Node::ThematicBreak(_) => Ok(Block::ThematicBreak),
        Node::Table(table) => {
            let alignments = table.align.into_iter().map(Alignment::from).collect();
            let rows = table
                .children
                .into_iter()
                .map(|node| {
                    let Node::TableRow(row) = node else {
                        return Err(MarkdownError::new("Markdown table contains a non-row node"));
                    };
                    let cells = row
                        .children
                        .into_iter()
                        .map(|node| {
                            let Node::TableCell(cell) = node else {
                                return Err(MarkdownError::new(
                                    "Markdown table row contains a non-cell node",
                                ));
                            };
                            convert_inlines(cell.children, inline_rules)
                        })
                        .collect::<Result<_, MarkdownError>>()?;
                    Ok(TableRow { cells })
                })
                .collect::<Result<_, MarkdownError>>()?;
            Ok(Block::Table { alignments, rows })
        }
        Node::FootnoteDefinition(footnote) => Ok(Block::FootnoteDefinition {
            identifier: footnote.identifier.into(),
            blocks: convert_blocks(footnote.children, inline_rules)?,
        }),
        Node::Html(_) => Err(MarkdownError::new(
            "Raw HTML is not supported in MarkdownViewer",
        )),
        other => Err(MarkdownError::new(format!(
            "Unsupported Markdown block node: {}",
            node_name(&other)
        ))),
    }
}

fn convert_inlines(
    nodes: Vec<Node>,
    inline_rules: &[InlineRule],
) -> Result<Vec<Inline>, MarkdownError> {
    let mut result = Vec::new();
    for node in nodes {
        match node {
            Node::Text(text) => push_text_with_soft_breaks(&mut result, text.value, inline_rules)?,
            Node::Break(_) => result.push(Inline::HardBreak),
            Node::Emphasis(emphasis) => result.push(Inline::Emphasis(convert_inlines(
                emphasis.children,
                inline_rules,
            )?)),
            Node::Strong(strong) => result.push(Inline::Strong(convert_inlines(
                strong.children,
                inline_rules,
            )?)),
            Node::Delete(delete) => result.push(Inline::Delete(convert_inlines(
                delete.children,
                inline_rules,
            )?)),
            Node::InlineCode(code) => result.push(Inline::Code(code.value.into())),
            Node::Link(link) => result.push(Inline::Link {
                url: link.url.into(),
                title: link.title.map(ShareRef::from),
                content: convert_inlines(link.children, inline_rules)?,
            }),
            Node::Image(image) => result.push(Inline::Image {
                url: image.url.into(),
                title: image.title.map(ShareRef::from),
                alt: image.alt.into(),
            }),
            Node::FootnoteReference(reference) => result.push(Inline::FootnoteReference {
                identifier: reference.identifier.into(),
            }),
            Node::Html(item) => {
                error!("Raw HTML is not supported in MarkdownViewer : {:?}", item);
                return Err(MarkdownError::new(
                    "Raw HTML is not supported in MarkdownViewer",
                ));
            }
            other => {
                error!("Unsupported Markdown inline node: {}", node_name(&other));
                return Err(MarkdownError::new(format!(
                    "Unsupported Markdown inline node: {}",
                    node_name(&other)
                )));
            }
        }
    }
    Ok(result)
}

fn push_text_with_soft_breaks(
    result: &mut Vec<Inline>,
    value: String,
    inline_rules: &[InlineRule],
) -> Result<(), MarkdownError> {
    let value = ShareRef::from(value);
    if !value.contains('\n') {
        return push_extended_image_text(result, value, inline_rules);
    }

    let mut parts = value.as_ref().split('\n').peekable();
    let mut offset = 0;
    while let Some(part) = parts.next() {
        let part_length = part.len();
        let has_next = parts.peek().is_some();
        push_extended_image_text(
            result,
            share_ref_range(&value, offset, offset + part_length),
            inline_rules,
        )?;
        offset += part_length;
        if has_next {
            result.push(Inline::SoftBreak);
            offset += 1;
        }
    }
    Ok(())
}

fn find_extended_image(value: &str) -> Option<(usize, usize, usize, usize)> {
    let start = value.find("![")?;
    let alt_end = start + 2 + value[start + 2..].find("](")?;
    let destination_start = alt_end + 2;
    let destination_end = destination_start + value[destination_start..].find(')')?;
    let destination = value[destination_start..destination_end].trim();
    if destination.contains(' ') && !destination.is_empty() {
        Some((start, alt_end, destination_start, destination_end))
    } else {
        None
    }
}

fn push_extended_image_text(
    result: &mut Vec<Inline>,
    value: ShareRef<str>,
    inline_rules: &[InlineRule],
) -> Result<(), MarkdownError> {
    let source = value.as_ref();
    let Some((mut start, mut alt_end, mut destination_start, mut destination_end)) =
        find_extended_image(source)
    else {
        return split_custom_inlines(result, value, inline_rules);
    };
    let mut offset = 0;
    loop {
        let remaining = &source[offset..];
        let raw_destination_start = destination_start;
        let raw_destination_end = destination_end;
        let raw_destination = &remaining[raw_destination_start..raw_destination_end];
        let destination_start_offset = offset + raw_destination_start + raw_destination.len()
            - raw_destination.trim_start().len();
        let destination_end_offset =
            offset + raw_destination_start + raw_destination.trim_end().len();
        if start > 0 {
            result.push(Inline::Text(share_ref_range(
                &value,
                offset,
                offset + start,
            )));
        }
        result.push(Inline::Image {
            url: share_ref_range(&value, destination_start_offset, destination_end_offset),
            title: None,
            alt: share_ref_range(&value, offset + start + 2, offset + alt_end),
        });
        offset += raw_destination_end + 1;
        let remaining = &source[offset..];
        let Some((next_start, next_alt_end, next_destination_start, next_destination_end)) =
            find_extended_image(remaining)
        else {
            break;
        };
        start = next_start;
        alt_end = next_alt_end;
        destination_start = next_destination_start;
        destination_end = next_destination_end;
    }
    if offset < source.len() {
        split_custom_inlines(
            result,
            share_ref_range(&value, offset, source.len()),
            inline_rules,
        )?;
    }
    Ok(())
}

fn node_name(node: &Node) -> &'static str {
    match node {
        Node::Root(_) => "root",
        Node::Blockquote(_) => "blockquote",
        Node::FootnoteDefinition(_) => "footnote definition",
        Node::MdxJsxFlowElement(_) => "MDX flow element",
        Node::List(_) => "list",
        Node::MdxjsEsm(_) => "MDX ESM",
        Node::Toml(_) => "TOML",
        Node::Yaml(_) => "YAML",
        Node::Break(_) => "break",
        Node::InlineCode(_) => "inline code",
        Node::InlineMath(_) => "inline math",
        Node::Delete(_) => "delete",
        Node::Emphasis(_) => "emphasis",
        Node::MdxTextExpression(_) => "MDX text expression",
        Node::FootnoteReference(_) => "footnote reference",
        Node::Html(_) => "HTML",
        Node::Image(_) => "image",
        Node::ImageReference(_) => "image reference",
        Node::MdxJsxTextElement(_) => "MDX text element",
        Node::Link(_) => "link",
        Node::LinkReference(_) => "link reference",
        Node::Strong(_) => "strong",
        Node::Text(_) => "text",
        Node::Code(_) => "code",
        Node::Math(_) => "math",
        Node::MdxFlowExpression(_) => "MDX flow expression",
        Node::Heading(_) => "heading",
        Node::Table(_) => "table",
        Node::ThematicBreak(_) => "thematic break",
        Node::TableRow(_) => "table row",
        Node::TableCell(_) => "table cell",
        Node::ListItem(_) => "list item",
        Node::Definition(_) => "definition",
        Node::Paragraph(_) => "paragraph",
    }
}

impl From<AlignKind> for Alignment {
    fn from(value: AlignKind) -> Self {
        match value {
            AlignKind::None => Self::None,
            AlignKind::Left => Self::Left,
            AlignKind::Center => Self::Center,
            AlignKind::Right => Self::Right,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockRule, BlockSyntax, InlineRule, InlineSyntax};
    use aimer_rubick::ShareRef;

    fn parse(source: &str) -> Document {
        Document::parse(source).expect("fixture should parse")
    }

    fn push_text_storage(value: &ShareRef<str>, output: &mut Vec<(*const u8, usize)>) {
        output.push((value.as_ref().as_ptr(), value.len()));
    }

    fn collect_inline_text_storage(inlines: &[Inline], output: &mut Vec<(*const u8, usize)>) {
        for inline in inlines {
            match inline {
                Inline::Text(value) | Inline::Code(value) => push_text_storage(value, output),
                Inline::SoftBreak | Inline::HardBreak => {}
                Inline::Emphasis(children)
                | Inline::Strong(children)
                | Inline::Delete(children) => collect_inline_text_storage(children, output),
                Inline::Link {
                    url,
                    title,
                    content,
                } => {
                    push_text_storage(url, output);
                    if let Some(title) = title {
                        push_text_storage(title, output);
                    }
                    collect_inline_text_storage(content, output);
                }
                Inline::Image { url, title, alt } => {
                    push_text_storage(url, output);
                    if let Some(title) = title {
                        push_text_storage(title, output);
                    }
                    push_text_storage(alt, output);
                }
                Inline::FootnoteReference { identifier } => {
                    push_text_storage(identifier, output);
                }
                Inline::Custom(data) => {
                    push_text_storage(&data.name, output);
                    push_text_storage(&data.text, output);
                    push_text_storage(&data.label, output);
                }
            }
        }
    }

    fn collect_block_text_storage(blocks: &[Block], output: &mut Vec<(*const u8, usize)>) {
        for block in blocks {
            match block {
                Block::Heading { content, .. } | Block::Paragraph(content) => {
                    collect_inline_text_storage(content, output);
                }
                Block::Blockquote(children) => collect_block_text_storage(children, output),
                Block::List { items, .. } => {
                    for item in items {
                        collect_block_text_storage(&item.blocks, output);
                    }
                }
                Block::Code {
                    value,
                    language,
                    meta,
                } => {
                    push_text_storage(value, output);
                    if let Some(language) = language {
                        push_text_storage(language, output);
                    }
                    if let Some(meta) = meta {
                        push_text_storage(meta, output);
                    }
                }
                Block::ThematicBreak => {}
                Block::Table { rows, .. } => {
                    for row in rows {
                        for cell in &row.cells {
                            collect_inline_text_storage(cell, output);
                        }
                    }
                }
                Block::FootnoteDefinition { identifier, blocks } => {
                    push_text_storage(identifier, output);
                    collect_block_text_storage(blocks, output);
                }
                Block::Custom(data) => {
                    push_text_storage(&data.name, output);
                    push_text_storage(&data.text, output);
                    collect_block_text_storage(&data.content.blocks, output);
                }
            }
        }
    }

    fn assert_cloned_document_shares_text_storage(document: &Document) {
        let cloned = document.clone();
        let mut original_storage = Vec::new();
        let mut cloned_storage = Vec::new();
        collect_block_text_storage(&document.blocks, &mut original_storage);
        collect_block_text_storage(&cloned.blocks, &mut cloned_storage);
        assert_eq!(original_storage, cloned_storage);
    }

    #[test]
    fn cloning_a_parsed_document_reuses_code_block_storage() {
        let document = parse("```rust\nlet answer = 42;\n```");
        let cloned = document.clone();
        let Block::Code { value, .. } = &document.blocks[0] else {
            panic!("expected a fenced code block")
        };
        let Block::Code {
            value: cloned_value,
            ..
        } = &cloned.blocks[0]
        else {
            panic!("expected a cloned fenced code block")
        };

        assert!(
            std::ptr::eq(value.as_ptr(), cloned_value.as_ptr()),
            "cloning a parsed document should share immutable code text"
        );
    }

    #[test]
    fn cloning_parsed_documents_reuses_all_text_storage() {
        let document = parse(
            "# heading *styled*\n\nparagraph `inline` [link](https://example.test \"title\") ![alt text](image.png \"image title\") ref[^note].\n\n```rust meta=demo\nlet value = 42;\n```\n\n> quote *text*\n\n- list **item**\n\n[^note]: footnote text\n\n| cell |\n| --- |\n| value |",
        );
        assert_cloned_document_shares_text_storage(&document);

        let custom_document = Document::parse_with_rules(
            ":::alert\nblock body\n:::\n\nClick {{button:continue}}.",
            &[BlockRule::new(
                "alert",
                crate::BlockSyntax::Paired {
                    opening: ":::alert",
                    closing: ":::",
                },
            )],
            &[InlineRule::new(
                "button",
                crate::InlineSyntax::Paired {
                    opening: "{{button:",
                    closing: "}}",
                },
            )],
        )
        .expect("custom Markdown should parse");
        assert_cloned_document_shares_text_storage(&custom_document);
    }

    #[test]
    fn parses_paired_custom_blocks_and_inlines() {
        let block_rule = BlockRule::new(
            "alert",
            BlockSyntax::Paired {
                opening: ":::alert",
                closing: ":::",
            },
        );
        let inline_rule = InlineRule::new(
            "button",
            InlineSyntax::Paired {
                opening: "{{button:",
                closing: "}}",
            },
        );

        let document = Document::parse_with_rules(
            ":::alert\n**Important**\n:::\n\nClick {{button:continue}}.",
            &[block_rule],
            &[inline_rule],
        )
        .expect("custom Markdown should parse");

        assert!(matches!(
            &document.blocks[0],
            Block::Custom(data)
                if data.name.as_ref() == "alert"
                    && data.text.as_ref() == "**Important**"
                    && matches!(data.content.blocks.as_slice(), [Block::Paragraph(_)])
        ));
        let Block::Paragraph(inlines) = &document.blocks[1] else {
            panic!("expected custom inline paragraph")
        };
        assert!(matches!(
            inlines.as_slice(),
            [Inline::Text(prefix), Inline::Custom(data), Inline::Text(suffix)]
                if prefix.as_ref() == "Click "
                    && data.name.as_ref() == "button"
                    && data.text.as_ref() == "continue"
                    && suffix.as_ref() == "."
        ));
    }

    #[test]
    fn rejects_duplicate_and_unclosed_custom_rules() {
        let rule = BlockRule::new(
            "alert",
            BlockSyntax::Paired {
                opening: ":::alert",
                closing: ":::",
            },
        );
        let duplicate =
            Document::parse_with_rules(":::alert\nbody\n:::", &[rule.clone(), rule.clone()], &[])
                .expect_err("duplicate rules must be rejected");
        assert!(duplicate.message().contains("Duplicate custom block rule"));

        let unclosed = Document::parse_with_rules(":::alert\nbody", &[rule], &[])
            .expect_err("unclosed blocks must be rejected");
        assert!(unclosed.message().contains("Unclosed custom block"));

        let unclosed_inline = Document::parse_with_rules(
            "Click {{button:value",
            &[],
            &[InlineRule::new(
                "button",
                InlineSyntax::Paired {
                    opening: "{{button:",
                    closing: "}}",
                },
            )],
        )
        .expect_err("unclosed inline values must be rejected");
        assert!(unclosed_inline.message().contains("Unclosed custom inline"));
    }

    fn inline_text(inlines: &[Inline]) -> String {
        inlines
            .iter()
            .map(|inline| match inline {
                Inline::Text(value) | Inline::Code(value) => value.to_string(),
                Inline::SoftBreak | Inline::HardBreak => "\n".to_string(),
                Inline::Emphasis(children)
                | Inline::Strong(children)
                | Inline::Delete(children) => inline_text(children),
                Inline::Link { content, .. } => inline_text(content),
                Inline::Image { alt, .. } => alt.to_string(),
                Inline::FootnoteReference { identifier } => identifier.to_string(),
                Inline::Custom(data) => data.text.to_string(),
            })
            .collect()
    }

    #[test]
    fn parses_headings_and_all_emphasis_forms() {
        let document = parse(
            "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6\n\n*italic* **bold** ***both*** ~~gone~~",
        );
        for (index, block) in document.blocks[..6].iter().enumerate() {
            assert!(matches!(block, Block::Heading { depth, .. } if *depth == index as u8 + 1));
        }
        let Block::Paragraph(inlines) = &document.blocks[6] else {
            panic!("expected paragraph")
        };
        assert!(matches!(&inlines[0], Inline::Emphasis(_)));
        assert!(
            inlines
                .iter()
                .any(|inline| matches!(inline, Inline::Strong(_)))
        );
        assert!(
            inlines
                .iter()
                .any(|inline| matches!(inline, Inline::Delete(_)))
        );
        assert!(
            matches!(inlines.iter().find(|inline| inline_text(std::slice::from_ref(inline)) == "both"), Some(Inline::Emphasis(children)) if matches!(children.as_slice(), [Inline::Strong(_)]))
        );
    }

    #[test]
    fn parses_lists_tasks_and_nested_blocks() {
        let document = parse("- plain\n- [x] done\n- [ ] todo\n  1. nested\n  2. second");
        let Block::List {
            ordered,
            start,
            items,
        } = &document.blocks[0]
        else {
            panic!("expected list")
        };
        assert!(!ordered);
        assert_eq!(*start, None);
        assert_eq!(
            items.iter().map(|item| item.checked).collect::<Vec<_>>(),
            [None, Some(true), Some(false)]
        );
        assert!(matches!(
            items[2].blocks.last(),
            Some(Block::List {
                ordered: true,
                start: Some(1),
                ..
            })
        ));
    }

    #[test]
    fn parses_links_images_autolinks_and_footnotes() {
        let document = parse(
            "[plain](https://example.com) [titled](https://example.com \"title\") <https://a.test> ![alt](image.jpg \"caption\") ref[^One].\n\n[^One]: Footnote *text*.",
        );
        let Block::Paragraph(inlines) = &document.blocks[0] else {
            panic!("expected paragraph")
        };
        assert!(inlines.iter().any(|inline| matches!(inline, Inline::Link { url, title: None, .. } if url.as_ref() == "https://example.com")));
        assert!(inlines.iter().any(
            |inline| matches!(inline, Inline::Link { title: Some(title), .. } if title.as_ref() == "title")
        ));
        assert!(inlines.iter().any(|inline| matches!(inline, Inline::Image { url, title: Some(title), alt } if url.as_ref() == "image.jpg" && title.as_ref() == "caption" && alt.as_ref() == "alt")));
        assert!(inlines.iter().any(|inline| matches!(inline, Inline::FootnoteReference { identifier } if identifier.as_ref() == "one")));
        assert!(
            matches!(&document.blocks[1], Block::FootnoteDefinition { identifier, .. } if identifier.as_ref() == "one")
        );
    }

    #[test]
    fn accepts_the_reference_image_destination_with_spaces() {
        let document = parse("![alt text](image line here)");
        assert!(matches!(
            &document.blocks[0],
            Block::Paragraph(inlines)
                if matches!(inlines.as_slice(), [Inline::Image { url, alt, .. }] if url.as_ref() == "image line here" && alt.as_ref() == "alt text")
        ));
    }

    #[test]
    fn parses_quotes_code_rules_breaks_and_escapes() {
        let document = parse(
            "> outer\n>> inner\n\n`inline`\n\n```python title=demo\nprint('ok')\n```\n\n    indented\n\n---\n\nline one  \nline two\nsoft\nline\n\n\\*literal\\*",
        );
        assert!(
            matches!(&document.blocks[0], Block::Blockquote(children) if matches!(children.get(1), Some(Block::Blockquote(_))))
        );
        assert!(
            matches!(&document.blocks[1], Block::Paragraph(inlines) if matches!(inlines.as_slice(), [Inline::Code(value)] if value.as_ref() == "inline"))
        );
        assert!(
            matches!(&document.blocks[2], Block::Code { language: Some(language), meta: Some(meta), value } if language.as_ref() == "python" && meta.as_ref() == "title=demo" && value.as_ref() == "print('ok')")
        );
        assert!(
            matches!(&document.blocks[3], Block::Code { language: None, value, .. } if value.as_ref() == "indented")
        );
        assert!(matches!(&document.blocks[4], Block::ThematicBreak));
        assert!(
            matches!(&document.blocks[5], Block::Paragraph(inlines) if inlines.iter().any(|inline| matches!(inline, Inline::HardBreak)) && inlines.iter().any(|inline| matches!(inline, Inline::SoftBreak)))
        );
        assert!(
            matches!(&document.blocks[6], Block::Paragraph(inlines) if inline_text(inlines) == "*literal*")
        );
    }

    #[test]
    fn parses_table_alignment_and_inline_cell_content() {
        let document = parse(
            "| Left | Center | Right | None |\n|:-----|:------:|------:|------|\n| *a* | **b** | `c` | d |",
        );
        let Block::Table { alignments, rows } = &document.blocks[0] else {
            panic!("expected table")
        };
        assert_eq!(
            alignments,
            &[
                Alignment::Left,
                Alignment::Center,
                Alignment::Right,
                Alignment::None
            ]
        );
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[1].cells[0].as_slice(), [Inline::Emphasis(_)]));
        assert!(matches!(rows[1].cells[1].as_slice(), [Inline::Strong(_)]));
        assert!(
            matches!(rows[1].cells[2].as_slice(), [Inline::Code(value)] if value.as_ref() == "c")
        );
    }

    #[test]
    fn rejects_raw_html_without_silently_dropping_it() {
        let error = Document::parse("<script>alert('no')</script>")
            .expect_err("raw HTML is intentionally unsupported");
        assert!(error.to_string().contains("HTML"));
    }

    #[test]
    fn preparing_source_without_custom_blocks_borrows_the_shared_source() {
        let source = ShareRef::from_static("plain Markdown");
        let prepared =
            prepare_source(&source, Some(&source), &[], &[]).expect("plain source should prepare");

        assert!(matches!(
            prepared.source,
            std::borrow::Cow::Borrowed("plain Markdown")
        ));
    }
}
