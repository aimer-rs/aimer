//! Dynamic system font discovery on Apple platforms.
//!
//! macOS and iOS ship their font catalogue behind Core Text rather than a
//! directory that can be enumerated cheaply. Walking every installed face —
//! which is what generic font databases do — costs hundreds of milliseconds at
//! startup and keeps large CJK and emoji faces resident in memory, so Cupid
//! instead asks Core Text four narrow questions:
//!
//! * [`system_font_path`] — "where does the family called *X* live?"
//! * [`fallback_font_path_for_probes`] — "which face would you use to draw
//!   these codepoints?"
//! * [`font_paths_for_codepoint`] — "which faces *could* draw this character?"
//! * [`language_fallback_sources`] — "which face would each UI language's
//!   cascade use to draw these codepoints?"
//!
//! Most answers are file system paths. The caller memory-maps those files,
//! which keeps the bytes out of the heap and lets the kernel evict them under
//! pressure. A face that no readable file backs is instead reassembled from
//! the tables Core Graphics exposes for it — `CTFontCopyGraphicsFont` followed
//! by `CGFontCopyTableTags`/`CGFontCopyTableForTag` — so Cupid can still
//! rasterize it without the platform's off-screen layer.
//!
//! Core Text's preferred answer is not always usable by the owned
//! rasterizer: on recent macOS builds the system UI cascade resolves CJK to
//! `PingFangUI.ttc`, a private file whose faces expose neither `glyf` outlines
//! nor bitmap strikes, so rendering it produces blank glyphs. This is why
//! [`font_paths_for_codepoint`] returns a *list* — the caller validates the
//! candidates in order and keeps the first one it can actually draw.
//!
//! The bindings come from [`objc2_core_text`] and [`objc2_core_foundation`].
//! Every Core Text object is returned as a [`CFRetained`] handle, so ownership
//! follows the Core Foundation *Create Rule* automatically and no manual
//! `CFRelease` is needed.
//!
//! This module is compiled only with the `apple-core-text` feature. The
//! platform bridge supplies discovery and table extraction; glyph lookup,
//! shaping, and rasterization remain owned by Cupid's Aimer font engine.
//!
//! [`CFRetained`]: objc2_core_foundation::CFRetained

use std::path::PathBuf;

use objc2_core_foundation::{
    CFCharacterSet, CFData, CFDictionary, CFLocale, CFNumber, CFRange, CFRetained, CFString, CFURL,
};
use objc2_core_graphics::CGFont;
use objc2_core_text::{
    CTFont, CTFontDescriptor, CTFontUIFontType, kCTFontCharacterSetAttribute,
    kCTFontTraitsAttribute, kCTFontURLAttribute, kCTFontWeightTrait,
};

use crate::font::TextLanguage;
use crate::text_pipeline::font_resolver::REGULAR_WEIGHT;

/// Point size used for the transient `CTFont` instances created here.
///
/// Font *selection* is size independent — only metrics scale with the point
/// size — so any positive value works. A concrete value still has to be passed
/// because `0.0` means "use the descriptor's size" to Core Text.
const LOOKUP_FONT_SIZE: f64 = 12.0;

/// The system UI font, used as the root of Core Text's cascade list.
///
/// Resolving a fallback always starts from a *current* font; starting from the
/// system UI font yields the same chain the platform itself would use.
const SYSTEM_UI_FONT_NAME: &str = ".AppleSystemUIFont";

/// Upper bound on the candidates reported for a single codepoint.
///
/// Widely covered characters — Latin punctuation, common Han ideographs — are
/// carried by dozens of installed faces. The first usable one wins, so the tail
/// of that list is never reached in practice and only costs memory.
const MAX_CANDIDATE_FACES: usize = 16;

/// UI languages whose cascades back up the device's own preferred languages.
///
/// A CJK codepoint must resolve even on a device configured for none of these,
/// so their cascades are consulted after the preferred languages. `ja` comes
/// first deliberately: the Japanese cascade pairs the UI font with faces this
/// crate decodes itself for every branch of unified Han (Hiragino Sans for the
/// shared ideographs, Hiragino Sans GB for the simplified-only ones), and a
/// kanji standing alone must stay on the face its kana neighbours use rather
/// than jump to a Korean or Chinese one.
const CJK_UI_LANGUAGES: [&str; 4] = ["ja", "zh-Hans", "zh-Hant", "ko"];

