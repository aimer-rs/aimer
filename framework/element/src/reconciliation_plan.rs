use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::hash::{BuildHasherDefault, Hasher};

use smallvec::SmallVec;

use crate::base::BuildContext;
use crate::components::element::{
    Element, ElementId, ScopedCursor, complete_generated_tree_reconciliation,
    has_scoped_rebuilds_since, identities_are_compatible, rename_scoped_rebuilds_since,
    scoped_rebuild_cursor, structural_children,
};

/// A hasher for the pointers and keys a plan sets up.
///
/// A plan builds several sets of element addresses for every reconciliation, and
/// an animation reconciles a subtree on every frame. The addresses come from
/// this process, so nothing is gained by making them hard to predict, and the
/// default hasher spent more time on them than the walk did. This multiplies
/// each word and keeps the well mixed high bits, which is what the table indexes
/// by, so aligned addresses spread as well as any other.
#[derive(Default)]
struct PlanHasher(u64);

impl PlanHasher {
    const MULTIPLIER: u64 = 0xf135_7aea_2e62_a9c5;

    #[inline]
    fn mix(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(Self::MULTIPLIER);
    }
}

impl Hasher for PlanHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.mix(u64::from_ne_bytes(chunk.try_into().expect("a chunk of eight bytes")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            self.mix(u64::from_ne_bytes(last) ^ ((rest.len() as u64) << 56));
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.mix(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.mix(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.mix(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.mix(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0.rotate_left(26)
    }
}

type PlanBuildHasher = BuildHasherDefault<PlanHasher>;
type PlanSet<T> = HashSet<T, PlanBuildHasher>;

/// Describes how one compatible element pair was selected during planning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconciliationMatchKind {
    /// The old and candidate root elements are compatible.
    Root,
    /// Siblings were paired by an equal reconciliation key.
    Keyed,
    /// Unkeyed siblings were paired at the same position.
    Positional,
}

/// One immutable old-to-candidate element match.
pub struct ReconciliationMatch<'a> {
    old: &'a dyn Element,
    new: &'a dyn Element,
    kind: ReconciliationMatchKind,
}

impl ReconciliationMatch<'_> {
    /// Returns how this pair was selected.
    #[inline]
    pub const fn kind(&self) -> ReconciliationMatchKind {
        self.kind
    }

    /// Returns the logical identity that commit transfers from the old node.
    #[inline]
    pub fn old_id(&self) -> ElementId {
        self.old.id()
    }

    /// Returns the candidate node's identity before commit.
    #[inline]
    pub fn candidate_id(&self) -> ElementId {
        self.new.id()
    }
}

/// A side-effect-free structural reconciliation plan.
///
/// Planning walks the old and disconnected candidate trees without adopting
/// runtime state, transferring identities, changing focus, or advancing the
/// installed-tree generation. Keyed siblings match by key regardless of order;
/// unkeyed siblings match only at the same position.
pub struct ReconciliationPlan<'a> {
    old_root: &'a dyn Element,
    new_root: &'a dyn Element,
    matches: Vec<ReconciliationMatch<'a>>,
}

impl<'a> ReconciliationPlan<'a> {
    /// Returns the immutable matches in parent-before-child order.
    #[inline]
    pub fn matches(&self) -> &[ReconciliationMatch<'a>] {
        &self.matches
    }

