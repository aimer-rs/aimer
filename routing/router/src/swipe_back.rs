//! Recognizes the swipe that pops a page.
//!
//! The recognizer is a plain state machine fed pointer samples and answering
//! with what the host should do next. It owns no tree, clock or widget, which
//! is what makes the thresholds testable. The host owns the consequences:
//! building the page beneath, presenting the drag, and committing or cancelling.
//!
//! A touch that begins in the edge zone is *held back* from the page until its
//! intent is known, because the page's own handlers (a button, a horizontal
//! scroller) would otherwise take the touch and the swipe could never win.
//! Once it moves past the slop it is either a swipe, or it was an ordinary touch
//! after all and the held press is [replayed](SwipeOutput::Replay) to the page.
//!
//! A touch anywhere else goes to the page at once and is only *watched*, the way
//! a scroll view watches a touch it does not yet know is a scroll. It becomes a
//! swipe if it travels clearly rightward and the page did not take it: a child
//! that [claimed](aimer_widget::pointer_claim) the pointer (a horizontal
//! scroller that is scrolling, a slider, a text selection) keeps it, and one that
//! gives it back (a scroller at its edge) does not. When the swipe takes the touch
//! the page is told it was cancelled, so a button that was pressed lets go.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_attribute::Vec2d;
use aimer_events::element::{ElementEvent, ScrollDeltaKind, TouchPhase};
use aimer_events::pointer::{PointerInfo, PointerSource, last_pointer_source};
use aimer_widget::{Element, EventDispatchContext, EventResult, PointerKey};

use crate::trackpad_swipe::{TrackpadConfig, TrackpadOutput, TrackpadSwipe};
use crate::transition_host::SwipeDrive;

/// A pop that a finger is in the middle of, from
/// [`NavigatorController::begin_swipe_back`](crate::NavigatorController::begin_swipe_back).
///
/// While it lives the page on screen follows [`drag`](Self::drag) and the page
/// beneath is shown where it was. History does not change until
/// [`release`](Self::release) commits, and a cancelled swipe changes nothing.
/// Another navigation arriving first cancels it, and a handle to a swipe that is
/// over ignores everything.
#[derive(Clone)]
pub struct SwipeBack(pub(crate) Rc<SwipeDrive>);

impl SwipeBack {
    /// Moves the page to `progress` of the way across, from `0.0` (where it
    /// was) to `1.0` (gone). Values outside the range are clamped.
    #[inline]
    pub fn drag(&self, progress: f32) {
        self.0.drag(progress);
    }

    /// Lets go: the page settles to the end and history pops if `commit`, and
    /// settles back and stays if not.
    #[inline]
    pub fn release(&self, commit: bool) {
        self.0.release(commit);
    }

    /// Whether the swipe has ended, by completing, cancelling or being replaced.
    #[inline]
    pub fn is_over(&self) -> bool {
        self.0.outcome().is_some()
    }
}

/// Where on a page a swipe back may begin.
///
/// A touch in the 20 px at the left edge always wins, on every setting: it is
/// held back from the page until it is known whether it is a swipe, so nothing
/// on the page can take it first. The setting is about the rest of the page.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SwipeBackArea {
    /// Only the left edge. The rest of the page sees every touch exactly as it
    /// would without a swipe: nothing is watched, so nothing can go wrong with a
    /// widget that does its own sideways dragging and does not say so.
    Edge,
    /// Anywhere on the page, as in the iOS Files app. A touch goes to the page
    /// at once and is only watched; it becomes a swipe when it travels clearly
    /// rightward and nothing on the page took it. A widget that drags sideways
    /// (a carousel, a slider) keeps the touch by claiming the pointer, which
    /// scroll views and gesture detectors with a drag handler do; one that
    /// handles pointer events itself should call
    /// [`claim_pointer`](aimer_widget::claim_pointer) when it starts a drag.
    #[default]
    Anywhere,
}

/// The swipe of one navigator, shared by every host it builds.
///
/// A navigator rebuilds its host to show the page beneath, and the touch that
/// caused it carries on in the host that replaces the first. So the touch's
/// state lives here, with the navigator, and a host only forwards to it.
pub(crate) struct SwipeLink {
    /// Whether there is a page to go back to and an effect to play.
    enabled: Cell<bool>,
    recognizer: RefCell<EdgeSwipe>,
    /// The two-finger trackpad swipe, which shares [`Self::active`] with the touch
    /// one: a navigator pops one page at a time, by whichever device asked first.
    trackpad: RefCell<TrackpadSwipe>,
    /// When the trackpad gesture being followed began, so its recognizer sees small
    /// numbers however long the process has been up.
    trackpad_epoch: Cell<f64>,
    /// The swipe being presented, from [`Self::begin`].
    active: RefCell<Option<SwipeBack>>,
    /// The touch whose events now go straight to the page, after the swipe
    /// turned out not to be one. The host stays its capture owner meanwhile.
    forwarding: Cell<Option<PointerKey>>,
    begin: RefCell<Box<dyn Fn() -> Option<SwipeBack>>>,
    /// Seconds on any monotonic clock.
    clock: Box<dyn Fn() -> f64>,
    /// When the press that is now being decided arrived, so the recognizer
    /// sees small numbers however long the process has been up.
    epoch: Cell<f64>,
}

impl SwipeLink {
    pub(crate) fn new(begin: impl Fn() -> Option<SwipeBack> + 'static, clock: impl Fn() -> f64 + 'static) -> Rc<Self> {
        Rc::new(Self {
            enabled: Cell::new(false),
            recognizer: RefCell::new(EdgeSwipe::new(SwipeConfig::default())),
            trackpad: RefCell::new(TrackpadSwipe::new(TrackpadConfig::default())),
            trackpad_epoch: Cell::new(0.0),
            active: RefCell::new(None),
            forwarding: Cell::new(None),
            begin: RefCell::new(Box::new(begin)),
            clock: Box::new(clock),
            epoch: Cell::new(0.0),
        })
    }

