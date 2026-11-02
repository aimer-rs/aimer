//! The handle holders store.

use std::fmt;
use std::sync::Arc;

use aimer_attribute::ResolvedSize;
use aimer_widget::AnyWidget;

use super::builtin::{FlatPill, LiquidGlass};
use super::{IndicatorLayer, IndicatorStyle, LayerPresentation};

/// A style of any kind, cheap to clone and compare.
///
/// This is what the framework stores wherever it holds a style: indicators and
/// tab bars accept `impl IndicatorStyle` and keep an `AnyIndicatorStyle`. The
/// standard styles are held inline, so storing one allocates nothing and
/// presenting a layer takes no virtual call; only a custom style is boxed.
/// `AnyIndicatorStyle` is itself an [`IndicatorStyle`].
///
/// Two handles are equal when they are the same standard style with the same
/// parameters, or clones of the same custom style.
#[derive(Clone)]
pub struct AnyIndicatorStyle(Repr);

#[derive(Clone)]
enum Repr {
    LiquidGlass(LiquidGlass),
    FlatPill(FlatPill),
    Custom(Arc<dyn IndicatorStyle>),
}

impl AnyIndicatorStyle {
    /// Wraps `style`. Equivalent to [`IndicatorStyle::into_any`].
    pub fn new(style: impl IndicatorStyle) -> Self {
        style.into_any()
    }

    pub(super) fn liquid_glass(style: LiquidGlass) -> Self {
        Self(Repr::LiquidGlass(style))
    }

    pub(super) fn flat_pill(style: FlatPill) -> Self {
        Self(Repr::FlatPill(style))
    }

    pub(super) fn custom(style: impl IndicatorStyle) -> Self {
        Self(Repr::Custom(Arc::new(style)))
    }

    /// Whether the style is held without an allocation.
    #[cfg(test)]
    pub(super) fn is_inline(&self) -> bool {
        !matches!(self.0, Repr::Custom(_))
    }
}

impl Default for AnyIndicatorStyle {
    /// The [`LIQUID_GLASS`](super::indicator_styles::LIQUID_GLASS) style.
    fn default() -> Self {
        Self::liquid_glass(super::indicator_styles::LIQUID_GLASS)
    }
}

impl IndicatorStyle for AnyIndicatorStyle {
    fn layer(&self, layer: IndicatorLayer, size: ResolvedSize) -> Option<AnyWidget> {
        if !self.is_valid() {
            return None;
        }
        match &self.0 {
            Repr::LiquidGlass(style) => style.layer(layer, size),
            Repr::FlatPill(style) => style.layer(layer, size),
            Repr::Custom(style) => style.layer(layer, size),
        }
    }

    #[inline]
    fn present(&self, layer: IndicatorLayer, lift: f32) -> LayerPresentation {
        match &self.0 {
            Repr::LiquidGlass(style) => style.present(layer, lift),
            Repr::FlatPill(style) => style.present(layer, lift),
            Repr::Custom(style) => style.present(layer, lift),
        }
    }

    fn is_valid(&self) -> bool {
        match &self.0 {
            Repr::LiquidGlass(style) => style.is_valid(),
            Repr::FlatPill(style) => style.is_valid(),
            Repr::Custom(style) => style.is_valid(),
        }
    }

    fn into_any(self) -> AnyIndicatorStyle {
        self
    }
}

impl PartialEq for AnyIndicatorStyle {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Repr::LiquidGlass(a), Repr::LiquidGlass(b)) => a == b,
            (Repr::FlatPill(a), Repr::FlatPill(b)) => a == b,
            (Repr::Custom(a), Repr::Custom(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl fmt::Debug for AnyIndicatorStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::LiquidGlass(style) => write!(f, "{style:?}"),
            Repr::FlatPill(style) => write!(f, "{style:?}"),
            Repr::Custom(_) => f.write_str("Custom"),
        }
    }
}
