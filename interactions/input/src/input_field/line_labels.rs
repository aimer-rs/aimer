//! Text after the end of a line, which can be pressed: the "27 usages | 1 implementation" that an IDE shows beside a definition.
//!
//! A [`LineLabels`] answers, for each line the field shows, with the label that goes after its text, if it has one. The label
//! is drawn in the text's own font, a little smaller, on the baseline of the last row of the line, and scrolls with the text.
//! It is made of parts, some of which are buttons: pressing one calls it with where it is, and the press goes no further, so
//! the caret does not move.

use std::rc::Rc;

use aimer_cupid::font::FontFamily;
use aimer_widget::base::Color;

/// A place in a field, in logical pixels from its top left: where a button of a label is, or where the pointer was when the
/// text was force-clicked. What is given for a press, so that what it opens can open beside it, and be kept inside the field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldSpot {
    pub x: f32,
    pub y: f32,
    /// How wide it is: none for a point.
    pub width: f32,
    /// How tall it is: none for a point.
    pub height: f32,
    /// How far the field goes to the right and down, if it is laid out: what a card that hangs from the spot is kept inside.
    pub extent: Option<(f32, f32)>,
}

/// One run of text of a label, and what pressing it does, if it is a button.
#[derive(Clone)]
pub(crate) struct LabelPart {
    pub(crate) text: String,
    pub(crate) press: Option<Rc<dyn Fn(FieldSpot)>>,
}

/// The label of one line: runs of text in one color, some of which are buttons.
///
/// # Examples
///
/// ```
/// use aimer_input::input::LineLabel;
/// use aimer_widget::base::Color;
///
/// let label = LineLabel::new(Color::Rgba(120, 124, 135, 255))
///     .button("27 usages", |_spot| {})
///     .text(" | ")
///     .button("1 implementation", |_spot| {});
/// assert_eq!(label.plain_text(), "27 usages | 1 implementation");
/// ```
#[derive(Clone)]
pub struct LineLabel {
    pub(crate) parts: Vec<LabelPart>,
    pub(crate) color: Color,
    pub(crate) size: f32,
    pub(crate) family: Option<FontFamily>,
    pub(crate) hover: Option<Color>,
}

impl LineLabel {
    /// How large the text of a label is, as a share of the field's own, unless it is said.
    pub const DEFAULT_SIZE: f32 = 0.85;

    /// An empty label in `color`.
    pub fn new(color: impl Into<Color>) -> Self {
        Self { parts: Vec::new(), color: color.into(), size: Self::DEFAULT_SIZE, family: None, hover: None }
    }

    /// The same label, its text `size` times as large as the field's: from half to one and a half.
    #[inline]
    pub fn size(mut self, size: f32) -> Self {
        self.size = if size.is_finite() { size.clamp(0.5, 1.5) } else { Self::DEFAULT_SIZE };
        self
    }

    /// The same label in the font `family`, and not in the text's own: a note that is not code is not set as code is.
    #[inline]
    pub fn font_family(mut self, family: FontFamily) -> Self {
        self.family = Some(family);
        self
    }

    /// The same label, with its buttons in `color` and underlined while the pointer is on them. Without one they are as the rest of
    /// the label is, and the pointer does not change them.
    #[inline]
    pub fn hover_color(mut self, color: impl Into<Color>) -> Self {
        self.hover = Some(color.into());
        self
    }

    /// The color the buttons take under the pointer, if the label says one.
    pub fn hover(&self) -> Option<Color> {
        self.hover
    }

    /// The font the label asks for, if it asks for one.
    pub fn font(&self) -> Option<FontFamily> {
        self.family
    }

    /// Adds text that is not pressed.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.parts.push(LabelPart { text: text.into(), press: None });
        self
    }

    /// Adds text that is a button: pressing it calls `on_press` with where it is.
    pub fn button(mut self, text: impl Into<String>, on_press: impl Fn(FieldSpot) + 'static) -> Self {
        self.parts.push(LabelPart { text: text.into(), press: Some(Rc::new(on_press)) });
        self
    }

    /// Presses the part `part`, counted from zero, as a press on it does, and says whether it was a button: the text that is
    /// not one, and a part there is not, are not pressed.
    pub fn press(&self, part: usize, spot: FieldSpot) -> bool {
        match self.parts.get(part).and_then(|part| part.press.as_ref()) {
            Some(press) => {
                press(spot);
                true
            }
            None => false,
        }
    }

    /// How many parts the label has, the buttons and the rest.
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// Whether the label has no part.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// All the text of the label, the buttons and the rest.
    pub fn plain_text(&self) -> String {
        self.parts.iter().map(|part| part.text.as_str()).collect()
    }
}

/// The labels after the lines of a [`TextArea`](crate::TextArea): see [`LineLabel`].
///
/// `of_line` is asked for each line that is in view, each time the field is drawn, counted from zero, and has to answer
/// quickly: a lookup, and not work. A line that wraps has its label after its last row.
///
/// # Examples
///
/// ```
/// use std::rc::Rc;
/// use aimer_input::input::{LineLabel, LineLabels};
/// use aimer_widget::base::Color;
///
/// let labels = LineLabels::new(16, |line| {
///     (line == 0).then(|| Rc::new(LineLabel::new(Color::Rgba(120, 124, 135, 255)).text("a note")))
/// });
/// # let _ = labels;
/// ```
#[derive(Clone)]
pub struct LineLabels {
    pub(crate) of_line: Rc<dyn Fn(usize) -> Option<Rc<LineLabel>>>,
    pub(crate) gap: f32,
}

impl LineLabels {
    /// Labels `gap` logical pixels after the text of the lines `of_line` gives one.
    pub fn new(gap: u32, of_line: impl Fn(usize) -> Option<Rc<LineLabel>> + 'static) -> Self {
        Self { of_line: Rc::new(of_line), gap: gap as f32 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_is_its_parts_in_order_and_only_the_buttons_are_pressed() {
        let label = LineLabel::new(Color::Rgba(1, 2, 3, 255)).button("a", |_| {}).text(" | ").button("b", |_| {});
        assert_eq!(label.plain_text(), "a | b");
        let pressed: Vec<bool> = label.parts.iter().map(|part| part.press.is_some()).collect();
        assert_eq!(pressed, [true, false, true]);
    }

    #[test]
    fn pressing_a_part_calls_a_button_with_the_spot_and_does_nothing_for_text_or_a_part_that_is_not_there() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        let label = LineLabel::new(Color::Rgba(1, 2, 3, 255))
            .button("a", move |spot| record.borrow_mut().push(spot.x))
            .text(" | ");
        let spot = FieldSpot { x: 7.0, y: 0.0, width: 1.0, height: 1.0, extent: None };
        assert!(label.press(0, spot));
        assert!(!label.press(1, spot), "text");
        assert!(!label.press(2, spot), "no such part");
        assert_eq!(*seen.borrow(), vec![7.0]);
        assert_eq!((label.len(), label.is_empty()), (2, false));
    }

    #[test]
    fn the_size_of_a_label_is_kept_to_what_can_be_read() {
        let size = |size: f32| LineLabel::new(Color::Rgba(0, 0, 0, 255)).size(size).size;
        assert_eq!(size(0.9), 0.9);
        assert_eq!(size(0.0), 0.5);
        assert_eq!(size(9.0), 1.5);
        assert_eq!(size(f32::NAN), LineLabel::DEFAULT_SIZE);
    }
}
