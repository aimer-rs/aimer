//! Where the rows of a menu sit, and what a press on them means.
//!
//! Pure arithmetic over rectangles in *logical* pixels: no canvas, no window,
//! no element, so every rule below is asserted directly. The element measures
//! the labels and hands the widths in; everything else is decided here.

use aimer_attribute::{Bounds, Vec2d};

use crate::shape::ContextMenuShape;
use crate::style::ContextMenuStyle;

/// The extent of each item along the axis the items are tiled on.
///
/// A pill tiles sideways, so each item is as wide as its label plus the padding
/// on both sides. A list stacks, so every row is as wide as the widest one.
pub(crate) fn item_widths(
    shape: ContextMenuShape,
    style: &ContextMenuStyle,
    label_widths: &[f32],
) -> Vec<f32> {
    let padded = label_widths
        .iter()
        .map(|width| width + style.item_padding * 2.0);
    if shape.is_horizontal() {
        padded.collect()
    } else {
        let widest = padded.fold(style.min_width, f32::max);
        vec![widest; label_widths.len()]
    }
}

/// The extra height above row `index`, which is the line that sets it apart: none for a pill, whose items stand side by side, and none for a row
/// that `gaps` does not say anything of.
#[inline]
fn gap_of(shape: ContextMenuShape, gaps: &[f32], index: usize) -> f32 {
    if shape.is_horizontal() { 0.0 } else { gaps.get(index).copied().unwrap_or(0.0) }
}

/// The size the rows need, in logical pixels. `gaps` is the extra height above each row, where one is set apart from the one before it by a
/// line.
///
/// An empty menu needs nothing at all — which is what keeps a menu opened with
/// no verbs from painting an empty panel.
pub(crate) fn content_size(
    shape: ContextMenuShape,
    style: &ContextMenuStyle,
    label_widths: &[f32],
    gaps: &[f32],
) -> (f32, f32) {
    if label_widths.is_empty() {
        return (0.0, 0.0);
    }
    let widths = item_widths(shape, style, label_widths);
    if shape.is_horizontal() {
        (
            widths.iter().sum::<f32>().max(style.min_width),
            style.row_height,
        )
    } else {
        let lines: f32 = (0..label_widths.len()).map(|index| gap_of(shape, gaps, index)).sum();
        (widths[0], style.row_height * label_widths.len() as f32 + lines)
    }
}

/// One rectangle per item, in the same space `origin` is expressed in. The rectangle of a row that has a line above it takes the line in its
/// top, so the rectangles fill the content with nothing between them: what a press means is [`hit_rect`]'s.
pub(crate) fn row_rects(
    shape: ContextMenuShape,
    style: &ContextMenuStyle,
    origin: Vec2d,
    label_widths: &[f32],
    gaps: &[f32],
) -> Vec<Bounds> {
    let widths = item_widths(shape, style, label_widths);
    let mut rects = Vec::with_capacity(widths.len());
    let mut cursor = if shape.is_horizontal() {
        origin.x
    } else {
        origin.y
    };
    for (index, width) in widths.into_iter().enumerate() {
        if shape.is_horizontal() {
            rects.push(Bounds::new(cursor, origin.y, width, style.row_height));
            cursor += width;
        } else {
            let height = style.row_height + gap_of(shape, gaps, index);
            rects.push(Bounds::new(origin.x, cursor, width, height));
            cursor += height;
        }
    }
    rects
}

/// What a press on `rect` is a press on: the row without the line above it, which belongs to no row.
pub(crate) fn hit_rect(shape: ContextMenuShape, rect: Bounds, gap: f32) -> Bounds {
    let gap = gap_of(shape, &[gap], 0);
    Bounds::new(rect.x, rect.y + gap, rect.width, rect.height - gap)
}

/// The union of `rects`, or `None` when there are none.
pub(crate) fn union(rects: &[Bounds]) -> Option<Bounds> {
    let first = *rects.first()?;
    Some(rects.iter().skip(1).fold(first, |acc, rect| {
        let x = acc.x.min(rect.x);
        let y = acc.y.min(rect.y);
        Bounds::new(
            x,
            y,
            (acc.x + acc.width).max(rect.x + rect.width) - x,
            (acc.y + acc.height).max(rect.y + rect.height) - y,
        )
    }))
}