    /// A link on the system clock, to be given its `begin` by the navigator once
    /// that exists.
    pub(crate) fn detached() -> Rc<Self> {
        use std::sync::OnceLock;
        use aimer_time::Instant;
        static START: OnceLock<Instant> = OnceLock::new();
        Self::new(|| None, || START.get_or_init(Instant::now).elapsed().as_secs_f64())
    }

    pub(crate) fn install(&self, begin: impl Fn() -> Option<SwipeBack> + 'static) {
        *self.begin.borrow_mut() = Box::new(begin);
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
    }

    /// Whether a touch outside the edge zone may become a swipe too.
    pub(crate) fn set_anywhere(&self, anywhere: bool) {
        self.recognizer.borrow_mut().set_anywhere(anywhere);
    }

    /// Whether the host hides its page from hit testing for an event at `pos`,
    /// taking the event itself.
    ///
    /// Hit testing asks this before routing, because the page would otherwise
    /// see a press first: in the edge zone it must not, so the swipe can win over
    /// whatever is there, and elsewhere the host has to see every event of the
    /// touch to watch it, which it does as the page's forwarder. Only for a
    /// finger, and only while the navigator can pop.
    ///
    /// A touch the host is already arbitrating keeps the page hidden wherever it
    /// goes, so its events reach the host by hit testing as well as by capture:
    /// the host is rebuilt when a swipe begins, and a capture is not something to
    /// stake a gesture on across a rebuild.
    pub(crate) fn hides_page(&self, pos: Vec2d, host_left: f32, host_width: f32) -> bool {
        let recognizer = self.recognizer.borrow();
        let from_left = pos.x - host_left;
        let inside = if recognizer.config.anywhere {
            (0.0..=host_width).contains(&from_left)
        } else {
            (0.0..=recognizer.config.edge).contains(&from_left)
        };
        let engaged = !recognizer.is_idle() || self.forwarding.get().is_some();
        self.enabled.get() && last_pointer_source() == PointerSource::Touch && (inside || engaged)
    }

    fn now(&self) -> f32 {
        ((self.clock)() - self.epoch.get()) as f32
    }

    /// Whether the page took the touch. A child that begins a drag of its own
    /// (a scroller that is scrolling, a slider, a selection) [claims] the
    /// pointer; that is the one signal that survives the page being reached
    /// through a retained-child placement, which strips a result's redraw flag,
    /// and a child that merely consumed a movement, as a pressed button does, has
    /// not taken anything a swipe cannot take.
    ///
    /// [claims]: aimer_widget::pointer_claim
    fn page_took(key: PointerKey) -> bool {
        aimer_widget::is_pointer_claimed(key)
    }

    /// Handles an event the host owns: a press it holds back or watches, and
    /// everything the touch that press began does from then on.
    pub(crate) fn handle(
        &self,
        event: &ElementEvent,
        context: &mut EventDispatchContext<'_, '_>,
        page: &dyn Element,
        host_left: f32,
        host_width: f32,
    ) -> EventResult {
        let key = |pointer: &PointerInfo| PointerKey::new(pointer.source, pointer.id);
        let pos = context.position();
        match event {
            ElementEvent::PointerDown(pointer) => {
                self.epoch.set((self.clock)());
                let hidden = self.hides_page(pos, host_left, host_width);
                let disposition = self.recognizer.borrow_mut().down(*pointer, host_left, host_width, self.enabled.get());
                match disposition {
                    Disposition::Hold => EventResult::consumed().with_pointer_capture(key(pointer)),
                    // The page has the press now; the host watches the rest.
                    Disposition::Track => self.forward(context, page, pointer.pos, event, key(pointer)),
                    // Hidden from hit testing but not ours (another finger is being
                    // arbitrated, or the host has no width): the page gets it as it
                    // would have, and the rest of that touch comes through here too.
                    Disposition::Pass if hidden => context.dispatch_child(page, pos, event),
                    Disposition::Pass => EventResult::ignored(),
                }
            }
            ElementEvent::PointerMove(pointer) => {
                if self.forwarding.get() == Some(key(pointer)) {
                    return self.forward(context, page, pointer.pos, event, key(pointer));
                }
                if !self.recognizer.borrow().owns(pointer) {
                    return self.pass_through(context, page, pos, event, host_left, host_width);
                }
                let watching = self.recognizer.borrow().is_watching();
                // A watched touch is the page's first: it sees the movement
                // before the host decides, and the host only takes what it left.
                let page_result = if watching {
                    let result = self.forward(context, page, pointer.pos, event, key(pointer));
                    if Self::page_took(key(pointer)) {
                        self.recognizer.borrow_mut().yield_to_page();
                        self.forwarding.set(Some(key(pointer)));
                        return result;
                    }
                    Some(result)
                } else {
                    None
                };
                let output = self.recognizer.borrow_mut().moved(*pointer, self.now());
                match output {
                    SwipeOutput::Begin => {
                        let swipe = (self.begin.borrow())();
                        match swipe {
                            Some(swipe) => {
                                if watching {
                                    // The page has been pressed; tell it the press is off.
                                    let _ = context.cancel_pointer(key(pointer));
                                }
                                swipe.drag(0.0);
                                *self.active.borrow_mut() = Some(swipe);
                                aimer_events::window::request_animation_frame();
                                EventResult::consumed().with_redraw()
                            }
                            None => match self.recognizer.borrow_mut().decline() {
                                SwipeOutput::Replay(press) => self.replay(context, page, press, event),
                                _ => {
                                    self.forwarding.set(Some(key(pointer)));
                                    page_result.unwrap_or_else(EventResult::consumed)
                                }
                            },
                        }
                    }
                    SwipeOutput::Update(progress) => {
                        if let Some(swipe) = self.active.borrow().as_ref() {
                            swipe.drag(progress);
                        }
                        aimer_events::window::request_animation_frame();
                        EventResult::consumed().with_redraw()
                    }
                    SwipeOutput::Replay(press) => self.replay(context, page, press, event),
                    SwipeOutput::Yield => {
                        self.forwarding.set(Some(key(pointer)));
                        page_result.unwrap_or_else(EventResult::consumed)
                    }
                    SwipeOutput::None | SwipeOutput::Release { .. } => page_result.unwrap_or_else(EventResult::consumed),
                }
            }
            ElementEvent::PointerUp(pointer) => {
                if self.forwarding.get() == Some(key(pointer)) {
                    self.forwarding.set(None);
                    return self.forward(context, page, pointer.pos, event, key(pointer)).without_capture_request()
                        .with_pointer_release(key(pointer));
                }
                if !self.recognizer.borrow().owns(pointer) {
                    return self.pass_through(context, page, pos, event, host_left, host_width);
                }
                let watching = self.recognizer.borrow().is_watching();
                // A tap, or the end of a drag the page owned: the page ends it.
                let page_result = watching.then(|| self.forward(context, page, pointer.pos, event, key(pointer)));
                let output = self.recognizer.borrow_mut().up(*pointer, self.now());
                match output {
                    SwipeOutput::Release { commit } => {
                        if let Some(swipe) = self.active.borrow_mut().take() {
                            swipe.release(commit);
                        }
                        aimer_events::window::request_animation_frame();
                        EventResult::consumed().with_redraw().with_pointer_release(key(pointer))
                    }
                    SwipeOutput::Replay(press) => self.replay(context, page, press, event),
                    _ => page_result.unwrap_or_else(EventResult::consumed)
                        .without_capture_request()
                        .with_pointer_release(key(pointer)),
                }
            }
            ElementEvent::Scroll { delta, phase, kind, is_direct_manipulation } => {
                self.handle_scroll(context, page, event, (*delta, *phase, *kind, *is_direct_manipulation), host_width)
            }
            ElementEvent::Cancel | ElementEvent::PointerExited(..) => {
                // Whatever else it was, the system took the trackpad gesture too.
                let now = self.trackpad_now(TouchPhase::Moved);
                self.finish_trackpad(self.trackpad.borrow_mut().cancel(now));
                let was_forwarded = self.forwarding.take();
                let was_watching = self.recognizer.borrow().watched_pointer();
                if let SwipeOutput::Release { commit } = self.recognizer.borrow_mut().cancel() {
                    if let Some(swipe) = self.active.borrow_mut().take() {
                        swipe.release(commit);
                    }
                    aimer_events::window::request_animation_frame();
                }
                if let Some(key) = was_forwarded.or(was_watching) {
                    // The page holds this touch, and is told what the system said.
                    return context.cancel_pointer(key).without_capture_request();
                }
                EventResult::ignored()
            }
            _ => self.pass_through(context, page, pos, event, host_left, host_width),
        }
    }

