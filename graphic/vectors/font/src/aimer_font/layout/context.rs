//! Checked GSUB context support shared by the Arabic, Indic, and Southeast
//! Asian shaping slices.

use super::super::FontMetrics;
use super::{
    ARAB_TAG, CALT_TAG, ClassDef, GSUB_TAG, Gdef, LayoutGlyph, LayoutTableState, MAX_CONTEXT_DEPTH,
    SfntError, SfntFace, Tag, checked_add, checked_mul, ensure, malformed, next_unignored,
    previous_unignored, read_u16, read_u32, relative_offset, slice_from,
};

pub(super) fn apply_arabic_contextual_substitutions(
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    candidates: &mut Vec<u16>,
) -> Result<bool, SfntError> {
    apply_contextual_substitutions(
        face,
        metrics,
        glyphs,
        gdef,
        gsub,
        ARAB_TAG,
        &[CALT_TAG],
        candidates,
    )
}

pub(super) fn apply_contextual_substitutions(
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    script_tag: Tag,
    feature_tags: &[Tag],
    candidates: &mut Vec<u16>,
) -> Result<bool, SfntError> {
    let table = gsub.table(face)?;
    let mut changed = false;
    candidates.clear();
    let mut seen_glyphs = [0_u64; 1024];
    record_candidate_glyphs(glyphs, candidates, &mut seen_glyphs);

    for feature_tag in feature_tags {
        let lookup_indices =
            gsub.feature_lookup_indices(script_tag, std::slice::from_ref(feature_tag));
        for lookup_index in lookup_indices {
            let lookup = gsub.lookup(*lookup_index)?;
            for subtable_offset in &lookup.subtable_offsets {
                let subtable = slice_from(table, *subtable_offset, GSUB_TAG)?;
                if context_subtable_may_match(subtable, lookup.lookup_type, candidates, 0)?
                    == Some(false)
                {
                    continue;
                }
                if apply_gsub_contextual_subtable(
                    table,
                    face,
                    metrics,
                    glyphs,
                    gdef,
                    gsub,
                    lookup.lookup_flags,
                    lookup.lookup_type,
                    subtable,
                    0,
                )? {
                    changed = true;
                    record_candidate_glyphs(glyphs, candidates, &mut seen_glyphs);
                }
            }
        }
    }

    Ok(changed)
}

/// Adds the currently present glyph IDs to a compact reusable candidate list.
/// Contextual subtables are tried in their original order, but most of a large
/// lookup's subtables have a disjoint first-input coverage. Keeping this
/// over-approximation lets the hot loop skip those subtables without changing
/// any substitution order; stale IDs only cause an extra checked attempt.
fn record_candidate_glyphs(
    glyphs: &[LayoutGlyph],
    candidates: &mut Vec<u16>,
    seen: &mut [u64; 1024],
) {
    for glyph in glyphs {
        let glyph_id = glyph.glyph_id;
        let word = usize::from(glyph_id >> 6);
        let mask = 1_u64 << (glyph_id & 63);
        if seen[word] & mask == 0 {
            seen[word] |= mask;
            candidates.push(glyph_id);
        }
    }
}

