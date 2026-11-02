use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use aimer_animation::{AnimInstant, AnimationController, curves};
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd_v2::Rect;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement, Rebuildable, State,
    StateUpdater, StatefulElement, StatefulWidget, VisitorElement, Widget,
};

/// The local logical rectangle occupied by a caret.
///
/// Coordinates are relative to the text field's outer local origin. The
/// rectangle includes the field's outline and padding and uses logical pixels
/// rather than device pixels, so a custom caret can use it directly with normal
/// widget dimensions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CaretGeometry {
    /// Horizontal position of the caret's leading edge.
    pub x: f32,
    /// Vertical position of the caret's top edge.
    pub y: f32,
    /// Width reserved for the caret.
    pub width: f32,
    /// Height reserved for the caret.
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CaretSnapshot {
    geometry: CaretGeometry,
    offset: usize,
    focused: bool,
    available: bool,
    composing: bool,
    visible: bool,
}

/// Read-only state supplied to a custom text-field caret widget.
///
/// The context is backed by the mounted field, so a caret element can retain
/// this value and read the current geometry while the field scrolls, edits, or
/// changes focus. The context is intentionally separate from the text editing
/// controller: a caret can observe presentation state without being able to
/// mutate the field.
#[derive(Clone)]
pub struct CaretContext {
    snapshot: Rc<Cell<CaretSnapshot>>,
    blink: CaretBlink,
    repaint: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

impl CaretContext {
    #[inline]
    pub(crate) fn new(blink: CaretBlink) -> Self {
        Self {
            snapshot: Rc::new(Cell::new(CaretSnapshot::default())),
            blink,
            repaint: Rc::new(RefCell::new(None)),
        }
    }

    /// Returns the caret rectangle in text-field local coordinates.
    #[inline]
    pub fn geometry(&self) -> CaretGeometry {
        self.snapshot.get().geometry
    }

    /// Returns the current caret position in grapheme-cluster units.
    #[inline]
    pub fn offset(&self) -> usize {
        self.snapshot.get().offset
    }

    /// Returns whether the owning field currently has focus.
    #[inline]
    pub fn is_focused(&self) -> bool {
        self.snapshot.get().focused
    }

    /// Returns whether the field produced a caret geometry for the current
    /// frame.
    #[inline]
    pub fn is_available(&self) -> bool {
        self.snapshot.get().available
    }

    /// Returns whether the builtin caret should be shown.
    ///
    /// The builtin caret remains solid while an input method reports its
    /// composition caret; ordinary insertion carets follow the shared blink
    /// timeline.
    #[inline]
    pub fn is_visible(&self) -> bool {
        self.snapshot.get().visible
    }

    /// Returns whether the input method is currently replacing the caret with
    /// composing text.
    #[inline]
    pub fn is_composing(&self) -> bool {
        self.snapshot.get().composing
    }

    /// Returns the shared blink timeline used by the owning field.
    #[inline]
    pub fn blink(&self) -> &CaretBlink {
        &self.blink
    }

    pub(crate) fn set_repaint_callback(&self, callback: Rc<dyn Fn()>) {
        *self.repaint.borrow_mut() = Some(callback);
    }

    pub(crate) fn request_repaint(&self) {
        if let Some(callback) = self.repaint.borrow().as_ref() {
            callback();
        }
    }

