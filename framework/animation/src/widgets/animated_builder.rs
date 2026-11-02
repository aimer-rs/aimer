use std::cell::{Cell, UnsafeCell};
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_widget::base::*;
use aimer_widget::{
    AnyElement, ChildBuilder, Drawable, Element, EventElement, EventResult, LayoutElement,
    Rebuildable, VisitorElement, Widget, carry_element_state,
};

use crate::control::controller::AnimationController;
use crate::control::keyframe_timeline::{KeyframeTimeline, TimelineFrame};
use crate::control::timed_keyframe::TimedKeyframeAnimation;
use crate::control::tween_animation::TweenAnimation;
use crate::primitives::animatable::Animatable;
use crate::primitives::curve::Curve;
use crate::primitives::time::AnimInstant;

type AnimatedElementBuilder = dyn Fn(f32, &BuildContext) -> AnyElement;

/// A widget that rebuilds its child on every animation tick.
///
/// Unlike [`crate::widgets::Animated`], which applies a fixed visual effect,
/// `AnimatedBuilder` gives you the current animation value each frame so you
/// can build any widget based on it. The builder receives the controller's
/// curved value. It runs once when the element is created and again whenever
/// that value changes; redraws continue while the controller is animating.
///
/// # Example
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::{AnimatedBuilder, AnimationController, curves};
/// use aimer_widget::ErrorWidget;
///
/// let controller = AnimationController::new(Duration::from_millis(250), curves::LINEAR);
/// let animated = AnimatedBuilder::new(controller, |value| {
///     ErrorWidget::new(format!("Progress: {:.0}%", value * 100.0))
/// });
/// ```
pub struct AnimatedBuilder {
    pub controller: AnimationController,
    builder: Rc<AnimatedElementBuilder>,
    live: bool,
}

impl AnimatedBuilder {
    /// Creates a builder driven by `controller`.
    ///
    /// The closure must be `'static` because the resulting element retains it.
    /// Construction does not start or reset the controller.
    pub fn new<F, W>(controller: AnimationController, builder: F) -> Self
    where
        F: Fn(f32) -> W + 'static,
        W: Widget,
    {
        let builder = Rc::new(builder);
        let native_builder = Rc::clone(&builder);
        let builder: Rc<AnimatedElementBuilder> =
            Rc::new(move |value: f32, ctx: &BuildContext| (native_builder)(value).to_element(ctx));
        Self {
            controller,
            builder,
            live: false,
        }
    }

    /// Creates a builder driven by `controller` that is handed a `child` which is
    /// built once and kept.
    ///
    /// The closure runs on every step of the animation and returns the widgets
    /// that depend on the value. Whatever does not depend on it belongs in
    /// `child`, which the closure places in its output as many times as it is
    /// called: the child is built the first time, is not built again, and the
    /// rebuild that follows each step does not look inside it. A tab bar that
    /// fades in is the same tab bar at every step, so only the fade around it is
    /// worth rebuilding.
    ///
    /// The child keeps its state for as long as the builder exists. It does not
    /// see a change to anything the closure captured; to change the child, make a
    /// new `AnimatedBuilder` with a new one.
    ///
    /// ```rust
    /// use std::time::Duration;
    ///
    /// use aimer_animation::{AnimatedBuilder, AnimationController, curves};
    /// use aimer_widget::{ErrorWidget, Widget};
    ///
    /// let controller = AnimationController::new(Duration::from_millis(250), curves::LINEAR);
    /// // A real builder wraps the child in a fade or a scale; this one just shows
    /// // it from the halfway mark on.
    /// let animated = AnimatedBuilder::with_child(
    ///     controller,
    ///     ErrorWidget::new("built once"),
    ///     |value, child| {
    ///         if value < 0.5 { ErrorWidget::new("not yet").boxed() } else { child.boxed() }
    ///     },
    /// );
    /// ```
    pub fn with_child<C, F, W>(controller: AnimationController, child: C, builder: F) -> Self
    where
        C: Widget + 'static,
        F: Fn(f32, ChildBuilder) -> W + 'static,
        W: Widget,
    {
        let child = ChildBuilder::from_widget(child);
        Self::new(controller, move |value| builder(value, child.clone()))
    }

