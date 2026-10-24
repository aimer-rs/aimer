use crate::primitives::animatable::Animatable;

/// Defines a range between two values of type `T` for animation.
///
/// Given a progress `t` (0.0–1.0, typically after curve transformation),
/// `Tween::lerp` produces the interpolated value.
///
/// # Example
/// ```rust
/// use self::aimer_animation::Tween;
///
/// let tween = Tween::new(0.0f32, 100.0);
/// let value = tween.lerp(0.5); // 50.0
/// ```
#[derive(Debug, Clone)]
pub struct Tween<T: Animatable> {
    pub begin: T,
    pub end: T,
}

impl<T: Animatable> Tween<T> {
    /// Create a new tween from `begin` to `end`.
    pub fn new(begin: T, end: T) -> Self {
        Self { begin, end }
    }

    /// Interpolate between `begin` and `end` at progress `t` (0.0–1.0).
    pub fn lerp(&self, t: f32) -> T {
        self.begin.lerp(&self.end, t)
    }
}

/// Extension trait to create a `Tween` from any `Animatable` value.
pub trait AnimatableExt: Animatable + Sized {
    /// Create a tween from `self` to `end`.
    fn tween_to(self, end: Self) -> Tween<Self>;
}

impl<T: Animatable> AnimatableExt for T {
    fn tween_to(self, end: Self) -> Tween<Self> {
        Tween::new(self, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tween_f32() {
        let t = Tween::new(0.0f32, 100.0);
        assert!((t.lerp(0.0) - 0.0).abs() < 1e-9);
        assert!((t.lerp(0.5) - 50.0).abs() < 1e-9);
        assert!((t.lerp(1.0) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn test_tween_tuple() {
        let t = Tween::new((0.0, 0.0), (10.0, 20.0));
        let r = t.lerp(0.25);
        assert!((r.0 - 2.5).abs() < 1e-9);
        assert!((r.1 - 5.0).abs() < 1e-9);
    }

    #[test]
    fn test_animatable_ext() {
        let t = 0.0f32.tween_to(100.0);
        assert!((t.lerp(0.5) - 50.0).abs() < 1e-9);
    }
}
