use std::ops::Range;

use aimer_attribute::Bounds;
use aimer_style::{TextDecoration, TextDecorationLine, TextDecorationStyle};

use crate::paragraph::{PreparedFragment, PreparedLayout};
use crate::selection::TextHitRegion;
use crate::text_span::ResolvedTextSpan;

/// A merged background rectangle covering one or more adjacent fragments that
/// share a line, a color and a vertical extent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreparedBackground {
    pub line: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: aimer_widget::base::Color,
}

/// One highlight rectangle of a selection, in element-local physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreparedSelection {
    pub line: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Paint-independent physical parameters for one text decoration style.
///
/// Position and width belong to the cached fragment geometry, while the
/// inherited color is resolved during painting so link hover can change it
/// without rebuilding the layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DecorationPaint {
    pub lines: TextDecorationLine,
    pub style: TextDecorationStyle,
    pub dedicated_color: Option<aimer_widget::base::Color>,
    pub thickness: f32,
    pub offset: f32,
    pub band_height: f32,
    pub period: f32,
}

/// Resolves the style-dependent physical parameters shared by plain and
/// paragraph-backed text.
///
/// Returns `None` for decorations that only request synthetic italic, because
/// italic is a glyph paint flag rather than a decoration line.
#[inline]
pub(crate) fn prepare_decoration_paint(
    decoration: TextDecoration,
    font_size: f32,
    scale: f32,
) -> Option<DecorationPaint> {
    let lines = decoration.line;
    let has_stroke = lines.contains(TextDecorationLine::UNDERLINE)
        || lines.contains(TextDecorationLine::LINE_THROUGH)
        || lines.contains(TextDecorationLine::OVERLINE);
    if !has_stroke {
        return None;
    }

    let thickness = decoration
        .thickness
        .map(|value| value * scale)
        .unwrap_or((font_size * 0.06).max(1.0));
    let offset = decoration.offset * scale;
    let (band_height, period) = match decoration.style {
        TextDecorationStyle::Double => (thickness * 3.0, 1.0),
        TextDecorationStyle::Dotted => (thickness, (thickness * 2.0).max(2.0)),
        TextDecorationStyle::Dashed => (thickness, (thickness * 4.0).max(2.0)),
        TextDecorationStyle::Wavy => (thickness * 4.0, (thickness * 6.0).max(4.0)),
        TextDecorationStyle::Solid => (thickness, 1.0),
    };

    Some(DecorationPaint {
        lines,
        style: decoration.style,
        dedicated_color: decoration.color,
        thickness,
        offset,
        band_height,
        period,
    })
}

/// Reports whether a vertical band intersects the visible rectangle.
///
/// A missing rectangle means nothing is clipped, so every band is visible.
pub(crate) fn vertical_span_is_visible(
    y: f32,
    height: f32,
    visible_rect: Option<(f32, f32, f32, f32)>,
) -> bool {
    let Some((_, visible_y, _, visible_height)) = visible_rect else {
        return true;
    };
    y + height >= visible_y && y <= visible_y + visible_height
}

/// Appends `run`, extending the previous run instead when the two touch on the
/// same line so a selection paints as one uninterrupted band.
pub(crate) fn push_selection_run(runs: &mut Vec<PreparedSelection>, run: PreparedSelection) {
    const TOUCH_EPSILON: f32 = 0.01;

    if let Some(previous) = runs.last_mut()
        && previous.line == run.line
        && (previous.y - run.y).abs() <= TOUCH_EPSILON
        && (previous.height - run.height).abs() <= TOUCH_EPSILON
        && (previous.x + previous.width - run.x).abs() <= TOUCH_EPSILON
    {
        previous.width = run.x + run.width - previous.x;
    } else {
        runs.push(run);
    }
}

/// Rounds every run to whole pixels so vertically adjacent lines share an edge
/// instead of leaving a seam or overlapping.
pub(crate) fn snap_selection_lines_to_pixels(runs: &mut [PreparedSelection]) {
    for run in runs {
        let bottom = (run.y + run.height).round();
        run.y = run.y.round();
        run.height = (bottom - run.y).max(0.0);
    }
}