/// Returns whether a contextual subtable can match any current first input.
/// `None` means the subtable format is outside this prefilter's bounded
/// decoder, so the normal checked implementation must still receive it.
fn context_subtable_may_match(
    subtable: &[u8],
    lookup_type: u16,
    candidates: &[u16],
    extension_depth: u8,
) -> Result<Option<bool>, SfntError> {
    if lookup_type == 7 {
        if extension_depth >= MAX_CONTEXT_DEPTH {
            return Err(malformed(GSUB_TAG));
        }
        if read_u16(subtable, 0, GSUB_TAG)? != 1 {
            return Ok(None);
        }
        let extension_type = read_u16(subtable, 2, GSUB_TAG)?;
        let extension_offset = usize::try_from(read_u32(subtable, 4, GSUB_TAG)?)
            .map_err(|_| SfntError::ArithmeticOverflow)?;
        let extension = slice_from(subtable, extension_offset, GSUB_TAG)?;
        return context_subtable_may_match(
            extension,
            extension_type,
            candidates,
            extension_depth + 1,
        );
    }

    let format = read_u16(subtable, 0, GSUB_TAG)?;
    let coverage_offset = match (lookup_type, format) {
        (5, 1 | 2) | (6, 1 | 2) => usize::from(read_u16(subtable, 2, GSUB_TAG)?),
        (5, 3) => {
            let glyph_count = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
            let substitution_count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
            if glyph_count == 0 {
                return Err(malformed(GSUB_TAG));
            }
            let coverage_offsets_end = checked_add(6, checked_mul(glyph_count, 2)?)?;
            ensure(subtable, 6, checked_mul(glyph_count, 2)?, GSUB_TAG)?;
            ensure(
                subtable,
                coverage_offsets_end,
                checked_mul(substitution_count, 4)?,
                GSUB_TAG,
            )?;
            usize::from(read_u16(subtable, 6, GSUB_TAG)?)
        }
        (6, 3) => {
            let mut cursor = 2;
            let backtrack_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
            cursor = checked_add(cursor, 2)?;
            cursor = checked_add(cursor, checked_mul(backtrack_count, 2)?)?;
            let input_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
            if input_count == 0 {
                return Err(malformed(GSUB_TAG));
            }
            cursor = checked_add(cursor, 2)?;
            let input_offsets_start = cursor;
            cursor = checked_add(cursor, checked_mul(input_count, 2)?)?;
            let lookahead_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
            cursor = checked_add(cursor, 2)?;
            cursor = checked_add(cursor, checked_mul(lookahead_count, 2)?)?;
            let substitution_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
            let record_offset = checked_add(cursor, 2)?;
            ensure(
                subtable,
                input_offsets_start,
                checked_mul(input_count, 2)?,
                GSUB_TAG,
            )?;
            ensure(
                subtable,
                record_offset,
                checked_mul(substitution_count, 4)?,
                GSUB_TAG,
            )?;
            usize::from(read_u16(subtable, input_offsets_start, GSUB_TAG)?)
        }
        _ => return Ok(None),
    };

    for glyph_id in candidates {
        if super::coverage_index(subtable, coverage_offset, *glyph_id, GSUB_TAG)?.is_some() {
            return Ok(Some(true));
        }
    }
    Ok(Some(false))
}

fn apply_gsub_contextual_subtable(
    table: &[u8],
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    lookup_flags: u16,
    lookup_type: u16,
    subtable: &[u8],
    context_depth: u8,
) -> Result<bool, SfntError> {
    match lookup_type {
        5 => match read_u16(subtable, 0, GSUB_TAG)? {
            1 => apply_gsub_context_format1(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                lookup_flags,
                subtable,
                context_depth,
            ),
            2 => apply_gsub_context_format2(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                lookup_flags,
                subtable,
                context_depth,
            ),
            3 => apply_gsub_context_format3(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                lookup_flags,
                subtable,
                context_depth,
            ),
            _ => Ok(false),
        },
        6 => match read_u16(subtable, 0, GSUB_TAG)? {
            1 => apply_gsub_chain_context_format1(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                lookup_flags,
                subtable,
                context_depth,
            ),
            2 => apply_gsub_chain_context_format2(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                lookup_flags,
                subtable,
                context_depth,
            ),
            3 => apply_gsub_chain_context_format3(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                lookup_flags,
                subtable,
                context_depth,
            ),
            _ => Ok(false),
        },
        7 => {
            if context_depth >= MAX_CONTEXT_DEPTH {
                return Err(malformed(GSUB_TAG));
            }
            if read_u16(subtable, 0, GSUB_TAG)? != 1 {
                return Ok(false);
            }
            let extension_type = read_u16(subtable, 2, GSUB_TAG)?;
            let extension_offset = usize::try_from(read_u32(subtable, 4, GSUB_TAG)?)
                .map_err(|_| SfntError::ArithmeticOverflow)?;
            let extension = slice_from(subtable, extension_offset, GSUB_TAG)?;
            apply_gsub_contextual_subtable(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                lookup_flags,
                extension_type,
                extension,
                context_depth + 1,
            )
        }
        _ => Ok(false),
    }
}

