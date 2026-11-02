//! Checked Indic shaping primitives.
//!
//! This module intentionally owns a bounded, table-driven Indic slice. It
//! performs the Unicode-ordering step needed by pre-base vowel signs and then
//! runs the Indic OpenType feature sequence through the same checked GSUB and
//! GPOS readers used by the Latin, Arabic, and CJK paths. A lookup kind that
//! is not implemented here returns `Ok(None)` to keep the compatibility
//! shaper responsible for the complete script instead of emitting a partial
//! result.

use super::super::{FontMetrics, SfntError, SfntFace};
use super::{
    ABVM_TAG, AimerShapedRun, BENG_TAG, BENG2_TAG, BLWM_TAG, DEVA_TAG, DEVA2_TAG, GPOS_TAG,
    GSUB_TAG, GUJR_TAG, GUJR2_TAG, GURU_TAG, GURU2_TAG, Gdef, INDIC_GPOS_FEATURE_TAGS,
    INDIC_GSUB_FEATURE_TAGS, KERN_TAG, KNDA_TAG, KNDA2_TAG, LayoutGlyph, LayoutState,
    LayoutTableState, MARK_TAG, MAX_EXTENSION_DEPTH, MKMK_TAG, MLYM_TAG, MLYM2_TAG, ORYA_TAG,
    ORYA2_TAG, SINH_TAG, TAML_TAG, TAML2_TAG, TELU_TAG, TELU2_TAG, Tag, apply_gpos,
    apply_gsub_single_at, apply_gsub_subtable, checked_add, context, read_u16, read_u32,
    relative_offset, slice_from,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IndicScript {
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Sinhala,
}

impl IndicScript {
    fn tags(self) -> &'static [Tag] {
        match self {
            Self::Devanagari => &[DEVA2_TAG, DEVA_TAG],
            Self::Bengali => &[BENG2_TAG, BENG_TAG],
            Self::Gurmukhi => &[GURU2_TAG, GURU_TAG],
            Self::Gujarati => &[GUJR2_TAG, GUJR_TAG],
            Self::Oriya => &[ORYA2_TAG, ORYA_TAG],
            Self::Tamil => &[TAML2_TAG, TAML_TAG],
            Self::Telugu => &[TELU2_TAG, TELU_TAG],
            Self::Kannada => &[KNDA2_TAG, KNDA_TAG],
            Self::Malayalam => &[MLYM2_TAG, MLYM_TAG],
            Self::Sinhala => &[SINH_TAG],
        }
    }
}

fn feature_lookups<'a>(
    layout: &'a LayoutTableState,
    script: IndicScript,
    feature_tag: Tag,
) -> &'a [super::LookupState] {
    for script_tag in script.tags() {
        let lookups = layout.feature_lookups_with_language(
            *script_tag,
            None,
            std::slice::from_ref(&feature_tag),
        );
        if !lookups.is_empty() {
            return lookups;
        }
    }
    &[]
}

fn feature_script_tag(
    layout: &LayoutTableState,
    script: IndicScript,
    feature_tag: Tag,
) -> Option<Tag> {
    script.tags().iter().copied().find(|script_tag| {
        !layout
            .feature_lookups_with_language(*script_tag, None, &[feature_tag])
            .is_empty()
    })
}

/// Returns the single Indic script carried by a run, allowing only neutral
/// ASCII punctuation/spacing and the join controls used by Indic shaping.
pub(super) fn script_for_text(text: &str) -> Option<IndicScript> {
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
        has_base |= !is_indic_mark(current, codepoint) && !is_join_control(codepoint);
    }
    if !has_base {
        return None;
    }
    script
}

fn script_for_codepoint(codepoint: char) -> Option<IndicScript> {
    match codepoint as u32 {
        0x0900..=0x097f => Some(IndicScript::Devanagari),
        0x0980..=0x09ff => Some(IndicScript::Bengali),
        0x0a00..=0x0a7f => Some(IndicScript::Gurmukhi),
        0x0a80..=0x0aff => Some(IndicScript::Gujarati),
        0x0b00..=0x0b7f => Some(IndicScript::Oriya),
        0x0b80..=0x0bff => Some(IndicScript::Tamil),
        0x0c00..=0x0c7f => Some(IndicScript::Telugu),
        0x0c80..=0x0cff => Some(IndicScript::Kannada),
        0x0d00..=0x0d7f => Some(IndicScript::Malayalam),
        0x0d80..=0x0dff => Some(IndicScript::Sinhala),
        _ => None,
    }
}

