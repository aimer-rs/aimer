//! The standard indicator styles.

use aimer_attribute::ResolvedSize;
use aimer_container::SizedBox;
use aimer_decoration::{
    BorderRadius, BoxDecoration, DecorationBox, LightEdge, Magnification, MagnificationStyle,
    MultiDecorationBox,
};
use aimer_widget::base::Color;
use aimer_widget::AnyWidget;

use super::{AnyIndicatorStyle, IndicatorLayer, IndicatorStyle, LayerPresentation};

/// The fill of a pill at rest: iOS's translucent system grey.
const REST_FILL: Color = Color::Rgba(120, 120, 128, 46);

/// A rest pill that, when lifted, turns into a liquid glass lens.
///
/// At rest it is a flat, translucent pill beneath the content. As the indicator
/// lifts, the pill fades out while a lens fades in above the content and grows
/// by [`lift_scale`](Self::lift_scale). The lens is real optics rather than a
/// tint: it [enlarges](Self::zoom) whatever is under its middle, swells it a
/// little more toward the center with a [bulge](Self::bulge), and bends it where
/// the glass curves away at the rim, whose depth is the lens's
/// [thickness](Self::thickness). A [rim](Self::rim) of light follows the edge.
///
/// The lens is drawn only while it is visible, so a resting bar pays nothing for
/// the backdrop pass.
///
/// Use [`indicator_styles::LIQUID_GLASS`](super::indicator_styles::LIQUID_GLASS)
/// for the default look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiquidGlass {
    rest_fill: Color,
    lift_scale: f32,
    zoom: f32,
    thickness: f32,
    bulge: f32,
    rim: f32,
}

/// A flat, translucent pill that stays flat.
///
/// It draws only the rest layer, so it costs nothing beyond the pill itself.
/// An optional [`lift_scale`](Self::lift_scale) makes it swell slightly while
/// the indicator travels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlatPill {
    fill: Color,
    lift_scale: f32,
}

/// The standard liquid glass style, as in the iOS 26 tab bar.
pub const LIQUID_GLASS: LiquidGlass = LiquidGlass {
    rest_fill: REST_FILL,
    lift_scale: 1.18,
    zoom: 1.25,
    thickness: 10.0,
    bulge: 0.15,
    rim: 0.8,
};

/// The standard flat pill: it never lifts.
pub const FLAT_PILL: FlatPill = FlatPill { fill: REST_FILL, lift_scale: 1.0 };

/// A [`FlatPill`] filled with `fill`.
pub const fn flat_pill(fill: Color) -> FlatPill {
    FlatPill { fill, lift_scale: 1.0 }
}

/// A pill: a rounded box of `size` filled with `fill`.
///
/// It is a decoration box and not a `Container` on purpose. The layer that
/// presents it moves its render node, and a container reports its hit area from
/// that node, so one that sits where it is shown and not where it was laid out
/// would be flagged as having drifted from its layout.
fn pill(size: ResolvedSize, fill: Color) -> AnyWidget {
    DecorationBox::new()
        .decoration(
            BoxDecoration::new()
                .background_color(fill)
                .border_radius(size.height * 0.5),
        )
        .box_child(SizedBox::new().width(size.width).height(size.height))
}

fn valid_scale(scale: f32) -> bool {
    scale.is_finite() && scale >= 1.0
}

/// How far into `0.0..=1.0` a lift is, ignoring the little a spring overshoots.
#[inline]
fn clamped(lift: f32) -> f32 {
    if lift.is_finite() { lift.clamp(0.0, 1.0) } else { 0.0 }
}

impl LiquidGlass {
    /// Sets the fill of the pill shown at rest.
    #[inline]
    pub const fn rest_fill(mut self, fill: Color) -> Self {
        self.rest_fill = fill;
        self
    }

    /// Sets how much larger than its slot the lens grows when fully lifted.
    /// `1.0` keeps it the size of the slot; values below `1.0` or that are not
    /// finite make the style invalid.
    #[inline]
    pub const fn lift_scale(mut self, scale: f32) -> Self {
        self.lift_scale = scale;
        self
    }

