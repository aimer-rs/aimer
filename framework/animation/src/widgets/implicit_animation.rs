use std::cell::UnsafeCell;
use std::panic::Location;
use std::rc::Rc;
use std::time::Duration;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
#[cfg(feature = "portable-guest")]
use aimer_widget::AnyWidget;
use aimer_widget::base::*;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, Key, LayoutElement, Rebuildable,
    State, StateUpdater, StatefulElement, StatefulWidget, VisitorElement, Widget,
    carry_element_state,
};

use crate::control::controller::AnimationController;
use crate::local_cell::LocalCell;
use crate::primitives::animatable::Animatable;
use crate::primitives::curve::{AnyCurve, Curve};
use crate::primitives::time::AnimInstant;
use crate::primitives::tween::Tween;

type ImplicitElementBuilder<T> = dyn Fn(&T, &BuildContext) -> AnyElement;
#[cfg(feature = "portable-guest")]
type ImplicitPortableBuilder<T> = dyn Fn(&T) -> AnyWidget;

fn request_next_frame() {
    aimer_events::window::request_animation_frame();
}

#[cfg(feature = "portable-guest")]
struct PortableAnimationState<T: Animatable + Clone + PartialEq + 'static> {
    begin: T,
    target: T,
    duration: Duration,
    curve: AnyCurve,
    started_at: Option<AnimInstant>,
}

#[cfg(feature = "portable-guest")]
impl<T> PortableAnimationState<T>
where
    T: Animatable + Clone + PartialEq + 'static,
{
    fn new(value: T, duration: Duration, curve: AnyCurve) -> Self {
        Self {
            begin: value.clone(),
            target: value,
            duration,
            curve,
            started_at: None,
        }
    }

    fn value_at(&self, now: AnimInstant) -> T {
        let Some(started_at) = self.started_at else {
            return self.target.clone();
        };
        let progress = if self.duration.is_zero() {
            1.0
        } else {
            (now.duration_since(started_at).as_secs_f32() / self.duration.as_secs_f32()).min(1.0)
        };
        self.begin
            .lerp(&self.target, self.curve.transform(progress))
    }

    fn update(&mut self, target: T, duration: Duration, curve: AnyCurve) -> (T, bool) {
        self.update_at(target, duration, curve, AnimInstant::now())
    }

    fn update_at(
        &mut self,
        target: T,
        duration: Duration,
        curve: AnyCurve,
        now: AnimInstant,
    ) -> (T, bool) {
        if self.target != target {
            let current = self.value_at(now);
            self.begin = current;
            self.target = target;
            self.duration = duration;
            self.curve = curve;
            self.started_at = (!duration.is_zero()).then_some(now);
        } else {
            self.duration = duration;
            self.curve = curve;
            if duration.is_zero() {
                self.started_at = None;
            }
        }

        let value = self.value_at(now);
        let active = self.started_at.is_some();
        if active
            && now.duration_since(self.started_at.expect("active animation has a start time"))
                >= self.duration
        {
            self.begin = self.target.clone();
            self.started_at = None;
            return (self.target.clone(), false);
        }
        (value, active)
    }
}

/// A widget that automatically animates when its value changes.
///
/// On the first build, the value is used directly (no animation).
/// When the widget is rebuilt with a different value, a tween animation
/// runs from the currently displayed value to the new value over the specified
/// duration. Retargeting an animation therefore remains continuous. Rebuilding
/// with an equal value does not restart the controller.
///
/// # Example
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::{curves, ImplicitAnimatedBuilder};
/// use aimer_widget::ErrorWidget;
///
/// let animated = ImplicitAnimatedBuilder::new(
///     160.0_f32,
///     Duration::from_millis(300),
///     curves::LINEAR,
///     |width| ErrorWidget::new(format!("Width: {width:.0}")),
/// );
/// ```
pub struct ImplicitAnimatedBuilder<T: Animatable + Clone + PartialEq + 'static> {
    pub value: T,
    pub duration: Duration,
    pub curve: AnyCurve,
    builder: Rc<ImplicitElementBuilder<T>>,
    #[cfg(feature = "portable-guest")]
    portable_builder: Rc<ImplicitPortableBuilder<T>>,
    widget_key: Option<Key>,
}

