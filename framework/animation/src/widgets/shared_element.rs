//! Names an element so a transition can find where it is on screen.
//!
//! A container transform grows a page out of the item that opened it. The page
//! and the item live in different subtrees, so they meet through a tag: wrap
//! the item in a [`SharedElement`] and the transition asks the registry here
//! where that tag currently is.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use aimer_attribute::{ResolvedSize, Size, Vec2d};
use aimer_canvas::Canvas;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventTreeRole, Key, LayoutElement,
    PortableWidget, Rebuildable, RequiredChild, RetainedNodeHandle, VisitorElement, Widget,
};

use super::container_transform::ElementBounds;

struct Registration {
    /// Which element registered, so that a replaced element dropping later does
    /// not remove the registration that replaced it.
    owner: u64,
    node: RetainedNodeHandle,
    content: Option<SharedContent>,
}

/// Builds the tagged item's widget again, so a transition can draw a copy of
/// the item above everything else.
///
/// Only a [`SharedElement`] given its child with
/// [`child_with`](SharedElement::child_with) has one. Each call to
/// [`build`](Self::build) produces an independent widget, which becomes an
/// independent element: the copy never shares state with the real item.
#[derive(Clone)]
pub struct SharedContent(Rc<dyn Fn() -> AnyWidget>);

impl SharedContent {
    /// Describes an item by a function that builds it.
    ///
    /// For code that draws copies of something without a [`SharedElement`],
    /// such as a custom transition.
    pub fn new(build: impl Fn() -> AnyWidget + 'static) -> Self {
        Self(Rc::new(build))
    }

    /// Builds a new copy of the item's widget.
    pub fn build(&self) -> AnyWidget {
        (self.0)()
    }
}

impl std::fmt::Debug for SharedContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedContent(..)")
    }
}

/// Where a tagged element is, and how to draw it again.
#[derive(Clone, Debug)]
pub struct SharedElementSnapshot {
    /// Where the element is, or where it was last seen.
    pub bounds: ElementBounds,
    /// How to build another copy, if the element was given a repeatable child.
    pub content: Option<SharedContent>,
}

/// How many dropped tags keep their last position. Going back needs only the
/// item that was opened, so a handful is plenty and the memory stays bounded.
const REMEMBERED_LIMIT: usize = 32;

