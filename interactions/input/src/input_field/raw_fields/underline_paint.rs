// The lines under ranges of the text: where each is, and the bar that is drawn for it on each line it is on.
//
// A bar is as wide as the range is on its line, a little above the bottom of the line, with rounded ends. It is drawn over the text
// with the inlays, inside the clip of the text, so it scrolls with it.

impl RawTextField {
    /// The underlines, if the field draws any: not a field that hides its text, which has nothing to point at.
    fn effective_underlines(&self) -> Option<&Underlines> {
        self.underlines.as_ref().filter(|underlines| !underlines.is_empty() && !self.input_type.is_obscured())
    }

    /// The bytes of the rows of the layout that are in view, from the first to the end of the last: what a window holds is much more
    /// than is shown, and only what is shown is asked about. `None` where no row is in view.
    fn visible_bytes(&self, layout: &LocalFieldLayout) -> Option<(usize, usize)> {
        let interaction = layout.geometry.interaction.as_ref()?;
        let window_top = layout.geometry.window.as_ref().map_or(0.0, |window| window.y_base);
        let content_top = layout.outline_top + layout.padding_top;
        let text_top = content_top + layout.base_y - layout.scroll_y + window_top;
        let (clip_top, clip_height) = self.clip_rows(layout);
        let mut seen: Option<(usize, usize)> = None;
        for (row, line) in layout.geometry.visual_lines.iter().zip(&interaction.lines) {
            let row_height = (line.ascent - line.descent + line.line_gap).max(1.0);
            let row_top = text_top + line.baseline - line.ascent - line.line_gap * 0.5;
            if row_top + row_height <= clip_top || row_top >= clip_top + clip_height {
                continue;
            }
            seen = Some(seen.map_or((row.byte_start, row.byte_end), |(from, to)| (from.min(row.byte_start), to.max(row.byte_end))));
        }
        seen
    }

    /// Draws the lines under the ranges of the underlines that are in view.
    fn paint_underlines(&self, canvas: &Canvas, layout: &LocalFieldLayout) {
        let Some(underlines) = self.effective_underlines() else {
            return;
        };
        let Some((from, to)) = self.visible_bytes(layout) else {
            return;
        };
        let value = self.controller.value();
        let len = value.len();
        // Ranges made for the text as it was are moved by what has been done to it since.
        let edit = underlines.text().and_then(|made_for| made_for.diff(&value.snapshot()));
        let scale = layout.scale;
        let content_top = layout.outline_top + layout.padding_top;
        let (clip_top, clip_height) = self.clip_rows(layout);
        // What the geometry says of a range is in the coordinates of the whole text, a window's offset included.
        let text_top = content_top + layout.base_y - layout.scroll_y;
        let text_left = layout.outline_left + layout.padding_left - layout.scroll_x;
        let thickness = underlines.line_thickness() * scale;
        let radius = (underlines.corner_radius() * scale).min(thickness * 0.5);
        for item in underlines.items() {
            let Some(range) = underline::moved(&item.range(), edit.as_ref()) else {
                continue;
            };
            let range = range.start.min(len)..range.end.min(len);
            if range.start >= range.end || range.end <= from || range.start >= to {
                continue;
            }
            let (red, green, blue, alpha) = item.color().to_rgba();
            let color = aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha);
            let rects = layout.geometry.selection_rects(value.grapheme_of_byte(range.start), value.grapheme_of_byte(range.end));
            for rect in rects {
                if rect.width <= 0.0 {
                    continue;
                }
                // A line of the range that is out of view is not drawn.
                let band_top = text_top + rect.y;
                if band_top + rect.height <= clip_top || band_top >= clip_top + clip_height {
                    continue;
                }
                // A little above the bottom of the line, which is where a line under a name is read.
                let top = band_top + rect.height - thickness - scale;
                canvas.fill_rect_styled(
                    Rect::new((text_left + rect.x) / scale, top / scale, rect.width / scale, thickness / scale),
                    color,
                    [radius / scale; 4],
                    [0.0; 4],
                    aimer_cupid::utilities::Color::transparent(),
                    [0.0; 4],
                    aimer_cupid::utilities::Color::transparent(),
                );
            }
        }
    }
}
