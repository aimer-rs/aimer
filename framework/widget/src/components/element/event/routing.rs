use crate::CALLED;
use super::*;


pub(super) struct RoutedEventResult {
    pub(super) result: EventResult,
    pub(super) capture_owner: Option<ElementId>,
    pub(super) focus_owner: Option<FocusCandidate<ElementId>>,
}

/// Gathers every focusable attachment of the tree, in traversal order.
pub(super) fn collect_focus_candidates(
    element: &dyn Element,
    candidates: &mut FocusCandidates<ElementId>,
) {
    if let (Some(id), Some(node)) = (element.element_id(), element.focus_node()) {
        candidates.push(FocusCandidate::new(id, node.clone(), element.autofocus()));
    }
    element.focus_children(&mut |child| collect_focus_candidates(child, candidates));
}

pub(super) fn event_pointer_key(event: &ElementEvent) -> Option<PointerKey> {
    match event {
        ElementEvent::PointerDown(pointer)
        | ElementEvent::PointerUp(pointer)
        | ElementEvent::PointerMove(pointer) => Some(PointerKey::new(pointer.source, pointer.id)),
        ElementEvent::PointerExited(source, id) => Some(PointerKey::new(*source, *id)),
        _ => None,
    }
}

#[inline]
pub(super) fn is_indexed_hit_test_boundary(role: EventTreeRole) -> bool {
    matches!(
        role,
        EventTreeRole::Legacy | EventTreeRole::IndexedHitTestBoundary
    )
}

pub(super) fn build_indexed_event_tree(
    element: &dyn Element,
    parent: Option<EventTargetId>,
    parent_branch: Option<ElementId>,
    tree: &mut EventTree,
    target_by_element: &mut HashMap<ElementId, EventTargetId>,
    path_indices: &HashMap<ElementId, usize>,
) {
    let role = element.event_tree_role();

    let is_indexed_target = matches!(
        role,
        EventTreeRole::Legacy
            | EventTreeRole::IndexedTarget
            | EventTreeRole::IndexedHitTestBoundary
    );
    let mut child_branch = parent_branch;
    let parent = if is_indexed_target {
        let id = element
            .element_id()
            .filter(|id| path_indices.contains_key(id))
            .expect("event participants must belong to the retained structural tree");
        assert!(
            parent.is_none() || parent_branch.is_some(),
            "indexed event descendants must be reachable through an event branch"
        );
        let target = tree.register_under(id, parent, parent_branch);
        target_by_element.insert(id, target);
        let bounds = bounds_from_element(element);
        tree.update_bounds(target, bounds);
        child_branch = None;
        Some(target)
    } else {
        parent
    };

    element.event_children(&mut |child| {
        let branch_start = tree.elements().len();
        let parent_branch = if parent.is_some() {
            child_branch.or_else(|| child.element_id())
        } else {
            None
        };
        build_indexed_event_tree(
            child,
            parent,
            parent_branch,
            tree,
            target_by_element,
            path_indices,
        );
        if is_indexed_hit_test_boundary(role)
            && let (Some(boundary), Some(branch)) = (parent, child.element_id())
        {
            tree.register_boundary_branch(boundary, branch, branch_start..tree.elements().len());
        }
    });

    if is_indexed_target
        && let Some(target) = parent
    {
        tree.finish_subtree(target);
    }
}

/// Indexes every element in reusable parent-link and direct-lookup arenas,
/// reporting the innermost focus scope on the way.
///
/// The scope is discovered here rather than by a walk of its own because this
/// walk already visits the whole tree once per structural generation. Each
/// trapping element seen overwrites the previous one, so the last of them in
/// depth-first order wins — which is the innermost scope of the deepest
/// trapping branch, and for siblings the one presented last. The direct
/// entries retain each element pointer and whether that node has structural
/// children, so event and invalidation lookups need not traverse parent paths.
pub(super) fn index_element_links(
    element: &dyn Element,
    parent: Option<usize>,
    child_index: u32,
    links: &mut Vec<ElementPath>,
    path_indices: &mut HashMap<ElementId, usize>,
    indexed_elements: &mut Vec<IndexedElement>,
    scope: &mut Option<ElementId>,
) {
    let link_index = links.len();
    links.push(ElementPath {
        parent,
        child_index,
    });
    indexed_elements.push(IndexedElement {
        element: retained_element_pointer(element),
        has_structural_children: false,
    });

    if let Some(id) = element.element_id() {
        path_indices.insert(id, link_index);
        if element.traps_focus() {
            *scope = Some(id);
        }
    }

    let mut child_index = 0u32;
    let mut has_structural_children = false;
    element.structural_children(&mut |child| {
        has_structural_children = true;
        let index = child_index;
        child_index = child_index
            .checked_add(1)
            .expect("exhausted structural child indexes");
        index_element_links(
            child,
            Some(link_index),
            index,
            links,
            path_indices,
            indexed_elements,
            scope,
        );
    });
    indexed_elements[link_index].has_structural_children = has_structural_children;
}

