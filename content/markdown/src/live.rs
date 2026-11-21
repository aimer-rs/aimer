//! Live-preview styling of markdown source, for a text field.
//!
//! [`live_preview`] makes a [`TextHighlighter`] that renders the markdown a user
//! is typing inside a [`TextArea`](aimer_input::TextArea) the way a live-preview
//! editor does. A line the cursor is not on shows as rendered text: its markers
//! (`# `, `> `, `**`, a link's `[` and `](url)`) collapse to slivers and what they
//! mark is styled. The line the cursor is on shows its source.
//!
//! No line changes height when the cursor reaches it. Source and rendered text
//! use the same font sizes, so toggling a line only changes how wide its
//! characters are, and nothing below it moves.
//!
//! It reads the document with the same parser and options as
//! [`MarkdownViewer`](crate::MarkdownViewer), so what the editor shows as bold, a
//! list or a code block is what the viewer renders from the same text, nested
//! emphasis, `_` delimiters, autolinks, tables and setext headings included.
//! The public [`Document`](crate::Document) drops source positions, which a text
//! field needs, so this walks the parser's own tree.

use std::borrow::Cow;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use aimer_color::prelude::Color;
use aimer_input::input::{HighlightContext, TextEmbed, TextEmbedder, TextHighlighter};
use aimer_style::{FontWeight, TextDecoration, TextDecorationLine, TextDecorationStyle};
use aimer_text::{SpanStyle, StyledRange};
use markdown::mdast::{Blockquote, Heading, Node, Table};
use markdown::{Constructs, ParseOptions};

use crate::MarkdownTheme;
use aimer_text_buffer::TextSnapshot;

use crate::live_cuts::CutIndex;
use crate::live_segments::{Piece, SegmentCache, WINDOWED_BYTES, segment_cuts};

/// The parse of the last text a highlighter saw. The tree does not depend on
/// where the cursor is, so moving it between lines costs a walk and not a parse.
type Parsed = Rc<RefCell<Option<(Box<str>, Rc<Node>)>>>;

/// Makes a highlighter that renders markdown in place; see the
/// [module documentation](self). Shorthand for
/// [`LivePreview::new`]`(theme).`[`highlighter`](LivePreview::highlighter)`()`.
///
/// # Examples
///
/// ```
/// use aimer_input::TextArea;
/// use aimer_markdown::{MarkdownTheme, live_preview};
///
/// let notes = TextArea::new().highlighter(live_preview(&MarkdownTheme::default()));
/// ```
pub fn live_preview(theme: &MarkdownTheme) -> TextHighlighter {
    LivePreview::new(theme).highlighter()
}

/// Renders markdown source in place; see the [module documentation](self).
///
/// Take its look from a [`MarkdownTheme`]: heading sizes and weights, the quote
/// colour, the code font and background, link and inline-code styles, and the
/// accent used for list markers. The text field's own style stays the body style.
///
/// A `LivePreview` keeps the parse of the last text it saw, and clones share it,
/// so moving the cursor between lines walks the tree again and does not parse.
/// Make one and clone it into each build.
///
/// # Examples
///
/// ```
/// use aimer_markdown::{LivePreview, MarkdownTheme};
/// use aimer_input::input::HighlightContext;
///
/// let preview = LivePreview::new(&MarkdownTheme::default());
/// let text = "# Hello\nbody";
///
/// // With the cursor on the body, the heading's `# ` collapses.
/// let ranges = preview.ranges(text, &HighlightContext::new(text, 10..10));
/// assert!(ranges.iter().any(|range| range.range == (0..2)));
/// ```
#[derive(Clone)]
pub struct LivePreview {
    styles: Rc<Styles>,
    theme: MarkdownTheme,
    parsed: Parsed,
    /// What was read of each piece of the notes read lately; see [`crate::live_segments`].
    segments: Rc<RefCell<SegmentCache>>,
    /// Where the pieces of a very long note are near a place; see [`crate::live_cuts`].
    cuts: Rc<RefCell<CutIndex>>,
    title: Option<SpanStyle>,
    image_height: f32,
}

impl LivePreview {
    /// A live preview that styles itself from `theme`.
    pub fn new(theme: &MarkdownTheme) -> Self {
        Self {
            styles: Rc::new(Styles::new(theme)),
            theme: *theme,
            parsed: Parsed::default(),
            segments: Rc::new(RefCell::new(SegmentCache::new())),
            cuts: Rc::new(RefCell::new(CutIndex::new())),
            title: None,
            image_height: DEFAULT_IMAGE_HEIGHT,
        }
    }

    /// Treats the first line of the text as a title and styles it with `style`,
    /// unless markdown already makes it a heading.
    ///
    /// Spans inside the title keep what they are and add the title's look, and
    /// the title is styled whether or not the cursor is on it. A style that sets
    /// a font size must set the same one in both states, as it does here, to keep
    /// the line the same height.
    #[must_use]
    pub fn title(mut self, style: SpanStyle) -> Self {
        self.title = Some(style);
        self
    }

    /// Sets the height, in logical pixels, of the box a picture on a paragraph of
    /// its own is shown in. The default is 280.
    ///
    /// The box is fixed because a picture has no height until it has loaded, and a
    /// field measures its embeds before it lays the text out. The picture is fitted
    /// inside the box, keeping its proportions.
    #[must_use]
    pub fn image_height(mut self, height: f32) -> Self {
        self.image_height = height;
        self
    }

    /// The styled ranges of markdown `text` for a cursor described by `context`.
    ///
    /// The ranges never overlap. An unfocused field is a
    /// [`HighlightContext::default`], where nothing is active.
    pub fn ranges(&self, text: &str, context: &HighlightContext) -> Vec<StyledRange> {
        if let Some(cuts) = segment_cuts(text)
            && let Some(ranges) = self.ranges_in_pieces(text, context, &cuts)
        {
            return ranges;
        }
        self.ranges_whole(text, context)
    }

    /// [`ranges`](Self::ranges) from a parse of the whole text.
    fn ranges_whole(&self, text: &str, context: &HighlightContext) -> Vec<StyledRange> {
        let Some(root) = parse_cached(&self.parsed, text) else {
            return Vec::new();
        };
        let ranges = paint(text, context, &self.styles, &root);
        match self.title {
            Some(title) => with_title(ranges, text, title),
            None => ranges,
        }
    }

    /// The pieces `text` is cut into at `cuts`, each with what was read of it: the bytes it spans,
    /// and the reading, which is kept for as long as the piece is the same.
    fn pieces(&self, text: &str, cuts: &[usize]) -> Option<Vec<(Range<usize>, Rc<Piece>)>> {
        let mut start = 0;
        let spans: Vec<Range<usize>> = cuts
            .iter()
            .copied()
            .chain(std::iter::once(text.len()))
            .map(|end| {
                let span = start..end;
                start = end;
                span
            })
            .collect();
        self.pieces_of(&spans, &|span| Cow::Borrowed(&text[span.clone()]))
    }

