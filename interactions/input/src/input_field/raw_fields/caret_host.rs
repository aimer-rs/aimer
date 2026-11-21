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
    /// The field's embedded widgets. They are retained children like the caret,
    /// and live in the same slot for the same reason: the state owns them, and
    /// every element the field is rebuilt into finds them here.
    pub(crate) embeds: Rc<EmbedStore>,
}

impl CaretSlot {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            child: RefCell::new(None),
            custom: Cell::new(None),
            builtin_color: Rc::new(Cell::new(Color::default())),
            embeds: Rc::new(EmbedStore::default()),
        }
    }

    #[inline]
    pub(crate) fn build(
        &self,
        builder: Option<&CaretBuilder>,
        context: CaretContext,
        color: impl Into<Color>,
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
    /// Moves the drawn caret to where the cursor went; see [`CaretGlide`].
    glide: RefCell<CaretGlide>,
    smooth: bool,
    /// The identity the walk gave this element the last time it updated: an element is not handed its
    /// own, and the synchronization of paint state, which also places the caret and the embeds, runs
    /// outside the walk of the element.
    walked_as: Cell<Option<aimer_widget::ElementId>>,
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
            glide: RefCell::new(CaretGlide::default()),
            smooth: true,
            walked_as: Cell::new(None),
        }
    }

    /// Chooses whether the caret glides to a moved cursor or jumps to it.
    #[inline]
    pub(crate) fn smooth(mut self, smooth: bool) -> Self {
        self.smooth = smooth;
        self
    }

    fn sync_caret_placement(&self, ctx: &BuildContext) {
        let context = self.field.caret_context();
        let geometry = context.geometry();
        let scale = if ctx.scale > 0.0 { ctx.scale } else { 1.0 };
        let available = context.is_focused() && context.is_available();
        let placement = if available {
            let mut rect = CaretRect {
                x: geometry.x,
                y: geometry.y,
                width: geometry.width,
                height: geometry.height,
            };
            if self.smooth {
                let (drawn, moving) =
                    self.glide
                        .borrow_mut()
                        .advance(rect, context.offset(), self.field.now());
                if moving {
                    // The caret is on its way: nothing else changes while it
                    // travels, so it has to ask for the frames it moves on. A frame
                    // alone does not re-run a field nothing marked dirty, so the
                    // caret marks itself, as a blink toggle does.
                    context.request_repaint();
                    aimer_events::window::request_animation_frame();
                }
                rect = drawn;
            }
            CaretPlacement {
                offset: Vec2d {
                    x: rect.x * scale,
                    y: rect.y * scale,
                },
                size: ResolvedSize {
                    width: rect.width * scale,
                    height: rect.height * scale,
                },
            }
        } else {
            // A caret that was not shown has no place to travel from.
            self.glide.borrow_mut().reset();
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
            // Nor is any element added or dropped: this host changed in place, and
            // the render tree revisits it alone, not the window from its root.
            self.notify_changed_in_place();
        }
    }
}

impl RawTextFieldHost {
    /// Learns who the walk in progress is, which is this element when it is updated.
    #[inline]
    fn learn_identity(&self) {
        if let Some(owner) = aimer_widget::draw_invalidation_owner() {
            self.walked_as.set(Some(owner));
        }
    }

    /// Says that where this element's own children are changed, and that no child was added, dropped
    /// or replaced: the render tree revisits this subtree alone. Before the element has been walked
    /// there is no one to name, and it is said as a change of the structure, which has the whole window
    /// collected again.
    fn notify_changed_in_place(&self) {
        match self.walked_as.get() {
            Some(id) => aimer_widget::notify_subtree_changed_in_place(id),
            None => aimer_widget::notify_retained_render_structure_changed(),
        }
    }

