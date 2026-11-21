use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use aimer_animation::AnimInstant;
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd::Rect;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement, Rebuildable, State, StateView,
    StateUpdater, StatefulElement, VisitorElement, Widget,
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
    extent: Option<(f32, f32)>,
    /// What hangs from the caret and is pressed like a button: `(left, top, width, height)`, from the caret's
    /// own top-left corner.
    popup: Option<(f32, f32, f32, f32)>,
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
    popup_press: Rc<RefCell<Option<Rc<dyn Fn(f32, f32)>>>>,
    popup_dismiss: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    popup_scroll: Rc<RefCell<Option<Rc<dyn Fn(f32) -> bool>>>>,
    popup_hover: Rc<RefCell<Option<Rc<dyn Fn(f32, f32)>>>>,
    blink: CaretBlink,
    repaint: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    /// The widgets of an app that hang from the caret and want to be told too.
    repaint_also: Rc<RefCell<Vec<Rc<dyn Fn()>>>>,
}

impl CaretContext {
    #[inline]
    pub(crate) fn new(blink: CaretBlink) -> Self {
        Self {
            snapshot: Rc::new(Cell::new(CaretSnapshot::default())),
            popup_press: Rc::new(RefCell::new(None)),
            popup_dismiss: Rc::new(RefCell::new(None)),
            popup_scroll: Rc::new(RefCell::new(None)),
            popup_hover: Rc::new(RefCell::new(None)),
            blink,
            repaint: Rc::new(RefCell::new(None)),
            repaint_also: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Returns the caret rectangle in text-field local coordinates.
    #[inline]
    pub fn geometry(&self) -> CaretGeometry {
        self.snapshot.get().geometry
    }

    /// Returns where the part of the field that shows text ends, as the far edges `(x, y)` in the same
    /// coordinates as [`geometry`](Self::geometry), or `None` for a field that has not been laid out.
    ///
    /// This is what lets something that hangs from the caret, such as a menu, stay in view: it opens
    /// upwards when there is not room below the caret, and moves left when there is not room beside it.
    #[inline]
    pub fn visible_extent(&self) -> Option<(f32, f32)> {
        self.snapshot.get().extent
    }

    /// Says where something that hangs from the caret, such as a menu, is, as `(left, top, width, height)` from the
    /// caret's top-left corner, or that nothing does.
    ///
    /// A press on it is not the field's: the caret stays where it is, and the press and its release are left to
    /// the widget that is there, as a press on a margin icon or on an embedded widget is. It is told from the
    /// caret and not in the coordinates of the field, so that it is where the caret is when the field scrolls.
    pub fn set_popup(&self, popup: Option<(f32, f32, f32, f32)>) {
        let mut snapshot = self.snapshot.get();
        snapshot.popup = popup;
        self.snapshot.set(snapshot);
    }

    /// Sets what is called when what hangs from the caret is pressed, with where, as `(x, y)` from its top-left corner.
    /// The widgets that hang from the caret are not reached by a press that is outside the caret's own box, as a menu
    /// is, so it is told here instead.
    pub fn on_popup_press(&self, callback: Option<Rc<dyn Fn(f32, f32)>>) {
        *self.popup_press.borrow_mut() = callback;
    }

    /// Sets what is called when what hangs from the caret is to go: the field has lost focus, or the part of it that shows
    /// text has changed size, as when a panel opens under it, so that the place the popup was put at is not the place it
    /// would be put at now. What hangs from the caret is not [set](Self::set_popup) any more when this is called.
    pub fn on_popup_dismiss(&self, callback: Option<Rc<dyn Fn()>>) {
        *self.popup_dismiss.borrow_mut() = callback;
    }

    /// Sets what is called when the wheel or the trackpad scrolls over what hangs from the caret, with how far in logical
    /// pixels, positive where the content is to move down (the way a scroll event tells it). It answers whether it scrolled:
    /// a list that is at its end answers `false`, and the scroll goes on to the field, as it does from any scrollable
    /// that has nowhere further to go. Without it, a scroll over what hangs from the caret scrolls the field under it.
    pub fn on_popup_scroll(&self, callback: Option<Rc<dyn Fn(f32) -> bool>>) {
        *self.popup_scroll.borrow_mut() = callback;
    }

    /// Sets what is called when the pointer moves over what hangs from the caret, with where, as `(x, y)` from its top-left
    /// corner. It is told of a move and not of the pointer resting there while the popup changes under it, so that a list
    /// that scrolls under a still pointer does not take the selection from the keys.
    pub fn on_popup_hover(&self, callback: Option<Rc<dyn Fn(f32, f32)>>) {
        *self.popup_hover.borrow_mut() = callback;
    }

    /// Scrolls what hangs from the caret by `delta_y` when `(x, y)`, in the coordinates of the field, is on it. Whether it
    /// scrolled.
    pub(crate) fn scroll_popup(&self, x: f32, y: f32, delta_y: f32) -> bool {
        if delta_y == 0.0 || !self.popup_at(x, y) {
            return false;
        }
        // Cloned out of the cell first: what is called may set it again.
        let callback = self.popup_scroll.borrow().clone();
        callback.is_some_and(|callback| callback(delta_y))
    }

    /// Tells what hangs from the caret that the pointer is at `(x, y)`, in the coordinates of the field, if it is on it.
    pub(crate) fn hover_popup(&self, x: f32, y: f32) {
        if !self.popup_at(x, y) {
            return;
        }
        let snapshot = self.snapshot.get();
        let (left, top, _, _) = snapshot.popup.expect("a popup is where the pointer is on it");
        // Cloned out of the cell first: what is called may set it again.
        let callback = self.popup_hover.borrow().clone();
        if let Some(callback) = callback {
            callback(x - snapshot.geometry.x - left, y - snapshot.geometry.y - top);
        }
    }

    /// Takes away what hangs from the caret and tells what shows it, if there is anything.
    pub(crate) fn dismiss_popup(&self) {
        let mut snapshot = self.snapshot.get();
        if snapshot.popup.take().is_none() {
            return;
        }
        self.snapshot.set(snapshot);
        // Cloned out of the cell first: what is called may set it again.
        let callback = self.popup_dismiss.borrow().clone();
        if let Some(callback) = callback {
            callback();
        }
    }

    /// Presses what hangs from the caret at `(x, y)`, in the coordinates of the field. Whether there is any there.
    pub(crate) fn press_popup(&self, x: f32, y: f32) -> bool {
        if !self.popup_at(x, y) {
            return false;
        }
        let snapshot = self.snapshot.get();
        let (left, top, _, _) = snapshot.popup.expect("a popup is where a press landed on it");
        // Cloned out of the cell first: what is called may set it again.
        let callback = self.popup_press.borrow().clone();
        if let Some(callback) = callback {
            callback(x - snapshot.geometry.x - left, y - snapshot.geometry.y - top);
        }
        true
    }

    /// Whether `(x, y)`, in the coordinates of the field, is on what hangs from the caret.
    pub(crate) fn popup_at(&self, x: f32, y: f32) -> bool {
        let snapshot = self.snapshot.get();
        let Some((left, top, width, height)) = snapshot.popup else {
            return false;
        };
        let (origin_x, origin_y) = (snapshot.geometry.x + left, snapshot.geometry.y + top);
        x >= origin_x && x < origin_x + width && y >= origin_y && y < origin_y + height
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

    /// Records where the visible part of the field ends, in the coordinates of the geometry.
    pub(crate) fn set_extent(&self, extent: Option<(f32, f32)>) {
        let mut snapshot = self.snapshot.get();
        let resized = snapshot.extent.is_some() && snapshot.extent != extent;
        snapshot.extent = extent;
        self.snapshot.set(snapshot);
        // A popup was put where there was room for it, which there may not be any more, and is where the field was.
        if resized {
            self.dismiss_popup();
        }
    }

    pub(crate) fn set_repaint_callback(&self, callback: Rc<dyn Fn()>) {
        *self.repaint.borrow_mut() = Some(callback);
    }

    /// Asks to be told when the caret is to be painted again: it shows or hides as it blinks, changes size, or is
    /// shown or hidden by focus. Unlike the caret that is built into the field, a caret widget of an app that has the
    /// [`DefaultCaret`] inside it, with something else beside it such as a menu, is not repainted by the caret's own
    /// callback, and rebuilds itself when it is told here.
    ///
    /// The callbacks are kept for as long as the context is: one that is not wanted any more should do nothing.
    pub fn on_repaint(&self, callback: Rc<dyn Fn()>) {
        self.repaint_also.borrow_mut().push(callback);
    }

    pub(crate) fn request_repaint(&self) {
        // Cloned out of the cell first: what is called may ask for another.
        let also: Vec<Rc<dyn Fn()>> = self.repaint_also.borrow().clone();
        for callback in also {
            callback();
        }
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
        // A frame starts by publishing `available = false` and ends with the real
        // geometry. Letting the first of those hide the caret would make every
        // real publish look like a visibility change, and the repaint it asks for
        // schedules the next frame — an idle focused field would never rest. The
        // unavailable publish therefore keeps the phase it inherited; only focus
        // loss hides the caret immediately.
        let visible = if available {
            focused && (composing || self.blink.is_visible())
        } else {
            focused && previous.visible
        };
        self.snapshot.set(CaretSnapshot {
            geometry,
            extent: previous.extent,
            popup: previous.popup,
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

impl Widget for DefaultCaret {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        DefaultCaretWidget {
            context: self.context,
            color: self.color,
        }
        .to_element(ctx)
    }

    fn name(&self) -> &'static str {
        "DefaultCaret"
    }
}

struct DefaultCaretWidget {
    context: CaretContext,
    color: Rc<Cell<Color>>,
}

impl Widget for DefaultCaretWidget {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name::<Self, DefaultCaretState>(self, ctx, "DefaultCaret", None)
            .0
            .boxed()
    }

    fn name(&self) -> &'static str {
        "DefaultCaret"
    }
}

struct DefaultCaretState {
    context: CaretContext,
    color: Rc<Cell<Color>>,
    revision: u64,
}

impl State<DefaultCaretWidget> for DefaultCaretState {
    fn create(widget: DefaultCaretWidget) -> Self {
        DefaultCaretState {
            context: widget.context,
            color: widget.color,
            revision: 0,
        }
    }

    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        let updater = updater;
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

    fn build(this: StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        let _revision = this.revision;
        DefaultCaretPaint {
            context: this.context.clone(),
            color: Rc::clone(&this.color),
        }
    }
}

struct DefaultCaretPaint {
    context: CaretContext,
    color: Rc<Cell<Color>>,
}

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
    fn can_paint(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && ctx.parent_size.width.is_finite()
            && ctx.parent_size.height.is_finite()
            && ctx.parent_size.width >= 0.0
            && ctx.parent_size.height >= 0.0
    }

    fn paint(&self, ctx: &BuildContext) {
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

impl VisitorElement for DefaultCaretElement {}

impl Element for DefaultCaretElement {
    fn name(&self) -> &'static str {
        "DefaultCaret"
    }
}

impl EventElement for DefaultCaretElement {}
impl LayoutElement for DefaultCaretElement {}
impl Rebuildable for DefaultCaretElement {}

/// What every clone of a [`CaretBlink`] shares.
#[derive(Debug)]
struct BlinkState {
    /// When the current cycle began: the first [`CaretBlink::tick`] after the
    /// timeline was created or reset. The phase is measured from here and the
    /// origin is never moved again, see [`CaretBlink::tick`].
    origin: Cell<Option<AnimInstant>>,
    visible: Cell<bool>,
    /// The Venus task ticking the blink's half periods, while one runs.
    task: Cell<Option<aimer_venus::TaskId>>,
    /// Set when the host had no timer to tick with, which leaves the field to
    /// ask for its frames itself.
    timerless: Cell<bool>,
}

/// The blink timeline of a text field caret.
///
/// A `CaretBlink` is a cheap, cloneable handle over one shared phase. Every
/// clone observes the same phase, so the state
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
#[derive(Clone, Debug)]
pub struct CaretBlink {
    period: Duration,
    state: Rc<BlinkState>,
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
    /// by zero.
    #[inline]
    pub fn with_period(period: Duration) -> Self {
        Self {
            period,
            state: Rc::new(BlinkState {
                origin: Cell::new(None),
                visible: Cell::new(true),
                task: Cell::new(None),
                timerless: Cell::new(false),
            }),
        }
    }

    /// Returns the duration of one full on-off cycle.
    #[inline]
    pub fn period(&self) -> Duration {
        self.period
    }

    /// Returns whether the caret should be painted at the current phase.
    #[inline]
    pub fn is_visible(&self) -> bool {
        self.state.visible.get()
    }

    /// Advances the timeline to `now` and reports whether the caret changed
    /// visibility.
    ///
    /// Call this once per frame while the field owns focus. The phase is the
    /// time since the cycle began, modulo the period, so a dropped or late frame
    /// delays a toggle by at most that frame and never shifts the ones after it.
    /// That matters because the toggle wakes tick on a fixed grid of half
    /// periods: a phase that was re-anchored to the frame that noticed each wrap
    /// would slide off that grid, and a frame would arrive before its toggle and
    /// leave the caret stuck for another half period.
    pub fn tick(&self, now: AnimInstant) -> bool {
        let was_visible = self.is_visible();
        let origin = match self.state.origin.get() {
            Some(origin) => origin,
            None => {
                self.state.origin.set(Some(now));
                now
            }
        };
        let period = self.period.as_nanos();
        let visible = period == 0
            || now.duration_since(origin).as_nanos() % period < period / 2;
        self.state.visible.set(visible);
        was_visible != visible
    }

    /// Starts the ticker that wakes the field at every visibility transition,
    /// unless one is already running.
    ///
    /// One Venus [`interval`](aimer_venus::Venus::interval) per focused field,
    /// owned by `scope`, ticks every half period on the host's own timer. A tick
    /// only requests a frame; that frame remains responsible for advancing the
    /// timeline and painting the new state. The interval does not drift and one
    /// task serves every blink, where re-arming a sleep after each toggle spawned
    /// a task per half period.
    ///
    /// The interval is started on the frame that restarts the timeline, so its
    /// ticks land on the toggles. [`reset`](Self::reset) stops it for that
    /// reason, and the next frame starts it again.
    ///
    /// Without a Venus runtime or a host timer nothing can wake the caret from
    /// here: [`timer_unavailable`](Self::timer_unavailable) then reports it, and
    /// the field falls back to asking for a frame on every frame, as isolated
    /// raw-field users already do.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn run_toggle_ticker(&self, scope: aimer_venus::ScopeId) {
        let half = self.period() / 2;
        if half.is_zero() || self.state.timerless.get() || self.state.task.get().is_some() {
            return;
        }
        let Some(venus) = aimer_venus::Venus::current() else {
            self.state.timerless.set(true);
            return;
        };
        let state = Rc::clone(&self.state);
        let mut ticker = venus.interval(half);
        let task = venus.spawn_in(scope, async move {
            while ticker.tick().await.is_ok() {
                aimer_events::window::request_animation_frame();
            }
            // Only a host without timers ends the loop on its own: aborting the
            // task drops it before it gets here.
            state.task.set(None);
            state.timerless.set(true);
        });
        self.state.task.set(Some(task));
    }

    /// Returns whether the host has no timer to tick the blink with, so the
    /// owning field must request its own frames.
    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub(crate) fn timer_unavailable(&self) -> bool {
        self.state.timerless.get()
    }

    /// Stops the ticker, if one runs, and releases its host timer.
    pub(crate) fn cancel_scheduled_toggle(&self) {
        if let Some(task) = self.state.task.take()
            && let Some(venus) = aimer_venus::Venus::current()
        {
            venus.abort(task);
        }
    }

    /// Restarts the timeline at the beginning of its visible half.
    ///
    /// Editing, moving the caret, or clicking into the field calls this so the
    /// caret stays solid while the user is busy, exactly like a native field.
    /// The new period begins at the next [`CaretBlink::tick`].
    pub fn reset(&self) {
        self.cancel_scheduled_toggle();
        self.state.origin.set(None);
        self.state.visible.set(true);
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

    /// A context with a popup at `(10, 20)` from a caret at the field's `(100, 50)`, 200 wide and 80 tall: in the
    /// field it covers x from 110 to 310 and y from 70 to 150.
    fn context_with_popup() -> CaretContext {
        let context = CaretContext::new(CaretBlink::new());
        context.publish(CaretGeometry { x: 100.0, y: 50.0, width: 1.0, height: 16.0 }, 0, true, true, false);
        context.set_popup(Some((10.0, 20.0, 200.0, 80.0)));
        context
    }

    #[test]
    fn a_scroll_over_the_popup_is_the_popups_and_one_beside_it_is_not() {
        let context = context_with_popup();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&seen);
        context.on_popup_scroll(Some(Rc::new(move |delta| {
            log.borrow_mut().push(delta);
            true
        })));

        assert!(context.scroll_popup(120.0, 80.0, -30.0), "over it");
        assert!(!context.scroll_popup(109.0, 80.0, -30.0), "just left of it");
        assert!(!context.scroll_popup(120.0, 150.0, -30.0), "just under it");
        assert!(!context.scroll_popup(120.0, 80.0, 0.0), "a scroll of nothing is not one");
        assert_eq!(*seen.borrow(), [-30.0]);
    }

    #[test]
    fn a_popup_that_cannot_scroll_further_lets_the_scroll_go_on_and_so_does_one_that_asked_for_none() {
        let context = context_with_popup();
        assert!(!context.scroll_popup(120.0, 80.0, 5.0), "nothing asked for it");
        context.on_popup_scroll(Some(Rc::new(|_| false)));
        assert!(!context.scroll_popup(120.0, 80.0, 5.0), "it is at its end");
        context.set_popup(None);
        context.on_popup_scroll(Some(Rc::new(|_| true)));
        assert!(!context.scroll_popup(120.0, 80.0, 5.0), "there is no popup");
    }

    #[test]
    fn a_move_over_the_popup_is_told_from_its_corner_and_a_move_beside_it_is_not_told() {
        let context = context_with_popup();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&seen);
        context.on_popup_hover(Some(Rc::new(move |x, y| log.borrow_mut().push((x, y)))));

        context.hover_popup(130.0, 95.0);
        context.hover_popup(130.0, 69.0);
        context.hover_popup(311.0, 95.0);
        assert_eq!(*seen.borrow(), [(20.0, 25.0)]);
    }

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
    fn a_late_frame_does_not_move_the_toggles_that_follow() {
        // The toggle wakes come from a fixed grid of half periods. A frame that
        // runs late must not drag the blink's phase after it, or the frames on
        // the grid land before their toggle and the caret sticks.
        let blink = CaretBlink::new();
        let start = AnimInstant::now();
        blink.tick(start);

        let mut visible = true;
        for step in 1..=24u32 {
            let late = if step == 2 {
                Duration::from_millis(30)
            } else {
                Duration::from_millis(2)
            };
            let toggled = blink.tick(start + HALF * step + late);

            visible = !visible;
            assert!(toggled, "the frame on grid step {step} flips the caret");
            assert_eq!(blink.is_visible(), visible, "at grid step {step}");
        }
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

    /// Every frame the field first publishes "no caret yet" and then the real
    /// geometry. That pair must not look like a visibility change, or an idle
    /// focused field repaints, and so requests another frame, forever.
    #[test]
    fn an_unchanged_republish_requests_no_repaint() {
        let context = CaretContext::new(CaretBlink::new());
        let repaints = Rc::new(Cell::new(0u32));
        let counted = Rc::clone(&repaints);
        context.set_repaint_callback(Rc::new(move || counted.set(counted.get() + 1)));
        let geometry = CaretGeometry {
            width: 1.5,
            height: 16.0,
            ..CaretGeometry::default()
        };

        // The first frame legitimately changes the size and shows the caret.
        context.publish(geometry, 3, true, false, false);
        context.publish(geometry, 3, true, true, false);
        assert!(context.is_visible());
        let settled = repaints.get();
        assert!(settled > 0);

        for _ in 0..8 {
            context.publish(geometry, 3, true, false, false);
            context.publish(geometry, 3, true, true, false);
        }

        assert!(context.is_visible());
        assert_eq!(repaints.get(), settled);
    }

    /// The caret that is built into the field repaints itself through one callback. A caret widget of an app, with the
    /// caret inside it, is not reached that way, and asks to be told too: without that its caret is painted once and
    /// never again, whatever the blink does.
    #[test]
    fn a_widget_that_hangs_from_the_caret_is_told_of_a_repaint_as_well_as_the_caret_that_is_built_in() {
        let blink = CaretBlink::new();
        let context = CaretContext::new(blink.clone());
        let (own, hanging) = (Rc::new(Cell::new(0u32)), Rc::new(Cell::new(0u32)));
        let (counted_own, counted_hanging) = (Rc::clone(&own), Rc::clone(&hanging));
        context.set_repaint_callback(Rc::new(move || counted_own.set(counted_own.get() + 1)));
        context.on_repaint(Rc::new(move || counted_hanging.set(counted_hanging.get() + 1)));
        let geometry = CaretGeometry { width: 1.5, height: 16.0, ..CaretGeometry::default() };

        context.publish(geometry, 0, true, true, false);
        assert!(own.get() > 0 && hanging.get() > 0, "both are told when the caret first shows");
        let (own_settled, hanging_settled) = (own.get(), hanging.get());
        context.publish(geometry, 0, true, true, false);
        assert_eq!((own.get(), hanging.get()), (own_settled, hanging_settled), "and neither when nothing changed");

        // The blink goes to its hidden half, and then back: each is a change both are told of.
        let start = AnimInstant::now();
        blink.tick(start);
        blink.tick(start + Duration::from_millis(500));
        context.publish(geometry, 0, true, true, false);
        assert!(!context.is_visible());
        assert_eq!((own.get(), hanging.get()), (own_settled + 1, hanging_settled + 1));
        blink.tick(start + Duration::from_millis(1000));
        context.publish(geometry, 0, true, true, false);
        assert!(context.is_visible());
        assert_eq!((own.get(), hanging.get()), (own_settled + 2, hanging_settled + 2));
    }

    /// The transient "unavailable" publish keeps the blink phase, so a hidden
    /// half stays hidden and the toggle back to visible is still reported.
    #[test]
    fn a_blink_toggle_is_still_reported_across_the_transient_publish() {
        let blink = CaretBlink::new();
        let context = CaretContext::new(blink.clone());
        let repaints = Rc::new(Cell::new(0u32));
        let counted = Rc::clone(&repaints);
        context.set_repaint_callback(Rc::new(move || counted.set(counted.get() + 1)));
        let geometry = CaretGeometry {
            width: 1.5,
            height: 16.0,
            ..CaretGeometry::default()
        };
        let start = AnimInstant::now();
        blink.tick(start);
        context.publish(geometry, 0, true, true, false);
        let after_appearing = repaints.get();

        blink.tick(start + HALF);
        context.publish(geometry, 0, true, false, false);
        context.publish(geometry, 0, true, true, false);
        assert!(!context.is_visible());
        assert_eq!(repaints.get(), after_appearing + 1);

        blink.tick(start + CaretBlink::DEFAULT_PERIOD);
        context.publish(geometry, 0, true, false, false);
        context.publish(geometry, 0, true, true, false);
        assert!(context.is_visible());
        assert_eq!(repaints.get(), after_appearing + 2);
    }

    /// Losing focus still hides the caret immediately, transient or not.
    #[test]
    fn losing_focus_hides_the_caret() {
        let context = CaretContext::new(CaretBlink::new());
        let geometry = CaretGeometry {
            width: 1.5,
            height: 16.0,
            ..CaretGeometry::default()
        };
        context.publish(geometry, 0, true, true, false);
        assert!(context.is_visible());

        context.publish(geometry, 0, false, false, false);
        assert!(!context.is_visible());
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
    use std::future::Future;
    use std::pin::Pin;
    use std::rc::Rc;
    use std::time::Duration;

    use aimer_animation::AnimInstant;
    use aimer_venus::{PollContext, Venus};
    use tokio::runtime::{Handle, Runtime};

    use super::CaretBlink;

    /// A host with a runtime-backed timer, like the application's.
    struct TimedHost(Handle);

    impl PollContext for TimedHost {
        fn enter(&self, poll: &mut dyn FnMut()) {
            let _guard = self.0.enter();
            poll();
        }

        fn sleep(&self, duration: Duration) -> Option<Pin<Box<dyn Future<Output = ()>>>> {
            let _guard = self.0.enter();
            Some(Box::pin(tokio::time::sleep(duration)))
        }
    }

    /// A host that can poll tasks but has no timer.
    struct TimerlessHost;

    impl PollContext for TimerlessHost {
        fn enter(&self, poll: &mut dyn FnMut()) {
            poll();
        }
    }

    /// Counts the frames asked for while it lives.
    struct Redraws {
        count: Rc<Cell<u32>>,
        previous: Option<Rc<dyn Fn()>>,
    }

    impl Redraws {
        fn count() -> Self {
            let count = Rc::new(Cell::new(0));
            let counted = count.clone();
            let previous = aimer_events::window::set_thread_redraw_requester(move || {
                counted.set(counted.get() + 1);
            });
            Self { count, previous }
        }

        fn get(&self) -> u32 {
            self.count.get()
        }
    }

    impl Drop for Redraws {
        fn drop(&mut self) {
            aimer_events::window::restore_thread_redraw_requester(self.previous.take());
        }
    }

    fn timed_venus(runtime: &Runtime) -> Rc<Venus> {
        let venus = Venus::new();
        venus.set_poll_context(TimedHost(runtime.handle().clone()));
        venus.install();
        venus
    }

    #[test]
    fn one_interval_keeps_asking_for_the_frame_of_every_toggle() {
        let runtime = Runtime::new().expect("a timer runtime");
        let venus = timed_venus(&runtime);
        let scope = venus.scope();
        let redraws = Redraws::count();

        let blink = CaretBlink::with_period(Duration::from_millis(4));
        blink.tick(AnimInstant::now());
        blink.run_toggle_ticker(scope.id());
        blink.run_toggle_ticker(scope.id());
        assert!(!blink.timer_unavailable());

        venus.run_microtasks();
        assert_eq!(redraws.get(), 0, "nothing is due before the first half period");

        std::thread::sleep(Duration::from_millis(12));
        venus.run_microtasks();
        let first = redraws.get();
        assert!(first >= 1, "the first transition asked for a frame");

        // The same ticker is still running: no frame has to arm it again.
        std::thread::sleep(Duration::from_millis(12));
        venus.run_microtasks();
        assert!(redraws.get() > first, "the interval kept ticking on its own");

        scope.cancel();
        Venus::uninstall();
    }

    #[test]
    fn a_reset_stops_the_ticker_until_the_next_frame_arms_it_again() {
        let runtime = Runtime::new().expect("a timer runtime");
        let venus = timed_venus(&runtime);
        let scope = venus.scope();
        let redraws = Redraws::count();

        let blink = CaretBlink::with_period(Duration::from_millis(4));
        blink.tick(AnimInstant::now());
        blink.run_toggle_ticker(scope.id());
        venus.run_microtasks();

        // Typing restarts the blink, so the old rhythm must not wake anything.
        blink.reset();
        std::thread::sleep(Duration::from_millis(12));
        venus.run_microtasks();
        assert_eq!(redraws.get(), 0, "an aborted ticker asks for nothing");

        blink.tick(AnimInstant::now());
        blink.run_toggle_ticker(scope.id());
        venus.run_microtasks();
        std::thread::sleep(Duration::from_millis(12));
        venus.run_microtasks();
        assert!(redraws.get() >= 1, "the next frame armed a new ticker");

        scope.cancel();
        Venus::uninstall();
    }

    #[test]
    fn a_host_without_a_timer_hands_the_blink_back_to_the_frame_loop() {
        let venus = Venus::new();
        venus.set_poll_context(TimerlessHost);
        venus.install();
        let scope = venus.scope();
        let redraws = Redraws::count();

        let blink = CaretBlink::with_period(Duration::from_millis(4));
        blink.tick(AnimInstant::now());
        blink.run_toggle_ticker(scope.id());
        venus.run_microtasks();

        assert!(blink.timer_unavailable(), "the field must keep asking for frames itself");
        venus.run_microtasks();
        assert_eq!(redraws.get(), 0, "and the failed ticker does not spin");

        scope.cancel();
        Venus::uninstall();
    }
}
