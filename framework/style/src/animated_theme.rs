#[cfg(feature = "portable-guest")]
use std::any::type_name;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Duration;

use aimer_animation::{AnimInstant, AnimationController, AnyCurve, Curve};
use aimer_macro::PortableWidget;
#[cfg(feature = "portable-guest")]
use aimer_provider::with_portable_provider;
use aimer_provider::{Provider, ProviderHandle};
use aimer_widget::base::{BuildContext, ResolvedSize, Size, Vec2d};
use aimer_widget::{
    AnyElement, AnyWidget, Brightness, ChildBuilder, Drawable, Element, EventElement,
    EventTreeRole, Key, LayoutElement, Rebuildable, RequiredChild, State, StateUpdater,
    StatefulElement, StatefulWidget, StatelessElement, VisitorElement, Widget, platform_brightness,
};

#[cfg(feature = "portable-guest")]
use aimer_widget::portable::__anteros::{
    BUILTIN_WIDGET_SCHEMA_VERSION, PROPERTY_ANIMATED_THEME_CURVE, PROPERTY_ANIMATED_THEME_CURVE_X1,
    PROPERTY_ANIMATED_THEME_CURVE_X2, PROPERTY_ANIMATED_THEME_CURVE_Y1,
    PROPERTY_ANIMATED_THEME_CURVE_Y2, PROPERTY_ANIMATED_THEME_DURATION_MILLIS,
    PROPERTY_ANIMATED_THEME_MODE, PROPERTY_ANIMATED_THEME_SCHEMA_VERSION,
    PROPERTY_ANIMATED_THEME_TYPE, PROPERTY_ANIMATED_THEME_VALUE, PropertyValue,
    WIDGET_ANIMATED_THEME, WidgetProperty,
};

use crate::{Theme, ThemeData, ThemeMode, ThemeSelection};

fn request_next_frame() {
    aimer_events::window::request_animation_frame();
}

/// Supplies a [`Theme`] value to descendants, follows the system appearance,
/// and animates every change to it.
///
/// Descendants read the interpolated value with [`Theme::of`]. When the theme
/// changes, `AnimatedTheme` interpolates it without replacing the descendant
/// widget tree. If the target changes during a transition, the new transition
/// begins at the currently displayed theme. A zero duration applies the target
/// immediately.
///
/// The default transition lasts 200 milliseconds and uses [`curves::LINEAR`].
///
/// # Following the system
///
/// A fresh `AnimatedTheme` supplies [`ThemeData::light`] and
/// [`ThemeData::dark`] and follows the appearance the platform reports, so an
/// application that says nothing about theming already switches with the system
/// — and animates while doing it. The switch is followed live: the user
/// changing appearance in the system settings crosses the application into the
/// other theme without restarting it.
///
/// ```
/// use aimer_style::{AnimatedTheme, ThemeData};
/// use aimer_widget::Widget;
///
/// fn themed_app(child: impl Widget + 'static) -> impl Widget {
///     // Light or dark, whichever the system asks for.
///     AnimatedTheme::new().child(child)
/// }
/// ```
///
/// A custom theme follows it too, once both appearances are supplied with
/// [`AnimatedTheme::adaptive`].
///
/// # Ignoring the system
///
/// An application with its own light/dark switch overrides the system with
/// [`AnimatedTheme::mode`], and one with a single theme states it with
/// [`AnimatedTheme::data`]. Neither registers as a follower of the platform, so
/// a system switch cannot rebuild them.
///
/// Changing the mode is a theme change like any other and animates the same
/// way: handing the decision back to a system that asks for the other
/// appearance crosses into it rather than snapping.
///
/// ```
/// use std::time::Duration;
///
/// use aimer_animation::curves;
/// use aimer_style::{AnimatedTheme, ThemeData, ThemeMode};
/// use aimer_widget::Widget;
///
/// fn dark_app(child: impl Widget + 'static) -> impl Widget {
///     AnimatedTheme::new().mode(ThemeMode::Dark)
///                         .duration(Duration::from_millis(300))
///                         .curve(curves::EASE_IN_OUT)
///                         .child(child)
/// }
///
/// fn single_theme_app(child: impl Widget + 'static) -> impl Widget {
///     AnimatedTheme::new().data(ThemeData::dark()).child(child)
/// }
/// ```
///
/// # A theme without a subtree is not a widget
///
/// Supplying a theme to nobody is a mistake worth catching at compile time, so
/// the descendant subtree is part of the type: only [`AnimatedTheme::child`]
/// produces something a parent can build.
///
/// ```compile_fail
/// use aimer_style::AnimatedTheme;
/// use aimer_widget::Widget;
///
/// // error: the trait bound `RequiredChild: Widget` is not satisfied
/// let _ = AnimatedTheme::new().boxed();
/// ```
#[derive(PortableWidget)]
#[portable_widget(id = "aimer_style::AnimatedTheme", schema_only, manual_lowering)]
pub struct AnimatedTheme<W = RequiredChild, T = ThemeData> {
    #[portable_skip]
    selection: ThemeSelection<T>,
    #[portable_skip]
    duration: Duration,
    #[portable_skip]
    curve: AnyCurve,
    #[portable_child]
    child: ChildBuilder,
    // `W` names the child only to keep the type-state: `RequiredChild` does not
    // implement `Widget`, so `AnimatedTheme<RequiredChild, T>` cannot satisfy the
    // bound on the `Widget` impl below and a theme without a subtree stays
    // unbuildable. The subtree itself is erased into the `ChildBuilder`, which
    // holds `ChildBuilder::required` until `child` attaches one — a placeholder
    // that allocates nothing and reports the mistake if it is ever built.
    #[portable_skip]
    marker: PhantomData<W>,
}

