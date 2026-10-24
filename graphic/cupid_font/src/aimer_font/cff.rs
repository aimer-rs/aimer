use super::{FontMetrics, Reader, SfntError, SfntFace, Tag, checked_add, checked_mul};

const CFF_TAG: Tag = Tag(*b"CFF ");
const CFF2_TAG: Tag = Tag(*b"CFF2");
const MAX_CFF_INDEX_COUNT: u32 = 1 << 20;
const MAX_CFF_OPERANDS: usize = 513;
const MAX_CFF_COMMANDS: usize = 1 << 20;
const MAX_CFF_SUBROUTINE_DEPTH: usize = 32;
const MAX_CFF_STEMS: usize = 96;
const MAX_CFF_COORDINATE: f32 = 1.0e9;

/// A lossless path command emitted by a Type 2 CFF/CFF2 charstring.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CffPathCommand {
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    CurveTo {
        control_1_x: f32,
        control_1_y: f32,
        control_2_x: f32,
        control_2_y: f32,
        x: f32,
        y: f32,
    },
    Close,
}

/// A CFF/CFF2 glyph outline before cubic flattening or scan conversion.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CffGlyphOutline {
    pub(crate) bounds: [f32; 4],
    pub(crate) commands: Vec<CffPathCommand>,
}

impl<'a> SfntFace<'a> {
    /// Extracts a CFF or CFF2 glyph as a lossless cubic path.
    ///
    /// `Ok(None)` means the face has no PostScript outline table or the glyph
    /// ID is outside `maxp.numGlyphs`. CFF2 variation operators are rejected
    /// until the variable-font phase supplies normalized axis coordinates.
    pub(crate) fn cff_outline(&self, glyph_id: u16) -> Result<Option<CffGlyphOutline>, SfntError> {
        let (tag, table) = if let Some(table) = self.table(*b"CFF ") {
            (CFF_TAG, table)
        } else if let Some(table) = self.table(*b"CFF2") {
            (CFF2_TAG, table)
        } else {
            return Ok(None);
        };
        let metrics = self.metrics()?;
        if glyph_id >= metrics.num_glyphs {
            return Ok(None);
        }

        CffFont::parse(table, tag, tag == CFF2_TAG, metrics)?.outline(glyph_id)
    }
}

struct CffFont<'a> {
    tag: Tag,
    charstrings: CffIndex<'a>,
    global_subrs: CffIndex<'a>,
    top_local_subrs: Option<CffIndex<'a>>,
    font_dicts: Vec<FontDict<'a>>,
    fd_select: Option<FdSelect>,
}

#[derive(Default)]
struct TopDict {
    charstrings_offset: Option<usize>,
    private: Option<(usize, usize)>,
    fd_array_offset: Option<usize>,
    fd_select_offset: Option<usize>,
}

struct FontDict<'a> {
    local_subrs: Option<CffIndex<'a>>,
}

struct FdSelect {
    glyph_fds: Vec<u16>,
}

struct CffIndex<'a> {
    bytes: &'a [u8],
    ranges: Vec<(usize, usize)>,
    end: usize,
}

impl<'a> CffIndex<'a> {
    fn parse(bytes: &'a [u8], offset: usize, cff2: bool, tag: Tag) -> Result<Self, SfntError> {
        let reader = Reader::new(bytes);
        let count_width = if cff2 { 4 } else { 2 };
        let count = if cff2 {
            reader.u32(offset)?
        } else {
            u32::from(reader.u16(offset)?)
        };
        if count > MAX_CFF_INDEX_COUNT {
            return Err(malformed(tag));
        }
        let count_usize = usize::try_from(count).map_err(|_| SfntError::ArithmeticOverflow)?;
        let count_end = checked_add(offset, count_width)?;
        if count == 0 {
            return Ok(Self {
                bytes,
                ranges: Vec::new(),
                end: count_end,
            });
        }

        let off_size = reader.u8(count_end)?;
        if !(1..=4).contains(&off_size) {
            return Err(malformed(tag));
        }
        let offsets_start = checked_add(count_end, 1)?;
        let offsets_count = checked_add(count_usize, 1)?;
        let offsets_size = checked_mul(offsets_count, usize::from(off_size))?;
        reader.range(offsets_start, offsets_size)?;
        let data_start = checked_add(offsets_start, offsets_size)?;
        let first = index_offset(&reader, offsets_start, 0, off_size)?;
        if first != 1 {
            return Err(malformed(tag));
        }
        let last = index_offset(&reader, offsets_start, count_usize, off_size)?;
        if last == 0 {
            return Err(malformed(tag));
        }
        let data_length = last.checked_sub(1).ok_or_else(|| malformed(tag))?;
        reader.range(data_start, data_length)?;
        let data_end = checked_add(data_start, data_length)?;

        let mut ranges = Vec::with_capacity(count_usize);
        let mut previous = first;
        for index in 0..count_usize {
            let next = index_offset(&reader, offsets_start, index + 1, off_size)?;
            if next < previous || next > last || previous == 0 {
                return Err(malformed(tag));
            }
            let start = checked_add(data_start, previous - 1)?;
            let end = checked_add(data_start, next - 1)?;
            ranges.push((start, end));
            previous = next;
        }

        Ok(Self {
            bytes,
            ranges,
            end: data_end,
        })
    }

    fn len(&self) -> usize {
        self.ranges.len()
    }

    fn item(&self, index: usize) -> Option<&'a [u8]> {
        let (start, end) = *self.ranges.get(index)?;
        self.bytes.get(start..end)
    }
}

fn index_offset(
    reader: &Reader<'_>,
    offset: usize,
    index: usize,
    off_size: u8,
) -> Result<usize, SfntError> {
    let position = checked_add(offset, checked_mul(index, usize::from(off_size))?)?;
    let value = match off_size {
        1 => u32::from(reader.u8(position)?),
        2 => u32::from(reader.u16(position)?),
        3 => reader.u24(position)?,
        4 => reader.u32(position)?,
        _ => return Err(SfntError::ArithmeticOverflow),
    };
    usize::try_from(value).map_err(|_| SfntError::ArithmeticOverflow)
}

