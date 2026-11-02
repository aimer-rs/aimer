#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use std::collections::HashSet;
#[cfg(target_os = "linux")]
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;
use std::sync::Arc;
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{LazyLock, RwLock};

use aimer_utils::info;

use crate::text_layout::FontId;

#[derive(Clone)]
pub struct FontRecord {
    pub id: FontId,
    pub bytes: Option<Arc<[u8]>>,
    pub(crate) collection_index: u32,
    pub(crate) path: Option<Arc<PathBuf>>,
    /// True when the font carries color glyph data (`sbix` / `CBDT` / `COLR` /
    /// `SVG `) or a private color strike such as Apple's `emjc`. Private
    /// color data is marked here only to route it to the platform compatibility
    /// renderer; the portable Aimer reader never decodes it.
    pub is_color: bool,
}

/// A script/category lane in the platform-independent fallback chain.
///
/// Each lane is discovered independently. This keeps a first emoji miss from
/// enumerating every installed CJK, Arabic, and Indic face, while preserving a
/// stable id range for faces found in a later lane. Apple platforms do not use
/// these probe lanes for selection — Core Text resolves a face per codepoint —
/// but the classification is still useful to keep the rasterizer's loading
/// state explicit and testable.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum FallbackScript {
    Emoji,
    Cjk,
    Hangul,
    Arabic,
    Hebrew,
    Devanagari,
    Tamil,
    Thai,
    Armenian,
    Georgian,
    Ethiopic,
    Myanmar,
    Khmer,
    Tibetan,
    Sinhala,
    Telugu,
    Kannada,
    Malayalam,
    Gujarati,
    Gurmukhi,
    Bengali,
    Oriya,
    Lao,
    Mongolian,
    Cherokee,
    Yi,
}

impl FallbackScript {
    pub(crate) const COUNT: usize = 26;
    pub(crate) const ALL: [Self; Self::COUNT] = [
        Self::Emoji,
        Self::Cjk,
        Self::Hangul,
        Self::Arabic,
        Self::Hebrew,
        Self::Devanagari,
        Self::Tamil,
        Self::Thai,
        Self::Armenian,
        Self::Georgian,
        Self::Ethiopic,
        Self::Myanmar,
        Self::Khmer,
        Self::Tibetan,
        Self::Sinhala,
        Self::Telugu,
        Self::Kannada,
        Self::Malayalam,
        Self::Gujarati,
        Self::Gurmukhi,
        Self::Bengali,
        Self::Oriya,
        Self::Lao,
        Self::Mongolian,
        Self::Cherokee,
        Self::Yi,
    ];

    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    const fn index(self) -> usize {
        match self {
            Self::Emoji => 0,
            Self::Cjk => 1,
            Self::Hangul => 2,
            Self::Arabic => 3,
            Self::Hebrew => 4,
            Self::Devanagari => 5,
            Self::Tamil => 6,
            Self::Thai => 7,
            Self::Armenian => 8,
            Self::Georgian => 9,
            Self::Ethiopic => 10,
            Self::Myanmar => 11,
            Self::Khmer => 12,
            Self::Tibetan => 13,
            Self::Sinhala => 14,
            Self::Telugu => 15,
            Self::Kannada => 16,
            Self::Malayalam => 17,
            Self::Gujarati => 18,
            Self::Gurmukhi => 19,
            Self::Bengali => 20,
            Self::Oriya => 21,
            Self::Lao => 22,
            Self::Mongolian => 23,
            Self::Cherokee => 24,
            Self::Yi => 25,
        }
    }

    /// Stable base id for the one face selected for this lane.
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    pub(crate) const fn id_base(self) -> FontId {
        FALLBACK_CHAIN_ID_BASE + self.index() as FontId * FALLBACK_CHAIN_ID_STRIDE
    }

    fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

/// Reserved ids for faces found by the platform-independent probe lanes.
///
/// Each lane currently contributes at most one face, but reserving a stride
/// leaves room for a future deterministic per-lane shortlist without making
/// runtime registration ids depend on which scripts happened to be rendered
/// first.
pub(crate) const FALLBACK_CHAIN_ID_BASE: FontId = 0x1000_0000;
const FALLBACK_CHAIN_ID_STRIDE: FontId = 0x1000;

/// Stable id used for the desktop system sans-serif face when bundled fonts
/// are disabled. It stays below the process-wide platform-fallback range and
/// the bundled CJK lane, so runtime registrations remain collision-free.
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
pub(crate) const SYSTEM_PRIMARY_FONT_ID: FontId = 0x3000_0000;

/// Classifies a codepoint before a fallback lookup.
pub(crate) fn fallback_script_for_codepoint(codepoint: char) -> Option<FallbackScript> {
    match codepoint as u32 {
        0x1F000..=0x1FAFF | 0x2600..=0x27BF => Some(FallbackScript::Emoji),
        0x3000..=0x303F
        | 0x3040..=0x30FF
        | 0x31F0..=0x31FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF
        | 0x20000..=0x3FFFF => Some(FallbackScript::Cjk),
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF => {
            Some(FallbackScript::Hangul)
        }
        0x0590..=0x05FF => Some(FallbackScript::Hebrew),
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => {
            Some(FallbackScript::Arabic)
        }
        0x0900..=0x097F => Some(FallbackScript::Devanagari),
        0x0B80..=0x0BFF => Some(FallbackScript::Tamil),
        0x0E00..=0x0E7F => Some(FallbackScript::Thai),
        0x0530..=0x058F => Some(FallbackScript::Armenian),
        0x10A0..=0x10FF => Some(FallbackScript::Georgian),
        0x1200..=0x137F => Some(FallbackScript::Ethiopic),
        0x1000..=0x109F => Some(FallbackScript::Myanmar),
        0x1780..=0x17FF => Some(FallbackScript::Khmer),
        0x0F00..=0x0FFF => Some(FallbackScript::Tibetan),
        0x0D80..=0x0DFF => Some(FallbackScript::Sinhala),
        0x0C00..=0x0C7F => Some(FallbackScript::Telugu),
        0x0C80..=0x0CFF => Some(FallbackScript::Kannada),
        0x0D00..=0x0D7F => Some(FallbackScript::Malayalam),
        0x0A80..=0x0AFF => Some(FallbackScript::Gujarati),
        0x0A00..=0x0A7F => Some(FallbackScript::Gurmukhi),
        0x0980..=0x09FF => Some(FallbackScript::Bengali),
        0x0B00..=0x0B7F => Some(FallbackScript::Oriya),
        0x0E80..=0x0EFF => Some(FallbackScript::Lao),
        0x1800..=0x18AF => Some(FallbackScript::Mongolian),
        0x13A0..=0x13FF => Some(FallbackScript::Cherokee),
        0xA000..=0xA4CF => Some(FallbackScript::Yi),
        _ => None,
    }
}

/// Maps a statically assigned chain id back to the lane that owns it.
pub(crate) fn fallback_script_for_font_id(font_id: FontId) -> Option<FallbackScript> {
    let offset = font_id.checked_sub(FALLBACK_CHAIN_ID_BASE)?;
    if offset >= FallbackScript::COUNT as FontId * FALLBACK_CHAIN_ID_STRIDE {
        return None;
    }
    FallbackScript::from_index((offset / FALLBACK_CHAIN_ID_STRIDE) as usize)
}

/// Immutable shared ownership of a font record used to seed local CPU contexts.
///
/// The record itself is never mutably exposed. A preparation context obtains a
/// cheap local copy for worker-local shaping and rasterization state.
#[derive(Clone)]
pub(crate) struct SharedFontRecord(Arc<FontRecord>);

impl SharedFontRecord {
    pub(crate) fn new(record: &FontRecord) -> Self {
        Self(Arc::new(record.clone()))
    }

