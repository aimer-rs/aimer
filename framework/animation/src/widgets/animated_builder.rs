use std::cell::{Cell, UnsafeCell};
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_widget::base::*;
use aimer_widget::portable::__anteros::{
    BUILTIN_WIDGET_SCHEMA_VERSION, ChildCardinality, PortableWidgetSchemaMetadata, PropertyId,
    PropertySchemaMetadata, PropertyValueKind, WidgetDocumentView, WidgetNodeView,
    WidgetSchemaMetadata,
};
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "illumos",
))]
use aimer_widget::portable::PortableNativeWidgetRegistration;
use aimer_widget::portable::{
    PortableMaterializeError, PortableNativeWidget, PortableWidgetSchema,
    optional_materialized_property, required_materialized_property,
};
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, Drawable, Element, EventElement, EventResult,
    LayoutElement, Rebuildable, VisitorElement, Widget, carry_element_state,
};

#[cfg(feature = "portable-guest")]
use aimer_widget::portable::__anteros::PropertyValue;
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{
    PortableBuildContext, PortableBuildError, PortableNodeId, SourceFingerprint,
};

use crate::control::controller::{AnimationController, AnimationStatus};
use crate::control::keyframe_timeline::{KeyframeTimeline, TimelineFrame};
use crate::control::timed_keyframe::TimedKeyframeAnimation;
use crate::control::tween_animation::TweenAnimation;
use crate::primitives::animatable::Animatable;
use crate::primitives::curve::Curve;
use crate::primitives::time::AnimInstant;

type AnimatedElementBuilder = dyn Fn(f32, &BuildContext) -> AnyElement;

#[cfg(feature = "portable-guest")]
type AnimatedPortableBuilder = dyn Fn(
    f32,
    &mut PortableBuildContext,
    SourceFingerprint,
) -> Result<PortableNodeId, PortableBuildError>;

const ANIMATED_BUILDER_WIDGET_CANONICAL_NAME: &str =
    "aimer.widget:aimer_animation::AnimatedBuilder";
const ANIMATED_BUILDER_DURATION_CANONICAL_NAME: &str =
    "aimer.property:aimer_animation::AnimatedBuilder:duration_millis";
const ANIMATED_BUILDER_CURVE_CANONICAL_NAME: &str =
    "aimer.property:aimer_animation::AnimatedBuilder:curve";
const ANIMATED_BUILDER_CURVE_X1_CANONICAL_NAME: &str =
    "aimer.property:aimer_animation::AnimatedBuilder:curve_x1";
const ANIMATED_BUILDER_CURVE_Y1_CANONICAL_NAME: &str =
    "aimer.property:aimer_animation::AnimatedBuilder:curve_y1";
const ANIMATED_BUILDER_CURVE_X2_CANONICAL_NAME: &str =
    "aimer.property:aimer_animation::AnimatedBuilder:curve_x2";
const ANIMATED_BUILDER_CURVE_Y2_CANONICAL_NAME: &str =
    "aimer.property:aimer_animation::AnimatedBuilder:curve_y2";
const ANIMATED_BUILDER_VALUE_CANONICAL_NAME: &str =
    "aimer.property:aimer_animation::AnimatedBuilder:value";
const ANIMATED_BUILDER_STATUS_CANONICAL_NAME: &str =
    "aimer.property:aimer_animation::AnimatedBuilder:status";
const ANIMATED_BUILDER_REPEAT_CANONICAL_NAME: &str =
    "aimer.property:aimer_animation::AnimatedBuilder:repeat";
const ANIMATED_BUILDER_AUTO_REVERSE_CANONICAL_NAME: &str =
    "aimer.property:aimer_animation::AnimatedBuilder:auto_reverse";

const PROPERTY_ANIMATED_BUILDER_DURATION: PropertyId =
    PropertyId::from_canonical_name(ANIMATED_BUILDER_DURATION_CANONICAL_NAME);
const PROPERTY_ANIMATED_BUILDER_CURVE: PropertyId =
    PropertyId::from_canonical_name(ANIMATED_BUILDER_CURVE_CANONICAL_NAME);