impl<'a> CffFont<'a> {
    fn parse(
        bytes: &'a [u8],
        tag: Tag,
        cff2: bool,
        metrics: FontMetrics,
    ) -> Result<Self, SfntError> {
        let reader = Reader::new(bytes);
        let (top_dict_bytes, global_subrs) = if cff2 {
            if reader.u8(0)? != 2 {
                return Err(malformed(tag));
            }
            let header_size = usize::from(reader.u8(2)?);
            if header_size < 5 {
                return Err(malformed(tag));
            }
            let top_dict_length = usize::from(reader.u16(3)?);
            reader.range(header_size, top_dict_length)?;
            let global_offset = checked_add(header_size, top_dict_length)?;
            let global_subrs = CffIndex::parse(bytes, global_offset, true, tag)?;
            (reader.range(header_size, top_dict_length)?, global_subrs)
        } else {
            if reader.u8(0)? != 1 {
                return Err(malformed(tag));
            }
            let header_size = usize::from(reader.u8(2)?);
            if header_size < 4 || !(1..=4).contains(&reader.u8(3)?) {
                return Err(malformed(tag));
            }
            let names = CffIndex::parse(bytes, header_size, false, tag)?;
            let top = CffIndex::parse(bytes, names.end, false, tag)?;
            let top_dict = top.item(0).ok_or_else(|| malformed(tag))?;
            let strings = CffIndex::parse(bytes, top.end, false, tag)?;
            let global_subrs = CffIndex::parse(bytes, strings.end, false, tag)?;
            (top_dict, global_subrs)
        };

        let top_dict = parse_top_dict(top_dict_bytes, tag)?;
        let charstrings_offset = top_dict.charstrings_offset.ok_or_else(|| malformed(tag))?;
        let charstrings = CffIndex::parse(bytes, charstrings_offset, cff2, tag)?;
        if charstrings.len() != usize::from(metrics.num_glyphs) {
            return Err(malformed(tag));
        }

        let top_local_subrs = parse_private_subrs(bytes, top_dict.private, cff2, tag)?;
        let mut font_dicts = Vec::new();
        if let Some(fd_array_offset) = top_dict.fd_array_offset {
            let fd_array = CffIndex::parse(bytes, fd_array_offset, cff2, tag)?;
            for index in 0..fd_array.len() {
                let dict = fd_array.item(index).ok_or_else(|| malformed(tag))?;
                let private = parse_font_dict(dict, tag)?;
                font_dicts.push(FontDict {
                    local_subrs: parse_private_subrs(bytes, private, cff2, tag)?,
                });
            }
        }
        let fd_select = top_dict
            .fd_select_offset
            .map(|offset| FdSelect::parse(bytes, offset, metrics.num_glyphs, tag))
            .transpose()?;
        if fd_select.is_some() && font_dicts.is_empty() {
            return Err(malformed(tag));
        }
        if fd_select.is_none() && font_dicts.len() > 1 {
            return Err(malformed(tag));
        }
        if fd_select.as_ref().is_some_and(|select| {
            select
                .glyph_fds
                .iter()
                .any(|fd| usize::from(*fd) >= font_dicts.len())
        }) {
            return Err(malformed(tag));
        }

        Ok(Self {
            tag,
            charstrings,
            global_subrs,
            top_local_subrs,
            font_dicts,
            fd_select,
        })
    }

    fn outline(&self, glyph_id: u16) -> Result<Option<CffGlyphOutline>, SfntError> {
        let Some(program) = self.charstrings.item(usize::from(glyph_id)) else {
            return Ok(None);
        };
        let mut state = CharStringState::default();
        let mut active = Vec::with_capacity(4);
        let stop = self.execute(program, false, glyph_id, &mut state, &mut active, 0)?;
        if !matches!(stop, Stop::EndChar) || !state.stack.is_empty() {
            return Err(malformed(self.tag));
        }
        Ok(Some(state.builder.finish()))
    }