    pub(crate) fn local_copy(&self) -> FontRecord {
        self.0.as_ref().clone()
    }
}

#[derive(Clone)]
pub(crate) enum FontData {
    Shared(Arc<[u8]>),
    #[cfg(not(target_arch = "wasm32"))]
    Mapped(Arc<memmap2::Mmap>),
}

/// Process-wide cache of memory-mapped font files, keyed by path.
///
/// Mapping a file is not free: it opens a file descriptor and asks the kernel
/// for a new virtual memory region, and every entry point that needs font
/// bytes — glyph coverage probing, charmap lookups, advance metrics,
/// shaping and rasterization — used to repeat that work on each call. A
/// system fallback face is consulted once per codepoint, so a page of mixed
/// scripts performed thousands of redundant mappings of the same handful of
/// files.
///
/// Entries are never evicted. The set of font files an application touches is
/// small and bounded by the faces it actually renders with, and the mappings
/// were already alive for the duration of every operation that used them; the
/// pages themselves stay owned by the page cache and are reclaimable under
/// memory pressure, so retaining the mapping costs address space rather than
/// resident memory.
#[cfg(not(target_arch = "wasm32"))]
static MAPPED_FONT_FILES: LazyLock<RwLock<HashMap<PathBuf, Arc<memmap2::Mmap>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Returns the mapping of `path` if one has already been published, without
/// creating it.
///
/// This is the half of the cache a caller wants when a mapping is worth
/// *reusing* but not worth *retaining*: probing a candidate font answers a
/// yes/no question and is usually a no, so publishing every file it touches
/// would grow a cache that never evicts by the whole installed font set.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn cached_font_file(path: &Path) -> Option<Arc<memmap2::Mmap>> {
    MAPPED_FONT_FILES.read().ok()?.get(path).cloned()
}

/// Publishes `mapping` as the process-wide mapping of `path` and returns the
/// mapping callers will observe from now on.
///
/// A concurrent caller may have published the same file meanwhile. Keeping the
/// entry that is already there guarantees one mapping per file, which is what
/// makes pointer identity of the bytes meaningful — so the returned handle is
/// the published one, which is not necessarily the argument.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn publish_font_file(path: &Path, mapping: Arc<memmap2::Mmap>) -> Arc<memmap2::Mmap> {
    let Ok(mut cache) = MAPPED_FONT_FILES.write() else {
        return mapping;
    };
    cache.entry(path.to_path_buf()).or_insert(mapping).clone()
}

/// Maps `path` read-only without consulting or filling the shared cache.
///
/// Returns `None` when the file cannot be opened or mapped.
fn map_font_file(path: &Path) -> Option<Arc<memmap2::Mmap>> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: the read-only mapping owns its file-backed virtual memory region
    // and remains valid independently of the `File` handle.
    Some(Arc::new(unsafe { memmap2::Mmap::map(&file).ok()? }))
}

/// Returns the shared read-only mapping of `path`, creating it on first use.
///
/// Returns `None` when the file cannot be opened or mapped; failures are not
/// cached, so a font that becomes readable later is picked up.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn mapped_font_file(path: &Path) -> Option<Arc<memmap2::Mmap>> {
    if let Some(mapping) = cached_font_file(path) {
        return Some(mapping);
    }

    Some(publish_font_file(path, map_font_file(path)?))
}

impl AsRef<[u8]> for FontData {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Shared(bytes) => bytes,
            #[cfg(not(target_arch = "wasm32"))]
            Self::Mapped(bytes) => bytes,
        }
    }
}

/// The OpenType `wght` value of a face that asks for no emphasis.
///
/// This is both the weight a face is read as when it declares none and the
/// weight at which a request is passed through untouched, so the path every
/// unemphasized line takes does no weight work at all.
pub(crate) const REGULAR_WEIGHT: u16 = 400;

fn face_metadata(data: FontData, collection_index: u32) -> Option<bool> {
    let face =
        crate::aimer_font::SfntFace::from_font_data(data, collection_index).ok()?;
    face.metrics().ok()?;
    face.table(*b"cmap")?;
    Some(face.has_color_tables() || face.has_apple_private_color_tables())
}

impl FontRecord {
    /// Creates a non-rendering placeholder used when no bundled or host font
    /// is available. Applications can replace it by registering a font before
    /// rendering; the placeholder keeps rasterizer construction fallible-free
    /// and still gives unsupported glyphs a stable id.
    #[allow(dead_code)]
    pub(crate) const fn unavailable(id: FontId) -> Self {
        Self {
            id,
            bytes: None,
            collection_index: 0,
            path: None,
            is_color: false,
        }
    }

    #[cfg(any(feature = "bundled-fonts", target_arch = "wasm32", test))]
    pub(crate) fn from_static_bytes(id: FontId, bytes: &'static [u8]) -> Option<Self> {
        let bytes: Arc<[u8]> = Arc::from(bytes);
        let is_color = face_metadata(FontData::Shared(bytes.clone()), 0)?;
        Some(Self {
            id,
            bytes: Some(bytes),
            collection_index: 0,
            path: None,
            is_color,
        })
    }

    pub fn from_bytes(id: FontId, bytes: Vec<u8>) -> Option<Self> {
        let bytes: Arc<[u8]> = Arc::from(bytes);
        let is_color = face_metadata(FontData::Shared(bytes.clone()), 0)?;
        Some(Self {
            id,
            bytes: Some(bytes),
            collection_index: 0,
            path: None,
            is_color,
        })
    }

    pub(crate) fn from_shared_bytes(id: FontId, bytes: Arc<[u8]>) -> Option<Self> {
        let is_color = face_metadata(FontData::Shared(bytes.clone()), 0)?;

        Some(Self {
            id,
            bytes: Some(bytes),
            collection_index: 0,
            path: None,
            is_color,
        })
    }

    /// Creates a lazily mapped record for an on-disk system font.
    ///
    /// The file is validated and its mapping is published once, but the font
    /// bytes remain file-backed rather than becoming part of the application
    /// binary or an eagerly copied heap allocation.
    #[allow(dead_code)]
    pub(crate) fn from_file(id: FontId, path: PathBuf, collection_index: u32) -> Option<Self> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let data = mapped_font_file(&path)?;
            let is_color = face_metadata(FontData::Mapped(data), collection_index)?;
            return Some(Self {
                id,
                bytes: None,
                collection_index,
                path: Some(Arc::new(path)),
                is_color,
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            let _ = (id, path, collection_index);
            None
        }
    }

    /// Retain shared in-memory data or hand out the process-wide memory map of
    /// a file-backed font, without copying the font into the process heap.
    ///
    /// The mapping is created once per file and shared by every record and
    /// every thread referring to it, so repeated calls are a hash lookup and an
    /// atomic refcount bump rather than an `open` plus `mmap`.
    pub(crate) fn data(&self) -> Option<FontData> {
        if let Some(bytes) = self.bytes.as_ref() {
            return Some(FontData::Shared(bytes.clone()));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            mapped_font_file(self.path.as_ref()?.as_ref()).map(FontData::Mapped)
        }
        #[cfg(target_arch = "wasm32")]
        None
    }

    #[allow(dead_code)]
    pub(crate) fn ensure_face(&self) -> Option<()> {
        let data = self.data()?;
        crate::aimer_font::SfntFace::from_font_data(data, self.collection_index)
            .ok()?;
        Some(())
    }

