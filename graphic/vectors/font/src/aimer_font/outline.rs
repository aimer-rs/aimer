use super::{FontMetrics, Reader, SfntError, SfntFace, Tag, checked_add, checked_mul};

const MAX_POINTS: usize = 1 << 20;
const MAX_COMPOSITE_COMPONENTS: usize = 1024;
const MAX_COMPOSITE_DEPTH: usize = 32;

const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;
const SCALED_COMPONENT_OFFSET: u16 = 0x0800;
const UNSCALED_COMPONENT_OFFSET: u16 = 0x1000;

/// A point from a TrueType outline in font units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OutlinePoint {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) on_curve: bool,
}

/// The contours extracted from one `glyf` glyph.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GlyphOutline {
    pub(crate) bounds: [i16; 4],
    pub(crate) points: Vec<OutlinePoint>,
    pub(crate) contours: Vec<ContourRange>,
    /// Point ranges for the top-level components of a composite glyph.
    ///
    /// Composite variation deltas address component origins rather than the
    /// flattened outline points, so the ranges are retained after parsing to
    /// let the variation layer translate each component as one unit.
    pub(crate) component_ranges: Vec<ContourRange>,
    pub(crate) is_composite: bool,
}

/// A half-open range into [`GlyphOutline::points`] for one contour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContourRange {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl<'a> SfntFace<'a> {
    /// Extracts a TrueType `glyf` outline through the face's `loca` table.
    ///
    /// `Ok(None)` means the face has no TrueType outline tables. CFF/CFF2
    /// faces are exposed by the sibling [`SfntFace::cff_outline`] backend so
    /// their cubic commands are not lossy-converted into quadratic points.
    /// Empty `glyf` entries return an outline with no contours.
    pub(crate) fn outline(&self, glyph_id: u16) -> Result<Option<GlyphOutline>, SfntError> {
        let Some(glyf) = self.table(*b"glyf") else {
            return Ok(None);
        };
        let Some(loca) = self.table(*b"loca") else {
            return Err(malformed_loca());
        };
        let metrics = outline_metrics(self)?;
        self.outline_with_tables(glyf, loca, metrics, glyph_id, None)
    }

    /// Extracts a TrueType outline while reusing already-validated metrics.
    pub(crate) fn outline_with_metrics(
        &self,
        glyph_id: u16,
        metrics: FontMetrics,
    ) -> Result<Option<GlyphOutline>, SfntError> {
        let Some(glyf) = self.table(*b"glyf") else {
            return Ok(None);
        };
        let Some(loca) = self.table(*b"loca") else {
            return Err(malformed_loca());
        };
        self.outline_with_tables(glyf, loca, metrics, glyph_id, None)
    }

    /// Extracts a TrueType outline and applies the requested `wght` instance
    /// when the face carries OpenType `fvar`/`gvar` data. Static faces and
    /// faces without a weight axis use their ordinary default outline.
    pub(crate) fn outline_with_metrics_at_weight(
        &self,
        glyph_id: u16,
        metrics: FontMetrics,
        weight: u16,
    ) -> Result<Option<GlyphOutline>, SfntError> {
        let (coordinates, coordinate_count) = self.coordinates_for_weight_instance(weight);
        self.outline_with_metrics_at_coordinates(
            glyph_id,
            metrics,
            &coordinates[..coordinate_count],
        )
    }

    /// Extracts a TrueType outline and applies a complete normalized variation
    /// instance to the glyph and all of its composite components.
    pub(crate) fn outline_with_metrics_at_coordinates(
        &self,
        glyph_id: u16,
        metrics: FontMetrics,
        coordinates: &[f32],
    ) -> Result<Option<GlyphOutline>, SfntError> {
        let Some(glyf) = self.table(*b"glyf") else {
            return Ok(None);
        };
        let Some(loca) = self.table(*b"loca") else {
            return Err(malformed_loca());
        };
        let variation = (!coordinates
            .iter()
            .all(|coordinate| coordinate.abs() < f32::EPSILON))
        .then_some(coordinates);
        self.outline_with_tables(glyf, loca, metrics, glyph_id, variation)
    }

    fn outline_with_tables(
        &self,
        glyf: &[u8],
        loca: &[u8],
        metrics: FontMetrics,
        glyph_id: u16,
        coordinates: Option<&[f32]>,
    ) -> Result<Option<GlyphOutline>, SfntError> {
        if glyph_id >= metrics.num_glyphs {
            return Ok(None);
        }

        // `loca` is shared by every glyph in the face. Parse its entries once
        // on the first outline request, then keep the hot glyph path to two
        // bounds-checked slice reads. The cached result also memoizes a
        // malformed table, so repeated first-use failures do not redo work.
        let offsets = self
            .loca_cache
            .get_or_init(|| parse_loca(loca, metrics))
            .as_ref()
            .map_err(|error| *error)?;
        let mut active = Vec::with_capacity(MAX_COMPOSITE_DEPTH.min(4));
        parse_glyph(
            self,
            glyf,
            offsets,
            metrics,
            glyph_id,
            0,
            &mut active,
            coordinates,
        )
        .map(Some)
    }
}