const PROPERTY_ANIMATED_BUILDER_CURVE_X1: PropertyId =
    PropertyId::from_canonical_name(ANIMATED_BUILDER_CURVE_X1_CANONICAL_NAME);
const PROPERTY_ANIMATED_BUILDER_CURVE_Y1: PropertyId =
    PropertyId::from_canonical_name(ANIMATED_BUILDER_CURVE_Y1_CANONICAL_NAME);
const PROPERTY_ANIMATED_BUILDER_CURVE_X2: PropertyId =
    PropertyId::from_canonical_name(ANIMATED_BUILDER_CURVE_X2_CANONICAL_NAME);
const PROPERTY_ANIMATED_BUILDER_CURVE_Y2: PropertyId =
    PropertyId::from_canonical_name(ANIMATED_BUILDER_CURVE_Y2_CANONICAL_NAME);
const PROPERTY_ANIMATED_BUILDER_VALUE: PropertyId =
    PropertyId::from_canonical_name(ANIMATED_BUILDER_VALUE_CANONICAL_NAME);
const PROPERTY_ANIMATED_BUILDER_STATUS: PropertyId =
    PropertyId::from_canonical_name(ANIMATED_BUILDER_STATUS_CANONICAL_NAME);
const PROPERTY_ANIMATED_BUILDER_REPEAT: PropertyId =
    PropertyId::from_canonical_name(ANIMATED_BUILDER_REPEAT_CANONICAL_NAME);
const PROPERTY_ANIMATED_BUILDER_AUTO_REVERSE: PropertyId =
    PropertyId::from_canonical_name(ANIMATED_BUILDER_AUTO_REVERSE_CANONICAL_NAME);

const ANIMATED_BUILDER_PROPERTIES: &[PropertySchemaMetadata<'static>] = &[
    PropertySchemaMetadata::from_canonical_name(
        ANIMATED_BUILDER_DURATION_CANONICAL_NAME,
        PropertyValueKind::I64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        ANIMATED_BUILDER_CURVE_CANONICAL_NAME,
        PropertyValueKind::I64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        ANIMATED_BUILDER_CURVE_X1_CANONICAL_NAME,
        PropertyValueKind::F64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        ANIMATED_BUILDER_CURVE_Y1_CANONICAL_NAME,
        PropertyValueKind::F64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        ANIMATED_BUILDER_CURVE_X2_CANONICAL_NAME,
        PropertyValueKind::F64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        ANIMATED_BUILDER_CURVE_Y2_CANONICAL_NAME,
        PropertyValueKind::F64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        ANIMATED_BUILDER_VALUE_CANONICAL_NAME,
        PropertyValueKind::F64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        ANIMATED_BUILDER_STATUS_CANONICAL_NAME,
        PropertyValueKind::I64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        ANIMATED_BUILDER_REPEAT_CANONICAL_NAME,
        PropertyValueKind::Bool,
    ),
    PropertySchemaMetadata::from_canonical_name(
        ANIMATED_BUILDER_AUTO_REVERSE_CANONICAL_NAME,
        PropertyValueKind::Bool,
    ),
];

