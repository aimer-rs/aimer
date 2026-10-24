//! What a context menu looks like.
//!
//! The look is described with the framework's own vocabulary — a
//! [`BoxDecoration`] for the panel, a [`TextStyle`] for the labels,
//! [`LayoutSpacing`] for the padding — so a menu is styled exactly the way a
//! `Container` is, and a house style can be built once and passed around as one
//! value.
//!
//! Every shape has a default that matches what its platform draws, so nothing
//! here has to be set to get a menu that looks right; see
//! [`ContextMenuStyle::for_shape`].

use aimer_style::{BoxDecoration, LayoutSpacing, Spacing, TextStyle};
use aimer_widget::base::Color;

use crate::shape::ContextMenuShape;

/// The dark, nearly opaque panel behind the labels.
pub const PANEL_COLOR: Color = Color::Rgba(58, 58, 60, 242);

/// The labels.
pub const LABEL_COLOR: Color = Color::Rgba(255, 255, 255, 255);

/// The labels of rows that cannot be chosen.
pub const DISABLED_LABEL_COLOR: Color = Color::Rgba(255, 255, 255, 102);

/// The hairline between two items of a pill.
pub const SEPARATOR_COLOR: Color = Color::Rgba(255, 255, 255, 38);

/// The wash under the row a pointer is pressing or hovering.
pub const HIGHLIGHT_COLOR: Color = Color::Rgba(255, 255, 255, 36);

/// Height of one row of the pill, in logical pixels.
pub const PILL_HEIGHT: f32 = 40.0;

/// Horizontal padding around each label of the pill, in logical pixels.
pub const PILL_ITEM_PADDING: f32 = 16.0;

/// Size of the pill's labels, in logical pixels.
pub const PILL_FONT_SIZE: u32 = 15;

/// Distance kept between the anchor and the pill, in logical pixels.
pub const PILL_GAP: f32 = 10.0;

/// Distance kept between a menu and the edges of the window, in logical pixels.
///
/// The region the system reserves — a status bar, a home indicator — is added
/// to this by [`aimer_modal::Floating`]; this is only what keeps a menu off the
/// bare edge of a window that reserves nothing.
pub const SCREEN_MARGIN: f32 = 8.0;

/// Height of one row of the list, in logical pixels.
pub const LIST_ROW_HEIGHT: f32 = 28.0;

/// Horizontal padding around each label of the list, in logical pixels.
pub const LIST_ITEM_PADDING: f32 = 14.0;

/// Size of the list's labels, in logical pixels.
pub const LIST_FONT_SIZE: u32 = 13;

/// Padding above the first row and below the last, in logical pixels.
pub const LIST_VERTICAL_PADDING: u32 = 5;

/// Narrowest a list may be, in logical pixels.
///
/// A menu whose width tracked its longest label alone would jump about between
/// openings; desktop menus have a floor for exactly that reason.
pub const LIST_MIN_WIDTH: f32 = 150.0;

/// Corner radius of the list panel, in logical pixels.
pub const LIST_RADIUS: f32 = 8.0;

/// The look of a context menu: its panel, its rows and its labels.
///
/// # Examples
///
/// ```
/// use aimer_ctxmenu::{ContextMenuShape, ContextMenuStyle};
/// use aimer_style::{BoxDecoration, TextStyle};
/// use aimer_widget::base::Color;
///
/// // The default look of each shape, ready to use.
/// let list = ContextMenuStyle::for_shape(ContextMenuShape::List);
/// assert_eq!(list.min_width, 150.0);
///
/// // Or a house style, in the same vocabulary as any `Container`.
/// let branded = ContextMenuStyle::for_shape(ContextMenuShape::List)
///     .panel(
///         BoxDecoration::new()
///             .background_color(Color::Rgba(24, 24, 27, 250))
///             .border_radius(12.0),
///     )
///     .label(TextStyle::new().font_size(14))
///     .row_height(32.0);
///
/// assert_eq!(branded.row_height, 32.0);
/// assert_eq!(branded.label.font_size, 14);
/// ```
#[derive(Clone)]
pub struct ContextMenuStyle {
    /// The panel behind the rows: its fill, radius, border and shadow.
    pub panel: BoxDecoration,
    /// Space kept between the panel's edge and its content.
    pub padding: LayoutSpacing,
    /// The labels' font, size and colour.
    pub label: TextStyle,
    /// Colour of a label that cannot be chosen.
    pub disabled_label_color: Color,
    /// The wash under the row a pointer is pressing or hovering.
    pub highlight_color: Color,
    /// The hairline drawn between two tiled items.
    pub separator_color: Color,
    /// Height of one row, in logical pixels.
    pub row_height: f32,
    /// Horizontal padding around one label, in logical pixels.
    pub item_padding: f32,
    /// Narrowest the panel's content may be, in logical pixels.
    pub min_width: f32,
    /// Distance kept between the anchor and the panel, in logical pixels.
    pub gap: f32,
    /// Distance kept between the panel and the edges of the window, in logical
    /// pixels.
    ///
    /// Whatever the system reserves for itself is respected on top of this, so
    /// a menu opened next to the status bar or the home indicator stays
    /// pressable.
    pub screen_margin: f32,
}