/// Verifies that a replaced subtree has the shape the path index recorded, and
/// collects the element pointers that replace the stale ones.
///
/// The walk visits the new subtree in the preorder the index was built in, so a
/// subtree of the same shape lands on the same positions. Each element must have
/// the same identity, parent link, child position and child presence as the one
/// it replaces, and none may trap focus: a focus scope is tracked across the
/// whole tree, not per subtree.
struct SubtreeCheck<'a> {
    /// The position the next visited element must occupy.
    next: usize,
    path_indices: &'a HashMap<ElementId, usize>,
    links: &'a [ElementPath],
    indexed: &'a [IndexedElement],
    pointers: Vec<*const (dyn Element + 'static)>,
}

impl SubtreeCheck<'_> {
    fn visit(&mut self, element: &dyn Element, parent: Option<usize>, child_index: u32) -> bool {
        let position = self.next;
        self.next += 1;
        let Some(link) = self.links.get(position) else {
            return false;
        };
        if link.parent != parent || link.child_index != child_index || element.traps_focus() {
            return false;
        }
        if let Some(id) = element.element_id()
            && self.path_indices.get(&id) != Some(&position)
        {
            return false;
        }
        self.pointers.push(retained_element_pointer(element));

        let mut matched = true;
        let mut has_structural_children = false;
        let mut next_child = 0u32;
        element.structural_children(&mut |child| {
            if !matched {
                return;
            }
            has_structural_children = true;
            let index = next_child;
            next_child += 1;
            matched = self.visit(child, Some(position), index);
        });
        matched && self.indexed[position].has_structural_children == has_structural_children
    }
}

/// The exclusive end of the preorder range of the subtree at `start`.
///
/// A later element belongs to the subtree exactly when its parent does, and
/// preorder keeps the subtree contiguous.
fn subtree_end(links: &[ElementPath], start: usize) -> usize {
    let mut end = start + 1;
    while end < links.len() && links[end].parent.is_some_and(|parent| parent >= start) {
        end += 1;
    }
    end
}

impl EventDispatcher {
    /// Whether `descendant` is `ancestor` or lies below it in the structural
    /// tree below `root`. Both must be known to the index; an element it does
    /// not know is related to nothing.
    #[doc(hidden)]
    pub fn is_ancestor_or_self(
        &mut self,
        root: &dyn Element,
        ancestor: ElementId,
        descendant: ElementId,
    ) -> bool {
        self.synchronize_path_index_for_current_tree(root);
        let (Some(&ancestor), Some(&mut_descendant)) =
            (self.path_indices.get(&ancestor), self.path_indices.get(&descendant))
        else {
            return false;
        };
        let mut position = Some(mut_descendant);
        while let Some(current) = position {
            if current == ancestor {
                return true;
            }
            position = self.path_links.get(current).and_then(|link| link.parent);
        }
        false
    }