pub(crate) const ANIMATED_BUILDER_SCHEMA: PortableWidgetSchemaMetadata<'static> =
    PortableWidgetSchemaMetadata::new(
        WidgetSchemaMetadata::from_canonical_name(
            ANIMATED_BUILDER_WIDGET_CANONICAL_NAME,
            BUILTIN_WIDGET_SCHEMA_VERSION,
            BUILTIN_WIDGET_SCHEMA_VERSION,
        ),
        ANIMATED_BUILDER_PROPERTIES,
        &[],
        ChildCardinality::exactly(1),
    );

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
    #[cfg(feature = "portable-guest")]
    portable_builder: Rc<AnimatedPortableBuilder>,
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
        #[cfg(feature = "portable-guest")]
        let portable_source = Rc::clone(&builder);
        let builder: Rc<AnimatedElementBuilder> =
            Rc::new(move |value: f32, ctx: &BuildContext| (native_builder)(value).to_element(ctx));
        #[cfg(feature = "portable-guest")]
        let portable_builder: Rc<AnimatedPortableBuilder> = Rc::new(move |value, ctx, source| {
            (portable_source)(value).to_portable_node(ctx, source)
        });
        Self {
            controller,
            builder,
            live: false,
            #[cfg(feature = "portable-guest")]
            portable_builder,
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

impl PortableWidgetSchema for AnimatedBuilder {
    const SCHEMA: PortableWidgetSchemaMetadata<'static> = ANIMATED_BUILDER_SCHEMA;
}

#[cfg(not(feature = "portable-guest"))]
impl aimer_widget::portable::PortableWidget for AnimatedBuilder {}

#[cfg(feature = "portable-guest")]
impl aimer_widget::portable::PortableWidget for AnimatedBuilder {
    fn to_portable_node(
        self,
        ctx: &mut PortableBuildContext,
        source: SourceFingerprint,
    ) -> Result<PortableNodeId, PortableBuildError> {
        let duration_millis =
            i64::try_from(self.controller.duration().as_millis()).map_err(|_| {
                property_encoding_error(
                    PROPERTY_ANIMATED_BUILDER_DURATION,
                    ANIMATED_BUILDER_DURATION_CANONICAL_NAME,
                    source,
                    PortableBuildError::InvalidPropertyValue {
                        rust_type: "Duration",
                    },
                )
            })?;
        let value = self.controller.value();
        if !value.is_finite() {
            return Err(property_encoding_error(
                PROPERTY_ANIMATED_BUILDER_VALUE,
                ANIMATED_BUILDER_VALUE_CANONICAL_NAME,
                source,
                PortableBuildError::NonFiniteFloat,
            ));
        }
        let (curve_tag, control_points) = portable_curve(&self.controller.curve()).ok_or_else(|| {
            property_encoding_error(
                PROPERTY_ANIMATED_BUILDER_CURVE,
                ANIMATED_BUILDER_CURVE_CANONICAL_NAME,
                source,
                PortableBuildError::InvalidPropertyValue {
                    rust_type: "custom Curve (only standard curves and cubic Béziers can be lowered to a portable guest)",
                },
            )
        })?;
        let mut properties = vec![
            aimer_widget::portable::__anteros::WidgetProperty::new(
                PROPERTY_ANIMATED_BUILDER_DURATION,
                PropertyValue::I64(duration_millis),
            ),
            aimer_widget::portable::__anteros::WidgetProperty::new(
                PROPERTY_ANIMATED_BUILDER_CURVE,
                PropertyValue::I64(curve_tag),
            ),
            aimer_widget::portable::__anteros::WidgetProperty::new(
                PROPERTY_ANIMATED_BUILDER_VALUE,
                PropertyValue::F64(value as f64),
            ),
            aimer_widget::portable::__anteros::WidgetProperty::new(
                PROPERTY_ANIMATED_BUILDER_STATUS,
                PropertyValue::I64(portable_status(self.controller.status())),
            ),
            aimer_widget::portable::__anteros::WidgetProperty::new(
                PROPERTY_ANIMATED_BUILDER_REPEAT,
                PropertyValue::Bool(self.controller.repeat()),
            ),
            aimer_widget::portable::__anteros::WidgetProperty::new(
                PROPERTY_ANIMATED_BUILDER_AUTO_REVERSE,
                PropertyValue::Bool(self.controller.auto_reverse()),
            ),
        ];
        if let Some([x1, y1, x2, y2]) = control_points {
            let points = [
                (
                    PROPERTY_ANIMATED_BUILDER_CURVE_X1,
                    ANIMATED_BUILDER_CURVE_X1_CANONICAL_NAME,
                    x1,
                ),
                (
                    PROPERTY_ANIMATED_BUILDER_CURVE_Y1,
                    ANIMATED_BUILDER_CURVE_Y1_CANONICAL_NAME,
                    y1,
                ),
                (
                    PROPERTY_ANIMATED_BUILDER_CURVE_X2,
                    ANIMATED_BUILDER_CURVE_X2_CANONICAL_NAME,
                    x2,
                ),
                (
                    PROPERTY_ANIMATED_BUILDER_CURVE_Y2,
                    ANIMATED_BUILDER_CURVE_Y2_CANONICAL_NAME,
                    y2,
                ),
            ];
            for (property, property_name, value) in points {
                if !value.is_finite() {
                    return Err(property_encoding_error(
                        property,
                        property_name,
                        source,
                        PortableBuildError::NonFiniteFloat,
                    ));
                }
                properties.push(
                    aimer_widget::portable::__anteros::WidgetProperty::new(
                        property,
                        PropertyValue::F64(value),
                    )
                    .optional(),
                );
            }
        }
        let curved_value = self.controller.curve().transform(value);
        let child = (self.portable_builder)(curved_value, ctx, source.child(0))?;
        ctx.push_node(
            ANIMATED_BUILDER_SCHEMA.widget().id(),
            BUILTIN_WIDGET_SCHEMA_VERSION,
            None,
            source,
            &properties,
            &[child],
        )
    }
}

#[cfg(feature = "portable-guest")]
fn property_encoding_error(
    property: PropertyId,
    property_name: &'static str,
    source: SourceFingerprint,
    cause: PortableBuildError,
) -> PortableBuildError {
    PortableBuildError::PropertyEncoding {
        property,
        property_name,
        source,
        cause: Box::new(cause),
    }
}

/// The portable tag and control points of `curve`, or `None` for a custom
/// curve, which only exists natively.
#[cfg(feature = "portable-guest")]
fn portable_curve(curve: &crate::primitives::curve::AnyCurve) -> Option<(i64, Option<[f64; 4]>)> {
    use crate::primitives::curve::Curve;

    let descriptor = curve.descriptor()?;
    Some((descriptor.tag, descriptor.control_points))
}

#[cfg(feature = "portable-guest")]
fn portable_status(status: AnimationStatus) -> i64 {
    match status {
        AnimationStatus::Dismissed => 0,
        AnimationStatus::Forward => 1,
        AnimationStatus::Reverse => 2,
        AnimationStatus::Completed => 3,
    }
}

pub(crate) fn materialize_animated_builder(
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    mut children: Vec<AnyWidget>,
) -> Result<AnyWidget, PortableMaterializeError> {
    if children.len() != 1 {
        return Err(PortableMaterializeError::InvalidChildCount {
            expected: 1,
            actual: children.len(),
        });
    }

    let duration_millis: i64 =
        required_materialized_property(document, &node, PROPERTY_ANIMATED_BUILDER_DURATION)?;
    let duration_millis = u64::try_from(duration_millis).map_err(|_| {
        PortableMaterializeError::InvalidPropertyValue {
            property: PROPERTY_ANIMATED_BUILDER_DURATION,
        }
    })?;
    let curve = materialize_curve(document, &node)?;
    let value: f32 =
        required_materialized_property(document, &node, PROPERTY_ANIMATED_BUILDER_VALUE)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(PortableMaterializeError::InvalidPropertyValue {
            property: PROPERTY_ANIMATED_BUILDER_VALUE,
        });
    }
    let status: i64 =
        required_materialized_property(document, &node, PROPERTY_ANIMATED_BUILDER_STATUS)?;
    let status = match status {
        0 => AnimationStatus::Dismissed,
        1 => AnimationStatus::Forward,
        2 => AnimationStatus::Reverse,
        3 => AnimationStatus::Completed,
        _ => {
            return Err(PortableMaterializeError::InvalidPropertyValue {
                property: PROPERTY_ANIMATED_BUILDER_STATUS,
            });
        }
    };
    let repeat: bool =
        required_materialized_property(document, &node, PROPERTY_ANIMATED_BUILDER_REPEAT)?;
    let auto_reverse: bool =
        required_materialized_property(document, &node, PROPERTY_ANIMATED_BUILDER_AUTO_REVERSE)?;

    let controller = AnimationController::with_millis(duration_millis, curve);
    controller.set_value(value);
    controller.set_repeat(repeat);
    controller.set_auto_reverse(auto_reverse);
    match status {
        AnimationStatus::Forward => controller.forward_from_first_tick(),
        AnimationStatus::Reverse => controller.reverse(),
        AnimationStatus::Dismissed | AnimationStatus::Completed => {}
    }

    let child = ChildBuilder::from_widget(children.pop().ok_or(
        PortableMaterializeError::InvalidChildCount {
            expected: 1,
            actual: 0,
        },
    )?);
    Ok(AnimatedBuilder::new(controller, move |_| child.clone()).boxed())
}

