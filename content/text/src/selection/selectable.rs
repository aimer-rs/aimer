use std::cell::RefCell;
use std::ops::Range;
use std::rc::{Rc, Weak};

use aimer_attribute::{Bounds, Vec2d};
use aimer_widget::InteractionBounds;
use aimer_widget::base::{BuildContext, Color, WindowHandle};

use crate::TextSource;
use crate::selection::session::{SelectionSession, SelectionSlot};
use crate::selection::{SelectionRegion, TextHitRegion, text_offset_at};

use crate::TextAccessibilitySnapshot;
use aimer_cupid::text_layout::{TextInteractionLayout, TextWritingMode};
use aimer_cupid::utilities::Mat3;

/// The geometry side of a selection participant.
///
/// A [`crate::selection::SelectionSlot`] owns the participant's *text*, so this
/// trait covers only what needs the laid-out paragraph: where the participant
/// painted, and which offset a position maps to.
///
/// Implementors must be reachable through an [`Rc`] that the element keeps
/// alive, because elements themselves are stored inline in their owner and may
/// move.
pub(crate) trait Selectable {
    /// Absolute logical bounds from the last frame, or `None` if never drawn.
    fn painted_bounds(&self) -> Option<Bounds>;

    /// The top-left corner of those bounds, or the origin if never drawn.
    ///
    /// A touch press is a finger resting on a *glyph*, so it records this and
    /// every later judgement compares it against the same corner: a participant
    /// that has since moved was scrolled out from under the finger, whether or
    /// not the finger itself ever reported a move. See
    /// [`TouchHoldGate::forget_if_content_moved`](crate::selection::touch_hold::TouchHoldGate::forget_if_content_moved).
    #[inline]
    fn painted_origin(&self) -> Vec2d {
        self.painted_bounds().map_or(Vec2d::ZERO, |bounds| Vec2d {
            x: bounds.x,
            y: bounds.y,
        })
    }

    /// The offset nearest `(x, y)`, given in absolute logical coordinates.
    fn offset_at(&self, x: f32, y: f32) -> Option<usize>;

    /// The zero-width caret rectangle at `offset`, in absolute logical
    /// coordinates.
    ///
    /// This is where a selection handle attaches, so it must be the *edge* of
    /// the grapheme rather than its box: the leading edge of the grapheme that
    /// starts there, or the trailing edge of the last one when the offset sits
    /// at the very end of the text.
    fn caret_rect(&self, offset: usize) -> Option<Bounds>;
}

/// Shared, per-frame geometry of one selectable text element.
///
/// The element writes into it while drawing and reads from it while handling
/// events; the session reads it through a [`Weak`] reference, which keeps the
/// element free to move without invalidating the registration.
///
/// # Examples
///
/// ```ignore
/// let geometry = Rc::new(TextGeometry::new(window));
/// geometry.bounds.save(scale, x, y, width, height);
/// assert!(geometry.painted_bounds().is_some());
/// ```
pub(crate) struct TextGeometry {
    /// Absolute logical bounds of the last painted frame. Once the render tree
    /// supplies this element's node, every absolute rectangle below is shifted
    /// by how far the node has moved since that paint ([`Self::shift`]).
    pub bounds: InteractionBounds,
    /// Per-grapheme hit regions of the last painted frame, in absolute logical
    /// coordinates.
    pub regions: RefCell<Vec<TextHitRegion>>,
    interaction: RefCell<Option<InteractionSnapshot>>,
    pub retained_v2_link_hover: RefCell<Option<TextSource>>,
    pub retained_v2_selection: RefCell<Option<Range<usize>>>,
    window: WindowHandle,
}

struct InteractionSnapshot {
    layout: Rc<TextInteractionLayout>,
    transform: Mat3,
    scale: f32,
}