fn outline_metrics(face: &SfntFace<'_>) -> Result<FontMetrics, SfntError> {
    let head_tag = Tag::from_bytes(*b"head");
    let head = face
        .table(*b"head")
        .ok_or(SfntError::MissingTable(head_tag))?;
    if head.len() < 54 {
        return Err(SfntError::MalformedTable(head_tag));
    }
    let head_reader = Reader::new(head);
    let units_per_em = head_reader.u16(18)?;
    let index_to_loc_format = head_reader.i16(50)?;
    if units_per_em == 0 || !(index_to_loc_format == 0 || index_to_loc_format == 1) {
        return Err(SfntError::MalformedTable(head_tag));
    }

    let maxp_tag = Tag::from_bytes(*b"maxp");
    let maxp = face
        .table(*b"maxp")
        .ok_or(SfntError::MissingTable(maxp_tag))?;
    if maxp.len() < 6 {
        return Err(SfntError::MalformedTable(maxp_tag));
    }
    let maxp_reader = Reader::new(maxp);
    let version = maxp_reader.u32(0)?;
    if version != 0x0000_5000 && version != 0x0001_0000 {
        return Err(SfntError::MalformedTable(maxp_tag));
    }
    let num_glyphs = maxp_reader.u16(4)?;
    if num_glyphs == 0 {
        return Err(SfntError::MalformedTable(maxp_tag));
    }

    Ok(FontMetrics {
        units_per_em,
        ascender: 0,
        descender: 0,
        line_gap: 0,
        cap_height: 0,
        x_min: 0,
        y_min: 0,
        x_max: 0,
        y_max: 0,
        num_glyphs,
        number_of_h_metrics: 0,
        index_to_loc_format,
    })
}

fn parse_glyph(
    face: &SfntFace<'_>,
    glyf: &[u8],
    offsets: &[usize],
    metrics: FontMetrics,
    glyph_id: u16,
    depth: usize,
    active: &mut Vec<u16>,
    coordinates: Option<&[f32]>,
) -> Result<GlyphOutline, SfntError> {
    if depth >= MAX_COMPOSITE_DEPTH {
        return Err(SfntError::OutlineRecursionLimit);
    }
    if active.contains(&glyph_id) {
        return Err(SfntError::CompositeCycle(glyph_id));
    }
    active.push(glyph_id);
    let outline = parse_glyph_inner(
        face,
        glyf,
        offsets,
        metrics,
        glyph_id,
        depth,
        active,
        coordinates,
    );
    let result = outline.and_then(|mut outline| {
        if let Some(coordinates) = coordinates {
            let bounds_updated = super::variation::apply_gvar_at_coordinates(
                face,
                glyph_id,
                coordinates,
                &mut outline,
            )?;
            if outline.is_composite && !bounds_updated {
                // A parent may have no gvar data even when a child varies.
                super::variation::update_bounds(&mut outline);
            }
        }
        Ok(outline)
    });
    active.pop();
    result
}