    fn local_subrs(&self, glyph_id: u16) -> Option<&CffIndex<'a>> {
        if let Some(fd_select) = &self.fd_select {
            let fd = usize::from(*fd_select.glyph_fds.get(usize::from(glyph_id))?);
            return self.font_dicts.get(fd)?.local_subrs.as_ref();
        }
        self.font_dicts
            .first()
            .and_then(|font_dict| font_dict.local_subrs.as_ref())
            .or(self.top_local_subrs.as_ref())
    }

    fn execute(
        &self,
        program: &[u8],
        is_subroutine: bool,
        glyph_id: u16,
        state: &mut CharStringState,
        active: &mut Vec<SubroutineRef>,
        depth: usize,
    ) -> Result<Stop, SfntError> {
        if depth > MAX_CFF_SUBROUTINE_DEPTH {
            return Err(SfntError::CffSubroutineRecursionLimit);
        }
        let mut cursor = 0;
        while cursor < program.len() {
            let operator = program[cursor];
            cursor = checked_add(cursor, 1)?;
            if is_charstring_number(operator) {
                let value = decode_charstring_number(program, &mut cursor, operator, self.tag)?;
                if state.stack.len() >= MAX_CFF_OPERANDS {
                    return Err(malformed(self.tag));
                }
                state.stack.push(value);
                continue;
            }

            match operator {
                1 | 3 | 18 | 23 => consume_stems(state, self.tag)?,
                4 => {
                    let [dy] = move_arguments(state, 1, self.tag)?;
                    state.y += dy;
                    state.builder.move_to(state.x, state.y, self.tag)?;
                    state.has_move_to = true;
                }
                5 => line_to(state, self.tag)?,
                6 => horizontal_line_to(state, self.tag)?,
                7 => vertical_line_to(state, self.tag)?,
                8 => curve_to(state, self.tag)?,
                10 => match self.call_subroutine(false, glyph_id, state, active, depth)? {
                    Stop::Return => {}
                    Stop::EndChar => return Ok(Stop::EndChar),
                },
                11 => {
                    if !is_subroutine {
                        return Err(malformed(self.tag));
                    }
                    return Ok(Stop::Return);
                }
                14 => {
                    if state.stack.len() == 1 && !state.width_decided {
                        state.stack.clear();
                        state.width_decided = true;
                    } else if !state.stack.is_empty() {
                        return Err(SfntError::UnsupportedCffOperator {
                            tag: self.tag,
                            operator: u16::from(operator),
                        });
                    }
                    if cursor != program.len() {
                        return Err(malformed(self.tag));
                    }
                    state.builder.close(self.tag)?;
                    return Ok(Stop::EndChar);
                }
                15..=17 => {
                    return Err(SfntError::UnsupportedCffOperator {
                        tag: self.tag,
                        operator: u16::from(operator),
                    });
                }
                19 | 20 => {
                    consume_stems(state, self.tag)?;
                    let mask_size = checked_add(state.stem_count, 7)? / 8;
                    let end = checked_add(cursor, mask_size)?;
                    if end > program.len() {
                        return Err(malformed(self.tag));
                    }
                    cursor = end;
                }
                21 => {
                    let [dx, dy] = move_arguments(state, 2, self.tag)?;
                    state.x += dx;
                    state.y += dy;
                    state.builder.move_to(state.x, state.y, self.tag)?;
                    state.has_move_to = true;
                }
                22 => {
                    let [dx] = move_arguments(state, 1, self.tag)?;
                    state.x += dx;
                    state.builder.move_to(state.x, state.y, self.tag)?;
                    state.has_move_to = true;
                }
                24 => curve_line(state, self.tag)?,
                25 => line_curve(state, self.tag)?,
                26 => vv_curve_to(state, self.tag)?,
                27 => hh_curve_to(state, self.tag)?,
                28 => unreachable!("shortint is decoded as a number"),
                29 => match self.call_subroutine(true, glyph_id, state, active, depth)? {
                    Stop::Return => {}
                    Stop::EndChar => return Ok(Stop::EndChar),
                },
                30 => vh_curve_to(state, self.tag)?,
                31 => hv_curve_to(state, self.tag)?,
                12 => {
                    let extended = *program.get(cursor).ok_or_else(|| malformed(self.tag))?;
                    cursor = checked_add(cursor, 1)?;
                    match extended {
                        16 => {
                            return Err(SfntError::UnsupportedCffOperator {
                                tag: self.tag,
                                operator: 0x0c10,
                            });
                        }
                        22 => {
                            return Err(SfntError::UnsupportedCffOperator {
                                tag: self.tag,
                                operator: 0x0c16,
                            });
                        }
                        34 => flex(state, self.tag)?,
                        35 => hflex(state, self.tag)?,
                        36 => hflex1(state, self.tag)?,
                        37 => flex1(state, self.tag)?,
                        30 => roll(state, self.tag)?,
                        _ => {
                            return Err(SfntError::UnsupportedCffOperator {
                                tag: self.tag,
                                operator: 0x0c00 | u16::from(extended),
                            });
                        }
                    }
                }
                _ => {
                    return Err(SfntError::UnsupportedCffOperator {
                        tag: self.tag,
                        operator: u16::from(operator),
                    });
                }
            }
        }

        Err(malformed(self.tag))
    }

    // In Type 2 charstrings, `endchar` in a subroutine ends the full glyph program.
    fn call_subroutine(
        &self,
        global: bool,
        glyph_id: u16,
        state: &mut CharStringState,
        active: &mut Vec<SubroutineRef>,
        depth: usize,
    ) -> Result<Stop, SfntError> {
        let operand = state.stack.pop().ok_or_else(|| malformed(self.tag))?;
        let bias = if global {
            subroutine_bias(self.global_subrs.len())
        } else {
            subroutine_bias(self.local_subrs(glyph_id).map_or(0, CffIndex::len))
        };
        let index = subroutine_index(operand, bias).ok_or_else(|| malformed(self.tag))?;
        let reference = SubroutineRef { global, index };
        if active.contains(&reference) {
            return Err(SfntError::CffSubroutineCycle { global, index });
        }
        if active.len() >= MAX_CFF_SUBROUTINE_DEPTH {
            return Err(SfntError::CffSubroutineRecursionLimit);
        }
        let Some(program) = (if global {
            self.global_subrs.item(index)
        } else {
            self.local_subrs(glyph_id)
                .and_then(|subrs| subrs.item(index))
        }) else {
            return Err(malformed(self.tag));
        };

        active.push(reference);
        let result = self.execute(program, true, glyph_id, state, active, depth + 1);
        active.pop();
        result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SubroutineRef {
    global: bool,
    index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stop {
    Return,
    EndChar,
}

#[derive(Default)]
struct CharStringState {
    stack: Vec<f32>,
    builder: CffPathBuilder,
    x: f32,
    y: f32,
    has_move_to: bool,
    width_decided: bool,
    stem_count: usize,
}

#[derive(Default)]
struct CffPathBuilder {
    commands: Vec<CffPathCommand>,
    bounds: Option<[f32; 4]>,
    current: Option<(f32, f32)>,
}

impl CffPathBuilder {
    fn move_to(&mut self, x: f32, y: f32, tag: Tag) -> Result<(), SfntError> {
        validate_point(x, y, tag)?;
        self.finish_contour(tag)?;
        self.push(CffPathCommand::MoveTo { x, y }, tag)?;
        self.current = Some((x, y));
        self.include(x, y);
        Ok(())
    }

    fn line_to(&mut self, x: f32, y: f32, tag: Tag) -> Result<(), SfntError> {
        if self.current.is_none() {
            return Err(malformed(tag));
        }
        validate_point(x, y, tag)?;
        self.push(CffPathCommand::LineTo { x, y }, tag)?;
        self.current = Some((x, y));
        self.include(x, y);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn curve_to(
        &mut self,
        control_1_x: f32,
        control_1_y: f32,
        control_2_x: f32,
        control_2_y: f32,
        x: f32,
        y: f32,
        tag: Tag,
    ) -> Result<(), SfntError> {
        let (start_x, start_y) = self.current.ok_or_else(|| malformed(tag))?;
        validate_point(control_1_x, control_1_y, tag)?;
        validate_point(control_2_x, control_2_y, tag)?;
        validate_point(x, y, tag)?;
        self.push(
            CffPathCommand::CurveTo {
                control_1_x,
                control_1_y,
                control_2_x,
                control_2_y,
                x,
                y,
            },
            tag,
        )?;
        self.current = Some((x, y));
        self.include_cubic(
            (start_x, start_y),
            (control_1_x, control_1_y),
            (control_2_x, control_2_y),
            (x, y),
        );
        Ok(())
    }

    fn close(&mut self, tag: Tag) -> Result<(), SfntError> {
        self.finish_contour(tag)
    }

    fn finish_contour(&mut self, tag: Tag) -> Result<(), SfntError> {
        if self.current.take().is_some() {
            self.push(CffPathCommand::Close, tag)?;
        }
        Ok(())
    }

    fn push(&mut self, command: CffPathCommand, tag: Tag) -> Result<(), SfntError> {
        if self.commands.len() >= MAX_CFF_COMMANDS {
            return Err(malformed(tag));
        }
        self.commands.push(command);
        Ok(())
    }

    fn include(&mut self, x: f32, y: f32) {
        if let Some(bounds) = &mut self.bounds {
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x);
            bounds[3] = bounds[3].max(y);
        } else {
            self.bounds = Some([x, y, x, y]);
        }
    }

    fn include_cubic(
        &mut self,
        start: (f32, f32),
        control_1: (f32, f32),
        control_2: (f32, f32),
        end: (f32, f32),
    ) {
        self.include(start.0, start.1);
        self.include(end.0, end.1);
        for t in cubic_extrema(start.0, control_1.0, control_2.0, end.0)
            .into_iter()
            .flatten()
        {
            self.include(
                cubic_value(start.0, control_1.0, control_2.0, end.0, t),
                start.1,
            );
        }
        for t in cubic_extrema(start.1, control_1.1, control_2.1, end.1)
            .into_iter()
            .flatten()
        {
            self.include(
                start.0,
                cubic_value(start.1, control_1.1, control_2.1, end.1, t),
            );
        }
    }

    fn finish(mut self) -> CffGlyphOutline {
        let _ = self.finish_contour(CFF_TAG);
        CffGlyphOutline {
            bounds: self.bounds.unwrap_or([0.0; 4]),
            commands: self.commands,
        }
    }
}

fn cubic_extrema(p0: f32, p1: f32, p2: f32, p3: f32) -> [Option<f32>; 2] {
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    if a.abs() < f32::EPSILON {
        let root = if b.abs() < f32::EPSILON {
            None
        } else {
            Some(-c / b)
        };
        return [root, None];
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return [None, None];
    }
    let square_root = discriminant.sqrt();
    [
        Some((-b + square_root) / (2.0 * a)),
        Some((-b - square_root) / (2.0 * a)),
    ]
    .map(|root| root.filter(|value| *value > 0.0 && *value < 1.0))
}

fn cubic_value(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let one_minus_t = 1.0 - t;
    one_minus_t * one_minus_t * one_minus_t * p0
        + 3.0 * one_minus_t * one_minus_t * t * p1
        + 3.0 * one_minus_t * t * t * p2
        + t * t * t * p3
}

fn require_move(state: &CharStringState, tag: Tag) -> Result<(), SfntError> {
    if state.has_move_to {
        Ok(())
    } else {
        Err(malformed(tag))
    }
}

fn validate_point(x: f32, y: f32, tag: Tag) -> Result<(), SfntError> {
    if x.is_finite()
        && y.is_finite()
        && x.abs() <= MAX_CFF_COORDINATE
        && y.abs() <= MAX_CFF_COORDINATE
    {
        Ok(())
    } else {
        Err(malformed(tag))
    }
}

fn move_arguments<const N: usize>(
    state: &mut CharStringState,
    expected: usize,
    tag: Tag,
) -> Result<[f32; N], SfntError> {
    debug_assert_eq!(N, expected);
    if !state.width_decided && state.stack.len() == expected + 1 {
        state.stack.remove(0);
    }
    state.width_decided = true;
    if state.stack.len() != expected {
        return Err(malformed(tag));
    }
    let arguments = std::mem::take(&mut state.stack);
    arguments.try_into().map_err(|_| malformed(tag))
}

fn consume_stems(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    if !state.width_decided && state.stack.len() % 2 == 1 {
        state.stack.remove(0);
    }
    state.width_decided = true;
    if !state.stack.len().is_multiple_of(2) {
        return Err(malformed(tag));
    }
    let count = state.stack.len() / 2;
    state.stem_count = state
        .stem_count
        .checked_add(count)
        .ok_or_else(|| malformed(tag))?;
    if state.stem_count > MAX_CFF_STEMS {
        return Err(malformed(tag));
    }
    state.stack.clear();
    Ok(())
}

/// Applies the Type 2 `roll` stack operator.
fn roll(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    if state.stack.len() < 2 {
        return Err(malformed(tag));
    }
    let shift = integer_operand(state.stack.pop().ok_or_else(|| malformed(tag))?, tag)?;
    let count = integer_operand(state.stack.pop().ok_or_else(|| malformed(tag))?, tag)?;
    if count < 0 {
        return Err(malformed(tag));
    }
    let count = usize::try_from(count).map_err(|_| malformed(tag))?;
    if count > state.stack.len() {
        return Err(malformed(tag));
    }
    if count > 1 {
        let shift = shift.rem_euclid(count as i32) as usize;
        let start = state.stack.len() - count;
        state.stack[start..].rotate_right(shift);
    }
    Ok(())
}

fn integer_operand(value: f32, tag: Tag) -> Result<i32, SfntError> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i32::MIN as f32
        || value > i32::MAX as f32
    {
        return Err(malformed(tag));
    }
    Ok(value as i32)
}

fn line_to(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    if state.stack.is_empty() || !state.stack.len().is_multiple_of(2) {
        return Err(malformed(tag));
    }
    let arguments = std::mem::take(&mut state.stack);
    for pair in arguments.as_chunks::<2>().0 {
        state.x += pair[0];
        state.y += pair[1];
        state.builder.line_to(state.x, state.y, tag)?;
    }
    Ok(())
}

fn horizontal_line_to(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    if state.stack.is_empty() {
        return Err(malformed(tag));
    }
    let arguments = std::mem::take(&mut state.stack);
    for (index, value) in arguments.into_iter().enumerate() {
        if index % 2 == 0 {
            state.x += value;
        } else {
            state.y += value;
        }
        state.builder.line_to(state.x, state.y, tag)?;
    }
    Ok(())
}

fn vertical_line_to(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    if state.stack.is_empty() {
        return Err(malformed(tag));
    }
    let arguments = std::mem::take(&mut state.stack);
    for (index, value) in arguments.into_iter().enumerate() {
        if index % 2 == 0 {
            state.y += value;
        } else {
            state.x += value;
        }
        state.builder.line_to(state.x, state.y, tag)?;
    }
    Ok(())
}

fn curve_to(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    if state.stack.is_empty() || !state.stack.len().is_multiple_of(6) {
        return Err(malformed(tag));
    }
    let arguments = std::mem::take(&mut state.stack);
    for values in arguments.as_chunks::<6>().0 {
        let control_1_x = state.x + values[0];
        let control_1_y = state.y + values[1];
        let control_2_x = control_1_x + values[2];
        let control_2_y = control_1_y + values[3];
        state.x = control_2_x + values[4];
        state.y = control_2_y + values[5];
        state.builder.curve_to(
            control_1_x,
            control_1_y,
            control_2_x,
            control_2_y,
            state.x,
            state.y,
            tag,
        )?;
    }
    Ok(())
}

fn curve_line(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    if state.stack.len() < 8 || !(state.stack.len() - 2).is_multiple_of(6) {
        return Err(malformed(tag));
    }
    let arguments = std::mem::take(&mut state.stack);
    let split = arguments.len() - 2;
    curve_arguments(state, &arguments[..split], tag)?;
    state.x += arguments[split];
    state.y += arguments[split + 1];
    state.builder.line_to(state.x, state.y, tag)
}

fn line_curve(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    if state.stack.len() < 8 || !(state.stack.len() - 6).is_multiple_of(2) {
        return Err(malformed(tag));
    }
    let arguments = std::mem::take(&mut state.stack);
    let split = arguments.len() - 6;
    for pair in arguments[..split].as_chunks::<2>().0 {
        state.x += pair[0];
        state.y += pair[1];
        state.builder.line_to(state.x, state.y, tag)?;
    }
    curve_arguments(state, &arguments[split..], tag)
}

fn curve_arguments(
    state: &mut CharStringState,
    arguments: &[f32],
    tag: Tag,
) -> Result<(), SfntError> {
    if arguments.is_empty() || !arguments.len().is_multiple_of(6) {
        return Err(malformed(tag));
    }
    for values in arguments.as_chunks::<6>().0 {
        let control_1_x = state.x + values[0];
        let control_1_y = state.y + values[1];
        let control_2_x = control_1_x + values[2];
        let control_2_y = control_1_y + values[3];
        state.x = control_2_x + values[4];
        state.y = control_2_y + values[5];
        state.builder.curve_to(
            control_1_x,
            control_1_y,
            control_2_x,
            control_2_y,
            state.x,
            state.y,
            tag,
        )?;
    }
    Ok(())
}

fn hh_curve_to(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    if state.stack.is_empty() {
        return Err(malformed(tag));
    }
    let arguments = std::mem::take(&mut state.stack);
    let mut index = 0;
    let initial_dy = if arguments.len() % 4 == 1 {
        let value = arguments[0];
        index = 1;
        value
    } else {
        0.0
    };
    if !(arguments.len() - index).is_multiple_of(4) {
        return Err(malformed(tag));
    }
    for (curve_index, values) in arguments[index..].as_chunks::<4>().0.iter().enumerate() {
        let control_1_x = state.x + values[0];
        let control_1_y = state.y + if curve_index == 0 { initial_dy } else { 0.0 };
        let control_2_x = control_1_x + values[1];
        let control_2_y = control_1_y + values[2];
        state.x = control_2_x + values[3];
        state.y = control_2_y;
        state.builder.curve_to(
            control_1_x,
            control_1_y,
            control_2_x,
            control_2_y,
            state.x,
            state.y,
            tag,
        )?;
    }
    Ok(())
}

fn vv_curve_to(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    if state.stack.is_empty() {
        return Err(malformed(tag));
    }
    let arguments = std::mem::take(&mut state.stack);
    let mut index = 0;
    if arguments.len() % 4 == 1 {
        state.x += arguments[0];
        index = 1;
    }
    if !(arguments.len() - index).is_multiple_of(4) {
        return Err(malformed(tag));
    }
    for values in arguments[index..].as_chunks::<4>().0 {
        let control_1_x = state.x;
        let control_1_y = state.y + values[0];
        let control_2_x = control_1_x + values[1];
        let control_2_y = control_1_y + values[2];
        state.x = control_2_x;
        state.y = control_2_y + values[3];
        state.builder.curve_to(
            control_1_x,
            control_1_y,
            control_2_x,
            control_2_y,
            state.x,
            state.y,
            tag,
        )?;
    }
    Ok(())
}

fn hv_curve_to(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    alternating_curve_to(state, tag, true)
}

fn vh_curve_to(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    alternating_curve_to(state, tag, false)
}

fn alternating_curve_to(
    state: &mut CharStringState,
    tag: Tag,
    starts_horizontal: bool,
) -> Result<(), SfntError> {
    require_move(state, tag)?;
    if state.stack.len() < 4 || state.stack.len() % 4 > 1 {
        return Err(malformed(tag));
    }
    let arguments = std::mem::take(&mut state.stack);
    let has_final_delta = arguments.len() % 4 == 1;
    let group_count = arguments.len() / 4;
    let mut index = 0;
    for group in 0..group_count {
        let values = &arguments[index..index + 4];
        index += 4;
        let horizontal = starts_horizontal == (group % 2 == 0);
        let (control_1_x, control_1_y, control_2_x, control_2_y, end_x, end_y) = if horizontal {
            let control_1_x = state.x + values[0];
            let control_1_y = state.y;
            let control_2_x = control_1_x + values[1];
            let control_2_y = control_1_y + values[2];
            let mut end_x = control_2_x;
            if has_final_delta && group + 1 == group_count {
                end_x += arguments[index];
                index += 1;
            }
            let end_y = control_2_y + values[3];
            (
                control_1_x,
                control_1_y,
                control_2_x,
                control_2_y,
                end_x,
                end_y,
            )
        } else {
            let control_1_x = state.x;
            let control_1_y = state.y + values[0];
            let control_2_x = control_1_x + values[1];
            let control_2_y = control_1_y + values[2];
            let end_x = control_2_x + values[3];
            let mut end_y = control_2_y;
            if has_final_delta && group + 1 == group_count {
                end_y += arguments[index];
                index += 1;
            }
            (
                control_1_x,
                control_1_y,
                control_2_x,
                control_2_y,
                end_x,
                end_y,
            )
        };
        state.x = end_x;
        state.y = end_y;
        state.builder.curve_to(
            control_1_x,
            control_1_y,
            control_2_x,
            control_2_y,
            end_x,
            end_y,
            tag,
        )?;
    }
    Ok(())
}

fn flex(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    let arguments = std::mem::take(&mut state.stack);
    if arguments.len() != 13 {
        return Err(malformed(tag));
    }
    curve_arguments(state, &arguments[..6], tag)?;
    curve_arguments(state, &arguments[6..12], tag)
}

fn hflex(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    let arguments = std::mem::take(&mut state.stack);
    if arguments.len() != 7 {
        return Err(malformed(tag));
    }
    let first = [
        arguments[0],
        0.0,
        arguments[1],
        arguments[2],
        arguments[3],
        0.0,
    ];
    curve_arguments(state, &first, tag)?;
    let second = [
        arguments[4],
        0.0,
        arguments[5],
        -arguments[2],
        arguments[6],
        0.0,
    ];
    curve_arguments(state, &second, tag)?;
    Ok(())
}

fn hflex1(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    let arguments = std::mem::take(&mut state.stack);
    if arguments.len() != 9 {
        return Err(malformed(tag));
    }
    let first = [
        arguments[0],
        arguments[1],
        arguments[2],
        arguments[3],
        arguments[4],
        0.0,
    ];
    curve_arguments(state, &first, tag)?;
    let second = [
        arguments[5],
        0.0,
        arguments[6],
        arguments[7],
        arguments[8],
        -(arguments[1] + arguments[3] + arguments[7]),
    ];
    curve_arguments(state, &second, tag)
}

fn flex1(state: &mut CharStringState, tag: Tag) -> Result<(), SfntError> {
    require_move(state, tag)?;
    let arguments = std::mem::take(&mut state.stack);
    if arguments.len() != 11 {
        return Err(malformed(tag));
    }
    let start_x = state.x;
    let start_y = state.y;
    let control_1_x = start_x + arguments[0];
    let control_1_y = start_y + arguments[1];
    let control_2_x = control_1_x + arguments[2];
    let control_2_y = control_1_y + arguments[3];
    let first_end_x = control_2_x + arguments[4];
    let first_end_y = control_2_y + arguments[5];
    state.builder.curve_to(
        control_1_x,
        control_1_y,
        control_2_x,
        control_2_y,
        first_end_x,
        first_end_y,
        tag,
    )?;
    let control_1_x = first_end_x + arguments[6];
    let control_1_y = first_end_y + arguments[7];
    let control_2_x = control_1_x + arguments[8];
    let control_2_y = control_1_y + arguments[9];
    let remaining = arguments[10];
    let (end_x, end_y) = if (control_2_x - start_x).abs() > (control_2_y - start_y).abs() {
        (control_2_x + remaining, control_2_y)
    } else {
        (control_2_x, control_2_y + remaining)
    };
    state.x = end_x;
    state.y = end_y;
    state.builder.curve_to(
        control_1_x,
        control_1_y,
        control_2_x,
        control_2_y,
        end_x,
        end_y,
        tag,
    )
}

fn is_charstring_number(value: u8) -> bool {
    value == 28 || value == 255 || (32..=254).contains(&value)
}

fn decode_charstring_number(
    bytes: &[u8],
    cursor: &mut usize,
    first: u8,
    tag: Tag,
) -> Result<f32, SfntError> {
    let value = match first {
        28 => {
            let end = checked_add(*cursor, 2)?;
            let bytes = bytes.get(*cursor..end).ok_or_else(|| malformed(tag))?;
            *cursor = end;
            f32::from(i16::from_be_bytes([bytes[0], bytes[1]]))
        }
        255 => {
            let end = checked_add(*cursor, 4)?;
            let bytes = bytes.get(*cursor..end).ok_or_else(|| malformed(tag))?;
            *cursor = end;
            i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32 / 65536.0
        }
        32..=246 => f32::from(first) - 139.0,
        247..=250 => {
            let next = *bytes.get(*cursor).ok_or_else(|| malformed(tag))?;
            *cursor = checked_add(*cursor, 1)?;
            f32::from(u16::from(first - 247) * 256 + u16::from(next) + 108)
        }
        251..=254 => {
            let next = *bytes.get(*cursor).ok_or_else(|| malformed(tag))?;
            *cursor = checked_add(*cursor, 1)?;
            -f32::from(u16::from(first - 251) * 256 + u16::from(next) + 108)
        }
        _ => return Err(malformed(tag)),
    };
    if value.is_finite() && value.abs() <= MAX_CFF_COORDINATE {
        Ok(value)
    } else {
        Err(malformed(tag))
    }
}

fn decode_dict_number(
    bytes: &[u8],
    cursor: &mut usize,
    first: u8,
    tag: Tag,
) -> Result<f32, SfntError> {
    match first {
        28 => {
            let end = checked_add(*cursor, 2)?;
            let bytes = bytes.get(*cursor..end).ok_or_else(|| malformed(tag))?;
            *cursor = end;
            Ok(f32::from(i16::from_be_bytes([bytes[0], bytes[1]])))
        }
        29 => {
            let end = checked_add(*cursor, 4)?;
            let bytes = bytes.get(*cursor..end).ok_or_else(|| malformed(tag))?;
            *cursor = end;
            Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32)
        }
        30 => decode_real(bytes, cursor, tag),
        255 => {
            let end = checked_add(*cursor, 4)?;
            let bytes = bytes.get(*cursor..end).ok_or_else(|| malformed(tag))?;
            *cursor = end;
            Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32 / 65536.0)
        }
        32..=246 => Ok(f32::from(first) - 139.0),
        247..=250 => {
            let next = *bytes.get(*cursor).ok_or_else(|| malformed(tag))?;
            *cursor = checked_add(*cursor, 1)?;
            Ok(f32::from(
                u16::from(first - 247) * 256 + u16::from(next) + 108,
            ))
        }
        251..=254 => {
            let next = *bytes.get(*cursor).ok_or_else(|| malformed(tag))?;
            *cursor = checked_add(*cursor, 1)?;
            Ok(-f32::from(
                u16::from(first - 251) * 256 + u16::from(next) + 108,
            ))
        }
        _ => Err(malformed(tag)),
    }
}

