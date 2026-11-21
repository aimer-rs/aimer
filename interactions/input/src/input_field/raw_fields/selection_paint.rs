// The selection: a rectangle for each line, or, where the field was asked to round it, one shape around all of it.

use crate::input_field::selection_shape;

/// How far a selected line break reaches past the end of its line, as a share of the height of the line.
const BREAK_REACH: f32 = 0.4;

/// How many lines past what shows are made into the outline, so that where the outline is cut is out of sight.
const OFFSCREEN_ROWS: usize = 2;

impl RawTextField {
    /// Paints the selection from `start` to `end` (grapheme offsets) behind the text, with the text's top left at `origin` in
    /// logical pixels. A field that was not asked to round it, or whose selection does not have a rectangle to a line, gets a
    /// square one for each line, as it always did.
    fn paint_selection(&self, canvas: &aimer_canvas::Canvas, layout: &LocalFieldLayout, origin: Vec2d, scale: f32, start: usize, end: usize) {
        let color = self.selection_color.to_rgba();
        let rects = layout.geometry.selection_rects(start, end);
        if self.selection_radius > 0.0 && self.paint_rounded_selection(canvas, layout, origin, scale, end, &rects, color) {
            return;
        }
        for rect in rects {
            canvas.fill_rect(
                Rect::new(origin.x + rect.x / scale, origin.y + rect.y / scale, rect.width / scale, rect.height / scale),
                [color.0, color.1, color.2, color.3],
            );
        }
    }

    /// Paints the selection as one rounded shape. Whether it did: it does not where the rectangles are not one to a line.
    fn paint_rounded_selection(
        &self,
        canvas: &aimer_canvas::Canvas,
        layout: &LocalFieldLayout,
        origin: Vec2d,
        scale: f32,
        end: usize,
        rects: &[aimer_cupid::text_layout::SelectionRect],
        color: (u8, u8, u8, u8),
    ) -> bool {
        let rects: Vec<(f32, f32, f32, f32)> = rects.iter().map(|rect| (rect.x, rect.y, rect.width, rect.height)).collect();
        let Some(rows) = selection_shape::rows_of(&rects, scale, BREAK_REACH, self.selection_ends_after_break(end)) else {
            return false;
        };
        // What shows, in the coordinates of the rectangles: the text starts `base_y` down in its box, and is scrolled.
        let top = (layout.scroll_y - layout.base_y) / scale;
        let rows = selection_shape::visible(&rows, top, top + layout.content_height / scale, OFFSCREEN_ROWS);
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
            return true;
        };
        // The scene is in the pixels of the box around the rows, which is where it is drawn.
        let left = rows.iter().fold(f32::INFINITY, |least, row| least.min(row.left));
        let right = rows.iter().fold(f32::NEG_INFINITY, |most, row| most.max(row.right));
        let (width, height) = (right - left, last.bottom - first.top);
        if !(width > 0.0 && height > 0.0) {
            return true;
        }
        let local: Vec<selection_shape::Row> = rows
            .iter()
            .map(|row| selection_shape::Row { left: row.left - left, right: row.right - left, top: row.top - first.top, bottom: row.bottom - first.top })
            .collect();
        let path = selection_shape::outline(&local, self.selection_radius);
        let scene = selection_shape::scene(path, width, height, aimer_cupid::svg::SvgColor::rgba8(color.0, color.1, color.2, color.3));
        canvas.draw_svg(Arc::new(scene), Rect::new(origin.x + left, origin.y + first.top, width, height), Arc::from([]));
        true
    }

    /// Whether the selection ends after a line break: the character before its end, in grapheme offset `end`, is one.
    fn selection_ends_after_break(&self, end: usize) -> bool {
        let value = self.controller.value();
        let byte = value.byte_of_grapheme(end);
        byte > 0 && value.snapshot().byte_at(byte - 1) == Some(b'\n')
    }
}