    /// Revalidates every match before any commit-time mutation occurs.
    pub fn validate(&self) -> Result<(), ReconciliationPlanError> {
        let mut old_nodes = PlanSet::with_capacity_and_hasher(self.matches.len(), PlanBuildHasher::default());
        let mut new_nodes = PlanSet::with_capacity_and_hasher(self.matches.len(), PlanBuildHasher::default());
        for (index, element_match) in self.matches.iter().enumerate() {
            if !identities_are_compatible(element_match.old, element_match.new) {
                return Err(ReconciliationPlanError::IncompatibleMatch { index });
            }
            let old_pointer = object_pointer(element_match.old);
            if !old_nodes.insert(old_pointer) {
                return Err(ReconciliationPlanError::DuplicateOldNode { index });
            }
            let new_pointer = object_pointer(element_match.new);
            if !new_nodes.insert(new_pointer) {
                return Err(ReconciliationPlanError::DuplicateCandidateNode { index });
            }
            match element_match.kind {
                ReconciliationMatchKind::Root if index == 0 => {}
                ReconciliationMatchKind::Root => {
                    return Err(ReconciliationPlanError::UnexpectedRootMatch { index });
                }
                ReconciliationMatchKind::Keyed
                    if element_match.old.reconciliation_key().is_some()
                        && element_match.old.reconciliation_key()
                            == element_match.new.reconciliation_key() => {}
                ReconciliationMatchKind::Positional
                    if element_match.old.reconciliation_key().is_none()
                        && element_match.new.reconciliation_key().is_none() => {}
                _ => return Err(ReconciliationPlanError::InvalidMatchKind { index }),
            }
        }
        Ok(())
    }

    /// Carries runtime state and commits this candidate at the current safe point.
    ///
    /// Validation completes before the first mutation. The established Aimer
    /// state carry then runs once. Because state carry can replace retained
    /// children, identity matches are resolved again against the resulting
    /// candidate before focus cleanup and tree-generation advancement.
    pub fn commit(self, ctx: &BuildContext) -> Result<(), ReconciliationPlanError> {
        self.validate()?;
        let old_root = self.old_root;
        let new_root = self.new_root;
        drop(self);
        let carried_from = scoped_rebuild_cursor();
        crate::components::element::carry_child_state(old_root, new_root, ctx);
        let plan = plan_element_reconciliation(old_root, new_root);
        plan.apply_identities_since(carried_from);
        complete_generated_tree_reconciliation(old_root, new_root);
        plan.refresh_reconciled_paint_state();
        Ok(())
    }

    pub(crate) fn commit_generated_tree(self) -> Result<(), ReconciliationPlanError> {
        self.validate()?;
        self.apply_identities();
        complete_generated_tree_reconciliation(self.old_root, self.new_root);
        self.refresh_reconciled_paint_state();
        Ok(())
    }

    /// Like [`Self::commit_generated_tree`], for a candidate that state was just
    /// carried into. `carried_from` is the log position from before the carry.
    pub(crate) fn commit_carried_tree(
        self,
        carried_from: ScopedCursor,
    ) -> Result<(), ReconciliationPlanError> {
        self.validate()?;
        self.apply_identities_since(carried_from);
        complete_generated_tree_reconciliation(self.old_root, self.new_root);
        self.refresh_reconciled_paint_state();
        Ok(())
    }

    fn refresh_reconciled_paint_state(&self) {
        let generation = crate::components::element::element_tree_generation();
        for element_match in &self.matches {
            let Some(id) = element_match.new.element_id() else {
                continue;
            };
            element_match.new.set_subtree_generation(generation);
            crate::components::element::mark_paint_element_invalidated(id);
        }
    }

    pub(crate) fn apply_identities(&self) {
        for element_match in &self.matches {
            if let Some(old_id) = element_match.old.element_id() {
                element_match.new.set_element_id(old_id);
            }
        }
    }

    /// Transfers identities, and renames the replacements logged since
    /// `carried_from` to match.
    ///
    /// Carrying state into the candidate reconciles the nested pairs it adopts,
    /// and logs each under the id its new root had then. Transferring identities
    /// afterwards changes that id, so those entries are renamed here to the id
    /// the root now has.
    fn apply_identities_since(&self, carried_from: ScopedCursor) {
        if !has_scoped_rebuilds_since(carried_from) {
            self.apply_identities();
            return;
        }
        let mut renamed = HashMap::new();
        for element_match in &self.matches {
            let Some(old_id) = element_match.old.element_id() else {
                continue;
            };
            if let Some(fresh) = element_match.new.element_id()
                && fresh != old_id
            {
                renamed.insert(fresh, old_id);
            }
            element_match.new.set_element_id(old_id);
        }
        rename_scoped_rebuilds_since(carried_from, |id| renamed.get(&id).copied());
    }
}


