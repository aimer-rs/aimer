//! Cached main-axis layout table shared by `RawFlex::computed_size` and
//! `RawFlex::draw`.
//!
//! A flex container used to resolve every child's size and position twice per
//! frame — once while measuring and once while painting — which made both
//! passes `O(children)` even when a viewport exposed a dozen of them. The table
//! built here holds that result for as long as the incoming
//! [`BoxConstraint`] and scale stay the same, so painting a scrolled list only
//! has to look up the index range it actually covers.
//!
//! Two properties make the lookup cheap:
//!
//! - Main-axis starts are accumulated in `f64`. A list of 120 000 rows of 110px
//!   reaches 13.2 million logical pixels, where the `f32` step is already ~2px
//!   and deep-scroll positions visibly drift.
//! - A list whose children all resolved to the same size stores one size and a
//!   single stride instead of a full offset table, which turns the range lookup
//!   into a division and keeps the memory flat.
//!
//! A table whose extents were *predicted* from a single probe carries a third
//! part: a [`Refinement`] holding the exact extent of every child a frame
//! actually painted. Correcting the prediction one child at a time is what lets
//! a list of non-uniform rows stay windowed — the alternative is to measure all
//! of them, which is the cost the prediction exists to avoid.

use std::cell::{Cell, RefCell, UnsafeCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_widget::ElementId;
use aimer_widget::base::BuildContext;

use crate::flex::FlexDirection;
use crate::flex::children_source::ChildrenSource;
use crate::flex::flex_child::distribute_flex_space_in_place;
#[cfg(test)]
use crate::flex::flex_child::distribute_flex_space_in_place_scalar_reference;

/// Marks a child that does not participate in flex distribution.
///
/// Flex weights are non-negative, so a negative slot is unambiguous and lets
/// one vector carry both "is a flex child" and "with which weight" without a
/// second allocation.
const NOT_FLEX: f32 = -1.0;

/// Where a table's extents came from.
///
/// The distinction decides whether the table may be trusted between frames:
/// only [`Origin::Estimated`] describes children that were never looked at, so
/// only it has to be verified against the ones a frame actually paints.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// Every child was measured.
    Measured,
    /// The caller stated one extent for every child.
    Declared,
    /// One child was measured and the rest are assumed to match it.
    Estimated,
}

/// Results produced by the dynamic child-measurement phase of a flex pass.
///
/// `flex_weights` starts as a child-kind table: negative entries identify
/// regular children and non-negative entries identify flex children. The
/// numeric phase replaces the non-negative entries with their allocated
/// shares, after which the flex children are measured with those shares.
struct MeasuredChildren {
    /// Resolved child sizes, with default placeholders for flex children until
    /// their allocated constraints have been applied.
    sizes: Vec<ResolvedSize>,
    /// Flex factors, or their allocated shares after numeric resolution.
    flex_weights: Vec<f32>,
    /// Main-axis extent consumed by regular children.
    sized_main: f32,
    /// Stable identity and generation metadata collected while measuring.
    child_metadata: Vec<(ElementId, u64)>,
    /// Whether every child seen during measurement was layout-stable.
    stable_children: bool,
    /// What each child was when it was measured, or empty when a child was
    /// missing.
    stamps: Vec<ChildStamp>,
}

/// What a child was when a table measured it.
///
/// A later pass compares this against the child to decide whether the size
/// recorded for it can be kept without asking it again.
#[derive(Clone, Copy)]
struct ChildStamp {
    id: ElementId,
    /// The child's subtree generation when it was measured.
    generation: u64,
    /// Whether the child promised its size survives an unrelated rebuild.
    stable: bool,
}

/// Exact extents recorded for the children of a predicted table.
///
/// A prediction assumes every child matches one probe. When a child that was
/// painted turns out not to, its exact extent is recorded here instead of the
/// whole table being thrown away and the list measured: children that were
/// never looked at keep the assumed extent, so the container stays windowed and
/// — because the assumption never changes — the offset of a child already on
/// screen never moves. Only the children *after* a correction shift, which is
/// what has to happen when one of them grew.
///
/// Corrections live in a Fenwick tree, so a prefix sum and an update both cost
/// `O(log n)`. The tree is allocated on the first disagreement, so a uniform
/// list — the shape a long list almost always has — never pays for it.
struct Refinement {
    /// One-indexed Fenwick tree of `measured - assumed` main-axis deltas.
    tree: Vec<f64>,
    /// Exact size of every recorded child.
    sizes: HashMap<usize, ResolvedSize>,
    /// Sum of every recorded delta, which is the correction the total carries.
    total: f64,
    /// Largest cross extent recorded.
    cross_max: f32,
}

impl Refinement {
    /// Creates an empty correction table for `len` children.
    fn new(len: usize) -> Self {
        Self {
            tree: vec![0.0; len + 1],
            sizes: HashMap::new(),
            total: 0.0,
            cross_max: 0.0,
        }
    }

    /// Sum of the corrections recorded for every child before `index`.
    #[inline]
    fn prefix(&self, index: usize) -> f64 {
        let mut sum = 0.0;
        let mut node = index.min(self.tree.len() - 1);
        while node > 0 {
            sum += self.tree[node];
            node &= node - 1;
        }
        sum
    }

    /// Adds `delta` to the correction recorded for child `index`.
    fn add(&mut self, index: usize, delta: f64) {
        self.total += delta;
        let len = self.tree.len();
        let mut node = index + 1;
        while node < len {
            self.tree[node] += delta;
            node += node & node.wrapping_neg();
        }
    }
}

/// Resolved sizes and main-axis offsets of one flex container's children.
///
/// Construct it with [`FlexLayout::build`] and keep it for as long as the
/// constraint it was measured under is still current.
pub(crate) struct FlexLayout {
    /// Main-axis start of every child followed by the total main extent.
    ///
    /// Empty when [`FlexLayout::stride`] is `Some`, in which case offsets are
    /// derived arithmetically.
    offsets: Vec<f64>,
    /// Resolved size per child, or a single shared size for a uniform list.
    sizes: Vec<ResolvedSize>,
    /// Distance between the starts of two adjacent children — the child extent
    /// plus the gap — when every child resolved to the same size.
    stride: Option<f64>,
    /// Number of children the table describes.
    len: usize,
    /// Stable identity and last installed-tree generation of each direct child,
    /// when every child opted into generation-independent sizing.
    child_metadata: Vec<(ElementId, u64)>,
    /// How each child looked when it was measured, for revalidating a measured
    /// table child by child. Updated in place as children are asked again.
    stamps: RefCell<Vec<ChildStamp>>,
    /// Indices of the children that did not opt into generation-independent
    /// sizing, which can change size without being replaced.
    unstable: Vec<usize>,
    /// Where the log of stable-generation changes stood when the stamps were
    /// last known to be current.
    log_cursor: Cell<u64>,
    /// Position of each stable child by identity, built when a revalidation
    /// first has a changed identity to place.
    stable_positions: RefCell<Option<HashMap<ElementId, usize>>>,
    /// Whether every direct child opted into generation-independent sizing.
    stable_children: bool,
    /// Size of the container itself, gaps included.
    total: ResolvedSize,
    /// Whether any child was sized by flex distribution.
    has_flex: bool,
    /// How the extents in this table were obtained.
    origin: Origin,
    /// Whether the main axis is the horizontal one, which is what decides
    /// which component of a [`ResolvedSize`] a correction applies to.
    is_row: bool,
    /// Exact extents recorded for the painted children of a predicted table.
    ///
    /// `None` until a child disagrees with the prediction, which is the case a
    /// uniform list never reaches.
    refinement: RefCell<Option<Box<Refinement>>>,
}

impl FlexLayout {
    /// Measures `children` along `direction` and records their positions.
    ///
    /// Non-flex children keep the intrinsic size they report under an unbounded
    /// main axis; flex children are measured with their share of the leftover
    /// space, exactly as [`distribute_flex_space_in_place`] hands it out. `gap_main` is
    /// the resolved spacing inserted between two adjacent children.
    ///
    /// Measuring requires every child to exist, so a child the source has not
    /// materialized contributes nothing. Callers holding a windowed source must
    /// materialize the whole range first; see
    /// [`RawFlex::measure_layout`](crate::flex::raw_flex::RawFlex).
    pub(crate) fn build(
        direction: FlexDirection,
        children: &dyn ChildrenSource,
        ctx: &BuildContext,
        gap_main: f32,
    ) -> Self {
        let len = children.len();
        // Taken before any child is looked at, so a change made while measuring
        // is seen by the next revalidation rather than lost.
        let log_cursor = aimer_widget::stable_generation_cursor();
        let is_row = !matches!(direction, FlexDirection::Column);
        let (max_main, max_cross) = if is_row {
            (ctx.box_constraint.max_width, ctx.box_constraint.max_height)
        } else {
            (ctx.box_constraint.max_height, ctx.box_constraint.max_width)
        };
        let total_gap = if len > 1 {
            gap_main * (len - 1) as f32
        } else {
            0.0
        };

        let mut child_ctx = ctx.clone();
        let mut measured = Self::measure_children(is_row, max_cross, children, &mut child_ctx);
        let has_flex = !measured.flex_weights.is_empty();

        // This phase only operates on contiguous scalar data. It deliberately
        // does not borrow or inspect a child, so a future numeric kernel can
        // replace it without crossing the dynamic element boundary.
        Self::resolve_flex_space(
            max_main,
            measured.sized_main,
            total_gap,
            &mut measured.flex_weights,
        );
        Self::measure_flex_children(
            is_row,
            max_main,
            max_cross,
            children,
            &mut child_ctx,
            &mut measured,
        );

        let mut layout = Self::from_sizes(measured.sizes, is_row, gap_main, has_flex);
        layout.child_metadata = measured.child_metadata;
        layout.unstable = measured
            .stamps
            .iter()
            .enumerate()
            .filter(|(_, stamp)| !stamp.stable)
            .map(|(index, _)| index)
            .collect();
        layout.log_cursor = Cell::new(log_cursor);
        layout.stamps = RefCell::new(measured.stamps);
        layout.stable_children = measured.stable_children && !layout.has_flex;
        layout
    }