    #[inline]
    pub(crate) fn publish(
        &self,
        geometry: CaretGeometry,
        offset: usize,
        focused: bool,
        available: bool,
        composing: bool,
    ) {
        let previous = self.snapshot.get();
        let visible = focused && available && (composing || self.blink.is_visible());
        self.snapshot.set(CaretSnapshot {
            geometry,
            offset,
            focused,
            available,
            composing,
            visible,
        });
        if previous.visible != visible
            || previous.geometry.width != geometry.width
            || previous.geometry.height != geometry.height
        {
            self.request_repaint();
        }
    }
}

/// A cloneable factory for a custom caret widget.
///
/// The factory is called when a text field mounts its visual caret. The
/// resulting widget is retained as a normal element, so stateful custom carets
/// are not recreated on every frame.
pub struct CaretBuilder(Rc<dyn Fn(CaretContext) -> AnyWidget>);

impl CaretBuilder {
    /// Creates a caret factory from a function that builds a widget for the
    /// field-provided context.
    #[inline]
    pub fn new<F, W>(builder: F) -> Self
    where
        F: Fn(CaretContext) -> W + 'static,
        W: Widget + 'static,
    {
        Self(Rc::new(move |context| builder(context).boxed()))
    }

    #[inline]
    pub(crate) fn build(&self, context: CaretContext) -> AnyWidget {
        (self.0)(context)
    }
}

impl Clone for CaretBuilder {
    #[inline]
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

/// The builtin vertical-bar caret used when a field has no custom builder.
///
/// A text field positions this widget at the current insertion point and
/// supplies its live [`CaretContext`]. The widget deliberately reads the
/// context during painting, so its retained element follows scrolling,
/// selection movement, focus, and blink changes without being rebuilt.
pub struct DefaultCaret {
    context: CaretContext,
    color: Rc<Cell<Color>>,
}

impl DefaultCaret {
    /// Creates the builtin caret for a field-provided context and color.
    #[inline]
    pub fn new(context: CaretContext, color: impl Into<Color>) -> Self {
        Self::with_color_cell(context, Rc::new(Cell::new(color.into())))
    }

    #[inline]
    pub(crate) fn with_color_cell(context: CaretContext, color: Rc<Cell<Color>>) -> Self {
        Self { context, color }
    }
}

impl aimer_widget::PortableWidget for DefaultCaret {}

impl Widget for DefaultCaret {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        DefaultCaretWidget {
            context: self.context,
            color: self.color,
        }
        .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "DefaultCaret"
    }
}

struct DefaultCaretWidget {
    context: CaretContext,
    color: Rc<Cell<Color>>,
}

impl aimer_widget::PortableWidget for DefaultCaretWidget {}

impl Widget for DefaultCaretWidget {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "DefaultCaret", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "DefaultCaret"
    }
}

struct DefaultCaretState {
    context: CaretContext,
    color: Rc<Cell<Color>>,
    updater: StateUpdater<Self>,
    revision: u64,
}

impl StatefulWidget for DefaultCaretWidget {
    type State = DefaultCaretState;

    fn create_state(self) -> Self::State {
        DefaultCaretState {
            context: self.context,
            color: self.color,
            updater: StateUpdater::empty(),
            revision: 0,
        }
    }
}

impl State<DefaultCaretWidget> for DefaultCaretState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
        let updater = self.updater;
        // The context outlives this state: the field that owns it keeps
        // publishing geometry while the caret element is replaced or torn down
        // (a page switch, or a frame's chrome rebuilt from scratch). A repaint
        // asked of a state that is gone has nothing to repaint, so it is dropped
        // rather than treated as a bug.
        self.context.set_repaint_callback(Rc::new(move || {
            let _ = updater.try_set_state(|state| {
                state.revision = state.revision.wrapping_add(1);
            });
        }));
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.context = new.context;
        self.color = new.color;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let _revision = self.revision;
        DefaultCaretPaint {
            context: self.context.clone(),
            color: Rc::clone(&self.color),
        }
    }
}

struct DefaultCaretPaint {
    context: CaretContext,
    color: Rc<Cell<Color>>,
}

impl aimer_widget::PortableWidget for DefaultCaretPaint {}

impl Widget for DefaultCaretPaint {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        DefaultCaretElement {
            context: self.context,
            color: self.color,
        }
        .boxed()
    }
}

struct DefaultCaretElement {
    context: CaretContext,
    color: Rc<Cell<Color>>,
}

