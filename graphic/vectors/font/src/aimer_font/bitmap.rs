use std::io::Cursor;

use image::{ImageReader, Limits, imageops::FilterType};

use super::{RasterizedGlyph, Reader, SfntError, SfntFace, checked_add, checked_mul};

const SBIX_TAG: [u8; 4] = *b"sbix";
const CBLC_TAG: [u8; 4] = *b"CBLC";
const CBDT_TAG: [u8; 4] = *b"CBDT";

const MAX_BITMAP_STRIKES: usize = 64;
const MAX_INDEX_SUBTABLES: usize = 1 << 16;
const MAX_SBIT_RECORDS: usize = 1 << 20;
const MAX_BITMAP_PAYLOAD: usize = 64 * 1024 * 1024;
const MAX_BITMAP_DIMENSION: u32 = 4096;
const MAX_DECODED_BITMAP_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DUPE_DEPTH: usize = 8;

/// Returns the `sbix` bitmap top edge in strike pixels.
///
/// `origin_y` is the top edge supplied by the table. Apple Color Emoji uses a
/// zero origin as a sentinel and needs a small baseline correction; the
/// caller supplies the face/strike context required to apply that rule.
#[inline]
fn sbix_bitmap_top(
    origin_y: i16,
    image_height: u32,
    strike_ppem: u16,
    units_per_em: u16,
    apple_color_emoji: bool,
) -> f32 {
    if apple_color_emoji && origin_y == 0 && strike_ppem != 0 && units_per_em != 0 {
        // Apple Color Emoji encodes a zero `sbix` origin as a sentinel. Native
        // readers replace it with -100 font units before adding the bitmap
        // height, which keeps the artwork just below the em-box top instead
        // of placing its top edge on the text baseline.
        return image_height as f32 - 100.0 * f32::from(strike_ppem) / f32::from(units_per_em);
    }
    f32::from(origin_y)
}

/// Parsed bitmap-color tables retained by one validated SFNT face.
///
/// The table index is immutable and cheap to share. Encoded image bytes remain
/// borrowed from the face and are decoded only for a requested glyph/strike.
pub(crate) struct BitmapTables {
    sbix: Option<SbixTable>,
    cbdt: Option<CbdtTable>,
}

impl BitmapTables {
    /// Reports whether a validated `sbix` index contains Apple's private
    /// `emjc` graphic type. The payload remains opaque and is never passed to
    /// the public image decoder.
    pub(crate) fn has_private_color_data(&self) -> bool {
        self.sbix
            .as_ref()
            .is_some_and(|table| table.has_private_graphics)
    }

    pub(crate) fn rasterize(
        &self,
        face: &SfntFace<'_>,
        glyph_id: u16,
        font_size: f32,
        advance_width: f32,
    ) -> Option<RasterizedGlyph> {
        if let Some(table) = &self.sbix
            && let Some(bytes) = face.table(SBIX_TAG)
            && let Some(glyph) = table.rasterize(bytes, glyph_id, font_size, advance_width)
        {
            return Some(glyph);
        }
        if let Some(table) = &self.cbdt
            && let Some(bytes) = face.table(CBDT_TAG)
        {
            return table.rasterize(bytes, glyph_id, font_size, advance_width);
        }
        None
    }
}

/// Parses the supported bitmap-color table indexes without decoding any image.
pub(crate) fn parse(face: &SfntFace<'_>) -> Result<Option<BitmapTables>, SfntError> {
    let has_sbix = face.table(SBIX_TAG).is_some();
    let has_cbdt = face.table(CBLC_TAG).is_some() && face.table(CBDT_TAG).is_some();
    if !has_sbix && !has_cbdt {
        return Ok(None);
    }
    let metrics = face.metrics()?;
    let apple_color_emoji = face
        .family_name()
        .ok()
        .flatten()
        .is_some_and(|name| name == "Apple Color Emoji");
    let sbix = face
        .table(SBIX_TAG)
        .map(|table| {
            parse_sbix(
                table,
                metrics.num_glyphs,
                metrics.units_per_em,
                apple_color_emoji,
            )
        })
        .transpose()?
        .flatten();
    let cbdt = match (face.table(CBLC_TAG), face.table(CBDT_TAG)) {
        (Some(cblc), Some(cbdt)) => parse_cbdt(cblc, cbdt, metrics.num_glyphs)?,
        _ => None,
    };

    Ok(Some(BitmapTables { sbix, cbdt }))
}

struct SbixTable {
    strikes: Box<[SbixStrike]>,
    has_private_graphics: bool,
    apple_color_emoji: bool,
    units_per_em: u16,
}

struct SbixStrike {
    ppem: u16,
    offset: usize,
    glyph_offsets: Box<[u32]>,
}

