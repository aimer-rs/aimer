use smallvec::SmallVec;

use crate::node::FocusNode;

/// The candidate list gathered for one traversal or synchronization pass.
///
/// Sixteen candidates are stored inline, which covers every realistic screen —
/// a form with sixteen focusable fields is already unusual — so a frame that
/// resolves focus performs no allocation.
pub type FocusCandidates<Id> = SmallVec<[FocusCandidate<Id>; 16]>;

/// One focusable target, identified by the element it is attached to.
///
/// Candidates are cheap to clone: the node is a shared handle and the identity
/// is a copy type.
#[derive(Clone)]
pub struct FocusCandidate<Id> {
    /// Identity of the element the node is attached to.
    pub id: Id,
    /// Handle to the focus state of the target.
    pub node: FocusNode,
    /// Whether the target asks for focus on first attachment.
    pub autofocus: bool,
}

impl<Id: Copy + Eq> FocusCandidate<Id> {
    /// Creates a candidate for `id`, attached to `node`.
    #[inline]
    pub fn new(id: Id, node: FocusNode, autofocus: bool) -> Self {
        Self {
            id,
            node,
            autofocus,
        }
    }

    /// Returns whether this candidate is the same attachment as `id` + `node`.
    ///
    /// Both halves matter: an identity is transferred to the element that
    /// replaces it during reconciliation, so a matching identity alone does not
    /// prove the same focus target is still attached.
    #[inline]
    pub fn is_attached_to(&self, id: Id, node: &FocusNode) -> bool {
        self.id == id && self.node.ptr_eq(node)
    }
}

/// Returns the candidate index that focus moves to, in tree order.
///
/// `current` is the index of the focus owner, if it is among the candidates.
/// Traversal wraps, so moving forward past the last candidate returns to the
/// first. With nothing focused, forward traversal enters at the first candidate
/// and backward traversal enters at the last.
///
/// ```
/// use aimer_focus::next_focus_index;
///
/// assert_eq!(next_focus_index(3, Some(2), false), Some(0));
/// assert_eq!(next_focus_index(3, None, true), Some(2));
/// assert_eq!(next_focus_index(0, None, false), None);
/// ```
#[inline]
pub fn next_focus_index(count: usize, current: Option<usize>, reverse: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match (current, reverse) {
        (Some(0), true) | (None, true) => count - 1,
        (Some(index), true) => index - 1,
        (Some(index), false) => (index + 1) % count,
        (None, false) => 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_of_an_empty_candidate_list_has_no_target() {
        assert_eq!(next_focus_index(0, None, false), None);
        assert_eq!(next_focus_index(0, Some(0), true), None);
    }

    #[test]
    fn forward_traversal_enters_at_the_first_candidate_and_wraps() {
        assert_eq!(next_focus_index(3, None, false), Some(0));
        assert_eq!(next_focus_index(3, Some(0), false), Some(1));
        assert_eq!(next_focus_index(3, Some(2), false), Some(0));
    }

    #[test]
    fn backward_traversal_enters_at_the_last_candidate_and_wraps() {
        assert_eq!(next_focus_index(3, None, true), Some(2));
        assert_eq!(next_focus_index(3, Some(2), true), Some(1));
        assert_eq!(next_focus_index(3, Some(0), true), Some(2));
    }

    #[test]
    fn a_candidate_is_attached_only_to_its_own_identity_and_node() {
        let node = FocusNode::new();
        let candidate = FocusCandidate::new(7_u32, node.clone(), false);

        assert!(candidate.is_attached_to(7, &node.clone()));
        assert!(!candidate.is_attached_to(8, &node));
        assert!(!candidate.is_attached_to(7, &FocusNode::new()));
    }
}
