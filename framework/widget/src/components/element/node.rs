use super::*;

pub(super) struct ElementNode<E> {
    pub(super) id: Cell<ElementId>,
    pub(super) element: E,
}

impl<E: Element + 'static> ElementNode<E> {
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "portable-guest")))]
    fn clear_retained_compositor_animation(&self, render_context: Option<&V2RenderContext>) {
        // Almost no element ever has an animation to clear, so the question is
        // answered from a set that is empty unless something is animating.
        if !compositor_animation_may_be_applied(self.id.get()) {
            return;
        }
        let Some(scope) = render_context else {
            return;
        };
        let Some(render_node) = scope.node_for_element(self.id.get()) else {
            return;
        };
        if scope
            .tree
            .set_compositor_animation(
                render_node,
                aimer_cupid::utilities::Mat3::identity(),
                1.0,
                None,
            )
            .is_ok()
        {
            note_compositor_animation_cleared(self.id.get());
        }
    }

    #[inline]
    fn draw_live(&self, ctx: &BuildContext) {
        let before = element_tree_generation();
        self.element.update(ctx);
        let after = element_tree_generation();
        if after != before {
            self.set_subtree_generation(after);
        }
    }

    #[inline]
    fn draw_local_v2_compatibility_live(&self, ctx: &BuildContext) {
        let before = element_tree_generation();
        self.element.draw_local_v2_compatibility(ctx);
        let after = element_tree_generation();
        if after != before {
            self.set_subtree_generation(after);
        }
    }

    /// Walks the subtree of an element whose animation was already sampled this
    /// frame, so its controller is not ticked a second time.
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "portable-guest")))]
    fn draw_sampled_frame(&self, ctx: &BuildContext, frame: CompositorAnimationFrame) {
        self.draw_local_v2_compatibility_live(ctx);
        if frame.active {
            request_animation_frame();
        }
    }

    fn record_local_v2_paint(
        &self,
        ctx: &BuildContext,
        render_context: &V2RenderContext,
        render_node: RenderNodeId,
    ) -> bool {
        if !self.element.can_paint_local_v2(ctx) {
            return false;
        }
        let Ok(build_context) = render_context.tree.context(render_node) else {
            return false;
        };
        ctx.with_local_v2_paint_context(build_context, |ctx| {
            self.element.paint_local_v2(ctx)
        });
        // An element that paints nothing, a layout container for one, never
        // commits a list. The tree still has to learn that its paint is current.
        let _ = render_context.tree.mark_recording_attempted(render_node);
        render_context
            .tree
            .set_paint_source(render_node, RenderPaintSource::LocalV2)
            .is_ok()
    }

    /// Brings a node that already has a render node to a resolved local v2
    /// paint: settles the element's state, drops a stale list and records a
    /// fresh one when needed. Returns `false` when the element cannot paint
    /// through the retained path this frame.
    fn resolve_local_v2_paint(
        &self,
        ctx: &BuildContext,
        scope: &V2RenderContext,
        render_node: RenderNodeId,
    ) -> bool {
        let source = scope
            .tree
            .paint_source(render_node)
            .unwrap_or(RenderPaintSource::Unresolved);
        if (source == RenderPaintSource::LocalV2 || self.element.can_paint_local_v2(ctx))
            && !self.sync_local_v2_state(ctx, scope, render_node)
        {
            return false;
        }
        if source == RenderPaintSource::LocalV2
            && (super::take_event_paint_invalidated(self.id.get())
                || take_local_paint_invalidated(self.id.get())
                || self.element.local_v2_paint_needs_recording(ctx))
        {
            // Recorded again, and damaged only if the picture turns out to differ.
            let _ = scope.tree.refresh_paint(render_node);
        }
        let needs_recording = source == RenderPaintSource::Unresolved
            || scope.tree.needs_recording(render_node).unwrap_or(true);
        !needs_recording || self.record_local_v2_paint(ctx, scope, render_node)
    }

    #[inline]
    fn sync_local_v2_state(
        &self,
        ctx: &BuildContext,
        render_context: &V2RenderContext,
        render_node: RenderNodeId,
    ) -> bool {
        let Ok(paint_context) = render_context.tree.context(render_node) else {
            return false;
        };
        ctx.with_local_v2_paint_context(paint_context, |ctx| {
            self.element.sync_local_v2_state(ctx)
        })
    }

    /// Walks the subtree after this element's local paint is settled: state,
    /// bounds and children still update; the render tree owns presentation.
    fn draw_v2_compatibility(&self, ctx: &BuildContext) {
        if has_active_v2_render_presentation() {
            self.draw_local_v2_compatibility_live(ctx);
        } else {
            self.draw_live(ctx);
        }
    }

    fn update_inner(&self, ctx: &BuildContext) {
        crate::frame_work_stats::record_paint_call();
        #[cfg(feature = "frame-stats")]
        record_draw_traversal();
        let (_draw, outermost) = begin_draw();
        let render_context = current_v2_render_context();
        let prepared_root = outermost
            && render_context
                .as_ref()
                .is_some_and(|scope| scope.take_prepared_root(self.id.get()));
        // An animation-only pass revisits elements after the frame's own rebuild
        // prepass over the whole tree, so rebuilding again would tick every
        // animator's clock a second time and rebuild what it just rebuilt.
        if outermost && !prepared_root && !is_animation_only_pass() {
            // Native frame dispatch enters through `draw`; keep the retained-
            // tree rebuild prepass here so direct draw callers also benefit
            // from the precise dirty-subtree index. Child draws belong to the
            // same pass and must not reset paths relative to a new root.
            self.rebuild_if_dirty(ctx);
        }
        let _invalidation_owner = DrawInvalidationOwnerGuard::enter(self.id.get());
        capture_rebuilt_root(self.id.get(), ctx);

        #[cfg(all(not(target_arch = "wasm32"), not(feature = "portable-guest")))]
        let retained_animation_candidate = render_context.as_ref().is_some_and(|scope| {
            scope.node_for_element(self.id.get()).is_some()
                && self.element.can_paint_local_v2(ctx)
        });
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "portable-guest")))]
        if retained_animation_candidate {
            match self.element.compositor_animation(ctx) {
                CompositorAnimationDecision::Compositor(frame) => {
                    // Whatever happens next, the render node may now hold this
                    // animation, so a later frame without one has to clear it.
                    note_compositor_animation_applied(self.id.get());
                    if frame.valid
                        && self.element.can_paint_local_v2(ctx)
                        && let Some(scope) = render_context.as_ref()
                        && let Some(render_node) = scope.node_for_element(self.id.get())
                        && scope
                            .tree
                            .set_compositor_animation(
                                render_node,
                                compositor_transform_matrix(frame.transform, ctx.scale),
                                frame.opacity.unwrap_or(1.0),
                                frame.clip.map(|size| {
                                    Rect::new(0.0, 0.0, size.width, size.height)
                                }),
                            )
                            .is_ok()
                        && scope
                            .node_for_element(self.id.get())
                            .is_some_and(|render_node| {
                                self.resolve_local_v2_paint(ctx, scope, render_node)
                            })
                    {
                        // An animation-only pass revisits this element alone: its
                        // subtree has not changed, so walking it would only repeat
                        // work the last full frame already did.
                        if !is_animation_only_pass() {
                            self.draw_v2_compatibility(ctx);
                        }
                        if frame.active {
                            // Say that the animation is the only reason for the next
                            // frame, so a frame loop may skip the rest of the tree.
                            if register_animating_element(self.id.get(), ctx, false) {
                                aimer_events::window::request_scoped_frame();
                            } else {
                                request_animation_frame();
                            }
                        }
                        return;
                    }
                    self.clear_retained_compositor_animation(render_context.as_ref());
                    if let Some(scope) = render_context.as_ref()
                        && let Some(render_node) = scope.node_for_element(self.id.get())
                    {
                        // The frame is not usable for the retained path (a
                        // non-finite sample, say): keep the element's state
                        // advancing and paint nothing this frame.
                        self.paint_nothing(ctx, scope, render_node, || {
                            self.draw_sampled_frame(ctx, frame);
                        });
                    }
                    return;
                }
                CompositorAnimationDecision::Live(frame) => {
                    self.clear_retained_compositor_animation(render_context.as_ref());
                    if let Some(scope) = render_context.as_ref()
                        && let Some(render_node) = scope.node_for_element(self.id.get())
                    {
                        self.paint_nothing(ctx, scope, render_node, || {
                            self.draw_sampled_frame(ctx, frame);
                        });
                    }
                    return;
                }
                CompositorAnimationDecision::None => {
                    self.clear_retained_compositor_animation(render_context.as_ref());
                }
            }
        }

        if let Some(scope) = render_context.as_ref() {
            if let Some(render_node) = scope.node_for_element(self.id.get()) {
                if !self.resolve_local_v2_paint(ctx, scope, render_node) {
                    self.paint_nothing(ctx, scope, render_node, || {
                        self.draw_v2_compatibility(ctx);
                    });
                    return;
                }

                // Keep the compatibility traversal for draw-time state and
                // descendants. Direct retained presentation drops its paint
                // commands in `draw_v2_compatibility`; legacy islands reopen
                // recording for their own ranges. An animation-only pass
                // revisits an element that promised its work is its own, so
                // its subtree has nothing to learn from the visit.
                let _walk = enter_subtree_walk(self.id.get());
                if !pass_skips_subtree(self.id.get()) {
                    self.draw_v2_compatibility(ctx);
                }
                return;
            }
        }

        // Reached with no render node while a render tree is active: whatever this
        // element paints as legacy content lies outside every island range and is
        // never replayed. A transient miss (an element created during this very
        // draw) is mapped by the next sync; the frame loop reports only those
        // still unmapped afterwards.
        if render_context.is_some() {
            record_unmapped_draw(self.id.get(), self.element.debug_name());
        }
        self.draw_live(ctx);
    }

    /// Handles an element that has no retained paint this frame: it cannot
    /// record one (`can_paint_local_v2` is false), or its state is not valid
    /// enough to paint (a non-finite animation value, say).
    ///
    /// Such an element paints nothing. A node that already holds a list loses
    /// it (stale paint must not stay on screen) and is marked dirty; one that
    /// has none stays unresolved. Either way the next frame asks the element
    /// again, because an element can become paintable later (a layout that was
    /// not ready, an asset that arrived). `run`, its traversal, goes on with
    /// paint commands suppressed so state and children still update. Whatever
    /// the element tried to draw through `update` is dropped; debug builds
    /// report that once per element type.
    fn paint_nothing(
        &self,
        ctx: &BuildContext,
        render_context: &V2RenderContext,
        render_node: RenderNodeId,
        run: impl FnOnce(),
    ) {
        if render_context.tree.paint_source(render_node) == Ok(RenderPaintSource::LocalV2)
            && let Ok(build_context) = render_context.tree.context(render_node)
        {
            ctx.with_local_v2_paint_context(build_context, |ctx| {
                aimer_canvas::Canvas::of(ctx).finish();
            });
            let _ = render_context.tree.invalidate_paint(render_node);
        }
        run();
        #[cfg(debug_assertions)]
        {
            record_declined_paint(self.id.get(), self.element.debug_name());
        }
    }
}

