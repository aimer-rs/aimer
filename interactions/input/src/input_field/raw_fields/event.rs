impl VisitorElement for RawTextField {
    fn debug_name(&self) -> &'static str {
        "TextField"
    }
}

impl EventElement for RawTextField {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if let Some(selection) = self.selection.borrow().as_ref().cloned()
            && let Some(result) = selection.intercept(event)
        {
            self.sync_cursor_from_area();
            return result.with_redraw();
        }
        let active_before = self.mouse_held.get();
        let consumed = (|| {
            if !self.enable
                && !matches!(
                    event,
                    ElementEvent::FocusLost | ElementEvent::PointerExited(_, _)
                )
            {
                return false;
            }

            // debug!("RawTextField on_event: {:?}", event);

            match event {
                ElementEvent::FocusGained => {
                    if self.is_focused() {
                        return false;
                    }
                    self.set_focused(true);
                    self.native_session.set(
                        NEXT_TEXT_EDITING_SESSION.fetch_add(1, Ordering::Relaxed),
                    );
                    // ime_trace!(
                    //     "field focus gained: session={} rev={}",
                    //     self.native_session.get(),
                    //     self.controller.revision(),
                    // );
                    self.sync_platform_text_state();
                    self.on_focus.call(&self.controller.text());
                    true
                }
                ElementEvent::FocusLost => {
                    if !self.is_focused() {
                        return false;
                    }
                    self.dismiss_menu();
                    self.set_focused(false);
                    self.native_session.set(0);
                    self.mouse_held.set(None);
                    self.on_blur.call(&self.controller.text());
                    true
                }
                ElementEvent::TextEditingDelta(delta) => {
                    // A delta applies while the editor's buffer and the
                    // controller text are the same string: it must be based on
                    // the last pushed snapshot or on one of the deltas that
                    // followed it, and nothing but those deltas may have moved
                    // the controller since. The editor keeps reporting against
                    // one snapshot while the next is in flight, so a revision
                    // that merely lags the controller is not stale.
                    ime_trace!(
                        "delta in: session={} rev={} replace={}..{} text={:?} sel={}..{} \
                         composing={:?} | field: session={} base={} mirror={} rev={}",
                        delta.session_id,
                        delta.revision,
                        delta.replacement.start,
                        delta.replacement.end,
                        delta.replacement_text,
                        delta.selection.start,
                        delta.selection.end,
                        delta.composing.map(|range| (range.start, range.end)),
                        self.native_session.get(),
                        self.native_base_revision.get(),
                        self.native_mirror_revision.get(),
                        self.controller.revision(),
                    );
                    if delta.session_id != self.native_session.get()
                        || delta.revision < self.native_base_revision.get()
                        || delta.revision > self.controller.revision()
                        || self.controller.revision() != self.native_mirror_revision.get()
                    {
                        ime_trace!("delta out: rejected as stale");
                        return false;
                    }
                    if !delta.replacement_text.is_empty() {
                        // Text just arrived, so the keyboard that produced it
                        // is the one currently up: this is the only moment its
                        // language describes what the field holds.
                        self.capture_input_language();
                    }
                    let before = self.controller.value();
                    let Some(adapted) = adapt_native_delta(&before, delta) else {
                        // The offsets no longer map: the editor and the
                        // controller diverged, and only a fresh snapshot
                        // re-anchors them. A newline in such a delta is a
                        // ghost of the diverged buffer, never a Return the
                        // user pressed on this text — nothing submits here.
                        ime_trace!("delta out: offsets do not map, rebasing");
                        self.rebase_native_editor();
                        return true;
                    };
                    if self.native_return_submits(&before, delta) {
                        // The editor already inserted the newline locally; the
                        // rebase snapshot reverts it.
                        ime_trace!("delta out: return submits {:?}", self.controller.text());
                        self.cursor.clear_selection();
                        self.cursor.reset_blink();
                        self.rebase_native_editor();
                        self.on_submitted.call(&self.controller.text());
                        return true;
                    }
                    let value = self.constrain_native_value(adapted.clone());
                    let corrected = value != adapted;
                    if !self.controller.apply_native_value(value) {
                        // Nothing changed, but the editor may have advanced
                        // its revision speculatively after reporting.
                        ime_trace!("delta out: no-op, rebasing");
                        self.rebase_native_editor();
                        return true;
                    }
                    self.native_mirror_revision.set(self.controller.revision());
                    self.sync_cursor_presentation();
                    self.sync_preedit_from_controller();
                    // On a phone every keystroke arrives as one of these
                    // deltas — it is typing, and typing keeps the caret solid
                    // exactly like the desktop editing paths do.
                    self.cursor.reset_blink();
                    ime_trace!(
                        "delta out: applied, text={:?} composing={:?} corrected={} rev={}",
                        self.controller.text(),
                        self.controller.value().composing().map(|r| (r.start(), r.end())),
                        corrected,
                        self.controller.revision(),
                    );
                    if corrected {
                        // The constrained value differs from the editor's
                        // buffer; push it back so both sides converge.
                        self.sync_platform_text_state();
                    }
                    let after = self.controller.value();
                    // A commit is a change even when the committed text equals
                    // the marked text byte for byte.
                    if after.composing().is_none()
                        && (before.text() != after.text() || before.composing().is_some())
                    {
                        self.on_changed.call(&self.controller.text());
                    }
                    true
                }
                ElementEvent::PointerDown(info) => {
                    let pos = &info.pos;
                    let is_inside = self.cached_bounds.is_inside(pos.x, pos.y);

                    if is_inside {
                        // A secondary click asks for the desktop menu, not a
                        // caret: it neither moves the selection it is about to
                        // act on nor starts a drag.
                        if info.button == PointerButton::Secondary {
                            if let Some(selection) = self.selection.borrow().as_ref().cloned()
                                && selection.open_context_menu(
                                    *pos,
                                    PointerKey::new(info.source, info.id),
                                )
                            {
                                self.sync_cursor_from_area();
                                return true;
                            }
                            if self.cursor.selection_range().is_none() {
                                self.select_word_under(*pos);
                            }
                            self.request_menu(MenuOrigin::Click(*pos));
                            return true;
                        }

                        self.mouse_held.set(Some(PointerKey::new(info.source, info.id)));
                        self.cursor.clear_selection();

                        if let Some(selection) = self.selection.borrow().as_ref().cloned()
                            && let Some(offset) = selection.offset_at(pos.x, pos.y)
                        {
                            selection.begin(
                                offset,
                                PointerKey::new(info.source, info.id),
                            );
                        }

                        // Double/triple-click detection
                        let now = self.now();
                        let elapsed = now.duration_since(self.last_click_time.get());
                        let prev_count = self.click_count.get();
                        let same_position = self.last_click_pos.get().is_some_and(|previous| {
                            let slop = crate::gesture::tap_slop(info.source);
                            (pos.x - previous.x).abs() <= slop
                                && (pos.y - previous.y).abs() <= slop
                        });
                        let new_count = if prev_count > 0
                            && elapsed.as_millis() < 500
                            && same_position
                        {
                            prev_count.saturating_add(1).min(3)
                        } else {
                            1
                        };
                        self.click_count.set(new_count);
                        self.last_click_time.set(now);
                        self.last_click_pos.set(Some(*pos));

                        // Defer cursor placement to draw() where canvas is available
                        self.pending_click.set(Some(*pos));
                        self.cursor.reset_blink();


                        // A finger has no modifier keys to copy with, so it is
                        // watched for the hold that raises the menu instead.
                        self.touch_hold.press(
                            PointerKey::new(info.source, info.id),
                            info.source,
                            *pos,
                            now,
                        );

                        // Clear IME preedit on new click
                        self.clear_preedit();
                        true
                    } else {
                        if self.mouse_held.get().is_some() {
                            return false;
                        }
                        self.dismiss_menu();
                        false
                    }
                }
                ElementEvent::Scroll { delta, .. } => self.scroll_vertical(delta.y),
                ElementEvent::CharInput { ch, action, .. } => {
                    if !self.is_focused() || self.read_only {
                        return false;
                    }
                    if *action == KeyAction::Released {
                        return false;
                    }
                    self.dismiss_menu();

                    let mut encoded = [0u8; 4];
                    self.insert_text(ch.encode_utf8(&mut encoded))
                }
                ElementEvent::TextInput { text, action, .. } => {
                    if !self.is_focused() || self.read_only {
                        return false;
                    }
                    if *action == KeyAction::Released {
                        return false;
                    }
                    self.dismiss_menu();

                    self.insert_text(text)
                }
                ElementEvent::KeyInput {
                    key,
                    action,
                    modifiers,
                } => {
                    if !self.is_focused() {
                        return false;
                    }
                    if *action == KeyAction::Released {
                        return false;
                    }
                    if let Some(filter) = &self.key_filter
                        && filter(key, modifiers)
                    {
                        return true;
                    }
                    self.dismiss_menu();

                    let is_shortcut = modifiers.ctrl || modifiers.meta;

                    // Handle Ctrl/Cmd shortcuts
                    if is_shortcut {
                        let result = match key {
                            NamedKey::Other(k) if k == "a" => {
                                // Select all
                                self.cursor.set_selection_anchor(Some(0));
                                self.cursor.set_offset(self.controller.grapheme_count());
                                true
                            }
                            NamedKey::Other(k) if k == "c" => {
                                // Copy
                                if !self.input_type.is_obscured()
                                    && let Some((start, end)) = self.cursor.selection_range()
                                {
                                    let selected = self.controller.get_range(start, end);
                                    clipboard_write(&selected);
                                }
                                true
                            }
                            NamedKey::Other(k) if k == "x" && !self.read_only => {
                                // Cut
                                if !self.input_type.is_obscured()
                                    && let Some((start, end)) = self.cursor.selection_range()
                                {
                                    let selected = self.controller.get_range(start, end);
                                    clipboard_write(&selected);
                                    self.replace_cursor_selection("", None);
                                }
                                true
                            }
                            NamedKey::Other(k) if k == "v" && !self.read_only => {
                                // Paste. Routing through `insert_text` replaces
                                // the selection, advances the cursor by whole
                                // grapheme clusters, honours `max_length`, and
                                // records the paste as a single undo entry.
                                if let Some(text) = clipboard_read() {
                                    self.insert_text(&text);
                                }
                                true
                            }
                            NamedKey::Other(k)
                                if k == "z" && !modifiers.shift && !self.read_only =>
                            {
                                // Undo
                                if self.controller.undo() {
                                    self.sync_cursor_from_controller();
                                    self.on_changed.call(&self.controller.text());
                                }
                                true
                            }
                            NamedKey::Other(k)
                                if k == "z" && modifiers.shift && !self.read_only =>
                            {
                                // Redo (Ctrl+Shift+Z)
                                if self.controller.redo() {
                                    self.sync_cursor_from_controller();
                                    self.on_changed.call(&self.controller.text());
                                }
                                true
                            }
                            NamedKey::Other(k) if k == "y" && !self.read_only => {
                                // Redo (Ctrl+Y — Windows convention)
                                if self.controller.redo() {
                                    self.sync_cursor_from_controller();
                                    self.on_changed.call(&self.controller.text());
                                }
                                true
                            }
                            NamedKey::Enter => {
                                // Ctrl+Enter / Cmd+Enter: submit even in multi-line mode
                                self.cursor.clear_selection();
                                self.on_submitted.call(&self.controller.text());
                                true
                            }
                            _ => false,
                        };
                        if result {
                            self.sync_selection_to_controller();
                            self.cursor.reset_blink();
                            return true;
                        }
                    }

                    let result = match key {
                        NamedKey::Backspace if !self.read_only => {
                            self.delete_backward();
                            true
                        }
                        NamedKey::Delete if !self.read_only => {
                            self.delete_forward();
                            true
                        }
                        NamedKey::Enter
                            if !self.read_only && self.max_lines.is_none_or(|max| max > 1) =>
                        {
                            // Multi-line mode: Enter inserts newline
                            if let Some(max) = self.max_lines
                                && self.line_count() >= max
                            {
                                return true;
                            }
                            self.insert_text("\n");
                            true
                        }
                        NamedKey::Enter => {
                            // Single-line mode (or Ctrl+Enter in multi-line): submit
                            self.cursor.clear_selection();
                            self.on_submitted.call(&self.controller.text());
                            true
                        }
                        NamedKey::ArrowLeft => {
                            self.move_left(modifiers.shift);
                            true
                        }
                        NamedKey::ArrowRight => {
                            self.move_right(modifiers.shift);
                            true
                        }
                        NamedKey::ArrowUp => {
                            self.move_vertical(-1, modifiers.shift);
                            true
                        }
                        NamedKey::ArrowDown => {
                            self.move_vertical(1, modifiers.shift);
                            true
                        }
                        NamedKey::Home => {
                            if modifiers.shift {
                                let offset = self.cursor.offset();
                                if self.cursor.selection_anchor().is_none() {
                                    self.cursor.set_selection_anchor(Some(offset));
                                }
                            } else {
                                self.cursor.clear_selection();
                            }
                            self.cursor.set_offset(0);
                            self.reveal_caret.set(true);
                            true
                        }
                        NamedKey::End => {
                            if modifiers.shift {
                                let offset = self.cursor.offset();
                                if self.cursor.selection_anchor().is_none() {
                                    self.cursor.set_selection_anchor(Some(offset));
                                }
                            } else {
                                self.cursor.clear_selection();
                            }
                            self.cursor.set_offset(self.controller.grapheme_count());
                            self.reveal_caret.set(true);
                            true
                        }
                        NamedKey::Escape => {
                            self.cursor.clear_selection();
                            self.focus_node.unfocus();
                            true
                        }
                        _ => false,
                    };
                    if result {
                        self.sync_selection_to_controller();
                        self.cursor.reset_blink();
                    }
                    result
                }
                ElementEvent::PointerMove(info) => {
                    let pos = &info.pos;
                    let is_inside = self.cached_bounds.is_inside(pos.x, pos.y);
                    let was_hovered = self.is_hovered();
                    if is_inside || self.mouse_held.get().is_some() {
                        aimer_utils::cursor::set_cursor(aimer_utils::cursor::CursorIcon::Text);
                    } else {
                        aimer_utils::cursor::reset_cursor();
                    }
                    self.set_hovered(is_inside);

                    // A finger that rested long enough asks for the menu, and
                    // stops being a drag.
                    let pointer = PointerKey::new(info.source, info.id);
                    if self.touch_hold.moved(pointer, *pos, self.now()) == HoldOutcome::Held
                    {
                        self.select_word_under(*pos);
                        self.request_menu(MenuOrigin::Hold);
                        if let Some(selection) = self.selection.borrow().as_ref().cloned() {
                            selection.end(pointer);
                        }
                        self.mouse_held.set(None);
                        return true;
                    }

                    if let Some(selection) = self.selection.borrow().as_ref().cloned()
                        && selection.active_pointer() == Some(pointer)
                    {
                        selection.extend_to_position(info.x(), info.y(), pointer);
                        self.sync_cursor_from_area();
                        return true;
                    }

                    // Drag-to-select: when mouse is held, defer position resolution to draw()
                    if owns_selection_pointer(self.mouse_held.get(), event) {
                        self.pending_click.set(Some(*pos));
                        return true;
                    }

                    was_hovered != is_inside
                }
                ElementEvent::PointerUp(info) => {
                    let pointer = PointerKey::new(info.source, info.id);
                    let held =
                        self.touch_hold.release(pointer, info.pos, self.now()) == HoldOutcome::Held;
                    if held {
                        self.select_word_under(info.pos);
                        self.request_menu(MenuOrigin::Hold);
                    }
                    if let Some(selection) = self.selection.borrow().as_ref().cloned()
                        && selection.active_pointer() == Some(pointer)
                    {
                        selection.end(pointer);
                        self.sync_cursor_from_area();
                    }
                    if owns_selection_pointer(self.mouse_held.get(), event) {
                        self.mouse_held.set(None);
                        true
                    } else {
                        held
                    }
                }
                ElementEvent::PointerExited(_, _) => {
                    if self.hovered.get() {
                        self.set_hovered(false);
                        aimer_utils::cursor::reset_cursor();
                        true
                    } else {
                        false
                    }
                }
                ElementEvent::ImePreedit { text, cursor } => {
                    if !self.is_focused() {
                        return false;
                    }
                    // Input methods resend an identical composition while the
                    // user browses candidates; reporting no change keeps those
                    // keystrokes from repainting the window.
                    self.set_preedit(text, *cursor)
                }
                ElementEvent::Cancel => {
                    // Presenting a modal cancels the gestures underneath its
                    // barrier — and the field's own menu is one of those
                    // modals, raised a frame earlier. So a cancel arriving
                    // while the menu is showing is the menu's own doing: it
                    // ends the gesture that asked for it, and must leave the
                    // panel, the focus and the selection it acts on alone.
                    if self.menu_is_open() {
                        self.pending_menu.set(None);
                        self.touch_hold.forget();
                        self.mouse_held.set(None);
                        return true;
                    }
                    self.dismiss_menu();
                    if let Some(selection) = self.selection.borrow().as_ref().cloned() {
                        selection.cancel();
                    }
                    self.mouse_held.set(None);
                    true
                }
                _ => false,
            }
        })();

