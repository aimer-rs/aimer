use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

use aimer_cupid::svg::*;

fn identifier(name: &str) -> Result<String, String> {
    let result = name.replace('-', "_").to_ascii_uppercase();
    if result.is_empty() || result.starts_with(|c: char| c.is_ascii_digit())
        || !result.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(format!("invalid icon filename: {name}"));
    }
    Ok(result)
}

fn names<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Vec<(&'a str, String)>, String> {
    let mut seen = BTreeSet::new();
    names.into_iter().map(|name| {
        let id = identifier(name)?;
        if !seen.insert(id.clone()) || id == "ALL" {
            return Err(format!("icon constant collision: {name} becomes {id}"));
        }
        Ok((name, id))
    }).collect()
}

fn parse(source: &str) -> Result<SvgParsedDocument, String> {
    let parsed = parse_svg_document(source).map_err(|error| error.to_string())?;
    let resources = &parsed.resources;
    if !parsed.unsupported_elements.is_empty() || !parsed.gradients.is_empty()
        || !resources.clip_paths.is_empty() || !resources.masks.is_empty()
        || !resources.filters.is_empty() || !resources.patterns.is_empty()
        || !resources.gradients.is_empty()
    {
        return Err("precompiled icons require solid shapes without resources or skipped elements".into());
    }
    for node in parsed.scene.nodes.iter() {
        if node.element != SvgElementKind::Path || node.geometry.is_none()
            || node.parent.is_some() || !node.children.is_empty()
            || node.svg_id.is_some() || !node.classes.is_empty() || node.is_definition
            || node.definition_owner.is_some() || node.clip_path.is_some()
            || node.mask.is_some() || node.filter.is_some()
            || node.paint_order != SvgPaintOrder::FillAndStroke
            || node.stroke.as_ref().is_some_and(|stroke| !stroke.dash_array.is_empty())
            || node.fill_paint.iter().chain(node.stroke_paint.iter())
                .any(|paint| !matches!(paint, SvgPaint::Solid(_)))
        {
            return Err("precompiled icons require flat, unnamed solid paths".into());
        }
    }
    // The retained parser currently diagnoses rather than implements dashes.
    if source.to_ascii_lowercase().contains("stroke-dasharray") {
        return Err("precompiled icons do not support dashed strokes".into());
    }
    Ok(parsed)
}

fn stroke(stroke: &Option<SvgStroke>) -> String {
    match stroke {
        None => "None".into(),
        Some(stroke) => format!(
            "Some(SvgStroke {{ color: {:?}, width: {:?}, line_cap: SvgLineCap::{:?}, line_join: SvgLineJoin::{:?}, miter_limit: {:?}, dash_array: Arc::from([]), dash_offset: {:?} }})",
            stroke.color, stroke.width, stroke.line_cap, stroke.line_join,
            stroke.miter_limit, stroke.dash_offset,
        ),
    }
}

fn paint(paint: &Option<SvgPaint>) -> String {
    match paint {
        None => "None".into(),
        Some(SvgPaint::Solid(color)) => format!("Some(SvgPaint::Solid({color:?}))"),
        _ => unreachable!("validated solid paints"),
    }
}

fn fill(fill: &Option<SvgFill>) -> String {
    match fill {
        None => "None".into(),
        Some(fill) => format!("Some(SvgFill {{ color: {:?}, rule: SvgFillRule::{:?} }})", fill.color, fill.rule),
    }
}

fn emit_factory(parsed: &SvgParsedDocument) -> String {
    let mut output = String::from("fn document(source: &'static str) -> aimer_svg::SvgDocument {\n");
    for (index, geometry) in parsed.scene.geometries.iter().enumerate() {
        writeln!(output, "const PATH_{index}: &[SvgPathCommand] = &[").unwrap();
        for command in geometry.commands.iter() {
            writeln!(output, "SvgPathCommand::{command:?},").unwrap();
        }
        output.push_str("];\n");
    }
    output.push_str("let geometries: Arc<[SvgGeometry]> = vec![\n");
    for index in 0..parsed.scene.geometries.len() {
        writeln!(output, "SvgGeometry {{ commands: Arc::from(PATH_{index}) }},").unwrap();
    }
    output.push_str("].into();\nlet nodes: Arc<[SvgNode]> = vec![\n");
    for node in parsed.scene.nodes.iter() {
        writeln!(output, "SvgNode {{ node_id: SvgNodeId({}), geometry: {:?},", node.node_id.0, node.geometry).unwrap();
        output.push_str("svg_id: None, classes: Arc::from([]), element: SvgElementKind::Path,\nis_definition: false, definition_owner: None, clip_path: None, mask: None, filter: None,\nparent: None, children: Arc::from([]),\n");
        writeln!(output, "transform: {:?}, opacity: {:?}, visible: {:?},", node.transform, node.opacity, node.visible).unwrap();
        writeln!(output, "fill_rule: SvgFillRule::{:?}, fill: {}, stroke: {},", node.fill_rule, fill(&node.fill), stroke(&node.stroke)).unwrap();
        writeln!(output, "fill_paint: {}, stroke_paint: {}, paint_order: SvgPaintOrder::FillAndStroke }},", paint(&node.fill_paint), paint(&node.stroke_paint)).unwrap();
    }
    output.push_str("].into();\ncrate::precompiled::document(source, nodes, geometries,\n");
    writeln!(output, "{:?}, {:?}, SvgPreserveAspectRatio {{ align: SvgAspectAlign::{:?}, mode: SvgAspectMode::{:?} }}, {}, {:?})\n}}",
        parsed.scene.viewport, parsed.view_box, parsed.preserve_aspect_ratio.align,
        parsed.preserve_aspect_ratio.mode, parsed.has_view_box, parsed.root_transform).unwrap();
    output
}

