//! The line numbers beside a text area, and the band behind the line the caret is on.

use std::rc::Rc;
use std::sync::Arc;

use aimer_cupid::svg::SvgScene;
use aimer_widget::base::Color;

use crate::input_field::highlight::HighlightContext;

/// Which side of the margin the line numbers are set against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineNumberAlign {
    /// Against the text: the last digits of the numbers line up. The default.
    #[default]
    Right,
    /// Against the left edge of the margin: the first digits of the numbers line up.
    Left,
    /// In the middle of the column of numbers: a number has as much room before it as after it.
    Center,
}

/// How the line numbers in the margin of a [`TextArea`](crate::TextArea) look.
///
/// The margin is as wide as the largest line number needs, plus the gaps either side, and its
/// numbers are in the text's own font, aligned as [`align`](Self::align) says (right by default), one on the first row of each line (a line
/// that wraps shows its number once). The number of the line the caret is on is drawn in
/// [`active_color`](Self::active_color).
///
/// # Examples
///
/// ```
/// use aimer_input::input::LineNumbers;
/// use aimer_widget::base::Color;
///
/// let numbers = LineNumbers::new()
///     .color(Color::Rgba(120, 124, 135, 255))
///     .active_color(Color::Rgba(220, 223, 228, 255))
///     .gaps(12, 12);
/// # let _ = numbers;
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineNumbers {
    pub(crate) color: Color,
    pub(crate) active_color: Color,
    pub(crate) gap_before: f32,
    pub(crate) gap_after: f32,
    pub(crate) min_digits: usize,
    pub(crate) align: LineNumberAlign,
}

impl LineNumbers {
    /// Line numbers in a muted grey, with a 12 logical pixel gap either side and room for two digits.
    #[inline]
    pub fn new() -> Self {
        Self {
            color: Color::Rgba(128, 128, 128, 255),
            active_color: Color::Rgba(220, 220, 220, 255),
            gap_before: 12.0,
            gap_after: 12.0,
            min_digits: 2,
            align: LineNumberAlign::Right,
        }
    }

    /// Which side of the margin the numbers are set against: [`LineNumberAlign::Left`] puts the first
    /// digit of every number at the same place, after the gap before, and [`LineNumberAlign::Center`]
    /// centers each number in the column between the gaps.
    #[inline]
    #[must_use]
    pub fn align(mut self, align: LineNumberAlign) -> Self {
        self.align = align;
        self
    }

    /// The color of the numbers.
    #[inline]
    #[must_use]
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = color.into();
        self
    }

    /// The color of the number of the line the caret is on.
    #[inline]
    #[must_use]
    pub fn active_color(mut self, color: impl Into<Color>) -> Self {
        self.active_color = color.into();
        self
    }

    /// The gap before the numbers (between the left edge of the area and them) and after them
    /// (between them and the text), in logical pixels.
    #[inline]
    #[must_use]
    pub fn gaps(mut self, before: u32, after: u32) -> Self {
        self.gap_before = before as f32;
        self.gap_after = after as f32;
        self
    }

    /// The fewest digits the margin makes room for, so it does not grow as a file passes 99 or
    /// 999 lines. The default is two.
    #[inline]
    #[must_use]
    pub fn min_digits(mut self, digits: usize) -> Self {
        self.min_digits = digits.max(1);
        self
    }
}

impl Default for LineNumbers {
    fn default() -> Self {
        Self::new()
    }
}

/// An icon in the margin of a text area, before a line's number, that does something when pressed:
/// fold or unfold, run, toggle how the line is shown.
///
/// It is drawn from an SVG `scene` filling the marker's square, and pressing it calls `on_press` and
/// nothing else: the caret does not move.
#[derive(Clone)]
pub struct GutterMarker {
    pub(crate) scene: Arc<SvgScene>,
    pub(crate) on_press: Rc<dyn Fn()>,
}

impl GutterMarker {
    /// A marker drawn as `scene`, which calls `on_press` when pressed.
    pub fn new(scene: Arc<SvgScene>, on_press: impl Fn() + 'static) -> Self {
        Self { scene, on_press: Rc::new(on_press) }
    }
}

