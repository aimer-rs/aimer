//! Which part of the element tree an advance of the tree generation touched.
//!
//! [`element_tree_generation`] says that *something* about the retained tree
//! changed, which is all a consumer can learn from it: it must assume the whole
//! tree did. Most advances are not that large. A rebuild replaces the child of
//! one stateful element with an equivalent subtree and leaves every element
//! outside it exactly where it was.
//!
//! The reconciliation that performs such a replacement records the root of the
//! subtree here, and advances only the *scoped* count. Anything else that moves
//! the generation (a hosted child added, a tree changed in a way no single root
//! describes) advances the *unscoped* count instead. A consumer that last saw
//! the tree at unscoped generation `g` and finds it still at `g` knows every
//! change since was one of the recorded subtree replacements, and may revisit
//! just those subtrees.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

/// Advances of the tree generation that no single replaced subtree describes.
static UNSCOPED_ELEMENT_TREE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// How many replacements the log keeps. A consumer that falls further behind
/// than this finds its cursor gone and must treat the tree as changed wholesale.
const MAX_LOGGED_ROOTS: usize = 256;

/// A consumer's position in the log of replaced subtrees.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScopedCursor(u64);

struct ScopedLog {
    /// Sequence number of `roots[0]`.
    first: u64,
    roots: Vec<ElementId>,
}

thread_local! {
    static LOG: RefCell<ScopedLog> = const { RefCell::new(ScopedLog { first: 0, roots: Vec::new() }) };
}

/// Records that the tree generation advanced for a change no subtree describes.
pub(super) fn note_unscoped_tree_change() {
    UNSCOPED_ELEMENT_TREE_GENERATION.fetch_add(1, Ordering::Release);
}

/// Records that the subtree rooted at `root` was replaced by an equivalent one.
///
/// `None` is a replacement whose root has no identity to report, which makes it
/// an unscoped change.
pub(super) fn note_scoped_rebuild(root: Option<ElementId>) {
    let Some(root) = root else {
        note_unscoped_tree_change();
        return;
    };
    LOG.with(|log| {
        let mut log = log.borrow_mut();
        log.roots.push(root);
        if log.roots.len() > MAX_LOGGED_ROOTS {
            let dropped = log.roots.len() / 2;
            log.roots.drain(..dropped);
            log.first += dropped as u64;
        }
    });
}

/// Returns how many advances of the tree generation were not contained in one
/// replaced subtree.
///
/// A consumer that saw the tree at this value and finds it unchanged may ask
/// [`scoped_rebuilds_since`] what else changed.
#[doc(hidden)]
#[inline]
pub fn unscoped_element_tree_generation() -> u64 {
    UNSCOPED_ELEMENT_TREE_GENERATION.load(Ordering::Acquire)
}

/// Returns the current end of the log of replaced subtrees.
#[doc(hidden)]
pub fn scoped_rebuild_cursor() -> ScopedCursor {
    LOG.with(|log| {
        let log = log.borrow();
        ScopedCursor(log.first + log.roots.len() as u64)
    })
}

/// Returns the roots of the subtrees replaced since `cursor`, oldest first.
///
/// The roots may repeat or nest. Returns `None` when the log no longer reaches
/// back to `cursor`, in which case the consumer cannot tell what changed and
/// must treat the whole tree as changed.
#[doc(hidden)]
pub fn scoped_rebuilds_since(cursor: ScopedCursor) -> Option<Vec<ElementId>> {
    LOG.with(|log| {
        let log = log.borrow();
        let offset = cursor.0.checked_sub(log.first)?;
        log.roots.get(offset as usize..).map(<[ElementId]>::to_vec)
    })
}