fn apply_gsub_context_format1(
    table: &[u8],
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    lookup_flags: u16,
    subtable: &[u8],
    context_depth: u8,
) -> Result<bool, SfntError> {
    if read_u16(subtable, 0, GSUB_TAG)? != 1 {
        return Ok(false);
    }
    let coverage_offset = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
    let rule_set_count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
    ensure(subtable, 6, checked_mul(rule_set_count, 2)?, GSUB_TAG)?;
    let mut changed = false;
    let mut first_index = 0;

    while first_index < glyphs.len() {
        if gdef.ignores(glyphs[first_index].glyph_id, lookup_flags)? {
            first_index += 1;
            continue;
        }
        let Some(rule_set_index) = super::coverage_index(
            subtable,
            coverage_offset,
            glyphs[first_index].glyph_id,
            GSUB_TAG,
        )?
        else {
            first_index += 1;
            continue;
        };
        if rule_set_index >= rule_set_count {
            return Err(malformed(GSUB_TAG));
        }
        let rule_set_offset = relative_offset(
            subtable,
            0,
            read_u16(subtable, 6 + rule_set_index * 2, GSUB_TAG)?,
            GSUB_TAG,
        )?;
        let rule_count = usize::from(read_u16(subtable, rule_set_offset, GSUB_TAG)?);
        ensure(
            subtable,
            checked_add(rule_set_offset, 2)?,
            checked_mul(rule_count, 2)?,
            GSUB_TAG,
        )?;

        for rule_index in 0..rule_count {
            let rule_offset = relative_offset(
                subtable,
                rule_set_offset,
                read_u16(
                    subtable,
                    checked_add(rule_set_offset, 2 + rule_index * 2)?,
                    GSUB_TAG,
                )?,
                GSUB_TAG,
            )?;
            let glyph_count = usize::from(read_u16(subtable, rule_offset, GSUB_TAG)?);
            let substitution_count = usize::from(read_u16(subtable, rule_offset + 2, GSUB_TAG)?);
            if glyph_count == 0 {
                return Err(malformed(GSUB_TAG));
            }
            let input_count = glyph_count - 1;
            let input_offset = checked_add(rule_offset, 4)?;
            ensure(
                subtable,
                input_offset,
                checked_mul(input_count, 2)?,
                GSUB_TAG,
            )?;
            let mut input_indices = Vec::with_capacity(glyph_count);
            input_indices.push(first_index);
            let mut previous_index = first_index;
            let mut matched = true;
            for input_index in 0..input_count {
                let Some(candidate) =
                    next_unignored(glyphs, previous_index.saturating_add(1), lookup_flags, gdef)?
                else {
                    matched = false;
                    break;
                };
                let expected = read_u16(subtable, input_offset + input_index * 2, GSUB_TAG)?;
                if glyphs[candidate].glyph_id != expected {
                    matched = false;
                    break;
                }
                input_indices.push(candidate);
                previous_index = candidate;
            }
            if !matched {
                continue;
            }
            let record_offset = checked_add(input_offset, checked_mul(input_count, 2)?)?;
            ensure(
                subtable,
                record_offset,
                checked_mul(substitution_count, 4)?,
                GSUB_TAG,
            )?;
            if apply_gsub_context_records(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                subtable,
                &input_indices,
                record_offset,
                substitution_count,
                context_depth,
            )? {
                changed = true;
            }
            break;
        }
        first_index += 1;
    }
    Ok(changed)
}

fn apply_gsub_context_format2(
    table: &[u8],
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    lookup_flags: u16,
    subtable: &[u8],
    context_depth: u8,
) -> Result<bool, SfntError> {
    let coverage_offset = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
    let class_definition_offset = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
    let class_set_count = usize::from(read_u16(subtable, 6, GSUB_TAG)?);
    ensure(subtable, 8, checked_mul(class_set_count, 2)?, GSUB_TAG)?;
    let class_definition = ClassDef::new(subtable, class_definition_offset, GSUB_TAG)?;
    let mut changed = false;
    let mut first_index = 0;

    while first_index < glyphs.len() {
        if gdef.ignores(glyphs[first_index].glyph_id, lookup_flags)?
            || super::coverage_index(
                subtable,
                coverage_offset,
                glyphs[first_index].glyph_id,
                GSUB_TAG,
            )?
            .is_none()
        {
            first_index += 1;
            continue;
        }
        let first_class = usize::from(class_definition_class(
            class_definition.as_ref(),
            glyphs[first_index].glyph_id,
        )?);
        if first_class >= class_set_count {
            return Err(malformed(GSUB_TAG));
        }
        let class_set_offset = relative_offset(
            subtable,
            0,
            read_u16(subtable, 8 + first_class * 2, GSUB_TAG)?,
            GSUB_TAG,
        )?;
        let rule_count = usize::from(read_u16(subtable, class_set_offset, GSUB_TAG)?);
        ensure(
            subtable,
            checked_add(class_set_offset, 2)?,
            checked_mul(rule_count, 2)?,
            GSUB_TAG,
        )?;

        for rule_index in 0..rule_count {
            let rule_offset = relative_offset(
                subtable,
                class_set_offset,
                read_u16(
                    subtable,
                    checked_add(class_set_offset, 2 + rule_index * 2)?,
                    GSUB_TAG,
                )?,
                GSUB_TAG,
            )?;
            let glyph_count = usize::from(read_u16(subtable, rule_offset, GSUB_TAG)?);
            let substitution_count = usize::from(read_u16(subtable, rule_offset + 2, GSUB_TAG)?);
            if glyph_count == 0 {
                return Err(malformed(GSUB_TAG));
            }
            let input_count = glyph_count - 1;
            let input_offset = checked_add(rule_offset, 4)?;
            ensure(
                subtable,
                input_offset,
                checked_mul(input_count, 2)?,
                GSUB_TAG,
            )?;
            let mut input_indices = Vec::with_capacity(glyph_count);
            input_indices.push(first_index);
            let mut previous_index = first_index;
            let mut matched = true;
            for input_index in 0..input_count {
                let Some(candidate) =
                    next_unignored(glyphs, previous_index.saturating_add(1), lookup_flags, gdef)?
                else {
                    matched = false;
                    break;
                };
                let expected_class = read_u16(subtable, input_offset + input_index * 2, GSUB_TAG)?;
                let actual_class =
                    class_definition_class(class_definition.as_ref(), glyphs[candidate].glyph_id)?;
                if actual_class != expected_class {
                    matched = false;
                    break;
                }
                input_indices.push(candidate);
                previous_index = candidate;
            }
            if !matched {
                continue;
            }
            let record_offset = checked_add(input_offset, checked_mul(input_count, 2)?)?;
            ensure(
                subtable,
                record_offset,
                checked_mul(substitution_count, 4)?,
                GSUB_TAG,
            )?;
            if apply_gsub_context_records(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                subtable,
                &input_indices,
                record_offset,
                substitution_count,
                context_depth,
            )? {
                changed = true;
            }
            break;
        }
        first_index += 1;
    }

    Ok(changed)
}