    /// The reading of each of `spans`, whose text `text_of` gives.
    fn pieces_of<'a>(
        &self,
        spans: &[Range<usize>],
        text_of: &dyn Fn(&Range<usize>) -> Cow<'a, str>,
    ) -> Option<Vec<(Range<usize>, Rc<Piece>)>> {
        let mut cache = self.segments.borrow_mut();
        spans
            .iter()
            .map(|span| Some((span.clone(), cache.piece(&text_of(span), parse)?)))
            .collect()
    }

    /// [`ranges`](Self::ranges) from the pieces of the text, each read and painted apart.
    ///
    /// The ranges of a piece are what the whole text's would be for its bytes, because a piece
    /// is cut where nothing after it changes what is before. What the cursor is on is the part of
    /// the whole's that touches the piece.
    fn ranges_in_pieces(&self, text: &str, context: &HighlightContext, cuts: &[usize]) -> Option<Vec<StyledRange>> {
        let pieces = self.pieces(text, cuts)?;
        Some(self.paint_pieces(&pieces, &|span| Cow::Borrowed(&text[span.clone()]), context))
    }

    /// The ranges of `range` of a text held as a snapshot, which is read a window at a time when
    /// it is long: the pieces that hold `range` are found, read and painted, and no others. They
    /// are the ones [`ranges`](Self::ranges) would give for those bytes, except that a link
    /// reference or a footnote is found only in its own piece; see [`crate::live_cuts`].
    /// A short text is read whole.
    pub fn ranges_window(&self, text: &TextSnapshot, range: Range<usize>, context: &HighlightContext) -> Vec<StyledRange> {
        if text.len() < WINDOWED_BYTES {
            return self.ranges(&text.slice(0..text.len()), context);
        }
        let spans = {
            let mut cuts = self.cuts.borrow_mut();
            cuts.follow(text);
            cuts.pieces(range)
        };
        match self.pieces_of(&spans, &|span| text.slice(span.clone())) {
            Some(pieces) => self.paint_pieces(&pieces, &|span| text.slice(span.clone()), context),
            None => Vec::new(),
        }
    }

    /// Paints each piece for the cursor and gathers the ranges, in the offsets of the note.
    fn paint_pieces<'a>(
        &self,
        pieces: &[(Range<usize>, Rc<Piece>)],
        text_of: &dyn Fn(&Range<usize>) -> Cow<'a, str>,
        context: &HighlightContext,
    ) -> Vec<StyledRange> {
        let mut ranges = Vec::new();
        for (span, piece) in pieces {
            let local = context.shifted_back(span.start);
            let length = span.len();
            // The active lines that reach the piece, which is all the painting of it can tell of the cursor.
            let active = local
                .active_lines()
                .filter(|lines| lines.start <= length)
                .map(|lines| (lines.start, lines.end.min(length)));
            let piece_text = text_of(span);
            let painted = piece.ranges(active, |root| paint(&piece_text, &local, &self.styles, root));
            match (span.start, self.title) {
                // The title is the first line, which the first piece holds.
                (0, Some(title)) => {
                    ranges.extend(with_title(painted.to_vec(), &piece_text, title));
                }
                _ => ranges.extend(painted.iter().map(|range| {
                    StyledRange::new(range.range.start + span.start..range.range.end + span.start, range.style)
                })),
            }
        }
        ranges
    }

    /// A [`TextHighlighter`] for a text field, sharing this preview's parse.
    pub fn highlighter(&self) -> TextHighlighter {
        let preview = self.clone();
        TextHighlighter::windowed(move |text, range, context| preview.ranges_window(text, range, context))
    }

    /// The blocks of `text` that are shown as widgets instead of text: tables and
    /// fenced code blocks, while the cursor is not in them.
    ///
    /// Each is drawn by a [`MarkdownViewer`](crate::MarkdownViewer) over the
    /// block's own source, so it looks exactly as the viewer renders it, and it is
    /// measured to the width of the field. The cursor entering a block gives its
    /// source back for editing, as it does for the styled text. An embed covers
    /// whole lines, from the start of the block's first line.
    ///
    /// Its key is made from the block's source, with a count so two identical
    /// blocks are two widgets: editing a block builds a new widget for it, and
    /// leaves the others as they are.
    pub fn embeds(&self, text: &str, context: &HighlightContext) -> Vec<TextEmbed> {
        let mut seen: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
        let mut embeds = Vec::new();
        if let Some(cuts) = segment_cuts(text)
            && let Some(pieces) = self.pieces(text, &cuts)
        {
            for (span, piece) in &pieces {
                self.embeds_of(&text[span.clone()], span.start, &piece.root, context, &mut seen, &mut embeds);
            }
            return embeds;
        }
        self.embeds_whole(text, context)
    }

    /// The embeds of the pieces of a text held as a snapshot that hold `range`, each whole: the
    /// ones a field needs to lay out those bytes. A short text is read whole. Identical blocks of
    /// the pieces read are told apart by their order among them.
    pub fn embeds_window(&self, text: &TextSnapshot, range: Range<usize>, context: &HighlightContext) -> Vec<TextEmbed> {
        if text.len() < WINDOWED_BYTES {
            return self.embeds(&text.slice(0..text.len()), context);
        }
        let spans = {
            let mut cuts = self.cuts.borrow_mut();
            cuts.follow(text);
            cuts.pieces(range)
        };
        let Some(pieces) = self.pieces_of(&spans, &|span| text.slice(span.clone())) else {
            return Vec::new();
        };
        let mut seen: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
        let mut embeds = Vec::new();
        for (span, piece) in &pieces {
            self.embeds_of(&text.slice(span.clone()), span.start, &piece.root, context, &mut seen, &mut embeds);
        }
        embeds
    }

    /// [`embeds`](Self::embeds) from a parse of the whole text.
    fn embeds_whole(&self, text: &str, context: &HighlightContext) -> Vec<TextEmbed> {
        let Some(root) = parse_cached(&self.parsed, text) else {
            return Vec::new();
        };
        let mut embeds = Vec::new();
        self.embeds_of(text, 0, &root, context, &mut std::collections::HashMap::new(), &mut embeds);
        embeds
    }

    /// The embeds of one parse: `text` is what `root` was parsed from, `base` is where it starts
    /// in the note, and the embeds' ranges are the note's.
    fn embeds_of(
        &self,
        text: &str,
        base: usize,
        root: &Node,
        context: &HighlightContext,
        seen: &mut std::collections::HashMap<u64, u64>,
        embeds: &mut Vec<TextEmbed>,
    ) {
        let Node::Root(root) = root else {
            return;
        };
        for node in &root.children {
            let Some(range) = range_of(node) else {
                continue;
            };
            let picture = single_image(node);
            let embeddable = picture
                || match node {
                    Node::Table(_) => true,
                    Node::Code(_) => {
                        let source = text[range.clone()].trim_start();
                        source.starts_with("```") || source.starts_with("~~~")
                    }
                    _ => false,
                };
            if !embeddable {
                continue;
            }
            let start = text[..range.start].rfind('\n').map_or(0, |newline| newline + 1);
            let range = base + start..base + range.end;
            if context.is_active(&range) {
                continue;
            }
            let source: Rc<str> = Rc::from(&text[start..range.end - base]);
            let source_key = hash_of(&*source, 0);
            let occurrence = seen.entry(source_key).and_modify(|count| *count += 1).or_insert(0);
            let key = hash_of(&*source, *occurrence);
            let theme = self.theme;
            let image_height = self.image_height;
            let embed = TextEmbed::new(range, key, move || {
                crate::MarkdownViewer::new()
                    .markdown(source.to_string())
                    .theme(theme)
                    .scrollable(false)
                    .padding(aimer_style::LayoutSpacing::all(0_u32))
                    .image_resolver(move |image| picture_widget(image, image_height))
            });
            embeds.push(if picture { embed.fixed_height(self.image_height) } else { embed });
        }
    }

    /// A [`TextEmbedder`] for a text field, showing the blocks [`embeds`](Self::embeds)
    /// finds as widgets. Pair it with [`highlighter`](Self::highlighter).
    pub fn embedder(&self) -> TextEmbedder {
        let preview = self.clone();
        TextEmbedder::windowed(move |text, range, context| preview.embeds_window(text, range, context))
    }
}

/// The height of a picture's box when nothing else says.
const DEFAULT_IMAGE_HEIGHT: f32 = 280.0;

/// Whether `node` is a paragraph holding one picture and nothing else but
/// spaces: a picture on a line of its own, which is the one that can stand in for
/// a block.
fn single_image(node: &Node) -> bool {
    let Node::Paragraph(paragraph) = node else {
        return false;
    };
    let mut pictures = 0;
    for child in &paragraph.children {
        match child {
            Node::Image(_) | Node::ImageReference(_) => pictures += 1,
            Node::Text(text) if text.value.trim().is_empty() => {}
            _ => return false,
        }
    }
    pictures == 1
}

/// The widget for a picture in a box `height` tall: a file read from disk or a
/// web address, fitted inside the box with its proportions kept.
fn picture_widget(image: &crate::MarkdownImage, height: f32) -> aimer_widget::AnyWidget {
    use aimer_style::BoxFit;
    use aimer_widget::Widget;

    let source = image.source.to_string();
    if source.starts_with("http://") || source.starts_with("https://") {
        aimer_assets::NetworkImage::new(source).height(height).fit(BoxFit::Contain).boxed()
    } else {
        // A path on disk, as the picker hands it over, and not a bundled asset.
        let path = source.strip_prefix("file://").unwrap_or(&source).to_owned();
        aimer_assets::Image::new(path).height(height).fit(BoxFit::Contain).boxed()
    }
}

/// A key for `source`, told apart from the same source `occurrence` times over.
fn hash_of(source: &str, occurrence: u64) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    occurrence.hash(&mut hasher);
    hasher.finish()
}

/// Styles the first line as a title, under whatever `ranges` already style on
/// it, unless one of them sizes it, which is how a heading shows.
fn with_title(ranges: Vec<StyledRange>, text: &str, title: SpanStyle) -> Vec<StyledRange> {
    let line_end = text.find('\n').unwrap_or(text.len());
    let line = &text[..line_end];
    let lead = line.len() - line.trim_start().len();
    if lead >= line.len()
        || ranges
            .iter()
            .any(|range| range.range.start < line_end && range.style.font_size.is_some_and(|size| size > 1))
    {
        return ranges;
    }
    let mut out = Vec::with_capacity(ranges.len() + 2);
    let mut at = lead;
    for range in ranges {
        if range.range.start >= line_end {
            out.push(range);
            continue;
        }
        let (start, end) = (range.range.start.max(lead), range.range.end.min(line_end));
        if start > at {
            out.push(StyledRange::new(at..start, title));
        }
        if start < end {
            out.push(StyledRange::new(start..end, merged(title, range.style)));
            at = at.max(end);
        }
        if range.range.end > line_end {
            out.push(StyledRange::new(line_end..range.range.end, range.style));
        }
    }
    if at < line_end {
        out.push(StyledRange::new(at..line_end, title));
    }
    out.sort_by_key(|range| range.range.start);
    out
}