aimer_tls::thread_local! {
    static REGISTRY: RefCell<HashMap<Key, Registration>> = RefCell::new(HashMap::new());
    /// Last known bounds and content of recently dropped tags, oldest first.
    static REMEMBERED: RefCell<Vec<(Key, SharedElementSnapshot)>> = const { RefCell::new(Vec::new()) };
    static NEXT_OWNER: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn next_owner() -> u64 {
    NEXT_OWNER.with(|next| {
        let owner = next.get();
        next.set(owner + 1);
        owner
    })
}

/// Returns where the element tagged `tag` is on screen right now.
///
/// The bounds are in logical pixels from the window's top-left corner and are
/// read when asked, so they follow scrolling and layout since the element was
/// last synchronized. `None` means nothing is tagged `tag`, the tagged element
/// has left the tree, or it has no visible area.
///
/// Rendering is single-threaded, so the registry belongs to the thread that
/// renders. If two live elements share a tag, the most recently synchronized
/// one answers.
pub fn shared_element_bounds(tag: &Key) -> Option<ElementBounds> {
    let node = REGISTRY.with(|registry| registry.borrow().get(tag).map(|entry| entry.node.clone()))?;
    let (x, y, width, height) = node.world_bounds()?;
    let bounds = ElementBounds::new(x, y, width, height);
    bounds.is_valid().then_some(bounds)
}

/// Like [`shared_element_bounds`], but when the tagged element has been
/// dropped, answers with where it was last seen.
///
/// Going back from a page resolves its transition before the page that held
/// the item has been rebuilt, so the live registry has nothing for the tag yet.
/// Only the most recently dropped tags are remembered.
pub fn last_shared_element_bounds(tag: &Key) -> Option<ElementBounds> {
    last_shared_element(tag).map(|snapshot| snapshot.bounds)
}

/// Like [`last_shared_element_bounds`], and also returns how to draw the
/// element again when it was given a repeatable child.
///
/// A transition that draws a copy of the item needs this, because going back it
/// resolves before the grid has rebuilt the tile it is shrinking into.
pub fn last_shared_element(tag: &Key) -> Option<SharedElementSnapshot> {
    let live = REGISTRY.with(|registry| {
        registry.borrow().get(tag).map(|entry| (entry.node.clone(), entry.content.clone()))
    });
    if let Some((node, content)) = live {
        // A registration whose node can no longer say where it is (it left the
        // render tree before its element was dropped, or has no area) is not an
        // answer: go on to where the tag was last seen.
        let bounds = node
            .world_bounds()
            .map(|(x, y, width, height)| ElementBounds::new(x, y, width, height))
            .filter(ElementBounds::is_valid);
        if let Some(bounds) = bounds {
            return Some(SharedElementSnapshot { bounds, content });
        }
    }
    REMEMBERED.with(|remembered| {
        remembered
            .borrow()
            .iter()
            .find_map(|(key, snapshot)| (key == tag).then(|| snapshot.clone()))
    })
}

/// Tags its child so a transition can find where the child is on screen.
///
/// It changes nothing about layout, painting or input. Give the item in a grid
/// and nothing else the tag the destination's transition asks for; see
/// [`shared_element_bounds`].
///
/// # Examples
///
/// ```
/// use aimer_animation::SharedElement;
/// use aimer_widget::ErrorWidget;
///
/// let cover = SharedElement::new()
///     .tag("album-42")
///     .child(ErrorWidget::new("cover art"));
/// ```
pub struct SharedElement<W = RequiredChild> {
    tag: Option<Key>,
    content: Option<SharedContent>,
    child: W,
}

impl Default for SharedElement {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedElement {
    /// Creates an untagged builder that requires a child.
    ///
    /// Finish it with [`SharedElement::child`] or [`SharedElement::box_child`].
    /// Without a [`tag`](SharedElement::tag) it is inert.
    #[inline]
    pub fn new() -> Self {
        Self {
            tag: None,
            content: None,
            child: RequiredChild,
        }
    }
}

impl<W> SharedElement<W> {
    /// Sets the tag the transition looks this element up by.
    #[inline]
    pub fn tag(mut self, tag: impl Into<Key>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Attaches the required child and completes this builder.
    #[inline]
    pub fn child<C: Widget>(self, child: C) -> SharedElement<C> {
        SharedElement {
            tag: self.tag,
            content: None,
            child,
        }
    }

    /// Attaches a child that can be built again, and completes this builder.
    ///
    /// A transition can then draw a copy of the item above the pages, scaled,
    /// which is how a container transform leads with the item itself. `build`
    /// runs once for the real child and again for every copy, so it should
    /// produce the same item each time. The copy is inert: it takes no input.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_animation::SharedElement;
    /// use aimer_widget::ErrorWidget;
    ///
    /// let cover = SharedElement::new()
    ///     .tag("album-42")
    ///     .child_with(|| ErrorWidget::new("cover art"));
    /// ```
    pub fn child_with<C: Widget + 'static>(self, build: impl Fn() -> C + 'static) -> SharedElement<Repeated> {
        let content = SharedContent::new(move || build().boxed());
        SharedElement {
            tag: self.tag,
            content: Some(content.clone()),
            child: Repeated(content),
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

/// The child of a [`SharedElement`] attached with
/// [`child_with`](SharedElement::child_with).
#[doc(hidden)]
pub struct Repeated(SharedContent);

impl PortableWidget for Repeated {}

impl Widget for Repeated {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        self.0.build().to_element(ctx)
    }
}

impl<W: Widget> PortableWidget for SharedElement<W> {}

impl<W: Widget> Widget for SharedElement<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawSharedElement {
            tag: self.tag,
            content: self.content,
            child: self.child.to_element(ctx),
            owner: next_owner(),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "SharedElement"
    }
}

struct RawSharedElement {
    tag: Option<Key>,
    content: Option<SharedContent>,
    child: AnyElement,
    owner: u64,
}

impl Drop for RawSharedElement {
    fn drop(&mut self) {
        let Some(tag) = &self.tag else { return };
        let removed = REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            if registry.get(tag).is_some_and(|entry| entry.owner == self.owner) {
                registry.remove(tag)
            } else {
                None
            }
        });
        let last = removed.and_then(|entry| {
            let (x, y, width, height) = entry.node.world_bounds()?;
            let bounds = ElementBounds::new(x, y, width, height);
            bounds.is_valid().then_some(SharedElementSnapshot { bounds, content: entry.content })
        });
        if let Some(last) = last {
            REMEMBERED.with(|remembered| {
                let mut remembered = remembered.borrow_mut();
                remembered.retain(|(key, _)| key != tag);
                if remembered.len() == REMEMBERED_LIMIT {
                    remembered.remove(0);
                }
                remembered.push((tag.clone(), last));
            });
        }
    }
}

impl Rebuildable for RawSharedElement {}

impl Drawable for RawSharedElement {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

    /// The element owns a render node of its own, with nothing recorded on it,
    /// so there is a node whose bounds are the child's.
    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        Canvas::of(ctx).finish();
    }

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        if let (Some(tag), Some(node)) = (&self.tag, ctx.local_v2_node_handle()) {
            REGISTRY.with(|registry| {
                registry.borrow_mut().insert(
                    tag.clone(),
                    Registration {
                        owner: self.owner,
                        node,
                        content: self.content.clone(),
                    },
                );
            });
        }
        true
    }
}

