//! What a container can learn from the retained render tree about which of
//! its children are on screen, and whether skipping them loses anything.
//!
//! A scrollable asks its child to prepare more content than fits in the
//! viewport, so rows are built and recorded a little before they are needed.
//! Once a row is prepared, visiting it again every frame while it sits in that
//! margin only re-confirms what the retained tree already holds. A frame that
//! exists only because the content scrolled cannot have changed any of it, so
//! a container may leave such a row alone until it reaches the screen.

use std::cell::Cell;

use aimer_cupid::draw_cmd_v2::LocalVisibleRegion;

use super::*;

aimer_tls::thread_local! {
    /// Whether the frame being drawn was requested by scrolling alone.
    static SCROLL_ONLY_FRAME: Cell<bool> = const { Cell::new(false) };
    /// Kill switch for diagnostics and differential tests.
    static SETTLED_SKIP_ENABLED: Cell<bool> = const { Cell::new(true) };
}

/// Physical pixels a child may sit beyond the visible window and still be
/// treated as inside it, absorbing rounding in the rounded child offsets.
const WINDOW_MARGIN: f32 = 2.0;

/// Whether an ancestor clip may hide a descendant that escapes clipping.
/// Outside retained rendering, preserve legacy culling. The retained query
/// reads a count and does not walk descendants or clone recording handles.
#[doc(hidden)]
pub fn may_escape_ancestor_clips() -> bool {
    V2_RENDER_CONTEXT_STACK.with(|stack| {
        stack.borrow().last().is_some_and(|context| context.tree.has_clip_escape())
    })
}

/// Marks the frame being drawn as requested by scrolling alone.
///
/// Such a frame cannot carry an animation tick, a state change, a rebuild or a
/// resize: any of those merges the frame request into a full one. The frame
/// loop sets this once the frame is known to be scroll-only and clears it when
/// the frame ends.
#[doc(hidden)]
pub fn set_scroll_only_frame(active: bool) {
    SCROLL_ONLY_FRAME.with(|flag| flag.set(active));
}

/// Enables or disables leaving settled off-screen children alone.
///
/// On by default. Turning it off makes every frame revisit every prepared
/// child, which is what differential tests compare against.
#[doc(hidden)]
pub fn set_settled_offscreen_skip(enabled: bool) {
    SETTLED_SKIP_ENABLED.with(|flag| flag.set(enabled));
}

/// Whether the frame being drawn may leave settled off-screen children alone.
#[doc(hidden)]
#[inline]
pub fn may_skip_settled_offscreen() -> bool {
    SCROLL_ONLY_FRAME.with(Cell::get) && SETTLED_SKIP_ENABLED.with(Cell::get)
}

/// The part of an element, in its own physical-pixel space, that is on screen.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibleWindow {
    /// `x, y, width, height`; `None` when nothing of the element is visible.
    rect: Option<(f32, f32, f32, f32)>,
}

impl VisibleWindow {
    /// Whether a child laid out at `x, y` with the given size overlaps the
    /// window, allowing a small margin for rounded offsets.
    #[inline]
    pub fn intersects(&self, x: f32, y: f32, width: f32, height: f32) -> bool {
        let Some((wx, wy, ww, wh)) = self.rect else {
            return false;
        };
        x < wx + ww + WINDOW_MARGIN
            && x + width > wx - WINDOW_MARGIN
            && y < wy + wh + WINDOW_MARGIN
            && y + height > wy - WINDOW_MARGIN
    }

    /// Whether nothing of the element is on screen.
    #[inline]
    pub fn is_hidden(&self) -> bool {
        self.rect.is_none()
    }
}

/// Reports where anything drawn by `element` or below it can be seen, in the
/// element's own physical-pixel space, as the retained render tree currently
/// clips it.
///
/// The window comes from the clips that apply to the element, not from its own
/// bounds, so children that overflow it are covered. `None` means no clip is
/// known to limit the element, or the tree does not place it by a plain
/// offset: the caller must then treat every child as visible. It is also
/// returned outside a retained draw, for an element without a render node and
/// for a scale that is not a positive finite number.
#[doc(hidden)]
pub fn retained_visible_window(element: ElementId, scale: f32) -> Option<VisibleWindow> {
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let context = current_v2_render_context()?;
    let node = context.node_for_element(element)?;
    match context.tree.local_visible_region(node).ok()? {
        LocalVisibleRegion::Unbounded => None,
        LocalVisibleRegion::Nothing => Some(VisibleWindow { rect: None }),
        LocalVisibleRegion::Within(region) => Some(VisibleWindow {
            rect: Some((
                region.x * scale,
                region.y * scale,
                region.width * scale,
                region.height * scale,
            )),
        }),
    }
}

