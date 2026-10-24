use std::cell::Cell;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_events::window::request_animation_frame;
use aimer_widget::base::*;
use aimer_widget::{
    AnyElement, CompositorAnimationDecision, CompositorAnimationFrame, CompositorTransform,
    Drawable, Element, EventElement, EventResult, LayoutElement, Rebuildable, RequiredChild,
    VisitorElement, Widget,
};

use crate::control::controller::AnimationController;
use crate::primitives::time::AnimInstant;

/// Describes the visual property that an [`Animated`] widget interpolates.
#[derive(Debug, Clone, Copy)]
pub enum AnimationEffect {
    /// Animate opacity from `from` to `to` (0.0 = invisible, 1.0 = fully
    /// opaque).
    Opacity { from: f32, to: f32 },
    /// Animate uniform scale from `from` to `to` (1.0 = normal size).
    Scale { from: f32, to: f32 },
    /// Animate translation offset in pixels.
    Translate {
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
    },
    /// Animate rotation in radians.
    Rotate { from: f32, to: f32 },
    /// Animate a slide-in from a direction (0.0 = off-screen, 1.0 = in place).
    SlideX { from: f32, to: f32 },
    /// Animate a slide-in vertically.
    SlideY { from: f32, to: f32 },
}

impl AnimationEffect {
    /// Interpolate between `from` and `to` using progress `t` (0.0–1.0).
    fn lerp(from: f32, to: f32, t: f32) -> f32 {
        from + (to - from) * t
    }

    #[inline]
    fn is_finite(self) -> bool {
        match self {
            Self::Opacity { from, to } | Self::Scale { from, to } | Self::Rotate { from, to } => {
                from.is_finite() && to.is_finite()
            }
            Self::Translate {
                from_x,
                from_y,
                to_x,
                to_y,
            } => from_x.is_finite() && from_y.is_finite() && to_x.is_finite() && to_y.is_finite(),
            Self::SlideX { from, to } | Self::SlideY { from, to } => {
                from.is_finite() && to.is_finite()
            }
        }
    }

    #[inline]
    fn sampled_value_is_finite(self, t: f32) -> bool {
        if !t.is_finite() || !self.is_finite() {
            return false;
        }
        match self {
            Self::Opacity { from, to } | Self::Scale { from, to } | Self::Rotate { from, to } => {
                Self::lerp(from, to, t).is_finite()
            }
            Self::Translate {
                from_x,
                from_y,
                to_x,
                to_y,
            } => Self::lerp(from_x, to_x, t).is_finite() && Self::lerp(from_y, to_y, t).is_finite(),
            Self::SlideX { from, to } | Self::SlideY { from, to } => {
                Self::lerp(from, to, t).is_finite()
            }
        }
    }

    #[inline]
    fn compositor_frame(
        self,
        ctx: &BuildContext,
        progress: f32,
        active: bool,
        clip: Option<ResolvedSize>,
    ) -> CompositorAnimationFrame {
        let mut valid = self.sampled_value_is_finite(progress);
        let mut opacity = None;
        let transform = match self {
            Self::Opacity { from, to } => {
                opacity = Some(Self::lerp(from, to, progress));
                CompositorTransform::Identity
            }
            Self::Scale { from, to } => {
                let scale = Self::lerp(from, to, progress);
                let cx = ctx.box_constraint.max_width / 2.0;
                let cy = ctx.box_constraint.max_height / 2.0;
                valid &= cx.is_finite() && cy.is_finite();
                CompositorTransform::Scale {
                    sx: scale,
                    sy: scale,
                    origin_x: cx,
                    origin_y: cy,
                }
            }
            Self::Translate {
                from_x,
                from_y,
                to_x,
                to_y,
            } => CompositorTransform::Translate {
                x: Self::lerp(from_x, to_x, progress),
                y: Self::lerp(from_y, to_y, progress),
            },
            Self::Rotate { from, to } => {
                let radians = Self::lerp(from, to, progress);
                let cx = ctx.box_constraint.max_width / 2.0;
                let cy = ctx.box_constraint.max_height / 2.0;
                valid &= cx.is_finite() && cy.is_finite();
                CompositorTransform::Rotate {
                    radians,
                    origin_x: cx,
                    origin_y: cy,
                }
            }
            Self::SlideX { from, to } => {
                let offset = Self::lerp(from, to, progress);
                let x = ctx.box_constraint.max_width * offset;
                valid &= x.is_finite();
                CompositorTransform::Translate { x, y: 0.0 }
            }
            Self::SlideY { from, to } => {
                let offset = Self::lerp(from, to, progress);
                let y = ctx.box_constraint.max_height * offset;
                valid &= y.is_finite();
                CompositorTransform::Translate { x: 0.0, y }
            }
        };
        valid &= clip.is_none_or(|size| {
            size.width.is_finite()
                && size.height.is_finite()
                && size.width >= 0.0
                && size.height >= 0.0
        });

        CompositorAnimationFrame::new(progress, transform, opacity, clip, active, valid)
    }
}

