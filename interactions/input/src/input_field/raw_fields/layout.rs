use aimer_canvas::{Canvas, FrameCanvas, Mat3};
use aimer_cupid::draw_cmd_v2::Rect;
use aimer_cupid::utilities::Color as V2Color;

struct LocalFieldDecoration {
    background: Color,
    box_shadows: Vec<aimer_style::BoxShadow>,
    radii: [f32; 4],
    border_width: [f32; 4],
    border_color: Color,
    outline_width: [f32; 4],
    outline_color: Color,
    content_clip_radii: [f32; 4],
}

struct LocalFieldLayout {
    scale: f32,
    box_width: f32,
    box_height: f32,
    outline_left: f32,
    outline_top: f32,
    padding_left: f32,
    padding_top: f32,
    content_width: f32,
    content_height: f32,
    line_height: f32,
    base_y: f32,
    scroll_x: f32,
    scroll_y: f32,
    geometry: Rc<EditableGeometry>,
    decoration: LocalFieldDecoration,
}

fn local_stroke_widths(
    sides: [aimer_style::BorderSlice; 4],
    physical_widths: [f32; 4],
    scale: f32,
) -> Option<[f32; 4]> {
    let mut widths = [0.0; 4];
    for (index, (side, width)) in sides.into_iter().zip(physical_widths).enumerate() {
        if !width.is_finite() || width < 0.0 {
            return None;
        }
        if width > 0.0 && side.style != BorderStyle::None {
            widths[index] = width / scale;
        }
    }
    Some(widths)
}

fn local_field_decoration(
    decoration: &BoxDecoration,
    box_width: f32,
    box_height: f32,
    scale: f32,
    padding: [f32; 4],
) -> Option<LocalFieldDecoration> {
    if decoration.box_shadow.iter().any(|shadow| {
        !shadow.offset_x.is_finite()
            || !shadow.offset_y.is_finite()
            || !shadow.blur.is_finite()
            || !shadow.spread.is_finite()
    }) {
        return None;
    }
    let radii = decoration.border_radius.resolve(box_width, box_height, scale);
    if radii.iter().any(|radius| !radius.is_finite() || *radius < 0.0) {
        return None;
    }
    let (border_left, border_top, border_right, border_bottom) =
        decoration.border.strokes(box_width, box_height, scale);
    let border_width = local_stroke_widths(
        [
            decoration.border.top,
            decoration.border.right,
            decoration.border.bottom,
            decoration.border.left,
        ],
        [border_top, border_right, border_bottom, border_left],
        scale,
    )?;
    let border_color = decoration.border.effective_color(box_width, box_height, scale);
    let (outline_left, outline_top, outline_right, outline_bottom) =
        decoration.outline.strokes(box_width, box_height, scale);
    let outline_width = local_stroke_widths(
        [
            decoration.outline.top,
            decoration.outline.right,
            decoration.outline.bottom,
            decoration.outline.left,
        ],
        [outline_top, outline_right, outline_bottom, outline_left],
        scale,
    )?;
    let outline_color = decoration.outline.effective_color(box_width, box_height, scale);
    let [pad_left, pad_top, pad_right, pad_bottom] = padding;
    let inner_radius = |radius: f32, x: f32, y: f32| {
        (radius - x.max(y).min(radius)).max(0.0) / scale
    };
    Some(LocalFieldDecoration {
        background: decoration.background_color.get().unwrap_or(Color::Transparent),
        box_shadows: decoration.box_shadow.clone(),
        radii: radii.map(|radius| radius / scale),
        border_width,
        border_color,
        outline_width,
        outline_color,
        content_clip_radii: [
            inner_radius(radii[0], pad_left, pad_top),
            inner_radius(radii[1], pad_right, pad_top),
            inner_radius(radii[2], pad_right, pad_bottom),
            inner_radius(radii[3], pad_left, pad_bottom),
        ],
    })
}

impl LayoutElement for RawTextField {
    /// Returns the bounds cached while the field is laid out or painted.
    ///
    /// Text fields use the same cache for pointer hit testing. Exposing it
    /// through the layout contract is important for enclosing focusable
    /// elements: without these bounds a focus wrapper is treated as covering
    /// the whole window, so the last field in a form wins every pointer press.
    fn pos_start_end(&self) -> Option<(aimer_attribute::Vec2d, aimer_attribute::Vec2d)> {
        self.cached_bounds.pos_start_end()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let (w, h) = self.compute_dimensions(ctx);
        let scale = ctx.scale;
        let (ol, ot, or, ob) = self.outline_strokes(w, h, scale);
        ResolvedSize {
            width: w + ol + or,
            height: h + ot + ob,
        }
    }
}

/// Publishes the same source-aware layout used to paint the field to the
/// shared selection session.
///
/// Ordinary fields can install the immutable interaction layout directly. A
/// secure field or an active preedit changes the displayed source text, so it
/// uses regions derived from the canonical clusters and maps each region back
/// to the controller's byte range.
fn publish_selection_geometry(
    field: &RawTextField,
    canvas: &aimer_canvas::FrameCanvas,
    geometry: &EditableGeometry,
    source_text: &str,
    abs_x: f32,
    abs_y: f32,
    pad_left: f32,
    pad_top: f32,
    scale: f32,
    content_width: f32,
    content_height: f32,
    scroll_x: f32,
    base_y: f32,
    scroll_y: f32,
) {
    let selection = field.selection.borrow().as_ref().cloned();
    let Some(selection) = selection else {
        return;
    };
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let bounds = aimer_attribute::Bounds::new(
        (abs_x + pad_left) / scale,
        (abs_y + pad_top) / scale,
        content_width / scale,
        content_height / scale,
    );
    selection.set_text(Rc::from(source_text));

    let Some(layout) = geometry.interaction.as_ref() else {
        selection.set_geometry(bounds, std::iter::empty());
        return;
    };

    // The transform is captured in the same canvas state used by the text
    // widget below. This keeps selection handles and hit testing correct when
    // a field is nested below a translated or scaled ancestor.
    canvas.save();
    canvas.translate((-scroll_x, base_y - scroll_y).into());
    let transform = canvas.get_transform();
    canvas.restore();

    if geometry.display.as_ref() == source_text && layout.text == source_text {
        selection.set_interaction_geometry(
            bounds,
            Rc::clone(layout),
            transform,
            scale,
        );
        return;
    }

    let mut regions = Vec::new();
    for cluster in &layout.clusters {
        let Some(source_range) = geometry.source_range_for_display_bytes(&cluster.text_range)
        else {
            continue;
        };
        let left = cluster.start_x.min(cluster.end_x);
        let right = cluster.start_x.max(cluster.end_x);
        let is_hard_break = layout
            .text
            .get(cluster.text_range.clone())
            .is_some_and(|text| text == "\n" || text == "\r\n");
        let width = if is_hard_break {
            1.0
        } else {
            (right - left).max(0.0)
        };
        regions.push(aimer_text::SelectionRegion::new(
            source_range,
            transformed_bounds(&transform, left, cluster.y, width, cluster.height, scale),
        ));
    }
    selection.set_geometry(bounds, regions);
}

