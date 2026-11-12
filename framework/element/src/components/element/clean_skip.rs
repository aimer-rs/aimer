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
    /// The whole canvas transform: an element can read its place from it, and a
    /// scale or rotation above it is as much an input as a translation.
    transform: Mat3,
}

impl ContextKey {
    /// Whether `ctx` gives the key this key was made from, without making a key.
    ///
    /// Compares the fields a moving subtree differs in first, and the canvas, whose
    /// queries cost the most, last: most candidates are rejected on the first or
    /// second comparison.
    fn matches(&self, ctx: &BuildContext, read_cursor: bool) -> bool {
        let position = ctx.parent_pos();
        if self.position != [position.x.to_bits(), position.y.to_bits()] {
            return false;
        }
        if self.size != [ctx.parent_size.width.to_bits(), ctx.parent_size.height.to_bits()]
            || self.scale != ctx.scale.to_bits()
        {
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
            return false;
        }
        let visible = ctx
            .visible_rect()
            .map(|(x, y, width, height)| [x.to_bits(), y.to_bits(), width.to_bits(), height.to_bits()]);
        if self.visible != visible || self.background != ctx.paint_background().as_u32() {
            return false;
        }
        if read_cursor {
            let cursor = ctx.cursor_pos_unobserved();
            if self.cursor != [cursor.x.to_bits(), cursor.y.to_bits()] {
                return false;
            }
        }
        self.transform == ctx.canvas.get_transform()
    }

    /// `read_cursor` is whether the subtree depends on the pointer; when it does
    /// not, the pointer is left out of the key.
    fn of(ctx: &BuildContext, read_cursor: bool) -> Self {
        let position = ctx.parent_pos();
        // Asked of the field directly so that building a key is not itself a
        // read: it would make every subtree look like it depends on the pointer.
        let cursor = if read_cursor { ctx.cursor_pos_unobserved() } else { Vec2d::default() };
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
            visible: ctx.visible_rect().map(|(x, y, width, height)| {
                [x.to_bits(), y.to_bits(), width.to_bits(), height.to_bits()]
            }),
            background: ctx.paint_background().as_u32(),
            transform: ctx.canvas.get_transform(),
        }
    }
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
#[derive(Clone, Copy, PartialEq)]
struct TreeKey {
    window: Option<VisibleWindow>,
    world: Option<[u32; 4]>,
}

impl TreeKey {
    fn of(ctx: &BuildContext, element: ElementId) -> Self {
        Self {
            window: retained_visible_window(element, ctx.scale),
            world: world_bounds(element),
        }
    }
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
    /// Present once the subtree held still across two walks; a record without
    /// it is never used to skip.
    tree_key: Option<TreeKey>,
    /// Whether the walk read the pointer; if it did not, the key leaves it out.
    cursor_read: bool,
}

/// What an element's walk started from, to tell afterwards what happened below it.
pub(crate) struct Entry {
    visits: u64,
    impure: u64,
    tree: u64,
    cursor: u64,
}

aimer_tls::thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    static VISITS: Cell<u64> = const { Cell::new(0) };
    /// What `VISITS` read when the statistics were last reset.
    static VISITS_BASE: Cell<u64> = const { Cell::new(0) };
    static IMPURE: Cell<u64> = const { Cell::new(0) };
    /// How many times a context has been asked for the pointer.
    static CURSOR_READS: Cell<u64> = const { Cell::new(0) };
    static STATS: Cell<CleanSkipStats> = const { Cell::new(CleanSkipStats { visits: 0, skips: 0, records: 0, verified: 0 }) };
    static RECORDS: RefCell<Vec<Option<Record>>> = const { RefCell::new(Vec::new()) };
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

/// Enables or disables leaving subtrees alone in scroll-only frames.
///
/// On by default. Turning it off makes every frame walk every subtree, which is
/// what differential tests compare against.
#[doc(hidden)]
pub fn set_clean_subtree_skip(enabled: bool) {
    ENABLED.with(|flag| flag.set(enabled));
    if !enabled {
        RECORDS.with(|records| records.borrow_mut().clear());
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
        if !record.key.matches(ctx, record.cursor_read) {
            miss("skip: the context differs");
            return None;
        }
        let Some(tree_key) = record.tree_key else {
            miss("skip: the subtree has not held still yet");
            return None;
        };
        if TreeKey::of(ctx, element) != tree_key {
            miss("skip: the retained tree shows it differently");
            return None;
        }
        if invalidated_within(element) {
            miss("skip: a paint invalidation is pending inside");
            return None;
        }
        // The subtree now counts as walked by this traversal, so the next one may
        // leave it alone in turn.
        record.traversal = traversal;
        Some(record.visits)
    });
    let Some(skipped) = visits else {
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
    if is_animation_only_pass() || !has_active_v2_render_tree() {
        miss("record: no full traversal");
        return;
    }
    let cursor_read = CURSOR_READS.with(Cell::get) != entry.cursor;
    let key = ContextKey::of(ctx, cursor_read);
    RECORDS.with(|records| {
        let mut records = records.borrow_mut();
        if records.is_empty() {
            records.resize(SLOTS, None);
        }
        let slot = &mut records[slot(element)];
        // A subtree given the same context twice in a row is the kind a skip can
        // apply to; only then is it worth asking the tree what it shows of it.
        let steady = slot
            .as_ref()
            .is_some_and(|held| held.element == element.get() && held.key == key);
        *slot = Some(Record {
            element: element.get(),
            traversal: traversal_counts().0,
            tree: element_tree_generation(),
            layout: layout_invalidation_generation(),
            rebuild: rebuild_invalidation_generation(),
            visits,
            key,
            tree_key: steady.then(|| TreeKey::of(ctx, element)),
            cursor_read,
        });
    });
    STATS.with(|stats| {
        let mut current = stats.get();
        current.records += 1;
        stats.set(current);
    });
}