        let result = if consumed {
            EventResult::consumed().with_redraw()
        } else {
            EventResult::ignored()
        };
        let active_after = self.mouse_held.get();
        match (event_pointer_key(event), active_before, active_after) {
            (Some(pointer), before, Some(after)) if before != Some(after) && pointer == after => {
                result.with_pointer_capture(pointer)
            }
            (Some(pointer), Some(before), None) if pointer == before => {
                result.with_pointer_release(pointer)
            }
            _ => result,
        }
    }
}

#[cfg(test)]
mod hover_tests {
    use aimer_attribute::position::Vec2d;
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::{Element, EventElement, FocusNode, Rebuildable};

    use super::test_support::{field_config, focused_field};
    use super::RawTextField;
    use crate::input_field::caret::CaretBlink;
    use crate::TextEditingController;

    #[test]
    fn pointer_exit_clears_a_field_hover() {
        let controller = TextEditingController::new();
        let field = focused_field(controller);
        field.cached_bounds.save(1.0, 0.0, 0.0, 200.0, 32.0);

        let inside = Vec2d { x: 10.0, y: 10.0 };
        let _ = field.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(
            inside,
            PointerButton::Primary,
        )));
        assert!(field.is_hovered());

        let _ = field.on_event(&ElementEvent::PointerExited(PointerSource::Mouse, 0));

        assert!(!field.is_hovered());
    }

    #[test]
    fn rebuilding_a_field_does_not_copy_hover_state() {
        let controller = TextEditingController::new();
        let old = focused_field(controller.clone());
        old.cached_bounds.save(1.0, 0.0, 0.0, 200.0, 32.0);
        let inside = Vec2d { x: 10.0, y: 10.0 };
        let _ = old.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(
            inside,
            PointerButton::Primary,
        )));
        assert!(old.is_hovered());

        let rebuilt = RawTextField::new(
            field_config(controller),
            CaretBlink::new(),
            FocusNode::new(),
        );
        rebuilt.adopt_runtime_state_from(&old as &dyn Element);

        assert!(!rebuilt.is_hovered());
    }
}

