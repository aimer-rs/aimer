//! The widget layer over the recognizer.
//!
//! [`GestureDetector`] is deliberately thin: it translates
//! [`ElementEvent`] into [`PointerEvent`], calls
//! [`recognize`](crate::gesture::recognize::recognize), and hands the result to
//! [`GestureHandlers`]. All the interesting behaviour lives in
//! [`crate::gesture::recognize`], where it can be tested without a window.

use std::cell::RefCell;
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{PointerEvent, PointerSource};
use aimer_macro::{PortableValue, PortableWidget};
use aimer_utils::AnimInstant;
use aimer_widget::InteractionBounds;
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, EventTreeRole,
    LayoutElement, PointerKey, Rebuildable, RequiredChild, VisitorElement, Widget,
};

use crate::callback::VoidCallback;
use crate::gesture::handlers::GestureHandlers;
use crate::gesture::recognize::{poll, recognize_claiming};
use crate::gesture::state::GestureState;
use crate::gesture::{
    DragCallback, DragUpdateCallback, GestureMask, GestureStreamCallback, PanCallback,
    ScaleCallback, ScrollCallback, SmartZoomCallback, SwipeCallback,
};

/// Controls whether a [`GestureDetector`] also exposes its child to event
/// traversal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_input::gesture::gesture_detector::GestureDetectorBehavior",
    max_encoded_bytes = 16
)]
pub enum GestureDetectorBehavior {
    /// The child participates in event, focus, and broadcast dispatch.
    #[portable_value(tag = 0)]
    PassThrough,
    /// The child remains laid out and painted but receives no events.
    #[portable_value(tag = 1)]
    BlockChild,
}

impl Default for GestureDetectorBehavior {
    fn default() -> Self {
        Self::PassThrough
    }
}

/// A transparent widget that recognizes pointer gestures over its child.
///
/// The detector paints nothing and adopts its child's layout; finish
/// construction with [`GestureDetector::child`] or
/// [`GestureDetector::dyn_child`]. Scroll events are consumed only when
/// [`GestureDetector::on_scroll`] is configured, so a detector that does not
/// handle scrolling lets it reach whatever is below. By default, events also
/// pass through to the child; use [`GestureDetector::behavior`] with
/// [`GestureDetectorBehavior::BlockChild`] to prevent all event delivery to it.
///
/// Every handler lives behind one [`Rc`], so rebuilding the widget costs a single
/// refcount bump however many gestures are configured — which matters when a
/// thousand-row list rebuilds its detectors every frame.
///
/// # Examples
///
/// The common gestures have one-line setters:
///
/// ```
/// use aimer_input::gesture::gesture_detector::GestureDetector;
/// use aimer_text::Text;
///
/// let detector = GestureDetector::new()
///     .on_tap(|| println!("tap"))
///     .child(Text::new("Tap me"));
/// ```
///
/// Pinch, rotation and two-finger pan arrive together through
/// [`GestureDetector::on_scale`], whether they come from two fingers on a touch
/// screen or from a trackpad, so one handler serves both. The trackpad's
/// smart-zoom double tap has a handler of its own, as does the platform's
/// multi-finger pan ([`GestureDetector::on_pan`]):
///
/// ```
/// use aimer_input::gesture::gesture_detector::GestureDetector;
/// use aimer_input::gesture::{ScaleData, SmartZoomData};
/// use aimer_text::Text;
///
/// let photo = GestureDetector::new()
///     .on_scale(|zoom: ScaleData| {
///         // Incremental: multiply the zoom, add the turn, follow the midpoint.
///         println!(
///             "x{:.2}, {:.0} degrees, moved by ({}, {})",
///             zoom.delta_scale,
///             zoom.delta_rotation.to_degrees(),
///             zoom.focal_delta_x,
///             zoom.focal_delta_y,
///         );
///     })
///     .on_smart_zoom(|at: SmartZoomData| println!("toggle zoom around {}, {}", at.x, at.y))
///     .child(Text::new("Photo"));
/// ```
///
/// Anything the setters do not cover — press feedback, the long-press
/// lifecycle, drag cancellation, a middle click — is reached through
/// [`GestureDetector::on_gesture`], which sees the whole stream:
///
/// ```
/// use aimer_events::pointer::PointerButton;
/// use aimer_input::gesture::GestureEvent;
/// use aimer_input::gesture::gesture_detector::GestureDetector;
/// use aimer_text::Text;
///
/// let detector = GestureDetector::new()
///     .on_gesture(|event: GestureEvent| match event {
///         GestureEvent::TapDown { .. } => println!("pressed"),
///         GestureEvent::TapCancel => println!("released without activating"),
///         GestureEvent::Tap { pointer } if pointer.button == PointerButton::Middle => {
///             println!("middle click")
///         }
///         _ => {}
///     })
///     .child(Text::new("Save"));
/// ```
#[allow(dead_code)]
#[derive(PortableWidget)]
#[portable_widget(id = "aimer_input::GestureDetector", schema_only)]
pub struct GestureDetector<W = RequiredChild> {
    // The flat no-argument mirrors give the derive stable callback slots. The
    // aggregate handler set remains the native hot-path dispatch object.
    #[portable_callback(async)]
    on_tap: VoidCallback,
    #[portable_callback(async)]
    on_double_press: VoidCallback,
    #[portable_callback(async)]
    on_long_press: VoidCallback,
    #[portable_skip]
    on_drag_start: DragCallback,
    #[portable_skip]
    on_drag_update: DragUpdateCallback,
    #[portable_callback(async)]
    on_drag_end: VoidCallback,
    #[portable_callback(async)]
    on_right_tap: VoidCallback,
    #[portable_skip]
    on_swipe: SwipeCallback,
    #[portable_skip]
    on_scroll: ScrollCallback,
    #[portable_skip]
    on_scale: ScaleCallback,
    #[portable_skip]
    on_pan: PanCallback,
    #[portable_skip]
    on_smart_zoom: SmartZoomCallback,
    #[portable_skip]
    on_gesture: GestureStreamCallback,
    #[portable_skip]
    handlers: Rc<GestureHandlers>,
    #[portable_optional]
    behavior: GestureDetectorBehavior,
    #[portable_child(discriminator = 0)]
    child: W,
}

impl Default for GestureDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureDetector {
    /// Creates a detector with no handlers and a required-child placeholder.
    #[inline]
    pub fn new() -> Self {
        Self {
            on_tap: VoidCallback::default(),
            on_double_press: VoidCallback::default(),
            on_long_press: VoidCallback::default(),
            on_drag_start: DragCallback::default(),
            on_drag_update: DragUpdateCallback::default(),
            on_drag_end: VoidCallback::default(),
            on_right_tap: VoidCallback::default(),
            on_swipe: SwipeCallback::default(),
            on_scroll: ScrollCallback::default(),
            on_scale: ScaleCallback::default(),
            on_pan: PanCallback::default(),
            on_smart_zoom: SmartZoomCallback::default(),
            on_gesture: GestureStreamCallback::default(),
            handlers: Rc::new(GestureHandlers::new()),
            behavior: GestureDetectorBehavior::PassThrough,
            child: RequiredChild,
        }
    }
}

