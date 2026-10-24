pub(crate) mod geometry;

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_style::TextTransform;
use aimer_style::{FontStyle, LineHeight, TextAlign, TextDecorationLine, TextOverflow};
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::layout_invalidation_generation;
use unicode_segmentation::UnicodeSegmentation;

use crate::paragraph::geometry::{
    DecorationPaint, PreparedBackground, prepare_background_runs, prepare_decoration_paint,
    vertical_span_is_visible,
};
use crate::text_span::{
    ResolvedTextSpan, adjusted_grapheme_advance, adjusted_width, ellipsize_first_line,
    layout_resolved_spans_with_indent,
};

/// One painted run of text: a maximal slice of a single span that shares a
/// line, in element-local physical pixels.
pub(crate) struct PreparedFragment {
    pub span_index: usize,
    pub text: String,
    pub source_range: Option<Range<usize>>,
    pub rendered_source_ranges: Vec<Range<usize>>,
    pub line: usize,
    pub x: f32,
    pub baseline: f32,
    pub width: f32,
    pub height: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// Cached line geometry for one decorated fragment.
///
/// The paint color is intentionally not stored here: a link can change its
/// inherited color while its layout remains valid. The physical stroke values
/// and line centers, however, depend only on the prepared fragment and scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreparedDecoration {
    pub fragment_index: usize,
    pub paint: DecorationPaint,
    pub underline_center: f32,
    pub line_through_center: f32,
    pub overline_center: f32,
}

/// Cached geometry of one grapheme cluster, in element-local physical pixels.
///
/// Measuring graphemes is the expensive part of hit testing and of painting a
/// selection, so it happens once per layout instead of once per frame.
pub(crate) struct GraphemeBox {
    pub source_range: Range<usize>,
    pub fragment_index: usize,
    pub rendered_range: Range<usize>,
    pub x: f32,
    pub width: f32,
}

/// A rendered grapheme's source slice and prepared local x position.
///
/// The paragraph painter reuses this geometry for letter- and word-spaced
/// runs, so dynamic link paint does not segment and measure the same text on
/// every frame.
#[derive(Clone, Debug)]
pub(crate) struct PreparedTextRun {
    pub rendered_range: Range<usize>,
    pub x: f32,
}

/// A hard line break, which owns no glyphs but is still selectable and
/// hit-testable up to the end of its line.
pub(crate) struct PreparedLineBreak {
    pub source_range: Range<usize>,
    pub line: usize,
    pub x: f32,
    pub y: f32,
    pub hit_width: f32,
    pub selection_width: f32,
    pub height: f32,
}

/// A complete, immutable paragraph layout for one width and one device pixel
/// ratio.
pub(crate) struct PreparedLayout {
    pub fragments: Vec<PreparedFragment>,
    pub decorations: Vec<PreparedDecoration>,
    pub graphemes: Vec<GraphemeBox>,
    pub paint_runs: Vec<Vec<PreparedTextRun>>,
    pub backgrounds: Vec<PreparedBackground>,
    pub line_breaks: Vec<PreparedLineBreak>,
    pub line_heights: Vec<f32>,
    pub size: ResolvedSize,
    pub aimer_interaction: Option<Rc<aimer_cupid::text_layout::TextInteractionLayout>>,
}