fn is_neutral_context(codepoint: char) -> bool {
    codepoint.is_ascii_whitespace()
        || codepoint.is_ascii_punctuation()
        || codepoint.is_ascii_digit()
        || is_join_control(codepoint)
}

fn is_join_control(codepoint: char) -> bool {
    matches!(codepoint, '\u{200c}' | '\u{200d}')
}

fn is_indic_script_codepoint(script: IndicScript, codepoint: char) -> bool {
    script_for_codepoint(codepoint) == Some(script) || is_join_control(codepoint)
}

fn is_indic_mark(script: IndicScript, codepoint: char) -> bool {
    let codepoint = codepoint as u32;
    match script {
        IndicScript::Devanagari => {
            matches!(codepoint, 0x0900..=0x0903 | 0x093a..=0x094f | 0x0951..=0x0957 | 0x0962..=0x0963)
        }
        IndicScript::Bengali => {
            matches!(codepoint, 0x0981..=0x0983 | 0x09bc | 0x09be..=0x09cd | 0x09d7 | 0x09e2..=0x09e3)
        }
        IndicScript::Gurmukhi => {
            matches!(codepoint, 0x0a01..=0x0a03 | 0x0a3c | 0x0a3e..=0x0a4d | 0x0a51)
        }
        IndicScript::Gujarati => {
            matches!(codepoint, 0x0a81..=0x0a83 | 0x0abc | 0x0abe..=0x0acd | 0x0ae2..=0x0ae3)
        }
        IndicScript::Oriya => {
            matches!(codepoint, 0x0b01..=0x0b03 | 0x0b3c | 0x0b3e..=0x0b4d | 0x0b55 | 0x0b62..=0x0b63)
        }
        IndicScript::Tamil => matches!(codepoint, 0x0b82..=0x0b83 | 0x0bbe..=0x0bcd),
        IndicScript::Telugu => {
            matches!(codepoint, 0x0c00..=0x0c04 | 0x0c3e..=0x0c4d | 0x0c55..=0x0c56 | 0x0c62..=0x0c63)
        }
        IndicScript::Kannada => {
            matches!(codepoint, 0x0c81..=0x0c83 | 0x0cbc | 0x0cbe..=0x0ccd | 0x0ce2..=0x0ce3)
        }
        IndicScript::Malayalam => {
            matches!(codepoint, 0x0d00..=0x0d03 | 0x0d3b..=0x0d4d | 0x0d57 | 0x0d62..=0x0d63)
        }
        IndicScript::Sinhala => {
            matches!(codepoint, 0x0dca | 0x0dcf..=0x0df3)
        }
    }
}