/// Most device-preferred languages consulted before [`CJK_UI_LANGUAGES`].
///
/// The preference list a user builds over years can be long; past the first
/// few entries it stops describing the text on screen and only multiplies
/// cascade queries.
const MAX_PREFERRED_LANGUAGES: usize = 4;

/// Anchors of Core Text's normalized weight scale, in `wght` order.
///
/// `kCTFontWeightTrait` runs from `-1.0` to `1.0` with `0.0` at regular, and
/// the spacing is not linear: Apple's own named constants crowd the bold half
/// of the scale together — `medium` at `0.23` sits nearly as high as `bold` at
/// `0.40`. These are those constants, paired with the OpenType weights they
/// correspond to, so a request lands on the face Apple itself would pick.
const WEIGHT_SCALE: [(u16, f64); 9] = [
    (100, -0.80),
    (200, -0.60),
    (300, -0.40),
    (400, 0.00),
    (500, 0.23),
    (600, 0.30),
    (700, 0.40),
    (800, 0.56),
    (900, 0.62),
];

/// Converts an OpenType `wght` value to Core Text's normalized weight trait.
///
/// Values between two anchors of [`WEIGHT_SCALE`] are interpolated, and values
/// outside it clamp to the ends: no face is designed beyond `100`–`900`, and
/// Core Text matches the nearest available weight anyway.
fn normalized_weight(weight: u16) -> f64 {
    let (lightest, heaviest) = (WEIGHT_SCALE[0], WEIGHT_SCALE[WEIGHT_SCALE.len() - 1]);
    let weight = weight.clamp(lightest.0, heaviest.0);
    let upper = WEIGHT_SCALE
        .iter()
        .position(|(anchor, _)| *anchor >= weight)
        .unwrap_or(WEIGHT_SCALE.len() - 1);
    let (upper_weight, upper_trait) = WEIGHT_SCALE[upper];
    if upper_weight == weight {
        return upper_trait;
    }
    let (lower_weight, lower_trait) = WEIGHT_SCALE[upper - 1];
    let progress = f64::from(weight - lower_weight) / f64::from(upper_weight - lower_weight);
    lower_trait + (upper_trait - lower_trait) * progress
}

/// Returns `font` restyled to `weight` on the OpenType `wght` scale.
///
/// Every fallback query here starts from the system UI font, and Core Text
/// answers a cascade query with the face it pairs with *that* font — stroke
/// weight included. Asked through the regular UI font, the cascade therefore
/// names `PingFang SC Regular` for a line that asked to be bold, and the text
/// arrives thin. Asked through the semibold one, it names
/// `PingFang SC Semibold`, which is the pairing Apple ships.
///
/// A request at [`REGULAR_WEIGHT`] returns `font` itself, so the path every
/// unemphasized line takes gains no work.
fn font_at_weight(font: CFRetained<CTFont>, weight: u16) -> CFRetained<CTFont> {
    if weight == REGULAR_WEIGHT {
        return font;
    }

    let value = CFNumber::new_f64(normalized_weight(weight));
    // SAFETY: `kCTFontWeightTrait` is a Core Text constant string, and the
    // trait it names takes the `CFNumber` paired with it here.
    let traits = CFDictionary::from_slices(&[unsafe { kCTFontWeightTrait }], &[&*value]);
    // SAFETY: `kCTFontTraitsAttribute` is a Core Text constant string, and the
    // attribute it names takes the traits dictionary paired with it here.
    let attributes =
        CFDictionary::from_slices(&[unsafe { kCTFontTraitsAttribute }], &[traits.as_opaque()]);
    // SAFETY: the dictionary holds documented attribute keys paired with the
    // value types those keys expect.
    let descriptor = unsafe { CTFontDescriptor::with_attributes(attributes.as_opaque()) };
    // SAFETY: a size of `0.0` keeps the original font's size and a null matrix
    // requests the identity transform.
    unsafe { font.copy_with_attributes(0.0, std::ptr::null(), Some(&descriptor)) }
}

/// Returns the file the given font was loaded from, if it is backed by one.
///
/// Fonts registered from memory (for example a `CGFont` built from a data
/// provider) have no `kCTFontURLAttribute`, and fonts served from the shared
/// on-demand font asset store may resolve to a non-`file://` URL. Both cases
/// return `None` rather than a path that cannot be opened.
fn font_file_path(font: &CTFont) -> Option<PathBuf> {
    // SAFETY: `kCTFontURLAttribute` is a Core Text constant string and `font`
    // is a valid, retained font reference.
    let attribute = unsafe { font.attribute(kCTFontURLAttribute) }?;
    let url = attribute.downcast::<CFURL>().ok()?;
    url.to_file_path()
}

