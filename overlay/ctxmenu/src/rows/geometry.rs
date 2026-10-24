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

/// The size the rows need, in logical pixels.
///
/// An empty menu needs nothing at all — which is what keeps a menu opened with
/// no verbs from painting an empty panel.
pub(crate) fn content_size(
    shape: ContextMenuShape,
    style: &ContextMenuStyle,
    label_widths: &[f32],
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
        (widths[0], style.row_height * label_widths.len() as f32)
    }
}

/// One rectangle per item, in the same space `origin` is expressed in.
pub(crate) fn row_rects(
    shape: ContextMenuShape,
    style: &ContextMenuStyle,
    origin: Vec2d,
    label_widths: &[f32],
) -> Vec<Bounds> {
    let widths = item_widths(shape, style, label_widths);
    let mut rects = Vec::with_capacity(widths.len());
    let mut cursor = if shape.is_horizontal() {
        origin.x
    } else {
        origin.y
    };
    for width in widths {
        if shape.is_horizontal() {
            rects.push(Bounds::new(cursor, origin.y, width, style.row_height));
            cursor += width;
        } else {
            rects.push(Bounds::new(origin.x, cursor, width, style.row_height));
            cursor += style.row_height;
        }
    }
    rects
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
        let (width, height) = content_size(ContextMenuShape::Pill, &style, &LABELS);

        assert_eq!(width, 40.0 + 70.0 + style.item_padding * 4.0);
        assert_eq!(height, style.row_height);
    }

    #[test]
    fn a_list_is_as_wide_as_its_widest_row_and_as_tall_as_all_of_them() {
        let style = list();
        let (width, height) = content_size(ContextMenuShape::List, &style, &LABELS);

        assert_eq!(width, style.min_width, "the floor wins over short labels");
        assert_eq!(height, style.row_height * 2.0);

        let (wide, _) = content_size(ContextMenuShape::List, &style, &[400.0]);
        assert_eq!(wide, 400.0 + style.item_padding * 2.0);
    }

    #[test]
    fn an_empty_menu_needs_no_room_at_all() {
        for (shape, style) in [
            (ContextMenuShape::Pill, pill()),
            (ContextMenuShape::List, list()),
        ] {
            assert_eq!(content_size(shape, &style, &[]), (0.0, 0.0));
            assert!(row_rects(shape, &style, at(10.0, 10.0), &[]).is_empty());
        }
    }

    #[test]
    fn pill_items_tile_it_in_order_with_no_gap() {
        let style = pill();
        let rects = row_rects(ContextMenuShape::Pill, &style, at(100.0, 200.0), &LABELS);

        assert_eq!(rects[0].x, 100.0);
        assert_eq!(rects[0].y, 200.0);
        assert_eq!(rects[0].x + rects[0].width, rects[1].x);
        assert_eq!(rects[1].height, style.row_height);
    }

    #[test]
    fn list_rows_stack_downwards_at_a_common_width() {
        let style = list();
        let rects = row_rects(ContextMenuShape::List, &style, at(100.0, 200.0), &LABELS);

        assert_eq!(rects[0].y, 200.0);
        assert_eq!(rects[1].y, 200.0 + style.row_height);
        assert_eq!(rects[0].width, rects[1].width);
        assert_eq!(rects[0].x, 100.0);
    }

    #[test]
    fn a_press_lands_on_the_row_it_is_over() {
        let style = list();
        let rects = row_rects(ContextMenuShape::List, &style, at(100.0, 200.0), &LABELS);

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
        let rects = row_rects(ContextMenuShape::List, &style, at(100.0, 200.0), &LABELS);

        assert_eq!(
            row_at(ContextMenuShape::List, &rects, 110.0, 190.0),
            None,
            "the panel's padding is deliberate empty space"
        );
    }

    #[test]
    fn a_press_on_a_pills_rounded_end_belongs_to_the_item_beside_it() {
        let style = pill();
        let rects = row_rects(ContextMenuShape::Pill, &style, at(100.0, 200.0), &LABELS);
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

    #[test]
    fn the_union_of_the_rows_is_the_content_they_fill() {
        let style = pill();
        let rects = row_rects(ContextMenuShape::Pill, &style, at(100.0, 200.0), &LABELS);
        let bounds = union(&rects).expect("two rows have a union");
        let (width, height) = content_size(ContextMenuShape::Pill, &style, &LABELS);

        assert_eq!(bounds.x, 100.0);
        assert_eq!(bounds.width, width);
        assert_eq!(bounds.height, height);
        assert_eq!(union(&[]), None);
    }
}
