use std::cell::RefCell;
use std::rc::Rc;

use aimer_attribute::Bounds;
use aimer_attribute::ResolvedSize;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{PointerButton, PointerSource};
use aimer_style::{LineHeight, TextAlign, TextStyle};
use aimer_utils::AnimInstant;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    Drawable, Element, EventElement, EventResult, LayoutElement, PointerKey, VisitorElement,
};

use crate::paragraph::{Paragraph, PreparedLayout, geometry};
use crate::selection::SelectionPoint;
use crate::selection::TextHitRegion;
use crate::selection::cursor::HoverCursor;
use crate::selection::selectable::{Selectable, SelectionBinding, TextGeometry};
use crate::selection::session::{SelectionSession, SelectionSlot};
use crate::selection::touch_hold::{TouchHold, TouchHoldGate, enter_hold, press_touch};
use crate::selection::ui;
use crate::text_span::ResolvedTextSpan;

/// The selectable element produced by [`Text`](crate::Text) inside a
/// [`SelectionArea`](crate::SelectionArea).
///
/// It is the paragraph-backed twin of
/// [`RawTextWidget`](crate::RawTextWidget): same string, same style, same
/// alignment, but laid out through the shared
/// [`Paragraph`](crate::paragraph::Paragraph) so it owns per-grapheme geometry
/// and can take part in a selection. Outside a region `Text` keeps emitting
/// `RawTextWidget`, which pays nothing for selection.
pub struct RawSelectableText {
    paragraph: Paragraph,
    text: crate::TextSource,
    selection_color: Color,
    binding: RefCell<SelectionBinding>,
    hover_cursor: HoverCursor,
    /// Keeps a finger from selecting until it has rested; a mouse never waits.
    pub(crate) touch_hold: TouchHoldGate,
}

struct LocalV2SelectableTextPaint {
    layout: Rc<PreparedLayout>,
    scale: f32,
    width: f32,
    height: f32,
}

impl RawSelectableText {
    /// Builds the element, registering `text` with the ambient session.
    pub(crate) fn new(
        ctx: &BuildContext,
        text: crate::TextSource,
        text_style: TextStyle,
        text_align: TextAlign,
        line_height: LineHeight,
        text_indent: f32,
        fallback_color: Color,
    ) -> Self {
        let binding = SelectionBinding::new(ctx, text.clone(), fallback_color);
        let selection_color = binding.session.selection_color();
        Self {
            paragraph: Paragraph::with_layout(
                vec![ResolvedTextSpan::plain_source(text.clone(), text_style)],
                text_align,
                text_style.text_overflow,
                line_height,
                text_indent,
            ),
            text,
            selection_color,
            binding: RefCell::new(binding),
            hover_cursor: HoverCursor::new(),
            touch_hold: TouchHoldGate::new(),
        }
    }

    #[inline]
    fn geometry(&self) -> Rc<TextGeometry> {
        Rc::clone(&self.binding.borrow().geometry)
    }

    #[inline]
    fn session(&self) -> Rc<SelectionSession> {
        Rc::clone(&self.binding.borrow().session)
    }

    #[inline]
    pub(crate) fn slot(&self) -> Rc<SelectionSlot> {
        Rc::clone(&self.binding.borrow().slot)
    }

    /// Returns accessibility geometry from the last painted frame.
    ///
    /// The snapshot is built from the same source-aware Aimer layout used by
    /// selection and painting. It is `None` before the element has painted or
    /// when the frame supplied an invalid affine transform.
    pub fn accessibility_snapshot(&self) -> Option<crate::TextAccessibilitySnapshot> {
        let binding = self.binding.borrow();
        binding
            .geometry
            .accessibility_snapshot(binding.slot.selected_range())
    }