fn parse_sbix(
    table: &[u8],
    num_glyphs: u16,
    units_per_em: u16,
    apple_color_emoji: bool,
) -> Result<Option<SbixTable>, SfntError> {
    let reader = Reader::new(table);
    let version = reader.u16(0).map_err(|_| malformed(SBIX_TAG))?;
    if version > 1 {
        return Ok(None);
    }
    let strike_count = usize::try_from(reader.u32(4).map_err(|_| malformed(SBIX_TAG))?)
        .map_err(|_| SfntError::ArithmeticOverflow)?;
    if strike_count == 0 {
        return Ok(None);
    }
    if strike_count > MAX_BITMAP_STRIKES {
        return Err(malformed(SBIX_TAG));
    }
    let strike_offsets_size = checked_mul(strike_count, 4)?;
    reader
        .range(8, strike_offsets_size)
        .map_err(|_| malformed(SBIX_TAG))?;

    let mut offsets = Vec::with_capacity(strike_count);
    for index in 0..strike_count {
        let offset = checked_add(8, checked_mul(index, 4)?)?;
        offsets.push(
            usize::try_from(reader.u32(offset).map_err(|_| malformed(SBIX_TAG))?)
                .map_err(|_| SfntError::ArithmeticOverflow)?,
        );
    }

    let glyph_count = usize::from(num_glyphs);
    let offset_count = checked_add(glyph_count, 1)?;
    let offsets_size = checked_mul(offset_count, 4)?;
    let mut strikes: Vec<SbixStrike> = Vec::with_capacity(strike_count);
    let mut has_private_graphics = false;
    let mut total_offsets = 0usize;
    for index in 0..strike_count {
        let strike_offset = offsets[index];
        let strike_end = offsets.get(index + 1).copied().unwrap_or(table.len());
        if strike_offset < 8
            || strike_offset >= strike_end
            || strike_end > table.len()
            || index > 0 && strike_offset <= strikes[index - 1].offset
        {
            return Err(malformed(SBIX_TAG));
        }
        let reader = Reader::new(table);
        let ppem = reader.u16(strike_offset).map_err(|_| malformed(SBIX_TAG))?;
        let offset_array = checked_add(strike_offset, 4)?;
        let offset_array_end = checked_add(offset_array, offsets_size)?;
        if ppem == 0 || offset_array_end > strike_end {
            return Err(malformed(SBIX_TAG));
        }
        total_offsets = total_offsets
            .checked_add(offset_count)
            .ok_or(SfntError::ArithmeticOverflow)?;
        if total_offsets > MAX_SBIT_RECORDS {
            return Err(malformed(SBIX_TAG));
        }

        let strike_data_len = strike_end - strike_offset;
        let minimum_data_offset = offset_array_end - strike_offset;
        let mut glyph_offsets = Vec::with_capacity(offset_count);
        for glyph in 0..offset_count {
            let offset = checked_add(offset_array, checked_mul(glyph, 4)?)?;
            let value = reader.u32(offset).map_err(|_| malformed(SBIX_TAG))?;
            let value_usize = usize::try_from(value).map_err(|_| SfntError::ArithmeticOverflow)?;
            if value_usize > strike_data_len {
                return Err(malformed(SBIX_TAG));
            }
            glyph_offsets.push(value);
        }
        for pair in glyph_offsets.windows(2) {
            let start = usize::try_from(pair[0]).map_err(|_| SfntError::ArithmeticOverflow)?;
            let end = usize::try_from(pair[1]).map_err(|_| SfntError::ArithmeticOverflow)?;
            if start > end
                || (start != end && (start < minimum_data_offset || end < minimum_data_offset))
            {
                return Err(malformed(SBIX_TAG));
            }
            if start != end {
                let record_start = checked_add(strike_offset, start)?;
                let record_end = checked_add(strike_offset, end)?;
                if table
                    .get(record_start..record_end)
                    .is_some_and(|record| record.get(4..8) == Some(b"emjc"))
                {
                    has_private_graphics = true;
                }
            }
        }

        strikes.push(SbixStrike {
            ppem,
            offset: strike_offset,
            glyph_offsets: glyph_offsets.into_boxed_slice(),
        });
    }

    Ok(Some(SbixTable {
        strikes: strikes.into_boxed_slice(),
        has_private_graphics,
        apple_color_emoji,
        units_per_em,
    }))
}

impl SbixTable {
    fn rasterize(
        &self,
        table: &[u8],
        glyph_id: u16,
        font_size: f32,
        advance_width: f32,
    ) -> Option<RasterizedGlyph> {
        if !font_size.is_finite() || font_size <= 0.0 {
            return None;
        }
        // The nearest strike that has this glyph at all: a font may add a glyph
        // to only some of its sizes.
        let strike = best_strike_where(
            &self.strikes,
            font_size,
            |strike| strike.ppem,
            |strike| strike.image(table, glyph_id, 0).is_some(),
        )?;
        let image = strike.image(table, glyph_id, 0)?;
        let decoded = decode_image(image.kind, image.bytes)?;
        let scale = font_size / f32::from(strike.ppem);
        let top = sbix_bitmap_top(
            image.origin_y,
            decoded.height,
            strike.ppem,
            self.units_per_em,
            self.apple_color_emoji,
        );
        rasterize_decoded_bitmap(
            decoded,
            scale,
            scale,
            f32::from(image.origin_x),
            top,
            advance_width,
        )
    }
}

struct SbixImage<'a> {
    origin_x: i16,
    origin_y: i16,
    kind: EmbeddedImageKind,
    bytes: &'a [u8],
}

#[derive(Clone, Copy)]
enum EmbeddedImageKind {
    Png,
    Jpeg,
    Tiff,
}

impl SbixStrike {
    fn image<'a>(&self, table: &'a [u8], glyph_id: u16, depth: usize) -> Option<SbixImage<'a>> {
        if depth > MAX_DUPE_DEPTH {
            return None;
        }
        let index = usize::from(glyph_id);
        let start = usize::try_from(*self.glyph_offsets.get(index)?).ok()?;
        let end = usize::try_from(*self.glyph_offsets.get(index + 1)?).ok()?;
        if start == end {
            return None;
        }
        let record_start = self.offset.checked_add(start)?;
        let record_end = self.offset.checked_add(end)?;
        let record = table.get(record_start..record_end)?;
        if record.len() < 8 {
            return None;
        }
        let origin_x = i16::from_be_bytes([record[0], record[1]]);
        let origin_y = i16::from_be_bytes([record[2], record[3]]);
        let kind = [record[4], record[5], record[6], record[7]];
        let payload = &record[8..];
        if kind == *b"dupe" {
            let target = u16::from_be_bytes([*payload.first()?, *payload.get(1)?]);
            let mut image = self.image(table, target, depth + 1)?;
            image.origin_x = origin_x;
            image.origin_y = origin_y;
            return Some(image);
        }

        let kind = match kind {
            [b'p', b'n', b'g', b' '] => EmbeddedImageKind::Png,
            [b'j', b'p', b'g', b' '] => EmbeddedImageKind::Jpeg,
            [b't', b'i', b'f', b'f'] => EmbeddedImageKind::Tiff,
            _ => return None,
        };
        Some(SbixImage {
            origin_x,
            origin_y,
            kind,
            bytes: payload,
        })
    }
}

