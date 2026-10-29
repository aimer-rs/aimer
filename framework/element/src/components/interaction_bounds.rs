//! Hit-test rectangle that moves from the canvas transform stack to the render
//! tree.
//!
//! Elements used to measure their interaction rectangle while updating, by
//! reading the legacy canvas transform. The retained render tree already knows
//! every node's world position, so an element that opts in through
//! [`Drawable::retained_v2_interaction_size`](crate::Drawable) is handed a live
//! [`InteractionSource`] after each tree synchronization. [`InteractionBounds`]
//! owns the transition: until the first adoption it behaves like a plain
//! [`CacheBounds`]; afterwards the render tree answers hit tests, read when they
//! happen rather than copied at synchronization time, because a scroll moves
//! nodes without one. The canvas measurement is then only kept to *compare*
//! against it.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use aimer_attribute::{Bounds, CacheBounds, Vec2d};
use aimer_cupid::draw_cmd_v2::{RenderNodeId, RenderTree};
use aimer_events::pointer::{PointerSource, last_pointer_source, min_touch_target};

/// Largest disagreement, in logical pixels, that still counts as the same box.
const TOLERANCE: f32 = 1.0;

/// An element whose interaction rectangle, measured from the legacy canvas
/// transform stack, differs from the one the render tree holds now.
#[derive(Clone, Debug, PartialEq)]
pub struct InteractionDisagreement {
    /// The element type.
    pub debug_name: &'static str,
    /// `[x, y, width, height]` from the canvas transform, in logical pixels.
    pub canvas: [f32; 4],
    /// `[x, y, width, height]` from the render tree, in logical pixels.
    pub tree: [f32; 4],
}

/// A live reference to the render node that supplies an element's interaction
/// rectangle: the node's current world origin plus a size the element reported.
///
/// This is the message the frame loop hands an element; the element keeps the
/// long-lived state in its [`InteractionBounds`], which updates in place on every
/// later synchronization instead of allocating again.
#[derive(Clone)]
pub struct InteractionSource {
    tree: RenderTree,
    node: RenderNodeId,
    /// Width and height of the rectangle, in logical pixels.
    size: (f32, f32),
    /// Where the rectangle starts relative to the node's world origin, in
    /// logical pixels (a margin, for instance).
    offset: (f32, f32),
}

impl std::fmt::Debug for InteractionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InteractionSource")
            .field("node", &self.node)
            .field("size", &self.size)
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

impl InteractionSource {
    /// Creates a source for `node` with an interaction rectangle of `size`
    /// (logical pixels) anchored at the node's world origin.
    #[inline]
    pub fn new(tree: RenderTree, node: RenderNodeId, size: (f32, f32)) -> Self {
        Self {
            tree,
            node,
            size,
            offset: (0.0, 0.0),
        }
    }

    /// Starts the rectangle `offset` (logical pixels) from the node's origin.
    #[inline]
    pub fn with_offset(mut self, offset: (f32, f32)) -> Self {
        self.offset = offset;
        self
    }
}

/// The adopted source, shared by pointer so an element stays small.
struct SourceCell {
    tree: RefCell<RenderTree>,
    node: Cell<RenderNodeId>,
    size: Cell<(f32, f32)>,
    offset: Cell<(f32, f32)>,
    /// The last world rectangle read, with the tree geometry revision it was
    /// read at. A hit test walks the node's ancestors, and a pointer move tests
    /// many elements, so the walk is repeated only after the tree changed.
    cached: Cell<Option<(u64, Bounds)>>,
}

impl SourceCell {
    /// The node's current world rectangle, or `None` once the node is gone or
    /// its geometry is not finite.
    fn bounds(&self) -> Option<Bounds> {
        let tree = self.tree.borrow();
        let revision = tree.geometry_revision();
        if let Some((cached_at, bounds)) = self.cached.get()
            && cached_at == revision
        {
            return Some(bounds);
        }
        let world = tree.element_bounds(self.node.get()).ok()?;
        let (offset, size) = (self.offset.get(), self.size.get());
        let bounds = Bounds::new(world.x + offset.0, world.y + offset.1, size.0, size.1);
        let finite = [bounds.x, bounds.y, bounds.width, bounds.height]
            .iter()
            .all(|value| value.is_finite());
        if finite {
            self.cached.set(Some((revision, bounds)));
            Some(bounds)
        } else {
            None
        }
    }
}

