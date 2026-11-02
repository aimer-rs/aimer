//! Platform-neutral slider and range-slider controls.

#![deny(missing_docs)]

use std::rc::Rc;

mod error;
mod range_slider;
mod semantics;
mod slider;
mod spec;
mod value;
mod visuals;
mod widgets;

pub use self::error::{RangeError, RangeField};
pub use self::range_slider::RangeSlider;
pub use self::semantics::{RangeRole, RangeSemantics, SemanticRangeValue};
pub use self::slider::Slider;
pub use self::spec::RangeSpec;
pub use self::value::RangeValue;
pub use self::visuals::{SliderThumb, SliderTrail};
pub use self::widgets::{RangeSliderState, SliderState};

/// A callback that receives a proposed single-slider value.
pub type RangeChangeCallback<T = f64> = Rc<dyn Fn(T)>;

/// A callback that receives a proposed lower and upper range-slider value.
pub type RangePairChangeCallback<T = f64> = Rc<dyn Fn((T, T))>;

/// Defines how a range constructor handles `min > max`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReversedBoundsPolicy {
    /// Reject reversed bounds with [`RangeError::ReversedBounds`].
    Reject,
    /// Swap reversed bounds so the effective range is increasing.
    Normalize,
}

/// Keyboard actions understood by [`Slider`] and [`RangeSlider`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SliderKey {
    /// Decrease by one step.
    ArrowLeft,
    /// Increase by one step.
    ArrowRight,
    /// Increase by one step.
    ArrowUp,
    /// Decrease by one step.
    ArrowDown,
    /// Move to the minimum.
    Home,
    /// Move to the maximum.
    End,
    /// Decrease by ten steps.
    PageDown,
    /// Increase by ten steps.
    PageUp,
}