#[cfg(test)]
mod key_event_tests {
    use aimer_events::element::NamedKey;
    use aimer_widget::EventElement;

    use super::test_support::{focused_field, key};
    use crate::TextEditingController;

    #[test]
    fn return_inserts_newline_in_unbounded_multiline_field() {
        let controller = TextEditingController::new();
        let mut field = focused_field(controller.clone());
        field.min_lines = Some(3);
        field.max_lines = None;

        assert!(field.on_event(&key(NamedKey::Enter)).is_consumed());
        assert_eq!(controller.text(), "\n");
    }
}

#[cfg(test)]
mod focus_tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_attribute::position::Vec2d;
    use aimer_events::element::ElementEvent;
    use aimer_widget::{Element, EventDispatcher, EventElement, FocusNode, RawFocusable, Rebuildable};

    use super::test_support::{commit, field_config, focused_field};
    use super::{RawTextField, TextFieldCallback};
    use crate::input_field::caret::CaretBlink;
    use crate::TextEditingController as TextFieldController;

    /// A focused field routes what is typed into its controller, and reports
    /// each focus edge exactly once.
    ///
    /// The element is wrapped in [`RawFocusable`] here because the node is
    /// offered by the focus region the field's state builds around it, never by
    /// the element itself; the region is what the state's `focusable_field()`
    /// mounts, so this exercises the same path a real field takes while staying
    /// at the raw element the rest of this module tests.
    #[test]
    fn focus_node_routes_text_and_emits_each_lifecycle_callback_once() {
        let controller = TextFieldController::new();
        let node = FocusNode::new();
        let focuses = Rc::new(Cell::new(0));
        let blurs = Rc::new(Cell::new(0));
        let mut config = field_config(controller.clone());
        config.auto_focus = false;
        let focus_count = focuses.clone();
        config.on_focus = TextFieldCallback::from(move |_| focus_count.set(focus_count.get() + 1));
        let blur_count = blurs.clone();
        config.on_blur = TextFieldCallback::from(move |_| blur_count.set(blur_count.get() + 1));
        let field = RawFocusable::new(
            node.clone(),
            RawTextField::new(config, CaretBlink::new(), node.clone()).boxed(),
        )
        .boxed();
        let mut dispatcher = EventDispatcher::new();

        node.request_focus();
        let _ = dispatcher.dispatch(field.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
        let _ = dispatcher.dispatch(field.as_ref(), Vec2d::default(), &commit("你好"));
        node.unfocus();
        let _ = dispatcher.dispatch(field.as_ref(), Vec2d::default(), &ElementEvent::Cancel);

        assert_eq!(controller.text(), "你好");
        assert_eq!(focuses.get(), 1);
        assert_eq!(blurs.get(), 1);
        assert!(!node.has_focus());
    }

    // Regression: an `on_changed` callback commonly calls `set_state`, which
    // reconciles this element against a freshly built replacement on every
    // keystroke, not just once. `RawTextField::new` always starts unfocused,
    // trusting the enclosing focus region to redeliver `FocusGained` and
    // teach the replacement otherwise — but the framework only fires that
    // event on an actual ownership *change*, and the field's own `FocusNode`
    // never stopped being the owner across a same-field rebuild. Without
    // `adopt_runtime_state_from` carrying `focused` over that swap, the very
    // first character typed would be the last one ever accepted.
    #[test]
    fn a_rebuild_triggered_by_typing_does_not_stop_further_typing() {
        let controller = TextFieldController::new();
        let focused = focused_field(controller.clone());

        let rebuilt = RawTextField::new(
            field_config(controller.clone()),
            CaretBlink::new(),
            FocusNode::new(),
        );
        // No `FocusGained` is redelivered to `rebuilt` here — exactly like a
        // real reconciliation, where the node's ownership never changed.
        rebuilt.adopt_runtime_state_from(&focused as &dyn Element);

        assert!(rebuilt.on_event(&commit("a")).is_consumed());
        assert_eq!(controller.text(), "a");
    }

    #[test]
    fn programmatic_controller_changes_redraw_every_attached_editor() {
        let redraws = Rc::new(Cell::new(0));
        let counted = redraws.clone();
        let previous = aimer_events::window::set_thread_redraw_requester(move || {
            counted.set(counted.get() + 1);
        });
        let controller = TextFieldController::new();
        let _first = RawTextField::new(
            field_config(controller.clone()),
            CaretBlink::new(),
            FocusNode::new(),
        );
        let _second = RawTextField::new(
            field_config(controller.clone()),
            CaretBlink::new(),
            FocusNode::new(),
        );

        controller.set_text("updated");

        assert_eq!(redraws.get(), 2);
        aimer_events::window::restore_thread_redraw_requester(previous);
    }
}

