//! Frames that exist only to advance compositor animations.
//!
//! A compositor animation changes a retained node's transform, opacity or clip
//! and nothing a build, layout or paint pass reads. Walking the whole element
//! tree to sample it therefore costs O(tree) for work that is O(animations).
//!
//! The first walk after anything else changed is a full one. Each element that
//! applies an active compositor animation during it leaves a copy of the
//! context its parents gave it here. While nothing else asks for a frame and no
//! generation has moved, the frame loop calls [`animation_only_targets`] and
//! [`run_animation_only_pass`] instead of walking the tree: each registered
//! element is revisited alone, with that context, and its subtree is left
//! alone.

use std::cell::{Cell, RefCell};

use aimer_attribute::position::Vec2d;
use aimer_canvas::FrameCanvas;
use aimer_cupid::utilities::{IdBuildHasher, Mat3};

use super::*;
use crate::components::context::BuildContextSnapshot;

/// Why an element is in the registry.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reason {
    /// It animates: every pass revisits it while it keeps asking for frames.
    Animating,
    /// It was the root of a rebuilt subtree: a pass revisits it only on the
    /// frame that rebuilds it again.
    Rebuilt,
}

struct Registration {
    reason: Reason,
    snapshot: BuildContextSnapshot,
    /// The canvas translation the element's ancestors had accumulated. Widgets
    /// measure their absolute position from it while updating, so a revisit
    /// without the ancestors has to start from the same place.
    translation: (f32, f32),
    /// Whether a revisit also walks the element's subtree. A compositor
    /// animation and a retained-child presentation leave the subtree untouched;
    /// an element that rebuilds its child each frame needs the subtree updated.
    walk_subtree: bool,
    /// Whether the element applied an active animation during the traversal in
    /// progress. An element that did not has finished or left the tree.
    touched: bool,
    /// Set on a rebuilt root when a full walk begins, cleared when the walk
    /// reaches it and records its context afresh. A root the walk never reaches
    /// has left the tree, and is dropped when the walk ends.
    stale: bool,
    /// Orders rebuilt roots by when they were last rebuilt, so the least
    /// recently rebuilt can be dropped when too many are kept.
    serial: u64,
}

/// How many rebuilt roots are kept. Each costs a context copy on every full walk.
const MAX_REBUILT_ROOTS: usize = 64;

aimer_tls::thread_local! {
    static REGISTRY: RefCell<HashMap<ElementId, Registration, IdBuildHasher>> =
        RefCell::new(HashMap::default());
    static PASS_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static FRAME_ELIGIBLE: Cell<bool> = const { Cell::new(false) };
    static WALKS: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
    /// The roots of the subtrees rebuilt since the previous walk or pass.
    static REBUILT_ROOTS: RefCell<Vec<ElementId>> = const { RefCell::new(Vec::new()) };
    static HAS_REBUILT_ROOTS: Cell<bool> = const { Cell::new(false) };
    /// Whether some rebuilt root is waiting for the full walk in progress to
    /// refresh it.
    static HAS_STALE_ROOTS: Cell<bool> = const { Cell::new(false) };
    static NEXT_SERIAL: Cell<u64> = const { Cell::new(0) };
    /// How many revisited elements whose subtree is being walked enclose the
    /// element being drawn.
    static SUBTREE_WALKS: Cell<u32> = const { Cell::new(0) };
}

fn next_serial() -> u64 {
    NEXT_SERIAL.with(|serial| {
        let value = serial.get();
        serial.set(value + 1);
        value
    })
}

/// How many whole-tree walks and animation-only passes this thread has run, in
/// that order. For diagnostics: a window that animates but keeps counting
/// walks has something asking for ordinary frames.
#[doc(hidden)]
pub fn traversal_counts() -> (u64, u64) {
    WALKS.with(Cell::get)
}

/// Tells the widget layer whether the frame about to be drawn was requested by
/// compositor animations alone.
///
/// The frame loop sets this from the reason the frame was scheduled and clears
/// it when the frame ends. It is only a candidate: the drawing code still
/// checks that nothing else changed.
#[doc(hidden)]
pub fn set_animation_only_frame(eligible: bool) {
    FRAME_ELIGIBLE.with(|flag| flag.set(eligible));
}

/// Whether the frame loop marked the current frame as animation-only.
#[doc(hidden)]
#[inline]
pub fn is_animation_only_frame() -> bool {
    FRAME_ELIGIBLE.with(Cell::get)
}

/// Whether the traversal in progress is an animation-only pass.
#[inline]
pub(crate) fn is_animation_only_pass() -> bool {
    PASS_ACTIVE.with(Cell::get)
}