/// Resolves a font family name to the file backing its default face.
///
/// The lookup goes through `CTFontCreateWithName`, which performs the same
/// best-match resolution as the rest of the system: an exact family match wins,
/// otherwise Core Text substitutes the closest available face. Because a match
/// is always produced, the returned path is only `None` when the resolved face
/// has no file backing it.
///
/// # Examples
///
/// ```ignore
/// let path = system_font_path("Helvetica").expect("Helvetica is always installed");
/// assert!(path.exists());
/// ```
pub(crate) fn system_font_path(family: &str) -> Option<PathBuf> {
    if family.is_empty() {
        return None;
    }
    let name = CFString::from_str(family);
    // SAFETY: a null matrix requests the identity transform, which is what the
    // documentation prescribes for "no transform".
    let font = unsafe { CTFont::with_name(&name, LOOKUP_FONT_SIZE, std::ptr::null()) };
    font_file_path(&font)
}

/// Resolves the file of the face Core Text would fall back to for `probes`.
///
/// The probes are joined into a single string and handed to
/// `CTFontCreateForString` alongside the system UI font. Core Text walks its
/// cascade list and returns the first face able to draw the string — the same
/// decision it makes when rendering text the current font cannot cover.
///
/// The UI font is restyled to `weight` first, so the cascade names the sibling
/// face Apple pairs with text of that stroke — see [`font_at_weight`].
///
/// Returns `None` when `probes` is empty or when the chosen face is not backed
/// by a file. Note that Core Text never fails outright: if nothing covers the
/// probes it hands back the current font, so callers must still verify that the
/// returned face actually contains glyphs for the codepoints they care about.
pub(crate) fn fallback_font_path_for_probes(probes: &[char], weight: u16) -> Option<PathBuf> {
    let sample: String = probes.iter().collect();
    // `CFRange` counts UTF-16 code units, not `char`s, so astral-plane probes
    // such as emoji contribute two units each.
    let length = sample.encode_utf16().count() as isize;
    if length == 0 {
        return None;
    }

    let base_name = CFString::from_str(SYSTEM_UI_FONT_NAME);
    let sample = CFString::from_str(&sample);
    // SAFETY: a null matrix requests the identity transform.
    let base_font = unsafe { CTFont::with_name(&base_name, LOOKUP_FONT_SIZE, std::ptr::null()) };
    let base_font = font_at_weight(base_font, weight);
    // SAFETY: the range spans exactly the UTF-16 content of `sample`.
    let fallback = unsafe {
        base_font.for_string(
            &sample,
            CFRange {
                location: 0,
                length,
            },
        )
    };

    font_file_path(&fallback)
}

/// Returns the files of every face that can draw `codepoint`, best first.
///
/// This is the query that drives on-demand fallback: instead of guessing which
/// faces might be needed from a table of script samples, the system is asked
/// about the one character that could not be drawn.
///
/// The order encodes decreasing confidence:
///
/// 1. the face `CTFontCreateForString` picks — what the platform itself would
///    render the character with;
/// 2. every face matching a descriptor built from a character set holding just
///    `codepoint`, which is the full set of installed faces covering it.
///
/// Only the cascade answer is weight aware: the catalogue match is the
/// exhaustive backstop, and narrowing it by weight would drop the only face
/// covering a codepoint whenever no sibling exists at the requested stroke.
///
/// Paths are deduplicated and the list is capped at [`MAX_CANDIDATE_FACES`],
/// because the caller pays a memory map plus a font parse per candidate it has
/// to reject.
pub(crate) fn font_paths_for_codepoint(codepoint: char, weight: u16) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut push = |path: PathBuf| {
        if paths.len() < MAX_CANDIDATE_FACES && !paths.contains(&path) {
            paths.push(path);
        }
    };

    if let Some(preferred) = fallback_font_path_for_probes(&[codepoint], weight) {
        push(preferred);
    }
    for path in matching_font_paths_for_codepoint(codepoint) {
        push(path);
    }

    paths
}

