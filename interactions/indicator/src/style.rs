//! How an indicator looks.

mod any;
mod builtin;

use aimer_attribute::ResolvedSize;
use aimer_widget::AnyWidget;

pub use any::AnyIndicatorStyle;
pub use builtin::{FlatPill, LiquidGlass};

/// The standard styles.
pub mod indicator_styles {
    pub use super::builtin::{FLAT_PILL, LIQUID_GLASS, flat_pill};
}

/// One of the two retained layers an indicator is drawn with.
///
/// A frame of an indicator only *presents* these layers, moving and fading
/// them, and never records them again, which is what keeps an animation cheap.
/// A style that changes its look as the indicator lifts therefore does so by
/// cross-fading two layers rather than by redrawing one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndicatorLayer {
    /// Drawn beneath the content, such as the flat pill a selection rests on.
    Rest,
    /// Drawn over the content, such as a lens that magnifies what it covers.
    Lift,
}

/// How one layer is shown for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerPresentation {
    /// Scales the layer about its own center. `1.0` leaves it unscaled.
    pub scale: f32,
    /// Multiplies the layer's opacity, clamped to `0.0..=1.0`. A layer that is
    /// fully transparent is left out of the frame entirely, so a style pays
    /// for an expensive layer, such as a lens that samples the backdrop, only
    /// while that layer is visible.
    pub opacity: f32,
}

impl LayerPresentation {
    /// The layer shown unchanged.
    pub const OPAQUE: Self = Self { scale: 1.0, opacity: 1.0 };
    /// The layer not shown.
    pub const HIDDEN: Self = Self { scale: 1.0, opacity: 0.0 };
}

/// Decides how an indicator looks, at rest and lifted.
///
/// A style has two jobs. [`layer`](Self::layer) builds the retained visual for
/// each [`IndicatorLayer`] once, at the slot's size, and
/// [`present`](Self::present) says how each is shown for a given *lift*, a value
/// from `0.0` at rest to `1.0` fully lifted that the controller animates. Only
/// `present` runs every frame, so it must be pure and allocation free.
///
/// ```
/// use aimer_attribute::ResolvedSize;
/// use aimer_container::SizedBox;
/// use aimer_indicator::{IndicatorLayer, IndicatorStyle, LayerPresentation};
/// use aimer_widget::{AnyWidget, Widget};
///
/// /// A solid block that grows a little when lifted.
/// struct Block;
///
/// impl IndicatorStyle for Block {
///     fn layer(&self, layer: IndicatorLayer, size: ResolvedSize) -> Option<AnyWidget> {
///         (layer == IndicatorLayer::Rest)
///             .then(|| SizedBox::new().width(size.width).height(size.height).boxed())
///     }
///
///     fn present(&self, _layer: IndicatorLayer, lift: f32) -> LayerPresentation {
///         LayerPresentation { scale: 1.0 + 0.1 * lift, opacity: 1.0 }
///     }
/// }
/// ```
///
/// A style must be `Send + Sync + 'static` so that holders can share it freely.
pub trait IndicatorStyle: Send + Sync + 'static {
    /// Builds the retained visual for `layer`, sized to one slot, or `None` if
    /// the style has nothing to draw there.
    ///
    /// It runs once per build of the indicator, not per frame. The widget it
    /// returns should be exactly `size` large and take no pointer input, since
    /// it is drawn over the content.
    fn layer(&self, layer: IndicatorLayer, size: ResolvedSize) -> Option<AnyWidget>;

    /// How `layer` is shown at `lift`.
    ///
    /// `lift` is `0.0` at rest and `1.0` fully lifted, and can leave that range
    /// by a little while a spring settles. This runs every frame for each layer
    /// the style built, so keep it to arithmetic.
    fn present(&self, layer: IndicatorLayer, lift: f32) -> LayerPresentation;

    /// Whether the style's parameters are usable, such as a finite lift scale.
    /// An invalid style draws nothing.
    fn is_valid(&self) -> bool {
        true
    }

    /// Converts this style into the handle holders store.
    ///
    /// The default boxes the style. The standard styles override it to avoid
    /// the allocation.
    fn into_any(self) -> AnyIndicatorStyle
    where
        Self: Sized,
    {
        AnyIndicatorStyle::custom(self)
    }
}
