/// Owns the one caret child for a mounted text field.
///
/// `TextFieldState` rebuilds its raw widget as configuration changes. Keeping
/// this slot in the state means a custom caret factory runs once, while the
/// resulting element is handed to each candidate tree through a
/// [`ChildBuilder`]. The mode bit allows `builtin_caret` to replace a custom
/// caret without exposing the caret to the field's editing state.
///
/// [`ChildBuilder`]: aimer_widget::ChildBuilder
pub(crate) struct CaretSlot {
    child: RefCell<Option<ChildBuilder>>,
    custom: Cell<Option<bool>>,
    builtin_color: Rc<Cell<Color>>,
}

impl CaretSlot {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            child: RefCell::new(None),
            custom: Cell::new(None),
            builtin_color: Rc::new(Cell::new(Color::default())),
        }
    }

    #[inline]
    pub(crate) fn build(
        &self,
        builder: Option<&CaretBuilder>,
        context: CaretContext,
        color: Colors,
        ctx: &BuildContext,
    ) -> AnyElement {
        let custom = builder.is_some();
        let color = color.into();
        if self.builtin_color.replace(color) != color {
            context.request_repaint();
        }
        let mut child = self.child.borrow_mut();
        if self.custom.get() != Some(custom) {
            let widget = builder
                .map(|builder| builder.build(context.clone()))
                .unwrap_or_else(|| {
                    DefaultCaret::with_color_cell(context, self.builtin_color.clone()).boxed()
                });
            let retained = ChildBuilder::from_widget(widget);
            let element = retained.build(ctx);
            *child = Some(retained);
            self.custom.set(Some(custom));
            element
        } else {
            child
                .as_ref()
                .expect("a caret slot's mode must have a matching child")
                .build(ctx)
        }
    }
}

/// Retains the field's interaction element and its independently built caret
/// widget as sibling children.
///
/// The field computes the caret rectangle while it paints text. The host then
/// translates the retained caret element to that rectangle without exposing the
/// caret to pointer hit testing or focus traversal.
pub(crate) struct RawTextFieldHost {
    pub(crate) field: RawTextField,
    pub(crate) caret: AnyElement,
    caret_placement: Rc<Cell<CaretPlacement>>,
}

impl RawTextFieldHost {
    #[inline]
    pub(crate) fn new(field: RawTextField, caret: AnyElement) -> Self {
        let caret_placement = Rc::new(Cell::new(CaretPlacement::default()));
        let caret = RawCaretPlacement {
            child: caret,
            placement: Rc::clone(&caret_placement),
        }
        .boxed();
        Self {
            field,
            caret,
            caret_placement,
        }
    }

    fn sync_caret_placement(&self, ctx: &BuildContext) {
        let context = self.field.caret_context();
        let geometry = context.geometry();
        let scale = if ctx.scale > 0.0 { ctx.scale } else { 1.0 };
        let available = context.is_focused() && context.is_available();
        let placement = if available {
            CaretPlacement {
                offset: Vec2d {
                    x: geometry.x * scale,
                    y: geometry.y * scale,
                },
                size: ResolvedSize {
                    width: geometry.width * scale,
                    height: geometry.height * scale,
                },
            }
        } else {
            CaretPlacement::default()
        };
        let previous = self.caret_placement.replace(placement);
        if previous != placement {
            // The caret node's transform is read from `pos()` when the render
            // tree synchronizes, and a synchronization is skipped while nothing
            // it depends on has moved. The field publishes the caret while it
            // paints, after this frame's synchronization, so without this the
            // caret would stay where the previous edit left it. Nothing is laid
            // out differently: the caret is out of flow, so retiring the layout
            // caches would only make the next pass measure the window again.
            aimer_widget::notify_retained_render_structure_changed();
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
struct CaretPlacement {
    offset: Vec2d,
    size: ResolvedSize,
}

struct RawCaretPlacement {
    child: AnyElement,
    placement: Rc<Cell<CaretPlacement>>,
}

impl VisitorElement for RawCaretPlacement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "TextFieldCaretPlacement"
    }
}

impl EventElement for RawCaretPlacement {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl Rebuildable for RawCaretPlacement {
    /// Only hands a mark on to its child, so placing it again leaves the
    /// dirty-path index as it was; see [`Rebuildable::forwards_marks_only`].
    #[inline]
    fn forwards_marks_only(&self) -> bool {
        true
    }

