//! Leaving a subtree alone in a frame that changed nothing inside it.
//!
//! Walking the element tree costs the same in every frame, whatever the frame is
//! for: a sidebar next to a scrolling page is walked again in every scroll frame
//! only to find what the last walk left.
//!
//! A subtree can be left alone when walking it would reproduce the last walk. The
//! conditions are checked directly, so they hold in any frame, a scroll-only one
//! or a full one; a frame requested by scrolling alone is no more than the case
//! in which they are guaranteed to hold for everything outside the scroll:
//!
//! 1. Every element walked in it promised that (`Drawable::reusable_walk`). One
//!    that did not bumps a counter, and a subtree that saw the counter move is
//!    never recorded.
//! 2. It is given the same context. The key holds everything an element can read
//!    from its context, including the pointer and the canvas translation, and the
//!    part of the subtree's root that the retained tree shows. A container reads
//!    that window to decide which rows to walk, and a scroll that moves content
//!    under a fixed clip changes it without changing the context. Every clip
//!    inside a subtree of repeatable elements is fixed relative to its root, so
//!    the root's window stands for all of them. The same holds for where it is
//!    in the world: a hover region compares the pointer with it.
//! 3. Nothing was invalidated since: the tree, layout and rebuild generations are
//!    the ones the walk ended with, and no paint invalidation is pending.
//! 4. The last full traversal walked it. A record from an older traversal may
//!    describe a subtree that changed in between.
//!
//! Only the first reached element of a qualifying subtree is skipped, and only
//! subtrees of a few elements are recorded: a record for a leaf costs more than
//! the walk it saves.
//!
//! This is an optimisation with a switch ([`set_clean_subtree_skip`]) so a
//! differential test can compare the frames with and without it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

#[cfg(test)]
mod tests;

use aimer_cupid::draw_cmd::RenderNodeState;
use aimer_cupid::utilities::Mat3;
use aimer_attribute::position::Vec2d;

use super::animation_only::stale_roots_waiting;
use super::*;

/// How many records are kept. Ids are handed out in sequence, so a window with
/// up to this many recorded subtrees maps them to distinct slots.
const SLOTS: usize = 4096;

/// How many elements a walk must have visited, the element itself included, for
/// its subtree to be worth a record.
const MIN_SUBTREE: u64 = 8;

/// What the skip did, for tests and diagnostics.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CleanSkipStats {
    /// Elements walked by the traversal, skipped subtrees excluded.
    pub visits: u64,
    /// Subtrees left alone.
    pub skips: u64,
    /// Subtrees recorded after a walk.
    pub records: u64,
    /// Subtrees verify mode would have skipped and walked instead.
    pub verified: u64,
    /// How many times the retained tree was asked what it shows of a subtree.
    pub tree_queries: u64,
}

/// Everything an element can read from the context it is given, as bits so that
/// two keys are equal only when the inputs are identical.
///
/// Built for every subtree that is recorded, so it reads the context and nothing
/// else. What the retained tree shows of the subtree is the [`TreeKey`], which
/// costs queries and is only kept for subtrees that held still.
#[derive(Clone, Copy, PartialEq)]
struct ContextKey {
    size: [u32; 2],
    scale: u32,
    position: [u32; 2],
    cursor: [u32; 2],
    constraint: [u32; 4],
    visible: Option<[u32; 4]>,
    background: u32,
    /// The whole canvas transform, for a subtree that read it: an element can read
    /// its place from it, and a scale or rotation above it is as much an input as a
    /// translation. A subtree that never looked at where it is drawn is the same
    /// wherever it is, which is what lets the rows of a list that scrolls be left
    /// alone while they move.
    transform: Option<Mat3>,
}

/// What a walk looked at beyond the fixed parts of its context, and so what its key
/// has to hold.
#[derive(Clone, Copy, Default, PartialEq)]
struct Reads {
    /// The pointer position was asked for.
    cursor: bool,
    /// The canvas transform was asked for.
    transform: bool,
}

impl Reads {
    #[cfg(test)]
    const NONE: Self = Self { cursor: false, transform: false };
}

