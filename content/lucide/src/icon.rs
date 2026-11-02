//! The [`Icon`] widget: a Lucide icon with the common adjustments built in.

use aimer_attribute::dimension::Dimension;
use aimer_svg::{Svg, SvgColor, SvgDocument, SvgStyle};
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, PortableWidget, Widget};

/// The placeholder an [`Icon`] carries until it is given an icon.
///
/// `Icon<IconDataRequired>` is deliberately not a [`Widget`], so forgetting
/// [`Icon::data`] is a compile error rather than an empty box at run time:
///
/// ```compile_fail
/// use aimer_lucide::Icon;
/// use aimer_widget::Widget;
///
/// // No icon was chosen, so there is nothing to draw.
/// let _ = Icon::new().size(24).boxed();
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IconDataRequired;

/// A Lucide icon widget with its size and color set directly.
///
/// Lucide icons are outlines: every shape is stroked with `currentColor` and
/// left unfilled. `Icon` wraps [`aimer_svg::Svg`] and turns the usual choices —
/// how big, what color — into one method each, so an icon needs no selector
/// string and no [`SvgStyle`] for the common case.
///
/// Create one with [`Icon::of`], or with [`Icon::new`] followed by
/// [`Icon::data`]. Everything else is optional and may be set in any order, before
/// or after the icon itself.
///
/// # Sizing
///
/// With neither [`size`](Icon::size), [`width`](Icon::width), nor
/// [`height`](Icon::height) set, an icon renders at its intrinsic `24 x 24`
/// viewport. Setting one dimension derives the other from the aspect ratio, and
/// Lucide icons are square, so `size(32)` and `width(32)` are the same.
/// Dimensions are logical pixels and follow the display scale factor.
///
/// # Color
///
/// [`color`](Icon::color) replaces the stroke on every shape of the icon,
/// which is the outline you see. Without it the icon keeps the color the
/// document declares. [`fill`](Icon::fill) paints the inside of closed shapes
/// for a solid look, usually together with a matching stroke. The stroke
/// *width* belongs to the icon file (`2` for all of Lucide) and is not
/// adjustable here; it scales with the icon.
///
/// # Examples
///
/// A 24-pixel icon in the accent color:
///
/// ```
/// use aimer_lucide::{Icon, Icons};
/// use aimer_svg::SvgColor;
///
/// let search = Icon::of(Icons::SEARCH).size(24).color(SvgColor::rgba8(0, 128, 255, 255));
/// ```
///
/// A solid, half-transparent star:
///
/// ```
/// use aimer_lucide::{Icon, Icons};
/// use aimer_svg::SvgColor;
///
/// let gold = SvgColor::rgba8(245, 180, 0, 255);
/// let star = Icon::of(Icons::STAR).size(20).color(gold).fill(gold).opacity(0.5);
/// ```
///
/// Any [`SvgDocument`] works as well as a bundled icon, and the type-state
/// builder reads the same way:
///
/// ```
/// use aimer_lucide::Icon;
/// use aimer_svg::SvgDocument;
///
/// let document = SvgDocument::from_svg(
///     br#"<svg viewBox="0 0 24 24"><path d="M2 2h20v20z"/></svg>"#,
/// ).unwrap();
/// let custom = Icon::new().data(document).size(16);
/// ```
///
/// An icon is a regular widget, so it can be boxed and handed to any container
/// that takes children, such as a `Row`:
///
/// ```
/// use aimer_lucide::{Icon, Icons};
/// use aimer_widget::{AnyWidget, Widget};
///
/// let child: AnyWidget = Icon::of(Icons::HOUSE).size(18).boxed();
/// ```
///
/// For per-path styling, hover or pressed styles, or press callbacks, build an
/// [`aimer_svg::Svg`] from the same constant instead.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon<D = IconDataRequired> {
    data: D,
    width: Option<Dimension>,
    height: Option<Dimension>,
    style: SvgStyle,
}

impl Icon {
    /// Creates an icon that still needs its icon.
    ///
    /// Finish it with [`Icon::data`]. Use [`Icon::of`] to do both at once.
    #[inline]
    pub fn new() -> Self {
        Self {
            data: IconDataRequired,
            width: None,
            height: None,
            style: SvgStyle::new(),
        }
    }