    /// Keeps rebuilding the child from the controller's value for as long as the
    /// element exists, even while the controller is not animating. Off by default.
    ///
    /// A controller that animates keeps the element in the frame's rebuild pass by
    /// itself. One that is set by hand with
    /// [`set_value`](AnimationController::set_value), as a finger dragging
    /// something does, is never animating, and a pass skips an element nothing has
    /// marked, so the child would be built once and then stay as it was however
    /// the value moved. Use this for such a controller on an element that lives
    /// only as long as the gesture does: while live it costs the pass a visit to
    /// the element every frame.
    pub fn live(mut self, live: bool) -> Self {
        self.live = live;
        self
    }

    /// Creates a builder that receives `animation`'s value instead of raw
    /// progress.
    ///
    /// The builder is called with the interpolated `T` under the same rules as
    /// [`new`](Self::new): once on creation and again whenever the value
    /// changes. The animation's controller drives it, so start the animation
    /// through the handle you kept (clones share playback).
    ///
    /// ```rust
    /// use std::time::Duration;
    ///
    /// use aimer_animation::{AnimatedBuilder, curves, TweenAnimation};
    /// use aimer_widget::ErrorWidget;
    ///
    /// let slide = TweenAnimation::new(0.0f32, 120.0, Duration::from_millis(250), curves::EASE_OUT);
    /// let animated = AnimatedBuilder::tween(slide.clone(), |offset| {
    ///     ErrorWidget::new(format!("offset {offset:.0}"))
    /// });
    /// slide.forward();
    /// ```
    pub fn tween<T, F, W>(animation: TweenAnimation<T>, builder: F) -> Self
    where
        T: Animatable + 'static,
        F: Fn(T) -> W + 'static,
        W: Widget,
    {
        let controller = animation.controller().clone();
        // The builder is handed the controller's curved progress, which is what
        // `TweenAnimation::value` maps through its tween.
        Self::new(controller, move |progress| {
            builder(animation.tween().lerp(progress))
        })
    }

    /// Creates a builder that receives the value of a timed keyframe
    /// animation.
    ///
    /// See [`tween`](Self::tween) for when the builder runs.
    ///
    /// ```rust
    /// use std::time::Duration;
    ///
    /// use aimer_animation::{AnimatedBuilder, KeyframeTrack, TimedKeyframeAnimation};
    /// use aimer_widget::ErrorWidget;
    ///
    /// let ms = Duration::from_millis;
    /// let track = KeyframeTrack::builder()
    ///     .at(ms(0), 1.0f32)
    ///     .at(ms(120), 1.2)
    ///     .at(ms(300), 1.0)
    ///     .build()
    ///     .unwrap();
    /// let pulse = TimedKeyframeAnimation::new(track);
    /// let animated = AnimatedBuilder::keyframes(pulse.clone(), |scale| {
    ///     ErrorWidget::new(format!("scale {scale:.2}"))
    /// });
    /// pulse.forward();
    /// ```
    pub fn keyframes<T, F, W>(animation: TimedKeyframeAnimation<T>, builder: F) -> Self
    where
        T: Animatable + Clone + 'static,
        F: Fn(T) -> W + 'static,
        W: Widget,
    {
        let controller = animation.controller().clone();
        Self::new(controller, move |progress| {
            builder(animation.track().at_progress(progress))
        })
    }