    /// Returns the file this face was loaded from, if it is backed by one.
    ///
    /// Fonts registered from memory have no path. A path is what identifies a
    /// face to the platform rasterizer, which is the only way to draw faces
    /// whose glyph data Cupid cannot decode.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[inline]
    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_ref().map(|path| path.as_path())
    }

    /// Runs `read` on this face's parsed tables, sharing them with every other
    /// caller of the same face instead of re-reading the font for each call.
    ///
    /// A face whose metrics tables are unusable cannot enter the shared cache;
    /// it is read from its bytes for this call alone, as it always was.
    pub(crate) fn with_face<R>(
        &self,
        read: impl FnOnce(&crate::aimer_font::SfntFace<'_>) -> R,
    ) -> Option<R> {
        let data = self.data()?;
        if let Ok(shared) = crate::aimer_font::shared_face(data.clone(), self.collection_index) {
            return Some(read(shared.sfnt()));
        }
        let face =
            crate::aimer_font::SfntFace::from_bytes(data.as_ref(), self.collection_index).ok()?;
        Some(read(&face))
    }

    /// Returns whether this face has an outline table understood by the
    /// Aimer-owned reader.
    pub(crate) fn has_standard_outline(&self) -> bool {
        let Some(data) = self.data() else {
            return false;
        };
        crate::aimer_font::SfntFace::from_font_data(data, self.collection_index)
            .ok()
            .is_some_and(|face| face.has_standard_outline())
    }

    /// Returns a face's design weight from its `OS/2` table.
    pub(crate) fn design_weight(&self) -> Option<u16> {
        self.with_face(|face| face.design_weight())?
    }

    /// Returns a glyph's scaled horizontal advance from the active reader.
    pub(crate) fn advance_width_for_glyph(&self, glyph_id: u16, font_size: f32) -> Option<f32> {
        self.with_face(|face| advance_width_in_face(face, glyph_id, font_size))?
    }

    /// Returns scaled ascent, descent, and line gap from the Aimer reader.
    pub(crate) fn line_metrics(&self, font_size: f32) -> Option<(f32, f32, f32)> {
        if !font_size.is_finite() || font_size <= 0.0 {
            return None;
        }
        let data = self.data()?;
        let metrics =
            crate::aimer_font::metrics_from_font_data(data, self.collection_index)
                .ok()?;
        Some(metrics.scaled_line_metrics(font_size))
    }

    pub(crate) fn glyph_index(&self, codepoint: char) -> Option<u16> {
        self.with_face(|face| face.glyph_index(codepoint as u32).ok().flatten())?
    }
}

pub fn advance_width_from_face(
    bytes: &[u8],
    collection_index: u32,
    glyph_id: u16,
    font_size: f32,
) -> Option<f32> {
    if !font_size.is_finite() || font_size <= 0.0 {
        return None;
    }
    let face =
        crate::aimer_font::SfntFace::from_bytes(bytes, collection_index).ok()?;
    advance_width_in_face(&face, glyph_id, font_size)
}

fn advance_width_in_face(
    face: &crate::aimer_font::SfntFace<'_>,
    glyph_id: u16,
    font_size: f32,
) -> Option<f32> {
    if !font_size.is_finite() || font_size <= 0.0 {
        return None;
    }
    let metrics = face.metrics().ok()?;
    let advance = face.glyph_advance_with_metrics(glyph_id, metrics).ok()??;
    let scale = font_size / f32::from(metrics.units_per_em);
    Some(f32::from(advance) * scale)
}

/// A probe group: one script / category with the codepoints used to verify
/// that a font actually covers it.  `hint_color` marks probe groups that
/// identify color-emoji fonts so we can set `is_color` even before decoding.
///
/// Probe groups exist only for platforms without a system-provided cascade
/// list. Apple platforms resolve fonts per codepoint through Core Text — see
/// [`system_fallback`](crate::system_fallback) — and therefore
/// need no hardcoded script table at all.
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
#[allow(dead_code)]
struct ProbeGroup {
    label: &'static str,
    probes: &'static [char],
    hint_color: bool,
}

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
const fn probe_group(label: &'static str, probes: &'static [char], hint_color: bool) -> ProbeGroup {
    ProbeGroup {
        label,
        probes,
        hint_color,
    }
}

/// All script / category probe groups we want covered in the fallback chain.
/// Order is significant: earlier groups are preferred over later ones when a
/// single font file covers multiple scripts (the first probe group that matches
/// controls whether the font is added to the chain for that group).
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
static PROBE_GROUPS: &[ProbeGroup] = &[
    probe_group("emoji", &['😀', '👍'], true),
    // Keep Hangul out of the Han lane. A pan-CJK face may carry both, but a
    // Korean run must be able to select the Hangul lane independently.
    probe_group("cjk", &['你', '漢'], false),
    probe_group("hangul", &['가', '나', '다'], false),
    probe_group("arabic", &['\u{0639}', '\u{0627}'], false), /* ع ا */
    probe_group("hebrew", &['\u{05D0}', '\u{05D1}'], false), /* א ב */
    probe_group("devanagari", &['\u{0915}', '\u{0930}'], false), /* क र */
    probe_group("tamil", &['\u{0B95}', '\u{0BB5}'], false),  /* க வ */
    probe_group("thai", &['\u{0E01}', '\u{0E02}'], false),   /* ก ข */
    probe_group("armenian", &['\u{0531}', '\u{0532}'], false), /* Ա Բ */
    probe_group("georgian", &['\u{10D0}', '\u{10D1}'], false), /* ა ბ */
    probe_group("ethiopic", &['\u{1200}', '\u{1201}'], false), /* ሀ ሁ */
    probe_group("myanmar", &['\u{1000}', '\u{1001}'], false), /* က ခ */
    probe_group("khmer", &['\u{1780}', '\u{1781}'], false),  /* ក ខ */
    probe_group("tibetan", &['\u{0F00}'], false),            // ༀ
    probe_group("sinhala", &['\u{0D9A}'], false),            // ක
    probe_group("telugu", &['\u{0C15}'], false),             // క
    probe_group("kannada", &['\u{0C95}'], false),            // ಕ
    probe_group("malayalam", &['\u{0D15}'], false),          // ക
    probe_group("gujarati", &['\u{0A95}'], false),           // ક
    probe_group("gurmukhi", &['\u{0A15}'], false),           // ਕ
    probe_group("bengali", &['\u{0995}'], false),            // ক
    probe_group("oriya", &['\u{0B15}'], false),              // କ
    probe_group("lao", &['\u{0E81}'], false),                // ກ
    probe_group("mongolian", &['\u{1820}'], false),          // ᠠ
    probe_group("cherokee", &['\u{13A0}'], false),           // Ꭰ
    probe_group("yi", &['\u{A000}'], false),                 /* ꀀ */
];