#[cfg(test)]
mod native_delta_tests {
    //! The delta stream of a hidden platform editor.
    //!
    //! A phone keyboard never talks to the field directly: it edits its own
    //! buffer and reports what it did against the revision of the last
    //! snapshot it saw. Accepting, rebasing and rejecting those reports is
    //! what keeps the two buffers the same text.

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use aimer_cupid::font::TextLanguage;
    use aimer_events::element::ElementEvent;
    use aimer_events::text_editing::{NativeTextRange, TextEditingDelta};
    use aimer_widget::EventElement;

    // The seam the delta tests drive the keyboard language through. An iOS
    // build asks UIKit instead, so it has neither the seam nor these tests.
    #[cfg(not(target_os = "ios"))]
    use super::simulate_keyboard_language;
    use super::test_support::{commit, focused_field};
    use crate::TextEditingController as TextFieldController;
    use crate::input_field::raw_fields::TextFieldCallback;

    /// A native editor mutation the way the iOS / Android keyboard shims
    /// report one: UTF-16 offsets against the mirrored Rust revision.
    fn native_delta(
        field: &crate::input_field::raw_fields::RawTextField,
        revision: u64,
        replacement: (usize, usize),
        replacement_text: &str,
        selection: (usize, usize),
        composing: Option<(usize, usize)>,
    ) -> ElementEvent {
        ElementEvent::TextEditingDelta(TextEditingDelta {
            session_id: field.native_session.get(),
            revision,
            replacement: NativeTextRange::new(replacement.0, replacement.1),
            replacement_text: replacement_text.to_owned(),
            selection: NativeTextRange::new(selection.0, selection.1),
            composing: composing.map(|(start, end)| NativeTextRange::new(start, end)),
        })
    }