fn transformed_bounds(
    transform: &aimer_cupid::utilities::Mat3,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    scale: f32,
) -> aimer_attribute::Bounds {
    let points = [
        transform.transform_point(x, y),
        transform.transform_point(x + width, y),
        transform.transform_point(x, y + height),
        transform.transform_point(x + width, y + height),
    ];
    let (min_x, max_x) = points
        .iter()
        .map(|point| point.0)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    let (min_y, max_y) = points
        .iter()
        .map(|point| point.1)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    aimer_attribute::Bounds::new(
        min_x / scale,
        min_y / scale,
        (max_x - min_x) / scale,
        (max_y - min_y) / scale,
    )
}

fn field_text_context<'a>(
    ctx: &BuildContext<'a>,
    width: f32,
    height: f32,
) -> BuildContext<'a> {
    BuildContext {
        parent_size: ResolvedSize { width, height },
        box_constraint: aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: width,
            max_height: height,
        },
        ..ctx.clone()
    }
}

impl RawTextField {
    fn local_v2_layout(&self, ctx: &BuildContext) -> Option<LocalFieldLayout> {
        let measurement_canvas = ctx.canvas.fork_for_recording();
        measurement_canvas.set_text_language(self.controller.input_language());
        let mut measurement_ctx = ctx.clone();
        measurement_ctx.replace_canvas(FrameCanvas::new(&measurement_canvas));
        let ctx = &measurement_ctx;
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }

        let (box_width, box_height) = self.compute_dimensions(ctx);
        if !box_width.is_finite()
            || !box_height.is_finite()
            || box_width < 0.0
            || box_height < 0.0
            || box_width / scale > 1_000_000.0
            || box_height / scale > 1_000_000.0
        {
            return None;
        }

        let decoration = self.active_decoration();
        let (outline_left, outline_top, outline_right, outline_bottom) =
            self.outline_strokes(box_width, box_height, scale);
        if [outline_left, outline_top, outline_right, outline_bottom]
            .iter()
            .any(|stroke| !stroke.is_finite() || *stroke < 0.0)
        {
            return None;
        }
        let padding_left = self.padding.left.value(box_width, scale);
        let padding_top = self.padding.top.value(box_height, scale);
        let padding_right = self.padding.right.value(box_width, scale);
        let padding_bottom = self.padding.bottom.value(box_height, scale);
        let content_width = (box_width - padding_left - padding_right).max(0.0);
        let content_height = (box_height - padding_top - padding_bottom).max(0.0);
        if !content_width.is_finite() || !content_height.is_finite() {
            return None;
        }
        let decoration = local_field_decoration(
            decoration,
            box_width,
            box_height,
            scale,
            [padding_left, padding_top, padding_right, padding_bottom],
        )?;

        let geometry = self.editable_geometry(ctx, content_width);
        let interaction = geometry.interaction.as_ref()?;
        let line_height = interaction.metrics.line_height;
        if !line_height.is_finite() || line_height <= 0.0 {
            return None;
        }
        let multiline = self.wraps_text();
        let line_count = geometry.visual_lines.len().max(1);
        let total_height = line_count as f32 * line_height;
        let base_y = vertical_block_offset(self.text_align, content_height, total_height);
        let (scroll_x, scroll_y) = if multiline {
            let scroll_extent = vertical_scroll_extent(line_count, line_height, content_height);
            let current = self.scroll_y.get();
            if !scroll_extent.is_finite() {
                return None;
            }
            let mut scroll = if current.is_finite() && current >= 0.0 {
                current.min(scroll_extent)
            } else {
                0.0
            };
            if self.reveal_caret.get() {
                scroll = scroll_to_reveal_line(
                    scroll,
                    geometry.line_for_offset(self.cursor.offset()),
                    line_height,
                    content_height,
                    scroll_extent,
                );
            }
            (0.0, scroll)
        } else {
            let max_scroll = (geometry.text_width - content_width).max(0.0);
            let current = self.scroll_x.get();
            let scroll_x = if current.is_finite() {
                current.clamp(0.0, max_scroll)
            } else {
                0.0
            };
            (scroll_x, 0.0)
        };

        let text_ctx = field_text_context(ctx, content_width, content_height);
        if self.controller.text().is_empty() {
            let placeholder = if self.placeholder_visible() {
                if !self.prompt.is_empty() {
                    Some((self.prompt.as_ref(), self.prompt_style))
                } else if !self.hint.is_empty() {
                    Some((self.hint.as_ref(), self.hint_style))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((text, style)) = placeholder {
                let text_widget = self.build_text_widget(text, &style, self.text_align);
                let _ = text_widget.computed_size(&text_ctx);
                if !text_widget.can_paint_local_v2(&text_ctx) {
                    return None;
                }
            }
        } else {
            let style = self.field_text_style();
            let text_widget = self.build_text_widget(
                geometry.display.as_ref(),
                &style,
                self.horizontal_text_align(),
            );
            let _ = text_widget.computed_size(&text_ctx);
            if !text_widget.can_paint_local_v2(&text_ctx) {
                return None;
            }
        }
        if self.is_composing() {
            let mut style = self.field_text_style();
            style.text_overflow = TextOverflow::Clip;
            let supports_preedit = self.with_preedit(|preedit| {
                let (preedit, _) = presentation_preedit(self.input_type, preedit, None);
                let text_widget = self.build_text_widget(preedit.as_ref(), &style, TextAlign::TopLeft);
                let _ = text_widget.computed_size(&text_ctx);
                text_widget.can_paint_local_v2(&text_ctx)
            });
            if !supports_preedit {
                return None;
            }
        }

        Some(LocalFieldLayout {
            scale,
            box_width,
            box_height,
            outline_left,
            outline_top,
            padding_left,
            padding_top,
            content_width,
            content_height,
            line_height,
            base_y,
            scroll_x,
            scroll_y,
            geometry,
            decoration,
        })
    }

    fn retained_v2_paint_state(&self) -> RetainedFieldPaintState {
        RetainedFieldPaintState {
            controller_revision: self.controller.revision(),
            cursor_offset: self.cursor.offset(),
            selection_anchor: self.cursor.selection_anchor(),
            preedit_cursor: self.preedit_cursor.get(),
            scroll_x_bits: self.scroll_x.get().to_bits(),
            scroll_y_bits: self.scroll_y.get().to_bits(),
            input_language: self.controller.input_language(),
            hovered: self.hovered.get(),
            focused: self.is_focused(),
            composing: self.is_composing(),
        }
    }
}

impl Drawable for RawTextField {
    /// Hit-tested over the box inside the outline, sized as it is drawn.
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        let (width, height) = self.compute_dimensions(ctx);
        Some(ResolvedSize { width, height })
    }