impl Drawable for DefaultCaretElement {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && ctx.parent_size.width.is_finite()
            && ctx.parent_size.height.is_finite()
            && ctx.parent_size.width >= 0.0
            && ctx.parent_size.height >= 0.0
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = Canvas::of(ctx);
        if self.context.is_focused() && self.context.is_visible() {
            let (red, green, blue, alpha) = self.color.get().to_rgba();
            canvas.fill_rect(
                Rect::new(
                    0.0,
                    0.0,
                    ctx.parent_size.width / ctx.scale,
                    ctx.parent_size.height / ctx.scale,
                ),
                [red, green, blue, alpha],
            );
        }
        canvas.finish();
    }
}

impl VisitorElement for DefaultCaretElement {
    fn debug_name(&self) -> &'static str {
        "DefaultCaret"
    }
}

impl EventElement for DefaultCaretElement {}
impl LayoutElement for DefaultCaretElement {}
impl Rebuildable for DefaultCaretElement {}

/// The blink timeline of a text field caret.
///
/// A `CaretBlink` is a cheap, cloneable handle over a repeating
/// [`AnimationController`]. Every clone observes the same phase, so the state
/// of a stateful field can own the timeline while the element it builds reads
/// the caret visibility from it. Because the phase lives in the state, a
/// rebuild no longer restarts the blink.
///
/// The timeline advances when [`CaretBlink::tick`] is called with the current
/// frame time. On native targets, a mounted field schedules the next frame only
/// at a visibility transition; browser builds keep their frame-driven fallback.
/// The caret is opaque during the first half of the period and hidden during the
/// second half, which yields a `period / 2` on-off rhythm without repainting the
/// field between transitions.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use aimer_animation::AnimInstant;
/// use aimer_input::input::CaretBlink;
///
/// let blink = CaretBlink::new();
/// let start = AnimInstant::now();
///
/// blink.tick(start);
/// assert!(blink.is_visible());
///
/// blink.tick(start + CaretBlink::DEFAULT_PERIOD / 2);
/// assert!(!blink.is_visible());
/// ```
#[derive(Debug, Default)]
struct CaretWakeState {
    generation: Cell<u64>,
    scheduled: Cell<bool>,
}

#[derive(Clone, Debug)]
pub struct CaretBlink {
    controller: AnimationController,
    wake: Rc<CaretWakeState>,
}

impl CaretBlink {
    /// The period of a full on-off cycle, matching the platform convention of
    /// a caret that is shown for half a second and hidden for half a second.
    pub const DEFAULT_PERIOD: Duration = Duration::from_millis(1000);

    /// Creates a blink timeline using [`CaretBlink::DEFAULT_PERIOD`].
    ///
    /// The timeline starts at the beginning of its visible half and does not
    /// consume any of the period until the first [`CaretBlink::tick`], so time
    /// spent building widgets is never counted as blink time.
    #[inline]
    pub fn new() -> Self {
        Self::with_period(Self::DEFAULT_PERIOD)
    }

    /// Creates a blink timeline that completes one on-off cycle per `period`.
    ///
    /// A zero `period` keeps the caret permanently visible instead of dividing
    /// by zero, because the controller resolves a zero duration as an
    /// immediately wrapping cycle.
    #[inline]
    pub fn with_period(period: Duration) -> Self {
        let controller = AnimationController::new(period, curves::LINEAR);
        controller.set_repeat(true);
        controller.forward_from_first_tick();
        Self {
            controller,
            wake: Rc::new(CaretWakeState::default()),
        }
    }

    /// Returns the duration of one full on-off cycle.
    #[inline]
    pub fn period(&self) -> Duration {
        self.controller.duration()
    }

    /// Returns whether the caret should be painted at the current phase.
    #[inline]
    pub fn is_visible(&self) -> bool {
        self.controller.value() < 0.5
    }

