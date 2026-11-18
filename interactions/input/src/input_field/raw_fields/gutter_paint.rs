// The margin of line numbers and the band behind the caret's line: how wide the margin is, and
// how both are painted.
//
// The margin is part of the field's left padding, so everything that places text, the caret, a
// click or a scroll by the padding is right without knowing the margin is there. The numbers are
// drawn as one block of text, a number on the first row of each line and an empty row for the rows
// a wrapped line goes on to, so that they are as far apart as the text's rows.

/// How wide a digit is, as a share of the font size. A monospace face is about 0.6 of it, and the
/// margin is made a little wider than that so a proportional face's widest digit fits too.
const DIGIT_ADVANCE: f32 = 0.62;

/// Where an icon of the margin was drawn, in logical pixels from the field's outer origin, and what
/// pressing it does.
#[derive(Clone)]
pub(crate) struct MarkerHit {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub on_press: Rc<dyn Fn()>,
}

impl RawTextField {
    /// The width of the column of icons before the numbers, in physical pixels: none without them.
    fn marker_column_width(&self, scale: f32) -> f32 {
        self.gutter_markers.as_ref().map_or(0.0, |markers| markers.size * scale)
    }

    /// The width of the margin of line numbers, in physical pixels: none without line numbers.
    fn gutter_width(&self, scale: f32) -> f32 {
        let Some(numbers) = self.line_numbers else {
            return 0.0;
        };
        let digits = digits_of(self.controller.value().line_count()).max(numbers.min_digits);
        let digit = self.scaled_font_size(&self.text_style, scale) * DIGIT_ADVANCE;
        (numbers.gap_before + numbers.gap_after) * scale + digits as f32 * digit + self.marker_column_width(scale)
    }

    /// What pressing the point `pos` (window coordinates) does, if it is on an icon of the margin.
    pub(crate) fn gutter_marker_at(&self, pos: Vec2d) -> Option<Rc<dyn Fn()>> {
        let origin = self.cached_bounds.get_bounds()?;
        let (x, y) = (pos.x - origin.x, pos.y - origin.y);
        self.marker_hits
            .borrow()
            .iter()
            .find(|hit| x >= hit.x && x <= hit.x + hit.size && y >= hit.y && y <= hit.y + hit.size)
            .map(|hit| Rc::clone(&hit.on_press))
    }

    /// The left padding in physical pixels: the one asked for, and the margin of line numbers.
    #[inline]
    fn padding_left_px(&self, width: f32, scale: f32) -> f32 {
        self.padding.left.value(width, scale) + self.gutter_width(scale)
    }

    /// Fills the band behind the line the caret is on, from the left edge of the field to its right
    /// edge. Drawn before the margin and the text, so both are on top of it.
    fn paint_current_line(&self, canvas: &Canvas, layout: &LocalFieldLayout) {
        let Some(color) = self.current_line else {
            return;
        };
        let Some(caret) = layout.geometry.caret_geometry(self.cursor.offset()) else {
            return;
        };
        // A caret in the lines an embedded widget stands in for is not in any row the user can see, so
        // there is no line to mark.
        if layout.geometry.embeds.iter().any(|embed| embed.range.start <= caret.offset && caret.offset < embed.range.end.max(embed.range.start + 1)) {
            return;
        }
        let scale = layout.scale;
        let top = layout.outline_top + layout.padding_top + layout.base_y + caret.y - layout.scroll_y;
        let height = line_box_height(caret.height, layout.line_height);
        let (view_top, view_bottom) = (layout.outline_top + layout.padding_top, layout.outline_top + layout.padding_top + layout.content_height);
        if top + height <= view_top || top >= view_bottom {
            return;
        }
        // Only the part inside the rows the field shows.
        let (clipped_top, clipped_bottom) = (top.max(view_top), (top + height).min(view_bottom));
        let (red, green, blue, alpha) = color.to_rgba();
        canvas.fill_rect(
            Rect::new(
                layout.outline_left / scale,
                clipped_top / scale,
                layout.box_width / scale,
                (clipped_bottom - clipped_top) / scale,
            ),
            [red, green, blue, alpha],
        );
    }