    /// The box starts inside the outline strokes.
    fn retained_v2_interaction_offset(&self, ctx: &BuildContext) -> (f32, f32) {
        let (width, height) = self.compute_dimensions(ctx);
        let (left, top, _, _) = self.outline_strokes(width, height, ctx.scale);
        (left, top)
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.cached_bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(&self) -> Option<aimer_widget::InteractionDisagreement> {
        self.cached_bounds.disagreement("TextField")
    }

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        if self.observed_revision.get() != self.controller.revision() {
            self.sync_cursor_from_controller();
            self.sync_preedit_from_controller();
        } else {
            self.sync_cursor_from_area();
        }

        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return false;
        }
        let (box_width, box_height) = self.compute_dimensions(ctx);
        let (outline_left, outline_top, outline_right, outline_bottom) =
            self.outline_strokes(box_width, box_height, scale);
        if [
            box_width,
            box_height,
            outline_left,
            outline_top,
            outline_right,
            outline_bottom,
        ]
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return false;
        }

        let padding_left = self.padding.left.value(box_width, scale);
        let padding_top = self.padding.top.value(box_height, scale);
        let padding_right = self.padding.right.value(box_width, scale);
        let padding_bottom = self.padding.bottom.value(box_height, scale);
        let content_width = (box_width - padding_left - padding_right).max(0.0);
        let content_height = (box_height - padding_top - padding_bottom).max(0.0);
        if [
            padding_left,
            padding_top,
            padding_right,
            padding_bottom,
            content_width,
            content_height,
        ]
        .iter()
        .any(|value| !value.is_finite())
        {
            return false;
        }
        let outer_transform = ctx.canvas.get_transform();
        let (abs_x, abs_y) = outer_transform.transform_point(outline_left, outline_top);
        if !abs_x.is_finite() || !abs_y.is_finite() {
            return false;
        }
        self.cached_bounds
            .save(scale, abs_x, abs_y, box_width, box_height);
        self.set_hovered(
            self.enable
                && self
                    .cached_bounds
                    .is_inside(ctx.cursor_pos.x, ctx.cursor_pos.y),
        );
        self.caret_origin
            .set((outline_left + padding_left, outline_top + padding_top));
        if self.enable && self.is_focused() {
            self.enable_platform_ime();
        }

        let content_ctx = field_text_context(ctx, content_width, content_height);
        let mut geometry = self.editable_geometry(&content_ctx, content_width);
        let mut line_height = geometry
            .interaction
            .as_ref()
            .map(|layout| layout.metrics.line_height)
            .filter(|height| height.is_finite() && *height > 0.0)
            .unwrap_or_else(|| {
                let style = self.field_text_style();
                content_ctx
                    .canvas
                    .measure_text_metrics_styled(
                        "",
                        self.scaled_font_size(&self.text_style, scale),
                        0.0,
                        style.font_family,
                        style.font_style,
                        style.font_weight.numeric(),
                    )
                    .line_height
            });
        if !line_height.is_finite() || line_height <= 0.0 {
            return false;
        }

        let is_multiline = self.max_lines != Some(1);
        let initial_total_height = geometry.visual_lines.len().max(1) as f32 * line_height;
        let initial_base_y = vertical_block_offset(
            self.text_align,
            content_height,
            initial_total_height,
        );
        if !initial_base_y.is_finite() {
            return false;
        }

        if let Some(click_pos) = self.pending_click.take() {
            let click_x = click_pos.x * scale - abs_x - padding_left
                + if is_multiline {
                    0.0
                } else {
                    self.scroll_x.get()
                };
            let click_y = click_pos.y * scale - abs_y - padding_top - initial_base_y
                + if is_multiline {
                    self.scroll_y.get()
                } else {
                    0.0
                };
            let click_offset = geometry
                .hit_test(click_x, click_y)
                .unwrap_or_else(|| geometry.display.graphemes(true).count());
            let click_count = self.click_count.get();
            match click_count {
                2 => self.select_word_at(click_offset),
                3 => {
                    self.select_line_at(click_offset);
                    self.click_count.set(0);
                    self.last_click_pos.set(None);
                }
                _ => {
                    if self.mouse_held.get().is_some() && self.cursor.selection_anchor().is_none() {
                        self.cursor.set_selection_anchor(Some(click_offset));
                    }
                    self.cursor.set_offset(click_offset);
                }
            }
            if let Some(selection) = self.selection.borrow().as_ref().cloned()
                && let Some(pointer) = self.mouse_held.get()
            {
                let text = self.controller.text();
                let (anchor, focus) = self.cursor_selection();
                if click_count == 1 && selection.active_pointer().is_none() {
                    selection.begin(grapheme_byte_offset(&text, focus), pointer);
                } else if click_count != 1 {
                    selection.begin_range(
                        grapheme_byte_offset(&text, anchor),
                        grapheme_byte_offset(&text, focus),
                        pointer,
                    );
                }
            } else if click_count != 1
                && let Some(selection) = self.selection.borrow().as_ref().cloned()
            {
                let text = self.controller.text();
                let (anchor, focus) = self.cursor_selection();
                selection.set_range(
                    grapheme_byte_offset(&text, anchor),
                    grapheme_byte_offset(&text, focus),
                );
            }
            self.sync_selection_to_controller();
            self.reveal_caret.set(true);
            self.cursor.reset_blink();
        }

        self.apply_chosen_action();
        self.raise_pending_menu();
        if self.observed_revision.get() != self.controller.revision() {
            self.sync_cursor_from_controller();
            self.sync_preedit_from_controller();
        }

        geometry = self.editable_geometry(&content_ctx, content_width);
        line_height = geometry
            .interaction
            .as_ref()
            .map(|layout| layout.metrics.line_height)
            .filter(|height| height.is_finite() && *height > 0.0)
            .unwrap_or(line_height);
        if !line_height.is_finite() || line_height <= 0.0 {
            return false;
        }

        let is_empty = self.controller.text().is_empty();
        let line_count = geometry.visual_lines.len().max(1);
        let total_height = line_count as f32 * line_height;
        if !total_height.is_finite() {
            return false;
        }
        let mut base_y = vertical_block_offset(self.text_align, content_height, total_height);
        if is_empty {
            self.scroll_x.set(0.0);
            if is_multiline {
                self.scroll_y.set(0.0);
                self.scroll_y_extent.set(0.0);
                self.reveal_caret.set(false);
            }
        } else if is_multiline {
            let scroll_extent = vertical_scroll_extent(line_count, line_height, content_height);
            let current_scroll = self.scroll_y.get();
            if !scroll_extent.is_finite() || !current_scroll.is_finite() || current_scroll < 0.0 {
                return false;
            }
            self.scroll_y_extent.set(scroll_extent);
            let mut scroll = current_scroll.min(scroll_extent);
            if self.reveal_caret.replace(false) {
                scroll = scroll_to_reveal_line(
                    scroll,
                    geometry.line_for_offset(self.cursor.offset()),
                    line_height,
                    content_height,
                    scroll_extent,
                );
            }
            self.scroll_y.set(scroll);
            base_y = vertical_block_offset(self.text_align, content_height, total_height);
        } else {
            if !self.scroll_x.get().is_finite() || self.scroll_x.get() < 0.0 {
                return false;
            }
            let composition_width = self.with_preedit(|preedit| {
                self.preedit_width(&content_ctx, preedit)
            });
            self.ensure_cursor_visible(content_width, &geometry, composition_width);
        }
        if !self.scroll_x.get().is_finite()
            || !self.scroll_y.get().is_finite()
            || self.scroll_x.get() < 0.0
            || self.scroll_y.get() < 0.0
            || !base_y.is_finite()
        {
            return false;
        }

        let text = self.controller.text();
        let scroll_x = if is_multiline { 0.0 } else { self.scroll_x.get() };
        let scroll_y = if is_multiline { self.scroll_y.get() } else { 0.0 };
        ctx.canvas.save();
        ctx.canvas
            .translate((outline_left, outline_top).into());
        ctx.canvas
            .translate((padding_left, padding_top).into());
        publish_selection_geometry(
            self,
            &ctx.canvas,
            &geometry,
            &text,
            abs_x,
            abs_y,
            padding_left,
            padding_top,
            scale,
            content_width,
            content_height,
            scroll_x,
            base_y,
            scroll_y,
        );
        ctx.canvas.restore();

        let Some(layout) = self.local_v2_layout(ctx) else {
            return false;
        };
        self.scroll_x.set(layout.scroll_x);
        self.scroll_y.set(layout.scroll_y);
        if is_multiline {
            self.scroll_y_extent
                .set(vertical_scroll_extent(line_count, line_height, content_height));
            self.reveal_caret.set(false);
        }

        if self.is_focused() {
            self.cursor.blink().tick(self.now());
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(scope) = self.caret_scope {
                self.cursor.blink().schedule_next_toggle(scope);
            }
            #[cfg(target_arch = "wasm32")]
            aimer_events::window::request_animation_frame();
            #[cfg(not(target_arch = "wasm32"))]
            if self.caret_scope.is_none() {
                aimer_events::window::request_animation_frame();
            }
        }

        let caret_context = self.caret_context();
        caret_context.publish(
            caret_context.geometry(),
            self.cursor.offset(),
            self.is_focused(),
            false,
            self.is_composing(),
        );
        if self.is_focused() {
            let caret = layout
                .geometry
                .caret_geometry(self.cursor.offset())
                .expect("a field interaction layout has an insertion caret");
            let (cursor_x, line_y) = if is_multiline {
                (caret.x, layout.base_y + caret.y - layout.scroll_y)
            } else {
                (
                    caret.x - layout.scroll_x,
                    layout.base_y + caret.y,
                )
            };
            let (cursor_top, cursor_height) = caret_band(line_y, line_height);
            let content_origin = (abs_x + padding_left, abs_y + padding_top);
            self.publish_ime_caret(
                cursor_x,
                cursor_top,
                cursor_height,
                content_origin,
                scale,
            );
            self.publish_caret(cursor_x, cursor_top, 1.5 * scale, cursor_height, scale);
            if self.is_composing() {
                self.with_preedit(|preedit| {
                    self.sync_preedit_caret(
                        preedit,
                        self.preedit_cursor.get(),
                        cursor_x,
                        line_y,
                        line_height,
                        &content_ctx,
                        scale,
                    );
                });
            }
        }
        true
    }

    fn draw_local_v2_compatibility(&self, _ctx: &BuildContext) {}

    fn update(&self, ctx: &BuildContext) {
        if self.observed_revision.get() != self.controller.revision() {
            self.sync_cursor_from_controller();
        }
        self.sync_cursor_from_area();
        if self.is_focused() {
            self.cursor.blink().tick(self.now());
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(scope) = self.caret_scope {
                self.cursor.blink().schedule_next_toggle(scope);
            }
            #[cfg(target_arch = "wasm32")]
            aimer_events::window::request_animation_frame();
            #[cfg(not(target_arch = "wasm32"))]
            if self.caret_scope.is_none() {
                // Isolated/raw fields without an installed Venus runtime retain
                // the frame-driven path used by their direct callers.
                aimer_events::window::request_animation_frame();
            }
        }
        let caret_context = self.caret_context();
        caret_context.publish(
            caret_context.geometry(),
            self.cursor.offset(),
            self.is_focused(),
            false,
            self.is_composing(),
        );
        ctx.canvas.save();
        // Han is unified, so the ideographs a field holds do not say whether
        // they want a Chinese or a Japanese face: `你好` is covered by both and
        // would keep whichever the system prefers until `吗` — written only in
        // Chinese — is typed, at which point the word already on screen
        // changes typeface. The field knows better, because it remembers the
        // keyboard it was typed on, and says so before it draws or measures a
        // single glyph: the caret is placed from the same advances.
        ctx.canvas
            .set_text_language(self.controller.input_language());

        let (box_width, box_height) = self.compute_dimensions(ctx);
        let scale = ctx.scale;

        // Translate inward by outline strokes so the outline has room to draw
        let (ol, ot, _or, _ob) = self.outline_strokes(box_width, box_height, scale);
        ctx.canvas.translate((ol, ot).into());

        // Cache absolute bounds for hit-testing
        let (abs_x, abs_y) = {
            let (tx, ty) = ctx.canvas.get_transform_translation();
            (tx, ty)
        };

        self.cached_bounds
            .save(scale, abs_x, abs_y, box_width, box_height);

        // Hover is derived from the current pointer location rather than
        // carried across a rebuilt field. The event dispatcher clears the
        // previous hit chain when the pointer moves, while this paint-time
        // check also covers a rebuild that happens without another move.
        self.set_hovered(
            self.enable
                && self
                    .cached_bounds
                    .is_inside(ctx.cursor_pos.x, ctx.cursor_pos.y),
        );

        // A field focused at construction time — `auto_focus`, or a rebuild that
        // preserved focus — never passed through `set_focused`, so make sure
        // platform text input is on before the first composition arrives. The
        // call is idempotent, so repeating it every frame is free.
        if self.enable && self.is_focused() {
            self.enable_platform_ime();
        }

        // --- Resolve active decoration ---
        let decoration = self.active_decoration();

        // --- Padding ---
        let pad_top = self.padding.top.value(box_height, scale);
        let pad_bottom = self.padding.bottom.value(box_height, scale);
        let pad_left = self.padding.left.value(box_width, scale);
        let pad_right = self.padding.right.value(box_width, scale);
        self.caret_origin.set((ol + pad_left, ot + pad_top));

        ctx.canvas.save();
        ctx.canvas.translate((pad_left, pad_top).into());

        let content_height = (box_height - pad_top - pad_bottom).max(0.0);
        let content_width = (box_width - pad_left - pad_right).max(0.0);

        let text = self.controller.text();
        let is_empty = text.is_empty();
        let font_size = self.scaled_font_size(&self.text_style, scale);

        let geometry = self.editable_geometry(ctx, content_width);
        let line_height = geometry
            .interaction
            .as_ref()
            .map(|layout| layout.metrics.line_height)
            .filter(|height| height.is_finite() && *height > 0.0)
            .unwrap_or_else(|| {
                let style = self.field_text_style();
                ctx.canvas
                    .measure_text_metrics_styled(
                        "",
                        font_size,
                        0.0,
                        style.font_family,
                        style.font_style,
                        style.font_weight.numeric(),
                    )
                    .line_height
            });
        let total_height = geometry.visual_lines.len() as f32 * line_height;
        let base_y = vertical_block_offset(self.text_align, content_height, total_height);
        let is_multiline = self.max_lines != Some(1);

        // --- Process pending click (deferred from on_event for canvas access) ---
        if let Some(click_pos) = self.pending_click.take() {
            let click_x = click_pos.x * scale - abs_x - pad_left
                + if is_multiline { 0.0 } else { self.scroll_x.get() };
            let click_y = click_pos.y * scale - abs_y - pad_top - base_y
                + if is_multiline {
                    self.scroll_y.get()
                } else {
                    0.0
                };
            let click_offset = geometry
                .hit_test(click_x, click_y)
                .unwrap_or_else(|| geometry.display.graphemes(true).count());

            // Apply double/triple-click selection
            let click_count = self.click_count.get();
            match click_count {
                2 => self.select_word_at(click_offset),
                3 => {
                    self.select_line_at(click_offset);
                    self.click_count.set(0);
                    self.last_click_pos.set(None);
                }
                _ => {
                    // For drag-to-select: set anchor to the click position (not the old cursor)
                    // so the selection extends from the click point to the drag destination.
                    if self.mouse_held.get().is_some() && self.cursor.selection_anchor().is_none() {
                        self.cursor.set_selection_anchor(Some(click_offset));
                    }
                    self.cursor.set_offset(click_offset);
                }
            }
            if let Some(selection) = self.selection.borrow().as_ref().cloned()
                && let Some(pointer) = self.mouse_held.get()
            {
                let text = self.controller.text();
                let (anchor, focus) = self.cursor_selection();
                if click_count == 1 && selection.active_pointer().is_none() {
                    selection.begin(grapheme_byte_offset(&text, focus), pointer);
                } else if click_count != 1 {
                    selection.begin_range(
                        grapheme_byte_offset(&text, anchor),
                        grapheme_byte_offset(&text, focus),
                        pointer,
                    );
                }
            } else if click_count != 1
                && let Some(selection) = self.selection.borrow().as_ref().cloned()
            {
                let text = self.controller.text();
                let (anchor, focus) = self.cursor_selection();
                selection.set_range(
                    grapheme_byte_offset(&text, anchor),
                    grapheme_byte_offset(&text, focus),
                );
            }
            self.sync_selection_to_controller();
            self.reveal_caret.set(true);
            self.cursor.reset_blink();
        }

        // A verb chosen from the open menu is applied here rather than in
        // `on_event`: the menu is a modal, so a tap on one of its rows is
        // handled inside the overlay and never reaches this element.
        self.apply_chosen_action();

        // A menu asked for by the last gesture is raised here, where the click
        // it was asked from has just become a caret offset and a selection —
        // so its verbs describe what the user actually pressed on.
        self.raise_pending_menu();

        // Context with parent_size set to the padded content area
        let mut content_ctx = ctx.clone();
        content_ctx.parent_size = ResolvedSize {
            width: content_width,
            height: content_height,
        };

        // Absolute canvas origin of the content area, used to translate caret
        // positions into the logical window coordinates the IME expects.
        let content_origin = (abs_x + pad_left, abs_y + pad_top);

        if is_empty {
            self.scroll_x.set(0.0);
            if self.max_lines != Some(1) {
                self.scroll_y.set(0.0);
                self.scroll_y_extent.set(0.0);
                self.reveal_caret.set(false);
            }
            publish_selection_geometry(
                self,
                &ctx.canvas,
                &geometry,
                &text,
                abs_x,
                abs_y,
                pad_left,
                pad_top,
                scale,
                content_width,
                content_height,
                0.0,
                base_y,
                0.0,
            );
            // --- Draw prompt (visible when field is empty and not composing) ---
            // --- Draw cursor / composition when field is empty but focused ---
            if self.is_focused() {
                let caret = geometry
                    .caret_geometry(self.cursor.offset())
                    .expect("an empty interaction layout always has a caret");
                let cursor_x = caret.x;
                let line_y = base_y + caret.y;
                let (cursor_top, cursor_height) = caret_band(line_y, line_height);

                self.publish_ime_caret(
                    cursor_x,
                    cursor_top,
                    cursor_height,
                    content_origin,
                    scale,
                );
                self.publish_caret(cursor_x, cursor_top, 1.5 * scale, cursor_height, scale);
                if self.is_composing() {
                    // The insertion point inside a composition is published here;
                    // the composition itself is painted by `paint_local_v2`.
                    self.with_preedit(|preedit| {
                        self.sync_preedit_caret(
                            preedit,
                            self.preedit_cursor.get(),
                            cursor_x,
                            line_y,
                            line_height,
                            &content_ctx,
                            scale,
                        );
                    });
                }

            }
        } else {
            // --- Text (painted by `paint_local_v2`) ---
            let is_multiline = self.max_lines != Some(1);

            if is_multiline {
                // --- Multi-line rendering ---
                let line_count = geometry.visual_lines.len().max(1);
                let total_text_height = line_count as f32 * line_height;
                let scroll_extent = vertical_scroll_extent(
                    line_count,
                    line_height,
                    content_height,
                );
                self.scroll_y_extent.set(scroll_extent);

                let cursor_line = geometry.line_for_offset(self.cursor.offset());
                let mut scroll = self.scroll_y.get().min(scroll_extent);
                if self.reveal_caret.replace(false) {
                    scroll = scroll_to_reveal_line(
                        scroll,
                        cursor_line,
                        line_height,
                        content_height,
                        scroll_extent,
                    );
                }
                self.scroll_y.set(scroll);

                let base_y =
                    vertical_block_offset(self.text_align, content_height, total_text_height);

                publish_selection_geometry(
                    self,
                    &ctx.canvas,
                    &geometry,
                    &text,
                    abs_x,
                    abs_y,
                    pad_left,
                    pad_top,
                    scale,
                    content_width,
                    content_height,
                    0.0,
                    base_y,
                    scroll,
                );

                // Draw cursor / composition from the same canonical caret.
                if self.is_focused() {
                    let caret = geometry
                        .caret_geometry(self.cursor.offset())
                        .expect("a non-empty interaction layout has a caret");
                    let cursor_x = caret.x;
                    let line_y = base_y + caret.y - scroll;
                    let (cursor_top, cursor_height) = caret_band(line_y, line_height);

                    self.publish_ime_caret(
                        cursor_x,
                        cursor_top,
                        cursor_height,
                        content_origin,
                        scale,
                    );
                    self.publish_caret(
                        cursor_x,
                        cursor_top,
                        1.5 * scale,
                        cursor_height,
                        scale,
                    );
                    if self.is_composing() {
                        // The insertion point inside a composition is published here;
                        // the composition itself is painted by `paint_local_v2`.
                        self.with_preedit(|preedit| {
                            self.sync_preedit_caret(
                                preedit,
                                self.preedit_cursor.get(),
                                cursor_x,
                                line_y,
                                line_height,
                                &content_ctx,
                                scale,
                            );
                        });
                    }

                    // The composition replaces the caret: drawing both would
                    // blink an insertion bar over the first composing glyph.
                }
            } else {
                // --- Single-line rendering (with horizontal scroll) ---
                let composition_width = self.with_preedit(|preedit| {
                    self.preedit_width(&content_ctx, preedit)
                });
                self.ensure_cursor_visible(content_width, &geometry, composition_width);
                let scroll = self.scroll_x.get();
                let base_y = vertical_block_offset(self.text_align, content_height, line_height);

                publish_selection_geometry(
                    self,
                    &ctx.canvas,
                    &geometry,
                    &text,
                    abs_x,
                    abs_y,
                    pad_left,
                    pad_top,
                    scale,
                    content_width,
                    content_height,
                    scroll,
                    base_y,
                    0.0,
                );

                if self.is_focused() {
                    let caret = geometry
                        .caret_geometry(self.cursor.offset())
                        .expect("a non-empty interaction layout has a caret");
                    let cursor_x = caret.x - scroll;
                    let line_y = base_y + caret.y;
                    let (cursor_top, cursor_height) = caret_band(line_y, line_height);

                    self.publish_ime_caret(
                        cursor_x,
                        cursor_top,
                        cursor_height,
                        content_origin,
                        scale,
                    );
                    self.publish_caret(cursor_x, cursor_top, 1.5 * scale, cursor_height, scale);
                    if self.is_composing() {
                        // The insertion point inside a composition is published here;
                        // the composition itself is painted by `paint_local_v2`.
                        self.with_preedit(|preedit| {
                            self.sync_preedit_caret(
                                preedit,
                                self.preedit_cursor.get(),
                                cursor_x,
                                line_y,
                                line_height,
                                &content_ctx,
                                scale,
                            );
                        });
                    }

                }
            }
        }

        ctx.canvas.restore(); // translate
        // The language belongs to this field alone, so the widgets drawn after
        // it are judged on their own characters again.
        ctx.canvas.set_text_language(None);
        ctx.canvas.restore(); // outer save

    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        let layout = self.local_v2_layout(ctx)?;
        let full_size = self.computed_size(ctx);
        let scale = layout.scale;
        let left = layout.outline_left / scale;
        let top = layout.outline_top / scale;
        let right_edge = left + layout.box_width / scale;
        let bottom_edge = top + layout.box_height / scale;
        let full_width = full_size.width / scale;
        let full_height = full_size.height / scale;
        let mut outsets = [0.0_f32; 4];
        for shadow in &layout.decoration.box_shadows {
            if shadow.inset || shadow.color == Color::Transparent {
                continue;
            }
            let blur = shadow.blur.max(0.0);
            if blur == 0.0
                && shadow.spread == 0.0
                && shadow.offset_x == 0.0
                && shadow.offset_y == 0.0
            {
                continue;
            }
            let shadow_left = left + shadow.offset_x - blur - shadow.spread;
            let shadow_right = right_edge + shadow.offset_x + blur + shadow.spread;
            let shadow_top = top + shadow.offset_y - blur - shadow.spread;
            let shadow_bottom = bottom_edge + shadow.offset_y + blur + shadow.spread;
            outsets[0] = outsets[0].max((-shadow_left).max(0.0));
            outsets[1] = outsets[1].max((-shadow_top).max(0.0));
            outsets[2] = outsets[2].max((shadow_right - full_width).max(0.0));
            outsets[3] = outsets[3].max((shadow_bottom - full_height).max(0.0));
        }
        outsets.iter().any(|outset| *outset > 0.0).then_some(outsets)
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.local_v2_layout(ctx).is_some()
    }

    fn local_v2_paint_needs_recording(&self, _ctx: &BuildContext) -> bool {
        self.retained_v2_paint_state.get() != Some(self.retained_v2_paint_state())
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let layout = self
            .local_v2_layout(ctx)
            .expect("RawTextField v2 support must be checked before painting");
        let paint_outsets = self.retained_v2_paint_outsets(ctx).unwrap_or([0.0; 4]);
        let canvas = Canvas::of(ctx);
        let translated = paint_outsets[0] != 0.0 || paint_outsets[1] != 0.0;
        if translated {
            canvas.push_transform(Mat3::translate(paint_outsets[0], paint_outsets[1]));
        }
        let measurement_canvas = ctx.canvas.fork_for_recording();
        measurement_canvas.set_text_language(self.controller.input_language());
        let mut measurement_ctx = ctx.clone();
        measurement_ctx.replace_canvas(FrameCanvas::new(&measurement_canvas));
        let ctx = &measurement_ctx;
        let scale = layout.scale;
        let (background_red, background_green, background_blue, background_alpha) =
            layout.decoration.background.to_rgba();
        let (border_red, border_green, border_blue, border_alpha) =
            layout.decoration.border_color.to_rgba();
        let (outline_red, outline_green, outline_blue, outline_alpha) =
            layout.decoration.outline_color.to_rgba();

        for shadow in layout
            .decoration
            .box_shadows
            .iter()
            .filter(|shadow| !shadow.inset)
        {
            let side_params = shadow.side.to_shader_params();
            canvas.draw_shadow_rect(
                Rect::new(
                    layout.outline_left / scale,
                    layout.outline_top / scale,
                    layout.box_width / scale,
                    layout.box_height / scale,
                ),
                shadow.color.into(),
                [
                    shadow.offset_x,
                    shadow.offset_y,
                    shadow.blur.max(0.0),
                    shadow.spread,
                ],
                layout.decoration.radii,
                false,
                [side_params.0, side_params.1, side_params.2],
            );
        }

        canvas.fill_rect_styled(
            Rect::new(
                layout.outline_left / scale,
                layout.outline_top / scale,
                layout.box_width / scale,
                layout.box_height / scale,
            ),
            V2Color::rgba8(
                background_red,
                background_green,
                background_blue,
                background_alpha,
            ),
            layout.decoration.radii,
            layout.decoration.border_width,
            V2Color::rgba8(border_red, border_green, border_blue, border_alpha),
            layout.decoration.outline_width,
            V2Color::rgba8(outline_red, outline_green, outline_blue, outline_alpha),
        );

        for shadow in layout
            .decoration
            .box_shadows
            .iter()
            .filter(|shadow| shadow.inset)
        {
            let side_params = shadow.side.to_shader_params();
            canvas.draw_shadow_rect(
                Rect::new(
                    layout.outline_left / scale,
                    layout.outline_top / scale,
                    layout.box_width / scale,
                    layout.box_height / scale,
                ),
                shadow.color.into(),
                [
                    shadow.offset_x,
                    shadow.offset_y,
                    shadow.blur.max(0.0),
                    shadow.spread,
                ],
                layout.decoration.radii,
                true,
                [side_params.0, side_params.1, side_params.2],
            );
        }

        canvas.push_clip(
            Rect::new(
                (layout.outline_left + layout.padding_left) / scale,
                (layout.outline_top + layout.padding_top) / scale,
                layout.content_width / scale,
                layout.content_height / scale,
            ),
            layout.decoration.content_clip_radii,
        );
        let text_language = self.controller.input_language();
        let has_text_language = text_language.is_some();
        if let Some(language) = text_language {
            canvas.set_text_language(Some(language));
        }

        if !layout.geometry.display.is_empty() {
            let style = self.field_text_style();
            let text_context = field_text_context(ctx, layout.content_width, layout.content_height);
            let text_widget = self.build_text_widget(
                layout.geometry.display.as_ref(),
                &style,
                self.horizontal_text_align(),
            );
            let _ = text_widget.computed_size(&text_context);
            let text_origin = Vec2d {
                x: (layout.outline_left + layout.padding_left - layout.scroll_x) / scale,
                y: (layout.outline_top
                    + layout.padding_top
                    + layout.base_y
                    - layout.scroll_y)
                    / scale,
            };
            let recorded = text_widget.record_local_v2_into(
                &canvas,
                &text_context,
                text_origin,
                false,
                |canvas| {
                    if let Some((start, end)) = self.cursor.selection_range() {
                        let (red, green, blue, alpha) = self.selection_color.to_rgba();
                        for rect in layout.geometry.selection_rects(start, end) {
                            canvas.fill_rect(
                                Rect::new(
                                    text_origin.x + rect.x / scale,
                                    text_origin.y + rect.y / scale,
                                    rect.width / scale,
                                    rect.height / scale,
                                ),
                                [red, green, blue, alpha],
                            );
                        }
                    }
                },
            );
            assert!(recorded, "field text v2 support must be checked before painting");
        } else if self.controller.text().is_empty() && !self.is_composing() {
            let placeholder = if !self.prompt.is_empty() {
                Some((self.prompt.as_ref(), self.prompt_style))
            } else if !self.hint.is_empty() {
                Some((self.hint.as_ref(), self.hint_style))
            } else {
                None
            };
            if let Some((text, style)) = placeholder {
                let text_context = field_text_context(ctx, layout.content_width, layout.content_height);
                let text_widget = self.build_text_widget(text, &style, self.text_align);
                let _ = text_widget.computed_size(&text_context);
                let recorded = text_widget.record_local_v2_into(
                    &canvas,
                    &text_context,
                    Vec2d {
                        x: (layout.outline_left + layout.padding_left) / scale,
                        y: (layout.outline_top + layout.padding_top) / scale,
                    },
                    false,
                    |_| {},
                );
                assert!(recorded, "placeholder v2 support must be checked before painting");
            }
        }

        if self.is_composing() {
            let mut style = self.field_text_style();
            style.text_overflow = TextOverflow::Clip;
            let text_ctx = field_text_context(ctx, layout.content_width, layout.content_height);
            self.with_preedit(|raw_preedit| {
                let (preedit, cursor) = presentation_preedit(
                    self.input_type,
                    raw_preedit,
                    self.preedit_cursor.get(),
                );
                let preedit = preedit.as_ref();
                let caret = layout
                    .geometry
                    .caret_geometry(self.cursor.offset())
                    .expect("a composing field has an insertion caret");
                let origin_x = layout.outline_left + layout.padding_left + caret.x
                    - layout.scroll_x;
                let origin_y = layout.outline_top
                    + layout.padding_top
                    + layout.base_y
                    + caret.y
                    - layout.scroll_y;
                let preedit_layout = self.preedit_layout(&text_ctx, preedit);
                let metrics = &preedit_layout.metrics;
                let preedit_widget =
                    self.build_text_widget(preedit, &style, TextAlign::TopLeft);
                let _ = preedit_widget.computed_size(&text_ctx);
                let recorded = preedit_widget.record_local_v2_into(
                    &canvas,
                    &text_ctx,
                    Vec2d {
                        x: origin_x / scale,
                        y: origin_y / scale,
                    },
                    false,
                    |_| {},
                );
                assert!(recorded, "preedit v2 support must be checked before painting");

                let underline_y = origin_y + layout.line_height * 0.85;
                let caret_color: Color = self.caret_color.into();
                let (red, green, blue, alpha) = caret_color.to_rgba();
                canvas.fill_rect(
                    Rect::new(
                        origin_x / scale,
                        underline_y / scale,
                        metrics.width / scale,
                        1.0,
                    ),
                    [red, green, blue, alpha],
                );

                let Some((start, end)) = cursor else {
                    return;
                };
                let start = floor_char_boundary(preedit, start);
                let end = floor_char_boundary(preedit, end.max(start));
                if end > start {
                    for rect in preedit_layout.selection_rects(start..end) {
                        canvas.fill_rect(
                            Rect::new(
                                (origin_x + rect.x) / scale,
                                (underline_y - scale) / scale,
                                rect.width / scale,
                                2.0,
                            ),
                            [red, green, blue, alpha],
                        );
                    }
                }
            });
        }

        if has_text_language {
            canvas.set_text_language(None);
        }
        canvas.pop_clip();
        if translated {
            canvas.pop_transform();
        }
        canvas.finish();
        self.retained_v2_paint_state
            .set(Some(self.retained_v2_paint_state()));
    }
}