/// An element's interaction rectangle, in logical pixels.
pub struct InteractionBounds {
    /// The canvas measurement. Answers hit tests until a source is adopted,
    /// then only serves as the comparison value (and the fallback when the
    /// source's node disappears).
    canvas: CacheBounds,
    /// Set once, on the first adoption; later adoptions update it in place.
    /// A `OnceCell` keeps the element at one pointer.
    source: OnceCell<Rc<SourceCell>>,
}

impl std::fmt::Debug for InteractionBounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InteractionBounds")
            .field("adopted", &self.is_adopted())
            .field("canvas", &self.canvas)
            .finish()
    }
}

impl Default for InteractionBounds {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl InteractionBounds {
    /// Creates bounds with nothing measured yet.
    #[inline]
    pub fn new() -> Self {
        Self {
            canvas: CacheBounds::new(),
            source: OnceCell::new(),
        }
    }

    /// Records the rectangle measured from the canvas transform, scaled to
    /// logical pixels.
    #[inline]
    pub fn save(&self, scale: f32, x: f32, y: f32, width: f32, height: f32) {
        self.canvas.save(scale, x, y, width, height);
    }

    /// Records a rectangle already in logical pixels as the canvas measurement.
    #[inline]
    pub fn set_bounds(&self, bounds: Bounds) {
        self.canvas.set_bounds(bounds);
    }

    /// The last rectangle measured from the canvas, in logical pixels.
    #[inline]
    pub fn canvas_bounds(&self) -> Option<Bounds> {
        self.canvas.get_bounds()
    }

    /// How far the render-tree origin has moved from the canvas measurement:
    /// zero until a source is adopted. Geometry an element built at paint time
    /// in absolute coordinates is still correct once shifted by this much.
    pub fn origin_shift(&self) -> (f32, f32) {
        let (Some(cell), Some(canvas)) = (self.source.get(), self.canvas.get_bounds()) else {
            return (0.0, 0.0);
        };
        cell.bounds()
            .map_or((0.0, 0.0), |tree| (tree.x - canvas.x, tree.y - canvas.y))
    }

    /// Forgets the canvas measurement, for an element that has no hit area this
    /// frame (an unsupported transform, for instance). An adopted source is
    /// unaffected.
    #[inline]
    pub fn clear_canvas(&self) {
        self.canvas.clear();
    }

    /// Takes the render tree as the source of the hit area from now on.
    ///
    /// The first adoption allocates the shared cell; later ones update it in
    /// place and only drop the cached rectangle.
    pub fn adopt(&self, source: InteractionSource) {
        if let Some(cell) = self.source.get() {
            if !cell.tree.borrow().same_tree(&source.tree) {
                *cell.tree.borrow_mut() = source.tree;
            }
            cell.node.set(source.node);
            cell.size.set(source.size);
            cell.offset.set(source.offset);
            cell.cached.set(None);
            return;
        }
        let _ = self.source.set(Rc::new(SourceCell {
            tree: RefCell::new(source.tree),
            node: Cell::new(source.node),
            size: Cell::new(source.size),
            offset: Cell::new(source.offset),
            cached: Cell::new(None),
        }));
    }

    /// Whether the render tree supplies the hit area.
    #[inline]
    pub fn is_adopted(&self) -> bool {
        self.source.get().is_some()
    }

    /// The rectangle hit tests use now.
    fn current(&self) -> Option<Bounds> {
        if let Some(cell) = self.source.get()
            && let Some(bounds) = cell.bounds()
        {
            return Some(bounds);
        }
        self.canvas.get_bounds()
    }

    /// The active rectangle, in logical pixels.
    #[inline]
    pub fn get_bounds(&self) -> Option<Bounds> {
        self.current()
    }

    /// Whether `(x, y)` lies inside the active rectangle.
    #[inline]
    pub fn is_inside(&self, x: f32, y: f32) -> bool {
        self.current().is_some_and(|bound| {
            bound.x <= x
                && x <= bound.x + bound.width
                && bound.y <= y
                && y <= bound.y + bound.height
        })
    }

