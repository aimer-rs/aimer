//! Regression tests for `EventDispatcher::patch_path_index`.
//!
//! The patch trusts the scoped-rebuild log to describe every change since the
//! index was built. A replacement whose root received a fresh identity is
//! logged under an id the index has never seen, and the subtree it replaced is
//! gone. These tests keep that dropped subtree alive so a wrong patch shows up
//! as a stale index instead of a use-after-free.

use std::cell::UnsafeCell;

use super::*;

/// Shared access to the child of a [`Slot`] after the slot is boxed.
#[derive(Clone)]
struct SlotHandle(Rc<UnsafeCell<AnyElement>>);

impl SlotHandle {
    /// Installs `new` the way a stateful rebuild does: reconcile identities,
    /// log the replacement, then swap the child. Returns the replaced child.
    fn replace(&self, new: AnyElement) -> AnyElement {
        // SAFETY: the tests are single-threaded and never hold a borrow of the
        // slot's child across this call.
        let child = unsafe { &mut *self.0.get() };
        reconcile_generated_tree(child.as_ref(), new.as_ref());
        std::mem::replace(child, new)
    }
}

/// A single-child element whose child can be swapped through its handle.
struct Slot {
    name: &'static str,
    child: Rc<UnsafeCell<AnyElement>>,
}

fn slot(name: &'static str, child: AnyElement) -> (AnyElement, SlotHandle) {
    let child = Rc::new(UnsafeCell::new(child));
    let element = Slot {
        name,
        child: child.clone(),
    }
    .boxed();
    (element, SlotHandle(child))
}

impl VisitorElement for Slot {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // SAFETY: see `SlotHandle::replace`; the borrow ends with the visit.
        visitor(unsafe { &*self.child.get() }.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        self.name
    }
}

impl EventElement for Slot {}
impl LayoutElement for Slot {}
impl Drawable for Slot {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for Slot {}

struct Leaf;

impl VisitorElement for Leaf {
    fn debug_name(&self) -> &'static str {
        "PatchLeaf"
    }
}

impl EventElement for Leaf {}
impl LayoutElement for Leaf {}
impl Drawable for Leaf {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for Leaf {}

fn synchronize(dispatcher: &mut EventDispatcher, root: &dyn Element) {
    begin_event_frame();
    root.set_subtree_generation(element_tree_generation());
    dispatcher.synchronize_path_index(root);
}

/// A rebuilt descendant is patched in place when nothing else changed. This is
/// the case the patch exists for, and it must keep working.
#[test]
fn a_compatible_replacement_is_patched_without_a_rebuild() {
    let (inner, inner_handle) = slot("PatchInner", Leaf.boxed());
    let (top, _) = slot("PatchTop", inner);
    let mut dispatcher = EventDispatcher::new();
    synchronize(&mut dispatcher, top.as_ref());
    assert_eq!(dispatcher.path_index_work(), (1, 0));

    let _old = inner_handle.replace(Leaf.boxed());
    synchronize(&mut dispatcher, top.as_ref());

    assert_eq!(dispatcher.path_index_work(), (1, 1));
}

/// A descendant is rebuilt, then an ancestor is swapped for an incompatible
/// element. The ancestor's root has a fresh identity the index does not know,
/// and the descendant's recorded parent lived inside the dropped subtree.
/// Patching would read that parent through a dangling pointer.
#[test]
fn an_ancestor_swapped_for_a_new_identity_forces_a_rebuild() {
    let (inner, inner_handle) = slot("PatchInner", Leaf.boxed());
    let (mid, _) = slot("PatchMid", inner);
    let (top, top_handle) = slot("PatchTop", mid);
    let mut dispatcher = EventDispatcher::new();
    synchronize(&mut dispatcher, top.as_ref());

    let _old_leaf = inner_handle.replace(Leaf.boxed());
    let replacement = slot("PatchReplacement", Leaf.boxed()).0;
    let replacement_id = replacement.id();
    // Held so the dropped subtree stays valid memory for the assertions.
    let _old_mid = top_handle.replace(replacement);
    synchronize(&mut dispatcher, top.as_ref());

    assert_eq!(
        dispatcher.path_index_work().1,
        0,
        "a replacement under an unknown identity must not be patched"
    );
    assert!(dispatcher.path_indices.contains_key(&replacement_id));
}

/// The replaced branch is unrelated to the known replacement, so no pointer
/// the patch dereferences is dangling, but the index would keep entries for the
/// dropped branch and event routing would follow them later.
#[test]
fn a_sibling_swapped_for_a_new_identity_does_not_leave_stale_entries() {
    let (left, left_handle) = slot("PatchLeft", Leaf.boxed());
    let (right_inner, _) = slot("PatchRightInner", Leaf.boxed());
    let (right, right_handle) = slot("PatchRight", right_inner);
    let root = IdentityBranch(vec![left, right]).boxed();
    let mut dispatcher = EventDispatcher::new();
    synchronize(&mut dispatcher, root.as_ref());

    let _old_left = left_handle.replace(Leaf.boxed());
    let replacement = slot("PatchReplacement", Leaf.boxed()).0;
    let replacement_id = replacement.id();
    let _old_right = right_handle.replace(replacement);
    synchronize(&mut dispatcher, root.as_ref());

    assert!(
        dispatcher.path_indices.contains_key(&replacement_id),
        "the index must know the element that replaced the dropped branch"
    );
}
