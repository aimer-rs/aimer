//! The name under the pointer, shown as a link while the platform's jump key is held.
//!
//! The field only draws it: what a click on it does is the field's `on_command_click`. It is found by the layout, as
//! the byte of a click is, because only the layout can say which byte is under a position, and it is drawn over the
//! text again, in a brighter colour and underlined, so that the text itself is not laid out again for a pointer move.

use std::ops::Range;

use aimer_attribute::position::Vec2d;
use aimer_events::element::Modifiers;
use aimer_style::{TextDecoration, TextStyle};
use aimer_widget::base::Color;

use aimer_widget::LayoutElement;

use super::{Canvas, EditableGeometry, RawTextField, Rc};

/// How far towards white the colour of a link is taken, out of 255.
const BRIGHTER: u32 = 90;
/// How far from `at` a name is looked for, in bytes: a name is not longer than that.
const REACH: usize = 256;

/// The bytes of the name that the byte `at` of `text` is in: a run of letters, digits and underscores. `None` when
/// what is at `at` is not part of a name, and for a name that is longer than [`REACH`] on either side.
pub(super) fn name_around(text: &str, at: usize) -> Option<Range<usize>> {
    let is_name = |c: char| c.is_alphanumeric() || c == '_';
    if at >= text.len() || !text.is_char_boundary(at) || !text[at..].chars().next().is_some_and(is_name) {
        return None;
    }
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_name(*c))
        .last()
        .map_or(at, |(index, _)| index);
    let end = at + text[at..].char_indices().find(|(_, c)| !is_name(*c)).map_or(text.len() - at, |(index, _)| index);
    (at - start <= REACH && end - at <= REACH).then_some(start..end)
}

/// `color` taken part of the way to white, with its transparency kept.
pub(super) fn brighter(color: Color) -> Color {
    let (red, green, blue, alpha) = color.to_rgba();
    let lift = |channel: u8| (channel as u32 + (255 - channel as u32) * BRIGHTER / 255) as u8;
    Color::Rgba(lift(red), lift(green), lift(blue), alpha)
}

impl RawTextField {
    /// Says that the name under the pointer has to be found again, by the next frame.
    pub(super) fn request_link(&self) {
        self.pending_link.set(true);
        aimer_events::window::request_animation_frame();
    }

    /// The bytes of the name that is shown as a link, if there is one and the text is as it was found.
    #[cfg(test)]
    pub(super) fn link_range(&self) -> Option<Range<usize>> {
        self.link.get().filter(|(_, _, revision)| *revision == self.controller.revision()).map(|(start, end, _)| start..end)
    }

    /// The bytes of the name that is under the point `(x, y)` of the text, which the layout says is at the boundary `byte`.
    ///
    /// The layout gives the nearest boundary between two characters, which is the start of the character the point is
    /// in when it is in its left half and the start of the next one when it is in its right half. So the two characters
    /// either side of it are the ones to look at, and the one whose box has the point in it is the one that is under it.
    pub(super) fn link_at(&self, geometry: &EditableGeometry, point: (f32, f32), byte: usize) -> Option<Range<usize>> {
        self.name_at(geometry, point, byte, &|at| self.link_around(at))
    }

    /// The bytes of the name that is under the point `(x, y)` of the text, where `around` says which name holds a byte: see
    /// [`link_at`](Self::link_at).
    pub(super) fn name_at(&self, geometry: &EditableGeometry, (x, y): (f32, f32), byte: usize, around: &dyn Fn(usize) -> Option<Range<usize>>) -> Option<Range<usize>> {
        let value = self.controller.value();
        let after = (byte < value.len()).then(|| byte..value.ceil_to_boundary(byte + 1));
        let before = byte.checked_sub(1).map(|last| value.floor_to_boundary(last)..byte);
        let under = [after, before].into_iter().flatten().find(|chars| {
            let rects = geometry.selection_rects(value.grapheme_of_byte(chars.start), value.grapheme_of_byte(chars.end));
            rects.iter().any(|rect| rect.x <= x && x < rect.x + rect.width && rect.y <= y && y < rect.y + rect.height)
        })?;
        around(under.start)
    }

    /// The bytes of the name that the byte `byte` is in: the app's say, when it has one, and a run of letters, digits and
    /// underscores when it has not.
    pub(super) fn link_around(&self, byte: usize) -> Option<Range<usize>> {
        if let Some(link_range) = &self.link_range {
            return link_range(byte).filter(|name| name.start <= byte && byte < name.end);
        }
        let value = self.controller.value();
        // The text around the byte and no more of it: a name is short, and the text may not be.
        let from = value.floor_to_boundary(byte.saturating_sub(REACH));
        let to = value.ceil_to_boundary((byte + REACH + 1).min(value.len()));
        let window = value.slice(from..to);
        let name = name_around(&window, byte - from)?;
        Some(from + name.start..from + name.end)
    }