/// Check whether font data (passed as a byte slice) satisfies `probes`.
/// Returns `Some(is_color)` on success, or `None` if the font doesn't match.
///
/// For color/emoji probe groups (`hint_color=true`) we additionally require
/// that the font has an `sbix` or `cbdt` table (real bitmap strikes) — a
/// COLR-only table is not enough, since placeholder/fallback fonts like
/// LastResort.otf also carry COLR but contain no usable emoji bitmaps.
///
/// For regular (non-color) probe groups we additionally verify that at least
/// one probe glyph has a non-empty bounding box, which filters out fonts that
/// declare a cmap entry for a codepoint but store the glyph as a composite with
/// no direct outline (e.g., some older pan-Unicode fonts for certain CJK
/// ranges).
#[cfg(all(not(any(target_os = "ios", target_os = "macos"))))]
fn face_matches_probes(
    face: &crate::aimer_font::SfntFace<'_>,
    probes: &[char],
    hint_color: bool,
) -> Option<bool> {
    if hint_color {
        return face.has_color_tables().then_some(true);
    }

    let glyph_ids: HashSet<u16> = probes
        .iter()
        .filter_map(|&codepoint| face.glyph_index(codepoint as u32).ok().flatten())
        .filter(|&id| id != 0)
        .collect();

    // Need at least 2 distinct non-zero glyph IDs among the probes.  Single-probe
    // groups (like tibetan, sinhala, etc.) get a pass on the distinctness check —
    // we just verify the glyph has a bounding box instead.
    if probes.len() >= 2 && glyph_ids.len() < 2 {
        // All probes mapped to the same glyph — very likely a placeholder font.
        return None;
    }

    // Additionally verify at least one probe glyph has a non-empty bounding box
    // so we know the font can actually produce visible outlines for it.
    let has_usable_outline = probes.iter().any(|&codepoint| {
        let Some(glyph_id) = face.glyph_index(codepoint as u32).ok().flatten() else {
            return false;
        };
        face.outline(glyph_id).ok().flatten().is_some()
            || face.cff_outline(glyph_id).ok().flatten().is_some()
    });
    if !has_usable_outline {
        return None;
    }

    Some(false) // non-color font confirmed usable
}

/// Returns a mapping of `path` suitable for *probing* a candidate face: the
/// shared one when the pipeline has already published it, a private one
/// otherwise.
///
/// Reusing the published mapping is what makes a warm cache free — a font the
/// application already renders with is not opened again — while a candidate
/// that is only being examined stays out of a cache that never evicts.
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn probe_font_file(path: &Path) -> Option<Arc<memmap2::Mmap>> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(mapping) = cached_font_file(path) {
        return Some(mapping);
    }

    map_font_file(path)
}

/// Hands the mapping a probe created over to the shared cache, so the face's
/// first render — and every metric query after it — finds the file already
/// mapped instead of paying for another `open` plus `mmap`.
///
/// Only worth calling for a face that has actually been accepted into the
/// chain; see [`probe_font_file`].
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn retain_probed_font_file(path: &Path, mapping: Arc<memmap2::Mmap>) {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = publish_font_file(path, mapping);
    #[cfg(target_arch = "wasm32")]
    let _ = (path, mapping);
}

/// Returns whether `lookup` — a font's character map — maps any of `probes`.
///
/// This is the cheap half of face selection. Answering it needs nothing but a
/// `cmap` subtable, which the platform font enumerator has already located for
/// every installed face, whereas the full probe in `face_matches_probes`
/// needs the face parsed: a table directory walk plus, for the groups it does
/// not reject outright, glyph metrics and color table lookups. Since the full
/// probe starts by requiring exactly this condition, using it as a gate rejects
/// no face the chain would have accepted while sparing the parse for the
/// overwhelming majority of faces, which cover none of the scripts still
/// wanted.
///
/// `lookup` is called at most once per probe and stops at the first hit.
#[cfg_attr(any(target_os = "ios", target_os = "macos"), allow(dead_code))]
fn any_probe_is_mapped(mut lookup: impl FnMut(char) -> Option<u32>, probes: &[char]) -> bool {
    probes.iter().any(|&codepoint| lookup(codepoint).is_some())
}

/// Claims the highest-priority group slot that is still open and whose probes
/// the offered face satisfies, returning the slot index and the color flag the
/// probe reported.
///
/// `open[i]` tells whether group `i` still needs a face; `probe` is only ever
/// called for open slots, at most once each, and never again once a slot has
/// been claimed. That is what keeps chain construction linear in the number of
/// faces: a single face is parsed once and offered to every group that still
/// wants one, instead of every group re-scanning every face.
///
/// The scan runs in slot order, so a face able to serve several scripts is
/// spent on the most important one — matching the priority encoded in the
/// group table.
#[cfg_attr(any(target_os = "ios", target_os = "macos"), allow(dead_code))]
fn first_open_match(
    open: &[bool],
    mut probe: impl FnMut(usize) -> Option<bool>,
) -> Option<(usize, bool)> {
    open.iter()
        .enumerate()
        .filter(|&(_, is_open)| *is_open)
        .find_map(|(group, _)| probe(group).map(|is_color| (group, is_color)))
}

/// Builds only the fallback face for `script`.
///
/// Linux asks Fontconfig for candidates in system preference order. Other
/// desktop targets scan their font catalogue. The caller gives each lane its
/// own id range, so the result is independent of the order in which scripts
/// first appear in the UI.
#[cfg(target_os = "linux")]
fn build_fallback_chain_for_script(script: FallbackScript) -> Vec<FontRecord> {
    let probes = &PROBE_GROUPS[script.index()];
    match fontconfig_font_record(
        script.id_base(),
        "sans-serif",
        probes.probes,
        Some(script),
        probes.hint_color,
    ) {
        Ok(Some(record)) => return vec![record],
        Ok(None) => return Vec::new(),
        Err(_) => {}
    }

    let paths = system_font_paths();
    build_fallback_chain_for_scripts(std::slice::from_ref(&script), &paths)
}

#[cfg(all(
    not(target_os = "linux"),
    not(any(target_os = "ios", target_os = "macos"))
))]
fn build_fallback_chain_for_script(script: FallbackScript) -> Vec<FontRecord> {
    let paths = system_font_paths();
    build_fallback_chain_for_scripts(std::slice::from_ref(&script), &paths)
}

#[cfg(target_os = "windows")]
fn windows_fallback_font_filename(script: FallbackScript) -> Option<&'static str> {
    match script {
        FallbackScript::Thai | FallbackScript::Khmer | FallbackScript::Lao => Some("LeelawUI.ttf"),
        FallbackScript::Myanmar => Some("mmrtext.ttf"),
        FallbackScript::Devanagari
        | FallbackScript::Tamil
        | FallbackScript::Sinhala
        | FallbackScript::Telugu
        | FallbackScript::Kannada
        | FallbackScript::Malayalam
        | FallbackScript::Gujarati
        | FallbackScript::Gurmukhi
        | FallbackScript::Bengali
        | FallbackScript::Oriya => Some("Nirmala.ttf"),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn prioritized_fallback_paths<'a>(
    paths: &'a [PathBuf],
    preferred_filenames: &[&str],
) -> Vec<&'a PathBuf> {
    let mut unique_preferences = Vec::with_capacity(preferred_filenames.len());
    for &preferred in preferred_filenames {
        if !unique_preferences
            .iter()
            .any(|known: &&str| known.eq_ignore_ascii_case(preferred))
        {
            unique_preferences.push(preferred);
        }
    }

    let mut ordered = Vec::with_capacity(paths.len());
    for preferred in &unique_preferences {
        ordered.extend(paths.iter().filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(preferred))
        }));
    }
    ordered.extend(paths.iter().filter(|path| {
        !unique_preferences.iter().any(|preferred| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(preferred))
        })
    }));
    ordered
}

