use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use aimer_attribute::position::Vec2d;
use smallvec::SmallVec;

use crate::{Element, ElementId, Key};
// Only the intra-doc links below name these.
#[cfg(doc)]
use crate::{Drawable, EventElement, LayoutElement, Rebuildable};

#[derive(Clone, Copy)]
struct StructuralChildPointer<'a>(*const (dyn Element + 'a));

impl PartialEq for StructuralChildPointer<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for StructuralChildPointer<'_> {}

impl Hash for StructuralChildPointer<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0 as *const ()).hash(state);
    }
}

/// How the rest of the framework walks an element's children, and how it
/// names the element itself.
///
/// This is the base of the [`Element`] family: [`EventElement`],
/// [`LayoutElement`], [`Rebuildable`] and [`Drawable`] are all defined in terms
/// of it, and their default methods reach the subtree through
/// [`visit_children`](VisitorElement::visit_children). An element that has
/// children therefore implements this trait first, and the defaults of the
/// other traits (invalidating layout, marking for rebuild, rebuilding dirty
/// descendants) then work without further code.
///
/// Only [`debug_name`](VisitorElement::debug_name) is required. The remaining
/// methods are identity hooks used by reconciliation and the other views of the
/// children; most elements keep their defaults.
///
/// Besides [`visit_children`](VisitorElement::visit_children), which is the
/// visual view, the trait owns the views the input system walks:
/// [`event_children`](VisitorElement::event_children),
/// [`structural_children`](VisitorElement::structural_children),
/// [`focus_children`](VisitorElement::focus_children), the hit-test family
/// ([`hit_test_children`](VisitorElement::hit_test_children) and its reversed and
/// position-aware variants) and
/// [`occlusion_children_at`](VisitorElement::occlusion_children_at). Each
/// defaults to the view before it, so an element overrides one only when its
/// children differ for that purpose.
pub trait VisitorElement {
    /// Visits the children that are painted, laid out and rebuilt with this
    /// element, in structural order.
    ///
    /// This is the *visual* view of the subtree. The defaults of
    /// [`LayoutElement::invalidate_layout`], [`Rebuildable::mark_needs_rebuild`]
    /// and [`Rebuildable::rebuild_if_dirty`] walk it, so an element that holds
    /// children and does not report them here is silently skipped by all three.
    /// Event delivery has its own views, [`VisitorElement::event_children`] and
    /// [`VisitorElement::hit_test_children`], which default to this one.
    ///
    /// A leaf keeps the default, which visits nothing. The visitor is called
    /// synchronously and must not be retained.
    #[allow(unused_variables)]
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {}

    /// Visits children in retained paint order and reports each child's stable
    /// source index for layout-aware v2 geometry.
    ///
    /// Most elements paint children in structural order. Layered containers
    /// can override this visitor while keeping `visit_children` unchanged for
    /// reconciliation and event structure.
    #[doc(hidden)]
    fn visit_retained_children<'a>(
        &'a self,
        visitor: &mut dyn FnMut(usize, &'a dyn Element),
    ) {
        let mut index = 0;
        self.visit_children(&mut |child| {
            visitor(index, child);
            index += 1;
        });
    }

