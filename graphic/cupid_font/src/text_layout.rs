use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;

use unicode_bidi::BidiInfo;
use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

use super::glyph_metrics::GlyphMetrics;
use super::glyph_rasterizer::{GlyphKey, GlyphPreparationContext, GlyphRasterizer, ShapedRunGlyph};
use crate::font::{FontFamily, FontStyle, FontWeight, TextLanguage};
use crate::unicode_script::Script;

pub type FontId = u32;

#[derive(Default)]
struct UnicodeLayoutScratch {
    cluster_plans: Vec<UnicodeClusterPlan>,
    can_break_before: Vec<bool>,
    shaped_run_cache: Vec<ShapedRunCacheEntry>,
}

#[derive(Clone, Copy)]
struct UnicodeClusterPlan {
    start: usize,
    end: usize,
    script: Option<Script>,
    font_id: Option<FontId>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct ShapeRunCacheKey {
    hash: u64,
    byte_len: usize,
    font_id: FontId,
    script: Option<Script>,
}

struct ShapedRunCacheEntry {
    key: ShapeRunCacheKey,
    /// Byte range in the current paragraph. The cache is cleared before each
    /// paragraph, so no borrowed text or lifetime parameter is needed.
    start: usize,
    end: usize,
    glyphs: Box<[ShapedRunGlyph]>,
}

const SHAPED_RUN_CACHE_CAPACITY: usize = 64;

#[inline]
fn shape_run_cache_hash(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

aimer_tls::thread_local! {
    static UNICODE_LAYOUT_SCRATCH: RefCell<UnicodeLayoutScratch> =
        RefCell::new(UnicodeLayoutScratch::default());
}

/// The writing direction used by the text layout pipeline.
///
/// `HorizontalTb` is the usual left-to-right, top-to-bottom layout. `VerticalRl`
/// places glyphs from top to bottom and starts each new column to the left of
/// the previous column, matching traditional CJK vertical writing. In vertical
/// mode a request's `bounds_height` is the column extent and `bounds_width` is
/// the available column area.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum TextWritingMode {
    /// Horizontal lines progress from left to right and then downward.
    #[default]
    HorizontalTb,
    /// Vertical columns progress from top to bottom and then right to left.
    VerticalRl,
}

impl TextWritingMode {
    /// Returns whether text runs in vertical columns rather than horizontal lines.
    #[inline]
    pub const fn is_vertical(self) -> bool {
        matches!(self, Self::VerticalRl)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    pub line_height: f32,
}

impl FontMetrics {
    pub fn from_rasterizer(rasterizer: &GlyphRasterizer, font_size: f32) -> Self {
        let (ascent, descent, line_gap) = rasterizer.line_metrics(font_size);
        Self::new(ascent, descent, line_gap)
    }

    pub fn new(ascent: f32, descent: f32, line_gap: f32) -> Self {
        Self {
            ascent,
            descent,
            line_gap,
            line_height: ascent - descent + line_gap,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLayoutOptions {
    pub origin_x: f32,
    pub origin_y: f32,
    pub max_width: f32,
    pub max_height: f32,
    pub font_size: f32,
    pub ellipsis: bool,
    /// Writing mode used by the paragraph layout.
    pub writing_mode: TextWritingMode,
}

impl TextLayoutOptions {
    pub fn new(font_size: f32, origin_x: f32, origin_y: f32, max_width: f32) -> Self {
        Self {
            origin_x,
            origin_y,
            max_width,
            max_height: 0.0,
            font_size,
            ellipsis: false,
            writing_mode: TextWritingMode::HorizontalTb,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionedShapedGlyph {
    pub font_id: FontId,
    pub glyph_id: u16,
    pub cluster: usize,
    pub text_range: std::ops::Range<usize>,
    pub x: f32,
    pub y: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub advance: f32,
    /// Vertical pen advance, negative for top-to-bottom shaping.
    pub y_advance: f32,
    pub font_size: f32,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub text_range: std::ops::Range<usize>,
    pub level: unicode_bidi::Level,
    pub font_id: FontId,
    pub glyph_range: std::ops::Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    pub text_range: std::ops::Range<usize>,
    pub glyph_range: std::ops::Range<usize>,
    pub baseline: f32,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    pub hard_break: bool,
}

/// One source cluster and its visual caret edges in a laid-out paragraph.
///
/// `text_range` is a UTF-8 byte range in [`ParagraphLayout::text`]. The range
/// is never split while wrapping, and `start_x`/`end_x` are the caret
/// positions at the logical start and end of that range. Consequently an RTL
/// cluster may have `start_x > end_x`. In vertical mode the corresponding
/// caret progression is `start_y`/`end_y` while `start_x == end_x` is the
/// column coordinate. The cluster order in a paragraph is visual order, while
/// the source range remains logical order.
#[derive(Clone, Debug, PartialEq)]
pub struct TextCluster {
    /// The logical UTF-8 byte range represented by this cluster.
    pub text_range: std::ops::Range<usize>,
    /// The visual line containing this cluster.
    pub line_index: usize,
    /// The resolved UAX #9 level for this cluster.
    pub level: unicode_bidi::Level,
    /// The caret x-coordinate at the logical start of the cluster, or the
    /// column coordinate in vertical mode.
    pub start_x: f32,
    /// The caret x-coordinate at the logical end of the cluster, or the same
    /// column coordinate in vertical mode.
    pub end_x: f32,
    /// The caret y-coordinate at the logical start of the cluster. Horizontal
    /// layout keeps this equal to the line baseline for symmetry with the
    /// vertical interaction path.
    pub start_y: f32,
    /// The caret y-coordinate at the logical end of the cluster.
    pub end_y: f32,
    /// The top of the line's selection/caret band.
    pub y: f32,
    /// The line's selection/caret band height.
    pub height: f32,
}

/// The caret band for a valid source boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretGeometry {
    /// The logical UTF-8 byte offset represented by this caret.
    pub offset: usize,
    /// The visual line containing the caret.
    pub line_index: usize,
    /// The caret's x-coordinate.
    pub x: f32,
    /// The top of the caret band.
    pub y: f32,
    /// The caret width. Horizontal text uses zero width; vertical text uses
    /// the width of the column's caret band.
    pub width: f32,
    /// The caret band height.
    pub height: f32,
}

/// One visual rectangle occupied by a source selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionRect {
    /// The visual line containing this rectangle.
    pub line_index: usize,
    /// The rectangle's left edge.
    pub x: f32,
    /// The rectangle's top edge.
    pub y: f32,
    /// The rectangle width.
    pub width: f32,
    /// The rectangle height.
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphMetrics {
    pub width: f32,
    pub height: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    pub line_height: f32,
    pub line_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphLayout {
    /// The original paragraph text used by source ranges.
    pub text: String,
    /// Glyphs in visual line order.
    pub glyphs: Vec<PositionedShapedGlyph>,
    /// Lines and their source/glyph ranges.
    pub lines: Vec<TextLine>,
    /// Bidi/font runs in visual paint order.
    pub runs: Vec<TextRun>,
    /// Source clusters in visual order with logical caret edges.
    pub clusters: Vec<TextCluster>,
    /// Aggregate paragraph metrics.
    pub metrics: ParagraphMetrics,
    /// Horizontal origin used while laying out the paragraph.
    pub origin_x: f32,
    /// Baseline origin used for the first line.
    pub origin_y: f32,
    /// Writing mode used to position the paragraph.
    pub writing_mode: TextWritingMode,
}

impl ParagraphLayout {
    /// Returns the caret geometry for a UTF-8 source boundary.
    ///
    /// Offsets inside a shaped cluster are rejected so callers cannot place a
    /// caret in the middle of a grapheme or ligature. The returned coordinate
    /// is in the same logical coordinate space as the paragraph's glyphs.
    pub fn caret_geometry(&self, offset: usize) -> Option<CaretGeometry> {
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return None;
        }

        if let Some(cluster) = self
            .clusters
            .iter()
            .find(|cluster| cluster.text_range.start == offset)
        {
            let (x, y, width, height) = self.caret_coordinates(cluster, true);
            return Some(CaretGeometry {
                offset,
                line_index: cluster.line_index,
                x,
                y,
                width,
                height,
            });
        }

        if let Some(cluster) = self
            .clusters
            .iter()
            .find(|cluster| cluster.text_range.end == offset)
        {
            let (x, y, width, height) = self.caret_coordinates(cluster, false);
            return Some(CaretGeometry {
                offset,
                line_index: cluster.line_index,
                x,
                y,
                width,
                height,
            });
        }

        if self.text.is_empty() && self.lines.len() == 1 {
            let line = &self.lines[0];
            return Some(CaretGeometry {
                offset,
                line_index: 0,
                x: if self.writing_mode.is_vertical() {
                    line.baseline
                } else {
                    self.origin_x
                },
                y: if self.writing_mode.is_vertical() {
                    self.origin_y
                } else {
                    line_top(line)
                },
                width: if self.writing_mode.is_vertical() {
                    (line.ascent - line.descent).max(0.0)
                } else {
                    0.0
                },
                height: if self.writing_mode.is_vertical() {
                    0.0
                } else {
                    line_height(line)
                },
            });
        }

        None
    }

    /// Maps a point to the nearest source cluster boundary.
    ///
    /// The hit test selects the visual half of a cluster first and then maps
    /// that edge back to its logical UTF-8 offset. This preserves intuitive
    /// movement through RTL runs while keeping grapheme and ligature clusters
    /// indivisible.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        if self.writing_mode.is_vertical() {
            return self.vertical_hit_test(x, y);
        }

        let line_index = self.line_index_at_y(y)?;
        let line = &self.lines[line_index];
        let mut first = None;
        let mut last = None;
        for cluster in &self.clusters {
            if cluster.line_index != line_index {
                continue;
            }
            if first.is_none() {
                first = Some(cluster);
            }
            last = Some(cluster);

            let left = cluster.start_x.min(cluster.end_x);
            let right = cluster.start_x.max(cluster.end_x);
            let midpoint = left + (right - left) / 2.0;
            if x < midpoint {
                return Some(if cluster.start_x <= cluster.end_x {
                    cluster.text_range.start
                } else {
                    cluster.text_range.end
                });
            }
        }

        let Some(first) = first else {
            return Some(line.text_range.start.min(self.text.len()));
        };
        let cluster = last.unwrap_or(first);
        Some(if cluster.start_x <= cluster.end_x {
            cluster.text_range.end
        } else {
            cluster.text_range.start
        })
    }

    /// Returns visual selection rectangles for a logical UTF-8 byte range.
    ///
    /// A range that intersects a cluster selects the complete cluster. The
    /// result is split at line and bidi-run gaps, and adjacent visual pieces
    /// on one line are coalesced.
    pub fn selection_rects(&self, selection: std::ops::Range<usize>) -> Vec<SelectionRect> {
        let start = selection.start.min(selection.end).min(self.text.len());
        let end = selection.start.max(selection.end).min(self.text.len());
        if start == end {
            return Vec::new();
        }

        if self.writing_mode.is_vertical() {
            return self.vertical_selection_rects(start..end);
        }

        let mut result: Vec<SelectionRect> = Vec::new();
        for cluster in &self.clusters {
            if cluster.text_range.start >= end || cluster.text_range.end <= start {
                continue;
            }

            let left = cluster.start_x.min(cluster.end_x);
            let right = cluster.start_x.max(cluster.end_x);
            let rect = SelectionRect {
                line_index: cluster.line_index,
                x: left,
                y: cluster.y,
                width: right - left,
                height: cluster.height,
            };
            if let Some(previous) = result.last_mut()
                && previous.line_index == rect.line_index
                && (previous.x + previous.width - rect.x).abs() < 0.001
                && (previous.y - rect.y).abs() < 0.001
                && (previous.height - rect.height).abs() < 0.001
            {
                let right = (previous.x + previous.width).max(rect.x + rect.width);
                previous.x = previous.x.min(rect.x);
                previous.width = right - previous.x;
            } else {
                result.push(rect);
            }
        }
        result
    }

    fn line_index_at_y(&self, y: f32) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }

        let mut nearest = 0;
        let mut nearest_distance = f32::INFINITY;
        for (index, line) in self.lines.iter().enumerate() {
            let top = line_top(line);
            let bottom = top + line_height(line);
            if y >= top && y <= bottom {
                return Some(index);
            }
            let distance = if y < top { top - y } else { y - bottom };
            if distance < nearest_distance {
                nearest = index;
                nearest_distance = distance;
            }
        }
        Some(nearest)
    }

    fn caret_coordinates(&self, cluster: &TextCluster, start: bool) -> (f32, f32, f32, f32) {
        if self.writing_mode.is_vertical() {
            let (top, bottom) = if start {
                (cluster.start_y, cluster.end_y)
            } else {
                (cluster.end_y, cluster.start_y)
            };
            (
                cluster.start_x,
                top.min(bottom),
                cluster.height.max(0.0),
                0.0,
            )
        } else if start {
            (cluster.start_x, cluster.y, 0.0, cluster.height)
        } else {
            (cluster.end_x, cluster.y, 0.0, cluster.height)
        }
    }

    fn vertical_hit_test(&self, x: f32, y: f32) -> Option<usize> {
        let line_index = self.line_index_at_x(x)?;
        let line = &self.lines[line_index];
        let mut first = None;
        let mut last = None;
        for cluster in &self.clusters {
            if cluster.line_index != line_index {
                continue;
            }
            if first.is_none() {
                first = Some(cluster);
            }
            last = Some(cluster);
            let top = cluster.start_y.min(cluster.end_y);
            let bottom = cluster.start_y.max(cluster.end_y);
            let midpoint = top + (bottom - top) / 2.0;
            if y < midpoint {
                return Some(if cluster.start_y <= cluster.end_y {
                    cluster.text_range.start
                } else {
                    cluster.text_range.end
                });
            }
        }

        let Some(first) = first else {
            return Some(line.text_range.start.min(self.text.len()));
        };
        let cluster = last.unwrap_or(first);
        Some(if cluster.start_y <= cluster.end_y {
            cluster.text_range.end
        } else {
            cluster.text_range.start
        })
    }

    fn vertical_selection_rects(&self, selection: std::ops::Range<usize>) -> Vec<SelectionRect> {
        let mut result = Vec::new();
        for cluster in &self.clusters {
            if cluster.text_range.start >= selection.end
                || cluster.text_range.end <= selection.start
            {
                continue;
            }
            let top = cluster.start_y.min(cluster.end_y);
            let bottom = cluster.start_y.max(cluster.end_y);
            result.push(SelectionRect {
                line_index: cluster.line_index,
                x: cluster.start_x,
                y: top,
                width: cluster.height,
                height: bottom - top,
            });
        }
        result.sort_by(|left, right| {
            left.line_index
                .cmp(&right.line_index)
                .then_with(|| left.y.total_cmp(&right.y))
        });
        result
    }

    fn line_index_at_x(&self, x: f32) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let line_width = self.lines.first().map_or(0.0, |line| line_height(line));
        let mut nearest = 0;
        let mut nearest_distance = f32::INFINITY;
        for (index, line) in self.lines.iter().enumerate() {
            let left = line.baseline.min(line.baseline + line_width);
            let right = line.baseline.max(line.baseline + line_width);
            if x >= left && x <= right {
                return Some(index);
            }
            let distance = if x < left { left - x } else { x - right };
            if distance < nearest_distance {
                nearest = index;
                nearest_distance = distance;
            }
        }
        Some(nearest)
    }
}

/// A positioned glyph ready for rendering.
#[derive(Clone)]
pub struct PositionedGlyph {
    pub codepoint: char,
    pub glyph_key: GlyphKey,
    pub line_index: usize,
    pub line_x: f32,
    pub advance: f32,
    /// Screen-space X of the glyph quad's top-left corner.
    pub x: f32,
    /// Screen-space Y of the glyph quad's top-left corner.
    pub y: f32,
    pub width: u32,
    pub height: u32,
    pub font_size: f32,
}

/// One glyph of a shaped cluster, carrying everything positioning needs.
///
/// Shaping is the width-independent stage of the text pipeline: its result is
/// cached across frames and survives a window resize, while positioning runs
/// again for every new wrapping width. The pixel box measured by the
/// rasterizer — bitmap size and the offsets from the pen and the baseline —
/// is a pure function of the glyph key, so it is measured once here, at
/// shaping time, instead of being looked up per glyph on every re-wrap.
/// Positioning is thereby pure arithmetic over this struct: it touches no
/// rasterizer, no cache and no lock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
    pub key: GlyphKey,
    /// Horizontal advance of the pen after this glyph.
    pub advance: f32,
    /// Vertical advance of the pen after this glyph. Horizontal layout keeps
    /// this at zero; vertical shaping uses it for top-to-bottom progression.
    pub y_advance: f32,
    /// Horizontal shaping offset relative to the pen position.
    pub x_offset: f32,
    /// Vertical shaping offset relative to the baseline.
    pub y_offset: f32,
    /// Bitmap width in pixels; zero for blank glyphs such as spaces.
    pub width: u32,
    /// Bitmap height in pixels; zero for blank glyphs such as spaces.
    pub height: u32,
    /// Offset from the pen position to the left edge of the bitmap.
    pub offset_x: f32,
    /// Offset from the baseline to the bottom edge of the bitmap.
    pub offset_y: f32,
}

#[derive(Clone)]
pub struct ShapedCluster {
    pub text: String,
    /// The logical UTF-8 byte range represented by this grapheme or ligature.
    pub text_range: std::ops::Range<usize>,
    /// The resolved bidi level at the start of this logical cluster.
    pub level: unicode_bidi::Level,
    pub base_codepoint: char,
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
    /// Whether a line may start at this cluster, per UAX #14.
    ///
    /// Scripts without word spaces — Han, Hiragana, Katakana — break between
    /// almost any two characters, while a space-only wrapper would have to
    /// rewind to the last space it saw, possibly a whole line back. Carrying
    /// the opportunity on the cluster keeps the rule table out of the layout
    /// loop: it is computed once while shaping, which is cached, and read as a
    /// single bool per cluster while positioning, which is not.
    pub can_break_before: bool,
}

#[derive(Clone)]
pub struct ShapedText {
    /// The source text from which this shaping result was produced.
    pub text: String,
    pub font_size: f32,
    /// Font ascent used for the first-line baseline.
    pub ascent: f32,
    /// Font descent, kept negative to match [`FontMetrics`].
    pub descent: f32,
    /// Font line gap included between successive baselines.
    pub line_gap: f32,
    pub line_height: f32,
    /// Width of the widest hard-break-separated line, in pixels, or the
    /// top-to-bottom extent of the widest vertical column.
    ///
    /// This is the width the text occupies when nothing wraps: positioning at
    /// any wrapping width the text fits in produces the same glyphs as
    /// positioning unbounded. The pipeline uses this to share one cached
    /// layout across every such width, so a window resize re-wraps only the
    /// text that actually wraps.
    pub max_line_width: f32,
    /// Writing mode for which glyph advances and substitutions were shaped.
    pub writing_mode: TextWritingMode,
    pub clusters: Vec<ShapedCluster>,
}

/// Interaction geometry retained beside the production Aimer layout.
///
/// The renderer consumes the positioned glyphs while selection and editing
/// consume this source-aware view. Both are produced from the same shaped
/// advances, so a ligature, fallback face, or wrapped line cannot acquire a
/// second, approximate caret geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInteractionLayout {
    /// The logical source text used by all byte ranges.
    pub text: String,
    /// Lines in visual paint order.
    pub lines: Vec<TextLine>,
    /// Source clusters in logical order for the current production layout.
    pub clusters: Vec<TextCluster>,
    /// Aggregate metrics in the same coordinate space as the glyphs.
    pub metrics: ParagraphMetrics,
    /// Horizontal origin used for the layout.
    pub origin_x: f32,
    /// Baseline origin of the first line.
    pub origin_y: f32,
    /// Writing mode used to interpret caret, hit-test, and selection geometry.
    pub writing_mode: TextWritingMode,
}

