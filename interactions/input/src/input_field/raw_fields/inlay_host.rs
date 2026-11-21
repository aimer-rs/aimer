// The inlays of a field: the room left in its lines for them, and where each is drawn.
//
// A label is as wide as it is measured to be, in the font it is drawn in. The room is asked of the layout as the letter spacing of
// a span (see `crate::input_field::inlay`), and each label is drawn in it, on the baseline of the line, when the text is painted.

/// Where the inlays went for one text, in one font: the answer is kept, since a frame asks for it several times and measuring
/// every label is shaping text.
pub(super) struct InlayMemo {
    key: InlayKey,
    placed: Rc<[Placed]>,
}

/// What the room that was left depends on: which labels, in what text, and in which font, of the bytes of the text that was
/// laid out.
#[derive(PartialEq)]
struct InlayKey {
    inlays: u64,
    revision: u64,
    font_size_bits: u32,
    scale_bits: u32,
    family: aimer_cupid::font::FontFamily,
    weight: u16,
    base: usize,
    len: usize,
}

impl RawTextField {
    /// The inlays, if the field draws any: not a field that hides its text, which has no text to put them in.
    fn effective_inlays(&self) -> Option<&Inlays> {
        // What is laid out while an input method composes is the text without the composition, which the offsets of the inlays are not
        // of, so there are none until it is done.
        self.inlays
            .as_ref()
            .filter(|inlays| !inlays.is_empty() && !self.input_type.is_obscured() && !self.is_composing())
    }

    /// What tells one set of inlays from another in what the layout is keyed by: zero for none.
    fn inlay_signature(&self) -> u64 {
        self.effective_inlays().map_or(0, |inlays| inlays.id().rotate_left(17))
    }

    /// The style a label is drawn in: the field's, smaller, in the color the label has.
    fn inlay_style(&self, inlays: &Inlays, color: Color) -> TextStyle {
        let mut style = self.field_text_style();
        style.text_overflow = TextOverflow::Clip;
        style.color = color;
        if let Some(family) = inlays.text_family() {
            style.font_family = family;
        }
        let base = if self.text_style.font_size == 0 { 14.0 } else { self.text_style.font_size as f32 };
        style.font_size = (base * inlays.text_size()).round().max(1.0) as _;
        style
    }

    /// How wide the label is, and the pad it asks for, in logical pixels.
    fn inlay_width(&self, ctx: &BuildContext, inlays: &Inlays, inlay: &Inlay) -> f32 {
        let style = self.inlay_style(inlays, inlay.color());
        let scale = if ctx.scale > 0.0 { ctx.scale } else { 1.0 };
        let measured = ctx
            .canvas
            .measure_text_metrics_styled(
                inlay.label(),
                self.scaled_font_size(&style, ctx.scale),
                0.0,
                style.font_family,
                style.font_style,
                style.font_weight.numeric(),
            )
            .width
            / scale;
        let pad = |padded: bool| if padded { inlays.pad() } else { 0.0 };
        let inside = if inlay.has_background() { 2.0 * inlays.text_inset() } else { 0.0 };
        measured + inside + pad(inlay.pad_left()) + pad(inlay.pad_right())
    }

    /// Where the inlays go in `text`, which is the bytes `base..` of what the field holds, and the room each needs. Left out are the
    /// inlays that cannot be placed, and those that would be in one of `boxes`, which an embedded widget stands in the place of.
    fn placed_inlays(&self, ctx: &BuildContext, text: &str, base: usize, boxes: &[EmbedBox]) -> Rc<[Placed]> {
        let Some(inlays) = self.effective_inlays() else {
            return Rc::from(Vec::new());
        };
        let style = self.field_text_style();
        let key = InlayKey {
            inlays: inlays.id(),
            revision: self.controller.revision(),
            font_size_bits: self.scaled_font_size(&style, ctx.scale).to_bits(),
            scale_bits: ctx.scale.to_bits(),
            family: style.font_family,
            weight: style.font_weight.numeric(),
            base,
            len: text.len(),
        };
        if let Some(memo) = self.inlay_memo.borrow().as_ref()
            && memo.key == key
            && boxes.is_empty()
        {
            return Rc::clone(&memo.placed);
        }
        // Labels made for the text as it was are moved by what has been done to it since.
        let edit = inlays.text().and_then(|made_for| made_for.diff(&self.controller.value().snapshot()));
        let mut placed = inlay::place(inlays, text, base, edit.as_ref(), &mut |inlay| self.inlay_width(ctx, inlays, inlay));
        placed.retain(|place| {
            !boxes
                .iter()
                .any(|embed| place.span.start - base < embed.range.end && embed.range.start < place.span.end - base)
        });
        let placed: Rc<[Placed]> = placed.into();
        if boxes.is_empty() {
            *self.inlay_memo.borrow_mut() = Some(InlayMemo { key, placed: Rc::clone(&placed) });
        }
        placed
    }
}

