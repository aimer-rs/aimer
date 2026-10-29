use super::*;

#[test]
fn paint_damage_starts_full_once_and_tracks_later_local_regions() {
    begin_paint_frame(37, 23);
    assert!(take_paint_frame_damage(37, 23).is_full());

    begin_paint_frame(37, 23);
    assert!(take_paint_frame_damage(37, 23).is_empty());

    begin_paint_frame(37, 23);
    mark_paint_damage(DamageRect::new(4, 5, 6, 7));
    assert_eq!(
        take_paint_frame_damage(37, 23).regions(),
        &[DamageRect::new(4, 5, 6, 7)]
    );

    begin_paint_frame(38, 23);
    assert!(take_paint_frame_damage(38, 23).is_full());
}

#[test]
fn full_damage_requested_between_frames_is_applied_to_the_next_frame() {
    begin_paint_frame(41, 29);
    assert!(take_paint_frame_damage(41, 29).is_full());

    mark_paint_damage_full();
    begin_paint_frame(41, 29);

    assert!(take_paint_frame_damage(41, 29).is_full());
}

#[test]
fn untracked_rebuild_invalidations_force_full_damage() {
    begin_paint_frame(41, 29);
    let _ = take_paint_frame_damage(41, 29);

    advance_rebuild_invalidation_generation();
    begin_paint_frame(41, 29);

    assert!(take_paint_frame_damage(41, 29).is_full());
}

struct StructuralTraversalElement {
    event_child: AnyElement,
    visual_child: AnyElement,
    direct_calls: Rc<Cell<usize>>,
}

impl VisitorElement for StructuralTraversalElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.visual_child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "StructuralTraversalElement"
    }
}

impl EventElement for StructuralTraversalElement {
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.event_child.as_ref());
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.direct_calls.set(self.direct_calls.get() + 1);
        visitor(self.event_child.as_ref());
        visitor(self.visual_child.as_ref());
    }
}

impl LayoutElement for StructuralTraversalElement {}

impl Drawable for StructuralTraversalElement {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for StructuralTraversalElement {}

struct DefaultStructuralTraversalElement {
    event_child: AnyElement,
    visual_child: AnyElement,
}

impl VisitorElement for DefaultStructuralTraversalElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.visual_child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "DefaultStructuralTraversalElement"
    }
}

impl EventElement for DefaultStructuralTraversalElement {
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.event_child.as_ref());
    }
}

impl LayoutElement for DefaultStructuralTraversalElement {}

impl Drawable for DefaultStructuralTraversalElement {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for DefaultStructuralTraversalElement {}

struct IndexedStructuralChildElement {
    child: AnyElement,
    direct_calls: Rc<Cell<usize>>,
}

impl VisitorElement for IndexedStructuralChildElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "IndexedStructuralChildElement"
    }
}

impl EventElement for IndexedStructuralChildElement {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.direct_calls.set(self.direct_calls.get() + 1);
        visitor(self.child.as_ref());
    }
}

impl LayoutElement for IndexedStructuralChildElement {}