    /// Sets how much the lens enlarges what is under it, `1.0` being not at all.
    /// Values below `1.0`, above `8.0`, or that are not finite make the style
    /// invalid.
    #[inline]
    pub const fn zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom;
        self
    }

    /// Sets how deep the glass is at its rim, in logical pixels: how far in from
    /// the edge the content is bent. `0.0` leaves the rim flat.
    ///
    /// Keep it modest. Around 10 bends the content smoothly; much past that the
    /// bend samples beyond the lens and shows a ghost of what is outside it, and
    /// deeper still it folds the content over. Negative or non-finite values
    /// make the style invalid.
    #[inline]
    pub const fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness;
        self
    }

    /// Sets the extra swell at the center of the lens, from `0.0` for none to
    /// `1.0`, which doubles the enlargement there. Values outside that range or
    /// that are not finite make the style invalid.
    #[inline]
    pub const fn bulge(mut self, bulge: f32) -> Self {
        self.bulge = bulge;
        self
    }

    /// Sets the strength of the rim of light, from `0.0` for none to `1.0`.
    /// Values outside that range or that are not finite make the style invalid.
    #[inline]
    pub const fn rim(mut self, rim: f32) -> Self {
        self.rim = rim;
        self
    }

    fn lens(&self, size: ResolvedSize) -> AnyWidget {
        let radius = size.height * 0.5;
        // No shadow decoration: it would be painted before the optics, which
        // sample the backdrop of the same node, and turn the whole lens grey.
        MultiDecorationBox::new()
            // The shape comes first: the optics and the rim both adopt it.
            .decoration(BorderRadius::from(radius))
            .decoration(
                Magnification::new()
                    .style(
                        MagnificationStyle::UNIFORM
                            | MagnificationStyle::BULGE
                            | MagnificationStyle::REFRACTION,
                    )
                    .zoom(self.zoom)
                    .bulge(self.bulge)
                    .thickness(self.thickness),
            )
            .decoration(
                LightEdge::new()
                    .width(6.0)
                    .strength(self.rim)
                    .reflection_strength(self.rim * 0.5),
            )
            .box_child(SizedBox::new().width(size.width).height(size.height))
    }
}

impl IndicatorStyle for LiquidGlass {
    fn layer(&self, layer: IndicatorLayer, size: ResolvedSize) -> Option<AnyWidget> {
        match layer {
            IndicatorLayer::Rest => Some(pill(size, self.rest_fill)),
            IndicatorLayer::Lift => Some(self.lens(size)),
        }
    }

    #[inline]
    fn present(&self, layer: IndicatorLayer, lift: f32) -> LayerPresentation {
        let lift = clamped(lift);
        match layer {
            IndicatorLayer::Rest => LayerPresentation { scale: 1.0, opacity: 1.0 - lift },
            IndicatorLayer::Lift => LayerPresentation {
                scale: 1.0 + (self.lift_scale - 1.0) * lift,
                opacity: lift,
            },
        }
    }

    fn is_valid(&self) -> bool {
        valid_scale(self.lift_scale)
            && self.zoom.is_finite()
            && (1.0..=8.0).contains(&self.zoom)
            && self.thickness.is_finite()
            && self.thickness >= 0.0
            && (0.0..=1.0).contains(&self.bulge)
            && (0.0..=1.0).contains(&self.rim)
    }

    fn into_any(self) -> AnyIndicatorStyle {
        AnyIndicatorStyle::liquid_glass(self)
    }
}

impl FlatPill {
    /// Makes the pill swell to `scale` times its size while fully lifted.
    /// Values below `1.0` or that are not finite make the style invalid.
    #[inline]
    pub const fn lift_scale(mut self, scale: f32) -> Self {
        self.lift_scale = scale;
        self
    }
}

impl IndicatorStyle for FlatPill {
    fn layer(&self, layer: IndicatorLayer, size: ResolvedSize) -> Option<AnyWidget> {
        (layer == IndicatorLayer::Rest).then(|| pill(size, self.fill))
    }

    #[inline]
    fn present(&self, _layer: IndicatorLayer, lift: f32) -> LayerPresentation {
        LayerPresentation {
            scale: 1.0 + (self.lift_scale - 1.0) * clamped(lift),
            opacity: 1.0,
        }
    }

    fn is_valid(&self) -> bool {
        valid_scale(self.lift_scale)
    }