    #[test]
    fn native_selection_delta_moves_the_canvas_caret_and_rejects_stale_revisions() {
        let controller = TextFieldController::with_text("A👩‍💻B");
        let field = focused_field(controller.clone());
        let session_id = field.native_session.get();
        let selection_delta = |revision| ElementEvent::TextEditingDelta(TextEditingDelta {
            session_id,
            revision,
            replacement: NativeTextRange::new(6, 6),
            replacement_text: String::new(),
            selection: NativeTextRange::new(1, 1),
            composing: None,
        });

        assert!(!field
            .on_event(&selection_delta(controller.revision() + 1))
            .is_consumed());
        let mut wrong_session = match selection_delta(controller.revision()) {
            ElementEvent::TextEditingDelta(delta) => delta,
            _ => unreachable!(),
        };
        wrong_session.session_id += 1;
        assert!(!field
            .on_event(&ElementEvent::TextEditingDelta(wrong_session))
            .is_consumed());
        assert_eq!(field.cursor.offset(), 3);
        assert!(field
            .on_event(&selection_delta(controller.revision()))
            .is_consumed());

        assert_eq!(controller.value().text(), "A👩‍💻B");
        assert_eq!(field.cursor.offset(), 1);
    }

    #[test]
    #[cfg(not(target_os = "ios"))]
    fn a_delta_typed_on_a_chinese_keyboard_teaches_the_field_its_language() {
        let controller = TextFieldController::new();
        let field = focused_field(controller.clone());
        assert_eq!(controller.input_language(), None);

        simulate_keyboard_language(Some("zh-Hans"));
        assert!(field
            .on_event(&native_delta(
                &field,
                controller.revision(),
                (0, 0),
                "你好",
                (2, 2),
                None,
            ))
            .is_consumed());

        assert_eq!(controller.input_language(), Some(TextLanguage::Chinese));
        simulate_keyboard_language(None);
    }

