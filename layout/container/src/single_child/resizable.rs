use std::cell::Cell;

use aimer_attribute::dimension::Dimension;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{PointerButton, PointerSource};
use aimer_utils::callback::Callback;
#[cfg(feature = "portable-guest")]
use aimer_utils::callback::CallbackExecutor;
use aimer_utils::cursor::{reset_cursor, set_cursor};
use aimer_widget::InteractionBounds;
use aimer_widget::base::*;
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{PortableBuildContext, PortableBuildError, SourceFingerprint};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, LayoutElement, PointerKey,
    Rebuildable, RequiredChild, VisitorElement, Widget,
};

pub mod band;
pub mod direction;
pub mod handle;

pub use band::ResizeBand;
pub use direction::Direction;
pub use handle::ResizeHandle;

/// The grab band of every edge, in logical pixels, when nothing else is asked
/// for.
const DEFAULT_HANDLE_THICKNESS: f32 = 6.0;

/// The reach, in logical pixels, of the region reported while a handle is
/// hovered: further than any window, yet small enough to stay exact in `f32`.
const WHOLE_SURFACE: f32 = 1.0e7;

/// A single-child box the user resizes by dragging its edges or corners.
///
/// The widget owns its size: [`Resizable::width`] and [`Resizable::height`] give
/// the size it starts at, and every drag replaces it. The size survives rebuilds
/// of the widget, so a parent that rebuilds for unrelated reasons does not snap
/// the box back to where it started.
///
/// A band along each edge is a grab zone: it reaches
/// [`Resizable::handle_thickness`] logical pixels into the box and
/// [`Resizable::handle_outset`] pixels out of it, so the pointer arriving at the
/// border is already on the handle. The four overlaps are corner zones that
/// resize both axes at once. [`Resizable::direction`] picks which of the eight
/// are live — all of them by default. While the pointer is over a live one the
/// window shows the matching resize cursor ([`ResizeHandle::cursor`]), and it
/// returns to the platform default on the way out. The press takes the pointer,
/// so the drag keeps running once the cursor leaves the widget, and it stops on
/// release.
///
/// The parent decides where the widget sits, so a resize changes the size alone:
/// dragging the left or top edge outwards asks for more space, but the box grows
/// from its fixed top-left corner. See [`ResizeHandle::resize`].
///
/// The child is laid out inside the current size and clipped to it, so shrinking
/// the box hides the overflow rather than letting it spill.
///
/// # Examples
///
/// A panel the user can widen, between 120 and 480 logical pixels:
///
/// ```
/// use aimer_container::{Container, Resizable};
///
/// let panel = Resizable::new()
///     .width(240.0)
///     .height(320.0)
///     .min_width(120.0)
///     .max_width(480.0)
///     .child(Container::new().child(aimer_container::ZeroSizedBox));
/// ```
///
/// A panel dragged by its right edge and its two right corners alone:
///
/// ```
/// use aimer_container::{Direction, Resizable, ZeroSizedBox};
///
/// let panel = Resizable::new()
///     .width(240.0)
///     .direction(Direction::RIGHT_EDGES)
///     .child(ZeroSizedBox);
/// ```
///
/// Observing the size while it is dragged:
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_attribute::size::ResolvedSize;
/// use aimer_container::{Resizable, ZeroSizedBox};
///
/// let last = Rc::new(Cell::new(ResolvedSize::default()));
/// let recorded = last.clone();
///
/// let resizable = Resizable::new()
///     .width(200.0)
///     .height(100.0)
///     .on_resize(move |size: ResolvedSize| recorded.set(size))
///     .child(ZeroSizedBox);
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(
    id = "aimer_container::single_child::Resizable",
    validate = validate_portable_resizable
)]
pub struct Resizable<W = RequiredChild> {
    #[portable_optional]
    width: f32,
    #[portable_optional]
    height: f32,
    #[portable_optional]
    min_width: f32,
    #[portable_optional]
    min_height: f32,
    #[portable_optional]
    max_width: f32,
    #[portable_optional]
    max_height: f32,
    #[portable_optional]
    handle_thickness: f32,
    handle_outset: Option<f32>,
    #[portable_optional]
    direction: Direction,
    #[portable_skip]
    on_resize: Callback<ResolvedSize>,
    #[portable_skip]
    on_resize_zone: Callback<Direction>,
    #[portable_child]
    child: W,
}

impl Default for Resizable {
    fn default() -> Self {
        Self::new()
    }
}

impl Resizable {
    /// Creates a zero-sized box with the default grab band, unbounded above and
    /// bounded below by zero.
    ///
    /// Finish the builder with [`Resizable::child`] or
    /// [`Resizable::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            min_width: 0.0,
            min_height: 0.0,
            max_width: f32::MAX,
            max_height: f32::MAX,
            handle_thickness: DEFAULT_HANDLE_THICKNESS,
            handle_outset: None,
            direction: Direction::ALL,
            on_resize: Callback::default(),
            on_resize_zone: Callback::default(),
            child: RequiredChild,
        }
    }
}

