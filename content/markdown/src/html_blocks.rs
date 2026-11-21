//! What the HTML in a Markdown document means, as the blocks and the inlines the document is made of.
//!
//! The viewer draws [`Block`]s and [`Inline`]s, and this makes them of tags: `<b>` is a [`Inline::Strong`], `<img>` an
//! [`Inline::Image`], `<table>` a [`Block::Table`], and `<p align="center">` a [`Block::Aligned`], so that what is
//! written in HTML is drawn as what is written in Markdown is, and has no way of its own to be drawn.
//!
//! Markdown and HTML are mixed as they are in README files: a `<details>` is opened in one block of HTML, the Markdown
//! inside it is blocks of their own, and a `</details>` closes it in another. [`BlockBuilder`] is given the HTML blocks
//! and the blocks between them in order, and keeps what is open; [`InlineBuilder`] does the same for the tags inside a
//! paragraph. A tag that is not closed is closed where what holds it ends, and an end tag that closes nothing is passed
//! over. A tag that means nothing here, `<span>` and `<sup>`, is passed over and what is in it is kept. `<script>` and
//! `<style>`, and the elements that embed something, are errors: they have nothing to show, and nothing is dropped
//! without being said.

use std::mem;

use aimer_rubick::ShareRef;

use crate::document::{Alignment, Block, Inline, ListItem, MarkdownError, TableRow};
use crate::html::{Token, tokenize};

/// The elements that cannot be shown, or that would have to be run: they are errors, so that nothing is dropped without being said.
const REJECTED: [&str; 5] = ["script", "style", "iframe", "object", "embed"];
/// The elements that hold blocks and are plain boxes of them: what is in one is as it would be outside it, and after it a new block begins.
const BOXES: [&str; 14] = [
    "p", "div", "section", "article", "header", "footer", "main", "nav", "aside", "figure", "figcaption", "address", "dl", "dd",
];
/// The elements of a table that say nothing a table does not already say.
const TABLE_PARTS: [&str; 5] = ["thead", "tbody", "tfoot", "colgroup", "col"];

fn rejected(name: &str) -> MarkdownError {
    MarkdownError::new(format!("Raw HTML <{name}> is not supported in MarkdownViewer"))
}

/// The alignment that a tag asks for, with `align="center"` or `style="text-align: center"`. Left is where blocks are already, and is none.
fn alignment_of(attributes: &[(String, String)]) -> Option<Alignment> {
    let named = |value: &str| match value.trim().to_ascii_lowercase().as_str() {
        "center" | "middle" => Some(Alignment::Center),
        "right" => Some(Alignment::Right),
        _ => None,
    };
    if let Some(alignment) = Token::attribute(attributes, "align").and_then(named) {
        return Some(alignment);
    }
    let style = Token::attribute(attributes, "style")?;
    style
        .split(';')
        .filter_map(|declaration| declaration.split_once(':'))
        .find(|(property, _)| property.trim().eq_ignore_ascii_case("text-align"))
        .and_then(|(_, value)| named(value))
}

/// What a cell asks to be aligned as; `align="left"` is the same as none.
fn cell_alignment(attributes: &[(String, String)]) -> Alignment {
    alignment_of(attributes).unwrap_or(Alignment::None)
}

/// `inlines` without the blanks at its start and its end, which HTML does not show, and nothing where there is only blanks.
fn trimmed(mut inlines: Vec<Inline>) -> Vec<Inline> {
    while let Some(Inline::Text(text)) = inlines.first() {
        let kept = text.trim_start();
        if kept.is_empty() {
            inlines.remove(0);
        } else {
            if kept.len() != text.len() {
                let kept = ShareRef::from(kept.to_owned());
                inlines[0] = Inline::Text(kept);
            }
            break;
        }
    }
    while let Some(Inline::Text(text)) = inlines.last() {
        let kept = text.trim_end();
        if kept.is_empty() {
            inlines.pop();
        } else {
            if kept.len() != text.len() {
                let kept = ShareRef::from(kept.to_owned());
                let last = inlines.len() - 1;
                inlines[last] = Inline::Text(kept);
            }
            break;
        }
    }
    inlines
}