    #[test]
    #[cfg(not(target_os = "ios"))]
    fn a_delta_typed_on_a_latin_keyboard_never_erases_the_learned_language() {
        let controller = TextFieldController::new();
        let field = focused_field(controller.clone());

        simulate_keyboard_language(Some("zh-Hans"));
        assert!(field
            .on_event(&native_delta(&field, controller.revision(), (0, 0), "你好", (2, 2), None))
            .is_consumed());
        simulate_keyboard_language(Some("en-US"));
        assert!(field
            .on_event(&native_delta(&field, controller.revision(), (2, 2), "!", (3, 3), None))
            .is_consumed());

        assert_eq!(controller.value().text(), "你好!");
        assert_eq!(controller.input_language(), Some(TextLanguage::Chinese));
        simulate_keyboard_language(None);
    }

    #[test]
    #[cfg(not(target_os = "ios"))]
    fn a_delta_reported_without_a_language_leaves_the_learned_one_alone() {
        let controller = TextFieldController::new();
        let field = focused_field(controller.clone());

        simulate_keyboard_language(Some("ja-JP"));
        assert!(field
            .on_event(&native_delta(&field, controller.revision(), (0, 0), "あ", (1, 1), None))
            .is_consumed());
        simulate_keyboard_language(None);
        assert!(field
            .on_event(&native_delta(&field, controller.revision(), (1, 1), "!", (2, 2), None))
            .is_consumed());

        assert_eq!(controller.input_language(), Some(TextLanguage::Japanese));
    }

