// The font engine (parsing, shaping, fallback, rasterization, layout) lives in
// `aimer_cupid_font`. The modules are re-exported here so the paths the
// preparation code and downstream crates use keep resolving.
#[cfg(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
))]
pub(crate) use aimer_cupid_font::{apple_fonts, core_text_raster};
pub(crate) use aimer_cupid_font::{
    aimer_font, font_resolver, glyph_metrics, system_fallback, unicode_script,
};
pub use aimer_cupid_font::{glyph_rasterizer, text_layout};

mod cache_key;
mod deferred_preparation;
pub mod glyph_atlas;
mod layout_cache;
mod paint_cache;
mod preparation_batch;
pub use self::text_layout::{TextHorizontalAlign, TextWritingMode};
#[cfg(test)]
mod phase6_verification;

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hashbrown::{HashMap, HashSet};

use aimer_utils::AnimInstant;
use bytemuck::{Pod, Zeroable};

use crate::font::{FontFamily, FontRegistry, FontStyle, FontWeight, TextLanguage};
use crate::pipeline::frame_upload::FrameUpload;
use crate::pipeline::image_pipeline::InstanceBufferPolicy;
use crate::text_pipeline::cache_key::{
    LayoutCacheKey, LayoutInput, OwnedLayoutInput, ShapingCacheKey, ShapingInput, SpanLayoutKeys,
    span_layout_keys_with_writing_mode,
};
use crate::text_pipeline::deferred_preparation::{
    PREPARATION_BUDGET, PREPARATION_CHUNK, PreparationBudget, prepare_ahead_of_view,
    request_is_on_screen,
};
use crate::text_pipeline::font_resolver::warm_fallbacks_in_background;
use crate::text_pipeline::glyph_atlas::{BatchCapacityPlan, ColorGlyphAtlas, GlyphAtlas};
use crate::text_pipeline::glyph_rasterizer::{
    GlyphKey, GlyphPreparationContext, GlyphRasterizer, glyph_runs,
};
use crate::text_pipeline::layout_cache::LayoutCache;
use crate::text_pipeline::paint_cache::{
    CachedPaintGlyph, CachedPaintKind, CachedTextPaint, TextPaintCache, TextPaintCacheKey,
    sanitize_shadow,
};
use crate::text_pipeline::preparation_batch::{BatchExecutor, IndexedJob, PreparationBatch};
use crate::text_pipeline::text_layout::{
    ShapedText, layout_shaped_text_result_with_bounds, line_alignment_offsets,
    positioned_line_widths, prepare_shaped_text_with_writing_mode,
};
use crate::utilities::Rgba8;

/// Per-instance data for one glyph quad.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GlyphInstance {
    position: [f32; 2],
    size: [f32; 2],
    uv_rect: [f32; 4],
    color: Rgba8,
    /// Clip rect: [x, y, width, height]. If width <= 0, no clip is applied.
    clip_rect: [f32; 4],
    /// Border radius for the clip rect: [top-left, top-right, bottom-right,
    /// bottom-left].
    clip_border_radius: [f32; 4],
    /// Horizontal shear factor for synthetic italic (tan of the slant angle).
    /// 0 = upright. The glyph shaders slant the quad by this, pinned at its
    /// bottom edge, so the advance/layout is unchanged.
    skew: f32,
    /// Exponent the glyph shader raises the atlas coverage to before blending.
    /// See [`coverage_exponent`]; `1.0` leaves the coverage untouched.
    coverage_exponent: f32,
    /// Padding to keep the struct 8-byte aligned for `Pod`/vertex upload.
    _pad: [f32; 2],
}

/// Gamma of the blend space text is composited in.
///
/// sRGB's transfer function is a 2.4 power with a linear toe; 2.2 is its
/// standard single-exponent approximation and the value every text stack that
/// corrects for this uses.
const TEXT_BLEND_GAMMA: f32 = 2.2;

