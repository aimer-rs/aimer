// The Aimer-owned parser, shaper, and rasterizer are the single portable font
// implementation used by Cupid. Apple-private glyphs remain an explicit
// platform bridge in the surrounding pipeline.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock, RwLock};

use super::font_resolver::FontData;
use super::glyph_rasterizer::{GlyphKey, NORMAL_GLYPH_WEIGHT, RasterizedGlyph};

mod apple_private;
mod bitmap;
mod cff;
mod color;
mod layout;
mod outline;
mod rasterize;
mod svg;
mod variation;

#[cfg(test)]
pub(crate) use layout::{shape_arabic_run, shape_latin_run, shape_run_with_options};
#[cfg(test)]
pub(crate) use rasterize::rasterize_font_glyph;
#[cfg(test)]
pub(crate) use rasterize::rasterize_font_glyphs;

pub(crate) fn recycle_shaped_glyphs(glyphs: Vec<layout::AimerLayoutGlyph>) {
    layout::recycle_shaped_glyphs(glyphs);
}

// Apple Color Emoji carries its `sbix` strikes in a roughly 190 MiB table on
// current macOS releases. The reader borrows table bytes, so accepting that
// face does not allocate the table; the bound still limits hostile inputs and
// keeps the total font smaller than a typical application asset budget.
const MAX_FONT_BYTES: usize = 256 * 1024 * 1024;
const MAX_TABLE_BYTES: usize = 256 * 1024 * 1024;
const MAX_TTC_FACES: u32 = 64;
const MAX_TABLE_COUNT: u16 = 4096;
const MAX_CMAP_GROUPS: u32 = 1 << 20;
const MAX_CMAP_VARIATION_RECORDS: u32 = 1 << 20;
const MAX_CMAP_VARIATION_ENTRIES: u32 = 1 << 20;

const TTC_TAG: Tag = Tag(*b"ttcf");
const WOFF_TAG: Tag = Tag(*b"wOFF");
const WOFF2_TAG: Tag = Tag(*b"wOF2");

/// A four-byte OpenType table identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct Tag([u8; 4]);

impl Tag {
    pub(crate) const fn from_bytes(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }
}

/// Deterministic failures returned while reading an SFNT container.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SfntError {
    Empty,
    FontTooLarge { size: usize, max: usize },
    Truncated { offset: usize, size: usize },
    ArithmeticOverflow,
    InvalidSignature(Tag),
    UnsupportedContainer(Tag),
    UnsupportedCollectionVersion(u32),
    InvalidFaceOffset(u32),
    FaceIndexOutOfBounds { index: u32, count: u32 },
    TooManyFaces { count: u32, max: u32 },
    TooManyTables { count: u16, max: u16 },
    TableTooLarge { tag: Tag, length: u32, max: usize },
    TableOutOfBounds { tag: Tag, offset: u32, length: u32 },
    DuplicateTable(Tag),
    MissingTable(Tag),
    MalformedTable(Tag),
    CmapSubtableOutOfBounds(u32),
    MalformedCmap(u16),
    CmapGlyphOutOfRange(u32),
    OutlineRecursionLimit,
    CompositeCycle(u16),
    UnsupportedCompositeAttachment,
    UnsupportedCffOperator { tag: Tag, operator: u16 },
    CffSubroutineRecursionLimit,
    CffSubroutineCycle { global: bool, index: usize },
}

impl fmt::Display for SfntError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for SfntError {}

#[derive(Clone, Copy, Debug)]
struct TableRecord {
    tag: Tag,
    offset: usize,
    length: usize,
}

/// Horizontal and global metrics read from the face's required tables.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FontMetrics {
    pub(crate) units_per_em: u16,
    pub(crate) ascender: i16,
    pub(crate) descender: i16,
    pub(crate) line_gap: i16,
    /// Design cap height from `OS/2` (version 2 and later); `0` when the face
    /// does not state one.
    pub(crate) cap_height: i16,
    pub(crate) x_min: i16,
    pub(crate) y_min: i16,
    pub(crate) x_max: i16,
    pub(crate) y_max: i16,
    pub(crate) num_glyphs: u16,
    pub(crate) number_of_h_metrics: u16,
    pub(crate) index_to_loc_format: i16,
}

impl FontMetrics {
    /// Largest baseline shift, in em, that centring the capitals may apply.
    ///
    /// A face whose stated cap height disagrees with its `hhea` box by more
    /// than this is more likely misdeclared than deliberately lopsided.
    const MAX_OPTICAL_SHIFT_EM: f32 = 0.15;

    /// Scaled `(ascent, descent, line_gap)` for a line set in `font_size`.
    ///
    /// `descent` is negative, as in `hhea`. The line box is the face's own
    /// `ascent - descent` tall, but the baseline is placed so the capitals are
    /// centred in it. Faces tuned the usual way already are: their ascent
    /// exceeds the cap height by about what their descent leaves under the
    /// baseline. A face that sets its ascent equal to its cap height, as Nimbus
    /// Sans does, reserves 0.27 em under the baseline that only descenders use,
    /// so its text sits high in a box centred on `hhea`. Faces that state no cap
    /// height keep their `hhea` values.
    pub(crate) fn scaled_line_metrics(&self, font_size: f32) -> (f32, f32, f32) {
        let scale = font_size / f32::from(self.units_per_em);
        let ascent = f32::from(self.ascender);
        let descent = f32::from(self.descender);
        let shift = if self.cap_height > 0 {
            let limit = Self::MAX_OPTICAL_SHIFT_EM * f32::from(self.units_per_em);
            let centred = f32::from(self.cap_height) / 2.0 - (ascent + descent) / 2.0;
            // The baseline stays inside the box: ascent cannot go negative and
            // the descent cannot cross to the other side of it.
            centred.clamp(-limit, limit).clamp(-ascent.max(0.0), (-descent).max(0.0))
        } else {
            0.0
        };
        (
            (ascent + shift) * scale,
            (descent + shift) * scale,
            f32::from(self.line_gap) * scale,
        )
    }
}

#[derive(Clone)]
struct HmtxTable {
    advances: Vec<u16>,
}

impl HmtxTable {
    fn parse(table: &[u8], metrics: FontMetrics) -> Result<Self, SfntError> {
        let tag = Tag::from_bytes(*b"hmtx");
        let required_size = checked_add(
            checked_mul(usize::from(metrics.number_of_h_metrics), 4)?,
            checked_mul(
                usize::from(metrics.num_glyphs - metrics.number_of_h_metrics),
                2,
            )?,
        )?;
        if table.len() < required_size {
            return Err(SfntError::MalformedTable(tag));
        }

        let reader = Reader::new(table);
        let mut advances = Vec::with_capacity(usize::from(metrics.num_glyphs));
        for glyph_id in 0..metrics.num_glyphs {
            let metric_index = glyph_id.min(metrics.number_of_h_metrics - 1);
            let offset = checked_mul(usize::from(metric_index), 4)?;
            advances.push(
                reader
                    .u16(offset)
                    .map_err(|_| SfntError::MalformedTable(tag))?,
            );
        }
        Ok(Self { advances })
    }
}

/// The vertical metrics and origin used for one glyph in a vertical run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VerticalGlyphMetrics {
    pub(crate) advance_height: u16,
    pub(crate) top_side_bearing: i16,
    /// The glyph origin in font units, measured from the top-to-bottom pen.
    /// A VORG record wins over the table default; without VORG this is derived
    /// from the glyph's top side bearing and the face bounding box.
    pub(crate) vert_origin_y: i32,
}

#[derive(Clone)]
struct VerticalMetricsTable {
    ascender: i16,
    descender: i16,
    line_gap: i16,
    number_of_v_metrics: u16,
    glyphs: Vec<VerticalMetricRecord>,
    vorg: Option<VorgTable>,
    origin_cache: Vec<OnceLock<Result<i32, SfntError>>>,
}

#[derive(Clone, Copy)]
struct VerticalMetricRecord {
    advance_height: u16,
    top_side_bearing: i16,
}

#[derive(Clone)]
struct VorgTable {
    default_vert_origin_y: i16,
    records: Vec<(u16, i16)>,
}

impl VorgTable {
    fn origin_for(&self, glyph_id: u16) -> Option<i16> {
        self.records
            .binary_search_by_key(&glyph_id, |(record_glyph_id, _)| *record_glyph_id)
            .ok()
            .map(|index| self.records[index].1)
    }
}

impl VerticalMetricsTable {
    fn parse(
        vhea: &[u8],
        vmtx: &[u8],
        vorg: Option<&[u8]>,
        metrics: FontMetrics,
    ) -> Result<Self, SfntError> {
        let vhea_tag = Tag::from_bytes(*b"vhea");
        if vhea.len() < 36 {
            return Err(SfntError::MalformedTable(vhea_tag));
        }
        let vhea_reader = Reader::new(vhea);
        if vhea_reader.u32(0)? != 0x0001_0000 {
            return Err(SfntError::MalformedTable(vhea_tag));
        }
        let number_of_v_metrics = vhea_reader.u16(34)?;
        if number_of_v_metrics == 0 || number_of_v_metrics > metrics.num_glyphs {
            return Err(SfntError::MalformedTable(vhea_tag));
        }

        let vmtx_tag = Tag::from_bytes(*b"vmtx");
        let required_size = checked_add(
            checked_mul(usize::from(number_of_v_metrics), 4)?,
            checked_mul(usize::from(metrics.num_glyphs - number_of_v_metrics), 2)?,
        )?;
        if vmtx.len() < required_size {
            return Err(SfntError::MalformedTable(vmtx_tag));
        }
        let vmtx_reader = Reader::new(vmtx);
        let last_metric_offset = checked_mul(usize::from(number_of_v_metrics - 1), 4)?;
        let final_advance_height = vmtx_reader
            .u16(last_metric_offset)
            .map_err(|_| SfntError::MalformedTable(vmtx_tag))?;
        let vorg = vorg
            .map(|table| VorgTable::parse(table, metrics.num_glyphs))
            .transpose()?;

        let mut glyphs = Vec::with_capacity(usize::from(metrics.num_glyphs));
        for glyph_id in 0..metrics.num_glyphs {
            let (advance_height, top_side_bearing) = if glyph_id < number_of_v_metrics {
                let offset = checked_mul(usize::from(glyph_id), 4)?;
                (
                    vmtx_reader
                        .u16(offset)
                        .map_err(|_| SfntError::MalformedTable(vmtx_tag))?,
                    vmtx_reader
                        .i16(checked_add(offset, 2)?)
                        .map_err(|_| SfntError::MalformedTable(vmtx_tag))?,
                )
            } else {
                let trailing_index = usize::from(glyph_id - number_of_v_metrics);
                let offset = checked_add(
                    checked_mul(usize::from(number_of_v_metrics), 4)?,
                    checked_mul(trailing_index, 2)?,
                )?;
                (
                    final_advance_height,
                    vmtx_reader
                        .i16(offset)
                        .map_err(|_| SfntError::MalformedTable(vmtx_tag))?,
                )
            };
            glyphs.push(VerticalMetricRecord {
                advance_height,
                top_side_bearing,
            });
        }

        Ok(Self {
            ascender: vhea_reader.i16(4)?,
            descender: vhea_reader.i16(6)?,
            line_gap: vhea_reader.i16(8)?,
            number_of_v_metrics,
            glyphs,
            vorg,
            origin_cache: std::iter::repeat_with(OnceLock::new)
                .take(usize::from(metrics.num_glyphs))
                .collect(),
        })
    }
}

impl VorgTable {
    fn parse(table: &[u8], num_glyphs: u16) -> Result<Self, SfntError> {
        let tag = Tag::from_bytes(*b"VORG");
        if table.len() < 8 {
            return Err(SfntError::MalformedTable(tag));
        }
        let reader = Reader::new(table);
        if reader.u16(0)? != 1 {
            return Err(SfntError::MalformedTable(tag));
        }
        let count = reader.u16(6)?;
        if usize::from(count) > usize::from(num_glyphs) {
            return Err(SfntError::MalformedTable(tag));
        }
        let records_size = checked_mul(usize::from(count), 4)?;
        let records_offset = 8;
        let records_end = checked_add(records_offset, records_size)?;
        if table.len() < records_end {
            return Err(SfntError::MalformedTable(tag));
        }

        let mut records = Vec::with_capacity(usize::from(count));
        let mut previous_glyph_id = None;
        for index in 0..count {
            let offset = checked_add(records_offset, checked_mul(usize::from(index), 4)?)?;
            let glyph_id = reader.u16(offset)?;
            if glyph_id >= num_glyphs
                || previous_glyph_id.is_some_and(|previous| glyph_id <= previous)
            {
                return Err(SfntError::MalformedTable(tag));
            }
            records.push((glyph_id, reader.i16(checked_add(offset, 2)?)?));
            previous_glyph_id = Some(glyph_id);
        }

        Ok(Self {
            default_vert_origin_y: reader.i16(4)?,
            records,
        })
    }
}

enum FaceBytes<'a> {
    Borrowed(&'a [u8]),
    Owned(FontData),
}

impl<'a> AsRef<[u8]> for FaceBytes<'a> {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Owned(data) => data.as_ref(),
        }
    }
}

/// A validated face in a standalone SFNT or TrueType collection.
///
/// The face owns a reference-counted or memory-mapped handle to the original
/// bytes. Directory records are validated once during construction, so table
/// access cannot create an out-of-bounds slice or overflow a host-sized
/// offset. Owning the handle lets a parsed face live in a per-font cache
/// without a self-referential lifetime or an unsafe leak.
pub(crate) struct SfntFace<'a> {
    bytes: FaceBytes<'a>,
    tables: Vec<TableRecord>,
    apple_private: apple_private::ApplePrivateTables,
    metrics_cache: OnceLock<Result<FontMetrics, SfntError>>,
    cmap_cache: OnceLock<Result<ParsedCmap, SfntError>>,
    hmtx_cache: OnceLock<Result<HmtxTable, SfntError>>,
    vertical_metrics_cache: OnceLock<Result<Option<VerticalMetricsTable>, SfntError>>,
    loca_cache: OnceLock<Result<Vec<usize>, SfntError>>,
    variation_cache: OnceLock<Result<Option<variation::VariationInfo>, SfntError>>,
    color_cache: OnceLock<Result<Option<color::ColorTables>, SfntError>>,
    bitmap_cache: OnceLock<Result<Option<bitmap::BitmapTables>, SfntError>>,
    svg_cache: OnceLock<Result<Option<svg::SvgTables>, SfntError>>,
}

impl<'a> SfntFace<'a> {
    /// Parses `face_index` from a TTF, OTF, or TTC byte slice.
    pub(crate) fn from_bytes(bytes: &'a [u8], face_index: u32) -> Result<Self, SfntError> {
        Self::from_storage(FaceBytes::Borrowed(bytes), face_index)
    }