struct CbdtTable {
    strikes: Box<[CbdtStrike]>,
}

struct CbdtStrike {
    ppem_x: u8,
    ppem_y: u8,
    subtables: Box<[CbdtIndexSubtable]>,
}

struct CbdtIndexSubtable {
    first_glyph: u16,
    last_glyph: u16,
    image_format: u16,
    image_data_offset: usize,
    entries: CbdtEntries,
}

enum CbdtEntries {
    Range {
        offsets: Box<[u32]>,
        metrics: Option<BitmapMetrics>,
    },
    Fixed {
        image_size: u32,
        glyph_ids: Option<Box<[u16]>>,
        metrics: BitmapMetrics,
    },
    Sparse {
        glyph_ids: Box<[u16]>,
        offsets: Box<[u32]>,
    },
}

#[derive(Clone, Copy)]
struct BitmapMetrics {
    width: u32,
    height: u32,
    bearing_x: i16,
    bearing_y: i16,
}

fn parse_cbdt(cblc: &[u8], cbdt: &[u8], num_glyphs: u16) -> Result<Option<CbdtTable>, SfntError> {
    let reader = Reader::new(cblc);
    let version = reader.u32(0).map_err(|_| malformed(CBLC_TAG))?;
    if version != 0x0002_0000 && version != 0x0003_0000 {
        return Ok(None);
    }
    let strike_count = usize::try_from(reader.u32(4).map_err(|_| malformed(CBLC_TAG))?)
        .map_err(|_| SfntError::ArithmeticOverflow)?;
    if strike_count == 0 {
        return Ok(None);
    }
    if strike_count > MAX_BITMAP_STRIKES {
        return Err(malformed(CBLC_TAG));
    }
    let size_table_bytes = checked_mul(strike_count, 48)?;
    let size_table_start = 8usize;
    reader
        .range(size_table_start, size_table_bytes)
        .map_err(|_| malformed(CBLC_TAG))?;

    let glyph_limit = usize::from(num_glyphs);
    let mut strikes = Vec::with_capacity(strike_count);
    let mut total_subtables = 0usize;
    for strike_index in 0..strike_count {
        let base = checked_add(size_table_start, checked_mul(strike_index, 48)?)?;
        let array_offset = usize::try_from(reader.u32(base).map_err(|_| malformed(CBLC_TAG))?)
            .map_err(|_| SfntError::ArithmeticOverflow)?;
        let array_size = usize::try_from(
            reader
                .u32(checked_add(base, 4)?)
                .map_err(|_| malformed(CBLC_TAG))?,
        )
        .map_err(|_| SfntError::ArithmeticOverflow)?;
        let subtable_count = usize::try_from(
            reader
                .u32(checked_add(base, 8)?)
                .map_err(|_| malformed(CBLC_TAG))?,
        )
        .map_err(|_| SfntError::ArithmeticOverflow)?;
        let start_glyph = reader
            .u16(checked_add(base, 16)?)
            .map_err(|_| malformed(CBLC_TAG))?;
        let end_glyph = reader
            .u16(checked_add(base, 18)?)
            .map_err(|_| malformed(CBLC_TAG))?;
        let ppem_x = reader
            .u8(checked_add(base, 20)?)
            .map_err(|_| malformed(CBLC_TAG))?;
        let ppem_y = reader
            .u8(checked_add(base, 21)?)
            .map_err(|_| malformed(CBLC_TAG))?;
        if ppem_x == 0
            || ppem_y == 0
            || start_glyph > end_glyph
            || usize::from(end_glyph) >= glyph_limit
            || subtable_count > MAX_INDEX_SUBTABLES
        {
            return Err(malformed(CBLC_TAG));
        }
        let array_records_size = checked_mul(subtable_count, 8)?;
        if array_size < array_records_size {
            return Err(malformed(CBLC_TAG));
        }
        let array_end = checked_add(array_offset, array_size)?;
        reader
            .range(array_offset, array_size)
            .map_err(|_| malformed(CBLC_TAG))?;

        total_subtables = total_subtables
            .checked_add(subtable_count)
            .ok_or(SfntError::ArithmeticOverflow)?;
        if total_subtables > MAX_INDEX_SUBTABLES {
            return Err(malformed(CBLC_TAG));
        }

        let mut records = Vec::with_capacity(subtable_count);
        let mut previous_last = None;
        for index in 0..subtable_count {
            let record = checked_add(array_offset, checked_mul(index, 8)?)?;
            let first = reader.u16(record).map_err(|_| malformed(CBLC_TAG))?;
            let last = reader
                .u16(checked_add(record, 2)?)
                .map_err(|_| malformed(CBLC_TAG))?;
            let additional_offset = usize::try_from(
                reader
                    .u32(checked_add(record, 4)?)
                    .map_err(|_| malformed(CBLC_TAG))?,
            )
            .map_err(|_| SfntError::ArithmeticOverflow)?;
            if first > last
                || first < start_glyph
                || last > end_glyph
                || previous_last.is_some_and(|previous| first <= previous)
            {
                return Err(malformed(CBLC_TAG));
            }
            let subtable_offset = checked_add(array_offset, additional_offset)?;
            if subtable_offset < checked_add(array_offset, array_records_size)?
                || subtable_offset >= array_end
            {
                return Err(malformed(CBLC_TAG));
            }
            previous_last = Some(last);
            records.push((first, last, subtable_offset));
        }

        let mut subtables = Vec::with_capacity(records.len());
        for (first, last, subtable_offset) in records {
            if let Some(subtable) =
                parse_index_subtable(cblc, cbdt, first, last, subtable_offset, array_end)?
            {
                subtables.push(subtable);
            }
        }
        if !subtables.is_empty() {
            strikes.push(CbdtStrike {
                ppem_x,
                ppem_y,
                subtables: subtables.into_boxed_slice(),
            });
        }
    }

    if strikes.is_empty() {
        return Ok(None);
    }
    Ok(Some(CbdtTable {
        strikes: strikes.into_boxed_slice(),
    }))
}