/// Generates the builder setters, each one delegating to the matching
/// mask-maintaining setter on [`GestureHandlers`].
macro_rules! builder_setters {
    ($($setter:ident => $install:ident : $callback:ty, $doc:expr;)*) => {
        impl<W> GestureDetector<W> {
            $(
                #[doc = $doc]
                #[inline]
                pub fn $setter(mut self, callback: impl Into<$callback>) -> Self {
                    let callback = callback.into();
                    self.$setter = callback.clone();
                    self.handlers_mut().$install(callback);
                    self
                }
            )*
        }
    };
}

builder_setters! {
    on_tap => set_on_tap: VoidCallback,
        "Sets the handler for a primary tap completed within the pointer's slop.\n\nA secondary click goes to [`Self::on_right_tap`] instead, and a middle or extra button reaches only [`Self::on_gesture`].";
    on_double_press => set_on_double_press: VoidCallback,
        "Sets the handler fired after two qualifying taps within the double-tap timeout.";
    on_long_press => set_on_long_press: VoidCallback,
        "Sets the handler fired once a held pointer reaches the long-press duration.\n\nFires while the pointer is still down, provided something redraws the frame while it is held; see [`Self::on_gesture`] for the surrounding start/move/end events.";
    on_drag_start => set_on_drag_start: DragCallback,
        "Sets the handler fired when movement first exceeds the pointer's slop.\n\nThe handler receives the position the press *started* at, so a dragged widget does not jump by the slop distance as it picks up.";
    on_drag_update => set_on_drag_update: DragUpdateCallback,
        "Sets the handler fired for movement while a drag is active.\n\n[`crate::gesture::DragUpdateData`] reports the current position and the delta since the previous update.";
    on_drag_end => set_on_drag_end: VoidCallback,
        "Sets the handler fired when an active drag ends.\n\nA flick fires this *and* [`Self::on_swipe`], in that order.";
    on_right_tap => set_on_right_tap: VoidCallback,
        "Sets the handler for a completed secondary-button tap.";
    on_swipe => set_on_swipe: SwipeCallback,
        "Sets the handler for a fast directional drag recognized as a swipe.\n\nThe handler receives the resulting [`crate::gesture::SwipeDirection`].";
    on_scroll => set_on_scroll: ScrollCallback,
        "Sets the handler for mouse-wheel or trackpad scrolling over the child.\n\nInstalling this causes the detector to consume matching scroll events; without it, those events fall through to lower layers.";
    on_scale => set_on_scale: ScaleCallback,
        "Sets the handler for a pinch, rotation or two-finger pan, from touch contacts or from the trackpad.\n\n[`crate::gesture::ScaleData`] reports the scale and rotation relative to the start of the gesture and since the previous update. A touch pinch measures them between the two contacts that arrived first; a trackpad pinch or twist is reported by the platform, so the focal point stays where the cursor was and does not move. The detector consumes the trackpad's pinch and rotate events only when this is set.";
    on_pan => set_on_pan: PanCallback,
        "Sets the handler for a platform multi-finger pan, such as an N-finger pan on iOS.\n\nThis is not the single-pointer drag: use [`Self::on_drag_update`] for that, and [`Self::on_scale`] for the pan of two touch contacts. The detector consumes the platform's pan events only when this is set.";
    on_smart_zoom => set_on_smart_zoom: SmartZoomCallback,
        "Sets the handler for the platform's smart-zoom gesture, a two-finger double tap on a macOS trackpad.\n\nConventionally it toggles the content between fitted and actual size around [`crate::gesture::SmartZoomData`]'s point. The detector consumes the event only when this is set.";
    on_gesture => set_on_gesture: GestureStreamCallback,
        "Sets the handler that receives every recognized gesture.\n\nThe escape hatch: a gesture with no setter of its own — the press lifecycle, the long-press start/move/end triple, drag cancellation, the scale and pan boundaries, a middle click — is read here. A new gesture is therefore a new [`crate::gesture::GestureEvent`] variant, not a new field on this struct.";
}

impl<W> GestureDetector<W> {
    /// The handler set, uniquely owned while the builder is still being
    /// configured.
    ///
    /// Mutating through the `Rc` in place is what keeps configuration free: the
    /// shared handle only ever gains a second owner in
    /// [`Widget::to_element`], which happens after the builder is finished.
    #[inline]
    fn handlers_mut(&mut self) -> &mut GestureHandlers {
        Rc::get_mut(&mut self.handlers)
            .expect("gesture handlers are uniquely owned while the detector is being built")
    }

    /// Supplies the terminal child and returns a statically typed detector.
    ///
    /// Existing handlers are preserved. A detector without a child is only an
    /// intermediate builder and does not implement [`Widget`].
    #[inline]
    pub fn child<C: Widget>(self, child: C) -> GestureDetector<C> {
        GestureDetector {
            on_tap: self.on_tap,
            on_double_press: self.on_double_press,
            on_long_press: self.on_long_press,
            on_drag_start: self.on_drag_start,
            on_drag_update: self.on_drag_update,
            on_drag_end: self.on_drag_end,
            on_right_tap: self.on_right_tap,
            on_swipe: self.on_swipe,
            on_scroll: self.on_scroll,
            on_scale: self.on_scale,
            on_pan: self.on_pan,
            on_smart_zoom: self.on_smart_zoom,
            on_gesture: self.on_gesture,
            handlers: self.handlers,
            behavior: self.behavior,
            child,
        }
    }

    /// Selects how events are routed between this detector and its child.
    ///
    /// [`GestureDetectorBehavior::PassThrough`] is the default. Selecting
    /// [`GestureDetectorBehavior::BlockChild`] keeps the child in the visual
    /// and structural tree but removes it from event, focus, and broadcast
    /// traversal.
    #[inline]
    pub fn behavior(mut self, behavior: GestureDetectorBehavior) -> Self {
        self.behavior = behavior;
        self
    }

    /// Supplies the terminal child and erases the completed detector's concrete
    /// type.
    ///
    /// Exactly equivalent to `self.child(child).boxed()`. Use it when branching
    /// APIs need one [`AnyWidget`] return type despite using different concrete
    /// child types.
    #[inline]
    pub fn dyn_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget + 'static> Widget for GestureDetector<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawGestureDetector {
            child: self.child.to_element(ctx),
            cached_bounds: InteractionBounds::new(),
            window: ctx.window.clone(),
            // One refcount bump, whatever the detector was configured with.
            handlers: self.handlers.clone(),
            state: RefCell::new(GestureState::default()),
            behavior: self.behavior,
        }
        .boxed()
    }
}

/// The element a [`GestureDetector`] builds: a pure recognizer wrapped around a
/// child.
///
/// It renders nothing of its own — pressed overlays and hover effects belong to
/// higher-level widgets such as [`crate::button::Button`] — and mirrors Flutter's
/// `GestureDetector` in that respect.
pub struct RawGestureDetector<E: Element> {
    pub child: E,
    pub(crate) cached_bounds: InteractionBounds,
    pub(crate) window: WindowHandle,
    pub(crate) handlers: Rc<GestureHandlers>,
    pub(crate) behavior: GestureDetectorBehavior,
    /// Interior mutability because `on_event` takes `&self`.
    pub(crate) state: RefCell<GestureState>,
}

impl<E: Element> RawGestureDetector<E> {
    /// Runs the recognizer over one pointer event and delivers what it found.
    fn process(&self, event: &PointerEvent) {
        let output = {
            let mut state = self.state.borrow_mut();
            recognize_claiming(
                &mut state,
                event,
                AnimInstant::now(),
                self.handlers.mask(),
                self.handlers.claimed(),
            )
        };
        self.handlers.dispatch_all(output);
    }