/// Returns every installed face whose character set contains `codepoint`.
///
/// Unlike `CTFontCreateForString` this ignores the cascade list and asks the
/// font catalogue directly, so it also surfaces faces the system would never
/// pick on its own. The order Core Text returns is preserved.
fn matching_font_paths_for_codepoint(codepoint: char) -> Vec<PathBuf> {
    let sample = CFString::from_str(&codepoint.to_string());
    // SAFETY: both arguments are valid; `None` requests the default allocator.
    let Some(character_set) =
        (unsafe { CFCharacterSet::with_characters_in_string(None, Some(&sample)) })
    else {
        return Vec::new();
    };

    // SAFETY: `kCTFontCharacterSetAttribute` is a Core Text constant string.
    let attribute_key = unsafe { kCTFontCharacterSetAttribute };
    let attributes = CFDictionary::from_slices(&[attribute_key], &[&*character_set]);
    // SAFETY: the dictionary holds a documented attribute key paired with the
    // `CFCharacterSet` value that key expects.
    let descriptor = unsafe { CTFontDescriptor::with_attributes(attributes.as_opaque()) };
    // SAFETY: no mandatory attributes are requested, so no generic type has to
    // line up.
    let Some(matches) = (unsafe { descriptor.matching_font_descriptors(None) }) else {
        return Vec::new();
    };

    // SAFETY: `CTFontDescriptorCreateMatchingFontDescriptors` is documented to
    // return an array of font descriptors.
    let matches = unsafe { matches.cast_unchecked::<CTFontDescriptor>() };

    matches
        .iter()
        .filter_map(|candidate| {
            // SAFETY: `kCTFontURLAttribute` is a Core Text constant string.
            let url = unsafe { candidate.attribute(kCTFontURLAttribute) }?;
            url.downcast::<CFURL>().ok()?.to_file_path()
        })
        .collect()
}

/// A fallback face offered by the platform, with the bytes to draw it from.
///
/// Most faces live in font files the caller can memory-map. A face without a
/// usable file — one registered from memory, or served from an asset store
/// behind a non-`file://` URL — is carried as the sfnt reassembled from its
/// Core Graphics tables instead, identified by its PostScript name.
pub(crate) enum SystemFaceSource {
    /// A face backed by an on-disk font file.
    File(PathBuf),
    /// A face reassembled from the tables Core Graphics exposes for it.
    Data {
        postscript_name: String,
        bytes: Vec<u8>,
    },
}

/// Returns the faces each UI language's cascade uses to draw `required`.
///
/// `CTFontCreateForString` alone answers with the face the *device's language*
/// prefers, which for CJK is a private file this crate cannot decode. Asked
/// through `CTFontCreateUIFontForLanguage` instead, every language names the
/// concrete face Apple pairs with its UI font — stroke weight included — and
/// several of those are faces Cupid rasterizes itself: on a Japanese cascade,
/// simplified-only Han resolves to Hiragino Sans GB, whose weight matches the
/// Hiragino face drawing the kana beside it.
///
/// Each language is asked about `required` as a whole and about every
/// character alone: the whole-string answer covers only the longest prefix the
/// first matching face can draw, so the face serving the characters *behind*
/// that prefix — the simplified-only tail of a Japanese line — appears only in
/// the per-character answers.
///
/// Each language's UI font is restyled to `weight` before its cascade is
/// walked, which is what makes a bold line of Han arrive on `W6`/`Semibold`
/// rather than on the regular cut — see [`font_at_weight`].
///
/// The device's preferred languages are consulted first so the answer blends
/// with the rest of the interface, then [`CJK_UI_LANGUAGES`] as a backstop.
/// Sources are deduplicated and capped at [`MAX_CANDIDATE_FACES`]; callers
/// must still validate coverage, because Core Text hands back the queried font
/// itself when nothing matches.
pub(crate) fn language_fallback_sources(
    required: &[char],
    language: Option<TextLanguage>,
    weight: u16,
) -> Vec<SystemFaceSource> {
    let mut sources: Vec<SystemFaceSource> = Vec::new();
    let full: String = required.iter().collect();
    if full.is_empty() {
        return sources;
    }

    let mut samples: Vec<String> = vec![full];
    if required.len() > 1 {
        samples.extend(required.iter().map(|codepoint| codepoint.to_string()));
    }

    for language in ui_cascade_languages(language) {
        let language = CFString::from_str(&language);
        // SAFETY: the constant designates the system UI font and the language
        // is a valid BCP-47 tag string.
        let Some(base) = (unsafe {
            CTFont::new_ui_font_for_language(
                CTFontUIFontType::System,
                LOOKUP_FONT_SIZE,
                Some(&language),
            )
        }) else {
            continue;
        };
        let base = font_at_weight(base, weight);
        for sample in &samples {
            if sources.len() >= MAX_CANDIDATE_FACES {
                return sources;
            }
            let string = CFString::from_str(sample);
            // `CFRange` counts UTF-16 code units, so astral-plane characters
            // contribute two units each.
            let length = sample.encode_utf16().count() as isize;
            // SAFETY: the range spans exactly the UTF-16 content of `string`.
            let resolved = unsafe {
                base.for_string_with_language(
                    &string,
                    CFRange {
                        location: 0,
                        length,
                    },
                    Some(&language),
                )
            };
            push_source(&mut sources, &resolved);
        }
    }

    sources
}