fn parse_index_subtable(
    cblc: &[u8],
    cbdt: &[u8],
    first_glyph: u16,
    last_glyph: u16,
    offset: usize,
    table_end: usize,
) -> Result<Option<CbdtIndexSubtable>, SfntError> {
    let reader = Reader::new(cblc);
    let index_format = reader.u16(offset).map_err(|_| malformed(CBLC_TAG))?;
    let image_format = reader
        .u16(checked_add(offset, 2)?)
        .map_err(|_| malformed(CBLC_TAG))?;
    if !matches!(image_format, 17..=19) {
        return Ok(None);
    }
    let image_data_offset = usize::try_from(
        reader
            .u32(checked_add(offset, 4)?)
            .map_err(|_| malformed(CBLC_TAG))?,
    )
    .map_err(|_| SfntError::ArithmeticOverflow)?;
    if image_data_offset > cbdt.len() {
        return Err(malformed(CBLC_TAG));
    }
    let count = checked_add(usize::from(last_glyph) - usize::from(first_glyph), 1)?;

    let entries = match index_format {
        1 => {
            let offsets = read_u32_offsets(
                &reader,
                checked_add(offset, 8)?,
                checked_add(count, 1)?,
                table_end,
            )?;
            CbdtEntries::Range {
                offsets: offsets.into_boxed_slice(),
                metrics: None,
            }
        }
        2 => {
            let image_size = reader
                .u32(checked_add(offset, 8)?)
                .map_err(|_| malformed(CBLC_TAG))?;
            ensure_range(cblc, offset, 20, table_end)?;
            let metrics = read_big_metrics(&reader, checked_add(offset, 12)?)?;
            if image_size == 0
                || usize::try_from(image_size)
                    .ok()
                    .is_none_or(|size| size > MAX_BITMAP_PAYLOAD)
            {
                return Err(malformed(CBLC_TAG));
            }
            CbdtEntries::Fixed {
                image_size,
                glyph_ids: None,
                metrics,
            }
        }
        3 => {
            let offsets = read_u16_offsets(
                &reader,
                checked_add(offset, 8)?,
                checked_add(count, 1)?,
                table_end,
            )?;
            CbdtEntries::Range {
                offsets: offsets.into_boxed_slice(),
                metrics: None,
            }
        }
        4 => {
            let sparse_count = usize::try_from(
                reader
                    .u32(checked_add(offset, 8)?)
                    .map_err(|_| malformed(CBLC_TAG))?,
            )
            .map_err(|_| SfntError::ArithmeticOverflow)?;
            if sparse_count == 0 || sparse_count > MAX_SBIT_RECORDS {
                return Err(malformed(CBLC_TAG));
            }
            let glyph_ids_offset = checked_add(offset, 12)?;
            let offsets_offset = checked_add(glyph_ids_offset, checked_mul(sparse_count, 2)?)?;
            let offsets_count = checked_add(sparse_count, 1)?;
            let offsets_end = checked_add(offsets_offset, checked_mul(offsets_count, 2)?)?;
            ensure_range(cblc, offset, offsets_end - offset, table_end)?;
            let mut glyph_ids = Vec::with_capacity(sparse_count);
            for index in 0..sparse_count {
                let glyph_id = reader
                    .u16(checked_add(glyph_ids_offset, checked_mul(index, 2)?)?)
                    .map_err(|_| malformed(CBLC_TAG))?;
                if glyph_id < first_glyph
                    || glyph_id > last_glyph
                    || glyph_ids
                        .last()
                        .is_some_and(|previous| glyph_id <= *previous)
                {
                    return Err(malformed(CBLC_TAG));
                }
                glyph_ids.push(glyph_id);
            }
            let mut offsets = Vec::with_capacity(offsets_count);
            for index in 0..offsets_count {
                offsets.push(u32::from(
                    reader
                        .u16(checked_add(offsets_offset, checked_mul(index, 2)?)?)
                        .map_err(|_| malformed(CBLC_TAG))?,
                ));
            }
            validate_offsets(&offsets, cbdt.len() - image_data_offset)?;
            CbdtEntries::Sparse {
                glyph_ids: glyph_ids.into_boxed_slice(),
                offsets: offsets.into_boxed_slice(),
            }
        }
        5 => {
            let image_size = reader
                .u32(checked_add(offset, 8)?)
                .map_err(|_| malformed(CBLC_TAG))?;
            ensure_range(cblc, offset, 24, table_end)?;
            let metrics = read_big_metrics(&reader, checked_add(offset, 12)?)?;
            let glyph_count = usize::try_from(
                reader
                    .u32(checked_add(offset, 20)?)
                    .map_err(|_| malformed(CBLC_TAG))?,
            )
            .map_err(|_| SfntError::ArithmeticOverflow)?;
            if image_size == 0
                || usize::try_from(image_size)
                    .ok()
                    .is_none_or(|size| size > MAX_BITMAP_PAYLOAD)
                || glyph_count == 0
                || glyph_count > MAX_SBIT_RECORDS
            {
                return Err(malformed(CBLC_TAG));
            }
            let glyph_ids_offset = checked_add(offset, 24)?;
            ensure_range(
                cblc,
                offset,
                checked_add(24, checked_mul(glyph_count, 2)?)?,
                table_end,
            )?;
            let mut glyph_ids = Vec::with_capacity(glyph_count);
            for index in 0..glyph_count {
                let glyph_id = reader
                    .u16(checked_add(glyph_ids_offset, checked_mul(index, 2)?)?)
                    .map_err(|_| malformed(CBLC_TAG))?;
                if glyph_id < first_glyph
                    || glyph_id > last_glyph
                    || glyph_ids
                        .last()
                        .is_some_and(|previous| glyph_id <= *previous)
                {
                    return Err(malformed(CBLC_TAG));
                }
                glyph_ids.push(glyph_id);
            }
            CbdtEntries::Fixed {
                image_size,
                glyph_ids: Some(glyph_ids.into_boxed_slice()),
                metrics,
            }
        }
        _ => return Ok(None),
    };

    if let CbdtEntries::Range { offsets, .. } = &entries {
        validate_offsets(offsets, cbdt.len() - image_data_offset)?;
    }
    Ok(Some(CbdtIndexSubtable {
        first_glyph,
        last_glyph,
        image_format,
        image_data_offset,
        entries,
    }))
}