impl TextInteractionLayout {
    /// Returns the caret geometry for a valid UTF-8 source boundary.
    ///
    /// Offsets inside a grapheme or ligature are rejected. This keeps cursor
    /// movement and selection anchored to the same cluster boundaries used by
    /// the shaper.
    pub fn caret_geometry(&self, offset: usize) -> Option<CaretGeometry> {
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return None;
        }

        if let Some(cluster) = self
            .clusters
            .iter()
            .find(|cluster| cluster.text_range.start == offset)
        {
            let (x, y, width, height) = self.caret_coordinates(cluster, true);
            return Some(CaretGeometry {
                offset,
                line_index: cluster.line_index,
                x,
                y,
                width,
                height,
            });
        }
        if let Some(cluster) = self
            .clusters
            .iter()
            .find(|cluster| cluster.text_range.end == offset)
        {
            let (x, y, width, height) = self.caret_coordinates(cluster, false);
            return Some(CaretGeometry {
                offset,
                line_index: cluster.line_index,
                x,
                y,
                width,
                height,
            });
        }

        if self.text.is_empty() && self.lines.len() == 1 {
            let line = &self.lines[0];
            return Some(CaretGeometry {
                offset,
                line_index: 0,
                x: if self.writing_mode.is_vertical() {
                    line.baseline
                } else {
                    self.origin_x
                },
                y: if self.writing_mode.is_vertical() {
                    self.origin_y
                } else {
                    line_top(line)
                },
                width: if self.writing_mode.is_vertical() {
                    (line.ascent - line.descent).max(0.0)
                } else {
                    0.0
                },
                height: if self.writing_mode.is_vertical() {
                    0.0
                } else {
                    line_height(line)
                },
            });
        }
        None
    }

    /// Maps a point to the nearest source cluster boundary.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        if self.writing_mode.is_vertical() {
            return self.vertical_hit_test(x, y);
        }

        let line_index = self.line_index_at_y(y)?;
        let line = &self.lines[line_index];
        let mut first = None;
        let mut last = None;
        for cluster in &self.clusters {
            if cluster.line_index != line_index {
                continue;
            }
            let left = cluster.start_x.min(cluster.end_x);
            let right = cluster.start_x.max(cluster.end_x);
            if (right - left).abs() <= f32::EPSILON {
                continue;
            }
            if first.is_none() {
                first = Some(cluster);
            }
            last = Some(cluster);
            let midpoint = left + (right - left) / 2.0;
            if x < midpoint {
                return Some(if cluster.start_x <= cluster.end_x {
                    cluster.text_range.start
                } else {
                    cluster.text_range.end
                });
            }
        }

        let Some(first) = first else {
            return Some(line.text_range.start.min(self.text.len()));
        };
        let cluster = last.unwrap_or(first);
        Some(if cluster.start_x <= cluster.end_x {
            cluster.text_range.end
        } else {
            cluster.text_range.start
        })
    }

    /// Returns visual selection rectangles for a logical UTF-8 byte range.
    pub fn selection_rects(&self, selection: std::ops::Range<usize>) -> Vec<SelectionRect> {
        let start = selection.start.min(selection.end).min(self.text.len());
        let end = selection.start.max(selection.end).min(self.text.len());
        if start == end {
            return Vec::new();
        }

        if self.writing_mode.is_vertical() {
            return self.vertical_selection_rects(start..end);
        }

        let mut result: Vec<SelectionRect> = Vec::new();
        for cluster in &self.clusters {
            if cluster.text_range.start >= end || cluster.text_range.end <= start {
                continue;
            }
            let left = cluster.start_x.min(cluster.end_x);
            let right = cluster.start_x.max(cluster.end_x);
            let width = if self.is_hard_break(cluster) {
                1.0
            } else {
                right - left
            };
            let rect = SelectionRect {
                line_index: cluster.line_index,
                x: left,
                y: cluster.y,
                width,
                height: cluster.height,
            };
            result.push(rect);
        }
        result.sort_by(|left, right| {
            left.line_index
                .cmp(&right.line_index)
                .then_with(|| left.x.total_cmp(&right.x))
        });
        let mut merged: Vec<SelectionRect> = Vec::with_capacity(result.len());
        for rect in result {
            if let Some(previous) = merged.last_mut()
                && previous.line_index == rect.line_index
                && (previous.x + previous.width - rect.x).abs() < 0.01
                && (previous.y - rect.y).abs() < 0.001
                && (previous.height - rect.height).abs() < 0.001
            {
                let right = (previous.x + previous.width).max(rect.x + rect.width);
                previous.x = previous.x.min(rect.x);
                previous.width = right - previous.x;
            } else {
                merged.push(rect);
            }
        }
        merged
    }

    /// Finds the visual line nearest to a y-coordinate.
    pub fn line_index_at_y(&self, y: f32) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let mut nearest = 0;
        let mut nearest_distance = f32::INFINITY;
        for (index, line) in self.lines.iter().enumerate() {
            let top = line_top(line);
            let bottom = top + line_height(line);
            if y >= top && y <= bottom {
                return Some(index);
            }
            let distance = if y < top { top - y } else { y - bottom };
            if distance < nearest_distance {
                nearest = index;
                nearest_distance = distance;
            }
        }
        Some(nearest)
    }

    fn is_hard_break(&self, cluster: &TextCluster) -> bool {
        self.text
            .get(cluster.text_range.clone())
            .is_some_and(|text| text == "\n" || text == "\r\n")
    }

    fn caret_coordinates(&self, cluster: &TextCluster, start: bool) -> (f32, f32, f32, f32) {
        if self.writing_mode.is_vertical() {
            let (top, bottom) = if start {
                (cluster.start_y, cluster.end_y)
            } else {
                (cluster.end_y, cluster.start_y)
            };
            (
                cluster.start_x,
                top.min(bottom),
                cluster.height.max(0.0),
                0.0,
            )
        } else if start {
            (cluster.start_x, cluster.y, 0.0, cluster.height)
        } else {
            (cluster.end_x, cluster.y, 0.0, cluster.height)
        }
    }

    fn vertical_hit_test(&self, x: f32, y: f32) -> Option<usize> {
        let line_index = self.line_index_at_x(x)?;
        let line = &self.lines[line_index];
        let mut first = None;
        let mut last = None;
        for cluster in &self.clusters {
            if cluster.line_index != line_index {
                continue;
            }
            if first.is_none() {
                first = Some(cluster);
            }
            last = Some(cluster);
            let top = cluster.start_y.min(cluster.end_y);
            let bottom = cluster.start_y.max(cluster.end_y);
            let midpoint = top + (bottom - top) / 2.0;
            if y < midpoint {
                return Some(if cluster.start_y <= cluster.end_y {
                    cluster.text_range.start
                } else {
                    cluster.text_range.end
                });
            }
        }

        let Some(first) = first else {
            return Some(line.text_range.start.min(self.text.len()));
        };
        let cluster = last.unwrap_or(first);
        Some(if cluster.start_y <= cluster.end_y {
            cluster.text_range.end
        } else {
            cluster.text_range.start
        })
    }

    fn vertical_selection_rects(&self, selection: std::ops::Range<usize>) -> Vec<SelectionRect> {
        let mut result = Vec::new();
        for cluster in &self.clusters {
            if cluster.text_range.start >= selection.end
                || cluster.text_range.end <= selection.start
            {
                continue;
            }
            let top = cluster.start_y.min(cluster.end_y);
            let bottom = cluster.start_y.max(cluster.end_y);
            result.push(SelectionRect {
                line_index: cluster.line_index,
                x: cluster.start_x,
                y: top,
                width: cluster.height,
                height: bottom - top,
            });
        }
        result.sort_by(|left, right| {
            left.line_index
                .cmp(&right.line_index)
                .then_with(|| left.y.total_cmp(&right.y))
        });

        let mut merged: Vec<SelectionRect> = Vec::with_capacity(result.len());
        for rect in result {
            if let Some(previous) = merged.last_mut()
                && previous.line_index == rect.line_index
                && (previous.x - rect.x).abs() < 0.001
                && (previous.width - rect.width).abs() < 0.001
                && (previous.y + previous.height - rect.y).abs() < 0.01
            {
                previous.height =
                    (previous.y + previous.height).max(rect.y + rect.height) - previous.y;
            } else {
                merged.push(rect);
            }
        }
        merged
    }

    fn line_index_at_x(&self, x: f32) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let line_width = self.metrics.line_height.max(0.0);
        let mut nearest = 0;
        let mut nearest_distance = f32::INFINITY;
        for (index, line) in self.lines.iter().enumerate() {
            let left = line.baseline.min(line.baseline + line_width);
            let right = line.baseline.max(line.baseline + line_width);
            if x >= left && x <= right {
                return Some(index);
            }
            let distance = if x < left { left - x } else { x - right };
            if distance < nearest_distance {
                nearest = index;
                nearest_distance = distance;
            }
        }
        Some(nearest)
    }
}

/// The complete positioned result retained by the production layout cache.
///
/// `glyphs` remains the renderer-facing representation while `interaction` is
/// the source-aware representation consumed by caret, hit-test, and selection
/// code. The option is `None` only for compatibility entries inserted by older
/// callers that have no shaped source attached.
#[derive(Clone)]
pub struct PositionedTextLayout {
    pub glyphs: Vec<PositionedGlyph>,
    pub interaction: Option<TextInteractionLayout>,
}

impl From<Vec<PositionedGlyph>> for PositionedTextLayout {
    fn from(glyphs: Vec<PositionedGlyph>) -> Self {
        Self {
            glyphs,
            interaction: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextHorizontalAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Returns how far each line must shift right to sit at `alignment` inside a
/// box `bounds_width` wide.
///
/// The offset is never negative: a line at least as wide as the box, and every
/// left-aligned line, gets `0.0`.
pub fn line_alignment_offsets(
    line_widths: &[f32],
    bounds_width: f32,
    alignment: TextHorizontalAlign,
) -> Vec<f32> {
    line_widths
        .iter()
        .map(|line_width| {
            let remaining = (bounds_width - line_width).max(0.0);
            match alignment {
                TextHorizontalAlign::Left => 0.0,
                TextHorizontalAlign::Center => remaining / 2.0,
                TextHorizontalAlign::Right => remaining,
            }
        })
        .collect()
}

/// Returns the width of each line, indexed by `line_index`.
///
/// A line's width is the farthest `line_x + advance` reached by any of its
/// glyphs. The result is empty for no glyphs, and a line with no glyphs reports
/// `0.0`.
pub fn positioned_line_widths(glyphs: &[PositionedGlyph]) -> Vec<f32> {
    let line_count = glyphs
        .iter()
        .map(|glyph| glyph.line_index)
        .max()
        .map_or(0, |last| last + 1);
    let mut widths = vec![0.0_f32; line_count];
    for glyph in glyphs {
        widths[glyph.line_index] = widths[glyph.line_index].max(glyph.line_x + glyph.advance);
    }
    widths
}

pub fn layout_paragraph_with_shaper(
    text: &str,
    font_bytes: &[u8],
    font_id: FontId,
    metrics: FontMetrics,
    options: TextLayoutOptions,
) -> ParagraphLayout {
    let state = super::aimer_font::AimerFontState::from_font_data(
        super::font_resolver::FontData::Shared(Arc::from(font_bytes)),
        0,
    )
    .ok();
    layout_paragraph_inner(
        text,
        font_id,
        &metrics,
        &options,
        |segment, text_offset, level, script, writing_mode| {
            shape_segment_with_aimer(
                state.as_ref(),
                segment,
                text_offset,
                font_id,
                options.font_size,
                level,
                script,
                writing_mode,
            )
        },
    )
}

/// Byte-indexed table of the line break opportunities in `text`, where index
/// `i` reports whether a line may start at byte `i`.
///
/// The table follows the Unicode line breaking algorithm (UAX #14), so it
/// keeps punctuation attached to the text it belongs to — a line never starts
/// with `,`, `!`, `」` or `，` — and it allows breaks between ideographs, which
/// is the only way text without word spaces can wrap at all.
///
/// The table is indexed by byte offset and therefore has `text.len() + 1`
/// entries; the trailing entry marks the mandatory break at the end of the
/// paragraph and the leading one is always `false`, since a paragraph start is
/// not a break opportunity.
pub fn line_break_opportunities(text: &str) -> Vec<bool> {
    let mut allowed = Vec::new();
    line_break_opportunities_into(text, &mut allowed);
    allowed
}

fn line_break_opportunities_into(text: &str, allowed: &mut Vec<bool>) {
    allowed.clear();
    allowed.resize(text.len() + 1, false);
    for (offset, _) in linebreaks(text) {
        allowed[offset] = true;
    }
    allowed[0] = false;
}

/// The script a grapheme cluster belongs to, or `None` when it belongs to no
/// script in particular.
///
/// Spaces, digits and most punctuation are `Script::Common`, and combining
/// marks that adopt the script of their base are `Script::Inherited`; neither
/// identifies a writing system, so both are reported as `None` and normally
/// left to the run they are surrounded by. Complex-script runs keep them out
/// of their shaping buffer when they sit at a script boundary.
fn cluster_script(cluster: &str) -> Option<Script> {
    cluster
        .chars()
        .find_map(|codepoint| match Script::for_codepoint(codepoint) {
            Script::Common | Script::Inherited | Script::Unknown => None,
            script => Some(script),
        })
}

/// Reports whether `cluster` may join a shaping run of script `run_script`,
/// adopting the run's script when it did not have one yet.
///
/// A shaping buffer carries exactly one script, guessed from its first strong
/// character, and that script selects the shaping engine. Appending Khmer to a
/// run opened by Latin text would therefore shape the Khmer with the default
/// engine: COENG would survive as a visible sign instead of pulling the
/// following consonant under its base as a subscript leg. Runs must end at
/// script boundaries for the same reason they end at font boundaries.
fn extends_script_run(run_script: &mut Option<Script>, cluster: &str) -> bool {
    extends_script_value(run_script, cluster_script(cluster))
}

fn extends_script_value(run_script: &mut Option<Script>, script: Option<Script>) -> bool {
    let Some(script) = script else {
        return true;
    };
    match run_script {
        Some(current) => *current == script,
        None => {
            *run_script = Some(script);
            true
        }
    }
}

/// Complex-script shapers must receive only their script's grapheme clusters.
///
/// Common characters are useful context for Latin shaping, but passing a
/// leading or trailing space/punctuation cluster through an owned Khmer/Indic
/// shaper can change contextual substitutions. Keep those clusters in a
/// neighboring neutral run and let the complex run start and end on a strong
/// script cluster.
#[inline]
fn script_requires_neutral_boundary(script: Option<Script>) -> bool {
    matches!(
        script,
        Some(
            Script::Arabic
                | Script::Bengali
                | Script::Devanagari
                | Script::Gurmukhi
                | Script::Gujarati
                | Script::Oriya
                | Script::Tamil
                | Script::Telugu
                | Script::Kannada
                | Script::Malayalam
                | Script::Sinhala
                | Script::Thai
                | Script::Lao
                | Script::Khmer
                | Script::Myanmar
        )
    )
}

#[inline]
fn is_shaping_control_cluster(cluster: &str) -> bool {
    !cluster.is_empty()
        && cluster
            .chars()
            .all(|codepoint| matches!(codepoint, '\u{200b}' | '\u{200c}' | '\u{200d}'))
}

#[inline]
fn starts_with_line_attached_ascii_punctuation(text: &str) -> bool {
    text.chars()
        .find(|codepoint| !codepoint.is_ascii_whitespace())
        .is_some_and(|codepoint| {
            matches!(
                codepoint,
                ',' | '.' | ':' | ';' | '!' | '?' | ')' | ']' | '}'
            )
        })
}

/// A contiguous run of text that shares the same BiDi level and script, and can
/// be shaped as a unit.
#[derive(Clone)]
struct ShapingRun<'a> {
    text: &'a str,
    start: usize,
    level: unicode_bidi::Level,
    script: Option<Script>,
}

/// Collect grapheme clusters into shaping runs: contiguous clusters that share
/// the same BiDi level and script are merged into a single run so that
/// complex-script shaping (Arabic, Devanagari, etc.) operates on the full
/// context instead of individual clusters.
/// Collects feature-path runs in the order in which they are painted.
///
/// `BidiInfo::visual_runs` already performs UAX #9 rule L2 at the level-run
/// granularity.  Script boundaries are finer than bidi boundaries, however,
/// so each visual level run is split again before shaping.  RTL script runs
/// are reversed as a group while their text remains in logical order; this is
/// the order required by the shaper and preserves the source byte ranges used
/// by selection and hit testing.
fn collect_shaping_runs<'a>(
    text: &'a str,
    _bidi: &BidiInfo,
    visual_runs: &[(std::ops::Range<usize>, unicode_bidi::Level)],
) -> Vec<ShapingRun<'a>> {
    let mut result = Vec::new();

