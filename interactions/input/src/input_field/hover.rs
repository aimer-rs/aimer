//! The name the pointer rests on, told to the app so that it can show something about it.
//!
//! The field finds the name under a still pointer, as it finds the name that a jump key makes a link, and tells the app once the
//! pointer has been on it for a moment: what the app shows, a card of documentation, is its own. The field takes it back when the
//! pointer goes, or the user does something else, so that what is shown is never about a place the pointer has left.

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

/// Where the name that is rested on is, in the field.
#[derive(Clone, Debug, PartialEq)]
pub struct HoverSpot {
    /// The bytes of the name.
    pub range: Range<usize>,
    /// Where its box is, from the top left corner of the field, in logical pixels: what something shown beside it is placed by.
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Where the part of the field that shows text ends, as the far edges `(right, bottom)` in the same terms, or `None` for a field
    /// that has not been laid out: what something shown beside the name is kept inside, as a menu is.
    pub extent: Option<(f32, f32)>,
}

/// What is told of the name the pointer rests on: see [`TextArea::on_hover`](crate::TextArea::on_hover).
///
/// # Examples
///
/// ```
/// use aimer_input::input::TextHover;
/// use std::time::Duration;
///
/// // Notes the name, if any, that the pointer has rested on for a third of a second.
/// let hover = TextHover::new(Duration::from_millis(300), |spot| {
///     let _ = spot.map(|spot| spot.range);
/// });
/// # let _ = hover;
/// ```
#[derive(Clone)]
pub struct TextHover {
    pub(crate) delay: Duration,
    pub(crate) range_at: Option<Rc<dyn Fn(usize) -> Option<Range<usize>>>>,
    pub(crate) on_change: Rc<dyn Fn(Option<HoverSpot>)>,
    pub(crate) on_dismiss: Option<Rc<dyn Fn()>>,
}

impl TextHover {
    /// Tells `on_change` of the name the pointer has rested on for `delay`: `Some` with where it is, and `None` when what was told
    /// of is to go, which is when the pointer leaves the name or the field, or a press, a key, a scroll, an edit or the loss of the
    /// focus ends it. Nothing is told again for a name that is still rested on, and nothing for a pointer that is dragging.
    ///
    /// A name is a run of letters, digits and underscores, unless [`range_at`](Self::range_at) says what it is.
    pub fn new(delay: Duration, on_change: impl Fn(Option<HoverSpot>) + 'static) -> Self {
        Self { delay, range_at: None, on_change: Rc::new(on_change), on_dismiss: None }
    }

    /// Says what is to be called when what was shown is to go at once: the user pressed in the text, typed or scrolled it, or the
    /// field lost the focus. What was shown can be told by the pointer leaving the name, which may be on its way to what is shown, and
    /// is let stay for a moment; a press, a key or a wheel that reaches the field is not on it, and there is no moment to give.
    /// Called for each of them, whether or not a name is being shown, and not for the pointer leaving.
    pub fn on_dismiss(mut self, on_dismiss: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(on_dismiss));
        self
    }

    /// Says what the name is at a byte of the text: the bytes of the name that holds it, or `None` where it is not a name there is
    /// anything to show of, such as a comment or a keyword. The answer must hold the byte.
    pub fn range_at(mut self, range_at: impl Fn(usize) -> Option<Range<usize>> + 'static) -> Self {
        self.range_at = Some(Rc::new(range_at));
        self
    }
}
