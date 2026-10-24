use core::hash::Hash;

use hashbrown::HashSet;
use smallvec::SmallVec;

use crate::node::{FocusNode, FocusRequest, focus_request_generation};
use crate::scope::{FocusTrapId, active_focus_trap, focus_trap_generation};
use crate::traversal::{FocusCandidate, next_focus_index};

/// The attachment that currently owns keyboard focus.
#[derive(Clone)]
pub struct FocusOwner<Id> {
    /// Identity of the owning element.
    pub id: Id,
    /// Handle to the focus state of the owner.
    pub node: FocusNode,
}

/// Returns the candidate `owner` is attached to, if it is still among them.
#[inline]
fn attached<Id: Copy + Eq>(
    candidates: &[FocusCandidate<Id>],
    owner: &FocusOwner<Id>,
) -> Option<FocusCandidate<Id>> {
    candidates
        .iter()
        .find(|candidate| candidate.is_attached_to(owner.id, &owner.node))
        .cloned()
}

impl<Id: Copy + Eq> FocusOwner<Id> {
    /// Returns whether this owner is the attachment described by `id` + `node`.
    #[inline]
    pub fn is_attached_to(&self, id: Id, node: &FocusNode) -> bool {
        self.id == id && self.node.ptr_eq(node)
    }
}

/// A change of focus ownership, described rather than delivered.
///
/// [`FocusManager`] knows which node lost focus and which gained it, but it
/// deliberately knows nothing about elements or events. It therefore reports
/// the change and leaves the host — the element tree in `aimer_widget` — to
/// resolve each identity and deliver its notification.
///
/// An unchanged frame produces an [empty](Self::is_empty) transition.
#[derive(Clone)]
pub struct FocusTransition<Id> {
    /// The attachment that stopped owning focus, if any.
    pub lost: Option<FocusOwner<Id>>,
    /// The attachment that started owning focus, if any.
    pub gained: Option<FocusOwner<Id>>,
}

impl<Id> FocusTransition<Id> {
    /// Returns a transition that changes nothing.
    #[inline]
    pub const fn unchanged() -> Self {
        Self {
            lost: None,
            gained: None,
        }
    }

    /// Returns whether ownership was left untouched.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.lost.is_none() && self.gained.is_none()
    }
}

/// Permission to resolve focus for one frame.
///
/// The token carries the counters observed when the frame was admitted, so they
/// are recorded at the end of the frame rather than read again. A request or a
/// trap change that happens *while* notifications are delivered therefore
/// raises a counter past the recorded value and is resolved on the next frame
/// instead of being swallowed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FocusSync {
    request_generation: u64,
    trap_generation: u64,
}

/// The single owner of keyboard focus for one element tree.
///
/// A focus manager decides *who* has focus; it never touches the tree itself.
/// The host walks its own structure to gather [`FocusCandidate`]s in traversal
/// order, hands them over, and applies the returned [`FocusTransition`]. That
/// split keeps focus policy — request ordering, autofocus, traversal — testable
/// without an element tree, and keeps this crate free of any dependency on one.
///
/// A frame resolves focus in three steps:
///
/// 1. [`begin_synchronization`](Self::begin_synchronization) reports whether
///    anything can have changed. An idle frame answers with a few integer
///    comparisons and no traversal at all.
/// 2. [`resolve`](Self::resolve) picks the target for the frame: the retained
///    owner, a first-attachment autofocus, or whatever the pending requests ask
///    for, applied in the order they were made.
/// 3. [`transition`](Self::transition) moves ownership to that target and
///    describes the change.
///
/// Two mechanisms confine focus to part of the application. A
/// [`set_trap`](Self::set_trap) region places the whole manager under a
/// [`FocusTrap`](crate::FocusTrap): while some other region traps focus this
/// manager owns nothing at all, which is how a modal presented over its own
/// dispatch root silences the tree underneath it. A
/// [`set_scope`](Self::set_scope) boundary confines focus *within* one tree, the
/// host restricting the candidates it hands over to the subtree of the scope.
/// Both remember the owner they displaced and restore it once the confinement
/// ends.
///
/// ```
/// use aimer_focus::{FocusCandidate, FocusManager, FocusNode};
///
/// let mut manager = FocusManager::<u32>::new();
/// let node = FocusNode::new();
/// let candidates = [FocusCandidate::new(1, node.clone(), false)];
///
/// node.request_focus();
/// let target = manager.resolve(&candidates);
/// let transition = manager.transition(target);
///
/// assert!(node.has_focus());
/// assert_eq!(manager.focused(), Some(1));
/// assert!(transition.gained.is_some());
/// ```
pub struct FocusManager<Id> {
    owner: Option<FocusOwner<Id>>,
    autofocus_seen: HashSet<Id>,
    tree_generation: u64,
    tree_root: Option<Id>,
    request_generation: u64,
    trap: Option<FocusTrapId>,
    trap_generation: u64,
    suspended: bool,
    scopes: SmallVec<[Id; 4]>,
    displaced: SmallVec<[Option<FocusOwner<Id>>; 4]>,
    pending_restore: Option<FocusOwner<Id>>,
}

