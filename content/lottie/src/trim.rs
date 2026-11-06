//! Cutting paths to a fraction of their length, for Lottie's trim paths.
//!
//! A path is measured piece by piece (a line, or a cubic whose length is the sum
//! of 16 chords), then cut at the requested distances. A cut line is a shorter
//! line and a cut cubic is a shorter cubic, found by de Casteljau subdivision,
//! so the tessellator still sees the same smooth curves.

use aimer_cupid::svg::SvgPathCommand::{self, Close, CubicTo, LineTo, MoveTo, QuadraticTo};

type Point = [f32; 2];

/// Chords per cubic when measuring it. The error is far below a pixel for the
/// curves Lottie files contain, and the cost is a handful of evaluations.
const CHORDS: usize = 16;

#[derive(Clone, Copy)]
enum Piece {
    Line { a: Point, b: Point },
    Cubic { a: Point, c1: Point, c2: Point, b: Point },
}

struct Subpath {
    start: Point,
    /// Each piece with its length.
    pieces: Vec<(Piece, f32)>,
    closed: bool,
    /// The last piece is the line `Close` added back to the start.
    closing_piece: bool,
    length: f32,
}

/// Keeps the parts of `input` inside `segments`, each a `(start, end)` pair of
/// fractions of a path's length, and appends them to `out` as open paths.
///
/// `simultaneous` trims every subpath by its own length; otherwise all the
/// subpaths are treated as one long path, in order. A closed path that is kept
/// whole stays closed.
pub(crate) fn trim(
    input: &[SvgPathCommand],
    segments: [Option<(f32, f32)>; 2],
    simultaneous: bool,
    out: &mut Vec<SvgPathCommand>,
) {
    let subpaths = measure(input);
    let ranges: Vec<(f32, f32)> = segments
        .into_iter()
        .flatten()
        .filter(|(a, b)| a.is_finite() && b.is_finite() && b > a)
        .collect();
    if subpaths.is_empty() || ranges.is_empty() {
        return;
    }
    if simultaneous {
        for sub in &subpaths {
            for &(a, b) in &ranges {
                cut(sub, a * sub.length, b * sub.length, out);
            }
        }
    } else {
        let total: f32 = subpaths.iter().map(|s| s.length).sum();
        for &(a, b) in &ranges {
            let (from, to) = (a * total, b * total);
            let mut offset = 0.0;
            for sub in &subpaths {
                let base = offset;
                offset += sub.length;
                let (d0, d1) = ((from - base).max(0.0), (to - base).min(sub.length));
                if d1 > d0 {
                    cut(sub, d0, d1, out);
                }
            }
        }
    }
}

/// Splits `input` into subpaths and measures them. Subpaths with no finite
/// length, such as a lone move, are dropped.
fn measure(input: &[SvgPathCommand]) -> Vec<Subpath> {
    let mut done = Vec::new();
    let mut current: Option<Subpath> = None;
    let (mut start, mut pos): (Point, Point) = ([0.0; 2], [0.0; 2]);

    fn finish(current: &mut Option<Subpath>, done: &mut Vec<Subpath>) {
        if let Some(sub) = current.take()
            && sub.length > 0.0
            && sub.length.is_finite()
        {
            done.push(sub);
        }
    }
    fn push(current: &mut Option<Subpath>, from: Point, piece: Piece) {
        let sub = current.get_or_insert_with(|| Subpath {
            start: from,
            pieces: Vec::new(),
            closed: false,
            closing_piece: false,
            length: 0.0,
        });
        let length = piece_length(&piece);
        // A piece with a broken length would poison every cut after it.
        if length > 0.0 && length.is_finite() {
            sub.length += length;
            sub.pieces.push((piece, length));
        }
    }

    for command in input {
        match *command {
            MoveTo { x, y } => {
                finish(&mut current, &mut done);
                start = [x, y];
                pos = start;
                current = Some(Subpath {
                    start,
                    pieces: Vec::new(),
                    closed: false,
                    closing_piece: false,
                    length: 0.0,
                });
            }
            LineTo { x, y } => {
                push(&mut current, pos, Piece::Line { a: pos, b: [x, y] });
                pos = [x, y];
            }
            QuadraticTo { control_x, control_y, x, y } => {
                // Degree elevation: exactly the same curve as a cubic.
                let (q, b) = ([control_x, control_y], [x, y]);
                let lift = |p: Point, q: Point| [p[0] + 2.0 / 3.0 * (q[0] - p[0]), p[1] + 2.0 / 3.0 * (q[1] - p[1])];
                push(&mut current, pos, Piece::Cubic { a: pos, c1: lift(pos, q), c2: lift(b, q), b });
                pos = b;
            }
            CubicTo { control1_x, control1_y, control2_x, control2_y, x, y } => {
                let piece = Piece::Cubic {
                    a: pos,
                    c1: [control1_x, control1_y],
                    c2: [control2_x, control2_y],
                    b: [x, y],
                };
                push(&mut current, pos, piece);
                pos = [x, y];
            }
            Close => {
                if let Some(sub) = current.as_mut() {
                    sub.closed = true;
                    if pos != start {
                        let before = sub.pieces.len();
                        push_closing(sub, pos, start);
                        sub.closing_piece = sub.pieces.len() > before;
                    }
                }
                finish(&mut current, &mut done);
                pos = start;
            }
        }
    }
    finish(&mut current, &mut done);
    done
}