    /// Measures regular children and records flex factors without doing any
    /// numeric flex-space resolution.
    ///
    /// This is the dynamic half of [`FlexLayout::build`]. Calls into
    /// `Element::computed_size` stay here, where trait dispatch and child
    /// ownership are explicit and cannot leak into the numeric phase.
    fn measure_children(
        is_row: bool,
        max_cross: f32,
        children: &dyn ChildrenSource,
        child_ctx: &mut BuildContext,
    ) -> MeasuredChildren {
        let len = children.len();
        let mut sizes: Vec<ResolvedSize> = Vec::with_capacity(len);
        let mut child_metadata = Vec::new();
        let mut stamps = Vec::with_capacity(len);
        let mut stamps_complete = true;
        let mut stable_children = true;
        // Allocated on first sight of a flex child; a plain list never pays for
        // it.
        let mut flex_weights: Vec<f32> = Vec::new();
        let mut sized_main: f32 = 0.0;

        for index in 0..len {
            let Some(child) = children.get(index) else {
                stable_children = false;
                stamps_complete = false;
                child_metadata.clear();
                sizes.push(ResolvedSize::default());
                continue;
            };
            let stable = child.is_layout_stable();
            if stamps_complete {
                stamps.push(ChildStamp {
                    id: child.id(),
                    generation: child.subtree_generation(),
                    stable,
                });
            }
            if stable_children && stable {
                child_metadata.push((child.id(), child.subtree_generation()));
            } else if stable_children {
                stable_children = false;
                child_metadata.clear();
            }
            if let Some(flex) = child.flex() {
                if flex_weights.is_empty() {
                    flex_weights = vec![NOT_FLEX; len];
                }
                flex_weights[index] = flex.max(0.0);
                sizes.push(ResolvedSize::default());
                continue;
            }

            set_main(&mut child_ctx.box_constraint, is_row, f32::MAX);
            set_cross(&mut child_ctx.box_constraint, is_row, max_cross);
            let size = child.computed_size(child_ctx);
            sized_main += main_of(size, is_row);
            sizes.push(size);
        }

        MeasuredChildren {
            sizes,
            flex_weights,
            sized_main,
            child_metadata,
            stable_children,
            stamps: if stamps_complete { stamps } else { Vec::new() },
        }
    }

    /// Resolves flex factors into main-axis shares using only scalar data.
    #[inline]
    fn resolve_flex_space(
        max_main: f32,
        sized_main: f32,
        total_gap: f32,
        flex_weights: &mut [f32],
    ) {
        if flex_weights.is_empty() {
            return;
        }

        if max_main == f32::MAX {
            for weight in flex_weights {
                if *weight > 0.0 {
                    *weight = f32::MAX;
                }
            }
        } else {
            distribute_flex_space_in_place(
                (max_main - sized_main - total_gap).max(0.0),
                flex_weights,
            );
        }
    }

    /// Measures flex children after their scalar shares have been resolved.
    ///
    /// The second set of `computed_size` calls remains dynamic by design. Only
    /// the share calculation above is a candidate for a numeric kernel.
    fn measure_flex_children(
        is_row: bool,
        max_main: f32,
        max_cross: f32,
        children: &dyn ChildrenSource,
        child_ctx: &mut BuildContext,
        measured: &mut MeasuredChildren,
    ) {
        if measured.flex_weights.is_empty() {
            return;
        }

        for index in 0..measured.sizes.len() {
            let encoded_weight = measured.flex_weights[index];
            if encoded_weight < 0.0 {
                continue;
            }
            let Some(child) = children.get(index) else {
                continue;
            };
            // Negative zero marks a zero-weight flex child. It still gets
            // a zero constraint, but must not overwrite its intrinsic
            // measured size with `set_main_of`.
            let share = if max_main == f32::MAX && encoded_weight == 0.0 {
                f32::MAX
            } else {
                encoded_weight
            };
            set_main(&mut child_ctx.box_constraint, is_row, share);
            set_cross(&mut child_ctx.box_constraint, is_row, max_cross);
            let mut size = child.computed_size(child_ctx);
            if !encoded_weight.is_sign_negative() && share != f32::MAX {
                set_main_of(&mut size, is_row, share);
            }
            measured.sizes[index] = size;
        }
    }

    /// Builds the table for a list whose children all occupy `main` along the
    /// main axis, without measuring a single child.
    ///
    /// This is the constructor behind
    /// [`FlexList::item_extent`](crate::FlexList::item_extent). Because the
    /// extent is declared, the total is pure arithmetic — `stride * len - gap` —
    /// so a container reports its scroll extent in `O(1)` and only the children
    /// a viewport exposes are ever touched. `cross` is the extent handed to every
    /// child across the axis, which is the container's own cross-axis maximum.
    pub(crate) fn declared(len: usize, main: f32, cross: f32, is_row: bool, gap_main: f32) -> Self {
        Self::uniform(
            len,
            sized(main, cross, is_row),
            is_row,
            gap_main,
            Origin::Declared,
        )
    }

    /// Builds the table for a list whose children are *assumed* to occupy the
    /// size a single probed child reported.
    ///
    /// This is the prediction used when no extent was declared. It costs one
    /// measure instead of `len`, which is what removes the cold-start pass over
    /// a long list, and it is exact whenever the list really is uniform — the
    /// overwhelmingly common shape of a scrolled list.
    ///
    /// Unlike [`FlexLayout::declared`] the result is *not* authoritative: it has
    /// to be verified against the children a frame paints, and the container has
    /// to fall back to measuring when they disagree. See
    /// [`RawFlex::estimated_layout`](crate::flex::raw_flex::RawFlex).
    pub(crate) fn estimated(len: usize, probe: ResolvedSize, is_row: bool, gap_main: f32) -> Self {
        Self::uniform(len, probe, is_row, gap_main, Origin::Estimated)
    }

    /// Builds a table in which every child shares `size`.
    ///
    /// One stored size and one stride replace the offset table entirely, so the
    /// memory is flat and [`FlexLayout::visible_range`] is a division.
    fn uniform(
        len: usize,
        size: ResolvedSize,
        is_row: bool,
        gap_main: f32,
        origin: Origin,
    ) -> Self {
        if len == 0 {
            return Self::from_sizes(Vec::new(), is_row, gap_main, false);
        }

        let cross = cross_of(size, is_row);
        let stride = main_of(size, is_row) as f64 + gap_main as f64;
        let main_total = stride * len as f64 - gap_main as f64;

        Self {
            offsets: Vec::new(),
            sizes: vec![size],
            stride: Some(stride),
            len,
            child_metadata: Vec::new(),
            stamps: RefCell::new(Vec::new()),
            unstable: Vec::new(),
            log_cursor: Cell::new(0),
            stable_positions: RefCell::new(None),
            stable_children: false,
            total: sized(saturate_f32(main_total), cross, is_row),
            has_flex: false,
            origin,
            is_row,
            refinement: RefCell::new(None),
        }
    }

