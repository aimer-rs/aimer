/// Where an embedded widget is drawn, in physical pixels relative to the field's
/// outer local origin, the same space the caret is placed in.
#[derive(Clone, Copy, Default, PartialEq)]
struct EmbedPlacement {
    offset: Vec2d,
    size: ResolvedSize,
    /// The part of the field the widget may draw in, as `(x, y, width, height)`: the box of the text,
    /// which is not the whole field (the margin of line numbers is beside it) and which a widget
    /// scrolled sideways or half out of the top would otherwise spill over.
    clip: (f32, f32, f32, f32),
}

/// A rectangle the field published for an embed, in logical pixels relative to
/// the field's outer local origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PublishedEmbed {
    pub key: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// The box of the text, `(x, y, width, height)`, in the same space.
    pub clip: (f32, f32, f32, f32),
}

impl PublishedEmbed {
    /// Whether the point `(x, y)` is on the part of the embed that shows: inside its box and inside
    /// the clip, which a text scrolled sideways leaves the box overhanging, over the margin beside it.
    fn covers(&self, x: f32, y: f32) -> bool {
        let (clip_x, clip_y, clip_width, clip_height) = self.clip;
        x >= self.x
            && x <= self.x + self.width
            && y >= self.y
            && y <= self.y + self.height
            && (clip_width <= 0.0
                || (x >= clip_x && x <= clip_x + clip_width && y >= clip_y && y <= clip_y + clip_height))
    }
}

/// Places an embedded widget at the rectangle its box was laid out at.
///
/// Like the caret's placement it is out of flow: the field decides where it
/// goes, and nothing about it takes part in the field's own layout. Unlike the
/// caret it is a hit-test target, so a click on the embedded widget reaches the
/// widget and not the text under it.
struct RawEmbedPlacement {
    child: Rc<AnyElement>,
    placement: Rc<Cell<EmbedPlacement>>,
}

impl VisitorElement for RawEmbedPlacement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref().as_ref());
    }
}

impl Element for RawEmbedPlacement {
    fn name(&self) -> &'static str {
        "TextFieldEmbedPlacement"
    }
}

impl EventElement for RawEmbedPlacement {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl Rebuildable for RawEmbedPlacement {
    /// Only hands a mark on to its child; see [`Rebuildable::forwards_marks_only`].
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

impl LayoutElement for RawEmbedPlacement {
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

impl Drawable for RawEmbedPlacement {
    fn update(&self, ctx: &BuildContext) {
        let placement = self.placement.get();
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = placement.size;
        child_ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: placement.size.width,
            max_height: placement.size.height,
        };
        child_ctx.set_visible_rect(ctx.visible_rect().map(|(x, y, width, height)| {
            (x - placement.offset.x, y - placement.offset.y, width, height)
        }));
        ctx.canvas.save();
        ctx.canvas.translate(placement.offset);
        self.child.update(&child_ctx);
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

struct EmbedChild {
    key: u64,
    /// The embedded widget's element, measured before it is placed.
    child: Rc<AnyElement>,
    wrapper: Box<AnyElement>,
    placement: Rc<Cell<EmbedPlacement>>,
}

/// The embedded widgets of one text field, kept for as long as the field's state
/// lives.
///
/// The field element is rebuilt with its widget, which would throw away every
/// embedded widget and the state in it, so the state owns this store and each
/// rebuilt element finds the same children in it. The children it holds change
/// while it stays in the tree, which is what
/// [`aimer_widget::notify_live_children_changed`] is for.
pub(crate) struct EmbedStore {
    /// Only ever borrowed from the single render thread, and never across a call
    /// that can resize the list: [`EmbedStore::ensure`] pushes and
    /// [`EmbedStore::sync`] retains, and the visitors only read.
    children: std::cell::UnsafeCell<Vec<EmbedChild>>,
    /// What the field last said about where its embeds are.
    published: RefCell<Vec<PublishedEmbed>>,
    /// Heights found by measuring, by embed key and measured width.
    measured: RefCell<std::collections::HashMap<(u64, u32), f32>>,
    /// Set while a press that landed on an embedded widget has not been released.
    pressed: Cell<bool>,
}

impl Default for EmbedStore {
    fn default() -> Self {
        Self {
            children: std::cell::UnsafeCell::new(Vec::new()),
            published: RefCell::new(Vec::new()),
            measured: RefCell::new(std::collections::HashMap::new()),
            pressed: Cell::new(false),
        }
    }
}

impl EmbedStore {
    #[inline]
    fn children(&self) -> &Vec<EmbedChild> {
        // A shared read of a list only `ensure` and `sync` mutate.
        unsafe { &*self.children.get() }
    }