fn materialize_curve(
    document: &WidgetDocumentView<'_>,
    node: &WidgetNodeView<'_>,
) -> Result<crate::primitives::curve::AnyCurve, PortableMaterializeError> {
    use crate::primitives::curve::{AnyCurve, Curve, curves};

    let tag: i64 = required_materialized_property(document, node, PROPERTY_ANIMATED_BUILDER_CURVE)?;
    let x1: Option<f64> =
        optional_materialized_property(document, node, PROPERTY_ANIMATED_BUILDER_CURVE_X1)?;
    let y1: Option<f64> =
        optional_materialized_property(document, node, PROPERTY_ANIMATED_BUILDER_CURVE_Y1)?;
    let x2: Option<f64> =
        optional_materialized_property(document, node, PROPERTY_ANIMATED_BUILDER_CURVE_X2)?;
    let y2: Option<f64> =
        optional_materialized_property(document, node, PROPERTY_ANIMATED_BUILDER_CURVE_Y2)?;
    let points = [x1, y1, x2, y2];
    match tag {
        0..=3 | 5..=14 => {
            if points.iter().any(Option::is_some) {
                return Err(PortableMaterializeError::InvalidPropertyValue {
                    property: PROPERTY_ANIMATED_BUILDER_CURVE,
                });
            }
            AnyCurve::from_descriptor(tag, None).ok_or(PortableMaterializeError::InvalidPropertyValue {
                property: PROPERTY_ANIMATED_BUILDER_CURVE,
            })
        }
        4 => {
            let [Some(x1), Some(y1), Some(x2), Some(y2)] = points else {
                return Err(PortableMaterializeError::InvalidPropertyValue {
                    property: PROPERTY_ANIMATED_BUILDER_CURVE,
                });
            };
            let x1 = finite_curve_coordinate(x1, PROPERTY_ANIMATED_BUILDER_CURVE_X1)?;
            let y1 = finite_curve_coordinate(y1, PROPERTY_ANIMATED_BUILDER_CURVE_Y1)?;
            let x2 = finite_curve_coordinate(x2, PROPERTY_ANIMATED_BUILDER_CURVE_X2)?;
            let y2 = finite_curve_coordinate(y2, PROPERTY_ANIMATED_BUILDER_CURVE_Y2)?;
            Ok(curves::cubic_bezier(x1, y1, x2, y2).into_any())
        }
        _ => Err(PortableMaterializeError::InvalidPropertyValue {
            property: PROPERTY_ANIMATED_BUILDER_CURVE,
        }),
    }
}

