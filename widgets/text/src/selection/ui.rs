//! The furniture a selection grows: two draggable knobs and a callout.
//!
//! A selection that can only be made, never adjusted, and only acted on through
//! a keyboard is a desktop selection. This module adds the other half — the two
//! blue handles at its ends and the floating menu above it — as state on the
//! [`SelectionSession`] plus one painter and one event interceptor.
//!
//! **The interceptor runs before the text does.** Both live *on top of* the
//! glyphs, so a press that grabbed a knob or tapped `Copy` would otherwise be
//! taken by the paragraph underneath and start a brand-new selection. Every
//! participant, and the region itself, therefore offers each pointer event to
//! [`intercept`] first and only handles what it gives back.
//!
//! **Neither the callout nor the knobs are painted by the region.** Both stand
//! clear of the selection — the callout floats above it, and each knob hangs
//! outside the line it marks — so every ancestor that clips, a `Scrollable`
//! above all, would cut them off. The callout is an
//! [`aimer_ctxmenu::ContextMenu`] presented through the modal host; the knobs go
//! through the same host's [`OverlayLayer`], which paints above every widget and
//! receives no events. See [`track_handles`].
//!
//! **What paints outside an element must be claimed by it.** Routing hit-tests,
//! so a region reporting only the box its text painted is a region whose knobs
//! cannot be pressed: the press lands on whatever encloses it instead, and a
//! scroll view reads it as a scroll. Every element that offers events to
//! [`intercept`] therefore grows its own box by [`hit_bounds_with_handles`].
//!
//! **The shape of the callout is the pointer's choice, not the platform's.** A
//! finger gets the pill floating above the selection; a right-click gets the
//! desktop list opening at the click. That is [`ContextMenuShape::for_source`]
//! deciding, so a phone browser and a desktop browser each behave the way their
//! user expects.
//!
//! Knob placement lives in [`super::handles`], which knows nothing about
//! sessions, canvases or events; the menu lives in `aimer_ctxmenu`; what is
//! left here is the state machine tying the two to a selection.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use aimer_attribute::{Bounds, ResolvedSize, Vec2d};
use aimer_ctxmenu::{ContextMenu, ContextMenuItem, ContextMenuShape};
use aimer_events::element::ElementEvent;
use aimer_events::pointer::PointerSource;
use aimer_modal::{AnchorHandle, ModalHandle, OverlayLayer, OverlayLayerHandle, OverlayPainter};
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{EventResult, PointerKey, claim_pointer, release_pointer};

use crate::selection::handles::{HANDLE_RADIUS, HandleCircle, HandleSide, grab_span, handle_at};
use crate::selection::selectable::{Selectable, TextGeometry};
use crate::selection::session::{SelectionSession, SelectionSlot};
use crate::selection::touch_hold::enter_hold;

/// The blue of the handles — the platform tint every system uses for them.
const HANDLE_COLOR: Color = Color::Rgba(0, 122, 255, 255);

/// One verb of the callout.
///
/// The list is deliberately short: these are the two actions that need no
/// context beyond the selection itself, which is what lets the callout be
/// answered entirely inside this crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MenuAction {
    /// Puts the selected text on the clipboard.
    Copy,
    /// Extends the selection to every participant of the session.
    SelectAll,
}

impl MenuAction {
    /// The label painted for this action.
    #[inline]
    const fn label(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::SelectAll => "Select All",
        }
    }
}

/// The actions the callout offers, in the order they are drawn.
const MENU_ACTIONS: [MenuAction; 2] = [MenuAction::Copy, MenuAction::SelectAll];

/// Everything the handles and the callout need to remember between frames.
///
/// It lives on the session rather than on an element because the two ends of a
/// selection can belong to two different widgets — neither of which is entitled
/// to own the furniture of the whole selection.
pub(crate) struct SelectionUi {
    /// The open callout, or `None` while none is showing.
    open: RefCell<Option<ModalHandle>>,
    /// The shape the open callout was opened in.
    shape: Cell<Option<ContextMenuShape>>,
    /// The rectangle the callout is pinned to, rewritten every frame while the
    /// pill is up so it follows a selection being adjusted.
    anchor: AnchorHandle,
    /// The knob a finger is dragging, and the pointer dragging it.
    dragging: Cell<Option<(HandleSide, PointerKey)>>,
    /// The overlay layer the knobs paint through, while there are knobs.
    layer: Cell<Option<OverlayLayerHandle>>,
    /// Whether the current selection was made by a finger, which is what
    /// decides that handles are drawn at all.
    touch: Cell<bool>,
    /// The session this furniture belongs to, held weakly: a callout must never
    /// be the reason a region stays alive.
    session: Weak<SelectionSession>,
}