/// The languages whose UI cascades [`language_fallback_sources`] consults.
fn ui_cascade_languages(language: Option<TextLanguage>) -> Vec<String> {
    let mut languages: Vec<String> = Vec::new();
    if let Some(language) = language {
        languages.push(cjk_language_tag(language).to_string());
    }
    if let Some(preferred) = CFLocale::preferred_languages() {
        // SAFETY: `CFLocaleCopyPreferredLanguages` is documented to return an
        // array of `CFString` language identifiers.
        let preferred = unsafe { preferred.cast_unchecked::<CFString>() };
        languages.extend(
            preferred
                .iter()
                .take(MAX_PREFERRED_LANGUAGES)
                .map(|language| language.to_string()),
        );
    }
    for language in CJK_UI_LANGUAGES {
        if !languages.iter().any(|existing| existing == language) {
            languages.push(language.to_string());
        }
    }
    languages
}

/// Returns the most specific UI-cascade tag available for a supported CJK
/// language hint.
fn cjk_language_tag(language: TextLanguage) -> &'static str {
    match language {
        TextLanguage::Chinese => "zh-Hans",
        TextLanguage::Japanese => "ja",
        TextLanguage::Korean => "ko",
    }
}

/// Appends `font` to `sources` unless an equivalent entry is already present.
///
/// File-backed faces are deduplicated by path, memory-backed ones by
/// PostScript name. Table extraction copies every table of the font, so it is
/// attempted only for fonts no readable file backs and only after the name
/// check has ruled out a duplicate.
fn push_source(sources: &mut Vec<SystemFaceSource>, font: &CTFont) {
    if let Some(path) = font_file_path(font) {
        let seen = sources
            .iter()
            .any(|source| matches!(source, SystemFaceSource::File(existing) if *existing == path));
        if !seen {
            sources.push(SystemFaceSource::File(path));
        }
        return;
    }

    // SAFETY: `font` is a valid, retained font reference.
    let postscript_name = unsafe { font.post_script_name() }.to_string();
    let seen = sources.iter().any(|source| {
        matches!(source, SystemFaceSource::Data { postscript_name: existing, .. } if *existing == postscript_name)
    });
    if seen {
        return;
    }
    if let Some(bytes) = font_table_data(font) {
        sources.push(SystemFaceSource::Data {
            postscript_name,
            bytes,
        });
    }
}

/// Reassembles an sfnt byte stream from the font's Core Graphics tables.
///
/// `CTFontCopyGraphicsFont` bridges the font to a `CGFont`, whose
/// `CGFontCopyTableTags`/`CGFontCopyTableForTag` expose every table of the
/// underlying face — including the single face of a `.ttc` collection member,
/// which is why the result always parses at collection index `0`.
///
/// Returns `None` when the font exposes no tables at all.
fn font_table_data(font: &CTFont) -> Option<Vec<u8>> {
    // SAFETY: a null attribute-descriptor out-parameter is documented as "the
    // caller does not want the descriptor back".
    let graphics = unsafe { font.graphics_font(std::ptr::null_mut()) };
    let tags = CGFont::table_tags(Some(&graphics))?;
    let count = tags.count();
    let mut tables: Vec<(u32, CFRetained<CFData>)> = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: `index` is in bounds; the array carries table tags stored as
        // pointer-sized integers, not object references.
        let tag = unsafe { tags.value_at_index(index) } as usize as u32;
        if let Some(data) = CGFont::table_for_tag(Some(&graphics), tag) {
            tables.push((tag, data));
        }
    }
    sfnt_from_tables(&tables)
}