    /// Turns per-child sizes into offsets, a total, and — when possible — a
    /// stride.
    fn from_sizes(
        mut sizes: Vec<ResolvedSize>,
        is_row: bool,
        gap_main: f32,
        has_flex: bool,
    ) -> Self {
        let len = sizes.len();
        let mut cross_max: f32 = 0.0;
        let mut uniform = len > 0;
        for size in &sizes {
            cross_max = cross_max.max(cross_of(*size, is_row));
            uniform &= *size == sizes[0];
        }

        if uniform {
            let extent = main_of(sizes[0], is_row) as f64;
            let stride = extent + gap_main as f64;
            let main_total = stride * len as f64 - gap_main as f64;
            sizes.truncate(1);
            sizes.shrink_to_fit();
            return Self {
                offsets: Vec::new(),
                sizes,
                stride: Some(stride),
                len,
                child_metadata: Vec::new(),
                stamps: RefCell::new(Vec::new()),
                unstable: Vec::new(),
                log_cursor: Cell::new(0),
                stable_positions: RefCell::new(None),
                stable_children: false,
                total: sized(saturate_f32(main_total), cross_max, is_row),
                has_flex,
                origin: Origin::Measured,
                is_row,
                refinement: RefCell::new(None),
            };
        }

        let mut offsets = Vec::with_capacity(len + 1);
        let mut main_total: f64 = 0.0;
        for (index, size) in sizes.iter().enumerate() {
            offsets.push(main_total);
            main_total += main_of(*size, is_row) as f64;
            if index + 1 < len {
                main_total += gap_main as f64;
            }
        }
        offsets.push(main_total);

        Self {
            offsets,
            sizes,
            stride: None,
            len,
            child_metadata: Vec::new(),
            stamps: RefCell::new(Vec::new()),
            unstable: Vec::new(),
            log_cursor: Cell::new(0),
            stable_positions: RefCell::new(None),
            stable_children: false,
            total: sized(saturate_f32(main_total), cross_max, is_row),
            has_flex,
            origin: Origin::Measured,
            is_row,
            refinement: RefCell::new(None),
        }
    }

    /// Whether a child was sized by flex distribution.
    ///
    /// Such a table cannot be revalidated one child at a time, because every
    /// share depends on what the others consumed.
    #[inline]
    pub(crate) fn has_flex(&self) -> bool {
        self.has_flex
    }

    /// Whether every direct child reported a size that can survive an
    /// unrelated element-tree generation change.
    #[inline]
    pub(crate) fn can_reuse_stable_children(&self) -> bool {
        self.stable_children && !self.has_flex && self.child_metadata.len() == self.len
    }

    /// Brings a measured table across a change of the element-tree generation
    /// by asking only the children that cannot vouch for their own size.
    ///
    /// A child that is layout-stable, is the element the table measured, and
    /// whose subtree was not replaced keeps the size recorded for it. Every
    /// other child is measured again under the constraint [`FlexLayout::build`]
    /// gave it. The table still describes the children exactly when each of
    /// those answers agrees with what is recorded, and then nothing is rebuilt.
    ///
    /// Returns `false` when the table cannot be vouched for, in which case the
    /// caller measures everything. A table with flex children is never
    /// revalidated, because every share depends on what the others consumed.
    pub(crate) fn revalidate_measured(
        &self,
        children: &dyn ChildrenSource,
        ctx: &BuildContext,
    ) -> bool {
        let mut stamps = self.stamps.borrow_mut();
        if self.has_flex || self.origin != Origin::Measured || stamps.len() != self.len {
            return false;
        }
        if children.len() != self.len {
            return false;
        }
        let cursor = aimer_widget::stable_generation_cursor();

        // The log says which stable children had their generation set since the
        // stamps were current. Every other stable child is as it was, so only
        // those and the children that cannot vouch for themselves are asked.
        let mut changed: Vec<usize> = Vec::new();
        let mut positions = self.stable_positions.borrow_mut();
        let positions = positions.get_or_insert_with(|| {
            stamps
                .iter()
                .enumerate()
                .filter(|(_, stamp)| stamp.stable)
                .map(|(index, stamp)| (stamp.id, index))
                .collect()
        });
        let logged = aimer_widget::stable_generations_since(self.log_cursor.get(), &mut |id| {
            if let Some(&index) = positions.get(&id) {
                changed.push(index);
            }
        });
        let indices: Vec<usize> = if logged {
            changed.extend_from_slice(&self.unstable);
            changed.sort_unstable();
            changed.dedup();
            changed
        } else {
            (0..self.len).collect()
        };

        let is_row = self.is_row;
        let max_cross = if is_row {
            ctx.box_constraint.max_height
        } else {
            ctx.box_constraint.max_width
        };
        // Cloned on the first child that has to be asked, so a table whose
        // children all vouch for themselves clones nothing.
        let mut child_ctx: Option<BuildContext> = None;

        for index in indices {
            let Some(child) = children.get(index) else {
                return false;
            };
            let stamp = &mut stamps[index];
            if child.id() != stamp.id {
                return false;
            }
            let stable = child.is_layout_stable();
            let generation = child.subtree_generation();
            if stamp.stable && stable && stamp.generation == generation {
                continue;
            }

            let child_ctx = child_ctx.get_or_insert_with(|| ctx.clone());
            set_main(&mut child_ctx.box_constraint, is_row, f32::MAX);
            set_cross(&mut child_ctx.box_constraint, is_row, max_cross);
            if child.computed_size(child_ctx) != self.size(index) {
                return false;
            }
            stamp.stable = stable;
            stamp.generation = generation;
        }
        self.log_cursor.set(cursor);
        true
    }

    /// Returns whether the stable direct-child metadata still describes
    /// `children` after a generation change.
    pub(crate) fn stable_children_match(&self, children: &dyn ChildrenSource) -> bool {
        if !self.can_reuse_stable_children() || children.len() != self.len {
            return false;
        }

        for index in 0..self.len {
            let Some(child) = children.get(index) else {
                return false;
            };
            let (id, generation) = self.child_metadata[index];
            if child.id() != id || child.subtree_generation() != generation {
                return false;
            }
        }
        true
    }

    /// Whether the table was derived from a declared item extent.
    ///
    /// Such a table is authoritative: the caller stated the extent, so it is
    /// never revalidated against the children — which is exactly what keeps a
    /// long list off the measuring path.
    #[inline]
    pub(crate) fn is_declared(&self) -> bool {
        self.origin == Origin::Declared
    }

    /// Whether the table was extrapolated from a single probed child.
    ///
    /// Such a table is a prediction: it holds only for as long as the children a
    /// frame paints agree with it.
    #[inline]
    pub(crate) fn is_estimated(&self) -> bool {
        self.origin == Origin::Estimated
    }

    /// Whether the table states what the children measured, and so stops
    /// describing them once the element tree below the container changes.
    ///
    /// A [`Origin::Declared`] extent comes from the caller and holds whatever
    /// the children turn out to be. A [`Origin::Estimated`] one is a prediction
    /// that every painted frame re-verifies through
    /// [`refine`](FlexLayout::refine), so it repairs itself and must *not* be
    /// thrown away — the corrections it accumulated are the only record of the
    /// rows a scrolled list has already resolved. Only a measured table has to
    /// be taken at its word, which is exactly why it has to expire.
    #[inline]
    pub(crate) fn describes_measured_children(&self) -> bool {
        self.origin == Origin::Measured
    }

    /// Number of children described by this table.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Size of the flex container, gaps included.
    ///
    /// A predicted table adds the corrections recorded so far, so the size grows
    /// or shrinks as painted children turn out to disagree with the prediction.
    #[inline]
    pub(crate) fn total(&self) -> ResolvedSize {
        let slot = self.refinement.borrow();
        let Some(refinement) = slot.as_ref() else {
            return self.total;
        };

        let mut total = self.total;
        let main = main_of(total, self.is_row) as f64 + refinement.total;
        set_main_of(&mut total, self.is_row, saturate_f32(main));
        if refinement.cross_max > cross_of(total, self.is_row) {
            set_cross_of(&mut total, self.is_row, refinement.cross_max);
        }
        total
    }

    /// Resolved size of child `index`.
    ///
    /// A child whose exact size was recorded through [`FlexLayout::refine`]
    /// reports that size; every other child of a predicted table reports the
    /// probe.
    ///
    /// # Panics
    ///
    /// Panics when `index` is out of bounds.
    #[inline]
    pub(crate) fn size(&self, index: usize) -> ResolvedSize {
        if let Some(refinement) = self.refinement.borrow().as_ref()
            && let Some(size) = refinement.sizes.get(&index)
        {
            return *size;
        }
        if self.stride.is_some() {
            self.sizes[0]
        } else {
            self.sizes[index]
        }
    }

    /// [`Self::offset`] as the `f32` layout uses, saturating at `f32::MAX`.
    #[inline]
    pub(crate) fn offset_f32(&self, index: usize) -> f32 {
        saturate_f32(self.offset(index))
    }

    /// Main-axis start of child `index`, relative to the container's content
    /// box.
    #[inline]
    pub(crate) fn offset(&self, index: usize) -> f64 {
        match self.stride {
            Some(stride) => {
                let base = stride * index as f64;
                match self.refinement.borrow().as_ref() {
                    Some(refinement) => base + refinement.prefix(index),
                    None => base,
                }
            }
            None => self.offsets[index],
        }
    }