impl TextGeometry {
    /// Creates empty geometry that repaints through `window`.
    #[inline]
    pub fn new(window: WindowHandle) -> Self {
        Self {
            bounds: InteractionBounds::new(),
            regions: RefCell::new(Vec::new()),
            interaction: RefCell::new(None),
            retained_v2_link_hover: RefCell::new(None),
            retained_v2_selection: RefCell::new(None),
            window,
        }
    }

    /// Replaces the fallback hit regions supplied by a custom participant.
    pub(crate) fn set_regions(&self, regions: impl IntoIterator<Item = SelectionRegion>) {
        *self.regions.borrow_mut() = regions.into_iter().map(TextHitRegion::from).collect();
        *self.interaction.borrow_mut() = None;
    }

    /// Replaces the optional shared Aimer layout used by selection queries.
    pub fn set_interaction_layout(
        &self,
        layout: Option<TextInteractionLayout>,
        transform: Mat3,
        scale: f32,
    ) {
        self.set_shared_interaction_layout(layout.map(Rc::new), transform, scale);
    }

    /// Replaces the interaction layout without cloning its immutable geometry.
    ///
    /// Prepared paragraphs retain this data alongside their paint geometry, so
    /// a frame only needs another reference to install it in selection state.
    pub(crate) fn set_shared_interaction_layout(
        &self,
        layout: Option<Rc<TextInteractionLayout>>,
        transform: Mat3,
        scale: f32,
    ) {
        *self.interaction.borrow_mut() = layout
            .filter(|_| transform.inverse_transform_point(0.0, 0.0).is_some())
            .map(|layout| InteractionSnapshot {
                layout,
                transform,
                scale,
            });
    }

    /// Returns a portable text accessibility snapshot of the last painted
    /// Aimer layout, if one exists.
    pub(crate) fn accessibility_snapshot(
        &self,
        selection: Option<Range<usize>>,
    ) -> Option<TextAccessibilitySnapshot> {
        let snapshot = self.interaction.borrow();
        let snapshot = snapshot.as_ref()?;
        TextAccessibilitySnapshot::from_layout(
            (*snapshot.layout).clone(),
            snapshot.transform,
            snapshot.scale,
        )
        .map(|snapshot| snapshot.with_selection(selection))
    }

    /// Saves the logical axis-aligned bounds of a paragraph after its canvas
    /// transform has been applied.
    pub fn save_painted_bounds(&self, scale: f32, transform: Mat3, width: f32, height: f32) {
        if transform.inverse_transform_point(0.0, 0.0).is_none() {
            self.bounds.set_bounds(Bounds::new(0.0, 0.0, 0.0, 0.0));
            return;
        }
        let bounds = transformed_rect_bounds(&transform, 0.0, 0.0, width, height);
        let scale = valid_device_scale(scale);
        self.bounds.set_bounds(Bounds::new(
            bounds.x / scale,
            bounds.y / scale,
            bounds.width / scale,
            bounds.height / scale,
        ));
    }

    /// How far the paragraph's render node has moved since it was painted, in
    /// logical pixels. A scroll moves the node without repainting, and the
    /// geometry saved at paint time is absolute.
    #[inline]
    fn shift(&self) -> (f32, f32) {
        self.bounds.origin_shift()
    }

    /// The origin this paragraph paints at, in logical pixels: its render
    /// node's once the tree supplies one, otherwise the canvas translation the
    /// caller read (`canvas_x`/`canvas_y`, device pixels).
    pub(crate) fn origin(&self, canvas_x: f32, canvas_y: f32, scale: f32) -> Vec2d {
        match self
            .bounds
            .is_adopted()
            .then(|| self.bounds.get_bounds())
            .flatten()
        {
            Some(bounds) => Vec2d {
                x: bounds.x,
                y: bounds.y,
            },
            None => crate::selection::touch_hold::frame_origin(canvas_x, canvas_y, scale),
        }
    }