    /// The active rectangle as top-left and bottom-right corners.
    #[inline]
    pub fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.current().map(|bound| {
            (
                Vec2d {
                    x: bound.x,
                    y: bound.y,
                },
                Vec2d {
                    x: bound.x + bound.width,
                    y: bound.y + bound.height,
                },
            )
        })
    }

    /// The active rectangle grown about its centre until each side is at least
    /// [`min_touch_target`], which is what a touch hits.
    ///
    /// A side already that long is left alone, so the result always contains the
    /// active rectangle.
    pub fn touch_bounds(&self) -> Option<Bounds> {
        let bounds = self.current()?;
        let min = min_touch_target();
        let grow_x = ((min - bounds.width) * 0.5).max(0.0);
        let grow_y = ((min - bounds.height) * 0.5).max(0.0);
        Some(Bounds::new(
            bounds.x - grow_x,
            bounds.y - grow_y,
            bounds.width + grow_x * 2.0,
            bounds.height + grow_y * 2.0,
        ))
    }

    /// The touch rectangle as top-left and bottom-right corners.
    #[inline]
    pub fn touch_pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.touch_bounds().map(|bound| {
            (
                Vec2d { x: bound.x, y: bound.y },
                Vec2d {
                    x: bound.x + bound.width,
                    y: bound.y + bound.height,
                },
            )
        })
    }

    /// Whether a pointer from `source` at `(x, y)` hits: the touch rectangle for
    /// a finger, the active rectangle for anything else.
    #[inline]
    pub fn is_inside_for(&self, x: f32, y: f32, source: PointerSource) -> bool {
        if self.is_inside(x, y) {
            return true;
        }
        source == PointerSource::Touch && self.touch_bounds().is_some_and(|bound| bound.is_inside(x, y))
    }

    /// The rectangle hit testing should use for the pointer being routed: the
    /// touch rectangle while a finger is, the active rectangle otherwise.
    ///
    /// The routed pointer is the one [`last_pointer_source`] reports, which the
    /// dispatcher sets as each event enters the tree.
    #[inline]
    pub fn hit_pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        if last_pointer_source() == PointerSource::Touch {
            self.touch_pos_start_end()
        } else {
            self.pos_start_end()
        }
    }

    /// How the render tree's rectangle currently differs from the last canvas
    /// measurement, when it differs by more than a pixel.
    pub fn disagreement(&self, debug_name: &'static str) -> Option<InteractionDisagreement> {
        let tree = self.source.get()?.bounds()?;
        let canvas = self.canvas.get_bounds()?;
        let canvas = [canvas.x, canvas.y, canvas.width, canvas.height];
        let tree = [tree.x, tree.y, tree.width, tree.height];
        canvas
            .iter()
            .zip(tree)
            .any(|(canvas, tree)| !((canvas - tree).abs() <= TOLERANCE))
            .then_some(InteractionDisagreement {
                debug_name,
                canvas,
                tree,
            })
    }
}

#[cfg(test)]
mod tests {
    use aimer_cupid::draw_cmd_v2::Rect;

    use super::*;

    fn source_at(x: f32, y: f32, size: (f32, f32)) -> (RenderTree, RenderNodeId, InteractionSource) {
        let tree = RenderTree::new();
        let node = tree.add_root(Rect::new(x, y, 10.0, 10.0)).unwrap();
        let source = InteractionSource::new(tree.clone(), node, size);
        (tree, node, source)
    }

    #[test]
    fn the_canvas_measurement_answers_until_a_source_is_adopted() {
        let bounds = InteractionBounds::new();
        bounds.save(2.0, 20.0, 20.0, 100.0, 100.0);

        assert!(!bounds.is_adopted());
        assert!(bounds.is_inside(15.0, 15.0));
        assert!(!bounds.is_inside(100.0, 100.0));
    }

    #[test]
    fn an_adopted_source_replaces_the_canvas_measurement() {
        let bounds = InteractionBounds::new();
        bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        let (_tree, _node, source) = source_at(200.0, 200.0, (50.0, 50.0));

        bounds.adopt(source);

        assert!(bounds.is_adopted());
        assert!(!bounds.is_inside(10.0, 10.0), "the old area no longer hits");
        assert!(bounds.is_inside(210.0, 210.0));
        assert_eq!(
            bounds.pos_start_end(),
            Some((Vec2d { x: 200.0, y: 200.0 }, Vec2d { x: 250.0, y: 250.0 }))
        );
    }