/// The column of [`GutterMarker`]s in the margin of a text area, before the line numbers: which line
/// has one, and how big the column is.
///
/// The text area asks for the marker of each line it is about to draw, so the answer is made only for
/// the lines on screen, and it is asked again whenever the area is drawn: it may depend on state that
/// changes. The column is as wide as a marker is tall, and takes its room from the margin, so it needs
/// [`LineNumbers`] to be on.
///
/// # Examples
///
/// ```
/// use aimer_input::input::GutterMarkers;
///
/// // A 16 px column in which no line has a marker yet.
/// let markers = GutterMarkers::new(16, |_line| None);
/// # let _ = markers;
/// ```
#[derive(Clone)]
pub struct GutterMarkers {
    pub(crate) of_line: Rc<dyn Fn(usize) -> Option<GutterMarker>>,
    pub(crate) size: f32,
}

impl GutterMarkers {
    /// A column of markers `size` logical pixels wide, in which `of_line` says what each line, counted
    /// from zero, has, if anything.
    pub fn new(size: u32, of_line: impl Fn(usize) -> Option<GutterMarker> + 'static) -> Self {
        Self { of_line: Rc::new(of_line), size: size as f32 }
    }
}

/// A colored bar down the left of some lines of a text area, behind the text, like the rule beside a
/// quotation.
///
/// The text area asks, for each line it draws, whether the line has a bar and where it stands: the
/// answer is made again each time the area is drawn, and is given the [`HighlightContext`] the text
/// is styled with, so a bar can follow what the caret is on the way a highlighter does. A bar is as
/// tall as the rows of its line, so the bars of consecutive lines make one rule.
///
/// # Examples
///
/// ```
/// use aimer_input::input::{LineBar, LineBars};
/// use aimer_widget::base::Color;
///
/// // A 2 px bar, 4 px from the left of the text, beside every third line.
/// let bars = LineBars::new(2, |line, _context| {
///     (line % 3 == 0).then(|| LineBar::new(Color::Rgba(120, 120, 140, 255), 4))
/// });
/// # let _ = bars;
/// ```
#[derive(Clone)]
pub struct LineBars {
    pub(crate) of_line: Rc<dyn Fn(usize, &HighlightContext, &mut Vec<LineBar>)>,
    pub(crate) width: f32,
}

/// The bar of one line: its color, and how far from the left of the text it stands, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineBar {
    pub(crate) color: Color,
    pub(crate) offset: f32,
    pub(crate) width: Option<f32>,
}

impl LineBar {
    /// A bar of `color`, `offset` logical pixels in from the left of the text.
    pub fn new(color: impl Into<Color>, offset: u32) -> Self {
        Self { color: color.into(), offset: offset as f32, width: None }
    }

    /// The same bar, `width` logical pixels wide and not as wide as the bars of the text area are.
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

impl LineBars {
    /// Bars `width` logical pixels wide, for the lines `of_line` gives one, counted from zero.
    pub fn new(width: u32, of_line: impl Fn(usize, &HighlightContext) -> Option<LineBar> + 'static) -> Self {
        Self::many(width, move |line, context, bars| bars.extend(of_line(line, context)))
    }

    /// Bars `width` logical pixels wide, unless a bar says otherwise, for the lines `of_line` gives any number of: it puts them
    /// in the list it is given, which is empty. A column of guides down the indents of code is a bar for each level.
    ///
    /// The list is kept from one line to the next, so that asking about a line costs no allocation of its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::input::{LineBar, LineBars};
    /// use aimer_widget::base::Color;
    ///
    /// // A thin guide every 28 px for as many levels as the line has, here its number.
    /// let guides = LineBars::many(1, |line, _context, bars| {
    ///     for level in 0..line.min(4) {
    ///         bars.push(LineBar::new(Color::Rgba(120, 120, 140, 120), level as u32 * 28));
    ///     }
    /// });
    /// # let _ = guides;
    /// ```
    pub fn many(width: u32, of_line: impl Fn(usize, &HighlightContext, &mut Vec<LineBar>) + 'static) -> Self {
        Self { of_line: Rc::new(of_line), width: width as f32 }
    }
}

/// How many decimal digits `number` has.
pub(crate) fn digits_of(mut number: usize) -> usize {
    let mut digits = 1;
    while number >= 10 {
        number /= 10;
        digits += 1;
    }
    digits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_counted_in_decimal() {
        for (number, digits) in [(0, 1), (9, 1), (10, 2), (99, 2), (100, 3), (12345, 5), (usize::MAX, 20)] {
            assert_eq!(digits_of(number), digits, "{number}");
        }
    }

    #[test]
    fn the_builder_keeps_what_it_is_given_and_never_asks_for_no_digits() {
        let numbers = LineNumbers::new().gaps(4, 8).min_digits(0);
        assert_eq!((numbers.gap_before, numbers.gap_after, numbers.min_digits), (4.0, 8.0, 1));
    }
}