    for (range, level) in visual_runs {
        let graphemes: Vec<(usize, &str)> = text[range.clone()].grapheme_indices(true).collect();
        let mut logical_runs = Vec::new();
        let mut grapheme_index = 0;

        while grapheme_index < graphemes.len() {
            let (relative_start, cluster) = graphemes[grapheme_index];
            let start = range.start + relative_start;

            if cluster == "\n" {
                logical_runs.push(ShapingRun {
                    text: cluster,
                    start,
                    level: *level,
                    script: None,
                });
                grapheme_index += 1;
                continue;
            }

            let run_start_index = grapheme_index;
            let mut script = cluster_script(cluster);
            let leading_neutral_run = script.is_none();
            grapheme_index += 1;
            while grapheme_index < graphemes.len() {
                let next_cluster = graphemes[grapheme_index].1;
                if next_cluster == "\n" {
                    break;
                }
                if leading_neutral_run && cluster_script(next_cluster).is_some() {
                    // Common/inherited clusters before a strong script must
                    // not become that script's leading context. A Khmer
                    // shaper, for example, may inspect the beginning of its
                    // buffer when forming a coeng leg; feeding it the space
                    // and slash before `សួស្តី` changes the selected form.
                    break;
                }
                if script_requires_neutral_boundary(script)
                    && cluster_script(next_cluster).is_none()
                    && !is_shaping_control_cluster(next_cluster)
                {
                    break;
                }
                if !extends_script_run(&mut script, next_cluster) {
                    break;
                }
                grapheme_index += 1;
            }

            let (_, last_cluster) = graphemes[grapheme_index - 1];
            let end = range.start + graphemes[grapheme_index - 1].0 + last_cluster.len();
            logical_runs.push(ShapingRun {
                text: &text[start..end],
                start,
                level: *level,
                script,
            });

            debug_assert!(grapheme_index > run_start_index);
        }

        if level.is_rtl() {
            // Paragraph separators are boundaries, not visual content. Keep
            // them in place while reversing the runs on either side.
            let mut segment_start = 0;
            for index in 0..logical_runs.len() {
                if logical_runs[index].text != "\n" {
                    continue;
                }
                result.extend(logical_runs[segment_start..index].iter().rev().cloned());
                result.push(logical_runs[index].clone());
                segment_start = index + 1;
            }
            result.extend(logical_runs[segment_start..].iter().rev().cloned());
        } else {
            result.extend(logical_runs);
        }
    }

    result
}

#[cfg(test)]
fn layout_paragraph<F>(
    text: &str,
    font_id: FontId,
    metrics: &FontMetrics,
    options: &TextLayoutOptions,
    mut shape: F,
) -> ParagraphLayout
where
    F: FnMut(&str, usize, TextWritingMode) -> Vec<PositionedShapedGlyph>,
{
    layout_paragraph_inner(
        text,
        font_id,
        metrics,
        options,
        |segment, offset, _, _, writing_mode| shape(segment, offset, writing_mode),
    )
}

fn layout_paragraph_inner<F>(
    text: &str,
    font_id: FontId,
    metrics: &FontMetrics,
    options: &TextLayoutOptions,
    mut shape: F,
) -> ParagraphLayout
where
    F: FnMut(
        &str,
        usize,
        unicode_bidi::Level,
        Option<Script>,
        TextWritingMode,
    ) -> Vec<PositionedShapedGlyph>,
{
    if options.writing_mode.is_vertical() {
        return layout_paragraph_vertical_inner(text, font_id, metrics, options, &mut shape);
    }

    let bidi = BidiInfo::new(text, None);
    // `visual_runs` returns a byte-indexed level vector plus visual ranges.
    // Pair each range with the level at its first byte. Keep every paragraph
    // separate because UAX #9 resolves direction per paragraph.
    let visual_run_ranges: Vec<(std::ops::Range<usize>, unicode_bidi::Level)> = bidi
        .paragraphs
        .iter()
        .flat_map(|para| {
            let (levels, ranges) = bidi.visual_runs(para, para.range.clone());
            ranges.into_iter().map(move |range| {
                let level = levels.get(range.start).copied().unwrap_or(para.level);
                (range, level)
            })
        })
        .collect();
    #[allow(clippy::unnecessary_filter_map)]
    let mut break_offsets: Vec<usize> = linebreaks(text)
        .filter_map(|(offset, opportunity)| match opportunity {
            BreakOpportunity::Mandatory | BreakOpportunity::Allowed => Some(offset),
        })
        .collect();
    break_offsets.push(text.len());
    break_offsets.sort_unstable();
    break_offsets.dedup();

    // Collect shaping runs (merged by BiDi level) before the layout loop.
    let shaping_runs = collect_shaping_runs(text, &bidi, &visual_run_ranges);

    let mut glyphs: Vec<PositionedShapedGlyph> = Vec::new();
    let mut runs = Vec::new();
    let mut lines = Vec::new();
    let mut line_start_text = 0;
    let mut line_start_glyph = 0;
    let mut line_width = 0.0;
    let mut baseline = options.origin_y;
    let max_width = options.max_width.max(0.0);
    let max_height = options.max_height.max(0.0);

    // Use a queue so remainder runs from word-wrapping are re-evaluated for
    // overflow on subsequent lines.  A plain `for` loop would emit the
    // remainder once and `continue`, skipping the overflow check — causing
    // long words to render past the second line's edge.
    let mut queue: VecDeque<ShapingRun<'_>> = shaping_runs.into_iter().collect();

    while let Some(shaping_run) = queue.pop_front() {
        let run_start = shaping_run.start;
        let run_text = shaping_run.text;
        let run_end = run_start + run_text.len();
        let level = shaping_run.level;
        let script = shaping_run.script;
        let is_rtl = level.is_rtl();

        // Handle newline runs.
        if run_text == "\n" {
            finish_line(
                &mut lines,
                line_start_text..run_start,
                line_start_glyph..glyphs.len(),
                baseline,
                line_width,
                metrics,
                true,
            );
            line_start_text = run_end;
            line_start_glyph = glyphs.len();
            line_width = 0.0;
            baseline += metrics.line_height;
            if should_stop_for_height(options.origin_y, baseline, metrics.line_height, max_height) {
                break;
            }
            continue;
        }

        // Shape the entire run at once (correct for Arabic, Devanagari, etc.).
        let mut shaped = shape(run_text, run_start, level, script, options.writing_mode);

        // For RTL runs, reverse the glyph order so they render right-to-left.
        if is_rtl {
            shaped.reverse();
        }

        // Determine total advance for this shaped run.
        let run_width: f32 = shaped.iter().map(|g| g.advance).sum();

        // Check whether a line break is allowed at the run boundary.
        let break_allowed = break_offsets.binary_search(&run_end).is_ok();
        let keep_punctuation_with_previous =
            script.is_none() && starts_with_line_attached_ascii_punctuation(run_text);

        if max_width > 0.0
            && line_width + run_width > max_width
            && (break_allowed || !run_text.chars().all(char::is_whitespace))
            && !keep_punctuation_with_previous
        {
            // Try to break the run at grapheme-cluster boundaries to avoid
            // splitting across lines at awkward positions.  We walk clusters
            // and emit them onto the current line until we'd overflow, then
            // start a new line for the remainder.
            //
            // `sub_x > options.origin_x` ensures we never split before placing
            // at least one cluster (avoids an infinite wrapping loop on a single
            // wide cluster that can never fit).
            let mut sub_x = options.origin_x + line_width;
            let mut remainder_start: Option<usize> = None;
            let mut last_word_break: Option<usize> = None;
            let mut cluster_offset = 0usize;
            for (_, cluster_str) in run_text.grapheme_indices(true) {
                // Each cluster contributes its share of the total advance.
                // We can't re-shape individual clusters without losing context,
                // so we approximate by summing the shaped glyphs whose cluster
                // index falls inside this cluster's byte range.
                let cluster_byte_start = run_start + cluster_offset;
                let cluster_byte_end = cluster_byte_start + cluster_str.len();
                let cluster_advance: f32 = shaped
                    .iter()
                    .filter(|g| g.cluster >= cluster_byte_start && g.cluster < cluster_byte_end)
                    .map(|g| g.advance)
                    .sum();

                // Track the last cluster a line is allowed to start at (UAX
                // #14).  A break sits *after* the space that separates two
                // words, so breaking here keeps that space — whose advance is
                // already part of `sub_x` — on the line it terminates.
                if break_offsets.binary_search(&cluster_byte_start).is_ok() {
                    last_word_break = Some(cluster_byte_start);
                }

                if sub_x + cluster_advance > options.origin_x + max_width
                    && sub_x > options.origin_x
                {
                    // Prefer breaking at the last break opportunity rather
                    // than mid-word.  If the line holds none, fall back to the
                    // cluster-level break.
                    //
                    // A break at the very start of the run only makes progress
                    // while the line already holds something; on an empty line
                    // it would re-queue the same run forever, so the
                    // cluster-level break takes over.
                    let candidate = last_word_break.unwrap_or(cluster_byte_start);
                    remainder_start = Some(if candidate == run_start && line_width <= 0.0 {
                        cluster_byte_start
                    } else {
                        candidate
                    });
                    break;
                }
                sub_x += cluster_advance;
                cluster_offset += cluster_str.len();
            }

            if let Some(break_point) = remainder_start {
                // Emit glyphs up to break_point onto the current line; the
                // next line starts at break_point, so everything from there on
                // is left for the remainder.
                let glyph_start = glyphs.len();
                let line_glyphs: Vec<_> = shaped
                    .iter()
                    .filter(|g| g.cluster < break_point)
                    .cloned()
                    .collect();
                // Track accumulated width for per-glyph positioning so the
                // space lands after the preceding characters, not at the
                // line's start x.
                let mut acc_w = 0.0_f32;
                for mut glyph in line_glyphs {
                    glyph.x = options.origin_x + line_width + acc_w;
                    glyph.y = baseline;
                    acc_w += glyph.advance;
                    glyphs.push(glyph);
                }
                let text_end = break_point;
                runs.push(TextRun {
                    text_range: run_start..text_end,
                    level,
                    font_id,
                    glyph_range: glyph_start..glyphs.len(),
                });
                let line_run_width = acc_w;
                finish_line(
                    &mut lines,
                    line_start_text..text_end,
                    line_start_glyph..glyphs.len(),
                    baseline,
                    line_run_width + line_width,
                    metrics,
                    false,
                );

                // Skip leading whitespace at the start of the new line so
                // wrapped lines don't begin with a space character.
                let mut trimmed = text_end;
                while trimmed < run_end && text.as_bytes()[trimmed] == b' ' {
                    trimmed += 1;
                }

                // Start a new line with the trimmed remainder.
                line_start_text = trimmed;
                line_start_glyph = glyphs.len();
                line_width = 0.0;
                baseline += metrics.line_height;
                if should_stop_for_height(
                    options.origin_y,
                    baseline,
                    metrics.line_height,
                    max_height,
                ) {
                    break;
                }

                // Push the remainder back to the queue so it is re-shaped
                // and checked for overflow on the new line (fixes second-line
                // word wrapping).  We do NOT emit remainder glyphs here — the
                // queue iteration will emit them via the normal path below,
                // avoiding double-emission.
                if trimmed < run_end {
                    queue.push_front(ShapingRun {
                        text: &text[trimmed..run_end],
                        start: trimmed,
                        level,
                        script,
                    });
                }
                continue;
            } else if line_width > 0.0 {
                // Couldn't split — move the whole run onto a new line.  An
                // already empty line has nothing to move the run away from, so
                // the run stays where it is and simply overflows instead of
                // opening a blank line above itself.
                finish_line(
                    &mut lines,
                    line_start_text..run_start,
                    line_start_glyph..glyphs.len(),
                    baseline,
                    line_width,
                    metrics,
                    false,
                );
                line_start_text = run_start;
                line_start_glyph = glyphs.len();
                line_width = 0.0;
                baseline += metrics.line_height;
                if should_stop_for_height(
                    options.origin_y,
                    baseline,
                    metrics.line_height,
                    max_height,
                ) {
                    break;
                }
            }
        }

        let glyph_start = glyphs.len();
        let mut glyph_x = line_width;
        for mut glyph in shaped {
            glyph.x += options.origin_x + glyph_x;
            glyph.y += baseline;
            glyph_x += glyph.advance;
            glyphs.push(glyph);
        }
        line_width += run_width;
        runs.push(TextRun {
            text_range: run_start..run_end,
            level,
            font_id,
            glyph_range: glyph_start..glyphs.len(),
        });
    }

    if line_start_text <= text.len() && (lines.is_empty() || line_start_text < text.len()) {
        finish_line(
            &mut lines,
            line_start_text..text.len(),
            line_start_glyph..glyphs.len(),
            baseline,
            line_width,
            metrics,
            false,
        );
    }

    if options.ellipsis && max_width > 0.0 {
        apply_ellipsis(&mut glyphs, &mut lines, font_id, options, metrics);
    }

    let clusters = build_text_clusters(&glyphs, &lines, &runs);
    let width = lines.iter().map(|line| line.width).fold(0.0, f32::max);
    let line_count = lines.len();
    // line_height includes one line_gap per line, but line_gap only appears
    // *between* lines — subtract the trailing one so the reported height
    // matches the actual rendered extent (first-line ascent through last-line
    // descent).
    let height = line_count as f32 * metrics.line_height - metrics.line_gap;
    ParagraphLayout {
        text: text.to_string(),
        glyphs,
        lines,
        runs,
        clusters,
        metrics: ParagraphMetrics {
            width,
            height,
            ascent: metrics.ascent,
            descent: metrics.descent,
            line_gap: metrics.line_gap,
            line_height: metrics.line_height,
            line_count,
        },
        origin_x: options.origin_x,
        origin_y: options.origin_y,
        writing_mode: options.writing_mode,
    }
}