fn read_u32_offsets(
    reader: &Reader<'_>,
    offset: usize,
    count: usize,
    table_end: usize,
) -> Result<Vec<u32>, SfntError> {
    let size = checked_mul(count, 4)?;
    let end = checked_add(offset, size)?;
    if end > table_end {
        return Err(malformed(CBLC_TAG));
    }
    reader
        .range(offset, size)
        .map_err(|_| malformed(CBLC_TAG))?;
    let mut offsets = Vec::with_capacity(count);
    for index in 0..count {
        offsets.push(
            reader
                .u32(checked_add(offset, checked_mul(index, 4)?)?)
                .map_err(|_| malformed(CBLC_TAG))?,
        );
    }
    Ok(offsets)
}

fn read_u16_offsets(
    reader: &Reader<'_>,
    offset: usize,
    count: usize,
    table_end: usize,
) -> Result<Vec<u32>, SfntError> {
    let size = checked_mul(count, 2)?;
    let end = checked_add(offset, size)?;
    if end > table_end {
        return Err(malformed(CBLC_TAG));
    }
    reader
        .range(offset, size)
        .map_err(|_| malformed(CBLC_TAG))?;
    let mut offsets = Vec::with_capacity(count);
    for index in 0..count {
        offsets.push(u32::from(
            reader
                .u16(checked_add(offset, checked_mul(index, 2)?)?)
                .map_err(|_| malformed(CBLC_TAG))?,
        ));
    }
    Ok(offsets)
}

fn read_big_metrics(reader: &Reader<'_>, offset: usize) -> Result<BitmapMetrics, SfntError> {
    let height = u32::from(reader.u8(offset).map_err(|_| malformed(CBLC_TAG))?);
    let width = u32::from(
        reader
            .u8(checked_add(offset, 1)?)
            .map_err(|_| malformed(CBLC_TAG))?,
    );
    let bearing_x = i16::from(
        reader
            .i8(checked_add(offset, 2)?)
            .map_err(|_| malformed(CBLC_TAG))?,
    );
    let bearing_y = i16::from(
        reader
            .i8(checked_add(offset, 3)?)
            .map_err(|_| malformed(CBLC_TAG))?,
    );
    let _ = reader
        .u8(checked_add(offset, 4)?)
        .map_err(|_| malformed(CBLC_TAG))?;
    let _ = reader
        .i8(checked_add(offset, 5)?)
        .map_err(|_| malformed(CBLC_TAG))?;
    let _ = reader
        .i8(checked_add(offset, 6)?)
        .map_err(|_| malformed(CBLC_TAG))?;
    let _ = reader
        .u8(checked_add(offset, 7)?)
        .map_err(|_| malformed(CBLC_TAG))?;
    if width == 0 || height == 0 {
        return Err(malformed(CBLC_TAG));
    }
    Ok(BitmapMetrics {
        width,
        height,
        bearing_x,
        bearing_y,
    })
}

impl CbdtTable {
    fn rasterize(
        &self,
        table: &[u8],
        glyph_id: u16,
        font_size: f32,
        advance_width: f32,
    ) -> Option<RasterizedGlyph> {
        if !font_size.is_finite() || font_size <= 0.0 {
            return None;
        }
        let strike = best_strike(&self.strikes, font_size, |strike| strike.ppem_y.into())?;
        for subtable in &strike.subtables {
            let Some((start, end, index_metrics)) = subtable.record(glyph_id) else {
                continue;
            };
            let start = subtable
                .image_data_offset
                .checked_add(usize::try_from(start).ok()?)?;
            let end = subtable
                .image_data_offset
                .checked_add(usize::try_from(end).ok()?)?;
            let bytes = table.get(start..end)?;
            let (metrics, payload) = match subtable.image_format {
                17 => (Some(read_small_metrics(bytes)?), bytes.get(5..)?),
                18 => (Some(read_big_metrics_from_bytes(bytes)?), bytes.get(8..)?),
                19 => (index_metrics, bytes),
                _ => continue,
            };
            let decoded = decode_image(EmbeddedImageKind::Png, payload)?;
            let metrics = metrics.unwrap_or(BitmapMetrics {
                width: decoded.width,
                height: decoded.height,
                bearing_x: 0,
                bearing_y: i16::try_from(decoded.height).ok()?,
            });
            if metrics.width != decoded.width || metrics.height != decoded.height {
                continue;
            }
            let scale_x = font_size / f32::from(strike.ppem_x);
            let scale_y = font_size / f32::from(strike.ppem_y);
            if let Some(glyph) = rasterize_decoded_bitmap(
                decoded,
                scale_x,
                scale_y,
                f32::from(metrics.bearing_x),
                f32::from(metrics.bearing_y),
                advance_width,
            ) {
                return Some(glyph);
            }
        }
        None
    }
}

