//! On-demand system font fallback, resolved one codepoint at a time.
//!
//! A pre-built fallback chain has to guess which faces the application will
//! need before it has seen a single character. The usual way to guess is a
//! table of script samples, but a face selected by a sample only guarantees
//! coverage of that sample: a Japanese face picked for `漢` has no glyph for
//! `你`, and no sample at all is taken for punctuation such as `！`, so those
//! characters end up drawn as `.notdef` boxes.
//!
//! This module removes the guess. When no already-loaded face covers a
//! codepoint, the platform is asked which face draws *that* codepoint, the
//! answer is memory-mapped, and it becomes part of the fallback set. Nothing
//! is loaded for text the application never renders.
//!
//! # Stable font ids
//!
//! Text preparation is parallel: shaping, layout and rasterization run in
//! separate [`GlyphRasterizer`] copies created from a shared snapshot, and a
//! glyph is handed between them as a `(font id, glyph id)` pair. A font id
//! assigned locally by whichever worker happened to resolve the codepoint
//! first would therefore mean different faces in different workers.
//!
//! The store lives process-wide instead: a face is identified by its
//! `(path, collection index)` pair and keeps one id for the lifetime of the
//! process, so every worker resolving the same codepoint observes the same id.
//! Ids start at [`SYSTEM_FALLBACK_ID_BASE`], which keeps them clear of the
//! per-script platform-independent lanes and of fonts registered at runtime.
//!
//! [`GlyphRasterizer`]: crate::glyph_rasterizer::GlyphRasterizer
//!
//! On Apple, the discovery and private-glyph compatibility path is available
//! only with the `apple-core-text` feature. Without it, this module leaves
//! unsupported system faces unresolved so the portable profile cannot invoke
//! Core Text implicitly.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use crate::font::TextLanguage;
use crate::text_layout::FontId;
use crate::font_resolver::{FontRecord, REGULAR_WEIGHT};

/// First font id handed out to a dynamically resolved system face.
///
/// Platform-independent fallback lanes occupy a reserved range below this
/// boundary, while runtime registrations use low ids and the reserved
/// monospace id stays at `0x7fff_fffe`. Keeping the dynamic store above both
/// ranges makes ids independent of which fallback lane was loaded first.
pub(crate) const SYSTEM_FALLBACK_ID_BASE: FontId = 0x4000_0000;

/// How far a face's design weight may sit from the requested one and still be
/// treated as the face that weight asked for.
///
/// Families ship discrete cuts, so an exact hit is the exception: a request for
/// `700` on a family topping out at `Semibold` has to accept `600`. One step of
/// the `wght` scale is wide enough for that neighbour and still narrow enough
/// that `Regular` never answers for a bold run.
pub(crate) const WEIGHT_MATCH_TOLERANCE: u16 = 100;

/// Bound the process-wide lazy coverage index for each discovered face.
const COVERAGE_INDEX_GLYPH_CAPACITY: usize = 16 * 1024;
const COVERAGE_INDEX_SCRIPT_CAPACITY: usize = 512;

/// How far `record`'s design sits from the weight a run asked for.
///
/// A face hiding its `OS/2` table is read as regular: that is the weight a
/// face without a declared one is drawn at everywhere else in the pipeline.
fn weight_distance(record: &FontRecord, requested: u16) -> u16 {
    record
        .design_weight()
        .unwrap_or(REGULAR_WEIGHT)
        .abs_diff(requested)
}

/// Where a resolved face's bytes come from.
#[derive(Clone)]
enum FaceSource {
    /// A face backed by an on-disk font file, memory-mapped on use.
    File(PathBuf),
    /// A face reassembled from the platform's font tables, for fonts that no
    /// readable file backs, identified by its PostScript name.
    #[cfg_attr(not(any(target_os = "ios", target_os = "macos")), allow(dead_code))]
    Memory {
        postscript_name: String,
        bytes: Arc<[u8]>,
    },
}

/// A face discovered on demand, described by where its bytes come from.
struct ResolvedFace {
    source: FaceSource,
    collection_index: u32,
    is_color: bool,
}

/// Process-wide set of faces discovered so far, plus the answers already given.
#[derive(Default)]
struct FallbackStore {
    /// Discovered faces in discovery order; ids are `index + BASE`.
    records: Vec<FontRecord>,
    /// Maps a face's on-disk identity to the id it was assigned, so a font
    /// covering many scripts is mapped once and reused.
    ids_by_source: HashMap<(PathBuf, u32), FontId>,
    /// The same for faces carried as reassembled bytes, keyed by the
    /// PostScript name that identifies them in the platform's catalogue.
    ids_by_name: HashMap<String, FontId>,
    /// Answers for codepoints already looked up, including the negative ones,
    /// so an unresolvable codepoint costs one platform query in total. Keyed by
    /// the requirement as well, because the same character resolves to
    /// different faces depending on the script around it — see
    /// [`ScriptRequirement`] — and by the requested weight, because a bold run
    /// and a regular one want different faces of the same family.
    ids_by_codepoint: HashMap<(char, ScriptRequirement, u16), Option<FontId>>,
    /// Non-zero glyph IDs for the same immutable system-fallback answers.
    /// Keeping the mapping here avoids reopening the selected face's cmap
    /// when a fresh `GlyphRasterizer` warms the same text.
    glyphs_by_codepoint: HashMap<(char, ScriptRequirement, u16), Option<u16>>,
}

impl FallbackStore {
    fn record_by_id(&self, font_id: FontId) -> Option<&FontRecord> {
        self.records
            .get(font_id.checked_sub(SYSTEM_FALLBACK_ID_BASE)? as usize)
    }

    /// Returns the id of `face`, registering it when it is seen the first time.
    fn intern(&mut self, face: ResolvedFace) -> FontId {
        let next_id = SYSTEM_FALLBACK_ID_BASE + self.records.len() as FontId;
        match face.source {
            FaceSource::File(path) => {
                let source = (path, face.collection_index);
                if let Some(font_id) = self.ids_by_source.get(&source) {
                    return *font_id;
                }
                self.records.push(FontRecord {
                    id: next_id,
                    bytes: None,
                    collection_index: source.1,
                    path: Some(Arc::new(source.0.clone())),
                    is_color: face.is_color,
                });
                self.ids_by_source.insert(source, next_id);
            }
            FaceSource::Memory {
                postscript_name,
                bytes,
            } => {
                if let Some(font_id) = self.ids_by_name.get(&postscript_name) {
                    return *font_id;
                }
                self.records.push(FontRecord {
                    id: next_id,
                    bytes: Some(bytes),
                    collection_index: face.collection_index,
                    path: None,
                    is_color: face.is_color,
                });
                self.ids_by_name.insert(postscript_name, next_id);
            }
        }
        next_id
    }
}

