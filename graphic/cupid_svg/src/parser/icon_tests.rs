use super::*;

#[test]
fn inherited_current_color_keeps_paint_opacity_and_css_color() {
    let parsed = parse_svg_document(br#"<svg width="24" height="24" fill="currentColor"
        fill-opacity="0.25" stroke="currentColor" stroke-opacity="0.5">
        <style>.blue { color: blue; }</style>
        <g class="blue"><path d="M0 0h10v10z"/></g>
    </svg>"#).unwrap();
    let node = parsed.scene.nodes.iter().find(|node| node.geometry.is_some()).unwrap();
    assert_eq!(node.fill.as_ref().unwrap().color, SvgColor { r: 0.0, g: 0.0, b: 1.0, a: 0.25 });
    assert_eq!(node.stroke.as_ref().unwrap().color, SvgColor { r: 0.0, g: 0.0, b: 1.0, a: 0.5 });
}

#[test]
fn compact_arc_flags_match_separated_flags() {
    let compact = parse_svg(br#"<svg viewBox="0 0 24 24"><path d="M10 3a41 41 0 000 18"/></svg>"#).unwrap();
    let separated = parse_svg(br#"<svg viewBox="0 0 24 24"><path d="M10 3a41 41 0 0 0 0 18"/></svg>"#).unwrap();
    assert_eq!(compact.geometries[0].commands, separated.geometries[0].commands);
    for flags in ["2 0", "0 2", "-1 0", "0.5 0"] {
        let source = format!("<svg><path d='M1 1A2 2 0 {flags} 5 5'/></svg>");
        assert!(parse_svg(source).is_err(), "{flags}");
    }
}

#[test]
fn rounded_rectangles_preserve_radii_and_validate_boundaries() {
    for (radii, start) in [("rx='2'", (3.0, 2.0)), ("ry='3'", (4.0, 2.0)),
        ("rx='99' ry='99'", (6.0, 2.0))] {
        let source = format!("<svg width='24' height='24'><rect x='1' y='2' width='10' height='8' {radii}/></svg>");
        let scene = parse_svg(source).unwrap();
        let commands = &scene.geometries[0].commands;
        assert_eq!(commands[0], SvgPathCommand::MoveTo { x: start.0, y: start.1 });
        assert_eq!(commands.iter().filter(|c| matches!(c, SvgPathCommand::CubicTo { .. })).count(), 4);
        assert_eq!(commands.last(), Some(&SvgPathCommand::Close));
    }
    for radii in ["", "rx='0'", "rx='2' ry='0'"] {
        let source = format!("<svg width='24' height='24'><rect width='10' height='8' {radii}/></svg>");
        assert_eq!(parse_svg(source).unwrap().geometries[0].commands.len(), 5);
    }
    for radii in ["rx='-1'", "ry='NaN'", "rx='inf'"] {
        let source = format!("<svg width='24' height='24'><rect width='10' height='8' {radii}/></svg>");
        assert!(parse_svg(source).is_err(), "{radii}");
    }
}

#[test]
fn lucide_current_color_inherits_and_resolves_filled_details() {
    let parsed = parse_svg_document(include_bytes!(
        "../../../../crates/aimer_lucide/icons/chart-scatter.svg"
    )).expect("Lucide currentColor is valid SVG");
    let black = SvgColor::rgba8(0, 0, 0, 255);
    assert!(parsed.scene.nodes.iter().all(|node| {
        node.stroke.as_ref().is_some_and(|stroke| stroke.color == black)
    }));
    assert!(parsed.scene.nodes.iter().any(|node| {
        node.fill.as_ref().is_some_and(|fill| fill.color == black)
    }));

    let parsed = parse_svg_document(br##"<svg viewBox="0 0 24 24" color="red"
        fill="currentColor" stroke="currentColor">
        <g color="blue"><path id="blue" d="M0 0h1v1z"/></g>
        <path id="red" d="M2 0h1v1z"/>
        <path id="green" color="green" fill="currentColor" d="M4 0h1v1z"/>
        <path id="fixed" color="blue" fill="#ffffff" d="M6 0h1v1z"/>
    </svg>"##).unwrap();
    for (id, color) in [
        ("blue", SvgColor::rgba8(0, 0, 255, 255)),
        ("red", SvgColor::rgba8(255, 0, 0, 255)),
        ("green", SvgColor::rgba8(0, 128, 0, 255)),
        ("fixed", SvgColor::rgba8(255, 255, 255, 255)),
    ] {
        let node = parsed.scene.nodes.iter().find(|node| node.svg_id.as_deref() == Some(id)).unwrap();
        assert_eq!(node.fill.as_ref().unwrap().color, color, "{id}");
        assert_eq!(parsed.node_paints[&node.node_id].fill, Some(SvgPaint::Solid(color)));
    }
}
