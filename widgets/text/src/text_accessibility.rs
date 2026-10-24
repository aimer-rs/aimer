use std::ops::Range;

use aimer_attribute::Bounds;
use aimer_cupid::text_layout::{
    ParagraphMetrics, TextCluster, TextInteractionLayout, TextLine, TextWritingMode,
};
use aimer_cupid::utilities::Mat3;

/// One visual line in a [`TextAccessibilitySnapshot`].
///
/// `source_range` uses UTF-8 byte offsets into [`TextAccessibilitySnapshot::text`].
/// The line order is visual/document order, while the source range remains
/// logical order. All geometry is in absolute logical coordinates after the
/// canvas transform and device-scale conversion have been applied.
#[derive(Clone, Debug, PartialEq)]
pub struct TextAccessibilityLine {
    /// The zero-based visual line index.
    pub index: usize,
    /// The logical UTF-8 byte range represented by this line.
    pub source_range: Range<usize>,
    /// The transformed line box.
    pub bounds: Bounds,
    /// The transformed start of the line baseline.
    pub baseline_start: (f32, f32),
    /// The transformed end of the line baseline.
    pub baseline_end: (f32, f32),
    /// The line ascent in the layout's local physical coordinate space.
    pub ascent: f32,
    /// The line descent in the layout's local physical coordinate space.
    pub descent: f32,
    /// The line gap in the layout's local physical coordinate space.
    pub line_gap: f32,
    /// Whether the source range ends at an explicit hard break.
    pub hard_break: bool,
}

/// One source cluster in visual order.
///
/// A cluster is the smallest caret/selection unit produced by shaping. It can
/// represent a grapheme, a ligature, a combining sequence, or a fallback-font
/// cluster, so consumers must not invent caret stops inside `source_range`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextAccessibilityCluster {
    /// The logical UTF-8 byte range represented by this cluster.
    pub source_range: Range<usize>,
    /// The visual line containing this cluster.
    pub line_index: usize,
    /// The resolved UAX #9 bidi level.
    pub level: unicode_bidi::Level,
    /// The transformed cluster bounds.
    pub bounds: Bounds,
    /// The transformed caret point at the logical start of the cluster.
    pub start: (f32, f32),
    /// The transformed caret point at the logical end of the cluster.
    pub end: (f32, f32),
}

/// A transformed caret for a valid UTF-8 source boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextAccessibilityCaret {
    /// The logical UTF-8 byte offset represented by the caret.
    pub offset: usize,
    /// The visual line containing the caret.
    pub line_index: usize,
    /// The transformed caret rectangle.
    pub bounds: Bounds,
}

/// One transformed rectangle occupied by a logical selection range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextAccessibilitySelectionRect {
    /// The visual line containing this rectangle.
    pub line_index: usize,
    /// The transformed selection rectangle.
    pub bounds: Bounds,
}

/// A source-aware accessibility view of one Aimer text layout.
///
/// The snapshot is deliberately independent of a native accessibility API. A
/// host adapter can expose [`Self::text`] and [`Self::bounds`] as a semantic
/// node, then use the source ranges, line boxes, caret, hit-test, and selection
/// geometry to implement text navigation without consulting a second shaping
/// or font stack.
///
/// The snapshot accepts the same physical-space canvas transform and device
/// scale used by the painter. Its public rectangles and points are returned in
/// absolute logical coordinates, matching pointer and selection APIs.
#[derive(Clone, Debug)]
pub struct TextAccessibilitySnapshot {
    layout: TextInteractionLayout,
    local_bounds: Bounds,
    bounds: Bounds,
    lines: Vec<TextAccessibilityLine>,
    clusters: Vec<TextAccessibilityCluster>,
    transform: Mat3,
    scale: f32,
    selection: Option<Range<usize>>,
}