static STORE: LazyLock<RwLock<FallbackStore>> =
    LazyLock::new(|| RwLock::new(FallbackStore::default()));

/// Serializes cold platform resolutions.
///
/// The store itself is safe to access concurrently, but a platform cascade
/// query happens outside its lock. Without a second check after that query,
/// two workers can observe different snapshots of the discovered faces and
/// choose different files for two codepoints in the same script. Cached hits
/// do not take this lock; only a miss pays the serialization cost.
static FALLBACK_RESOLUTION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Lazy decodability answers for immutable, process-wide fallback faces.
///
/// The fallback store keeps a face for the lifetime of the process, so these
/// answers never become stale. The cache is separate from [`FallbackStore`]'s
/// identity lock: cmap parsing stays outside that lock and two concurrent
/// misses may perform the same cold parse without blocking face interning.
#[derive(Default)]
struct FallbackCoverageIndex {
    glyphs: HashMap<char, bool>,
    scripts: HashMap<ScriptRequirement, bool>,
}

static FALLBACK_COVERAGE_INDEX: LazyLock<RwLock<HashMap<FontId, FallbackCoverageIndex>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Returns a face able to draw `codepoint`, asking the platform when needed.
///
/// The lookup is cached both ways: a resolved face is reused for every other
/// codepoint it covers, and a codepoint the platform cannot serve is
/// remembered as unresolvable so it is never queried again.
///
/// `weight` is the OpenType `wght` the run is drawn at, and it selects among
/// the family's cuts: a bold line of Han asks for the `Semibold`/`W6` face
/// Apple pairs with bold UI text instead of the regular one emboldened by
/// hand. Every weight keeps its own answer, so one run cannot decide the
/// stroke of the next.
///
/// Returns `None` when the platform exposes no per-codepoint font matching
/// (every target except macOS and iOS today), or when no installed face covers
/// `codepoint`.
pub(crate) fn fallback_for_codepoint(
    codepoint: char,
    requirement: ScriptRequirement,
    weight: u16,
) -> Option<FontRecord> {
    let cache_key = (codepoint, requirement, weight);
    if let Ok(store) = STORE.read()
        && let Some(cached) = store.ids_by_codepoint.get(&cache_key)
    {
        return cached.and_then(|font_id| store.record_by_id(font_id).cloned());
    }

    // Keep the fast cached path above the lock. Recheck after taking it because
    // another worker may have resolved this exact key while this caller was
    // waiting.
    let _resolution = FALLBACK_RESOLUTION_LOCK.lock().ok()?;
    if let Ok(store) = STORE.read()
        && let Some(cached) = store.ids_by_codepoint.get(&cache_key)
    {
        return cached.and_then(|font_id| store.record_by_id(font_id).cloned());
    }

    // Asking the platform is expensive, so faces discovered for earlier
    // codepoints get a chance first: one CJK face typically covers a whole
    // paragraph, turning every codepoint after the first into a cmap lookup.
    // A known face is accepted only if it also covers the rest of the
    // codepoint's script in the text being drawn, so a narrow face adopted for
    // a neighbouring script cannot claim half of a word — see
    // [`ScriptRequirement`] — and only if this crate decodes its glyphs itself:
    // a platform-only face admitted earlier as a last resort must not shadow a
    // decodable face a fresh resolution would find.
    // A known face is also only reused when it is designed near the weight the
    // run asked for: the regular cut adopted for an earlier line would
    // otherwise shadow the bold one a fresh resolution finds, and the emphasis
    // would be lost to whichever run happened to be drawn first.
    let known: Vec<FontRecord> = STORE.read().ok()?.records.clone();
    let decodes_script = |record: &&FontRecord| {
        cached_record_decodes_codepoint(record, codepoint)
            && cached_record_decodes_all(record, requirement)
    };
    let reusable = known
        .iter()
        .filter(decodes_script)
        .min_by_key(|record| weight_distance(record, weight))
        .filter(|record| weight_distance(record, weight) <= WEIGHT_MATCH_TOLERANCE)
        .cloned();
    if let Some(record) = reusable {
        return settle(cache_key, Some(record.id));
    }

    // Resolution touches the file system, so it happens outside the lock. A
    // concurrent resolver may win the race; interning is idempotent, so the
    // duplicate work is wasted but never observable.
    let resolved = resolve_face_for_codepoint(codepoint, requirement, weight);

    let font_id = {
        let mut store = STORE.write().ok()?;
        resolved.map(|face| store.intern(face)).or_else(|| {
            // Nothing covers the script as a whole — a platform that offers no
            // per-character matching, or a system carrying only a partial face.
            // Drawing the character in a narrow face still beats a blank box.
            known
                .iter()
                .find(|record| record_draws_codepoint(record, codepoint))
                .map(|record| record.id)
        })
    };
    settle(cache_key, font_id)
}

/// Returns the cached system face together with the glyph it uses for the
/// requested codepoint. System fallback records are immutable for the process,
/// so the glyph mapping can be shared across short-lived rasterizers too.
pub(crate) fn fallback_glyph_for_codepoint(
    codepoint: char,
    requirement: ScriptRequirement,
    weight: u16,
) -> Option<(FontRecord, u16)> {
    let cache_key = (codepoint, requirement, weight);
    if let Ok(store) = STORE.read()
        && let Some(glyph_id) = store.glyphs_by_codepoint.get(&cache_key)
    {
        let font_id = store.ids_by_codepoint.get(&cache_key).copied().flatten()?;
        let glyph_id = (*glyph_id)?;
        let record = store.record_by_id(font_id)?.clone();
        return Some((record, glyph_id));
    }

    let record = fallback_for_codepoint(codepoint, requirement, weight)?;
    let glyph_id = record
        .glyph_index(codepoint)
        .filter(|glyph_id| *glyph_id != 0);
    if let Ok(mut store) = STORE.write() {
        store.glyphs_by_codepoint.insert(cache_key, glyph_id);
    }
    glyph_id.map(|glyph_id| (record, glyph_id))
}

/// Records `font_id` as the answer to `cache_key`, keeping any answer already
/// there, and hands back the face every caller of that key will see.
///
/// Two threads missing the same key resolve side by side — the lookup reads
/// the file system and deliberately does so outside the lock — and they can
/// legitimately reach different faces: one reuses a face interned in the
/// meantime while the other asks the platform. Letting the later writer
/// overwrite the earlier would leave the two callers drawing the same
/// character in different faces, which is exactly the split a run must never
/// show. The first answer therefore stands and the loser adopts it; the
/// duplicated work is wasted but never observable.
fn settle(
    cache_key: (char, ScriptRequirement, u16),
    font_id: Option<FontId>,
) -> Option<FontRecord> {
    let mut store = STORE.write().ok()?;
    let settled = *store.ids_by_codepoint.entry(cache_key).or_insert(font_id);
    store.record_by_id(settled?).cloned()
}