/// Returns the exponent a glyph's coverage must be raised to before it is
/// blended in linear light.
///
/// A rasterizer's coverage is a *geometric* quantity: half a pixel covered by
/// black means "paint half way to black", which is a statement about the
/// picture, not about photons. Blending it on an sRGB target performs the mix
/// in linear light instead, and half way in linear light is far lighter than
/// half way in sRGB — so an antialiased edge loses weight, and a stroke looks
/// thinner the more of it falls on partially covered pixels.
///
/// Raising the coverage to `gamma^(2*luminance - 1)` puts the weight back:
/// dark text (luminance `0`) gets `1/2.2`, which strengthens partial coverage;
/// light text on a dark background gets `2.2`, which weakens it by the same
/// amount in the other direction; mid-luminance text — the one case linear
/// blending already renders correctly — gets exactly `1.0` and is untouched.
///
/// The background is not known here, so the text's own luminance stands in for
/// "which way the blend runs", which is the assumption every fixed-curve text
/// gamma uses and is right whenever text contrasts with what it sits on.
#[inline]
fn coverage_exponent(color: [f32; 4]) -> f32 {
    let channel = |value: f32| {
        if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let luminance =
        0.2126 * channel(color[0]) + 0.7152 * channel(color[1]) + 0.0722 * channel(color[2]);

    TEXT_BLEND_GAMMA.powf(2.0 * luminance - 1.0)
}

/// Returns the on-screen size of a glyph quad drawn from an atlas region of
/// `region_size` texels.
///
/// A glyph is a bitmap, not a shape. It reaches the screen unaltered only when
/// its quad is exactly as large as the region behind it, so that one quad pixel
/// maps to one texel and the linear sampler lands on texel centres. Any other
/// size resamples the whole glyph — every stroke is blended across two texels,
/// which reads as a loss of stroke weight rather than as a change of size,
/// because the ink the stroke carries is spread over more pixels than the
/// rasterizer put it on.
///
/// The region is therefore the only admissible source for the size. The
/// layout's own measurement of the glyph is not: it comes from a different
/// pass, and the two can disagree by a pixel.
#[inline]
fn glyph_quad_size(region_size: (u32, u32)) -> [f32; 2] {
    [region_size.0 as f32, region_size.1 as f32]
}

/// Snaps a glyph quad's top-left corner onto the device pixel grid.
///
/// Glyphs are rasterized at a single phase — [`GlyphKey`]'s `subpixel_x` and
/// `subpixel_y` are always `0` — so the bitmap in the atlas is drawn as if its
/// origin were a whole pixel. The quad's size already equals the bitmap's, so
/// placing that quad on a whole pixel makes every texel land on exactly one
/// pixel; placing it anywhere else makes the linear atlas sampler blend each
/// texel across two, which costs contrast and stroke weight.
///
/// Layout positions are fractional (advances, glyph bearings and a centred
/// line's alignment offset all are), so without this a piece of text renders
/// differently depending on where it happens to sit — most visibly for a label
/// centred in a box too narrow for it, which lands on a half pixel.
#[inline]
fn snap_to_pixel_grid(position: [f32; 2]) -> [f32; 2] {
    [position[0].round(), position[1].round()]
}

fn glyph_intersects_clip(position: [f32; 2], size: [f32; 2], clip: [f32; 4]) -> bool {
    if clip[2] <= 0.0 {
        return true;
    }

    let glyph_right = position[0] + size[0];
    let glyph_bottom = position[1] + size[1];
    let clip_right = clip[0] + clip[2];
    let clip_bottom = clip[1] + clip[3];

    glyph_right > clip[0]
        && position[0] < clip_right
        && glyph_bottom > clip[1]
        && position[1] < clip_bottom
}

#[inline]
fn shadow_padding(shadow: TextShadowRequest) -> f32 {
    let offset_x = shadow
        .offset_x
        .is_finite()
        .then_some(shadow.offset_x.abs())
        .unwrap_or(0.0);
    let offset_y = shadow
        .offset_y
        .is_finite()
        .then_some(shadow.offset_y.abs())
        .unwrap_or(0.0);
    let blur = shadow
        .blur
        .is_finite()
        .then_some(shadow.blur.max(0.0))
        .unwrap_or(0.0);
    offset_x.max(offset_y) + blur
}

#[inline]
fn shadow_intersects_clip(
    position: [f32; 2],
    size: [f32; 2],
    shadow: TextShadowRequest,
    clip: [f32; 4],
) -> bool {
    let offset_x = shadow
        .offset_x
        .is_finite()
        .then_some(shadow.offset_x)
        .unwrap_or(0.0);
    let offset_y = shadow
        .offset_y
        .is_finite()
        .then_some(shadow.offset_y)
        .unwrap_or(0.0);
    let blur = shadow
        .blur
        .is_finite()
        .then_some(shadow.blur.max(0.0))
        .unwrap_or(0.0);
    glyph_intersects_clip(
        [position[0] + offset_x - blur, position[1] + offset_y - blur],
        [size[0] + 2.0 * blur, size[1] + 2.0 * blur],
        clip,
    )
}

#[inline]
#[cfg(test)]
fn shadow_is_visible(color: Rgba8) -> bool {
    color.as_array()[3] > 0
}

#[inline]
fn normalize_pixel_uv_rect(pixel_rect: [f32; 4], atlas_width: u32, atlas_height: u32) -> [f32; 4] {
    let width = atlas_width as f32;
    let height = atlas_height as f32;
    [
        pixel_rect[0] / width,
        pixel_rect[1] / height,
        pixel_rect[2] / width,
        pixel_rect[3] / height,
    ]
}

impl GlyphInstance {
    #[cfg(feature = "wgpu")]
    const ATTRIBS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
        0 => Float32x2,
        1 => Float32x2,
        2 => Float32x4,
        3 => Unorm8x4,
        4 => Float32x4,
        5 => Float32x4,
        6 => Float32,
        7 => Float32,
    ];

    #[cfg(feature = "wgpu")]
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<GlyphInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }

    const GENERIC_ATTRIBUTES: [crate::backend::VertexAttribute; 8] = [
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, position) as u64,
            shader_location: 0,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, size) as u64,
            shader_location: 1,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, uv_rect) as u64,
            shader_location: 2,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Unorm8x4,
            offset: std::mem::offset_of!(Self, color) as u64,
            shader_location: 3,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, clip_rect) as u64,
            shader_location: 4,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, clip_border_radius) as u64,
            shader_location: 5,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32,
            offset: std::mem::offset_of!(Self, skew) as u64,
            shader_location: 6,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32,
            offset: std::mem::offset_of!(Self, coverage_exponent) as u64,
            shader_location: 7,
        },
    ];
}

/// Per-instance data for one decoration line quad (underline/overline/strike).
/// The line geometry is a plain quad; the actual stroke (and its dotted/dashed/
/// wavy shape) is produced procedurally by `text_decoration.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct DecorationInstance {
    /// Top-left of the band quad, screen space.
    position: [f32; 2],
    /// Band size: [width, band_height].
    size: [f32; 2],
    color: Rgba8,
    clip_rect: [f32; 4],
    clip_border_radius: [f32; 4],
    /// [style_id, thickness_px, period_px, band_height_px].
    params: [f32; 4],
}

impl DecorationInstance {
    #[cfg(feature = "wgpu")]
    const ATTRIBS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x2,
        1 => Float32x2,
        2 => Unorm8x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Float32x4,
    ];

    #[cfg(feature = "wgpu")]
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<DecorationInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }

    const GENERIC_ATTRIBUTES: [crate::backend::VertexAttribute; 6] = [
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, position) as u64,
            shader_location: 0,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, size) as u64,
            shader_location: 1,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Unorm8x4,
            offset: std::mem::offset_of!(Self, color) as u64,
            shader_location: 2,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, clip_rect) as u64,
            shader_location: 3,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, clip_border_radius) as u64,
            shader_location: 4,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, params) as u64,
            shader_location: 5,
        },
    ];
}

/// A single styled decoration line to render, in final screen-space geometry.
/// The producer (widget/renderer) computes where the line sits from the text
/// metrics; the engine only rasterizes the styled stroke inside the band.
#[derive(Clone, Copy, Debug)]
pub struct TextDecorationDraw {
    /// Top-left of the band quad.
    pub x: f32,
    pub y: f32,
    /// Band width (line length).
    pub width: f32,
    /// Band height — tall enough to hold the stroke plus wave/double spacing.
    pub band_height: f32,
    /// Stroke thickness in pixels.
    pub thickness: f32,
    /// Repeat period for dotted/dashed/wavy styles (pixels).
    pub period: f32,
    /// Style id, matching `aimer_style::TextDecorationStyle::id`.
    pub style: u32,
    pub color: Rgba8,
    pub clip_rect: [f32; 4],
    pub clip_border_radius: [f32; 4],
}

impl TextDecorationDraw {
    fn to_instance(self) -> DecorationInstance {
        DecorationInstance {
            position: [self.x, self.y],
            size: [self.width, self.band_height],
            color: self.color,
            clip_rect: self.clip_rect,
            clip_border_radius: self.clip_border_radius,
            params: [
                self.style as f32,
                self.thickness,
                self.period,
                self.band_height,
            ],
        }
    }
}

/// Paint data for one glyph shadow. The text pipeline expands blurred shadows
/// into a small, bounded sample set so the same atlas, clip, opacity, and
/// transform path is used as the foreground glyphs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadowRequest {
    /// Horizontal offset in physical pixels.
    pub offset_x: f32,
    /// Vertical offset in physical pixels.
    pub offset_y: f32,
    /// Blur radius in physical pixels.
    pub blur: f32,
    /// RGBA paint color.
    pub color: Rgba8,
}

