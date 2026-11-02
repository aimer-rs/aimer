//! The clipboard verbs a text field offers where there is no keyboard.
//!
//! `Cmd`/`Ctrl` + `A`/`C`/`X`/`V` covers a desktop, and covers nothing at all on
//! a phone: a soft keyboard has no modifier row, so on touch the only way to
//! reach cut, copy, paste and select-all is the menu every mobile system raises
//! from a long press. The same verbs are what a right-click asks for on a
//! desktop, so both gestures land here and differ only in the shape
//! [`aimer_ctxmenu`] draws them in.
//!
//! Two pieces, both pure: which verbs are worth offering right now, and whether
//! a press has become a hold. Neither touches a canvas, a window or a clock —
//! the instant is handed in — so both are asserted directly.

use std::cell::Cell;

use aimer_animation::AnimInstant;
use aimer_attribute::position::Vec2d;
use aimer_ctxmenu::ContextMenuItem;
use aimer_events::pointer::PointerSource;
use aimer_widget::PointerKey;

use crate::gesture::{LONG_PRESS_DURATION, tap_slop};

/// A verb one row of a field's context menu runs.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(FieldAction::SelectAll.label(), "Select All");
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FieldAction {
    /// Removes the selection and puts it on the clipboard.
    Cut,
    /// Puts the selection on the clipboard.
    Copy,
    /// Replaces the selection with the clipboard's text.
    Paste,
    /// Selects the whole field.
    SelectAll,
}

impl FieldAction {
    /// The label painted for this verb.
    #[inline]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::SelectAll => "Select All",
        }
    }
}

/// What a field can do at the moment its menu is asked for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FieldMenuState {
    /// Whether anything is selected.
    pub has_selection: bool,
    /// Whether the field holds any text at all.
    pub has_text: bool,
    /// Whether the selection already covers the whole field.
    pub all_selected: bool,
    /// Whether the field refuses edits.
    pub read_only: bool,
    /// Whether the clipboard has something to paste.
    pub clipboard_has_text: bool,
}

/// The verbs worth offering for `state`, in the order every platform lists
/// them.
///
/// Verbs that would do nothing are left out rather than shown greyed: the pill
/// a finger gets is only a few rows wide, and a row that can never be chosen
/// spends width that the ones that can need.
pub(crate) fn actions_for(state: FieldMenuState) -> Vec<FieldAction> {
    let mut actions = Vec::with_capacity(4);
    if state.has_selection {
        if !state.read_only {
            actions.push(FieldAction::Cut);
        }
        actions.push(FieldAction::Copy);
    }
    if !state.read_only && state.clipboard_has_text {
        actions.push(FieldAction::Paste);
    }
    if state.has_text && !state.all_selected {
        actions.push(FieldAction::SelectAll);
    }
    actions
}

/// The menu rows for `actions`.
#[inline]
pub(crate) fn items(actions: &[FieldAction]) -> Vec<ContextMenuItem> {
    actions
        .iter()
        .map(|action| ContextMenuItem::new(action.label()))
        .collect()
}

/// What offering a pointer event to a [`TouchHold`] meant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HoldOutcome {
    /// No press of this pointer is being watched.
    Idle,
    /// The press is still a candidate.
    Waiting,
    /// The finger wandered, so this is a drag and never a hold.
    Abandoned,
    /// The finger rested long enough: this is a hold.
    Held,
}

#[derive(Clone, Copy)]
struct PendingPress {
    pointer: PointerKey,
    origin: Vec2d,
    down_at: AnimInstant,
    source: PointerSource,
}

/// Watches one finger for a press that turns into a hold.
///
/// Only touch pointers are watched: a mouse asks for a menu by its secondary
/// button, and holding a mouse button still means drag-to-select.
///
/// There is no timer to wake the gate mid-hold, so the threshold is tested
/// against the events that do arrive — a move, or the release. A finger held
/// perfectly still therefore raises the menu when it lifts, which is exactly
/// when the menu could be used anyway.
#[derive(Default)]
pub(crate) struct TouchHold {
    pending: Cell<Option<PendingPress>>,
}