/// Returns a face previously handed out by [`fallback_for_codepoint`].
///
/// Workers that only rasterize receive glyph keys resolved elsewhere and never
/// run the lookup themselves; this is how they obtain the matching face.
pub(crate) fn fallback_by_id(font_id: FontId) -> Option<FontRecord> {
    STORE.read().ok()?.record_by_id(font_id).cloned()
}

/// Resolves the generic sans-serif face without embedding a font in the
/// application binary.
///
/// Apple uses its language-aware Core Text cascade. Other desktop targets use
/// the existing on-disk font discovery path, which maps only the selected face
/// and keeps the mapping shared with later fallback lookups.
pub(crate) fn system_primary_font() -> Option<FontRecord> {
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    {
        return fallback_for_codepoint('A', ScriptRequirement::EMPTY, REGULAR_WEIGHT).filter(
            |record| {
                record
                    .glyph_index('A')
                    .is_some_and(|glyph_id| glyph_id != 0)
            },
        );
    }

    #[cfg(not(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    )))]
    {
        crate::font_resolver::system_primary_font()
    }
}

/// Resolves the generic monospace face from the host font catalogue.
pub(crate) fn system_monospace_font(font_id: FontId) -> Option<FontRecord> {
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    {
        use crate::apple_fonts::system_font_path;

        for family in ["SF Mono", "Menlo", "Monaco", "Courier New"] {
            let Some(path) = system_font_path(family) else {
                continue;
            };
            let Some(record) = FontRecord::from_file(font_id, path, 0) else {
                continue;
            };
            if ['M', 'i', '0']
                .iter()
                .all(|&codepoint| record.glyph_index(codepoint).is_some_and(|id| id != 0))
            {
                return Some(record);
            }
        }
        None
    }

    #[cfg(not(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    )))]
    {
        crate::font_resolver::system_monospace_font(font_id)
    }
}

/// Sample characters that a face serving `codepoint`'s script should cover.
///
/// Asking the platform about a single character is not enough to choose a
/// face. Apple's cascade answers with the face the *device's language* prefers,
/// so on a Japanese system every Han character a Japanese face happens to carry
/// — `你`, `好`, `多`, `就` — is served by Hiragino Sans, while the characters
/// only Chinese faces carry — `吗`, `顶` — fall through to PingFang. One word
/// then arrives in two typefaces of different stroke weight, which is what a
/// reader sees as "some characters are thinner".
///
/// The probes make the choice about the *script* instead of the character: a
/// candidate face is preferred only when it draws these too, which rules out a
/// face covering just the part of the script the platform's language prefers.
/// The Han probes are deliberately simplified-only characters, so the face that
/// wins is the one Apple ships for Chinese.
///
/// An empty slice means "no script-wide expectation": the character is judged
/// on its own, exactly as before.
///
/// The rule is not the platform query's alone: a face already loaded for a
/// neighbouring script sits in the rasterizer's own chain and is consulted
/// first, so [`GlyphRasterizer`] applies the same probes before accepting one.
///
/// [`GlyphRasterizer`]: crate::glyph_rasterizer::GlyphRasterizer
pub(crate) fn script_probes(codepoint: char) -> &'static [char] {
    /// Simplified-only Han characters, absent from Japanese-only faces.
    const HAN: [char; 3] = ['吗', '顶', '这'];

    match codepoint as u32 {
        // CJK Unified Ideographs, its extension A, and the compatibility block:
        // the range a Japanese and a Chinese face both claim in part.
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => &HAN,
        // Ideographic supplements live beyond the BMP and are carried by the
        // same faces.
        0x20000..=0x3FFFF => &HAN,
        _ => &[],
    }
}

/// The characters a face must draw to be accepted for a codepoint's script.
///
/// Han is unified, so a Japanese face carries every ideograph Japanese shares
/// with Chinese and none of the rest. Asked about one character at a time, the
/// platform hands out whichever face the device's language prefers, and a word
/// arrives in two typefaces of different stroke weight — the defect that made
/// `你好吗` render `好` lighter than its neighbours on a Japanese phone.
///
/// The requirement turns the question from "who draws this character" into "who
/// draws this script", and the honest answer to *which* script depends on the
/// text: `時` inside `あの時は` belongs to a Japanese word and must keep the face
/// its kana use, while `好` inside `你好吗` must not. So the preferred form is
/// [`ScriptRequirement::from_run`], built from the characters actually present
/// in the run being drawn; [`ScriptRequirement::probes`] is the contextless
/// fallback for callers holding a single character, and asks for the simplified
/// only samples a Japanese face never carries.
///
/// The set is a fixed inline array, so passing one costs no allocation.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) struct ScriptRequirement {
    chars: [char; Self::CAPACITY],
    len: usize,
    language: Option<TextLanguage>,
}

impl ScriptRequirement {
    /// Most characters a requirement keeps.
    ///
    /// A run longer than this is represented by its first distinct characters:
    /// the point is to tell one face's half of a script from another's, and a
    /// dozen ideographs settle that as surely as a thousand.
    const CAPACITY: usize = 12;

    /// A requirement any face satisfies — "judge the character on its own".
    pub(crate) const EMPTY: Self = Self {
        chars: ['\0'; Self::CAPACITY],
        len: 0,
        language: None,
    };

    /// The static samples for `codepoint`'s script, for callers with no run.
    pub(crate) fn probes(codepoint: char) -> Self {
        Self::from_chars(script_probes(codepoint).iter().copied())
    }

    /// The characters a face must draw to serve `run`, written in `language`.
    ///
    /// The run's own characters come first in the sense that matters: a face
    /// has to draw every ideograph actually being rendered. What they cannot
    /// settle is *which* language's ideographs those are, because Han is
    /// unified — `你好` is drawn by a Japanese face as readily as by a Chinese
    /// one, and stays on whichever the platform's cascade prefers until `吗` is
    /// typed and no Japanese face covers the run any more. That is the moment a
    /// word visibly changes typeface mid-typing.
    ///
    /// `language` closes that gap with knowledge the text does not carry — on
    /// iOS the language of the keyboard the field is being typed on. A Chinese
    /// run therefore also demands [`script_probes`]' simplified-only samples,
    /// which no Japanese face carries, so the very first character already
    /// lands on the face the whole word will end on.
    ///
    /// The hint never overrules the text: a run carrying kana is Japanese and a
    /// run carrying hangul is Korean whichever keyboard produced them, and
    /// neither demands Chinese coverage. Without a hint and without such
    /// evidence the run is judged on its own characters, which is what every
    /// caller that knows nothing about the text gets.
    ///
    /// Empty when `run` holds no partly covered script, which leaves every
    /// other script on the plain per-character path.
    pub(crate) fn from_run(run: &str, language: Option<TextLanguage>) -> Self {
        let ideographs = || {
            run.chars()
                .filter(|codepoint| !script_probes(*codepoint).is_empty())
        };
        let Some(first) = ideographs().next() else {
            return Self::EMPTY;
        };

        let language = written_language(run).or(language);

        // The samples are pushed ahead of the run's own characters so a run
        // longer than the inline array keeps the part that decides the script.
        let mut requirement = Self {
            language,
            ..Self::EMPTY
        };
        if language == Some(TextLanguage::Chinese) {
            requirement.extend(script_probes(first).iter().copied());
        }
        requirement.extend(ideographs());
        requirement
    }