    /// Gives the recognizer a chance to report a long press while the pointer is
    /// still held.
    ///
    /// Called from [`Drawable::update`], because a frame is the only regular tick
    /// available: the windowing layer can be asked to redraw but not to redraw
    /// *later*. A press held in an otherwise completely static frame therefore
    /// reports its long press on release instead, which the recognizer handles as
    /// a late long press rather than mistaking it for a slow tap.
    fn poll_held_gestures(&self) {
        let output = {
            let mut state = self.state.borrow_mut();
            if state.press.is_none() {
                return;
            }
            poll(&mut state, AnimInstant::now(), self.handlers.mask())
        };

        if !output.is_empty() {
            self.handlers.dispatch_all(output);
            self.window.request_redraw();
        }
    }
}

/// The pointer event an [`ElementEvent`] corresponds to, if any.
fn to_pointer_event(event: &ElementEvent) -> Option<PointerEvent> {
    match event {
        ElementEvent::PointerDown(pointer) => Some(PointerEvent::Down(*pointer)),
        ElementEvent::PointerUp(pointer) => Some(PointerEvent::Up(*pointer)),
        ElementEvent::PointerMove(pointer) => Some(PointerEvent::Move(*pointer)),
        ElementEvent::Scroll { delta, .. } => Some(PointerEvent::Scroll {
            delta_x: delta.x,
            delta_y: delta.y,
        }),
        ElementEvent::Cancel => Some(PointerEvent::Cancel),
        _ => platform_gesture(event).map(|(pointer_event, _)| pointer_event),
    }
}

/// The pointer event for a gesture the platform recognized itself, with the
/// gestures a detector must be listening for to care about it.
///
/// These carry no contact, so unlike a press they have no pointer to capture and
/// are not filtered by bounds: the dispatcher already hit-tested them at the
/// cursor, exactly as it does for a scroll.
fn platform_gesture(event: &ElementEvent) -> Option<(PointerEvent, GestureMask)> {
    match event {
        ElementEvent::Pinch { pos, delta, phase } => Some((
            PointerEvent::Pinch {
                pos: *pos,
                delta: *delta,
                phase: *phase,
            },
            GestureMask::SCALE,
        )),
        ElementEvent::Rotate { pos, delta, phase } => Some((
            PointerEvent::Rotate {
                pos: *pos,
                delta: *delta,
                phase: *phase,
            },
            GestureMask::SCALE,
        )),
        ElementEvent::Pan { pos, delta, phase } => Some((
            PointerEvent::Pan {
                pos: *pos,
                delta_x: delta.x,
                delta_y: delta.y,
                phase: *phase,
            },
            GestureMask::PAN,
        )),
        ElementEvent::SmartZoom { pos } => {
            Some((PointerEvent::SmartZoom { pos: *pos }, GestureMask::SMART_ZOOM))
        }
        _ => None,
    }
}

/// Whether a pointer event is this detector's business.
///
/// Inside the bounds, always. Outside them, only a release for a pointer this
/// detector is already tracking: the finger that pressed inside owns the gesture
/// until it lifts, wherever it lifts.
fn should_accept_pointer_event(
    cached_bounds: &InteractionBounds,
    state: &GestureState,
    event: &ElementEvent,
    pos: Vec2d,
) -> bool {
    let source = event.pointer().map_or(PointerSource::Mouse, |pointer| pointer.source);
    if cached_bounds.is_inside_for(pos.x, pos.y, source) {
        return true;
    }

    match event {
        ElementEvent::PointerUp(pointer) => state.has_active_touch(pointer.source, pointer.id),
        _ => false,
    }
}

/// Claims the pointer on press and gives it back on release, so the gesture
/// keeps receiving events even after it leaves the detector's bounds.
fn pointer_capture_effect(result: EventResult, event: &ElementEvent) -> EventResult {
    match event {
        ElementEvent::PointerDown(pointer) => {
            result.with_pointer_capture(PointerKey::new(pointer.source, pointer.id))
        }
        ElementEvent::PointerUp(pointer) => {
            result.with_pointer_release(PointerKey::new(pointer.source, pointer.id))
        }
        _ => result,
    }
}

// ── Element trait impls ─────────────────────────────────────────────────

impl<E: Element> VisitorElement for RawGestureDetector<E> {
    /// The detector always paints its child, whatever its event behavior is, so
    /// the retained render tree (built from this visitor) must see it. Event
    /// routing is a separate concern handled by `event_children`.
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }

    fn debug_name(&self) -> &'static str {
        "GestureDetector"
    }
}

impl<E: Element> EventElement for RawGestureDetector<E> {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if matches!(event, ElementEvent::Cancel) {
            self.process(&PointerEvent::Cancel);
            self.window.request_redraw();
            return EventResult::consumed().with_redraw();
        }

        if let ElementEvent::Scroll { delta, .. } = event {
            if !self.handlers.consumes_scroll() {
                return EventResult::ignored();
            }
            self.process(&PointerEvent::Scroll {
                delta_x: delta.x,
                delta_y: delta.y,
            });
            self.window.request_redraw();
            return EventResult::consumed().with_redraw();
        }

        if let Some((gesture, wanted)) = platform_gesture(event) {
            // Observing is not claiming, as for a scroll: the raw `on_gesture`
            // handler is told, but only a handler installed for the gesture
            // itself keeps it from the widgets behind.
            if !self.handlers.mask().intersects(wanted) {
                return EventResult::ignored();
            }
            self.process(&gesture);
            self.window.request_redraw();
            return if self.handlers.claims(wanted) {
                EventResult::consumed().with_redraw()
            } else {
                EventResult::ignored().with_redraw()
            };
        }

        let Some(pointer) = event.pointer() else {
            return EventResult::ignored();
        };

        if !should_accept_pointer_event(
            &self.cached_bounds,
            &self.state.borrow(),
            event,
            pointer.pos,
        ) {
            return EventResult::ignored();
        }

        let Some(pointer_event) = to_pointer_event(event) else {
            return EventResult::ignored();
        };

        let output = {
            let mut state = self.state.borrow_mut();
            recognize_claiming(
                &mut state,
                &pointer_event,
                AnimInstant::now(),
                self.handlers.mask(),
                self.handlers.claimed(),
            )
        };

        // A move that advanced nothing through an idle recognizer is a pure
        // hover. Claiming it — and scheduling a frame for it — used to render
        // the whole window at input rate whenever the cursor merely crossed a
        // detector, so it falls through untouched instead.
        if output.is_empty()
            && matches!(event, ElementEvent::PointerMove(_))
            && !self.state.borrow().is_engaged()
        {
            return EventResult::ignored();
        }

        // A drag handler that has begun a drag owns the pointer for it. Saying so
        // is what lets an ancestor that arbitrates touches (a scroll view, a
        // navigator that pops on a sideways swipe) leave the gesture alone,
        // without having to infer it from what this consumed.
        //
        // Only a handler installed for the drag owns it. The raw `on_gesture`
        // stream (a button's pressed highlight) merely observes it, and a swipe
        // moves nothing until the finger lifts: claiming for either would make a
        // scroll view stand down whenever the first sample past the press already
        // travelled beyond the tap slop, which is every fast flick.
        if self.handlers.owns_drag()
            && output.iter().any(|gesture| matches!(gesture, crate::gesture::GestureEvent::DragStart { .. }))
        {
            aimer_widget::claim_pointer(aimer_widget::PointerKey::new(pointer.source, pointer.id));
        }