    /// Parses `face_index` while retaining the caller's shared font storage.
    ///
    /// This is the cache-facing constructor. It preserves memory-mapped font
    /// data and avoids copying a font merely to make the validated face
    /// independent of the caller's borrow.
    pub(crate) fn from_font_data(
        data: FontData,
        face_index: u32,
    ) -> Result<SfntFace<'static>, SfntError> {
        SfntFace::from_storage(FaceBytes::Owned(data), face_index)
    }

    fn from_storage(storage: FaceBytes<'a>, face_index: u32) -> Result<Self, SfntError> {
        let bytes = storage.as_ref();
        if bytes.is_empty() {
            return Err(SfntError::Empty);
        }
        if bytes.len() > MAX_FONT_BYTES {
            return Err(SfntError::FontTooLarge {
                size: bytes.len(),
                max: MAX_FONT_BYTES,
            });
        }

        let reader = Reader::new(bytes);
        let signature = reader.tag(0)?;

        if signature == TTC_TAG {
            let version = reader.u32(4)?;
            if version != 0x0001_0000 && version != 0x0002_0000 {
                return Err(SfntError::UnsupportedCollectionVersion(version));
            }

            let face_count = reader.u32(8)?;
            if face_count > MAX_TTC_FACES {
                return Err(SfntError::TooManyFaces {
                    count: face_count,
                    max: MAX_TTC_FACES,
                });
            }
            if face_index >= face_count {
                return Err(SfntError::FaceIndexOutOfBounds {
                    index: face_index,
                    count: face_count,
                });
            }

            let offsets_size = checked_mul(
                usize::try_from(face_count).map_err(|_| SfntError::ArithmeticOverflow)?,
                4,
            )?;
            reader.range(12, offsets_size)?;

            let selected_offset = reader.u32(
                12 + checked_mul(
                    usize::try_from(face_index).map_err(|_| SfntError::ArithmeticOverflow)?,
                    4,
                )?,
            )?;

            // Validate every collection offset before parsing the selected
            // face. A malformed unselected face must not hide in a shared
            // font buffer and become a later source of unchecked offsets.
            for face in 0..face_count {
                let offset = reader.u32(
                    12 + checked_mul(
                        usize::try_from(face).map_err(|_| SfntError::ArithmeticOverflow)?,
                        4,
                    )?,
                )?;
                validate_face_offset(&reader, offset)?;
            }

            let tables = Self::from_directory(bytes, selected_offset)?;
            return Ok(Self::with_tables(storage, tables));
        }

        if signature == WOFF_TAG || signature == WOFF2_TAG {
            return Err(SfntError::UnsupportedContainer(signature));
        }
        if !is_sfnt_signature(signature) {
            return Err(SfntError::InvalidSignature(signature));
        }
        if face_index != 0 {
            return Err(SfntError::FaceIndexOutOfBounds {
                index: face_index,
                count: 1,
            });
        }

        let tables = Self::from_directory(bytes, 0)?;
        Ok(Self::with_tables(storage, tables))
    }

    fn with_tables(storage: FaceBytes<'a>, mut tables: Vec<TableRecord>) -> Self {
        tables.sort_unstable_by_key(|record| record.tag);
        let apple_private = apple_private::ApplePrivateTables::from_records(&tables);
        Self {
            bytes: storage,
            tables,
            apple_private,
            metrics_cache: OnceLock::new(),
            cmap_cache: OnceLock::new(),
            hmtx_cache: OnceLock::new(),
            vertical_metrics_cache: OnceLock::new(),
            loca_cache: OnceLock::new(),
            variation_cache: OnceLock::new(),
            color_cache: OnceLock::new(),
            bitmap_cache: OnceLock::new(),
            svg_cache: OnceLock::new(),
        }
    }

    /// Returns a validated table slice, or `None` when the face does not
    /// contain `tag`.
    pub(crate) fn table(&self, tag: [u8; 4]) -> Option<&[u8]> {
        let tag = Tag::from_bytes(tag);
        self.tables
            .binary_search_by_key(&tag, |record| record.tag)
            .ok()
            .map(|index| {
                let record = &self.tables[index];
                // `from_directory` checked this range and stores host-sized
                // values, so this access cannot fail while the owned byte handle
                // remains valid.
                &self.bytes.as_ref()[record.offset..record.offset + record.length]
            })
    }

    /// Reports whether the face advertises a color-glyph table understood by
    /// the enabled Aimer or compatibility color renderer.
    pub(crate) fn has_color_tables(&self) -> bool {
        self.table(*b"sbix").is_some()
            || self.table(*b"CBDT").is_some()
            || self.table(*b"COLR").is_some()
            || self.table(*b"SVG ").is_some()
    }

    /// Returns the Apple-private glyph-data classification for this face.
    ///
    /// Only table tags are inspected. The private payload is intentionally
    /// opaque to the portable reader.
    pub(crate) fn apple_private_tables(&self) -> apple_private::ApplePrivateTables {
        self.apple_private
    }

    /// Reports whether a face has Apple-private color strikes that require
    /// platform compatibility rendering rather than the public bitmap reader.
    pub(crate) fn has_apple_private_color_tables(&self) -> bool {
        if self.apple_private.has_private_color() {
            return true;
        }
        if self.table(*b"sbix").is_none() {
            return false;
        }
        self.bitmap_cache
            .get_or_init(|| bitmap::parse(self))
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .is_some_and(bitmap::BitmapTables::has_private_color_data)
    }

    /// Reports whether the portable rasterizer must decline this face.
    pub(crate) fn requires_platform_rasterization(&self) -> bool {
        if self.apple_private.is_empty() {
            // Private color strikes inside `sbix` decide on their own, unlike
            // the directory tags below where a public outline takes
            // precedence: a color face has no outline to prefer. The iOS
            // emoji face keeps a `glyf` table whose emoji entries are empty,
            // and honoring it made the owned path return blank glyphs
            // instead of declining to the platform.
            return self.has_apple_private_color_tables();
        }
        self.apple_private
            .requires_platform_raster(self.has_standard_outline())
    }

    /// Returns the COLR v0 layers for a glyph when the face also has a
    /// supported CPAL default palette. Unsupported color formats return
    /// `Ok(None)` so callers can retain their compatibility fallback.
    pub(crate) fn color_layers(
        &self,
        glyph_id: u16,
    ) -> Result<Option<&[color::ColorLayer]>, SfntError> {
        self.color_cache
            .get_or_init(|| color::parse(self))
            .as_ref()
            .map_err(|error| *error)
            .map(|tables| tables.as_ref().and_then(|tables| tables.layers(glyph_id)))
    }

    /// Returns a color from the face's default CPAL palette.
    pub(crate) fn palette_color(
        &self,
        palette_index: u16,
    ) -> Result<Option<color::ColorRgba>, SfntError> {
        self.color_cache
            .get_or_init(|| color::parse(self))
            .as_ref()
            .map_err(|error| *error)
            .map(|tables| {
                tables
                    .as_ref()
                    .and_then(|tables| tables.palette_color(palette_index))
            })
    }

    /// Rasterizes a supported embedded bitmap strike using the face-local
    /// parsed `sbix` or `CBDT`/`CBLC` index. Encoded image bytes are decoded
    /// only for the requested glyph and are never retained in the face cache.
    pub(crate) fn bitmap_glyph(
        &self,
        glyph_id: u16,
        font_size: f32,
        advance_width: f32,
    ) -> Option<RasterizedGlyph> {
        let tables = self
            .bitmap_cache
            .get_or_init(|| bitmap::parse(self))
            .as_ref()
            .ok()?
            .as_ref()?;
        tables.rasterize(self, glyph_id, font_size, advance_width)
    }

    /// Returns an owned, cached SVG glyph document when the face contains a
    /// supported `SVG ` entry for `glyph_id`. Unsupported SVG paint and node
    /// features return `None` so the caller can use the normal compatibility
    /// fallback or the monochrome outline.
    pub(crate) fn svg_glyph(&self, glyph_id: u16) -> Option<Arc<svg::SvgGlyph>> {
        let tables = self
            .svg_cache
            .get_or_init(|| svg::parse(self))
            .as_ref()
            .ok()?
            .as_ref()?;
        tables.glyph(self, glyph_id).ok().flatten()
    }

    /// Reports whether the face carries an outline table the Aimer rasterizer
    /// can decode without a platform text API.
    pub(crate) fn has_standard_outline(&self) -> bool {
        self.table(*b"glyf").is_some()
            || self.table(*b"CFF ").is_some()
            || self.table(*b"CFF2").is_some()
    }

    /// Reads the face's OpenType `OS/2` design weight when it is present.
    pub(crate) fn design_weight(&self) -> Option<u16> {
        let table = self.table(*b"OS/2")?;
        let weight = Reader::new(table).u16(4).ok()?;
        (1..=1000).contains(&weight).then_some(weight)
    }

    /// Reads the design cap height from `OS/2` when the table is version 2 or
    /// later and states a positive one.
    fn cap_height(&self) -> Option<i16> {
        let reader = Reader::new(self.table(*b"OS/2")?);
        if reader.u16(0).ok()? < 2 {
            return None;
        }
        let cap_height = reader.i16(88).ok()?;
        (cap_height > 0).then_some(cap_height)
    }

    /// Looks up a Unicode scalar value in the face's best supported base
    /// `cmap` subtable.
    pub(crate) fn glyph_index(&self, codepoint: u32) -> Result<Option<u16>, SfntError> {
        if codepoint > 0x0010_ffff {
            return Ok(None);
        }
        let Some(cmap) = self.table(*b"cmap") else {
            return Ok(None);
        };
        let parsed = self.cmap_cache.get_or_init(|| ParsedCmap::parse(cmap));
        parsed
            .as_ref()
            .map_err(|error| *error)
            .and_then(|cmap| cmap.glyph_index(codepoint))
    }

    /// Reports whether the face has a non-zero glyph for a Unicode scalar.
    pub(crate) fn covers(&self, codepoint: u32) -> Result<bool, SfntError> {
        Ok(self.glyph_index(codepoint)?.is_some())
    }

    /// Looks up a Unicode variation sequence using a format 14 `cmap`
    /// subtable, falling back to the base character when the selector is not
    /// declared by the font.
    pub(crate) fn glyph_index_with_variation(
        &self,
        codepoint: u32,
        selector: u32,
    ) -> Result<Option<u16>, SfntError> {
        if codepoint > 0x0010_ffff || selector > 0x0010_ffff {
            return Ok(None);
        }
        let Some(cmap) = self.table(*b"cmap") else {
            return Ok(None);
        };
        let base_glyph = self.glyph_index(codepoint)?;
        self.cmap_cache
            .get_or_init(|| ParsedCmap::parse(cmap))
            .as_ref()
            .map_err(|error| *error)
            .and_then(|cmap| cmap.glyph_index_with_variation(codepoint, selector, base_glyph))
    }

    /// Reads the face metrics from `head`, `hhea`, and `maxp`.
    pub(crate) fn metrics(&self) -> Result<FontMetrics, SfntError> {
        self.metrics_cache
            .get_or_init(|| self.parse_metrics())
            .as_ref()
            .copied()
            .map_err(|error| *error)
    }

    fn parse_metrics(&self) -> Result<FontMetrics, SfntError> {
        let head = self.required_table(*b"head")?;
        if head.len() < 54 {
            return Err(SfntError::MalformedTable(Tag::from_bytes(*b"head")));
        }
        let head_reader = Reader::new(head);
        let units_per_em = head_reader.u16(18)?;
        if units_per_em == 0 {
            return Err(SfntError::MalformedTable(Tag::from_bytes(*b"head")));
        }

        let maxp = self.required_table(*b"maxp")?;
        if maxp.len() < 6 {
            return Err(SfntError::MalformedTable(Tag::from_bytes(*b"maxp")));
        }
        let maxp_reader = Reader::new(maxp);
        let version = maxp_reader.u32(0)?;
        if version != 0x0000_5000 && version != 0x0001_0000 {
            return Err(SfntError::MalformedTable(Tag::from_bytes(*b"maxp")));
        }
        let num_glyphs = maxp_reader.u16(4)?;
        if num_glyphs == 0 {
            return Err(SfntError::MalformedTable(Tag::from_bytes(*b"maxp")));
        }

        let hhea = self.required_table(*b"hhea")?;
        if hhea.len() < 36 {
            return Err(SfntError::MalformedTable(Tag::from_bytes(*b"hhea")));
        }
        let hhea_reader = Reader::new(hhea);
        let number_of_h_metrics = hhea_reader.u16(34)?;
        if number_of_h_metrics == 0 || number_of_h_metrics > num_glyphs {
            return Err(SfntError::MalformedTable(Tag::from_bytes(*b"hhea")));
        }

        Ok(FontMetrics {
            units_per_em,
            ascender: hhea_reader.i16(4)?,
            descender: hhea_reader.i16(6)?,
            line_gap: hhea_reader.i16(8)?,
            cap_height: self.cap_height().unwrap_or(0),
            x_min: head_reader.i16(36)?,
            y_min: head_reader.i16(38)?,
            x_max: head_reader.i16(40)?,
            y_max: head_reader.i16(42)?,
            num_glyphs,
            number_of_h_metrics,
            index_to_loc_format: head_reader.i16(50)?,
        })
    }

    /// Returns the validated vertical metrics table, if the face carries both
    /// `vhea` and `vmtx`. An optional `VORG` table supplies per-glyph vertical
    /// origins or its default origin; otherwise the origin is derived from
    /// the glyph's top side bearing and the face bounds.
    fn vertical_metrics(&self) -> Result<Option<&VerticalMetricsTable>, SfntError> {
        self.vertical_metrics_cache
            .get_or_init(|| {
                let (Some(vhea), Some(vmtx)) = (self.table(*b"vhea"), self.table(*b"vmtx")) else {
                    return Ok(None);
                };
                let metrics = self.metrics()?;
                VerticalMetricsTable::parse(vhea, vmtx, self.table(*b"VORG"), metrics).map(Some)
            })
            .as_ref()
            .map_err(|error| *error)
            .map(Option::as_ref)
    }

    fn glyph_y_max(&self, glyph_id: u16, metrics: FontMetrics) -> Result<Option<i32>, SfntError> {
        if let Some(outline) = self.outline_with_metrics(glyph_id, metrics)? {
            return Ok(Some(i32::from(outline.bounds[3])));
        }
        let Some(outline) = self.cff_outline(glyph_id)? else {
            return Ok(None);
        };
        Ok(outline.bounds[3]
            .is_finite()
            .then_some(outline.bounds[3].round() as i32))
    }

    /// Returns one glyph's vertical advance, top side bearing, and vertical
    /// origin. Glyphs outside `maxp.numGlyphs` are reported as unavailable.
    pub(crate) fn vertical_glyph_metrics(
        &self,
        glyph_id: u16,
    ) -> Result<Option<VerticalGlyphMetrics>, SfntError> {
        let Some(vertical_metrics) = self.vertical_metrics()? else {
            return Ok(None);
        };
        let Some(record) = vertical_metrics.glyphs.get(usize::from(glyph_id)).copied() else {
            return Ok(None);
        };
        let vert_origin_y = if let Some(vorg) = vertical_metrics.vorg.as_ref() {
            vorg.origin_for(glyph_id)
                .map(i32::from)
                .unwrap_or_else(|| i32::from(vorg.default_vert_origin_y))
        } else {
            let metrics = self.metrics()?;
            *vertical_metrics.origin_cache[usize::from(glyph_id)]
                .get_or_init(|| {
                    Ok(self
                        .glyph_y_max(glyph_id, metrics)?
                        .unwrap_or(i32::from(metrics.y_max))
                        + i32::from(record.top_side_bearing))
                })
                .as_ref()
                .map_err(|error| *error)?
        };
        Ok(Some(VerticalGlyphMetrics {
            advance_height: record.advance_height,
            top_side_bearing: record.top_side_bearing,
            vert_origin_y,
        }))
    }

    /// Returns one glyph's vertical metrics after applying its selected VVAR
    /// instance. The base `vmtx`/`VORG` result remains the fast path for static
    /// faces and variable faces without vertical metric deltas.
    pub(crate) fn vertical_glyph_metrics_at_weight(
        &self,
        glyph_id: u16,
        weight: u16,
    ) -> Result<Option<VerticalGlyphMetrics>, SfntError> {
        let (coordinates, coordinate_count) = self.coordinates_for_weight_instance(weight);
        self.vertical_glyph_metrics_at_coordinates(glyph_id, &coordinates[..coordinate_count])
    }

    /// Returns one glyph's vertical metrics after applying a complete
    /// normalized variation instance.
    pub(crate) fn vertical_glyph_metrics_at_coordinates(
        &self,
        glyph_id: u16,
        coordinates: &[f32],
    ) -> Result<Option<VerticalGlyphMetrics>, SfntError> {
        let Some(metrics) = self.vertical_glyph_metrics(glyph_id)? else {
            return Ok(None);
        };
        let [advance_delta, bearing_delta, origin_delta] =
            self.vertical_metric_deltas_at_coordinates(glyph_id, coordinates)?;
        let origin_delta = if self
            .vertical_metrics()?
            .is_some_and(|vertical_metrics| vertical_metrics.vorg.is_none())
        {
            bearing_delta.saturating_add(origin_delta)
        } else {
            origin_delta
        };
        Ok(Some(VerticalGlyphMetrics {
            advance_height: (i32::from(metrics.advance_height) + advance_delta)
                .clamp(0, i32::from(u16::MAX)) as u16,
            top_side_bearing: (i32::from(metrics.top_side_bearing) + bearing_delta)
                .clamp(i32::from(i16::MIN), i32::from(i16::MAX))
                as i16,
            vert_origin_y: metrics.vert_origin_y.saturating_add(origin_delta),
        }))
    }

    /// Returns a glyph's horizontal advance in font units.
    ///
    /// Glyph IDs outside `maxp.numGlyphs` return `Ok(None)`. Glyphs after
    /// `hhea.numberOfHMetrics` reuse the final long horizontal metric as
    /// required by the TrueType and OpenType specifications.
    pub(crate) fn glyph_advance(&self, glyph_id: u16) -> Result<Option<u16>, SfntError> {
        let metrics = self.metrics()?;
        self.glyph_advance_with_metrics(glyph_id, metrics)
    }

    /// Returns a glyph's horizontal advance while reusing already-read face
    /// metrics. Rasterizing a pending run uses this form so every glyph does
    /// not re-read `head`, `hhea`, and `maxp`.
    pub(crate) fn glyph_advance_with_metrics(
        &self,
        glyph_id: u16,
        metrics: FontMetrics,
    ) -> Result<Option<u16>, SfntError> {
        if glyph_id >= metrics.num_glyphs {
            return Ok(None);
        }
        self.glyph_advances_with_metrics(metrics)
            .map(|advances| advances[usize::from(glyph_id)])
            .map(Some)
    }

    /// Returns one glyph's horizontal advance after applying its selected
    /// HVAR instance. The signed result is clamped to a usable non-negative
    /// advance because malformed variation data must not move the pen
    /// backwards indefinitely.
    pub(crate) fn glyph_advance_with_metrics_at_weight(
        &self,
        glyph_id: u16,
        metrics: FontMetrics,
        weight: u16,
    ) -> Result<Option<i32>, SfntError> {
        let (coordinates, coordinate_count) = self.coordinates_for_weight_instance(weight);
        self.glyph_advance_with_metrics_at_coordinates(
            glyph_id,
            metrics,
            &coordinates[..coordinate_count],
        )
    }

    /// Returns one glyph's horizontal advance after applying a complete
    /// normalized variation instance.
    pub(crate) fn glyph_advance_with_metrics_at_coordinates(
        &self,
        glyph_id: u16,
        metrics: FontMetrics,
        coordinates: &[f32],
    ) -> Result<Option<i32>, SfntError> {
        let Some(base) = self.glyph_advance_with_metrics(glyph_id, metrics)? else {
            return Ok(None);
        };
        let delta = self.horizontal_metric_deltas_at_coordinates(glyph_id, coordinates)?[0];
        Ok(Some((i32::from(base) + delta).max(0)))
    }

    /// Returns the validated horizontal-advance slice for a raster batch.
    ///
    /// The table is parsed once and retained by the face. Callers that draw a
    /// run should hold this slice while walking glyph ids instead of invoking
    /// [`Self::glyph_advance_with_metrics`] for every glyph.
    pub(crate) fn glyph_advances_with_metrics(
        &self,
        metrics: FontMetrics,
    ) -> Result<&[u16], SfntError> {
        self.hmtx_cache
            .get_or_init(|| {
                self.required_table(*b"hmtx")
                    .and_then(|table| HmtxTable::parse(table, metrics))
            })
            .as_ref()
            .map_err(|error| *error)
            .map(|hmtx| hmtx.advances.as_slice())
    }

    /// Visits the glyph and horizontal advance for every character in a run.
    ///
    /// The cmap and hmtx parse results are acquired once before the loop. A
    /// shaping run can therefore avoid repeating the table lookup and
    /// `OnceLock`/`Result` handling that the single-glyph API intentionally
    /// keeps at its boundary. `false` means that at least one character is not
    /// covered by this face or maps outside its validated glyph range.
    pub(crate) fn for_each_glyph_with_advance<F>(
        &self,
        text: &str,
        metrics: FontMetrics,
        mut emit: F,
    ) -> Result<bool, SfntError>
    where
        F: FnMut(usize, u16, u16),
    {
        let Some(cmap) = self.table(*b"cmap") else {
            return Ok(false);
        };
        let parsed_cmap = self
            .cmap_cache
            .get_or_init(|| ParsedCmap::parse(cmap))
            .as_ref()
            .map_err(|error| *error)?;
        let hmtx = self
            .hmtx_cache
            .get_or_init(|| {
                self.required_table(*b"hmtx")
                    .and_then(|table| HmtxTable::parse(table, metrics))
            })
            .as_ref()
            .map_err(|error| *error)?;

        for (cluster, codepoint) in text.char_indices() {
            let Some(glyph_id) = parsed_cmap.glyph_index(codepoint as u32)? else {
                return Ok(false);
            };
            let Some(advance) = hmtx.advances.get(usize::from(glyph_id)).copied() else {
                return Ok(false);
            };
            emit(cluster, glyph_id, advance);
        }
        Ok(true)
    }

    /// Visits glyphs and advances while consuming Unicode variation-selector
    /// pairs as one glyph cluster.
    ///
    /// A format-14 `cmap` can map `(base, selector)` to a glyph that differs
    /// from the base character. The selector itself is default-ignorable and
    /// must not become a second glyph or advance. `false` means that the face
    /// cannot map every base/variation pair in `text`.
    pub(crate) fn for_each_glyph_with_advance_and_variations<F>(
        &self,
        text: &str,
        metrics: FontMetrics,
        mut emit: F,
    ) -> Result<bool, SfntError>
    where
        F: FnMut(usize, u16, u16),
    {
        let Some(cmap) = self.table(*b"cmap") else {
            return Ok(false);
        };
        let parsed_cmap = self
            .cmap_cache
            .get_or_init(|| ParsedCmap::parse(cmap))
            .as_ref()
            .map_err(|error| *error)?;
        let hmtx = self
            .hmtx_cache
            .get_or_init(|| {
                self.required_table(*b"hmtx")
                    .and_then(|table| HmtxTable::parse(table, metrics))
            })
            .as_ref()
            .map_err(|error| *error)?;

        let mut characters = text.char_indices().peekable();
        while let Some((cluster, codepoint)) = characters.next() {
            let glyph_id = if let Some(&(_, selector)) = characters.peek()
                && is_variation_selector(selector as u32)
            {
                characters.next();
                parsed_cmap.glyph_index_with_variation(
                    codepoint as u32,
                    selector as u32,
                    parsed_cmap.glyph_index(codepoint as u32)?,
                )?
            } else {
                parsed_cmap.glyph_index(codepoint as u32)?
            };
            let Some(glyph_id) = glyph_id else {
                return Ok(false);
            };
            let Some(advance) = hmtx.advances.get(usize::from(glyph_id)).copied() else {
                return Ok(false);
            };
            emit(cluster, glyph_id, advance);
        }
        Ok(true)
    }

    /// Decodes a name record from the OpenType `name` table.
    ///
    /// Unicode and Windows UTF-16BE records are preferred. Macintosh records
    /// are returned as a loss-tolerant byte string until the portable
    /// MacRoman decoder is added to the font subsystem.
    pub(crate) fn name(&self, name_id: u16) -> Result<Option<String>, SfntError> {
        let tag = Tag::from_bytes(*b"name");
        let Some(table) = self.table(*b"name") else {
            return Ok(None);
        };
        let reader = Reader::new(table);
        let format = reader.u16(0)?;
        if format > 1 {
            return Err(SfntError::MalformedTable(tag));
        }
        let count = reader.u16(2)?;
        let string_offset = usize::from(reader.u16(4)?);
        let records_size = checked_mul(usize::from(count), 12)?;
        let records_end = checked_add(6, records_size)?;
        reader.range(0, records_end)?;
        reader.range(0, string_offset)?;

        let mut fallback = None;
        for index in 0..count {
            let record_offset = checked_add(6, checked_mul(usize::from(index), 12)?)?;
            let platform = reader.u16(record_offset)?;
            let record_name_id = reader.u16(checked_add(record_offset, 6)?)?;
            if record_name_id != name_id {
                continue;
            }
            let length = usize::from(reader.u16(checked_add(record_offset, 8)?)?);
            let offset = usize::from(reader.u16(checked_add(record_offset, 10)?)?);
            let data_offset = checked_add(string_offset, offset)?;
            let data = reader
                .range(data_offset, length)
                .map_err(|_| SfntError::MalformedTable(tag))?;
            let decoded = if platform == 0 || platform == 3 {
                decode_utf16be(data, tag)?
            } else {
                String::from_utf8_lossy(data).into_owned()
            };
            if platform == 0 || platform == 3 {
                return Ok(Some(decoded));
            }
            if fallback.is_none() {
                fallback = Some(decoded);
            }
        }
        Ok(fallback)
    }

    /// Returns the typographic family name, falling back to the legacy family
    /// name identifier used by older OpenType fonts.
    pub(crate) fn family_name(&self) -> Result<Option<String>, SfntError> {
        if let Some(name) = self.name(16)? {
            return Ok(Some(name));
        }
        self.name(1)
    }

    fn required_table(&self, tag: [u8; 4]) -> Result<&[u8], SfntError> {
        self.table(tag)
            .ok_or(SfntError::MissingTable(Tag::from_bytes(tag)))
    }

    fn from_directory(bytes: &[u8], face_offset: u32) -> Result<Vec<TableRecord>, SfntError> {
        let reader = Reader::new(bytes);
        validate_face_offset(&reader, face_offset)?;
        let offset = usize::try_from(face_offset).map_err(|_| SfntError::ArithmeticOverflow)?;
        let signature = reader.tag(offset)?;
        if !is_sfnt_signature(signature) {
            return Err(SfntError::InvalidSignature(signature));
        }
        let table_count = reader.u16(checked_add(offset, 4)?)?;
        if table_count > MAX_TABLE_COUNT {
            return Err(SfntError::TooManyTables {
                count: table_count,
                max: MAX_TABLE_COUNT,
            });
        }

        let directory_size = checked_add(12, checked_mul(usize::from(table_count), 16)?)?;
        reader.range(offset, directory_size)?;

        let mut tables = Vec::with_capacity(usize::from(table_count));
        for index in 0..table_count {
            let record_offset = checked_add(
                offset,
                checked_add(12, checked_mul(usize::from(index), 16)?)?,
            )?;
            let tag = reader.tag(record_offset)?;
            if tables.iter().any(|record: &TableRecord| record.tag == tag) {
                return Err(SfntError::DuplicateTable(tag));
            }

            let table_offset = reader.u32(checked_add(record_offset, 8)?)?;
            let table_length = reader.u32(checked_add(record_offset, 12)?)?;
            if usize::try_from(table_length).map_err(|_| SfntError::ArithmeticOverflow)?
                > MAX_TABLE_BYTES
            {
                return Err(SfntError::TableTooLarge {
                    tag,
                    length: table_length,
                    max: MAX_TABLE_BYTES,
                });
            }

            let table_offset_usize =
                usize::try_from(table_offset).map_err(|_| SfntError::ArithmeticOverflow)?;
            let table_length_usize =
                usize::try_from(table_length).map_err(|_| SfntError::ArithmeticOverflow)?;
            let Some(table_end) = table_offset_usize.checked_add(table_length_usize) else {
                return Err(SfntError::ArithmeticOverflow);
            };
            if table_end > reader.bytes.len() {
                return Err(SfntError::TableOutOfBounds {
                    tag,
                    offset: table_offset,
                    length: table_length,
                });
            }

            tables.push(TableRecord {
                tag,
                offset: table_offset_usize,
                length: table_length_usize,
            });
        }

        Ok(tables)
    }
}

/// Parsed Aimer-owned state retained for one font face.
///
/// The face keeps the shared font storage alive, while the layout object
/// lazily retains validated GDEF/GSUB/GPOS metadata. Both shaping and
/// standard-outline rasterization can therefore reuse the same parse across
/// runs without making raster-only callers pay for layout parsing.
pub(crate) struct ParsedAimerFont {
    face: SfntFace<'static>,
    metrics: FontMetrics,
    layout: OnceLock<Result<layout::LayoutState, SfntError>>,
    raster_cache: Arc<rasterize::SharedGlyphRasterCache>,
    variation_instances: RwLock<VariationInstanceRegistry>,
}

const MAX_VARIATION_INSTANCES: usize = 1 << 16;

#[derive(Default)]
struct VariationInstanceRegistry {
    ids_by_coordinates: HashMap<Vec<i16>, u32>,
    coordinates_by_id: HashMap<u32, Arc<[f32]>>,
    next_id: u32,
}

impl VariationInstanceRegistry {
    fn intern(&mut self, mut coordinates: Vec<f32>) -> Option<u32> {
        let mut identity = Vec::with_capacity(coordinates.len());
        for coordinate in &mut coordinates {
            if !coordinate.is_finite() {
                return None;
            }
            let quantized = (*coordinate * 16_384.0).round().clamp(-16_384.0, 16_384.0) as i16;
            *coordinate = f32::from(quantized) / 16_384.0;
            identity.push(quantized);
        }
        if let Some(instance_id) = self.ids_by_coordinates.get(&identity).copied() {
            return Some(instance_id);
        }
        if self.coordinates_by_id.len() >= MAX_VARIATION_INSTANCES {
            return None;
        }

        let mut instance_id = self.next_id.max(1);
        loop {
            if !self.coordinates_by_id.contains_key(&instance_id) {
                break;
            }
            instance_id = instance_id.checked_add(1)?;
        }
        self.next_id = instance_id.checked_add(1).unwrap_or(1);
        let coordinates = Arc::from(coordinates.into_boxed_slice());
        self.ids_by_coordinates.insert(identity, instance_id);
        self.coordinates_by_id.insert(instance_id, coordinates);
        Some(instance_id)
    }

    fn coordinates(&self, instance_id: u32) -> Option<&[f32]> {
        self.coordinates_by_id.get(&instance_id).map(AsRef::as_ref)
    }
}

pub(crate) type SharedParsedAimerFont = Arc<ParsedAimerFont>;

impl ParsedAimerFont {
    fn layout(&self) -> Result<&layout::LayoutState, SfntError> {
        self.layout
            .get_or_init(|| layout::LayoutState::parse(&self.face))
            .as_ref()
            .map_err(|error| *error)
    }

    fn intern_variation_axes(&self, weight: u16, axes: &[(u32, f32)]) -> Option<u32> {
        let coordinates = self
            .face
            .normalized_variation_coordinates(weight, axes)
            .ok()??;
        self.variation_instances.write().ok()?.intern(coordinates)
    }

    fn with_variation_coordinates<R>(
        &self,
        weight: u16,
        variation_id: u32,
        f: impl FnOnce(&[f32]) -> R,
    ) -> R {
        if variation_id != 0
            && let Ok(instances) = self.variation_instances.read()
            && let Some(coordinates) = instances.coordinates(variation_id)
        {
            return f(coordinates);
        }
        let (coordinates, coordinate_count) = self.face.coordinates_for_weight_instance(weight);
        f(&coordinates[..coordinate_count])
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ParsedAimerFontKey {
    data_address: usize,
    data_len: usize,
    face_index: u32,
}

static PARSED_AIMER_FONTS: OnceLock<RwLock<HashMap<ParsedAimerFontKey, Arc<ParsedAimerFont>>>> =
    OnceLock::new();

/// The primary face is the hot path for every worker and every plain-text
/// rasterizer. Keep one lock-free handle for it after the first request; the
/// bounded map remains the fallback for registered and system faces.
static PRIMARY_AIMER_FONT: OnceLock<Option<SharedParsedAimerFont>> = OnceLock::new();

/// Keep parsed state warm across short-lived preparation contexts without
/// retaining an unbounded number of application-registered font faces.
const PARSED_AIMER_FONT_CACHE_CAPACITY: usize = 64;

fn parsed_aimer_fonts() -> &'static RwLock<HashMap<ParsedAimerFontKey, Arc<ParsedAimerFont>>> {
    PARSED_AIMER_FONTS.get_or_init(|| RwLock::new(HashMap::new()))
}

fn primary_aimer_font(data: FontData, face_index: u32) -> Option<SharedParsedAimerFont> {
    PRIMARY_AIMER_FONT
        .get_or_init(|| parsed_aimer_font(data, face_index).ok())
        .clone()
}

fn parsed_aimer_font(data: FontData, face_index: u32) -> Result<Arc<ParsedAimerFont>, SfntError> {
    let bytes = data.as_ref();
    let key = ParsedAimerFontKey {
        data_address: bytes.as_ptr() as usize,
        data_len: bytes.len(),
        face_index,
    };
    if let Ok(cache) = parsed_aimer_fonts().read()
        && let Some(parsed) = cache.get(&key)
    {
        return Ok(parsed.clone());
    }

    let face = SfntFace::from_font_data(data, face_index)?;
    let metrics = face.metrics()?;
    let parsed = Arc::new(ParsedAimerFont {
        face,
        metrics,
        layout: OnceLock::new(),
        raster_cache: Arc::new(rasterize::SharedGlyphRasterCache::default()),
        variation_instances: RwLock::new(VariationInstanceRegistry::default()),
    });

    if let Ok(mut cache) = parsed_aimer_fonts().write() {
        if let Some(existing) = cache.get(&key) {
            return Ok(existing.clone());
        }
        if cache.len() >= PARSED_AIMER_FONT_CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert(key, parsed.clone());
    }
    Ok(parsed)
}

/// Parses and compiles the shared state needed before a shaping batch enters
/// Rayon. The returned handle is immutable and can be cloned into every
/// worker without another table parse or layout compilation.
pub(crate) fn prewarm_font_data(
    data: FontData,
    face_index: u32,
) -> Result<SharedParsedAimerFont, SfntError> {
    let parsed = parsed_aimer_font(data, face_index)?;
    parsed.layout()?;
    Ok(parsed)
}

/// Returns the shared primary face without taking the parsed-face map lock
/// after its first construction. Callers must use this only for the fixed
/// primary face, never for a caller-selected fallback.
pub(crate) fn primary_state(data: FontData, face_index: u32) -> Option<AimerFontState> {
    primary_aimer_font(data, face_index).map(AimerFontState::from_shared)
}

/// Returns the fixed primary face metrics from the same lock-free handle used
/// by shaping and rasterization.
pub(crate) fn primary_metrics(data: FontData, face_index: u32) -> Option<FontMetrics> {
    primary_aimer_font(data, face_index).map(|parsed| parsed.metrics)
}

/// Returns metrics from the shared parsed face without constructing a second
/// temporary SFNT reader for the same font data.
pub(crate) fn metrics_from_font_data(
    data: FontData,
    face_index: u32,
) -> Result<FontMetrics, SfntError> {
    parsed_aimer_font(data, face_index).map(|parsed| parsed.metrics)
}

pub(crate) struct AimerFontState {
    shared: Arc<ParsedAimerFont>,
    raster_cache: rasterize::GlyphRasterCache,
}

impl AimerFontState {
    /// Builds a cacheable face and metrics record. OpenType layout state is
    /// parsed lazily on the first shaping call and retained thereafter.
    pub(crate) fn from_font_data(data: FontData, face_index: u32) -> Result<Self, SfntError> {
        let shared = parsed_aimer_font(data, face_index)?;
        Ok(Self::from_shared(shared))
    }

    /// Builds worker-local raster state around an already parsed immutable
    /// face. The parsed face, OpenType layout, and shared outline caches stay
    /// process-wide; only the small mutable raster handle is local.
    pub(crate) fn from_shared(shared: SharedParsedAimerFont) -> Self {
        let raster_cache = rasterize::GlyphRasterCache::with_shared(shared.raster_cache.clone());
        Self {
            shared,
            raster_cache,
        }
    }

    /// Returns the immutable parsed face so preparation workers can share it
    /// without rebuilding the SFNT reader or OpenType layout state.
    pub(crate) fn shared(&self) -> SharedParsedAimerFont {
        self.shared.clone()
    }

    /// Compiles this face's OpenType layout state before a worker batch starts.
    pub(crate) fn prewarm_layout(&self) -> Result<(), SfntError> {
        self.shared.layout().map(|_| ())
    }

    /// Returns the face's declared design weight from the shared SFNT state.
    pub(crate) fn design_weight(&self) -> Option<u16> {
        self.shared.face.design_weight()
    }

    /// Reports whether the shared face can select a `wght` variation.
    pub(crate) fn has_weight_variations(&self) -> bool {
        self.shared.face.has_weight_variations()
    }

    /// Reports whether this face exposes any arbitrary variation model.
    pub(crate) fn has_variations(&self) -> bool {
        self.shared.face.has_variations()
    }

    /// Reports whether this face has an HVAR store that can change horizontal
    /// advances for the selected variation coordinates.
    pub(crate) fn has_horizontal_metric_variations(&self) -> bool {
        self.shared.face.has_horizontal_metric_variations()
    }

    /// Interns arbitrary design-space axis values for this parsed face.
    ///
    /// The returned id is stable for the lifetime of the shared parsed face.
    /// Values are normalized, clamped, and quantized at the SFNT variation
    /// precision before interning, so equivalent requests share all caches.
    /// An empty request uses id zero, which preserves the fast weight-only
    /// path; invalid or unsupported requests return `None`.
    pub(crate) fn variation_instance_for_axes(
        &self,
        weight: u16,
        axes: &[(u32, f32)],
    ) -> Option<u32> {
        if axes.is_empty() {
            return Some(0);
        }
        self.shared.intern_variation_axes(weight, axes)
    }

    /// Shapes `text` through the cached face and layout state.
    pub(crate) fn shape_run(
        &self,
        text: &str,
    ) -> Result<Option<layout::AimerShapedRun>, SfntError> {
        self.shape_run_with_options(text, None, false)
    }

    /// Shapes `text` with an optional CJK language hint and vertical feature
    /// selection through the cached face and layout state.
    pub(crate) fn shape_run_with_options(
        &self,
        text: &str,
        language: Option<crate::font::TextLanguage>,
        vertical: bool,
    ) -> Result<Option<layout::AimerShapedRun>, SfntError> {
        self.shape_run_with_options_at_weight(text, language, vertical, NORMAL_GLYPH_WEIGHT)
    }

    /// Shapes a run while selecting the requested readable `wght` instance.
    /// Vertical CJK metrics use the same instance so glyph advances, origins,
    /// and outlines cannot disagree.
    pub(crate) fn shape_run_with_options_at_weight(
        &self,
        text: &str,
        language: Option<crate::font::TextLanguage>,
        vertical: bool,
        weight: u16,
    ) -> Result<Option<layout::AimerShapedRun>, SfntError> {
        let layout = self.shared.layout()?;
        self.shared
            .with_variation_coordinates(weight, 0, |coordinates| {
                layout::shape_run_with_layout_options_at_coordinates(
                    &self.shared.face,
                    layout,
                    text,
                    language,
                    vertical,
                    coordinates,
                )
            })
    }

    /// Shapes a run at an interned arbitrary variation instance.
    pub(crate) fn shape_run_with_options_at_variation(
        &self,
        text: &str,
        language: Option<crate::font::TextLanguage>,
        vertical: bool,
        weight: u16,
        variation_id: u32,
    ) -> Result<Option<layout::AimerShapedRun>, SfntError> {
        let layout = self.shared.layout()?;
        self.shared
            .with_variation_coordinates(weight, variation_id, |coordinates| {
                layout::shape_run_with_layout_options_at_coordinates(
                    &self.shared.face,
                    layout,
                    text,
                    language,
                    vertical,
                    coordinates,
                )
            })
    }

    /// Returns the selected HVAR horizontal advance delta in font units.
    pub(crate) fn horizontal_advance_delta(
        &self,
        glyph_id: u16,
        weight: u16,
    ) -> Result<i32, SfntError> {
        self.shared.face.horizontal_advance_delta(glyph_id, weight)
    }

    /// Returns HVAR's advance delta for an interned arbitrary variation
    /// instance.
    pub(crate) fn horizontal_advance_delta_at_variation(
        &self,
        glyph_id: u16,
        weight: u16,
        variation_id: u32,
    ) -> Result<i32, SfntError> {
        self.shared
            .with_variation_coordinates(weight, variation_id, |coordinates| {
                self.shared
                    .face
                    .horizontal_metric_deltas_at_coordinates(glyph_id, coordinates)
                    .map(|deltas| deltas[0])
            })
    }