impl SelectionUi {
    /// Creates the state of a selection with no furniture yet, belonging to the
    /// session being constructed.
    #[inline]
    pub fn new(session: Weak<SelectionSession>) -> Self {
        Self {
            open: RefCell::new(None),
            shape: Cell::new(None),
            anchor: AnchorHandle::new(),
            dragging: Cell::new(None),
            layer: Cell::new(None),
            touch: Cell::new(false),
            session,
        }
    }

    /// Offers the pill above the selection, following it as it changes.
    ///
    /// This is the touch shape, so the anchor keeps being rewritten: a knob
    /// dragged to a new offset moves the selection, and the pill has to follow
    /// it rather than stay where the gesture happened to end — see
    /// [`track_menu`].
    pub fn show_menu(&self) {
        let Some(anchor) = self
            .session
            .upgrade()
            .and_then(|session| session.menu_anchor())
        else {
            return;
        };
        self.open(ContextMenuShape::Pill, anchor);
    }

    /// Opens the desktop list with its corner at `pos`, where a right-click
    /// landed.
    ///
    /// The anchor is fixed: a desktop menu stays where it was opened even as
    /// the selection under it changes, which is what makes `Select All` from it
    /// feel like a menu rather than a jumping target.
    pub fn show_menu_at(&self, pos: Vec2d) {
        self.open(ContextMenuShape::List, Bounds::new(pos.x, pos.y, 0.0, 0.0));
    }

    /// Presents the callout in `shape`, pinned to `anchor`.
    fn open(&self, shape: ContextMenuShape, anchor: Bounds) {
        self.hide_menu();
        self.anchor.set_bounds(anchor);
        let session = self.session.clone();
        let handle = ContextMenu::new()
            .shape(shape)
            .anchor(self.anchor.clone())
            .items(
                MENU_ACTIONS
                    .iter()
                    .map(|action| ContextMenuItem::new(action.label()))
                    .collect(),
            )
            // The verb decides what becomes of the callout: `Copy` finishes the
            // job and closes it, `Select All` only reshapes what it acts on.
            .dismiss_on_select(false)
            .on_select(move |index| {
                if let Some(session) = session.upgrade()
                    && let Some(action) = MENU_ACTIONS.get(index).copied()
                {
                    session.perform(action);
                }
            })
            .show();
        *self.open.borrow_mut() = Some(handle);
        self.shape.set(Some(shape));
    }

    /// Takes the callout away.
    ///
    /// Repeated calls are harmless, which is what lets every dismissal path —
    /// a new gesture, a cleared selection, a cancelled drag — call it without
    /// asking first.
    pub fn hide_menu(&self) {
        if let Some(handle) = self.open.borrow_mut().take() {
            handle.dismiss();
        }
        self.shape.set(None);
    }

    /// Whether a callout is showing.
    ///
    /// Only tests ask: everything else acts on the callout through
    /// [`Self::show_menu`] and [`Self::hide_menu`], which are idempotent.
    #[cfg(test)]
    #[inline]
    pub fn is_menu_open(&self) -> bool {
        self.open.borrow().is_some()
    }

    /// Whether a callout is still presented, as the modal host sees it.
    ///
    /// Unlike [`Self::is_menu_open`] this asks the host rather than trusting
    /// the handle: the barrier and `Escape` close a callout without telling its
    /// owner, and a press being answered as a dismissal already counts as
    /// closed. That distinction is what lets a press *on* the callout keep the
    /// selection while the press that dismisses it drops the selection too.
    #[inline]
    pub fn is_menu_showing(&self) -> bool {
        self.open
            .borrow()
            .as_ref()
            .is_some_and(ModalHandle::is_showing)
    }

    /// The shape of the open callout, if one is showing.
    #[inline]
    pub fn menu_shape(&self) -> Option<ContextMenuShape> {
        self.shape.get()
    }

    /// The rectangle the open callout is pinned to.
    #[cfg(test)]
    #[inline]
    pub fn menu_anchor_bounds(&self) -> Option<Bounds> {
        self.anchor.bounds()
    }

    /// Records whether the current selection came from a finger.
    #[inline]
    pub fn set_touch(&self, touch: bool) {
        self.touch.set(touch);
    }

