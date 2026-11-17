// Laying out a window of a long text: the field's side of it.
//
// A field with a long text does not lay all of it out. It keeps an index of the text's
// hard lines (where they are, how tall, how many graphemes), and a geometry of only the
// lines near what is on screen; see `editable_text::windowed`. This is what decides
// which lines, and keeps the text where it is when what was estimated of the lines
// above is corrected.

/// How far past the viewport, each way, a window reaches, in viewports, at least.
const WINDOW_MARGIN_VIEWPORTS: f64 = 1.0;

/// The least a window reaches past the viewport, each way, in physical pixels: a field
/// of a few lines has a window of more than a few.
const WINDOW_MARGIN_MIN: f64 = 600.0;

impl RawTextField {
    /// The text to lay a window of, when the field windows its text: shared, not copied.
    ///
    /// A field does when its text is long, wraps and shows what was typed, and no input
    /// method's composition is in progress, in which case it is the text without it.
    fn windowed_display(&self) -> Option<Arc<str>> {
        if !self.wraps_text() || self.input_type.is_obscured() {
            return None;
        }
        let value = self.controller.value();
        if value.text().len() < self.window_threshold {
            return None;
        }
        let Some(composing) = value.composing() else {
            return Some(Arc::clone(value.text_arc()));
        };
        // While an input method composes, what is laid out is the text without the composition,
        // which is painted on its own at the caret. That is a copy of the text, made once for
        // each change of it and not for each of the times a frame asks.
        let revision = self.controller.revision();
        if let Some((memo_revision, text)) = self.composition_display_memo.borrow().as_ref()
            && *memo_revision == revision
        {
            return Some(Arc::clone(text));
        }
        let source = value.text();
        let mut display = String::with_capacity(source.len() - (composing.end() - composing.start()));
        display.push_str(&source[..composing.start()]);
        display.push_str(&source[composing.end()..]);
        let display: Arc<str> = Arc::from(display);
        *self.composition_display_memo.borrow_mut() = Some((revision, Arc::clone(&display)));
        Some(display)
    }

    /// What the retained paint of a windowed text depends on beyond the text: which lines were laid
    /// out, and the heights that placed them.
    fn window_signature(&self) -> Option<((usize, usize), u64)> {
        let lines = self.window_lines.get()?;
        let index = self.line_index.borrow();
        Some((lines, index.as_ref()?.version()))
    }

    /// The index of `text`, made from the one the field had, if any.
    fn refreshed_line_index(&self, text: &Arc<str>, metrics: LineMetrics) -> Rc<LineIndex> {
        let mut slot = self.line_index.borrow_mut();
        let index = match slot.as_ref() {
            Some(index) if Arc::ptr_eq(index.text(), text) && index.metrics().same_wrapping(&metrics) => {
                Rc::clone(index)
            }
            Some(index) => Rc::new(index.updated(Arc::clone(text), metrics)),
            None => Rc::new(LineIndex::new(Arc::clone(text), metrics)),
        };
        *slot = Some(Rc::clone(&index));
        index
    }

    /// The part of the text that is on screen, from top to bottom, in the heights of the
    /// text stacked.
    fn visible_text_span(&self, ctx: &BuildContext, line_height: f32) -> (f64, f64) {
        if let Some(max_lines) = self.max_lines {
            // A field that scrolls inside itself shows `max_lines` lines from where it has scrolled to.
            let top = f64::from(self.scroll_y.get().max(0.0));
            return (top, top + max_lines as f64 * f64::from(line_height));
        }
        // A field that grows with its text is scrolled by what holds it, which says what
        // part of it can be seen.
        match ctx.visible_rect() {
            Some((_, y, _, height)) => (f64::from(y.max(0.0)), f64::from((y + height).max(0.0))),
            None => (0.0, f64::from(ctx.box_constraint.max_height).clamp(line_height as f64, 4000.0)),
        }
    }

