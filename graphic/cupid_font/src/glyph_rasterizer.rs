mod glyph_run;
mod run_advances;

pub use run_advances::RunAdvances;

use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use hashbrown::{HashMap, HashSet};

use unicode_segmentation::UnicodeSegmentation;

pub(super) use self::glyph_run::{
    GlyphRun, SEQUENTIAL_MAX_GLYPHS_PER_RUN, glyph_runs, group_into_runs,
};
use super::text_layout::FontId;
use crate::font::{FontFamily, FontRegistry, FontStyle, FontWeight, TextLanguage};
use crate::text_pipeline::font_resolver::{
    FallbackScript, FontData, FontRecord, SharedFontRecord, fallback_script_for_codepoint,
    fallback_script_for_font_id, shared_fallback_chain_for_script,
};
use crate::text_pipeline::glyph_metrics::{self, GlyphMetrics};
use crate::text_pipeline::system_fallback::{
    SYSTEM_FALLBACK_ID_BASE, ScriptRequirement, WEIGHT_MATCH_TOLERANCE, fallback_by_id,
    fallback_glyph_for_codepoint, script_probes,
};
#[cfg(not(any(feature = "bundled-fonts", target_arch = "wasm32", test)))]
use crate::text_pipeline::system_fallback::{system_monospace_font, system_primary_font};
use crate::text_pipeline::unicode_script::Script;

// Counts the glyph lookups made for characters, so tests can show that
// measuring a run resolves each distinct character once. Compiled out of every
// non-test build.
#[cfg(test)]
thread_local! {
    static KEY_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn key_lookups() -> usize {
    KEY_LOOKUPS.with(std::cell::Cell::get)
}

/// The bundled primary face is compiled for the opt-in deterministic profile,
/// wasm targets, and this module's unit tests. Browsers do not expose a
/// readable system font file to the wasm fallback path, so the generic face
/// must be available locally there.
#[cfg(any(feature = "bundled-fonts", target_arch = "wasm32", test))]
const PRIMARY_FONT: &[u8] =
    include_bytes!("../../../fonts/GoogleSans-VariableFont_GRAD,opsz,wght.ttf");
const MONOSPACE_FONT_ID: FontId = 0x7fff_fffe;
/// Stable id reserved for the self-rasterized CJK fallback.
///
/// It lives below [`SYSTEM_FALLBACK_ID_BASE`] so a key can be recovered from a
/// worker snapshot like any other internally decoded face, while staying away
/// from ids assigned to runtime registrations and the platform fallback lanes.
const BUNDLED_CJK_FONT_ID: FontId = 0x2000_0000;
#[cfg(any(feature = "bundled-fonts", test))]
const BUNDLED_CJK_FONT: &[u8] = include_bytes!("../../../fonts/NotoSansJP-VariableFont_wght.ttf");
#[cfg(any(feature = "bundled-fonts", test))]
static BUNDLED_CJK_RECORD: OnceLock<Option<FontRecord>> = OnceLock::new();

/// A rasterized glyph bitmap with its metrics.
///
/// `bitmap` layout depends on `is_color`:
///   * `is_color == false` — `width * height` bytes, single-channel coverage
///     (8-bit alpha), as produced by the active outline rasterizer.
///   * `is_color == true`  — `width * height * 4` bytes, RGBA8
///     (non-premultiplied), as produced by the owned COLR compositor or the
///     compatibility color renderer.
///
/// The text pipeline routes color glyphs to a separate RGBA8 atlas + shader.
#[derive(Clone)]
pub struct RasterizedGlyph {
    pub bitmap: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Horizontal offset from the pen position to the left edge of the bitmap.
    pub offset_x: f32,
    /// Vertical offset from the baseline to the bottom edge of the bitmap
    /// (y-up, matches the font's scaled glyph bounding-box minimum y.
    pub offset_y: f32,
    /// Horizontal advance width.
    pub advance_width: f32,
    /// Whether the bitmap is RGBA8 color data (true) or single-channel alpha
    /// (false).
    pub is_color: bool,
}

/// OpenType weight a glyph key carries when nothing demands another one.
///
/// Static faces Cupid rasterizes itself draw their single design regardless of
/// the requested weight, so their keys stay on this value and one bitmap
/// serves every style. Variable faces are the exception: their keys carry the
/// selected `wght` instance, just like platform-only faces — see
/// [`GlyphRasterizer::glyph_weight_for_request`].
pub(crate) const NORMAL_GLYPH_WEIGHT: u16 = 400;

/// OpenType weight from which text reads as emphasized.
///
/// Semibold is the first cut a reader takes for bold, and it is the weight
/// Apple's own UI faces pair with bold text, so the threshold sits there
/// rather than at `700`.
pub(crate) const BOLD_WEIGHT_THRESHOLD: u16 = 600;
static PRIMARY_PRINTABLE_ASCII_COVERAGE: OnceLock<bool> = OnceLock::new();

/// Returns whether a glyph needs a small synthetic stroke to reach the
/// requested weight. Regular text can need this too: a fallback face may only
/// expose a W3/300 cut while the primary face is drawn at regular/400.
#[inline]
fn synthetic_weight_needed(requested: u16, drawn: u16) -> bool {
    if requested < NORMAL_GLYPH_WEIGHT {
        return false;
    }

    let delta = requested.saturating_sub(drawn);
    if requested >= BOLD_WEIGHT_THRESHOLD {
        delta > WEIGHT_MATCH_TOLERANCE
    } else {
        // At regular and medium weights, a one-cut deficit is visible and
        // should be corrected. Bold keeps the historical strict comparison so
        // a neighbouring semibold cut is not emboldened twice.
        delta >= WEIGHT_MATCH_TOLERANCE
    }
}

/// Computes the offset for the second coverage sample used by synthetic
/// weight normalization. The offset scales with the requested deficit and is
/// capped at the existing bold stroke so normal fallback glyphs receive only
/// the amount of ink they are missing.
#[inline]
fn synthetic_weight_offset_for(font_size: f32, requested: u16, drawn: u16) -> Option<f32> {
    if !synthetic_weight_needed(requested, drawn) {
        return None;
    }

    let deficit = f32::from(requested.saturating_sub(drawn));
    let bold_span = f32::from(BOLD_WEIGHT_THRESHOLD - NORMAL_GLYPH_WEIGHT);
    let fraction = (deficit / bold_span.max(1.0)).clamp(0.0, 1.0);
    Some((font_size.max(1.0) * 0.03 * fraction).max(0.5))
}

/// Key for caching rasterized-shaped glyphs.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub struct GlyphKey {
    pub font_id: FontId,
    pub glyph_id: u16,
    pub size_tenths: u32,
    pub subpixel_x: u8,
    pub subpixel_y: u8,
    /// OpenType `wght` the glyph is rasterized at.
    ///
    /// [`NORMAL_GLYPH_WEIGHT`] for static faces Cupid decodes itself; the
    /// selected `wght` instance for readable variable faces and platform-only
    /// faces, so one variable face can stand beside light and bold text alike
    /// without every entry — cache, metrics, atlas — collapsing onto a single
    /// stroke.
    pub weight: u16,
    /// Per-face identity for an arbitrary OpenType variation instance.
    ///
    /// Zero selects the legacy/default coordinate path. Non-zero values are
    /// interned by the shared Aimer face and make outline, metric, bitmap, and
    /// atlas caches distinguish otherwise identical glyph requests.
    pub variation_id: u32,
}

impl Hash for GlyphKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let font_and_size = u64::from(self.font_id) | (u64::from(self.size_tenths) << 32);
        let glyph_subpixel_and_weight = u64::from(self.glyph_id)
            | (u64::from(self.subpixel_x) << 16)
            | (u64::from(self.subpixel_y) << 24)
            | (u64::from(self.weight) << 32);
        let compact = font_and_size
            ^ glyph_subpixel_and_weight
                .wrapping_mul(0x9e37_79b1_85eb_ca87)
                .rotate_left(17);
        state.write_u64(compact ^ u64::from(self.variation_id).rotate_left(37));
    }
}

#[derive(Clone, Copy)]
pub struct ShapedRunGlyph {
    pub glyph_key: GlyphKey,
    /// Horizontal advance in pixels. Vertical runs carry their pen movement
    /// separately in [`Self::y_advance`].
    pub advance: f32,
    /// Vertical advance in pixels, normally zero for horizontal text.
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub cluster: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SyntheticWeightPlan {
    extra_offsets: [f32; 2],
    extra_count: u8,
}

impl SyntheticWeightPlan {
    pub(crate) fn extra_offsets(&self) -> &[f32] {
        &self.extra_offsets[..usize::from(self.extra_count)]
    }

    #[cfg(test)]
    fn span(&self) -> f32 {
        let mut minimum = 0.0_f32;
        let mut maximum = 0.0_f32;
        for &offset in self.extra_offsets() {
            minimum = minimum.min(offset);
            maximum = maximum.max(offset);
        }
        maximum - minimum
    }
}

#[derive(Default)]
struct RunBuffers {
    pending_keys: Vec<GlyphKey>,
    fallback_keys: Vec<GlyphKey>,
    metric_keys: Vec<GlyphKey>,
    shaped_glyphs: Vec<ShapedRunGlyph>,
    prepared: Vec<(GlyphKey, RasterizedGlyph)>,
    pending_seen: HashSet<GlyphKey>,
    metric_values: Vec<Option<GlyphMetrics>>,
}

impl GlyphKey {
    #[inline]
    pub fn new(font_id: FontId, glyph_id: u16, font_size: f32) -> Self {
        Self {
            font_id,
            glyph_id,
            size_tenths: (font_size * 10.0) as u32,
            subpixel_x: 0,
            subpixel_y: 0,
            weight: NORMAL_GLYPH_WEIGHT,
            variation_id: 0,
        }
    }

    /// Returns this key addressed at `weight` on the OpenType `wght` scale.
    #[inline]
    pub fn weighted(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    /// Returns this key addressed at an interned arbitrary variation
    /// instance. The id is face-local and is normally obtained from
    /// Aimer's variation-request API.
    #[inline]
    pub fn with_variation_id(mut self, variation_id: u32) -> Self {
        self.variation_id = variation_id;
        self
    }
}

// Aimer owns the parsed face, outline, and coverage caches. No compatibility
// scaling context is needed here.

#[cfg(any(feature = "bundled-fonts", target_arch = "wasm32", test))]
fn monospace_font_record() -> Option<FontRecord> {
    static MONOSPACE_FONT_RECORD: OnceLock<Option<FontRecord>> = OnceLock::new();
    MONOSPACE_FONT_RECORD
        .get_or_init(|| {
            FontRecord::from_static_bytes(
                MONOSPACE_FONT_ID,
                include_bytes!("../../../fonts/JetBrainsMono-Regular.ttf"),
            )
        })
        .clone()
}

#[cfg(not(any(feature = "bundled-fonts", target_arch = "wasm32", test)))]
fn monospace_font_record() -> Option<FontRecord> {
    system_monospace_font(MONOSPACE_FONT_ID)
}

#[cfg(any(feature = "bundled-fonts", target_arch = "wasm32", test))]
fn primary_font_record() -> FontRecord {
    static PRIMARY_FONT_RECORD: OnceLock<Option<FontRecord>> = OnceLock::new();
    PRIMARY_FONT_RECORD
        .get_or_init(|| FontRecord::from_static_bytes(0, PRIMARY_FONT))
        .clone()
        .expect("failed to load primary font")
}

#[cfg(not(any(feature = "bundled-fonts", target_arch = "wasm32", test)))]
fn primary_font_record() -> FontRecord {
    system_primary_font().unwrap_or_else(|| FontRecord::unavailable(0))
}

// ---------------------------------------------------------------------------
// Color glyph rasterization (embedded bitmaps and layered outlines)
// ---------------------------------------------------------------------------
/// Rasterizes a glyph through the platform, for faces Cupid cannot decode.
///
/// Apple ships system fonts whose glyph data uses private formats — `hvgl`
/// outlines in the Chinese UI face, `emjc` compressed strikes in the iOS emoji
/// face — which the owned outline decoder cannot read. The platform can, and the
/// bitmap it produces is indistinguishable from a decoded one downstream.
///
/// `weight` is the OpenType `wght` from the glyph key; the platform pins a
/// variable face to it so the stroke matches the text standing beside the
/// glyph. The advance is passed through rather than re-queried so a glyph
/// keeps the `hmtx` advance shaping and layout already used for it.
#[cfg(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
))]
fn rasterize_platform_glyph(
    record: &FontRecord,
    glyph_id: u16,
    font_size: f32,
    weight: u16,
    advance_width: f32,
) -> Option<RasterizedGlyph> {
    crate::text_pipeline::core_text_raster::rasterize_glyph(
        record.path()?,
        record.collection_index,
        glyph_id,
        font_size,
        weight,
        record.is_color,
        advance_width,
    )
}

/// Platforms whose fonts Cupid can always decode itself.
#[cfg(not(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
)))]
fn rasterize_platform_glyph(
    _record: &FontRecord,
    _glyph_id: u16,
    _font_size: f32,
    _weight: u16,
    _advance_width: f32,
) -> Option<RasterizedGlyph> {
    None
}

/// The kana that names the default companion face for runs carrying none.
///
/// A Han-only run has no kana of its own to pair with, yet drawing it at a
/// different weight than a mixed run would means the whole line changes
/// stroke the moment a kana is typed beside it. Resolving the face this
/// probe *would* use gives kana-less runs the same baseline up front — see
/// [`GlyphRasterizer::default_companion_weight`].
const COMPANION_WEIGHT_PROBE: char = 'あ';

/// Reports whether `codepoint` is kana — hiragana or katakana, including the
/// phonetic extensions and halfwidth forms.
///
/// Kana is what marks a run as Japanese: it identifies the face whose stroke
/// weight the run's platform-drawn ideographs must match — see
/// [`GlyphRasterizer::platform_glyph_weight`].
#[inline]
fn is_kana(codepoint: char) -> bool {
    matches!(
        codepoint as u32,
        0x3041..=0x309F | 0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F
    )
}

/// Reports whether `record`'s glyph data is readable only by the platform.
///
/// A face qualifies when it carries neither outline tables Cupid decodes —
/// `glyf`, `CFF `, `CFF2` — nor color strikes: everything needed for shaping
/// is present, but the pixels live in a private format such as Apple's
/// `hvgl`. The check reads only the table directory, so it costs no parsing
/// beyond the header.
fn record_outlines_unreadable(record: &FontRecord) -> bool {
    if record.is_color {
        return false;
    }
    !record.has_standard_outline()
}

#[inline]
pub fn point_inside(contours: &[Vec<(f32, f32)>], x: f32, y: f32) -> bool {
    let mut inside = false;
    for contour in contours {
        let mut prev = *contour.last().expect("contour is non-empty");
        for &curr in contour {
            if (curr.1 > y) != (prev.1 > y)
                && x < (prev.0 - curr.0) * (y - curr.1) / (prev.1 - curr.1) + curr.0
            {
                inside = !inside;
            }
            prev = curr;
        }
    }
    inside
}

/// Immutable font ownership copied into worker-local preparation contexts.
///
/// Font bytes and parsed font objects remain reference counted, while every
/// context created from this snapshot receives independent shaping and bitmap
/// caches. Fallback discovery remains lazy when the source rasterizer has not
/// loaded its fallback chain yet.
#[derive(Clone)]
pub(super) struct FontSnapshot {
    primary: SharedFontRecord,
    family_faces: Arc<[SharedFamilyFontRecord]>,
    registered_font_revision: u64,
    fallbacks: Option<Arc<[SharedFontRecord]>>,
    system_fallbacks_loaded: bool,
    loaded_fallback_scripts: Arc<[FallbackScript]>,
    registered_fallback_ids: Arc<[FontId]>,
    enable_fallbacks: bool,
    /// Fallback face decisions resolved while the owner announced the batch.
    /// Workers clone this small map so they do not repeat the same chain and
    /// platform lookup for every grapheme of a shaped span.
    resolved_codepoint_cache: Arc<HashMap<(char, ScriptRequirement, u16), (FontId, u16, bool)>>,
    /// The companion baseline discovered while resolving kana-less runs.
    /// Carrying the tri-state value keeps workers from probing the same kana
    /// solely to reconstruct an already-known effective weight.
    default_companion_weight: Option<Option<u16>>,
    /// Parsed Aimer faces that were made ready before worker execution.
    aimer_faces: Arc<
        [(
            FontId,
            crate::text_pipeline::aimer_font::SharedParsedAimerFont,
        )],
    >,
}

#[derive(Clone)]
struct SharedFamilyFontRecord {
    family: FontFamily,
    weight: u16,
    style: FontStyle,
    record: SharedFontRecord,
}

/// Mutable CPU-only text preparation state owned by one worker job.
///
/// This context contains no atlas, GPU object, canvas state, or renderer cache.
/// Its rasterizer and scratch buffers are never shared between workers.
pub(super) struct GlyphPreparationContext {
    rasterizer: GlyphRasterizer,
}

impl GlyphPreparationContext {
    pub(super) fn new(snapshot: FontSnapshot) -> Self {
        Self {
            rasterizer: GlyphRasterizer::from_font_snapshot(snapshot),
        }
    }

    pub(super) fn rasterizer_mut(&mut self) -> &mut GlyphRasterizer {
        &mut self.rasterizer
    }

    /// Rasterizes a whole run and hands back every glyph it produced.
    ///
    /// A run is the unit of glyph preparation because the face behind it is
    /// then resolved, mapped and scaled once instead of once per glyph.
    /// Coverage that was released to stay within the cache budget is drawn
    /// again, since the renderer asking for a run needs the bitmaps.
    pub(super) fn prepare_glyph_run(&mut self, run: &GlyphRun) -> Vec<(GlyphKey, RasterizedGlyph)> {
        self.rasterizer.discard_partial_bitmaps(&run.keys);
        self.rasterizer.rasterize_run(&run.keys, run.font_size);

        run.keys
            .iter()
            .filter_map(|key| {
                self.rasterizer
                    .cached_glyph_for_commit(*key)
                    .map(|glyph| (*key, glyph))
            })
            .collect()
    }
}

pub struct GlyphRasterizer {
    /// Primary sans-serif font, either bundled or selected from the host.
    primary: FontRecord,
    family_faces: Vec<FamilyFontRecord>,
    registered_font_revision: u64,
    /// Fallback fonts for extended Unicode coverage (CJK, etc.).
    /// Loaded lazily on first encounter of a glyph not in the primary font,
    /// to avoid the massive memory cost (~800MB) of parsing large CJK fonts
    /// when only ASCII text is rendered.
    fallbacks: Option<Vec<FontRecord>>,
    /// Whether the broad platform/discovered fallback chain has been loaded.
    /// `fallbacks` may already contain an explicitly
    /// registered face without implying that the whole chain is resident.
    system_fallbacks_loaded: bool,
    /// Script lanes already requested by this rasterizer. A lane is recorded
    /// separately from the broad-chain flag so one emoji miss does not make a
    /// later CJK lookup believe every fallback lane is resident.
    loaded_fallback_scripts: HashSet<FallbackScript>,
    /// Runtime-registered faces are owned by the caller and survive a
    /// process-local fallback release.
    registered_fallback_ids: HashSet<FontId>,
    /// Whether to attempt loading fallbacks when needed.
    enable_fallbacks: bool,
    cache: HashMap<GlyphKey, RasterizedGlyph>,
    retained_bitmap_bytes: usize,
    advance_cache: HashMap<GlyphKey, f32>,
    /// Lazy cmap and script-coverage answers, partitioned by face.
    coverage_index_cache: HashMap<FontId, CoverageIndex>,
    /// Whether the fixed primary face covers the printable ASCII range.
    ///
    /// The shaping frontend can use one known face for a plain ASCII run
    /// instead of repeating family and coverage resolution for every cluster.
    primary_printable_ascii_coverage: Option<bool>,
    /// What the text currently being shaped or measured demands of a face.
    ///
    /// Set by [`Self::begin_script_run`] for the passes that hold the whole
    /// string, so a kanji is judged against its own line rather than against a
    /// fixed sample of another language's script.
    script_run: ScriptRequirement,
    /// First kana of the announced run, when it has one.
    ///
    /// The kana names the face whose stroke weight the run's platform-drawn
    /// ideographs must match — see [`Self::glyph_weight_for_request`].
    run_companion: Option<char>,
    /// The design weight of the face [`COMPANION_WEIGHT_PROBE`] resolves to,
    /// memoized after the first kana-less run asks for it.
    ///
    /// This is the companion baseline of runs carrying no kana of their own
    /// — see [`Self::default_companion_weight`].
    default_companion_weight: Option<Option<u16>>,
    /// Temporarily prevents the bundled Japanese face from answering the
    /// companion-weight probe for an ambiguous Han-only run.
    skip_bundled_cjk: bool,
    /// Whether a face's glyph data is readable only by the platform, per face.
    ///
    /// Feeds [`Self::glyph_weight_for_request`]; the answer costs a table-directory
    /// probe, so it is remembered rather than re-derived per glyph.
    platform_only_cache: HashMap<FontId, bool>,
    /// A face's design weight — its `OS/2` weight class — per face.
    design_weight_cache: HashMap<FontId, Option<u16>>,
    /// Whether a readable face exposes a valid `fvar` `wght` variation model.
    variable_font_cache: HashMap<FontId, bool>,
    unsupported_codepoints: HashSet<char>,
    /// Cached font bytes per font_id to avoid re-reading from disk or
    /// re-cloning Arc<[u8]> on every `shape_cluster` call.
    font_bytes_cache: HashMap<FontId, FontData>,
    /// Cached parsed Aimer face and OpenType layout state per font id.
    aimer_font_cache: HashMap<FontId, Option<crate::text_pipeline::aimer_font::AimerFontState>>,
    /// Reusable storage for the current raster batch. Keeping this alongside
    /// the rasterizer avoids allocating one pending/output vector per run.
    run_buffers: RunBuffers,
    /// Resolved fallback answers for one announced script requirement and
    /// effective weight. A shaped run asks for the same face decision once per
    /// grapheme; retaining the complete answer avoids rescanning the fallback
    /// chain and re-entering the process-wide system resolver for each one.
    /// Entries are cleared whenever the available face set changes.
    resolved_codepoint_cache: HashMap<(char, ScriptRequirement, u16), (FontId, u16, bool)>,
    #[cfg(test)]
    shape_call_count: usize,
    #[cfg(test)]
    rasterize_call_count: usize,
    #[cfg(test)]
    simple_ltr_path_count: usize,
    #[cfg(test)]
    reused_shape_output_count: usize,
}

#[derive(Clone)]
struct FamilyFontRecord {
    family: FontFamily,
    weight: u16,
    style: FontStyle,
    record: FontRecord,
}

/// Lazy coverage answers owned by one face.
///
/// Keeping the cmap answers and the script decisions together makes
/// invalidation face-granular: replacing one registered face does not evict
/// every other face's coverage work. The index is intentionally lazy because
/// CJK faces contain tens of thousands of mappings while most runs touch only
/// a small handful of them.
#[derive(Default)]
struct CoverageIndex {
    glyphs: HashMap<char, Option<u16>>,
    scripts: HashMap<ScriptRequirement, bool>,
}

fn registered_family_faces() -> (Vec<FamilyFontRecord>, u64) {
    let (registered_faces, revision) = FontRegistry::faces_with_revision();
    let mut family_faces = Vec::new();
    if let Some(record) = monospace_font_record() {
        family_faces.push(FamilyFontRecord {
            family: FontFamily::MONOSPACE,
            weight: FontWeight::Normal.numeric(),
            style: FontStyle::Normal,
            record,
        });
    }
    family_faces.extend(registered_faces.into_iter().filter_map(|face| {
        Some(FamilyFontRecord {
            family: face.family,
            weight: face.weight,
            style: face.style,
            record: FontRecord::from_shared_bytes(face.face_id, face.bytes)?,
        })
    }));
    (family_faces, revision)
}

fn font_record_changed(previous: &FontRecord, current: &FontRecord) -> bool {
    previous.collection_index != current.collection_index
        || previous.is_color != current.is_color
        || previous.bytes.as_deref() != current.bytes.as_deref()
        || previous.path != current.path
}

fn family_style_distance(requested: FontStyle, candidate: FontStyle) -> u8 {
    if requested == candidate {
        0
    } else if candidate == FontStyle::Normal {
        1
    } else if matches!(requested, FontStyle::Oblique | FontStyle::ObliqueDeg(_))
        && matches!(candidate, FontStyle::Oblique | FontStyle::ObliqueDeg(_))
    {
        2
    } else {
        3
    }
}

impl GlyphRasterizer {
    const BITMAP_CACHE_CAPACITY_BYTES: usize = 8 * 1024 * 1024;
    const COVERAGE_GLYPH_CAPACITY: usize = 16 * 1024;
    const COVERAGE_SCRIPT_CAPACITY: usize = 512;

    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let primary = primary_font_record();
        let (family_faces, registered_font_revision) = registered_family_faces();