    /// Reports whether the current selection came from a finger, which is when
    /// handles are drawn.
    #[inline]
    pub fn is_touch(&self) -> bool {
        self.touch.get()
    }

    /// Forgets a half-finished interaction, as a cancelled gesture must.
    #[inline]
    pub fn forget_gesture(&self) {
        self.dragging.set(None);
    }

    /// The knob currently being dragged, if any.
    #[inline]
    pub fn dragging(&self) -> Option<(HandleSide, PointerKey)> {
        self.dragging.get()
    }
}

impl Drop for SelectionUi {
    /// Takes the knobs' layer down with the session that owned it.
    ///
    /// A layer outlives the tree that installed it — that is the point of one —
    /// so a region removed from the widget tree would otherwise leave its knobs
    /// painting over whatever replaced it until the next frame noticed.
    fn drop(&mut self) {
        if let Some(layer) = self.layer.take() {
            layer.remove();
        }
    }
}

impl SelectionSession {
    /// The two knobs of the current selection, in document order.
    ///
    /// `None` unless the selection was made by a finger and both of its ends
    /// belong to participants that have painted.
    pub(crate) fn handle_circles(&self) -> Option<(HandleCircle, HandleCircle)> {
        if !self.ui.is_touch() || !self.has_selection() {
            return None;
        }
        let (start, end) = self.endpoint_carets()?;
        Some((
            HandleCircle::of(start, HandleSide::Start),
            HandleCircle::of(end, HandleSide::End),
        ))
    }

    /// Runs one of the callout's actions.
    ///
    /// Copying dismisses the callout, the way accepting any menu does;
    /// selecting all keeps it, because the user is plainly still working with
    /// the selection.
    pub(crate) fn perform(self: &Rc<Self>, action: MenuAction) {
        match action {
            MenuAction::Copy => {
                let text = self.selected_text();
                if !text.is_empty() {
                    let _ = aimer_native::clipboard::set_text(&text);
                }
                self.ui.hide_menu();
            }
            MenuAction::SelectAll => {
                // `select_all` does not know a finger from a mouse, and losing
                // that would take the knobs away from a touch selection.
                let touch = self.ui.is_touch();
                self.select_all();
                self.ui.set_touch(touch);
            }
        }
        self.window().request_redraw();
    }
}

/// Offers a pointer event to the selection's furniture before the text under it
/// sees it.
///
/// Returns [`Some`] when the knobs or the callout took the event, in which case
/// the caller must return that result unchanged and do nothing else.
pub(crate) fn intercept(
    session: &Rc<SelectionSession>,
    event: &ElementEvent,
) -> Option<EventResult> {
    // The callout itself is not consulted here: it is a modal, so the host
    // offers it every event before the tree and dismisses it on an outside
    // press. What is left is the knobs, which live in the text's own layer.
    match event {
        ElementEvent::PointerDown(info) => {
            let pointer = PointerKey::new(info.source, info.id);
            let (start, end) = session.handle_circles()?;
            let side = handle_at(start, end, info.pos.x, info.pos.y)?;
            session.ui.dragging.set(Some((side, pointer)));
            session.ui.hide_menu();
            // Adjusting a selection is a drag of its own, so the knob takes the
            // pointer for itself and an enclosing scrollable leaves it alone.
            claim_pointer(pointer);
            Some(EventResult::consumed().with_pointer_capture(pointer))
        }
        ElementEvent::PointerMove(info) => {
            let pointer = PointerKey::new(info.source, info.id);
            let (side, owner) = session.ui.dragging()?;
            if owner != pointer {
                return None;
            }
            session.set_endpoint(side, info.pos.x, info.pos.y);
            Some(EventResult::consumed())
        }
        ElementEvent::PointerUp(info) => {
            let pointer = PointerKey::new(info.source, info.id);
            let (_, owner) = session.ui.dragging()?;
            if owner != pointer {
                return None;
            }
            session.ui.dragging.set(None);
            release_pointer(pointer);
            if session.has_selection() {
                session.ui.show_menu();
            }
            session.window().request_redraw();
            Some(EventResult::consumed())
        }
        ElementEvent::Cancel => {
            session.ui.forget_gesture();
            None
        }
        _ => None,
    }
}