    /// Advances the timeline to `now` and reports whether the caret changed
    /// visibility.
    ///
    /// Call this once per frame while the field owns focus. The phase is
    /// derived from the elapsed time, so a dropped frame delays a toggle by at
    /// most that frame instead of shifting the whole rhythm.
    pub fn tick(&self, now: AnimInstant) -> bool {
        let was_visible = self.is_visible();
        self.controller.tick(now);
        was_visible != self.is_visible()
    }

    /// Schedules the next visibility transition on the mounted field's scope.
    ///
    /// At most one timer is armed for a blink handle. The timer only requests a
    /// frame; the next draw remains responsible for advancing the timeline and
    /// painting the new state. A missing Venus runtime leaves the handle
    /// unarmed so isolated raw-field users can retain their frame-driven path.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn schedule_next_toggle(&self, scope: aimer_venus::ScopeId) {
        let delay =
            Duration::from_nanos((self.period().as_nanos() / 2).min(u64::MAX as u128) as u64);
        if delay.is_zero() || self.wake.scheduled.replace(true) {
            return;
        }

        let generation = self.wake.generation.get();
        let wake = Rc::clone(&self.wake);
        let Some(venus) = aimer_venus::Venus::current() else {
            self.wake.scheduled.set(false);
            return;
        };
        venus.spawn_in(scope, async move {
            tokio::time::sleep(delay).await;
            if wake.generation.get() == generation {
                wake.scheduled.set(false);
                aimer_events::window::request_animation_frame();
            }
        });
    }

    /// Invalidates a pending native wake, if any.
    pub(crate) fn cancel_scheduled_toggle(&self) {
        self.wake
            .generation
            .set(self.wake.generation.get().wrapping_add(1));
        self.wake.scheduled.set(false);
    }

    /// Restarts the timeline at the beginning of its visible half.
    ///
    /// Editing, moving the caret, or clicking into the field calls this so the
    /// caret stays solid while the user is busy, exactly like a native field.
    /// The new period begins at the next [`CaretBlink::tick`].
    pub fn reset(&self) {
        self.cancel_scheduled_toggle();
        self.controller.reset();
        self.controller.forward_from_first_tick();
    }
}

impl Default for CaretBlink {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF: Duration = Duration::from_millis(500);

    #[test]
    fn new_caret_is_visible_before_the_first_tick() {
        let blink = CaretBlink::new();

        assert!(blink.is_visible());
        assert_eq!(blink.period(), CaretBlink::DEFAULT_PERIOD);
    }

    #[test]
    fn building_does_not_consume_the_period() {
        let blink = CaretBlink::new();
        let start = AnimInstant::now() + Duration::from_secs(30);

        blink.tick(start);

        assert!(blink.is_visible());
        assert!(blink.tick(start + HALF));
    }

    #[test]
    fn caret_hides_after_half_a_period() {
        let blink = CaretBlink::new();
        let start = AnimInstant::now();
        blink.tick(start);

        let toggled = blink.tick(start + HALF);

        assert!(toggled);
        assert!(!blink.is_visible());
    }

    #[test]
    fn caret_reappears_after_a_full_period() {
        let blink = CaretBlink::new();
        let start = AnimInstant::now();
        blink.tick(start);
        blink.tick(start + HALF);

        let toggled = blink.tick(start + CaretBlink::DEFAULT_PERIOD);

        assert!(toggled);
        assert!(blink.is_visible());
    }

    #[test]
    fn ticking_inside_a_half_period_reports_no_toggle() {
        let blink = CaretBlink::new();
        let start = AnimInstant::now();
        blink.tick(start);

        assert!(!blink.tick(start + Duration::from_millis(16)));
        assert!(!blink.tick(start + Duration::from_millis(320)));
        assert!(blink.is_visible());
    }

    #[test]
    fn reset_restores_visibility_and_restarts_the_phase() {
        let blink = CaretBlink::new();
        let start = AnimInstant::now();
        blink.tick(start);
        blink.tick(start + HALF);
        assert!(!blink.is_visible());

        blink.reset();
        let resumed = start + Duration::from_millis(700);
        blink.tick(resumed);

        assert!(blink.is_visible());
        assert!(!blink.tick(resumed + Duration::from_millis(499)));
        assert!(blink.tick(resumed + HALF));
    }