impl<T> ImplicitAnimatedBuilder<T>
where
    T: Animatable + Clone + PartialEq + 'static,
{
    /// Creates an implicit animation for `value`.
    ///
    /// `T` must support interpolation through [`Animatable`]. The builder is
    /// called with the initial value immediately and with interpolated values
    /// during drawing. `duration` and `curve` are adopted on later rebuilds.
    pub fn new<F, W>(value: T, duration: Duration, curve: impl Curve, builder: F) -> Self
    where
        F: Fn(&T) -> W + 'static,
        W: Widget + 'static,
    {
        let builder = Rc::new(builder);
        let element_builder = {
            let builder = Rc::clone(&builder);
            Rc::new(move |value: &T, ctx: &BuildContext| builder(value).to_element(ctx))
        };
        #[cfg(feature = "portable-guest")]
        let portable_builder = {
            let builder = Rc::clone(&builder);
            Rc::new(move |value: &T| builder(value).boxed())
        };
        Self {
            value,
            duration,
            curve: curve.into_any(),
            builder: element_builder,
            #[cfg(feature = "portable-guest")]
            portable_builder,
            widget_key: None,
        }
    }

    /// Sets the identity of the animated builder for widget reconciliation.
    #[track_caller]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        let location = Location::caller();
        self.widget_key = Some(key.into().with_location(location));
        self
    }
}

impl<T> StatefulWidget for ImplicitAnimatedBuilder<T>
where
    T: Animatable + Clone + PartialEq + 'static,
{
    type State = ImplicitAnimatedState<T>;

    fn create_state(self) -> Self::State {
        ImplicitAnimatedState {
            target: self.value.clone(),
            current: Rc::new(LocalCell::new(self.value.clone())),
            duration: self.duration,
            controller: AnimationController::new(self.duration, self.curve.clone()),
            curve: self.curve,
            builder: self.builder.clone(),
            tween: Rc::new(LocalCell::new(None)),
            updater: StateUpdater::empty(),
        }
    }
}

impl<T> Widget for ImplicitAnimatedBuilder<T>
where
    T: Animatable + Clone + PartialEq + 'static,
{
    fn key(&self) -> Option<Key> {
        self.widget_key.clone()
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let __key = Widget::key(&self);
        StatefulElement::new_with_name(self, ctx, "ImplicitAnimatedBuilder", __key)
            .0
            .boxed()
    }
}

impl<T> aimer_widget::PortableWidget for ImplicitAnimatedBuilder<T>
where
    T: Animatable + Clone + PartialEq + 'static,
{
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        ctx: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<aimer_widget::portable::PortableNodeId, aimer_widget::portable::PortableBuildError>
    {
        let slot = ctx.slot_for(self.widget_key.as_ref(), source);
        let value = self.value;
        let duration = self.duration;
        let curve = self.curve;
        let portable_builder = self.portable_builder;
        ctx.with_animation_state(
            slot,
            || PortableAnimationState::new(value.clone(), duration, curve.clone()),
            |state, ctx| {
                let (value, active) = state.update(value.clone(), duration, curve.clone());
                if active {
                    ctx.request_frame();
                }
                let child = portable_builder(&value);
                aimer_widget::PortableWidget::to_portable_node(child, ctx, source)
            },
        )
    }
}

#[doc(hidden)]
pub struct ImplicitAnimatedState<T: Animatable + Clone + PartialEq + 'static> {
    target: T,
    current: Rc<LocalCell<T>>,
    duration: Duration,
    curve: AnyCurve,
    builder: Rc<ImplicitElementBuilder<T>>,
    controller: AnimationController,
    tween: Rc<LocalCell<Option<Tween<T>>>>,
    updater: StateUpdater<Self>,
}