    /// The painted bounds as a start and end corner, where the node is now.
    pub fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.painted_bounds().map(|bounds| {
            (
                Vec2d {
                    x: bounds.x,
                    y: bounds.y,
                },
                Vec2d {
                    x: bounds.x + bounds.width,
                    y: bounds.y + bounds.height,
                },
            )
        })
    }

    /// Reports whether a pointer is inside the transformed paragraph box.
    #[inline]
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        let (shift_x, shift_y) = self.shift();
        let (x, y) = (x - shift_x, y - shift_y);
        if let Some(snapshot) = self.interaction.borrow().as_ref() {
            let scale = valid_device_scale(snapshot.scale);
            let Some((local_x, local_y)) = snapshot
                .transform
                .inverse_transform_point(x * scale, y * scale)
            else {
                return false;
            };
            let left = snapshot.layout.origin_x;
            // `origin_y` is the first line's baseline, while `metrics.height`
            // covers the full line box. Subtract the ascent and half the line
            // gap to recover that box's top and keep hit testing aligned with
            // the painted selection band.
            let top = match snapshot.layout.writing_mode {
                TextWritingMode::HorizontalTb => {
                    snapshot.layout.origin_y
                        - snapshot.layout.metrics.ascent
                        - snapshot.layout.metrics.line_gap * 0.5
                }
                TextWritingMode::VerticalRl => snapshot.layout.origin_y,
            };
            return local_x >= left
                && local_x <= left + snapshot.layout.metrics.width
                && local_y >= top
                && local_y <= top + snapshot.layout.metrics.height;
        }

        self.bounds
            .canvas_bounds()
            .is_some_and(|bounds| bounds.is_inside(x, y))
    }

    /// Reports whether `(x, y)` lands on real glyph geometry rather than
    /// merely inside the element's bounds.
    ///
    /// This is what separates the I-beam from the default cursor past the end
    /// of a short line.
    pub fn hits_glyph(&self, x: f32, y: f32) -> bool {
        let (shift_x, shift_y) = self.shift();
        let (x, y) = (x - shift_x, y - shift_y);
        if let Some(snapshot) = self.interaction.borrow().as_ref() {
            let scale = valid_device_scale(snapshot.scale);
            let Some((local_x, local_y)) = snapshot
                .transform
                .inverse_transform_point(x * scale, y * scale)
            else {
                return false;
            };
            return snapshot.layout.clusters.iter().any(|cluster| {
                if cluster.text_range.start == cluster.text_range.end
                    || snapshot
                        .layout
                        .text
                        .get(cluster.text_range.clone())
                        .is_some_and(|text| text == "\n" || text == "\r\n")
                {
                    return false;
                }
                if matches!(
                    snapshot.layout.writing_mode,
                    aimer_cupid::text_layout::TextWritingMode::VerticalRl
                ) {
                    let left = cluster.start_x - cluster.height * 0.5;
                    let right = cluster.start_x + cluster.height * 0.5;
                    let top = cluster.start_y.min(cluster.end_y);
                    let bottom = cluster.start_y.max(cluster.end_y);
                    local_x >= left && local_x <= right && local_y >= top && local_y <= bottom
                } else {
                    let left = cluster.start_x.min(cluster.end_x);
                    let right = cluster.start_x.max(cluster.end_x);
                    let top = cluster.y;
                    let bottom = cluster.y + cluster.height;
                    local_x >= left && local_x <= right && local_y >= top && local_y <= bottom
                }
            });
        }

        self.regions.borrow().iter().any(|region| {
            let bounds = region.bounds;
            bounds.x <= x
                && x <= bounds.x + bounds.width
                && bounds.y <= y
                && y <= bounds.y + bounds.height
        })
    }

    /// The window this participant paints into.
    #[inline]
    pub fn window(&self) -> &WindowHandle {
        &self.window
    }
}

impl Selectable for TextGeometry {
    #[inline]
    fn painted_bounds(&self) -> Option<Bounds> {
        let (shift_x, shift_y) = self.shift();
        self.bounds.canvas_bounds().map(|bounds| {
            Bounds::new(
                bounds.x + shift_x,
                bounds.y + shift_y,
                bounds.width,
                bounds.height,
            )
        })
    }