/// A structural plan invariant changed between planning and commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconciliationPlanError {
    /// A planned pair no longer has compatible type, name, and key identity.
    IncompatibleMatch { index: usize },
    /// One old node was assigned to more than one candidate node.
    DuplicateOldNode { index: usize },
    /// One candidate node was assigned from more than one old node.
    DuplicateCandidateNode { index: usize },
    /// A root match appeared anywhere except the first plan entry.
    UnexpectedRootMatch { index: usize },
    /// A match classification disagrees with the pair's keys.
    InvalidMatchKind { index: usize },
}

impl fmt::Display for ReconciliationPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompatibleMatch { index } => {
                write!(formatter, "reconciliation match {index} became incompatible")
            }
            Self::DuplicateOldNode { index } => {
                write!(formatter, "reconciliation match {index} reuses an old node")
            }
            Self::DuplicateCandidateNode { index } => write!(
                formatter,
                "reconciliation match {index} reuses a candidate node"
            ),
            Self::UnexpectedRootMatch { index } => {
                write!(formatter, "reconciliation match {index} is an unexpected root")
            }
            Self::InvalidMatchKind { index } => {
                write!(formatter, "reconciliation match {index} has an invalid kind")
            }
        }
    }
}

impl Error for ReconciliationPlanError {}

/// Computes compatible old-to-candidate matches without mutating either tree.
pub fn plan_element_reconciliation<'a>(
    old: &'a dyn Element,
    new: &'a dyn Element,
) -> ReconciliationPlan<'a> {
    let mut matches = Vec::new();
    if identities_are_compatible(old, new) {
        let mut visited_pairs = PlanSet::default();
        collect_matches(
            old,
            new,
            ReconciliationMatchKind::Root,
            &mut matches,
            &mut visited_pairs,
        );
    }
    ReconciliationPlan {
        old_root: old,
        new_root: new,
        matches,
    }
}

fn collect_matches<'a>(
    old: &'a dyn Element,
    new: &'a dyn Element,
    kind: ReconciliationMatchKind,
    matches: &mut Vec<ReconciliationMatch<'a>>,
    visited_pairs: &mut PlanSet<(*const (dyn Element + 'a), *const (dyn Element + 'a))>,
) {
    if !visited_pairs.insert((object_pointer(old), object_pointer(new))) {
        return;
    }
    matches.push(ReconciliationMatch { old, new, kind });
    if crate::components::element::same_retained_child(old, new) {
        return;
    }
    let old_children = structural_children(old);
    let new_children = structural_children(new);
    let mut claimed = SmallVec::<[bool; 8]>::from_elem(false, old_children.len());
    // Most parents have no keyed child at all, and a map made for each of them is
    // made for nothing: it is built the first time an old child has a key.
    let mut keyed_candidates: Option<
        HashMap<(&'a crate::key::Key, std::any::TypeId, &'static str), KeyedCandidates, PlanBuildHasher>,
    > = None;

    // Keep old keyed children in source order for each complete identity. The
    // cursor preserves the existing duplicate-key behavior while making the
    // common keyed lookup O(1) instead of scanning every old sibling.
    for (old_index, old_child) in old_children.iter().copied().enumerate() {
        if let Some(old_key) = old_child.reconciliation_key() {
            keyed_candidates
                .get_or_insert_with(HashMap::default)
                .entry((
                    old_key,
                    old_child.element_type_id(),
                    old_child.debug_name(),
                ))
                .or_default()
                .push(old_index);
        }
    }

    for (new_index, new_child) in new_children.iter().copied().enumerate() {
        let selected = if let Some(new_key) = new_child.reconciliation_key() {
            keyed_candidates
                .as_mut()
                .and_then(|candidates| {
                    candidates.get_mut(&(new_key, new_child.element_type_id(), new_child.debug_name()))
                })
                .and_then(|candidates: &mut KeyedCandidates| candidates.take(&claimed))
                .map(|old_index| (old_index, ReconciliationMatchKind::Keyed))
        } else {
            old_children.get(new_index).and_then(|old_child| {
                (!claimed[new_index]
                    && old_child.reconciliation_key().is_none()
                    && identities_are_compatible(*old_child, new_child))
                .then_some((new_index, ReconciliationMatchKind::Positional))
            })
        };

        if let Some((old_index, kind)) = selected {
            claimed[old_index] = true;
            collect_matches(
                old_children[old_index],
                new_child,
                kind,
                matches,
                visited_pairs,
            );
        }
    }
}

