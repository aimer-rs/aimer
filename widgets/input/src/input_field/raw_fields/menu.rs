//! The clipboard menu of a text field: asking for one, raising it, running it.
//!
//! Kept beside [`RawTextField`] rather than inside it because the field is
//! already the largest element in the crate, and because none of this needs the
//! rest of it: a menu is a request, an anchor and a verb.

use std::rc::Rc;

use aimer_attribute::bounds::Bounds;
use aimer_attribute::position::Vec2d;
use aimer_ctxmenu::{ContextMenu, ContextMenuShape};

use super::{MenuOrigin, RawTextField, clipboard_read, clipboard_write};
use crate::input_field::context_menu::{FieldAction, FieldMenuState, actions_for, items};

impl RawTextField {
    /// Asks for a context menu, to be raised by the next frame.
    ///
    /// The verbs depend on what is selected, and what is selected is only known
    /// once the next frame's pre-paint sync has turned a deferred click into a
    /// caret offset — the field has no canvas to measure with while handling
    /// an event. Requesting here and raising there is what lets a hold over a
    /// word offer `Copy` rather than only `Paste`.
    pub(super) fn request_menu(&self, origin: MenuOrigin) {
        self.pending_menu.set(Some(origin));
    }

    /// What the field can do right now.
    pub(super) fn menu_state(&self) -> FieldMenuState {
        let count = self.controller.grapheme_count();
        let selection = self.cursor.selection_range();
        FieldMenuState {
            has_selection: selection.is_some(),
            has_text: count > 0,
            all_selected: matches!(selection, Some((0, end)) if end >= count),
            read_only: self.read_only,
            clipboard_has_text: clipboard_read().is_some_and(|text| !text.is_empty()),
        }
    }

    /// Raises the menu asked for by [`RawTextField::request_menu`], if any.
    ///
    /// A menu with nothing worth offering — an empty, read-only field with an
    /// empty clipboard — is not raised at all, rather than shown as an empty
    /// panel the user has to dismiss.
    pub(super) fn raise_pending_menu(&self) {
        let Some(origin) = self.pending_menu.take() else {
            return;
        };
        let actions = actions_for(self.menu_state());
        if actions.is_empty() {
            self.close_menu();
            return;
        }

        let (shape, anchor) = match origin {
            MenuOrigin::Click(pos) => (ContextMenuShape::List, Bounds::new(pos.x, pos.y, 0.0, 0.0)),
            MenuOrigin::Hold => match self.cached_bounds.get_bounds() {
                Some(bounds) => (ContextMenuShape::Pill, bounds),
                None => return,
            },
        };

        *self.menu_actions.borrow_mut() = actions.clone();
        self.menu_origin.set(Some(origin));

        // The menu cannot reach back into the element — it holds no reference
        // to it and outlives no part of it — so a choice is recorded here and
        // applied by the next pre-paint sync, which is also where a deferred
        // click becomes a caret offset.
        let chosen = Rc::clone(&self.chosen_action);
        let rows = Rc::clone(&self.menu_actions);
        self.close_menu();
        let handle = ContextMenu::new()
            .shape(shape)
            .around(anchor)
            .items(items(&actions))
            // The verb decides: everything but `Select All` finishes the job.
            .dismiss_on_select(false)
            .on_select(move |index| {
                chosen.set(rows.borrow().get(index).copied());
            })
            .show();
        *self.menu.borrow_mut() = Some(handle);
        self.menu_shape.set(Some(shape));
    }

    /// Runs the verb the open menu was told to run, if any.
    pub(super) fn apply_chosen_action(&self) {
        if let Some(action) = self.chosen_action.take() {
            self.run_menu_action(action);
        }
    }

    /// Closes the open menu, if there is one.
    ///
    /// Repeated calls are harmless, which is what lets every dismissal path
    /// call it without asking first.
    pub(super) fn close_menu(&self) {
        if let Some(handle) = self.menu.borrow_mut().take() {
            handle.dismiss();
        }
        self.menu_shape.set(None);
    }