    /// The number of embedded widgets alive.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.children().len()
    }

    /// The element of the embed `key`, built from `embed` if there is none yet.
    fn ensure(&self, embed: &TextEmbed, ctx: &BuildContext) -> Rc<AnyElement> {
        if let Some(child) = self.children().iter().find(|child| child.key == embed.key()) {
            return Rc::clone(&child.child);
        }
        let element = Rc::new(embed.build().to_element(ctx));
        let placement = Rc::new(Cell::new(EmbedPlacement::default()));
        let wrapper = Box::new(
            RawEmbedPlacement {
                child: Rc::clone(&element),
                placement: Rc::clone(&placement),
            }
            .boxed(),
        );
        // Not held across anything that can resize the list.
        unsafe { &mut *self.children.get() }.push(EmbedChild {
            key: embed.key(),
            child: Rc::clone(&element),
            wrapper,
            placement,
        });
        aimer_widget::notify_live_children_changed();
        element
    }

    /// The height, in physical pixels, the embed takes in a field `width` wide.
    ///
    /// A fixed height is its logical value at `ctx`'s scale. Otherwise the widget
    /// is measured the way a flex pass measures a child, with the main axis
    /// unbounded, and the answer is remembered for that width.
    pub(crate) fn height(&self, embed: &TextEmbed, width: f32, ctx: &BuildContext) -> f32 {
        let scale = if ctx.scale > 0.0 { ctx.scale } else { 1.0 };
        // Whatever decides the height, the widget has to exist to be placed.
        let child = self.ensure(embed, ctx);
        if let EmbedHeight::Fixed(height) = embed.height() {
            return if height.is_finite() { height.max(0.0) * scale } else { 0.0 };
        }
        let cache_key = (embed.key(), width.to_bits());
        if let Some(height) = self.measured.borrow().get(&cache_key) {
            return *height;
        }
        let mut measuring = ctx.clone();
        measuring.parent_size = ResolvedSize { width, height: f32::INFINITY };
        measuring.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: width,
            max_height: f32::INFINITY,
        };
        let height = child.computed_size(&measuring).height;
        let height = if height.is_finite() { height.max(0.0) } else { 0.0 };
        self.measured.borrow_mut().insert(cache_key, height);
        height
    }

    /// Records where the field laid its embeds out this frame.
    pub(crate) fn publish(&self, embeds: Vec<PublishedEmbed>) {
        *self.published.borrow_mut() = embeds;
    }

    /// Whether the field has published any embed, which is the only time the
    /// host has anything to place.
    #[inline]
    pub(crate) fn has_published(&self) -> bool {
        !self.published.borrow().is_empty()
    }

