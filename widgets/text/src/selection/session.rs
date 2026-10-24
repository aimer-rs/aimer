use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::{Rc, Weak};

use aimer_attribute::Bounds;
use aimer_events::pointer::PointerSource;
use aimer_widget::base::{Color, WindowHandle};
use aimer_widget::{FocusNode, PointerKey, claim_pointer, is_pointer_claimed, release_pointer};

use crate::TextSource;
use crate::selection::handles::HandleSide;
use crate::selection::selectable::{Selectable, SelectionCoordinator};
use crate::selection::ui::SelectionUi;
use crate::selection::{PointSelection, SelectionPoint, SelectionState};

/// A participant's registration inside a [`SelectionSession`].
///
/// The slot is created when a selectable text element is built and dropped with
/// it. It holds the participant's `TextSource` handle, so registering shared
/// text costs a pointer and a refcount, never a string copy.
///
/// # Examples
///
/// ```ignore
/// let slot = session.register(Rc::from("Title"), Rc::downgrade(&geometry));
/// slot.stamp();
/// assert_eq!(slot.text().as_ref(), "Title");
/// ```
pub(crate) struct SelectionSlot {
    session: Weak<SelectionSession>,
    text: RefCell<TextSource>,
    draw_stamp: Cell<u64>,
    pending_stamp: Cell<u64>,
    selectable: Weak<dyn Selectable>,
}

impl SelectionSlot {
    /// The shared text source. Cloning retains its backing handle.
    #[inline]
    pub fn text(&self) -> TextSource {
        self.text.borrow().clone()
    }

    /// The session this participant belongs to, while it is still alive.
    #[inline]
    pub fn session(&self) -> Option<Rc<SelectionSession>> {
        self.session.upgrade()
    }

    /// Replaces the text snapshot after a rebuild, clamping a live selection
    /// endpoint in this slot to the new length.
    pub fn set_text(&self, text: impl Into<TextSource>) {
        let text = text.into();
        if *self.text.borrow() == text {
            return;
        }
        let length = text.len();
        *self.text.borrow_mut() = text;
        if let Some(session) = self.session() {
            session.clamp_endpoints(self, length);
        }
    }

    /// Records that this participant painted, which is what defines document
    /// order.
    ///
    /// Called once per frame from `draw`, so it must stay a single write. The
    /// stamp stays *pending* until the next
    /// [`SelectionSession::begin_frame`]: a participant painting must never
    /// reorder the participants the very frame it paints, or the ones drawn
    /// after it would compare against a half-updated order and paint nothing.
    #[inline]
    pub fn stamp(&self) {
        if let Some(session) = self.session() {
            self.pending_stamp.set(session.next_stamp());
        }
    }

    /// The part of this participant's text that is selected, if any.
    pub fn selected_range(&self) -> Option<Range<usize>> {
        self.session()?.range_of(self)
    }

    /// Draw order of the last completed frame; participants that never painted
    /// come last.
    fn order(&self) -> u64 {
        match self.draw_stamp.get() {
            0 => u64::MAX,
            stamp => stamp,
        }
    }

    /// Publishes the stamp collected during the last frame as this
    /// participant's document order.
    #[inline]
    fn commit_stamp(&self) {
        let pending = self.pending_stamp.get();
        if pending != 0 {
            self.draw_stamp.set(pending);
        }
    }

    fn geometry(&self) -> Option<Rc<dyn Selectable>> {
        self.selectable.upgrade()
    }
}

/// One continuous selection shared by every selectable text of a region.
///
/// The session owns the registry of participants, the anchor/focus pair and the
/// pointer that is dragging. It hit-tests the registered geometry itself, which
/// is what lets a drag cross the gap between two widgets, skip a widget
/// entirely, or leave the region altogether.
///
/// # Examples
///
/// ```ignore
/// let session = SelectionSession::new(window, coordinator, selection_color);
/// let slot = session.register(Rc::from("Hello"), Rc::downgrade(&geometry));
/// session.begin(SelectionPoint::new(slot, 0), pointer);
/// session.select_all();
/// assert_eq!(session.selected_text(), "Hello");
/// ```
pub(crate) struct SelectionSession {
    slots: RefCell<Vec<Weak<SelectionSlot>>>,
    state: RefCell<SelectionState>,
    next_stamp: Cell<u64>,
    focused: Cell<bool>,
    /// The keyboard focus of the element that owns this session — the region,
    /// or a standalone text.
    ///
    /// The selection lives exactly as long as this node holds the focus: that
    /// is what makes a press anywhere else drop it, including presses the owner
    /// is never offered because they landed outside its bounds. The node is
    /// *attached* rather than created here, because the element has to hand the
    /// very same node to the framework, and a session outlives the elements
    /// that come and go around it across rebuilds.
    focus_node: RefCell<FocusNode>,
    window: WindowHandle,
    coordinator: Rc<SelectionCoordinator>,
    selection_color: Color,
    /// The knobs and the callout offered on top of the selection.
    pub(crate) ui: SelectionUi,
}