impl CbdtIndexSubtable {
    fn record(&self, glyph_id: u16) -> Option<(u32, u32, Option<BitmapMetrics>)> {
        match &self.entries {
            CbdtEntries::Range { offsets, metrics } => {
                if glyph_id < self.first_glyph || glyph_id > self.last_glyph {
                    return None;
                }
                let index = usize::from(glyph_id - self.first_glyph);
                Some((offsets[index], offsets[index + 1], *metrics))
            }
            CbdtEntries::Fixed {
                image_size,
                glyph_ids,
                metrics,
            } => {
                let index = glyph_ids.as_ref().map_or_else(
                    || {
                        (glyph_id >= self.first_glyph && glyph_id <= self.last_glyph)
                            .then_some(usize::from(glyph_id - self.first_glyph))
                    },
                    |glyph_ids| glyph_ids.binary_search(&glyph_id).ok(),
                )?;
                let start =
                    u32::try_from(u64::from(*image_size) * u64::try_from(index).ok()?).ok()?;
                let end = start.checked_add(*image_size)?;
                Some((start, end, Some(*metrics)))
            }
            CbdtEntries::Sparse { glyph_ids, offsets } => {
                let index = glyph_ids.binary_search(&glyph_id).ok()?;
                Some((offsets[index], offsets[index + 1], None))
            }
        }
    }
}

fn read_small_metrics(bytes: &[u8]) -> Option<BitmapMetrics> {
    if bytes.len() < 5 {
        return None;
    }
    let width = u32::from(bytes[1]);
    let height = u32::from(bytes[0]);
    (width > 0 && height > 0).then_some(BitmapMetrics {
        width,
        height,
        bearing_x: i16::from(bytes[2] as i8),
        bearing_y: i16::from(bytes[3] as i8),
    })
}

fn read_big_metrics_from_bytes(bytes: &[u8]) -> Option<BitmapMetrics> {
    if bytes.len() < 8 {
        return None;
    }
    let width = u32::from(bytes[1]);
    let height = u32::from(bytes[0]);
    (width > 0 && height > 0).then_some(BitmapMetrics {
        width,
        height,
        bearing_x: i16::from(bytes[2] as i8),
        bearing_y: i16::from(bytes[3] as i8),
    })
}

struct DecodedBitmap {
    width: u32,
    height: u32,
    image: image::RgbaImage,
}

fn decode_image(kind: EmbeddedImageKind, bytes: &[u8]) -> Option<DecodedBitmap> {
    if bytes.is_empty() || bytes.len() > MAX_BITMAP_PAYLOAD {
        return None;
    }
    match kind {
        EmbeddedImageKind::Png if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") => return None,
        EmbeddedImageKind::Jpeg if !bytes.starts_with(&[0xff, 0xd8]) => return None,
        EmbeddedImageKind::Tiff
            if !(bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")) =>
        {
            return None;
        }
        _ => {}
    }

    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_BITMAP_DIMENSION);
    limits.max_image_height = Some(MAX_BITMAP_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BITMAP_BYTES);
    reader.limits(limits);
    let image = reader.decode().ok()?.to_rgba8();
    let width = image.width();
    let height = image.height();
    if width == 0
        || height == 0
        || width > MAX_BITMAP_DIMENSION
        || height > MAX_BITMAP_DIMENSION
        || u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .is_none_or(|bytes| bytes > MAX_DECODED_BITMAP_BYTES)
    {
        return None;
    }
    Some(DecodedBitmap {
        width,
        height,
        image,
    })
}

fn rasterize_decoded_bitmap(
    decoded: DecodedBitmap,
    scale_x: f32,
    scale_y: f32,
    origin_x: f32,
    origin_y: f32,
    advance_width: f32,
) -> Option<RasterizedGlyph> {
    if !scale_x.is_finite()
        || !scale_y.is_finite()
        || scale_x <= 0.0
        || scale_y <= 0.0
        || !origin_x.is_finite()
        || !origin_y.is_finite()
    {
        return None;
    }
    let width = scaled_dimension(decoded.width, scale_x)?;
    let height = scaled_dimension(decoded.height, scale_y)?;
    let image = if width == decoded.width && height == decoded.height {
        decoded.image
    } else {
        image::imageops::resize(&decoded.image, width, height, FilterType::Lanczos3)
    };
    Some(RasterizedGlyph {
        bitmap: image.into_raw(),
        width,
        height,
        offset_x: origin_x * scale_x,
        offset_y: origin_y * scale_y - height as f32,
        advance_width,
        is_color: true,
    })
}

fn scaled_dimension(value: u32, scale: f32) -> Option<u32> {
    let scaled = f64::from(value) * f64::from(scale);
    if !scaled.is_finite() || scaled <= 0.0 || scaled > f64::from(MAX_BITMAP_DIMENSION) {
        return None;
    }
    Some(scaled.ceil().max(1.0) as u32)
}

fn best_strike<T>(strikes: &[T], font_size: f32, ppem: impl Fn(&T) -> u16) -> Option<&T> {
    best_strike_where(strikes, font_size, ppem, |_| true)
}