fn parse_glyph_inner(
    face: &SfntFace<'_>,
    glyf: &[u8],
    offsets: &[usize],
    metrics: FontMetrics,
    glyph_id: u16,
    depth: usize,
    active: &mut Vec<u16>,
    coordinates: Option<&[f32]>,
) -> Result<GlyphOutline, SfntError> {
    let Some((start, end)) = glyph_range(offsets, glyf.len(), metrics, glyph_id)? else {
        return Err(malformed_glyf());
    };
    if start == end {
        return Ok(GlyphOutline {
            bounds: [0; 4],
            points: Vec::new(),
            contours: Vec::new(),
            component_ranges: Vec::new(),
            is_composite: false,
        });
    }
    let glyph = glyf.get(start..end).ok_or_else(malformed_glyf)?;
    if glyph.len() < 10 {
        return Err(malformed_glyf());
    }
    let reader = Reader::new(glyph);
    let contour_count = reader.i16(0)?;
    let bounds = [
        reader.i16(2)?,
        reader.i16(4)?,
        reader.i16(6)?,
        reader.i16(8)?,
    ];
    if contour_count >= 0 {
        return parse_simple_glyph(glyph, contour_count as usize, bounds);
    }
    parse_composite_glyph(
        face,
        glyf,
        offsets,
        metrics,
        glyph,
        bounds,
        depth,
        active,
        coordinates,
    )
}

fn glyph_range(
    offsets: &[usize],
    glyf_length: usize,
    metrics: FontMetrics,
    glyph_id: u16,
) -> Result<Option<(usize, usize)>, SfntError> {
    let entry_count = usize::from(metrics.num_glyphs)
        .checked_add(1)
        .ok_or(SfntError::ArithmeticOverflow)?;
    if offsets.len() < entry_count {
        return Err(malformed_loca());
    }

    let start = offsets
        .get(usize::from(glyph_id))
        .copied()
        .ok_or_else(malformed_loca)?;
    let end = offsets
        .get(usize::from(glyph_id) + 1)
        .copied()
        .ok_or_else(malformed_loca)?;
    if start > end || end > glyf_length {
        return Err(malformed_loca());
    }
    Ok(Some((start, end)))
}

fn parse_loca(loca: &[u8], metrics: FontMetrics) -> Result<Vec<usize>, SfntError> {
    let entry_count = usize::from(metrics.num_glyphs)
        .checked_add(1)
        .ok_or(SfntError::ArithmeticOverflow)?;
    let entry_size = match metrics.index_to_loc_format {
        0 => 2,
        1 => 4,
        _ => return Err(malformed_loca()),
    };
    let required_size = checked_mul(entry_count, entry_size)?;
    if loca.len() < required_size {
        return Err(malformed_loca());
    }

    let reader = Reader::new(loca);
    let mut offsets = Vec::with_capacity(entry_count);
    for index in 0..entry_count {
        offsets.push(loca_offset(&reader, index, metrics.index_to_loc_format)?);
    }
    Ok(offsets)
}

fn loca_offset(
    reader: &Reader<'_>,
    index: usize,
    index_to_loc_format: i16,
) -> Result<usize, SfntError> {
    match index_to_loc_format {
        0 => checked_mul(usize::from(reader.u16(checked_mul(index, 2)?)?), 2),
        1 => usize::try_from(reader.u32(checked_mul(index, 4)?)?)
            .map_err(|_| SfntError::ArithmeticOverflow),
        _ => Err(malformed_loca()),
    }
}