fn layout_paragraph_vertical_inner<F>(
    text: &str,
    _font_id: FontId,
    metrics: &FontMetrics,
    options: &TextLayoutOptions,
    shape: &mut F,
) -> ParagraphLayout
where
    F: FnMut(
        &str,
        usize,
        unicode_bidi::Level,
        Option<Script>,
        TextWritingMode,
    ) -> Vec<PositionedShapedGlyph>,
{
    let bidi = BidiInfo::new(text, None);
    let visual_run_ranges: Vec<(std::ops::Range<usize>, unicode_bidi::Level)> = bidi
        .paragraphs
        .iter()
        .flat_map(|para| {
            let (levels, ranges) = bidi.visual_runs(para, para.range.clone());
            ranges.into_iter().map(move |range| {
                let level = levels.get(range.start).copied().unwrap_or(para.level);
                (range, level)
            })
        })
        .collect();
    let shaping_runs = collect_shaping_runs(text, &bidi, &visual_run_ranges);
    let mut queue: VecDeque<ShapingRun<'_>> = shaping_runs.into_iter().collect();
    let break_opportunities = line_break_opportunities(text);

    let mut glyphs: Vec<PositionedShapedGlyph> = Vec::new();
    let mut runs = Vec::new();
    let mut lines = Vec::new();
    let mut line_start_text = 0usize;
    let mut line_start_glyph = 0usize;
    let mut line_extent = 0.0_f32;
    let mut line_index = 0usize;
    let mut pen_y = options.origin_y;
    let column_width = metrics.line_height.max(options.font_size).max(1.0);
    let mut column_x = vertical_column_x(
        options.origin_x,
        options.max_width.max(0.0),
        column_width,
        line_index,
    );
    let max_height = options.max_height.max(0.0);

    while let Some(shaping_run) = queue.pop_front() {
        let run_start = shaping_run.start;
        let run_text = shaping_run.text;
        let run_end = run_start + run_text.len();
        let level = shaping_run.level;
        let script = shaping_run.script;

        if run_text == "\n" {
            finish_line(
                &mut lines,
                line_start_text..run_start,
                line_start_glyph..glyphs.len(),
                column_x,
                line_extent,
                metrics,
                true,
            );
            line_start_text = run_end;
            line_start_glyph = glyphs.len();
            line_extent = 0.0;
            line_index += 1;
            pen_y = options.origin_y;
            column_x = vertical_column_x(
                options.origin_x,
                options.max_width.max(0.0),
                column_width,
                line_index,
            );
            continue;
        }

        let shaped = shape(run_text, run_start, level, script, options.writing_mode);
        let mut run_text_start = run_start;
        let mut run_glyph_start = glyphs.len();
        let mut previous_cluster = None;

        for glyph in shaped {
            let advance = (-glyph.y_advance).max(0.0);
            let is_new_cluster = previous_cluster != Some(glyph.cluster);
            let can_break_before = is_new_cluster
                && break_opportunities
                    .get(glyph.cluster)
                    .copied()
                    .unwrap_or(false);
            if can_break_before
                && max_height > 0.0
                && line_extent + advance > max_height
                && line_extent > 0.0
            {
                if run_glyph_start < glyphs.len() {
                    runs.push(TextRun {
                        text_range: run_text_start..glyph.cluster,
                        level,
                        font_id: glyphs[run_glyph_start].font_id,
                        glyph_range: run_glyph_start..glyphs.len(),
                    });
                }
                finish_line(
                    &mut lines,
                    line_start_text..glyph.cluster,
                    line_start_glyph..glyphs.len(),
                    column_x,
                    line_extent,
                    metrics,
                    false,
                );
                line_start_text = glyph.cluster;
                line_start_glyph = glyphs.len();
                run_text_start = glyph.cluster;
                run_glyph_start = glyphs.len();
                line_extent = 0.0;
                line_index += 1;
                pen_y = options.origin_y;
                column_x = vertical_column_x(
                    options.origin_x,
                    options.max_width.max(0.0),
                    column_width,
                    line_index,
                );
            }

            previous_cluster = Some(glyph.cluster);
            let glyph_font_id = glyph.font_id;
            let glyph_cluster = glyph.cluster;
            let glyph_y = pen_y + glyph.y_offset;
            glyphs.push(PositionedShapedGlyph {
                font_id: glyph_font_id,
                glyph_id: glyph.glyph_id,
                cluster: glyph_cluster,
                text_range: glyph.text_range,
                x: column_x + glyph.x_offset,
                y: glyph_y,
                x_offset: glyph.x_offset,
                y_offset: glyph.y_offset,
                advance: 0.0,
                y_advance: glyph.y_advance,
                font_size: glyph.font_size,
                source: glyph.source,
            });
            line_extent += advance;
            pen_y += advance;
        }

        if run_glyph_start < glyphs.len() {
            runs.push(TextRun {
                text_range: run_text_start..run_end,
                level,
                font_id: glyphs[run_glyph_start].font_id,
                glyph_range: run_glyph_start..glyphs.len(),
            });
        }
    }

    if line_start_text <= text.len() && (lines.is_empty() || line_start_text < text.len()) {
        finish_line(
            &mut lines,
            line_start_text..text.len(),
            line_start_glyph..glyphs.len(),
            column_x,
            line_extent,
            metrics,
            false,
        );
    }

    let clusters = build_vertical_text_clusters(&glyphs, &lines, &runs);
    let width = lines.len() as f32 * column_width;
    let height = lines.iter().map(|line| line.width).fold(0.0, f32::max);
    let line_count = lines.len();
    ParagraphLayout {
        text: text.to_string(),
        glyphs,
        lines,
        runs,
        clusters,
        metrics: ParagraphMetrics {
            width,
            height,
            ascent: metrics.ascent,
            descent: metrics.descent,
            line_gap: metrics.line_gap,
            line_height: metrics.line_height,
            line_count,
        },
        origin_x: options.origin_x,
        origin_y: options.origin_y,
        writing_mode: options.writing_mode,
    }
}

fn build_vertical_text_clusters(
    glyphs: &[PositionedShapedGlyph],
    lines: &[TextLine],
    runs: &[TextRun],
) -> Vec<TextCluster> {
    let mut clusters = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        let start = line.glyph_range.start.min(glyphs.len());
        let end = line.glyph_range.end.min(glyphs.len());
        let mut glyph_index = start;
        while glyph_index < end {
            let source_range = glyphs[glyph_index].text_range.clone();
            let mut cluster_end = glyph_index + 1;
            while cluster_end < end && glyphs[cluster_end].text_range == source_range {
                cluster_end += 1;
            }

            if source_range.start != source_range.end {
                let mut top = f32::INFINITY;
                let mut bottom = f32::NEG_INFINITY;
                let column_x = glyphs[glyph_index].x - glyphs[glyph_index].x_offset;
                for glyph in &glyphs[glyph_index..cluster_end] {
                    let pen_y = glyph.y - glyph.y_offset;
                    let glyph_end = pen_y + (-glyph.y_advance).max(0.0);
                    top = top.min(pen_y.min(glyph_end));
                    bottom = bottom.max(pen_y.max(glyph_end));
                }

                if top.is_finite() && bottom.is_finite() {
                    let level = runs
                        .iter()
                        .find(|run| {
                            run.glyph_range.start <= glyph_index
                                && glyph_index < run.glyph_range.end
                        })
                        .map_or_else(unicode_bidi::Level::ltr, |run| run.level);
                    let (start_y, end_y) = if level.is_rtl() {
                        (bottom, top)
                    } else {
                        (top, bottom)
                    };
                    clusters.push(TextCluster {
                        text_range: source_range,
                        line_index,
                        level,
                        start_x: column_x,
                        end_x: column_x,
                        start_y,
                        end_y,
                        y: top,
                        height: line_height(line),
                    });
                }
            }
            glyph_index = cluster_end;
        }
    }
    clusters
}

fn line_height(line: &TextLine) -> f32 {
    (line.ascent - line.descent + line.line_gap).max(0.0)
}

/// Top of a line's box. The first baseline sits `ascent + line_gap / 2` below it,
/// the same place the text painter puts it, so half the gap lies above the
/// ascent line.
fn line_top(line: &TextLine) -> f32 {
    line.baseline - line.ascent - line.line_gap * 0.5
}

fn build_text_clusters(
    glyphs: &[PositionedShapedGlyph],
    lines: &[TextLine],
    runs: &[TextRun],
) -> Vec<TextCluster> {
    let mut clusters = Vec::new();

    for (line_index, line) in lines.iter().enumerate() {
        let start = line.glyph_range.start.min(glyphs.len());
        let end = line.glyph_range.end.min(glyphs.len());
        let mut glyph_index = start;
        while glyph_index < end {
            let source_range = glyphs[glyph_index].text_range.clone();
            let mut cluster_end = glyph_index + 1;
            while cluster_end < end && glyphs[cluster_end].text_range == source_range {
                cluster_end += 1;
            }

            if source_range.start != source_range.end {
                let mut left = f32::INFINITY;
                let mut right = f32::NEG_INFINITY;
                for glyph in &glyphs[glyph_index..cluster_end] {
                    let advance_end = glyph.x + glyph.advance;
                    left = left.min(glyph.x.min(advance_end));
                    right = right.max(glyph.x.max(advance_end));
                }

                if left.is_finite() && right.is_finite() {
                    let level = runs
                        .iter()
                        .find(|run| {
                            run.glyph_range.start <= glyph_index
                                && glyph_index < run.glyph_range.end
                        })
                        .map_or_else(unicode_bidi::Level::ltr, |run| run.level);
                    let (start_x, end_x) = if level.is_rtl() {
                        (right, left)
                    } else {
                        (left, right)
                    };
                    clusters.push(TextCluster {
                        text_range: source_range,
                        line_index,
                        level,
                        start_x,
                        end_x,
                        start_y: line.baseline,
                        end_y: line.baseline,
                        y: line_top(line),
                        height: line_height(line),
                    });
                }
            }

            glyph_index = cluster_end;
        }
    }

    clusters
}

fn shape_segment_with_aimer(
    state: Option<&super::aimer_font::AimerFontState>,
    segment: &str,
    text_offset: usize,
    font_id: FontId,
    font_size: f32,
    _level: unicode_bidi::Level,
    _script: Option<Script>,
    writing_mode: TextWritingMode,
) -> Vec<PositionedShapedGlyph> {
    let Some(state) = state else {
        return fallback_shape_segment_with_writing_mode(
            segment,
            text_offset,
            font_id,
            font_size,
            writing_mode,
        );
    };
    let Ok(Some(shaped)) = state.shape_run_with_options(segment, None, writing_mode.is_vertical())
    else {
        return fallback_shape_segment_with_writing_mode(
            segment,
            text_offset,
            font_id,
            font_size,
            writing_mode,
        );
    };

    let upem = f32::from(shaped.units_per_em);
    let scale = if upem > 0.0 { font_size / upem } else { 1.0 };
    let mut cluster_starts: Vec<usize> = shaped.glyphs.iter().map(|glyph| glyph.cluster).collect();
    cluster_starts.sort_unstable();
    cluster_starts.dedup();

    let mut result = Vec::with_capacity(shaped.glyphs.len());
    for glyph in &shaped.glyphs {
        let cluster_start = glyph.cluster;
        let cluster_end = cluster_starts
            .iter()
            .copied()
            .find(|candidate| *candidate > cluster_start)
            .unwrap_or(segment.len());
        let cluster_end = if segment.is_char_boundary(cluster_end) {
            cluster_end
        } else {
            segment.len()
        };
        let cluster = text_offset + cluster_start;
        let text_range = text_offset + cluster_start..text_offset + cluster_end;
        result.push(PositionedShapedGlyph {
            font_id,
            glyph_id: glyph.glyph_id,
            cluster,
            text_range,
            x: 0.0,
            y: 0.0,
            x_offset: glyph.x_offset as f32 * scale,
            y_offset: glyph.y_offset as f32 * scale,
            advance: glyph.x_advance as f32 * scale,
            y_advance: glyph.y_advance as f32 * scale,
            font_size,
            source: segment[cluster_start..cluster_end].to_string(),
        });
    }
    super::aimer_font::recycle_shaped_glyphs(shaped.glyphs);
    result
}

fn fallback_shape_segment_with_writing_mode(
    segment: &str,
    text_offset: usize,
    font_id: FontId,
    font_size: f32,
    writing_mode: TextWritingMode,
) -> Vec<PositionedShapedGlyph> {
    // Group by grapheme cluster so that combining marks (e.g. "e\u{301}")
    // are emitted as a single glyph with the cluster's full byte range.
    segment
        .grapheme_indices(true)
        .map(|(cluster_byte_offset, cluster_str)| {
            let cluster_start = text_offset + cluster_byte_offset;
            let cluster_end = cluster_start + cluster_str.len();
            // Use the first (base) codepoint as the representative glyph id.
            let glyph_char = cluster_str.chars().next().unwrap_or('\0');
            PositionedShapedGlyph {
                font_id,
                glyph_id: glyph_char as u32 as u16,
                cluster: cluster_start,
                text_range: cluster_start..cluster_end,
                x: 0.0,
                y: 0.0,
                x_offset: 0.0,
                y_offset: 0.0,
                advance: if writing_mode.is_vertical() {
                    0.0
                } else {
                    font_size * 0.5
                },
                y_advance: if writing_mode.is_vertical() {
                    -font_size
                } else {
                    0.0
                },
                font_size,
                source: cluster_str.to_string(),
            }
        })
        .collect()
}

fn finish_line(
    lines: &mut Vec<TextLine>,
    text_range: std::ops::Range<usize>,
    glyph_range: std::ops::Range<usize>,
    baseline: f32,
    width: f32,
    metrics: &FontMetrics,
    hard_break: bool,
) {
    lines.push(TextLine {
        text_range,
        glyph_range,
        baseline,
        width,
        ascent: metrics.ascent,
        descent: metrics.descent,
        line_gap: metrics.line_gap,
        hard_break,
    });
}

fn should_stop_for_height(
    origin_y: f32,
    next_baseline: f32,
    line_height: f32,
    max_height: f32,
) -> bool {
    max_height > 0.0 && next_baseline - origin_y + line_height > max_height
}

fn apply_ellipsis(
    glyphs: &mut Vec<PositionedShapedGlyph>,
    lines: &mut [TextLine],
    font_id: FontId,
    options: &TextLayoutOptions,
    metrics: &FontMetrics,
) {
    if let Some(line) = lines.last_mut() {
        let ellipsis_width = options.font_size * 0.5;
        while line.width + ellipsis_width > options.max_width
            && line.glyph_range.end > line.glyph_range.start
        {
            if let Some(glyph) = glyphs.pop() {
                line.glyph_range.end -= 1;
                line.width -= glyph.advance;
                line.text_range.end = glyph.cluster;
            } else {
                break;
            }
        }
        let x = options.origin_x + line.width;
        glyphs.push(PositionedShapedGlyph {
            font_id,
            glyph_id: '…' as u32 as u16,
            cluster: line.text_range.end,
            text_range: line.text_range.end..line.text_range.end,
            x,
            y: line.baseline,
            x_offset: 0.0,
            y_offset: 0.0,
            advance: ellipsis_width,
            y_advance: 0.0,
            font_size: options.font_size,
            source: "…".to_string(),
        });
        line.glyph_range.end = glyphs.len();
        line.width += ellipsis_width;
        line.ascent = metrics.ascent;
    }
}

pub fn shape_text(rasterizer: &mut GlyphRasterizer, text: &str, font_size: f32) -> ShapedText {
    shape_text_styled(
        rasterizer,
        text,
        font_size,
        FontFamily::SANS_SERIF,
        FontWeight::Normal,
        FontStyle::Normal,
        None,
    )
}

fn simple_ltr_break_before<I>(breaks: &mut std::iter::Peekable<I>, offset: usize) -> bool
where
    I: Iterator<Item = (usize, BreakOpportunity)>,
{
    while let Some(&(break_offset, _)) = breaks.peek() {
        if break_offset < offset {
            breaks.next();
        } else if break_offset == offset {
            breaks.next();
            return true;
        } else {
            break;
        }
    }
    false
}

fn append_simple_ltr_line<I>(
    rasterizer: &mut GlyphRasterizer,
    text: &str,
    line_start: usize,
    line_end: usize,
    font_size: f32,
    font_id: FontId,
    font_weight: FontWeight,
    breaks: &mut std::iter::Peekable<I>,
    clusters: &mut Vec<ShapedCluster>,
) where
    I: Iterator<Item = (usize, BreakOpportunity)>,
{
    let cluster_output_start = clusters.len();
    let line_len = line_end - line_start;

    // Printable ASCII is one byte per grapheme. Build the owned output
    // clusters directly, without first materializing a grapheme-index table.
    for cluster_start in line_start..line_end {
        let codepoint = text.as_bytes()[cluster_start] as char;
        clusters.push(ShapedCluster {
            text: codepoint.to_string(),
            text_range: cluster_start..cluster_start + 1,
            level: unicode_bidi::Level::ltr(),
            base_codepoint: codepoint,
            glyphs: Vec::new(),
            width: 0.0,
            can_break_before: simple_ltr_break_before(breaks, cluster_start),
        });
    }

    if line_len == 0 {
        return;
    }

    let shaped_glyphs = rasterizer.shape_run_with_font_id_reusing(
        &text[line_start..line_end],
        font_size,
        font_id,
        font_weight,
    );
    if shaped_glyphs.is_empty() {
        rasterizer.recycle_shaped_run(shaped_glyphs);
        return;
    }

    // The owned shaper emits an LTR glyph stream with non-decreasing cluster offsets.
    // Consume equal offsets as one ligature/cluster group so no sorted
    // cluster-start allocation is needed on this path.
    debug_assert!(
        shaped_glyphs
            .windows(2)
            .all(|glyphs| glyphs[0].cluster <= glyphs[1].cluster)
    );

    let mut glyph_index = 0;
    let mut group_end = 0;
    let mut group_cluster_index = cluster_output_start;
    let mut group_text_end = line_end;
    rasterizer.with_metrics_for_shaped_glyphs(&shaped_glyphs, font_size, |metrics| {
        let glyph = &shaped_glyphs[glyph_index];
        if glyph_index == group_end {
            let relative_cluster = glyph.cluster.min(line_len - 1);
            group_cluster_index = cluster_output_start + relative_cluster;
            group_end = glyph_index + 1;
            while group_end < shaped_glyphs.len()
                && shaped_glyphs[group_end].cluster <= glyph.cluster
            {
                group_end += 1;
            }
            let next_cluster = shaped_glyphs
                .get(group_end)
                .map_or(line_len, |next| next.cluster.min(line_len));
            group_text_end = line_start + next_cluster;
        }

        let cluster = &mut clusters[group_cluster_index];
        cluster.text_range.end = cluster.text_range.end.max(group_text_end);
        cluster.width += glyph.advance;
        cluster.glyphs.push(ShapedGlyph {
            key: glyph.glyph_key,
            advance: glyph.advance,
            y_advance: glyph.y_advance,
            x_offset: glyph.x_offset,
            y_offset: glyph.y_offset,
            width: metrics.width,
            height: metrics.height,
            offset_x: metrics.offset_x,
            offset_y: metrics.offset_y,
        });
        glyph_index += 1;
    });
    debug_assert_eq!(glyph_index, shaped_glyphs.len());
    rasterizer.recycle_shaped_run(shaped_glyphs);
}