/// The share of a line's height the caret covers.
///
/// A native caret spans its whole line box bar a hairline of leading, so it
/// reaches over the ascender and under the descender of the text it stands
/// in — a caret trimmed much shorter than that reads as a tick next to the
/// glyphs instead of an insertion point between them.
const CARET_LINE_FRACTION: f32 = 0.94;

/// The offset at which a text block of `block_height` is drawn inside a
/// content area of `content_height`.
///
/// Text is laid out as one block of whole lines and then aligned vertically
/// within the content area, so every part of a field that has to meet the
/// text — the caret, the selection highlight, the composition, and click hit
/// testing — starts from this offset. A block taller than the area it is
/// drawn in starts at the top and overflows downwards, which is what the
/// clipped viewport of a scrolling field expects.
#[inline]
fn vertical_block_offset(align: TextAlign, content_height: f32, block_height: f32) -> f32 {
    let spare_height = (content_height - block_height).max(0.0);
    match align {
        TextAlign::TopLeft | TextAlign::TopCenter | TextAlign::TopRight => 0.0,
        TextAlign::MidLeft | TextAlign::MidCenter | TextAlign::MidRight => spare_height / 2.0,
        TextAlign::BotLeft | TextAlign::BotCenter | TextAlign::BotRight => spare_height,
    }
}