    /// Reports whether any face satisfies this requirement.
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The characters a face must draw.
    pub(crate) fn as_slice(&self) -> &[char] {
        &self.chars[..self.len]
    }

    /// The language hint that should lead a platform CJK cascade, when the
    /// run supplied one and its characters did not overrule it.
    pub(crate) fn language(&self) -> Option<TextLanguage> {
        self.language
    }

    /// A compact stable hash for diagnostics and cache-related tests.
    ///
    /// Production caches use the complete requirement value as their key, so a
    /// hash collision can only affect this diagnostic value, never fallback
    /// selection.
    #[cfg(test)]
    pub(crate) fn fingerprint(&self) -> u64 {
        // FNV-1a: the set is at most a dozen characters, so a hasher would cost
        // more than the hash.
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let language = match self.language {
            None => 0,
            Some(TextLanguage::Chinese) => 1,
            Some(TextLanguage::Japanese) => 2,
            Some(TextLanguage::Korean) => 3,
        };
        hash ^= language;
        hash = hash.wrapping_mul(0x100_0000_01b3);
        for codepoint in self.as_slice() {
            hash ^= u64::from(*codepoint as u32);
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        hash
    }

    /// Collects distinct characters, stopping at [`Self::CAPACITY`].
    fn from_chars(source: impl Iterator<Item = char>) -> Self {
        let mut requirement = Self::EMPTY;
        requirement.extend(source);
        requirement
    }

    /// Appends the distinct characters of `source`, stopping at
    /// [`Self::CAPACITY`].
    fn extend(&mut self, source: impl Iterator<Item = char>) {
        for codepoint in source {
            if self.len == Self::CAPACITY {
                break;
            }
            if self.as_slice().contains(&codepoint) {
                continue;
            }
            self.chars[self.len] = codepoint;
            self.len += 1;
        }
    }
}

/// The language `run` writes itself in, when its own characters say so.
///
/// Kana and hangul are written by one language each, so an ideograph standing
/// among them belongs to that language's word and must keep the face its
/// neighbours use. Han alone says nothing — that is the question a caller's
/// hint answers.
fn written_language(run: &str) -> Option<TextLanguage> {
    run.chars().find_map(|codepoint| match codepoint as u32 {
        // Hiragana, katakana, the katakana phonetic extensions and halfwidth
        // katakana.
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9D => Some(TextLanguage::Japanese),
        // Hangul syllables, jamo, and the compatibility jamo block.
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF => {
            Some(TextLanguage::Korean)
        }
        _ => None,
    })
}

/// Reports whether `record` can draw every character of `required`.
///
/// An empty `required` is satisfied by any face, which is what keeps scripts
/// without a probe set on the plain per-character path.
#[cfg(all(test, any(target_os = "ios", target_os = "macos")))]
fn record_draws_all(record: &FontRecord, required: &[char]) -> bool {
    required
        .iter()
        .all(|codepoint| record_draws_codepoint(record, *codepoint))
}

/// Reports whether this crate can decode `codepoint`, using the process-wide
/// face coverage index when the answer has already been established.
fn cached_record_decodes_codepoint(record: &FontRecord, codepoint: char) -> bool {
    if let Ok(index) = FALLBACK_COVERAGE_INDEX.read()
        && let Some(face) = index.get(&record.id)
        && let Some(decoded) = face.glyphs.get(&codepoint)
    {
        return *decoded;
    }

    let decoded = record_decodes_codepoint(record, codepoint);
    if let Ok(mut index) = FALLBACK_COVERAGE_INDEX.write() {
        let face = index.entry(record.id).or_default();
        if face.glyphs.len() >= COVERAGE_INDEX_GLYPH_CAPACITY {
            face.glyphs.clear();
        }
        face.glyphs.insert(codepoint, decoded);
    }
    decoded
}

/// Reports whether this crate can decode every character of `required` from
/// `record`, caching the complete script decision as well as its cmap probes.
fn cached_record_decodes_all(record: &FontRecord, required: ScriptRequirement) -> bool {
    if required.is_empty() {
        return true;
    }
    if let Ok(index) = FALLBACK_COVERAGE_INDEX.read()
        && let Some(face) = index.get(&record.id)
        && let Some(decoded) = face.scripts.get(&required)
    {
        return *decoded;
    }

    let decoded = required
        .as_slice()
        .iter()
        .all(|codepoint| cached_record_decodes_codepoint(record, *codepoint));
    if let Ok(mut index) = FALLBACK_COVERAGE_INDEX.write() {
        let face = index.entry(record.id).or_default();
        if face.scripts.len() >= COVERAGE_INDEX_SCRIPT_CAPACITY {
            face.scripts.clear();
        }
        face.scripts.insert(required, decoded);
    }
    decoded
}

/// Reports whether `record` can draw `codepoint`, not merely map it.
///
/// The distinction matters: font files exist whose character map covers a
/// codepoint while the face behind it carries no glyph data at all, and
/// accepting one of those reintroduces the blank boxes this module exists to
/// remove.
fn record_draws_codepoint(record: &FontRecord, codepoint: char) -> bool {
    if record_decodes_codepoint(record, codepoint) {
        return true;
    }
    record
        .glyph_index(codepoint)
        .is_some_and(|glyph_id| glyph_id != 0 && platform_draws_glyph(record, glyph_id))
}

/// Reports whether this crate can rasterize `codepoint` from `record` itself.
///
/// A face passes when its glyph carries data Cupid's own rasterizers decode —
/// a color strike or an outline with a non-empty bounding box. Faces that only
/// the platform's off-screen rasterizer can draw do not pass; those are
/// tolerated as a last resort, never preferred, because the platform layer
/// renders with different antialiasing and its private faces do not match the
/// stroke weight of the faces drawn by Cupid in the same line.
fn record_decodes_codepoint(record: &FontRecord, codepoint: char) -> bool {
    let Some(data) = record.data() else {
        return false;
    };
    let Ok(face) = crate::aimer_font::SfntFace::from_bytes(
        data.as_ref(),
        record.collection_index,
    ) else {
        return false;
    };
    let Ok(Some(glyph_id)) = face.glyph_index(codepoint as u32) else {
        return false;
    };
    if glyph_id == 0 {
        return false;
    }
    if record.is_color {
        return true;
    }
    face.outline(glyph_id)
        .ok()
        .flatten()
        .is_some_and(|outline| !outline.contours.is_empty())
        || face
            .cff_outline(glyph_id)
            .ok()
            .flatten()
            .is_some_and(|outline| !outline.commands.is_empty())
}

/// Reports whether the platform can draw a glyph this crate cannot decode.
///
/// Apple's system faces keep their glyph data in private formats — `hvgl`
/// outlines, `emjc` strikes — so "no readable outline" does not mean "no
/// glyph": the platform rasterizer draws these, and rejecting the face would
/// leave the codepoint with no face at all.
#[cfg(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
))]
fn platform_draws_glyph(record: &FontRecord, glyph_id: u16) -> bool {
    record.path().is_some_and(|path| {
        crate::core_text_raster::draws_glyph(path, record.collection_index, glyph_id)
    })
}

