//! Easing curves.
//!
//! A [`Curve`] maps linear progress `t` in `0.0..=1.0` to eased progress. The
//! standard curves live in [`curves`] as constants (`curves::EASE_IN_OUT`) and
//! any type that implements the trait can be used wherever the framework takes
//! one:
//!
//! ```
//! use aimer_animation::{Curve, curves};
//!
//! /// Eases in steps of a quarter.
//! struct Stepped;
//!
//! impl Curve for Stepped {
//!     fn transform(&self, t: f32) -> f32 {
//!         (t * 4.0).floor() / 4.0
//!     }
//! }
//!
//! assert_eq!(Stepped.transform(0.6), 0.5);
//! assert_eq!(curves::LINEAR.transform(0.6), 0.6);
//! ```
//!
//! Holders store curves as [`AnyCurve`], a cheap handle that is itself a
//! [`Curve`]: the standard curves are kept inline without allocating, and only a
//! custom curve is boxed.

mod any;
mod builtin;
#[cfg(test)]
mod golden;

pub use any::AnyCurve;

/// The standard curves.
pub mod curves {
    pub use super::builtin::*;
}

/// Maps linear progress to eased progress.
///
/// Implement [`transform`](Self::transform) for a custom curve. The framework
/// only ever hands it a `t` inside `0.0..=1.0`, and it should return `0.0` at
/// `0.0` and `1.0` at `1.0` for an animation to start and end where it is meant
/// to; values in between may overshoot, as a bounce or an elastic curve does.
///
/// A curve must be `Send + Sync + 'static` so that holders can share it freely.
pub trait Curve: Send + Sync + 'static {
    /// Returns the eased value for progress `t`.
    fn transform(&self, t: f32) -> f32;

    /// Describes this curve for a portable guest, or `None` if it cannot be
    /// described.
    ///
    /// Only the standard curves and cubic Béziers can be serialized into a
    /// portable widget document. A custom curve keeps the default `None`: it
    /// works natively, and lowering it to a guest reports an error.
    fn descriptor(&self) -> Option<CurveDescriptor> {
        None
    }

    /// Whether the curve's parameters are usable, such as the finite control
    /// points of a Bézier. Builders that cannot accept a broken curve check
    /// this and fall back or report an error.
    fn is_valid(&self) -> bool {
        true
    }

    /// Converts this curve into the handle holders store.
    ///
    /// The default boxes the curve. The standard curves override it to avoid
    /// the allocation.
    fn into_any(self) -> AnyCurve
    where
        Self: Sized,
    {
        AnyCurve::custom(self)
    }
}