/// The words in `inlines`, with no styling: what `<code>` and `<kbd>` show.
fn plain_text(inlines: &[Inline], output: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Code(text) => output.push_str(text),
            Inline::SoftBreak => output.push(' '),
            Inline::HardBreak => output.push('\n'),
            Inline::Emphasis(children) | Inline::Strong(children) | Inline::Delete(children) => plain_text(children, output),
            Inline::Link { content, .. } => plain_text(content, output),
            Inline::Image { alt, .. } => output.push_str(alt),
            Inline::FootnoteReference { .. } | Inline::Custom(_) => {}
        }
    }
}

/// What an open inline tag makes of what is in it, when it is closed.
enum InlineKind {
    Root,
    Strong,
    Emphasis,
    Delete,
    Code,
    Link { url: ShareRef<str>, title: Option<ShareRef<str>> },
    /// A tag with no meaning here: what is in it is kept.
    Plain,
}

struct InlineFrame {
    tag: String,
    kind: InlineKind,
    children: Vec<Inline>,
}

/// Collects the inlines of a paragraph, where HTML tags open and close among them: the inlines between `<b>` and `</b>` are the
/// children of one [`Inline::Strong`].
pub(crate) struct InlineBuilder {
    /// The root frame, which is never closed, and the tags that are open above it.
    frames: Vec<InlineFrame>,
}

impl InlineBuilder {
    pub(crate) fn new() -> Self {
        Self { frames: vec![InlineFrame { tag: String::new(), kind: InlineKind::Root, children: Vec::new() }] }
    }

    /// Where the inlines that are made now go.
    pub(crate) fn top(&mut self) -> &mut Vec<Inline> {
        &mut self.frames.last_mut().expect("the root frame is never closed").children
    }

    pub(crate) fn push(&mut self, inline: Inline) {
        self.top().push(inline);
    }

    /// Adds text of HTML, in which a run of blanks, line breaks among them, is one blank.
    pub(crate) fn text(&mut self, text: &str) {
        let mut collapsed = String::with_capacity(text.len());
        let mut blank = false;
        for character in text.chars() {
            if character.is_ascii_whitespace() {
                blank = true;
            } else {
                if blank {
                    collapsed.push(' ');
                    blank = false;
                }
                collapsed.push(character);
            }
        }
        if blank {
            collapsed.push(' ');
        }
        if !collapsed.is_empty() {
            self.push(Inline::Text(ShareRef::from(collapsed)));
        }
    }

    /// Gives the tokens of HTML that are among the inlines of a paragraph.
    pub(crate) fn feed(&mut self, tokens: Vec<Token>) -> Result<(), MarkdownError> {
        for token in tokens {
            match token {
                Token::Text(text) => self.text(&text),
                Token::Open { name, attributes, self_closing } => {
                    self.open(&name, &attributes)?;
                    if self_closing {
                        self.close(&name);
                    }
                }
                Token::Close { name } => self.close(&name),
            }
        }
        Ok(())
    }

    /// A start tag: a break and an image are made at once, and any other is open until its end tag.
    pub(crate) fn open(&mut self, name: &str, attributes: &[(String, String)]) -> Result<(), MarkdownError> {
        let kind = match name {
            _ if REJECTED.contains(&name) => return Err(rejected(name)),
            "br" => {
                self.push(Inline::HardBreak);
                return Ok(());
            }
            "img" => {
                if let Some(source) = Token::attribute(attributes, "src").filter(|source| !source.is_empty()) {
                    let alt = Token::attribute(attributes, "alt").unwrap_or_default();
                    self.push(Inline::Image {
                        url: ShareRef::from(source.to_owned()),
                        title: Token::attribute(attributes, "title").map(|title| ShareRef::from(title.to_owned())),
                        alt: ShareRef::from(alt.to_owned()),
                    });
                }
                return Ok(());
            }
            "b" | "strong" => InlineKind::Strong,
            "i" | "em" | "cite" | "dfn" | "var" => InlineKind::Emphasis,
            "s" | "del" | "strike" => InlineKind::Delete,
            "code" | "kbd" | "tt" | "samp" => InlineKind::Code,
            "a" => match Token::attribute(attributes, "href").filter(|url| !url.is_empty()) {
                Some(url) => InlineKind::Link {
                    url: ShareRef::from(url.to_owned()),
                    title: Token::attribute(attributes, "title").map(|title| ShareRef::from(title.to_owned())),
                },
                None => InlineKind::Plain,
            },
            _ => InlineKind::Plain,
        };
        self.frames.push(InlineFrame { tag: name.to_owned(), kind, children: Vec::new() });
        Ok(())
    }