    /// Creates a builder that receives a [`TimelineFrame`], from which any
    /// number of tracks are sampled in step.
    ///
    /// See [`tween`](Self::tween) for when the builder runs.
    ///
    /// ```rust
    /// use std::time::Duration;
    ///
    /// use aimer_animation::{AnimatedBuilder, KeyframeTimeline, KeyframeTrack};
    /// use aimer_widget::ErrorWidget;
    ///
    /// let ms = Duration::from_millis;
    /// let fade = KeyframeTrack::builder().at(ms(0), 0.0f32).at(ms(200), 1.0).build().unwrap();
    /// let grow = KeyframeTrack::builder().at(ms(0), 0.8f32).at(ms(400), 1.0).build().unwrap();
    /// let timeline = KeyframeTimeline::new(Duration::ZERO).including(&fade).including(&grow);
    ///
    /// let animated = AnimatedBuilder::timeline(timeline.clone(), move |frame| {
    ///     ErrorWidget::new(format!("{:.2} {:.2}", frame.sample(&fade), frame.sample(&grow)))
    /// });
    /// timeline.forward();
    /// ```
    pub fn timeline<F, W>(timeline: KeyframeTimeline, builder: F) -> Self
    where
        F: Fn(TimelineFrame) -> W + 'static,
        W: Widget,
    {
        let controller = timeline.controller().clone();
        Self::new(controller, move |progress| {
            builder(timeline.frame_at_progress(progress))
        })
    }
}

impl Widget for AnimatedBuilder {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let curved_value = self.controller.curve().transform(self.controller.value());
        let child = (self.builder)(curved_value, ctx);
        let window = ctx.window.clone();

        AnimatedBuilderElement {
            child: UnsafeCell::new(child),
            controller: self.controller.clone(),
            builder: self.builder.clone(),
            live: self.live,
            last_value: Cell::new(curved_value),
            output_changed: Cell::new(false),
            reachable: aimer_widget::KeepReachable::new(),
            window,
        }
        .boxed()
    }
}

/// The element produced by `AnimatedBuilder`.
///
/// Before the retained render tree synchronizes, it ticks the controller and
/// rebuilds the child from the builder closure when the value changes. The new
/// child is therefore present in the render tree for the same frame that uses
/// its value.
struct AnimatedBuilderElement {
    child: UnsafeCell<AnyElement>,
    controller: AnimationController,
    builder: Rc<AnimatedElementBuilder>,
    /// Whether to stay reachable though the controller is not animating.
    live: bool,
    last_value: Cell<f32>,
    output_changed: Cell<bool>,
    /// Keeps this element on the rebuild pass's dirty-path index while it
    /// animates, so the pass prunes the rest of the tree.
    reachable: aimer_widget::KeepReachable,
    window: WindowHandle,
}

// Safety: rendering pipeline is single-threaded
unsafe impl Send for AnimatedBuilderElement {}
unsafe impl Sync for AnimatedBuilderElement {}

impl Drawable for AnimatedBuilderElement {
    fn update(&self, ctx: &BuildContext) {
        let output_changed = self.output_changed.replace(false);

        if output_changed || self.controller.is_animating() {
            aimer_widget::mark_paint_damage_full();
        }

        unsafe { &*self.child.get() }.update(ctx);

        if self.controller.is_animating() {
            self.window.request_redraw();
        }
    }

    /// The builder records nothing itself: its rebuilt child owns its render
    /// nodes and resolves its own paint source, so the builder never forces the
    /// subtree onto the legacy path.
    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        unsafe { &*self.child.get() }.sync_paint_geometry(ctx);
        true
    }

    #[inline]
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.output_changed.set(false);
        unsafe { &*self.child.get() }.update(ctx);
        if self.controller.is_animating() || self.live {
            if !self.reachable.is_registered() {
                // Created while drawing, so no rebuild pass has visited it yet and
                // none would: the index it is pruned by was built without it. Ask
                // for a pass that looks everywhere; that one registers its path.
                self.reachable.hold();
            }
            // Each frame replaces this element's own child and nothing else, so
            // the next one may revisit just this subtree.
            aimer_widget::request_isolated_subtree_animation_frame(ctx);
        }
    }
}

impl VisitorElement for AnimatedBuilderElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(unsafe { &*self.child.get() }.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedBuilderElement"
    }
}