/// The strike nearest `font_size` among those `usable` accepts.
///
/// Strikes are not guaranteed to hold the same glyphs: a font may add a newer
/// glyph to only some sizes, and the nearest strike is useless to a glyph it
/// lacks while a farther one can draw it.
fn best_strike_where<T>(
    strikes: &[T],
    font_size: f32,
    ppem: impl Fn(&T) -> u16,
    usable: impl Fn(&T) -> bool,
) -> Option<&T> {
    strikes.iter().filter(|strike| usable(strike)).min_by(|left, right| {
        let left_ppem = f32::from(ppem(left));
        let right_ppem = f32::from(ppem(right));
        let left_distance = (left_ppem - font_size).abs();
        let right_distance = (right_ppem - font_size).abs();
        left_distance
            .total_cmp(&right_distance)
            .then_with(|| right_ppem.total_cmp(&left_ppem))
    })
}

fn validate_offsets(offsets: &[u32], max_offset: usize) -> Result<(), SfntError> {
    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(malformed(CBLC_TAG));
    }
    if offsets.iter().any(|offset| {
        usize::try_from(*offset)
            .ok()
            .is_none_or(|value| value > max_offset)
    }) {
        return Err(malformed(CBLC_TAG));
    }
    Ok(())
}

fn ensure_range(bytes: &[u8], offset: usize, size: usize, limit: usize) -> Result<(), SfntError> {
    let end = checked_add(offset, size)?;
    if end > limit {
        return Err(malformed(CBLC_TAG));
    }
    bytes
        .get(offset..end)
        .ok_or_else(|| malformed(CBLC_TAG))
        .map(|_| ())
}