    fn local_v2_paint_data(&self, ctx: &BuildContext) -> Option<LocalV2SelectableTextPaint> {
        if !self.paragraph.supports_retained_v2_rich_text() {
            return None;
        }
        let scale = ctx.scale;
        let width = self.paragraph.available_width(ctx);
        let height = ctx.parent_size.height;
        if !scale.is_finite()
            || scale <= 0.0
            || !width.is_finite()
            || !height.is_finite()
            || width < 0.0
            || height < 0.0
            || width > 1_000_000.0
            || height > 1_000_000.0
        {
            return None;
        }
        let layout = self.paragraph.layout_for_paint(ctx);
        if !layout.size.width.is_finite()
            || !layout.size.height.is_finite()
            || layout.size.width < 0.0
            || layout.size.height < 0.0
            || layout.size.width > 1_000_000.0
            || layout.size.height > 1_000_000.0
            || layout.line_heights.iter().any(|height| !height.is_finite())
            || layout.fragments.iter().any(|fragment| {
                !fragment.x.is_finite()
                    || !fragment.baseline.is_finite()
                    || !fragment.width.is_finite()
                    || !fragment.height.is_finite()
                    || fragment.width < 0.0
                    || fragment.height < 0.0
            })
        {
            return None;
        }
        Some(LocalV2SelectableTextPaint {
            layout,
            scale,
            width,
            height,
        })
    }
}

impl VisitorElement for RawSelectableText {
    fn debug_name(&self) -> &'static str {
        "SelectableText"
    }
}

impl aimer_widget::Rebuildable for RawSelectableText {
    #[inline]
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Keeps a selection covering this text alive across a rebuild.
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|value| value.downcast_ref::<Self>())
        else {
            return;
        };
        let adopted = old.binding.borrow().adopt(self.text.clone());
        *self.binding.borrow_mut() = adopted;
    }
}

impl LayoutElement for RawSelectableText {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.paragraph.prepare_for_paint(ctx).size
    }

    fn invalidate_layout(&self) {
        self.paragraph.invalidate();
    }

    fn pos_start_end(&self) -> Option<(aimer_attribute::Vec2d, aimer_attribute::Vec2d)> {
        self.geometry().pos_start_end()
    }
}