/// Parses with the options [`Document`](crate::Document) uses.
fn parse(text: &str) -> Option<Node> {
    let options = ParseOptions {
        constructs: Constructs::gfm(),
        gfm_strikethrough_single_tilde: false,
        ..ParseOptions::default()
    };
    markdown::to_mdast(text, &options).ok()
}

fn parse_cached(cache: &Parsed, text: &str) -> Option<Rc<Node>> {
    if let Some((cached, root)) = cache.borrow().as_ref()
        && **cached == *text
    {
        return Some(Rc::clone(root));
    }
    let root = Rc::new(parse(text)?);
    *cache.borrow_mut() = Some((text.into(), Rc::clone(&root)));
    Some(root)
}

/// The theme, turned into the span styles the painter lays down.
struct Styles {
    headings: [SpanStyle; 6],
    quote: SpanStyle,
    link: SpanStyle,
    inline_code: SpanStyle,
    code_line: SpanStyle,
    marker: SpanStyle,
    muted: SpanStyle,
    rule: SpanStyle,
}

impl Styles {
    fn new(theme: &MarkdownTheme) -> Self {
        // Headings change size and weight only, so a field keeps its own colours.
        let heading = |index: usize| {
            let style = &theme.headings[index];
            SpanStyle::new().font_size(style.font_size).font_weight(style.font_weight)
        };
        Self {
            headings: [heading(0), heading(1), heading(2), heading(3), heading(4), heading(5)],
            quote: merged(colored(theme.blockquote.color), italic()),
            link: theme.link,
            inline_code: theme.inline_code,
            // The block keeps the field's size: a line's height is the same in
            // the rendered and the source form.
            code_line: SpanStyle::new()
                .font_family(theme.code_block.font_family)
                .background_color(theme.code_background),
            marker: merged(colored(theme.icon_color), bold()),
            muted: merged(colored(theme.comment_color), decoration_style(TextDecorationLine::LINE_THROUGH)),
            rule: merged(colored(theme.rule_color), decoration_style(TextDecorationLine::LINE_THROUGH)),
        }
    }

    fn heading(&self, depth: u8) -> SpanStyle {
        self.headings[usize::from(depth.clamp(1, 6)) - 1]
    }
}

fn paint(text: &str, context: &HighlightContext, styles: &Styles, root: &Node) -> Vec<StyledRange> {
    let mut painter = Painter { text, context, styles, items: Vec::new() };
    painter.block(root, SpanStyle::new());
    flatten(painter.items)
}

/// Collects styled ranges that nest: a range either holds another or is
/// disjoint from it. [`flatten`] then keeps the innermost style of every byte.
struct Painter<'a> {
    text: &'a str,
    context: &'a HighlightContext,
    styles: &'a Styles,
    items: Vec<StyledRange>,
}

impl Painter<'_> {
    fn push(&mut self, range: Range<usize>, style: SpanStyle) {
        if range.start < range.end && range.end <= self.text.len() && !is_plain(&style) {
            self.items.push(StyledRange::new(range, style));
        }
    }

    fn conceal(&mut self, range: Range<usize>) {
        self.push(range, SpanStyle::concealed());
    }

    /// Whether the cursor is on a line `range` touches.
    fn active(&self, range: &Range<usize>) -> bool {
        self.context.is_active(range)
    }

    fn blocks(&mut self, nodes: &[Node], base: SpanStyle) {
        for node in nodes {
            self.block(node, base);
        }
    }

    fn block(&mut self, node: &Node, base: SpanStyle) {
        let Some(range) = range_of(node) else {
            return;
        };
        match node {
            Node::Root(root) => self.blocks(&root.children, base),
            Node::Heading(heading) => self.heading(heading, range, base),
            Node::Paragraph(paragraph) => {
                self.push(range, base);
                self.inlines(&paragraph.children, base);
            }
            Node::Blockquote(quote) => self.blockquote(quote, range, base),
            Node::List(list) => {
                for item in &list.children {
                    self.list_item(item, base);
                }
            }
            Node::Code(_) => self.code(range),
            Node::ThematicBreak(_) => {
                if !self.active(&range) {
                    self.push(range, self.styles.rule);
                }
            }
            Node::Table(table) => self.table(table, range),
            Node::FootnoteDefinition(note) => self.blocks(&note.children, base),
            _ => {}
        }
    }

    fn heading(&mut self, heading: &Heading, range: Range<usize>, base: SpanStyle) {
        let style = self.styles.heading(heading.depth);
        let size = SpanStyle { font_size: style.font_size, ..SpanStyle::new() };
        let (Some(first), Some(last)) = (
            heading.children.first().and_then(range_of),
            heading.children.last().and_then(range_of),
        ) else {
            // Nothing after the marker to collapse into, and collapsing it would
            // shrink the line.
            self.push(range, size);
            return;
        };
        if self.active(&range) {
            // Source keeps the heading's size, so the line is as tall as when it
            // is rendered.
            self.push(range, size);
            return;
        }
        self.push(range.clone(), merged(base, style));
        if self.text[range.start..].trim_start().starts_with('#') {
            self.conceal(range.start..first.start);
            let closing = last.end..range.end;
            if self.text[closing.clone()].contains('#') {
                self.conceal(closing);
            }
        } else {
            // A setext heading: the underline keeps the heading's size but
            // draws nothing, so the line stays as it is in the source form.
            let underline = self.text[..range.end].rfind('\n').map_or(0, |newline| newline + 1);
            if underline >= last.end {
                self.push(underline..range.end, merged(size, colored(HIDDEN)));
            }
        }
        self.inlines(&heading.children, merged(base, style));
    }

    fn blockquote(&mut self, quote: &Blockquote, range: Range<usize>, base: SpanStyle) {
        if self.active(&range) {
            self.blocks(&quote.children, base);
            return;
        }
        self.quote_markers(range);
        self.blocks(&quote.children, merged(base, self.styles.quote));
    }

    /// Collapses the `>` prefix of every line of a quote, nested ones included.
    fn quote_markers(&mut self, range: Range<usize>) {
        let bytes = self.text.as_bytes();
        let mut line_start = self.text[..range.start].rfind('\n').map_or(0, |newline| newline + 1);
        while line_start < range.end {
            let line_end = self.text[line_start..].find('\n').map_or(self.text.len(), |newline| line_start + newline);
            let mut at = line_start.max(range.start);
            while at < line_end && bytes[at] == b' ' && at < line_start + 3 {
                at += 1;
            }
            if bytes.get(at) == Some(&b'>') {
                let start = at;
                loop {
                    at += 1;
                    if bytes.get(at) == Some(&b' ') {
                        at += 1;
                    }
                    let mut next = at;
                    while bytes.get(next) == Some(&b' ') {
                        next += 1;
                    }
                    if bytes.get(next) == Some(&b'>') {
                        at = next;
                    } else {
                        break;
                    }
                }
                // A marker with nothing after it stays: collapsing it would
                // shrink the line.
                if at < line_end.min(range.end) {
                    self.conceal(start..at);
                }
            }
            line_start = line_end + 1;
        }
    }

    fn list_item(&mut self, node: &Node, base: SpanStyle) {
        let Node::ListItem(item) = node else {
            return;
        };
        let Some(range) = range_of(node) else {
            return;
        };
        let bytes = self.text.as_bytes();
        let line_start = self.text[..range.start].rfind('\n').map_or(0, |newline| newline + 1);
        let line_end = self.text[range.start..].find('\n').map_or(self.text.len(), |newline| range.start + newline);
        let mut content = base;
        if !self.active(&(line_start..line_end)) {
            let marker = marker_len(&self.text[range.start..line_end]);
            self.push(range.start..range.start + marker, self.styles.marker);
            if let Some(checked) = item.checked {
                let mut open = range.start + marker;
                while bytes.get(open) == Some(&b' ') {
                    open += 1;
                }
                if bytes.get(open) == Some(&b'[') && bytes.get(open + 2) == Some(&b']') {
                    self.push(open..open + 3, self.styles.marker);
                }
                if checked {
                    content = merged(base, self.styles.muted);
                }
            }
        }
        for child in &item.children {
            // The struck-through style belongs to the item's own text, not to
            // the lists nested under it.
            self.block(child, if matches!(child, Node::Paragraph(_)) { content } else { base });
        }
    }

    fn code(&mut self, range: Range<usize>) {
        let active = self.active(&range);
        let source = &self.text[range.clone()];
        let fence = ["```", "~~~"].into_iter().find(|fence| source.trim_start().starts_with(fence));
        let mut start = range.start;
        let lines: Vec<Range<usize>> = source
            .split('\n')
            .map(|line| {
                let line = start..start + line.len();
                start = line.end + 1;
                line
            })
            .collect();
        for (index, line) in lines.iter().enumerate() {
            let is_fence = fence.is_some_and(|fence| {
                index == 0 || (index + 1 == lines.len() && self.text[line.clone()].trim_start().starts_with(fence))
            });
            // The fence of a block the cursor is not in keeps its size and goes
            // transparent: the line stays as a blank gap above and below the
            // code instead of collapsing and reappearing.
            let style = if is_fence && !active { colored(HIDDEN) } else { self.styles.code_line };
            self.push(line.clone(), style);
        }
    }

    fn table(&mut self, table: &Table, range: Range<usize>) {
        if self.active(&range) {
            return;
        }
        for (index, row) in table.children.iter().enumerate() {
            let Node::TableRow(row) = row else {
                continue;
            };
            let base = if index == 0 { bold() } else { SpanStyle::new() };
            for cell in &row.children {
                let Some(children) = cell.children() else {
                    continue;
                };
                if let (Some(first), Some(last)) = (children.first().and_then(range_of), children.last().and_then(range_of)) {
                    self.push(first.start..last.end, base);
                }
                self.inlines(children, base);
            }
        }
        // The `|---|---|` row: hidden, with the height of its neighbours.
        let mut start = range.start;
        if let Some(delimiter) = self.text[range.clone()].split('\n').nth(1) {
            start += self.text[range.clone()].find('\n').map_or(0, |newline| newline + 1);
            self.push(start..start + delimiter.len(), colored(HIDDEN));
        }
    }

    fn inlines(&mut self, nodes: &[Node], base: SpanStyle) {
        for node in nodes {
            self.inline(node, base);
        }
    }

    fn inline(&mut self, node: &Node, base: SpanStyle) {
        let Some(range) = range_of(node) else {
            return;
        };
        match node {
            Node::Strong(strong) => self.span(&strong.children, range, base, bold()),
            Node::Emphasis(emphasis) => self.span(&emphasis.children, range, base, italic()),
            Node::Delete(delete) => self.span(&delete.children, range, base, decoration_style(TextDecorationLine::LINE_THROUGH)),
            Node::Link(link) => self.span(&link.children, range, base, self.styles.link),
            Node::LinkReference(link) => self.span(&link.children, range, base, self.styles.link),
            Node::InlineCode(_) => self.code_span(range, base),
            Node::Image(_) | Node::ImageReference(_) => {
                if !self.active(&range) {
                    self.push(range, merged(base, self.styles.link));
                }
            }
            _ => {}
        }
    }

    /// An inline element with markers around its children: `**strong**`,
    /// `*emphasis*`, `~~delete~~`, `[link](url)`.
    fn span(&mut self, children: &[Node], range: Range<usize>, base: SpanStyle, own: SpanStyle) {
        let active = self.active(&range);
        let style = if active { base } else { merged(base, own) };
        if !active {
            self.push(range.clone(), style);
            if let (Some(first), Some(last)) = (children.first().and_then(range_of), children.last().and_then(range_of)) {
                self.conceal(range.start..first.start);
                self.conceal(last.end..range.end);
            }
        }
        self.inlines(children, style);
    }

    fn code_span(&mut self, range: Range<usize>, base: SpanStyle) {
        if self.active(&range) {
            return;
        }
        let ticks = self.text[range.clone()].bytes().take_while(|&byte| byte == b'`').count();
        if range.len() <= ticks * 2 {
            return;
        }
        self.push(range.clone(), merged(base, self.styles.inline_code));
        self.conceal(range.start..range.start + ticks);
        self.conceal(range.end - ticks..range.end);
    }
}