/// The `(top, height)` of the caret drawn on a line of `line_height` placed at
/// `line_y`.
///
/// The caret covers nearly all of its line and is centered on it, so the
/// leading it gives up is split evenly above and below the text. Every caret
/// a field paints — the one on the text, the one in an empty field, and the
/// one inside a composition — is measured here, against the line and never
/// against the height of the box the field was given: a field is routinely
/// handed a box several lines tall, and a caret sized from that box spans
/// text it does not stand in.
#[inline]
fn caret_band(line_y: f32, line_height: f32) -> (f32, f32) {
    let height = line_height * CARET_LINE_FRACTION;
    (line_y + (line_height - height) / 2.0, height)
}

#[cfg(test)]
mod caret_layout_tests {
    //! Where the caret is painted inside a field.
    //!
    //! A field draws its text as a stack of lines and its caret as a bar on
    //! the line the cursor sits on, so the caret must follow that line — never
    //! the height of the box the field happens to be given. A single-line
    //! field is routinely handed the whole remaining height of a column, and a
    //! caret spanning that box is nowhere near its text.

    use aimer_style::{TextAlign, TextStyle};
    use aimer_widget::Drawable;

    use super::test_support::{
        dummy_build_context, focused_multiline_field, focused_single_line_field,
    };
    use crate::input_field::controller::TextFieldController;

