use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd_v2::Rect;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_widget::base::BuildContext;
use aimer_widget::{Drawable, Element, LayoutElement};
use aimer_widget::{
    request_mouse_region_hover_reconciliation, with_deferred_mouse_region_hover_reconciliation,
};

use crate::ScrollAxis;
use crate::raw_scroll::{DragMode, RawScrollableContainer};
use crate::scrollable::cache_extent::cache_rect;
use crate::scrollable::constants::SNAP_EPSILON;
use crate::scrollable::recovery_end::finish_overscroll_recovery;

fn snap_scroll_offset(offset: Vec2d) -> Vec2d {
    Vec2d {
        x: offset.x.round(),
        y: offset.y.round(),
    }
}

/// How far the visible content moved since the previous drawn frame, in the
/// child's coordinates.
///
/// The content is translated by the negated scroll offset, so the visible
/// rectangle travels opposite to it. `None` — the first frame — is no travel:
/// there is nothing yet to measure against, and a viewport that has never
/// moved has no direction to lead toward.
fn visible_travel(previous: Option<Vec2d>, offset: Vec2d) -> Vec2d {
    match previous {
        Some(previous) => Vec2d {
            x: previous.x - offset.x,
            y: previous.y - offset.y,
        },
        None => Vec2d::ZERO,
    }
}

fn intersect_rect(left: Rect, right: Rect) -> Rect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    Rect::new(x, y, (right_edge - x).max(0.0), (bottom_edge - y).max(0.0))
}