/// What a subtree can tell of the visible rectangle it is given, as bits.
///
/// Elements read it to leave out what is off screen: a container walks a child only
/// if the child's rectangle overlaps it. Along an axis on which the subtree, with a
/// margin of its own extent all round, lies wholly inside the rectangle, every
/// rectangle the subtree can ask about is inside it too, so where the rectangle's
/// edges are along that axis does not change what the subtree does, and they are
/// replaced by one marker. That is what lets the rows of a list that scrolls be told
/// apart from rows that changed: while they move, only the rectangle moves, along
/// the axis of the scroll. Along an axis the rectangle cuts across, its edges stay
/// exact. The axes are told apart so that a viewport exactly as wide as its rows,
/// which has no room for a margin across, still lets them move up and down.
#[inline]
fn visible_bits(ctx: &BuildContext) -> Option<[u32; 4]> {
    let (x, y, width, height) = ctx.visible_rect()?;
    let (mut x_bits, mut width_bits) = (x.to_bits(), width.to_bits());
    let (mut y_bits, mut height_bits) = (y.to_bits(), height.to_bits());
    if RELATIVE_WINDOW.with(Cell::get) {
        let (across, down) = (ctx.parent_size.width, ctx.parent_size.height);
        if contained_with_margin(x, width, across) {
            (x_bits, width_bits) = (u32::MAX, u32::MAX);
        }
        if contained_with_margin(y, height, down) {
            (y_bits, height_bits) = (u32::MAX, u32::MAX);
        }
    }
    Some([x_bits, y_bits, width_bits, height_bits])
}

/// Whether the span `start..start + length` reaches `extent` past both ends of
/// `0..extent`.
#[inline]
pub(super) fn contained_with_margin(start: f32, length: f32, extent: f32) -> bool {
    start.is_finite()
        && length.is_finite()
        && extent.is_finite()
        && extent > 0.0
        && start <= -extent
        && start + length >= extent + extent
}

impl ContextKey {
    /// Whether `ctx` gives the key this key was made from, without making a key.
    ///
    /// Compares the fields a moving subtree differs in first, and the canvas, whose
    /// queries cost the most, last: most candidates are rejected on the first or
    /// second comparison.
    fn matches(&self, ctx: &BuildContext, reads: Reads) -> bool {
        let position = ctx.parent_pos();
        if self.position != [position.x.to_bits(), position.y.to_bits()] {
            miss("differs: position");
            return false;
        }
        if self.size != [ctx.parent_size.width.to_bits(), ctx.parent_size.height.to_bits()]
            || self.scale != ctx.scale.to_bits()
        {
            miss("differs: size or scale");
            return false;
        }
        let constraint = ctx.box_constraint;
        if self.constraint
            != [
                constraint.min_width.to_bits(),
                constraint.min_height.to_bits(),
                constraint.max_width.to_bits(),
                constraint.max_height.to_bits(),
            ]
        {
            miss("differs: constraint");
            return false;
        }
        if self.visible != visible_bits(ctx) {
            miss("differs: visible rectangle");
            return false;
        }
        if self.background != ctx.paint_background().as_u32() {
            miss("differs: background");
            return false;
        }
        if reads.cursor {
            let cursor = ctx.cursor_pos_unobserved();
            if self.cursor != [cursor.x.to_bits(), cursor.y.to_bits()] {
                miss("differs: pointer");
                return false;
            }
        }
        if reads.transform && self.transform != Some(ctx.canvas.get_transform_unobserved()) {
            miss("differs: canvas transform");
            return false;
        }
        true
    }

    /// `reads` is what the subtree depends on beyond the fixed parts; what it does
    /// not depend on is left out of the key.
    fn of(ctx: &BuildContext, reads: Reads) -> Self {
        let position = ctx.parent_pos();
        // Asked of the field directly so that building a key is not itself a
        // read: it would make every subtree look like it depends on the pointer.
        let cursor = if reads.cursor { ctx.cursor_pos_unobserved() } else { Vec2d::default() };
        let constraint = ctx.box_constraint;
        Self {
            size: [ctx.parent_size.width.to_bits(), ctx.parent_size.height.to_bits()],
            scale: ctx.scale.to_bits(),
            position: [position.x.to_bits(), position.y.to_bits()],
            cursor: [cursor.x.to_bits(), cursor.y.to_bits()],
            constraint: [
                constraint.min_width.to_bits(),
                constraint.min_height.to_bits(),
                constraint.max_width.to_bits(),
                constraint.max_height.to_bits(),
            ],
            visible: visible_bits(ctx),
            background: ctx.paint_background().as_u32(),
            transform: reads.transform.then(|| ctx.canvas.get_transform_unobserved()),
        }
    }
}