impl SelectionSession {
    /// Creates an empty session painting selections in `selection_color`.
    pub fn new(
        window: WindowHandle,
        coordinator: Rc<SelectionCoordinator>,
        selection_color: Color,
    ) -> Rc<Self> {
        // Cyclic because the callout's overlay painter has to reach back into
        // the session while it paints, and must do so weakly.
        Rc::new_cyclic(|session| Self {
            slots: RefCell::new(Vec::new()),
            state: RefCell::new(SelectionState::default()),
            next_stamp: Cell::new(0),
            focused: Cell::new(false),
            focus_node: RefCell::new(FocusNode::new()),
            ui: SelectionUi::new(session.clone()),
            window,
            coordinator,
            selection_color,
        })
    }

    /// The window every participant of this session paints into.
    #[inline]
    pub fn window(&self) -> &WindowHandle {
        &self.window
    }

    /// The color participants paint their part of the selection with.
    #[inline]
    pub const fn selection_color(&self) -> Color {
        self.selection_color
    }

    /// Reports whether the session holds the keyboard focus for selection.
    #[inline]
    pub fn is_focused(&self) -> bool {
        self.focused.get()
    }

    /// Makes `node` the keyboard focus of this session.
    ///
    /// The owning element calls this with the node it exposes to the framework,
    /// so that [`Self::focus`] asks for the focus *of that element*. A rebuilt
    /// element attaches its own node the same way, which is what keeps a
    /// selection responsive to outside presses across a rebuild.
    #[inline]
    pub fn attach_focus_node(&self, node: &FocusNode) {
        self.focus_node.replace(node.clone());
    }

    /// Registers a participant with its text handle and its geometry provider.
    pub fn register(
        self: &Rc<Self>,
        text: impl Into<TextSource>,
        selectable: Weak<dyn Selectable>,
    ) -> Rc<SelectionSlot> {
        let slot = Rc::new(SelectionSlot {
            session: Rc::downgrade(self),
            text: RefCell::new(text.into()),
            draw_stamp: Cell::new(0),
            pending_stamp: Cell::new(0),
            selectable,
        });
        let mut slots = self.slots.borrow_mut();
        slots.retain(|slot| slot.strong_count() > 0);
        slots.push(Rc::downgrade(&slot));
        slot
    }

    /// Opens a new frame: the stamps collected while the previous frame painted
    /// become the document order every participant of this frame reads.
    ///
    /// Called from the region's `draw`, before the subtree paints, so the order
    /// stays frozen for the whole frame.
    pub fn begin_frame(&self) {
        for slot in self.slots.borrow().iter().filter_map(Weak::upgrade) {
            slot.commit_stamp();
        }
        self.next_stamp.set(0);
    }

    /// Takes the selection for this session, clearing whichever session held it.
    pub fn claim(self: &Rc<Self>) {
        self.coordinator.claim(self);
    }

    /// Makes the session the keyboard target for select-all and copy.
    ///
    /// The flag is set at once — the shortcuts must work from the very press
    /// that started the selection — while the node only *asks* for the focus,
    /// which the framework grants when it next synchronizes the tree. Taking
    /// the tree's focus is what arranges for [`Self::blur`] to be called when
    /// the user presses anywhere else.
    #[inline]
    pub fn focus(&self) {
        self.focused.set(true);
        self.focus_node.borrow().request_focus();
    }

    /// Answers the loss of the keyboard focus, which is how the session learns
    /// of a press it was never offered.
    ///
    /// A press outside the region is delivered to whatever is under it and to
    /// nothing else, so the selection cannot be dropped by watching for one.
    /// Losing the focus to that press is the notification, and dropping the
    /// selection is the answer — with one exception. The callout is a modal: it
    /// takes *every* press while it is up, including the press on `Copy`, and
    /// the tree below it loses the focus either way. Clearing then would leave
    /// `Copy` nothing to copy, so the selection stays and the focus is asked
    /// for again — and the press that dismisses the callout, which is already
    /// on its way out by the time this runs, clears the selection as any other
    /// outside press does.
    pub fn blur(&self) {
        if self.ui.is_menu_showing() {
            self.focus_node.borrow().request_focus();
            return;
        }
        self.clear();
    }