    /// Records the exact size `size` of child `index`.
    ///
    /// This is how a prediction is corrected without giving up on windowing:
    /// the children that were never looked at keep the probed extent, so the
    /// offsets of `index` and everything before it are untouched and nothing
    /// already on screen moves. The container's total shifts by the difference,
    /// which is what a scroll view sees as its extent converging on the truth.
    ///
    /// Returns `false`, changing nothing, for a table that is not a prediction:
    /// a declared extent is authoritative and a measured one is exact already.
    pub(crate) fn refine(&self, index: usize, size: ResolvedSize) -> bool {
        if self.origin != Origin::Estimated || index >= self.len {
            return false;
        }

        let assumed = main_of(self.sizes[0], self.is_row) as f64;
        let mut slot = self.refinement.borrow_mut();
        let refinement = slot.get_or_insert_with(|| Box::new(Refinement::new(self.len)));
        let previous = refinement
            .sizes
            .insert(index, size)
            .map_or(assumed, |old| main_of(old, self.is_row) as f64);
        let delta = main_of(size, self.is_row) as f64 - previous;
        if delta != 0.0 {
            refinement.add(index, delta);
        }
        refinement.cross_max = refinement.cross_max.max(cross_of(size, self.is_row));
        true
    }

    /// Takes over what `previous` learned about the extent of its children.
    ///
    /// A prediction made anew would otherwise start from its probed row alone,
    /// and a scroll view reading its total sees the extent it had learned
    /// collapse back to the guess. What a prediction corrected, and what a
    /// measured table holds for every child, is recorded against *this*
    /// prediction's assumed size, so the total stays consistent, and a frame
    /// that finds one of them wrong corrects it as usual.
    ///
    /// Does nothing unless this table is a prediction and `previous` describes
    /// the same number of children along the same axis. A table with flex
    /// children is not inherited from, because each share depends on what the
    /// others consumed.
    pub(crate) fn inherit_learned(&self, previous: &FlexLayout) {
        if self.origin != Origin::Estimated
            || previous.has_flex
            || self.len != previous.len
            || self.is_row != previous.is_row
        {
            return;
        }
        match previous.origin {
            Origin::Estimated => {
                let corrected: Vec<(usize, ResolvedSize)> =
                    match previous.refinement.borrow().as_ref() {
                        Some(refinement) => refinement
                            .sizes
                            .iter()
                            .map(|(&index, &size)| (index, size))
                            .collect(),
                        None => return,
                    };
                for (index, size) in corrected {
                    self.refine(index, size);
                }
            }
            Origin::Measured => {
                for index in 0..previous.len {
                    self.refine(index, previous.size(index));
                }
            }
            Origin::Declared => {}
        }
    }

