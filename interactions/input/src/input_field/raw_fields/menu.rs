//! The clipboard menu of a text field: asking for one, raising it, running it.
//!
//! Kept beside [`RawTextField`] rather than inside it because the field is
//! already the largest element in the crate, and because none of this needs the
//! rest of it: a menu is a request, an anchor and a verb.

use std::rc::Rc;

use aimer_attribute::bounds::Bounds;
use aimer_attribute::position::Vec2d;
use aimer_ctxmenu::{ContextMenu, ContextMenuShape, ContextMenuStyle, ModalHandle};

use super::{MenuOrigin, RawTextField, clipboard_read, clipboard_write};
use crate::input_field::context_menu::{ContextMenuExtras, ContextMenuRequest, FieldAction, FieldMenuState, actions_for, in_groups, items};
use crate::input_field::line_labels::FieldSpot;

/// What the open menu and the field that raised it share: the menu's callbacks write here, and the field reads it on its next frame.
///
/// It is one bundle, behind one pointer that a field made again takes from the one before it, so that a row that is chosen after the field was
/// built again is applied by the field that is on screen, and not lost with the one that raised the menu.
#[derive(Default)]
pub(crate) struct MenuShared {
    /// The verbs of the open menu, in the order it draws them: the field's own, and not the rows an app added.
    pub actions: std::cell::RefCell<Vec<FieldAction>>,
    /// How many rows an app added before the verbs of the open menu and after them.
    pub extra_counts: std::cell::Cell<(usize, usize)>,
    /// The verb the open menu was just told to run.
    pub chosen: std::cell::Cell<Option<FieldAction>>,
    /// The open menu, which is a modal: the host places it above every clip and dismisses it on an outside press, so all the field keeps is the
    /// handle that closes it. It is here, and not in the field, because a field that is made again is adopted from more than once, and a
    /// handle that was moved to the first of them would be gone from the others.
    pub handle: std::cell::RefCell<Option<ModalHandle>>,
}