    /// Reports whether anything is selected at all.
    ///
    /// A collapsed selection — the caret a press leaves behind — is not a
    /// selection: nothing would be copied and nothing is painted.
    pub fn has_selection(&self) -> bool {
        let state = self.state.borrow();
        let Some(selection) = state.selection() else {
            return false;
        };
        !(Rc::ptr_eq(&selection.anchor.slot, &selection.focus.slot)
            && selection.anchor.offset == selection.focus.offset)
    }

    /// The caret rectangles of the first and last endpoint, in document order.
    ///
    /// `None` while nothing is selected, or while either endpoint belongs to a
    /// participant that has not painted — a handle cannot be drawn against
    /// geometry that does not exist yet.
    pub fn endpoint_carets(&self) -> Option<(Bounds, Bounds)> {
        let state = self.state.borrow();
        let selection = state.selection()?;
        let (start, end) = self.document_order(selection);
        let start_caret = start.slot.geometry()?.caret_rect(start.offset)?;
        let end_caret = end.slot.geometry()?.caret_rect(end.offset)?;
        Some((start_caret, end_caret))
    }

    /// The rectangle the callout is placed against: the caret the selection
    /// *starts* at, in document order.
    ///
    /// A selection spanning several lines or several widgets has a bounding box
    /// whose centre is nowhere in particular — the callout would float over the
    /// middle of the region, far from the word the gesture began on. Anchoring
    /// it to the first endpoint keeps it where the user is looking, which is
    /// what every platform does.
    ///
    /// Falls back to [`Self::selection_bounds`] when that endpoint's
    /// participant has painted its box but not its glyphs, so a callout is
    /// never lost for want of a caret.
    ///
    /// The knob of that endpoint is part of the anchor, because it is part of
    /// what the callout must not cover: a pill placed against the bare caret
    /// hangs its gap over the knob standing above it, and the knob — which
    /// paints above every widget, the callout included — would then sit on top
    /// of the pill's edge.
    pub fn menu_anchor(&self) -> Option<Bounds> {
        let anchor = self
            .selection_start_caret()
            .or_else(|| self.selection_bounds())?;
        Some(match self.handle_circles() {
            Some((start, _)) => union_of(anchor, start.circle_bounds()),
            None => anchor,
        })
    }

    /// The caret rectangle of the first endpoint in document order.
    fn selection_start_caret(&self) -> Option<Bounds> {
        if !self.has_selection() {
            return None;
        }
        let state = self.state.borrow();
        let selection = state.selection()?;
        let (start, _) = self.document_order(selection);
        start.slot.geometry()?.caret_rect(start.offset)
    }

    /// The rectangle every selected participant painted into.
    pub fn selection_bounds(&self) -> Option<Bounds> {
        let mut union: Option<Bounds> = None;
        for slot in self.ordered_slots() {
            let Some(range) = self.range_of(&slot) else {
                continue;
            };
            if range.is_empty() {
                continue;
            }
            let Some(bounds) = slot
                .geometry()
                .and_then(|geometry| geometry.painted_bounds())
            else {
                continue;
            };
            union = Some(match union {
                None => bounds,
                Some(union) => union_of(union, bounds),
            });
        }
        union
    }

    /// Moves one end of the selection to the offset nearest an absolute logical
    /// position, leaving the other end where it is.
    ///
    /// This is what dragging a handle does. `side` is in document order, so the
    /// end being dragged keeps its identity even when it overtakes the other
    /// one.
    pub fn set_endpoint(&self, side: HandleSide, x: f32, y: f32) -> bool {
        let Some((slot, offset)) = self.offset_nearest(x, y) else {
            return false;
        };
        let moved = SelectionPoint::new(slot, offset);
        let mut state = self.state.borrow_mut();
        let Some(selection) = state.selection().cloned() else {
            return false;
        };
        let (start, end) = if is_before(&selection.focus, &selection.anchor) {
            (selection.focus, selection.anchor)
        } else {
            (selection.anchor, selection.focus)
        };
        let (anchor, focus) = match side {
            HandleSide::Start => (end, moved),
            HandleSide::End => (start, moved),
        };
        if anchor.offset == focus.offset && Rc::ptr_eq(&anchor.slot, &focus.slot) {
            return false;
        }
        state.set(PointSelection { anchor, focus });
        drop(state);
        self.window.request_redraw();
        true
    }

    /// Starts a gesture, collapsing the selection at `point`.
    ///
    /// The pointer is claimed for as long as the gesture lasts — see
    /// [`aimer_widget::pointer_claim`] — so an enclosing scrollable reads the
    /// drag that follows as the selection it is instead of stealing it for a
    /// scroll.
    pub fn begin(self: &Rc<Self>, point: SelectionPoint, pointer: PointerKey) {
        self.ui.hide_menu();
        self.ui.set_touch(pointer.source == PointerSource::Touch);
        claim_pointer(pointer);
        self.claim();
        self.focus();
        self.state.borrow_mut().begin(point, pointer);
        self.window.request_redraw();
    }

