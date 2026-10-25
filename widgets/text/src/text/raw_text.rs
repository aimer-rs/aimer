use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_cupid::text_pipeline::text_layout::TextHorizontalAlign;
use aimer_macro::{EventElement, Rebuildable};
use aimer_style::*;
use aimer_utils::debug;
use aimer_widget::base::BuildContext;
use aimer_widget::{TextOverflowMode, *};

use crate::paragraph::Paragraph;
use crate::paragraph::geometry::{DecorationPaint, prepare_decoration_paint};
use crate::text_source::TextSource;
use crate::text_span::ResolvedTextSpan;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlainDecorationCacheKey {
    width_bits: u32,
    height_bits: u32,
    scale_bits: u32,
    font_size_bits: u32,
    layout_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlainDecorationLine {
    x: f32,
    width: f32,
    underline_center: f32,
    line_through_center: f32,
    overline_center: f32,
}

struct PlainDecorationCache {
    key: PlainDecorationCacheKey,
    paint: DecorationPaint,
    lines: Vec<PlainDecorationLine>,
}

#[derive(Default)]
struct RawTextAuxCache {
    paragraph: Option<Paragraph>,
    plain_decorations: Option<PlainDecorationCache>,
    /// The parent size, as `[width, height]` bits, the text was last recorded
    /// against. See [`RawTextWidget::local_v2_paint_needs_recording`].
    recorded_parent_size: Option<[u32; 2]>,
}

/// Low-level element that lays out and paints one run of text.
///
/// [`crate::Text`] is the usual constructor. Direct construction requires
/// callers to provide a fresh [`LayoutCache`] and typeface slot in addition to
/// the text, style, and alignment.
#[derive(Rebuildable, EventElement)]
pub struct RawTextWidget {
    pub text: TextSource,
    pub text_style: TextStyle,
    pub text_align: TextAlign,
    pub line_height: LineHeight,
    pub text_indent: f32,
    pub cache: LayoutCache,
    pub _typeface: Cell<Option<()>>,
}

struct LocalV2TextPaint {
    scale: f32,
    width: f32,
    height: f32,
    font_size: f32,
    baseline: f32,
    overflow: TextOverflowMode,
    horizontal_align: TextHorizontalAlign,
    origin_offset: Vec2d,
}

struct LocalV2ParagraphPaint {
    layout: Rc<crate::paragraph::PreparedLayout>,
    scale: f32,
    width: f32,
    height: f32,
    outsets: [f32; 4],
    vertical_offset: f32,
}

impl RawTextWidget {
    pub(crate) fn font_size(&self, scale: f32) -> f32 {
        let base = if self.text_style.font_size == 0 {
            14.0
        } else {
            self.text_style.font_size as f32
        };
        base * scale
    }

    /// Whether the plain glyph recorder covers this text. Paragraph layout and
    /// decorations use the prepared-fragment recorder instead, whose outsets
    /// are part of the retained node bounds.
    fn uses_plain_glyph_paint(&self) -> bool {
        !self.uses_paragraph_layout() && self.text_style.text_decoration.line.is_none()
    }

    fn local_v2_paint_data(&self, ctx: &BuildContext) -> Option<LocalV2TextPaint> {
        if !self.uses_plain_glyph_paint() {
            return None;
        }

        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let overflow = match self.text_style.text_overflow {
            TextOverflow::Clip => TextOverflowMode::Clip,
            TextOverflow::Ellipsis => TextOverflowMode::Ellipsis,
            TextOverflow::Wrap => TextOverflowMode::Wrap,
            TextOverflow::Value(_) => TextOverflowMode::Clip,
        };

        let size = self.computed_size(ctx);
        let width = ctx.parent_size.width;
        let height = ctx.parent_size.height;
        if !width.is_finite()
            || !height.is_finite()
            || !size.width.is_finite()
            || !size.height.is_finite()
            || width < 0.0
            || height < 0.0
            || size.width < 0.0
            || size.height < 0.0
            || (overflow == TextOverflowMode::Wrap && width != size.width)
        {
            return None;
        }

        let font_size = self.font_size(scale);
        if !font_size.is_finite() {
            return None;
        }
        let max_width = if overflow == TextOverflowMode::Wrap {
            width
        } else {
            0.0
        };
        let metrics = ctx.canvas.measure_text_metrics_styled(
            self.text.as_str(),
            font_size,
            max_width,
            self.text_style.font_family,
            self.text_style.font_style,
            self.text_style.font_weight.numeric(),
        );
        let baseline_offset = metrics.ascent + metrics.line_gap * 0.5;
        let baseline =
            vertical_alignment_baseline(self.text_align, height, metrics.height, baseline_offset);
        if !baseline.is_finite() || !metrics.height.is_finite() {
            return None;
        }
        let horizontal_align = match self.text_align {
            TextAlign::TopLeft | TextAlign::MidLeft | TextAlign::BotLeft => {
                TextHorizontalAlign::Left
            }
            TextAlign::TopCenter | TextAlign::MidCenter | TextAlign::BotCenter => {
                TextHorizontalAlign::Center
            }
            TextAlign::TopRight | TextAlign::MidRight | TextAlign::BotRight => {
                TextHorizontalAlign::Right
            }
        };
        let italic_outset = if self.text_style.font_style == FontStyle::Italic {
            font_size / scale * 0.5
        } else {
            0.0
        };

        Some(LocalV2TextPaint {
            scale,
            width,
            height,
            font_size,
            baseline,
            overflow,
            horizontal_align,
            origin_offset: Vec2d {
                x: italic_outset,
                y: 0.0,
            },
        })
    }

    fn local_v2_paragraph_paint_data(&self, ctx: &BuildContext) -> Option<LocalV2ParagraphPaint> {
        if !self.uses_paragraph_layout() {
            return None;
        }
        let scale = ctx.scale;
        let (width, height) = self
            .with_paragraph(|paragraph| (paragraph.available_width(ctx), ctx.parent_size.height));
        if !scale.is_finite()
            || scale <= 0.0
            || !width.is_finite()
            || !height.is_finite()
            || width < 0.0
            || height < 0.0
            || width > 1_000_000.0
            || height > 1_000_000.0
        {
            return None;
        }
        let layout = self.with_paragraph(|paragraph| {
            paragraph
                .supports_retained_v2_rich_text()
                .then(|| paragraph.layout_for_paint(ctx))
        })?;
        if !layout.size.width.is_finite()
            || !layout.size.height.is_finite()
            || layout.size.width < 0.0
            || layout.size.height < 0.0
            || layout.size.width > 1_000_000.0
            || layout.size.height > 1_000_000.0
            || layout.fragments.iter().any(|fragment| {
                !fragment.x.is_finite()
                    || !fragment.baseline.is_finite()
                    || !fragment.width.is_finite()
                    || !fragment.height.is_finite()
                    || fragment.width < 0.0
                    || fragment.height < 0.0
            })
        {
            return None;
        }
        let paint_outsets =
            self.with_paragraph(|paragraph| paragraph.retained_v2_paint_outsets(&layout, scale));
        let vertical_offset = match self.text_align {
            TextAlign::TopLeft | TextAlign::TopCenter | TextAlign::TopRight => 0.0,
            TextAlign::MidLeft | TextAlign::MidCenter | TextAlign::MidRight => {
                (height - layout.size.height).max(0.0) / 2.0
            }
            TextAlign::BotLeft | TextAlign::BotCenter | TextAlign::BotRight => {
                (height - layout.size.height).max(0.0)
            }
        };
        Some(LocalV2ParagraphPaint {
            layout,
            scale,
            width,
            height,
            outsets: paint_outsets,
            vertical_offset,
        })
    }

    /// Records this widget's text into an existing local v2 canvas.
    ///
    /// `origin_offset` is in logical pixels within the caller's element list.
    /// The callback runs after span backgrounds and before glyphs, which lets
    /// editing fields insert selection fills in the same paint order. Passing
    /// `include_paint_outsets = false` keeps the text within an enclosing
    /// viewport that supplies its own clip.
    #[doc(hidden)]
    pub fn record_local_v2_into(
        &self,
        canvas: &aimer_canvas::Canvas,
        ctx: &BuildContext,
        origin_offset: Vec2d,
        include_paint_outsets: bool,
        before_foreground: impl FnOnce(&aimer_canvas::Canvas),
    ) -> bool {
        self.note_recorded_parent_size(ctx);
        let mut before_foreground = Some(before_foreground);
        if let Some(data) = self.local_v2_paragraph_paint_data(ctx) {
            let outset_left = if include_paint_outsets {
                data.outsets[0]
            } else {
                0.0
            };
            let outset_top = if include_paint_outsets {
                data.outsets[1]
            } else {
                0.0
            };
            let draw_origin = Vec2d {
                x: origin_offset.x + outset_left,
                y: origin_offset.y + outset_top + data.vertical_offset / data.scale,
            };
            let clipped = self.with_paragraph(|paragraph| paragraph.needs_clip());
            if clipped {
                canvas.push_clip(
                    aimer_cupid::utilities::Rect::new(
                        draw_origin.x,
                        draw_origin.y,
                        data.width / data.scale,
                        data.height / data.scale,
                    ),
                    [0.0; 4],
                );
            }
            self.with_paragraph(|paragraph| {
                paragraph.record_retained_v2_backgrounds(
                    canvas,
                    &data.layout,
                    data.scale,
                    draw_origin,
                );
            });
            before_foreground
                .take()
                .expect("foreground callback is used once")(canvas);
            self.with_paragraph(|paragraph| {
                paragraph.record_retained_v2_foreground(
                    canvas,
                    &data.layout,
                    data.scale,
                    draw_origin,
                    None,
                    None,
                );
            });
            if clipped {
                canvas.pop_clip();
            }
            return true;
        }

        let Some(data) = self.local_v2_paint_data(ctx) else {
            return false;
        };
        let draw_origin = Vec2d {
            x: origin_offset.x
                + if include_paint_outsets {
                    data.origin_offset.x
                } else {
                    0.0
                },
            y: origin_offset.y,
        };
        before_foreground
            .take()
            .expect("foreground callback is used once")(canvas);
        canvas.draw_text_styled(
            std::sync::Arc::<str>::from(self.text.as_str()),
            aimer_cupid::utilities::Vec2d::new(
                draw_origin.x,
                draw_origin.y + data.baseline / data.scale,
            ),
            data.font_size / data.scale,
            self.text_style.color.into(),
            Some(data.width / data.scale),
            Some(data.height / data.scale),
            data.overflow,
            data.horizontal_align,
            self.text_style.font_family,
            self.text_style.font_style,
            self.text_style.font_weight.numeric(),
            None,
            true,
        );
        true
    }

    /// Returns the source-aware interaction layout used by this text widget.
    ///
    /// The returned geometry is produced by the same styled shaper and
    /// paragraph layout that paints the widget. Callers that place carets or
    /// resolve pointer offsets can therefore retain cluster boundaries,
    /// fallback-font advances, wrapping, and paragraph style adjustments
    /// without measuring a second approximation of the text.
    pub fn interaction_layout(
        &self,
        ctx: &BuildContext,
    ) -> Rc<aimer_cupid::text_layout::TextInteractionLayout> {
        if self.uses_paragraph_layout() {
            return self.with_paragraph(|paragraph| {
                paragraph
                    .prepare(ctx)
                    .aimer_interaction
                    .clone()
                    .expect("a prepared paragraph always has interaction geometry")
            });
        }

        let max_width = matches!(
            self.text_style.text_overflow,
            TextOverflow::Wrap | TextOverflow::Ellipsis
        )
        .then_some(ctx.parent_size.width)
        .unwrap_or(0.0);
        Rc::new(ctx.canvas.layout_text_styled(
            &self.text,
            self.font_size(ctx.scale),
            max_width,
            self.text_style.font_family,
            self.text_style.font_style,
            self.text_style.font_weight.numeric(),
        ))
    }

    fn parent_size_bits(ctx: &BuildContext) -> [u32; 2] {
        [
            ctx.parent_size.width.to_bits(),
            ctx.parent_size.height.to_bits(),
        ]
    }

    /// Remembers the parent size this text is being recorded against.
    fn note_recorded_parent_size(&self, ctx: &BuildContext) {
        let bits = Self::parent_size_bits(ctx);
        self.cache.with_extra(|slot: &mut Option<RawTextAuxCache>| {
            slot.get_or_insert_with(RawTextAuxCache::default)
                .recorded_parent_size = Some(bits);
        });
    }

    fn uses_paragraph_layout(&self) -> bool {
        self.line_height != LineHeight::Normal
            || self.text_indent != 0.0
            || self.text_style.text_transform != TextTransform::None
            || self.text_style.letter_spacing != 0.0
            || self.text_style.word_spacing != 0.0
            || self.text_style.text_shadow.is_some()
            || self.text_style.background_color.is_some()
            || !self.text_style.text_decoration.line.is_none()
    }

    fn with_paragraph<R>(&self, callback: impl FnOnce(&Paragraph) -> R) -> R {
        self.cache.with_extra(|slot: &mut Option<RawTextAuxCache>| {
            let auxiliary = slot.get_or_insert_with(RawTextAuxCache::default);
            let paragraph = auxiliary.paragraph.get_or_insert_with(|| {
                Paragraph::with_layout(
                    vec![ResolvedTextSpan::plain_source(
                        self.text.clone(),
                        self.text_style,
                    )],
                    self.text_align,
                    self.text_style.text_overflow,
                    self.line_height,
                    self.text_indent,
                )
            });
            callback(paragraph)
        })
    }
}

impl Drawable for RawTextWidget {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.local_v2_paint_data(ctx).is_some() || self.local_v2_paragraph_paint_data(ctx).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        assert!(self.record_local_v2_into(&canvas, ctx, Vec2d { x: 0.0, y: 0.0 }, true, |_| {},));
        canvas.finish();
    }

    /// The recorded text aligns, wraps and clips inside its parent's box, not
    /// its own, so it goes stale when the parent changes size while the text
    /// does not: centered text in a window that was resized is still where the
    /// old window's middle was. The node's own bounds say nothing about this.
    ///
    /// Runs for every text node each frame, so it is one borrow and two integer
    /// compares. A text that was never recorded has nothing to refresh; its
    /// first recording is requested by the render node itself.
    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        let now = Self::parent_size_bits(ctx);
        self.cache.with_extra(|slot: &mut Option<RawTextAuxCache>| {
            slot.as_ref()
                .and_then(|aux| aux.recorded_parent_size)
                .is_some_and(|recorded| recorded != now)
        })
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        if let Some(data) = self.local_v2_paragraph_paint_data(ctx) {
            return data
                .outsets
                .iter()
                .any(|outset| *outset > 0.0)
                .then_some(data.outsets);
        }
        let data = self.local_v2_paint_data(ctx)?;
        (data.origin_offset.x > 0.0).then_some([
            data.origin_offset.x,
            0.0,
            data.origin_offset.x,
            0.0,
        ])
    }
}