    /// Moves the embedded widgets to where the field laid their boxes out, and
    /// drops the ones it no longer shows.
    fn sync_embed_placement(&self, ctx: &BuildContext) {
        let scale = if ctx.scale > 0.0 { ctx.scale } else { 1.0 };
        if self.field.embeds.sync(scale) {
            // The render tree reads each placement when it synchronises, and a
            // synchronisation is skipped while nothing it depends on has moved,
            // as for the caret. An embed that moved or went is a change of this
            // host alone; the ones that went also said that its children changed
            // (see `EmbedStore::sync`), which is the change of the structure.
            self.notify_changed_in_place();
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
}

impl Element for RawCaretPlacement {
    fn name(&self) -> &'static str {
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
        caret_ctx.set_visible_rect(ctx.visible_rect().map(|(x, y, width, height)| {
            (
                x - placement.offset.x,
                y - placement.offset.y,
                width,
                height,
            )
        }));
        ctx.canvas.save();
        ctx.canvas.translate(placement.offset);
        self.child.update(&caret_ctx);
        ctx.canvas.restore();
    }

    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint(&self, _ctx: &BuildContext) {}

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

}

impl Drawable for RawTextFieldHost {
    fn update(&self, ctx: &BuildContext) {
        self.learn_identity();
        self.field.update(ctx);
        self.sync_caret_placement(ctx);
        self.sync_embed_placement(ctx);
        // The caret is drawn last, over the embeds: what hangs from it, a completion menu, reaches over
        // the lines below the caret, and a rendered block there must not be drawn over it.
        self.field.embeds.update(ctx);
        self.caret.update(ctx);
    }

    fn sync_paint_state(&self, ctx: &BuildContext) -> bool {
        if !self.field.sync_paint_state(ctx) {
            return false;
        }
        self.sync_caret_placement(ctx);
        self.sync_embed_placement(ctx);
        true
    }

    fn update_retained(&self, ctx: &BuildContext) {
        self.learn_identity();
        self.field.update_retained(ctx);
        self.sync_caret_placement(ctx);
        self.sync_embed_placement(ctx);
        // The caret is drawn last, over the embeds: what hangs from it, a completion menu, reaches over
        // the lines below the caret, and a rendered block there must not be drawn over it.
        self.field.embeds.update(ctx);
        self.caret.update(ctx);
    }

    fn paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        self.field.paint_outsets(ctx)
    }

    /// An embedded widget is clipped to the box of the text: it moves with the text when the field
    /// scrolls, and must not draw over the margin of line numbers or beyond the field.
    fn child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
    ) -> Option<(aimer_cupid::draw_cmd::Rect, Option<aimer_cupid::draw_cmd::Rect>)> {
        let (bounds, clip) = self.field.embeds.geometry_of(child, ctx.scale)?;
        Some((bounds, Some(clip)))
    }

    fn paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.field.paint_needs_recording(ctx)
    }

    fn can_paint(&self, ctx: &BuildContext) -> bool {
        self.field.can_paint(ctx)
    }

    fn paint(&self, ctx: &BuildContext) {
        self.field.paint(ctx);
    }

    fn interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        self.field.interaction_size(ctx)
    }

    fn interaction_offset(&self, ctx: &BuildContext) -> (f32, f32) {
        self.field.interaction_offset(ctx)
    }

    fn adopt_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.field.adopt_interaction_source(source);
    }

    fn interaction_disagreement(&self) -> Option<aimer_widget::InteractionDisagreement> {
        self.field.interaction_disagreement()
    }
}

impl VisitorElement for RawTextFieldHost {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // Back to front, as they are drawn: the caret, and what hangs from it, is over the embeds.
        self.field.embeds.visit(visitor);
        visitor(&self.caret);
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // The field is embedded and has no retained ElementId of its own.
        // Its callbacks are forwarded directly by this host, so it must not be
        // registered as a separate indexed event target. The embedded widgets
        // are real children: a press on one belongs to it.
        self.field.embeds.visit(visitor);
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.field);
        self.field.embeds.visit(visitor);
        visitor(&self.caret);
    }

    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.field);
    }

    fn hit_test_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // The caret is not a target; the embedded widgets are.
        self.field.embeds.visit(visitor);
    }
}

impl Element for RawTextFieldHost {
    fn name(&self) -> &'static str {
        "TextField"
    }
}