    #[inline]
    fn offset_at(&self, x: f32, y: f32) -> Option<usize> {
        let (shift_x, shift_y) = self.shift();
        let (x, y) = (x - shift_x, y - shift_y);
        if let Some(snapshot) = self.interaction.borrow().as_ref() {
            let scale = valid_device_scale(snapshot.scale);
            let (local_x, local_y) = snapshot
                .transform
                .inverse_transform_point(x * scale, y * scale)?;
            return snapshot.layout.hit_test(local_x, local_y);
        }
        text_offset_at(&self.regions.borrow(), x, y)
    }

    fn caret_rect(&self, offset: usize) -> Option<Bounds> {
        let (shift_x, shift_y) = self.shift();
        self.caret_rect_painted(offset).map(|caret| {
            Bounds::new(
                caret.x + shift_x,
                caret.y + shift_y,
                caret.width,
                caret.height,
            )
        })
    }
}

impl TextGeometry {
    /// The caret rectangle where the paragraph was painted.
    fn caret_rect_painted(&self, offset: usize) -> Option<Bounds> {
        if let Some(snapshot) = self.interaction.borrow().as_ref() {
            let caret = snapshot.layout.caret_geometry(offset)?;
            let bounds = transformed_rect_bounds(
                &snapshot.transform,
                caret.x,
                caret.y,
                caret.width,
                caret.height,
            );
            let scale = valid_device_scale(snapshot.scale);
            return Some(Bounds::new(
                bounds.x / scale,
                bounds.y / scale,
                bounds.width / scale,
                bounds.height / scale,
            ));
        }
        let regions = self.regions.borrow();
        if let Some(region) = regions
            .iter()
            .find(|region| region.source_range.start == offset)
        {
            let bounds = region.bounds;
            return Some(Bounds::new(bounds.x, bounds.y, 0.0, bounds.height));
        }
        let region = regions
            .iter()
            .rfind(|region| region.source_range.end <= offset)?;
        let bounds = region.bounds;
        Some(Bounds::new(
            bounds.x + bounds.width,
            bounds.y,
            0.0,
            bounds.height,
        ))
    }
}

#[inline]
fn valid_device_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > f32::EPSILON {
        scale
    } else {
        1.0
    }
}