/// Collapses the fragments' inline backgrounds into as few rectangles as
/// possible, skipping fully transparent and degenerate ones.
pub(crate) fn prepare_background_runs(
    fragments: &[PreparedFragment],
    spans: &[ResolvedTextSpan],
) -> Vec<PreparedBackground> {
    const TOUCH_EPSILON: f32 = 0.01;

    let mut runs: Vec<PreparedBackground> = Vec::new();
    for fragment in fragments {
        let Some(color) = spans[fragment.span_index].style.background_color else {
            continue;
        };
        if color.as_u32() >> 24 == 0 || fragment.width <= 0.0 || fragment.height <= 0.0 {
            continue;
        }

        let y = fragment.baseline - fragment.ascent;
        if let Some(previous) = runs.last_mut()
            && previous.line == fragment.line
            && previous.color == color
            && (previous.y - y).abs() <= TOUCH_EPSILON
            && (previous.height - fragment.height).abs() <= TOUCH_EPSILON
            && (previous.x + previous.width - fragment.x).abs() <= TOUCH_EPSILON
        {
            previous.width = fragment.x + fragment.width - previous.x;
            continue;
        }

        runs.push(PreparedBackground {
            line: fragment.line,
            x: fragment.x,
            y,
            width: fragment.width,
            height: fragment.height,
            color,
        });
    }
    runs
}

/// Appends one hit region per cached grapheme, plus one per line break, in
/// absolute logical coordinates.
///
/// `abs_x` and `abs_y` are the element's absolute physical translation and
/// `scale` the device pixel ratio, so the emitted [`Bounds`] can be compared
/// against pointer positions directly.
pub(crate) fn hit_regions(
    layout: &PreparedLayout,
    abs_x: f32,
    abs_y: f32,
    scale: f32,
    visible_rect: Option<(f32, f32, f32, f32)>,
    regions: &mut Vec<TextHitRegion>,
) {
    for grapheme in &layout.graphemes {
        let fragment = &layout.fragments[grapheme.fragment_index];
        let top = fragment.baseline - fragment.ascent;
        if !vertical_span_is_visible(top, fragment.height, visible_rect) {
            continue;
        }
        regions.push(TextHitRegion::new(
            grapheme.source_range.clone(),
            Bounds::new(
                (abs_x + grapheme.x) / scale,
                (abs_y + top) / scale,
                grapheme.width / scale,
                fragment.height / scale,
            ),
        ));
    }
    for line_break in &layout.line_breaks {
        if !vertical_span_is_visible(line_break.y, line_break.height, visible_rect) {
            continue;
        }
        regions.push(TextHitRegion::new(
            line_break.source_range.start..line_break.source_range.start,
            Bounds::new(
                (abs_x + line_break.x) / scale,
                (abs_y + line_break.y) / scale,
                line_break.hit_width / scale,
                line_break.height / scale,
            ),
        ));
    }
}

/// Builds the merged, pixel-snapped highlight rectangles covering `selection`.
///
/// The rectangles come from the layout's cached grapheme geometry, so no text
/// is measured here.
pub(crate) fn selection_runs(
    layout: &PreparedLayout,
    selection: Range<usize>,
    visible_rect: Option<(f32, f32, f32, f32)>,
) -> Vec<PreparedSelection> {
    let mut runs: Vec<PreparedSelection> = Vec::new();
    let mut fragment_index = usize::MAX;
    let mut start: Option<f32> = None;
    let mut end = 0.0_f32;

    for grapheme in &layout.graphemes {
        if grapheme.fragment_index != fragment_index {
            flush_fragment_run(layout, &mut runs, fragment_index, start.take(), end);
            fragment_index = grapheme.fragment_index;
            end = 0.0;
        }
        let fragment = &layout.fragments[fragment_index];
        if !vertical_span_is_visible(
            line_box_top(layout, fragment),
            layout.line_heights[fragment.line],
            visible_rect,
        ) {
            continue;
        }
        if grapheme.source_range.start < selection.end
            && selection.start < grapheme.source_range.end
        {
            start.get_or_insert(grapheme.x);
            end = grapheme.x + grapheme.width;
        }
    }
    flush_fragment_run(layout, &mut runs, fragment_index, start, end);

    for line_break in &layout.line_breaks {
        if !vertical_span_is_visible(line_break.y, line_break.height, visible_rect) {
            continue;
        }
        if line_break.source_range.start < selection.end
            && selection.start < line_break.source_range.end
        {
            runs.push(PreparedSelection {
                line: line_break.line,
                x: line_break.x,
                y: line_break.y,
                width: line_break.selection_width,
                height: line_break.height,
            });
        }
    }

    runs.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.x.total_cmp(&right.x))
    });
    let mut merged = Vec::with_capacity(runs.len());
    for run in runs {
        push_selection_run(&mut merged, run);
    }
    snap_selection_lines_to_pixels(&mut merged);
    merged
}