    /// Draws the line numbers in the margin: each on the first row of its line, standing on that
    /// row's baseline. A row's height is its own (a rendered heading is taller than the code around
    /// it), so each number is placed by the row it belongs to and not by counting rows of one height.
    fn paint_gutter(&self, canvas: &Canvas, ctx: &BuildContext, layout: &LocalFieldLayout) {
        let Some(numbers) = self.line_numbers else {
            return;
        };
        let scale = layout.scale;
        let value = self.controller.value();
        let text = value.snapshot();
        let caret_line = text.byte_to_line(value.byte_of_grapheme(self.cursor.offset()));

        let gutter_width = self.gutter_width(scale);
        let inner_width = (gutter_width - (numbers.gap_before + numbers.gap_after) * scale).max(1.0);
        let markers_left = layout.outline_left + layout.padding_left - gutter_width + numbers.gap_before * scale;
        let marker_column = self.marker_column_width(scale);
        let left = markers_left + marker_column;
        let inner_width = (inner_width - marker_column).max(1.0);
        self.marker_hits.borrow_mut().clear();
        let mut style = self.field_text_style();
        style.text_overflow = TextOverflow::Clip;

        // The numbers' own text metrics: where a number's baseline is in its box.
        let font_size = self.scaled_font_size(&self.text_style, scale);
        let metrics = ctx.canvas.measure_text_metrics_styled(
            "0",
            font_size,
            0.0,
            style.font_family,
            style.font_style,
            style.font_weight.numeric(),
        );
        let number_top_to_baseline = metrics.ascent + metrics.line_gap * 0.5;
        let number_height = (metrics.ascent - metrics.descent + metrics.line_gap).max(1.0);
        // How tall a line of the field's own text is: a row far taller than it has an embedded widget in it.
        let plain_line = metrics.line_height.max(number_height);

        let window_top = layout.geometry.window.as_ref().map_or(0.0, |window| window.y_base);
        let content_top = layout.outline_top + layout.padding_top;
        let text_top = content_top + layout.base_y - layout.scroll_y + window_top;

        canvas.push_clip(
            Rect::new(
                (layout.outline_left + layout.padding_left - gutter_width) / scale,
                content_top / scale,
                gutter_width / scale,
                layout.content_height / scale,
            ),
            [0.0; 4].into(),
        );
        // `row` is a visual row of the laid-out lines: a number on the first row of a line.
        let rows: Vec<(usize, f32, f32, f32, bool)> = match layout.geometry.interaction.as_ref() {
            Some(interaction) if !layout.geometry.visual_lines.is_empty() => layout
                .geometry
                .visual_lines
                .iter()
                .zip(&interaction.lines)
                .filter_map(|(row, line)| {
                    let number = text.byte_to_line(row.byte_start.min(text.len()));
                    // The row's own box: its top, from the baseline, and how tall it is.
                    (text.line_range(number).start == row.byte_start).then_some((
                        number,
                        line.baseline,
                        (line.ascent - line.descent + line.line_gap).max(1.0),
                        // Where the row's box starts, above its baseline.
                        line.baseline - line.ascent - line.line_gap * 0.5,
                        // Whether an embedded widget stands in this row.
                        layout.geometry.embeds.iter().any(|embed| embed.range.start < row.byte_end.max(row.byte_start + 1) && row.byte_start < embed.range.end),
                    ))
                })
                .collect(),
            // An empty text still has its first line, and its number.
            _ => vec![(0, layout.line_height, layout.line_height, 0.0, false)],
        };
        for (number, baseline, row_height, row_top, embedded) in rows {
            // A row an embedded widget stands in for is taller than a line and has its number on
            // its first line, at the top, and not on the baseline at the bottom.
            let tall = embedded && row_height > plain_line * 1.3;
            let top = if tall {
                text_top + row_top + (plain_line - number_height) * 0.5
            } else {
                text_top + baseline - number_top_to_baseline
            };
            if top + number_height <= content_top || top >= content_top + layout.content_height {
                continue;
            }
            style.color = if number == caret_line { numbers.active_color } else { numbers.color };
            let label = (number + 1).to_string();
            let widget = self.build_text_widget(&label, &style, TextAlign::TopRight);
            let text_context = field_text_context(ctx, inner_width, number_height);
            let _ = widget.computed_size(&text_context);
            let origin = Vec2d { x: left / scale, y: top / scale };
            let recorded = widget.record_local_v2_into(canvas, &text_context, origin, false, |_| {});
            debug_assert!(recorded, "the line numbers are plain text");
            let (marker_height, marker_baseline) = if tall { (plain_line, row_top + plain_line) } else { (row_height, baseline) };
            self.paint_marker(canvas, number, text_top, marker_baseline, marker_height, markers_left, scale);
        }
        canvas.pop_clip();
    }