    fn into_any(self) -> AnyIndicatorStyle {
        AnyIndicatorStyle::flat_pill(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOT: ResolvedSize = ResolvedSize { width: 100.0, height: 56.0 };

    #[test]
    fn at_rest_only_the_pill_is_visible() {
        let rest = LIQUID_GLASS.present(IndicatorLayer::Rest, 0.0);
        let lens = LIQUID_GLASS.present(IndicatorLayer::Lift, 0.0);
        assert_eq!(rest, LayerPresentation::OPAQUE);
        assert_eq!(lens, LayerPresentation { scale: 1.0, opacity: 0.0 });
    }

    #[test]
    fn fully_lifted_only_the_lens_is_visible_and_it_is_larger() {
        let rest = LIQUID_GLASS.present(IndicatorLayer::Rest, 1.0);
        let lens = LIQUID_GLASS.present(IndicatorLayer::Lift, 1.0);
        assert_eq!(rest.opacity, 0.0);
        assert_eq!(lens, LayerPresentation { scale: 1.18, opacity: 1.0 });
    }

    #[test]
    fn the_layers_cross_fade_without_a_gap_in_between() {
        for step in 0..=20 {
            let lift = step as f32 / 20.0;
            let rest = LIQUID_GLASS.present(IndicatorLayer::Rest, lift).opacity;
            let lens = LIQUID_GLASS.present(IndicatorLayer::Lift, lift).opacity;
            assert!((rest + lens - 1.0).abs() < 1.0e-6, "lift {lift}: {rest} + {lens}");
        }
    }

    #[test]
    fn a_spring_overshoot_never_leaves_the_visible_range() {
        for lift in [-0.2, 1.15, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for layer in [IndicatorLayer::Rest, IndicatorLayer::Lift] {
                let shown = LIQUID_GLASS.present(layer, lift);
                assert!((0.0..=1.0).contains(&shown.opacity), "{layer:?} at {lift}: {shown:?}");
                assert!(shown.scale.is_finite() && shown.scale >= 1.0, "{layer:?} at {lift}: {shown:?}");
            }
        }
    }

    #[test]
    fn both_layers_exist_for_liquid_glass_and_only_the_rest_for_a_flat_pill() {
        assert!(LIQUID_GLASS.layer(IndicatorLayer::Rest, SLOT).is_some());
        assert!(LIQUID_GLASS.layer(IndicatorLayer::Lift, SLOT).is_some());
        assert!(FLAT_PILL.layer(IndicatorLayer::Rest, SLOT).is_some());
        assert!(FLAT_PILL.layer(IndicatorLayer::Lift, SLOT).is_none());
    }

    #[test]
    fn a_flat_pill_stays_opaque_and_swells_only_when_asked() {
        assert_eq!(FLAT_PILL.present(IndicatorLayer::Rest, 1.0), LayerPresentation::OPAQUE);
        let swelling = FLAT_PILL.lift_scale(1.1);
        assert_eq!(swelling.present(IndicatorLayer::Rest, 1.0), LayerPresentation { scale: 1.1, opacity: 1.0 });
    }

    #[test]
    fn an_unusable_lift_scale_makes_the_style_invalid_and_draws_nothing() {
        for scale in [0.5, f32::NAN, f32::INFINITY] {
            let style = LIQUID_GLASS.lift_scale(scale);
            assert!(!style.is_valid());
            assert!(style.into_any().layer(IndicatorLayer::Rest, SLOT).is_none());
        }
        assert!(!FLAT_PILL.lift_scale(0.9).is_valid());
    }

    #[test]
    fn the_lens_optics_are_validated() {
        assert!(LIQUID_GLASS.is_valid());
        assert!(LIQUID_GLASS.zoom(1.0).thickness(0.0).bulge(0.0).rim(0.0).is_valid());
        assert!(LIQUID_GLASS.zoom(8.0).bulge(1.0).rim(1.0).is_valid());
        for broken in [
            LIQUID_GLASS.zoom(0.9),
            LIQUID_GLASS.zoom(8.5),
            LIQUID_GLASS.zoom(f32::NAN),
            LIQUID_GLASS.thickness(-1.0),
            LIQUID_GLASS.thickness(f32::INFINITY),
            LIQUID_GLASS.bulge(1.5),
            LIQUID_GLASS.bulge(f32::NAN),
            LIQUID_GLASS.rim(-0.1),
            LIQUID_GLASS.rim(f32::NAN),
        ] {
            assert!(!broken.is_valid(), "{broken:?}");
            assert!(broken.into_any().layer(IndicatorLayer::Lift, SLOT).is_none(), "{broken:?}");
        }
    }

    #[test]
    fn the_default_lens_stays_inside_the_range_that_does_not_ghost() {
        // Past roughly this much depth the bend samples beyond the lens and shows
        // a ghost of the content outside it.
        assert!(LIQUID_GLASS.thickness <= 10.0);
    }

    #[test]
    fn standard_styles_are_held_inline_and_compare_by_value() {
        assert!(LIQUID_GLASS.into_any().is_inline());
        assert!(FLAT_PILL.into_any().is_inline());
        assert_eq!(LIQUID_GLASS.into_any(), LIQUID_GLASS.into_any());
        assert_ne!(LIQUID_GLASS.into_any(), FLAT_PILL.into_any());
        assert_ne!(LIQUID_GLASS.into_any(), LIQUID_GLASS.lift_scale(1.3).into_any());
        assert_ne!(LIQUID_GLASS.into_any(), LIQUID_GLASS.zoom(2.0).into_any());
    }

    struct Custom;

    impl IndicatorStyle for Custom {
        fn layer(&self, _layer: IndicatorLayer, _size: ResolvedSize) -> Option<AnyWidget> {
            None
        }

        fn present(&self, _layer: IndicatorLayer, _lift: f32) -> LayerPresentation {
            LayerPresentation::HIDDEN
        }
    }

    #[test]
    fn a_custom_style_is_boxed_and_compared_by_identity() {
        let custom = Custom.into_any();
        assert!(!custom.is_inline());
        assert_eq!(custom, custom.clone());
        assert_ne!(custom, Custom.into_any());
        assert_eq!(custom.present(IndicatorLayer::Rest, 0.5), LayerPresentation::HIDDEN);
    }
}