/// What distinguishes the context a subtree is given from the one it was given a
/// walk ago, folded into one word.
///
/// Only a heuristic, for telling a subtree that holds still from one that moves:
/// a record carries its own exact key and is checked against it, so a field left
/// out here (the canvas transform, the scale, the background) can at worst make a
/// subtree look steady a walk too early, and cost a record that is replaced. It is
/// asked for every subtree of every walk, so it reads the few fields that move
/// with a scroll or a layout change and nothing that needs a query of the canvas.
#[inline]
fn context_hash(ctx: &BuildContext, reads: Reads) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |word: u32| {
        hash = (hash ^ u64::from(word)).wrapping_mul(0x0100_0000_01b3);
    };
    let position = ctx.parent_pos();
    mix(position.x.to_bits());
    mix(position.y.to_bits());
    mix(ctx.parent_size.width.to_bits());
    mix(ctx.parent_size.height.to_bits());
    mix(ctx.box_constraint.max_width.to_bits());
    mix(ctx.box_constraint.max_height.to_bits());
    if let Some(visible) = visible_bits(ctx) {
        for word in visible {
            mix(word);
        }
    }
    if reads.cursor {
        let cursor = ctx.cursor_pos_unobserved();
        mix(cursor.x.to_bits());
        mix(cursor.y.to_bits());
    }
    if reads.transform {
        let transform = ctx.canvas.get_transform_unobserved();
        mix(transform.cols[2][0].to_bits());
        mix(transform.cols[2][1].to_bits());
    }
    hash
}

/// What the retained tree shows of a subtree's root, for the inputs a walk reads
/// from the tree rather than from its context.
///
/// A container reads the part of itself that is on screen to decide which rows to
/// walk, and a hover region compares the pointer with where it is in the world;
/// a scroll that moves content under a fixed clip, or under a pointer that has
/// not moved, changes both without changing the context. Every clip inside a
/// subtree of repeatable elements is fixed relative to its root, so the root's
/// window and place stand for all of them.
///
/// Two queries of the tree, so it is kept only for a subtree whose context key
/// held still across two walks: the only ones a skip can ever apply to.
///
/// Both parts are only as exact as the subtree can tell. The window is made
/// relative where the subtree lies wholly inside it, and the place in the world
/// only matters to a subtree that read the pointer, and then only to a pointer
/// that is inside it: see [`same_for_the_pointer`].
#[derive(Clone, Copy, PartialEq)]
struct TreeKey {
    window: Option<VisibleWindow>,
    world: Option<[u32; 4]>,
}

impl TreeKey {
    fn of(ctx: &BuildContext, element: ElementId, reads: Reads) -> Self {
        STATS.with(|stats| {
            let mut current = stats.get();
            current.tree_queries += 1;
            stats.set(current);
        });
        let relative = RELATIVE_WINDOW.with(Cell::get);
        let window = retained_visible_window(element, ctx.scale);
        Self {
            window: match window {
                Some(window) if relative => {
                    Some(window.relative_to(ctx.parent_size.width, ctx.parent_size.height))
                }
                other => other,
            },
            world: if reads.cursor || !relative { world_bounds(element) } else { None },
        }
    }

    /// Whether a subtree recorded with `self` sees the same of the tree as one
    /// given `now`, with the pointer at `cursor` both times.
    fn matches(&self, now: &Self, cursor: (f32, f32)) -> bool {
        self.window == now.window && same_for_the_pointer(self.world, now.world, cursor)
    }
}

