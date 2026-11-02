use super::{Reader, SfntError, SfntFace, Tag, checked_add, checked_mul};

const COLR_TAG: Tag = Tag(*b"COLR");
const CPAL_TAG: Tag = Tag(*b"CPAL");
const FOREGROUND_PALETTE_INDEX: u16 = u16::MAX;
const MAX_BASE_COLOR_GLYPHS: usize = 1 << 16;
const MAX_COLOR_LAYERS: usize = 1 << 20;
const MAX_PALETTE_ENTRIES: usize = 1 << 12;
const MAX_PALETTES: usize = 1 << 12;
const MAX_COLOR_RECORDS: usize = 1 << 20;

/// One COLR v0 layer in back-to-front paint order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ColorLayer {
    pub(crate) glyph_id: u16,
    pub(crate) palette_index: u16,
}

/// One CPAL palette color in the byte order expected by the RGBA atlas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ColorRgba {
    pub(crate) red: u8,
    pub(crate) green: u8,
    pub(crate) blue: u8,
    pub(crate) alpha: u8,
}

impl ColorRgba {
    pub(crate) const fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    fn from_bgra(bytes: [u8; 4]) -> Self {
        Self::new(bytes[2], bytes[1], bytes[0], bytes[3])
    }
}

#[derive(Clone, Copy)]
struct BaseColorGlyph {
    glyph_id: u16,
    first_layer: usize,
    layer_count: usize,
}

#[derive(Clone)]
pub(crate) struct ColorTables {
    base_glyphs: Vec<BaseColorGlyph>,
    layers: Vec<ColorLayer>,
    default_palette: Vec<ColorRgba>,
}

impl ColorTables {
    pub(crate) fn layers(&self, glyph_id: u16) -> Option<&[ColorLayer]> {
        let base = self
            .base_glyphs
            .binary_search_by_key(&glyph_id, |base| base.glyph_id)
            .ok()
            .map(|index| self.base_glyphs[index])?;
        self.layers
            .get(base.first_layer..base.first_layer + base.layer_count)
    }

    pub(crate) fn palette_color(&self, palette_index: u16) -> Option<ColorRgba> {
        if palette_index == FOREGROUND_PALETTE_INDEX {
            // The rasterizer has no per-glyph foreground argument yet. A black
            // foreground matches the portable default used by the legacy color
            // path and keeps COLR foreground layers deterministic.
            return Some(ColorRgba::new(0, 0, 0, 255));
        }
        self.default_palette
            .get(usize::from(palette_index))
            .copied()
    }
}

/// Parses the owned subset of the OpenType color model.
///
/// COLR v0 and the first CPAL palette are deliberately parsed as borrowed,
/// immutable state. COLR v1, malformed tables, and faces without a usable
/// palette return no owned color representation so the caller can use its
/// compatibility fallback.
pub(crate) fn parse(face: &SfntFace<'_>) -> Result<Option<ColorTables>, SfntError> {
    let Some(colr) = face.table(*b"COLR") else {
        return Ok(None);
    };
    let Some(cpal) = face.table(*b"CPAL") else {
        return Ok(None);
    };

    let Some((base_glyphs, layers)) = parse_colr_v0(colr)? else {
        return Ok(None);
    };
    let Some(default_palette) = parse_cpal_default_palette(cpal)? else {
        return Ok(None);
    };

    for layer in &layers {
        if layer.palette_index != FOREGROUND_PALETTE_INDEX
            && usize::from(layer.palette_index) >= default_palette.len()
        {
            return Err(SfntError::MalformedTable(CPAL_TAG));
        }
    }

    Ok(Some(ColorTables {
        base_glyphs,
        layers,
        default_palette,
    }))
}