    /// Asserts that `height` reads as a caret standing on a `line` tall line:
    /// nearly the whole line, and never taller than it.
    #[track_caller]
    fn assert_line_tall(height: f32, line: f32) {
        assert!(
            height >= line * 0.85 && height <= line,
            "caret is {height} tall, expected nearly the whole {line} tall line",
        );
    }

    #[test]
    fn an_empty_multiline_caret_is_one_line_tall() {
        let field = focused_multiline_field(TextFieldController::new(), 3);
        let ctx = dummy_build_context(400.0, 200.0);
        let line = line_height(&field, &ctx);

        field.update(&ctx);

        let (_, height) = caret_band(&field);
        assert_line_tall(height, line);
    }

    #[test]
    fn a_multiline_caret_is_one_line_tall_with_text() {
        let field = focused_multiline_field(TextFieldController::with_initial("hello"), 3);
        let ctx = dummy_build_context(400.0, 200.0);
        let line = line_height(&field, &ctx);

        field.update(&ctx);

        let (_, height) = caret_band(&field);
        assert_line_tall(height, line);
    }

    /// The height of one line of the field's text, as the field measures it.
    fn line_height(field: &super::RawTextField, ctx: &aimer_widget::base::BuildContext) -> f32 {
        let font_size = field.scaled_font_size(&field.text_style, ctx.scale);
        ctx.canvas.measure_text_metrics("", font_size, 0.0).line_height
    }