/// Records that `element` applied an active compositor animation.
///
/// During a full traversal the context is copied so a later pass can revisit
/// the element alone. During an animation-only pass the copy already exists, so
/// the element only confirms that it is still animating.
///
/// Returns `false`, and registers nothing, when the element cannot be revisited
/// alone: its ancestors left something other than a plain translation on the
/// canvas, which the pass does not reproduce. The caller must then keep the
/// next frame a full one.
pub(crate) fn register_animating_element(
    element: ElementId,
    ctx: &BuildContext,
    walk_subtree: bool,
) -> bool {
    let in_pass = is_animation_only_pass();
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        if in_pass && let Some(existing) = registry.get_mut(&element) {
            existing.touched = true;
            existing.reason = Reason::Animating;
            existing.walk_subtree |= walk_subtree;
            return true;
        }
        let Some(translation) = reproducible_translation(ctx) else {
            registry.remove(&element);
            return false;
        };
        registry.insert(
            element,
            Registration {
                reason: Reason::Animating,
                snapshot: ctx.snapshot(),
                translation,
                walk_subtree,
                touched: true,
                stale: false,
                serial: next_serial(),
            },
        );
        true
    })
}

/// The canvas translation a lone revisit can reproduce, if the canvas holds a
/// plain translation. Anything else (a scale, a rotation) is not reproduced.
fn reproducible_translation(ctx: &BuildContext) -> Option<(f32, f32)> {
    let translation = ctx.canvas.get_transform_translation();
    (ctx.canvas.get_transform() == Mat3::translate(translation.0, translation.1))
        .then_some(translation)
}

/// Tells the widget layer which subtrees were rebuilt since the previous walk
/// or pass.
///
/// The frame loop sets this before it walks the tree or runs a pass and clears
/// it afterwards. While it is set, the first element updated for each of these
/// roots records the context it was given, so that the next rebuild of the same
/// root can be handled by revisiting it alone.
#[doc(hidden)]
pub fn set_frame_rebuilt_roots(roots: &[ElementId]) {
    REBUILT_ROOTS.with(|rebuilt| {
        let mut rebuilt = rebuilt.borrow_mut();
        rebuilt.clear();
        rebuilt.extend_from_slice(roots);
    });
    HAS_REBUILT_ROOTS.with(|flag| flag.set(!roots.is_empty()));
}

/// Whether `element` is the root of a subtree rebuilt for the current frame.
#[inline]
fn is_rebuilt_root(element: ElementId) -> bool {
    HAS_REBUILT_ROOTS.with(Cell::get) && REBUILT_ROOTS.with(|rebuilt| rebuilt.borrow().contains(&element))
}

/// Records the context of `element` if it is the root of a subtree rebuilt for
/// this frame, and refreshes it if a rebuilt root recorded earlier is waiting
/// for this walk. Called as each element is updated; costs two flag reads for
/// every element of a frame that has neither.
#[inline]
pub(crate) fn capture_rebuilt_root(element: ElementId, ctx: &BuildContext) {
    let rebuilt = HAS_REBUILT_ROOTS.with(Cell::get) && is_rebuilt_root(element);
    if !rebuilt && !HAS_STALE_ROOTS.with(Cell::get) {
        return;
    }
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        match registry.get_mut(&element) {
            Some(existing) if existing.reason == Reason::Rebuilt => {
                if rebuilt {
                    existing.serial = next_serial();
                }
                if existing.stale {
                    // A full walk reached it: what it was given this time is the
                    // context to revisit it with from now on.
                    match reproducible_translation(ctx) {
                        Some(translation) => {
                            existing.snapshot = ctx.snapshot();
                            existing.translation = translation;
                            existing.stale = false;
                        }
                        None => {
                            registry.remove(&element);
                        }
                    }
                }
            }
            // An animator keeps what it has.
            Some(_) => {}
            None if rebuilt => {
                if let Some(translation) = reproducible_translation(ctx) {
                    if registry
                        .values()
                        .filter(|registration| registration.reason == Reason::Rebuilt)
                        .count()
                        >= MAX_REBUILT_ROOTS
                        && let Some(oldest) = registry
                            .iter()
                            .filter(|(_, registration)| registration.reason == Reason::Rebuilt)
                            .min_by_key(|(_, registration)| registration.serial)
                            .map(|(id, _)| *id)
                    {
                        registry.remove(&oldest);
                    }
                    registry.insert(
                        element,
                        Registration {
                            reason: Reason::Rebuilt,
                            snapshot: ctx.snapshot(),
                            translation,
                            walk_subtree: true,
                            touched: true,
                            stale: false,
                            serial: next_serial(),
                        },
                    );
                }
            }
            None => {}
        }
        if registry.values().all(|registration| !registration.stale) {
            HAS_STALE_ROOTS.with(|flag| flag.set(false));
        }
    });
}