#[cfg(all(not(any(target_os = "ios", target_os = "macos"))))]
fn build_fallback_chain_for_scripts(
    scripts: &[FallbackScript],
    font_paths: &[PathBuf],
) -> Vec<FontRecord> {
    let mut slots: Vec<Option<FontRecord>> = (0..scripts.len()).map(|_| None).collect();
    let mut open = vec![true; scripts.len()];
    let mut open_count = scripts.len();
    if open_count == 0 {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let preferred_filenames = scripts
            .iter()
            .filter_map(|script| windows_fallback_font_filename(*script))
            .collect::<Vec<_>>();
        if !preferred_filenames.is_empty() {
            for path in prioritized_fallback_paths(font_paths, &preferred_filenames) {
                let filename = path.file_name().and_then(|name| name.to_str());
                let mut preferred_open = scripts
                    .iter()
                    .enumerate()
                    .map(|(index, script)| {
                        open[index]
                            && windows_fallback_font_filename(*script).is_some_and(|preferred| {
                                filename.is_some_and(|name| name.eq_ignore_ascii_case(preferred))
                            })
                    })
                    .collect::<Vec<_>>();
                if !preferred_open.iter().any(|is_open| *is_open) {
                    continue;
                }

                let Some(mapping) = probe_font_file(path) else {
                    continue;
                };
                let data = &mapping[..];
                for collection_index in 0..64 {
                    let Ok(face) = crate::aimer_font::SfntFace::from_bytes(
                        data,
                        collection_index,
                    ) else {
                        break;
                    };

                    while let Some((group, is_color)) = first_open_match(&preferred_open, |group| {
                        let probe_group = &PROBE_GROUPS[scripts[group].index()];
                        if !any_probe_is_mapped(
                            |codepoint| {
                                face.glyph_index(codepoint as u32)
                                    .ok()
                                    .flatten()
                                    .map(u32::from)
                            },
                            probe_group.probes,
                        ) {
                            return None;
                        }
                        face_matches_probes(&face, probe_group.probes, probe_group.hint_color)
                    }) {
                        let record_path = Arc::new(path.clone());
                        slots[group] = Some(FontRecord {
                            id: scripts[group].id_base(),
                            bytes: None,
                            collection_index,
                            path: Some(record_path.clone()),
                            is_color,
                        });
                        retain_probed_font_file(record_path.as_path(), mapping.clone());
                        preferred_open[group] = false;
                        open[group] = false;
                        open_count -= 1;
                        if open_count == 0 {
                            return slots.into_iter().flatten().collect();
                        }
                    }
                }
            }
        }
    }

    for path in font_paths {
        if open_count == 0 {
            return slots.into_iter().flatten().collect();
        }
        let Some(mapping) = probe_font_file(path) else {
            continue;
        };
        let data = &mapping[..];
        for collection_index in 0..64 {
            let Ok(face) =
                crate::aimer_font::SfntFace::from_bytes(data, collection_index)
            else {
                break;
            };

            while let Some((group, is_color)) = first_open_match(&open, |group| {
                let probe_group = &PROBE_GROUPS[scripts[group].index()];
                if !any_probe_is_mapped(
                    |codepoint| {
                        face.glyph_index(codepoint as u32)
                            .ok()
                            .flatten()
                            .map(u32::from)
                    },
                    probe_group.probes,
                ) {
                    return None;
                }
                face_matches_probes(&face, probe_group.probes, probe_group.hint_color)
            }) {
                let record_path = Arc::new(path.clone());
                slots[group] = Some(FontRecord {
                    id: scripts[group].id_base(),
                    bytes: None,
                    collection_index,
                    path: Some(record_path.clone()),
                    is_color,
                });
                retain_probed_font_file(record_path.as_path(), mapping.clone());
                open[group] = false;
                open_count -= 1;
                if open_count == 0 {
                    return slots.into_iter().flatten().collect();
                }
            }
        }
    }

    slots.into_iter().flatten().collect()
}

#[cfg(all(not(any(target_os = "ios", target_os = "macos"))))]
fn system_font_paths() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    #[cfg(target_os = "android")]
    roots.extend([
        PathBuf::from("/system/fonts"),
        PathBuf::from("/product/fonts"),
    ]);
    #[cfg(target_os = "windows")]
    if let Some(windows_directory) = std::env::var_os("WINDIR") {
        roots.push(PathBuf::from(windows_directory).join("Fonts"));
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".fonts"));
        roots.push(home.join(".local/share/fonts"));
    }

    let mut pending = roots;
    let mut paths = Vec::new();
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };
            if matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            ) {
                paths.push(path);
            }
        }
    }
    paths.sort_unstable();
    paths.dedup();
    paths
}

#[cfg(target_os = "linux")]
fn fontconfig_language(script: FallbackScript) -> Option<&'static str> {
    match script {
        FallbackScript::Emoji | FallbackScript::Cjk => None,
        FallbackScript::Hangul => Some("ko"),
        FallbackScript::Arabic => Some("ar"),
        FallbackScript::Hebrew => Some("he"),
        FallbackScript::Devanagari => Some("hi"),
        FallbackScript::Tamil => Some("ta"),
        FallbackScript::Thai => Some("th"),
        FallbackScript::Armenian => Some("hy"),
        FallbackScript::Georgian => Some("ka"),
        FallbackScript::Ethiopic => Some("am"),
        FallbackScript::Myanmar => Some("my"),
        FallbackScript::Khmer => Some("km"),
        FallbackScript::Tibetan => Some("bo"),
        FallbackScript::Sinhala => Some("si"),
        FallbackScript::Telugu => Some("te"),
        FallbackScript::Kannada => Some("kn"),
        FallbackScript::Malayalam => Some("ml"),
        FallbackScript::Gujarati => Some("gu"),
        FallbackScript::Gurmukhi => Some("pa"),
        FallbackScript::Bengali => Some("bn"),
        FallbackScript::Oriya => Some("or"),
        FallbackScript::Lao => Some("lo"),
        FallbackScript::Mongolian => Some("mn"),
        FallbackScript::Cherokee => Some("chr"),
        FallbackScript::Yi => Some("ii"),
    }
}

#[cfg(target_os = "linux")]
struct FcMatchUnavailable;

#[cfg(target_os = "linux")]
fn fontconfig_font_record(
    id: FontId,
    family: &str,
    probes: &[char],
    script: Option<FallbackScript>,
    hint_color: bool,
) -> Result<Option<FontRecord>, FcMatchUnavailable> {
    let mut pattern = format!("{family}:weight=regular:slant=roman");
    if let Some(language) = script.and_then(fontconfig_language) {
        write!(&mut pattern, ":lang={language}").expect("writing to a String cannot fail");
    }
    pattern.push_str(":charset=");
    for (index, &codepoint) in probes.iter().enumerate() {
        if index != 0 {
            pattern.push(',');
        }
        write!(&mut pattern, "{:x}", codepoint as u32).expect("writing to a String cannot fail");
    }

    // Use the host's Fontconfig ordering when available. Calling through argv
    // avoids shell parsing, and the path scanner below remains a fallback for
    // minimal systems without the `fc-match` utility.
    let output = Command::new("fc-match")
        .arg("--sort")
        .arg(r"--format=%{file}\t%{index}\n")
        .arg(pattern)
        .output()
        .map_err(|_| FcMatchUnavailable)?;
    if !output.status.success() {
        return Err(FcMatchUnavailable);
    }

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((filename, font_index)) = line.rsplit_once('\t') else {
            continue;
        };
        let Ok(font_index) = font_index.parse::<u32>() else {
            continue;
        };
        // Fontconfig may encode a variable-font named instance in the high
        // bits. Cupid's SFNT reader needs only the collection face index.
        let collection_index = font_index & 0xffff;
        let path = PathBuf::from(filename);
        let Some(mapping) = probe_font_file(&path) else {
            continue;
        };
        let Ok(face) =
            crate::aimer_font::SfntFace::from_bytes(&mapping, collection_index)
        else {
            continue;
        };
        let has_color_tables = face.has_color_tables() || face.has_apple_private_color_tables();
        let Some(probe_is_color) = face_matches_probes(&face, probes, hint_color) else {
            continue;
        };

        let path = Arc::new(path);
        retain_probed_font_file(path.as_path(), mapping);
        return Ok(Some(FontRecord {
            id,
            bytes: None,
            collection_index,
            path: Some(path),
            is_color: has_color_tables || probe_is_color,
        }));
    }
    Ok(None)
}

