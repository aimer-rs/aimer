//! The shape of a selection that spans lines: one outline around all of it, with rounded corners where the
//! lines meet, as editors draw it, and not a rectangle for each line.
//!
//! Each line of the selection is a [`Row`], and rows that touch are one shape. The outline goes down the
//! right side of the rows and up the left side, and each corner of it is rounded: a corner that points out is
//! cut, and one that points in, where a line is shorter or longer than the one before it, is filled, so
//! that the lines run into each other. A corner is rounded by no more than half of the edges on either side
//! of it, so a short step or a short line is still a curve and never a loop.

use aimer_cupid::svg::{
    SvgColor, SvgElementKind, SvgFill, SvgFillRule, SvgGeometry, SvgNode, SvgNodeId, SvgPaintOrder, SvgPathCommand, SvgResourceGraph,
    SvgScene, SvgTransform, SvgViewport,
};
use std::sync::Arc;

/// One line of a selection, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Row {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// How far apart, in pixels, two numbers can be and still be the same place: the bottom of a row and the top of
/// the next one are not exactly one number once a layout has scaled them.
const SAME_PLACE: f32 = 0.5;

/// The outline of `rows`, which go from the top down, as the commands of a path: a closed shape for each run of rows
/// that touch, with its corners rounded by `radius` where the edges are long enough.
pub(crate) fn outline(rows: &[Row], radius: f32) -> Vec<SvgPathCommand> {
    let mut path = Vec::new();
    let mut start = 0;
    for end in 1..=rows.len() {
        if end == rows.len() || (rows[end].top - rows[end - 1].bottom).abs() > SAME_PLACE {
            if let Some(corners) = corners_of(&rows[start..end]) {
                round(&corners, radius, &mut path);
            }
            start = end;
        }
    }
    path
}

/// The corners of the shape of `rows`, which touch, clockwise from the top left, with none that lies on the line
/// between its neighbours. `None` for rows that have no area.
fn corners_of(rows: &[Row]) -> Option<Vec<(f32, f32)>> {
    let (first, last) = (rows.first()?, rows.last()?);
    let mut corners = Vec::with_capacity(rows.len() * 4);
    // Down the right side: the edge of each row, then across to the edge of the next.
    corners.push((first.left, first.top));
    for (above, below) in rows.iter().zip(rows.iter().skip(1)) {
        corners.push((above.right, above.top));
        corners.push((above.right, below.top));
        corners.push((below.right, below.top));
    }
    corners.push((last.right, last.top));
    corners.push((last.right, last.bottom));
    // Up the left side.
    corners.push((last.left, last.bottom));
    for (above, below) in rows.iter().zip(rows.iter().skip(1)).rev() {
        corners.push((below.left, below.top));
        corners.push((above.left, below.top));
    }
    Some(simplified(corners)).filter(|corners| corners.len() >= 3)
}

/// `corners` without a corner that is where the one before it already is, nor one that is on the line between the
/// ones around it, which is not a corner to round.
fn simplified(corners: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    let same = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01;
    let mut distinct: Vec<(f32, f32)> = Vec::with_capacity(corners.len());
    for corner in corners {
        if !distinct.last().is_some_and(|last| same(*last, corner)) {
            distinct.push(corner);
        }
    }
    while distinct.len() > 1 && same(distinct[0], distinct[distinct.len() - 1]) {
        distinct.pop();
    }
    // The outline only turns at right angles, so a corner is on a line when its neighbours are on one axis with it.
    let straight = |before: (f32, f32), at: (f32, f32), after: (f32, f32)| {
        ((before.0 - at.0).abs() < 0.01 && (at.0 - after.0).abs() < 0.01) || ((before.1 - at.1).abs() < 0.01 && (at.1 - after.1).abs() < 0.01)
    };
    let count = distinct.len();
    let kept: Vec<(f32, f32)> = (0..count)
        .filter(|&at| !straight(distinct[(at + count - 1) % count], distinct[at], distinct[(at + 1) % count]))
        .map(|at| distinct[at])
        .collect();
    kept
}

/// Appends to `path` the closed shape of `corners`, each rounded by `radius`, or by less where an edge beside it is
/// shorter than twice that.
fn round(corners: &[(f32, f32)], radius: f32, path: &mut Vec<SvgPathCommand>) {
    let count = corners.len();
    let toward = |from: (f32, f32), to: (f32, f32), by: f32| {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length = dx.hypot(dy);
        if length <= f32::EPSILON { from } else { (from.0 + dx / length * by, from.1 + dy / length * by) }
    };
    let length = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).hypot(a.1 - b.1);
    for (at, &corner) in corners.iter().enumerate() {
        let (before, after) = (corners[(at + count - 1) % count], corners[(at + 1) % count]);
        let rounding = radius.max(0.0).min(length(before, corner) / 2.0).min(length(corner, after) / 2.0);
        let (from, to) = (toward(corner, before, rounding), toward(corner, after, rounding));
        if at == 0 {
            path.push(SvgPathCommand::MoveTo { x: from.0, y: from.1 });
        } else {
            path.push(SvgPathCommand::LineTo { x: from.0, y: from.1 });
        }
        if rounding > 0.0 {
            path.push(SvgPathCommand::QuadraticTo { control_x: corner.0, control_y: corner.1, x: to.0, y: to.1 });
        }
    }
    path.push(SvgPathCommand::Close);
}