    /// Draws the icon of line `number`, if it has one, centred on the height of its row, and notes where
    /// it is for a press to find.
    #[allow(clippy::too_many_arguments)]
    fn paint_marker(&self, canvas: &Canvas, number: usize, text_top: f32, baseline: f32, row_height: f32, left: f32, scale: f32) {
        let Some(markers) = self.gutter_markers.as_ref() else {
            return;
        };
        let Some(marker) = (markers.of_line)(number) else {
            return;
        };
        // The row's box is the baseline less the line's ascent and half its gap; the icon is centred on
        // the middle of it, which is a little above the baseline, as a lower-case letter's body is.
        let size = markers.size;
        let center_y = (text_top + baseline - row_height * 0.5 + row_height * 0.18) / scale;
        let rect = Rect::new(left / scale, center_y - size * 0.5, size, size);
        canvas.draw_svg(Arc::clone(&marker.scene), rect, Arc::from(Vec::new()));
        self.marker_hits.borrow_mut().push(MarkerHit { x: rect.x, y: rect.y, size, on_press: marker.on_press });
    }

    /// Draws the bars beside the lines that have one: a bar for each row of the line, as tall as the
    /// row, so the bars of a line's rows and of the lines after it are one rule. They are under the
    /// text, and scroll sideways with it.
    fn paint_line_bars(&self, canvas: &Canvas, layout: &LocalFieldLayout) {
        let Some(bars) = self.line_bars.as_ref() else {
            return;
        };
        let Some(interaction) = layout.geometry.interaction.as_ref() else {
            return;
        };
        let scale = layout.scale;
        let value = self.controller.value();
        let text = value.snapshot();
        let context = self.highlight_context(&|| text.slice(0..text.len()), None);
        let window_top = layout.geometry.window.as_ref().map_or(0.0, |window| window.y_base);
        let content_top = layout.outline_top + layout.padding_top;
        let text_top = content_top + layout.base_y - layout.scroll_y + window_top;
        let text_left = layout.outline_left + layout.padding_left - self.scroll_x_offset();
        let content_left = layout.outline_left + layout.padding_left;

        canvas.push_clip(
            Rect::new(content_left / scale, content_top / scale, layout.content_width / scale, layout.content_height / scale),
            [0.0; 4].into(),
        );
        // The line a row belongs to, and what it was asked, so a line that wraps is asked once.
        let mut asked: Option<(usize, Option<LineBar>)> = None;
        for (row, line) in layout.geometry.visual_lines.iter().zip(&interaction.lines) {
            let number = text.byte_to_line(row.byte_start.min(text.len()));
            let bar = match asked {
                Some((line_number, bar)) if line_number == number => bar,
                _ => {
                    let bar = (bars.of_line)(number, &context);
                    asked = Some((number, bar));
                    bar
                }
            };
            let Some(bar) = bar else {
                continue;
            };
            let height = (line.ascent - line.descent + line.line_gap).max(1.0);
            let top = text_top + line.baseline - line.ascent - line.line_gap * 0.5;
            if top + height <= content_top || top >= content_top + layout.content_height {
                continue;
            }
            let (red, green, blue, alpha) = bar.color.to_rgba();
            canvas.fill_rect(
                Rect::new((text_left + bar.offset * scale) / scale, top / scale, bars.width, height / scale),
                [red, green, blue, alpha],
            );
        }
        canvas.pop_clip();
    }
}
