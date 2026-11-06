use crate::geometry::{bezier, ellipse, rect};
use crate::model::Bezier;
use aimer_cupid::svg::SvgPathCommand::{self, Close, CubicTo, LineTo, MoveTo};

fn out(f: impl FnOnce(&mut Vec<SvgPathCommand>)) -> Vec<SvgPathCommand> {
    let mut v = Vec::new();
    f(&mut v);
    v
}

/// Every coordinate a command list touches, controls included.
fn coords(cmds: &[SvgPathCommand]) -> Vec<(f32, f32)> {
    cmds.iter()
        .flat_map(|c| match *c {
            MoveTo { x, y } | LineTo { x, y } => vec![(x, y)],
            SvgPathCommand::QuadraticTo { control_x, control_y, x, y } => {
                vec![(control_x, control_y), (x, y)]
            }
            CubicTo { control1_x, control1_y, control2_x, control2_y, x, y } => {
                vec![(control1_x, control1_y), (control2_x, control2_y), (x, y)]
            }
            Close => vec![],
        })
        .collect()
}

fn bounds(cmds: &[SvgPathCommand]) -> (f32, f32, f32, f32) {
    coords(cmds).into_iter().fold(
        (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
        |(x0, y0, x1, y1), (x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
    )
}

#[test]
fn plain_rect_is_four_lines_around_its_center() {
    assert_eq!(
        out(|o| rect([10.0, 20.0], [20.0, 10.0], 0.0, o)),
        [
            MoveTo { x: 0.0, y: 15.0 },
            LineTo { x: 20.0, y: 15.0 },
            LineTo { x: 20.0, y: 25.0 },
            LineTo { x: 0.0, y: 25.0 },
            Close,
        ]
    );
}

#[test]
fn rounded_rect_stays_inside_its_box_and_has_four_arcs() {
    let cmds = out(|o| rect([0.0, 0.0], [40.0, 20.0], 6.0, o));
    assert_eq!(cmds.iter().filter(|c| matches!(c, CubicTo { .. })).count(), 4);
    assert_eq!(bounds(&cmds), (-20.0, -10.0, 20.0, 10.0));
    assert_eq!(cmds.last(), Some(&Close));
}

#[test]
fn roundness_is_clamped_to_half_the_short_side() {
    let big = out(|o| rect([0.0, 0.0], [40.0, 20.0], 500.0, o));
    assert_eq!(bounds(&big), (-20.0, -10.0, 20.0, 10.0));
    let negative = out(|o| rect([0.0, 0.0], [40.0, 20.0], -3.0, o));
    assert_eq!(negative.len(), 5, "a negative radius is a plain rectangle");
}

#[test]
fn degenerate_rects_do_not_produce_nan() {
    for size in [[0.0, 0.0], [10.0, 0.0], [-4.0, 8.0]] {
        let cmds = out(|o| rect([0.0, 0.0], size, 2.0, o));
        assert!(coords(&cmds).iter().all(|(x, y)| x.is_finite() && y.is_finite()), "{size:?}");
    }
}

#[test]
fn ellipse_touches_its_box_with_four_arcs() {
    let cmds = out(|o| ellipse([5.0, 5.0], [20.0, 10.0], o));
    assert_eq!(cmds.iter().filter(|c| matches!(c, CubicTo { .. })).count(), 4);
    assert_eq!(cmds[0], MoveTo { x: 15.0, y: 5.0 });
    assert_eq!(bounds(&cmds), (-5.0, 0.0, 15.0, 10.0));
    assert_eq!(cmds.last(), Some(&Close));
}

fn path(closed: bool, v: &[[f32; 2]], i: &[[f32; 2]], o: &[[f32; 2]]) -> Bezier {
    Bezier { closed, vertices: v.to_vec(), in_tangents: i.to_vec(), out_tangents: o.to_vec() }
}

#[test]
fn straight_segments_become_lines() {
    let z = [[0.0; 2]; 3];
    let cmds = out(|o| bezier(&path(false, &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], &z, &z), o));
    assert_eq!(
        cmds,
        [MoveTo { x: 0.0, y: 0.0 }, LineTo { x: 10.0, y: 0.0 }, LineTo { x: 10.0, y: 10.0 }]
    );
}

#[test]
fn closed_path_adds_the_closing_segment() {
    let z = [[0.0; 2]; 3];
    let cmds = out(|o| bezier(&path(true, &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], &z, &z), o));
    assert_eq!(cmds.len(), 5, "move, two lines, closing line, close: {cmds:?}");
    assert_eq!(cmds.last(), Some(&Close));
}

#[test]
fn tangents_are_relative_to_their_vertex() {
    let cmds = out(|o| {
        bezier(
            &path(
                false,
                &[[0.0, 0.0], [10.0, 10.0]],
                &[[0.0, 0.0], [-3.0, 0.0]],
                &[[4.0, 0.0], [0.0, 0.0]],
            ),
            o,
        )
    });
    assert_eq!(
        cmds[1],
        CubicTo {
            control1_x: 4.0,
            control1_y: 0.0,
            control2_x: 7.0,
            control2_y: 10.0,
            x: 10.0,
            y: 10.0,
        }
    );
}

#[test]
fn empty_and_single_vertex_paths_draw_nothing_harmful() {
    assert!(out(|o| bezier(&path(true, &[], &[], &[]), o)).is_empty());
    let one = out(|o| bezier(&path(true, &[[1.0, 2.0]], &[[0.0; 2]], &[[0.0; 2]]), o));
    assert!(one.len() <= 2, "{one:?}");
}