/// Platforms whose fonts this crate decodes itself.
#[cfg(not(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
)))]
fn platform_draws_glyph(_record: &FontRecord, _glyph_id: u16) -> bool {
    false
}

/// Asks the platform which installed face draws `codepoint`.
///
/// The platform proposes candidates in preference order and each is checked
/// with [`face_drawing`] until one can actually be drawn. Validation
/// is not optional: `CTFontCreateForString` never fails outright — it returns
/// the font it was queried with when nothing matches — and its preferred
/// answer for CJK on recent macOS builds is a private file carrying no glyph
/// data the owned outline decoder can read.
///
/// Candidates are examined in two passes: a face whose glyphs this crate
/// decodes itself wins outright, and only when no such face covers the request
/// may one that needs the platform's off-screen rasterizer be chosen. The
/// platform layer renders with different antialiasing and its private CJK
/// faces do not match the stroke weight of the faces Cupid draws in the same
/// line, so it is a last resort, never a preference.
#[cfg(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
))]
fn resolve_face_for_codepoint(
    codepoint: char,
    requirement: ScriptRequirement,
    weight: u16,
) -> Option<ResolvedFace> {
    use crate::apple_fonts::{
        SystemFaceSource, fallback_font_path_for_probes, font_paths_for_codepoint,
        language_fallback_sources,
    };

    let required: Vec<char> = std::iter::once(codepoint)
        .chain(requirement.as_slice().iter().copied())
        .collect();
    let probes = requirement.as_slice();

    // The language-aware cascades answer first: for every UI language the
    // platform names the concrete face it pairs with the system font — stroke
    // weight included — so their answers blend with the rest of the line. The
    // plain cascade and the catalogue matches follow as backstops.
    let mut candidates: Vec<FaceSource> = Vec::new();
    let push_path = |candidates: &mut Vec<FaceSource>, path: PathBuf| {
        let seen = candidates
            .iter()
            .any(|source| matches!(source, FaceSource::File(existing) if *existing == path));
        if !seen {
            candidates.push(FaceSource::File(path));
        }
    };
    for source in language_fallback_sources(&required, requirement.language(), weight) {
        match source {
            SystemFaceSource::File(path) => push_path(&mut candidates, path),
            SystemFaceSource::Data {
                postscript_name,
                bytes,
            } => candidates.push(FaceSource::Memory {
                postscript_name,
                bytes: Arc::from(bytes),
            }),
        }
    }
    if !probes.is_empty() {
        // Asked about the whole script at once, the platform proposes a face
        // covering all of it rather than the one its language prefers for this
        // single character.
        if let Some(path) = fallback_font_path_for_probes(&required, weight) {
            push_path(&mut candidates, path);
        }
    }
    for path in font_paths_for_codepoint(codepoint, weight) {
        push_path(&mut candidates, path);
    }

    // A face covering the script wins over one covering the character alone,
    // so a script no face covers whole still gets drawn instead of falling
    // back to a blank box; within each rung, self-decodable faces win over
    // platform-only ones.
    if !probes.is_empty() {
        for strict_weight in [true, false] {
            for accept_platform in [false, true] {
                if let Some(face) = candidates.iter().find_map(|source| {
                    face_drawing(source, &required, weight, accept_platform, strict_weight)
                }) {
                    return Some(face);
                }
            }
        }
    }

    for strict_weight in [true, false] {
        for accept_platform in [false, true] {
            if let Some(face) = candidates.iter().find_map(|source| {
                face_drawing(source, &[codepoint], weight, accept_platform, strict_weight)
            }) {
                return Some(face);
            }
        }
    }
    None
}

/// Returns the face of `source` able to draw every character of `required`.
///
/// A font file is not a face: collections (`.ttc`) hold many, and only some may
/// map the characters. Each face is checked for a mapping *and* for renderable
/// glyph data — a color strike or an outline with a non-empty bounding box —
/// because a cmap entry alone is satisfied by faces that draw nothing. Only
/// with `accept_platform` may a file-backed glyph that solely the platform
/// rasterizer draws qualify; reassembled bytes have no file the platform could
/// raster from, so they qualify on decodable data alone.
#[cfg(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
))]
fn face_drawing(
    source: &FaceSource,
    required: &[char],
    weight: u16,
    accept_platform: bool,
    strict_weight: bool,
) -> Option<ResolvedFace> {
    match source {
        FaceSource::File(path) => {
            let file = std::fs::File::open(path).ok()?;
            // SAFETY: the read-only mapping is confined to this function; the
            // record built from it re-maps the file lazily through
            // `FontRecord::data`.
            let data = unsafe { memmap2::Mmap::map(&file).ok()? };
            let (collection_index, is_color) = face_in_data_drawing(
                data.as_ref(),
                required,
                weight,
                strict_weight,
                |collection_index, glyph_id| {
                    accept_platform
                        && crate::core_text_raster::draws_glyph(
                            path,
                            collection_index,
                            glyph_id,
                        )
                },
            )?;
            Some(ResolvedFace {
                source: source.clone(),
                collection_index,
                is_color,
            })
        }
        FaceSource::Memory { bytes, .. } => {
            let (collection_index, is_color) =
                face_in_data_drawing(bytes.as_ref(), required, weight, strict_weight, |_, _| {
                    false
                })?;
            Some(ResolvedFace {
                source: source.clone(),
                collection_index,
                is_color,
            })
        }
    }
}