    /// Brings the path index up to date after subtrees were replaced by ones of
    /// the same shape, without rebuilding it.
    ///
    /// A replacement installs new element instances at the same identities, so
    /// the only thing that went stale is the pointer stored for each element of
    /// the replaced subtree. Returns `true` when every pointer was refreshed.
    /// Returns `false` without changing the index when the tree changed in any
    /// way the log of replacements does not fully describe, or a replaced
    /// subtree is not shaped exactly as before; the caller rebuilds the index.
    pub(super) fn patch_path_index(&mut self, root: &dyn Element) -> bool {
        if self.indexed_root != root.element_id()
            || unscoped_element_tree_generation() != self.indexed_unscoped
        {
            return false;
        }
        let Some(replaced) = scoped_rebuilds_since(self.scoped_cursor) else {
            return false;
        };
        // The log must account for every change. A generation that moved with
        // nothing logged, or that moved by more than the replacements logged,
        // is a change nobody described.
        if replaced.is_empty()
            || self.indexed_tree_generation.checked_add(replaced.len() as u64)
                != Some(element_tree_generation())
        {
            return false;
        }

        // Every logged root must be one the index knows. A root it does not
        // know cannot be placed: reconciliation gives a replacement root a
        // fresh identity when the old and new roots are incompatible, so it may
        // be exactly what dropped a subtree this index still points into, and
        // the recorded parent of any other replacement could then dangle. A
        // replacement inside another one is not an exception, because the
        // verification below requires every element of a replaced subtree to
        // keep its identity. A root outside this tree entirely is rare enough
        // that rebuilding the index is the cheaper answer than proving it
        // foreign.
        let mut starts = SmallVec::<[usize; 4]>::new();
        for id in &replaced {
            let Some(&start) = self.path_indices.get(id) else {
                return false;
            };
            starts.push(start);
        }
        if starts.is_empty() {
            return false;
        }
        starts.sort_unstable();
        starts.dedup();
        // A replacement inside another replaced subtree is covered by it.
        let mut outermost = SmallVec::<[(usize, usize); 4]>::new();
        for start in starts {
            if outermost.last().is_none_or(|&(_, end)| start >= end) {
                outermost.push((start, subtree_end(&self.path_links, start)));
            }
        }

        let focus_scope = self
            .focus_scope
            .and_then(|scope| self.path_indices.get(&scope).copied());
        let mut patches = SmallVec::<[(usize, Vec<*const (dyn Element + 'static)>); 4]>::new();
        for &(start, end) in &outermost {
            if focus_scope.is_some_and(|scope| (start..end).contains(&scope)) {
                return false;
            }
            let link = self.path_links[start];
            let element: &dyn Element = match link.parent {
                None => root,
                Some(parent) => {
                    // SAFETY: `parent` lies outside every replaced subtree, so
                    // it was not replaced since the index recorded it. Every
                    // replacement since then is in the log read above, and a
                    // replacement inside another is dropped as covered. The
                    // pointer was valid when recorded and nothing has moved or
                    // dropped the element since, so it is valid now.
                    let parent: &dyn Element = unsafe { &*self.indexed_elements[parent].element };
                    let mut found = None;
                    let mut index = 0u32;
                    parent.structural_children(&mut |child| {
                        if index == link.child_index {
                            found = Some(child);
                        }
                        index += 1;
                    });
                    match found {
                        Some(element) => element,
                        None => return false,
                    }
                }
            };
            let mut check = SubtreeCheck {
                next: start,
                path_indices: &self.path_indices,
                links: &self.path_links,
                indexed: &self.indexed_elements,
                pointers: Vec::with_capacity(end - start),
            };
            if !check.visit(element, link.parent, link.child_index) || check.next != end {
                return false;
            }
            patches.push((start, check.pointers));
        }

        for (start, pointers) in patches {
            for (offset, pointer) in pointers.into_iter().enumerate() {
                self.indexed_elements[start + offset].element = pointer;
            }
        }
        self.scoped_cursor = scoped_rebuild_cursor();
        self.indexed_tree_generation = element_tree_generation();
        true
    }
}

pub(super) fn resolve_element_path<'a>(
    root: &'a dyn Element,
    owner: ElementId,
    path_indices: &HashMap<ElementId, usize>,
    links: &[ElementPath],
) -> Option<&'a dyn Element> {
    let mut child_indexes: SmallVec<[u32; 16]> = SmallVec::new();
    let mut link_index = *path_indices.get(&owner)?;
    let mut remaining = links.len();

    loop {
        remaining = remaining.checked_sub(1)?;
        let link = *links.get(link_index)?;
        let Some(parent) = link.parent else {
            break;
        };
        child_indexes.push(link.child_index);
        link_index = parent;
    }

    let mut current = root;
    for index in child_indexes.iter().rev() {
        current = structural_child_at(current, *index as usize)?;
    }
    (current.element_id() == Some(owner)).then_some(current)
}

pub(super) fn dispatch_routed_event(
    dispatcher: &mut EventDispatcher,
    root: &dyn Element,
    pos: Vec2d,
    event: &ElementEvent,
) -> RoutedEventResult {
    debug_assert_eq!(dispatcher.indexed_root, root.element_id());
    debug_assert_eq!(
        dispatcher.indexed_root_address,
        Some(root as *const dyn Element as *const ())
    );
    dispatch_indexed_event(dispatcher, root, pos, event)
}

fn dispatch_indexed_event(
    dispatcher: &mut EventDispatcher,
    root: &dyn Element,
    pos: Vec2d,
    event: &ElementEvent,
) -> RoutedEventResult {
    let frame_index = dispatcher.event_hit_depth.get();
    let _depth_guard = EventHitDepthGuard::enter(dispatcher.event_hit_depth.clone());
    if dispatcher.event_hit_frames.len() <= frame_index {
        dispatcher
            .event_hit_frames
            .resize_with(frame_index + 1, EventHitFrame::default);
    }
    let event_tree = &dispatcher.event_tree;
    let epoch = dispatcher.event_hit_frames[frame_index].prepare(event_tree.elements().len());
    for root_target in event_tree.roots().iter().rev().copied() {
        let Some(root_entry) = event_tree.get(root_target) else {
            continue;
        };
        let root_id = root_entry.element_id();
        let Some(root_element) = resolve_indexed_event_element(root, dispatcher, root_id) else {
            continue;
        };
        if !contains(root_element, pos) {
            continue;
        }

        if is_indexed_hit_test_boundary(root_element.event_tree_role()) {
            dispatcher.event_hit_frames[frame_index].mark_hit_path(event_tree, root_target, epoch);
            let mut branch_ranges: SmallVec<[(usize, usize); 16]> = SmallVec::new();
            root_element.hit_test_children_at(pos, &mut |child| {
                if let Some(branch) = child.element_id()
                    && let Some(range) = event_tree.boundary_branch_range(root_target, branch)
                {
                    branch_ranges.push((range.start, range.end));
                }
            });
            dispatcher.event_hit_frames[frame_index]
                .mark_boundary_filtered(root_target, epoch);
            for (start, end) in branch_ranges.into_iter().rev() {
                if end == start + 1 {
                    let target = EventTargetId::from_index(start);
                    if event_tree.target_contains(target, pos.x, pos.y) {
                        dispatcher.event_hit_frames[frame_index]
                            .mark_hit_path(event_tree, target, epoch);
                    }
                    continue;
                }
                for target in event_tree.hit_test_range(start..end, pos.x, pos.y) {
                    dispatcher.event_hit_frames[frame_index].mark_hit_path(event_tree, target, epoch);
                }
            }
        } else {
            let range = root_entry.subtree_range(root_target);
            if range.start == 0 && range.end == event_tree.elements().len() {
                for target in event_tree.hit_test(pos.x, pos.y) {
                    dispatcher.event_hit_frames[frame_index].mark_hit_path(event_tree, target, epoch);
                }
            } else {
                for target in event_tree.hit_test_range(range, pos.x, pos.y) {
                    dispatcher.event_hit_frames[frame_index].mark_hit_path(event_tree, target, epoch);
                }
            }
        }
    }

    let mut result = RoutedEventResult {
        result: EventResult::ignored(),
        capture_owner: None,
        focus_owner: None,
    };
    let active_root_count = dispatcher.event_hit_frames[frame_index].roots.len();
    for root_index in 0..active_root_count {
        let root_target = dispatcher.event_hit_frames[frame_index].roots[root_index];
        let child = dispatch_indexed_target_inner(
            dispatcher,
            frame_index,
            epoch,
            root,
            root_target,
            pos,
            event,
        );
        result.result = result.result.merge(child.result);
        if result.focus_owner.is_none() {
            result.focus_owner = child.focus_owner;
        }
        if result.capture_owner.is_none() {
            result.capture_owner = child.capture_owner;
        }
        if child.result.is_consumed() {
            break;
        }
    }
    result
}

/// Routes a forwarding element's private event-child view through the retained
/// event index when the child has an indexed target.
pub(super) fn dispatch_nested_event(
    dispatcher: &mut EventDispatcher,
    path_root: &dyn Element,
    boundary: Option<ElementId>,
    root: &dyn Element,
    pos: Vec2d,
    event: &ElementEvent,
) -> RoutedEventResult {
    if let Some(target) = root
        .element_id()
        .and_then(|id| dispatcher.event_target_by_element.get(&id).copied())
    {
        return dispatch_indexed_event_subtree(dispatcher, path_root, target, pos, event);
    }

    if let Some(target) = boundary
        .and_then(|id| dispatcher.event_target_by_element.get(&id).copied())
    {
        return dispatch_indexed_event_children(
            dispatcher,
            path_root,
            target,
            boundary.expect("indexed forwarding target has an element ID"),
            root,
            pos,
            event,
        );
    }

    let _ = boundary;
    let mut children = EventChildren::new();
    dispatch_routed_event_inner(dispatcher, path_root, root, pos, event, &mut children)
}

fn dispatch_indexed_event_subtree(
    dispatcher: &mut EventDispatcher,
    path_root: &dyn Element,
    target: EventTargetId,
    pos: Vec2d,
    event: &ElementEvent,
) -> RoutedEventResult {
    let frame_index = dispatcher.event_hit_depth.get();
    let _depth_guard = EventHitDepthGuard::enter(dispatcher.event_hit_depth.clone());
    if dispatcher.event_hit_frames.len() <= frame_index {
        dispatcher
            .event_hit_frames
            .resize_with(frame_index + 1, EventHitFrame::default);
    }

    let tree = &dispatcher.event_tree;
    let Some(entry) = tree.get(target) else {
        return RoutedEventResult {
            result: EventResult::ignored(),
            capture_owner: None,
            focus_owner: None,
        };
    };
    let id = entry.element_id();
    let Some(element) = resolve_indexed_event_element(path_root, dispatcher, id) else {
        return RoutedEventResult {
            result: EventResult::ignored(),
            capture_owner: None,
            focus_owner: None,
        };
    };
    if !contains(element, pos) {
        return RoutedEventResult {
            result: EventResult::ignored(),
            capture_owner: None,
            focus_owner: None,
        };
    }

    let range = entry.subtree_range(target);
    let epoch = dispatcher.event_hit_frames[frame_index].prepare(tree.elements().len());
    dispatcher.event_hit_frames[frame_index].mark_hit_path(tree, target, epoch);
    for candidate in tree.hit_test_range(range, pos.x, pos.y) {
        dispatcher.event_hit_frames[frame_index].mark_hit_path(tree, candidate, epoch);
    }
    // `mark_hit_path` also marks the indexed ancestors of this private view.
    // Start at the requested child so sibling event branches stay private to
    // the forwarding boundary.
    dispatcher.event_hit_frames[frame_index].roots.clear();
    dispatcher.event_hit_frames[frame_index].roots.push(target);

    dispatch_indexed_target_inner(
        dispatcher,
        frame_index,
        epoch,
        path_root,
        target,
        pos,
        event,
    )
}

fn dispatch_indexed_event_children(
    dispatcher: &mut EventDispatcher,
    path_root: &dyn Element,
    boundary: EventTargetId,
    boundary_id: ElementId,
    root: &dyn Element,
    pos: Vec2d,
    event: &ElementEvent,
) -> RoutedEventResult {
    if !contains(root, pos) {
        return RoutedEventResult {
            result: EventResult::ignored(),
            capture_owner: None,
            focus_owner: None,
        };
    }

    let frame_index = dispatcher.event_hit_depth.get();
    let _depth_guard = EventHitDepthGuard::enter(dispatcher.event_hit_depth.clone());
    if dispatcher.event_hit_frames.len() <= frame_index {
        dispatcher
            .event_hit_frames
            .resize_with(frame_index + 1, EventHitFrame::default);
    }
    let tree = &dispatcher.event_tree;
    if tree.get(boundary).is_none() {
        return RoutedEventResult {
            result: EventResult::ignored(),
            capture_owner: None,
            focus_owner: None,
        };
    }
    let epoch = dispatcher.event_hit_frames[frame_index].prepare(tree.elements().len());
    let mut branches: SmallVec<[ElementId; 16]> = SmallVec::new();
    root.hit_test_children_at(pos, &mut |child| {
        if let Some(id) = child.element_id() {
            branches.push(id);
        }
    });
    for branch in branches {
        if let Some(range) = tree.boundary_branch_range(boundary, branch) {
            for candidate in tree.hit_test_range(range, pos.x, pos.y) {
                dispatcher.event_hit_frames[frame_index].mark_hit_path(tree, candidate, epoch);
            }
        }
    }
    // The boundary registers the root of its private view as a branch under the
    // root's own identity. A root that is not an event target itself, such as a
    // stateful or animated wrapper around the content, has no indexed entry to
    // take the fast path above, and its children are not the registered
    // branches: without this the press is dropped and nothing inside the
    // region can be clicked.
    if let Some(range) = root
        .element_id()
        .and_then(|id| tree.boundary_branch_range(boundary, id))
    {
        for candidate in tree.hit_test_range(range, pos.x, pos.y) {
            dispatcher.event_hit_frames[frame_index].mark_hit_path(tree, candidate, epoch);
        }
    }

    let mut outcome = RoutedEventResult {
        result: EventResult::ignored(),
        capture_owner: None,
        focus_owner: None,
    };
    let child_count = dispatcher.event_hit_frames[frame_index].active_children[boundary.index()].len();
    for child_index in 0..child_count {
        let child = dispatcher.event_hit_frames[frame_index].active_children[boundary.index()][child_index];
        let child_outcome = dispatch_indexed_target_inner(
            dispatcher,
            frame_index,
            epoch,
            path_root,
            child,
            pos,
            event,
        );
        outcome.result = outcome.result.merge(child_outcome.result);
        if outcome.focus_owner.is_none() {
            outcome.focus_owner = child_outcome.focus_owner;
        }
        if outcome.capture_owner.is_none() {
            outcome.capture_owner = child_outcome.capture_owner.or_else(|| {
                (!matches!(child_outcome.result.capture_request(), CaptureRequest::None))
                    .then_some(boundary_id)
            });
        }
        if child_outcome.result.is_consumed() {
            break;
        }
    }

    let child_result = {
        let mut context = EventDispatchContext::new(dispatcher, path_root, Some(boundary_id), pos);
        root.on_event_with_context(event, &mut context)
    };
    if outcome.capture_owner.is_none()
        && !matches!(child_result.capture_request(), CaptureRequest::None)
    {
        outcome.capture_owner = root.element_id();
    }
    outcome.result = outcome.result.merge(child_result);
    dispatcher.merge_nested_focus_owner(&mut outcome.focus_owner);
    if outcome.focus_owner.is_none() {
        outcome.focus_owner = focus_candidate_at(root, pos);
    }
    outcome
}

fn dispatch_indexed_target_inner(
    dispatcher: &mut EventDispatcher,
    frame_index: usize,
    epoch: u32,
    path_root: &dyn Element,
    target: EventTargetId,
    pos: Vec2d,
    event: &ElementEvent,
) -> RoutedEventResult {
    if dispatcher.event_hit_frames[frame_index].marks[target.index()] != epoch {
        return RoutedEventResult {
            result: EventResult::ignored(),
            capture_owner: None,
            focus_owner: None,
        };
    }
    let Some(target_element) = dispatcher.event_tree.get(target) else {
        return RoutedEventResult {
            result: EventResult::ignored(),
            capture_owner: None,
            focus_owner: None,
        };
    };
    let id = target_element.element_id();
    let Some(element) = resolve_indexed_event_element(path_root, dispatcher, id)
    else {
        return RoutedEventResult {
            result: EventResult::ignored(),
            capture_owner: None,
            focus_owner: None,
        };
    };
    if !contains(element, pos) {
        return RoutedEventResult {
            result: EventResult::ignored(),
            capture_owner: None,
            focus_owner: None,
        };
    }

    record_routed_event_visit();
    dispatcher.record_hit_chain_element(element);
    let mut outcome = RoutedEventResult {
        result: EventResult::ignored(),
        capture_owner: None,
        focus_owner: None,
    };
    let hit_test_boundary = is_indexed_hit_test_boundary(element.event_tree_role())
        && !dispatcher.event_hit_frames[frame_index].boundary_was_filtered(target, epoch);
    let mut hit_test_branches: SmallVec<[ElementId; 16]> = SmallVec::new();
    if hit_test_boundary {
        element.hit_test_children_at(pos, &mut |child| {
            if let Some(id) = child.element_id() {
                hit_test_branches.push(id);
            }
        });
    }
    let child_count = dispatcher.event_hit_frames[frame_index].active_children[target.index()].len();
    let hit_child_count = if hit_test_boundary {
        (0..child_count)
            .filter(|child_index| {
                let child_target = dispatcher.event_hit_frames[frame_index].active_children
                    [target.index()][*child_index];
                dispatcher
                    .event_tree
                    .get(child_target)
                    .and_then(|child| child.parent_branch())
                    .is_some_and(|branch| hit_test_branches.contains(&branch))
            })
            .count()
    } else {
        child_count
    };
    dispatcher.record_hit_chain_children(hit_child_count);
    if hit_child_count == 0 {
        dispatcher.record_empty_hit_chain_node(element);
    }
    for child_index in 0..child_count {
        let child_target = dispatcher.event_hit_frames[frame_index].active_children[target.index()][child_index];
        if hit_test_boundary
            && !dispatcher
                .event_tree
                .get(child_target)
                .and_then(|child| child.parent_branch())
                .is_some_and(|branch| hit_test_branches.contains(&branch))
        {
            continue;
        }
        let child = dispatch_indexed_target_inner(
            dispatcher,
            frame_index,
            epoch,
            path_root,
            child_target,
            pos,
            event,
        );
        outcome.result = outcome.result.merge(child.result);
        if outcome.focus_owner.is_none() {
            outcome.focus_owner = child.focus_owner;
        }
        if outcome.capture_owner.is_none() {
            outcome.capture_owner = child.capture_owner.or_else(|| {
                (!matches!(child.result.capture_request(), CaptureRequest::None)).then_some(id)
            });
        }
        if child.result.is_consumed() {
            break;
        }
    }

    if outcome.focus_owner.is_none() {
        outcome.focus_owner = focus_candidate_at(element, pos);
    }
    if !outcome.result.is_consumed() {
        let own_result = {
            let mut context = EventDispatchContext::new(dispatcher, path_root, Some(id), pos);
            element.on_event_with_context(event, &mut context)
        };
        if outcome.capture_owner.is_none()
            && !matches!(own_result.capture_request(), CaptureRequest::None)
        {
            outcome.capture_owner = Some(id);
        }
        outcome.result = outcome.result.merge(own_result);
        dispatcher.merge_nested_focus_owner(&mut outcome.focus_owner);
        if outcome.focus_owner.is_none() {
            outcome.focus_owner = focus_candidate_at(element, pos);
        }
    }
    outcome
}

fn resolve_indexed_event_element<'a>(
    root: &'a dyn Element,
    dispatcher: &EventDispatcher,
    id: ElementId,
) -> Option<&'a dyn Element> {
    let root_id = root.element_id()?;
    let root_address = root as *const dyn Element as *const ();
    if dispatcher.indexed_root != Some(root_id)
        || dispatcher.indexed_root_address != Some(root_address)
    {
        return None;
    }
    dispatcher.resolve_indexed_element(root, id)
}