fn shape_simple_ltr_text(
    rasterizer: &mut GlyphRasterizer,
    text: &str,
    font_size: f32,
    font_id: FontId,
    font_weight: FontWeight,
    ascent: f32,
    descent: f32,
    line_gap: f32,
) -> ShapedText {
    #[cfg(test)]
    rasterizer.record_simple_ltr_path();

    let mut clusters = Vec::with_capacity(text.len());
    let mut breaks = linebreaks(text).peekable();
    let mut line_start = 0;

    while line_start < text.len() {
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |offset| line_start + offset);
        append_simple_ltr_line(
            rasterizer,
            text,
            line_start,
            line_end,
            font_size,
            font_id,
            font_weight,
            &mut breaks,
            &mut clusters,
        );

        if line_end == text.len() {
            break;
        }

        simple_ltr_break_before(&mut breaks, line_end);
        clusters.push(ShapedCluster {
            text: "\n".to_string(),
            text_range: line_end..line_end + 1,
            level: unicode_bidi::Level::ltr(),
            base_codepoint: '\n',
            glyphs: Vec::new(),
            width: 0.0,
            can_break_before: false,
        });
        line_start = line_end + 1;
    }

    let mut max_line_width = 0.0_f32;
    let mut line_width = 0.0_f32;
    for cluster in &clusters {
        if cluster.text == "\n" {
            line_width = 0.0;
        } else {
            line_width += cluster.width;
            max_line_width = max_line_width.max(line_width);
        }
    }

    ShapedText {
        text: text.to_string(),
        font_size,
        ascent,
        descent,
        line_gap,
        line_height: ascent - descent + line_gap,
        max_line_width,
        writing_mode: TextWritingMode::HorizontalTb,
        clusters,
    }
}

/// Shapes `text`, choosing faces for the run as a whole.
///
/// `language` names the language the run is written in, for the ideographs
/// whose face the characters alone cannot settle — see
/// [`GlyphRasterizer::begin_script_run`]. `None` leaves the run judged on its
/// own characters, which is all a caller that knows nothing about the text can
/// offer.
#[allow(clippy::too_many_arguments)]
pub fn shape_text_styled(
    rasterizer: &mut GlyphRasterizer,
    text: &str,
    font_size: f32,
    font_family: FontFamily,
    font_weight: FontWeight,
    font_style: FontStyle,
    language: Option<TextLanguage>,
) -> ShapedText {
    shape_text_styled_with_writing_mode(
        rasterizer,
        text,
        font_size,
        font_family,
        font_weight,
        font_style,
        language,
        TextWritingMode::HorizontalTb,
    )
}

/// Shapes `text` with an explicit horizontal or vertical writing mode.
#[allow(clippy::too_many_arguments)]
pub fn shape_text_styled_with_writing_mode(
    rasterizer: &mut GlyphRasterizer,
    text: &str,
    font_size: f32,
    font_family: FontFamily,
    font_weight: FontWeight,
    font_style: FontStyle,
    language: Option<TextLanguage>,
    writing_mode: TextWritingMode,
) -> ShapedText {
    // Which face an ideograph belongs to depends on the characters beside it, so
    // the whole paragraph is announced before a single one is resolved.
    rasterizer.begin_script_run(text, language);

    let (ascent, _descent, line_gap) =
        rasterizer.line_metrics_for_family(font_size, font_family, font_weight, font_style);
    let line_height = ascent - _descent + line_gap;

    let primary_ascii_font = (writing_mode == TextWritingMode::HorizontalTb
        && font_family == FontFamily::SANS_SERIF
        && text
            .bytes()
            .all(|byte| byte == b'\n' || (b' '..=b'~').contains(&byte))
        && rasterizer.primary_covers_printable_ascii())
    .then(|| rasterizer.primary_font_id());

    if let Some(font_id) = primary_ascii_font {
        let shaped = shape_simple_ltr_text(
            rasterizer,
            text,
            font_size,
            font_id,
            font_weight,
            ascent,
            _descent,
            line_gap,
        );
        rasterizer.end_script_run();
        return shaped;
    }

    let bidi = BidiInfo::new(text, None);

    let clusters = UNICODE_LAYOUT_SCRATCH.with(|scratch| {
        let scratch = &mut *scratch.borrow_mut();
        scratch.cluster_plans.clear();
        scratch.shaped_run_cache.clear();
        for (start, cluster) in text.grapheme_indices(true) {
            let end = start + cluster.len();
            let script = cluster_script(cluster);
            let font_id = if cluster.is_empty() || cluster == "\n" {
                None
            } else {
                primary_ascii_font.or_else(|| {
                    rasterizer.font_id_for_family_cluster(
                        cluster,
                        font_size,
                        font_family,
                        font_weight,
                        font_style,
                    )
                })
            };
            scratch.cluster_plans.push(UnicodeClusterPlan {
                start,
                end,
                script,
                font_id,
            });
        }
        line_break_opportunities_into(text, &mut scratch.can_break_before);

        let mut clusters = Vec::with_capacity(scratch.cluster_plans.len());
        let mut grapheme_index = 0;

        while grapheme_index < scratch.cluster_plans.len() {
            let cluster_plan = scratch.cluster_plans[grapheme_index];
            let cluster_start = cluster_plan.start;
            let cluster_end = cluster_plan.end;
            let cluster = &text[cluster_start..cluster_end];
            if cluster == "\n" {
                clusters.push(ShapedCluster {
                    text: cluster.to_string(),
                    text_range: cluster_start..cluster_end,
                    level: bidi
                        .levels
                        .get(cluster_start)
                        .copied()
                        .unwrap_or_else(unicode_bidi::Level::ltr),
                    base_codepoint: '\n',
                    glyphs: Vec::new(),
                    width: 0.0,
                    can_break_before: false,
                });
                grapheme_index += 1;
                continue;
            }

            let Some(font_id) = cluster_plan.font_id else {
                grapheme_index += 1;
                continue;
            };

            let run_start_index = grapheme_index;
            let mut run_script = cluster_plan.script;
            let leading_neutral_run = run_script.is_none();
            grapheme_index += 1;
            while grapheme_index < scratch.cluster_plans.len() {
                let next_plan = scratch.cluster_plans[grapheme_index];
                let next_start = next_plan.start;
                let next_end = next_plan.end;
                let next_cluster = &text[next_start..next_end];
                if next_cluster == "\n"
                    || next_plan.font_id != Some(font_id)
                    || (leading_neutral_run && next_plan.script.is_some())
                    || (script_requires_neutral_boundary(run_script)
                        && next_plan.script.is_none()
                        && !is_shaping_control_cluster(next_cluster))
                    || !extends_script_value(&mut run_script, next_plan.script)
                {
                    break;
                }
                grapheme_index += 1;
            }

            let run_plans = &scratch.cluster_plans[run_start_index..grapheme_index];
            let run_end = run_plans.last().map_or(cluster_start, |plan| plan.end);
            let run_text = &text[cluster_start..run_end];
            let cache_key = ShapeRunCacheKey {
                hash: shape_run_cache_hash(run_text),
                byte_len: run_text.len(),
                font_id,
                script: run_script,
            };
            let cached_run = scratch.shaped_run_cache.iter().position(|entry| {
                entry.key == cache_key && &text[entry.start..entry.end] == run_text
            });
            let shaped_glyphs = if let Some(cache_index) = cached_run {
                rasterizer
                    .reuse_shaped_run_from_slice(&scratch.shaped_run_cache[cache_index].glyphs)
            } else {
                let shaped_glyphs = rasterizer
                    .shape_run_with_font_id_reusing_with_options_and_script(
                        run_text,
                        font_size,
                        font_id,
                        font_weight,
                        language,
                        writing_mode.is_vertical(),
                        run_script,
                    );
                if !shaped_glyphs.is_empty()
                    && scratch.shaped_run_cache.len() < SHAPED_RUN_CACHE_CAPACITY
                {
                    scratch.shaped_run_cache.push(ShapedRunCacheEntry {
                        key: cache_key,
                        start: cluster_start,
                        end: run_end,
                        glyphs: shaped_glyphs.clone().into_boxed_slice(),
                    });
                }
                shaped_glyphs
            };
            let cluster_output_start = clusters.len();

            clusters.extend(run_plans.iter().map(|plan| {
                let cluster = &text[plan.start..plan.end];
                ShapedCluster {
                    text: cluster.to_string(),
                    text_range: plan.start..plan.end,
                    level: bidi
                        .levels
                        .get(plan.start)
                        .copied()
                        .unwrap_or_else(unicode_bidi::Level::ltr),
                    base_codepoint: cluster.chars().next().unwrap_or('\0'),
                    glyphs: Vec::new(),
                    width: 0.0,
                    can_break_before: scratch.can_break_before[plan.start],
                }
            }));

            if shaped_glyphs.is_empty() {
                rasterizer.recycle_shaped_run(shaped_glyphs);
                continue;
            }

            // The owned shaper emits a monotonic glyph-cluster stream for each
            // direction. Keep one cursor in the grapheme table and derive the
            // next logical boundary from the adjacent glyph group instead of
            // sorting and scanning a second cluster-start vector per glyph.
            let descending = shaped_glyphs
                .first()
                .zip(shaped_glyphs.last())
                .is_some_and(|(first, last)| first.cluster > last.cluster);
            debug_assert!(shaped_glyphs.windows(2).all(|glyphs| {
                if descending {
                    glyphs[0].cluster >= glyphs[1].cluster
                } else {
                    glyphs[0].cluster <= glyphs[1].cluster
                }
            }));

            let run_cluster_count = run_plans.len();
            let mut source_cluster_cursor = if descending { run_cluster_count - 1 } else { 0 };
            let mut glyph_index = 0;
            let mut group_end = 0;
            let mut previous_descending_cluster = None;
            let mut group_cluster_index = cluster_output_start;
            let mut group_text_end = run_end;
            let mut append_glyph =
                |glyph_index: usize, glyph: &ShapedRunGlyph, metrics: GlyphMetrics| {
                    if glyph_index == group_end {
                        if descending {
                            while source_cluster_cursor > 0
                                && run_plans[source_cluster_cursor].start - cluster_start
                                    > glyph.cluster
                            {
                                source_cluster_cursor -= 1;
                            }
                        } else {
                            while source_cluster_cursor + 1 < run_cluster_count
                                && run_plans[source_cluster_cursor + 1].start - cluster_start
                                    <= glyph.cluster
                            {
                                source_cluster_cursor += 1;
                            }
                        }
                        group_cluster_index = cluster_output_start + source_cluster_cursor;
                        group_end = glyph_index + 1;
                        while group_end < shaped_glyphs.len()
                            && shaped_glyphs[group_end].cluster == glyph.cluster
                        {
                            group_end += 1;
                        }
                        let relative_end = if descending {
                            previous_descending_cluster.unwrap_or(run_end - cluster_start)
                        } else {
                            shaped_glyphs
                                .get(group_end)
                                .map_or(run_end - cluster_start, |next| next.cluster)
                        };
                        group_text_end = (cluster_start + relative_end).min(run_end);
                        if descending {
                            previous_descending_cluster = Some(glyph.cluster);
                        }
                    }

                    let cluster = &mut clusters[group_cluster_index];
                    cluster.text_range.end = cluster.text_range.end.max(group_text_end);
                    cluster.width += if writing_mode.is_vertical() {
                        (-glyph.y_advance).max(0.0)
                    } else {
                        glyph.advance
                    };
                    cluster.glyphs.push(ShapedGlyph {
                        key: glyph.glyph_key,
                        advance: glyph.advance,
                        y_advance: glyph.y_advance,
                        x_offset: glyph.x_offset,
                        y_offset: glyph.y_offset,
                        width: metrics.width,
                        height: metrics.height,
                        offset_x: metrics.offset_x,
                        offset_y: metrics.offset_y,
                    });
                };
            if shaped_glyphs.len() == 1 {
                let glyph = &shaped_glyphs[0];
                let metrics = rasterizer.metrics_for_key(glyph.glyph_key, font_size);
                append_glyph(0, glyph, metrics);
            } else {
                rasterizer.with_metrics_for_shaped_glyphs(&shaped_glyphs, font_size, |metrics| {
                    let glyph = &shaped_glyphs[glyph_index];
                    append_glyph(glyph_index, glyph, metrics);
                    glyph_index += 1;
                });
                debug_assert_eq!(glyph_index, shaped_glyphs.len());
            }
            rasterizer.recycle_shaped_run(shaped_glyphs);
        }

        clusters
    });

    rasterizer.end_script_run();

    // The widest hard-break line, accumulated the way positioning accumulates
    // its pen: cluster by cluster, reset at every explicit newline.
    let mut max_line_width = 0.0_f32;
    let mut line_width = 0.0_f32;
    for cluster in &clusters {
        if cluster.text == "\n" {
            line_width = 0.0;
        } else {
            line_width += cluster.width;
            max_line_width = max_line_width.max(line_width);
        }
    }

    ShapedText {
        text: text.to_string(),
        font_size,
        ascent,
        descent: _descent,
        line_gap,
        line_height,
        max_line_width,
        writing_mode: writing_mode,
        clusters,
    }
}

/// Shapes a worker-local span with the requested writing mode.
#[allow(clippy::too_many_arguments)]
pub fn prepare_shaped_text_with_writing_mode(
    context: &mut GlyphPreparationContext,
    text: &str,
    font_size: f32,
    font_family: FontFamily,
    font_weight: FontWeight,
    font_style: FontStyle,
    language: Option<TextLanguage>,
    writing_mode: TextWritingMode,
) -> ShapedText {
    shape_text_styled_with_writing_mode(
        context.rasterizer_mut(),
        text,
        font_size,
        font_family,
        font_weight,
        font_style,
        language,
        writing_mode,
    )
}

pub fn layout_shaped_text(
    shaped_text: &ShapedText,
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
) -> Vec<PositionedGlyph> {
    if shaped_text.writing_mode.is_vertical() {
        return layout_vertical_shaped_text(shaped_text, origin_x, origin_y, max_width, max_width);
    }
    layout_horizontal_shaped_text(shaped_text, origin_x, origin_y, max_width)
}

/// Positions shaped text using independent horizontal and vertical bounds.
///
/// Horizontal requests wrap against `max_width`; vertical requests consume
/// `max_height` from top to bottom and create new right-to-left columns inside
/// `max_width`. A zero bound keeps the corresponding direction unbounded.
pub(crate) fn layout_shaped_text_with_bounds(
    shaped_text: &ShapedText,
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
    max_height: f32,
) -> Vec<PositionedGlyph> {
    if shaped_text.writing_mode.is_vertical() {
        layout_vertical_shaped_text(shaped_text, origin_x, origin_y, max_width, max_height)
    } else {
        layout_horizontal_shaped_text(shaped_text, origin_x, origin_y, max_width)
    }
}