#[derive(Clone)]
pub struct TextDrawRequest {
    pub x: f32,
    pub y: f32,
    // Reference-counted so cloning the request per frame (and from the draw
    // list) is a cheap refcount bump rather than a fresh string allocation.
    pub text: Arc<str>,
    pub font_size: f32,
    pub color: Rgba8,
    pub bounds_width: f32,
    pub bounds_height: f32,
    pub overflow: TextOverflowMode,
    pub horizontal_align: TextHorizontalAlign,
    /// Writing mode for shaping and column/line placement.
    pub writing_mode: TextWritingMode,
    pub line_height: Option<f32>,
    /// Optional glyph shadow painted before the foreground run.
    pub shadow: Option<TextShadowRequest>,
    /// Whether this request also paints its foreground glyphs. Shadow-only
    /// requests are used by the canvas convenience API.
    pub draw_glyphs: bool,
    pub font_family: FontFamily,
    pub font_style: FontStyle,
    pub font_weight: Option<u16>,
    /// The language this text is written in, when the producer knows it.
    ///
    /// Han is unified, so a run of ideographs does not say whether it wants a
    /// Chinese or a Japanese face and stays on whichever the platform's
    /// cascade prefers — until a character only one language writes is typed
    /// and the whole word changes typeface. A text field knows the keyboard it
    /// is edited with and says so here; `None` leaves the run judged on its
    /// own characters.
    pub language: Option<TextLanguage>,
    pub italic: bool,
    pub clip_rect: [f32; 4],
    pub clip_border_radius: [f32; 4],
    pub spans: Vec<RichTextSpan>,
}

#[derive(Clone, Debug)]
pub struct RichTextSpan {
    pub text: Arc<str>,
    pub font_size: Option<f32>,
    pub color: Option<Rgba8>,
    pub font_weight: Option<u16>,
    pub italic: Option<bool>,
}

impl RichTextSpan {
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            font_size: None,
            color: None,
            font_weight: None,
            italic: None,
        }
    }

    pub fn with_style(mut self, font_size: Option<f32>, color: Option<Rgba8>) -> Self {
        self.font_size = font_size;
        self.color = color;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Hash, Eq, PartialEq)]
pub enum TextOverflowMode {
    #[default]
    Clip,
    Wrap,
    Ellipsis,
}

/// Glyph-instance ranges owned by a single text request. `[alpha_start,
/// alpha_end)` indexes `instances` and `[color_start, color_end)` indexes
/// `color_instances`. `prepare` fills both lists in request order, so each
/// request owns a contiguous slice of each and can be drawn on its own at the
/// right z-position in the draw stream.
#[derive(Clone, Copy, Default)]
struct TextRequestRange {
    alpha_start: u32,
    alpha_end: u32,
    color_start: u32,
    color_end: u32,
}

/// Exact inputs that produced the retained instance lists.
///
/// A fingerprint alone would make a hash collision observable as stale text,
/// so the warm-frame gate keeps the small request snapshot and compares every
/// render-affecting field before skipping preparation. The snapshot owns only
/// `Arc` text handles and is replaced when a frame actually changes.
struct PreparedFrameCache {
    width: u32,
    height: u32,
    is_srgb: bool,
    atlas_generation: u64,
    color_atlas_generation: u64,
    font_revision: u64,
    requests: Vec<TextDrawRequest>,
    decorations: Vec<TextDecorationDraw>,
}

impl PreparedFrameCache {
    fn new(
        width: u32,
        height: u32,
        is_srgb: bool,
        atlas_generation: u64,
        color_atlas_generation: u64,
        font_revision: u64,
        requests: &[TextDrawRequest],
        decorations: &[TextDecorationDraw],
    ) -> Self {
        Self {
            width,
            height,
            is_srgb,
            atlas_generation,
            color_atlas_generation,
            font_revision,
            requests: requests.to_vec(),
            decorations: decorations.to_vec(),
        }
    }

    fn matches(
        &self,
        width: u32,
        height: u32,
        is_srgb: bool,
        atlas_generation: u64,
        color_atlas_generation: u64,
        font_revision: u64,
        requests: &[TextDrawRequest],
        decorations: &[TextDecorationDraw],
    ) -> bool {
        self.width == width
            && self.height == height
            && self.is_srgb == is_srgb
            && self.atlas_generation == atlas_generation
            && self.color_atlas_generation == color_atlas_generation
            && self.font_revision == font_revision
            && self.requests.len() == requests.len()
            && self
                .requests
                .iter()
                .zip(requests)
                .all(|(cached, current)| same_text_request(cached, current))
            && self.decorations.len() == decorations.len()
            && self
                .decorations
                .iter()
                .zip(decorations)
                .all(|(cached, current)| same_decoration(cached, current))
    }
}

#[inline]
fn same_f32(left: f32, right: f32) -> bool {
    left.to_bits() == right.to_bits()
}

#[inline]
fn same_f32_array<const N: usize>(left: &[f32; N], right: &[f32; N]) -> bool {
    left.iter()
        .zip(right)
        .all(|(&left, &right)| same_f32(left, right))
}

#[inline]
fn same_option_f32(left: Option<f32>, right: Option<f32>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same_f32(left, right),
        (None, None) => true,
        _ => false,
    }
}

#[inline]
fn same_text(left: &Arc<str>, right: &Arc<str>) -> bool {
    Arc::ptr_eq(left, right) || left.as_ref() == right.as_ref()
}

#[inline]
fn same_shadow(left: Option<TextShadowRequest>, right: Option<TextShadowRequest>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            same_f32(left.offset_x, right.offset_x)
                && same_f32(left.offset_y, right.offset_y)
                && same_f32(left.blur, right.blur)
                && left.color == right.color
        }
        (None, None) => true,
        _ => false,
    }
}

#[inline]
fn same_span(left: &RichTextSpan, right: &RichTextSpan) -> bool {
    same_text(&left.text, &right.text)
        && same_option_f32(left.font_size, right.font_size)
        && left.color == right.color
        && left.font_weight == right.font_weight
        && left.italic == right.italic
}

#[inline]
fn same_text_request(left: &TextDrawRequest, right: &TextDrawRequest) -> bool {
    same_f32(left.x, right.x)
        && same_f32(left.y, right.y)
        && same_text(&left.text, &right.text)
        && same_f32(left.font_size, right.font_size)
        && left.color == right.color
        && same_f32(left.bounds_width, right.bounds_width)
        && same_f32(left.bounds_height, right.bounds_height)
        && left.overflow == right.overflow
        && left.horizontal_align == right.horizontal_align
        && left.writing_mode == right.writing_mode
        && same_option_f32(left.line_height, right.line_height)
        && same_shadow(left.shadow, right.shadow)
        && left.draw_glyphs == right.draw_glyphs
        && left.font_family == right.font_family
        && left.font_style == right.font_style
        && left.font_weight == right.font_weight
        && left.language == right.language
        && left.italic == right.italic
        && same_f32_array(&left.clip_rect, &right.clip_rect)
        && same_f32_array(&left.clip_border_radius, &right.clip_border_radius)
        && left.spans.len() == right.spans.len()
        && left
            .spans
            .iter()
            .zip(&right.spans)
            .all(|(left, right)| same_span(left, right))
}