pub(super) fn dispatch_cached_hit_chain_inner(
    dispatcher: &mut EventDispatcher,
    path_root: &dyn Element,
    elements: &[CachedHitElement],
    index: usize,
    pos: Vec2d,
    event: &ElementEvent,
) -> RoutedEventResult {
    let entry = elements[index];
    // SAFETY: `dispatch_cached_hit_chain` validated the root address, element
    // identities, and subtree generation before entering this replay. The
    // retained UI tree cannot be concurrently mutated during dispatch.
    let root = unsafe { &*entry.element };
    record_routed_event_visit();

    let mut result = EventResult::ignored();
    let mut capture_owner = None;
    let mut focus_owner = None;
    let mut stopped = false;

    if index + 1 < elements.len() {
        let child_outcome = dispatch_cached_hit_chain_inner(
            dispatcher,
            path_root,
            elements,
            index + 1,
            pos,
            event,
        );
        result = result.merge(child_outcome.result);
        if focus_owner.is_none() {
            focus_owner = child_outcome.focus_owner;
        }
        if capture_owner.is_none() {
            capture_owner = child_outcome.capture_owner.or_else(|| {
                (!matches!(child_outcome.result.capture_request(), CaptureRequest::None))
                    .then(|| root.element_id())
                    .flatten()
            });
        }
        if child_outcome.result.is_consumed() {
            stopped = true;
        }
    }

    if stopped {
        if focus_owner.is_none() {
            focus_owner = focus_candidate_at(root, pos);
        }
        return RoutedEventResult {
            result,
            capture_owner,
            focus_owner,
        };
    }

    let own_result = if !event_callback_enabled(root) {
        EventResult::ignored()
    } else {
        let mut context = EventDispatchContext::new(dispatcher, path_root, root.element_id(), pos);
        root.on_event_with_context(event, &mut context)
    };
    dispatcher.merge_nested_focus_owner(&mut focus_owner);
    if focus_owner.is_none() {
        focus_owner = focus_candidate_at(root, pos);
    }
    if capture_owner.is_none() && !matches!(own_result.capture_request(), CaptureRequest::None) {
        capture_owner = root.element_id();
    }
    result = result.merge(own_result);

    RoutedEventResult {
        result,
        capture_owner,
        focus_owner,
    }
}