/// A widget that wraps a child and animates it using an
/// [`AnimationController`].
///
/// The `Animated` widget applies a canvas transform (opacity, scale, translate,
/// or rotate) to its child based on the current animation progress. Its element
/// ticks the controller directly while drawing, applies the resulting value,
/// and requests another redraw while the controller remains active.
///
/// # Example
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::{Animated, AnimationController, AnimationEffect, curves};
/// use aimer_widget::ErrorWidget;
///
/// let controller = AnimationController::new(Duration::from_millis(250), curves::LINEAR);
/// let animated = Animated::new(controller,
///                              AnimationEffect::Opacity { from: 0.0, to: 1.0 },
///                              ErrorWidget::new("Unable to load preview"));
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_animation::Animated", schema_only)]
pub struct Animated<T = RequiredChild> {
    #[portable_skip]
    pub controller: AnimationController,
    #[portable_skip]
    pub effect: AnimationEffect,
    #[portable_child]
    pub child: T,
}
//
// impl Animated {
//     pub fn new() -> Self {
//         Self {
//             controller: AnimationController::new(),
//             effect: AnimationEffect::Opacity { from: 0.0, to: 1.0 },
//             child: RequiredChild,
//         }
//     }
// }

impl<T: Widget> Animated<T> {
    /// Wraps `child` in a controller-driven visual effect.
    ///
    /// The controller is sampled during drawing. Its configured curve is
    /// applied by [`AnimationController::tick`], and redraws continue only
    /// while the controller is animating. This constructor does not start or
    /// reset the controller; callers control its lifecycle.
    pub fn new(controller: AnimationController, effect: AnimationEffect, child: T) -> Self {
        Self {
            controller,
            effect,
            child,
        }
    }
}

impl<T: Widget + 'static> Widget for Animated<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child_element = self.child.to_element(ctx);

        let controller = self.controller.clone();
        let animating = Cell::new(self.controller.is_animating());

        // Create a StateUpdater-like mechanism: we use a shared dirty flag + window ref
        // to request redraws while the animation is running.
        let window = ctx.window.clone();
        AnimatedElement {
            child: child_element,
            controller,
            effect: self.effect,
            animating,
            window,
        }
        .boxed()
    }
}

/// The element produced by [`Animated`]. On each frame it ticks the
/// controller, updates the child, then requests another redraw if the
/// animation is still running. The retained render tree applies the sampled
/// transform and opacity to the child's node.
struct AnimatedElement {
    child: AnyElement,
    controller: AnimationController,
    effect: AnimationEffect,
    animating: Cell<bool>,
    window: WindowHandle,
}

// Safety: rendering pipeline is single-threaded
unsafe impl Send for AnimatedElement {}
unsafe impl Sync for AnimatedElement {}

impl Drawable for AnimatedElement {
    fn update(&self, ctx: &BuildContext) {
        let frame = self.sample_frame(ctx);
        self.child.update(ctx);
        if frame.active {
            request_animation_frame();
        }
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    #[inline]
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

    #[inline]
    fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
        let frame = self.sample_frame(ctx);
        if frame.valid {
            CompositorAnimationDecision::Compositor(frame)
        } else {
            CompositorAnimationDecision::Live(frame)
        }
    }
}

impl AnimatedElement {
    #[inline]
    fn sample_frame(&self, ctx: &BuildContext) -> CompositorAnimationFrame {
        let now = AnimInstant::now();
        let progress = self.controller.tick(now);
        let active = self.controller.is_animating();
        self.animating.set(active);
        self.effect
            .compositor_frame(ctx, progress, active, Some(self.child.computed_size(ctx)))
    }
}

impl VisitorElement for AnimatedElement {
    /// The animated child is drawn by this element, so the retained render tree
    /// (built from this visitor) must see it; `event_children` alone is not
    /// enough to give it a render node.
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedElement"
    }
}

impl EventElement for AnimatedElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.child.on_event(event)
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl Rebuildable for AnimatedElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}

impl LayoutElement for AnimatedElement {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }
}
