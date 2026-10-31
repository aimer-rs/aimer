use super::super::{OutlinePath, PathCommand, flattened_path};
use super::*;

#[test]
fn normalization_preserves_scalar_rounding_and_byte_truncation() {
    for grid in [0_u32, 1, 2, 3, 4, 8, 15] {
        let denominator = grid * grid;
        for length in [0, 1, 7, 15, 16, 17, 31, 32, 33, 256, 257] {
            let input: Vec<u8> = (0..length).map(|i| i as u8).collect();
            let expected: Vec<u8> = input
                .iter()
                .map(|&count| {
                    ((u32::from(count) * 255 + denominator / 2)
                        .checked_div(denominator)
                        .unwrap_or(0)) as u8
                })
                .collect();
            let mut actual = input;
            normalize_bitmap(&mut actual, grid);
            assert_eq!(actual, expected, "grid {grid}, length {length}");
        }
    }
}

#[test]
fn coverage_spans_match_sample_counting_across_pixel_and_vector_boundaries() {
    for grid in [4_u32, 8] {
        for width in [1, 2, 7, 15, 16, 17, 31, 32, 33, 65] {
            for (start, end) in [
                (0.0_f32, width as f32),
                (0.125, width as f32 - 0.125),
                (0.375, 0.625),
                (0.75, 1.0),
                (1.0, 1.25),
                (1.125, width as f32),
            ] {
                let start = start.min(width as f32);
                let end = end.min(width as f32);
                if start >= end {
                    continue;
                }
                let mut expected = vec![0_u8; width + 2];
                for column in 0..width {
                    for sample in 0..grid {
                        let x = column as f32 + (sample as f32 + 0.5) / grid as f32;
                        if x >= start && x < end {
                            expected[column + 1] += 1;
                        }
                    }
                }
                let mut actual = vec![0_u8; width + 2];
                let spans = [CoverageSpan { start, end }];
                let written = fill_coverage_spans(&spans, 1, width, grid, &mut actual);
                assert_eq!(
                    actual, expected,
                    "grid {grid}, width {width}, span {start}..{end}"
                );
                assert_eq!(written, expected.iter().any(|&x| x != 0));
            }
        }
    }
}

#[test]
#[ignore = "manual uncached scan-conversion benchmark"]
fn profile_scan_conversion() {
    use std::hint::black_box;
    use aimer_time::Instant;

    for size in [8, 16, 32, 64, 128] {
        let extent = size as f32;
        let contours = vec![
            vec![
                Point { x: 0.25, y: 0.125 },
                Point {
                    x: extent - 0.25,
                    y: 0.5,
                },
                Point {
                    x: extent - 1.75,
                    y: extent - 0.25,
                },
                Point {
                    x: 0.5,
                    y: extent - 1.125,
                },
                Point { x: 0.25, y: 0.125 },
            ],
            vec![
                Point {
                    x: extent * 0.2,
                    y: extent * 0.2,
                },
                Point {
                    x: extent * 0.2,
                    y: extent * 0.7,
                },
                Point {
                    x: extent * 0.7,
                    y: extent * 0.7,
                },
                Point {
                    x: extent * 0.7,
                    y: extent * 0.2,
                },
                Point {
                    x: extent * 0.2,
                    y: extent * 0.2,
                },
            ],
        ];
        let edges = contours_to_edges(&contours);
        let rows = edge_rows(&edges, size).unwrap();
        for grid in [4, 8] {
            let mut bitmap = vec![0; size * size];
            let mut scratch = RasterScratch::default();
            let mut samples = [0.0; 21];
            for sample in &mut samples {
                let started = Instant::now();
                for _ in 0..100 {
                    black_box(scan_convert_into_with_rows(
                        black_box(&edges),
                        Some(&rows),
                        size,
                        size,
                        grid,
                        &mut bitmap,
                        &mut scratch,
                    ));
                    black_box(&bitmap);
                }
                *sample = started.elapsed().as_secs_f64() * 1e6 / 100.0;
            }
            samples.sort_by(f64::total_cmp);
            println!("size={size:3}, grid={grid}: median={:.3} us", samples[10]);
        }
    }
}

#[test]
fn disjoint_spans_share_partial_pixels_without_losing_coverage() {
    for grid in [4_u32, 8] {
        let spans = [
            CoverageSpan {
                start: 0.0,
                end: 0.375,
            },
            CoverageSpan {
                start: 0.625,
                end: 17.375,
            },
            CoverageSpan {
                start: 17.625,
                end: 33.0,
            },
        ];
        let mut expected = vec![0_u8; 33];
        for column in 0..33 {
            for sample in 0..grid {
                let x = column as f32 + (sample as f32 + 0.5) / grid as f32;
                if spans.iter().any(|span| x >= span.start && x < span.end) {
                    expected[column] += 1;
                }
            }
        }
        let mut actual = vec![0; 33];
        for _ in 0..grid {
            assert!(fill_coverage_spans(&spans, 0, 33, grid, &mut actual));
        }
        for count in &mut expected {
            *count *= grid as u8;
        }
        assert_eq!(actual, expected);
    }
}