/// Offers the callout after a gesture that finished with something selected.
///
/// Only a finger asks for it: a mouse has a keyboard beside it, and a menu
/// popping up after every drag would be in the way.
pub(crate) fn offer_menu_after_gesture(session: &Rc<SelectionSession>, source: PointerSource) {
    if source == PointerSource::Mouse || !session.has_selection() {
        return;
    }
    session.ui.set_touch(true);
    session.ui.show_menu();
    session.window().request_redraw();
}

/// Answers a secondary press — a right-click, or whatever the platform maps to
/// it — over a selectable text.
///
/// A right-click on an existing selection acts on it; one that lands elsewhere
/// first selects the word under the pointer, which is the convention every
/// desktop follows. Reports whether the press was taken.
pub(crate) fn open_context_menu(
    session: &Rc<SelectionSession>,
    slot: &Rc<SelectionSlot>,
    geometry: &TextGeometry,
    pos: Vec2d,
    pointer: PointerKey,
) -> bool {
    if !geometry
        .painted_bounds()
        .is_some_and(|bounds| bounds.is_inside(pos.x, pos.y))
    {
        return false;
    }
    if !session.has_selection() {
        let Some(offset) = geometry.offset_at(pos.x, pos.y) else {
            return false;
        };
        enter_hold(session, slot, offset, pointer);
        session.end(pointer);
    }
    match ContextMenuShape::for_source(pointer.source) {
        ContextMenuShape::List => {
            // A right-click is not a finger: it earns the desktop list at the
            // click, and no knobs.
            session.ui.set_touch(false);
            session.ui.show_menu_at(pos);
        }
        ContextMenuShape::Pill => {
            session.ui.set_touch(true);
            session.ui.show_menu();
        }
    }
    session.window().request_redraw();
    true
}

/// Keeps an open pill over the selection it belongs to.
///
/// Called once per frame, from the same place as [`paint_handles`]: a knob drag
/// moves the selection under the callout, and a desktop list — which is pinned
/// to the click, not to the text — must *not* follow it. A callout left with
/// nothing selected is taken down.
pub(crate) fn track_menu(session: &Rc<SelectionSession>) {
    if session.ui.menu_shape() != Some(ContextMenuShape::Pill) {
        return;
    }
    match session.menu_anchor() {
        Some(anchor) => session.ui.anchor.set_bounds(anchor),
        None => session.ui.hide_menu(),
    }
}

/// Keeps the knobs' overlay layer installed for exactly as long as there are
/// knobs.
///
/// Called once per frame, beside [`track_menu`]. A knob hangs *outside* the line
/// it marks — above the first caret, below the last — so painting it into the
/// text's own layer hands it to every clip the text sits in: inside a
/// `Scrollable` whose padding is outside its viewport, the clip begins exactly
/// where the content does and shears the knob in half. The layer paints above
/// every widget instead, and takes no events, so the press still reaches the
/// element that reported the knob as part of its box.
///
/// Costs one [`Cell`] read per frame once installed.
pub(crate) fn track_handles(session: &Rc<SelectionSession>) {
    match (session.handle_circles().is_some(), session.ui.layer.get()) {
        (true, None) => session
            .ui
            .layer
            .set(Some(OverlayLayer::install(handles_painter(session)))),
        (false, Some(layer)) => {
            layer.remove();
            session.ui.layer.set(None);
        }
        _ => {}
    }
}

/// The painter the knobs' layer runs, holding its session weakly.
///
/// Returning `false` retires the layer, which is how the knobs of a region that
/// was removed from the tree stop painting even though nothing was there to take
/// the layer down.
fn handles_painter(session: &Rc<SelectionSession>) -> OverlayPainter {
    let session = Rc::downgrade(session);
    Rc::new(move |ctx: &BuildContext, canvas: &aimer_canvas::Canvas| {
        let Some(session) = session.upgrade() else {
            return false;
        };
        paint_handles(ctx, canvas, &session);
        true
    })
}

/// Grows `bounds` to cover the knobs of `session`, in absolute logical pixels.
///
/// The box an element reports is what routing tests a press against, and a knob
/// is drawn outside the text: without this a press on one reaches whatever
/// encloses the region — a scroll view, which reads it as a scroll — and the
/// selection can never be adjusted. `None` already claims everything, and a
/// selection with no knobs claims nothing extra.
pub(crate) fn hit_bounds_with_handles(
    session: &Rc<SelectionSession>,
    bounds: Option<(Vec2d, Vec2d)>,
) -> Option<(Vec2d, Vec2d)> {
    let (from, to) = bounds?;
    let Some((start, end)) = session.handle_circles() else {
        return Some((from, to));
    };
    let span = grab_span(start, end);
    Some((
        Vec2d {
            x: from.x.min(span.x),
            y: from.y.min(span.y),
        },
        Vec2d {
            x: to.x.max(span.x + span.width),
            y: to.y.max(span.y + span.height),
        },
    ))
}