impl AnimatedTheme {
    /// Creates an animated theme that follows the system appearance between the
    /// built-in light and dark themes, with the default transition settings.
    ///
    /// Attach the descendant subtree last with [`AnimatedTheme::child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            selection: ThemeSelection::adaptive(ThemeData::light(), ThemeData::dark()),
            duration: Duration::from_millis(200),
            curve: AnyCurve::default(),
            child: ChildBuilder::required(),
            marker: PhantomData,
        }
    }
}

impl Default for AnimatedTheme {
    fn default() -> Self {
        Self::new()
    }
}

/// Cloning an `AnimatedTheme` copies its settings and shares its child subtree,
/// so a build can keep the configuration it has to resolve again on the next
/// appearance change without duplicating the widgets below it.
impl<W, T: Clone> Clone for AnimatedTheme<W, T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            selection: self.selection.clone(),
            duration: self.duration,
            curve: self.curve.clone(),
            child: self.child.clone(),
            marker: PhantomData,
        }
    }
}

impl<W, T> AnimatedTheme<W, T> {
    /// Sets the one theme supplied to descendants, whatever the system asks
    /// for.
    ///
    /// This replaces any light/dark pair set before it: an application that
    /// names a single theme means to use it, so the result no longer follows the
    /// system. Add an appearance back with [`AnimatedTheme::dark`].
    #[inline]
    pub fn data<U: Theme>(self, data: U) -> AnimatedTheme<W, U> {
        AnimatedTheme {
            selection: ThemeSelection::fixed(data),
            duration: self.duration,
            curve: self.curve,
            child: self.child,
            marker: PhantomData,
        }
    }

    /// Sets the pair of themes to follow the system appearance between.
    ///
    /// This is how a custom theme adapts: the platform reports an appearance and
    /// the matching half of the pair is supplied to descendants, animated like
    /// any other theme change.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// AnimatedTheme::new().adaptive(BrandTheme::light(), BrandTheme::dark())
    ///                     .child(app)
    /// ```
    #[inline]
    pub fn adaptive<U: Theme>(self, light: U, dark: U) -> AnimatedTheme<W, U> {
        AnimatedTheme {
            selection: ThemeSelection::adaptive(light, dark),
            duration: self.duration,
            curve: self.curve,
            child: self.child,
            marker: PhantomData,
        }
    }

    /// Sets the theme used when the resolved appearance is dark, keeping the
    /// current theme as the light one.
    #[inline]
    pub fn dark(mut self, dark: T) -> Self {
        self.selection.dark = Some(dark);
        self
    }

    /// Sets whether the system appearance is followed or overridden.
    ///
    /// [`ThemeMode::System`] is the default. An explicit mode pins the
    /// appearance and stops the platform from being consulted at all — which is
    /// what an application offering its own theme switch wants.
    #[inline]
    pub fn mode(mut self, mode: ThemeMode) -> Self {
        self.selection.mode = mode;
        self
    }

    /// Sets how long a theme transition lasts.
    ///
    /// A zero duration disables interpolation and publishes the target theme
    /// immediately.
    #[inline]
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Sets the curve used to transform the transition's linear progress.
    #[inline]
    pub fn curve(mut self, curve: impl Curve) -> Self {
        self.curve = curve.into_any();
        self
    }

    /// Attaches the descendant widget subtree and produces a valid widget.
    ///
    /// The subtree is stored as a [`ChildBuilder`], so the theme can build it
    /// again on every tick of a transition without the caller having to describe
    /// it more than once.
    #[inline]
    pub fn child<C: Widget + 'static>(self, child: C) -> AnimatedTheme<C, T> {
        AnimatedTheme {
            selection: self.selection,
            duration: self.duration,
            curve: self.curve,
            child: ChildBuilder::from_widget(child),
            marker: PhantomData,
        }
    }

    /// Attaches the descendant subtree and type-erases the completed theme
    /// widget.
    ///
    /// This is equivalent to calling [`AnimatedTheme::child`] followed by
    /// [`Widget::boxed`]. Use it when different code paths must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget
    where
        T: Theme,
    {
        self.child(child).boxed()
    }
}