fn layout_horizontal_shaped_text(
    shaped_text: &ShapedText,
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
) -> Vec<PositionedGlyph> {
    let font_size = shaped_text.font_size;
    let line_height = shaped_text.line_height;

    let mut glyphs: Vec<PositionedGlyph> = Vec::new();
    let mut pen_x = origin_x;
    let mut pen_y = origin_y;
    let mut line_index = 0;

    let mut last_break_glyph_idx: usize = usize::MAX;
    let mut last_break_pen_x: f32 = origin_x;

    for cluster in &shaped_text.clusters {
        if cluster.text == "\n" {
            pen_x = origin_x;
            pen_y += line_height;
            line_index += 1;
            last_break_glyph_idx = usize::MAX;
            continue;
        }

        if cluster.can_break_before {
            last_break_glyph_idx = glyphs.len();
            last_break_pen_x = pen_x;
        }

        if max_width > 0.0 && pen_x + cluster.width > origin_x + max_width && pen_x > origin_x {
            if last_break_glyph_idx != usize::MAX {
                let wrap_offset = last_break_pen_x - origin_x;

                let moved_width = pen_x - last_break_pen_x;
                for glyph in &mut glyphs[last_break_glyph_idx..] {
                    glyph.x -= wrap_offset;
                    glyph.line_x -= wrap_offset;
                    glyph.y += line_height;
                    glyph.line_index += 1;
                }

                pen_x = if last_break_glyph_idx < glyphs.len() {
                    origin_x + moved_width
                } else {
                    origin_x
                };
                pen_y += line_height;
                line_index += 1;
                last_break_glyph_idx = usize::MAX;
            } else {
                pen_x = origin_x;
                pen_y += line_height;
                line_index += 1;
            }
        }

        for glyph in &cluster.glyphs {
            if glyph.width > 0 && glyph.height > 0 {
                let gx = pen_x + glyph.offset_x + glyph.x_offset;

                let gy = pen_y - glyph.offset_y - glyph.height as f32 + glyph.y_offset;

                glyphs.push(PositionedGlyph {
                    codepoint: cluster.base_codepoint,
                    glyph_key: glyph.key,
                    line_index,
                    line_x: pen_x - origin_x,
                    advance: glyph.advance,
                    x: gx,
                    y: gy,
                    width: glyph.width,
                    height: glyph.height,
                    font_size,
                });
            }

            pen_x += glyph.advance;
        }
    }
    glyphs
}

fn layout_vertical_shaped_text(
    shaped_text: &ShapedText,
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
    max_height: f32,
) -> Vec<PositionedGlyph> {
    let font_size = shaped_text.font_size;
    let column_width = shaped_text.line_height.max(font_size).max(1.0);
    let max_height = max_height.max(0.0);
    let mut glyphs = Vec::new();
    let mut column_index = 0usize;
    let mut pen_y = origin_y;
    let mut column_x = vertical_column_x(origin_x, max_width, column_width, column_index);

    for cluster in &shaped_text.clusters {
        if cluster.text == "\n" {
            column_index += 1;
            pen_y = origin_y;
            column_x = vertical_column_x(origin_x, max_width, column_width, column_index);
            continue;
        }

        if cluster.can_break_before
            && max_height > 0.0
            && pen_y + cluster.width > origin_y + max_height
            && pen_y > origin_y
        {
            column_index += 1;
            pen_y = origin_y;
            column_x = vertical_column_x(origin_x, max_width, column_width, column_index);
        }

        let cluster_start_y = pen_y;
        for glyph in &cluster.glyphs {
            if glyph.width > 0 && glyph.height > 0 {
                let gx = column_x + glyph.offset_x + glyph.x_offset;
                let gy = pen_y - glyph.offset_y - glyph.height as f32 + glyph.y_offset;

                glyphs.push(PositionedGlyph {
                    codepoint: cluster.base_codepoint,
                    glyph_key: glyph.key,
                    line_index: column_index,
                    line_x: column_x - origin_x,
                    // `advance` is retained as the horizontal field for the
                    // renderer's existing data contract. Vertical progression
                    // is already reflected in the absolute y position.
                    advance: 0.0,
                    x: gx,
                    y: gy,
                    width: glyph.width,
                    height: glyph.height,
                    font_size,
                });
            }

            pen_y += (-glyph.y_advance).max(0.0);
        }

        // A cluster made only of marks can have no positive glyph advance. Its
        // measured width is still authoritative for wrapping and keeps the
        // next cluster from being placed in the wrong column.
        if pen_y == cluster_start_y {
            pen_y += cluster.width;
        }
    }

    glyphs
}

#[inline]
fn vertical_column_x(origin_x: f32, max_width: f32, column_width: f32, column_index: usize) -> f32 {
    if max_width > 0.0 {
        origin_x + (max_width - column_width).max(0.0) - column_index as f32 * column_width
    } else {
        origin_x - column_index as f32 * column_width
    }
}

/// Builds source-aware interaction geometry from the same production-shaped
/// text used by [`layout_shaped_text`].
///
/// The returned clusters retain ligature ranges from the shaping stage, so a
/// source offset inside a ligature has no caret position of its own. Wrapping
/// uses the exact same cluster advances as the renderer and therefore cannot
/// make selection drift from painted glyphs.
pub fn layout_shaped_text_with_interaction(
    shaped_text: &ShapedText,
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
) -> TextInteractionLayout {
    let max_height = if shaped_text.writing_mode.is_vertical() {
        max_width
    } else {
        0.0
    };
    layout_shaped_text_with_interaction_with_bounds(
        shaped_text,
        origin_x,
        origin_y,
        max_width,
        max_height,
    )
}

/// Builds interaction geometry with independent horizontal and vertical
/// bounds. Vertical requests wrap when their column height reaches
/// `max_height` and continue in a new column to the left.
pub fn layout_shaped_text_with_interaction_with_bounds(
    shaped_text: &ShapedText,
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
    max_height: f32,
) -> TextInteractionLayout {
    let glyphs =
        layout_shaped_text_with_bounds(shaped_text, origin_x, origin_y, max_width, max_height);
    build_shaped_text_interaction(
        shaped_text,
        &glyphs,
        origin_x,
        origin_y,
        max_width,
        max_height,
    )
}

/// Produces the cache value consumed by the renderer and, on the Aimer path,
/// by interaction consumers.
pub fn layout_shaped_text_result(
    shaped_text: &ShapedText,
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
) -> PositionedTextLayout {
    layout_shaped_text_result_with_bounds(shaped_text, origin_x, origin_y, max_width, 0.0)
}

/// Produces a cached layout with separate horizontal and vertical wrapping
/// bounds. Horizontal callers retain the same behavior as
/// [`layout_shaped_text_result`].
pub fn layout_shaped_text_result_with_bounds(
    shaped_text: &ShapedText,
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
    max_height: f32,
) -> PositionedTextLayout {
    let glyphs =
        layout_shaped_text_with_bounds(shaped_text, origin_x, origin_y, max_width, max_height);
    PositionedTextLayout {
        interaction: Some(build_shaped_text_interaction(
            shaped_text,
            &glyphs,
            origin_x,
            origin_y,
            max_width,
            max_height,
        )),
        glyphs,
    }
}

fn build_shaped_text_interaction(
    shaped_text: &ShapedText,
    glyphs: &[PositionedGlyph],
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
    max_height: f32,
) -> TextInteractionLayout {
    if shaped_text.writing_mode.is_vertical() {
        return build_vertical_shaped_text_interaction(
            shaped_text,
            glyphs,
            origin_x,
            origin_y,
            max_width,
            max_height,
        );
    }

    let ascent = shaped_text.ascent.max(0.0);
    let descent = shaped_text.descent.min(0.0);
    let line_gap = shaped_text.line_gap.max(0.0);
    let line_height = shaped_text.line_height.max(0.0);
    let max_width = max_width.max(0.0);
    let mut clusters = Vec::with_capacity(shaped_text.clusters.len());
    let mut line_ranges = Vec::new();
    let mut line_widths = Vec::new();
    let mut line_start_source = 0usize;
    let mut line_index = 0usize;
    let mut pen_x = origin_x;
    let mut pen_y = origin_y;
    let mut last_break_cluster = usize::MAX;
    let mut last_break_pen_x = origin_x;

    for source_cluster in &shaped_text.clusters {
        if source_cluster.text == "\n" {
            clusters.push(TextCluster {
                text_range: source_cluster.text_range.clone(),
                line_index,
                level: source_cluster.level,
                start_x: pen_x,
                end_x: pen_x,
                start_y: pen_y,
                end_y: pen_y,
                y: pen_y - ascent - line_gap * 0.5,
                height: line_height,
            });
            line_ranges.push(line_start_source..source_cluster.text_range.end);
            line_widths.push((pen_x - origin_x).max(0.0));
            line_start_source = source_cluster.text_range.end;
            line_index += 1;
            pen_x = origin_x;
            pen_y += line_height;
            last_break_cluster = usize::MAX;
            last_break_pen_x = origin_x;
            continue;
        }

        if source_cluster.can_break_before {
            last_break_cluster = clusters.len();
            last_break_pen_x = pen_x;
        }

        if max_width > 0.0
            && pen_x + source_cluster.width > origin_x + max_width
            && pen_x > origin_x
        {
            if last_break_cluster != usize::MAX {
                let boundary = clusters
                    .get(last_break_cluster)
                    .map_or(source_cluster.text_range.start, |cluster| {
                        cluster.text_range.start
                    });
                let wrap_offset = last_break_pen_x - origin_x;
                for cluster in &mut clusters[last_break_cluster..] {
                    cluster.start_x -= wrap_offset;
                    cluster.end_x -= wrap_offset;
                    cluster.y += line_height;
                    cluster.line_index += 1;
                }
                line_ranges.push(line_start_source..boundary);
                line_widths.push((last_break_pen_x - origin_x).max(0.0));
                line_start_source = boundary;
                pen_x = origin_x + (pen_x - last_break_pen_x);
                pen_y += line_height;
                line_index += 1;
                last_break_cluster = usize::MAX;
                last_break_pen_x = origin_x;
            } else {
                line_ranges.push(line_start_source..source_cluster.text_range.start);
                line_widths.push((pen_x - origin_x).max(0.0));
                line_start_source = source_cluster.text_range.start;
                pen_x = origin_x;
                pen_y += line_height;
                line_index += 1;
            }
        }

        let start_x = pen_x;
        let end_x = pen_x + source_cluster.width;
        clusters.push(TextCluster {
            text_range: source_cluster.text_range.clone(),
            line_index,
            level: source_cluster.level,
            start_x,
            end_x,
            start_y: pen_y,
            end_y: pen_y,
            y: pen_y - ascent - line_gap * 0.5,
            height: line_height,
        });
        pen_x = end_x;
    }

    if line_ranges.is_empty() || line_start_source <= shaped_text.text.len() {
        line_ranges.push(line_start_source.min(shaped_text.text.len())..shaped_text.text.len());
        line_widths.push((pen_x - origin_x).max(0.0));
    }

    // A ligature expands the source range of its first shaped cluster. The
    // following grapheme records remain in `ShapedText` for wrapping/debugging
    // but are not separate caret stops in the interaction view.
    let mut source_end = 0usize;
    clusters.retain(|cluster| {
        if cluster.text_range.start < source_end {
            return false;
        }
        source_end = source_end.max(cluster.text_range.end);
        true
    });

    let lines = line_ranges
        .into_iter()
        .enumerate()
        .map(|(index, text_range)| {
            let hard_break = text_range
                .end
                .checked_sub(1)
                .and_then(|end| shaped_text.text.as_bytes().get(end))
                .is_some_and(|byte| *byte == b'\n');
            let glyph_start = glyphs
                .iter()
                .position(|glyph| glyph.line_index == index)
                .unwrap_or(0);
            let glyph_end = glyphs
                .iter()
                .rposition(|glyph| glyph.line_index == index)
                .map_or(glyph_start, |last| last + 1);
            TextLine {
                text_range,
                glyph_range: glyph_start..glyph_end,
                baseline: origin_y + index as f32 * line_height,
                width: line_widths.get(index).copied().unwrap_or(0.0),
                ascent,
                descent,
                line_gap,
                hard_break,
            }
        })
        .collect::<Vec<_>>();

    let width = line_widths.iter().copied().fold(0.0, f32::max);
    let line_count = lines.len();
    TextInteractionLayout {
        text: shaped_text.text.clone(),
        lines,
        clusters,
        metrics: ParagraphMetrics {
            width,
            height: line_count as f32 * line_height - line_gap,
            ascent,
            descent,
            line_gap,
            line_height,
            line_count,
        },
        origin_x,
        origin_y,
        writing_mode: shaped_text.writing_mode,
    }
}

fn build_vertical_shaped_text_interaction(
    shaped_text: &ShapedText,
    glyphs: &[PositionedGlyph],
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
    max_height: f32,
) -> TextInteractionLayout {
    let ascent = shaped_text.ascent.max(0.0);
    let descent = shaped_text.descent.min(0.0);
    let line_gap = shaped_text.line_gap.max(0.0);
    let line_height = shaped_text.line_height.max(0.0);
    let column_width = line_height.max(shaped_text.font_size).max(1.0);
    let line_band_width = (ascent - descent).max(0.0);
    let max_height = max_height.max(0.0);
    let mut clusters = Vec::with_capacity(shaped_text.clusters.len());
    let mut line_ranges = Vec::new();
    let mut line_extents = Vec::new();
    let mut line_start_source = 0usize;
    let mut line_index = 0usize;
    let mut pen_y = origin_y;
    let mut column_x = vertical_column_x(origin_x, max_width, column_width, line_index);

    for source_cluster in &shaped_text.clusters {
        if source_cluster.text == "\n" {
            clusters.push(TextCluster {
                text_range: source_cluster.text_range.clone(),
                line_index,
                level: source_cluster.level,
                start_x: column_x,
                end_x: column_x,
                start_y: pen_y,
                end_y: pen_y,
                y: pen_y,
                height: line_band_width,
            });
            line_ranges.push(line_start_source..source_cluster.text_range.end);
            line_extents.push((pen_y - origin_y).max(0.0));
            line_start_source = source_cluster.text_range.end;
            line_index += 1;
            pen_y = origin_y;
            column_x = vertical_column_x(origin_x, max_width, column_width, line_index);
            continue;
        }

        if source_cluster.can_break_before
            && max_height > 0.0
            && pen_y + source_cluster.width > origin_y + max_height
            && pen_y > origin_y
        {
            let boundary = source_cluster.text_range.start;
            line_ranges.push(line_start_source..boundary);
            line_extents.push((pen_y - origin_y).max(0.0));
            line_start_source = boundary;
            line_index += 1;
            pen_y = origin_y;
            column_x = vertical_column_x(origin_x, max_width, column_width, line_index);
        }

        let start_y = pen_y;
        let end_y = start_y + source_cluster.width;
        clusters.push(TextCluster {
            text_range: source_cluster.text_range.clone(),
            line_index,
            level: source_cluster.level,
            start_x: column_x,
            end_x: column_x,
            start_y,
            end_y,
            y: start_y,
            height: line_band_width,
        });
        pen_y = end_y;
    }

    if line_ranges.is_empty() || line_start_source <= shaped_text.text.len() {
        line_ranges.push(line_start_source.min(shaped_text.text.len())..shaped_text.text.len());
        line_extents.push((pen_y - origin_y).max(0.0));
    }

    let mut source_end = 0usize;
    clusters.retain(|cluster| {
        if cluster.text_range.start < source_end {
            return false;
        }
        source_end = source_end.max(cluster.text_range.end);
        true
    });

    let lines = line_ranges
        .into_iter()
        .enumerate()
        .map(|(index, text_range)| {
            let hard_break = text_range
                .end
                .checked_sub(1)
                .and_then(|end| shaped_text.text.as_bytes().get(end))
                .is_some_and(|byte| *byte == b'\n');
            let glyph_start = glyphs
                .iter()
                .position(|glyph| glyph.line_index == index)
                .unwrap_or(0);
            let glyph_end = glyphs
                .iter()
                .rposition(|glyph| glyph.line_index == index)
                .map_or(glyph_start, |last| last + 1);
            TextLine {
                text_range,
                glyph_range: glyph_start..glyph_end,
                baseline: vertical_column_x(origin_x, max_width, column_width, index),
                width: line_extents.get(index).copied().unwrap_or(0.0),
                ascent,
                descent,
                line_gap,
                hard_break,
            }
        })
        .collect::<Vec<_>>();

    let width = lines.len() as f32 * column_width;
    let height = line_extents.iter().copied().fold(0.0, f32::max);
    let line_count = lines.len();
    TextInteractionLayout {
        text: shaped_text.text.clone(),
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
        origin_x,
        origin_y,
        writing_mode: shaped_text.writing_mode,
    }
}