fn decode_real(bytes: &[u8], cursor: &mut usize, tag: Tag) -> Result<f32, SfntError> {
    let mut integer = 0.0_f32;
    let mut fraction = 0.0_f32;
    let mut divisor = 1.0_f32;
    let mut exponent = 0_i32;
    let mut exponent_sign = 1_i32;
    let mut negative = false;
    let mut in_fraction = false;
    let mut in_exponent = false;
    let mut saw_digit = false;
    let mut terminated = false;

    'bytes: for byte in bytes.get(*cursor..).ok_or_else(|| malformed(tag))? {
        for nibble in [byte >> 4, byte & 0x0f] {
            match nibble {
                0..=9 if in_exponent => {
                    exponent = exponent
                        .checked_mul(10)
                        .and_then(|value| value.checked_add(i32::from(nibble)))
                        .ok_or_else(|| malformed(tag))?;
                }
                0..=9 if in_fraction => {
                    fraction = fraction * 10.0 + f32::from(nibble);
                    divisor *= 10.0;
                    saw_digit = true;
                }
                0..=9 => {
                    integer = integer * 10.0 + f32::from(nibble);
                    saw_digit = true;
                }
                0x0a if !in_exponent => in_fraction = true,
                0x0b if saw_digit => in_exponent = true,
                0x0c if saw_digit => {
                    in_exponent = true;
                    exponent_sign = -1;
                }
                0x0e if !saw_digit && !in_exponent => negative = true,
                0x0f => {
                    *cursor = checked_add(*cursor, 1)?;
                    terminated = true;
                    break 'bytes;
                }
                _ => return Err(malformed(tag)),
            }
        }
        *cursor = checked_add(*cursor, 1)?;
    }
    if !terminated || !saw_digit {
        return Err(malformed(tag));
    }
    let value = (integer + fraction / divisor) * 10.0_f32.powi(exponent * exponent_sign);
    if value.is_finite() {
        Ok(if negative { -value } else { value })
    } else {
        Err(malformed(tag))
    }
}

