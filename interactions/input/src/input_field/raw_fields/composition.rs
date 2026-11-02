/// Input-method composition (preedit) state and presentation.
///
/// A composition reaches the field on two paths that must end in the same
/// presentation: desktop input methods send [`ElementEvent::ImePreedit`] /
/// [`ElementEvent::ImeCommit`], while the iOS / Android keyboard shims mutate
/// a hidden native editor and report [`ElementEvent::TextEditingDelta`]s whose
/// composing range lives in the controller value. Either way the field paints
/// the committed text with the composing segment stripped, the composition as
/// underlined provisional text at the insertion point, and the caret inside
/// the composition.
impl RawTextField {
    /// Borrows the composition string without cloning it.
    ///
    /// The value is moved out of its cell for the duration of `f` and put back
    /// afterwards, so reading it every frame costs no allocation.
    fn with_preedit<R>(&self, f: impl FnOnce(&str) -> R) -> R {
        let preedit = self.preedit_text.take();
        let value = f(&preedit);
        self.preedit_text.set(preedit);
        value
    }

    /// Returns whether an input-method composition is currently in progress.
    fn is_composing(&self) -> bool {
        self.with_preedit(|preedit| !preedit.is_empty())
    }

    /// Replaces the composition, returning whether anything changed.
    ///
    /// Input methods resend an identical preedit while candidates are browsed;
    /// reporting no change lets the caller skip a repaint.
    fn set_preedit(&self, text: &str, cursor: Option<(usize, usize)>) -> bool {
        let cursor_changed = self.preedit_cursor.replace(cursor) != cursor;
        let text_changed = self.with_preedit(|preedit| preedit != text);
        if text_changed {
            self.reveal_caret.set(true);
            if text.is_empty() {
                self.controller.cancel_composing();
            } else {
                let (anchor, focus) = self.cursor_selection();
                self.controller
                    .update_composing_graphemes(anchor, focus, text);
                let value = self.controller.value();
                if let Some(composing) = value.composing() {
                    let start = unicode_segmentation::UnicodeSegmentation::graphemes(
                        &value.text()[..composing.start()],
                        true,
                    )
                    .count();
                    self.cursor.set_offset(start);
                    self.cursor.clear_selection();
                }
            }
            self.observed_revision.set(self.controller.revision());
            self.preedit_text.set(text.to_owned());
        }
        text_changed || cursor_changed
    }

    /// Abandons any composition in progress.
    ///
    /// The platform text editor may still hold the abandoned marked text, so
    /// the restored value is pushed back to it; the push also invalidates any
    /// composition delta still in flight.
    fn clear_preedit(&self) {
        self.preedit_cursor.set(None);
        if self.is_composing() {
            self.controller.cancel_composing();
            self.observed_revision.set(self.controller.revision());
            self.preedit_text.set(String::new());
            self.sync_platform_text_state();
        }
    }

    fn clear_preedit_presentation(&self) {
        self.preedit_cursor.set(None);
        self.preedit_text.set(String::new());
    }

    /// Mirrors the controller's composing range into the preedit presentation.
    ///
    /// A composition that arrives through a native [`ElementEvent::TextEditingDelta`]
    /// (the iOS / Android keyboard shims) lives in the controller value rather
    /// than in an [`ElementEvent::ImePreedit`], so the cells the draw path
    /// reads — the preedit string, the caret inside it, and the canvas caret
    /// parked before it — are derived from that value here. Without this the
    /// composing segment is stripped from the display and painted nowhere,
    /// leaving mobile CJK input invisible and the caret on the wrong glyph.
    fn sync_preedit_from_controller(&self) {
        let value = self.controller.value();
        let Some(composing) = value.composing() else {
            ime_trace!("preedit sync: cleared, caret back to the text");
            self.clear_preedit_presentation();
            return;
        };
        ime_trace!(
            "preedit sync: {:?}, caret parked before it",
            &value.text()[composing.start()..composing.end()],
        );
        let preedit = &value.text()[composing.start()..composing.end()];
        // The caret inside the composition: the native selection expressed as
        // byte offsets relative to the composing segment.
        let anchor = value
            .selection()
            .anchor()
            .clamp(composing.start(), composing.end())
            - composing.start();
        let focus = value
            .selection()
            .focus()
            .clamp(composing.start(), composing.end())
            - composing.start();
        self.preedit_cursor
            .set(Some((anchor.min(focus), anchor.max(focus))));
        if self.with_preedit(|current| current != preedit) {
            self.preedit_text.set(preedit.to_owned());
        }
        // The canvas caret indexes the display text, which strips the
        // composition; park it where the composition begins.
        let start = unicode_segmentation::UnicodeSegmentation::graphemes(
            &value.text()[..composing.start()],
            true,
        )
        .count();
        self.cursor.set_offset(start);
        self.cursor.clear_selection();
    }

