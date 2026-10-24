use std::time::Duration;

use aimer_animation::{AnyCurve, Curve, curves};

/// Paint-only timing used when a modal enters or leaves the application
/// overlay.
///
/// Modal animation never changes layout. The barrier fades while the content
/// fades and scales around its center, keeping hit testing stable for the whole
/// transition.
#[derive(Clone, Debug, PartialEq)]
pub struct ModalAnimation {
    pub(crate) enter_duration: Duration,
    pub(crate) exit_duration: Duration,
    pub(crate) enter_curve: AnyCurve,
    pub(crate) exit_curve: AnyCurve,
    pub(crate) content_scale_from: f32,
}

impl ModalAnimation {
    /// Creates a subtle fade-and-scale transition.
    pub fn new() -> Self {
        Self {
            enter_duration: Duration::from_millis(200),
            exit_duration: Duration::from_millis(150),
            enter_curve: curves::EASE_OUT.into_any(),
            exit_curve: curves::EASE_IN.into_any(),
            content_scale_from: 0.96,
        }
    }

    /// Sets the time used to reveal the modal.
    pub fn enter_duration(mut self, duration: Duration) -> Self {
        self.enter_duration = duration;
        self
    }

    /// Sets the time used to dismiss the modal.
    pub fn exit_duration(mut self, duration: Duration) -> Self {
        self.exit_duration = duration;
        self
    }

    /// Sets the easing curve used while revealing the modal.
    pub fn enter_curve(mut self, curve: impl Curve) -> Self {
        self.enter_curve = curve.into_any();
        self
    }

    /// Sets the easing curve used while dismissing the modal.
    pub fn exit_curve(mut self, curve: impl Curve) -> Self {
        self.exit_curve = curve.into_any();
        self
    }

    /// Sets the content's initial scale in the inclusive `0.0..=1.0` range.
    pub fn content_scale_from(mut self, scale: f32) -> Self {
        self.content_scale_from = normalize_scale(scale);
        self
    }

    /// Returns the normalized initial content scale.
    pub fn content_scale(&self) -> f32 {
        self.content_scale_from
    }
}

impl Default for ModalAnimation {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_scale(scale: f32) -> f32 {
    if scale.is_nan() {
        1.0
    } else {
        scale.clamp(0.0, 1.0)
    }
}

pub(crate) fn visual_values(progress: f32, scale_from: f32) -> (f32, f32) {
    let progress = progress.clamp(0.0, 1.0);
    let scale_from = normalize_scale(scale_from);
    (progress, scale_from + (1.0 - scale_from) * progress)
}

#[cfg(test)]
mod tests {
    use super::visual_values;

    #[test]
    fn visual_values_keep_layout_endpoints_stable() {
        assert_eq!(visual_values(0.0, 0.9), (0.0, 0.9));
        assert_eq!(visual_values(1.0, 0.9), (1.0, 1.0));
        let (opacity, scale) = visual_values(0.5, 0.9);
        assert!((opacity - 0.5).abs() < f32::EPSILON);
        assert!((scale - 0.95).abs() < f32::EPSILON);
    }
}