unsafe impl<E: Element + 'static> ErasedFrom<ElementNode<E>> for dyn Element {
    const TEMPLATE: *const Self = std::ptr::null::<ElementNode<E>>() as *const dyn Element;
}

impl<E: Element + 'static> VisitorElement for ElementNode<E> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.element.visit_children(visitor);
    }

    fn visit_retained_v2_children<'a>(
        &'a self,
        visitor: &mut dyn FnMut(usize, &'a dyn Element),
    ) {
        self.element.visit_retained_v2_children(visitor);
    }

    fn debug_name(&self) -> &'static str {
        self.element.debug_name()
    }

    fn element_type_id(&self) -> TypeId {
        TypeId::of::<E>()
    }

    fn reconciliation_key(&self) -> Option<&Key> {
        self.element.reconciliation_key()
    }

    fn retained_identity(&self) -> Option<usize> {
        self.element.retained_identity()
    }

    fn is_retained_slot(&self) -> bool {
        self.element.is_retained_slot()
    }

    fn element_id(&self) -> Option<ElementId> {
        Some(self.id.get())
    }

    fn set_element_id(&self, id: ElementId) {
        self.id.set(id);
    }
}

impl<E: Element + 'static> LayoutElement for ElementNode<E> {
    fn pos(&self) -> Option<Vec2d> {
        self.element.pos()
    }

    fn size(&self) -> Option<Size> {
        self.element.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        crate::frame_work_stats::record_layout_call();
        self.element.layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.element.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.element.content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.element.layer()
    }

    fn flex(&self) -> Option<f32> {
        self.element.flex()
    }

    fn is_layout_stable(&self) -> bool {
        self.element.is_layout_stable()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.element.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        // Layout caches carry this generation in their keys, so invalidating
        // the retained root is one marker write rather than a recursive walk
        // through every child. The concrete element's old recursive method is
        // still available when a caller owns that concrete element directly;
        // erased trees use this boundary for the normal frame path.
        advance_layout_invalidation_generation();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.element.pos_start_end()
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        self.element.event_tree_bounds()
    }
}