/// A serializable description of a standard curve: a tag plus, for a cubic
/// Bézier, its control points `(x1, y1, x2, y2)`.
///
/// The tags are part of the portable widget format and never change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveDescriptor {
    /// Identifies the curve: `0` linear, `1` ease-in, `2` ease-out, `3`
    /// ease-in-out, `4` cubic Bézier, `5` decelerate, `6`-`8` bounce out/in/in-out,
    /// `9`-`11` elastic in/out/in-out, `12` fast-out-slow-in, `13`
    /// linear-out-slow-in, `14` fast-out-linear-in.
    pub tag: i64,
    /// The control points of a cubic Bézier (tag `4`); `None` for every other curve.
    pub control_points: Option<[f64; 4]>,
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::Instant;

    use super::golden::{GOLDEN, INPUTS};
    use super::*;

    /// Every standard curve, by golden name, as the handle holders store.
    fn standard() -> [(&'static str, AnyCurve); 15] {
        [
            ("LINEAR", curves::LINEAR.into_any()),
            ("EASE_IN", curves::EASE_IN.into_any()),
            ("EASE_OUT", curves::EASE_OUT.into_any()),
            ("EASE_IN_OUT", curves::EASE_IN_OUT.into_any()),
            ("CUBIC_BEZIER", curves::cubic_bezier(0.17, 0.74, 0.30, 1.0).into_any()),
            ("DECELERATE", curves::DECELERATE.into_any()),
            ("BOUNCE_OUT", curves::BOUNCE_OUT.into_any()),
            ("BOUNCE_IN", curves::BOUNCE_IN.into_any()),
            ("BOUNCE_IN_OUT", curves::BOUNCE_IN_OUT.into_any()),
            ("ELASTIC_IN", curves::ELASTIC_IN.into_any()),
            ("ELASTIC_OUT", curves::ELASTIC_OUT.into_any()),
            ("ELASTIC_IN_OUT", curves::ELASTIC_IN_OUT.into_any()),
            ("FAST_OUT_SLOW_IN", curves::FAST_OUT_SLOW_IN.into_any()),
            ("LINEAR_OUT_SLOW_IN", curves::LINEAR_OUT_SLOW_IN.into_any()),
            ("FAST_OUT_LINEAR_IN", curves::FAST_OUT_LINEAR_IN.into_any()),
        ]
    }

    fn assert_matches_golden(name: &str, expected: &[f32; 25], curve: &impl Curve) {
        for (input, expected) in INPUTS.iter().zip(expected) {
            let got = curve.transform(*input);
            assert_eq!(got.to_bits(), expected.to_bits(), "{name} at t = {input}: got {got}, was {expected}");
        }
    }

    #[test]
    fn every_standard_curve_matches_what_the_enum_produced() {
        for ((name, expected), (standard_name, curve)) in GOLDEN.iter().zip(standard()) {
            assert_eq!(*name, standard_name);
            assert_matches_golden(name, expected, &curve);
        }
    }

    #[test]
    fn the_curve_types_themselves_match_what_the_enum_produced() {
        // The constants are the types; check them directly, not through the handle.
        let golden = |name: &str| GOLDEN.iter().find(|(n, _)| *n == name).unwrap().1;
        assert_matches_golden("LINEAR", &golden("LINEAR"), &curves::LINEAR);
        assert_matches_golden("EASE_IN", &golden("EASE_IN"), &curves::EASE_IN);
        assert_matches_golden("EASE_OUT", &golden("EASE_OUT"), &curves::EASE_OUT);
        assert_matches_golden("EASE_IN_OUT", &golden("EASE_IN_OUT"), &curves::EASE_IN_OUT);
        assert_matches_golden("CUBIC_BEZIER", &golden("CUBIC_BEZIER"), &curves::cubic_bezier(0.17, 0.74, 0.30, 1.0));
        assert_matches_golden("DECELERATE", &golden("DECELERATE"), &curves::DECELERATE);
        assert_matches_golden("BOUNCE_OUT", &golden("BOUNCE_OUT"), &curves::BOUNCE_OUT);
        assert_matches_golden("BOUNCE_IN", &golden("BOUNCE_IN"), &curves::BOUNCE_IN);
        assert_matches_golden("BOUNCE_IN_OUT", &golden("BOUNCE_IN_OUT"), &curves::BOUNCE_IN_OUT);
        assert_matches_golden("ELASTIC_IN", &golden("ELASTIC_IN"), &curves::ELASTIC_IN);
        assert_matches_golden("ELASTIC_OUT", &golden("ELASTIC_OUT"), &curves::ELASTIC_OUT);
        assert_matches_golden("ELASTIC_IN_OUT", &golden("ELASTIC_IN_OUT"), &curves::ELASTIC_IN_OUT);
        assert_matches_golden("FAST_OUT_SLOW_IN", &golden("FAST_OUT_SLOW_IN"), &curves::FAST_OUT_SLOW_IN);
        assert_matches_golden("LINEAR_OUT_SLOW_IN", &golden("LINEAR_OUT_SLOW_IN"), &curves::LINEAR_OUT_SLOW_IN);
        assert_matches_golden("FAST_OUT_LINEAR_IN", &golden("FAST_OUT_LINEAR_IN"), &curves::FAST_OUT_LINEAR_IN);
    }

    #[test]
    fn standard_curves_keep_the_enums_boundary_behaviour() {
        for (name, curve) in standard() {
            assert!(curve.transform(0.0).abs() < 1e-6, "{name} starts at 0");
            assert!((curve.transform(1.0) - 1.0).abs() < 1e-6, "{name} ends at 1");
        }
        assert!(curves::EASE_IN.transform(0.5) < 0.5);
        assert!(curves::EASE_OUT.transform(0.5) > 0.5);
        assert!(curves::DECELERATE.transform(0.5) > 0.5);
        assert!((curves::EASE_IN_OUT.transform(0.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn out_of_range_progress_is_clamped_by_every_curve() {
        for (name, curve) in standard() {
            assert_eq!(curve.transform(-3.0), curve.transform(0.0), "{name}");
            assert_eq!(curve.transform(3.0), curve.transform(1.0), "{name}");
        }
    }

    #[test]
    fn every_standard_curve_round_trips_through_its_descriptor() {
        let mut tags = Vec::new();
        for (name, curve) in standard() {
            let descriptor = curve.descriptor().unwrap_or_else(|| panic!("{name} has a descriptor"));
            assert_eq!(descriptor.control_points.is_some(), descriptor.tag == 4, "{name}");

            let decoded = AnyCurve::from_descriptor(descriptor.tag, descriptor.control_points)
                .unwrap_or_else(|| panic!("{name} decodes"));

            assert_eq!(decoded, curve, "{name}");
            tags.push(descriptor.tag);
        }
        tags.sort_unstable();
        assert_eq!(tags, (0..=14).collect::<Vec<_>>(), "the tag table is unchanged");
    }

    #[test]
    fn the_portable_tag_table_is_stable() {
        let tag = |curve: AnyCurve| curve.descriptor().unwrap().tag;
        assert_eq!(tag(curves::LINEAR.into_any()), 0);
        assert_eq!(tag(curves::EASE_IN.into_any()), 1);
        assert_eq!(tag(curves::EASE_OUT.into_any()), 2);
        assert_eq!(tag(curves::EASE_IN_OUT.into_any()), 3);
        assert_eq!(tag(curves::cubic_bezier(0.0, 0.0, 1.0, 1.0).into_any()), 4);
        assert_eq!(tag(curves::DECELERATE.into_any()), 5);
        assert_eq!(tag(curves::BOUNCE_OUT.into_any()), 6);
        assert_eq!(tag(curves::BOUNCE_IN.into_any()), 7);
        assert_eq!(tag(curves::BOUNCE_IN_OUT.into_any()), 8);
        assert_eq!(tag(curves::ELASTIC_IN.into_any()), 9);
        assert_eq!(tag(curves::ELASTIC_OUT.into_any()), 10);
        assert_eq!(tag(curves::ELASTIC_IN_OUT.into_any()), 11);
        assert_eq!(tag(curves::FAST_OUT_SLOW_IN.into_any()), 12);
        assert_eq!(tag(curves::LINEAR_OUT_SLOW_IN.into_any()), 13);
        assert_eq!(tag(curves::FAST_OUT_LINEAR_IN.into_any()), 14);
    }

    #[test]
    fn a_descriptor_that_does_not_describe_a_curve_is_rejected() {
        let points = Some([0.1, 0.2, 0.3, 0.4]);

        assert!(AnyCurve::from_descriptor(15, None).is_none(), "unknown tag");
        assert!(AnyCurve::from_descriptor(-1, None).is_none(), "negative tag");
        assert!(AnyCurve::from_descriptor(1, points).is_none(), "points on a curve that has none");
        assert!(AnyCurve::from_descriptor(4, None).is_none(), "a Bézier without points");
        assert!(AnyCurve::from_descriptor(4, Some([f64::NAN, 0.0, 1.0, 1.0])).is_none(), "non-finite points");
        assert!(AnyCurve::from_descriptor(4, points).is_some());
    }

    #[test]
    fn a_bezier_with_a_non_finite_control_point_is_invalid() {
        assert!(curves::cubic_bezier(0.1, 0.2, 0.3, 0.4).is_valid());
        for broken in [
            curves::cubic_bezier(f32::NAN, 0.0, 1.0, 1.0),
            curves::cubic_bezier(0.0, f32::INFINITY, 1.0, 1.0),
            curves::cubic_bezier(0.0, 0.0, f32::NEG_INFINITY, 1.0),
            curves::cubic_bezier(0.0, 0.0, 1.0, f32::NAN),
        ] {
            assert!(!broken.is_valid());
            assert!(!broken.into_any().is_valid(), "the handle reports it too");
        }
        assert!(curves::EASE_IN.is_valid() && curves::EASE_IN.into_any().is_valid());
    }

    /// A user's curve: quarter steps.
    struct Stepped;

    impl Curve for Stepped {
        fn transform(&self, t: f32) -> f32 {
            // Would misbehave on an unclamped input, which the framework never passes.
            assert!((0.0..=1.0).contains(&t), "custom curves only see 0..=1, got {t}");
            (t * 4.0).floor() / 4.0
        }
    }

    #[test]
    fn a_custom_curve_works_through_the_handle_and_only_sees_clamped_progress() {
        let curve = Stepped.into_any();

        assert_eq!(curve.transform(0.6), 0.5);
        assert_eq!(curve.transform(-2.0), 0.0);
        assert_eq!(curve.transform(2.0), 1.0);
    }

    #[test]
    fn a_custom_curve_has_no_descriptor_and_is_valid() {
        let curve = Stepped.into_any();

        assert_eq!(curve.descriptor(), None);
        assert!(curve.is_valid());
    }

    #[test]
    fn handles_compare_by_curve_and_custom_curves_by_identity() {
        assert_eq!(curves::EASE_IN.into_any(), curves::EASE_IN.into_any());
        assert_ne!(curves::EASE_IN.into_any(), curves::EASE_OUT.into_any());
        assert_eq!(curves::cubic_bezier(0.1, 0.2, 0.3, 0.4).into_any(), curves::cubic_bezier(0.1, 0.2, 0.3, 0.4).into_any());
        assert_ne!(curves::cubic_bezier(0.1, 0.2, 0.3, 0.4).into_any(), curves::cubic_bezier(0.1, 0.2, 0.3, 0.5).into_any());

        let custom = Stepped.into_any();
        assert_eq!(custom, custom.clone(), "a clone is the same curve");
        assert_ne!(custom, Stepped.into_any(), "two instances are different curves");
        assert_ne!(custom, curves::LINEAR.into_any());
    }

    #[test]
    fn the_default_handle_is_linear() {
        assert_eq!(AnyCurve::default(), curves::LINEAR.into_any());
        assert_eq!(AnyCurve::default().transform(0.3), 0.3);
    }

    #[test]
    fn standard_curves_are_stored_without_allocating_and_custom_ones_are_not() {
        for (name, curve) in standard() {
            assert!(curve.is_inline(), "{name} should not allocate");
        }
        assert!(!Stepped.into_any().is_inline());
    }

    #[test]
    fn converting_a_handle_into_a_handle_changes_nothing() {
        let curve = curves::EASE_OUT.into_any();

        assert_eq!(curve.clone().into_any(), curve);
    }

    #[test]
    fn handles_describe_themselves() {
        assert_eq!(format!("{:?}", curves::EASE_IN_OUT.into_any()), "EaseInOut");
        assert_eq!(format!("{:?}", curves::cubic_bezier(0.1, 0.2, 0.3, 0.4).into_any()), "CubicBezier(0.1, 0.2, 0.3, 0.4)");
        assert_eq!(format!("{:?}", Stepped.into_any()), "Custom");
    }

    #[test]
    fn curves_can_be_shared_across_threads() {
        fn shareable<T: Send + Sync + 'static>() {}
        shareable::<AnyCurve>();
        shareable::<curves::EaseIn>();
        shareable::<curves::CubicBezier>();
    }

    #[test]
    #[ignore = "manual numeric-kernel profile"]
    fn profile_curve_transform() {
        const MEASURED: usize = 1_024;
        const WARMUP: usize = 128;
        const ROUNDS: usize = 7;

        let cases: [(&str, AnyCurve); 7] = [
            ("linear", curves::LINEAR.into_any()),
            ("ease-in-out", curves::EASE_IN_OUT.into_any()),
            ("bounce-out", curves::BOUNCE_OUT.into_any()),
            ("elastic-out", curves::ELASTIC_OUT.into_any()),
            ("cubic-bezier", curves::cubic_bezier(0.17, 0.74, 0.30, 1.0).into_any()),
            ("fast-out-slow-in", curves::FAST_OUT_SLOW_IN.into_any()),
            ("custom", Stepped.into_any()),
        ];

        for (name, curve) in cases {
            let mut samples = Vec::with_capacity(ROUNDS);
            let mut checksum = 0.0;
            for _ in 0..ROUNDS {
                for index in 0..WARMUP {
                    let t = index as f32 / WARMUP as f32;
                    checksum = black_box(checksum + curve.transform(t));
                }

                let start = Instant::now();
                for index in 0..MEASURED {
                    let t = index as f32 / MEASURED as f32;
                    checksum = black_box(checksum + curve.transform(t));
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED as f64);
            }

            samples.sort_by(f64::total_cmp);
            let p50 = samples[ROUNDS / 2];
            let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
            println!("{name}: p50 {p50:.3} us, p95 {p95:.3} us");
            assert!(checksum.is_finite());
        }
    }
}