/// Serializes `tables` into a standalone sfnt font file image.
///
/// The directory is emitted in ascending tag order with spec-conforming
/// binary-search fields and per-table checksums. `head.checkSumAdjustment` is
/// left as extracted — no consumer of the result verifies whole-file sums.
fn sfnt_from_tables(tables: &[(u32, CFRetained<CFData>)]) -> Option<Vec<u8>> {
    const HEADER_LEN: usize = 12;
    const ENTRY_LEN: usize = 16;

    let count = u16::try_from(tables.len()).ok()?;
    if count == 0 {
        return None;
    }

    // CFF-flavoured fonts require the `OTTO` signature; everything else uses
    // the TrueType version tag.
    let cff_tag = u32::from_be_bytes(*b"CFF ");
    let version: u32 = if tables.iter().any(|(tag, _)| *tag == cff_tag) {
        0x4F54_544F
    } else {
        0x0001_0000
    };

    let mut entry_selector: u16 = 0;
    while (2usize << entry_selector) <= tables.len() {
        entry_selector += 1;
    }
    let search_range = (1u16 << entry_selector) * ENTRY_LEN as u16;
    let range_shift = count * ENTRY_LEN as u16 - search_range;

    let mut order: Vec<usize> = (0..tables.len()).collect();
    order.sort_by_key(|&index| tables[index].0);

    let mut offset = HEADER_LEN + ENTRY_LEN * tables.len();
    let mut directory = Vec::with_capacity(ENTRY_LEN * tables.len());
    let mut body: Vec<u8> = Vec::new();
    for &index in &order {
        let (tag, data) = &tables[index];
        let length = data.length() as usize;
        // SAFETY: the pointer covers `length` bytes owned by the retained
        // `CFData`, which outlives this copy.
        let bytes = unsafe { std::slice::from_raw_parts(data.byte_ptr(), length) };
        directory.extend_from_slice(&tag.to_be_bytes());
        directory.extend_from_slice(&table_checksum(bytes).to_be_bytes());
        directory.extend_from_slice(&u32::try_from(offset).ok()?.to_be_bytes());
        directory.extend_from_slice(&u32::try_from(length).ok()?.to_be_bytes());
        body.extend_from_slice(bytes);
        let padding = length.wrapping_neg() & 3;
        body.extend_from_slice(&[0u8; 3][..padding]);
        offset += length + padding;
    }

    let mut font = Vec::with_capacity(HEADER_LEN + directory.len() + body.len());
    font.extend_from_slice(&version.to_be_bytes());
    font.extend_from_slice(&count.to_be_bytes());
    font.extend_from_slice(&search_range.to_be_bytes());
    font.extend_from_slice(&entry_selector.to_be_bytes());
    font.extend_from_slice(&range_shift.to_be_bytes());
    font.extend_from_slice(&directory);
    font.extend_from_slice(&body);
    Some(font)
}