impl Drawable for RawSelectableText {
    /// The paragraph's interaction rectangle is its laid-out box. The render
    /// tree supplies where that box is now, so geometry painted earlier follows
    /// a scroll.
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.paragraph.prepare_for_paint(ctx).size)
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.geometry().bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.geometry().bounds.disagreement("RawSelectableText")
    }

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        self.slot().stamp();
        self.sync_paint_geometry(ctx);
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let origin = self.geometry().origin(abs_x, abs_y, ctx.scale);
        if let Some((pointer, offset)) = self.touch_hold.poll_stationary(AnimInstant::now(), origin)
        {
            enter_hold(&self.session(), &self.slot(), offset, pointer);
        }
        true
    }

    fn draw_local_v2_compatibility(&self, _ctx: &BuildContext) {}

    fn update(&self, ctx: &BuildContext) {
        // The text is painted by `paint_local_v2`; this keeps the geometry the
        // interaction reads current and polls a resting touch.
        self.sync_local_v2_state(ctx);
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        let data = self.local_v2_paint_data(ctx)?;
        let outsets = self
            .paragraph
            .retained_v2_paint_outsets(&data.layout, data.scale);
        outsets
            .iter()
            .any(|outset| *outset > 0.0)
            .then_some(outsets)
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.local_v2_paint_data(ctx).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let data = self
            .local_v2_paint_data(ctx)
            .expect("RawSelectableText v2 support must be checked before painting");
        let selected_range = self.slot().selected_range().unwrap_or(0..0);
        let outsets = self
            .paragraph
            .retained_v2_paint_outsets(&data.layout, data.scale);
        let origin_offset = aimer_attribute::Vec2d {
            x: outsets[0],
            y: outsets[1],
        };
        let canvas = aimer_canvas::Canvas::of(ctx);
        if self.paragraph.needs_clip() {
            canvas.push_clip(
                aimer_cupid::utilities::Rect::new(
                    origin_offset.x,
                    origin_offset.y,
                    data.width / data.scale,
                    data.height / data.scale,
                ),
                [0.0; 4],
            );
        }
        self.paragraph.record_retained_v2_backgrounds(
            &canvas,
            &data.layout,
            data.scale,
            origin_offset,
        );
        let (red, green, blue, alpha) = self.selection_color.to_rgba();
        if let Some(layout) = data.layout.aimer_interaction.as_ref() {
            for rect in layout.selection_rects(selected_range.clone()) {
                canvas.fill_rect(
                    aimer_cupid::utilities::Rect::new(
                        rect.x / data.scale + origin_offset.x,
                        rect.y / data.scale + origin_offset.y,
                        rect.width / data.scale,
                        rect.height / data.scale,
                    ),
                    [red, green, blue, alpha],
                );
            }
        } else {
            for rect in
                geometry::selection_runs(&data.layout, selected_range.clone(), ctx.visible_rect)
            {
                canvas.fill_rect(
                    aimer_cupid::utilities::Rect::new(
                        rect.x / data.scale + origin_offset.x,
                        rect.y / data.scale + origin_offset.y,
                        rect.width / data.scale,
                        rect.height / data.scale,
                    ),
                    [red, green, blue, alpha],
                );
            }
        }
        self.paragraph.record_retained_v2_foreground(
            &canvas,
            &data.layout,
            data.scale,
            origin_offset,
            None,
            None,
        );
        if self.paragraph.needs_clip() {
            canvas.pop_clip();
        }
        canvas.finish();
        *self.geometry().retained_v2_selection.borrow_mut() = Some(selected_range);
    }

    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        if self.local_v2_paint_data(ctx).is_none() {
            return false;
        }
        let selection = Some(self.slot().selected_range().unwrap_or(0..0));
        selection != *self.geometry().retained_v2_selection.borrow()
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let Some(data) = self.local_v2_paint_data(ctx) else {
            return;
        };
        let transform = ctx.canvas.get_transform();
        let geometry_state = self.geometry();
        geometry_state.save_painted_bounds(
            data.scale,
            transform,
            data.layout.size.width,
            data.layout.size.height,
        );
        geometry_state.regions.borrow_mut().clear();
        geometry_state.set_shared_interaction_layout(
            data.layout.aimer_interaction.clone(),
            transform,
            data.scale,
        );
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        if let Some(shared) = data.layout.aimer_interaction.as_ref() {
            let mut regions = geometry_state.regions.borrow_mut();
            for cluster in &shared.clusters {
                let left = cluster.start_x.min(cluster.end_x);
                let right = cluster.start_x.max(cluster.end_x);
                let is_hard_break = shared
                    .text
                    .get(cluster.text_range.clone())
                    .is_some_and(|text| text == "\n" || text == "\r\n");
                regions.push(TextHitRegion::new(
                    if is_hard_break {
                        cluster.text_range.start..cluster.text_range.start
                    } else {
                        cluster.text_range.clone()
                    },
                    Bounds::new(
                        (abs_x + left) / data.scale,
                        (abs_y + cluster.y) / data.scale,
                        if is_hard_break {
                            (shared.metrics.width - left).max(data.scale) / data.scale
                        } else {
                            (right - left) / data.scale
                        },
                        cluster.height / data.scale,
                    ),
                ));
            }
        } else {
            geometry::hit_regions(
                &data.layout,
                abs_x,
                abs_y,
                data.scale,
                ctx.visible_rect,
                &mut geometry_state.regions.borrow_mut(),
            );
        }
    }
}

impl EventElement for RawSelectableText {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        // The selection's knobs and callout are painted over this text, so a
        // press that grabbed one must never reach the paragraph underneath.
        if let Some(result) = ui::intercept(&self.session(), event) {
            return result;
        }

        let geometry = self.geometry();
        // A press is a finger resting on a glyph, so one whose paragraph has
        // since slid is no longer a hold and must not be judged as one below.
        self.touch_hold
            .forget_if_content_moved(geometry.painted_origin());
        let over_glyphs = event
            .pointer()
            .is_some_and(|info| geometry.hits_glyph(info.x(), info.y()));
        let cursor_claimed =
            self.hover_cursor
                .apply(geometry.window(), event, true, false, over_glyphs);