impl TouchHold {
    /// A gate watching nothing.
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            pending: Cell::new(None),
        }
    }

    /// Begins watching a press, reporting whether it is a candidate at all.
    pub(crate) fn press(
        &self,
        pointer: PointerKey,
        source: PointerSource,
        origin: Vec2d,
        now: AnimInstant,
    ) -> HoldOutcome {
        if source == PointerSource::Mouse {
            self.pending.set(None);
            return HoldOutcome::Idle;
        }
        self.pending.set(Some(PendingPress {
            pointer,
            origin,
            down_at: now,
            source,
        }));
        HoldOutcome::Waiting
    }

    /// Offers a move to the watched press.
    ///
    /// A finger that wanders beyond the slop of its device is dragging, and a
    /// drag is never a hold — but one that has *already* rested long enough is
    /// a hold the moment it is noticed, so a slow finger does not have to lift
    /// to reach the menu.
    pub(crate) fn moved(&self, pointer: PointerKey, pos: Vec2d, now: AnimInstant) -> HoldOutcome {
        let Some(press) = self.pending.get().filter(|press| press.pointer == pointer) else {
            return HoldOutcome::Idle;
        };
        if wandered(press, pos) {
            self.pending.set(None);
            return HoldOutcome::Abandoned;
        }
        if rested(press, now) {
            self.pending.set(None);
            return HoldOutcome::Held;
        }
        HoldOutcome::Waiting
    }

    /// Offers a release to the watched press.
    pub(crate) fn release(&self, pointer: PointerKey, pos: Vec2d, now: AnimInstant) -> HoldOutcome {
        let Some(press) = self.pending.get().filter(|press| press.pointer == pointer) else {
            return HoldOutcome::Idle;
        };
        self.pending.set(None);
        if wandered(press, pos) {
            HoldOutcome::Abandoned
        } else if rested(press, now) {
            HoldOutcome::Held
        } else {
            HoldOutcome::Abandoned
        }
    }

    /// Forgets the watched press, whatever it was about to become.
    #[inline]
    pub(crate) fn forget(&self) {
        self.pending.set(None);
    }
}

#[inline]
fn wandered(press: PendingPress, pos: Vec2d) -> bool {
    let slop = tap_slop(press.source);
    (pos.x - press.origin.x).abs() > slop || (pos.y - press.origin.y).abs() > slop
}