    #[test]
    #[cfg(not(target_os = "ios"))]
    fn the_learned_language_outlives_focus_and_rebuilds() {
        let controller = TextFieldController::new();
        let field = focused_field(controller.clone());
        simulate_keyboard_language(Some("zh-Hans"));
        assert!(field
            .on_event(&native_delta(&field, controller.revision(), (0, 0), "你好", (2, 2), None))
            .is_consumed());
        simulate_keyboard_language(None);

        assert!(field.on_event(&ElementEvent::FocusLost).is_consumed());
        drop(field);
        let rebuilt = focused_field(controller.clone());

        assert_eq!(rebuilt.controller.input_language(), Some(TextLanguage::Chinese));
    }

    #[test]
    fn native_commits_are_corrected_to_single_line_grapheme_constraints() {
        let controller = TextFieldController::with_text("ab");
        let mut field = focused_field(controller.clone());
        field.max_lines = Some(1);
        field.max_length = Some(4);
        let delta = TextEditingDelta {
            session_id: field.native_session.get(),
            revision: controller.revision(),
            replacement: NativeTextRange::new(2, 2),
            replacement_text: "\n👩‍💻XY".into(),
            selection: NativeTextRange::new(10, 10),
            composing: None,
        };

        assert!(field
            .on_event(&ElementEvent::TextEditingDelta(delta))
            .is_consumed());

        assert_eq!(controller.value().text(), "ab 👩‍💻");
        assert_eq!(field.cursor.offset(), 4);
    }

    #[test]
    fn rapid_native_deltas_against_one_snapshot_all_apply() {
        let controller = TextFieldController::with_text("你好a");
        let field = focused_field(controller.clone());
        let base = controller.revision();

        // A held delete key repeats faster than any snapshot round-trips, so
        // the editor bases every delta of the burst on the same revision. Each
        // one is computed against the text the previous one produced, which is
        // exactly the controller text as long as none of them is dropped.
        let deltas = [
            ((2usize, 3usize), (2usize, 2usize)),
            ((1, 2), (1, 1)),
            ((0, 1), (0, 0)),
        ];
        for (replacement, selection) in deltas {
            let delta = native_delta(&field, base, replacement, "", selection, None);
            assert!(field.on_event(&delta).is_consumed());
        }

        assert_eq!(controller.text(), "");
        assert_eq!(field.cursor.offset(), 0);
    }

    #[test]
    fn a_commit_delta_lands_while_the_composing_echo_is_in_flight() {
        let controller = TextFieldController::new();
        let field = focused_field(controller.clone());
        let base = controller.revision();

        // The user confirms the candidate before the snapshot echoing the
        // composition round-trips: the commit still carries the revision of
        // the snapshot the editor last saw.
        let composing = native_delta(&field, base, (0, 0), "ni", (2, 2), Some((0, 2)));
        assert!(field.on_event(&composing).is_consumed());
        let committed = native_delta(&field, base, (0, 2), "你好", (2, 2), None);
        assert!(field.on_event(&committed).is_consumed());

        assert!(!field.is_composing());
        assert_eq!(controller.text(), "你好");
        assert_eq!(field.display_text(), "你好");
        assert_eq!(field.cursor.offset(), 2);
    }