    /// Seconds into the trackpad gesture at hand: the clock, counted from the
    /// start of the gesture.
    fn trackpad_now(&self, phase: TouchPhase) -> f32 {
        let clock = (self.clock)();
        if matches!(phase, TouchPhase::Started) && !self.trackpad.borrow().is_swiping() {
            self.trackpad_epoch.set(clock);
        }
        (clock - self.trackpad_epoch.get()) as f32
    }

    /// Whether the host stands in front of its page for the scroll now being routed.
    ///
    /// A scroll that nothing used is absorbed by the first opaque element it
    /// reaches, which is nearly every page root, so it never gets back out to the
    /// host. The host therefore takes it first and hands it to the page itself,
    /// which is what lets it see the part the page left.
    pub(crate) fn hides_page_for_scroll(&self) -> bool {
        self.enabled.get() && aimer_events::pointer::is_routing_scroll()
    }

    /// A scroll, which may be the fingers of a trackpad dragging the page sideways.
    ///
    /// The page gets every scroll first, as it would without a swipe, and keeps
    /// what it uses: a list that moved, a horizontal scroller that still can. A
    /// gesture the page used is the page's to the end. What the page left, or only
    /// absorbed, is what a rightward two-finger drag is made of, and it pops the
    /// page, following the fingers and finishing where they lift. Once it has
    /// begun it is the host's alone: the page is not scrolled by the rest of it,
    /// nor by the glide after the fingers lift.
    fn handle_scroll(
        &self,
        context: &mut EventDispatchContext<'_, '_>,
        page: &dyn Element,
        event: &ElementEvent,
        (delta, phase, kind, direct): (Vec2d, TouchPhase, ScrollDeltaKind, bool),
        host_width: f32,
    ) -> EventResult {
        let pos = context.position();
        let now = self.trackpad_now(phase);
        if !self.enabled.get() {
            // Nowhere to go back to: a swipe that was going is let go, and the page
            // has the scroll as ever.
            self.finish_trackpad(self.trackpad.borrow_mut().cancel(now));
            return context.dispatch_child(page, pos, event);
        }
        if kind != ScrollDeltaKind::Pixel {
            // A wheel with notches, or the first pulses of a trackpad gesture, which
            // the smoother cannot yet tell from one and reports as lines, between
            // the pixel steps that follow. They are no part of a swipe, and no
            // reason to let go of one; but while one is going the page is not
            // scrolled by the gesture it is part of.
            if self.trackpad.borrow().is_swiping() {
                // The one thing a line can say about a swipe is that the fingers
                // lifted: after a quarter of a second without a pixel step the
                // smoother forgets which device the gesture came from, and reports
                // its end as the end of a wheel.
                let ended = matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) || !direct;
                if ended {
                    let output = self.trackpad.borrow_mut().scroll(Vec2d::ZERO, phase, direct, now, host_width);
                    return self.apply_trackpad(output, host_width, now, EventResult::consumed());
                }
                return EventResult::consumed();
            }
            return context.dispatch_child(page, pos, event);
        }
        // A finger on the glass is already popping this page.
        if self.active.borrow().is_some() && !self.trackpad.borrow().is_swiping() {
            return context.dispatch_child(page, pos, event);
        }
        // A swipe, or the glide that follows its release, is the host's.
        let owned = {
            let swipe = self.trackpad.borrow();
            swipe.is_swiping() || (!direct && swipe.holds(now))
        };
        if owned {
            let output = self.trackpad.borrow_mut().scroll(delta, phase, direct, now, host_width);
            return self.apply_trackpad(output, host_width, now, EventResult::consumed());
        }
        // Anything else is the page's first. A scroller that moved says so; the
        // result alone cannot tell, because the page is reached through a retained
        // child that reports a scroll that moved something and one that was merely
        // absorbed alike.
        let _ = aimer_events::pointer::take_scroll_used();
        let page_result = context.dispatch_child(page, pos, event);
        let used = aimer_events::pointer::take_scroll_used()
            || (page_result.is_consumed() && page_result.needs_redraw());
        if used {
            self.trackpad.borrow_mut().yield_to_page();
            return page_result;
        }
        let output = self.trackpad.borrow_mut().scroll(delta, phase, direct, now, host_width);
        self.apply_trackpad(output, host_width, now, page_result)
    }

    /// Carries out what the trackpad recognizer decided. `otherwise` is the answer
    /// when it has nothing to do: the page's own, or the host's if it holds the
    /// glide.
    fn apply_trackpad(&self, output: TrackpadOutput, host_width: f32, now: f32, otherwise: EventResult) -> EventResult {
        match output {
            TrackpadOutput::None => {
                if self.trackpad.borrow().holds(now) { EventResult::consumed() } else { otherwise }
            }
            TrackpadOutput::Begin => {
                let swipe = (self.begin.borrow())();
                let Some(swipe) = swipe else {
                    // Nothing to play, so the rest of this gesture is the page's.
                    self.trackpad.borrow_mut().decline();
                    return otherwise;
                };
                swipe.drag(self.trackpad.borrow().progress(host_width));
                *self.active.borrow_mut() = Some(swipe);
                aimer_events::window::request_animation_frame();
                EventResult::consumed().with_redraw()
            }
            TrackpadOutput::Update(progress) => {
                if let Some(swipe) = self.active.borrow().as_ref() {
                    swipe.drag(progress);
                }
                aimer_events::window::request_animation_frame();
                EventResult::consumed().with_redraw()
            }
            release @ TrackpadOutput::Release { .. } => {
                self.finish_trackpad(release);
                EventResult::consumed().with_redraw()
            }
        }
    }

    /// Asks, between events, whether a trackpad swipe that has lifted has run out of
    /// glide: the host calls this every frame it presents a swipe. A glide that is
    /// quiet for a moment, or has gone on too long, is over even if the platform
    /// never reported its end, and the swipe is let go where it got to.
    pub(crate) fn poll_trackpad(&self, host_width: f32) {
        let now = self.trackpad_now(TouchPhase::Moved);
        let output = self.trackpad.borrow_mut().poll(now, host_width);
        self.finish_trackpad(output);
    }

    /// Lets the swipe go if the recognizer says it ended.
    fn finish_trackpad(&self, output: TrackpadOutput) {
        if let TrackpadOutput::Release { commit } = output {
            if let Some(swipe) = self.active.borrow_mut().take() {
                swipe.release(commit);
            }
            aimer_events::window::request_animation_frame();
        }
    }

    /// An event the swipe has no interest in. If the host hid the page from hit
    /// testing for it, the page still has to get it, from here.
    fn pass_through(
        &self,
        context: &mut EventDispatchContext<'_, '_>,
        page: &dyn Element,
        pos: Vec2d,
        event: &ElementEvent,
        host_left: f32,
        host_width: f32,
    ) -> EventResult {
        if self.hides_page(pos, host_left, host_width) {
            context.dispatch_child(page, pos, event)
        } else {
            EventResult::ignored()
        }
    }

    /// The touch was not a swipe: the page gets the press that was held back,
    /// then the event that settled it, and every event after.
    fn replay(
        &self,
        context: &mut EventDispatchContext<'_, '_>,
        page: &dyn Element,
        press: PointerInfo,
        event: &ElementEvent,
    ) -> EventResult {
        let key = PointerKey::new(press.source, press.id);
        let _ = context.dispatch_child(page, press.pos, &ElementEvent::PointerDown(press));
        let ends = matches!(event, ElementEvent::PointerUp(_));
        if !ends {
            self.forwarding.set(Some(key));
        }
        let at = event.get_pointer_pos().unwrap_or(press.pos);
        let result = self.forward(context, page, at, event, key);
        if ends { result.without_capture_request().with_pointer_release(key) } else { result }
    }

    /// Gives `event` to the page, keeping the host as the touch's owner so the
    /// rest of the touch still comes through here.
    fn forward(
        &self,
        context: &mut EventDispatchContext<'_, '_>,
        page: &dyn Element,
        at: Vec2d,
        event: &ElementEvent,
        key: PointerKey,
    ) -> EventResult {
        let result = context.dispatch_child(page, at, event);
        // The page's own answer stands; the host only makes sure it hears the rest.
        result.without_capture_request().with_pointer_capture(key)
    }
}