impl<E: Element + 'static> Rebuildable for ElementNode<E> {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        #[cfg(feature = "frame-stats")]
        crate::rebuild_stats::record_visit();
        note_rebuild_visit();
        let mut traversal = begin_rebuild_traversal(self.id.get());
        let own_dirty = element_has_dirty_work(&self.element);
        let path_ready = dirty_paths_cover_current_traversal();
        let forced_descend = REBUILD_FORCE_DESCEND_DEPTH.with(|depth| depth.get() > 0);
        if path_ready
            && !forced_descend
            && !own_dirty
            && !dirty_path_contains(self.id.get())
        {
            traversal.complete = true;
            #[cfg(feature = "frame-stats")]
            crate::rebuild_stats::record_pruned();
            return;
        }

        note_rebuild_descent();
        // The path is only read below this point, so a pruned boundary, which
        // is nearly all of them while an animator runs, never pays for it.
        let _path = RebuildPathGuard::push(self.id.get());
        set_element_rebuild_path(&self.element);
        let _descend = RebuildDescendGuard::enter(own_dirty);
        let before = element_tree_generation();
        // With the index ready and no work of its own, this element is on a
        // path only because something below it is: let a container that can
        // reach its children by position rebuild just those.
        let indexed = path_ready && !forced_descend && !own_dirty;
        if !(indexed && self.element.rebuild_indexed_children(ctx)) {
            self.element.rebuild_if_dirty(ctx);
        }
        let after = element_tree_generation();
        if after != before {
            self.set_subtree_generation(after);
        }
        // A descendant rebuild advances the shared generation but does not
        // stale this node's own local paint. The descendant's dirty path and
        // retained-tree sync carry that change to its actual paint owner.
        if own_dirty || after != before {
            record_current_paint_invalidation(self.id.get(), own_dirty);
        }
        traversal.complete = true;
    }

    fn option_any(&self) -> Option<&dyn std::any::Any> {
        self.element.option_any()
    }

    fn is_stateful_element(&self) -> bool {
        self.element.is_stateful_element()
    }

    fn forwards_marks_only(&self) -> bool {
        self.element.forwards_marks_only()
    }

    fn is_carry_state(&self) -> bool {
        self.element.is_carry_state()
    }

    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let before = element_tree_generation();
        self.element.adopt_runtime_state_from(old);
        let after = element_tree_generation();
        if after != before {
            self.set_subtree_generation(after);
        }
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.element.with_rebuild_context(ctx, callback);
    }

    fn mark_needs_rebuild(&self) {
        let (_mark, outermost) = begin_rebuild_mark();
        let has_rebuild_source = element_has_rebuild_source(&self.element);
        // The erased owner is the boundary used by the retained tree. Keep one
        // invalidation generation for the complete recursive mark, including
        // custom containers that implement their own forwarding method.
        // An element with nothing under it and no source of rebuilds of its own
        // has no work a mark could reach, so it cannot make the index skip any.
        // Marking such leaves is what every retained child does each time its
        // parent places it again, which an animation does every frame; treating
        // each as an unknown producer discarded the index and sent the next pass
        // through the whole page.
        //
        // An element that only forwards the mark to its children (a container, a
        // retained child) is no producer either: what it reaches is judged where
        // the mark gets there. Marking one each frame is as ordinary as marking a
        // leaf, and treating it as unknown did the same damage.
        let opaque = !has_rebuild_source
            && has_children(&self.element)
            && !self.element.forwards_marks_only();
        if outermost {
            MARK_REACHED_OPAQUE_ELEMENT.with(|reached| reached.set(false));
        } else if opaque {
            MARK_REACHED_OPAQUE_ELEMENT.with(|reached| reached.set(true));
        }
        let unknown = outermost && opaque;
        with_rebuild_invalidation(|| {
            if unknown {
                // A custom rebuild producer has no path to publish. Preserve
                // the old conservative behavior so its mark can never make
                // the index incorrectly skip work.
                mark_paint_invalidations_unknown();
                invalidate_dirty_paths();
            }
            self.element.mark_needs_rebuild();
        });
        // A forwarder at the root of the mark decided nothing for what lies
        // below it. If the mark reached an element that could be a producer, the
        // index cannot be trusted any more than if that element had been marked
        // on its own.
        if outermost
            && !has_rebuild_source
            && !unknown
            && MARK_REACHED_OPAQUE_ELEMENT.with(Cell::get)
        {
            mark_paint_invalidations_unknown();
            invalidate_dirty_paths();
        }
        // A stateful element may delegate the mark to a carrying child without
        // marking itself. If that child contains an untracked custom producer,
        // the retained path index has no source to point at; fall back to the
        // conservative walk for this explicit recursive mark.
        if outermost
            && has_rebuild_source
            && !element_has_dirty_work(&self.element)
            && !placement_left_clean(&self.element)
        {
            mark_paint_invalidations_unknown();
            invalidate_dirty_paths();
        }
    }

    fn subtree_generation(&self) -> u64 {
        if !self.element.is_layout_stable() {
            return element_tree_generation();
        }
        STABLE_SUBTREE_GENERATIONS.with(|generations| {
            generations.borrow().get(&self.id.get()).copied().unwrap_or(0)
        })
    }

    fn set_subtree_generation(&self, generation: u64) {
        if self.element.is_layout_stable() {
            let id = self.id.get();
            let previous = STABLE_SUBTREE_GENERATIONS
                .with(|generations| generations.borrow_mut().insert(id, generation));
            if previous != Some(generation) {
                note_stable_generation_change(id);
            }
        }
    }
}