#[cfg(all(
    not(any(target_os = "ios", target_os = "macos")),
    not(target_arch = "wasm32")
))]
fn path_looks_monospace(path: &Path) -> bool {
    let name = path.to_string_lossy().to_ascii_lowercase();
    ["mono", "fixed", "courier", "console", "code"]
        .iter()
        .any(|hint| name.contains(hint))
}

/// Ranks an Android system font file for the generic sans-serif or monospace
/// slot; lower ranks are probed first.
///
/// `/system/fonts` is a flat directory in which display faces sort ahead of the
/// UI family (`CarroisGothicSC-Regular.ttf`, a small-caps face, precedes
/// `Roboto-Regular.ttf`), so taking the first alphabetical face that covers the
/// probes renders every default-styled string in the wrong typeface. Names follow
/// the families the platform's `fonts.xml` maps to `sans-serif` and `monospace`.
#[cfg(any(target_os = "android", test))]
fn android_font_priority(filename: &str, monospace: bool) -> u8 {
    const SANS: &[&str] = &[
        "Roboto-Regular.ttf",
        "RobotoStatic-Regular.ttf",
        "GoogleSansText-Regular.ttf",
        "GoogleSans-Regular.ttf",
        "DroidSans.ttf",
    ];
    const MONOSPACE: &[&str] = &["DroidSansMono.ttf"];
    let preferred = if monospace { MONOSPACE } else { SANS };
    preferred
        .iter()
        .position(|name| name.eq_ignore_ascii_case(filename))
        .map_or(preferred.len() as u8, |rank| rank as u8)
}

#[cfg(all(
    not(any(target_os = "ios", target_os = "macos")),
    not(target_arch = "wasm32")
))]
fn system_font_record_from_paths(
    id: FontId,
    probes: &[char],
    prefer_monospace: bool,
) -> Option<FontRecord> {
    let mut paths = system_font_paths();
    if prefer_monospace {
        paths.sort_by_key(|path| (!path_looks_monospace(path), path.clone()));
    }
    #[cfg(target_os = "android")]
    paths.sort_by_key(|path| {
        let filename = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
        (
            android_font_priority(filename, prefer_monospace),
            prefer_monospace && !path_looks_monospace(path),
            path.clone(),
        )
    });
    #[cfg(target_os = "windows")]
    if !prefer_monospace {
        // Prefer the Windows UI family over the first alphabetic face (which
        // can be a condensed display font such as Agency FB).
        paths.sort_by_key(|path| {
            let filename = path.file_name().and_then(|name| name.to_str());
            let priority = match filename {
                Some(name) if name.eq_ignore_ascii_case("SegUIVar.ttf") => 0,
                Some(name) if name.eq_ignore_ascii_case("segoeui.ttf") => 1,
                _ => 2,
            };
            (priority, path.clone())
        });
    }

    for path in paths {
        let Some(mapping) = probe_font_file(&path) else {
            continue;
        };
        let data = &mapping[..];
        for collection_index in 0..64 {
            let Ok(face) =
                crate::aimer_font::SfntFace::from_bytes(data, collection_index)
            else {
                break;
            };
            let is_color = face.has_color_tables() || face.has_apple_private_color_tables();
            let covers = probes.iter().all(|&codepoint| {
                let Ok(Some(glyph_id)) = face.glyph_index(codepoint as u32) else {
                    return false;
                };
                glyph_id != 0
                    && (is_color
                        || face.outline(glyph_id).ok().flatten().is_some()
                        || face.cff_outline(glyph_id).ok().flatten().is_some())
            });
            if !covers {
                continue;
            }

            let path = Arc::new(path.clone());
            retain_probed_font_file(path.as_path(), mapping.clone());
            return Some(FontRecord {
                id,
                bytes: None,
                collection_index,
                path: Some(path),
                is_color,
            });
        }
    }
    None
}

/// Finds the first readable desktop face that can serve ordinary sans-serif
/// text. The result is cached process-wide and remains file-backed.
#[cfg(any(
    not(any(target_os = "ios", target_os = "macos")),
    all(
        any(target_os = "ios", target_os = "macos"),
        not(feature = "apple-core-text")
    )
))]
pub(crate) fn system_primary_font() -> Option<FontRecord> {
    #[cfg(all(
        not(any(target_os = "ios", target_os = "macos")),
        not(target_arch = "wasm32")
    ))]
    {
        static PRIMARY: OnceLock<Option<FontRecord>> = OnceLock::new();
        PRIMARY
            .get_or_init(|| {
                #[cfg(target_os = "linux")]
                match fontconfig_font_record(
                    SYSTEM_PRIMARY_FONT_ID,
                    "sans-serif",
                    &['A', 'a', '0'],
                    None,
                    false,
                ) {
                    Ok(record) => return record,
                    Err(_) => {}
                }
                system_font_record_from_paths(SYSTEM_PRIMARY_FONT_ID, &['A', 'a', '0'], false)
            })
            .clone()
    }

    #[cfg(any(target_arch = "wasm32", target_os = "ios", target_os = "macos"))]
    {
        None
    }
}

/// Finds a readable desktop monospace face without putting a font in the
/// application binary. The caller supplies the generic-family id so the
/// bundled and system implementations share the same key space.
#[cfg(any(
    not(any(target_os = "ios", target_os = "macos")),
    all(
        any(target_os = "ios", target_os = "macos"),
        not(feature = "apple-core-text")
    )
))]
pub(crate) fn system_monospace_font(id: FontId) -> Option<FontRecord> {
    #[cfg(all(
        not(any(target_os = "ios", target_os = "macos")),
        not(target_arch = "wasm32")
    ))]
    {
        static MONOSPACE: OnceLock<Option<FontRecord>> = OnceLock::new();
        MONOSPACE
            .get_or_init(|| {
                #[cfg(target_os = "linux")]
                match fontconfig_font_record(id, "monospace", &['M', 'i', '0'], None, false) {
                    Ok(record) => return record,
                    Err(_) => {}
                }
                system_font_record_from_paths(id, &['M', 'i', '0'], true)
            })
            .clone()
    }

    #[cfg(any(target_arch = "wasm32", target_os = "ios", target_os = "macos"))]
    {
        let _ = id;
        None
    }
}

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
static SHARED_FALLBACKS_BY_SCRIPT: OnceLock<[OnceLock<Vec<FontRecord>>; FallbackScript::COUNT]> =
    OnceLock::new();

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn shared_fallback_lanes() -> &'static [OnceLock<Vec<FontRecord>>; FallbackScript::COUNT] {
    SHARED_FALLBACKS_BY_SCRIPT.get_or_init(|| std::array::from_fn(|_| OnceLock::new()))
}

pub fn shared_fallback_chain() -> Vec<FontRecord> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    {
        Vec::new()
    }

    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    {
        let fallbacks = shared_fallback_lanes();
        let missing_scripts = FallbackScript::ALL
            .iter()
            .copied()
            .filter(|script| fallbacks[script.index()].get().is_none())
            .collect::<Vec<_>>();

        if !missing_scripts.is_empty() {
            #[cfg(target_os = "linux")]
            for script in missing_scripts {
                let records = build_fallback_chain_for_script(script);
                let _ = fallbacks[script.index()].set(records);
            }

            #[cfg(not(target_os = "linux"))]
            {
                let paths = system_font_paths();
                let records = build_fallback_chain_for_scripts(&missing_scripts, &paths);
                let mut lane_records: Vec<Vec<FontRecord>> =
                    (0..missing_scripts.len()).map(|_| Vec::new()).collect();
                for record in records {
                    if let Some(index) = missing_scripts
                        .iter()
                        .position(|script| script.id_base() == record.id)
                    {
                        lane_records[index].push(record);
                    }
                }
                for (script, records) in missing_scripts.into_iter().zip(lane_records) {
                    let _ = fallbacks[script.index()].set(records);
                }
            }
        }

        let mut chain = Vec::new();
        for script in FallbackScript::ALL {
            chain.extend(
                fallbacks[script.index()]
                    .get_or_init(|| build_fallback_chain_for_script(script))
                    .iter()
                    .cloned(),
            );
        }
        chain
    }
}

