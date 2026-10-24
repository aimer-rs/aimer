//! Targets without a compositor have no animation-only frames: every frame
//! walks the whole tree. The functions keep the shape of `animation_only.rs`
//! so callers do not need their own configuration checks.

use aimer_canvas::FrameCanvas;

use super::*;

#[doc(hidden)]
pub fn set_animation_only_frame(_eligible: bool) {}

#[doc(hidden)]
#[inline]
pub fn is_animation_only_frame() -> bool {
    false
}

#[inline]
pub(crate) fn is_animation_only_pass() -> bool {
    false
}

pub(crate) fn register_animating_element(
    _element: ElementId,
    _ctx: &BuildContext,
    _walk_subtree: bool,
) -> bool {
    false
}

#[inline]
pub(crate) fn pass_skips_subtree(_element: ElementId) -> bool {
    false
}

pub(crate) struct SubtreeWalk;

#[inline]
pub(crate) fn enter_subtree_walk(_element: ElementId) -> Option<SubtreeWalk> {
    None
}

pub(crate) fn begin_full_traversal() {}

#[doc(hidden)]
pub fn animation_only_targets<'a>(
    _rebuilt: &[ElementId],
    _resolve: impl FnMut(ElementId) -> Option<&'a dyn Element>,
) -> Option<Vec<(ElementId, &'a dyn Element)>> {
    None
}

#[doc(hidden)]
pub fn with_v2_animation_only_context<R>(
    _tree: RenderTree,
    _element_nodes: Rc<ElementNodeMap>,
    callback: impl FnOnce() -> R,
) -> R {
    callback()
}

#[doc(hidden)]
pub fn run_animation_only_pass(_targets: &[(ElementId, &dyn Element)], _canvas: &FrameCanvas<'_>) {}

/// Without a compositor every animation frame is a full one.
pub fn request_isolated_animation_frame(_ctx: &BuildContext) {
    aimer_events::window::request_animation_frame();
}

#[doc(hidden)]
pub fn traversal_counts() -> (u64, u64) {
    (0, 0)
}

/// Without a compositor every animation frame is a full one.
pub fn request_isolated_subtree_animation_frame(_ctx: &BuildContext) {
    aimer_events::window::request_animation_frame();
}

#[doc(hidden)]
pub fn set_frame_rebuilt_roots(_roots: &[ElementId]) {}

#[inline]
pub(crate) fn capture_rebuilt_root(_element: ElementId, _ctx: &BuildContext) {}

pub(crate) fn finish_full_traversal() {}
