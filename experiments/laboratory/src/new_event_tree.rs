use aimer_attribute::{Bounds, CacheBounds};
use std::cell::Cell;

/// Identifies an element in the general element tree's stable node storage.
///
/// The element tree assigns this ID when it creates a node. Keep the ID stable
/// for the lifetime of the retained tree; replace the corresponding event tree
/// when the element tree is replaced.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ElementId(usize);

impl ElementId {
    /// Returns the element-arena index represented by this ID.
    #[inline]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Identifies one registered target in an [`EventTree`].
///
/// Elements that do not handle events never receive an event-target ID and do
/// not appear in the event tree.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EventTargetId(usize);

/// A sparse event-tree entry for one event-handling element.
pub struct EventElement {
    element: ElementId,
    bound: CacheBounds,
    parent: Option<EventTargetId>,
    children: Vec<EventTargetId>,
}

impl EventElement {
    /// Returns the ID of the corresponding general-tree element.
    #[inline]
    pub const fn element_id(&self) -> ElementId {
        self.element
    }

    /// Returns a copy of the cached hit-test bounds for this target.
    #[inline]
    pub fn bounds(&self) -> Option<Bounds> {
        self.bound.get_bounds()
    }

    /// Returns the closest event-handling ancestor, if one exists.
    #[inline]
    pub const fn parent(&self) -> Option<EventTargetId> {
        self.parent
    }

    /// Returns event-handling descendants whose nearest event ancestor is this
    /// target, in paint order.
    #[inline]
    pub fn children(&self) -> &[EventTargetId] {
        &self.children
    }
}

/// A sparse, bounds-pruned hit-test view over the general element tree.
///
/// Register only elements that handle events. A balanced hierarchy groups
/// consecutive registrations and stores the union of their bounds at each
/// branch. Hit testing prunes branches outside the pointer position. Since
/// registrations follow paint order, [`hit_test`](Self::hit_test) yields
/// overlapping targets topmost first. The event tree stores IDs instead of
/// references, leaving element ownership in the general tree and avoiding
/// self-referential data.
#[derive(Default)]
pub struct EventTree {
    elements: Vec<EventElement>,
    roots: Vec<EventTargetId>,
    index: EventSpatialIndex,
}

#[derive(Default)]
struct EventSpatialIndex {
    capacity: usize,
    nodes: Vec<EventSpatialNode>,
}

enum EventSpatialNode {
    Empty,
    Branch(Cell<SpatialBounds>),
    Leaf(usize),
}

#[derive(Clone, Copy, Default)]
struct SpatialBounds {
    bounds: Option<Bounds>,
    has_unbounded_target: bool,
}

impl SpatialBounds {
    #[inline]
    fn contains(self, x: f32, y: f32) -> bool {
        self.has_unbounded_target || self.bounds.is_some_and(|bounds| bounds.is_inside(x, y))
    }
}

impl Default for EventSpatialNode {
    fn default() -> Self {
        Self::Empty
    }
}

impl EventSpatialIndex {
    fn register(&mut self, target: usize, elements: &[EventElement]) {
        if target >= self.capacity {
            let capacity = target
                .saturating_add(1)
                .checked_next_power_of_two()
                .expect("event tree capacity overflow");
            self.rebuild(capacity, elements);
            return;
        }

        self.nodes[self.capacity + target] = EventSpatialNode::Leaf(target);
        self.refit_target(target, elements);
    }

    fn rebuild(&mut self, capacity: usize, elements: &[EventElement]) {
        let mut nodes = Vec::with_capacity(capacity * 2);
        nodes.resize_with(capacity * 2, EventSpatialNode::default);
        for target in 0..elements.len() {
            nodes[capacity + target] = EventSpatialNode::Leaf(target);
        }
        for node in 1..capacity {
            nodes[node] = EventSpatialNode::Branch(Cell::new(SpatialBounds::default()));
        }

        self.capacity = capacity;
        self.nodes = nodes;
        for node in (1..capacity).rev() {
            self.refit_node(node, elements);
        }
    }

    fn refit_target(&self, target: usize, elements: &[EventElement]) {
        let mut node = (self.capacity + target) / 2;
        while node > 0 {
            self.refit_node(node, elements);
            node /= 2;
        }
    }

    fn refit_node(&self, node: usize, elements: &[EventElement]) {
        let bounds = union_spatial_bounds(
            self.node_bounds(node * 2, elements),
            self.node_bounds(node * 2 + 1, elements),
        );
        if let EventSpatialNode::Branch(cache) = &self.nodes[node] {
            cache.set(bounds);
        }
    }

    fn node_bounds(&self, node: usize, elements: &[EventElement]) -> SpatialBounds {
        match &self.nodes[node] {
            EventSpatialNode::Empty => SpatialBounds::default(),
            EventSpatialNode::Branch(bounds) => bounds.get(),
            EventSpatialNode::Leaf(target) => {
                let bounds = elements
                    .get(*target)
                    .and_then(|element| element.bound.get_bounds());
                SpatialBounds {
                    bounds,
                    has_unbounded_target: bounds.is_none(),
                }
            }
        }
    }
}

fn union_spatial_bounds(left: SpatialBounds, right: SpatialBounds) -> SpatialBounds {
    let bounds = match (left.bounds, right.bounds) {
        (None, None) => None,
        (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
        (Some(left), Some(right)) => {
            let x = left.x.min(right.x);
            let y = left.y.min(right.y);
            let max_x = (left.x + left.width).max(right.x + right.width);
            let max_y = (left.y + left.height).max(right.y + right.height);
            Some(Bounds::new(x, y, max_x - x, max_y - y))
        }
    };
    SpatialBounds {
        bounds,
        has_unbounded_target: left.has_unbounded_target || right.has_unbounded_target,
    }
}

impl EventTree {
    /// Creates an empty event tree.
    #[inline]
    pub const fn new() -> Self {
        Self {
            elements: Vec::new(),
            roots: Vec::new(),
            index: EventSpatialIndex {
                capacity: 0,
                nodes: Vec::new(),
            },
        }
    }

    /// Adds one event-handling element in paint order.
    ///
    /// Keep the returned ID on the corresponding element so layout can update
    /// its bounds and refit the index path in logarithmic time. Non-event
    /// elements are not registered.
    #[inline]
    pub fn register(&mut self, element: ElementId) -> EventTargetId {
        self.register_under(element, None)
    }

    fn register_under(
        &mut self,
        element: ElementId,
        parent: Option<EventTargetId>,
    ) -> EventTargetId {
        let target = EventTargetId(self.elements.len());
        if let Some(parent) = parent {
            self.elements
                .get_mut(parent.0)
                .expect("event parent is registered before its children")
                .children
                .push(target);
        } else {
            self.roots.push(target);
        }
        self.elements.push(EventElement {
            element,
            bound: CacheBounds::new(),
            parent,
            children: Vec::new(),
        });
        self.index.register(target.0, &self.elements);
        target
    }

    /// Returns the registered target at `target`, if it still exists.
    #[inline]
    pub fn get(&self, target: EventTargetId) -> Option<&EventElement> {
        self.elements.get(target.0)
    }

    /// Returns the event targets without event-handling ancestors.
    #[inline]
    pub fn roots(&self) -> &[EventTargetId] {
        &self.roots
    }

    /// Returns a copy of a target's cached bounds, if it has been laid out.
    #[inline]
    pub fn bounds(&self, target: EventTargetId) -> Option<Bounds> {
        self.get(target).and_then(EventElement::bounds)
    }

    fn update_bounds(
        &self,
        target: EventTargetId,
        scale: f32,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> bool {
        let Some(element) = self.elements.get(target.0) else {
            return false;
        };
        element.bound.save(scale, x, y, width, height);
        self.index.refit_target(target.0, &self.elements);
        true
    }

    /// Returns event targets containing `(x, y)`, topmost first.
    ///
    /// The returned IDs can be resolved through [`get`](Self::get), then
    /// [`EventElement::element_id`] can be used to find the handler in the
    /// general element tree. Branch bounds are the union of descendant target
    /// bounds. Bounds should be written in the same coordinate space as the
    /// hit-test point; layout should account for ancestor offsets and clipping
    /// when it updates them.
    #[inline]
    pub fn hit_test(&self, x: f32, y: f32) -> impl Iterator<Item = EventTargetId> + '_ {
        self.hit_test_iter(x, y, true)
    }

    /// Returns event targets containing `(x, y)` in paint order.
    ///
    /// This order is useful when supplying children to framework dispatchers:
    /// they walk the supplied children in reverse so the last painted child is
    /// offered the event first.
    #[inline]
    pub fn hit_test_paint_order(&self, x: f32, y: f32) -> impl Iterator<Item = EventTargetId> + '_ {
        self.hit_test_iter(x, y, false)
    }

    fn hit_test_iter(&self, x: f32, y: f32, reverse_paint_order: bool) -> EventTreeHitTest<'_> {
        EventTreeHitTest::new(self, x, y, reverse_paint_order)
    }

    /// Returns the registered event targets in paint order.
    #[inline]
    pub fn elements(&self) -> &[EventElement] {
        &self.elements
    }
}

struct EventTreeHitTest<'a> {
    tree: &'a EventTree,
    stack: [usize; usize::BITS as usize],
    stack_len: usize,
    x: f32,
    y: f32,
    reverse_paint_order: bool,
    #[cfg(test)]
    index_bounds_checked: usize,
    #[cfg(test)]
    target_bounds_checked: usize,
}

impl<'a> EventTreeHitTest<'a> {
    fn new(tree: &'a EventTree, x: f32, y: f32, reverse_paint_order: bool) -> Self {
        let mut stack = [0; usize::BITS as usize];
        let stack_len = if tree.index.capacity > 0 {
            stack[0] = 1;
            1
        } else {
            0
        };
        Self {
            tree,
            stack,
            stack_len,
            x,
            y,
            reverse_paint_order,
            #[cfg(test)]
            index_bounds_checked: 0,
            #[cfg(test)]
            target_bounds_checked: 0,
        }
    }

    #[inline]
    fn push(&mut self, node: usize) {
        assert!(
            self.stack_len < self.stack.len(),
            "event index depth overflow"
        );
        self.stack[self.stack_len] = node;
        self.stack_len += 1;
    }
}

impl Iterator for EventTreeHitTest<'_> {
    type Item = EventTargetId;

    fn next(&mut self) -> Option<Self::Item> {
        while self.stack_len > 0 {
            self.stack_len -= 1;
            let node = self.stack[self.stack_len];
            match &self.tree.index.nodes[node] {
                EventSpatialNode::Empty => {}
                EventSpatialNode::Branch(bounds) => {
                    #[cfg(test)]
                    {
                        self.index_bounds_checked += 1;
                    }
                    if bounds.get().contains(self.x, self.y) {
                        // The right child contains later paint-order entries.
                        // LIFO order can yield them first for dispatch, or
                        // preserve insertion order for a parent's child view.
                        if self.reverse_paint_order {
                            self.push(node * 2);
                            self.push(node * 2 + 1);
                        } else {
                            self.push(node * 2 + 1);
                            self.push(node * 2);
                        }
                    }
                }
                EventSpatialNode::Leaf(target) => {
                    #[cfg(test)]
                    {
                        self.target_bounds_checked += 1;
                    }
                    let element = &self.tree.elements[*target];
                    if element
                        .bound
                        .get_bounds()
                        .is_none_or(|bounds| bounds.is_inside(self.x, self.y))
                    {
                        return Some(EventTargetId(*target));
                    }
                }
            }
        }
        None
    }
}

