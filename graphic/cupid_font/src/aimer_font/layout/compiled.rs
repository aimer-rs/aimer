//! Pre-decoded GSUB/GPOS data used by the hot shaping loops.

use super::super::SfntError;
use super::super::Tag;
use super::{
    ClassDef, GPOS_TAG, GSUB_TAG, Gdef, LayoutGlyph, MAX_EXTENSION_DEPTH, ValueAdjustment,
    checked_add, checked_mul, ensure, malformed, read_i16, read_u16, read_u32, relative_offset,
    slice_from,
};

#[derive(Clone)]
pub(super) enum CompiledSubtable {
    Single(SinglePlan),
    Ligature(LigaturePlan),
    Pair(PairPlan),
    Cursive(CursivePlan),
    MarkToBase(MarkPlan),
    MarkToMark(MarkPlan),
}

#[derive(Clone)]
enum CoveragePlan {
    Format1(Vec<u16>),
    Format2(Vec<CoverageRange>),
}

#[derive(Clone, Copy)]
struct CoverageRange {
    start: u16,
    end: u16,
    base: usize,
}

#[derive(Clone)]
pub(super) struct SinglePlan {
    coverage: CoveragePlan,
    replacement: SingleReplacement,
}

#[derive(Clone)]
pub(super) enum SingleReplacement {
    Delta(i16),
    Values(Vec<u16>),
}

#[derive(Clone)]
pub(super) struct LigaturePlan {
    coverage: CoveragePlan,
    sets: Vec<Vec<LigatureRule>>,
}

#[derive(Clone)]
pub(super) struct LigatureRule {
    glyph_id: u16,
    components: Vec<u16>,
}

#[derive(Clone)]
pub(super) struct PairPlan {
    coverage: CoveragePlan,
    kind: PairKind,
}

#[derive(Clone)]
pub(super) enum PairKind {
    Format1 {
        sets: Vec<Vec<PairRecord>>,
    },
    Format2 {
        class_1: ClassDef,
        class_2: ClassDef,
        class_2_count: usize,
        records: Vec<(ValueAdjustment, ValueAdjustment)>,
    },
}

#[derive(Clone, Copy)]
pub(super) struct PairRecord {
    second_glyph: u16,
    first: ValueAdjustment,
    second: ValueAdjustment,
}

#[derive(Clone)]
pub(super) struct CursivePlan {
    coverage: CoveragePlan,
    records: Vec<(Option<(i32, i32)>, Option<(i32, i32)>)>,
}

#[derive(Clone)]
pub(super) struct MarkPlan {
    mark_coverage: CoveragePlan,
    base_coverage: CoveragePlan,
    class_count: usize,
    mark_records: Vec<(usize, Option<(i32, i32)>)>,
    base_records: Vec<Vec<Option<(i32, i32)>>>,
}

pub(super) fn compile_subtable(
    table: &[u8],
    table_tag: Tag,
    lookup_type: u16,
    offset: usize,
) -> Result<Option<CompiledSubtable>, SfntError> {
    let subtable = slice_from(table, offset, table_tag)?;
    compile_subtable_bytes(subtable, table_tag, lookup_type, 0)
}

fn compile_subtable_bytes(
    subtable: &[u8],
    table_tag: Tag,
    lookup_type: u16,
    extension_depth: u8,
) -> Result<Option<CompiledSubtable>, SfntError> {
    if table_tag == GSUB_TAG {
        match lookup_type {
            1 => Ok(Some(CompiledSubtable::Single(parse_single(subtable)?))),
            4 => Ok(Some(CompiledSubtable::Ligature(parse_ligature(subtable)?))),
            7 => {
                if extension_depth >= MAX_EXTENSION_DEPTH {
                    return Err(malformed(GSUB_TAG));
                }
                if read_u16(subtable, 0, GSUB_TAG)? != 1 {
                    return Ok(None);
                }
                let extension_type = read_u16(subtable, 2, GSUB_TAG)?;
                let extension_offset = usize::try_from(read_u32(subtable, 4, GSUB_TAG)?)
                    .map_err(|_| SfntError::ArithmeticOverflow)?;
                let extension = slice_from(subtable, extension_offset, GSUB_TAG)?;
                compile_subtable_bytes(extension, GSUB_TAG, extension_type, extension_depth + 1)
            }
            _ => Ok(None),
        }
    } else if table_tag == GPOS_TAG {
        match lookup_type {
            2 => Ok(Some(CompiledSubtable::Pair(parse_pair(subtable)?))),
            3 => Ok(Some(CompiledSubtable::Cursive(parse_cursive(subtable)?))),
            4 => Ok(Some(CompiledSubtable::MarkToBase(parse_mark(subtable)?))),
            // The existing Arabic fixture uses lookup type 5 for a
            // mark-to-mark record, while OpenType uses type 6 for
            // mark-to-mark. Keep both checked paths until the old fixture is
            // retired.
            5 | 6 => Ok(Some(CompiledSubtable::MarkToMark(parse_mark(subtable)?))),
            9 => {
                if extension_depth >= MAX_EXTENSION_DEPTH {
                    return Err(malformed(GPOS_TAG));
                }
                if read_u16(subtable, 0, GPOS_TAG)? != 1 {
                    return Ok(None);
                }
                let extension_type = read_u16(subtable, 2, GPOS_TAG)?;
                let extension_offset = usize::try_from(read_u32(subtable, 4, GPOS_TAG)?)
                    .map_err(|_| SfntError::ArithmeticOverflow)?;
                let extension = slice_from(subtable, extension_offset, GPOS_TAG)?;
                compile_subtable_bytes(extension, GPOS_TAG, extension_type, extension_depth + 1)
            }
            _ => Ok(None),
        }
    } else {
        Ok(None)
    }
}

