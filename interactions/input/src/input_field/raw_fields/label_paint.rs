// The labels after the end of lines: where each is drawn, and where its buttons are, for a press to find.
//
// A label stands on the baseline of the last row of its line, a gap after the text of that row, in the text's own font a
// little smaller. It is drawn with the text, inside the clip of the text, so it scrolls with it and is cut off by the edge
// of the field.

/// Where a button of a label was drawn, in logical pixels from the field's outer origin, and what pressing it does.
#[derive(Clone)]
pub(crate) struct LabelHit {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub on_press: Rc<dyn Fn(crate::input_field::line_labels::FieldSpot)>,
}

impl RawTextField {
    /// Which button of a label, in the order they were drawn, the pointer is on: none when it is on no button, or off the field.
    fn hovered_label(&self) -> Option<usize> {
        let (pos, origin) = (self.pointer_pos.get()?, self.cached_bounds.get_bounds()?);
        let (x, y) = (pos.x - origin.x, pos.y - origin.y);
        self.label_hits
            .borrow()
            .iter()
            .position(|hit| x >= hit.x && x <= hit.x + hit.width && y >= hit.y && y <= hit.y + hit.height)
    }

    /// Whether `pos`, in the coordinates of the window, is on a button of a label.
    pub(crate) fn label_at(&self, pos: Vec2d) -> bool {
        let Some(origin) = self.cached_bounds.get_bounds() else {
            return false;
        };
        let (x, y) = (pos.x - origin.x, pos.y - origin.y);
        self.label_hits
            .borrow()
            .iter()
            .any(|hit| x >= hit.x && x <= hit.x + hit.width && y >= hit.y && y <= hit.y + hit.height)
    }

    /// Draws the labels of the lines in view, and notes where their buttons are.
    fn paint_line_labels(&self, canvas: &Canvas, ctx: &BuildContext, layout: &LocalFieldLayout) {
        // Where the pointer is is judged by where the buttons were the last time, which is where they are unless the text moved.
        let hovered = self.hovered_label();
        self.label_hits.borrow_mut().clear();
        let Some(labels) = self.line_labels.as_ref() else {
            return;
        };
        let Some(interaction) = layout.geometry.interaction.as_ref() else {
            return;
        };
        let scale = layout.scale;
        let value = self.controller.value();
        let text = value.snapshot();
        let window_top = layout.geometry.window.as_ref().map_or(0.0, |window| window.y_base);
        let content_top = layout.outline_top + layout.padding_top;
        let text_top = content_top + layout.base_y - layout.scroll_y + window_top;
        let text_left = layout.outline_left + layout.padding_left - layout.scroll_x;
        let rows = &layout.geometry.visual_lines;
        let base_size = if self.text_style.font_size == 0 { 14.0 } else { self.text_style.font_size as f32 };
        for (index, (row, line)) in rows.iter().zip(&interaction.lines).enumerate() {
            // A window holds many more rows than are in view, and a row that is clipped away is not asked about.
            let row_height = (line.ascent - line.descent + line.line_gap).max(1.0);
            let row_top = text_top + line.baseline - line.ascent - line.line_gap * 0.5;
            if row_top + row_height <= content_top || row_top >= content_top + layout.content_height {
                continue;
            }
            let number = text.byte_to_line(row.byte_start.min(text.len()));
            // A line that wraps has its label after its last row.
            let last = rows
                .get(index + 1)
                .is_none_or(|next| text.byte_to_line(next.byte_start.min(text.len())) != number);
            if !last {
                continue;
            }
            let Some(label) = (labels.of_line)(number) else {
                continue;
            };
            let mut style = self.field_text_style();
            style.text_overflow = TextOverflow::Clip;
            style.color = label.color;
            if let Some(family) = label.family {
                style.font_family = family;
            }
            style.font_size = (base_size * label.size).round().max(1.0) as _;
            let font_size = self.scaled_font_size(&style, scale);
            let metrics = ctx.canvas.measure_text_metrics_styled(
                "0",
                font_size,
                0.0,
                style.font_family,
                style.font_style,
                style.font_weight.numeric(),
            );
            let label_height = (metrics.ascent - metrics.descent + metrics.line_gap).max(1.0);
            let top = text_top + line.baseline - (metrics.ascent + metrics.line_gap * 0.5);
            // End-of-line inlay hints are drawn after the last character without widening `line.width`,
            // so start the label after them.
            let mut line_end = line.width;
            if let Ok(found) = layout
                .geometry
                .inlays
                .binary_search_by_key(&row.byte_end, |place| place.at)
            {
                let place = &layout.geometry.inlays[found];
                if place.eol {
                    let inlay_end = layout
                        .geometry
                        .inlay_origin(place, scale)
                        .map_or(line.width + place.gap * scale, |(gap_left, _)| {
                            gap_left + place.gap * scale
                        });
                    line_end = line_end.max(inlay_end);
                }
            }
            let mut left = text_left + line_end + labels.gap * scale;
            for part in &label.parts {
                let width = ctx
                    .canvas
                    .measure_text_metrics_styled(
                        &part.text,
                        font_size,
                        0.0,
                        style.font_family,
                        style.font_style,
                        style.font_weight.numeric(),
                    )
                    .width;
                // A button that the pointer is on is in the color the label says, and underlined.
                let under_pointer = part.press.is_some() && hovered == Some(self.label_hits.borrow().len());
                let mut part_style = style;
                let hot = label.hover.filter(|_| under_pointer);
                if let Some(color) = hot {
                    part_style.color = color;
                }
                let widget = self.build_text_widget(&part.text, &part_style, TextAlign::TopLeft);
                let text_context = field_text_context(ctx, width + 2.0, label_height);
                let _ = widget.computed_size(&text_context);
                let origin = Vec2d { x: left / scale, y: top / scale };
                let recorded = widget.record_local_v2_into(canvas, &text_context, origin, false, |_| {});
                debug_assert!(recorded, "the label of a line is plain text");
                if let Some(color) = hot {
                    let (red, green, blue, alpha) = color.to_rgba();
                    let thickness = scale.max(1.0);
                    canvas.fill_rect(
                        Rect::new(left / scale, (text_top + line.baseline + 2.0 * scale) / scale, width / scale, thickness / scale),
                        [red, green, blue, alpha],
                    );
                }
                if let Some(press) = &part.press {
                    self.label_hits.borrow_mut().push(LabelHit {
                        x: left / scale,
                        y: top / scale,
                        width: width / scale,
                        height: label_height / scale,
                        on_press: Rc::clone(press),
                    });
                }
                left += width;
            }
        }
    }
}