impl Drawable for IndexedStructuralChildElement {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for IndexedStructuralChildElement {}

#[test]
fn default_structural_traversal_preserves_event_visual_union() {
    let element = DefaultStructuralTraversalElement {
        event_child: DowncastableElement.boxed(),
        visual_child: DowncastableElement.boxed(),
    };

    let children = structural_children(&element);

    assert_eq!(children.len(), 2);
    assert!(std::ptr::eq(children[0], element.event_child.as_ref()));
    assert!(std::ptr::eq(children[1], element.visual_child.as_ref()));
}

#[test]
fn structural_children_uses_element_specific_traversal() {
    let element = StructuralTraversalElement {
        event_child: DowncastableElement.boxed(),
        visual_child: DowncastableElement.boxed(),
        direct_calls: Rc::new(Cell::new(0)),
    };

    let children = structural_children(&element);

    assert_eq!(element.direct_calls.get(), 1);
    assert_eq!(children.len(), 2);
    assert!(std::ptr::eq(children[0], element.event_child.as_ref()));
    assert!(std::ptr::eq(children[1], element.visual_child.as_ref()));
}

#[test]
fn indexed_element_resolution_does_not_walk_the_retained_tree_again() {
    let direct_calls = Rc::new(Cell::new(0));
    let visual_child = DowncastableElement.boxed();
    let visual_child_id = visual_child.id();
    let root = StructuralTraversalElement {
        event_child: DowncastableElement.boxed(),
        visual_child,
        direct_calls: direct_calls.clone(),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    dispatcher.synchronize_paths(root.as_ref());
    assert!(direct_calls.get() > 0, "indexing visits structural children");

    direct_calls.set(0);
    for _ in 0..10_000 {
        let resolved = dispatcher.resolve_indexed_element(root.as_ref(), visual_child_id);
        assert_eq!(
            resolved.and_then(|element| element.element_id()),
            Some(visual_child_id)
        );
    }
    assert_eq!(direct_calls.get(), 0, "10,000 indexed lookups must not walk children");
}

#[test]
fn hit_chain_cache_checks_structural_children_from_the_index() {
    let direct_calls = Rc::new(Cell::new(0));
    let root = IndexedStructuralChildElement {
        child: DowncastableElement.boxed(),
        direct_calls: direct_calls.clone(),
    }
    .boxed();
    let mut dispatcher = EventDispatcher::new();

    dispatcher.synchronize_paths(root.as_ref());
    direct_calls.set(0);
    dispatcher.begin_hit_chain_recording();
    dispatcher.record_hit_chain_element(root.as_ref());
    dispatcher.record_empty_hit_chain_node(root.as_ref());
    let recorder = dispatcher
        .hit_chain_recorder
        .take()
        .expect("hit-chain recording was started");

    assert!(recorder
        .finish(
            PointerKey::new(PointerSource::Mouse, 1),
            root.as_ref(),
            root.subtree_generation(),
            Vec2d::default(),
        )
        .is_none(), "a structural child omitted by hit testing blocks caching");
    assert_eq!(direct_calls.get(), 0, "cache validation must not walk children");
}

#[test]
fn erased_element_forwards_structural_traversal() {
    let direct_calls = Rc::new(Cell::new(0));
    let element = StructuralTraversalElement {
        event_child: DowncastableElement.boxed(),
        visual_child: DowncastableElement.boxed(),
        direct_calls: direct_calls.clone(),
    };
    let erased = element.boxed();

    let _ = structural_children(erased.as_ref());

    assert_eq!(direct_calls.get(), 1);
}

#[test]
fn focus_dispatch_uses_structural_traversal() {
    let direct_calls = Rc::new(Cell::new(0));
    let root = StructuralTraversalElement {
        event_child: DowncastableElement.boxed(),
        visual_child: DowncastableElement.boxed(),
        direct_calls: direct_calls.clone(),
    }
    .boxed();
    let event = ElementEvent::KeyInput {
        key: NamedKey::Tab,
        action: KeyAction::Pressed,
        modifiers: Modifiers::default(),
    };

    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.dispatch(&root, Vec2d::default(), &event);

    assert!(direct_calls.get() > 0);
}

struct DowncastableElement;

impl VisitorElement for DowncastableElement {
    fn debug_name(&self) -> &'static str {
        "DowncastableElement"
    }
}

impl EventElement for DowncastableElement {}
impl LayoutElement for DowncastableElement {}
impl Drawable for DowncastableElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for DowncastableElement {
    fn option_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

#[test]
fn boxed_element_delegates_runtime_downcasting() {
    let element: Box<dyn Element> = Box::new(DowncastableElement);

    assert!(
        element
            .option_any()
            .is_some_and(|value| value.is::<DowncastableElement>())
    );
}

struct StorageElement<const N: usize>([u8; N]);

impl<const N: usize> VisitorElement for StorageElement<N> {
    fn debug_name(&self) -> &'static str {
        "StorageElement"
    }
}

impl<const N: usize> EventElement for StorageElement<N> {}
impl<const N: usize> LayoutElement for StorageElement<N> {}
impl<const N: usize> Drawable for StorageElement<N> {
    fn update(&self, _ctx: &BuildContext) {}
}
impl<const N: usize> Rebuildable for StorageElement<N> {
    fn option_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

#[test]
fn erased_elements_select_inline_or_heap_storage_and_dispatch_after_moves() {
    let inline = StorageElement([]).boxed();
    let heap = StorageElement([0; INLINE_CAPACITY + 1]).boxed();

    assert!(inline.is_inline());
    assert!(heap.is_heap());

    let owners = std::hint::black_box([inline, heap]);
    assert_eq!(owners[0].debug_name(), "StorageElement");
    assert!(
        owners[1]
            .option_any()
            .is_some_and(|value| { value.is::<StorageElement<{ INLINE_CAPACITY + 1 }>>() })
    );
}

#[test]
fn boxed_in_uses_the_supplied_ui_memory() {
    let memory = UiMemory::new(2 * 1024 * 1024);
    let allocator = memory.allocator();
    let element = StorageElement([0; INLINE_CAPACITY + 1]).boxed_in(&allocator);

    assert!(element.is_heap());
    assert_eq!(allocator.committed_bytes(), 2 * 1024 * 1024);
}

struct IdentityLeaf {
    key: Option<Key>,
}

impl IdentityLeaf {
    fn unkeyed() -> Self {
        Self { key: None }
    }

