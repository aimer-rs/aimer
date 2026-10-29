use std::cell::Cell;
use std::ops::Range;

use aimer_attribute::Bounds;
use hashbrown::HashMap;

use crate::components::element::ElementId;

/// Identifies one registered target in an [`EventTree`].
///
/// Elements that do not handle events never receive an event-target ID and do
/// not appear in the event tree.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct EventTargetId(usize);

impl EventTargetId {
    #[inline]
    pub(super) const fn from_index(index: usize) -> Self {
        Self(index)
    }

    #[inline]
    pub(super) const fn index(self) -> usize {
        self.0
    }
}

/// A sparse event-tree entry for one event-handling element.
pub(super) struct EventElement {
    element: ElementId,
    bound: Cell<Option<Bounds>>,
    parent: Option<EventTargetId>,
    parent_branch: Option<ElementId>,
    subtree_end: usize,
    children: Vec<EventTargetId>,
}

impl EventElement {
    /// Returns the ID of the corresponding general-tree element.
    #[inline]
    pub(super) const fn element_id(&self) -> ElementId {
        self.element
    }

    /// Returns the closest event-handling ancestor, if one exists.
    #[inline]
    pub(super) const fn parent(&self) -> Option<EventTargetId> {
        self.parent
    }

    /// Returns the structural child branch below `parent` that contains this
    /// target. Indexed hit-test boundaries use it to retain their own child
    /// filtering while targets below transparent wrappers remain flattened.
    #[inline]
    pub(super) const fn parent_branch(&self) -> Option<ElementId> {
        self.parent_branch
    }

    /// Returns the half-open target-index range occupied by this target and
    /// its descendants.
    #[inline]
    pub(super) fn subtree_range(&self, target: EventTargetId) -> Range<usize> {
        target.index()..self.subtree_end
    }

    /// Returns event-handling descendants whose nearest event ancestor is this
    /// target, in paint order.
    #[inline]
    pub(super) fn children(&self) -> &[EventTargetId] {
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
pub(super) struct EventTree {
    elements: Vec<EventElement>,
    roots: Vec<EventTargetId>,
    boundary_branches: HashMap<(EventTargetId, ElementId), Range<usize>>,
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
                let bounds = elements.get(*target).and_then(|element| element.bound.get());
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
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn register_under(
        &mut self,
        element: ElementId,
        parent: Option<EventTargetId>,
        parent_branch: Option<ElementId>,
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
            bound: Cell::new(None),
            parent,
            parent_branch,
            subtree_end: target.index() + 1,
            children: Vec::new(),
        });
        self.index.register(target.0, &self.elements);
        target
    }

    pub(super) fn finish_subtree(&mut self, target: EventTargetId) {
        let end = self.elements.len();
        if let Some(element) = self.elements.get_mut(target.index()) {
            element.subtree_end = end;
        }
    }

    pub(super) fn register_boundary_branch(
        &mut self,
        boundary: EventTargetId,
        branch: ElementId,
        targets: Range<usize>,
    ) {
        if !targets.is_empty() {
            self.boundary_branches.insert((boundary, branch), targets);
        }
    }

    #[inline]
    pub(super) fn boundary_branch_range(
        &self,
        boundary: EventTargetId,
        branch: ElementId,
    ) -> Option<Range<usize>> {
        self.boundary_branches.get(&(boundary, branch)).cloned()
    }

    #[inline]
    pub(super) fn target_contains(&self, target: EventTargetId, x: f32, y: f32) -> bool {
        self.elements
            .get(target.index())
            .is_some_and(|element| element.bound.get().is_none_or(|bounds| bounds.is_inside(x, y)))
    }

    /// Returns the registered target at `target`, if it still exists.
    #[inline]
    pub(super) fn get(&self, target: EventTargetId) -> Option<&EventElement> {
        self.elements.get(target.0)
    }

    /// Returns the event targets without event-handling ancestors.
    #[inline]
    pub(super) fn roots(&self) -> &[EventTargetId] {
        &self.roots
    }