fn transformed_rect_bounds(transform: &Mat3, x: f32, y: f32, width: f32, height: f32) -> Bounds {
    let points = [
        transform.transform_point(x, y),
        transform.transform_point(x + width, y),
        transform.transform_point(x, y + height),
        transform.transform_point(x + width, y + height),
    ];
    let (min_x, max_x) = points
        .iter()
        .map(|point| point.0)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    let (min_y, max_y) = points
        .iter()
        .map(|point| point.1)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    Bounds::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

/// The ambient selection session installed by a selection region.
///
/// Text widgets look this state up while building; finding it is what makes
/// them join the region instead of owning a private selection.
pub(crate) struct SelectionScope(pub Rc<SelectionSession>);

/// Arbitrates between selection sessions inside one widget tree.
///
/// Only one session may hold a selection at a time, so claiming clears the
/// previous one. This is what makes clicking a standalone text drop a region's
/// selection, and the other way round.
#[derive(Default)]
pub(crate) struct SelectionCoordinator {
    current: RefCell<Weak<SelectionSession>>,
}

impl SelectionCoordinator {
    /// Grants the selection to `session`, clearing whichever session held it.
    pub fn claim(&self, session: &Rc<SelectionSession>) {
        let previous = self.current.borrow().upgrade();
        if previous
            .as_ref()
            .is_some_and(|previous| Rc::ptr_eq(previous, session))
        {
            return;
        }
        if let Some(previous) = previous {
            previous.clear();
        }
        *self.current.borrow_mut() = Rc::downgrade(session);
    }

    /// The session currently holding the selection, if it is still alive.
    #[cfg(test)]
    pub fn current(&self) -> Option<Rc<SelectionSession>> {
        self.current.borrow().upgrade()
    }
}

/// Returns the tree-wide coordinator, inserting it on first use.
pub(crate) fn selection_coordinator(ctx: &BuildContext) -> Rc<SelectionCoordinator> {
    if let Some(coordinator) = ctx.get_state::<SelectionCoordinator>() {
        return coordinator;
    }
    ctx.insert_state(SelectionCoordinator::default());
    ctx.get_state::<SelectionCoordinator>()
        .expect("selection coordinator was just inserted")
}

/// Everything a selectable text keeps alive to take part in a selection.
///
/// It lives behind one [`std::cell::RefCell`] so a rebuilt element can adopt the
/// whole registration from the element it replaces, which is what keeps a
/// selection alive across a rebuild.
pub(crate) struct SelectionBinding {
    pub geometry: Rc<TextGeometry>,
    pub session: Rc<SelectionSession>,
    pub slot: Rc<SelectionSlot>,
    /// `true` when the element created the session itself, which is the case
    /// outside a selection region.
    pub owns_session: bool,
}

impl SelectionBinding {
    /// Registers `text` with the ambient session of `ctx`, or with a fresh
    /// private one when the element sits outside a selection region.
    pub fn new(
        ctx: &BuildContext,
        text: impl Into<crate::TextSource>,
        fallback_color: Color,
    ) -> Self {
        let window = ctx.window.clone();
        let (session, owns_session) = match ctx.get_state::<SelectionScope>() {
            Some(scope) => (Rc::clone(&scope.0), false),
            None => (
                SelectionSession::new(window.clone(), selection_coordinator(ctx), fallback_color),
                true,
            ),
        };
        let geometry = Rc::new(TextGeometry::new(window));
        let slot = session.register(text, Rc::downgrade(&geometry) as _);
        Self {
            geometry,
            session,
            slot,
            owns_session,
        }
    }

    /// Clones the registration of the element being replaced and refreshes its
    /// text, which clamps any live endpoint inside it.
    pub fn adopt(&self, text: impl Into<crate::TextSource>) -> Self {
        self.slot.set_text(text.into());
        Self {
            geometry: Rc::clone(&self.geometry),
            session: Rc::clone(&self.session),
            slot: Rc::clone(&self.slot),
            owns_session: self.owns_session,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Selectable, TextGeometry};
    use aimer_attribute::Bounds;
    use aimer_cupid::text_layout::{
        ParagraphMetrics, TextCluster, TextInteractionLayout, TextLine, TextWritingMode,
    };
    use aimer_cupid::utilities::Mat3;
    use aimer_widget::base::WindowHandle;
    use winit::dpi::PhysicalSize;

    fn interaction_layout() -> TextInteractionLayout {
        TextInteractionLayout {
            text: "a".to_owned(),
            lines: vec![TextLine {
                text_range: 0..1,
                glyph_range: 0..0,
                baseline: 16.0,
                width: 10.0,
                ascent: 12.0,
                descent: -4.0,
                line_gap: 0.0,
                hard_break: false,
            }],
            clusters: vec![TextCluster {
                text_range: 0..1,
                line_index: 0,
                level: unicode_bidi::Level::ltr(),
                start_x: 0.0,
                end_x: 10.0,
                start_y: 16.0,
                end_y: 16.0,
                y: 4.0,
                height: 20.0,
            }],
            metrics: ParagraphMetrics {
                width: 10.0,
                height: 20.0,
                ascent: 12.0,
                descent: -4.0,
                line_gap: 0.0,
                line_height: 20.0,
                line_count: 1,
            },
            origin_x: 0.0,
            origin_y: 0.0,
            writing_mode: TextWritingMode::HorizontalTb,
        }
    }

    #[test]
    fn aimer_interaction_maps_pointer_and_caret_through_the_canvas_transform() {
        let geometry = TextGeometry::new(WindowHandle::headless(PhysicalSize::new(200, 200), 1.0));
        let transform = Mat3::translate(20.0, 30.0).mul(&Mat3::scale(2.0, 3.0));
        geometry.save_painted_bounds(1.0, transform, 10.0, 20.0);
        geometry.set_interaction_layout(Some(interaction_layout()), transform, 1.0);

        assert_eq!(
            geometry.painted_bounds(),
            Some(Bounds::new(20.0, 30.0, 20.0, 60.0))
        );
        assert!(geometry.contains_point(24.0, 54.0));
        assert!(!geometry.contains_point(41.0, 30.0));
        assert!(geometry.hits_glyph(24.0, 54.0));
        assert_eq!(Selectable::offset_at(&geometry, 24.0, 54.0), Some(0));
        assert_eq!(
            Selectable::caret_rect(&geometry, 1),
            Some(Bounds::new(40.0, 42.0, 0.0, 60.0))
        );
        let accessibility = geometry
            .accessibility_snapshot(Some(0..1))
            .expect("the painted interaction layout is exposed to accessibility");
        assert_eq!(accessibility.text(), "a");
        assert_eq!(accessibility.selection(), Some(0..1));
        assert_eq!(
            accessibility.selected_rects(),
            vec![crate::TextAccessibilitySelectionRect {
                line_index: 0,
                bounds: Bounds::new(20.0, 42.0, 20.0, 60.0),
            }]
        );
    }

    /// A scroll moves the paragraph's node without painting it again. Pointer
    /// queries and the geometry they report must follow the node, not stay where
    /// the paragraph was last painted.
    #[test]
    fn queries_follow_the_render_node_after_it_moves_without_a_repaint() {
        use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};

        let geometry = TextGeometry::new(WindowHandle::headless(PhysicalSize::new(200, 200), 1.0));
        let transform = Mat3::translate(20.0, 100.0);
        geometry.save_painted_bounds(1.0, transform, 10.0, 20.0);
        geometry.set_interaction_layout(Some(interaction_layout()), transform, 1.0);
        let tree = RenderTree::new();
        let node = tree.add_root(Rect::new(20.0, 100.0, 10.0, 20.0)).unwrap();
        geometry.bounds.adopt(aimer_widget::InteractionSource::new(
            tree.clone(),
            node,
            (10.0, 20.0),
        ));
        assert!(geometry.contains_point(24.0, 104.0), "where it was painted");

        // The content scrolls 30px up.
        tree.set_bounds(node, Rect::new(20.0, 70.0, 10.0, 20.0))
            .unwrap();

        assert!(
            !geometry.contains_point(24.0, 104.0),
            "the old place no longer hits"
        );
        assert!(geometry.contains_point(24.0, 74.0));
        assert!(geometry.hits_glyph(24.0, 74.0));
        assert_eq!(Selectable::offset_at(&geometry, 24.0, 74.0), Some(0));
        assert_eq!(
            geometry.painted_bounds(),
            Some(Bounds::new(20.0, 70.0, 10.0, 20.0))
        );
        assert_eq!(
            Selectable::caret_rect(&geometry, 1),
            Some(Bounds::new(30.0, 74.0, 0.0, 20.0))
        );
    }

    #[test]
    fn aimer_interaction_contains_the_line_box_above_a_baseline_origin() {
        let geometry = TextGeometry::new(WindowHandle::headless(PhysicalSize::new(200, 200), 1.0));
        let mut layout = interaction_layout();
        layout.origin_y = layout.lines[0].baseline;
        geometry.save_painted_bounds(1.0, Mat3::identity(), 10.0, 20.0);
        geometry.set_interaction_layout(Some(layout), Mat3::identity(), 1.0);

        assert!(geometry.contains_point(5.0, 4.0));
        assert!(!geometry.contains_point(5.0, 25.0));
    }
}