impl TextAccessibilitySnapshot {
    /// Builds a snapshot from the layout used to paint the text.
    ///
    /// `transform` is the physical-space canvas transform and `scale` is the
    /// device scale used by the frame. Singular or non-finite transforms return
    /// `None`, because they cannot provide a reliable inverse hit-test mapping.
    pub fn from_layout(layout: TextInteractionLayout, transform: Mat3, scale: f32) -> Option<Self> {
        transform.inverse_transform_point(0.0, 0.0)?;
        let scale = valid_device_scale(scale);
        if !metrics_are_finite(&layout.metrics)
            || layout.metrics.line_count != layout.lines.len()
            || layout
                .lines
                .iter()
                .any(|line| !source_range_is_valid(&layout.text, &line.text_range))
            || layout.clusters.iter().any(|cluster| {
                cluster.line_index >= layout.lines.len()
                    || !source_range_is_valid(&layout.text, &cluster.text_range)
            })
        {
            return None;
        }
        let local_bounds = paragraph_bounds(&layout);
        let bounds = map_rect(&transform, scale, local_bounds);
        if !bounds_are_finite(bounds) {
            return None;
        }

        let lines: Vec<TextAccessibilityLine> = layout
            .lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let local_bounds = line_bounds(&layout, line);
                TextAccessibilityLine {
                    index,
                    source_range: line.text_range.clone(),
                    bounds: map_rect(&transform, scale, local_bounds),
                    baseline_start: map_point(
                        &transform,
                        scale,
                        line_baseline_start(&layout, line),
                    ),
                    baseline_end: map_point(&transform, scale, line_baseline_end(&layout, line)),
                    ascent: line.ascent,
                    descent: line.descent,
                    line_gap: line.line_gap,
                    hard_break: line.hard_break,
                }
            })
            .collect();
        let clusters: Vec<TextAccessibilityCluster> = layout
            .clusters
            .iter()
            .map(|cluster| TextAccessibilityCluster {
                source_range: cluster.text_range.clone(),
                line_index: cluster.line_index,
                level: cluster.level,
                bounds: map_rect(&transform, scale, cluster_bounds(&layout, cluster)),
                start: map_point(&transform, scale, (cluster.start_x, cluster.start_y)),
                end: map_point(&transform, scale, (cluster.end_x, cluster.end_y)),
            })
            .collect();
        if lines.iter().any(|line| {
            !bounds_are_finite(line.bounds)
                || !point_is_finite(line.baseline_start)
                || !point_is_finite(line.baseline_end)
        }) || clusters.iter().any(|cluster| {
            !bounds_are_finite(cluster.bounds)
                || !point_is_finite(cluster.start)
                || !point_is_finite(cluster.end)
        }) {
            return None;
        }

        Some(Self {
            layout,
            local_bounds,
            bounds,
            lines,
            clusters,
            transform,
            scale,
            selection: None,
        })
    }

    /// Returns the original source text for all source ranges in this snapshot.
    #[inline]
    pub fn text(&self) -> &str {
        &self.layout.text
    }

    /// Returns the underlying interaction layout used to build this snapshot.
    ///
    /// This escape hatch is useful for adapters that need a layout-specific
    /// field not duplicated by this portable semantic model. The returned layout
    /// remains in local physical coordinates; use the snapshot geometry methods
    /// for absolute logical coordinates.
    #[inline]
    pub fn layout(&self) -> &TextInteractionLayout {
        &self.layout
    }

    /// Returns aggregate paragraph metrics in local physical coordinates.
    #[inline]
    pub fn metrics(&self) -> &ParagraphMetrics {
        &self.layout.metrics
    }

    /// Returns the writing mode used to interpret the geometry.
    #[inline]
    pub const fn writing_mode(&self) -> TextWritingMode {
        self.layout.writing_mode
    }

    /// Returns the transformed paragraph bounds in absolute logical coordinates.
    #[inline]
    pub const fn bounds(&self) -> Bounds {
        self.bounds
    }

    /// Converts this snapshot into the generic host-facing text node.
    ///
    /// The host owns `id` and should keep it stable for the lifetime of the
    /// retained text element. The generic node publishes the complete source
    /// text as its accessible name and the transformed paragraph bounds as its
    /// hit-test bounds. Line, cluster, bidi, caret, and selection geometry
    /// remains available from this snapshot; it is intentionally not exposed
    /// as synthetic accessibility children because those are interaction
    /// units, not independent semantic content.
    #[inline]
    pub fn to_semantic_node(
        &self,
        id: aimer_accessibility::NodeId,
    ) -> aimer_accessibility::SemanticNode {
        let bounds = self.bounds;
        let bounds =
            aimer_accessibility::Bounds::new(bounds.x, bounds.y, bounds.width, bounds.height)
                .expect("validated text accessibility bounds must be finite");
        aimer_accessibility::SemanticNode::new(id, aimer_accessibility::Role::Text)
            .with_name(self.text())
            .with_bounds(bounds)
    }

    /// Wraps this snapshot in a one-node generic accessibility tree.
    ///
    /// Use [`Self::to_semantic_node`] when the text node must be composed with
    /// a larger host-owned tree.
    #[inline]
    pub fn to_semantic_tree(
        &self,
        id: aimer_accessibility::NodeId,
    ) -> aimer_accessibility::SemanticTree {
        aimer_accessibility::SemanticTree::new(self.to_semantic_node(id))
    }

    /// Returns visual lines with their logical source ranges and transformed
    /// line geometry.
    #[inline]
    pub fn lines(&self) -> &[TextAccessibilityLine] {
        &self.lines
    }

    /// Returns source clusters in visual order.
    #[inline]
    pub fn clusters(&self) -> &[TextAccessibilityCluster] {
        &self.clusters
    }

    /// Returns one visual line by index.
    #[inline]
    pub fn line(&self, index: usize) -> Option<&TextAccessibilityLine> {
        self.lines.get(index)
    }

    /// Returns one visual cluster by index.
    #[inline]
    pub fn cluster(&self, index: usize) -> Option<&TextAccessibilityCluster> {
        self.clusters.get(index)
    }

    /// Replaces the snapshot's optional logical selection.
    ///
    /// Endpoints are ordered, clamped to the source text, and moved back to
    /// UTF-8 character boundaries. The snapshot remains valid when a host
    /// receives stale or partially byte-indexed selection data.
    #[inline]
    pub fn with_selection(mut self, selection: Option<Range<usize>>) -> Self {
        self.selection = selection.and_then(|selection| normalize_range(self.text(), selection));
        self
    }

    /// Returns the normalized logical selection, if one was attached.
    #[inline]
    pub fn selection(&self) -> Option<Range<usize>> {
        self.selection.clone()
    }

    /// Returns the selected source text, if a selection was attached and is a
    /// valid range in the source string.
    #[inline]
    pub fn selected_text(&self) -> Option<&str> {
        self.selection
            .as_ref()
            .and_then(|range| self.text().get(range.clone()))
    }

    fn local_point(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        self.transform
            .inverse_transform_point(x * self.scale, y * self.scale)
    }

    /// Reports whether an absolute logical point is inside the transformed
    /// paragraph geometry.
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        let Some((local_x, local_y)) = self.local_point(x, y) else {
            return false;
        };
        self.local_bounds.is_inside(local_x, local_y)
    }

    /// Maps an absolute logical point to the nearest source cluster boundary.
    #[inline]
    pub fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        let (local_x, local_y) = self.local_point(x, y)?;
        self.layout.hit_test(local_x, local_y)
    }

    /// Returns transformed caret geometry for a valid source boundary.
    pub fn caret(&self, offset: usize) -> Option<TextAccessibilityCaret> {
        let caret = self.layout.caret_geometry(offset)?;
        Some(TextAccessibilityCaret {
            offset: caret.offset,
            line_index: caret.line_index,
            bounds: map_rect(
                &self.transform,
                self.scale,
                Bounds::new(caret.x, caret.y, caret.width, caret.height),
            ),
        })
    }

    /// Returns only the transformed caret rectangle for `offset`.
    #[inline]
    pub fn caret_rect(&self, offset: usize) -> Option<Bounds> {
        self.caret(offset).map(|caret| caret.bounds)
    }

    /// Returns transformed selection rectangles for a logical UTF-8 byte range.
    pub fn selection_rects(&self, selection: Range<usize>) -> Vec<TextAccessibilitySelectionRect> {
        let Some(selection) = normalize_range(self.text(), selection) else {
            return Vec::new();
        };
        self.layout
            .selection_rects(selection)
            .into_iter()
            .map(|rect| TextAccessibilitySelectionRect {
                line_index: rect.line_index,
                bounds: map_rect(
                    &self.transform,
                    self.scale,
                    Bounds::new(rect.x, rect.y, rect.width, rect.height),
                ),
            })
            .collect()
    }

    /// Returns transformed selection rectangles for the attached selection.
    #[inline]
    pub fn selected_rects(&self) -> Vec<TextAccessibilitySelectionRect> {
        self.selection
            .clone()
            .map_or_else(Vec::new, |selection| self.selection_rects(selection))
    }
}

