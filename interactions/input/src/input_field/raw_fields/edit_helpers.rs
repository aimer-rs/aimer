/// Returns the longest prefix of `text` holding at most `max_graphemes`
/// grapheme clusters.
///
/// Cutting on a cluster boundary is what keeps a length limit from splitting a
/// family emoji into stray code points or stranding a combining accent without
/// its base letter.
fn truncate_graphemes(text: &str, max_graphemes: usize) -> &str {
    use unicode_segmentation::UnicodeSegmentation;
    match text.grapheme_indices(true).nth(max_graphemes) {
        Some((byte, _)) => &text[..byte],
        None => text,
    }
}

/// Splits `text` into grapheme clusters, the unit every cursor offset counts.
fn grapheme_slices(text: &str) -> Vec<&str> {
    unicode_segmentation::UnicodeSegmentation::graphemes(text, true).collect()
}

fn normalize_single_line(text: &str) -> Cow<'_, str> {
    if !text
        .chars()
        .any(|ch| matches!(ch, '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}'))
    {
        return Cow::Borrowed(text);
    }

    let mut normalized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            normalized.push(' ');
        } else if matches!(ch, '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}') {
            normalized.push(' ');
        } else {
            normalized.push(ch);
        }
    }
    Cow::Owned(normalized)
}

fn presentation_preedit<'a>(
    input_type: InputType,
    preedit: &'a str,
    cursor: Option<(usize, usize)>,
) -> (Cow<'a, str>, Option<(usize, usize)>) {
    if !input_type.is_obscured() {
        return (Cow::Borrowed(preedit), cursor);
    }

    let masked = "\u{2022}".repeat(grapheme_count(preedit));
    let cursor = cursor.map(|(start, end)| {
        let start = floor_char_boundary(preedit, start);
        let end = floor_char_boundary(preedit, end.max(start));
        let bullet_bytes = '\u{2022}'.len_utf8();
        (
            grapheme_count(&preedit[..start]) * bullet_bytes,
            grapheme_count(&preedit[..end]) * bullet_bytes,
        )
    });
    (Cow::Owned(masked), cursor)
}

#[inline]
fn vertical_scroll_extent(line_count: usize, line_height: f32, viewport_height: f32) -> f32 {
    (line_count as f32 * line_height - viewport_height).max(0.0)
}

#[inline]
fn vertical_scroll_target(current: f32, delta_y: f32, extent: f32) -> f32 {
    (current - delta_y).clamp(0.0, extent)
}

#[inline]
fn scroll_to_reveal_line(
    current: f32,
    line_index: usize,
    line_height: f32,
    viewport_height: f32,
    extent: f32,
) -> f32 {
    let line_top = line_index as f32 * line_height;
    let line_bottom = line_top + line_height;
    if line_top < current {
        line_top
    } else if line_bottom > current + viewport_height {
        (line_bottom - viewport_height).min(extent)
    } else {
        current.min(extent)
    }
}

impl RawTextField {
    /// Returns whether `delta` is the Return key of a software keyboard
    /// acting on a single-line field.
    ///
    /// A software keyboard has no key event channel: its Return key arrives
    /// as a lone newline in the text stream. A single-line field treats it
    /// the way the desktop path treats [`NamedKey::Enter`] — submit the
    /// current text — rather than editing. Anything longer than the newline
    /// itself (a multi-line paste, a composition) is an edit, not a
    /// keypress, and a Return that confirms a composition arrives as the
    /// committed text instead of a newline.
    fn native_return_submits(&self, before: &TextEditingValue, delta: &TextEditingDelta) -> bool {
        self.max_lines == Some(1)
            && before.composing().is_none()
            && delta.composing.is_none()
            && matches!(delta.replacement_text.as_str(), "\n" | "\r\n" | "\r")
    }

    fn constrain_native_value(&self, value: TextEditingValue) -> TextEditingValue {
        if value.composing().is_some() {
            return value;
        }
        let mut text = if self.max_lines == Some(1) {
            normalize_single_line(value.text()).into_owned()
        } else {
            value.text().to_owned()
        };
        if let Some(max_length) = self.max_length {
            text = truncate_graphemes(&text, max_length).to_owned();
        }
        TextEditingValue::new(text, value.selection(), None)
    }
    fn scroll_vertical(&self, delta_y: f32) -> bool {
        if self.max_lines == Some(1) {
            return false;
        }
        let current = self.scroll_y.get();
        let target = vertical_scroll_target(current, delta_y, self.scroll_y_extent.get());
        if target == current {
            return false;
        }
        self.scroll_y.set(target);
        self.reveal_caret.set(false);
        true
    }

    fn move_vertical(&self, direction: isize, extend: bool) {
        let offset = self.cursor.offset();
        let target = if let Some(geometry) = self.geometry_cache.latest() {
            vertical_target(&geometry.visual_lines, offset, direction)
        } else {
            let display = self.display_text();
            let fallback = wrap_visual_lines(&display, f32::INFINITY, |_| 0.0);
            vertical_target(&fallback, offset, direction)
        };
        if target == offset {
            return;
        }
        let anchor = if extend {
            self.cursor.selection_anchor().unwrap_or(offset)
        } else {
            target
        };
        self.controller.set_selection_graphemes(anchor, target);
        self.sync_cursor_from_controller();
    }
}