    /// Moves every embedded widget to its published rectangle and drops the ones
    /// the field no longer shows. Returns whether anything moved or went.
    pub(crate) fn sync(&self, scale: f32) -> bool {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let published = self.published.borrow();
        let mut changed = false;
        for child in self.children() {
            let placement = published
                .iter()
                .find(|embed| embed.key == child.key)
                .map(|embed| EmbedPlacement {
                    offset: Vec2d { x: embed.x * scale, y: embed.y * scale },
                    size: ResolvedSize { width: embed.width * scale, height: embed.height * scale },
                    clip: (embed.clip.0 * scale, embed.clip.1 * scale, embed.clip.2 * scale, embed.clip.3 * scale),
                })
                .unwrap_or_default();
            if child.placement.replace(placement) != placement {
                changed = true;
            }
        }
        let alive = self
            .children()
            .iter()
            .filter(|child| published.iter().any(|embed| embed.key == child.key))
            .count();
        if alive != self.children().len() {
            // No visitor is running: this is the update pass, which nothing
            // else's visit is part of.
            unsafe { &mut *self.children.get() }
                .retain(|child| published.iter().any(|embed| embed.key == child.key));
            self.measured
                .borrow_mut()
                .retain(|(key, _), _| published.iter().any(|embed| embed.key == *key));
            aimer_widget::notify_live_children_changed();
            changed = true;
        }
        changed
    }

    /// The embed under the point `(x, y)`, in logical pixels from the field's outer
    /// origin, by the rectangles the field published.
    pub(crate) fn published_at(&self, x: f32, y: f32) -> Option<PublishedEmbed> {
        self.published.borrow().iter().find(|embed| embed.covers(x, y)).copied()
    }

    /// Where the embed whose element is `child` is, for the render tree: its bounds, in logical
    /// pixels local to the field, and the clip to the box of the text, in logical pixels local to the
    /// embed. `None` for any other child.
    pub(crate) fn geometry_of(
        &self,
        child: &dyn Element,
        scale: f32,
    ) -> Option<(aimer_cupid::draw_cmd::Rect, aimer_cupid::draw_cmd::Rect)> {
        use aimer_cupid::draw_cmd::Rect;
        let placement = self.children().iter().find(|embed| embed.wrapper.id() == child.id())?.placement.get();
        if !(scale.is_finite() && scale > 0.0) {
            return None;
        }
        let (offset, size, clip) = (placement.offset, placement.size, placement.clip);
        let bounds = Rect::new(offset.x / scale, offset.y / scale, size.width / scale, size.height / scale);
        let clip = Rect::new((clip.0 - offset.x) / scale, (clip.1 - offset.y) / scale, clip.2 / scale, clip.3 / scale);
        Some((bounds, clip))
    }

    /// Notes that a press landed on an embedded widget, or that it was let go.
    #[inline]
    pub(crate) fn set_pressed(&self, pressed: bool) {
        self.pressed.set(pressed);
    }

    /// Takes the note of a press on an embedded widget: whether there was one.
    #[inline]
    pub(crate) fn take_pressed(&self) -> bool {
        self.pressed.replace(false)
    }

    /// Whether the point `(x, y)`, in logical pixels from the field's outer
    /// origin, is over an embedded widget, by the rectangles the field published.
    pub(crate) fn published_contains(&self, x: f32, y: f32) -> bool {
        self.published.borrow().iter().any(|embed| embed.covers(x, y))
    }

    /// Updates every embedded widget, translated to its place.
    pub(crate) fn update(&self, ctx: &BuildContext) {
        for child in self.children() {
            child.wrapper.update(ctx);
        }
    }

    pub(crate) fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in self.children() {
            visitor(child.wrapper.as_ref().as_ref());
        }
    }

    /// The clip of the embed `key`, for tests.
    #[cfg(test)]
    pub(crate) fn clip_of(&self, key: u64) -> Option<(f32, f32, f32, f32)> {
        self.children().iter().find(|child| child.key == key).map(|child| child.placement.get().clip)
    }

    /// The placement of the embed `key`, for tests.
    #[cfg(test)]
    pub(crate) fn placement_of(&self, key: u64) -> Option<(Vec2d, ResolvedSize)> {
        self.children()
            .iter()
            .find(|child| child.key == key)
            .map(|child| (child.placement.get().offset, child.placement.get().size))
    }
}