fn finite_curve_coordinate(
    value: f64,
    property: PropertyId,
) -> Result<f32, PortableMaterializeError> {
    let value = value as f32;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(PortableMaterializeError::InvalidPropertyValue { property })
    }
}

impl PortableNativeWidget for AnimatedBuilder {
    fn materialize_widget(
        document: &WidgetDocumentView<'_>,
        node: WidgetNodeView<'_>,
        children: Vec<AnyWidget>,
    ) -> Result<AnyWidget, PortableMaterializeError> {
        materialize_animated_builder(document, node, children)
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

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "illumos",
))]
#[aimer_widget::portable::__linkme::distributed_slice(
    aimer_widget::portable::materializer::PORTABLE_NATIVE_WIDGET_SCHEMAS
)]
#[linkme(crate = aimer_widget::portable::__linkme)]
#[allow(non_upper_case_globals)]
pub(crate) static ANIMATED_BUILDER_NATIVE_SCHEMA: PortableWidgetSchemaMetadata<'static> =
    ANIMATED_BUILDER_SCHEMA;

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "illumos",
))]
#[aimer_widget::portable::__linkme::distributed_slice(
    aimer_widget::portable::materializer::PORTABLE_NATIVE_WIDGET_REGISTRATIONS
)]
#[linkme(crate = aimer_widget::portable::__linkme)]
#[allow(non_upper_case_globals)]
pub(crate) static ANIMATED_BUILDER_NATIVE_REGISTRATION: PortableNativeWidgetRegistration =
    PortableNativeWidgetRegistration::new(ANIMATED_BUILDER_SCHEMA, materialize_animated_builder);