impl CoveragePlan {
    fn parse(table: &[u8], offset: usize, tag: Tag) -> Result<Self, SfntError> {
        let coverage = slice_from(table, offset, tag)?;
        match read_u16(coverage, 0, tag)? {
            1 => {
                let count = usize::from(read_u16(coverage, 2, tag)?);
                ensure(coverage, 4, checked_mul(count, 2)?, tag)?;
                let mut glyphs = Vec::with_capacity(count);
                let mut previous = None;
                for index in 0..count {
                    let glyph_id = read_u16(coverage, 4 + index * 2, tag)?;
                    if previous.is_some_and(|value| glyph_id <= value) {
                        return Err(malformed(tag));
                    }
                    previous = Some(glyph_id);
                    glyphs.push(glyph_id);
                }
                Ok(Self::Format1(glyphs))
            }
            2 => {
                let count = usize::from(read_u16(coverage, 2, tag)?);
                ensure(coverage, 4, checked_mul(count, 6)?, tag)?;
                let mut ranges = Vec::with_capacity(count);
                let mut previous_end = None;
                for index in 0..count {
                    let record = 4 + index * 6;
                    let start = read_u16(coverage, record, tag)?;
                    let end = read_u16(coverage, record + 2, tag)?;
                    if start > end || previous_end.is_some_and(|value| start <= value) {
                        return Err(malformed(tag));
                    }
                    let base = usize::from(read_u16(coverage, record + 4, tag)?);
                    ranges.push(CoverageRange { start, end, base });
                    previous_end = Some(end);
                }
                Ok(Self::Format2(ranges))
            }
            _ => Err(malformed(tag)),
        }
    }

    fn index(&self, glyph_id: u16) -> Option<usize> {
        match self {
            Self::Format1(glyphs) => glyphs.binary_search(&glyph_id).ok(),
            Self::Format2(ranges) => {
                let mut low = 0;
                let mut high = ranges.len();
                while low < high {
                    let index = low + (high - low) / 2;
                    let range = ranges[index];
                    if glyph_id < range.start {
                        high = index;
                    } else if glyph_id > range.end {
                        low = index + 1;
                    } else {
                        return Some(range.base + usize::from(glyph_id - range.start));
                    }
                }
                None
            }
        }
    }
}

fn parse_single(subtable: &[u8]) -> Result<SinglePlan, SfntError> {
    let format = read_u16(subtable, 0, GSUB_TAG)?;
    let coverage = CoveragePlan::parse(
        subtable,
        usize::from(read_u16(subtable, 2, GSUB_TAG)?),
        GSUB_TAG,
    )?;
    let replacement = match format {
        1 => SingleReplacement::Delta(read_i16(subtable, 4, GSUB_TAG)?),
        2 => {
            let count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
            ensure(subtable, 6, checked_mul(count, 2)?, GSUB_TAG)?;
            let mut values = Vec::with_capacity(count);
            for index in 0..count {
                values.push(read_u16(subtable, 6 + index * 2, GSUB_TAG)?);
            }
            SingleReplacement::Values(values)
        }
        _ => return Err(malformed(GSUB_TAG)),
    };
    Ok(SinglePlan {
        coverage,
        replacement,
    })
}