/// The rows of the rectangles a layout gives for a selection, as `(x, y, width, height)` in physical pixels at `scale`, in
/// logical pixels. A line that is selected to its end goes on to the next, so its break is selected too, and it is drawn
/// reaching past the end of its text by `reach` of its height: that is what makes a blank line in a selection show, and what
/// shows that the selection runs on. The last line does it only when `breaks_last`, the selection ending after a break.
///
/// `None` where the rectangles are not one to a line, in order down: a line of text that runs both ways has two on it,
/// and what is not one span is not drawn as one shape.
pub(crate) fn rows_of(rects: &[(f32, f32, f32, f32)], scale: f32, reach: f32, breaks_last: bool) -> Option<Vec<Row>> {
    let mut rows: Vec<Row> = Vec::with_capacity(rects.len());
    for &(x, y, width, height) in rects {
        let row = Row { left: x / scale, right: (x + width) / scale, top: y / scale, bottom: (y + height) / scale };
        if rows.last().is_some_and(|last| row.top < last.bottom - SAME_PLACE) {
            return None;
        }
        rows.push(row);
    }
    let count = rows.len();
    for (at, row) in rows.iter_mut().enumerate() {
        if at + 1 < count || breaks_last {
            row.right += (row.bottom - row.top) * reach;
        }
    }
    Some(rows)
}

/// The rows that are within `margin` rows of what shows, from `top` down to `bottom` in logical pixels: the shape of the
/// whole selection is the same wherever it is cut as long as the cut is out of sight, and a selection of a whole file is
/// drawn for the screen it is on and not for its length. The rows are in order, so this is found by search.
pub(crate) fn visible(rows: &[Row], top: f32, bottom: f32, margin: usize) -> &[Row] {
    let first = rows.partition_point(|row| row.bottom <= top);
    let last = rows.partition_point(|row| row.top < bottom);
    if first >= last {
        return &[];
    }
    &rows[first.saturating_sub(margin)..(last + margin).min(rows.len())]
}