    fn is_carry_state(&self) -> bool {
        self.child.is_carry_state()
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.child.with_rebuild_context(ctx, callback);
    }
}

impl LayoutElement for RawCaretPlacement {
    fn pos(&self) -> Option<Vec2d> {
        Some(self.placement.get().offset)
    }

    fn layout(&self, _ctx: &BuildContext) -> ResolvedSize {
        self.placement.get().size
    }

    fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        self.placement.get().size
    }

    fn content_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        self.placement.get().size
    }
}

impl Drawable for RawCaretPlacement {
    fn update(&self, ctx: &BuildContext) {
        let placement = self.placement.get();
        let mut caret_ctx = ctx.clone();
        caret_ctx.parent_size = placement.size;
        caret_ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: placement.size.width,
            max_height: placement.size.height,
        };
        caret_ctx.visible_rect = ctx.visible_rect.map(|(x, y, width, height)| {
            (
                x - placement.offset.x,
                y - placement.offset.y,
                width,
                height,
            )
        });
        ctx.canvas.save();
        ctx.canvas.translate(placement.offset);
        self.child.update(&caret_ctx);
        ctx.canvas.restore();
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

}

impl Drawable for RawTextFieldHost {
    fn update(&self, ctx: &BuildContext) {
        self.field.update(ctx);
        self.sync_caret_placement(ctx);
        self.caret.update(ctx);
    }

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        if !self.field.sync_local_v2_state(ctx) {
            return false;
        }
        self.sync_caret_placement(ctx);
        true
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.field.draw_local_v2_compatibility(ctx);
        self.sync_caret_placement(ctx);
        self.caret.update(ctx);
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        self.field.retained_v2_paint_outsets(ctx)
    }

    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.field.local_v2_paint_needs_recording(ctx)
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.field.can_paint_local_v2(ctx)
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        self.field.paint_local_v2(ctx);
    }

    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        self.field.retained_v2_interaction_size(ctx)
    }

    fn retained_v2_interaction_offset(&self, ctx: &BuildContext) -> (f32, f32) {
        self.field.retained_v2_interaction_offset(ctx)
    }

    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.field.adopt_retained_v2_interaction_source(source);
    }

    fn retained_v2_interaction_disagreement(&self) -> Option<aimer_widget::InteractionDisagreement> {
        self.field.retained_v2_interaction_disagreement()
    }
}

impl VisitorElement for RawTextFieldHost {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.caret);
    }

    fn debug_name(&self) -> &'static str {
        "TextField"
    }
}

impl EventElement for RawTextFieldHost {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.field.on_event(event)
    }

    // The host carries the element identity, so it is the host that publishes
    // the embedded field.
    fn semantics(&self, id: aimer_accessibility::NodeId) -> Option<aimer_accessibility::SemanticNode> {
        Some(self.field.semantic_node(id))
    }

    fn perform_semantic_action(&self, action: &aimer_accessibility::SemanticAction) -> bool {
        self.field.perform_semantic_action(action)
    }

    fn on_event_with_context(
        &self,
        event: &ElementEvent,
        _context: &mut EventDispatchContext<'_, '_>,
    ) -> EventResult {
        // `field` is embedded and therefore has no retained ElementId. The
        // host is the indexed event target and owns pointer capture for its
        // editing logic, so forward directly instead of indexing it as a
        // separate child target.
        self.field.on_event(event)
    }

    fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {
        // The field is embedded and has no retained ElementId of its own.
        // Its callbacks are forwarded directly by this host, so it must not be
        // registered as a separate indexed event target.
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.field);
        visitor(&self.caret);
    }

    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.field);
    }

    fn hit_test_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {
    }
}

impl LayoutElement for RawTextFieldHost {
    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.field.layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.field.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.field.content_size(ctx)
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.field.pos_start_end()
    }
}

impl Rebuildable for RawTextFieldHost {
    /// Only hands a mark on to its child, so placing it again leaves the
    /// dirty-path index as it was; see [`Rebuildable::forwards_marks_only`].
    #[inline]
    fn forwards_marks_only(&self) -> bool {
        true
    }