impl<W> Resizable<W> {
    /// Sets the width the box starts at, in logical pixels.
    ///
    /// Only the first build uses it: afterwards the size the user dragged the
    /// box to wins, so this is a starting point rather than a constraint.
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets the height the box starts at, in logical pixels.
    ///
    /// Only the first build uses it; see [`Resizable::width`].
    #[inline]
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Sets the smallest width a drag may reach, in logical pixels.
    ///
    /// The default is zero.
    #[inline]
    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width;
        self
    }

    /// Sets the smallest height a drag may reach, in logical pixels.
    ///
    /// The default is zero.
    #[inline]
    pub fn min_height(mut self, min_height: f32) -> Self {
        self.min_height = min_height;
        self
    }

    /// Sets the largest width a drag may reach, in logical pixels.
    ///
    /// The default is [`f32::INFINITY`], which lets a drag grow the box as far
    /// as the pointer goes.
    #[inline]
    pub fn max_width(mut self, max_width: f32) -> Self {
        self.max_width = max_width;
        self
    }

    /// Sets the largest height a drag may reach, in logical pixels.
    ///
    /// The default is [`f32::INFINITY`]; see [`Resizable::max_width`].
    #[inline]
    pub fn max_height(mut self, max_height: f32) -> Self {
        self.max_height = max_height;
        self
    }

    /// Sets how far the grab band along each edge reaches *into* the box, in
    /// logical pixels.
    ///
    /// The default is six. Zero or less leaves the band with its outer half
    /// alone; a box with neither reach cannot be resized at all, and every
    /// pointer event goes to the child.
    #[inline]
    pub fn handle_thickness(mut self, handle_thickness: f32) -> Self {
        self.handle_thickness = handle_thickness;
        self
    }

    /// Sets how far the grab band along each edge reaches *out of* the box, in
    /// logical pixels.
    ///
    /// A window edge can be grabbed from just outside it, and this is the same
    /// idea: the pointer approaching the border shows the resize cursor as it
    /// arrives rather than a pixel after it has crossed. The default is
    /// [`Resizable::handle_thickness`], which puts an equally wide band either
    /// side of the border; pass zero for a box that answers inside its own
    /// bounds alone.
    ///
    /// The box asks for pointer events over this outer band, so a neighbour it
    /// overlaps only ever sees them where no handle is live — keep the reach at
    /// a few pixels in a dense layout.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::{Resizable, ZeroSizedBox};
    ///
    /// // A wide grab band inside, a narrow one outside.
    /// let panel = Resizable::new()
    ///     .handle_thickness(10.0)
    ///     .handle_outset(3.0)
    ///     .child(ZeroSizedBox);
    /// ```
    #[inline]
    pub fn handle_outset(mut self, handle_outset: f32) -> Self {
        self.handle_outset = Some(handle_outset);
        self
    }

    /// Sets which of the eight sides the user may drag.
    ///
    /// [`Direction`] is a set of bit flags, so any combination of the four edges
    /// and four corners can be live at once: `Direction::RIGHT |
    /// Direction::BOTTOM_RIGHT` gives a panel a right edge and one corner, and
    /// [`Direction::NONE`] freezes the box altogether. The default is
    /// [`Direction::ALL`].
    ///
    /// A side left out is not a handle: the pointer over its band gets the
    /// child's cursor and the child's events. A corner left out does not cut a
    /// hole in the edges it overlaps — the nearer live edge answers there
    /// instead.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::{Direction, Resizable, ZeroSizedBox};
    ///
    /// // Width only, from either side.
    /// let horizontal = Resizable::new()
    ///     .direction(Direction::LEFT | Direction::RIGHT)
    ///     .child(ZeroSizedBox);
    /// ```
    #[inline]
    pub fn direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    /// Sets the callback that reports the new size, in logical pixels.
    ///
    /// It runs on every drag step that actually changes the size — not once the
    /// drag ends — so a size held elsewhere stays in step with what is on
    /// screen. A drag that only pushes further past a limit changes nothing and
    /// reports nothing.
    #[inline]
    pub fn on_resize(mut self, on_resize: impl Into<Callback<ResolvedSize>>) -> Self {
        self.on_resize = on_resize.into();
        self
    }

    /// Sets the callback that reports which grab zone the pointer is over.
    ///
    /// The zone is the [`Direction`] of the handle under the pointer — a single
    /// bit, since the pointer is over one side at a time — and
    /// [`Direction::NONE`] once it is over the child or has left the box. It is
    /// the same answer the cursor is drawn from, so it tells an application
    /// exactly when the box would resize and along which side, which is what a
    /// highlighted edge or a hint in a status bar needs.
    ///
    /// It runs only when the zone *changes*, not on every move, and a drag holds
    /// the zone it started on wherever the pointer wanders — the same handle keeps
    /// resizing the box, so reporting anything else would be a lie. Only a mouse
    /// hovers, so a touch never reports a zone until it presses one.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use aimer_container::{Direction, Resizable, ZeroSizedBox};
    ///
    /// let zone = Rc::new(Cell::new(Direction::NONE));
    /// let reported = zone.clone();
    ///
    /// let panel = Resizable::new()
    ///     .width(200.0)
    ///     .on_resize_zone(move |zone: Direction| reported.set(zone))
    ///     .child(ZeroSizedBox);
    ///
    /// // Nothing is hovered yet.
    /// assert_eq!(zone.get(), Direction::NONE);
    /// ```
    #[inline]
    pub fn on_resize_zone(mut self, on_resize_zone: impl Into<Callback<Direction>>) -> Self {
        self.on_resize_zone = on_resize_zone.into();
        self
    }

    /// Supplies the child and returns a statically typed widget.
    #[inline]
    pub fn child<C: Widget>(self, child: C) -> Resizable<C> {
        Resizable {
            width: self.width,
            height: self.height,
            min_width: self.min_width,
            min_height: self.min_height,
            max_width: self.max_width,
            max_height: self.max_height,
            handle_thickness: self.handle_thickness,
            handle_outset: self.handle_outset,
            direction: self.direction,
            on_resize: self.on_resize,
            on_resize_zone: self.on_resize_zone,
            child,
        }
    }

    /// Supplies the child and erases the completed widget's concrete type.
    ///
    /// Exactly equivalent to [`Resizable::child`] followed by [`Widget::boxed`].
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget + 'static> Widget for Resizable<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let min = ResolvedSize {
            width: self.min_width.max(0.0),
            height: self.min_height.max(0.0),
        };
        let max = ResolvedSize {
            width: self.max_width.max(min.width),
            height: self.max_height.max(min.height),
        };

        RawResizable {
            size: Cell::new(clamp_size(
                ResolvedSize {
                    width: self.width,
                    height: self.height,
                },
                min,
                max,
            )),
            min,
            max,
            band: ResizeBand::new(
                self.handle_thickness,
                self.handle_outset.unwrap_or(self.handle_thickness),
            ),
            direction: self.direction,
            on_resize: self.on_resize,
            on_resize_zone: self.on_resize_zone,
            bounds: InteractionBounds::new(),
            active: Cell::new(None),
            grab: Cell::new(Grab::IDLE),
            hovered: Cell::new(None),
            child: self.child.to_element(ctx),
        }
        .boxed()
    }
}

#[cfg(feature = "portable-guest")]
fn validate_portable_resizable<W>(
    resizable: &Resizable<W>,
    _ctx: &PortableBuildContext,
    source: SourceFingerprint,
) -> Result<(), PortableBuildError> {
    if resizable.on_resize.raw().is_some() {
        return Err(PortableBuildError::UnsupportedProperty {
            widget: "Resizable",
            property: "on_resize",
            source,
        });
    }
    if resizable.on_resize_zone.raw().is_some() {
        return Err(PortableBuildError::UnsupportedProperty {
            widget: "Resizable",
            property: "on_resize_zone",
            source,
        });
    }
    Ok(())
}

/// Where a drag started: the pointer position and the size at that moment.
///
/// Keeping the size the drag *started* from — rather than accumulating each
/// step's delta — is what makes a drag that runs into a limit and comes back
/// track the pointer again instead of drifting away from it.
#[derive(Debug, Clone, Copy)]
struct Grab {
    origin: Vec2d,
    size: ResolvedSize,
}

impl Grab {
    const IDLE: Self = Self {
        origin: Vec2d { x: 0.0, y: 0.0 },
        size: ResolvedSize {
            width: 0.0,
            height: 0.0,
        },
    };
}