/// A scene of one shape, filled with `color`, that fits `width` by `height`. Its coordinates are the pixels of the box, so
/// drawing it into a rectangle of that size draws it as it is.
pub(crate) fn scene(path: Vec<SvgPathCommand>, width: f32, height: f32, color: SvgColor) -> SvgScene {
    let node = SvgNode {
        node_id: SvgNodeId(0),
        svg_id: None,
        classes: Arc::from([]),
        element: SvgElementKind::Path,
        is_definition: false,
        definition_owner: None,
        clip_path: None,
        mask: None,
        filter: None,
        parent: None,
        children: Arc::from([]),
        transform: SvgTransform::default(),
        opacity: 1.0,
        geometry: Some(0),
        fill_rule: SvgFillRule::NonZero,
        fill: Some(SvgFill { color, rule: SvgFillRule::NonZero }),
        stroke: None,
        fill_paint: None,
        stroke_paint: None,
        paint_order: SvgPaintOrder::FillAndStroke,
        visible: true,
    };
    SvgScene {
        viewport: SvgViewport { width, height },
        nodes: Arc::from([node]),
        geometries: Arc::from([SvgGeometry { commands: path.into() }]),
        resources: Arc::new(SvgResourceGraph::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(left: f32, right: f32, line: usize) -> Row {
        Row { left, right, top: line as f32 * 20.0, bottom: (line + 1) as f32 * 20.0 }
    }

    /// The corners the path rounds: the control point of each of its curves.
    fn controls(path: &[SvgPathCommand]) -> Vec<(f32, f32)> {
        path.iter()
            .filter_map(|command| match command {
                SvgPathCommand::QuadraticTo { control_x, control_y, .. } => Some((*control_x, *control_y)),
                _ => None,
            })
            .collect()
    }

    /// The points the path goes through, with each curve cut into `steps` pieces, one closed shape at a time.
    fn flattened(path: &[SvgPathCommand]) -> Vec<Vec<(f32, f32)>> {
        let mut shapes: Vec<Vec<(f32, f32)>> = Vec::new();
        let mut at = (0.0, 0.0);
        for command in path {
            match *command {
                SvgPathCommand::MoveTo { x, y } => {
                    shapes.push(vec![(x, y)]);
                    at = (x, y);
                }
                SvgPathCommand::LineTo { x, y } => {
                    shapes.last_mut().expect("a shape was begun").push((x, y));
                    at = (x, y);
                }
                SvgPathCommand::QuadraticTo { control_x, control_y, x, y } => {
                    let shape = shapes.last_mut().expect("a shape was begun");
                    for step in 1..=8 {
                        let t = step as f32 / 8.0;
                        let (a, b) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t));
                        shape.push((a * at.0 + b * control_x + t * t * x, a * at.1 + b * control_y + t * t * y));
                    }
                    at = (x, y);
                }
                SvgPathCommand::CubicTo { .. } => unreachable!("the outline has no cubic curves"),
                SvgPathCommand::Close => {}
            }
        }
        shapes
    }

    fn area(shape: &[(f32, f32)]) -> f32 {
        let twice: f32 = shape.iter().zip(shape.iter().cycle().skip(1)).map(|(a, b)| a.0 * b.1 - b.0 * a.1).sum();
        twice.abs() / 2.0
    }

    #[test]
    fn rows_that_do_not_exist_have_no_outline() {
        assert!(outline(&[], 4.0).is_empty());
    }

    #[test]
    fn one_row_is_a_rectangle_with_four_rounded_corners() {
        let path = outline(&[row(10.0, 110.0, 0)], 4.0);
        assert_eq!(controls(&path), [(10.0, 0.0), (110.0, 0.0), (110.0, 20.0), (10.0, 20.0)]);
        let shapes = flattened(&path);
        assert_eq!(shapes.len(), 1);
        // The corners are cut, so it covers a little less than the rectangle, and not much less.
        let covered = area(&shapes[0]);
        assert!(covered < 2000.0 && covered > 2000.0 - 4.0 * 16.0, "{covered}");
    }

    #[test]
    fn no_radius_is_the_plain_rectangle() {
        let path = outline(&[row(10.0, 110.0, 0)], 0.0);
        assert!(controls(&path).is_empty(), "nothing is curved");
        assert_eq!(area(&flattened(&path)[0]), 2000.0);
    }

    #[test]
    fn lines_that_end_at_the_same_place_are_one_edge_and_not_a_corner() {
        let path = outline(&[row(0.0, 100.0, 0), row(0.0, 100.0, 1), row(0.0, 100.0, 2)], 4.0);
        assert_eq!(controls(&path), [(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)], "four corners and no seams");
    }

    #[test]
    fn a_shorter_line_under_a_longer_one_has_a_corner_that_points_out_and_one_that_points_in() {
        let path = outline(&[row(0.0, 100.0, 0), row(0.0, 40.0, 1)], 4.0);
        assert_eq!(
            controls(&path),
            [(0.0, 0.0), (100.0, 0.0), (100.0, 20.0), (40.0, 20.0), (40.0, 40.0), (0.0, 40.0)],
            "the step is two corners: the longer line's, which points out, and the shorter one's, which points in"
        );
        // It is the two rows and less what the cut corners take, and the filled one gives back.
        let covered = area(&flattened(&path)[0]);
        assert!((covered - (100.0 * 20.0 + 40.0 * 20.0)).abs() < 40.0, "{covered}");
    }

    #[test]
    fn a_corner_is_rounded_by_no_more_than_half_of_the_edge_beside_it() {
        // The step is two pixels, so no corner of it can be rounded by more than one.
        let path = outline(&[row(0.0, 100.0, 0), row(0.0, 98.0, 1)], 8.0);
        let mut previous = (0.0, 0.0);
        for command in &path {
            match *command {
                SvgPathCommand::MoveTo { x, y } | SvgPathCommand::LineTo { x, y } => previous = (x, y),
                SvgPathCommand::QuadraticTo { control_x, control_y, x, y } => {
                    // Where the curve begins is no further from its corner than where it ends is.
                    let (began, ended) = ((previous.0 - control_x).hypot(previous.1 - control_y), (x - control_x).hypot(y - control_y));
                    assert!(began <= 8.0 + 0.001 && ended <= 8.0 + 0.001);
                    if (control_x, control_y) == (100.0, 20.0) || (control_x, control_y) == (98.0, 20.0) {
                        assert!(began <= 1.001 && ended <= 1.001, "the step: {began} and {ended}");
                    }
                    previous = (x, y);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn a_short_line_between_long_ones_is_a_notch_and_a_blank_line_is_a_stub() {
        // The stub of a line break that is all of the middle line, as when a blank line is selected.
        let path = outline(&[row(0.0, 100.0, 0), row(0.0, 8.0, 1), row(0.0, 100.0, 2)], 4.0);
        assert_eq!(
            controls(&path),
            [(0.0, 0.0), (100.0, 0.0), (100.0, 20.0), (8.0, 20.0), (8.0, 40.0), (100.0, 40.0), (100.0, 60.0), (0.0, 60.0)]
        );
        assert_eq!(flattened(&path).len(), 1, "the lines are one shape");
    }

    #[test]
    fn rows_that_do_not_touch_are_shapes_of_their_own() {
        let far = Row { left: 0.0, right: 50.0, top: 100.0, bottom: 120.0 };
        let path = outline(&[row(0.0, 100.0, 0), far], 4.0);
        assert_eq!(flattened(&path).len(), 2);
    }

    #[test]
    fn a_line_that_starts_further_in_is_a_corner_on_the_left_too() {
        // The first line of a selection that begins in the middle of it.
        let path = outline(&[row(60.0, 100.0, 0), row(0.0, 100.0, 1)], 4.0);
        assert_eq!(controls(&path), [(60.0, 0.0), (100.0, 0.0), (100.0, 40.0), (0.0, 40.0), (0.0, 20.0), (60.0, 20.0)]);
    }

    /// A rectangle as a layout gives it, in physical pixels at `scale` 2: 20 logical pixels tall.
    fn rect(x: f32, line: usize, width: f32) -> (f32, f32, f32, f32) {
        (x, line as f32 * 40.0, width, 40.0)
    }

    #[test]
    fn the_rows_are_the_rectangles_in_logical_pixels_and_every_line_but_the_last_reaches_past_its_end() {
        let rects = [rect(20.0, 0, 200.0), rect(20.0, 1, 2.0), rect(20.0, 2, 100.0)];
        let rows = rows_of(&rects, 2.0, 0.4, false).expect("one rectangle to a line");
        // The reach is a share of the height of the line: 0.4 of 20.
        assert_eq!(rows[0], Row { left: 10.0, right: 118.0, top: 0.0, bottom: 20.0 });
        assert_eq!(rows[1], Row { left: 10.0, right: 19.0, top: 20.0, bottom: 40.0 }, "a blank line is a stub: its break, and the reach");
        assert_eq!(rows[2], Row { left: 10.0, right: 60.0, top: 40.0, bottom: 60.0 }, "the last line ends where the selection does");
    }

    #[test]
    fn the_last_line_reaches_too_when_the_selection_ends_after_its_line_break() {
        let rows = rows_of(&[rect(0.0, 0, 200.0)], 2.0, 0.4, true).expect("one rectangle");
        assert_eq!(rows[0].right, 108.0);
    }

    #[test]
    fn lines_with_more_than_one_rectangle_or_out_of_order_are_not_made_into_rows() {
        // Text that runs both ways has pieces on one line, which are not one span.
        assert_eq!(rows_of(&[rect(0.0, 0, 50.0), rect(100.0, 0, 50.0)], 2.0, 0.4, false), None);
        assert_eq!(rows_of(&[rect(0.0, 1, 50.0), rect(0.0, 0, 50.0)], 2.0, 0.4, false), None);
        assert_eq!(rows_of(&[], 2.0, 0.4, false), Some(Vec::new()));
    }

    #[test]
    fn only_the_rows_near_what_shows_are_kept_so_a_selection_of_a_whole_file_costs_the_screen() {
        let rows: Vec<Row> = (0..1000).map(|line| row(0.0, 100.0, line)).collect();
        // What shows is 400 to 600 pixels down: lines 20 to 30, and two lines either side that the outline needs to close on.
        let kept = visible(&rows, 400.0, 600.0, 2);
        assert_eq!((kept.first().map(|row| row.top), kept.last().map(|row| row.bottom)), (Some(360.0), Some(640.0)));
        assert!(visible(&rows, -100.0, -10.0, 2).is_empty(), "above the first row: nothing shows, so nothing is drawn");
        assert_eq!(visible(&rows, -100.0, 10.0, 2).len(), 3, "the first row shows, and two more for the outline to close on");
        assert!(visible(&rows, 50_000.0, 60_000.0, 2).is_empty());
    }

    #[test]
    fn the_scene_is_one_filled_shape_in_the_box_it_is_given() {
        let path = outline(&[row(0.0, 100.0, 0)], 4.0);
        let scene = scene(path, 100.0, 20.0, SvgColor::rgba8(240, 100, 178, 90));
        assert_eq!((scene.viewport.width, scene.viewport.height), (100.0, 20.0));
        assert_eq!(scene.nodes.len(), 1);
        let node = &scene.nodes[0];
        assert!(node.stroke.is_none() && node.visible);
        assert_eq!(node.fill.as_ref().expect("filled").color, SvgColor::rgba8(240, 100, 178, 90));
        assert!(scene.geometry(node).is_some());
    }
}
