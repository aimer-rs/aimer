//! Keys and job inputs for the shaping and layout caches.
//!
//! Both caches are consulted several times per span on every frame, so a key
//! must be cheap to build: the text rides in it as the `Arc<str>` the draw
//! request already carries — a refcount bump, not a string copy — and every
//! `f32` parameter is stored as its rounded integer representation so the key
//! can implement `Hash + Eq`.

use std::sync::Arc;

use hashbrown::HashMap;

use crate::font::{FontFamily, FontStyle, TextLanguage};
use crate::text_pipeline::text_layout::{ShapedText, TextWritingMode};

/// Key used to memoize the output of `layout_text` across frames.
#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub(super) struct LayoutCacheKey {
    text: Arc<str>,
    /// `font_size` × 100, rounded, stored as u32 to make it hashable.
    font_size_u32: u32,
    /// `bounds_width` × 100, rounded, stored as u32.
    bounds_width_u32: u32,
    /// `bounds_height` × 100, rounded, stored as u32. It is the wrapping
    /// extent for vertical writing and remains zero for legacy horizontal keys.
    bounds_height_u32: u32,
    font_family: FontFamily,
    font_style: FontStyle,
    font_weight: u16,
    /// The language the run is written in, which selects the face its
    /// ideographs are drawn from and therefore their advances.
    language: Option<TextLanguage>,
    writing_mode: TextWritingMode,
}

impl LayoutCacheKey {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        text: impl Into<Arc<str>>,
        font_size: f32,
        bounds_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
        language: Option<TextLanguage>,
    ) -> Self {
        Self::new_with_writing_mode(
            text,
            font_size,
            bounds_width,
            0.0,
            font_family,
            font_style,
            font_weight,
            language,
            TextWritingMode::HorizontalTb,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_with_writing_mode(
        text: impl Into<Arc<str>>,
        font_size: f32,
        bounds_width: f32,
        bounds_height: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
        language: Option<TextLanguage>,
        writing_mode: TextWritingMode,
    ) -> Self {
        Self {
            text: text.into(),
            font_size_u32: (font_size * 100.0).round() as u32,
            bounds_width_u32: (bounds_width * 100.0).round() as u32,
            bounds_height_u32: (bounds_height * 100.0).round() as u32,
            font_family,
            font_style,
            font_weight,
            language,
            writing_mode,
        }
    }
}

/// Width-independent sibling of [`LayoutCacheKey`]: shaped glyph ids and
/// advances depend on the text and the font parameters, never on the
/// wrapping width.
#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub(super) struct ShapingCacheKey {
    text: Arc<str>,
    /// `font_size` × 100, rounded, stored as u32 to make it hashable.
    font_size_u32: u32,
    font_family: FontFamily,
    font_style: FontStyle,
    font_weight: u16,
    /// Part of the identity for the same reason it is part of
    /// [`LayoutCacheKey`]: `你好` shaped for a Chinese field may not be handed
    /// to a Japanese one, whose face gives it different glyphs.
    language: Option<TextLanguage>,
    writing_mode: TextWritingMode,
}

impl ShapingCacheKey {
    pub(super) fn new(
        text: impl Into<Arc<str>>,
        font_size: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
        language: Option<TextLanguage>,
    ) -> Self {
        Self::new_with_writing_mode(
            text,
            font_size,
            font_family,
            font_style,
            font_weight,
            language,
            TextWritingMode::HorizontalTb,
        )
    }

    pub(super) fn new_with_writing_mode(
        text: impl Into<Arc<str>>,
        font_size: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
        language: Option<TextLanguage>,
        writing_mode: TextWritingMode,
    ) -> Self {
        Self {
            text: text.into(),
            font_size_u32: (font_size * 100.0).round() as u32,
            font_family,
            font_style,
            font_weight,
            language,
            writing_mode,
        }
    }
}

/// Slack subtracted from a wrapping width before a layout is declared
/// width-independent.
///
/// [`ShapedText::max_line_width`] and the positioning pen accumulate the same
/// advances in slightly different grouping, so the two sums can disagree by a
/// few float ulps. Half a pixel dwarfs that error by orders of magnitude
/// while flipping essentially no real layout the other way: text within half
/// a pixel of its column's edge simply keeps its width-keyed cache entry.
const WIDTH_INDEPENDENCE_SLACK: f32 = 0.5;