fn parse_ligature(subtable: &[u8]) -> Result<LigaturePlan, SfntError> {
    if read_u16(subtable, 0, GSUB_TAG)? != 1 {
        return Err(malformed(GSUB_TAG));
    }
    let coverage = CoveragePlan::parse(
        subtable,
        usize::from(read_u16(subtable, 2, GSUB_TAG)?),
        GSUB_TAG,
    )?;
    let set_count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
    ensure(subtable, 6, checked_mul(set_count, 2)?, GSUB_TAG)?;
    let mut sets = Vec::with_capacity(set_count);
    for set_index in 0..set_count {
        let set_offset = relative_offset(
            subtable,
            0,
            read_u16(subtable, 6 + set_index * 2, GSUB_TAG)?,
            GSUB_TAG,
        )?;
        let count = usize::from(read_u16(subtable, set_offset, GSUB_TAG)?);
        ensure(
            subtable,
            checked_add(set_offset, 2)?,
            checked_mul(count, 2)?,
            GSUB_TAG,
        )?;
        let mut rules = Vec::with_capacity(count);
        for rule_index in 0..count {
            let offset = checked_add(set_offset, 2 + rule_index * 2)?;
            let ligature_offset = relative_offset(
                subtable,
                set_offset,
                read_u16(subtable, offset, GSUB_TAG)?,
                GSUB_TAG,
            )?;
            let glyph_id = read_u16(subtable, ligature_offset, GSUB_TAG)?;
            let component_count = usize::from(read_u16(
                subtable,
                checked_add(ligature_offset, 2)?,
                GSUB_TAG,
            )?);
            // Some variable fonts contain one-component ligature records as
            // a compact single-glyph substitution. Treat those as valid
            // replacement rules; zero components is still malformed.
            if component_count == 0 {
                return Err(malformed(GSUB_TAG));
            }
            ensure(
                subtable,
                checked_add(ligature_offset, 4)?,
                checked_mul(component_count - 1, 2)?,
                GSUB_TAG,
            )?;
            let mut components = Vec::with_capacity(component_count - 1);
            for component_index in 1..component_count {
                components.push(read_u16(
                    subtable,
                    ligature_offset + 2 + component_index * 2,
                    GSUB_TAG,
                )?);
            }
            rules.push(LigatureRule {
                glyph_id,
                components,
            });
        }
        sets.push(rules);
    }
    Ok(LigaturePlan { coverage, sets })
}

fn parse_pair(subtable: &[u8]) -> Result<PairPlan, SfntError> {
    let format = read_u16(subtable, 0, GPOS_TAG)?;
    let coverage = CoveragePlan::parse(
        subtable,
        usize::from(read_u16(subtable, 2, GPOS_TAG)?),
        GPOS_TAG,
    )?;
    let value_format_1 = read_u16(subtable, 4, GPOS_TAG)?;
    let value_format_2 = read_u16(subtable, 6, GPOS_TAG)?;
    let value_size_1 = super::value_record_size(value_format_1, GPOS_TAG)?;
    let value_size_2 = super::value_record_size(value_format_2, GPOS_TAG)?;
    let kind = match format {
        1 => {
            let set_count = usize::from(read_u16(subtable, 8, GPOS_TAG)?);
            ensure(subtable, 10, checked_mul(set_count, 2)?, GPOS_TAG)?;
            let record_size = checked_add(2, checked_add(value_size_1, value_size_2)?)?;
            let mut sets = Vec::with_capacity(set_count);
            for set_index in 0..set_count {
                let set_offset = relative_offset(
                    subtable,
                    0,
                    read_u16(subtable, 10 + set_index * 2, GPOS_TAG)?,
                    GPOS_TAG,
                )?;
                let pair_count = usize::from(read_u16(subtable, set_offset, GPOS_TAG)?);
                ensure(
                    subtable,
                    checked_add(set_offset, 2)?,
                    checked_mul(pair_count, record_size)?,
                    GPOS_TAG,
                )?;
                let mut records = Vec::with_capacity(pair_count);
                for index in 0..pair_count {
                    let record = set_offset + 2 + index * record_size;
                    let second_glyph = read_u16(subtable, record, GPOS_TAG)?;
                    let (first, first_size) = super::read_value_adjustment(
                        subtable,
                        record + 2,
                        value_format_1,
                        GPOS_TAG,
                    )?;
                    let (second, _) = super::read_value_adjustment(
                        subtable,
                        record + 2 + first_size,
                        value_format_2,
                        GPOS_TAG,
                    )?;
                    records.push(PairRecord {
                        second_glyph,
                        first,
                        second,
                    });
                }
                sets.push(records);
            }
            PairKind::Format1 { sets }
        }
        2 => {
            let class_1_offset = usize::from(read_u16(subtable, 8, GPOS_TAG)?);
            let class_2_offset = usize::from(read_u16(subtable, 10, GPOS_TAG)?);
            let class_1_count = usize::from(read_u16(subtable, 12, GPOS_TAG)?);
            let class_2_count = usize::from(read_u16(subtable, 14, GPOS_TAG)?);
            if class_1_count == 0 || class_2_count == 0 {
                return Err(malformed(GPOS_TAG));
            }
            let class_1 = ClassDef::new(subtable, class_1_offset, GPOS_TAG)?
                .ok_or_else(|| malformed(GPOS_TAG))?;
            let class_2 = ClassDef::new(subtable, class_2_offset, GPOS_TAG)?
                .ok_or_else(|| malformed(GPOS_TAG))?;
            let record_size = checked_add(value_size_1, value_size_2)?;
            let record_count = checked_mul(class_1_count, class_2_count)?;
            ensure(
                subtable,
                16,
                checked_mul(record_count, record_size)?,
                GPOS_TAG,
            )?;
            let mut records = Vec::with_capacity(record_count);
            for index in 0..record_count {
                let record = 16 + index * record_size;
                let (first, first_size) =
                    super::read_value_adjustment(subtable, record, value_format_1, GPOS_TAG)?;
                let (second, _) = super::read_value_adjustment(
                    subtable,
                    record + first_size,
                    value_format_2,
                    GPOS_TAG,
                )?;
                records.push((first, second));
            }
            PairKind::Format2 {
                class_1,
                class_2,
                class_2_count,
                records,
            }
        }
        _ => return Err(malformed(GPOS_TAG)),
    };
    Ok(PairPlan { coverage, kind })
}