    #[test]
    fn the_interaction_size_is_independent_of_the_nodes_own_size() {
        let bounds = InteractionBounds::new();
        // The node is 10x10 (a content box); the element hit-tests 40x30.
        let (_tree, _node, source) = source_at(5.0, 5.0, (40.0, 30.0));

        bounds.adopt(source);

        assert!(bounds.is_inside(44.0, 34.0));
        assert!(!bounds.is_inside(46.0, 34.0));
    }

    #[test]
    fn the_rectangle_starts_at_its_offset_from_the_nodes_origin() {
        let bounds = InteractionBounds::new();
        let (tree, node, _) = source_at(100.0, 100.0, (1.0, 1.0));
        // A 10px margin inside the node's origin, 30x20 of hit area.
        bounds.adopt(InteractionSource::new(tree, node, (30.0, 20.0)).with_offset((10.0, 10.0)));

        assert_eq!(
            bounds.pos_start_end(),
            Some((Vec2d { x: 110.0, y: 110.0 }, Vec2d { x: 140.0, y: 130.0 }))
        );
        assert!(!bounds.is_inside(105.0, 105.0), "the margin does not hit");
    }

    fn with_min_touch_target<R>(size: f32, test: impl FnOnce() -> R) -> R {
        let previous = aimer_events::pointer::min_touch_target();
        aimer_events::pointer::set_min_touch_target(size);
        let result = test();
        aimer_events::pointer::set_min_touch_target(previous);
        result
    }

    #[test]
    fn a_small_rectangle_grows_about_its_centre_to_the_touch_target() {
        with_min_touch_target(44.0, || {
            let bounds = InteractionBounds::new();
            bounds.save(1.0, 100.0, 200.0, 20.0, 30.0);

            let grown = bounds.touch_bounds().expect("measured");

            assert_eq!((grown.x, grown.y), (88.0, 193.0));
            assert_eq!((grown.width, grown.height), (44.0, 44.0));
        });
    }

    #[test]
    fn a_rectangle_already_at_the_touch_target_does_not_grow() {
        with_min_touch_target(44.0, || {
            let bounds = InteractionBounds::new();
            bounds.save(1.0, 10.0, 10.0, 200.0, 44.0);

            let grown = bounds.touch_bounds().expect("measured");

            assert_eq!((grown.x, grown.y), (10.0, 10.0));
            assert_eq!((grown.width, grown.height), (200.0, 44.0));
        });
    }

    #[test]
    fn a_turned_off_touch_target_leaves_the_rectangle_alone() {
        with_min_touch_target(0.0, || {
            let bounds = InteractionBounds::new();
            bounds.save(1.0, 100.0, 200.0, 20.0, 30.0);

            assert!(!bounds.is_inside_for(95.0, 215.0, PointerSource::Touch));
            assert_eq!(bounds.touch_pos_start_end(), bounds.pos_start_end());
        });
    }

    #[test]
    fn only_a_touch_hits_the_extra_area() {
        with_min_touch_target(44.0, || {
            let bounds = InteractionBounds::new();
            bounds.save(1.0, 100.0, 200.0, 20.0, 30.0);

            // Ten pixels left of the rectangle, level with its middle.
            assert!(bounds.is_inside_for(95.0, 215.0, PointerSource::Touch));
            assert!(!bounds.is_inside_for(95.0, 215.0, PointerSource::Mouse));
            // The rectangle itself answers to both.
            assert!(bounds.is_inside_for(110.0, 215.0, PointerSource::Mouse));
            // Beyond the grown area, nothing does.
            assert!(!bounds.is_inside_for(80.0, 215.0, PointerSource::Touch));
        });
    }

    #[test]
    fn the_hit_rectangle_follows_the_last_pointer_source() {
        with_min_touch_target(44.0, || {
            let bounds = InteractionBounds::new();
            bounds.save(1.0, 100.0, 200.0, 20.0, 30.0);
            let exact = bounds.pos_start_end();

            aimer_events::pointer::note_pointer_source(PointerSource::Mouse);
            assert_eq!(bounds.hit_pos_start_end(), exact);

            aimer_events::pointer::note_pointer_source(PointerSource::Touch);
            assert_eq!(bounds.hit_pos_start_end(), bounds.touch_pos_start_end());
            assert_ne!(bounds.hit_pos_start_end(), exact);

            aimer_events::pointer::note_pointer_source(PointerSource::Mouse);
        });
    }