/// Whether `pos` lies within `element`'s laid-out bounds.
///
/// An element that reports no bounds is taken to be everywhere, which is what
/// keeps a wrapper that never lays anything out from swallowing the events of
/// the subtree it stands for.
#[inline]
pub(super) fn contains(element: &dyn Element, pos: Vec2d) -> bool {
    element.pos_start_end().is_none_or(|(start, end)| {
        pos.x >= start.x && pos.x <= end.x && pos.y >= start.y && pos.y <= end.y
    })
}

/// Offers `element` as the focus target of a press at `pos`, if it is one.
///
/// The bounds are only consulted once the element has answered with a node, so
/// the overwhelming majority of elements — which are not focus targets — cost a
/// single call.
#[inline]
pub(super) fn focus_candidate_at(element: &dyn Element, pos: Vec2d) -> Option<FocusCandidate<ElementId>> {
    let node = element.focus_node()?;
    let id = element.element_id()?;
    contains(element, pos).then(|| FocusCandidate::new(id, node.clone(), element.autofocus()))
}

fn dispatch_routed_event_inner<'tree>(
    dispatcher: &mut EventDispatcher,
    path_root: &dyn Element,
    root: &'tree dyn Element,
    pos: Vec2d,
    event: &ElementEvent,
    children: &mut EventChildren<'tree>,
) -> RoutedEventResult {
    if !contains(root, pos) {
        dispatcher.record_hit_chain_miss();
        return RoutedEventResult {
            result: EventResult::ignored(),
            capture_owner: None,
            focus_owner: None,
        };
    }
    record_routed_event_visit();
    dispatcher.record_hit_chain_element(root);

    let mut result = EventResult::ignored();
    let mut capture_owner = None;
    let mut focus_owner = None;
    let mut stopped = false;
    let start = children.len();
    let mut hit_test_children = 0;
    root.hit_test_children_at(pos, &mut |child| {
        hit_test_children += 1;
        children.push(child);
    });
    dispatcher.record_hit_chain_children(hit_test_children);
    if hit_test_children == 0 {
        dispatcher.record_empty_hit_chain_node(root);
    }

    while children.len() > start {
        if stopped {
            children.truncate(start);
            break;
        }
        let child = children
            .pop()
            .expect("routed event scratch contains an element beyond its entry length");
        let child_outcome = dispatch_routed_event_inner(
            dispatcher,
            path_root,
            child,
            pos,
            event,
            children,
        );
        result = result.merge(child_outcome.result);
        if focus_owner.is_none() {
            focus_owner = child_outcome.focus_owner;
        }
        if capture_owner.is_none() {
            capture_owner = child_outcome.capture_owner.or_else(|| {
                (!matches!(child_outcome.result.capture_request(), CaptureRequest::None))
                    .then(|| root.element_id())
                    .flatten()
            });
        }
        if child_outcome.result.is_consumed() {
            // The press stopped here, but it landed inside this element all the
            // same: a control that takes a press for itself still sits within
            // whatever region encloses it, and that region is what the focus
            // should move to. Giving up the search here would leave the press
            // with no target at all — and a press with no target takes the
            // keyboard away, so clicking a field inside a focusable region
            // would blur it.
            stopped = true;
        }
    }
    children.truncate(start);

    if stopped {
        if focus_owner.is_none() {
            focus_owner = focus_candidate_at(root, pos);
        }
        return RoutedEventResult {
            result,
            capture_owner,
            focus_owner,
        };
    }

    let own_result = if !event_callback_enabled(root) {
        EventResult::ignored()
    } else {
        let mut context = EventDispatchContext::new(dispatcher, path_root, root.element_id(), pos);
        root.on_event_with_context(event, &mut context)
    };
    dispatcher.merge_nested_focus_owner(&mut focus_owner);
    if focus_owner.is_none() {
        focus_owner = focus_candidate_at(root, pos);
    }
    if capture_owner.is_none() && !matches!(own_result.capture_request(), CaptureRequest::None) {
        capture_owner = root.element_id();
    }
    result = result.merge(own_result);

    RoutedEventResult {
        result,
        capture_owner,
        focus_owner,
    }
}