    /// Returns a variable face's selected horizontal advance in pixels.
    pub(crate) fn advance_width_for_glyph_at_weight(
        &self,
        glyph_id: u16,
        font_size: f32,
        weight: u16,
    ) -> Option<f32> {
        self.advance_width_for_glyph_at_variation(glyph_id, font_size, weight, 0)
    }

    /// Returns a variable face's selected horizontal advance in pixels for an
    /// arbitrary interned variation instance.
    pub(crate) fn advance_width_for_glyph_at_variation(
        &self,
        glyph_id: u16,
        font_size: f32,
        weight: u16,
        variation_id: u32,
    ) -> Option<f32> {
        let advance = self
            .shared
            .with_variation_coordinates(weight, variation_id, |coordinates| {
                self.shared.face.glyph_advance_with_metrics_at_coordinates(
                    glyph_id,
                    self.shared.metrics,
                    coordinates,
                )
            })
            .ok()??;
        Some(advance as f32 * (font_size / f32::from(self.shared.metrics.units_per_em)))
    }

    /// Reports whether the current Aimer layout subset can handle `text`.
    ///
    /// Callers use this before touching the face's layout cache so an empty
    /// slice can be rejected without paying a repeated layout-state lookup.
    pub(crate) fn can_shape_text(text: &str) -> bool {
        layout::can_shape_text(text)
    }

    /// Reports whether the current Aimer layout subset can handle `text` with
    /// the supplied CJK language and vertical-substitution options.
    pub(crate) fn can_shape_text_with_options(
        text: &str,
        language: Option<crate::font::TextLanguage>,
        vertical: bool,
    ) -> bool {
        layout::can_shape_text_with_options(text, language, vertical)
    }

    /// Reports whether a paragraph-derived script hint enters an owned layout
    /// slice without rescanning every codepoint. The layout dispatcher still
    /// performs its checked script validation before returning shaped output.
    #[inline]
    pub(crate) fn can_shape_text_with_script_hint(
        text: &str,
        script: Option<crate::unicode_script::Script>,
        language: Option<crate::font::TextLanguage>,
        vertical: bool,
    ) -> bool {
        layout::can_shape_text_with_script_hint(text, script, language, vertical)
    }

    /// Looks up a Unicode scalar through the face-local parsed cmap.
    pub(crate) fn glyph_index(&self, codepoint: char) -> Option<u16> {
        self.shared
            .face
            .glyph_index(codepoint as u32)
            .ok()
            .flatten()
    }

    /// Rasterizes a pending glyph slice through the cached face and metrics.
    #[cfg(test)]
    pub(crate) fn rasterize_glyphs(
        &mut self,
        glyphs: &[(u16, u8, u8)],
        font_size: f32,
    ) -> Vec<Option<RasterizedGlyph>> {
        rasterize::rasterize_face_glyphs_cached(
            &self.shared.face,
            &self.shared.metrics,
            glyphs,
            font_size,
            &mut self.raster_cache,
        )
    }

    /// Rasterizes a batch and emits successful glyphs directly to the caller.
    ///
    /// The callback receives each original key together with its completed
    /// bitmap. Failed glyphs are omitted so the caller can route only those
    /// keys through its compatibility fallback. The return value is `true`
    /// only when every requested key was rasterized by Aimer.
    pub(crate) fn rasterize_glyphs_into<F>(
        &mut self,
        glyphs: &[GlyphKey],
        font_size: f32,
        emit: F,
    ) -> bool
    where
        F: FnMut(GlyphKey, RasterizedGlyph),
    {
        let shared = self.shared.clone();
        rasterize::rasterize_face_glyphs_into_with_coordinates(
            &self.shared.face,
            &self.shared.metrics,
            glyphs,
            font_size,
            &mut self.raster_cache,
            move |key, visit| {
                shared.with_variation_coordinates(key.weight, key.variation_id, visit);
            },
            emit,
        )
    }

    #[cfg(test)]
    pub(crate) fn outline_cache_len(&self) -> usize {
        self.raster_cache.outline_cache_len()
    }

    #[cfg(test)]
    pub(crate) fn flattened_edge_cache_len(&self) -> usize {
        self.raster_cache.flattened_edge_cache_len()
    }
}

/// Validates the minimum face tables required by the portable font path.
///
/// Parsing the directory alone is not enough: a face without horizontal
/// metrics can be structurally valid SFNT data while still being unusable for
/// shaping, layout, or rasterization. This entry point keeps registration on
/// the same checked reader used by glyph rendering.
pub(crate) fn validate_font(bytes: &[u8]) -> Result<(), SfntError> {
    let face = SfntFace::from_bytes(bytes, 0)?;
    face.metrics()?;
    if face.table(*b"cmap").is_none() {
        return Err(SfntError::MissingTable(Tag::from_bytes(*b"cmap")));
    }
    Ok(())
}

#[derive(Clone)]
struct ParsedCmap {
    format12: Option<Vec<CmapGroup>>,
    format4: Option<Vec<CmapSegment>>,
    format0: Option<Vec<u16>>,
    format14: Option<Result<Vec<VariationSelectorRecord>, SfntError>>,
}

#[derive(Clone, Copy)]
struct CmapGroup {
    start: u32,
    end: u32,
    start_glyph: u32,
}

#[derive(Clone)]
struct CmapSegment {
    start: u16,
    end: u16,
    delta: i16,
    glyphs: Option<Vec<u16>>,
}

#[derive(Clone)]
struct VariationSelectorRecord {
    selector: u32,
    default_ranges: Vec<UnicodeRange>,
    non_default_mappings: Vec<VariationMapping>,
}

#[derive(Clone, Copy)]
struct UnicodeRange {
    start: u32,
    end: u32,
}

#[derive(Clone, Copy)]
struct VariationMapping {
    codepoint: u32,
    glyph_id: u16,
}

impl ParsedCmap {
    fn parse(bytes: &[u8]) -> Result<Self, SfntError> {
        let cmap = Cmap::new(bytes)?;
        let mut format12 = None;
        let mut format4 = None;
        let mut format0 = None;
        let mut format14 = None;

        for index in 0..cmap.records {
            let subtable = cmap.subtable(index)?;
            match Reader::new(subtable).u16(0)? {
                12 if format12.is_none() => {
                    format12 = Some(parse_format12_cmap(subtable)?);
                }
                4 if format4.is_none() => {
                    format4 = Some(parse_format4_cmap(subtable)?);
                }
                0 if format0.is_none() => {
                    format0 = Some(parse_format0_cmap(subtable)?);
                }
                14 if format14.is_none() => {
                    // Keep optional format-14 errors local to variation
                    // lookup. A malformed UVS table must not invalidate the
                    // ordinary base cmap used by every other glyph.
                    format14 = Some(parse_format14_cmap(subtable));
                }
                _ => {}
            }
        }

        Ok(Self {
            format12,
            format4,
            format0,
            format14,
        })
    }

    fn glyph_index(&self, codepoint: u32) -> Result<Option<u16>, SfntError> {
        if let Some(groups) = &self.format12 {
            let mut low = 0;
            let mut high = groups.len();
            while low < high {
                let index = low + (high - low) / 2;
                let group = groups[index];
                if codepoint < group.start {
                    high = index;
                } else if codepoint > group.end {
                    low = index + 1;
                } else {
                    let glyph = group
                        .start_glyph
                        .checked_add(codepoint - group.start)
                        .ok_or(SfntError::CmapGlyphOutOfRange(u32::MAX))?;
                    return Ok(nonzero_glyph(
                        u16::try_from(glyph).map_err(|_| SfntError::CmapGlyphOutOfRange(glyph))?,
                    ));
                }
            }
            return Ok(None);
        }

        if let Some(segments) = &self.format4 {
            if codepoint > u32::from(u16::MAX) {
                return Ok(None);
            }
            let codepoint = codepoint as u16;
            let mut low = 0;
            let mut high = segments.len();
            while low < high {
                let index = low + (high - low) / 2;
                let segment = &segments[index];
                if codepoint < segment.start {
                    high = index;
                } else if codepoint > segment.end {
                    low = index + 1;
                } else {
                    let glyph = segment.glyphs.as_ref().map_or_else(
                        || add_delta(codepoint, segment.delta),
                        |glyphs| glyphs[usize::from(codepoint - segment.start)],
                    );
                    return Ok(nonzero_glyph(if segment.glyphs.is_some() {
                        add_delta(glyph, segment.delta)
                    } else {
                        glyph
                    }));
                }
            }
            return Ok(None);
        }

        if let Some(glyphs) = &self.format0 {
            if codepoint <= 0xff {
                return Ok(nonzero_glyph(glyphs[codepoint as usize]));
            }
        }
        Ok(None)
    }

    fn glyph_index_with_variation(
        &self,
        codepoint: u32,
        selector: u32,
        base_glyph: Option<u16>,
    ) -> Result<Option<u16>, SfntError> {
        let Some(format14) = &self.format14 else {
            return Ok(base_glyph);
        };
        let records = format14.as_ref().map_err(|error| *error)?;
        let mut low = 0;
        let mut high = records.len();
        while low < high {
            let index = low + (high - low) / 2;
            let record = &records[index];
            if selector < record.selector {
                high = index;
            } else if selector > record.selector {
                low = index + 1;
            } else {
                let mut mapping_low = 0;
                let mut mapping_high = record.non_default_mappings.len();
                while mapping_low < mapping_high {
                    let mapping_index = mapping_low + (mapping_high - mapping_low) / 2;
                    let mapping = record.non_default_mappings[mapping_index];
                    if codepoint < mapping.codepoint {
                        mapping_high = mapping_index;
                    } else if codepoint > mapping.codepoint {
                        mapping_low = mapping_index + 1;
                    } else {
                        return Ok(nonzero_glyph(mapping.glyph_id));
                    }
                }

                let mut range_low = 0;
                let mut range_high = record.default_ranges.len();
                while range_low < range_high {
                    let range_index = range_low + (range_high - range_low) / 2;
                    let range = record.default_ranges[range_index];
                    if codepoint < range.start {
                        range_high = range_index;
                    } else if codepoint > range.end {
                        range_low = range_index + 1;
                    } else {
                        return Ok(base_glyph);
                    }
                }
                return Ok(None);
            }
        }
        Ok(base_glyph)
    }
}

fn parse_format0_cmap(subtable: &[u8]) -> Result<Vec<u16>, SfntError> {
    let reader = Reader::new(subtable);
    let length = usize::from(reader.u16(2)?);
    if length < 262 || length > subtable.len() {
        return Err(SfntError::MalformedCmap(0));
    }
    let bytes = reader.range(6, 256)?;
    Ok(bytes
        .iter()
        .map(|glyph| u16::from(*glyph))
        .collect::<Vec<_>>())
}

fn parse_format4_cmap(subtable: &[u8]) -> Result<Vec<CmapSegment>, SfntError> {
    let reader = Reader::new(subtable);
    let length = usize::from(reader.u16(2)?);
    if length < 16 || length > subtable.len() {
        return Err(SfntError::MalformedCmap(4));
    }
    let reader = Reader::new(&subtable[..length]);
    let segments_x2 = reader.u16(6)?;
    if segments_x2 == 0 || segments_x2 % 2 != 0 {
        return Err(SfntError::MalformedCmap(4));
    }
    let segments = usize::from(segments_x2 / 2);
    let end_codes = 14;
    let reserved_pad = checked_add(end_codes, checked_mul(segments, 2)?)?;
    let start_codes = checked_add(reserved_pad, 2)?;
    let id_deltas = checked_add(start_codes, checked_mul(segments, 2)?)?;
    let id_range_offsets = checked_add(id_deltas, checked_mul(segments, 2)?)?;
    let array_end = checked_add(id_range_offsets, checked_mul(segments, 2)?)?;
    reader.range(0, array_end)?;

    let mut parsed = Vec::with_capacity(segments);
    let mut previous_end = None;
    for index in 0..segments {
        let end = reader.u16(checked_add(end_codes, checked_mul(index, 2)?)?)?;
        let start = reader.u16(checked_add(start_codes, checked_mul(index, 2)?)?)?;
        if start > end || previous_end.is_some_and(|value| start <= value) {
            return Err(SfntError::MalformedCmap(4));
        }
        previous_end = Some(end);

        let delta = reader.i16(checked_add(id_deltas, checked_mul(index, 2)?)?)?;
        let range_offset_position = checked_add(id_range_offsets, checked_mul(index, 2)?)?;
        let range_offset = reader.u16(range_offset_position)?;
        let glyphs = if range_offset == 0 {
            None
        } else {
            let count = usize::from(end - start) + 1;
            let mut glyphs = Vec::with_capacity(count);
            for glyph_index in 0..count {
                let glyph_offset = checked_add(
                    checked_add(range_offset_position, usize::from(range_offset))?,
                    checked_mul(glyph_index, 2)?,
                )?;
                glyphs.push(reader.u16(glyph_offset)?);
            }
            Some(glyphs)
        };
        parsed.push(CmapSegment {
            start,
            end,
            delta,
            glyphs,
        });
    }
    Ok(parsed)
}

fn parse_format12_cmap(subtable: &[u8]) -> Result<Vec<CmapGroup>, SfntError> {
    let reader = Reader::new(subtable);
    let length = usize::try_from(reader.u32(4)?).map_err(|_| SfntError::ArithmeticOverflow)?;
    if length < 16 || length > subtable.len() {
        return Err(SfntError::MalformedCmap(12));
    }
    let reader = Reader::new(&subtable[..length]);
    let groups = reader.u32(12)?;
    if groups > MAX_CMAP_GROUPS {
        return Err(SfntError::MalformedCmap(12));
    }
    let groups_usize = usize::try_from(groups).map_err(|_| SfntError::ArithmeticOverflow)?;
    let groups_end = checked_add(16, checked_mul(groups_usize, 12)?)?;
    reader.range(0, groups_end)?;

    let mut parsed = Vec::with_capacity(groups_usize);
    let mut previous_end = None;
    for index in 0..groups_usize {
        let offset = checked_add(16, checked_mul(index, 12)?)?;
        let start = reader.u32(offset)?;
        let end = reader.u32(checked_add(offset, 4)?)?;
        if start > end || end > 0x0010_ffff || previous_end.is_some_and(|value| start <= value) {
            return Err(SfntError::MalformedCmap(12));
        }
        let start_glyph = reader.u32(checked_add(offset, 8)?)?;
        let last_glyph = start_glyph
            .checked_add(end - start)
            .ok_or(SfntError::CmapGlyphOutOfRange(u32::MAX))?;
        if last_glyph > u32::from(u16::MAX) {
            return Err(SfntError::CmapGlyphOutOfRange(start_glyph));
        }
        previous_end = Some(end);
        parsed.push(CmapGroup {
            start,
            end,
            start_glyph,
        });
    }
    Ok(parsed)
}

fn parse_format14_cmap(subtable: &[u8]) -> Result<Vec<VariationSelectorRecord>, SfntError> {
    let reader = Reader::new(subtable);
    let length = usize::try_from(reader.u32(2)?).map_err(|_| SfntError::ArithmeticOverflow)?;
    if length < 10 || length > subtable.len() {
        return Err(SfntError::MalformedCmap(14));
    }
    let reader = Reader::new(&subtable[..length]);
    let record_count = reader.u32(6)?;
    if record_count > MAX_CMAP_VARIATION_RECORDS {
        return Err(SfntError::MalformedCmap(14));
    }
    let record_count = usize::try_from(record_count).map_err(|_| SfntError::ArithmeticOverflow)?;
    let records_end = checked_add(10, checked_mul(record_count, 11)?)?;
    reader.range(0, records_end)?;

    let mut records = Vec::with_capacity(record_count);
    let mut previous_selector = None;
    let mut total_entries = 0_u32;
    for index in 0..record_count {
        let offset = checked_add(10, checked_mul(index, 11)?)?;
        let selector = reader.u24(offset)?;
        if !is_variation_selector(selector)
            || previous_selector.is_some_and(|previous| selector <= previous)
        {
            return Err(SfntError::MalformedCmap(14));
        }
        previous_selector = Some(selector);

        let default_offset = reader.u32(checked_add(offset, 3)?)?;
        let non_default_offset = reader.u32(checked_add(offset, 7)?)?;
        let default_ranges = if default_offset == 0 {
            Vec::new()
        } else {
            parse_default_uvs_ranges(&subtable[..length], default_offset)?
        };
        let non_default_mappings = if non_default_offset == 0 {
            Vec::new()
        } else {
            parse_non_default_uvs_mappings(&subtable[..length], non_default_offset)?
        };
        let default_count =
            u32::try_from(default_ranges.len()).map_err(|_| SfntError::ArithmeticOverflow)?;
        let non_default_count =
            u32::try_from(non_default_mappings.len()).map_err(|_| SfntError::ArithmeticOverflow)?;
        total_entries = total_entries
            .checked_add(default_count)
            .and_then(|value| value.checked_add(non_default_count))
            .ok_or(SfntError::MalformedCmap(14))?;
        if total_entries > MAX_CMAP_VARIATION_ENTRIES {
            return Err(SfntError::MalformedCmap(14));
        }
        records.push(VariationSelectorRecord {
            selector,
            default_ranges,
            non_default_mappings,
        });
    }
    Ok(records)
}

fn parse_default_uvs_ranges(subtable: &[u8], offset: u32) -> Result<Vec<UnicodeRange>, SfntError> {
    let offset = usize::try_from(offset).map_err(|_| SfntError::ArithmeticOverflow)?;
    let reader = Reader::new(subtable);
    let count = reader.u32(offset)?;
    if count > MAX_CMAP_VARIATION_ENTRIES {
        return Err(SfntError::MalformedCmap(14));
    }
    let count = usize::try_from(count).map_err(|_| SfntError::ArithmeticOverflow)?;
    let end = checked_add(offset, checked_add(4, checked_mul(count, 4)?)?)?;
    reader.range(0, end)?;

    let mut ranges = Vec::with_capacity(count);
    let mut previous_end = None;
    for index in 0..count {
        let record = checked_add(offset, checked_add(4, checked_mul(index, 4)?)?)?;
        let start = reader.u24(record)?;
        let additional = u32::from(reader.range(checked_add(record, 3)?, 1)?[0]);
        let end = start
            .checked_add(additional)
            .ok_or(SfntError::MalformedCmap(14))?;
        if end > 0x0010_ffff || previous_end.is_some_and(|previous| start <= previous) {
            return Err(SfntError::MalformedCmap(14));
        }
        previous_end = Some(end);
        ranges.push(UnicodeRange { start, end });
    }
    Ok(ranges)
}

fn parse_non_default_uvs_mappings(
    subtable: &[u8],
    offset: u32,
) -> Result<Vec<VariationMapping>, SfntError> {
    let offset = usize::try_from(offset).map_err(|_| SfntError::ArithmeticOverflow)?;
    let reader = Reader::new(subtable);
    let count = reader.u32(offset)?;
    if count > MAX_CMAP_VARIATION_ENTRIES {
        return Err(SfntError::MalformedCmap(14));
    }
    let count = usize::try_from(count).map_err(|_| SfntError::ArithmeticOverflow)?;
    let end = checked_add(offset, checked_add(4, checked_mul(count, 5)?)?)?;
    reader.range(0, end)?;

    let mut mappings = Vec::with_capacity(count);
    let mut previous_codepoint = None;
    for index in 0..count {
        let record = checked_add(offset, checked_add(4, checked_mul(index, 5)?)?)?;
        let codepoint = reader.u24(record)?;
        if previous_codepoint.is_some_and(|previous| codepoint <= previous) {
            return Err(SfntError::MalformedCmap(14));
        }
        previous_codepoint = Some(codepoint);
        mappings.push(VariationMapping {
            codepoint,
            glyph_id: reader.u16(checked_add(record, 3)?)?,
        });
    }
    Ok(mappings)
}

#[inline]
fn is_variation_selector(value: u32) -> bool {
    matches!(value, 0xfe00..=0xfe0f | 0xe0100..=0xe01ef)
}

struct Cmap<'a> {
    bytes: &'a [u8],
    records: u16,
}

impl<'a> Cmap<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, SfntError> {
        let reader = Reader::new(bytes);
        if reader.u16(0)? != 0 {
            return Err(SfntError::MalformedCmap(0));
        }
        let records = reader.u16(2)?;
        let records_size = checked_mul(usize::from(records), 8)?;
        reader.range(4, records_size)?;
        Ok(Self { bytes, records })
    }

    fn glyph_index(&self, codepoint: u32) -> Result<Option<u16>, SfntError> {
        let mut format12 = None;
        let mut format4 = None;
        let mut format0 = None;

        for index in 0..self.records {
            let subtable = self.subtable(index)?;
            let format = Reader::new(subtable).u16(0)?;
            match format {
                12 if format12.is_none() => format12 = Some(subtable),
                4 if format4.is_none() => format4 = Some(subtable),
                0 if format0.is_none() => format0 = Some(subtable),
                _ => {}
            }
        }

        if let Some(subtable) = format12 {
            return lookup_format12(subtable, codepoint);
        }
        if let Some(subtable) = format4 {
            return lookup_format4(subtable, codepoint);
        }
        if let Some(subtable) = format0 {
            return lookup_format0(subtable, codepoint);
        }
        Ok(None)
    }

    fn glyph_index_with_variation(
        &self,
        codepoint: u32,
        selector: u32,
        base_glyph: Option<u16>,
    ) -> Result<Option<u16>, SfntError> {
        let Some(subtable) = self.subtable_with_format(14)? else {
            return Ok(base_glyph);
        };
        lookup_format14(subtable, codepoint, selector, base_glyph)
    }

    fn subtable_with_format(&self, wanted_format: u16) -> Result<Option<&'a [u8]>, SfntError> {
        for index in 0..self.records {
            let subtable = self.subtable(index)?;
            if Reader::new(subtable).u16(0)? == wanted_format {
                return Ok(Some(subtable));
            }
        }
        Ok(None)
    }

    fn subtable(&self, index: u16) -> Result<&'a [u8], SfntError> {
        let offset = checked_add(4, checked_mul(usize::from(index), 8)?)?;
        let offset = Reader::new(self.bytes).u32(checked_add(offset, 4)?)?;
        let offset_usize = usize::try_from(offset).map_err(|_| SfntError::ArithmeticOverflow)?;
        self.bytes
            .get(offset_usize..)
            .ok_or(SfntError::CmapSubtableOutOfBounds(offset))
    }
}

fn lookup_format0(subtable: &[u8], codepoint: u32) -> Result<Option<u16>, SfntError> {
    let reader = Reader::new(subtable);
    let length = usize::from(reader.u16(2)?);
    if length < 262 || length > subtable.len() {
        return Err(SfntError::MalformedCmap(0));
    }
    if codepoint > 0xff {
        return Ok(None);
    }
    let glyph = subtable[6 + codepoint as usize] as u16;
    Ok((glyph != 0).then_some(glyph))
}

fn lookup_format4(subtable: &[u8], codepoint: u32) -> Result<Option<u16>, SfntError> {
    let reader = Reader::new(subtable);
    let length = usize::from(reader.u16(2)?);
    if length < 16 || length > subtable.len() {
        return Err(SfntError::MalformedCmap(4));
    }
    if codepoint > u32::from(u16::MAX) {
        return Ok(None);
    }

    let reader = Reader::new(&subtable[..length]);
    let segments_x2 = reader.u16(6)?;
    if segments_x2 == 0 || segments_x2 % 2 != 0 {
        return Err(SfntError::MalformedCmap(4));
    }
    let segments = usize::from(segments_x2 / 2);
    let end_codes = 14;
    let reserved_pad = checked_add(end_codes, checked_mul(segments, 2)?)?;
    let start_codes = checked_add(reserved_pad, 2)?;
    let id_deltas = checked_add(start_codes, checked_mul(segments, 2)?)?;
    let id_range_offsets = checked_add(id_deltas, checked_mul(segments, 2)?)?;
    let array_end = checked_add(id_range_offsets, checked_mul(segments, 2)?)?;
    reader.range(0, array_end)?;

    let codepoint = codepoint as u16;
    for index in 0..segments {
        let end = reader.u16(checked_add(end_codes, checked_mul(index, 2)?)?)?;
        let start = reader.u16(checked_add(start_codes, checked_mul(index, 2)?)?)?;
        if start > end {
            return Err(SfntError::MalformedCmap(4));
        }
        if codepoint < start || codepoint > end {
            continue;
        }

        let delta = reader.i16(checked_add(id_deltas, checked_mul(index, 2)?)?)?;
        let range_offset_position = checked_add(id_range_offsets, checked_mul(index, 2)?)?;
        let range_offset = reader.u16(range_offset_position)?;
        if range_offset == 0 {
            return Ok(nonzero_glyph(add_delta(codepoint, delta)));
        }

        let glyph_offset = checked_add(
            checked_add(range_offset_position, usize::from(range_offset))?,
            checked_mul(usize::from(codepoint - start), 2)?,
        )?;
        let glyph = reader.u16(glyph_offset)?;
        if glyph == 0 {
            return Ok(None);
        }
        return Ok(nonzero_glyph(add_delta(glyph, delta)));
    }

    Ok(None)
}

fn lookup_format12(subtable: &[u8], codepoint: u32) -> Result<Option<u16>, SfntError> {
    let reader = Reader::new(subtable);
    let length = usize::try_from(reader.u32(4)?).map_err(|_| SfntError::ArithmeticOverflow)?;
    if length < 16 || length > subtable.len() {
        return Err(SfntError::MalformedCmap(12));
    }
    let reader = Reader::new(&subtable[..length]);
    let groups = reader.u32(12)?;
    if groups > MAX_CMAP_GROUPS {
        return Err(SfntError::MalformedCmap(12));
    }
    let group_bytes = checked_mul(
        usize::try_from(groups).map_err(|_| SfntError::ArithmeticOverflow)?,
        12,
    )?;
    let groups_end = checked_add(16, group_bytes)?;
    reader.range(0, groups_end)?;

    let mut previous_end = None;
    for index in 0..groups {
        let offset = checked_add(
            16,
            checked_mul(
                usize::try_from(index).map_err(|_| SfntError::ArithmeticOverflow)?,
                12,
            )?,
        )?;
        let start = reader.u32(offset)?;
        let end = reader.u32(checked_add(offset, 4)?)?;
        if start > end || end > 0x0010_ffff || previous_end.is_some_and(|value| start <= value) {
            return Err(SfntError::MalformedCmap(12));
        }
        previous_end = Some(end);
    }

    let mut low = 0_u32;
    let mut high = groups;
    while low < high {
        let index = low + (high - low) / 2;
        let offset = checked_add(
            16,
            checked_mul(
                usize::try_from(index).map_err(|_| SfntError::ArithmeticOverflow)?,
                12,
            )?,
        )?;
        let start = reader.u32(offset)?;
        let end = reader.u32(checked_add(offset, 4)?)?;
        if codepoint < start {
            high = index;
        } else if codepoint > end {
            low = index + 1;
        } else {
            let start_glyph = reader.u32(checked_add(offset, 8)?)?;
            let delta = codepoint - start;
            let glyph = start_glyph
                .checked_add(delta)
                .ok_or(SfntError::CmapGlyphOutOfRange(u32::MAX))?;
            if glyph > u32::from(u16::MAX) {
                return Err(SfntError::CmapGlyphOutOfRange(glyph));
            }
            return Ok(nonzero_glyph(glyph as u16));
        }
    }

    Ok(None)
}