    /// An end tag: it closes the tag it names and every tag that was opened after that one. One that names nothing that is open is passed over.
    pub(crate) fn close(&mut self, name: &str) {
        let Some(index) = self.frames.iter().rposition(|frame| frame.tag == name && !frame.tag.is_empty()) else {
            return;
        };
        while self.frames.len() > index {
            self.close_top();
        }
    }

    fn close_top(&mut self) {
        let frame = self.frames.pop().expect("a frame that is open is there");
        let children = frame.children;
        let inline = match frame.kind {
            InlineKind::Root => unreachable!("the root frame is never closed"),
            InlineKind::Plain => {
                self.top().extend(children);
                return;
            }
            _ if children.is_empty() => return,
            InlineKind::Strong => Inline::Strong(children),
            InlineKind::Emphasis => Inline::Emphasis(children),
            InlineKind::Delete => Inline::Delete(children),
            InlineKind::Code => {
                let mut code = String::new();
                plain_text(&children, &mut code);
                Inline::Code(ShareRef::from(code))
            }
            InlineKind::Link { url, title } => Inline::Link { url, title, content: children },
        };
        self.push(inline);
    }

    /// The inlines, with every tag that is still open closed.
    pub(crate) fn finish(mut self) -> Vec<Inline> {
        while self.frames.len() > 1 {
            self.close_top();
        }
        self.frames.pop().expect("the root frame is there").children
    }
}

/// What an open block tag makes of what is in it, when it is closed.
enum BlockKind {
    Root,
    /// A box of blocks: what is in it is as it would be outside it.
    Plain,
    Aligned(Alignment),
    Heading { depth: u8, alignment: Option<Alignment> },
    Details { open: bool, summary: Option<Vec<Inline>> },
    Summary,
    Quote,
    List { ordered: bool, start: Option<u32>, items: Vec<ListItem> },
    Item,
    Table { rows: Vec<TableRow>, alignments: Vec<Alignment> },
    Row { cells: Vec<Vec<Inline>>, alignments: Vec<Alignment> },
    Cell { alignment: Alignment },
    Pre { text: String, language: Option<String> },
}

impl BlockKind {
    /// Whether what is in it is inlines, which are its words, and not blocks: a heading, a summary and a cell.
    fn holds_words(&self) -> bool {
        matches!(self, Self::Heading { .. } | Self::Summary | Self::Cell { .. })
    }
}

struct BlockFrame {
    tag: &'static str,
    kind: BlockKind,
    /// The blocks that are in it. In a frame that is not for blocks, a list or a table, they are what was written where it cannot be:
    /// they go before it.
    blocks: Vec<Block>,
    inline: InlineBuilder,
}

impl BlockFrame {
    fn new(tag: &'static str, kind: BlockKind) -> Self {
        Self { tag, kind, blocks: Vec::new(), inline: InlineBuilder::new() }
    }
}

/// Collects the blocks of a document that has HTML among its Markdown, in the order it is given the HTML blocks and the blocks between
/// them, and keeps the tags that are open: the blocks between `<details>` and `</details>` are those of one [`Block::Details`].
pub(crate) struct BlockBuilder {
    /// The root frame, which is never closed, and the tags that are open above it.
    frames: Vec<BlockFrame>,
}

impl BlockBuilder {
    pub(crate) fn new() -> Self {
        Self { frames: vec![BlockFrame::new("", BlockKind::Root)] }
    }

    fn top(&mut self) -> &mut BlockFrame {
        self.frames.last_mut().expect("the root frame is never closed")
    }

    /// A block that Markdown made, which is among the HTML.
    pub(crate) fn block(&mut self, block: Block) {
        self.flush();
        self.emit(block);
    }

    /// A block of HTML.
    pub(crate) fn html(&mut self, source: &str) -> Result<(), MarkdownError> {
        for token in tokenize(source) {
            self.token(token)?;
        }
        Ok(())
    }

    /// The blocks, with every tag that is still open closed.
    pub(crate) fn finish(mut self) -> Vec<Block> {
        while self.frames.len() > 1 {
            self.pop();
        }
        self.flush();
        self.frames.pop().expect("the root frame is there").blocks
    }