    /// Whether the placeholder should be painted.
    ///
    /// An input method composing into an empty field has the field showing the
    /// composition, not nothing: painting the placeholder underneath it stacks
    /// two texts in the same place, which is what a CJK user sees as their
    /// prediction tangled with the hint.
    #[inline]
    fn placeholder_visible(&self) -> bool {
        !self.is_composing()
    }

    /// Draws the input-method composition, its underlines, and its caret.
    ///
    /// `origin_x` and `top` are the content-local canvas coordinates of the
    /// insertion point, and `height` is the line height the composition shares
    /// with the surrounding text. `cursor` is the byte range the input method
    /// reports inside `preedit`: an empty range is the composition caret, while
    /// a non-empty one marks the clause being edited and is underlined twice as
    /// thick so long Japanese or Korean compositions show which part is active.
    fn preedit_layout(
        &self,
        content_ctx: &BuildContext,
        preedit: &str,
    ) -> Rc<aimer_cupid::text_layout::TextInteractionLayout> {
        let mut style = self.field_text_style();
        style.text_overflow = TextOverflow::Clip;
        let widget = self.build_text_widget(preedit, &style, TextAlign::TopLeft);
        widget.interaction_layout(content_ctx)
    }

    /// Measures the provisional text with the exact shaper used to paint it.
    fn preedit_width(&self, content_ctx: &BuildContext, preedit: &str) -> f32 {
        let (preedit, _) = presentation_preedit(self.input_type, preedit, None);
        self.preedit_layout(content_ctx, preedit.as_ref())
            .metrics
            .width
    }


    pub(super) fn sync_preedit_caret(
        &self,
        preedit: &str,
        cursor: Option<(usize, usize)>,
        origin_x: f32,
        top: f32,
        height: f32,
        content_ctx: &BuildContext,
        scale: f32,
    ) {
        let (preedit, cursor) = presentation_preedit(self.input_type, preedit, cursor);
        let Some((start, end)) = cursor else {
            return;
        };
        let start = floor_char_boundary(preedit.as_ref(), start);
        let end = floor_char_boundary(preedit.as_ref(), end.max(start));
        if end > start {
            return;
        }
        let layout = self.preedit_layout(content_ctx, preedit.as_ref());
        let start_x = origin_x
            + layout
                .caret_geometry(start)
                .map_or(layout.metrics.width, |caret| caret.x);
        let (caret_top, caret_height) = caret_band(top, height);
        self.publish_composition_caret(
            start_x,
            caret_top,
            1.5 * scale,
            caret_height,
            scale,
        );
    }
}

#[cfg(test)]
mod composition_tests {
    //! Composing text with an input method: the provisional preedit, the
    //! commit that replaces it, and what a field draws in between.

    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_events::element::ElementEvent;
    use aimer_events::text_editing::{NativeTextRange, TextEditingDelta};
    use aimer_widget::{Drawable, EventElement};

    use super::ImeCaretArea;
    use super::test_support::{commit, dummy_build_context, focused_field};
    use crate::TextEditingController as TextFieldController;
    use crate::input_field::raw_fields::TextFieldCallback;

    fn preedit(text: &str, cursor: Option<(usize, usize)>) -> ElementEvent {
        ElementEvent::ImePreedit {
            text: text.to_owned(),
            cursor,
        }
    }

    fn caret() -> ImeCaretArea {
        ImeCaretArea {
            x: 10.0,
            y: 20.0,
            width: 1.0,
            height: 16.0,
        }
    }