/// Perform a hit-test on the element tree and dispatch the event to the deepest
/// hit element. Returns the effects produced by the target element.
pub fn dispatch_event(root: &dyn Element, pos: Vec2d, event: &ElementEvent) -> EventResult {
    let mut dispatcher = EventDispatcher::new();
    dispatcher.synchronize_paths(root);
    dispatch_indexed_event(&mut dispatcher, root, pos, event).result
}

#[cfg(test)]
pub(super) fn dispatch_event_inner<'a>(
    root: &'a dyn Element,
    pos: Vec2d,
    event: &ElementEvent,
    children: &mut EventChildren<'a>,
) -> EventResult {
    if !contains(root, pos) {
        return EventResult::ignored();
    }

    let mut result = EventResult::ignored();
    let start = children.len();
    root.hit_test_children_at(pos, &mut |child| {
        if contains(child, pos) {
            children.push(child);
        }
    });

    while children.len() > start {
        let child = children
            .pop()
            .expect("event child scratch contains an element beyond its entry length");
        let child_result = dispatch_event_inner(child, pos, event, children);
        result = result.merge(child_result);
        if child_result.is_consumed() {
            children.truncate(start);
            return result;
        }
    }
    children.truncate(start);

    if !event_callback_enabled(root) {
        result
    } else {
        result.merge(root.on_event(event))
    }
}