/// Thresholds of the swipe, in logical pixels and seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SwipeConfig {
    /// Width of the zone along the host's left edge where a swipe may begin and
    /// always wins: a press there is held back from the page until decided.
    pub edge: f32,
    /// Travel before a touch in the edge zone is judged a swipe or not.
    pub slop: f32,
    /// Fraction of the host's width past which letting go completes the pop.
    pub commit_fraction: f32,
    /// Rightward speed at which letting go completes the pop at any distance,
    /// and leftward speed at which it cancels at any distance.
    pub fling_velocity: f32,
    /// Whether a touch anywhere else may become a swipe too.
    ///
    /// Such a touch goes to the page at once, so taps and scrolls behave as they
    /// always did, and is only *watched*: it becomes a swipe if it travels
    /// clearly rightward and nothing on the page took it.
    pub anywhere: bool,
    /// Travel before a watched touch is judged.
    pub watch_slop: f32,
    /// How much more sideways than down a watched touch must travel to be a
    /// swipe. An edge swipe needs only to head right more than anywhere else; a
    /// touch in the middle of a page is usually scrolling it, and a thumb drifts.
    pub watch_ratio: f32,
}

impl Default for SwipeConfig {
    fn default() -> Self {
        Self {
            edge: 20.0,
            slop: 8.0,
            commit_fraction: 0.5,
            fling_velocity: 600.0,
            anywhere: true,
            // Beyond the 18 px a gesture detector tolerates and the 10 dp a scroll
            // view waits for, so a child that is dragging gets to claim the touch
            // before this decides it is not.
            watch_slop: 20.0,
            watch_ratio: 1.75,
        }
    }
}