impl PartialEq for TextAccessibilitySnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.layout == other.layout
            && self.local_bounds == other.local_bounds
            && self.bounds == other.bounds
            && self.lines == other.lines
            && self.clusters == other.clusters
            && self.transform.cols == other.transform.cols
            && self.scale == other.scale
            && self.selection == other.selection
    }
}

fn line_height(line: &TextLine) -> f32 {
    (line.ascent - line.descent + line.line_gap).max(0.0)
}

fn line_bounds(layout: &TextInteractionLayout, line: &TextLine) -> Bounds {
    if layout.writing_mode == TextWritingMode::VerticalRl {
        Bounds::new(
            line.baseline,
            layout.origin_y,
            (line.ascent - line.descent).max(0.0),
            line.width.max(0.0),
        )
    } else {
        Bounds::new(
            layout.origin_x,
            line.baseline - line.ascent - line.line_gap * 0.5,
            line.width.max(0.0),
            line_height(line),
        )
    }
}

fn line_baseline_start(layout: &TextInteractionLayout, line: &TextLine) -> (f32, f32) {
    if layout.writing_mode == TextWritingMode::VerticalRl {
        (line.baseline, layout.origin_y)
    } else {
        (layout.origin_x, line.baseline)
    }
}

fn line_baseline_end(layout: &TextInteractionLayout, line: &TextLine) -> (f32, f32) {
    if layout.writing_mode == TextWritingMode::VerticalRl {
        (line.baseline, layout.origin_y + line.width.max(0.0))
    } else {
        (layout.origin_x + line.width.max(0.0), line.baseline)
    }
}