    fn token(&mut self, token: Token) -> Result<(), MarkdownError> {
        // What is preformatted is its text, as it is written, and the tags in it say nothing but the language.
        if matches!(self.top().kind, BlockKind::Pre { .. }) {
            self.preformatted(token);
            return Ok(());
        }
        match token {
            Token::Text(text) => self.top().inline.text(&text),
            Token::Open { name, attributes, self_closing } => {
                self.open(&name, &attributes)?;
                if self_closing {
                    self.close(&name);
                }
            }
            Token::Close { name } => self.close(&name),
        }
        Ok(())
    }

    fn preformatted(&mut self, token: Token) {
        match token {
            Token::Text(text) => {
                if let BlockKind::Pre { text: kept, .. } = &mut self.top().kind {
                    kept.push_str(&text);
                }
            }
            Token::Open { name, attributes, .. } if name == "code" => {
                let language = Token::attribute(&attributes, "class")
                    .and_then(|class| class.split_whitespace().find_map(|class| class.strip_prefix("language-")))
                    .map(str::to_owned);
                if let BlockKind::Pre { language: kept, .. } = &mut self.top().kind {
                    *kept = kept.take().or(language);
                }
            }
            Token::Close { name } if name == "pre" => self.pop(),
            _ => {}
        }
    }