fn prepare_decorations(
    fragments: &[PreparedFragment],
    spans: &[ResolvedTextSpan],
    scale: f32,
) -> Vec<PreparedDecoration> {
    fragments
        .iter()
        .enumerate()
        .filter_map(|(fragment_index, fragment)| {
            let span = &spans[fragment.span_index];
            let font_size = span.style.font_size.max(1) as f32 * scale;
            let paint = prepare_decoration_paint(span.style.text_decoration, font_size, scale)?;
            Some(PreparedDecoration {
                fragment_index,
                paint,
                underline_center: fragment.baseline
                    + fragment.descent.max(1.0) * 0.5
                    + paint.offset,
                line_through_center: fragment.baseline - fragment.ascent * 0.35 + paint.offset,
                overline_center: fragment.baseline - fragment.ascent + paint.offset,
            })
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PreparedLayoutKey {
    width_bits: u32,
    scale_bits: u32,
    layout_generation: u64,
    include_graphemes: bool,
}

/// Resolves the color a span is painted with, letting a hovered link override
/// its own color only.
pub(crate) fn display_color(
    span: &ResolvedTextSpan,
    hovered_link: Option<&crate::TextSource>,
    link_hover_color: Option<Color>,
) -> Color {
    if hovered_link.is_some() && span.link.as_ref() == hovered_link {
        link_hover_color.unwrap_or(span.style.color)
    } else {
        span.style.color
    }
}

/// A laid-out run of resolved spans with a one-entry layout cache.
///
/// The paragraph owns everything that does not depend on interaction: the
/// spans, their alignment, their overflow mode and the cached
/// [`PreparedLayout`]. Elements add selection, links and hover on top.
///
/// # Examples
///
/// ```ignore
/// let paragraph = Paragraph::new(spans, TextAlign::TopLeft, TextOverflow::Wrap);
/// let layout = paragraph.prepare(ctx);
/// let canvas = Canvas::of(ctx);
/// let scale = ctx.scale;
/// paragraph.record_retained_v2_backgrounds(&canvas, &layout, scale, origin);
/// paragraph.record_retained_v2_foreground(&canvas, &layout, scale, origin, None, None);
/// ```
pub(crate) struct Paragraph {
    spans: Vec<ResolvedTextSpan>,
    text_align: TextAlign,
    overflow: TextOverflow,
    line_height: LineHeight,
    text_indent: f32,
    layout_cache: RefCell<Option<(PreparedLayoutKey, Rc<PreparedLayout>)>>,
}

fn centered_line_baseline(top: f32, ascent: f32, descent: f32, height: f32) -> f32 {
    top + ascent + (height - ascent - descent).max(0.0) * 0.5
}

impl Paragraph {
    /// Creates a paragraph over already resolved spans.
    #[inline]
    pub fn new(
        spans: Vec<ResolvedTextSpan>,
        text_align: TextAlign,
        overflow: TextOverflow,
    ) -> Self {
        Self::with_layout(spans, text_align, overflow, LineHeight::default(), 0.0)
    }

    /// Creates a paragraph with its paragraph-level line-height and first-line
    /// indentation settings.
    #[inline]
    pub fn with_layout(
        spans: Vec<ResolvedTextSpan>,
        text_align: TextAlign,
        overflow: TextOverflow,
        line_height: LineHeight,
        text_indent: f32,
    ) -> Self {
        Self {
            spans,
            text_align,
            overflow,
            line_height,
            text_indent,
            layout_cache: RefCell::new(None),
        }
    }

    /// Reports whether the paragraph paints outside its own width and therefore
    /// needs a clip.
    #[inline]
    pub const fn needs_clip(&self) -> bool {
        matches!(self.overflow, TextOverflow::Clip | TextOverflow::Ellipsis)
    }

    pub(crate) fn supports_retained_v2_rich_text(&self) -> bool {
        self.text_indent.is_finite()
            && match self.line_height {
                LineHeight::Normal => true,
                LineHeight::Px(value) | LineHeight::Factor(value) => value.is_finite(),
            }
            && !self.spans.is_empty()
            && self.spans.iter().all(|span| {
                let style = span.style;
                style.letter_spacing.is_finite()
                    && style.word_spacing.is_finite()
                    && style.text_shadow.is_none_or(|shadow| {
                        shadow.offset_x.is_finite()
                            && shadow.offset_y.is_finite()
                            && shadow.blur.is_finite()
                    })
            })
    }

    pub(crate) fn retained_v2_paint_outsets(
        &self,
        layout: &PreparedLayout,
        scale: f32,
    ) -> [f32; 4] {
        if !scale.is_finite() || scale <= 0.0 {
            return [0.0; 4];
        }
        let mut min_x = 0.0_f32;
        let mut min_y = 0.0_f32;
        let mut max_x = layout.size.width;
        let mut max_y = layout.size.height;

        for fragment in &layout.fragments {
            let style = self.spans[fragment.span_index].style;
            let mut left = fragment.x;
            let mut right = fragment.x + fragment.width;
            if style.font_style == FontStyle::Italic
                || style
                    .text_decoration
                    .line
                    .contains(TextDecorationLine::ITALIC)
            {
                let overhang = style.font_size.max(1) as f32 * scale * 0.5;
                left -= overhang;
                right += overhang;
            }
            let top = fragment.baseline - fragment.ascent;
            let bottom = top + fragment.height;
            min_x = min_x.min(left);
            min_y = min_y.min(top);
            max_x = max_x.max(right);
            max_y = max_y.max(bottom);

            if let Some(shadow) = style.text_shadow
                && shadow.color.as_u32() >> 24 != 0
            {
                let offset_x = shadow.offset_x * scale;
                let offset_y = shadow.offset_y * scale;
                let blur = shadow.blur.max(0.0) * scale;
                min_x = min_x.min(fragment.x + offset_x - blur);
                max_x = max_x.max(fragment.x + fragment.width + offset_x + blur);
                min_y = min_y.min(top + offset_y - blur);
                max_y = max_y.max(bottom + offset_y + blur);
            }
        }

        for decoration in &layout.decorations {
            let fragment = &layout.fragments[decoration.fragment_index];
            let paint = decoration.paint;
            let centers = [
                (paint.lines.contains(TextDecorationLine::UNDERLINE))
                    .then_some(decoration.underline_center),
                (paint.lines.contains(TextDecorationLine::LINE_THROUGH))
                    .then_some(decoration.line_through_center),
                (paint.lines.contains(TextDecorationLine::OVERLINE))
                    .then_some(decoration.overline_center),
            ];
            for center in centers.into_iter().flatten() {
                let top = center - paint.band_height * 0.5;
                min_x = min_x.min(fragment.x);
                max_x = max_x.max(fragment.x + fragment.width);
                min_y = min_y.min(top);
                max_y = max_y.max(top + paint.band_height);
            }
        }

        [
            (-min_x).max(0.0) / scale,
            (-min_y).max(0.0) / scale,
            (max_x - layout.size.width).max(0.0) / scale,
            (max_y - layout.size.height).max(0.0) / scale,
        ]
    }

    pub(crate) fn record_retained_v2_backgrounds(
        &self,
        canvas: &Canvas,
        layout: &PreparedLayout,
        scale: f32,
        origin_offset: Vec2d,
    ) {
        for background in &layout.backgrounds {
            let (red, green, blue, alpha) = background.color.to_rgba();
            canvas.fill_rect(
                aimer_cupid::utilities::Rect::new(
                    background.x / scale + origin_offset.x,
                    background.y / scale + origin_offset.y,
                    background.width / scale,
                    background.height / scale,
                ),
                [red, green, blue, alpha],
            );
        }
    }

    pub(crate) fn record_retained_v2_foreground(
        &self,
        canvas: &Canvas,
        layout: &PreparedLayout,
        scale: f32,
        origin_offset: Vec2d,
        hovered_link: Option<&crate::TextSource>,
        link_hover_color: Option<Color>,
    ) {
        use aimer_cupid::text_pipeline::TextOverflowMode;
        use aimer_cupid::text_pipeline::TextShadowRequest;
        use aimer_cupid::text_pipeline::text_layout::TextHorizontalAlign;

        for (fragment_index, fragment) in layout.fragments.iter().enumerate() {
            if fragment.text.is_empty() {
                continue;
            }
            let span = &self.spans[fragment.span_index];
            let style = span.style;
            let color = display_color(span, hovered_link, link_hover_color);
            let italic = style.font_style == FontStyle::Italic
                || style
                    .text_decoration
                    .line
                    .contains(TextDecorationLine::ITALIC);
            let shadow = style.text_shadow.and_then(|shadow| {
                if shadow.color.as_u32() >> 24 == 0 {
                    return None;
                }
                let shadow_color: aimer_cupid::utilities::Color = shadow.color.into();
                Some(TextShadowRequest {
                    offset_x: shadow.offset_x * scale,
                    offset_y: shadow.offset_y * scale,
                    blur: shadow.blur.max(0.0) * scale,
                    color: shadow_color.to_rgba8(),
                })
            });
            let font_size = style.font_size.max(1) as f32;
            if italic {
                canvas.set_italic(true);
            }
            let mut draw_run = |text: &str, x: f32| {
                canvas.draw_text_styled(
                    std::sync::Arc::<str>::from(text),
                    aimer_cupid::utilities::Vec2d::new(
                        x / scale + origin_offset.x,
                        fragment.baseline / scale + origin_offset.y,
                    ),
                    font_size,
                    color.into(),
                    None,
                    None,
                    TextOverflowMode::Clip,
                    TextHorizontalAlign::Left,
                    style.font_family,
                    style.font_style,
                    style.font_weight.numeric(),
                    shadow,
                    true,
                );
            };
            let has_spacing = style.letter_spacing != 0.0 || style.word_spacing != 0.0;
            if has_spacing {
                for run in &layout.paint_runs[fragment_index] {
                    draw_run(&fragment.text[run.rendered_range.clone()], run.x);
                }
            } else {
                draw_run(&fragment.text, fragment.x);
            }
            if italic {
                canvas.set_italic(false);
            }
        }

        for decoration in &layout.decorations {
            let fragment = &layout.fragments[decoration.fragment_index];
            let span = &self.spans[fragment.span_index];
            let color = decoration
                .paint
                .dedicated_color
                .unwrap_or_else(|| display_color(span, hovered_link, link_hover_color));
            let paint = decoration.paint;
            let draw_decoration = |center_y: f32| {
                canvas.draw_text_decoration(
                    aimer_cupid::utilities::Rect::new(
                        fragment.x / scale + origin_offset.x,
                        (center_y - paint.band_height / 2.0) / scale + origin_offset.y,
                        fragment.width / scale,
                        paint.band_height / scale,
                    ),
                    color.into(),
                    paint.style.id(),
                    paint.thickness / scale,
                    paint.period / scale,
                );
            };
            if paint.lines.contains(TextDecorationLine::UNDERLINE) {
                draw_decoration(decoration.underline_center);
            }
            if paint.lines.contains(TextDecorationLine::LINE_THROUGH) {
                draw_decoration(decoration.line_through_center);
            }
            if paint.lines.contains(TextDecorationLine::OVERLINE) {
                draw_decoration(decoration.overline_center);
            }
        }
    }

    pub(crate) fn visit_link_fragments(
        &self,
        layout: &PreparedLayout,
        visible_rect: Option<(f32, f32, f32, f32)>,
        mut visit: impl FnMut(&crate::TextSource, &PreparedFragment),
    ) {
        for fragment in &layout.fragments {
            if !vertical_span_is_visible(
                fragment.baseline - fragment.ascent,
                fragment.height,
                visible_rect,
            ) {
                continue;
            }
            if let Some(target) = self.spans[fragment.span_index].link.as_ref() {
                visit(target, fragment);
            }
        }
    }

    pub(crate) fn link_at_point(
        &self,
        layout: &PreparedLayout,
        point: Vec2d,
        origin: Vec2d,
        scale: f32,
    ) -> Option<crate::TextSource> {
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let x = point.x * scale - origin.x;
        let y = point.y * scale - origin.y;
        layout.fragments.iter().find_map(|fragment| {
            let target = self.spans[fragment.span_index].link.as_ref()?;
            let top = fragment.baseline - fragment.ascent;
            (fragment.x <= x
                && x <= fragment.x + fragment.width
                && top <= y
                && y <= top + fragment.height)
                .then(|| target.clone())
        })
    }

    /// The width lines are laid out against, preferring an explicit constraint
    /// over the parent's size.
    pub fn available_width(&self, ctx: &BuildContext) -> f32 {
        if ctx.box_constraint.max_width > 0.0 && ctx.box_constraint.max_width < f32::MAX {
            ctx.box_constraint.max_width
        } else {
            ctx.parent_size.width
        }
    }

    /// Drops the cached layout so the next [`Paragraph::prepare`] recomputes it.
    #[inline]
    pub fn invalidate(&self) {
        self.layout_cache.borrow_mut().take();
    }

    /// Returns the layout for the current width and scale, computing it only
    /// when the cached one no longer applies.
    pub fn prepare(&self, ctx: &BuildContext) -> Rc<PreparedLayout> {
        self.prepare_with_graphemes(ctx, true)
    }

    /// Returns the layout needed by a paint-only caller.
    ///
    /// A single untransformed run already carries source-aware interaction
    /// geometry from the shared shaper. In that case the per-grapheme fallback
    /// boxes used by older selection paths would only repeat every width
    /// measurement. Complex paragraphs only build those boxes when spacing
    /// changes each grapheme's painted position; ordinary rich spans use the
    /// clusters already produced by the shared shaper.
    pub(crate) fn prepare_for_paint(&self, ctx: &BuildContext) -> Rc<PreparedLayout> {
        self.prepare_with_graphemes(ctx, false)
    }

    /// Returns the layout a paint pass needs for `ctx`, laying the text out
    /// when the cache no longer applies.
    ///
    /// A cached layout for the same width, scale and layout generation is
    /// reused whether or not it carries per-grapheme boxes: those only add to
    /// what painting reads. Anything else is computed here. Declining to paint
    /// because the cache was retired (every rebuild advances the layout
    /// generation) would leave the element painting nothing for that frame, so
    /// text disappeared on each step of a window resize.
    pub(crate) fn layout_for_paint(&self, ctx: &BuildContext) -> Rc<PreparedLayout> {
        let width_bits = self.wrap_width(ctx).to_bits();
        let scale_bits = ctx.scale.to_bits();
        let layout_generation = layout_invalidation_generation();
        if let Some((cached, layout)) = self.layout_cache.borrow().as_ref()
            && cached.width_bits == width_bits
            && cached.scale_bits == scale_bits
            && cached.layout_generation == layout_generation
        {
            return Rc::clone(layout);
        }
        self.prepare_for_paint(ctx)
    }

    fn prepare_with_graphemes(
        &self,
        ctx: &BuildContext,
        include_graphemes: bool,
    ) -> Rc<PreparedLayout> {
        let width = self.wrap_width(ctx);
        let key = PreparedLayoutKey {
            width_bits: width.to_bits(),
            scale_bits: ctx.scale.to_bits(),
            layout_generation: layout_invalidation_generation(),
            include_graphemes,
        };
        if let Some((cached_key, layout)) = self.layout_cache.borrow().as_ref()
            && *cached_key == key
        {
            return Rc::clone(layout);
        }

        let layout = Rc::new(self.compute_layout(ctx, include_graphemes));
        *self.layout_cache.borrow_mut() = Some((key, Rc::clone(&layout)));
        layout
    }

    fn wrap_width(&self, ctx: &BuildContext) -> f32 {
        if matches!(self.overflow, TextOverflow::Wrap | TextOverflow::Ellipsis) {
            self.available_width(ctx)
        } else {
            0.0
        }
    }

    /// Returns the Aimer-owned source geometry for the simple selectable
    /// paragraph shape that can be represented by one shared font run.
    ///
    /// Rich spans, transformed text, custom line boxes, spacing, alignment,
    /// and ellipsis use the composed paragraph geometry below; this fast path
    /// preserves the single-run shaping result for the common case.
    pub(crate) fn aimer_interaction_layout(
        &self,
        ctx: &BuildContext,
    ) -> Option<Rc<aimer_cupid::text_layout::TextInteractionLayout>> {
        let [span] = self.spans.as_slice() else {
            return None;
        };
        let style = span.style;
        if self.text_indent != 0.0
            || !matches!(self.line_height, LineHeight::Normal)
            || !matches!(self.text_align, TextAlign::TopLeft)
            || !matches!(self.overflow, TextOverflow::Clip | TextOverflow::Wrap)
            || style.text_transform != TextTransform::None
            || style.letter_spacing != 0.0
            || style.word_spacing != 0.0
        {
            return None;
        }

        Some(ctx.canvas.layout_text_styled_shared(
            &span.text,
            style.font_size.max(1) as f32 * ctx.scale,
            self.wrap_width(ctx),
            style.font_family,
            style.font_style,
            style.font_weight.numeric(),
        ))
    }

    /// Composes the paragraph's source-aware interaction view from the same
    /// fragments used for painting. This is the fallback for paragraph
    /// features that cannot be represented by one unmodified Aimer run.
    #[allow(clippy::too_many_arguments)]
    fn composed_aimer_interaction_layout(
        &self,
        ctx: &BuildContext,
        fragments: &[PreparedFragment],
        graphemes: &[GraphemeBox],
        line_breaks: &[PreparedLineBreak],
        line_heights: &[f32],
        line_top: &[f32],
        line_ascent: &[f32],
        line_descent: &[f32],
        line_width: &[f32],
        width: f32,
        height: f32,
    ) -> aimer_cupid::text_layout::TextInteractionLayout {
        use aimer_cupid::text_layout::{
            ParagraphMetrics, TextCluster, TextInteractionLayout, TextLine, TextWritingMode,
        };

        let text = self
            .spans
            .iter()
            .map(|span| span.text.as_ref())
            .collect::<String>();
        let baseline_for_line = |line: usize| {
            centered_line_baseline(
                line_top[line],
                line_ascent[line],
                line_descent[line],
                line_heights[line],
            )
        };
        let mut clusters = Vec::with_capacity(graphemes.len() + line_breaks.len());
        let mut grapheme_cursor = 0;
        for (fragment_index, fragment) in fragments.iter().enumerate() {
            if fragment.text.is_empty() {
                continue;
            }
            while grapheme_cursor < graphemes.len()
                && graphemes[grapheme_cursor].fragment_index < fragment_index
            {
                grapheme_cursor += 1;
            }
            let grapheme_start = grapheme_cursor;
            while grapheme_cursor < graphemes.len()
                && graphemes[grapheme_cursor].fragment_index == fragment_index
            {
                grapheme_cursor += 1;
            }
            let fragment_graphemes = &graphemes[grapheme_start..grapheme_cursor];
            let style = self.spans[fragment.span_index].style;
            let fragment_layout = ctx.canvas.layout_text_styled_shared(
                &fragment.text,
                style.font_size.max(1) as f32 * ctx.scale,
                0.0,
                style.font_family,
                style.font_style,
                style.font_weight.numeric(),
            );
            let fragment_scale = if fragment_layout.metrics.width > 0.0 {
                fragment.width / fragment_layout.metrics.width
            } else {
                1.0
            };
            let has_spacing = style.letter_spacing.is_finite()
                && style.word_spacing.is_finite()
                && (style.letter_spacing != 0.0 || style.word_spacing != 0.0);

            if has_spacing {
                for grapheme in fragment_graphemes {
                    let level = fragment_layout
                        .clusters
                        .iter()
                        .find(|cluster| {
                            cluster.text_range.start <= grapheme.rendered_range.start
                                && grapheme.rendered_range.end <= cluster.text_range.end
                        })
                        .map_or_else(unicode_bidi::Level::ltr, |cluster| cluster.level);
                    let (start_x, end_x) = if level.is_rtl() {
                        (grapheme.x + grapheme.width, grapheme.x)
                    } else {
                        (grapheme.x, grapheme.x + grapheme.width)
                    };
                    push_composed_cluster(
                        &mut clusters,
                        TextCluster {
                            text_range: grapheme.source_range.clone(),
                            line_index: fragment.line,
                            level,
                            start_x,
                            end_x,
                            start_y: baseline_for_line(fragment.line),
                            end_y: baseline_for_line(fragment.line),
                            y: line_top[fragment.line],
                            height: line_heights[fragment.line],
                        },
                    );
                }
                continue;
            }

            let mut mapped_cluster = false;
            for cluster in &fragment_layout.clusters {
                let Some(source_range) =
                    source_range_for_rendered_range(fragment, &cluster.text_range)
                else {
                    continue;
                };
                if source_range.start == source_range.end {
                    continue;
                }
                mapped_cluster = true;
                push_composed_cluster(
                    &mut clusters,
                    TextCluster {
                        text_range: source_range,
                        line_index: fragment.line,
                        level: cluster.level,
                        start_x: fragment.x + cluster.start_x * fragment_scale,
                        end_x: fragment.x + cluster.end_x * fragment_scale,
                        start_y: baseline_for_line(fragment.line),
                        end_y: baseline_for_line(fragment.line),
                        y: line_top[fragment.line],
                        height: line_heights[fragment.line],
                    },
                );
            }

            // Keep interaction usable if a compatibility canvas cannot expose
            // clusters for a fragment even though it can measure and paint it.
            if !mapped_cluster {
                for grapheme in fragment_graphemes {
                    push_composed_cluster(
                        &mut clusters,
                        TextCluster {
                            text_range: grapheme.source_range.clone(),
                            line_index: fragment.line,
                            level: unicode_bidi::Level::ltr(),
                            start_x: grapheme.x,
                            end_x: grapheme.x + grapheme.width,
                            start_y: baseline_for_line(fragment.line),
                            end_y: baseline_for_line(fragment.line),
                            y: line_top[fragment.line],
                            height: line_heights[fragment.line],
                        },
                    );
                }
            }
        }

        for line_break in line_breaks {
            let x = clusters
                .iter()
                .rev()
                .find(|cluster| {
                    cluster.line_index == line_break.line && cluster.start_x != cluster.end_x
                })
                .map_or(line_break.x, |cluster| cluster.end_x);
            push_composed_cluster(
                &mut clusters,
                TextCluster {
                    text_range: line_break.source_range.clone(),
                    line_index: line_break.line,
                    level: unicode_bidi::Level::ltr(),
                    start_x: x,
                    end_x: x,
                    start_y: baseline_for_line(line_break.line),
                    end_y: baseline_for_line(line_break.line),
                    y: line_break.y,
                    height: line_break.height,
                },
            );
        }

        let line_count = line_heights.len().max(1);
        let mut line_ranges = vec![None; line_count];
        for cluster in &clusters {
            if cluster.text_range.start != cluster.text_range.end {
                merge_line_source_range(&mut line_ranges, cluster.line_index, &cluster.text_range);
            }
        }
        let mut hard_breaks = vec![false; line_count];
        for line_break in line_breaks {
            merge_line_source_range(&mut line_ranges, line_break.line, &line_break.source_range);
            if let Some(hard_break) = hard_breaks.get_mut(line_break.line) {
                *hard_break = true;
            }
        }

        let mut previous_end = 0;
        let lines = (0..line_count)
            .map(|line| {
                let text_range = line_ranges[line]
                    .take()
                    .unwrap_or(previous_end..previous_end);
                previous_end = text_range.end;
                let ascent_for_line = line_ascent.get(line).copied().unwrap_or(0.0);
                let descent_for_line = line_descent.get(line).copied().unwrap_or(0.0);
                let line_box_gap = (line_heights.get(line).copied().unwrap_or(0.0)
                    - ascent_for_line
                    - descent_for_line)
                    .max(0.0);
                TextLine {
                    text_range,
                    glyph_range: 0..0,
                    baseline: centered_line_baseline(
                        line_top.get(line).copied().unwrap_or(0.0),
                        ascent_for_line,
                        descent_for_line,
                        line_heights.get(line).copied().unwrap_or(0.0),
                    ),
                    width: line_width.get(line).copied().unwrap_or(0.0),
                    ascent: ascent_for_line,
                    descent: -descent_for_line,
                    line_gap: line_box_gap,
                    hard_break: hard_breaks.get(line).copied().unwrap_or(false),
                }
            })
            .collect::<Vec<_>>();

        let ascent = line_ascent.iter().copied().fold(0.0, f32::max);
        let descent = -line_descent.iter().copied().fold(0.0, f32::max);
        let line_gap = (0..line_count)
            .map(|line| {
                (line_heights.get(line).copied().unwrap_or(0.0)
                    - line_ascent.get(line).copied().unwrap_or(0.0)
                    - line_descent.get(line).copied().unwrap_or(0.0))
                .max(0.0)
            })
            .fold(0.0, f32::max);
        let line_height = line_heights.iter().copied().fold(0.0, f32::max);
        TextInteractionLayout {
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
        }
    }

    fn compute_layout(&self, ctx: &BuildContext, include_graphemes: bool) -> PreparedLayout {
        let wrap_width = self.wrap_width(ctx);
        let first_line_indent = (if self.text_indent.is_finite() {
            self.text_indent
        } else {
            0.0
        }) * ctx.scale;
        let mut layout = layout_resolved_spans_with_indent(
            &self.spans,
            wrap_width,
            first_line_indent,
            |text, style| {
                let font_size = style.font_size.max(1) as f32 * ctx.scale;
                ctx.canvas.measure_text_styled(
                    text,
                    font_size,
                    style.font_family,
                    style.font_style,
                    style.font_weight.numeric(),
                )
            },
        );
        if matches!(self.overflow, TextOverflow::Ellipsis) {
            ellipsize_first_line(&mut layout, &self.spans, wrap_width, |text, style| {
                adjusted_width(text, style, &mut |text, style| {
                    ctx.canvas.measure_text_styled(
                        text,
                        style.font_size.max(1) as f32 * ctx.scale,
                        style.font_family,
                        style.font_style,
                        style.font_weight.numeric(),
                    )
                })
            });
        }

        let mut line_ascent = vec![0.0_f32; layout.line_count];
        let mut line_descent = vec![0.0_f32; layout.line_count];
        let mut line_gap = vec![0.0_f32; layout.line_count];
        let mut line_font_size = vec![0.0_f32; layout.line_count];
        let mut line_width = vec![0.0_f32; layout.line_count];
        for fragment in &layout.fragments {
            let style = self.spans[fragment.span_index].style;
            let metrics = ctx.canvas.measure_text_metrics_styled(
                &fragment.text,
                style.font_size.max(1) as f32 * ctx.scale,
                0.0,
                style.font_family,
                style.font_style,
                style.font_weight.numeric(),
            );
            line_ascent[fragment.line] = line_ascent[fragment.line].max(metrics.ascent);
            line_descent[fragment.line] = line_descent[fragment.line].max(-metrics.descent);
            line_gap[fragment.line] = line_gap[fragment.line].max(metrics.line_gap);
            line_font_size[fragment.line] =
                line_font_size[fragment.line].max(style.font_size.max(1) as f32 * ctx.scale);
            line_width[fragment.line] = line_width[fragment.line].max(fragment.x + fragment.width);
        }
        for line_break in &layout.line_breaks {
            let style = self.spans[line_break.span_index].style;
            let metrics = ctx.canvas.measure_text_metrics_styled(
                " ",
                style.font_size.max(1) as f32 * ctx.scale,
                0.0,
                style.font_family,
                style.font_style,
                style.font_weight.numeric(),
            );
            for line in line_break.line..=(line_break.line + 1).min(layout.line_count - 1) {
                line_ascent[line] = line_ascent[line].max(metrics.ascent);
                line_descent[line] = line_descent[line].max(-metrics.descent);
                line_gap[line] = line_gap[line].max(metrics.line_gap);
            }
        }

        let natural_line_heights = (0..layout.line_count)
            .map(|line| line_ascent[line] + line_descent[line] + line_gap[line])
            .collect::<Vec<_>>();
        let line_heights = natural_line_heights
            .iter()
            .enumerate()
            .map(|(line, natural)| self.resolved_line_height(*natural, line_font_size[line]))
            .collect::<Vec<_>>();
        let mut line_top = vec![0.0; layout.line_count];
        for line in 1..layout.line_count {
            line_top[line] = line_top[line - 1] + line_heights[line - 1];
        }
        let height = layout
            .line_count
            .checked_sub(1)
            .map(|last| line_top[last] + line_heights[last])
            .unwrap_or(0.0);
        let natural_width = line_width.iter().copied().fold(0.0, f32::max);
        let width = if matches!(self.overflow, TextOverflow::Wrap) {
            wrap_width
        } else {
            natural_width
        };

        let fragments = layout
            .fragments
            .into_iter()
            .map(|fragment| {
                let line_offset = self.line_offset(width, line_width[fragment.line]);
                PreparedFragment {
                    span_index: fragment.span_index,
                    text: fragment.text,
                    source_range: fragment.source_range,
                    rendered_source_ranges: fragment.rendered_source_ranges,
                    line: fragment.line,
                    x: fragment.x + line_offset,
                    baseline: centered_line_baseline(
                        line_top[fragment.line],
                        line_ascent[fragment.line],
                        line_descent[fragment.line],
                        line_heights[fragment.line],
                    ),
                    width: fragment.width,
                    height: line_ascent[fragment.line] + line_descent[fragment.line],
                    ascent: line_ascent[fragment.line],
                    descent: line_descent[fragment.line],
                }
            })
            .collect::<Vec<_>>();
        let decorations = prepare_decorations(&fragments, &self.spans, ctx.scale);
        let aimer_interaction = self.aimer_interaction_layout(ctx);
        let needs_graphemes = include_graphemes
            || fragments.iter().any(|fragment| {
                let style = self.spans[fragment.span_index].style;
                style.letter_spacing.is_finite()
                    && style.word_spacing.is_finite()
                    && (style.letter_spacing != 0.0 || style.word_spacing != 0.0)
            });
        let (graphemes, paint_runs) = if needs_graphemes {
            self.measure_graphemes(ctx, &fragments)
        } else {
            (Vec::new(), vec![Vec::new(); fragments.len()])
        };
        let backgrounds = prepare_background_runs(&fragments, &self.spans);
        let line_breaks = layout
            .line_breaks
            .into_iter()
            .map(|line_break| {
                let line_offset = self.line_offset(width, line_width[line_break.line]);
                let x = line_width[line_break.line] + line_offset;
                PreparedLineBreak {
                    source_range: line_break.source_range,
                    line: line_break.line,
                    x,
                    y: line_top[line_break.line],
                    hit_width: (width - x).max(ctx.scale),
                    selection_width: ctx.scale,
                    height: line_heights[line_break.line],
                }
            })
            .collect::<Vec<_>>();

        let aimer_interaction = aimer_interaction.or_else(|| {
            Some(Rc::new(self.composed_aimer_interaction_layout(
                ctx,
                &fragments,
                &graphemes,
                &line_breaks,
                &line_heights,
                &line_top,
                &line_ascent,
                &line_descent,
                &line_width,
                width,
                height,
            )))
        });

        PreparedLayout {
            fragments,
            decorations,
            graphemes,
            paint_runs,
            backgrounds,
            line_breaks,
            line_heights,
            size: ResolvedSize { width, height },
            aimer_interaction,
        }
    }

    fn resolved_line_height(&self, natural: f32, font_size: f32) -> f32 {
        let requested = match self.line_height {
            LineHeight::Normal => natural,
            LineHeight::Px(value) if value.is_finite() => value,
            LineHeight::Factor(value) if value.is_finite() => value * font_size,
            LineHeight::Px(_) | LineHeight::Factor(_) => natural,
        };
        requested.max(natural).max(0.0)
    }

    fn line_offset(&self, width: f32, line_width: f32) -> f32 {
        match self.text_align {
            TextAlign::TopCenter | TextAlign::MidCenter | TextAlign::BotCenter => {
                (width - line_width) / 2.0
            }
            TextAlign::TopRight | TextAlign::MidRight | TextAlign::BotRight => width - line_width,
            _ => 0.0,
        }
    }

    fn measure_graphemes(
        &self,
        ctx: &BuildContext,
        fragments: &[PreparedFragment],
    ) -> (Vec<GraphemeBox>, Vec<Vec<PreparedTextRun>>) {
        let mut graphemes = Vec::new();
        let mut paint_runs = vec![Vec::new(); fragments.len()];
        for (fragment_index, fragment) in fragments.iter().enumerate() {
            let style = self.spans[fragment.span_index].style;
            let font_size = style.font_size.max(1) as f32 * ctx.scale;
            let mut x = fragment.x;
            let rendered_graphemes = fragment.text.graphemes(true).collect::<Vec<_>>();
            for (index, (offset, grapheme)) in fragment.text.grapheme_indices(true).enumerate() {
                let width = adjusted_grapheme_advance(
                    grapheme,
                    &style,
                    index,
                    rendered_graphemes.len(),
                    &mut |text, style| {
                        ctx.canvas.measure_text_styled(
                            text,
                            font_size,
                            style.font_family,
                            style.font_style,
                            style.font_weight.numeric(),
                        )
                    },
                );
                paint_runs[fragment_index].push(PreparedTextRun {
                    rendered_range: offset..offset + grapheme.len(),
                    x,
                });
                let source_range = if fragment.rendered_source_ranges.is_empty() {
                    fragment
                        .source_range
                        .as_ref()
                        .map(|range| range.start + offset..range.start + offset + grapheme.len())
                } else {
                    fragment.rendered_source_ranges.get(index).cloned()
                };
                let Some(source_range) = source_range else {
                    x += width;
                    continue;
                };
                graphemes.push(GraphemeBox {
                    source_range,
                    fragment_index,
                    rendered_range: offset..offset + grapheme.len(),
                    x,
                    width,
                });
                x += width;
            }
        }
        (graphemes, paint_runs)
    }
}

fn source_range_for_rendered_range(
    fragment: &PreparedFragment,
    rendered_range: &Range<usize>,
) -> Option<Range<usize>> {
    if rendered_range.start >= rendered_range.end {
        return None;
    }

    if fragment.rendered_source_ranges.is_empty() {
        let source_range = fragment.source_range.as_ref()?;
        let start = source_range.start.checked_add(rendered_range.start)?;
        let end = source_range.start.checked_add(rendered_range.end)?;
        return (end <= source_range.end).then_some(start..end);
    }

    let mut source_start = usize::MAX;
    let mut source_end = 0;
    for (index, (offset, grapheme)) in fragment.text.grapheme_indices(true).enumerate() {
        let grapheme_end = offset + grapheme.len();
        if offset >= rendered_range.end || grapheme_end <= rendered_range.start {
            continue;
        }
        let Some(source_range) = fragment.rendered_source_ranges.get(index) else {
            continue;
        };
        source_start = source_start.min(source_range.start);
        source_end = source_end.max(source_range.end);
    }
    (source_start != usize::MAX).then_some(source_start..source_end)
}

fn push_composed_cluster(
    clusters: &mut Vec<aimer_cupid::text_layout::TextCluster>,
    cluster: aimer_cupid::text_layout::TextCluster,
) {
    if let Some(previous) = clusters.last_mut()
        && previous.line_index == cluster.line_index
        && previous.text_range == cluster.text_range
        && previous.level == cluster.level
    {
        if previous.level.is_rtl() {
            previous.start_x = previous.start_x.max(cluster.start_x);
            previous.end_x = previous.end_x.min(cluster.end_x);
        } else {
            previous.start_x = previous.start_x.min(cluster.start_x);
            previous.end_x = previous.end_x.max(cluster.end_x);
        }
        previous.y = previous.y.min(cluster.y);
        previous.height = previous.height.max(cluster.height);
    } else {
        clusters.push(cluster);
    }
}

fn merge_line_source_range(
    line_ranges: &mut [Option<Range<usize>>],
    line: usize,
    source_range: &Range<usize>,
) {
    let Some(line_range) = line_ranges.get_mut(line) else {
        return;
    };
    if let Some(line_range) = line_range {
        line_range.start = line_range.start.min(source_range.start);
        line_range.end = line_range.end.max(source_range.end);
    } else {
        *line_range = Some(source_range.clone());
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use aimer_style::{
        LineHeight, TextDecoration, TextDecorationLine, TextDecorationStyle, TextStyle,
    };
    use aimer_widget::base::{BuildContext, Color};

    use super::{Paragraph, display_color};
    use crate::text_span::ResolvedTextSpan;

    #[test]
    fn hovered_link_uses_the_configured_color_only_for_its_spans() {
        let hovered = crate::TextSource::from("https://aimer.dev");
        let hover_color = Color::Hex(0x388BFD);
        let linked = ResolvedTextSpan {
            text: crate::TextSource::from("Aimer"),
            style: TextStyle::new().color(Color::Hex(0x0969DA)),
            link: Some(hovered.clone()),
        };
        let plain = ResolvedTextSpan::plain(Rc::from(" docs"), TextStyle::default());

        assert_eq!(
            display_color(&linked, Some(&hovered), Some(hover_color)),
            hover_color
        );
        assert_eq!(
            display_color(&plain, Some(&hovered), Some(hover_color)),
            plain.style.color
        );
        assert_eq!(
            display_color(&plain, None, Some(hover_color)),
            plain.style.color
        );
    }

    #[test]
    fn line_height_resolves_natural_absolute_and_factor_forms() {
        let paragraph = super::Paragraph::with_layout(
            Vec::new(),
            aimer_style::TextAlign::TopLeft,
            aimer_style::TextOverflow::Clip,
            aimer_style::LineHeight::Px(40.0),
            0.0,
        );
        assert_eq!(paragraph.resolved_line_height(20.0, 16.0), 40.0);

        let factor = super::Paragraph::with_layout(
            Vec::new(),
            aimer_style::TextAlign::TopLeft,
            aimer_style::TextOverflow::Clip,
            aimer_style::LineHeight::Factor(1.5),
            0.0,
        );
        assert_eq!(factor.resolved_line_height(20.0, 16.0), 24.0);

        let normal = super::Paragraph::new(
            Vec::new(),
            aimer_style::TextAlign::TopLeft,
            aimer_style::TextOverflow::Clip,
        );
        assert_eq!(normal.resolved_line_height(20.0, 16.0), 20.0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn prepared_layout_caches_decoration_geometry_per_fragment() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_style::{TextAlign, TextOverflow};
        use aimer_widget::base::{BuildContext, WindowHandle};

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let paragraph = Paragraph::with_layout(
            vec![ResolvedTextSpan::plain(
                Rc::from("first\nsecond"),
                TextStyle::new().font_size(16).text_decoration(
                    TextDecoration::new()
                        .line(TextDecorationLine::UNDERLINE | TextDecorationLine::OVERLINE)
                        .style(TextDecorationStyle::Dashed)
                        .thickness(2.0)
                        .offset(1.0),
                ),
            )],
            TextAlign::TopLeft,
            TextOverflow::Clip,
            LineHeight::Px(40.0),
            0.0,
        );

        let first = paragraph.prepare(&context);
        let second = paragraph.prepare(&context);

        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(first.decorations.len(), 2);
        assert_eq!(first.decorations[0].fragment_index, 0);
        assert_eq!(first.decorations[1].fragment_index, 1);
        assert_eq!(
            first.decorations[0].paint.style,
            TextDecorationStyle::Dashed
        );
        assert_eq!(first.decorations[0].paint.thickness, 2.0);
        assert_eq!(first.decorations[0].paint.offset, 1.0);
        assert!(
            (first.decorations[1].underline_center - first.decorations[0].underline_center - 40.0)
                .abs()
                < 0.001
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn decorations_resolve_the_hover_color_when_recorded() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{Canvas, FrameCanvas, InnerCanvas};
        use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
        use aimer_style::{TextAlign, TextDecoration, TextDecorationLine, TextOverflow};
        use aimer_widget::base::WindowHandle;

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let target = crate::TextSource::from("target");
        let paragraph = Paragraph::with_layout(
            vec![ResolvedTextSpan {
                text: crate::TextSource::from("link"),
                style: TextStyle::new()
                    .text_decoration(TextDecoration::new().line(TextDecorationLine::UNDERLINE)),
                link: Some(target.clone()),
            }],
            TextAlign::TopLeft,
            TextOverflow::Wrap,
            LineHeight::Normal,
            0.0,
        );
        let layout = paragraph.prepare(&context);

        let decoration_color = |hover: Color| {
            let tree = RenderTree::new();
            let root = tree.add_root(Rect::new(0.0, 0.0, 200.0, 100.0)).unwrap();
            let retained = tree.context(root).unwrap();
            context.with_local_v2_paint_context(retained, |context| {
                let canvas = Canvas::of(context);
                paragraph.record_retained_v2_foreground(
                    &canvas,
                    &layout,
                    1.0,
                    Vec2d::default(),
                    Some(&target),
                    Some(hover),
                );
            });
            tree.draw_list_snapshot(root)
                .unwrap()
                .commands
                .iter()
                .find_map(|command| match command {
                    DrawCommand::DrawTextDecoration { color, .. } => Some(*color),
                    _ => None,
                })
                .expect("the decorated paragraph should emit a decoration")
        };

        let red = Color::Rgba(255, 0, 0, 255);
        let blue = Color::Rgba(0, 0, 255, 255);
        let expected_red: aimer_cupid::utilities::Color = red.into();
        let expected_blue: aimer_cupid::utilities::Color = blue.into();
        assert_eq!(decoration_color(red).to_array(), expected_red.to_array());
        assert_eq!(decoration_color(blue).to_array(), expected_blue.to_array());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn explicit_line_height_controls_every_line_box_and_final_height() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_style::{TextAlign, TextOverflow};
        use aimer_widget::base::{BuildContext, WindowHandle};

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let paragraph = Paragraph::with_layout(
            vec![ResolvedTextSpan::plain(
                Rc::from("first\nsecond"),
                TextStyle::new().font_size(16),
            )],
            TextAlign::TopLeft,
            TextOverflow::Clip,
            LineHeight::Px(40.0),
            0.0,
        );

        let layout = paragraph.prepare(&context);

        assert_eq!(layout.line_heights, vec![40.0, 40.0]);
        assert_eq!(layout.size.height, 80.0);
        let baseline_step = layout.fragments[1].baseline - layout.fragments[0].baseline;
        assert!(
            (baseline_step - 40.0).abs() < 1e-3,
            "baselines are {baseline_step} apart, expected 40"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn aimer_interaction_composes_rich_spacing_transform_and_line_box_geometry() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_cupid::text_layout::TextWritingMode;
        use aimer_style::{TextAlign, TextOverflow, TextTransform};
        use aimer_widget::base::{BuildContext, WindowHandle};

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let paragraph = Paragraph::with_layout(
            vec![
                ResolvedTextSpan::plain(
                    Rc::from("ab "),
                    TextStyle::new().font_size(16).letter_spacing(1.0),
                ),
                ResolvedTextSpan::plain(
                    Rc::from("ß"),
                    TextStyle::new()
                        .font_size(16)
                        .text_transform(TextTransform::Uppercase),
                ),
            ],
            TextAlign::TopCenter,
            TextOverflow::Wrap,
            LineHeight::Px(40.0),
            4.0,
        );

        let layout = paragraph.prepare(&context);
        let interaction = layout
            .aimer_interaction
            .as_ref()
            .expect("complex paragraphs must use the shared interaction layout");

        assert_eq!(interaction.text, "ab ß");
        assert_eq!(interaction.writing_mode, TextWritingMode::HorizontalTb);
        assert_eq!(interaction.lines.len(), 1);
        assert_eq!(interaction.metrics.line_count, 1);
        assert!((interaction.metrics.height - layout.size.height).abs() < 0.001);
        assert!(interaction.clusters.iter().all(|cluster| {
            cluster.start_y == cluster.end_y
                && interaction
                    .lines
                    .get(cluster.line_index)
                    .is_some_and(|line| cluster.start_y == line.baseline)
        }));
        assert!(interaction.clusters.iter().any(|cluster| {
            cluster.text_range == (0..1) && cluster.height == layout.line_heights[0]
        }));
        assert!(
            interaction
                .clusters
                .iter()
                .any(|cluster| cluster.text_range == (3..5))
        );
        assert!(
            interaction
                .clusters
                .iter()
                .filter(|cluster| cluster.text_range == (3..5))
                .count()
                == 1
        );
        assert!(
            interaction
                .clusters
                .first()
                .is_some_and(|cluster| cluster.start_x > 0.0)
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn aimer_interaction_ellipsis_stops_at_the_visible_source_boundary() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_style::{TextAlign, TextOverflow};
        use aimer_widget::base::{BuildContext, WindowHandle};

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 80.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(80, 100), 1.0),
            runtime.handle().clone(),
        );
        let paragraph = Paragraph::with_layout(
            vec![ResolvedTextSpan::plain(
                Rc::from("hello world this is a long paragraph"),
                TextStyle::new().font_size(16),
            )],
            TextAlign::TopLeft,
            TextOverflow::Ellipsis,
            LineHeight::Normal,
            0.0,
        );

        let layout = paragraph.prepare(&context);
        let interaction = layout
            .aimer_interaction
            .as_ref()
            .expect("ellipsis must use the shared interaction layout");

        assert_eq!(interaction.lines.len(), 1);
        assert!(interaction.lines[0].text_range.end < interaction.text.len());
        assert!(
            interaction
                .caret_geometry(interaction.lines[0].text_range.end)
                .is_some()
        );
        assert!(
            interaction
                .clusters
                .iter()
                .all(|cluster| cluster.text_range.end <= interaction.lines[0].text_range.end)
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn aimer_interaction_composes_hard_breaks_across_spans() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_style::{TextAlign, TextOverflow};
        use aimer_widget::base::{BuildContext, WindowHandle};

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let paragraph = Paragraph::with_layout(
            vec![
                ResolvedTextSpan::plain(Rc::from("first\n"), TextStyle::new().font_size(16)),
                ResolvedTextSpan::plain(Rc::from("second"), TextStyle::new().font_size(20)),
            ],
            TextAlign::TopLeft,
            TextOverflow::Clip,
            LineHeight::Px(40.0),
            0.0,
        );

        let layout = paragraph.prepare(&context);
        let interaction = layout
            .aimer_interaction
            .as_ref()
            .expect("hard-break paragraphs must use the shared interaction layout");

        assert_eq!(interaction.lines.len(), 2);
        assert!(
            interaction
                .clusters
                .iter()
                .any(|cluster| cluster.text_range == (5..6) && cluster.start_x == cluster.end_x)
        );
        assert_eq!(interaction.lines[0].text_range, 0..6);
        assert_eq!(interaction.lines[1].text_range, 6..12);
        assert!(interaction.lines[0].hard_break);
        assert!(!interaction.lines[1].hard_break);
        let first_y = interaction
            .clusters
            .iter()
            .find(|cluster| cluster.line_index == 0 && cluster.start_x != cluster.end_x)
            .map(|cluster| cluster.y)
            .unwrap();
        let second_y = interaction
            .clusters
            .iter()
            .find(|cluster| cluster.line_index == 1 && cluster.start_x != cluster.end_x)
            .map(|cluster| cluster.y)
            .unwrap();
        assert_eq!(second_y - first_y, 40.0);
        assert_eq!(interaction.line_index_at_y(first_y + 39.0), Some(0));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn paint_layout_keeps_shared_interaction_without_grapheme_boxes() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_style::{TextAlign, TextOverflow};
        use aimer_widget::base::WindowHandle;

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let paragraph = Paragraph::new(
            vec![ResolvedTextSpan::plain(
                Rc::from("paint-only paragraph"),
                TextStyle::new().font_size(16),
            )],
            TextAlign::TopLeft,
            TextOverflow::Wrap,
        );

        let paint_layout = paragraph.prepare_for_paint(&context);

        assert!(paint_layout.aimer_interaction.is_some());
        assert!(paint_layout.graphemes.is_empty());
        assert_eq!(paint_layout.paint_runs.len(), paint_layout.fragments.len());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn paint_layout_skips_graphemes_for_unspaced_rich_spans() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_style::{FontStyle, FontWeight, TextAlign, TextOverflow};
        use aimer_widget::base::WindowHandle;

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let paragraph = Paragraph::new(
            vec![
                ResolvedTextSpan::plain(
                    Rc::from("plain rich span"),
                    TextStyle::new().font_size(16),
                ),
                ResolvedTextSpan::plain(
                    Rc::from(" with emphasis"),
                    TextStyle::new()
                        .font_size(16)
                        .font_style(FontStyle::Italic)
                        .font_weight(FontWeight::Bold),
                ),
            ],
            TextAlign::TopLeft,
            TextOverflow::Wrap,
        );

        let paint_layout = paragraph.prepare_for_paint(&context);
        let interaction = paint_layout
            .aimer_interaction
            .as_ref()
            .expect("rich spans must retain interaction geometry");

        assert!(paint_layout.graphemes.is_empty());
        assert!(paint_layout.paint_runs.iter().all(Vec::is_empty));
        assert_eq!(interaction.text, "plain rich span with emphasis");
        assert!(!interaction.clusters.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cached_grapheme_boxes_reproduce_the_measured_widths() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_style::{TextAlign, TextOverflow};
        use aimer_widget::base::{BuildContext, WindowHandle};

        use super::Paragraph;

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let paragraph = Paragraph::new(
            vec![ResolvedTextSpan::plain(
                Rc::from("héllo"),
                TextStyle::new().font_size(20),
            )],
            TextAlign::TopLeft,
            TextOverflow::Clip,
        );

        let layout = paragraph.prepare(&context);

        let fragment = &layout.fragments[0];
        assert_eq!(layout.graphemes.len(), 5);
        assert_eq!(layout.graphemes[0].source_range, 0..1);
        assert_eq!(layout.graphemes[1].source_range, 1..3);
        assert_eq!(layout.graphemes[0].x, fragment.x);
        for boxes in layout.graphemes.windows(2) {
            assert!((boxes[0].x + boxes[0].width - boxes[1].x).abs() < 0.01);
        }
        let measured = layout
            .graphemes
            .iter()
            .map(|grapheme| grapheme.width)
            .sum::<f32>();
        assert!((measured - fragment.width).abs() < 1.0);
    }
}