/// Rewrites the roots logged after `cursor` whose identity has since changed.
///
/// A replacement is logged under the identity its new root has when it is
/// logged. One nested in a larger rebuild (state carried into a freshly built
/// subtree) is logged before the enclosing reconciliation hands the new
/// elements the identities of the ones they replace, so its entry names an id
/// that no element has any more. `renamed` maps such an id to the one the root
/// ended up with. Without this the entry reads as a root nobody can place,
/// which is what a replacement under a genuinely new identity looks like.
pub(crate) fn rename_scoped_rebuilds_since(
    cursor: ScopedCursor,
    renamed: impl Fn(ElementId) -> Option<ElementId>,
) {
    LOG.with(|log| {
        let mut log = log.borrow_mut();
        // A cursor older than the log means every retained entry is newer.
        let from = cursor.0.saturating_sub(log.first) as usize;
        for root in log.roots.iter_mut().skip(from) {
            if let Some(id) = renamed(*root) {
                *root = id;
            }
        }
    });
}

/// Whether anything was recorded after `cursor`, including having lost track of
/// where `cursor` was.
#[doc(hidden)]
pub fn has_scoped_rebuilds_since(cursor: ScopedCursor) -> bool {
    LOG.with(|log| {
        let log = log.borrow();
        cursor.0 < log.first || cursor.0 < log.first + log.roots.len() as u64
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> ElementId {
        ElementId::next()
    }

    #[test]
    fn each_consumer_sees_the_replacements_after_its_own_cursor() {
        let _guard = test_generation_guard();
        let early = scoped_rebuild_cursor();
        let first = id();
        note_scoped_rebuild(Some(first));
        let late = scoped_rebuild_cursor();
        let second = id();
        note_scoped_rebuild(Some(second));

        assert_eq!(scoped_rebuilds_since(early), Some(vec![first, second]));
        assert_eq!(scoped_rebuilds_since(late), Some(vec![second]));
        assert_eq!(scoped_rebuilds_since(scoped_rebuild_cursor()), Some(vec![]));
        assert!(has_scoped_rebuilds_since(late));
        assert!(!has_scoped_rebuilds_since(scoped_rebuild_cursor()));
    }

    #[test]
    fn a_scoped_rebuild_leaves_the_unscoped_count_alone() {
        let _guard = test_generation_guard();
        let before = unscoped_element_tree_generation();

        note_scoped_rebuild(Some(id()));

        assert_eq!(unscoped_element_tree_generation(), before);
    }

    #[test]
    fn an_unscoped_change_moves_the_unscoped_count() {
        let _guard = test_generation_guard();
        let before = unscoped_element_tree_generation();

        note_unscoped_tree_change();
        note_scoped_rebuild(None);

        assert_eq!(unscoped_element_tree_generation(), before + 2);
    }

    #[test]
    fn a_consumer_that_falls_too_far_behind_loses_its_place() {
        let _guard = test_generation_guard();
        let stale = scoped_rebuild_cursor();

        for _ in 0..MAX_LOGGED_ROOTS * 2 {
            note_scoped_rebuild(Some(id()));
        }

        assert_eq!(scoped_rebuilds_since(stale), None);
        assert!(has_scoped_rebuilds_since(stale), "it must act, even without a list");
        let fresh = scoped_rebuild_cursor();
        note_scoped_rebuild(Some(id()));
        assert_eq!(scoped_rebuilds_since(fresh).map(|roots| roots.len()), Some(1));
    }

    #[test]
    fn renaming_touches_only_the_entries_after_the_cursor() {
        let _guard = test_generation_guard();
        let (before, nested, after) = (id(), id(), id());
        note_scoped_rebuild(Some(before));
        let cursor = scoped_rebuild_cursor();
        note_scoped_rebuild(Some(nested));
        note_scoped_rebuild(Some(after));
        let (renamed_nested, renamed_before) = (id(), id());

        rename_scoped_rebuilds_since(cursor, |root| {
            [(nested, renamed_nested), (before, renamed_before)]
                .into_iter()
                .find_map(|(from, to)| (from == root).then_some(to))
        });

        assert_eq!(
            scoped_rebuilds_since(cursor),
            Some(vec![renamed_nested, after])
        );
        let all = scoped_rebuilds_since(ScopedCursor(cursor.0 - 1));
        assert_eq!(all.and_then(|roots| roots.first().copied()), Some(before));
    }
}