/// Asks for the next frame on behalf of the element being updated, and lets
/// that frame revisit the element alone.
///
/// This is how an element that animates without the compositor opts in to
/// animation-only frames. Call it from the element's own `update`,
/// `sync_local_v2_state` or `draw_local_v2_compatibility` while it still has
/// frames to produce, in place of `request_animation_frame`.
///
/// By calling it the element promises two things. Its per-frame work is
/// confined to the element itself: the state it advances and the presentation
/// it publishes for its render node. And nothing below it needs a visit to stay
/// current, because an animation-only frame skips its subtree. Anything below it
/// that animates must ask for its own frames, and an ordinary request keeps the
/// frame a full one.
///
/// Outside an element's update there is no element to revisit, and a context
/// that cannot be reproduced cannot be revisited; both fall back to an ordinary
/// request.
pub fn request_isolated_animation_frame(ctx: &BuildContext) {
    request_isolated(ctx, false);
}

/// Like [`request_isolated_animation_frame`], for an element that replaces its
/// own child every frame.
///
/// The next animation-only frame revisits the element *and its subtree*, since
/// the subtree is what changed, and still leaves the rest of the tree alone.
/// The element promises that what it changes stays inside its own subtree. A
/// change that does reach outside it, or that gives the subtree a different
/// shape or size, is noticed when the render tree is synchronized and sends the
/// frame back to the full walk.
pub fn request_isolated_subtree_animation_frame(ctx: &BuildContext) {
    request_isolated(ctx, true);
}

fn request_isolated(ctx: &BuildContext, walk_subtree: bool) {
    let owner = DRAW_INVALIDATION_OWNER.with(Cell::get);
    match owner {
        Some(element) if register_animating_element(element, ctx, walk_subtree) => {
            aimer_events::window::request_scoped_frame();
        }
        _ => aimer_events::window::request_animation_frame(),
    }
}

/// Marks the subtree of the element being drawn as walked, for as long as the
/// guard lives.
pub(crate) struct SubtreeWalk;

impl Drop for SubtreeWalk {
    fn drop(&mut self) {
        SUBTREE_WALKS.with(|walks| walks.set(walks.get() - 1));
    }
}

/// Starts walking the subtree of `element` if an animation-only pass revisits
/// it as an element that replaced its subtree.
///
/// Everything below such an element is new or changed, however deep, so each
/// descendant has to be drawn too; asking every one of them whether the pass may
/// skip it, as if it had been revisited alone, stopped the walk one level below
/// the element and left whatever was deeper showing its previous frame.
#[inline]
pub(crate) fn enter_subtree_walk(element: ElementId) -> Option<SubtreeWalk> {
    if !is_animation_only_pass() {
        return None;
    }
    let walks = is_rebuilt_root(element)
        || REGISTRY.with(|registry| {
            registry
                .borrow()
                .get(&element)
                .is_some_and(|registration| registration.walk_subtree)
        });
    walks.then(|| {
        SUBTREE_WALKS.with(|walks| walks.set(walks.get() + 1));
        SubtreeWalk
    })
}

/// Whether an animation-only pass may leave `element`'s subtree alone.
#[inline]
pub(crate) fn pass_skips_subtree(element: ElementId) -> bool {
    is_animation_only_pass()
        && SUBTREE_WALKS.with(Cell::get) == 0
        && !is_rebuilt_root(element)
        && REGISTRY.with(|registry| {
            registry
                .borrow()
                .get(&element)
                .is_none_or(|registration| !registration.walk_subtree)
        })
}

/// Called as a full traversal begins: forgets the elements that animate, so only
/// those that animate during it are left afterwards, and marks every recorded
/// rebuilt root stale, to be refreshed when the traversal reaches it.
pub(crate) fn begin_full_traversal() {
    WALKS.with(|walks| walks.set((walks.get().0 + 1, walks.get().1)));
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        if registry.is_empty() {
            return;
        }
        registry.retain(|_, registration| registration.reason == Reason::Rebuilt);
        for registration in registry.values_mut() {
            registration.stale = true;
        }
        HAS_STALE_ROOTS.with(|flag| flag.set(!registry.is_empty()));
    });
}

/// Called as a full traversal ends: drops the rebuilt roots it did not reach.
pub(crate) fn finish_full_traversal() {
    if !HAS_STALE_ROOTS.with(Cell::get) {
        return;
    }
    REGISTRY.with(|registry| registry.borrow_mut().retain(|_, registration| !registration.stale));
    HAS_STALE_ROOTS.with(|flag| flag.set(false));
}