/// Returns the face of `data` covering `required` that `weight` asked for.
///
/// A font file is not one face: a collection such as `PingFang.ttc` holds
/// every cut of the family, and taking the first one that covers the text
/// always yields the lightest — which is how a bold line of Han ended up
/// drawn at the regular stroke. Among the covering faces the one designed
/// closest to `weight` wins, ties going to the earlier face so a request at
/// the regular weight keeps naming the face it always did.
///
/// A glyph qualifies through a color strike, an outline with a non-empty
/// bounding box, or — for glyph data this crate cannot decode — through
/// `platform_draws`, which the caller scopes to what its candidate allows.
#[cfg(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
))]
fn face_in_data_drawing(
    data: &[u8],
    required: &[char],
    weight: u16,
    strict_weight: bool,
    platform_draws: impl Fn(u32, u16) -> bool,
) -> Option<(u32, bool)> {
    let mut candidates = Vec::new();
    for collection_index in 0..64 {
        let Ok(face) =
            crate::aimer_font::SfntFace::from_bytes(data, collection_index)
        else {
            break;
        };
        let is_color = face.has_color_tables() || face.has_apple_private_color_tables();
        let draws = |codepoint: &char| {
            let Ok(Some(glyph_id)) = face.glyph_index(*codepoint as u32) else {
                return false;
            };
            if glyph_id == 0 {
                return false;
            }
            let has_outline = face.outline(glyph_id).ok().flatten().is_some()
                || face.cff_outline(glyph_id).ok().flatten().is_some();
            is_color || has_outline || platform_draws(collection_index, glyph_id)
        };
        if required.iter().all(draws) {
            let design = face.design_weight().unwrap_or(REGULAR_WEIGHT);
            let distance = design.abs_diff(weight);
            if strict_weight && distance > WEIGHT_MATCH_TOLERANCE {
                continue;
            }
            candidates.push((collection_index, is_color, distance));
        }
    }
    candidates
        .into_iter()
        .min_by_key(|(collection_index, _, distance)| (*distance, *collection_index))
        .map(|(collection_index, is_color, _)| (collection_index, is_color))
}