    /// Visit children for event dispatch. By default delegates to
    /// `visit_children`. Override this when `visit_children` is not
    /// implemented (e.g. because the element handles its own child
    /// rendering) but events still need to reach the children.
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.visit_children(visitor);
    }

    /// Visit the children that make up this element's structural tree.
    ///
    /// Focus traversal and reconciliation need the union of the event and
    /// visual child views. Elements whose two views have one canonical child
    /// source can override this method to avoid walking and de-duplicating
    /// that source twice. An override must visit every child exposed by either
    /// [`Self::event_children`] or [`VisitorElement::visit_children`] exactly
    /// once, in structural order.
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // CALLED.update(|c| c + 1);
        let mut children: SmallVec<[&'a dyn Element; 8]> = SmallVec::new();
        self.event_children(&mut |child| children.push(child));
        let event_child_count = children.len();
        let mut visual_index = 0;
        let mut seen: Option<HashSet<StructuralChildPointer<'a>>> = None;
        self.visit_children(&mut |child| {
            if visual_index < event_child_count
                && std::ptr::eq(children[visual_index], child)
            {
                visual_index += 1;
                return
            }

            visual_index += 1;
            let child_pointer = StructuralChildPointer(std::ptr::from_ref(child));
            let already_seen = if let Some(seen) = seen.as_mut() {
                !seen.insert(child_pointer)
            } else if children.len() >= 16 {
                let mut pointers: HashSet<StructuralChildPointer<'a>> = children
                    .iter()
                    .map(|existing| StructuralChildPointer(std::ptr::from_ref(*existing)))
                    .collect();
                let already_seen = !pointers.insert(child_pointer);
                seen = Some(pointers);
                already_seen
            } else {
                children
                    .iter()
                    .any(|existing| std::ptr::eq(*existing, child))
            };
            if !already_seen {
                children.push(child);
            }
        });
        for child in children {
            visitor(child);
        }
    }

    /// Visit children that can participate in focus traversal.
    ///
    /// This defaults to the full structural tree because most elements use the
    /// same children for painting, events, reconciliation, and focus. A visual
    /// overlay that must remain part of reconciliation but cannot own focus can
    /// override this to omit that overlay while leaving
    /// [`VisitorElement::structural_children`] complete.
    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.structural_children(visitor);
    }

    /// Visit children that a pointer could plausibly hit.
    ///
    /// Position-based dispatch uses this hook instead of
    /// [`VisitorElement::event_children`], which lets an element that paints only
    /// part of its children — a clipped or virtualized list — keep the
    /// hit-test walk proportional to what is on screen. Focus-directed and
    /// broadcast delivery deliberately keep using `event_children`, so a child
    /// that was never painted still receives keyboard input and lifecycle
    /// notifications.
    ///
    /// The default forwards to `event_children`; overriding it is purely an
    /// optimization and must never hide a child that was actually drawn.
    fn hit_test_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.event_children(visitor);
    }

    /// Visit hit-test children in reverse order for callers that need a
    /// topmost-first stream.
    ///
    /// The default collects the ordinary hit-test view and reverses it, which
    /// preserves the existing topmost-first behavior for custom elements. Large
    /// child containers can override this to walk their retained storage in
    /// reverse directly and avoid a temporary child buffer.
    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        let mut children: SmallVec<[&'a dyn Element; 32]> = SmallVec::new();
        self.hit_test_children(&mut |child| children.push(child));
        for child in children.into_iter().rev() {
            visitor(child);
        }
    }

    /// Visit hit-test children that could contain a pointer at `pos`.
    ///
    /// The default delegates to the existing position-independent hook, so
    /// current element implementations retain their behavior. Containers with
    /// a retained spatial index may override this to skip known-outside
    /// children without affecting focus or broadcast traversal.
    #[inline]
    fn hit_test_children_at<'a>(&'a self, _pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.hit_test_children(visitor);
    }

    /// Visit position-aware hit-test children in routed topmost-first order for
    /// callers that request a reverse stream directly.
    ///
    /// The routed dispatcher uses [`VisitorElement::hit_test_children_at`] with
    /// one shared reverse scratch stack, so it does not call this default
    /// buffer. Direct callers still get the historical behavior, and large
    /// containers can override this method to query retained bounds directly.
    #[inline]
    fn hit_test_children_at_reversed<'a>(
        &'a self,
        _pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.hit_test_children_reversed(visitor);
    }

    /// Visits the children that [`crate::pointer_occluded_at`] searches for an
    /// occluder at `pos`.
    ///
    /// The default is the hit-test view, so a spatially indexed container keeps
    /// the search as narrow as its routing. An element that routes its child by
    /// hand and so hides it from that view should visit the child here.
    #[inline]
    fn occlusion_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.hit_test_children_at(pos, visitor);
    }

    /// Names the element's type for diagnostics and reconciliation.
    ///
    /// Reconciliation treats two elements as the same widget only when their
    /// concrete type, this name and their [`reconciliation_key`] agree, so the
    /// name must be stable for a given kind of element and distinct between
    /// kinds that can occupy the same position. It is also what appears in
    /// frame statistics and unmapped-draw reports; a short type name such as
    /// `"Container"` is conventional.
    ///
    /// [`reconciliation_key`]: VisitorElement::reconciliation_key
    fn debug_name(&self) -> &'static str;

    /// Returns the `TypeId` of the concrete element type, enabling runtime
    /// type checks without the `Reconcilable` trait. Used by the state-carrying
    /// logic to identify `StatefulElement`s in the element tree.
    fn element_type_id(&self) -> std::any::TypeId {
        // Default returns a dummy that never matches any real element type.
        std::any::TypeId::of::<()>()
    }

    /// Returns the key used to preserve logical identity during reconciliation.
    fn reconciliation_key(&self) -> Option<&Key> {
        None
    }

    /// Names the retained element this one is a placement of, if it is one.
    ///
    /// Two placements that name the same element stand for one subtree seen
    /// twice, so there is nothing to carry, match or compare between them.
    #[doc(hidden)]
    fn retained_identity(&self) -> Option<usize> {
        None
    }

    /// Whether this retained element is a slot its owner declared with
    /// `BuildContext::retain`.
    ///
    /// A slot is a part the owner named and keeps, so it is never the stand-in
    /// for a different retained part that happened to occupy its position.
    #[doc(hidden)]
    fn is_retained_slot(&self) -> bool {
        false
    }

    /// Returns the identity this element carries, if it carries one.
    ///
    /// Elements are given an [`ElementId`] when they are erased with
    /// [`Element::boxed`]; the erasing wrapper answers this, so a concrete
    /// element keeps the default. [`Element::id`] is the infallible form for
    /// elements that are known to be erased.
    #[doc(hidden)]
    fn element_id(&self) -> Option<ElementId> {
        None
    }

    /// Replaces this element's identity.
    ///
    /// Reconciliation calls this to hand the id of the element being replaced
    /// to its compatible successor, which is what keeps an element's identity
    /// stable across rebuilds. Only the erasing wrapper stores the id; the
    /// default ignores it.
    #[doc(hidden)]
    fn set_element_id(&self, _id: ElementId) {}
}