/// Whether an element should also appear in the sparse event tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventParticipation {
    /// The element is retained in the general tree only.
    None,
    /// The element handles events and receives an event-tree entry.
    HandlesEvents,
}

/// A retained general element tree with a sparse event-target index.
///
/// Every element is stored once in `nodes`. Only elements explicitly marked
/// [`EventParticipation::HandlesEvents`] are registered in `events`. The
/// event tree stores their IDs and hit-test bounds in a prunable hierarchy, so
/// lookup can skip both ordinary layout containers and unrelated event targets.
#[derive(Default)]
pub struct ElementTree {
    nodes: Vec<ElementNode>,
    roots: Vec<ElementId>,
    events: EventTree,
}

struct ElementNode {
    element: crate::element::AnyElement,
    parent: Option<ElementId>,
    children: Vec<ElementId>,
    event_target: Option<EventTargetId>,
    event_scope: Option<EventTargetId>,
}

/// An insertion failed because its requested parent is not in this tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MissingParent(pub ElementId);

impl ElementTree {
    /// Creates an empty retained element tree and event index.
    #[inline]
    pub const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            roots: Vec::new(),
            events: EventTree::new(),
        }
    }

    /// Inserts an element under `parent`, or as a root when `parent` is `None`.
    ///
    /// The returned ID is the element's stable arena slot for this tree. Mark
    /// only actual event handlers as [`EventParticipation::HandlesEvents`];
    /// ordinary containers remain in the general tree but add no event entry.
    /// Insert handlers in paint order so overlapping hit targets are tested
    /// topmost first.
    pub fn insert(
        &mut self,
        parent: Option<ElementId>,
        element: crate::element::AnyElement,
        event_participation: EventParticipation,
    ) -> Result<ElementId, MissingParent> {
        let parent_index = match parent {
            Some(parent) => Some(self.node_index(parent).ok_or(MissingParent(parent))?),
            None => None,
        };

        let id = ElementId(self.nodes.len());
        let event_parent = parent_index.and_then(|index| self.nodes[index].event_scope);
        let event_target = match event_participation {
            EventParticipation::None => None,
            EventParticipation::HandlesEvents => Some(self.events.register_under(id, event_parent)),
        };
        let event_scope = event_target.or(event_parent);

        self.nodes.push(ElementNode {
            element,
            parent,
            children: Vec::new(),
            event_target,
            event_scope,
        });

        if let Some(parent_index) = parent_index {
            self.nodes[parent_index].children.push(id);
        } else {
            self.roots.push(id);
        }

        Ok(id)
    }

    /// Returns the retained element for `id`, if the ID belongs to this tree.
    #[inline]
    pub fn element(&self, id: ElementId) -> Option<&crate::element::AnyElement> {
        self.node(id).map(|node| &node.element)
    }

    /// Returns mutable access to the retained element for `id`.
    #[inline]
    pub fn element_mut(&mut self, id: ElementId) -> Option<&mut crate::element::AnyElement> {
        self.nodes.get_mut(id.0).map(|node| &mut node.element)
    }

    /// Returns the parent ID for `id`, or `None` when it is a root or absent.
    #[inline]
    pub fn parent(&self, id: ElementId) -> Option<ElementId> {
        self.node(id).and_then(|node| node.parent)
    }

    /// Returns the children of `id` in insertion order.
    #[inline]
    pub fn children(&self, id: ElementId) -> Option<&[ElementId]> {
        self.node(id).map(|node| node.children.as_slice())
    }

    /// Returns the root IDs in insertion order.
    #[inline]
    pub fn roots(&self) -> &[ElementId] {
        &self.roots
    }

    /// Returns the sparse event tree retained alongside the elements.
    #[inline]
    pub fn event_tree(&self) -> &EventTree {
        &self.events
    }

    /// Updates an event handler's cached bounds and refits its event-index path.
    ///
    /// Layout updates take logarithmic time in the number of event targets.
    /// Returns `false` when `id` is missing or is not an event handler.
    #[inline]
    pub fn update_event_bounds(
        &self,
        id: ElementId,
        scale: f32,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> bool {
        let Some(target) = self.node(id).and_then(|node| node.event_target) else {
            return false;
        };
        self.events
            .update_bounds(target, scale, x, y, width, height)
    }

    /// Returns the event-handling elements whose bounds contain `(x, y)`,
    /// topmost first.
    #[inline]
    pub fn hit_test(&self, x: f32, y: f32) -> impl Iterator<Item = ElementId> + '_ {
        self.events
            .hit_test(x, y)
            .filter_map(|target| self.events.get(target).map(EventElement::element_id))
    }

    /// Returns the number of elements in the general tree.
    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns whether the general element tree is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[inline]
    fn node(&self, id: ElementId) -> Option<&ElementNode> {
        self.nodes.get(id.0)
    }

    #[inline]
    fn node_index(&self, id: ElementId) -> Option<usize> {
        self.nodes.get(id.0).map(|_| id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{AnyElement, Element};
    use std::hint::black_box;
    use std::time::Instant;

    struct StressElement;

    impl Element for StressElement {}

    fn stress_tree() -> (ElementTree, Vec<(ElementId, f32)>) {
        const NODE_COUNT: usize = 100_000;
        const HANDLER_INTERVAL: usize = 100;

        let mut tree = ElementTree::new();
        let root = tree
            .insert(
                None,
                AnyElement::new(StressElement),
                EventParticipation::None,
            )
            .unwrap();

        // Build a six-level general-tree path ending in the button target.
        let mut parent = root;
        for _ in 0..5 {
            parent = tree
                .insert(
                    Some(parent),
                    AnyElement::new(StressElement),
                    EventParticipation::None,
                )
                .unwrap();
        }
        let button = tree
            .insert(
                Some(parent),
                AnyElement::new(StressElement),
                EventParticipation::HandlesEvents,
            )
            .unwrap();
        assert!(tree.update_event_bounds(button, 1.0, 0.0, 0.0, 4.0, 4.0));

        let mut expected_hits = vec![(button, 2.0_f32)];
        let mut next_x = 10.0_f32;
        for index in 7..NODE_COUNT {
            let handles_events = index % HANDLER_INTERVAL == 0;
            let element = tree
                .insert(
                    Some(root),
                    AnyElement::new(StressElement),
                    if handles_events {
                        EventParticipation::HandlesEvents
                    } else {
                        EventParticipation::None
                    },
                )
                .unwrap();

            if handles_events {
                assert!(tree.update_event_bounds(element, 1.0, next_x, 0.0, 4.0, 4.0));
                expected_hits.push((element, next_x + 2.0));
                next_x += 10.0;
            }
        }

        (tree, expected_hits)
    }

    #[test]
    fn sparse_event_tree_stress_keeps_only_event_handlers() {
        let (tree, expected_hits) = stress_tree();

        assert_eq!(tree.len(), 100_000);
        assert_eq!(tree.event_tree().elements().len(), 1_000);
        assert!(!tree.update_event_bounds(tree.roots()[0], 1.0, 0.0, 0.0, 1.0, 1.0));

        for (expected, x) in expected_hits {
            let mut hits = tree.hit_test(x, 2.0);
            assert_eq!(hits.next(), Some(expected));
            assert_eq!(hits.next(), None);
        }

        // A point inside a regular container but outside every handler misses.
        assert_eq!(tree.hit_test(6.0, 2.0).next(), None);
    }

    // Mirrored from jaime/src/showcase.rs: seven categories with 55 example
    // buttons. Each category is expanded, matching CollapsibleList::new().
    const SHOWCASE_CATEGORY_BUTTON_COUNTS: [usize; 7] = [10, 5, 7, 5, 8, 12, 8];
    const SHOWCASE_BUTTON_COUNT: usize = 55;
    const SHOWCASE_VIEWPORT_HEIGHT: f32 = 720.0;
    const SHOWCASE_PANE_WIDTH: f32 = 341.0;
    const SHOWCASE_PANE_STRIDE: f32 = 360.0;
    const SHOWCASE_CATEGORY_WIDTH: f32 = 300.0;
    const SHOWCASE_CATEGORY_TOP_INSET: f32 = if cfg!(target_os = "macos") { 38.0 } else { 0.0 };
    const SHOWCASE_HEADER_HEIGHT: f32 = 30.0;
    const SHOWCASE_BUTTON_HEIGHT: f32 = 39.0;
    const SHOWCASE_BUTTON_STEP: f32 = 42.0;

    #[derive(Clone, Copy)]
    struct TestRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    }

    struct ShowcaseFixture {
        tree: ElementTree,
        general_bounds: Vec<CacheBounds>,
        button_targets: Vec<(ElementId, TestRect)>,
        header_targets: Vec<(ElementId, TestRect)>,
    }

    fn showcase_content_height() -> f32 {
        SHOWCASE_CATEGORY_BUTTON_COUNTS
            .iter()
            .map(|count| SHOWCASE_HEADER_HEIGHT + *count as f32 * SHOWCASE_BUTTON_STEP + 3.0)
            .sum::<f32>()
            + SHOWCASE_CATEGORY_TOP_INSET
    }

    fn insert_showcase_node(
        tree: &mut ElementTree,
        general_bounds: &mut Vec<CacheBounds>,
        parent: Option<ElementId>,
        participation: EventParticipation,
        rect: TestRect,
    ) -> ElementId {
        let id = tree
            .insert(parent, AnyElement::new(StressElement), participation)
            .expect("showcase fixture inserts parents before children");
        assert_eq!(id.index(), general_bounds.len());

        let bounds = CacheBounds::new();
        bounds.save(1.0, rect.x, rect.y, rect.width, rect.height);
        general_bounds.push(bounds);

        if participation == EventParticipation::HandlesEvents {
            assert!(tree.update_event_bounds(id, 1.0, rect.x, rect.y, rect.width, rect.height,));
        }

        id
    }

    fn build_showcase_fixture(copies: usize, scroll_offset: f32) -> ShowcaseFixture {
        build_showcase_fixture_with_pane_stride(copies, scroll_offset, SHOWCASE_PANE_STRIDE)
    }

    fn build_showcase_fixture_with_pane_stride(
        copies: usize,
        scroll_offset: f32,
        pane_stride: f32,
    ) -> ShowcaseFixture {
        assert!(copies > 0);

        let mut tree = ElementTree::new();
        let mut general_bounds = Vec::new();
        let mut button_targets = Vec::with_capacity(copies * SHOWCASE_BUTTON_COUNT);
        let mut header_targets = Vec::with_capacity(copies * SHOWCASE_CATEGORY_BUTTON_COUNTS.len());
        let content_height = showcase_content_height();

        for copy in 0..copies {
            let pane_x = copy as f32 * pane_stride;
            let viewport = TestRect {
                x: pane_x,
                y: 0.0,
                width: SHOWCASE_PANE_WIDTH,
                height: SHOWCASE_VIEWPORT_HEIGHT,
            };
            let sidebar = TestRect {
                x: pane_x,
                width: 340.0,
                ..viewport
            };
            let category_column = TestRect {
                x: pane_x + 18.0,
                y: -scroll_offset,
                width: SHOWCASE_CATEGORY_WIDTH,
                height: content_height,
            };

            // ExampleShowcase state -> Container -> Column -> Expanded -> Row.
            let showcase = insert_showcase_node(
                &mut tree,
                &mut general_bounds,
                None,
                EventParticipation::None,
                viewport,
            );
            let container = insert_showcase_node(
                &mut tree,
                &mut general_bounds,
                Some(showcase),
                EventParticipation::None,
                viewport,
            );
            let column = insert_showcase_node(
                &mut tree,
                &mut general_bounds,
                Some(container),
                EventParticipation::None,
                viewport,
            );
            let expanded = insert_showcase_node(
                &mut tree,
                &mut general_bounds,
                Some(column),
                EventParticipation::None,
                viewport,
            );
            let row = insert_showcase_node(
                &mut tree,
                &mut general_bounds,
                Some(expanded),
                EventParticipation::None,
                viewport,
            );

            // The content pane is commented out in the current showcase build.
            let sidebar_container = insert_showcase_node(
                &mut tree,
                &mut general_bounds,
                Some(row),
                EventParticipation::None,
                sidebar,
            );
            let scrollable = insert_showcase_node(
                &mut tree,
                &mut general_bounds,
                Some(sidebar_container),
                EventParticipation::None,
                sidebar,
            );
            let category_column_id = insert_showcase_node(
                &mut tree,
                &mut general_bounds,
                Some(scrollable),
                EventParticipation::None,
                category_column,
            );
            let _divider = insert_showcase_node(
                &mut tree,
                &mut general_bounds,
                Some(row),
                EventParticipation::None,
                TestRect {
                    x: pane_x + 340.0,
                    y: 0.0,
                    width: 1.0,
                    height: SHOWCASE_VIEWPORT_HEIGHT,
                },
            );

            let mut content_y = SHOWCASE_CATEGORY_TOP_INSET;
            for (category_index, button_count) in
                SHOWCASE_CATEGORY_BUTTON_COUNTS.into_iter().enumerate()
            {
                let category_y = content_y - scroll_offset;
                let body_height = button_count as f32 * SHOWCASE_BUTTON_STEP;
                let category_bounds = TestRect {
                    x: category_column.x,
                    y: category_y,
                    width: category_column.width,
                    height: SHOWCASE_HEADER_HEIGHT + body_height,
                };
                let category_parent = if cfg!(target_os = "macos") && category_index == 0 {
                    insert_showcase_node(
                        &mut tree,
                        &mut general_bounds,
                        Some(category_column_id),
                        EventParticipation::None,
                        TestRect {
                            y: -scroll_offset,
                            height: category_bounds.height + SHOWCASE_CATEGORY_TOP_INSET,
                            ..category_bounds
                        },
                    )
                } else {
                    category_column_id
                };
                let category = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(category_parent),
                    EventParticipation::None,
                    category_bounds,
                );
                let category_build = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(category),
                    EventParticipation::None,
                    category_bounds,
                );

                // CollapsibleList builds a focusable, key-relay and tap header.
                // Keep its pointer target in the event index too.
                let header_rect = TestRect {
                    height: SHOWCASE_HEADER_HEIGHT,
                    ..category_bounds
                };
                let focusable = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(category_build),
                    EventParticipation::None,
                    header_rect,
                );
                let key_relay = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(focusable),
                    EventParticipation::None,
                    header_rect,
                );
                let header_gesture = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(key_relay),
                    EventParticipation::HandlesEvents,
                    header_rect,
                );
                header_targets.push((header_gesture, header_rect));
                let header_container = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(header_gesture),
                    EventParticipation::None,
                    header_rect,
                );
                let header_row = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(header_container),
                    EventParticipation::None,
                    header_rect,
                );
                let header_expanded = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(header_row),
                    EventParticipation::None,
                    TestRect {
                        x: header_rect.x + 12.0,
                        width: 240.0,
                        ..header_rect
                    },
                );
                let header_label_container = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(header_expanded),
                    EventParticipation::None,
                    TestRect {
                        x: header_rect.x + 12.0,
                        width: 240.0,
                        ..header_rect
                    },
                );
                let _header_label = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(header_label_container),
                    EventParticipation::None,
                    TestRect {
                        x: header_rect.x + 16.0,
                        y: header_rect.y + 5.0,
                        width: 200.0,
                        height: 20.0,
                    },
                );
                let chevron = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(header_row),
                    EventParticipation::None,
                    TestRect {
                        x: header_rect.x + 268.0,
                        y: header_rect.y + 3.0,
                        width: 24.0,
                        height: 24.0,
                    },
                );
                let rotation = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(chevron),
                    EventParticipation::None,
                    TestRect {
                        x: header_rect.x + 268.0,
                        y: header_rect.y + 3.0,
                        width: 24.0,
                        height: 24.0,
                    },
                );
                let _chevron_svg = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(rotation),
                    EventParticipation::None,
                    TestRect {
                        x: header_rect.x + 268.0,
                        y: header_rect.y + 3.0,
                        width: 24.0,
                        height: 24.0,
                    },
                );

                // The expanded body is AnimatedCollapse -> Container ->
                // ListBody's Container -> Column of example buttons.
                let body_rect = TestRect {
                    y: category_y + SHOWCASE_HEADER_HEIGHT,
                    height: body_height,
                    ..category_bounds
                };
                let collapse = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(category_build),
                    EventParticipation::None,
                    body_rect,
                );
                let body_container = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(collapse),
                    EventParticipation::None,
                    body_rect,
                );
                let body_slot = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(body_container),
                    EventParticipation::None,
                    body_rect,
                );
                let buttons_column = insert_showcase_node(
                    &mut tree,
                    &mut general_bounds,
                    Some(body_slot),
                    EventParticipation::None,
                    body_rect,
                );

                for button_index in 0..button_count {
                    let button_rect = TestRect {
                        x: category_column.x,
                        y: body_rect.y + button_index as f32 * SHOWCASE_BUTTON_STEP,
                        width: category_column.width,
                        height: SHOWCASE_BUTTON_HEIGHT,
                    };
                    let button = insert_showcase_node(
                        &mut tree,
                        &mut general_bounds,
                        Some(buttons_column),
                        EventParticipation::None,
                        button_rect,
                    );
                    let mouse_region = insert_showcase_node(
                        &mut tree,
                        &mut general_bounds,
                        Some(button),
                        EventParticipation::None,
                        button_rect,
                    );
                    let button_gesture = insert_showcase_node(
                        &mut tree,
                        &mut general_bounds,
                        Some(mouse_region),
                        EventParticipation::HandlesEvents,
                        button_rect,
                    );
                    button_targets.push((button_gesture, button_rect));
                    let decoration = insert_showcase_node(
                        &mut tree,
                        &mut general_bounds,
                        Some(button_gesture),
                        EventParticipation::None,
                        button_rect,
                    );
                    let content_container = insert_showcase_node(
                        &mut tree,
                        &mut general_bounds,
                        Some(decoration),
                        EventParticipation::None,
                        button_rect,
                    );
                    let button_row_rect = TestRect {
                        x: button_rect.x + 8.0,
                        width: button_rect.width - 16.0,
                        ..button_rect
                    };
                    let button_row = insert_showcase_node(
                        &mut tree,
                        &mut general_bounds,
                        Some(content_container),
                        EventParticipation::None,
                        button_row_rect,
                    );
                    let icon_rect = TestRect {
                        x: button_row_rect.x,
                        width: 24.0,
                        ..button_rect
                    };
                    let icon_container = insert_showcase_node(
                        &mut tree,
                        &mut general_bounds,
                        Some(button_row),
                        EventParticipation::None,
                        icon_rect,
                    );
                    let _icon_text = insert_showcase_node(
                        &mut tree,
                        &mut general_bounds,
                        Some(icon_container),
                        EventParticipation::None,
                        icon_rect,
                    );
                    let _label = insert_showcase_node(
                        &mut tree,
                        &mut general_bounds,
                        Some(button_row),
                        EventParticipation::None,
                        TestRect {
                            x: button_row_rect.x + 34.0,
                            width: button_row_rect.width - 34.0,
                            ..button_rect
                        },
                    );
                }

                content_y += SHOWCASE_HEADER_HEIGHT + body_height + 3.0;
            }
        }

        assert_eq!(button_targets.len(), copies * SHOWCASE_BUTTON_COUNT);
        assert_eq!(general_bounds.len(), tree.len());
        ShowcaseFixture {
            tree,
            general_bounds,
            button_targets,
            header_targets,
        }
    }

    fn pruned_general_tree_hit_count(
        tree: &ElementTree,
        bounds: &[CacheBounds],
        x: f32,
        y: f32,
        stack: &mut Vec<ElementId>,
    ) -> (usize, usize, Option<ElementId>) {
        stack.clear();
        stack.extend(tree.roots.iter().copied());

        let mut bounds_checked = 0;
        let mut hit_handlers = 0;
        let mut hit_element = None;
        while let Some(id) = stack.pop() {
            bounds_checked += 1;
            if !bounds[id.index()].is_inside(x, y) {
                continue;
            }

            let node = tree.node(id).expect("tree links contain valid element IDs");
            if node.event_target.is_some() {
                hit_handlers += 1;
                hit_element = Some(id);
                black_box(id);
            }
            stack.extend(node.children.iter().copied());
        }

        (bounds_checked, hit_handlers, hit_element)
    }

    fn event_tree_hit_count(
        tree: &ElementTree,
        x: f32,
        y: f32,
    ) -> (usize, usize, Option<ElementId>) {
        let mut hit_test = tree.events.hit_test_iter(x, y, true);
        let mut hits = 0;
        let mut hit_element = None;
        while let Some(target) = hit_test.next() {
            let event = tree.events.get(target).expect("hit target exists");
            hits += 1;
            hit_element = Some(event.element);
            black_box(event.element);
        }
        let bounds_checked = hit_test.index_bounds_checked + hit_test.target_bounds_checked;
        (bounds_checked, hits, hit_element)
    }

    fn tree_depth(tree: &ElementTree, mut id: ElementId) -> usize {
        let mut depth = 1;
        while let Some(parent) = tree.parent(id) {
            depth += 1;
            id = parent;
        }
        depth
    }

    #[test]
    fn showcase_sidebar_tree_routes_visible_buttons_through_layout_wrappers() {
        let fixture = build_showcase_fixture(1, 0.0);
        let expected_node_count = 623 + if cfg!(target_os = "macos") { 1 } else { 0 };
        assert_eq!(fixture.tree.len(), expected_node_count);
        assert_eq!(fixture.button_targets.len(), SHOWCASE_BUTTON_COUNT);
        assert_eq!(fixture.header_targets.len(), 7);
        assert_eq!(fixture.tree.event_tree().elements().len(), 62);
        let expected_button_depth = 17 + if cfg!(target_os = "macos") { 1 } else { 0 };
        assert_eq!(
            tree_depth(&fixture.tree, fixture.button_targets[0].0),
            expected_button_depth
        );

        // Later categories remain in the scrollable tree beyond the 720 px view.
        let visible_buttons = fixture
            .button_targets
            .iter()
            .filter(|(_, rect)| rect.y >= 0.0 && rect.y + rect.height <= SHOWCASE_VIEWPORT_HEIGHT)
            .count();
        assert_eq!(
            visible_buttons,
            if cfg!(target_os = "macos") { 14 } else { 15 }
        );

        let mut stack = Vec::with_capacity(fixture.tree.len());
        for (expected, rect) in fixture.button_targets.iter().take(visible_buttons) {
            let x = rect.x + 20.0;
            let y = rect.y + rect.height / 2.0;
            let (_, general_hits, general_target) = pruned_general_tree_hit_count(
                &fixture.tree,
                &fixture.general_bounds,
                x,
                y,
                &mut stack,
            );
            let event_hits = fixture.tree.hit_test(x, y).collect::<Vec<_>>();

            assert_eq!(general_hits, 1);
            assert_eq!(general_target, Some(*expected));
            assert_eq!(event_hits, vec![*expected]);
        }

        // A visible header is also a pointer target because CollapsibleList
        // installs a GestureDetector over its header.
        let (header, header_rect) = fixture.header_targets[0];
        let header_x = header_rect.x + 20.0;
        let header_y = header_rect.y + header_rect.height / 2.0;
        assert_eq!(
            fixture.tree.hit_test(header_x, header_y).next(),
            Some(header)
        );

        // An inter-button gap and the adjacent divider are not event targets.
        let first_button = fixture.button_targets[0].1;
        assert_eq!(
            fixture
                .tree
                .hit_test(
                    first_button.x + 20.0,
                    first_button.y + SHOWCASE_BUTTON_HEIGHT + 1.0
                )
                .next(),
            None
        );
        assert_eq!(fixture.tree.hit_test(340.5, 100.0).next(), None);

        // Scrolling moves the later categories into the same viewport space;
        // the event target's cached bounds and the general-tree geometry agree.
        let scrolled = build_showcase_fixture(1, 1_950.0);
        let (last_button, last_rect) = *scrolled.button_targets.last().unwrap();
        assert!(last_rect.y >= 0.0);
        assert!(last_rect.y + last_rect.height <= SHOWCASE_VIEWPORT_HEIGHT);
        let x = last_rect.x + 20.0;
        let y = last_rect.y + last_rect.height / 2.0;
        let (_, general_hits, general_target) = pruned_general_tree_hit_count(
            &scrolled.tree,
            &scrolled.general_bounds,
            x,
            y,
            &mut stack,
        );
        assert_eq!(general_hits, 1);
        assert_eq!(general_target, Some(last_button));
        assert_eq!(scrolled.tree.hit_test(x, y).next(), Some(last_button));
    }

    #[test]
    fn event_spatial_index_refits_moved_bounds_and_keeps_paint_order() {
        let mut tree = EventTree::new();
        let targets = (0..9)
            .map(|index| tree.register(ElementId(index)))
            .collect::<Vec<_>>();

        for (index, target) in targets.iter().copied().enumerate() {
            assert!(tree.update_bounds(target, 1.0, index as f32 * 10.0, 0.0, 4.0, 4.0,));
        }
        assert!(tree.update_bounds(targets[0], 1.0, 80.0, 0.0, 4.0, 4.0));

        let mut outside = tree.hit_test_iter(5.0, 2.0, true);
        assert_eq!(outside.next(), None);
        assert_eq!(outside.index_bounds_checked, 1);
        assert_eq!(outside.target_bounds_checked, 0);

        assert_eq!(
            tree.hit_test(82.0, 2.0).collect::<Vec<_>>(),
            vec![targets[8], targets[0]]
        );
        assert_eq!(
            tree.hit_test_paint_order(82.0, 2.0).collect::<Vec<_>>(),
            vec![targets[0], targets[8]]
        );
    }

    #[test]
    fn unbounded_event_targets_match_framework_hit_testing() {
        let mut tree = EventTree::new();
        let unbounded = tree.register(ElementId(0));
        let bounded = tree.register(ElementId(1));
        assert!(tree.update_bounds(bounded, 1.0, 40.0, 40.0, 4.0, 4.0));

        assert_eq!(tree.hit_test(2.0, 3.0).collect::<Vec<_>>(), vec![unbounded]);
        assert_eq!(
            tree.hit_test(42.0, 42.0).collect::<Vec<_>>(),
            vec![bounded, unbounded]
        );

        assert!(tree.update_bounds(unbounded, 1.0, 80.0, 80.0, 4.0, 4.0));
        assert_eq!(tree.hit_test(2.0, 3.0).next(), None);
        assert_eq!(
            tree.hit_test(82.0, 82.0).collect::<Vec<_>>(),
            vec![unbounded]
        );
    }

    #[test]
    fn registering_inside_existing_capacity_refits_unbounded_ancestors() {
        let mut tree = EventTree::new();
        let first = tree.register(ElementId(0));
        let second = tree.register(ElementId(1));
        let third = tree.register(ElementId(2));
        for (target, x) in [(first, 0.0), (second, 20.0), (third, 40.0)] {
            assert!(tree.update_bounds(target, 1.0, x, 0.0, 4.0, 4.0));
        }

        // The fourth leaf fits the existing power-of-two index capacity. It
        // has no layout bound yet, so its new ancestor summary must remain
        // unbounded until layout publishes its rectangle.
        let unbounded = tree.register(ElementId(3));
        assert_eq!(
            tree.hit_test(500.0, 500.0).collect::<Vec<_>>(),
            vec![unbounded]
        );

        assert!(tree.update_bounds(unbounded, 1.0, 60.0, 0.0, 4.0, 4.0));
        assert_eq!(tree.hit_test(500.0, 500.0).next(), None);
        assert_eq!(tree.hit_test(62.0, 2.0).next(), Some(unbounded));
    }

    #[test]
    #[ignore = "manual debug-profile comparison; run with --ignored --nocapture"]
    fn compare_pruned_general_tree_walk_with_showcase_event_tree() {
        const ITERATIONS: usize = 1_000;

        for copies in [1, 100] {
            let fixture = build_showcase_fixture(copies, 0.0);
            let (target, rect) = fixture.button_targets[(copies - 1) * SHOWCASE_BUTTON_COUNT];
            let x = rect.x + 20.0;
            let y = rect.y + rect.height / 2.0;
            let mut stack = Vec::with_capacity(fixture.tree.len());

            // Warm both paths and check that they resolve the same target.
            let (_, _, general_warmup) = pruned_general_tree_hit_count(
                &fixture.tree,
                &fixture.general_bounds,
                x,
                y,
                &mut stack,
            );
            let (_, _, event_warmup) = event_tree_hit_count(&fixture.tree, x, y);
            assert_eq!(general_warmup, Some(target));
            assert_eq!(event_warmup, Some(target));

            let started = Instant::now();
            let mut general_bounds_checked = 0;
            let mut general_hits = 0;
            let mut general_hit = None;
            for _ in 0..ITERATIONS {
                let (checked, hits, hit) = pruned_general_tree_hit_count(
                    &fixture.tree,
                    &fixture.general_bounds,
                    x,
                    y,
                    &mut stack,
                );
                general_bounds_checked += checked;
                general_hits += hits;
                general_hit = hit;
            }
            let general_elapsed = started.elapsed();

            let started = Instant::now();
            let mut event_bounds_checked = 0;
            let mut event_hits = 0;
            let mut event_hit = None;
            for _ in 0..ITERATIONS {
                let (checked, hits, hit) = event_tree_hit_count(&fixture.tree, x, y);
                event_bounds_checked += checked;
                event_hits += hits;
                event_hit = hit;
            }
            let event_elapsed = started.elapsed();

            assert_eq!(general_hits, event_hits);
            assert_eq!(general_hit, event_hit);

            let general_micros_per_query =
                general_elapsed.as_secs_f64() * 1_000_000.0 / ITERATIONS as f64;
            let event_micros_per_query =
                event_elapsed.as_secs_f64() * 1_000_000.0 / ITERATIONS as f64;
            let relative_time = general_elapsed.as_secs_f64() / event_elapsed.as_secs_f64();

            println!(
                "showcase copies={copies}, nodes={}, event targets={}, iterations={ITERATIONS}",
                fixture.tree.len(),
                fixture.tree.event_tree().elements().len(),
            );
            println!(
                "pruned general tree: checked {general_bounds_checked} bounds, {general_micros_per_query:.2} µs/query"
            );
            println!(
                "spatial event tree: checked {event_bounds_checked} bounds, {event_micros_per_query:.2} µs/query"
            );
            println!("general/event elapsed ratio: {relative_time:.2}x");
        }
    }

    fn general_tree_hit_count(
        tree: &ElementTree,
        x: f32,
        y: f32,
        stack: &mut Vec<ElementId>,
    ) -> (usize, usize, Option<ElementId>) {
        stack.clear();
        stack.extend(tree.roots.iter().copied());

        let mut visited_nodes = 0;
        let mut hit_handlers = 0;
        let mut hit_element = None;
        while let Some(id) = stack.pop() {
            visited_nodes += 1;
            let node = tree.node(id).expect("tree links contain valid element IDs");
            if let Some(target) = node.event_target {
                let event = tree.events.get(target).expect("registered target exists");
                if event.bound.is_inside(x, y) {
                    hit_handlers += 1;
                    hit_element = Some(event.element);
                    black_box(event.element);
                }
            }

            // This models an unprunable general-tree hit walk: all ordinary
            // nodes' bounds include this point, so the walk visits each node.
            stack.extend(node.children.iter().copied());
        }

        (visited_nodes, hit_handlers, hit_element)
    }

    #[test]
    #[ignore = "manual debug-profile comparison; run with --ignored --nocapture"]
    fn compare_general_tree_walk_with_sparse_event_tree() {
        const ITERATIONS: usize = 200;
        let (tree, _) = stress_tree();
        let mut stack = Vec::with_capacity(tree.len());

        let started = Instant::now();
        let mut general_nodes_visited = 0;
        let mut general_hits = 0;
        let mut general_hit = None;
        for _ in 0..ITERATIONS {
            let (visited, hits, hit) = general_tree_hit_count(&tree, 2.0, 2.0, &mut stack);
            general_nodes_visited += visited;
            general_hits += hits;
            general_hit = hit;
        }
        let general_elapsed = started.elapsed();

        let started = Instant::now();
        let mut event_bounds_checked = 0;
        let mut event_hits = 0;
        let mut event_hit = None;
        for _ in 0..ITERATIONS {
            let mut hit_test = tree.events.hit_test_iter(2.0, 2.0, true);
            while let Some(target) = hit_test.next() {
                let event = tree.events.get(target).expect("hit target exists");
                event_hits += 1;
                event_hit = Some(event.element);
                black_box(event.element);
            }
            event_bounds_checked += hit_test.index_bounds_checked + hit_test.target_bounds_checked;
        }
        let event_elapsed = started.elapsed();

        assert_eq!(general_hits, event_hits);
        assert_eq!(general_hit, event_hit);
        assert_eq!(general_nodes_visited, tree.len() * ITERATIONS);
        assert!(event_bounds_checked > 0);

        let general_micros_per_query =
            general_elapsed.as_secs_f64() * 1_000_000.0 / ITERATIONS as f64;
        let event_micros_per_query = event_elapsed.as_secs_f64() * 1_000_000.0 / ITERATIONS as f64;
        let relative_time = general_elapsed.as_secs_f64() / event_elapsed.as_secs_f64();

        println!(
            "nodes={}, event handlers={}, iterations={ITERATIONS}",
            tree.len(),
            tree.events.elements.len()
        );
        println!(
            "general tree: visited {general_nodes_visited} nodes, {general_micros_per_query:.2} µs/query"
        );
        println!(
            "event tree: checked {event_bounds_checked} index/target bounds, {event_micros_per_query:.2} µs/query"
        );
        println!("general/event elapsed ratio: {relative_time:.2}x");
    }

    mod dense_sparse_framework_dispatch {
        use super::*;
        use aimer_events::element::{
            ElementEvent as FrameworkEvent, KeyAction, Modifiers, NamedKey,
        };
        use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
        use aimer_events::text_editing::{NativeTextRange, TextEditingDelta};
        use aimer_widget::base::BuildContext;
        use aimer_widget::{
            AnyElement as FrameworkAnyElement, Drawable as FrameworkDrawable,
            Element as FrameworkElement, ElementId as FrameworkElementId, EventDispatcher,
            EventElement as FrameworkEventElement, EventResult, EventTreeRole, FocusNode, FollowUp,
            LayoutElement, PointerKey, Rebuildable, VisitorElement, begin_event_frame,
            broadcast_event, dispatch_event, dispatch_focused_event,
        };
        use std::cell::RefCell;
        use std::collections::HashMap;
        use std::path::PathBuf;
        use std::rc::Rc;
        use std::sync::Arc;

        #[derive(Default)]
        struct DispatchMetrics {
            callback_visits: Cell<usize>,
            layout_bound_reads: Cell<usize>,
            spatial_index_checks: Cell<usize>,
            deliveries: RefCell<Vec<(usize, std::mem::Discriminant<FrameworkEvent>)>>,
            effects: RefCell<Vec<(usize, EventResult)>>,
            framework_ids: RefCell<HashMap<usize, FrameworkElementId>>,
        }

        impl DispatchMetrics {
            fn reset(&self) {
                self.callback_visits.set(0);
                self.layout_bound_reads.set(0);
                self.spatial_index_checks.set(0);
                self.deliveries.borrow_mut().clear();
                self.effects.borrow_mut().clear();
            }
        }

        struct ComparisonElement {
            logical_id: Option<usize>,
            participant: bool,
            event_child_targets: Vec<EventTargetId>,
            children: Vec<FrameworkAnyElement>,
            bounds: Rc<Cell<Option<Bounds>>>,
            focus: Option<FocusNode>,
            capture_target: Option<ElementId>,
            metrics: Rc<DispatchMetrics>,
            spatial_index: Option<Rc<EventTree>>,
            overlapping_children: bool,
        }

        impl VisitorElement for ComparisonElement {
            fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn FrameworkElement)) {
                for child in &self.children {
                    visitor(child.as_ref());
                }
            }

            fn debug_name(&self) -> &'static str {
                "EventTreeComparisonElement"
            }
        }

        impl FrameworkEventElement for ComparisonElement {
            fn event_tree_role(&self) -> EventTreeRole {
                match (self.spatial_index.is_some(), self.participant) {
                    (true, true) => EventTreeRole::IndexedTarget,
                    (true, false) => EventTreeRole::Transparent,
                    (false, _) => EventTreeRole::IndexedHitTestBoundary,
                }
            }

            fn focus_node(&self) -> Option<&FocusNode> {
                self.focus.as_ref()
            }

            fn on_event(&self, event: &FrameworkEvent) -> EventResult {
                self.metrics
                    .callback_visits
                    .set(self.metrics.callback_visits.get() + 1);
                let Some(id) = self.logical_id.filter(|_| self.participant) else {
                    return EventResult::ignored();
                };
                self.metrics
                    .deliveries
                    .borrow_mut()
                    .push((id, std::mem::discriminant(event)));

                let result = match event {
                    FrameworkEvent::PointerDown(pointer) => {
                        let result = EventResult::consumed().with_redraw();
                        if self.capture_target == Some(ElementId(id)) {
                            result.with_pointer_capture(PointerKey::new(pointer.source, pointer.id))
                        } else {
                            result
                        }
                    }
                    FrameworkEvent::PointerMove(_)
                        if self.capture_target == Some(ElementId(id)) =>
                    {
                        EventResult::ignored().with_follow_up(FollowUp::DragOver)
                    }
                    FrameworkEvent::PointerMove(_) => EventResult::ignored(),
                    FrameworkEvent::PointerUp(_) if self.capture_target == Some(ElementId(id)) => {
                        EventResult::consumed()
                            .with_redraw()
                            .with_follow_up(FollowUp::DragDrop)
                    }
                    FrameworkEvent::TextInput { .. }
                    | FrameworkEvent::CharInput { .. }
                    | FrameworkEvent::ImePreedit { .. }
                    | FrameworkEvent::KeyInput { .. }
                        if self.focus.as_ref().is_some_and(FocusNode::has_focus) =>
                    {
                        EventResult::consumed().with_redraw()
                    }
                    FrameworkEvent::HoveredFileCancelled => EventResult::redraw(),
                    FrameworkEvent::HoveredFileMoved { .. }
                    | FrameworkEvent::DragOver { .. }
                    | FrameworkEvent::DragDrop { .. } => EventResult::redraw(),
                    FrameworkEvent::Cancel if self.capture_target == Some(ElementId(id)) => {
                        EventResult::consumed()
                    }
                    _ => EventResult::ignored(),
                };
                self.metrics.effects.borrow_mut().push((id, result));
                result
            }

            fn hit_test_children_at<'a>(
                &'a self,
                pos: aimer_attribute::Vec2d,
                visitor: &mut dyn FnMut(&'a dyn FrameworkElement),
            ) {
                let Some(index) = &self.spatial_index else {
                    self.visit_children(visitor);
                    return;
                };
                if self.event_child_targets.is_empty() {
                    return;
                }

                let mut hits = index.hit_test_iter(pos.x, pos.y, false);
                while let Some(target) = hits.next() {
                    if let Ok(child_index) = self
                        .event_child_targets
                        .binary_search_by_key(&target.0, |child_target| child_target.0)
                    {
                        visitor(self.children[child_index].as_ref());
                    }
                }
                self.metrics.spatial_index_checks.set(
                    self.metrics.spatial_index_checks.get()
                        + hits.index_bounds_checked
                        + hits.target_bounds_checked,
                );
            }

            fn has_overlapping_hit_targets(&self) -> bool {
                self.overlapping_children
            }
        }

        impl LayoutElement for ComparisonElement {
            fn pos_start_end(&self) -> Option<(aimer_attribute::Vec2d, aimer_attribute::Vec2d)> {
                self.metrics
                    .layout_bound_reads
                    .set(self.metrics.layout_bound_reads.get() + 1);
                let bounds = self.bounds.get()?;
                Some((
                    (bounds.x, bounds.y).into(),
                    (bounds.x + bounds.width, bounds.y + bounds.height).into(),
                ))
            }
        }

        impl FrameworkDrawable for ComparisonElement {
            fn update(&self, _ctx: &BuildContext) {}
        }

        impl Rebuildable for ComparisonElement {}

        struct FrameworkCase {
            root: FrameworkAnyElement,
            dispatcher: EventDispatcher,
            metrics: Rc<DispatchMetrics>,
            focus_nodes: Vec<(ElementId, FocusNode)>,
            bound_cells: Vec<Rc<Cell<Option<Bounds>>>>,
            event_targets: Vec<(ElementId, EventTargetId)>,
            spatial_index: Option<Rc<EventTree>>,
        }

        struct LegacyHitBoundary {
            child: FrameworkAnyElement,
            bounds: Bounds,
            hit_region: Bounds,
            metrics: Rc<DispatchMetrics>,
        }

        impl VisitorElement for LegacyHitBoundary {
            fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn FrameworkElement)) {
                visitor(self.child.as_ref());
            }

            fn debug_name(&self) -> &'static str {
                "LegacyHitBoundary"
            }
        }

        impl FrameworkEventElement for LegacyHitBoundary {
            fn on_event(&self, _event: &FrameworkEvent) -> EventResult {
                self.metrics
                    .callback_visits
                    .set(self.metrics.callback_visits.get() + 1);
                EventResult::ignored()
            }

            fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn FrameworkElement)) {
                visitor(self.child.as_ref());
            }

            fn hit_test_children_at<'a>(
                &'a self,
                pos: aimer_attribute::Vec2d,
                visitor: &mut dyn FnMut(&'a dyn FrameworkElement),
            ) {
                if self.hit_region.is_inside(pos.x, pos.y) {
                    visitor(self.child.as_ref());
                }
            }
        }

        impl LayoutElement for LegacyHitBoundary {
            fn pos_start_end(&self) -> Option<(aimer_attribute::Vec2d, aimer_attribute::Vec2d)> {
                Some((
                    (self.bounds.x, self.bounds.y).into(),
                    (
                        self.bounds.x + self.bounds.width,
                        self.bounds.y + self.bounds.height,
                    )
                        .into(),
                ))
            }
        }

        impl FrameworkDrawable for LegacyHitBoundary {
            fn update(&self, _ctx: &BuildContext) {}
        }

        impl Rebuildable for LegacyHitBoundary {}

        impl FrameworkCase {
            fn dispatch(
                &mut self,
                pos: aimer_attribute::Vec2d,
                event: &FrameworkEvent,
            ) -> EventResult {
                self.dispatcher.dispatch(self.root.as_ref(), pos, event)
            }

            fn focused_element(&self) -> Option<ElementId> {
                self.focus_nodes
                    .iter()
                    .find_map(|(id, node)| node.has_focus().then_some(*id))
            }

            fn captured_logical_owner(&self, pointer: PointerKey) -> Option<ElementId> {
                let framework_id = self.dispatcher.captured_owner(pointer)?;
                self.metrics
                    .framework_ids
                    .borrow()
                    .iter()
                    .find_map(|(logical, actual)| {
                        (*actual == framework_id).then_some(ElementId(*logical))
                    })
            }

            fn request_focus(&self, id: ElementId) {
                self.focus_nodes
                    .iter()
                    .find_map(|(node_id, node)| (*node_id == id).then_some(node))
                    .expect("event participant has a focus node")
                    .request_focus();
            }

            fn request_unfocus(&self, id: ElementId) {
                self.focus_nodes
                    .iter()
                    .find_map(|(node_id, node)| (*node_id == id).then_some(node))
                    .expect("event participant has a focus node")
                    .unfocus();
            }

            fn update_target_bounds(&self, id: ElementId, bounds: Bounds) {
                if let Some(index) = &self.spatial_index {
                    let target_index = self
                        .event_targets
                        .binary_search_by_key(&id.index(), |(node_id, _)| node_id.index())
                        .expect("event participant has an index entry");
                    let (_, target) = self.event_targets[target_index];
                    self.bound_cells[target_index].set(Some(bounds));
                    assert!(index.update_bounds(
                        target,
                        1.0,
                        bounds.x,
                        bounds.y,
                        bounds.width,
                        bounds.height,
                    ));
                } else {
                    self.bound_cells[id.index()].set(Some(bounds));
                }
            }
        }

        fn element_bounds(tree: &ElementTree) -> Vec<Option<Bounds>> {
            let mut bounds = vec![None; tree.len()];
            for event in tree.event_tree().elements() {
                bounds[event.element_id().index()] = event.bounds();
            }
            bounds
        }

        fn copy_event_index(tree: &ElementTree) -> Rc<EventTree> {
            let mut index = EventTree::new();
            for event in tree.event_tree().elements() {
                let target = index.register_under(event.element_id(), event.parent());
                if let Some(bounds) = event.bounds() {
                    assert!(index.update_bounds(
                        target,
                        1.0,
                        bounds.x,
                        bounds.y,
                        bounds.width,
                        bounds.height,
                    ));
                }
            }
            Rc::new(index)
        }

        fn make_element(
            logical_id: Option<usize>,
            participant: bool,
            event_child_targets: Vec<EventTargetId>,
            children: Vec<FrameworkAnyElement>,
            bounds: Option<Bounds>,
            focus: Option<FocusNode>,
            capture_target: Option<ElementId>,
            metrics: Rc<DispatchMetrics>,
            spatial_index: Option<Rc<EventTree>>,
            overlapping_children: bool,
        ) -> FrameworkAnyElement {
            let element = ComparisonElement {
                logical_id,
                participant,
                event_child_targets,
                children,
                bounds: Rc::new(Cell::new(bounds)),
                focus,
                capture_target,
                metrics: metrics.clone(),
                spatial_index,
                overlapping_children,
            }
            .boxed();
            record_framework_id(&metrics, logical_id.filter(|_| participant), &element);
            element
        }

        fn record_framework_id(
            metrics: &DispatchMetrics,
            logical_id: Option<usize>,
            element: &FrameworkAnyElement,
        ) {
            if let Some(logical_id) = logical_id {
                metrics
                    .framework_ids
                    .borrow_mut()
                    .insert(logical_id, element.as_ref().id());
            }
        }

        fn build_general_node(
            tree: &ElementTree,
            bound_cells: &[Rc<Cell<Option<Bounds>>>],
            focus_nodes: &[Option<FocusNode>],
            metrics: Rc<DispatchMetrics>,
            capture_target: Option<ElementId>,
            id: ElementId,
        ) -> FrameworkAnyElement {
            let node = tree.node(id).expect("fixture tree links valid elements");
            let participant = node.event_target.is_some();
            let children = tree
                .children(id)
                .expect("fixture node exists")
                .iter()
                .copied()
                .map(|child| {
                    build_general_node(
                        tree,
                        bound_cells,
                        focus_nodes,
                        metrics.clone(),
                        capture_target,
                        child,
                    )
                })
                .collect();
            let element = ComparisonElement {
                logical_id: participant.then_some(id.index()),
                participant,
                event_child_targets: Vec::new(),
                children,
                bounds: bound_cells[id.index()].clone(),
                focus: focus_nodes[id.index()].clone(),
                capture_target,
                metrics: metrics.clone(),
                spatial_index: None,
                overlapping_children: false,
            }
            .boxed();
            record_framework_id(&metrics, participant.then_some(id.index()), &element);
            element
        }

        fn build_sparse_node(
            target: EventTargetId,
            event_index: &Rc<EventTree>,
            bound_cells: &[Rc<Cell<Option<Bounds>>>],
            focus_nodes: &[FocusNode],
            element_ids: &[ElementId],
            metrics: &Rc<DispatchMetrics>,
            capture_target: Option<ElementId>,
        ) -> FrameworkAnyElement {
            let event = event_index.get(target).expect("event target exists");
            let event_child_targets = event.children().to_vec();
            let children = event_child_targets
                .iter()
                .copied()
                .map(|child| {
                    build_sparse_node(
                        child,
                        event_index,
                        bound_cells,
                        focus_nodes,
                        element_ids,
                        metrics,
                        capture_target,
                    )
                })
                .collect();
            let id = element_ids[target.0];
            let element = ComparisonElement {
                logical_id: Some(id.index()),
                participant: true,
                event_child_targets,
                children,
                bounds: bound_cells[target.0].clone(),
                focus: Some(focus_nodes[target.0].clone()),
                capture_target,
                metrics: metrics.clone(),
                spatial_index: Some(event_index.clone()),
                overlapping_children: false,
            }
            .boxed();
            record_framework_id(metrics, Some(id.index()), &element);
            element
        }

        fn build_general_case(
            tree: &ElementTree,
            bounds: &[Option<Bounds>],
            capture_target: Option<ElementId>,
        ) -> FrameworkCase {
            let metrics = Rc::new(DispatchMetrics::default());
            let bound_cells = bounds
                .iter()
                .copied()
                .map(|bounds| Rc::new(Cell::new(bounds)))
                .collect::<Vec<_>>();
            let mut focus_by_element = vec![None; tree.len()];
            let mut focus_nodes = Vec::with_capacity(tree.event_tree().elements().len());
            let mut event_targets = Vec::with_capacity(tree.event_tree().elements().len());
            for (target_index, event) in tree.event_tree().elements().iter().enumerate() {
                let id = event.element_id();
                let focus = FocusNode::new();
                focus_by_element[id.index()] = Some(focus.clone());
                focus_nodes.push((id, focus));
                event_targets.push((id, EventTargetId(target_index)));
            }
            let children = tree
                .roots()
                .iter()
                .copied()
                .map(|id| {
                    build_general_node(
                        tree,
                        &bound_cells,
                        &focus_by_element,
                        metrics.clone(),
                        capture_target,
                        id,
                    )
                })
                .collect();
            let root = make_element(
                None,
                false,
                Vec::new(),
                children,
                None,
                None,
                capture_target,
                metrics.clone(),
                None,
                true,
            );
            FrameworkCase {
                root,
                dispatcher: EventDispatcher::new(),
                metrics,
                focus_nodes,
                bound_cells,
                event_targets,
                spatial_index: None,
            }
        }

        fn build_sparse_case(
            tree: &ElementTree,
            capture_target: Option<ElementId>,
        ) -> FrameworkCase {
            let metrics = Rc::new(DispatchMetrics::default());
            let spatial_index = copy_event_index(tree);
            let mut focus_nodes = Vec::with_capacity(tree.event_tree().elements().len());
            let mut focus_by_target = Vec::with_capacity(tree.event_tree().elements().len());
            let mut bound_cells = Vec::with_capacity(tree.event_tree().elements().len());
            let mut event_targets = Vec::with_capacity(tree.event_tree().elements().len());
            let mut element_ids = Vec::with_capacity(tree.event_tree().elements().len());

            for (target_index, event) in tree.event_tree().elements().iter().enumerate() {
                let id = event.element_id();
                let target = EventTargetId(target_index);
                let focus = FocusNode::new();
                focus_by_target.push(focus.clone());
                focus_nodes.push((id, focus));
                bound_cells.push(Rc::new(Cell::new(event.bounds())));
                event_targets.push((id, target));
                element_ids.push(id);
            }

            let event_child_targets = tree.event_tree().roots().to_vec();
            let children = event_child_targets
                .iter()
                .copied()
                .map(|target| {
                    build_sparse_node(
                        target,
                        &spatial_index,
                        &bound_cells,
                        &focus_by_target,
                        &element_ids,
                        &metrics,
                        capture_target,
                    )
                })
                .collect();
            let root = make_element(
                None,
                false,
                event_child_targets,
                children,
                None,
                None,
                capture_target,
                metrics.clone(),
                Some(spatial_index.clone()),
                true,
            );
            FrameworkCase {
                root,
                dispatcher: EventDispatcher::new(),
                metrics,
                focus_nodes,
                bound_cells,
                event_targets,
                spatial_index: Some(spatial_index),
            }
        }

        fn build_legacy_boundary_case(indexed_child: bool) -> FrameworkCase {
            let metrics = Rc::new(DispatchMetrics::default());
            let id = ElementId(700);
            let child_bounds = Rc::new(Cell::new(Some(Bounds::new(0.0, 0.0, 100.0, 100.0))));
            let child = ComparisonElement {
                logical_id: Some(id.index()),
                participant: true,
                event_child_targets: Vec::new(),
                children: Vec::new(),
                bounds: child_bounds,
                focus: None,
                capture_target: None,
                metrics: metrics.clone(),
                spatial_index: indexed_child.then(|| Rc::new(EventTree::new())),
                overlapping_children: false,
            }
            .boxed();
            let root = LegacyHitBoundary {
                child,
                bounds: Bounds::new(0.0, 0.0, 100.0, 100.0),
                hit_region: Bounds::new(0.0, 0.0, 20.0, 20.0),
                metrics: metrics.clone(),
            }
            .boxed();
            FrameworkCase {
                root,
                dispatcher: EventDispatcher::new(),
                metrics,
                focus_nodes: Vec::new(),
                bound_cells: Vec::new(),
                event_targets: Vec::new(),
                spatial_index: None,
            }
        }

        fn comparison_pair(
            tree: &ElementTree,
            bounds: &[Option<Bounds>],
            capture_target: Option<ElementId>,
        ) -> (FrameworkCase, FrameworkCase) {
            (
                build_general_case(tree, bounds, capture_target),
                build_sparse_case(tree, capture_target),
            )
        }

        fn assert_dispatch_parity(
            general: &mut FrameworkCase,
            sparse: &mut FrameworkCase,
            pos: aimer_attribute::Vec2d,
            event: FrameworkEvent,
        ) -> EventResult {
            general.metrics.reset();
            sparse.metrics.reset();
            let general_result = general.dispatch(pos, &event);
            let sparse_result = sparse.dispatch(pos, &event);
            assert_eq!(general_result, sparse_result);
            assert_eq!(
                *general.metrics.deliveries.borrow(),
                *sparse.metrics.deliveries.borrow(),
            );
            assert_eq!(
                *general.metrics.effects.borrow(),
                *sparse.metrics.effects.borrow(),
            );
            assert_eq!(general.focused_element(), sparse.focused_element());
            assert_eq!(
                general.dispatcher.capture_count(),
                sparse.dispatcher.capture_count()
            );
            if let Some(pointer) = event_pointer_key(&event) {
                assert_eq!(
                    general.captured_logical_owner(pointer),
                    sparse.captured_logical_owner(pointer),
                );
            }
            general_result
        }

        fn event_pointer_key(event: &FrameworkEvent) -> Option<PointerKey> {
            match event {
                FrameworkEvent::PointerDown(info)
                | FrameworkEvent::PointerMove(info)
                | FrameworkEvent::PointerUp(info) => Some(PointerKey::new(info.source, info.id)),
                FrameworkEvent::PointerExited(source, id) => Some(PointerKey::new(*source, *id)),
                _ => None,
            }
        }

        fn showcase_bounds(fixture: &ShowcaseFixture) -> Vec<Option<Bounds>> {
            fixture
                .general_bounds
                .iter()
                .map(CacheBounds::get_bounds)
                .collect()
        }

        fn pointer(pos: aimer_attribute::Vec2d) -> PointerInfo {
            PointerInfo::new(pos, PointerSource::Mouse, 0, PointerButton::Primary)
        }

        #[test]
        fn real_dispatcher_routes_pointer_capture_focus_keys_and_broadcast_equivalently() {
            let fixture = build_showcase_fixture(1, 0.0);
            let bounds = showcase_bounds(&fixture);
            let (capture_target, rect) = fixture.button_targets[0];
            let point = (rect.x + 20.0, rect.y + rect.height / 2.0).into();
            let outside = (-100.0, -100.0).into();
            let (mut general, mut sparse) =
                comparison_pair(&fixture.tree, &bounds, Some(capture_target));
            let pointer_key = PointerKey::new(PointerSource::Mouse, 0);
            begin_event_frame();

            let result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                point,
                FrameworkEvent::PointerDown(pointer(point)),
            );
            assert!(result.is_consumed());
            assert!(result.needs_redraw());
            assert!(general.dispatcher.captured_owner(pointer_key).is_some());
            assert_eq!(general.focused_element(), Some(capture_target));
            assert!(
                sparse.metrics.callback_visits.get() < general.metrics.callback_visits.get(),
                "sparse event view should skip ordinary wrapper callbacks"
            );

            // A touch pointer with the same numeric ID owns an independent
            // capture from the mouse pointer.
            let touch_key = PointerKey::new(PointerSource::Touch, 0);
            let touch_down = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                point,
                FrameworkEvent::PointerDown(PointerInfo::new(
                    point,
                    PointerSource::Touch,
                    0,
                    PointerButton::Primary,
                )),
            );
            assert!(touch_down.is_consumed());
            assert_eq!(general.dispatcher.capture_count(), 2);
            assert_eq!(
                general.captured_logical_owner(touch_key),
                Some(capture_target)
            );
            assert_eq!(
                sparse.captured_logical_owner(touch_key),
                Some(capture_target)
            );

            let touch_up = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                outside,
                FrameworkEvent::PointerUp(PointerInfo::new(
                    outside,
                    PointerSource::Touch,
                    0,
                    PointerButton::Primary,
                )),
            );
            assert!(touch_up.is_consumed());
            assert_eq!(general.dispatcher.capture_count(), 1);
            assert_eq!(sparse.dispatcher.capture_count(), 1);

            let move_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                outside,
                FrameworkEvent::PointerMove(pointer(outside)),
            );
            assert!(!move_result.is_consumed());
            assert!(general.dispatcher.is_captured(pointer_key));
            let up_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                outside,
                FrameworkEvent::PointerUp(pointer(outside)),
            );
            assert!(up_result.is_consumed());
            assert!(!general.dispatcher.is_captured(pointer_key));

            let next_focus = fixture.button_targets[1].0;
            general.request_focus(next_focus);
            sparse.request_focus(next_focus);
            let _ =
                assert_dispatch_parity(&mut general, &mut sparse, outside, FrameworkEvent::Cancel);
            assert_eq!(general.focused_element(), Some(next_focus));

            let text_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                outside,
                FrameworkEvent::TextInput {
                    text: "event tree".into(),
                    action: KeyAction::Pressed,
                    modifiers: Modifiers::default(),
                },
            );
            assert!(text_result.is_consumed());

            for event in [
                FrameworkEvent::CharInput {
                    ch: 'é',
                    action: KeyAction::Pressed,
                    modifiers: Modifiers::default(),
                },
                FrameworkEvent::ImePreedit {
                    text: "composition".into(),
                    cursor: Some((0, 4)),
                },
                FrameworkEvent::TextEditingDelta(TextEditingDelta {
                    session_id: 7,
                    revision: 3,
                    replacement: NativeTextRange::new(0, 1),
                    replacement_text: "x".into(),
                    selection: NativeTextRange::new(1, 1),
                    composing: None,
                }),
            ] {
                let _ = assert_dispatch_parity(&mut general, &mut sparse, outside, event);
            }

            let tab_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                outside,
                FrameworkEvent::KeyInput {
                    key: NamedKey::Tab,
                    action: KeyAction::Pressed,
                    modifiers: Modifiers::default(),
                },
            );
            assert!(tab_result.is_consumed());
            assert_ne!(general.focused_element(), Some(next_focus));

            let reverse_tab_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                outside,
                FrameworkEvent::KeyInput {
                    key: NamedKey::Tab,
                    action: KeyAction::Pressed,
                    modifiers: Modifiers {
                        shift: true,
                        ..Modifiers::default()
                    },
                },
            );
            assert!(reverse_tab_result.is_consumed());

            let (_, header_rect) = fixture.header_targets[0];
            let header_point = (header_rect.x + 20.0, header_rect.y + 10.0).into();
            for event in [
                FrameworkEvent::HoveredFile {
                    path: PathBuf::from("report.txt"),
                    pos: Some(header_point),
                },
                FrameworkEvent::HoveredFileMoved {
                    paths: Arc::<[PathBuf]>::from(Vec::new()),
                    pos: header_point,
                },
                FrameworkEvent::DroppedFile {
                    path: PathBuf::from("report.txt"),
                    pos: Some(header_point),
                },
                FrameworkEvent::DragOver {
                    pos: header_point,
                    source: PointerSource::Mouse,
                    id: 0,
                },
                FrameworkEvent::DragDrop {
                    pos: header_point,
                    source: PointerSource::Mouse,
                    id: 0,
                },
            ] {
                let _ = assert_dispatch_parity(&mut general, &mut sparse, header_point, event);
            }

            general.metrics.reset();
            sparse.metrics.reset();
            let broadcast = FrameworkEvent::HoveredFileCancelled;
            let general_result = broadcast_event(general.root.as_ref(), &broadcast);
            let sparse_result = broadcast_event(sparse.root.as_ref(), &broadcast);
            assert_eq!(general_result, sparse_result);
            assert_eq!(general_result, EventResult::redraw());
            assert_eq!(
                *general.metrics.deliveries.borrow(),
                *sparse.metrics.deliveries.borrow(),
            );
            assert_eq!(
                *general.metrics.effects.borrow(),
                *sparse.metrics.effects.borrow(),
            );
            assert_eq!(general.metrics.deliveries.borrow().len(), 62);

            // The framework's direct focused-event helper also sees the same
            // ordered participant projection as the dispatcher.
            general.metrics.reset();
            sparse.metrics.reset();
            let focused = FrameworkEvent::TextInput {
                text: "focused helper".into(),
                action: KeyAction::Pressed,
                modifiers: Modifiers::default(),
            };
            let general_result = dispatch_focused_event(general.root.as_ref(), &focused);
            let sparse_result = dispatch_focused_event(sparse.root.as_ref(), &focused);
            assert_eq!(general_result, sparse_result);
            assert_eq!(
                *general.metrics.deliveries.borrow(),
                *sparse.metrics.deliveries.borrow(),
            );
            assert_eq!(
                *general.metrics.effects.borrow(),
                *sparse.metrics.effects.borrow(),
            );

            let focused = general
                .focused_element()
                .expect("directed events focus a field");
            general.request_unfocus(focused);
            sparse.request_unfocus(focused);
            let _ =
                assert_dispatch_parity(&mut general, &mut sparse, outside, FrameworkEvent::Cancel);
            assert_eq!(general.focused_element(), None);
            assert_eq!(sparse.focused_element(), None);

            let down_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                point,
                FrameworkEvent::PointerDown(pointer(point)),
            );
            assert!(down_result.is_consumed());
            let cancel_result =
                assert_dispatch_parity(&mut general, &mut sparse, outside, FrameworkEvent::Cancel);
            assert!(cancel_result.is_consumed());
            assert_eq!(general.dispatcher.capture_count(), 0);
            assert_eq!(sparse.dispatcher.capture_count(), 0);
        }

        #[test]
        fn nested_event_participants_keep_route_and_broadcast_order() {
            let mut tree = ElementTree::new();
            let root = tree
                .insert(
                    None,
                    AnyElement::new(StressElement),
                    EventParticipation::None,
                )
                .unwrap();
            let parent = tree
                .insert(
                    Some(root),
                    AnyElement::new(StressElement),
                    EventParticipation::HandlesEvents,
                )
                .unwrap();
            let child = tree
                .insert(
                    Some(parent),
                    AnyElement::new(StressElement),
                    EventParticipation::HandlesEvents,
                )
                .unwrap();
            let grandchild = tree
                .insert(
                    Some(child),
                    AnyElement::new(StressElement),
                    EventParticipation::HandlesEvents,
                )
                .unwrap();
            assert!(tree.update_event_bounds(parent, 1.0, 0.0, 0.0, 30.0, 30.0));
            assert!(tree.update_event_bounds(child, 1.0, 5.0, 5.0, 10.0, 10.0));
            assert!(tree.update_event_bounds(grandchild, 1.0, 5.0, 5.0, 10.0, 10.0));
            let bounds = element_bounds(&tree);
            let point = (5.0, 5.0).into();
            let parent_only = (25.0, 25.0).into();
            let (mut general, mut sparse) = comparison_pair(&tree, &bounds, Some(grandchild));
            begin_event_frame();

            let down_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                point,
                FrameworkEvent::PointerDown(pointer(point)),
            );
            assert!(down_result.is_consumed());
            let move_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                parent_only,
                FrameworkEvent::PointerMove(pointer(parent_only)),
            );
            assert!(!move_result.is_consumed());
            assert!(move_result.needs_redraw());
            assert_eq!(
                *general.metrics.deliveries.borrow(),
                vec![
                    (
                        grandchild.index(),
                        std::mem::discriminant(&FrameworkEvent::PointerMove(pointer(parent_only)))
                    ),
                    (
                        parent.index(),
                        std::mem::discriminant(&FrameworkEvent::DragOver {
                            pos: parent_only,
                            source: PointerSource::Mouse,
                            id: 0,
                        })
                    ),
                ]
            );
            let up_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                parent_only,
                FrameworkEvent::PointerUp(pointer(parent_only)),
            );
            assert!(up_result.is_consumed());
            assert_eq!(general.dispatcher.capture_count(), 0);
            assert_eq!(
                *general.metrics.deliveries.borrow(),
                vec![
                    (
                        grandchild.index(),
                        std::mem::discriminant(&FrameworkEvent::PointerUp(pointer(parent_only)))
                    ),
                    (
                        parent.index(),
                        std::mem::discriminant(&FrameworkEvent::DragDrop {
                            pos: parent_only,
                            source: PointerSource::Mouse,
                            id: 0,
                        })
                    ),
                ]
            );

            general.metrics.reset();
            sparse.metrics.reset();
            let file_cancel = FrameworkEvent::HoveredFileCancelled;
            let general_result = broadcast_event(general.root.as_ref(), &file_cancel);
            let sparse_result = broadcast_event(sparse.root.as_ref(), &file_cancel);
            assert_eq!(general_result, sparse_result);
            assert_eq!(
                general
                    .metrics
                    .deliveries
                    .borrow()
                    .iter()
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>(),
                vec![grandchild.index(), child.index(), parent.index()]
            );
            assert_eq!(
                *general.metrics.deliveries.borrow(),
                *sparse.metrics.deliveries.borrow(),
            );
        }

        #[test]
        fn overlapping_roots_preserve_topmost_paint_order() {
            let fixture = build_showcase_fixture_with_pane_stride(2, 0.0, 0.0);
            let bounds = showcase_bounds(&fixture);
            let topmost = fixture.button_targets[SHOWCASE_BUTTON_COUNT].0;
            let rect = fixture.button_targets[0].1;
            let point = (rect.x + 20.0, rect.y + rect.height / 2.0).into();
            let (mut general, mut sparse) = comparison_pair(&fixture.tree, &bounds, None);
            begin_event_frame();

            let result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                point,
                FrameworkEvent::PointerDown(pointer(point)),
            );
            assert!(result.is_consumed());
            let deliveries = general.metrics.deliveries.borrow();
            assert_eq!(deliveries.len(), 2);
            assert_eq!(deliveries[0].0, topmost.index());
            assert_eq!(deliveries[1].0, topmost.index());
            assert_eq!(
                deliveries[0].1,
                std::mem::discriminant(&FrameworkEvent::PointerDown(pointer(point)))
            );
            assert_eq!(
                deliveries[1].1,
                std::mem::discriminant(&FrameworkEvent::FocusGained)
            );
        }

        #[test]
        fn overlapping_non_consuming_targets_continue_to_lower_paint_order() {
            let fixture = build_showcase_fixture_with_pane_stride(2, 0.0, 0.0);
            let bounds = showcase_bounds(&fixture);
            let lower = fixture.button_targets[0].0;
            let topmost = fixture.button_targets[SHOWCASE_BUTTON_COUNT].0;
            let rect = fixture.button_targets[0].1;
            let point = (rect.x + 20.0, rect.y + rect.height / 2.0).into();
            let (mut general, mut sparse) = comparison_pair(&fixture.tree, &bounds, None);
            begin_event_frame();

            let result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                point,
                FrameworkEvent::PointerMove(pointer(point)),
            );

            assert!(!result.is_consumed());
            let deliveries = general.metrics.deliveries.borrow();
            let topmost_order = deliveries
                .iter()
                .position(|(id, event)| {
                    *id == topmost.index()
                        && *event
                            == std::mem::discriminant(&FrameworkEvent::PointerMove(pointer(point)))
                })
                .expect("topmost overlapping target receives the move");
            let lower_order = deliveries
                .iter()
                .position(|(id, event)| {
                    *id == lower.index()
                        && *event
                            == std::mem::discriminant(&FrameworkEvent::PointerMove(pointer(point)))
                })
                .expect("non-consuming topmost target does not hide the lower target");
            assert!(topmost_order < lower_order);
            assert_eq!(
                deliveries.as_slice(),
                sparse.metrics.deliveries.borrow().as_slice(),
            );
        }

        #[test]
        fn moved_layout_bounds_update_the_handler_and_spatial_index_together() {
            let mut tree = ElementTree::new();
            let root = tree
                .insert(
                    None,
                    AnyElement::new(StressElement),
                    EventParticipation::None,
                )
                .unwrap();
            let target = tree
                .insert(
                    Some(root),
                    AnyElement::new(StressElement),
                    EventParticipation::HandlesEvents,
                )
                .unwrap();
            assert!(tree.update_event_bounds(target, 1.0, 0.0, 0.0, 10.0, 10.0));
            let bounds = element_bounds(&tree);
            let (general, sparse) = comparison_pair(&tree, &bounds, None);
            let moved = Bounds::new(40.0, 40.0, 10.0, 10.0);
            general.update_target_bounds(target, moved);
            sparse.update_target_bounds(target, moved);

            let old_point = (5.0, 5.0).into();
            let new_point = (45.0, 45.0).into();
            assert_eq!(
                dispatch_event(general.root.as_ref(), old_point, &FrameworkEvent::Cancel),
                dispatch_event(sparse.root.as_ref(), old_point, &FrameworkEvent::Cancel),
            );
            general.metrics.reset();
            sparse.metrics.reset();
            let general_result = dispatch_event(
                general.root.as_ref(),
                new_point,
                &FrameworkEvent::PointerDown(pointer(new_point)),
            );
            let sparse_result = dispatch_event(
                sparse.root.as_ref(),
                new_point,
                &FrameworkEvent::PointerDown(pointer(new_point)),
            );
            assert_eq!(general_result, sparse_result);
            assert_eq!(
                *general.metrics.deliveries.borrow(),
                *sparse.metrics.deliveries.borrow(),
            );
            assert_eq!(
                *general.metrics.effects.borrow(),
                *sparse.metrics.effects.borrow(),
            );
            assert_eq!(
                *sparse.metrics.deliveries.borrow(),
                vec![(
                    target.index(),
                    std::mem::discriminant(&FrameworkEvent::PointerDown(pointer(new_point)))
                )]
            );
        }

        #[test]
        fn indexed_hit_test_boundaries_keep_descendants_inside_parent_branch() {
            let mut general = build_legacy_boundary_case(false);
            let mut sparse = build_legacy_boundary_case(true);
            begin_event_frame();

            // The child itself contains this point, but the legacy parent's
            // custom hit-test boundary excludes it. The sparse descendant must
            // not bypass that boundary.
            let clipped = (50.0, 50.0).into();
            let clipped_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                clipped,
                FrameworkEvent::PointerDown(pointer(clipped)),
            );
            assert_eq!(clipped_result, EventResult::ignored());
            assert!(general.metrics.deliveries.borrow().is_empty());
            assert!(sparse.metrics.deliveries.borrow().is_empty());

            let inside = (10.0, 10.0).into();
            let inside_result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                inside,
                FrameworkEvent::PointerDown(pointer(inside)),
            );
            assert!(inside_result.is_consumed());
            assert_eq!(
                general.metrics.deliveries.borrow().as_slice(),
                &[(
                    700,
                    std::mem::discriminant(&FrameworkEvent::PointerDown(pointer(inside)))
                )],
            );
        }

        #[test]
        fn unbounded_event_targets_remain_hit_eligible_in_both_dispatchers() {
            let mut tree = ElementTree::new();
            let root = tree
                .insert(
                    None,
                    AnyElement::new(StressElement),
                    EventParticipation::None,
                )
                .unwrap();
            let target = tree
                .insert(
                    Some(root),
                    AnyElement::new(StressElement),
                    EventParticipation::HandlesEvents,
                )
                .unwrap();
            let bounds = element_bounds(&tree);
            assert_eq!(bounds[target.index()], None);
            let (mut general, mut sparse) = comparison_pair(&tree, &bounds, None);
            begin_event_frame();

            let point = (-10_000.0, 25_000.0).into();
            let result = assert_dispatch_parity(
                &mut general,
                &mut sparse,
                point,
                FrameworkEvent::DragOver {
                    pos: point,
                    source: PointerSource::Mouse,
                    id: 0,
                },
            );

            assert!(!result.is_consumed());
            assert!(result.needs_redraw());
            assert_eq!(
                general.metrics.deliveries.borrow().as_slice(),
                &[(
                    target.index(),
                    std::mem::discriminant(&FrameworkEvent::DragOver {
                        pos: point,
                        source: PointerSource::Mouse,
                        id: 0,
                    })
                )],
            );
            assert!(
                sparse.metrics.callback_visits.get() < general.metrics.callback_visits.get(),
                "the indexed route should skip the ordinary wrapper callback"
            );
        }

        #[test]
        fn showcase_event_tree_root_count_scales_with_all_event_participants() {
            for copies in [1, 100] {
                let fixture = build_showcase_fixture(copies, 0.0);
                let roots = fixture.tree.event_tree().roots().len();
                let targets = fixture.tree.event_tree().elements().len();
                assert_eq!(roots, targets);
                assert_eq!(
                    roots,
                    copies * (SHOWCASE_BUTTON_COUNT + SHOWCASE_CATEGORY_BUTTON_COUNTS.len())
                );
            }
        }

        fn timed_dispatch(
            case: &mut FrameworkCase,
            pos: aimer_attribute::Vec2d,
            event: &FrameworkEvent,
            iterations: usize,
        ) -> (std::time::Duration, usize, usize, EventResult) {
            for _ in 0..3 {
                let _ = case.dispatch(pos, event);
            }
            case.metrics.reset();
            let started = Instant::now();
            let mut result = EventResult::ignored();
            for _ in 0..iterations {
                result = black_box(case.dispatch(pos, event));
            }
            (
                started.elapsed(),
                case.metrics.callback_visits.get(),
                case.metrics.layout_bound_reads.get(),
                result,
            )
        }

        #[test]
        #[ignore = "manual debug-profile end-to-end comparison; run with --ignored --nocapture"]
        fn compare_real_framework_dispatch_build_layout_and_warm_routes() {
            for copies in [1, 100] {
                let model_started = Instant::now();
                let fixture = build_showcase_fixture(copies, 0.0);
                let model_build = model_started.elapsed();
                let bounds = showcase_bounds(&fixture);
                let (capture_target, rect) =
                    fixture.button_targets[(copies - 1) * SHOWCASE_BUTTON_COUNT];
                let point: aimer_attribute::Vec2d =
                    (rect.x + 20.0, rect.y + rect.height / 2.0).into();

                let general_started = Instant::now();
                let mut general = build_general_case(&fixture.tree, &bounds, Some(capture_target));
                let general_build = general_started.elapsed();
                let sparse_started = Instant::now();
                let mut sparse = build_sparse_case(&fixture.tree, Some(capture_target));
                let sparse_build = sparse_started.elapsed();

                let update_iterations = 2_000;
                let base = bounds[capture_target.index()].expect("button is laid out");
                let started = Instant::now();
                for index in 0..update_iterations {
                    general.update_target_bounds(
                        capture_target,
                        Bounds::new(base.x + (index % 2) as f32, base.y, base.width, base.height),
                    );
                }
                let general_update = started.elapsed();
                let started = Instant::now();
                for index in 0..update_iterations {
                    sparse.update_target_bounds(
                        capture_target,
                        Bounds::new(base.x + (index % 2) as f32, base.y, base.width, base.height),
                    );
                }
                let sparse_update = started.elapsed();

                let event = FrameworkEvent::HoveredFileMoved {
                    paths: Arc::<[PathBuf]>::from(Vec::new()),
                    pos: point,
                };
                begin_event_frame();
                let iterations = 2_000;
                let general_route = timed_dispatch(&mut general, point, &event, iterations);
                let sparse_route = timed_dispatch(&mut sparse, point, &event, iterations);

                assert_eq!(general_route.3, sparse_route.3);
                assert_eq!(
                    general.metrics.deliveries.borrow().as_slice(),
                    sparse.metrics.deliveries.borrow().as_slice()
                );
                println!(
                    "showcase copies={copies}, nodes={}, event participants={}, routes={iterations}",
                    fixture.tree.len(),
                    fixture.tree.event_tree().elements().len(),
                );
                println!(
                    "construct model+registration={:.2} ms, general framework tree={:.2} ms, sparse event view={:.2} ms",
                    model_build.as_secs_f64() * 1_000.0,
                    general_build.as_secs_f64() * 1_000.0,
                    sparse_build.as_secs_f64() * 1_000.0,
                );
                println!(
                    "changed-target bounds: general={:.2} µs/update, event bounds+index refit={:.2} µs/update",
                    general_update.as_secs_f64() * 1_000_000.0 / update_iterations as f64,
                    sparse_update.as_secs_f64() * 1_000_000.0 / update_iterations as f64,
                );
                println!(
                    "warmed EventDispatcher: general callbacks={}, bound reads={}, {:.2} µs/route; sparse callbacks={}, bound reads={}, {:.2} µs/route",
                    general_route.1,
                    general_route.2,
                    general_route.0.as_secs_f64() * 1_000_000.0 / iterations as f64,
                    sparse_route.1,
                    sparse_route.2,
                    sparse_route.0.as_secs_f64() * 1_000_000.0 / iterations as f64,
                );
            }

            let stress_model_started = Instant::now();
            let (tree, _) = stress_tree();
            let stress_model_build = stress_model_started.elapsed();
            let bounds = element_bounds(&tree);
            let event_ids = tree
                .event_tree()
                .elements()
                .iter()
                .map(EventElement::element_id)
                .collect::<Vec<_>>();
            let capture_target = event_ids[0];
            let build_started = Instant::now();
            let mut general = build_general_case(&tree, &bounds, Some(capture_target));
            let general_build = build_started.elapsed();
            let build_started = Instant::now();
            let mut sparse = build_sparse_case(&tree, Some(capture_target));
            let sparse_build = build_started.elapsed();
            let point: aimer_attribute::Vec2d = (2.0, 2.0).into();
            let event = FrameworkEvent::HoveredFileMoved {
                paths: Arc::<[PathBuf]>::from(Vec::new()),
                pos: point,
            };
            begin_event_frame();
            let iterations = 3;
            let general_route = timed_dispatch(&mut general, point, &event, iterations);
            let sparse_route = timed_dispatch(&mut sparse, point, &event, iterations);
            assert_eq!(general_route.3, sparse_route.3);
            assert_eq!(
                general.metrics.deliveries.borrow().as_slice(),
                sparse.metrics.deliveries.borrow().as_slice()
            );
            println!(
                "stress nodes={}, event participants={}, routes={iterations}",
                tree.len(),
                tree.event_tree().elements().len(),
            );
            println!(
                "construct ElementTree+registration={:.2} ms, general framework tree={:.2} ms, sparse event view={:.2} ms",
                stress_model_build.as_secs_f64() * 1_000.0,
                general_build.as_secs_f64() * 1_000.0,
                sparse_build.as_secs_f64() * 1_000.0,
            );
            println!(
                "warmed EventDispatcher: general callbacks={}, bound reads={}, {:.2} µs/route; sparse callbacks={}, bound reads={}, {:.2} µs/route",
                general_route.1,
                general_route.2,
                general_route.0.as_secs_f64() * 1_000_000.0 / iterations as f64,
                sparse_route.1,
                sparse_route.2,
                sparse_route.0.as_secs_f64() * 1_000_000.0 / iterations as f64,
            );
        }
    }
}