fn apply_gsub_context_format3(
    table: &[u8],
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    lookup_flags: u16,
    subtable: &[u8],
    context_depth: u8,
) -> Result<bool, SfntError> {
    let glyph_count = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
    let substitution_count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
    if glyph_count == 0 {
        return Err(malformed(GSUB_TAG));
    }
    let coverage_offsets_end = checked_add(6, checked_mul(glyph_count, 2)?)?;
    let record_offset = coverage_offsets_end;
    ensure(
        subtable,
        record_offset,
        checked_mul(substitution_count, 4)?,
        GSUB_TAG,
    )?;
    let mut changed = false;
    let mut first_index = 0;

    while first_index < glyphs.len() {
        let Some(input_indices) = match_coverage_sequence(
            subtable,
            6,
            glyph_count,
            first_index,
            glyphs,
            lookup_flags,
            gdef,
        )?
        else {
            first_index += 1;
            continue;
        };
        if apply_gsub_context_records(
            table,
            face,
            metrics,
            glyphs,
            gdef,
            gsub,
            subtable,
            &input_indices,
            record_offset,
            substitution_count,
            context_depth,
        )? {
            changed = true;
        }
        first_index += 1;
    }

    Ok(changed)
}

fn class_definition_class(definition: Option<&ClassDef>, glyph_id: u16) -> Result<u16, SfntError> {
    definition.map_or(Ok(0), |definition| definition.class(glyph_id))
}

fn match_coverage_sequence(
    subtable: &[u8],
    coverage_offsets_start: usize,
    glyph_count: usize,
    first_index: usize,
    glyphs: &[LayoutGlyph],
    lookup_flags: u16,
    gdef: &Gdef,
) -> Result<Option<Vec<usize>>, SfntError> {
    if glyphs.get(first_index).is_none()
        || gdef.ignores(glyphs[first_index].glyph_id, lookup_flags)?
    {
        return Ok(None);
    }
    let mut input_indices = Vec::with_capacity(glyph_count);
    input_indices.push(first_index);
    if super::coverage_index(
        subtable,
        usize::from(read_u16(subtable, coverage_offsets_start, GSUB_TAG)?),
        glyphs[first_index].glyph_id,
        GSUB_TAG,
    )?
    .is_none()
    {
        return Ok(None);
    }

    let mut previous_index = first_index;
    for position in 1..glyph_count {
        let Some(candidate) =
            next_unignored(glyphs, previous_index.saturating_add(1), lookup_flags, gdef)?
        else {
            return Ok(None);
        };
        let coverage_offset = usize::from(read_u16(
            subtable,
            checked_add(coverage_offsets_start, checked_mul(position, 2)?)?,
            GSUB_TAG,
        )?);
        if super::coverage_index(
            subtable,
            coverage_offset,
            glyphs[candidate].glyph_id,
            GSUB_TAG,
        )?
        .is_none()
        {
            return Ok(None);
        }
        input_indices.push(candidate);
        previous_index = candidate;
    }
    Ok(Some(input_indices))
}