impl SwipeConfig {
    /// Only a touch that begins in the edge zone can be a swipe.
    pub(crate) fn edge_only() -> Self {
        Self { anywhere: false, ..Self::default() }
    }
}

/// What the host does with the press that just arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disposition {
    /// Not ours: deliver it to the page as usual.
    Pass,
    /// Hold it back until the touch declares itself.
    Hold,
    /// Deliver it to the page now, and keep watching the touch: it may turn out
    /// to be a swipe, and then the page is told the touch is cancelled.
    Track,
}

/// What the host does after a movement, release or cancellation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SwipeOutput {
    /// Nothing visible happens.
    None,
    /// The touch is a swipe: build the page beneath and start presenting.
    Begin,
    /// The page follows the finger. Progress is in `0.0..=1.0`.
    Update(f32),
    /// The finger lifted or the touch was cancelled; settle the page.
    Release { commit: bool },
    /// It was not a swipe: give the page the press held back, then this event.
    Replay(PointerInfo),
    /// It was not a swipe, and the page already has the whole touch: carry on
    /// giving it the events.
    Yield,
}

#[derive(Clone, Copy, Debug)]
enum Stage {
    Idle,
    /// Held back; waiting for the touch to move past the slop.
    Pending { press: PointerInfo, width: f32 },
    /// Already with the page; waiting for the touch to declare itself.
    Watching { press: PointerInfo, width: f32 },
    /// A swipe; progress is measured from where the slop was crossed, so the
    /// page does not jump by the slop when it picks up.
    Dragging {
        press: PointerInfo,
        /// Whether the press was held back, and so has still to be given to the
        /// page if the swipe cannot go ahead.
        held: bool,
        id: u64,
        anchor_x: f32,
        width: f32,
        progress: f32,
        prev: (f32, f32),
        last: (f32, f32),
    },
}

/// The swipe state machine for one host.
#[derive(Debug)]
pub(crate) struct EdgeSwipe {
    config: SwipeConfig,
    stage: Stage,
}

impl EdgeSwipe {
    pub(crate) fn new(config: SwipeConfig) -> Self {
        Self { config, stage: Stage::Idle }
    }

    /// Whether no touch is held back, watched or being swiped.
    pub(crate) fn is_idle(&self) -> bool {
        matches!(self.stage, Stage::Idle)
    }

    /// The touch that is with the page and being watched, if any.
    pub(crate) fn watched_pointer(&self) -> Option<PointerKey> {
        match self.stage {
            Stage::Watching { press, .. } => Some(PointerKey::new(press.source, press.id)),
            _ => None,
        }
    }

    /// Whether a touch is with the page and being watched.
    pub(crate) fn is_watching(&self) -> bool {
        matches!(self.stage, Stage::Watching { .. })
    }

    /// Whether a touch outside the edge zone may become a swipe. Takes effect for
    /// the next press.
    pub(crate) fn set_anywhere(&mut self, anywhere: bool) {
        self.config.anywhere = anywhere;
    }

    /// The page took the touch it was being watched on: stop watching.
    pub(crate) fn yield_to_page(&mut self) {
        if matches!(self.stage, Stage::Watching { .. }) {
            self.stage = Stage::Idle;
        }
    }

    /// Gives up a swipe that [began](SwipeOutput::Begin) but cannot be presented,
    /// so the page gets the touch after all.
    pub(crate) fn decline(&mut self) -> SwipeOutput {
        match std::mem::replace(&mut self.stage, Stage::Idle) {
            Stage::Dragging { press, held: true, .. } => SwipeOutput::Replay(press),
            Stage::Dragging { held: false, .. } => SwipeOutput::Yield,
            other => {
                self.stage = other;
                SwipeOutput::None
            }
        }
    }

    /// Whether a swipe is being presented, so the page must not see the touch.
    pub(crate) fn is_dragging(&self) -> bool {
        matches!(self.stage, Stage::Dragging { .. })
    }

    /// Whether the touch with this id belongs to the recognizer.
    pub(crate) fn owns(&self, pointer: &PointerInfo) -> bool {
        match self.stage {
            Stage::Idle => false,
            Stage::Pending { press, .. } | Stage::Watching { press, .. } => {
                press.source == pointer.source && press.id == pointer.id
            }
            Stage::Dragging { id, .. } => pointer.source == PointerSource::Touch && id == pointer.id,
        }
    }

    /// A press. `host_left` and `host_width` locate the host in the window;
    /// `can_pop` is whether there is a page to go back to.
    pub(crate) fn down(&mut self, pointer: PointerInfo, host_left: f32, host_width: f32, can_pop: bool) -> Disposition {
        // One touch at a time: a second finger is the page's, never a swipe's.
        if !matches!(self.stage, Stage::Idle)
            || !can_pop
            || pointer.source != PointerSource::Touch
            || !(host_width.is_finite() && host_width > 0.0)
        {
            return Disposition::Pass;
        }
        let from_edge = pointer.pos.x - host_left;
        if (0.0..=self.config.edge).contains(&from_edge) {
            self.stage = Stage::Pending { press: pointer, width: host_width };
            Disposition::Hold
        } else if self.config.anywhere && from_edge > self.config.edge && from_edge <= host_width {
            self.stage = Stage::Watching { press: pointer, width: host_width };
            Disposition::Track
        } else {
            Disposition::Pass
        }
    }