/// Whether a subtree at `before` and one at `after` are the same to a pointer at
/// `cursor`: a pointer that is inside neither is outside both times, and a hover
/// region compares nothing else.
///
/// Edges count as inside, as they do when a hit test compares a pointer with a
/// rectangle. A rectangle that appeared or vanished is a difference.
fn same_for_the_pointer(
    before: Option<[u32; 4]>,
    after: Option<[u32; 4]>,
    cursor: (f32, f32),
) -> bool {
    if before == after {
        return true;
    }
    let (Some(before), Some(after)) = (before, after) else {
        return false;
    };
    let inside = |rect: [u32; 4]| {
        let [x, y, width, height] = rect.map(f32::from_bits);
        x <= cursor.0 && cursor.0 <= x + width && y <= cursor.1 && cursor.1 <= y + height
    };
    !inside(before) && !inside(after)
}

/// Where the retained node of `element` is in the world right now.
fn world_bounds(element: ElementId) -> Option<[u32; 4]> {
    let scope = current_v2_render_context()?;
    let node = scope.node_for_element(element)?;
    let rect = scope.tree.context(node).ok()?.world_bounds()?;
    Some([rect.x.to_bits(), rect.y.to_bits(), rect.width.to_bits(), rect.height.to_bits()])
}

/// What one walk of a subtree left behind.
#[derive(Clone, Copy)]
struct Record {
    /// The element the slot holds a record of; zero for an empty slot.
    element: u64,
    /// The full traversal that walked, or last skipped, the subtree.
    traversal: u64,
    tree: u64,
    layout: u64,
    rebuild: u64,
    /// How many elements the walk visited, so a skip can count them.
    visits: u64,
    key: ContextKey,
    /// What the retained tree showed of the subtree's root when it was recorded.
    tree_key: TreeKey,
    /// The tree's geometry revision when `tree_key` was last known to be current:
    /// while it is unchanged the tree shows what it showed, and is not asked.
    geometry: u64,
    /// What the walk read; what it did not read, the key leaves out.
    reads: Reads,
}

/// The context a subtree was last walked with, in one word. Written for every
/// walk of a subtree big enough to matter, and read to tell whether the next walk
/// gave it the same one: only then is it worth building a record, because only a
/// subtree that holds still can be left alone.
#[derive(Clone, Copy)]
struct Seen {
    element: u64,
    hash: u64,
}

/// What an element's walk started from, to tell afterwards what happened below it.
pub(crate) struct Entry {
    visits: u64,
    impure: u64,
    tree: u64,
    cursor: u64,
    transform: u64,
}

aimer_tls::thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    /// Whether a subtree is given the same key wherever it is: the visible
    /// rectangle's edges are normalized when it lies wholly inside, and the canvas
    /// transform is part of the key only for a subtree that read it.
    static RELATIVE_WINDOW: Cell<bool> = const { Cell::new(true) };
    static VISITS: Cell<u64> = const { Cell::new(0) };
    /// What `VISITS` read when the statistics were last reset.
    static VISITS_BASE: Cell<u64> = const { Cell::new(0) };
    static IMPURE: Cell<u64> = const { Cell::new(0) };
    /// How many times a context has been asked for the pointer.
    static CURSOR_READS: Cell<u64> = const { Cell::new(0) };
    static STATS: Cell<CleanSkipStats> = const { Cell::new(CleanSkipStats { visits: 0, skips: 0, records: 0, verified: 0, tree_queries: 0 }) };
    static RECORDS: RefCell<Vec<Option<Record>>> = const { RefCell::new(Vec::new()) };
    static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };
    static PROBE: Cell<bool> = const { Cell::new(false) };
    static VERIFY: Cell<bool> = const { Cell::new(false) };
    static VIOLATIONS: RefCell<Vec<CleanSkipViolation>> = const { RefCell::new(Vec::new()) };
    static BLOCKERS: RefCell<HashMap<&'static str, u64>> = RefCell::new(HashMap::new());
    static REQUESTERS: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
}

/// A subtree whose walk changed what the retained tree holds although a skip would
/// have left it alone.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct CleanSkipViolation {
    /// The element at the root of the subtree that was walked.
    pub root: ElementId,
    /// What changed and in which element, as text.
    pub detail: String,
}

/// How many violations are kept; a broken promise tends to repeat every frame.
const MAX_VIOLATIONS: usize = 100;