fn apply_gsub_chain_context_format1(
    table: &[u8],
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    lookup_flags: u16,
    subtable: &[u8],
    context_depth: u8,
) -> Result<bool, SfntError> {
    if read_u16(subtable, 0, GSUB_TAG)? != 1 {
        return Ok(false);
    }
    let coverage_offset = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
    let rule_set_count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
    ensure(subtable, 6, checked_mul(rule_set_count, 2)?, GSUB_TAG)?;
    let mut changed = false;
    let mut first_index = 0;

    while first_index < glyphs.len() {
        if gdef.ignores(glyphs[first_index].glyph_id, lookup_flags)? {
            first_index += 1;
            continue;
        }
        let Some(rule_set_index) = super::coverage_index(
            subtable,
            coverage_offset,
            glyphs[first_index].glyph_id,
            GSUB_TAG,
        )?
        else {
            first_index += 1;
            continue;
        };
        if rule_set_index >= rule_set_count {
            return Err(malformed(GSUB_TAG));
        }

        let rule_set_offset = relative_offset(
            subtable,
            0,
            read_u16(subtable, 6 + rule_set_index * 2, GSUB_TAG)?,
            GSUB_TAG,
        )?;
        let rule_count = usize::from(read_u16(subtable, rule_set_offset, GSUB_TAG)?);
        ensure(
            subtable,
            checked_add(rule_set_offset, 2)?,
            checked_mul(rule_count, 2)?,
            GSUB_TAG,
        )?;

        let mut matched = None;
        for rule_index in 0..rule_count {
            let rule_offset = relative_offset(
                subtable,
                rule_set_offset,
                read_u16(
                    subtable,
                    checked_add(
                        checked_add(rule_set_offset, 2)?,
                        checked_mul(rule_index, 2)?,
                    )?,
                    GSUB_TAG,
                )?,
                GSUB_TAG,
            )?;
            if let Some(rule) = match_gsub_chain_rule_format1(
                subtable,
                rule_offset,
                first_index,
                glyphs,
                lookup_flags,
                gdef,
            )? {
                matched = Some(rule);
                break;
            }
        }

        if let Some((input_indices, record_offset, substitution_count)) = matched {
            if apply_gsub_context_records(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                subtable,
                &input_indices,
                record_offset,
                substitution_count,
                context_depth,
            )? {
                changed = true;
            }
        }
        first_index += 1;
    }

    Ok(changed)
}

fn apply_gsub_chain_context_format2(
    table: &[u8],
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    lookup_flags: u16,
    subtable: &[u8],
    context_depth: u8,
) -> Result<bool, SfntError> {
    let coverage_offset = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
    let backtrack_class_definition = ClassDef::new(
        subtable,
        usize::from(read_u16(subtable, 4, GSUB_TAG)?),
        GSUB_TAG,
    )?;
    let input_class_definition = ClassDef::new(
        subtable,
        usize::from(read_u16(subtable, 6, GSUB_TAG)?),
        GSUB_TAG,
    )?;
    let lookahead_class_definition = ClassDef::new(
        subtable,
        usize::from(read_u16(subtable, 8, GSUB_TAG)?),
        GSUB_TAG,
    )?;
    let class_set_count = usize::from(read_u16(subtable, 10, GSUB_TAG)?);
    ensure(subtable, 12, checked_mul(class_set_count, 2)?, GSUB_TAG)?;
    let mut changed = false;
    let mut first_index = 0;

    while first_index < glyphs.len() {
        if gdef.ignores(glyphs[first_index].glyph_id, lookup_flags)?
            || super::coverage_index(
                subtable,
                coverage_offset,
                glyphs[first_index].glyph_id,
                GSUB_TAG,
            )?
            .is_none()
        {
            first_index += 1;
            continue;
        }
        let first_class = usize::from(class_definition_class(
            input_class_definition.as_ref(),
            glyphs[first_index].glyph_id,
        )?);
        if first_class >= class_set_count {
            return Err(malformed(GSUB_TAG));
        }
        let class_set_offset = relative_offset(
            subtable,
            0,
            read_u16(subtable, 12 + first_class * 2, GSUB_TAG)?,
            GSUB_TAG,
        )?;
        let rule_count = usize::from(read_u16(subtable, class_set_offset, GSUB_TAG)?);
        ensure(
            subtable,
            checked_add(class_set_offset, 2)?,
            checked_mul(rule_count, 2)?,
            GSUB_TAG,
        )?;

        for rule_index in 0..rule_count {
            let rule_offset = relative_offset(
                subtable,
                class_set_offset,
                read_u16(
                    subtable,
                    checked_add(class_set_offset, 2 + rule_index * 2)?,
                    GSUB_TAG,
                )?,
                GSUB_TAG,
            )?;
            let Some((input_indices, record_offset, substitution_count)) =
                match_gsub_chain_rule_format2(
                    subtable,
                    rule_offset,
                    first_index,
                    glyphs,
                    lookup_flags,
                    gdef,
                    backtrack_class_definition.as_ref(),
                    input_class_definition.as_ref(),
                    lookahead_class_definition.as_ref(),
                )?
            else {
                continue;
            };
            if apply_gsub_context_records(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                subtable,
                &input_indices,
                record_offset,
                substitution_count,
                context_depth,
            )? {
                changed = true;
            }
            break;
        }
        first_index += 1;
    }

    Ok(changed)
}