impl<Id> Default for FocusManager<Id> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<Id> FocusManager<Id> {
    /// Creates a manager with no focus owner.
    ///
    /// The recorded tree generation starts saturated so the first frame always
    /// synchronizes.
    #[inline]
    pub fn new() -> Self {
        Self {
            owner: None,
            autofocus_seen: HashSet::new(),
            tree_generation: u64::MAX,
            tree_root: None,
            request_generation: 0,
            trap: None,
            trap_generation: 0,
            suspended: false,
            scopes: SmallVec::new_const(),
            displaced: SmallVec::new_const(),
            pending_restore: None,
        }
    }

    /// Places this manager inside `trap`, or outside every trap.
    ///
    /// A manager belongs to the region that was trapping when its dispatch root
    /// was created: the application tree belongs to no trap, and the content of
    /// a modal belongs to the trap the modal holds. Focus is granted only while
    /// that region is the [active](crate::active_focus_trap) one, so presenting
    /// a modal suspends every other region and dismissing it lets them resume.
    #[inline]
    pub fn set_trap(&mut self, trap: Option<FocusTrapId>) {
        self.trap = trap;
    }

    /// Returns the region this manager grants focus in, if it is confined.
    #[inline]
    pub const fn trap(&self) -> Option<FocusTrapId> {
        self.trap
    }

    /// Returns whether another region currently traps focus.
    ///
    /// A suspended manager owns nothing, resolves to nothing, and refuses
    /// traversal, which is what keeps `Tab` and typed text out of the
    /// application behind a modal.
    #[inline]
    pub fn is_suspended(&self) -> bool {
        active_focus_trap() != self.trap
    }

    /// Returns the attachment that owns focus, if any.
    #[inline]
    pub fn owner(&self) -> Option<&FocusOwner<Id>> {
        self.owner.as_ref()
    }
}

impl<Id: Copy + Eq + Hash> FocusManager<Id> {
    /// Returns the identity of the focus owner, if any.
    #[inline]
    pub fn focused(&self) -> Option<Id> {
        self.owner.as_ref().map(|owner| owner.id)
    }

    /// Returns whether `id` + `node` is the current focus owner.
    #[inline]
    pub fn owns(&self, id: Id, node: &FocusNode) -> bool {
        self.owner
            .as_ref()
            .is_some_and(|owner| owner.is_attached_to(id, node))
    }

    /// Drops the autofocus history of identities `keep` rejects.
    ///
    /// Autofocus fires once per attachment, so the manager remembers which
    /// identities it has already honoured. Identities that have left the tree
    /// are forgotten here to keep that memory bounded.
    #[inline]
    pub fn retain_history(&mut self, mut keep: impl FnMut(&Id) -> bool) {
        self.autofocus_seen.retain(|id| keep(id));
    }

    /// Reports whether focus has to be resolved for this frame.
    ///
    /// Returns the request generation to record once the frame is resolved, or
    /// `None` when the tree, its root, and the request counter are all
    /// unchanged since the last synchronization — the common case, which costs
    /// three comparisons.
    ///
    /// The generation is returned rather than read again at the end so that a
    /// request made *while* the transition is delivered is not swallowed: it
    /// raises the counter past the recorded value and is resolved next frame.
    #[inline]
    pub fn begin_synchronization(
        &self,
        tree_generation: u64,
        tree_root: Option<Id>,
    ) -> Option<FocusSync> {
        let sync = FocusSync {
            request_generation: focus_request_generation(),
            trap_generation: focus_trap_generation(),
        };
        (self.tree_generation != tree_generation
            || self.tree_root != tree_root
            || self.request_generation != sync.request_generation
            || self.trap_generation != sync.trap_generation)
            .then_some(sync)
    }