pub fn generate(directory: &Path, output: &Path, release: bool) -> Result<(), String> {
    let mut files = std::fs::read_dir(directory).map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path())).collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    files.retain(|path| path.extension().is_some_and(|extension| extension == "svg"));
    files.sort_by(|left, right| left.file_stem().cmp(&right.file_stem()));
    let stems = files.iter().map(|path| path.file_stem().and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("invalid icon filename: {}", path.display())))
        .collect::<Result<Vec<_>, _>>()?;
    let names = names(stems.iter().copied())?;
    let mut facade = String::new();
    let mut shard = String::new();
    let mut shard_index = 0;
    for (path, (name, id)) in files.iter().zip(names.iter()) {
        // Each icon owns its geometry and cache; using one never initializes
        // other documents, even when the caller enumerates the collection.
        let mut item = format!("mod icon_{id} {{\n");
        if release {
            let source = std::fs::read_to_string(path).map_err(|error| format!("{name}: {error}"))?;
            let parsed = parse(&source).map_err(|error| format!("{name}.svg: {error}"))?;
            item.push_str("use aimer_cupid::svg::*;\nuse std::sync::Arc;\n");
            item.push_str(&emit_factory(&parsed));
            writeln!(item, "pub(super) static ICON: aimer_svg::StaticSvg = aimer_svg::StaticSvg::with_precompiled({name:?}, include_str!({:?}), document);", path.to_str().unwrap()).unwrap();
        } else {
            writeln!(item, "pub(super) static ICON: aimer_svg::StaticSvg = aimer_svg::StaticSvg::new({name:?}, include_str!({:?}));", path.to_str().unwrap()).unwrap();
        }
        writeln!(item, "}}\nimpl crate::Icons {{\n/// The `{name}` Lucide icon.\npub const {id}: &'static aimer_svg::StaticSvg = &icon_{id}::ICON;\n}}").unwrap();
        if item.lines().count() > 900 {
            return Err(format!("{name}: generated icon exceeds the source module line limit"));
        }
        if shard.lines().count() + item.lines().count() > 900 {
            write_shard(output, &mut facade, shard_index, &shard)?;
            shard_index += 1;
            shard.clear();
        }
        shard.push_str(&item);
    }
    if !shard.is_empty() {
        write_shard(output, &mut facade, shard_index, &shard)?;
    }
    facade.push_str("impl crate::Icons {\n/// All icons in filename order, paired with their kebab-case labels.\n/// Enumerating this slice does not parse or initialize any icon.\npub const ALL: &'static [(&'static str, &'static aimer_svg::StaticSvg)] = &[\n");
    for chunk in names.chunks(4) {
        for (name, id) in chunk {
            write!(facade, "({name:?}, Self::{id}), ").unwrap();
        }
        facade.push('\n');
    }
    facade.push_str("];\n}\n");
    std::fs::write(output.join("icons.rs"), facade).map_err(|error| error.to_string())
}

fn write_shard(output: &Path, facade: &mut String, index: usize, source: &str) -> Result<(), String> {
    let path = output.join(format!("icons_{index}.rs"));
    writeln!(facade, "#[allow(non_snake_case)]\nmod shard_{index} {{ include!({:?}); }}", path.to_str().unwrap()).unwrap();
    std::fs::write(path, source).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_map_to_constants_and_collisions_are_rejected() {
        assert_eq!(identifier("arrow-down").unwrap(), "ARROW_DOWN");
        assert_eq!(identifier("alarm-clock-2").unwrap(), "ALARM_CLOCK_2");
        assert!(names(["foo-bar", "foo_bar"]).is_err());
        assert!(names(["all"]).is_err());
        for invalid in ["", "123", "not an icon", "a.b", "é"] {
            assert!(identifier(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn malformed_and_unrepresentable_icons_are_rejected() {
        for source in ["", "<svg>", "<svg><path d='M NaN 0'/></svg>",
            "<svg><image href='https://example.com/icon.png'/></svg>",
            "<svg><g><path d='M0 0h1'/></g></svg>",
            "<svg><path id='named' d='M0 0h1'/></svg>",
            "<svg><path d='M0 0h1' stroke-dasharray='2 1'/></svg>"] {
            assert!(parse(source).is_err(), "{source}");
        }
    }
}