/// The element behind [`Resizable`], which owns the size the user dragged to.
pub struct RawResizable<E: Element> {
    size: Cell<ResolvedSize>,
    min: ResolvedSize,
    max: ResolvedSize,
    band: ResizeBand,
    direction: Direction,
    on_resize: Callback<ResolvedSize>,
    on_resize_zone: Callback<Direction>,
    bounds: InteractionBounds,
    active: Cell<Option<ResizeHandle>>,
    grab: Cell<Grab>,
    hovered: Cell<Option<ResizeHandle>>,
    child: E,
}

impl<E: Element> RawResizable<E> {
    /// The handle under `point`, in logical pixels, or `None` when the point is
    /// the child's.
    #[inline]
    fn handle_at(&self, point: Vec2d) -> Option<ResizeHandle> {
        let bounds = self.bounds.get_bounds();
        let handle = bounds
            .and_then(|bounds| ResizeHandle::hit_band(bounds, point, self.band, self.direction));
        aimer_utils::pointer_trace!(
            "resizable handle_at ({:.1}, {:.1}) bounds {:?} -> {:?}",
            point.x,
            point.y,
            bounds,
            handle
        );
        handle
    }

    /// Shows the cursor `handle` asks for, and restores the platform default
    /// once no handle is hovered any more.
    ///
    /// The resize shape is asked for again on **every** move over a handle,
    /// rather than once when the handle under the pointer changes. The cursor is
    /// window state shared with everybody: the application restores the platform
    /// default after every pointer move no element consumed, and a move that
    /// lands outside this element is never offered to it, so a record of "the
    /// shape I last asked for" is stale the moment the pointer steps out of the
    /// box. Trusting such a record left the box hovered with a plain arrow —
    /// coming back onto the very same edge was not a change, so nothing was asked
    /// for again.
    ///
    /// Answers whether this element now owns a resize cursor. The caller must
    /// **consume** the pointer move it acted on when it does, for the same
    /// reason: an unconsumed move restores the default and would wipe the shape
    /// asked for here before the next frame is drawn.
    #[inline]
    fn show_cursor(&self, handle: Option<ResizeHandle>) -> bool {
        match handle {
            Some(handle) => {
                self.enter_zone(Some(handle));
                set_cursor(handle.cursor());
                true
            }
            None => {
                // Only on the way out: the pointer over the child asks for
                // nothing, and the shape this element put up is its own to take
                // down.
                if self.hovered.get().is_some() {
                    self.enter_zone(None);
                    reset_cursor();
                }
                false
            }
        }
    }

    /// Records the handle the pointer is over and reports the zone whenever it
    /// is a different one.
    ///
    /// Unlike the cursor — window state anybody may overwrite, hence asked for
    /// again on every move — the zone is this element's own knowledge, so the
    /// record is trustworthy and the report is a transition: entering a side,
    /// crossing to the next, and leaving for [`Direction::NONE`].
    #[inline]
    fn enter_zone(&self, handle: Option<ResizeHandle>) {
        if self.hovered.get() == handle {
            return;
        }
        aimer_utils::pointer_trace!("resize zone {:?} -> {:?}", self.hovered.get(), handle);
        self.hovered.set(handle);
        self.on_resize_zone
            .call(handle.map_or(Direction::NONE, ResizeHandle::direction));
    }

    /// Applies `size` and reports it, answering whether anything changed.
    #[inline]
    fn apply(&self, size: ResolvedSize) -> bool {
        let clamped = clamp_size(size, self.min, self.max);
        if clamped == self.size.get() {
            return false;
        }
        self.size.set(clamped);
        // The row this sits in measured it at the old width and cached that.
        aimer_widget::notify_layout_changed();
        self.on_resize.call(clamped);
        true
    }

    /// Ends the drag in progress, if there is one, and drops the resize cursor.
    #[inline]
    fn cancel_drag(&self) {
        self.active.set(None);
        let _ = self.show_cursor(None);
    }

    /// The current size in physical pixels.
    #[inline]
    fn scaled_size(&self, scale: f32) -> ResolvedSize {
        let size = self.size.get();
        ResolvedSize {
            width: size.width * scale,
            height: size.height * scale,
        }
    }

    /// The size the box occupies, never larger than the space its parent handed
    /// it.
    ///
    /// A stored [`f32::MAX`] — the "fill the parent" sentinel — therefore
    /// resolves to the parent's bounded constraint instead of overflowing into
    /// an unbounded child, which would let a flex child inside consume
    /// `f32::MAX` and push every following sibling off-screen.
    #[inline]
    fn effective_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let size = self.scaled_size(ctx.scale);
        ResolvedSize {
            width: size.width.min(ctx.box_constraint.max_width),
            height: size.height.min(ctx.box_constraint.max_height),
        }
    }
}

/// `size` held within `min` and `max`.
///
/// Written as `max` then `min` rather than [`f32::clamp`], which panics when the
/// two limits cross — a builder mistake should pin the size, not the process.
#[inline]
fn clamp_size(size: ResolvedSize, min: ResolvedSize, max: ResolvedSize) -> ResolvedSize {
    ResolvedSize {
        width: size.width.max(min.width).min(max.width),
        height: size.height.max(min.height).min(max.height),
    }
}

impl<E: Element> VisitorElement for RawResizable<E> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }

    fn debug_name(&self) -> &'static str {
        "Resizable"
    }
}

impl<E: Element + 'static> Rebuildable for RawResizable<E> {
    #[inline]
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Keeps the size the user dragged to, and any drag still in flight.
    ///
    /// The size is runtime state the widget never carried: the widget only knows
    /// the size the box *started* at, so a rebuild triggered by anything else —
    /// a sibling's `set_state`, a theme change, a frame of an animation — would
    /// otherwise throw the drag away and snap the box back.
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|value| value.downcast_ref::<Self>())
        else {
            return;
        };

        self.size
            .set(clamp_size(old.size.get(), self.min, self.max));
        self.active.set(old.active.get());
        self.grab.set(old.grab.get());
        self.hovered.set(old.hovered.get());
    }
}