fn parse_cursive(subtable: &[u8]) -> Result<CursivePlan, SfntError> {
    if read_u16(subtable, 0, GPOS_TAG)? != 1 {
        return Err(malformed(GPOS_TAG));
    }
    let coverage = CoveragePlan::parse(
        subtable,
        usize::from(read_u16(subtable, 2, GPOS_TAG)?),
        GPOS_TAG,
    )?;
    let count = usize::from(read_u16(subtable, 4, GPOS_TAG)?);
    ensure(subtable, 6, checked_mul(count, 4)?, GPOS_TAG)?;
    let mut records = Vec::with_capacity(count);
    for index in 0..count {
        let record = 6 + index * 4;
        let entry_offset = usize::from(read_u16(subtable, record, GPOS_TAG)?);
        let exit_offset = usize::from(read_u16(subtable, record + 2, GPOS_TAG)?);
        records.push((
            super::anchor_position(subtable, 0, entry_offset)?,
            super::anchor_position(subtable, 0, exit_offset)?,
        ));
    }
    Ok(CursivePlan { coverage, records })
}

fn parse_mark(subtable: &[u8]) -> Result<MarkPlan, SfntError> {
    if read_u16(subtable, 0, GPOS_TAG)? != 1 {
        return Err(malformed(GPOS_TAG));
    }
    let mark_coverage = CoveragePlan::parse(
        subtable,
        usize::from(read_u16(subtable, 2, GPOS_TAG)?),
        GPOS_TAG,
    )?;
    let base_coverage = CoveragePlan::parse(
        subtable,
        usize::from(read_u16(subtable, 4, GPOS_TAG)?),
        GPOS_TAG,
    )?;
    let class_count = usize::from(read_u16(subtable, 6, GPOS_TAG)?);
    if class_count == 0 {
        return Err(malformed(GPOS_TAG));
    }
    let mark_array_offset = usize::from(read_u16(subtable, 8, GPOS_TAG)?);
    let base_array_offset = usize::from(read_u16(subtable, 10, GPOS_TAG)?);
    let mark_count = usize::from(read_u16(subtable, mark_array_offset, GPOS_TAG)?);
    ensure(
        subtable,
        checked_add(mark_array_offset, 2)?,
        checked_mul(mark_count, 4)?,
        GPOS_TAG,
    )?;
    let mut mark_records = Vec::with_capacity(mark_count);
    for index in 0..mark_count {
        let record = mark_array_offset + 2 + index * 4;
        let class = usize::from(read_u16(subtable, record, GPOS_TAG)?);
        if class >= class_count {
            return Err(malformed(GPOS_TAG));
        }
        let anchor_offset = usize::from(read_u16(subtable, record + 2, GPOS_TAG)?);
        mark_records.push((
            class,
            super::anchor_position(subtable, mark_array_offset, anchor_offset)?,
        ));
    }

    let base_count = usize::from(read_u16(subtable, base_array_offset, GPOS_TAG)?);
    let base_record_size = checked_mul(class_count, 2)?;
    ensure(
        subtable,
        checked_add(base_array_offset, 2)?,
        checked_mul(base_count, base_record_size)?,
        GPOS_TAG,
    )?;
    let mut base_records = Vec::with_capacity(base_count);
    for index in 0..base_count {
        let record = base_array_offset + 2 + index * base_record_size;
        let mut anchors = Vec::with_capacity(class_count);
        for class in 0..class_count {
            let anchor_offset = usize::from(read_u16(subtable, record + class * 2, GPOS_TAG)?);
            anchors.push(super::anchor_position(
                subtable,
                base_array_offset,
                anchor_offset,
            )?);
        }
        base_records.push(anchors);
    }
    Ok(MarkPlan {
        mark_coverage,
        base_coverage,
        class_count,
        mark_records,
        base_records,
    })
}