#[inline]
fn same_decoration(left: &TextDecorationDraw, right: &TextDecorationDraw) -> bool {
    same_f32(left.x, right.x)
        && same_f32(left.y, right.y)
        && same_f32(left.width, right.width)
        && same_f32(left.band_height, right.band_height)
        && same_f32(left.thickness, right.thickness)
        && same_f32(left.period, right.period)
        && left.style == right.style
        && left.color == right.color
        && same_f32_array(&left.clip_rect, &right.clip_rect)
        && same_f32_array(&left.clip_border_radius, &right.clip_border_radius)
}

/// CPU wall-clock breakdown for one [`TextPipelineV2::prepare`] call.
///
/// This is a diagnostic snapshot for profiling repeated-cold preparation. It
/// is collected only by [`TextPipelineV2::prepare_profiled`]; the normal
/// [`TextPipelineV2::prepare`] path does not start timers or allocate a
/// profile. Stage values are inclusive of the work named by the field, and
/// can overlap with `instance_build` when atlas population happens while
/// instances are being assembled.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextPreparationProfile {
    /// Total CPU time spent in the profiled prepare call.
    pub total: Duration,
    /// Whether the call reused the complete retained frame without rebuilding
    /// layout, atlas plans, instances, or uploads.
    pub cache_hit: bool,
    /// Number of span paint templates reused during this call.
    pub paint_cache_hits: usize,
    /// Number of span paint templates built during this call.
    pub paint_cache_misses: usize,
    /// Request culling and off-screen layout-miss analysis.
    pub request_analysis: Duration,
    /// All `SpanLayoutKeys` construction done by the call.
    pub key_construction: Duration,
    /// Owner-side fallback resolution, selected-face snapshot, and Aimer
    /// prewarming.
    pub fallback_resolution: Duration,
    /// Immutable worker snapshot construction and face-state transfer.
    pub font_snapshot: Duration,
    /// Worker-side shaping and shaping-result merge.
    pub shaping: Duration,
    /// Combined fallback-resolution and shaping time.
    pub fallback_and_shaping: Duration,
    /// Layout-job construction, execution, and result merge.
    pub layout: Duration,
    /// Glyph-job construction, execution, result merge, and commit.
    pub glyph_preparation: Duration,
    /// Atlas capacity planning and plan application.
    pub atlas_planning: Duration,
    /// Rasterizer-to-atlas insertion work during instance assembly.
    pub atlas_population: Duration,
    /// Instance/decorations assembly, including atlas probes and population.
    pub instance_build: Duration,
    /// Atlas texture writes and any atlas upload staging.
    pub atlas_upload: Duration,
    /// Instance-buffer sizing and GPU buffer writes.
    pub instance_upload: Duration,
    /// Number of shaping jobs submitted by this call.
    pub shaping_jobs: usize,
    /// Number of layout jobs submitted by this call.
    pub layout_jobs: usize,
    /// Number of glyph runs submitted by this call.
    pub glyph_jobs: usize,
    /// Number of alpha-atlas descriptors planned by this call.
    pub alpha_glyphs: usize,
    /// Number of color-atlas descriptors planned by this call.
    pub color_glyphs: usize,
}

pub struct TextPipelineV2<B: crate::backend::GpuBackend = crate::backend::DefaultGpuBackend> {
    rasterizer: GlyphRasterizer,
    executor: BatchExecutor,
    postponed_preparation: bool,
    atlas: GlyphAtlas<B>,
    color_atlas: ColorGlyphAtlas<B>,
    pipeline: B::RenderPipeline,
    color_pipeline: B::RenderPipeline,
    viewport_buffer: B::Buffer,
    bind_group_layout: B::BindGroupLayout,
    bind_group: B::BindGroup,
    color_bind_group: B::BindGroup,
    sampler: B::Sampler,
    instance_buffer: B::Buffer,
    instance_policy: InstanceBufferPolicy,
    instances: Vec<GlyphInstance>,
    instance_upload: FrameUpload<GlyphInstance>,
    color_instance_buffer: B::Buffer,
    color_instance_policy: InstanceBufferPolicy,
    color_instances: Vec<GlyphInstance>,
    color_instance_upload: FrameUpload<GlyphInstance>,
    decoration_pipeline: B::RenderPipeline,
    decoration_instance_buffer: B::Buffer,
    decoration_instance_policy: InstanceBufferPolicy,
    decoration_instances: Vec<DecorationInstance>,
    decoration_instance_upload: FrameUpload<DecorationInstance>,
    atlas_generation: u64,
    color_atlas_generation: u64,
    last_viewport: (u32, u32),
    last_prepared_surface: (u32, u32),
    layout_cache: LayoutCache,
    shaping_cache: HashMap<ShapingCacheKey, Arc<ShapedText>>,
    paint_cache: TextPaintCache,
    request_ranges: Vec<TextRequestRange>,
    visible_span_ranges: Vec<Range<usize>>,
    visible_span_keys: Vec<SpanLayoutKeys>,
    alpha_glyph_descriptors: Vec<(GlyphKey, u32, u32)>,
    color_glyph_descriptors: Vec<(GlyphKey, u32, u32)>,
    planned_alpha_descriptors: Vec<(GlyphKey, u32, u32)>,
    planned_color_descriptors: Vec<(GlyphKey, u32, u32)>,
    seen_glyphs: HashSet<GlyphKey>,
    planned_alpha_atlas_generation: u64,
    planned_color_atlas_generation: u64,
    frame_generation: u64,
    prepared_frame_generation: u64,
    prepared_frame: Option<PreparedFrameCache>,
}

// ── Backend-generic text preparation and rendering ─────────────────────────
mod generic_prepare;