fn parse_top_dict(bytes: &[u8], tag: Tag) -> Result<TopDict, SfntError> {
    let mut result = TopDict::default();
    let mut operands = Vec::new();
    parse_dict(
        bytes,
        tag,
        |operator, values| {
            match operator {
                15 | 16 | 24 => {}
                17 => {
                    if values.len() != 1 {
                        return Err(malformed(tag));
                    }
                    result.charstrings_offset = Some(dict_offset(values[0], tag)?);
                }
                18 => {
                    if values.len() != 2 {
                        return Err(malformed(tag));
                    }
                    result.private =
                        Some((dict_offset(values[0], tag)?, dict_offset(values[1], tag)?));
                }
                0x0c24 => {
                    if values.len() != 1 {
                        return Err(malformed(tag));
                    }
                    result.fd_array_offset = Some(dict_offset(values[0], tag)?);
                }
                0x0c25 => {
                    if values.len() != 1 {
                        return Err(malformed(tag));
                    }
                    result.fd_select_offset = Some(dict_offset(values[0], tag)?);
                }
                // ROS marks a CID-keyed font. The FDArray/FDSelect entries below
                // already choose the correct private subroutines for each glyph;
                // the registry, ordering, and supplement strings are metadata and
                // do not affect outline decoding.
                0x0c1e if values.len() != 3 => return Err(malformed(tag)),
                0x0c1e => {}
                _ => {}
            }
            Ok(())
        },
        &mut operands,
    )
    .map(|_| result)
}

