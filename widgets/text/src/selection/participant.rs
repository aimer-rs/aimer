use std::ops::Range;
use std::rc::Rc;

use aimer_attribute::{Bounds, Vec2d};
use aimer_events::element::ElementEvent;
use aimer_events::pointer::PointerSource;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{EventResult, PointerKey};

use super::SelectionRegion;
use crate::selection::selectable::{Selectable, SelectionBinding, SelectionScope, TextGeometry};
use crate::selection::session::{SelectionSession, SelectionSlot};
use crate::selection::ui;

/// A widget-owned participant in the ambient [`crate::SelectionArea`].
///
/// The participant is deliberately small: widgets provide their painted hit
/// regions, while this type owns the shared selection session, selection
/// handles, and cross-widget drag state. Editable widgets can mirror the
/// selected byte range into their own grapheme-based editing controller.
#[derive(Clone)]
pub struct SelectionParticipant {
    geometry: Rc<TextGeometry>,
    session: Rc<SelectionSession>,
    slot: Rc<SelectionSlot>,
}

impl SelectionParticipant {
    /// Registers a participant with the `SelectionArea` in `ctx`.
    ///
    /// Returns `None` when the widget is not inside a selection area. This
    /// keeps the opt-in behavior of ordinary text while allowing editable
    /// widgets to use the same selection implementation when wrapped.
    pub fn from_context(ctx: &BuildContext, text: Rc<str>) -> Option<Self> {
        let scope = ctx.get_state::<SelectionScope>()?;
        let session = Rc::clone(&scope.0);
        let geometry = Rc::new(TextGeometry::new(ctx.window.clone()));
        let slot = session.register(text, Rc::downgrade(&geometry) as _);
        Some(Self {
            geometry,
            session,
            slot,
        })
    }

    /// Creates a participant using the existing text-selection binding.
    pub(crate) fn from_binding(binding: &SelectionBinding) -> Self {
        Self {
            geometry: Rc::clone(&binding.geometry),
            session: Rc::clone(&binding.session),
            slot: Rc::clone(&binding.slot),
        }
    }

    /// Carries the participant registration across a rebuilt element.
    pub fn adopt(&self, text: Rc<str>) -> Self {
        let binding = SelectionBinding {
            geometry: Rc::clone(&self.geometry),
            session: Rc::clone(&self.session),
            slot: Rc::clone(&self.slot),
            owns_session: false,
        };
        Self::from_binding(&binding.adopt(text))
    }

    /// The shared highlight color selected by the enclosing area.
    #[inline]
    pub fn selection_color(&self) -> Color {
        self.session.selection_color()
    }

    /// Updates the text snapshot used by copy and select-all operations.
    #[inline]
    pub fn set_text(&self, text: Rc<str>) {
        self.slot.set_text(text);
    }

    /// Records the participant's absolute painted geometry for hit-testing.
    pub fn set_geometry(&self, bounds: Bounds, regions: impl IntoIterator<Item = SelectionRegion>) {
        self.geometry.bounds.set_bounds(bounds);
        self.geometry.set_regions(regions);
        self.slot.stamp();
    }

    /// Records source-aware shaped geometry for selection hit-testing.
    ///
    /// The layout must use the same source byte offsets as the text passed to
    /// [`Self::set_text`]. `transform` maps its local coordinates into the
    /// device-space canvas used by the last paint, and `scale` converts that
    /// device space to the logical coordinates used by pointer events.
    pub fn set_interaction_geometry(
        &self,
        bounds: Bounds,
        layout: Rc<aimer_cupid::text_layout::TextInteractionLayout>,
        transform: aimer_cupid::utilities::Mat3,
        scale: f32,
    ) {
        self.geometry.bounds.set_bounds(bounds);
        self.geometry.regions.borrow_mut().clear();
        self.geometry
            .set_shared_interaction_layout(Some(layout), transform, scale);
        self.slot.stamp();
    }

    /// Returns the selected byte range belonging to this participant.
    #[inline]
    pub fn selected_range(&self) -> Option<Range<usize>> {
        self.slot.selected_range()
    }

    /// Resolves an absolute pointer position to a byte offset.
    #[inline]
    pub fn offset_at(&self, x: f32, y: f32) -> Option<usize> {
        self.geometry.offset_at(x, y)
    }

    /// Reports whether an absolute pointer position is inside the participant.
    #[inline]
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        self.geometry.contains_point(x, y)
    }

    /// Starts a shared selection gesture at `offset`.
    #[inline]
    pub fn begin(&self, offset: usize, pointer: PointerKey) {
        self.session.begin(
            crate::selection::SelectionPoint::new(Rc::clone(&self.slot), offset),
            pointer,
        );
    }

    /// Starts a shared selection gesture with an existing range.
    #[inline]
    pub fn begin_range(&self, anchor: usize, focus: usize, pointer: PointerKey) {
        self.session.begin_range(
            crate::selection::SelectionPoint::new(Rc::clone(&self.slot), anchor),
            crate::selection::SelectionPoint::new(Rc::clone(&self.slot), focus),
            pointer,
        );
    }

    /// Replaces the selection with a range in this participant.
    #[inline]
    pub fn set_range(&self, anchor: usize, focus: usize) {
        self.session.set_local_range(&self.slot, anchor, focus);
    }

    /// Extends the active shared selection to an absolute pointer position.
    #[inline]
    pub fn extend_to_position(&self, x: f32, y: f32, pointer: PointerKey) -> bool {
        self.session.extend_to_position(x, y, pointer)
    }

    /// Ends a shared selection gesture.
    #[inline]
    pub fn end(&self, pointer: PointerKey) -> bool {
        self.session.end(pointer)
    }

    /// Cancels the active shared selection gesture.
    #[inline]
    pub fn cancel(&self) {
        self.session.cancel();
    }

    /// Offers an event to selection handles before the widget handles it.
    #[inline]
    pub fn intercept(&self, event: &ElementEvent) -> Option<EventResult> {
        ui::intercept(&self.session, event)
    }

    /// Opens the shared selection context menu at a secondary click.
    pub fn open_context_menu(&self, pos: Vec2d, pointer: PointerKey) -> bool {
        ui::open_context_menu(&self.session, &self.slot, &self.geometry, pos, pointer)
    }

    /// Offers the shared touch selection menu after a completed gesture.
    #[inline]
    pub fn offer_menu_after_gesture(&self, source: PointerSource) {
        ui::offer_menu_after_gesture(&self.session, source);
    }

    /// Shows the shared touch selection menu.
    #[inline]
    pub fn show_menu(&self) {
        self.session.ui.show_menu();
    }

    /// Returns whether this participant's session currently owns the pointer.
    #[inline]
    pub fn active_pointer(&self) -> Option<PointerKey> {
        self.session.active_pointer()
    }
}