#[derive(Default)]
struct KeyedCandidates {
    indices: SmallVec<[usize; 1]>,
    next: usize,
}

impl KeyedCandidates {
    #[inline]
    fn push(&mut self, old_index: usize) {
        self.indices.push(old_index);
    }

    #[inline]
    fn take(&mut self, claimed: &[bool]) -> Option<usize> {
        while let Some(&old_index) = self.indices.get(self.next) {
            self.next += 1;
            if !claimed[old_index] {
                return Some(old_index);
            }
        }
        None
    }
}

/// Identifies an element by its whole object pointer: address and type metadata.
///
/// The address alone is not enough. An element that keeps its only child as its
/// first field — `Expanded` is one — has the same address as that child, so a
/// pair of addresses for the wrapper is also the pair for what is inside it, and
/// the child's own pair was dropped as already visited. Nothing below such an
/// element was matched, so none of it kept its identity across a rebuild. The
/// metadata tells a wrapper from the child that begins where it does.
#[inline]
fn object_pointer<'a>(element: &'a dyn Element) -> *const (dyn Element + 'a) {
    element as *const (dyn Element + 'a)
}

#[cfg(test)]
mod tests {
    use aimer_time::Instant;

    use super::*;
    use crate::base::BuildContext;
    use crate::components::element::AnyElement;
    use crate::{Drawable, EventElement, LayoutElement, Rebuildable, VisitorElement};

    /// An element with any number of children, which is all a plan looks at.
    struct Node {
        children: Vec<AnyElement>,
    }

    impl VisitorElement for Node {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            for child in &self.children {
                visitor(child);
            }
        }

        fn debug_name(&self) -> &'static str {
            "PlanNode"
        }
    }

    impl EventElement for Node {}
    impl LayoutElement for Node {}
    impl Rebuildable for Node {}
    impl Drawable for Node {
        fn update(&self, _ctx: &BuildContext) {}
    }

    /// A balanced tree of `count` nodes, each with up to `fanout` children.
    fn tree(count: usize, fanout: usize) -> AnyElement {
        let mut below = count - 1;
        let mut children = Vec::new();
        for slot in 0..fanout {
            if below == 0 {
                break;
            }
            // Share what is left evenly among the slots that are still to come.
            let share = below.div_ceil(fanout - slot);
            below -= share;
            children.push(tree(share, fanout));
        }
        Node { children }.boxed()
    }

    fn median_micros(mut samples: Vec<u128>) -> u128 {
        samples.sort_unstable();
        samples[samples.len() / 2]
    }

    #[test]
    fn a_plan_pairs_every_node_of_two_trees_of_the_same_shape() {
        let (old, new) = (tree(300, 4), tree(300, 4));

        let plan = plan_element_reconciliation(old.as_ref(), new.as_ref());

        assert_eq!(plan.matches().len(), 300);
        assert!(plan.validate().is_ok());
    }

    /// Time to plan and validate one reconciliation of 2,000 elements.
    ///
    /// Run with `cargo nextest run -p aimer_element -E 'test(profile_planning)'
    /// --run-ignored only --no-capture`.
    #[test]
    #[ignore = "wall-clock measurement"]
    fn profile_planning_a_reconciliation() {
        let (old, new) = (tree(2_000, 4), tree(2_000, 4));
        for _ in 0..20 {
            let plan = plan_element_reconciliation(old.as_ref(), new.as_ref());
            plan.validate().unwrap();
        }
        let samples: Vec<u128> = (0..300)
            .map(|_| {
                let started = Instant::now();
                let plan = plan_element_reconciliation(old.as_ref(), new.as_ref());
                plan.validate().unwrap();
                started.elapsed().as_micros()
            })
            .collect();
        println!("plan + validate, 2000 elements: p50 {} us", median_micros(samples));
    }
}