/// Resolves the elements an animation-only pass has to revisit: everything that
/// registered an active animation, and the root of each subtree rebuilt for this
/// frame (`rebuilt`).
///
/// Returns `None` when the frame cannot be a pass: a rebuilt root has no
/// recorded context (its first rebuild, or one that a full walk did not reach),
/// or `resolve` cannot find an element. The caller must then walk the whole
/// tree. An empty list is a pass with nothing to do, such as the frame that
/// follows a state update whose rebuild the frame before it already handled.
/// The elements are returned in creation order so the pass is deterministic.
#[doc(hidden)]
pub fn animation_only_targets<'a>(
    rebuilt: &[ElementId],
    mut resolve: impl FnMut(ElementId) -> Option<&'a dyn Element>,
) -> Option<Vec<(ElementId, &'a dyn Element)>> {
    let mut ids: Vec<ElementId> = REGISTRY.with(|registry| {
        let registry = registry.borrow();
        if rebuilt.iter().any(|root| !registry.contains_key(root)) {
            return None;
        }
        Some(
            registry
                .iter()
                .filter(|(_, registration)| registration.reason == Reason::Animating)
                .map(|(id, _)| *id)
                .chain(rebuilt.iter().copied())
                .collect(),
        )
    })?;
    ids.sort_unstable_by_key(|id| id.get());
    ids.dedup();
    let mut targets = Vec::with_capacity(ids.len());
    for id in ids {
        targets.push((id, resolve(id)?));
    }
    Some(targets)
}

/// Runs `callback` as an animation-only pass over the retained tree.
///
/// Unlike [`with_v2_render_tree_context`] this keeps the registrations made by
/// the last full traversal, and marks the traversal so the elements visited
/// skip their subtrees.
#[doc(hidden)]
pub fn with_v2_animation_only_context<R>(
    tree: RenderTree,
    element_nodes: Rc<ElementNodeMap>,
    callback: impl FnOnce() -> R,
) -> R {
    forget_unmapped_compositor_animations(&element_nodes);
    V2_RENDER_CONTEXT_STACK.with(|stack| {
        stack.borrow_mut().push(V2RenderContext(Rc::new(V2RenderScope {
            tree,
            element_nodes,
            prepared_root: Cell::new(None),
        })));
    });
    let _context = V2RenderContextGuard;
    WALKS.with(|walks| walks.set((walks.get().0, walks.get().1 + 1)));
    let previous = PASS_ACTIVE.with(|flag| flag.replace(true));
    let _pass = PassGuard(previous);
    callback()
}

struct PassGuard(bool);

impl Drop for PassGuard {
    fn drop(&mut self) {
        PASS_ACTIVE.with(|flag| flag.set(self.0));
    }
}

/// Revisits each target alone, then forgets those that stopped animating.
///
/// Must run inside [`with_v2_animation_only_context`].
#[doc(hidden)]
pub fn run_animation_only_pass(targets: &[(ElementId, &dyn Element)], canvas: &FrameCanvas<'_>) {
    REGISTRY.with(|registry| {
        for registration in registry.borrow_mut().values_mut() {
            registration.touched = false;
        }
    });
    for (id, element) in targets {
        // The registry is released before the element runs: it registers itself
        // again from inside `update`.
        let revisit = REGISTRY.with(|registry| {
            registry.borrow().get(id).map(|registration| {
                (
                    registration.snapshot.restore(canvas.clone()),
                    registration.translation,
                )
            })
        });
        if let Some((ctx, (x, y))) = revisit {
            ctx.canvas.save();
            ctx.canvas.translate(Vec2d { x, y });
            element.update(&ctx);
            ctx.canvas.restore();
        }
    }
    // An animator that did not ask for another frame has finished. A rebuilt
    // root stays until the next full walk, because the next rebuild of the same
    // root can use what was recorded for it.
    REGISTRY.with(|registry| {
        registry
            .borrow_mut()
            .retain(|_, registration| registration.touched || registration.reason == Reason::Rebuilt)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_registry_is_a_pass_with_nothing_to_do() {
        begin_full_traversal();
        finish_full_traversal();
        assert_eq!(animation_only_targets(&[], |_| None).map(|targets| targets.len()), Some(0));
    }

    #[test]
    fn a_rebuilt_root_with_no_recorded_context_cannot_be_a_pass() {
        begin_full_traversal();
        finish_full_traversal();
        let unknown = ElementId::next();
        assert!(animation_only_targets(&[unknown], |_| None).is_none());
    }

    #[test]
    fn the_frame_marker_is_off_until_the_frame_loop_sets_it() {
        set_animation_only_frame(false);
        assert!(!is_animation_only_frame());
        set_animation_only_frame(true);
        assert!(is_animation_only_frame());
        set_animation_only_frame(false);
    }
}