    /// Returns whether a focus request or a trap change has arrived since the
    /// last synchronization.
    ///
    /// Unlike [`begin_synchronization`](Self::begin_synchronization) this does
    /// not look at the tree, so a frame loop can ask after every frame for the
    /// price of two integer comparisons and only settle focus when the answer
    /// is yes.
    #[inline]
    pub fn has_unsettled_requests(&self) -> bool {
        self.request_generation != focus_request_generation()
            || self.trap_generation != focus_trap_generation()
    }

    /// Records that focus is resolved for `tree_generation` and `tree_root`.
    ///
    /// `sync` must be the token returned by
    /// [`begin_synchronization`](Self::begin_synchronization) for this frame.
    #[inline]
    pub fn mark_synchronized(
        &mut self,
        tree_generation: u64,
        tree_root: Option<Id>,
        sync: FocusSync,
    ) {
        self.tree_generation = tree_generation;
        self.tree_root = tree_root;
        self.request_generation = sync.request_generation;
        self.trap_generation = sync.trap_generation;
    }

    /// Confines focus to the trapping scope rooted at `active`, if there is one.
    ///
    /// The host reports the innermost scope that traps focus in its tree, and
    /// restricts the candidates it hands to [`resolve`](Self::resolve) and
    /// [`traverse`](Self::traverse) to that subtree — which is the whole of the
    /// confinement, since a target that is never offered can neither be given
    /// focus nor be reached by `Tab`. What this method adds is memory: entering
    /// a scope records the owner it displaces, and leaving it — whether the
    /// scope was dismissed or simply removed from the tree — restores that
    /// owner if it is still attached.
    ///
    /// Scopes nest. Leaving several at once, as a rebuild that removes a whole
    /// branch does, restores the owner displaced by the outermost of them.
    pub fn set_scope(&mut self, active: Option<Id>) {
        if self.scopes.last().copied() == active {
            return;
        }

        if let Some(active) = active
            && !self.scopes.contains(&active)
        {
            self.scopes.push(active);
            self.displaced.push(self.owner.clone());
            return;
        }

        while self.scopes.last().copied() != active {
            self.scopes.pop();
            self.pending_restore = self.displaced.pop().flatten();
        }
    }

    /// Returns the trapping scope focus is currently confined to, if any.
    #[inline]
    pub fn scope(&self) -> Option<Id> {
        self.scopes.last().copied()
    }

    /// Reconciles suspension with the active trap, reporting whether focus is
    /// suspended.
    ///
    /// Crossing into suspension records the displaced owner, and crossing back
    /// out queues it for restoration; staying on either side does nothing.
    fn reconcile_trap(&mut self) -> bool {
        let suspended = self.is_suspended();
        if suspended == self.suspended {
            return suspended;
        }

        self.suspended = suspended;
        if suspended {
            self.displaced.push(self.owner.clone());
        } else {
            self.pending_restore = self.displaced.pop().flatten();
        }
        suspended
    }

    /// Returns the target that should own focus, consuming pending requests.
    ///
    /// The starting point is the retained owner, so a rebuild that reattaches
    /// the same node keeps focus. A confinement that has just ended overrides
    /// it: the owner displaced when the trap or scope was entered is put back,
    /// which is why focus returns to the button that opened a dialog rather than
    /// staying on whatever the dialog left focused. If neither is attached, the
    /// first candidate asking for autofocus takes it — once per identity, so
    /// dismissing that focus does not immediately reclaim it. Pending node
    /// requests are then applied in the order they were made, letting the newest
    /// request of the frame win wherever its node sits in the tree.
    pub fn resolve(&mut self, candidates: &[FocusCandidate<Id>]) -> Option<FocusCandidate<Id>> {
        if self.reconcile_trap() {
            return None;
        }

        let restore = self.pending_restore.take();
        let mut target = restore
            .as_ref()
            .and_then(|owner| attached(candidates, owner))
            .or_else(|| {
                self.owner
                    .as_ref()
                    .and_then(|owner| attached(candidates, owner))
            });

        if target.is_none() {
            for candidate in candidates {
                if candidate.autofocus
                    && self.autofocus_seen.insert(candidate.id)
                    && target.is_none()
                {
                    target = Some(candidate.clone());
                }
            }
        }

        let mut requests: SmallVec<[&FocusCandidate<Id>; 16]> = candidates
            .iter()
            .filter(|candidate| candidate.node.request().is_some())
            .collect();
        if requests.is_empty() {
            return target;
        }
        requests.sort_unstable_by_key(|candidate| {
            candidate
                .node
                .request()
                .map_or(u64::MAX, |request| request.order())
        });

        for candidate in requests {
            let Some(request) = candidate.node.request() else {
                continue;
            };
            candidate.node.clear_request();
            match request {
                FocusRequest::Focus(_) => target = Some(candidate.clone()),
                FocusRequest::Unfocus(_) => {
                    if target
                        .as_ref()
                        .is_some_and(|target| target.node.ptr_eq(&candidate.node))
                    {
                        target = None;
                    }
                }
            }
        }
        target
    }