    /// A movement of any pointer. `time` is in seconds on any monotonic clock.
    pub(crate) fn moved(&mut self, pointer: PointerInfo, time: f32) -> SwipeOutput {
        if !self.owns(&pointer) {
            return SwipeOutput::None;
        }
        match self.stage {
            Stage::Idle => SwipeOutput::None,
            Stage::Pending { press, width } => {
                let dx = pointer.pos.x - press.pos.x;
                let dy = pointer.pos.y - press.pos.y;
                if dx * dx + dy * dy <= self.config.slop * self.config.slop {
                    return SwipeOutput::None;
                }
                // Decided: a swipe only if it heads right more than it heads
                // anywhere else. Everything else was an ordinary touch.
                if dx > 0.0 && dx > dy.abs() {
                    self.begin(press, true, width, pointer, time)
                } else {
                    self.stage = Stage::Idle;
                    SwipeOutput::Replay(press)
                }
            }
            Stage::Watching { press, width } => {
                let dx = pointer.pos.x - press.pos.x;
                let dy = pointer.pos.y - press.pos.y;
                if dx * dx + dy * dy <= self.config.watch_slop * self.config.watch_slop {
                    return SwipeOutput::None;
                }
                if dx > 0.0 && dx >= self.config.watch_ratio * dy.abs() {
                    self.begin(press, false, width, pointer, time)
                } else if dx <= 0.0 || dy.abs() > dx {
                    // Going down, up or left: a scroll, or a drag of the page's own.
                    self.stage = Stage::Idle;
                    SwipeOutput::Yield
                } else {
                    // Rightward but not clearly: neither a swipe nor given up on.
                    SwipeOutput::None
                }
            }
            Stage::Dragging { press, held, id, anchor_x, width, prev, last, .. } => {
                let progress = ((pointer.pos.x - anchor_x) / width).clamp(0.0, 1.0);
                // Velocity is measured over at least `VELOCITY_WINDOW`, so that
                // samples arriving a millisecond apart do not read as noise.
                let prev = if time - last.1 >= VELOCITY_WINDOW { last } else { prev };
                self.stage = Stage::Dragging {
                    press, held, id, anchor_x, width, progress, prev, last: (pointer.pos.x, time),
                };
                SwipeOutput::Update(progress)
            }
        }
    }

    fn begin(&mut self, press: PointerInfo, held: bool, width: f32, pointer: PointerInfo, time: f32) -> SwipeOutput {
        let x = pointer.pos.x;
        self.stage = Stage::Dragging {
            press, held, id: pointer.id, anchor_x: x, width, progress: 0.0, prev: (x, time), last: (x, time),
        };
        SwipeOutput::Begin
    }

    /// The pointer lifted.
    pub(crate) fn up(&mut self, pointer: PointerInfo, time: f32) -> SwipeOutput {
        if !self.owns(&pointer) {
            return SwipeOutput::None;
        }
        match std::mem::replace(&mut self.stage, Stage::Idle) {
            Stage::Idle => SwipeOutput::None,
            Stage::Pending { press, .. } => SwipeOutput::Replay(press),
            Stage::Watching { .. } => SwipeOutput::Yield,
            Stage::Dragging { progress, prev, last, .. } => {
                // A finger that rested before lifting is not a flick.
                let velocity = if time - last.1 > STALE_AFTER || last.1 <= prev.1 {
                    0.0
                } else {
                    (last.0 - prev.0) / (last.1 - prev.1)
                };
                let commit = if velocity >= self.config.fling_velocity {
                    true
                } else if velocity <= -self.config.fling_velocity {
                    false
                } else {
                    progress >= self.config.commit_fraction
                };
                SwipeOutput::Release { commit }
            }
        }
    }

    /// The touch was taken away by the system or left the window.
    pub(crate) fn cancel(&mut self) -> SwipeOutput {
        match std::mem::replace(&mut self.stage, Stage::Idle) {
            Stage::Dragging { .. } => SwipeOutput::Release { commit: false },
            // Held back, the page never saw the press; watched, it is told by the
            // same cancellation. Either way there is nothing to undo here.
            Stage::Idle | Stage::Pending { .. } | Stage::Watching { .. } => SwipeOutput::None,
        }
    }
}

/// Shortest span the release velocity is measured over, in seconds.
const VELOCITY_WINDOW: f32 = 0.016;
/// Age of the last sample, in seconds, past which the finger counts as at rest.
const STALE_AFTER: f32 = 0.1;

#[cfg(test)]
mod tests {
    use aimer_events::pointer::PointerButton;

    use super::*;

    const WIDTH: f32 = 400.0;

    fn touch(id: u64, x: f32, y: f32) -> PointerInfo {
        PointerInfo::new(Vec2d { x, y }, PointerSource::Touch, id, PointerButton::Primary)
    }

    fn swipe() -> EdgeSwipe {
        EdgeSwipe::new(SwipeConfig::edge_only())
    }

    fn anywhere() -> EdgeSwipe {
        EdgeSwipe::new(SwipeConfig::default())
    }

    /// Presses at the edge and drags to `x` at `y`, ending just after the slop.
    fn begun() -> EdgeSwipe {
        let mut swipe = swipe();
        assert_eq!(swipe.down(touch(1, 5.0, 300.0), 0.0, WIDTH, true), Disposition::Hold);
        assert_eq!(swipe.moved(touch(1, 20.0, 302.0), 0.00), SwipeOutput::Begin);
        swipe
    }

    #[test]
    fn a_press_outside_the_edge_zone_belongs_to_the_page() {
        let mut swipe = swipe();
        assert_eq!(swipe.down(touch(1, 21.0, 300.0), 0.0, WIDTH, true), Disposition::Pass);
        assert!(!swipe.owns(&touch(1, 21.0, 300.0)));
    }