aimer_tls::thread_local! {
    /// Whether the recursive mark in progress reached an element that could be a
    /// producer of rebuilds the dirty-path index does not know of.
    static MARK_REACHED_OPAQUE_ELEMENT: Cell<bool> = const { Cell::new(false) };
}

#[inline]
fn has_children(element: &dyn Element) -> bool {
    let mut any = false;
    element.visit_children(&mut |_| any = true);
    any
}

fn element_has_rebuild_source(element: &dyn Element) -> bool {
    let Some(value) = element.option_any() else {
        return false;
    };
    value.is::<crate::widget::stateless::StatelessElement>()
        || value.is::<crate::widget::stateful::StatefulElement>()
}

/// Returns whether a placement mark left `element` clean on purpose.
///
/// A placement skips a stateful element whose last build read no inherited
/// context. That is a known outcome, not a producer the dirty-path index lost
/// track of, so it must not discard the index.
fn placement_left_clean(element: &dyn Element) -> bool {
    if !super::placement_mark_active() {
        return false;
    }
    let Some(value) = element.option_any() else {
        return false;
    };
    value
        .downcast_ref::<crate::widget::stateful::StatefulElement>()
        .is_some_and(|stateful| !stateful.dirty_source.borrow().reads_context())
}

fn element_has_dirty_work(element: &dyn Element) -> bool {
    let Some(value) = element.option_any() else {
        return false;
    };
    if let Some(stateless) = value
        .downcast_ref::<crate::widget::stateless::StatelessElement>()
    {
        return stateless.dirty.get();
    }
    value
        .downcast_ref::<crate::widget::stateful::StatefulElement>()
        .is_some_and(|stateful| stateful.dirty.borrow().get())
}