/// The theme an [`AnimatedTheme`] settled on, and the animation that carries it.
///
/// Resolving the appearance and animating the result are two jobs: this is the
/// second one. Splitting them is what lets a system switch reach the animation
/// the same way an application's own theme change does — as a new configuration
/// of this widget, which retargets the running transition instead of replacing
/// the subtree.
struct ResolvedTheme<T> {
    data: T,
    duration: Duration,
    curve: AnyCurve,
    child: ChildBuilder,
    key: Option<Key>,
}

impl<T: Theme> Widget for ResolvedTheme<T> {
    fn key(&self) -> Option<Key> {
        self.key.clone()
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let __key = Widget::key(&self);
        StatefulElement::new_with_name(self, ctx, "AnimatedTheme", __key)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedTheme"
    }
}

impl<T: Theme> aimer_widget::PortableWidget for ResolvedTheme<T> {}

#[derive(Clone, Debug)]
struct ThemeTransition<T> {
    begin: T,
    end: T,
}

impl<T: Theme> ThemeTransition<T> {
    fn new(value: T) -> Self {
        Self {
            begin: value.clone(),
            end: value,
        }
    }

    fn sample(&self, progress: f32) -> T {
        self.begin.lerp(&self.end, progress)
    }

    fn retarget(&mut self, target: T, progress: f32) -> bool {
        if self.end == target {
            return false;
        }
        self.begin = self.sample(progress);
        self.end = target;
        true
    }
}

#[doc(hidden)]
pub struct AnimatedThemeState<T: Theme> {
    target: T,
    current: Rc<RefCell<T>>,
    duration: Duration,
    curve: AnyCurve,
    child: ChildBuilder,
    controller: AnimationController,
    transition: Rc<RefCell<ThemeTransition<T>>>,
    handle: ProviderHandle<T>,
}

impl<T: Theme> StatefulWidget for ResolvedTheme<T> {
    type State = AnimatedThemeState<T>;

    fn create_state(self) -> Self::State {
        AnimatedThemeState {
            target: self.data.clone(),
            current: Rc::new(RefCell::new(self.data.clone())),
            duration: self.duration,
            controller: AnimationController::new(self.duration, self.curve.clone()),
            curve: self.curve,
            child: self.child.clone(),
            transition: Rc::new(RefCell::new(ThemeTransition::new(self.data.clone()))),
            handle: ProviderHandle::new(self.data.clone()),
        }
    }
}

impl<T: Theme> State<ResolvedTheme<T>> for AnimatedThemeState<T> {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        self.duration = new.duration;
        self.curve = new.curve;
        self.child = new.child;
        self.controller.set_duration(self.duration);
        self.controller.set_curve(self.curve.clone());

        let new_target = new.target;
        if !self
            .transition
            .borrow_mut()
            .retarget(new_target.clone(), self.controller.value())
        {
            return;
        }

        self.target = new_target;
        self.controller.reset();
        if self.duration.is_zero() {
            self.controller.set_value(1.0);
            self.publish(self.target.clone());
        } else {
            *self.current.borrow_mut() = self.transition.borrow().sample(0.0);
            self.controller.forward_from_first_tick();
        }
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        AnimatedThemeFrame {
            current: self.current.clone(),
            child: self.child.clone(),
            controller: self.controller.clone(),
            transition: self.transition.clone(),
            handle: self.handle.clone(),
        }
    }
}

impl<T: Theme> AnimatedThemeState<T> {
    fn publish(&self, value: T) {
        if *self.current.borrow() != value {
            *self.current.borrow_mut() = value.clone();
            self.handle.update(|theme| *theme = value);
        }
    }
}

impl<W: Widget + 'static, T: Theme> AnimatedTheme<W, T> {
    /// Reads the appearance to resolve against, subscribing to it only when it
    /// can change the answer.
    ///
    /// A theme that overrides the system resolves the same way whatever the
    /// platform reports, so registering it as a follower would buy a rebuild per
    /// system switch and change nothing. One that follows reads the appearance
    /// from inside the build, so a switch marks that build — and nothing else —
    /// for rebuild.
    #[inline]
    fn system_appearance(&self, ctx: &BuildContext) -> Brightness {
        if self.selection.follows_system() {
            ctx.watch_platform_brightness()
        } else {
            platform_brightness()
        }
    }

    /// Lowers the builder into the widget that animates one settled theme.
    #[inline]
    fn resolved(&self, brightness: Brightness, key: Option<Key>) -> ResolvedTheme<T> {
        ResolvedTheme {
            data: self.selection.resolve(brightness),
            duration: self.duration,
            curve: self.curve.clone(),
            // `W: Widget` on this impl is only satisfiable through `child`, so a
            // theme that reaches this point always carries a subtree.
            child: self.child.clone(),
            key,
        }
    }
}