/// Ensures this crate's native portable-widget registration is retained when
/// the crate is linked from an archive.
///
/// Linker-section registrations are discovered only from object files that
/// make it into the final link. Hosts that use animation types only at the
/// type level can call this anchor before reading the portable registry.
#[doc(hidden)]
#[inline(never)]
pub fn ensure_portable_native_registrations() {
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "illumos",
    ))]
    {
        std::hint::black_box(&ANIMATED_BUILDER_NATIVE_SCHEMA);
        std::hint::black_box(&ANIMATED_BUILDER_NATIVE_REGISTRATION);
    }
}

#[cfg(test)]
mod tests {
    use aimer_widget::portable::__anteros::{
        ChildCardinality, ModelLimits, PropertyId, PropertyPresence, PropertyValue, Version,
        WidgetDocument, WidgetDocumentView, WidgetNode, WidgetProperty, WidgetSchemaId,
    };
    use aimer_widget::portable::{
        PortableNativeWidgetRegistration, PortableWidgetSchema,
        linked_portable_native_widget_registrations, linked_portable_native_widget_schemas,
    };
    use aimer_widget::{AnyElement, PortableWidget, Widget};

    use super::*;
    #[cfg(feature = "portable-guest")]
    use crate::primitives::curve::curves;

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

    fn property_id(canonical_name: &str) -> PropertyId {
        PropertyId::from_canonical_name(canonical_name)
    }

    struct PortableLeaf;

    impl Widget for PortableLeaf {
        fn to_element(self, _ctx: &aimer_widget::base::BuildContext) -> AnyElement {
            panic!("portable test leaf must not build natively");
        }
    }

    impl PortableWidget for PortableLeaf {
        #[cfg(feature = "portable-guest")]
        fn to_portable_node(
            self,
            ctx: &mut aimer_widget::portable::PortableBuildContext,
            source: aimer_widget::portable::SourceFingerprint,
        ) -> Result<
            aimer_widget::portable::PortableNodeId,
            aimer_widget::portable::PortableBuildError,
        > {
            ctx.push_node(
                WidgetSchemaId::from_canonical_name("aimer.widget:test::PortableLeaf"),
                Version::new(1, 0),
                None,
                source,
                &[],
                &[],
            )
        }
    }

    fn valid_properties() -> Vec<WidgetProperty> {
        vec![
            WidgetProperty::new(
                property_id(PROPERTY_DURATION_CANONICAL_NAME),
                PropertyValue::I64(321),
            ),
            WidgetProperty::new(
                property_id(PROPERTY_CURVE_CANONICAL_NAME),
                PropertyValue::I64(4),
            ),
            WidgetProperty::new(
                property_id(PROPERTY_CURVE_X1_CANONICAL_NAME),
                PropertyValue::F64(0.1),
            )
            .optional(),
            WidgetProperty::new(
                property_id(PROPERTY_CURVE_Y1_CANONICAL_NAME),
                PropertyValue::F64(0.2),
            )
            .optional(),
            WidgetProperty::new(
                property_id(PROPERTY_CURVE_X2_CANONICAL_NAME),
                PropertyValue::F64(0.8),
            )
            .optional(),
            WidgetProperty::new(
                property_id(PROPERTY_CURVE_Y2_CANONICAL_NAME),
                PropertyValue::F64(0.9),
            )
            .optional(),
            WidgetProperty::new(
                property_id(PROPERTY_VALUE_CANONICAL_NAME),
                PropertyValue::F64(0.25),
            ),
            WidgetProperty::new(
                property_id(PROPERTY_STATUS_CANONICAL_NAME),
                PropertyValue::I64(1),
            ),
            WidgetProperty::new(
                property_id(PROPERTY_REPEAT_CANONICAL_NAME),
                PropertyValue::Bool(true),
            ),
            WidgetProperty::new(
                property_id(PROPERTY_AUTO_REVERSE_CANONICAL_NAME),
                PropertyValue::Bool(true),
            ),
        ]
    }

