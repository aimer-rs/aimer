//! Frame-boundary invalidation records used by the retained element index.

use std::cell::RefCell;
use std::rc::Rc;

use hashbrown::{HashMap, hash_map::Entry};

use aimer_cupid::damage_region::DamageRect;

use crate::components::element::ElementId;

/// The broadest known category of a retained-element mutation.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElementChangeKind {
    /// The existing element's paint output changed.
    Paint,
    /// The element's size or position may have changed.
    Layout,
    /// The element's child set or structural order changed.
    Structure,
    /// A transform, clip, opacity, or paint order changed.
    TransformOrder,
    /// A resource became available or changed.
    Resource,
    /// The source cannot provide a narrower mutation contract.
    Unknown,
}

impl ElementChangeKind {
    fn coalesce(self, next: Self) -> Self {
        if self == next {
            self
        } else {
            Self::Unknown
        }
    }
}

/// Revisions observed when an invalidation is queued or resolved.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ElementInvalidationRevisions {
    /// Structural generation of the retained element tree.
    pub tree: u64,
    /// Rebuild generation that retires retained paint.
    pub rebuild: u64,
    /// Layout-cache invalidation generation.
    pub layout: u64,
    /// Resource generation, when the producer exposes one.
    pub resource: Option<u64>,
}

/// Stable frame-space bounds reported by an element, in its event coordinate
/// system. The compositor converts them to target pixels at its boundary.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementInvalidationBounds {
    /// Inclusive frame-space start point.
    pub start: aimer_attribute::position::Vec2d,
    /// Exclusive frame-space end point.
    pub end: aimer_attribute::position::Vec2d,
}

/// One coalesced mutation awaiting frame-boundary resolution.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct ElementInvalidation {
    /// The canonical retained identity, or `None` for an untracked producer.
    pub element_id: Option<ElementId>,
    /// The most conservative known change category.
    pub change: ElementChangeKind,
    /// The root-relative identity chain supplied by a tracked source.
    pub affected_path: Option<Rc<[ElementId]>>,
    /// Revisions sampled when the first mutation was queued.
    pub queued_revisions: ElementInvalidationRevisions,
    /// Revisions sampled before the retained frame walk.
    pub before_frame_revisions: Option<ElementInvalidationRevisions>,
    /// Revisions sampled after the retained frame walk.
    pub after_frame_revisions: Option<ElementInvalidationRevisions>,
    /// Bounds from the last available tree before the frame walk.
    pub old_bounds: Option<ElementInvalidationBounds>,
    /// Bounds after rebuild and layout for this frame.
    pub new_bounds: Option<ElementInvalidationBounds>,
    /// A conservative pixel-space damage box supplied by a paint owner.
    pub damage_hint: Option<DamageRect>,
    /// The ID could not be resolved in the current generation-safe index.
    pub stale_id: bool,
    /// The ID resolved before drawing but was removed during the frame walk.
    pub removed: bool,
    /// This record must stay on the conservative full-frame path.
    pub requires_full_fallback: bool,
    /// The source queued the mutation while a frame paint collector was active.
    pub queued_during_frame: bool,
}

/// Coalesced invalidations resolved around one retained frame walk.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ElementInvalidationBatch {
    records: Vec<ElementInvalidation>,
}

impl ElementInvalidationBatch {
    /// Returns the records resolved for this frame.
    #[inline]
    pub fn records(&self) -> &[ElementInvalidation] {
        &self.records
    }

    /// Returns whether no element reported a change since the last frame.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Returns whether an unknown, stale, or unbounded change requires full damage.
    #[inline]
    pub fn requires_full_fallback(&self) -> bool {
        self.records
            .iter()
            .any(|record| record.requires_full_fallback)
    }

    pub(crate) fn from_records(records: Vec<ElementInvalidation>) -> Self {
        Self { records }
    }

    pub(crate) fn records_mut(&mut self) -> &mut [ElementInvalidation] {
        &mut self.records
    }
}