fn malformed(tag: [u8; 4]) -> SfntError {
    SfntError::MalformedTable(super::Tag::from_bytes(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_strike_prefers_the_larger_tie() {
        let strikes = [8_u16, 16_u16];
        let selected = best_strike(&strikes, 12.0, |strike| *strike);
        assert_eq!(selected, Some(&16));
    }

    #[test]
    fn descending_bitmap_offsets_are_rejected() {
        assert_eq!(validate_offsets(&[0, 8, 7], 8), Err(malformed(CBLC_TAG)));
        assert!(validate_offsets(&[0, 0, 8], 8).is_ok());
    }

    #[test]
    fn small_metrics_are_read_in_cbdt_order() {
        let metrics = read_small_metrics(&[12, 9, 0xfe, 0xf4, 8]).expect("metrics");
        assert_eq!(metrics.width, 9);
        assert_eq!(metrics.height, 12);
        assert_eq!(metrics.bearing_x, -2);
        assert_eq!(metrics.bearing_y, -12);
    }

    #[test]
    fn scaled_dimension_is_bounded() {
        assert_eq!(scaled_dimension(10, 2.0), Some(20));
        assert_eq!(scaled_dimension(MAX_BITMAP_DIMENSION, 2.0), None);
    }

    #[test]
    fn sbix_png_strike_decodes_with_baseline_placement() {
        let png = tiny_png();
        let record_len = 8 + png.len();
        let strike_offset = 12usize;
        let record_start = 4 + 8;
        let record_end = record_start + record_len;
        let mut table = Vec::new();
        table.extend_from_slice(&1_u16.to_be_bytes());
        table.extend_from_slice(&0_u16.to_be_bytes());
        table.extend_from_slice(&1_u32.to_be_bytes());
        table.extend_from_slice(&(strike_offset as u32).to_be_bytes());
        table.extend_from_slice(&16_u16.to_be_bytes());
        table.extend_from_slice(&72_i16.to_be_bytes());
        table.extend_from_slice(&(record_start as u32).to_be_bytes());
        table.extend_from_slice(&(record_end as u32).to_be_bytes());
        table.extend_from_slice(&1_i16.to_be_bytes());
        table.extend_from_slice(&2_i16.to_be_bytes());
        table.extend_from_slice(b"png ");
        table.extend_from_slice(png);

        let parsed = parse_sbix(&table, 1, 2048, false)
            .expect("valid sbix")
            .expect("one strike");
        let glyph = parsed
            .rasterize(&table, 0, 16.0, 7.0)
            .expect("embedded PNG glyph");
        assert_eq!((glyph.width, glyph.height), (1, 1));
        assert_eq!(glyph.offset_x, 1.0);
        assert_eq!(glyph.offset_y, 1.0);
        assert_eq!(glyph.advance_width, 7.0);
        assert!(glyph.is_color);
        assert_eq!(glyph.bitmap.len(), 4);
    }

    /// Apple Color Emoji ships newer emoji (the grey heart, for one) in only
    /// some of its strikes. The strike nearest the requested size may hold no
    /// bitmap for a glyph that a larger one has, and giving up there turned
    /// the glyph into a blank.
    #[test]
    fn sbix_uses_the_nearest_strike_that_has_the_glyph() {
        let png = tiny_png();
        let (empty_strike, full_strike) = (16usize, 32usize);
        let data_start = 4 + 3 * 4; // header plus three glyph offsets
        let record_len = 8 + png.len();
        let mut table = Vec::new();
        table.extend_from_slice(&1_u16.to_be_bytes());
        table.extend_from_slice(&0_u16.to_be_bytes());
        table.extend_from_slice(&2_u32.to_be_bytes());
        table.extend_from_slice(&(empty_strike as u32).to_be_bytes());
        table.extend_from_slice(&(full_strike as u32).to_be_bytes());
        // The 16 ppem strike: two glyphs, neither with a bitmap.
        table.extend_from_slice(&16_u16.to_be_bytes());
        table.extend_from_slice(&72_u16.to_be_bytes());
        for _ in 0..3 {
            table.extend_from_slice(&(data_start as u32).to_be_bytes());
        }
        // The 32 ppem strike: glyph 1 has a PNG.
        table.extend_from_slice(&32_u16.to_be_bytes());
        table.extend_from_slice(&72_u16.to_be_bytes());
        table.extend_from_slice(&(data_start as u32).to_be_bytes());
        table.extend_from_slice(&(data_start as u32).to_be_bytes());
        table.extend_from_slice(&((data_start + record_len) as u32).to_be_bytes());
        table.extend_from_slice(&0_i16.to_be_bytes());
        table.extend_from_slice(&0_i16.to_be_bytes());
        table.extend_from_slice(b"png ");
        table.extend_from_slice(png);

        let parsed = parse_sbix(&table, 2, 2048, false)
            .expect("valid sbix")
            .expect("two strikes");

        let glyph = parsed
            .rasterize(&table, 1, 16.0, 7.0)
            .expect("the larger strike must supply a glyph the nearest one lacks");
        assert!(glyph.is_color);
        assert!(glyph.width > 0 && glyph.height > 0);
        assert!(
            parsed.rasterize(&table, 0, 16.0, 7.0).is_none(),
            "a glyph no strike holds is still absent"
        );
    }

    #[test]
    fn nearest_usable_strike_skips_strikes_that_fail_the_check() {
        let strikes = [20_u16, 32_u16, 64_u16];
        let selected = best_strike_where(&strikes, 16.0, |strike| *strike, |strike| *strike >= 32);
        assert_eq!(selected, Some(&32));
        let none = best_strike_where(&strikes, 16.0, |strike| *strike, |_| false);
        assert_eq!(none, None);
    }

    #[test]
    fn apple_color_emoji_zero_origin_uses_the_native_baseline_rule() {
        let top = sbix_bitmap_top(0, 18, 160, 2048, true);
        let expected = 18.0 - 100.0 * 160.0 / 2048.0;
        assert!(
            (top - expected).abs() < f32::EPSILON,
            "Apple Color Emoji top should be {expected}, got {top}"
        );
    }

    #[test]
    fn sbix_emjc_graphics_are_classified_without_private_decode() {
        let record_start = 4 + 8;
        let record_end = record_start + 8 + 3;
        let mut table = Vec::new();
        table.extend_from_slice(&1_u16.to_be_bytes());
        table.extend_from_slice(&0_u16.to_be_bytes());
        table.extend_from_slice(&1_u32.to_be_bytes());
        table.extend_from_slice(&12_u32.to_be_bytes());
        table.extend_from_slice(&16_u16.to_be_bytes());
        table.extend_from_slice(&72_i16.to_be_bytes());
        table.extend_from_slice(&(record_start as u32).to_be_bytes());
        table.extend_from_slice(&(record_end as u32).to_be_bytes());
        table.extend_from_slice(&0_i16.to_be_bytes());
        table.extend_from_slice(&0_i16.to_be_bytes());
        table.extend_from_slice(b"emjc");
        table.extend_from_slice(&[0x01, 0x02, 0x03]);

        let parsed = parse_sbix(&table, 1, 2048, false)
            .expect("private sbix directory should parse")
            .expect("one strike");
        assert!(parsed.has_private_graphics);
        assert!(parsed.rasterize(&table, 0, 16.0, 7.0).is_none());
    }

    #[test]
    fn cbdt_format_17_decodes_small_metrics_and_png() {
        let png = tiny_png();
        let image = [1_u8, 1, 0, 2, 1]
            .into_iter()
            .chain(png.iter().copied())
            .collect::<Vec<_>>();
        let mut cbdt = image.clone();

        let array_offset = 56usize;
        let subtable_offset = array_offset + 8;
        let array_size = 8 + 16;
        let mut cblc = Vec::new();
        cblc.extend_from_slice(&0x0002_0000_u32.to_be_bytes());
        cblc.extend_from_slice(&1_u32.to_be_bytes());
        cblc.extend_from_slice(&(array_offset as u32).to_be_bytes());
        cblc.extend_from_slice(&(array_size as u32).to_be_bytes());
        cblc.extend_from_slice(&1_u32.to_be_bytes());
        cblc.extend_from_slice(&0_u32.to_be_bytes());
        cblc.extend_from_slice(&0_u16.to_be_bytes());
        cblc.extend_from_slice(&0_u16.to_be_bytes());
        cblc.push(16);
        cblc.push(16);
        cblc.push(32);
        cblc.push(0);
        cblc.extend_from_slice(&[0; 24]);
        cblc.extend_from_slice(&0_u16.to_be_bytes());
        cblc.extend_from_slice(&0_u16.to_be_bytes());
        cblc.extend_from_slice(&8_u32.to_be_bytes());
        cblc.extend_from_slice(&1_u16.to_be_bytes());
        cblc.extend_from_slice(&17_u16.to_be_bytes());
        cblc.extend_from_slice(&0_u32.to_be_bytes());
        cblc.extend_from_slice(&0_u32.to_be_bytes());
        cblc.extend_from_slice(&(image.len() as u32).to_be_bytes());
        assert_eq!(cblc.len(), subtable_offset + 16);

        let parsed = parse_cbdt(&cblc, &cbdt, 1)
            .expect("valid CBLC/CBDT")
            .expect("one bitmap strike");
        let glyph = parsed
            .rasterize(&mut cbdt, 0, 16.0, 9.0)
            .expect("embedded CBDT glyph");
        assert_eq!((glyph.width, glyph.height), (1, 1));
        assert_eq!(glyph.offset_y, 1.0);
        assert_eq!(glyph.advance_width, 9.0);
        assert!(glyph.is_color);
    }

    fn tiny_png() -> &'static [u8] {
        &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99,
            0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ]
    }
}