impl<W: Widget + 'static, T: Theme> Widget for AnimatedTheme<W, T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        // Resolving the appearance always happens in this one place, whether the
        // system is followed or overridden. Both answers therefore hang under
        // the same element, and an application that flips between them keeps the
        // animation running underneath instead of being rebuilt from scratch —
        // turning "follow the system" back on is a theme change, and a theme
        // change animates.
        let theme = self.clone();
        StatelessElement::from_builder(
            ctx,
            move |ctx| {
                theme
                    .resolved(theme.system_appearance(ctx), None)
                    .to_element(ctx)
            },
            self.key(),
            "AdaptiveTheme",
        )
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "AdaptiveTheme"
    }
}

#[cfg(not(feature = "portable-guest"))]
impl<W: Widget + 'static, T: Theme> aimer_widget::PortableWidget for AnimatedTheme<W, T> {}

#[cfg(feature = "portable-guest")]
impl<W: Widget + 'static, T: Theme> aimer_widget::PortableWidget for AnimatedTheme<W, T> {
    fn to_portable_node(
        self,
        ctx: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<aimer_widget::portable::PortableNodeId, aimer_widget::portable::PortableBuildError>
    {
        let build_context = ctx.build_context();
        let brightness = self.system_appearance(&build_context);
        let data = self.selection.resolve(brightness);
        let Some(codec) = T::portable_codec() else {
            return Err(
                aimer_widget::portable::PortableBuildError::ProviderEncoding {
                    provider: "AnimatedTheme",
                    value: type_name::<T>(),
                    source,
                    message: "custom themes must provide an explicit portable codec".to_owned(),
                },
            );
        };
        let bytes = codec.encode(&data).map_err(|error| {
            aimer_widget::portable::PortableBuildError::ProviderEncoding {
                provider: "AnimatedTheme",
                value: type_name::<T>(),
                source,
                message: error.to_string(),
            }
        })?;
        if bytes.len() > codec.schema().maximum_encoded_bytes() as usize {
            return Err(
                aimer_widget::portable::PortableBuildError::ProviderEncoding {
                    provider: "AnimatedTheme",
                    value: type_name::<T>(),
                    source,
                    message: format!(
                        "encoded theme is {} bytes, above the {} byte limit",
                        bytes.len(),
                        codec.schema().maximum_encoded_bytes(),
                    ),
                },
            );
        }
        let value = ctx.push_owned_blob(bytes)?;
        let duration = i64::try_from(self.duration.as_millis()).map_err(|_| {
            aimer_widget::portable::PortableBuildError::ProviderEncoding {
                provider: "AnimatedTheme",
                value: type_name::<T>(),
                source,
                message: "animation duration does not fit the portable i64 field".to_owned(),
            }
        })?;
        let (curve, control_points) = portable_curve(&self.curve).ok_or_else(|| {
            aimer_widget::portable::PortableBuildError::ProviderEncoding {
                provider: "AnimatedTheme",
                value: type_name::<T>(),
                source,
                message: "custom curves cannot be lowered to a portable guest; use a standard curve or a cubic Bézier".to_owned(),
            }
        })?;
        let mut properties = vec![
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_TYPE,
                PropertyValue::I64(codec.schema().id().value() as i64),
            ),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_SCHEMA_VERSION,
                PropertyValue::I64(pack_version(codec.schema().version())),
            ),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_VALUE, value),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_MODE,
                PropertyValue::I64(portable_theme_mode(self.selection.mode)),
            ),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_DURATION_MILLIS,
                PropertyValue::I64(duration),
            ),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE, PropertyValue::I64(curve)),
        ];
        if let Some([x1, y1, x2, y2]) = control_points {
            properties.extend([
                WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE_X1, PropertyValue::F64(x1))
                    .optional(),
                WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE_Y1, PropertyValue::F64(y1))
                    .optional(),
                WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE_X2, PropertyValue::F64(x2))
                    .optional(),
                WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE_Y2, PropertyValue::F64(y2))
                    .optional(),
            ]);
        }
        let child = with_portable_provider(ctx, ProviderHandle::new(data), |ctx| {
            self.child.into_portable_node(ctx, source.child(0))
        })?;
        let key = Key::fixed(source.identity().to_bytes());
        ctx.push_node(
            WIDGET_ANIMATED_THEME,
            BUILTIN_WIDGET_SCHEMA_VERSION,
            Some(&key),
            source,
            &properties,
            &[child],
        )
    }
}