/// The sfnt checksum of a table: the big-endian `u32` sum over its bytes,
/// zero-padded to a word boundary.
fn table_checksum(bytes: &[u8]) -> u32 {
    let mut sum = 0u32;
    let mut words = bytes.chunks_exact(4);
    for word in &mut words {
        sum = sum.wrapping_add(u32::from_be_bytes([word[0], word[1], word[2], word[3]]));
    }
    let remainder = words.remainder();
    if remainder.is_empty() {
        return sum;
    }
    let mut last = [0u8; 4];
    last[..remainder.len()].copy_from_slice(remainder);
    sum.wrapping_add(u32::from_be_bytes(last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_paths_for_codepoint_lists_the_preferred_face_first() {
        let preferred = fallback_font_path_for_probes(&['你'], REGULAR_WEIGHT)
            .expect("the system always proposes a face");
        let paths = font_paths_for_codepoint('你', REGULAR_WEIGHT);
        assert_eq!(paths.first(), Some(&preferred));
    }

    #[test]
    fn font_paths_for_codepoint_offers_alternatives_to_the_preferred_face() {
        // Han ideographs are carried by many installed faces, so the platform's
        // first pick must not be the only option — rejecting it has to leave
        // something to fall back on.
        let paths = font_paths_for_codepoint('你', REGULAR_WEIGHT);
        assert!(
            paths.len() > 1,
            "expected alternatives beyond the preferred face, got {paths:?}"
        );
        assert!(paths.iter().all(|path| path.exists()));
    }

    #[test]
    fn font_paths_for_codepoint_deduplicates_and_stays_bounded() {
        let paths = font_paths_for_codepoint('！', REGULAR_WEIGHT);
        assert!(paths.len() <= MAX_CANDIDATE_FACES);
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len(), "duplicate candidates: {paths:?}");
    }

    #[test]
    fn font_paths_for_codepoint_finds_a_color_emoji_face() {
        let paths = font_paths_for_codepoint('\u{1F600}', REGULAR_WEIGHT);
        let first = paths.first().expect("emoji must resolve to a font file");
        let name = first
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(
            name.contains("emoji"),
            "expected an emoji face, got {first:?}"
        );
    }

    // The weight a style asks for is an OpenType `wght` value, while Core Text
    // matches on a normalized trait whose two halves are scaled differently.
    // A request that grew heavier must never translate to a lighter trait, or
    // bold text resolves to a thinner face than regular text does.
    #[test]
    fn the_normalized_weight_scale_rises_with_the_opentype_weight() {
        assert_eq!(normalized_weight(REGULAR_WEIGHT), 0.0);

        let mut previous = f64::NEG_INFINITY;
        for weight in (100..=900).step_by(50) {
            let normalized = normalized_weight(weight);
            assert!(
                normalized > previous,
                "wght {weight} did not rise above the weight below it"
            );
            assert!(
                (-1.0..=1.0).contains(&normalized),
                "wght {weight} left Core Text's normalized range: {normalized}"
            );
            previous = normalized;
        }
    }

    // Weights outside the designed range are clamped rather than extrapolated:
    // a trait beyond ±1.0 is not a value Core Text matches against.
    #[test]
    fn the_normalized_weight_scale_clamps_outside_the_designed_range() {
        assert_eq!(normalized_weight(0), normalized_weight(100));
        assert_eq!(normalized_weight(u16::MAX), normalized_weight(900));
    }

    // The defect this exists for: a bold line of Han came back on the regular
    // face because every cascade query started from the regular UI font.
    #[test]
    fn a_bold_cascade_proposes_a_face_the_regular_one_does_not() {
        let required = ['你', '好'];
        let regular = language_fallback_sources(&required, None, REGULAR_WEIGHT);
        let bold = language_fallback_sources(&required, None, 700);
        assert!(!bold.is_empty(), "a bold cascade must still name faces");

        let names = |sources: &[SystemFaceSource]| -> Vec<String> {
            sources
                .iter()
                .map(|source| match source {
                    SystemFaceSource::File(path) => path.display().to_string(),
                    SystemFaceSource::Data {
                        postscript_name, ..
                    } => postscript_name.clone(),
                })
                .collect()
        };
        let regular = names(&regular);
        let bold = names(&bold);
        assert!(
            bold.iter().any(|face| !regular.contains(face)),
            "a bold request named only the regular faces: {bold:?}"
        );
    }

    #[test]
    fn system_font_path_resolves_an_existing_file() {
        let path =
            system_font_path("Helvetica").expect("Helvetica must resolve on Apple platforms");
        assert!(
            path.is_absolute(),
            "expected an absolute path, got {path:?}"
        );
        assert!(path.exists(), "resolved font file does not exist: {path:?}");
    }

    #[test]
    fn system_font_path_rejects_an_empty_family() {
        assert!(system_font_path("").is_none());
    }

    #[test]
    fn system_font_path_falls_back_for_an_unknown_family() {
        // Core Text substitutes a best match instead of failing, so an unknown
        // family still yields a usable file rather than a dangling path.
        let path = system_font_path("aimer-does-not-exist-font-family")
            .expect("Core Text substitutes a default face");
        assert!(
            path.exists(),
            "substituted font file does not exist: {path:?}"
        );
    }

    #[test]
    fn fallback_font_path_for_probes_finds_a_color_emoji_face() {
        let path = fallback_font_path_for_probes(&['\u{1F600}', '\u{1F601}'], REGULAR_WEIGHT)
            .expect("emoji probes must resolve to a font file");
        assert!(path.exists(), "resolved font file does not exist: {path:?}");
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(
            name.contains("emoji"),
            "expected an emoji face for emoji probes, got {path:?}"
        );
    }

    #[test]
    fn fallback_font_path_for_probes_handles_a_cjk_script() {
        let path = fallback_font_path_for_probes(&['漢', '字'], REGULAR_WEIGHT)
            .expect("CJK probes must resolve to a font file");
        assert!(path.exists(), "resolved font file does not exist: {path:?}");
    }

    #[test]
    fn fallback_font_path_for_probes_rejects_empty_probes() {
        assert!(fallback_font_path_for_probes(&[], REGULAR_WEIGHT).is_none());
    }

    /// Reports whether any face of `data` has readable glyph data for all of
    /// `required` — the acceptance rule the fallback resolver applies.
    fn any_face_decodes_all(data: &[u8], required: &[char]) -> bool {
        (0..64).any(|index| {
            let Ok(face) = crate::text_pipeline::aimer_font::SfntFace::from_bytes(data, index)
            else {
                return false;
            };
            required.iter().all(|codepoint| {
                let Ok(Some(glyph_id)) = face.glyph_index(*codepoint as u32) else {
                    return false;
                };
                glyph_id != 0
                    && (face.outline(glyph_id).ok().flatten().is_some()
                        || face.cff_outline(glyph_id).ok().flatten().is_some())
            })
        })
    }

    // The property the mixed-weight fix rests on: some UI language's cascade
    // must offer a simplified-Han face whose outlines this crate reads itself,
    // so Han never has to fall onto the platform's off-screen rasterizer.
    #[test]
    fn language_fallback_sources_offer_a_decodable_simplified_han_face() {
        let required = ['吗', '顶', '这'];
        let sources = language_fallback_sources(&required, None, REGULAR_WEIGHT);
        assert!(!sources.is_empty(), "no cascade proposed any face");
        assert!(
            sources.iter().any(|source| match source {
                SystemFaceSource::File(path) =>
                    std::fs::read(path).is_ok_and(|data| any_face_decodes_all(&data, &required)),
                SystemFaceSource::Data { bytes, .. } => any_face_decodes_all(bytes, &required),
            }),
            "every language cascade answered with faces cupid cannot decode"
        );
    }

    #[test]
    fn language_fallback_sources_reject_an_empty_requirement() {
        assert!(language_fallback_sources(&[], None, REGULAR_WEIGHT).is_empty());
    }

    #[test]
    fn an_explicit_cjk_language_heads_its_ui_cascade() {
        use crate::font::TextLanguage;

        assert_eq!(
            ui_cascade_languages(Some(TextLanguage::Chinese))
                .first()
                .map(String::as_str),
            Some("zh-Hans")
        );
        assert_eq!(
            ui_cascade_languages(Some(TextLanguage::Japanese))
                .first()
                .map(String::as_str),
            Some("ja")
        );
        assert_eq!(
            ui_cascade_languages(Some(TextLanguage::Korean))
                .first()
                .map(String::as_str),
            Some("ko")
        );
    }

    // The extraction path for faces without a readable file behind them:
    // tables copied out of Core Graphics must reassemble into an sfnt this
    // crate parses and decodes glyphs from.
    #[test]
    fn font_table_data_reassembles_a_decodable_font() {
        let name = CFString::from_str("Helvetica");
        // SAFETY: a null matrix requests the identity transform.
        let font = unsafe { CTFont::with_name(&name, LOOKUP_FONT_SIZE, std::ptr::null()) };
        let data = font_table_data(&font).expect("system fonts expose their tables");
        let face = crate::text_pipeline::aimer_font::SfntFace::from_bytes(&data, 0)
            .expect("the reassembled sfnt must parse");
        let glyph_id = face
            .glyph_index('A' as u32)
            .expect("helvetica cmap must parse")
            .expect("helvetica maps basic latin");
        assert!(
            face.outline(glyph_id).ok().flatten().is_some()
                || face.cff_outline(glyph_id).ok().flatten().is_some(),
            "the reassembled glyph data must decode"
        );
    }

    #[test]
    fn table_checksum_pads_the_trailing_word_with_zeroes() {
        assert_eq!(table_checksum(&[]), 0);
        assert_eq!(table_checksum(&[0x01]), 0x0100_0000);
        assert_eq!(table_checksum(&[0x00, 0x00, 0x00, 0x01, 0x02]), 0x0200_0001);
    }
}