fn match_gsub_chain_rule_format2(
    subtable: &[u8],
    rule_offset: usize,
    first_index: usize,
    glyphs: &[LayoutGlyph],
    lookup_flags: u16,
    gdef: &Gdef,
    backtrack_class_definition: Option<&ClassDef>,
    input_class_definition: Option<&ClassDef>,
    lookahead_class_definition: Option<&ClassDef>,
) -> Result<Option<(Vec<usize>, usize, usize)>, SfntError> {
    let mut cursor = rule_offset;
    let backtrack_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    cursor = checked_add(cursor, 2)?;
    ensure(subtable, cursor, checked_mul(backtrack_count, 2)?, GSUB_TAG)?;
    let mut previous_index = first_index;
    for index in 0..backtrack_count {
        let Some(candidate) = previous_unignored(glyphs, previous_index, lookup_flags, gdef)?
        else {
            return Ok(None);
        };
        let expected_class = read_u16(
            subtable,
            checked_add(cursor, checked_mul(index, 2)?)?,
            GSUB_TAG,
        )?;
        if class_definition_class(backtrack_class_definition, glyphs[candidate].glyph_id)?
            != expected_class
        {
            return Ok(None);
        }
        previous_index = candidate;
    }
    cursor = checked_add(cursor, checked_mul(backtrack_count, 2)?)?;

    let input_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    if input_count == 0 {
        return Err(malformed(GSUB_TAG));
    }
    cursor = checked_add(cursor, 2)?;
    let input_tail_count = input_count - 1;
    ensure(
        subtable,
        cursor,
        checked_mul(input_tail_count, 2)?,
        GSUB_TAG,
    )?;
    let mut input_indices = Vec::with_capacity(input_count);
    input_indices.push(first_index);
    let mut last_input_index = first_index;
    for index in 0..input_tail_count {
        let Some(candidate) = next_unignored(
            glyphs,
            last_input_index.saturating_add(1),
            lookup_flags,
            gdef,
        )?
        else {
            return Ok(None);
        };
        let expected_class = read_u16(
            subtable,
            checked_add(cursor, checked_mul(index, 2)?)?,
            GSUB_TAG,
        )?;
        if class_definition_class(input_class_definition, glyphs[candidate].glyph_id)?
            != expected_class
        {
            return Ok(None);
        }
        input_indices.push(candidate);
        last_input_index = candidate;
    }
    cursor = checked_add(cursor, checked_mul(input_tail_count, 2)?)?;

    let lookahead_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    cursor = checked_add(cursor, 2)?;
    ensure(subtable, cursor, checked_mul(lookahead_count, 2)?, GSUB_TAG)?;
    let mut lookahead_index = last_input_index;
    for index in 0..lookahead_count {
        let Some(candidate) = next_unignored(
            glyphs,
            lookahead_index.saturating_add(1),
            lookup_flags,
            gdef,
        )?
        else {
            return Ok(None);
        };
        let expected_class = read_u16(
            subtable,
            checked_add(cursor, checked_mul(index, 2)?)?,
            GSUB_TAG,
        )?;
        if class_definition_class(lookahead_class_definition, glyphs[candidate].glyph_id)?
            != expected_class
        {
            return Ok(None);
        }
        lookahead_index = candidate;
    }
    cursor = checked_add(cursor, checked_mul(lookahead_count, 2)?)?;

    let substitution_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    let record_offset = checked_add(cursor, 2)?;
    ensure(
        subtable,
        record_offset,
        checked_mul(substitution_count, 4)?,
        GSUB_TAG,
    )?;
    Ok(Some((input_indices, record_offset, substitution_count)))
}