impl<B: crate::backend::GpuBackend> TextPipelineV2<B> {
    const INITIAL_CAPACITY: usize = 64;
    const LAYOUT_CACHE_CAPACITY: usize = 4096;
    const SHAPING_CACHE_CAPACITY: usize = 4096;
    /// Creates the pipeline through the selected [`GpuBackend`].
    ///
    /// Creates GPU resources through the selected backend, including shader
    /// modules, atlases, samplers, buffers, bind groups, and render pipelines.
    pub fn new(backend: &B, format: B::TextureFormat, antialiasing: crate::AntiAlias) -> Self {
        use crate::backend::*;

        warm_fallbacks_in_background();

        let rasterizer = GlyphRasterizer::new();
        let atlas = GlyphAtlas::new(backend);
        let color_atlas = ColorGlyphAtlas::new(backend);

        let shader = backend.create_shader_module(
            backend.builtin_shader_source(BuiltinShader::Text),
            "text shader",
        );
        let color_shader = backend.create_shader_module(
            backend.builtin_shader_source(BuiltinShader::TextColor),
            "text color shader",
        );
        let decoration_shader = backend.create_shader_module(
            backend.builtin_shader_source(BuiltinShader::TextDecoration),
            "text decoration shader",
        );

        let sampler = backend.create_sampler(&SamplerDescriptor {
            label: Some("text atlas sampler".to_string()),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: f32::MAX,
            compare: None,
            max_anisotropy: 1,
        });

        let viewport_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("text viewport uniform".to_string()),
            size: 16,
            usage: vec![BufferUsage::Uniform, BufferUsage::CopyDst],
        });

        let bind_group_layout = backend.create_bind_group_layout(&[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: vec![ShaderStage::Vertex, ShaderStage::Fragment],
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: vec![ShaderStage::Fragment],
                ty: BindingType::Texture {
                    multisampled: false,
                    view_dimension: TextureViewDimension::D2,
                    sample_type: TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: vec![ShaderStage::Fragment],
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ]);

        let bind_group = Self::create_bind_group(
            backend,
            &bind_group_layout,
            &viewport_buffer,
            &atlas.view,
            &sampler,
        );
        let color_bind_group = Self::create_bind_group(
            backend,
            &bind_group_layout,
            &viewport_buffer,
            &color_atlas.view,
            &sampler,
        );

        let pipeline_layout = backend.create_pipeline_layout(&[&bind_group_layout]);

        let glyph_buffers = [Some(VertexBufferLayout {
            array_stride: size_of::<GlyphInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &GlyphInstance::GENERIC_ATTRIBUTES,
        })];

        let decoration_buffers = [Some(VertexBufferLayout {
            array_stride: size_of::<DecorationInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &DecorationInstance::GENERIC_ATTRIBUTES,
        })];

        let pipeline = backend.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("text pipeline v2".to_string()),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &glyph_buffers,
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWriteMask::ALL,
                })],
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: crate::pipeline::multisample_state(antialiasing),
        });

        let color_pipeline = backend.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("text color pipeline".to_string()),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &color_shader,
                entry_point: "vs_main",
                buffers: &glyph_buffers,
            },
            fragment: Some(FragmentState {
                module: &color_shader,
                entry_point: "fs_main",
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWriteMask::ALL,
                })],
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: crate::pipeline::multisample_state(antialiasing),
        });

        let decoration_pipeline = backend.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("text decoration pipeline".to_string()),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &decoration_shader,
                entry_point: "vs_main",
                buffers: &decoration_buffers,
            },
            fragment: Some(FragmentState {
                module: &decoration_shader,
                entry_point: "fs_main",
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWriteMask::ALL,
                })],
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: crate::pipeline::multisample_state(antialiasing),
        });

        let instance_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("text instance buffer".to_string()),
            size: (Self::INITIAL_CAPACITY * size_of::<GlyphInstance>()) as u64,
            usage: vec![BufferUsage::Vertex, BufferUsage::CopyDst],
        });

        let color_instance_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("text color instance buffer".to_string()),
            size: (Self::INITIAL_CAPACITY * size_of::<GlyphInstance>()) as u64,
            usage: vec![BufferUsage::Vertex, BufferUsage::CopyDst],
        });

        let decoration_instance_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("text decoration instance buffer".to_string()),
            size: (Self::INITIAL_CAPACITY * size_of::<DecorationInstance>()) as u64,
            usage: vec![BufferUsage::Vertex, BufferUsage::CopyDst],
        });

        Self {
            rasterizer,
            executor: BatchExecutor::new(),
            postponed_preparation: false,
            atlas,
            color_atlas,
            pipeline,
            color_pipeline,
            viewport_buffer,
            bind_group_layout,
            bind_group,
            color_bind_group,
            sampler,
            instance_buffer,
            instance_policy: InstanceBufferPolicy::new(Self::INITIAL_CAPACITY),
            instances: Vec::new(),
            instance_upload: FrameUpload::new(),
            color_instance_buffer,
            color_instance_policy: InstanceBufferPolicy::new(Self::INITIAL_CAPACITY),
            color_instances: Vec::new(),
            color_instance_upload: FrameUpload::new(),
            decoration_pipeline,
            decoration_instance_buffer,
            decoration_instance_policy: InstanceBufferPolicy::new(Self::INITIAL_CAPACITY),
            decoration_instances: Vec::new(),
            decoration_instance_upload: FrameUpload::new(),
            atlas_generation: 0,
            color_atlas_generation: 0,
            last_viewport: (0, 0),
            last_prepared_surface: (0, 0),
            layout_cache: LayoutCache::new(Self::LAYOUT_CACHE_CAPACITY),
            shaping_cache: HashMap::new(),
            paint_cache: TextPaintCache::default(),
            request_ranges: Vec::new(),
            visible_span_ranges: Vec::new(),
            visible_span_keys: Vec::new(),
            alpha_glyph_descriptors: Vec::new(),
            color_glyph_descriptors: Vec::new(),
            planned_alpha_descriptors: Vec::new(),
            planned_color_descriptors: Vec::new(),
            seen_glyphs: HashSet::new(),
            planned_alpha_atlas_generation: 0,
            planned_color_atlas_generation: 0,
            frame_generation: 0,
            prepared_frame_generation: 0,
            prepared_frame: None,
        }
    }

    /// Backend-driven equivalent of the private `create_bind_group` helper.
    fn create_bind_group(
        backend: &B,
        layout: &B::BindGroupLayout,
        viewport_buffer: &B::Buffer,
        atlas_view: &B::TextureView,
        sampler: &B::Sampler,
    ) -> B::BindGroup {
        use crate::backend::{BindGroupEntry, BindingResource};
        backend.create_bind_group(
            layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(viewport_buffer),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(atlas_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(sampler),
                },
            ],
        )
    }

    /// Backend-driven equivalent of [`TextPipelineV2::flush_atlas`].
    ///
    /// Uploads any pending atlas changes and rebuilds the bind groups if
    /// either atlas texture was reallocated (generation changed).
    pub fn flush_atlas(&mut self, backend: &B) {
        self.atlas.upload(backend);
        self.color_atlas.upload(backend);

        let atlas_gen = self.atlas.generation();
        if atlas_gen != self.atlas_generation {
            self.atlas_generation = atlas_gen;
            self.bind_group = Self::create_bind_group(
                backend,
                &self.bind_group_layout,
                &self.viewport_buffer,
                &self.atlas.view,
                &self.sampler,
            );
        }

        let color_gen = self.color_atlas.generation();
        if color_gen != self.color_atlas_generation {
            self.color_atlas_generation = color_gen;
            self.color_bind_group = Self::create_bind_group(
                backend,
                &self.bind_group_layout,
                &self.viewport_buffer,
                &self.color_atlas.view,
                &self.sampler,
            );
        }
    }

    /// Backend-driven viewport-uniform write.
    ///
    /// Writes `[width, height, is_srgb, 0.0]` to the viewport buffer through
    /// [`GpuBackend::write_buffer`], skipping the write when the viewport
    /// size is unchanged since the last call (mirrors the `last_viewport`
    /// gate inside the concrete [`TextPipelineV2::prepare_inner`]).
    pub fn write_viewport(&mut self, backend: &B, width: u32, height: u32, is_srgb: bool) {
        if self.last_viewport == (width, height) {
            return;
        }
        self.last_viewport = (width, height);
        let is_srgb_f32 = if is_srgb { 1.0_f32 } else { 0.0 };
        backend.write_buffer(
            &self.viewport_buffer,
            0,
            bytemuck::cast_slice(&[width as f32, height as f32, is_srgb_f32, 0.0]),
        );
    }

    pub fn render_request<'a>(&'a self, pass: &mut B::RenderPass<'a>, index: usize)
    where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        use crate::backend::GpuRenderPass;
        let Some(range) = self.request_ranges.get(index) else {
            return;
        };
        if range.alpha_end > range.alpha_start {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, &self.instance_buffer, 0);
            pass.draw(0..6, range.alpha_start..range.alpha_end);
        }
        if range.color_end > range.color_start {
            pass.set_pipeline(&self.color_pipeline);
            pass.set_bind_group(0, &self.color_bind_group, &[]);
            pass.set_vertex_buffer(0, &self.color_instance_buffer, 0);
            pass.draw(0..6, range.color_start..range.color_end);
        }
    }

    pub(crate) fn render_decoration_range<'a>(
        &'a self,
        pass: &mut B::RenderPass<'a>,
        start: usize,
        end: usize,
    ) where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        use crate::backend::GpuRenderPass;
        let end = end.min(self.decoration_instances.len());
        if start >= end {
            return;
        }
        pass.set_pipeline(&self.decoration_pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, &self.decoration_instance_buffer, 0);
        pass.draw(0..6, start as u32..end as u32);
    }

    /// Whether the last generic render left viewport-ahead text unprepared.
    #[inline]
    pub fn has_postponed_preparation(&self) -> bool {
        self.postponed_preparation
    }

    /// Returns the capacity-backed GPU allocation for text instance buffers.
    pub fn instance_buffer_bytes(&self) -> u64 {
        (self.instance_policy.capacity() * size_of::<GlyphInstance>()) as u64
            + (self.color_instance_policy.capacity() * size_of::<GlyphInstance>()) as u64
            + (self.decoration_instance_policy.capacity() * size_of::<DecorationInstance>()) as u64
    }

    /// Returns bytes held by the CPU-side glyph bitmap cache.
    pub fn glyph_bitmap_cache_bytes(&self) -> usize {
        self.rasterizer.bitmap_cache_bytes()
    }

    /// Returns GPU atlas texture bytes.
    pub fn glyph_atlas_bytes(&self) -> u64 {
        self.atlas.memory_bytes() + self.color_atlas.memory_bytes()
    }

    /// Returns the number of cached glyphs.
    pub fn cached_glyph_count(&self) -> usize {
        self.rasterizer.cached_glyph_count()
    }

    /// Returns the number of alpha and color glyph instances prepared last.
    pub fn frame_glyph_instances(&self) -> (usize, usize) {
        (self.instances.len(), self.color_instances.len())
    }

    /// Returns the number of retained layout-cache entries.
    pub fn layout_cache_entries(&self) -> usize {
        self.layout_cache.len()
    }
}