/// The cache identity of one span's layout at one requested wrapping width.
///
/// A wrapped layout only depends on the width when the text wraps at it:
/// a span whose widest line fits inside the requested width positions
/// identically at every width it fits in, so its layout is cached under the
/// canonical unbounded key (`width == 0`) and survives a window resize. The
/// canonical form is only knowable once the span's shaping is cached; keys
/// built before that carry the requested width, so a read consults
/// [`primary`](Self::primary) first and falls back to
/// [`fallback`](Self::fallback) — the width-keyed entry an earlier frame may
/// have minted before the shaping existed.
#[derive(Clone)]
pub(super) struct SpanLayoutKeys {
    /// The width-independent sibling key; also what a preparation batch
    /// shapes under.
    pub(super) shaping_key: ShapingCacheKey,
    /// The key this frame prepares and draws under.
    pub(super) primary: LayoutCacheKey,
    /// The width-keyed entry an earlier frame may hold for the same glyphs,
    /// present only when `primary` was canonicalized away from the requested
    /// width.
    pub(super) fallback: Option<LayoutCacheKey>,
    /// The wrapping width `primary` stands for — `0.0` when canonicalized.
    pub(super) layout_width: f32,
    /// The wrapping height `primary` stands for — `0.0` when canonicalized.
    pub(super) layout_height: f32,
}

/// Builds cache keys for a horizontal or vertical span. In vertical mode the
/// requested height is the wrapping extent; width remains part of the layout
/// identity because it determines the right-to-left column origin.
#[allow(clippy::too_many_arguments)]
pub(super) fn span_layout_keys_with_writing_mode(
    shaping_cache: &HashMap<ShapingCacheKey, Arc<ShapedText>>,
    text: &Arc<str>,
    font_size: f32,
    font_family: FontFamily,
    font_style: FontStyle,
    font_weight: u16,
    language: Option<TextLanguage>,
    requested_width: f32,
    requested_height: f32,
    writing_mode: TextWritingMode,
) -> SpanLayoutKeys {
    let shaping_key = ShapingCacheKey::new_with_writing_mode(
        text.clone(),
        font_size,
        font_family,
        font_style,
        font_weight,
        language,
        writing_mode,
    );

    let requested_extent = if writing_mode.is_vertical() {
        requested_height
    } else {
        requested_width
    };
    // A vertical-rl layout is anchored to the right edge of the requested
    // column area. Even when the text fits in one column, changing
    // `requested_width` changes every glyph's x coordinate, so it cannot use
    // the horizontal width-independent canonical entry.
    let fits_unwrapped = !writing_mode.is_vertical()
        && requested_extent > 0.0
        && shaping_cache.get(&shaping_key).is_some_and(|shaped| {
            shaped.max_line_width + WIDTH_INDEPENDENCE_SLACK <= requested_extent
        });
    let (layout_width, layout_height, fallback) = if fits_unwrapped {
        let width_keyed = LayoutCacheKey::new_with_writing_mode(
            text.clone(),
            font_size,
            requested_width,
            requested_height,
            font_family,
            font_style,
            font_weight,
            language,
            writing_mode,
        );
        (0.0, 0.0, Some(width_keyed))
    } else {
        (requested_width, requested_height, None)
    };

    SpanLayoutKeys {
        primary: LayoutCacheKey::new_with_writing_mode(
            text.clone(),
            font_size,
            layout_width,
            layout_height,
            font_family,
            font_style,
            font_weight,
            language,
            writing_mode,
        ),
        shaping_key,
        fallback,
        layout_width,
        layout_height,
    }
}

/// Everything a worker needs to shape one span.
#[derive(Clone)]
pub(super) struct ShapingInput {
    pub(super) text: Arc<str>,
    pub(super) font_size: f32,
    pub(super) font_family: FontFamily,
    pub(super) font_style: FontStyle,
    pub(super) font_weight: u16,
    /// The language the span is written in, carried to the worker so it
    /// chooses the same faces the drawing pass will.
    pub(super) language: Option<TextLanguage>,
    pub(super) writing_mode: TextWritingMode,
}

/// Everything a worker needs to position one shaped span.
#[derive(Clone)]
pub(super) struct LayoutInput {
    pub(super) shaping_key: ShapingCacheKey,
    pub(super) layout_width: f32,
    pub(super) layout_height: f32,
}