    #[test]
    fn the_zone_follows_the_host_not_the_window() {
        let mut swipe = swipe();
        assert_eq!(swipe.down(touch(1, 105.0, 0.0), 100.0, WIDTH, true), Disposition::Hold);
        let mut swipe = EdgeSwipe::new(SwipeConfig::default());
        assert_eq!(swipe.down(touch(1, 5.0, 0.0), 100.0, WIDTH, true), Disposition::Pass);
    }

    #[test]
    fn only_a_touch_that_can_pop_starts_a_swipe() {
        let mut swipe = swipe();
        assert_eq!(swipe.down(touch(1, 5.0, 0.0), 0.0, WIDTH, false), Disposition::Pass);
        let mouse = PointerInfo::new(Vec2d { x: 5.0, y: 0.0 }, PointerSource::Mouse, 0, PointerButton::Primary);
        assert_eq!(swipe.down(mouse, 0.0, WIDTH, true), Disposition::Pass);
        assert_eq!(swipe.down(touch(1, 5.0, 0.0), 0.0, 0.0, true), Disposition::Pass, "a host with no width has nothing to swipe");
    }

    #[test]
    fn movement_inside_the_slop_decides_nothing() {
        let mut swipe = swipe();
        swipe.down(touch(1, 5.0, 300.0), 0.0, WIDTH, true);
        assert_eq!(swipe.moved(touch(1, 11.0, 304.0), 0.01), SwipeOutput::None);
        assert!(swipe.owns(&touch(1, 11.0, 304.0)));
    }

    #[test]
    fn a_rightward_horizontal_drag_past_the_slop_begins_a_swipe_and_progress_starts_at_zero() {
        let mut swipe = begun();
        assert!(swipe.is_dragging());
        assert_eq!(swipe.moved(touch(1, 20.0, 302.0), 0.02), SwipeOutput::Update(0.0));
        assert_eq!(swipe.moved(touch(1, 120.0, 302.0), 0.04), SwipeOutput::Update(0.25));
    }

    #[test]
    fn progress_is_clamped_to_the_host() {
        let mut swipe = begun();
        assert_eq!(swipe.moved(touch(1, 10_000.0, 302.0), 0.05), SwipeOutput::Update(1.0));
        assert_eq!(swipe.moved(touch(1, -500.0, 302.0), 0.06), SwipeOutput::Update(0.0));
    }

    #[test]
    fn a_vertical_leftward_or_diagonal_drag_is_replayed_to_the_page() {
        for (x, y) in [(6.0, 330.0), (-20.0, 300.0), (30.0, 330.0)] {
            let mut swipe = swipe();
            let press = touch(1, 5.0, 300.0);
            swipe.down(press, 0.0, WIDTH, true);
            assert_eq!(swipe.moved(touch(1, x, y), 0.02), SwipeOutput::Replay(press), "({x}, {y})");
            assert!(!swipe.owns(&press), "the page owns the touch from here on");
        }
    }

    #[test]
    fn lifting_before_the_slop_replays_the_press_so_it_is_still_a_tap() {
        let mut swipe = swipe();
        let press = touch(1, 5.0, 300.0);
        swipe.down(press, 0.0, WIDTH, true);
        assert_eq!(swipe.up(touch(1, 5.0, 300.0), 0.05), SwipeOutput::Replay(press));
        assert!(!swipe.owns(&press));
    }

    #[test]
    fn letting_go_past_half_commits_and_short_of_it_cancels() {
        for (x, commit) in [(220.0, true), (150.0, false)] {
            let mut swipe = begun();
            swipe.moved(touch(1, x, 302.0), 1.0);
            assert_eq!(swipe.up(touch(1, x, 302.0), 2.0), SwipeOutput::Release { commit }, "x = {x}");
            assert!(!swipe.is_dragging());
        }
    }

    #[test]
    fn a_fast_flick_decides_regardless_of_distance() {
        // 40 px in 40 ms is 1000 px/s.
        let mut swipe = begun();
        swipe.moved(touch(1, 40.0, 302.0), 0.10);
        swipe.moved(touch(1, 80.0, 302.0), 0.14);
        assert_eq!(swipe.up(touch(1, 80.0, 302.0), 0.14), SwipeOutput::Release { commit: true });

        let mut swipe = begun();
        swipe.moved(touch(1, 300.0, 302.0), 0.10);
        swipe.moved(touch(1, 260.0, 302.0), 0.14);
        assert_eq!(swipe.up(touch(1, 260.0, 302.0), 0.14), SwipeOutput::Release { commit: false });
    }

    #[test]
    fn a_touch_taken_away_cancels_the_swipe() {
        let mut swipe = begun();
        swipe.moved(touch(1, 350.0, 302.0), 0.10);
        assert_eq!(swipe.cancel(), SwipeOutput::Release { commit: false });
        assert!(!swipe.is_dragging());
        assert_eq!(swipe.cancel(), SwipeOutput::None, "nothing left to cancel");
    }

    #[test]
    fn anywhere_a_press_outside_the_edge_zone_goes_to_the_page_at_once_and_is_watched() {
        let mut swipe = anywhere();
        assert_eq!(swipe.down(touch(1, 200.0, 300.0), 0.0, WIDTH, true), Disposition::Track);
        assert!(swipe.owns(&touch(1, 200.0, 300.0)));
        assert!(!swipe.is_idle());
    }

    #[test]
    fn anywhere_the_edge_zone_still_holds_the_press_back_so_the_swipe_always_wins_there() {
        let mut swipe = anywhere();
        assert_eq!(swipe.down(touch(1, 5.0, 300.0), 0.0, WIDTH, true), Disposition::Hold);
    }