impl<T> State<ImplicitAnimatedBuilder<T>> for ImplicitAnimatedState<T>
where
    T: Animatable + Clone + PartialEq + 'static,
{
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.duration = new.duration;
        self.curve = new.curve;
        self.builder = new.builder;
        self.controller.set_duration(self.duration);
        self.controller.set_curve(self.curve.clone());

        if self.target != new.target {
            let new_target = new.target;
            let current = self.tween.with(|tween| {
                tween
                    .as_ref()
                    .map(|tween| tween.lerp(self.controller.value()))
                    .unwrap_or_else(|| self.current.with(Clone::clone))
            });
            self.current.with_mut(|value| *value = current.clone());
            self.tween
                .with_mut(|tween| *tween = Some(Tween::new(current, new_target.clone())));
            self.target = new_target;
            self.controller.reset();
            self.controller.forward_from_first_tick();
            request_next_frame();
        }
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        ImplicitAnimatedFrame {
            current: self.current.clone(),
            target: self.target.clone(),
            builder: self.builder.clone(),
            controller: self.controller.clone(),
            tween: self.tween.clone(),
        }
    }
}

struct ImplicitAnimatedFrame<T: Animatable + Clone + PartialEq + 'static> {
    current: Rc<LocalCell<T>>,
    target: T,
    builder: Rc<ImplicitElementBuilder<T>>,
    controller: AnimationController,
    tween: Rc<LocalCell<Option<Tween<T>>>>,
}

impl<T: Animatable + Clone + PartialEq + 'static> Widget for ImplicitAnimatedFrame<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let value = self.current.with(Clone::clone);
        let child = (self.builder)(&value, ctx);
        ImplicitAnimatedElement {
            child: UnsafeCell::new(child),
            current: self.current.clone(),
            target: self.target.clone(),
            builder: self.builder.clone(),
            controller: self.controller.clone(),
            tween: self.tween.clone(),
            reachable: aimer_widget::KeepReachable::new(),
        }
        .boxed()
    }
}

impl<T: Animatable + Clone + PartialEq + 'static> aimer_widget::PortableWidget
    for ImplicitAnimatedFrame<T>
{
}

struct ImplicitAnimatedElement<T: Animatable + Clone + PartialEq + 'static> {
    child: UnsafeCell<AnyElement>,
    current: Rc<LocalCell<T>>,
    target: T,
    builder: Rc<ImplicitElementBuilder<T>>,
    controller: AnimationController,
    tween: Rc<LocalCell<Option<Tween<T>>>>,
    /// Keeps this element on the rebuild pass's dirty-path index while it
    /// animates. Nothing marks it dirty, so a pass that has indexed the tree
    /// would otherwise skip it and leave the animation where its first frame
    /// put it.
    reachable: aimer_widget::KeepReachable,
}

unsafe impl<T: Animatable + Clone + PartialEq + 'static> Send for ImplicitAnimatedElement<T> {}
unsafe impl<T: Animatable + Clone + PartialEq + 'static> Sync for ImplicitAnimatedElement<T> {}

impl<T: Animatable + Clone + PartialEq + 'static> ImplicitAnimatedElement<T> {
    /// Advances the animation one frame and publishes the child it names.
    ///
    /// This runs from `rebuild_if_dirty`, which the frame loop calls before the
    /// render tree synchronizes: a child replaced while drawing would have no
    /// render node until the next sync and would paint nothing in between.
    fn advance(&self, ctx: &BuildContext) -> bool {
        let progress = self.controller.tick(AnimInstant::now());
        let value = self.tween.with(|tween| {
            tween
                .as_ref()
                .map(|tween| tween.lerp(progress))
                .unwrap_or_else(|| self.current.with(Clone::clone))
        });

        // The builder is called fresh every frame so the subtree reflects the
        // interpolated value, but its element still owns runtime state an
        // ordinary rebuild would hand over — a hovered resize handle, a drag
        // in flight, a scroll offset. Carry it across so animating a value
        // does not also wipe everything nested below it.
        //
        // A settled animation produces the same value every frame, and the
        // child it already built is the child that value names: keep it
        // instead of rebuilding. Replacing the whole subtree would churn its
        // element identities — the ids the dispatcher's captured pointers and
        // focus records resolve through — so a press that spans a redraw the
        // animation did not even ask for would be dropped.
        let changed = self.current.with(|current| *current != value);
        if changed {
            self.current.with_mut(|current| *current = value.clone());
            let new_child = (self.builder)(&value, ctx);
            aimer_widget::preserving_dirty_paths(|| {
                carry_element_state(
                    unsafe { &*self.child.get() }.as_ref(),
                    new_child.as_ref(),
                    ctx,
                )
            });
            unsafe { *self.child.get() = new_child };
        }

        if self.controller.is_animating() {
            self.reachable.hold();
            request_next_frame();
        } else {
            self.reachable.release();
            self.current
                .with_mut(|current| *current = self.target.clone());
        }
        changed
    }
}