fn parse_simple_glyph(
    glyph: &[u8],
    contour_count: usize,
    bounds: [i16; 4],
) -> Result<GlyphOutline, SfntError> {
    if contour_count == 0 {
        return Ok(GlyphOutline {
            bounds,
            points: Vec::new(),
            contours: Vec::new(),
            component_ranges: Vec::new(),
            is_composite: false,
        });
    }
    let reader = Reader::new(glyph);
    let end_points_offset = 10;
    let end_points_size = checked_mul(contour_count, 2)?;
    let instruction_length_offset = checked_add(end_points_offset, end_points_size)?;
    let instruction_length = usize::from(reader.u16(instruction_length_offset)?);
    let flags_offset = checked_add(instruction_length_offset, 2)?;
    let coordinates_offset = checked_add(flags_offset, instruction_length)?;
    reader.range(0, coordinates_offset)?;

    let mut end_points = Vec::with_capacity(contour_count);
    let mut previous = None;
    for index in 0..contour_count {
        let point =
            usize::from(reader.u16(checked_add(end_points_offset, checked_mul(index, 2)?)?)?);
        if previous.is_some_and(|value| point <= value) {
            return Err(malformed_glyf());
        }
        end_points.push(point);
        previous = Some(point);
    }
    let point_count = end_points.last().copied().unwrap_or(0) + 1;
    if point_count > MAX_POINTS {
        return Err(malformed_glyf());
    }

    let mut cursor = coordinates_offset;
    let mut flags = Vec::with_capacity(point_count);
    while flags.len() < point_count {
        let flag = reader.u8(cursor)?;
        cursor = checked_add(cursor, 1)?;
        let repeat = if flag & 0x08 != 0 {
            let repeat = reader.u8(cursor)?;
            cursor = checked_add(cursor, 1)?;
            usize::from(repeat)
        } else {
            0
        };
        let new_length = checked_add(flags.len(), checked_add(repeat, 1)?)?;
        if new_length > point_count {
            return Err(malformed_glyf());
        }
        flags.extend(std::iter::repeat_n(flag, repeat + 1));
    }

    let mut points = vec![
        OutlinePoint {
            x: 0.0,
            y: 0.0,
            on_curve: false,
        };
        point_count
    ];
    decode_axis_into(&reader, &mut cursor, &flags, 0x02, 0x10, &mut points, true)?;
    decode_axis_into(&reader, &mut cursor, &flags, 0x04, 0x20, &mut points, false)?;
    for (point, flag) in points.iter_mut().zip(flags) {
        point.on_curve = flag & 0x01 != 0;
    }

    let mut contours = Vec::with_capacity(contour_count);
    let mut start = 0;
    for end in end_points {
        if end < start || end >= points.len() {
            return Err(malformed_glyf());
        }
        contours.push(ContourRange {
            start,
            end: end + 1,
        });
        start = end + 1;
    }
    Ok(GlyphOutline {
        bounds,
        points,
        contours,
        component_ranges: Vec::new(),
        is_composite: false,
    })
}

fn decode_axis_into(
    reader: &Reader<'_>,
    cursor: &mut usize,
    flags: &[u8],
    short_bit: u8,
    same_bit: u8,
    points: &mut [OutlinePoint],
    x_axis: bool,
) -> Result<(), SfntError> {
    let mut coordinate = 0_i32;
    for (&flag, point) in flags.iter().zip(points.iter_mut()) {
        let delta = if flag & short_bit != 0 {
            let value = i32::from(reader.u8(*cursor)?);
            *cursor = checked_add(*cursor, 1)?;
            if flag & same_bit != 0 { value } else { -value }
        } else if flag & same_bit == 0 {
            let value = i32::from(reader.i16(*cursor)?);
            *cursor = checked_add(*cursor, 2)?;
            value
        } else {
            0
        };
        coordinate = coordinate.checked_add(delta).ok_or_else(malformed_glyf)?;
        if coordinate < i32::from(i16::MIN) || coordinate > i32::from(i16::MAX) {
            return Err(malformed_glyf());
        }
        if x_axis {
            point.x = coordinate as f32;
        } else {
            point.y = coordinate as f32;
        }
    }
    Ok(())
}