    /// The vertical band the caret was painted in, as published to the input
    /// method — the same rectangle the field fills on the canvas.
    fn caret_band(field: &super::RawTextField) -> (f32, f32) {
        let caret = field
            .ime_cursor_area
            .get()
            .expect("a focused field publishes the caret it painted");
        (caret.y, caret.height)
    }

    #[test]
    fn a_single_line_caret_is_one_line_tall_in_a_tall_box() {
        let field = focused_single_line_field(TextFieldController::with_initial("hello"));
        let ctx = dummy_build_context(400.0, 600.0);
        let line = line_height(&field, &ctx);

        field.update(&ctx);

        let (_, height) = caret_band(&field);
        assert_line_tall(height, line);
    }

    #[test]
    fn a_single_line_caret_sits_on_its_text_line() {
        let field = focused_single_line_field(TextFieldController::with_initial("hello"));
        let ctx = dummy_build_context(400.0, 600.0);
        let line = line_height(&field, &ctx);

        field.update(&ctx);

        // Text is top aligned by default, so the caret belongs to the first
        // line of the content area rather than to the middle of the box.
        let (top, _) = caret_band(&field);
        assert!(
            top < line,
            "caret starts at {top}, past the first {line}-tall line of text",
        );
    }

    #[test]
    fn an_empty_single_line_caret_sits_on_the_first_line() {
        let field = focused_single_line_field(TextFieldController::new());
        let ctx = dummy_build_context(400.0, 600.0);
        let line = line_height(&field, &ctx);

        field.update(&ctx);

        let (top, height) = caret_band(&field);
        assert!(top < line, "caret of an empty field starts at {top}");
        assert_line_tall(height, line);
    }

