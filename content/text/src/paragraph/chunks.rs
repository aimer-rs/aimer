//! Laying a long paragraph out in chunks of hard lines.
//!
//! A hard line (the text between two line feeds) wraps, measures and stacks
//! independently of the others: only its height, and so where the lines after it
//! begin, depends on it. So a paragraph of thousands of lines is cut into chunks
//! of whole hard lines, each chunk is laid out by the ordinary code as a paragraph
//! of its own, and the paragraph's layout is those chunk layouts shifted into
//! place. A chunk is remembered by what is in it, so after an edit only the chunk
//! the edit touched, and its neighbour when the cut moved, is laid out again.
//!
//! The cuts are chosen by the content of the lines and not by counting them: a line
//! ends a chunk when its hash says so (on average one in [`CUT_MODULUS`]), so an
//! edit moves the cuts near it and no others, where cutting every N lines would
//! shift every cut after an inserted line and miss on all of them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use aimer_attribute::ResolvedSize;
use aimer_cupid::text_layout::{
    ParagraphMetrics, TextCluster, TextInteractionLayout, TextLine, TextWritingMode,
};
use aimer_style::{TextOverflow, TextStyle, TextTransform};
use aimer_widget::text_layout_epoch;

use super::{
    BuildContext, GraphemeBox, Paragraph, PreparedFragment, PreparedLayout, PreparedLineBreak,
    prepare_background_runs, prepare_decorations,
};
use crate::TextSource;
use crate::text_span::ResolvedTextSpan;

/// Paragraphs shorter than this are laid out whole: the cache and the assembly are
/// not worth their cost for text that is cheap to lay out anyway.
pub(super) const CHUNKING_MIN_BYTES: usize = 8 * 1024;

/// On average one hard line in this many ends a chunk.
pub(super) const CUT_MODULUS: u64 = 16;

/// No chunk holds more hard lines than this, whatever the hashes say, so a run of
/// lines that never cut cannot make a chunk as slow to lay out as the paragraph.
const MAX_CHUNK_LINES: usize = 64;

/// How many laid-out chunks are remembered, and how many bytes of layout they may
/// hold between them. Past either, the least recently used quarter is dropped.
///
/// A chunk carries a cluster for every character of it, so a note of a megabyte holds
/// tens of megabytes of layout; the budget is what keeps a few such notes from
/// holding all the memory there is. A note larger than the budget loses its
/// oldest chunks as it is laid out, and pays for laying out those again.
const MAX_ENTRIES: usize = 8192;
const MAX_BYTES: usize = 96 * 1024 * 1024;

/// How many distinct styles are numbered. A note has a handful; past this the
/// numbering and everything keyed by it start over.
const MAX_STYLES: usize = 1024;

/// What a laid-out chunk is remembered under.
///
/// `content` is a 64-bit hash of the chunk's text and of the style and link of every
/// run in it; with the lengths beside it, two chunks that differ are not taken for
/// one another in any case that matters here. `paragraph` stands for everything
/// outside the chunk that its layout depends on: the width it wraps to, the scale,
/// the fonts, the language, the alignment and the line height.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ChunkKey {
    content: u64,
    len: usize,
    runs: usize,
    paragraph: u64,
}

struct Entry {
    layout: Rc<PreparedLayout>,
    used: u64,
    bytes: usize,
}

/// About how many bytes `layout` holds: its clusters, grapheme boxes and
/// fragments, which is nearly all of it.
fn layout_bytes(layout: &PreparedLayout) -> usize {
    let clusters = layout
        .aimer_interaction
        .as_ref()
        .map_or(0, |interaction| interaction.clusters.len() * std::mem::size_of::<TextCluster>());
    clusters
        + layout.graphemes.len() * std::mem::size_of::<GraphemeBox>()
        + layout.fragments.len() * (std::mem::size_of::<PreparedFragment>() + 24)
        + layout.line_breaks.len() * std::mem::size_of::<PreparedLineBreak>()
}

/// The chunks laid out recently, and the numbering of styles their keys use.
///
/// A style is numbered by the order it was first seen in, in the cache and not in
/// the paragraph: the same style must have the same number in the next paragraph
/// built after an edit, or every chunk would look new.
struct ChunkCache {
    styles: Vec<TextStyle>,
    entries: HashMap<ChunkKey, Entry>,
    clock: u64,
    bytes: usize,
    budget: usize,
}

impl Default for ChunkCache {
    fn default() -> Self {
        Self::with_budget(MAX_BYTES)
    }
}

impl ChunkCache {
    fn style_id(&mut self, style: &TextStyle) -> u64 {
        if self.styles.len() >= MAX_STYLES {
            self.styles.clear();
            self.entries.clear();
            self.bytes = 0;
        }
        // Runs alternate between a few styles, so the latest is the likeliest.
        if let Some(at) = self.styles.iter().rposition(|known| known == style) {
            return at as u64;
        }
        self.styles.push(*style);
        (self.styles.len() - 1) as u64
    }

    fn get(&mut self, key: &ChunkKey) -> Option<Rc<PreparedLayout>> {
        self.clock += 1;
        let entry = self.entries.get_mut(key)?;
        entry.used = self.clock;
        Some(Rc::clone(&entry.layout))
    }

    fn insert(&mut self, key: ChunkKey, layout: Rc<PreparedLayout>) {
        self.clock += 1;
        let bytes = layout_bytes(&layout);
        if let Some(replaced) = self.entries.insert(key, Entry { layout, used: self.clock, bytes }) {
            self.bytes -= replaced.bytes;
        }
        self.bytes += bytes;
        // The chunk just added is the most recently used, so it is the last to go.
        while (self.entries.len() > MAX_ENTRIES || self.bytes > self.budget) && self.entries.len() > 1 {
            let mut stamps = self.entries.values().map(|entry| entry.used).collect::<Vec<_>>();
            stamps.sort_unstable();
            let oldest_quarter = stamps[stamps.len() / 4];
            self.entries.retain(|_, entry| entry.used > oldest_quarter);
            self.bytes = self.entries.values().map(|entry| entry.bytes).sum();
        }
    }

    fn with_budget(budget: usize) -> Self {
        Self { styles: Vec::new(), entries: HashMap::new(), clock: 0, bytes: 0, budget }
    }
}

thread_local! {
    static CACHE: RefCell<ChunkCache> = RefCell::new(ChunkCache::default());
}