    /// The hard lines to lay out for the viewport `top..bottom`: those that cover it and a
    /// margin, kept as they were while they still cover half of the margin, so that
    /// scrolling a little does not lay a window out again.
    fn window_lines_for(&self, index: &LineIndex, top: f64, bottom: f64) -> Range<usize> {
        let margin = ((bottom - top) * WINDOW_MARGIN_VIEWPORTS).max(WINDOW_MARGIN_MIN);
        let total = index.total_height();
        let covering = index.lines_between((top - margin / 2.0).max(0.0), (bottom + margin / 2.0).min(total));
        if let Some((start, end)) = self.window_lines.get()
            && end <= index.len()
            && start <= covering.start
            && covering.end <= end
        {
            return start..end;
        }
        let window = index.lines_between((top - margin).max(0.0), (bottom + margin).min(total));
        self.window_lines.set(Some((window.start, window.end)));
        window
    }

    /// The hard lines of `lines`, grown to take in every embed that any of them touches:
    /// an embed is laid out whole, and it can take many source lines.
    fn lines_with_whole_embeds(index: &LineIndex, mut lines: Range<usize>, embeds: &[TextEmbed]) -> Range<usize> {
        loop {
            let bytes = index.line_bytes(lines.start).start..index.line_bytes(lines.end - 1).end;
            let mut grown = false;
            for embed in embeds {
                let range = embed.range();
                if range.start >= range.end || range.start >= bytes.end || bytes.start >= range.end {
                    continue;
                }
                let first = index.line_of_byte(range.start);
                let last = (index.line_of_byte(range.end - 1) + 1).min(index.len());
                if first < lines.start {
                    lines.start = first;
                    grown = true;
                }
                if last > lines.end {
                    lines.end = last;
                    grown = true;
                }
            }
            if !grown {
                return lines;
            }
        }
    }

    /// The embeds that touch `bytes`, each given the box its widget measures, in the offsets of
    /// the text from `bytes.start`. The same checks as for a whole text, against the whole
    /// text, and only for these.
    fn window_embed_boxes(
        &self,
        ctx: &BuildContext,
        content_width: f32,
        index: &LineIndex,
        bytes: &Range<usize>,
        embeds: &[TextEmbed],
    ) -> Vec<EmbedBox> {
        if embeds.is_empty() {
            return Vec::new();
        }
        let mut inside: Vec<&TextEmbed> = embeds
            .iter()
            .filter(|embed| {
                let range = embed.range();
                range.start < bytes.end && bytes.start < range.end
            })
            .collect();
        inside.sort_by_key(|embed| embed.range().start);
        let mut boxes: Vec<EmbedBox> = Vec::with_capacity(inside.len());
        let mut end = 0;
        for embed in inside {
            let range = embed.range();
            if range.start >= range.end
                || range.start < end
                || range.end > index.text().len()
                || !index.text().is_char_boundary(range.start)
                || !index.text().is_char_boundary(range.end)
            {
                continue;
            }
            end = range.end;
            boxes.push(EmbedBox {
                key: embed.key(),
                range: range.start - bytes.start..range.end - bytes.start,
                width: content_width,
                height: self.embeds.height(embed, content_width, ctx),
            });
        }
        boxes
    }