    #[test]
    fn anywhere_still_needs_a_touch_a_page_to_go_back_to_and_a_host_with_width() {
        let mut swipe = anywhere();
        assert_eq!(swipe.down(touch(1, 200.0, 0.0), 0.0, WIDTH, false), Disposition::Pass);
        let mouse = PointerInfo::new(Vec2d { x: 200.0, y: 0.0 }, PointerSource::Mouse, 0, PointerButton::Primary);
        assert_eq!(swipe.down(mouse, 0.0, WIDTH, true), Disposition::Pass);
        assert_eq!(swipe.down(touch(1, 200.0, 0.0), 0.0, 0.0, true), Disposition::Pass);
    }

    #[test]
    fn a_watched_touch_that_moves_a_little_is_still_undecided() {
        let mut swipe = anywhere();
        swipe.down(touch(1, 200.0, 300.0), 0.0, WIDTH, true);
        assert_eq!(swipe.moved(touch(1, 207.0, 303.0), 0.01), SwipeOutput::None);
        assert!(!swipe.is_idle());
    }

    #[test]
    fn a_clearly_rightward_drag_of_a_watched_touch_begins_the_swipe_from_that_point() {
        let mut swipe = anywhere();
        swipe.down(touch(1, 200.0, 300.0), 0.0, WIDTH, true);
        assert_eq!(swipe.moved(touch(1, 230.0, 305.0), 0.02), SwipeOutput::Begin);
        assert!(swipe.is_dragging());
        // Progress is measured from where it began, so the page does not jump.
        assert_eq!(swipe.moved(touch(1, 330.0, 305.0), 0.04), SwipeOutput::Update(0.25));
    }

    #[test]
    fn a_watched_touch_that_goes_vertical_or_left_is_the_pages_for_good() {
        for (x, y) in [(203.0, 340.0), (160.0, 300.0), (200.0, 260.0)] {
            let mut swipe = anywhere();
            swipe.down(touch(1, 200.0, 300.0), 0.0, WIDTH, true);
            assert_eq!(swipe.moved(touch(1, x, y), 0.02), SwipeOutput::Yield, "({x}, {y})");
            assert!(swipe.is_idle(), "({x}, {y}): nothing more to watch");
            assert!(!swipe.owns(&touch(1, x, y)));
        }
    }

    #[test]
    fn a_diagonal_drag_is_not_decided_either_way() {
        // Rightward, but not clearly more sideways than down: a list scrolled with a
        // drifting thumb must not turn into a back swipe, nor be given up on yet.
        let mut swipe = anywhere();
        swipe.down(touch(1, 200.0, 300.0), 0.0, WIDTH, true);
        assert_eq!(swipe.moved(touch(1, 222.0, 322.0), 0.02), SwipeOutput::None);
        assert!(!swipe.is_idle());
    }

    #[test]
    fn lifting_a_watched_touch_hands_it_back_with_nothing_to_replay() {
        let mut swipe = anywhere();
        swipe.down(touch(1, 200.0, 300.0), 0.0, WIDTH, true);
        assert_eq!(swipe.up(touch(1, 201.0, 300.0), 0.05), SwipeOutput::Yield, "a tap: the page already has the press");
        assert!(swipe.is_idle());
    }

    #[test]
    fn a_page_that_took_the_touch_ends_the_watch() {
        let mut swipe = anywhere();
        swipe.down(touch(1, 200.0, 300.0), 0.0, WIDTH, true);
        swipe.yield_to_page();
        assert!(swipe.is_idle());
        assert_eq!(swipe.moved(touch(1, 260.0, 300.0), 0.02), SwipeOutput::None, "not ours any more");
    }

    #[test]
    fn declining_a_swipe_that_began_from_a_watch_does_not_replay_a_press_the_page_already_has() {
        let mut swipe = anywhere();
        swipe.down(touch(1, 200.0, 300.0), 0.0, WIDTH, true);
        assert_eq!(swipe.moved(touch(1, 230.0, 305.0), 0.02), SwipeOutput::Begin);
        assert_eq!(swipe.decline(), SwipeOutput::Yield);
        assert!(swipe.is_idle());
    }

    #[test]
    fn a_watched_swipe_commits_and_cancels_like_an_edge_one() {
        for (to, commit) in [(430.0, true), (300.0, false)] {
            let mut swipe = anywhere();
            swipe.down(touch(1, 200.0, 300.0), 0.0, WIDTH, true);
            swipe.moved(touch(1, 230.0, 304.0), 0.02);
            swipe.moved(touch(1, to, 304.0), 1.0);
            assert_eq!(swipe.up(touch(1, to, 304.0), 2.0), SwipeOutput::Release { commit }, "released at x = {to}");
        }
    }

    #[test]
    fn declining_a_begun_swipe_gives_the_press_back_to_the_page() {
        let mut swipe = swipe();
        let press = touch(1, 5.0, 300.0);
        swipe.down(press, 0.0, WIDTH, true);
        assert_eq!(swipe.moved(touch(1, 20.0, 302.0), 0.0), SwipeOutput::Begin);
        assert_eq!(swipe.decline(), SwipeOutput::Replay(press));
        assert!(swipe.is_idle());
        assert_eq!(swipe.decline(), SwipeOutput::None, "nothing left to decline");
    }

    #[test]
    fn the_recognizer_is_idle_until_a_touch_is_held_and_again_once_it_is_decided() {
        let mut swipe = swipe();
        assert!(swipe.is_idle());
        swipe.down(touch(1, 5.0, 300.0), 0.0, WIDTH, true);
        assert!(!swipe.is_idle());
        swipe.up(touch(1, 5.0, 300.0), 0.1);
        assert!(swipe.is_idle());
    }

    #[test]
    fn other_fingers_neither_start_nor_disturb_a_swipe() {
        let mut swipe = begun();
        assert!(!swipe.owns(&touch(2, 5.0, 100.0)));
        assert_eq!(swipe.down(touch(2, 5.0, 100.0), 0.0, WIDTH, true), Disposition::Pass);
        assert_eq!(swipe.moved(touch(2, 300.0, 100.0), 0.05), SwipeOutput::None);
        assert_eq!(swipe.up(touch(2, 300.0, 100.0), 0.06), SwipeOutput::None);
        assert!(swipe.is_dragging());
    }
}