impl<E: Element> Drawable for RawScrollableContainer<E> {
    /// Hit-tested over the scroll viewport as laid out.
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.layout_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("Scrollable")
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        let size = self.layout_size(ctx);
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && size.width.is_finite()
            && size.height.is_finite()
            && size.width >= 0.0
            && size.height >= 0.0
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        Canvas::of(ctx).finish();
    }

    fn retained_v2_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.layout_size(ctx))
    }

    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        let (viewport_w, viewport_h) = self.viewport_size(ctx);
        let viewport_w = viewport_w.min(1e7_f32);
        let viewport_h = viewport_h.min(1e7_f32);

        if child.id() == self.child.id() {
            let mut child_ctx = ctx.clone();
            child_ctx.box_constraint.min_width = child_ctx.box_constraint.min_width.min(viewport_w);
            child_ctx.box_constraint.min_height =
                child_ctx.box_constraint.min_height.min(viewport_h);
            child_ctx.box_constraint.max_width = viewport_w;
            child_ctx.box_constraint.max_height = viewport_h;
            child_ctx.parent_size = ResolvedSize {
                width: viewport_w,
                height: viewport_h,
            };
            match self.ctrl.axis {
                ScrollAxis::Vertical => child_ctx.box_constraint.max_height = f32::MAX,
                ScrollAxis::Horizontal => child_ctx.box_constraint.max_width = f32::MAX,
            }
            let offset = snap_scroll_offset(self.ctrl.visual_offset(self.ctrl.scroll_offset.get()));
            child_ctx.parent_pos.x += offset.x;
            child_ctx.parent_pos.y += offset.y;
            child_ctx.visible_rect = Some(cache_rect(
                self.ctrl.axis,
                Vec2d {
                    x: -offset.x,
                    y: -offset.y,
                },
                (viewport_w, viewport_h),
                Vec2d::ZERO,
            ));
            return Some(child_ctx);
        }

        let mut bar_ctx = ctx.clone();
        if self
            .vertical_scroll_bar
            .as_ref()
            .is_some_and(|bar| bar.id() == child.id())
        {
            bar_ctx.box_constraint.max_width = self.vertical_bar_width;
            bar_ctx.box_constraint.max_height = viewport_h;
            bar_ctx.parent_size = ResolvedSize {
                width: self.vertical_bar_width,
                height: viewport_h,
            };
            bar_ctx.parent_pos.x += self
                .ctrl
                .scroll_bar_placement
                .cross_offset(viewport_w, self.vertical_bar_width);
            return Some(bar_ctx);
        }
        if self
            .horizontal_scroll_bar
            .as_ref()
            .is_some_and(|bar| bar.id() == child.id())
        {
            bar_ctx.box_constraint.max_width = viewport_w;
            bar_ctx.box_constraint.max_height = self.horizontal_bar_height;
            bar_ctx.parent_size = ResolvedSize {
                width: viewport_w,
                height: self.horizontal_bar_height,
            };
            bar_ctx.parent_pos.y += self
                .ctrl
                .scroll_bar_placement
                .cross_offset(viewport_h, self.horizontal_bar_height);
            return Some(bar_ctx);
        }
        None
    }

    fn retained_v2_child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
    ) -> Option<(Rect, Option<Rect>)> {
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let (viewport_w, viewport_h) = self.viewport_size(ctx);
        let viewport_w = viewport_w.min(1e7_f32);
        let viewport_h = viewport_h.min(1e7_f32);
        if !viewport_w.is_finite() || !viewport_h.is_finite() {
            return None;
        }

        if child.id() == self.child.id() {
            let child_ctx = self.retained_v2_child_context(ctx, child)?;
            let size = child.content_size(&child_ctx);
            let position = child.pos().unwrap_or_default();
            let offset = snap_scroll_offset(self.ctrl.visual_offset(self.ctrl.scroll_offset.get()));
            let x = position.x + offset.x;
            let y = position.y + offset.y;
            let bounds = Rect::new(
                x / scale,
                y / scale,
                size.width / scale,
                size.height / scale,
            );
            let viewport_clip = Rect::new(
                -x / scale,
                -y / scale,
                viewport_w / scale,
                viewport_h / scale,
            );
            let clip = child
                .retained_clip(&child_ctx)
                .map(|child_clip| intersect_rect(viewport_clip, child_clip))
                .unwrap_or(viewport_clip);
            return Some((bounds, Some(clip)));
        }

        if self
            .vertical_scroll_bar
            .as_ref()
            .is_some_and(|bar| bar.id() == child.id())
        {
            let bar_ctx = self.retained_v2_child_context(ctx, child)?;
            let size = child.content_size(&bar_ctx);
            let bounds = if matches!(self.ctrl.axis, ScrollAxis::Vertical) {
                Rect::new(
                    self.ctrl
                        .scroll_bar_placement
                        .cross_offset(viewport_w, self.vertical_bar_width)
                        / scale,
                    0.0,
                    size.width / scale,
                    size.height / scale,
                )
            } else {
                Rect::new(0.0, 0.0, 0.0, 0.0)
            };
            return Some((bounds, child.retained_clip(&bar_ctx)));
        }

        if self
            .horizontal_scroll_bar
            .as_ref()
            .is_some_and(|bar| bar.id() == child.id())
        {
            let bar_ctx = self.retained_v2_child_context(ctx, child)?;
            let size = child.content_size(&bar_ctx);
            let bounds = if matches!(self.ctrl.axis, ScrollAxis::Horizontal) {
                Rect::new(
                    0.0,
                    self.ctrl
                        .scroll_bar_placement
                        .cross_offset(viewport_h, self.horizontal_bar_height)
                        / scale,
                    size.width / scale,
                    size.height / scale,
                )
            } else {
                Rect::new(0.0, 0.0, 0.0, 0.0)
            };
            return Some((bounds, child.retained_clip(&bar_ctx)));
        }
        None
    }

    fn update(&self, ctx: &BuildContext) {
        // println!("Scrollable drawing child: {})", self.child.debug_name() );

        // The thumb highlight is resolved from the cursor below, in viewport terms.
        self.sync_viewport_origin();

        let scrolling_before_draw = self.ctrl.is_scrolling.get();

        let (raw_viewport_w, raw_viewport_h) = self.viewport_size(ctx);
        // debug!("View port size: {:?} x {:?}", raw_viewport_w, raw_viewport_h);
        // Cap viewport size to avoid precision issues with f32::MAX in
        // shaders/transforms
        let max_dim = 1e7_f32;
        let viewport_w = raw_viewport_w.min(max_dim);
        let viewport_h = raw_viewport_h.min(max_dim);
        let previous_content_size = self.ctrl.cached_content_size.get();
        let previous_max_scroll = self.ctrl.cached_max_scroll.get();
        let previous_offset = self.ctrl.scroll_offset.get();
        let preserve_content_end = self.ctrl.cached_content_size_valid.get()
            && match self.ctrl.axis {
                ScrollAxis::Vertical => {
                    previous_max_scroll.y > SNAP_EPSILON
                        && (previous_offset.y + previous_max_scroll.y).abs() <= SNAP_EPSILON
                }
                ScrollAxis::Horizontal => {
                    previous_max_scroll.x > SNAP_EPSILON
                        && (previous_offset.x + previous_max_scroll.x).abs() <= SNAP_EPSILON
                }
            };
        // Materialize and refine the visible window before physics can clamp
        // against its estimate and before any commands enter retained paint.
        let mut prepare_ctx = ctx.clone();
        prepare_ctx.box_constraint.min_width = prepare_ctx.box_constraint.min_width.min(viewport_w);
        prepare_ctx.box_constraint.min_height =
            prepare_ctx.box_constraint.min_height.min(viewport_h);
        prepare_ctx.box_constraint.max_width = viewport_w;
        prepare_ctx.box_constraint.max_height = viewport_h;
        prepare_ctx.parent_size = ResolvedSize {
            width: viewport_w,
            height: viewport_h,
        };
        match self.ctrl.axis {
            ScrollAxis::Vertical => prepare_ctx.box_constraint.max_height = f32::MAX,
            ScrollAxis::Horizontal => prepare_ctx.box_constraint.max_width = f32::MAX,
        }
        let initial_offset = self.ctrl.scroll_offset.get();
        prepare_ctx.visible_rect = Some(cache_rect(
            self.ctrl.axis,
            Vec2d {
                x: -initial_offset.x,
                y: -initial_offset.y,
            },
            (viewport_w, viewport_h),
            Vec2d::ZERO,
        ));
        let mut prepared_extent = self.child.prepare_layout(&prepare_ctx);
        let initial_content_size = self.content_size(ctx);
        let mut content_size = initial_content_size;
        // Refining the end anchor can expose another window. Bound the work
        // just as the windowed flex layout bounds its own reconciliation.
        for _ in 0..3 {
            let offset = self.ctrl.scroll_offset.get();
            prepare_ctx.visible_rect = Some(cache_rect(
                self.ctrl.axis,
                Vec2d {
                    x: -offset.x,
                    y: -offset.y,
                },
                (viewport_w, viewport_h),
                Vec2d::ZERO,
            ));
            prepared_extent |= self.child.prepare_layout(&prepare_ctx);
            let refined = self.child.computed_size(&prepare_ctx);
            if refined == content_size {
                break;
            }
            self.refresh_content_size_after_draw(
                ctx,
                viewport_w,
                viewport_h,
                content_size,
                true,
                preserve_content_end,
            );
            content_size = refined;
        }
        if content_size != initial_content_size
            && self.ctrl.drag_mode.get() == DragMode::None
            && !scrolling_before_draw
        {
            // A layout shrink is not a user overscroll gesture. Settle an idle
            // viewport immediately instead of painting a spring-back frame.
            let offset = self.ctrl.clamp_offset(self.ctrl.scroll_offset.get());
            self.ctrl.set_scroll_offset(offset);
        }
        // A newly built scroll element can briefly report a predicted extent
        // before its windowed child paints and refines that prediction. Keep the
        // last known range for physics during that frame so a preserved offset
        // is not clamped toward the prediction before the child can correct it.
        let provisional_extent = !prepared_extent
            && self.ctrl.cached_content_size_valid.get()
            && self.ctrl.last_scale.get() == ctx.scale
            && previous_content_size != content_size
            && initial_content_size == content_size;
        let range_content_size = if provisional_extent {
            previous_content_size
        } else {
            self.ctrl.cached_content_size.set(content_size);
            self.ctrl.cached_content_size_valid.set(true);
            content_size
        };
        let transform = ctx.canvas.get_transform_translation();
        let layout_size = self.layout_size(ctx);
        let max_x = (range_content_size.width - viewport_w).max(0.0);
        let max_y = (range_content_size.height - viewport_h).max(0.0);

        self.bounds.save(
            ctx.scale,
            transform.0,
            transform.1,
            layout_size.width,
            layout_size.height,
        );
        self.ctrl.cached_viewport.set((viewport_w, viewport_h));
        self.ctrl.cursor_pos.set(Some(ctx.cursor_pos));

        let mut final_max = Vec2d { x: max_x, y: max_y };
        let user_max = self.ctrl.scroll_behavior.max_scroll;
        if user_max.x != f32::MAX {
            final_max.x = final_max.x.max(user_max.x * ctx.scale);
        }
        if user_max.y != f32::MAX {
            final_max.y = final_max.y.max(user_max.y * ctx.scale);
        }

        self.ctrl.cached_max_scroll.set(final_max);

        if prepared_extent && preserve_content_end {
            let mut offset = self.ctrl.scroll_offset.get();
            match self.ctrl.axis {
                ScrollAxis::Vertical => offset.y = -final_max.y,
                ScrollAxis::Horizontal => offset.x = -final_max.x,
            }
            self.ctrl.set_scroll_offset(offset);
        }

        if prepared_extent
            && content_size != previous_content_size
            && self.ctrl.drag_mode.get() == DragMode::None
            && !scrolling_before_draw
        {
            let offset = self.ctrl.clamp_offset(self.ctrl.scroll_offset.get());
            self.ctrl.set_scroll_offset(offset);
        }

        let user_min = self.ctrl.scroll_behavior.min_scroll;
        self.ctrl.cached_min_scroll.set(Vec2d {
            x: user_min.x * ctx.scale,
            y: user_min.y * ctx.scale,
        });

        self.ctrl.last_scale.set(ctx.scale);

        let mut offset = self.ctrl.scroll_offset.get();

        if self.ctrl.drag_mode.get() == DragMode::None {
            // let vel = self.ctrl.pointer_velocity.get();
            // let vel_mag = (vel.x * vel.x + vel.y * vel.y).sqrt();
            // if vel_mag > VELOCITY_EPSILON {
            //     info!("[scroll] DRAW momentum vel_mag={:.2} offset=({:.1},{:.1})",
            // vel_mag, offset.x, offset.y); }
            let (new_offset, needs_redraw) = self.ctrl.update_momentum(offset);
            offset = new_offset;

            // A platform that cannot report a finger lift keeps its gesture
            // open through the browser's own momentum tail. Landing the
            // recovered edge is the last moment that gesture is still about
            // what the user did, so it is terminated here and the distance it
            // still owes is dropped — otherwise the queued tail stretches the
            // edge a second time. A no-op everywhere the platform reports the
            // lift itself.
            finish_overscroll_recovery(&self.ctrl, offset);

            if needs_redraw {
                self.ctrl.request_animation_frame();
            } else {
                // Not dragging and momentum/fling/spring-back have fully settled:
                // this is where a scroll session ends. `end_scroll` is edge-
                // triggered, so it fires the callback only once (on the actual
                // scrolling → idle transition) and is a no-op on later idle frames.
                self.ctrl.end_scroll();
            }
        }

        self.ctrl.set_scroll_offset(offset);

        // Level-triggered per-frame notification: fires `on_scroll` only when the
        // logical offset actually moved since the last frame (epsilon-guarded), so
        // it covers drags, wheel/keyboard, momentum, spring-back and programmatic
        // scrolls without emitting on idle frames.
        let moved = self.ctrl.notify_scroll();
        self.ctrl.record_draw_frame(scrolling_before_draw, moved);

        // Write-back: persist the live position so a full teardown/re-create can
        // restore it (see `scroll_storage`). Stored in logical (unscaled) pixels
        // to survive a scale change. Only when the user opted in via `storage_key`.
        if self.ctrl.remember_scroll_offset {
            crate::scrollable::scroll_storage::save_offset(
                &self.ctrl.storage_key,
                Vec2d {
                    x: offset.x / ctx.scale,
                    y: offset.y / ctx.scale,
                },
            );
        }

        let offset = self.ctrl.visual_offset(offset);
        let snapped_offset = snap_scroll_offset(offset);
        let travel = visible_travel(self.ctrl.last_drawn_offset.get(), snapped_offset);
        if travel != Vec2d::ZERO
            && ctx.is_rect_visible(0.0, 0.0, layout_size.width, layout_size.height)
        {
            // Moving retained content changes the pixels in the fixed
            // viewport even though no widget state or layout bounds changed.
            // Report that viewport before child hover reconciliation can add
            // smaller damage rectangles and accidentally promote them all to
            // a full-target repaint.
            aimer_widget::PaintDamageTracker::new().mark_current_bounds_clipped(
                ctx,
                layout_size,
                ResolvedSize {
                    width: viewport_w.round(),
                    height: viewport_h.round(),
                },
                true,
            );
        }
        let content_moved = travel != Vec2d::ZERO;
        self.ctrl.last_drawn_offset.set(Some(snapped_offset));

        // Clip to viewport
        ctx.canvas.save();

        // Scroll offsets are already scaled into physical canvas coordinates.
        // Snap them directly so text keeps a stable rasterization phase while
        // moving and adjacent child edges cannot develop sub-pixel seams.
        let offset_x = snapped_offset.x;
        let offset_y = snapped_offset.y;

        ctx.canvas.translate(Vec2d {
            x: offset_x,
            y: offset_y,
        });

        let mut child_ctx = ctx.clone();
        child_ctx.box_constraint.min_width = child_ctx.box_constraint.min_width.min(viewport_w);
        child_ctx.box_constraint.min_height = child_ctx.box_constraint.min_height.min(viewport_h);
        child_ctx.box_constraint.max_width = viewport_w;
        child_ctx.box_constraint.max_height = viewport_h;
        child_ctx.parent_size = ResolvedSize {
            width: viewport_w,
            height: viewport_h,
        };
        match self.ctrl.axis {
            ScrollAxis::Vertical => child_ctx.box_constraint.max_height = f32::MAX,
            ScrollAxis::Horizontal => child_ctx.box_constraint.max_width = f32::MAX,
        }
        // The child is asked to materialize more than fits on screen. Handing
        // down the exact viewport makes every consumer of `visible_rect` cull
        // to it, which puts a line's whole cost — build, layout, shaping,
        // highlighting, glyph rasterization — on the single frame its edge
        // crosses the boundary, and that frame is the pause the user feels.
        // The extra content is still clipped on the GPU by the viewport clip
        // set above, so it costs nothing to draw.
        child_ctx.visible_rect = Some(cache_rect(
            self.ctrl.axis,
            Vec2d {
                x: -offset_x,
                y: -offset_y,
            },
            (viewport_w, viewport_h),
            travel,
        ));

        if aimer_widget::has_active_v2_render_tree() {
            let scale = ctx.scale;
            let child_pos = self.child.pos().unwrap_or_default();
            let child_x = child_pos.x + offset_x;
            let child_y = child_pos.y + offset_y;
            let bounds = Rect::new(
                child_x / scale,
                child_y / scale,
                content_size.width / scale,
                content_size.height / scale,
            );
            let viewport_clip = Rect::new(
                -child_x / scale,
                -child_y / scale,
                viewport_w / scale,
                viewport_h / scale,
            );
            let clip = self
                .child
                .retained_clip(&child_ctx)
                .map(|child_clip| intersect_rect(viewport_clip, child_clip))
                .or(Some(viewport_clip));
            let _ = aimer_widget::update_v2_render_node_geometry(self.child.id(), bounds, clip);

            if let Some(vertical_bar) = &self.vertical_scroll_bar {
                let active = matches!(self.ctrl.axis, ScrollAxis::Vertical);
                let bounds = if active {
                    Rect::new(
                        self.ctrl
                            .scroll_bar_placement
                            .cross_offset(viewport_w, self.vertical_bar_width)
                            / scale,
                        0.0,
                        self.vertical_bar_width / scale,
                        viewport_h / scale,
                    )
                } else {
                    Rect::new(0.0, 0.0, 0.0, 0.0)
                };
                let _ =
                    aimer_widget::update_v2_render_node_geometry(vertical_bar.id(), bounds, None);
            }
            if let Some(horizontal_bar) = &self.horizontal_scroll_bar {
                let active = matches!(self.ctrl.axis, ScrollAxis::Horizontal);
                let bounds = if active {
                    Rect::new(
                        0.0,
                        self.ctrl
                            .scroll_bar_placement
                            .cross_offset(viewport_h, self.horizontal_bar_height)
                            / scale,
                        viewport_w / scale,
                        self.horizontal_bar_height / scale,
                    )
                } else {
                    Rect::new(0.0, 0.0, 0.0, 0.0)
                };
                let _ =
                    aimer_widget::update_v2_render_node_geometry(horizontal_bar.id(), bounds, None);
            }
        }

        // The viewport clip makes the content rectangle a known paint bound.
        // Check it before entering the erased child so an off-screen scrollable
        // still updates its physics and bounds without walking its content.
        if ctx.is_rect_visible(0.0, 0.0, viewport_w, viewport_h) {
            let draw_content = || {
                if aimer_widget::has_active_v2_render_tree() {
                    self.child.rebuild_if_dirty(&child_ctx);
                    let mut v2_child_ctx = child_ctx.clone();
                    v2_child_ctx.parent_pos.x += offset_x;
                    v2_child_ctx.parent_pos.y += offset_y;
                    self.child.update(&v2_child_ctx);
                } else {
                    self.child.update(&child_ctx);
                }

                if provisional_extent || !self.child.is_layout_stable() {
                    self.refresh_content_size_after_draw(
                        ctx,
                        viewport_w,
                        viewport_h,
                        content_size,
                        provisional_extent,
                        preserve_content_end,
                    );
                }
            };
            let should_dispatch_hover = if content_moved
                && self.ctrl.drag_mode.get() == DragMode::None
                && self.event_dispatcher.borrow().capture_count() == 0
            {
                let (_, needs_reconciliation) =
                    with_deferred_mouse_region_hover_reconciliation(|| {
                        request_mouse_region_hover_reconciliation();
                        draw_content();
                    });
                needs_reconciliation
            } else {
                draw_content();
                false
            };
            if should_dispatch_hover {
                let event = ElementEvent::PointerMove(PointerInfo::mouse(
                    ctx.cursor_pos,
                    PointerButton::Primary,
                ));
                let _ = self.event_dispatcher.borrow_mut().dispatch(
                    &self.child,
                    ctx.cursor_pos,
                    &event,
                );
            }
        }

        // Restore before the scrollbars, which are separate nodes from the
        // clipped content whether they overlay it or reserve inline space.
        ctx.canvas.restore();

        if let Some(vertical_bar) = &self.vertical_scroll_bar
            && matches!(self.ctrl.axis, ScrollAxis::Vertical)
        {
            let mut bar_ctx = ctx.clone();
            bar_ctx.box_constraint.max_width = self.vertical_bar_width;
            bar_ctx.box_constraint.max_height = viewport_h;
            bar_ctx.parent_size = ResolvedSize {
                width: self.vertical_bar_width,
                height: viewport_h,
            };
            ctx.canvas.save();
            ctx.canvas.translate(Vec2d {
                x: self
                    .ctrl
                    .scroll_bar_placement
                    .cross_offset(viewport_w, self.vertical_bar_width),
                y: 0.0,
            });
            vertical_bar.update(&bar_ctx);
            ctx.canvas.restore();
        }
        if let Some(horizontal_bar) = &self.horizontal_scroll_bar
            && matches!(self.ctrl.axis, ScrollAxis::Horizontal)
        {
            let mut bar_ctx = ctx.clone();
            bar_ctx.box_constraint.max_width = viewport_w;
            bar_ctx.box_constraint.max_height = self.horizontal_bar_height;
            bar_ctx.parent_size = ResolvedSize {
                width: viewport_w,
                height: self.horizontal_bar_height,
            };
            ctx.canvas.save();
            ctx.canvas.translate(Vec2d {
                x: 0.0,
                y: self
                    .ctrl
                    .scroll_bar_placement
                    .cross_offset(viewport_h, self.horizontal_bar_height),
            });
            horizontal_bar.update(&bar_ctx);
            ctx.canvas.restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_frame_has_no_direction_to_lead_toward() {
        let travel = visible_travel(None, Vec2d { x: 3.0, y: 40.0 });

        assert_eq!(travel, Vec2d::ZERO);
    }

    #[test]
    fn scrolling_toward_the_content_end_moves_the_visible_rect_forward() {
        // Scrolling down translates the content up, i.e. to a more negative
        // offset, while the visible rectangle moves down the content.
        let travel = visible_travel(
            Some(Vec2d { x: 0.0, y: -100.0 }),
            Vec2d { x: 0.0, y: -140.0 },
        );

        assert_eq!(travel.y, 40.0);
    }

    #[test]
    fn scroll_translation_snaps_scaled_offsets_to_physical_pixels() {
        let offset = Vec2d {
            x: -10.49,
            y: -20.51,
        };

        let snapped = snap_scroll_offset(offset);

        assert_eq!(snapped.x, -10.0);
        assert_eq!(snapped.y, -21.0);
        assert_eq!(snapped.x.fract(), 0.0);
        assert_eq!(snapped.y.fract(), 0.0);
    }
}