    #[test]
    fn a_vertically_centered_single_line_caret_follows_its_text() {
        let mut field = focused_single_line_field(TextFieldController::with_initial("hello"));
        field.text_align = TextAlign::MidLeft;
        let ctx = dummy_build_context(400.0, 600.0);
        let line = line_height(&field, &ctx);

        field.update(&ctx);

        // The text is centered in the content area, and the caret centers with
        // it instead of stretching across the box.
        let (top, height) = caret_band(&field);
        let center = top + height / 2.0;
        assert!(
            (center - 300.0).abs() <= line,
            "caret centered at {center}, expected the middle of a 600 tall box",
        );
        assert_line_tall(height, line);
    }

    #[test]
    fn a_caret_includes_the_text_style_letter_spacing() {
        let mut field = focused_single_line_field(TextFieldController::with_initial("ab"));
        field.text_style = TextStyle::default().letter_spacing(8.0);
        field.cursor.set_offset(1);
        let ctx = dummy_build_context(400.0, 60.0);
        let font_size = field.scaled_font_size(&field.text_style, ctx.scale);
        let expected = 4.0 + ctx.canvas.measure_text("a", font_size) + 8.0;

        field.update(&ctx);

        let caret = field
            .ime_cursor_area
            .get()
            .expect("a focused field publishes its caret");
        assert!(
            (caret.x - expected).abs() <= 0.01,
            "caret is at {}, expected shaped position {}",
            caret.x,
            expected,
        );
    }

    #[test]
    fn a_caret_after_a_trailing_newline_moves_to_the_new_line() {
        let field = focused_multiline_field(TextFieldController::with_initial("hello\n"), 3);
        let ctx = dummy_build_context(400.0, 200.0);
        let line = line_height(&field, &ctx);

        field.update(&ctx);

        let caret = field
            .ime_cursor_area
            .get()
            .expect("a focused field publishes its caret");
        assert!(
            caret.y > 4.0 + line,
            "caret after a trailing newline is at {}, expected the second line",
            caret.y,
        );
    }
}

#[cfg(test)]
mod pointer_selection_tests {
    use aimer_animation::AnimInstant;
    use aimer_attribute::Vec2d;
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_widget::{Drawable, EventElement};

    use super::test_support::{dummy_build_context, focused_single_line_field};
    use crate::TextEditingController;

    #[test]
    fn a_left_click_keeps_the_controller_selection_with_the_canvas_caret() {
        let controller = TextEditingController::with_text("hello");
        let field = focused_single_line_field(controller.clone());
        let ctx = dummy_build_context(400.0, 60.0);

        field.update(&ctx);
        assert!(field
            .on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
                Vec2d { x: 4.0, y: 10.0 },
                PointerButton::Primary,
            )))
            .is_consumed());
        field.update(&ctx);

        assert_eq!(field.cursor.offset(), 0);
        assert_eq!(controller.selection_graphemes(), (0, 0));
    }

    #[test]
    fn rapid_clicks_at_different_positions_do_not_select_a_word() {
        let field = focused_single_line_field(crate::TextEditingController::with_text(
            "hello world",
        ));
        field.test_clock.set(Some(AnimInstant::now()));
        let ctx = dummy_build_context(400.0, 60.0);

        field.update(&ctx);
        for x in [4.0, 50.0] {
            assert!(field
                .on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
                    Vec2d { x, y: 10.0 },
                    PointerButton::Primary,
                )))
                .is_consumed());
            field.update(&ctx);
            let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(
                Vec2d { x, y: 10.0 },
                PointerButton::Primary,
            )));
        }

        assert_eq!(field.cursor.selection_range(), None);
    }

    #[test]
    fn a_context_click_after_a_caret_click_selects_the_word_under_the_pointer() {
        let field = focused_single_line_field(crate::TextEditingController::with_text(
            "hello world",
        ));
        let ctx = dummy_build_context(400.0, 60.0);

        field.update(&ctx);
        let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
            Vec2d { x: 4.0, y: 10.0 },
            PointerButton::Primary,
        )));
        field.update(&ctx);
        let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(
            Vec2d { x: 4.0, y: 10.0 },
            PointerButton::Primary,
        )));

        assert!(field
            .on_event(&ElementEvent::PointerDown(PointerInfo::new(
                Vec2d { x: 50.0, y: 10.0 },
                aimer_events::pointer::PointerSource::Mouse,
                0,
                PointerButton::Secondary,
            )))
            .is_consumed());
        field.update(&ctx);

        assert_eq!(field.cursor.selection_range(), Some((6, 11)));
        assert!(field.menu_is_open());
    }
}