    /// Whether the field's menu is showing.
    ///
    /// The registry is asked rather than the handle alone, because the barrier
    /// and `Escape` close a menu without telling its owner.
    #[inline]
    pub(super) fn menu_is_open(&self) -> bool {
        self.menu
            .borrow()
            .as_ref()
            .is_some_and(|handle| handle.is_showing())
    }

    /// Runs a verb chosen from the menu.
    ///
    /// Every verb but `Select All` finishes the job and closes the menu.
    /// `Select All` only reshapes what the menu acts on, so the menu is offered
    /// again in the same place — now with `Cut` and `Copy` in it, which is what
    /// the user reached for it to do.
    pub(super) fn run_menu_action(&self, action: FieldAction) {
        match action {
            FieldAction::Cut if !self.read_only && !self.input_type.is_obscured() => {
                if let Some((start, end)) = self.cursor.selection_range() {
                    let removed = self.controller.get_range(start, end);
                    clipboard_write(&removed);
                    self.replace_cursor_selection("", None);
                }
                self.close_menu();
            }
            FieldAction::Copy if !self.input_type.is_obscured() => {
                if let Some((start, end)) = self.cursor.selection_range() {
                    clipboard_write(&self.controller.get_range(start, end));
                }
                self.close_menu();
            }
            FieldAction::Paste if !self.read_only => {
                if let Some(text) = clipboard_read() {
                    self.insert_text(&text);
                }
                self.close_menu();
            }
            FieldAction::SelectAll => {
                self.cursor.set_selection_anchor(Some(0));
                self.cursor.set_offset(self.controller.grapheme_count());
                if let Some(origin) = self.menu_origin.get() {
                    self.request_menu(origin);
                } else {
                    self.close_menu();
                }
            }
            _ => self.close_menu(),
        }
        self.sync_selection_to_controller();
        self.cursor.reset_blink();
    }

    /// Closes the menu and forgets any press that was about to raise one.
    ///
    /// Typing, losing focus or a cancelled gesture all invalidate a menu built
    /// around a selection that no longer means what it did.
    pub(super) fn dismiss_menu(&self) {
        self.pending_menu.set(None);
        self.menu_origin.set(None);
        self.touch_hold.forget();
        self.close_menu();
    }

    /// Selects the word under a deferred click, resolved by the next frame.
    ///
    /// Reuses the double-click path: the offset under a position is only
    /// reachable with a canvas, and pre-paint sync resolves one for drawing.
    pub(super) fn select_word_under(&self, pos: Vec2d) {
        self.pending_click.set(Some(pos));
        self.click_count.set(2);
    }
}

#[cfg(test)]
mod tests {
    //! The field's clipboard menu, driven through the element itself.
    //!
    //! What a menu offers depends on a selection, and a selection depends on a
    //! canvas the field only has while painting, so these tests do what `draw`
    //! does: fill the cached bounds, drive the gesture, then raise the pending
    //! menu. Nothing here touches the clipboard except through
    //! [`super::RawTextField::menu_state`], and every assertion is about verbs the
    //! clipboard cannot take away.

    use std::time::Duration;

    use aimer_animation::AnimInstant;
    use aimer_attribute::position::Vec2d;
    use aimer_ctxmenu::ContextMenuShape;
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::{Drawable, EventElement};

    use super::super::test_support::{dummy_build_context, focused_field};
    use super::{FieldAction, MenuOrigin, RawTextField};
    use crate::gesture::LONG_PRESS_DURATION;
    use crate::input_field::controller::TextFieldController;

    /// A field holding `text`, painted once at a known place.
    fn field(text: &str) -> RawTextField {
        let controller = TextFieldController::new();
        controller.set_text(text.to_owned());
        let field = focused_field(controller);
        field.cached_bounds.save(1.0, 0.0, 0.0, 200.0, 32.0);
        field.test_clock.set(Some(AnimInstant::now()));
        field
    }

    fn advance(field: &RawTextField, by: Duration) {
        let now = field.test_clock.get().expect("a fixed clock");
        field.test_clock.set(Some(now + by));
    }