/// What choosing the row `index` of a menu means, where `before` rows of an app come first, then the field's own `verbs`, then the rows of
/// the app that come after. A row of the app has done what it was given by the time this is asked, so it only closes the menu.
pub(super) fn chosen_for(index: usize, before: usize, verbs: &[FieldAction]) -> FieldAction {
    index
        .checked_sub(before)
        .and_then(|verb| verbs.get(verb))
        .copied()
        .unwrap_or(FieldAction::Dismiss)
}

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

    /// What the open menu and this field share.
    pub(crate) fn menu_shared(&self) -> Rc<MenuShared> {
        Rc::clone(&self.menu_shared.borrow())
    }

    /// Takes the menu of the field this one replaces: the menu that is open and what it is told to run, which are shared and not moved, since a
    /// field is adopted from more than once, and the one that was asked for and not yet raised.
    pub(super) fn adopt_menu_from(&self, old: &RawTextField) {
        self.menu_shape.set(old.menu_shape.get());
        self.pending_menu.set(old.pending_menu.get());
        self.menu_origin.set(old.menu_origin.get());
        self.menu_hit_at.set(old.menu_hit_at.get());
        self.menu_hit.set(old.menu_hit.get());
        *self.menu_shared.borrow_mut() = old.menu_shared();
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
        let (anchor_shape, anchor) = match origin {
            MenuOrigin::Click(pos) => (ContextMenuShape::List, Bounds::new(pos.x, pos.y, 0.0, 0.0)),
            MenuOrigin::Hold => match self.cached_bounds.get_bounds() {
                Some(bounds) => (ContextMenuShape::Pill, bounds),
                None => return,
            },
        };
        // What the app adds is asked for here, once the click has become a caret or a selection: that is what it decides by.
        let extras = self
            .context_menu_extras
            .as_ref()
            .map(|extras| extras(&self.menu_request(origin)))
            .unwrap_or_default();
        let ContextMenuExtras { before, after } = extras;
        if actions.is_empty() && before.is_empty() && after.is_empty() {
            self.close_menu();
            return;
        }
        let shape = anchor_shape;

        let shared = self.menu_shared();
        *shared.actions.borrow_mut() = actions.clone();
        shared.extra_counts.set((before.len(), after.len()));
        self.menu_origin.set(Some(origin));

        // The menu cannot reach back into the element — it holds no reference
        // to it and outlives no part of it — so a choice is recorded here and
        // applied by the next pre-paint sync, which is also where a deferred
        // click becomes a caret offset.
        let on_chosen = Rc::clone(&shared);
        self.close_menu();
        let listed = in_groups(before, items(&actions, super::PLATFORM_IS_MAC), after);
        let handle = ContextMenu::new()
            .shape(shape)
            // The look is for the list of a desktop: a pill that a finger asks for keeps the one that suits a finger.
            .style(self.menu_style(shape))
            .around(anchor)
            .items(listed)
            // The verb decides: everything but `Select All` finishes the job.
            .dismiss_on_select(false)
            .on_select(move |index| {
                let verb = chosen_for(index, on_chosen.extra_counts.get().0, &on_chosen.actions.borrow());
                on_chosen.chosen.set(Some(verb));
                // The verb is applied by the field's next frame, which the menu's own row does not ask for.
                aimer_events::window::request_animation_frame();
            })
            .show();
        *self.menu_shared().handle.borrow_mut() = Some(handle);
        self.menu_shape.set(Some(shape));
    }

    /// The look of the menu of `shape`: the app's, for the list, or the default of the shape.
    fn menu_style(&self, shape: ContextMenuShape) -> ContextMenuStyle {
        match (&self.context_menu_style, shape) {
            (Some(style), ContextMenuShape::List) => style.clone(),
            _ => ContextMenuStyle::for_shape(shape),
        }
    }

    /// What an app that is asked what to add to the menu is told.
    fn menu_request(&self, origin: MenuOrigin) -> ContextMenuRequest {
        let value = self.controller.value();
        let (start, end) = self.cursor.selection_range().unwrap_or_else(|| {
            let caret = self.cursor.offset();
            (caret, caret)
        });
        let (start, end) = (value.byte_of_grapheme(start), value.byte_of_grapheme(end));
        let bounds = self.cached_bounds.get_bounds();
        let (x, y) = match origin {
            MenuOrigin::Click(pos) => (pos.x, pos.y),
            MenuOrigin::Hold => bounds.map_or((0.0, 0.0), |bounds| (bounds.x, bounds.y)),
        };
        let origin_of = bounds.map_or((0.0, 0.0), |bounds| (bounds.x, bounds.y));
        ContextMenuRequest {
            byte: self.menu_hit.take(),
            selection: start..end.max(start),
            spot: FieldSpot {
                x: x - origin_of.0,
                y: y - origin_of.1,
                width: 0.0,
                height: 0.0,
                extent: bounds.map(|bounds| (bounds.width, bounds.height)),
            },
        }
    }

    /// Runs the verb the open menu was told to run, if any.
    pub(super) fn apply_chosen_action(&self) {
        if let Some(action) = self.menu_shared().chosen.take() {
            self.run_menu_action(action);
        }
    }

    /// Closes the open menu, if there is one.
    ///
    /// Repeated calls are harmless, which is what lets every dismissal path
    /// call it without asking first.
    pub(super) fn close_menu(&self) {
        if let Some(handle) = self.menu_shared().handle.borrow_mut().take() {
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
        self.menu_shared()
            .handle
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
        self.queue_click(pos);
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

    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use aimer_animation::AnimInstant;
    use aimer_attribute::position::Vec2d;
    use aimer_ctxmenu::{ContextMenuItem, ContextMenuShape};
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::{Drawable, EventElement, Rebuildable};

    use super::super::test_support::{dummy_build_context, focused_field};
    use super::{ContextMenuExtras, ContextMenuRequest, FieldAction, MenuOrigin, RawTextField};
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
        field.menu_shared().actions.borrow().clone()
    }

    /// A field that is asked for what to add to its menu, and the requests it was asked with.
    fn field_with_extras(text: &str, before: &'static [&'static str], after: &'static [&'static str]) -> (RawTextField, Rc<RefCell<Vec<ContextMenuRequest>>>) {
        let mut field = field(text);
        let seen: Rc<RefCell<Vec<ContextMenuRequest>>> = Rc::default();
        let record = Rc::clone(&seen);
        field.context_menu_extras = Some(Rc::new(move |request: &ContextMenuRequest| {
            record.borrow_mut().push(request.clone());
            ContextMenuExtras {
                before: before.iter().map(|label| ContextMenuItem::new(*label)).collect(),
                after: after.iter().map(|label| ContextMenuItem::new(*label)).collect(),
            }
        }));
        (field, seen)
    }

    #[test]
    fn what_the_app_adds_is_listed_before_and_after_the_clipboard_verbs() {
        let (field, _) = field_with_extras("hello world", &["Context"], &["Usages", "Rename"]);
        field.request_menu(MenuOrigin::Hold);
        field.raise_pending_menu();

        assert!(field.menu_is_open());
        assert_eq!(field.menu_shared().extra_counts.get(), (1, 2), "one row before the verbs and two after");
        assert!(verbs(&field).contains(&FieldAction::SelectAll), "the verbs are still the field's own");
    }

    #[test]
    fn a_field_with_nothing_to_paste_or_copy_still_raises_a_menu_of_what_the_app_adds() {
        let controller = TextFieldController::new();
        let mut field = focused_field(controller);
        field.read_only = true;
        field.cached_bounds.save(1.0, 0.0, 0.0, 200.0, 32.0);
        field.context_menu_extras = Some(Rc::new(|_: &ContextMenuRequest| ContextMenuExtras { before: Vec::new(), after: vec![ContextMenuItem::new("Usages")] }));

        field.request_menu(MenuOrigin::Hold);
        field.raise_pending_menu();

        assert!(field.menu_is_open());
        assert_eq!(field.menu_shared().extra_counts.get(), (0, 1));
    }

    #[test]
    fn an_app_that_adds_nothing_leaves_the_menu_as_it_was() {
        let (field, _) = field_with_extras("hello world", &[], &[]);
        field.request_menu(MenuOrigin::Hold);
        field.raise_pending_menu();
        assert!(field.menu_is_open());
        assert_eq!(field.menu_shared().extra_counts.get(), (0, 0));
    }

    #[test]
    fn the_app_is_told_what_is_selected_in_bytes_and_where_the_menu_opens() {
        // "é" is two bytes: the selection is of the bytes, which is what the app's text is counted in.
        let (field, seen) = field_with_extras("\u{e9}b cd", &[], &["Usages"]);
        field.cursor.set_selection_anchor(Some(0));
        field.cursor.set_offset(2);
        field.request_menu(MenuOrigin::Click(Vec2d { x: 20.0, y: 16.0 }));
        field.raise_pending_menu();

        let seen = seen.borrow();
        assert_eq!(seen.len(), 1, "asked once for each menu that is raised");
        assert_eq!(seen[0].selection, 0..3, "two characters, three bytes");
        assert_eq!((seen[0].spot.x, seen[0].spot.y), (20.0, 16.0), "where it was clicked, in the field's own coordinates");
        assert!(seen[0].spot.extent.is_some());
    }

    #[test]
    fn a_caret_is_a_selection_of_nothing_at_the_caret() {
        let (field, seen) = field_with_extras("hello world", &[], &["Usages"]);
        field.cursor.set_offset(4);
        field.request_menu(MenuOrigin::Hold);
        field.raise_pending_menu();
        assert_eq!(seen.borrow()[0].selection, 4..4);
    }

    #[test]
    fn a_row_the_app_added_closes_the_menu_and_a_clipboard_row_is_its_verb() {
        let verbs = [FieldAction::Copy, FieldAction::SelectAll];
        // One row before the verbs and one after them.
        assert_eq!(super::chosen_for(0, 1, &verbs), FieldAction::Dismiss, "the app's row runs what it was given, and the menu closes");
        assert_eq!(super::chosen_for(1, 1, &verbs), FieldAction::Copy);
        assert_eq!(super::chosen_for(2, 1, &verbs), FieldAction::SelectAll);
        assert_eq!(super::chosen_for(3, 1, &verbs), FieldAction::Dismiss, "the row after the verbs");
        assert_eq!(super::chosen_for(9, 1, &verbs), FieldAction::Dismiss, "a row that is not there is nothing to run");
    }

    /// A field made again with the same controller, as a pane that is built again does, and given what the one before had.
    fn rebuilt(old: &RawTextField) -> RawTextField {
        let controller = old.controller.clone();
        let new = RawTextField::new(super::super::test_support::field_config(controller), crate::input_field::caret::CaretBlink::new(), aimer_widget::FocusNode::new());
        new.adopt_runtime_state_from(old as &dyn aimer_widget::Element);
        new
    }

    #[test]
    fn a_menu_that_is_open_when_the_field_is_built_again_is_the_new_fields_to_close() {
        let old = field("hello world");
        old.request_menu(MenuOrigin::Hold);
        old.raise_pending_menu();
        assert!(old.menu_is_open());

        let new = rebuilt(&old);

        assert!(new.menu_is_open(), "it is the same menu, and the new field knows it is open");
        new.dismiss_menu();
        assert!(!new.menu_is_open(), "and closes it: it is not left over the text with no one to close it");
        assert!(!old.menu_is_open(), "the same menu is closed for the one it was raised by too");
    }

    #[test]
    fn a_row_chosen_after_the_field_was_built_again_is_applied_by_the_new_field() {
        let old = field("hello world");
        old.request_menu(MenuOrigin::Hold);
        old.raise_pending_menu();
        let new = rebuilt(&old);

        // What the menu's own callback does: it was made by the old field, and writes where the old field says.
        old.menu_shared().chosen.set(Some(FieldAction::SelectAll));
        new.apply_chosen_action();

        assert_eq!(new.cursor.selection_range(), Some((0, new.controller.grapheme_count())), "applied to the text that is on screen");
        assert_eq!(new.menu_shared().chosen.get(), None, "once");
    }

    #[test]
    fn a_menu_that_was_asked_for_but_not_yet_raised_is_raised_by_the_new_field() {
        let old = field("hello world");
        old.request_menu(MenuOrigin::Hold);
        let new = rebuilt(&old);
        new.cached_bounds.save(1.0, 0.0, 0.0, 200.0, 32.0);
        new.raise_pending_menu();
        assert!(new.menu_is_open(), "the press was not lost with the field that took it");
    }

    #[test]
    fn the_apps_look_is_the_look_of_the_list_and_the_pill_keeps_its_own() {
        let mut field = field("hello world");
        assert_eq!(field.menu_style(ContextMenuShape::List).row_height, aimer_ctxmenu::ContextMenuStyle::list().row_height, "none given: the default");
        field.context_menu_style = Some(aimer_ctxmenu::ContextMenuStyle::list().row_height(40.0));
        assert_eq!(field.menu_style(ContextMenuShape::List).row_height, 40.0);
        assert_eq!(field.menu_style(ContextMenuShape::Pill).row_height, aimer_ctxmenu::ContextMenuStyle::pill().row_height, "a finger's menu is not the desktop's");
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
        field.menu_shared().chosen.set(Some(FieldAction::SelectAll));

        assert!(field.sync_paint_state(&context));

        assert_eq!(field.cursor.selection_range(), Some((0, end)));
        assert_eq!(field.controller.selection_graphemes(), (0, end));
        assert_eq!(field.menu_shared().chosen.get(), None);
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
    fn the_focus_the_menu_takes_leaves_it_and_the_keyboard_alone() {
        let field = field("hello world");
        field.request_menu(MenuOrigin::Hold);
        field.raise_pending_menu();
        assert!(field.menu_is_open());
        assert!(field.ime_enabled.get());

        // Presenting the menu confines focus to its own layer, so the field
        // underneath is told it lost the keyboard — by the menu, not the user.
        let _ = field.on_event(&ElementEvent::FocusLost);

        assert!(field.menu_is_open(), "the menu may not close itself");
        assert!(field.is_focused(), "the selection it acts on has to survive");
        assert!(
            field.ime_enabled.get(),
            "the software keyboard must not be hidden and re-raised under the menu"
        );
    }

    #[test]
    fn losing_focus_with_no_menu_showing_blurs_the_field() {
        let field = field("hello world");

        let _ = field.on_event(&ElementEvent::FocusLost);

        assert!(!field.is_focused());
        assert!(!field.ime_enabled.get());
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