/// The index of the item under `(x, y)`.
///
/// A pill resolves a press on its rounded corners to the nearest item, because
/// its items tile it edge to edge and the corner is plainly part of the button
/// beside it. A list does not: the space above its first row and below its last
/// is the panel's deliberate padding, and a press there must run nothing.
pub(crate) fn row_at(shape: ContextMenuShape, rects: &[Bounds], x: f32, y: f32) -> Option<usize> {
    let hit = rects.iter().position(|rect| contains(*rect, x, y));
    if hit.is_some() || !shape.is_horizontal() {
        return hit;
    }
    let bounds = union(rects)?;
    contains(bounds, x, y).then(|| if x <= bounds.x { 0 } else { rects.len() - 1 })
}

/// Whether `(x, y)` is inside `bounds`, edges included.
#[inline]
pub(crate) fn contains(bounds: Bounds, x: f32, y: f32) -> bool {
    x >= bounds.x && x <= bounds.x + bounds.width && y >= bounds.y && y <= bounds.y + bounds.height
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS: [f32; 2] = [40.0, 70.0];

    fn at(x: f32, y: f32) -> Vec2d {
        Vec2d { x, y }
    }

    fn pill() -> ContextMenuStyle {
        ContextMenuStyle::pill()
    }

    fn list() -> ContextMenuStyle {
        ContextMenuStyle::list()
    }

    #[test]
    fn a_pill_is_as_wide_as_its_padded_labels_and_one_row_tall() {
        let style = pill();
        let (width, height) = content_size(ContextMenuShape::Pill, &style, &LABELS, &[]);

        assert_eq!(width, 40.0 + 70.0 + style.item_padding * 4.0);
        assert_eq!(height, style.row_height);
    }

    #[test]
    fn a_list_is_as_wide_as_its_widest_row_and_as_tall_as_all_of_them() {
        let style = list();
        let (width, height) = content_size(ContextMenuShape::List, &style, &LABELS, &[]);

        assert_eq!(width, style.min_width, "the floor wins over short labels");
        assert_eq!(height, style.row_height * 2.0);

        let (wide, _) = content_size(ContextMenuShape::List, &style, &[400.0], &[]);
        assert_eq!(wide, 400.0 + style.item_padding * 2.0);
    }

    #[test]
    fn an_empty_menu_needs_no_room_at_all() {
        for (shape, style) in [
            (ContextMenuShape::Pill, pill()),
            (ContextMenuShape::List, list()),
        ] {
            assert_eq!(content_size(shape, &style, &[], &[]), (0.0, 0.0));
            assert!(row_rects(shape, &style, at(10.0, 10.0), &[], &[]).is_empty());
        }
    }

    #[test]
    fn pill_items_tile_it_in_order_with_no_gap() {
        let style = pill();
        let rects = row_rects(ContextMenuShape::Pill, &style, at(100.0, 200.0), &LABELS, &[]);

        assert_eq!(rects[0].x, 100.0);
        assert_eq!(rects[0].y, 200.0);
        assert_eq!(rects[0].x + rects[0].width, rects[1].x);
        assert_eq!(rects[1].height, style.row_height);
    }

    #[test]
    fn list_rows_stack_downwards_at_a_common_width() {
        let style = list();
        let rects = row_rects(ContextMenuShape::List, &style, at(100.0, 200.0), &LABELS, &[]);

        assert_eq!(rects[0].y, 200.0);
        assert_eq!(rects[1].y, 200.0 + style.row_height);
        assert_eq!(rects[0].width, rects[1].width);
        assert_eq!(rects[0].x, 100.0);
    }

    #[test]
    fn a_press_lands_on_the_row_it_is_over() {
        let style = list();
        let rects = row_rects(ContextMenuShape::List, &style, at(100.0, 200.0), &LABELS, &[]);

        assert_eq!(
            row_at(ContextMenuShape::List, &rects, 110.0, 205.0),
            Some(0)
        );
        assert_eq!(
            row_at(
                ContextMenuShape::List,
                &rects,
                110.0,
                200.0 + style.row_height + 2.0
            ),
            Some(1)
        );
        assert_eq!(row_at(ContextMenuShape::List, &rects, 0.0, 0.0), None);
    }

    #[test]
    fn a_press_beside_a_list_row_runs_nothing() {
        let style = list();
        let rects = row_rects(ContextMenuShape::List, &style, at(100.0, 200.0), &LABELS, &[]);

        assert_eq!(
            row_at(ContextMenuShape::List, &rects, 110.0, 190.0),
            None,
            "the panel's padding is deliberate empty space"
        );
    }

    #[test]
    fn a_press_on_a_pills_rounded_end_belongs_to_the_item_beside_it() {
        let style = pill();
        let rects = row_rects(ContextMenuShape::Pill, &style, at(100.0, 200.0), &LABELS, &[]);
        let y = 200.0 + style.row_height * 0.5;

        assert_eq!(
            row_at(ContextMenuShape::Pill, &rects, 100.0, 200.0),
            Some(0),
            "the pill's very corner is the first item's"
        );
        let end = rects[1].x + rects[1].width;
        assert_eq!(
            row_at(ContextMenuShape::Pill, &rects, end, y),
            Some(1),
            "and the other corner is the last item's"
        );
        assert_eq!(row_at(ContextMenuShape::Pill, &rects, 500.0, y), None);
    }

    const GAPS: [f32; 2] = [0.0, 9.0];

    #[test]
    fn a_separator_adds_its_height_above_the_row_it_sets_apart() {
        let style = list();
        let (_, height) = content_size(ContextMenuShape::List, &style, &LABELS, &GAPS);
        assert_eq!(height, style.row_height * 2.0 + 9.0);
        let rects = row_rects(ContextMenuShape::List, &style, at(100.0, 200.0), &LABELS, &GAPS);
        assert_eq!(rects[0].y, 200.0);
        assert_eq!(rects[1].y, 200.0 + style.row_height, "the row's rectangle begins where the one above ends, and has the line in its top");
        assert_eq!(rects[1].height, style.row_height + 9.0);
        assert_eq!(rects[1].y + rects[1].height, 200.0 + height, "and the rows fill the content");
    }

    #[test]
    fn gaps_that_are_not_given_are_none_and_a_pill_has_none_whatever_is_given() {
        let style = list();
        let few = row_rects(ContextMenuShape::List, &style, at(0.0, 0.0), &LABELS, &[]);
        let none = row_rects(ContextMenuShape::List, &style, at(0.0, 0.0), &LABELS, &[0.0, 0.0]);
        assert_eq!(few, none);
        let pill = pill();
        assert_eq!(
            content_size(ContextMenuShape::Pill, &pill, &LABELS, &GAPS),
            content_size(ContextMenuShape::Pill, &pill, &LABELS, &[]),
            "the items of a pill stand side by side: there is no above"
        );
        assert_eq!(row_rects(ContextMenuShape::Pill, &pill, at(0.0, 0.0), &LABELS, &GAPS), row_rects(ContextMenuShape::Pill, &pill, at(0.0, 0.0), &LABELS, &[]));
    }

    #[test]
    fn a_press_in_the_line_between_two_groups_runs_nothing_and_one_below_it_runs_the_row() {
        let style = list();
        let rects = row_rects(ContextMenuShape::List, &style, at(100.0, 200.0), &LABELS, &GAPS);
        let hits: Vec<Bounds> = rects.iter().zip([0.0, 9.0]).map(|(rect, gap)| hit_rect(ContextMenuShape::List, *rect, gap)).collect();
        let line = 200.0 + style.row_height + 4.0;
        assert_eq!(row_at(ContextMenuShape::List, &hits, 110.0, line), None, "the line is not a row");
        assert_eq!(row_at(ContextMenuShape::List, &hits, 110.0, 200.0 + style.row_height + 9.0 + 2.0), Some(1));
        assert_eq!(row_at(ContextMenuShape::List, &hits, 110.0, 205.0), Some(0));
        assert_eq!(hit_rect(ContextMenuShape::List, rects[0], 0.0), rects[0], "a row with no line is its whole rectangle");
        let pill_rect = Bounds::new(0.0, 0.0, 50.0, 40.0);
        assert_eq!(hit_rect(ContextMenuShape::Pill, pill_rect, 9.0), pill_rect, "a pill has no lines");
    }

    #[test]
    fn the_union_of_the_rows_is_the_content_they_fill() {
        let style = pill();
        let rects = row_rects(ContextMenuShape::Pill, &style, at(100.0, 200.0), &LABELS, &[]);
        let bounds = union(&rects).expect("two rows have a union");
        let (width, height) = content_size(ContextMenuShape::Pill, &style, &LABELS, &[]);

        assert_eq!(bounds.x, 100.0);
        assert_eq!(bounds.width, width);
        assert_eq!(bounds.height, height);
        assert_eq!(union(&[]), None);
    }
}