/// Counts the grapheme clusters in `text` without allocating.
fn grapheme_count(text: &str) -> usize {
    unicode_segmentation::UnicodeSegmentation::graphemes(text, true).count()
}

/// Converts the field's grapheme offset into the byte offset used by the
/// shared selection session.
fn grapheme_byte_offset(text: &str, offset: usize) -> usize {
    unicode_segmentation::UnicodeSegmentation::grapheme_indices(text, true)
        .nth(offset)
        .map_or(text.len(), |(byte, _)| byte)
}

/// Clamps `byte` down to the nearest character boundary of `text`.
///
/// Input methods report composition ranges in bytes; a range that lands inside
/// a multi-byte character must not be used to slice the string.
fn floor_char_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

fn event_pointer_key(event: &ElementEvent) -> Option<PointerKey> {
    match event {
        ElementEvent::PointerDown(info)
        | ElementEvent::PointerUp(info)
        | ElementEvent::PointerMove(info) => Some(PointerKey::new(info.source, info.id)),
        ElementEvent::PointerExited(source, id) => Some(PointerKey::new(*source, *id)),
        _ => None,
    }
}

fn owns_selection_pointer(active: Option<PointerKey>, event: &ElementEvent) -> bool {
    active.is_some() && active == event_pointer_key(event)
}

#[cfg(test)]
mod vertical_scroll_tests {
    use aimer_attribute::position::Vec2d;
    use aimer_events::element::{ElementEvent, ScrollDeltaKind, TouchPhase};
    use aimer_widget::EventElement;

    use super::{scroll_to_reveal_line, vertical_scroll_extent, vertical_scroll_target};
    use super::test_support::focused_field;
    use crate::TextEditingController;

    #[test]
    fn vertical_scroll_extent_is_the_overflow_below_the_viewport() {
        assert_eq!(vertical_scroll_extent(6, 20.0, 50.0), 70.0);
        assert_eq!(vertical_scroll_extent(2, 20.0, 50.0), 0.0);
    }

    #[test]
    fn caret_visibility_scrolls_only_far_enough_to_reveal_its_line() {
        assert_eq!(scroll_to_reveal_line(0.0, 3, 20.0, 40.0, 80.0), 40.0);
        assert_eq!(scroll_to_reveal_line(50.0, 1, 20.0, 40.0, 80.0), 20.0);
        assert_eq!(scroll_to_reveal_line(20.0, 2, 20.0, 40.0, 80.0), 20.0);
    }

    #[test]
    fn wheel_delta_clamps_vertical_scroll_to_both_edges() {
        assert_eq!(vertical_scroll_target(10.0, -30.0, 80.0), 40.0);
        assert_eq!(vertical_scroll_target(70.0, -30.0, 80.0), 80.0);
        assert_eq!(vertical_scroll_target(10.0, 30.0, 80.0), 0.0);
    }

    #[test]
    fn scroll_events_move_only_multiline_fields_with_overflow() {
        let mut multiline = focused_field(TextEditingController::with_text(
            "one two three four five six",
        ));
        multiline.max_lines = Some(3);
        multiline.scroll_y_extent.set(40.0);
        let scroll = ElementEvent::Scroll {
            delta: Vec2d { x: 9.0, y: -25.0 },
            phase: TouchPhase::Moved,
            kind: ScrollDeltaKind::Pixel,
            is_direct_manipulation: true,
        };

        let _ = multiline.on_event(&scroll);
        assert_eq!(multiline.scroll_y.get(), 25.0);
        let _ = multiline.on_event(&scroll);
        assert_eq!(multiline.scroll_y.get(), 40.0);

        let mut single_line = focused_field(TextEditingController::with_text(
            "one two three four five six",
        ));
        single_line.max_lines = Some(1);
        single_line.scroll_y_extent.set(40.0);
        let _ = single_line.on_event(&scroll);
        assert_eq!(single_line.scroll_y.get(), 0.0);
    }
}

#[cfg(test)]
mod pointer_capture_tests {
    use aimer_attribute::Vec2d;
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::PointerKey;

    use super::owns_selection_pointer;

    #[test]
    fn selection_drag_matches_pointer_source_and_id() {
        let touch = PointerKey::new(PointerSource::Touch, 0);
        let touch_move = ElementEvent::PointerMove(PointerInfo::touch(Vec2d::default(), 0));
        let mouse_move = ElementEvent::PointerMove(PointerInfo::mouse(
            Vec2d::default(),
            PointerButton::Primary,
        ));

        assert!(owns_selection_pointer(Some(touch), &touch_move));
        assert!(!owns_selection_pointer(Some(touch), &mouse_move));
        assert!(!owns_selection_pointer(None, &touch_move));
    }
}

#[cfg(test)]
mod char_boundary_tests {
    #[test]
    fn composition_ranges_are_clamped_to_character_boundaries() {
        assert_eq!(super::floor_char_boundary("你好", 1), 0);
        assert_eq!(super::floor_char_boundary("你好", 3), 3);
        assert_eq!(super::floor_char_boundary("你好", 99), 6);
    }
}