fn set_element_rebuild_path(element: &dyn Element) {
    let Some(value) = element.option_any() else {
        return;
    };
    REBUILD_PATH.with(|path| {
        let path = path.borrow();
        if let Some(stateless) = value
            .downcast_ref::<crate::widget::stateless::StatelessElement>()
        {
            stateless.dirty_source.set_path(&path);
        } else if let Some(stateful) = value
            .downcast_ref::<crate::widget::stateful::StatefulElement>()
        {
            stateful.dirty_source.borrow().set_path(&path);
        }
    });
}

impl<E: Element + 'static> EventElement for ElementNode<E> {
    fn event_tree_role(&self) -> EventTreeRole {
        self.element.event_tree_role()
    }

    fn focus_node(&self) -> Option<&FocusNode> {
        self.element.focus_node()
    }

    fn autofocus(&self) -> bool {
        self.element.autofocus()
    }

    fn traps_focus(&self) -> bool {
        self.element.traps_focus()
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        let result = self.element.on_event(event);
        if result.needs_redraw() {
            super::mark_event_paint_invalidated(self.id.get());
        }
        result
    }

    fn on_event_with_context(
        &self,
        event: &ElementEvent,
        context: &mut EventDispatchContext<'_, '_>,
    ) -> EventResult {
        let result = self.element.on_event_with_context(event, context);
        if result.needs_redraw() {
            super::mark_event_paint_invalidated(self.id.get());
        }
        result
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.element.event_children(visitor);
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.element.structural_children(visitor);
    }

    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.element.focus_children(visitor);
    }

    fn hit_test_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.element.hit_test_children(visitor);
    }

    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.element.hit_test_children_reversed(visitor);
    }

    fn hit_test_children_at<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.element.hit_test_children_at(pos, visitor);
    }

    fn hit_test_children_at_reversed<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.element.hit_test_children_at_reversed(pos, visitor);
    }

    #[inline]
    fn has_overlapping_hit_targets(&self) -> bool {
        self.element.has_overlapping_hit_targets()
    }

    #[inline]
    fn occludes_pointer_at(&self, pos: Vec2d) -> bool {
        self.element.occludes_pointer_at(pos)
    }

    #[inline]
    fn occlusion_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.element.occlusion_children_at(pos, visitor);
    }

}

impl<E: Element + 'static> Drawable for ElementNode<E> {
    fn update(&self, ctx: &BuildContext) {
        self.update_inner(ctx);
    }

    #[inline]
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.element.can_paint_local_v2(ctx)
    }

    #[inline]
    fn paint_local_v2(&self, ctx: &BuildContext) {
        self.element.paint_local_v2(ctx)
    }

    #[inline]
    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        self.element.sync_local_v2_state(ctx)
    }

    #[inline]
    fn retained_clip(&self, ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd_v2::Rect> {
        self.element.retained_clip(ctx)
    }

    #[inline]
    fn retained_v2_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        self.element.retained_v2_bounds(ctx)
    }

    #[inline]
    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        self.element.retained_v2_paint_outsets(ctx)
    }

    #[inline]
    fn retained_v2_interaction_size(
        &self,
        ctx: &BuildContext,
    ) -> Option<aimer_attribute::size::ResolvedSize> {
        self.element.retained_v2_interaction_size(ctx)
    }

    #[inline]
    fn retained_v2_interaction_offset(&self, ctx: &BuildContext) -> (f32, f32) {
        self.element.retained_v2_interaction_offset(ctx)
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(
        &self,
        source: crate::components::interaction_bounds::InteractionSource,
    ) {
        self.element.adopt_retained_v2_interaction_source(source)
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<crate::components::interaction_bounds::InteractionDisagreement> {
        self.element.retained_v2_interaction_disagreement()
    }

    #[inline]
    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        self.element.retained_v2_child_context(ctx, child)
    }

    #[inline]
    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        self.element
            .retained_v2_child_context_at(ctx, child, child_index)
    }

    #[inline]
    fn retained_v2_child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        self.element.retained_v2_child_geometry(ctx, child)
    }

    #[inline]
    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        self.element
            .retained_v2_child_geometry_at(ctx, child, child_index)
    }

    #[inline]
    fn retained_v2_child_clip_radius(&self, ctx: &BuildContext, child: &dyn Element) -> [f32; 4] {
        self.element.retained_v2_child_clip_radius(ctx, child)
    }

    #[inline]
    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.element.local_v2_paint_needs_recording(ctx)
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.element.sync_paint_geometry(ctx);
    }

    #[inline]
    fn prepare_layout(&self, ctx: &BuildContext) -> bool {
        self.element.prepare_layout(ctx)
    }

    #[inline]
    fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
        self.element.compositor_animation(ctx)
    }

}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "portable-guest")))]
fn compositor_transform_matrix(
    transform: CompositorTransform,
    device_scale: f32,
) -> aimer_cupid::utilities::Mat3 {
    use aimer_cupid::utilities::Mat3;

    let device_scale = if device_scale.is_finite() && device_scale > 0.0 {
        device_scale
    } else {
        1.0
    };
    match transform {
        CompositorTransform::Identity => Mat3::identity(),
        CompositorTransform::Translate { x, y } => {
            Mat3::translate(x / device_scale, y / device_scale)
        }
        CompositorTransform::Scale {
            sx,
            sy,
            origin_x,
            origin_y,
        } => Mat3::translate(origin_x / device_scale, origin_y / device_scale)
            .mul(&Mat3::scale(sx, sy))
            .mul(&Mat3::translate(-origin_x / device_scale, -origin_y / device_scale)),
        CompositorTransform::Rotate {
            radians,
            origin_x,
            origin_y,
        } => Mat3::translate(origin_x / device_scale, origin_y / device_scale)
            .mul(&Mat3::rotate(radians))
            .mul(&Mat3::translate(-origin_x / device_scale, -origin_y / device_scale)),
    }
}