    pub(super) fn update_bounds(
        &self,
        target: EventTargetId,
        bounds: Option<Bounds>,
    ) -> bool {
        let Some(element) = self.elements.get(target.0) else {
            return false;
        };
        element.bound.set(bounds);
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
    pub(super) fn hit_test(&self, x: f32, y: f32) -> impl Iterator<Item = EventTargetId> + '_ {
        self.hit_test_iter(x, y, true)
    }

    /// Returns targets within `range` that contain `(x, y)`, in topmost-first
    /// order. Boundary child ranges let position-aware containers prune
    /// unbounded offscreen targets before visiting their spatial leaves.
    #[inline]
    pub(super) fn hit_test_range(
        &self,
        range: Range<usize>,
        x: f32,
        y: f32,
    ) -> impl Iterator<Item = EventTargetId> + '_ {
        EventTreeHitTest::new_range(self, x, y, true, range)
    }

    fn hit_test_iter(&self, x: f32, y: f32, reverse_paint_order: bool) -> EventTreeHitTest<'_> {
        EventTreeHitTest::new(self, x, y, reverse_paint_order)
    }

    /// Returns the registered event targets in paint order.
    #[inline]
    pub(super) fn elements(&self) -> &[EventElement] {
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
    target_range: Option<(usize, usize)>,
    #[cfg(test)]
    index_bounds_checked: usize,
    #[cfg(test)]
    target_bounds_checked: usize,
}

impl<'a> EventTreeHitTest<'a> {
    fn new(tree: &'a EventTree, x: f32, y: f32, reverse_paint_order: bool) -> Self {
        Self::new_with_range(tree, x, y, reverse_paint_order, None)
    }

    fn new_range(
        tree: &'a EventTree,
        x: f32,
        y: f32,
        reverse_paint_order: bool,
        range: Range<usize>,
    ) -> Self {
        let start = range.start.min(tree.elements.len());
        let end = range.end.min(tree.elements.len());
        Self::new_with_range(
            tree,
            x,
            y,
            reverse_paint_order,
            Some((start, end)),
        )
    }

    fn new_with_range(
        tree: &'a EventTree,
        x: f32,
        y: f32,
        reverse_paint_order: bool,
        target_range: Option<(usize, usize)>,
    ) -> Self {
        let mut stack = [0; usize::BITS as usize];
        let stack_len = if tree.index.capacity > 0
            && target_range.is_none_or(|(start, end)| start < end)
        {
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
            target_range,
            #[cfg(test)]
            index_bounds_checked: 0,
            #[cfg(test)]
            target_bounds_checked: 0,
        }
    }

    #[inline]
    fn push(&mut self, node: usize) {
        assert!(self.stack_len < self.stack.len(), "event index depth overflow");
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
            if let Some((start, end)) = self.target_range {
                let (node_start, node_end) =
                    event_spatial_node_target_range(node, self.tree.index.capacity);
                if node_end <= start || node_start >= end {
                    continue;
                }
            }
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
                        .get()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::BuildContext;
    use crate::{Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement};

    struct Probe;

    impl VisitorElement for Probe {
        fn debug_name(&self) -> &'static str {
            "EventTreeProbe"
        }
    }

    impl EventElement for Probe {}
    impl LayoutElement for Probe {}
    impl Rebuildable for Probe {}

    impl Drawable for Probe {
        fn update(&self, _ctx: &BuildContext) {}
    }

    #[test]
    fn range_query_skips_unbounded_targets_outside_the_requested_branch() {
        let mut tree = EventTree::new();
        for _ in 0..2_048 {
            let element = Probe.boxed();
            tree.register_under(element.id(), None, None);
        }

        let mut query = EventTreeHitTest::new_range(&tree, 5.0, 5.0, true, 500..505);
        let hits: Vec<_> = query.by_ref().collect();

        assert_eq!(hits, (500..505).rev().map(EventTargetId).collect::<Vec<_>>());
        assert_eq!(query.target_bounds_checked, 5);
        assert!(query.index_bounds_checked < 32);
    }
}

#[inline]
fn event_spatial_node_target_range(node: usize, capacity: usize) -> (usize, usize) {
    let level = usize::BITS as usize - 1 - node.leading_zeros() as usize;
    let first = 1usize << level;
    let span = capacity >> level;
    let start = (node - first) * span;
    (start, start + span)
}