fn is_prebase_matra(script: IndicScript, codepoint: char) -> bool {
    matches!(
        (script, codepoint as u32),
        (IndicScript::Devanagari, 0x093f | 0x0947..=0x0948)
            | (IndicScript::Bengali, 0x09bf | 0x09c7..=0x09c8)
            | (IndicScript::Gurmukhi, 0x0a3f)
            | (IndicScript::Gujarati, 0x0abf)
            | (IndicScript::Oriya, 0x0b3f | 0x0b47..=0x0b48)
            | (IndicScript::Tamil, 0x0bc6..=0x0bc8)
            | (IndicScript::Telugu, 0x0c46..=0x0c48)
            | (IndicScript::Kannada, 0x0cc6..=0x0cc8)
            | (IndicScript::Malayalam, 0x0d46..=0x0d48)
            | (IndicScript::Sinhala, 0x0dd9)
    )
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
    scratch.codepoints.clear();
    scratch.codepoints.extend(text.chars());
    scratch.glyphs.clear();
    if !face.for_each_glyph_with_advance(text, metrics, |cluster, glyph_id, advance| {
        scratch
            .glyphs
            .push(LayoutGlyph::from_glyph_id(glyph_id, cluster, advance));
    })? || scratch.glyphs.len() != scratch.codepoints.len()
    {
        return Ok(None);
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
    let reordered = reorder_prebase_matras(&mut scratch.items, script);
    scratch
        .glyphs
        .extend(scratch.items.drain(..).map(|(_, glyph)| glyph));
    let Some((gsub_supported, gsub_changed)) = apply_indic_gsub(
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
    let Some((gpos_supported, gpos_changed)) = apply_indic_gpos(
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
    normalize_indic_clusters(
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

fn normalize_indic_clusters(
    glyphs: &mut [LayoutGlyph],
    codepoints: &[char],
    gdef: &Gdef,
    script: IndicScript,
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
        if let Some(base_index) = indic_base_index(glyphs, codepoints, gdef, script, mark_index)? {
            glyphs[mark_index].cluster = glyphs[base_index].cluster;
        }
    }
    Ok(())
}

fn reorder_prebase_matras(items: &mut Vec<(char, LayoutGlyph)>, script: IndicScript) -> bool {
    let mut changed = false;
    let mut segment_start = 0;
    while segment_start < items.len() {
        while segment_start < items.len()
            && !is_indic_script_codepoint(script, items[segment_start].0)
        {
            segment_start += 1;
        }
        if segment_start == items.len() {
            break;
        }
        let mut segment_end = segment_start + 1;
        while segment_end < items.len() && is_indic_script_codepoint(script, items[segment_end].0) {
            segment_end += 1;
        }

        let mut syllable_start = segment_start;
        let mut index = segment_start + 1;
        while index <= segment_end {
            if index == segment_end
                || (is_indic_base(script, items[index].0)
                    && !is_linked_to_previous(items, script, index))
            {
                changed |= reorder_prebase_in_span(items, syllable_start, index, script);
                syllable_start = index;
            }
            index += 1;
        }
        segment_start = segment_end;
    }
    changed
}

fn reorder_prebase_in_span(
    items: &mut Vec<(char, LayoutGlyph)>,
    start: usize,
    end: usize,
    script: IndicScript,
) -> bool {
    let Some(base_index) = (start..end).find(|index| is_indic_base(script, items[*index].0)) else {
        return false;
    };
    // A linked consonant cluster must retain its logical order while GSUB
    // forms the conjunct. The final vowel-sign position is then represented
    // by the mark positioning data. Moving it before the cluster here would
    // prevent `pres`/`blwf` lookups from seeing the input sequence they were
    // authored for.
    if (start..end).any(|index| is_virama(script, items[index].0)) {
        return false;
    }
    if !items[base_index + 1..end]
        .iter()
        .any(|(codepoint, _)| is_prebase_matra(script, *codepoint))
    {
        return false;
    }
    let base_cluster = items[base_index].1.cluster;
    let mut prebase = Vec::new();
    let mut remaining = Vec::with_capacity(end - start);
    for item in items.drain(start..end) {
        if is_prebase_matra(script, item.0) {
            prebase.push(item);
        } else {
            remaining.push(item);
        }
    }
    let insertion = remaining
        .iter()
        .position(|(_, glyph)| glyph.cluster == base_cluster)
        .unwrap_or(remaining.len());
    remaining.splice(insertion..insertion, prebase);
    items.splice(start..start, remaining);
    true
}

fn is_indic_base(script: IndicScript, codepoint: char) -> bool {
    is_indic_script_codepoint(script, codepoint)
        && !is_indic_mark(script, codepoint)
        && !is_join_control(codepoint)
}

fn is_linked_to_previous(items: &[(char, LayoutGlyph)], script: IndicScript, index: usize) -> bool {
    let mut previous = index;
    while let Some(candidate) = previous.checked_sub(1) {
        if is_join_control(items[candidate].0) {
            previous = candidate;
            continue;
        }
        return is_virama(script, items[candidate].0);
    }
    false
}

fn is_virama(script: IndicScript, codepoint: char) -> bool {
    matches!(
        (script, codepoint as u32),
        (IndicScript::Devanagari, 0x094d)
            | (IndicScript::Bengali, 0x09cd)
            | (IndicScript::Gurmukhi, 0x0a4d)
            | (IndicScript::Gujarati, 0x0acd)
            | (IndicScript::Oriya, 0x0b4d)
            | (IndicScript::Tamil, 0x0bcd)
            | (IndicScript::Telugu, 0x0c4d)
            | (IndicScript::Kannada, 0x0ccd)
            | (IndicScript::Malayalam, 0x0d4d)
            | (IndicScript::Sinhala, 0x0dca)
    )
}

fn apply_indic_gsub(
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    context_candidates: &mut Vec<u16>,
    gdef: &Gdef,
    state: &LayoutState,
    script: IndicScript,
) -> Result<Option<(bool, bool)>, SfntError> {
    let Some(layout) = state.gsub.as_ref() else {
        return Ok(Some((false, false)));
    };
    let advances = face.glyph_advances_with_metrics(metrics)?;
    let mut supported = false;
    let mut changed = false;

    for feature_tag in INDIC_GSUB_FEATURE_TAGS {
        let Some(script_tag) = feature_script_tag(layout, script, *feature_tag) else {
            continue;
        };
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

pub(super) fn apply_single_lookup(
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    advances: &[u16],
    glyphs: &mut [LayoutGlyph],
    gdef: &Gdef,
    layout: &LayoutTableState,
    lookup: &super::LookupState,
) -> Result<bool, SfntError> {
    let table = layout.table(face)?;
    let mut changed = false;
    for glyph in glyphs.iter_mut() {
        if lookup.lookup_flags != 0 && gdef.ignores(glyph.glyph_id, lookup.lookup_flags)? {
            continue;
        }
        for (subtable_index, subtable_offset) in lookup.subtable_offsets.iter().enumerate() {
            if let Some(compiled) = lookup
                .compiled_subtables
                .get(subtable_index)
                .and_then(Option::as_ref)
            {
                let result = compiled.apply_single_at(advances, glyph)?;
                match result {
                    Some(true) => {
                        changed = true;
                        break;
                    }
                    Some(false) => continue,
                    None => {}
                }
            }
            let subtable = slice_from(table, *subtable_offset, GSUB_TAG)?;
            let result = apply_gsub_single_at(
                face,
                metrics,
                glyph,
                lookup.lookup_flags,
                lookup.execution_type,
                subtable,
                0,
            )?;
            if result {
                changed = true;
                break;
            }
        }
    }
    Ok(changed)
}

pub(super) fn apply_alternate_lookup(
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    layout: &LayoutTableState,
    lookup: &super::LookupState,
) -> Result<bool, SfntError> {
    let table = layout.table(face)?;
    let mut changed = false;
    for glyph_index in 0..glyphs.len() {
        if lookup.lookup_flags != 0
            && gdef.ignores(glyphs[glyph_index].glyph_id, lookup.lookup_flags)?
        {
            continue;
        }
        for subtable_offset in &lookup.subtable_offsets {
            let subtable = slice_from(table, *subtable_offset, GSUB_TAG)?;
            if super::apply_gsub_lookup_at(
                face,
                metrics,
                glyphs,
                gdef,
                lookup.lookup_flags,
                lookup.lookup_type,
                subtable,
                glyph_index,
                0,
            )? {
                changed = true;
                break;
            }
        }
    }
    Ok(changed)
}

pub(super) fn apply_ligature_lookup(
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    advances: &[u16],
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    layout: &LayoutTableState,
    lookup: &super::LookupState,
) -> Result<bool, SfntError> {
    let table = layout.table(face)?;
    let mut changed = false;
    for (subtable_index, subtable_offset) in lookup.subtable_offsets.iter().enumerate() {
        let previous_len = glyphs.len();
        if let Some(compiled) = lookup
            .compiled_subtables
            .get(subtable_index)
            .and_then(Option::as_ref)
        {
            if compiled
                .apply_ligature(advances, glyphs, gdef, lookup.lookup_flags)?
                .is_some()
            {
                changed |= glyphs.len() < previous_len;
                continue;
            }
        }
        let subtable = slice_from(table, *subtable_offset, GSUB_TAG)?;
        apply_gsub_subtable(
            face,
            metrics,
            glyphs,
            gdef,
            lookup.lookup_flags,
            lookup.execution_type,
            subtable,
            0,
        )?;
        changed |= glyphs.len() < previous_len;
    }
    Ok(changed)
}

pub(super) fn apply_multiple_lookup(
    face: &SfntFace<'_>,
    advances: &[u16],
    glyphs: &mut Vec<LayoutGlyph>,
    layout: &LayoutTableState,
    lookup: &super::LookupState,
) -> Result<bool, SfntError> {
    let table = layout.table(face)?;
    let mut changed = false;
    let mut index = 0;
    while index < glyphs.len() {
        let mut applied = false;
        for subtable_offset in &lookup.subtable_offsets {
            let subtable = slice_from(table, *subtable_offset, GSUB_TAG)?;
            let Some(replacement_ids) = multiple_substitution_for_glyph(
                glyphs[index].glyph_id,
                lookup.execution_type,
                subtable,
                0,
            )?
            else {
                continue;
            };
            if replacement_ids.is_empty() {
                return Err(super::malformed(GSUB_TAG));
            }
            let cluster = glyphs[index].cluster;
            let replacements = replacement_ids
                .into_iter()
                .map(|glyph_id| {
                    let advance = advances
                        .get(usize::from(glyph_id))
                        .copied()
                        .ok_or_else(|| super::malformed(GSUB_TAG))?;
                    Ok(LayoutGlyph::from_glyph_id(glyph_id, cluster, advance))
                })
                .collect::<Result<Vec<_>, SfntError>>()?;
            let replacement_count = replacements.len();
            glyphs.splice(index..=index, replacements);
            index += replacement_count;
            changed = true;
            applied = true;
            break;
        }
        if !applied {
            index += 1;
        }
    }
    Ok(changed)
}

pub(super) fn multiple_substitution_for_glyph(
    glyph_id: u16,
    lookup_type: u16,
    subtable: &[u8],
    extension_depth: u8,
) -> Result<Option<Vec<u16>>, SfntError> {
    match lookup_type {
        2 => {
            if super::read_u16(subtable, 0, GSUB_TAG)? != 1 {
                return Ok(None);
            }
            let coverage_offset = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
            let Some(coverage_index) =
                super::coverage_index(subtable, coverage_offset, glyph_id, GSUB_TAG)?
            else {
                return Ok(None);
            };
            let sequence_count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
            if coverage_index >= sequence_count {
                return Err(super::malformed(GSUB_TAG));
            }
            let sequence_offset = relative_offset(
                subtable,
                0,
                read_u16(subtable, 6 + coverage_index * 2, GSUB_TAG)?,
                GSUB_TAG,
            )?;
            let glyph_count = usize::from(read_u16(subtable, sequence_offset, GSUB_TAG)?);
            let glyph_data = checked_add(sequence_offset, 2)?;
            super::ensure(
                subtable,
                glyph_data,
                checked_add(glyph_count, glyph_count)?,
                GSUB_TAG,
            )?;
            let mut replacements = Vec::with_capacity(glyph_count);
            for index in 0..glyph_count {
                replacements.push(read_u16(subtable, glyph_data + index * 2, GSUB_TAG)?);
            }
            Ok(Some(replacements))
        }
        7 => {
            if extension_depth >= MAX_EXTENSION_DEPTH {
                return Err(super::malformed(GSUB_TAG));
            }
            if read_u16(subtable, 0, GSUB_TAG)? != 1 {
                return Ok(None);
            }
            let extension_type = read_u16(subtable, 2, GSUB_TAG)?;
            let extension_offset = usize::try_from(read_u32(subtable, 4, GSUB_TAG)?)
                .map_err(|_| SfntError::ArithmeticOverflow)?;
            let extension = slice_from(subtable, extension_offset, GSUB_TAG)?;
            multiple_substitution_for_glyph(
                glyph_id,
                extension_type,
                extension,
                extension_depth + 1,
            )
        }
        _ => Ok(None),
    }
}

fn apply_indic_gpos(
    face: &SfntFace<'_>,
    glyphs: &mut [LayoutGlyph],
    codepoints: &[char],
    gdef: &Gdef,
    state: &LayoutState,
    script: IndicScript,
) -> Result<Option<(bool, bool)>, SfntError> {
    let Some(layout) = state.gpos.as_ref() else {
        return Ok(Some((false, false)));
    };
    let mut supported = false;
    let mut changed = false;
    let mut has_pair = false;
    let mut has_mark = false;
    let mut pair_script_tag = None;

    for feature_tag in INDIC_GPOS_FEATURE_TAGS {
        let Some(script_tag) = feature_script_tag(layout, script, *feature_tag) else {
            continue;
        };
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
            if *feature_tag == KERN_TAG {
                if !matches!(lookup.lookup_type, 2 | 9) {
                    // Contextual kerning is optional. The checked pair
                    // path remains available, while an unsupported kern
                    // lookup is skipped and the checked scalar layout is not
                    // needed solely for a discretionary adjustment.
                    continue;
                }
                has_pair = true;
                pair_script_tag = Some(script_tag);
            } else {
                if !matches!(lookup.lookup_type, 4 | 5 | 6 | 9) {
                    return Ok(None);
                }
                has_mark = true;
            }
        }
    }

    if has_pair {
        changed |= apply_gpos(
            face,
            glyphs,
            gdef,
            Some(layout),
            pair_script_tag.unwrap_or_else(|| script.tags()[0]),
            None,
            std::slice::from_ref(&KERN_TAG),
        )?;
    }

    if has_mark {
        let Some(mark_changed) =
            apply_indic_mark_positioning(face, glyphs, codepoints, gdef, layout, script)?
        else {
            return Ok(None);
        };
        changed |= mark_changed;
    }

    Ok(Some((supported, changed)))
}

fn apply_indic_mark_positioning(
    face: &SfntFace<'_>,
    glyphs: &mut [LayoutGlyph],
    codepoints: &[char],
    gdef: &Gdef,
    layout: &LayoutTableState,
    script: IndicScript,
) -> Result<Option<bool>, SfntError> {
    if glyphs.len() != codepoints.len() {
        return Ok(None);
    }
    let base_feature_tags = [ABVM_TAG, BLWM_TAG, MARK_TAG];
    let mark_to_mark_lookups = feature_lookups(layout, script, MKMK_TAG);
    let has_base_lookups = base_feature_tags
        .iter()
        .any(|feature_tag| !feature_lookups(layout, script, *feature_tag).is_empty());
    let has_any_lookup = has_base_lookups || !mark_to_mark_lookups.is_empty();
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
    if !has_any_lookup {
        return Ok(None);
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
        let Some(base_index) = indic_base_index(glyphs, codepoints, gdef, script, mark_index)?
        else {
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
                if apply_indic_mark_lookup(
                    table, glyphs, gdef, lookup, mark_index, base_index, true,
                )? {
                    changed = true;
                    break;
                }
            }
        } else {
            let mut attached = false;
            for feature_tag in base_feature_tags {
                for lookup in feature_lookups(layout, script, feature_tag) {
                    if apply_indic_mark_lookup(
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

fn apply_indic_mark_lookup(
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
        let adjustment = if let Some(compiled) = lookup
            .compiled_subtables
            .get(subtable_index)
            .and_then(Option::as_ref)
        {
            if compiled.is_mark_to_mark() == mark_to_mark {
                compiled
                    .mark_adjustment(glyphs[mark_index].glyph_id, glyphs[base_index].glyph_id)?
            } else {
                None
            }
        } else {
            None
        };
        let adjustment = match adjustment {
            Some(adjustment) => Some(adjustment),
            None => {
                let subtable = slice_from(table, *subtable_offset, GPOS_TAG)?;
                if mark_to_mark {
                    super::mark_to_mark_adjustment_for_lookup(
                        subtable,
                        lookup.lookup_type,
                        glyphs[mark_index].glyph_id,
                        glyphs[base_index].glyph_id,
                        0,
                    )?
                } else {
                    super::mark_to_base_adjustment_for_lookup(
                        subtable,
                        lookup.lookup_type,
                        glyphs[mark_index].glyph_id,
                        glyphs[base_index].glyph_id,
                        0,
                    )?
                }
            }
        };
        let Some((x_offset, y_offset)) = adjustment else {
            continue;
        };
        let x_offset = if mark_index > base_index && !mark_to_mark {
            x_offset - glyphs[base_index].x_advance
        } else {
            x_offset
        };
        glyphs[mark_index].x_offset = base_x_offset + x_offset;
        glyphs[mark_index].y_offset = base_y_offset + y_offset;
        glyphs[mark_index].x_advance = 0;
        glyphs[mark_index].y_advance = 0;
        glyphs[mark_index].cluster = glyphs[base_index].cluster;
        return Ok(true);
    }
    Ok(false)
}

fn indic_base_index(
    glyphs: &[LayoutGlyph],
    codepoints: &[char],
    gdef: &Gdef,
    script: IndicScript,
    mark_index: usize,
) -> Result<Option<usize>, SfntError> {
    let forward = is_prebase_matra(script, codepoints[mark_index]);
    if forward {
        for candidate in mark_index + 1..glyphs.len() {
            if !is_indic_script_codepoint(script, codepoints[candidate]) {
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
    for candidate in (0..mark_index).rev() {
        if !is_indic_script_codepoint(script, codepoints[candidate]) {
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
    if !forward {
        for candidate in mark_index + 1..glyphs.len() {
            if !is_indic_script_codepoint(script, codepoints[candidate]) {
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
    script: IndicScript,
) -> Result<bool, SfntError> {
    Ok(gdef.class(glyph_id)? == 3 || is_indic_mark(script, codepoint))
}