/// Broadcast an event to every element in the tree, regardless of hit-testing.
/// Returns the combined effects produced by every element.
/// Repeated callers can retain an [`EventDispatcher`] and use
/// [`EventDispatcher::broadcast`] to reuse its indexes.
pub fn broadcast_event(root: &dyn Element, event: &ElementEvent) -> EventResult {
    EventDispatcher::new().broadcast(root, event)
}

impl EventDispatcher {
    /// Broadcasts an event to every event target, regardless of hit-testing.
    ///
    /// Reuses the retained event tree and direct element index while the root
    /// and its subtree generation remain unchanged.
    ///
    /// Delivery visits children before their parent in reverse child order and
    /// continues after consumption, combining every target's effects. Capture
    /// and focus requests in the result are not applied by this operation.
    ///
    /// # Panics
    ///
    /// Panics if an event target lacks a retained identity in the structural
    /// tree. [`Element::boxed`] supplies these identities.
    pub fn broadcast(&mut self, root: &dyn Element, event: &ElementEvent) -> EventResult {
        self.synchronize_paths_for_current_tree(root);
        let mut result = EventResult::ignored();
        for target in self.event_tree.roots().iter().rev().copied() {
            result = result.merge(broadcast_indexed_target(root, self, target, event));
        }
        result
    }
}