fn apply_gsub_chain_context_format3(
    table: &[u8],
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    lookup_flags: u16,
    subtable: &[u8],
    context_depth: u8,
) -> Result<bool, SfntError> {
    let mut cursor = 2;
    let backtrack_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    cursor = checked_add(cursor, 2)?;
    let backtrack_offsets_start = cursor;
    cursor = checked_add(cursor, checked_mul(backtrack_count, 2)?)?;
    let input_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    if input_count == 0 {
        return Err(malformed(GSUB_TAG));
    }
    cursor = checked_add(cursor, 2)?;
    let input_offsets_start = cursor;
    cursor = checked_add(cursor, checked_mul(input_count, 2)?)?;
    let lookahead_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    cursor = checked_add(cursor, 2)?;
    let lookahead_offsets_start = cursor;
    cursor = checked_add(cursor, checked_mul(lookahead_count, 2)?)?;
    let substitution_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    let record_offset = checked_add(cursor, 2)?;
    ensure(
        subtable,
        record_offset,
        checked_mul(substitution_count, 4)?,
        GSUB_TAG,
    )?;

    let mut changed = false;
    let mut first_index = 0;
    while first_index < glyphs.len() {
        let Some(input_indices) = match_coverage_sequence(
            subtable,
            input_offsets_start,
            input_count,
            first_index,
            glyphs,
            lookup_flags,
            gdef,
        )?
        else {
            first_index += 1;
            continue;
        };
        let mut previous_index = first_index;
        let mut matched = true;
        for index in 0..backtrack_count {
            let Some(candidate) = previous_unignored(glyphs, previous_index, lookup_flags, gdef)?
            else {
                matched = false;
                break;
            };
            let coverage_offset = usize::from(read_u16(
                subtable,
                checked_add(backtrack_offsets_start, checked_mul(index, 2)?)?,
                GSUB_TAG,
            )?);
            if super::coverage_index(
                subtable,
                coverage_offset,
                glyphs[candidate].glyph_id,
                GSUB_TAG,
            )?
            .is_none()
            {
                matched = false;
                break;
            }
            previous_index = candidate;
        }
        if !matched {
            first_index += 1;
            continue;
        }
        let mut lookahead_index = *input_indices.last().ok_or_else(|| malformed(GSUB_TAG))?;
        for index in 0..lookahead_count {
            let Some(candidate) = next_unignored(
                glyphs,
                lookahead_index.saturating_add(1),
                lookup_flags,
                gdef,
            )?
            else {
                matched = false;
                break;
            };
            let coverage_offset = usize::from(read_u16(
                subtable,
                checked_add(lookahead_offsets_start, checked_mul(index, 2)?)?,
                GSUB_TAG,
            )?);
            if super::coverage_index(
                subtable,
                coverage_offset,
                glyphs[candidate].glyph_id,
                GSUB_TAG,
            )?
            .is_none()
            {
                matched = false;
                break;
            }
            lookahead_index = candidate;
        }
        if matched
            && apply_gsub_context_records(
                table,
                face,
                metrics,
                glyphs,
                gdef,
                gsub,
                subtable,
                &input_indices,
                record_offset,
                substitution_count,
                context_depth,
            )?
        {
            changed = true;
        }
        first_index += 1;
    }

    Ok(changed)
}

fn match_gsub_chain_rule_format1(
    subtable: &[u8],
    rule_offset: usize,
    first_index: usize,
    glyphs: &[LayoutGlyph],
    lookup_flags: u16,
    gdef: &Gdef,
) -> Result<Option<(Vec<usize>, usize, usize)>, SfntError> {
    let mut cursor = rule_offset;
    let backtrack_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    cursor = checked_add(cursor, 2)?;
    ensure(subtable, cursor, checked_mul(backtrack_count, 2)?, GSUB_TAG)?;
    let mut previous_index = first_index;
    for index in 0..backtrack_count {
        let Some(candidate) = previous_unignored(glyphs, previous_index, lookup_flags, gdef)?
        else {
            return Ok(None);
        };
        let expected = read_u16(
            subtable,
            checked_add(cursor, checked_mul(index, 2)?)?,
            GSUB_TAG,
        )?;
        if glyphs[candidate].glyph_id != expected {
            return Ok(None);
        }
        previous_index = candidate;
    }
    cursor = checked_add(cursor, checked_mul(backtrack_count, 2)?)?;

    let input_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    if input_count == 0 {
        return Err(malformed(GSUB_TAG));
    }
    cursor = checked_add(cursor, 2)?;
    let input_glyph_count = input_count - 1;
    ensure(
        subtable,
        cursor,
        checked_mul(input_glyph_count, 2)?,
        GSUB_TAG,
    )?;
    let mut input_indices = Vec::with_capacity(input_count);
    input_indices.push(first_index);
    let mut last_input_index = first_index;
    for index in 0..input_glyph_count {
        let Some(candidate) = next_unignored(
            glyphs,
            last_input_index.saturating_add(1),
            lookup_flags,
            gdef,
        )?
        else {
            return Ok(None);
        };
        let expected = read_u16(
            subtable,
            checked_add(cursor, checked_mul(index, 2)?)?,
            GSUB_TAG,
        )?;
        if glyphs[candidate].glyph_id != expected {
            return Ok(None);
        }
        input_indices.push(candidate);
        last_input_index = candidate;
    }
    cursor = checked_add(cursor, checked_mul(input_glyph_count, 2)?)?;

    let lookahead_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    cursor = checked_add(cursor, 2)?;
    ensure(subtable, cursor, checked_mul(lookahead_count, 2)?, GSUB_TAG)?;
    let mut lookahead_index = last_input_index;
    for index in 0..lookahead_count {
        let Some(candidate) = next_unignored(
            glyphs,
            lookahead_index.saturating_add(1),
            lookup_flags,
            gdef,
        )?
        else {
            return Ok(None);
        };
        let expected = read_u16(
            subtable,
            checked_add(cursor, checked_mul(index, 2)?)?,
            GSUB_TAG,
        )?;
        if glyphs[candidate].glyph_id != expected {
            return Ok(None);
        }
        lookahead_index = candidate;
    }
    cursor = checked_add(cursor, checked_mul(lookahead_count, 2)?)?;

    let substitution_count = usize::from(read_u16(subtable, cursor, GSUB_TAG)?);
    let record_offset = checked_add(cursor, 2)?;
    ensure(
        subtable,
        record_offset,
        checked_mul(substitution_count, 4)?,
        GSUB_TAG,
    )?;
    Ok(Some((input_indices, record_offset, substitution_count)))
}