    /// Moves ownership to `target` and describes the change.
    ///
    /// The `has_focus` flag of both nodes is updated here, so a widget can read
    /// its own node without waiting for the notification. Handing focus to the
    /// attachment that already owns it changes nothing and reports an
    /// [empty](FocusTransition::is_empty) transition.
    ///
    /// While another region [traps](Self::is_suspended) focus no target is
    /// granted, whatever the host offers. Ownership is decided by
    /// [`resolve`](Self::resolve), but focus is also handed out directly — a
    /// pointer press on a field is not a request — and a suspended region must
    /// not take the keyboard back through that door.
    pub fn transition(&mut self, target: Option<FocusCandidate<Id>>) -> FocusTransition<Id> {
        let target = target.filter(|_| !self.is_suspended());
        if let (Some(owner), Some(target)) = (self.owner.as_ref(), target.as_ref())
            && owner.is_attached_to(target.id, &target.node)
        {
            return FocusTransition::unchanged();
        }

        let lost = self.owner.take().inspect(|owner| {
            owner.node.set_focused(false);
        });
        let gained = target.map(|target| {
            target.node.set_focused(true);
            FocusOwner {
                id: target.id,
                node: target.node,
            }
        });
        self.owner = gained.clone();

        FocusTransition { lost, gained }
    }

    /// Returns the candidate focus moves to when traversing the list.
    ///
    /// Traversal follows the order of `candidates` and wraps at both ends;
    /// `reverse` walks backwards, as Shift-Tab does. Returns `None` when there
    /// is nothing focusable, and while another region
    /// [traps](Self::is_suspended) focus — so a host that finds no target can
    /// pass `Tab` on to whatever does own the keyboard.
    pub fn traverse(
        &self,
        candidates: &[FocusCandidate<Id>],
        reverse: bool,
    ) -> Option<FocusCandidate<Id>> {
        if self.is_suspended() {
            return None;
        }
        let current = self.owner.as_ref().and_then(|owner| {
            candidates
                .iter()
                .position(|candidate| candidate.is_attached_to(owner.id, &owner.node))
        });
        let next = next_focus_index(candidates.len(), current, reverse)?;
        candidates.get(next).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::request_focus_resolution;
    use crate::scope::FocusTrap;

    fn candidate(id: u32, node: &FocusNode) -> FocusCandidate<u32> {
        FocusCandidate::new(id, node.clone(), false)
    }

    fn autofocus_candidate(id: u32, node: &FocusNode) -> FocusCandidate<u32> {
        FocusCandidate::new(id, node.clone(), true)
    }

    fn synchronize(
        manager: &mut FocusManager<u32>,
        candidates: &[FocusCandidate<u32>],
    ) -> FocusTransition<u32> {
        let target = manager.resolve(candidates);
        manager.transition(target)
    }

    #[test]
    fn a_focus_request_takes_ownership_and_marks_the_node() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();
        let candidates = [candidate(1, &node)];

        node.request_focus();
        let transition = synchronize(&mut manager, &candidates);

        assert_eq!(manager.focused(), Some(1));
        assert!(node.has_focus());
        assert!(transition.lost.is_none());
        assert!(
            transition
                .gained
                .is_some_and(|gained| gained.is_attached_to(1, &node))
        );
        assert!(node.request().is_none());
    }

    #[test]
    fn ownership_is_retained_while_the_same_node_stays_attached() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();
        let candidates = [candidate(1, &node), candidate(2, &FocusNode::new())];

