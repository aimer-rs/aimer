//! The first Aimer-owned OpenType layout slice.
//!
//! This module deliberately exposes bounded, checked script slices:
//! default Latin ligatures, Arabic joining forms/positioning, CJK vertical
//! features, Indic reordering/substitution/mark positioning, and bounded
//! Southeast Asian reordering/substitution/mark positioning. The parser
//! consumes the same checked SFNT view as the outline rasterizer. When a
//! script-specific feature is absent or unsupported, the checked cmap/hmtx
//! path still returns a complete glyph run instead of handing the font to a
//! second shaping engine.

use std::cell::RefCell;

use super::{Reader, SfntError, SfntFace, Tag};
use crate::font::TextLanguage;
use crate::text_pipeline::glyph_rasterizer::NORMAL_GLYPH_WEIGHT;
use hashbrown::HashMap;

use crate::pipeline::text_pipeline::unicode_script::Script;

mod compiled;
mod context;
mod indic;
mod southeast_asian;

use compiled::CompiledSubtable;

const GDEF_TAG: Tag = Tag(*b"GDEF");
const GSUB_TAG: Tag = Tag(*b"GSUB");
const GPOS_TAG: Tag = Tag(*b"GPOS");
const DFLT_TAG: Tag = Tag(*b"DFLT");
const LATN_TAG: Tag = Tag(*b"latn");
const ARAB_TAG: Tag = Tag(*b"arab");
const HANI_TAG: Tag = Tag(*b"hani");
const DFLT_LANG_TAG: Tag = Tag(*b"dflt");
const ZHS_TAG: Tag = Tag(*b"ZHS ");
const JAN_TAG: Tag = Tag(*b"JAN ");
const KOR_TAG: Tag = Tag(*b"KOR ");
const LIGA_TAG: Tag = Tag(*b"liga");
const CLIG_TAG: Tag = Tag(*b"clig");
const KERN_TAG: Tag = Tag(*b"kern");
const CALT_TAG: Tag = Tag(*b"calt");
const LOCL_TAG: Tag = Tag(*b"locl");
const VRT2_TAG: Tag = Tag(*b"vrt2");
const VERT_TAG: Tag = Tag(*b"vert");
const VPAL_TAG: Tag = Tag(*b"vpal");
const VKRN_TAG: Tag = Tag(*b"vkrn");
const ISOL_TAG: Tag = Tag(*b"isol");
const INIT_TAG: Tag = Tag(*b"init");
const MEDI_TAG: Tag = Tag(*b"medi");
const FINA_TAG: Tag = Tag(*b"fina");
const MARK_TAG: Tag = Tag(*b"mark");
const MKMK_TAG: Tag = Tag(*b"mkmk");
const CURS_TAG: Tag = Tag(*b"curs");
const RLIG_TAG: Tag = Tag(*b"rlig");
const DEVA_TAG: Tag = Tag(*b"deva");
const DEVA2_TAG: Tag = Tag(*b"dev2");
const BENG_TAG: Tag = Tag(*b"beng");
const BENG2_TAG: Tag = Tag(*b"bng2");
const GURU_TAG: Tag = Tag(*b"guru");
const GURU2_TAG: Tag = Tag(*b"gur2");
const GUJR_TAG: Tag = Tag(*b"gujr");
const GUJR2_TAG: Tag = Tag(*b"gjr2");
const ORYA_TAG: Tag = Tag(*b"orya");
const ORYA2_TAG: Tag = Tag(*b"ory2");
const TAML_TAG: Tag = Tag(*b"taml");
const TAML2_TAG: Tag = Tag(*b"tml2");
const TELU_TAG: Tag = Tag(*b"telu");
const TELU2_TAG: Tag = Tag(*b"tel2");
const KNDA_TAG: Tag = Tag(*b"knda");
const KNDA2_TAG: Tag = Tag(*b"knd2");
const MLYM_TAG: Tag = Tag(*b"mlym");
const MLYM2_TAG: Tag = Tag(*b"mlm2");
const SINH_TAG: Tag = Tag(*b"sinh");
const THAI_TAG: Tag = Tag(*b"thai");
const LAOO_TAG: Tag = Tag(*b"lao ");
const KHMR_TAG: Tag = Tag(*b"khmr");
const MYM2_TAG: Tag = Tag(*b"mym2");
const MYMR_TAG: Tag = Tag(*b"mymr");
const CCMP_TAG: Tag = Tag(*b"ccmp");
const NUKT_TAG: Tag = Tag(*b"nukt");
const AKHN_TAG: Tag = Tag(*b"akhn");
const RPHF_TAG: Tag = Tag(*b"rphf");
const RKRF_TAG: Tag = Tag(*b"rkrf");
const PREF_TAG: Tag = Tag(*b"pref");
const BLWF_TAG: Tag = Tag(*b"blwf");
const ABVF_TAG: Tag = Tag(*b"abvf");
const HALF_TAG: Tag = Tag(*b"half");
const PSTF_TAG: Tag = Tag(*b"pstf");
const VATU_TAG: Tag = Tag(*b"vatu");
const CJCT_TAG: Tag = Tag(*b"cjct");
const PRES_TAG: Tag = Tag(*b"pres");
const ABVS_TAG: Tag = Tag(*b"abvs");
const BLWS_TAG: Tag = Tag(*b"blws");
const PSTS_TAG: Tag = Tag(*b"psts");
const HALN_TAG: Tag = Tag(*b"haln");
const RCLT_TAG: Tag = Tag(*b"rclt");
const ABVM_TAG: Tag = Tag(*b"abvm");
const BLWM_TAG: Tag = Tag(*b"blwm");
const DIST_TAG: Tag = Tag(*b"dist");
// Extension lookups are allowed to nest only through malformed data in this
// subset; keep that data from turning a bounded parse into unbounded recursion.
const MAX_EXTENSION_DEPTH: u8 = 8;
const MAX_CONTEXT_DEPTH: u8 = 8;

const INDIC_SCRIPT_TAGS: &[Tag] = &[
    DEVA2_TAG, DEVA_TAG, BENG2_TAG, BENG_TAG, GURU2_TAG, GURU_TAG, GUJR2_TAG, GUJR_TAG, ORYA2_TAG,
    ORYA_TAG, TAML2_TAG, TAML_TAG, TELU2_TAG, TELU_TAG, KNDA2_TAG, KNDA_TAG, MLYM2_TAG, MLYM_TAG,
    SINH_TAG,
];
const INDIC_GSUB_FEATURE_TAGS: &[Tag] = &[
    LOCL_TAG, CCMP_TAG, NUKT_TAG, AKHN_TAG, RPHF_TAG, RKRF_TAG, PREF_TAG, BLWF_TAG, ABVF_TAG,
    HALF_TAG, PSTF_TAG, VATU_TAG, CJCT_TAG, PRES_TAG, ABVS_TAG, BLWS_TAG, PSTS_TAG, HALN_TAG,
    RCLT_TAG, RLIG_TAG, LIGA_TAG, CLIG_TAG, CALT_TAG,
];
const INDIC_GPOS_FEATURE_TAGS: &[Tag] = &[ABVM_TAG, BLWM_TAG, MARK_TAG, MKMK_TAG, KERN_TAG];
const SOUTHEAST_ASIAN_SCRIPT_TAGS: &[Tag] = &[THAI_TAG, LAOO_TAG, KHMR_TAG, MYM2_TAG, MYMR_TAG];
const SOUTHEAST_ASIAN_GSUB_FEATURE_TAGS: &[Tag] = &[
    LOCL_TAG, CCMP_TAG, RLIG_TAG, LIGA_TAG, CLIG_TAG, CALT_TAG, NUKT_TAG, AKHN_TAG, RPHF_TAG,
    RKRF_TAG, PREF_TAG, BLWF_TAG, ABVF_TAG, HALF_TAG, PSTF_TAG, VATU_TAG, CJCT_TAG, PRES_TAG,
    ABVS_TAG, BLWS_TAG, PSTS_TAG, HALN_TAG, RCLT_TAG,
];
// Khmer uses its own ordered language-form and typographic-form stages.
const KHMER_GSUB_FEATURE_TAGS: &[Tag] = &[
    LOCL_TAG, CCMP_TAG, PREF_TAG, BLWF_TAG, ABVF_TAG, PSTF_TAG, PRES_TAG, BLWS_TAG, ABVS_TAG,
    PSTS_TAG, CLIG_TAG,
];
const MYANMAR_GSUB_FEATURE_TAGS: &[Tag] = &[
    LOCL_TAG, CCMP_TAG, RPHF_TAG, PREF_TAG, BLWF_TAG, PSTF_TAG, PRES_TAG, ABVS_TAG, BLWS_TAG,
    PSTS_TAG,
];
const SOUTHEAST_ASIAN_GPOS_FEATURE_TAGS: &[Tag] =
    &[DIST_TAG, ABVM_TAG, BLWM_TAG, MARK_TAG, MKMK_TAG, KERN_TAG];

/// A glyph produced by the Aimer layout subset, still measured in font units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AimerLayoutGlyph {
    pub(crate) glyph_id: u16,
    pub(crate) cluster: usize,
    pub(crate) x_advance: i32,
    pub(crate) y_advance: i32,
    pub(crate) x_offset: i32,
    pub(crate) y_offset: i32,
}

/// A shaped run before the rasterizer applies the requested pixel scale.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AimerShapedRun {
    pub(crate) units_per_em: u16,
    pub(crate) glyphs: Vec<AimerLayoutGlyph>,
}

/// Reusable temporary storage for one owned shaping call on a thread.
///
/// The owned layout engine keeps a reusable Unicode buffer. Keeping the
/// equivalent temporary storage here avoids allocating the same
/// codepoint, glyph, and Arabic-state vectors for every short mixed-script
/// run. The final glyph vector is returned to this scratch store by the
/// rasterizer after it has copied the scaled output into its run buffer.
#[derive(Default)]
pub(crate) struct LayoutScratch {
    glyphs: Vec<LayoutGlyph>,
    codepoints: Vec<char>,
    source_codepoints: Vec<(usize, char)>,
    items: Vec<(char, LayoutGlyph)>,
    myanmar_replacement: Vec<(char, LayoutGlyph)>,
    joining_types: Vec<Option<ArabicJoiningType>>,
    transparent: Vec<bool>,
    forms: Vec<Option<ArabicJoiningForm>>,
    adjustments: Vec<(i32, i32, i32, i32)>,
    context_candidates: Vec<u16>,
}

thread_local! {
    static LAYOUT_SCRATCH: RefCell<LayoutScratch> = RefCell::new(LayoutScratch::default());
}

pub(crate) fn recycle_shaped_glyphs(glyphs: Vec<AimerLayoutGlyph>) {
    LAYOUT_SCRATCH.with(|scratch| {
        scratch.borrow_mut().glyphs = glyphs;
    });
}