    /// Starts a gesture that already covers `anchor..focus`.
    ///
    /// This is what a completed touch hold does: it enters the selection with
    /// the word under the finger already highlighted, so the hold is visible,
    /// and leaves the gesture open for the finger to extend. The gesture counts
    /// as dragged from the outset, which is what keeps a hold on a link from
    /// also following it.
    pub fn begin_range(
        self: &Rc<Self>,
        anchor: SelectionPoint,
        focus: SelectionPoint,
        pointer: PointerKey,
    ) {
        self.ui.hide_menu();
        self.ui.set_touch(pointer.source == PointerSource::Touch);
        claim_pointer(pointer);
        self.claim();
        self.focus();
        {
            let mut state = self.state.borrow_mut();
            state.begin(anchor, pointer);
            state.update(focus, pointer);
        }
        self.window.request_redraw();
    }

    /// Extends the selection to the offset nearest an absolute logical position.
    ///
    /// The participant is chosen by distance, so a position in the gap between
    /// two of them, or outside them all, still extends the selection.
    pub fn extend_to_position(&self, x: f32, y: f32, pointer: PointerKey) -> bool {
        if self.state.borrow().active_pointer() != Some(pointer) {
            return false;
        }
        let Some((slot, offset)) = self.offset_nearest(x, y) else {
            return false;
        };
        if !self
            .state
            .borrow_mut()
            .update(SelectionPoint::new(slot, offset), pointer)
        {
            return false;
        }
        self.window.request_redraw();
        true
    }

    /// Ends the gesture owned by `pointer`.
    ///
    /// The claim taken when the gesture began is given back here, so the very
    /// next drag of that pointer is a scroll again.
    pub fn end(&self, pointer: PointerKey) -> bool {
        release_pointer(pointer);
        self.state.borrow_mut().end(pointer)
    }

    /// Reports whether the current gesture ever covered more than one offset.
    pub fn was_dragged(&self) -> bool {
        self.state.borrow().was_dragged()
    }

    /// The pointer currently dragging the selection, while its gesture is still
    /// live.
    ///
    /// A gesture [claims](aimer_widget::pointer_claim) its pointer for as long as
    /// it lasts, and the framework drops every claim the moment a pointer goes up
    /// or an interaction is cancelled — a claim can never outlive the finger that
    /// made it. A gesture whose claim is gone is therefore a gesture whose finger
    /// is gone, and this reports none.
    ///
    /// That distinction is what keeps a selection dismissable. A release is not
    /// guaranteed to arrive — a scrollable that has taken over one finger
    /// swallows the lift of another — and touch identities are reused, so the
    /// next press of the same finger would look like the press that opened such a
    /// gesture. Everything that asks this question asks it to tell a press of its
    /// own gesture from a fresh one, and a fresh press mistaken for its own
    /// dismisses nothing at all.
    pub fn active_pointer(&self) -> Option<PointerKey> {
        let pointer = self.state.borrow().active_pointer()?;
        is_pointer_claimed(pointer).then_some(pointer)
    }

    /// Restores the selection from before the current gesture.
    pub fn cancel(&self) {
        self.release_gesture_claim();
        self.state.borrow_mut().cancel();
        self.window.request_redraw();
    }

    /// Gives back the claim of whichever pointer is still dragging, if any.
    ///
    /// Losing the gesture — to a cancellation, or to a press that dropped the
    /// selection — must also lose the claim, or the pointer would stay spoken
    /// for after the selection it spoke for is gone.
    fn release_gesture_claim(&self) {
        if let Some(pointer) = self.state.borrow().active_pointer() {
            release_pointer(pointer);
        }
    }

    /// Drops the selection and the keyboard focus.
    pub fn clear(&self) {
        self.release_gesture_claim();
        self.ui.hide_menu();
        self.ui.forget_gesture();
        self.state.borrow_mut().clear();
        self.focused.set(false);
        self.focus_node.borrow().unfocus();
        self.window.request_redraw();
    }

    /// Drops whatever the session holds so a fresh gesture can replace it,
    /// reporting whether there was anything to drop.
    ///
    /// This is what a press means for a selection that is already on screen. The
    /// answer keeps a press on unselected text from asking for a repaint — or for
    /// the tree's focus to be resolved again — for nothing, which on a paragraph
    /// nobody has selected is every press there is.
    pub fn dismiss(&self) -> bool {
        let idle = {
            let state = self.state.borrow();
            state.selection().is_none() && state.active_pointer().is_none()
        };
        if idle {
            return false;
        }
        self.clear();
        true
    }