        Self {
            primary,
            family_faces,
            registered_font_revision,
            fallbacks: None, // loaded lazily on first miss
            system_fallbacks_loaded: false,
            loaded_fallback_scripts: HashSet::default(),
            registered_fallback_ids: HashSet::default(),
            enable_fallbacks: true,
            cache: HashMap::default(),
            retained_bitmap_bytes: 0,
            advance_cache: HashMap::default(),
            coverage_index_cache: HashMap::default(),
            primary_printable_ascii_coverage: None,
            script_run: ScriptRequirement::EMPTY,
            run_companion: None,
            default_companion_weight: None,
            skip_bundled_cjk: false,
            platform_only_cache: HashMap::default(),
            design_weight_cache: HashMap::default(),
            variable_font_cache: HashMap::default(),
            unsupported_codepoints: HashSet::default(),
            font_bytes_cache: HashMap::default(),
            aimer_font_cache: HashMap::default(),
            run_buffers: RunBuffers::default(),
            resolved_codepoint_cache: HashMap::default(),
            #[cfg(test)]
            shape_call_count: 0,
            #[cfg(test)]
            rasterize_call_count: 0,
            #[cfg(test)]
            simple_ltr_path_count: 0,
            #[cfg(test)]
            reused_shape_output_count: 0,
        }
    }

    /// Create a lightweight rasterizer with only the primary font (no
    /// fallbacks). Suitable for text measurement where CJK rendering is not
    /// needed.
    pub fn primary_only() -> Self {
        let primary = primary_font_record();
        let (family_faces, registered_font_revision) = registered_family_faces();
        Self {
            primary,
            family_faces,
            registered_font_revision,
            fallbacks: None,
            system_fallbacks_loaded: false,
            loaded_fallback_scripts: HashSet::default(),
            registered_fallback_ids: HashSet::default(),
            enable_fallbacks: false,
            cache: HashMap::default(),
            retained_bitmap_bytes: 0,
            advance_cache: HashMap::default(),
            coverage_index_cache: HashMap::default(),
            primary_printable_ascii_coverage: None,
            script_run: ScriptRequirement::EMPTY,
            run_companion: None,
            default_companion_weight: None,
            skip_bundled_cjk: false,
            platform_only_cache: HashMap::default(),
            design_weight_cache: HashMap::default(),
            variable_font_cache: HashMap::default(),
            unsupported_codepoints: HashSet::default(),
            font_bytes_cache: HashMap::default(),
            aimer_font_cache: HashMap::default(),
            run_buffers: RunBuffers::default(),
            resolved_codepoint_cache: HashMap::default(),
            #[cfg(test)]
            shape_call_count: 0,
            #[cfg(test)]
            rasterize_call_count: 0,
            #[cfg(test)]
            simple_ltr_path_count: 0,
            #[cfg(test)]
            reused_shape_output_count: 0,
        }
    }

    /// Captures immutable font ownership for worker-local CPU preparation.
    pub(super) fn font_snapshot(&mut self) -> FontSnapshot {
        self.refresh_registered_family_faces();
        let aimer_faces = self.prewarm_aimer_snapshot_faces();
        FontSnapshot {
            primary: SharedFontRecord::new(&self.primary),
            family_faces: Arc::from(
                self.family_faces
                    .iter()
                    .map(|face| SharedFamilyFontRecord {
                        family: face.family,
                        weight: face.weight,
                        style: face.style,
                        record: SharedFontRecord::new(&face.record),
                    })
                    .collect::<Vec<_>>(),
            ),
            registered_font_revision: self.registered_font_revision,
            fallbacks: self.fallbacks.as_ref().map(|fallbacks| {
                Arc::from(
                    fallbacks
                        .iter()
                        .map(SharedFontRecord::new)
                        .collect::<Vec<_>>(),
                )
            }),
            system_fallbacks_loaded: self.system_fallbacks_loaded,
            loaded_fallback_scripts: Arc::from(
                FallbackScript::ALL
                    .iter()
                    .copied()
                    .filter(|script| self.loaded_fallback_scripts.contains(script))
                    .collect::<Vec<_>>(),
            ),
            registered_fallback_ids: Arc::from({
                let mut ids = self
                    .registered_fallback_ids
                    .iter()
                    .copied()
                    .collect::<Vec<_>>();
                ids.sort_unstable();
                ids
            }),
            enable_fallbacks: self.enable_fallbacks,
            resolved_codepoint_cache: Arc::new(self.resolved_codepoint_cache.clone()),
            default_companion_weight: self.default_companion_weight,
            aimer_faces,
        }
    }

    /// Resolves the fallback faces needed by one complete text span before a
    /// worker batch is launched.
    ///
    /// Worker contexts are deliberately independent for raster-cache safety,
    /// but fallback resolution itself is deterministic and read-mostly. Doing
    /// the face selection once on the owner lets the snapshot carry the
    /// discovered records and the shared Aimer face parses to every worker.
    /// The same grapheme boundaries and run announcement used by shaping are
    /// used here, so this only moves resolution earlier; it does not change
    /// face choice or glyph output.
    pub(super) fn warm_fallbacks_for_text(
        &mut self,
        text: &str,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
        language: Option<TextLanguage>,
    ) {
        if text.is_empty()
            || (family == FontFamily::SANS_SERIF
                && text
                    .bytes()
                    .all(|byte| byte == b'\n' || (b' '..=b'~').contains(&byte))
                && self.primary_covers_printable_ascii())
        {
            return;
        }

        self.begin_script_run(text, language);
        if family == FontFamily::SANS_SERIF {
            // The primary face is the fixed SANS_SERIF choice. Avoid routing
            // every grapheme through the general family selector (which would
            // probe the primary cmap twice) and resolve only the codepoints
            // that can actually enter the fallback chain. The cache key and
            // effective weight are deliberately identical to
            // `glyph_key_for_codepoint_at_weight`, so worker snapshots make
            // the same face choice without re-entering fallback resolution.
            let mut run_weight = None;
            for (_, cluster) in text.grapheme_indices(true) {
                let Some(codepoint) = cluster.chars().find(|codepoint| !codepoint.is_control())
                else {
                    continue;
                };
                if self
                    .glyph_index_for_font(self.primary.id, codepoint)
                    .is_some()
                {
                    continue;
                }
                let resolved_weight = match run_weight {
                    Some(run_weight) => run_weight,
                    None => {
                        let resolved_weight = self.effective_run_weight(weight);
                        run_weight = Some(resolved_weight);
                        resolved_weight
                    }
                };
                let requirement = self.requirement_for(codepoint);
                let cache_key = (codepoint, requirement, resolved_weight);
                if self.resolved_codepoint_cache.contains_key(&cache_key) {
                    continue;
                }
                self.ensure_fallbacks_for_codepoint(codepoint);
                let resolved =
                    self.fallback_glyph_for_codepoint(codepoint, requirement, resolved_weight);
                self.resolved_codepoint_cache.insert(cache_key, resolved);
            }
        } else {
            for (_, cluster) in text.grapheme_indices(true) {
                let _ = self.font_id_for_family_cluster(cluster, font_size, family, weight, style);
            }
        }
        self.end_script_run();
    }

    /// Makes the faces selected by the announced batch ready before workers
    /// start. Unselected fallback faces stay lazy; parsing every installed
    /// script lane here would turn a narrow text update into a broad cold-start
    /// scan and would not improve the resulting pixels.
    fn prewarm_aimer_snapshot_faces(
        &mut self,
    ) -> Arc<
        [(
            FontId,
            crate::text_pipeline::aimer_font::SharedParsedAimerFont,
        )],
    > {
        let mut font_ids =
            Vec::with_capacity(self.resolved_codepoint_cache.len() + self.family_faces.len() + 1);
        font_ids.push(self.primary.id);
        font_ids.extend(self.family_faces.iter().map(|face| face.record.id));
        font_ids.extend(
            self.fallbacks
                .as_ref()
                .into_iter()
                .flatten()
                .map(|record| record.id),
        );
        font_ids.extend(
            self.resolved_codepoint_cache
                .values()
                .map(|(font_id, _, _)| *font_id),
        );
        font_ids.sort_unstable();
        font_ids.dedup();

        let mut faces = Vec::with_capacity(font_ids.len());
        for font_id in font_ids {
            self.ensure_aimer_font_cached(font_id);
            let Some(shared) = self
                .aimer_font_cache
                .get(&font_id)
                .and_then(|state| state.as_ref())
                .and_then(|state| state.prewarm_layout().ok().map(|_| state.shared()))
            else {
                continue;
            };
            faces.push((font_id, shared));
        }
        Arc::from(faces)
    }

    fn from_font_snapshot(snapshot: FontSnapshot) -> Self {
        let aimer_font_cache = snapshot
            .aimer_faces
            .iter()
            .map(|(font_id, shared)| {
                (
                    *font_id,
                    Some(
                        crate::text_pipeline::aimer_font::AimerFontState::from_shared(
                            shared.clone(),
                        ),
                    ),
                )
            })
            .collect();
        Self {
            primary: snapshot.primary.local_copy(),
            family_faces: snapshot
                .family_faces
                .iter()
                .map(|face| FamilyFontRecord {
                    family: face.family,
                    weight: face.weight,
                    style: face.style,
                    record: face.record.local_copy(),
                })
                .collect(),
            registered_font_revision: snapshot.registered_font_revision,
            fallbacks: snapshot
                .fallbacks
                .map(|fallbacks| fallbacks.iter().map(SharedFontRecord::local_copy).collect()),
            system_fallbacks_loaded: snapshot.system_fallbacks_loaded,
            loaded_fallback_scripts: snapshot.loaded_fallback_scripts.iter().copied().collect(),
            registered_fallback_ids: snapshot.registered_fallback_ids.iter().copied().collect(),
            enable_fallbacks: snapshot.enable_fallbacks,
            default_companion_weight: snapshot.default_companion_weight,
            cache: HashMap::default(),
            retained_bitmap_bytes: 0,
            advance_cache: HashMap::default(),
            coverage_index_cache: HashMap::default(),
            primary_printable_ascii_coverage: None,
            script_run: ScriptRequirement::EMPTY,
            run_companion: None,
            skip_bundled_cjk: false,
            platform_only_cache: HashMap::default(),
            design_weight_cache: HashMap::default(),
            variable_font_cache: HashMap::default(),
            unsupported_codepoints: HashSet::default(),
            font_bytes_cache: HashMap::default(),
            aimer_font_cache,
            run_buffers: RunBuffers::default(),
            resolved_codepoint_cache: (*snapshot.resolved_codepoint_cache).clone(),
            #[cfg(test)]
            shape_call_count: 0,
            #[cfg(test)]
            rasterize_call_count: 0,
            #[cfg(test)]
            simple_ltr_path_count: 0,
            #[cfg(test)]
            reused_shape_output_count: 0,
        }
    }

    #[cfg(test)]
    pub fn reset_shape_call_count(&mut self) {
        self.shape_call_count = 0;
    }

    #[cfg(test)]
    pub fn shape_call_count(&self) -> usize {
        self.shape_call_count
    }

    #[cfg(test)]
    pub(super) fn record_simple_ltr_path(&mut self) {
        self.simple_ltr_path_count += 1;
    }

    #[cfg(test)]
    pub(super) fn simple_ltr_path_count(&self) -> usize {
        self.simple_ltr_path_count
    }

    #[cfg(test)]
    pub(super) fn reused_shape_output_count(&self) -> usize {
        self.reused_shape_output_count
    }

    #[cfg(test)]
    pub fn reset_rasterize_call_count(&mut self) {
        self.rasterize_call_count = 0;
    }

    #[cfg(test)]
    pub fn rasterize_call_count(&self) -> usize {
        self.rasterize_call_count
    }

    /// Ensure every platform-independent fallback lane is loaded.
    ///
    /// Ordinary misses use [`Self::ensure_fallbacks_for_script`] instead; this
    /// broad operation remains available for an unknown codepoint and for an
    /// explicit warm-all request.
    fn ensure_fallbacks(&mut self) {
        if self.system_fallbacks_loaded || !self.enable_fallbacks {
            return;
        }
        for script in FallbackScript::ALL {
            self.ensure_fallbacks_for_script(script);
        }
        self.system_fallbacks_loaded = true;
    }

    /// Loads one platform-independent fallback lane without warming unrelated
    /// scripts. Apple still uses its per-codepoint cascade for the actual
    /// record, but tracking the requested lane here keeps snapshot and release
    /// behavior identical across platforms.
    fn ensure_fallbacks_for_script(&mut self, script: FallbackScript) {
        if !self.enable_fallbacks || self.loaded_fallback_scripts.contains(&script) {
            return;
        }
        let chain = shared_fallback_chain_for_script(script);
        let fallbacks = self.fallbacks.get_or_insert_with(Vec::new);
        let mut changed = false;
        for record in chain {
            if !fallbacks.iter().any(|fallback| fallback.id == record.id) {
                fallbacks.push(record);
                changed = true;
            }
        }
        if changed {
            self.resolved_codepoint_cache.clear();
        }
        self.loaded_fallback_scripts.insert(script);
    }

    /// Loads the self-supplied CJK face without paying for unrelated scripts.
    ///
    /// This is deliberately a separate lane from the broad fallback chain:
    /// the checked-in Japanese face is used only for a Japanese run, while
    /// Chinese and Korean runs stay on their own language-aware cascade. Emoji
    /// and less common scripts remain cold until they are asked for. The fixed
    /// id keeps keys stable across worker snapshots and after a local fallback
    /// release.
    fn ensure_bundled_cjk_fallback(&mut self) {
        #[cfg(not(any(feature = "bundled-fonts", test)))]
        {
            return;
        }

        #[cfg(any(feature = "bundled-fonts", test))]
        {
            if !self.enable_fallbacks
                || (self.script_run.language() != Some(TextLanguage::Japanese)
                    && self.run_companion.is_none())
            {
                return;
            }
            let fallbacks = self.fallbacks.get_or_insert_with(Vec::new);
            if fallbacks
                .iter()
                .any(|fallback| fallback.id == BUNDLED_CJK_FONT_ID)
            {
                return;
            }
            if let Some(record) = BUNDLED_CJK_RECORD
                .get_or_init(|| {
                    FontRecord::from_static_bytes(BUNDLED_CJK_FONT_ID, BUNDLED_CJK_FONT)
                })
                .clone()
            {
                fallbacks.push(record);
                self.resolved_codepoint_cache.clear();
            }
        }
    }

    /// Loads the smallest fallback set appropriate for `codepoint`.
    fn ensure_fallbacks_for_codepoint(&mut self, codepoint: char) {
        if let Some(script) = fallback_script_for_codepoint(codepoint) {
            if script == FallbackScript::Cjk
                && (self.script_run.language() == Some(TextLanguage::Japanese)
                    || self.run_companion.is_some())
            {
                self.ensure_bundled_cjk_fallback();
                let bundled_maps = self
                    .glyph_index_for_font(BUNDLED_CJK_FONT_ID, codepoint)
                    .is_some_and(|glyph_id| glyph_id != 0);
                if !bundled_maps {
                    self.ensure_fallbacks_for_script(script);
                }
                return;
            }

            self.ensure_fallbacks_for_script(script);
            return;
        }

        // A codepoint outside the known lanes has no safe narrow candidate;
        // retain the old broad behavior for that uncommon path.
        self.ensure_fallbacks();
    }

    /// Releases discovered fallback faces and the local data derived from
    /// them.
    ///
    /// Explicitly registered faces remain available. Discovered faces use
    /// stable ids, so a later lookup or a worker holding an old [`GlyphKey`]
    /// can load the same face again. Bitmap, metric, cmap and shaping entries
    /// for released faces are invalidated together; retaining any of them
    /// would either keep the face alive or allow a stale cache entry to select
    /// the wrong record after reload.
    pub fn release_fallbacks(&mut self) -> usize {
        let Some(fallbacks) = self.fallbacks.take() else {
            self.system_fallbacks_loaded = false;
            self.loaded_fallback_scripts.clear();
            return 0;
        };

        let mut released_ids: HashSet<FontId> = HashSet::default();
        let retained = fallbacks
            .into_iter()
            .filter_map(|record| {
                if self.registered_fallback_ids.contains(&record.id) {
                    Some(record)
                } else {
                    released_ids.insert(record.id);
                    None
                }
            })
            .collect::<Vec<_>>();

        if released_ids.is_empty() {
            self.fallbacks = Some(retained);
            self.system_fallbacks_loaded = false;
            self.loaded_fallback_scripts.clear();
            return 0;
        }

        self.fallbacks = (!retained.is_empty()).then_some(retained);
        self.system_fallbacks_loaded = false;
        self.loaded_fallback_scripts.clear();
        self.resolved_codepoint_cache.clear();

        self.invalidate_face_caches(&released_ids);

        released_ids.len()
    }

    /// Ensures that a key naming a face from another preparation context can
    /// find that face locally after lazy loading or fallback release.
    fn ensure_fallback_loaded(&mut self, font_id: FontId) {
        if self.font_record_by_id(font_id).is_some() {
            return;
        }
        if font_id == BUNDLED_CJK_FONT_ID {
            self.ensure_bundled_cjk_fallback();
            return;
        }
        if let Some(script) = fallback_script_for_font_id(font_id) {
            self.ensure_fallbacks_for_script(script);
        } else if font_id < SYSTEM_FALLBACK_ID_BASE {
            self.ensure_fallbacks();
        } else {
            self.ensure_system_fallback_loaded(font_id);
        }
    }

    /// Refreshes the deterministic registered-family snapshot after the
    /// process-wide registry changes. Replacements keep their face id but
    /// invalidate every cache derived from the old bytes; removals invalidate
    /// the same data before the old face disappears from the family snapshot.
    fn refresh_registered_family_faces(&mut self) {
        if FontRegistry::revision() == self.registered_font_revision {
            return;
        }
        // A newly registered family face can win a fallback decision even when
        // none of the old face ids disappeared, so selection answers must not
        // survive a registry revision.
        self.resolved_codepoint_cache.clear();
        let (family_faces, revision) = registered_family_faces();
        let changed_ids = self
            .family_faces
            .iter()
            .filter(|face| face.family != FontFamily::MONOSPACE)
            .filter_map(|previous| {
                let current = family_faces
                    .iter()
                    .find(|face| face.record.id == previous.record.id);
                current
                    .is_none_or(|current| font_record_changed(&previous.record, &current.record))
                    .then_some(previous.record.id)
            })
            .collect::<HashSet<_>>();
        self.invalidate_face_caches(&changed_ids);
        self.family_faces = family_faces;
        self.registered_font_revision = revision;
    }

    /// Invalidates all data derived from the supplied face ids.
    ///
    /// The bitmap and metric caches are local to this rasterizer, while the
    /// glyph metrics table is process-wide for worker reuse. They must be
    /// invalidated together: keeping either one would allow an old face to
    /// answer after a registration was replaced or removed.
    fn invalidate_face_caches(&mut self, invalidated_ids: &HashSet<FontId>) {
        if invalidated_ids.is_empty() {
            return;
        }

        let invalidated_bitmap_bytes = self
            .cache
            .iter()
            .filter(|(key, _)| invalidated_ids.contains(&key.font_id))
            .map(|(_, glyph)| glyph.bitmap.capacity())
            .sum::<usize>();
        self.cache
            .retain(|key, _| !invalidated_ids.contains(&key.font_id));
        self.retained_bitmap_bytes = self
            .retained_bitmap_bytes
            .saturating_sub(invalidated_bitmap_bytes);
        self.advance_cache
            .retain(|key, _| !invalidated_ids.contains(&key.font_id));
        self.coverage_index_cache
            .retain(|font_id, _| !invalidated_ids.contains(font_id));
        self.platform_only_cache
            .retain(|font_id, _| !invalidated_ids.contains(font_id));
        self.design_weight_cache
            .retain(|font_id, _| !invalidated_ids.contains(font_id));
        self.variable_font_cache
            .retain(|font_id, _| !invalidated_ids.contains(font_id));
        self.font_bytes_cache
            .retain(|font_id, _| !invalidated_ids.contains(font_id));
        self.aimer_font_cache
            .retain(|font_id, _| !invalidated_ids.contains(font_id));
        self.unsupported_codepoints.clear();
        self.default_companion_weight = None;
        self.resolved_codepoint_cache.clear();

        for font_id in invalidated_ids.iter().copied() {
            glyph_metrics::forget_font(font_id);
        }
    }

    pub fn primary_font_id(&self) -> FontId {
        self.primary.id
    }

    pub fn font_id_for_family(
        &mut self,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> FontId {
        self.refresh_registered_family_faces();
        self.family_record(family, weight, style)
            .map_or(self.primary.id, |record| record.id)
    }

    pub fn glyph_key_for_family_codepoint(
        &mut self,
        codepoint: char,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> GlyphKey {
        #[cfg(test)]
        KEY_LOOKUPS.with(|count| count.set(count.get() + 1));
        self.refresh_registered_family_faces();
        if let Some(font_id) = self
            .family_record(family, weight, style)
            .map(|record| record.id)
            && let Some(glyph_id) = self.glyph_index_for_font(font_id, codepoint)
        {
            let glyph_weight = self.glyph_weight_for_request(font_id, weight);
            return GlyphKey::new(font_id, glyph_id, font_size).weighted(glyph_weight);
        }

        self.glyph_key_for_codepoint_at_weight(codepoint, font_size, weight)
    }

    /// Returns a family-resolved glyph key at arbitrary OpenType variation
    /// axes. The key carries a face-local coordinate identity when the
    /// selected face is an Aimer-readable variable font.
    pub fn glyph_key_for_family_codepoint_with_variations(
        &mut self,
        codepoint: char,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
        axes: &[(u32, f32)],
    ) -> GlyphKey {
        let key = self.glyph_key_for_family_codepoint(codepoint, font_size, family, weight, style);
        let variation_id = self.variation_id_for_font(key.font_id, key.weight, axes);
        key.with_variation_id(variation_id)
    }

    pub fn advance_width_for_family(
        &mut self,
        codepoint: char,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> f32 {
        let key = self.glyph_key_for_family_codepoint(codepoint, font_size, family, weight, style);
        if let Some(width) = self.advance_cache.get(&key) {
            return *width;
        }
        let width = self
            .aimer_advance_width_for_key(key, font_size)
            .unwrap_or_else(|| {
                self.select_font_for_key(key)
                    .advance_width_for_glyph(key.glyph_id, font_size)
                    .unwrap_or(0.0)
            });
        self.advance_cache.insert(key, width);
        width
    }

    pub fn measure_text_for_family(
        &mut self,
        text: &str,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
        language: Option<TextLanguage>,
    ) -> f32 {
        // Measuring must choose the same faces the shaping pass will, or the
        // width reported for a mixed CJK line belongs to a font nothing draws.
        // The language travels with it for the same reason: a run drawn in a
        // Chinese face may not be measured in a Japanese one.
        self.begin_script_run(text, language);
        let mut advances = RunAdvances::new();
        let width = text
            .chars()
            .map(|codepoint| advances.advance(self, codepoint, font_size, family, weight, style))
            .sum();
        self.end_script_run();
        width
    }

    fn family_record(
        &self,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> Option<&FontRecord> {
        if family == FontFamily::SANS_SERIF {
            return Some(&self.primary);
        }
        let numeric_weight = weight.numeric();
        self.family_faces
            .iter()
            .filter(|face| face.family == family)
            .min_by_key(|face| {
                (
                    family_style_distance(style, face.style),
                    face.weight.abs_diff(numeric_weight),
                    face.weight,
                    face.record.id,
                )
            })
            .map(|face| &face.record)
    }

    pub fn register_font_bytes(&mut self, bytes: Vec<u8>) -> Option<FontId> {
        let font_id = self.next_fallback_font_id();
        let record = FontRecord::from_bytes(font_id, bytes)?;
        self.fallbacks.get_or_insert_with(Vec::new).push(record);
        self.registered_fallback_ids.insert(font_id);
        self.unsupported_codepoints.clear();
        self.cache.clear();
        self.retained_bitmap_bytes = 0;
        self.advance_cache.clear();
        self.coverage_index_cache.clear();
        self.resolved_codepoint_cache.clear();
        self.platform_only_cache.clear();
        self.design_weight_cache.clear();
        glyph_metrics::forget_font(font_id);
        self.font_bytes_cache.remove(&font_id);
        self.aimer_font_cache.remove(&font_id);
        Some(font_id)
    }

    /// Next id available for a font registered at runtime.
    ///
    /// Ids at or above [`SYSTEM_FALLBACK_ID_BASE`] belong to faces discovered
    /// on demand and are owned by the shared store, so they are skipped here.
    /// The platform-independent lanes also own a reserved high range below
    /// that boundary; skipping it keeps a runtime registration from colliding
    /// with a lane that has not been loaded yet.
    fn next_fallback_font_id(&self) -> FontId {
        self.fallbacks
            .as_ref()
            .into_iter()
            .flatten()
            .map(|record| record.id)
            .filter(|id| {
                *id < SYSTEM_FALLBACK_ID_BASE
                    && fallback_script_for_font_id(*id).is_none()
                    && *id != BUNDLED_CJK_FONT_ID
            })
            .chain(
                (self.primary.id < SYSTEM_FALLBACK_ID_BASE
                    && fallback_script_for_font_id(self.primary.id).is_none()
                    && self.primary.id != BUNDLED_CJK_FONT_ID)
                    .then_some(self.primary.id),
            )
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub fn glyph_key_for_codepoint(&mut self, codepoint: char, font_size: f32) -> GlyphKey {
        self.glyph_key_for_codepoint_at_weight(codepoint, font_size, FontWeight::Normal)
    }

    /// Returns a glyph key for `codepoint` at arbitrary OpenType variation
    /// axes. Axis tags are packed big-endian four-byte tags such as
    /// `u32::from_be_bytes(*b"wdth")`; the requested `FontWeight` remains the
    /// `wght` value unless the axis list explicitly includes `wght`.
    ///
    /// The axis values are retained by the shared Aimer face and only a
    /// compact face-local id is placed in the key. Invalid or unsupported
    /// requests use the ordinary weight-only key.
    pub fn glyph_key_for_codepoint_with_variations(
        &mut self,
        codepoint: char,
        font_size: f32,
        weight: FontWeight,
        axes: &[(u32, f32)],
    ) -> GlyphKey {
        let key = self.glyph_key_for_codepoint_at_weight(codepoint, font_size, weight);
        let variation_id = self.variation_id_for_font(key.font_id, key.weight, axes);
        key.with_variation_id(variation_id)
    }

    /// The key for `codepoint` when the text around it asks for `weight`.
    ///
    /// The style's weight already chose among the registered faces before the
    /// fallback chain is consulted, so static faces Cupid decodes itself stay
    /// on [`NORMAL_GLYPH_WEIGHT`]. Readable variable faces and faces only the
    /// platform draws are addressed at the run's effective weight instead —
    /// see [`Self::glyph_weight_for_request`].
    fn glyph_key_for_codepoint_at_weight(
        &mut self,
        codepoint: char,
        font_size: f32,
        weight: FontWeight,
    ) -> GlyphKey {
        let primary_glyph_id = self.glyph_index_for_font(self.primary.id, codepoint);

        // The primary face answers before any weight is computed: it is one
        // design serving every style, and asking what weight the run wants
        // would resolve the companion face — a fallback lookup — for text
        // that needs no fallback at all.
        if let Some(glyph_id) = primary_glyph_id {
            return GlyphKey::new(self.primary.id, glyph_id, font_size);
        }

        // The face is chosen at the weight the run wants its faces designed
        // at, so an emphasized line lands on the family's bold cut instead of
        // its regular one — see [`Self::effective_run_weight`].
        let run_weight = self.effective_run_weight(weight);
        let cache_key = (codepoint, self.requirement_for(codepoint), run_weight);
        let resolved = self.resolved_codepoint_cache.get(&cache_key).copied();
        let (font_id, glyph_id, supported) = if let Some(resolved) = resolved {
            // The snapshot already carries every face named by this map. Do
            // not reload a fallback lane or re-enter the platform resolver on
            // a worker cache hit; that was the work the owner performed while
            // announcing the batch.
            resolved
        } else {
            self.ensure_fallbacks_for_codepoint(codepoint);
            let resolved = self.font_and_glyph_for_codepoint(codepoint, run_weight);
            self.resolved_codepoint_cache.insert(cache_key, resolved);
            resolved
        };
        if !supported {
            self.unsupported_codepoints.insert(codepoint);
        }
        let glyph_weight = self.glyph_weight_for_resolved_face(font_id, weight, run_weight);
        GlyphKey::new(font_id, glyph_id, font_size).weighted(glyph_weight)
    }

    /// Reports whether `key`'s bitmap still has to be emboldened by hand to
    /// read as text drawn at `requested`.
    ///
    /// Cupid answers a bold style with a bold *face* wherever one exists, and
    /// falls back to drawing the glyph twice a fraction of an em apart when it
    /// does not — a face carrying a single design has no other bold to give.
    /// The choice is per glyph rather than per span because a single line
    /// reaches its stroke by both routes at once: on iOS a Chinese ideograph
    /// is drawn by the platform at the run's bold instance while the
    /// characters beside it come from a face Cupid decodes, and emboldening
    /// the whole line by hand left the ideograph bolder than its neighbours —
    /// invisible at the regular weight, where nothing is drawn twice, and
    /// plain the moment the text turned bold.
    ///
    /// A glyph counts as already emphasized when the weight it is *drawn* at
    /// — the instance a platform face is rendered at, otherwise the design its
    /// face was cut at — reaches `requested` within
    /// [`WEIGHT_MATCH_TOLERANCE`], since families ship discrete cuts and a
    /// request for `700` is commonly answered by a `600` semibold.
    pub fn glyph_needs_synthetic_bold(&mut self, key: GlyphKey, requested: u16) -> bool {
        requested >= BOLD_WEIGHT_THRESHOLD
            && synthetic_weight_needed(requested, self.drawn_weight(key))
    }

    /// Returns the synthetic-stroke offset needed for `key` to reach the
    /// requested weight, including regular fallback text that is one design
    /// cut lighter than the surrounding face.
    pub(crate) fn synthetic_weight_offset(
        &mut self,
        key: GlyphKey,
        requested: u16,
        font_size: f32,
    ) -> Option<f32> {
        synthetic_weight_offset_for(font_size, requested, self.drawn_weight(key))
    }

    /// Returns the synthetic-stroke offset for a positioned glyph when its
    /// face is designed lighter than the requested weight.
    #[cfg(test)]
    pub(crate) fn synthetic_weight_offset_for_codepoint(
        &mut self,
        key: GlyphKey,
        requested: u16,
        font_size: f32,
        codepoint: char,
    ) -> Option<f32> {
        self.synthetic_weight_plan_for_codepoint(key, requested, font_size, codepoint)
            .map(|plan| plan.span())
    }

    /// Returns the additional positioned copies needed to normalize a glyph's
    /// weight. Synthetic emphasis uses one copy shifted to the right.
    pub(crate) fn synthetic_weight_plan_for_codepoint(
        &mut self,
        key: GlyphKey,
        requested: u16,
        font_size: f32,
        _codepoint: char,
    ) -> Option<SyntheticWeightPlan> {
        self.synthetic_weight_offset(key, requested, font_size)
            .map(|offset| SyntheticWeightPlan {
                extra_offsets: [offset, 0.0],
                extra_count: 1,
            })
    }

    /// The OpenType weight `key`'s bitmap is actually drawn at.
    ///
    /// A variable face is rendered at the instance the key names; a static
    /// face renders the one design it was cut at, whatever weight asked for
    /// it.
    fn drawn_weight(&mut self, key: GlyphKey) -> u16 {
        if self.face_needs_platform_raster(key.font_id) {
            key.weight
        } else if self.face_has_weight_variations(key.font_id) {
            // The owned variation path renders a readable face at the weight
            // carried by the key. Its OS/2 class may describe the font's
            // default instance (the bundled Noto face advertises W100 while
            // its regular instance is W400), so that class is not the weight
            // visible in the bitmap.
            key.weight
        } else {
            self.face_design_weight(key.font_id)
                .unwrap_or(NORMAL_GLYPH_WEIGHT)
        }
    }

    pub fn font_id_for_codepoint(&mut self, codepoint: char) -> FontId {
        if self
            .glyph_index_for_font(self.primary.id, codepoint)
            .is_none()
            && !self.unsupported_codepoints.contains(&codepoint)
        {
            self.ensure_fallbacks_for_codepoint(codepoint);
        }

        let (font_id, _, supported) =
            self.font_and_glyph_for_codepoint(codepoint, NORMAL_GLYPH_WEIGHT);
        if !supported {
            self.unsupported_codepoints.insert(codepoint);
        }
        font_id
    }

    /// The face and glyph drawing `codepoint` for a run designed at `weight`.
    ///
    /// `weight` is the OpenType `wght` the run's faces should be *designed*
    /// at, not the style's raw weight: it already carries the companion
    /// baseline — see [`Self::effective_run_weight`].
    fn font_and_glyph_for_codepoint(
        &mut self,
        codepoint: char,
        weight: u16,
    ) -> (FontId, u16, bool) {
        if let Some(glyph_id) = self.glyph_index_for_font(self.primary.id, codepoint) {
            return (self.primary.id, glyph_id, true);
        }

        // Han is unified, so a Japanese face carries the characters Japanese
        // shares with Chinese and none of the rest. Loaded first for the kana
        // it was chosen for, it sits at the head of the chain and would claim
        // exactly that half, leaving the simplified-only characters beside it
        // to another face — one word drawn in two typefaces. A face is
        // therefore accepted only when it covers every character of the script
        // standing beside this one.
        //
        // A chain face is likewise only taken outright when it is designed
        // near the weight the run asked for: the regular cut adopted for an
        // earlier line must not shadow the bold one the platform would name,
        // which is what left a bold `你好` drawn at the regular stroke while
        // the `吗` beside it — drawn by the platform at the run's weight —
        // came out bold.
        let requirement = self.requirement_for(codepoint);
        self.fallback_glyph_for_codepoint(codepoint, requirement, weight)
    }

    /// Resolves a codepoint already known to miss the primary face. Keeping
    /// the primary probe outside this helper lets the batch fallback warm pass
    /// reuse its result and its already-computed script requirement.
    fn fallback_glyph_for_codepoint(
        &mut self,
        codepoint: char,
        requirement: ScriptRequirement,
        weight: u16,
    ) -> (FontId, u16, bool) {
        let chain = self.chain_glyph_for_codepoint(codepoint, requirement, weight);
        if let Some((font_id, glyph_id, true)) = chain {
            return (font_id, glyph_id, true);
        }
        if let Some((font_id, glyph_id)) =
            self.resolve_system_fallback(codepoint, requirement, weight)
            && (chain.is_none() || self.face_matches_weight(font_id, weight))
        {
            return (font_id, glyph_id, true);
        }
        // Nothing is designed at the requested weight after all: the face the
        // chain already draws the run with keeps the line in one typeface,
        // which matters more than a stroke no installed face offers.
        if let Some((font_id, glyph_id, _)) = chain {
            return (font_id, glyph_id, true);
        }
        // No installed face covers the script whole — a system carrying only a
        // partial one. Drawing the character in a narrow face still beats a
        // blank box.
        if !requirement.is_empty()
            && let Some((font_id, glyph_id, _)) =
                self.chain_glyph_for_codepoint(codepoint, ScriptRequirement::EMPTY, weight)
        {
            return (font_id, glyph_id, true);
        }
        (self.primary.id, 0, false)
    }

    /// Announces the text about to be shaped or measured.
    ///
    /// Face selection is per codepoint, but the *right* face for an ideograph
    /// depends on the characters around it: `時` in `あの時は` belongs to a
    /// Japanese word and must keep the face its kana use, while `好` in `你好吗`
    /// must come from the same Chinese face as its neighbours. Passes holding
    /// the whole string say so here, and every lookup until
    /// [`Self::end_script_run`] is judged against it.
    ///
    /// `language` is what the characters themselves cannot say: Han is
    /// unified, so a run of ideographs alone leaves Chinese and Japanese faces
    /// equally entitled to it, and the run silently changes typeface the
    /// moment a character only one of them carries is typed. Callers who know
    /// the language — a text field knows the keyboard it is being typed on —
    /// pass it here; see [`ScriptRequirement::from_run`] for how far it
    /// reaches.
    pub fn begin_script_run(&mut self, text: &str, language: Option<TextLanguage>) {
        self.script_run = ScriptRequirement::from_run(text, language);
        self.run_companion = text.chars().find(|codepoint| is_kana(*codepoint));
    }

    /// Forgets the run announced by [`Self::begin_script_run`].
    ///
    /// Lookups fall back to the script's fixed samples, which is all a caller
    /// holding a single character can be judged against.
    pub fn end_script_run(&mut self) {
        self.script_run = ScriptRequirement::EMPTY;
        self.run_companion = None;
    }

    /// The OpenType weight glyphs of `font_id` are addressed and drawn at when
    /// the text around them asks for `requested`.
    ///
    /// Static faces Cupid decodes itself always answer
    /// [`NORMAL_GLYPH_WEIGHT`]: they render their single design regardless,
    /// and keeping their keys on one value means one bitmap serves every
    /// style. A readable variable face and a face only the platform can draw
    /// must instead be told which instance to render:
    ///
    /// * the platform-only baseline is the run's *companion weight* — the design weight of
    ///   the face drawing the run's kana. Apple pairs its Japanese UI faces at
    ///   `W3` (`300` on the `wght` scale) with regular text, so an ideograph
    ///   pinned to the default `400` stands visibly bolder than the かな beside
    ///   it — the "PingFang is still bolder" defect on iOS, where no readable
    ///   Chinese face exists and the platform draws all of Han;
    /// * the style's distance from normal is carried on top, so bold text
    ///   gets a bold instance: a bold run beside `W3` kana asks for
    ///   `300 + (700 - 400) = 600`, which is exactly the `W6`/`Semibold`
    ///   pairing Apple ships.
    ///
    /// Runs without kana take the same baseline from the face kana *would*
    /// resolve to — see [`Self::default_companion_weight`] — so an ideograph
    /// keeps one stroke whether kana stand beside it yet or not, instead of
    /// snapping from bold to thin the moment one is typed.
    fn glyph_weight_for_request(&mut self, font_id: FontId, requested: FontWeight) -> u16 {
        if self.face_needs_platform_raster(font_id) {
            return self.effective_run_weight(requested);
        }
        if self.face_has_weight_variations(font_id) {
            return requested.numeric();
        }
        NORMAL_GLYPH_WEIGHT
    }

    /// Converts a resolved fallback answer into the key weight for one face.
    /// The fallback search uses the effective run weight to choose a matching
    /// face, but a readable variable face receives the caller's standard
    /// OpenType request directly. Only platform-only faces need the companion
    /// adjustment used by Apple's private variable fonts.
    fn glyph_weight_for_resolved_face(
        &mut self,
        font_id: FontId,
        requested: FontWeight,
        run_weight: u16,
    ) -> u16 {
        if self.face_needs_platform_raster(font_id) {
            run_weight
        } else if self.face_has_weight_variations(font_id) {
            requested.numeric()
        } else {
            NORMAL_GLYPH_WEIGHT
        }
    }

    /// The `wght` value the run's faces should be designed and drawn at.
    ///
    /// This is the companion baseline of [`Self::run_companion_weight`] with
    /// the style's distance from normal carried on top, and it answers two
    /// questions with one number: which instance a variable platform face is
    /// rendered at, and which cut of a family the fallback resolver picks for
    /// the characters Cupid draws itself. Answering them separately is what
    /// made a bold `你好吗` arrive in two strokes — the platform drew `吗` at
    /// the bold instance while `你好` stayed on whatever regular face the
    /// cascade had named.
    fn effective_run_weight(&mut self, requested: FontWeight) -> u16 {
        let baseline = self
            .run_companion_weight()
            .or_else(|| {
                (!self.script_run.is_empty())
                    .then(|| self.default_companion_weight())
                    .flatten()
            })
            .unwrap_or(NORMAL_GLYPH_WEIGHT);
        let offset = i32::from(requested.numeric()) - i32::from(NORMAL_GLYPH_WEIGHT);
        (i32::from(baseline) + offset).clamp(1, 1000) as u16
    }

    /// Reports whether only the platform can rasterize `font_id`'s glyphs.
    ///
    /// True for faces carrying neither readable outlines (`glyf`, `CFF `,
    /// `CFF2`) nor color strikes — Apple's private `hvgl` faces. The answer is
    /// a property of the face, so it is cached per font id.
    fn face_needs_platform_raster(&mut self, font_id: FontId) -> bool {
        if let Some(known) = self.platform_only_cache.get(&font_id) {
            return *known;
        }
        let needs = self
            .font_record_by_id(font_id)
            .cloned()
            .is_some_and(|record| record_outlines_unreadable(&record));
        self.platform_only_cache.insert(font_id, needs);
        needs
    }

    /// The design weight of the face drawing the announced run's kana.
    ///
    /// `None` when the run has no kana, or when the kana's face hides its
    /// `OS/2` table.
    fn run_companion_weight(&mut self) -> Option<u16> {
        let companion = self.run_companion?;
        let font_id = self.font_id_for_codepoint(companion);
        self.face_design_weight(font_id)
    }

    /// The design weight of the face kana resolve to on this system.
    ///
    /// This is the companion baseline of runs carrying no kana of their own:
    /// an ideograph typed alone must land on the weight it will keep once
    /// kana appear beside it, or the line visibly changes stroke mid-typing.
    /// The probe is judged as kana text rather than against the announced
    /// run, whose requirement would send it to a face chosen for another
    /// script. The answer is a property of the installed font set, so it is
    /// resolved once per rasterizer; `None` when no face covers kana or the
    /// covering face hides its `OS/2` table.
    fn default_companion_weight(&mut self) -> Option<u16> {
        // With neither a chain nor the right to build one there is no
        // companion to pair with: every glyph comes from the primary face, and
        // probing for kana would only cost a lookup whose answer nothing
        // reads. Faces handed to the rasterizer directly are a chain all the
        // same, so on-demand resolution being off does not by itself leave the
        // run without a companion.
        if !self.enable_fallbacks && self.fallbacks.as_ref().is_none_or(Vec::is_empty) {
            return None;
        }
        if let Some(known) = self.default_companion_weight {
            return known;
        }
        let run = std::mem::replace(&mut self.script_run, ScriptRequirement::EMPTY);
        let previous_skip_bundled_cjk = self.skip_bundled_cjk;
        self.skip_bundled_cjk = true;
        let font_id = self.font_id_for_codepoint(COMPANION_WEIGHT_PROBE);
        self.skip_bundled_cjk = previous_skip_bundled_cjk;
        self.script_run = run;
        let weight = self.face_design_weight(font_id);
        self.default_companion_weight = Some(weight);
        weight
    }

    /// The `OS/2` weight class `font_id` was designed at, cached per face.
    fn face_design_weight(&mut self, font_id: FontId) -> Option<u16> {
        if let Some(known) = self.design_weight_cache.get(&font_id) {
            return *known;
        }
        let shared_weight = {
            self.ensure_aimer_font_cached(font_id);
            self.aimer_font_cache
                .get(&font_id)
                .and_then(|state| state.as_ref())
                .and_then(crate::text_pipeline::aimer_font::AimerFontState::design_weight)
        };
        let weight = shared_weight.or_else(|| {
            self.font_record_by_id(font_id)
                .cloned()
                .and_then(|record| record.design_weight())
        });
        self.design_weight_cache.insert(font_id, weight);
        weight
    }

    /// Returns whether a readable face can be rendered through Aimer's
    /// `fvar`/`gvar` `wght`-instance path.
    fn face_has_weight_variations(&mut self, font_id: FontId) -> bool {
        if let Some(known) = self.variable_font_cache.get(&font_id) {
            return *known;
        }
        self.ensure_aimer_font_cached(font_id);
        let variable = self
            .aimer_font_cache
            .get(&font_id)
            .and_then(|state| state.as_ref())
            .is_some_and(crate::text_pipeline::aimer_font::AimerFontState::has_weight_variations);
        self.variable_font_cache.insert(font_id, variable);
        variable
    }

    /// What a face must cover to be accepted for `codepoint`.
    fn requirement_for(&self, codepoint: char) -> ScriptRequirement {
        if script_probes(codepoint).is_empty() {
            ScriptRequirement::EMPTY
        } else if self.script_run.is_empty() {
            ScriptRequirement::probes(codepoint)
        } else {
            self.script_run
        }
    }

    /// Finds `codepoint` in the faces already adopted by this rasterizer.
    ///
    /// Faces are tried in adoption order, and one is accepted only when it also
    /// draws every character of `requirement`, which is what keeps a face
    /// covering half of a unified script from claiming it. An empty requirement
    /// accepts the first face mapping the codepoint.
    ///
    /// Adoption order decides among the faces that qualify, so the first one
    /// designed near `weight` wins rather than the nearest one anywhere in the
    /// chain: a face adopted for this very run must not lose the run to a
    /// stranger whose `OS/2` weight happens to match more exactly. The third
    /// element of the answer reports whether the face returned is designed
    /// near `weight` at all; when it is not, the caller asks the platform for
    /// a better cut before settling for it.
    fn chain_glyph_for_codepoint(
        &mut self,
        codepoint: char,
        requirement: ScriptRequirement,
        weight: u16,
    ) -> Option<(FontId, u16, bool)> {
        let fallback_count = self.fallbacks.as_ref().map_or(0, Vec::len);
        let mut first: Option<(FontId, u16)> = None;
        for index in 0..fallback_count {
            let Some(font_id) = self
                .fallbacks
                .as_ref()
                .and_then(|fallbacks| fallbacks.get(index))
                .map(|record| record.id)
            else {
                break;
            };
            if font_id == BUNDLED_CJK_FONT_ID
                && (self.skip_bundled_cjk
                    || (!script_probes(codepoint).is_empty()
                        && self.script_run.language() != Some(TextLanguage::Japanese)
                        && self.run_companion.is_none()))
            {
                // Noto Sans JP is the deterministic Japanese face we ship;
                // an all-Han run without a Japanese language signal is
                // ambiguous, so let the language-aware system cascade choose
                // Chinese/Korean rather than silently applying Japanese glyph
                // forms.
                continue;
            }
            let Some(glyph_id) = self.glyph_index_for_font(font_id, codepoint) else {
                continue;
            };
            if !self.font_covers_script(font_id, requirement) {
                continue;
            }
            if self.face_matches_weight(font_id, weight) {
                return Some((font_id, glyph_id, true));
            }
            first.get_or_insert((font_id, glyph_id));
        }
        first.map(|(font_id, glyph_id)| (font_id, glyph_id, false))
    }

    /// Reports whether `font_id` can serve a run asking for `weight`.
    ///
    /// Families ship discrete cuts, so the test is proximity rather than
    /// equality — see [`WEIGHT_MATCH_TOLERANCE`]. A face hiding its `OS/2`
    /// table is read as regular, the weight it is drawn at everywhere else.
    ///
    /// A face only the platform can draw is exempt: it is variable — that is
    /// why the owned outline decoder does not read it — and is *rendered* at the
    /// instance its key names, so it serves every weight and none of them is
    /// its design. Judging it by an `OS/2` weight it never publishes read it as
    /// regular and refused it a bold run, which is what left a bold `你好`
    /// drawn at the light stroke of whatever decodable face happened to cover
    /// it while `你好吗` — where the simplified-only `吗` leaves no such face —
    /// arrived properly bold.
    fn face_matches_weight(&mut self, font_id: FontId, weight: u16) -> bool {
        if self.face_has_weight_variations(font_id) {
            // Aimer can select the requested `wght` instance for a readable
            // variable face, so its OS/2 default class must not disqualify it.
            return true;
        }
        if self.face_needs_platform_raster(font_id) {
            return true;
        }
        self.face_design_weight(font_id)
            .unwrap_or(NORMAL_GLYPH_WEIGHT)
            .abs_diff(weight)
            <= WEIGHT_MATCH_TOLERANCE
    }

    /// Reports whether `font_id` draws every character `requirement` demands.
    ///
    /// The answer is cached per face and requirement, so a paragraph of Han
    /// costs one coverage pass per face rather than one per character.
    fn font_covers_script(&mut self, font_id: FontId, requirement: ScriptRequirement) -> bool {
        if requirement.is_empty() {
            return true;
        }
        if let Some(covered) = self
            .coverage_index_cache
            .get(&font_id)
            .and_then(|index| index.scripts.get(&requirement))
        {
            return *covered;
        }
        let covered = requirement.as_slice().iter().all(|probe| {
            self.glyph_index_for_font(font_id, *probe)
                .is_some_and(|glyph_id| glyph_id != 0)
        });
        let index = self.coverage_index_cache.entry(font_id).or_default();
        if index.scripts.len() >= Self::COVERAGE_SCRIPT_CAPACITY {
            index.scripts.clear();
        }
        index.scripts.insert(requirement, covered);
        covered
    }

    /// Asks the platform for a face covering `codepoint` and adopts it.
    ///
    /// This runs only after every loaded face has been tried, which keeps the
    /// cost off the common path: the answer is cached process wide, so a
    /// codepoint is queried once per process no matter how many rasterizers
    /// encounter it.
    fn resolve_system_fallback(
        &mut self,
        codepoint: char,
        requirement: ScriptRequirement,
        weight: u16,
    ) -> Option<(FontId, u16)> {
        if !self.enable_fallbacks || self.unsupported_codepoints.contains(&codepoint) {
            return None;
        }
        let (record, glyph_id) = fallback_glyph_for_codepoint(codepoint, requirement, weight)?;
        let font_id = record.id;
        self.adopt_fallback(record);
        Some((font_id, glyph_id))
    }

    /// Adds a face to this rasterizer's chain unless its id is already there.
    fn adopt_fallback(&mut self, record: FontRecord) {
        let fallbacks = self.fallbacks.get_or_insert_with(Vec::new);
        if !fallbacks.iter().any(|fallback| fallback.id == record.id) {
            fallbacks.push(record);
            // Adoption order is part of fallback selection. A new face can
            // therefore change the answer for an earlier codepoint even when
            // that codepoint was already cached locally.
            self.resolved_codepoint_cache.clear();
        }
    }

    /// Adopts the on-demand face named by `font_id`, if this is a foreign id.
    ///
    /// Shaping, layout and rasterization run in separate rasterizers, so a key
    /// can name a face this instance never resolved itself. The shared store
    /// keeps ids stable process wide, which makes recovering the face a plain
    /// lookup.
    fn ensure_system_fallback_loaded(&mut self, font_id: FontId) {
        if font_id < SYSTEM_FALLBACK_ID_BASE || self.font_record_by_id(font_id).is_some() {
            return;
        }
        if let Some(record) = fallback_by_id(font_id) {
            self.adopt_fallback(record);
        }
    }

    fn glyph_index_for_font(&mut self, font_id: FontId, codepoint: char) -> Option<u16> {
        if let Some(glyph_id) = self
            .coverage_index_cache
            .get(&font_id)
            .and_then(|index| index.glyphs.get(&codepoint))
        {
            return *glyph_id;
        }

        let glyph_id = {
            self.ensure_aimer_font_cached(font_id);
            self.aimer_font_cache
                .get(&font_id)
                .and_then(|state| state.as_ref())
                .and_then(|state| state.glyph_index(codepoint))
        };
        let index = self.coverage_index_cache.entry(font_id).or_default();
        if index.glyphs.len() >= Self::COVERAGE_GLYPH_CAPACITY {
            index.glyphs.clear();
        }
        index.glyphs.insert(codepoint, glyph_id);
        glyph_id
    }

    /// Reports whether the primary face covers every printable ASCII scalar.
    ///
    /// This is cached after the first query because the primary face is fixed
    /// for the lifetime of a rasterizer. A run containing only printable ASCII
    /// and hard breaks can then skip per-cluster family resolution safely.
    pub(super) fn primary_covers_printable_ascii(&mut self) -> bool {
        if let Some(covered) = self.primary_printable_ascii_coverage {
            return covered;
        }
        let covered = *PRIMARY_PRINTABLE_ASCII_COVERAGE.get_or_init(|| {
            (b' '..=b'~').all(|codepoint| {
                self.glyph_index_for_font(self.primary.id, char::from(codepoint))
                    .is_some()
            })
        });
        self.primary_printable_ascii_coverage = Some(covered);
        covered
    }

    fn font_record_by_id(&self, font_id: FontId) -> Option<&FontRecord> {
        if font_id == self.primary.id {
            return Some(&self.primary);
        }
        if let Some(face) = self
            .family_faces
            .iter()
            .find(|face| face.record.id == font_id)
        {
            return Some(&face.record);
        }
        self.fallbacks
            .as_ref()?
            .iter()
            .find(|record| record.id == font_id)
    }

    fn ensure_font_data_cached(&mut self, font_id: FontId) {
        if self.font_bytes_cache.contains_key(&font_id) {
            return;
        }
        let data = self.font_record_by_id(font_id).and_then(FontRecord::data);
        if let Some(data) = data {
            self.font_bytes_cache.insert(font_id, data);
        }
    }

    fn ensure_aimer_font_cached(&mut self, font_id: FontId) {
        if self.aimer_font_cache.contains_key(&font_id) {
            return;
        }
        self.ensure_font_data_cached(font_id);
        let state = self
            .font_bytes_cache
            .get(&font_id)
            .cloned()
            .and_then(|data| {
                let collection_index = self.collection_index_for_font_id(font_id);
                if font_id == self.primary.id {
                    crate::text_pipeline::aimer_font::primary_state(data, collection_index)
                } else {
                    crate::text_pipeline::aimer_font::AimerFontState::from_font_data(
                        data,
                        collection_index,
                    )
                    .ok()
                }
            });
        self.aimer_font_cache.insert(font_id, state);
    }

    fn variation_id_for_font(&mut self, font_id: FontId, weight: u16, axes: &[(u32, f32)]) -> u32 {
        if axes.is_empty() {
            return 0;
        }
        self.ensure_aimer_font_cached(font_id);
        self.aimer_font_cache
            .get(&font_id)
            .and_then(|state| state.as_ref())
            .and_then(|state| state.variation_instance_for_axes(weight, axes))
            .unwrap_or(0)
    }

    fn aimer_advance_width_for_key(&mut self, key: GlyphKey, font_size: f32) -> Option<f32> {
        if key.variation_id == 0 && !self.face_has_weight_variations(key.font_id) {
            return None;
        }
        self.ensure_aimer_font_cached(key.font_id);
        self.aimer_font_cache
            .get(&key.font_id)
            .and_then(|state| state.as_ref())
            .filter(|state| key.variation_id != 0 || state.has_weight_variations())
            .and_then(|state| {
                state.advance_width_for_glyph_at_variation(
                    key.glyph_id,
                    font_size,
                    key.weight,
                    key.variation_id,
                )
            })
    }

    fn collection_index_for_font_id(&self, font_id: FontId) -> u32 {
        self.font_record_by_id(font_id)
            .map(|record| record.collection_index)
            .unwrap_or(0)
    }

    fn rasterize_aimer_run_into_cache(
        &mut self,
        font_id: FontId,
        pending: &[GlyphKey],
        font_size: f32,
    ) -> bool {
        self.ensure_aimer_font_cached(font_id);

        let aimer_font_cache = &mut self.aimer_font_cache;
        let cache = &mut self.cache;
        let retained_bitmap_bytes = &mut self.retained_bitmap_bytes;
        let advance_cache = &mut self.advance_cache;
        #[cfg(test)]
        let rasterize_call_count = &mut self.rasterize_call_count;

        match aimer_font_cache.get_mut(&font_id) {
            Some(Some(state)) => state.rasterize_glyphs_into(pending, font_size, |key, glyph| {
                #[cfg(test)]
                {
                    *rasterize_call_count += 1;
                }
                advance_cache.insert(key, glyph.advance_width);
                glyph_metrics::store(key, &glyph);
                Self::insert_cached_glyph_parts(cache, retained_bitmap_bytes, key, glyph);
            }),
            _ => false,
        }
    }

    #[cfg(test)]
    fn glyph_index_cache_len(&self) -> usize {
        self.coverage_index_cache
            .values()
            .map(|index| index.glyphs.len())
            .sum()
    }

    fn select_font_for_key(&mut self, key: GlyphKey) -> &mut FontRecord {
        self.ensure_fallback_loaded(key.font_id);

        if key.font_id == self.primary.id {
            &mut self.primary
        } else if let Some(index) = self
            .family_faces
            .iter()
            .position(|face| face.record.id == key.font_id)
        {
            &mut self.family_faces[index].record
        } else {
            self.fallbacks
                .as_mut()
                .and_then(|fbs| fbs.iter_mut().find(|fb| fb.id == key.font_id))
                .unwrap_or(&mut self.primary)
        }
    }

    /// Rasterize a single glyph at the given size, returning cached result if
    /// available.
    pub fn rasterize(&mut self, codepoint: char, font_size: f32) -> &RasterizedGlyph {
        let key = self.glyph_key_for_codepoint(codepoint, font_size);

        self.rasterize_key(key, font_size)
    }

    pub fn rasterize_key(&mut self, key: GlyphKey, font_size: f32) -> &RasterizedGlyph {
        self.rasterize_run(&[key], font_size);

        self.cache.get(&key).expect("glyph was just inserted")
    }

    /// Rasterizes and caches every glyph of `keys` that is not cached yet.
    ///
    /// The keys must all name the same face and be drawn at `font_size` —
    /// which is what [`group_into_runs`] produces — because the face is
    /// resolved, mapped and turned into a scaler once for the whole slice
    /// rather than once per glyph. Duplicate keys are drawn once.
    pub(super) fn rasterize_run(&mut self, keys: &[GlyphKey], font_size: f32) {
        self.refresh_registered_family_faces();
        self.run_buffers.pending_keys.clear();
        self.run_buffers.pending_seen.clear();
        for key in keys {
            if !self.cache.contains_key(key) && self.run_buffers.pending_seen.insert(*key) {
                self.run_buffers.pending_keys.push(*key);
            }
        }
        let pending = std::mem::take(&mut self.run_buffers.pending_keys);
        self.rasterize_pending_run(pending, font_size);
    }

    fn rasterize_pending_run(&mut self, pending: Vec<GlyphKey>, font_size: f32) {
        let Some(first) = pending.first().copied() else {
            self.run_buffers.pending_keys = pending;
            return;
        };
        debug_assert!(pending.iter().all(|key| key.font_id == first.font_id));
        self.cache.reserve(pending.len());
        self.advance_cache.reserve(pending.len());

        self.ensure_fallback_loaded(first.font_id);

        let complete = self.rasterize_aimer_run_into_cache(first.font_id, &pending, font_size);
        if complete {
            self.run_buffers.pending_keys = pending;
            return;
        }

        let mut fallback = std::mem::take(&mut self.run_buffers.fallback_keys);
        fallback.clear();
        fallback.extend(
            pending
                .iter()
                .filter(|key| !self.cache.contains_key(*key))
                .copied(),
        );
        if fallback.is_empty() {
            self.run_buffers.fallback_keys = fallback;
            self.run_buffers.pending_keys = pending;
            return;
        }

        let mut prepared = std::mem::take(&mut self.run_buffers.prepared);
        prepared.clear();
        self.draw_fallback_run(first.font_id, &fallback, font_size, &mut prepared);

        for (key, glyph) in prepared.drain(..) {
            #[cfg(test)]
            {
                self.rasterize_call_count += 1;
            }
            self.advance_cache.insert(key, glyph.advance_width);
            glyph_metrics::store(key, &glyph);
            self.insert_cached_glyph(key, glyph);
        }
        self.run_buffers.prepared = prepared;
        self.run_buffers.fallback_keys = fallback;
        self.run_buffers.pending_keys = pending;
    }

    /// Draws only glyphs the owned reader declined.
    ///
    /// Aimer is the sole portable parser, shaper, and rasterizer. A private
    /// Apple glyph may still be handed to the optional Core Text bridge; every
    /// other unsupported glyph receives an empty, correctly-advancing cache
    /// entry rather than silently invoking another font engine.
    fn draw_fallback_run(
        &mut self,
        font_id: FontId,
        pending: &[GlyphKey],
        font_size: f32,
        output: &mut Vec<(GlyphKey, RasterizedGlyph)>,
    ) {
        let record = self.font_record_by_id(font_id).unwrap_or(&self.primary);
        for key in pending.iter().copied() {
            let fallback_advance = record
                .advance_width_for_glyph(key.glyph_id, font_size)
                .unwrap_or(font_size * 0.5);
            let glyph = rasterize_platform_glyph(
                record,
                key.glyph_id,
                font_size,
                key.weight,
                fallback_advance,
            )
            .unwrap_or_else(|| RasterizedGlyph {
                bitmap: Vec::new(),
                width: 0,
                height: 0,
                offset_x: 0.0,
                offset_y: 0.0,
                advance_width: fallback_advance,
                is_color: record.is_color,
            });
            output.push((key, glyph));
        }
    }

    pub fn rasterize_bitmap_key(&mut self, key: GlyphKey, font_size: f32) -> &RasterizedGlyph {
        self.discard_partial_bitmaps(std::slice::from_ref(&key));
        self.rasterize_key(key, font_size)
    }

    /// Forgets cached entries whose coverage was released.
    ///
    /// A glyph whose bitmap was dropped to stay within the cache budget keeps
    /// its metrics, so it still answers as cached — but a caller that needs the
    /// coverage would get nothing. Dropping the entry makes the next
    /// rasterization draw it again.
    fn discard_partial_bitmaps(&mut self, keys: &[GlyphKey]) {
        for key in keys {
            if self
                .cache
                .get(key)
                .is_some_and(|glyph| glyph.width > 0 && glyph.height > 0 && glyph.bitmap.is_empty())
            {
                self.cache.remove(key);
            }
        }
    }

    /// A copy of the cached glyph `key` names, for handing to the renderer.
    fn cached_glyph_for_commit(&self, key: GlyphKey) -> Option<RasterizedGlyph> {
        self.cache.get(&key).cloned()
    }

    pub fn release_bitmap(&mut self, key: GlyphKey) {
        if let Some(glyph) = self.cache.get_mut(&key) {
            self.retained_bitmap_bytes = self
                .retained_bitmap_bytes
                .saturating_sub(glyph.bitmap.capacity());
            glyph.bitmap.clear();
            glyph.bitmap.shrink_to_fit();
        }
    }

    fn insert_cached_glyph(&mut self, key: GlyphKey, glyph: RasterizedGlyph) {
        Self::insert_cached_glyph_parts(
            &mut self.cache,
            &mut self.retained_bitmap_bytes,
            key,
            glyph,
        );
    }

    fn insert_cached_glyph_parts(
        cache: &mut HashMap<GlyphKey, RasterizedGlyph>,
        retained_bitmap_bytes: &mut usize,
        key: GlyphKey,
        glyph: RasterizedGlyph,
    ) {
        if let Some(previous) = cache.remove(&key) {
            *retained_bitmap_bytes =
                retained_bitmap_bytes.saturating_sub(previous.bitmap.capacity());
        }

        let incoming_bytes = glyph.bitmap.capacity();
        Self::make_bitmap_capacity_for_parts(cache, retained_bitmap_bytes, incoming_bytes);
        *retained_bitmap_bytes = retained_bitmap_bytes.saturating_add(incoming_bytes);
        cache.insert(key, glyph);
    }

    #[cfg(test)]
    fn make_bitmap_capacity_for(&mut self, incoming_bytes: usize) {
        Self::make_bitmap_capacity_for_parts(
            &mut self.cache,
            &mut self.retained_bitmap_bytes,
            incoming_bytes,
        );
    }

    fn make_bitmap_capacity_for_parts(
        cache: &mut HashMap<GlyphKey, RasterizedGlyph>,
        retained_bitmap_bytes: &mut usize,
        incoming_bytes: usize,
    ) {
        if retained_bitmap_bytes.saturating_add(incoming_bytes) <= Self::BITMAP_CACHE_CAPACITY_BYTES
        {
            return;
        }

        for glyph in cache.values_mut() {
            *retained_bitmap_bytes = retained_bitmap_bytes.saturating_sub(glyph.bitmap.capacity());
            glyph.bitmap.clear();
            glyph.bitmap.shrink_to_fit();
            if retained_bitmap_bytes.saturating_add(incoming_bytes)
                <= Self::BITMAP_CACHE_CAPACITY_BYTES
            {
                break;
            }
        }
    }

    pub fn bitmap_cache_bytes(&self) -> usize {
        self.retained_bitmap_bytes
    }

    pub fn cached_glyph_count(&self) -> usize {
        self.cache.len()
    }

    pub(super) fn needs_prepared_glyph(&mut self, key: GlyphKey, needs_bitmap: bool) -> bool {
        self.refresh_registered_family_faces();
        self.cache
            .get(&key)
            .is_none_or(|glyph| needs_bitmap && glyph.bitmap.is_empty())
    }

    pub(super) fn commit_prepared_glyph(&mut self, key: GlyphKey, glyph: RasterizedGlyph) {
        self.advance_cache.insert(key, glyph.advance_width);
        glyph_metrics::store(key, &glyph);
        self.insert_cached_glyph(key, glyph);
    }

    pub(super) fn cached_glyph_descriptor(&mut self, key: GlyphKey) -> Option<(bool, u32, u32)> {
        self.refresh_registered_family_faces();
        self.cache
            .get(&key)
            .map(|glyph| (glyph.is_color, glyph.width, glyph.height))
    }

    #[cfg(test)]
    fn cached_glyph(&self, key: GlyphKey) -> Option<&RasterizedGlyph> {
        self.cache.get(&key)
    }

    pub fn glyph_metrics_for_key(&mut self, key: GlyphKey, font_size: f32) -> RasterizedGlyph {
        self.rasterize_key(key, font_size).clone()
    }

    /// Returns the pixel box of `key` — bitmap size, pen and baseline offsets
    /// and advance — rasterizing only when the glyph has never been measured.
    ///
    /// Shaping is the consumer: it bakes these numbers into every
    /// [`ShapedGlyph`](super::text_layout::ShapedGlyph) so positioning reads
    /// them as plain fields. Because the metrics depend solely on the glyph
    /// key, they are shared process-wide — a shaping job running on a freshly
    /// created worker context reuses what any earlier frame or sibling worker
    /// measured, and publishes what it had to rasterize itself.
    pub(super) fn metrics_for_key(&mut self, key: GlyphKey, font_size: f32) -> GlyphMetrics {
        self.refresh_registered_family_faces();
        if let Some(glyph) = self.cache.get(&key) {
            return GlyphMetrics::from(glyph);
        }
        if let Some(metrics) = glyph_metrics::cached(key) {
            return metrics;
        }

        let glyph = self.rasterize_key(key, font_size);
        glyph_metrics::store(key, glyph);
        GlyphMetrics::from(glyph)
    }

    /// Makes the pixel boxes for one same-face shaped run available at once.
    ///
    /// Keys that already have process-wide metrics are not rasterized again.
    /// Remaining keys are sent through one run so the scaler, outline cache,
    /// and coverage path are reused. The callback receives one metric for each
    /// input key, including duplicates, in the original order.
    pub(super) fn with_metrics_for_keys<F>(
        &mut self,
        keys: &[GlyphKey],
        font_size: f32,
        mut emit: F,
    ) where
        F: FnMut(GlyphMetrics),
    {
        if keys.is_empty() {
            return;
        }

        self.refresh_registered_family_faces();
        let mut metric_values = std::mem::take(&mut self.run_buffers.metric_values);
        glyph_metrics::cached_many(keys, &mut metric_values);
        self.run_buffers.pending_keys.clear();
        self.run_buffers.pending_seen.clear();
        for (key, cached) in keys.iter().zip(&metric_values) {
            if self.cache.contains_key(key) || cached.is_some() {
                continue;
            }
            if self.run_buffers.pending_seen.insert(*key) {
                self.run_buffers.pending_keys.push(*key);
            }
        }

        let pending = std::mem::take(&mut self.run_buffers.pending_keys);
        self.rasterize_pending_run(pending, font_size);

        for (index, key) in keys.iter().enumerate() {
            let metrics = if let Some(glyph) = self.cache.get(key) {
                GlyphMetrics::from(glyph)
            } else {
                metric_values[index].unwrap_or_else(|| self.metrics_for_key(*key, font_size))
            };
            emit(metrics);
        }
        self.run_buffers.metric_values = metric_values;
    }

    /// Makes the pixel boxes for shaped glyphs available without constructing
    /// a temporary key vector at the call site.
    pub(super) fn with_metrics_for_shaped_glyphs<F>(
        &mut self,
        glyphs: &[ShapedRunGlyph],
        font_size: f32,
        emit: F,
    ) where
        F: FnMut(GlyphMetrics),
    {
        let mut keys = std::mem::take(&mut self.run_buffers.metric_keys);
        keys.clear();
        keys.extend(glyphs.iter().map(|glyph| glyph.glyph_key));
        self.with_metrics_for_keys(&keys, font_size, emit);
        self.run_buffers.metric_keys = keys;
    }

    /// Rasterizes every glyph `text` needs and emits cached glyphs by reference.
    ///
    /// The callback runs synchronously while the glyph remains in this
    /// rasterizer's cache; it must not retain the borrowed glyph. This avoids
    /// cloning every bitmap for consumers such as the atlas uploader that use
    /// each glyph immediately. The owned [`Self::preload_text`] API remains
    /// available for callers that need to retain the results.
    pub fn preload_text_into<F>(
        &mut self,
        text: &str,
        font_size: f32,
        language: Option<TextLanguage>,
        mut emit: F,
    ) where
        F: FnMut(GlyphKey, &RasterizedGlyph),
    {
        // A glyph preloaded under a different face than the one shaping picks is
        // a wasted rasterization, so the warm-up sees the run and its language
        // too.
        self.begin_script_run(text, language);
        let keys = text
            .chars()
            .filter(|codepoint| !codepoint.is_control())
            .map(|codepoint| self.glyph_key_for_codepoint(codepoint, font_size))
            .collect::<Vec<_>>();
        self.end_script_run();

        let same_face = keys
            .first()
            .is_none_or(|first| keys.iter().all(|key| key.font_id == first.font_id));
        if same_face {
            for chunk in keys.chunks(SEQUENTIAL_MAX_GLYPHS_PER_RUN) {
                self.discard_partial_bitmaps(chunk);
                self.rasterize_run(chunk, font_size);
            }
        } else {
            let runs = group_into_runs(
                keys.iter().copied().map(|key| (key, font_size)).collect(),
                SEQUENTIAL_MAX_GLYPHS_PER_RUN,
            );
            for run in runs {
                self.discard_partial_bitmaps(&run.keys);
                self.rasterize_run(&run.keys, run.font_size);
            }
        }

        for key in keys {
            if let Some(glyph) = self.cache.get(&key) {
                emit(key, glyph);
            }
        }
    }

    /// Rasterizes every glyph `text` needs and returns owned copies of the
    /// resulting cache entries.
    pub fn preload_text(
        &mut self,
        text: &str,
        font_size: f32,
        language: Option<TextLanguage>,
    ) -> Vec<(GlyphKey, RasterizedGlyph)> {
        let mut output = Vec::with_capacity(text.chars().count());
        self.preload_text_into(text, font_size, language, |key, glyph| {
            output.push((key, glyph.clone()));
        });
        output
    }

    pub fn advance_width(&mut self, codepoint: char, font_size: f32) -> f32 {
        let key = self.glyph_key_for_codepoint(codepoint, font_size);
        if let Some(width) = self.advance_cache.get(&key) {
            return *width;
        }

        if key.font_id != self.primary.id {
            self.ensure_fallback_loaded(key.font_id);
        }

        let width = self
            .aimer_advance_width_for_key(key, font_size)
            .unwrap_or_else(|| {
                self.select_font_for_key(key)
                    .advance_width_for_glyph(key.glyph_id, font_size)
                    .unwrap_or(0.0)
            });
        self.advance_cache.insert(key, width);
        width
    }

    pub fn advance_width_for_key(&mut self, key: GlyphKey, font_size: f32) -> f32 {
        self.refresh_registered_family_faces();
        if let Some(width) = self.advance_cache.get(&key) {
            return *width;
        }

        if key.font_id != self.primary.id {
            self.ensure_fallback_loaded(key.font_id);
        }

        let width = self
            .aimer_advance_width_for_key(key, font_size)
            .unwrap_or_else(|| {
                self.select_font_for_key(key)
                    .advance_width_for_glyph(key.glyph_id, font_size)
                    .unwrap_or(0.0)
            });
        self.advance_cache.insert(key, width);
        width
    }

    /// Returns line metrics (ascent, descent, line_gap) for the given font
    /// size. Uses the primary font for consistent line spacing.
    pub fn line_metrics(&self, font_size: f32) -> (f32, f32, f32) {
        self.line_metrics_for_family(
            font_size,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
        )
    }

    pub fn line_metrics_for_family(
        &self,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> (f32, f32, f32) {
        let stale_registered_family = family != FontFamily::SANS_SERIF
            && family != FontFamily::MONOSPACE
            && FontRegistry::revision() != self.registered_font_revision;
        let current_record = if stale_registered_family {
            FontRegistry::resolve(family, weight, style)
                .and_then(|face| FontRecord::from_shared_bytes(face.face_id, face.bytes))
        } else {
            None
        };
        let record = if stale_registered_family {
            current_record.as_ref().unwrap_or(&self.primary)
        } else {
            self.family_record(family, weight, style)
                .unwrap_or(&self.primary)
        };
        if record.id == self.primary.id
            && font_size.is_finite()
            && font_size > 0.0
            && let Some(data) = record.data()
            && let Some(metrics) =
                crate::text_pipeline::aimer_font::primary_metrics(data, record.collection_index)
        {
            return metrics.scaled_line_metrics(font_size);
        }
        if let Some(metrics) = record.line_metrics(font_size) {
            return metrics;
        }
        (font_size * 0.8, font_size * -0.2, 0.0)
    }

    /// Convenience: measure the advance width of a string.
    pub fn measure_text(&mut self, text: &str, font_size: f32) -> f32 {
        text.chars().map(|c| self.advance_width(c, font_size)).sum()
    }

    /// Shape a single grapheme cluster using the correct font (primary or
    /// fallback).
    ///
    /// Uses Aimer's checked OpenType shaper to shape the entire cluster as a unit, so that
    /// complex-script sequences (e.g. Khmer base + COENG + subscript
    /// consonant) produce the correct ligature glyph IDs and advances
    /// rather than being split into separate unrelated glyphs.
    ///
    /// Returns a list of `(GlyphKey, advance, x_offset, y_offset)` tuples.
    /// If shaping fails or the cluster is empty, returns an empty vec.
    pub fn shape_cluster(
        &mut self,
        cluster: &str,
        font_size: f32,
    ) -> Vec<(GlyphKey, f32, f32, f32)> {
        self.shape_cluster_for_family(
            cluster,
            font_size,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
        )
    }

    pub fn shape_cluster_for_family(
        &mut self,
        cluster: &str,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> Vec<(GlyphKey, f32, f32, f32)> {
        self.shape_run_for_family(cluster, font_size, family, weight, style)
            .into_iter()
            .map(|glyph| {
                (
                    glyph.glyph_key,
                    glyph.advance,
                    glyph.x_offset,
                    glyph.y_offset,
                )
            })
            .collect()
    }

    pub fn font_id_for_family_cluster(
        &mut self,
        cluster: &str,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> Option<FontId> {
        let base_char = cluster.chars().find(|codepoint| !codepoint.is_control())?;
        if family == FontFamily::SANS_SERIF {
            // SANS_SERIF is the fixed primary family. Re-entering the
            // family-record selector here only repeats a primary cmap probe
            // before the same fallback-aware key path does it again.
            return Some(
                self.glyph_key_for_codepoint_at_weight(base_char, font_size, weight)
                    .font_id,
            );
        }
        Some(
            self.glyph_key_for_family_codepoint(base_char, font_size, family, weight, style)
                .font_id,
        )
    }

    pub fn shape_run_for_family(
        &mut self,
        text: &str,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> Vec<ShapedRunGlyph> {
        let Some(font_id) = self.font_id_for_family_cluster(text, font_size, family, weight, style)
        else {
            return Vec::new();
        };
        self.shape_run_with_font_id(text, font_size, font_id, weight)
    }

    /// Shapes a family-selected run at arbitrary OpenType variation axes.
    pub fn shape_run_for_family_with_variations(
        &mut self,
        text: &str,
        font_size: f32,
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
        axes: &[(u32, f32)],
    ) -> Vec<ShapedRunGlyph> {
        let Some(base_char) = text.chars().find(|codepoint| !codepoint.is_control()) else {
            return Vec::new();
        };
        let key = self.glyph_key_for_family_codepoint_with_variations(
            base_char, font_size, family, weight, style, axes,
        );
        self.shape_run_with_font_id_with_variations(text, font_size, key.font_id, weight, axes)
    }

    pub fn shape_run_with_font_id(
        &mut self,
        text: &str,
        font_size: f32,
        font_id: FontId,
        weight: FontWeight,
    ) -> Vec<ShapedRunGlyph> {
        let mut output = Vec::new();
        self.shape_run_with_font_id_into(
            text,
            font_size,
            font_id,
            weight,
            None,
            false,
            None,
            &mut output,
        );
        output
    }

    /// Shapes a run at arbitrary OpenType variation axes through the
    /// Aimer path. The returned glyph keys carry the same
    /// variation identity that rasterization and metrics consume.
    pub fn shape_run_with_font_id_with_variations(
        &mut self,
        text: &str,
        font_size: f32,
        font_id: FontId,
        weight: FontWeight,
        axes: &[(u32, f32)],
    ) -> Vec<ShapedRunGlyph> {
        let glyph_weight = self.glyph_weight_for_request(font_id, weight);
        let variation_id = self.variation_id_for_font(font_id, glyph_weight, axes);
        let mut output = Vec::new();
        self.shape_run_with_font_id_and_variation_into(
            text,
            font_size,
            font_id,
            weight,
            None,
            false,
            variation_id,
            None,
            &mut output,
        );
        output
    }

    /// Shapes a run using the rasterizer's reusable output storage.
    pub(super) fn shape_run_with_font_id_reusing(
        &mut self,
        text: &str,
        font_size: f32,
        font_id: FontId,
        weight: FontWeight,
    ) -> Vec<ShapedRunGlyph> {
        self.shape_run_with_font_id_reusing_with_options(
            text, font_size, font_id, weight, None, false,
        )
    }

    /// Shapes a run while passing the optional CJK language and vertical
    /// substitution hints to the owned shaper.
    pub(super) fn shape_run_with_font_id_reusing_with_options(
        &mut self,
        text: &str,
        font_size: f32,
        font_id: FontId,
        weight: FontWeight,
        language: Option<TextLanguage>,
        vertical: bool,
    ) -> Vec<ShapedRunGlyph> {
        self.shape_run_with_font_id_reusing_with_options_and_script(
            text, font_size, font_id, weight, language, vertical, None,
        )
    }

    /// Shapes a run with a paragraph-derived script hint.
    ///
    /// The hint only skips the owned-shaper eligibility scan. The checked
    /// Aimer layout dispatcher still validates the text, so a stale or
    /// unsupported hint follows the same owned fallback path.
    pub(super) fn shape_run_with_font_id_reusing_with_options_and_script(
        &mut self,
        text: &str,
        font_size: f32,
        font_id: FontId,
        weight: FontWeight,
        language: Option<TextLanguage>,
        vertical: bool,
        script_hint: Option<Script>,
    ) -> Vec<ShapedRunGlyph> {
        let mut output = std::mem::take(&mut self.run_buffers.shaped_glyphs);
        #[cfg(test)]
        {
            self.reused_shape_output_count += 1;
        }
        self.shape_run_with_font_id_into(
            text,
            font_size,
            font_id,
            weight,
            language,
            vertical,
            script_hint,
            &mut output,
        );
        output
    }

    /// Copies cached shaped output into the rasterizer's reusable buffer.
    pub(super) fn reuse_shaped_run_from_slice(
        &mut self,
        glyphs: &[ShapedRunGlyph],
    ) -> Vec<ShapedRunGlyph> {
        let mut output = std::mem::take(&mut self.run_buffers.shaped_glyphs);
        output.clear();
        output.extend_from_slice(glyphs);
        output
    }

    /// Returns shaped output storage to the rasterizer for a later run.
    pub(super) fn recycle_shaped_run(&mut self, mut glyphs: Vec<ShapedRunGlyph>) {
        glyphs.clear();
        self.run_buffers.shaped_glyphs = glyphs;
    }

    fn shape_run_with_font_id_into(
        &mut self,
        text: &str,
        font_size: f32,
        font_id: FontId,
        weight: FontWeight,
        language: Option<TextLanguage>,
        vertical: bool,
        script_hint: Option<Script>,
        output: &mut Vec<ShapedRunGlyph>,
    ) {
        self.shape_run_with_font_id_and_variation_into(
            text,
            font_size,
            font_id,
            weight,
            language,
            vertical,
            0,
            script_hint,
            output,
        );
    }

    fn shape_run_with_font_id_and_variation_into(
        &mut self,
        text: &str,
        font_size: f32,
        font_id: FontId,
        weight: FontWeight,
        language: Option<TextLanguage>,
        vertical: bool,
        variation_id: u32,
        _script_hint: Option<Script>,
        output: &mut Vec<ShapedRunGlyph>,
    ) {
        output.clear();
        if text.is_empty() {
            return;
        }
        self.ensure_fallback_loaded(font_id);
        // One face shapes the whole run, so its glyphs share one weight; see
        // [`Self::glyph_weight_for_request`].
        let glyph_weight = self.glyph_weight_for_request(font_id, weight);

        self.ensure_aimer_font_cached(font_id);
        if let Some(Some(state)) = self.aimer_font_cache.get(&font_id) {
            if let Ok(Some(shaped)) = state.shape_run_with_options_at_variation(
                text,
                language,
                vertical,
                glyph_weight,
                variation_id,
            ) {
                let upem = f32::from(shaped.units_per_em);
                let scale = if upem > 0.0 { font_size / upem } else { 1.0 };
                let shaped_glyphs = shaped.glyphs;
                let has_horizontal_metric_variations =
                    !vertical && state.has_horizontal_metric_variations();
                #[cfg(test)]
                {
                    self.shape_call_count += 1;
                }
                output.extend(shaped_glyphs.iter().map(|glyph| {
                    let horizontal_delta = if has_horizontal_metric_variations {
                        state
                            .horizontal_advance_delta_at_variation(
                                glyph.glyph_id,
                                glyph_weight,
                                variation_id,
                            )
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    ShapedRunGlyph {
                        glyph_key: GlyphKey::new(font_id, glyph.glyph_id, font_size)
                            .weighted(glyph_weight)
                            .with_variation_id(variation_id),
                        advance: (glyph.x_advance + horizontal_delta) as f32 * scale,
                        y_advance: glyph.y_advance as f32 * scale,
                        x_offset: glyph.x_offset as f32 * scale,
                        y_offset: glyph.y_offset as f32 * scale,
                        cluster: glyph.cluster,
                    }
                }));
                crate::text_pipeline::aimer_font::recycle_shaped_glyphs(shaped_glyphs);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    use hashbrown::{HashMap, HashSet};

    use super::*;
    use crate::font::{FontFamily, FontRegistration, FontRegistry, FontStyle, FontWeight};
    use crate::text_pipeline::text_layout::{layout_shaped_text, shape_text_styled};

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn runtime_font_ids_skip_a_system_primary_face() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        rasterizer.primary.id = SYSTEM_FALLBACK_ID_BASE;

        assert_eq!(rasterizer.next_fallback_font_id(), 1);
    }

    #[derive(Default)]
    struct HashWriteCounter {
        writes: usize,
        value: u64,
    }

    impl Hasher for HashWriteCounter {
        fn finish(&self) -> u64 {
            0
        }

        fn write(&mut self, _bytes: &[u8]) {
            self.writes += 1;
        }

        fn write_u64(&mut self, value: u64) {
            self.writes += 1;
            self.value = value;
        }
    }

    fn compact_glyph_key_hash(key: GlyphKey) -> u64 {
        let mut hasher = HashWriteCounter::default();
        key.hash(&mut hasher);
        assert_eq!(hasher.writes, 1);
        hasher.value
    }

    /// The portable Aimer profile must not silently re-enter Core Text for a
    /// glyph the owned reader declined. Its last-resort contract is an empty
    /// bitmap with the shaped advance preserved by the caller.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        not(feature = "apple-core-text")
    ))]
    #[test]
    fn portable_owned_font_does_not_call_the_platform_rasterizer() {
        let record = primary_font_record();
        assert!(rasterize_platform_glyph(&record, 1, 16.0, 400, 8.0).is_none());
    }

    #[test]
    fn glyph_key_hashes_as_one_compact_value() {
        let key = GlyphKey {
            font_id: 7,
            glyph_id: 42,
            size_tenths: 160,
            subpixel_x: 1,
            subpixel_y: 2,
            weight: 400,
            variation_id: 0,
        };
        let expected = compact_glyph_key_hash(key);
        for distinct_key in [
            GlyphKey { font_id: 8, ..key },
            GlyphKey {
                glyph_id: 43,
                ..key
            },
            GlyphKey {
                size_tenths: 170,
                ..key
            },
            GlyphKey {
                subpixel_x: 2,
                ..key
            },
            GlyphKey {
                subpixel_y: 3,
                ..key
            },
            GlyphKey { weight: 300, ..key },
            GlyphKey {
                variation_id: 1,
                ..key
            },
        ] {
            assert_ne!(compact_glyph_key_hash(distinct_key), expected);
        }
    }

    #[test]
    fn rasterizer_uses_fast_hashers_for_internal_caches() {
        fn assert_fast_map<K, V>(_: &HashMap<K, V>) {}
        fn assert_fast_set<K>(_: &HashSet<K>) {}

        let rasterizer = GlyphRasterizer::primary_only();
        assert_fast_map(&rasterizer.cache);
        assert_fast_map(&rasterizer.advance_cache);
        assert_fast_set(&rasterizer.unsupported_codepoints);
        assert_fast_map(&rasterizer.font_bytes_cache);
        assert_fast_map(&rasterizer.aimer_font_cache);
    }

    #[test]
    fn aimer_font_state_is_reused_for_repeated_shape_calls() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let font_id = rasterizer.primary_font_id();

        let first = rasterizer.shape_run_with_font_id("AV", 16.0, font_id, FontWeight::Normal);
        let second = rasterizer.shape_run_with_font_id("AV", 16.0, font_id, FontWeight::Normal);

        assert_eq!(
            first
                .iter()
                .map(|glyph| (glyph.glyph_key, glyph.advance, glyph.cluster))
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|glyph| (glyph.glyph_key, glyph.advance, glyph.cluster))
                .collect::<Vec<_>>(),
        );
        assert_eq!(rasterizer.aimer_font_cache.len(), 1);
        assert!(
            rasterizer
                .aimer_font_cache
                .get(&font_id)
                .is_some_and(Option::is_some)
        );
    }

    #[test]
    fn a_run_draws_the_same_glyphs_as_drawing_them_one_by_one() {
        let text = "Aimer 123 {}();";
        let mut alone = GlyphRasterizer::new();
        let mut together = GlyphRasterizer::new();
        let keys = text
            .chars()
            .map(|codepoint| alone.glyph_key_for_codepoint(codepoint, 15.0))
            .collect::<Vec<_>>();

        together.rasterize_run(&keys, 15.0);

        for key in &keys {
            let expected = alone.rasterize_key(*key, 15.0).clone();
            let actual = together
                .cached_glyph(*key)
                .expect("a run rasterizes every key it is given");
            assert_eq!(actual.bitmap, expected.bitmap, "coverage must not change");
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height)
            );
            assert_eq!(
                (actual.offset_x, actual.offset_y),
                (expected.offset_x, expected.offset_y)
            );
            assert_eq!(actual.advance_width, expected.advance_width);
            assert_eq!(actual.is_color, expected.is_color);
        }
    }

    #[test]
    fn a_run_draws_a_repeated_glyph_once() {
        let mut rasterizer = GlyphRasterizer::new();
        let key = rasterizer.glyph_key_for_codepoint('e', 16.0);

        rasterizer.rasterize_run(&[key, key, key], 16.0);

        assert_eq!(
            rasterizer.rasterize_call_count(),
            1,
            "the same glyph drawn twice is coverage computed twice"
        );
    }

    #[test]
    fn glyphs_a_run_leaves_out_are_still_cached_from_before() {
        let mut rasterizer = GlyphRasterizer::new();
        let key = rasterizer.glyph_key_for_codepoint('m', 20.0);
        rasterizer.rasterize_key(key, 20.0);
        let before = rasterizer.rasterize_call_count();

        rasterizer.rasterize_run(&[key], 20.0);

        assert_eq!(
            rasterizer.rasterize_call_count(),
            before,
            "a cached glyph must not be drawn again"
        );
    }

    #[cfg(any(
        not(any(target_os = "ios", target_os = "macos")),
        feature = "apple-core-text"
    ))]
    #[test]
    fn codepoints_outside_the_static_probe_groups_rasterize_to_visible_glyphs() {
        let mut rasterizer = GlyphRasterizer::new();
        for codepoint in ['你', '好', '！', '，', 'ü', '₫'] {
            let key = rasterizer.glyph_key_for_codepoint(codepoint, 20.0);
            assert_ne!(
                key.glyph_id, 0,
                "{codepoint:?} resolved to .notdef instead of a real glyph"
            );
            let glyph = rasterizer.rasterize_key(key, 20.0);
            assert!(
                !glyph.bitmap.is_empty(),
                "{codepoint:?} rasterized to an empty bitmap"
            );
        }
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn apple_host_fallback_matrix_rasterizes_nonbundled_script_samples() {
        let mut rasterizer = GlyphRasterizer::new();
        for (label, codepoint) in [
            ("Arabic", 'م'),
            ("Emoji", '😀'),
            ("Korean", '한'),
            ("Myanmar", 'မ'),
        ] {
            let key = rasterizer.glyph_key_for_codepoint(codepoint, 20.0);
            assert_ne!(
                key.glyph_id, 0,
                "{label} {codepoint:?} resolved to .notdef instead of a host glyph"
            );
            let glyph = rasterizer.rasterize_key(key, 20.0);
            assert!(
                !glyph.bitmap.is_empty(),
                "{label} {codepoint:?} rasterized to an empty host bitmap"
            );
        }
    }

    #[test]
    fn aimer_font_rasterization_is_the_standard_glyph_path() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let key = rasterizer.glyph_key_for_codepoint('A', 16.0);
        let expected = crate::text_pipeline::aimer_font::rasterize_font_glyph(
            PRIMARY_FONT,
            0,
            key.glyph_id,
            16.0,
            key.subpixel_x,
            key.subpixel_y,
        )
        .expect("the primary face's A glyph should use its Aimer outline");
        let actual = rasterizer.rasterize_key(key, 16.0).clone();

        assert_eq!(actual.bitmap, expected.bitmap);
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height)
        );
        assert_eq!(
            (actual.offset_x, actual.offset_y),
            (expected.offset_x, expected.offset_y)
        );
        assert_eq!(actual.advance_width, expected.advance_width);
        assert!(!actual.is_color);
    }

    #[test]
    fn owned_font_shapes_latin_runs_with_checked_open_type_tables() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let font_id = rasterizer.primary_font_id();
        let shaped =
            rasterizer.shape_run_with_font_id("office", 1000.0, font_id, FontWeight::Normal);

        assert_eq!(
            shaped
                .iter()
                .map(|glyph| glyph.glyph_key.glyph_id)
                .collect::<Vec<_>>(),
            vec![271, 386, 203, 213]
        );
        assert_eq!(
            shaped.iter().map(|glyph| glyph.cluster).collect::<Vec<_>>(),
            vec![0, 1, 4, 5]
        );
        assert_eq!(
            shaped.iter().map(|glyph| glyph.advance).collect::<Vec<_>>(),
            vec![574.0, 905.0, 530.0, 561.0]
        );

        let shaped = rasterizer.shape_run_with_font_id("AV", 1000.0, font_id, FontWeight::Normal);
        assert_eq!(
            shaped
                .iter()
                .map(|glyph| glyph.glyph_key.glyph_id)
                .collect::<Vec<_>>(),
            vec![1, 156]
        );
        assert_eq!(
            shaped.iter().map(|glyph| glyph.advance).collect::<Vec<_>>(),
            vec![590.0, 633.0]
        );
    }

    #[test]
    fn owned_font_routes_arabic_joining_forms_through_the_checked_shaper() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let font_id = rasterizer
            .register_font_bytes(
                crate::text_pipeline::aimer_font::tests::arabic_joining_font_for_test(),
            )
            .expect("the Arabic shaping fixture must register");
        let shaped = rasterizer.shape_run_with_font_id("ببب", 1000.0, font_id, FontWeight::Normal);

        assert_eq!(
            shaped
                .iter()
                .map(|glyph| glyph.glyph_key.glyph_id)
                .collect::<Vec<_>>(),
            vec![3, 5, 4]
        );
        assert_eq!(
            shaped.iter().map(|glyph| glyph.cluster).collect::<Vec<_>>(),
            vec![0, 2, 4]
        );
        assert_eq!(
            shaped.iter().map(|glyph| glyph.advance).collect::<Vec<_>>(),
            vec![700.0, 800.0, 700.0]
        );

        let shaped = rasterizer.shape_run_with_font_id("بَب", 1000.0, font_id, FontWeight::Normal);
        assert_eq!(
            shaped
                .iter()
                .map(|glyph| glyph.glyph_key.glyph_id)
                .collect::<Vec<_>>(),
            vec![3, 8, 4]
        );
        assert_eq!(
            shaped.iter().map(|glyph| glyph.cluster).collect::<Vec<_>>(),
            vec![0, 2, 4]
        );
        assert_eq!(
            shaped.iter().map(|glyph| glyph.advance).collect::<Vec<_>>(),
            vec![700.0, 0.0, 700.0]
        );
        assert_eq!((shaped[1].x_offset, shaped[1].y_offset), (200.0, 700.0));

        let shaped = rasterizer.shape_run_with_font_id("بب", 1000.0, font_id, FontWeight::Normal);
        assert_eq!(shaped.len(), 1);
        assert_eq!(shaped[0].glyph_key.glyph_id, 9);
        assert_eq!(shaped[0].cluster, 0);
        assert_eq!(shaped[0].advance, 900.0);

        let shaped = rasterizer.shape_run_with_font_id("بََب", 1000.0, font_id, FontWeight::Normal);
        assert_eq!(
            shaped
                .iter()
                .map(|glyph| glyph.glyph_key.glyph_id)
                .collect::<Vec<_>>(),
            vec![3, 8, 8, 4]
        );
        assert_eq!((shaped[1].x_offset, shaped[1].y_offset), (200.0, 700.0));
        assert_eq!((shaped[2].x_offset, shaped[2].y_offset), (250.0, 1000.0));

        let shaped = rasterizer.shape_run_with_font_id("با", 1000.0, font_id, FontWeight::Normal);
        assert_eq!(
            shaped
                .iter()
                .map(|glyph| glyph.glyph_key.glyph_id)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert_eq!((shaped[1].x_offset, shaped[1].y_offset), (100.0, 60.0));
        assert_eq!(
            shaped.iter().map(|glyph| glyph.advance).collect::<Vec<_>>(),
            vec![700.0, 600.0]
        );

        let shaped = rasterizer.shape_run_with_font_id("اب", 1000.0, font_id, FontWeight::Normal);
        assert_eq!(
            shaped
                .iter()
                .map(|glyph| glyph.glyph_key.glyph_id)
                .collect::<Vec<_>>(),
            vec![7, 6]
        );
        assert_eq!(
            shaped.iter().map(|glyph| glyph.cluster).collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(
            shaped.iter().map(|glyph| glyph.advance).collect::<Vec<_>>(),
            vec![600.0, 600.0]
        );
    }

    #[test]
    fn owned_rasterization_matches_scalar_reference_quality() {
        const SIZES: &[f32] = &[9.0, 12.0, 16.0, 24.0, 32.0];
        const LATIN: &[char] = &['A', 'a', 'e', 'g', 'M', 'S', '0', '@', '&', 'R'];
        const CJK: &[char] = &['あ', '你', '漢', '語', '日', '本', '々', '猫'];

        let latin = FontRecord::from_static_bytes(
            0x2000_0001,
            include_bytes!("../../../fonts/JetBrainsMono-Regular.ttf"),
        )
        .expect("the checked-in Latin quality face must load");
        let cjk = FontRecord::from_static_bytes(
            0x2000_0002,
            include_bytes!("../../../fonts/NotoSansJP-VariableFont_wght.ttf"),
        )
        .expect("the checked-in CJK quality face must load");

        for (label, record, codepoints) in [("Latin", latin, LATIN), ("CJK", cjk, CJK)] {
            let glyph_ids = codepoints
                .iter()
                .map(|codepoint| {
                    record
                        .glyph_index(*codepoint)
                        .unwrap_or_else(|| panic!("{label} face must cover {codepoint:?}"))
                })
                .collect::<Vec<_>>();
            let data = record.data().expect("quality face bytes must be present");
            let mut stats = RasterQualityStats::default();

            for font_size in SIZES {
                let aimer = crate::text_pipeline::aimer_font::rasterize_font_glyphs(
                    data.as_ref(),
                    record.collection_index,
                    &glyph_ids
                        .iter()
                        .map(|glyph_id| (*glyph_id, 0, 0))
                        .collect::<Vec<_>>(),
                    *font_size,
                );
                let reference = glyph_ids
                    .iter()
                    .map(|glyph_id| {
                        crate::text_pipeline::aimer_font::rasterize_font_glyph(
                            data.as_ref(),
                            record.collection_index,
                            *glyph_id,
                            *font_size,
                            0,
                            0,
                        )
                    })
                    .collect::<Vec<_>>();

                assert_eq!(aimer.len(), reference.len());
                for (aimer, reference) in aimer.iter().zip(reference.iter()) {
                    accumulate_raster_quality(&mut stats, aimer.as_ref(), reference.as_ref());
                }
            }

            let mean_absolute_error = stats.mean_absolute_error();
            println!(
                "Owned rasterization {label}: {} glyph samples, mean absolute coverage error {mean_absolute_error:.4}, max edge error {} px, missing {}",
                stats.samples, stats.max_edge_error, stats.missing_samples,
            );

            assert_eq!(
                stats.missing_samples, 0,
                "{label} quality samples must draw"
            );
            assert!(
                stats.max_edge_error <= 1,
                "{label} unhinted bounds must stay within one pixel: {}",
                stats.max_edge_error
            );
            assert!(
                mean_absolute_error <= 0.30,
                "{label} unhinted coverage error is too high: {mean_absolute_error:.4}"
            );
        }
    }

    #[derive(Default)]
    struct RasterQualityStats {
        samples: usize,
        compared_pixels: u64,
        absolute_error: u64,
        max_edge_error: i32,
        missing_samples: usize,
    }

    impl RasterQualityStats {
        fn mean_absolute_error(&self) -> f64 {
            if self.compared_pixels == 0 {
                return 0.0;
            }
            self.absolute_error as f64 / (self.compared_pixels as f64 * 255.0)
        }
    }

    fn accumulate_raster_quality(
        stats: &mut RasterQualityStats,
        aimer: Option<&RasterizedGlyph>,
        reference: Option<&RasterizedGlyph>,
    ) {
        let (Some(aimer), Some(reference)) = (aimer, reference) else {
            stats.missing_samples += 1;
            return;
        };
        if aimer.bitmap.is_empty() || reference.bitmap.is_empty() {
            stats.missing_samples += 1;
            return;
        }

        assert!(
            (aimer.advance_width - reference.advance_width).abs() <= 0.001,
            "scalar and batched advances diverged: {} vs {}",
            aimer.advance_width,
            reference.advance_width
        );
        stats.samples += 1;
        let aimer_x = aimer.offset_x.round() as i32;
        let aimer_y = aimer.offset_y.round() as i32;
        let reference_x = reference.offset_x.round() as i32;
        let reference_y = reference.offset_y.round() as i32;
        let aimer_right = aimer_x + i32::try_from(aimer.width).expect("width fits in i32");
        let aimer_top = aimer_y + i32::try_from(aimer.height).expect("height fits in i32");
        let reference_right =
            reference_x + i32::try_from(reference.width).expect("width fits in i32");
        let reference_top =
            reference_y + i32::try_from(reference.height).expect("height fits in i32");
        stats.max_edge_error = stats.max_edge_error.max(
            (aimer_x - reference_x)
                .abs()
                .max((aimer_y - reference_y).abs())
                .max((aimer_right - reference_right).abs())
                .max((aimer_top - reference_top).abs()),
        );

        let left = aimer_x.min(reference_x);
        let right = aimer_right.max(reference_right);
        let bottom = aimer_y.min(reference_y);
        let top = aimer_top.max(reference_top);
        for y in bottom..top {
            for x in left..right {
                let aimer_coverage = raster_quality_pixel(aimer, aimer_x, aimer_y, x, y);
                let reference_coverage =
                    raster_quality_pixel(reference, reference_x, reference_y, x, y);
                stats.absolute_error += u64::from(aimer_coverage.abs_diff(reference_coverage));
            }
        }
        stats.compared_pixels += u64::try_from(right - left).expect("width is non-negative")
            * u64::try_from(top - bottom).expect("height is non-negative");
    }

    fn raster_quality_pixel(
        glyph: &RasterizedGlyph,
        origin_x: i32,
        origin_y: i32,
        x: i32,
        y: i32,
    ) -> u8 {
        let local_x = x - origin_x;
        let local_y = origin_y + i32::try_from(glyph.height).expect("height fits in i32") - 1 - y;
        if local_x < 0
            || local_y < 0
            || local_x >= i32::try_from(glyph.width).expect("width fits in i32")
            || local_y >= i32::try_from(glyph.height).expect("height fits in i32")
        {
            return 0;
        }
        let index = usize::try_from(local_y).expect("local y is non-negative")
            * usize::try_from(glyph.width).expect("width fits in usize")
            + usize::try_from(local_x).expect("local x is non-negative");
        glyph.bitmap.get(index).copied().unwrap_or(0)
    }

    /// A face carrying the Japanese half of Han and nothing of the rest.
    ///
    /// Apple systems always ship one, because Japanese faces have no reason to
    /// draw simplified-only characters. Which face the platform *offers* for a
    /// given character depends on the device's language, so the rule this face
    /// exists to test is asserted by adopting it directly rather than by
    /// hoping the cascade proposes it.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    fn japanese_only_han_face(next_id: FontId) -> FontRecord {
        use crate::text_pipeline::apple_fonts::font_paths_for_codepoint;
        use crate::text_pipeline::font_resolver::REGULAR_WEIGHT;

        let draws = |record: &FontRecord, codepoint: char| {
            record
                .glyph_index(codepoint)
                .is_some_and(|glyph_id| glyph_id != 0)
        };

        font_paths_for_codepoint('好', REGULAR_WEIGHT)
            .into_iter()
            .flat_map(|path| {
                (0..8).map(move |collection_index| FontRecord {
                    id: next_id,
                    bytes: None,
                    collection_index,
                    path: Some(Arc::new(path.clone())),
                    is_color: false,
                })
            })
            .find(|record| draws(record, '好') && !draws(record, '吗'))
            .expect("apple systems ship a japanese face carrying 好 but not 吗")
    }

    // Han is unified: `時` and `好` are the same codepoints in Japanese and in
    // Chinese, so a Japanese face carries them while carrying none of the
    // simplified-only characters beside them. Loaded first for the kana of
    // `あの時は`, that face would claim the Han it happens to cover and leave
    // `你吗` to another one — one line drawn in two typefaces.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn han_keeps_one_face_when_a_japanese_face_was_loaded_first() {
        let mut rasterizer = GlyphRasterizer::new();
        let japanese = japanese_only_han_face(rasterizer.next_fallback_font_id());
        let japanese_id = japanese.id;
        rasterizer.adopt_fallback(japanese);

        let faces: Vec<(char, FontId)> = "時你好吗"
            .chars()
            .map(|codepoint| (codepoint, rasterizer.font_id_for_codepoint(codepoint)))
            .collect();

        assert!(
            faces.iter().all(|(_, font_id)| *font_id != japanese_id),
            "a face covering only half of han was chosen for it: {faces:?}"
        );
        let (_, first) = faces[0];
        assert!(
            faces.iter().all(|(_, font_id)| *font_id == first),
            "han split across faces: {faces:?}"
        );
    }

    // The rule cuts both ways. A kanji standing among kana belongs to a
    // Japanese word, so taking it away from the kana's face is the same defect
    // seen from the other side: `時` drawn in a Chinese face reads bolder than
    // the `あの` and `は` around it. Told which text it is resolving, the
    // rasterizer keeps the word in one face.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_kanji_among_kana_keeps_the_face_its_kana_use() {
        let mut rasterizer = GlyphRasterizer::new();
        let japanese = japanese_only_han_face(rasterizer.next_fallback_font_id());
        let japanese_id = japanese.id;
        rasterizer.adopt_fallback(japanese);

        rasterizer.begin_script_run("あの時は", None);
        let faces: Vec<(char, FontId)> = "あの時は"
            .chars()
            .map(|codepoint| (codepoint, rasterizer.font_id_for_codepoint(codepoint)))
            .collect();
        rasterizer.end_script_run();

        assert!(
            faces.iter().all(|(_, font_id)| *font_id == japanese_id),
            "a japanese word was split between a kana face and another: {faces:?}"
        );
    }

    // And a Chinese word must still elect the face covering all of it, even
    // though a Japanese face sits at the head of the chain drawing part of it.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_chinese_run_rejects_a_face_covering_only_half_of_it() {
        let mut rasterizer = GlyphRasterizer::new();
        let japanese = japanese_only_han_face(rasterizer.next_fallback_font_id());
        let japanese_id = japanese.id;
        rasterizer.adopt_fallback(japanese);

        rasterizer.begin_script_run("你好吗", None);
        let faces: Vec<(char, FontId)> = "你好吗"
            .chars()
            .map(|codepoint| (codepoint, rasterizer.font_id_for_codepoint(codepoint)))
            .collect();
        rasterizer.end_script_run();

        assert!(
            faces.iter().all(|(_, font_id)| *font_id != japanese_id),
            "a face covering only half of the word was chosen for it: {faces:?}"
        );
        let (_, first) = faces[0];
        assert!(
            faces.iter().all(|(_, font_id)| *font_id == first),
            "one chinese word split across faces: {faces:?}"
        );
    }

    // Preferring a script-wide face may not cost coverage: every character of
    // a mixed line must still resolve to a real glyph, whichever face was
    // loaded first.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_mixed_japanese_and_chinese_line_draws_every_character() {
        let mut rasterizer = GlyphRasterizer::new();
        let japanese = japanese_only_han_face(rasterizer.next_fallback_font_id());
        rasterizer.adopt_fallback(japanese);

        for codepoint in "あの時は你好吗".chars() {
            let key = rasterizer.glyph_key_for_codepoint(codepoint, 20.0);
            assert_ne!(
                key.glyph_id, 0,
                "{codepoint:?} resolved to .notdef instead of a real glyph"
            );
        }
    }

    /// A face whose glyphs only the platform can draw — Apple's `hvgl` file.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    fn platform_only_chinese_face(next_id: FontId) -> FontRecord {
        use crate::text_pipeline::apple_fonts::font_paths_for_codepoint;
        use crate::text_pipeline::font_resolver::REGULAR_WEIGHT;

        font_paths_for_codepoint('吗', REGULAR_WEIGHT)
            .into_iter()
            .flat_map(|path| {
                (0..32).map(move |collection_index| FontRecord {
                    id: next_id,
                    bytes: None,
                    collection_index,
                    path: Some(Arc::new(path.clone())),
                    is_color: false,
                })
            })
            .find(|record| {
                matches!(record.glyph_index('吗'), Some(glyph_id) if glyph_id != 0)
                    && record_outlines_unreadable(record)
            })
            .expect("apple systems ship a chinese face only the platform can draw")
    }

    /// A kana face Cupid decodes itself, together with its design weight.
    ///
    /// The face must not cover simplified-only Han, so the Han of a mixed run
    /// falls through to the platform-only face beside it.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    fn decodable_kana_face(next_id: FontId) -> (FontRecord, u16) {
        use crate::text_pipeline::apple_fonts::font_paths_for_codepoint;
        use crate::text_pipeline::font_resolver::REGULAR_WEIGHT;

        font_paths_for_codepoint('に', REGULAR_WEIGHT)
            .into_iter()
            .flat_map(|path| {
                (0..8).map(move |collection_index| FontRecord {
                    id: next_id,
                    bytes: None,
                    collection_index,
                    path: Some(Arc::new(path.clone())),
                    is_color: false,
                })
            })
            .find_map(|record| {
                let maps_kana = matches!(record.glyph_index('に'), Some(glyph_id) if glyph_id != 0);
                let maps_simplified =
                    matches!(record.glyph_index('吗'), Some(glyph_id) if glyph_id != 0);
                if !maps_kana || maps_simplified || record_outlines_unreadable(&record) {
                    return None;
                }
                let weight = record.design_weight()?;
                Some((record, weight))
            })
            .expect("apple systems ship a decodable japanese face")
    }

    // The "PingFang is still bolder" defect on iOS: no readable face covers
    // simplified Han there, so the platform draws it, and pinned to the flat
    // default of 400 it stands beside kana whose face is designed at W3
    // (300). The key must carry the kana face's weight so the platform draws
    // the same stroke the reader sees around it.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_platform_drawn_ideograph_matches_the_weight_of_the_kana_beside_it() {
        // The chain is built by hand: `GlyphRasterizer::new()` shares a
        // process-wide chain that a background warm-up fills asynchronously,
        // so the companion could resolve to whichever system face happened to
        // be loaded by then rather than to the face this test adopts.
        let mut rasterizer = GlyphRasterizer::primary_only();
        let (kana, kana_weight) = decodable_kana_face(rasterizer.next_fallback_font_id());
        rasterizer.adopt_fallback(kana);
        let platform = platform_only_chinese_face(rasterizer.next_fallback_font_id());
        let platform_id = platform.id;
        rasterizer.adopt_fallback(platform);

        rasterizer.begin_script_run("には你好吗", None);
        let key = rasterizer.glyph_key_for_codepoint('吗', 20.0);
        rasterizer.end_script_run();

        assert_eq!(
            key.font_id, platform_id,
            "the simplified-only character must fall to the platform face"
        );
        assert_eq!(
            key.weight, kana_weight,
            "a platform-drawn ideograph must match its kana companions"
        );
    }

    // A run without kana has no companion of its own, but it must not fall
    // back to a *different* weight than the one kana would bring: a Chinese
    // line typed alone would render bolder, then snap thinner the moment a
    // kana lands beside it. The baseline therefore comes from the face kana
    // *would* resolve to, so the ideographs hold one stroke throughout.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_han_only_run_is_drawn_at_the_weight_kana_would_pair_with() {
        // The chain is built by hand: `GlyphRasterizer::new()` shares a
        // process-wide chain that a background warm-up fills asynchronously,
        // so the companion could resolve to whichever system face happened to
        // be loaded by then rather than to the face this test adopts.
        let mut rasterizer = GlyphRasterizer::primary_only();
        let (kana, kana_weight) = decodable_kana_face(rasterizer.next_fallback_font_id());
        rasterizer.adopt_fallback(kana);
        let platform = platform_only_chinese_face(rasterizer.next_fallback_font_id());
        let platform_id = platform.id;
        rasterizer.adopt_fallback(platform);

        rasterizer.begin_script_run("你好吗", None);
        let han_only = rasterizer.glyph_key_for_codepoint('吗', 20.0);
        rasterizer.end_script_run();

        rasterizer.begin_script_run("には你好吗", None);
        let beside_kana = rasterizer.glyph_key_for_codepoint('吗', 20.0);
        rasterizer.end_script_run();

        assert_eq!(han_only.font_id, platform_id);
        assert_eq!(
            han_only.weight, kana_weight,
            "a han-only run must already sit on the companion weight"
        );
        assert_eq!(
            han_only.weight, beside_kana.weight,
            "typing kana beside the han must not change its weight"
        );
    }

    // Weight-aware fallback: a bold style must reach the platform face as a
    // bold instance, carried as the style's distance from normal on top of
    // the companion baseline — W3 kana beside bold text ask for 600, Apple's
    // own W6/Semibold pairing.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_bold_run_addresses_the_platform_face_at_a_bold_instance() {
        // The chain is built by hand: `GlyphRasterizer::new()` shares a
        // process-wide chain that a background warm-up fills asynchronously,
        // so the companion could resolve to whichever system face happened to
        // be loaded by then rather than to the face this test adopts.
        let mut rasterizer = GlyphRasterizer::primary_only();
        let (kana, kana_weight) = decodable_kana_face(rasterizer.next_fallback_font_id());
        rasterizer.adopt_fallback(kana);
        let platform = platform_only_chinese_face(rasterizer.next_fallback_font_id());
        rasterizer.adopt_fallback(platform);

        rasterizer.begin_script_run("には你好吗", None);
        let key = rasterizer.glyph_key_for_family_codepoint(
            '吗',
            20.0,
            FontFamily::SANS_SERIF,
            FontWeight::Bold,
            FontStyle::Normal,
        );
        rasterizer.end_script_run();

        let expected = kana_weight + (FontWeight::Bold.numeric() - FontWeight::Normal.numeric());
        assert_eq!(key.weight, expected);
    }

    // The reported defect: bold `你好` arrived at the regular stroke while
    // bold `你好吗` arrived bold, because only the platform-drawn `吗` was ever
    // told what weight the run asked for — the faces Cupid decodes itself
    // were chosen on coverage alone. Bold Han must reach a bolder stroke
    // whether the simplified-only character stands beside it or not.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_bold_han_run_is_drawn_bolder_whatever_stands_beside_it() {
        // A face Cupid decodes carries its stroke in its own design and is
        // keyed at the neutral weight; a face only the platform draws carries
        // it in the key instead. Both answer the same question.
        let stroke = |run: &str, weight: FontWeight| {
            let mut rasterizer = GlyphRasterizer::new();
            rasterizer.begin_script_run(run, None);
            let key = rasterizer.glyph_key_for_family_codepoint(
                '你',
                20.0,
                FontFamily::SANS_SERIF,
                weight,
                FontStyle::Normal,
            );
            rasterizer.end_script_run();
            if rasterizer.face_needs_platform_raster(key.font_id) {
                key.weight
            } else {
                rasterizer
                    .face_design_weight(key.font_id)
                    .unwrap_or(NORMAL_GLYPH_WEIGHT)
            }
        };

        let regular = stroke("你好", FontWeight::Normal);
        for run in ["你好", "你好吗"] {
            assert!(
                stroke(run, FontWeight::Bold) > regular,
                "bold {run:?} was drawn no bolder than regular text"
            );
        }
    }

    // A face only the platform can draw is variable — that is the whole reason
    // the owned outline decoder does not read it — so it is *rendered* at the instance
    // the key names and no design weight of its own can disqualify it. Judged
    // by the `OS/2` weight it does not publish, such a face was read as regular
    // and refused for a bold run, which handed the run back to whatever lighter
    // face already covered it.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_face_only_the_platform_draws_answers_any_requested_weight() {
        let mut rasterizer = GlyphRasterizer::new();
        let platform = platform_only_chinese_face(rasterizer.next_fallback_font_id());
        let font_id = platform.id;
        rasterizer.adopt_fallback(platform);

        for weight in [
            300,
            NORMAL_GLYPH_WEIGHT,
            600,
            FontWeight::Bold.numeric(),
            900,
        ] {
            assert!(
                rasterizer.face_matches_weight(font_id, weight),
                "a variable platform face was refused the weight {weight}"
            );
        }
    }

    // The reported defect in its final shape: bold `你好` came out at the light
    // stroke while bold `你好吗` came out bold, because `吗` is simplified-only.
    // With it in the run no Japanese face covers the script, the chain is empty
    // and the platform face is taken — drawn at the run's bold instance.
    // Without it the light Japanese cut covers the run, and the platform's bold
    // answer was refused for publishing no matching design weight, so the run
    // kept that light cut. One word must not change stroke because a character
    // was typed beside it.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_bold_han_run_keeps_one_stroke_however_much_of_it_a_light_face_covers() {
        // The chain is built by hand and system lookups stay off, so the
        // assertion is about Cupid's choice rather than about which cascade the
        // host machine's language prefers.
        let drawn_weight = |run: &str, weight: FontWeight| {
            let mut rasterizer = GlyphRasterizer::primary_only();
            let japanese = japanese_only_han_face(rasterizer.next_fallback_font_id());
            let japanese_id = japanese.id;
            rasterizer.adopt_fallback(japanese);
            let platform = platform_only_chinese_face(rasterizer.next_fallback_font_id());
            rasterizer.adopt_fallback(platform);

            rasterizer.begin_script_run(run, None);
            let key = rasterizer.glyph_key_for_family_codepoint(
                '好',
                20.0,
                FontFamily::SANS_SERIF,
                weight,
                FontStyle::Normal,
            );
            rasterizer.end_script_run();
            (rasterizer.drawn_weight(key), key.font_id == japanese_id)
        };

        let (light, on_japanese) = drawn_weight("好", FontWeight::Normal);
        assert!(
            on_japanese,
            "regular text must keep the decodable face covering it"
        );
        let (short_run, _) = drawn_weight("好", FontWeight::Bold);
        let (long_run, _) = drawn_weight("好吗", FontWeight::Bold);
        assert_eq!(
            short_run, long_run,
            "the run changed stroke because a simplified-only character was typed beside it"
        );
        assert!(
            short_run > light,
            "bold han was drawn at the light stroke of the face that happened to cover it"
        );
    }

    // Emphasis a face already carries must not be applied a second time. The
    // platform draws its private faces at the instance the key names, so a
    // bold run's ideograph arrives genuinely bold; stamping the pipeline's
    // synthetic stroke on top of it is what left `吗` heavier than the `你好`
    // beside it, which no reader sees at the regular weight because the
    // synthetic stroke only runs for bold.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_platform_glyph_drawn_bold_asks_for_no_synthetic_stroke() {
        let mut rasterizer = GlyphRasterizer::new();
        let platform = platform_only_chinese_face(rasterizer.next_fallback_font_id());
        let font_id = platform.id;
        let glyph_id = platform
            .glyph_index('吗')
            .expect("the platform face was chosen for this character");
        rasterizer.adopt_fallback(platform);

        let bold = FontWeight::Bold.numeric();
        let drawn_bold = GlyphKey::new(font_id, glyph_id, 20.0).weighted(bold);
        assert!(
            !rasterizer.glyph_needs_synthetic_bold(drawn_bold, bold),
            "a glyph the platform already drew bold was emboldened twice"
        );

        let drawn_regular = GlyphKey::new(font_id, glyph_id, 20.0).weighted(NORMAL_GLYPH_WEIGHT);
        assert!(
            rasterizer.glyph_needs_synthetic_bold(drawn_regular, bold),
            "a glyph drawn at the regular instance must still be emboldened"
        );
    }

    // The reported conflict end to end: a field typed on a Chinese keyboard
    // holds `你好`, every character of which Japanese writes too, so the run
    // was covered by the Japanese face the system prefers — and jumped to a
    // Chinese one the moment `吗`, written only in Chinese, was typed. Saying
    // which language the text is in must settle the face before the word is
    // finished, and settle it the same way whatever the reader types next.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_chinese_field_keeps_one_face_while_the_word_is_being_typed() {
        // The chain is built by hand, with a Japanese face ahead of the
        // Chinese one and system lookups off: that is the arrangement the
        // defect needs, and it holds whatever cascade the host machine's
        // language prefers.
        let face_of = |run: &str, language| {
            let mut rasterizer = GlyphRasterizer::primary_only();
            let japanese = japanese_only_han_face(rasterizer.next_fallback_font_id());
            let japanese_id = japanese.id;
            rasterizer.adopt_fallback(japanese);
            let chinese = platform_only_chinese_face(rasterizer.next_fallback_font_id());
            rasterizer.adopt_fallback(chinese);

            rasterizer.begin_script_run(run, language);
            let face = rasterizer.glyph_key_for_family_codepoint(
                '好',
                20.0,
                FontFamily::SANS_SERIF,
                FontWeight::Normal,
                FontStyle::Normal,
            );
            rasterizer.end_script_run();
            (face.font_id, japanese_id)
        };

        let chinese = Some(TextLanguage::Chinese);
        let (partial_word, japanese_id) = face_of("你好", chinese);
        let (whole_word, _) = face_of("你好吗", chinese);
        let (first_character, _) = face_of("你", chinese);
        assert_eq!(
            partial_word, whole_word,
            "the word changed typeface when the next character was typed"
        );
        assert_eq!(
            partial_word, first_character,
            "the very first character must already sit on the face of the word"
        );
        assert_ne!(
            partial_word, japanese_id,
            "a field declared chinese was drawn in the japanese face covering the run"
        );
    }

    // A Japanese field must not be dragged onto a Chinese face by the same
    // rule: kanji-only words are ordinary Japanese, and they stay on the face
    // the kana around them use.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_japanese_field_keeps_the_face_its_kana_use() {
        let face_of = |run: &str, language| {
            let mut rasterizer = GlyphRasterizer::primary_only();
            let japanese = japanese_only_han_face(rasterizer.next_fallback_font_id());
            rasterizer.adopt_fallback(japanese);
            let chinese = platform_only_chinese_face(rasterizer.next_fallback_font_id());
            rasterizer.adopt_fallback(chinese);

            rasterizer.begin_script_run(run, language);
            let face = rasterizer.glyph_key_for_family_codepoint(
                '日',
                20.0,
                FontFamily::SANS_SERIF,
                FontWeight::Normal,
                FontStyle::Normal,
            );
            rasterizer.end_script_run();
            face.font_id
        };

        let japanese = Some(TextLanguage::Japanese);
        assert_eq!(
            face_of("日本語", japanese),
            face_of("日本語の", japanese),
            "a kanji word changed typeface when kana were typed after it"
        );
    }

    // The reported defect, seen from the pipeline: at the regular weight every
    // character of a Chinese run rendered alike, and at bold they stopped
    // agreeing, because the faces behind them reach the requested stroke by
    // different routes. Whatever route each takes, the run must be emboldened
    // by hand as a whole or not at all.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_bold_han_run_is_emboldened_by_hand_as_a_whole() {
        let run = "你好吗";
        let mut rasterizer = GlyphRasterizer::new();
        rasterizer.begin_script_run(run, None);
        let strokes: Vec<(char, bool)> = run
            .chars()
            .map(|codepoint| {
                let key = rasterizer.glyph_key_for_family_codepoint(
                    codepoint,
                    20.0,
                    FontFamily::SANS_SERIF,
                    FontWeight::Bold,
                    FontStyle::Normal,
                );
                (
                    codepoint,
                    rasterizer.glyph_needs_synthetic_bold(key, FontWeight::Bold.numeric()),
                )
            })
            .collect();
        rasterizer.end_script_run();

        let (_, first) = strokes[0];
        assert!(
            strokes.iter().all(|(_, synthetic)| *synthetic == first),
            "the run was emboldened unevenly: {strokes:?}"
        );
    }

    // A static family carrying one design cannot answer a bold style on its own,
    // so the synthetic stroke stays: that is the only bold the family has.
    #[test]
    fn a_static_cut_still_asks_for_the_synthetic_stroke_when_bold() {
        let mut rasterizer = GlyphRasterizer::new();
        let key = rasterizer.glyph_key_for_family_codepoint(
            'A',
            20.0,
            FontFamily::MONOSPACE,
            FontWeight::Bold,
            FontStyle::Normal,
        );
        let design = rasterizer
            .face_design_weight(key.font_id)
            .unwrap_or(NORMAL_GLYPH_WEIGHT);
        assert!(
            design < BOLD_WEIGHT_THRESHOLD,
            "this test needs a regular primary face, got a face designed at {design}"
        );
        assert!(rasterizer.glyph_needs_synthetic_bold(key, FontWeight::Bold.numeric()));
    }

    #[test]
    fn bundled_primary_font_selects_requested_weight_variations() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let primary_id = rasterizer.primary_font_id();

        assert!(
            rasterizer.face_has_weight_variations(primary_id),
            "the bundled primary face must expose a readable wght axis"
        );
        rasterizer.ensure_aimer_font_cached(primary_id);
        let layout_result = rasterizer
            .aimer_font_cache
            .get(&primary_id)
            .and_then(|state| state.as_ref())
            .expect("the bundled primary face should be parsed")
            .prewarm_layout();
        assert!(
            layout_result.is_ok(),
            "the bundled primary face layout should parse: {layout_result:?}"
        );

        for weight in [FontWeight::Normal, FontWeight::Bold, FontWeight::Bolder] {
            let key = rasterizer.glyph_key_for_family_codepoint(
                'A',
                20.0,
                FontFamily::SANS_SERIF,
                weight,
                FontStyle::Normal,
            );
            assert_eq!(key.font_id, primary_id);
            assert_eq!(key.weight, weight.numeric());
            assert!(!rasterizer.glyph_needs_synthetic_bold(key, weight.numeric()));
        }

        let regular_t_key = rasterizer.glyph_key_for_family_codepoint(
            't',
            20.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        let regular_t = rasterizer.rasterize_key(regular_t_key, 20.0).bitmap.clone();
        let heavy_t_key = rasterizer.glyph_key_for_family_codepoint(
            't',
            20.0,
            FontFamily::SANS_SERIF,
            FontWeight::Bolder,
            FontStyle::Normal,
        );
        let heavy_t = rasterizer.rasterize_key(heavy_t_key, 20.0).bitmap.clone();
        assert_ne!(
            regular_t, heavy_t,
            "the composite t must rasterize its requested variable weight"
        );
        let ink = |bitmap: &[u8]| bitmap.iter().map(|pixel| u64::from(*pixel)).sum::<u64>();
        assert!(
            ink(&heavy_t) > ink(&regular_t),
            "the heavy t must deposit more ink than the regular t"
        );
    }

    // Below the threshold nothing is emphasized, so no glyph may be drawn
    // twice — the pass costs an instance per glyph and blurs the stroke.
    #[test]
    fn text_below_the_bold_threshold_is_never_drawn_twice() {
        let mut rasterizer = GlyphRasterizer::new();
        let key = rasterizer.glyph_key_for_family_codepoint(
            'A',
            20.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        for weight in [100, NORMAL_GLYPH_WEIGHT, BOLD_WEIGHT_THRESHOLD - 1] {
            assert!(
                !rasterizer.glyph_needs_synthetic_bold(key, weight),
                "unemphasized text at {weight} was drawn twice"
            );
        }
    }

    #[test]
    fn a_regular_fallback_cut_gets_a_small_normalization_stroke() {
        assert!(synthetic_weight_needed(NORMAL_GLYPH_WEIGHT, 300));
        assert!(!synthetic_weight_needed(
            NORMAL_GLYPH_WEIGHT,
            NORMAL_GLYPH_WEIGHT
        ));
        assert!(!synthetic_weight_needed(300, 400));

        let normal_offset = synthetic_weight_offset_for(20.0, NORMAL_GLYPH_WEIGHT, 300)
            .expect("a W3 fallback should receive a normal-weight correction");
        let bold_offset = synthetic_weight_offset_for(20.0, FontWeight::Bold.numeric(), 400)
            .expect("a regular face should receive a bold correction");
        assert!(normal_offset > 0.0);
        assert!(normal_offset < bold_offset);
    }

    #[test]
    fn myanmar_and_hangul_keep_the_requested_regular_weight() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let primary_id = rasterizer.primary_font_id();
        let fallback_id = primary_id.saturating_add(1);
        let key = GlyphKey::new(fallback_id, 1, 44.0).weighted(NORMAL_GLYPH_WEIGHT);

        for codepoint in ['မ', '한'] {
            assert!(
                rasterizer
                    .synthetic_weight_offset_for_codepoint(
                        key,
                        NORMAL_GLYPH_WEIGHT,
                        44.0,
                        codepoint,
                    )
                    .is_none(),
                "{codepoint:?} should not receive an extra regular-weight stroke"
            );

            let bold_plan = rasterizer
                .synthetic_weight_plan_for_codepoint(
                    key,
                    FontWeight::Bold.numeric(),
                    44.0,
                    codepoint,
                )
                .expect("bold fallback text should still receive synthetic emphasis");
            assert_eq!(bold_plan.extra_offsets().len(), 1);
        }

        let primary_key = GlyphKey::new(primary_id, 1, 44.0).weighted(NORMAL_GLYPH_WEIGHT);
        assert!(
            rasterizer
                .synthetic_weight_offset_for_codepoint(primary_key, NORMAL_GLYPH_WEIGHT, 44.0, 'မ',)
                .is_none(),
            "the regular correction must not duplicate the embedded primary face"
        );
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn installed_myanmar_and_hangul_fallbacks_use_only_needed_weight_adjustments() {
        let mut rasterizer = GlyphRasterizer::new();
        let primary_id = rasterizer.primary_font_id();
        rasterizer.begin_script_run("မြန်မာ 한글", None);

        for codepoint in ['မ', '한'] {
            let key = rasterizer.glyph_key_for_family_codepoint(
                codepoint,
                44.0,
                FontFamily::SANS_SERIF,
                FontWeight::Normal,
                FontStyle::Normal,
            );
            assert_ne!(
                key.font_id, primary_id,
                "{codepoint:?} must resolve to a fallback face"
            );
            assert!(
                !rasterizer.face_needs_platform_raster(key.font_id),
                "{codepoint:?} must use a readable fallback in the owned path"
            );
            let plan = rasterizer.synthetic_weight_plan_for_codepoint(
                key,
                NORMAL_GLYPH_WEIGHT,
                44.0,
                codepoint,
            );
            assert!(
                plan.is_none_or(|plan| plan.extra_offsets().len() == 1),
                "{codepoint:?} must not receive a special double-stroke regular correction"
            );
        }

        rasterizer.end_script_run();
    }

    // The bundled primary face is variable, so the glyph key must preserve
    // the requested OpenType weight for both shaping and rasterization.
    #[test]
    fn a_variable_primary_face_is_keyed_at_the_requested_weight() {
        let mut rasterizer = GlyphRasterizer::new();
        let key = rasterizer.glyph_key_for_family_codepoint(
            'A',
            20.0,
            FontFamily::SANS_SERIF,
            FontWeight::Bold,
            FontStyle::Normal,
        );
        assert_eq!(key.weight, FontWeight::Bold.numeric());
        assert!(!rasterizer.glyph_needs_synthetic_bold(key, FontWeight::Bold.numeric()));
    }

    #[test]
    fn an_owned_variable_face_key_tracks_the_requested_weight() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let font_id = rasterizer
            .register_font_bytes(
                include_bytes!("../../../fonts/NotoSansJP-VariableFont_wght.ttf").to_vec(),
            )
            .expect("the bundled variable CJK face should register");

        let regular = rasterizer.glyph_key_for_family_codepoint(
            '你',
            20.0,
            FontFamily::MONOSPACE,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        let bold = rasterizer.glyph_key_for_family_codepoint(
            '你',
            20.0,
            FontFamily::MONOSPACE,
            FontWeight::Bold,
            FontStyle::Normal,
        );

        assert_eq!(regular.font_id, font_id);
        assert_eq!(bold.font_id, font_id);
        assert_eq!(regular.weight, NORMAL_GLYPH_WEIGHT);
        assert_eq!(bold.weight, FontWeight::Bold.numeric());
        assert_ne!(regular, bold);
    }

    #[test]
    fn bitmap_cache_bytes_are_tracked_across_replace_and_release() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let key = GlyphKey::new(rasterizer.primary_font_id(), 1, 16.0);
        let glyph = |bytes| RasterizedGlyph {
            bitmap: vec![255; bytes],
            width: bytes as u32,
            height: 1,
            offset_x: 0.0,
            offset_y: 0.0,
            advance_width: 12.0,
            is_color: false,
        };

        rasterizer.commit_prepared_glyph(key, glyph(1024));
        assert_eq!(rasterizer.retained_bitmap_bytes, 1024);
        assert_eq!(rasterizer.bitmap_cache_bytes(), 1024);

        rasterizer.commit_prepared_glyph(key, glyph(256));
        assert_eq!(rasterizer.retained_bitmap_bytes, 256);
        assert_eq!(rasterizer.bitmap_cache_bytes(), 256);

        rasterizer.release_bitmap(key);
        assert_eq!(rasterizer.retained_bitmap_bytes, 0);
        assert_eq!(rasterizer.bitmap_cache_bytes(), 0);
    }

    #[test]
    fn shaping_already_measured_glyphs_does_not_rasterize_again() {
        // Shaping bakes each glyph's bitmap box into the shaped clusters, and
        // each shaping job runs in a freshly created worker context whose
        // bitmap cache starts empty. The box depends solely on the glyph key,
        // so a glyph measured once — by any earlier frame or sibling worker —
        // must never be rasterized again just to be measured.
        let mut renderer = GlyphRasterizer::new();
        let text = "Resize 你好 ជំរាបសួរ mixed العربية text";
        let shaped = shape_text_styled(
            &mut renderer,
            text,
            18.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        );
        let expected = layout_shaped_text(&shaped, 0.0, 0.0, 200.0);
        assert!(!expected.is_empty());

        let mut worker = GlyphPreparationContext::new(renderer.font_snapshot());
        worker.rasterizer_mut().reset_rasterize_call_count();
        let reshaped = shape_text_styled(
            worker.rasterizer_mut(),
            text,
            18.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        );

        assert_eq!(
            worker.rasterizer_mut().rasterize_call_count(),
            0,
            "a fresh worker must reuse published glyph metrics instead of rasterizing again"
        );
        let actual = layout_shaped_text(&reshaped, 0.0, 0.0, 200.0);
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_eq!(actual.glyph_key, expected.glyph_key);
            assert_eq!(actual.x, expected.x);
            assert_eq!(actual.y, expected.y);
            assert_eq!(actual.width, expected.width);
            assert_eq!(actual.height, expected.height);
        }
    }

    #[test]
    fn batched_metrics_match_scalar_metrics_and_preserve_duplicate_order() {
        let mut batched = GlyphRasterizer::new();
        let keys = "Aimer metrics A"
            .chars()
            .map(|codepoint| batched.glyph_key_for_codepoint(codepoint, 17.0))
            .collect::<Vec<_>>();

        let mut actual = Vec::with_capacity(keys.len());
        batched.with_metrics_for_keys(&keys, 17.0, |metrics| actual.push(metrics));

        let mut scalar = GlyphRasterizer::new();
        let expected = keys
            .iter()
            .copied()
            .map(|key| scalar.metrics_for_key(key, 17.0))
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }

    #[test]
    fn font_snapshot_is_naturally_send_and_sync() {
        assert_send_sync::<FontSnapshot>();
    }

    #[test]
    fn worker_context_starts_with_a_prewarmed_primary_aimer_face() {
        let mut renderer = GlyphRasterizer::new();
        let primary_id = renderer.primary_font_id();
        let snapshot = renderer.font_snapshot();
        let worker = GlyphPreparationContext::new(snapshot);

        assert!(
            worker
                .rasterizer
                .aimer_font_cache
                .get(&primary_id)
                .is_some_and(Option::is_some),
            "the worker snapshot should carry the primary parsed Aimer face"
        );
    }

    #[test]
    fn bundled_cjk_record_reuses_process_shared_font_bytes() {
        let mut first = GlyphRasterizer::new();
        first.begin_script_run("日本語", Some(TextLanguage::Japanese));
        first.ensure_bundled_cjk_fallback();
        first.end_script_run();
        let first_bytes = first
            .fallbacks
            .as_ref()
            .and_then(|fallbacks| {
                fallbacks
                    .iter()
                    .find(|record| record.id == BUNDLED_CJK_FONT_ID)
            })
            .and_then(|record| record.bytes.as_ref())
            .expect("the bundled Japanese face must be installed")
            .clone();

        let mut second = GlyphRasterizer::new();
        second.begin_script_run("日本語", Some(TextLanguage::Japanese));
        second.ensure_bundled_cjk_fallback();
        second.end_script_run();
        let second_bytes = second
            .fallbacks
            .as_ref()
            .and_then(|fallbacks| {
                fallbacks
                    .iter()
                    .find(|record| record.id == BUNDLED_CJK_FONT_ID)
            })
            .and_then(|record| record.bytes.as_ref())
            .expect("the bundled Japanese face must be installed")
            .clone();

        assert!(std::sync::Arc::ptr_eq(&first_bytes, &second_bytes));
    }

    #[test]
    fn worker_context_matches_shaping_and_layout_output() {
        let mut renderer = GlyphRasterizer::new();
        let mut worker = GlyphPreparationContext::new(renderer.font_snapshot());
        let text = "office text wraps";

        let expected_shaped = shape_text_styled(
            &mut renderer,
            text,
            18.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        );
        let actual_shaped = shape_text_styled(
            worker.rasterizer_mut(),
            text,
            18.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        );

        assert_eq!(actual_shaped.font_size, expected_shaped.font_size);
        assert_eq!(actual_shaped.line_height, expected_shaped.line_height);
        assert_eq!(actual_shaped.clusters.len(), expected_shaped.clusters.len());
        for (actual, expected) in actual_shaped.clusters.iter().zip(&expected_shaped.clusters) {
            assert_eq!(actual.text, expected.text);
            assert_eq!(actual.base_codepoint, expected.base_codepoint);
            assert_eq!(actual.width, expected.width);
            assert_eq!(actual.glyphs, expected.glyphs);
        }

        let expected_layout = layout_shaped_text(&expected_shaped, 0.0, 0.0, 80.0);
        let actual_layout = layout_shaped_text(&actual_shaped, 0.0, 0.0, 80.0);
        assert_eq!(actual_layout.len(), expected_layout.len());
        for (actual, expected) in actual_layout.iter().zip(&expected_layout) {
            assert!(actual.glyph_key == expected.glyph_key);
            assert_eq!((actual.x, actual.y), (expected.x, expected.y));
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height)
            );
            assert_eq!(actual.font_size, expected.font_size);
        }
    }

    /// The one glyph a worker prepares when handed a run holding only `key`.
    fn prepared_glyph(
        worker: &mut GlyphPreparationContext,
        key: GlyphKey,
        font_size: f32,
    ) -> RasterizedGlyph {
        let run = GlyphRun {
            font_size,
            keys: vec![key],
        };

        worker
            .prepare_glyph_run(&run)
            .pop()
            .expect("a run yields the glyph it was given")
            .1
    }

    #[test]
    fn worker_context_preserves_fallback_resolution() {
        let mut renderer = GlyphRasterizer::new();
        let mut worker = GlyphPreparationContext::new(renderer.font_snapshot());

        for codepoint in ['A', '界', '😀'] {
            let expected = renderer.glyph_key_for_codepoint(codepoint, 20.0);
            let actual = worker
                .rasterizer_mut()
                .glyph_key_for_codepoint(codepoint, 20.0);
            assert!(actual == expected);
        }
    }

    #[test]
    fn worker_context_returns_owned_alpha_glyph_with_matching_bitmap() {
        let mut renderer = GlyphRasterizer::new();
        let mut worker = GlyphPreparationContext::new(renderer.font_snapshot());
        let key = renderer.glyph_key_for_codepoint('A', 20.0);
        let expected = renderer.rasterize_key(key, 20.0).clone();
        let actual = prepared_glyph(&mut worker, key, 20.0);

        assert!(!actual.is_color);
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height)
        );
        assert_eq!(actual.bitmap, expected.bitmap);
    }

    #[test]
    fn worker_context_returns_owned_color_glyph_with_matching_dimensions() {
        let mut renderer = GlyphRasterizer::new();
        let mut worker = GlyphPreparationContext::new(renderer.font_snapshot());
        let key = renderer.glyph_key_for_codepoint('😀', 32.0);
        let expected = renderer.rasterize_key(key, 32.0).clone();
        let actual = prepared_glyph(&mut worker, key, 32.0);

        assert_eq!(actual.is_color, expected.is_color);
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height)
        );
        assert_eq!(actual.bitmap.len(), expected.bitmap.len());
        if expected.is_color {
            assert_eq!(actual.bitmap, expected.bitmap);
        }
    }

    #[test]
    fn worker_context_remains_usable_after_malformed_font_is_rejected() {
        let mut renderer = GlyphRasterizer::new();
        assert!(renderer.register_font_bytes(vec![0, 1, 2, 3]).is_none());

        let mut worker = GlyphPreparationContext::new(renderer.font_snapshot());
        let key = worker.rasterizer_mut().glyph_key_for_codepoint('A', 16.0);
        let glyph = prepared_glyph(&mut worker, key, 16.0);
        assert!(glyph.width > 0);
        assert!(glyph.height > 0);
        assert!(!glyph.bitmap.is_empty());
    }

    #[test]
    fn warm_renderer_cache_suppresses_worker_jobs_until_bitmap_is_needed() {
        let mut renderer = GlyphRasterizer::new();
        let key = renderer.glyph_key_for_codepoint('A', 16.0);
        let glyph = renderer.rasterize_key(key, 16.0).clone();

        assert!(!renderer.needs_prepared_glyph(key, true));
        renderer.commit_prepared_glyph(key, glyph);
        renderer.release_bitmap(key);

        assert!(!renderer.needs_prepared_glyph(key, false));
        assert!(renderer.needs_prepared_glyph(key, true));
    }

    #[test]
    fn repeated_glyph_resolution_memoizes_font_cmap_lookup() {
        let mut rasterizer = GlyphRasterizer::primary_only();

        let first = rasterizer.glyph_key_for_codepoint('A', 16.0);
        let second = rasterizer.glyph_key_for_codepoint('A', 24.0);
        assert_eq!(
            (first.font_id, first.glyph_id),
            (second.font_id, second.glyph_id)
        );
        assert_eq!(rasterizer.glyph_index_cache_len(), 1);

        let first_miss = rasterizer.glyph_key_for_codepoint('\u{10ffff}', 16.0);
        let second_miss = rasterizer.glyph_key_for_codepoint('\u{10ffff}', 24.0);
        assert_eq!(
            (first_miss.font_id, first_miss.glyph_id),
            (second_miss.font_id, second_miss.glyph_id)
        );
        assert_eq!(rasterizer.glyph_index_cache_len(), 2);

        assert!(
            rasterizer
                .register_font_bytes(PRIMARY_FONT.to_vec())
                .is_some()
        );
        assert_eq!(rasterizer.glyph_index_cache_len(), 0);
    }

    #[test]
    fn coverage_index_memoizes_glyph_and_script_answers_per_face() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let font_id = rasterizer.primary.id;
        let requirement = ScriptRequirement::probes('你');

        assert!(!rasterizer.font_covers_script(font_id, requirement));
        let first = rasterizer
            .coverage_index_cache
            .get(&font_id)
            .expect("the first coverage query must create a face index");
        assert!(!first.glyphs.is_empty());
        assert!(first.glyphs.len() <= requirement.as_slice().len());
        assert_eq!(first.scripts.len(), 1);
        let first_glyph_count = first.glyphs.len();

        let _ = rasterizer.glyph_index_for_font(font_id, 'A');
        let glyph_count = rasterizer
            .coverage_index_cache
            .get(&font_id)
            .expect("the face index must remain attached to its font")
            .glyphs
            .len();
        assert_eq!(glyph_count, first_glyph_count + 1);

        assert!(!rasterizer.font_covers_script(font_id, requirement));
        let second = rasterizer
            .coverage_index_cache
            .get(&font_id)
            .expect("the repeated query must reuse the face index");
        assert_eq!(second.glyphs.len(), glyph_count);
        assert_eq!(second.scripts.len(), 1);
    }

    #[test]
    fn batched_font_rasterization_matches_individual_rasterization() {
        let text = "AaVv";
        let mut batched = GlyphRasterizer::primary_only();
        let actual = batched.preload_text(text, 16.0, None);

        let mut individual = GlyphRasterizer::primary_only();
        let expected = text
            .chars()
            .map(|codepoint| {
                let key = individual.glyph_key_for_codepoint(codepoint, 16.0);
                (key, individual.rasterize_key(key, 16.0).clone())
            })
            .collect::<Vec<_>>();

        assert_eq!(actual.len(), expected.len());
        for ((actual_key, actual_glyph), (expected_key, expected_glyph)) in
            actual.iter().zip(expected.iter())
        {
            assert_eq!(actual_key, expected_key);
            assert_eq!(actual_glyph.bitmap, expected_glyph.bitmap);
            assert_eq!(actual_glyph.width, expected_glyph.width);
            assert_eq!(actual_glyph.height, expected_glyph.height);
            assert_eq!(actual_glyph.offset_x, expected_glyph.offset_x);
            assert_eq!(actual_glyph.offset_y, expected_glyph.offset_y);
            assert_eq!(actual_glyph.advance_width, expected_glyph.advance_width);
            assert_eq!(actual_glyph.is_color, expected_glyph.is_color);
        }
    }

    #[test]
    fn streamed_preload_emits_the_same_glyphs_as_owned_preload() {
        let text = "AaVv";
        let mut streamed = GlyphRasterizer::primary_only();
        let mut streamed_glyphs = Vec::new();
        streamed.preload_text_into(text, 16.0, None, |key, glyph| {
            streamed_glyphs.push((key, glyph.clone()));
        });

        let mut owned = GlyphRasterizer::primary_only();
        let owned_glyphs = owned.preload_text(text, 16.0, None);

        assert_eq!(streamed_glyphs.len(), owned_glyphs.len());
        for ((streamed_key, streamed_glyph), (owned_key, owned_glyph)) in
            streamed_glyphs.iter().zip(owned_glyphs.iter())
        {
            assert_eq!(streamed_key, owned_key);
            assert_eq!(streamed_glyph.bitmap, owned_glyph.bitmap);
            assert_eq!(streamed_glyph.width, owned_glyph.width);
            assert_eq!(streamed_glyph.height, owned_glyph.height);
            assert_eq!(streamed_glyph.offset_x, owned_glyph.offset_x);
            assert_eq!(streamed_glyph.offset_y, owned_glyph.offset_y);
            assert_eq!(streamed_glyph.advance_width, owned_glyph.advance_width);
            assert_eq!(streamed_glyph.is_color, owned_glyph.is_color);
        }
    }

    #[cfg(all(any(
        not(any(target_os = "ios", target_os = "macos")),
        feature = "apple-core-text"
    )))]
    #[test]
    fn streamed_preload_keeps_mixed_faces_in_separate_runs() {
        let mut rasterizer = GlyphRasterizer::new();
        let glyphs = rasterizer.preload_text("A你B", 16.0, None);

        assert_eq!(glyphs.len(), 3);
        assert_eq!(glyphs[0].0.font_id, rasterizer.primary_font_id());
        assert_ne!(glyphs[1].0.font_id, rasterizer.primary_font_id());
        assert_eq!(glyphs[2].0.font_id, rasterizer.primary_font_id());
        assert!(glyphs.iter().all(|(_, glyph)| glyph.advance_width > 0.0));
    }

    #[test]
    fn registered_families_resolve_consistently_across_rasterizers() {
        let family = FontRegistry::register(FontRegistration {
            family: "cupid-family-resolution-test",
            bytes: PRIMARY_FONT,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .unwrap();
        FontRegistry::register(FontRegistration {
            family: "cupid-family-resolution-test",
            bytes: PRIMARY_FONT,
            weight: FontWeight::Bold,
            style: FontStyle::Normal,
        })
        .unwrap();
        FontRegistry::register(FontRegistration {
            family: "cupid-family-resolution-test",
            bytes: PRIMARY_FONT,
            weight: FontWeight::Normal,
            style: FontStyle::Italic,
        })
        .unwrap();

        let mut first = GlyphRasterizer::new();
        let mut second = GlyphRasterizer::new();
        let normal = first.font_id_for_family(family, FontWeight::Normal, FontStyle::Normal);
        let bold = first.font_id_for_family(family, FontWeight::Bold, FontStyle::Normal);
        let nearest_bold =
            first.font_id_for_family(family, FontWeight::Value(600), FontStyle::Normal);
        let italic = first.font_id_for_family(family, FontWeight::Bold, FontStyle::Italic);

        assert_ne!(normal, bold);
        assert_eq!(nearest_bold, bold);
        assert_ne!(italic, bold);
        assert_eq!(
            second.font_id_for_family(family, FontWeight::Bold, FontStyle::Normal),
            bold
        );

        let key = first.glyph_key_for_family_codepoint(
            'A',
            16.0,
            family,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        assert_eq!(key.font_id, normal);
    }

    #[test]
    fn a_family_registered_after_rasterizer_creation_is_seen_on_first_lookup() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let family = FontRegistry::register(FontRegistration {
            family: "cupid-late-family-registration-test",
            bytes: PRIMARY_FONT,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .unwrap();
        let registered_id = FontRegistry::resolve(family, FontWeight::Normal, FontStyle::Normal)
            .expect("the registered face should be resolvable")
            .face_id;

        let key = rasterizer.glyph_key_for_family_codepoint(
            'A',
            16.0,
            family,
            FontWeight::Normal,
            FontStyle::Normal,
        );

        assert_eq!(
            key.font_id, registered_id,
            "family lookup must observe a deterministic registration made after construction"
        );
    }

    #[test]
    fn registry_replacement_invalidates_all_face_derived_caches() {
        let family = FontRegistry::register(FontRegistration {
            family: "cupid-face-cache-replacement-test",
            bytes: include_bytes!("../../../fonts/JetBrainsMono-Regular.ttf"),
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .expect("the original face should register");
        let replacement = include_bytes!("../../../fonts/GoogleSans-Regular.ttf");
        let mut rasterizer = GlyphRasterizer::primary_only();
        let key = rasterizer.glyph_key_for_family_codepoint(
            'A',
            16.0,
            family,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        let font_id = key.font_id;
        let original_bitmap = rasterizer.rasterize_key(key, 16.0).bitmap.clone();
        let _ = rasterizer.glyph_index_for_font(font_id, 'A');
        let _ = rasterizer.font_covers_script(font_id, ScriptRequirement::probes('你'));
        let _ = rasterizer.face_needs_platform_raster(font_id);
        let _ = rasterizer.face_design_weight(font_id);
        let _ = rasterizer.shape_cluster_for_family(
            "A",
            16.0,
            family,
            FontWeight::Normal,
            FontStyle::Normal,
        );

        assert!(
            rasterizer
                .cache
                .keys()
                .any(|cached| cached.font_id == font_id)
        );
        assert!(glyph_metrics::cached(key).is_some());
        assert!(
            rasterizer
                .advance_cache
                .keys()
                .any(|cached| cached.font_id == font_id)
        );
        assert!(
            rasterizer
                .coverage_index_cache
                .get(&font_id)
                .is_some_and(|index| !index.glyphs.is_empty() && !index.scripts.is_empty())
        );
        assert!(rasterizer.platform_only_cache.contains_key(&font_id));
        assert!(rasterizer.design_weight_cache.contains_key(&font_id));
        assert!(rasterizer.font_bytes_cache.contains_key(&font_id));
        assert!(rasterizer.aimer_font_cache.contains_key(&font_id));

        FontRegistry::replace(FontRegistration {
            family: "cupid-face-cache-replacement-test",
            bytes: replacement,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .expect("the replacement face should register");

        rasterizer.refresh_registered_family_faces();
        assert!(
            rasterizer
                .cache
                .keys()
                .all(|cached| cached.font_id != font_id)
        );
        assert!(glyph_metrics::cached(key).is_none());
        assert!(
            rasterizer
                .advance_cache
                .keys()
                .all(|cached| cached.font_id != font_id)
        );
        assert!(rasterizer.coverage_index_cache.get(&font_id).is_none());
        assert!(!rasterizer.platform_only_cache.contains_key(&font_id));
        assert!(!rasterizer.design_weight_cache.contains_key(&font_id));
        assert!(!rasterizer.font_bytes_cache.contains_key(&font_id));
        assert!(!rasterizer.aimer_font_cache.contains_key(&font_id));

        let replacement_key = rasterizer.glyph_key_for_family_codepoint(
            'A',
            16.0,
            family,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        assert_eq!(replacement_key.font_id, font_id);

        rasterizer.rasterize_key(replacement_key, 16.0);
        assert!(rasterizer.cache.contains_key(&replacement_key));
        assert_ne!(
            rasterizer
                .cache
                .get(&replacement_key)
                .expect("replacement glyph should be cached")
                .bitmap,
            original_bitmap,
            "replacement bytes must be used after invalidation"
        );
        assert!(FontRegistry::remove(
            family,
            FontWeight::Normal,
            FontStyle::Normal
        ));
    }

    #[test]
    fn registry_removal_invalidates_cached_face_and_falls_back_to_primary() {
        let family = FontRegistry::register(FontRegistration {
            family: "cupid-face-cache-removal-test",
            bytes: include_bytes!("../../../fonts/JetBrainsMono-Regular.ttf"),
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .expect("the face should register");
        let mut rasterizer = GlyphRasterizer::primary_only();
        let key = rasterizer.glyph_key_for_family_codepoint(
            'A',
            16.0,
            family,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        let font_id = key.font_id;
        rasterizer.rasterize_key(key, 16.0);
        assert!(
            rasterizer
                .cache
                .keys()
                .any(|cached| cached.font_id == font_id)
        );

        assert!(FontRegistry::remove(
            family,
            FontWeight::Normal,
            FontStyle::Normal
        ));

        let replacement_key = rasterizer.glyph_key_for_family_codepoint(
            'A',
            16.0,
            family,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        assert_eq!(replacement_key.font_id, rasterizer.primary_font_id());
        assert!(
            rasterizer
                .cache
                .keys()
                .all(|cached| cached.font_id != font_id)
        );
    }

    #[test]
    fn generic_families_are_distinct_and_renderable() {
        let mut rasterizer = GlyphRasterizer::new();
        let sans = rasterizer.font_id_for_family(
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        let monospace = rasterizer.font_id_for_family(
            FontFamily::MONOSPACE,
            FontWeight::Normal,
            FontStyle::Normal,
        );

        assert_eq!(sans, rasterizer.primary_font_id());
        assert_ne!(monospace, sans);
        let key = rasterizer.glyph_key_for_family_codepoint(
            'M',
            16.0,
            FontFamily::MONOSPACE,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        assert_eq!(key.font_id, monospace);
        assert!(rasterizer.rasterize_key(key, 16.0).width > 0);
    }

    #[test]
    fn rasterizing_a_known_face_does_not_build_the_fallback_chain() {
        let mut rasterizer = GlyphRasterizer::new();
        let key = rasterizer.glyph_key_for_family_codepoint(
            'M',
            16.0,
            FontFamily::MONOSPACE,
            FontWeight::Normal,
            FontStyle::Normal,
        );

        rasterizer.rasterize_key(key, 16.0);

        assert!(
            rasterizer.fallbacks.is_none(),
            "a face the rasterizer already owns needs no system fallback chain"
        );
    }

    #[test]
    fn family_measurement_and_shaping_use_the_same_face() {
        let mut rasterizer = GlyphRasterizer::new();
        let mono_i = rasterizer.advance_width_for_family(
            'i',
            20.0,
            FontFamily::MONOSPACE,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        let mono_m = rasterizer.advance_width_for_family(
            'M',
            20.0,
            FontFamily::MONOSPACE,
            FontWeight::Normal,
            FontStyle::Normal,
        );
        assert!((mono_i - mono_m).abs() < 0.001);

        let measured = rasterizer.measure_text_for_family(
            "Mi",
            20.0,
            FontFamily::MONOSPACE,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        );
        let shaped_width: f32 = rasterizer
            .shape_cluster_for_family(
                "Mi",
                20.0,
                FontFamily::MONOSPACE,
                FontWeight::Normal,
                FontStyle::Normal,
            )
            .iter()
            .map(|(_, advance, _, _)| advance)
            .sum();
        assert!((measured - shaped_width).abs() < 0.001);
    }

    #[test]
    fn selected_family_missing_glyph_uses_existing_unicode_fallback_chain() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let fallback_id = rasterizer
            .register_font_bytes(
                include_bytes!("../../../fonts/NotoSansJP-VariableFont_wght.ttf").to_vec(),
            )
            .expect("the bundled CJK fallback should register");

        let key = rasterizer.glyph_key_for_family_codepoint(
            '你',
            16.0,
            FontFamily::MONOSPACE,
            FontWeight::Normal,
            FontStyle::Normal,
        );

        assert_eq!(key.font_id, fallback_id);
        assert!(rasterizer.rasterize_key(key, 16.0).width > 0);
    }

    #[test]
    fn primary_font_bytes_are_shared_between_rasterizers() {
        let first = GlyphRasterizer::new();
        let second = GlyphRasterizer::primary_only();

        assert!(Arc::ptr_eq(
            first.primary.bytes.as_ref().expect("primary bytes missing"),
            second
                .primary
                .bytes
                .as_ref()
                .expect("primary bytes missing")
        ));
    }

    #[test]
    fn register_font_bytes_adds_in_memory_fallback() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let bytes = PRIMARY_FONT.to_vec();

        let font_id = rasterizer
            .register_font_bytes(bytes)
            .expect("embedded font bytes should register");

        assert_ne!(font_id, rasterizer.primary_font_id());
        let fallbacks = rasterizer
            .fallbacks
            .as_ref()
            .expect("registered fallback missing");
        let registered = fallbacks
            .iter()
            .find(|record| record.id == font_id)
            .expect("registered font record missing");
        assert!(registered.bytes.is_some());
        assert!(registered.glyph_index('A').is_some());
    }

    #[test]
    fn registering_a_face_does_not_warm_unrequested_fallback_lanes() {
        let mut rasterizer = GlyphRasterizer::new();

        rasterizer
            .register_font_bytes(PRIMARY_FONT.to_vec())
            .expect("embedded font bytes should register");

        assert!(!rasterizer.system_fallbacks_loaded);
        assert!(rasterizer.loaded_fallback_scripts.is_empty());
    }

    #[test]
    fn latin_lookup_does_not_load_fallbacks() {
        let mut rasterizer = GlyphRasterizer::new();

        for c in "Hello from Cupid!".chars() {
            rasterizer.glyph_key_for_codepoint(c, 32.0);
        }

        assert!(rasterizer.fallbacks.is_none());
        assert!(rasterizer.unsupported_codepoints.is_empty());
    }

    #[test]
    fn fallback_miss_loads_only_the_requested_script_lane() {
        let mut rasterizer = GlyphRasterizer::new();

        rasterizer.ensure_fallbacks_for_codepoint('😀');

        assert!(!rasterizer.system_fallbacks_loaded);
        assert!(
            rasterizer
                .loaded_fallback_scripts
                .contains(&FallbackScript::Emoji)
        );
        assert!(
            !rasterizer
                .loaded_fallback_scripts
                .contains(&FallbackScript::Cjk)
        );
        assert!(
            !rasterizer
                .loaded_fallback_scripts
                .contains(&FallbackScript::Arabic)
        );
    }

    #[test]
    fn releasing_fallbacks_forgets_loaded_script_lanes() {
        let mut rasterizer = GlyphRasterizer::new();
        rasterizer.ensure_fallbacks_for_codepoint('😀');
        rasterizer.ensure_fallbacks_for_codepoint('你');

        assert!(
            rasterizer
                .loaded_fallback_scripts
                .contains(&FallbackScript::Emoji)
        );
        assert!(
            rasterizer
                .loaded_fallback_scripts
                .contains(&FallbackScript::Cjk)
        );

        rasterizer.release_fallbacks();

        assert!(rasterizer.loaded_fallback_scripts.is_empty());
        assert!(!rasterizer.system_fallbacks_loaded);
    }

    #[test]
    fn aimer_font_loads_bundled_cjk_fallback_only_on_cjk_miss() {
        const EXPECTED_BUNDLED_CJK_FONT_ID: FontId = 0x2000_0000;

        let mut rasterizer = GlyphRasterizer::new();
        assert!(rasterizer.fallbacks.is_none());

        let latin = rasterizer.glyph_key_for_codepoint('A', 32.0);
        assert_eq!(latin.font_id, rasterizer.primary_font_id());
        assert!(rasterizer.fallbacks.is_none());

        rasterizer.begin_script_run("あの時は", Some(TextLanguage::Japanese));
        let cjk = rasterizer.glyph_key_for_codepoint('時', 32.0);
        rasterizer.end_script_run();
        assert_eq!(cjk.font_id, EXPECTED_BUNDLED_CJK_FONT_ID);
        assert!(rasterizer.fallbacks.as_ref().is_some_and(|fallbacks| {
            fallbacks
                .iter()
                .any(|record| record.id == EXPECTED_BUNDLED_CJK_FONT_ID)
        }));
    }

    #[test]
    fn aimer_font_honors_an_explicit_japanese_language_for_han_only_runs() {
        const EXPECTED_BUNDLED_CJK_FONT_ID: FontId = 0x2000_0000;

        let mut rasterizer = GlyphRasterizer::new();
        rasterizer.begin_script_run("漢字", Some(TextLanguage::Japanese));
        rasterizer.ensure_bundled_cjk_fallback();
        let (font_id, _, supported) =
            rasterizer.font_and_glyph_for_codepoint('漢', NORMAL_GLYPH_WEIGHT);
        rasterizer.end_script_run();

        assert!(supported);
        assert_eq!(font_id, EXPECTED_BUNDLED_CJK_FONT_ID);
    }

    #[test]
    fn aimer_font_uses_the_japanese_bundle_for_kana_only_runs() {
        const EXPECTED_BUNDLED_CJK_FONT_ID: FontId = 0x2000_0000;

        let mut rasterizer = GlyphRasterizer::new();
        rasterizer.begin_script_run("かな", None);
        let key = rasterizer.glyph_key_for_codepoint('か', 32.0);
        rasterizer.end_script_run();

        assert_eq!(key.font_id, EXPECTED_BUNDLED_CJK_FONT_ID);
    }

    #[test]
    fn aimer_font_does_not_use_japanese_bundle_for_chinese_or_korean_runs() {
        const BUNDLED_CJK_FONT_ID: FontId = 0x2000_0000;

        let mut rasterizer = GlyphRasterizer::new();
        rasterizer.begin_script_run("你好", Some(TextLanguage::Chinese));
        rasterizer.ensure_fallbacks_for_codepoint('你');
        rasterizer.end_script_run();

        assert!(!rasterizer.fallbacks.as_ref().is_some_and(|fallbacks| {
            fallbacks
                .iter()
                .any(|record| record.id == BUNDLED_CJK_FONT_ID)
        }));
        assert!(
            rasterizer
                .loaded_fallback_scripts
                .contains(&FallbackScript::Cjk)
        );

        rasterizer.begin_script_run("한글", Some(TextLanguage::Korean));
        rasterizer.ensure_fallbacks_for_codepoint('한');
        rasterizer.end_script_run();

        assert!(!rasterizer.fallbacks.as_ref().is_some_and(|fallbacks| {
            fallbacks
                .iter()
                .any(|record| record.id == BUNDLED_CJK_FONT_ID)
        }));
        assert!(
            rasterizer
                .loaded_fallback_scripts
                .contains(&FallbackScript::Hangul)
        );
    }

    #[test]
    fn released_cjk_fallback_reloads_with_the_same_id() {
        const EXPECTED_BUNDLED_CJK_FONT_ID: FontId = 0x2000_0000;

        let mut rasterizer = GlyphRasterizer::new();
        rasterizer.begin_script_run("あの時は", Some(TextLanguage::Japanese));
        let first = rasterizer.glyph_key_for_codepoint('時', 32.0);
        rasterizer.end_script_run();

        assert_eq!(first.font_id, EXPECTED_BUNDLED_CJK_FONT_ID);
        let first_glyph = rasterizer.rasterize_key(first, 32.0).clone();
        assert!(!first_glyph.bitmap.is_empty());
        assert_eq!(rasterizer.cached_glyph_count(), 1);
        assert_eq!(rasterizer.release_fallbacks(), 1);
        assert!(rasterizer.fallbacks.is_none());
        assert_eq!(rasterizer.cached_glyph_count(), 0);
        assert_eq!(rasterizer.bitmap_cache_bytes(), 0);

        rasterizer.begin_script_run("あの時は", Some(TextLanguage::Japanese));
        let reloaded = rasterizer.glyph_key_for_codepoint('時', 32.0);
        rasterizer.end_script_run();

        assert_eq!(reloaded.font_id, first.font_id);
        assert_eq!(reloaded.glyph_id, first.glyph_id);
    }

    #[test]
    fn fallback_release_keeps_explicitly_registered_faces() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let registered_id = rasterizer
            .register_font_bytes(PRIMARY_FONT.to_vec())
            .expect("the registered face should be readable");

        assert_eq!(rasterizer.release_fallbacks(), 0);
        let registered = rasterizer
            .fallbacks
            .as_ref()
            .and_then(|fallbacks| fallbacks.iter().find(|record| record.id == registered_id))
            .expect("explicit registrations must survive fallback release");
        assert!(registered.glyph_index('A').is_some());
    }

    #[test]
    fn preload_text_is_idempotent_for_cached_glyphs() {
        let mut rasterizer = GlyphRasterizer::new();

        rasterizer.preload_text("Hello", 16.0, None);
        let cache_len = rasterizer.cache.len();
        let advance_cache_len = rasterizer.advance_cache.len();

        rasterizer.preload_text("Hello", 16.0, None);

        assert_eq!(rasterizer.cache.len(), cache_len);
        assert_eq!(rasterizer.advance_cache.len(), advance_cache_len);
        assert!(rasterizer.fallbacks.is_none());
    }

    /// macOS ships AppleColorEmoji at
    /// /System/Library/Fonts/AppleColorEmoji.ttc. On a system without that
    /// font (or in CI containers), the chain just won't contain it; the
    /// test stays informative either way by asserting *if* the
    /// font was loaded, the record is correctly tagged as color.
    // #[test]
    #[allow(dead_code)]
    fn khmer_glyphs_use_renderable_fallback_font() {
        let mut rasterizer = GlyphRasterizer::new();

        // ក ខ គ are basic Khmer consonants that must be present in any Khmer font.
        for c in "កខគ".chars() {
            let key = rasterizer.glyph_key_for_codepoint(c, 16.0);
            assert_ne!(
                key.font_id,
                rasterizer.primary_font_id(),
                "U+{:04X} {} should use a Khmer fallback font, not the primary",
                c as u32,
                c
            );

            let glyph = rasterizer.glyph_metrics_for_key(key, 16.0);
            assert!(
                glyph.width > 0,
                "U+{:04X} {} Khmer glyph should have bitmap width > 0",
                c as u32,
                c
            );
            assert!(
                glyph.height > 0,
                "U+{:04X} {} Khmer glyph should have bitmap height > 0",
                c as u32,
                c
            );
            assert!(
                !glyph.bitmap.is_empty(),
                "U+{:04X} {} Khmer glyph bitmap must not be empty",
                c as u32,
                c
            );
            assert!(
                !glyph.is_color,
                "U+{:04X} {} Khmer glyph should be monochrome",
                c as u32, c
            );
        }
    }

    /// Verify that `shape_cluster` handles Khmer subscript clusters (base +
    /// COENG + subscript) as a single shaped unit, producing exactly one
    /// visible glyph (the ligature) rather than three separate
    /// mispositioned glyphs for each codepoint.
    // #[test]
    #[allow(dead_code)]
    fn khmer_coeng_cluster_shapes_as_ligature() {
        let mut rasterizer = GlyphRasterizer::new();

        // "ក្ត" = ក (U+1780) + ្ (U+17D2 COENG) + ត (U+178F)
        // With proper shaping this should produce 1 ligature glyph, not 3 separate
        // glyphs.
        let cluster = "ក្ត";
        let shaped = rasterizer.shape_cluster(cluster, 16.0);

        // A Khmer font is required for this test.
        if shaped.is_empty() {
            eprintln!("[note] No Khmer fallback font found — skipping coeng cluster test");
            return;
        }

        // The shaped output should have fewer glyphs than codepoints (3).
        // The platform reference for Khmer Sangam MN produces 2 glyphs for this cluster:
        // one for the base consonant with full advance, one zero-advance mark
        // (subscript).
        assert!(
            shaped.len() < cluster.chars().count(),
            "Khmer COENG cluster should produce fewer glyphs than codepoints; got {} shaped glyphs for {} codepoints",
            shaped.len(),
            cluster.chars().count()
        );

        // Each shaped glyph must use the Khmer fallback font (not the primary).
        for (key, _, _, _) in &shaped {
            assert_ne!(
                key.font_id,
                rasterizer.primary_font_id(),
                "Khmer cluster glyph must use a fallback font, not the primary"
            );
        }

        // Every shaped glyph must rasterize to a non-empty bitmap.
        for (key, _, _, _) in shaped {
            let glyph = rasterizer.rasterize_key(key, 16.0);
            assert!(
                glyph.width > 0 && glyph.height > 0 && !glyph.bitmap.is_empty(),
                "Shaped Khmer glyph (id={}) must have a renderable bitmap",
                key.glyph_id
            );
        }
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn fallback_chain_keeps_both_emoji_and_cjk() {
        let mut rasterizer = GlyphRasterizer::new();

        // Emoji and CJK are served by different files, so resolving both must
        // grow the chain rather than replace its single entry: the color face
        // may not answer for Han, and the Han face has no emoji strikes.
        let emoji = rasterizer.glyph_key_for_codepoint('😀', 32.0);
        let han = rasterizer.glyph_key_for_codepoint('漢', 32.0);

        assert_ne!(emoji.font_id, rasterizer.primary_font_id());
        assert_ne!(han.font_id, rasterizer.primary_font_id());
        assert_ne!(
            emoji.font_id, han.font_id,
            "emoji and CJK must resolve to different faces"
        );

        let chain = rasterizer.fallbacks.as_ref().expect("chain was populated");
        assert!(chain.iter().any(|fallback| fallback.is_color));
        assert!(chain.iter().any(|fallback| !fallback.is_color));
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn emoji_glyph_rasterizes_as_color() {
        let mut rasterizer = GlyphRasterizer::new();

        let key = rasterizer.glyph_key_for_codepoint('😀', 32.0);
        assert_ne!(
            key.font_id,
            rasterizer.primary_font_id(),
            "'😀' must resolve to a color face, not to the Latin primary font"
        );

        let glyph = rasterizer.glyph_metrics_for_key(key, 32.0);
        assert!(glyph.is_color, "'😀' should be tagged as a color glyph");
        assert!(
            glyph.width > 0 && glyph.height > 0,
            "'😀' bitmap dimensions must be non-zero"
        );
        // RGBA8 → 4 bytes per pixel.
        assert_eq!(
            glyph.bitmap.len(),
            (glyph.width * glyph.height * 4) as usize,
            "'😀' bitmap must be RGBA8 (4 bytes per pixel)"
        );
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn apple_color_symbol_glyphs_keep_their_bitmap_near_the_text_baseline() {
        let mut rasterizer = GlyphRasterizer::new();

        for codepoint in ['♿', '☑', '↔', '⌨'] {
            let key = rasterizer.glyph_key_for_codepoint(codepoint, 18.0);
            let glyph = rasterizer.glyph_metrics_for_key(key, 18.0);
            assert!(glyph.is_color, "{codepoint:?} should use a color fallback");
            assert!(glyph.width > 0 && glyph.height > 0);

            // In the shared y-down layout contract the bitmap bottom is
            // `baseline - offset_y`. Apple Color Emoji's zero sbix origin is
            // a sentinel, not a bottom edge; accepting the old -height value
            // puts the entire symbol below the baseline and outside a row.
            assert!(
                glyph.offset_y > -4.0 && glyph.offset_y < 4.0,
                "{codepoint:?} bitmap bottom drifted from the baseline: offset_y={}",
                glyph.offset_y
            );
        }
    }

    /// Simplified Chinese is the hardest fallback case on Apple platforms:
    /// characters used only there — `吗`, `们`, `这` — are missing from the
    /// Japanese and Korean faces that happen to cover shared ideographs, so the
    /// only face left is the system's own Chinese font, whose outlines live in
    /// a private table the owned outline decoder cannot read.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn simplified_chinese_only_codepoints_rasterize_to_visible_glyphs() {
        let mut rasterizer = GlyphRasterizer::new();

        for codepoint in ['吗', '们', '这'] {
            let key = rasterizer.glyph_key_for_codepoint(codepoint, 32.0);
            assert_ne!(
                key.font_id,
                rasterizer.primary_font_id(),
                "{codepoint:?} must resolve to a system face"
            );
            assert_ne!(key.glyph_id, 0, "{codepoint:?} resolved to .notdef");

            let glyph = rasterizer.rasterize_key(key, 32.0);
            assert!(
                glyph.width > 0 && glyph.height > 0 && !glyph.bitmap.is_empty(),
                "{codepoint:?} rasterized to nothing ({}x{}, {} bytes)",
                glyph.width,
                glyph.height,
                glyph.bitmap.len()
            );
        }
    }

    #[test]
    fn uploaded_glyph_bitmap_can_be_released_without_losing_metrics() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let key = rasterizer.glyph_key_for_codepoint('A', 24.0);
        let expected = rasterizer.rasterize_key(key, 24.0).clone();
        assert!(!expected.bitmap.is_empty());

        rasterizer.release_bitmap(key);

        let cached = rasterizer.cached_glyph(key).unwrap();
        assert!(cached.bitmap.is_empty());
        assert_eq!(cached.width, expected.width);
        assert_eq!(cached.height, expected.height);
        assert_eq!(cached.advance_width, expected.advance_width);
    }

    #[test]
    fn bitmap_cache_pruning_keeps_metrics_and_releases_pixel_capacity() {
        let mut rasterizer = GlyphRasterizer::primary_only();
        let key = GlyphKey::new(rasterizer.primary_font_id(), 1, 16.0);
        rasterizer.commit_prepared_glyph(
            key,
            RasterizedGlyph {
                bitmap: vec![255; 1024],
                width: 32,
                height: 32,
                offset_x: 1.0,
                offset_y: 2.0,
                advance_width: 12.0,
                is_color: false,
            },
        );

        rasterizer.make_bitmap_capacity_for(GlyphRasterizer::BITMAP_CACHE_CAPACITY_BYTES);

        let cached = rasterizer.cached_glyph(key).unwrap();
        assert!(cached.bitmap.is_empty());
        assert_eq!((cached.width, cached.height), (32, 32));
        assert_eq!(cached.advance_width, 12.0);
    }
}