/// Turns verify mode on or off.
///
/// In verify mode a subtree that would have been left alone is walked instead,
/// and what the walk leaves in the retained tree is compared with what was there
/// before it. A difference means the skip would have lost something, and is
/// recorded as a [`CleanSkipViolation`] naming the element. The frame is drawn as
/// it is without the skip, so this checks the promise at the point it is made, on
/// any page, with no second run to compare.
#[doc(hidden)]
pub fn set_clean_skip_verify(enabled: bool) {
    VERIFY.with(|flag| flag.set(enabled));
}

/// The violations verify mode found since the last reset.
#[doc(hidden)]
pub fn clean_skip_violations() -> Vec<CleanSkipViolation> {
    VIOLATIONS.with(|violations| violations.borrow().clone())
}

/// Turns on or off treating a subtree that lies wholly inside the visible
/// rectangle as the same wherever the rectangle's edges are.
///
/// On by default. It is what lets the rows of a list that scrolls be left alone
/// while they move; off, they are walked in every frame they move, as they were.
#[doc(hidden)]
pub fn set_clean_skip_relative_window(enabled: bool) {
    RELATIVE_WINDOW.with(|flag| flag.set(enabled));
    RECORDS.with(|records| records.borrow_mut().clear());
    SEEN.with(|seen| seen.borrow_mut().clear());
}

/// Enables or disables leaving subtrees alone in scroll-only frames.
///
/// On by default. Turning it off makes every frame walk every subtree, which is
/// what differential tests compare against.
#[doc(hidden)]
pub fn set_clean_subtree_skip(enabled: bool) {
    ENABLED.with(|flag| flag.set(enabled));
    if !enabled {
        RECORDS.with(|records| records.borrow_mut().clear());
        SEEN.with(|seen| seen.borrow_mut().clear());
    }
}

/// What the skip has done on this thread since the last reset.
#[doc(hidden)]
pub fn clean_subtree_skip_stats() -> CleanSkipStats {
    let mut stats = STATS.with(Cell::get);
    stats.visits = VISITS.with(Cell::get).wrapping_sub(VISITS_BASE.with(Cell::get));
    stats
}

/// Zeroes [`clean_subtree_skip_stats`].
#[doc(hidden)]
pub fn reset_clean_subtree_skip_stats() {
    VISITS_BASE.with(|base| base.set(VISITS.with(Cell::get)));
    STATS.with(|stats| stats.set(CleanSkipStats::default()));
    BLOCKERS.with(|blockers| blockers.borrow_mut().clear());
    REQUESTERS.with(|requesters| requesters.borrow_mut().clear());
    VIOLATIONS.with(|violations| violations.borrow_mut().clear());
}

/// Starts or stops counting which element types keep a subtree from being
/// skipped. Diagnostic: it costs a hash lookup per such element.
#[doc(hidden)]
pub fn set_clean_skip_blocker_probe(enabled: bool) {
    PROBE.with(|probe| probe.set(enabled));
}

/// The element types that were walked without promising a repeatable walk, with
/// how many times, most frequent first. Needs [`set_clean_skip_blocker_probe`].
#[doc(hidden)]
pub fn clean_skip_blockers() -> Vec<(&'static str, u64)> {
    let mut blockers: Vec<_> =
        BLOCKERS.with(|blockers| blockers.borrow().iter().map(|(name, count)| (*name, *count)).collect());
    blockers.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
    blockers
}

#[inline]
pub(crate) fn enabled() -> bool {
    ENABLED.with(Cell::get)
}

/// Records that an element was walked that does not promise a repeatable walk.
#[inline]
pub(crate) fn note_impure(name: impl FnOnce() -> &'static str) {
    IMPURE.with(|impure| impure.set(impure.get() + 1));
    if PROBE.with(Cell::get) {
        BLOCKERS.with(|blockers| *blockers.borrow_mut().entry(name()).or_default() += 1);
    }
}

/// Counts why a subtree was not skipped or recorded, when the probe is on.
#[inline]
fn miss(reason: &'static str) {
    if PROBE.with(Cell::get) {
        BLOCKERS.with(|blockers| *blockers.borrow_mut().entry(reason).or_default() += 1);
    }
}

/// Records that a context was asked for the pointer position.
#[inline]
pub(crate) fn note_cursor_read() {
    CURSOR_READS.with(|reads| reads.set(reads.get() + 1));
}