/// Flattens nested ranges into disjoint ones, the innermost style winning every
/// byte. A range that crosses its parent's end is cut there.
fn flatten(mut items: Vec<StyledRange>) -> Vec<StyledRange> {
    // Outer first; of equal ranges the one pushed last, the inner one, ends up
    // on top because the sort is stable.
    items.sort_by(|a, b| a.range.start.cmp(&b.range.start).then(b.range.end.cmp(&a.range.end)));
    let mut out = Vec::with_capacity(items.len() * 2);
    let mut stack: Vec<(usize, SpanStyle)> = Vec::new();
    let mut at = 0;
    let mut emit = |out: &mut Vec<StyledRange>, from: usize, to: usize, style: SpanStyle| {
        if from < to {
            out.push(StyledRange::new(from..to, style));
        }
    };
    for item in items {
        let (start, end) = (item.range.start, item.range.end);
        while let Some(&(top_end, top_style)) = stack.last()
            && top_end <= start
        {
            emit(&mut out, at, top_end, top_style);
            at = at.max(top_end);
            stack.pop();
        }
        if let Some(&(_, top_style)) = stack.last() {
            emit(&mut out, at, start, top_style);
        }
        at = at.max(start);
        let end = stack.last().map_or(end, |&(top_end, _)| end.min(top_end));
        stack.push((end, item.style));
    }
    while let Some((top_end, top_style)) = stack.pop() {
        emit(&mut out, at, top_end, top_style);
        at = at.max(top_end);
    }
    out
}

fn range_of(node: &Node) -> Option<Range<usize>> {
    node.position().map(|position| position.start.offset..position.end.offset)
}

/// The length of the list marker (`-`, `*`, `+`, `12.`, `3)`) at the start of
/// `item`, not counting the space after it.
fn marker_len(item: &str) -> usize {
    let digits = item.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 { digits + 1 } else { 1 }
}

const HIDDEN: Color = Color::Rgba(0, 0, 0, 0);

fn colored(color: Color) -> SpanStyle {
    SpanStyle::new().color(color)
}

fn bold() -> SpanStyle {
    SpanStyle::new().font_weight(FontWeight::Bold)
}

fn italic() -> SpanStyle {
    decoration_style(TextDecorationLine::ITALIC)
}

fn decoration_style(line: TextDecorationLine) -> SpanStyle {
    SpanStyle::new().text_decoration(TextDecoration::from_parts(line, TextDecorationStyle::Solid))
}

/// Whether `style` sets nothing.
fn is_plain(style: &SpanStyle) -> bool {
    style.font_size.is_none()
        && style.font_family.is_none()
        && style.font_style.is_none()
        && style.font_weight.is_none()
        && style.color.is_none()
        && style.background_color.is_none()
        && style.background_radius.is_none()
        && style.text_decoration.is_none()
        && style.text_transform.is_none()
        && style.letter_spacing.is_none()
        && style.word_spacing.is_none()
        && style.text_shadow.is_none()
        && style.inline_box.is_none()
}