    /// Lays out the lines `lines` of a text that is styled or has embeds: its ranges and its
    /// embeds, those of them that touch the lines and in the lines' own offsets, laid out as
    /// the text of the lines alone, which is what the whole text's layout would put there.
    fn lay_out_rich_window(
        &self,
        ctx: &BuildContext,
        content_width: f32,
        style: &TextStyle,
        index: &LineIndex,
        lines: Range<usize>,
        ranges: Option<&Rc<[aimer_text::StyledRange]>>,
        embeds: &[TextEmbed],
    ) -> RichWindow {
        let bytes = index.line_bytes(lines.start).start..index.line_bytes(lines.end - 1).end;
        let text = &index.text()[bytes.clone()];

        let boxes = self.window_embed_boxes(ctx, content_width, index, &bytes, embeds);

        let local_ranges: Option<Rc<[aimer_text::StyledRange]>> = ranges
            .map(|all| {
                all.iter()
                    .filter(|styled| styled.range.start < bytes.end && bytes.start < styled.range.end)
                    .map(|styled| {
                        aimer_text::StyledRange::new(
                            styled.range.start.max(bytes.start) - bytes.start..styled.range.end.min(bytes.end) - bytes.start,
                            styled.style.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|local| !local.is_empty())
            .map(Rc::from);
        let local_ranges = with_embed_boxes(local_ranges, &boxes, ctx.scale);

        let mut layout_ctx = ctx.clone();
        layout_ctx.parent_size = ResolvedSize { width: content_width, height: ctx.parent_size.height };
        layout_ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: content_width,
            max_height: ctx.box_constraint.max_height,
        };
        let mut widget = self.build_text_widget(text, style, TextAlign::TopLeft);
        widget.ranges = local_ranges.clone();
        let widget = Rc::new(widget);
        // A window is a few kilobytes of text and the one before it was most of the same: laid
        // out in chunks, the part that is not new is found.
        let interaction = aimer_text::with_chunking_min_bytes(0, || widget.interaction_layout(&layout_ctx));
        refine_from_layout(index, lines, &interaction);
        RichWindow { interaction: (*interaction).clone(), ranges: local_ranges, boxes, widget }
    }

    /// The geometry of the lines of the text that are near what is on screen.
    fn windowed_geometry(
        &self,
        ctx: &BuildContext,
        content_width: f32,
        text: Arc<str>,
    ) -> Rc<EditableGeometry> {
        let font_size = self.scaled_font_size(&self.text_style, ctx.scale);
        let style = self.field_text_style();
        let language = self.controller.input_language();
        let (family, font_style, weight) = (style.font_family, style.font_style, style.font_weight.numeric());
        let outer_language = ctx.canvas.text_language();
        ctx.canvas.set_text_language(language);

        let line_height = ctx.canvas.measure_text_metrics_styled("", font_size, 0.0, family, font_style, weight).line_height;
        // What is known of how wide a grapheme is until lines have been laid out.
        let metrics = LineMetrics { content_width, line_height, advance: font_size * 0.55 };
        let index = self.refreshed_line_index(&text, metrics);
        let layout_line = |line: &str| {
            ctx.canvas.layout_text_styled_shared(line, font_size, content_width, family, font_style, weight)
        };

        // Styled text and embeds are laid out as a window of text with its ranges and embeds,
        // and plain text a line at a time.
        let highlighter = self.effective_highlighter();
        let embedder = self.effective_embedder();
        let rich = highlighter.is_some() || embedder.is_some();
        let context = match highlighter {
            Some(highlighter) if highlighter.depends_on_selection() => self.highlight_context(&text, Some(&index)),
            _ if embedder.is_some() => self.highlight_context(&text, Some(&index)),
            _ => HighlightContext::default(),
        };
        let active = context.active_lines();
        let ranges = if rich { self.highlights.resolve_in(highlighter, &text, &context) } else { None };
        let embeds: Rc<[TextEmbed]> = if embedder.is_some() {
            self.current_embeds(&text, &context)
        } else {
            Rc::from(Vec::new())
        };

        // Where the viewport is, kept on the line that is at its top: laying out lines above it
        // corrects what was estimated of them, and that moves everything below. The line
        // is the one the last frame kept the viewport on if the scroll is where that frame
        // left it, and the one at the top now if something scrolled since.
        let multiline_scroll = self.max_lines.is_some();
        let anchor = multiline_scroll.then(|| {
            let scroll = self.scroll_y.get();
            match self.window_anchor.get() {
                Some((line, offset, scrolled)) if scrolled == scroll && line < index.len() => (line, offset),
                _ => {
                    let line = index.line_at_y(f64::from(scroll));
                    (line, (f64::from(scroll) - index.top(line)) as f32)
                }
            }
        });
        let key_for = |lines: &Range<usize>, boxes: &[EmbedBox]| EditableGeometryKey {
            revision: self.controller.revision(),
            font_size_bits: font_size.to_bits(),
            width_bits: content_width.to_bits(),
            obscure: false,
            wrap: true,
            font_family: family,
            font_style,
            font_weight: weight,
            text_transform: style.text_transform,
            letter_spacing_bits: style.letter_spacing.to_bits(),
            word_spacing_bits: style.word_spacing.to_bits(),
            language,
            highlighter: highlighter_id(highlighter),
            active_start: active.as_ref().map_or(usize::MAX, |lines| lines.start),
            active_end: active.as_ref().map_or(usize::MAX, |lines| lines.end),
            // A windowed geometry is placed by the heights of the lines above it.
            embeds: embed_signature(embedder, boxes) ^ index.version().rotate_left(29),
            window: (lines.start, lines.end),
        };
        // The lines to lay out for where the viewport is now, found without laying out anything.
        let select_lines = || {
            // A field that grows is asked for its size before its parent has placed it, and then
            // it does not know what is shown. Its size is the whole note's, which the index
            // answers; the window it has is the one that is on screen, and laying out the top
            // of the note in its place would put that in the cache instead and lay the real
            // window out again in the pass that does know.
            if !multiline_scroll
                && ctx.visible_rect().is_none()
                && let Some((start, end)) = self.window_lines.get()
                && start < end
                && end <= index.len()
            {
                return start..end;
            }
            if let Some((line, offset)) = anchor {
                self.scroll_y.set((index.top(line) + f64::from(offset)) as f32);
            }
            let (top, bottom) = self.visible_text_span(ctx, line_height);
            let mut lines = self.window_lines_for(&index, top, bottom);
            if rich {
                lines = Self::lines_with_whole_embeds(&index, lines, &embeds);
                self.window_lines.set(Some((lines.start, lines.end)));
            }
            lines
        };

        // Asked again with nothing changed, the geometry that was made is the answer.
        let lines = select_lines();
        let boxes = if rich {
            let bytes = index.line_bytes(lines.start).start..index.line_bytes(lines.end - 1).end;
            self.window_embed_boxes(ctx, content_width, &index, &bytes, &embeds)
        } else {
            Vec::new()
        };
        if let Some(geometry) = self.geometry_cache.get(&key_for(&lines, &boxes)) {
            if let Some((line, offset)) = anchor {
                self.window_anchor.set(Some((line, offset, self.scroll_y.get())));
            }
            ctx.canvas.set_text_language(outer_language);
            return geometry;
        }

        // Otherwise the window is laid out, which teaches the index the heights of its lines, and
        // those move the lines below them: so the window is chosen again, for where they are now.
        let mut lines = lines;
        let mut rich_window: Option<RichWindow> = None;
        let mut passes = 0;
        loop {
            let version = index.version();
            if rich {
                // The window is laid out to learn its heights, and that layout is the one used.
                rich_window = Some(self.lay_out_rich_window(
                    ctx,
                    content_width,
                    &style,
                    &index,
                    lines.clone(),
                    ranges.as_ref(),
                    &embeds,
                ));
            } else {
                measure_lines(&index, lines.clone(), &layout_line);
            }
            passes += 1;
            if index.version() == version || passes >= 3 {
                break;
            }
            lines = select_lines();
        }
        if let Some((line, offset)) = anchor {
            // Again, for what the last pass learned.
            self.scroll_y.set((index.top(line) + f64::from(offset)) as f32);
            self.window_anchor.set(Some((line, offset, self.scroll_y.get())));
        }

        let embed_boxes: &[EmbedBox] = rich_window.as_ref().map_or(&[], |window| &window.boxes);
        let key = key_for(&lines, embed_boxes);
        let geometry = match rich_window.take() {
            Some(window) => {
                let widget = Rc::clone(&window.widget);
                EditableGeometry::windowed(
                    Rc::clone(&index),
                    lines.clone(),
                    window.interaction,
                    content_width,
                    self.horizontal_text_align(),
                )
                .with_ranges(window.ranges)
                .with_embeds(window.boxes)
                .with_display_widget(style, TextAlign::TopLeft, widget)
            }
            None => {
                let interaction = lay_out_lines(&index, lines.clone(), &layout_line);
                EditableGeometry::windowed(
                    Rc::clone(&index),
                    lines.clone(),
                    interaction,
                    content_width,
                    self.horizontal_text_align(),
                )
            }
        };
        let geometry = self.geometry_cache.store(key, geometry);
        ctx.canvas.set_text_language(outer_language);
        geometry
    }
}

/// A window of styled text laid out.
struct RichWindow {
    /// The layout of the window's text alone.
    interaction: aimer_cupid::text_layout::TextInteractionLayout,
    /// The styled ranges and the embeds in it, in its own offsets.
    ranges: Option<Rc<[aimer_text::StyledRange]>>,
    boxes: Vec<EmbedBox>,
    /// The widget that laid it out, which paints it.
    widget: Rc<RawTextWidget>,
}