impl EventElement for RawTextFieldHost {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedHitTestBoundary
    }

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
        // A press or release over an embedded widget belongs to the widget. It has
        // had its turn by the time the event gets here, because children go first,
        // and a widget that acts on the tap, such as a copy button, does not
        // consume the press. Letting it fall through would put the cursor in the
        // block, which swaps the widget for its source before the tap completes.
        //
        // A click the widget has no use for is the user asking to edit what it
        // shows: a widget that handles its own tap, such as the copy button,
        // consumes the release, and one that does not lets the click through to
        // here, where it moves the cursor into the block.
        match event {
            // An icon in the margin is a button: it does its work on the press, and neither the press nor
            // its release reaches the field, so the caret stays where it is.
            ElementEvent::PointerDown(info) if let Some(press) = self.field.gutter_marker_at(info.pos) => {
                self.field.marker_pressed.set(true);
                press();
                return EventResult::consumed().with_redraw();
            }
            ElementEvent::PointerUp(_) if self.field.marker_pressed.replace(false) => {
                return EventResult::consumed();
            }
            // What hangs from the caret, a menu, is pressed by its own widgets: the press does not go to the field,
            // which would move the caret and take the menu away before the release of the click arrives.
            ElementEvent::PointerDown(info) if self.field.press_popup(info.pos) => {
                self.field.popup_pressed.set(true);
                return EventResult::consumed();
            }
            ElementEvent::PointerUp(_) if self.field.popup_pressed.replace(false) => {
                return EventResult::consumed();
            }
            ElementEvent::PointerDown(info) if self.field.embed_at(info.pos) => {
                self.field.embeds.set_pressed(true);
                return EventResult::consumed();
            }
            // A press anywhere else is a new gesture. A widget that handles its own tap, a button, consumes
            // the release, so the host never saw the release of the press it noted; left standing, that
            // note would take the release of this press, and the field, which never saw it, would believe
            // the button still down and extend a selection wherever the pointer went next.
            ElementEvent::PointerDown(_) => {
                self.field.embeds.set_pressed(false);
            }
            // Only a release that ends a press made on an embedded widget is the
            // host's. Any other release, even one that happens to land over an
            // embed, ends a press the text field took, and must reach it: a field
            // that never sees its release believes the button is still down and
            // extends a selection wherever the pointer goes next.
            ElementEvent::PointerUp(info) if self.field.embeds.take_pressed() => {
                if self.field.embed_at(info.pos) {
                    self.field.edit_embed_at(info.pos);
                }
                return EventResult::consumed();
            }
            _ => {}
        }
        // `field` is embedded and therefore has no retained ElementId. The
        // host is the indexed event target and owns pointer capture for its
        // editing logic, so forward directly instead of indexing it as a
        // separate child target.
        self.field.on_event(event)
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

    /// Lets a rebuilt host find this one to take its state over; without it
    /// [`adopt_runtime_state_from`](Rebuildable::adopt_runtime_state_from) cannot
    /// tell what it was offered and carries nothing.
    #[inline]
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|value| value.downcast_ref::<Self>())
        else {
            return;
        };
        self.field.adopt_runtime_state_from(&old.field);
        // The caret is mid-motion, or about to be, across a rebuild: typing reports
        // the change to the app, which rebuilds the field. A new glide would have
        // nowhere to travel from and would draw the caret in place.
        *self.glide.borrow_mut() = std::mem::take(&mut *old.glide.borrow_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimer_cupid::utilities::Rgba8;
    use crate::input_field::caret::{CaretBlink, CaretGeometry};
    use crate::input_field::raw_fields::test_support::{dummy_build_context, field_config};

    struct ProbeCaret(Rc<Cell<(f32, f32)>>);

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

    impl VisitorElement for ProbeCaretElement {}

    impl Element for ProbeCaretElement {
        fn name(&self) -> &'static str {
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
        let tree = aimer_cupid::draw_cmd::RenderTree::new();
        let root = tree
            .add_root(aimer_cupid::draw_cmd::Rect::new(0.0, 0.0, 240.0, 60.0))
            .unwrap();
        let node_context = tree.context(root).unwrap();
        context.with_local_v2_paint_context(node_context, |context| {
            assert!(caret.can_paint(context));
            caret.paint(context);
        });
        tree.draw_list_snapshot(root)
            .unwrap()
            .commands
            .iter()
            .rev()
            .find_map(|command| match command {
                aimer_cupid::draw_cmd::DrawCommand::FillRect { color, .. } => {
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

    #[test]
    fn a_caret_with_no_configured_colour_follows_the_text_colour() {
        let text = Color::Rgba(236, 238, 242, 255);
        let mut config = field_config(crate::input_field::controller::TextFieldController::new());
        config.text_style = config.text_style.color(text);
        config.cursor_color = None;

        // Text is always drawn in a colour that reads against its background,
        // so a caret that follows it does too: light on dark, dark on light.
        assert_eq!(config.caret_color(), text);

        config.cursor_color = Some(Colors::Red);
        assert_eq!(config.caret_color(), Colors::Red.into());
    }

    /// A host over a focused field holding `hello world`, with the field's clock
    /// fixed at `start` so the test steps time itself.
    fn glide_host(start: aimer_animation::AnimInstant) -> (RawTextFieldHost, BuildContext<'static>) {
        let field = RawTextField::new(
            field_config(TextEditingController::with_text("hello world")),
            CaretBlink::new(),
            FocusNode::new(),
        );
        let _ = field.on_event(&ElementEvent::FocusGained);
        field.test_clock.set(Some(start));
        let context = dummy_build_context(240.0, 60.0);
        let caret = ProbeCaret(Rc::new(Cell::new((0.0, 0.0)))).to_element(&context);
        (RawTextFieldHost::new(field, caret), context)
    }

    #[test]
    fn the_host_glides_the_caret_to_a_moved_cursor() {
        let start = aimer_animation::AnimInstant::now();
        let (host, context) = glide_host(start);
        host.field.cursor.set_offset(0);
        host.update(&context);
        let from = host.caret_placement.get().offset.x;

        host.field.cursor.set_offset(5);
        host.update(&context);
        assert_eq!(host.caret_placement.get().offset.x, from, "it leaves from where it was");

        host.field.test_clock.set(Some(start + CaretGlide::DURATION / 2));
        host.update(&context);
        let midway = host.caret_placement.get().offset.x;

        host.field.test_clock.set(Some(start + CaretGlide::DURATION * 2));
        host.update(&context);
        let arrived = host.caret_placement.get().offset.x;

        assert!(arrived > from + 10.0, "the cursor moved right: {from} -> {arrived}");
        assert!(midway > from && midway < arrived, "{from} < {midway} < {arrived}");
        assert_eq!(arrived, host.field.caret_context().geometry().x, "it ends on the published caret");
    }

    /// Typing moves the caret as surely as an arrow key does, so it travels too:
    /// the caret leaves from where it was and arrives at the glyph's far side.
    #[test]
    fn the_host_glides_the_caret_while_typing() {
        let start = aimer_animation::AnimInstant::now();
        let (host, context) = glide_host(start);
        host.field.cursor.set_offset(11);
        host.update(&context);
        let from = host.caret_placement.get().offset.x;

        assert!(host.field.insert_text("W"));
        host.update(&context);
        assert_eq!(host.caret_placement.get().offset.x, from, "it leaves from where it was");

        host.field.test_clock.set(Some(start + CaretGlide::DURATION / 2));
        host.update(&context);
        let midway = host.caret_placement.get().offset.x;

        host.field.test_clock.set(Some(start + CaretGlide::DURATION * 2));
        host.update(&context);
        let arrived = host.caret_placement.get().offset.x;

        assert!(arrived > from + 3.0, "the caret moved past the typed letter: {from} -> {arrived}");
        assert!(midway > from && midway < arrived, "{from} < {midway} < {arrived}");
    }

    /// A frame that follows an edit synchronises the host before it draws it, and
    /// the field has just started laying out: its caret is unavailable until it is
    /// painted. That passing state must not make the caret forget where it was, or
    /// every typed letter appears in place instead of gliding.
    #[test]
    fn typing_glides_when_the_frame_syncs_before_the_caret_is_published() {
        let start = aimer_animation::AnimInstant::now();
        let (host, context) = glide_host(start);
        host.field.cursor.set_offset(11);
        host.sync_paint_state(&context);
        host.update(&context);
        let from = host.caret_placement.get().offset.x;

        assert!(host.field.insert_text("W"));
        host.sync_paint_state(&context);
        host.update(&context);
        assert_eq!(host.caret_placement.get().offset.x, from, "it leaves from where it was");

        host.field.test_clock.set(Some(start + CaretGlide::DURATION / 2));
        host.sync_paint_state(&context);
        host.update(&context);
        let midway = host.caret_placement.get().offset.x;

        host.field.test_clock.set(Some(start + CaretGlide::DURATION * 2));
        host.sync_paint_state(&context);
        host.update(&context);
        let arrived = host.caret_placement.get().offset.x;

        assert!(arrived > from + 3.0, "the caret moved past the typed letter: {from} -> {arrived}");
        assert!(midway > from && midway < arrived, "{from} < {midway} < {arrived}");
    }

    /// Typing reports the change to the app, which rebuilds the field, so a typed
    /// letter arrives at a new host that has to carry on the old one's caret. A host
    /// that starts its glide afresh has nowhere to glide from and draws the caret in
    /// place, which is why typing did not glide while arrow keys, which rebuild
    /// nothing, did.
    #[test]
    fn a_rebuilt_host_carries_the_caret_glide_on() {
        let start = aimer_animation::AnimInstant::now();
        let controller = TextEditingController::with_text("hello world");
        let make = || {
            let field = RawTextField::new(field_config(controller.clone()), CaretBlink::new(), FocusNode::new());
            let _ = field.on_event(&ElementEvent::FocusGained);
            field.test_clock.set(Some(start));
            let context = dummy_build_context(240.0, 60.0);
            let caret = ProbeCaret(Rc::new(Cell::new((0.0, 0.0)))).to_element(&context);
            (RawTextFieldHost::new(field, caret), context)
        };
        let (old, context) = make();
        old.field.cursor.set_offset(11);
        old.update(&context);
        let from = old.caret_placement.get().offset.x;
        assert!(from > 10.0, "the old host drew the caret at the end of the text: {from}");

        // The rebuild: a new host takes over from the old, then the letter arrives.
        let (new, context) = make();
        new.adopt_runtime_state_from(&old);
        assert!(new.field.insert_text("W"));
        new.update(&context);
        assert_eq!(new.caret_placement.get().offset.x, from, "the caret leaves from where it was");

        new.field.test_clock.set(Some(start + CaretGlide::DURATION * 2));
        new.update(&context);
        assert!(new.caret_placement.get().offset.x > from + 3.0, "and arrives past the new letter");
    }

    #[test]
    fn a_host_that_does_not_smooth_the_caret_jumps() {
        let start = aimer_animation::AnimInstant::now();
        let (host, context) = glide_host(start);
        let host = host.smooth(false);
        host.field.cursor.set_offset(0);
        host.update(&context);
        let from = host.caret_placement.get().offset.x;

        host.field.cursor.set_offset(5);
        host.update(&context);

        assert!(host.caret_placement.get().offset.x > from + 10.0);
    }

    /// A frame asked for with nothing marked dirty does not re-run the field, so a
    /// glide that only asked for frames would stop where the last event-driven
    /// frame left it. The caret has to mark itself dirty, as a blink toggle does,
    /// for every frame it still has to move in.
    #[test]
    fn a_moving_caret_asks_for_a_repaint_of_itself_until_it_arrives() {
        let start = aimer_animation::AnimInstant::now();
        let (host, context) = glide_host(start);
        let repaints = Rc::new(Cell::new(0_u32));
        let counted = Rc::clone(&repaints);
        host.field.caret_context().set_repaint_callback(Rc::new(move || counted.set(counted.get() + 1)));
        host.field.cursor.set_offset(0);
        host.update(&context);

        host.field.cursor.set_offset(5);
        host.update(&context);
        host.field.test_clock.set(Some(start + CaretGlide::DURATION / 2));
        let before = repaints.get();
        host.update(&context);
        assert!(repaints.get() > before, "still moving, so it asks for another frame of itself");

        host.field.test_clock.set(Some(start + CaretGlide::DURATION * 2));
        host.update(&context);
        let arrived = repaints.get();
        host.update(&context);
        assert_eq!(repaints.get(), arrived, "and stops asking once it has arrived");
    }
}