        self.handlers.dispatch_all(output);
        self.window.request_redraw();
        pointer_capture_effect(EventResult::consumed().with_redraw(), event)
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if self.behavior == GestureDetectorBehavior::PassThrough {
            visitor(&self.child);
        }
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }

    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.event_children(visitor);
    }
}

impl<E: Element> LayoutElement for RawGestureDetector<E> {
    #[inline]
    fn size(&self) -> Option<Size> {
        None
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        None
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let size = self.child.layout(ctx);
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        self.cached_bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        size
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    /// A finger hits a larger area than a cursor, so the rectangle depends on the
    /// pointer being routed.
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.cached_bounds.hit_pos_start_end()
    }

}

impl<E: Element> Drawable for RawGestureDetector<E> {
    fn update(&self, ctx: &BuildContext<'_>) {
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let child_size = self.child.computed_size(ctx);
        self.cached_bounds
            .save(ctx.scale, abs_x, abs_y, child_size.width, child_size.height);
        self.poll_held_gestures();
        self.child.update(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        canvas.finish();
    }

    /// The detector hit-tests the border box its child reports, which can be
    /// larger than the content box the render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.child.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.cached_bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.cached_bounds.disagreement("GestureDetector")
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext<'_>) {
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let child_size = self.child.computed_size(ctx);
        self.cached_bounds
            .save(ctx.scale, abs_x, abs_y, child_size.width, child_size.height);
        self.child.sync_paint_geometry(ctx);
    }
}

impl<E: Element + 'static> Rebuildable for RawGestureDetector<E> {
    #[inline]
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Claims the gesture the element being replaced was in the middle of.
    ///
    /// A gesture outlives the element that started it. `TapDown` fires while the
    /// pointer is still down, so anything that reacts to a press — a
    /// [`crate::button::Button`] darkening itself, a row highlighting — calls
    /// `set_state`, and that rebuild replaces every element below it, this one
    /// included. A replacement starting from nothing would find no press when
    /// the release arrived and report no tap at all: the button would light up
    /// under the finger and then do nothing.
    ///
    /// So the recognizer's state is *moved* out of `old`, which reconciliation
    /// drops immediately afterwards, leaving exactly one element tracking the
    /// gesture.
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|value| value.downcast_ref::<Self>())
        else {
            return;
        };

        // The state-carry walk can revisit a detector after an ancestor has
        // already moved its press into this replacement. Do not replace a live
        // recognizer with the now-drained default state from that second visit.
        if *old.state.borrow() == GestureState::default()
            && *self.state.borrow() != GestureState::default()
        {
            return;
        }

        *self.state.borrow_mut() = std::mem::take(&mut *old.state.borrow_mut());
    }
}

#[cfg(test)]
mod tests {
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::{CaptureRequest, broadcast_event, dispatch_event, dispatch_focused_event};

    use super::*;
    use crate::gesture::state::Press;
    use crate::gesture::{GestureMask, ScaleData, ScrollData, SwipeDirection};

    struct TestWidget;