    /// Creates an icon for `data` in one call.
    ///
    /// Equivalent to `Icon::new().data(data)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_lucide::{Icon, Icons};
    ///
    /// let house = Icon::of(Icons::HOUSE);
    /// ```
    #[inline]
    pub fn of(data: impl Into<SvgDocument>) -> Icon<SvgDocument> {
        Self::new().data(data)
    }
}

impl Default for Icon {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<D> Icon<D> {
    /// Sets which icon to draw, completing the builder.
    ///
    /// Accepts a bundled constant such as `Icons::HOUSE` or any parsed
    /// [`SvgDocument`]. A bundled icon shares one retained scene across every
    /// widget that uses it, so this clones a handle, not the drawing. Calling it
    /// again replaces the icon and keeps the size and color.
    ///
    /// The first use of a bundled icon in a debug build parses it; release
    /// builds ship it already parsed.
    ///
    /// # Panics
    ///
    /// Panics if `data` is an embedded source that cannot be parsed, which
    /// cannot happen for the bundled collection.
    #[inline]
    pub fn data(self, data: impl Into<SvgDocument>) -> Icon<SvgDocument> {
        Icon {
            data: data.into(),
            width: self.width,
            height: self.height,
            style: self.style,
        }
    }

    /// Makes the icon a square of `size` logical pixels.
    ///
    /// Shorthand for setting [`width`](Self::width) and [`height`](Self::height)
    /// to the same value. Accepts any [`Dimension`], so a plain number is
    /// logical pixels.
    #[inline]
    pub fn size(mut self, size: impl Into<Dimension>) -> Self {
        let size = size.into();
        self.width = Some(size.clone());
        self.height = Some(size);
        self
    }

    /// Sets the width in logical pixels.
    ///
    /// When the height is not set it follows from the icon's aspect ratio.
    #[inline]
    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.width = Some(width.into());
        self
    }

    /// Sets the height in logical pixels.
    ///
    /// When the width is not set it follows from the icon's aspect ratio.
    #[inline]
    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.height = Some(height.into());
        self
    }

    /// Sets the color of the icon's outline.
    ///
    /// Replaces the stroke of every shape. The alpha channel of `color` is
    /// honored, and [`opacity`](Self::opacity) applies on top of it.
    #[inline]
    pub fn color(mut self, color: impl Into<SvgColor>) -> Self {
        self.style = self.style.stroke(color);
        self
    }

    /// Fills the inside of the icon's closed shapes with `color`.
    ///
    /// Lucide icons are unfilled by default. Open strokes such as a check mark
    /// have no inside and are unaffected.
    #[inline]
    pub fn fill(mut self, color: impl Into<SvgColor>) -> Self {
        self.style = self.style.fill(color);
        self
    }

    /// Sets the opacity of the whole icon, clamped to `0.0..=1.0`.
    #[inline]
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.style = self.style.opacity(opacity);
        self
    }

    /// Replaces the style applied to every shape of the icon.
    ///
    /// This discards any earlier [`color`](Self::color), [`fill`](Self::fill), and
    /// [`opacity`](Self::opacity), so call it first, or use it instead of them.
    /// It is the way to reach properties the dedicated methods do not cover,
    /// such as a transform.
    #[inline]
    pub fn style(mut self, style: SvgStyle) -> Self {
        self.style = style;
        self
    }
}

impl PortableWidget for Icon<SvgDocument> {}

impl Widget for Icon<SvgDocument> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let mut svg = Svg::new(self.data);
        if let Some(width) = self.width {
            svg = svg.width(width);
        }
        if let Some(height) = self.height {
            svg = svg.height(height);
        }
        // Every Lucide shape — circles, rectangles, and lines included — becomes
        // a `path` node, so one selector reaches the whole icon.
        if self.style != SvgStyle::new() {
            svg = svg.style("path", self.style);
        }
        svg.to_element(ctx)
    }

    #[inline]
    fn debug_name(&self) -> &'static str {
        "Icon"
    }
}