impl VisitorElement for RawTextWidget {
    fn debug_name(&self) -> &'static str {
        "RawTextWidget"
    }
}

fn horizontal_alignment_offset(alignment: TextAlign, width: f32, text_width: f32) -> f32 {
    let remaining_width = (width - text_width).max(0.0);
    match alignment {
        TextAlign::TopLeft | TextAlign::MidLeft | TextAlign::BotLeft => 0.0,
        TextAlign::TopCenter | TextAlign::MidCenter | TextAlign::BotCenter => remaining_width / 2.0,
        TextAlign::TopRight | TextAlign::MidRight | TextAlign::BotRight => remaining_width,
    }
}

fn vertical_alignment_baseline(
    alignment: TextAlign,
    height: f32,
    text_height: f32,
    ascent: f32,
) -> f32 {
    match alignment {
        TextAlign::TopLeft | TextAlign::TopCenter | TextAlign::TopRight => ascent,
        TextAlign::MidLeft | TextAlign::MidCenter | TextAlign::MidRight => {
            (height - text_height).max(0.0) / 2.0 + ascent
        }
        TextAlign::BotLeft | TextAlign::BotCenter | TextAlign::BotRight => {
            (height - text_height).max(0.0) + ascent
        }
    }
}

impl LayoutElement for RawTextWidget {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        if self.uses_paragraph_layout() {
            return self.with_paragraph(|paragraph| paragraph.prepare_for_paint(ctx).size);
        }
        let scale_bits = ctx.scale.to_bits();
        if let Some(cached) = self.cache.get_computed(ctx.box_constraint, scale_bits) {
            return cached;
        }