        match event {
            ElementEvent::PointerDown(info) => {
                let pos = info.pos;
                let geometry = self.geometry();
                let pointer = PointerKey::new(info.source, info.id);
                if info.button == PointerButton::Secondary {
                    return ui::open_context_menu(
                        &self.session(),
                        &self.slot(),
                        &geometry,
                        pos,
                        pointer,
                    )
                    .into();
                }
                // The offset lookup snaps to the nearest glyph, so a press that
                // never touched this text — the tree broadcasts the presses
                // nobody took — is told apart by the painted bounds first, and
                // dismisses the selection instead of starting a new one.
                let inside = geometry.contains_point(pos.x, pos.y);
                let offset = inside.then(|| geometry.offset_at(pos.x, pos.y)).flatten();
                let Some(offset) = offset else {
                    self.touch_hold.clear();
                    let session = self.session();
                    if session.active_pointer() != Some(pointer) {
                        session.clear();
                    }
                    return false.into();
                };
                if info.source == PointerSource::Touch {
                    // A finger means a scroll as often as a selection, so the
                    // press is only remembered until the hold has been earned —
                    // together with where this paragraph sat when it landed, the
                    // one evidence a later frame has that the page has moved on.
                    return press_touch(
                        &self.session(),
                        &self.touch_hold,
                        pointer,
                        offset,
                        pos,
                        geometry.painted_origin(),
                    );
                }
                self.session()
                    .begin(SelectionPoint::new(self.slot(), offset), pointer);
                EventResult::consumed().with_pointer_capture(pointer)
            }
            ElementEvent::PointerMove(info) => {
                let pointer = PointerKey::new(info.source, info.id);
                match self.touch_hold.poll(pointer, info.pos, AnimInstant::now()) {
                    TouchHold::Entered(offset) => {
                        enter_hold(&self.session(), &self.slot(), offset, pointer);
                        return EventResult::consumed().with_pointer_capture(pointer);
                    }
                    // Still resting, or gone off scrolling: either way the
                    // gesture is not this element's yet.
                    TouchHold::Waiting | TouchHold::Abandoned => return false.into(),
                    TouchHold::Idle => {}
                }
                let session = self.session();
                if session.active_pointer() != Some(pointer) {
                    return cursor_claimed.into();
                }
                session.extend_to_position(info.x(), info.y(), pointer);
                EventResult::consumed()
            }
            ElementEvent::PointerUp(info) => {
                let pointer = PointerKey::new(info.source, info.id);
                if self.touch_hold.release_was_stationary(pointer, info.pos) {
                    let session = self.session();
                    session.end(pointer);
                    ui::offer_menu_after_gesture(&session, info.source);
                    return EventResult::consumed();
                }
                if let TouchHold::Entered(offset) =
                    self.touch_hold.poll(pointer, info.pos, AnimInstant::now())
                {
                    // Held still and let go: the word under the finger stays
                    // selected.
                    let session = self.session();
                    enter_hold(&session, &self.slot(), offset, pointer);
                    session.end(pointer);
                    ui::offer_menu_after_gesture(&session, info.source);
                    return EventResult::consumed();
                }
                self.touch_hold.clear();
                let session = self.session();
                if session.active_pointer() != Some(pointer) {
                    return false.into();
                }
                session.extend_to_position(info.x(), info.y(), pointer);
                session.end(pointer);
                ui::offer_menu_after_gesture(&session, info.source);
                EventResult::consumed().with_pointer_release(pointer)
            }
            ElementEvent::Cancel => {
                self.touch_hold.clear();
                self.session().cancel();
                false.into()
            }
            _ => cursor_claimed.into(),
        }
    }
}

#[cfg(test)]
impl RawSelectableText {
    /// Builds an element registered with `session` that behaves as if it had
    /// already painted `regions` inside `bounds`.
    pub(crate) fn painted(
        session: &Rc<SelectionSession>,
        window: &aimer_widget::base::WindowHandle,
        text: Rc<str>,
        regions: Vec<crate::selection::TextHitRegion>,
        bounds: aimer_attribute::Bounds,
    ) -> Self {
        let text = crate::TextSource::from(text);
        let geometry = Rc::new(TextGeometry::new(window.clone()));
        let slot = session.register(text.clone(), Rc::downgrade(&geometry) as _);
        slot.stamp();
        *geometry.regions.borrow_mut() = regions;
        geometry
            .bounds
            .save(1.0, bounds.x, bounds.y, bounds.width, bounds.height);
        Self {
            paragraph: Paragraph::new(
                vec![ResolvedTextSpan::plain_source(
                    text.clone(),
                    TextStyle::default(),
                )],
                TextAlign::TopLeft,
                aimer_style::TextOverflow::Clip,
            ),
            text,
            selection_color: session.selection_color(),
            binding: RefCell::new(SelectionBinding {
                geometry,
                session: Rc::clone(session),
                slot,
                owns_session: false,
            }),
            hover_cursor: HoverCursor::new(),
            touch_hold: TouchHoldGate::new(),
        }
    }
}