impl EventElement for AnimatedBuilderElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        unsafe { &*self.child.get() }.on_event(event)
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(unsafe { &*self.child.get() }.as_ref());
    }
}

impl Rebuildable for AnimatedBuilderElement {
    /// Only hands a mark on to its child, so placing it again leaves the
    /// dirty-path index as it was; see [`Rebuildable::forwards_marks_only`].
    #[inline]
    fn forwards_marks_only(&self) -> bool {
        true
    }

    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        let curved_value = self.controller.tick(aimer_widget::frame_instant());
        if self.controller.is_animating() || self.live {
            // Nothing marks this element dirty while it animates, so without
            // this the rebuild pass would skip it and the animation would stop.
            self.reachable.hold();
        } else {
            self.reachable.release();
        }
        let output_changed = curved_value != self.last_value.get();
        if output_changed {
            // The value moved, and nothing else: whatever the builder hands over as
            // a retained child is the same child as last frame, so placing it again
            // must not mark it for rebuild.
            let child = aimer_widget::without_placement_marks(|| (self.builder)(curved_value, ctx));
            // Preserve state through the replacement, then publish the new
            // child before the retained render tree synchronizes its nodes.
            aimer_widget::preserving_dirty_paths(|| {
                carry_element_state(unsafe { &*self.child.get() }.as_ref(), child.as_ref(), ctx)
            });
            unsafe { *self.child.get() = child };
            self.last_value.set(curved_value);
            self.output_changed.set(true);
            aimer_widget::rebuild_replaced_subtree(unsafe { &*self.child.get() }.as_ref(), ctx);
        } else {
            unsafe { &*self.child.get() }.rebuild_if_dirty(ctx);
        }
    }
}

impl LayoutElement for AnimatedBuilderElement {
    fn pos(&self) -> Option<Vec2d> {
        unsafe { &*self.child.get() }.pos()
    }

    fn size(&self) -> Option<Size> {
        unsafe { &*self.child.get() }.size()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        unsafe { &*self.child.get() }.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        unsafe { &*self.child.get() }.content_size(ctx)
    }

    fn get_size_from_child(&self) -> Option<Size> {
        unsafe { &*self.child.get() }.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        unsafe { &*self.child.get() }.invalidate_layout();
    }
}

#[cfg(test)]
mod tests {
    use aimer_widget::{AnyElement, Widget};

    use super::*;

    const WIDGET_CANONICAL_NAME: &str = "aimer.widget:aimer_animation::AnimatedBuilder";
    const PROPERTY_DURATION_CANONICAL_NAME: &str =
        "aimer.property:aimer_animation::AnimatedBuilder:duration_millis";
    const PROPERTY_CURVE_CANONICAL_NAME: &str =
        "aimer.property:aimer_animation::AnimatedBuilder:curve";
    const PROPERTY_CURVE_X1_CANONICAL_NAME: &str =
        "aimer.property:aimer_animation::AnimatedBuilder:curve_x1";
    const PROPERTY_CURVE_Y1_CANONICAL_NAME: &str =
        "aimer.property:aimer_animation::AnimatedBuilder:curve_y1";
    const PROPERTY_CURVE_X2_CANONICAL_NAME: &str =
        "aimer.property:aimer_animation::AnimatedBuilder:curve_x2";
    const PROPERTY_CURVE_Y2_CANONICAL_NAME: &str =
        "aimer.property:aimer_animation::AnimatedBuilder:curve_y2";
    const PROPERTY_VALUE_CANONICAL_NAME: &str =
        "aimer.property:aimer_animation::AnimatedBuilder:value";
    const PROPERTY_STATUS_CANONICAL_NAME: &str =
        "aimer.property:aimer_animation::AnimatedBuilder:status";
    const PROPERTY_REPEAT_CANONICAL_NAME: &str =
        "aimer.property:aimer_animation::AnimatedBuilder:repeat";
    const PROPERTY_AUTO_REVERSE_CANONICAL_NAME: &str =
        "aimer.property:aimer_animation::AnimatedBuilder:auto_reverse";
}