/// Returns the fallback faces for one script, building that lane at most once
/// for the process.
pub(crate) fn shared_fallback_chain_for_script(script: FallbackScript) -> Vec<FontRecord> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    {
        let _ = script;
        Vec::new()
    }

    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    {
        shared_fallback_lanes()[script.index()]
            .get_or_init(|| build_fallback_chain_for_script(script))
            .clone()
    }
}

/// Pre-build fallback lanes and retain their validated file mappings so first
/// use avoids walking the system font directories. Safe to call from any
/// thread; the inner `OnceLock` is also used by `GlyphRasterizer::ensure_fallbacks`.
#[cfg_attr(
    any(target_os = "ios", target_os = "macos", target_arch = "wasm32"),
    allow(dead_code)
)]
pub fn warm_fallbacks() {
    let start = aimer_utils::AnimInstant::now();
    let _ = shared_fallback_chain();
    info!("warm_fallbacks() took {} ms", start.elapsed().as_millis());
}

/// Reports whether the caller is the one that has to run the warm-up.
///
/// The warm-up is process-wide work guarded by an `OnceLock`, so a second
/// runner would not repeat it — it would *block* on the first one. Claiming
/// the flag before spawning keeps that blocked thread from existing at all,
/// and the swap makes the claim atomic, so racing callers cannot both see an
/// unclaimed flag.
#[cfg_attr(
    any(target_os = "ios", target_os = "macos", target_arch = "wasm32"),
    allow(dead_code)
)]
fn claim_warm_up(started: &AtomicBool) -> bool {
    !started.swap(true, Ordering::Relaxed)
}