fn flush_fragment_run(
    layout: &PreparedLayout,
    runs: &mut Vec<PreparedSelection>,
    fragment_index: usize,
    start: Option<f32>,
    end: f32,
) {
    let Some(start) = start else {
        return;
    };
    let fragment = &layout.fragments[fragment_index];
    push_selection_run(
        runs,
        PreparedSelection {
            line: fragment.line,
            x: start,
            y: line_box_top(layout, fragment),
            width: end - start,
            height: layout.line_heights[fragment.line],
        },
    );
}

fn line_box_top(layout: &PreparedLayout, fragment: &PreparedFragment) -> f32 {
    let leading = (layout.line_heights[fragment.line] - fragment.height).max(0.0);
    fragment.baseline - fragment.ascent - leading * 0.5
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use aimer_style::{TextDecoration, TextDecorationLine, TextDecorationStyle, TextStyle};

    use super::{
        PreparedSelection, prepare_background_runs, prepare_decoration_paint,
        snap_selection_lines_to_pixels, vertical_span_is_visible,
    };
    use crate::paragraph::PreparedFragment;
    use crate::text_span::ResolvedTextSpan;

    #[test]
    fn decoration_paint_resolves_physical_stroke_geometry_once() {
        let paint = prepare_decoration_paint(
            TextDecoration::new()
                .line(
                    TextDecorationLine::UNDERLINE
                        | TextDecorationLine::LINE_THROUGH
                        | TextDecorationLine::OVERLINE,
                )
                .style(TextDecorationStyle::Wavy)
                .thickness(2.0)
                .offset(3.0),
            16.0,
            2.0,
        )
        .expect("real decoration lines should produce paint geometry");

        assert_eq!(paint.lines.bits(), 0b0111);
        assert_eq!(paint.style, TextDecorationStyle::Wavy);
        assert_eq!(paint.thickness, 4.0);
        assert_eq!(paint.offset, 6.0);
        assert_eq!(paint.band_height, 16.0);
        assert_eq!(paint.period, 24.0);
    }

    #[test]
    fn italic_only_decoration_has_no_stroke_geometry() {
        assert!(
            prepare_decoration_paint(
                TextDecoration::new().line(TextDecorationLine::ITALIC),
                16.0,
                1.0,
            )
            .is_none()
        );
    }

    #[test]
    fn rich_text_visibility_keeps_partial_lines_and_rejects_hidden_lines() {
        let viewport = Some((0.0, 40.0, 200.0, 20.0));

        assert!(vertical_span_is_visible(35.0, 10.0, viewport));
        assert!(vertical_span_is_visible(55.0, 10.0, viewport));
        assert!(!vertical_span_is_visible(10.0, 20.0, viewport));
        assert!(!vertical_span_is_visible(61.0, 10.0, viewport));
        assert!(vertical_span_is_visible(500.0, 10.0, None));
    }

    #[test]
    fn selection_line_overlap_does_not_overflow_past_a_shorter_next_line() {
        let mut highlights = vec![
            PreparedSelection {
                line: 0,
                x: 10.0,
                y: 20.25,
                width: 100.0,
                height: 10.48,
            },
            PreparedSelection {
                line: 1,
                x: 10.0,
                y: 30.73,
                width: 20.0,
                height: 10.48,
            },
        ];

        snap_selection_lines_to_pixels(&mut highlights);

        assert_eq!(highlights.len(), 2);
        assert_eq!(highlights[0].x, 10.0);
        assert_eq!(highlights[0].width, 100.0);
        assert_eq!(highlights[0].y, 20.0);
        assert_eq!(highlights[0].height, 11.0);
        assert_eq!(highlights[1].x, 10.0);
        assert_eq!(highlights[1].width, 20.0);
        assert_eq!(highlights[1].y, 31.0);
        assert_eq!(highlights[1].height, 10.0);
        assert_eq!(highlights[0].y + highlights[0].height, highlights[1].y);
    }

    #[test]
    fn backgrounds_merge_on_one_line_but_not_across_lines_or_colors() {
        let spans = vec![
            ResolvedTextSpan::plain(
                Rc::from("ab"),
                TextStyle::new().background_color(aimer_widget::base::Color::RED),
            ),
            ResolvedTextSpan::plain(
                Rc::from("c"),
                TextStyle::new().background_color(aimer_widget::base::Color::RED),
            ),
            ResolvedTextSpan::plain(
                Rc::from("d"),
                TextStyle::new().background_color(aimer_widget::base::Color::BLUE),
            ),
        ];
        let fragments = vec![
            PreparedFragment {
                span_index: 0,
                text: "ab".into(),
                source_range: None,
                rendered_source_ranges: Vec::new(),
                line: 0,
                x: 10.0,
                baseline: 18.0,
                width: 20.0,
                height: 12.0,
                ascent: 8.0,
                descent: 4.0,
            },
            PreparedFragment {
                span_index: 1,
                text: "c".into(),
                source_range: None,
                rendered_source_ranges: Vec::new(),
                line: 0,
                x: 30.0,
                baseline: 18.0,
                width: 10.0,
                height: 12.0,
                ascent: 8.0,
                descent: 4.0,
            },
            PreparedFragment {
                span_index: 2,
                text: "d".into(),
                source_range: None,
                rendered_source_ranges: Vec::new(),
                line: 0,
                x: 40.0,
                baseline: 18.0,
                width: 10.0,
                height: 12.0,
                ascent: 8.0,
                descent: 4.0,
            },
            PreparedFragment {
                span_index: 0,
                text: "a".into(),
                source_range: None,
                rendered_source_ranges: Vec::new(),
                line: 1,
                x: 0.0,
                baseline: 34.0,
                width: 10.0,
                height: 16.0,
                ascent: 12.0,
                descent: 4.0,
            },
        ];

        let runs = prepare_background_runs(&fragments, &spans);

        assert_eq!(runs.len(), 3);
        assert_eq!(
            (runs[0].x, runs[0].y, runs[0].width, runs[0].height),
            (10.0, 10.0, 30.0, 12.0)
        );
        assert_eq!(runs[0].color, aimer_widget::base::Color::RED);
        assert_eq!((runs[1].x, runs[1].width), (40.0, 10.0));
        assert_eq!(runs[1].color, aimer_widget::base::Color::BLUE);
        assert_eq!((runs[2].x, runs[2].y, runs[2].height), (0.0, 22.0, 16.0));
    }

    #[test]
    fn transparent_backgrounds_do_not_create_runs() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("hidden"),
            TextStyle::new().background_color(aimer_widget::base::Color::Transparent),
        )];
        let fragments = vec![PreparedFragment {
            span_index: 0,
            text: "hidden".into(),
            source_range: None,
            rendered_source_ranges: Vec::new(),
            line: 0,
            x: 0.0,
            baseline: 10.0,
            width: 30.0,
            height: 10.0,
            ascent: 8.0,
            descent: 2.0,
        }];

        assert!(prepare_background_runs(&fragments, &spans).is_empty());
    }
}