/// Platforms without per-codepoint font matching keep the pre-built chain.
#[cfg(not(all(
    any(target_os = "ios", target_os = "macos"),
    feature = "apple-core-text"
)))]
fn resolve_face_for_codepoint(
    _codepoint: char,
    _requirement: ScriptRequirement,
    _weight: u16,
) -> Option<ResolvedFace> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        not(feature = "apple-core-text")
    ))]
    #[test]
    fn portable_profile_does_not_resolve_apple_system_faces() {
        assert!(
            resolve_face_for_codepoint('吗', ScriptRequirement::probes('吗'), REGULAR_WEIGHT,)
                .is_none()
        );
    }

    /// The `wght` value an emphasized run asks its faces to be designed at.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    const BOLD_WEIGHT: u16 = 700;

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn resolves_a_codepoint_no_probe_group_covers() {
        let record = fallback_for_codepoint('！', ScriptRequirement::probes('！'), REGULAR_WEIGHT)
            .expect("fullwidth punctuation must resolve");
        assert!(record.id >= SYSTEM_FALLBACK_ID_BASE);
        assert!(matches!(record.glyph_index('！'), Some(glyph_id) if glyph_id != 0));
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn assigns_one_stable_id_per_face() {
        let requirement = ScriptRequirement::probes('你');
        let first = fallback_for_codepoint('你', requirement, REGULAR_WEIGHT)
            .expect("han characters must resolve");
        let again = fallback_for_codepoint('你', requirement, REGULAR_WEIGHT)
            .expect("cached lookups must resolve too");
        assert_eq!(first.id, again.id);

        let shared_face = fallback_for_codepoint('好', requirement, REGULAR_WEIGHT)
            .expect("han characters must resolve");
        assert_eq!(
            shared_face.id, first.id,
            "codepoints served by the same file must share one id"
        );
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn caches_the_selected_glyph_with_the_stable_face_answer() {
        let requirement = ScriptRequirement::probes('你');
        let (record, glyph_id) = fallback_glyph_for_codepoint('你', requirement, REGULAR_WEIGHT)
            .expect("the platform fallback must expose a drawable Han glyph");
        assert_ne!(glyph_id, 0);

        let store = STORE.read().expect("fallback store read lock");
        let key = ('你', requirement, REGULAR_WEIGHT);
        assert_eq!(store.ids_by_codepoint.get(&key), Some(&Some(record.id)));
        assert_eq!(store.glyphs_by_codepoint.get(&key), Some(&Some(glyph_id)));
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn exposes_resolved_faces_by_id_for_rasterizing_workers() {
        let record = fallback_for_codepoint('한', ScriptRequirement::probes('한'), REGULAR_WEIGHT)
            .expect("hangul must resolve");
        let by_id = fallback_by_id(record.id).expect("a resolved face must be addressable by id");
        assert_eq!(by_id.id, record.id);
        assert_eq!(by_id.collection_index, record.collection_index);
    }

    // A kanji standing among kana belongs to a Japanese word, so the only
    // coverage its face owes is that word's own ideographs — asking for
    // simplified-only samples would push it onto a Chinese face and leave the
    // kana beside it in a lighter one.
    #[test]
    fn a_japanese_run_requires_only_the_ideographs_it_contains() {
        assert_eq!(
            ScriptRequirement::from_run("あの時は", None).as_slice(),
            &['時'],
            "a japanese run may not demand chinese-only coverage"
        );
    }

    #[test]
    fn a_chinese_run_requires_every_ideograph_it_contains() {
        assert_eq!(
            ScriptRequirement::from_run("你好吗", None).as_slice(),
            &['你', '好', '吗'],
            "a face drawing half of a chinese word may not be accepted for it"
        );
    }

    // The reported conflict: `你好` is written in Japanese too, so a run holding
    // nothing else is covered by a Japanese face and keeps it — until `吗` is
    // typed, no Japanese face covers the run any more, and the word jumps to
    // PingFang mid-typing. A run known to be Chinese must demand the samples no
    // Japanese face carries from the first character on.
    #[test]
    fn a_chinese_run_of_shared_han_demands_the_samples_only_chinese_faces_carry() {
        let requirement = ScriptRequirement::from_run("你好", Some(TextLanguage::Chinese));
        for drawn in ['你', '好'] {
            assert!(
                requirement.as_slice().contains(&drawn),
                "the characters drawn must stay part of the requirement: {requirement:?}"
            );
        }
        for sample in script_probes('你') {
            assert!(
                requirement.as_slice().contains(sample),
                "a chinese run accepted a face without {sample:?}: {requirement:?}"
            );
        }
    }

    // Kanji standing alone is ordinary Japanese, and demanding Chinese-only
    // samples for it would take the whole interface off the Japanese face.
    #[test]
    fn a_japanese_run_of_shared_han_keeps_only_its_own_ideographs() {
        assert_eq!(
            ScriptRequirement::from_run("日本語", Some(TextLanguage::Japanese)).as_slice(),
            &['日', '本', '語'],
            "a japanese run may not demand chinese-only coverage"
        );
    }

    // The language is a hint about what the characters cannot say, so the
    // moment they can say it themselves the hint is worth nothing: kana in the
    // run is Japanese whichever keyboard produced it.
    #[test]
    fn the_characters_overrule_the_language_they_were_typed_in() {
        assert_eq!(
            ScriptRequirement::from_run("あの時は", Some(TextLanguage::Chinese)).as_slice(),
            &['時'],
            "kana in the run must settle the script by itself"
        );
        assert_eq!(
            ScriptRequirement::from_run("한글漢字", Some(TextLanguage::Chinese)).as_slice(),
            &['漢', '字'],
            "hangul in the run must settle the script by itself"
        );
    }

    // Korean writes hanja with its own faces and shares nothing with the
    // Chinese samples, so a Korean run is judged on its own characters.
    #[test]
    fn a_korean_run_keeps_only_its_own_ideographs() {
        assert_eq!(
            ScriptRequirement::from_run("漢字", Some(TextLanguage::Korean)).as_slice(),
            &['漢', '字']
        );
    }

    #[test]
    fn the_language_hint_is_part_of_script_requirement_identity() {
        let japanese = ScriptRequirement::from_run("漢字", Some(TextLanguage::Japanese));
        let chinese = ScriptRequirement::from_run("漢字", Some(TextLanguage::Chinese));

        assert_eq!(japanese.language(), Some(TextLanguage::Japanese));
        assert_eq!(chinese.language(), Some(TextLanguage::Chinese));
        assert_ne!(japanese, chinese);
    }

    #[test]
    fn fallback_coverage_index_memoizes_decodability_per_face() {
        let font_id = 0x3fff_ff00;
        let record = FontRecord::from_bytes(
            font_id,
            include_bytes!("../fonts/GoogleSans-Regular.ttf").to_vec(),
        )
        .expect("the bundled test face should be readable");
        let requirement = ScriptRequirement::probes('你');

        assert!(cached_record_decodes_codepoint(&record, 'A'));
        assert!(!cached_record_decodes_all(&record, requirement));
        let first = FALLBACK_COVERAGE_INDEX
            .read()
            .expect("coverage index lock should be available");
        let first_face = first
            .get(&font_id)
            .expect("the first fallback query must create a face index");
        let first_glyph_count = first_face.glyphs.len();
        assert_eq!(first_face.scripts.len(), 1);
        drop(first);

        assert!(cached_record_decodes_codepoint(&record, 'A'));
        assert!(!cached_record_decodes_all(&record, requirement));
        let second = FALLBACK_COVERAGE_INDEX
            .read()
            .expect("coverage index lock should be available");
        let second_face = second
            .get(&font_id)
            .expect("the repeated fallback query must reuse the face index");
        assert_eq!(second_face.glyphs.len(), first_glyph_count);
        assert_eq!(second_face.scripts.len(), 1);
    }

    #[test]
    fn runs_without_ideographs_require_nothing() {
        for run in ["Hello, world!", "あいうえお", "한글", "😀"] {
            for language in [
                None,
                Some(TextLanguage::Chinese),
                Some(TextLanguage::Japanese),
                Some(TextLanguage::Korean),
            ] {
                assert!(
                    ScriptRequirement::from_run(run, language).is_empty(),
                    "{run:?} holds no script a face may cover in part"
                );
            }
        }
    }

    #[test]
    fn a_requirement_keeps_distinct_characters_within_its_capacity() {
        let repeated = ScriptRequirement::from_run("時時時好", None);
        assert_eq!(repeated.as_slice(), &['時', '好']);

        let long: String = ('\u{4e00}'..='\u{4e20}').collect();
        assert_eq!(
            ScriptRequirement::from_run(&long, Some(TextLanguage::Chinese))
                .as_slice()
                .len(),
            ScriptRequirement::CAPACITY,
            "a long run must stay within the inline array"
        );
    }

    #[test]
    fn requirements_are_fingerprinted_by_their_characters_and_language() {
        assert_eq!(
            ScriptRequirement::from_run("あの時は", None).fingerprint(),
            ScriptRequirement::from_run("時", Some(TextLanguage::Japanese)).fingerprint(),
            "the same demanded characters and language must share one cache key"
        );
        assert_ne!(
            ScriptRequirement::from_run("時", None).fingerprint(),
            ScriptRequirement::from_run("你好吗", None).fingerprint(),
            "different demands may not share cached answers"
        );
        assert_ne!(
            ScriptRequirement::from_run("時", None).fingerprint(),
            ScriptRequirement::from_run("時", Some(TextLanguage::Chinese)).fingerprint(),
            "a run claimed by chinese may not reuse the japanese answer"
        );
        assert_ne!(
            ScriptRequirement::EMPTY.fingerprint(),
            ScriptRequirement::from_run("時", None).fingerprint()
        );
    }

    #[test]
    fn fallback_cache_keys_include_the_complete_script_requirement() {
        let codepoint = '好';
        let japanese = ScriptRequirement::from_run("時", None);
        let chinese = ScriptRequirement::from_run("吗", None);
        let mut store = FallbackStore::default();

        store
            .ids_by_codepoint
            .insert((codepoint, japanese, REGULAR_WEIGHT), Some(1));
        store
            .ids_by_codepoint
            .insert((codepoint, chinese, REGULAR_WEIGHT), Some(2));

        assert_eq!(store.ids_by_codepoint.len(), 2);
        assert_eq!(
            store
                .ids_by_codepoint
                .get(&(codepoint, japanese, REGULAR_WEIGHT)),
            Some(&Some(1))
        );
        assert_eq!(
            store
                .ids_by_codepoint
                .get(&(codepoint, chinese, REGULAR_WEIGHT)),
            Some(&Some(2))
        );
    }

    #[test]
    fn only_han_asks_for_script_wide_coverage() {
        for codepoint in ['你', '吗', '漢', '\u{3400}', '\u{20000}'] {
            assert!(
                !script_probes(codepoint).is_empty(),
                "{codepoint:?} shares its block with faces covering only part of it"
            );
        }
        for codepoint in ['A', 'あ', 'ア', '한', '！', '😀', 'ü'] {
            assert!(
                script_probes(codepoint).is_empty(),
                "{codepoint:?} needs no script-wide probe"
            );
        }
    }

    // The defect this rule exists for: Apple's cascade answers per character
    // and per *device language*, so on a Japanese system the Han characters a
    // Japanese face happens to carry are served by it and the simplified-only
    // ones fall through to a Chinese face — one word, two typefaces, two
    // stroke weights.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn one_word_of_han_is_served_by_one_face() {
        let faces: Vec<_> = "你好吗？顶多就"
            .chars()
            .filter(|codepoint| !script_probes(*codepoint).is_empty())
            .map(|codepoint| {
                let record = fallback_for_codepoint(
                    codepoint,
                    ScriptRequirement::probes(codepoint),
                    REGULAR_WEIGHT,
                )
                .expect("han must resolve");
                (codepoint, record.id, record.collection_index)
            })
            .collect();

        let first = faces.first().expect("the sample carries han characters").1;
        assert!(
            faces.iter().all(|(_, id, _)| *id == first),
            "han characters split across faces: {faces:?}"
        );
    }

    // The device's language decides which of these Core Text offers first, so
    // the rule cannot be asserted through the cascade on a developer machine.
    // It can be asserted where it acts: a face carrying only the Japanese half
    // of Han must be rejected for `你` even though it draws `你` perfectly.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_face_covering_only_half_of_han_is_not_chosen_for_it() {
        use crate::apple_fonts::font_paths_for_codepoint;

        let required: Vec<char> = std::iter::once('你')
            .chain(script_probes('你').iter().copied())
            .collect();
        let partial: Vec<_> = font_paths_for_codepoint('你', REGULAR_WEIGHT)
            .into_iter()
            .filter(|path| {
                let source = FaceSource::File(path.clone());
                face_drawing(&source, &['你'], REGULAR_WEIGHT, true, false).is_some()
                    && face_drawing(&source, &required, REGULAR_WEIGHT, true, false).is_none()
            })
            .collect();

        assert!(
            !partial.is_empty(),
            "apple systems always ship a japanese face carrying 你 but not 吗"
        );

        let chosen =
            resolve_face_for_codepoint('你', ScriptRequirement::probes('你'), REGULAR_WEIGHT)
                .expect("han must resolve");
        let FaceSource::File(chosen_path) = &chosen.source else {
            return; // A reassembled face is never one of the partial files.
        };
        assert!(
            !partial.contains(chosen_path),
            "chose a face covering only part of han: {chosen_path:?}"
        );
    }

    // The screenshot defect this fix exists for: a Japanese line holding a few
    // simplified-only characters had its whole Han run pushed onto a private
    // face only the platform's off-screen rasterizer could draw, so every
    // kanji came back heavier than the kana beside it. Apple pairs the UI font
    // of every CJK language with faces Cupid decodes itself (Hiragino Sans GB
    // for simplified Han on a Japanese system), so whenever such a face covers
    // the requirement it must win over a platform-only face.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn han_resolves_to_a_face_cupid_decodes_itself() {
        for run in ["你好吗", "あの時は你好吗"] {
            let requirement = ScriptRequirement::from_run(run, None);
            for codepoint in run.chars().filter(|cp| !script_probes(*cp).is_empty()) {
                let record = fallback_for_codepoint(codepoint, requirement, REGULAR_WEIGHT)
                    .expect("han must resolve");
                assert!(
                    record_decodes_codepoint(&record, codepoint),
                    "{codepoint:?} of {run:?} resolved to a face only the platform \
                     rasterizer draws: {:?}",
                    record.path
                );
            }
        }
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn the_han_face_draws_the_probes_it_was_chosen_for() {
        let record = fallback_for_codepoint('你', ScriptRequirement::probes('你'), REGULAR_WEIGHT)
            .expect("han must resolve");

        assert!(record_draws_all(&record, script_probes('你')));
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn detects_color_faces_for_emoji() {
        let record = fallback_for_codepoint('😀', ScriptRequirement::EMPTY, REGULAR_WEIGHT)
            .expect("emoji must resolve");
        assert!(record.is_color, "emoji faces carry color glyph tables");
    }

    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn resolved_faces_can_actually_be_rasterized() {
        for codepoint in ['你', '好', '！', '，'] {
            let record = fallback_for_codepoint(
                codepoint,
                ScriptRequirement::probes(codepoint),
                REGULAR_WEIGHT,
            )
            .expect("must resolve");
            let glyph_id = record
                .glyph_index(codepoint)
                .expect("a resolved face maps the codepoint");
            assert!(
                record_draws_codepoint(&record, codepoint),
                "{codepoint:?} resolved to a face without drawable glyph {glyph_id}"
            );
        }
    }

    // The defect the weight plumbing exists for: a bold line of Han came back
    // at the regular stroke, because the weight the text asked for never
    // reached the resolver — it only ever asked the platform about coverage.
    // Apple pairs every CJK UI face with a bolder sibling, so a bold request
    // must land on that one instead.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_bold_run_of_han_resolves_to_a_bolder_face() {
        let requirement = ScriptRequirement::from_run("你好", Some(TextLanguage::Chinese));
        let regular = fallback_for_codepoint('你', requirement, REGULAR_WEIGHT)
            .expect("han must resolve at the regular weight");
        let bold = fallback_for_codepoint('你', requirement, BOLD_WEIGHT)
            .expect("han must resolve at the bold weight");

        let regular_weight = regular.design_weight().unwrap_or(REGULAR_WEIGHT);
        let bold_weight = bold.design_weight().unwrap_or(REGULAR_WEIGHT);
        assert!(
            bold_weight > regular_weight,
            "a bold run kept the regular face ({regular_weight} -> {bold_weight}): {:?}",
            bold.path
        );
    }

    // Choosing by weight may not undo the rule it sits on top of: the bolder
    // face still has to draw the whole script standing beside the character,
    // or a bold word arrives split across two typefaces again.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn a_bolder_face_still_covers_the_script_it_was_chosen_for() {
        let requirement = ScriptRequirement::from_run("你好吗", None);
        for codepoint in "你好吗".chars() {
            let record = fallback_for_codepoint(codepoint, requirement, BOLD_WEIGHT)
                .expect("han must resolve at the bold weight");
            assert!(
                record_draws_all(&record, requirement.as_slice()),
                "{codepoint:?} resolved to a face covering only part of its run: {:?}",
                record.path
            );
        }
    }

    // One cached answer may not serve two weights, or the first run to ask
    // decides the stroke of every run after it.
    #[cfg(all(
        any(target_os = "ios", target_os = "macos"),
        feature = "apple-core-text"
    ))]
    #[test]
    fn answers_are_cached_per_requested_weight() {
        let requirement = ScriptRequirement::probes('你');
        let regular = fallback_for_codepoint('你', requirement, REGULAR_WEIGHT)
            .expect("han must resolve at the regular weight");
        let bold = fallback_for_codepoint('你', requirement, BOLD_WEIGHT)
            .expect("han must resolve at the bold weight");
        let again = fallback_for_codepoint('你', requirement, REGULAR_WEIGHT)
            .expect("a repeated request must resolve too");

        assert_eq!(
            regular.id, again.id,
            "a repeated request must hit its own cached answer"
        );
        assert_ne!(
            regular.id, bold.id,
            "one cache entry may not answer for two weights"
        );
    }

    #[test]
    fn unknown_ids_resolve_to_nothing() {
        assert!(fallback_by_id(0).is_none());
        assert!(fallback_by_id(SYSTEM_FALLBACK_ID_BASE - 1).is_none());
        assert!(fallback_by_id(FontId::MAX).is_none());
    }
}