fn cluster_bounds(layout: &TextInteractionLayout, cluster: &TextCluster) -> Bounds {
    if layout.writing_mode == TextWritingMode::VerticalRl {
        Bounds::new(
            cluster.start_x,
            cluster.start_y.min(cluster.end_y),
            cluster.height.max(0.0),
            (cluster.end_y - cluster.start_y).abs(),
        )
    } else {
        Bounds::new(
            cluster.start_x.min(cluster.end_x),
            cluster.y,
            (cluster.end_x - cluster.start_x).abs(),
            cluster.height.max(0.0),
        )
    }
}

fn paragraph_bounds(layout: &TextInteractionLayout) -> Bounds {
    let mut bounds = None;
    for line in &layout.lines {
        let line_bounds = line_bounds(layout, line);
        bounds = Some(match bounds {
            Some(bounds) => union_bounds(bounds, line_bounds),
            None => line_bounds,
        });
    }
    for cluster in &layout.clusters {
        let cluster_bounds = cluster_bounds(layout, cluster);
        bounds = Some(match bounds {
            Some(bounds) => union_bounds(bounds, cluster_bounds),
            None => cluster_bounds,
        });
    }
    bounds.unwrap_or_else(|| {
        Bounds::new(
            layout.origin_x,
            layout.origin_y,
            layout.metrics.width.max(0.0),
            layout.metrics.height.max(0.0),
        )
    })
}