/// Records that something asked for another frame while a subtree was walked. A
/// walk that did is not at rest, whatever its elements promised, so no subtree
/// that contains it is recorded.
#[inline]
#[track_caller]
pub(crate) fn note_frame_requested() {
    note_impure(|| "<frame request>");
    if PROBE.with(Cell::get) {
        // The first frames of the stack that belong to a widget, not to the
        // plumbing that carries a request to the window.
        let stack = std::backtrace::Backtrace::force_capture().to_string();
        let relevant: Vec<&str> = stack
            .lines()
            .filter(|line| line.trim_start().starts_with("at "))
            .filter(|line| {
                !["clean_skip", "window_handle", "aimer_events", "aimer_app.rs", "/rustc/", ".cargo/"]
                    .iter()
                    .any(|plumbing| line.contains(plumbing))
            })
            .take(3)
            .collect();
        REQUESTERS.with(|requesters| {
            *requesters.borrow_mut().entry(relevant.join(" <- ")).or_default() += 1;
        });
    }
}

/// Where frames were requested from while a walk was in progress, most frequent
/// first. Needs [`set_clean_skip_blocker_probe`].
#[doc(hidden)]
pub fn clean_skip_frame_requesters() -> Vec<(String, u64)> {
    let mut requesters: Vec<_> = REQUESTERS.with(|requesters| {
        requesters.borrow().iter().map(|(at, count)| (at.clone(), *count)).collect()
    });
    requesters.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    requesters
}

/// Starts the walk of an element.
#[inline]
pub(crate) fn enter() -> Entry {
    let visits = VISITS.with(|visits| {
        let before = visits.get();
        visits.set(before + 1);
        before
    });
    Entry {
        visits,
        impure: IMPURE.with(Cell::get),
        tree: element_tree_generation(),
        cursor: CURSOR_READS.with(Cell::get),
        transform: aimer_canvas::transform_reads(),
    }
}

#[inline]
fn slot(element: ElementId) -> usize {
    (element.get() as usize) & (SLOTS - 1)
}

/// Why this frame may not leave a subtree alone, if it may not.
#[inline]
fn frame_blocker() -> Option<&'static str> {
    if is_animation_only_pass() {
        Some("skip: an animation-only pass")
    } else if stale_roots_waiting() {
        Some("skip: rebuilt roots are waiting to be reached")
    } else if !has_active_v2_render_tree() {
        Some("skip: no retained tree")
    } else {
        None
    }
}

/// Whether an element whose paint was invalidated and not yet resolved sits at
/// or below `root`.
///
/// A scroll invalidates the paint of the element that scrolled, so some
/// invalidation is pending in every scroll frame; only one inside the subtree
/// matters to it. An invalidated element without a node has no retained paint to
/// be stale.
fn invalidated_within(root: ElementId) -> bool {
    if !paint_invalidations_pending() {
        return false;
    }
    let Some(scope) = current_v2_render_context() else {
        return true;
    };
    let Some(root_node) = scope.node_for_element(root) else {
        return true;
    };
    any_pending_paint_invalidation(|pending| {
        let Some(mut node) = scope.node_for_element(pending) else {
            return false;
        };
        loop {
            if node == root_node {
                return true;
            }
            match scope.tree.parent_of(node) {
                Ok(Some(parent)) => node = parent,
                _ => return false,
            }
        }
    })
}

/// The retained tree's geometry revision, or zero when none is active.
#[inline]
fn geometry_revision_now() -> u64 {
    current_v2_render_context().map_or(0, |scope| scope.tree.geometry_revision())
}

/// What to do with a subtree that is a candidate for being left alone.
pub(crate) enum Decision {
    /// Walk it.
    Walk,
    /// Leave it alone: its last walk is still what a walk would produce.
    Skip,
    /// It could be left alone, but verify mode is on: walk it and compare.
    Verify,
}