/// Simple horizontal text layout with basic line breaking.
pub fn layout_text(
    rasterizer: &mut GlyphRasterizer,
    text: &str,
    font_size: f32,
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
) -> Vec<PositionedGlyph> {
    let shaped_text = shape_text(rasterizer, text, font_size);
    layout_shaped_text(&shaped_text, origin_x, origin_y, max_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_alignment_offsets_each_visual_line_independently() {
        assert_eq!(
            line_alignment_offsets(&[120.0, 40.0], 200.0, TextHorizontalAlign::Center),
            vec![40.0, 80.0]
        );
    }

    #[test]
    fn right_alignment_never_moves_an_oversized_line_before_the_origin() {
        assert_eq!(
            line_alignment_offsets(&[240.0], 200.0, TextHorizontalAlign::Right),
            vec![0.0]
        );
    }

    fn test_layout(text: &str, max_width: f32) -> ParagraphLayout {
        let metrics = FontMetrics::new(8.0, -2.0, 2.0);
        let options = TextLayoutOptions::new(10.0, 0.0, 0.0, max_width);
        layout_paragraph(
            text,
            0,
            &metrics,
            &options,
            |segment, text_offset, writing_mode| {
                fallback_shape_segment_with_writing_mode(
                    segment,
                    text_offset,
                    0,
                    options.font_size,
                    writing_mode,
                )
            },
        )
    }

    #[test]
    fn preserves_explicit_newlines() {
        let layout = test_layout("first\nsecond", 0.0);

        assert_eq!(layout.lines.len(), 2);
        assert!(layout.lines[0].hard_break);
        assert_eq!(layout.lines[0].text_range, 0..5);
        assert_eq!(layout.lines[1].text_range, 6..12);
        assert_eq!(layout.metrics.line_count, 2);
        assert_eq!(
            layout.metrics.height,
            layout.metrics.line_height * 2.0 - layout.metrics.line_gap
        );
    }

    #[test]
    fn wraps_without_splitting_grapheme_clusters() {
        let layout = test_layout("Cafe\u{301} noir", 20.0);

        assert!(layout.lines.len() > 1);
        assert!(layout.glyphs.iter().any(|glyph| glyph.text_range == (3..6)));
        assert!(!layout.lines.iter().any(|line| line.text_range.end == 4));
    }

    #[test]
    fn paragraph_layout_exposes_grapheme_clusters_and_interaction_geometry() {
        let layout = test_layout("A e\u{301}", 0.0);

        let ranges: Vec<_> = layout
            .clusters
            .iter()
            .map(|cluster| cluster.text_range.clone())
            .collect();
        assert_eq!(ranges, vec![0..1, 1..2, 2..5]);

        let caret = layout
            .caret_geometry(2)
            .expect("cluster boundary has a caret");
        assert_eq!(caret.offset, 2);
        assert_eq!(caret.line_index, 0);
        assert_eq!(caret.x, 10.0);
        // The line box starts half the line gap above the ascent line.
        assert_eq!(caret.y, -9.0);
        assert_eq!(caret.height, 12.0);
        assert!(layout.caret_geometry(3).is_none());

        assert_eq!(layout.hit_test(2.0, -2.0), Some(0));
        assert_eq!(layout.hit_test(3.0, -2.0), Some(1));

        let selection = layout.selection_rects(0..5);
        assert_eq!(selection.len(), 1);
        assert_eq!(selection[0].line_index, 0);
        assert_eq!(selection[0].x, 0.0);
        assert_eq!(selection[0].width, 15.0);
        assert_eq!(selection[0].y, -9.0);
        assert_eq!(selection[0].height, 12.0);
    }

    #[test]
    fn paragraph_layout_maps_rtl_clusters_in_visual_order() {
        let layout = test_layout("abc אבג", 0.0);

        let ranges: Vec<_> = layout
            .clusters
            .iter()
            .map(|cluster| cluster.text_range.clone())
            .collect();
        assert_eq!(ranges, vec![0..1, 1..2, 2..3, 3..4, 8..10, 6..8, 4..6]);
        assert!(layout.clusters[4].level.is_rtl());
        assert!(layout.clusters[4].start_x > layout.clusters[4].end_x);
        assert_eq!(layout.hit_test(21.0, -2.0), Some(10));
    }

    #[test]
    fn shaped_glyph_ranges_preserve_ligature_source_clusters() {
        let layout = layout_paragraph_with_shaper(
            "office",
            include_bytes!("../fonts/GoogleSans-Regular.ttf"),
            0,
            FontMetrics::new(12.0, -4.0, 2.0),
            TextLayoutOptions::new(16.0, 0.0, 0.0, 200.0),
        );

        assert!(layout.glyphs.iter().any(|glyph| glyph.text_range == (1..4)));
        assert!(
            layout
                .clusters
                .iter()
                .any(|cluster| cluster.text_range == (1..4))
        );
        assert!(layout.caret_geometry(2).is_none());
        assert!(layout.caret_geometry(4).is_some());
        assert!(layout.caret_geometry(5).is_some());
    }

    #[test]
    fn selection_geometry_splits_at_wrapped_lines() {
        let layout = test_layout("ab cd", 15.0);

        let selection = layout.selection_rects(0..5);
        assert_eq!(selection.len(), 2);
        assert_eq!(selection[0].line_index, 0);
        assert_eq!(selection[0].x, 0.0);
        assert_eq!(selection[0].width, 15.0);
        assert_eq!(selection[1].line_index, 1);
        assert_eq!(selection[1].x, 0.0);
        assert_eq!(selection[1].width, 10.0);
    }

    #[test]
    fn paragraph_wrapping_breaks_between_ideographs() {
        // Every cluster advances 5.0 with the fallback shaper, so eight of
        // them fill a 40px line.  A space-only wrapper would rewind to the
        // single space and leave "ab " alone on the first line.
        let layout = test_layout("ab 你好世界你好世界", 40.0);

        assert_eq!(layout.lines[0].glyph_range, 0..8);
        assert_eq!(layout.lines[0].width, 40.0);
    }

    #[test]
    fn paragraph_wrapping_terminates_when_no_cluster_fits() {
        // A width narrower than a single cluster leaves no usable break
        // opportunity; layout must still consume the text instead of
        // re-queueing the same run forever.
        let layout = test_layout("你好世界", 1.0);

        assert_eq!(layout.lines.len(), 4);
        assert_eq!(layout.glyphs.len(), 4);
    }

    #[test]
    fn ellipsis_truncates_at_cluster_boundary() {
        let metrics = FontMetrics::new(8.0, -2.0, 2.0);
        let mut options = TextLayoutOptions::new(10.0, 0.0, 0.0, 18.0);
        options.ellipsis = true;
        let layout = layout_paragraph(
            "Cafe\u{301}",
            0,
            &metrics,
            &options,
            |segment, text_offset, writing_mode| {
                fallback_shape_segment_with_writing_mode(
                    segment,
                    text_offset,
                    0,
                    options.font_size,
                    writing_mode,
                )
            },
        );

        assert_eq!(
            layout.glyphs.last().map(|glyph| glyph.source.as_str()),
            Some("…")
        );
        assert!(!layout.lines.iter().any(|line| line.text_range.end == 4));
        assert!(layout.metrics.width <= options.max_width);
    }

    #[test]
    fn shape_text_styled_shapes_a_same_font_paragraph_as_one_run() {
        let mut rasterizer = GlyphRasterizer::new();
        rasterizer.reset_shape_call_count();
        let text = "A long paragraph with repeated words and enough graphemes to expose per-cluster shaping.";

        let shaped = shape_text_styled(
            &mut rasterizer,
            text,
            16.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        );

        assert_eq!(rasterizer.shape_call_count(), 1);
        assert_eq!(shaped.clusters.len(), text.graphemes(true).count());
    }

    #[test]
    fn simple_ascii_shaping_preserves_ligatures_and_hard_breaks() {
        let mut rasterizer = GlyphRasterizer::new();
        rasterizer.reset_shape_call_count();

        let shaped = shape_text_styled(
            &mut rasterizer,
            "office Aimer\nfast path",
            16.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        );

        assert_eq!(
            shaped
                .clusters
                .iter()
                .map(|cluster| cluster.text.as_str())
                .collect::<Vec<_>>(),
            vec![
                "o", "f", "f", "i", "c", "e", " ", "A", "i", "m", "e", "r", "\n", "f", "a", "s",
                "t", " ", "p", "a", "t", "h",
            ]
        );
        assert!(
            shaped
                .clusters
                .iter()
                .any(|cluster| cluster.text_range == (1..4) && cluster.glyphs.len() == 1),
            "the ffi ligature must retain its full source range"
        );
        assert!(shaped.clusters.iter().any(|cluster| cluster.text == "\n"));
        assert_eq!(rasterizer.shape_call_count(), 2);
        assert_eq!(rasterizer.simple_ltr_path_count(), 1);
    }

    #[test]
    fn full_unicode_shaping_reuses_output_storage_without_losing_clusters() {
        let mut rasterizer = GlyphRasterizer::new();
        let shaped = shape_text(&mut rasterizer, "éclair سلام", 16.0);

        assert!(shaped.clusters.iter().any(|cluster| cluster.text == "é"));
        assert!(shaped.clusters.iter().any(|cluster| cluster.text == "س"));
        assert!(shaped.clusters.iter().any(|cluster| cluster.text == "م"));
        assert!(rasterizer.reused_shape_output_count() > 0);
    }

    #[test]
    fn shape_text_styled_preserves_graphemes_and_hard_breaks() {
        let mut rasterizer = GlyphRasterizer::new();
        rasterizer.reset_shape_call_count();

        let shaped = shape_text(&mut rasterizer, "Cafe\u{301}\nnext", 16.0);
        let cluster_text: Vec<&str> = shaped
            .clusters
            .iter()
            .map(|cluster| cluster.text.as_str())
            .collect();

        assert_eq!(
            cluster_text,
            vec!["C", "a", "f", "e\u{301}", "\n", "n", "e", "x", "t"]
        );
        assert_eq!(rasterizer.shape_call_count(), 2);
    }

    #[test]
    fn shape_text_styled_maps_ligatures_to_grapheme_clusters() {
        let mut rasterizer = GlyphRasterizer::new();

        let shaped = shape_text(&mut rasterizer, "office", 16.0);
        let glyph_count: usize = shaped
            .clusters
            .iter()
            .map(|cluster| cluster.glyphs.len())
            .sum();

        assert_eq!(shaped.clusters.len(), 6);
        assert!(glyph_count < shaped.clusters.len());
        assert_eq!(rasterizer.shape_call_count(), 1);
    }

    #[test]
    fn production_layout_keeps_ligatures_indivisible_for_interaction() {
        let mut rasterizer = GlyphRasterizer::new();
        let shaped = shape_text(&mut rasterizer, "office", 16.0);
        let layout = layout_shaped_text_with_interaction(&shaped, 0.0, 16.0, 0.0);

        let ligature = layout
            .clusters
            .iter()
            .find(|cluster| cluster.text_range == (1..4))
            .expect("the ffi ligature must expose one source cluster");
        assert!(ligature.end_x > ligature.start_x);
        assert!(layout.caret_geometry(2).is_none());
        assert_eq!(layout.caret_geometry(1).unwrap().x, ligature.start_x);
        assert_eq!(layout.caret_geometry(4).unwrap().x, ligature.end_x);

        let selection = layout.selection_rects(2..3);
        assert_eq!(selection.len(), 1);
        assert_eq!(selection[0].x, ligature.start_x);
        assert_eq!(selection[0].width, ligature.end_x - ligature.start_x);
    }

    /// A font whose line gap is not zero (Nimbus Sans on Linux) puts half the
    /// gap above the first baseline. The clusters, and so the caret and the
    /// selection band, must span that whole line box rather than start at the
    /// ascent line, or they sit `line_gap / 2` below the painted text.
    #[test]
    fn interaction_clusters_span_the_line_box_when_the_font_has_a_line_gap() {
        let mut rasterizer = GlyphRasterizer::new();
        let mut shaped = shape_text(&mut rasterizer, "ab\ncd", 16.0);
        shaped.line_gap = 4.0;
        shaped.line_height = shaped.ascent - shaped.descent + shaped.line_gap;
        // Paragraph origin as the canvas places it: the line box starts at 0.
        let origin_y = shaped.ascent + shaped.line_gap * 0.5;

        let layout = layout_shaped_text_with_interaction(&shaped, 0.0, origin_y, 0.0);

        assert_eq!(layout.lines.len(), 2);
        for cluster in &layout.clusters {
            let top = cluster.line_index as f32 * shaped.line_height;
            assert!(
                (cluster.y - top).abs() < 1e-3,
                "cluster {:?} starts at {}, expected {top}",
                cluster.text_range,
                cluster.y
            );
            assert!((cluster.height - shaped.line_height).abs() < 1e-3);
        }
        for (offset, line) in [(1, 0), (4, 1)] {
            let caret = layout.caret_geometry(offset).expect("caret inside a line");
            assert!((caret.y - line as f32 * shaped.line_height).abs() < 1e-3);
            assert!((caret.height - shaped.line_height).abs() < 1e-3);
            assert!((line_top(&layout.lines[line]) - caret.y).abs() < 1e-3);
        }
    }

    #[test]
    fn aimer_font_keeps_rtl_paragraph_runs_in_visual_order() {
        let text = "שלום abc";
        let layout = layout_paragraph_with_shaper(
            text,
            include_bytes!("../fonts/GoogleSans-Regular.ttf"),
            0,
            FontMetrics::new(12.0, -4.0, 2.0),
            TextLayoutOptions::new(16.0, 0.0, 0.0, 200.0),
        );

        let ranges: Vec<_> = layout
            .runs
            .iter()
            .map(|run| run.text_range.clone())
            .collect();

        assert_eq!(ranges, vec![9..12, 0..9]);
        assert!(layout.runs[0].level.is_ltr());
        assert!(layout.runs[1].level.is_rtl());
    }

    #[test]
    fn script_less_clusters_stay_inside_the_surrounding_run() {
        let mut script = None;

        // Leading punctuation has no script of its own, so it neither starts
        // nor breaks a run.
        assert!(extends_script_run(&mut script, "("));
        assert_eq!(script, None);

        assert!(extends_script_run(&mut script, "K"));
        assert_eq!(script, Some(Script::Latin));

        // Spaces, dashes and combining marks must not end a Latin run.
        assert!(extends_script_run(&mut script, " "));
        assert!(extends_script_run(&mut script, "—"));
        assert!(extends_script_run(&mut script, "e\u{301}"));
        assert_eq!(script, Some(Script::Latin));
    }

    #[test]
    fn a_different_script_ends_the_run() {
        let mut script = Some(Script::Latin);

        assert!(!extends_script_run(&mut script, "ស"));
        assert_eq!(
            script,
            Some(Script::Latin),
            "a rejected cluster must not change the run it failed to join"
        );
    }

    #[test]
    fn khmer_shapes_the_same_inside_and_outside_latin_text() {
        // A shaping buffer carries a single script, guessed from its first
        // strong character. Merging Khmer into a Latin run therefore shapes it
        // with the default shaper: COENG stays a visible sign and the
        // subscript consonant keeps its full size instead of tucking under the
        // base as a leg.
        let mut rasterizer = GlyphRasterizer::new();
        for khmer in ["សួស្តី", "ស្រឡាញ់", "សួស្តី\u{200b}ពិភពលោក"]
        {
            let isolated = shape_text(&mut rasterizer, khmer, 32.0);
            let mixed = shape_text(&mut rasterizer, &format!("Khmer — {khmer} (Suosdei)"), 32.0);

            assert_eq!(
                khmer_glyph_ids(&isolated),
                khmer_glyph_ids(&mixed),
                "mixed Latin/Khmer shaping must preserve the owned Khmer run for {khmer}"
            );
            assert!(
                khmer_glyph_ids(&isolated).len() < khmer.chars().count(),
                "COENG must combine into a subscript glyph, got {:?}",
                khmer_glyph_ids(&isolated)
            );
        }
    }

    #[test]
    fn khmer_shape_is_not_changed_by_common_context() {
        let khmer = "សួស្តី";
        let text = format!("Straße / café\u{301} / 你好 / {khmer} — mixed CASE and spaces");
        let mut rasterizer = GlyphRasterizer::new();
        let isolated = shape_text(&mut rasterizer, khmer, 16.0);
        let mixed = shape_text(&mut rasterizer, &text, 16.0);

        assert_eq!(
            khmer_glyph_ids(&isolated),
            khmer_glyph_ids(&mixed),
            "common spaces and punctuation must not change the Khmer subscript leg"
        );
    }

    #[test]
    fn khmer_showcase_has_a_stable_owned_raster_golden() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let shaped = shape_text(&mut rasterizer, "សួស្តី\u{200b}ពិភពលោក", 32.0);
        let glyphs = shaped
            .clusters
            .iter()
            .flat_map(|cluster| cluster.glyphs.iter())
            .collect::<Vec<_>>();

        assert_eq!(
            glyphs
                .iter()
                .map(|glyph| glyph.key.glyph_id)
                .collect::<Vec<_>>(),
            vec![
                3620, 3906, 3620, 3847, 3895, 1866, 3611, 3894, 3612, 3611, 3665, 3701, 3589,
            ]
        );

        let mut checksum = 0xcbf2_9ce4_8422_2325_u64;
        let mut visible_pixels = 0_usize;
        for glyph in glyphs {
            let rendered = rasterizer.rasterize_key(glyph.key, 32.0);
            for value in [
                u64::from(glyph.key.glyph_id),
                u64::from(rendered.width),
                u64::from(rendered.height),
            ] {
                for byte in value.to_le_bytes() {
                    checksum ^= u64::from(byte);
                    checksum = checksum.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
            visible_pixels += rendered.bitmap.iter().filter(|pixel| **pixel != 0).count();
            for byte in &rendered.bitmap {
                checksum ^= u64::from(*byte);
                checksum = checksum.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }

        assert!(
            visible_pixels > 1_000,
            "Khmer showcase must contain visible ink"
        );
        assert_eq!(
            checksum, 11_027_706_440_622_539_205,
            "update the Khmer raster golden after an intentional change"
        );
    }

    fn khmer_glyph_ids(shaped: &ShapedText) -> Vec<u16> {
        shaped
            .clusters
            .iter()
            .filter(|cluster| cluster_script(&cluster.text) == Some(Script::Khmer))
            .flat_map(|cluster| cluster.glyphs.iter())
            .map(|glyph| glyph.key.glyph_id)
            .collect()
    }

    #[test]
    fn shaping_bakes_the_pixel_box_into_every_glyph() {
        // The pixel box is a pure function of the glyph key, so shaping — the
        // width-independent stage that survives a resize — is where it is
        // measured. Positioning must never need to ask anyone for it again.
        let mut rasterizer = GlyphRasterizer::new();

        let shaped = shape_text(&mut rasterizer, "Ag ex 你好", 17.0);

        for cluster in &shaped.clusters {
            for glyph in &cluster.glyphs {
                let metrics = rasterizer.metrics_for_key(glyph.key, 17.0);
                assert_eq!(glyph.width, metrics.width);
                assert_eq!(glyph.height, metrics.height);
                assert_eq!(glyph.offset_x, metrics.offset_x);
                assert_eq!(glyph.offset_y, metrics.offset_y);
            }
        }
    }

    #[test]
    fn positioning_reads_only_the_shaped_text() {
        // A resize frame re-wraps everything on screen; the wrap must be pure
        // arithmetic over the cached shaping — no rasterizer, no locks. The
        // rasterizer is dropped before layout to prove nothing else is read.
        let mut rasterizer = GlyphRasterizer::new();
        let shaped = shape_text(&mut rasterizer, "alpha bravo charlie delta", 16.0);
        drop(rasterizer);

        let unwrapped = layout_shaped_text(&shaped, 0.0, 0.0, 0.0);
        let full_width = unwrapped
            .last()
            .map_or(0.0, |glyph| glyph.line_x + glyph.advance);
        let wrapped = layout_shaped_text(&shaped, 0.0, 0.0, full_width / 2.0);

        assert!(!wrapped.is_empty());
        assert!(
            wrapped.iter().map(|glyph| glyph.line_index).max() > Some(0),
            "halving the width must wrap the text"
        );
        assert_eq!(
            wrapped.len(),
            unwrapped.len(),
            "wrapping must reposition glyphs, not lose them"
        );
    }

    #[test]
    fn shaping_reports_the_widest_hard_break_line() {
        let mut rasterizer = GlyphRasterizer::new();

        let shaped = shape_text(&mut rasterizer, "long first line\nab", 16.0);

        let first_line_width: f32 = shaped
            .clusters
            .iter()
            .take_while(|cluster| cluster.text != "\n")
            .map(|cluster| cluster.width)
            .sum();
        assert!(first_line_width > 0.0);
        assert_eq!(shaped.max_line_width, first_line_width);
    }

    #[test]
    fn layout_at_a_fitting_width_matches_the_unbounded_layout() {
        // The wrapping width only matters when the text wraps at it: any
        // width the widest line fits in must position every glyph exactly
        // where the unbounded layout does, which is what lets one cached
        // layout stand in for every such width.
        let mut rasterizer = GlyphRasterizer::new();
        let shaped = shape_text(&mut rasterizer, "alpha bravo\ncharlie delta", 16.0);

        let unbounded = layout_shaped_text(&shaped, 0.0, 0.0, 0.0);
        let fitting = layout_shaped_text(&shaped, 0.0, 0.0, shaped.max_line_width);

        assert!(!unbounded.is_empty());
        assert_eq!(fitting.len(), unbounded.len());
        for (fitting, unbounded) in fitting.iter().zip(&unbounded) {
            assert_eq!(fitting.glyph_key, unbounded.glyph_key);
            assert_eq!((fitting.x, fitting.y), (unbounded.x, unbounded.y));
            assert_eq!(fitting.line_index, unbounded.line_index);
        }
    }

    #[test]
    fn vertical_layout_wraps_down_then_starts_the_next_column_on_the_left() {
        let glyph = |glyph_id| ShapedGlyph {
            key: GlyphKey::new(0, glyph_id, 16.0),
            advance: 0.0,
            y_advance: -10.0,
            x_offset: 0.0,
            y_offset: 0.0,
            width: 8,
            height: 8,
            offset_x: 0.0,
            offset_y: 0.0,
        };
        let shaped = ShapedText {
            text: "abcd".to_string(),
            font_size: 16.0,
            ascent: 12.0,
            descent: -4.0,
            line_gap: 0.0,
            line_height: 16.0,
            max_line_width: 40.0,
            writing_mode: TextWritingMode::VerticalRl,
            clusters: (0..4)
                .map(|index| ShapedCluster {
                    text: char::from(b'a' + index as u8).to_string(),
                    text_range: index..index + 1,
                    level: unicode_bidi::Level::ltr(),
                    base_codepoint: char::from(b'a' + index as u8),
                    glyphs: vec![glyph(index as u16 + 1)],
                    width: 10.0,
                    can_break_before: index > 0,
                })
                .collect(),
        };

        let positioned = layout_shaped_text_with_bounds(&shaped, 0.0, 0.0, 32.0, 25.0);

        assert_eq!(positioned.len(), 4);
        assert_eq!(
            positioned
                .iter()
                .map(|glyph| glyph.line_index)
                .collect::<Vec<_>>(),
            vec![0, 0, 1, 1]
        );
        assert_eq!(positioned[0].x, 16.0);
        assert_eq!(positioned[1].y - positioned[0].y, 10.0);
        assert_eq!(positioned[2].x, 0.0);
        assert_eq!(positioned[2].y, positioned[0].y);
    }

    #[test]
    fn paragraph_vertical_layout_wraps_columns_with_the_same_source_ranges() {
        let metrics = FontMetrics::new(8.0, -2.0, 2.0);
        let mut options = TextLayoutOptions::new(10.0, 0.0, 0.0, 32.0);
        options.max_height = 25.0;
        options.writing_mode = TextWritingMode::VerticalRl;

        let layout = layout_paragraph(
            "一二三四",
            0,
            &metrics,
            &options,
            |segment, text_offset, writing_mode| {
                fallback_shape_segment_with_writing_mode(
                    segment,
                    text_offset,
                    0,
                    options.font_size,
                    writing_mode,
                )
            },
        );

        assert_eq!(layout.writing_mode, TextWritingMode::VerticalRl);
        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.lines[0].text_range, 0..6);
        assert_eq!(layout.lines[1].text_range, 6..12);
        assert_eq!(layout.lines[0].baseline, 20.0);
        assert_eq!(layout.lines[1].baseline, 8.0);
        assert_eq!(layout.glyphs[0].x, 20.0);
        assert_eq!(layout.glyphs[2].x, 8.0);
        assert_eq!(layout.glyphs[1].y, 10.0);
        assert_eq!(layout.glyphs[2].y, 0.0);
    }

    #[test]
    fn vertical_interaction_uses_column_and_progression_geometry() {
        let shaped = ShapedText {
            text: "abcd".to_string(),
            font_size: 16.0,
            ascent: 12.0,
            descent: -4.0,
            line_gap: 0.0,
            line_height: 16.0,
            max_line_width: 40.0,
            writing_mode: TextWritingMode::VerticalRl,
            clusters: (0..4)
                .map(|index| ShapedCluster {
                    text: char::from(b'a' + index as u8).to_string(),
                    text_range: index..index + 1,
                    level: unicode_bidi::Level::ltr(),
                    base_codepoint: char::from(b'a' + index as u8),
                    glyphs: vec![ShapedGlyph {
                        key: GlyphKey::new(0, index as u16 + 1, 16.0),
                        advance: 0.0,
                        y_advance: -10.0,
                        x_offset: 0.0,
                        y_offset: 0.0,
                        width: 8,
                        height: 8,
                        offset_x: 0.0,
                        offset_y: 0.0,
                    }],
                    width: 10.0,
                    can_break_before: index > 0,
                })
                .collect(),
        };

        let layout = layout_shaped_text_with_interaction_with_bounds(&shaped, 0.0, 0.0, 32.0, 25.0);

        assert_eq!(layout.writing_mode, TextWritingMode::VerticalRl);
        assert_eq!(layout.lines.len(), 2);
        let caret = layout
            .caret_geometry(2)
            .expect("column boundary has a caret");
        assert_eq!(caret.x, 0.0);
        assert_eq!(caret.y, 0.0);
        assert_eq!(caret.width, 16.0);
        assert_eq!(caret.height, 0.0);
        assert_eq!(layout.hit_test(16.0, 2.0), Some(0));
        assert_eq!(layout.hit_test(16.0, 12.0), Some(1));

        let selection = layout.selection_rects(0..4);
        assert_eq!(selection.len(), 2);
        assert_eq!(selection[0].width, 16.0);
        assert_eq!(selection[0].height, 20.0);
    }

    #[test]
    fn vertical_shaping_uses_owned_vertical_pen_for_non_cjk_text() {
        let mut rasterizer = GlyphRasterizer::new();
        let shaped = shape_text_styled_with_writing_mode(
            &mut rasterizer,
            "AB",
            16.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
            TextWritingMode::VerticalRl,
        );

        assert_eq!(shaped.writing_mode, TextWritingMode::VerticalRl);
        let glyphs: Vec<_> = shaped
            .clusters
            .iter()
            .flat_map(|cluster| cluster.glyphs.iter())
            .collect();
        assert_eq!(glyphs.len(), 2);
        assert!(glyphs.iter().all(|glyph| glyph.advance == 0.0));
        assert!(glyphs.iter().all(|glyph| glyph.y_advance < 0.0));
        assert!(shaped.max_line_width > 0.0);
    }

    #[test]
    fn break_opportunities_follow_unicode_line_breaking() {
        let text = "Hello, World!你好，世界";
        let breaks = line_break_opportunities(text);

        let offset_of = |needle: &str| text.find(needle).unwrap();

        // A break belongs *after* the space, never between "Hello" and ",".
        assert!(breaks[offset_of("World")]);
        assert!(!breaks[offset_of(",")]);
        // Han ideographs break between each other...
        assert!(breaks[offset_of("好")]);
        assert!(breaks[offset_of("世")]);
        // ...but never before closing/trailing punctuation.
        assert!(!breaks[offset_of("，")]);
        // The paragraph start is not a break opportunity.
        assert!(!breaks[0]);
    }

    #[test]
    fn cjk_lines_fill_the_available_width_instead_of_rewinding_to_the_last_space() {
        // Chinese has no spaces, so a space-only word wrapper rewinds to the
        // last Latin space it saw — here the one inside "Hello, World!" — and
        // pushes the whole ideographic tail onto the next line, leaving a
        // ragged half-empty line behind.
        let mut rasterizer = GlyphRasterizer::new();
        assert!(
            rasterizer
                .register_font_bytes(
                    include_bytes!("../fonts/NotoSansJP-VariableFont_wght.ttf").to_vec()
                )
                .is_some(),
            "the deterministic CJK fixture must be readable"
        );
        let font_size = 16.0;
        let text = "「Hello, World!」（世界你好！）之類字串的電腦程式在大多数通用编程语言中";
        let max_width = 240.0;

        let shaped = shape_text(&mut rasterizer, text, font_size);
        let glyphs = layout_shaped_text(&shaped, 0.0, 0.0, max_width);
        let widths = positioned_line_widths(&glyphs);

        assert!(widths.len() > 1, "the sample must wrap at {max_width}px");
        for (index, width) in widths.iter().enumerate().take(widths.len() - 1) {
            assert!(
                *width > max_width - font_size * 1.5,
                "line {index} stopped at {width}px of {max_width}px"
            );
        }
    }

    #[test]
    fn latin_words_still_wrap_whole() {
        let mut rasterizer = GlyphRasterizer::new();
        let font_size = 16.0;
        let text = "alpha bravo charlie delta";

        let shaped = shape_text(&mut rasterizer, text, font_size);
        let single_line = layout_shaped_text(&shaped, 0.0, 0.0, 0.0);
        let word_width: f32 = shaped
            .clusters
            .iter()
            .take("alpha".len())
            .map(|cluster| cluster.width)
            .sum();
        let max_width = single_line
            .last()
            .map_or(0.0, |glyph| glyph.line_x + glyph.advance)
            / 2.0;

        let glyphs = layout_shaped_text(&shaped, 0.0, 0.0, max_width);
        let line_count = glyphs
            .iter()
            .map(|glyph| glyph.line_index)
            .max()
            .map_or(0, |last| last + 1);

        assert!(line_count > 1);
        // Every line starts a word, so the first glyph of each line sits at the
        // left margin and no line begins mid-word.
        for line in 0..line_count {
            let first = glyphs
                .iter()
                .find(|glyph| glyph.line_index == line)
                .expect("each line holds at least one glyph");
            assert!(first.line_x.abs() < 0.01, "line {line} starts mid-word");
        }
        assert!(word_width > 0.0);
    }

    #[test]
    fn shape_text_styled_splits_runs_at_fallback_font_boundaries() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        rasterizer
            .register_font_bytes(
                include_bytes!("../fonts/NotoSansJP-VariableFont_wght.ttf").to_vec(),
            )
            .expect("the bundled CJK fallback should register");
        rasterizer.reset_shape_call_count();

        let shaped = shape_text(&mut rasterizer, "a你b", 16.0);
        let font_ids: Vec<FontId> = shaped
            .clusters
            .iter()
            .filter_map(|cluster| cluster.glyphs.first())
            .map(|glyph| glyph.key.font_id)
            .collect();

        assert_eq!(shaped.clusters.len(), 3);
        assert_eq!(rasterizer.shape_call_count(), 3);
        assert_ne!(font_ids[0], font_ids[1]);
        assert_eq!(font_ids[0], font_ids[2]);
    }

    /// `❤` and `♥` default to text presentation, and `U+FE0F` is how text asks
    /// for the emoji form. Resolving the face from the base character alone
    /// gave `❤️` the monochrome glyph of a symbols font on macOS.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn emoji_presentation_selector_draws_text_default_symbols_as_color_emoji() {
        let mut rasterizer = GlyphRasterizer::new();

        for symbol in ['\u{2764}', '\u{2665}', '\u{263A}'] {
            let text = format!("{symbol}\u{FE0F}");
            let shaped = shape_text(&mut rasterizer, &text, 16.0);

            assert_eq!(shaped.clusters.len(), 1, "{text:?} is one grapheme");
            let key = shaped.clusters[0]
                .glyphs
                .first()
                .unwrap_or_else(|| panic!("{text:?} must shape to a glyph"))
                .key;
            let glyph = rasterizer.glyph_metrics_for_key(key, 16.0);
            assert!(
                glyph.is_color,
                "{symbol:?} + U+FE0F must use the color emoji face, not a text symbol face"
            );
            assert!(
                glyph.width > 0 && glyph.height > 0,
                "{symbol:?} + U+FE0F must have a visible bitmap"
            );
        }
    }

    /// Keycaps are `U+FE0F` sequences too, but their bases are ASCII digits the
    /// text face already draws; the selector must not move them to the emoji
    /// face as a standalone digit.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn emoji_presentation_selector_leaves_ascii_keycap_bases_alone() {
        let mut rasterizer = GlyphRasterizer::new();
        let resolve = |rasterizer: &mut GlyphRasterizer, cluster: &str| {
            rasterizer.font_id_for_family_cluster(
                cluster,
                16.0,
                FontFamily::SANS_SERIF,
                FontWeight::Normal,
                FontStyle::Normal,
            )
        };

        assert_eq!(
            resolve(&mut rasterizer, "1\u{FE0F}\u{20E3}"),
            resolve(&mut rasterizer, "1"),
        );
        assert_eq!(
            resolve(&mut rasterizer, "#\u{FE0F}"),
            resolve(&mut rasterizer, "#"),
        );
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn mixed_latin_cjk_emoji_and_combining_text_stays_renderable() {
        let mut rasterizer = GlyphRasterizer::new();
        let text = "A你😀e\u{301}";
        let shaped = shape_text_styled(
            &mut rasterizer,
            text,
            20.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            Some(TextLanguage::Chinese),
        );

        let clusters: Vec<&str> = shaped
            .clusters
            .iter()
            .map(|cluster| cluster.text.as_str())
            .collect();
        assert_eq!(clusters, vec!["A", "你", "😀", "e\u{301}"]);

        let primary = rasterizer.primary_font_id();
        let face_for = |cluster: &ShapedCluster| {
            cluster
                .glyphs
                .first()
                .map(|glyph| glyph.key.font_id)
                .expect("every non-empty grapheme must shape to a glyph")
        };
        let latin = face_for(&shaped.clusters[0]);
        let cjk = face_for(&shaped.clusters[1]);
        let emoji = face_for(&shaped.clusters[2]);
        let combining = face_for(&shaped.clusters[3]);

        assert_eq!(latin, primary);
        assert_eq!(combining, primary);
        assert_ne!(cjk, primary);
        assert_ne!(emoji, primary);
        assert_ne!(cjk, emoji, "CJK and emoji must not share a fallback face");

        for cluster in &shaped.clusters {
            for glyph in &cluster.glyphs {
                assert_ne!(
                    glyph.key.glyph_id, 0,
                    "{} resolved to .notdef",
                    cluster.text
                );
                let rendered = rasterizer.rasterize_key(glyph.key, 20.0);
                assert!(
                    !rendered.bitmap.is_empty(),
                    "{} produced an empty bitmap",
                    cluster.text
                );
            }
        }
    }
}