    #[test]
    fn committed_phrase_is_one_edit_with_one_change_notification() {
        let controller = TextFieldController::new();
        let changes = Rc::new(Cell::new(0));
        let mut field = focused_field(controller.clone());
        let counter = changes.clone();
        field.on_changed = TextFieldCallback::from(move |_: String| {
            counter.set(counter.get() + 1);
        });

        assert!(field.on_event(&commit("你好世界")).is_consumed());

        assert_eq!(controller.text(), "你好世界");
        assert_eq!(field.cursor.offset(), 4);
        assert_eq!(changes.get(), 1);
        assert!(controller.undo());
        assert_eq!(controller.text(), "");
        assert!(!controller.undo());
    }

    #[test]
    fn committed_phrase_is_inserted_at_the_cursor() {
        let controller = TextFieldController::with_initial("ab");
        let field = focused_field(controller.clone());
        field.cursor.set_offset(1);

        let _ = field.on_event(&commit("你好"));

        assert_eq!(controller.text(), "a你好b");
        assert_eq!(field.cursor.offset(), 3);
    }

    #[test]
    fn committed_phrase_replaces_the_selection() {
        let controller = TextFieldController::with_initial("abc");
        let field = focused_field(controller.clone());
        field.cursor.set_selection_anchor(Some(0));
        field.cursor.set_offset(3);

        let _ = field.on_event(&commit("你好"));

        assert_eq!(controller.text(), "你好");
        assert_eq!(field.cursor.offset(), 2);
        assert_eq!(field.cursor.selection_anchor(), None);
    }

    #[test]
    fn max_length_truncates_the_commit_instead_of_rejecting_it() {
        let controller = TextFieldController::with_initial("a");
        let mut field = focused_field(controller.clone());
        field.max_length = Some(3);
        field.cursor.set_offset(1);

        assert!(field.on_event(&commit("你好世界")).is_consumed());

        assert_eq!(controller.text(), "a你好");
        assert_eq!(field.cursor.offset(), 3);
    }

    #[test]
    fn a_full_field_ignores_a_commit() {
        let controller = TextFieldController::with_initial("ab");
        let mut field = focused_field(controller.clone());
        field.max_length = Some(2);
        field.cursor.set_offset(2);

        assert!(!field.on_event(&commit("你好")).is_consumed());

        assert_eq!(controller.text(), "ab");
    }

    #[test]
    fn read_only_and_unfocused_fields_ignore_a_commit() {
        let controller = TextFieldController::new();
        let mut read_only = focused_field(controller.clone());
        read_only.read_only = true;
        let unfocused = focused_field(controller.clone());
        unfocused.focused.set(false);

        assert!(!read_only.on_event(&commit("你好")).is_consumed());
        assert!(!unfocused.on_event(&commit("你好")).is_consumed());
        assert_eq!(controller.text(), "");
    }

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
    fn a_native_composing_delta_paints_like_an_ime_preedit() {
        let controller = TextFieldController::new();
        let field = focused_field(controller.clone());

        let delta = native_delta(
            &field,
            controller.revision(),
            (0, 0),
            "你好",
            (2, 2),
            Some((0, 2)),
        );
        assert!(field.on_event(&delta).is_consumed());

        assert!(field.is_composing());
        assert!(field.with_preedit(|preedit| preedit == "你好"));
        assert_eq!(field.preedit_cursor.get(), Some((6, 6)));
        assert_eq!(field.display_text(), "", "a composition is provisional");
        assert_eq!(field.cursor.offset(), 0);
    }

    #[test]
    fn a_composing_caret_is_published_for_the_retained_caret_widget() {
        let field = focused_field(TextFieldController::new());
        assert!(field.on_event(&preedit("ni", Some((2, 2)))).is_consumed());
        let context = field.caret_context();
        let build_context = dummy_build_context(240.0, 60.0);

        field.update(&build_context);

        assert!(context.is_focused());
        assert!(context.is_composing());
        assert!(context.is_available());
        assert!(context.is_visible());
    }