impl CompiledSubtable {
    pub(super) fn is_ligature(&self) -> bool {
        matches!(self, Self::Ligature(_))
    }

    pub(super) fn apply_single_at(
        &self,
        advances: &[u16],
        glyph: &mut LayoutGlyph,
    ) -> Result<Option<bool>, SfntError> {
        let Self::Single(plan) = self else {
            return Ok(None);
        };
        let Some(index) = plan.coverage.index(glyph.glyph_id) else {
            return Ok(Some(false));
        };
        let replacement = match &plan.replacement {
            SingleReplacement::Delta(delta) => glyph.glyph_id.wrapping_add(*delta as u16),
            SingleReplacement::Values(values) => {
                *values.get(index).ok_or_else(|| malformed(GSUB_TAG))?
            }
        };
        if replacement == glyph.glyph_id {
            return Ok(Some(false));
        }
        let Some(advance) = advances.get(usize::from(replacement)).copied() else {
            return Err(malformed(GSUB_TAG));
        };
        glyph.glyph_id = replacement;
        glyph.x_advance = i32::from(advance);
        glyph.y_advance = 0;
        glyph.x_offset = 0;
        glyph.y_offset = 0;
        Ok(Some(true))
    }

    pub(super) fn apply_ligature(
        &self,
        advances: &[u16],
        glyphs: &mut Vec<LayoutGlyph>,
        gdef: &Gdef,
        lookup_flags: u16,
    ) -> Result<Option<()>, SfntError> {
        let Self::Ligature(plan) = self else {
            return Ok(None);
        };
        let mut index = 0;
        while index < glyphs.len() {
            if lookup_flags != 0 && gdef.ignores(glyphs[index].glyph_id, lookup_flags)? {
                index += 1;
                continue;
            }
            let Some(set_index) = plan.coverage.index(glyphs[index].glyph_id) else {
                index += 1;
                continue;
            };
            let rules = plan
                .sets
                .get(set_index)
                .ok_or_else(|| malformed(GSUB_TAG))?;
            let mut best = None;
            for rule in rules {
                if rule.components.len() > glyphs.len() - index - 1 {
                    continue;
                }
                let mut matches = true;
                for (offset, expected) in rule.components.iter().enumerate() {
                    let candidate = index + offset + 1;
                    if (lookup_flags != 0
                        && gdef.ignores(glyphs[candidate].glyph_id, lookup_flags)?)
                        || glyphs[candidate].glyph_id != *expected
                    {
                        matches = false;
                        break;
                    }
                }
                if matches
                    && best
                        .as_ref()
                        .is_none_or(|best: &(u16, usize)| rule.components.len() + 1 > best.1)
                {
                    best = Some((rule.glyph_id, rule.components.len() + 1));
                }
            }
            let Some((ligature_glyph, component_count)) = best else {
                index += 1;
                continue;
            };
            let Some(advance) = advances.get(usize::from(ligature_glyph)).copied() else {
                return Err(malformed(GSUB_TAG));
            };
            let cluster = glyphs[index].cluster;
            glyphs[index] = LayoutGlyph::from_glyph_id(ligature_glyph, cluster, advance);
            glyphs.drain(index + 1..index + component_count);
            if component_count == 1 {
                index += 1;
            }
        }
        Ok(Some(()))
    }