fn lookup_format14(
    subtable: &[u8],
    codepoint: u32,
    selector: u32,
    base_glyph: Option<u16>,
) -> Result<Option<u16>, SfntError> {
    let reader = Reader::new(subtable);
    let length = usize::try_from(reader.u32(2)?).map_err(|_| SfntError::ArithmeticOverflow)?;
    if length < 10 || length > subtable.len() {
        return Err(SfntError::MalformedCmap(14));
    }
    let reader = Reader::new(&subtable[..length]);
    let records = reader.u32(6)?;
    let records_bytes = checked_mul(
        usize::try_from(records).map_err(|_| SfntError::ArithmeticOverflow)?,
        11,
    )?;
    let records_end = checked_add(10, records_bytes)?;
    reader.range(0, records_end)?;

    for index in 0..records {
        let offset = checked_add(
            10,
            checked_mul(
                usize::try_from(index).map_err(|_| SfntError::ArithmeticOverflow)?,
                11,
            )?,
        )?;
        let record_selector = reader.u24(offset)?;
        let default_offset = reader.u32(checked_add(offset, 3)?)?;
        let non_default_offset = reader.u32(checked_add(offset, 7)?)?;
        if record_selector != selector {
            continue;
        }

        if non_default_offset != 0
            && let Some(glyph) =
                lookup_non_default_uvs(&subtable[..length], non_default_offset, codepoint)?
        {
            return Ok(nonzero_glyph(glyph));
        }
        if default_offset != 0 && is_default_uvs(&subtable[..length], default_offset, codepoint)? {
            return Ok(base_glyph);
        }
        return Ok(None);
    }

    Ok(base_glyph)
}

fn lookup_non_default_uvs(
    subtable: &[u8],
    offset: u32,
    codepoint: u32,
) -> Result<Option<u16>, SfntError> {
    let offset = usize::try_from(offset).map_err(|_| SfntError::ArithmeticOverflow)?;
    let reader = Reader::new(subtable);
    let mappings = reader.u32(offset)?;
    let mapping_bytes = checked_mul(
        usize::try_from(mappings).map_err(|_| SfntError::ArithmeticOverflow)?,
        5,
    )?;
    let end = checked_add(checked_add(offset, 4)?, mapping_bytes)?;
    reader.range(0, end)?;

    for index in 0..mappings {
        let record_offset = checked_add(
            checked_add(offset, 4)?,
            checked_mul(
                usize::try_from(index).map_err(|_| SfntError::ArithmeticOverflow)?,
                5,
            )?,
        )?;
        let unicode = reader.u24(record_offset)?;
        if unicode == codepoint {
            return Ok(Some(reader.u16(checked_add(record_offset, 3)?)?));
        }
    }
    Ok(None)
}