fn push_closing(sub: &mut Subpath, from: Point, to: Point) {
    let piece = Piece::Line { a: from, b: to };
    let length = piece_length(&piece);
    if length > 0.0 && length.is_finite() {
        sub.length += length;
        sub.pieces.push((piece, length));
    }
}

fn distance(a: Point, b: Point) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn cubic_at(a: Point, c1: Point, c2: Point, b: Point, t: f32) -> Point {
    let u = 1.0 - t;
    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [
        w0 * a[0] + w1 * c1[0] + w2 * c2[0] + w3 * b[0],
        w0 * a[1] + w1 * c1[1] + w2 * c2[1] + w3 * b[1],
    ]
}

fn piece_length(piece: &Piece) -> f32 {
    match *piece {
        Piece::Line { a, b } => distance(a, b),
        Piece::Cubic { a, c1, c2, b } => {
            let mut length = 0.0;
            let mut previous = a;
            for i in 1..=CHORDS {
                let point = cubic_at(a, c1, c2, b, i as f32 / CHORDS as f32);
                length += distance(previous, point);
                previous = point;
            }
            length
        }
    }
}

/// The curve parameter at distance `d` along a piece whose measured length is
/// `length`, found by walking the same chords the length was summed from.
fn parameter_at(piece: &Piece, d: f32, length: f32) -> f32 {
    let d = d.clamp(0.0, length);
    match *piece {
        Piece::Line { .. } => d / length,
        Piece::Cubic { a, c1, c2, b } => {
            let mut walked = 0.0;
            let mut previous = a;
            for i in 1..=CHORDS {
                let point = cubic_at(a, c1, c2, b, i as f32 / CHORDS as f32);
                let chord = distance(previous, point);
                if walked + chord >= d && chord > 0.0 {
                    return ((i - 1) as f32 + (d - walked) / chord) / CHORDS as f32;
                }
                walked += chord;
                previous = point;
            }
            1.0
        }
    }
}

fn split(points: [Point; 4], t: f32) -> ([Point; 4], [Point; 4]) {
    let mix = |p: Point, q: Point| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
    let [p0, p1, p2, p3] = points;
    let (a, b, c) = (mix(p0, p1), mix(p1, p2), mix(p2, p3));
    let (d, e) = (mix(a, b), mix(b, c));
    let f = mix(d, e);
    ([p0, a, d, f], [f, e, c, p3])
}

/// The part of `piece` between parameters `t0` and `t1`.
fn sub_piece(piece: &Piece, t0: f32, t1: f32) -> Piece {
    match *piece {
        Piece::Line { a, b } => {
            let at = |t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            Piece::Line { a: at(t0), b: at(t1) }
        }
        Piece::Cubic { a, c1, c2, b } => {
            let (head, _) = split([a, c1, c2, b], t1);
            let local = if t1 > 0.0 { (t0 / t1).clamp(0.0, 1.0) } else { 0.0 };
            let (_, part) = split(head, local);
            Piece::Cubic { a: part[0], c1: part[1], c2: part[2], b: part[3] }
        }
    }
}

fn piece_start(piece: &Piece) -> Point {
    match *piece {
        Piece::Line { a, .. } | Piece::Cubic { a, .. } => a,
    }
}

fn push_piece(piece: &Piece, out: &mut Vec<SvgPathCommand>) {
    match *piece {
        Piece::Line { b, .. } => out.push(LineTo { x: b[0], y: b[1] }),
        Piece::Cubic { c1, c2, b, .. } => out.push(CubicTo {
            control1_x: c1[0],
            control1_y: c1[1],
            control2_x: c2[0],
            control2_y: c2[1],
            x: b[0],
            y: b[1],
        }),
    }
}

/// Emits `sub` between distances `d0` and `d1` along it.
fn cut(sub: &Subpath, d0: f32, d1: f32, out: &mut Vec<SvgPathCommand>) {
    if sub.closed && d0 <= 0.0 && d1 >= sub.length {
        // Whole and closed: keep it closed so the seam is a join, not two caps.
        out.push(MoveTo { x: sub.start[0], y: sub.start[1] });
        let kept = sub.pieces.len() - usize::from(sub.closing_piece);
        for (piece, _) in &sub.pieces[..kept] {
            push_piece(piece, out);
        }
        out.push(Close);
        return;
    }
    let mut started = false;
    let mut walked = 0.0;
    for (piece, length) in &sub.pieces {
        let (lo, hi) = (walked, walked + length);
        walked = hi;
        let (from, to) = (d0.max(lo), d1.min(hi));
        if to <= from {
            if lo >= d1 {
                break;
            }
            continue;
        }
        let part = sub_piece(
            piece,
            parameter_at(piece, from - lo, *length),
            parameter_at(piece, to - lo, *length),
        );
        if !started {
            let [x, y] = piece_start(&part);
            out.push(MoveTo { x, y });
            started = true;
        }
        push_piece(&part, out);
    }
}