    pub(super) fn is_pair(&self) -> bool {
        matches!(self, Self::Pair(_))
    }

    pub(super) fn is_cursive(&self) -> bool {
        matches!(self, Self::Cursive(_))
    }

    pub(super) fn is_mark_to_mark(&self) -> bool {
        matches!(self, Self::MarkToMark(_))
    }

    pub(super) fn pair_adjustment(
        &self,
        first_glyph: u16,
        second_glyph: u16,
    ) -> Result<Option<(ValueAdjustment, ValueAdjustment)>, SfntError> {
        let Self::Pair(plan) = self else {
            return Ok(None);
        };
        let Some(first_index) = plan.coverage.index(first_glyph) else {
            return Ok(None);
        };
        let pair = match &plan.kind {
            PairKind::Format1 { sets } => sets
                .get(first_index)
                .ok_or_else(|| malformed(GPOS_TAG))?
                .binary_search_by_key(&second_glyph, |record| record.second_glyph)
                .ok()
                .map(|index| {
                    let record = sets[first_index][index];
                    (record.first, record.second)
                }),
            PairKind::Format2 {
                class_1,
                class_2,
                class_2_count,
                records,
            } => {
                let class_1 = usize::from(class_1.class(first_glyph)?);
                let class_2 = usize::from(class_2.class(second_glyph)?);
                let record_index = class_1
                    .checked_mul(*class_2_count)
                    .and_then(|base| base.checked_add(class_2));
                record_index.and_then(|index| records.get(index).copied())
            }
        };
        Ok(pair.filter(|(first, second)| !first.is_zero() || !second.is_zero()))
    }

    pub(super) fn cursive_adjustment(
        &self,
        current_glyph: u16,
        previous_glyph: u16,
    ) -> Result<Option<(i32, i32)>, SfntError> {
        let Self::Cursive(plan) = self else {
            return Ok(None);
        };
        let Some(current_index) = plan.coverage.index(current_glyph) else {
            return Ok(None);
        };
        let Some(previous_index) = plan.coverage.index(previous_glyph) else {
            return Ok(None);
        };
        let (current_entry, _) = plan
            .records
            .get(current_index)
            .ok_or_else(|| malformed(GPOS_TAG))?;
        let (_, previous_exit) = plan
            .records
            .get(previous_index)
            .ok_or_else(|| malformed(GPOS_TAG))?;
        let (Some((current_x, current_y)), Some((previous_x, previous_y))) =
            (current_entry, previous_exit)
        else {
            return Ok(None);
        };
        Ok(Some((previous_x - current_x, previous_y - current_y)))
    }

    pub(super) fn mark_adjustment(
        &self,
        mark_glyph: u16,
        base_glyph: u16,
    ) -> Result<Option<(i32, i32)>, SfntError> {
        let plan = match self {
            Self::MarkToBase(plan) | Self::MarkToMark(plan) => plan,
            _ => return Ok(None),
        };
        let Some(mark_index) = plan.mark_coverage.index(mark_glyph) else {
            return Ok(None);
        };
        let Some(base_index) = plan.base_coverage.index(base_glyph) else {
            return Ok(None);
        };
        let (mark_class, mark_anchor) = plan
            .mark_records
            .get(mark_index)
            .ok_or_else(|| malformed(GPOS_TAG))?;
        if *mark_class >= plan.class_count {
            return Err(malformed(GPOS_TAG));
        }
        let base_anchor = plan
            .base_records
            .get(base_index)
            .and_then(|anchors| anchors.get(*mark_class))
            .ok_or_else(|| malformed(GPOS_TAG))?;
        let (Some((mark_x, mark_y)), Some((base_x, base_y))) = (mark_anchor, base_anchor) else {
            return Ok(None);
        };
        Ok(Some((base_x - mark_x, base_y - mark_y)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_component_ligature_replacement_advances_the_cursor() {
        let plan = CompiledSubtable::Ligature(LigaturePlan {
            coverage: CoveragePlan::Format1(vec![10]),
            sets: vec![vec![LigatureRule {
                glyph_id: 20,
                components: Vec::new(),
            }]],
        });
        let mut glyphs = vec![LayoutGlyph::from_glyph_id(10, 0, 500)];
        let advances = vec![500; 21];

        plan.apply_ligature(&advances, &mut glyphs, &Gdef { glyph_class: None }, 0)
            .expect("the one-component replacement should be executable");

        assert_eq!(glyphs.len(), 1);
        assert_eq!(glyphs[0].glyph_id, 20);
    }
}