#[cfg(all(test, feature = "wgpu"))]
mod tests {
    use std::sync::Arc;

    use super::{
        DecorationInstance, GlyphInstance, PreparedFrameCache, Rgba8, TextDecorationDraw,
        TextDrawRequest, TextHorizontalAlign, TextOverflowMode, TextShadowRequest, TextWritingMode,
        coverage_exponent, glyph_intersects_clip, glyph_quad_size, normalize_pixel_uv_rect,
        shadow_intersects_clip, shadow_is_visible, shadow_padding, snap_to_pixel_grid,
    };

    fn cache_request(text: &str, x: f32) -> TextDrawRequest {
        TextDrawRequest {
            x,
            y: 2.0,
            text: Arc::from(text),
            font_size: 16.0,
            color: Rgba8::new(0, 0, 0, 255),
            bounds_width: 240.0,
            bounds_height: 24.0,
            overflow: TextOverflowMode::Clip,
            horizontal_align: TextHorizontalAlign::Left,
            writing_mode: TextWritingMode::HorizontalTb,
            line_height: None,
            shadow: None,
            draw_glyphs: true,
            font_family: crate::font::FontFamily::SANS_SERIF,
            font_style: crate::font::FontStyle::Normal,
            font_weight: None,
            language: None,
            italic: false,
            clip_rect: [0.0, 0.0, 240.0, 24.0],
            clip_border_radius: [0.0; 4],
            spans: Vec::new(),
        }
    }

    #[test]
    fn prepared_frame_cache_requires_exact_render_inputs() {
        let requests = vec![cache_request("same", 0.0)];
        let cache = PreparedFrameCache::new(400, 300, false, 7, 9, 11, &requests, &[]);

        assert!(cache.matches(400, 300, false, 7, 9, 11, &requests, &[]));
        assert!(!cache.matches(
            400,
            300,
            false,
            7,
            9,
            11,
            &[cache_request("same", 1.0)],
            &[]
        ));
        assert!(!cache.matches(401, 300, false, 7, 9, 11, &requests, &[]));
        assert!(!cache.matches(400, 300, false, 8, 9, 11, &requests, &[]));
        assert!(!cache.matches(400, 300, false, 7, 9, 12, &requests, &[]));
    }

    #[test]
    fn prepared_frame_cache_detects_nested_span_changes() {
        let mut request = cache_request("outer", 0.0);
        request.spans.push(super::RichTextSpan::new("inner"));
        let requests = vec![request];
        let cache = PreparedFrameCache::new(400, 300, false, 0, 0, 0, &requests, &[]);

        let mut changed = cache_request("outer", 0.0);
        changed.spans.push(super::RichTextSpan::new("changed"));
        assert!(!cache.matches(400, 300, false, 0, 0, 0, &[changed], &[]));
    }

    /// How many atlas texels one pixel of `quad_size` spans.
    ///
    /// The pipeline keeps this at exactly `[1.0, 1.0]`; anything else means the
    /// sampler is rescaling the glyph.
    fn texel_scale(quad_size: [f32; 2], region_size: (u32, u32)) -> [f32; 2] {
        let axis = |quad: f32, region: u32| {
            if region == 0 {
                0.0
            } else {
                region as f32 / quad
            }
        };

        [
            axis(quad_size[0], region_size.0),
            axis(quad_size[1], region_size.1),
        ]
    }

    #[test]
    fn glyph_culling_keeps_unclipped_and_partially_visible_glyphs() {
        assert!(glyph_intersects_clip(
            [500.0, 500.0],
            [20.0, 20.0],
            [0.0, 0.0, -1.0, 0.0],
        ));
        assert!(glyph_intersects_clip(
            [90.0, 90.0],
            [20.0, 20.0],
            [0.0, 0.0, 100.0, 100.0],
        ));
        assert!(glyph_intersects_clip(
            [-10.0, 10.0],
            [20.0, 20.0],
            [0.0, 0.0, 100.0, 100.0],
        ));
    }

