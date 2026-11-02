//! Checked shaping for the first Southeast Asian script slice.
//!
//! Thai, Lao, Khmer, and Myanmar all use combining marks and font-specific
//! GSUB/GPOS data, but they do not share one universal syllable algorithm.
//! This module therefore keeps the Unicode work deliberately bounded: it
//! recognizes one script per run, applies Khmer/Myanmar syllable ordering and
//! the Thai/Lao pre-base reorder, and delegates font-specific forms and mark
//! anchors to the checked OpenType lookup readers. A lookup kind outside this
//! slice returns `Ok(None)` so the checked scalar layout retains ownership of
//! cases that need a complete script implementation. Unsupported contextual
//! GPOS lookups are skipped while preserving supported GSUB substitutions.

use super::super::{FontMetrics, SfntError, SfntFace};
use super::indic::{
    apply_alternate_lookup, apply_ligature_lookup, apply_multiple_lookup, apply_single_lookup,
};
use super::{
    ABVM_TAG, AimerShapedRun, BLWM_TAG, DIST_TAG, GPOS_TAG, Gdef, KERN_TAG,
    KHMER_GSUB_FEATURE_TAGS, KHMR_TAG, LAOO_TAG, LayoutGlyph, LayoutState, LayoutTableState,
    MARK_TAG, MAX_EXTENSION_DEPTH, MKMK_TAG, MYANMAR_GSUB_FEATURE_TAGS, MYM2_TAG, MYMR_TAG,
    SOUTHEAST_ASIAN_GPOS_FEATURE_TAGS, SOUTHEAST_ASIAN_GSUB_FEATURE_TAGS, THAI_TAG, Tag,
    apply_gpos, checked_add, checked_mul, context, coverage_index,
    mark_to_base_adjustment_for_lookup, mark_to_mark_adjustment_for_lookup, read_u16, read_u32,
    slice_from,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SoutheastAsianScript {
    Thai,
    Lao,
    Khmer,
    Myanmar,
}

impl SoutheastAsianScript {
    #[inline]
    fn tag(self) -> Tag {
        match self {
            Self::Thai => THAI_TAG,
            Self::Lao => LAOO_TAG,
            Self::Khmer => KHMR_TAG,
            Self::Myanmar => MYM2_TAG,
        }
    }
}

fn script_tag_with_features(
    layout: &LayoutTableState,
    script: SoutheastAsianScript,
    feature_tags: &[Tag],
) -> Tag {
    if matches!(script, SoutheastAsianScript::Myanmar)
        && !feature_tags.iter().any(|feature_tag| {
            !layout
                .feature_lookups_with_language(MYM2_TAG, None, std::slice::from_ref(feature_tag))
                .is_empty()
        })
    {
        // Older fonts may only expose the limited legacy `mymr` script tag.
        return MYMR_TAG;
    }
    script.tag()
}

/// Returns the one Southeast Asian script represented by `text`.
///
/// ASCII spacing, punctuation, digits, zero-width spacing, and join controls
/// are accepted as neutral context. A run containing two Southeast Asian
/// scripts, a second non-neutral script, or only combining marks stays on the
/// compatibility path.
pub(super) fn script_for_text(text: &str) -> Option<SoutheastAsianScript> {
    let mut script = None;
    let mut has_base = false;
    for codepoint in text.chars() {
        let Some(current) = script_for_codepoint(codepoint) else {
            if is_neutral_context(codepoint) {
                continue;
            }
            return None;
        };
        if script.is_some_and(|previous| previous != current) {
            return None;
        }
        script = Some(current);
        has_base |= !is_sea_mark(current, codepoint) && !is_join_control(codepoint);
    }
    if !has_base {
        return None;
    }
    script
}

#[inline]
fn script_for_codepoint(codepoint: char) -> Option<SoutheastAsianScript> {
    match codepoint as u32 {
        0x0e00..=0x0e7f => Some(SoutheastAsianScript::Thai),
        0x0e80..=0x0eff => Some(SoutheastAsianScript::Lao),
        0x1780..=0x17ff | 0x19e0..=0x19ff => Some(SoutheastAsianScript::Khmer),
        0x1000..=0x109f | 0xa9e0..=0xa9ff | 0xaa60..=0xaa7f => Some(SoutheastAsianScript::Myanmar),
        _ => None,
    }
}

#[inline]
fn is_script_codepoint(script: SoutheastAsianScript, codepoint: char) -> bool {
    script_for_codepoint(codepoint) == Some(script)
}

#[inline]
fn is_neutral_context(codepoint: char) -> bool {
    codepoint.is_ascii_whitespace()
        || codepoint.is_ascii_punctuation()
        || codepoint.is_ascii_digit()
        || codepoint == '\u{200b}'
        || is_join_control(codepoint)
}

#[inline]
fn is_join_control(codepoint: char) -> bool {
    matches!(codepoint, '\u{200c}' | '\u{200d}')
}

#[inline]
fn is_sea_mark(script: SoutheastAsianScript, codepoint: char) -> bool {
    match script {
        SoutheastAsianScript::Thai => matches!(
            codepoint as u32,
            0x0e31 | 0x0e34..=0x0e3a | 0x0e3f | 0x0e47..=0x0e4e
        ),
        SoutheastAsianScript::Lao => matches!(
            codepoint as u32,
            0x0eb1 | 0x0eb4..=0x0ebc | 0x0ec8..=0x0ecd
        ),
        SoutheastAsianScript::Khmer => matches!(codepoint as u32, 0x17b4..=0x17d3 | 0x17dd),
        SoutheastAsianScript::Myanmar => matches!(
            codepoint as u32,
            0x102b..=0x103e
                | 0x1056..=0x1059
                | 0x105e..=0x1060
                | 0x1062..=0x1064
                | 0x1067..=0x106d
                | 0x1082..=0x108d
                | 0x109d
        ),
    }
}

#[inline]
fn is_prebase_mark(script: SoutheastAsianScript, codepoint: char) -> bool {
    matches!(
        (script, codepoint as u32),
        (SoutheastAsianScript::Thai, 0x0e40..=0x0e44)
            | (SoutheastAsianScript::Lao, 0x0ec0..=0x0ec4)
            | (SoutheastAsianScript::Myanmar, 0x1031)
    )
}

#[inline]
fn is_base(script: SoutheastAsianScript, codepoint: char) -> bool {
    is_script_codepoint(script, codepoint)
        && !is_sea_mark(script, codepoint)
        && !is_join_control(codepoint)
}

pub(super) fn shape_run_with_layout(
    face: &SfntFace<'_>,
    layout: &LayoutState,
    text: &str,
    scratch: &mut super::LayoutScratch,
) -> Result<Option<AimerShapedRun>, SfntError> {
    let Some(script) = script_for_text(text) else {
        return Ok(None);
    };
    let metrics = face.metrics()?;
    let advances = face.glyph_advances_with_metrics(metrics)?;
    scratch.codepoints.clear();
    scratch.glyphs.clear();
    for (cluster, codepoint) in text.char_indices() {
        if codepoint == '\u{200b}' {
            // HarfBuzz preserves a default-ignorable record for ZWSP, but
            // replaces its cmap glyph with the font's invisible space glyph
            // and clears the advance. Keep the source cluster so interaction
            // geometry remains stable while matching that zero-width output.
            let Some(glyph_id) = face.glyph_index(' ' as u32)? else {
                return Ok(None);
            };
            scratch.codepoints.push(codepoint);
            scratch
                .glyphs
                .push(LayoutGlyph::from_glyph_id(glyph_id, cluster, 0));
            continue;
        }
        // U+17C4 KHMER VOWEL SIGN OO is decomposed by the Khmer shaper into
        // E + AA before the font's GSUB lookups run. A one-codepoint cmap
        // lookup cannot reproduce that shaping input, and faces such as
        // Google Sans consequently lose the `លោក` sign unless the same
        // bounded decomposition is made explicit.
        let decomposition: &[char] = if codepoint == '\u{17c4}' {
            &KHMER_OO_DECOMPOSITION
        } else {
            std::slice::from_ref(&codepoint)
        };
        for &shaping_codepoint in decomposition {
            let Some(glyph_id) = face.glyph_index(shaping_codepoint as u32)? else {
                return Ok(None);
            };
            let Some(advance) = advances.get(usize::from(glyph_id)).copied() else {
                return Ok(None);
            };
            scratch.codepoints.push(shaping_codepoint);
            scratch
                .glyphs
                .push(LayoutGlyph::from_glyph_id(glyph_id, cluster, advance));
        }
    }

    scratch.items.clear();
    scratch.items.extend(
        scratch
            .codepoints
            .iter()
            .copied()
            .zip(scratch.glyphs.drain(..)),
    );
    scratch.source_codepoints.clear();
    scratch.source_codepoints.extend(
        scratch
            .items
            .iter()
            .map(|(codepoint, glyph)| (glyph.cluster, *codepoint)),
    );
    // The cmap pass is in source order, so cluster keys are already sorted.
    // Keep this map before visual reordering; it also preserves the first
    // source codepoint for the Khmer composite glyph's duplicate cluster.
    let reordered =
        reorder_prebase_marks(&mut scratch.items, script, &mut scratch.myanmar_replacement);
    scratch
        .glyphs
        .extend(scratch.items.drain(..).map(|(_, glyph)| glyph));

    let Some((gsub_supported, gsub_changed)) = apply_southeast_asian_gsub(
        face,
        metrics,
        &mut scratch.glyphs,
        &mut scratch.context_candidates,
        &layout.gdef,
        layout,
        script,
    )?
    else {
        return Ok(None);
    };
    let source_codepoints = &scratch.source_codepoints;
    let shaped_glyphs = &scratch.glyphs;
    scratch.codepoints.clear();
    scratch.codepoints.extend(
        shaped_glyphs
            .iter()
            .map(|glyph| super::source_codepoint_for_cluster(source_codepoints, glyph.cluster)),
    );
    let Some((gpos_supported, gpos_changed)) = apply_southeast_asian_gpos(
        face,
        &mut scratch.glyphs,
        &scratch.codepoints,
        &layout.gdef,
        layout,
        script,
    )?
    else {
        return Ok(None);
    };
    normalize_sea_clusters(
        &mut scratch.glyphs,
        &scratch.codepoints,
        &layout.gdef,
        script,
    )?;

    if !gsub_supported && !gpos_supported {
        return Ok(None);
    }
    if !reordered && !gsub_changed && !gpos_changed {
        return Ok(None);
    }

    Ok(Some(AimerShapedRun {
        units_per_em: metrics.units_per_em,
        glyphs: std::mem::take(&mut scratch.glyphs),
    }))
}

const KHMER_OO_DECOMPOSITION: [char; 2] = ['\u{17c1}', '\u{17b6}'];

fn reorder_prebase_marks(
    items: &mut Vec<(char, LayoutGlyph)>,
    script: SoutheastAsianScript,
    myanmar_replacement: &mut Vec<(char, LayoutGlyph)>,
) -> bool {
    if matches!(script, SoutheastAsianScript::Khmer) {
        return reorder_khmer_syllables(items);
    }
    if matches!(script, SoutheastAsianScript::Myanmar) {
        return reorder_myanmar_syllables(items, myanmar_replacement);
    }
    let mut changed = false;
    let mut index = 0;
    while index < items.len() {
        if !is_script_codepoint(script, items[index].0) {
            index += 1;
            continue;
        }
        let segment_end = items[index..]
            .iter()
            .position(|(codepoint, _)| !is_script_codepoint(script, *codepoint))
            .map_or(items.len(), |offset| index + offset);
        let mut base_index = index;
        while base_index < segment_end {
            if !is_base(script, items[base_index].0) {
                base_index += 1;
                continue;
            }
            let next_base = (base_index + 1..segment_end)
                .find(|candidate| is_base(script, items[*candidate].0))
                .unwrap_or(segment_end);
            let Some(prebase_index) = (base_index + 1..next_base)
                .find(|candidate| is_prebase_mark(script, items[*candidate].0))
            else {
                base_index = next_base;
                continue;
            };
            let item = items.remove(prebase_index);
            items.insert(base_index, item);
            changed = true;
            base_index += 1;
        }
        index = segment_end;
    }
    changed
}

fn reorder_myanmar_syllables(
    items: &mut Vec<(char, LayoutGlyph)>,
    replacement: &mut Vec<(char, LayoutGlyph)>,
) -> bool {
    let mut changed = false;
    let mut segment_start = 0;
    while segment_start < items.len() {
        while segment_start < items.len()
            && !is_script_codepoint(SoutheastAsianScript::Myanmar, items[segment_start].0)
        {
            segment_start += 1;
        }
        if segment_start == items.len() {
            break;
        }
        let segment_end = items[segment_start..]
            .iter()
            .position(|(codepoint, _)| {
                !is_script_codepoint(SoutheastAsianScript::Myanmar, *codepoint)
            })
            .map_or(items.len(), |offset| segment_start + offset);

        let mut syllable_start = segment_start;
        for index in segment_start + 1..segment_end {
            if is_myanmar_base_start(items, index) {
                changed |= reorder_myanmar_syllable(items, syllable_start, index, replacement);
                syllable_start = index;
            }
        }
        changed |= reorder_myanmar_syllable(items, syllable_start, segment_end, replacement);
        segment_start = segment_end;
    }
    changed
}

#[inline]
fn is_myanmar_kinzi_prefix(items: &[(char, LayoutGlyph)], index: usize) -> bool {
    items
        .get(index..index.saturating_add(3))
        .is_some_and(|sequence| {
            sequence[0].0 == '\u{1004}'
                && sequence[1].0 == '\u{103a}'
                && sequence[2].0 == '\u{1039}'
        })
}

#[inline]
fn is_myanmar_base_start(items: &[(char, LayoutGlyph)], index: usize) -> bool {
    if is_myanmar_kinzi_prefix(items, index) {
        return true;
    }
    let previous = index
        .checked_sub(1)
        .and_then(|previous| items.get(previous))
        .map(|(codepoint, _)| *codepoint);
    is_base(SoutheastAsianScript::Myanmar, items[index].0) && previous != Some('\u{1039}')
}

fn reorder_myanmar_syllable(
    items: &mut Vec<(char, LayoutGlyph)>,
    start: usize,
    end: usize,
    replacement: &mut Vec<(char, LayoutGlyph)>,
) -> bool {
    if start >= end {
        return false;
    }

    let kinzi_end = if is_myanmar_kinzi_prefix(items, start) {
        start + 3
    } else {
        start
    };
    let Some(base_index) =
        (kinzi_end..end).find(|index| is_base(SoutheastAsianScript::Myanmar, items[*index].0))
    else {
        return false;
    };

    let moved_prebase = (base_index + 1..end).any(|index| items[index].0 == '\u{1031}');
    let moved_medial_ra = (base_index + 1..end).any(|index| items[index].0 == '\u{103c}');
    replacement.clear();
    replacement.extend(
        (start..end)
            .filter(|index| *index >= kinzi_end && items[*index].0 == '\u{1031}')
            .map(|index| items[index]),
    );
    replacement.extend(
        (start..end)
            .filter(|index| *index >= kinzi_end && items[*index].0 == '\u{103c}')
            .map(|index| items[index]),
    );
    replacement.extend((start..base_index).filter_map(|index| {
        let item = items[index];
        (index >= kinzi_end && !matches!(item.0, '\u{1031}' | '\u{103c}')).then_some(item)
    }));
    let body_base_index = replacement.len();
    replacement.push(items[base_index]);
    if kinzi_end > start {
        replacement.extend(items[start..kinzi_end].iter().copied());
    }
    replacement.extend((base_index + 1..end).filter_map(|index| {
        let item = items[index];
        (!matches!(item.0, '\u{1031}' | '\u{103c}')).then_some(item)
    }));
    let moved_anusvara = reorder_myanmar_anusvara(replacement, body_base_index);

    let changed = moved_prebase || moved_medial_ra || kinzi_end > start || moved_anusvara;
    drop(items.splice(start..end, replacement.iter().copied()));
    replacement.clear();
    changed
}

fn reorder_myanmar_anusvara(body: &mut Vec<(char, LayoutGlyph)>, base_index: usize) -> bool {
    let mut changed = false;
    let mut index = base_index + 1;
    while index < body.len() {
        if body[index].0 != '\u{1036}' || !is_myanmar_below_vowel(body[index - 1].0) {
            index += 1;
            continue;
        }
        let mut below_start = index - 1;
        while below_start > base_index + 1 && is_myanmar_below_vowel(body[below_start - 1].0) {
            below_start -= 1;
        }
        let anusvara = body.remove(index);
        body.insert(below_start, anusvara);
        changed = true;
        index = below_start + 1;
    }
    changed
}

#[inline]
fn is_myanmar_below_vowel(codepoint: char) -> bool {
    matches!(codepoint, '\u{102f}' | '\u{1030}')
}

fn reorder_khmer_syllables(items: &mut Vec<(char, LayoutGlyph)>) -> bool {
    let mut changed = false;
    let mut segment_start = 0;
    while segment_start < items.len() {
        while segment_start < items.len()
            && !is_script_codepoint(SoutheastAsianScript::Khmer, items[segment_start].0)
        {
            segment_start += 1;
        }
        if segment_start == items.len() {
            break;
        }
        let segment_end = items[segment_start..]
            .iter()
            .position(|(codepoint, _)| {
                !is_script_codepoint(SoutheastAsianScript::Khmer, *codepoint)
            })
            .map_or(items.len(), |offset| segment_start + offset);

        let mut syllable_start = segment_start;
        for index in segment_start..segment_end {
            if index > segment_start && is_khmer_base_start(items, index) {
                changed |= reorder_khmer_syllable(items, syllable_start, index);
                syllable_start = index;
            }
        }
        changed |= reorder_khmer_syllable(items, syllable_start, segment_end);
        segment_start = segment_end;
    }
    changed
}

#[inline]
fn is_khmer_base_start(items: &[(char, LayoutGlyph)], index: usize) -> bool {
    is_base(SoutheastAsianScript::Khmer, items[index].0)
        && !items
            .get(index.checked_sub(1).unwrap_or(index))
            .is_some_and(|(codepoint, _)| *codepoint == '\u{17d2}')
}

fn reorder_khmer_syllable(items: &mut Vec<(char, LayoutGlyph)>, start: usize, end: usize) -> bool {
    if start >= end {
        return false;
    }
    let Some(base_index) = (start..end).find(|index| is_khmer_base_start(items, *index)) else {
        return false;
    };

    // Khmer's composite OO sign is expanded into E + AA before this pass.
    // Move the E component before the base, but leave the AA component after
    // it so the font's contextual lookups can turn the pair into the same
    // base-plus-AA form as the platform shaper. A raw E + OO pair follows the
    // same visual ordering through the `composite_oo_component` guard. The
    // subscript-RA leg is the other structural reorder this bounded path
    // performs before GSUB.
    let mut prebase_parts = Vec::new();
    let mut body = Vec::with_capacity(end - base_index);
    for (source_index, item) in items[base_index..end].iter().copied().enumerate() {
        let composite_oo_component = item.0 == '\u{17c4}'
            && source_index > 0
            && items[base_index + source_index - 1].0 == '\u{17c1}';
        if is_khmer_prebase_vowel(item.0) && !composite_oo_component {
            prebase_parts.push(item);
        } else {
            body.push(item);
        }
    }
    let mut preposed_ra = None;
    let mut index = 1;
    while index + 1 < body.len() {
        if body[index].0 == '\u{17d2}' && body[index + 1].0 == '\u{179a}' {
            preposed_ra = Some(index);
            break;
        }
        if is_base(SoutheastAsianScript::Khmer, body[index].0) && body[index].0 != '\u{17d2}' {
            break;
        }
        index += 1;
    }
    if let Some(index) = preposed_ra {
        prebase_parts.extend(body.drain(index..index + 2));
    }
    if prebase_parts.is_empty() {
        return false;
    }

    let mut replacement = items[start..base_index].to_vec();
    replacement.extend(prebase_parts);
    replacement.extend(body);
    items.splice(start..end, replacement);
    true
}

#[inline]
fn is_khmer_prebase_vowel(codepoint: char) -> bool {
    matches!(codepoint as u32, 0x17c1..=0x17c5)
}

fn apply_southeast_asian_gsub(
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    context_candidates: &mut Vec<u16>,
    gdef: &Gdef,
    state: &LayoutState,
    script: SoutheastAsianScript,
) -> Result<Option<(bool, bool)>, SfntError> {
    let Some(layout) = state.gsub.as_ref() else {
        return Ok(Some((false, false)));
    };
    let advances = face.glyph_advances_with_metrics(metrics)?;
    let feature_tags = match script {
        SoutheastAsianScript::Khmer => KHMER_GSUB_FEATURE_TAGS,
        SoutheastAsianScript::Myanmar => MYANMAR_GSUB_FEATURE_TAGS,
        _ => SOUTHEAST_ASIAN_GSUB_FEATURE_TAGS,
    };
    let script_tag = script_tag_with_features(layout, script, feature_tags);
    let mut supported = false;
    let mut changed = false;

    for feature_tag in feature_tags {
        let lookups = layout.feature_lookups_with_language(
            script_tag,
            None,
            std::slice::from_ref(feature_tag),
        );
        if lookups.is_empty() {
            continue;
        }
        supported = true;
        if lookups
            .iter()
            .any(|lookup| matches!(lookup.lookup_type, 5 | 6 | 7))
        {
            changed |= context::apply_contextual_substitutions(
                face,
                metrics,
                glyphs,
                gdef,
                layout,
                script_tag,
                std::slice::from_ref(feature_tag),
                context_candidates,
            )?;
        }
        for lookup in lookups {
            match lookup.execution_type {
                1 => {
                    changed |=
                        apply_single_lookup(face, metrics, advances, glyphs, gdef, layout, lookup)?;
                }
                2 => {
                    changed |= apply_multiple_lookup(face, advances, glyphs, layout, lookup)?;
                }
                3 => {
                    changed |= apply_alternate_lookup(face, metrics, glyphs, gdef, layout, lookup)?;
                }
                4 => {
                    changed |= apply_ligature_lookup(
                        face, metrics, advances, glyphs, gdef, layout, lookup,
                    )?;
                }
                7 => {
                    changed |=
                        apply_single_lookup(face, metrics, advances, glyphs, gdef, layout, lookup)?;
                    changed |= apply_multiple_lookup(face, advances, glyphs, layout, lookup)?;
                    changed |= apply_ligature_lookup(
                        face, metrics, advances, glyphs, gdef, layout, lookup,
                    )?;
                }
                5 | 6 => {}
                _ => return Ok(None),
            }
        }
    }

    Ok(Some((supported, changed)))
}

fn apply_southeast_asian_gpos(
    face: &SfntFace<'_>,
    glyphs: &mut [LayoutGlyph],
    codepoints: &[char],
    gdef: &Gdef,
    state: &LayoutState,
    script: SoutheastAsianScript,
) -> Result<Option<(bool, bool)>, SfntError> {
    let Some(layout) = state.gpos.as_ref() else {
        return Ok(Some((false, false)));
    };
    let script_tag = script_tag_with_features(layout, script, SOUTHEAST_ASIAN_GPOS_FEATURE_TAGS);
    let mut supported = false;
    let mut changed = false;
    let mut has_pair = false;
    let mut has_dist_pair = false;
    let mut has_mark_pair = false;
    let mut has_mark_lookup = false;
    let mut has_single_lookup = false;

    for feature_tag in SOUTHEAST_ASIAN_GPOS_FEATURE_TAGS {
        let lookups = layout.feature_lookups_with_language(
            script_tag,
            None,
            std::slice::from_ref(feature_tag),
        );
        if lookups.is_empty() {
            continue;
        }
        supported = true;
        for lookup in lookups {
            if *feature_tag == DIST_TAG {
                if matches!(lookup.lookup_type, 1) {
                    has_single_lookup = true;
                } else if matches!(lookup.lookup_type, 2 | 9) {
                    has_dist_pair = true;
                } else if lookup.lookup_type == 8 {
                    // Some faces add contextual `dist` lookups alongside
                    // their direct pair adjustments. The checked pair path
                    // below handles the measurable advance change; leave an
                    // unmatched contextual lookup inert until contextual
                    // GPOS positioning is implemented.
                } else {
                    return Ok(None);
                }
            } else if *feature_tag == KERN_TAG {
                if matches!(lookup.lookup_type, 2 | 9) {
                    has_pair = true;
                }
            } else if *feature_tag == MARK_TAG && lookup.lookup_type == 2 {
                // Myanmar Text includes pair positioning in its `mark`
                // feature alongside mark-to-base and mark-to-mark lookups.
                has_mark_pair = true;
            } else if matches!(lookup.lookup_type, 4 | 5 | 6 | 9) {
                has_mark_lookup = true;
            } else if matches!(lookup.lookup_type, 7 | 8) {
                // Contextual positioning lookups can wrap mark attachment
                // lookups. This bounded path cannot execute those rules; keep
                // supported script substitutions instead of abandoning the
                // entire run for scalar cmap output.
            } else {
                // Single/alternate positioning is outside this bounded
                // slice. Keep the checked scalar layout responsible for it.
                return Ok(None);
            }
        }
    }

    if has_single_lookup {
        let Some(single_changed) =
            apply_southeast_asian_single_positioning(face, glyphs, gdef, layout, script_tag)?
        else {
            return Ok(None);
        };
        changed |= single_changed;
    }

    if has_dist_pair {
        changed |= apply_gpos(
            face,
            glyphs,
            gdef,
            Some(layout),
            script_tag,
            None,
            std::slice::from_ref(&DIST_TAG),
        )?;
    }

    if has_pair {
        changed |= apply_gpos(
            face,
            glyphs,
            gdef,
            Some(layout),
            script_tag,
            None,
            std::slice::from_ref(&KERN_TAG),
        )?;
    }

    if has_mark_pair {
        changed |= apply_gpos(
            face,
            glyphs,
            gdef,
            Some(layout),
            script_tag,
            None,
            std::slice::from_ref(&MARK_TAG),
        )?;
    }

    if has_mark_lookup {
        let Some(mark_changed) =
            apply_sea_mark_positioning(face, glyphs, codepoints, gdef, layout, script_tag, script)?
        else {
            return Ok(None);
        };
        changed |= mark_changed;
    }

    Ok(Some((supported, changed)))
}

fn apply_southeast_asian_single_positioning(
    face: &SfntFace<'_>,
    glyphs: &mut [LayoutGlyph],
    gdef: &Gdef,
    layout: &LayoutTableState,
    script_tag: Tag,
) -> Result<Option<bool>, SfntError> {
    let lookups =
        layout.feature_lookups_with_language(script_tag, None, std::slice::from_ref(&DIST_TAG));
    if lookups.is_empty() {
        return Ok(Some(false));
    }
    if lookups
        .iter()
        .any(|lookup| !matches!(lookup.lookup_type, 1 | 9))
    {
        return Ok(None);
    }

    let table = layout.table(face)?;
    let mut changed = false;
    for lookup in lookups {
        for glyph in glyphs.iter_mut() {
            if gdef.ignores(glyph.glyph_id, lookup.lookup_flags)? {
                continue;
            }
            for subtable_offset in &lookup.subtable_offsets {
                let Some(adjustment) = single_position_adjustment(
                    table,
                    *subtable_offset,
                    lookup.lookup_type,
                    glyph.glyph_id,
                    0,
                )?
                else {
                    continue;
                };
                glyph.x_offset += adjustment.x_placement;
                glyph.y_offset += adjustment.y_placement;
                glyph.x_advance += adjustment.x_advance;
                glyph.y_advance += adjustment.y_advance;
                changed = true;
                break;
            }
        }
    }
    Ok(Some(changed))
}

fn single_position_adjustment(
    table: &[u8],
    subtable_offset: usize,
    lookup_type: u16,
    glyph_id: u16,
    extension_depth: u8,
) -> Result<Option<super::ValueAdjustment>, SfntError> {
    let subtable = slice_from(table, subtable_offset, GPOS_TAG)?;
    match lookup_type {
        1 => {
            let format = read_u16(subtable, 0, GPOS_TAG)?;
            let coverage_offset = usize::from(read_u16(subtable, 2, GPOS_TAG)?);
            let Some(coverage_index) =
                coverage_index(subtable, coverage_offset, glyph_id, GPOS_TAG)?
            else {
                return Ok(None);
            };
            let value_format = read_u16(subtable, 4, GPOS_TAG)?;
            let value_size = super::value_record_size(value_format, GPOS_TAG)?;
            let record_offset = match format {
                1 => 6,
                2 => {
                    let value_count = usize::from(read_u16(subtable, 6, GPOS_TAG)?);
                    if coverage_index >= value_count {
                        return Err(super::malformed(GPOS_TAG));
                    }
                    checked_add(8, checked_mul(coverage_index, value_size)?)?
                }
                _ => return Ok(None),
            };
            super::ensure(subtable, record_offset, value_size, GPOS_TAG)?;
            let (adjustment, _) =
                super::read_value_adjustment(subtable, record_offset, value_format, GPOS_TAG)?;
            Ok(Some(adjustment))
        }
        9 => {
            if extension_depth >= MAX_EXTENSION_DEPTH {
                return Err(super::malformed(GPOS_TAG));
            }
            if read_u16(subtable, 0, GPOS_TAG)? != 1 {
                return Ok(None);
            }
            let extension_type = read_u16(subtable, 2, GPOS_TAG)?;
            let extension_offset = usize::try_from(read_u32(subtable, 4, GPOS_TAG)?)
                .map_err(|_| SfntError::ArithmeticOverflow)?;
            let extension = checked_add(subtable_offset, extension_offset)?;
            single_position_adjustment(
                table,
                extension,
                extension_type,
                glyph_id,
                extension_depth + 1,
            )
        }
        _ => Ok(None),
    }
}

fn apply_sea_mark_positioning(
    face: &SfntFace<'_>,
    glyphs: &mut [LayoutGlyph],
    codepoints: &[char],
    gdef: &Gdef,
    layout: &LayoutTableState,
    script_tag: Tag,
    script: SoutheastAsianScript,
) -> Result<Option<bool>, SfntError> {
    if glyphs.len() != codepoints.len() {
        return Ok(None);
    }

    // Khmer applies below-base positioning first; Myanmar uses its required
    // `mark` feature before the optional above/below mark features.
    let base_feature_tags = match script {
        SoutheastAsianScript::Khmer => [BLWM_TAG, ABVM_TAG, MARK_TAG],
        SoutheastAsianScript::Myanmar => [MARK_TAG, ABVM_TAG, BLWM_TAG],
        _ => [ABVM_TAG, BLWM_TAG, MARK_TAG],
    };
    let mark_to_mark_lookups =
        layout.feature_lookups_with_language(script_tag, None, std::slice::from_ref(&MKMK_TAG));
    let has_base_lookups = base_feature_tags.iter().any(|feature_tag| {
        !layout
            .feature_lookups_with_language(script_tag, None, std::slice::from_ref(feature_tag))
            .is_empty()
    });
    if !has_base_lookups && mark_to_mark_lookups.is_empty() {
        return Ok(None);
    }

    let mut has_mark = false;
    for (index, glyph) in glyphs.iter().enumerate() {
        if is_mark(gdef, glyph.glyph_id, codepoints[index], script)? {
            has_mark = true;
            break;
        }
    }
    if !has_mark {
        return Ok(Some(false));
    }

    let table = layout.table(face)?;
    let mut changed = false;
    for mark_index in 0..glyphs.len() {
        if !is_mark(
            gdef,
            glyphs[mark_index].glyph_id,
            codepoints[mark_index],
            script,
        )? {
            continue;
        }
        let Some(base_index) = sea_base_index(glyphs, codepoints, gdef, script, mark_index)? else {
            continue;
        };
        let mark_to_mark = is_mark(
            gdef,
            glyphs[base_index].glyph_id,
            codepoints[base_index],
            script,
        )?;
        if mark_to_mark {
            for lookup in mark_to_mark_lookups {
                if apply_sea_mark_lookup(table, glyphs, gdef, lookup, mark_index, base_index, true)?
                {
                    changed = true;
                    break;
                }
            }
        } else {
            let mut attached = false;
            for feature_tag in base_feature_tags {
                for lookup in layout.feature_lookups_with_language(
                    script_tag,
                    None,
                    std::slice::from_ref(&feature_tag),
                ) {
                    if apply_sea_mark_lookup(
                        table, glyphs, gdef, lookup, mark_index, base_index, false,
                    )? {
                        changed = true;
                        attached = true;
                        break;
                    }
                }
                if attached {
                    break;
                }
            }
        }
    }

    Ok(Some(changed))
}

fn apply_sea_mark_lookup(
    table: &[u8],
    glyphs: &mut [LayoutGlyph],
    gdef: &Gdef,
    lookup: &super::LookupState,
    mark_index: usize,
    base_index: usize,
    mark_to_mark: bool,
) -> Result<bool, SfntError> {
    if gdef.ignores(glyphs[mark_index].glyph_id, lookup.lookup_flags)?
        || gdef.ignores(glyphs[base_index].glyph_id, lookup.lookup_flags)?
    {
        return Ok(false);
    }

    let base_x_offset = glyphs[base_index].x_offset;
    let base_y_offset = glyphs[base_index].y_offset;
    for (subtable_index, subtable_offset) in lookup.subtable_offsets.iter().enumerate() {
        let adjustment = lookup
            .compiled_subtables
            .get(subtable_index)
            .and_then(Option::as_ref)
            .filter(|compiled| compiled.is_mark_to_mark() == mark_to_mark)
            .map(|compiled| {
                compiled.mark_adjustment(glyphs[mark_index].glyph_id, glyphs[base_index].glyph_id)
            })
            .transpose()?
            .flatten();
        let adjustment = match adjustment {
            Some(adjustment) => Some(adjustment),
            None => {
                let subtable = slice_from(table, *subtable_offset, GPOS_TAG)?;
                if mark_to_mark {
                    mark_to_mark_adjustment_for_lookup(
                        subtable,
                        lookup.lookup_type,
                        glyphs[mark_index].glyph_id,
                        glyphs[base_index].glyph_id,
                        0,
                    )?
                } else {
                    mark_to_base_adjustment_for_lookup(
                        subtable,
                        lookup.lookup_type,
                        glyphs[mark_index].glyph_id,
                        glyphs[base_index].glyph_id,
                        0,
                    )?
                }
            }
        };
        let Some((mut x_offset, y_offset)) = adjustment else {
            continue;
        };
        if mark_index > base_index && !mark_to_mark {
            x_offset -= glyphs[base_index].x_advance;
        }
        glyphs[mark_index].x_offset = base_x_offset + x_offset;
        glyphs[mark_index].y_offset = base_y_offset + y_offset;
        glyphs[mark_index].x_advance = 0;
        glyphs[mark_index].y_advance = 0;
        glyphs[mark_index].cluster = glyphs[base_index].cluster;
        return Ok(true);
    }
    Ok(false)
}

fn sea_base_index(
    glyphs: &[LayoutGlyph],
    codepoints: &[char],
    gdef: &Gdef,
    script: SoutheastAsianScript,
    mark_index: usize,
) -> Result<Option<usize>, SfntError> {
    // A pre-base sign is visually stored before its base after Khmer
    // reordering, while its source cluster remains after that base. Prefer a
    // forward base whose source cluster is not later than the mark. The
    // cluster check also covers substituted signs whose codepoint metadata is
    // no longer enough to identify them as pre-base marks.
    for candidate in mark_index + 1..glyphs.len() {
        if !is_script_codepoint(script, codepoints[candidate]) {
            break;
        }
        if !is_mark(
            gdef,
            glyphs[candidate].glyph_id,
            codepoints[candidate],
            script,
        )? && glyphs[candidate].cluster <= glyphs[mark_index].cluster
        {
            return Ok(Some(candidate));
        }
    }
    for candidate in (0..mark_index).rev() {
        if !is_script_codepoint(script, codepoints[candidate]) {
            break;
        }
        if !is_mark(
            gdef,
            glyphs[candidate].glyph_id,
            codepoints[candidate],
            script,
        )? {
            return Ok(Some(candidate));
        }
    }
    if !is_prebase_mark(script, codepoints[mark_index]) {
        for candidate in mark_index + 1..glyphs.len() {
            if !is_script_codepoint(script, codepoints[candidate]) {
                break;
            }
            if !is_mark(
                gdef,
                glyphs[candidate].glyph_id,
                codepoints[candidate],
                script,
            )? {
                return Ok(Some(candidate));
            }
        }
    }
    Ok(None)
}

fn is_mark(
    gdef: &Gdef,
    glyph_id: u16,
    codepoint: char,
    script: SoutheastAsianScript,
) -> Result<bool, SfntError> {
    Ok(gdef.class(glyph_id)? == 3 || is_sea_mark(script, codepoint))
}

fn normalize_sea_clusters(
    glyphs: &mut [LayoutGlyph],
    codepoints: &[char],
    gdef: &Gdef,
    script: SoutheastAsianScript,
) -> Result<(), SfntError> {
    for mark_index in 0..glyphs.len() {
        if !is_mark(
            gdef,
            glyphs[mark_index].glyph_id,
            codepoints[mark_index],
            script,
        )? {
            continue;
        }
        if let Some(base_index) = sea_base_index(glyphs, codepoints, gdef, script, mark_index)? {
            glyphs[mark_index].cluster = glyphs[base_index].cluster;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_scripts_and_rejects_mixed_runs() {
        assert_eq!(script_for_text("สวัสดี"), Some(SoutheastAsianScript::Thai));
        assert_eq!(script_for_text("ສະບາຍດີ"), Some(SoutheastAsianScript::Lao));
        assert_eq!(script_for_text("សួស្តី"), Some(SoutheastAsianScript::Khmer));
        assert_eq!(
            script_for_text("မင်္ဂလာပါ"),
            Some(SoutheastAsianScript::Myanmar)
        );
        assert_eq!(script_for_text("สวัสดี ສະບາຍດີ"), None);
        assert_eq!(script_for_text("\u{0e31}"), None);
    }

    #[test]
    fn keeps_lao_spacing_vowels_as_bases() {
        assert!(!is_sea_mark(SoutheastAsianScript::Lao, '\u{0eb0}'));
        assert!(!is_sea_mark(SoutheastAsianScript::Lao, '\u{0eb2}'));
        assert!(is_sea_mark(SoutheastAsianScript::Lao, '\u{0eb4}'));
    }

    #[test]
    fn reorders_myanmar_prebase_vowel_without_crossing_a_base() {
        let mut items = vec![
            ('\u{1000}', LayoutGlyph::from_glyph_id(1, 0, 600)),
            ('\u{1031}', LayoutGlyph::from_glyph_id(2, 3, 0)),
            ('\u{1001}', LayoutGlyph::from_glyph_id(3, 6, 600)),
        ];

        assert!(reorder_prebase_marks(
            &mut items,
            SoutheastAsianScript::Myanmar,
            &mut Vec::new(),
        ));
        assert_eq!(
            items
                .iter()
                .map(|(codepoint, _)| *codepoint)
                .collect::<Vec<_>>(),
            vec!['\u{1031}', '\u{1000}', '\u{1001}']
        );
    }

    #[test]
    fn reorders_khmer_preposed_ra_and_e_vowel_inside_one_syllable() {
        let mut ra_items = vec![
            ('\u{1780}', LayoutGlyph::from_glyph_id(1, 0, 600)),
            ('\u{17d2}', LayoutGlyph::from_glyph_id(2, 0, 0)),
            ('\u{179a}', LayoutGlyph::from_glyph_id(3, 0, 292)),
        ];
        assert!(reorder_prebase_marks(
            &mut ra_items,
            SoutheastAsianScript::Khmer,
            &mut Vec::new(),
        ));
        assert_eq!(
            ra_items
                .iter()
                .map(|(codepoint, _)| *codepoint)
                .collect::<Vec<_>>(),
            vec!['\u{17d2}', '\u{179a}', '\u{1780}']
        );

        let mut e_items = vec![
            ('\u{1781}', LayoutGlyph::from_glyph_id(4, 0, 590)),
            ('\u{17d2}', LayoutGlyph::from_glyph_id(5, 0, 0)),
            ('\u{1798}', LayoutGlyph::from_glyph_id(6, 0, 0)),
            ('\u{17c1}', LayoutGlyph::from_glyph_id(7, 0, 312)),
        ];
        assert!(reorder_prebase_marks(
            &mut e_items,
            SoutheastAsianScript::Khmer,
            &mut Vec::new(),
        ));
        assert_eq!(
            e_items
                .iter()
                .map(|(codepoint, _)| *codepoint)
                .collect::<Vec<_>>(),
            vec!['\u{17c1}', '\u{1781}', '\u{17d2}', '\u{1798}']
        );
    }
}