    fn host_document<R>(
        properties: &[WidgetProperty],
        child_count: usize,
        inspect: impl FnOnce(
            &WidgetDocumentView<'_>,
            aimer_widget::portable::__anteros::WidgetNodeView<'_>,
        ) -> R,
    ) -> R {
        let child_indices = [1_u32, 2_u32];
        let mut nodes = Vec::with_capacity(child_count + 1);
        nodes.push(
            WidgetNode::new(
                WidgetSchemaId::from_canonical_name(WIDGET_CANONICAL_NAME),
                Version::new(1, 0),
            )
            .properties(properties)
            .children(&child_indices[..child_count]),
        );
        for index in 0..child_count {
            nodes.push(WidgetNode::new(
                WidgetSchemaId::from_canonical_name(if index == 0 {
                    "aimer.widget:test::Child1"
                } else {
                    "aimer.widget:test::Child2"
                }),
                Version::new(1, 0),
            ));
        }
        let image = WidgetDocument::new(1, 0, 0, &nodes, &[], &[])
            .encode(ModelLimits::new(4_096, 32, 64, 128))
            .unwrap();
        let document =
            WidgetDocumentView::decode(&image, ModelLimits::new(4_096, 32, 64, 128)).unwrap();
        inspect(&document, document.node(0).unwrap())
    }

    #[test]
    fn schema_declares_controller_properties_and_one_structural_child() {
        let schema = <AnimatedBuilder as PortableWidgetSchema>::SCHEMA;

        assert_eq!(
            schema.widget().id(),
            WidgetSchemaId::from_canonical_name(WIDGET_CANONICAL_NAME)
        );
        assert_eq!(schema.children(), ChildCardinality::exactly(1));
        assert_eq!(schema.properties().len(), 10);
        assert_eq!(
            schema.properties()[0].id(),
            property_id(PROPERTY_DURATION_CANONICAL_NAME)
        );
        assert_eq!(
            schema.properties()[2].presence(),
            PropertyPresence::Optional
        );
        assert_eq!(
            schema.properties()[5].presence(),
            PropertyPresence::Optional
        );
    }