        let font_size = self.font_size(ctx.scale);

        let result = match self.text_style.text_overflow {
            TextOverflow::Wrap => {
                let width = if ctx.box_constraint.max_width > 0.0 {
                    ctx.box_constraint.max_width
                } else {
                    ctx.parent_size.width
                };
                let metrics = ctx.canvas.measure_text_metrics_styled(
                    &self.text,
                    font_size,
                    width,
                    self.text_style.font_family,
                    self.text_style.font_style,
                    self.text_style.font_weight.numeric(),
                );

                ResolvedSize {
                    width,
                    height: metrics.height.ceil(),
                }
            }
            _ => {
                let metrics = ctx.canvas.measure_text_metrics_styled(
                    &self.text,
                    font_size,
                    0.0,
                    self.text_style.font_family,
                    self.text_style.font_style,
                    self.text_style.font_weight.numeric(),
                );
                ResolvedSize {
                    width: metrics.width.ceil(),
                    height: metrics.height.ceil(),
                }
            }
        };

        self.cache
            .set_computed(ctx.box_constraint, scale_bits, result);
        result
    }

    #[inline]
    fn is_layout_stable(&self) -> bool {
        true
    }
    fn invalidate_layout(&self) {
        self.cache.invalidate();
        self.cache.with_extra(|slot: &mut Option<RawTextAuxCache>| {
            if let Some(auxiliary) = slot.as_mut() {
                if let Some(paragraph) = auxiliary.paragraph.as_ref() {
                    paragraph.invalidate();
                }
                auxiliary.plain_decorations = None;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use aimer_style::{LineHeight, TextAlign, TextShadow, TextStyle};
    use aimer_widget::{Drawable, LayoutCache, LayoutElement};

    use crate::text_source::TextSource;

    use super::{RawTextWidget, horizontal_alignment_offset, vertical_alignment_baseline};

    #[test]
    fn advanced_text_reuses_its_paragraph_storage() {
        let widget = RawTextWidget {
            text: TextSource::Static("A long transformed paragraph"),
            text_style: TextStyle::default(),
            text_align: TextAlign::TopLeft,
            line_height: LineHeight::Px(24.0),
            text_indent: 8.0,
            cache: LayoutCache::new(),
            _typeface: Cell::new(None),
        };

        let first_ptr = widget.with_paragraph(std::ptr::from_ref);
        let second_ptr = widget.with_paragraph(std::ptr::from_ref);

        assert_eq!(first_ptr, second_ptr);
        widget.invalidate_layout();
        assert_eq!(second_ptr, widget.with_paragraph(std::ptr::from_ref));
        assert!(widget.uses_paragraph_layout());
    }

    /// Anything that rebuilds the tree advances the layout generation, which
    /// retires the paragraph's cached layout. Painting has to lay the text out
    /// again rather than decline: a declining element paints nothing, so an
    /// underlined heading used to vanish on every step of a window resize and
    /// come back when the drag stopped.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_underlined_heading_still_paints_after_the_layout_generation_advances() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_style::{TextDecoration, TextDecorationLine};
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
        let widget = RawTextWidget {
            text: TextSource::Static("Consistence Looking"),
            text_style: TextStyle::default()
                .text_decoration(TextDecoration::new().line(TextDecorationLine::UNDERLINE)),
            text_align: TextAlign::TopLeft,
            line_height: LineHeight::Normal,
            text_indent: 0.0,
            cache: LayoutCache::new(),
            _typeface: Cell::new(None),
        };

        let _ = widget.computed_size(&context);
        assert!(
            widget.can_paint_local_v2(&context),
            "laid out, so it can paint"
        );

        aimer_widget::notify_element_tree_changed();
        assert!(
            widget.can_paint_local_v2(&context),
            "a text whose cached layout was retired must lay out again, not decline"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn decorated_text_reuses_its_prepared_layout_and_lines() {
        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_style::{TextDecoration, TextDecorationLine};
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
        let widget = RawTextWidget {
            text: TextSource::Static("first\nsecond"),
            text_style: TextStyle::default()
                .text_decoration(TextDecoration::new().line(TextDecorationLine::UNDERLINE)),
            text_align: TextAlign::TopLeft,
            line_height: LineHeight::Normal,
            text_indent: 0.0,
            cache: LayoutCache::new(),
            _typeface: Cell::new(None),
        };

        let first = widget
            .local_v2_paragraph_paint_data(&context)
            .unwrap()
            .layout;
        #[cfg(debug_assertions)]
        let first_stats = inner.text_cache_stats();

        let second = widget
            .local_v2_paragraph_paint_data(&context)
            .unwrap()
            .layout;
        #[cfg(debug_assertions)]
        let second_stats = inner.text_cache_stats();

        assert_eq!(first.line_heights.len(), 2);
        assert_eq!(first.decorations.len(), 2);
        assert!(std::rc::Rc::ptr_eq(&first, &second));
        #[cfg(debug_assertions)]
        {
            assert_eq!(
                second_stats, first_stats,
                "reusing prepared geometry measures nothing"
            );
        }
    }

    #[test]
    fn top_center_does_not_place_oversized_text_before_the_left_edge() {
        assert_eq!(
            horizontal_alignment_offset(TextAlign::TopCenter, 200.0, 260.0),
            0.0
        );
    }

    #[test]
    fn mid_center_centers_the_entire_multiline_block() {
        assert_eq!(
            vertical_alignment_baseline(TextAlign::MidCenter, 200.0, 60.0, 16.0),
            86.0
        );
    }

    #[test]
    fn bottom_alignment_places_the_entire_multiline_block_at_the_bottom() {
        assert_eq!(
            vertical_alignment_baseline(TextAlign::BotCenter, 200.0, 60.0, 16.0),
            156.0
        );
    }
}
