//! How far open a hinge is.
//!
//! A [`DisplayFeature`](crate::DisplayFeature) says *where* a device bends; this
//! says *how far*. They are separate because they change at different rates and
//! for different readers: the geometry changes when a device is opened or
//! closed, the angle changes continuously while someone moves the hinge. A
//! widget that only lays out around the fold must not be rebuilt for every
//! degree.

/// How open a hinge is, as the platform classifies it.
///
/// Prefer this over the [angle](HingeState::angle) when a coarse answer is
/// enough: the platform does not promise any particular precision for the angle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HingeStatus {
    /// The platform cannot say.
    Unknown,
    /// Shut.
    Closed,
    /// Open part of the way.
    PartiallyOpen,
    /// Open as far as the device allows.
    FullyOpen,
}

impl HingeStatus {
    /// Translates UIKit's `UIHingeStatus`.
    ///
    /// A value this does not know (a status added by a later OS) is
    /// [`Unknown`](HingeStatus::Unknown) rather than a guess.
    #[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
    pub(crate) fn from_ui_hinge_status(raw: isize) -> Self {
        match raw {
            1 => Self::Closed,
            2 => Self::PartiallyOpen,
            3 => Self::FullyOpen,
            _ => Self::Unknown,
        }
    }
}

/// The state of a foldable device's hinge.
///
/// # Examples
///
/// ```
/// use aimer_screen::{HingeState, HingeStatus};
///
/// let hinge = HingeState::new(std::f32::consts::FRAC_PI_2, HingeStatus::PartiallyOpen);
///
/// assert_eq!(hinge.status, HingeStatus::PartiallyOpen);
/// assert!((hinge.degrees() - 90.0).abs() < 1e-4);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HingeState {
    /// The angle of the hinge in radians.
    ///
    /// How often this updates and how finely is platform policy and can change
    /// with system state, so do not depend on a particular rate or precision.
    /// Always finite.
    pub angle: f32,
    /// How open the hinge is.
    pub status: HingeStatus,
}

impl HingeState {
    /// Creates a hinge state from an `angle` in radians.
    ///
    /// A non-finite angle is not a measurement: it becomes `0.0` with the status
    /// [`Unknown`](HingeStatus::Unknown), so a reading can always be compared
    /// and no widget is rebuilt over a `NaN` that never equals itself.
    #[inline]
    pub fn new(angle: f32, status: HingeStatus) -> Self {
        if angle.is_finite() {
            Self { angle, status }
        } else {
            Self {
                angle: 0.0,
                status: HingeStatus::Unknown,
            }
        }
    }

    /// The angle of the hinge in degrees.
    #[inline]
    pub fn degrees(&self) -> f32 {
        self.angle.to_degrees()
    }

    /// Translates UIKit's `UIHinge`, whose angle is a `CGFloat` in radians and
    /// whose status is a `UIHingeStatus`.
    #[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
    pub(crate) fn from_ui_hinge(angle: f64, status: isize) -> Self {
        Self::new(angle as f32, HingeStatus::from_ui_hinge_status(status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uikit_statuses_keep_their_meaning() {
        assert_eq!(HingeStatus::from_ui_hinge_status(0), HingeStatus::Unknown);
        assert_eq!(HingeStatus::from_ui_hinge_status(1), HingeStatus::Closed);
        assert_eq!(HingeStatus::from_ui_hinge_status(2), HingeStatus::PartiallyOpen);
        assert_eq!(HingeStatus::from_ui_hinge_status(3), HingeStatus::FullyOpen);
    }

    #[test]
    fn a_status_from_a_later_os_is_unknown() {
        assert_eq!(HingeStatus::from_ui_hinge_status(4), HingeStatus::Unknown);
        assert_eq!(HingeStatus::from_ui_hinge_status(-1), HingeStatus::Unknown);
    }

    #[test]
    fn the_angle_is_radians_and_reads_back_in_degrees() {
        let hinge = HingeState::from_ui_hinge(std::f64::consts::PI, 3);

        assert_eq!(hinge.status, HingeStatus::FullyOpen);
        assert!((hinge.angle - std::f32::consts::PI).abs() < 1e-6);
        assert!((hinge.degrees() - 180.0).abs() < 1e-3);
    }

    #[test]
    fn a_non_finite_angle_is_not_a_measurement() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let hinge = HingeState::from_ui_hinge(bad, 2);
            assert_eq!(hinge, HingeState::new(0.0, HingeStatus::Unknown));
        }
    }

    #[test]
    fn equal_readings_compare_equal() {
        // The state store skips a repeated reading by comparing; a NaN angle
        // would defeat that, which is why `new` refuses one.
        let a = HingeState::from_ui_hinge(f64::NAN, 2);
        let b = HingeState::from_ui_hinge(f64::NAN, 2);
        assert_eq!(a, b);
    }
}