fn union_bounds(left: Bounds, right: Bounds) -> Bounds {
    let min_x = left.x.min(right.x);
    let min_y = left.y.min(right.y);
    let max_x = (left.x + left.width).max(right.x + right.width);
    let max_y = (left.y + left.height).max(right.y + right.height);
    Bounds::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

fn map_point(transform: &Mat3, scale: f32, point: (f32, f32)) -> (f32, f32) {
    let point = transform.transform_point(point.0, point.1);
    (point.0 / scale, point.1 / scale)
}

fn map_rect(transform: &Mat3, scale: f32, rect: Bounds) -> Bounds {
    let points = [
        map_point(transform, scale, (rect.x, rect.y)),
        map_point(transform, scale, (rect.x + rect.width, rect.y)),
        map_point(transform, scale, (rect.x, rect.y + rect.height)),
        map_point(
            transform,
            scale,
            (rect.x + rect.width, rect.y + rect.height),
        ),
    ];
    let (min_x, max_x) = points
        .iter()
        .map(|point| point.0)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    let (min_y, max_y) = points
        .iter()
        .map(|point| point.1)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    Bounds::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

fn bounds_are_finite(bounds: Bounds) -> bool {
    bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
}

fn point_is_finite(point: (f32, f32)) -> bool {
    point.0.is_finite() && point.1.is_finite()
}

fn metrics_are_finite(metrics: &ParagraphMetrics) -> bool {
    metrics.width.is_finite()
        && metrics.height.is_finite()
        && metrics.ascent.is_finite()
        && metrics.descent.is_finite()
        && metrics.line_gap.is_finite()
        && metrics.line_height.is_finite()
}

fn source_range_is_valid(text: &str, range: &Range<usize>) -> bool {
    range.start <= range.end
        && range.end <= text.len()
        && text.is_char_boundary(range.start)
        && text.is_char_boundary(range.end)
}

fn valid_device_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > f32::EPSILON {
        scale
    } else {
        1.0
    }
}

fn normalize_range(text: &str, selection: Range<usize>) -> Option<Range<usize>> {
    let start = clamp_to_char_boundary(text, selection.start.min(selection.end));
    let end = clamp_to_char_boundary(text, selection.start.max(selection.end));
    (start <= end).then_some(start..end)
}

fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimer_cupid::text_layout::{ParagraphMetrics, TextCluster, TextLine, TextWritingMode};
    use unicode_segmentation::UnicodeSegmentation;

    fn layout() -> TextInteractionLayout {
        TextInteractionLayout {
            text: "aאב".to_owned(),
            lines: vec![TextLine {
                text_range: 0..5,
                glyph_range: 0..0,
                baseline: 16.0,
                width: 30.0,
                ascent: 12.0,
                descent: -4.0,
                line_gap: 0.0,
                hard_break: false,
            }],
            clusters: vec![
                TextCluster {
                    text_range: 0..1,
                    line_index: 0,
                    level: unicode_bidi::Level::ltr(),
                    start_x: 0.0,
                    end_x: 10.0,
                    start_y: 16.0,
                    end_y: 16.0,
                    y: 4.0,
                    height: 20.0,
                },
                TextCluster {
                    text_range: 1..5,
                    line_index: 0,
                    level: unicode_bidi::Level::rtl(),
                    start_x: 30.0,
                    end_x: 10.0,
                    start_y: 16.0,
                    end_y: 16.0,
                    y: 4.0,
                    height: 20.0,
                },
            ],
            metrics: ParagraphMetrics {
                width: 30.0,
                height: 20.0,
                ascent: 12.0,
                descent: -4.0,
                line_gap: 0.0,
                line_height: 20.0,
                line_count: 1,
            },
            origin_x: 0.0,
            origin_y: 0.0,
            writing_mode: TextWritingMode::HorizontalTb,
        }
    }

    fn vertical_layout() -> TextInteractionLayout {
        TextInteractionLayout {
            text: "ab".to_owned(),
            lines: vec![TextLine {
                text_range: 0..2,
                glyph_range: 0..0,
                baseline: 20.0,
                width: 20.0,
                ascent: 8.0,
                descent: -2.0,
                line_gap: 0.0,
                hard_break: false,
            }],
            clusters: vec![
                TextCluster {
                    text_range: 0..1,
                    line_index: 0,
                    level: unicode_bidi::Level::ltr(),
                    start_x: 20.0,
                    end_x: 20.0,
                    start_y: 0.0,
                    end_y: 10.0,
                    y: 0.0,
                    height: 10.0,
                },
                TextCluster {
                    text_range: 1..2,
                    line_index: 0,
                    level: unicode_bidi::Level::ltr(),
                    start_x: 20.0,
                    end_x: 20.0,
                    start_y: 10.0,
                    end_y: 20.0,
                    y: 10.0,
                    height: 10.0,
                },
            ],
            metrics: ParagraphMetrics {
                width: 10.0,
                height: 20.0,
                ascent: 8.0,
                descent: -2.0,
                line_gap: 0.0,
                line_height: 10.0,
                line_count: 1,
            },
            origin_x: 0.0,
            origin_y: 0.0,
            writing_mode: TextWritingMode::VerticalRl,
        }
    }

    fn unicode_matrix_layout() -> TextInteractionLayout {
        let text = "Latin Ελληνικά Русский עברית العربية हिंदी বাংলা தமிழ் ไทย ខ្មែរ မြန်မာ 中文 日本語 한국어 e\u{301} 😀 © «»";
        let mut clusters = Vec::new();
        let mut width = 0.0;
        for (start, cluster_text) in text.grapheme_indices(true) {
            let end = start + cluster_text.len();
            let cluster_width = if cluster_text == " " { 6.0 } else { 12.0 };
            let rtl = cluster_text
                .chars()
                .next()
                .is_some_and(|codepoint| matches!(codepoint, '\u{0590}'..='\u{08ff}'));
            clusters.push(TextCluster {
                text_range: start..end,
                line_index: 0,
                level: if rtl {
                    unicode_bidi::Level::rtl()
                } else {
                    unicode_bidi::Level::ltr()
                },
                start_x: if rtl { width + cluster_width } else { width },
                end_x: if rtl { width } else { width + cluster_width },
                start_y: 20.0,
                end_y: 20.0,
                y: 4.0,
                height: 20.0,
            });
            width += cluster_width;
        }
        TextInteractionLayout {
            text: text.to_owned(),
            lines: vec![TextLine {
                text_range: 0..text.len(),
                glyph_range: 0..0,
                baseline: 20.0,
                width,
                ascent: 16.0,
                descent: -4.0,
                line_gap: 0.0,
                hard_break: false,
            }],
            clusters,
            metrics: ParagraphMetrics {
                width,
                height: 20.0,
                ascent: 16.0,
                descent: -4.0,
                line_gap: 0.0,
                line_height: 20.0,
                line_count: 1,
            },
            origin_x: 0.0,
            origin_y: 0.0,
            writing_mode: TextWritingMode::HorizontalTb,
        }
    }

    #[test]
    fn snapshot_keeps_source_ranges_and_maps_geometry() {
        let snapshot = TextAccessibilitySnapshot::from_layout(
            layout(),
            Mat3::translate(50.0, 70.0).mul(&Mat3::scale(2.0, 3.0)),
            2.0,
        )
        .expect("finite affine text layout");

        assert_eq!(snapshot.text(), "aאב");
        assert_eq!(snapshot.clusters()[1].source_range, 1..5);
        assert!(snapshot.clusters()[1].level.is_rtl());
        assert_eq!(snapshot.bounds(), Bounds::new(25.0, 41.0, 30.0, 30.0));
        assert_eq!(
            snapshot.caret_rect(1),
            Some(Bounds::new(55.0, 41.0, 0.0, 30.0))
        );
        assert_eq!(snapshot.hit_test(27.5, 40.0), Some(0));
    }

    #[test]
    fn snapshot_normalizes_selection_and_preserves_visual_selection_geometry() {
        let snapshot = TextAccessibilitySnapshot::from_layout(layout(), Mat3::identity(), 1.0)
            .expect("finite affine text layout")
            .with_selection(Some(4..0));

        assert_eq!(snapshot.selection(), Some(0..3));
        assert_eq!(snapshot.selected_text(), Some("aא"));
        assert_eq!(
            snapshot.selected_rects(),
            vec![TextAccessibilitySelectionRect {
                line_index: 0,
                bounds: Bounds::new(0.0, 4.0, 30.0, 20.0),
            }]
        );
        assert_eq!(snapshot.lines()[0].source_range, 0..5);
        assert_eq!(snapshot.lines()[0].baseline_start, (0.0, 16.0));
        assert_eq!(snapshot.lines()[0].baseline_end, (30.0, 16.0));
    }

    #[test]
    fn snapshot_uses_vertical_caret_and_column_geometry() {
        let snapshot = TextAccessibilitySnapshot::from_layout(
            vertical_layout(),
            Mat3::translate(5.0, 7.0),
            1.0,
        )
        .expect("finite affine text layout")
        .with_selection(Some(0..2));

        assert_eq!(snapshot.writing_mode(), TextWritingMode::VerticalRl);
        assert_eq!(snapshot.bounds(), Bounds::new(25.0, 7.0, 10.0, 20.0));
        assert_eq!(
            snapshot.caret_rect(1),
            Some(Bounds::new(25.0, 17.0, 10.0, 0.0))
        );
        assert_eq!(snapshot.hit_test(25.0, 8.0), Some(0));
        assert_eq!(
            snapshot.selected_rects(),
            vec![TextAccessibilitySelectionRect {
                line_index: 0,
                bounds: Bounds::new(25.0, 7.0, 10.0, 20.0),
            }]
        );
    }

    #[test]
    fn snapshot_rejects_singular_transforms_and_accepts_invalid_scale_as_one() {
        assert!(
            TextAccessibilitySnapshot::from_layout(layout(), Mat3::scale(0.0, 1.0), 1.0,).is_none()
        );

        let snapshot = TextAccessibilitySnapshot::from_layout(layout(), Mat3::identity(), f32::NAN)
            .expect("invalid device scales use the compatibility scale");
        assert_eq!(snapshot.bounds(), Bounds::new(0.0, 4.0, 30.0, 20.0));
    }

    #[test]
    fn snapshot_rejects_malformed_source_ranges_and_line_indices() {
        let mut malformed_range = layout();
        malformed_range.lines[0].text_range = 0..6;
        assert!(
            TextAccessibilitySnapshot::from_layout(malformed_range, Mat3::identity(), 1.0,)
                .is_none()
        );

        let mut malformed_line = layout();
        malformed_line.clusters[0].line_index = 1;
        assert!(
            TextAccessibilitySnapshot::from_layout(malformed_line, Mat3::identity(), 1.0,)
                .is_none()
        );
    }

    #[test]
    fn snapshot_maps_to_a_generic_semantic_text_node() {
        let snapshot = TextAccessibilitySnapshot::from_layout(layout(), Mat3::identity(), 1.0)
            .expect("finite affine text layout");
        let node = snapshot.to_semantic_node(aimer_accessibility::NodeId::new(42));

        assert_eq!(node.id(), aimer_accessibility::NodeId::new(42));
        assert_eq!(node.role(), aimer_accessibility::Role::Text);
        assert_eq!(node.name(), Some("aאב"));
        assert_eq!(node.bounds().unwrap().x(), 0.0);
        assert_eq!(node.bounds().unwrap().y(), 4.0);
        assert!(node.children().is_empty());

        let tree = snapshot
            .to_semantic_tree(aimer_accessibility::NodeId::new(42))
            .snapshot()
            .expect("a text projection is a valid semantic tree");
        assert_eq!(tree.root(), &node);
    }

    #[test]
    fn snapshot_and_semantic_projection_preserve_the_full_unicode_matrix() {
        let snapshot = TextAccessibilitySnapshot::from_layout(
            unicode_matrix_layout(),
            Mat3::translate(20.0, 30.0).mul(&Mat3::scale(2.0, 2.0)),
            2.0,
        )
        .expect("the full Unicode accessibility layout should be valid");

        assert_eq!(
            snapshot.clusters().len(),
            snapshot.text().graphemes(true).count()
        );
        assert!(
            snapshot
                .clusters()
                .iter()
                .any(|cluster| cluster.level.is_rtl())
        );
        for cluster in snapshot.clusters() {
            assert!(snapshot.text().is_char_boundary(cluster.source_range.start));
            assert!(snapshot.text().is_char_boundary(cluster.source_range.end));
            assert!(
                snapshot
                    .text()
                    .get(cluster.source_range.clone())
                    .is_some_and(|source| !source.is_empty())
            );
            assert!(cluster.bounds.width >= 0.0 && cluster.bounds.height >= 0.0);
        }

        let node = snapshot.to_semantic_node(aimer_accessibility::NodeId::new(88));
        assert_eq!(node.role(), aimer_accessibility::Role::Text);
        assert_eq!(node.name(), Some(snapshot.text()));
        assert_eq!(node.bounds().unwrap().width(), snapshot.bounds().width);
        assert_eq!(
            snapshot
                .to_semantic_tree(aimer_accessibility::NodeId::new(88))
                .snapshot()
                .expect("the Unicode projection should publish")
                .root(),
            &node
        );
    }
}
