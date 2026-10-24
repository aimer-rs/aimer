//! The small part of OpenType Variations needed by the owned text path.
//!
//! A number of system and bundled fallback fonts are variable TrueType fonts.
//! Their default instance is not required to be regular; the checked-in Noto
//! Sans JP face, for example, defaults to `wght=100`. Reading only `glyf`
//! therefore produces visibly thin CJK text even when the surrounding run is
//! regular. This module reads the `wght` axis, applies `gvar` deltas to
//! outlines before they reach the rasterizer, and evaluates checked HVAR/VVAR
//! metric stores for the same selected instance.

use super::outline::GlyphOutline;
use super::{Reader, SfntError, SfntFace, Tag, checked_add, checked_mul};

const FVAR_TAG: Tag = Tag(*b"fvar");
const GVAR_TAG: Tag = Tag(*b"gvar");
const HVAR_TAG: Tag = Tag(*b"HVAR");
const VVAR_TAG: Tag = Tag(*b"VVAR");
const WGHT_TAG: Tag = Tag(*b"wght");

const FVAR_VERSION: u32 = 0x0001_0000;
const GVAR_VERSION: u32 = 0x0001_0000;
const METRIC_VARIATION_VERSION: u32 = 0x0001_0000;
const FVAR_AXIS_SIZE: usize = 20;
const FVAR_MAX_AXES: u16 = 64;
const GVAR_MAX_GLYPHS: usize = 1 << 20;
const MAX_VARIATION_REGIONS: usize = 1 << 15;
const MAX_VARIATION_DATA_SUBTABLES: usize = 1 << 16;
const MAX_VARIATION_MAP_ENTRIES: usize = 1 << 20;
const MAX_VARIATION_DELTA_VALUES: usize = 1 << 24;
const NO_VARIATION_INDEX: u32 = u32::MAX;

const EMBEDDED_PEAK_TUPLE: u16 = 0x8000;
const INTERMEDIATE_REGION: u16 = 0x4000;
const PRIVATE_POINT_NUMBERS: u16 = 0x2000;
const TUPLE_INDEX_MASK: u16 = 0x0fff;
const SHARED_POINT_NUMBERS: u16 = 0x8000;
const TUPLE_COUNT_MASK: u16 = 0x0fff;

/// One axis record from `fvar`, retained without borrowing the SFNT table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VariationAxis {
    pub(crate) tag: Tag,
    pub(crate) min: f32,
    pub(crate) default: f32,
    pub(crate) max: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct VariationInfo {
    axes: Vec<VariationAxis>,
    weight_axis: Option<usize>,
    gvar: Option<GvarInfo>,
    hvar: Option<MetricVariationTable>,
    vvar: Option<MetricVariationTable>,
}

#[derive(Clone, Debug)]
struct GvarInfo {
    axis_count: usize,
    shared_tuples: Vec<Vec<f32>>,
    glyph_offsets: Vec<usize>,
    data_offset: usize,
}

#[derive(Clone, Debug)]
struct MetricVariationTable {
    store: ItemVariationStore,
    advance_map: Option<DeltaSetIndexMap>,
    side_bearing_map: Option<DeltaSetIndexMap>,
    third_map: Option<DeltaSetIndexMap>,
}

#[derive(Clone, Debug)]
struct DeltaSetIndexMap {
    entries: Option<Vec<DeltaSetIndex>>,
}

#[derive(Clone, Copy, Debug)]
struct DeltaSetIndex {
    outer: u16,
    inner: u16,
}

#[derive(Clone, Debug)]
struct ItemVariationStore {
    tag: Tag,
    regions: Vec<Vec<VariationRegionAxis>>,
    data: Vec<VariationData>,
}

#[derive(Clone, Copy, Debug)]
struct VariationRegionAxis {
    start: f32,
    peak: f32,
    end: f32,
}

#[derive(Clone, Debug)]
struct VariationData {
    item_count: usize,
    region_indices: Vec<u16>,
    deltas: Vec<i32>,
}

#[derive(Clone, Debug)]
struct TupleHeader {
    data_size: usize,
    tuple_index: u16,
    peak: Option<Vec<f32>>,
    intermediate_start: Option<Vec<f32>>,
    intermediate_end: Option<Vec<f32>>,
}

#[derive(Clone, Copy, Default)]
struct PointDelta {
    x: f32,
    y: f32,
}

impl<'a> SfntFace<'a> {
    /// Returns the cached variation metadata, if this face carries `fvar`.
    pub(crate) fn variation_info(&self) -> Result<Option<&VariationInfo>, SfntError> {
        match self
            .variation_cache
            .get_or_init(|| parse_variation_info(self))
        {
            Ok(Some(info)) => Ok(Some(info)),
            Ok(None) => Ok(None),
            Err(error) => Err(*error),
        }
    }

    /// Returns the face's `wght` axis when it has one.
    pub(crate) fn weight_axis(&self) -> Result<Option<VariationAxis>, SfntError> {
        Ok(self
            .variation_info()?
            .and_then(|info| info.weight_axis.map(|index| info.axes[index])))
    }

    /// Reports whether this face has a valid variation model.
    pub(crate) fn has_variations(&self) -> bool {
        self.variation_info().is_ok_and(|info| info.is_some())
    }

    /// Reports whether this face can select a requested OpenType weight.
    pub(crate) fn has_weight_variations(&self) -> bool {
        self.weight_axis().ok().flatten().is_some()
    }

    /// Returns the `HVAR` advance-width delta for one requested weight.
    pub(crate) fn horizontal_advance_delta(
        &self,
        glyph_id: u16,
        weight: u16,
    ) -> Result<i32, SfntError> {
        Ok(self.horizontal_metric_deltas(glyph_id, weight)?[0])
    }

    /// Returns HVAR deltas for advance width, left side bearing, and right
    /// side bearing, respectively.
    pub(crate) fn horizontal_metric_deltas(
        &self,
        glyph_id: u16,
        weight: u16,
    ) -> Result<[i32; 3], SfntError> {
        let Some(info) = self.variation_info()? else {
            return Ok([0; 3]);
        };
        let Some(hvar) = info.hvar.as_ref() else {
            return Ok([0; 3]);
        };
        let (coordinates, coordinate_count) = coordinates_for_weight(info, weight);
        hvar.deltas(glyph_id, &coordinates[..coordinate_count])
    }

    /// Returns HVAR deltas for a complete normalized variation instance.
    pub(crate) fn horizontal_metric_deltas_at_coordinates(
        &self,
        glyph_id: u16,
        coordinates: &[f32],
    ) -> Result<[i32; 3], SfntError> {
        let Some(info) = self.variation_info()? else {
            return Ok([0; 3]);
        };
        let Some(hvar) = info.hvar.as_ref() else {
            return Ok([0; 3]);
        };
        hvar.deltas(glyph_id, coordinates)
    }