    fn press(source: PointerSource, button: PointerButton, x: f32, y: f32) -> ElementEvent {
        ElementEvent::PointerDown(PointerInfo::new(Vec2d { x, y }, source, 0, button))
    }

    fn release(source: PointerSource, x: f32, y: f32) -> ElementEvent {
        ElementEvent::PointerUp(PointerInfo::new(
            Vec2d { x, y },
            source,
            0,
            PointerButton::Primary,
        ))
    }

    fn verbs(field: &RawTextField) -> Vec<FieldAction> {
        field.menu_actions.borrow().clone()
    }

    #[test]
    fn a_finger_that_rests_raises_the_pill() {
        let field = field("hello world");
        let _ = field.on_event(&press(
            PointerSource::Touch,
            PointerButton::Primary,
            20.0,
            16.0,
        ));
        advance(&field, LONG_PRESS_DURATION);

        assert!(
            field
                .on_event(&release(PointerSource::Touch, 20.0, 16.0))
                .is_consumed()
        );
        field.raise_pending_menu();

        assert!(field.menu_is_open());
        assert_eq!(field.menu_shape.get(), Some(ContextMenuShape::Pill));
    }

    #[test]
    fn a_finger_that_taps_raises_nothing() {
        let field = field("hello world");
        let _ = field.on_event(&press(
            PointerSource::Touch,
            PointerButton::Primary,
            20.0,
            16.0,
        ));
        advance(&field, Duration::from_millis(60));

        let _ = field.on_event(&release(PointerSource::Touch, 20.0, 16.0));
        field.raise_pending_menu();

        assert!(!field.menu_is_open());
    }

    #[test]
    fn a_finger_that_drags_raises_nothing() {
        let field = field("hello world");
        let _ = field.on_event(&press(
            PointerSource::Touch,
            PointerButton::Primary,
            20.0,
            16.0,
        ));
        let _ = field.on_event(&ElementEvent::PointerMove(PointerInfo::new(
            Vec2d { x: 180.0, y: 16.0 },
            PointerSource::Touch,
            0,
            PointerButton::Primary,
        )));
        advance(&field, LONG_PRESS_DURATION);

        let _ = field.on_event(&release(PointerSource::Touch, 180.0, 16.0));
        field.raise_pending_menu();

        assert!(!field.menu_is_open(), "a drag is a selection, not a menu");
    }

    #[test]
    fn a_right_click_opens_the_desktop_list_where_it_landed() {
        let field = field("hello world");

        assert!(
            field
                .on_event(&press(
                    PointerSource::Mouse,
                    PointerButton::Secondary,
                    20.0,
                    16.0
                ))
                .is_consumed()
        );
        field.raise_pending_menu();

        assert!(field.menu_is_open());
        assert_eq!(field.menu_shape.get(), Some(ContextMenuShape::List));
        assert_eq!(
            field
                .menu_origin
                .get()
                .map(|origin| matches!(origin, MenuOrigin::Click(_))),
            Some(true),
            "opened at the click"
        );
    }

    #[test]
    fn a_right_click_does_not_start_a_drag_selection() {
        let field = field("hello world");
        let _ = field.on_event(&press(
            PointerSource::Mouse,
            PointerButton::Secondary,
            20.0,
            16.0,
        ));

        assert!(field.mouse_held.get().is_none());
    }

    #[test]
    fn a_selection_puts_cut_and_copy_in_the_menu() {
        let field = field("hello world");
        field.cursor.set_selection_anchor(Some(0));
        field.cursor.set_offset(5);

        field.request_menu(MenuOrigin::Hold);
        field.raise_pending_menu();

        let verbs = verbs(&field);
        assert_eq!(verbs.first(), Some(&FieldAction::Cut));
        assert_eq!(verbs.get(1), Some(&FieldAction::Copy));
    }