/// Starts building the platform fallback chain on a background thread, at most
/// once per process.
///
/// The chain is otherwise built by the first glyph that no loaded face covers,
/// on whichever thread happens to be rasterizing — so the frame that first
/// shows CJK, emoji or any other uncovered script pays for a sweep of the
/// installed font set. Doing it ahead of time off the UI thread means that
/// frame finds the chain already built.
///
/// This is a no-op where no chain is built: Apple platforms resolve fallbacks
/// per codepoint through the system instead (see
/// [`system_fallback`](crate::system_fallback)), and wasm has no
/// threads to spare the UI one.
#[inline]
pub fn warm_fallbacks_in_background() {
    #[cfg(not(any(target_os = "ios", target_os = "macos", target_arch = "wasm32")))]
    {
        static STARTED: AtomicBool = AtomicBool::new(false);
        if !claim_warm_up(&STARTED) {
            return;
        }

        // A failure to spawn is not worth failing startup over: the chain is
        // still built lazily on first miss, exactly as it was before.
        if std::thread::Builder::new()
            .name("cupid-font-warmup".into())
            .spawn(warm_fallbacks)
            .is_err()
        {
            info!("warm_fallbacks() could not be started off the UI thread");
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_arch = "wasm32"))]
    use std::path::PathBuf;
    #[cfg(not(target_arch = "wasm32"))]
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::{
        FallbackScript, any_probe_is_mapped, android_font_priority, claim_warm_up,
        fallback_script_for_codepoint, first_open_match,
    };

    #[cfg(not(target_arch = "wasm32"))]
    use super::{FontData, FontRecord, cached_font_file, mapped_font_file, publish_font_file};

    const TEST_FONT: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");

    #[test]
    fn fallback_lane_classifier_keeps_scripts_distinct() {
        assert_eq!(
            fallback_script_for_codepoint('😀'),
            Some(FallbackScript::Emoji)
        );
        assert_eq!(
            fallback_script_for_codepoint('你'),
            Some(FallbackScript::Cjk)
        );
        assert_eq!(
            fallback_script_for_codepoint('한'),
            Some(FallbackScript::Hangul)
        );
        assert_eq!(
            fallback_script_for_codepoint('ع'),
            Some(FallbackScript::Arabic)
        );
        assert_eq!(fallback_script_for_codepoint('A'), None);
    }

    #[test]
    fn android_sans_prefers_roboto_over_alphabetically_earlier_display_faces() {
        // Pixel images ship CarroisGothicSC (small caps) and AndroidClock ahead of
        // Roboto in directory order; neither may win the generic sans slot.
        let mut names = [
            "AndroidClock.ttf",
            "CarroisGothicSC-Regular.ttf",
            "ComingSoon.ttf",
            "DancingScript-Regular.ttf",
            "DroidSans.ttf",
            "Roboto-Regular.ttf",
            "RobotoFlex-Regular.ttf",
        ];
        names.sort_by_key(|name| (android_font_priority(name, false), *name));
        assert_eq!(names[0], "Roboto-Regular.ttf");
        assert!(
            android_font_priority("DroidSans.ttf", false)
                < android_font_priority("CarroisGothicSC-Regular.ttf", false)
        );
    }

    #[test]
    fn android_monospace_prefers_droid_sans_mono() {
        assert!(
            android_font_priority("DroidSansMono.ttf", true)
                < android_font_priority("CutiveMono.ttf", true)
        );
        assert_eq!(
            android_font_priority("Roboto-Regular.ttf", true),
            android_font_priority("CutiveMono.ttf", true)
        );
    }

    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    #[test]
    fn fallback_lane_ids_are_stable_and_disjoint() {
        for (index, script) in FallbackScript::ALL.iter().copied().enumerate() {
            assert_eq!(
                script.id_base(),
                super::FALLBACK_CHAIN_ID_BASE + index as u32 * 0x1000
            );
            assert_eq!(
                super::fallback_script_for_font_id(script.id_base()),
                Some(script)
            );
            assert_eq!(
                super::fallback_script_for_font_id(script.id_base() + 1),
                Some(script)
            );
        }
    }

    #[test]
    fn aimer_face_metadata_supports_a_registered_standard_font() {
        let record = FontRecord::from_bytes(7, TEST_FONT.to_vec())
            .expect("the checked-in face should pass the Aimer validator");

        assert!(
            record
                .glyph_index('A')
                .is_some_and(|glyph_id| glyph_id != 0)
        );
        assert_eq!(record.design_weight(), Some(400));
        assert!(record.has_standard_outline());
        assert!(!record.is_color);
        assert!(
            record
                .advance_width_for_glyph(record.glyph_index('A').unwrap(), 16.0)
                .is_some_and(|advance| advance > 0.0)
        );
        assert!(
            record
                .line_metrics(16.0)
                .is_some_and(|(ascent, descent, _)| ascent > 0.0 && descent < 0.0)
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn jetbrains_record() -> FontRecord {
        FontRecord {
            id: 1,
            bytes: None,
            collection_index: 0,
            path: Some(Arc::new(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts/JetBrainsMono-Regular.ttf"),
            )),
            is_color: false,
        }
    }

    /// Fallback resolution asks every known face about every new codepoint, so a
    /// lookup that rebuilt the face re-parsed its whole character map each time.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn glyph_lookups_on_one_record_parse_its_cmap_once() {
        let record = jetbrains_record();
        let before = crate::aimer_font::cmap_parses();
        for codepoint in ('a'..='z').chain('A'..='Z') {
            assert!(record.glyph_index(codepoint).is_some());
        }
        let parses = crate::aimer_font::cmap_parses() - before;
        assert!(parses <= 1, "52 lookups parsed the cmap {parses} times");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn advance_lookups_on_one_record_parse_its_metrics_once() {
        let record = jetbrains_record();
        let glyph = record.glyph_index('a').expect("the face maps `a`");
        let before = crate::aimer_font::hmtx_parses();
        for _ in 0..50 {
            assert!(record.advance_width_for_glyph(glyph, 16.0).is_some());
        }
        let parses = crate::aimer_font::hmtx_parses() - before;
        assert!(parses <= 1, "50 advances parsed hmtx {parses} times");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn file_backed_font_data_is_memory_mapped_instead_of_heap_copied() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/font_resolver.rs");
        let record = FontRecord {
            id: 1,
            bytes: None,
            collection_index: 0,
            path: Some(Arc::new(path)),
            is_color: false,
        };

        assert!(matches!(record.data(), Some(FontData::Mapped(_))));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn file_backed_font_data_maps_each_file_once_per_process() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts/JetBrainsMono-Regular.ttf");
        let first = FontRecord {
            id: 1,
            bytes: None,
            collection_index: 0,
            path: Some(Arc::new(path.clone())),
            is_color: false,
        };
        let second = FontRecord {
            id: 2,
            bytes: None,
            collection_index: 0,
            path: Some(Arc::new(path)),
            is_color: false,
        };

        let a = first.data().expect("font file should map");
        let b = first.data().expect("font file should map");
        let c = second.data().expect("font file should map");

        assert_eq!(
            a.as_ref().as_ptr(),
            b.as_ref().as_ptr(),
            "repeated access to one record must reuse the same mapping"
        );
        assert_eq!(
            a.as_ref().as_ptr(),
            c.as_ref().as_ptr(),
            "records sharing a path must reuse the same mapping"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_path_is_only_cached_once_a_mapping_has_been_published() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");

        assert!(
            cached_font_file(&path).is_none(),
            "a file nothing has mapped yet must not be reported as cached"
        );

        let mapped = mapped_font_file(&path).expect("readable file should map");

        assert_eq!(
            cached_font_file(&path)
                .expect("a mapping handed out by `mapped_font_file` is published")
                .as_ptr(),
            mapped.as_ptr(),
            "the lookup must return the very mapping that was published"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn publishing_a_second_mapping_keeps_the_one_already_shared() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
        let shared = mapped_font_file(&path).expect("readable file should map");

        let file = std::fs::File::open(&path).expect("readable file should open");
        // SAFETY: read-only mapping of a file that stays valid for the test.
        let private = Arc::new(unsafe { memmap2::Mmap::map(&file) }.expect("file should map"));
        assert_ne!(
            private.as_ptr(),
            shared.as_ptr(),
            "a private mapping is a distinct region"
        );

        let published = publish_font_file(&path, private);

        assert_eq!(
            published.as_ptr(),
            shared.as_ptr(),
            "publishing must hand back the mapping already in use, so pointer identity holds"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn mapping_an_unreadable_path_fails_without_poisoning_the_cache() {
        let missing = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts/does-not-exist.ttf");
        let record = FontRecord {
            id: 1,
            bytes: None,
            collection_index: 0,
            path: Some(Arc::new(missing)),
            is_color: false,
        };

        assert!(record.data().is_none());
        assert!(record.glyph_index('A').is_none());

        let readable = FontRecord {
            id: 2,
            bytes: None,
            collection_index: 0,
            path: Some(Arc::new(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts/JetBrainsMono-Regular.ttf"),
            )),
            is_color: false,
        };
        assert!(readable.glyph_index('A').is_some());
    }

    #[test]
    fn a_face_claims_the_highest_priority_group_it_still_fits() {
        let open = [false, true, true];

        let claimed = first_open_match(&open, |group| (group >= 1).then_some(group == 2));

        assert_eq!(claimed, Some((1, false)));
    }

    #[test]
    fn a_face_matching_nothing_open_claims_no_group() {
        let open = [false, true, true];

        let claimed = first_open_match(&open, |group| (group == 0).then_some(false));

        assert!(claimed.is_none());
    }

    #[test]
    fn the_cheap_gate_stops_at_the_first_mapped_probe() {
        let mut looked_up = Vec::new();

        let mapped = any_probe_is_mapped(
            |codepoint| {
                looked_up.push(codepoint);
                (codepoint == '漢').then_some(42)
            },
            &['你', '漢', '한'],
        );

        assert!(mapped);
        assert_eq!(
            looked_up,
            vec!['你', '漢'],
            "the gate must answer as soon as one probe is mapped"
        );
    }

    #[test]
    fn the_cheap_gate_rejects_a_character_map_covering_no_probe() {
        assert!(!any_probe_is_mapped(|_| None, &['你', '漢', '한']));
        assert!(
            !any_probe_is_mapped(|_| Some(7), &[]),
            "a group without probes cannot be satisfied"
        );
    }

    #[test]
    fn only_the_first_claim_starts_the_warm_up() {
        let started = AtomicBool::new(false);

        assert!(
            claim_warm_up(&started),
            "the first caller is the one that must do the work"
        );
        assert!(!claim_warm_up(&started));
        assert!(!claim_warm_up(&started));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn concurrent_claims_start_the_warm_up_exactly_once() {
        let started = AtomicBool::new(false);
        let claims = std::sync::atomic::AtomicUsize::new(0);

        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    if claim_warm_up(&started) {
                        claims.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        });

        assert_eq!(
            claims.load(Ordering::Relaxed),
            1,
            "racing callers must not each spawn a chain build"
        );
    }

    #[test]
    fn each_open_group_is_probed_at_most_once_and_filled_ones_never_are() {
        let open = [true, false, true, true];
        let mut probed = Vec::new();

        let claimed = first_open_match(&open, |group| {
            probed.push(group);
            (group == 2).then_some(true)
        });

        assert_eq!(claimed, Some((2, true)));
        assert_eq!(
            probed,
            vec![0, 2],
            "probing must stop at the claimed group and skip already filled ones"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn preferred_fallback_paths_keep_the_remaining_search_order() {
        let paths = vec![
            PathBuf::from(r"C:\Fonts\a.ttf"),
            PathBuf::from(r"C:\Fonts\bar.ttf"),
            PathBuf::from(r"C:\Fonts\z.ttf"),
        ];
        let ordered = super::prioritized_fallback_paths(&paths, &["BAR.ttf", "bar.ttf"]);
        let names = ordered
            .iter()
            .map(|path| path.file_name().and_then(|name| name.to_str()).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(names, ["bar.ttf", "a.ttf", "z.ttf"]);
    }

    #[cfg(all(
        not(target_arch = "wasm32"),
        not(any(target_os = "ios", target_os = "macos"))
    ))]
    #[test]
    fn batched_fallback_discovery_matches_each_script_lane() {
        let paths = super::system_font_paths();
        let batched = super::build_fallback_chain_for_scripts(&FallbackScript::ALL, &paths);

        for script in FallbackScript::ALL {
            let separate = super::build_fallback_chain_for_scripts(&[script], &paths);
            let batched_lane = batched
                .iter()
                .filter(|record| record.id == script.id_base())
                .map(|record| {
                    (
                        record.id,
                        record.path.as_ref().map(|path| path.as_ref().clone()),
                        record.collection_index,
                        record.is_color,
                    )
                })
                .collect::<Vec<_>>();
            let separate_lane = separate
                .iter()
                .map(|record| {
                    (
                        record.id,
                        record.path.as_ref().map(|path| path.as_ref().clone()),
                        record.collection_index,
                        record.is_color,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(batched_lane, separate_lane, "lane {script:?} changed");
        }
    }
}