impl RawTextField {
    /// Draws the inlays of the lines in view, each in the room that was left for it: in its pill, if it has one, and on the baseline
    /// of the line, inside the clip of the text, so that they scroll with it.
    fn paint_inlays(&self, canvas: &Canvas, ctx: &BuildContext, layout: &LocalFieldLayout) {
        let Some(inlays) = self.effective_inlays() else {
            return;
        };
        let geometry = &layout.geometry;
        let placed = &geometry.inlays;
        let Some(interaction) = geometry.interaction.as_ref() else {
            return;
        };
        if placed.is_empty() {
            return;
        }
        let scale = layout.scale;
        let window_top = geometry.window.as_ref().map_or(0.0, |window| window.y_base);
        let content_top = layout.outline_top + layout.padding_top;
        let text_top = content_top + layout.base_y - layout.scroll_y + window_top;
        let text_left = layout.outline_left + layout.padding_left - layout.scroll_x;
        // Only the rows that are in view are asked about: a window holds many more than are.
        let Some((from, to)) = self.visible_bytes(layout) else {
            return;
        };
        let first = placed.partition_point(|place| place.at < from);
        let last = placed.partition_point(|place| place.at <= to);
        for place in &placed[first..last] {
            let Some((gap_left, baseline)) = geometry.inlay_origin(place, scale) else {
                continue;
            };
            let mut left = text_left + gap_left;
            for (offset, width) in place.widths.iter().enumerate() {
                let inlay = &inlays.items()[place.items.start + offset];
                let width = width * scale;
                let pad = |padded: bool| if padded { inlays.pad() * scale } else { 0.0 };
                let (outer_left, outer_right) = (pad(inlay.pad_left()), pad(inlay.pad_right()));
                let pill_left = left + outer_left;
                let pill_width = (width - outer_left - outer_right).max(0.0);
                self.paint_inlay(canvas, ctx, inlays, inlay, (pill_left, text_top + baseline), pill_width, scale);
                left += width;
            }
        }
    }

    /// Draws one inlay: `left` is where its pill begins and `baseline` where the line stands, in physical pixels from the field's outer
    /// origin, and `width` how wide the pill is.
    fn paint_inlay(
        &self,
        canvas: &Canvas,
        ctx: &BuildContext,
        inlays: &Inlays,
        inlay: &Inlay,
        (left, baseline): (f32, f32),
        width: f32,
        scale: f32,
    ) {
        let style = self.inlay_style(inlays, inlay.color());
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
        let top = baseline - (metrics.ascent + metrics.line_gap * 0.5);
        let mut text_left = left;
        if let Some(color) = inlay.pill() {
            // A pill is a hair taller than its text, and never rounder than half of it.
            let (pad_y, rounded) = (scale.max(1.0), inlays.corner_radius() * scale);
            let (height, red, green, blue, alpha) = {
                let (red, green, blue, alpha) = color.to_rgba();
                (label_height + 2.0 * pad_y, red, green, blue, alpha)
            };
            let radius = rounded.min(height * 0.5).min(width * 0.5);
            canvas.fill_rect_styled(
                Rect::new(left / scale, (top - pad_y) / scale, width / scale, height / scale),
                aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha),
                [radius / scale; 4],
                [0.0; 4],
                aimer_cupid::utilities::Color::transparent(),
                [0.0; 4],
                aimer_cupid::utilities::Color::transparent(),
            );
            text_left += inlays.text_inset() * scale;
        }
        let widget = self.build_text_widget(inlay.label(), &style, TextAlign::TopLeft);
        let text_context = field_text_context(ctx, width + 2.0, label_height);
        let _ = widget.computed_size(&text_context);
        let origin = Vec2d { x: text_left / scale, y: top / scale };
        let recorded = widget.record_local_v2_into(canvas, &text_context, origin, false, |_| {});
        debug_assert!(recorded, "the label of an inlay is plain text");
    }
}