    #[test]
    fn select_all_from_the_menu_offers_it_again_with_copy_in_it() {
        let field = field("hello world");
        field.request_menu(MenuOrigin::Hold);
        field.raise_pending_menu();
        assert!(!verbs(&field).contains(&FieldAction::Copy));

        field.run_menu_action(FieldAction::SelectAll);
        field.raise_pending_menu();

        assert_eq!(
            field.cursor.selection_range(),
            Some((0, field.controller.grapheme_count()))
        );
        assert_eq!(
            field.controller.selection_graphemes(),
            (0, field.controller.grapheme_count())
        );
        assert!(field.menu_is_open(), "the menu stays, reshaped");
        assert!(verbs(&field).contains(&FieldAction::Copy));
    }

    #[test]
    fn retained_prepaint_sync_applies_a_queued_menu_action() {
        let field = field("hello world");
        let context = dummy_build_context(200.0, 32.0);
        let end = field.controller.grapheme_count();
        field.chosen_action.set(Some(FieldAction::SelectAll));

        assert!(field.sync_local_v2_state(&context));

        assert_eq!(field.cursor.selection_range(), Some((0, end)));
        assert_eq!(field.controller.selection_graphemes(), (0, end));
        assert_eq!(field.chosen_action.get(), None);
    }

    #[test]
    fn an_empty_read_only_field_raises_no_menu_at_all() {
        let controller = TextFieldController::new();
        let mut field = focused_field(controller);
        field.read_only = true;
        field.cached_bounds.save(1.0, 0.0, 0.0, 200.0, 32.0);

        field.request_menu(MenuOrigin::Hold);
        field.raise_pending_menu();

        assert!(!field.menu_is_open());
    }

    #[test]
    fn typing_dismisses_the_menu() {
        let field = field("hello world");
        field.request_menu(MenuOrigin::Hold);
        field.raise_pending_menu();
        assert!(field.menu_is_open());

        let _ = field.on_event(&super::super::test_support::commit("x"));

        assert!(!field.menu_is_open());
    }

    #[test]
    fn a_press_outside_the_field_dismisses_the_menu() {
        let field = field("hello world");
        field.request_menu(MenuOrigin::Hold);
        field.raise_pending_menu();

        let _ = field.on_event(&press(
            PointerSource::Mouse,
            PointerButton::Primary,
            400.0,
            400.0,
        ));

        assert!(!field.menu_is_open());
    }

    #[test]
    fn a_cancelled_gesture_forgets_the_press_that_would_raise_a_menu() {
        let field = field("hello world");
        let _ = field.on_event(&press(
            PointerSource::Touch,
            PointerButton::Primary,
            20.0,
            16.0,
        ));

        let _ = field.on_event(&ElementEvent::Cancel);
        advance(&field, LONG_PRESS_DURATION);
        let _ = field.on_event(&release(PointerSource::Touch, 20.0, 16.0));
        field.raise_pending_menu();

        assert!(!field.menu_is_open(), "the press is gone, so is its menu");
        assert!(
            field.is_focused(),
            "cancelling a pointer gesture does not relinquish keyboard focus"
        );
    }

    #[test]
    fn the_cancel_the_menu_itself_causes_leaves_it_open() {
        let field = field("hello world");
        let _ = field.on_event(&press(
            PointerSource::Mouse,
            PointerButton::Secondary,
            20.0,
            16.0,
        ));
        field.raise_pending_menu();
        assert!(field.menu_is_open());

        // What the modal host broadcasts one frame after presenting a modal: the
        // gestures underneath a barrier are cancelled — and the field's own menu is
        // that barrier.
        let _ = field.on_event(&ElementEvent::Cancel);

        assert!(field.menu_is_open(), "a menu may not cancel itself");
        assert!(
            field.is_focused(),
            "the selection the menu acts on has to survive it"
        );
    }

    #[test]
    fn the_placeholder_hides_while_an_input_method_composes() {
        let field = field("");

        assert!(field.placeholder_visible(), "an idle empty field shows it");

        let _ = field.on_event(&ElementEvent::ImePreedit {
            text: "ni".to_owned(),
            cursor: Some((2, 2)),
        });

        assert!(
            !field.placeholder_visible(),
            "the composition is the field's content now"
        );

        let _ = field.on_event(&ElementEvent::ImePreedit {
            text: String::new(),
            cursor: None,
        });

        assert!(field.placeholder_visible(), "and comes back when it ends");
    }
}