    fn open(&mut self, name: &str, attributes: &[(String, String)]) -> Result<(), MarkdownError> {
        if REJECTED.contains(&name) {
            return Err(rejected(name));
        }
        if let Some(tag) = BOXES.iter().find(|tag| **tag == name) {
            let kind = alignment_of(attributes).map_or(BlockKind::Plain, BlockKind::Aligned);
            self.push_frame(tag, kind);
            return Ok(());
        }
        match name {
            "center" => self.push_frame("center", BlockKind::Aligned(Alignment::Center)),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let depth = name.as_bytes()[1] - b'0';
                let tag = ["h1", "h2", "h3", "h4", "h5", "h6"][usize::from(depth) - 1];
                self.push_frame(tag, BlockKind::Heading { depth, alignment: alignment_of(attributes) });
            }
            "hr" => {
                self.flush();
                self.emit(Block::ThematicBreak);
            }
            "blockquote" => self.push_frame("blockquote", BlockKind::Quote),
            "ul" | "ol" => {
                let ordered = name == "ol";
                let start = Token::attribute(attributes, "start").and_then(|start| start.trim().parse().ok());
                self.push_frame(if ordered { "ol" } else { "ul" }, BlockKind::List { ordered, start, items: Vec::new() });
            }
            "li" => {
                if matches!(self.top().kind, BlockKind::Item) {
                    self.pop();
                }
                if matches!(self.top().kind, BlockKind::List { .. }) {
                    self.push_frame("li", BlockKind::Item);
                }
            }
            "details" => {
                let open = Token::attribute(attributes, "open").is_some();
                self.push_frame("details", BlockKind::Details { open, summary: None });
            }
            "summary" => {
                if matches!(self.top().kind, BlockKind::Details { .. }) {
                    self.push_frame("summary", BlockKind::Summary);
                }
            }
            "table" => self.push_frame("table", BlockKind::Table { rows: Vec::new(), alignments: Vec::new() }),
            "tr" => {
                self.close_cell();
                if matches!(self.top().kind, BlockKind::Row { .. }) {
                    self.pop();
                }
                if matches!(self.top().kind, BlockKind::Table { .. }) {
                    self.push_frame("tr", BlockKind::Row { cells: Vec::new(), alignments: Vec::new() });
                }
            }
            "td" | "th" => {
                self.close_cell();
                if matches!(self.top().kind, BlockKind::Table { .. }) {
                    self.push_frame("tr", BlockKind::Row { cells: Vec::new(), alignments: Vec::new() });
                }
                if matches!(self.top().kind, BlockKind::Row { .. }) {
                    self.push_frame("td", BlockKind::Cell { alignment: cell_alignment(attributes) });
                }
            }
            "pre" => self.push_frame("pre", BlockKind::Pre { text: String::new(), language: None }),
            _ if TABLE_PARTS.contains(&name) || name == "caption" => {}
            _ => self.top().inline.open(name, attributes)?,
        }
        Ok(())
    }

    fn close(&mut self, name: &str) {
        let tag = match name {
            "th" => "td",
            other => other,
        };
        if let Some(index) = self.frames.iter().skip(1).rposition(|frame| frame.tag == tag) {
            // `skip(1)` took the root off the count.
            while self.frames.len() > index + 1 {
                self.pop();
            }
        } else if !TABLE_PARTS.contains(&name) && name != "caption" {
            self.top().inline.close(name);
        }
    }

    /// Closes the cell that is open, if one is: a cell is ended by the start of the next, or of the row.
    fn close_cell(&mut self) {
        if matches!(self.top().kind, BlockKind::Cell { .. }) {
            self.pop();
        }
    }

    fn push_frame(&mut self, tag: &'static str, kind: BlockKind) {
        // What is before it is a paragraph of its own.
        self.flush();
        self.frames.push(BlockFrame::new(tag, kind));
    }

    /// Makes the words that are open in the frame on top a paragraph of its own: not in a frame that is for words.
    fn flush(&mut self) {
        let frame = self.top();
        if frame.kind.holds_words() {
            return;
        }
        let words = mem::replace(&mut frame.inline, InlineBuilder::new()).finish();
        let words = trimmed(words);
        if !words.is_empty() {
            frame.blocks.push(Block::Paragraph(words));
        }
    }

    /// Puts `block` where blocks go: in the frame nearest the top that is for blocks. A paragraph that is found where there are words is
    /// words that are added to them.
    fn emit(&mut self, block: Block) {
        let mut index = self.frames.len() - 1;
        loop {
            let frame = &mut self.frames[index];
            if frame.kind.holds_words() {
                if let Block::Paragraph(words) = block {
                    frame.inline.top().extend(words);
                    return;
                }
            } else if !matches!(frame.kind, BlockKind::Pre { .. }) {
                frame.blocks.push(block);
                return;
            }
            index -= 1;
        }
    }

    /// Closes the frame on top, and puts what it made where it goes.
    fn pop(&mut self) {
        self.flush();
        let frame = self.frames.pop().expect("a frame that is open is there");
        let words = trimmed(frame.inline.finish());
        let mut blocks = frame.blocks;
        match frame.kind {
            BlockKind::Root => unreachable!("the root frame is never closed"),
            BlockKind::Plain => blocks.drain(..).for_each(|block| self.emit(block)),
            BlockKind::Aligned(alignment) => {
                if !blocks.is_empty() {
                    self.emit(Block::Aligned { alignment, blocks });
                }
            }
            BlockKind::Heading { depth, alignment } => {
                blocks.drain(..).for_each(|block| self.emit(block));
                let heading = Block::Heading { depth, content: words };
                self.emit(match alignment {
                    Some(alignment) => Block::Aligned { alignment, blocks: vec![heading] },
                    None => heading,
                });
            }
            BlockKind::Details { open, summary } => {
                let summary = summary.unwrap_or_else(|| vec![Inline::Text(ShareRef::from_static("Details"))]);
                self.emit(Block::Details { summary, open, blocks });
            }
            BlockKind::Summary => {
                blocks.drain(..).for_each(|block| self.emit(block));
                if let BlockKind::Details { summary, .. } = &mut self.top().kind {
                    *summary = Some(words);
                }
            }
            BlockKind::Quote => self.emit(Block::Blockquote(blocks)),
            BlockKind::List { ordered, start, items } => {
                blocks.drain(..).for_each(|block| self.emit(block));
                if !items.is_empty() {
                    self.emit(Block::List { ordered, start, items });
                }
            }
            BlockKind::Item => {
                if let BlockKind::List { items, .. } = &mut self.top().kind {
                    items.push(ListItem { checked: None, blocks });
                } else {
                    blocks.drain(..).for_each(|block| self.emit(block));
                }
            }
            BlockKind::Table { rows, alignments } => {
                blocks.drain(..).for_each(|block| self.emit(block));
                if !rows.is_empty() {
                    self.emit(Block::Table { alignments, rows });
                }
            }
            BlockKind::Row { cells, alignments } => {
                blocks.drain(..).for_each(|block| self.emit(block));
                if let BlockKind::Table { rows, alignments: table } = &mut self.top().kind {
                    // A table is aligned as its first row is.
                    if rows.is_empty() {
                        *table = alignments;
                    }
                    rows.push(TableRow { cells });
                }
            }
            BlockKind::Cell { alignment } => {
                blocks.drain(..).for_each(|block| self.emit(block));
                if let BlockKind::Row { cells, alignments } = &mut self.top().kind {
                    cells.push(words);
                    alignments.push(alignment);
                }
            }
            BlockKind::Pre { text, language } => {
                self.emit(Block::Code { value: ShareRef::from(text), language: language.map(ShareRef::from), meta: None });
            }
        }
    }
}

#[cfg(test)]
mod tests;