/// Owned layout input used by the persistent preparation workers.
///
/// The shaped result is reference-counted so a layout job can outlive the
/// caller's cache borrow without copying the shaped clusters. Multiple widths
/// for the same shaped span therefore share one allocation.
#[derive(Clone)]
pub(super) struct OwnedLayoutInput {
    pub(super) shaped: Arc<ShapedText>,
    pub(super) layout_width: f32,
    pub(super) layout_height: f32,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hashbrown::HashMap;

    use super::{
        LayoutCacheKey, OwnedLayoutInput, ShapingCacheKey, span_layout_keys_with_writing_mode,
    };
    use crate::font::{FontFamily, FontStyle, FontWeight};
    use crate::text_pipeline::text_layout::{ShapedText, TextWritingMode};

    #[test]
    fn writing_mode_is_part_of_shaping_and_layout_identity() {
        let horizontal = ShapingCacheKey::new(
            "same",
            16.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
        );
        let vertical = ShapingCacheKey::new_with_writing_mode(
            "same",
            16.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
            TextWritingMode::VerticalRl,
        );
        assert_ne!(horizontal, vertical);

        let horizontal_layout = LayoutCacheKey::new(
            "same",
            16.0,
            200.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
        );
        let vertical_layout = LayoutCacheKey::new_with_writing_mode(
            "same",
            16.0,
            200.0,
            48.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
            TextWritingMode::VerticalRl,
        );
        assert_ne!(horizontal_layout, vertical_layout);
    }

    #[test]
    fn text_cache_keys_isolate_font_families_and_variants() {
        let sans_layout = LayoutCacheKey::new(
            "same",
            16.0,
            100.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
        );
        let mono_layout = LayoutCacheKey::new(
            "same",
            16.0,
            100.0,
            FontFamily::MONOSPACE,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
        );
        assert_ne!(sans_layout, mono_layout);

        let normal_shape = ShapingCacheKey::new(
            "same",
            16.0,
            FontFamily::MONOSPACE,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
        );
        let italic_shape = ShapingCacheKey::new(
            "same",
            16.0,
            FontFamily::MONOSPACE,
            FontStyle::Italic,
            FontWeight::Normal.numeric(),
            None,
        );
        assert_ne!(normal_shape, italic_shape);
    }

    #[test]
    fn span_layout_keys_accepts_shared_shaping_results() {
        let shaping_cache: HashMap<ShapingCacheKey, Arc<ShapedText>> = HashMap::new();
        let text = Arc::<str>::from("shared");

        let keys = span_layout_keys_with_writing_mode(
            &shaping_cache,
            &text,
            16.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
            240.0,
            0.0,
            TextWritingMode::HorizontalTb,
        );

        assert_eq!(keys.layout_width, 240.0);
    }

    #[test]
    fn vertical_layout_keys_keep_both_requested_bounds() {
        let text = Arc::<str>::from("vertical");
        let shaping_key = ShapingCacheKey::new_with_writing_mode(
            text.clone(),
            16.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
            TextWritingMode::VerticalRl,
        );
        let shaped = Arc::new(ShapedText {
            text: text.to_string(),
            font_size: 16.0,
            ascent: 12.0,
            descent: -4.0,
            line_gap: 0.0,
            line_height: 16.0,
            max_line_width: 8.0,
            writing_mode: TextWritingMode::VerticalRl,
            clusters: Vec::new(),
        });
        let mut shaping_cache = HashMap::new();
        shaping_cache.insert(shaping_key, shaped);

        let keys = span_layout_keys_with_writing_mode(
            &shaping_cache,
            &text,
            16.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
            240.0,
            48.0,
            TextWritingMode::VerticalRl,
        );

        assert_eq!(keys.layout_width, 240.0);
        assert_eq!(keys.layout_height, 48.0);
        assert!(keys.fallback.is_none());
    }

    #[test]
    fn owned_layout_input_clone_shares_the_shaped_allocation() {
        let shaped = Arc::new(ShapedText {
            text: "shared".into(),
            font_size: 16.0,
            ascent: 12.0,
            descent: -4.0,
            line_gap: 0.0,
            line_height: 16.0,
            max_line_width: 42.0,
            writing_mode: TextWritingMode::HorizontalTb,
            clusters: Vec::new(),
        });
        let input = OwnedLayoutInput {
            shaped: shaped.clone(),
            layout_width: 240.0,
            layout_height: 0.0,
        };
        let clone = input.clone();

        assert!(Arc::ptr_eq(&input.shaped, &clone.shaped));
    }
}