    /// Selects every participant in the session, in document order.
    pub fn select_all(self: &Rc<Self>) {
        self.claim();
        self.focus();
        let ordered = self.ordered_slots();
        let (Some(first), Some(last)) = (ordered.first(), ordered.last()) else {
            return;
        };
        let end = last.text().len();
        self.state.borrow_mut().set(PointSelection {
            anchor: SelectionPoint::new(Rc::clone(first), 0),
            focus: SelectionPoint::new(Rc::clone(last), end),
        });
        self.window.request_redraw();
    }

    /// Replaces the selection with a range inside one participant.
    pub(crate) fn set_local_range(
        self: &Rc<Self>,
        slot: &Rc<SelectionSlot>,
        anchor: usize,
        focus: usize,
    ) {
        self.claim();
        self.focus();
        let length = slot.text().len();
        self.state.borrow_mut().set(PointSelection {
            anchor: SelectionPoint::new(Rc::clone(slot), anchor.min(length)),
            focus: SelectionPoint::new(Rc::clone(slot), focus.min(length)),
        });
        self.window.request_redraw();
    }

    /// The selected text of every participant in document order, joined by
    /// `\n`.
    ///
    /// The text comes from the slots' snapshots, so participants that are
    /// clipped or scrolled out of view still contribute.
    pub fn selected_text(&self) -> String {
        let mut parts = Vec::new();
        for slot in self.ordered_slots() {
            let Some(range) = self.range_of(&slot) else {
                continue;
            };
            let text = slot.text();
            let Some(selected) = text.get(range) else {
                continue;
            };
            parts.push(selected.to_owned());
        }
        parts.join("\n")
    }

    /// The part of `slot`'s text covered by the current selection.
    pub fn range_of(&self, slot: &SelectionSlot) -> Option<Range<usize>> {
        let state = self.state.borrow();
        let selection = state.selection()?;
        let (start, end) = self.document_order(selection);
        let length = slot.text().len();

        if Rc::ptr_eq(&start.slot, &end.slot) {
            if !std::ptr::eq(Rc::as_ptr(&start.slot), slot) {
                return None;
            }
            let low = start.offset.min(end.offset).min(length);
            let high = start.offset.max(end.offset).min(length);
            return Some(low..high);
        }

        let is_start = std::ptr::eq(Rc::as_ptr(&start.slot), slot);
        let is_end = std::ptr::eq(Rc::as_ptr(&end.slot), slot);
        if is_start {
            return Some(start.offset.min(length)..length);
        }
        if is_end {
            return Some(0..end.offset.min(length));
        }
        let order = slot.order();
        if order > start.slot.order() && order < end.slot.order() {
            return Some(0..length);
        }
        None
    }

    fn document_order<'a>(
        &self,
        selection: &'a PointSelection,
    ) -> (&'a SelectionPoint, &'a SelectionPoint) {
        if is_before(&selection.focus, &selection.anchor) {
            (&selection.focus, &selection.anchor)
        } else {
            (&selection.anchor, &selection.focus)
        }
    }

    fn next_stamp(&self) -> u64 {
        let stamp = self.next_stamp.get() + 1;
        self.next_stamp.set(stamp);
        stamp
    }

    /// The live participants, ordered by the draw order of the last frame.
    fn ordered_slots(&self) -> Vec<Rc<SelectionSlot>> {
        let mut slots = self
            .slots
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        slots.sort_by_key(|slot| slot.order());
        slots
    }

    /// Clamps a live endpoint inside `slot` after its text shrank.
    fn clamp_endpoints(&self, slot: &SelectionSlot, length: usize) {
        let mut state = self.state.borrow_mut();
        let Some(selection) = state.selection() else {
            return;
        };
        let mut clamped = selection.clone();
        let mut changed = false;
        for point in [&mut clamped.anchor, &mut clamped.focus] {
            if std::ptr::eq(Rc::as_ptr(&point.slot), slot) && point.offset > length {
                point.offset = length;
                changed = true;
            }
        }
        if changed {
            let active = state.active_pointer();
            state.replace_selection(clamped);
            debug_assert_eq!(state.active_pointer(), active);
        }
    }

    /// Picks the participant nearest an absolute logical position and asks it
    /// for the offset there.
    fn offset_nearest(&self, x: f32, y: f32) -> Option<(Rc<SelectionSlot>, usize)> {
        let ordered = self.ordered_slots();
        let mut best: Option<(Rc<SelectionSlot>, Rc<dyn Selectable>, f32)> = None;
        for slot in ordered {
            let Some(geometry) = slot.geometry() else {
                continue;
            };
            let Some(bounds) = geometry.painted_bounds() else {
                continue;
            };
            let distance = distance_to_bounds(bounds, x, y);
            if best
                .as_ref()
                .is_none_or(|(_, _, closest)| distance < *closest)
            {
                best = Some((slot, geometry, distance));
            }
        }
        let (slot, geometry, _) = best?;
        let offset = geometry.offset_at(x, y)?;
        Some((slot, offset))
    }
}