    #[test]
    fn glyph_culling_rejects_glyphs_fully_outside_clip() {
        assert!(!glyph_intersects_clip(
            [101.0, 10.0],
            [20.0, 20.0],
            [0.0, 0.0, 100.0, 100.0],
        ));
        assert!(!glyph_intersects_clip(
            [10.0, -21.0],
            [20.0, 20.0],
            [0.0, 0.0, 100.0, 100.0],
        ));
    }

    #[test]
    fn shadow_culling_keeps_a_shadow_inside_an_otherwise_outside_clip() {
        let shadow = TextShadowRequest {
            offset_x: -24.0,
            offset_y: 0.0,
            blur: 2.0,
            color: Rgba8::new(0, 0, 0, 128),
        };

        assert!(shadow_intersects_clip(
            [105.0, 10.0],
            [8.0, 12.0],
            shadow,
            [0.0, 0.0, 100.0, 100.0],
        ));
        assert_eq!(shadow_padding(shadow), 26.0);
    }

    #[test]
    fn transparent_shadow_alpha_paints_nothing() {
        assert!(!shadow_is_visible(Rgba8::TRANSPARENT));
        assert!(shadow_is_visible(Rgba8::new(0, 0, 0, 128)));
    }

    #[test]
    fn pixel_uv_rect_is_normalized_against_the_final_atlas_size() {
        let pixel_rect = [64.0, 32.0, 96.0, 64.0];

        assert_eq!(
            normalize_pixel_uv_rect(pixel_rect, 256, 128),
            [0.25, 0.25, 0.375, 0.5]
        );
        assert_eq!(
            normalize_pixel_uv_rect(pixel_rect, 512, 256),
            [0.125, 0.125, 0.1875, 0.25]
        );
    }

    // A glyph's coverage is written as an alpha the GPU blends in *linear*
    // light on an sRGB target. Blending there is not what the rasterizer meant:
    // half coverage of black over a light background comes out far lighter than
    // half way to black, so a stroke's apparent weight depends on how much of it
    // sits on partially covered pixels. The exponent below is what puts the
    // weight back.
    #[test]
    fn dark_text_has_its_partial_coverage_strengthened() {
        let exponent = coverage_exponent([0.0, 0.0, 0.0, 1.0]);

        assert!(exponent < 1.0, "dark text must gain coverage: {exponent}");
        assert!((0.5_f32.powf(exponent) - 0.729).abs() < 0.01);
    }

    #[test]
    fn light_text_has_its_partial_coverage_weakened() {
        let exponent = coverage_exponent([1.0, 1.0, 1.0, 1.0]);

        assert!(exponent > 1.0, "light text must lose coverage: {exponent}");
        assert!((0.5_f32.powf(exponent) - 0.217).abs() < 0.01);
    }

    // Mid-luminance text is the one case linear blending already gets right,
    // and the correction must not disturb it — otherwise every mid-tone label
    // in the framework changes weight for nothing.
    #[test]
    fn mid_luminance_text_is_left_alone() {
        let exponent = coverage_exponent([0.5, 0.5, 0.5, 1.0]);

        assert!((exponent - 1.0).abs() < 0.05, "{exponent}");
    }

    // Luminance, not the plain channel average: pure blue is dark, pure green
    // is light, and they must be corrected in opposite directions.
    #[test]
    fn the_exponent_follows_perceived_luminance() {
        assert!(coverage_exponent([0.0, 0.0, 1.0, 1.0]) < 1.0);
        assert!(coverage_exponent([0.0, 1.0, 0.0, 1.0]) > 1.0);
    }

    #[test]
    fn colors_outside_the_unit_range_are_still_answered() {
        for color in [
            [-1.0, -1.0, -1.0, 1.0],
            [2.0, 2.0, 2.0, 1.0],
            [f32::NAN, 0.0, 0.0, 1.0],
        ] {
            let exponent = coverage_exponent(color);
            assert!(
                exponent.is_finite() && exponent > 0.0,
                "{color:?} produced {exponent}"
            );
        }
    }

    // The defect in one number: black text at half coverage over a light grey
    // background. Gamma-space compositing — what the rasterizer's coverage
    // means, and what a native text stack produces — lands at half the
    // background's sRGB value; blending the raw coverage in linear light lands
    // far lighter. The correction has to close most of that gap.
    #[test]
    fn corrected_coverage_lands_near_gamma_space_compositing() {
        let background = 0.647_f32;
        let target = background * 0.5;

        let composite = |alpha: f32| {
            let background_linear = srgb_to_linear(background);
            linear_to_srgb(background_linear * (1.0 - alpha))
        };

        let raw = composite(0.5);
        let corrected = composite(0.5_f32.powf(coverage_exponent([0.0, 0.0, 0.0, 1.0])));

        assert!(raw > target + 0.1, "the defect must be visible: {raw}");
        assert!(
            (corrected - target).abs() < (raw - target).abs() * 0.5,
            "correction left {corrected}, target {target}, was {raw}"
        );
    }