#[cfg(feature = "portable-guest")]
fn pack_version(version: aimer_widget::portable::__anteros::Version) -> i64 {
    (((version.major() as u64) << 32) | version.minor() as u64) as i64
}

#[cfg(feature = "portable-guest")]
fn portable_theme_mode(mode: ThemeMode) -> i64 {
    match mode {
        ThemeMode::System => 0,
        ThemeMode::Light => 1,
        ThemeMode::Dark => 2,
    }
}

/// The portable tag and control points of `curve`, or `None` for a custom
/// curve, which only exists natively.
#[cfg(feature = "portable-guest")]
fn portable_curve(curve: &AnyCurve) -> Option<(i64, Option<[f64; 4]>)> {
    let descriptor = curve.descriptor()?;
    Some((descriptor.tag, descriptor.control_points))
}

struct AnimatedThemeFrame<T: Theme> {
    current: Rc<RefCell<T>>,
    child: ChildBuilder,
    controller: AnimationController,
    transition: Rc<RefCell<ThemeTransition<T>>>,
    handle: ProviderHandle<T>,
}

impl<T: Theme> Widget for AnimatedThemeFrame<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = Provider::new()
            .handle(self.handle.clone())
            .child(self.child.clone())
            .to_element(ctx);
        AnimatedThemeElement {
            current: self.current.clone(),
            child,
            controller: self.controller.clone(),
            transition: self.transition.clone(),
            handle: self.handle.clone(),
        }
        .boxed()
    }
}

impl<T: Theme> aimer_widget::PortableWidget for AnimatedThemeFrame<T> {}

struct AnimatedThemeElement<T: Theme> {
    current: Rc<RefCell<T>>,
    child: AnyElement,
    controller: AnimationController,
    transition: Rc<RefCell<ThemeTransition<T>>>,
    handle: ProviderHandle<T>,
}

impl<T: Theme> VisitorElement for AnimatedThemeElement<T> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedTheme"
    }
}

impl<T: Theme> Drawable for AnimatedThemeElement<T> {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);

        if self.controller.is_animating() {
            request_next_frame();
        }
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.child.update(ctx);
        if self.controller.is_animating() {
            request_next_frame();
        }
    }
}

impl<T: Theme> EventElement for AnimatedThemeElement<T> {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl<T: Theme> Rebuildable for AnimatedThemeElement<T> {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        let progress = self.controller.tick(AnimInstant::now());
        let value = self.transition.borrow().sample(progress);
        if *self.current.borrow() != value {
            *self.current.borrow_mut() = value.clone();
            self.handle.update(|theme| *theme = value);
        }
        if self.controller.is_animating() {
            // Nothing marks this element dirty while the transition runs, and a
            // tick that changes nothing visible publishes nothing, so the next
            // pass would skip it and the transition would stall.
            aimer_widget::keep_ticking_elements_reachable();
        }
        self.child.rebuild_if_dirty(ctx);
    }

    fn is_carry_state(&self) -> bool {
        true
    }

    fn mark_needs_rebuild(&self) {
        self.child.mark_needs_rebuild();
    }
}

impl<T: Theme> LayoutElement for AnimatedThemeElement<T> {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use aimer_animation::Animatable;
    use aimer_color::prelude::Color;
    use aimer_widget::ErrorWidget;
    use aimer_widget::base::WindowHandle;

    use super::*;
    use aimer_animation::curves;

    #[derive(Clone, Debug, PartialEq)]
    struct CustomTheme {
        value: f32,
    }

    impl Animatable for CustomTheme {
        fn lerp(&self, other: &Self, t: f32) -> Self {
            if t <= 0.0 {
                return self.clone();
            }
            if t >= 1.0 {
                return other.clone();
            }
            Self {
                value: self.value.lerp(&other.value, t),
            }
        }
    }

    impl crate::Theme for CustomTheme {}

    struct TestWidget;