        node.request_focus();
        synchronize(&mut manager, &candidates);
        let transition = synchronize(&mut manager, &candidates);

        assert!(transition.is_empty());
        assert_eq!(manager.focused(), Some(1));
    }

    #[test]
    fn ownership_is_dropped_when_the_owner_leaves_the_candidate_list() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();

        node.request_focus();
        synchronize(&mut manager, &[candidate(1, &node)]);
        let transition = synchronize(&mut manager, &[]);

        assert!(transition.lost.is_some());
        assert!(transition.gained.is_none());
        assert!(!node.has_focus());
        assert_eq!(manager.focused(), None);
    }

    #[test]
    fn the_last_request_of_a_frame_wins_regardless_of_tree_order() {
        let mut manager = FocusManager::<u32>::new();
        let first = FocusNode::new();
        let second = FocusNode::new();
        let candidates = [candidate(1, &first), candidate(2, &second)];

        second.request_focus();
        first.request_focus();
        synchronize(&mut manager, &candidates);

        assert_eq!(manager.focused(), Some(1));
        assert!(first.has_focus());
        assert!(!second.has_focus());
    }

    #[test]
    fn unfocusing_the_owner_leaves_nothing_focused() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();
        let candidates = [candidate(1, &node)];

        node.request_focus();
        synchronize(&mut manager, &candidates);
        node.unfocus();
        let transition = synchronize(&mut manager, &candidates);

        assert_eq!(manager.focused(), None);
        assert!(!node.has_focus());
        assert!(transition.lost.is_some());
    }

    #[test]
    fn a_transition_reports_both_sides_of_a_handover() {
        let mut manager = FocusManager::<u32>::new();
        let first = FocusNode::new();
        let second = FocusNode::new();

        first.request_focus();
        synchronize(&mut manager, &[candidate(1, &first)]);
        second.request_focus();
        let transition = synchronize(&mut manager, &[candidate(1, &first), candidate(2, &second)]);

        assert!(
            transition
                .lost
                .is_some_and(|lost| lost.is_attached_to(1, &first))
        );
        assert!(
            transition
                .gained
                .is_some_and(|gained| gained.is_attached_to(2, &second))
        );
        assert!(!first.has_focus());
        assert!(second.has_focus());
    }

    #[test]
    fn autofocus_is_honoured_once_per_identity() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();
        let candidates = [autofocus_candidate(1, &node)];

        synchronize(&mut manager, &candidates);
        assert_eq!(manager.focused(), Some(1));

        node.unfocus();
        synchronize(&mut manager, &candidates);
        assert_eq!(manager.focused(), None);

        synchronize(&mut manager, &candidates);
        assert_eq!(manager.focused(), None);
    }

    #[test]
    fn autofocus_history_is_forgotten_for_identities_that_left_the_tree() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();
        let candidates = [autofocus_candidate(1, &node)];

        synchronize(&mut manager, &candidates);
        node.unfocus();
        synchronize(&mut manager, &candidates);
        manager.retain_history(|_| false);
        synchronize(&mut manager, &candidates);

        assert_eq!(manager.focused(), Some(1));
    }

    #[test]
    fn traversal_walks_the_candidate_order_from_the_owner() {
        let mut manager = FocusManager::<u32>::new();
        let first = FocusNode::new();
        let second = FocusNode::new();
        let candidates = [candidate(1, &first), candidate(2, &second)];

        assert!(
            manager
                .traverse(&candidates, false)
                .is_some_and(|target| target.id == 1)
        );

        first.request_focus();
        synchronize(&mut manager, &candidates);

        assert!(
            manager
                .traverse(&candidates, false)
                .is_some_and(|target| target.id == 2)
        );
        assert!(
            manager
                .traverse(&candidates, true)
                .is_some_and(|target| target.id == 2)
        );
        assert!(manager.traverse(&[], false).is_none());
    }

    #[test]
    fn synchronization_is_skipped_until_the_tree_or_a_request_changes() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();

        let sync = manager
            .begin_synchronization(1, Some(0))
            .expect("the first frame always synchronizes");
        manager.mark_synchronized(1, Some(0), sync);

        assert!(manager.begin_synchronization(1, Some(0)).is_none());
        assert!(manager.begin_synchronization(2, Some(0)).is_some());
        assert!(manager.begin_synchronization(1, Some(9)).is_some());

        node.request_focus();
        assert!(manager.begin_synchronization(1, Some(0)).is_some());
    }

    #[test]
    fn acquiring_a_trap_makes_the_next_frame_resolve_focus() {
        let mut manager = FocusManager::<u32>::new();

        let sync = manager
            .begin_synchronization(1, Some(0))
            .expect("the first frame always synchronizes");
        manager.mark_synchronized(1, Some(0), sync);
        assert!(manager.begin_synchronization(1, Some(0)).is_none());

        let trap = FocusTrap::acquire();
        assert!(manager.begin_synchronization(1, Some(0)).is_some());

        let sync = manager
            .begin_synchronization(1, Some(0))
            .expect("the acquired trap has not been accounted for yet");
        manager.mark_synchronized(1, Some(0), sync);
        assert!(manager.begin_synchronization(1, Some(0)).is_none());

        drop(trap);
        assert!(manager.begin_synchronization(1, Some(0)).is_some());
    }

    #[test]
    fn a_trap_elsewhere_suspends_the_manager_and_releases_its_owner() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();
        let candidates = [candidate(1, &node)];

        node.request_focus();
        synchronize(&mut manager, &candidates);

        let _trap = FocusTrap::acquire();
        let transition = synchronize(&mut manager, &candidates);

        assert!(manager.is_suspended());
        assert_eq!(manager.focused(), None);
        assert!(!node.has_focus());
        assert!(
            transition
                .lost
                .is_some_and(|lost| lost.is_attached_to(1, &node))
        );
    }

    #[test]
    fn a_suspended_manager_grants_nothing_and_refuses_traversal() {
        let mut manager = FocusManager::<u32>::new();
        let first = FocusNode::new();
        let second = FocusNode::new();
        let candidates = [candidate(1, &first), candidate(2, &second)];

        let _trap = FocusTrap::acquire();
        first.request_focus();
        let transition = synchronize(&mut manager, &candidates);

        assert!(transition.is_empty());
        assert_eq!(manager.focused(), None);
        assert!(!first.has_focus());
        assert!(manager.traverse(&candidates, false).is_none());
    }

    #[test]
    fn releasing_the_trap_restores_the_suspended_owner() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();
        let candidates = [candidate(1, &node), candidate(2, &FocusNode::new())];

        node.request_focus();
        synchronize(&mut manager, &candidates);

        let trap = FocusTrap::acquire();
        synchronize(&mut manager, &candidates);
        drop(trap);
        let transition = synchronize(&mut manager, &candidates);

        assert!(!manager.is_suspended());
        assert_eq!(manager.focused(), Some(1));
        assert!(node.has_focus());
        assert!(
            transition
                .gained
                .is_some_and(|gained| gained.is_attached_to(1, &node))
        );
    }

    #[test]
    fn nested_traps_suspend_once_and_restore_the_same_owner() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();
        let candidates = [candidate(1, &node)];

        node.request_focus();
        synchronize(&mut manager, &candidates);

        let outer = FocusTrap::acquire();
        synchronize(&mut manager, &candidates);
        let inner = FocusTrap::acquire();
        synchronize(&mut manager, &candidates);

        drop(inner);
        synchronize(&mut manager, &candidates);
        assert_eq!(
            manager.focused(),
            None,
            "the outer trap still confines focus"
        );

        drop(outer);
        synchronize(&mut manager, &candidates);

        assert_eq!(manager.focused(), Some(1));
    }

    #[test]
    fn a_suspended_manager_refuses_focus_handed_to_it_directly() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();

        let _trap = FocusTrap::acquire();
        let transition = manager.transition(Some(candidate(1, &node)));

        assert!(transition.is_empty());
        assert_eq!(manager.focused(), None);
        assert!(!node.has_focus());
    }

    #[test]
    fn a_manager_inside_the_active_trap_keeps_granting_focus() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();
        let candidates = [candidate(1, &node)];

        let trap = FocusTrap::acquire();
        manager.set_trap(Some(trap.id()));
        node.request_focus();
        synchronize(&mut manager, &candidates);

        assert!(!manager.is_suspended());
        assert_eq!(manager.focused(), Some(1));

        // Its own trap released, the region is gone and owns nothing.
        drop(trap);
        synchronize(&mut manager, &candidates);
        assert_eq!(manager.focused(), None);
    }

    #[test]
    fn a_restored_owner_that_left_the_tree_is_not_reinstated() {
        let mut manager = FocusManager::<u32>::new();
        let node = FocusNode::new();

        node.request_focus();
        synchronize(&mut manager, &[candidate(1, &node)]);

        let trap = FocusTrap::acquire();
        synchronize(&mut manager, &[candidate(1, &node)]);
        drop(trap);
        synchronize(&mut manager, &[]);

        assert_eq!(manager.focused(), None);
        assert!(!node.has_focus());
    }

    #[test]
    fn entering_a_trapping_scope_displaces_the_owner_outside_it() {
        let mut manager = FocusManager::<u32>::new();
        let outside = FocusNode::new();
        let inside = FocusNode::new();

        outside.request_focus();
        synchronize(&mut manager, &[candidate(1, &outside)]);

        // The host confines the candidate list to the scope subtree.
        manager.set_scope(Some(7));
        let transition = synchronize(&mut manager, &[candidate(2, &inside)]);

        assert_eq!(manager.scope(), Some(7));
        assert_eq!(manager.focused(), None);
        assert!(!outside.has_focus());
        assert!(
            transition
                .lost
                .is_some_and(|lost| lost.is_attached_to(1, &outside))
        );
    }

    #[test]
    fn leaving_a_trapping_scope_restores_the_owner_it_displaced() {
        let mut manager = FocusManager::<u32>::new();
        let outside = FocusNode::new();
        let inside = FocusNode::new();
        let whole_tree = [candidate(1, &outside), candidate(2, &inside)];

        outside.request_focus();
        synchronize(&mut manager, &whole_tree);

        manager.set_scope(Some(7));
        inside.request_focus();
        synchronize(&mut manager, &[candidate(2, &inside)]);
        assert_eq!(manager.focused(), Some(2));

        manager.set_scope(None);
        let transition = synchronize(&mut manager, &whole_tree);

        assert_eq!(manager.focused(), Some(1));
        assert!(outside.has_focus());
        assert!(!inside.has_focus());
        assert!(
            transition
                .lost
                .is_some_and(|lost| lost.is_attached_to(2, &inside))
        );
    }

    #[test]
    fn leaving_nested_scopes_at_once_restores_the_outermost_displaced_owner() {
        let mut manager = FocusManager::<u32>::new();
        let outside = FocusNode::new();
        let outer = FocusNode::new();
        let inner = FocusNode::new();
        let whole_tree = [
            candidate(1, &outside),
            candidate(2, &outer),
            candidate(3, &inner),
        ];

        outside.request_focus();
        synchronize(&mut manager, &whole_tree);

        manager.set_scope(Some(20));
        outer.request_focus();
        synchronize(&mut manager, &[candidate(2, &outer), candidate(3, &inner)]);

        manager.set_scope(Some(30));
        inner.request_focus();
        synchronize(&mut manager, &[candidate(3, &inner)]);
        assert_eq!(manager.focused(), Some(3));

        manager.set_scope(None);
        synchronize(&mut manager, &whole_tree);

        assert_eq!(manager.focused(), Some(1));
    }

    #[test]
    fn leaving_only_the_innermost_scope_restores_within_the_enclosing_one() {
        let mut manager = FocusManager::<u32>::new();
        let outer = FocusNode::new();
        let inner = FocusNode::new();
        let outer_scope = [candidate(2, &outer), candidate(3, &inner)];

        manager.set_scope(Some(20));
        outer.request_focus();
        synchronize(&mut manager, &outer_scope);

        manager.set_scope(Some(30));
        inner.request_focus();
        synchronize(&mut manager, &[candidate(3, &inner)]);

        manager.set_scope(Some(20));
        synchronize(&mut manager, &outer_scope);

        assert_eq!(manager.scope(), Some(20));
        assert_eq!(manager.focused(), Some(2));
        assert!(outer.has_focus());
    }
    /// A frame that built an autofocusing field must learn that focus is owed
    /// without walking the tree, so the answer reads counters only.
    #[test]
    fn a_manager_reports_requests_nobody_has_settled() {
        let mut manager = FocusManager::<u32>::new();
        assert!(!manager.has_unsettled_requests());

        request_focus_resolution();
        assert!(manager.has_unsettled_requests());

        let sync = manager
            .begin_synchronization(0, None)
            .expect("a new request opens the gate");
        manager.mark_synchronized(0, None, sync);
        assert!(!manager.has_unsettled_requests());
    }
}