/// Decides whether the subtree of `element` can be left alone: whether its last
/// walk is still what a walk would produce now.
pub(crate) fn try_skip(element: ElementId, ctx: &BuildContext) -> Decision {
    let generations = (
        element_tree_generation(),
        layout_invalidation_generation(),
        rebuild_invalidation_generation(),
    );
    let visits = RECORDS.with(|records| {
        let mut records = records.borrow_mut();
        // The slot first: almost every element has no record, and that is
        // answered without asking anything else.
        let Some(record) = records.get_mut(slot(element)).and_then(Option::as_mut) else {
            miss("skip: no record");
            return None;
        };
        if record.element != element.get() {
            miss("skip: the slot holds another element");
            return None;
        }
        if (record.tree, record.layout, record.rebuild) != generations {
            miss("skip: a generation moved");
            return None;
        }
        if let Some(reason) = frame_blocker() {
            miss(reason);
            return None;
        }
        let traversal = traversal_counts().0;
        if record.traversal + 1 != traversal {
            miss("skip: the record is not from the last traversal");
            return None;
        }
        if !record.key.matches(ctx, record.reads) {
            miss("skip: the context differs");
            return None;
        }
        // What the tree shows of the subtree is only asked while its geometry has
        // moved since it was last known to match: in a frame where nothing moved
        // (a hover, an idle frame) the answer cannot have changed.
        let geometry = geometry_revision_now();
        if geometry != record.geometry {
            let cursor = ctx.cursor_pos_unobserved();
            if !record.tree_key.matches(&TreeKey::of(ctx, element, record.reads), (cursor.x, cursor.y)) {
                miss("skip: the retained tree shows it differently");
                return None;
            }
            record.geometry = geometry;
        }
        if invalidated_within(element) {
            miss("skip: a paint invalidation is pending inside");
            return None;
        }
        // The subtree now counts as walked by this traversal, so the next one may
        // leave it alone in turn.
        record.traversal = traversal;
        Some((record.visits, record.reads))
    });
    let Some((skipped, reads)) = visits else {
        return Decision::Walk;
    };
    if VERIFY.with(Cell::get) {
        // Walked after all, so the walk counts its own visits.
        STATS.with(|stats| {
            let mut current = stats.get();
            current.verified += 1;
            stats.set(current);
        });
        return Decision::Verify;
    }
    VISITS.with(|visits| visits.set(visits.get() + skipped));
    // Left alone, the subtree does not read anything, but whatever contains it still
    // depends on what it would have read: a container's record that left out the
    // pointer or the transform because its children were skipped would be taken for
    // one that does not need them.
    if reads.cursor {
        note_cursor_read();
    }
    if reads.transform {
        aimer_canvas::note_transform_read();
    }
    STATS.with(|stats| {
        let mut current = stats.get();
        current.skips += 1;
        stats.set(current);
    });
    Decision::Skip
}

/// What the retained tree and the elements hold for one element of a subtree.
#[derive(PartialEq)]
struct Held {
    element: ElementId,
    name: &'static str,
    /// The cached interaction rectangle, as bits.
    rect: Option<[u32; 4]>,
    node: Option<RenderNodeState>,
}

/// Everything verify mode compares, for a subtree, in tree order.
pub(crate) struct Fingerprint(Vec<Held>);

/// Reads what the retained tree and the elements hold for `root` and everything
/// below it.
pub(crate) fn fingerprint(root: &dyn Element) -> Fingerprint {
    let scope = current_v2_render_context();
    let mut held = Vec::new();
    fn collect(
        element: &dyn Element,
        scope: Option<&V2RenderContext>,
        held: &mut Vec<Held>,
    ) {
        let id = element.id();
        held.push(Held {
            element: id,
            name: element.debug_name(),
            rect: element.pos_start_end().map(|(start, end)| {
                [start.x.to_bits(), start.y.to_bits(), end.x.to_bits(), end.y.to_bits()]
            }),
            node: scope
                .and_then(|scope| {
                    scope
                        .node_for_element(id)
                        .and_then(|node| scope.tree.node_state(node).ok())
                }),
        });
        element.visit_retained_v2_children(&mut |_, child| collect(child, scope, held));
    }
    collect(root, scope.as_ref(), &mut held);
    Fingerprint(held)
}