    /// Returns the VVAR deltas for advance height, top side bearing, and
    /// vertical origin, respectively.
    pub(crate) fn vertical_metric_deltas(
        &self,
        glyph_id: u16,
        weight: u16,
    ) -> Result<[i32; 3], SfntError> {
        let Some(info) = self.variation_info()? else {
            return Ok([0; 3]);
        };
        let Some(vvar) = info.vvar.as_ref() else {
            return Ok([0; 3]);
        };
        let (coordinates, coordinate_count) = coordinates_for_weight(info, weight);
        vvar.deltas(glyph_id, &coordinates[..coordinate_count])
    }

    /// Returns VVAR deltas for a complete normalized variation instance.
    pub(crate) fn vertical_metric_deltas_at_coordinates(
        &self,
        glyph_id: u16,
        coordinates: &[f32],
    ) -> Result<[i32; 3], SfntError> {
        let Some(info) = self.variation_info()? else {
            return Ok([0; 3]);
        };
        let Some(vvar) = info.vvar.as_ref() else {
            return Ok([0; 3]);
        };
        vvar.deltas(glyph_id, coordinates)
    }

    /// Reports whether this face has a parsed HVAR advance-width store.
    pub(crate) fn has_horizontal_metric_variations(&self) -> bool {
        self.variation_info()
            .is_ok_and(|info| info.is_some_and(|info| info.hvar.is_some()))
    }

    /// Reports whether this face has a parsed VVAR metric store.
    pub(crate) fn has_vertical_metric_variations(&self) -> bool {
        self.variation_info()
            .is_ok_and(|info| info.is_some_and(|info| info.vvar.is_some()))
    }

    /// Returns a normalized coordinate vector for an arbitrary axis request.
    ///
    /// Values are clamped to the axis range and quantized to OpenType's
    /// F2DOT14 precision before being returned. Axis order is the order in
    /// `fvar`, so requests with different input ordering share one stable
    /// identity. `None` means the face has no variation model or the request
    /// contains an unknown, duplicate, or non-finite axis value.
    pub(crate) fn normalized_variation_coordinates(
        &self,
        weight: u16,
        axes: &[(u32, f32)],
    ) -> Result<Option<Vec<f32>>, SfntError> {
        let Some(info) = self.variation_info()? else {
            return Ok(None);
        };
        Ok(coordinates_for_axes(info, weight, axes))
    }

    /// Returns the selected weight instance as normalized coordinates.
    pub(crate) fn coordinates_for_weight_instance(
        &self,
        weight: u16,
    ) -> ([f32; FVAR_MAX_AXES as usize], usize) {
        self.variation_info()
            .ok()
            .flatten()
            .map_or(([0.0; FVAR_MAX_AXES as usize], 0), |info| {
                coordinates_for_weight(info, weight)
            })
    }
}

fn parse_variation_info(face: &SfntFace<'_>) -> Result<Option<VariationInfo>, SfntError> {
    let Some(fvar) = face.table(*b"fvar") else {
        return Ok(None);
    };
    let reader = Reader::new(fvar);
    if reader.u32(0)? != FVAR_VERSION {
        return Err(malformed(FVAR_TAG));
    }

    let axes_offset = usize::from(reader.u16(4)?);
    let axis_count = reader.u16(8)?;
    let axis_size = usize::from(reader.u16(10)?);
    if axis_count == 0 || axis_count > FVAR_MAX_AXES || axis_size < FVAR_AXIS_SIZE {
        return Err(malformed(FVAR_TAG));
    }
    let axes_size = checked_mul(usize::from(axis_count), axis_size)?;
    reader.range(axes_offset, axes_size)?;

    let mut axes = Vec::with_capacity(usize::from(axis_count));
    let mut weight_axis = None;
    for index in 0..usize::from(axis_count) {
        let offset = checked_add(axes_offset, checked_mul(index, axis_size)?)?;
        let axis = VariationAxis {
            tag: reader.tag(offset)?,
            min: fixed_16_16(reader.u32(checked_add(offset, 4)?)?),
            default: fixed_16_16(reader.u32(checked_add(offset, 8)?)?),
            max: fixed_16_16(reader.u32(checked_add(offset, 12)?)?),
        };
        if !axis.min.is_finite()
            || !axis.default.is_finite()
            || !axis.max.is_finite()
            || axis.min > axis.default
            || axis.default > axis.max
        {
            return Err(malformed(FVAR_TAG));
        }
        if axis.tag == WGHT_TAG {
            weight_axis = Some(index);
        }
        axes.push(axis);
    }

    let gvar = face
        .table(*b"gvar")
        .map(|table| parse_gvar(table, usize::from(axis_count)))
        .transpose()?;
    let hvar = face
        .table(*b"HVAR")
        .map(|table| parse_metric_variation_table(table, usize::from(axis_count), HVAR_TAG, false))
        .transpose()?;
    let vvar = face
        .table(*b"VVAR")
        .map(|table| parse_metric_variation_table(table, usize::from(axis_count), VVAR_TAG, true))
        .transpose()?;

    Ok(Some(VariationInfo {
        axes,
        weight_axis,
        gvar,
        hvar,
        vvar,
    }))
}

impl MetricVariationTable {
    fn deltas(&self, glyph_id: u16, coordinates: &[f32]) -> Result<[i32; 3], SfntError> {
        let index = |map: Option<&DeltaSetIndexMap>| {
            map.map_or(DeltaSetIndex::direct(glyph_id), |map| map.get(glyph_id))
        };
        Ok([
            self.store
                .compute_delta(index(self.advance_map.as_ref()), coordinates)?,
            self.store
                .compute_delta(index(self.side_bearing_map.as_ref()), coordinates)?,
            self.store
                .compute_delta(index(self.third_map.as_ref()), coordinates)?,
        ])
    }
}

impl DeltaSetIndex {
    fn direct(glyph_id: u16) -> Self {
        Self {
            outer: 0,
            inner: glyph_id,
        }
    }
}

impl DeltaSetIndexMap {
    fn get(&self, glyph_id: u16) -> DeltaSetIndex {
        let Some(entries) = self.entries.as_ref() else {
            return DeltaSetIndex::direct(glyph_id);
        };
        entries[usize::from(glyph_id).min(entries.len() - 1)]
    }
}