impl<E: Element> EventElement for RawResizable<E> {
    /// Resizing changes handling, not the single-child structural shape.
    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerDown(info) => {
                if self.active.get().is_some() || info.button != PointerButton::Primary {
                    return EventResult::ignored();
                }
                let Some(handle) = self.handle_at(info.pos) else {
                    return EventResult::ignored();
                };

                self.active.set(Some(handle));
                self.grab.set(Grab {
                    origin: info.pos,
                    size: self.size.get(),
                });
                if info.source == PointerSource::Mouse {
                    let _ = self.show_cursor(Some(handle));
                }

                EventResult::consumed().with_pointer_capture(PointerKey::new(info.source, info.id))
            }
            ElementEvent::PointerMove(info) => match self.active.get() {
                Some(handle) => {
                    let grab = self.grab.get();
                    let delta = Vec2d {
                        x: info.pos.x - grab.origin.x,
                        y: info.pos.y - grab.origin.y,
                    };

                    // A drag keeps the shape of the handle it started on wherever
                    // the pointer wanders, so it is asked for again here too.
                    if info.source == PointerSource::Mouse {
                        let _ = self.show_cursor(Some(handle));
                    }

                    let result = EventResult::consumed();
                    if self.apply(handle.resize(grab.size, delta)) {
                        result.with_redraw()
                    } else {
                        result
                    }
                }
                None => {
                    if info.source != PointerSource::Mouse {
                        return EventResult::ignored();
                    }
                    // A hovered handle claims the move: an unconsumed one makes
                    // the application restore the platform default cursor, which
                    // would undo the resize shape just asked for.
                    if self.show_cursor(self.handle_at(info.pos)) {
                        EventResult::consumed()
                    } else {
                        EventResult::ignored()
                    }
                }
            },
            ElementEvent::PointerUp(info) => {
                if self.active.take().is_none() {
                    return EventResult::ignored();
                }
                if info.source == PointerSource::Mouse {
                    let _ = self.show_cursor(self.handle_at(info.pos));
                }

                EventResult::consumed().with_pointer_release(PointerKey::new(info.source, info.id))
            }
            ElementEvent::PointerExited(PointerSource::Mouse, _) => {
                aimer_utils::pointer_trace!(
                    "resizable received an exit, zone {:?}",
                    self.hovered.get()
                );
                let _ = self.show_cursor(None);
                EventResult::ignored()
            }
            ElementEvent::Cancel => {
                self.cancel_drag();
                EventResult::ignored()
            }
            _ => EventResult::ignored(),
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }
}

impl<E: Element> LayoutElement for RawResizable<E> {
    fn size(&self) -> Option<Size> {
        let size = self.size.get();
        Some(Size {
            width: Dimension::Px(size.width),
            height: Dimension::Px(size.height),
        })
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.effective_size(ctx)
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let size = self.effective_size(ctx);
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        size
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.size()
    }

    /// The box grown by the outer reach of its grab band, or the whole surface
    /// while a handle is hovered.
    ///
    /// This is the region the framework offers pointer events in, and the outer
    /// half of every band lies past the border, so reporting the box alone would
    /// leave that half unreachable: the cursor could only change once the pointer
    /// had crossed inside. Everything the element does outside the band it still
    /// ignores, so a neighbour underneath keeps its events.
    ///
    /// A hovered handle owns two pieces of shared state — the window cursor and
    /// the zone reported to the application — and the move that lets go of it is
    /// the very move that lands *outside* this region, which the element would
    /// never be offered. Reporting the whole surface hands it that one move,
    /// after which the hover is gone and the box is a box again: the exception
    /// lasts exactly as long as the state that needs it, and the extra work is
    /// one hit test on a move the element then ignores.
    ///
    /// The surface is an explicit, very large rectangle rather than "no region".
    /// A wrapper that widens its own bounds by what its child reports — a
    /// decorated `Container` around a sidebar, say — can only widen them by a
    /// rectangle: no answer reads as "nothing to add", so the wrapper fell back to
    /// its own box and dropped the press that followed the hover at the outer half
    /// of the band, which is why the handle only worked on the border itself.
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        if self.hovered.get().is_some() {
            return Some((
                Vec2d {
                    x: -WHOLE_SURFACE,
                    y: -WHOLE_SURFACE,
                },
                Vec2d {
                    x: WHOLE_SURFACE,
                    y: WHOLE_SURFACE,
                },
            ));
        }
        let bounds = self.band.grow(self.bounds.get_bounds()?);
        Some((
            Vec2d {
                x: bounds.x,
                y: bounds.y,
            },
            Vec2d {
                x: bounds.x + bounds.width,
                y: bounds.y + bounds.height,
            },
        ))
    }
}