    /// Finds the name under the pointer, if the jump key is held and there is one, after a move or a change of the keys.
    pub(super) fn resolve_link(
        &self,
        geometry: &Rc<EditableGeometry>,
        scale: f32,
        (abs_x, abs_y): (f32, f32),
        (pad_left, pad_top): (f32, f32),
        base_y: f32,
        is_multiline: bool,
    ) {
        if !self.pending_link.take() {
            return;
        }
        let found = self.hover_pos.get().filter(|_| Modifiers::current().command()).and_then(|pos: Vec2d| {
            let x = pos.x * scale - abs_x - pad_left + self.scroll_x_offset();
            let y = pos.y * scale - abs_y - pad_top - base_y + if is_multiline { self.scroll_y.get() } else { 0.0 };
            let grapheme = geometry.hit_test(x, y)?;
            let byte = self.controller.value().byte_of_grapheme(grapheme);
            let name = self.link_at(geometry, (x, y), byte)?;
            Some((name.start, name.end, self.controller.revision()))
        });
        if found != self.link.replace(found) {
            // The pointer is a hand over a link and the text cursor over the rest of the text, which is what the
            // move that brought it here set before the name was known.
            if self.hover_pos.get().is_some() {
                aimer_utils::cursor::set_cursor(if found.is_some() {
                    aimer_utils::cursor::CursorIcon::Pointer
                } else {
                    aimer_utils::cursor::CursorIcon::Text
                });
            }
            aimer_events::window::request_animation_frame();
        }
    }

    /// Draws the name that is a link over the text again, brighter and underlined.
    pub(super) fn paint_link(
        &self,
        canvas: &Canvas,
        text_context: &aimer_widget::base::BuildContext<'_>,
        geometry: &EditableGeometry,
        style: &TextStyle,
        scale: f32,
        text_origin: Vec2d,
    ) {
        let Some((start, end, revision)) = self.link.get() else {
            return;
        };
        if revision != self.controller.revision() || !Modifiers::current().command() {
            return;
        }
        let value = self.controller.value();
        let rects = geometry.selection_rects(value.grapheme_of_byte(start), value.grapheme_of_byte(end));
        // A name that is broken over two lines is not drawn: it would need two pieces.
        let [rect] = rects.as_slice() else {
            return;
        };
        let name = value.slice(start..end);
        // The colour the highlighter gave the name, and the text's own where it gave none.
        let styled = geometry
            .ranges
            .as_ref()
            .and_then(|ranges| ranges.iter().find(|range| range.range.start <= start && start < range.range.end))
            .map(|range| range.style.clone());
        let mut link = *style;
        if let Some(styled) = styled {
            link.color = styled.color.unwrap_or(link.color);
            link.font_style = styled.font_style.unwrap_or(link.font_style);
            link.font_weight = styled.font_weight.unwrap_or(link.font_weight);
        }
        link.color = brighter(link.color);
        link.text_decoration = TextDecoration::Underline;
        let widget = self.build_text_widget(&name, &link, aimer_style::TextAlign::TopLeft);
        let _ = widget.computed_size(text_context);
        let recorded = widget.record_local_v2_into(
            canvas,
            text_context,
            Vec2d { x: text_origin.x + rect.x / scale, y: text_origin.y + rect.y / scale },
            false,
            |_| {},
        );
        debug_assert!(recorded, "link text v2 support is the field's, which was checked before painting");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_the_letters_digits_and_underscores_around_a_byte() {
        let text = "let foo_bar2 = baz(1);";
        assert_eq!(name_around(text, 4), Some(4..12), "its first letter");
        assert_eq!(name_around(text, 11), Some(4..12), "its last");
        assert_eq!(name_around(text, 7), Some(4..12), "the middle");
        assert_eq!(name_around(text, 15), Some(15..18), "a name followed by a bracket");
    }

    #[test]
    fn what_is_not_in_a_name_is_not_a_name() {
        let text = "a  = (b)";
        assert_eq!(name_around(text, 1), None, "a space");
        assert_eq!(name_around(text, 3), None, "a sign");
        assert_eq!(name_around(text, 8), None, "the end of the text");
        assert_eq!(name_around("", 0), None, "no text");
    }

    #[test]
    fn a_name_is_found_in_bytes_when_it_has_wide_letters_and_in_a_text_without_a_final_break() {
        // 'é' is two bytes and 'ж' two: the name is bytes 1..7, and the byte of its last letter is 5.
        let text = "(éabж)";
        assert_eq!(name_around(text, 1), Some(1..7));
        assert_eq!(name_around(text, 5), Some(1..7));
        assert_eq!(name_around(text, 2), None, "the middle of a letter is not a byte to start from");
        assert_eq!(name_around("end", 2), Some(0..3));
    }

    #[test]
    fn a_name_that_is_far_too_long_is_not_found_and_does_not_cost_the_text() {
        let long = "x".repeat(10_000);
        assert_eq!(name_around(&long, 5_000), None);
        assert_eq!(name_around("x_1", 0), Some(0..3));
    }

    #[test]
    fn a_link_is_the_colour_of_the_name_brightened_and_keeps_its_transparency() {
        assert_eq!(brighter(Color::Rgba(0, 0, 0, 10)), Color::Rgba(90, 90, 90, 10));
        assert_eq!(brighter(Color::Rgba(255, 255, 255, 255)), Color::Rgba(255, 255, 255, 255));
        let (red, green, blue, _) = brighter(Color::Rgba(40, 120, 200, 255)).to_rgba();
        assert!(red > 40 && green > 120 && blue > 200);
    }
}