impl VisitorElement for AnyElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().visit_children(visitor)
    }

    fn visit_retained_v2_children<'a>(
        &'a self,
        visitor: &mut dyn FnMut(usize, &'a dyn Element),
    ) {
        self.as_ref().visit_retained_v2_children(visitor)
    }

    fn debug_name(&self) -> &'static str {
        self.as_ref().debug_name()
    }

    fn element_type_id(&self) -> std::any::TypeId {
        self.as_ref().element_type_id()
    }

    fn reconciliation_key(&self) -> Option<&Key> {
        self.as_ref().reconciliation_key()
    }

    fn retained_identity(&self) -> Option<usize> {
        self.as_ref().retained_identity()
    }

    fn is_retained_slot(&self) -> bool {
        self.as_ref().is_retained_slot()
    }

    fn element_id(&self) -> Option<ElementId> {
        self.as_ref().element_id()
    }

    fn set_element_id(&self, id: ElementId) {
        self.as_ref().set_element_id(id);
    }
}

impl LayoutElement for AnyElement {
    fn pos(&self) -> Option<Vec2d> {
        self.as_ref().pos()
    }

    fn size(&self) -> Option<Size> {
        self.as_ref().size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.as_ref().layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.as_ref().computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.as_ref().content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.as_ref().layer()
    }

    fn flex(&self) -> Option<f32> {
        self.as_ref().flex()
    }

    fn is_layout_stable(&self) -> bool {
        self.as_ref().is_layout_stable()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.as_ref().get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.as_ref().invalidate_layout()
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.as_ref().pos_start_end()
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        self.as_ref().event_tree_bounds()
    }
}

impl Rebuildable for AnyElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.as_ref().rebuild_if_dirty(ctx)
    }

    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        self.as_ref().adopt_runtime_state_from(old)
    }

    fn option_any(&self) -> Option<&dyn std::any::Any> {
        self.as_ref().option_any()
    }

    fn is_stateful_element(&self) -> bool {
        self.as_ref().is_stateful_element()
    }

    fn forwards_marks_only(&self) -> bool {
        self.as_ref().forwards_marks_only()
    }

    fn is_carry_state(&self) -> bool {
        self.as_ref().is_carry_state()
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.as_ref().with_rebuild_context(ctx, callback)
    }

    fn mark_needs_rebuild(&self) {
        self.as_ref().mark_needs_rebuild()
    }

    fn subtree_generation(&self) -> u64 {
        self.as_ref().subtree_generation()
    }

    fn set_subtree_generation(&self, generation: u64) {
        self.as_ref().set_subtree_generation(generation)
    }
}

impl EventElement for AnyElement {
    fn event_tree_role(&self) -> EventTreeRole {
        self.as_ref().event_tree_role()
    }

    fn focus_node(&self) -> Option<&FocusNode> {
        self.as_ref().focus_node()
    }

    fn autofocus(&self) -> bool {
        self.as_ref().autofocus()
    }

    fn traps_focus(&self) -> bool {
        self.as_ref().traps_focus()
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.as_ref().on_event(event)
    }