impl Default for ContextMenuStyle {
    #[inline]
    fn default() -> Self {
        Self::for_shape(ContextMenuShape::default())
    }
}

impl ContextMenuStyle {
    /// The default look of `shape`.
    #[inline]
    pub fn for_shape(shape: ContextMenuShape) -> Self {
        match shape {
            ContextMenuShape::Pill => Self::pill(),
            ContextMenuShape::List => Self::list(),
        }
    }

    /// The horizontal bar a touch screen expects: tall rows, a fully rounded
    /// panel, no padding, floating clear of what it acts on.
    pub fn pill() -> Self {
        Self {
            panel: BoxDecoration::new()
                .background_color(PANEL_COLOR)
                .border_radius(PILL_HEIGHT * 0.5),
            padding: LayoutSpacing::new(),
            label: label_style(PILL_FONT_SIZE),
            disabled_label_color: DISABLED_LABEL_COLOR,
            highlight_color: HIGHLIGHT_COLOR,
            separator_color: SEPARATOR_COLOR,
            row_height: PILL_HEIGHT,
            item_padding: PILL_ITEM_PADDING,
            min_width: 0.0,
            gap: PILL_GAP,
            screen_margin: SCREEN_MARGIN,
        }
    }

    /// The vertical list a desktop expects: short rows, a small radius, padding
    /// above the first row and below the last, opening right at the pointer.
    pub fn list() -> Self {
        Self {
            panel: BoxDecoration::new()
                .background_color(PANEL_COLOR)
                .border_radius(LIST_RADIUS),
            padding: LayoutSpacing::vertical(Spacing::Px(LIST_VERTICAL_PADDING)),
            label: label_style(LIST_FONT_SIZE),
            disabled_label_color: DISABLED_LABEL_COLOR,
            highlight_color: HIGHLIGHT_COLOR,
            separator_color: SEPARATOR_COLOR,
            row_height: LIST_ROW_HEIGHT,
            item_padding: LIST_ITEM_PADDING,
            min_width: LIST_MIN_WIDTH,
            gap: 0.0,
            screen_margin: SCREEN_MARGIN,
        }
    }

    /// Sets the panel's fill, radius, border and shadow.
    #[inline]
    pub fn panel(mut self, panel: BoxDecoration) -> Self {
        self.panel = panel;
        self
    }

    /// Sets the space between the panel's edge and its content.
    #[inline]
    pub fn padding(mut self, padding: LayoutSpacing) -> Self {
        self.padding = padding;
        self
    }

    /// Sets the labels' font, size and colour.
    #[inline]
    pub fn label(mut self, label: TextStyle) -> Self {
        self.label = label;
        self
    }

    /// Sets the colour of a label that cannot be chosen.
    #[inline]
    pub fn disabled_label_color(mut self, color: Color) -> Self {
        self.disabled_label_color = color;
        self
    }

    /// Sets the wash under the row a pointer is pressing or hovering.
    #[inline]
    pub fn highlight_color(mut self, color: Color) -> Self {
        self.highlight_color = color;
        self
    }

    /// Sets the hairline drawn between two tiled items.
    #[inline]
    pub fn separator_color(mut self, color: Color) -> Self {
        self.separator_color = color;
        self
    }

    /// Sets the height of one row, in logical pixels.
    #[inline]
    pub fn row_height(mut self, row_height: f32) -> Self {
        self.row_height = row_height;
        self
    }