    impl Widget for TestWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            panic!("not needed for builder tests")
        }
    }

    impl aimer_widget::PortableWidget for TestWidget {}

    struct TestElement;

    struct RecordingElement {
        events: Rc<std::cell::Cell<usize>>,
    }

    impl VisitorElement for RecordingElement {
        fn debug_name(&self) -> &'static str {
            "RecordingElement"
        }
    }

    impl EventElement for RecordingElement {
        fn on_event(&self, _event: &ElementEvent) -> EventResult {
            self.events.set(self.events.get() + 1);
            EventResult::ignored()
        }
    }

    impl LayoutElement for RecordingElement {}
    impl Drawable for RecordingElement {
        fn update(&self, _ctx: &BuildContext<'_>) {}
    }
    impl Rebuildable for RecordingElement {}

    impl VisitorElement for TestElement {
        fn debug_name(&self) -> &'static str {
            "TestElement"
        }
    }

    impl EventElement for TestElement {}
    impl LayoutElement for TestElement {}
    impl Drawable for TestElement {
        fn update(&self, _ctx: &BuildContext<'_>) {}
    }
    impl Rebuildable for TestElement {}

    /// A laid-out detector counting the taps it recognizes, so two of them can
    /// share a counter across a simulated rebuild.
    fn counting_detector(taps: Rc<std::cell::Cell<usize>>) -> RawGestureDetector<TestElement> {
        let mut handlers = GestureHandlers::new();
        handlers.set_on_tap(move || taps.set(taps.get() + 1));

        let cached_bounds = InteractionBounds::new();
        cached_bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);

        RawGestureDetector {
            child: TestElement,
            cached_bounds,
            window: WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 100), 1.0),
            handlers: Rc::new(handlers),
            state: RefCell::new(GestureState::default()),
            behavior: GestureDetectorBehavior::PassThrough,
        }
    }

    /// An ancestor that arbitrates touches, such as a navigator that pops on a
    /// sideways swipe, has to be able to tell that a descendant is dragging. The
    /// claim is how: a pointer a drag handler is dragging is not for it to take.
    #[test]
    fn a_detector_that_starts_a_drag_claims_the_pointer_for_it() {
        let detector = detector_with(|handlers| handlers.set_on_drag_start(|_| {}));
        let key = aimer_widget::PointerKey::new(PointerSource::Touch, 3);

        let _ = detector.on_event(&ElementEvent::PointerDown(touch(10.0, 10.0, 3)));
        assert!(!aimer_widget::is_pointer_claimed(key), "pressed, not yet dragging");
        let _ = detector.on_event(&ElementEvent::PointerMove(touch(80.0, 10.0, 3)));
        assert!(aimer_widget::is_pointer_claimed(key), "the drag began");

        aimer_widget::release_pointer(key);
    }

    #[test]
    fn a_detector_with_no_drag_handler_never_claims_a_pointer() {
        let detector = detector_with(|handlers| handlers.set_on_tap(|| {}));
        let key = aimer_widget::PointerKey::new(PointerSource::Touch, 4);

        let _ = detector.on_event(&ElementEvent::PointerDown(touch(10.0, 10.0, 4)));
        let _ = detector.on_event(&ElementEvent::PointerMove(touch(90.0, 10.0, 4)));

        assert!(!aimer_widget::is_pointer_claimed(key), "a button dragged off is not a drag");
    }

    /// `Button` installs the raw stream only to show a pressed highlight. Observing
    /// a drag is not owning it, and claiming it would stop a scroll view from
    /// taking the finger whenever the first sample after the press already
    /// travelled past the tap slop — a fast flick.
    #[test]
    fn a_detector_that_only_observes_gestures_never_claims_a_pointer() {
        let detector = detector_with(|handlers| handlers.set_on_gesture(|_: GestureEvent| {}));
        let key = aimer_widget::PointerKey::new(PointerSource::Touch, 5);

        let _ = detector.on_event(&ElementEvent::PointerDown(touch(10.0, 10.0, 5)));
        let _ = detector.on_event(&ElementEvent::PointerMove(touch(10.0, 80.0, 5)));

        assert!(
            !aimer_widget::is_pointer_claimed(key),
            "an observer does not own the drag it watches"
        );
    }

    /// A swipe is only known once the finger lifts, so while the finger moves the
    /// detector has taken nothing from anyone. Claiming would make it veto a
    /// vertical scroll over a horizontal swipe.
    #[test]
    fn a_swipe_detector_never_claims_a_pointer() {
        let detector = detector_with(|handlers| handlers.set_on_swipe(|_: SwipeDirection| {}));
        let key = aimer_widget::PointerKey::new(PointerSource::Touch, 6);

        let _ = detector.on_event(&ElementEvent::PointerDown(touch(10.0, 10.0, 6)));
        let _ = detector.on_event(&ElementEvent::PointerMove(touch(10.0, 80.0, 6)));

        assert!(!aimer_widget::is_pointer_claimed(key), "a swipe moves nothing while it is made");
    }

    #[test]
    fn gesture_detector_is_an_indexed_target() {
        let detector = counting_detector(Rc::new(std::cell::Cell::new(0)));

        assert_eq!(
            detector.event_tree_role(),
            aimer_widget::EventTreeRole::IndexedTarget
        );
        assert_eq!(detector.event_tree_bounds(), None);
    }

    fn recording_detector(
        behavior: GestureDetectorBehavior,
        events: Rc<std::cell::Cell<usize>>,
    ) -> AnyElement {
        let cached_bounds = InteractionBounds::new();
        cached_bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);

        RawGestureDetector {
            child: RecordingElement { events }.boxed(),
            cached_bounds,
            window: WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 100), 1.0),
            handlers: Rc::new(GestureHandlers::new()),
            state: RefCell::new(GestureState::default()),
            behavior,
        }
        .boxed()
    }

    fn touch(x: f32, y: f32, id: u64) -> PointerInfo {
        PointerInfo::touch(Vec2d { x, y }, id)
    }

    fn pressing(pointer: PointerInfo) -> GestureState {
        let mut state = GestureState::default();
        state.touches.insert(pointer);
        state.press = Some(Press {
            pointer,
            down_at: AnimInstant::now(),
            long_pressed: false,
            long_press_last: None,
        });
        state
    }

    #[test]
    fn the_builder_configures_every_gesture_before_a_child_is_added() {
        let detector = GestureDetector::new()
            .on_tap(|| {})
            .on_double_press(|| {})
            .on_long_press(|| {})
            .on_drag_start(|_: PointerInfo| {})
            .on_drag_update(|_: crate::gesture::DragUpdateData| {})
            .on_drag_end(|| {})
            .on_right_tap(|| {})
            .on_swipe(|_: SwipeDirection| {})
            .on_scroll(|_: ScrollData| {})
            .on_scale(|_: ScaleData| {})
            .child(TestWidget);

        let mask = detector.handlers.mask();

        assert!(mask.contains(GestureMask::TAP));
        assert!(mask.contains(GestureMask::DOUBLE_TAP));
        assert!(mask.contains(GestureMask::LONG_PRESS));
        assert!(mask.contains(GestureMask::DRAG));
        assert!(mask.contains(GestureMask::SWIPE));
        assert!(mask.contains(GestureMask::SCALE));
        assert!(detector.handlers.consumes_scroll());
    }

    #[test]
    fn child_event_behavior_defaults_to_pass_through_and_can_block() {
        let pass_through = GestureDetector::new().child(TestWidget);
        assert_eq!(pass_through.behavior, GestureDetectorBehavior::PassThrough);

        let blocked = GestureDetector::new()
            .behavior(GestureDetectorBehavior::BlockChild)
            .child(TestWidget);
        assert_eq!(blocked.behavior, GestureDetectorBehavior::BlockChild);
    }

    #[test]
    fn blocking_child_events_removes_the_child_from_all_event_dispatch_paths() {
        let events = Rc::new(std::cell::Cell::new(0));
        let detector = recording_detector(GestureDetectorBehavior::BlockChild, events.clone());
        let pointer = touch(5.0, 5.0, 7);

        let pointer_result = dispatch_event(
            detector.as_ref(),
            pointer.pos,
            &ElementEvent::PointerDown(pointer),
        );
        let _ = dispatch_focused_event(detector.as_ref(), &ElementEvent::FocusGained);
        let _ = broadcast_event(detector.as_ref(), &ElementEvent::FocusGained);

        assert!(pointer_result.is_consumed());
        assert_eq!(events.get(), 0);

        let mut structural_children = 0;
        detector.structural_children(&mut |_| structural_children += 1);
        assert_eq!(structural_children, 1);
    }

    #[test]
    fn the_retained_render_tree_can_reach_the_detectors_child() {
        // The render tree is built from `visit_retained_v2_children`, which
        // defaults to `visit_children`. A detector that draws its child but does
        // not expose it there leaves the child without a render node, and its
        // paint is silently dropped — for every behavior, not only pass-through.
        for behavior in [
            GestureDetectorBehavior::PassThrough,
            GestureDetectorBehavior::BlockChild,
        ] {
            let detector = recording_detector(behavior, Rc::new(std::cell::Cell::new(0)));

            let mut visited = 0;
            detector.visit_children(&mut |_| visited += 1);
            assert_eq!(visited, 1, "{behavior:?} must expose its child to paint");

            let mut retained = 0;
            detector.visit_retained_v2_children(&mut |_, _| retained += 1);
            assert_eq!(
                retained, 1,
                "{behavior:?} must expose its child to the render tree"
            );
        }
    }

    #[test]
    fn pass_through_child_events_reach_the_child_before_the_detector() {
        let events = Rc::new(std::cell::Cell::new(0));
        let detector = recording_detector(GestureDetectorBehavior::PassThrough, events.clone());
        let pointer = touch(5.0, 5.0, 7);

        let result = dispatch_event(
            detector.as_ref(),
            pointer.pos,
            &ElementEvent::PointerDown(pointer),
        );

        assert!(result.is_consumed());
        assert_eq!(events.get(), 1);
    }

    // Regression for "the Scroll is not able to scroll with mouse wheel or
    // trackpad": a detector with no `on_scroll` handler — a header `TextButton`
    // is a MouseRegion plus a GestureDetector — must not consume a scroll,
    // otherwise, sitting on a top `Stack` layer and dispatched first, it
    // swallows every wheel and trackpad scroll before a `Scrollable` on a lower
    // layer can see it, and nothing scrolls.
    #[test]
    fn a_detector_without_a_scroll_handler_lets_the_scroll_fall_through() {
        let detector = GestureDetector::new().on_tap(|| {}).child(TestWidget);

        assert!(
            !detector.handlers.consumes_scroll(),
            "a handler-less detector must let the scroll propagate to lower layers"
        );
    }

    #[test]
    fn a_detector_with_a_scroll_handler_consumes_the_scroll() {
        let detector = GestureDetector::new()
            .on_scroll(|_: ScrollData| {})
            .child(TestWidget);

        assert!(detector.handlers.consumes_scroll());
    }

    // The raw handler is the escape hatch for gestures with no setter, so it has
    // to ask the recognizer for everything.
    #[test]
    fn a_raw_gesture_handler_asks_for_every_gesture() {
        let detector = GestureDetector::new()
            .on_gesture(|_: crate::gesture::GestureEvent| {})
            .child(TestWidget);

        assert!(detector.handlers.mask().contains(GestureMask::SCALE));
        assert!(detector.handlers.mask().contains(GestureMask::LONG_PRESS));
    }

    #[test]
    fn adding_a_child_keeps_the_handlers_that_were_already_configured() {
        let detector = GestureDetector::new().on_scale(|_: ScaleData| {});
        let mask_before = detector.handlers.mask();

        let with_child = detector.child(TestWidget);

        assert_eq!(with_child.handlers.mask(), mask_before);
    }

    #[test]
    fn every_pointer_element_event_maps_to_a_pointer_event() {
        let pointer = touch(1.0, 2.0, 3);

        assert!(matches!(
            to_pointer_event(&ElementEvent::PointerDown(pointer)),
            Some(PointerEvent::Down(_))
        ));
        assert!(matches!(
            to_pointer_event(&ElementEvent::PointerUp(pointer)),
            Some(PointerEvent::Up(_))
        ));
        assert!(matches!(
            to_pointer_event(&ElementEvent::PointerMove(pointer)),
            Some(PointerEvent::Move(_))
        ));
        assert!(matches!(
            to_pointer_event(&ElementEvent::Cancel),
            Some(PointerEvent::Cancel)
        ));
        assert!(
            to_pointer_event(&ElementEvent::PointerExited(PointerSource::Mouse, 0)).is_none(),
            "leaving the window is not a gesture"
        );
    }

    // The translated pointer must keep its button, or the recognizer is back to
    // treating every press as primary.
    #[test]
    fn translation_preserves_the_button_the_press_was_made_with() {
        let secondary = PointerInfo::mouse(Vec2d { x: 5.0, y: 5.0 }, PointerButton::Secondary);

        let translated = to_pointer_event(&ElementEvent::PointerDown(secondary));

        assert!(matches!(
            translated,
            Some(PointerEvent::Down(pointer)) if pointer.button == PointerButton::Secondary
        ));
    }

    #[test]
    fn a_press_inside_the_cached_bounds_is_accepted() {
        let bounds = InteractionBounds::new();
        bounds.save(1.0, 10.0, 20.0, 100.0, 50.0);
        let pointer = touch(25.0, 35.0, 7);

        assert!(should_accept_pointer_event(
            &bounds,
            &GestureState::default(),
            &ElementEvent::PointerDown(pointer),
            pointer.pos
        ));
    }

    #[test]
    fn a_press_outside_the_cached_bounds_is_rejected() {
        let bounds = InteractionBounds::new();
        bounds.save(1.0, 10.0, 20.0, 100.0, 50.0);
        let pointer = touch(200.0, 35.0, 7);

        assert!(!should_accept_pointer_event(
            &bounds,
            &GestureState::default(),
            &ElementEvent::PointerDown(pointer),
            pointer.pos
        ));
    }

    #[test]
    fn a_release_outside_the_bounds_is_accepted_for_a_pointer_being_tracked() {
        let bounds = InteractionBounds::new();
        bounds.save(1.0, 10.0, 20.0, 100.0, 50.0);
        let state = pressing(touch(25.0, 35.0, 7));
        let outside = touch(115.0, 35.0, 7);

        assert!(should_accept_pointer_event(
            &bounds,
            &state,
            &ElementEvent::PointerUp(outside),
            outside.pos
        ));

        let other_device = PointerInfo::mouse(outside.pos, PointerButton::Primary);

        assert!(
            !should_accept_pointer_event(
                &bounds,
                &state,
                &ElementEvent::PointerUp(other_device),
                other_device.pos
            ),
            "a mouse release is not the tracked finger, however matching the id"
        );
    }

    #[test]
    fn an_accepted_pointer_sequence_captures_then_releases() {
        let pointer = touch(5.0, 5.0, 7);
        let key = PointerKey::new(pointer.source, pointer.id);

        let down =
            pointer_capture_effect(EventResult::consumed(), &ElementEvent::PointerDown(pointer));
        let up = pointer_capture_effect(EventResult::consumed(), &ElementEvent::PointerUp(pointer));

        assert_eq!(down.capture_request(), CaptureRequest::Capture(key));
        assert_eq!(up.capture_request(), CaptureRequest::Release(key));
    }

    // Regression for "the theme toggle and the platform buttons stopped
    // working": `Button` reacts to `TapDown` by setting its pressed state, and
    // that rebuild replaces this element while the finger is still down. Unless
    // the replacement takes the press over, the release finds no gesture in
    // progress and no tap is ever reported — the button darkens and then does
    // nothing at all.
    fn click(detector: &RawGestureDetector<TestElement>, x: f32, y: f32) {
        let pointer = PointerInfo::mouse(Vec2d { x, y }, PointerButton::Primary);
        let _ = detector.on_event(&ElementEvent::PointerDown(pointer));
        let _ = detector.on_event(&ElementEvent::PointerUp(pointer));
    }

    /// A detector over a 20x20 control, the kind of target a fingertip misses.
    fn small_control(taps: Rc<std::cell::Cell<usize>>) -> RawGestureDetector<TestElement> {
        let detector = counting_detector(taps);
        detector.cached_bounds.save(1.0, 100.0, 100.0, 20.0, 20.0);
        detector
    }

    fn tap_with(detector: &RawGestureDetector<TestElement>, pointer: PointerInfo) {
        let _ = detector.on_event(&ElementEvent::PointerDown(pointer));
        let _ = detector.on_event(&ElementEvent::PointerUp(pointer));
    }

    /// Runs `test` with the touch target at 44px and the pointer source restored.
    fn with_touch_target(test: impl FnOnce()) {
        let previous = aimer_events::pointer::min_touch_target();
        aimer_events::pointer::set_min_touch_target(44.0);
        test();
        aimer_events::pointer::set_min_touch_target(previous);
        aimer_events::pointer::note_pointer_source(PointerSource::Mouse);
    }

    #[test]
    fn a_touch_just_outside_a_small_control_still_taps_it() {
        with_touch_target(|| {
            let taps = Rc::new(std::cell::Cell::new(0));
            let detector = small_control(taps.clone());

            // Ten pixels left of the control: a miss by mouse, a hit by finger.
            tap_with(&detector, touch(90.0, 110.0, 1));

            assert_eq!(taps.get(), 1);
        });
    }

    #[test]
    fn a_mouse_just_outside_a_small_control_does_not_tap_it() {
        with_touch_target(|| {
            let taps = Rc::new(std::cell::Cell::new(0));
            let detector = small_control(taps.clone());

            click(&detector, 90.0, 110.0);

            assert_eq!(taps.get(), 0, "the extra area is for fingers only");
        });
    }

    #[test]
    fn a_touch_beyond_the_extra_area_misses() {
        with_touch_target(|| {
            let taps = Rc::new(std::cell::Cell::new(0));
            let detector = small_control(taps.clone());

            tap_with(&detector, touch(70.0, 110.0, 1));

            assert_eq!(taps.get(), 0);
        });
    }

    #[test]
    fn the_routed_rectangle_is_larger_only_while_a_finger_is_routed() {
        with_touch_target(|| {
            let detector = small_control(Rc::new(std::cell::Cell::new(0)));

            aimer_events::pointer::note_pointer_source(PointerSource::Mouse);
            assert_eq!(
                detector.pos_start_end(),
                Some((Vec2d { x: 100.0, y: 100.0 }, Vec2d { x: 120.0, y: 120.0 }))
            );

            aimer_events::pointer::note_pointer_source(PointerSource::Touch);
            assert_eq!(
                detector.pos_start_end(),
                Some((Vec2d { x: 88.0, y: 88.0 }, Vec2d { x: 132.0, y: 132.0 }))
            );
        });
    }

    #[test]
    fn a_touch_in_the_extra_area_reaches_the_detector_through_dispatch() {
        with_touch_target(|| {
            let taps = Rc::new(std::cell::Cell::new(0));
            let mut handlers = GestureHandlers::new();
            let counted = taps.clone();
            handlers.set_on_tap(move || counted.set(counted.get() + 1));
            let cached_bounds = InteractionBounds::new();
            cached_bounds.save(1.0, 100.0, 100.0, 20.0, 20.0);
            let detector = RawGestureDetector {
                child: RecordingElement { events: Rc::default() }.boxed(),
                cached_bounds,
                window: WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 100), 1.0),
                handlers: Rc::new(handlers),
                state: RefCell::new(GestureState::default()),
                behavior: GestureDetectorBehavior::PassThrough,
            }
            .boxed();
            let pointer = touch(90.0, 110.0, 1);

            // The dispatcher notes the pointer source itself as the event enters.
            let mut dispatcher = aimer_widget::EventDispatcher::new();
            let _ = dispatcher.dispatch(detector.as_ref(), pointer.pos, &ElementEvent::PointerDown(pointer));
            let _ = dispatcher.dispatch(detector.as_ref(), pointer.pos, &ElementEvent::PointerUp(pointer));

            assert_eq!(taps.get(), 1);
        });
    }

    #[test]
    fn the_render_tree_rectangle_becomes_the_hit_area() {
        use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};

        let taps = Rc::new(std::cell::Cell::new(0));
        let detector = counting_detector(taps.clone());
        let tree = RenderTree::new();
        let node = tree.add_root(Rect::new(200.0, 200.0, 10.0, 10.0)).unwrap();

        detector.adopt_retained_v2_interaction_source(aimer_widget::InteractionSource::new(
            tree,
            node,
            (50.0, 50.0),
        ));

        assert_eq!(
            detector.pos_start_end(),
            Some((Vec2d { x: 200.0, y: 200.0 }, Vec2d { x: 250.0, y: 250.0 }))
        );
        click(&detector, 10.0, 10.0);
        assert_eq!(taps.get(), 0, "the canvas-derived area no longer hits");
        click(&detector, 210.0, 210.0);
        assert_eq!(taps.get(), 1);
    }

    fn layout_context() -> BuildContext<'static> {
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
    async fn a_detector_opts_in_with_the_size_its_child_reports() {
        let detector = counting_detector(Rc::new(std::cell::Cell::new(0)));
        let ctx = layout_context();

        // The interaction size is the child's computed size, which for a box
        // model widget is the border box, not the render node's content box.
        assert_eq!(
            detector.retained_v2_interaction_size(&ctx),
            Some(detector.child.computed_size(&ctx))
        );
    }

    #[test]
    fn a_rebuild_between_the_press_and_the_release_still_reports_the_tap() {
        let taps = Rc::new(std::cell::Cell::new(0));
        let pressed = counting_detector(taps.clone());
        let pointer = PointerInfo::mouse(Vec2d { x: 10.0, y: 10.0 }, PointerButton::Primary);

        let _ = pressed.on_event(&ElementEvent::PointerDown(pointer));

        let rebuilt = counting_detector(taps.clone());
        rebuilt.adopt_runtime_state_from(&pressed as &dyn Element);

        let _ = rebuilt.on_event(&ElementEvent::PointerUp(pointer));

        assert_eq!(
            taps.get(),
            1,
            "a rebuild triggered by the press must not swallow the tap"
        );
    }

    // The state is moved, not copied: two elements both believing they own the
    // press would report the tap twice.
    #[test]
    fn the_element_being_replaced_gives_the_press_up_rather_than_sharing_it() {
        let taps = Rc::new(std::cell::Cell::new(0));
        let pressed = counting_detector(taps.clone());
        let pointer = PointerInfo::mouse(Vec2d { x: 10.0, y: 10.0 }, PointerButton::Primary);

        let _ = pressed.on_event(&ElementEvent::PointerDown(pointer));

        let rebuilt = counting_detector(taps.clone());
        rebuilt.adopt_runtime_state_from(&pressed as &dyn Element);

        let _ = pressed.on_event(&ElementEvent::PointerUp(pointer));

        assert_eq!(taps.get(), 0, "the old element no longer owns the gesture");
    }

    #[test]
    fn a_move_neither_captures_nor_releases_the_pointer() {
        let pointer = touch(5.0, 5.0, 7);

        let moved =
            pointer_capture_effect(EventResult::consumed(), &ElementEvent::PointerMove(pointer));

        assert_eq!(moved.capture_request(), CaptureRequest::None);
    }

    // Regression for "moving the cursor pins a core": a hover move over an
    // idle recognizer produces no gesture, so consuming it — and scheduling a
    // frame for it — turned every mouse move over any detector into a full
    // window redraw. A forum page is covered edge to edge in detectors, which
    // is how merely waving the cursor rendered the whole tree at input rate.
    #[test]
    fn a_hover_move_over_an_idle_recognizer_asks_for_nothing() {
        let detector = counting_detector(Rc::new(std::cell::Cell::new(0)));
        let hover = PointerInfo::mouse(Vec2d { x: 10.0, y: 10.0 }, PointerButton::Primary);

        let result = detector.on_event(&ElementEvent::PointerMove(hover));

        assert!(
            !result.is_consumed(),
            "an idle hover move must fall through"
        );
        assert!(
            !result.needs_redraw(),
            "nothing changed, nothing to repaint"
        );
        assert!(
            !detector.window.take_redraw_request(),
            "no frame may be scheduled for a move that advanced no gesture"
        );
    }

    // The guard above must not starve a live gesture: a move while the press
    // is held still belongs to this detector, keeps the frames coming for the
    // long-press poll, and stays consumed so nothing underneath steals it.
    #[test]
    fn a_move_during_a_press_still_claims_the_gesture_and_repaints() {
        let detector = counting_detector(Rc::new(std::cell::Cell::new(0)));
        let pointer = touch(10.0, 10.0, 2);
        *detector.state.borrow_mut() = pressing(pointer);

        let result = detector.on_event(&ElementEvent::PointerMove(touch(12.0, 12.0, 2)));

        assert!(result.is_consumed(), "a held press owns its moves");
        assert!(
            result.needs_redraw(),
            "a live gesture keeps the frames coming"
        );
    }

    // ── Platform gestures ──────────────────────────────────────────────

    use aimer_events::element::TouchPhase;

    use crate::gesture::{GestureEvent, PanData, SmartZoomData};

    const CURSOR: Vec2d = Vec2d { x: 30.0, y: 40.0 };

    /// A laid-out detector with exactly the handlers `configure` installs.
    fn detector_with(
        configure: impl FnOnce(&mut GestureHandlers),
    ) -> RawGestureDetector<TestElement> {
        let mut handlers = GestureHandlers::new();
        configure(&mut handlers);

        let cached_bounds = InteractionBounds::new();
        cached_bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);

        RawGestureDetector {
            child: TestElement,
            cached_bounds,
            window: WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 100), 1.0),
            handlers: Rc::new(handlers),
            state: RefCell::new(GestureState::default()),
            behavior: GestureDetectorBehavior::PassThrough,
        }
    }

    fn pinch_event(delta: f32, phase: TouchPhase) -> ElementEvent {
        ElementEvent::Pinch {
            pos: CURSOR,
            delta,
            phase,
        }
    }

    #[test]
    fn a_trackpad_pinch_reaches_on_scale_as_a_scale_update_and_is_consumed() {
        let scales: Rc<RefCell<Vec<ScaleData>>> = Rc::default();
        let recorded = scales.clone();
        let detector = detector_with(|handlers| {
            handlers.set_on_scale(move |data: ScaleData| recorded.borrow_mut().push(data));
        });

        let started = detector.on_event(&pinch_event(0.0, TouchPhase::Started));
        let moved = detector.on_event(&pinch_event(0.5, TouchPhase::Moved));

        assert!(started.is_consumed() && moved.is_consumed());
        assert_eq!(
            *scales.borrow(),
            vec![ScaleData {
                focal_x: 30.0,
                focal_y: 40.0,
                scale: 1.5,
                delta_scale: 1.5,
                rotation: 0.0,
                delta_rotation: 0.0,
                focal_delta_x: 0.0,
                focal_delta_y: 0.0,
            }]
        );
    }

    #[test]
    fn a_trackpad_rotation_reaches_on_scale_with_the_angle() {
        let scales: Rc<RefCell<Vec<ScaleData>>> = Rc::default();
        let recorded = scales.clone();
        let detector = detector_with(|handlers| {
            handlers.set_on_scale(move |data: ScaleData| recorded.borrow_mut().push(data));
        });

        detector.on_event(&ElementEvent::Rotate {
            pos: CURSOR,
            delta: 0.25,
            phase: TouchPhase::Started,
        });

        let seen = scales.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].rotation, 0.25);
        assert_eq!(seen[0].delta_rotation, 0.25);
        assert_eq!(seen[0].delta_scale, 1.0);
    }

    // The raw handler observes the pinch, but a Button watching its press that way
    // must not swallow the pinch meant for the canvas it sits on.
    #[test]
    fn the_raw_handler_observes_a_trackpad_pinch_without_claiming_it() {
        let seen: Rc<RefCell<Vec<GestureEvent>>> = Rc::default();
        let recorded = seen.clone();
        let detector = detector_with(|handlers| {
            handlers.set_on_gesture(move |event: GestureEvent| recorded.borrow_mut().push(event));
        });

        let result = detector.on_event(&pinch_event(0.2, TouchPhase::Started));

        assert!(!result.is_consumed(), "observing is not claiming");
        assert!(
            seen.borrow()
                .iter()
                .any(|event| matches!(event, GestureEvent::ScaleStart { .. }))
        );
    }

    #[test]
    fn a_detector_that_does_not_care_about_scale_ignores_the_pinch_entirely() {
        let detector = detector_with(|handlers| handlers.set_on_tap(|| {}));

        let result = detector.on_event(&pinch_event(0.2, TouchPhase::Started));

        assert!(!result.is_consumed());
        assert_eq!(*detector.state.borrow(), GestureState::default());
    }

    #[test]
    fn a_platform_pan_reaches_on_pan_and_is_consumed() {
        let pans: Rc<RefCell<Vec<PanData>>> = Rc::default();
        let recorded = pans.clone();
        let detector = detector_with(|handlers| {
            handlers.set_on_pan(move |data: PanData| recorded.borrow_mut().push(data));
        });

        detector.on_event(&ElementEvent::Pan {
            pos: CURSOR,
            delta: Vec2d::ZERO,
            phase: TouchPhase::Started,
        });
        let moved = detector.on_event(&ElementEvent::Pan {
            pos: CURSOR,
            delta: Vec2d { x: 5.0, y: -2.0 },
            phase: TouchPhase::Moved,
        });

        assert!(moved.is_consumed());
        assert_eq!(
            *pans.borrow(),
            vec![PanData {
                x: 30.0,
                y: 40.0,
                delta_x: 5.0,
                delta_y: -2.0
            }]
        );
    }

    #[test]
    fn smart_zoom_reaches_on_smart_zoom_and_is_consumed() {
        let zooms: Rc<RefCell<Vec<SmartZoomData>>> = Rc::default();
        let recorded = zooms.clone();
        let detector = detector_with(|handlers| {
            handlers.set_on_smart_zoom(move |data: SmartZoomData| recorded.borrow_mut().push(data));
        });

        let result = detector.on_event(&ElementEvent::SmartZoom { pos: CURSOR });

        assert!(result.is_consumed());
        assert_eq!(*zooms.borrow(), vec![SmartZoomData { x: 30.0, y: 40.0 }]);
    }

    #[test]
    fn smart_zoom_falls_through_a_detector_with_no_handler_for_it() {
        let detector = detector_with(|handlers| handlers.set_on_scale(|_: ScaleData| {}));

        assert!(
            !detector
                .on_event(&ElementEvent::SmartZoom { pos: CURSOR })
                .is_consumed()
        );
    }

    // A platform gesture is not filtered by bounds: the dispatcher has already
    // hit-tested it at the cursor, as for a scroll.
    #[test]
    fn a_platform_gesture_is_accepted_wherever_the_dispatcher_delivered_it() {
        let scales: Rc<RefCell<Vec<ScaleData>>> = Rc::default();
        let recorded = scales.clone();
        let detector = detector_with(|handlers| {
            handlers.set_on_scale(move |data: ScaleData| recorded.borrow_mut().push(data));
        });

        detector.on_event(&ElementEvent::Pinch {
            pos: Vec2d {
                x: 5000.0,
                y: 5000.0,
            },
            delta: 0.1,
            phase: TouchPhase::Started,
        });

        assert_eq!(scales.borrow().len(), 1);
    }

    #[test]
    fn a_cancel_ends_a_trackpad_pinch_that_was_running() {
        let seen: Rc<RefCell<Vec<GestureEvent>>> = Rc::default();
        let recorded = seen.clone();
        let detector = detector_with(|handlers| {
            handlers.set_on_gesture(move |event: GestureEvent| recorded.borrow_mut().push(event));
        });
        detector.on_event(&pinch_event(0.0, TouchPhase::Started));

        detector.on_event(&ElementEvent::Cancel);

        assert_eq!(seen.borrow().last(), Some(&GestureEvent::ScaleEnd));
    }

    #[test]
    fn platform_gestures_map_to_pointer_events() {
        assert!(matches!(
            to_pointer_event(&pinch_event(0.1, TouchPhase::Moved)),
            Some(PointerEvent::Pinch { .. })
        ));
        assert!(matches!(
            to_pointer_event(&ElementEvent::Rotate {
                pos: CURSOR,
                delta: 0.1,
                phase: TouchPhase::Moved
            }),
            Some(PointerEvent::Rotate { .. })
        ));
        assert!(matches!(
            to_pointer_event(&ElementEvent::Pan {
                pos: CURSOR,
                delta: Vec2d::ZERO,
                phase: TouchPhase::Moved
            }),
            Some(PointerEvent::Pan { .. })
        ));
        assert!(matches!(
            to_pointer_event(&ElementEvent::SmartZoom { pos: CURSOR }),
            Some(PointerEvent::SmartZoom { .. })
        ));
    }
}