fn parse_font_dict(bytes: &[u8], tag: Tag) -> Result<Option<(usize, usize)>, SfntError> {
    let mut private = None;
    let mut operands = Vec::new();
    parse_dict(
        bytes,
        tag,
        |operator, values| {
            if operator == 18 {
                if values.len() != 2 {
                    return Err(malformed(tag));
                }
                private = Some((dict_offset(values[0], tag)?, dict_offset(values[1], tag)?));
            }
            Ok(())
        },
        &mut operands,
    )?;
    Ok(private)
}

fn parse_private_subrs<'a>(
    bytes: &'a [u8],
    private: Option<(usize, usize)>,
    cff2: bool,
    tag: Tag,
) -> Result<Option<CffIndex<'a>>, SfntError> {
    let Some((size, offset)) = private else {
        return Ok(None);
    };
    let reader = Reader::new(bytes);
    let private_bytes = reader.range(offset, size)?;
    let mut subrs_offset = None;
    let mut operands = Vec::new();
    parse_dict(
        private_bytes,
        tag,
        |operator, values| {
            if operator == 19 {
                if values.len() != 1 {
                    return Err(malformed(tag));
                }
                subrs_offset = Some(dict_offset(values[0], tag)?);
            }
            Ok(())
        },
        &mut operands,
    )?;
    let Some(relative_offset) = subrs_offset else {
        return Ok(None);
    };
    let subrs_offset = checked_add(offset, relative_offset)?;
    Ok(Some(CffIndex::parse(bytes, subrs_offset, cff2, tag)?))
}