    #[test]
    fn clones_share_one_timeline() {
        let blink = CaretBlink::new();
        let shared = blink.clone();
        let start = AnimInstant::now();

        blink.tick(start);
        blink.tick(start + HALF);

        assert!(!shared.is_visible());

        shared.reset();
        shared.tick(start + Duration::from_millis(600));

        assert!(blink.is_visible());
    }

    #[test]
    fn a_custom_period_scales_both_halves() {
        let blink = CaretBlink::with_period(Duration::from_millis(200));
        let start = AnimInstant::now();
        blink.tick(start);

        assert!(!blink.tick(start + Duration::from_millis(99)));
        assert!(blink.tick(start + Duration::from_millis(100)));
        assert!(blink.tick(start + Duration::from_millis(200)));
    }

    #[test]
    fn caret_context_starts_hidden_until_the_field_publishes_geometry() {
        let context = CaretContext::new(CaretBlink::new());

        assert_eq!(context.geometry(), CaretGeometry::default());
        assert_eq!(context.offset(), 0);
        assert!(!context.is_focused());
        assert!(!context.is_available());
        assert!(!context.is_visible());
        assert!(!context.is_composing());
    }

    #[test]
    fn builtin_caret_is_a_widget() {
        fn assert_widget<W: Widget>(_: W) {}

        let context = CaretContext::new(CaretBlink::new());
        assert_widget(DefaultCaret::new(
            context,
            aimer_widget::base::Colors::default(),
        ));
    }

    /// The field keeps publishing geometry after its caret element is gone —
    /// a page switch tears the caret down before the field that owns the
    /// context. The repaint that publishing asks for must not panic.
    #[test]
    fn publishing_after_the_caret_is_torn_down_does_not_panic() {
        use crate::input_field::raw_fields::test_support::dummy_build_context;

        let ctx = dummy_build_context(100.0, 40.0);
        let context = CaretContext::new(CaretBlink::new());
        let caret = DefaultCaret::new(context.clone(), aimer_widget::base::Colors::default())
            .to_element(&ctx);
        drop(caret);

        context.publish(
            CaretGeometry {
                width: 1.5,
                height: 16.0,
                ..CaretGeometry::default()
            },
            0,
            true,
            true,
            false,
        );
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_scheduling_tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use aimer_animation::AnimInstant;
    use aimer_venus::{PollContext, Venus};
    use tokio::runtime::{Handle, Runtime};

    use super::CaretBlink;

    struct TokioPollContext(Handle);

    impl PollContext for TokioPollContext {
        fn enter(&self, poll: &mut dyn FnMut()) {
            let _guard = self.0.enter();
            poll();
        }
    }

    #[test]
    fn schedules_one_redraw_at_the_next_visibility_transition() {
        let runtime = Runtime::new().expect("a timer runtime");
        let venus = Venus::new();
        venus.set_poll_context(TokioPollContext(runtime.handle().clone()));
        venus.install();
        let scope = venus.scope();

        let redraws = Rc::new(Cell::new(0));
        let counted = redraws.clone();
        let previous = aimer_events::window::set_thread_redraw_requester(move || {
            counted.set(counted.get() + 1);
        });

        let blink = CaretBlink::with_period(Duration::from_millis(4));
        blink.tick(AnimInstant::now());
        blink.schedule_next_toggle(scope.id());
        blink.schedule_next_toggle(scope.id());

        venus.run_microtasks();
        assert_eq!(redraws.get(), 0);

        std::thread::sleep(Duration::from_millis(12));
        venus.run_microtasks();
        assert_eq!(redraws.get(), 1);

        scope.cancel();
        aimer_events::window::restore_thread_redraw_requester(previous);
        Venus::uninstall();
    }
}
