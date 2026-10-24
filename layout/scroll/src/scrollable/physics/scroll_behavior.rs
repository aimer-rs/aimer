use aimer_attribute::position::Vec2d;

#[derive(Clone, Copy)]
pub struct ScrollBehavior {
    pub max_scroll: Vec2d,
    pub min_scroll: Vec2d,
    pub velocity: Vec2d,
    pub scroll_offset: Vec2d,
    pub bouncy: bool,
    pub bouncy_resistance: f32,
    pub bouncy_recovery: f32,
    /// Per-120 Hz-frame velocity retention during a fling.
    ///
    /// Applied as `friction.powf(frame_ratio)` each momentum frame where
    /// `frame_ratio = dt / FRAME_REF_120`, matching UIScrollView's discrete
    /// per-frame deceleration model exactly.
    ///
    /// `0.999` per 120 fps frame ≈ UIScrollView.DecelerationRate.normal (0.998
    /// per 60 fps frame, since `0.999^2 = 0.998`).  At 60 fps this is applied
    /// as `0.999^2` per frame.
    /// Velocity at 1 s: 89 %, 2 s: 79 %, 5 s: 55 %, 10 s: 30 %.
    pub friction: f32,
}

impl ScrollBehavior {
    const DEFAULT_FRICTION: f32 = 0.999;
}

impl Default for ScrollBehavior {
    fn default() -> Self {
        // friction 0.999 per 120 fps ≈ UIScrollView.DecelerationRate.normal
        // (0.998 per 60 fps).  Applied as friction.powf(frame_ratio) each
        // frame (discrete, like UIScrollView), NOT as a continuous exponential.
        // Velocity at 1 s: 89 %, 5 s: 55 %, 10 s: 30 %.
        let defaults = (0.75, 0.045, ScrollBehavior::DEFAULT_FRICTION);

        Self {
            max_scroll: Vec2d {
                x: f32::MAX,
                y: f32::MAX,
            },
            min_scroll: Vec2d { x: 0.0, y: 0.0 },
            velocity: Vec2d { x: 0.0, y: 0.0 },
            scroll_offset: Vec2d { x: 0.0, y: 0.0 },
            bouncy: true,
            bouncy_resistance: defaults.0,
            bouncy_recovery: defaults.1,
            friction: defaults.2,
        }
    }
}

impl ScrollBehavior {
    pub fn no_bounce() -> Self {
        Self {
            bouncy: false,
            ..Default::default()
        }
    }
}

#[derive(Default, Copy, Clone, Debug, PartialEq, Eq, aimer_macro::PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_scroll::scrollable::ScrollAxis",
    max_encoded_bytes = 16
)]
pub enum ScrollAxis {
    #[default]
    #[portable_value(tag = 0)]
    Vertical,
    #[portable_value(tag = 1)]
    Horizontal,
}