    #[test]
    fn a_node_moved_without_a_new_synchronization_moves_the_hit_area() {
        // A scroll updates node geometry in place. The hit area must follow it
        // instead of keeping the rectangle seen at the last synchronization.
        let bounds = InteractionBounds::new();
        let (tree, node, source) = source_at(0.0, 100.0, (50.0, 20.0));
        bounds.adopt(source);
        assert!(bounds.is_inside(10.0, 110.0));

        tree.set_bounds(node, Rect::new(0.0, 40.0, 10.0, 10.0)).unwrap();

        assert!(!bounds.is_inside(10.0, 110.0));
        assert!(bounds.is_inside(10.0, 50.0));
    }

    #[test]
    fn a_canvas_measurement_after_adoption_does_not_move_the_hit_area() {
        let bounds = InteractionBounds::new();
        let (_tree, _node, source) = source_at(200.0, 200.0, (50.0, 50.0));
        bounds.adopt(source);

        bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);

        assert!(!bounds.is_inside(10.0, 10.0));
        assert!(bounds.is_inside(210.0, 210.0));
    }

    #[test]
    fn a_disagreement_is_reported_only_beyond_a_pixel() {
        let bounds = InteractionBounds::new();
        let (tree, node, source) = source_at(0.0, 0.0, (100.0, 100.0));
        bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        assert_eq!(bounds.disagreement("Probe"), None, "nothing adopted yet");
        bounds.adopt(source);

        tree.set_bounds(node, Rect::new(0.5, 0.0, 10.0, 10.0)).unwrap();
        assert_eq!(bounds.disagreement("Probe"), None, "within a pixel");

        tree.set_bounds(node, Rect::new(0.0, 14.0, 10.0, 10.0)).unwrap();
        let reported = bounds.disagreement("Probe").expect("14 px apart");
        assert_eq!(reported.debug_name, "Probe");
        assert_eq!(reported.canvas, [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(reported.tree, [0.0, 14.0, 100.0, 100.0]);
    }

    #[test]
    fn the_origin_shift_is_how_far_the_node_moved_since_the_canvas_painted() {
        let bounds = InteractionBounds::new();
        bounds.save(1.0, 100.0, 100.0, 40.0, 20.0);
        assert_eq!(bounds.origin_shift(), (0.0, 0.0), "nothing adopted yet");
        let (tree, node, _) = source_at(100.0, 100.0, (1.0, 1.0));
        bounds.adopt(InteractionSource::new(tree.clone(), node, (40.0, 20.0)));
        assert_eq!(bounds.origin_shift(), (0.0, 0.0), "node and canvas agree");

        // A scroll moves the node without another paint.
        tree.set_bounds(node, Rect::new(100.0, 70.0, 10.0, 10.0)).unwrap();

        assert_eq!(bounds.origin_shift(), (0.0, -30.0));
        assert_eq!(
            bounds.canvas_bounds(),
            Some(Bounds::new(100.0, 100.0, 40.0, 20.0)),
            "the last painted rectangle is still available"
        );
    }

    #[test]
    fn a_set_rectangle_is_taken_in_logical_pixels_as_is() {
        let bounds = InteractionBounds::new();

        bounds.set_bounds(Bounds::new(3.0, 4.0, 5.0, 6.0));

        assert_eq!(bounds.get_bounds(), Some(Bounds::new(3.0, 4.0, 5.0, 6.0)));
    }

    #[test]
    fn a_vanished_node_falls_back_to_the_canvas_measurement() {
        let bounds = InteractionBounds::new();
        bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        let other = RenderTree::new();
        // A node id from a different tree is unknown to this one.
        let (_tree, node, _) = source_at(0.0, 0.0, (1.0, 1.0));
        bounds.adopt(InteractionSource::new(other, node, (1.0, 1.0)));

        assert!(bounds.is_inside(50.0, 50.0));
    }
}