    /// Sets the horizontal padding around one label, in logical pixels.
    #[inline]
    pub fn item_padding(mut self, item_padding: f32) -> Self {
        self.item_padding = item_padding;
        self
    }

    /// Sets the narrowest the panel's content may be, in logical pixels.
    #[inline]
    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width;
        self
    }

    /// Sets the distance kept between the anchor and the panel.
    #[inline]
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Sets the distance kept between the panel and the edges of the window.
    #[inline]
    pub fn screen_margin(mut self, screen_margin: f32) -> Self {
        self.screen_margin = screen_margin;
        self
    }

    /// The panel's corner radius for a panel of `width` by `height` physical
    /// pixels, which is also the radius of a tiled item at either end.
    #[inline]
    pub fn panel_radius(&self, width: f32, height: f32, scale: f32) -> [f32; 4] {
        self.panel.border_radius.resolve(width, height, scale)
    }
}

/// A label style with a fixed size, white and unhinted by any theme, because a
/// menu paints its own dark panel and cannot inherit the page's text colour.
#[inline]
fn label_style(font_size: u32) -> TextStyle {
    TextStyle::new().font_size(font_size).color(LABEL_COLOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_shape_has_the_look_its_platform_draws() {
        let pill = ContextMenuStyle::for_shape(ContextMenuShape::Pill);
        let list = ContextMenuStyle::for_shape(ContextMenuShape::List);

        assert_eq!(pill.row_height, PILL_HEIGHT);
        assert_eq!(pill.gap, PILL_GAP, "the pill floats clear of the selection");
        assert_eq!(pill.min_width, 0.0, "a pill is only as wide as its verbs");

        assert_eq!(list.row_height, LIST_ROW_HEIGHT);
        assert_eq!(list.gap, 0.0, "the list opens right at the pointer");
        assert_eq!(list.min_width, LIST_MIN_WIDTH);
    }

    #[test]
    fn every_shape_keeps_itself_off_the_window_edge() {
        for shape in [ContextMenuShape::Pill, ContextMenuShape::List] {
            assert_eq!(
                ContextMenuStyle::for_shape(shape).screen_margin,
                SCREEN_MARGIN,
                "a menu flush against the edge of the screen is hard to press"
            );
        }
    }

    #[test]
    fn a_pill_is_as_round_as_it_is_tall() {
        let pill = ContextMenuStyle::pill();

        assert_eq!(
            pill.panel_radius(200.0, PILL_HEIGHT, 1.0),
            [PILL_HEIGHT * 0.5; 4]
        );
    }

    #[test]
    fn every_part_of_the_look_can_be_replaced() {
        let styled = ContextMenuStyle::list()
            .panel(BoxDecoration::new().background_color(Color::Rgba(1, 2, 3, 4)))
            .padding(LayoutSpacing::all(Spacing::Px(3)))
            .label(TextStyle::new().font_size(20))
            .disabled_label_color(Color::Rgba(5, 5, 5, 5))
            .highlight_color(Color::Rgba(6, 6, 6, 6))
            .separator_color(Color::Rgba(7, 7, 7, 7))
            .row_height(11.0)
            .item_padding(12.0)
            .min_width(13.0)
            .gap(14.0)
            .screen_margin(15.0);

        assert_eq!(
            styled.panel.background_color.get(),
            Some(Color::Rgba(1, 2, 3, 4))
        );
        assert!(styled.padding == LayoutSpacing::all(Spacing::Px(3)));
        assert_eq!(styled.label.font_size, 20);
        assert_eq!(styled.disabled_label_color, Color::Rgba(5, 5, 5, 5));
        assert_eq!(styled.highlight_color, Color::Rgba(6, 6, 6, 6));
        assert_eq!(styled.separator_color, Color::Rgba(7, 7, 7, 7));
        assert_eq!(styled.row_height, 11.0);
        assert_eq!(styled.item_padding, 12.0);
        assert_eq!(styled.min_width, 13.0);
        assert_eq!(styled.gap, 14.0);
        assert_eq!(styled.screen_margin, 15.0);
    }

    #[test]
    fn the_default_look_is_the_default_shapes() {
        let default = ContextMenuStyle::default();
        let shape = ContextMenuStyle::for_shape(ContextMenuShape::default());

        assert_eq!(default.row_height, shape.row_height);
        assert_eq!(default.gap, shape.gap);
        assert_eq!(default.panel, shape.panel);
    }
}