thread_local! {
    static PENDING: RefCell<HashMap<Option<ElementId>, ElementInvalidation>> =
        RefCell::new(HashMap::new());
}

pub(crate) fn enqueue(
    element_id: Option<ElementId>,
    path: Option<Rc<[ElementId]>>,
    change: ElementChangeKind,
    revisions: ElementInvalidationRevisions,
    queued_during_frame: bool,
    damage_hint: Option<DamageRect>,
) {
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        match pending.entry(element_id) {
            Entry::Occupied(mut entry) => {
                let record = entry.get_mut();
                record.change = record.change.coalesce(change);
                if path.is_some() {
                    record.affected_path = path;
                }
                record.damage_hint = match (record.damage_hint, damage_hint) {
                    (Some(previous), Some(next)) => Some(union_damage(previous, next)),
                    (None, None) => None,
                    _ => {
                        record.requires_full_fallback = true;
                        None
                    }
                };
                record.queued_during_frame |= queued_during_frame;
                record.requires_full_fallback |= change == ElementChangeKind::Unknown;
                crate::frame_work_stats::record_invalidation_coalesced();
            }
            Entry::Vacant(entry) => {
                entry.insert(ElementInvalidation {
                    element_id,
                    change,
                    affected_path: path,
                    queued_revisions: revisions,
                    before_frame_revisions: None,
                    after_frame_revisions: None,
                    old_bounds: None,
                    new_bounds: None,
                    damage_hint,
                    stale_id: false,
                    removed: false,
                    requires_full_fallback: element_id.is_none()
                        || change == ElementChangeKind::Unknown
                        || (queued_during_frame && damage_hint.is_none()),
                    queued_during_frame,
                });
                crate::frame_work_stats::record_invalidation_queued();
            }
        }
    });
}

fn union_damage(left: DamageRect, right: DamageRect) -> DamageRect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (u64::from(left.x) + u64::from(left.width))
        .max(u64::from(right.x) + u64::from(right.width))
        .min(u64::from(u32::MAX));
    let bottom_edge = (u64::from(left.y) + u64::from(left.height))
        .max(u64::from(right.y) + u64::from(right.height))
        .min(u64::from(u32::MAX));
    DamageRect::new(
        x,
        y,
        right_edge.saturating_sub(u64::from(x)) as u32,
        bottom_edge.saturating_sub(u64::from(y)) as u32,
    )
}

/// The records waiting for a frame, as one window keeps them while another
/// window is the active one.
pub(crate) type PendingRecords = HashMap<Option<ElementId>, ElementInvalidation>;

/// Exchanges the thread's waiting records with `other`.
pub(crate) fn swap_pending(other: &mut PendingRecords) {
    PENDING.with(|pending| std::mem::swap(&mut *pending.borrow_mut(), other));
}

/// Moves `other`'s records onto the thread's, and reports whether any of them
/// cannot be attributed to an element.
///
/// A record for an element the thread already has one for is dropped in favour
/// of the thread's, which is reported as unattributed too: two records for one
/// element are two accounts of what changed, and only a full repaint covers both.
pub(crate) fn absorb_pending(other: &mut PendingRecords) -> bool {
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        let mut unattributed = false;
        for (element, record) in other.drain() {
            match (element, pending.entry(element)) {
                (None, _) => unattributed = true,
                (Some(_), Entry::Occupied(_)) => unattributed = true,
                (Some(_), Entry::Vacant(slot)) => {
                    slot.insert(record);
                }
            }
        }
        unattributed
    })
}

pub(crate) fn take_pending() -> ElementInvalidationBatch {
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        let mut records = Vec::with_capacity(pending.len());
        records.extend(pending.drain().map(|(_, record)| record));
        records.sort_unstable_by_key(|record| record.element_id.map(ElementId::get));
        ElementInvalidationBatch::from_records(records)
    })
}