fn parse_dict<F>(
    bytes: &[u8],
    tag: Tag,
    mut on_operator: F,
    operands: &mut Vec<f32>,
) -> Result<(), SfntError>
where
    F: FnMut(u16, &[f32]) -> Result<(), SfntError>,
{
    let mut cursor = 0;
    while cursor < bytes.len() {
        let first = bytes[cursor];
        cursor = checked_add(cursor, 1)?;
        if is_dict_number(first) {
            if operands.len() >= MAX_CFF_OPERANDS {
                return Err(malformed(tag));
            }
            operands.push(decode_dict_number(bytes, &mut cursor, first, tag)?);
            continue;
        }
        let operator = if first == 12 {
            let second = *bytes.get(cursor).ok_or_else(|| malformed(tag))?;
            cursor = checked_add(cursor, 1)?;
            0x0c00 | u16::from(second)
        } else {
            u16::from(first)
        };
        on_operator(operator, operands)?;
        operands.clear();
    }
    if operands.is_empty() {
        Ok(())
    } else {
        Err(malformed(tag))
    }
}

fn is_dict_number(value: u8) -> bool {
    value == 28 || value == 29 || value == 30 || value == 255 || (32..=254).contains(&value)
}

fn dict_offset(value: f32, tag: Tag) -> Result<usize, SfntError> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return Err(malformed(tag));
    }
    usize::try_from(value as u64).map_err(|_| malformed(tag))
}

