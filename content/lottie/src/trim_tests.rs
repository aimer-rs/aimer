use crate::trim::trim;
use aimer_cupid::svg::SvgPathCommand::{self, Close, CubicTo, LineTo, MoveTo};

fn run(input: &[SvgPathCommand], segments: &[(f32, f32)], simultaneous: bool) -> Vec<SvgPathCommand> {
    let mut seg = [None, None];
    for (slot, s) in seg.iter_mut().zip(segments) {
        *slot = Some(*s);
    }
    let mut out = Vec::new();
    trim(input, seg, simultaneous, &mut out);
    out
}

fn near(a: &[SvgPathCommand], b: &[SvgPathCommand]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| match (*a, *b) {
            (MoveTo { x: ax, y: ay }, MoveTo { x: bx, y: by })
            | (LineTo { x: ax, y: ay }, LineTo { x: bx, y: by }) => {
                (ax - bx).abs() < 1e-3 && (ay - by).abs() < 1e-3
            }
            (Close, Close) => true,
            _ => false,
        })
}

fn line(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<SvgPathCommand> {
    vec![MoveTo { x: x0, y: y0 }, LineTo { x: x1, y: y1 }]
}

fn square() -> Vec<SvgPathCommand> {
    vec![
        MoveTo { x: 0.0, y: 0.0 },
        LineTo { x: 10.0, y: 0.0 },
        LineTo { x: 10.0, y: 10.0 },
        LineTo { x: 0.0, y: 10.0 },
        Close,
    ]
}

#[test]
fn a_line_is_cut_to_the_range() {
    let out = run(&line(0.0, 0.0, 10.0, 0.0), &[(0.2, 0.7)], true);
    assert!(near(&out, &line(2.0, 0.0, 7.0, 0.0)), "{out:?}");
}

#[test]
fn a_cut_can_span_corners() {
    let input = [
        MoveTo { x: 0.0, y: 0.0 },
        LineTo { x: 10.0, y: 0.0 },
        LineTo { x: 10.0, y: 10.0 },
    ];
    let out = run(&input, &[(0.25, 0.75)], true);
    let want = [
        MoveTo { x: 5.0, y: 0.0 },
        LineTo { x: 10.0, y: 0.0 },
        LineTo { x: 10.0, y: 5.0 },
    ];
    assert!(near(&out, &want), "{out:?}");
}

#[test]
fn the_whole_range_returns_the_path_unchanged() {
    let open = line(0.0, 0.0, 10.0, 0.0);
    assert!(near(&run(&open, &[(0.0, 1.0)], true), &open));
    // A closed path kept whole stays closed, so its corner at the seam is joined.
    assert!(near(&run(&square(), &[(0.0, 1.0)], true), &square()));
}

#[test]
fn a_partial_closed_path_is_open_and_includes_the_closing_edge() {
    // Perimeter 40; 30..40 is the closing edge from (0,10) back to (0,0).
    let out = run(&square(), &[(0.75, 1.0)], true);
    assert!(near(&out, &line(0.0, 10.0, 0.0, 0.0)), "{out:?}");
    assert!(!out.contains(&Close));
}

#[test]
fn a_wrapping_trim_gives_two_pieces() {
    let out = run(&line(0.0, 0.0, 10.0, 0.0), &[(0.9, 1.0), (0.0, 0.1)], true);
    let mut want = line(9.0, 0.0, 10.0, 0.0);
    want.extend(line(0.0, 0.0, 1.0, 0.0));
    assert!(near(&out, &want), "{out:?}");
}

fn two_lines() -> Vec<SvgPathCommand> {
    // Lengths 10 and 20.
    let mut v = line(0.0, 0.0, 10.0, 0.0);
    v.extend(line(0.0, 5.0, 20.0, 5.0));
    v
}

#[test]
fn simultaneous_trims_every_path_by_its_own_length() {
    let out = run(&two_lines(), &[(0.0, 0.5)], true);
    let mut want = line(0.0, 0.0, 5.0, 0.0);
    want.extend(line(0.0, 5.0, 10.0, 5.0));
    assert!(near(&out, &want), "{out:?}");
}

#[test]
fn individually_treats_the_paths_as_one_long_path() {
    // Total 30; the first half, 0..15, is all of path one and 5 units of path two.
    let out = run(&two_lines(), &[(0.0, 0.5)], false);
    let mut want = line(0.0, 0.0, 10.0, 0.0);
    want.extend(line(0.0, 5.0, 5.0, 5.0));
    assert!(near(&out, &want), "{out:?}");
    // A range inside the second path leaves the first out entirely.
    let later = run(&two_lines(), &[(0.5, 1.0)], false);
    assert!(near(&later, &line(5.0, 5.0, 20.0, 5.0)), "{later:?}");
}

/// A quarter circle of radius 100 from (100, 0) to (0, 100), as one cubic.
fn quarter() -> Vec<SvgPathCommand> {
    let k = 100.0 * 0.552_284_7;
    vec![
        MoveTo { x: 100.0, y: 0.0 },
        CubicTo { control1_x: 100.0, control1_y: k, control2_x: k, control2_y: 100.0, x: 0.0, y: 100.0 },
    ]
}

#[test]
fn a_curve_stays_a_curve_and_is_cut_by_arc_length() {
    let out = run(&quarter(), &[(0.0, 0.5)], true);
    assert_eq!(out.len(), 2, "{out:?}");
    let SvgPathCommand::CubicTo { x, y, .. } = out[1] else { panic!("a cubic expected: {out:?}") };
    // Half the arc length of a quarter circle is 45 degrees.
    let half = 100.0 * std::f32::consts::FRAC_1_SQRT_2;
    assert!((x - half).abs() < 0.5 && (y - half).abs() < 0.5, "({x}, {y})");
}

#[test]
fn the_cut_curve_still_starts_where_the_original_did() {
    let out = run(&quarter(), &[(0.5, 1.0)], true);
    let MoveTo { x, y } = out[0] else { panic!("{out:?}") };
    let half = 100.0 * std::f32::consts::FRAC_1_SQRT_2;
    assert!((x - half).abs() < 0.5 && (y - half).abs() < 0.5, "({x}, {y})");
    let SvgPathCommand::CubicTo { x, y, .. } = out[1] else { panic!("{out:?}") };
    assert!((x - 0.0).abs() < 1e-2 && (y - 100.0).abs() < 1e-2);
}

#[test]
fn nothing_in_nothing_out() {
    assert!(run(&[], &[(0.0, 1.0)], true).is_empty());
    assert!(run(&line(0.0, 0.0, 10.0, 0.0), &[], true).is_empty());
    assert!(run(&[MoveTo { x: 1.0, y: 1.0 }], &[(0.0, 1.0)], true).is_empty(), "a lone move has no length");
    assert!(run(&line(3.0, 3.0, 3.0, 3.0), &[(0.0, 1.0)], true).is_empty(), "a zero-length path draws nothing");
}

#[test]
fn an_empty_range_draws_nothing() {
    assert!(run(&line(0.0, 0.0, 10.0, 0.0), &[(0.4, 0.4)], true).is_empty());
}

#[test]
fn broken_numbers_do_not_poison_the_output() {
    let nan = [MoveTo { x: f32::NAN, y: 0.0 }, LineTo { x: 10.0, y: 0.0 }];
    let out = run(&nan, &[(0.0, 0.5)], true);
    for c in out {
        if let MoveTo { x, y } | LineTo { x, y } = c {
            assert!(x.is_finite() && y.is_finite(), "{x} {y}");
        }
    }
}