    fn keyed(key: &'static str) -> Self {
        Self {
            key: Some(Key::Static(key)),
        }
    }
}

impl VisitorElement for IdentityLeaf {
    fn debug_name(&self) -> &'static str {
        "IdentityLeaf"
    }

    fn reconciliation_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }
}

impl EventElement for IdentityLeaf {}
impl LayoutElement for IdentityLeaf {}
impl Drawable for IdentityLeaf {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for IdentityLeaf {}

pub(super) struct ReplacementLeaf;

impl VisitorElement for ReplacementLeaf {
    fn debug_name(&self) -> &'static str {
        "ReplacementLeaf"
    }
}

impl EventElement for ReplacementLeaf {}
impl LayoutElement for ReplacementLeaf {}
impl Drawable for ReplacementLeaf {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for ReplacementLeaf {}

pub(super) struct IdentityBranch(pub(super) Vec<AnyElement>);

impl VisitorElement for IdentityBranch {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.0 {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "IdentityBranch"
    }
}

impl EventElement for IdentityBranch {}
impl LayoutElement for IdentityBranch {}
impl Drawable for IdentityBranch {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for IdentityBranch {}

struct StableRoot {
    children: Vec<AnyElement>,
    visits: Rc<Cell<usize>>,
}

impl VisitorElement for StableRoot {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.visits.set(self.visits.get() + 1);
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "StableRoot"
    }
}

impl EventElement for StableRoot {}
impl LayoutElement for StableRoot {
    fn is_layout_stable(&self) -> bool {
        true
    }
}
impl Drawable for StableRoot {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for StableRoot {}

fn child_ids(element: &dyn Element) -> Vec<ElementId> {
    let mut ids = Vec::new();
    element.visit_children(&mut |child| ids.push(child.id()));
    ids
}

#[test]
fn boxed_elements_have_unique_ids_that_survive_owner_moves() {
    let first = IdentityLeaf::unkeyed().boxed();
    let second = IdentityLeaf::unkeyed().boxed();
    let first_id = first.id();

    assert_ne!(first_id, second.id());

    let moved = std::hint::black_box(vec![second, first]);
    assert_eq!(moved[1].id(), first_id);
}

#[test]
fn reconciliation_preserves_keyed_ids_across_reorder() {
    let old = IdentityBranch(vec![
        IdentityLeaf::keyed("a").boxed(),
        IdentityLeaf::keyed("b").boxed(),
    ])
    .boxed();
    let new = IdentityBranch(vec![
        IdentityLeaf::keyed("b").boxed(),
        IdentityLeaf::keyed("a").boxed(),
    ])
    .boxed();
    let old_ids = child_ids(old.as_ref());

    reconcile_element_identities(old.as_ref(), new.as_ref());

    assert_eq!(child_ids(new.as_ref()), vec![old_ids[1], old_ids[0]]);
}

#[test]
fn reconciliation_preserves_compatible_unkeyed_ids_by_position() {
    let old = IdentityBranch(vec![
        IdentityLeaf::unkeyed().boxed(),
        IdentityLeaf::unkeyed().boxed(),
    ])
    .boxed();
    let new = IdentityBranch(vec![
        IdentityLeaf::unkeyed().boxed(),
        IdentityLeaf::unkeyed().boxed(),
    ])
    .boxed();
    let old_ids = child_ids(old.as_ref());

    reconcile_element_identities(old.as_ref(), new.as_ref());

    assert_eq!(child_ids(new.as_ref()), old_ids);
}

#[test]
fn reconciliation_assigns_replacements_a_new_id() {
    let old = IdentityBranch(vec![IdentityLeaf::unkeyed().boxed()]).boxed();
    let new = IdentityBranch(vec![ReplacementLeaf.boxed()]).boxed();
    let old_id = child_ids(old.as_ref())[0];
    let replacement_id = child_ids(new.as_ref())[0];

    reconcile_element_identities(old.as_ref(), new.as_ref());

    assert_ne!(child_ids(new.as_ref())[0], old_id);
    assert_eq!(child_ids(new.as_ref())[0], replacement_id);
}

#[test]
fn generated_tree_reconciliation_advances_the_structure_generation() {
    let old = IdentityLeaf::unkeyed().boxed();
    let new = IdentityLeaf::unkeyed().boxed();
    let generation = element_tree_generation();

    reconcile_generated_tree(old.as_ref(), new.as_ref());

    assert!(element_tree_generation() > generation);
    assert_eq!(new.id(), old.id());
}

#[test]
fn dispatcher_reindexes_at_most_once_per_event_frame() {
    let visits = Rc::new(Cell::new(0));
    let root = StableRoot {
        children: Vec::new(),
        visits: visits.clone(),
    }
    .boxed();
    let root_generation = root.subtree_generation();
    let mut dispatcher = EventDispatcher::new();

    begin_event_frame();
    dispatcher.synchronize_paths(root.as_ref());
    assert_eq!(dispatcher.indexed_subtree_generation, root_generation);
    let first_index_visits = visits.get();
    assert!(first_index_visits > 0);

    advance_element_tree_generation();
    assert_ne!(element_tree_generation(), root_generation);
    assert_eq!(root.subtree_generation(), root_generation);

    dispatcher.synchronize_paths(root.as_ref());
    assert_eq!(visits.get(), first_index_visits);

    let changed_generation = root_generation + 1;
    root.set_subtree_generation(changed_generation);
    dispatcher.synchronize_paths(root.as_ref());
    assert_eq!(
        visits.get(),
        first_index_visits,
        "a generation change after the first dispatch waits for the next frame"
    );

    begin_event_frame();
    dispatcher.synchronize_paths(root.as_ref());

    assert_eq!(dispatcher.indexed_subtree_generation, changed_generation);
    let second_index_visits = visits.get();
    assert!(second_index_visits > first_index_visits);

    dispatcher.synchronize_paths(root.as_ref());
    assert_eq!(visits.get(), second_index_visits);
}

struct LayoutInvalidationLeaf {
    invalidations: Rc<Cell<usize>>,
}

impl VisitorElement for LayoutInvalidationLeaf {
    fn debug_name(&self) -> &'static str {
        "LayoutInvalidationLeaf"
    }
}

impl EventElement for LayoutInvalidationLeaf {}

impl LayoutElement for LayoutInvalidationLeaf {
    fn invalidate_layout(&self) {
        self.invalidations.set(self.invalidations.get() + 1);
    }
}

impl Drawable for LayoutInvalidationLeaf {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for LayoutInvalidationLeaf {}

struct LayoutInvalidationBranch {
    child: AnyElement,
}

impl VisitorElement for LayoutInvalidationBranch {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "LayoutInvalidationBranch"
    }
}

impl EventElement for LayoutInvalidationBranch {}

impl LayoutElement for LayoutInvalidationBranch {
    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
}

impl Drawable for LayoutInvalidationBranch {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for LayoutInvalidationBranch {}

#[test]
fn erased_layout_invalidation_marks_once_without_walking_children() {
    let invalidations = Rc::new(Cell::new(0));
    let root = LayoutInvalidationBranch {
        child: LayoutInvalidationLeaf {
            invalidations: invalidations.clone(),
        }
        .boxed(),
    }
    .boxed();
    let generation = layout_invalidation_generation();

    root.invalidate_layout();

    assert!(layout_invalidation_generation() > generation);
    assert_eq!(invalidations.get(), 0);
}