/// The fields of a node's state that differ, as text.
fn node_differences(before: &RenderNodeState, after: &RenderNodeState) -> String {
    let mut changed = Vec::new();
    macro_rules! field {
        ($name:ident) => {
            if before.$name != after.$name {
                changed.push(format!(
                    "{} {:?} -> {:?}",
                    stringify!($name),
                    before.$name,
                    after.$name
                ));
            }
        };
    }
    field!(bounds);
    field!(clip);
    field!(clip_radius);
    field!(escapes_clip);
    field!(animation_clip);
    field!(presentation_transform);
    field!(presentation_opacity);
    field!(transform);
    field!(opacity);
    field!(paint_source);
    field!(list_revision);
    field!(list_dirty);
    field!(list_stale);
    changed.join("; ")
}

/// Compares what a walk of `root` left with what was held before it, and records
/// what the skip would have lost.
pub(crate) fn finish_verification(root: &dyn Element, before: Fingerprint) {
    let after = fingerprint(root);
    if before.0 == after.0 {
        return;
    }
    let mut found = Vec::new();
    if before.0.len() != after.0.len() {
        found.push(format!(
            "the subtree changed shape: {} elements before the walk, {} after",
            before.0.len(),
            after.0.len()
        ));
    } else {
        for (was, is) in before.0.iter().zip(&after.0) {
            if was == is {
                continue;
            }
            let what = if was.rect != is.rect {
                format!("interaction rectangle {:?} -> {:?}", was.rect, is.rect)
            } else {
                match (&was.node, &is.node) {
                    (Some(was), Some(is)) => node_differences(was, is),
                    (None, None) => "nothing recorded".to_owned(),
                    _ => "gained or lost its render node".to_owned(),
                }
            };
            found.push(format!("{} {:?}: {what}", is.name, is.element));
        }
    }
    let root_id = root.id();
    VIOLATIONS.with(|violations| {
        let mut violations = violations.borrow_mut();
        for detail in found {
            if violations.len() >= MAX_VIOLATIONS {
                break;
            }
            violations.push(CleanSkipViolation { root: root_id, detail });
        }
    });
}

/// Ends the walk of an element that promised a repeatable walk, recording its
/// subtree if everything below promised it too and it is big enough to matter.
pub(crate) fn leave(element: ElementId, ctx: &BuildContext, entry: Entry) {
    if IMPURE.with(Cell::get) != entry.impure {
        miss("record: something below is not repeatable");
        return;
    }
    let visits = VISITS.with(Cell::get).saturating_sub(entry.visits);
    if visits < MIN_SUBTREE {
        miss("record: too few elements");
        return;
    }
    if element_tree_generation() != entry.tree {
        miss("record: the tree changed during the walk");
        return;
    }
    let reads = Reads {
        cursor: CURSOR_READS.with(Cell::get) != entry.cursor,
        // With position independence off, every subtree counts as having read it.
        transform: aimer_canvas::transform_reads() != entry.transform
            || !RELATIVE_WINDOW.with(Cell::get),
    };
    // One word per walk is all a subtree that keeps moving costs: a record is built
    // only when the context is the one the previous walk was given.
    let hash = context_hash(ctx, reads);
    let steady = SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        if seen.is_empty() {
            seen.resize(SLOTS, Seen { element: 0, hash: 0 });
        }
        let held = &mut seen[slot(element)];
        let steady = held.element == element.get() && held.hash == hash;
        *held = Seen { element: element.get(), hash };
        steady
    });
    if !steady {
        miss("record: not steady");
        return;
    }
    // Asked only of a subtree that holds still, which is rare next to one that moves.
    if is_animation_only_pass() || !has_active_v2_render_tree() {
        miss("record: no full traversal");
        return;
    }
    let key = ContextKey::of(ctx, reads);
    let tree_key = TreeKey::of(ctx, element, reads);
    let geometry = geometry_revision_now();
    RECORDS.with(|records| {
        let mut records = records.borrow_mut();
        if records.is_empty() {
            records.resize(SLOTS, None);
        }
        records[slot(element)] = Some(Record {
            element: element.get(),
            traversal: traversal_counts().0,
            tree: element_tree_generation(),
            layout: layout_invalidation_generation(),
            rebuild: rebuild_invalidation_generation(),
            visits,
            key,
            tree_key,
            geometry,
            reads,
        });
    });
    STATS.with(|stats| {
        let mut current = stats.get();
        current.records += 1;
        stats.set(current);
    });
}