impl<T: Animatable + Clone + PartialEq + 'static> Drawable for ImplicitAnimatedElement<T> {
    fn update(&self, ctx: &BuildContext) {
        unsafe { &*self.child.get() }.update(ctx);
    }

    /// The wrapper records nothing itself: its rebuilt child owns its render
    /// nodes and resolves its own paint source, so the animation never forces
    /// the subtree onto the legacy path.
    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}
}

impl<T: Animatable + Clone + PartialEq + 'static> VisitorElement for ImplicitAnimatedElement<T> {
    fn debug_name(&self) -> &'static str {
        "ImplicitAnimatedElement"
    }

    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(unsafe { &*self.child.get() }.as_ref());
    }
}

impl<T: Animatable + Clone + PartialEq + 'static> EventElement for ImplicitAnimatedElement<T> {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        unsafe { &*self.child.get() }.on_event(event)
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(unsafe { &*self.child.get() }.as_ref());
    }
}

impl<T: Animatable + Clone + PartialEq + 'static> Rebuildable for ImplicitAnimatedElement<T> {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        if self.advance(ctx) {
            // A fresh subtree nothing has visited yet, which also registers it.
            aimer_widget::rebuild_replaced_subtree(unsafe { &*self.child.get() }.as_ref(), ctx);
        } else {
            unsafe { &*self.child.get() }.rebuild_if_dirty(ctx);
        }
    }
}