fn parse_composite_glyph(
    face: &SfntFace<'_>,
    glyf: &[u8],
    offsets: &[usize],
    metrics: FontMetrics,
    glyph: &[u8],
    bounds: [i16; 4],
    depth: usize,
    active: &mut Vec<u16>,
    coordinates: Option<&[f32]>,
) -> Result<GlyphOutline, SfntError> {
    let reader = Reader::new(glyph);
    let mut cursor = 10;
    let mut points = Vec::new();
    let mut contours = Vec::new();
    let mut component_ranges = Vec::new();
    let mut point_count = 0_usize;
    let mut components = 0;
    let mut has_more = true;
    let mut has_instructions = false;

    while has_more {
        components += 1;
        if components > MAX_COMPOSITE_COMPONENTS {
            return Err(SfntError::OutlineRecursionLimit);
        }
        let flags = reader.u16(cursor)?;
        cursor = checked_add(cursor, 2)?;
        let component_id = reader.u16(cursor)?;
        cursor = checked_add(cursor, 2)?;
        if component_id >= metrics.num_glyphs {
            return Err(malformed_glyf());
        }
        if flags & ARGS_ARE_XY_VALUES == 0 {
            return Err(SfntError::UnsupportedCompositeAttachment);
        }
        let (arg_x, arg_y) = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            let x = i32::from(reader.i16(cursor)?);
            let y = i32::from(reader.i16(checked_add(cursor, 2)?)?);
            cursor = checked_add(cursor, 4)?;
            (x, y)
        } else {
            let x = i32::from(reader.i8(cursor)?);
            let y = i32::from(reader.i8(checked_add(cursor, 1)?)?);
            cursor = checked_add(cursor, 2)?;
            (x, y)
        };

        let scale_flags =
            flags & (WE_HAVE_A_SCALE | WE_HAVE_AN_X_AND_Y_SCALE | WE_HAVE_A_TWO_BY_TWO);
        if scale_flags.count_ones() > 1 {
            return Err(malformed_glyf());
        }
        let (a, b, c, d) = if flags & WE_HAVE_A_SCALE != 0 {
            let scale = f32::from(reader.i16(cursor)?) / 16384.0;
            cursor = checked_add(cursor, 2)?;
            (scale, 0.0, 0.0, scale)
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            let x_scale = f32::from(reader.i16(cursor)?) / 16384.0;
            let y_scale = f32::from(reader.i16(checked_add(cursor, 2)?)?) / 16384.0;
            cursor = checked_add(cursor, 4)?;
            (x_scale, 0.0, 0.0, y_scale)
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            let a = f32::from(reader.i16(cursor)?) / 16384.0;
            let b = f32::from(reader.i16(checked_add(cursor, 2)?)?) / 16384.0;
            let c = f32::from(reader.i16(checked_add(cursor, 4)?)?) / 16384.0;
            let d = f32::from(reader.i16(checked_add(cursor, 6)?)?) / 16384.0;
            cursor = checked_add(cursor, 8)?;
            (a, b, c, d)
        } else {
            (1.0, 0.0, 0.0, 1.0)
        };

        let mut dx = arg_x as f32;
        let mut dy = arg_y as f32;
        if flags & SCALED_COMPONENT_OFFSET != 0 && flags & UNSCALED_COMPONENT_OFFSET != 0 {
            return Err(malformed_glyf());
        }
        if flags & SCALED_COMPONENT_OFFSET != 0 {
            (dx, dy) = (a * dx + c * dy, b * dx + d * dy);
        }

        // Vary the child's points before this component's transform; the
        // parent's gvar delta moves the assembled component afterward.
        let component = parse_glyph(
            face,
            glyf,
            offsets,
            metrics,
            component_id,
            depth + 1,
            active,
            coordinates,
        )?;
        let component_points = component.points.len();
        point_count = checked_add(point_count, component_points)?;
        if point_count > MAX_POINTS {
            return Err(malformed_glyf());
        }
        let component_start = points.len();
        component_ranges.push(ContourRange {
            start: component_start,
            end: checked_add(component_start, component_points)?,
        });
        points.reserve(component_points);
        for point in component.points {
            points.push(OutlinePoint {
                x: a * point.x + c * point.y + dx,
                y: b * point.x + d * point.y + dy,
                on_curve: point.on_curve,
            });
        }
        contours.reserve(component.contours.len());
        for contour in component.contours {
            contours.push(ContourRange {
                start: component_start + contour.start,
                end: component_start + contour.end,
            });
        }
        has_more = flags & MORE_COMPONENTS != 0;
        has_instructions = flags & WE_HAVE_INSTRUCTIONS != 0;
    }

    if has_instructions {
        let length = usize::from(reader.u16(cursor)?);
        cursor = checked_add(cursor, 2)?;
        reader.range(cursor, length)?;
    }

    Ok(GlyphOutline {
        bounds,
        points,
        contours,
        component_ranges,
        is_composite: true,
    })
}

fn malformed_glyf() -> SfntError {
    SfntError::MalformedTable(Tag::from_bytes(*b"glyf"))
}

fn malformed_loca() -> SfntError {
    SfntError::MalformedTable(Tag::from_bytes(*b"loca"))
}