impl FdSelect {
    fn parse(bytes: &[u8], offset: usize, glyph_count: u16, tag: Tag) -> Result<Self, SfntError> {
        let reader = Reader::new(bytes);
        let format = reader.u8(offset)?;
        let mut glyph_fds = Vec::with_capacity(usize::from(glyph_count));
        match format {
            0 => {
                let data = reader.range(checked_add(offset, 1)?, usize::from(glyph_count))?;
                glyph_fds.extend(data.iter().map(|&fd| u16::from(fd)));
            }
            3 => {
                let ranges = usize::from(reader.u16(checked_add(offset, 1)?)?);
                let range_data = checked_add(3, checked_mul(ranges, 3)?)?;
                reader.range(offset, checked_add(range_data, 2)?)?;
                let mut cursor = checked_add(offset, 3)?;
                let mut previous_end = 0_u16;
                for index in 0..ranges {
                    let start = reader.u16(cursor)?;
                    let fd = u16::from(reader.u8(checked_add(cursor, 2)?)?);
                    if (index == 0 && start != 0) || (index > 0 && start != previous_end) {
                        return Err(malformed(tag));
                    }
                    if start > glyph_count {
                        return Err(malformed(tag));
                    }
                    let next = reader.u16(checked_add(cursor, 3)?)?;
                    if next <= start || next > glyph_count {
                        return Err(malformed(tag));
                    }
                    glyph_fds.extend(std::iter::repeat_n(fd, usize::from(next - start)));
                    previous_end = next;
                    cursor = checked_add(cursor, 3)?;
                }
                let sentinel = reader.u16(cursor)?;
                if sentinel != glyph_count || glyph_fds.len() != usize::from(glyph_count) {
                    return Err(malformed(tag));
                }
            }
            _ => return Err(malformed(tag)),
        }
        Ok(Self { glyph_fds })
    }
}

fn subroutine_bias(count: usize) -> i32 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

fn subroutine_index(value: f32, bias: i32) -> Option<usize> {
    if !value.is_finite() || value.fract() != 0.0 {
        return None;
    }
    let value = value as i64 + i64::from(bias);
    usize::try_from(value).ok()
}

fn malformed(tag: Tag) -> SfntError {
    SfntError::MalformedTable(tag)
}

#[cfg(test)]
mod tests {
    use super::{CFF_TAG, CharStringState, SfntError, parse_top_dict, roll};

    #[test]
    fn roll_rotates_the_top_stack_operands() {
        let mut state = CharStringState {
            stack: vec![1.0, 2.0, 3.0, 4.0, 5.0, 3.0, 1.0],
            ..CharStringState::default()
        };

        roll(&mut state, CFF_TAG).expect("integer roll operands should be accepted");

        assert_eq!(state.stack, vec![1.0, 2.0, 5.0, 3.0, 4.0]);
    }

    #[test]
    fn roll_rejects_a_negative_stack_count() {
        let mut state = CharStringState {
            stack: vec![1.0, -1.0, 0.0],
            ..CharStringState::default()
        };

        assert_eq!(
            roll(&mut state, CFF_TAG),
            Err(SfntError::MalformedTable(CFF_TAG))
        );
    }

    #[test]
    fn accepts_cid_keyed_top_dict_metadata() {
        let ros = [139, 139, 139, 12, 30];

        parse_top_dict(&ros, CFF_TAG).expect("CID-keyed ROS metadata should not block outlines");
    }
}