    #[test]
    fn a_native_composing_delta_parks_the_caret_before_the_composition() {
        let controller = TextFieldController::with_text("ab");
        let field = focused_field(controller.clone());
        controller.set_selection_graphemes(1, 1);
        field.sync_cursor_from_controller();

        let delta = native_delta(
            &field,
            controller.revision(),
            (1, 1),
            "你",
            (2, 2),
            Some((1, 2)),
        );
        assert!(field.on_event(&delta).is_consumed());

        assert_eq!(controller.text(), "a你b");
        assert_eq!(field.display_text(), "ab");
        assert!(field.with_preedit(|preedit| preedit == "你"));
        assert_eq!(field.preedit_cursor.get(), Some((3, 3)));
        assert_eq!(
            field.cursor.offset(),
            1,
            "the canvas caret indexes the display text, which strips the composition",
        );
    }

    #[test]
    fn a_native_commit_delta_clears_the_preedit_and_reports_one_change() {
        let controller = TextFieldController::new();
        let changes = Rc::new(Cell::new(0));
        let mut field = focused_field(controller.clone());
        let counter = changes.clone();
        field.on_changed = TextFieldCallback::from(move |_: String| {
            counter.set(counter.get() + 1);
        });

        let composing = native_delta(
            &field,
            controller.revision(),
            (0, 0),
            "你好",
            (2, 2),
            Some((0, 2)),
        );
        assert!(field.on_event(&composing).is_consumed());
        assert_eq!(changes.get(), 0, "a provisional composition is not a change");

        // The keyboard confirms the candidate: the committed text equals the
        // marked text byte for byte, only the composing range goes away.
        let committed = native_delta(
            &field,
            controller.revision(),
            (0, 2),
            "你好",
            (2, 2),
            None,
        );
        assert!(field.on_event(&committed).is_consumed());

        assert!(!field.is_composing());
        assert_eq!(field.preedit_cursor.get(), None);
        assert_eq!(field.display_text(), "你好");
        assert_eq!(field.cursor.offset(), 2);
        assert_eq!(changes.get(), 1, "the commit is the user-visible change");
        assert!(controller.undo());
        assert_eq!(controller.text(), "");
    }

    #[test]
    fn a_single_line_field_composes_and_commits_like_a_multiline_one() {
        let controller = TextFieldController::new();
        let mut field = focused_field(controller.clone());
        field.max_lines = Some(1);

        assert!(field.on_event(&preedit("nihao", Some((5, 5)))).is_consumed());
        assert!(field.is_composing());
        assert_eq!(field.display_text(), "");

        assert!(field.on_event(&commit("你好")).is_consumed());

        assert!(!field.is_composing());
        assert_eq!(controller.text(), "你好");
        assert_eq!(field.cursor.offset(), 2);
    }

    #[test]
    fn a_commit_ends_the_composition() {
        let controller = TextFieldController::new();
        let field = focused_field(controller.clone());
        let _ = field.on_event(&preedit("ni", Some((2, 2))));

        let _ = field.on_event(&commit("你"));

        assert!(!field.is_composing());
        assert_eq!(field.preedit_cursor.get(), None);
    }

    #[test]
    fn an_unchanged_preedit_is_not_consumed() {
        let field = focused_field(TextFieldController::new());

        assert!(field.on_event(&preedit("nihao", Some((5, 5)))).is_consumed());
        assert!(!field.on_event(&preedit("nihao", Some((5, 5)))).is_consumed());
        assert!(field.on_event(&preedit("nihao", Some((3, 5)))).is_consumed());
        assert!(field.is_composing());
    }

    #[test]
    fn an_empty_preedit_clears_the_composition() {
        let field = focused_field(TextFieldController::new());
        let _ = field.on_event(&preedit("ni", Some((2, 2))));

        assert!(field.on_event(&preedit("", None)).is_consumed());

        assert!(!field.is_composing());
        assert_eq!(field.preedit_cursor.get(), None);
        assert!(!field.on_event(&preedit("", None)).is_consumed());
    }

    #[test]
    fn blurring_abandons_the_composition_and_platform_input() {
        let field = focused_field(TextFieldController::new());
        let _ = field.on_event(&preedit("ni", Some((2, 2))));
        field.enable_platform_ime();
        field.update_ime_cursor_area(caret());

        let _ = field.on_event(&ElementEvent::FocusLost);

        assert!(!field.is_focused());
        assert!(!field.is_composing());
        assert!(!field.ime_enabled.get());
        assert_eq!(field.ime_cursor_area.get(), None);
    }
}