impl ItemVariationStore {
    fn compute_delta(&self, index: DeltaSetIndex, coordinates: &[f32]) -> Result<i32, SfntError> {
        let encoded_index = (u32::from(index.outer) << 16) | u32::from(index.inner);
        if coordinates.is_empty() || encoded_index == NO_VARIATION_INDEX {
            return Ok(0);
        }
        let Some(data) = self.data.get(usize::from(index.outer)) else {
            return Ok(0);
        };
        let item = usize::from(index.inner);
        if item >= data.item_count {
            return Ok(0);
        }

        let region_count = data.region_indices.len();
        let row_start = checked_mul(item, region_count)?;
        let row = data
            .deltas
            .get(row_start..checked_add(row_start, region_count)?)
            .ok_or_else(|| malformed(self.tag))?;
        let mut delta = 0.0_f64;
        for (region_index, value) in data.region_indices.iter().zip(row) {
            let Some(region) = self.regions.get(usize::from(*region_index)) else {
                return Err(malformed(self.tag));
            };
            delta += f64::from(*value) * f64::from(region_scalar(region, coordinates));
        }
        if !delta.is_finite() {
            return Err(malformed(self.tag));
        }
        Ok(delta
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
    }
}

fn coordinates_for_weight(
    info: &VariationInfo,
    weight: u16,
) -> ([f32; FVAR_MAX_AXES as usize], usize) {
    let mut coordinates = [0.0; FVAR_MAX_AXES as usize];
    if let Some(weight_axis) = info.weight_axis {
        coordinates[weight_axis] = normalize_weight(info.axes[weight_axis], f32::from(weight));
    }
    (coordinates, info.axes.len())
}

fn coordinates_for_axes(
    info: &VariationInfo,
    weight: u16,
    axes: &[(u32, f32)],
) -> Option<Vec<f32>> {
    if axes.is_empty() {
        return None;
    }

    let (mut coordinates, coordinate_count) = coordinates_for_weight(info, weight);
    let mut seen = [false; FVAR_MAX_AXES as usize];
    for &(tag_value, value) in axes {
        if !value.is_finite() {
            return None;
        }
        let tag = Tag::from_bytes(tag_value.to_be_bytes());
        let index = info.axes.iter().position(|axis| axis.tag == tag)?;
        if seen[index] {
            return None;
        }
        seen[index] = true;
        coordinates[index] = normalize_axis(info.axes[index], value);
    }

    Some(coordinates[..coordinate_count].to_vec())
}

fn parse_metric_variation_table(
    table: &[u8],
    axis_count: usize,
    tag: Tag,
    vertical: bool,
) -> Result<MetricVariationTable, SfntError> {
    let reader = Reader::new(table);
    let header_size = if vertical { 24 } else { 20 };
    reader.range(0, header_size).map_err(|_| malformed(tag))?;
    if reader.u32(0).map_err(|_| malformed(tag))? != METRIC_VARIATION_VERSION {
        return Err(malformed(tag));
    }

    let store_offset = usize::try_from(reader.u32(4).map_err(|_| malformed(tag))?)
        .map_err(|_| SfntError::ArithmeticOverflow)?;
    let store = parse_item_variation_store(table, store_offset, axis_count, tag)?;
    let advance_map = parse_optional_delta_set_index_map(
        table,
        usize::try_from(reader.u32(8).map_err(|_| malformed(tag))?)
            .map_err(|_| SfntError::ArithmeticOverflow)?,
        tag,
    )?;
    let side_bearing_map = parse_optional_delta_set_index_map(
        table,
        usize::try_from(reader.u32(12).map_err(|_| malformed(tag))?)
            .map_err(|_| SfntError::ArithmeticOverflow)?,
        tag,
    )?;
    let third_map = if vertical {
        parse_optional_delta_set_index_map(
            table,
            usize::try_from(reader.u32(20).map_err(|_| malformed(tag))?)
                .map_err(|_| SfntError::ArithmeticOverflow)?,
            tag,
        )?
    } else {
        parse_optional_delta_set_index_map(
            table,
            usize::try_from(reader.u32(16).map_err(|_| malformed(tag))?)
                .map_err(|_| SfntError::ArithmeticOverflow)?,
            tag,
        )?
    };

    Ok(MetricVariationTable {
        store,
        advance_map,
        side_bearing_map,
        third_map,
    })
}

fn parse_optional_delta_set_index_map(
    table: &[u8],
    offset: usize,
    tag: Tag,
) -> Result<Option<DeltaSetIndexMap>, SfntError> {
    if offset == 0 {
        return Ok(None);
    }
    parse_delta_set_index_map(table, offset, tag).map(Some)
}

fn parse_delta_set_index_map(
    table: &[u8],
    offset: usize,
    tag: Tag,
) -> Result<DeltaSetIndexMap, SfntError> {
    let reader = Reader::new(table);
    let format = reader.u8(offset).map_err(|_| malformed(tag))?;
    let entry_format = reader
        .u8(checked_add(offset, 1)?)
        .map_err(|_| malformed(tag))?;
    let entry_size = usize::from((entry_format >> 4 & 0x03) + 1);
    let inner_bits = usize::from((entry_format & 0x0f) + 1);
    if inner_bits > 16 || inner_bits > entry_size * 8 {
        return Err(malformed(tag));
    }
    let (map_count, map_data_offset) = match format {
        0 => (
            usize::from(
                reader
                    .u16(checked_add(offset, 2)?)
                    .map_err(|_| malformed(tag))?,
            ),
            checked_add(offset, 4)?,
        ),
        1 => (
            usize::try_from(
                reader
                    .u32(checked_add(offset, 2)?)
                    .map_err(|_| malformed(tag))?,
            )
            .map_err(|_| SfntError::ArithmeticOverflow)?,
            checked_add(offset, 6)?,
        ),
        _ => return Err(malformed(tag)),
    };
    if map_count > MAX_VARIATION_MAP_ENTRIES {
        return Err(malformed(tag));
    }
    if map_count == 0 {
        return Ok(DeltaSetIndexMap { entries: None });
    }
    let data_size = checked_mul(map_count, entry_size)?;
    let data = reader
        .range(map_data_offset, data_size)
        .map_err(|_| malformed(tag))?;
    let mut entries = Vec::with_capacity(map_count);
    for index in 0..map_count {
        let entry_offset = checked_mul(index, entry_size)?;
        let entry = match entry_size {
            1 => u32::from(data[entry_offset]),
            2 => u32::from(u16::from_be_bytes(
                data[entry_offset..entry_offset + 2]
                    .try_into()
                    .expect("validated two-byte map entry"),
            )),
            3 => {
                (u32::from(data[entry_offset]) << 16)
                    | (u32::from(data[entry_offset + 1]) << 8)
                    | u32::from(data[entry_offset + 2])
            }
            4 => u32::from_be_bytes(
                data[entry_offset..entry_offset + 4]
                    .try_into()
                    .expect("validated four-byte map entry"),
            ),
            _ => return Err(malformed(tag)),
        };
        let mask = (1_u32 << inner_bits) - 1;
        entries.push(DeltaSetIndex {
            outer: u16::try_from(entry >> inner_bits).unwrap_or(u16::MAX),
            inner: u16::try_from(entry & mask).unwrap_or(u16::MAX),
        });
    }
    Ok(DeltaSetIndexMap {
        entries: Some(entries),
    })
}

fn parse_item_variation_store(
    table: &[u8],
    offset: usize,
    axis_count: usize,
    tag: Tag,
) -> Result<ItemVariationStore, SfntError> {
    let store = Reader::new(table)
        .range(
            offset,
            table
                .len()
                .checked_sub(offset)
                .ok_or(SfntError::ArithmeticOverflow)?,
        )
        .map_err(|_| malformed(tag))?;
    let reader = Reader::new(store);
    if reader.u16(0).map_err(|_| malformed(tag))? != 1 {
        return Err(malformed(tag));
    }
    let region_offset = usize::try_from(reader.u32(2).map_err(|_| malformed(tag))?)
        .map_err(|_| SfntError::ArithmeticOverflow)?;
    let data_count = usize::from(reader.u16(6).map_err(|_| malformed(tag))?);
    if data_count > MAX_VARIATION_DATA_SUBTABLES {
        return Err(malformed(tag));
    }
    let offsets_size = checked_mul(data_count, 4)?;
    reader.range(8, offsets_size).map_err(|_| malformed(tag))?;

    let regions = parse_variation_regions(store, region_offset, axis_count, tag)?;
    let mut data = Vec::with_capacity(data_count);
    for index in 0..data_count {
        let data_offset = usize::try_from(
            reader
                .u32(checked_add(8, checked_mul(index, 4)?)?)
                .map_err(|_| malformed(tag))?,
        )
        .map_err(|_| SfntError::ArithmeticOverflow)?;
        if data_offset == 0 {
            data.push(VariationData {
                item_count: 0,
                region_indices: Vec::new(),
                deltas: Vec::new(),
            });
            continue;
        }
        data.push(parse_variation_data(store, data_offset, &regions, tag)?);
    }
    Ok(ItemVariationStore { tag, regions, data })
}

fn parse_variation_regions(
    store: &[u8],
    offset: usize,
    axis_count: usize,
    tag: Tag,
) -> Result<Vec<Vec<VariationRegionAxis>>, SfntError> {
    let reader = Reader::new(store);
    let region_axis_count = usize::from(reader.u16(offset).map_err(|_| malformed(tag))?);
    let region_count = usize::from(
        reader
            .u16(checked_add(offset, 2)?)
            .map_err(|_| malformed(tag))?,
    );
    if region_axis_count != axis_count || region_count > MAX_VARIATION_REGIONS {
        return Err(malformed(tag));
    }
    let region_size = checked_mul(region_axis_count, 6)?;
    let total_size = checked_add(4, checked_mul(region_count, region_size)?)?;
    reader
        .range(offset, total_size)
        .map_err(|_| malformed(tag))?;

    let mut regions = Vec::with_capacity(region_count);
    for region_index in 0..region_count {
        let region_offset = checked_add(
            offset,
            checked_add(4, checked_mul(region_index, region_size)?)?,
        )?;
        let mut region = Vec::with_capacity(region_axis_count);
        for axis_index in 0..region_axis_count {
            let axis_offset = checked_add(region_offset, checked_mul(axis_index, 6)?)?;
            let start = fixed_2_14(reader.i16(axis_offset).map_err(|_| malformed(tag))?);
            let peak = fixed_2_14(
                reader
                    .i16(checked_add(axis_offset, 2)?)
                    .map_err(|_| malformed(tag))?,
            );
            let end = fixed_2_14(
                reader
                    .i16(checked_add(axis_offset, 4)?)
                    .map_err(|_| malformed(tag))?,
            );
            if start > peak || peak > end {
                return Err(malformed(tag));
            }
            region.push(VariationRegionAxis { start, peak, end });
        }
        regions.push(region);
    }
    Ok(regions)
}

fn parse_variation_data(
    store: &[u8],
    offset: usize,
    regions: &[Vec<VariationRegionAxis>],
    tag: Tag,
) -> Result<VariationData, SfntError> {
    let reader = Reader::new(store);
    let item_count = usize::from(reader.u16(offset).map_err(|_| malformed(tag))?);
    let word_delta_count = reader
        .u16(checked_add(offset, 2)?)
        .map_err(|_| malformed(tag))?;
    let long_words = word_delta_count & 0x8000 != 0;
    let word_count = usize::from(word_delta_count & 0x7fff);
    let region_count = usize::from(
        reader
            .u16(checked_add(offset, 4)?)
            .map_err(|_| malformed(tag))?,
    );
    if word_count > region_count || region_count > regions.len() {
        return Err(malformed(tag));
    }
    let region_indices_offset = checked_add(offset, 6)?;
    let region_indices_size = checked_mul(region_count, 2)?;
    let delta_offset = checked_add(region_indices_offset, region_indices_size)?;
    reader
        .range(region_indices_offset, region_indices_size)
        .map_err(|_| malformed(tag))?;
    let row_size = if long_words {
        checked_add(
            checked_mul(word_count, 4)?,
            checked_mul(region_count - word_count, 2)?,
        )?
    } else {
        checked_add(checked_mul(word_count, 2)?, region_count - word_count)?
    };
    let total_values = checked_mul(item_count, region_count)?;
    if total_values > MAX_VARIATION_DELTA_VALUES {
        return Err(malformed(tag));
    }
    let total_delta_size = checked_mul(item_count, row_size)?;
    reader
        .range(delta_offset, total_delta_size)
        .map_err(|_| malformed(tag))?;

    let mut region_indices = Vec::with_capacity(region_count);
    for index in 0..region_count {
        let value = reader
            .u16(checked_add(region_indices_offset, checked_mul(index, 2)?)?)
            .map_err(|_| malformed(tag))?;
        if usize::from(value) >= regions.len() {
            return Err(malformed(tag));
        }
        region_indices.push(value);
    }

    let mut deltas = Vec::with_capacity(total_values);
    let mut cursor = delta_offset;
    for _ in 0..item_count {
        for region_index in 0..region_count {
            let delta = if region_index < word_count {
                if long_words {
                    let value = reader_i32(&reader, cursor).map_err(|_| malformed(tag))?;
                    cursor = checked_add(cursor, 4)?;
                    value
                } else {
                    let value = i32::from(reader.i16(cursor).map_err(|_| malformed(tag))?);
                    cursor = checked_add(cursor, 2)?;
                    value
                }
            } else if long_words {
                let value = i32::from(reader.i16(cursor).map_err(|_| malformed(tag))?);
                cursor = checked_add(cursor, 2)?;
                value
            } else {
                let value = i32::from(reader.i8(cursor).map_err(|_| malformed(tag))?);
                cursor = checked_add(cursor, 1)?;
                value
            };
            deltas.push(delta);
        }
    }
    Ok(VariationData {
        item_count,
        region_indices,
        deltas,
    })
}

fn region_scalar(region: &[VariationRegionAxis], coordinates: &[f32]) -> f32 {
    let mut scalar = 1.0;
    for (axis_index, axis) in region.iter().enumerate() {
        // A region axis that crosses zero does not constrain the region. This
        // is the OpenType "no variation along this axis" form.
        if axis.start < 0.0 && axis.end > 0.0 {
            continue;
        }
        let coordinate = coordinates.get(axis_index).copied().unwrap_or(0.0);
        if coordinate < axis.start || coordinate > axis.end {
            return 0.0;
        }
        if coordinate == axis.peak {
            continue;
        }
        if coordinate < axis.peak {
            if axis.peak == axis.start {
                return 0.0;
            }
            scalar *= (coordinate - axis.start) / (axis.peak - axis.start);
        } else {
            if axis.end == axis.peak {
                return 0.0;
            }
            scalar *= (axis.end - coordinate) / (axis.end - axis.peak);
        }
    }
    scalar.clamp(0.0, 1.0)
}

fn parse_gvar(table: &[u8], expected_axis_count: usize) -> Result<GvarInfo, SfntError> {
    let reader = Reader::new(table);
    if reader.u32(0)? != GVAR_VERSION {
        return Err(malformed(GVAR_TAG));
    }
    let axis_count = usize::from(reader.u16(4)?);
    if axis_count != expected_axis_count {
        return Err(malformed(GVAR_TAG));
    }
    let shared_tuple_count = usize::from(reader.u16(6)?);
    let shared_tuples_offset =
        usize::try_from(reader.u32(8)?).map_err(|_| SfntError::ArithmeticOverflow)?;
    let glyph_count = usize::from(reader.u16(12)?);
    if glyph_count > GVAR_MAX_GLYPHS {
        return Err(malformed(GVAR_TAG));
    }
    let flags = reader.u16(14)?;
    let data_offset =
        usize::try_from(reader.u32(16)?).map_err(|_| SfntError::ArithmeticOverflow)?;
    let offset_size = if flags & 1 != 0 { 4 } else { 2 };
    let offsets_size = checked_mul(
        glyph_count
            .checked_add(1)
            .ok_or(SfntError::ArithmeticOverflow)?,
        offset_size,
    )?;
    let offsets = reader.range(20, offsets_size)?;
    if data_offset > table.len() {
        return Err(malformed(GVAR_TAG));
    }

    let mut glyph_offsets = Vec::with_capacity(glyph_count + 1);
    for index in 0..=glyph_count {
        let offset = checked_mul(index, offset_size)?;
        let value = if offset_size == 4 {
            usize::try_from(Reader::new(offsets).u32(offset)?)
                .map_err(|_| SfntError::ArithmeticOverflow)?
        } else {
            checked_mul(usize::from(Reader::new(offsets).u16(offset)?), 2)?
        };
        glyph_offsets.push(value);
    }
    if glyph_offsets.windows(2).any(|window| window[0] > window[1])
        || glyph_offsets.last().copied().unwrap_or(0) > table.len() - data_offset
    {
        return Err(malformed(GVAR_TAG));
    }

    let shared_tuple_size = checked_mul(axis_count, 2)?;
    let shared_tuples_size = checked_mul(shared_tuple_count, shared_tuple_size)?;
    let shared_tuples_bytes = Reader::new(table).range(shared_tuples_offset, shared_tuples_size)?;
    let shared_reader = Reader::new(shared_tuples_bytes);
    let mut shared_tuples = Vec::with_capacity(shared_tuple_count);
    for tuple_index in 0..shared_tuple_count {
        let tuple_offset = checked_mul(tuple_index, shared_tuple_size)?;
        let mut tuple = Vec::with_capacity(axis_count);
        for axis in 0..axis_count {
            tuple.push(fixed_2_14(
                shared_reader.i16(checked_add(tuple_offset, checked_mul(axis, 2)?)?)?,
            ));
        }
        shared_tuples.push(tuple);
    }

    Ok(GvarInfo {
        axis_count,
        shared_tuples,
        glyph_offsets,
        data_offset,
    })
}

/// Applies the `wght` instance to a TrueType outline.
pub(crate) fn apply_gvar(
    face: &SfntFace<'_>,
    glyph_id: u16,
    weight: u16,
    outline: &mut GlyphOutline,
) -> Result<(), SfntError> {
    let (coordinates, coordinate_count) = face.coordinates_for_weight_instance(weight);
    apply_gvar_at_coordinates(face, glyph_id, &coordinates[..coordinate_count], outline).map(|_| ())
}

/// Applies a complete normalized variation instance to a TrueType outline,
/// including component-origin deltas for composite glyphs. Coordinates are
/// ordered according to `fvar` and use F2DOT14 precision, as produced by
/// [`SfntFace::normalized_variation_coordinates`].
/// Returns whether this glyph's own variation data recomputed its bounds.
pub(crate) fn apply_gvar_at_coordinates(
    face: &SfntFace<'_>,
    glyph_id: u16,
    coordinates: &[f32],
    outline: &mut GlyphOutline,
) -> Result<bool, SfntError> {
    let Some(info) = face.variation_info()? else {
        return Ok(false);
    };
    let Some(gvar) = info.gvar.as_ref() else {
        return Ok(false);
    };
    if coordinates
        .iter()
        .all(|coordinate| coordinate.abs() < f32::EPSILON)
    {
        return Ok(false);
    }

    let glyph_index = usize::from(glyph_id);
    let Some(start_offset) = gvar.glyph_offsets.get(glyph_index).copied() else {
        return Ok(false);
    };
    let Some(end_offset) = gvar.glyph_offsets.get(glyph_index + 1).copied() else {
        return Ok(false);
    };
    let start = checked_add(gvar.data_offset, start_offset)?;
    let end = checked_add(gvar.data_offset, end_offset)?;
    let table = face.table(*b"gvar").ok_or_else(|| malformed(GVAR_TAG))?;
    let data = table.get(start..end).ok_or_else(|| malformed(GVAR_TAG))?;
    if data.is_empty() {
        return Ok(false);
    }

    let mut deltas = Vec::new();
    let point_count = if outline.is_composite {
        outline.component_ranges.len()
    } else {
        outline.points.len()
    };
    if point_count > usize::from(u16::MAX) - 4 {
        return Err(malformed(GVAR_TAG));
    }
    let total_point_count = point_count + 4;
    decode_glyph_variations(
        data,
        gvar,
        coordinates,
        total_point_count,
        point_count,
        outline,
        &mut deltas,
    )?;

    if deltas.is_empty() {
        return Ok(false);
    }
    if outline.is_composite {
        for (component, delta) in outline.component_ranges.iter().zip(deltas) {
            for point in &mut outline.points[component.start..component.end] {
                point.x += delta.x;
                point.y += delta.y;
            }
        }
    } else {
        for (point, delta) in outline.points.iter_mut().zip(deltas) {
            point.x += delta.x;
            point.y += delta.y;
        }
    }
    update_bounds(outline);
    Ok(true)
}

fn decode_glyph_variations(
    data: &[u8],
    gvar: &GvarInfo,
    coordinates: &[f32],
    total_point_count: usize,
    outline_point_count: usize,
    outline: &GlyphOutline,
    output: &mut Vec<PointDelta>,
) -> Result<(), SfntError> {
    let reader = Reader::new(data);
    let tuple_count_flags = reader.u16(0)?;
    let tuple_count = usize::from(tuple_count_flags & TUPLE_COUNT_MASK);
    if tuple_count == 0 {
        return Ok(());
    }
    let serialized_offset = usize::from(reader.u16(2)?);
    if serialized_offset < 4 || serialized_offset > data.len() {
        return Err(malformed(GVAR_TAG));
    }

    let mut cursor = 4;
    let mut headers = Vec::with_capacity(tuple_count);
    for _ in 0..tuple_count {
        let data_size = usize::from(reader.u16(cursor)?);
        cursor = checked_add(cursor, 2)?;
        let tuple_index = reader.u16(cursor)?;
        cursor = checked_add(cursor, 2)?;

        let peak = if tuple_index & EMBEDDED_PEAK_TUPLE != 0 {
            Some(read_tuple(&reader, &mut cursor, gvar.axis_count)?)
        } else {
            None
        };
        let (intermediate_start, intermediate_end) = if tuple_index & INTERMEDIATE_REGION != 0 {
            (
                Some(read_tuple(&reader, &mut cursor, gvar.axis_count)?),
                Some(read_tuple(&reader, &mut cursor, gvar.axis_count)?),
            )
        } else {
            (None, None)
        };
        headers.push(TupleHeader {
            data_size,
            tuple_index,
            peak,
            intermediate_start,
            intermediate_end,
        });
    }
    if cursor > serialized_offset {
        return Err(malformed(GVAR_TAG));
    }

    let mut data_cursor = serialized_offset;
    let shared_points = if tuple_count_flags & SHARED_POINT_NUMBERS != 0 {
        let (points, consumed) = decode_packed_points(&data[data_cursor..])?;
        data_cursor = checked_add(data_cursor, consumed)?;
        points
    } else {
        None
    };

    for header in headers {
        let tuple_end = checked_add(data_cursor, header.data_size)?;
        let tuple_data = data
            .get(data_cursor..tuple_end)
            .ok_or_else(|| malformed(GVAR_TAG))?;
        data_cursor = tuple_end;

        let peak = header
            .peak
            .as_deref()
            .or_else(|| {
                let index = usize::from(header.tuple_index & TUPLE_INDEX_MASK);
                gvar.shared_tuples.get(index).map(Vec::as_slice)
            })
            .ok_or_else(|| malformed(GVAR_TAG))?;
        let Some(scalar) = tuple_scalar(
            peak,
            header.intermediate_start.as_deref(),
            header.intermediate_end.as_deref(),
            coordinates,
        ) else {
            continue;
        };

        let (points, point_data_offset) = if header.tuple_index & PRIVATE_POINT_NUMBERS != 0 {
            let (points, consumed) = decode_packed_points(tuple_data)?;
            (points, consumed)
        } else {
            (shared_points.clone(), 0)
        };
        let delta_count = points.as_ref().map_or(total_point_count, Vec::len);
        let delta_data = tuple_data
            .get(point_data_offset..)
            .ok_or_else(|| malformed(GVAR_TAG))?;
        let x_deltas = decode_packed_deltas(delta_data, delta_count)?;
        let y_offset = packed_delta_stream_len(delta_data, delta_count)?;
        let y_deltas = decode_packed_deltas(
            delta_data
                .get(y_offset..)
                .ok_or_else(|| malformed(GVAR_TAG))?,
            delta_count,
        )?;

        let mut tuple_deltas = vec![PointDelta::default(); outline_point_count];
        let mut present = vec![false; outline_point_count];
        match points {
            Some(points) => {
                for (index, point_index) in points.into_iter().enumerate() {
                    if point_index >= total_point_count {
                        return Err(malformed(GVAR_TAG));
                    }
                    if point_index < outline_point_count {
                        tuple_deltas[point_index] = PointDelta {
                            x: x_deltas[index] as f32 * scalar,
                            y: y_deltas[index] as f32 * scalar,
                        };
                        present[point_index] = true;
                    }
                }
                if !outline.is_composite {
                    interpolate_iup(outline, &mut tuple_deltas, &present);
                }
            }
            None => {
                for point_index in 0..outline_point_count {
                    tuple_deltas[point_index] = PointDelta {
                        x: x_deltas[point_index] as f32 * scalar,
                        y: y_deltas[point_index] as f32 * scalar,
                    };
                }
            }
        }

        if output.is_empty() {
            output.resize(outline_point_count, PointDelta::default());
        }
        for (output, delta) in output.iter_mut().zip(tuple_deltas) {
            output.x += delta.x;
            output.y += delta.y;
        }
    }
    Ok(())
}

fn read_tuple(
    reader: &Reader<'_>,
    cursor: &mut usize,
    axis_count: usize,
) -> Result<Vec<f32>, SfntError> {
    let mut tuple = Vec::with_capacity(axis_count);
    for _ in 0..axis_count {
        tuple.push(fixed_2_14(reader.i16(*cursor)?));
        *cursor = checked_add(*cursor, 2)?;
    }
    Ok(tuple)
}

fn tuple_scalar(
    peak: &[f32],
    intermediate_start: Option<&[f32]>,
    intermediate_end: Option<&[f32]>,
    coordinates: &[f32],
) -> Option<f32> {
    if peak.len() != coordinates.len()
        || intermediate_start.is_some_and(|start| start.len() != coordinates.len())
        || intermediate_end.is_some_and(|end| end.len() != coordinates.len())
    {
        return None;
    }

    let mut scalar = 1.0;
    for axis in 0..coordinates.len() {
        let coordinate = coordinates[axis];
        let peak = peak[axis];
        if peak == 0.0 || peak == coordinate {
            continue;
        }
        if coordinate == 0.0 {
            return None;
        }

        if let (Some(start), Some(end)) = (intermediate_start, intermediate_end) {
            let start = start[axis];
            let end = end[axis];
            if coordinate < start || coordinate > end || start > peak || peak > end {
                return None;
            }
            if coordinate < peak {
                if peak != start {
                    scalar *= (coordinate - start) / (peak - start);
                }
            } else if peak != end {
                scalar *= (end - coordinate) / (end - peak);
            }
        } else {
            if coordinate < peak.min(0.0) || coordinate > peak.max(0.0) {
                return None;
            }
            scalar *= coordinate / peak;
        }
    }
    (scalar.is_finite() && scalar > 0.0).then_some(scalar)
}

/// Reads packed point numbers. A zero count means the tuple is dense.
fn decode_packed_points(data: &[u8]) -> Result<(Option<Vec<usize>>, usize), SfntError> {
    let reader = Reader::new(data);
    let first = reader.u8(0)?;
    let (count, mut cursor) = if first == 0 {
        return Ok((None, 1));
    } else if first < 0x80 {
        (usize::from(first), 1)
    } else {
        (usize::from(reader.u16(0)? & 0x7fff), 2)
    };
    if count == 0 {
        return Ok((None, cursor));
    }

    let mut points = Vec::with_capacity(count);
    let mut point = 0_usize;
    while points.len() < count {
        let control = reader.u8(cursor)?;
        cursor = checked_add(cursor, 1)?;
        let run_count = usize::from(control & 0x7f) + 1;
        if points.len() + run_count > count {
            return Err(malformed(GVAR_TAG));
        }
        let two_bytes = control & 0x80 != 0;
        for _ in 0..run_count {
            let delta = if two_bytes {
                let value = usize::from(reader.u16(cursor)?);
                cursor = checked_add(cursor, 2)?;
                value
            } else {
                let value = usize::from(reader.u8(cursor)?);
                cursor = checked_add(cursor, 1)?;
                value
            };
            point = point
                .checked_add(delta)
                .ok_or(SfntError::ArithmeticOverflow)?;
            if points.last().is_some_and(|previous| point <= *previous) {
                return Err(malformed(GVAR_TAG));
            }
            points.push(point);
        }
    }
    Ok((Some(points), cursor))
}

fn decode_packed_deltas(data: &[u8], count: usize) -> Result<Vec<i32>, SfntError> {
    let reader = Reader::new(data);
    let mut cursor = 0;
    let mut deltas = Vec::with_capacity(count);
    while deltas.len() < count {
        let control = reader.u8(cursor)?;
        cursor = checked_add(cursor, 1)?;
        let run_count = usize::from(control & 0x3f) + 1;
        if deltas.len() + run_count > count {
            return Err(malformed(GVAR_TAG));
        }
        match control & 0xc0 {
            0x80 => deltas.extend(std::iter::repeat_n(0, run_count)),
            0x40 => {
                for _ in 0..run_count {
                    deltas.push(i32::from(reader.i16(cursor)?));
                    cursor = checked_add(cursor, 2)?;
                }
            }
            0xc0 => {
                for _ in 0..run_count {
                    deltas.push(reader_i32(&reader, cursor)?);
                    cursor = checked_add(cursor, 4)?;
                }
            }
            _ => {
                for _ in 0..run_count {
                    deltas.push(i32::from(reader.i8(cursor)?));
                    cursor = checked_add(cursor, 1)?;
                }
            }
        }
    }
    Ok(deltas)
}

fn packed_delta_stream_len(data: &[u8], count: usize) -> Result<usize, SfntError> {
    let reader = Reader::new(data);
    let mut cursor = 0;
    let mut seen = 0;
    while seen < count {
        let control = reader.u8(cursor)?;
        cursor = checked_add(cursor, 1)?;
        let run_count = usize::from(control & 0x3f) + 1;
        if seen + run_count > count {
            return Err(malformed(GVAR_TAG));
        }
        let element_size = match control & 0xc0 {
            0x80 => 0,
            0x40 => 2,
            0xc0 => 4,
            _ => 1,
        };
        cursor = checked_add(cursor, checked_mul(run_count, element_size)?)?;
        reader.range(0, cursor)?;
        seen += run_count;
    }
    Ok(cursor)
}

fn interpolate_iup(outline: &GlyphOutline, deltas: &mut [PointDelta], present: &[bool]) {
    for contour in &outline.contours {
        let count = contour.end - contour.start;
        let range = contour.start..contour.end;
        let explicit = (0..count)
            .filter(|index| present[contour.start + *index])
            .collect::<Vec<_>>();
        if explicit.is_empty() {
            continue;
        }
        if explicit.len() == 1 {
            let delta = deltas[contour.start + explicit[0]];
            for index in range {
                deltas[index] = delta;
            }
            continue;
        }

        for index in 0..count {
            if present[contour.start + index] {
                continue;
            }
            let previous = explicit
                .iter()
                .rev()
                .find(|&&candidate| candidate < index)
                .copied()
                .unwrap_or_else(|| *explicit.last().expect("explicit is non-empty"));
            let next = explicit
                .iter()
                .find(|&&candidate| candidate > index)
                .copied()
                .unwrap_or(explicit[0]);
            let point = outline.points[contour.start + index];
            let previous_point = outline.points[contour.start + previous];
            let next_point = outline.points[contour.start + next];
            let previous_delta = deltas[contour.start + previous];
            let next_delta = deltas[contour.start + next];
            deltas[contour.start + index] = PointDelta {
                x: interpolate_axis(
                    point.x,
                    previous_point.x,
                    next_point.x,
                    previous_delta.x,
                    next_delta.x,
                ),
                y: interpolate_axis(
                    point.y,
                    previous_point.y,
                    next_point.y,
                    previous_delta.y,
                    next_delta.y,
                ),
            };
        }
    }
}

fn interpolate_axis(
    value: f32,
    first: f32,
    second: f32,
    first_delta: f32,
    second_delta: f32,
) -> f32 {
    if first == second {
        return first_delta;
    }
    let (low, high, low_delta, high_delta) = if first < second {
        (first, second, first_delta, second_delta)
    } else {
        (second, first, second_delta, first_delta)
    };
    if value <= low {
        low_delta
    } else if value >= high {
        high_delta
    } else {
        low_delta + (high_delta - low_delta) * (value - low) / (high - low)
    }
}

pub(super) fn update_bounds(outline: &mut GlyphOutline) {
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for point in &outline.points {
        bounds[0] = bounds[0].min(point.x);
        bounds[1] = bounds[1].min(point.y);
        bounds[2] = bounds[2].max(point.x);
        bounds[3] = bounds[3].max(point.y);
    }
    if bounds[0].is_finite() {
        outline.bounds = [
            clamp_i16(bounds[0].floor()),
            clamp_i16(bounds[1].floor()),
            clamp_i16(bounds[2].ceil()),
            clamp_i16(bounds[3].ceil()),
        ];
    }
}

fn normalize_axis(axis: VariationAxis, value: f32) -> f32 {
    let value = value.clamp(axis.min, axis.max);
    let normalized = if value >= axis.default {
        let span = axis.max - axis.default;
        if span == 0.0 {
            0.0
        } else {
            (value - axis.default) / span
        }
    } else {
        let span = axis.default - axis.min;
        if span == 0.0 {
            0.0
        } else {
            (value - axis.default) / span
        }
    };
    (normalized * 16_384.0).round().clamp(-16_384.0, 16_384.0) / 16_384.0
}

fn normalize_weight(axis: VariationAxis, weight: f32) -> f32 {
    normalize_axis(axis, weight)
}

fn fixed_16_16(value: u32) -> f32 {
    i32::from_be_bytes(value.to_be_bytes()) as f32 / 65_536.0
}

fn fixed_2_14(value: i16) -> f32 {
    f32::from(value) / 16_384.0
}

fn reader_i32(reader: &Reader<'_>, offset: usize) -> Result<i32, SfntError> {
    Ok(i32::from_be_bytes(
        reader
            .range(offset, 4)?
            .try_into()
            .expect("range is four bytes"),
    ))
}

fn clamp_i16(value: f32) -> i16 {
    value.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

fn malformed(tag: Tag) -> SfntError {
    SfntError::MalformedTable(tag)
}

#[cfg(test)]
mod tests {
    use super::super::{SfntFace, Tag};
    use super::{VariationAxis, VariationInfo, coordinates_for_axes};

    #[test]
    fn reads_the_weight_axis_of_the_bundled_variable_cjk_face() {
        let bytes = include_bytes!("../../fonts/NotoSansJP-VariableFont_wght.ttf");
        let face = SfntFace::from_bytes(bytes, 0).expect("bundled CJK face should parse");
        let axis = face
            .weight_axis()
            .expect("variation metadata should parse")
            .expect("bundled face should have a weight axis");

        assert_eq!(axis.min, 100.0);
        assert_eq!(axis.default, 100.0);
        assert_eq!(axis.max, 900.0);
    }

    #[test]
    fn normalizes_arbitrary_axes_in_face_order_at_f2dot14_precision() {
        let info = VariationInfo {
            axes: vec![
                VariationAxis {
                    tag: Tag::from_bytes(*b"wght"),
                    min: 100.0,
                    default: 400.0,
                    max: 900.0,
                },
                VariationAxis {
                    tag: Tag::from_bytes(*b"wdth"),
                    min: 75.0,
                    default: 100.0,
                    max: 125.0,
                },
            ],
            weight_axis: Some(0),
            gvar: None,
            hvar: None,
            vvar: None,
        };

        let coordinates =
            coordinates_for_axes(&info, 700, &[(u32::from_be_bytes(*b"wdth"), 112.5)])
                .expect("known finite axes should normalize");

        assert_eq!(coordinates.len(), 2);
        assert!((coordinates[0] - 0.6).abs() < 0.0001);
        assert!((coordinates[1] - 0.5).abs() < 0.0001);
    }

    #[test]
    fn rejects_unknown_duplicate_and_non_finite_axis_requests() {
        let info = VariationInfo {
            axes: vec![VariationAxis {
                tag: Tag::from_bytes(*b"wdth"),
                min: 75.0,
                default: 100.0,
                max: 125.0,
            }],
            weight_axis: None,
            gvar: None,
            hvar: None,
            vvar: None,
        };

        assert!(
            coordinates_for_axes(&info, 400, &[(u32::from_be_bytes(*b"opsz"), 12.0)]).is_none()
        );
        assert!(
            coordinates_for_axes(
                &info,
                400,
                &[
                    (u32::from_be_bytes(*b"wdth"), 90.0),
                    (u32::from_be_bytes(*b"wdth"), 110.0),
                ]
            )
            .is_none()
        );
        assert!(
            coordinates_for_axes(&info, 400, &[(u32::from_be_bytes(*b"wdth"), f32::NAN)]).is_none()
        );
    }

    #[test]
    fn applies_the_regular_weight_to_a_default_thin_outline() {
        let bytes = include_bytes!("../../fonts/NotoSansJP-VariableFont_wght.ttf");
        let face = SfntFace::from_bytes(bytes, 0).expect("bundled CJK face should parse");
        let metrics = face.metrics().expect("metrics should parse");
        let glyph_id = face
            .glyph_index('你' as u32)
            .expect("cmap should parse")
            .expect("the CJK face should cover 你");
        let default = face
            .outline_with_metrics(glyph_id, metrics)
            .expect("default outline should parse")
            .expect("你 should have an outline");
        let regular = face
            .outline_with_metrics_at_weight(glyph_id, metrics, 400)
            .expect("regular outline should parse")
            .expect("你 should have a regular outline");

        assert_ne!(default.points, regular.points);
    }

    #[test]
    fn applies_the_heavy_weight_to_the_google_sans_t_outline() {
        let bytes = include_bytes!("../../fonts/GoogleSans-VariableFont_GRAD,opsz,wght.ttf");
        let face = SfntFace::from_bytes(bytes, 0).expect("Google Sans should parse");
        let metrics = face.metrics().expect("metrics should parse");
        let glyph_id = face
            .glyph_index('t' as u32)
            .expect("cmap should parse")
            .expect("Google Sans should cover t");
        let regular = face
            .outline_with_metrics_at_weight(glyph_id, metrics, 400)
            .expect("regular outline should parse")
            .expect("t should have an outline");
        let heavy = face
            .outline_with_metrics_at_weight(glyph_id, metrics, 900)
            .expect("heavy outline should parse")
            .expect("t should have a heavy outline");

        assert_ne!(regular.points, heavy.points);
    }

    #[test]
    fn composite_google_sans_t_uses_varied_child_shapes() {
        let bytes = include_bytes!("../../fonts/GoogleSans-VariableFont_GRAD,opsz,wght.ttf");
        let face = SfntFace::from_bytes(bytes, 0).expect("Google Sans should parse");
        let metrics = face.metrics().expect("metrics should parse");
        let glyph_id = face
            .glyph_index('t' as u32)
            .expect("cmap should parse")
            .expect("Google Sans should cover t");
        let heavy = face
            .outline_with_metrics_at_weight(glyph_id, metrics, 900)
            .expect("heavy outline should parse")
            .expect("t should have a heavy outline");

        // The fixture's t is built from these three unscaled glyphs. A
        // composite gvar delta may move a component, but must not leave its
        // internal shape at the default weight.
        let components = [7490, 2350, 7492];
        assert_eq!(heavy.component_ranges.len(), components.len());
        for (range, glyph_id) in heavy.component_ranges.iter().zip(components) {
            let child = face
                .outline_with_metrics_at_weight(glyph_id, metrics, 900)
                .expect("heavy component should parse")
                .expect("component should have an outline");
            let points = &heavy.points[range.start..range.end];
            assert_eq!(points.len(), child.points.len());
            let origin = points[0];
            let child_origin = child.points[0];
            for (point, child_point) in points.iter().zip(&child.points) {
                assert!((point.x - origin.x - (child_point.x - child_origin.x)).abs() < 0.01);
                assert!((point.y - origin.y - (child_point.y - child_origin.y)).abs() < 0.01);
                assert_eq!(point.on_curve, child_point.on_curve);
            }
        }
    }
}
