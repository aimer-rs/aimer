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
        let icon = self
            .marker_hits
            .borrow()
            .iter()
            .find(|hit| x >= hit.x && x <= hit.x + hit.size && y >= hit.y && y <= hit.y + hit.size)
            .map(|hit| Rc::clone(&hit.on_press));
        // A button of a label after a line is pressed as an icon of the margin is: it is called with where it is.
        icon.or_else(|| {
            self.label_hits
                .borrow()
                .iter()
                .find(|hit| x >= hit.x && x <= hit.x + hit.width && y >= hit.y && y <= hit.y + hit.height)
                .map(|hit| {
                    let press = Rc::clone(&hit.on_press);
                    let spot = crate::input_field::line_labels::FieldSpot {
                        x: hit.x,
                        y: hit.y,
                        width: hit.width,
                        height: hit.height,
                        extent: Some((origin.width, origin.height)),
                    };
                    Rc::new(move || press(spot)) as Rc<dyn Fn()>
                })
        })
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
        // With centered numbers the icons are centered too: in the room before the numbers' column (the
        // gap and the icons' own column), as far from the left edge of the margin as from the numbers.
        let icons_left = if numbers.align == LineNumberAlign::Center { markers_left - numbers.gap_before * scale * 0.5 } else { markers_left };
        self.marker_hits.borrow_mut().clear();
        let mut style = self.field_text_style();
        style.text_overflow = TextOverflow::Clip;
        let align = match numbers.align {
            LineNumberAlign::Left => TextAlign::TopLeft,
            LineNumberAlign::Center => TextAlign::TopCenter,
            LineNumberAlign::Right => TextAlign::TopRight,
        };

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
        // Where a row's number would be, and whether it is in view. A row an embedded widget stands in for
        // is taller than a line and has its number at the top; any other has it on the baseline.
        let plain_top = |baseline: f32| text_top + baseline - number_top_to_baseline;
        let tall_top = |row_top: f32| text_top + row_top + (plain_line - number_height) * 0.5;
        let in_view = |top: f32| top + number_height > content_top && top < content_top + layout.content_height;
        // `row` is a visual row of the laid-out lines: a number on the first row of a line.
        let rows: Vec<(usize, f32, f32, f32, bool)> = match layout.geometry.interaction.as_ref() {
            Some(interaction) if !layout.geometry.visual_lines.is_empty() => layout
                .geometry
                .visual_lines
                .iter()
                .zip(&interaction.lines)
                .filter_map(|(row, line)| {
                    // The row's own box: its top, from the baseline, and how tall it is.
                    let row_height = (line.ascent - line.descent + line.line_gap).max(1.0);
                    // Where the row's box starts, above its baseline.
                    let row_top = line.baseline - line.ascent - line.line_gap * 0.5;
                    let may_be_tall = row_height > plain_line * 1.3;
                    // A window holds many more rows than are in view. What is out of view is left before
                    // anything is asked of the text, which is a walk of its tree for each row.
                    if !in_view(plain_top(line.baseline)) && !(may_be_tall && in_view(tall_top(row_top))) {
                        return None;
                    }
                    let number = text.byte_to_line(row.byte_start.min(text.len()));
                    (text.line_range(number).start == row.byte_start).then_some((
                        number,
                        line.baseline,
                        row_height,
                        row_top,
                        // Whether an embedded widget stands in this row: only a row that is tall can be
                        // drawn for one, so only for those is it looked for.
                        may_be_tall && layout.geometry.embeds.iter().any(|embed| embed.range.start < row.byte_end.max(row.byte_start + 1) && row.byte_start < embed.range.end),
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
            let top = if tall { tall_top(row_top) } else { plain_top(baseline) };
            if !in_view(top) {
                continue;
            }
            style.color = if number == caret_line { numbers.active_color } else { numbers.color };
            let label = (number + 1).to_string();
            let widget = self.build_text_widget(&label, &style, align);
            let text_context = field_text_context(ctx, inner_width, number_height);
            let _ = widget.computed_size(&text_context);
            let origin = Vec2d { x: left / scale, y: top / scale };
            let recorded = widget.record_local_v2_into(canvas, &text_context, origin, false, |_| {});
            debug_assert!(recorded, "the line numbers are plain text");
            self.paint_marker(canvas, number, top + number_height * 0.5, icons_left, scale);
        }
        canvas.pop_clip();
    }

    /// Draws the icon of line `number`, if it has one, centred on `center_y` (physical pixels, from the
    /// field's outer origin), and notes where it is for a press to find. `center_y` is the middle of the
    /// number's own box, so the icon is level with its number on any row, plain or tall.
    fn paint_marker(&self, canvas: &Canvas, number: usize, center_y: f32, left: f32, scale: f32) {
        let Some(markers) = self.gutter_markers.as_ref() else {
            return;
        };
        let Some(marker) = (markers.of_line)(number) else {
            return;
        };
        let size = markers.size;
        let rect = Rect::new(left / scale, center_y / scale - size * 0.5, size, size);
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
        // The line a row belongs to, and what it was asked, so a line that wraps is asked once. The list is kept from line to
        // line, and is made empty before each is asked.
        let mut asked: Option<usize> = None;
        let mut list: Vec<LineBar> = Vec::new();
        for (row, line) in layout.geometry.visual_lines.iter().zip(&interaction.lines) {
            // A window holds many more rows than are in view, and a row that is clipped away is not
            // asked about: finding its line and its bar is for nothing.
            let height = (line.ascent - line.descent + line.line_gap).max(1.0);
            let top = text_top + line.baseline - line.ascent - line.line_gap * 0.5;
            if top + height <= content_top || top >= content_top + layout.content_height {
                continue;
            }
            let number = text.byte_to_line(row.byte_start.min(text.len()));
            if asked != Some(number) {
                list.clear();
                (bars.of_line)(number, &context, &mut list);
                asked = Some(number);
            }
            for bar in &list {
                let (red, green, blue, alpha) = bar.color.to_rgba();
                canvas.fill_rect(
                    Rect::new((text_left + bar.offset * scale) / scale, top / scale, bar.width.unwrap_or(bars.width), height / scale),
                    [red, green, blue, alpha],
                );
            }
        }
        canvas.pop_clip();
    }
}