    /// Index of the first child whose main-axis start no longer satisfies
    /// `keep`.
    ///
    /// Offsets are non-decreasing, so this is a plain binary search. It costs
    /// `O(log n)` lookups, each of which is itself `O(log n)` once corrections
    /// were recorded.
    fn partition(&self, keep: impl Fn(f64) -> bool) -> usize {
        let (mut low, mut high) = (0usize, self.len);
        while low < high {
            let middle = low + (high - low) / 2;
            if keep(self.offset(middle)) {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low
    }

    /// Returns the half-open index range whose children intersect the main-axis
    /// span `[start, end)`.
    ///
    /// Resolution is `O(1)` for a uniform list and `O(log n)` otherwise. The
    /// range may include one child on each side that only touches the span.
    pub(crate) fn visible_range(&self, start: f64, end: f64) -> Range<usize> {
        if self.len == 0 || end < start {
            return 0..0;
        }

        if let Some(stride) = self.stride {
            if self.refinement.borrow().is_some() {
                // Corrections broke the uniform spacing, so the range has to be
                // searched for. The bounds match the arithmetic below: a child
                // starting exactly on the far edge of the span still counts.
                let first = self.partition(|offset| offset <= start).saturating_sub(1);
                let last = self.partition(|offset| offset <= end);
                return first.min(self.len)..last.min(self.len);
            }
            if stride <= 0.0 {
                // Zero-extent children all sit on the same offset, so no span
                // can narrow the list.
                return 0..self.len;
            }
            let first = (start / stride).floor().max(0.0) as usize;
            if first >= self.len {
                return self.len..self.len;
            }
            let last = ((end / stride).floor() as usize).saturating_add(1);
            return first..last.min(self.len);
        }

        // `offsets` is non-decreasing, so the last child starting at or before
        // `start` is the one covering it.
        let first = self
            .offsets
            .partition_point(|offset| *offset <= start)
            .saturating_sub(1);
        let last = self.offsets.partition_point(|offset| *offset < end);
        first.min(self.len)..last.min(self.len)
    }

    /// Returns the visible range when a constant amount of extra main-axis
    /// space is inserted before every child after the first one.
    ///
    /// This is used by semantic justification. The regular table remains
    /// unchanged because its cached offsets still describe measured layout;
    /// only the visibility search needs to account for the distributed space.
    pub(crate) fn visible_range_with_extra_space(
        &self,
        start: f64,
        end: f64,
        leading: f64,
        between: f64,
    ) -> Range<usize> {
        if self.len == 0 || end < start {
            return 0..0;
        }

        let offset = |index: usize| self.offset(index) + leading + between * index as f64;
        let partition = |keep: &dyn Fn(f64) -> bool| {
            let (mut low, mut high) = (0usize, self.len);
            while low < high {
                let middle = low + (high - low) / 2;
                if keep(offset(middle)) {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            low
        };
        let first = partition(&|value| value <= start).saturating_sub(1);
        let last = partition(&|value| value < end);
        first.min(self.len)..last.min(self.len)
    }
}

/// How many constraints a container remembers an answer for.
///
/// A container in a scroll view is asked under an unbounded extent and under its
/// own, at its own width and at an unbounded one: four in all.
const CACHED_CONSTRAINTS: usize = 4;

/// A [`FlexLayout`] together with everything that decides whether it still
/// describes the container.
struct CachedTable {
    constraint: BoxConstraint,
    scale_bits: u32,
    generation: u64,
    layout_generation: u64,
    layout: Rc<FlexLayout>,
}

/// The order in which a painted child range should be visited.
#[derive(Clone)]
pub(crate) enum LayerOrder {
    /// Every child in the range is on the default layer, so the range can be
    /// visited directly without allocating an order table.
    Unlayered,
    /// Children sorted from the lowest layer to the highest layer.
    Sorted(Rc<[(u32, usize)]>),
}

impl LayerOrder {
    /// Visits the indices in paint order, avoiding an order allocation for an
    /// unlayered range.
    #[inline]
    pub(crate) fn visit(&self, range: Range<usize>, mut visitor: impl FnMut(usize)) {
        match self {
            Self::Unlayered => {
                for index in range {
                    visitor(index);
                }
            }
            Self::Sorted(order) => {
                for &(_, index) in order.iter() {
                    visitor(index);
                }
            }
        }
    }
}

struct CachedLayerOrder {
    generation: u64,
    range: (usize, usize),
    order: LayerOrder,
}

const HIT_TEST_CHILD_THRESHOLD: usize = 64;
const HIT_TEST_BIN_COUNT: usize = 64;

/// A coarse main-axis interval index for a wide flex child range.
///
/// The exact child bounds are still checked by the routed walk. This index only
/// rejects children whose retained bounds cannot intersect the pointer on the
/// flex main axis, so overflow and cross-axis alignment retain their existing
/// semantics. Unknown bounds are kept in every bin, just as the fallback walk
/// treats them as candidates.
struct CachedHitTestIndex {
    generation: u64,
    layout_generation: u64,
    range: (usize, usize),
    is_row: bool,
    min_main: f32,
    max_main: f32,
    bin_height: f32,
    offsets: Rc<[usize]>,
    children: Rc<[usize]>,
    outside: Rc<[usize]>,
}

impl CachedHitTestIndex {
    #[inline]
    fn candidates(&self, range: &Range<usize>, pos: Vec2d) -> Option<&[usize]> {
        if self.generation != aimer_widget::element_tree_generation()
            || self.layout_generation != aimer_widget::layout_invalidation_generation()
            || self.range != (range.start, range.end)
            || !pos.x.is_finite()
            || !pos.y.is_finite()
        {
            return None;
        }

        let main = if self.is_row { pos.x } else { pos.y };
        let candidates = if main < self.min_main || main > self.max_main {
            self.outside.as_ref()
        } else {
            let bin = ((main - self.min_main) / self.bin_height)
                .floor()
                .clamp(0.0, (HIT_TEST_BIN_COUNT - 1) as f32) as usize;
            let start = self.offsets[bin];
            let end = self.offsets[bin + 1];
            &self.children[start..end]
        };

        Some(candidates)
    }
}

/// Holds one flex container's [`FlexLayout`] between frames.
///
/// The table is keyed by the constraint and scale it was measured under, so a
/// scroll — which only changes the visible rectangle — reuses it instead of
/// re-measuring the child list. The last painted index range is kept alongside
/// it so hit testing can skip the children that were never painted.
///
/// A table that measured its children also carries the [element-tree
/// generation](aimer_widget::element_tree_generation) it was built in, for the
/// same reason [`LayoutCache`](aimer_widget::LayoutCache) does: it states what
/// the children were, so replacing a generated subtree below this container — a
/// `setState`, or an `AsyncBuilder` swapping its loading state for the data it
/// waited on — retires it. A container that itself never rebuilt would
/// otherwise keep reporting the extent its content had before the swap, and a
/// scroll view above it would never learn that there is something to scroll.
/// The table also carries the layout-invalidation generation, so an explicit
/// resize/layout invalidation retires it without walking the child list.
///
/// A declared or predicted table is kept across generations, see
/// [`FlexLayout::describes_measured_children`].
pub(crate) struct FlexLayoutCache {
    /// The tables made for the constraints this container was asked under, the
    /// one made last first.
    ///
    /// One container is asked under several constraints in a frame: the pass that
    /// sizes the scroll view above it gives it an unbounded extent, the pass that
    /// places and paints it gives it the extent it measured, and a wrapping parent
    /// asks again at each line width. A single table made each pass throw the
    /// previous one's answer away, so every pass measured every child again.
    tables: UnsafeCell<[Option<CachedTable>; CACHED_CONSTRAINTS]>,
    painted: Cell<Option<(usize, usize)>>,
    paint_order: UnsafeCell<Option<CachedLayerOrder>>,
    hit_test_index: UnsafeCell<Option<CachedHitTestIndex>>,
}

impl FlexLayoutCache {
    /// Creates an empty cache.
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            tables: UnsafeCell::new([const { None }; CACHED_CONSTRAINTS]),
            painted: Cell::new(None),
            paint_order: UnsafeCell::new(None),
            hit_test_index: UnsafeCell::new(None),
        }
    }

    /// The held tables, the one made last first.
    #[inline]
    fn tables(&self) -> impl Iterator<Item = &CachedTable> {
        // Only ever read through these shared references, and never while `set`
        // holds an exclusive one; the element tree is single-threaded.
        unsafe { &*self.tables.get() }.iter().flatten()
    }

    /// Returns the cached table when it was measured under the same inputs and
    /// the element tree still holds the children it was measured from.
    #[inline]
    pub(crate) fn get(&self, constraint: BoxConstraint, scale_bits: u32) -> Option<Rc<FlexLayout>> {
        self.tables().find_map(|cached| {
            if cached.constraint != constraint || cached.scale_bits != scale_bits {
                return None;
            }
            if cached.layout_generation != aimer_widget::layout_invalidation_generation() {
                return None;
            }
            if cached.generation != aimer_widget::element_tree_generation()
                && cached.layout.describes_measured_children()
            {
                return None;
            }
            Some(Rc::clone(&cached.layout))
        })
    }

    /// Returns the table held now when it is a prediction or a measure,
    /// whatever constraint it was made under.
    ///
    /// A table is keyed by the constraint of the pass that made it, but what it
    /// learned — the exact extent of every child a frame corrected, or of every
    /// child a bounded pass measured — does not belong to that constraint
    /// alone. This is how a replacement prediction finds what to inherit when
    /// the constraint it is asked under differs.
    #[inline]
    pub(crate) fn learned(&self) -> Option<Rc<FlexLayout>> {
        let cached = self.tables().next()?;
        (!cached.layout.is_declared()).then(|| Rc::clone(&cached.layout))
    }

    /// Returns a measured table whose generation is stale but whose children
    /// may still be generation-independent.
    #[inline]
    pub(crate) fn get_stale_stable(
        &self,
        constraint: BoxConstraint,
        scale_bits: u32,
    ) -> Option<Rc<FlexLayout>> {
        self.tables().find_map(|cached| {
            if cached.constraint != constraint
                || cached.scale_bits != scale_bits
                || cached.layout_generation != aimer_widget::layout_invalidation_generation()
                || cached.generation == aimer_widget::element_tree_generation()
                || !cached.layout.describes_measured_children()
                || !cached.layout.can_reuse_stable_children()
            {
                return None;
            }
            Some(Rc::clone(&cached.layout))
        })
    }

    /// Returns a measured table built under these inputs whose only stale part
    /// is the element-tree generation, for revalidation child by child.
    #[inline]
    pub(crate) fn get_stale_measured(
        &self,
        constraint: BoxConstraint,
        scale_bits: u32,
    ) -> Option<Rc<FlexLayout>> {
        self.tables().find_map(|cached| {
            if cached.constraint != constraint
                || cached.scale_bits != scale_bits
                || cached.layout_generation != aimer_widget::layout_invalidation_generation()
                || cached.generation == aimer_widget::element_tree_generation()
                || !cached.layout.describes_measured_children()
            {
                return None;
            }
            Some(Rc::clone(&cached.layout))
        })
    }

    /// Stores `layout` as the table for `constraint` and `scale_bits`.
    #[inline]
    pub(crate) fn set(&self, constraint: BoxConstraint, scale_bits: u32, layout: Rc<FlexLayout>) {
        let table = CachedTable {
            constraint,
            scale_bits,
            generation: aimer_widget::element_tree_generation(),
            layout_generation: aimer_widget::layout_invalidation_generation(),
            layout,
        };
        // Exclusive reference, dropped before `get` can run again.
        let held = unsafe { &mut *self.tables.get() };
        let same_key = held.iter().position(|held| {
            held.as_ref()
                .is_some_and(|held| held.constraint == constraint && held.scale_bits == scale_bits)
        });
        match same_key {
            Some(index) => held[index] = Some(table),
            None => {
                // The oldest answer makes room for this one.
                held.rotate_right(1);
                held[0] = Some(table);
            }
        }
    }

    /// Drops the predictions held for another constraint on the main axis that
    /// the table made under `kept` has outgrown, and reports whether any was.
    ///
    /// A container is measured by its parent under an unbounded main axis, which
    /// is predicted from one probe, and then laid out in a box as long as that
    /// prediction, under which every child is measured and the answer is exact.
    /// The two tables are keyed by their constraints, so the prediction would go
    /// on answering the parent with what it guessed. Only a prediction at the
    /// same cross-axis extent is dropped — another width can legitimately
    /// differ — and a replacement inherits what `kept` knows, since `kept` is
    /// the table made last.
    pub(crate) fn retire_outdated_predictions(
        &self,
        kept: BoxConstraint,
        scale_bits: u32,
        is_row: bool,
        total: ResolvedSize,
    ) -> bool {
        let cross = |constraint: &BoxConstraint| {
            if is_row {
                constraint.max_height
            } else {
                constraint.max_width
            }
        };
        let mut retired = false;
        // Exclusive reference, dropped before `get` can run again.
        let held = unsafe { &mut *self.tables.get() };
        for slot in held.iter_mut() {
            let outdated = slot.as_ref().is_some_and(|held| {
                held.scale_bits == scale_bits
                    && held.constraint != kept
                    && cross(&held.constraint) == cross(&kept)
                    && held.layout.is_estimated()
                    && held.layout.total() != total
            });
            if outdated {
                *slot = None;
                retired = true;
            }
        }
        retired
    }

    /// Returns the cached paint order for `range`, rebuilding it only when the
    /// range or element-tree generation changes.
    ///
    /// Layer is structural element state: a generated child replacement
    /// advances the element-tree generation, while scrolling a windowed list
    /// changes the painted range. Those two keys make reusing the order safe
    /// without rescanning every child's layer on every frame.
    #[inline]
    pub(crate) fn cached_layer_order(
        &self,
        range: Range<usize>,
        mut layer_of: impl FnMut(usize) -> Option<u32>,
    ) -> LayerOrder {
        let generation = aimer_widget::element_tree_generation();
        let key = (range.start, range.end);
        let cached = unsafe { &*self.paint_order.get() };
        if let Some(cached) = cached.as_ref()
            && cached.generation == generation
            && cached.range == key
        {
            return cached.order.clone();
        }

        let mut order = Vec::with_capacity(range.len().min(64));
        let mut layered = false;
        for index in range {
            let Some(layer) = layer_of(index) else {
                continue;
            };
            layered |= layer != 0;
            order.push((layer, index));
        }

        let order = if layered {
            order.sort_by_key(|(layer, _)| *layer);
            LayerOrder::Sorted(Rc::from(order.into_boxed_slice()))
        } else {
            LayerOrder::Unlayered
        };

        let slot = unsafe { &mut *self.paint_order.get() };
        *slot = Some(CachedLayerOrder {
            generation,
            range: key,
            order: order.clone(),
        });
        order
    }

    /// Takes over the table and painted range of the cache this one replaces.
    ///
    /// Everything a predicted table learned by painting — the exact extent of
    /// every row a frame corrected — lives in the table. A rebuild of the
    /// container would otherwise start from the prediction again, which a scroll
    /// view sees as its content suddenly changing size.
    ///
    /// The table keeps the generation it was measured in, so the replacement
    /// inherits exactly as much of it as [`FlexLayoutCache::get`] still trusts:
    /// a prediction or a declared extent, which never described a particular
    /// child, comes across whole, while extents measured from rows that were
    /// built anew are retired by the very rebuild that carried them. That is
    /// what a `Column` an `AsyncBuilder` rebuilds needs — same direction, same
    /// count, completely different children.
    ///
    /// Only sound for a container describing the same children under the same
    /// rules, which the caller establishes. Nothing already cached here is
    /// overwritten.
    pub(crate) fn adopt(&self, old: &Self) {
        let held = unsafe { &mut *self.tables.get() };
        if held.iter().all(Option::is_none) {
            // Taken rather than cloned: the cache it came from is dropped with
            // the container being replaced.
            *held = unsafe { std::mem::take(&mut *old.tables.get()) };
        }
        if self.painted.get().is_none() {
            self.painted.set(old.painted.get());
        }
        // The replacement may contain different layer-bearing children even
        // when its layout table can be adopted. Recompute paint order on its
        // first draw instead of carrying an order for the old elements.
        unsafe {
            *self.paint_order.get() = None;
            *self.hit_test_index.get() = None;
        }
    }

    /// Drops the table and the painted range.
    #[inline]
    pub(crate) fn invalidate(&self) {
        unsafe {
            (*self.tables.get()).fill_with(|| None);
            *self.paint_order.get() = None;
            *self.hit_test_index.get() = None;
        }
        self.painted.set(None);
    }

    /// Records the index range painted by the most recent frame.
    #[inline]
    pub(crate) fn set_painted(&self, range: &Range<usize>) {
        self.painted.set(Some((range.start, range.end)));
        unsafe {
            *self.hit_test_index.get() = None;
        }
    }

    /// Returns the index range painted by the most recent frame.
    #[inline]
    pub(crate) fn painted(&self) -> Option<Range<usize>> {
        self.painted.get().map(|(start, end)| start..end)
    }

    /// Retires retained child intervals before a new frame updates bounds.
    #[inline]
    pub(crate) fn invalidate_hit_test_index(&self) {
        unsafe {
            *self.hit_test_index.get() = None;
        }
    }

    /// Builds a coarse main-axis index from the retained bounds of a wide
    /// painted range. The index is rebuilt after painting, so event dispatch
    /// does not call every child's virtual `pos_start_end` method.
    pub(crate) fn rebuild_hit_test_index(
        &self,
        range: Range<usize>,
        is_row: bool,
        mut bounds_of: impl FnMut(usize) -> Option<(Vec2d, Vec2d)>,
    ) {
        let child_count = range.len();
        if child_count <= HIT_TEST_CHILD_THRESHOLD {
            unsafe {
                *self.hit_test_index.get() = None;
            }
            return;
        }

        let mut intervals = Vec::with_capacity(child_count);
        let mut min_main = f32::INFINITY;
        let mut max_main = f32::NEG_INFINITY;
        let mut known_count = 0usize;

        for index in range.clone() {
            let interval = bounds_of(index)
                .filter(|(start, end)| {
                    let start_main = if is_row { start.x } else { start.y };
                    let end_main = if is_row { end.x } else { end.y };
                    start_main.is_finite() && end_main.is_finite() && start_main <= end_main
                })
                .map(|(start, end)| {
                    (
                        if is_row { start.x } else { start.y },
                        if is_row { end.x } else { end.y },
                    )
                });
            if let Some((start, end)) = interval {
                min_main = min_main.min(start);
                max_main = max_main.max(end);
                known_count += 1;
            }
            intervals.push(interval);
        }

        let span = max_main - min_main;
        let indexable = known_count > 0 && span.is_finite();
        let bin_height = if indexable && span > 0.0 {
            span / HIT_TEST_BIN_COUNT as f32
        } else {
            1.0
        };
        let bin_range = |start: f32, end: f32| {
            let first_bin = ((start - min_main) / bin_height)
                .floor()
                .clamp(0.0, (HIT_TEST_BIN_COUNT - 1) as f32) as usize;
            let last_bin = ((end - min_main) / bin_height)
                .floor()
                .clamp(0.0, (HIT_TEST_BIN_COUNT - 1) as f32) as usize;
            (first_bin, last_bin)
        };

        let mut estimated_entries = 0usize;
        if indexable {
            for interval in &intervals {
                let entries = interval.map_or(HIT_TEST_BIN_COUNT, |(start, end)| {
                    let (first_bin, last_bin) = bin_range(start, end);
                    last_bin - first_bin + 1
                });
                estimated_entries = estimated_entries.saturating_add(entries);
            }
        }

        // If most intervals span most bins, consulting the index would only
        // add indirection. Keep the exact linear fallback for that shape.
        if !indexable || estimated_entries > child_count.saturating_mul(4) {
            unsafe {
                *self.hit_test_index.get() = None;
            }
            return;
        }

        let mut bins: Vec<Vec<usize>> = (0..HIT_TEST_BIN_COUNT).map(|_| Vec::new()).collect();
        let mut outside = Vec::new();
        for (offset, interval) in intervals.into_iter().enumerate() {
            let index = range.start + offset;
            if let Some((start, end)) = interval {
                let (first_bin, last_bin) = bin_range(start, end);
                for bin in first_bin..=last_bin {
                    bins[bin].push(index);
                }
            } else {
                outside.push(index);
                for bin in &mut bins {
                    bin.push(index);
                }
            }
        }

        let mut offsets = Vec::with_capacity(HIT_TEST_BIN_COUNT + 1);
        let mut indexed_children = Vec::with_capacity(estimated_entries);
        offsets.push(0);
        for bin in bins {
            indexed_children.extend(bin);
            offsets.push(indexed_children.len());
        }

        let slot = unsafe { &mut *self.hit_test_index.get() };
        *slot = Some(CachedHitTestIndex {
            generation: aimer_widget::element_tree_generation(),
            layout_generation: aimer_widget::layout_invalidation_generation(),
            range: (range.start, range.end),
            is_row,
            min_main,
            max_main,
            bin_height,
            offsets: Rc::from(offsets.into_boxed_slice()),
            children: Rc::from(indexed_children.into_boxed_slice()),
            outside: Rc::from(outside.into_boxed_slice()),
        });
    }

    /// Returns indexed candidates in ascending child order, or `None` when the
    /// range has no usable sparse index and the exact fallback should run.
    #[inline]
    pub(crate) fn hit_test_candidates(&self, range: &Range<usize>, pos: Vec2d) -> Option<&[usize]> {
        unsafe {
            (&*self.hit_test_index.get())
                .as_ref()?
                .candidates(range, pos)
        }
    }
}

impl Default for FlexLayoutCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Reads the main-axis extent of `size`.
#[inline]
fn main_of(size: ResolvedSize, is_row: bool) -> f32 {
    if is_row { size.width } else { size.height }
}

/// Reads the cross-axis extent of `size`.
#[inline]
fn cross_of(size: ResolvedSize, is_row: bool) -> f32 {
    if is_row { size.height } else { size.width }
}

/// Overwrites the main-axis extent of `size`.
#[inline]
fn set_main_of(size: &mut ResolvedSize, is_row: bool, value: f32) {
    if is_row {
        size.width = value;
    } else {
        size.height = value;
    }
}

/// Overwrites the cross-axis extent of `size`.
#[inline]
fn set_cross_of(size: &mut ResolvedSize, is_row: bool, value: f32) {
    if is_row {
        size.height = value;
    } else {
        size.width = value;
    }
}

/// Builds a size from main- and cross-axis extents.
#[inline]
/// Narrows an `f64` extent to `f32`, saturating at `f32::MAX` instead of
/// overflowing to infinity. Children that each ask for `f32::MAX` (a percentage
/// of an unbounded axis) would otherwise sum to a non-finite size that the
/// render tree cannot hold.
#[inline]
pub(crate) fn saturate_f32(value: f64) -> f32 {
    (value as f32).min(f32::MAX)
}

fn sized(main: f32, cross: f32, is_row: bool) -> ResolvedSize {
    if is_row {
        ResolvedSize {
            width: main,
            height: cross,
        }
    } else {
        ResolvedSize {
            width: cross,
            height: main,
        }
    }
}

/// Replaces the main-axis maximum of `constraint`.
#[inline]
fn set_main(constraint: &mut BoxConstraint, is_row: bool, value: f32) {
    if is_row {
        constraint.max_width = value;
    } else {
        constraint.max_height = value;
    }
}

/// Replaces the cross-axis maximum of `constraint`.
#[inline]
fn set_cross(constraint: &mut BoxConstraint, is_row: bool, value: f32) {
    if is_row {
        constraint.max_height = value;
    } else {
        constraint.max_width = value;
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the cached main-axis table built in
    //! [`flex_layout`](super).

    use std::cell::Cell;
    use std::hint::black_box;
    use std::time::Instant;

    use super::*;
    use crate::flex::JustifyContent;
    use crate::flex::raw_flex::justify_distribution;

    #[test]
    fn cached_layer_order_reuses_a_sorted_range() {
        let cache = FlexLayoutCache::new();
        let layers = [4, 1, 3, 2];
        let calls = Cell::new(0);

        let first = cache.cached_layer_order(0..layers.len(), |index| {
            calls.set(calls.get() + 1);
            Some(layers[index])
        });
        let LayerOrder::Sorted(order) = first else {
            panic!("nonzero layers must produce a sorted order");
        };
        assert_eq!(order.as_ref(), &[(1, 1), (2, 3), (3, 2), (4, 0)]);
        assert_eq!(calls.get(), layers.len());

        let second = cache.cached_layer_order(0..layers.len(), |_| {
            panic!("a matching generation and range must use the cache")
        });
        let LayerOrder::Sorted(order) = second else {
            panic!("the cached result must retain its layer state");
        };
        assert_eq!(order.as_ref(), &[(1, 1), (2, 3), (3, 2), (4, 0)]);
    }

    #[test]
    fn cached_layer_order_invalidates_with_the_layout_cache() {
        let cache = FlexLayoutCache::new();
        let calls = Cell::new(0);

        let first = cache.cached_layer_order(0..3, |index| {
            calls.set(calls.get() + 1);
            Some(index as u32 + 1)
        });
        assert!(matches!(first, LayerOrder::Sorted(_)));
        assert_eq!(calls.get(), 3);

        cache.invalidate();
        let second = cache.cached_layer_order(0..3, |index| {
            calls.set(calls.get() + 1);
            Some(index as u32 + 1)
        });
        assert!(matches!(second, LayerOrder::Sorted(_)));
        assert_eq!(calls.get(), 6);

        let unlayered = cache.cached_layer_order(3..6, |_| Some(0));
        assert!(matches!(unlayered, LayerOrder::Unlayered));
    }

    #[test]
    #[ignore = "manual numeric-kernel profile"]
    fn profile_size_table_construction() {
        const MEASURED: usize = 64;
        const WARMUP: usize = 16;
        const ROUNDS: usize = 7;

        let cases: [(&str, Vec<ResolvedSize>); 4] = [
            (
                "uniform-256",
                (0..256).map(|_| sized(800.0, 20.0, false)).collect(),
            ),
            (
                "uniform-2048",
                (0..2_048).map(|_| sized(800.0, 20.0, false)).collect(),
            ),
            (
                "varying-256",
                (0..256)
                    .map(|index| sized(800.0, 20.0 + (index % 5) as f32, false))
                    .collect(),
            ),
            (
                "varying-2048",
                (0..2_048)
                    .map(|index| sized(800.0, 20.0 + (index % 5) as f32, false))
                    .collect(),
            ),
        ];

        for (name, template) in cases {
            let mut samples = Vec::with_capacity(ROUNDS);
            let mut checksum = 0.0;
            for _ in 0..ROUNDS {
                let inputs: Vec<_> = (0..WARMUP + MEASURED).map(|_| template.clone()).collect();
                let mut inputs = inputs.into_iter();

                for _ in 0..WARMUP {
                    let layout = black_box(FlexLayout::from_sizes(
                        black_box(inputs.next().expect("warmup input")),
                        false,
                        0.0,
                        false,
                    ));
                    checksum = black_box(checksum + layout.total().height);
                }

                let start = Instant::now();
                for _ in 0..MEASURED {
                    let layout = black_box(FlexLayout::from_sizes(
                        black_box(inputs.next().expect("measured input")),
                        false,
                        0.0,
                        false,
                    ));
                    checksum = black_box(checksum + layout.total().height);
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED as f64);
            }

            samples.sort_by(f64::total_cmp);
            let p50 = samples[ROUNDS / 2];
            let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
            println!("{name}: p50 {p50:.3} us, p95 {p95:.3} us");
            assert!(checksum.is_finite());
        }
    }

    #[test]
    fn flex_space_resolution_keeps_non_flex_slots_and_zero_markers() {
        let mut weights = [NOT_FLEX, 1.0, 0.0, 2.0];

        FlexLayout::resolve_flex_space(600.0, 100.0, 10.0, &mut weights);

        assert_eq!(weights[0], NOT_FLEX);
        assert!((weights[1] - 163.33333).abs() < 0.001);
        assert_eq!(weights[2], 0.0);
        assert!(weights[2].is_sign_negative());
        assert!((weights[3] - 326.66666).abs() < 0.001);
    }

    #[test]
    fn selected_flex_dispatch_preserves_layout_positions_and_sizes() {
        let mut selected = [NOT_FLEX, 1.0, 0.0, 2.0, 3.0, NOT_FLEX];
        let mut scalar = selected;
        let remaining = 1_000.0 - 95.0 - 20.0;

        FlexLayout::resolve_flex_space(1_000.0, 95.0, 20.0, &mut selected);
        distribute_flex_space_in_place_scalar_reference(remaining, &mut scalar[1..5]);

        let build = |weights: &[f32]| {
            let regular_sizes = [40.0, 0.0, 0.0, 0.0, 0.0, 55.0];
            let sizes = weights
                .iter()
                .enumerate()
                .map(|(index, weight)| {
                    let main = if *weight > 0.0 {
                        *weight
                    } else if *weight == 0.0 {
                        0.0
                    } else {
                        regular_sizes[index]
                    };
                    sized(main, 24.0 + index as f32, false)
                })
                .collect();
            FlexLayout::from_sizes(sizes, false, 4.0, true)
        };

        let selected_layout = build(&selected);
        let scalar_layout = build(&scalar);
        assert_eq!(selected_layout.len(), scalar_layout.len());
        assert!((selected_layout.total().height - scalar_layout.total().height).abs() < 1.0e-3);

        for index in 0..selected_layout.len() {
            let selected_size = selected_layout.size(index);
            let scalar_size = scalar_layout.size(index);
            assert!((selected_size.height - scalar_size.height).abs() < 1.0e-3);
            assert!((selected_layout.offset(index) - scalar_layout.offset(index)).abs() < 1.0e-3);
        }
    }

    fn column_of(sizes: &[(f32, f32)], gap: f32) -> FlexLayout {
        let sizes = sizes
            .iter()
            .map(|(width, height)| ResolvedSize {
                width: *width,
                height: *height,
            })
            .collect();
        FlexLayout::from_sizes(sizes, false, gap, false)
    }

    #[test]
    fn justify_content_distributes_free_main_axis_space() {
        assert_eq!(
            justify_distribution(JustifyContent::Start, 60.0, 3),
            (0.0, 0.0)
        );
        assert_eq!(
            justify_distribution(JustifyContent::Center, 60.0, 3),
            (30.0, 0.0)
        );
        assert_eq!(
            justify_distribution(JustifyContent::End, 60.0, 3),
            (60.0, 0.0)
        );
        assert_eq!(
            justify_distribution(JustifyContent::SpaceBetween, 60.0, 3),
            (0.0, 30.0)
        );
        assert_eq!(
            justify_distribution(JustifyContent::SpaceAround, 60.0, 3),
            (10.0, 20.0)
        );
        assert_eq!(
            justify_distribution(JustifyContent::SpaceEvenly, 60.0, 3),
            (15.0, 15.0)
        );
    }

    #[test]
    fn space_between_falls_back_to_start_for_one_child() {
        assert_eq!(
            justify_distribution(JustifyContent::SpaceBetween, 60.0, 1),
            (0.0, 0.0)
        );
    }

    #[test]
    fn visible_range_accounts_for_distributed_main_axis_space() {
        let layout = FlexLayout::from_sizes(
            vec![
                ResolvedSize {
                    width: 10.0,
                    height: 10.0,
                },
                ResolvedSize {
                    width: 10.0,
                    height: 10.0,
                },
                ResolvedSize {
                    width: 10.0,
                    height: 10.0,
                },
            ],
            true,
            0.0,
            false,
        );

        assert_eq!(
            layout.visible_range_with_extra_space(25.0, 35.0, 0.0, 15.0),
            1..2
        );
    }

    #[test]
    fn uniform_column_stores_one_size_and_a_stride() {
        let layout = column_of(&[(10.0, 20.0); 4], 5.0);

        assert_eq!(layout.len(), 4);
        assert_eq!(layout.sizes.len(), 1);
        assert_eq!(layout.stride, Some(25.0));
        assert_eq!(layout.offset(0), 0.0);
        assert_eq!(layout.offset(3), 75.0);
        // Four 20px children with three 5px gaps.
        assert_eq!(layout.total().height, 95.0);
        assert_eq!(layout.total().width, 10.0);
    }

    #[test]
    fn varying_column_records_every_offset() {
        let layout = column_of(&[(10.0, 20.0), (30.0, 40.0), (5.0, 10.0)], 2.0);

        assert_eq!(layout.stride, None);
        assert_eq!(layout.offset(0), 0.0);
        assert_eq!(layout.offset(1), 22.0);
        assert_eq!(layout.offset(2), 64.0);
        assert_eq!(layout.total().height, 74.0);
        assert_eq!(layout.total().width, 30.0);
    }

    #[test]
    fn uniform_visible_range_covers_the_touched_children() {
        let layout = column_of(&[(10.0, 100.0); 1_000], 0.0);

        assert_eq!(layout.visible_range(0.0, 250.0), 0..3);
        assert_eq!(layout.visible_range(450.0, 650.0), 4..7);
        assert_eq!(layout.visible_range(99_900.0, 100_500.0), 999..1_000);
    }

    #[test]
    fn varying_visible_range_matches_the_uniform_result() {
        let layout = column_of(
            &[(10.0, 100.0), (20.0, 100.0), (10.0, 100.0), (10.0, 100.0)],
            0.0,
        );

        assert_eq!(layout.visible_range(0.0, 150.0), 0..2);
        assert_eq!(layout.visible_range(150.0, 250.0), 1..3);
        assert_eq!(layout.visible_range(1_000.0, 1_100.0), 4..4);
    }

    #[test]
    fn empty_layout_has_an_empty_range() {
        let layout = column_of(&[], 4.0);

        assert_eq!(layout.len(), 0);
        assert_eq!(layout.total(), ResolvedSize::default());
        assert_eq!(layout.visible_range(0.0, 100.0), 0..0);
    }

    #[test]
    fn declared_extent_builds_a_stride_without_sizes() {
        let layout = FlexLayout::declared(100_000, 200.0, 400.0, false, 10.0);

        assert!(layout.is_declared());
        assert_eq!(layout.len(), 100_000);
        assert_eq!(layout.stride, Some(210.0));
        assert_eq!(layout.offsets.len(), 0);
        assert_eq!(layout.offset(99_999), 99_999.0 * 210.0);
        assert_eq!(
            layout.size(50_000),
            ResolvedSize {
                width: 400.0,
                height: 200.0,
            }
        );
        // 100 000 children of 200px with 99 999 gaps of 10px.
        assert_eq!(layout.total().height, 100_000.0 * 210.0 - 10.0);
        assert_eq!(layout.total().width, 400.0);
        // Children start at 0, 210, and 420, so three of them touch a 600px
        // viewport.
        assert_eq!(layout.visible_range(0.0, 600.0), 0..3);
    }

    /// A prediction has to be indistinguishable from the measured table of a
    /// uniform list, apart from admitting that it is a prediction.
    #[test]
    fn an_estimated_extent_matches_a_measured_uniform_table() {
        let probe = ResolvedSize {
            width: 10.0,
            height: 200.0,
        };
        let estimated = FlexLayout::estimated(100_000, probe, false, 10.0);
        let measured = column_of(&[(10.0, 200.0); 4], 10.0);

        assert!(estimated.is_estimated());
        assert!(!estimated.is_declared());
        assert_eq!(estimated.stride, measured.stride);
        assert_eq!(estimated.offsets.len(), 0);
        assert_eq!(estimated.size(50_000), probe);
        assert_eq!(estimated.offset(99_999), 99_999.0 * 210.0);
        assert_eq!(estimated.total().height, 100_000.0 * 210.0 - 10.0);
        // Nothing but the probe was measured, so its cross extent is all the
        // container can report.
        assert_eq!(estimated.total().width, 10.0);
    }

    /// A measured table must never be mistaken for a prediction, or it would be
    /// re-verified against its own children forever.
    #[test]
    fn a_measured_table_is_neither_declared_nor_estimated() {
        let layout = column_of(&[(10.0, 20.0); 4], 5.0);

        assert!(!layout.is_declared());
        assert!(!layout.is_estimated());
    }

    #[test]
    fn declared_empty_list_matches_a_measured_empty_list() {
        let layout = FlexLayout::declared(0, 200.0, 400.0, false, 10.0);

        assert_eq!(layout.len(), 0);
        assert_eq!(layout.total(), ResolvedSize::default());
        assert_eq!(layout.visible_range(0.0, 100.0), 0..0);
    }

    /// A tall list must keep exact offsets: `f32` cannot represent every
    /// multiple of 110 past ~8.4 million.
    #[test]
    fn deep_offsets_stay_exact() {
        let layout = column_of(&[(10.0, 80.0); 120_000], 30.0);

        assert_eq!(layout.offset(119_999), 119_999.0 * 110.0);
        assert_eq!(
            layout.visible_range(13_199_890.0, 13_199_970.0),
            119_999..120_000
        );
    }

    /// A hundred-thousand-row prediction with one 400px row somewhere inside it.
    fn predicted_column() -> FlexLayout {
        FlexLayout::estimated(
            100_000,
            ResolvedSize {
                width: 10.0,
                height: 200.0,
            },
            false,
            0.0,
        )
    }

    /// Correcting one child must move the children after it and nothing else.
    ///
    /// This is the invariant that lets a prediction be corrected while the user is
    /// looking at it: the row under the viewport keeps the offset it was painted at,
    /// so the correction is never visible as a jump.
    #[test]
    fn a_correction_moves_only_the_children_after_it() {
        let layout = predicted_column();

        assert!(layout.refine(
            40_000,
            ResolvedSize {
                width: 10.0,
                height: 500.0,
            }
        ));

        assert_eq!(layout.offset(0), 0.0);
        assert_eq!(layout.offset(40_000), 40_000.0 * 200.0);
        assert_eq!(layout.offset(40_001), 40_000.0 * 200.0 + 500.0);
        assert_eq!(layout.offset(99_999), 99_999.0 * 200.0 + 300.0);
        assert_eq!(layout.size(40_000).height, 500.0);
        assert_eq!(layout.size(40_001).height, 200.0);
        // The rows that were never looked at keep the probe, so the total carries
        // exactly the one correction.
        assert_eq!(layout.total().height, 100_000.0 * 200.0 + 300.0);
    }

    /// Several corrections have to accumulate, in any order, and a repeated one must
    /// replace rather than add to the previous value.
    #[test]
    fn corrections_accumulate_and_replace() {
        let layout = predicted_column();
        let tall = |height| ResolvedSize {
            width: 10.0,
            height,
        };

        layout.refine(7, tall(300.0));
        layout.refine(3, tall(250.0));
        layout.refine(7, tall(400.0));

        // Row 3 grew by 50 and row 7 by 200.
        assert_eq!(layout.offset(4), 4.0 * 200.0 + 50.0);
        assert_eq!(layout.offset(8), 8.0 * 200.0 + 250.0);
        assert_eq!(layout.total().height, 100_000.0 * 200.0 + 250.0);
    }

    /// A correction changes which children a span covers, so the range lookup has to
    /// read the corrections rather than the stride alone.
    #[test]
    fn a_corrected_range_accounts_for_the_correction() {
        let layout = predicted_column();

        // Rows now start at 0, 200, 1000, 1200, ...
        layout.refine(
            1,
            ResolvedSize {
                width: 10.0,
                height: 800.0,
            },
        );

        assert_eq!(layout.visible_range(0.0, 600.0), 0..2);
        // Rows 2, 3, and 4 span 1000..1200, 1200..1400, and 1400..1600.
        assert_eq!(layout.visible_range(1_100.0, 1_500.0), 2..5);
    }

    /// The container's cross-axis size has to grow with a wider corrected child, or
    /// the row would be clipped by the size its own parent was told.
    #[test]
    fn a_correction_widens_the_container() {
        let layout = predicted_column();

        layout.refine(
            5,
            ResolvedSize {
                width: 90.0,
                height: 200.0,
            },
        );

        assert_eq!(layout.total().width, 90.0);
    }

    /// Only a prediction may be corrected. A declared extent is what the caller
    /// stated, and a measured table is exact already — correcting either would make
    /// the container disagree with itself.
    #[test]
    fn only_a_prediction_accepts_corrections() {
        let declared = FlexLayout::declared(100, 200.0, 400.0, false, 0.0);
        let measured = column_of(&[(10.0, 200.0); 100], 0.0);
        let predicted = predicted_column();
        let tall = ResolvedSize {
            width: 10.0,
            height: 900.0,
        };

        assert!(!declared.refine(5, tall));
        assert!(!measured.refine(5, tall));
        assert!(!predicted.refine(100_000, tall), "out of bounds");

        assert_eq!(declared.total().height, 100.0 * 200.0);
        assert_eq!(measured.total().height, 100.0 * 200.0);
    }

    /// Correcting every row has to leave the table exactly as measuring the list
    /// would have: that is what makes the prediction converge instead of merely
    /// approximate.
    #[test]
    fn correcting_every_row_reaches_the_measured_result() {
        let predicted = FlexLayout::estimated(
            6,
            ResolvedSize {
                width: 10.0,
                height: 200.0,
            },
            false,
            5.0,
        );
        let heights = [50.0, 200.0, 300.0, 120.0, 200.0, 80.0];
        let measured = column_of(&heights.map(|height: f32| (10.0, height)), 5.0);

        for (index, height) in heights.iter().enumerate() {
            predicted.refine(
                index,
                ResolvedSize {
                    width: 10.0,
                    height: *height,
                },
            );
        }

        assert_eq!(predicted.total(), measured.total());
        for index in 0..heights.len() {
            assert_eq!(predicted.offset(index), measured.offset(index));
            assert_eq!(predicted.size(index), measured.size(index));
        }
    }
}