    impl Widget for TestWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            panic!("not needed for state lifecycle tests")
        }
    }

    impl aimer_widget::PortableWidget for TestWidget {}

    #[cfg(feature = "portable-guest")]
    struct PortableLeaf;

    #[cfg(feature = "portable-guest")]
    impl Widget for PortableLeaf {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            panic!("portable test leaf is not materialized")
        }
    }

    #[cfg(feature = "portable-guest")]
    impl aimer_widget::PortableWidget for PortableLeaf {
        fn to_portable_node(
            self,
            ctx: &mut aimer_widget::portable::PortableBuildContext,
            source: aimer_widget::portable::SourceFingerprint,
        ) -> Result<
            aimer_widget::portable::PortableNodeId,
            aimer_widget::portable::PortableBuildError,
        > {
            let context = ctx.build_context();
            assert_eq!(
                *<ThemeData as Theme>::read(&context),
                ThemeData::dark(),
                "AnimatedTheme must install its resolved value while lowering its child",
            );
            ctx.push_node(
                aimer_widget::portable::__anteros::WIDGET_SIZED_BOX,
                aimer_widget::portable::__anteros::BUILTIN_WIDGET_SCHEMA_VERSION,
                None,
                source,
                &[],
                &[],
            )
        }
    }

    #[cfg(feature = "portable-guest")]
    fn portable_context() -> aimer_widget::portable::PortableBuildContext {
        aimer_widget::portable::PortableBuildContext::new(
            1,
            1,
            aimer_widget::portable::PortableWidgetLimits::new(8, 32, 16, 16, 64, 2_048)
                .with_max_blob_bytes(128),
            aimer_widget::portable::PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap()
    }

    fn theme(value: u8) -> ThemeData {
        ThemeData::new().primary_color(Color::Rgba(value, value, value, 255))
    }

    fn widget(data: ThemeData, duration: Duration) -> ResolvedTheme<ThemeData> {
        AnimatedTheme::new()
            .data(data)
            .duration(duration)
            .child(TestWidget)
            .resolved(Brightness::Light, None)
    }

    fn custom_widget(data: CustomTheme, duration: Duration) -> ResolvedTheme<CustomTheme> {
        AnimatedTheme::new()
            .data(data)
            .duration(duration)
            .child(TestWidget)
            .resolved(Brightness::Light, None)
    }

    #[test]
    fn a_fresh_theme_follows_the_system_appearance() {
        let widget = AnimatedTheme::new().child(TestWidget);

        assert_eq!(
            widget.resolved(Brightness::Light, None).data,
            ThemeData::light()
        );
        assert_eq!(
            widget.resolved(Brightness::Dark, None).data,
            ThemeData::dark()
        );
    }

    #[test]
    fn a_custom_pair_follows_the_system_appearance() {
        let widget = AnimatedTheme::new()
            .adaptive(CustomTheme { value: 1.0 }, CustomTheme { value: 2.0 })
            .child(TestWidget);

        assert_eq!(
            widget.resolved(Brightness::Dark, None).data,
            CustomTheme { value: 2.0 }
        );
        assert_eq!(
            widget.resolved(Brightness::Light, None).data,
            CustomTheme { value: 1.0 }
        );
    }

    #[test]
    fn a_single_theme_ignores_the_system_appearance() {
        let widget = AnimatedTheme::new().data(theme(7)).child(TestWidget);

        assert_eq!(widget.resolved(Brightness::Dark, None).data, theme(7));
    }

    #[test]
    fn an_explicit_mode_ignores_the_system_appearance() {
        let light_only = AnimatedTheme::new()
            .mode(ThemeMode::Light)
            .child(TestWidget);
        let dark_only = AnimatedTheme::new().mode(ThemeMode::Dark).child(TestWidget);

        assert_eq!(
            light_only.resolved(Brightness::Dark, None).data,
            ThemeData::light()
        );
        assert_eq!(
            dark_only.resolved(Brightness::Light, None).data,
            ThemeData::dark()
        );
    }

    #[test]
    fn only_a_theme_following_the_system_watches_the_platform() {
        let following = AnimatedTheme::new().child(TestWidget);
        let pinned = AnimatedTheme::new().mode(ThemeMode::Dark).child(TestWidget);
        let single = AnimatedTheme::new().data(theme(7)).child(TestWidget);

        assert!(following.selection.follows_system());
        assert!(!pinned.selection.follows_system());
        assert!(!single.selection.follows_system());
    }

    #[test]
    fn a_dark_counterpart_restores_following_after_a_single_theme() {
        let widget = AnimatedTheme::new()
            .data(theme(7))
            .dark(theme(9))
            .child(TestWidget);

        assert!(widget.selection.follows_system());
        assert_eq!(widget.resolved(Brightness::Dark, None).data, theme(9));
        assert_eq!(widget.resolved(Brightness::Light, None).data, theme(7));
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn animated_theme_portable_lowering_emits_value_mode_animation_and_child() {
        let mut context = portable_context();
        let root = aimer_widget::PortableWidget::to_portable_node(
            AnimatedTheme::new()
                .mode(ThemeMode::Dark)
                .duration(Duration::from_millis(321))
                .curve(curves::cubic_bezier(0.1, 0.2, 0.8, 0.9))
                .child(PortableLeaf),
            &mut context,
            aimer_widget::portable::SourceFingerprint::new(
                aimer_widget::portable::StableId128::from_bytes([9; 16]),
            ),
        )
        .unwrap();
        let document = context.finish_document(root).unwrap();
        let limits = document.model_limits();
        let image = document.encode().unwrap();
        let view =
            aimer_widget::portable::__anteros::WidgetDocumentView::decode(&image, limits).unwrap();
        let node = view.node(view.root_node()).unwrap();

        assert_eq!(
            node.widget_type(),
            aimer_widget::portable::__anteros::WIDGET_ANIMATED_THEME
        );
        assert_eq!(node.children().count(), 1);
        assert!(node.properties().any(|property| {
            property.property_id()
                == aimer_widget::portable::__anteros::PROPERTY_ANIMATED_THEME_MODE
                && property.value() == aimer_widget::portable::__anteros::PropertyValue::I64(2)
        }));
        assert!(node.properties().any(|property| {
            property.property_id()
                == aimer_widget::portable::__anteros::PROPERTY_ANIMATED_THEME_DURATION_MILLIS
                && property.value() == aimer_widget::portable::__anteros::PropertyValue::I64(321)
        }));
        assert_eq!(view.blob(0).map(<[u8]>::len), Some(24));
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn animated_theme_exposes_derived_schema_for_one_child() {
        use aimer_widget::portable::PortableWidgetSchema;

        let schema = <AnimatedTheme<PortableLeaf, ThemeData> as PortableWidgetSchema>::SCHEMA;
        assert_eq!(schema.widget().id(), WIDGET_ANIMATED_THEME);
        assert_eq!(schema.children().minimum(), 1);
        assert_eq!(schema.children().maximum(), 1);
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn custom_theme_without_a_codec_is_rejected_at_the_guest_source() {
        let mut context = portable_context();
        let error = aimer_widget::PortableWidget::to_portable_node(
            AnimatedTheme::new()
                .data(CustomTheme { value: 1.0 })
                .child(PortableLeaf),
            &mut context,
            aimer_widget::portable::SourceFingerprint::new(
                aimer_widget::portable::StableId128::from_bytes([10; 16]),
            ),
        )
        .expect_err("custom themes need an explicit portable codec");

        assert!(matches!(
            error,
            aimer_widget::portable::PortableBuildError::ProviderEncoding { .. }
        ));
    }

    #[test]
    fn unchanged_target_does_not_restart() {
        let data = theme(10);
        let mut transition = ThemeTransition::new(data);

        assert!(!transition.retarget(data, 0.5));
        assert_eq!(transition.sample(0.0), data);
    }

    #[test]
    fn interrupted_transition_starts_from_displayed_value() {
        let mut transition = ThemeTransition::new(theme(0));
        assert!(transition.retarget(theme(100), 0.0));

        assert!(transition.retarget(theme(200), 0.5));

        assert_eq!(
            transition.sample(0.0).primary_color,
            Color::Rgba(50, 50, 50, 255)
        );
        assert_eq!(
            transition.sample(1.0).primary_color,
            Color::Rgba(200, 200, 200, 255)
        );
    }

    #[test]
    fn custom_non_copy_theme_retargets_from_displayed_value() {
        let mut transition = ThemeTransition::new(CustomTheme { value: 0.0 });
        assert!(transition.retarget(CustomTheme { value: 100.0 }, 0.0));

        assert!(transition.retarget(CustomTheme { value: 200.0 }, 0.5));

        assert_eq!(transition.sample(0.0), CustomTheme { value: 50.0 });
        assert_eq!(transition.sample(1.0), CustomTheme { value: 200.0 });
    }

    #[test]
    fn zero_duration_uses_exact_target() {
        let mut state = widget(theme(0), Duration::from_millis(200)).create_state();
        let new_state = widget(theme(101), Duration::ZERO).create_state();

        <AnimatedThemeState<ThemeData> as State<ResolvedTheme<ThemeData>>>::adopt_config_from(
            &mut state, new_state,
        );

        assert_eq!(*state.handle.read(), theme(101));
        assert_eq!(*state.current.borrow(), theme(101));
        assert!(!state.controller.is_animating());
    }

    #[test]
    fn custom_non_copy_theme_zero_duration_publishes_exact_target() {
        let mut state =
            custom_widget(CustomTheme { value: 0.0 }, Duration::from_millis(200)).create_state();
        let new_state = custom_widget(CustomTheme { value: 101.0 }, Duration::ZERO).create_state();

        <AnimatedThemeState<CustomTheme> as State<ResolvedTheme<CustomTheme>>>::adopt_config_from(
            &mut state, new_state,
        );

        assert_eq!(*state.handle.read(), CustomTheme { value: 101.0 });
        assert_eq!(*state.current.borrow(), CustomTheme { value: 101.0 });
        assert!(!state.controller.is_animating());
    }

    #[test]
    fn changed_theme_starts_the_controller() {
        let mut state = widget(theme(0), Duration::from_millis(200)).create_state();
        let new_state = widget(theme(101), Duration::from_millis(400)).create_state();

        <AnimatedThemeState<ThemeData> as State<ResolvedTheme<ThemeData>>>::adopt_config_from(
            &mut state, new_state,
        );

        assert_eq!(state.controller.duration(), Duration::from_millis(400));
        assert!(state.controller.is_animating());
        assert_eq!(*state.current.borrow(), theme(0));
    }

    /// A child that rebuilds itself, the way a derived widget does.
    ///
    /// Conversions and builds are counted apart, because a theme tick reuses the
    /// child's element — so its state and GPU resources survive the transition —
    /// and asks the subtree inside it to build again, which is how the
    /// interpolated theme reaches it.
    struct Probe {
        conversions: Rc<Cell<usize>>,
        builds: Rc<Cell<usize>>,
    }

    impl Widget for Probe {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            self.conversions.set(self.conversions.get() + 1);
            let builds = self.builds;
            aimer_widget::Element::boxed(aimer_widget::StatelessElement::from_builder(
                ctx,
                move |ctx| {
                    builds.set(builds.get() + 1);
                    ErrorWidget::new("probe").to_element(ctx)
                },
                Some(Key::Static("probe")),
                "Probe",
            ))
        }

        fn key(&self) -> Option<Key> {
            Some(Key::Static("probe"))
        }

        fn debug_name(&self) -> &'static str {
            "Probe"
        }
    }

    impl aimer_widget::PortableWidget for Probe {}

    fn context() -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        BuildContext::new(
            canvas,
            ResolvedSize::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        )
    }

    #[tokio::test]
    async fn animated_theme_element_is_transparent_to_event_routing() {
        let ctx = context();
        let value = theme(42);
        let current = Rc::new(RefCell::new(value.clone()));
        let element = AnimatedThemeElement {
            current,
            child: Probe {
                conversions: Rc::new(Cell::new(0)),
                builds: Rc::new(Cell::new(0)),
            }
            .to_element(&ctx),
            controller: AnimationController::new(Duration::ZERO, curves::LINEAR),
            transition: Rc::new(RefCell::new(ThemeTransition::new(value.clone()))),
            handle: ProviderHandle::new(value),
        };

        assert_eq!(
            element.event_tree_role(),
            aimer_widget::EventTreeRole::Transparent
        );
    }

    fn frame_of(state: &AnimatedThemeState<ThemeData>, ctx: &BuildContext) {
        let frame =
            <AnimatedThemeState<ThemeData> as State<ResolvedTheme<ThemeData>>>::build(state, ctx)
                .to_element(ctx);
        // Drawing is the frame boundary: it ticks the controller, places the
        // retained child again, and rebuilds the marked subtree before paint.
        frame.update(ctx);
    }

    #[tokio::test]
    async fn a_light_dark_flip_animates_and_rebuilds_the_child_on_every_tick() {
        let conversions = Rc::new(Cell::new(0));
        let builds = Rc::new(Cell::new(0));
        let themed = AnimatedTheme::new()
            .adaptive(theme(0), theme(200))
            .duration(Duration::from_millis(200))
            .child(Probe {
                conversions: conversions.clone(),
                builds: builds.clone(),
            });
        let ctx = context();

        let mut state = themed.resolved(Brightness::Light, None).create_state();
        frame_of(&state, &ctx);
        assert_eq!(builds.get(), 1, "the first build reaches the child subtree");

        let flipped = themed.resolved(Brightness::Dark, None).create_state();
        <AnimatedThemeState<ThemeData> as State<ResolvedTheme<ThemeData>>>::adopt_config_from(
            &mut state, flipped,
        );

        assert!(
            state.controller.is_animating(),
            "a light/dark flip crosses into the other theme instead of snapping"
        );
        assert_eq!(
            *state.current.borrow(),
            theme(0),
            "the transition starts from the theme on screen"
        );

        for tick in 2..=4 {
            frame_of(&state, &ctx);
            assert_eq!(
                builds.get(),
                tick,
                "a theme tick rebuilds the child subtree instead of losing it"
            );
        }

        assert_eq!(
            conversions.get(),
            1,
            "the child's element is reused across the whole transition"
        );
    }

    #[test]
    fn active_transition_requests_next_frame_through_animation_scheduler() {
        let requests = Arc::new(AtomicUsize::new(0));
        let observed_requests = requests.clone();
        aimer_events::window::set_redraw_requester(move || {
            observed_requests.fetch_add(1, Ordering::Relaxed);
        });

        request_next_frame();

        assert_eq!(requests.load(Ordering::Relaxed), 1);
    }
}