#[inline]
fn rested(press: PendingPress, now: AnimInstant) -> bool {
    now.duration_since(press.down_at) >= LONG_PRESS_DURATION
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn state() -> FieldMenuState {
        FieldMenuState {
            has_text: true,
            clipboard_has_text: true,
            ..Default::default()
        }
    }

    fn pointer() -> PointerKey {
        PointerKey::new(PointerSource::Touch, 1)
    }

    fn at(x: f32, y: f32) -> Vec2d {
        Vec2d { x, y }
    }

    #[test]
    fn a_field_with_no_selection_offers_paste_and_select_all() {
        assert_eq!(
            actions_for(state()),
            vec![FieldAction::Paste, FieldAction::SelectAll]
        );
    }

    #[test]
    fn a_selection_brings_cut_and_copy_in_front() {
        assert_eq!(
            actions_for(FieldMenuState {
                has_selection: true,
                ..state()
            }),
            vec![
                FieldAction::Cut,
                FieldAction::Copy,
                FieldAction::Paste,
                FieldAction::SelectAll
            ]
        );
    }

    #[test]
    fn a_read_only_field_offers_only_the_verbs_that_read() {
        assert_eq!(
            actions_for(FieldMenuState {
                has_selection: true,
                read_only: true,
                ..state()
            }),
            vec![FieldAction::Copy, FieldAction::SelectAll]
        );
    }

    #[test]
    fn an_empty_clipboard_offers_no_paste() {
        assert_eq!(
            actions_for(FieldMenuState {
                clipboard_has_text: false,
                ..state()
            }),
            vec![FieldAction::SelectAll]
        );
    }

    #[test]
    fn a_field_already_selected_whole_offers_no_select_all() {
        assert_eq!(
            actions_for(FieldMenuState {
                has_selection: true,
                all_selected: true,
                ..state()
            }),
            vec![FieldAction::Cut, FieldAction::Copy, FieldAction::Paste]
        );
    }

    #[test]
    fn an_empty_read_only_field_offers_nothing() {
        assert!(
            actions_for(FieldMenuState {
                read_only: true,
                ..Default::default()
            })
            .is_empty()
        );
    }

    #[test]
    fn the_rows_are_labelled_after_their_verbs() {
        let rows = items(&actions_for(state()));

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label(), "Paste");
        assert_eq!(rows[1].label(), "Select All");
        assert!(rows.iter().all(|row| row.is_enabled()));
    }

    #[test]
    fn a_finger_resting_long_enough_is_a_hold() {
        let gate = TouchHold::new();
        let now = AnimInstant::now();

        assert_eq!(
            gate.press(pointer(), PointerSource::Touch, at(10.0, 10.0), now),
            HoldOutcome::Waiting
        );
        assert_eq!(
            gate.release(pointer(), at(11.0, 10.0), now + LONG_PRESS_DURATION),
            HoldOutcome::Held
        );
    }

    #[test]
    fn a_finger_lifted_too_soon_is_a_tap_and_not_a_hold() {
        let gate = TouchHold::new();
        let now = AnimInstant::now();

        gate.press(pointer(), PointerSource::Touch, at(10.0, 10.0), now);

        assert_eq!(
            gate.release(pointer(), at(10.0, 10.0), now + Duration::from_millis(80)),
            HoldOutcome::Abandoned
        );
    }

    #[test]
    fn a_finger_that_wandered_is_a_drag_and_never_a_hold() {
        let gate = TouchHold::new();
        let now = AnimInstant::now();
        gate.press(pointer(), PointerSource::Touch, at(10.0, 10.0), now);

        assert_eq!(
            gate.moved(pointer(), at(200.0, 10.0), now + Duration::from_millis(10)),
            HoldOutcome::Abandoned
        );
        assert_eq!(
            gate.release(pointer(), at(200.0, 10.0), now + LONG_PRESS_DURATION),
            HoldOutcome::Idle,
            "the press is forgotten, so its release means nothing"
        );
    }

    #[test]
    fn a_slow_finger_reaches_the_menu_without_lifting() {
        let gate = TouchHold::new();
        let now = AnimInstant::now();
        gate.press(pointer(), PointerSource::Touch, at(10.0, 10.0), now);

        assert_eq!(
            gate.moved(pointer(), at(11.0, 11.0), now + LONG_PRESS_DURATION),
            HoldOutcome::Held
        );
    }

    #[test]
    fn only_the_finger_that_pressed_can_hold() {
        let gate = TouchHold::new();
        let now = AnimInstant::now();
        gate.press(pointer(), PointerSource::Touch, at(10.0, 10.0), now);

        let other = PointerKey::new(PointerSource::Touch, 2);

        assert_eq!(
            gate.release(other, at(10.0, 10.0), now + LONG_PRESS_DURATION),
            HoldOutcome::Idle
        );
        assert_eq!(
            gate.release(pointer(), at(10.0, 10.0), now + LONG_PRESS_DURATION),
            HoldOutcome::Held,
            "and the finger that did press is still watched"
        );
    }

    #[test]
    fn a_mouse_is_never_watched_for_a_hold() {
        let gate = TouchHold::new();
        let now = AnimInstant::now();
        let mouse = PointerKey::new(PointerSource::Mouse, 0);

        assert_eq!(
            gate.press(mouse, PointerSource::Mouse, at(10.0, 10.0), now),
            HoldOutcome::Idle
        );
        assert_eq!(
            gate.release(mouse, at(10.0, 10.0), now + LONG_PRESS_DURATION),
            HoldOutcome::Idle
        );
    }

    #[test]
    fn a_forgotten_press_can_no_longer_hold() {
        let gate = TouchHold::new();
        let now = AnimInstant::now();
        gate.press(pointer(), PointerSource::Touch, at(10.0, 10.0), now);

        gate.forget();

        assert_eq!(
            gate.release(pointer(), at(10.0, 10.0), now + LONG_PRESS_DURATION),
            HoldOutcome::Idle
        );
    }
}