fn apply_gsub_context_records(
    table: &[u8],
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    subtable: &[u8],
    input_indices: &[usize],
    record_offset: usize,
    substitution_count: usize,
    context_depth: u8,
) -> Result<bool, SfntError> {
    let mut changed = false;
    for index in 0..substitution_count {
        let record = checked_add(record_offset, checked_mul(index, 4)?)?;
        let sequence_index = usize::from(read_u16(subtable, record, GSUB_TAG)?);
        let lookup_index = read_u16(subtable, checked_add(record, 2)?, GSUB_TAG)?;
        let Some(target_index) = input_indices.get(sequence_index).copied() else {
            return Err(malformed(GSUB_TAG));
        };
        if apply_gsub_lookup_at(
            table,
            face,
            metrics,
            glyphs,
            gdef,
            gsub,
            target_index,
            lookup_index,
            context_depth,
        )? {
            changed = true;
        }
    }
    Ok(changed)
}

fn apply_gsub_lookup_at(
    table: &[u8],
    face: &SfntFace<'_>,
    metrics: FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: &LayoutTableState,
    target_index: usize,
    lookup_index: u16,
    context_depth: u8,
) -> Result<bool, SfntError> {
    if context_depth >= MAX_CONTEXT_DEPTH {
        return Err(malformed(GSUB_TAG));
    }
    let lookup = gsub.lookup(lookup_index)?;
    if gdef.ignores(glyphs[target_index].glyph_id, lookup.lookup_flags)? {
        return Ok(false);
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
            target_index,
            0,
        )? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(glyph_id: u16) -> LayoutGlyph {
        LayoutGlyph::from_glyph_id(glyph_id, 0, 0)
    }

    #[test]
    fn context_prefilter_checks_first_coverage_for_format3() {
        // Contextual substitution format 3: one input coverage and no
        // substitutions. The zero substitution count keeps this fixture
        // focused on the prefilter's first-input decision.
        let subtable = [
            0, 3, // format
            0, 1, // glyph count
            0, 0, // substitution count
            0, 8, // input coverage offset
            0, 1, // coverage format
            0, 1, // coverage glyph count
            0, 42, // covered glyph
        ];
        let matching = [glyph(42)];
        let non_matching = [glyph(43)];

        assert_eq!(
            context_subtable_may_match(
                &subtable,
                5,
                &matching.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
                0
            )
            .expect("valid contextual subtable"),
            Some(true)
        );
        assert_eq!(
            context_subtable_may_match(
                &subtable,
                5,
                &non_matching.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
                0,
            )
            .expect("valid contextual subtable"),
            Some(false)
        );
    }

    #[test]
    fn context_prefilter_checks_chain_format3_input_coverage() {
        let subtable = [
            0, 3, // format
            0, 0, // backtrack count
            0, 1, // input count
            0, 12, // first input coverage offset
            0, 0, // lookahead count
            0, 0, // substitution count
            0, 1, // coverage format
            0, 1, // coverage glyph count
            0, 99, // covered glyph
        ];
        let candidates = [99];
        assert_eq!(
            context_subtable_may_match(&subtable, 6, &candidates, 0)
                .expect("valid chain contextual subtable"),
            Some(true)
        );
    }

    #[test]
    fn context_prefilter_rejects_malformed_format3() {
        let malformed_subtable = [0, 3, 0, 0, 0, 0];
        assert!(context_subtable_may_match(&malformed_subtable, 5, &[], 0).is_err());
    }
}