/// Records the handles of `session` into its overlay layer.
///
/// The layer covers the window from its origin, so a knob's absolute logical
/// bounds are already the coordinates it is recorded at.
fn paint_handles(
    ctx: &BuildContext,
    canvas: &aimer_canvas::Canvas,
    session: &Rc<SelectionSession>,
) {
    let Some((start, end)) = session.handle_circles() else {
        return;
    };
    let _ = ctx;
    let (red, green, blue, alpha) = HANDLE_COLOR.to_rgba();
    let color = aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha);
    let rect = |bounds: Bounds| {
        aimer_cupid::draw_cmd_v2::Rect::new(bounds.x, bounds.y, bounds.width, bounds.height)
    };
    for knob in [start, end] {
        canvas.fill_rect(rect(knob.bar_bounds()), [red, green, blue, alpha]);
        canvas.fill_rect_styled(
            rect(knob.circle_bounds()),
            color,
            [HANDLE_RADIUS; 4],
            [0.0; 4],
            aimer_cupid::utilities::Color::transparent(),
            [0.0; 4],
            aimer_cupid::utilities::Color::transparent(),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use aimer_attribute::Bounds;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::PointerKey;
    use aimer_widget::base::{Color, WindowHandle};

    use aimer_ctxmenu::ContextMenuShape;

    use super::*;
    use crate::selection::selectable::{SelectionCoordinator, TextGeometry};
    use crate::selection::session::{SelectionSession, SelectionSlot};
    use crate::selection::{SelectionPoint, TextHitRegion};

    const SELECTION_COLOR: Color = Color::Rgba(51, 153, 255, 96);

    fn window() -> WindowHandle {
        WindowHandle::headless(winit::dpi::PhysicalSize::new(400, 800), 1.0)
    }

    /// A session holding one ten-character participant whose glyphs are ten
    /// logical pixels wide and sixteen tall, on one line at `y = 100`.
    fn session() -> (Rc<SelectionSession>, Rc<SelectionSlot>, Rc<TextGeometry>) {
        let window = window();
        let session = SelectionSession::new(
            window.clone(),
            Rc::new(SelectionCoordinator::default()),
            SELECTION_COLOR,
        );
        let text = "abcdefghij";
        let geometry = Rc::new(TextGeometry::new(window));
        geometry.bounds.save(1.0, 0.0, 100.0, 100.0, 16.0);
        *geometry.regions.borrow_mut() = (0..text.len())
            .map(|index| {
                TextHitRegion::new(
                    index..index + 1,
                    Bounds::new(index as f32 * 10.0, 100.0, 10.0, 16.0),
                )
            })
            .collect();
        let slot = session.register(Rc::from(text), Rc::downgrade(&geometry) as _);
        slot.stamp();
        session.begin_frame();
        (session, slot, geometry)
    }

    fn finger() -> PointerKey {
        PointerKey::new(PointerSource::Touch, 0)
    }

    fn select(
        session: &Rc<SelectionSession>,
        slot: &Rc<SelectionSlot>,
        range: std::ops::Range<usize>,
    ) {
        session.begin_range(
            SelectionPoint::new(Rc::clone(slot), range.start),
            SelectionPoint::new(Rc::clone(slot), range.end),
            finger(),
        );
        session.end(finger());
    }

    fn press(x: f32, y: f32, source: PointerSource) -> ElementEvent {
        ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x, y },
            source,
            0,
            PointerButton::Primary,
        ))
    }

    fn moved(x: f32, y: f32, source: PointerSource) -> ElementEvent {
        ElementEvent::PointerMove(PointerInfo::new(
            Vec2d { x, y },
            source,
            0,
            PointerButton::Primary,
        ))
    }

    /// Where a pill centred on `anchor` sits, horizontally.
    fn anchor_center_x(anchor: Bounds) -> f32 {
        anchor.x + anchor.width * 0.5
    }

    fn release(x: f32, y: f32, source: PointerSource) -> ElementEvent {
        ElementEvent::PointerUp(PointerInfo::new(
            Vec2d { x, y },
            source,
            0,
            PointerButton::Primary,
        ))
    }

    /// A knob marks a glyph, but it is not *drawn* on one: it hangs outside the
    /// line, and painting it into the text's own layer puts it behind every clip
    /// the text sits in. A `Scrollable` clips to its viewport, and a viewport
    /// whose padding is outside it begins exactly where the content does — so
    /// the knob, centred on the first caret, loses the half of itself that
    /// sticks out. It therefore paints where the callout paints: above
    /// everything, clipped by nothing.
    #[test]
    fn the_knobs_are_painted_through_an_overlay_layer_no_ancestor_can_clip() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);

        track_handles(&session);

        assert!(
            OverlayLayer::is_installed(),
            "the knobs paint above every clip, the way the callout does"
        );
    }

    #[test]
    fn a_selection_with_no_knobs_installs_no_layer() {
        let (session, slot, _geometry) = session();
        session.begin(
            SelectionPoint::new(Rc::clone(&slot), 2),
            PointerKey::new(PointerSource::Mouse, 0),
        );
        session.extend_to_position(55.0, 108.0, PointerKey::new(PointerSource::Mouse, 0));

        track_handles(&session);

        assert!(
            !OverlayLayer::is_installed(),
            "a mouse selection grows none"
        );
    }

    #[test]
    fn clearing_the_selection_takes_the_layer_down_with_the_knobs() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        track_handles(&session);

        session.clear();
        track_handles(&session);

        assert!(!OverlayLayer::is_installed());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_layer_paints_a_bar_and_a_knob_for_each_end() {
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};

        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        let inner = InnerCanvas::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime is available in tests");
        let ctx = BuildContext::new(
            FrameCanvas::new(&inner),
            ResolvedSize {
                width: 400.0,
                height: 800.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            window(),
            runtime.handle().clone(),
        );

        // The layer records into the overlay's own list, as the host does.
        let tree = RenderTree::new();
        let overlay = tree.add_root(Rect::new(0.0, 0.0, 400.0, 800.0)).unwrap();
        let node_context = tree.context(overlay).unwrap();
        ctx.with_local_v2_paint_context(node_context, |ctx| {
            let canvas = aimer_canvas::Canvas::of(ctx);
            assert!(
                handles_painter(&session)(ctx, &canvas),
                "the layer stays installed"
            );
            canvas.finish();
        });

        let tint: aimer_cupid::utilities::Color = HANDLE_COLOR.into();
        let painted = tree
            .draw_list_snapshot(overlay)
            .unwrap()
            .commands
            .iter()
            .filter(|command| {
                matches!(command, DrawCommand::FillRect { color, .. }
                    if color.to_array() == tint.to_array())
            })
            .count();

        assert_eq!(painted, 4, "a bar and a knob at each end of the selection");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_layer_retires_with_the_region_it_belongs_to() {
        use aimer_canvas::{FrameCanvas, InnerCanvas};

        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        let painter = handles_painter(&session);
        let inner = InnerCanvas::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime is available in tests");
        let ctx = BuildContext::new(
            FrameCanvas::new(&inner),
            ResolvedSize {
                width: 400.0,
                height: 800.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            window(),
            runtime.handle().clone(),
        );

        drop(session);

        let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
        let overlay = tree
            .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 400.0, 800.0))
            .unwrap();
        let node_context = tree.context(overlay).unwrap();
        ctx.with_local_v2_paint_context(node_context, |ctx| {
            let canvas = aimer_canvas::Canvas::of(ctx);
            assert!(
                !painter(ctx, &canvas),
                "a region that went away leaves no knobs to paint"
            );
            canvas.finish();
        });
    }

    #[test]
    fn a_touch_selection_grows_a_knob_at_each_end() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);

        let (start, end) = session
            .handle_circles()
            .expect("a touch selection has knobs");

        assert_eq!(start.center_x, 20.0);
        assert_eq!(end.center_x, 50.0);
        assert!(start.center_y < 100.0, "the start knob is above the line");
        assert!(end.center_y > 116.0, "the end knob is below the line");
    }

    #[test]
    fn the_callout_is_anchored_to_the_start_of_the_selection() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);

        let anchor = session.menu_anchor().expect("a selection has an anchor");

        assert!(
            anchor.x <= 20.0 && anchor.x + anchor.width >= 20.0,
            "the start caret, not the whole selection"
        );
        assert!(
            anchor.y + anchor.height <= 116.0,
            "the pill sits over where the selection began, not over the line below it"
        );
        assert!(
            anchor.width < 20.0,
            "a caret is thin, so the pill sits over where the selection began"
        );
    }

    /// The knobs paint above every widget — the callout included — so a pill
    /// placed against the bare caret would have the start knob sitting on its
    /// edge. The anchor therefore covers the knob, and the pill's gap is kept
    /// from the knob rather than from the glyphs behind it.
    #[test]
    fn the_pill_hangs_above_the_knob_rather_than_behind_it() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        let (start, _) = session.handle_circles().expect("knobs");

        let anchor = session.menu_anchor().expect("a selection has an anchor");

        assert_eq!(anchor.y, start.circle_bounds().y);
    }

    #[test]
    fn a_selection_with_no_knobs_anchors_the_menu_on_its_caret_alone() {
        let (session, slot, _geometry) = session();
        let mouse = PointerKey::new(PointerSource::Mouse, 0);
        session.begin(SelectionPoint::new(Rc::clone(&slot), 2), mouse);
        session.extend_to_position(55.0, 108.0, mouse);

        let anchor = session.menu_anchor().expect("a selection has an anchor");

        assert_eq!(anchor, Bounds::new(20.0, 100.0, 0.0, 16.0));
    }

    #[test]
    fn the_callout_falls_back_to_the_selection_when_the_first_end_has_no_glyphs() {
        let (session, slot, geometry) = session();
        select(&session, &slot, 2..5);
        geometry.regions.borrow_mut().clear();

        assert_eq!(session.menu_anchor(), session.selection_bounds());
    }

    #[test]
    fn a_mouse_selection_grows_no_knobs() {
        let (session, slot, _geometry) = session();
        session.begin(
            SelectionPoint::new(Rc::clone(&slot), 2),
            PointerKey::new(PointerSource::Mouse, 0),
        );
        session.extend_to_position(55.0, 108.0, PointerKey::new(PointerSource::Mouse, 0));

        assert!(session.handle_circles().is_none());
    }

    #[test]
    fn a_collapsed_selection_grows_no_knobs() {
        let (session, slot, _geometry) = session();
        session.begin(SelectionPoint::new(Rc::clone(&slot), 2), finger());

        assert!(!session.has_selection());
        assert!(session.handle_circles().is_none());
    }

    #[test]
    fn dragging_the_end_knob_moves_only_that_end() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);

        let (_, end) = session.handle_circles().expect("knobs");
        assert!(
            intercept(
                &session,
                &press(end.center_x, end.center_y, PointerSource::Touch)
            )
            .is_some()
        );
        assert!(intercept(&session, &moved(85.0, 108.0, PointerSource::Touch)).is_some());

        assert_eq!(slot.selected_range(), Some(2..9));
    }

    #[test]
    fn dragging_the_start_knob_moves_only_that_end() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);

        let (start, _) = session.handle_circles().expect("knobs");
        assert!(
            intercept(
                &session,
                &press(start.center_x, start.center_y, PointerSource::Touch)
            )
            .is_some()
        );
        assert!(intercept(&session, &moved(2.0, 108.0, PointerSource::Touch)).is_some());

        assert_eq!(slot.selected_range(), Some(0..5));
    }

    #[test]
    fn a_press_that_misses_the_knobs_is_left_to_the_text() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);

        assert!(intercept(&session, &press(95.0, 300.0, PointerSource::Touch)).is_none());
    }

    #[test]
    fn only_the_finger_that_grabbed_a_knob_drags_it() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        let (_, end) = session.handle_circles().expect("knobs");
        intercept(
            &session,
            &press(end.center_x, end.center_y, PointerSource::Touch),
        );

        let other = ElementEvent::PointerMove(PointerInfo::new(
            Vec2d { x: 85.0, y: 108.0 },
            PointerSource::Touch,
            1,
            PointerButton::Primary,
        ));

        assert!(intercept(&session, &other).is_none());
        assert_eq!(slot.selected_range(), Some(2..5));
    }

    #[test]
    fn letting_a_knob_go_offers_the_callout() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        let (_, end) = session.handle_circles().expect("knobs");
        intercept(
            &session,
            &press(end.center_x, end.center_y, PointerSource::Touch),
        );
        assert!(!session.ui.is_menu_open(), "grabbing hides it");

        intercept(&session, &moved(85.0, 108.0, PointerSource::Touch));
        intercept(&session, &release(85.0, 108.0, PointerSource::Touch));

        assert!(session.ui.is_menu_open());
    }

    #[test]
    fn copying_from_the_callout_dismisses_it() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        session.ui.show_menu();

        session.perform(MenuAction::Copy);

        assert!(!session.ui.is_menu_open());
    }

    #[test]
    fn select_all_from_the_callout_extends_the_selection_and_keeps_it() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        session.ui.show_menu();

        session.perform(MenuAction::SelectAll);

        assert_eq!(slot.selected_range(), Some(0..10));
        assert!(session.ui.is_menu_open());
        assert!(
            session.handle_circles().is_some(),
            "select-all from the callout keeps the knobs a finger earned"
        );
    }

    #[test]
    fn a_right_click_opens_the_desktop_list_at_the_click() {
        let (session, slot, geometry) = session();
        let pointer = PointerKey::new(PointerSource::Mouse, 0);

        assert!(open_context_menu(
            &session,
            &slot,
            &geometry,
            Vec2d { x: 35.0, y: 108.0 },
            pointer
        ));

        assert_eq!(session.ui.menu_shape(), Some(ContextMenuShape::List));
        assert_eq!(
            session.ui.menu_anchor_bounds(),
            Some(Bounds::new(35.0, 108.0, 0.0, 0.0)),
            "its corner sits where the click landed"
        );
        assert!(
            session.handle_circles().is_none(),
            "a right-click earns the menu but no knobs"
        );
    }

    #[test]
    fn a_finger_gesture_opens_the_pill_over_the_start_of_the_selection() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);

        offer_menu_after_gesture(&session, PointerSource::Touch);

        assert_eq!(session.ui.menu_shape(), Some(ContextMenuShape::Pill));
        assert_eq!(session.ui.menu_anchor_bounds(), session.menu_anchor());
    }

    #[test]
    fn a_mouse_gesture_is_never_offered_the_callout() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        session.ui.hide_menu();

        offer_menu_after_gesture(&session, PointerSource::Mouse);

        assert!(!session.ui.is_menu_open());
    }

    #[test]
    fn a_finger_gesture_that_selected_nothing_is_not_offered_the_callout() {
        let (session, slot, _geometry) = session();
        session.begin(SelectionPoint::new(Rc::clone(&slot), 2), finger());

        offer_menu_after_gesture(&session, PointerSource::Touch);

        assert!(!session.ui.is_menu_open());
    }

    #[test]
    fn clearing_the_selection_takes_the_callout_with_it() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        session.ui.show_menu();

        session.clear();

        assert!(!session.ui.is_menu_open());
        assert!(session.handle_circles().is_none());
    }

    #[test]
    fn an_open_pill_follows_the_selection_it_belongs_to() {
        let (session, slot, _geometry) = session();
        select(&session, &slot, 2..5);
        session.ui.show_menu();
        // The pill is centred on its anchor, and the anchor covers the knob:
        // its centre is the caret the selection starts at.
        assert_eq!(
            session.ui.menu_anchor_bounds().map(anchor_center_x),
            Some(20.0)
        );

        // The callout stays up while the selection under it grows, which is
        // exactly what `Select All` from it does.
        session.perform(MenuAction::SelectAll);
        track_menu(&session);

        assert_eq!(
            session.ui.menu_anchor_bounds(),
            session.menu_anchor(),
            "the pill moves with the selection rather than staying behind"
        );
        assert_eq!(
            session.ui.menu_anchor_bounds().map(anchor_center_x),
            Some(0.0)
        );
        assert_eq!(slot.selected_range(), Some(0..10));
    }

    #[test]
    fn a_desktop_list_stays_where_it_was_opened() {
        let (session, slot, geometry) = session();
        select(&session, &slot, 2..5);
        open_context_menu(
            &session,
            &slot,
            &geometry,
            Vec2d { x: 35.0, y: 108.0 },
            PointerKey::new(PointerSource::Mouse, 0),
        );

        select(&session, &slot, 6..9);
        track_menu(&session);

        assert_eq!(
            session.ui.menu_anchor_bounds(),
            Some(Bounds::new(35.0, 108.0, 0.0, 0.0)),
            "a menu that jumped about would make its rows a moving target"
        );
    }

    #[test]
    fn a_callout_with_nothing_left_to_hang_off_takes_itself_down() {
        let (session, slot, geometry) = session();
        select(&session, &slot, 2..5);
        session.ui.show_menu();

        // The participant goes away — its subtree was removed — so there is
        // neither a caret nor a box for the pill to sit over.
        drop(geometry);
        track_menu(&session);

        assert!(!session.ui.is_menu_open());
    }
}