/// `over` on top of `base`: what `over` sets wins, and decoration lines add up,
/// so an italic quote can hold an underlined link.
fn merged(base: SpanStyle, over: SpanStyle) -> SpanStyle {
    let text_decoration = match (base.text_decoration, over.text_decoration) {
        (Some(base), Some(over)) => Some(TextDecoration { line: base.line | over.line, ..over }),
        (base, over) => over.or(base),
    };
    SpanStyle {
        font_size: over.font_size.or(base.font_size),
        font_family: over.font_family.or(base.font_family),
        font_style: over.font_style.or(base.font_style),
        font_weight: over.font_weight.or(base.font_weight),
        color: over.color.or(base.color),
        background_color: over.background_color.or(base.background_color),
        background_radius: over.background_radius.or(base.background_radius),
        text_decoration,
        text_transform: over.text_transform.or(base.text_transform),
        letter_spacing: over.letter_spacing.or(base.letter_spacing),
        word_spacing: over.word_spacing.or(base.word_spacing),
        text_shadow: over.text_shadow.or(base.text_shadow),
        inline_box: over.inline_box.or(base.inline_box),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The size text has when nothing sets one.
    const BODY: u32 = 16;

    fn theme() -> MarkdownTheme {
        MarkdownTheme::default()
    }

    fn with_cursor(text: &str, cursor: usize) -> Vec<StyledRange> {
        LivePreview::new(&theme()).ranges(text, &HighlightContext::new(text, cursor..cursor))
    }

    fn unfocused(text: &str) -> Vec<StyledRange> {
        LivePreview::new(&theme()).ranges(text, &HighlightContext::default())
    }

    /// The style `text[at]` gets, if any range holds it.
    fn style_at(styled: &[StyledRange], at: usize) -> Option<SpanStyle> {
        styled.iter().find(|range| range.range.contains(&at)).map(|range| range.style)
    }

    fn is_concealed(styled: &[StyledRange], at: usize) -> bool {
        style_at(styled, at).is_some_and(|style| style.font_size == Some(1))
    }

    fn offset(text: &str, needle: &str) -> usize {
        text.find(needle).unwrap_or_else(|| panic!("{needle:?} is not in {text:?}"))
    }

    fn has_decoration(style: SpanStyle, line: TextDecorationLine) -> bool {
        style.text_decoration.is_some_and(|decoration| decoration.line.contains(line))
    }

    #[test]
    fn a_heading_takes_its_size_with_its_marker_collapsed_while_the_cursor_is_elsewhere() {
        let text = "# Hello\nbody";
        let styled = with_cursor(text, text.len());

        assert!(is_concealed(&styled, 0));
        assert!(is_concealed(&styled, 1));
        let hello = style_at(&styled, offset(text, "Hello")).expect("the heading text is styled");
        assert_eq!(hello.font_size, theme().headings[0].font_size.into());
        assert!(hello.font_weight.is_some());
    }

    #[test]
    fn a_heading_under_the_cursor_keeps_its_size_and_shows_its_source() {
        let text = "# Hello\nbody";
        let styled = with_cursor(text, 3);

        for at in [0, offset(text, "Hello")] {
            let style = style_at(&styled, at).expect("the whole line is sized");
            assert_eq!(style.font_size, Some(theme().headings[0].font_size), "at {at}");
            assert!(style.font_weight.is_none(), "the source is plain weight");
        }
        assert!(!is_concealed(&styled, 0));
    }

    #[test]
    fn lower_levels_take_the_size_of_their_level() {
        let text = "### Third\nbody";
        let styled = with_cursor(text, text.len());

        assert_eq!(style_at(&styled, offset(text, "Third")).unwrap().font_size, Some(theme().headings[2].font_size));
    }

    #[test]
    fn a_heading_marker_with_nothing_after_it_is_not_collapsed() {
        let text = "# \nbody";
        let styled = with_cursor(text, text.len());

        assert!(!is_concealed(&styled, 0));
    }

    #[test]
    fn a_closing_sequence_collapses_with_the_opening_one() {
        let text = "## Title ##\nbody";
        let styled = with_cursor(text, text.len());

        assert!(is_concealed(&styled, 0));
        assert!(is_concealed(&styled, offset(text, "##\n")));
        assert!(!is_concealed(&styled, offset(text, "Title")));
    }

    #[test]
    fn a_setext_heading_keeps_the_height_of_its_underline() {
        let text = "Title\n=====\nbody";
        let away = with_cursor(text, text.len());
        let on_it = with_cursor(text, 2);

        let size = theme().headings[0].font_size;
        assert_eq!(style_at(&away, 1).unwrap().font_size, Some(size));
        let underline = style_at(&away, offset(text, "=====")).unwrap();
        assert_eq!(underline.font_size, Some(size), "the hidden underline keeps its size");
        assert!(underline.color.is_some(), "and draws nothing");
        assert_eq!(style_at(&on_it, offset(text, "=====")).unwrap().font_size, Some(size));
        assert!(style_at(&on_it, offset(text, "=====")).unwrap().color.is_none());
    }

    #[test]
    fn inline_markers_collapse_and_their_text_is_styled() {
        let text = "title\nsome **bold** and *it* and ~~old~~ and `code` end";
        let styled = with_cursor(text, 0);
        assert!(is_concealed(&styled, offset(text, "**bold")));
        assert!(style_at(&styled, offset(text, "bold")).unwrap().font_weight.is_some());
        assert!(is_concealed(&styled, offset(text, "*it")));
        assert!(has_decoration(style_at(&styled, offset(text, "it*")).unwrap(), TextDecorationLine::ITALIC));
        assert!(is_concealed(&styled, offset(text, "~~old")));
        assert!(has_decoration(style_at(&styled, offset(text, "old~~")).unwrap(), TextDecorationLine::LINE_THROUGH));
        assert!(is_concealed(&styled, offset(text, "`code")));
        assert!(style_at(&styled, offset(text, "code`")).unwrap().background_color.is_some());
        assert!(style_at(&styled, offset(text, "some")).is_none(), "plain words stay unstyled");
    }

    #[test]
    fn emphasis_nests_and_underscores_work() {
        let text = "title\nA **bold *both* more** and _em_ and __st__";
        let styled = with_cursor(text, 0);

        let both = style_at(&styled, offset(text, "both")).unwrap();
        assert!(both.font_weight.is_some(), "the inner span keeps the outer bold");
        assert!(has_decoration(both, TextDecorationLine::ITALIC), "and adds italic");
        assert!(is_concealed(&styled, offset(text, "*both")));
        assert!(is_concealed(&styled, offset(text, "_em")));
        assert!(has_decoration(style_at(&styled, offset(text, "em_")).unwrap(), TextDecorationLine::ITALIC));
        assert!(is_concealed(&styled, offset(text, "__st")));
        assert!(style_at(&styled, offset(text, "st__")).unwrap().font_weight.is_some());
    }

    #[test]
    fn a_code_span_may_hold_backticks_and_markdown() {
        let text = "title\nuse ``a`b **c**`` here";
        let styled = with_cursor(text, 0);

        assert!(is_concealed(&styled, offset(text, "``a")));
        assert!(is_concealed(&styled, offset(text, "``")) );
        assert!(style_at(&styled, offset(text, "a`b")).unwrap().background_color.is_some());
        assert!(!is_concealed(&styled, offset(text, "**c")), "markdown inside a code span is text");
    }

    #[test]
    fn the_line_under_the_cursor_shows_its_inline_source() {
        let text = "title\nsome **bold** and `code`";
        let styled = with_cursor(text, offset(text, "bold"));

        assert!(style_at(&styled, offset(text, "**bold")).is_none());
        assert!(style_at(&styled, offset(text, "`code")).is_none());
    }

    #[test]
    fn a_link_shows_its_label_and_hides_its_address() {
        let text = "title\nsee [the docs](https://example.com/x \"t\") now";
        let styled = with_cursor(text, 0);

        assert!(is_concealed(&styled, offset(text, "[the")));
        let label = style_at(&styled, offset(text, "the docs")).unwrap();
        assert!(label.color.is_some() && label.text_decoration.is_some());
        assert!(is_concealed(&styled, offset(text, "](https")));
        assert!(is_concealed(&styled, offset(text, "\"t\")")));
        assert!(style_at(&styled, offset(text, " now")).is_none());
    }

    #[test]
    fn a_bare_address_is_a_link_with_nothing_to_hide() {
        let text = "title\nsee https://example.com end";
        let styled = with_cursor(text, 0);

        let link = style_at(&styled, offset(text, "https")).unwrap();
        assert!(link.color.is_some());
        assert!(!is_concealed(&styled, offset(text, "https")));
    }

    #[test]
    fn delimiters_the_parser_leaves_as_text_are_not_collapsed() {
        for text in ["title\n2 * 3 = 6", "title\nan **unclosed bold", "title\nlone `tick", "title\n[not a link] (x)", "title\n** not bold **", "title\nsnake_case_name"] {
            let styled = with_cursor(text, 0);
            assert!(
                styled.iter().all(|range| range.style.font_size != Some(1)),
                "nothing collapses in {text:?}: {:?}",
                styled.iter().map(|range| range.range.clone()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_quote_collapses_the_marker_of_every_line_and_mutes_its_text() {
        let text = "title\n> quoted **strong**\n> second line\n>\n> > nested\nlazy?";
        let styled = with_cursor(text, 0);

        assert!(is_concealed(&styled, offset(text, "> quoted")));
        assert!(is_concealed(&styled, offset(text, "> second")));
        assert!(is_concealed(&styled, offset(text, "> > nested")));
        assert!(is_concealed(&styled, offset(text, "> nested")));
        assert!(!is_concealed(&styled, offset(text, ">\n")), "an empty quote line keeps its marker");
        let quoted = style_at(&styled, offset(text, "quoted")).unwrap();
        assert!(quoted.color.is_some() && has_decoration(quoted, TextDecorationLine::ITALIC));
        let strong = style_at(&styled, offset(text, "strong")).unwrap();
        assert!(strong.font_weight.is_some());
        assert!(strong.color.is_some(), "inline spans inherit the quote");
    }

    #[test]
    fn a_quote_the_cursor_is_in_shows_its_source() {
        let text = "title\n> one\n> two";
        let styled = with_cursor(text, offset(text, "two"));

        assert!(!is_concealed(&styled, offset(text, "> one")));
        assert!(!is_concealed(&styled, offset(text, "> two")));
    }

    #[test]
    fn list_markers_are_coloured_and_their_text_stays_plain() {
        let text = "title\n- one\n* two\n12. three\n    - nested";
        let styled = with_cursor(text, 0);

        for marker in ["- one", "* two", "12. three", "- nested"] {
            let style = style_at(&styled, offset(text, marker)).unwrap_or_else(|| panic!("{marker} is styled"));
            assert!(style.color.is_some() && style.font_weight.is_some(), "{marker}");
        }
        assert!(style_at(&styled, offset(text, "one")).is_none());
        assert!(style_at(&styled, offset(text, "nested")).is_none());
        assert!(style_at(&styled, offset(text, "12.") + 2).is_some(), "the whole of `12.` is the marker");
    }

    #[test]
    fn a_checked_task_is_struck_through_and_an_open_one_is_not() {
        let text = "title\n- [x] done\n- [ ] todo\n  - [x] inner\n  - sub";
        let styled = with_cursor(text, 0);

        let done = style_at(&styled, offset(text, "done")).unwrap();
        assert!(has_decoration(done, TextDecorationLine::LINE_THROUGH));
        assert!(style_at(&styled, offset(text, "todo")).is_none());
        assert!(style_at(&styled, offset(text, "[x]")).unwrap().color.is_some());
        assert!(has_decoration(style_at(&styled, offset(text, "inner")).unwrap(), TextDecorationLine::LINE_THROUGH));
        assert!(style_at(&styled, offset(text, "sub")).is_none(), "the strike does not leak to siblings");
    }

    #[test]
    fn a_code_block_is_monospace_and_hides_its_fences_until_the_cursor_enters() {
        let text = "title\n```rust\nlet x = 1;\n```\nafter";
        let away = with_cursor(text, text.len());

        assert!(style_at(&away, offset(text, "let")).unwrap().font_family.is_some());
        let fence = style_at(&away, offset(text, "```rust")).unwrap();
        assert!(fence.color.is_some(), "a hidden fence is transparent");
        assert!(fence.font_size.is_none(), "and keeps its size, so the line keeps its height");

        for cursor in [offset(text, "```rust"), offset(text, "let"), text.rfind("```").unwrap()] {
            let inside = with_cursor(text, cursor);
            let fence = style_at(&inside, offset(text, "```rust")).unwrap();
            assert!(fence.color.is_none(), "the fence shows while the cursor is in the block (cursor {cursor})");
        }
    }

    #[test]
    fn markdown_inside_a_code_block_is_left_alone() {
        let text = "title\n```\n# not a heading **nor bold**\n```";
        let styled = with_cursor(text, 0);

        let line = style_at(&styled, offset(text, "# not")).unwrap();
        assert!(line.font_size.is_none() && line.font_weight.is_none());
        assert!(!is_concealed(&styled, offset(text, "**nor")));
    }

    #[test]
    fn an_unclosed_fence_runs_to_the_end_and_an_indented_block_is_code() {
        let text = "title\n```\ncode one\ncode two";
        assert!(style_at(&with_cursor(text, 0), offset(text, "code two")).unwrap().font_family.is_some());

        let text = "title\n\n    indented code\n\nafter";
        assert!(style_at(&with_cursor(text, text.len()), offset(text, "indented")).unwrap().font_family.is_some());
    }

    #[test]
    fn a_rule_is_struck_through_until_the_cursor_reaches_it() {
        let text = "title\n\n---\n\nafter";
        assert!(has_decoration(style_at(&with_cursor(text, text.len()), offset(text, "---")).unwrap(), TextDecorationLine::LINE_THROUGH));
        assert!(style_at(&with_cursor(text, offset(text, "---")), offset(text, "---")).is_none());
    }

    #[test]
    fn a_table_is_a_bold_header_over_a_hidden_delimiter_row() {
        let text = "title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nafter";
        let away = with_cursor(text, text.len());

        assert!(style_at(&away, offset(text, "a |")).unwrap().font_weight.is_some());
        assert!(style_at(&away, offset(text, "1 |")).is_none());
        assert!(style_at(&away, offset(text, "|---")).unwrap().color.is_some());
        let inside = with_cursor(text, offset(text, "| 1"));
        assert!(style_at(&inside, offset(text, "|---")).is_none(), "the table the cursor is in shows its source");
    }

    #[test]
    fn an_unfocused_field_shows_everything_rendered() {
        let text = "# Hello\nsome **bold**";
        let styled = unfocused(text);

        assert!(is_concealed(&styled, 0));
        assert!(is_concealed(&styled, offset(text, "**bold")));
    }

    #[test]
    fn offsets_are_bytes_so_non_ascii_text_lines_up() {
        let text = "é title\nÜnï **bøld** `cödé`\n> *itälic*\n- [x] dønë";
        let styled = with_cursor(text, 0);

        assert!(is_concealed(&styled, offset(text, "**bøld")));
        assert!(style_at(&styled, offset(text, "bøld")).unwrap().font_weight.is_some());
        assert!(is_concealed(&styled, offset(text, "`cödé")));
    }

    #[test]
    fn ranges_never_overlap_and_never_split_a_character() {
        let text = "Héllo wörld\n# Ünï **bøld *both*** `cödé` [lïnk](https://ex.com)\n> *itälic* ~~strïke~~\n- [x] dønë **b**\n```\nçode\n```\n---\n| ä | b |\n|---|---|\n| 1 | 2 |\nend";
        for cursor in (0..=text.len()).filter(|&at| text.is_char_boundary(at)) {
            let styled = with_cursor(text, cursor);
            for pair in styled.windows(2) {
                assert!(pair[0].range.end <= pair[1].range.start, "{:?} overlaps {:?} (cursor {cursor})", pair[0].range, pair[1].range);
            }
            for range in &styled {
                assert!(text.is_char_boundary(range.range.start) && text.is_char_boundary(range.range.end), "{:?} cuts a character", range.range);
                assert!(range.range.start < range.range.end && range.range.end <= text.len());
            }
        }
    }

    #[test]
    fn no_line_changes_height_when_the_cursor_moves() {
        let text = "# Big heading\n## Smaller **b**\nSetext\n======\n> quote *i*\n- [x] done `c`\nsee [x](y)\n```\ncode\n```\n\n# \n| a | b |\n|---|---|\n| 1 | 2 |\n---\nplain";
        let mut start = 0;
        let lines: Vec<Range<usize>> = text
            .split('\n')
            .map(|line| {
                let range = start..start + line.len();
                start = range.end + 1;
                range
            })
            .collect();
        // A collapsed sliver adds nothing: the line is as tall as its tallest
        // ordinary text.
        let heights = |styled: &[StyledRange]| {
            lines
                .iter()
                .map(|line| {
                    line.clone()
                        .filter_map(|at| style_at(styled, at))
                        .filter_map(|style| style.font_size)
                        .filter(|&size| size > 1)
                        .max()
                        .unwrap_or(BODY)
                })
                .collect::<Vec<_>>()
        };
        let rendered = heights(&unfocused(text));

        for line in &lines {
            assert_eq!(heights(&with_cursor(text, line.start)), rendered, "the cursor on {:?} reflowed the lines", &text[line.clone()]);
        }
    }

    fn embeds_at(text: &str, cursor: usize) -> Vec<aimer_input::input::TextEmbed> {
        LivePreview::new(&theme()).embeds(text, &HighlightContext::new(text, cursor..cursor))
    }

    const NOTE: &str = "title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\ntext\n\n```rust\nlet x = 1;\n```\nend";

    #[test]
    fn a_table_and_a_fenced_block_become_embeds_while_the_cursor_is_elsewhere() {
        let embeds = embeds_at(NOTE, NOTE.len());

        let table = offset(NOTE, "| a");
        let fence = offset(NOTE, "```rust");
        let ranges: Vec<_> = embeds.iter().map(|embed| embed.range()).collect();
        assert_eq!(ranges, [table..offset(NOTE, "\n\ntext"), fence..fence + "```rust\nlet x = 1;\n```".len()]);
        assert_ne!(embeds[0].key(), embeds[1].key());
    }

    #[test]
    fn the_block_the_cursor_is_in_shows_its_source_and_the_other_stays_embedded() {
        let in_table = embeds_at(NOTE, offset(NOTE, "| 1"));
        let in_code = embeds_at(NOTE, offset(NOTE, "let x"));

        assert_eq!(in_table.len(), 1);
        assert_eq!(in_table[0].range().start, offset(NOTE, "```rust"), "only the code block remains");
        assert_eq!(in_code.len(), 1);
        assert_eq!(in_code[0].range().start, offset(NOTE, "| a"), "only the table remains");
    }

    #[test]
    fn an_unfocused_field_embeds_every_block() {
        let embeds = LivePreview::new(&theme()).embeds(NOTE, &HighlightContext::default());

        assert_eq!(embeds.len(), 2);
    }

    #[test]
    fn the_key_follows_what_is_shown_and_identical_blocks_are_distinct() {
        let twice = "title\n\n```\nsame\n```\n\n```\nsame\n```\nend";
        let keys: Vec<_> = embeds_at(twice, twice.len()).iter().map(|embed| embed.key()).collect();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1], "two blocks of the same source need two widgets");

        let again: Vec<_> = embeds_at(twice, twice.len()).iter().map(|embed| embed.key()).collect();
        assert_eq!(keys, again, "the same text gives the same keys, so the widgets are kept");

        let edited = twice.replace("same\n```\nend", "other\n```\nend");
        let after: Vec<_> = embeds_at(&edited, edited.len()).iter().map(|embed| embed.key()).collect();
        assert_eq!(after[0], keys[0], "an untouched block keeps its widget");
        assert_ne!(after[1], keys[1], "an edited one gets a new one");
    }

    #[test]
    fn an_indented_block_is_embedded_from_the_start_of_its_line() {
        let text = "title\n\n  ```\n  code\n  ```\nend";
        let embeds = embeds_at(text, text.len());

        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0].range().start, offset(text, "  ```"), "the indent is part of the box, not text before it");
    }

    #[test]
    fn only_tables_and_fenced_code_are_embedded() {
        let text = "title\n\n# Heading\n\n> quote\n\n- item\n\n    indented code\n\n---\n\nplain\nend";

        assert!(embeds_at(text, text.len()).is_empty());
    }

    const PICTURE: &str = "title\n\n![a cat](/tmp/cat.png)\n\nend";

    #[test]
    fn an_image_alone_in_its_paragraph_is_a_fixed_height_embed() {
        use aimer_input::input::EmbedHeight;

        let embeds = embeds_at(PICTURE, PICTURE.len());
        let image = offset(PICTURE, "![a cat]");

        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0].range(), image..image + "![a cat](/tmp/cat.png)".len());
        assert_eq!(embeds[0].height(), EmbedHeight::Fixed(280.0), "an image has no height until it loads, so the box is fixed");

        let short = LivePreview::new(&theme()).image_height(150.0).embeds(PICTURE, &HighlightContext::default());
        assert_eq!(short[0].height(), EmbedHeight::Fixed(150.0));
    }

    #[test]
    fn an_image_with_a_web_address_is_embedded_too() {
        let text = "title\n\n![logo](https://example.com/logo.png)\n\nend";

        assert_eq!(embeds_at(text, text.len()).len(), 1);
    }

    #[test]
    fn an_image_among_words_stays_text() {
        for text in ["title\n\nsee ![a](b.png) here\n\nend", "title\n\n![a](b.png) caption\n\nend", "title\n\n![a](b.png)\nnext line\n\nend"] {
            assert!(embeds_at(text, text.len()).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn the_cursor_on_an_image_line_shows_its_markdown() {
        assert!(embeds_at(PICTURE, offset(PICTURE, "cat")).is_empty());
    }

    #[test]
    fn two_pictures_are_two_widgets() {
        let text = "title\n\n![a](x.png)\n\n![a](x.png)\n\nend";
        let keys: Vec<_> = embeds_at(text, text.len()).iter().map(|embed| embed.key()).collect();

        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1]);
    }

    #[test]
    fn the_embedder_is_the_one_a_text_area_takes() {
        let embedder = LivePreview::new(&theme()).embedder();

        assert_eq!(embedder.embeds(NOTE, &HighlightContext::default()).len(), 2);
    }

    fn titled(text: &str, cursor: usize) -> Vec<StyledRange> {
        let title = SpanStyle::new().font_size(24).font_weight(FontWeight::Bold);
        LivePreview::new(&theme()).title(title).ranges(text, &HighlightContext::new(text, cursor..cursor))
    }

    #[test]
    fn the_first_line_is_a_title_wherever_the_cursor_is() {
        let text = "Plain words\nbody";
        for cursor in [0, 5, text.len()] {
            let style = style_at(&titled(text, cursor), 3).expect("the title is styled");
            assert_eq!(style.font_size, Some(24), "cursor at {cursor}");
            assert!(style.font_weight.is_some());
        }
        assert!(style_at(&titled(text, 0), offset(text, "body")).is_none(), "and only the first line");
    }

    #[test]
    fn a_title_stops_at_the_end_of_its_line_even_when_a_paragraph_goes_on() {
        let text = "Title line\nsame paragraph **bold** here";
        let styled = titled(text, 0);

        assert_eq!(style_at(&styled, 2).unwrap().font_size, Some(24));
        assert!(style_at(&styled, offset(text, "same")).is_none());
        assert!(style_at(&styled, offset(text, "bold")).unwrap().font_size.is_none());
    }

    #[test]
    fn spans_inside_a_title_keep_the_title_size() {
        let text = "A **big** `title`\nbody";
        let styled = titled(text, text.len());

        let bold = style_at(&styled, offset(text, "big")).unwrap();
        assert_eq!(bold.font_size, Some(24), "bold inside the title is still title-sized");
        assert!(bold.font_weight.is_some());
        assert!(is_concealed(&styled, offset(text, "**big")), "markers inside it still collapse");
        assert_eq!(style_at(&styled, offset(text, "A")).unwrap().font_size, Some(24));
    }

    #[test]
    fn a_first_line_that_is_a_heading_takes_the_heading_and_not_the_title() {
        let text = "### Third\nbody";
        let styled = titled(text, text.len());

        assert_eq!(style_at(&styled, offset(text, "Third")).unwrap().font_size, Some(theme().headings[2].font_size));
    }

    #[test]
    fn a_blank_first_line_has_no_title_and_the_ranges_stay_flat() {
        let text = "\nafter **x** and\n# H";
        for cursor in 0..=text.len() {
            let styled = titled(text, cursor);
            assert!(style_at(&styled, 0).is_none());
            let mut sorted = styled.clone();
            sorted.sort_by_key(|range| range.range.start);
            assert!(sorted.windows(2).all(|pair| pair[0].range.end <= pair[1].range.start));
        }
    }

    #[test]
    fn the_tree_is_parsed_once_for_a_text_however_the_cursor_moves() {
        let cache = Parsed::default();

        let first = parse_cached(&cache, "# a\nb").unwrap();
        let again = parse_cached(&cache, "# a\nb").unwrap();
        let other = parse_cached(&cache, "# a\nbc").unwrap();

        assert!(Rc::ptr_eq(&first, &again));
        assert!(!Rc::ptr_eq(&first, &other));
    }

    #[test]
    fn the_highlighter_follows_the_cursor_through_a_field() {
        let highlighter = live_preview(&theme());
        let text = "title\n# Hello";

        // Unfocused: the heading is rendered.
        assert!(highlighter.highlight(text).iter().any(|range| range.style.font_size == Some(1)));
    }

    // ── Reading a note in pieces ─────────────────────────────────────────────────

    /// A small deterministic generator, so the tests are the same on every run.
    struct Random(u64);
    impl Random {
        fn next(&mut self, below: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % below as u64) as usize
        }
    }

    /// One block of markdown, and the blank lines that follow it. The blocks are the ones that
    /// make cutting hard: lists with blank lines in them, fences that hold blank lines, HTML
    /// that a closing tag ends, indented lines after a blank line, and the rest.
    fn block(random: &mut Random, index: usize) -> String {
        let words = |count: usize| (0..count).map(|n| format!("w{}x{n}", random_word(index, n))).collect::<Vec<_>>().join(" ");
        let long = |random: &mut Random, count: usize| words(count + random.next(20));
        let body = match random.next(16) {
            0 => format!("# Heading {index}\n"),
            1 => format!("## Section {index} {}\n", long(random, 4)),
            2 => format!("{} **bold {index}** and `code` and [a link](https://example.com/{index}) and _em_.\n", long(random, 60)),
            3 => format!("- one {}\n- two {}\n  - nested {}\n", long(random, 8), long(random, 8), long(random, 8)),
            4 => format!("- loose {}\n\n- items {}\n\n  continued {}\n\n- end {}\n", long(random, 20), long(random, 20), long(random, 20), long(random, 20)),
            5 => format!("1. first {}\n\n2. second {}\n\n3. third {}\n", long(random, 20), long(random, 20), long(random, 20)),
            6 => format!("```rust\nfn demo_{index}() {{\n    let a = {index};\n\n    a + 1 // {}\n}}\n```\n", long(random, 6)),
            7 => format!("~~~\nplain {}\n\n\nafter two blanks\n~~~\n", long(random, 10)),
            8 => format!("| a | b |\n|---|---|\n| {index} | {} |\n| x | y |\n", long(random, 3)),
            9 => format!("> quoted {}\n> more {}\n", long(random, 20), long(random, 20)),
            10 => format!("<!--\nA comment {}\n\nwith a blank line in it\n-->\n", long(random, 5)),
            11 => format!("<pre>\nraw {}\n\nraw again\n</pre>\n", long(random, 5)),
            12 => format!("    indented code {}\n\n    more of it {}\n", long(random, 10), long(random, 10)),
            13 => format!("Setext {index}\n===\n"),
            14 => "---\n".to_owned(),
            _ => format!("{}\n", long(random, 40)),
        };
        let blanks = "\n".repeat(1 + random.next(3));
        format!("{body}{blanks}")
    }

    fn random_word(index: usize, n: usize) -> usize {
        (index * 31 + n * 17) % 97
    }

    fn document(random: &mut Random, blocks: usize) -> String {
        let mut text = String::new();
        for index in 0..blocks {
            text.push_str(&block(random, index));
        }
        text
    }

    fn contexts(text: &str, random: &mut Random) -> Vec<HighlightContext> {
        let mut contexts = vec![HighlightContext::default()];
        for _ in 0..6 {
            let mut at = random.next(text.len() + 1);
            while !text.is_char_boundary(at) {
                at += 1;
            }
            let end = (at + random.next(2000)).min(text.len());
            let mut end = end;
            while !text.is_char_boundary(end) {
                end += 1;
            }
            contexts.push(HighlightContext::new(text, at..at));
            contexts.push(HighlightContext::new(text, at..end));
        }
        contexts
    }

    #[test]
    fn reading_a_note_in_pieces_gives_what_reading_it_whole_gives() {
        let preview = LivePreview::new(&theme()).title(SpanStyle::new().font_size(24).font_weight(FontWeight::Bold));
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        let mut cut_notes = 0;
        for round in 0..120 {
            let blocks = 12 + random.next(40);
            let text = document(&mut random, blocks);
            if segment_cuts(&text).is_some() {
                cut_notes += 1;
            }
            for context in contexts(&text, &mut random) {
                let pieces = preview.ranges(&text, &context);
                let whole = preview.ranges_whole(&text, &context);
                assert_eq!(
                    format!("{pieces:?}"),
                    format!("{whole:?}"),
                    "round {round}: ranges, with the cursor on {:?}\n{text}",
                    context.active_lines()
                );
                let embeds = |embeds: Vec<TextEmbed>| embeds.iter().map(|embed| (embed.range(), embed.key())).collect::<Vec<_>>();
                assert_eq!(
                    embeds(preview.embeds(&text, &context)),
                    embeds(preview.embeds_whole(&text, &context)),
                    "round {round}: embeds, with the cursor on {:?}",
                    context.active_lines()
                );
            }
        }
        assert!(cut_notes > 80, "the notes were cut in {cut_notes} of 120 rounds");
    }

    /// What `ranges` overlap `window`, as text to compare.
    fn overlapping(ranges: &[StyledRange], window: &Range<usize>) -> String {
        let kept: Vec<_> = ranges.iter().filter(|r| r.range.start < window.end && window.start < r.range.end).collect();
        format!("{kept:?}")
    }

    #[test]
    fn reading_a_window_of_a_long_note_gives_what_reading_it_whole_gives_there() {
        let preview = LivePreview::new(&theme()).title(SpanStyle::new().font_size(24).font_weight(FontWeight::Bold));
        let reference = LivePreview::new(&theme()).title(SpanStyle::new().font_size(24).font_weight(FontWeight::Bold));
        let mut random = Random(0xA5A5_5A5A_1234_4321);
        let mut text = document(&mut random, 1500);
        assert!(text.len() > WINDOWED_BYTES, "{} bytes", text.len());
        let mut snapshot = TextSnapshot::from_text(&text);
        for round in 0..25 {
            if round > 0 {
                // Typed into the note, as a field would, so the preview follows an edit.
                let mut at = random.next(text.len());
                while !text.is_char_boundary(at) {
                    at += 1;
                }
                let inserted = ["a", "\n", "**", "\n\n# New\n\n"][random.next(4)];
                text.insert_str(at, inserted);
                snapshot = snapshot.with_replaced(at..at, inserted);
            }
            for context in contexts(&text, &mut random).into_iter().take(5) {
                let mut start = random.next(text.len());
                while !text.is_char_boundary(start) {
                    start += 1;
                }
                let mut end = (start + 1 + random.next(8000)).min(text.len());
                while !text.is_char_boundary(end) {
                    end += 1;
                }
                let window = start..end;
                let windowed = preview.ranges_window(&snapshot, window.clone(), &context);
                let whole = reference.ranges_whole(&text, &context);
                assert_eq!(
                    overlapping(&windowed, &window),
                    overlapping(&whole, &window),
                    "round {round}: window {window:?}, cursor on {:?}",
                    context.active_lines()
                );
                let embeds = |embeds: Vec<TextEmbed>| {
                    embeds
                        .iter()
                        .filter(|embed| embed.range().start < window.end && window.start < embed.range().end)
                        .map(|embed| embed.range())
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    embeds(preview.embeds_window(&snapshot, window.clone(), &context)),
                    embeds(reference.embeds_whole(&text, &context)),
                    "round {round}: embeds in {window:?}"
                );
            }
        }
    }

    #[test]
    fn a_note_with_a_definition_is_read_whole_and_reads_as_before() {
        let preview = LivePreview::new(&theme());
        let mut random = Random(0x1234_5678_9ABC_DEF1);
        let mut text = document(&mut random, 30);
        text.push_str("\nSee [the site][site].\n\n[site]: https://example.com\n");
        assert!(segment_cuts(&text).is_none());
        let context = HighlightContext::default();
        assert_eq!(format!("{:?}", preview.ranges(&text, &context)), format!("{:?}", preview.ranges_whole(&text, &context)));
    }

    #[test]
    fn an_edit_reads_again_only_the_piece_it_was_typed_in() {
        let preview = LivePreview::new(&theme());
        let mut random = Random(0xDEAD_BEEF_CAFE_F00D);
        let text = document(&mut random, 60);
        let cuts = segment_cuts(&text).expect("a long note is cut");
        let pieces = cuts.len() + 1;
        assert!(pieces >= 8, "{pieces} pieces");

        let context = HighlightContext::default();
        let before = crate::live_segments::pieces_parsed();
        let _ = preview.ranges(&text, &context);
        let first = crate::live_segments::pieces_parsed() - before;
        assert!(first >= pieces - 1 && first <= pieces, "the first reading parses every piece: {first} of {pieces}");

        // A character typed in the middle of the note, in a piece that is a paragraph.
        let middle = cuts[cuts.len() / 2] + 3;
        let mut edited = text.clone();
        edited.insert(middle, 'x');
        let before = crate::live_segments::pieces_parsed();
        let ranges = preview.ranges(&edited, &context);
        assert!(crate::live_segments::pieces_parsed() - before <= 2, "the edit read {} pieces", crate::live_segments::pieces_parsed() - before);
        assert_eq!(format!("{ranges:?}"), format!("{:?}", preview.ranges_whole(&edited, &context)));

        // A line added above: every piece after it moved, and none of them is read again.
        let mut shifted = edited.clone();
        shifted.insert_str(0, "A new first line\n\n");
        let before = crate::live_segments::pieces_parsed();
        let ranges = preview.ranges(&shifted, &context);
        assert!(crate::live_segments::pieces_parsed() - before <= 3, "a line above read {} pieces", crate::live_segments::pieces_parsed() - before);
        assert_eq!(format!("{ranges:?}"), format!("{:?}", preview.ranges_whole(&shifted, &context)));

        // Moving the cursor between pieces paints the two that changed, and parses nothing.
        let before = crate::live_segments::pieces_parsed();
        for at in [cuts[1] + 2, cuts[4] + 2, cuts[1] + 2] {
            let _ = preview.ranges(&shifted, &HighlightContext::new(&shifted, at..at));
        }
        assert_eq!(crate::live_segments::pieces_parsed(), before, "a cursor move does not parse");
    }

    /// What a keystroke costs the preview of a long note: the ranges and the embeds after a
    /// character typed in the middle, read in pieces and read whole.
    ///
    /// ```text
    /// cargo nextest run -p aimer_markdown --run-ignored only -E 'test(bench_a_keystroke)' --no-capture
    /// ```
    #[test]
    #[ignore = "a timing, not a check"]
    fn bench_a_keystroke() {
        use std::time::Instant;

        let mut random = Random(0xABCD_EF01_2345_6789);
        for kilobytes in [30, 100, 300] {
            let mut text = String::new();
            let mut index = 0;
            while text.len() < kilobytes * 1000 {
                text.push_str(&block(&mut random, index));
                index += 1;
            }
            let context = HighlightContext::default();
            let cursor = HighlightContext::new(&text, text.len() / 2..text.len() / 2);
            for pieces in [false, true] {
                let preview = LivePreview::new(&theme());
                let run = |preview: &LivePreview, text: &str, context: &HighlightContext| {
                    if pieces {
                        let _ = preview.ranges(text, context);
                        let _ = preview.embeds(text, context);
                    } else {
                        let _ = preview.ranges_whole(text, context);
                        let _ = preview.embeds_whole(text, context);
                    }
                };
                let started = Instant::now();
                run(&preview, &text, &context);
                let first = started.elapsed();
                let mut edited = text.clone();
                let mut total = std::time::Duration::ZERO;
                for step in 0..10 {
                    edited.insert(text.len() / 2 + step, 'x');
                    let started = Instant::now();
                    run(&preview, &edited, &cursor);
                    total += started.elapsed();
                }
                println!(
                    "{kilobytes:>4} KB {:<6} first {first:>10.2?}   each keystroke {:>10.2?}",
                    if pieces { "pieces" } else { "whole" },
                    total / 10,
                );
            }
        }
    }

    /// A keystroke in a note of megabytes, read a window at a time: styling and embeds of the
    /// 8 KB around the cursor, after the edit, the way a field asks for them.
    #[test]
    #[ignore = "benchmark"]
    fn bench_a_windowed_keystroke() {
        use std::time::Instant;

        let mut random = Random(0xABCD_EF01_2345_6789);
        for megabytes in [1, 5, 30] {
            let mut text = String::new();
            let mut index = 0;
            while text.len() < megabytes * 1_000_000 {
                text.push_str(&block(&mut random, index));
                index += 1;
            }
            let preview = LivePreview::new(&theme());
            let mut snapshot = TextSnapshot::from_text(&text);
            let at = text.len() / 2;
            let window = |at: usize| at.saturating_sub(4000)..(at + 4000).min(snapshot_len(&text));
            let context = HighlightContext::in_snapshot(&snapshot, at..at);
            let started = Instant::now();
            let _ = preview.ranges_window(&snapshot, window(at), &context);
            let _ = preview.embeds_window(&snapshot, window(at), &context);
            let first = started.elapsed();
            let mut total = std::time::Duration::ZERO;
            for step in 0..20 {
                snapshot = snapshot.with_replaced(at + step..at + step, "x");
                let context = HighlightContext::in_snapshot(&snapshot, at + step + 1..at + step + 1);
                let started = Instant::now();
                let _ = preview.ranges_window(&snapshot, window(at), &context);
                let _ = preview.embeds_window(&snapshot, window(at), &context);
                total += started.elapsed();
            }
            println!("{megabytes:>3} MB   first window {first:>10.2?}   each keystroke {:>10.2?}", total / 20);
        }
    }

    fn snapshot_len(text: &str) -> usize {
        text.len()
    }
}