impl EventElement for RawSharedElement {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }

    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl VisitorElement for RawSharedElement {
    #[inline]
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "SharedElement"
    }
}

impl LayoutElement for RawSharedElement {
    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }

    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::OnceLock;

    use aimer_attribute::BoxConstraint;
    use aimer_cupid::draw_cmd_v2::{Rect, RenderNodeId, RenderTree};

    use super::*;

    fn context() -> BuildContext<'static> {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("shared element test runtime should build")
        });
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        let mut context = BuildContext::new(
            aimer_canvas::FrameCanvas::new(inner),
            ResolvedSize {
                width: 101.0,
                height: 101.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            aimer_widget::base::WindowHandle::headless(Default::default(), 1.0),
            runtime.handle().clone(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 101.0,
            max_height: 101.0,
        };
        context
    }

    struct Leaf;

    impl Widget for Leaf {
        fn to_element(self, _context: &BuildContext) -> AnyElement {
            LeafElement.boxed()
        }
    }

    impl PortableWidget for Leaf {}

    struct LeafElement;

    impl Drawable for LeafElement {
        fn update(&self, _context: &BuildContext) {}
    }

    impl EventElement for LeafElement {}

    impl LayoutElement for LeafElement {
        fn computed_size(&self, _context: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: 10.0,
                height: 10.0,
            }
        }
    }

    impl Rebuildable for LeafElement {}

    impl VisitorElement for LeafElement {
        fn debug_name(&self) -> &'static str {
            "Leaf"
        }
    }

    /// A render node standing in for the one the element owns.
    struct Node {
        tree: RenderTree,
        id: RenderNodeId,
    }

    impl Node {
        fn at(bounds: Rect) -> Self {
            let tree = RenderTree::new();
            let id = tree.add_root(bounds).unwrap();
            Self { tree, id }
        }

        /// Runs the element's state synchronization against this node.
        fn sync(&self, element: &dyn Element, context: &BuildContext<'_>) -> bool {
            let retained = self.tree.context(self.id).unwrap();
            context.with_local_v2_paint_context(retained, |context| {
                element.sync_local_v2_state(context)
            })
        }
    }

    fn tagged(tag: &str, context: &BuildContext) -> AnyElement {
        SharedElement::new().tag(tag).child(Leaf).to_element(context)
    }

    fn bounds(tag: &str) -> Option<ElementBounds> {
        shared_element_bounds(&Key::from(tag))
    }

    fn last_bounds(tag: &str) -> Option<ElementBounds> {
        last_shared_element_bounds(&Key::from(tag))
    }

    #[test]
    fn a_synchronized_element_reports_its_world_bounds() {
        let context = context();
        let element = tagged("report", &context);
        let node = Node::at(Rect::new(30.0, 40.0, 50.0, 60.0));

        assert!(node.sync(element.as_ref(), &context));

        assert_eq!(bounds("report"), Some(ElementBounds::new(30.0, 40.0, 50.0, 60.0)));
    }

    #[test]
    fn the_bounds_follow_the_node_without_another_sync() {
        let context = context();
        let element = tagged("follow", &context);
        let node = Node::at(Rect::new(30.0, 40.0, 50.0, 60.0));
        node.sync(element.as_ref(), &context);

        // A scroll moves the node; nothing synchronizes the element again.
        node.tree.set_bounds(node.id, Rect::new(30.0, -60.0, 50.0, 60.0)).unwrap();

        assert_eq!(bounds("follow"), Some(ElementBounds::new(30.0, -60.0, 50.0, 60.0)));
    }

    #[test]
    fn an_unknown_tag_has_no_bounds() {
        assert_eq!(bounds("never-registered"), None);
    }

    #[test]
    fn an_untagged_element_registers_nothing() {
        let context = context();
        let element = SharedElement::new().child(Leaf).to_element(&context);
        let node = Node::at(Rect::new(0.0, 0.0, 10.0, 10.0));

        assert!(node.sync(element.as_ref(), &context));

        assert_eq!(bounds("untagged"), None);
    }

    #[test]
    fn dropping_the_element_unregisters_its_tag() {
        let context = context();
        let element = tagged("dropped", &context);
        let node = Node::at(Rect::new(1.0, 2.0, 3.0, 4.0));
        node.sync(element.as_ref(), &context);
        assert!(bounds("dropped").is_some());

        drop(element);

        assert_eq!(bounds("dropped"), None);
    }

    #[test]
    fn the_most_recent_element_wins_and_the_older_one_cannot_remove_it() {
        let context = context();
        let older = tagged("shared", &context);
        let newer = tagged("shared", &context);
        let old_node = Node::at(Rect::new(0.0, 0.0, 10.0, 10.0));
        let new_node = Node::at(Rect::new(5.0, 5.0, 20.0, 20.0));
        old_node.sync(older.as_ref(), &context);
        new_node.sync(newer.as_ref(), &context);

        assert_eq!(bounds("shared"), Some(ElementBounds::new(5.0, 5.0, 20.0, 20.0)));
        drop(older);

        assert_eq!(bounds("shared"), Some(ElementBounds::new(5.0, 5.0, 20.0, 20.0)));
    }

    #[test]
    fn an_element_without_area_has_no_bounds() {
        let context = context();
        let element = tagged("empty", &context);
        let node = Node::at(Rect::new(10.0, 10.0, 0.0, 0.0));
        node.sync(element.as_ref(), &context);

        assert_eq!(bounds("empty"), None);
    }

    #[test]
    fn a_dropped_element_is_remembered_for_the_way_back() {
        let context = context();
        let element = tagged("remembered", &context);
        let node = Node::at(Rect::new(8.0, 9.0, 30.0, 40.0));
        node.sync(element.as_ref(), &context);

        drop(element);

        // Going back resolves before the page that held the item is rebuilt.
        assert_eq!(bounds("remembered"), None);
        assert_eq!(
            last_bounds("remembered"),
            Some(ElementBounds::new(8.0, 9.0, 30.0, 40.0))
        );
    }

    #[test]
    fn live_bounds_win_over_remembered_ones() {
        let context = context();
        let first = tagged("replaced", &context);
        let first_node = Node::at(Rect::new(1.0, 1.0, 10.0, 10.0));
        first_node.sync(first.as_ref(), &context);
        drop(first);

        let second = tagged("replaced", &context);
        let second_node = Node::at(Rect::new(50.0, 60.0, 10.0, 10.0));
        second_node.sync(second.as_ref(), &context);

        assert_eq!(
            last_bounds("replaced"),
            Some(ElementBounds::new(50.0, 60.0, 10.0, 10.0))
        );
    }

    #[test]
    fn only_the_most_recently_dropped_tags_are_remembered() {
        let context = context();
        for index in 0..(REMEMBERED_LIMIT + 8) {
            let tag = format!("flood-{index}");
            let element = tagged(&tag, &context);
            Node::at(Rect::new(0.0, 0.0, 10.0, 10.0)).sync(element.as_ref(), &context);
        }

        assert_eq!(last_bounds("flood-0"), None);
        assert!(last_bounds(&format!("flood-{}", REMEMBERED_LIMIT + 7)).is_some());
    }

    #[test]
    fn layout_is_the_childs() {
        let context = context();
        let element = tagged("layout", &context);

        let size = element.computed_size(&context);

        assert_eq!((size.width, size.height), (10.0, 10.0));
    }

    /// A tile whose widget is built by a counted factory.
    fn counted(tag: &str, builds: &Rc<Cell<usize>>, context: &BuildContext) -> AnyElement {
        let counter = Rc::clone(builds);
        SharedElement::new()
            .tag(tag)
            .child_with(move || {
                counter.set(counter.get() + 1);
                Leaf
            })
            .to_element(context)
    }

    fn snapshot(tag: &str) -> Option<SharedElementSnapshot> {
        last_shared_element(&Key::from(tag))
    }

    #[test]
    fn a_repeatable_child_can_be_built_again_for_the_overlay() {
        let builds = Rc::new(Cell::new(0));
        let context = context();
        let element = counted("repeatable", &builds, &context);
        Node::at(Rect::new(1.0, 2.0, 30.0, 40.0)).sync(element.as_ref(), &context);
        assert_eq!(builds.get(), 1, "the real child is built once");

        let content = snapshot("repeatable").and_then(|s| s.content).expect("a repeatable child");
        let _copy = content.build().to_element(&context);

        assert_eq!(builds.get(), 2, "the overlay gets its own copy");
        let _another = content.build();
        assert_eq!(builds.get(), 3);
    }

    #[test]
    fn a_plain_child_has_bounds_but_no_content_to_repeat() {
        let context = context();
        let element = tagged("plain", &context);
        Node::at(Rect::new(1.0, 2.0, 30.0, 40.0)).sync(element.as_ref(), &context);

        let snapshot = snapshot("plain").unwrap();

        assert_eq!(snapshot.bounds, ElementBounds::new(1.0, 2.0, 30.0, 40.0));
        assert!(snapshot.content.is_none());
    }

    #[test]
    fn a_dropped_elements_content_is_remembered_with_its_bounds() {
        let builds = Rc::new(Cell::new(0));
        let context = context();
        let element = counted("remembered-content", &builds, &context);
        Node::at(Rect::new(5.0, 6.0, 7.0, 8.0)).sync(element.as_ref(), &context);

        drop(element);

        let snapshot = snapshot("remembered-content").expect("remembered");
        assert_eq!(snapshot.bounds, ElementBounds::new(5.0, 6.0, 7.0, 8.0));
        // Going back builds the overlay before the grid has built its tile.
        let before = builds.get();
        drop(snapshot.content.expect("content survives the drop").build());
        assert_eq!(builds.get(), before + 1);
    }
}