thread_local! {
    /// The least text, in bytes, a paragraph must have to be laid out in chunks while a
    /// [`with_chunking_min_bytes`] scope is open.
    static MIN_BYTES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Runs `lay_out` with paragraphs of at least `min_bytes` of text laid out in chunks, where
/// the usual threshold is higher.
///
/// A windowed field lays out a few kilobytes at a time, over and over as it scrolls, and
/// most of each window is what the one before it held; chunks are what makes that the
/// case in the cache too.
pub fn with_chunking_min_bytes<R>(min_bytes: usize, lay_out: impl FnOnce() -> R) -> R {
    let previous = MIN_BYTES.with(|slot| slot.replace(Some(min_bytes)));
    let result = lay_out();
    MIN_BYTES.with(|slot| slot.set(previous));
    result
}

#[cfg(test)]
thread_local! {
    /// How many chunks were laid out, rather than found, for tests.
    static CHUNKS_LAID_OUT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn chunks_laid_out() -> usize {
    CHUNKS_LAID_OUT.with(std::cell::Cell::get)
}

/// Where the clusters of a chunk's line breaks begin.
///
/// They follow the clusters of the text, one per break unless it merged into the
/// cluster before it, so they are found from the end, breaks last to first.
fn first_break_cluster(
    clusters: &[TextCluster],
    line_breaks: &[PreparedLineBreak],
) -> usize {
    let mut from = clusters.len();
    for line_break in line_breaks.iter().rev() {
        match clusters[..from].last() {
            Some(cluster)
                if cluster.line_index == line_break.line
                    && cluster.text_range == line_break.source_range =>
            {
                from -= 1;
            }
            _ => {}
        }
    }
    from
}

/// The part of one span a chunk holds.
struct SpanSlice {
    span: usize,
    range: Range<usize>,
}

/// One chunk of the paragraph: a run of whole hard lines.
struct ChunkSpec {
    /// Where the chunk starts in the paragraph's text, in bytes.
    start: usize,
    len: usize,
    slices: Vec<SpanSlice>,
    content: u64,
    /// The span holding the line feed that ends the chunk before this one, which
    /// is the style the line break above this chunk's first line is measured in.
    prefix: Option<usize>,
}

impl ChunkSpec {
    fn new(start: usize) -> Self {
        Self { start, len: 0, slices: Vec::new(), content: 0, prefix: None }
    }

    /// The chunk after one that ended in a line feed held by `span`, whose style
    /// (`header`) it takes part in the key of.
    fn following(start: usize, span: usize, header: u64) -> Self {
        Self { start, len: 0, slices: Vec::new(), content: mix(0, header), prefix: Some(span) }
    }

    /// How many bytes of the layout's own text come before the chunk's: the line
    /// feed that stands for the break above it.
    fn prefix_len(&self) -> usize {
        usize::from(self.prefix.is_some())
    }
}

#[inline]
fn mix(hash: u64, word: u64) -> u64 {
    (hash.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95)
}

/// Folds `bytes` into `hash`, a word at a time: a byte loop is slow enough in a
/// debug build to be felt on a note of several hundred kilobytes.
fn hash_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    let mut words = bytes.chunks_exact(8);
    for word in &mut words {
        hash = mix(hash, u64::from_le_bytes(word.try_into().expect("a chunk of eight bytes")));
    }
    let mut tail = 0u64;
    for (index, byte) in words.remainder().iter().enumerate() {
        tail |= u64::from(*byte) << (8 * index);
    }
    mix(mix(hash, tail), bytes.len() as u64)
}

/// Cuts the spans' text into chunks of whole hard lines.
fn plan_chunks(spans: &[ResolvedTextSpan], cache: &mut ChunkCache) -> Vec<ChunkSpec> {
    let mut chunks = Vec::new();
    let mut current = ChunkSpec::new(0);
    let mut line_hash = 0u64;
    let mut lines_in_chunk = 0usize;
    let mut offset = 0usize;
    for (index, span) in spans.iter().enumerate() {
        let text = span.text.as_str();
        let header = {
            let link = span.link.as_ref().map_or(0, |link| hash_bytes(1, link.as_bytes()));
            mix(cache.style_id(&span.style), link)
        };
        let mut slice_start = 0;
        let mut at = 0;
        if let Some(inline_box) = span.inline_box {
            // A box is one atom however many source lines it stands for: no cut falls inside it,
            // and its line feeds end no line. The line it is on goes on, with its size mixed in.
            line_hash = hash_bytes(line_hash, text.as_bytes());
            line_hash = mix(line_hash, u64::from(inline_box.width.to_bits()));
            line_hash = mix(line_hash, u64::from(inline_box.height.to_bits()));
            line_hash = mix(line_hash, u64::from(inline_box.baseline.to_bits()));
            at = text.len();
        }
        while at < text.len() {
            match text[at..].find('\n') {
                Some(found) => {
                    let end = at + found + 1;
                    line_hash = hash_bytes(line_hash, &text.as_bytes()[at..end]);
                    current.content = mix(current.content, line_hash);
                    lines_in_chunk += 1;
                    let cut = (line_hash >> 32) % CUT_MODULUS == 0 || lines_in_chunk >= MAX_CHUNK_LINES;
                    line_hash = 0;
                    at = end;
                    if cut {
                        current.content = mix(current.content, mix(header, (end - slice_start) as u64));
                        current.slices.push(SpanSlice { span: index, range: slice_start..end });
                        current.len = offset + end - current.start;
                        let next_start = current.start + current.len;
                        chunks.push(std::mem::replace(
                            &mut current,
                            ChunkSpec::following(next_start, index, header),
                        ));
                        lines_in_chunk = 0;
                        slice_start = end;
                    }
                }
                None => {
                    line_hash = hash_bytes(line_hash, &text.as_bytes()[at..]);
                    at = text.len();
                }
            }
        }
        if slice_start < text.len() {
            current.content =
                mix(current.content, mix(header, (text.len() - slice_start) as u64));
            current.slices.push(SpanSlice { span: index, range: slice_start..text.len() });
        }
        offset += text.len();
    }
    if !current.slices.is_empty() {
        // The last chunk may end inside a line.
        current.content = mix(current.content, line_hash);
        current.len = offset - current.start;
        chunks.push(current);
    }
    chunks
}

impl Paragraph {
    /// The layout of a long paragraph made from chunk layouts, or `None` when the
    /// paragraph is one that is laid out whole.
    pub(super) fn compute_chunked(
        &self,
        ctx: &BuildContext,
        include_graphemes: bool,
    ) -> Option<PreparedLayout> {
        let min_bytes = MIN_BYTES.with(std::cell::Cell::get).unwrap_or(CHUNKING_MIN_BYTES);
        self.chunked_layout(ctx, include_graphemes, min_bytes)
    }

    /// Whether the paragraph is one whose hard lines lay out apart: wrapped text
    /// (so every chunk wraps to the paragraph's own width), or text that is clipped and starts
    /// each line where the paragraph does (it has no width to wrap to, and the width the chunks
    /// make up is that of the widest), without a first-line indent or case change that carries
    /// from one line to the next, and in more than one span (a single span takes the shared
    /// shaper's geometry, which a paragraph of several spans does not).
    fn lines_lay_out_apart(&self) -> bool {
        (matches!(self.overflow, TextOverflow::Wrap)
            || (matches!(self.overflow, TextOverflow::Clip) && !self.aligns_within_width()))
            && self.text_indent == 0.0
            && self.spans.len() >= 2
            && self
                .spans
                .iter()
                .all(|span| span.style.text_transform == TextTransform::None)
    }

    /// What outside a chunk its layout depends on; see [`ChunkKey`].
    fn chunk_signature(&self, ctx: &BuildContext, include_graphemes: bool) -> u64 {
        let mut hash = mix(0, u64::from(self.wrap_width(ctx).to_bits()));
        hash = mix(hash, u64::from(ctx.scale.to_bits()));
        hash = mix(hash, text_layout_epoch());
        hash = mix(hash, u64::from(include_graphemes));
        hash = hash_bytes(hash, format!("{:?}", ctx.canvas.text_language()).as_bytes());
        hash_bytes(hash, format!("{:?}{:?}", self.text_align, self.line_height).as_bytes())
    }

    /// [`compute_chunked`](Self::compute_chunked) for paragraphs of at least
    /// `min_bytes` of text.
    pub(super) fn chunked_layout(
        &self,
        ctx: &BuildContext,
        include_graphemes: bool,
        min_bytes: usize,
    ) -> Option<PreparedLayout> {
        if !self.lines_lay_out_apart() {
            return None;
        }
        let total = self.spans.iter().map(|span| span.text.len()).sum::<usize>();
        if total < min_bytes {
            return None;
        }
        let chunks = CACHE.with(|cache| plan_chunks(&self.spans, &mut cache.borrow_mut()));
        if chunks.len() < 2 {
            return None;
        }

        // The whole paragraph measures every grapheme as soon as any run of it is
        // spaced, so a chunk of unspaced runs must too.
        let include_graphemes = include_graphemes
            || self.spans.iter().any(|span| {
                !span.text.is_empty()
                    && span.style.letter_spacing.is_finite()
                    && span.style.word_spacing.is_finite()
                    && (span.style.letter_spacing != 0.0 || span.style.word_spacing != 0.0)
            });
        let signature = self.chunk_signature(ctx, include_graphemes);
        let mut parts = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            let key = ChunkKey {
                content: chunk.content,
                len: chunk.len,
                runs: chunk.slices.len(),
                paragraph: signature,
            };
            let found = CACHE.with(|cache| cache.borrow_mut().get(&key));
            let layout = match found {
                Some(layout) => layout,
                None => {
                    #[cfg(test)]
                    CHUNKS_LAID_OUT.with(|count| count.set(count.get() + 1));
                    let layout = Rc::new(self.lay_out_chunk(ctx, include_graphemes, chunk));
                    CACHE.with(|cache| cache.borrow_mut().insert(key, Rc::clone(&layout)));
                    layout
                }
            };
            parts.push(layout);
        }
        self.assemble(ctx, &chunks, &parts)
    }

    /// Lays one chunk out as a paragraph of its own.
    fn lay_out_chunk(
        &self,
        ctx: &BuildContext,
        include_graphemes: bool,
        chunk: &ChunkSpec,
    ) -> PreparedLayout {
        // The line feed above a chunk's first line comes first, so that line is
        // measured with the break metrics it has in the whole paragraph.
        let prefix = chunk.prefix.map(|span| {
            let source = &self.spans[span];
            let mut prefix = ResolvedTextSpan::plain(Rc::<str>::from("\n"), source.style);
            prefix.link = source.link.clone();
            prefix
        });
        let spans = prefix
            .into_iter()
            .chain(chunk.slices.iter().map(|slice| {
                let source = &self.spans[slice.span];
                let mut span = ResolvedTextSpan::plain(
                    Rc::<str>::from(&source.text.as_str()[slice.range.clone()]),
                    source.style,
                );
                span.link = source.link.clone();
                span.inline_box = source.inline_box;
                span
            }))
            .collect();
        let chunk_paragraph = Paragraph::with_layout(
            spans,
            self.text_align,
            self.overflow,
            self.line_height,
            0.0,
        );
        chunk_paragraph.compute_layout_whole(ctx, include_graphemes, true)
    }

    /// Puts the chunk layouts together: each moved down by the height of those
    /// before it, on by their lines and bytes, and onto the paragraph's own spans.
    ///
    /// A chunk is laid out with the line feed that ends the one before it in front
    /// of it, and ends with the empty line after its own last line feed. Both lines
    /// belong to the neighbour in the whole paragraph, so they are left out here:
    /// the first line of a chunk is the second of its layout, and the line after
    /// its last line feed is not there.
    fn assemble(
        &self,
        ctx: &BuildContext,
        chunks: &[ChunkSpec],
        parts: &[Rc<PreparedLayout>],
    ) -> Option<PreparedLayout> {
        // Wrapped, every chunk is as wide as the paragraph is. Clipped, each is as wide as its widest
        // line, and the paragraph as the widest of them.
        let wrapped = matches!(self.overflow, TextOverflow::Wrap);
        let width = if wrapped {
            parts.first()?.size.width
        } else {
            parts.iter().map(|part| part.size.width).fold(0.0, f32::max)
        };
        let fragment_count = parts.iter().map(|part| part.fragments.len()).sum();
        let cluster_count = parts
            .iter()
            .map(|part| part.aimer_interaction.as_ref().map_or(0, |layout| layout.clusters.len()))
            .sum();
        let mut fragments = Vec::with_capacity(fragment_count);
        let mut graphemes = Vec::new();
        let mut paint_runs = Vec::with_capacity(fragment_count);
        let mut line_breaks = Vec::new();
        let mut line_heights = Vec::new();
        let mut text = String::new();
        let mut lines = Vec::new();
        let mut clusters = Vec::with_capacity(cluster_count);
        let mut break_clusters = Vec::new();
        let (mut ascent, mut descent, mut line_gap, mut line_height) =
            (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
        let (mut line_offset, mut y_offset) = (0usize, 0.0_f32);
        let shift = |range: &Range<usize>, base: usize| range.start + base..range.end + base;

        for (index, (chunk, part)) in chunks.iter().zip(parts).enumerate() {
            let interaction = part.aimer_interaction.as_ref()?;
            let prefix_len = chunk.prefix_len();
            let layout_lines = part.line_heights.len();
            // The lines of the layout that are the chunk's own.
            let first_line = prefix_len;
            let end_line = if index + 1 == chunks.len() {
                layout_lines
            } else {
                layout_lines.saturating_sub(1)
            };
            if first_line >= end_line {
                return None;
            }
            // Where the chunk's text starts in the layout's, and the layout's top
            // of its own first line, which is where the chunk lands.
            let base = chunk.start.checked_sub(prefix_len)?;
            let skipped_top: f32 = part.line_heights[..first_line].iter().sum();
            let y_shift = y_offset - skipped_top;
            let own_line = |line: usize| (first_line..end_line).contains(&line);
            let line_shift = |line: usize| line + line_offset - first_line;

            let first_fragment = fragments.len();
            for fragment in &part.fragments {
                if !own_line(fragment.line) {
                    return None;
                }
                fragments.push(PreparedFragment {
                    span_index: chunk.slices[fragment.span_index.checked_sub(prefix_len)?].span,
                    text: fragment.text.clone(),
                    source_range: fragment.source_range.as_ref().map(|range| shift(range, base)),
                    rendered_source_ranges: fragment
                        .rendered_source_ranges
                        .iter()
                        .map(|range| shift(range, base))
                        .collect(),
                    line: line_shift(fragment.line),
                    x: fragment.x,
                    baseline: fragment.baseline + y_shift,
                    width: fragment.width,
                    height: fragment.height,
                    ascent: fragment.ascent,
                    descent: fragment.descent,
                });
            }
            for grapheme in &part.graphemes {
                graphemes.push(GraphemeBox {
                    source_range: shift(&grapheme.source_range, base),
                    fragment_index: grapheme.fragment_index + first_fragment,
                    rendered_range: grapheme.rendered_range.clone(),
                    x: grapheme.x,
                    width: grapheme.width,
                });
            }
            paint_runs.extend(part.paint_runs.iter().cloned());
            for line_break in part.line_breaks.iter().filter(|line_break| own_line(line_break.line)) {
                line_breaks.push(PreparedLineBreak {
                    source_range: shift(&line_break.source_range, base),
                    line: line_shift(line_break.line),
                    x: line_break.x,
                    y: line_break.y + y_shift,
                    // A line break can be hit out to the edge of the paragraph, which is not the
                    // edge of the chunk when the paragraph is as wide as its widest line.
                    hit_width: if wrapped {
                        line_break.hit_width
                    } else {
                        (width - line_break.x).max(ctx.scale)
                    },
                    selection_width: line_break.selection_width,
                    height: line_break.height,
                });
            }
            let own_heights = &part.line_heights[first_line..end_line];
            line_heights.extend_from_slice(own_heights);

            text.push_str(&interaction.text[prefix_len..]);
            for line in &interaction.lines[first_line..end_line] {
                lines.push(TextLine {
                    text_range: shift(&line.text_range, base),
                    glyph_range: line.glyph_range.clone(),
                    baseline: line.baseline + y_shift,
                    width: line.width,
                    ascent: line.ascent,
                    descent: line.descent,
                    line_gap: line.line_gap,
                    hard_break: line.hard_break,
                });
                ascent = ascent.max(line.ascent);
                descent = descent.min(line.descent);
                line_gap = line_gap.max(line.line_gap);
            }
            // The whole paragraph lists the clusters of its text and then those of its
            // line breaks, as the layout of a chunk does: keep the two apart.
            let breaks_from = first_break_cluster(&interaction.clusters, &part.line_breaks);
            for (at, cluster) in interaction.clusters.iter().enumerate() {
                if !own_line(cluster.line_index) {
                    continue;
                }
                let target = if at >= breaks_from { &mut break_clusters } else { &mut clusters };
                target.push(TextCluster {
                    text_range: shift(&cluster.text_range, base),
                    line_index: line_shift(cluster.line_index),
                    level: cluster.level,
                    start_x: cluster.start_x,
                    end_x: cluster.end_x,
                    start_y: cluster.start_y + y_shift,
                    end_y: cluster.end_y + y_shift,
                    y: cluster.y + y_shift,
                    height: cluster.height,
                });
            }
            line_height = own_heights.iter().copied().fold(line_height, f32::max);

            line_offset += own_heights.len();
            y_offset += own_heights.iter().sum::<f32>();
        }

        clusters.append(&mut break_clusters);
        let height = y_offset;
        let line_count = line_heights.len().max(1);
        let decorations = prepare_decorations(&fragments, &self.spans, ctx.scale);
        let backgrounds = prepare_background_runs(&fragments, &self.spans);
        Some(PreparedLayout {
            fragments,
            decorations,
            graphemes,
            paint_runs,
            backgrounds,
            line_breaks,
            line_heights,
            size: ResolvedSize { width, height },
            aimer_interaction: Some(Rc::new(TextInteractionLayout {
                text,
                lines,
                clusters,
                metrics: ParagraphMetrics {
                    width,
                    height,
                    ascent,
                    descent,
                    line_gap,
                    line_height,
                    line_count,
                },
                origin_x: 0.0,
                origin_y: 0.0,
                writing_mode: TextWritingMode::HorizontalTb,
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use aimer_attribute::{ResolvedSize, Vec2d};
    use aimer_canvas::{FrameCanvas, InnerCanvas};
    use aimer_style::{
        FontWeight, LineHeight, TextAlign, TextDecoration, TextDecorationLine, TextOverflow,
        TextStyle,
    };
    use aimer_widget::base::{BuildContext, Color, WindowHandle};

    use super::super::{Paragraph, PreparedLayout};
    use crate::TextSource;
    use crate::text_span::ResolvedTextSpan;

    fn with_context<R>(width: f32, scale: f32, check: impl FnOnce(&BuildContext) -> R) -> R {
        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize { width, height: 400.0 },
            scale,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(width as u32, 400), 1.0),
            runtime.handle().clone(),
        );
        check(&context)
    }

    fn base() -> TextStyle {
        TextStyle::new().font_size(16)
    }

    fn styles() -> Vec<(TextStyle, Option<&'static str>)> {
        vec![
            (base(), None),
            (base().font_weight(FontWeight::Bold), None),
            (base().color(Color::Rgba(200, 30, 30, 255)), None),
            (base().background_color(Color::Rgba(250, 230, 120, 255)), None),
            (
                base().text_decoration(TextDecoration::new().line(TextDecorationLine::UNDERLINE)),
                Some("https://example.com"),
            ),
            (base().letter_spacing(1.5), None),
            (base().font_size(26), None),
        ]
    }

    /// A note of `lines` hard lines in several styles: short and long (wrapping)
    /// lines, blank lines, and styled runs that begin and end anywhere, including
    /// across a line feed.
    fn spans(lines: usize, trailing_newline: bool) -> Vec<ResolvedTextSpan> {
        let styles = styles();
        let mut text = String::new();
        for index in 0..lines {
            match index % 6 {
                0 => text.push_str(&format!("Heading {index}\n")),
                1 => text.push_str(&format!(
                    "A long paragraph line {index} with enough words in it that it has to wrap \
                     across the width of the editor more than once, at least when it is narrow\n"
                )),
                2 => text.push('\n'),
                3 => text.push_str(&format!("- item {index} with **bold** words\n")),
                4 => text.push_str(&format!("tab\tseparated {index}\r\n")),
                _ => text.push_str(&format!("last of a group {index}")),
            }
            if index % 6 == 5 {
                text.push('\n');
            }
        }
        if !trailing_newline && text.ends_with('\n') {
            text.pop();
        }
        if trailing_newline && !text.ends_with('\n') {
            text.push('\n');
        }
        // Cut the text into runs of varying length, each in the next style.
        let mut spans = Vec::new();
        let mut rest = text.as_str();
        let mut turn = 0usize;
        while !rest.is_empty() {
            let mut take = 7 + (turn * 13) % 61;
            while take < rest.len() && !rest.is_char_boundary(take) {
                take += 1;
            }
            let take = take.min(rest.len());
            let (head, tail) = rest.split_at(take);
            let (style, link) = styles[turn % styles.len()].clone();
            let mut span = ResolvedTextSpan::plain(Rc::from(head), style);
            span.link = link.map(|target| TextSource::from(Rc::<str>::from(target)));
            spans.push(span);
            rest = tail;
            turn += 1;
        }
        spans
    }

    fn near(a: f32, b: f32, what: &str) {
        assert!(
            // Sums taken in another order differ by rounding: far under a pixel.
            (a - b).abs() <= 0.02 + 1e-5 * a.abs().max(b.abs()),
            "{what}: {a} against {b}"
        );
    }

    fn assert_same(chunked: &PreparedLayout, whole: &PreparedLayout, what: &str) {
        assert_eq!(chunked.fragments.len(), whole.fragments.len(), "{what}: fragments");
        for (index, (got, want)) in chunked.fragments.iter().zip(&whole.fragments).enumerate() {
            let at = format!("{what}: fragment {index}");
            assert_eq!(got.span_index, want.span_index, "{at} span");
            assert_eq!(got.text, want.text, "{at} text");
            assert_eq!(got.source_range, want.source_range, "{at} source");
            assert_eq!(got.rendered_source_ranges, want.rendered_source_ranges, "{at} rendered");
            assert_eq!(got.line, want.line, "{at} line");
            near(got.x, want.x, &format!("{at} x"));
            near(got.baseline, want.baseline, &format!("{at} baseline"));
            near(got.width, want.width, &format!("{at} width"));
            near(got.height, want.height, &format!("{at} height"));
            near(got.ascent, want.ascent, &format!("{at} ascent"));
            near(got.descent, want.descent, &format!("{at} descent"));
        }
        assert_eq!(chunked.decorations.len(), whole.decorations.len(), "{what}: decorations");
        for (got, want) in chunked.decorations.iter().zip(&whole.decorations) {
            assert_eq!(got.fragment_index, want.fragment_index, "{what}: decoration fragment");
            near(got.underline_center, want.underline_center, &format!("{what}: underline"));
        }
        assert_eq!(chunked.graphemes.len(), whole.graphemes.len(), "{what}: graphemes");
        for (got, want) in chunked.graphemes.iter().zip(&whole.graphemes) {
            assert_eq!(got.source_range, want.source_range, "{what}: grapheme source");
            assert_eq!(got.fragment_index, want.fragment_index, "{what}: grapheme fragment");
            assert_eq!(got.rendered_range, want.rendered_range, "{what}: grapheme rendered");
            near(got.x, want.x, &format!("{what}: grapheme x"));
            near(got.width, want.width, &format!("{what}: grapheme width"));
        }
        assert_eq!(chunked.paint_runs.len(), whole.paint_runs.len(), "{what}: paint runs");
        for (got, want) in chunked.paint_runs.iter().zip(&whole.paint_runs) {
            assert_eq!(got.len(), want.len(), "{what}: paint run length");
            for (got, want) in got.iter().zip(want) {
                assert_eq!(got.rendered_range, want.rendered_range, "{what}: paint run range");
                near(got.x, want.x, &format!("{what}: paint run x"));
            }
        }
        assert_eq!(chunked.backgrounds.len(), whole.backgrounds.len(), "{what}: backgrounds");
        for (got, want) in chunked.backgrounds.iter().zip(&whole.backgrounds) {
            assert_eq!(got.line, want.line, "{what}: background line");
            near(got.x, want.x, &format!("{what}: background x"));
            near(got.y, want.y, &format!("{what}: background y"));
            near(got.width, want.width, &format!("{what}: background width"));
            near(got.height, want.height, &format!("{what}: background height"));
        }
        assert_eq!(chunked.line_breaks.len(), whole.line_breaks.len(), "{what}: line breaks");
        for (got, want) in chunked.line_breaks.iter().zip(&whole.line_breaks) {
            assert_eq!(got.source_range, want.source_range, "{what}: break source");
            assert_eq!(got.line, want.line, "{what}: break line");
            near(got.x, want.x, &format!("{what}: break x"));
            near(got.y, want.y, &format!("{what}: break y"));
            near(got.hit_width, want.hit_width, &format!("{what}: break hit width"));
            near(got.height, want.height, &format!("{what}: break height"));
        }
        assert_eq!(chunked.line_heights.len(), whole.line_heights.len(), "{what}: line heights");
        for (got, want) in chunked.line_heights.iter().zip(&whole.line_heights) {
            near(*got, *want, &format!("{what}: line height"));
        }
        near(chunked.size.width, whole.size.width, &format!("{what}: width"));
        near(chunked.size.height, whole.size.height, &format!("{what}: height"));

        let got = chunked.aimer_interaction.as_ref().expect("interaction geometry");
        let want = whole.aimer_interaction.as_ref().expect("interaction geometry");
        assert_eq!(got.text, want.text, "{what}: interaction text");
        assert_eq!(got.lines.len(), want.lines.len(), "{what}: interaction lines");
        for (index, (got, want)) in got.lines.iter().zip(&want.lines).enumerate() {
            let at = format!("{what}: interaction line {index}");
            assert_eq!(got.text_range, want.text_range, "{at} range");
            assert_eq!(got.hard_break, want.hard_break, "{at} hard break");
            near(got.baseline, want.baseline, &format!("{at} baseline"));
            near(got.width, want.width, &format!("{at} width"));
            near(got.ascent, want.ascent, &format!("{at} ascent"));
            near(got.descent, want.descent, &format!("{at} descent"));
            near(got.line_gap, want.line_gap, &format!("{at} gap"));
        }
        assert_eq!(got.clusters.len(), want.clusters.len(), "{what}: clusters");
        for (index, (got, want)) in got.clusters.iter().zip(&want.clusters).enumerate() {
            let at = format!("{what}: cluster {index}");
            assert_eq!(got.text_range, want.text_range, "{at} range");
            assert_eq!(got.line_index, want.line_index, "{at} line");
            assert_eq!(got.level, want.level, "{at} level");
            near(got.start_x, want.start_x, &format!("{at} start x"));
            near(got.end_x, want.end_x, &format!("{at} end x"));
            near(got.start_y, want.start_y, &format!("{at} start y"));
            near(got.end_y, want.end_y, &format!("{at} end y"));
            near(got.y, want.y, &format!("{at} y"));
            near(got.height, want.height, &format!("{at} height"));
        }
        near(got.metrics.width, want.metrics.width, &format!("{what}: metrics width"));
        near(got.metrics.height, want.metrics.height, &format!("{what}: metrics height"));
        near(got.metrics.ascent, want.metrics.ascent, &format!("{what}: metrics ascent"));
        near(got.metrics.descent, want.metrics.descent, &format!("{what}: metrics descent"));
        near(got.metrics.line_gap, want.metrics.line_gap, &format!("{what}: metrics gap"));
        near(got.metrics.line_height, want.metrics.line_height, &format!("{what}: metrics line height"));
        assert_eq!(got.metrics.line_count, want.metrics.line_count, "{what}: line count");
    }

    fn paragraph(spans: Vec<ResolvedTextSpan>, align: TextAlign, height: LineHeight) -> Paragraph {
        Paragraph::with_layout(spans, align, TextOverflow::Wrap, height, 0.0)
    }

    #[test]
    fn chunks_assemble_to_what_laying_the_paragraph_out_whole_gives() {
        let cases = [
            ("left", TextAlign::TopLeft, LineHeight::default(), 300.0, 1.0, false),
            ("trailing newline", TextAlign::TopLeft, LineHeight::default(), 300.0, 1.0, true),
            ("centred", TextAlign::TopCenter, LineHeight::default(), 280.0, 1.0, false),
            ("fixed line height", TextAlign::TopLeft, LineHeight::Px(30.0), 300.0, 1.0, false),
            ("scaled", TextAlign::TopRight, LineHeight::default(), 240.0, 2.0, true),
        ];
        for (what, align, line_height, width, scale, trailing) in cases {
            with_context(width, scale, |ctx| {
                for include_graphemes in [false, true] {
                    let whole = paragraph(spans(120, trailing), align, line_height)
                        .compute_layout_whole(ctx, include_graphemes, false);
                    let chunked = paragraph(spans(120, trailing), align, line_height)
                        .chunked_layout(ctx, include_graphemes, 0)
                        .unwrap_or_else(|| panic!("{what}: the paragraph was not chunked"));
                    assert_same(&chunked, &whole, &format!("{what} graphemes={include_graphemes}"));
                }
            });
        }
    }

    fn clipped(spans: Vec<ResolvedTextSpan>, align: TextAlign, height: LineHeight) -> Paragraph {
        Paragraph::with_layout(spans, align, TextOverflow::Clip, height, 0.0)
    }

    #[test]
    fn chunks_of_text_that_does_not_wrap_assemble_to_what_laying_it_out_whole_gives() {
        // Without a width to wrap to, the paragraph is as wide as its widest line, which lies in one
        // chunk or another: the chunks are laid out apart and the width is the widest of them.
        let cases = [
            ("left", TextAlign::TopLeft, LineHeight::default(), 300.0, 1.0, false),
            ("trailing newline", TextAlign::TopLeft, LineHeight::default(), 300.0, 1.0, true),
            ("fixed line height", TextAlign::MidLeft, LineHeight::Px(30.0), 300.0, 1.0, false),
            ("scaled", TextAlign::TopLeft, LineHeight::default(), 240.0, 2.0, true),
        ];
        for (what, align, line_height, width, scale, trailing) in cases {
            with_context(width, scale, |ctx| {
                for include_graphemes in [false, true] {
                    let whole = clipped(spans(120, trailing), align, line_height)
                        .compute_layout_whole(ctx, include_graphemes, false);
                    let chunked = clipped(spans(120, trailing), align, line_height)
                        .chunked_layout(ctx, include_graphemes, 0)
                        .unwrap_or_else(|| panic!("{what}: the paragraph was not chunked"));
                    assert_same(&chunked, &whole, &format!("{what} graphemes={include_graphemes}"));
                }
            });
        }
    }

    #[test]
    fn a_chunk_of_text_that_does_not_wrap_keeps_its_layout_when_the_width_it_is_given_changes() {
        // Nothing in it depends on that width, so a resize does not lay the text out again.
        let text = note(60);
        let paragraph = || clipped(markdown_spans(&text), TextAlign::TopLeft, LineHeight::default());
        with_context(300.0, 1.0, |ctx| {
            let _ = paragraph().chunked_layout(ctx, true, 0).expect("chunked");
        });
        let laid_out = super::chunks_laid_out();
        with_context(420.0, 1.0, |ctx| {
            let _ = paragraph().chunked_layout(ctx, true, 0).expect("chunked");
        });
        assert_eq!(super::chunks_laid_out(), laid_out);
    }

    /// `spans(lines, ..)` with a box standing for several source lines (a table, a code block)
    /// every so often: the span that holds the lines it stands for is cut out of the runs around
    /// it, as an embed does, and given a box of its own size.
    fn spans_with_boxes(lines: usize, trailing_newline: bool) -> Vec<ResolvedTextSpan> {
        let plain = spans(lines, trailing_newline);
        let mut out = Vec::new();
        for (index, span) in plain.into_iter().enumerate() {
            out.push(span);
            if index % 5 == 4 {
                // A box whose source runs over three hard lines, tall enough to matter.
                // Every other one is a short box in the middle of a line, with no line feed.
                let source = if index % 2 == 0 { "\n| a | b |\n| - | - |\n| 0 | 1 |\n" } else { "[img]" };
                let mut boxed = ResolvedTextSpan::plain(Rc::from(source), base());
                boxed.inline_box = Some(crate::text_span::InlineBox::new(240.0, 20.0 + (index % 4) as f32 * 30.0));
                out.push(boxed);
            }
        }
        out
    }

    #[test]
    fn a_paragraph_with_boxes_assembles_to_what_laying_it_out_whole_gives() {
        let cases = [
            ("left", TextAlign::TopLeft, LineHeight::default(), 300.0, 1.0, false),
            ("trailing newline", TextAlign::TopLeft, LineHeight::default(), 300.0, 1.0, true),
            ("scaled", TextAlign::TopRight, LineHeight::default(), 240.0, 2.0, true),
        ];
        for (what, align, line_height, width, scale, trailing) in cases {
            with_context(width, scale, |ctx| {
                for include_graphemes in [false, true] {
                    let whole = paragraph(spans_with_boxes(120, trailing), align, line_height)
                        .compute_layout_whole(ctx, include_graphemes, false);
                    let chunked = paragraph(spans_with_boxes(120, trailing), align, line_height)
                        .chunked_layout(ctx, include_graphemes, 0)
                        .unwrap_or_else(|| panic!("{what}: the paragraph with boxes was not chunked"));
                    assert_same(&chunked, &whole, &format!("boxes, {what} graphemes={include_graphemes}"));
                }
            });
        }
    }

    /// A note styled by what is in it, as a highlighter does: headings, `**bold**`
    /// words, and plain text, so that editing a line changes the styling of that
    /// line alone.
    fn markdown_spans(text: &str) -> Vec<ResolvedTextSpan> {
        let mut spans = Vec::new();
        let mut push = |text: &str, style: TextStyle| {
            if !text.is_empty() {
                spans.push(ResolvedTextSpan::plain(Rc::from(text), style));
            }
        };
        for line in text.split_inclusive('\n') {
            if line.starts_with("## ") {
                push(line, base().font_size(24).font_weight(FontWeight::Bold));
                continue;
            }
            let mut rest = line;
            while let Some(open) = rest.find("**") {
                let Some(close) = rest[open + 2..].find("**") else { break };
                let end = open + 2 + close + 2;
                push(&rest[..open], base());
                push(&rest[open..end], base().font_weight(FontWeight::Bold));
                rest = &rest[end..];
            }
            push(rest, base());
        }
        spans
    }

    fn note(sections: usize) -> String {
        let section = "## A section heading\nSome **bold** words and more words in a line, enough text \
            to wrap across the width of the pane a little.\n- item one\n- item two\n\n";
        (0..sections).map(|index| section.replace("heading", &format!("heading {index}"))).collect()
    }

    fn whole_layout(ctx: &BuildContext, text: &str) -> PreparedLayout {
        paragraph(markdown_spans(text), TextAlign::TopLeft, LineHeight::default())
            .compute_layout_whole(ctx, true, false)
    }

    fn chunked(ctx: &BuildContext, text: &str) -> PreparedLayout {
        paragraph(markdown_spans(text), TextAlign::TopLeft, LineHeight::default())
            .chunked_layout(ctx, true, 0)
            .expect("the note is chunked")
    }

    #[test]
    fn an_edit_lays_out_only_the_chunks_it_touched() {
        with_context(300.0, 1.0, |ctx| {
            let text = note(80);
            let first = super::chunks_laid_out();
            let before = chunked(ctx, &text);
            let all = super::chunks_laid_out() - first;
            assert!(all >= 8, "the note is cut into chunks: {all}");
            assert_same(&before, &whole_layout(ctx, &text), "the note");

            let edits: [(&str, Box<dyn Fn(&str) -> String>); 4] = [
                ("a character typed", Box::new(|text| {
                    let mut edited = text.to_owned();
                    edited.insert(text.len() / 2, 'x');
                    edited
                })),
                ("a line inserted", Box::new(|text| {
                    let at = text[text.len() / 3..].find('\n').unwrap() + text.len() / 3 + 1;
                    let mut edited = text.to_owned();
                    edited.insert_str(at, "a new line\n");
                    edited
                })),
                ("a line deleted", Box::new(|text| {
                    let from = text[text.len() / 4..].find('\n').unwrap() + text.len() / 4 + 1;
                    let to = from + text[from..].find('\n').unwrap() + 1;
                    format!("{}{}", &text[..from], &text[to..])
                })),
                ("a bold word closed", Box::new(|text| text.replacen("**bold**", "**bold*", 1))),
            ];
            for (what, edit) in edits {
                let edited = edit(&text);
                let laid_out = super::chunks_laid_out();
                let after = chunked(ctx, &edited);
                let redone = super::chunks_laid_out() - laid_out;
                assert!(redone <= 4, "{what}: {redone} of {all} chunks were laid out again");
                assert_same(&after, &whole_layout(ctx, &edited), what);
                // Back to the text before: nothing is laid out again that still is there.
                let laid_out = super::chunks_laid_out();
                let _ = chunked(ctx, &text);
                assert!(super::chunks_laid_out() - laid_out <= 4, "{what}: undoing it");
            }
        });
    }

    #[test]
    fn a_chunk_keeps_its_layout_when_the_width_is_the_same_but_not_when_it_changes() {
        let text = note(40);
        with_context(300.0, 1.0, |ctx| {
            let _ = chunked(ctx, &text);
            let laid_out = super::chunks_laid_out();
            let _ = chunked(ctx, &text);
            assert_eq!(super::chunks_laid_out(), laid_out, "the same width reuses every chunk");
        });
        with_context(220.0, 1.0, |ctx| {
            let laid_out = super::chunks_laid_out();
            let narrow = chunked(ctx, &text);
            assert!(super::chunks_laid_out() > laid_out, "another width is laid out again");
            assert_same(&narrow, &whole_layout(ctx, &text), "narrower");
        });
    }

    fn empty_layout() -> Rc<PreparedLayout> {
        Rc::new(PreparedLayout {
            fragments: Vec::new(),
            decorations: Vec::new(),
            graphemes: Vec::new(),
            paint_runs: Vec::new(),
            backgrounds: Vec::new(),
            line_breaks: Vec::new(),
            line_heights: Vec::new(),
            size: ResolvedSize { width: 0.0, height: 0.0 },
            aimer_interaction: None,
        })
    }

    #[test]
    fn the_cache_drops_its_least_recently_used_chunks_past_its_byte_budget() {
        use super::{ChunkCache, ChunkKey, layout_bytes};

        // A layout with a cluster per character, as a chunk of text has.
        let layout_of = |characters: usize| {
            let mut layout = Rc::try_unwrap(empty_layout()).ok().expect("a fresh layout");
            let cluster = aimer_cupid::text_layout::TextCluster {
                text_range: 0..1,
                line_index: 0,
                level: unicode_bidi::Level::ltr(),
                start_x: 0.0,
                end_x: 1.0,
                start_y: 0.0,
                end_y: 0.0,
                y: 0.0,
                height: 1.0,
            };
            layout.aimer_interaction = Some(Rc::new(aimer_cupid::text_layout::TextInteractionLayout {
                text: String::new(),
                lines: Vec::new(),
                clusters: vec![cluster; characters],
                metrics: aimer_cupid::text_layout::ParagraphMetrics {
                    width: 0.0,
                    height: 0.0,
                    ascent: 0.0,
                    descent: 0.0,
                    line_gap: 0.0,
                    line_height: 0.0,
                    line_count: 0,
                },
                origin_x: 0.0,
                origin_y: 0.0,
                writing_mode: aimer_cupid::text_layout::TextWritingMode::HorizontalTb,
            }));
            Rc::new(layout)
        };
        let key = |content: u64| ChunkKey { content, len: 1, runs: 1, paragraph: 0 };
        let one = layout_bytes(&layout_of(100));
        assert!(one >= 100 * std::mem::size_of::<aimer_cupid::text_layout::TextCluster>());

        // Room for ten chunks of this size.
        let mut cache = ChunkCache::with_budget(10 * one);
        for content in 0..10 {
            cache.insert(key(content), layout_of(100));
        }
        assert_eq!(cache.entries.len(), 10, "ten fit");
        // Keep the oldest in use, then add one more than fits.
        assert!(cache.get(&key(0)).is_some());
        cache.insert(key(10), layout_of(100));

        assert!(cache.bytes <= 10 * one, "the budget holds: {} of {}", cache.bytes, 10 * one);
        assert!(cache.get(&key(10)).is_some(), "the chunk just added stays");
        assert!(cache.get(&key(0)).is_some(), "so does the one in use");
        assert!(cache.get(&key(1)).is_none(), "the least recently used goes first");
        assert_eq!(
            cache.bytes,
            cache.entries.values().map(|entry| entry.bytes).sum::<usize>(),
            "the bytes counted are the bytes held"
        );

        // A chunk larger than the whole budget is still kept, alone.
        let mut small = ChunkCache::with_budget(one / 2);
        small.insert(key(1), layout_of(100));
        small.insert(key(2), layout_of(100));
        assert_eq!(small.entries.len(), 1);
        assert!(small.get(&key(2)).is_some());
    }

    #[test]
    fn a_scope_chunks_paragraphs_the_usual_threshold_would_lay_out_whole() {
        let text = note(12);
        assert!(text.len() < super::CHUNKING_MIN_BYTES, "{}", text.len());
        with_context(300.0, 1.0, |ctx| {
            let paragraph = || paragraph(markdown_spans(&text), TextAlign::TopLeft, LineHeight::default());
            let laid_out = super::chunks_laid_out();
            let whole = paragraph().prepare(ctx);
            assert_eq!(super::chunks_laid_out(), laid_out, "a short paragraph is laid out whole");

            let chunked = super::with_chunking_min_bytes(0, || paragraph().prepare(ctx));
            assert!(super::chunks_laid_out() > laid_out, "in the scope it is chunked");
            assert_same(&chunked, &whole, "the same layout either way");

            // And the scope ends.
            let after = super::chunks_laid_out();
            let _ = paragraph().prepare(ctx);
            assert_eq!(super::chunks_laid_out(), after);
        });
    }

    #[test]
    fn paragraphs_that_are_not_chunked_say_so() {
        with_context(300.0, 1.0, |ctx| {
            // A single span has the shared shaper's geometry, and is laid out whole.
            let single = paragraph(
                vec![ResolvedTextSpan::plain(Rc::from("one\ntwo\n".repeat(400)), base())],
                TextAlign::TopLeft,
                LineHeight::default(),
            );
            assert!(single.chunked_layout(ctx, true, 0).is_none());
            // Text that does not wrap is as wide as its widest line, and one that is centred or set
            // to the right is placed by that width, which no chunk knows.
            for align in [TextAlign::TopCenter, TextAlign::BotRight] {
                let placed = clipped(spans(60, false), align, LineHeight::default());
                assert!(placed.chunked_layout(ctx, true, 0).is_none(), "{align:?}");
            }
            // Short text is not worth it.
            let short = paragraph(spans(60, false), TextAlign::TopLeft, LineHeight::default());
            assert!(short.chunked_layout(ctx, true, usize::MAX).is_none());
        });
    }
}