/// Reports whether `left` comes before `right` in document order.
///
/// Participants are compared by draw order, and two points inside the same
/// participant by their offset — which is what tells the two ends of a
/// single-widget selection apart.
fn is_before(left: &SelectionPoint, right: &SelectionPoint) -> bool {
    let (left_order, right_order) = (left.slot.order(), right.slot.order());
    if left_order != right_order {
        return left_order < right_order;
    }
    left.offset < right.offset
}

/// The smallest rectangle covering both.
fn union_of(left: Bounds, right: Bounds) -> Bounds {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (left.x + left.width).max(right.x + right.width);
    let bottom_edge = (left.y + left.height).max(right.y + right.height);
    Bounds::new(x, y, right_edge - x, bottom_edge - y)
}

/// Squared distance from `(x, y)` to `bounds`, weighting the vertical axis so a
/// position between two lines picks the nearer line rather than the nearer
/// glyph.
fn distance_to_bounds(bounds: Bounds, x: f32, y: f32) -> f32 {
    let dx = if x < bounds.x {
        bounds.x - x
    } else if x > bounds.x + bounds.width {
        x - (bounds.x + bounds.width)
    } else {
        0.0
    };
    let dy = if y < bounds.y {
        bounds.y - y
    } else if y > bounds.y + bounds.height {
        y - (bounds.y + bounds.height)
    } else {
        0.0
    };
    dy * dy * 1024.0 + dx * dx
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use aimer_attribute::Bounds;
    use aimer_events::pointer::PointerSource;
    use aimer_rubick::ShareRef;
    use aimer_widget::PointerKey;
    use aimer_widget::base::{Color, WindowHandle};

    use super::{SelectionSession, SelectionSlot};
    use crate::TextSource;
    use crate::selection::selectable::{SelectionCoordinator, TextGeometry};
    use crate::selection::{SelectionPoint, TextHitRegion};

    const SELECTION_COLOR: Color = Color::Rgba(51, 153, 255, 96);

    fn window() -> WindowHandle {
        WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 200), 1.0)
    }

    fn session(window: &WindowHandle) -> Rc<SelectionSession> {
        SelectionSession::new(
            window.clone(),
            Rc::new(SelectionCoordinator::default()),
            SELECTION_COLOR,
        )
    }

    /// Registers a participant whose glyphs are ten logical pixels wide and ten
    /// tall, laid out on one line starting at `y`.
    fn participant(
        session: &Rc<SelectionSession>,
        window: &WindowHandle,
        text: &str,
        y: f32,
    ) -> (Rc<TextGeometry>, Rc<SelectionSlot>) {
        let geometry = Rc::new(TextGeometry::new(window.clone()));
        geometry
            .bounds
            .save(1.0, 0.0, y, text.len() as f32 * 10.0, 10.0);
        *geometry.regions.borrow_mut() = (0..text.len())
            .map(|index| {
                TextHitRegion::new(
                    index..index + 1,
                    Bounds::new(index as f32 * 10.0, y, 10.0, 10.0),
                )
            })
            .collect();
        let slot = session.register(Rc::from(text), Rc::downgrade(&geometry) as _);
        (geometry, slot)
    }

    #[test]
    fn registering_share_ref_text_preserves_its_source() {
        let window = window();
        let session = session(&window);
        let source = ShareRef::from_static("shared selectable text");
        let geometry = Rc::new(TextGeometry::new(window));
        let slot = session.register(
            TextSource::from(source.clone()),
            Rc::downgrade(&geometry) as _,
        );
        let TextSource::ShareRef(slot_source) = slot.text() else {
            panic!("selection registration should retain the ShareRef source")
        };

        assert!(ShareRef::ptr_eq(&source, &slot_source));
    }

    fn mouse() -> PointerKey {
        PointerKey::new(PointerSource::Mouse, 0)
    }

    #[test]
    fn forward_drag_selects_suffix_all_and_prefix_across_three_participants() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        let (_second_geometry, second) = participant(&session, &window, "second", 20.0);
        let (_third_geometry, third) = participant(&session, &window, "third", 40.0);
        for slot in [&first, &second, &third] {
            slot.stamp();
        }
        session.begin_frame();

        session.begin(SelectionPoint::new(Rc::clone(&first), 2), mouse());
        assert!(session.extend_to_position(25.0, 45.0, mouse()));

        assert_eq!(first.selected_range(), Some(2..5));
        assert_eq!(second.selected_range(), Some(0..6));
        assert_eq!(third.selected_range(), Some(0..3));
        assert_eq!(session.selected_text(), "rst\nsecond\nthi");
    }

    #[test]
    fn a_participant_painting_does_not_reorder_the_frame_it_paints_in() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        let (_second_geometry, second) = participant(&session, &window, "second", 20.0);
        let (_third_geometry, third) = participant(&session, &window, "third", 40.0);
        for slot in [&first, &second, &third] {
            slot.stamp();
        }
        session.begin_frame();
        session.select_all();

        // The first participant scrolled out of view, so the next frame paints
        // only the other two — with stamps that would otherwise collide with
        // the order of the frame before.
        session.begin_frame();
        let ranges = [&second, &third].map(|slot| {
            slot.stamp();
            slot.selected_range()
        });

        assert_eq!(ranges, [Some(0..6), Some(0..5)]);
        assert_eq!(first.selected_range(), Some(0..5));
        assert_eq!(session.selected_text(), "first\nsecond\nthird");
    }

    #[test]
    fn reversed_drag_produces_the_same_ranges() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        let (_second_geometry, second) = participant(&session, &window, "second", 20.0);
        let (_third_geometry, third) = participant(&session, &window, "third", 40.0);
        for slot in [&first, &second, &third] {
            slot.stamp();
        }
        session.begin_frame();

        session.begin(SelectionPoint::new(Rc::clone(&third), 3), mouse());
        assert!(session.extend_to_position(25.0, 5.0, mouse()));

        assert_eq!(first.selected_range(), Some(3..5));
        assert_eq!(second.selected_range(), Some(0..6));
        assert_eq!(third.selected_range(), Some(0..3));
    }

    #[test]
    fn document_order_follows_the_draw_stamp_not_registration() {
        let window = window();
        let session = session(&window);
        let (_a_geometry, a) = participant(&session, &window, "aa", 0.0);
        let (_b_geometry, b) = participant(&session, &window, "bb", 20.0);
        let (_c_geometry, c) = participant(&session, &window, "cc", 40.0);
        for slot in [&c, &b, &a] {
            slot.stamp();
        }
        session.begin_frame();

        session.select_all();

        assert_eq!(session.selected_text(), "cc\nbb\naa");
    }

    #[test]
    fn extending_into_the_gap_between_participants_picks_the_nearer_one() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        let (_second_geometry, second) = participant(&session, &window, "second", 20.0);
        for slot in [&first, &second] {
            slot.stamp();
        }
        session.begin_frame();

        session.begin(SelectionPoint::new(Rc::clone(&first), 0), mouse());
        assert!(session.extend_to_position(1000.0, 12.0, mouse()));

        assert_eq!(first.selected_range(), Some(0..5));
        assert_eq!(second.selected_range(), None);
    }

    #[test]
    fn extending_outside_the_region_clamps_to_the_first_or_last_participant() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        let (_second_geometry, second) = participant(&session, &window, "second", 20.0);
        for slot in [&first, &second] {
            slot.stamp();
        }
        session.begin_frame();

        session.begin(SelectionPoint::new(Rc::clone(&second), 3), mouse());
        assert!(session.extend_to_position(-500.0, -500.0, mouse()));
        assert_eq!(first.selected_range(), Some(0..5));

        session.begin(SelectionPoint::new(Rc::clone(&first), 1), mouse());
        assert!(session.extend_to_position(500.0, 500.0, mouse()));
        assert_eq!(first.selected_range(), Some(1..5));
        assert_eq!(second.selected_range(), Some(0..6));
    }

    #[test]
    fn only_the_pointer_that_started_the_gesture_extends_it() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        let (_second_geometry, second) = participant(&session, &window, "second", 20.0);
        for slot in [&first, &second] {
            slot.stamp();
        }
        session.begin_frame();

        session.begin(SelectionPoint::new(Rc::clone(&first), 0), mouse());
        let other_finger = PointerKey::new(PointerSource::Touch, 0);

        assert!(!session.extend_to_position(25.0, 25.0, other_finger));
        assert_eq!(second.selected_range(), None);
        assert!(session.extend_to_position(25.0, 25.0, mouse()));
        assert_eq!(second.selected_range(), Some(0..3));
    }

    #[test]
    fn cancel_restores_the_multi_widget_selection_from_before_pointer_down() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        let (_second_geometry, second) = participant(&session, &window, "second", 20.0);
        for slot in [&first, &second] {
            slot.stamp();
        }
        session.begin_frame();
        session.select_all();

        session.begin(SelectionPoint::new(Rc::clone(&second), 1), mouse());
        assert!(session.extend_to_position(25.0, 25.0, mouse()));
        session.cancel();

        assert_eq!(first.selected_range(), Some(0..5));
        assert_eq!(second.selected_range(), Some(0..6));
        assert!(!session.was_dragged());
        assert_eq!(session.active_pointer(), None);
    }

    #[test]
    fn ending_a_gesture_commits_it() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        first.stamp();
        session.begin_frame();

        session.begin(SelectionPoint::new(Rc::clone(&first), 1), mouse());
        assert!(session.extend_to_position(35.0, 5.0, mouse()));
        assert!(session.end(mouse()));
        session.cancel();

        assert_eq!(first.selected_range(), Some(1..4));
    }

    #[test]
    fn clear_drops_the_selection_and_the_focus() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        first.stamp();
        session.begin_frame();
        session.select_all();
        assert!(session.is_focused());

        session.clear();

        assert_eq!(first.selected_range(), None);
        assert!(!session.is_focused());
        assert_eq!(session.selected_text(), "");
    }

    #[test]
    fn a_dropped_participant_leaves_the_selection_usable() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        let (second_geometry, second) = participant(&session, &window, "second", 20.0);
        let (_third_geometry, third) = participant(&session, &window, "third", 40.0);
        for slot in [&first, &second, &third] {
            slot.stamp();
        }
        session.begin_frame();
        session.select_all();

        drop(second);
        drop(second_geometry);

        assert_eq!(session.selected_text(), "first\nthird");
        assert_eq!(first.selected_range(), Some(0..5));
        assert_eq!(third.selected_range(), Some(0..5));
    }

    #[test]
    fn a_participant_that_never_painted_still_contributes_its_text() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        let (_hidden_geometry, hidden) = participant(&session, &window, "hidden", 20.0);
        first.stamp();
        session.begin_frame();

        session.select_all();

        assert_eq!(hidden.selected_range(), Some(0..6));
        assert_eq!(session.selected_text(), "first\nhidden");
    }

    #[test]
    fn rebuilding_a_participant_with_shorter_text_clamps_the_selection() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "second", 0.0);
        first.stamp();
        session.begin_frame();
        session.select_all();
        assert_eq!(first.selected_range(), Some(0..6));

        first.set_text(Rc::from("ab"));

        assert_eq!(first.selected_range(), Some(0..2));
        assert_eq!(session.selected_text(), "ab");
    }

    #[test]
    fn select_all_covers_every_participant() {
        let window = window();
        let session = session(&window);
        let (_first_geometry, first) = participant(&session, &window, "first", 0.0);
        let (_second_geometry, second) = participant(&session, &window, "", 20.0);
        let (_third_geometry, third) = participant(&session, &window, "third", 40.0);
        for slot in [&first, &second, &third] {
            slot.stamp();
        }
        session.begin_frame();

        session.select_all();

        assert_eq!(first.selected_range(), Some(0..5));
        assert_eq!(second.selected_range(), Some(0..0));
        assert_eq!(third.selected_range(), Some(0..5));
        assert_eq!(session.selected_text(), "first\n\nthird");
    }

    #[test]
    fn a_second_session_claiming_clears_the_first_in_both_directions() {
        let window = window();
        let coordinator = Rc::new(SelectionCoordinator::default());
        let region = SelectionSession::new(window.clone(), coordinator.clone(), SELECTION_COLOR);
        let standalone =
            SelectionSession::new(window.clone(), coordinator.clone(), SELECTION_COLOR);
        let (_region_geometry, region_slot) = participant(&region, &window, "region", 0.0);
        let (_alone_geometry, alone_slot) = participant(&standalone, &window, "alone", 40.0);
        region_slot.stamp();
        alone_slot.stamp();
        region.begin_frame();
        standalone.begin_frame();

        region.select_all();
        assert_eq!(region_slot.selected_range(), Some(0..6));
        standalone.select_all();

        assert_eq!(region_slot.selected_range(), None);
        assert!(!region.is_focused());
        assert_eq!(alone_slot.selected_range(), Some(0..5));
        assert!(Rc::ptr_eq(
            &coordinator
                .current()
                .expect("a session holds the selection"),
            &standalone
        ));

        region.select_all();

        assert_eq!(alone_slot.selected_range(), None);
        assert_eq!(region_slot.selected_range(), Some(0..6));
    }
}