/// Returns the first source codepoint associated with a shaped cluster.
///
/// Cluster values are byte offsets and the source map is kept in the same
/// order as the pre-substitution glyph sequence. A lower-bound search keeps
/// the post-GSUB codepoint reconstruction linearithmic without cloning the
/// whole source item list or scanning it once for every output glyph.
#[inline]
pub(super) fn source_codepoint_for_cluster(
    source_codepoints: &[(usize, char)],
    cluster: usize,
) -> char {
    let mut low = 0;
    let mut high = source_codepoints.len();
    while low < high {
        let middle = low + (high - low) / 2;
        if source_codepoints[middle].0 < cluster {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    source_codepoints
        .get(low)
        .filter(|(source_cluster, _)| *source_cluster == cluster)
        .map_or('\0', |(_, codepoint)| *codepoint)
}

/// Parsed, face-local OpenType state shared by every run shaped through one
/// cached font face.
///
/// The state keeps the validated GDEF class data and GSUB/GPOS lookup
/// topology. The table bytes remain owned by [`SfntFace`], so the cache stores
/// offsets and lookup metadata instead of copying large layout tables.
#[derive(Clone)]
pub(crate) struct LayoutState {
    gdef: Gdef,
    gsub: Option<LayoutTableState>,
    gpos: Option<LayoutTableState>,
}

#[derive(Clone)]
struct LayoutTableState {
    tag: Tag,
    lookups: Vec<LookupState>,
    feature_lookups: Vec<FeatureLookupState>,
    feature_lookup_map: HashMap<FeatureQueryKey, usize>,
}

#[derive(Clone)]
struct LookupState {
    lookup_type: u16,
    execution_type: u16,
    lookup_flags: u16,
    subtable_offsets: Vec<usize>,
    compiled_subtables: Vec<Option<CompiledSubtable>>,
    compiled_ligature_only: bool,
    compiled_pair_only: bool,
}

#[derive(Clone)]
struct FeatureLookupState {
    script_tag: Tag,
    language_tag: Option<Tag>,
    wanted: Vec<Tag>,
    lookup_indices: Vec<u16>,
    lookups: Vec<LookupState>,
}

/// Compact key for the feature combinations used by the bounded shaping
/// plans. Runtime shaping only asks for one feature or one of the two-feature
/// ligature sets, so keeping the key inline avoids allocating a temporary
/// feature vector for every lookup query.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FeatureQueryKey {
    script_tag: Tag,
    language_tag: Option<Tag>,
    wanted_len: u8,
    wanted: [Tag; 2],
}

impl FeatureQueryKey {
    fn new(script_tag: Tag, language_tag: Option<Tag>, wanted: &[Tag]) -> Option<Self> {
        if wanted.len() > 2 {
            return None;
        }
        let mut key = Self {
            script_tag,
            language_tag,
            wanted_len: wanted.len() as u8,
            wanted: [Tag([0; 4]); 2],
        };
        key.wanted[..wanted.len()].copy_from_slice(wanted);
        Some(key)
    }
}

impl LayoutState {
    pub(crate) fn parse(face: &SfntFace<'_>) -> Result<Self, SfntError> {
        Ok(Self {
            gdef: Gdef::parse(face.table(*b"GDEF"))?,
            gsub: face
                .table(*b"GSUB")
                .map(|table| LayoutTableState::parse(table, GSUB_TAG))
                .transpose()?,
            gpos: face
                .table(*b"GPOS")
                .map(|table| LayoutTableState::parse(table, GPOS_TAG))
                .transpose()?,
        })
    }
}

impl LayoutTableState {
    fn parse(table: &[u8], tag: Tag) -> Result<Self, SfntError> {
        ensure(table, 0, 10, tag)?;
        let lookup_list_offset = usize::from(read_u16(table, 8, tag)?);
        let lookup_count = usize::from(read_u16(table, lookup_list_offset, tag)?);
        ensure(
            table,
            checked_add(lookup_list_offset, 2)?,
            checked_mul(lookup_count, 2)?,
            tag,
        )?;

        let mut feature_specs = if tag == GSUB_TAG {
            vec![
                (LATN_TAG, None, vec![LIGA_TAG, CLIG_TAG]),
                (ARAB_TAG, None, vec![ISOL_TAG]),
                (ARAB_TAG, None, vec![INIT_TAG]),
                (ARAB_TAG, None, vec![MEDI_TAG]),
                (ARAB_TAG, None, vec![FINA_TAG]),
                (ARAB_TAG, None, vec![RLIG_TAG, LIGA_TAG]),
                (ARAB_TAG, None, vec![CALT_TAG]),
                (HANI_TAG, None, vec![LOCL_TAG]),
                (HANI_TAG, Some(ZHS_TAG), vec![LOCL_TAG]),
                (HANI_TAG, Some(JAN_TAG), vec![LOCL_TAG]),
                (HANI_TAG, Some(KOR_TAG), vec![LOCL_TAG]),
                (HANI_TAG, None, vec![VRT2_TAG]),
                (HANI_TAG, None, vec![VERT_TAG]),
                (HANI_TAG, None, vec![VPAL_TAG]),
            ]
        } else {
            vec![
                (LATN_TAG, None, vec![KERN_TAG]),
                (ARAB_TAG, None, vec![CURS_TAG]),
                (ARAB_TAG, None, vec![MARK_TAG]),
                (ARAB_TAG, None, vec![MKMK_TAG]),
                (HANI_TAG, None, vec![VKRN_TAG]),
            ]
        };
        if tag == GSUB_TAG {
            for script_tag in INDIC_SCRIPT_TAGS {
                for feature_tag in INDIC_GSUB_FEATURE_TAGS {
                    feature_specs.push((*script_tag, None, vec![*feature_tag]));
                }
            }
            for script_tag in SOUTHEAST_ASIAN_SCRIPT_TAGS {
                for feature_tag in SOUTHEAST_ASIAN_GSUB_FEATURE_TAGS {
                    feature_specs.push((*script_tag, None, vec![*feature_tag]));
                }
            }
        } else {
            for script_tag in INDIC_SCRIPT_TAGS {
                for feature_tag in INDIC_GPOS_FEATURE_TAGS {
                    feature_specs.push((*script_tag, None, vec![*feature_tag]));
                }
            }
            for script_tag in SOUTHEAST_ASIAN_SCRIPT_TAGS {
                for feature_tag in SOUTHEAST_ASIAN_GPOS_FEATURE_TAGS {
                    feature_specs.push((*script_tag, None, vec![*feature_tag]));
                }
            }
        }
        let mut feature_specs_with_indices = Vec::with_capacity(feature_specs.len());
        let mut active_lookup_indices = Vec::new();
        for (script_tag, language_tag, wanted) in feature_specs {
            let lookup_indices = feature_lookup_indices_with_language(
                table,
                tag,
                script_tag,
                language_tag,
                &wanted,
            )?;
            if lookup_indices
                .iter()
                .any(|lookup_index| usize::from(*lookup_index) >= lookup_count)
            {
                return Err(malformed(tag));
            }
            active_lookup_indices.extend(lookup_indices.iter().copied());
            feature_specs_with_indices.push((script_tag, language_tag, wanted, lookup_indices));
        }

        let mut lookups = Vec::with_capacity(lookup_count);
        for lookup_index in 0..lookup_count {
            let lookup_offset = relative_offset(
                table,
                lookup_list_offset,
                read_u16(
                    table,
                    checked_add(
                        checked_add(lookup_list_offset, 2)?,
                        checked_mul(lookup_index, 2)?,
                    )?,
                    tag,
                )?,
                tag,
            )?;
            let lookup = slice_from(table, lookup_offset, tag)?;
            let lookup_type = read_u16(lookup, 0, tag)?;
            let lookup_flags = read_u16(lookup, 2, tag)?;
            let subtable_count = usize::from(read_u16(lookup, 4, tag)?);
            ensure(lookup, 6, checked_mul(subtable_count, 2)?, tag)?;
            if lookup_flags & 0x0010 != 0 {
                let filtering_set = checked_add(6, checked_mul(subtable_count, 2)?)?;
                let _ = read_u16(lookup, filtering_set, tag)?;
            }

            let mut subtable_offsets = Vec::with_capacity(subtable_count);
            for subtable_index in 0..subtable_count {
                let offset = checked_add(6, checked_mul(subtable_index, 2)?)?;
                let subtable_offset =
                    relative_offset(lookup, 0, read_u16(lookup, offset, tag)?, tag)?;
                subtable_offsets.push(checked_add(lookup_offset, subtable_offset)?);
            }
            let active = active_lookup_indices.contains(&(lookup_index as u16))
                && !subtable_offsets.is_empty();
            // OpenType extension lookups carry the real lookup type in each
            // extension subtable. Decode it once so the run loop does not
            // probe the same extension through every GSUB operation kind.
            // Keep type 7 for inconsistent or unsupported extension records;
            // the existing checked fallback remains authoritative there.
            let execution_type = if active && tag == GSUB_TAG && lookup_type == 7 {
                let mut extension_type = None;
                let mut consistent = true;
                for subtable_offset in &subtable_offsets {
                    let subtable = slice_from(table, *subtable_offset, GSUB_TAG)?;
                    if read_u16(subtable, 0, GSUB_TAG)? != 1 {
                        consistent = false;
                        break;
                    }
                    let current = read_u16(subtable, 2, GSUB_TAG)?;
                    if extension_type.is_some_and(|previous| previous != current) {
                        consistent = false;
                        break;
                    }
                    extension_type = Some(current);
                }
                if consistent {
                    extension_type.unwrap_or(lookup_type)
                } else {
                    lookup_type
                }
            } else {
                lookup_type
            };
            let compiled_subtables = if active {
                subtable_offsets
                    .iter()
                    .map(|offset| compiled::compile_subtable(table, tag, lookup_type, *offset))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            };
            let compiled_ligature_only = active
                && compiled_subtables
                    .iter()
                    .all(|subtable| subtable.as_ref().is_some_and(CompiledSubtable::is_ligature));
            let compiled_pair_only = active
                && compiled_subtables
                    .iter()
                    .all(|subtable| subtable.as_ref().is_some_and(CompiledSubtable::is_pair));
            lookups.push(LookupState {
                lookup_type,
                execution_type,
                lookup_flags,
                subtable_offsets,
                compiled_subtables,
                compiled_ligature_only,
                compiled_pair_only,
            });
        }

        let mut feature_lookups = Vec::with_capacity(feature_specs_with_indices.len());
        for (script_tag, language_tag, wanted, lookup_indices) in feature_specs_with_indices {
            let active_lookups = lookup_indices
                .iter()
                .map(|lookup_index| {
                    lookups
                        .get(usize::from(*lookup_index))
                        .cloned()
                        .ok_or_else(|| malformed(tag))
                })
                .collect::<Result<Vec<_>, _>>()?;
            feature_lookups.push(FeatureLookupState {
                script_tag,
                language_tag,
                wanted,
                lookup_indices,
                lookups: active_lookups,
            });
        }

        let mut feature_lookup_map = HashMap::with_capacity(feature_lookups.len());
        for (index, feature) in feature_lookups.iter().enumerate() {
            if let Some(key) =
                FeatureQueryKey::new(feature.script_tag, feature.language_tag, &feature.wanted)
            {
                // Preserve the old linear `.find()` behavior if malformed or
                // unusual data describes the same feature tuple twice.
                feature_lookup_map.entry(key).or_insert(index);
            }
        }

        Ok(Self {
            tag,
            lookups,
            feature_lookups,
            feature_lookup_map,
        })
    }

    fn table<'a>(&self, face: &'a SfntFace<'_>) -> Result<&'a [u8], SfntError> {
        face.table(self.tag.0)
            .ok_or(SfntError::MissingTable(self.tag))
    }

    fn feature_lookups(&self, script_tag: Tag, wanted: &[Tag]) -> &[LookupState] {
        self.feature_lookups_with_language(script_tag, None, wanted)
    }

    fn feature_lookups_with_language(
        &self,
        script_tag: Tag,
        language_tag: Option<Tag>,
        wanted: &[Tag],
    ) -> &[LookupState] {
        let Some(key) = FeatureQueryKey::new(script_tag, language_tag, wanted) else {
            return self
                .feature_lookups
                .iter()
                .find(|feature| {
                    feature.script_tag == script_tag
                        && feature.language_tag == language_tag
                        && feature.wanted == wanted
                })
                .map_or(&[], |feature| &feature.lookups);
        };
        let exact = self.feature_lookup_map.get(&key).copied();
        let index = exact.or_else(|| {
            language_tag.and_then(|_| {
                FeatureQueryKey::new(script_tag, None, wanted)
                    .and_then(|key| self.feature_lookup_map.get(&key).copied())
            })
        });
        index
            .and_then(|index| self.feature_lookups.get(index))
            .map_or(&[], |feature| &feature.lookups)
    }

    fn feature_lookup_indices(&self, script_tag: Tag, wanted: &[Tag]) -> &[u16] {
        let Some(key) = FeatureQueryKey::new(script_tag, None, wanted) else {
            return self
                .feature_lookups
                .iter()
                .find(|feature| {
                    feature.script_tag == script_tag
                        && feature.language_tag.is_none()
                        && feature.wanted == wanted
                })
                .map_or(&[], |feature| &feature.lookup_indices);
        };
        self.feature_lookup_map
            .get(&key)
            .and_then(|index| self.feature_lookups.get(*index))
            .map_or(&[], |feature| &feature.lookup_indices)
    }

    fn lookup(&self, lookup_index: u16) -> Result<&LookupState, SfntError> {
        self.lookups
            .get(usize::from(lookup_index))
            .ok_or(SfntError::MalformedTable(self.tag))
    }
}

#[cfg(test)]
impl LayoutState {
    pub(crate) fn compiled_fast_path_count(&self) -> usize {
        self.gsub
            .iter()
            .chain(self.gpos.iter())
            .flat_map(|table| table.lookups.iter())
            .flat_map(|lookup| lookup.compiled_subtables.iter())
            .filter(|subtable| subtable.is_some())
            .count()
    }

    pub(crate) fn active_execution_plan_count(&self) -> usize {
        self.gsub
            .iter()
            .chain(self.gpos.iter())
            .flat_map(|table| table.feature_lookups.iter())
            .flat_map(|feature| feature.lookups.iter())
            .count()
    }
}

/// Shapes a run through the currently supported Aimer-owned script subset.
///
/// `Ok(None)` means this specialized path declined the run. The caller then
/// uses the checked scalar layout path. Keeping script detection here makes
/// the low-level rasterizer seam safe for direct callers while the paragraph
/// layout pass still splits runs at its richer Unicode script boundaries.
pub(crate) fn shape_run(
    face: &SfntFace<'_>,
    text: &str,
) -> Result<Option<AimerShapedRun>, SfntError> {
    shape_run_with_options(face, text, None, false)
}

pub(crate) fn shape_run_with_options(
    face: &SfntFace<'_>,
    text: &str,
    language: Option<TextLanguage>,
    vertical: bool,
) -> Result<Option<AimerShapedRun>, SfntError> {
    let layout = LayoutState::parse(face)?;
    shape_run_with_layout_options(face, &layout, text, language, vertical)
}

pub(crate) fn shape_run_with_layout(
    face: &SfntFace<'_>,
    layout: &LayoutState,
    text: &str,
) -> Result<Option<AimerShapedRun>, SfntError> {
    shape_run_with_layout_options(face, layout, text, None, false)
}

pub(crate) fn shape_run_with_layout_options(
    face: &SfntFace<'_>,
    layout: &LayoutState,
    text: &str,
    language: Option<TextLanguage>,
    vertical: bool,
) -> Result<Option<AimerShapedRun>, SfntError> {
    shape_run_with_layout_options_at_weight(
        face,
        layout,
        text,
        language,
        vertical,
        NORMAL_GLYPH_WEIGHT,
    )
}

pub(crate) fn shape_run_with_layout_options_at_weight(
    face: &SfntFace<'_>,
    layout: &LayoutState,
    text: &str,
    language: Option<TextLanguage>,
    vertical: bool,
    weight: u16,
) -> Result<Option<AimerShapedRun>, SfntError> {
    let (coordinates, coordinate_count) = face.coordinates_for_weight_instance(weight);
    shape_run_with_layout_options_at_coordinates(
        face,
        layout,
        text,
        language,
        vertical,
        &coordinates[..coordinate_count],
    )
}

pub(crate) fn shape_run_with_layout_options_at_coordinates(
    face: &SfntFace<'_>,
    layout: &LayoutState,
    text: &str,
    language: Option<TextLanguage>,
    vertical: bool,
    coordinates: &[f32],
) -> Result<Option<AimerShapedRun>, SfntError> {
    LAYOUT_SCRATCH.with(|scratch| {
        shape_run_with_layout_options_at_coordinates_with_scratch(
            face,
            layout,
            text,
            language,
            vertical,
            coordinates,
            &mut scratch.borrow_mut(),
        )
    })
}

fn shape_run_with_layout_options_at_coordinates_with_scratch(
    face: &SfntFace<'_>,
    layout: &LayoutState,
    text: &str,
    language: Option<TextLanguage>,
    vertical: bool,
    coordinates: &[f32],
    scratch: &mut LayoutScratch,
) -> Result<Option<AimerShapedRun>, SfntError> {
    if text.is_ascii() {
        return if vertical {
            shape_simple_run(face, text, vertical, coordinates, scratch)
        } else {
            shape_latin_run_with_layout(face, layout, text, scratch)
        };
    }

    if !vertical && indic::script_for_text(text).is_some() {
        if let Some(shaped) = indic::shape_run_with_layout(face, layout, text, scratch)? {
            return Ok(Some(shaped));
        }
    }

    if !vertical && southeast_asian::script_for_text(text).is_some() {
        if let Some(shaped) = southeast_asian::shape_run_with_layout(face, layout, text, scratch)? {
            return Ok(Some(shaped));
        }
    }

    if !vertical
        && text
            .chars()
            .any(|codepoint| arabic_joining_type(codepoint).is_some())
    {
        if let Some(shaped) = shape_arabic_run_with_layout(face, layout, text, scratch)? {
            return Ok(Some(shaped));
        }
    }

    if can_shape_cjk_run_text(text)
        && (language.is_some() || vertical || can_shape_cjk_variation_text(text))
    {
        if let Some(shaped) = shape_cjk_run_with_options(
            face,
            layout,
            text,
            language,
            vertical,
            coordinates,
            scratch,
        )? {
            return Ok(Some(shaped));
        }
    }

    shape_simple_run(face, text, vertical, coordinates, scratch)
}

/// Reports whether this layout slice can produce an owned result for `text`.
///
/// Keeping this inexpensive predicate beside the dispatcher lets the caller
/// skip face/layout-state setup for scripts that intentionally remain on the
/// checked layout path. It must stay in sync with
/// [`shape_run_with_layout`]: `true` means only that the slice is eligible,
/// not that the font necessarily contains all requested glyphs.
#[inline]
pub(crate) fn can_shape_text(text: &str) -> bool {
    can_shape_text_with_options(text, None, false)
}

#[inline]
pub(crate) fn can_shape_text_with_options(
    text: &str,
    _language: Option<TextLanguage>,
    _vertical: bool,
) -> bool {
    !text.is_empty()
}

/// Reports whether a known paragraph script belongs to an owned shaping
/// slice. This is an admission fast path only: [`shape_run_with_layout`]
/// still checks the complete text and returns `None` for unsupported or mixed
/// input, allowing the caller to use the checked scalar fallback.
#[inline]
pub(crate) fn can_shape_text_with_script_hint(
    text: &str,
    _script: Option<Script>,
    _language: Option<TextLanguage>,
    _vertical: bool,
) -> bool {
    !text.is_empty()
}

/// Shapes a run with the checked cmap/hmtx tables when no script-specific
/// feature plan is needed. This is the owned fallback for Greek, Cyrillic,
/// Hebrew, Hangul, emoji-capable fonts, and other scripts whose glyph
/// selection is one Unicode scalar at a time.
fn shape_simple_run(
    face: &SfntFace<'_>,
    text: &str,
    vertical: bool,
    coordinates: &[f32],
    scratch: &mut LayoutScratch,
) -> Result<Option<AimerShapedRun>, SfntError> {
    if text.is_empty() {
        return Ok(None);
    }

    let metrics = face.metrics()?;
    scratch.glyphs.clear();
    if !face.for_each_glyph_with_advance_and_variations(
        text,
        metrics,
        |cluster, glyph_id, advance| {
            scratch
                .glyphs
                .push(LayoutGlyph::from_glyph_id(glyph_id, cluster, advance));
        },
    )? {
        return Ok(None);
    }

    if vertical {
        for glyph in &mut scratch.glyphs {
            if let Some(vertical_metrics) =
                face.vertical_glyph_metrics_at_coordinates(glyph.glyph_id, coordinates)?
            {
                glyph.x_advance = 0;
                glyph.y_advance = -i32::from(vertical_metrics.advance_height);
                glyph.y_offset = -vertical_metrics.vert_origin_y;
            } else {
                glyph.x_advance = 0;
                glyph.y_advance = -i32::from(metrics.units_per_em);
            }
        }
    }

    Ok(Some(AimerShapedRun {
        units_per_em: metrics.units_per_em,
        glyphs: std::mem::take(&mut scratch.glyphs),
    }))
}

fn shape_cjk_run_with_options(
    face: &SfntFace<'_>,
    layout: &LayoutState,
    text: &str,
    language: Option<TextLanguage>,
    vertical: bool,
    coordinates: &[f32],
    scratch: &mut LayoutScratch,
) -> Result<Option<AimerShapedRun>, SfntError> {
    if !can_shape_text_with_options(text, language, vertical) {
        return Ok(None);
    }

    let metrics = face.metrics()?;
    scratch.glyphs.clear();
    if !face.for_each_glyph_with_advance_and_variations(
        text,
        metrics,
        |cluster, glyph_id, advance| {
            scratch
                .glyphs
                .push(LayoutGlyph::from_glyph_id(glyph_id, cluster, advance));
        },
    )? {
        return Ok(None);
    }

    let mut changed = can_shape_cjk_variation_text(text);
    let language_tag = language.map(cjk_language_tag);
    if language.is_some() {
        changed |= apply_single_feature_substitutions(
            face,
            metrics,
            &mut scratch.glyphs,
            layout,
            HANI_TAG,
            language_tag,
            &[LOCL_TAG],
        )?;
    }
    if vertical {
        let vertical_feature = if layout.gsub.as_ref().is_some_and(|gsub| {
            !gsub
                .feature_lookups_with_language(HANI_TAG, language_tag, &[VRT2_TAG])
                .is_empty()
        }) {
            VRT2_TAG
        } else {
            VERT_TAG
        };
        apply_single_feature_substitutions(
            face,
            metrics,
            &mut scratch.glyphs,
            layout,
            HANI_TAG,
            language_tag,
            &[vertical_feature],
        )?;
        // `vpal` supplies proportional vertical alternates for punctuation
        // and symbols whose default vertical advance is intentionally wider
        // than the surrounding CJK em square. Apply it after `vert`/`vrt2`
        // so the alternate receives the final vertical metrics below.
        apply_single_feature_substitutions(
            face,
            metrics,
            &mut scratch.glyphs,
            layout,
            HANI_TAG,
            language_tag,
            &[VPAL_TAG],
        )?;

        // Vertical substitution alone is not enough to position a glyph. A
        // face without a usable vhea/vmtx pair must stay on the compatibility
        // path; otherwise a successful `vert` substitution would still be
        // advanced with horizontal metrics and paint at the wrong origin.
        for glyph in &mut scratch.glyphs {
            let Some(vertical_metrics) =
                face.vertical_glyph_metrics_at_coordinates(glyph.glyph_id, coordinates)?
            else {
                return Ok(None);
            };
            glyph.x_advance = 0;
            glyph.y_advance = -i32::from(vertical_metrics.advance_height);
            // OpenType VORG is expressed from the vertical pen origin in
            // font coordinates. The top-to-bottom pen places the glyph's
            // coordinate origin below that point.
            glyph.y_offset = -vertical_metrics.vert_origin_y;
        }
        apply_gpos(
            face,
            &mut scratch.glyphs,
            &layout.gdef,
            layout.gpos.as_ref(),
            HANI_TAG,
            language_tag,
            &[VKRN_TAG],
        )?;
        changed = true;
    }

    if !changed {
        return Ok(None);
    }

    Ok(Some(AimerShapedRun {
        units_per_em: metrics.units_per_em,
        glyphs: std::mem::take(&mut scratch.glyphs),
    }))
}

fn can_shape_cjk_variation_text(text: &str) -> bool {
    can_shape_cjk_text(text, true)
}

fn can_shape_cjk_run_text(text: &str) -> bool {
    text.chars().any(is_cjk_ideograph) && can_shape_cjk_text(text, false)
}

fn can_shape_cjk_text(text: &str, require_variation: bool) -> bool {
    let mut has_variation = false;
    let mut previous_was_cjk_ideograph = false;
    for codepoint in text.chars() {
        if super::is_variation_selector(codepoint as u32) {
            if !previous_was_cjk_ideograph {
                return false;
            }
            has_variation = true;
            previous_was_cjk_ideograph = false;
        } else {
            previous_was_cjk_ideograph = is_cjk_ideograph(codepoint);
            if !previous_was_cjk_ideograph && !is_cjk_run_context(codepoint) {
                return false;
            }
        }
    }
    !require_variation || has_variation
}

#[inline]
fn cjk_language_tag(language: TextLanguage) -> Tag {
    match language {
        TextLanguage::Chinese => ZHS_TAG,
        TextLanguage::Japanese => JAN_TAG,
        TextLanguage::Korean => KOR_TAG,
    }
}

#[inline]
fn is_cjk_ideograph(codepoint: char) -> bool {
    matches!(
        codepoint as u32,
        0x3400..=0x4dbf
            | 0x4e00..=0x9fff
            | 0xf900..=0xfaff
            | 0x20000..=0x323af
    )
}

#[inline]
fn is_cjk_run_context(codepoint: char) -> bool {
    matches!(
        codepoint as u32,
        0x3000..=0x303f
            | 0x3040..=0x30ff
            | 0x3100..=0x312f
            | 0xff00..=0xffef
    ) || codepoint.is_ascii_whitespace()
        || codepoint.is_ascii_punctuation()
        || codepoint.is_ascii_digit()
}

type LayoutGlyph = AimerLayoutGlyph;

impl AimerLayoutGlyph {
    fn from_glyph_id(glyph_id: u16, cluster: usize, x_advance: u16) -> Self {
        Self {
            glyph_id,
            cluster,
            x_advance: i32::from(x_advance),
            y_advance: 0,
            x_offset: 0,
            y_offset: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArabicJoiningType {
    Dual,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArabicJoiningForm {
    Isolated,
    Initial,
    Medial,
    Final,
}

/// Returns the joining behavior for the Arabic letters covered by this first
/// shaping slice. Unknown Arabic codepoints are left to the compatibility
/// shaper rather than being assigned a guessed form.
fn arabic_joining_type(codepoint: char) -> Option<ArabicJoiningType> {
    let codepoint = codepoint as u32;
    if matches!(codepoint, 0x0622..=0x0625 | 0x0627 | 0x062f..=0x0630 | 0x0631..=0x0632 | 0x0648 | 0x0649 | 0x06c9 | 0x06cc)
    {
        return Some(ArabicJoiningType::Right);
    }
    if matches!(
        codepoint,
        0x0621
            | 0x0626
            | 0x0628..=0x062e
            | 0x0633..=0x063a
            | 0x0641..=0x064a
            | 0x0671..=0x06d3
            | 0x06fa..=0x06fc
            | 0x0640
    ) {
        return Some(ArabicJoiningType::Dual);
    }
    None
}

fn is_arabic_mark(codepoint: char) -> bool {
    matches!(
        codepoint as u32,
        0x0610..=0x061a
            | 0x064b..=0x065f
            | 0x0670
            | 0x06d6..=0x06ed
            | 0x08d3..=0x08ff
    )
}

fn previous_joining_type(
    joining_types: &[Option<ArabicJoiningType>],
    transparent: &[bool],
    index: usize,
) -> Option<ArabicJoiningType> {
    let mut previous = index.checked_sub(1)?;
    loop {
        if let Some(joining_type) = joining_types[previous] {
            return Some(joining_type);
        }
        if !transparent[previous] {
            return None;
        }
        previous = previous.checked_sub(1)?;
    }
}

fn next_joining_type(
    joining_types: &[Option<ArabicJoiningType>],
    transparent: &[bool],
    index: usize,
) -> Option<ArabicJoiningType> {
    let mut next = index.checked_add(1)?;
    loop {
        if let Some(joining_type) = joining_types.get(next).copied().flatten() {
            return Some(joining_type);
        }
        if !transparent.get(next).copied().unwrap_or(false) {
            return None;
        }
        next = next.checked_add(1)?;
    }
}

fn arabic_joining_form(
    current: ArabicJoiningType,
    previous: Option<ArabicJoiningType>,
    next: Option<ArabicJoiningType>,
) -> ArabicJoiningForm {
    let joins_previous = previous.is_some_and(|previous| {
        matches!(previous, ArabicJoiningType::Dual)
            && matches!(current, ArabicJoiningType::Dual | ArabicJoiningType::Right)
    });
    let joins_next = next.is_some_and(|next| {
        matches!(current, ArabicJoiningType::Dual)
            && matches!(next, ArabicJoiningType::Dual | ArabicJoiningType::Right)
    });

    match (joins_previous, joins_next) {
        (true, true) => ArabicJoiningForm::Medial,
        (true, false) => ArabicJoiningForm::Final,
        (false, true) => ArabicJoiningForm::Initial,
        (false, false) => ArabicJoiningForm::Isolated,
    }
}

/// Shapes an ASCII/Latin run through the Aimer-owned OpenType subset.
///
/// `Ok(None)` means this subset does not claim the run (for example, the text
/// is non-ASCII or a codepoint is not covered by the face). `Err` means a
/// table required by the attempted subset is malformed and the caller should
/// use its compatibility fallback.
pub(crate) fn shape_latin_run(
    face: &SfntFace<'_>,
    text: &str,
) -> Result<Option<AimerShapedRun>, SfntError> {
    let layout = LayoutState::parse(face)?;
    LAYOUT_SCRATCH
        .with(|scratch| shape_latin_run_with_layout(face, &layout, text, &mut scratch.borrow_mut()))
}

fn shape_latin_run_with_layout(
    face: &SfntFace<'_>,
    layout: &LayoutState,
    text: &str,
    scratch: &mut LayoutScratch,
) -> Result<Option<AimerShapedRun>, SfntError> {
    if text.is_empty() || !text.is_ascii() {
        return Ok(None);
    }

    let metrics = face.metrics()?;
    scratch.glyphs.clear();
    if !face.for_each_glyph_with_advance(text, metrics, |cluster, glyph_id, advance| {
        scratch
            .glyphs
            .push(LayoutGlyph::from_glyph_id(glyph_id, cluster, advance));
    })? {
        return Ok(None);
    }

    if let Some(gsub) = layout.gsub.as_ref() {
        apply_gsub(face, metrics, &mut scratch.glyphs, &layout.gdef, gsub)?;
    }
    apply_gpos(
        face,
        &mut scratch.glyphs,
        &layout.gdef,
        layout.gpos.as_ref(),
        LATN_TAG,
        None,
        &[KERN_TAG],
    )?;

    Ok(Some(AimerShapedRun {
        units_per_em: metrics.units_per_em,
        glyphs: std::mem::take(&mut scratch.glyphs),
    }))
}

/// Shapes the first Arabic Aimer subset: joining-form substitutions selected
/// from the `arab` script's `isol`, `init`, `medi`, and `fina` features plus
/// joining-form substitutions, Arabic ligatures, checked mark attachments,
/// and cursive positioning.
///
/// Unsupported layout records intentionally return `Ok(None)` so the
/// checked scalar layout retains ownership of runs that this subset cannot
/// represent safely.
pub(crate) fn shape_arabic_run(
    face: &SfntFace<'_>,
    text: &str,
) -> Result<Option<AimerShapedRun>, SfntError> {
    let layout = LayoutState::parse(face)?;
    LAYOUT_SCRATCH.with(|scratch| {
        shape_arabic_run_with_layout(face, &layout, text, &mut scratch.borrow_mut())
    })
}

fn shape_arabic_run_with_layout(
    face: &SfntFace<'_>,
    layout: &LayoutState,
    text: &str,
    scratch: &mut LayoutScratch,
) -> Result<Option<AimerShapedRun>, SfntError> {
    if text.is_empty() {
        return Ok(None);
    }

    scratch.joining_types.clear();
    scratch.transparent.clear();
    let mut has_arabic = false;
    for codepoint in text.chars() {
        let joining_type = arabic_joining_type(codepoint);
        if joining_type.is_some() {
            has_arabic = true;
            scratch.transparent.push(false);
        } else if is_arabic_mark(codepoint) {
            // Transparent marks do not break joining; the GPOS attachment
            // passes below either attach them or decline the whole run.
            scratch.transparent.push(true);
        } else if !(codepoint.is_ascii_digit()
            || codepoint.is_ascii_punctuation()
            || codepoint.is_ascii_whitespace())
        {
            return Ok(None);
        } else {
            scratch.transparent.push(false);
        }
        scratch.joining_types.push(joining_type);
    }
    if !has_arabic {
        return Ok(None);
    }

    let metrics = face.metrics()?;
    scratch.glyphs.clear();
    if !face.for_each_glyph_with_advance(text, metrics, |cluster, glyph_id, advance| {
        scratch
            .glyphs
            .push(LayoutGlyph::from_glyph_id(glyph_id, cluster, advance));
    })? {
        return Ok(None);
    }

    scratch.forms.clear();
    scratch.forms.extend(
        scratch
            .joining_types
            .iter()
            .enumerate()
            .map(|(index, form)| {
                form.map(|joining_type| {
                    arabic_joining_form(
                        joining_type,
                        previous_joining_type(&scratch.joining_types, &scratch.transparent, index),
                        next_joining_type(&scratch.joining_types, &scratch.transparent, index),
                    )
                })
            }),
    );

    let joining_changed = apply_arabic_joining(
        face,
        metrics,
        &mut scratch.glyphs,
        &scratch.forms,
        &layout.gdef,
        layout.gsub.as_ref(),
    )?;
    let contextual_changed = if let Some(gsub) = layout.gsub.as_ref() {
        context::apply_arabic_contextual_substitutions(
            face,
            metrics,
            &mut scratch.glyphs,
            &layout.gdef,
            gsub,
            &mut scratch.context_candidates,
        )?
    } else {
        false
    };
    // The current ligature path consumes glyphs from the vector. Keep it out
    // of marked runs until mark cluster bookkeeping is carried through those
    // substitutions as well.
    let ligature_changed = if scratch.transparent.iter().any(|is_mark| *is_mark) {
        false
    } else {
        apply_arabic_ligatures(
            face,
            metrics,
            &mut scratch.glyphs,
            &layout.gdef,
            layout.gsub.as_ref(),
        )?
    };
    let cursive_changed = if ligature_changed {
        // The current ligature path does not yet retain a per-component
        // attachment map, so do not attach against a collapsed glyph vector.
        false
    } else {
        apply_arabic_cursive(
            face,
            &mut scratch.glyphs,
            &scratch.joining_types,
            &scratch.transparent,
            &layout.gdef,
            layout.gpos.as_ref(),
        )?
    };
    let mark_changed = apply_arabic_mark_positioning(
        face,
        &mut scratch.glyphs,
        &scratch.transparent,
        &layout.gdef,
        layout.gpos.as_ref(),
    )?;
    if scratch.transparent.iter().any(|is_mark| *is_mark) && !mark_changed {
        return Ok(None);
    }
    if !joining_changed
        && !contextual_changed
        && !ligature_changed
        && !cursive_changed
        && !mark_changed
    {
        return Ok(None);
    }

    Ok(Some(AimerShapedRun {
        units_per_em: metrics.units_per_em,
        glyphs: std::mem::take(&mut scratch.glyphs),
    }))
}

#[derive(Clone)]
struct Gdef {
    glyph_class: Option<ClassDef>,
}

impl Gdef {
    fn parse(table: Option<&[u8]>) -> Result<Self, SfntError> {
        let Some(table) = table else {
            return Ok(Self { glyph_class: None });
        };
        ensure(table, 0, 12, GDEF_TAG)?;
        if read_u16(table, 0, GDEF_TAG)? != 1 {
            return Err(malformed(GDEF_TAG));
        }

        let glyph_class_offset = usize::from(read_u16(table, 4, GDEF_TAG)?);
        Ok(Self {
            glyph_class: ClassDef::new(table, glyph_class_offset, GDEF_TAG)?,
        })
    }

    fn class(&self, glyph_id: u16) -> Result<u16, SfntError> {
        self.glyph_class
            .as_ref()
            .map_or(Ok(0), |definition| definition.class(glyph_id))
    }

    fn ignores(&self, glyph_id: u16, lookup_flags: u16) -> Result<bool, SfntError> {
        if lookup_flags & 0x000e == 0 {
            return Ok(false);
        }
        let class = self.class(glyph_id)?;
        Ok((lookup_flags & 0x0002 != 0 && class == 1)
            || (lookup_flags & 0x0004 != 0 && class == 2)
            || (lookup_flags & 0x0008 != 0 && class == 3))
    }
}

#[derive(Clone)]
enum ClassDef {
    Format1 { start: u16, classes: Vec<u16> },
    Format2 { ranges: Vec<(u16, u16, u16)> },
}

impl ClassDef {
    fn new(bytes: &[u8], offset: usize, tag: Tag) -> Result<Option<Self>, SfntError> {
        if offset == 0 {
            return Ok(None);
        }

        let format = read_u16(bytes, offset, tag)?;
        match format {
            1 => {
                let start = read_u16(bytes, offset + 2, tag)?;
                let count = usize::from(read_u16(bytes, offset + 4, tag)?);
                let size = checked_add(6, checked_mul(count, 2)?)?;
                ensure(bytes, offset, size, tag)?;
                let mut classes = Vec::with_capacity(count);
                for index in 0..count {
                    classes.push(read_u16(bytes, offset + 6 + index * 2, tag)?);
                }
                Ok(Some(Self::Format1 { start, classes }))
            }
            2 => {
                let count = usize::from(read_u16(bytes, offset + 2, tag)?);
                let size = checked_add(4, checked_mul(count, 6)?)?;
                ensure(bytes, offset, size, tag)?;
                let mut previous_end = None;
                for index in 0..count {
                    let record = checked_add(offset, checked_add(4, checked_mul(index, 6)?)?)?;
                    let start = read_u16(bytes, record, tag)?;
                    let end = read_u16(bytes, record + 2, tag)?;
                    if start > end || previous_end.is_some_and(|previous| start <= previous) {
                        return Err(malformed(tag));
                    }
                    previous_end = Some(end);
                }
                let mut ranges = Vec::with_capacity(count);
                for index in 0..count {
                    let record = checked_add(offset, checked_add(4, checked_mul(index, 6)?)?)?;
                    ranges.push((
                        read_u16(bytes, record, tag)?,
                        read_u16(bytes, record + 2, tag)?,
                        read_u16(bytes, record + 4, tag)?,
                    ));
                }
                Ok(Some(Self::Format2 { ranges }))
            }
            _ => return Err(malformed(tag)),
        }
    }

    fn class(&self, glyph_id: u16) -> Result<u16, SfntError> {
        match self {
            Self::Format1 { start, classes } => {
                let index = usize::from(glyph_id.saturating_sub(*start));
                if glyph_id < *start || index >= classes.len() {
                    return Ok(0);
                }
                Ok(classes[index])
            }
            Self::Format2 { ranges } => {
                let count = ranges.len();
                let mut low = 0;
                let mut high = count;
                while low < high {
                    let index = low + (high - low) / 2;
                    let (start, end, class) = ranges[index];
                    if glyph_id < start {
                        high = index;
                    } else if glyph_id > end {
                        low = index + 1;
                    } else {
                        return Ok(class);
                    }
                }
                Ok(0)
            }
        }
    }
}

fn apply_gsub(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    layout: &LayoutTableState,
) -> Result<(), SfntError> {
    let table = layout.table(face)?;
    let advances = face.glyph_advances_with_metrics(metrics)?;
    let lookups = layout.feature_lookups(LATN_TAG, &[LIGA_TAG, CLIG_TAG]);

    for lookup in lookups {
        if lookup.compiled_ligature_only {
            for compiled in lookup.compiled_subtables.iter().flatten() {
                compiled.apply_ligature(advances, glyphs, gdef, lookup.lookup_flags)?;
            }
            continue;
        }
        for (subtable_index, subtable_offset) in lookup.subtable_offsets.iter().enumerate() {
            if let Some(compiled) = lookup
                .compiled_subtables
                .get(subtable_index)
                .and_then(Option::as_ref)
            {
                if compiled
                    .apply_ligature(advances, glyphs, gdef, lookup.lookup_flags)?
                    .is_some()
                {
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
        }
    }

    Ok(())
}

fn apply_single_feature_substitutions(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyphs: &mut [LayoutGlyph],
    layout: &LayoutState,
    script_tag: Tag,
    language_tag: Option<Tag>,
    wanted: &[Tag],
) -> Result<bool, SfntError> {
    let Some(gsub) = layout.gsub.as_ref() else {
        return Ok(false);
    };
    let lookups = gsub.feature_lookups_with_language(script_tag, language_tag, wanted);
    if lookups.is_empty() {
        return Ok(false);
    }
    let table = gsub.table(face)?;
    let advances = face.glyph_advances_with_metrics(metrics)?;
    let mut changed = false;

    for lookup in lookups {
        if !matches!(lookup.lookup_type, 1 | 7) {
            continue;
        }
        for glyph in glyphs.iter_mut() {
            if lookup.lookup_flags != 0
                && layout.gdef.ignores(glyph.glyph_id, lookup.lookup_flags)?
            {
                continue;
            }
            for (subtable_index, subtable_offset) in lookup.subtable_offsets.iter().enumerate() {
                if let Some(compiled) = lookup
                    .compiled_subtables
                    .get(subtable_index)
                    .and_then(Option::as_ref)
                {
                    match compiled.apply_single_at(advances, glyph)? {
                        Some(true) => {
                            changed = true;
                            break;
                        }
                        Some(false) => continue,
                        None => {}
                    }
                }
                let subtable = slice_from(table, *subtable_offset, GSUB_TAG)?;
                if apply_gsub_single_at(
                    face,
                    metrics,
                    glyph,
                    lookup.lookup_flags,
                    lookup.execution_type,
                    subtable,
                    0,
                )? {
                    changed = true;
                    break;
                }
            }
        }
    }

    Ok(changed)
}

fn apply_arabic_joining(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyphs: &mut [LayoutGlyph],
    forms: &[Option<ArabicJoiningForm>],
    gdef: &Gdef,
    gsub: Option<&LayoutTableState>,
) -> Result<bool, SfntError> {
    let Some(layout) = gsub else {
        return Ok(false);
    };
    let table = layout.table(face)?;
    let advances = face.glyph_advances_with_metrics(metrics)?;
    let mut changed = false;

    for (index, form) in forms.iter().copied().enumerate() {
        let Some(feature_tag) = form.map(|form| match form {
            ArabicJoiningForm::Isolated => ISOL_TAG,
            ArabicJoiningForm::Initial => INIT_TAG,
            ArabicJoiningForm::Medial => MEDI_TAG,
            ArabicJoiningForm::Final => FINA_TAG,
        }) else {
            continue;
        };
        if gdef.ignores(glyphs[index].glyph_id, 0)? {
            continue;
        }

        let lookups = layout.feature_lookups(ARAB_TAG, &[feature_tag]);
        for lookup in lookups {
            if gdef.ignores(glyphs[index].glyph_id, lookup.lookup_flags)? {
                continue;
            }
            let mut applied = false;
            for (subtable_index, subtable_offset) in lookup.subtable_offsets.iter().enumerate() {
                if let Some(compiled) = lookup
                    .compiled_subtables
                    .get(subtable_index)
                    .and_then(Option::as_ref)
                {
                    match compiled.apply_single_at(advances, &mut glyphs[index])? {
                        Some(true) => {
                            changed = true;
                            applied = true;
                            break;
                        }
                        Some(false) => continue,
                        None => {}
                    }
                }
                let subtable = slice_from(table, *subtable_offset, GSUB_TAG)?;
                if apply_gsub_single_at(
                    face,
                    metrics,
                    &mut glyphs[index],
                    lookup.lookup_flags,
                    lookup.execution_type,
                    subtable,
                    0,
                )? {
                    changed = true;
                    applied = true;
                    break;
                }
            }
            if applied {
                break;
            }
        }
    }

    Ok(changed)
}

fn apply_arabic_ligatures(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    gsub: Option<&LayoutTableState>,
) -> Result<bool, SfntError> {
    let Some(layout) = gsub else {
        return Ok(false);
    };
    let table = layout.table(face)?;
    let advances = face.glyph_advances_with_metrics(metrics)?;
    let lookups = layout.feature_lookups(ARAB_TAG, &[RLIG_TAG, LIGA_TAG]);
    let mut changed = false;

    for lookup in lookups {
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
                    if glyphs.len() < previous_len {
                        changed = true;
                    }
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
            if glyphs.len() < previous_len {
                changed = true;
            }
        }
    }

    Ok(changed)
}

fn apply_arabic_cursive(
    face: &SfntFace<'_>,
    glyphs: &mut [LayoutGlyph],
    joining_types: &[Option<ArabicJoiningType>],
    marks: &[bool],
    gdef: &Gdef,
    gpos: Option<&LayoutTableState>,
) -> Result<bool, SfntError> {
    let Some(layout) = gpos else {
        return Ok(false);
    };
    if joining_types.len() != glyphs.len() || marks.len() != glyphs.len() {
        return Ok(false);
    }
    let table = layout.table(face)?;
    let lookups = layout.feature_lookups(ARAB_TAG, &[CURS_TAG]);
    let mut changed = false;

    for current_index in 0..glyphs.len() {
        if marks[current_index] || joining_types[current_index].is_none() {
            continue;
        }
        let Some(previous_index) = previous_cursive_index(joining_types, marks, current_index)
        else {
            continue;
        };

        for lookup in lookups {
            if gdef.ignores(glyphs[current_index].glyph_id, lookup.lookup_flags)?
                || gdef.ignores(glyphs[previous_index].glyph_id, lookup.lookup_flags)?
            {
                continue;
            }
            let mut applied = false;
            for (subtable_index, subtable_offset) in lookup.subtable_offsets.iter().enumerate() {
                if let Some(compiled) = lookup
                    .compiled_subtables
                    .get(subtable_index)
                    .and_then(Option::as_ref)
                {
                    if compiled.is_cursive() {
                        let Some((x_offset, y_offset)) = compiled.cursive_adjustment(
                            glyphs[current_index].glyph_id,
                            glyphs[previous_index].glyph_id,
                        )?
                        else {
                            continue;
                        };
                        glyphs[current_index].x_offset = glyphs[previous_index].x_offset + x_offset;
                        glyphs[current_index].y_offset = glyphs[previous_index].y_offset + y_offset;
                        changed = true;
                        applied = true;
                        break;
                    }
                }
                let subtable = slice_from(table, *subtable_offset, GPOS_TAG)?;
                let Some((x_offset, y_offset)) = cursive_adjustment_for_lookup(
                    subtable,
                    lookup.lookup_type,
                    glyphs[current_index].glyph_id,
                    glyphs[previous_index].glyph_id,
                    0,
                )?
                else {
                    continue;
                };
                glyphs[current_index].x_offset = glyphs[previous_index].x_offset + x_offset;
                glyphs[current_index].y_offset = glyphs[previous_index].y_offset + y_offset;
                changed = true;
                applied = true;
                break;
            }
            if applied {
                break;
            }
        }
    }

    Ok(changed)
}

fn previous_cursive_index(
    joining_types: &[Option<ArabicJoiningType>],
    marks: &[bool],
    current_index: usize,
) -> Option<usize> {
    let mut index = current_index.checked_sub(1)?;
    loop {
        if marks[index] {
            index = index.checked_sub(1)?;
            continue;
        }
        return joining_types[index].is_some().then_some(index);
    }
}

fn apply_arabic_mark_positioning(
    face: &SfntFace<'_>,
    glyphs: &mut [LayoutGlyph],
    marks: &[bool],
    gdef: &Gdef,
    gpos: Option<&LayoutTableState>,
) -> Result<bool, SfntError> {
    let Some(layout) = gpos else {
        return Ok(false);
    };
    let table = layout.table(face)?;
    let mark_lookups = layout.feature_lookups(ARAB_TAG, &[MARK_TAG]);
    let mark_to_mark_lookups = layout.feature_lookups(ARAB_TAG, &[MKMK_TAG]);
    let mut changed = false;
    let mut all_attached = true;

    for mark_index in 0..glyphs.len() {
        if !marks.get(mark_index).copied().unwrap_or(false) {
            continue;
        }
        let Some(base_index) = mark_index.checked_sub(1) else {
            all_attached = false;
            continue;
        };
        let mark_to_mark = marks[base_index];
        let lookups: &[LookupState] = if mark_to_mark {
            mark_to_mark_lookups
        } else {
            mark_lookups
        };
        let base_x_offset = if mark_to_mark {
            glyphs[base_index].x_offset
        } else {
            0
        };
        let base_y_offset = if mark_to_mark {
            glyphs[base_index].y_offset
        } else {
            0
        };
        let mut attached = false;

        for lookup in lookups {
            if gdef.ignores(glyphs[mark_index].glyph_id, lookup.lookup_flags)?
                || gdef.ignores(glyphs[base_index].glyph_id, lookup.lookup_flags)?
            {
                continue;
            }
            let mut lookup_applied = false;
            for (subtable_index, subtable_offset) in lookup.subtable_offsets.iter().enumerate() {
                if let Some(compiled) = lookup
                    .compiled_subtables
                    .get(subtable_index)
                    .and_then(Option::as_ref)
                {
                    if compiled.is_mark_to_mark() == mark_to_mark {
                        let Some((x_offset, y_offset)) = compiled.mark_adjustment(
                            glyphs[mark_index].glyph_id,
                            glyphs[base_index].glyph_id,
                        )?
                        else {
                            continue;
                        };
                        glyphs[mark_index].x_offset = base_x_offset + x_offset;
                        glyphs[mark_index].y_offset = base_y_offset + y_offset;
                        glyphs[mark_index].x_advance = 0;
                        glyphs[mark_index].y_advance = 0;
                        changed = true;
                        lookup_applied = true;
                        break;
                    }
                }
                let subtable = slice_from(table, *subtable_offset, GPOS_TAG)?;
                let adjustment = if mark_to_mark {
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
                };
                let Some((x_offset, y_offset)) = adjustment else {
                    continue;
                };
                glyphs[mark_index].x_offset = base_x_offset + x_offset;
                glyphs[mark_index].y_offset = base_y_offset + y_offset;
                glyphs[mark_index].x_advance = 0;
                glyphs[mark_index].y_advance = 0;
                changed = true;
                lookup_applied = true;
                break;
            }
            if lookup_applied {
                attached = true;
                break;
            }
        }
        if !attached {
            all_attached = false;
        }
    }

    Ok(all_attached && changed)
}

pub(super) fn mark_to_mark_adjustment_for_lookup(
    subtable: &[u8],
    lookup_type: u16,
    mark1_glyph: u16,
    mark2_glyph: u16,
    extension_depth: u8,
) -> Result<Option<(i32, i32)>, SfntError> {
    match lookup_type {
        5 | 6 => mark_to_mark_adjustment(subtable, mark1_glyph, mark2_glyph),
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
            mark_to_mark_adjustment_for_lookup(
                extension,
                extension_type,
                mark1_glyph,
                mark2_glyph,
                extension_depth + 1,
            )
        }
        _ => Ok(None),
    }
}

fn mark_to_mark_adjustment(
    subtable: &[u8],
    mark1_glyph: u16,
    mark2_glyph: u16,
) -> Result<Option<(i32, i32)>, SfntError> {
    if read_u16(subtable, 0, GPOS_TAG)? != 1 {
        return Ok(None);
    }
    let mark1_coverage_offset = usize::from(read_u16(subtable, 2, GPOS_TAG)?);
    let mark2_coverage_offset = usize::from(read_u16(subtable, 4, GPOS_TAG)?);
    let class_count = usize::from(read_u16(subtable, 6, GPOS_TAG)?);
    if class_count == 0 {
        return Err(malformed(GPOS_TAG));
    }
    let mark1_array_offset = usize::from(read_u16(subtable, 8, GPOS_TAG)?);
    let mark2_array_offset = usize::from(read_u16(subtable, 10, GPOS_TAG)?);
    let Some(mark1_index) = coverage_index(subtable, mark1_coverage_offset, mark1_glyph, GPOS_TAG)?
    else {
        return Ok(None);
    };
    let Some(mark2_index) = coverage_index(subtable, mark2_coverage_offset, mark2_glyph, GPOS_TAG)?
    else {
        return Ok(None);
    };

    let mark1_count = usize::from(read_u16(subtable, mark1_array_offset, GPOS_TAG)?);
    if mark1_index >= mark1_count {
        return Err(malformed(GPOS_TAG));
    }
    ensure(
        subtable,
        checked_add(mark1_array_offset, 2)?,
        checked_mul(mark1_count, 4)?,
        GPOS_TAG,
    )?;
    let mark1_record = checked_add(
        checked_add(mark1_array_offset, 2)?,
        checked_mul(mark1_index, 4)?,
    )?;
    let mark_class = usize::from(read_u16(subtable, mark1_record, GPOS_TAG)?);
    if mark_class >= class_count {
        return Err(malformed(GPOS_TAG));
    }
    let mark1_anchor_offset =
        usize::from(read_u16(subtable, checked_add(mark1_record, 2)?, GPOS_TAG)?);
    let Some((mark1_x, mark1_y)) =
        anchor_position(subtable, mark1_array_offset, mark1_anchor_offset)?
    else {
        return Ok(None);
    };

    let mark2_count = usize::from(read_u16(subtable, mark2_array_offset, GPOS_TAG)?);
    if mark2_index >= mark2_count {
        return Err(malformed(GPOS_TAG));
    }
    let mark2_record_size = checked_mul(class_count, 2)?;
    let mark2_record = checked_add(
        checked_add(mark2_array_offset, 2)?,
        checked_mul(mark2_index, mark2_record_size)?,
    )?;
    ensure(subtable, mark2_record, mark2_record_size, GPOS_TAG)?;
    let mark2_anchor_offset = usize::from(read_u16(
        subtable,
        checked_add(mark2_record, checked_mul(mark_class, 2)?)?,
        GPOS_TAG,
    )?);
    let Some((mark2_x, mark2_y)) =
        anchor_position(subtable, mark2_array_offset, mark2_anchor_offset)?
    else {
        return Ok(None);
    };

    Ok(Some((mark2_x - mark1_x, mark2_y - mark1_y)))
}

fn cursive_adjustment_for_lookup(
    subtable: &[u8],
    lookup_type: u16,
    current_glyph: u16,
    previous_glyph: u16,
    extension_depth: u8,
) -> Result<Option<(i32, i32)>, SfntError> {
    match lookup_type {
        3 => cursive_adjustment(subtable, current_glyph, previous_glyph),
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
            cursive_adjustment_for_lookup(
                extension,
                extension_type,
                current_glyph,
                previous_glyph,
                extension_depth + 1,
            )
        }
        _ => Ok(None),
    }
}

fn cursive_adjustment(
    subtable: &[u8],
    current_glyph: u16,
    previous_glyph: u16,
) -> Result<Option<(i32, i32)>, SfntError> {
    if read_u16(subtable, 0, GPOS_TAG)? != 1 {
        return Ok(None);
    }
    let coverage_offset = usize::from(read_u16(subtable, 2, GPOS_TAG)?);
    let record_count = usize::from(read_u16(subtable, 4, GPOS_TAG)?);
    let Some(current_index) = coverage_index(subtable, coverage_offset, current_glyph, GPOS_TAG)?
    else {
        return Ok(None);
    };
    let Some(previous_index) = coverage_index(subtable, coverage_offset, previous_glyph, GPOS_TAG)?
    else {
        return Ok(None);
    };
    if current_index >= record_count || previous_index >= record_count {
        return Err(malformed(GPOS_TAG));
    }
    ensure(subtable, 6, checked_mul(record_count, 4)?, GPOS_TAG)?;

    let current_record = checked_add(6, checked_mul(current_index, 4)?)?;
    let previous_record = checked_add(6, checked_mul(previous_index, 4)?)?;
    let current_entry_offset = usize::from(read_u16(subtable, current_record, GPOS_TAG)?);
    let previous_exit_offset = usize::from(read_u16(
        subtable,
        checked_add(previous_record, 2)?,
        GPOS_TAG,
    )?);
    let Some((current_x, current_y)) = anchor_position(subtable, 0, current_entry_offset)? else {
        return Ok(None);
    };
    let Some((previous_x, previous_y)) = anchor_position(subtable, 0, previous_exit_offset)? else {
        return Ok(None);
    };

    Ok(Some((previous_x - current_x, previous_y - current_y)))
}

pub(super) fn mark_to_base_adjustment_for_lookup(
    subtable: &[u8],
    lookup_type: u16,
    mark_glyph: u16,
    base_glyph: u16,
    extension_depth: u8,
) -> Result<Option<(i32, i32)>, SfntError> {
    match lookup_type {
        4 => mark_to_base_adjustment(subtable, mark_glyph, base_glyph),
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
            mark_to_base_adjustment_for_lookup(
                extension,
                extension_type,
                mark_glyph,
                base_glyph,
                extension_depth + 1,
            )
        }
        _ => Ok(None),
    }
}

fn mark_to_base_adjustment(
    subtable: &[u8],
    mark_glyph: u16,
    base_glyph: u16,
) -> Result<Option<(i32, i32)>, SfntError> {
    if read_u16(subtable, 0, GPOS_TAG)? != 1 {
        return Ok(None);
    }
    let mark_coverage_offset = usize::from(read_u16(subtable, 2, GPOS_TAG)?);
    let base_coverage_offset = usize::from(read_u16(subtable, 4, GPOS_TAG)?);
    let class_count = usize::from(read_u16(subtable, 6, GPOS_TAG)?);
    if class_count == 0 {
        return Err(malformed(GPOS_TAG));
    }
    let mark_array_offset = usize::from(read_u16(subtable, 8, GPOS_TAG)?);
    let base_array_offset = usize::from(read_u16(subtable, 10, GPOS_TAG)?);
    let Some(mark_index) = coverage_index(subtable, mark_coverage_offset, mark_glyph, GPOS_TAG)?
    else {
        return Ok(None);
    };
    let Some(base_index) = coverage_index(subtable, base_coverage_offset, base_glyph, GPOS_TAG)?
    else {
        return Ok(None);
    };

    let mark_count = usize::from(read_u16(subtable, mark_array_offset, GPOS_TAG)?);
    if mark_index >= mark_count {
        return Err(malformed(GPOS_TAG));
    }
    ensure(
        subtable,
        checked_add(mark_array_offset, 2)?,
        checked_mul(mark_count, 4)?,
        GPOS_TAG,
    )?;
    let mark_record = checked_add(
        checked_add(mark_array_offset, 2)?,
        checked_mul(mark_index, 4)?,
    )?;
    let mark_class = usize::from(read_u16(subtable, mark_record, GPOS_TAG)?);
    if mark_class >= class_count {
        return Err(malformed(GPOS_TAG));
    }
    let mark_anchor_offset =
        usize::from(read_u16(subtable, checked_add(mark_record, 2)?, GPOS_TAG)?);
    let Some((mark_x, mark_y)) = anchor_position(subtable, mark_array_offset, mark_anchor_offset)?
    else {
        return Ok(None);
    };

    let base_count = usize::from(read_u16(subtable, base_array_offset, GPOS_TAG)?);
    if base_index >= base_count {
        return Err(malformed(GPOS_TAG));
    }
    let base_record_size = checked_mul(class_count, 2)?;
    let base_record = checked_add(
        checked_add(base_array_offset, 2)?,
        checked_mul(base_index, base_record_size)?,
    )?;
    ensure(subtable, base_record, base_record_size, GPOS_TAG)?;
    let base_anchor_offset = usize::from(read_u16(
        subtable,
        checked_add(base_record, checked_mul(mark_class, 2)?)?,
        GPOS_TAG,
    )?);
    let Some((base_x, base_y)) = anchor_position(subtable, base_array_offset, base_anchor_offset)?
    else {
        return Ok(None);
    };

    Ok(Some((base_x - mark_x, base_y - mark_y)))
}

fn anchor_position(
    table: &[u8],
    base: usize,
    relative_offset: usize,
) -> Result<Option<(i32, i32)>, SfntError> {
    if relative_offset == 0 {
        return Ok(None);
    }
    let offset = checked_add(base, relative_offset)?;
    if read_u16(table, offset, GPOS_TAG)? != 1 {
        // Format 2/3 anchors require device tables or contour point data;
        // leave those runs to the checked scalar layout until those anchors
        // have a specialized implementation.
        return Ok(None);
    }
    Ok(Some((
        i32::from(read_i16(table, checked_add(offset, 2)?, GPOS_TAG)?),
        i32::from(read_i16(table, checked_add(offset, 4)?, GPOS_TAG)?),
    )))
}

fn apply_gsub_single_at(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyph: &mut LayoutGlyph,
    lookup_flags: u16,
    lookup_type: u16,
    subtable: &[u8],
    extension_depth: u8,
) -> Result<bool, SfntError> {
    if lookup_flags & 0x0002 != 0 {
        // Preserve the lookup flag check so a lookup that excludes marks does
        // not accidentally rewrite a marked Arabic run.
        return Ok(false);
    }
    match lookup_type {
        1 => apply_single_substitution(face, metrics, glyph, subtable),
        7 => {
            if extension_depth >= MAX_EXTENSION_DEPTH {
                return Err(malformed(GSUB_TAG));
            }
            if read_u16(subtable, 0, GSUB_TAG)? != 1 {
                return Ok(false);
            }
            let extension_type = read_u16(subtable, 2, GSUB_TAG)?;
            let extension_offset = usize::try_from(read_u32(subtable, 4, GSUB_TAG)?)
                .map_err(|_| SfntError::ArithmeticOverflow)?;
            let extension = slice_from(subtable, extension_offset, GSUB_TAG)?;
            apply_gsub_single_at(
                face,
                metrics,
                glyph,
                lookup_flags,
                extension_type,
                extension,
                extension_depth + 1,
            )
        }
        _ => Ok(false),
    }
}

fn apply_single_substitution(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyph: &mut LayoutGlyph,
    subtable: &[u8],
) -> Result<bool, SfntError> {
    let format = read_u16(subtable, 0, GSUB_TAG)?;
    let coverage_offset = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
    let Some(coverage_index) = coverage_index(subtable, coverage_offset, glyph.glyph_id, GSUB_TAG)?
    else {
        return Ok(false);
    };
    let replacement = match format {
        1 => {
            let delta = read_i16(subtable, 4, GSUB_TAG)?;
            glyph.glyph_id.wrapping_add(delta as u16)
        }
        2 => {
            let replacement_count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
            ensure(subtable, 6, checked_mul(replacement_count, 2)?, GSUB_TAG)?;
            if coverage_index >= replacement_count {
                return Err(malformed(GSUB_TAG));
            }
            read_u16(subtable, 6 + coverage_index * 2, GSUB_TAG)?
        }
        _ => return Ok(false),
    };

    if replacement == glyph.glyph_id {
        return Ok(false);
    }
    let Some(advance) = face.glyph_advance_with_metrics(replacement, metrics)? else {
        return Err(malformed(GSUB_TAG));
    };
    glyph.glyph_id = replacement;
    glyph.x_advance = i32::from(advance);
    glyph.y_advance = 0;
    glyph.x_offset = 0;
    glyph.y_offset = 0;
    Ok(true)
}

fn apply_alternate_substitution(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyph: &mut LayoutGlyph,
    subtable: &[u8],
) -> Result<bool, SfntError> {
    if read_u16(subtable, 0, GSUB_TAG)? != 1 {
        return Ok(false);
    }
    let coverage_offset = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
    let Some(coverage_index) = coverage_index(subtable, coverage_offset, glyph.glyph_id, GSUB_TAG)?
    else {
        return Ok(false);
    };
    let alternate_set_count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
    ensure(subtable, 6, checked_mul(alternate_set_count, 2)?, GSUB_TAG)?;
    if coverage_index >= alternate_set_count {
        return Err(malformed(GSUB_TAG));
    }
    let alternate_set_offset = usize::from(read_u16(subtable, 6 + coverage_index * 2, GSUB_TAG)?);
    if alternate_set_offset == 0 {
        return Err(malformed(GSUB_TAG));
    }
    let alternate_set = slice_from(subtable, alternate_set_offset, GSUB_TAG)?;
    let alternate_count = usize::from(read_u16(alternate_set, 0, GSUB_TAG)?);
    if alternate_count == 0 {
        return Err(malformed(GSUB_TAG));
    }
    ensure(alternate_set, 2, checked_mul(alternate_count, 2)?, GSUB_TAG)?;

    // With no explicit alternate index, OpenType shaping uses the first glyph
    // in the alternate set.
    let replacement = read_u16(alternate_set, 2, GSUB_TAG)?;
    if replacement == glyph.glyph_id {
        return Ok(false);
    }
    let Some(advance) = face.glyph_advance_with_metrics(replacement, metrics)? else {
        return Err(malformed(GSUB_TAG));
    };
    glyph.glyph_id = replacement;
    glyph.x_advance = i32::from(advance);
    glyph.y_advance = 0;
    glyph.x_offset = 0;
    glyph.y_offset = 0;
    Ok(true)
}

/// Applies one lookup record to the glyph at `target_index`.
///
/// Contextual GSUB records may target a ligature lookup, including through an
/// extension lookup. The normal run path applies ligature lookups across the
/// whole buffer, but a contextual record must keep its target fixed so the
/// surrounding rule remains valid.
fn apply_gsub_lookup_at(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    lookup_flags: u16,
    lookup_type: u16,
    subtable: &[u8],
    target_index: usize,
    extension_depth: u8,
) -> Result<bool, SfntError> {
    if target_index >= glyphs.len() {
        return Err(malformed(GSUB_TAG));
    }
    match lookup_type {
        1 => apply_gsub_single_at(
            face,
            metrics,
            &mut glyphs[target_index],
            lookup_flags,
            lookup_type,
            subtable,
            extension_depth,
        ),
        3 => {
            if lookup_flags != 0 && gdef.ignores(glyphs[target_index].glyph_id, lookup_flags)? {
                return Ok(false);
            }
            apply_alternate_substitution(face, metrics, &mut glyphs[target_index], subtable)
        }
        4 => apply_ligature_substitution_at(
            face,
            metrics,
            glyphs,
            gdef,
            lookup_flags,
            subtable,
            target_index,
        ),
        7 => {
            if extension_depth >= MAX_EXTENSION_DEPTH {
                return Err(malformed(GSUB_TAG));
            }
            if read_u16(subtable, 0, GSUB_TAG)? != 1 {
                return Ok(false);
            }
            let extension_type = read_u16(subtable, 2, GSUB_TAG)?;
            let extension_offset = usize::try_from(read_u32(subtable, 4, GSUB_TAG)?)
                .map_err(|_| SfntError::ArithmeticOverflow)?;
            let extension = slice_from(subtable, extension_offset, GSUB_TAG)?;
            apply_gsub_lookup_at(
                face,
                metrics,
                glyphs,
                gdef,
                lookup_flags,
                extension_type,
                extension,
                target_index,
                extension_depth + 1,
            )
        }
        _ => Ok(false),
    }
}

fn apply_gsub_subtable(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    lookup_flags: u16,
    lookup_type: u16,
    subtable: &[u8],
    extension_depth: u8,
) -> Result<(), SfntError> {
    match lookup_type {
        4 => apply_ligature_substitution(face, metrics, glyphs, gdef, lookup_flags, subtable),
        7 => {
            if extension_depth >= MAX_EXTENSION_DEPTH {
                return Err(malformed(GSUB_TAG));
            }
            if read_u16(subtable, 0, GSUB_TAG)? != 1 {
                return Ok(());
            }
            let extension_type = read_u16(subtable, 2, GSUB_TAG)?;
            let extension_offset = usize::try_from(read_u32(subtable, 4, GSUB_TAG)?)
                .map_err(|_| SfntError::ArithmeticOverflow)?;
            let extension = slice_from(subtable, extension_offset, GSUB_TAG)?;
            apply_gsub_subtable(
                face,
                metrics,
                glyphs,
                gdef,
                lookup_flags,
                extension_type,
                extension,
                extension_depth + 1,
            )
        }
        _ => Ok(()),
    }
}

fn apply_ligature_substitution(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    lookup_flags: u16,
    subtable: &[u8],
) -> Result<(), SfntError> {
    if read_u16(subtable, 0, GSUB_TAG)? != 1 {
        return Ok(());
    }
    let coverage_offset = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
    let set_count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
    ensure(subtable, 6, checked_mul(set_count, 2)?, GSUB_TAG)?;

    let mut index = 0;
    while index < glyphs.len() {
        if gdef.ignores(glyphs[index].glyph_id, lookup_flags)? {
            index += 1;
            continue;
        }
        let Some(set_index) =
            coverage_index(subtable, coverage_offset, glyphs[index].glyph_id, GSUB_TAG)?
        else {
            index += 1;
            continue;
        };
        if set_index >= set_count {
            return Err(malformed(GSUB_TAG));
        }

        let set_offset = relative_offset(
            subtable,
            0,
            read_u16(subtable, 6 + set_index * 2, GSUB_TAG)?,
            GSUB_TAG,
        )?;
        let ligature_count = usize::from(read_u16(subtable, set_offset, GSUB_TAG)?);
        ensure(
            subtable,
            set_offset + 2,
            checked_mul(ligature_count, 2)?,
            GSUB_TAG,
        )?;

        let mut best = None;
        for ligature_index in 0..ligature_count {
            let offset = set_offset + 2 + ligature_index * 2;
            let ligature_offset = relative_offset(
                subtable,
                set_offset,
                read_u16(subtable, offset, GSUB_TAG)?,
                GSUB_TAG,
            )?;
            let ligature_glyph = read_u16(subtable, ligature_offset, GSUB_TAG)?;
            let component_count = usize::from(read_u16(subtable, ligature_offset + 2, GSUB_TAG)?);
            if component_count == 0 {
                return Err(malformed(GSUB_TAG));
            }
            ensure(
                subtable,
                ligature_offset + 4,
                checked_mul(component_count - 1, 2)?,
                GSUB_TAG,
            )?;
            if component_count > glyphs.len() - index {
                continue;
            }

            let mut matches = true;
            for component_index in 1..component_count {
                let candidate = index + component_index;
                if gdef.ignores(glyphs[candidate].glyph_id, lookup_flags)?
                    || glyphs[candidate].glyph_id
                        != read_u16(
                            subtable,
                            ligature_offset + 2 + component_index * 2,
                            GSUB_TAG,
                        )?
                {
                    matches = false;
                    break;
                }
            }
            if matches
                && best
                    .as_ref()
                    .is_none_or(|(_, best_count)| component_count > *best_count)
            {
                best = Some((ligature_glyph, component_count));
            }
        }

        let Some((ligature_glyph, component_count)) = best else {
            index += 1;
            continue;
        };
        let Some(advance) = face.glyph_advance_with_metrics(ligature_glyph, metrics)? else {
            return Err(malformed(GSUB_TAG));
        };
        let cluster = glyphs[index].cluster;
        glyphs[index] = LayoutGlyph::from_glyph_id(ligature_glyph, cluster, advance);
        glyphs.drain(index + 1..index + component_count);
        if component_count == 1 {
            index += 1;
        }
    }

    Ok(())
}

fn apply_ligature_substitution_at(
    face: &SfntFace<'_>,
    metrics: super::FontMetrics,
    glyphs: &mut Vec<LayoutGlyph>,
    gdef: &Gdef,
    lookup_flags: u16,
    subtable: &[u8],
    target_index: usize,
) -> Result<bool, SfntError> {
    if lookup_flags & 0x0002 != 0 {
        return Ok(false);
    }
    if read_u16(subtable, 0, GSUB_TAG)? != 1 {
        return Ok(false);
    }
    let coverage_offset = usize::from(read_u16(subtable, 2, GSUB_TAG)?);
    let set_count = usize::from(read_u16(subtable, 4, GSUB_TAG)?);
    ensure(subtable, 6, checked_mul(set_count, 2)?, GSUB_TAG)?;
    if gdef.ignores(glyphs[target_index].glyph_id, lookup_flags)? {
        return Ok(false);
    }
    let Some(set_index) = coverage_index(
        subtable,
        coverage_offset,
        glyphs[target_index].glyph_id,
        GSUB_TAG,
    )?
    else {
        return Ok(false);
    };
    if set_index >= set_count {
        return Err(malformed(GSUB_TAG));
    }

    let set_offset = relative_offset(
        subtable,
        0,
        read_u16(subtable, 6 + set_index * 2, GSUB_TAG)?,
        GSUB_TAG,
    )?;
    let ligature_count = usize::from(read_u16(subtable, set_offset, GSUB_TAG)?);
    ensure(
        subtable,
        checked_add(set_offset, 2)?,
        checked_mul(ligature_count, 2)?,
        GSUB_TAG,
    )?;

    let mut best = None;
    for ligature_index in 0..ligature_count {
        let offset = checked_add(set_offset, 2 + ligature_index * 2)?;
        let ligature_offset = relative_offset(
            subtable,
            set_offset,
            read_u16(subtable, offset, GSUB_TAG)?,
            GSUB_TAG,
        )?;
        let ligature_glyph = read_u16(subtable, ligature_offset, GSUB_TAG)?;
        let component_count = usize::from(read_u16(
            subtable,
            checked_add(ligature_offset, 2)?,
            GSUB_TAG,
        )?);
        if component_count == 0 {
            return Err(malformed(GSUB_TAG));
        }
        ensure(
            subtable,
            checked_add(ligature_offset, 4)?,
            checked_mul(component_count - 1, 2)?,
            GSUB_TAG,
        )?;
        if component_count > glyphs.len() - target_index {
            continue;
        }
        let mut matches = true;
        for component_index in 1..component_count {
            let candidate = target_index + component_index;
            if gdef.ignores(glyphs[candidate].glyph_id, lookup_flags)?
                || glyphs[candidate].glyph_id
                    != read_u16(
                        subtable,
                        ligature_offset + 2 + component_index * 2,
                        GSUB_TAG,
                    )?
            {
                matches = false;
                break;
            }
        }
        if matches
            && best
                .as_ref()
                .is_none_or(|(_, best_count)| component_count > *best_count)
        {
            best = Some((ligature_glyph, component_count));
        }
    }

    let Some((ligature_glyph, component_count)) = best else {
        return Ok(false);
    };
    let Some(advance) = face.glyph_advance_with_metrics(ligature_glyph, metrics)? else {
        return Err(malformed(GSUB_TAG));
    };
    let cluster = glyphs[target_index].cluster;
    glyphs[target_index] = LayoutGlyph::from_glyph_id(ligature_glyph, cluster, advance);
    glyphs.drain(target_index + 1..target_index + component_count);
    Ok(true)
}

fn apply_gpos(
    face: &SfntFace<'_>,
    glyphs: &mut [LayoutGlyph],
    gdef: &Gdef,
    gpos: Option<&LayoutTableState>,
    script_tag: Tag,
    language_tag: Option<Tag>,
    wanted: &[Tag],
) -> Result<bool, SfntError> {
    let Some(layout) = gpos else {
        return Ok(false);
    };
    let lookups = layout.feature_lookups_with_language(script_tag, language_tag, wanted);
    let has_pair_lookup = lookups
        .iter()
        .any(|lookup| matches!(lookup.lookup_type, 2 | 9));
    if !has_pair_lookup {
        return Ok(false);
    }
    let table = layout.table(face)?;

    let has_ignored_pair_lookup = lookups
        .iter()
        .any(|lookup| matches!(lookup.lookup_type, 2 | 9) && lookup.lookup_flags & 0x000e != 0);
    if !has_ignored_pair_lookup {
        let mut changed = false;
        let mut pair_cache = HashMap::with_capacity(glyphs.len().min(256));
        for first_index in 0..glyphs.len().saturating_sub(1) {
            let second_index = first_index + 1;
            let first_glyph = glyphs[first_index].glyph_id;
            let second_glyph = glyphs[second_index].glyph_id;
            let cache_key = (u32::from(first_glyph) << 16) | u32::from(second_glyph);
            let adjustment = if let Some(adjustment) = pair_cache.get(&cache_key) {
                *adjustment
            } else {
                let mut first = ValueAdjustment::default();
                let mut second = ValueAdjustment::default();
                for lookup in lookups
                    .iter()
                    .filter(|lookup| matches!(lookup.lookup_type, 2 | 9))
                {
                    if let Some((lookup_first, lookup_second)) =
                        pair_adjustment_for_lookup_state(table, lookup, first_glyph, second_glyph)?
                    {
                        first.add_assign(lookup_first);
                        second.add_assign(lookup_second);
                    }
                }
                let adjustment = (!first.is_zero() || !second.is_zero()).then_some((first, second));
                pair_cache.insert(cache_key, adjustment);
                adjustment
            };
            if let Some((first, second)) = adjustment {
                changed = true;
                let (before_second, second_glyph) = glyphs.split_at_mut(second_index);
                apply_pair_adjustment(
                    &mut before_second[first_index],
                    &mut second_glyph[0],
                    first,
                    second,
                );
            }
        }
        return Ok(changed);
    }

    let mut changed = false;
    for lookup in lookups
        .iter()
        .filter(|lookup| matches!(lookup.lookup_type, 2 | 9))
    {
        let skips_glyphs = lookup.lookup_flags & 0x000e != 0;
        for first_index in 0..glyphs.len() {
            if skips_glyphs && gdef.ignores(glyphs[first_index].glyph_id, lookup.lookup_flags)? {
                continue;
            }
            let Some(second_index) = (if skips_glyphs {
                next_unignored(glyphs, first_index + 1, lookup.lookup_flags, gdef)?
            } else {
                first_index
                    .checked_add(1)
                    .filter(|index| *index < glyphs.len())
            }) else {
                continue;
            };
            let first_glyph = glyphs[first_index].glyph_id;
            let second_glyph = glyphs[second_index].glyph_id;
            if lookup.compiled_pair_only {
                for compiled in lookup.compiled_subtables.iter().flatten() {
                    let Some((first, second)) =
                        compiled.pair_adjustment(first_glyph, second_glyph)?
                    else {
                        continue;
                    };
                    changed |= !first.is_zero() || !second.is_zero();
                    glyphs[first_index].x_advance += first.x_advance;
                    glyphs[first_index].y_advance += first.y_advance;
                    glyphs[first_index].x_offset += first.x_placement;
                    glyphs[first_index].y_offset += first.y_placement;
                    glyphs[second_index].x_advance += second.x_advance;
                    glyphs[second_index].y_advance += second.y_advance;
                    glyphs[second_index].x_offset += second.x_placement;
                    glyphs[second_index].y_offset += second.y_placement;
                    break;
                }
                continue;
            }
            for (subtable_index, subtable_offset) in lookup.subtable_offsets.iter().enumerate() {
                if let Some(compiled) = lookup
                    .compiled_subtables
                    .get(subtable_index)
                    .and_then(Option::as_ref)
                {
                    if compiled.is_pair() {
                        let Some((first, second)) =
                            compiled.pair_adjustment(first_glyph, second_glyph)?
                        else {
                            continue;
                        };
                        changed |= !first.is_zero() || !second.is_zero();
                        glyphs[first_index].x_advance += first.x_advance;
                        glyphs[first_index].y_advance += first.y_advance;
                        glyphs[first_index].x_offset += first.x_placement;
                        glyphs[first_index].y_offset += first.y_placement;
                        glyphs[second_index].x_advance += second.x_advance;
                        glyphs[second_index].y_advance += second.y_advance;
                        glyphs[second_index].x_offset += second.x_placement;
                        glyphs[second_index].y_offset += second.y_placement;
                        break;
                    }
                }
                let subtable = slice_from(table, *subtable_offset, GPOS_TAG)?;
                let adjustment = pair_adjustment_for_lookup(
                    subtable,
                    lookup.lookup_type,
                    first_glyph,
                    second_glyph,
                    0,
                )?;
                if let Some((first, second)) = adjustment {
                    changed |= !first.is_zero() || !second.is_zero();
                    glyphs[first_index].x_advance += first.x_advance;
                    glyphs[first_index].y_advance += first.y_advance;
                    glyphs[first_index].x_offset += first.x_placement;
                    glyphs[first_index].y_offset += first.y_placement;
                    glyphs[second_index].x_advance += second.x_advance;
                    glyphs[second_index].y_advance += second.y_advance;
                    glyphs[second_index].x_offset += second.x_placement;
                    glyphs[second_index].y_offset += second.y_placement;
                    // Subtables in one lookup are alternatives. Once one
                    // has a real adjustment, later script partitions must
                    // not add another adjustment to the same pair.
                    break;
                }
            }
        }
    }

    Ok(changed)
}

fn pair_adjustment_for_lookup_state(
    table: &[u8],
    lookup: &LookupState,
    first_glyph: u16,
    second_glyph: u16,
) -> Result<Option<(ValueAdjustment, ValueAdjustment)>, SfntError> {
    for (subtable_index, subtable_offset) in lookup.subtable_offsets.iter().enumerate() {
        if let Some(compiled) = lookup
            .compiled_subtables
            .get(subtable_index)
            .and_then(Option::as_ref)
        {
            if compiled.is_pair() {
                if let Some(adjustment) = compiled.pair_adjustment(first_glyph, second_glyph)? {
                    return Ok(Some(adjustment));
                }
                continue;
            }
        }
        let subtable = slice_from(table, *subtable_offset, GPOS_TAG)?;
        if let Some(adjustment) =
            pair_adjustment_for_lookup(subtable, lookup.lookup_type, first_glyph, second_glyph, 0)?
        {
            return Ok(Some(adjustment));
        }
    }
    Ok(None)
}

fn apply_pair_adjustment(
    first_glyph: &mut LayoutGlyph,
    second_glyph: &mut LayoutGlyph,
    first: ValueAdjustment,
    second: ValueAdjustment,
) {
    first_glyph.x_advance += first.x_advance;
    first_glyph.y_advance += first.y_advance;
    first_glyph.x_offset += first.x_placement;
    first_glyph.y_offset += first.y_placement;
    second_glyph.x_advance += second.x_advance;
    second_glyph.y_advance += second.y_advance;
    second_glyph.x_offset += second.x_placement;
    second_glyph.y_offset += second.y_placement;
}

fn pair_adjustment_for_lookup(
    subtable: &[u8],
    lookup_type: u16,
    first_glyph: u16,
    second_glyph: u16,
    extension_depth: u8,
) -> Result<Option<(ValueAdjustment, ValueAdjustment)>, SfntError> {
    match lookup_type {
        2 => pair_adjustment(subtable, first_glyph, second_glyph),
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
            pair_adjustment_for_lookup(
                extension,
                extension_type,
                first_glyph,
                second_glyph,
                extension_depth + 1,
            )
        }
        _ => Ok(None),
    }
}

fn next_unignored(
    glyphs: &[LayoutGlyph],
    start: usize,
    lookup_flags: u16,
    gdef: &Gdef,
) -> Result<Option<usize>, SfntError> {
    for index in start..glyphs.len() {
        if !gdef.ignores(glyphs[index].glyph_id, lookup_flags)? {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn previous_unignored(
    glyphs: &[LayoutGlyph],
    before: usize,
    lookup_flags: u16,
    gdef: &Gdef,
) -> Result<Option<usize>, SfntError> {
    let mut index = before;
    while let Some(candidate) = index.checked_sub(1) {
        if !gdef.ignores(glyphs[candidate].glyph_id, lookup_flags)? {
            return Ok(Some(candidate));
        }
        index = candidate;
    }
    Ok(None)
}

#[derive(Clone, Copy, Default)]
struct ValueAdjustment {
    x_placement: i32,
    y_placement: i32,
    x_advance: i32,
    y_advance: i32,
}

impl ValueAdjustment {
    fn is_zero(self) -> bool {
        self.x_placement == 0 && self.y_placement == 0 && self.x_advance == 0 && self.y_advance == 0
    }

    fn add_assign(&mut self, other: Self) {
        self.x_placement += other.x_placement;
        self.y_placement += other.y_placement;
        self.x_advance += other.x_advance;
        self.y_advance += other.y_advance;
    }
}

fn pair_adjustment(
    subtable: &[u8],
    first_glyph: u16,
    second_glyph: u16,
) -> Result<Option<(ValueAdjustment, ValueAdjustment)>, SfntError> {
    let format = read_u16(subtable, 0, GPOS_TAG)?;
    let coverage_offset = usize::from(read_u16(subtable, 2, GPOS_TAG)?);
    let value_format_1 = read_u16(subtable, 4, GPOS_TAG)?;
    let value_format_2 = read_u16(subtable, 6, GPOS_TAG)?;
    let value_size_1 = value_record_size(value_format_1, GPOS_TAG)?;
    let value_size_2 = value_record_size(value_format_2, GPOS_TAG)?;

    match format {
        1 => {
            let Some(set_index) = coverage_index(subtable, coverage_offset, first_glyph, GPOS_TAG)?
            else {
                return Ok(None);
            };
            let pair_set_count = usize::from(read_u16(subtable, 8, GPOS_TAG)?);
            if set_index >= pair_set_count {
                return Err(malformed(GPOS_TAG));
            }
            let set_offset = relative_offset(
                subtable,
                0,
                read_u16(subtable, 10 + set_index * 2, GPOS_TAG)?,
                GPOS_TAG,
            )?;
            let pair_count = usize::from(read_u16(subtable, set_offset, GPOS_TAG)?);
            let record_size = checked_add(2, checked_add(value_size_1, value_size_2)?)?;
            ensure(
                subtable,
                set_offset + 2,
                checked_mul(pair_count, record_size)?,
                GPOS_TAG,
            )?;
            for index in 0..pair_count {
                let record = set_offset + 2 + index * record_size;
                if read_u16(subtable, record, GPOS_TAG)? != second_glyph {
                    continue;
                }
                let (first, first_size) =
                    read_value_adjustment(subtable, record + 2, value_format_1, GPOS_TAG)?;
                let (second, _) = read_value_adjustment(
                    subtable,
                    record + 2 + first_size,
                    value_format_2,
                    GPOS_TAG,
                )?;
                if first.is_zero() && second.is_zero() {
                    // A covered class-0 pair can be a zero-valued default;
                    // it must not mask a later alternative subtable.
                    return Ok(None);
                }
                return Ok(Some((first, second)));
            }
            Ok(None)
        }
        2 => {
            if coverage_index(subtable, coverage_offset, first_glyph, GPOS_TAG)?.is_none() {
                return Ok(None);
            }
            let class_definition_1_offset = usize::from(read_u16(subtable, 8, GPOS_TAG)?);
            let class_definition_2_offset = usize::from(read_u16(subtable, 10, GPOS_TAG)?);
            let class_1_count = usize::from(read_u16(subtable, 12, GPOS_TAG)?);
            let class_2_count = usize::from(read_u16(subtable, 14, GPOS_TAG)?);
            let Some(class_definition_1) =
                ClassDef::new(subtable, class_definition_1_offset, GPOS_TAG)?
            else {
                return Err(malformed(GPOS_TAG));
            };
            let Some(class_definition_2) =
                ClassDef::new(subtable, class_definition_2_offset, GPOS_TAG)?
            else {
                return Err(malformed(GPOS_TAG));
            };
            let class_1 = usize::from(class_definition_1.class(first_glyph)?);
            let class_2 = usize::from(class_definition_2.class(second_glyph)?);
            if class_1 >= class_1_count || class_2 >= class_2_count {
                return Ok(None);
            }
            let record_size = checked_add(value_size_1, value_size_2)?;
            let class_index = checked_add(checked_mul(class_1, class_2_count)?, class_2)?;
            let record = checked_add(16, checked_mul(class_index, record_size)?)?;
            ensure(subtable, record, record_size, GPOS_TAG)?;
            let (first, first_size) =
                read_value_adjustment(subtable, record, value_format_1, GPOS_TAG)?;
            let (second, _) =
                read_value_adjustment(subtable, record + first_size, value_format_2, GPOS_TAG)?;
            if first.is_zero() && second.is_zero() {
                // A covered class-0 pair can be a zero-valued default;
                // it must not mask a later alternative subtable.
                return Ok(None);
            }
            Ok(Some((first, second)))
        }
        _ => Ok(None),
    }
}

fn read_value_adjustment(
    bytes: &[u8],
    mut offset: usize,
    format: u16,
    tag: Tag,
) -> Result<(ValueAdjustment, usize), SfntError> {
    let start = offset;
    let mut value = ValueAdjustment::default();
    for (flag, slot) in [
        (0x0001, &mut value.x_placement),
        (0x0002, &mut value.y_placement),
        (0x0004, &mut value.x_advance),
        (0x0008, &mut value.y_advance),
    ] {
        if format & flag != 0 {
            *slot = i32::from(read_i16(bytes, offset, tag)?);
            offset = checked_add(offset, 2)?;
        }
    }
    for flag in [0x0010, 0x0020, 0x0040, 0x0080] {
        if format & flag != 0 {
            let _ = read_u16(bytes, offset, tag)?;
            offset = checked_add(offset, 2)?;
        }
    }
    if format & !0x00ff != 0 {
        return Err(malformed(tag));
    }
    Ok((value, offset - start))
}

fn value_record_size(format: u16, tag: Tag) -> Result<usize, SfntError> {
    if format & !0x00ff != 0 {
        return Err(malformed(tag));
    }
    checked_mul(format.count_ones() as usize, 2)
}

fn feature_lookup_indices(
    table: &[u8],
    table_tag: Tag,
    script_tag: Tag,
    wanted: &[Tag],
) -> Result<Vec<u16>, SfntError> {
    feature_lookup_indices_with_language(table, table_tag, script_tag, None, wanted)
}

fn feature_lookup_indices_with_language(
    table: &[u8],
    table_tag: Tag,
    script_tag: Tag,
    language_tag: Option<Tag>,
    wanted: &[Tag],
) -> Result<Vec<u16>, SfntError> {
    ensure(table, 0, 10, table_tag)?;
    let script_list_offset = usize::from(read_u16(table, 4, table_tag)?);
    let feature_list_offset = usize::from(read_u16(table, 6, table_tag)?);
    let selected_features = selected_feature_indices(
        table,
        script_list_offset,
        script_tag,
        language_tag,
        table_tag,
    )?;
    let feature_count = usize::from(read_u16(table, feature_list_offset, table_tag)?);
    ensure(
        table,
        feature_list_offset + 2,
        checked_mul(feature_count, 6)?,
        table_tag,
    )?;

    let feature_indices: Vec<usize> = selected_features
        .unwrap_or_else(|| (0..feature_count).collect())
        .into_iter()
        .filter(|index| *index < feature_count)
        .collect();
    let mut lookup_indices = Vec::new();
    for feature_index in feature_indices {
        let record = feature_list_offset + 2 + feature_index * 6;
        let feature_tag = read_tag(table, record, table_tag)?;
        if !wanted.contains(&feature_tag) {
            continue;
        }
        let feature_offset = relative_offset(
            table,
            feature_list_offset,
            read_u16(table, record + 4, table_tag)?,
            table_tag,
        )?;
        let lookup_count = usize::from(read_u16(table, feature_offset + 2, table_tag)?);
        ensure(
            table,
            feature_offset + 4,
            checked_mul(lookup_count, 2)?,
            table_tag,
        )?;
        for lookup_index in 0..lookup_count {
            lookup_indices.push(read_u16(
                table,
                feature_offset + 4 + lookup_index * 2,
                table_tag,
            )?);
        }
    }
    lookup_indices.sort_unstable();
    lookup_indices.dedup();
    Ok(lookup_indices)
}

fn selected_feature_indices(
    table: &[u8],
    script_list_offset: usize,
    requested_script: Tag,
    requested_language: Option<Tag>,
    tag: Tag,
) -> Result<Option<Vec<usize>>, SfntError> {
    if script_list_offset == 0 {
        return Ok(None);
    }
    let script_count = usize::from(read_u16(table, script_list_offset, tag)?);
    ensure(
        table,
        script_list_offset + 2,
        checked_mul(script_count, 6)?,
        tag,
    )?;

    let mut requested = None;
    let mut default_script = None;
    for index in 0..script_count {
        let record = script_list_offset + 2 + index * 6;
        let script_tag = read_tag(table, record, tag)?;
        let script_offset = relative_offset(
            table,
            script_list_offset,
            read_u16(table, record + 4, tag)?,
            tag,
        )?;
        if script_tag == requested_script {
            requested = Some(script_offset);
        } else if script_tag == DFLT_TAG {
            default_script = Some(script_offset);
        }
    }
    let Some(script_offset) = requested.or(default_script) else {
        return Ok(Some(Vec::new()));
    };

    let default_language_offset = usize::from(read_u16(table, script_offset, tag)?);
    let language_count = usize::from(read_u16(table, script_offset + 2, tag)?);
    ensure(
        table,
        script_offset + 4,
        checked_mul(language_count, 6)?,
        tag,
    )?;
    let mut first = None;
    let mut default = None;
    let mut requested = None;
    for index in 0..language_count {
        let record = script_offset + 4 + index * 6;
        let language_tag = read_tag(table, record, tag)?;
        let offset = relative_offset(table, script_offset, read_u16(table, record + 4, tag)?, tag)?;
        first.get_or_insert(offset);
        if language_tag == DFLT_LANG_TAG {
            default = Some(offset);
        }
        if requested_language == Some(language_tag) {
            requested = Some(offset);
        }
    }
    let default_language = if default_language_offset != 0 {
        Some(relative_offset(
            table,
            script_offset,
            default_language_offset as u16,
            tag,
        )?)
    } else {
        None
    };
    let language_offset = requested.or(default_language).or(default).or(first);
    let Some(language_offset) = language_offset else {
        return Ok(Some(Vec::new()));
    };

    let required_feature = read_u16(table, language_offset + 2, tag)?;
    let feature_count = usize::from(read_u16(table, language_offset + 4, tag)?);
    ensure(
        table,
        language_offset + 6,
        checked_mul(feature_count, 2)?,
        tag,
    )?;
    let mut features = Vec::with_capacity(feature_count + 1);
    if required_feature != u16::MAX {
        features.push(usize::from(required_feature));
    }
    for index in 0..feature_count {
        features.push(usize::from(read_u16(
            table,
            language_offset + 6 + index * 2,
            tag,
        )?));
    }
    features.sort_unstable();
    features.dedup();
    Ok(Some(features))
}

fn coverage_index(
    table: &[u8],
    offset: usize,
    glyph_id: u16,
    tag: Tag,
) -> Result<Option<usize>, SfntError> {
    let coverage = slice_from(table, offset, tag)?;
    match read_u16(coverage, 0, tag)? {
        1 => {
            let count = usize::from(read_u16(coverage, 2, tag)?);
            ensure(coverage, 4, checked_mul(count, 2)?, tag)?;
            let mut low = 0;
            let mut high = count;
            while low < high {
                let index = low + (high - low) / 2;
                let candidate = read_u16(coverage, 4 + index * 2, tag)?;
                if glyph_id < candidate {
                    high = index;
                } else if glyph_id > candidate {
                    low = index + 1;
                } else {
                    return Ok(Some(index));
                }
            }
            Ok(None)
        }
        2 => {
            let count = usize::from(read_u16(coverage, 2, tag)?);
            ensure(coverage, 4, checked_mul(count, 6)?, tag)?;
            let mut low = 0;
            let mut high = count;
            while low < high {
                let index = low + (high - low) / 2;
                let record = 4 + index * 6;
                let start = read_u16(coverage, record, tag)?;
                let end = read_u16(coverage, record + 2, tag)?;
                if start > end {
                    return Err(malformed(tag));
                }
                if glyph_id < start {
                    high = index;
                } else if glyph_id > end {
                    low = index + 1;
                } else {
                    let base = usize::from(read_u16(coverage, record + 4, tag)?);
                    return Ok(Some(base + usize::from(glyph_id - start)));
                }
            }
            Ok(None)
        }
        _ => Err(malformed(tag)),
    }
}

fn read_tag(bytes: &[u8], offset: usize, tag: Tag) -> Result<Tag, SfntError> {
    Reader::new(bytes).tag(offset).map_err(|_| malformed(tag))
}

fn read_u16(bytes: &[u8], offset: usize, tag: Tag) -> Result<u16, SfntError> {
    Reader::new(bytes).u16(offset).map_err(|_| malformed(tag))
}

fn read_u32(bytes: &[u8], offset: usize, tag: Tag) -> Result<u32, SfntError> {
    let high = u32::from(read_u16(bytes, offset, tag)?);
    let low = u32::from(read_u16(bytes, checked_add(offset, 2)?, tag)?);
    Ok((high << 16) | low)
}

fn read_i16(bytes: &[u8], offset: usize, tag: Tag) -> Result<i16, SfntError> {
    Reader::new(bytes).i16(offset).map_err(|_| malformed(tag))
}

fn ensure(bytes: &[u8], offset: usize, size: usize, tag: Tag) -> Result<(), SfntError> {
    offset
        .checked_add(size)
        .and_then(|end| bytes.get(offset..end))
        .map(|_| ())
        .ok_or_else(|| malformed(tag))
}

fn slice_from<'a>(bytes: &'a [u8], offset: usize, tag: Tag) -> Result<&'a [u8], SfntError> {
    bytes.get(offset..).ok_or_else(|| malformed(tag))
}

fn relative_offset(bytes: &[u8], base: usize, relative: u16, tag: Tag) -> Result<usize, SfntError> {
    let offset = base
        .checked_add(usize::from(relative))
        .ok_or(SfntError::ArithmeticOverflow)?;
    ensure(bytes, offset, 0, tag)?;
    Ok(offset)
}

fn checked_add(left: usize, right: usize) -> Result<usize, SfntError> {
    left.checked_add(right).ok_or(SfntError::ArithmeticOverflow)
}

fn checked_mul(left: usize, right: usize) -> Result<usize, SfntError> {
    left.checked_mul(right).ok_or(SfntError::ArithmeticOverflow)
}

fn malformed(tag: Tag) -> SfntError {
    SfntError::MalformedTable(tag)
}