    fn srgb_to_linear(channel: f32) -> f32 {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    fn linear_to_srgb(channel: f32) -> f32 {
        if channel <= 0.0031308 {
            channel * 12.92
        } else {
            1.055 * channel.powf(1.0 / 2.4) - 0.055
        }
    }

    // A glyph's quad is sized by the pipeline, but the pixels inside it come
    // from the atlas region its UVs address. The two are produced by different
    // passes — layout measures the glyph, the atlas stores whatever bitmap was
    // rasterized for its key — so nothing but this invariant keeps them equal,
    // and a single pixel of disagreement resamples the whole glyph.
    #[test]
    fn a_glyph_quad_samples_one_texel_per_pixel() {
        for region in [(38, 38), (1, 1), (12, 27)] {
            assert_eq!(texel_scale(glyph_quad_size(region), region), [1.0, 1.0]);
        }
    }

    // The defect this invariant exists for: the layout measured the glyph one
    // pixel smaller than the bitmap the atlas holds. Sizing the quad by that
    // measurement rescales every stroke through the sampler.
    #[test]
    fn a_quad_disagreeing_with_its_region_resamples_the_glyph() {
        let scale = texel_scale([38.0, 38.0], (39, 39));

        assert!(scale[0] != 1.0 && scale[1] != 1.0, "{scale:?}");
    }

    // An empty region has nothing to sample; the scale must stay finite rather
    // than divide its way to infinity.
    #[test]
    fn an_empty_region_has_no_scale() {
        assert_eq!(texel_scale(glyph_quad_size((0, 0)), (0, 0)), [0.0, 0.0]);
    }

    #[test]
    fn a_glyph_quad_is_placed_on_a_whole_pixel() {
        assert_eq!(snap_to_pixel_grid([10.4, 20.6]), [10.0, 21.0]);
        assert_eq!(snap_to_pixel_grid([-3.4, -0.2]), [-3.0, -0.0]);
        assert_eq!(snap_to_pixel_grid([7.0, 9.0]), [7.0, 9.0]);
    }

    // The defect this guards: the same label rendered twice, once at an integer
    // origin and once shifted by half a pixel — as a line centred in a box too
    // narrow for it is — must produce the same quads, or the linear atlas
    // sampler blurs one of them and its strokes lose weight.
    #[test]
    fn the_same_text_lands_on_the_same_pixels_wherever_its_line_starts() {
        let glyph_offsets = [0.0_f32, 6.34, 12.68, 19.02];
        let crisp: Vec<[f32; 2]> = glyph_offsets
            .iter()
            .map(|offset| snap_to_pixel_grid([40.0 + offset, 12.0]))
            .collect();
        let shifted: Vec<[f32; 2]> = glyph_offsets
            .iter()
            .map(|offset| snap_to_pixel_grid([40.5 + offset, 12.0]))
            .collect();

        for position in crisp.iter().chain(&shifted) {
            assert_eq!(position[0], position[0].round());
            assert_eq!(position[1], position[1].round());
        }
        let phases: Vec<f32> = shifted
            .iter()
            .zip(&crisp)
            .map(|(shifted, crisp)| shifted[0] - crisp[0])
            .collect();
        assert!(
            phases.iter().all(|phase| phase.abs() <= 1.0),
            "snapping moved a glyph by more than a pixel: {phases:?}"
        );
    }

    // Guards the CPU->GPU packing of a decoration line: `params` must be
    // [style_id, thickness, period, band_height] and geometry must map to the
    // instance's position/size, matching what `text_decoration.wgsl` reads.
    #[test]
    fn decoration_instance_packing() {
        let draw = TextDecorationDraw {
            x: 10.0,
            y: 20.0,
            width: 120.0,
            band_height: 6.0,
            thickness: 2.0,
            period: 8.0,
            style: 4, // Wavy
            color: Rgba8::new(255, 0, 0, 255),
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
        };
        let inst = draw.to_instance();
        assert_eq!(inst.position, [10.0, 20.0]);
        assert_eq!(inst.size, [120.0, 6.0]);
        assert_eq!(inst.color, Rgba8::new(255, 0, 0, 255));
        // params: style, thickness, period, band_height (band_height duplicated
        // so the fragment shader has it without relying on the interpolated size).
        assert_eq!(inst.params, [4.0, 2.0, 8.0, 6.0]);
    }

    #[test]
    fn text_instances_use_unorm_bytes_for_paint_colors() {
        assert_eq!(std::mem::size_of::<GlyphInstance>(), 84);
        assert_eq!(
            GlyphInstance::ATTRIBS[3].format,
            wgpu::VertexFormat::Unorm8x4
        );
        assert_eq!(
            GlyphInstance::ATTRIBS[3].offset,
            std::mem::offset_of!(GlyphInstance, color) as u64
        );
        assert_eq!(std::mem::size_of::<DecorationInstance>(), 68);
        assert_eq!(
            DecorationInstance::ATTRIBS[2].format,
            wgpu::VertexFormat::Unorm8x4
        );
        assert_eq!(
            DecorationInstance::ATTRIBS[2].offset,
            std::mem::offset_of!(DecorationInstance, color) as u64
        );
    }

    // ── Generic backend path (stable backend-generic path) ───────────────────────
    //
    // Exercises `TextPipelineV2::new`, `GlyphAtlas::get_or_insert`
    // / `flush_atlas` (atlas insertion + upload through the backend
    // trait), and `write_viewport` (viewport uniform write through the
    // backend trait), then renders the resulting instance through the same
    // `render_request` the concrete path uses (valid because
    // `WgpuBackend::RenderPass == wgpu::RenderPass`) and reads back a pixel to
    // prove the whole chain actually ran on the GPU. Lives here (rather than in
    // `lib.rs`'s `generic_backend_tests`) because it needs direct access to
    // private fields (`atlas`, `instances`, `request_ranges`, …) to bypass the
    // full CPU text-shaping pipeline, which is out of scope for this pass.
    #[test]
    fn generic_text_pipeline_renders_opaque_glyph_quad() {
        use crate::backend::GpuBackend;
        use crate::backend::wgpu::WgpuBackend;
        use crate::text_pipeline::glyph_rasterizer::GlyphKey;
        use aimer_utils::SyncFuture;

        const SIZE: u32 = 64;
        const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

        let gpu = (|| {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .block()
                .ok()?;
            adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("cupid text generic-backend device"),
                    ..Default::default()
                })
                .block()
                .ok()
        })();
        let Some((device, queue)) = gpu else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        let backend = WgpuBackend::new(device, queue);

        let mut pipeline = super::TextPipelineV2::new(&backend, FORMAT, crate::AntiAlias::Analytic);

        // Insert a fully opaque 8×8 alpha glyph into the atlas and flush it to
        // the GPU texture through the backend-driven upload path.
        let key = GlyphKey::new(1, 1, 16.0);
        let bitmap = vec![255u8; 8 * 8];
        let region = pipeline.atlas.get_or_insert(&backend, key, 8, 8, &bitmap);
        pipeline.flush_atlas(&backend);
        pipeline.write_viewport(&backend, SIZE, SIZE, false);

        let uv_rect = region.uvs(pipeline.atlas.width, pipeline.atlas.height);
        let instance = GlyphInstance {
            position: [0.0, 0.0],
            size: [SIZE as f32, SIZE as f32],
            uv_rect,
            color: Rgba8::new(255, 0, 0, 255),
            // width < 0 disables clipping.
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            skew: 0.0,
            coverage_exponent: 1.0,
            _pad: [0.0; 2],
        };
        pipeline.instances.push(instance);
        pipeline.request_ranges.push(super::TextRequestRange {
            alpha_start: 0,
            alpha_end: 1,
            color_start: 0,
            color_end: 0,
        });
        backend.write_buffer(
            &pipeline.instance_buffer,
            0,
            bytemuck::cast_slice(&pipeline.instances),
        );

        let target = backend.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text generic test target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = backend
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text generic test pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pipeline.render_request(&mut pass, 0);
        }
        backend.queue.submit(Some(encoder.finish()));

        let readback = backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text generic test readback"),
            size: (SIZE * SIZE * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut copy_encoder = backend
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        copy_encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        backend.queue.submit(Some(copy_encoder.finish()));

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| {
            result.expect("the readback buffer to map");
        });
        backend
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device to finish the readback");
        let pixels = slice
            .get_mapped_range()
            .expect("the mapped readback range")
            .to_vec();
        readback.unmap();

        let stride = SIZE as usize * 4;
        let (cx, cy) = (SIZE as usize / 2, SIZE as usize / 2);
        let offset = cy * stride + cx * 4;
        let pixel = [
            pixels[offset],
            pixels[offset + 1],
            pixels[offset + 2],
            pixels[offset + 3],
        ];
        assert_eq!(
            pixel,
            [255, 0, 0, 255],
            "expected the generic backend path to render an opaque red glyph quad, got {pixel:?}"
        );
    }
}