    fn on_event_with_context(
        &self,
        event: &ElementEvent,
        context: &mut EventDispatchContext<'_, '_>,
    ) -> EventResult {
        self.as_ref().on_event_with_context(event, context)
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().event_children(visitor)
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().structural_children(visitor)
    }

    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().focus_children(visitor)
    }

    fn hit_test_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().hit_test_children(visitor)
    }

    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().hit_test_children_reversed(visitor)
    }

    fn hit_test_children_at<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.as_ref().hit_test_children_at(pos, visitor)
    }

    fn hit_test_children_at_reversed<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.as_ref().hit_test_children_at_reversed(pos, visitor)
    }

    #[inline]
    fn has_overlapping_hit_targets(&self) -> bool {
        self.as_ref().has_overlapping_hit_targets()
    }

    #[inline]
    fn occludes_pointer_at(&self, pos: Vec2d) -> bool {
        self.as_ref().occludes_pointer_at(pos)
    }

    #[inline]
    fn occlusion_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().occlusion_children_at(pos, visitor);
    }

}

impl Drawable for AnyElement {
    #[inline]
    fn update(&self, ctx: &BuildContext) {
        self.as_ref().update(ctx)
    }

    #[inline]
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.as_ref().can_paint_local_v2(ctx)
    }

    #[inline]
    fn paint_local_v2(&self, ctx: &BuildContext) {
        self.as_ref().paint_local_v2(ctx)
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.as_ref().sync_paint_geometry(ctx)
    }

    #[inline]
    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        self.as_ref().sync_local_v2_state(ctx)
    }

    #[inline]
    fn prepare_layout(&self, ctx: &BuildContext) -> bool {
        self.as_ref().prepare_layout(ctx)
    }

    #[inline]
    fn retained_clip(&self, ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd_v2::Rect> {
        self.as_ref().retained_clip(ctx)
    }

    #[inline]
    fn retained_v2_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        self.as_ref().retained_v2_bounds(ctx)
    }

    #[inline]
    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        self.as_ref().retained_v2_paint_outsets(ctx)
    }

    #[inline]
    fn retained_v2_interaction_size(
        &self,
        ctx: &BuildContext,
    ) -> Option<aimer_attribute::size::ResolvedSize> {
        self.as_ref().retained_v2_interaction_size(ctx)
    }

    #[inline]
    fn retained_v2_interaction_offset(&self, ctx: &BuildContext) -> (f32, f32) {
        self.as_ref().retained_v2_interaction_offset(ctx)
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(
        &self,
        source: crate::components::interaction_bounds::InteractionSource,
    ) {
        self.as_ref().adopt_retained_v2_interaction_source(source)
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<crate::components::interaction_bounds::InteractionDisagreement> {
        self.as_ref().retained_v2_interaction_disagreement()
    }

    #[inline]
    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        self.as_ref().retained_v2_child_context(ctx, child)
    }

    #[inline]
    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        self.as_ref()
            .retained_v2_child_context_at(ctx, child, child_index)
    }

    #[inline]
    fn retained_v2_child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        self.as_ref().retained_v2_child_geometry(ctx, child)
    }

    #[inline]
    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        self.as_ref()
            .retained_v2_child_geometry_at(ctx, child, child_index)
    }

    #[inline]
    fn retained_v2_child_clip_radius(&self, ctx: &BuildContext, child: &dyn Element) -> [f32; 4] {
        self.as_ref().retained_v2_child_clip_radius(ctx, child)
    }

    #[inline]
    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.as_ref().local_v2_paint_needs_recording(ctx)
    }

    #[inline]
    fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
        self.as_ref().compositor_animation(ctx)
    }

}

impl VisitorElement for Box<dyn Element> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().visit_children(visitor)
    }

    fn visit_retained_v2_children<'a>(
        &'a self,
        visitor: &mut dyn FnMut(usize, &'a dyn Element),
    ) {
        self.as_ref().visit_retained_v2_children(visitor)
    }
    fn debug_name(&self) -> &'static str {
        self.as_ref().debug_name()
    }
    fn element_type_id(&self) -> std::any::TypeId {
        self.as_ref().element_type_id()
    }
    fn reconciliation_key(&self) -> Option<&Key> {
        self.as_ref().reconciliation_key()
    }

    fn retained_identity(&self) -> Option<usize> {
        self.as_ref().retained_identity()
    }

    fn is_retained_slot(&self) -> bool {
        self.as_ref().is_retained_slot()
    }
    fn element_id(&self) -> Option<ElementId> {
        self.as_ref().element_id()
    }
    fn set_element_id(&self, id: ElementId) {
        self.as_ref().set_element_id(id);
    }
}

impl LayoutElement for Box<dyn Element> {
    fn pos(&self) -> Option<Vec2d> {
        self.as_ref().pos()
    }
    fn size(&self) -> Option<Size> {
        self.as_ref().size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.as_ref().layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.as_ref().computed_size(ctx)
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.as_ref().content_size(ctx)
    }
    fn layer(&self) -> u32 {
        self.as_ref().layer()
    }

    fn flex(&self) -> Option<f32> {
        self.as_ref().flex()
    }

    fn is_layout_stable(&self) -> bool {
        self.as_ref().is_layout_stable()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.as_ref().get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.as_ref().invalidate_layout()
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.as_ref().pos_start_end()
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        self.as_ref().event_tree_bounds()
    }
}