    fn is_carry_state(&self) -> bool {
        true
    }

    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|value| value.downcast_ref::<Self>())
        else {
            return;
        };
        self.field.adopt_runtime_state_from(&old.field);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimer_cupid::utilities::Rgba8;
    use crate::input_field::caret::{CaretBlink, CaretGeometry};
    use crate::input_field::raw_fields::test_support::{dummy_build_context, field_config};

    struct ProbeCaret(Rc<Cell<(f32, f32)>>);

    impl aimer_widget::PortableWidget for ProbeCaret {}

    impl Widget for ProbeCaret {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            ProbeCaretElement(self.0).boxed()
        }
    }

    struct ProbeCaretElement(Rc<Cell<(f32, f32)>>);

    impl Drawable for ProbeCaretElement {
        fn update(&self, ctx: &BuildContext) {
            self.0
                .set((ctx.parent_size.width, ctx.parent_size.height));
        }
    }

    impl VisitorElement for ProbeCaretElement {
        fn debug_name(&self) -> &'static str {
            "ProbeCaret"
        }
    }

    impl EventElement for ProbeCaretElement {}
    impl LayoutElement for ProbeCaretElement {}
    impl Rebuildable for ProbeCaretElement {}

    /// The deepest first child of `element`: the leaf that records the paint
    /// when a widget is a chain of wrappers.
    fn innermost(element: &dyn Element) -> &dyn Element {
        let mut inner = None;
        element.visit_children(&mut |child| {
            if inner.is_none() {
                inner = Some(child);
            }
        });
        inner.map_or(element, innermost)
    }

    /// The color of the last rectangle `caret` records in its own list.
    fn recorded_fill_color(caret: &AnyElement, context: &BuildContext<'_>) -> Option<[u8; 4]> {
        let caret = innermost(caret.as_ref());
        let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
        let root = tree
            .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 240.0, 60.0))
            .unwrap();
        let node_context = tree.context(root).unwrap();
        context.with_local_v2_paint_context(node_context, |context| {
            assert!(caret.can_paint_local_v2(context));
            caret.paint_local_v2(context);
        });
        tree.draw_list_snapshot(root)
            .unwrap()
            .commands
            .iter()
            .rev()
            .find_map(|command| match command {
                aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { color, .. } => {
                    Some(color.to_rgba8().0)
                }
                _ => None,
            })
    }

    #[test]
    fn retained_builtin_caret_tracks_updated_cursor_color() {
        let slot = CaretSlot::new();
        let context = dummy_build_context(240.0, 60.0);
        let caret_context = CaretContext::new(CaretBlink::new());
        caret_context.publish(
            CaretGeometry {
                width: 1.5,
                height: 16.0,
                ..CaretGeometry::default()
            },
            0,
            true,
            true,
            false,
        );

        let caret = slot.build(
            None,
            caret_context.clone(),
            Colors::Red,
            &context,
        );
        assert_eq!(
            recorded_fill_color(&caret, &context),
            Some(Rgba8::from_argb(Colors::Red.as_u32()).0)
        );

        let caret = slot.build(
            None,
            caret_context,
            Colors::Blue,
            &context,
        );
        assert_eq!(
            recorded_fill_color(&caret, &context),
            Some(Rgba8::from_argb(Colors::Blue.as_u32()).0)
        );
    }

    #[test]
    fn host_positions_the_retained_caret_at_the_published_geometry() {
        let field = RawTextField::new(
            field_config(TextEditingController::new()),
            CaretBlink::new(),
            FocusNode::new(),
        );
        let _ = field.on_event(&ElementEvent::FocusGained);
        let caret_context = field.caret_context();
        let size = Rc::new(Cell::new((0.0, 0.0)));
        let context = dummy_build_context(240.0, 60.0);
        let caret = ProbeCaret(size.clone()).to_element(&context);
        let host = RawTextFieldHost::new(field, caret);

        host.update(&context);

        let (width, height) = size.get();
        assert_eq!(width, 1.5);
        assert!(height > 0.0);
        assert!(caret_context.is_focused());
        assert_eq!(caret_context.offset(), 0);
        assert_eq!(caret_context.geometry().x, 4.0);
        assert!(caret_context.geometry().y > 4.0);
        assert_eq!(caret_context.geometry().width, 1.5);
        assert!(caret_context.geometry().height > 0.0);
    }
}