#[test]
fn scan_conversion_validates_storage_and_preserves_zero_coverage() {
    let mut scratch = RasterScratch::default();
    let mut bitmap = [99; 17];
    assert!(!scan_convert_into_with_rows(
        &[],
        None,
        18,
        1,
        4,
        &mut bitmap,
        &mut scratch
    ));
    assert_eq!(bitmap, [99; 17]);
    assert!(!scan_convert_into_with_rows(
        &[],
        None,
        usize::MAX,
        2,
        4,
        &mut bitmap,
        &mut scratch
    ));
    assert_eq!(bitmap, [99; 17]);
    assert!(!scan_convert_into_with_rows(
        &[],
        None,
        16,
        1,
        0,
        &mut bitmap,
        &mut scratch
    ));
    assert_eq!(bitmap, [99; 17]);
    assert!(!scan_convert_into_with_rows(
        &[],
        None,
        16,
        1,
        4,
        &mut bitmap,
        &mut scratch
    ));
    assert_eq!(bitmap, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 99]);
    assert!(!scan_convert_into_with_rows(
        &[],
        None,
        0,
        1,
        8,
        &mut [],
        &mut scratch
    ));
}

#[test]
fn scan_conversion_uses_nonzero_winding_fill_for_holes() {
    let contours = vec![
        vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 4.0, y: 0.0 },
            Point { x: 4.0, y: 4.0 },
            Point { x: 0.0, y: 4.0 },
            Point { x: 0.0, y: 0.0 },
        ],
        vec![
            Point { x: 1.0, y: 1.0 },
            Point { x: 1.0, y: 3.0 },
            Point { x: 3.0, y: 3.0 },
            Point { x: 3.0, y: 1.0 },
            Point { x: 1.0, y: 1.0 },
        ],
    ];

    let bitmap = scan_convert_reference(&contours, 4, 4, 16);

    assert_eq!(bitmap[0], 255, "the outer contour should remain filled");
    assert_eq!(bitmap[5], 0, "the inner contour should remain a hole");
}

#[test]
fn scan_conversion_keeps_overlapping_contours_filled() {
    // Both contours have the same winding direction. This is how variable
    // and overlap-flagged TrueType glyphs join adjacent strokes: even-odd
    // filling would turn the overlap into a transparent dropout.
    let contours = vec![
        vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 4.0, y: 0.0 },
            Point { x: 4.0, y: 4.0 },
            Point { x: 0.0, y: 4.0 },
            Point { x: 0.0, y: 0.0 },
        ],
        vec![
            Point { x: 1.0, y: 1.0 },
            Point { x: 3.0, y: 1.0 },
            Point { x: 3.0, y: 3.0 },
            Point { x: 1.0, y: 3.0 },
            Point { x: 1.0, y: 1.0 },
        ],
    ];
    let edges = contours_to_edges(&contours);
    let row_edges = edge_rows(&edges, 4).expect("the test rows should build");
    let mut bitmap = vec![0; 16];

    scan_convert_into_with_rows(
        &edges,
        Some(&row_edges),
        4,
        4,
        SAMPLE_GRID,
        &mut bitmap,
        &mut RasterScratch::default(),
    );

    assert_eq!(
        bitmap[5], 255,
        "overlapping contours must not create a hole"
    );
}

#[test]
fn scan_conversion_supports_even_odd_svg_fills() {
    let contours = vec![
        vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 4.0, y: 0.0 },
            Point { x: 4.0, y: 4.0 },
            Point { x: 0.0, y: 4.0 },
            Point { x: 0.0, y: 0.0 },
        ],
        vec![
            Point { x: 1.0, y: 1.0 },
            Point { x: 3.0, y: 1.0 },
            Point { x: 3.0, y: 3.0 },
            Point { x: 1.0, y: 3.0 },
            Point { x: 1.0, y: 1.0 },
        ],
    ];
    let edges = contours_to_edges(&contours);
    let row_edges = edge_rows(&edges, 4).expect("the test rows should build");
    let mut bitmap = vec![0; 16];

    scan_convert_into_with_fill_rule(
        &edges,
        Some(&row_edges),
        4,
        4,
        SAMPLE_GRID,
        &mut bitmap,
        &mut RasterScratch::default(),
        CoverageFillRule::EvenOdd,
    );

    assert_eq!(bitmap[0], 255, "the outer contour should remain filled");
    assert_eq!(bitmap[5], 0, "even-odd SVG fills should preserve the hole");
}