impl<T: Animatable + Clone + PartialEq + 'static> LayoutElement for ImplicitAnimatedElement<T> {
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
    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::primitives::curve::curves;
    use crate::widgets::test_frame_requester;

    struct TestWidget;

    struct TestElement;

    impl Drawable for TestElement {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl EventElement for TestElement {}

    impl LayoutElement for TestElement {}

    impl Rebuildable for TestElement {}

    impl VisitorElement for TestElement {
        fn debug_name(&self) -> &'static str {
            "TestElement"
        }
    }

    impl Widget for TestWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            TestElement.boxed()
        }
    }

    impl aimer_widget::PortableWidget for TestWidget {}

    /// A widget/element pair that records, on every `adopt_runtime_state_from`
    /// call, the identity of the element it replaced — the same hook
    /// `RawResizable` relies on to keep a hovered handle across a rebuild.
    struct RecordingWidget {
        id: u32,
        log: Rc<RefCell<Vec<u32>>>,
    }

    struct RecordingElement {
        id: u32,
        log: Rc<RefCell<Vec<u32>>>,
    }

    impl Widget for RecordingWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            RecordingElement {
                id: self.id,
                log: self.log,
            }
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for RecordingWidget {}

    impl Drawable for RecordingElement {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl EventElement for RecordingElement {}

    impl LayoutElement for RecordingElement {}

    impl VisitorElement for RecordingElement {
        fn debug_name(&self) -> &'static str {
            "RecordingElement"
        }
    }

    impl Rebuildable for RecordingElement {
        fn option_any(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }

        fn adopt_runtime_state_from(&self, old: &dyn Element) {
            if let Some(old) = old
                .option_any()
                .and_then(|value| value.downcast_ref::<Self>())
            {
                self.log.borrow_mut().push(old.id);
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dummy_async_handle() -> tokio::runtime::Handle {
        static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let _guard = runtime.enter();
        tokio::runtime::Handle::current()
    }

    fn dummy_build_context() -> BuildContext<'static> {
        let canvas = {
            let leaked: &'static aimer_canvas::InnerCanvas =
                Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(leaked)
        };

        BuildContext::new(
            canvas,
            Default::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            dummy_async_handle(),
        )
    }

    fn widget(value: f32) -> ImplicitAnimatedBuilder<f32> {
        ImplicitAnimatedBuilder::new(value, Duration::from_millis(100), curves::LINEAR, |_| {
            TestWidget
        })
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn portable_lowering_emits_the_current_built_child() {
        use aimer_widget::portable::__anteros::WidgetDocumentView;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, PortableWidgetSchema,
            SourceFingerprint,
        };
        use aimer_widget::{ErrorWidget, PortableWidget};

        let mut context = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048).with_max_blob_bytes(128),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let root = ImplicitAnimatedBuilder::new(
            1.0_f32,
            Duration::from_millis(100),
            curves::LINEAR,
            |_| ErrorWidget::new("child"),
        )
        .to_portable_node(
            &mut context,
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([7; 16])),
        )
        .unwrap();
        let document = context.finish_document(root).unwrap();
        let image = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&image, document.model_limits()).unwrap();

        assert_eq!(
            view.node(view.root_node()).unwrap().widget_type(),
            <ErrorWidget as PortableWidgetSchema>::SCHEMA.widget().id(),
        );
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn portable_target_changes_request_guest_animation_frames() {
        use aimer_widget::ErrorWidget;
        use aimer_widget::PortableWidget;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        };

        let mut context = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048).with_max_blob_bytes(128),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let source =
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([8; 16]));

        let first = ImplicitAnimatedBuilder::new(
            0.0_f32,
            Duration::from_millis(100),
            curves::LINEAR,
            |_| ErrorWidget::new("child"),
        )
        .to_portable_node(&mut context, source)
        .unwrap();
        context.finish_document(first).unwrap();

        let second = ImplicitAnimatedBuilder::new(
            1.0_f32,
            Duration::from_millis(100),
            curves::LINEAR,
            |_| ErrorWidget::new("child"),
        )
        .to_portable_node(&mut context, source)
        .unwrap();
        context.finish_document(second).unwrap();

        assert!(context.has_async_work());
        assert!(context.take_frame_request());
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn portable_animation_state_interpolates_at_frame_times() {
        let start = AnimInstant::now();
        let mut state =
            PortableAnimationState::new(0.0_f32, Duration::from_millis(100), curves::LINEAR.into_any());

        assert_eq!(
            state.update_at(0.0, Duration::from_millis(100), curves::LINEAR.into_any(), start),
            (0.0, false)
        );
        assert_eq!(
            state.update_at(1.0, Duration::from_millis(100), curves::LINEAR.into_any(), start),
            (0.0, true)
        );

        let (middle, active) = state.update_at(
            1.0,
            Duration::from_millis(100),
            curves::LINEAR.into_any(),
            start + Duration::from_millis(50),
        );
        assert!(active);
        assert!((middle - 0.5).abs() < 0.01);

        assert_eq!(
            state.update_at(
                1.0,
                Duration::from_millis(100),
                curves::LINEAR.into_any(),
                start + Duration::from_millis(100),
            ),
            (1.0, false)
        );
    }

    #[test]
    fn explicit_key_sets_reconciliation_identity() {
        let animated = widget(1.0).key("implicit-animation");

        assert_eq!(
            Widget::key(&animated),
            Some(Key::Value("implicit-animation".to_owned()))
        );
    }

    #[test]
    #[cfg(not(target_os = "ios"))]
    fn active_animation_defers_its_next_frame_request() {
        test_frame_requester::install();
        test_frame_requester::reset();
        let ctx = dummy_build_context();
        let controller = AnimationController::with_millis(100, curves::LINEAR);
        controller.forward_from_first_tick();
        let element = ImplicitAnimatedElement {
            child: UnsafeCell::new(TestElement.boxed()),
            current: Rc::new(LocalCell::new(0.0)),
            target: 1.0,
            builder: Rc::new(|_, _| TestElement.boxed()),
            controller,
            tween: Rc::new(LocalCell::new(Some(Tween::new(0.0, 1.0)))),
            reachable: aimer_widget::KeepReachable::new(),
        };

        element.rebuild_if_dirty(&ctx);

        assert_eq!(test_frame_requester::count(), 1);
        assert!(!ctx.window.take_redraw_request());
    }

    // Regression: the child element used to be replaced outright on every
    // frame the animation redrew, discarding whatever runtime state it held —
    // a `Resizable` wrapped in an `ImplicitAnimatedBuilder` lost its hovered
    // resize handle the moment it was reported, since the very next frame
    // rebuilt it from scratch with a fresh `hovered: None`. The handle's own
    // `enter_zone` then saw no change on the way out and never reported
    // `Direction::NONE` again.
    #[test]
    fn advancing_carries_runtime_state_into_every_rebuilt_child() {
        let ctx = dummy_build_context();
        let log: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
        let next_id = Rc::new(Cell::new(0u32));

        let builder_log = log.clone();
        let builder_id = next_id.clone();
        let builder: Rc<ImplicitElementBuilder<f32>> =
            Rc::new(move |_value: &f32, ctx: &BuildContext| {
                let id = builder_id.get();
                builder_id.set(id + 1);
                RecordingWidget {
                    id,
                    log: builder_log.clone(),
                }
                .to_element(ctx)
            });

        let first_child = (builder)(&0.0, &ctx);
        let controller = AnimationController::with_millis(100, curves::LINEAR);
        controller.forward_from_first_tick();

        let element = ImplicitAnimatedElement {
            child: UnsafeCell::new(first_child),
            current: Rc::new(LocalCell::new(0.0)),
            target: 1.0,
            builder,
            controller,
            tween: Rc::new(LocalCell::new(Some(Tween::new(0.0, 1.0)))),
            reachable: aimer_widget::KeepReachable::new(),
        };

        // A frame whose interpolated value changed rebuilds the child (ids 1,
        // then 2), and each rebuild must hand its state over from the element
        // it replaces (ids 0, then 1) — the same hand-over a normal rebuild
        // performs for free. The first frame only starts the animation — the
        // controller's first tick is this one, so the value has not moved yet
        // and the child is kept — and a draw that still shows the same value
        // keeps the child it built, so the later frames are separated by
        // enough time for the animation to advance.
        element.rebuild_if_dirty(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
        element.rebuild_if_dirty(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
        element.rebuild_if_dirty(&ctx);

        assert_eq!(*log.borrow(), vec![0, 1]);
    }

    #[test]
    fn changed_target_starts_from_current_value() {
        let mut state = widget(2.0).create_state();
        let new_state = widget(10.0).create_state();

        state.adopt_config_from(new_state);

        state.tween.with(|tween| {
            let tween = tween.as_ref().unwrap();
            assert_eq!(tween.begin, 2.0);
            assert_eq!(tween.end, 10.0);
        });
        assert!(state.controller.is_animating());
    }

    #[test]
    fn interrupted_animation_retargets_from_sampled_value() {
        let mut state = widget(0.0).create_state();
        state.adopt_config_from(widget(10.0).create_state());
        state.controller.set_value(0.5);

        state.adopt_config_from(widget(20.0).create_state());

        state.tween.with(|tween| {
            let tween = tween.as_ref().unwrap();
            assert!((tween.begin - 5.0).abs() < f32::EPSILON);
            assert_eq!(tween.end, 20.0);
        });
    }

    #[test]
    fn unchanged_target_does_not_restart_animation() {
        let mut state = widget(3.0).create_state();

        state.adopt_config_from(widget(3.0).create_state());

        assert!(state.tween.with(Option::is_none));
        assert!(!state.controller.is_animating());
    }
}
