use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use aimer_lucide::Icons;
use aimer_svg::SvgDocument;
use aimer_cupid::svg::SvgPathCommand;

fn coordinates(command: &SvgPathCommand) -> [f32; 6] {
    match *command {
        SvgPathCommand::MoveTo { x, y } | SvgPathCommand::LineTo { x, y } => [x, y, 0.0, 0.0, 0.0, 0.0],
        SvgPathCommand::QuadraticTo { control_x, control_y, x, y } => [control_x, control_y, x, y, 0.0, 0.0],
        SvgPathCommand::CubicTo { control1_x, control1_y, control2_x, control2_y, x, y } =>
            [control1_x, control1_y, control2_x, control2_y, x, y],
        SvgPathCommand::Close => [0.0; 6],
    }
}

#[test]
fn catalog_covers_every_source_icon_once_and_shares_documents() {
    let sources = std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("icons")).unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "svg"))
        .map(|path| path.file_stem().unwrap().to_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    let names = Icons::ALL.iter().map(|(name, _)| (*name).to_owned()).collect::<BTreeSet<_>>();
    assert_eq!(names, sources);
    assert_eq!(Icons::ALL.len(), names.len());
    assert!(Icons::ALL.windows(2).all(|pair| pair[0].0 < pair[1].0));
    let first = Icons::HOUSE.document();
    assert!(Arc::ptr_eq(first.scene(), Icons::HOUSE.document().scene()));
    assert_eq!(Icons::HOUSE.source(), include_str!("../icons/house.svg"));
}

#[test]
fn every_icon_matches_runtime_svg_parsing() {
    let mut maximum_error = 0.0_f32;
    for (name, icon) in Icons::ALL {
        let expected = SvgDocument::from_svg(icon.source()).unwrap_or_else(|error| panic!("{name}: {error}"));
        let actual = icon.document();
        assert_eq!(actual.view_box(), expected.view_box(), "{name}");
        assert_eq!(actual.preserve_aspect_ratio(), expected.preserve_aspect_ratio(), "{name}");
        assert_eq!(actual.fit_policy(), expected.fit_policy(), "{name}");
        assert_eq!(actual.resources(), expected.resources(), "{name}");
        assert_eq!(actual.diagnostics(), expected.diagnostics(), "{name}");
        assert_eq!(actual.scene().viewport, expected.scene().viewport, "{name}");
        assert_eq!(actual.scene().geometries.len(), expected.scene().geometries.len(), "{name}");
        for (actual, expected) in actual.scene().geometries.iter().zip(expected.scene().geometries.iter()) {
            assert_eq!(actual.commands.len(), expected.commands.len(), "{name}");
            for (left, right) in actual.commands.iter().zip(expected.commands.iter()) {
                assert_eq!(std::mem::discriminant(left), std::mem::discriminant(right), "{name}");
                for (left, right) in coordinates(left).into_iter().zip(coordinates(right)) {
                    let error = (left - right).abs();
                    maximum_error = maximum_error.max(error);
                    // Host build scripts and optimized target code can round
                    // arc trigonometry differently. This is 0.0001 SVG units,
                    // far below a display pixel at ordinary icon sizes.
                    assert!(error <= 0.0001, "{name}: coordinate {left} differs from {right}");
                }
            }
        }
        assert_eq!(actual.scene().nodes.len(), expected.scene().nodes.len(), "{name}");
        for (left, right) in actual.scene().nodes.iter().zip(expected.scene().nodes.iter()) {
            assert_eq!(left.node_id, right.node_id, "{name}");
            assert_eq!(left.geometry, right.geometry, "{name}");
            assert_eq!(left.fill, right.fill, "{name}");
            assert_eq!(left.stroke, right.stroke, "{name}");
            assert_eq!(left.transform, right.transform, "{name}");
            assert_eq!(left.opacity, right.opacity, "{name}");
            assert_eq!(left.visible, right.visible, "{name}");
            assert_eq!(left.fill_rule, right.fill_rule, "{name}");
            assert_eq!(left.fill_paint, right.fill_paint, "{name}");
            assert_eq!(left.stroke_paint, right.stroke_paint, "{name}");
            let actual_paint = actual.paint_for(left.node_id).unwrap();
            let expected_paint = expected.paint_for(right.node_id).unwrap();
            assert_eq!(actual_paint.fill, expected_paint.fill, "{name}");
            assert_eq!(actual_paint.stroke, expected_paint.stroke, "{name}");
        }
    }
    println!("maximum build-time/runtime coordinate difference: {maximum_error}");
}