fn is_default_uvs(subtable: &[u8], offset: u32, codepoint: u32) -> Result<bool, SfntError> {
    let offset = usize::try_from(offset).map_err(|_| SfntError::ArithmeticOverflow)?;
    let reader = Reader::new(subtable);
    let ranges = reader.u32(offset)?;
    let range_bytes = checked_mul(
        usize::try_from(ranges).map_err(|_| SfntError::ArithmeticOverflow)?,
        4,
    )?;
    let end = checked_add(checked_add(offset, 4)?, range_bytes)?;
    reader.range(0, end)?;

    for index in 0..ranges {
        let record_offset = checked_add(
            checked_add(offset, 4)?,
            checked_mul(
                usize::try_from(index).map_err(|_| SfntError::ArithmeticOverflow)?,
                4,
            )?,
        )?;
        let start = reader.u24(record_offset)?;
        let additional = u32::from(reader.range(checked_add(record_offset, 3)?, 1)?[0]);
        let end = start
            .checked_add(additional)
            .ok_or(SfntError::MalformedCmap(14))?;
        if end > 0x0010_ffff {
            return Err(SfntError::MalformedCmap(14));
        }
        if (start..=end).contains(&codepoint) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn add_delta(glyph: u16, delta: i16) -> u16 {
    (i32::from(glyph) + i32::from(delta)).rem_euclid(1 << 16) as u16
}

fn nonzero_glyph(glyph: u16) -> Option<u16> {
    (glyph != 0).then_some(glyph)
}

fn decode_utf16be(data: &[u8], tag: Tag) -> Result<String, SfntError> {
    if !data.len().is_multiple_of(2) {
        return Err(SfntError::MalformedTable(tag));
    }
    let mut words = Vec::with_capacity(data.len() / 2);
    for pair in data.as_chunks::<2>().0 {
        words.push(u16::from_be_bytes(*pair));
    }
    Ok(String::from_utf16_lossy(&words))
}

#[derive(Clone, Copy)]
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn range(&self, offset: usize, size: usize) -> Result<&'a [u8], SfntError> {
        let end = offset
            .checked_add(size)
            .ok_or(SfntError::ArithmeticOverflow)?;
        self.bytes
            .get(offset..end)
            .ok_or(SfntError::Truncated { offset, size })
    }

    fn tag(&self, offset: usize) -> Result<Tag, SfntError> {
        let bytes = self.range(offset, 4)?;
        Ok(Tag::from_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u16(&self, offset: usize) -> Result<u16, SfntError> {
        let bytes = self.range(offset, 2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u8(&self, offset: usize) -> Result<u8, SfntError> {
        Ok(self.range(offset, 1)?[0])
    }

    fn i8(&self, offset: usize) -> Result<i8, SfntError> {
        Ok(self.u8(offset)? as i8)
    }

    fn i16(&self, offset: usize) -> Result<i16, SfntError> {
        let bytes = self.range(offset, 2)?;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&self, offset: usize) -> Result<u32, SfntError> {
        let bytes = self.range(offset, 4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u24(&self, offset: usize) -> Result<u32, SfntError> {
        let bytes = self.range(offset, 3)?;
        Ok((u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]))
    }
}

fn checked_add(left: usize, right: usize) -> Result<usize, SfntError> {
    left.checked_add(right).ok_or(SfntError::ArithmeticOverflow)
}

fn checked_mul(left: usize, right: usize) -> Result<usize, SfntError> {
    left.checked_mul(right).ok_or(SfntError::ArithmeticOverflow)
}

fn validate_face_offset(reader: &Reader<'_>, offset: u32) -> Result<(), SfntError> {
    let offset = usize::try_from(offset).map_err(|_| SfntError::ArithmeticOverflow)?;
    if offset > reader.bytes.len() {
        return Err(SfntError::InvalidFaceOffset(offset as u32));
    }
    reader.range(offset, 12).map(|_| ())
}

fn is_sfnt_signature(tag: Tag) -> bool {
    tag == Tag(*b"OTTO")
        || tag == Tag(*b"true")
        || tag == Tag(*b"typ1")
        || tag == Tag::from_bytes([0, 1, 0, 0])
}

const MAX_FUZZ_FONT_BYTES: usize = 4 * 1024 * 1024;

/// Exercises the bounded SFNT reader from a cargo-fuzz target.
pub fn fuzz_directory(data: &[u8]) {
    let Some((&selector, font_bytes)) = data.split_first() else {
        return;
    };
    let font_bytes = &font_bytes[..font_bytes.len().min(MAX_FUZZ_FONT_BYTES)];
    let face_index = u32::from(selector & 0x0f);
    let Ok(face) = SfntFace::from_bytes(font_bytes, face_index) else {
        return;
    };

    for tag in [
        *b"head", *b"maxp", *b"hhea", *b"hmtx", *b"cmap", *b"glyf", *b"loca", *b"CFF ", *b"CFF2",
    ] {
        std::hint::black_box(face.table(tag));
    }
    let _ = std::hint::black_box(face.metrics());
    for codepoint in [0, 0x20, 0x41, 0x4e00, 0x1f600, 0x0010_ffff] {
        let _ = std::hint::black_box(face.glyph_index(codepoint));
        let _ = std::hint::black_box(face.glyph_index_with_variation(codepoint, 0xfe0f));
    }
    let _ = std::hint::black_box(face.family_name());
}

/// Exercises glyph, composite, CFF, and CFF2 outline paths from a cargo-fuzz
/// target while keeping each iteration's work bounded.
pub fn fuzz_outlines(data: &[u8]) {
    let Some((&selector, font_bytes)) = data.split_first() else {
        return;
    };
    let font_bytes = &font_bytes[..font_bytes.len().min(MAX_FUZZ_FONT_BYTES)];
    let face_index = u32::from(selector & 0x0f);
    let Ok(face) = SfntFace::from_bytes(font_bytes, face_index) else {
        return;
    };
    let Ok(metrics) = face.metrics() else {
        return;
    };
    let mut glyph_ids = [0_u16; 66];
    glyph_ids[0] = 0;
    glyph_ids[1] = metrics.num_glyphs.saturating_sub(1);
    let mut count = 2;
    for chunk in font_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .take(glyph_ids.len() - count)
    {
        glyph_ids[count] = u16::from_be_bytes(*chunk);
        count += 1;
    }
    for glyph_id in glyph_ids.into_iter().take(count) {
        let _ = std::hint::black_box(face.outline(glyph_id));
        let _ = std::hint::black_box(face.cff_outline(glyph_id));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::cff::CffPathCommand;
    use super::{MAX_TABLE_BYTES, SfntError, SfntFace, Tag, VerticalGlyphMetrics};
    use super::FontMetrics;

    fn face_metrics(ascender: i16, descender: i16, line_gap: i16, cap_height: i16) -> FontMetrics {
        FontMetrics {
            units_per_em: 1000,
            ascender,
            descender,
            line_gap,
            cap_height,
            x_min: 0,
            y_min: 0,
            x_max: 0,
            y_max: 0,
            num_glyphs: 1,
            number_of_h_metrics: 1,
            index_to_loc_format: 0,
        }
    }

    /// Nimbus Sans sets its ascent equal to its cap height and reserves 0.27 em
    /// below the baseline that only descenders use, so the capitals of a line
    /// sit high in a box centred on `ascent + descent`. The baseline moves down
    /// inside the same box until the cap height is centred in it.
    #[test]
    fn a_font_whose_ascent_is_its_cap_height_gets_its_capitals_centred_in_the_line() {
        let metrics = face_metrics(729, -271, 200, 718);

        let (ascent, descent, gap) = metrics.scaled_line_metrics(1000.0);

        // The box is the same size; only the baseline within it moved.
        assert!((ascent - descent - (729.0 + 271.0)).abs() < 1e-3);
        assert_eq!(gap, 200.0);
        // Capitals now span the box's middle: as much above its centre as below.
        let centre_above_baseline = (ascent + descent) / 2.0;
        assert!((centre_above_baseline - 718.0 / 2.0).abs() < 1e-3);
    }

    #[test]
    fn a_font_that_is_already_balanced_keeps_its_metrics() {
        // Capitals are centred on (969 - 242) / 2 = 363.5 already.
        let metrics = face_metrics(969, -242, 0, 727);

        let (ascent, descent, _) = metrics.scaled_line_metrics(1000.0);

        assert!((ascent - 969.0).abs() < 1.0 && (descent + 242.0).abs() < 1.0);
    }

    #[test]
    fn a_font_without_a_cap_height_keeps_its_metrics() {
        let metrics = face_metrics(729, -271, 200, 0);

        assert_eq!(metrics.scaled_line_metrics(1000.0), (729.0, -271.0, 200.0));
    }

    #[test]
    fn the_shift_scales_with_the_font_size() {
        let metrics = face_metrics(729, -271, 200, 718);

        let (ascent, descent, _) = metrics.scaled_line_metrics(16.0);

        assert!((ascent - descent - 16.0).abs() < 1e-4);
        assert!(((ascent + descent) / 2.0 - 718.0 / 2000.0 * 16.0).abs() < 1e-3);
    }

    fn minimal_sfnt() -> Vec<u8> {
        let mut bytes = vec![0; 32];
        bytes[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        bytes[4..6].copy_from_slice(&1_u16.to_be_bytes());
        bytes[12..16].copy_from_slice(b"head");
        bytes[20..24].copy_from_slice(&28_u32.to_be_bytes());
        bytes[24..28].copy_from_slice(&4_u32.to_be_bytes());
        bytes[28..32].copy_from_slice(b"test");
        bytes
    }

    fn format4_cmap() -> Vec<u8> {
        let mut bytes = vec![0; 44];
        bytes[2..4].copy_from_slice(&1_u16.to_be_bytes());
        bytes[4..6].copy_from_slice(&3_u16.to_be_bytes());
        bytes[6..8].copy_from_slice(&1_u16.to_be_bytes());
        bytes[8..12].copy_from_slice(&12_u32.to_be_bytes());
        bytes[12..14].copy_from_slice(&4_u16.to_be_bytes());
        bytes[14..16].copy_from_slice(&32_u16.to_be_bytes());
        bytes[18..20].copy_from_slice(&4_u16.to_be_bytes());
        bytes[20..22].copy_from_slice(&4_u16.to_be_bytes());
        bytes[22..24].copy_from_slice(&1_u16.to_be_bytes());
        bytes[26..28].copy_from_slice(&0x0041_u16.to_be_bytes());
        bytes[28..30].copy_from_slice(&0xffff_u16.to_be_bytes());
        bytes[32..34].copy_from_slice(&0x0041_u16.to_be_bytes());
        bytes[34..36].copy_from_slice(&0xffff_u16.to_be_bytes());
        bytes[36..38].copy_from_slice(&0x0041_u16.wrapping_neg().wrapping_add(3).to_be_bytes());
        bytes[38..40].copy_from_slice(&1_u16.to_be_bytes());
        bytes[40..42].copy_from_slice(&0_u16.to_be_bytes());
        bytes[42..44].copy_from_slice(&0_u16.to_be_bytes());
        bytes
    }

    fn format12_cmap() -> Vec<u8> {
        let mut bytes = vec![0; 40];
        bytes[2..4].copy_from_slice(&1_u16.to_be_bytes());
        bytes[4..6].copy_from_slice(&3_u16.to_be_bytes());
        bytes[6..8].copy_from_slice(&10_u16.to_be_bytes());
        bytes[8..12].copy_from_slice(&12_u32.to_be_bytes());
        bytes[12..14].copy_from_slice(&12_u16.to_be_bytes());
        bytes[16..20].copy_from_slice(&28_u32.to_be_bytes());
        bytes[24..28].copy_from_slice(&1_u32.to_be_bytes());
        bytes[28..32].copy_from_slice(&0x4e00_u32.to_be_bytes());
        bytes[32..36].copy_from_slice(&0x4e00_u32.to_be_bytes());
        bytes[36..40].copy_from_slice(&10_u32.to_be_bytes());
        bytes
    }

    fn format4_cmap_with_glyph_array() -> Vec<u8> {
        let mut bytes = format4_cmap();
        bytes.resize(46, 0);
        bytes[14..16].copy_from_slice(&34_u16.to_be_bytes());
        bytes[36..38].copy_from_slice(&0_u16.to_be_bytes());
        bytes[40..42].copy_from_slice(&4_u16.to_be_bytes());
        bytes[44..46].copy_from_slice(&3_u16.to_be_bytes());
        bytes
    }

    fn format12_and_format14_cmap() -> Vec<u8> {
        let mut bytes = vec![0; 78];
        bytes[2..4].copy_from_slice(&2_u16.to_be_bytes());
        bytes[4..6].copy_from_slice(&3_u16.to_be_bytes());
        bytes[6..8].copy_from_slice(&10_u16.to_be_bytes());
        bytes[8..12].copy_from_slice(&20_u32.to_be_bytes());
        bytes[12..14].copy_from_slice(&0_u16.to_be_bytes());
        bytes[14..16].copy_from_slice(&5_u16.to_be_bytes());
        bytes[16..20].copy_from_slice(&48_u32.to_be_bytes());

        bytes[20..22].copy_from_slice(&12_u16.to_be_bytes());
        bytes[24..28].copy_from_slice(&28_u32.to_be_bytes());
        bytes[32..36].copy_from_slice(&1_u32.to_be_bytes());
        bytes[36..40].copy_from_slice(&0x4e00_u32.to_be_bytes());
        bytes[40..44].copy_from_slice(&0x4e00_u32.to_be_bytes());
        bytes[44..48].copy_from_slice(&10_u32.to_be_bytes());

        bytes[48..50].copy_from_slice(&14_u16.to_be_bytes());
        bytes[50..54].copy_from_slice(&30_u32.to_be_bytes());
        bytes[54..58].copy_from_slice(&1_u32.to_be_bytes());
        bytes[58..61].copy_from_slice(&[0, 0xfe, 0]);
        bytes[65..69].copy_from_slice(&21_u32.to_be_bytes());
        bytes[69..73].copy_from_slice(&1_u32.to_be_bytes());
        bytes[73..76].copy_from_slice(&[0, 0x4e, 0]);
        bytes[76..78].copy_from_slice(&77_u16.to_be_bytes());
        bytes
    }

    fn sfnt_with_cmap(cmap: &[u8]) -> Vec<u8> {
        let table_offset = 28_u32;
        let mut bytes = vec![0; table_offset as usize + cmap.len()];
        bytes[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        bytes[4..6].copy_from_slice(&1_u16.to_be_bytes());
        bytes[12..16].copy_from_slice(b"cmap");
        bytes[20..24].copy_from_slice(&table_offset.to_be_bytes());
        bytes[24..28].copy_from_slice(&(cmap.len() as u32).to_be_bytes());
        bytes[table_offset as usize..].copy_from_slice(cmap);
        bytes
    }

    fn sfnt_with_tables(tables: &[([u8; 4], &[u8])]) -> Vec<u8> {
        // Real SFNT directories are tag-sorted. The compact table
        // range cache intentionally uses binary search, so keep fixtures
        // valid for both the checked Aimer reader and the reference shaper.
        let mut sorted_tables = tables.to_vec();
        sorted_tables.sort_unstable_by_key(|(tag, _)| *tag);

        let directory_end = 12 + sorted_tables.len() * 16;
        let mut bytes = vec![0; directory_end];
        bytes[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        bytes[4..6].copy_from_slice(&(sorted_tables.len() as u16).to_be_bytes());
        for (index, (tag, data)) in sorted_tables.iter().enumerate() {
            let record_offset = 12 + index * 16;
            let padding = (4 - bytes.len() % 4) % 4;
            bytes.resize(bytes.len() + padding, 0);
            let table_offset = bytes.len();
            bytes[record_offset..record_offset + 4].copy_from_slice(tag);
            bytes[record_offset + 8..record_offset + 12]
                .copy_from_slice(&(table_offset as u32).to_be_bytes());
            bytes[record_offset + 12..record_offset + 16]
                .copy_from_slice(&(data.len() as u32).to_be_bytes());
            bytes.extend_from_slice(data);
        }
        bytes
    }

    fn metrics_tables() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut head = vec![0; 54];
        head[18..20].copy_from_slice(&1000_u16.to_be_bytes());
        head[36..38].copy_from_slice(&(-10_i16).to_be_bytes());
        head[38..40].copy_from_slice(&(-200_i16).to_be_bytes());
        head[40..42].copy_from_slice(&900_i16.to_be_bytes());
        head[42..44].copy_from_slice(&800_i16.to_be_bytes());
        head[50..52].copy_from_slice(&0_i16.to_be_bytes());

        let mut hhea = vec![0; 36];
        hhea[4..6].copy_from_slice(&800_i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200_i16).to_be_bytes());
        hhea[8..10].copy_from_slice(&200_i16.to_be_bytes());
        hhea[34..36].copy_from_slice(&2_u16.to_be_bytes());

        let mut maxp = vec![0; 6];
        maxp[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        maxp[4..6].copy_from_slice(&3_u16.to_be_bytes());

        let mut hmtx = vec![0; 10];
        hmtx[0..2].copy_from_slice(&1000_u16.to_be_bytes());
        hmtx[4..6].copy_from_slice(&700_u16.to_be_bytes());
        hmtx[6..8].copy_from_slice(&10_i16.to_be_bytes());
        hmtx[8..10].copy_from_slice(&20_i16.to_be_bytes());
        (head, hhea, maxp, hmtx)
    }

    fn simple_glyf_tables() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let (mut head, mut hhea, mut maxp, hmtx) = metrics_tables();
        head[50..52].copy_from_slice(&1_i16.to_be_bytes());
        hhea[34..36].copy_from_slice(&1_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&1_u16.to_be_bytes());

        let mut glyf = vec![0; 19];
        glyf[0..2].copy_from_slice(&1_i16.to_be_bytes());
        glyf[4..6].copy_from_slice(&100_i16.to_be_bytes());
        glyf[6..8].copy_from_slice(&100_i16.to_be_bytes());
        glyf[8..10].copy_from_slice(&100_i16.to_be_bytes());
        glyf[10..12].copy_from_slice(&2_u16.to_be_bytes());
        glyf[14..17].copy_from_slice(&[0x31, 0x33, 0x35]);
        glyf[17] = 100;
        glyf[18] = 100;

        let mut loca = vec![0; 8];
        loca[4..8].copy_from_slice(&19_u32.to_be_bytes());
        (head, hhea, maxp, hmtx, loca, glyf)
    }

    fn simple_colr_v0_font() -> Vec<u8> {
        let (head, hhea, maxp, hmtx, loca, glyf) = simple_glyf_tables();
        let mut colr = vec![0; 14 + 6 + 4];
        colr[2..4].copy_from_slice(&1_u16.to_be_bytes());
        colr[4..8].copy_from_slice(&14_u32.to_be_bytes());
        colr[8..12].copy_from_slice(&20_u32.to_be_bytes());
        colr[12..14].copy_from_slice(&1_u16.to_be_bytes());
        colr[14..16].copy_from_slice(&0_u16.to_be_bytes());
        colr[16..18].copy_from_slice(&0_u16.to_be_bytes());
        colr[18..20].copy_from_slice(&1_u16.to_be_bytes());
        colr[20..22].copy_from_slice(&0_u16.to_be_bytes());
        colr[22..24].copy_from_slice(&0_u16.to_be_bytes());

        let mut cpal = vec![0; 14 + 4];
        cpal[2..4].copy_from_slice(&1_u16.to_be_bytes());
        cpal[4..6].copy_from_slice(&1_u16.to_be_bytes());
        cpal[6..8].copy_from_slice(&1_u16.to_be_bytes());
        cpal[8..12].copy_from_slice(&14_u32.to_be_bytes());
        cpal[12..14].copy_from_slice(&0_u16.to_be_bytes());
        cpal[14..18].copy_from_slice(&[0, 0, 255, 255]);

        sfnt_with_tables(&[
            (*b"COLR", colr.as_slice()),
            (*b"CPAL", cpal.as_slice()),
            (*b"glyf", glyf.as_slice()),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", loca.as_slice()),
            (*b"maxp", maxp.as_slice()),
        ])
    }

    fn bitmap_fingerprint(bitmap: &[u8]) -> u64 {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        bitmap.iter().fold(OFFSET, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
        })
    }

    fn composite_glyf_tables() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let (mut head, mut hhea, mut maxp, _, _, simple) = simple_glyf_tables();
        head[50..52].copy_from_slice(&1_i16.to_be_bytes());
        hhea[34..36].copy_from_slice(&1_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&2_u16.to_be_bytes());

        let mut composite = vec![0; 18];
        composite[0..2].copy_from_slice(&(-1_i16).to_be_bytes());
        composite[2..4].copy_from_slice(&10_i16.to_be_bytes());
        composite[4..6].copy_from_slice(&20_i16.to_be_bytes());
        composite[6..8].copy_from_slice(&110_i16.to_be_bytes());
        composite[8..10].copy_from_slice(&120_i16.to_be_bytes());
        composite[10..12].copy_from_slice(&3_u16.to_be_bytes());
        composite[14..16].copy_from_slice(&10_i16.to_be_bytes());
        composite[16..18].copy_from_slice(&20_i16.to_be_bytes());

        let mut hmtx = vec![0; 6];
        hmtx[0..2].copy_from_slice(&1000_u16.to_be_bytes());
        let mut loca = vec![0; 12];
        loca[4..8].copy_from_slice(&19_u32.to_be_bytes());
        loca[8..12].copy_from_slice(&37_u32.to_be_bytes());
        let mut glyf = simple;
        glyf.extend_from_slice(&composite);
        (head, hhea, maxp, hmtx, loca, glyf)
    }

    fn cff_index(items: &[&[u8]], cff2: bool) -> Vec<u8> {
        let count = items.len();
        let mut bytes = Vec::new();
        if cff2 {
            bytes.extend_from_slice(&(count as u32).to_be_bytes());
        } else {
            bytes.extend_from_slice(&(count as u16).to_be_bytes());
        }
        if count == 0 {
            return bytes;
        }
        bytes.push(1);
        let mut offset = 1_u8;
        bytes.push(offset);
        for item in items {
            offset = offset
                .checked_add(item.len() as u8)
                .expect("test INDEX fits");
            bytes.push(offset);
        }
        for item in items {
            bytes.extend_from_slice(item);
        }
        bytes
    }

    fn cff_integer(value: usize) -> u8 {
        assert!((0..=107).contains(&value));
        (139 + value) as u8
    }

    fn cff_number(value: i32) -> u8 {
        assert!((-107..=107).contains(&value));
        (value + 139) as u8
    }

    fn cff1_table(charstring: &[u8]) -> Vec<u8> {
        let name = cff_index(&[b"Test"], false);
        let strings = cff_index(&[], false);
        let global_subrs = cff_index(&[], false);
        let charstrings_offset = 4 + name.len() + 7 + strings.len() + global_subrs.len();
        let top_dict = [cff_integer(charstrings_offset), 17];
        let top = cff_index(&[&top_dict], false);
        assert_eq!(top.len(), 7);

        let mut bytes = vec![1, 0, 4, 4];
        bytes.extend_from_slice(&name);
        bytes.extend_from_slice(&top);
        bytes.extend_from_slice(&strings);
        bytes.extend_from_slice(&global_subrs);
        bytes.extend_from_slice(&cff_index(&[charstring], false));
        bytes
    }

    fn cff1_subroutine_cycle_table() -> Vec<u8> {
        let name = cff_index(&[b"Test"], false);
        let strings = cff_index(&[], false);
        let global_subrs = cff_index(&[], false);
        let charstring = [cff_number(-107), 10, 14];
        let local_subroutine = [cff_number(-107), 10, 11];
        let charstrings_offset = 27;
        let private_offset = charstrings_offset + cff_index(&[&charstring], false).len();
        let private = [cff_integer(2), 19];
        let top_dict = [
            cff_integer(charstrings_offset),
            17,
            cff_integer(private.len()),
            cff_integer(private_offset),
            18,
        ];
        let top = cff_index(&[&top_dict], false);
        assert_eq!(top.len(), 10);

        let mut bytes = vec![1, 0, 4, 4];
        bytes.extend_from_slice(&name);
        bytes.extend_from_slice(&top);
        bytes.extend_from_slice(&strings);
        bytes.extend_from_slice(&global_subrs);
        bytes.extend_from_slice(&cff_index(&[&charstring], false));
        bytes.extend_from_slice(&private);
        bytes.extend_from_slice(&cff_index(&[&local_subroutine], false));
        bytes
    }

    fn cff2_table(charstring: &[u8]) -> Vec<u8> {
        let fd_array_offset = 14;
        let fd_array = cff_index(&[&[]], true);
        let charstrings_offset = fd_array_offset + fd_array.len();
        let top_dict = [
            cff_integer(charstrings_offset),
            17,
            cff_integer(fd_array_offset),
            12,
            36,
        ];
        let mut bytes = vec![2, 0, 5, 0, top_dict.len() as u8];
        bytes.extend_from_slice(&top_dict);
        bytes.extend_from_slice(&cff_index(&[], true));
        bytes.extend_from_slice(&fd_array);
        bytes.extend_from_slice(&cff_index(&[charstring], true));
        bytes
    }

    fn cff_metrics_tables() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let (head, mut hhea, mut maxp, _) = metrics_tables();
        hhea[34..36].copy_from_slice(&1_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&1_u16.to_be_bytes());
        (head, hhea, maxp)
    }

    fn unicode_name_table() -> Vec<u8> {
        let mut bytes = vec![0; 36];
        bytes[2..4].copy_from_slice(&1_u16.to_be_bytes());
        bytes[4..6].copy_from_slice(&18_u16.to_be_bytes());
        bytes[6..8].copy_from_slice(&3_u16.to_be_bytes());
        bytes[8..10].copy_from_slice(&1_u16.to_be_bytes());
        bytes[10..12].copy_from_slice(&0x0409_u16.to_be_bytes());
        bytes[12..14].copy_from_slice(&1_u16.to_be_bytes());
        bytes[14..16].copy_from_slice(&18_u16.to_be_bytes());
        bytes[18..36].copy_from_slice(&[
            0, b'T', 0, b'e', 0, b's', 0, b't', 0, b' ', 0, b'S', 0, b'a', 0, b'n', 0, b's',
        ]);
        bytes
    }

    #[test]
    fn reads_a_bounded_sfnt_table() {
        let bytes = minimal_sfnt();
        let face = SfntFace::from_bytes(&bytes, 0).expect("minimal SFNT should parse");

        assert_eq!(face.table(*b"head"), Some(&b"test"[..]));
    }

    #[test]
    fn maps_unicode_through_a_format4_cmap() {
        let bytes = sfnt_with_cmap(&format4_cmap());
        let face = SfntFace::from_bytes(&bytes, 0).expect("format 4 cmap should parse");

        assert_eq!(face.glyph_index('A' as u32), Ok(Some(3)));
        assert_eq!(face.glyph_index('B' as u32), Ok(None));
    }

    #[test]
    fn maps_a_format4_glyph_array_entry() {
        let bytes = sfnt_with_cmap(&format4_cmap_with_glyph_array());
        let face = SfntFace::from_bytes(&bytes, 0).expect("format 4 glyph array should parse");

        assert_eq!(face.glyph_index('A' as u32), Ok(Some(3)));
    }

    #[test]
    fn retains_parsed_cmap_metrics_and_hmtx_after_first_lookup() {
        let (head, hhea, maxp, hmtx) = metrics_tables();
        let cmap = format4_cmap();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"cmap", cmap.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("cache fixture should parse");

        assert!(face.cmap_cache.get().is_none());
        assert!(face.metrics_cache.get().is_none());
        assert!(face.hmtx_cache.get().is_none());

        assert_eq!(face.glyph_index('A' as u32), Ok(Some(3)));
        assert_eq!(face.glyph_advance(1), Ok(Some(700)));

        assert!(face.cmap_cache.get().is_some());
        assert!(face.metrics_cache.get().is_some());
        assert!(face.hmtx_cache.get().is_some());
    }

    #[test]
    fn compiles_layout_fast_paths_for_a_cached_face() {
        use std::sync::Arc;

        use super::super::font_resolver::FontData;
        use super::AimerFontState;

        let data = FontData::Shared(Arc::from(
            &include_bytes!("../fonts/JetBrainsMono-Regular.ttf")[..],
        ));
        let state = AimerFontState::from_font_data(data, 0)
            .expect("checked-in font should build a cached state");

        state
            .shape_run("AV")
            .expect("the cached layout should shape a Latin run");
        let layout = state
            .shared
            .layout
            .get()
            .expect("the first shape should initialize layout state")
            .as_ref()
            .expect("the checked-in layout tables should parse");
        assert!(layout.compiled_fast_path_count() > 0);
        assert!(layout.active_execution_plan_count() > 0);
    }

    #[test]
    fn retains_decoded_outlines_and_flattened_edges_for_repeated_glyphs() {
        use std::sync::Arc;

        use super::super::font_resolver::FontData;
        use super::AimerFontState;

        let data = FontData::Shared(Arc::from(
            &include_bytes!("../fonts/JetBrainsMono-Regular.ttf")[..],
        ));
        let mut state = AimerFontState::from_font_data(data, 0)
            .expect("checked-in font should build a cached state");
        let glyph_id = state
            .shared
            .face
            .glyph_index('A' as u32)
            .expect("A lookup should parse")
            .expect("A should be covered");

        let first = state
            .rasterize_glyphs(&[(glyph_id, 0, 0)], 16.0)
            .into_iter()
            .next()
            .flatten()
            .expect("A should rasterize");
        let second = state
            .rasterize_glyphs(&[(glyph_id, 0, 0)], 16.0)
            .into_iter()
            .next()
            .flatten()
            .expect("A should rasterize again");

        assert_eq!(first.bitmap, second.bitmap);
        assert_eq!((first.width, first.height), (second.width, second.height));
        assert_eq!(
            (first.offset_x, first.offset_y),
            (second.offset_x, second.offset_y)
        );
        assert_eq!(state.outline_cache_len(), 1);
        assert_eq!(state.flattened_edge_cache_len(), 1);
    }

    #[test]
    fn batch_rasterization_matches_scalar_glyph_results() {
        use std::sync::Arc;

        use super::super::font_resolver::FontData;
        use super::super::glyph_rasterizer::GlyphKey;
        use super::AimerFontState;

        let data = FontData::Shared(Arc::from(
            &include_bytes!("../fonts/JetBrainsMono-Regular.ttf")[..],
        ));
        let mut scalar = AimerFontState::from_font_data(data.clone(), 0)
            .expect("checked-in font should build a cached state");
        let mut batch = AimerFontState::from_font_data(data, 0)
            .expect("checked-in font should build a cached state");
        let glyphs = ['A', 'V', 'g']
            .into_iter()
            .map(|codepoint| {
                let glyph_id = scalar
                    .shared
                    .face
                    .glyph_index(codepoint as u32)
                    .expect("the glyph lookup should parse")
                    .expect("the test font should cover the glyph");
                GlyphKey::new(7, glyph_id, 16.0)
            })
            .collect::<Vec<_>>();
        let expected = glyphs
            .iter()
            .map(|key| {
                scalar
                    .rasterize_glyphs(&[(key.glyph_id, 0, 0)], 16.0)
                    .into_iter()
                    .next()
                    .flatten()
                    .expect("the scalar glyph should rasterize")
            })
            .collect::<Vec<_>>();
        let mut actual = Vec::new();
        let complete = super::rasterize::rasterize_face_glyphs_into(
            &batch.shared.face,
            &batch.shared.metrics,
            &glyphs,
            16.0,
            &mut batch.raster_cache,
            |_, glyph| actual.push(glyph),
        );

        assert!(complete);
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected.iter()) {
            assert_eq!(actual.bitmap, expected.bitmap);
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height)
            );
            assert_eq!(
                (actual.offset_x, actual.offset_y),
                (expected.offset_x, expected.offset_y)
            );
            assert_eq!(actual.advance_width, expected.advance_width);
        }
    }

    #[test]
    fn arbitrary_variation_instances_are_stable_and_distinct() {
        use std::sync::Arc;

        use super::super::font_resolver::FontData;
        use super::super::glyph_rasterizer::GlyphKey;
        use super::AimerFontState;

        let data = FontData::Shared(Arc::from(
            &include_bytes!("../fonts/NotoSansJP-VariableFont_wght.ttf")[..],
        ));
        let mut state = AimerFontState::from_font_data(data, 0)
            .expect("the bundled variable CJK face should build a cached state");
        let wght = u32::from_be_bytes(*b"wght");

        let regular = state
            .variation_instance_for_axes(400, &[(wght, 400.0)])
            .expect("the wght axis should be accepted as an arbitrary request");
        let bold = state
            .variation_instance_for_axes(400, &[(wght, 900.0)])
            .expect("the upper wght instance should be accepted");
        let bold_again = state
            .variation_instance_for_axes(400, &[(wght, 900.0)])
            .expect("repeating an axis request should remain valid");

        assert_ne!(regular, bold);
        assert_eq!(bold, bold_again);
        assert_ne!(
            GlyphKey::new(7, 1, 16.0).with_variation_id(regular),
            GlyphKey::new(7, 1, 16.0).with_variation_id(bold),
        );

        let glyph_id = state
            .shared
            .face
            .glyph_index(0x4e00)
            .expect("the CJK cmap should parse")
            .expect("the bundled face should cover the ideograph");
        let keys = [
            GlyphKey::new(7, glyph_id, 16.0)
                .weighted(400)
                .with_variation_id(regular),
            GlyphKey::new(7, glyph_id, 16.0)
                .weighted(400)
                .with_variation_id(bold),
        ];
        let mut rendered = Vec::new();
        let complete = state.rasterize_glyphs_into(&keys, 16.0, |key, glyph| {
            rendered.push((key.variation_id, glyph.bitmap.clone()));
        });
        assert!(complete);
        assert_eq!(rendered.len(), 2);
        assert_eq!(state.outline_cache_len(), 2);
        assert_ne!(rendered[0].1, rendered[1].1);
    }

    #[test]
    fn maps_a_non_bmp_unicode_value_through_a_format12_cmap() {
        let bytes = sfnt_with_cmap(&format12_cmap());
        let face = SfntFace::from_bytes(&bytes, 0).expect("format 12 cmap should parse");

        assert_eq!(face.glyph_index(0x4e00), Ok(Some(10)));
        assert_eq!(face.glyph_index(0x4e01), Ok(None));
    }

    #[test]
    fn maps_a_unicode_variation_sequence_through_a_format14_cmap() {
        let bytes = sfnt_with_cmap(&format12_and_format14_cmap());
        let face = SfntFace::from_bytes(&bytes, 0).expect("format 14 cmap should parse");

        assert_eq!(
            face.glyph_index_with_variation(0x4e00, 0xfe00),
            Ok(Some(77))
        );
        assert_eq!(
            face.glyph_index_with_variation(0x4e00, 0xfe01),
            Ok(Some(10))
        );
        assert!(face.cmap_cache.get().is_some());
    }

    #[test]
    fn keeps_base_cmap_usable_when_an_optional_format14_table_is_malformed() {
        let mut cmap = format12_and_format14_cmap();
        cmap[58..61].copy_from_slice(&[0, 0, 1]);
        let bytes = sfnt_with_cmap(&cmap);
        let face = SfntFace::from_bytes(&bytes, 0).expect("base cmap should still parse");

        assert_eq!(face.glyph_index(0x4e00), Ok(Some(10)));
        assert!(matches!(
            face.glyph_index_with_variation(0x4e00, 0xfe00),
            Err(SfntError::MalformedCmap(14))
        ));
    }

    #[test]
    fn shapes_a_cjk_unicode_variation_sequence_through_the_owned_path() {
        use std::sync::Arc;

        use super::super::font_resolver::FontData;
        use super::AimerFontState;

        let (head, hhea, mut maxp, hmtx) = metrics_tables();
        maxp[4..6].copy_from_slice(&3_u16.to_be_bytes());
        let mut cmap = format12_and_format14_cmap();
        cmap[44..48].copy_from_slice(&1_u32.to_be_bytes());
        cmap[76..78].copy_from_slice(&2_u16.to_be_bytes());
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"cmap", cmap.as_slice()),
        ]);
        let data = FontData::Shared(Arc::from(bytes));
        let state = AimerFontState::from_font_data(data, 0)
            .expect("the CJK variation fixture should build a cached state");
        assert!(AimerFontState::can_shape_text("\u{4e00}\u{fe00}"));
        assert!(AimerFontState::can_shape_text("\u{4e00}"));

        let shaped = state
            .shape_run("\u{4e00}\u{fe00}")
            .expect("the owned CJK variation path should not fail")
            .expect("the owned path should claim a supported CJK variation sequence");

        assert_eq!(shaped.glyphs.len(), 1);
        assert_eq!(shaped.glyphs[0].glyph_id, 2);
        assert_eq!(shaped.glyphs[0].cluster, 0);
        assert_eq!(shaped.glyphs[0].x_advance, 700);
    }

    #[test]
    fn applies_cjk_language_forms_from_the_requested_language_system() {
        use crate::font::TextLanguage;

        let bytes = cjk_layout_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("CJK layout fixture must parse");

        let chinese =
            super::shape_run_with_options(&face, "\u{4e00}", Some(TextLanguage::Chinese), false)
                .expect("Chinese locl shaping should parse")
                .expect("Chinese locl should claim the CJK run");
        let japanese =
            super::shape_run_with_options(&face, "\u{4e00}", Some(TextLanguage::Japanese), false)
                .expect("Japanese locl shaping should parse")
                .expect("Japanese locl should claim the CJK run");

        assert_eq!(chinese.glyphs[0].glyph_id, 2);
        assert_eq!(japanese.glyphs[0].glyph_id, 3);
        assert_eq!(chinese.glyphs[0].cluster, japanese.glyphs[0].cluster);
        assert_eq!(chinese.glyphs[0].x_advance, japanese.glyphs[0].x_advance);
    }

    #[test]
    fn prefers_cjk_vrt2_over_legacy_vert_in_vertical_mode() {
        let bytes = vertical_cjk_layout_font_for_test(true);
        let face = SfntFace::from_bytes(&bytes, 0).expect("CJK layout fixture must parse");

        let shaped = super::shape_run_with_options(&face, "\u{4e00}", None, true)
            .expect("vertical CJK shaping should parse")
            .expect("vertical CJK substitutions should claim the run");

        assert_eq!(shaped.glyphs.len(), 1);
        assert_eq!(shaped.glyphs[0].glyph_id, 4);
        assert_eq!(shaped.glyphs[0].cluster, 0);
    }

    #[test]
    fn applies_vpal_after_vertical_alternates_before_reading_vertical_metrics() {
        let bytes = vertical_cjk_layout_font_with_vpal();
        let face = SfntFace::from_bytes(&bytes, 0).expect("CJK vpal fixture must parse");

        let shaped = super::shape_run_with_options(&face, "\u{4e00}", None, true)
            .expect("vertical vpal shaping should parse")
            .expect("vertical vpal shaping should claim the run");

        assert_eq!(shaped.glyphs.len(), 1);
        assert_eq!(shaped.glyphs[0].glyph_id, 5);
        assert_eq!(shaped.glyphs[0].x_advance, 0);
        assert_eq!(shaped.glyphs[0].y_advance, -800);
        assert_eq!(shaped.glyphs[0].y_offset, -850);
    }

    #[test]
    fn reads_vertical_metrics_and_vorg_origin_for_each_glyph() {
        let bytes = vertical_cjk_layout_font_for_test(true);
        let face = SfntFace::from_bytes(&bytes, 0).expect("vertical CJK fixture must parse");

        assert!(face.vertical_metrics_cache.get().is_none());
        assert_eq!(
            face.vertical_glyph_metrics(1)
                .expect("glyph 1 vertical metrics should parse"),
            Some(VerticalGlyphMetrics {
                advance_height: 900,
                top_side_bearing: 100,
                vert_origin_y: 850,
            })
        );
        assert_eq!(
            face.vertical_glyph_metrics(4)
                .expect("glyph 4 vertical metrics should parse"),
            Some(VerticalGlyphMetrics {
                advance_height: 800,
                top_side_bearing: 60,
                vert_origin_y: 880,
            })
        );
        assert_eq!(
            face.vertical_glyph_metrics(5)
                .expect("glyph 5 vertical metrics should parse"),
            Some(VerticalGlyphMetrics {
                advance_height: 800,
                top_side_bearing: 50,
                vert_origin_y: 850,
            })
        );
        assert!(face.vertical_metrics_cache.get().is_some());
    }

    #[test]
    fn rejects_truncated_vertical_metrics_without_claiming_the_run() {
        let bytes = vertical_cjk_layout_font_for_test(false);
        let face = SfntFace::from_bytes(&bytes, 0).expect("vertical CJK fixture must parse");

        assert!(matches!(
            face.vertical_metrics(),
            Err(SfntError::MalformedTable(tag)) if tag == Tag::from_bytes(*b"vmtx")
        ));
        assert!(matches!(
            super::shape_run_with_options(&face, "\u{4e00}", None, true),
            Err(SfntError::MalformedTable(tag)) if tag == Tag::from_bytes(*b"vmtx")
        ));
    }

    #[test]
    fn owned_vertical_substitution_uses_default_advance_without_vertical_metrics() {
        let bytes = cjk_layout_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("CJK layout fixture must parse");

        let shaped = super::shape_run_with_options(&face, "\u{4e00}", None, true)
            .expect("missing vertical tables are not malformed")
            .expect("owned vertical substitution should claim the run");
        assert_eq!(shaped.glyphs.len(), 1);
        assert_eq!(shaped.glyphs[0].glyph_id, 1);
        assert_eq!(shaped.glyphs[0].x_advance, 0);
        assert_eq!(shaped.glyphs[0].y_advance, -1000);
    }

    #[test]
    fn applies_vertical_advance_and_origin_after_cjk_substitution() {
        let bytes = vertical_cjk_layout_font_for_test(true);
        let face = SfntFace::from_bytes(&bytes, 0).expect("vertical CJK fixture must parse");

        let shaped = super::shape_run_with_options(&face, "\u{4e00}\u{4e00}", None, true)
            .expect("vertical CJK shaping should parse")
            .expect("vertical CJK metrics should claim the run");

        assert_eq!(shaped.glyphs.len(), 2);
        assert!(shaped.glyphs.iter().all(|glyph| glyph.glyph_id == 4
            && glyph.x_advance == 0
            && glyph.y_advance == -800
            && glyph.y_offset == -880));
        assert_eq!(shaped.glyphs[0].cluster, 0);
        assert_eq!(shaped.glyphs[1].cluster, 3);

        let mut pen_y = 0;
        let pen_positions = shaped
            .glyphs
            .iter()
            .map(|glyph| {
                let position = pen_y;
                pen_y += glyph.y_advance;
                position
            })
            .collect::<Vec<_>>();
        assert_eq!(pen_positions, vec![0, -800]);
    }

    #[test]
    fn applies_vertical_pair_positioning_after_metrics_and_substitution() {
        let bytes = vertical_cjk_layout_font_with_gpos();
        let face = SfntFace::from_bytes(&bytes, 0).expect("vertical GPOS fixture must parse");

        let shaped = super::shape_run_with_options(&face, "\u{4e00}\u{4e00}", None, true)
            .expect("vertical CJK shaping should parse")
            .expect("vertical CJK GPOS should claim the run");

        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.y_advance)
                .collect::<Vec<_>>(),
            vec![-820, -800]
        );
    }

    #[test]
    fn reads_face_metrics_and_reuses_the_last_horizontal_advance() {
        let (head, hhea, maxp, hmtx) = metrics_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("metrics font should parse");
        let metrics = face.metrics().expect("metrics should parse");

        assert_eq!(metrics.units_per_em, 1000);
        assert_eq!(metrics.ascender, 800);
        assert_eq!(metrics.descender, -200);
        assert_eq!(metrics.line_gap, 200);
        assert_eq!(face.glyph_advance(0), Ok(Some(1000)));
        assert_eq!(face.glyph_advance(1), Ok(Some(700)));
        assert_eq!(face.glyph_advance(2), Ok(Some(700)));
        assert_eq!(face.glyph_advance(3), Ok(None));
    }

    #[test]
    fn applies_hvar_advance_delta_at_weight_instance() {
        let (head, hhea, maxp, hmtx) = metrics_tables();
        let mut fvar = vec![0; 36];
        fvar[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        fvar[4..6].copy_from_slice(&16_u16.to_be_bytes());
        fvar[8..10].copy_from_slice(&1_u16.to_be_bytes());
        fvar[10..12].copy_from_slice(&20_u16.to_be_bytes());
        fvar[16..20].copy_from_slice(b"wght");
        fvar[20..24].copy_from_slice(&(100_i32 << 16).to_be_bytes());
        fvar[24..28].copy_from_slice(&(400_i32 << 16).to_be_bytes());
        fvar[28..32].copy_from_slice(&(900_i32 << 16).to_be_bytes());

        // One variation region activates from the default weight to 900.
        // The direct item index for glyph 1 carries a +100 advance delta.
        let mut hvar = vec![0; 63];
        hvar[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        hvar[4..8].copy_from_slice(&20_u32.to_be_bytes());
        hvar[8..12].copy_from_slice(&56_u32.to_be_bytes());
        hvar[20..22].copy_from_slice(&1_u16.to_be_bytes());
        hvar[22..26].copy_from_slice(&12_u32.to_be_bytes());
        hvar[26..28].copy_from_slice(&1_u16.to_be_bytes());
        hvar[28..32].copy_from_slice(&22_u32.to_be_bytes());
        hvar[32..34].copy_from_slice(&1_u16.to_be_bytes());
        hvar[34..36].copy_from_slice(&1_u16.to_be_bytes());
        hvar[42..44].copy_from_slice(&3_u16.to_be_bytes());
        hvar[44..46].copy_from_slice(&1_u16.to_be_bytes());
        hvar[46..48].copy_from_slice(&1_u16.to_be_bytes());
        hvar[48..50].copy_from_slice(&0_u16.to_be_bytes());
        hvar[50..52].copy_from_slice(&0_i16.to_be_bytes());
        hvar[52..54].copy_from_slice(&100_i16.to_be_bytes());
        hvar[54..56].copy_from_slice(&0_i16.to_be_bytes());
        hvar[36..38].copy_from_slice(&0_u16.to_be_bytes());
        hvar[38..40].copy_from_slice(&0x4000_u16.to_be_bytes());
        hvar[40..42].copy_from_slice(&0x4000_u16.to_be_bytes());
        hvar[56..57].copy_from_slice(&[0]);
        hvar[57..58].copy_from_slice(&[0]);
        hvar[58..60].copy_from_slice(&3_u16.to_be_bytes());
        hvar[60..63].copy_from_slice(&[0, 1, 2]);

        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"fvar", fvar.as_slice()),
            (*b"HVAR", hvar.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("HVAR fixture should parse");
        let metrics = face.metrics().expect("metrics should parse");

        assert_eq!(
            face.glyph_advance_with_metrics_at_weight(1, metrics, 400)
                .expect("default metric should parse"),
            Some(700)
        );
        assert_eq!(
            face.glyph_advance_with_metrics_at_weight(1, metrics, 900)
                .expect("bold metric should parse"),
            Some(800)
        );
    }

    #[test]
    fn applies_vvar_metric_deltas_at_weight_instance() {
        let (head, hhea, maxp, hmtx) = metrics_tables();
        let mut fvar = vec![0; 36];
        fvar[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        fvar[4..6].copy_from_slice(&16_u16.to_be_bytes());
        fvar[8..10].copy_from_slice(&1_u16.to_be_bytes());
        fvar[10..12].copy_from_slice(&20_u16.to_be_bytes());
        fvar[16..20].copy_from_slice(b"wght");
        fvar[20..24].copy_from_slice(&(100_i32 << 16).to_be_bytes());
        fvar[24..28].copy_from_slice(&(400_i32 << 16).to_be_bytes());
        fvar[28..32].copy_from_slice(&(900_i32 << 16).to_be_bytes());

        // The three VVAR maps use the direct item index for glyph 1. Its
        // advance-height, top-side-bearing, and vertical-origin rows each
        // receive +100 at the upper end of the weight axis.
        let mut vvar = vec![0; 60];
        vvar[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        vvar[4..8].copy_from_slice(&24_u32.to_be_bytes());
        vvar[24..26].copy_from_slice(&1_u16.to_be_bytes());
        vvar[26..30].copy_from_slice(&12_u32.to_be_bytes());
        vvar[30..32].copy_from_slice(&1_u16.to_be_bytes());
        vvar[32..36].copy_from_slice(&22_u32.to_be_bytes());
        vvar[36..38].copy_from_slice(&1_u16.to_be_bytes());
        vvar[38..40].copy_from_slice(&1_u16.to_be_bytes());
        vvar[40..42].copy_from_slice(&0_u16.to_be_bytes());
        vvar[42..44].copy_from_slice(&0x4000_u16.to_be_bytes());
        vvar[44..46].copy_from_slice(&0x4000_u16.to_be_bytes());
        vvar[46..48].copy_from_slice(&3_u16.to_be_bytes());
        vvar[48..50].copy_from_slice(&1_u16.to_be_bytes());
        vvar[50..52].copy_from_slice(&1_u16.to_be_bytes());
        vvar[52..54].copy_from_slice(&0_u16.to_be_bytes());
        vvar[54..56].copy_from_slice(&0_i16.to_be_bytes());
        vvar[56..58].copy_from_slice(&100_i16.to_be_bytes());
        vvar[58..60].copy_from_slice(&0_i16.to_be_bytes());

        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"fvar", fvar.as_slice()),
            (*b"VVAR", vvar.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("VVAR fixture should parse");

        assert_eq!(
            face.vertical_metric_deltas(1, 400)
                .expect("default metric should parse"),
            [0, 0, 0]
        );
        assert_eq!(
            face.vertical_metric_deltas(1, 900)
                .expect("bold metric should parse"),
            [100, 100, 100]
        );
    }

    #[test]
    fn extracts_a_simple_glyf_outline_through_loca() {
        let (head, hhea, maxp, hmtx, loca, glyf) = simple_glyf_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", loca.as_slice()),
            (*b"glyf", glyf.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("outline font should parse");
        let outline = face
            .outline(0)
            .expect("outline should parse")
            .expect("glyph should have an outline");

        assert_eq!(outline.contours.len(), 1);
        let contour = outline.contours[0];
        assert_eq!(contour.end - contour.start, 3);
        assert_eq!(
            (
                outline.points[contour.start].x,
                outline.points[contour.start].y
            ),
            (0.0, 0.0)
        );
        assert_eq!(
            (
                outline.points[contour.start + 1].x,
                outline.points[contour.start + 1].y
            ),
            (100.0, 0.0)
        );
        assert_eq!(
            (
                outline.points[contour.start + 2].x,
                outline.points[contour.start + 2].y
            ),
            (100.0, 100.0)
        );
    }

    #[test]
    fn rasterizes_a_simple_true_type_outline_with_scaled_metrics() {
        let (head, hhea, maxp, hmtx, loca, glyf) = simple_glyf_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", loca.as_slice()),
            (*b"glyf", glyf.as_slice()),
        ]);

        let glyph = super::rasterize::rasterize_font_glyph(&bytes, 0, 0, 10.0, 0, 0)
            .expect("the simple TrueType glyph should rasterize");

        assert_eq!((glyph.width, glyph.height), (1, 1));
        assert_eq!(glyph.bitmap.len(), 1);
        assert_eq!(glyph.bitmap, vec![159]);
        assert_eq!(glyph.advance_width, 10.0);
        assert!(!glyph.is_color);
    }

    #[test]
    fn rasterizes_fractional_font_sizes_with_fractional_metrics() {
        let (head, hhea, maxp, hmtx, loca, glyf) = simple_glyf_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", loca.as_slice()),
            (*b"glyf", glyf.as_slice()),
        ]);

        let integer = super::rasterize::rasterize_font_glyph(&bytes, 0, 0, 10.0, 0, 0)
            .expect("the integer-size glyph should rasterize");
        let fractional = super::rasterize::rasterize_font_glyph(&bytes, 0, 0, 10.5, 0, 0)
            .expect("the fractional-size glyph should rasterize");

        assert_eq!(integer.advance_width, 10.0);
        assert_eq!(fractional.advance_width, 10.5);
        assert_eq!((fractional.width, fractional.height), (2, 2));
        assert_eq!(fractional.bitmap, vec![0, 0, 159, 0]);
    }

    #[test]
    fn rasterizes_fractional_pen_positions_with_stable_bearings() {
        let (head, hhea, maxp, hmtx, loca, glyf) = simple_glyf_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", loca.as_slice()),
            (*b"glyf", glyf.as_slice()),
        ]);

        let integer = super::rasterize::rasterize_font_glyph(&bytes, 0, 0, 10.0, 0, 0)
            .expect("the integer-position glyph should rasterize");
        let fractional = super::rasterize::rasterize_font_glyph(&bytes, 0, 0, 10.0, 4, 0)
            .expect("the fractional-position glyph should rasterize");
        let fractional_y = super::rasterize::rasterize_font_glyph(&bytes, 0, 0, 10.0, 0, 4)
            .expect("the fractional y-position glyph should rasterize");

        assert_eq!(fractional.advance_width, integer.advance_width);
        assert_eq!(fractional.offset_x, -0.5);
        assert_eq!(fractional.offset_y, integer.offset_y);
        assert_eq!(fractional.bitmap, vec![48, 112]);
        assert_eq!(fractional_y.offset_x, integer.offset_x);
        assert_eq!(fractional_y.offset_y, -0.5);
        assert_ne!(fractional_y.bitmap, integer.bitmap);
    }

    #[test]
    fn rasterizes_a_cff_cubic_outline_into_monochrome_coverage() {
        let charstring = [
            cff_integer(0),
            cff_integer(0),
            21,
            cff_integer(0),
            cff_integer(50),
            cff_integer(50),
            cff_integer(50),
            cff_integer(50),
            cff_integer(0),
            8,
            14,
        ];
        let cff = cff1_table(&charstring);
        let (head, hhea, maxp) = cff_metrics_tables();
        let hmtx = [0x03, 0xe8, 0, 0];
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", &hmtx),
            (*b"CFF ", cff.as_slice()),
        ]);

        let glyph = super::rasterize::rasterize_font_glyph(&bytes, 0, 0, 10.0, 0, 0)
            .expect("the CFF cubic glyph should rasterize");

        assert!(glyph.width > 0 && glyph.height > 0);
        assert!(glyph.bitmap.iter().any(|coverage| *coverage > 0));
        assert_eq!(glyph.advance_width, 10.0);
        assert!(!glyph.is_color);
    }

    #[test]
    fn preserves_the_advance_of_an_empty_glyph_without_allocating_coverage() {
        let (mut head, mut hhea, mut maxp, hmtx) = metrics_tables();
        head[50..52].copy_from_slice(&1_i16.to_be_bytes());
        hhea[34..36].copy_from_slice(&1_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&1_u16.to_be_bytes());
        let loca = [0_u8; 8];
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", &loca),
            (*b"glyf", &[]),
        ]);

        let glyph = super::rasterize::rasterize_font_glyph(&bytes, 0, 0, 10.0, 0, 0)
            .expect("an empty TrueType glyph should still have metrics");

        assert!(glyph.bitmap.is_empty());
        assert_eq!((glyph.width, glyph.height), (0, 0));
        assert_eq!(glyph.advance_width, 10.0);
    }

    #[test]
    fn rejects_non_positive_or_non_finite_raster_sizes() {
        let (head, hhea, maxp, hmtx, loca, glyf) = simple_glyf_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", loca.as_slice()),
            (*b"glyf", glyf.as_slice()),
        ]);

        for size in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(
                super::rasterize::rasterize_font_glyph(&bytes, 0, 0, size, 0, 0).is_none(),
                "invalid size {size:?} must not allocate a bitmap"
            );
        }
    }

    #[test]
    fn extracts_a_simple_glyf_outline_from_short_loca_offsets() {
        let (mut head, hhea, maxp, hmtx, _, mut glyf) = simple_glyf_tables();
        head[50..52].copy_from_slice(&0_i16.to_be_bytes());
        glyf.push(0);
        let mut loca = vec![0; 4];
        loca[2..4].copy_from_slice(&10_u16.to_be_bytes());
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", loca.as_slice()),
            (*b"glyf", glyf.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("short loca font should parse");

        assert_eq!(
            face.outline(0)
                .expect("outline should parse")
                .unwrap()
                .contours
                .len(),
            1
        );
    }

    #[test]
    fn extracts_a_translated_composite_glyf_outline() {
        let (head, hhea, maxp, hmtx, loca, glyf) = composite_glyf_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", loca.as_slice()),
            (*b"glyf", glyf.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("composite font should parse");
        let outline = face
            .outline(1)
            .expect("composite outline should parse")
            .expect("composite glyph should have an outline");

        let contour = outline.contours[0];
        assert_eq!(
            (
                outline.points[contour.start].x,
                outline.points[contour.start].y
            ),
            (10.0, 20.0)
        );
        assert_eq!(
            (
                outline.points[contour.start + 1].x,
                outline.points[contour.start + 1].y
            ),
            (110.0, 20.0)
        );

        let metrics = face.metrics().expect("metrics should parse");
        let with_coordinates = face
            .outline_with_metrics_at_coordinates(1, metrics, &[1.0])
            .expect("static composite should parse")
            .expect("composite glyph should have an outline");
        assert_eq!(outline.points, with_coordinates.points);
    }

    #[test]
    fn extracts_a_cff1_cubic_outline() {
        let charstring = [
            cff_integer(0),
            cff_integer(0),
            21,
            cff_integer(0),
            cff_integer(50),
            cff_integer(50),
            cff_integer(50),
            cff_integer(50),
            cff_integer(0),
            8,
            14,
        ];
        let cff = cff1_table(&charstring);
        let (head, hhea, maxp) = cff_metrics_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"CFF ", cff.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("CFF1 directory should parse");
        let outline = face
            .cff_outline(0)
            .expect("CFF1 outline should parse")
            .expect("CFF1 glyph should have an outline");

        assert_eq!(outline.bounds, [0.0, 0.0, 100.0, 100.0]);
        assert!(matches!(
            outline.commands.as_slice(),
            [
                CffPathCommand::MoveTo { x: 0.0, y: 0.0 },
                CffPathCommand::CurveTo {
                    x: 100.0,
                    y: 100.0,
                    ..
                },
                CffPathCommand::Close,
            ]
        ));
    }

    #[test]
    fn extracts_a_cff2_cubic_outline() {
        let charstring = [
            cff_integer(0),
            cff_integer(0),
            21,
            cff_integer(100),
            cff_integer(0),
            5,
            cff_integer(0),
            cff_integer(100),
            5,
            14,
        ];
        let cff = cff2_table(&charstring);
        let (head, hhea, maxp) = cff_metrics_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"CFF2", cff.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("CFF2 directory should parse");
        let outline = face
            .cff_outline(0)
            .expect("CFF2 outline should parse")
            .expect("CFF2 glyph should have an outline");

        assert_eq!(outline.bounds, [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(outline.commands.len(), 4);
    }

    #[test]
    fn cff_flex_variants_preserve_the_initial_y_coordinate() {
        let hflex = [
            cff_integer(0),
            cff_integer(0),
            21,
            cff_integer(20),
            cff_integer(30),
            cff_integer(10),
            cff_integer(20),
            cff_integer(30),
            cff_integer(40),
            cff_integer(50),
            12,
            35,
            14,
        ];
        let hflex1 = [
            cff_integer(0),
            cff_integer(0),
            21,
            cff_integer(10),
            cff_integer(20),
            cff_integer(30),
            cff_integer(40),
            cff_integer(50),
            cff_integer(60),
            cff_integer(70),
            cff_integer(80),
            cff_integer(90),
            12,
            36,
            14,
        ];
        for (charstring, expected_end_x) in [(&hflex[..], 190.0), (&hflex1[..], 310.0)] {
            let cff = cff1_table(charstring);
            let (head, hhea, maxp) = cff_metrics_tables();
            let bytes = sfnt_with_tables(&[
                (*b"head", head.as_slice()),
                (*b"hhea", hhea.as_slice()),
                (*b"maxp", maxp.as_slice()),
                (*b"CFF ", cff.as_slice()),
            ]);
            let face = SfntFace::from_bytes(&bytes, 0).expect("CFF flex directory should parse");
            let outline = face
                .cff_outline(0)
                .expect("CFF flex outline should parse")
                .expect("CFF flex glyph should have an outline");
            assert!(matches!(
                outline.commands.as_slice(),
                [
                    CffPathCommand::MoveTo { x: 0.0, y: 0.0 },
                    CffPathCommand::CurveTo { x: _, y: _, .. },
                    CffPathCommand::CurveTo { x, y: 0.0, .. },
                    CffPathCommand::Close,
                ] if (*x - expected_end_x).abs() < f32::EPSILON
            ));
        }
    }

    #[test]
    fn rejects_cff2_blend_until_variation_coordinates_are_supported() {
        let charstring = [cff_integer(0), cff_integer(0), 21, 16, 14];
        let cff = cff2_table(&charstring);
        let (head, hhea, maxp) = cff_metrics_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"CFF2", cff.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("CFF2 directory should parse");

        assert!(matches!(
            face.cff_outline(0),
            Err(SfntError::UnsupportedCffOperator { tag, operator: 16 })
                if tag == super::Tag::from_bytes(*b"CFF2")
        ));
    }

    #[test]
    fn rejects_a_cff_subroutine_cycle_without_recursing_forever() {
        let cff = cff1_subroutine_cycle_table();
        let (head, hhea, maxp) = cff_metrics_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"CFF ", cff.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("CFF directory should parse");

        assert!(matches!(
            face.cff_outline(0),
            Err(SfntError::CffSubroutineCycle {
                global: false,
                index: 0
            })
        ));
    }

    #[test]
    fn rejects_a_recursive_composite_glyf_outline() {
        let (head, hhea, maxp, hmtx, loca, mut glyf) = composite_glyf_tables();
        glyf[19 + 12..19 + 14].copy_from_slice(&1_u16.to_be_bytes());
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", loca.as_slice()),
            (*b"glyf", glyf.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("directory should still parse");

        assert!(matches!(face.outline(1), Err(SfntError::CompositeCycle(1))));
        let metrics = face.metrics().expect("metrics should parse");
        assert!(matches!(
            face.outline_with_metrics_at_coordinates(1, metrics, &[1.0]),
            Err(SfntError::CompositeCycle(1))
        ));
    }

    #[test]
    fn decodes_a_unicode_family_name() {
        let name = unicode_name_table();
        let bytes = sfnt_with_tables(&[(*b"name", name.as_slice())]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("name font should parse");

        assert_eq!(
            face.name(1).expect("name should parse"),
            Some("Test Sans".to_owned())
        );
        assert_eq!(
            face.family_name().expect("family name should parse"),
            Some("Test Sans".to_owned())
        );
    }

    #[test]
    fn reads_a_checked_in_true_type_font_without_the_legacy_parser() {
        let bytes = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");
        let face = SfntFace::from_bytes(bytes, 0).expect("checked-in TTF should parse");
        let metrics = face.metrics().expect("checked-in metrics should parse");

        assert!(metrics.units_per_em > 0);
        assert!(metrics.num_glyphs > 0);
        assert!(face.has_standard_outline());
        assert!(!face.has_color_tables());
        assert_eq!(face.design_weight(), Some(400));
        super::validate_font(bytes)
            .expect("checked-in face should satisfy registration validation");
        assert!(
            face.family_name()
                .expect("checked-in name should parse")
                .is_some()
        );
        let glyph_id = face
            .glyph_index('A' as u32)
            .expect("checked-in cmap should parse")
            .expect("A should be covered");
        assert!(
            face.outline(glyph_id)
                .expect("checked-in outline should parse")
                .is_some()
        );
        assert!(face.covers('A' as u32).expect("coverage should parse"));
    }

    #[test]
    fn decodes_colr_v0_layers_and_cpal_v0_colors() {
        let mut colr = vec![0; 14 + 6 + 2 * 4];
        colr[2..4].copy_from_slice(&1_u16.to_be_bytes());
        colr[4..8].copy_from_slice(&14_u32.to_be_bytes());
        colr[8..12].copy_from_slice(&20_u32.to_be_bytes());
        colr[12..14].copy_from_slice(&2_u16.to_be_bytes());
        colr[14..16].copy_from_slice(&7_u16.to_be_bytes());
        colr[16..18].copy_from_slice(&0_u16.to_be_bytes());
        colr[18..20].copy_from_slice(&2_u16.to_be_bytes());
        colr[20..22].copy_from_slice(&9_u16.to_be_bytes());
        colr[22..24].copy_from_slice(&1_u16.to_be_bytes());
        colr[24..26].copy_from_slice(&10_u16.to_be_bytes());
        colr[26..28].copy_from_slice(&0_u16.to_be_bytes());

        let mut cpal = vec![0; 14 + 2 * 4];
        cpal[2..4].copy_from_slice(&2_u16.to_be_bytes());
        cpal[4..6].copy_from_slice(&1_u16.to_be_bytes());
        cpal[6..8].copy_from_slice(&2_u16.to_be_bytes());
        cpal[8..12].copy_from_slice(&14_u32.to_be_bytes());
        cpal[12..14].copy_from_slice(&0_u16.to_be_bytes());
        cpal[14..18].copy_from_slice(&[0, 0, 255, 255]);
        cpal[18..22].copy_from_slice(&[0, 255, 0, 128]);

        let bytes = sfnt_with_tables(&[(*b"COLR", colr.as_slice()), (*b"CPAL", cpal.as_slice())]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("color face should parse");

        assert_eq!(
            face.color_layers(7).expect("COLR should parse"),
            Some(
                &[
                    super::color::ColorLayer {
                        glyph_id: 9,
                        palette_index: 1,
                    },
                    super::color::ColorLayer {
                        glyph_id: 10,
                        palette_index: 0,
                    },
                ][..]
            )
        );
        assert_eq!(
            face.palette_color(0).expect("CPAL should parse"),
            Some(super::color::ColorRgba::new(255, 0, 0, 255))
        );
        assert_eq!(
            face.palette_color(1).expect("CPAL should parse"),
            Some(super::color::ColorRgba::new(0, 255, 0, 128))
        );
        assert!(face.has_color_tables());
    }

    #[test]
    fn classifies_hvgl_as_private_outline_data_and_declines_owned_rasterization() {
        let (head, hhea, maxp, hmtx) = metrics_tables();
        // The bytes are intentionally not a made-up hvgl decoder fixture:
        // portable behavior depends only on the validated directory tag.
        let private_payload = [0xff, 0x00, 0x7f, 0x11];
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"hvgl", &private_payload),
            (*b"maxp", maxp.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("private face directory should parse");

        let private = face.apple_private_tables();
        assert!(private.has_hvgl());
        assert!(!private.has_emjc());
        assert!(!face.has_standard_outline());
        assert!(!face.has_color_tables());
        assert!(face.requires_platform_rasterization());

        let metrics = face.metrics().expect("private face metrics should parse");
        let mut cache = super::rasterize::GlyphRasterCache::default();
        assert!(
            super::rasterize::rasterize_face_glyph(
                &face,
                &metrics,
                0,
                32.0,
                0,
                0,
                super::super::glyph_rasterizer::NORMAL_GLYPH_WEIGHT,
                &mut cache,
            )
            .is_none()
        );
    }

    #[test]
    fn classifies_emjc_as_private_color_data_without_entering_public_decoders() {
        let (head, hhea, maxp, hmtx) = metrics_tables();
        // An invalid public sbix table beside emjc must not make the private
        // face enter the sbix parser. The private-only guard runs first.
        let invalid_sbix = [0_u8; 8];
        let cmap = format12_cmap();
        let private_payload = [0x01, 0x02, 0x03];
        let bytes = sfnt_with_tables(&[
            (*b"cmap", cmap.as_slice()),
            (*b"emjc", &private_payload),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"sbix", &invalid_sbix),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("private color directory should parse");

        assert!(face.has_apple_private_color_tables());
        assert!(face.has_color_tables());
        assert!(face.requires_platform_rasterization());

        let record = super::super::font_resolver::FontRecord::from_bytes(7, bytes.clone())
            .expect("private color metadata should be accepted");
        assert!(record.is_color, "emjc must route through color fallback");

        let metrics = face.metrics().expect("private color metrics should parse");
        let mut cache = super::rasterize::GlyphRasterCache::default();
        assert!(
            super::rasterize::rasterize_face_glyph(
                &face,
                &metrics,
                0,
                32.0,
                0,
                0,
                super::super::glyph_rasterizer::NORMAL_GLYPH_WEIGHT,
                &mut cache,
            )
            .is_none()
        );
    }

    /// An `sbix` table with one strike holding one glyph whose graphic type is
    /// `graphic_type`. The three payload bytes are never decoded by the
    /// classification under test.
    fn one_glyph_sbix(graphic_type: &[u8; 4]) -> Vec<u8> {
        let record_start = 12;
        let record_end = record_start + 8 + 3;
        let mut sbix = Vec::new();
        sbix.extend_from_slice(&1_u16.to_be_bytes());
        sbix.extend_from_slice(&0_u16.to_be_bytes());
        sbix.extend_from_slice(&1_u32.to_be_bytes());
        sbix.extend_from_slice(&12_u32.to_be_bytes());
        sbix.extend_from_slice(&16_u16.to_be_bytes());
        sbix.extend_from_slice(&72_i16.to_be_bytes());
        sbix.extend_from_slice(&(record_start as u32).to_be_bytes());
        sbix.extend_from_slice(&(record_end as u32).to_be_bytes());
        sbix.extend_from_slice(&0_i16.to_be_bytes());
        sbix.extend_from_slice(&0_i16.to_be_bytes());
        sbix.extend_from_slice(graphic_type);
        sbix.extend_from_slice(&[0x01, 0x02, 0x03]);
        sbix
    }

    #[test]
    fn classifies_emjc_graphic_type_inside_sbix_as_private_color_data() {
        let (head, mut hhea, mut maxp, hmtx) = metrics_tables();
        hhea[34..36].copy_from_slice(&1_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&1_u16.to_be_bytes());
        let sbix = one_glyph_sbix(b"emjc");

        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"sbix", sbix.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("sbix private face should parse");

        assert!(face.has_apple_private_color_tables());
        assert!(face.requires_platform_rasterization());
    }

    /// The iOS emoji face (`AppleColorEmoji-160px.ttc`) keeps a `glyf` table
    /// whose emoji entries are empty, beside `emjc` strikes only the platform
    /// can decode. Treating that placeholder as "a public outline" sent the
    /// face down the owned path, which found nothing to draw and returned a
    /// blank glyph instead of declining, so every emoji vanished on iOS.
    #[test]
    fn emjc_color_face_with_placeholder_outlines_still_routes_to_the_platform() {
        let (head, mut hhea, mut maxp, hmtx) = metrics_tables();
        hhea[34..36].copy_from_slice(&1_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&1_u16.to_be_bytes());
        let sbix = one_glyph_sbix(b"emjc");
        let placeholder_glyf = [0_u8; 4];

        let bytes = sfnt_with_tables(&[
            (*b"glyf", placeholder_glyf.as_slice()),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"sbix", sbix.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("emoji face should parse");

        assert!(face.has_standard_outline());
        assert!(face.has_apple_private_color_tables());
        assert!(
            face.requires_platform_rasterization(),
            "private color strikes must reach the platform even beside a glyf table"
        );

        let metrics = face.metrics().expect("emoji face metrics should parse");
        let mut cache = super::rasterize::GlyphRasterCache::default();
        assert!(
            super::rasterize::rasterize_face_glyph(
                &face,
                &metrics,
                0,
                16.0,
                0,
                0,
                super::super::glyph_rasterizer::NORMAL_GLYPH_WEIGHT,
                &mut cache,
            )
            .is_none(),
            "declining is what hands the glyph to the platform rasterizer"
        );
    }

    /// The other side of the rule: a bitmap emoji face whose strikes are
    /// public formats (`png `) is decoded by the owned path, outline table or
    /// not, and must not be pushed onto the platform.
    #[test]
    fn public_sbix_color_face_with_outlines_stays_on_the_owned_path() {
        let (head, mut hhea, mut maxp, hmtx) = metrics_tables();
        hhea[34..36].copy_from_slice(&1_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&1_u16.to_be_bytes());
        let sbix = one_glyph_sbix(b"png ");
        let glyf = [0_u8; 4];

        let bytes = sfnt_with_tables(&[
            (*b"glyf", glyf.as_slice()),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"sbix", sbix.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("public sbix face should parse");

        assert!(face.has_standard_outline());
        assert!(!face.has_apple_private_color_tables());
        assert!(!face.requires_platform_rasterization());
    }

    #[test]
    fn owned_rasterizer_emits_rgba_for_a_colr_outline_layer() {
        let bytes = simple_colr_v0_font();
        let face = SfntFace::from_bytes(&bytes, 0).expect("COLR outline face should parse");
        let metrics = face.metrics().expect("COLR outline metrics should parse");
        let mut cache = super::rasterize::GlyphRasterCache::default();
        let glyph = super::rasterize::rasterize_face_glyph(
            &face,
            &metrics,
            0,
            32.0,
            0,
            0,
            crate::glyph_rasterizer::NORMAL_GLYPH_WEIGHT,
            &mut cache,
        )
        .expect("COLR outline should rasterize");

        assert!(glyph.is_color);
        assert_eq!(
            glyph.bitmap.len(),
            (glyph.width * glyph.height * 4) as usize
        );
        assert!(glyph.bitmap.chunks_exact(4).any(|pixel| pixel[3] != 0));
        assert!(
            glyph
                .bitmap
                .chunks_exact(4)
                .filter(|pixel| pixel[3] != 0)
                .all(|pixel| pixel[0] == 255 && pixel[1] == 0 && pixel[2] == 0)
        );
    }

    #[test]
    fn owned_colr_rasterization_matches_size_pixel_goldens() {
        let bytes = simple_colr_v0_font();
        let face = SfntFace::from_bytes(&bytes, 0).expect("COLR outline face should parse");
        let metrics = face.metrics().expect("COLR outline metrics should parse");
        let mut cache = super::rasterize::GlyphRasterCache::default();
        let actual = [16.0_f32, 24.0, 32.0].map(|font_size| {
            let glyph = super::rasterize::rasterize_face_glyph(
                &face,
                &metrics,
                0,
                font_size,
                0,
                0,
                crate::glyph_rasterizer::NORMAL_GLYPH_WEIGHT,
                &mut cache,
            )
            .expect("COLR outline should rasterize");

            assert!(glyph.is_color);
            assert_eq!(
                glyph.bitmap.len(),
                (glyph.width * glyph.height * 4) as usize
            );
            assert!(glyph.bitmap.chunks_exact(4).any(|pixel| pixel[3] != 0));
            (glyph.width, glyph.height, bitmap_fingerprint(&glyph.bitmap))
        });

        assert_eq!(
            actual,
            [
                (2, 2, 14_551_470_036_939_313_687),
                (3, 3, 16_467_940_706_764_202_044),
                (4, 4, 3_532_343_437_148_095_129),
            ]
        );
    }

    #[test]
    fn owned_rasterizer_emits_rgba_for_an_svg_glyph() {
        let (head, hhea, maxp, hmtx, loca, glyf) = simple_glyf_tables();
        let document = br##"<svg xmlns="http://www.w3.org/2000/svg"><path fill="#ff0000" d="M0 0L800 0L800 -800Z"/></svg>"##;
        let document_start = 10 + 2 + 12;
        let mut svg = vec![0_u8; document_start + document.len()];
        svg[2..6].copy_from_slice(&10_u32.to_be_bytes());
        svg[10..12].copy_from_slice(&1_u16.to_be_bytes());
        svg[12..14].copy_from_slice(&0_u16.to_be_bytes());
        svg[14..16].copy_from_slice(&0_u16.to_be_bytes());
        svg[16..20].copy_from_slice(&(document_start as u32).to_be_bytes());
        svg[20..24].copy_from_slice(&(document.len() as u32).to_be_bytes());
        svg[document_start..].copy_from_slice(document);

        let bytes = sfnt_with_tables(&[
            (*b"SVG ", svg.as_slice()),
            (*b"glyf", glyf.as_slice()),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"loca", loca.as_slice()),
            (*b"maxp", maxp.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("SVG face should parse");
        assert!(face.has_color_tables());
        assert!(face.svg_glyph(0).is_some());
        let metrics = face.metrics().expect("SVG metrics should parse");
        let mut cache = super::rasterize::GlyphRasterCache::default();
        let glyph = super::rasterize::rasterize_face_glyph(
            &face,
            &metrics,
            0,
            32.0,
            0,
            0,
            crate::glyph_rasterizer::NORMAL_GLYPH_WEIGHT,
            &mut cache,
        )
        .expect("SVG glyph should rasterize");

        assert!(glyph.is_color);
        assert_eq!(
            glyph.bitmap.len(),
            (glyph.width * glyph.height * 4) as usize
        );
        assert!(glyph.bitmap.chunks_exact(4).any(|pixel| pixel[3] != 0));
        assert!(
            glyph
                .bitmap
                .chunks_exact(4)
                .filter(|pixel| pixel[3] != 0)
                .all(|pixel| pixel[0] == 255 && pixel[1] == 0 && pixel[2] == 0)
        );
    }

    #[test]
    fn rejects_colr_layer_ranges_and_palette_indices() {
        let mut malformed_colr = vec![0; 14 + 6 + 4];
        malformed_colr[2..4].copy_from_slice(&1_u16.to_be_bytes());
        malformed_colr[4..8].copy_from_slice(&14_u32.to_be_bytes());
        malformed_colr[8..12].copy_from_slice(&20_u32.to_be_bytes());
        malformed_colr[12..14].copy_from_slice(&1_u16.to_be_bytes());
        malformed_colr[14..16].copy_from_slice(&0_u16.to_be_bytes());
        malformed_colr[16..18].copy_from_slice(&1_u16.to_be_bytes());
        malformed_colr[18..20].copy_from_slice(&1_u16.to_be_bytes());
        let cpal = {
            let mut table = vec![0; 18];
            table[2..4].copy_from_slice(&1_u16.to_be_bytes());
            table[4..6].copy_from_slice(&1_u16.to_be_bytes());
            table[6..8].copy_from_slice(&1_u16.to_be_bytes());
            table[8..12].copy_from_slice(&14_u32.to_be_bytes());
            table[14..18].copy_from_slice(&[0, 0, 255, 255]);
            table
        };
        let bytes = sfnt_with_tables(&[
            (*b"COLR", malformed_colr.as_slice()),
            (*b"CPAL", cpal.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("directory should parse");
        assert!(matches!(
            face.color_layers(0),
            Err(SfntError::MalformedTable(tag)) if tag == Tag::from_bytes(*b"COLR")
        ));

        malformed_colr[16..18].copy_from_slice(&0_u16.to_be_bytes());
        malformed_colr[18..20].copy_from_slice(&1_u16.to_be_bytes());
        malformed_colr[22..24].copy_from_slice(&1_u16.to_be_bytes());
        let bytes = sfnt_with_tables(&[
            (*b"COLR", malformed_colr.as_slice()),
            (*b"CPAL", cpal.as_slice()),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("directory should parse");
        assert!(matches!(
            face.color_layers(0),
            Err(SfntError::MalformedTable(tag)) if tag == Tag::from_bytes(*b"CPAL")
        ));
    }

    #[test]
    fn reads_a_checked_in_cjk_font_coverage() {
        let bytes = include_bytes!("../fonts/NotoSansJP-VariableFont_wght.ttf");
        let face = SfntFace::from_bytes(bytes, 0).expect("checked-in CJK TTF should parse");

        assert!(face.covers(0x4e00).expect("CJK coverage should parse"));
    }

    #[test]
    fn rejects_an_out_of_range_format12_glyph_id() {
        let mut cmap = format12_cmap();
        cmap[36..40].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        let bytes = sfnt_with_cmap(&cmap);
        let face = SfntFace::from_bytes(&bytes, 0).expect("directory should still parse");

        assert!(matches!(
            face.glyph_index(0x4e00),
            Err(SfntError::CmapGlyphOutOfRange(0x0001_0000))
        ));
    }

    #[test]
    fn rejects_a_short_hmtx_table() {
        let (head, hhea, maxp, hmtx) = metrics_tables();
        let bytes = sfnt_with_tables(&[
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"hmtx", &hmtx[..8]),
        ]);
        let face = SfntFace::from_bytes(&bytes, 0).expect("directory should still parse");

        assert!(matches!(
            face.glyph_advance(0),
            Err(SfntError::MalformedTable(tag)) if tag == super::Tag::from_bytes(*b"hmtx")
        ));
    }

    #[test]
    fn bounded_fuzz_entry_points_traverse_directory_and_outline_inputs() {
        super::fuzz_directory(&[0, 1, 2, 3, 4, 5, 6, 7]);
        super::fuzz_outlines(&[0xff; 512]);
    }

    #[test]
    fn selects_a_face_from_a_true_type_collection() {
        let first = minimal_sfnt();
        let second = minimal_sfnt();
        let first_offset = 20;
        let second_offset = first_offset + first.len();
        let mut bytes = vec![0; second_offset + second.len()];
        bytes[0..4].copy_from_slice(b"ttcf");
        bytes[4..8].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        bytes[8..12].copy_from_slice(&2_u32.to_be_bytes());
        bytes[12..16].copy_from_slice(&(first_offset as u32).to_be_bytes());
        bytes[16..20].copy_from_slice(&(second_offset as u32).to_be_bytes());
        bytes[first_offset..second_offset].copy_from_slice(&first);
        bytes[second_offset..].copy_from_slice(&second);
        bytes[first_offset + 20..first_offset + 24]
            .copy_from_slice(&((first_offset + 28) as u32).to_be_bytes());
        bytes[second_offset + 20..second_offset + 24]
            .copy_from_slice(&((second_offset + 28) as u32).to_be_bytes());
        bytes[second_offset + 28..second_offset + 32].copy_from_slice(b"face");

        let face = SfntFace::from_bytes(&bytes, 1).expect("second TTC face should parse");

        assert_eq!(face.table(*b"head"), Some(&b"face"[..]));
    }

    #[test]
    fn rejects_a_truncated_directory() {
        let bytes = [0, 1, 0, 0, 0, 1];

        assert!(matches!(
            SfntFace::from_bytes(&bytes, 0),
            Err(SfntError::Truncated { .. })
        ));
    }

    #[test]
    fn rejects_a_table_that_extends_past_the_font() {
        let mut bytes = minimal_sfnt();
        bytes[20..24].copy_from_slice(&31_u32.to_be_bytes());

        assert!(matches!(
            SfntFace::from_bytes(&bytes, 0),
            Err(SfntError::TableOutOfBounds { tag, .. }) if tag == super::Tag::from_bytes(*b"head")
        ));
    }

    #[test]
    fn rejects_an_oversized_table_before_allocating_or_slicing() {
        let mut bytes = minimal_sfnt();
        bytes[24..28].copy_from_slice(&((MAX_TABLE_BYTES as u32) + 1).to_be_bytes());

        assert!(matches!(
            SfntFace::from_bytes(&bytes, 0),
            Err(SfntError::TableTooLarge { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_table_tags() {
        let mut bytes = vec![0; 44];
        bytes[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        bytes[4..6].copy_from_slice(&2_u16.to_be_bytes());
        bytes[12..16].copy_from_slice(b"head");
        bytes[20..24].copy_from_slice(&44_u32.to_be_bytes());
        bytes[28..32].copy_from_slice(b"head");
        bytes[36..40].copy_from_slice(&44_u32.to_be_bytes());

        assert!(matches!(
            SfntFace::from_bytes(&bytes, 0),
            Err(SfntError::DuplicateTable(tag)) if tag == super::Tag::from_bytes(*b"head")
        ));
    }

    #[test]
    fn shapes_latin_ligatures_and_pair_kerning_from_open_type_tables() {
        let face = SfntFace::from_bytes(include_bytes!("../fonts/GoogleSans-Regular.ttf"), 0)
            .expect("the checked-in shaping fixture must parse");

        let office = super::shape_latin_run(&face, "office")
            .expect("GSUB parsing should succeed")
            .expect("ASCII text should use the Aimer shaping seam");
        assert_eq!(
            office
                .glyphs
                .iter()
                .map(|glyph| glyph.glyph_id)
                .collect::<Vec<_>>(),
            vec![271, 386, 203, 213]
        );
        assert_eq!(office.glyphs.len(), 4);
        assert_eq!(
            office
                .glyphs
                .iter()
                .map(|glyph| glyph.cluster)
                .collect::<Vec<_>>(),
            vec![0, 1, 4, 5]
        );
        assert_eq!(
            office
                .glyphs
                .iter()
                .map(|glyph| glyph.x_advance)
                .collect::<Vec<_>>(),
            vec![574, 905, 530, 561]
        );

        let av = super::shape_latin_run(&face, "AV")
            .expect("GPOS parsing should succeed")
            .expect("ASCII text should use the Aimer shaping seam");
        let a = face
            .glyph_index('A' as u32)
            .expect("A cmap lookup should succeed")
            .expect("the fixture must contain A");
        let v = face
            .glyph_index('V' as u32)
            .expect("V cmap lookup should succeed")
            .expect("the fixture must contain V");
        let unkerned_width = i32::from(
            face.glyph_advance(a)
                .expect("A advance lookup should succeed")
                .expect("A must have an advance"),
        ) + i32::from(
            face.glyph_advance(v)
                .expect("V advance lookup should succeed")
                .expect("V must have an advance"),
        );
        let kerned_width: i32 = av.glyphs.iter().map(|glyph| glyph.x_advance).sum();
        assert!(
            kerned_width < unkerned_width,
            "AV must receive a negative pair adjustment"
        );
        assert_eq!(
            av.glyphs
                .iter()
                .map(|glyph| glyph.x_advance)
                .collect::<Vec<_>>(),
            vec![590, 633]
        );
    }

    #[test]
    fn shapes_an_indic_prebase_matra_before_feature_substitution() {
        let bytes = indic_reordering_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("Indic fixture must parse");

        let shaped = super::shape_run_with_options(&face, "\u{0915}\u{093f}", None, false)
            .expect("Indic GSUB parsing should succeed")
            .expect("the Indic fixture should use the Aimer shaping seam");

        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.glyph_id)
                .collect::<Vec<_>>(),
            vec![3, 1]
        );
        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.cluster)
                .collect::<Vec<_>>(),
            vec![0, 0]
        );
        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.x_advance)
                .collect::<Vec<_>>(),
            vec![0, 600]
        );
    }

    #[test]
    fn applies_an_indic_contextual_substitution_after_reordering() {
        let bytes = indic_context_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("Indic context fixture must parse");

        let shaped = super::shape_run_with_options(&face, "\u{0915}\u{094d}", None, false)
            .expect("Indic contextual GSUB parsing should succeed")
            .expect("the Indic context fixture should use the Aimer shaping seam");

        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.glyph_id)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
    }

    #[test]
    fn attaches_an_indic_prebase_matra_with_abvm_positioning() {
        let bytes = indic_mark_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("Indic mark fixture must parse");

        let shaped = super::shape_run_with_options(&face, "\u{0915}\u{093f}", None, false)
            .expect("Indic GPOS parsing should succeed")
            .expect("the Indic mark fixture should use the Aimer shaping seam");

        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.glyph_id)
                .collect::<Vec<_>>(),
            vec![8, 3]
        );
        assert_eq!(
            (shaped.glyphs[0].x_offset, shaped.glyphs[0].y_offset),
            (200, 700)
        );
        assert_eq!(shaped.glyphs[0].x_advance, 0);
    }

    #[test]
    fn shapes_arabic_joining_forms_from_the_arab_script_features() {
        let bytes = arabic_joining_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("Arabic fixture must parse");

        let shaped = super::shape_arabic_run(&face, "ببب")
            .expect("Arabic GSUB parsing should succeed")
            .expect("the Arabic fixture should use the Aimer shaping seam");

        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.glyph_id)
                .collect::<Vec<_>>(),
            vec![3, 5, 4]
        );
        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.cluster)
                .collect::<Vec<_>>(),
            vec![0, 2, 4]
        );
        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.x_advance)
                .collect::<Vec<_>>(),
            vec![700, 800, 700]
        );
    }

    #[test]
    fn leaves_uncovered_arabic_mark_runs_to_the_compatibility_shaper() {
        let bytes = arabic_joining_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("Arabic fixture must parse");

        assert_eq!(super::shape_arabic_run(&face, "بَ\u{0650}"), Ok(None));
    }

    #[test]
    fn attaches_an_arabic_mark_with_mark_to_base_positioning() {
        let bytes = arabic_joining_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("Arabic fixture must parse");

        let shaped = super::shape_arabic_run(&face, "بَب")
            .expect("Arabic GSUB and GPOS parsing should succeed")
            .expect("the Arabic mark fixture should use the Aimer shaping seam");

        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.glyph_id)
                .collect::<Vec<_>>(),
            vec![3, 8, 4]
        );
        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.cluster)
                .collect::<Vec<_>>(),
            vec![0, 2, 4]
        );
        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.x_advance)
                .collect::<Vec<_>>(),
            vec![700, 0, 700]
        );
        assert_eq!(shaped.glyphs[1].x_offset, 200);
        assert_eq!(shaped.glyphs[1].y_offset, 700);
    }

    #[test]
    fn applies_an_arabic_required_ligature_after_joining_forms() {
        let bytes = arabic_joining_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("Arabic fixture must parse");

        let shaped = super::shape_arabic_run(&face, "بب")
            .expect("Arabic GSUB parsing should succeed")
            .expect("the Arabic ligature fixture should use the Aimer shaping seam");

        assert_eq!(shaped.glyphs.len(), 1);
        assert_eq!(shaped.glyphs[0].glyph_id, 9);
        assert_eq!(shaped.glyphs[0].cluster, 0);
        assert_eq!(shaped.glyphs[0].x_advance, 900);
    }

    #[test]
    fn attaches_a_second_arabic_mark_with_mark_to_mark_positioning() {
        let bytes = arabic_joining_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("Arabic fixture must parse");

        let shaped = super::shape_arabic_run(&face, "بََب")
            .expect("Arabic mark-to-mark parsing should succeed")
            .expect("the Arabic mark-to-mark fixture should use the Aimer shaping seam");

        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.glyph_id)
                .collect::<Vec<_>>(),
            vec![3, 8, 8, 4]
        );
        assert_eq!(shaped.glyphs[1].x_offset, 200);
        assert_eq!(shaped.glyphs[1].y_offset, 700);
        assert_eq!(shaped.glyphs[2].x_offset, 250);
        assert_eq!(shaped.glyphs[2].y_offset, 1000);
        assert_eq!(shaped.glyphs[1].x_advance, 0);
        assert_eq!(shaped.glyphs[2].x_advance, 0);
    }

    #[test]
    fn applies_arabic_cursive_entry_exit_positioning() {
        let bytes = arabic_joining_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("Arabic fixture must parse");

        let shaped = super::shape_arabic_run(&face, "با")
            .expect("Arabic cursive parsing should succeed")
            .expect("the Arabic cursive fixture should use the Aimer shaping seam");

        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.glyph_id)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert_eq!(
            (shaped.glyphs[1].x_offset, shaped.glyphs[1].y_offset),
            (100, 60)
        );
        assert_eq!(shaped.glyphs[0].x_advance, 700);
        assert_eq!(shaped.glyphs[1].x_advance, 600);
    }

    #[test]
    fn applies_arabic_contextual_substitution_after_joining_forms() {
        let bytes = arabic_joining_font_for_test();
        let face = SfntFace::from_bytes(&bytes, 0).expect("Arabic fixture must parse");

        let shaped = super::shape_arabic_run(&face, "اب")
            .expect("Arabic contextual parsing should succeed")
            .expect("the Arabic contextual fixture should use the Aimer shaping seam");

        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.glyph_id)
                .collect::<Vec<_>>(),
            vec![7, 6]
        );
        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.cluster)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.x_advance)
                .collect::<Vec<_>>(),
            vec![600, 600]
        );
    }

    #[test]
    fn shapes_google_sans_devanagari_through_the_owned_path() {
        let face = SfntFace::from_bytes(include_bytes!("../fonts/GoogleSans-Regular.ttf"), 0)
            .expect("Google Sans must parse");
        let shaped = super::shape_run_with_options(&face, "कि", None, false)
            .expect("Google Sans Indic shaping should parse")
            .expect("Google Sans Devanagari should use the Aimer shaping seam");

        assert_eq!(shaped.glyphs.len(), 2);
        assert_eq!(
            shaped
                .glyphs
                .iter()
                .map(|glyph| (glyph.glyph_id, glyph.cluster, glyph.x_advance))
                .collect::<Vec<_>>(),
            vec![(2911, 0, 248), (2569, 0, 870)]
        );
    }

    #[test]
    fn shapes_southeast_asian_runs_through_the_owned_path() {
        let face = SfntFace::from_bytes(include_bytes!("../fonts/GoogleSans-Regular.ttf"), 0)
            .expect("Google Sans must parse");
        let cases = [
            (
                "สวัสดี",
                vec![3227, 3220, 3250, 3227, 3205, 3259],
                vec![572, 453, 0, 572, 608, 0],
                vec![(0, 0), (0, 0), (5, 0), (0, 0), (0, 0), (-5, 0)],
            ),
            (
                "ສະບາຍດີ",
                vec![3544, 3553, 3530, 3554, 3517, 3524, 3577],
                vec![653, 426, 629, 439, 586, 596, 0],
                vec![(0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (-32, 0)],
            ),
            (
                "សួស្តី",
                vec![3663, 3949, 3663, 3890, 3938],
                vec![903, 0, 903, 0, 0],
                vec![(0, 0), (-37, 0), (0, 0), (-11, 0), (26, 0)],
            ),
            (
                "សួស្តីពិភពលោក",
                vec![
                    3663, 3949, 3663, 3890, 3938, 3654, 3937, 3655, 3654, 3708, 3744, 3632,
                ],
                vec![903, 0, 903, 0, 0, 600, 0, 631, 600, 328, 1208, 604],
                vec![
                    (0, 0),
                    (-37, 0),
                    (0, 0),
                    (-11, 0),
                    (26, 0),
                    (0, 0),
                    (8, 0),
                    (0, 0),
                    (0, 0),
                    (0, 0),
                    (0, 0),
                    (0, 0),
                ],
            ),
            (
                "សួស្តី\u{200b}ពិភពលោក",
                vec![
                    3663, 3949, 3663, 3890, 3938, 1866, 3654, 3937, 3655, 3654, 3708, 3744, 3632,
                ],
                vec![903, 0, 903, 0, 0, 0, 600, 0, 631, 600, 328, 1208, 604],
                vec![
                    (0, 0),
                    (-37, 0),
                    (0, 0),
                    (-11, 0),
                    (26, 0),
                    (0, 0),
                    (0, 0),
                    (8, 0),
                    (0, 0),
                    (0, 0),
                    (0, 0),
                    (0, 0),
                    (0, 0),
                ],
            ),
        ];

        for (text, expected_ids, expected_advances, expected_offsets) in cases {
            let shaped = super::shape_run_with_options(&face, text, None, false)
                .expect("Southeast Asian shaping should parse")
                .expect("Google Sans Southeast Asian text should use the Aimer shaping seam");
            assert_eq!(
                shaped
                    .glyphs
                    .iter()
                    .map(|glyph| glyph.glyph_id)
                    .collect::<Vec<_>>(),
                expected_ids,
                "glyph IDs for {text}"
            );
            assert_eq!(
                shaped
                    .glyphs
                    .iter()
                    .map(|glyph| glyph.x_advance)
                    .collect::<Vec<_>>(),
                expected_advances,
                "advances for {text}"
            );
            assert_eq!(
                shaped
                    .glyphs
                    .iter()
                    .map(|glyph| (glyph.x_offset, glyph.y_offset))
                    .collect::<Vec<_>>(),
                expected_offsets,
                "offsets for {text}"
            );
        }
    }

    #[test]
    fn keeps_myanmar_on_compatibility_path_when_the_face_has_no_coverage() {
        let face = SfntFace::from_bytes(include_bytes!("../fonts/GoogleSans-Regular.ttf"), 0)
            .expect("Google Sans must parse");
        assert!(
            super::shape_run_with_options(&face, "မင်္ဂလာပါ", None, false)
                .expect("missing Myanmar coverage should not be malformed")
                .is_none()
        );
    }

    pub(crate) fn arabic_joining_font_for_test() -> Vec<u8> {
        let (mut head, mut hhea, mut maxp, _) = metrics_tables();
        hhea[34..36].copy_from_slice(&10_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&10_u16.to_be_bytes());

        let mut hmtx = vec![0; 40];
        for (glyph_id, advance) in [
            (0, 600_u16),
            (1, 600),
            (2, 600),
            (3, 700),
            (4, 700),
            (5, 800),
            (6, 600),
            (7, 600),
            (8, 0),
            (9, 900),
        ] {
            let offset = glyph_id * 4;
            hmtx[offset..offset + 2].copy_from_slice(&advance.to_be_bytes());
        }
        head[50..52].copy_from_slice(&0_i16.to_be_bytes());

        // Keep the directory lexicographically sorted for the cached
        // table lookup; the checked Aimer reader accepts either order.
        sfnt_with_tables(&[
            (*b"GPOS", arabic_gpos().as_slice()),
            (*b"GSUB", arabic_gsub().as_slice()),
            (*b"cmap", arabic_cmap().as_slice()),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"maxp", maxp.as_slice()),
        ])
    }

    fn arabic_cmap() -> Vec<u8> {
        let mut subtable = vec![0; 48];
        subtable[0..2].copy_from_slice(&4_u16.to_be_bytes());
        subtable[2..4].copy_from_slice(&48_u16.to_be_bytes());
        subtable[6..8].copy_from_slice(&8_u16.to_be_bytes());
        subtable[14..16].copy_from_slice(&0x0627_u16.to_be_bytes());
        subtable[16..18].copy_from_slice(&0x0628_u16.to_be_bytes());
        subtable[18..20].copy_from_slice(&0x064e_u16.to_be_bytes());
        subtable[20..22].copy_from_slice(&0xffff_u16.to_be_bytes());
        subtable[24..26].copy_from_slice(&0x0627_u16.to_be_bytes());
        subtable[26..28].copy_from_slice(&0x0628_u16.to_be_bytes());
        subtable[28..30].copy_from_slice(&0x064e_u16.to_be_bytes());
        subtable[30..32].copy_from_slice(&0xffff_u16.to_be_bytes());
        subtable[32..34].copy_from_slice(&(-1573_i16).to_be_bytes());
        subtable[34..36].copy_from_slice(&(-1575_i16).to_be_bytes());
        subtable[36..38].copy_from_slice(&(-1606_i16).to_be_bytes());

        let mut cmap = vec![0; 12];
        cmap[2..4].copy_from_slice(&1_u16.to_be_bytes());
        cmap[4..6].copy_from_slice(&3_u16.to_be_bytes());
        cmap[6..8].copy_from_slice(&1_u16.to_be_bytes());
        cmap[8..12].copy_from_slice(&12_u32.to_be_bytes());
        cmap.extend_from_slice(&subtable);
        cmap
    }

    fn arabic_gsub() -> Vec<u8> {
        let feature_tags = [*b"init", *b"medi", *b"fina", *b"isol", *b"rlig", *b"calt"];
        let feature_lookups = [0_u16, 1, 2, 3, 4, 6];
        let mappings = [[(1_u16, 3_u16)], [(1, 5)], [(1, 4)], [(1, 6)]];

        let mut script_list = vec![0; 32];
        script_list[0..2].copy_from_slice(&1_u16.to_be_bytes());
        script_list[2..6].copy_from_slice(b"arab");
        script_list[6..8].copy_from_slice(&8_u16.to_be_bytes());
        script_list[8..10].copy_from_slice(&6_u16.to_be_bytes());
        script_list[18..20].copy_from_slice(&6_u16.to_be_bytes());
        for (index, feature_index) in [0_u16, 1, 2, 3, 4, 5].into_iter().enumerate() {
            let offset = 20 + index * 2;
            script_list[offset..offset + 2].copy_from_slice(&feature_index.to_be_bytes());
        }

        let mut feature_list = vec![0; 74];
        feature_list[0..2].copy_from_slice(&6_u16.to_be_bytes());
        for (index, tag) in feature_tags.into_iter().enumerate() {
            let record = 2 + index * 6;
            feature_list[record..record + 4].copy_from_slice(&tag);
            let feature_offset = 38 + index * 6;
            feature_list[record + 4..record + 6]
                .copy_from_slice(&(feature_offset as u16).to_be_bytes());
            feature_list[feature_offset + 2..feature_offset + 4]
                .copy_from_slice(&1_u16.to_be_bytes());
            feature_list[feature_offset + 4..feature_offset + 6]
                .copy_from_slice(&feature_lookups[index].to_be_bytes());
        }

        let mut lookup_list = vec![0; 16];
        lookup_list[0..2].copy_from_slice(&7_u16.to_be_bytes());
        for (index, mapping) in mappings.into_iter().enumerate() {
            let subtable = single_substitution(&mapping);
            let lookup_offset = lookup_list.len();
            lookup_list[2 + index * 2..4 + index * 2]
                .copy_from_slice(&(lookup_offset as u16).to_be_bytes());
            lookup_list.extend_from_slice(&1_u16.to_be_bytes());
            lookup_list.extend_from_slice(&0_u16.to_be_bytes());
            lookup_list.extend_from_slice(&1_u16.to_be_bytes());
            lookup_list.extend_from_slice(&8_u16.to_be_bytes());
            lookup_list.extend_from_slice(&subtable);
        }
        let ligature = arabic_ligature_substitution();
        let lookup_offset = lookup_list.len();
        lookup_list[10..12].copy_from_slice(&(lookup_offset as u16).to_be_bytes());
        lookup_list.extend_from_slice(&4_u16.to_be_bytes());
        lookup_list.extend_from_slice(&0_u16.to_be_bytes());
        lookup_list.extend_from_slice(&1_u16.to_be_bytes());
        lookup_list.extend_from_slice(&8_u16.to_be_bytes());
        lookup_list.extend_from_slice(&ligature);

        let contextual_substitution = single_substitution(&[(2_u16, 7_u16)]);
        let lookup_offset = lookup_list.len();
        lookup_list[12..14].copy_from_slice(&(lookup_offset as u16).to_be_bytes());
        lookup_list.extend_from_slice(&1_u16.to_be_bytes());
        lookup_list.extend_from_slice(&0_u16.to_be_bytes());
        lookup_list.extend_from_slice(&1_u16.to_be_bytes());
        lookup_list.extend_from_slice(&8_u16.to_be_bytes());
        lookup_list.extend_from_slice(&contextual_substitution);

        let contextual = arabic_contextual_substitution();
        let lookup_offset = lookup_list.len();
        lookup_list[14..16].copy_from_slice(&(lookup_offset as u16).to_be_bytes());
        lookup_list.extend_from_slice(&6_u16.to_be_bytes());
        lookup_list.extend_from_slice(&0_u16.to_be_bytes());
        lookup_list.extend_from_slice(&1_u16.to_be_bytes());
        lookup_list.extend_from_slice(&8_u16.to_be_bytes());
        lookup_list.extend_from_slice(&contextual);

        let script_offset = 10_u16;
        let feature_offset = script_offset + script_list.len() as u16;
        let lookup_offset = feature_offset + feature_list.len() as u16;
        let mut table = vec![0; 10];
        table[0..2].copy_from_slice(&1_u16.to_be_bytes());
        table[4..6].copy_from_slice(&script_offset.to_be_bytes());
        table[6..8].copy_from_slice(&feature_offset.to_be_bytes());
        table[8..10].copy_from_slice(&lookup_offset.to_be_bytes());
        table.extend_from_slice(&script_list);
        table.extend_from_slice(&feature_list);
        table.extend_from_slice(&lookup_list);
        table
    }

    fn arabic_ligature_substitution() -> Vec<u8> {
        let mut subtable = vec![0; 26];
        subtable[0..2].copy_from_slice(&1_u16.to_be_bytes());
        subtable[2..4].copy_from_slice(&20_u16.to_be_bytes());
        subtable[4..6].copy_from_slice(&1_u16.to_be_bytes());
        subtable[6..8].copy_from_slice(&8_u16.to_be_bytes());
        subtable[8..10].copy_from_slice(&1_u16.to_be_bytes());
        subtable[10..12].copy_from_slice(&4_u16.to_be_bytes());
        subtable[12..14].copy_from_slice(&9_u16.to_be_bytes());
        subtable[14..16].copy_from_slice(&2_u16.to_be_bytes());
        subtable[16..18].copy_from_slice(&4_u16.to_be_bytes());
        subtable[20..22].copy_from_slice(&1_u16.to_be_bytes());
        subtable[22..24].copy_from_slice(&1_u16.to_be_bytes());
        subtable[24..26].copy_from_slice(&3_u16.to_be_bytes());
        subtable
    }

    fn arabic_contextual_substitution() -> Vec<u8> {
        let mut subtable = vec![0; 32];
        subtable[0..2].copy_from_slice(&1_u16.to_be_bytes());
        subtable[2..4].copy_from_slice(&26_u16.to_be_bytes());
        subtable[4..6].copy_from_slice(&1_u16.to_be_bytes());
        subtable[6..8].copy_from_slice(&8_u16.to_be_bytes());

        subtable[8..10].copy_from_slice(&1_u16.to_be_bytes());
        subtable[10..12].copy_from_slice(&4_u16.to_be_bytes());
        subtable[12..14].copy_from_slice(&0_u16.to_be_bytes());
        subtable[14..16].copy_from_slice(&2_u16.to_be_bytes());
        subtable[16..18].copy_from_slice(&6_u16.to_be_bytes());
        subtable[18..20].copy_from_slice(&0_u16.to_be_bytes());
        subtable[20..22].copy_from_slice(&1_u16.to_be_bytes());
        subtable[22..24].copy_from_slice(&0_u16.to_be_bytes());
        subtable[24..26].copy_from_slice(&5_u16.to_be_bytes());

        subtable[26..28].copy_from_slice(&1_u16.to_be_bytes());
        subtable[28..30].copy_from_slice(&1_u16.to_be_bytes());
        subtable[30..32].copy_from_slice(&2_u16.to_be_bytes());
        subtable
    }

    fn arabic_gpos() -> Vec<u8> {
        let mut script_list = vec![0; 26];
        script_list[0..2].copy_from_slice(&1_u16.to_be_bytes());
        script_list[2..6].copy_from_slice(b"arab");
        script_list[6..8].copy_from_slice(&8_u16.to_be_bytes());
        script_list[8..10].copy_from_slice(&6_u16.to_be_bytes());
        script_list[16..18].copy_from_slice(&u16::MAX.to_be_bytes());
        script_list[18..20].copy_from_slice(&3_u16.to_be_bytes());
        script_list[20..22].copy_from_slice(&0_u16.to_be_bytes());
        script_list[22..24].copy_from_slice(&1_u16.to_be_bytes());
        script_list[24..26].copy_from_slice(&2_u16.to_be_bytes());

        let mut feature_list = vec![0; 38];
        feature_list[0..2].copy_from_slice(&3_u16.to_be_bytes());
        feature_list[2..6].copy_from_slice(b"mark");
        feature_list[6..8].copy_from_slice(&20_u16.to_be_bytes());
        feature_list[8..12].copy_from_slice(b"mkmk");
        feature_list[12..14].copy_from_slice(&26_u16.to_be_bytes());
        feature_list[14..18].copy_from_slice(b"curs");
        feature_list[18..20].copy_from_slice(&32_u16.to_be_bytes());
        feature_list[22..24].copy_from_slice(&1_u16.to_be_bytes());
        feature_list[24..26].copy_from_slice(&0_u16.to_be_bytes());
        feature_list[28..30].copy_from_slice(&1_u16.to_be_bytes());
        feature_list[30..32].copy_from_slice(&1_u16.to_be_bytes());
        feature_list[34..36].copy_from_slice(&1_u16.to_be_bytes());
        feature_list[36..38].copy_from_slice(&2_u16.to_be_bytes());

        let lookup_tables = [
            gpos_lookup(4, mark_to_base_subtable()),
            gpos_lookup(5, mark_to_mark_subtable()),
            gpos_lookup(3, cursive_subtable()),
        ];
        let mut lookup_list = vec![0; 8];
        lookup_list[0..2].copy_from_slice(&3_u16.to_be_bytes());
        let mut lookup_offset = lookup_list.len();
        for (index, lookup) in lookup_tables.iter().enumerate() {
            lookup_list[2 + index * 2..4 + index * 2]
                .copy_from_slice(&(lookup_offset as u16).to_be_bytes());
            lookup_list.extend_from_slice(lookup);
            lookup_offset += lookup.len();
        }

        let script_offset = 10_u16;
        let feature_offset = script_offset + script_list.len() as u16;
        let lookup_offset = feature_offset + feature_list.len() as u16;
        let mut table = vec![0; 10];
        table[0..2].copy_from_slice(&1_u16.to_be_bytes());
        table[4..6].copy_from_slice(&script_offset.to_be_bytes());
        table[6..8].copy_from_slice(&feature_offset.to_be_bytes());
        table[8..10].copy_from_slice(&lookup_offset.to_be_bytes());
        table.extend_from_slice(&script_list);
        table.extend_from_slice(&feature_list);
        table.extend_from_slice(&lookup_list);
        table
    }

    fn gpos_lookup(lookup_type: u16, subtable: Vec<u8>) -> Vec<u8> {
        let mut lookup = vec![0; 8];
        lookup[0..2].copy_from_slice(&lookup_type.to_be_bytes());
        lookup[4..6].copy_from_slice(&1_u16.to_be_bytes());
        lookup[6..8].copy_from_slice(&8_u16.to_be_bytes());
        lookup.extend_from_slice(&subtable);
        lookup
    }

    fn mark_to_base_subtable() -> Vec<u8> {
        let mut subtable = vec![0; 46];
        subtable[0..2].copy_from_slice(&1_u16.to_be_bytes());
        subtable[2..4].copy_from_slice(&12_u16.to_be_bytes());
        subtable[4..6].copy_from_slice(&18_u16.to_be_bytes());
        subtable[6..8].copy_from_slice(&1_u16.to_be_bytes());
        subtable[8..10].copy_from_slice(&24_u16.to_be_bytes());
        subtable[10..12].copy_from_slice(&36_u16.to_be_bytes());

        subtable[12..14].copy_from_slice(&1_u16.to_be_bytes());
        subtable[14..16].copy_from_slice(&1_u16.to_be_bytes());
        subtable[16..18].copy_from_slice(&8_u16.to_be_bytes());
        subtable[18..20].copy_from_slice(&1_u16.to_be_bytes());
        subtable[20..22].copy_from_slice(&1_u16.to_be_bytes());
        subtable[22..24].copy_from_slice(&3_u16.to_be_bytes());

        subtable[24..26].copy_from_slice(&1_u16.to_be_bytes());
        subtable[26..28].copy_from_slice(&0_u16.to_be_bytes());
        subtable[28..30].copy_from_slice(&6_u16.to_be_bytes());
        subtable[30..32].copy_from_slice(&1_u16.to_be_bytes());
        subtable[32..34].copy_from_slice(&100_i16.to_be_bytes());

        subtable[34..36].copy_from_slice(&0_i16.to_be_bytes());
        subtable[36..38].copy_from_slice(&1_u16.to_be_bytes());
        subtable[38..40].copy_from_slice(&4_u16.to_be_bytes());
        subtable[40..42].copy_from_slice(&1_u16.to_be_bytes());
        subtable[42..44].copy_from_slice(&300_i16.to_be_bytes());
        subtable[44..46].copy_from_slice(&700_i16.to_be_bytes());
        subtable
    }

    fn mark_to_mark_subtable() -> Vec<u8> {
        let mut subtable = vec![0; 46];
        subtable[0..2].copy_from_slice(&1_u16.to_be_bytes());
        subtable[2..4].copy_from_slice(&12_u16.to_be_bytes());
        subtable[4..6].copy_from_slice(&18_u16.to_be_bytes());
        subtable[6..8].copy_from_slice(&1_u16.to_be_bytes());
        subtable[8..10].copy_from_slice(&24_u16.to_be_bytes());
        subtable[10..12].copy_from_slice(&36_u16.to_be_bytes());

        subtable[12..14].copy_from_slice(&1_u16.to_be_bytes());
        subtable[14..16].copy_from_slice(&1_u16.to_be_bytes());
        subtable[16..18].copy_from_slice(&8_u16.to_be_bytes());
        subtable[18..20].copy_from_slice(&1_u16.to_be_bytes());
        subtable[20..22].copy_from_slice(&1_u16.to_be_bytes());
        subtable[22..24].copy_from_slice(&8_u16.to_be_bytes());

        subtable[24..26].copy_from_slice(&1_u16.to_be_bytes());
        subtable[26..28].copy_from_slice(&0_u16.to_be_bytes());
        subtable[28..30].copy_from_slice(&6_u16.to_be_bytes());
        subtable[30..32].copy_from_slice(&1_u16.to_be_bytes());
        subtable[32..34].copy_from_slice(&50_i16.to_be_bytes());
        subtable[34..36].copy_from_slice(&0_i16.to_be_bytes());

        subtable[36..38].copy_from_slice(&1_u16.to_be_bytes());
        subtable[38..40].copy_from_slice(&4_u16.to_be_bytes());
        subtable[40..42].copy_from_slice(&1_u16.to_be_bytes());
        subtable[42..44].copy_from_slice(&100_i16.to_be_bytes());
        subtable[44..46].copy_from_slice(&300_i16.to_be_bytes());
        subtable
    }

    fn cursive_subtable() -> Vec<u8> {
        let mut subtable = vec![0; 34];
        subtable[0..2].copy_from_slice(&1_u16.to_be_bytes());
        subtable[2..4].copy_from_slice(&26_u16.to_be_bytes());
        subtable[4..6].copy_from_slice(&2_u16.to_be_bytes());
        subtable[6..8].copy_from_slice(&20_u16.to_be_bytes());
        subtable[8..10].copy_from_slice(&0_u16.to_be_bytes());
        subtable[10..12].copy_from_slice(&0_u16.to_be_bytes());
        subtable[12..14].copy_from_slice(&14_u16.to_be_bytes());

        subtable[14..16].copy_from_slice(&1_u16.to_be_bytes());
        subtable[16..18].copy_from_slice(&500_i16.to_be_bytes());
        subtable[18..20].copy_from_slice(&0_i16.to_be_bytes());
        subtable[20..22].copy_from_slice(&1_u16.to_be_bytes());
        subtable[22..24].copy_from_slice(&400_i16.to_be_bytes());
        subtable[24..26].copy_from_slice(&(-60_i16).to_be_bytes());

        subtable[26..28].copy_from_slice(&1_u16.to_be_bytes());
        subtable[28..30].copy_from_slice(&2_u16.to_be_bytes());
        subtable[30..32].copy_from_slice(&2_u16.to_be_bytes());
        subtable[32..34].copy_from_slice(&3_u16.to_be_bytes());
        subtable
    }

    fn single_substitution(mapping: &[(u16, u16)]) -> Vec<u8> {
        let coverage_offset = 6 + mapping.len() * 2;
        let mut subtable = vec![0; coverage_offset + 4 + mapping.len() * 2];
        subtable[0..2].copy_from_slice(&2_u16.to_be_bytes());
        subtable[2..4].copy_from_slice(&(coverage_offset as u16).to_be_bytes());
        subtable[4..6].copy_from_slice(&(mapping.len() as u16).to_be_bytes());
        for (index, (_, replacement)) in mapping.iter().enumerate() {
            let offset = 6 + index * 2;
            subtable[offset..offset + 2].copy_from_slice(&replacement.to_be_bytes());
        }
        subtable[coverage_offset..coverage_offset + 2].copy_from_slice(&1_u16.to_be_bytes());
        subtable[coverage_offset + 2..coverage_offset + 4]
            .copy_from_slice(&(mapping.len() as u16).to_be_bytes());
        for (index, (glyph, _)) in mapping.iter().enumerate() {
            let offset = coverage_offset + 4 + index * 2;
            subtable[offset..offset + 2].copy_from_slice(&glyph.to_be_bytes());
        }
        subtable
    }

    fn indic_reordering_font_for_test() -> Vec<u8> {
        let (head, mut hhea, mut maxp, _) = metrics_tables();
        hhea[34..36].copy_from_slice(&4_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&4_u16.to_be_bytes());

        let mut hmtx = vec![0; 16];
        for (glyph_id, advance) in [(0, 600_u16), (1, 600), (2, 0), (3, 0)] {
            let offset = glyph_id * 4;
            hmtx[offset..offset + 2].copy_from_slice(&advance.to_be_bytes());
        }

        let gsub = script_single_feature_gsub(*b"deva", *b"pres", &[(2, 3)]);
        sfnt_with_tables(&[
            (*b"GSUB", gsub.as_slice()),
            (*b"cmap", indic_cmap().as_slice()),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"maxp", maxp.as_slice()),
        ])
    }

    fn indic_context_font_for_test() -> Vec<u8> {
        let (head, mut hhea, mut maxp, _) = metrics_tables();
        hhea[34..36].copy_from_slice(&4_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&4_u16.to_be_bytes());

        let mut hmtx = vec![0; 16];
        for (glyph_id, advance) in [(0, 600_u16), (1, 600), (2, 0), (3, 600)] {
            let offset = glyph_id * 4;
            hmtx[offset..offset + 2].copy_from_slice(&advance.to_be_bytes());
        }

        let gsub = script_feature_gsub_with_lookups(
            *b"deva",
            *b"half",
            &[
                (5, contextual_substitution()),
                (1, single_substitution(&[(1, 3)])),
            ],
            &[0],
        );
        sfnt_with_tables(&[
            (*b"GSUB", gsub.as_slice()),
            (*b"cmap", indic_context_cmap().as_slice()),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"maxp", maxp.as_slice()),
        ])
    }

    fn indic_mark_font_for_test() -> Vec<u8> {
        let (head, mut hhea, mut maxp, _) = metrics_tables();
        hhea[34..36].copy_from_slice(&10_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&10_u16.to_be_bytes());

        let mut hmtx = vec![0; 40];
        for (glyph_id, advance) in [(0, 600_u16), (3, 600), (8, 0)] {
            let offset = glyph_id * 4;
            hmtx[offset..offset + 2].copy_from_slice(&advance.to_be_bytes());
        }

        let gpos = script_feature_gsub_with_lookups(
            *b"deva",
            *b"abvm",
            &[(4, mark_to_base_subtable())],
            &[0],
        );
        sfnt_with_tables(&[
            (*b"GPOS", gpos.as_slice()),
            (*b"cmap", indic_cmap_with_glyphs(3, 8).as_slice()),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"maxp", maxp.as_slice()),
        ])
    }

    fn indic_cmap() -> Vec<u8> {
        indic_cmap_with_glyphs(1, 2)
    }

    fn indic_cmap_with_glyphs(base_glyph: u32, matra_glyph: u32) -> Vec<u8> {
        let mut cmap = vec![0; 52];
        cmap[2..4].copy_from_slice(&1_u16.to_be_bytes());
        cmap[4..6].copy_from_slice(&3_u16.to_be_bytes());
        cmap[6..8].copy_from_slice(&10_u16.to_be_bytes());
        cmap[8..12].copy_from_slice(&12_u32.to_be_bytes());
        cmap[12..14].copy_from_slice(&12_u16.to_be_bytes());
        cmap[16..20].copy_from_slice(&40_u32.to_be_bytes());
        cmap[24..28].copy_from_slice(&2_u32.to_be_bytes());
        cmap[28..32].copy_from_slice(&0x0915_u32.to_be_bytes());
        cmap[32..36].copy_from_slice(&0x0915_u32.to_be_bytes());
        cmap[36..40].copy_from_slice(&base_glyph.to_be_bytes());
        cmap[40..44].copy_from_slice(&0x093f_u32.to_be_bytes());
        cmap[44..48].copy_from_slice(&0x093f_u32.to_be_bytes());
        cmap[48..52].copy_from_slice(&matra_glyph.to_be_bytes());
        cmap
    }

    fn indic_context_cmap() -> Vec<u8> {
        let mut cmap = indic_cmap();
        cmap[40..44].copy_from_slice(&0x094d_u32.to_be_bytes());
        cmap[44..48].copy_from_slice(&0x094d_u32.to_be_bytes());
        cmap[48..52].copy_from_slice(&2_u32.to_be_bytes());
        cmap
    }

    fn script_single_feature_gsub(
        script_tag: [u8; 4],
        feature_tag: [u8; 4],
        mapping: &[(u16, u16)],
    ) -> Vec<u8> {
        script_feature_gsub_with_lookups(
            script_tag,
            feature_tag,
            &[(1, single_substitution(mapping))],
            &[0],
        )
    }

    fn script_feature_gsub_with_lookups(
        script_tag: [u8; 4],
        feature_tag: [u8; 4],
        lookup_specs: &[(u16, Vec<u8>)],
        feature_lookup_indices: &[u16],
    ) -> Vec<u8> {
        let mut lookup_list = vec![0; 2 + lookup_specs.len() * 2];
        lookup_list[0..2].copy_from_slice(&(lookup_specs.len() as u16).to_be_bytes());
        let mut lookup_offset = lookup_list.len();
        for (index, (lookup_type, subtable)) in lookup_specs.iter().enumerate() {
            lookup_list[2 + index * 2..4 + index * 2]
                .copy_from_slice(&(lookup_offset as u16).to_be_bytes());
            lookup_list.extend_from_slice(&lookup_type.to_be_bytes());
            lookup_list.extend_from_slice(&0_u16.to_be_bytes());
            lookup_list.extend_from_slice(&1_u16.to_be_bytes());
            lookup_list.extend_from_slice(&8_u16.to_be_bytes());
            lookup_list.extend_from_slice(subtable);
            lookup_offset += 8 + subtable.len();
        }

        let mut language = vec![0; 8];
        language[0..2].copy_from_slice(&0_u16.to_be_bytes());
        language[2..4].copy_from_slice(&u16::MAX.to_be_bytes());
        language[4..6].copy_from_slice(&1_u16.to_be_bytes());
        language[6..8].copy_from_slice(&0_u16.to_be_bytes());

        let mut script = vec![0; 4];
        script[0..2].copy_from_slice(&4_u16.to_be_bytes());
        script.extend_from_slice(&language);

        let mut script_list = vec![0; 8];
        script_list[0..2].copy_from_slice(&1_u16.to_be_bytes());
        script_list[2..6].copy_from_slice(&script_tag);
        script_list[6..8].copy_from_slice(&8_u16.to_be_bytes());
        script_list.extend_from_slice(&script);

        let feature_table_len = 4 + feature_lookup_indices.len() * 2;
        let mut feature_list = vec![0; 8 + feature_table_len];
        feature_list[0..2].copy_from_slice(&1_u16.to_be_bytes());
        feature_list[2..6].copy_from_slice(&feature_tag);
        feature_list[6..8].copy_from_slice(&8_u16.to_be_bytes());
        feature_list[10..12].copy_from_slice(&(feature_lookup_indices.len() as u16).to_be_bytes());
        for (index, lookup_index) in feature_lookup_indices.iter().enumerate() {
            let offset = 12 + index * 2;
            feature_list[offset..offset + 2].copy_from_slice(&lookup_index.to_be_bytes());
        }

        let script_offset = 10_u16;
        let feature_offset = script_offset + script_list.len() as u16;
        let lookup_offset = feature_offset + feature_list.len() as u16;
        let mut table = vec![0; 10];
        table[0..2].copy_from_slice(&1_u16.to_be_bytes());
        table[4..6].copy_from_slice(&script_offset.to_be_bytes());
        table[6..8].copy_from_slice(&feature_offset.to_be_bytes());
        table[8..10].copy_from_slice(&lookup_offset.to_be_bytes());
        table.extend_from_slice(&script_list);
        table.extend_from_slice(&feature_list);
        table.extend_from_slice(&lookup_list);
        table
    }

    fn contextual_substitution() -> Vec<u8> {
        let mut subtable = vec![0; 28];
        subtable[0..2].copy_from_slice(&1_u16.to_be_bytes());
        subtable[2..4].copy_from_slice(&22_u16.to_be_bytes());
        subtable[4..6].copy_from_slice(&1_u16.to_be_bytes());
        subtable[6..8].copy_from_slice(&8_u16.to_be_bytes());
        subtable[8..10].copy_from_slice(&1_u16.to_be_bytes());
        subtable[10..12].copy_from_slice(&4_u16.to_be_bytes());
        subtable[12..14].copy_from_slice(&2_u16.to_be_bytes());
        subtable[14..16].copy_from_slice(&1_u16.to_be_bytes());
        subtable[16..18].copy_from_slice(&2_u16.to_be_bytes());
        subtable[18..20].copy_from_slice(&0_u16.to_be_bytes());
        subtable[20..22].copy_from_slice(&1_u16.to_be_bytes());
        subtable[22..24].copy_from_slice(&1_u16.to_be_bytes());
        subtable[24..26].copy_from_slice(&1_u16.to_be_bytes());
        subtable[26..28].copy_from_slice(&1_u16.to_be_bytes());
        subtable
    }

    fn cjk_layout_font_for_test() -> Vec<u8> {
        let (head, mut hhea, mut maxp, _) = metrics_tables();
        hhea[34..36].copy_from_slice(&6_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&6_u16.to_be_bytes());
        let mut hmtx = vec![0; 24];
        for glyph_id in 0..6 {
            let offset = glyph_id * 4;
            hmtx[offset..offset + 2].copy_from_slice(&700_u16.to_be_bytes());
        }
        let mut cmap = format12_cmap();
        cmap[36..40].copy_from_slice(&1_u32.to_be_bytes());

        let gsub = cjk_layout_gsub();
        sfnt_with_tables(&[
            (*b"GSUB", gsub.as_slice()),
            (*b"cmap", cmap.as_slice()),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"maxp", maxp.as_slice()),
        ])
    }

    fn vertical_cjk_layout_font_for_test(include_vorg: bool) -> Vec<u8> {
        vertical_cjk_layout_font_for_test_with_gpos(include_vorg, false, false)
    }

    fn vertical_cjk_layout_font_with_gpos() -> Vec<u8> {
        vertical_cjk_layout_font_for_test_with_gpos(true, true, false)
    }

    fn vertical_cjk_layout_font_with_vpal() -> Vec<u8> {
        vertical_cjk_layout_font_for_test_with_gpos(true, false, true)
    }

    fn vertical_cjk_layout_font_for_test_with_gpos(
        include_vorg: bool,
        include_vertical_gpos: bool,
        include_vpal: bool,
    ) -> Vec<u8> {
        let (head, mut hhea, mut maxp, _) = metrics_tables();
        hhea[34..36].copy_from_slice(&6_u16.to_be_bytes());
        maxp[4..6].copy_from_slice(&6_u16.to_be_bytes());
        let mut hmtx = vec![0; 24];
        for glyph_id in 0..6 {
            let offset = glyph_id * 4;
            hmtx[offset..offset + 2].copy_from_slice(&700_u16.to_be_bytes());
        }
        let mut cmap = format12_cmap();
        cmap[36..40].copy_from_slice(&1_u32.to_be_bytes());

        let mut vhea = vec![0; 36];
        vhea[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        vhea[4..6].copy_from_slice(&900_i16.to_be_bytes());
        vhea[6..8].copy_from_slice(&(-200_i16).to_be_bytes());
        vhea[8..10].copy_from_slice(&100_i16.to_be_bytes());
        vhea[34..36].copy_from_slice(&(if include_vpal { 6_u16 } else { 3_u16 }).to_be_bytes());

        let vertical_metrics = if include_vpal {
            vec![
                (1000_u16, 80_i16),
                (900, 100),
                (800, 90),
                (800, 70),
                (800, 60),
                (800, 50),
            ]
        } else {
            vec![(1000_u16, 80_i16), (900, 100), (800, 90)]
        };
        let mut vmtx = vec![0; vertical_metrics.len() * 4];
        for (index, (advance, bearing)) in vertical_metrics.into_iter().enumerate() {
            let offset = index * 4;
            vmtx[offset..offset + 2].copy_from_slice(&advance.to_be_bytes());
            vmtx[offset + 2..offset + 4].copy_from_slice(&bearing.to_be_bytes());
        }
        if !include_vpal {
            for bearing in [70_i16, 60, 50] {
                vmtx.extend_from_slice(&bearing.to_be_bytes());
            }
        }
        if !include_vorg {
            vmtx.truncate(4);
        }

        let vorg = {
            let mut table = vec![0; 12];
            table[0..2].copy_from_slice(&1_u16.to_be_bytes());
            table[4..6].copy_from_slice(&850_i16.to_be_bytes());
            table[6..8].copy_from_slice(&1_u16.to_be_bytes());
            table[8..10].copy_from_slice(&4_u16.to_be_bytes());
            table[10..12].copy_from_slice(&880_i16.to_be_bytes());
            table
        };
        let gsub = cjk_layout_gsub_with_vpal(include_vpal);
        let gpos = vertical_pair_gpos();
        let mut tables = vec![
            (*b"GSUB", gsub.as_slice()),
            (*b"cmap", cmap.as_slice()),
            (*b"head", head.as_slice()),
            (*b"hhea", hhea.as_slice()),
            (*b"hmtx", hmtx.as_slice()),
            (*b"maxp", maxp.as_slice()),
            (*b"vhea", vhea.as_slice()),
            (*b"vmtx", vmtx.as_slice()),
        ];
        if include_vorg {
            tables.push((*b"VORG", vorg.as_slice()));
        }
        if include_vertical_gpos {
            tables.push((*b"GPOS", gpos.as_slice()));
        }
        sfnt_with_tables(&tables)
    }

    fn vertical_pair_gpos() -> Vec<u8> {
        let mut script = vec![0; 4];
        script[0..2].copy_from_slice(&4_u16.to_be_bytes());
        script.extend_from_slice(&cjk_lang_sys(&[0]));

        let mut script_list = vec![0; 8];
        script_list[0..2].copy_from_slice(&1_u16.to_be_bytes());
        script_list[2..6].copy_from_slice(b"hani");
        script_list[6..8].copy_from_slice(&8_u16.to_be_bytes());
        script_list.extend_from_slice(&script);

        let mut feature_list = vec![0; 8];
        feature_list[0..2].copy_from_slice(&1_u16.to_be_bytes());
        feature_list[2..6].copy_from_slice(b"vkrn");
        feature_list[6..8].copy_from_slice(&8_u16.to_be_bytes());
        feature_list.extend_from_slice(&[0, 0, 0, 1, 0, 0]);

        let pair_subtable = {
            let coverage_offset = 18;
            let mut subtable = vec![0; 24];
            subtable[0..2].copy_from_slice(&1_u16.to_be_bytes());
            subtable[2..4].copy_from_slice(&coverage_offset_u16(coverage_offset));
            subtable[4..6].copy_from_slice(&0x0008_u16.to_be_bytes());
            subtable[8..10].copy_from_slice(&1_u16.to_be_bytes());
            subtable[10..12].copy_from_slice(&12_u16.to_be_bytes());
            subtable[12..14].copy_from_slice(&1_u16.to_be_bytes());
            subtable[14..16].copy_from_slice(&4_u16.to_be_bytes());
            subtable[16..18].copy_from_slice(&(-20_i16).to_be_bytes());
            subtable[coverage_offset..coverage_offset + 2].copy_from_slice(&1_u16.to_be_bytes());
            subtable[coverage_offset + 2..coverage_offset + 4]
                .copy_from_slice(&1_u16.to_be_bytes());
            subtable[coverage_offset + 4..coverage_offset + 6]
                .copy_from_slice(&4_u16.to_be_bytes());
            subtable
        };
        let lookup = gpos_lookup(2, pair_subtable);
        let mut lookup_list = vec![0; 4];
        lookup_list[0..2].copy_from_slice(&1_u16.to_be_bytes());
        lookup_list[2..4].copy_from_slice(&4_u16.to_be_bytes());
        lookup_list.extend_from_slice(&lookup);

        let script_offset = 10_u16;
        let feature_offset = script_offset + script_list.len() as u16;
        let lookup_offset = feature_offset + feature_list.len() as u16;
        let mut table = vec![0; 10];
        table[0..2].copy_from_slice(&1_u16.to_be_bytes());
        table[4..6].copy_from_slice(&script_offset.to_be_bytes());
        table[6..8].copy_from_slice(&feature_offset.to_be_bytes());
        table[8..10].copy_from_slice(&lookup_offset.to_be_bytes());
        table.extend_from_slice(&script_list);
        table.extend_from_slice(&feature_list);
        table.extend_from_slice(&lookup_list);
        table
    }

    fn coverage_offset_u16(offset: usize) -> [u8; 2] {
        u16::try_from(offset)
            .expect("fixture offset fits")
            .to_be_bytes()
    }

    fn cjk_layout_gsub() -> Vec<u8> {
        cjk_layout_gsub_with_vpal(false)
    }

    fn cjk_layout_gsub_with_vpal(include_vpal: bool) -> Vec<u8> {
        let mut feature_lookups = vec![
            (*b"locl", 0_u16),
            (*b"locl", 1_u16),
            (*b"locl", 2_u16),
            (*b"vrt2", 3_u16),
            (*b"vert", 4_u16),
        ];
        if include_vpal {
            feature_lookups.push((*b"vpal", 5_u16));
        }
        let mut feature_list = vec![0; 2 + feature_lookups.len() * 6];
        feature_list[0..2].copy_from_slice(&(feature_lookups.len() as u16).to_be_bytes());
        let mut feature_offset = feature_list.len();
        for (index, (tag, lookup_index)) in feature_lookups.into_iter().enumerate() {
            let record = 2 + index * 6;
            feature_list[record..record + 4].copy_from_slice(&tag);
            feature_list[record + 4..record + 6]
                .copy_from_slice(&(feature_offset as u16).to_be_bytes());
            feature_list.extend_from_slice(&[0, 0, 0, 1]);
            feature_list.extend_from_slice(&lookup_index.to_be_bytes());
            feature_offset += 6;
        }

        let default_language = if include_vpal {
            cjk_lang_sys(&[0, 3, 4, 5])
        } else {
            cjk_lang_sys(&[0, 3, 4])
        };
        let chinese_language = cjk_lang_sys(&[1]);
        let japanese_language = cjk_lang_sys(&[2]);
        let mut script = vec![0; 4 + 2 * 6];
        let default_offset = script.len();
        script[0..2].copy_from_slice(&(default_offset as u16).to_be_bytes());
        script[2..4].copy_from_slice(&2_u16.to_be_bytes());
        script[4..8].copy_from_slice(b"ZHS ");
        script[8..10].copy_from_slice(
            &(u16::try_from(default_offset + default_language.len()).expect("fixture fits"))
                .to_be_bytes(),
        );
        script[10..14].copy_from_slice(b"JAN ");
        script[14..16].copy_from_slice(
            &(u16::try_from(default_offset + default_language.len() + chinese_language.len())
                .expect("fixture fits"))
            .to_be_bytes(),
        );
        script.extend_from_slice(&default_language);
        script.extend_from_slice(&chinese_language);
        script.extend_from_slice(&japanese_language);

        let mut script_list = vec![0; 8];
        script_list[0..2].copy_from_slice(&1_u16.to_be_bytes());
        script_list[2..6].copy_from_slice(b"hani");
        script_list[6..8].copy_from_slice(&8_u16.to_be_bytes());
        script_list.extend_from_slice(&script);

        let mut lookups = vec![
            single_substitution(&[(1, 1)]),
            single_substitution(&[(1, 2)]),
            single_substitution(&[(1, 3)]),
            single_substitution(&[(1, 4)]),
            single_substitution(&[(1, 5)]),
        ];
        if include_vpal {
            lookups.push(single_substitution(&[(4, 5)]));
        }
        let mut lookup_list = vec![0; 2 + lookups.len() * 2];
        lookup_list[0..2].copy_from_slice(&(lookups.len() as u16).to_be_bytes());
        let mut lookup_offset = lookup_list.len();
        for (index, subtable) in lookups.into_iter().enumerate() {
            lookup_list[2 + index * 2..4 + index * 2]
                .copy_from_slice(&(lookup_offset as u16).to_be_bytes());
            lookup_list.extend_from_slice(&[0, 1, 0, 0, 0, 1, 0, 8]);
            lookup_list.extend_from_slice(&subtable);
            lookup_offset += 8 + subtable.len();
        }

        let script_offset = 10_u16;
        let feature_offset = script_offset + script_list.len() as u16;
        let lookup_offset = feature_offset + feature_list.len() as u16;
        let mut table = vec![0; 10];
        table[0..2].copy_from_slice(&1_u16.to_be_bytes());
        table[4..6].copy_from_slice(&script_offset.to_be_bytes());
        table[6..8].copy_from_slice(&feature_offset.to_be_bytes());
        table[8..10].copy_from_slice(&lookup_offset.to_be_bytes());
        table.extend_from_slice(&script_list);
        table.extend_from_slice(&feature_list);
        table.extend_from_slice(&lookup_list);
        table
    }

    fn cjk_lang_sys(feature_indices: &[u16]) -> Vec<u8> {
        let mut bytes = vec![0; 6 + feature_indices.len() * 2];
        bytes[2..4].copy_from_slice(&u16::MAX.to_be_bytes());
        bytes[4..6].copy_from_slice(&(feature_indices.len() as u16).to_be_bytes());
        for (index, feature_index) in feature_indices.iter().copied().enumerate() {
            bytes[6 + index * 2..8 + index * 2].copy_from_slice(&feature_index.to_be_bytes());
        }
        bytes
    }
}
