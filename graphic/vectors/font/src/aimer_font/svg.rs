//! Bounded support for OpenType `SVG ` glyph documents.
//!
//! The OpenType table is only an index into XML documents. Parsing a document
//! is intentionally lazy and the parsed result is reduced to Aimer-owned solid
//! path commands before it enters the rasterizer. Gradients, images, text,
//! strokes, filters, masks, animations, and other effects remain on the
//! compatibility path until their rendering contract is implemented here.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::svg::{SvgElementKind, SvgFillRule, SvgPathCommand, SvgTransform, parse_svg};

use super::color::ColorRgba;
use super::{FontMetrics, Reader, SfntError, SfntFace, Tag, checked_add, checked_mul};

const SVG_TAG: Tag = Tag(*b"SVG ");
const SVG_HEADER_SIZE: usize = 10;
const SVG_DOCUMENT_INDEX_ENTRY_SIZE: usize = 12;
const MAX_SVG_ENTRIES: usize = 4096;
const MAX_SVG_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_SVG_DOCUMENT_BYTES_TOTAL: usize = 64 * 1024 * 1024;
const MAX_SVG_PATHS: usize = 4096;
const MAX_SVG_COMMANDS: usize = 1 << 20;

#[derive(Clone, Copy, Debug)]
struct SvgDocumentEntry {
    start_glyph_id: u16,
    end_glyph_id: u16,
    document_start: usize,
    document_end: usize,
}

/// One supported, already-normalized SVG fill path.
#[derive(Clone, Debug)]
pub(crate) struct SvgPath {
    pub(crate) commands: Arc<[SvgPathCommand]>,
    pub(crate) color: ColorRgba,
    pub(crate) fill_rule: SvgFillRule,
    pub(crate) bounds: [f32; 4],
}

/// A cached Aimer-owned SVG glyph document.
#[derive(Clone, Debug)]
pub(crate) struct SvgGlyph {
    pub(crate) paths: Arc<[SvgPath]>,
    /// Bounds are in the OpenType y-up design grid after the SVG y-down
    /// coordinate system has been mirrored.
    pub(crate) bounds: [f32; 4],
}

/// Parsed `SVG ` index plus a lazy per-glyph document cache.
pub(crate) struct SvgTables {
    entries: Vec<SvgDocumentEntry>,
    units_per_em: u16,
    glyph_cache: Mutex<HashMap<u16, Result<Option<Arc<SvgGlyph>>, SfntError>>>,
}

/// Parses the checked `SVG ` document index without retaining document bytes.
pub(crate) fn parse(face: &SfntFace<'_>) -> Result<Option<SvgTables>, SfntError> {
    let Some(table) = face.table(*b"SVG ") else {
        return Ok(None);
    };
    let metrics = face.metrics()?;
    let entries = parse_index(table, metrics)?;
    Ok(Some(SvgTables {
        entries,
        units_per_em: metrics.units_per_em,
        glyph_cache: Mutex::new(HashMap::new()),
    }))
}

impl SvgTables {
    pub(crate) fn glyph(
        &self,
        face: &SfntFace<'_>,
        glyph_id: u16,
    ) -> Result<Option<Arc<SvgGlyph>>, SfntError> {
        if let Ok(cache) = self.glyph_cache.lock()
            && let Some(glyph) = cache.get(&glyph_id)
        {
            return glyph.clone();
        }

        let parsed = self
            .entry_for(glyph_id)
            .map(|entry| {
                let table = face
                    .table(*b"SVG ")
                    .ok_or(SfntError::MissingTable(SVG_TAG))?;
                let document = table
                    .get(entry.document_start..entry.document_end)
                    .ok_or(SfntError::MalformedTable(SVG_TAG))?;
                Ok(parse_document(document, self.units_per_em).map(Arc::new))
            })
            .unwrap_or(Ok(None));

        if let Ok(mut cache) = self.glyph_cache.lock() {
            if let Some(existing) = cache.get(&glyph_id) {
                return existing.clone();
            }
            cache.insert(glyph_id, parsed.clone());
        }
        parsed
    }