#[test]
fn default_scan_conversion_preserves_fine_edge_coverage() {
    assert!(
        SAMPLE_GRID >= 4,
        "the default converter needs at least 4x4 coverage samples"
    );

    let contours = vec![vec![
        Point { x: 0.0, y: 0.0 },
        Point { x: 4.0, y: 0.0 },
        Point { x: 0.0, y: 4.0 },
        Point { x: 0.0, y: 0.0 },
    ]];
    let edges = contours_to_edges(&contours);
    let row_edges = edge_rows(&edges, 4).expect("the test rows should build");
    let mut bitmap = vec![0; 16];

    scan_convert_into_with_rows(
        &edges,
        Some(&row_edges),
        4,
        4,
        SAMPLE_GRID,
        &mut bitmap,
        &mut RasterScratch::default(),
    );

    assert!(
        bitmap.iter().any(|coverage| {
            *coverage > 0
                && *coverage < 255
                && !matches!(*coverage, 63 | 64 | 127 | 128 | 191 | 192)
        }),
        "the diagonal edge should contain coverage finer than a 2x2 grid: {bitmap:?}"
    );
}

#[test]
fn optimized_scan_conversion_reuses_scratch_and_matches_reference() {
    let contours = vec![vec![
        Point { x: 0.0, y: 0.0 },
        Point { x: 3.5, y: 0.0 },
        Point { x: 3.5, y: 3.5 },
        Point { x: 0.0, y: 3.5 },
        Point { x: 0.0, y: 0.0 },
    ]];
    let edges = contours_to_edges(&contours);
    let row_edges = edge_rows(&edges, 4).expect("the test rows should build");
    let expected = scan_convert_reference(&contours, 4, 4, 16);
    let mut actual = vec![0; 16];
    let mut scratch = RasterScratch::default();

    scan_convert_into_with_rows(
        &edges,
        Some(&row_edges),
        4,
        4,
        SAMPLE_GRID,
        &mut actual,
        &mut scratch,
    );
    let active_capacity = scratch.active_edges.capacity();
    let position_capacity = scratch.active_positions.capacity();
    scan_convert_into_with_rows(
        &edges,
        Some(&row_edges),
        4,
        4,
        SAMPLE_GRID,
        &mut actual,
        &mut scratch,
    );

    assert_eq!(actual, expected);
    assert!(active_capacity > 0);
    assert_eq!(scratch.active_edges.capacity(), active_capacity);
    assert_eq!(scratch.active_positions.capacity(), position_capacity);
}

#[test]
fn scan_conversion_reports_whether_it_wrote_coverage() {
    let contours = vec![vec![
        Point { x: 0.0, y: 0.0 },
        Point { x: 2.0, y: 0.0 },
        Point { x: 0.0, y: 2.0 },
        Point { x: 0.0, y: 0.0 },
    ]];
    let edges = contours_to_edges(&contours);
    let row_edges = edge_rows(&edges, 2).expect("the test rows should build");
    let mut bitmap = vec![0; 4];

    let has_coverage = scan_convert_into_with_rows(
        &edges,
        Some(&row_edges),
        2,
        2,
        SAMPLE_GRID,
        &mut bitmap,
        &mut RasterScratch::default(),
    );

    assert!(has_coverage);
    assert!(bitmap.iter().any(|coverage| *coverage != 0));
}

#[test]
fn precomputed_scanline_plan_matches_reference_for_slanted_holes() {
    let contours = vec![
        vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 5.0, y: 0.0 },
            Point { x: 4.0, y: 5.0 },
            Point { x: 0.0, y: 4.0 },
            Point { x: 0.0, y: 0.0 },
        ],
        vec![
            Point { x: 1.0, y: 1.0 },
            Point { x: 1.0, y: 3.0 },
            Point { x: 3.0, y: 3.0 },
            Point { x: 3.0, y: 1.0 },
            Point { x: 1.0, y: 1.0 },
        ],
    ];
    let edges = contours_to_edges(&contours);
    let row_edges = edge_rows(&edges, 5).expect("the test rows should build");
    let plan = build_scanline_plan(&edges, &row_edges, 5, 5, SAMPLE_GRID)
        .expect("the bounded test plan should build");
    let mut actual = vec![0; 25];

    assert!(fill_scanline_plan(&plan, 5, 5, &mut actual));
    assert_eq!(actual, scan_convert_reference(&contours, 5, 5, 25));
}

#[test]
fn active_edge_coverage_matches_reference_for_a_slanted_outline() {
    let contours = vec![vec![
        Point { x: 0.0, y: 0.0 },
        Point { x: 5.0, y: 0.0 },
        Point { x: 4.0, y: 5.0 },
        Point { x: 0.0, y: 4.0 },
        Point { x: 0.0, y: 0.0 },
    ]];
    let path = OutlinePath {
        bounds: [0.0, 0.0, 5.0, 5.0],
        commands: vec![
            PathCommand::MoveTo(Point { x: 0.0, y: 0.0 }),
            PathCommand::LineTo(Point { x: 5.0, y: 0.0 }),
            PathCommand::LineTo(Point { x: 4.0, y: 5.0 }),
            PathCommand::LineTo(Point { x: 0.0, y: 4.0 }),
            PathCommand::Close,
        ],
    };
    let flattened = flattened_path(&path, 1.0, 0, 0, 0.0).expect("the test path should flatten");
    let actual = build_coverage_bitmap(&flattened, &mut RasterScratch::default())
        .expect("the active-edge bitmap should build");

    assert_eq!(actual, scan_convert_reference(&contours, 5, 5, 25));
}