    #[test]
    fn a_rust_side_edit_closes_the_outstanding_delta_window() {
        let controller = TextFieldController::with_text("ab");
        let field = focused_field(controller.clone());
        let base = controller.revision();

        // The field edits on its own; the editor must rebase on the snapshot
        // that edit pushes before its deltas count again.
        assert!(field.on_event(&commit("c")).is_consumed());

        let stale = native_delta(&field, base, (0, 1), "", (0, 0), None);
        assert!(!field.on_event(&stale).is_consumed());
        assert_eq!(controller.text(), "abc");
    }

    #[test]
    fn a_delta_that_edits_nothing_still_advances_the_sync_epoch() {
        let controller = TextFieldController::with_text("ab");
        let field = focused_field(controller.clone());
        let revision = controller.revision();
        let (anchor, focus) = controller.selection_graphemes();

        // The editor reports the state the controller already holds. Android's
        // shim advances its mirrored revision after every report, so a delta
        // that produces no transaction still has to move the revision forward,
        // otherwise the next delta of the burst is discarded as coming from
        // the future.
        let noop = native_delta(
            &field,
            revision,
            (focus, focus),
            "",
            (anchor, focus),
            None,
        );
        assert!(field.on_event(&noop).is_consumed());

        assert_eq!(controller.text(), "ab");
        assert!(controller.revision() > revision);
    }

    #[test]
    fn a_return_delta_submits_a_single_line_field_without_editing() {
        let controller = TextFieldController::with_text("你好");
        let submitted = Rc::new(RefCell::new(Vec::new()));
        let changes = Rc::new(Cell::new(0));
        let mut field = focused_field(controller.clone());
        field.max_lines = Some(1);
        let sink = submitted.clone();
        field.on_submitted = TextFieldCallback::from(move |text: String| {
            sink.borrow_mut().push(text);
        });
        let counter = changes.clone();
        field.on_changed = TextFieldCallback::from(move |_: String| {
            counter.set(counter.get() + 1);
        });

        // A software keyboard has no key event channel: its Return key arrives
        // as a newline insertion in the text stream.
        let revision = controller.revision();
        let ret = native_delta(&field, revision, (2, 2), "\n", (3, 3), None);
        assert!(field.on_event(&ret).is_consumed());

        assert_eq!(submitted.borrow().as_slice(), ["你好"]);
        assert_eq!(changes.get(), 0, "a submit is not an edit");
        assert_eq!(controller.text(), "你好");
        assert!(
            controller.revision() > revision,
            "the snapshot reverting the editor's local newline must outrank \
             the revision the editor advanced speculatively",
        );
    }

    #[test]
    fn a_return_delta_with_unmappable_offsets_rebases_instead_of_submitting() {
        let controller = TextFieldController::with_text("你好");
        let submitted = Rc::new(Cell::new(0));
        let mut field = focused_field(controller.clone());
        field.max_lines = Some(1);
        let counter = submitted.clone();
        field.on_submitted = TextFieldCallback::from(move |_: String| {
            counter.set(counter.get() + 1);
        });

        // A newline whose offsets do not map onto the controller text is a
        // ghost: the editor's buffer diverged from the field, so whatever the
        // user pressed, it was not Return on this text. Submitting here fires
        // in the middle of typing.
        let revision = controller.revision();
        let ghost = native_delta(&field, revision, (9, 9), "\n", (10, 10), None);
        assert!(field.on_event(&ghost).is_consumed());

        assert_eq!(submitted.get(), 0);
        assert_eq!(controller.text(), "你好");
        assert!(
            controller.revision() > revision,
            "a diverged editor is re-anchored by a fresh snapshot",
        );
    }

    #[test]
    fn a_return_delta_stays_a_newline_in_a_multiline_field() {
        let controller = TextFieldController::with_text("你好");
        let submitted = Rc::new(Cell::new(0));
        let mut field = focused_field(controller.clone());
        field.max_lines = None;
        let counter = submitted.clone();
        field.on_submitted = TextFieldCallback::from(move |_: String| {
            counter.set(counter.get() + 1);
        });

        let ret = native_delta(&field, controller.revision(), (2, 2), "\n", (3, 3), None);
        assert!(field.on_event(&ret).is_consumed());

        assert_eq!(controller.text(), "你好\n");
        assert_eq!(submitted.get(), 0);
    }
}