/// Identifies which thumb receives a range-slider interaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeThumb {
    /// The lower-valued thumb.
    Lower,
    /// The upper-valued thumb.
    Upper,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_snaps_to_the_nearest_step_and_clamps_to_its_bounds() {
        let mut slider = Slider::new().range(0.0..10.0).step(3.0).value(5.0);

        assert_eq!(slider.current_value(), 6.0);

        slider.set_value(-4.0).unwrap();
        assert_eq!(slider.current_value(), 0.0);

        slider.set_value(40.0).unwrap();
        assert_eq!(slider.current_value(), 10.0);
    }

    #[test]
    fn slider_validates_finite_configuration_and_has_explicit_reversed_policy() {
        assert!(matches!(
            Slider::new()
                .range(10.0..0.0)
                .step(1.0)
                .value(5.0)
                .validate(),
            Err(RangeError::ReversedBounds {
                min: 10.0,
                max: 0.0
            })
        ));
        assert!(matches!(
            Slider::new()
                .range(f64::NAN..10.0)
                .step(1.0)
                .value(5.0)
                .validate(),
            Err(RangeError::NonFinite {
                field: RangeField::Minimum,
                ..
            })
        ));
        assert!(matches!(
            Slider::new()
                .range(0.0..f64::INFINITY)
                .step(1.0)
                .value(5.0)
                .validate(),
            Err(RangeError::NonFinite {
                field: RangeField::Maximum,
                ..
            })
        ));
        assert!(matches!(
            Slider::new()
                .range(0.0..10.0)
                .step(0.0)
                .value(5.0)
                .validate(),
            Err(RangeError::NonPositiveStep { step: 0.0 })
        ));

        let slider = Slider::new()
            .range(10.0..0.0)
            .step(2.0)
            .value(8.0)
            .reversed_bounds_policy(ReversedBoundsPolicy::Normalize);
        assert_eq!(slider.min(), 0.0);
        assert_eq!(slider.max(), 10.0);
        assert_eq!(slider.current_value(), 8.0);
        assert_eq!(
            slider.reversed_bounds_policy_value(),
            ReversedBoundsPolicy::Normalize
        );
    }

    #[test]
    fn equal_bounds_are_a_valid_zero_range_control() {
        let mut slider = Slider::new().range(4.0..4.0).step(1.0).value(99.0);

        assert_eq!(slider.current_value(), 4.0);
        assert_eq!(slider.value_at_position(20.0, 0.0).unwrap(), 4.0);
        assert_eq!(slider.position_for_value(-20.0, 0.0).unwrap(), 0.0);
        assert!(!slider.handle_key(SliderKey::ArrowRight).unwrap());
        assert_eq!(slider.current_value(), 4.0);
    }

    #[test]
    fn enabled_keyboard_input_moves_by_steps_and_reaches_endpoints() {
        let mut slider = Slider::new().range(0.0..10.0).step(2.0).value(4.0);

        assert!(slider.handle_key(SliderKey::ArrowRight).unwrap());
        assert_eq!(slider.current_value(), 6.0);
        assert!(slider.handle_key(SliderKey::ArrowDown).unwrap());
        assert_eq!(slider.current_value(), 4.0);
        assert!(slider.handle_key(SliderKey::PageUp).unwrap());
        assert_eq!(slider.current_value(), 10.0);
        assert!(!slider.handle_key(SliderKey::End).unwrap());
        assert!(slider.handle_key(SliderKey::Home).unwrap());
        assert_eq!(slider.current_value(), 0.0);
    }

    #[test]
    fn disabled_keyboard_and_pointer_input_do_not_change_value() {
        let mut slider = Slider::new().range(0.0..100.0).step(10.0).value(40.0);
        slider.set_disabled(true);

        assert!(!slider.handle_key(SliderKey::ArrowRight).unwrap());
        assert!(!slider.set_from_position(100.0, 100.0).unwrap());
        assert_eq!(slider.current_value(), 40.0);
        assert!(!slider.semantics().is_enabled());

        // Controlled parent updates remain possible while interaction is off.
        assert!(slider.set_value(70.0).unwrap());
        assert_eq!(slider.current_value(), 70.0);
    }

    #[test]
    fn pointer_conversion_clamps_and_rounds_in_both_directions() {
        let slider = Slider::new().range(0.0..10.0).step(2.0).value(0.0);

        assert_eq!(slider.value_at_position(-10.0, 100.0).unwrap(), 0.0);
        assert_eq!(slider.value_at_position(25.0, 100.0).unwrap(), 2.0);
        assert_eq!(slider.value_at_position(55.0, 100.0).unwrap(), 6.0);
        assert_eq!(slider.value_at_position(110.0, 100.0).unwrap(), 10.0);
        assert_eq!(slider.position_for_value(5.0, 100.0).unwrap(), 60.0);
        assert_eq!(slider.position_for_value(-5.0, 100.0).unwrap(), 0.0);
        assert_eq!(slider.position_for_value(20.0, 100.0).unwrap(), 100.0);
    }

    #[test]
    fn pointer_conversion_rejects_non_finite_coordinates_and_negative_tracks() {
        let slider = Slider::new().range(0.0..10.0).step(1.0).value(5.0);

        assert!(matches!(
            slider.value_at_position(f64::NAN, 100.0),
            Err(RangeError::NonFinite {
                field: RangeField::Position,
                ..
            })
        ));
        assert!(matches!(
            slider.value_at_position(10.0, f64::INFINITY),
            Err(RangeError::NonFinite {
                field: RangeField::TrackLength,
                ..
            })
        ));
        assert!(matches!(
            slider.position_for_value(5.0, -1.0),
            Err(RangeError::NegativeTrackLength { length: -1.0 })
        ));
    }

    #[test]
    fn slider_semantics_publish_range_state_and_invalid_raw_metadata() {
        let slider = Slider::new().range(0.0..10.0).step(2.0).value(5.0);
        let semantics = slider.semantics();

        assert_eq!(semantics.role(), RangeRole::Slider);
        assert_eq!(semantics.min(), 0.0);
        assert_eq!(semantics.max(), 10.0);
        assert_eq!(semantics.step(), 2.0);
        assert_eq!(semantics.value(), SemanticRangeValue::Single(6.0));
        assert!(semantics.is_enabled());
        assert!(!semantics.invalid_range());

        let invalid = RangeSemantics::from_slider(10.0, 0.0, 1.0, 5.0, true);
        assert!(invalid.invalid_range());
        let invalid_value = RangeSemantics::from_slider(0.0, 10.0, 1.0, 11.0, true);
        assert!(invalid_value.invalid_range());
    }

    #[test]
    fn range_slider_keeps_two_ordered_values_and_clamps_crossing_thumbs() {
        let mut slider = RangeSlider::new()
            .range(0.0..100.0)
            .step(10.0)
            .values(20.0..70.0);

        assert!(slider.set_lower(90.0).unwrap());
        assert_eq!(slider.lower(), 70.0);
        assert!(!slider.set_upper(10.0).unwrap());
        assert_eq!(slider.upper(), 70.0);

        slider.set_values(20.0, 70.0).unwrap();
        assert!(slider.set_upper(10.0).unwrap());
        assert_eq!(slider.upper(), 20.0);

        assert!(matches!(
            slider.set_values(80.0, 30.0),
            Err(RangeError::ReversedValues {
                lower: 80.0,
                upper: 30.0
            })
        ));
        assert_eq!((slider.lower(), slider.upper()), (20.0, 20.0));
    }

    #[test]
    fn range_slider_has_distinct_thumb_keyboard_pointer_and_semantic_seams() {
        let mut slider = RangeSlider::new()
            .range(0.0..100.0)
            .step(10.0)
            .values(20.0..70.0);

        assert!(
            slider
                .handle_key(RangeThumb::Lower, SliderKey::ArrowRight)
                .unwrap()
        );
        assert_eq!(slider.lower(), 30.0);
        assert!(
            slider
                .set_thumb_from_position(RangeThumb::Upper, 95.0, 100.0)
                .unwrap()
        );
        assert_eq!(slider.upper(), 100.0);
        assert!(
            slider
                .set_thumb_from_position(RangeThumb::Lower, 110.0, 100.0)
                .unwrap()
        );
        assert_eq!(slider.lower(), 100.0);

        let semantics = slider.semantics();
        assert_eq!(semantics.role(), RangeRole::RangeSlider);
        assert_eq!(
            semantics.value(),
            SemanticRangeValue::Pair {
                lower: 100.0,
                upper: 100.0
            }
        );
        assert!(!semantics.invalid_range());
    }

    #[test]
    fn range_slider_rejects_reversed_initial_values_and_marks_invalid_metadata() {
        assert!(matches!(
            RangeSlider::new()
                .range(0.0..100.0)
                .step(1.0)
                .values(80.0..20.0)
                .validate(),
            Err(RangeError::ReversedValues {
                lower: 80.0,
                upper: 20.0
            })
        ));

        let invalid = RangeSemantics::from_range_slider(0.0, 100.0, 1.0, 80.0, 20.0, true);
        assert!(invalid.invalid_range());
    }

    #[test]
    fn controlled_non_finite_updates_fail_without_losing_the_previous_value() {
        let mut slider = Slider::new().range(0.0..10.0).step(1.0).value(5.0);
        assert!(matches!(
            slider.set_value(f64::NAN),
            Err(RangeError::NonFinite {
                field: RangeField::Value,
                ..
            })
        ));
        assert_eq!(slider.current_value(), 5.0);

        let mut range_slider = RangeSlider::new()
            .range(0.0..10.0)
            .step(1.0)
            .values(2.0..8.0);
        assert!(matches!(
            range_slider.set_values(f64::INFINITY, 8.0),
            Err(RangeError::NonFinite {
                field: RangeField::LowerValue,
                ..
            })
        ));
        assert_eq!((range_slider.lower(), range_slider.upper()), (2.0, 8.0));
    }

    #[test]
    fn range_slider_equal_bounds_and_zero_length_tracks_are_stable() {
        let mut slider = RangeSlider::new()
            .range(6.0..6.0)
            .step(0.5)
            .values(-20.0..20.0);

        assert_eq!((slider.lower(), slider.upper()), (6.0, 6.0));
        assert_eq!(slider.value_at_position(200.0, 0.0).unwrap(), 6.0);
        assert_eq!(slider.position_for_value(100.0, 0.0).unwrap(), 0.0);
        assert!(
            !slider
                .set_thumb_from_position(RangeThumb::Lower, 0.0, 0.0)
                .unwrap()
        );
        assert_eq!((slider.lower(), slider.upper()), (6.0, 6.0));
    }

    #[test]
    fn range_spec_handles_wide_finite_endpoints_without_non_finite_output() {
        let spec = RangeSpec::new(-f64::MAX, f64::MAX, f64::MAX).unwrap();

        assert_eq!(spec.value_at_position(0.0, 1.0).unwrap(), -f64::MAX);
        assert_eq!(spec.value_at_position(0.5, 1.0).unwrap(), 0.0);
        assert_eq!(spec.value_at_position(1.0, 1.0).unwrap(), f64::MAX);
        assert_eq!(spec.position_for_value(0.0, 1.0).unwrap(), 0.5);
    }
}