    #[cfg(feature = "portable-guest")]
    fn portable_context() -> aimer_widget::portable::PortableBuildContext {
        aimer_widget::portable::PortableBuildContext::new(
            1,
            0,
            aimer_widget::portable::PortableWidgetLimits::new(8, 32, 8, 8, 64, 4_096),
            aimer_widget::portable::PortableLimits::new(8, 16, 64, 128, 4_096),
        )
        .unwrap()
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn guest_lowering_round_trips_controller_configuration_and_child() {
        let mut context = portable_context();
        let source = aimer_widget::portable::SourceFingerprint::new(
            aimer_widget::portable::StableId128::from_bytes([9; 16]),
        );
        let root = AnimatedBuilder::new(
            AnimationController::with_millis(321, curves::cubic_bezier(0.1, 0.2, 0.8, 0.9)),
            |_| PortableLeaf,
        )
        .to_portable_node(&mut context, source)
        .unwrap();
        let document = context.finish_document(root).unwrap();
        let limits = document.model_limits();
        let image = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&image, limits).unwrap();
        let node = view.node(view.root_node()).unwrap();

        assert_eq!(
            node.widget_type(),
            WidgetSchemaId::from_canonical_name(WIDGET_CANONICAL_NAME)
        );
        assert_eq!(node.children().count(), 1);
        assert_eq!(
            node.properties()
                .find(|property| property.property_id()
                    == property_id(PROPERTY_DURATION_CANONICAL_NAME))
                .unwrap()
                .value(),
            PropertyValue::I64(321)
        );
        assert_eq!(
            node.properties()
                .find(
                    |property| property.property_id() == property_id(PROPERTY_CURVE_CANONICAL_NAME)
                )
                .unwrap()
                .value(),
            PropertyValue::I64(4)
        );
        assert_eq!(
            node.properties()
                .find(
                    |property| property.property_id() == property_id(PROPERTY_VALUE_CANONICAL_NAME)
                )
                .unwrap()
                .value(),
            PropertyValue::F64(0.0)
        );

        let child = vec![PortableLeaf.boxed()];
        let materialized = materialize_animated_builder(&view, node, child).unwrap();
        assert!(materialized.is_inline() || materialized.is_heap());
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn guest_lowering_rejects_non_finite_curve_control_points() {
        let mut context = portable_context();
        let source = aimer_widget::portable::SourceFingerprint::new(
            aimer_widget::portable::StableId128::from_bytes([7; 16]),
        );
        let error = AnimatedBuilder::new(
            AnimationController::with_millis(321, curves::cubic_bezier(f32::NAN, 0.2, 0.8, 0.9)),
            |_| PortableLeaf,
        )
        .to_portable_node(&mut context, source)
        .unwrap_err();

        assert!(matches!(
            error,
            aimer_widget::portable::PortableBuildError::PropertyEncoding {
                property,
                cause,
                ..
            } if property == property_id(PROPERTY_CURVE_X1_CANONICAL_NAME)
                && matches!(*cause, aimer_widget::portable::PortableBuildError::NonFiniteFloat)
        ));
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "illumos",
    ))]
    #[test]
    fn linker_registration_exposes_animated_builder_constructor() {
        let widget_type = WidgetSchemaId::from_canonical_name(WIDGET_CANONICAL_NAME);
        let schema = linked_portable_native_widget_schemas()
            .iter()
            .copied()
            .find(|schema| schema.widget().id() == widget_type)
            .expect("AnimatedBuilder must publish a host schema");
        assert_eq!(schema.children(), ChildCardinality::exactly(1));
        assert_eq!(schema.properties().len(), 10);

        let registration = linked_portable_native_widget_registrations()
            .iter()
            .copied()
            .find(|registration| registration.widget_type() == widget_type)
            .expect("AnimatedBuilder must publish a host registration");

        assert_eq!(
            registration.schema().children(),
            ChildCardinality::exactly(1)
        );
        assert_eq!(registration.schema().properties().len(), 10);
        let _: PortableNativeWidgetRegistration = registration;
    }

    #[test]
    fn host_materializer_rejects_invalid_controller_properties_and_children() {
        let mut properties = valid_properties();
        properties[0] = WidgetProperty::new(
            property_id(PROPERTY_DURATION_CANONICAL_NAME),
            PropertyValue::I64(-1),
        );
        host_document(&properties, 1, |document, node| {
            assert!(matches!(
                materialize_animated_builder(document, node, vec![PortableLeaf.boxed()]),
                Err(aimer_widget::portable::PortableMaterializeError::InvalidPropertyValue {
                    property
                }) if property == property_id(PROPERTY_DURATION_CANONICAL_NAME)
            ));
        });

        let mut properties = valid_properties();
        properties[1] = WidgetProperty::new(
            property_id(PROPERTY_CURVE_CANONICAL_NAME),
            PropertyValue::I64(99),
        );
        host_document(&properties, 1, |document, node| {
            assert!(matches!(
                materialize_animated_builder(document, node, vec![PortableLeaf.boxed()]),
                Err(aimer_widget::portable::PortableMaterializeError::InvalidPropertyValue {
                    property
                }) if property == property_id(PROPERTY_CURVE_CANONICAL_NAME)
            ));
        });

        let properties = valid_properties();
        host_document(&properties, 0, |document, node| {
            assert!(matches!(
                materialize_animated_builder(document, node, Vec::new()),
                Err(
                    aimer_widget::portable::PortableMaterializeError::InvalidChildCount {
                        expected: 1,
                        actual: 0,
                    }
                )
            ));
        });
    }
}