fn broadcast_indexed_target(
    root: &dyn Element,
    dispatcher: &EventDispatcher,
    target: EventTargetId,
    event: &ElementEvent,
) -> EventResult {
    let Some(entry) = dispatcher.event_tree.get(target) else {
        return EventResult::ignored();
    };
    let mut result = EventResult::ignored();
    for child in entry.children().iter().rev().copied() {
        result = result.merge(broadcast_indexed_target(
            root,
            dispatcher,
            child,
            event,
        ));
    }
    if let Some(element) = dispatcher.resolve_indexed_element(root, entry.element_id()) {
        result.merge(element.on_event(event))
    } else {
        result
    }
}

#[cfg(test)]
pub(super) fn broadcast_event_inner<'a>(
    root: &'a dyn Element,
    event: &ElementEvent,
    children: &mut EventChildren<'a>,
) -> EventResult {
    let mut result = EventResult::ignored();
    let start = children.len();
    root.event_children(&mut |child| children.push(child));

    while children.len() > start {
        let child = children
            .pop()
            .expect("event child scratch contains an element beyond its entry length");
        result = result.merge(broadcast_event_inner(child, event, children));
    }
    children.truncate(start);

    if !event_callback_enabled(root) {
        result
    } else {
        result.merge(root.on_event(event))
    }
}

/// Deliver a focus-directed event to the element that owns keyboard focus.
///
/// Keyboard text and input-method composition carry no meaningful position, so
/// the tree is walked depth-first and every element is offered the event until
/// one consumes it; elements without focus ignore it. Unlike
/// [`broadcast_event`], delivery stops at the first consumer, so a focused field
/// nested inside another field never sees the same text twice.
pub fn dispatch_focused_event(root: &dyn Element, event: &ElementEvent) -> EventResult {
    let mut dispatcher = EventDispatcher::new();
    dispatcher.synchronize_paths(root);
    for target in dispatcher.event_tree.roots().iter().rev().copied() {
        let result = dispatch_focused_indexed_target(
            root,
            &dispatcher.event_tree,
            &dispatcher.path_indices,
            &dispatcher.path_links,
            target,
            event,
        );
        if result.is_consumed() {
            return result;
        }
    }
    EventResult::ignored()
}

fn dispatch_focused_indexed_target(
    root: &dyn Element,
    tree: &EventTree,
    path_indices: &HashMap<ElementId, usize>,
    path_links: &[ElementPath],
    target: EventTargetId,
    event: &ElementEvent,
) -> EventResult {
    let Some(entry) = tree.get(target) else {
        return EventResult::ignored();
    };
    for child in entry.children().iter().rev().copied() {
        let result = dispatch_focused_indexed_target(
            root,
            tree,
            path_indices,
            path_links,
            child,
            event,
        );
        if result.is_consumed() {
            return result;
        }
    }
    resolve_element_path(root, entry.element_id(), path_indices, path_links)
        .map(|element| element.on_event(event))
        .unwrap_or_else(EventResult::ignored)
}

#[cfg(test)]
pub(super) fn dispatch_focused_event_inner<'a>(
    root: &'a dyn Element,
    event: &ElementEvent,
    children: &mut EventChildren<'a>,
) -> EventResult {
    let mut result = EventResult::ignored();
    let start = children.len();
    root.event_children(&mut |child| children.push(child));

    while children.len() > start {
        let child = children
            .pop()
            .expect("event child scratch contains an element beyond its entry length");
        result = result.merge(dispatch_focused_event_inner(child, event, children));
        if result.is_consumed() {
            children.truncate(start);
            return result;
        }
    }
    children.truncate(start);

    if !event_callback_enabled(root) {
        result
    } else {
        result.merge(root.on_event(event))
    }
}