impl<E: Element> Drawable for RawResizable<E> {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        let size = self.effective_size(ctx);
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && size.width.is_finite()
            && size.height.is_finite()
            && size.width >= 0.0
            && size.height >= 0.0
            && size.width <= 1_000_000.0
            && size.height <= 1_000_000.0
    }

    fn retained_clip(&self, ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd_v2::Rect> {
        if !self.can_paint_local_v2(ctx) {
            return None;
        }
        let size = self.effective_size(ctx);
        Some(aimer_cupid::draw_cmd_v2::Rect::new(
            0.0,
            0.0,
            size.width / ctx.scale,
            size.height / ctx.scale,
        ))
    }

    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        _child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        if !self.can_paint_local_v2(ctx) {
            return None;
        }
        let size = self.effective_size(ctx);
        let mut child_ctx = ctx.clone();
        child_ctx.box_constraint.max_width = size.width;
        child_ctx.box_constraint.max_height = size.height;
        child_ctx.parent_size = size;
        Some(child_ctx)
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.effective_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("Resizable")
    }

    fn update(&self, ctx: &BuildContext) {
        let size = self.effective_size(ctx);

        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);

        ctx.canvas.save();

        let mut child_ctx = ctx.clone();
        child_ctx.box_constraint.max_width = size.width;
        child_ctx.box_constraint.max_height = size.height;
        child_ctx.parent_size = size;

        if ctx.is_rect_visible(0.0, 0.0, size.width, size.height) {
            self.child.update(&child_ctx);
        }

        ctx.canvas.restore();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use aimer_events::pointer::PointerInfo;
    use aimer_utils::cursor::{
        Cursor, CursorIcon, restore_thread_cursor_handler, set_thread_cursor_handler,
    };
    use aimer_widget::{CaptureRequest, dispatch_event};

    use super::*;

    struct StubChild;

    impl VisitorElement for StubChild {
        fn debug_name(&self) -> &'static str {
            "StubChild"
        }
    }

    impl EventElement for StubChild {
        fn event_tree_role(&self) -> aimer_widget::EventTreeRole {
            aimer_widget::EventTreeRole::Transparent
        }
    }
    impl LayoutElement for StubChild {}
    impl Rebuildable for StubChild {}
    impl Drawable for StubChild {
        fn update(&self, _ctx: &BuildContext) {}
    }

    struct DrawProbe {
        draws: Rc<Cell<usize>>,
    }

    impl VisitorElement for DrawProbe {
        fn debug_name(&self) -> &'static str {
            "DrawProbe"
        }
    }

    impl EventElement for DrawProbe {}
    impl LayoutElement for DrawProbe {}
    impl Rebuildable for DrawProbe {}
    impl Drawable for DrawProbe {
        fn update(&self, _ctx: &BuildContext) {
            self.draws.set(self.draws.get() + 1);
        }
    }

    fn draw_context(visible_rect: Option<(f32, f32, f32, f32)>) -> BuildContext<'static> {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        let mut context = BuildContext::new(
            aimer_canvas::FrameCanvas::new(inner),
            ResolvedSize {
                width: 100.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        context.box_constraint.max_width = 100.0;
        context.box_constraint.max_height = 100.0;
        context.visible_rect = visible_rect;
        context
    }

    fn drawing_resizable(draws: Rc<Cell<usize>>) -> RawResizable<DrawProbe> {
        RawResizable {
            size: Cell::new(ResolvedSize {
                width: 100.0,
                height: 100.0,
            }),
            min: ResolvedSize::default(),
            max: ResolvedSize {
                width: f32::MAX,
                height: f32::MAX,
            },
            band: ResizeBand::inside(8.0),
            direction: Direction::ALL,
            on_resize: Callback::default(),
            on_resize_zone: Callback::default(),
            bounds: InteractionBounds::new(),
            active: Cell::new(None),
            grab: Cell::new(Grab::IDLE),
            hovered: Cell::new(None),
            child: DrawProbe { draws },
        }
    }

    #[tokio::test]
    async fn offscreen_resizable_skips_child_inside_its_clip() {
        let draws = Rc::new(Cell::new(0));
        let element = drawing_resizable(draws.clone());
        let mut ctx = draw_context(Some((0.0, 101.0, 100.0, 20.0)));

        element.update(&ctx);

        assert_eq!(draws.get(), 0);

        ctx.visible_rect = Some((0.0, 0.0, 100.0, 100.0));
        element.update(&ctx);
        assert_eq!(draws.get(), 1);
    }

    /// A resizable laid out over the top-left `200x100` corner, with an `8`
    /// logical pixel grab band inside its border and every side live.
    fn resizable(on_resize: Callback<ResolvedSize>) -> RawResizable<StubChild> {
        resizable_dragged_by(Direction::ALL, on_resize)
    }

    /// The same box, dragged by `direction` alone.
    fn resizable_dragged_by(
        direction: Direction,
        on_resize: Callback<ResolvedSize>,
    ) -> RawResizable<StubChild> {
        resizable_with(ResizeBand::inside(8.0), direction, on_resize)
    }

    /// The same box, with the grab band `band`.
    fn resizable_with(
        band: ResizeBand,
        direction: Direction,
        on_resize: Callback<ResolvedSize>,
    ) -> RawResizable<StubChild> {
        resizable_reporting(band, direction, on_resize, Callback::default())
    }

    /// The same box, watching the zone under the pointer.
    fn resizable_zoned(on_resize_zone: Callback<Direction>) -> RawResizable<StubChild> {
        resizable_reporting(
            ResizeBand::inside(8.0),
            Direction::ALL,
            Callback::default(),
            on_resize_zone,
        )
    }

    /// The box every other helper is built from.
    fn resizable_reporting(
        band: ResizeBand,
        direction: Direction,
        on_resize: Callback<ResolvedSize>,
        on_resize_zone: Callback<Direction>,
    ) -> RawResizable<StubChild> {
        let element = RawResizable {
            size: Cell::new(ResolvedSize {
                width: 200.0,
                height: 100.0,
            }),
            min: ResolvedSize {
                width: 50.0,
                height: 40.0,
            },
            max: ResolvedSize {
                width: 300.0,
                height: 150.0,
            },
            band,
            direction,
            on_resize,
            on_resize_zone,
            bounds: InteractionBounds::new(),
            active: Cell::new(None),
            grab: Cell::new(Grab::IDLE),
            hovered: Cell::new(None),
            child: StubChild,
        };
        element.bounds.save(1.0, 0.0, 0.0, 200.0, 100.0);
        element
    }

    fn mouse_at(x: f32, y: f32) -> PointerInfo {
        PointerInfo::mouse(Vec2d { x, y }, PointerButton::Primary)
    }

    /// Routes a mouse move through the framework's own hit testing, which offers
    /// an element only the events landing in the region it reports — the gate a
    /// direct `on_event` call steps over.
    fn dispatch_move(element: &dyn Element, x: f32, y: f32) -> EventResult {
        let pos = Vec2d { x, y };
        dispatch_event(element, pos, &ElementEvent::PointerMove(mouse_at(x, y)))
    }

    #[test]
    fn a_press_on_an_edge_takes_the_pointer_and_the_drag_widens_the_box() {
        let element = resizable(Callback::default());
        let pointer = PointerKey::new(PointerSource::Mouse, 0);

        let down = element.on_event(&ElementEvent::PointerDown(mouse_at(198.0, 50.0)));
        assert!(down.is_consumed());
        assert_eq!(down.capture_request(), CaptureRequest::Capture(pointer));

        let moved = element.on_event(&ElementEvent::PointerMove(mouse_at(238.0, 50.0)));

        assert!(moved.is_consumed());
        assert!(moved.needs_redraw());
        assert_eq!(
            element.size.get(),
            ResolvedSize {
                width: 240.0,
                height: 100.0
            }
        );
    }

    #[test]
    fn a_corner_drag_resizes_both_axes() {
        let element = resizable(Callback::default());

        let _ = element.on_event(&ElementEvent::PointerDown(mouse_at(198.0, 98.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(218.0, 128.0)));

        assert_eq!(
            element.size.get(),
            ResolvedSize {
                width: 220.0,
                height: 130.0
            }
        );
    }

    // The size is taken from where the drag started, so a drag that overshoots a
    // limit and comes back must land on the pointer again rather than drift by
    // everything the limit swallowed.
    #[test]
    fn a_drag_stays_within_its_limits_and_returns_from_them_intact() {
        let element = resizable(Callback::default());

        let _ = element.on_event(&ElementEvent::PointerDown(mouse_at(198.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(600.0, 50.0)));
        assert_eq!(element.size.get().width, 300.0, "clamped to max_width");

        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(0.0, 50.0)));
        assert_eq!(element.size.get().width, 50.0, "clamped to min_width");

        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(218.0, 50.0)));
        assert_eq!(
            element.size.get().width,
            220.0,
            "back on the pointer, not offset by what the limits swallowed"
        );
    }

    #[test]
    fn a_release_gives_the_pointer_back_and_ends_the_drag() {
        let element = resizable(Callback::default());
        let pointer = PointerKey::new(PointerSource::Mouse, 0);

        let _ = element.on_event(&ElementEvent::PointerDown(mouse_at(198.0, 50.0)));
        let up = element.on_event(&ElementEvent::PointerUp(mouse_at(238.0, 50.0)));
        assert_eq!(up.capture_request(), CaptureRequest::Release(pointer));

        let after = element.size.get();
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(400.0, 50.0)));

        assert_eq!(
            element.size.get(),
            after,
            "a released pointer resizes nothing"
        );
    }

    #[test]
    fn a_press_in_the_interior_is_left_to_the_child() {
        let element = resizable(Callback::default());

        let down = element.on_event(&ElementEvent::PointerDown(mouse_at(100.0, 50.0)));

        assert!(!down.is_consumed());
        assert_eq!(down.capture_request(), CaptureRequest::None);
    }

    #[test]
    fn every_resize_reports_the_new_size() {
        let sizes = Rc::new(RefCell::new(Vec::new()));
        let recorded = sizes.clone();
        let element = resizable(Callback::from(move |size: ResolvedSize| {
            recorded.borrow_mut().push(size)
        }));

        let _ = element.on_event(&ElementEvent::PointerDown(mouse_at(198.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(210.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(220.0, 50.0)));
        // Already at the limit: nothing changes, so nothing is reported.
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(600.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(700.0, 50.0)));

        assert_eq!(
            sizes
                .borrow()
                .iter()
                .map(|size| size.width)
                .collect::<Vec<_>>(),
            vec![212.0, 222.0, 300.0]
        );
    }

    #[test]
    fn hovering_an_edge_asks_for_its_resize_cursor_and_leaving_restores_the_default() {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let recorded = requests.clone();
        let previous = set_thread_cursor_handler(move |cursor| recorded.borrow_mut().push(cursor));

        let element = resizable(Callback::default());
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 98.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(100.0, 50.0)));

        restore_thread_cursor_handler(previous);

        assert_eq!(
            *requests.borrow(),
            vec![
                Cursor::Icon(CursorIcon::EwResize),
                Cursor::Icon(CursorIcon::NwseResize),
                Cursor::Icon(CursorIcon::Default),
            ]
        );
    }

    // The application restores the platform default cursor after every pointer
    // move no element consumed, so an unclaimed hover move wiped the resize
    // cursor before it was ever seen on screen.
    #[test]
    fn a_hover_over_a_handle_claims_the_move_that_set_the_cursor() {
        let element = resizable(Callback::default());

        let over_edge = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 50.0)));
        assert!(
            over_edge.is_consumed(),
            "an unconsumed move makes the application reset the cursor"
        );

        let over_child = element.on_event(&ElementEvent::PointerMove(mouse_at(100.0, 50.0)));
        assert!(
            !over_child.is_consumed(),
            "the interior belongs to the child, cursor and all"
        );
    }

    #[test]
    fn the_zone_under_the_pointer_is_reported_as_it_changes() {
        let zones = Rc::new(RefCell::new(Vec::new()));
        let recorded = zones.clone();
        let element = resizable_zoned(Callback::from(move |zone: Direction| {
            recorded.borrow_mut().push(zone)
        }));

        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 98.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(100.0, 50.0)));

        assert_eq!(
            *zones.borrow(),
            vec![Direction::RIGHT, Direction::BOTTOM_RIGHT, Direction::NONE,]
        );
    }

    // The cursor is asked for again on every move, but a zone report is a change
    // in what the pointer is over, so resting on one handle says it once.
    #[test]
    fn resting_on_a_zone_reports_it_once() {
        let zones = Rc::new(RefCell::new(Vec::new()));
        let recorded = zones.clone();
        let element = resizable_zoned(Callback::from(move |zone: Direction| {
            recorded.borrow_mut().push(zone)
        }));

        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 40.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(100.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(90.0, 50.0)));

        assert_eq!(*zones.borrow(), vec![Direction::RIGHT, Direction::NONE]);
    }

    // A drag keeps the handle it started on, so the zone must not flicker as the
    // pointer crosses the box on its way.
    #[test]
    fn a_drag_holds_the_zone_it_started_on() {
        let zones = Rc::new(RefCell::new(Vec::new()));
        let recorded = zones.clone();
        let element = resizable_zoned(Callback::from(move |zone: Direction| {
            recorded.borrow_mut().push(zone)
        }));

        let _ = element.on_event(&ElementEvent::PointerDown(mouse_at(198.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(100.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerUp(mouse_at(100.0, 50.0)));

        assert_eq!(
            *zones.borrow(),
            vec![Direction::RIGHT, Direction::NONE],
            "the right edge held throughout the drag, and the release let it go"
        );
    }

    #[test]
    fn a_zone_that_is_not_a_handle_is_never_reported() {
        let zones = Rc::new(RefCell::new(Vec::new()));
        let recorded = zones.clone();
        let element = resizable_reporting(
            ResizeBand::inside(8.0),
            Direction::RIGHT_EDGES,
            Callback::default(),
            Callback::from(move |zone: Direction| recorded.borrow_mut().push(zone)),
        );

        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(2.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 50.0)));

        assert_eq!(*zones.borrow(), vec![Direction::RIGHT]);
    }

    // The framework offers an element only the events that land in the region it
    // reports, so the move that carries the pointer off the box — the one that
    // ends the hover — is never seen by a box that reports its own bounds. The
    // zone stayed stuck on the side the pointer left by, and the cursor with it.
    #[test]
    fn leaving_the_box_releases_the_zone_and_the_cursor() {
        let zones = Rc::new(RefCell::new(Vec::new()));
        let recorded = zones.clone();
        let element = resizable_zoned(Callback::from(move |zone: Direction| {
            recorded.borrow_mut().push(zone)
        }))
        .boxed();

        let cursors = Rc::new(RefCell::new(Vec::new()));
        let seen = cursors.clone();
        let previous = set_thread_cursor_handler(move |cursor| seen.borrow_mut().push(cursor));

        let _ = dispatch_move(element.as_ref(), 198.0, 50.0);
        let _ = dispatch_move(element.as_ref(), 260.0, 50.0);

        restore_thread_cursor_handler(previous);

        assert_eq!(*zones.borrow(), vec![Direction::RIGHT, Direction::NONE]);
        assert_eq!(
            cursors.borrow().last(),
            Some(&Cursor::Icon(CursorIcon::Default)),
            "the shape this element put up is its own to take down"
        );
    }

    // Reporting no region is what an element does to hear an event it would
    // otherwise miss, so it must last exactly as long as the reason for it: once
    // the hover is let go, the box is a box again and the events outside it
    // belong to whatever is there.
    #[test]
    fn the_box_reports_its_region_again_once_the_pointer_is_gone() {
        let element = resizable(Callback::default()).boxed();
        assert!(element.pos_start_end().is_some());

        let _ = dispatch_move(element.as_ref(), 198.0, 50.0);
        let (start, end) = element
            .pos_start_end()
            .expect("a hovered handle reports a region");
        assert!(
            start.x < -1.0e6 && start.y < -1.0e6 && end.x > 1.0e6 && end.y > 1.0e6,
            "a hovered handle must hear the move that takes the pointer away"
        );

        let _ = dispatch_move(element.as_ref(), 260.0, 50.0);
        assert!(element.pos_start_end().is_some());
    }

    #[test]
    fn a_side_left_out_of_the_direction_neither_drags_nor_changes_the_cursor() {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let recorded = requests.clone();
        let previous = set_thread_cursor_handler(move |cursor| recorded.borrow_mut().push(cursor));

        let element = resizable_dragged_by(Direction::RIGHT_EDGES, Callback::default());

        let over_left = element.on_event(&ElementEvent::PointerMove(mouse_at(2.0, 50.0)));
        let press_left = element.on_event(&ElementEvent::PointerDown(mouse_at(2.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 50.0)));

        restore_thread_cursor_handler(previous);

        assert!(!over_left.is_consumed(), "the left band is the child's");
        assert!(!press_left.is_consumed());
        assert_eq!(press_left.capture_request(), CaptureRequest::None);
        assert_eq!(
            element.size.get().width,
            200.0,
            "a band that is not a handle resizes nothing"
        );
        assert_eq!(
            *requests.borrow(),
            vec![Cursor::Icon(CursorIcon::EwResize)],
            "only the live right edge ever asks for a cursor"
        );
    }

    // Switching a corner off must not punch a hole in the live edges it overlaps.
    #[test]
    fn a_drag_at_a_disabled_corner_falls_back_to_its_live_edge() {
        let element = resizable_dragged_by(Direction::EDGES, Callback::default());

        let _ = element.on_event(&ElementEvent::PointerDown(mouse_at(199.0, 96.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(219.0, 116.0)));

        assert_eq!(
            element.size.get(),
            ResolvedSize {
                width: 220.0,
                height: 100.0
            },
            "the right edge answered, so the height is untouched"
        );
    }

    #[test]
    fn an_empty_direction_leaves_every_pointer_event_to_the_child() {
        let element = resizable_dragged_by(Direction::NONE, Callback::default());

        let over_corner = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 98.0)));
        let press_corner = element.on_event(&ElementEvent::PointerDown(mouse_at(198.0, 98.0)));

        assert!(!over_corner.is_consumed());
        assert!(!press_corner.is_consumed());
        assert_eq!(press_corner.capture_request(), CaptureRequest::None);
    }

    // Regression: the resize shape used to be asked for only when the handle
    // under the pointer changed. The application restores the platform default
    // after every move no element consumed — and a move outside this element is
    // never offered to it at all — so once the pointer had left the box sideways
    // the window showed a plain arrow while the element still believed it owned
    // the resize shape. Coming back onto the very same edge was not a change, so
    // nothing was asked for and the cursor only recovered after a detour through
    // the interior.
    #[test]
    fn every_move_over_a_handle_asks_for_the_resize_cursor_again() {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let recorded = requests.clone();
        let previous = set_thread_cursor_handler(move |cursor| recorded.borrow_mut().push(cursor));

        let element = resizable(Callback::default());
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 40.0)));
        // The pointer left the box here: the element never sees that move, and
        // the application resets the cursor behind its back.
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 50.0)));

        restore_thread_cursor_handler(previous);

        assert_eq!(
            *requests.borrow(),
            vec![
                Cursor::Icon(CursorIcon::EwResize),
                Cursor::Icon(CursorIcon::EwResize),
            ]
        );
    }

    // The counterpart: the shape this element put up is its own to take down, and
    // exactly once, so a pointer resting over the child does not hammer the
    // platform with the same request.
    #[test]
    fn the_default_cursor_is_restored_once_on_the_way_out() {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let recorded = requests.clone();
        let previous = set_thread_cursor_handler(move |cursor| recorded.borrow_mut().push(cursor));

        let element = resizable(Callback::default());
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(198.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(100.0, 50.0)));
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(90.0, 50.0)));

        restore_thread_cursor_handler(previous);

        assert_eq!(
            *requests.borrow(),
            vec![
                Cursor::Icon(CursorIcon::EwResize),
                Cursor::Icon(CursorIcon::Default),
            ]
        );
    }

    #[test]
    fn a_hover_short_of_the_border_already_shows_the_resize_cursor() {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let recorded = requests.clone();
        let previous = set_thread_cursor_handler(move |cursor| recorded.borrow_mut().push(cursor));

        let element = resizable_with(
            ResizeBand::new(8.0, 8.0),
            Direction::ALL,
            Callback::default(),
        );
        let over_outer_band = element.on_event(&ElementEvent::PointerMove(mouse_at(204.0, 50.0)));
        let past_the_band = element.on_event(&ElementEvent::PointerMove(mouse_at(240.0, 50.0)));

        restore_thread_cursor_handler(previous);

        assert!(
            over_outer_band.is_consumed(),
            "four pixels short of the border is already the right edge"
        );
        assert!(!past_the_band.is_consumed());
        assert_eq!(
            *requests.borrow(),
            vec![
                Cursor::Icon(CursorIcon::EwResize),
                Cursor::Icon(CursorIcon::Default),
            ]
        );
    }

    #[test]
    fn a_press_short_of_the_border_starts_the_drag() {
        let element = resizable_with(
            ResizeBand::new(8.0, 8.0),
            Direction::ALL,
            Callback::default(),
        );
        let pointer = PointerKey::new(PointerSource::Mouse, 0);

        let down = element.on_event(&ElementEvent::PointerDown(mouse_at(204.0, 104.0)));
        assert_eq!(down.capture_request(), CaptureRequest::Capture(pointer));

        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(224.0, 114.0)));

        assert_eq!(
            element.size.get(),
            ResolvedSize {
                width: 220.0,
                height: 110.0
            },
            "the corner just outside the box answered"
        );
    }

    // The framework only offers an element the events landing within the region it
    // reports, so the outer half of the band is reachable only if the region
    // reaches with it.
    #[test]
    fn the_reported_region_covers_the_outer_band() {
        let element = resizable_with(
            ResizeBand::new(8.0, 5.0),
            Direction::ALL,
            Callback::default(),
        );

        let (start, end) = element.pos_start_end().expect("the box is laid out");

        assert_eq!(start, Vec2d { x: -5.0, y: -5.0 });
        assert_eq!(end, Vec2d { x: 205.0, y: 105.0 });
    }

    #[test]
    fn a_band_without_an_outer_reach_reports_the_box_alone() {
        let element = resizable(Callback::default());

        let (start, end) = element.pos_start_end().expect("the box is laid out");

        assert_eq!(start, Vec2d { x: 0.0, y: 0.0 });
        assert_eq!(end, Vec2d { x: 200.0, y: 100.0 });
    }

    // The default puts an equally wide band either side of the border, which is
    // what makes the cursor change as the pointer arrives at it.
    #[test]
    fn the_outer_reach_defaults_to_the_thickness_and_is_settable() {
        let default = Resizable::new().handle_thickness(9.0);
        assert_eq!(default.handle_outset, None);

        let asked_for = default.handle_outset(2.0);
        assert_eq!(asked_for.handle_outset, Some(2.0));
    }

    // Anything can rebuild the widget mid-drag — a sibling's `set_state`, an
    // animation frame — and the widget only remembers the size the box started
    // at, so without the hand-over the box would snap back under the pointer.
    #[test]
    fn a_rebuild_keeps_the_size_the_user_dragged_to() {
        let dragged = resizable(Callback::default());
        let _ = dragged.on_event(&ElementEvent::PointerDown(mouse_at(198.0, 50.0)));
        let _ = dragged.on_event(&ElementEvent::PointerMove(mouse_at(238.0, 50.0)));

        let rebuilt = resizable(Callback::default());
        rebuilt.adopt_runtime_state_from(&dragged as &dyn Element);

        assert_eq!(rebuilt.size.get().width, 240.0);

        let _ = rebuilt.on_event(&ElementEvent::PointerMove(mouse_at(258.0, 50.0)));

        assert_eq!(
            rebuilt.size.get().width,
            260.0,
            "the drag still in flight must carry on through the rebuild"
        );
    }

    #[test]
    fn a_cancelled_drag_stops_resizing_and_drops_the_resize_cursor() {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let recorded = requests.clone();
        let previous = set_thread_cursor_handler(move |cursor| recorded.borrow_mut().push(cursor));

        let element = resizable(Callback::default());
        let _ = element.on_event(&ElementEvent::PointerDown(mouse_at(198.0, 50.0)));
        let _ = element.on_event(&ElementEvent::Cancel);
        let _ = element.on_event(&ElementEvent::PointerMove(mouse_at(600.0, 50.0)));

        restore_thread_cursor_handler(previous);

        assert_eq!(element.size.get().width, 200.0);
        assert_eq!(
            requests.borrow().last(),
            Some(&Cursor::Icon(CursorIcon::Default))
        );
    }

    #[test]
    fn a_secondary_press_never_starts_a_resize() {
        let element = resizable(Callback::default());

        let down = element.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
            Vec2d { x: 198.0, y: 50.0 },
            PointerButton::Secondary,
        )));

        assert!(!down.is_consumed());
        assert!(element.active.get().is_none());
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_tests {
    use aimer_anteros::{PropertyValue, WidgetDocumentView, WidgetProperty};
    use aimer_widget::portable::{
        PortableBuildContext, PortableBuildError, PortableLimits, PortableValue,
        PortableWidgetLimits, PortableWidgetResource, PortableWidgetSchema, SourceFingerprint,
        StableId128,
    };
    use aimer_widget::{PortableWidget, RequiredChild};

    use super::{Direction, Resizable};
    use crate::ZeroSizedBox;

    fn source(value: u8) -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_bytes([value; 16]))
    }

    fn limits() -> PortableWidgetLimits {
        PortableWidgetLimits::new(4, 16, 4, 4, 64, 4_096).with_max_blob_bytes(256)
    }

    fn context(limits: PortableWidgetLimits) -> PortableBuildContext {
        PortableBuildContext::new(5, 2, limits, PortableLimits::new(8, 32, 64, 256, 2_048)).unwrap()
    }

    #[test]
    fn resizable_lowering_preserves_every_supported_property() {
        let schema = <Resizable<RequiredChild> as PortableWidgetSchema>::SCHEMA;
        let mut context = context(limits());
        let root = Resizable::new()
            .width(320.0)
            .height(180.0)
            .min_width(100.0)
            .min_height(80.0)
            .max_width(640.0)
            .max_height(360.0)
            .handle_thickness(9.0)
            .handle_outset(3.0)
            .direction(Direction::RIGHT_EDGES)
            .child(ZeroSizedBox::new())
            .to_portable_node(&mut context, source(4))
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        let properties = node.properties().collect::<Vec<_>>();

        assert_eq!(
            schema.widget().canonical_name(),
            "aimer.widget:aimer_container::single_child::Resizable"
        );
        assert_eq!(
            schema.children(),
            aimer_anteros::ChildCardinality::exactly(1)
        );
        assert_eq!(node.children().collect::<Vec<_>>(), vec![0]);
        assert_eq!(properties.len(), 9);

        for (field, value) in [
            ("width", 320.0),
            ("height", 180.0),
            ("min_width", 100.0),
            ("min_height", 80.0),
            ("max_width", 640.0),
            ("max_height", 360.0),
            ("handle_thickness", 9.0),
            ("handle_outset", 3.0),
        ] {
            let canonical_name =
                format!("aimer.property:aimer_container::single_child::Resizable:{field}");
            let property = schema
                .properties()
                .iter()
                .find(|property| property.canonical_name() == canonical_name)
                .unwrap();
            assert!(properties.contains(
                &WidgetProperty::new(property.id(), PropertyValue::F64(value)).optional()
            ));
        }

        let direction_schema = schema
            .properties()
            .iter()
            .find(|property| property.canonical_name().ends_with(":direction"))
            .unwrap();
        let direction_blob = properties
            .iter()
            .find(|property| property.property_id() == direction_schema.id())
            .and_then(|property| match property.value() {
                PropertyValue::BlobRef(index) => view.blob(index),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            Direction::decode_value(
                direction_blob,
                <Direction as PortableValue>::SCHEMA.version(),
            )
            .unwrap(),
            Direction::RIGHT_EDGES,
        );
    }

    #[test]
    fn resizable_lowering_enforces_property_and_blob_limits() {
        let widget = || {
            Resizable::new()
                .width(320.0)
                .height(180.0)
                .min_width(100.0)
                .min_height(80.0)
                .max_width(640.0)
                .max_height(360.0)
                .handle_thickness(9.0)
                .handle_outset(3.0)
                .direction(Direction::RIGHT_EDGES)
                .child(ZeroSizedBox::new())
        };
        let mut properties = context(limits().with_max_properties(8));
        let result = widget().to_portable_node(&mut properties, source(5));
        assert!(
            matches!(
                &result,
                Err(PortableBuildError::LimitExceeded {
                    resource: PortableWidgetResource::Properties,
                    max: 8,
                    actual: 9,
                })
            ),
            "{result:?}"
        );

        let mut blobs = context(limits().with_max_blob_bytes(0));
        let result = widget().to_portable_node(&mut blobs, source(6));
        let cause = match result {
            Err(PortableBuildError::PropertyEncoding { cause, .. }) => cause,
            other => panic!("unexpected blob-limit result: {other:?}"),
        };
        assert!(matches!(
            *cause,
            PortableBuildError::LimitExceeded {
                resource: PortableWidgetResource::BlobBytes,
                max: 0,
                ..
            }
        ));
    }

    #[test]
    fn resizable_rejects_callbacks_that_cannot_cross_the_portable_abi() {
        let mut resize = context(limits());
        assert!(matches!(
            Resizable::new()
                .on_resize(|_| {})
                .child(ZeroSizedBox::new())
                .to_portable_node(&mut resize, source(7)),
            Err(PortableBuildError::UnsupportedProperty {
                widget: "Resizable",
                property: "on_resize",
                ..
            })
        ));

        let mut zone = context(limits());
        assert!(matches!(
            Resizable::new()
                .on_resize_zone(|_| {})
                .child(ZeroSizedBox::new())
                .to_portable_node(&mut zone, source(8)),
            Err(PortableBuildError::UnsupportedProperty {
                widget: "Resizable",
                property: "on_resize_zone",
                ..
            })
        ));
    }
}