/// [`retained_visible_window`] for the element whose `update` is running.
///
/// A container's `update` does not know its own identity; the traversal that
/// calls it does. `None` outside such a traversal.
#[doc(hidden)]
pub fn retained_visible_window_of_current(scale: f32) -> Option<VisibleWindow> {
    let element = DRAW_INVALIDATION_OWNER.with(Cell::get)?;
    retained_visible_window(element, scale)
}

/// Whether `element` holds recorded retained paint that is up to date, so a
/// visit while it is off screen would find nothing to do.
///
/// `false` outside a retained draw and for an element without a render node.
#[doc(hidden)]
pub fn retained_element_settled(element: ElementId) -> bool {
    let Some(context) = current_v2_render_context() else {
        return false;
    };
    context
        .node_for_element(element)
        .is_some_and(|node| context.tree.is_settled(node).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A viewport at the window origin whose content is scrolled up by 150
    /// logical pixels, plus one element mapped to each node.
    struct Scene {
        tree: RenderTree,
        nodes: Rc<ElementNodeMap>,
        content: ElementId,
        top: ElementId,
        bottom: ElementId,
    }

    fn scene() -> Scene {
        let tree = RenderTree::new();
        let viewport = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let content_node = tree
            .add_child(viewport, Rect::new(0.0, -150.0, 100.0, 400.0))
            .unwrap();
        tree.set_clip(content_node, Some(Rect::new(0.0, 150.0, 100.0, 100.0)))
            .unwrap();
        let top_node = tree
            .add_child(content_node, Rect::new(0.0, 0.0, 100.0, 40.0))
            .unwrap();
        let bottom_node = tree
            .add_child(content_node, Rect::new(0.0, 300.0, 100.0, 40.0))
            .unwrap();

        let (content, top, bottom) = (ElementId::next(), ElementId::next(), ElementId::next());
        let mut nodes = ElementNodeMap::default();
        nodes.insert(content, content_node);
        nodes.insert(top, top_node);
        nodes.insert(bottom, bottom_node);
        Scene {
            tree,
            nodes: Rc::new(nodes),
            content,
            top,
            bottom,
        }
    }

    fn within<R>(scene: &Scene, callback: impl FnOnce() -> R) -> R {
        with_v2_render_tree_context(scene.tree.clone(), scene.nodes.clone(), None, callback)
    }

    #[test]
    fn the_window_is_the_clip_in_the_elements_own_physical_space() {
        let scene = scene();
        let window = within(&scene, || retained_visible_window(scene.content, 2.0))
            .expect("the content is clipped by the viewport");

        // The viewport shows logical y 0..100 of the world, which is local
        // y 150..250 of the content: physical 300..500.
        assert_eq!(window.rect, Some((0.0, 300.0, 200.0, 200.0)));
        assert!(window.intersects(0.0, 400.0, 200.0, 80.0));
        assert!(window.intersects(0.0, 280.0, 200.0, 40.0));
        assert!(!window.intersects(0.0, 0.0, 200.0, 80.0));
        assert!(!window.intersects(0.0, 600.0, 200.0, 80.0));
        assert!(!window.is_hidden());
    }

    #[test]
    fn a_child_just_outside_the_window_still_counts_as_inside_it() {
        let scene = scene();
        let window = within(&scene, || retained_visible_window(scene.content, 2.0)).unwrap();

        // Touching the edge, or one rounded pixel past it, is not skipped.
        assert!(window.intersects(0.0, 220.0, 200.0, 80.0 + WINDOW_MARGIN - 0.5));
        assert!(!window.intersects(0.0, 220.0, 200.0, 40.0));
    }

    #[test]
    fn an_element_scrolled_out_of_view_sees_its_own_bounds_outside_its_window() {
        let scene = scene();
        let window = within(&scene, || retained_visible_window(scene.bottom, 2.0))
            .expect("the viewport clips the element");

        // The viewport is 300 logical pixels above this element's origin, so
        // the element's own 100x40 box lies below everything that can be seen.
        assert!(!window.is_hidden());
        assert!(!window.intersects(0.0, 0.0, 200.0, 80.0));
    }

    #[test]
    fn an_element_with_nothing_clipped_in_has_a_hidden_window() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let child = tree.add_child(root, Rect::new(50.0, 50.0, 20.0, 20.0)).unwrap();
        tree.set_clip(root, Some(Rect::new(0.0, 0.0, 10.0, 10.0))).unwrap();
        tree.set_clip(child, Some(Rect::new(0.0, 0.0, 10.0, 10.0))).unwrap();
        let element = ElementId::next();
        let mut nodes = ElementNodeMap::default();
        nodes.insert(element, child);

        let window = with_v2_render_tree_context(tree, Rc::new(nodes), None, || {
            retained_visible_window(element, 1.0)
        })
        .expect("the clips are known");

        assert!(window.is_hidden());
        assert!(!window.intersects(0.0, 0.0, 1000.0, 1000.0));
    }

    #[test]
    fn an_element_no_clip_limits_has_no_window() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let element = ElementId::next();
        let mut nodes = ElementNodeMap::default();
        nodes.insert(element, root);

        let window = with_v2_render_tree_context(tree, Rc::new(nodes), None, || {
            retained_visible_window(element, 1.0)
        });

        assert!(window.is_none());
    }

    #[test]
    fn the_window_is_withheld_for_an_unusable_scale() {
        let scene = scene();
        within(&scene, || {
            assert!(retained_visible_window(scene.content, 0.0).is_none());
            assert!(retained_visible_window(scene.content, -1.0).is_none());
            assert!(retained_visible_window(scene.content, f32::NAN).is_none());
            assert!(retained_visible_window(scene.content, f32::INFINITY).is_none());
        });
    }

    #[test]
    fn a_scaled_ancestor_leaves_no_window() {
        let scene = scene();
        let node = scene.nodes[&scene.content];
        scene
            .tree
            .set_transform(node, aimer_cupid::utilities::Mat3::scale(2.0, 2.0))
            .unwrap();

        assert!(within(&scene, || retained_visible_window(scene.top, 2.0)).is_none());
    }

    #[test]
    fn there_is_no_window_without_a_retained_draw_or_a_render_node() {
        let scene = scene();
        assert!(retained_visible_window(scene.content, 2.0).is_none());
        within(&scene, || {
            assert!(retained_visible_window(ElementId::next(), 2.0).is_none());
        });
    }

    #[test]
    fn the_current_element_is_the_one_whose_update_is_running() {
        let scene = scene();
        within(&scene, || {
            assert!(retained_visible_window_of_current(2.0).is_none());
            let _owner = DrawInvalidationOwnerGuard::enter(scene.content);
            assert_eq!(
                retained_visible_window_of_current(2.0),
                retained_visible_window(scene.content, 2.0)
            );
            assert!(retained_visible_window_of_current(2.0).is_some());
        });
    }

    #[test]
    fn settled_means_recorded_and_clean_inside_a_retained_draw() {
        let scene = scene();
        let node = scene.nodes[&scene.top];
        assert!(!within(&scene, || retained_element_settled(scene.top)));

        scene
            .tree
            .set_paint_source(node, RenderPaintSource::LocalV2)
            .unwrap();
        assert!(!within(&scene, || retained_element_settled(scene.top)));

        scene
            .tree
            .context(node)
            .unwrap()
            .begin_recording()
            .unwrap()
            .commit(Vec::new())
            .unwrap();
        assert!(within(&scene, || retained_element_settled(scene.top)));
        assert!(!retained_element_settled(scene.top), "outside a retained draw");
        within(&scene, || {
            assert!(!retained_element_settled(ElementId::next()));
        });

        scene.tree.invalidate_paint(node).unwrap();
        assert!(!within(&scene, || retained_element_settled(scene.top)));
    }

    #[test]
    fn skipping_needs_a_scroll_only_frame_and_the_switch_on() {
        set_scroll_only_frame(false);
        set_settled_offscreen_skip(true);
        assert!(!may_skip_settled_offscreen());

        set_scroll_only_frame(true);
        assert!(may_skip_settled_offscreen());

        set_settled_offscreen_skip(false);
        assert!(!may_skip_settled_offscreen());

        set_settled_offscreen_skip(true);
        set_scroll_only_frame(false);
        assert!(!may_skip_settled_offscreen());
    }
}