fn parse_colr_v0(
    table: &[u8],
) -> Result<Option<(Vec<BaseColorGlyph>, Vec<ColorLayer>)>, SfntError> {
    let reader = Reader::new(table);
    if table.len() < 14 {
        return Err(SfntError::MalformedTable(COLR_TAG));
    }
    // Version 1 has a different base/layer representation. It is recognized
    // by the directory but intentionally left to the compatibility renderer.
    if reader.u16(0)? != 0 {
        return Ok(None);
    }

    let base_count = usize::from(reader.u16(2)?);
    let base_offset = usize::try_from(reader.u32(4)?).map_err(|_| SfntError::ArithmeticOverflow)?;
    let layer_offset =
        usize::try_from(reader.u32(8)?).map_err(|_| SfntError::ArithmeticOverflow)?;
    let layer_count = usize::from(reader.u16(12)?);

    if base_count > MAX_BASE_COLOR_GLYPHS || layer_count > MAX_COLOR_LAYERS {
        return Err(SfntError::MalformedTable(COLR_TAG));
    }
    reader
        .range(base_offset, checked_mul(base_count, 6)?)
        .map_err(|_| SfntError::MalformedTable(COLR_TAG))?;
    reader
        .range(layer_offset, checked_mul(layer_count, 4)?)
        .map_err(|_| SfntError::MalformedTable(COLR_TAG))?;

    let mut base_glyphs = Vec::with_capacity(base_count);
    let mut previous_glyph = None;
    for index in 0..base_count {
        let offset = checked_add(base_offset, checked_mul(index, 6)?)?;
        let glyph_id = reader.u16(offset)?;
        let first_layer = usize::from(reader.u16(checked_add(offset, 2)?)?);
        let layer_count = usize::from(reader.u16(checked_add(offset, 4)?)?);
        let layer_end = checked_add(first_layer, layer_count)?;
        if layer_count == 0 || layer_end > layer_count {
            return Err(SfntError::MalformedTable(COLR_TAG));
        }
        if previous_glyph.is_some_and(|previous| glyph_id <= previous) {
            return Err(SfntError::MalformedTable(COLR_TAG));
        }
        previous_glyph = Some(glyph_id);
        base_glyphs.push(BaseColorGlyph {
            glyph_id,
            first_layer,
            layer_count,
        });
    }

    let mut layers = Vec::with_capacity(layer_count);
    for index in 0..layer_count {
        let offset = checked_add(layer_offset, checked_mul(index, 4)?)?;
        layers.push(ColorLayer {
            glyph_id: reader.u16(offset)?,
            palette_index: reader.u16(checked_add(offset, 2)?)?,
        });
    }
    Ok(Some((base_glyphs, layers)))
}

fn parse_cpal_default_palette(table: &[u8]) -> Result<Option<Vec<ColorRgba>>, SfntError> {
    let reader = Reader::new(table);
    if table.len() < 12 {
        return Err(SfntError::MalformedTable(CPAL_TAG));
    }
    let version = reader.u16(0)?;
    if version > 1 {
        return Ok(None);
    }
    let palette_entries = usize::from(reader.u16(2)?);
    let palette_count = usize::from(reader.u16(4)?);
    let color_record_count = usize::from(reader.u16(6)?);
    let color_records_offset =
        usize::try_from(reader.u32(8)?).map_err(|_| SfntError::ArithmeticOverflow)?;
    if palette_entries == 0
        || palette_entries > MAX_PALETTE_ENTRIES
        || palette_count == 0
        || palette_count > MAX_PALETTES
        || color_record_count > MAX_COLOR_RECORDS
    {
        return Err(SfntError::MalformedTable(CPAL_TAG));
    }
    reader
        .range(12, checked_mul(palette_count, 2)?)
        .map_err(|_| SfntError::MalformedTable(CPAL_TAG))?;
    let palette_start = usize::from(reader.u16(12)?);
    let palette_end = checked_add(palette_start, palette_entries)?;
    if palette_end > color_record_count {
        return Err(SfntError::MalformedTable(CPAL_TAG));
    }
    reader
        .range(color_records_offset, checked_mul(color_record_count, 4)?)
        .map_err(|_| SfntError::MalformedTable(CPAL_TAG))?;

    let mut palette = Vec::with_capacity(palette_entries);
    for index in palette_start..palette_end {
        let offset = checked_add(color_records_offset, checked_mul(index, 4)?)?;
        palette.push(ColorRgba::from_bgra([
            reader.u8(offset)?,
            reader.u8(checked_add(offset, 1)?)?,
            reader.u8(checked_add(offset, 2)?)?,
            reader.u8(checked_add(offset, 3)?)?,
        ]));
    }
    Ok(Some(palette))
}