impl Rebuildable for Box<dyn Element> {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.as_ref().rebuild_if_dirty(ctx)
    }

    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        self.as_ref().adopt_runtime_state_from(old)
    }

    fn option_any(&self) -> Option<&dyn std::any::Any> {
        self.as_ref().option_any()
    }

    fn is_stateful_element(&self) -> bool {
        self.as_ref().is_stateful_element()
    }

    fn forwards_marks_only(&self) -> bool {
        self.as_ref().forwards_marks_only()
    }

    fn is_carry_state(&self) -> bool {
        self.as_ref().is_carry_state()
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.as_ref().with_rebuild_context(ctx, callback)
    }

    fn mark_needs_rebuild(&self) {
        self.as_ref().mark_needs_rebuild()
    }

    fn subtree_generation(&self) -> u64 {
        self.as_ref().subtree_generation()
    }

    fn set_subtree_generation(&self, generation: u64) {
        self.as_ref().set_subtree_generation(generation)
    }
}

impl EventElement for Box<dyn Element> {
    fn event_tree_role(&self) -> EventTreeRole {
        self.as_ref().event_tree_role()
    }

    fn focus_node(&self) -> Option<&FocusNode> {
        self.as_ref().focus_node()
    }

    fn autofocus(&self) -> bool {
        self.as_ref().autofocus()
    }

    fn traps_focus(&self) -> bool {
        self.as_ref().traps_focus()
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.as_ref().on_event(event)
    }

    fn on_event_with_context(
        &self,
        event: &ElementEvent,
        context: &mut EventDispatchContext<'_, '_>,
    ) -> EventResult {
        self.as_ref().on_event_with_context(event, context)
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().event_children(visitor)
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().structural_children(visitor)
    }

    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().focus_children(visitor)
    }

    fn hit_test_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().hit_test_children(visitor)
    }

    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().hit_test_children_reversed(visitor)
    }

    fn hit_test_children_at<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.as_ref().hit_test_children_at(pos, visitor)
    }

    fn hit_test_children_at_reversed<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.as_ref().hit_test_children_at_reversed(pos, visitor)
    }

    #[inline]
    fn has_overlapping_hit_targets(&self) -> bool {
        self.as_ref().has_overlapping_hit_targets()
    }

    #[inline]
    fn occludes_pointer_at(&self, pos: Vec2d) -> bool {
        self.as_ref().occludes_pointer_at(pos)
    }

    #[inline]
    fn occlusion_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.as_ref().occlusion_children_at(pos, visitor);
    }

}

impl Drawable for Box<dyn Element> {
    #[inline]
    fn update(&self, ctx: &BuildContext) {
        self.as_ref().update(ctx)
    }

    #[inline]
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.as_ref().can_paint_local_v2(ctx)
    }

    #[inline]
    fn paint_local_v2(&self, ctx: &BuildContext) {
        self.as_ref().paint_local_v2(ctx)
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.as_ref().sync_paint_geometry(ctx)
    }

    #[inline]
    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        self.as_ref().sync_local_v2_state(ctx)
    }

    #[inline]
    fn prepare_layout(&self, ctx: &BuildContext) -> bool {
        self.as_ref().prepare_layout(ctx)
    }

    #[inline]
    fn retained_clip(&self, ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd_v2::Rect> {
        self.as_ref().retained_clip(ctx)
    }

    #[inline]
    fn retained_v2_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        self.as_ref().retained_v2_bounds(ctx)
    }

    #[inline]
    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        self.as_ref().retained_v2_paint_outsets(ctx)
    }

    #[inline]
    fn retained_v2_interaction_size(
        &self,
        ctx: &BuildContext,
    ) -> Option<aimer_attribute::size::ResolvedSize> {
        self.as_ref().retained_v2_interaction_size(ctx)
    }

    #[inline]
    fn retained_v2_interaction_offset(&self, ctx: &BuildContext) -> (f32, f32) {
        self.as_ref().retained_v2_interaction_offset(ctx)
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(
        &self,
        source: crate::components::interaction_bounds::InteractionSource,
    ) {
        self.as_ref().adopt_retained_v2_interaction_source(source)
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<crate::components::interaction_bounds::InteractionDisagreement> {
        self.as_ref().retained_v2_interaction_disagreement()
    }

    #[inline]
    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        self.as_ref().retained_v2_child_context(ctx, child)
    }

    #[inline]
    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        self.as_ref()
            .retained_v2_child_context_at(ctx, child, child_index)
    }

    #[inline]
    fn retained_v2_child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        self.as_ref().retained_v2_child_geometry(ctx, child)
    }

    #[inline]
    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        self.as_ref()
            .retained_v2_child_geometry_at(ctx, child, child_index)
    }

    #[inline]
    fn retained_v2_child_clip_radius(&self, ctx: &BuildContext, child: &dyn Element) -> [f32; 4] {
        self.as_ref().retained_v2_child_clip_radius(ctx, child)
    }

    #[inline]
    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.as_ref().local_v2_paint_needs_recording(ctx)
    }

    #[inline]
    fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
        self.as_ref().compositor_animation(ctx)
    }

}