    fn entry_for(&self, glyph_id: u16) -> Option<SvgDocumentEntry> {
        self.entries
            .binary_search_by(|entry| {
                if glyph_id < entry.start_glyph_id {
                    Ordering::Greater
                } else if glyph_id > entry.end_glyph_id {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            })
            .ok()
            .and_then(|index| self.entries.get(index).copied())
    }
}

fn parse_index(table: &[u8], metrics: FontMetrics) -> Result<Vec<SvgDocumentEntry>, SfntError> {
    let reader = Reader::new(table);
    if table.len() < SVG_HEADER_SIZE {
        return Err(SfntError::MalformedTable(SVG_TAG));
    }
    if reader.u16(0)? != 0 {
        return Err(SfntError::MalformedTable(SVG_TAG));
    }

    let index_start = usize::try_from(reader.u32(2)?).map_err(|_| SfntError::ArithmeticOverflow)?;
    if index_start < SVG_HEADER_SIZE {
        return Err(SfntError::MalformedTable(SVG_TAG));
    }
    let index_count = usize::from(reader.u16(index_start)?);
    if index_count > MAX_SVG_ENTRIES {
        return Err(SfntError::MalformedTable(SVG_TAG));
    }
    let records_start = checked_add(index_start, 2)?;
    let records_size = checked_mul(index_count, SVG_DOCUMENT_INDEX_ENTRY_SIZE)?;
    let records_end = checked_add(records_start, records_size)?;
    reader.range(records_start, records_size)?;

    let mut entries = Vec::with_capacity(index_count);
    let mut previous_end = None;
    let mut document_bytes = 0_usize;
    for index in 0..index_count {
        let record_start = checked_add(
            records_start,
            checked_mul(index, SVG_DOCUMENT_INDEX_ENTRY_SIZE)?,
        )?;
        let start_glyph_id = reader.u16(record_start)?;
        let end_glyph_id = reader.u16(checked_add(record_start, 2)?)?;
        if start_glyph_id > end_glyph_id
            || u32::from(end_glyph_id) >= u32::from(metrics.num_glyphs)
            || previous_end.is_some_and(|end| start_glyph_id <= end)
        {
            return Err(SfntError::MalformedTable(SVG_TAG));
        }
        let document_start = usize::try_from(reader.u32(checked_add(record_start, 4)?)?)
            .map_err(|_| SfntError::ArithmeticOverflow)?;
        let document_length = usize::try_from(reader.u32(checked_add(record_start, 8)?)?)
            .map_err(|_| SfntError::ArithmeticOverflow)?;
        if document_length == 0 || document_length > MAX_SVG_DOCUMENT_BYTES {
            return Err(SfntError::MalformedTable(SVG_TAG));
        }
        let document_end = checked_add(document_start, document_length)?;
        if document_end > table.len() {
            return Err(SfntError::MalformedTable(SVG_TAG));
        }
        document_bytes = document_bytes
            .checked_add(document_length)
            .ok_or(SfntError::ArithmeticOverflow)?;
        if document_bytes > MAX_SVG_DOCUMENT_BYTES_TOTAL {
            return Err(SfntError::MalformedTable(SVG_TAG));
        }
        // A document must not point into the SVG header or its index. This
        // also rejects overlapping index/data ranges that would otherwise be
        // interpreted as XML by the lazy parser.
        if document_start < records_end {
            return Err(SfntError::MalformedTable(SVG_TAG));
        }
        previous_end = Some(end_glyph_id);
        entries.push(SvgDocumentEntry {
            start_glyph_id,
            end_glyph_id,
            document_start,
            document_end,
        });
    }
    Ok(entries)
}

fn parse_document(bytes: &[u8], _units_per_em: u16) -> Option<SvgGlyph> {
    if bytes.is_empty() || bytes.len() > MAX_SVG_DOCUMENT_BYTES || bytes.starts_with(&[0x1f, 0x8b])
    {
        return None;
    }
    let scene = parse_svg(bytes).ok()?;
    let mut paths = Vec::new();
    let mut command_count = 0_usize;
    for node in scene.nodes.iter() {
        if node.element != SvgElementKind::Path || !node.visible {
            continue;
        }
        if node.stroke.is_some() {
            return None;
        }
        let Some(fill) = node.fill.as_ref() else {
            continue;
        };
        let alpha = scaled_alpha(fill.color.a * node.opacity)?;
        if alpha == 0 {
            continue;
        }
        let geometry = scene.geometry(node)?;
        let mut commands = Vec::with_capacity(geometry.commands.len());
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for command in geometry.commands.iter().copied() {
            let command = map_command(command, node.transform)?;
            include_command_bounds(command, &mut bounds);
            commands.push(command);
            command_count = command_count.checked_add(1)?;
            if command_count > MAX_SVG_COMMANDS {
                return None;
            }
        }
        if paths.len() >= MAX_SVG_PATHS {
            return None;
        }
        if !bounds.iter().all(|value| value.is_finite())
            || bounds[0] > bounds[2]
            || bounds[1] > bounds[3]
        {
            return None;
        }
        paths.push(SvgPath {
            commands: commands.into(),
            color: ColorRgba::new(
                (fill.color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
                (fill.color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
                (fill.color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
                alpha,
            ),
            fill_rule: fill.rule,
            bounds,
        });
    }
    if paths.is_empty() {
        return None;
    }

    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for path in &paths {
        bounds[0] = bounds[0].min(path.bounds[0]);
        bounds[1] = bounds[1].min(path.bounds[1]);
        bounds[2] = bounds[2].max(path.bounds[2]);
        bounds[3] = bounds[3].max(path.bounds[3]);
    }
    if !bounds.iter().all(|value| value.is_finite())
        || bounds[0] > bounds[2]
        || bounds[1] > bounds[3]
    {
        return None;
    }
    Some(SvgGlyph {
        paths: paths.into(),
        bounds,
    })
}

fn map_command(command: SvgPathCommand, transform: SvgTransform) -> Option<SvgPathCommand> {
    let point = |x: f32, y: f32| {
        let (x, y) = transform.transform_point(x, y);
        (x.is_finite() && y.is_finite()).then_some((x, -y))
    };
    Some(match command {
        SvgPathCommand::MoveTo { x, y } => {
            let (x, y) = point(x, y)?;
            SvgPathCommand::MoveTo { x, y }
        }
        SvgPathCommand::LineTo { x, y } => {
            let (x, y) = point(x, y)?;
            SvgPathCommand::LineTo { x, y }
        }
        SvgPathCommand::QuadraticTo {
            control_x,
            control_y,
            x,
            y,
        } => {
            let (control_x, control_y) = point(control_x, control_y)?;
            let (x, y) = point(x, y)?;
            SvgPathCommand::QuadraticTo {
                control_x,
                control_y,
                x,
                y,
            }
        }
        SvgPathCommand::CubicTo {
            control1_x,
            control1_y,
            control2_x,
            control2_y,
            x,
            y,
        } => {
            let (control1_x, control1_y) = point(control1_x, control1_y)?;
            let (control2_x, control2_y) = point(control2_x, control2_y)?;
            let (x, y) = point(x, y)?;
            SvgPathCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            }
        }
        SvgPathCommand::Close => SvgPathCommand::Close,
    })
}

fn include_command_bounds(command: SvgPathCommand, bounds: &mut [f32; 4]) {
    let mut include = |x: f32, y: f32| {
        bounds[0] = bounds[0].min(x);
        bounds[1] = bounds[1].min(y);
        bounds[2] = bounds[2].max(x);
        bounds[3] = bounds[3].max(y);
    };
    match command {
        SvgPathCommand::MoveTo { x, y } | SvgPathCommand::LineTo { x, y } => include(x, y),
        SvgPathCommand::QuadraticTo {
            control_x,
            control_y,
            x,
            y,
        } => {
            include(control_x, control_y);
            include(x, y);
        }
        SvgPathCommand::CubicTo {
            control1_x,
            control1_y,
            control2_x,
            control2_y,
            x,
            y,
        } => {
            include(control1_x, control1_y);
            include(control2_x, control2_y);
            include(x, y);
        }
        SvgPathCommand::Close => {}
    }
}

fn scaled_alpha(opacity: f32) -> Option<u8> {
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return None;
    }
    Some((opacity * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::super::{FontMetrics, SfntError};
    use super::{MAX_SVG_DOCUMENT_BYTES, parse_document, parse_index};

    fn metrics(num_glyphs: u16) -> FontMetrics {
        FontMetrics {
            units_per_em: 1000,
            ascender: 800,
            descender: -200,
            line_gap: 0,
            cap_height: 0,
            x_min: 0,
            y_min: -200,
            x_max: 1000,
            y_max: 800,
            num_glyphs,
            number_of_h_metrics: num_glyphs,
            index_to_loc_format: 0,
        }
    }

    fn svg_table(start_glyph_id: u16, document: &[u8]) -> Vec<u8> {
        let index_start = 10_usize;
        let document_start = 10 + 2 + 12;
        let mut table = vec![0_u8; document_start + document.len()];
        table[2..6].copy_from_slice(&(index_start as u32).to_be_bytes());
        table[index_start..index_start + 2].copy_from_slice(&1_u16.to_be_bytes());
        table[index_start + 2..index_start + 4].copy_from_slice(&start_glyph_id.to_be_bytes());
        table[index_start + 4..index_start + 6].copy_from_slice(&start_glyph_id.to_be_bytes());
        table[index_start + 6..index_start + 10]
            .copy_from_slice(&(document_start as u32).to_be_bytes());
        table[index_start + 10..index_start + 14]
            .copy_from_slice(&(document.len() as u32).to_be_bytes());
        table[document_start..].copy_from_slice(document);
        table
    }

    #[test]
    fn parses_checked_svg_document_index() {
        let table = svg_table(
            7,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0L10 0L10 -10Z"/></svg>"#,
        );
        let entries = parse_index(&table, metrics(8)).expect("SVG index should parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].start_glyph_id, 7);
        assert_eq!(entries[0].end_glyph_id, 7);
    }

    #[test]
    fn rejects_svg_index_that_points_into_its_own_records() {
        let mut table = svg_table(1, b"<svg/>");
        table[16..20].copy_from_slice(&10_u32.to_be_bytes());
        assert!(matches!(
            parse_index(&table, metrics(2)),
            Err(SfntError::MalformedTable(_))
        ));
    }

    #[test]
    fn parses_solid_path_and_mirrors_svg_y_axis() {
        let glyph = parse_document(
            br##"<svg xmlns="http://www.w3.org/2000/svg"><path fill="#12aBcC" d="M0 -10L20 -10L20 0Z"/></svg>"##,
            1000,
        )
        .expect("solid SVG path should parse");
        assert_eq!(glyph.paths.len(), 1);
        assert_eq!(glyph.paths[0].color.alpha, 255);
        assert_eq!(glyph.bounds, [0.0, -0.0, 20.0, 10.0]);
    }

    #[test]
    fn unsupported_svg_paint_falls_back_without_panicking() {
        let glyph = parse_document(
            br#"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs><path fill="url(#g)" d="M0 0L10 0L10 -10Z"/></svg>"#,
            1000,
        );
        assert!(glyph.is_none());
    }

    #[test]
    fn malformed_svg_documents_fall_back_without_panicking() {
        assert!(parse_document(b"<svg><path", 1000).is_none());
    }

    #[test]
    fn oversized_svg_documents_are_rejected_before_parsing() {
        let document = vec![b' '; MAX_SVG_DOCUMENT_BYTES + 1];
        assert!(parse_document(&document, 1000).is_none());
    }
}
