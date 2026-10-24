use std::cell::{Cell, RefCell, UnsafeCell};
use std::rc::Rc;

use aimer_animation::{AnimInstant, AnimatedBuilder, AnimationController, Curve, PresentationClip, curves};
use aimer_cupid::utilities::Rect;
use aimer_widget::base::{BuildContext, ResolvedSize, Size, Vec2d};
use aimer_events::element::ElementEvent;
use aimer_widget::{AnyElement, AnyWidget, ChildBuilder, Drawable, Element, EventDispatchContext, EventElement, EventResult,
    Key, LayoutElement, Rebuildable, VisitorElement, Widget, request_isolated_animation_frame};

use aimer_attribute::Dimension;
use aimer_container::SizedBox;

use crate::outlet::RouteChildContext;
use crate::swipe_back::SwipeLink;
use crate::{AnchoredFrame, NavigationAction, NavigationContext, NavigationTransition,
    NavigationTransitionFrame, PaintPresentation, ResolvedNavigationTransition, Route,
    TransitionChoice, TransitionPage};

/// Ambient operation shared with nested shell outlets, independently of their route type.
#[derive(Clone, Copy, Default)]
pub(crate) struct NavigationRequest(pub NavigationAction);

#[derive(Clone)]
pub(crate) struct RouteBoundary(pub Key);

/// What a navigator (or a shell's outlet) hands its pages when it builds: the
/// link every host of it shares, and a swipe to start, if one was asked for.
pub(crate) struct SwipeSetup<R> {
    /// `None` for an outlet, whose touches belong to the navigator around it.
    pub(crate) link: Option<Rc<SwipeLink>>,
    pub(crate) start: Option<(R, Rc<SwipeDrive>)>,
}

pub(crate) struct NavigationPages<R: Route> {
    current: Option<(R, String, ChildBuilder)>,
    playback: Option<Rc<Playback>>,
    /// The URL of the page a swipe is uncovering, so that when the swipe commits
    /// and history pops to it, the page already built is adopted.
    swipe_target: Option<String>,
}

impl<R: Route> Default for NavigationPages<R> {
    fn default() -> Self { Self { current: None, playback: None, swipe_target: None } }
}

impl<R: Route> NavigationPages<R> {
    pub(crate) fn build(
        &mut self,
        route: R,
        action: NavigationAction,
        fallback: Option<&dyn NavigationTransition<R>>,
        ctx: &BuildContext,
        swipe: Option<SwipeSetup<R>>,
        build: impl Fn(R) -> AnyWidget,
    ) -> NavigationFrame {
        let path = route.format();
        let (link, starting) = match swipe {
            Some(SwipeSetup { link, start }) => (link, start),
            None => (None, None),
        };
        let starting_swipe = starting.is_some();
        if self.current.as_ref().is_none_or(|(_, old_path, _)| *old_path != path) {
            // Before the playback is finished, which would discard it: a swipe
            // that committed has already built the page history is popping to.
            let adopted = self.take_committed_underlay(&path);
            self.swipe_target = None;
            if let Some(playback) = self.playback.take() { playback.finish(); }
            let effect = self.current.as_ref().filter(|_| adopted.is_none()).and_then(|(from, _, _)| {
                // A persistent shell delegates child motion to its own outlet.
                if from.shell_key().is_some() && from.shell_key() == route.shell_key() { return None; }
                let navigation = NavigationContext { from, to: &route, action, build_context: ctx };
                let owner = if action == NavigationAction::Pop { from } else { &route };
                match owner.navigation_transition() {
                    TransitionChoice::Disabled => None,
                    TransitionChoice::Inherit => fallback.and_then(|effect| effect.resolve(&navigation)),
                    TransitionChoice::Custom(effect) => effect.resolve(&navigation),
                }
            }).filter(|effect| !effect.duration.is_zero());
            // Ordinary route entries are distinct even when their widget types
            // match. Shell entries intentionally share a boundary identity.
            let key = self.current.as_ref().filter(|(from, _, _)|
                route.shell_key().is_some() && route.shell_key() == from.shell_key())
                .and_then(|(_, _, child)| child.key()).unwrap_or_else(Key::unique);
            let child = adopted.unwrap_or_else(|| ChildBuilder::from_widget(ScopedPage {
                widget: build(route.clone()), context: RouteChildContext::capture_page(ctx, key.clone()), key,
            }));
            let previous = self.current.replace((route, path, child));
            self.playback = effect.and_then(|effect| previous.map(|(_, _, child)| {
                let controller = AnimationController::new(effect.duration, effect.curve.clone());
                controller.forward_from_first_tick();
                aimer_events::window::request_animation_frame();
                Rc::new(Playback { effect, controller, outgoing: RefCell::new(Some(child)), finished: Cell::new(false), swipe: None })
            }));
        } else if !starting_swipe {
            // An explicit parent rebuild must refresh route configuration
            // (theme/provider reads and new builder props). Keep this entry's
            // identity so reconciliation carries its state and active motion.
            let key = self.current.as_ref().and_then(|(_, _, child)| child.key()).expect("route pages have keys");
            let child = ChildBuilder::from_widget(ScopedPage {
                widget: build(route.clone()), context: RouteChildContext::capture_page(ctx, key.clone()), key,
            });
            self.current = Some((route, path, child));
        }
        // Starting a swipe rebuilds the navigator only to show the page beneath.
        // Nothing the page reads has changed, so it is reused as it is rather
        // than rebuilt under the user's finger.
        if let Some((previous, drive)) = starting {
            if !self.begin_swipe(previous, &drive, fallback, ctx, &build) { drive.cancel_if_unsettled(); }
        }
        NavigationFrame {
            child: self.current.as_ref().expect("a page was installed").2.clone(),
            playback: self.playback.clone().filter(|playback| !playback.finished.get()),
            swipe: link,
        }
    }

    /// Starts a swipe back: builds the page beneath `previous`, and makes the finger the
    /// clock of the pop. Returns `false` when there is nothing to play: no page
    /// is on screen, one is already animating, or the departing route has no
    /// effect (or opted out of it).
    fn begin_swipe(
        &mut self,
        previous: R,
        drive: &Rc<SwipeDrive>,
        fallback: Option<&dyn NavigationTransition<R>>,
        ctx: &BuildContext,
        build: &impl Fn(R) -> AnyWidget,
    ) -> bool {
        if self.playback.as_ref().is_some_and(|playback| !playback.finished.get()) { return false; }
        let Some((from, _, _)) = self.current.as_ref() else { return false };
        // A persistent shell delegates child motion to its own outlet.
        if from.shell_key().is_some() && from.shell_key() == previous.shell_key() { return false; }
        let navigation = NavigationContext { from, to: &previous, action: NavigationAction::Pop, build_context: ctx };
        let effect = match from.navigation_transition() {
            TransitionChoice::Disabled => None,
            TransitionChoice::Inherit => fallback.and_then(|effect| effect.resolve(&navigation)),
            TransitionChoice::Custom(effect) => effect.resolve(&navigation),
        }.filter(|effect| !effect.duration.is_zero());
        let Some(effect) = effect else { return false };
        let key = Key::unique();
        self.swipe_target = Some(previous.format());
        let underlay = ChildBuilder::from_widget(ScopedPage {
            widget: build(previous), context: RouteChildContext::capture_page(ctx, key.clone()), key,
        });
        self.playback = Some(Rc::new(Playback::swipe(effect, Rc::clone(drive), underlay)));
        aimer_events::window::request_animation_frame();
        true
    }

    /// The page a committed swipe uncovered, if `path` is where history went.
    fn take_committed_underlay(&mut self, path: &str) -> Option<ChildBuilder> {
        let playback = self.playback.as_ref()?;
        let committed = playback.swipe.as_ref()?.outcome() == Some(true);
        if !committed || self.swipe_target.as_deref() != Some(path) { return None; }
        playback.outgoing.borrow_mut().take()
    }
}

struct Playback {
    effect: ResolvedNavigationTransition,
    controller: AnimationController,
    outgoing: RefCell<Option<ChildBuilder>>,
    finished: Cell<bool>,
    /// Set when a finger, not the clock, is what moves the pages.
    swipe: Option<Rc<SwipeDrive>>,
}

impl Playback {
    /// A pop driven by an edge swipe.
    ///
    /// The page on screen stays where it is in the tree and is presented as the
    /// departing page; `underlay` is the page it uncovers, held in the slot a
    /// timed navigation keeps its departing page in. Nothing here runs on a clock
    /// until the finger lifts.
    fn swipe(effect: ResolvedNavigationTransition, drive: Rc<SwipeDrive>, underlay: ChildBuilder) -> Self {
        Self {
            // Not a clock: the finger is. It carries the swipe's progress for the
            // effect's layers that are built from a controller, such as a blur, and
            // progress is already eased by the time it gets here, so this one adds
            // no easing of its own.
            controller: AnimationController::new(effect.duration, curves::LINEAR),
            effect,
            outgoing: RefCell::new(Some(underlay)),
            finished: Cell::new(false),
            swipe: Some(drive),
        }
    }

    fn finish(&self) {
        self.finished.set(true);
        self.controller.set_value(1.0);
        if let Some(drive) = &self.swipe {
            // Another navigation got there first: the swipe did not complete.
            drive.cancel_if_unsettled();
        }
        self.outgoing.borrow_mut().take();
    }

    /// Whether the page beneath must outlive this playback until the navigator
    /// has rebuilt: a committed swipe pops, and removing it first would show an
    /// empty frame.
    fn holds_pages_for_rebuild(&self) -> bool {
        self.swipe.as_ref().is_some_and(|drive| drive.outcome() == Some(true))
    }

    fn sample_swipe(&self, drive: &SwipeDrive, now: AnimInstant, size: ResolvedSize, origin: (f32, f32)) -> AnchoredFrame {
        let (progress, done) = drive.progress(now, self.effect.duration, &self.effect.curve);
        // The effect's layers read progress from the controller, as they do when a
        // clock runs it, so they stay where the pages are.
        self.controller.set_value(progress);
        if done && !self.finished.replace(true) {
            // The host drops the page beneath on the first frame that sees the swipe
            // finished. Nothing else is bound to ask for one: a touch keeps frames
            // coming with its own events, but the fingers on a trackpad have lifted
            // long before the release finishes settling.
            aimer_events::window::request_animation_frame();
        }
        // A finished swipe keeps presenting where it ended rather than a blank
        // frame, because the page that stays is still the one on screen until
        // the navigator rebuilds around it.
        self.effect.sample_anchored_at(progress, size, origin).unwrap_or(AnchoredFrame::FINISHED)
    }

    fn sample(&self, now: AnimInstant, size: ResolvedSize) -> NavigationTransitionFrame {
        self.sample_at(now, size, (0.0, 0.0))
    }

    /// Samples for a host whose top-left corner is at `origin` in the window.
    fn sample_at(&self, now: AnimInstant, size: ResolvedSize, origin: (f32, f32)) -> NavigationTransitionFrame {
        self.sample_anchored_at(now, size, origin).frame
    }

    /// Like [`sample_at`](Self::sample_at), with the anchor layer's presentation.
    fn sample_anchored_at(&self, now: AnimInstant, size: ResolvedSize, origin: (f32, f32)) -> AnchoredFrame {
        if let Some(drive) = &self.swipe { return self.sample_swipe(drive, now, size, origin); }
        if self.finished.get() { return AnchoredFrame::FINISHED; }
        let progress = self.controller.tick(now);
        if !self.controller.is_animating() { self.finish(); return AnchoredFrame::FINISHED; }
        self.effect.sample_anchored_at(progress, size, origin).unwrap_or_else(|| {
            self.finish();
            AnchoredFrame::FINISHED
        })
    }
}

/// The finger's side of a swipe-driven pop.
///
/// Whoever owns the touch writes it; the [`Playback`] that presents the pop
/// reads it every frame. They only share this, because the touch outlives the
/// element that first saw it: building the page beneath rebuilds the host.
pub(crate) struct SwipeDrive {
    /// Fraction of the host the page has been dragged across, in `0.0..=1.0`.
    finger: Cell<f32>,
    /// Set when the finger lifts: whether the pop completes.
    release: Cell<Option<bool>>,
    settle: Cell<Option<Settle>>,
    /// Final outcome, once the page has finished settling or the swipe was cut short.
    outcome: Cell<Option<bool>>,
    reported: Cell<bool>,
    /// Pops the navigator's history. Called once, by the host, after a commit.
    on_commit: Box<dyn Fn()>,
}

/// The release animation: a straight run from where the finger lifted.
#[derive(Clone, Copy)]
struct Settle {
    from: f32,
    to: f32,
    started: Option<AnimInstant>,
}

impl SwipeDrive {
    pub(crate) fn new(on_commit: impl Fn() + 'static) -> Rc<Self> {
        Rc::new(Self {
            finger: Cell::new(0.0),
            release: Cell::new(None),
            settle: Cell::new(None),
            outcome: Cell::new(None),
            reported: Cell::new(false),
            on_commit: Box::new(on_commit),
        })
    }

    /// Moves the page to `progress` of the host's width.
    pub(crate) fn drag(&self, progress: f32) {
        if self.release.get().is_none() {
            self.finger.set(progress.clamp(0.0, 1.0));
        }
    }

    /// The finger lifted: settle to the end if `commit`, back to the start if not.
    pub(crate) fn release(&self, commit: bool) {
        if self.release.get().is_none() {
            self.release.set(Some(commit));
        }
    }

    pub(crate) fn outcome(&self) -> Option<bool> {
        self.outcome.get()
    }

    /// The outcome, the first time it is asked for after it is known.
    pub(crate) fn take_settled(&self) -> Option<bool> {
        let outcome = self.outcome.get()?;
        (!self.reported.replace(true)).then_some(outcome)
    }

    pub(crate) fn cancel_if_unsettled(&self) {
        if self.outcome.get().is_none() {
            self.outcome.set(Some(false));
        }
    }

    /// Progress to present at `now`, and whether the swipe is over.
    fn progress(&self, now: AnimInstant, duration: std::time::Duration, curve: &aimer_animation::AnyCurve) -> (f32, bool) {
        if let Some(commit) = self.outcome.get() {
            return (if commit { 1.0 } else { 0.0 }, true);
        }
        let Some(commit) = self.release.get() else { return (self.finger.get(), false); };
        let to = if commit { 1.0 } else { 0.0 };
        let mut settle = self.settle.get().unwrap_or(Settle { from: self.finger.get(), to, started: None });
        let started = *settle.started.get_or_insert(now);
        self.settle.set(Some(settle));
        // The same speed as the full effect, so a short remainder is quick.
        let total = duration.as_secs_f32() * (to - settle.from).abs();
        let elapsed = now.duration_since(started).as_secs_f32();
        if total <= f32::EPSILON || elapsed >= total {
            self.outcome.set(Some(commit));
            return (to, true);
        }
        (settle.from + (to - settle.from) * curve.transform(elapsed / total), false)
    }
}

#[derive(Clone)]
pub(crate) struct NavigationFrame {
    child: ChildBuilder,
    playback: Option<Rc<Playback>>,
    /// The navigator's edge swipe, for a host that has one to give.
    swipe: Option<Rc<SwipeLink>>,
}

impl aimer_widget::PortableWidget for NavigationFrame {}

impl Widget for NavigationFrame {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let outgoing = self.playback.as_ref().and_then(|playback| {
            playback.outgoing.borrow().as_ref().map(|child| child.build(ctx))
        });
        if let Some(child) = &outgoing { clear_departing_focus(child.as_ref(), ctx); }
        // A copy of the item the effect leads with, laid out at the size the item
        // had, which the host then scales above both pages.
        let anchor = self.playback.as_ref().and_then(|playback| playback.effect.anchor()).map(|anchor| {
            let size = anchor.size();
            SizedBox::new().width(Dimension::Px(size.width)).height(Dimension::Px(size.height))
                .child(anchor.build()).to_element(ctx)
        });
        // A layer between the pages, such as a blur of the page underneath, that
        // is rebuilt from the clock on every frame, or from the finger's progress
        // for a swipe, which has no clock.
        let veil = self.playback.as_ref().filter(|_| outgoing.is_some())
            .and_then(|playback| {
                let effect = playback.effect.veil()?.clone();
                // A swipe's controller is set by hand from the finger, never animated,
                // so the layer has to ask to stay in the frame's rebuild pass.
                Some(AnimatedBuilder::new(playback.controller.clone(), move |progress| effect(progress))
                    .live(playback.swipe.is_some())
                    .to_element(ctx))
            });
        NavigationFrameElement {
            incoming: self.child.build(ctx), outgoing: UnsafeCell::new(outgoing), anchor: UnsafeCell::new(anchor),
            veil: UnsafeCell::new(veil),
            playback: self.playback, swipe: self.swipe,
            incoming_clip: PresentationClip::default(), outgoing_clip: PresentationClip::default(),
            anchor_clip: PresentationClip::default(), veil_clip: PresentationClip::default(),
        }.boxed()
    }
}

fn clear_departing_focus(element: &dyn Element, ctx: &BuildContext) {
    element.with_rebuild_context(ctx, &mut |ctx| {
        if let Some(node) = element.focus_node().filter(|node| node.has_focus()) {
            node.set_focused(false);
            if element.event_tree_role() != aimer_widget::EventTreeRole::Transparent {
                let _ = element.on_event(&aimer_events::element::ElementEvent::FocusLost);
            }
        }
        element.event_children(&mut |child| clear_departing_focus(child, ctx));
    });
}

/// Where each layer of a host is in the retained paint order.
struct Ordinals {
    incoming: usize,
    outgoing: usize,
    anchor: usize,
    /// The veil, wherever it is painted; meaningful only while there is one.
    veil: usize,
}

struct NavigationFrameElement {
    incoming: AnyElement,
    outgoing: UnsafeCell<Option<AnyElement>>,
    /// An inert copy of an item, painted above both pages while the effect runs.
    anchor: UnsafeCell<Option<AnyElement>>,
    /// An inert layer painted between the two pages while the effect runs.
    veil: UnsafeCell<Option<AnyElement>>,
    playback: Option<Rc<Playback>>,
    swipe: Option<Rc<SwipeLink>>,
    /// Which pages this host has put a rounded clip on, so a radius that
    /// returns to zero removes only what the host added.
    incoming_clip: PresentationClip,
    outgoing_clip: PresentationClip,
    anchor_clip: PresentationClip,
    veil_clip: PresentationClip,
}

// Safety: element traversal and structural mutation run exclusively on the
// rendering thread. Outgoing children are removed only after child callbacks
// return, never while a visitor holds a reference into the slot.
impl NavigationFrameElement {
    fn outgoing(&self) -> Option<&AnyElement> {
        // Safety: read-only during traversal; cleanup runs at a frame safe point.
        unsafe { (&*self.outgoing.get()).as_ref() }
    }

    fn anchor(&self) -> Option<&AnyElement> {
        // Safety: as for `outgoing`.
        unsafe { (&*self.anchor.get()).as_ref() }
    }

    fn veil(&self) -> Option<&AnyElement> {
        // Safety: as for `outgoing`.
        unsafe { (&*self.veil.get()).as_ref() }
    }

    /// Whether the veil is painted above the pages and the copy of the item.
    fn veil_above(&self) -> bool {
        self.playback.as_ref().is_some_and(|playback| playback.effect.veil_above())
    }

    /// The retained paint ordinals of the pages, the veil between them and the
    /// copy above them, which are painted in that order: the page below, the
    /// veil, the page on top, the copy. A page or layer that is not there takes
    /// no ordinal.
    fn ordinals(&self) -> Ordinals {
        let has_old = self.outgoing().is_some();
        // A veil above everything takes the last ordinal and moves no other.
        let veil = has_old && self.veil().is_some() && !self.veil_above();
        let (incoming, outgoing) = match (has_old, self.outgoing_on_top()) {
            (false, _) => (0, 0),
            (true, false) => (1 + usize::from(veil), 0),
            (true, true) => (0, 1 + usize::from(veil)),
        };
        Ordinals {
            incoming,
            outgoing,
            anchor: usize::from(has_old) + usize::from(veil) + 1,
            // Between the pages it is the second node painted; above everything it
            // follows the pages and the item's copy.
            veil: if self.veil_above() {
                1 + usize::from(has_old) + usize::from(self.anchor().is_some())
            } else {
                1
            },
        }
    }

    fn sample(&self, ctx: &BuildContext, origin: (f32, f32)) -> AnchoredFrame {
        self.playback.as_ref().map_or(AnchoredFrame::FINISHED,
            |playback| playback.sample_anchored_at(AnimInstant::now(), self.logical_size(ctx), origin))
    }

    /// Where the host's top-left corner is in the window, in logical pixels.
    /// Read live, so it follows layout without the effect being resolved again.
    fn window_origin(ctx: &BuildContext) -> (f32, f32) {
        ctx.local_v2_node_handle()
            .and_then(|node| node.world_bounds())
            .map_or((0.0, 0.0), |(x, y, _, _)| (x, y))
    }

    /// Whether the page in the outgoing slot is painted above the one in the
    /// incoming slot.
    ///
    /// A swipe reverses the slots' roles: the page on screen sits in the incoming
    /// slot but is the one presented as departing, so "the departing page on top"
    /// puts the *incoming* slot on top.
    fn outgoing_on_top(&self) -> bool {
        self.playback.as_ref().is_some_and(|playback| playback.effect.outgoing_on_top != playback.swipe.is_some())
    }

    fn is_swipe(&self) -> bool {
        self.playback.as_ref().is_some_and(|playback| playback.swipe.is_some())
    }

    fn logical_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let physical = self.incoming.computed_size(ctx);
        ResolvedSize { width: physical.width / ctx.scale, height: physical.height / ctx.scale }
    }

    fn cleanup(&self) {
        if let Some(drive) = self.playback.as_ref().and_then(|playback| playback.swipe.as_ref()) {
            if drive.take_settled() == Some(true) {
                // The rebuild this causes replaces the whole host, taking both pages with it.
                (drive.on_commit)();
            }
        }
        if self.playback.as_ref().is_some_and(|playback| playback.holds_pages_for_rebuild()) { return; }
        let finished = self.playback.as_ref().is_some_and(|playback| playback.finished.get());
        if finished && (self.outgoing().is_some() || self.anchor().is_some() || self.veil().is_some()) {
            // Safety: no child traversal or callbacks are active at this point.
            unsafe {
                (*self.outgoing.get()).take();
                (*self.anchor.get()).take();
                (*self.veil.get()).take();
            }
            aimer_widget::notify_hosted_element_tree_changed();
            aimer_events::window::request_animation_frame();
        }
    }
}

impl VisitorElement for NavigationFrameElement {
    fn debug_name(&self) -> &'static str { "NavigationTransitionHost" }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // Stable structural order keeps the incoming page in slot zero.
        visitor(self.incoming.as_ref());
        if let Some(child) = self.outgoing() { visitor(child.as_ref()); }
        if let Some(child) = self.anchor() { visitor(child.as_ref()); }
        if let Some(child) = self.veil() { visitor(child.as_ref()); }
    }
    fn visit_retained_v2_children<'a>(&'a self, visitor: &mut dyn FnMut(usize, &'a dyn Element)) {
        // Later is painted above. The structural slot is fixed (arriving page 0,
        // departing page 1, item copy 2, veil 3); only the paint order changes:
        // the page below, the veil, the page on top. The copy always leads, so
        // it is always last.
        let outgoing = self.outgoing();
        let on_top = self.outgoing_on_top();
        let veil = self.veil().filter(|_| outgoing.is_some());
        let (between, above) = if self.veil_above() { (None, veil) } else { (veil, None) };
        if let Some(child) = outgoing.filter(|_| !on_top) { visitor(1, child.as_ref()); }
        if on_top { visitor(0, self.incoming.as_ref()); }
        if let Some(child) = between { visitor(3, child.as_ref()); }
        if !on_top { visitor(0, self.incoming.as_ref()); }
        if let Some(child) = outgoing.filter(|_| on_top) { visitor(1, child.as_ref()); }
        if let Some(child) = self.anchor() { visitor(2, child.as_ref()); }
        if let Some(child) = above { visitor(3, child.as_ref()); }
    }
}

impl Drawable for NavigationFrameElement {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool { ctx.scale.is_finite() && ctx.scale > 0.0 }
    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        // A trackpad swipe glides on after the fingers lift, and nothing says when it
        // stops but the events drying up, so every frame asks whether they have.
        if self.is_swipe() {
            if let Some(link) = &self.swipe {
                link.poll_trackpad(self.horizontal_extent().1);
            }
        }
        let frame = self.sample(ctx, Self::window_origin(ctx));
        if self.playback.as_ref().is_some_and(|playback| !playback.finished.get()) {
            request_isolated_animation_frame(ctx);
        }
        // Indices are retained visitation ordinals, not structural source indices:
        // the page painted first is ordinal 0.
        let has_old = self.outgoing().is_some();
        let ordinals = self.ordinals();
        let (outgoing_ordinal, incoming_ordinal) = (ordinals.outgoing, ordinals.incoming);
        // Both pages are as large as the host, which is what a corner radius rounds.
        let size = self.logical_size(ctx);
        // During a swipe the page on screen is the departing one.
        let (outgoing_slot, incoming_slot) = if self.is_swipe() {
            (frame.frame.incoming, frame.frame.outgoing)
        } else {
            (frame.frame.outgoing, frame.frame.incoming)
        };
        if has_old && !outgoing_slot.apply_to_child(ctx, outgoing_ordinal, size, &self.outgoing_clip) { return false; }
        if !incoming_slot.apply_to_child(ctx, incoming_ordinal, size, &self.incoming_clip) { return false; }
        // A veil that follows a page is shown as that page is, at full strength: as
        // large as the host like the pages, it is then clipped to where the page is.
        if let Some(page) = self.playback.as_ref().and_then(|playback| playback.effect.veil_page()) {
            if has_old && self.veil().is_some() {
                let followed = match page {
                    TransitionPage::Incoming => frame.frame.incoming,
                    TransitionPage::Outgoing => frame.frame.outgoing,
                };
                let veil = PaintPresentation { opacity: 1.0, ..followed };
                if !veil.apply_to_child(ctx, ordinals.veil, size, &self.veil_clip) { return false; }
            }
        }
        // The item copy is last, and is as large as the item was.
        let Some(anchor) = self.playback.as_ref().and_then(|playback| playback.effect.anchor()) else { return true; };
        if self.anchor().is_none() { return true; }
        frame.anchor.apply_to_child(ctx, ordinals.anchor, anchor.size(), &self.anchor_clip)
    }
    fn retained_clip(&self, ctx: &BuildContext) -> Option<Rect> {
        let playback = self.playback.as_ref()?;
        if playback.finished.get() || !self.can_paint_local_v2(ctx) { return None; }
        let size = self.logical_size(ctx);
        (size.width.is_finite() && size.height.is_finite() && size.width >= 0.0 && size.height >= 0.0)
            .then_some(Rect::new(0.0, 0.0, size.width, size.height))
    }
    fn retained_v2_child_context<'a>(&self, ctx: &BuildContext<'a>, _: &dyn Element) -> Option<BuildContext<'a>> {
        // This is an overlay, not a layout container. With two children the
        // generic walker cannot infer transparency and would constrain padded
        // pages again to their already-reduced content sizes.
        Some(ctx.clone())
    }
    fn update(&self, ctx: &BuildContext) {
        self.sample(ctx, (0.0, 0.0));
        if let Some(child) = self.outgoing() { child.update(ctx); }
        self.incoming.update(ctx);
        if let Some(child) = self.anchor() { child.update(ctx); }
        if let Some(child) = self.veil() { child.update(ctx); }
        if self.playback.as_ref().is_some_and(|playback| !playback.finished.get()) {
            aimer_events::window::request_animation_frame();
        }
        self.cleanup();
    }
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        if let Some(child) = self.outgoing() { child.update(ctx); }
        self.incoming.update(ctx);
        if let Some(child) = self.anchor() { child.update(ctx); }
        if let Some(child) = self.veil() { child.update(ctx); }
        self.cleanup();
    }
}

impl NavigationFrameElement {
    /// Where the host is in the window: its left edge and its width.
    fn horizontal_extent(&self) -> (f32, f32) {
        self.incoming.pos_start_end().map_or((0.0, 0.0), |(start, end)| (start.x, end.x - start.x))
    }
}

impl EventElement for NavigationFrameElement {
    /// A touch that begins at the left edge is the swipe's before it is the
    /// page's: a button or a horizontal scroller there would otherwise take the
    /// press, and no swipe could ever start from it. Only the *page* is hidden
    /// from hit testing, and only there, only for a finger, and only while the
    /// navigator has somewhere to go back to; the host then gets the press.
    fn hit_test_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if let Some(link) = &self.swipe {
            // A scroll goes to the host first, which hands it to the page itself.
            if link.hides_page_for_scroll() { return; }
            let (left, width) = self.horizontal_extent();
            if link.hides_page(pos, left, width) { return; }
        }
        self.hit_test_children(visitor);
    }
    fn on_event_with_context(&self, event: &ElementEvent, context: &mut EventDispatchContext<'_, '_>) -> EventResult {
        let Some(link) = &self.swipe else { return EventResult::ignored(); };
        let (left, width) = self.horizontal_extent();
        link.handle(event, context, self.incoming.as_ref(), left, width)
    }
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) { visitor(self.incoming.as_ref()); }
    /// The page that left and the copy of the item are only visual.
    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) { visitor(self.incoming.as_ref()); }
}
impl Rebuildable for NavigationFrameElement {}
impl LayoutElement for NavigationFrameElement {
    fn pos(&self) -> Option<Vec2d> { self.incoming.pos() }
    fn size(&self) -> Option<Size> { self.incoming.size() }
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize { self.incoming.computed_size(ctx) }
    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        if let Some(child) = self.outgoing() { child.layout(ctx); }
        self.incoming.layout(ctx)
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize { self.incoming.content_size(ctx) }
    fn layer(&self) -> u32 { self.incoming.layer() }
    fn flex(&self) -> Option<f32> { self.incoming.flex() }
    fn get_size_from_child(&self) -> Option<Size> { self.incoming.get_size_from_child() }
    /// The area the host answers to: the union of the pages it presents.
    ///
    /// While two are presented they slide across one another and together always
    /// cover the host. The cursor does not slide with them, and a scroll is routed
    /// by where the cursor is, so a host that covered only the page that is
    /// sliding away would stop being reached once that page had left the cursor
    /// behind, and a trackpad swipe would freeze half way with nothing to end it.
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        let incoming = self.incoming.pos_start_end();
        let Some((outgoing_start, outgoing_end)) = self.outgoing().and_then(|page| page.pos_start_end()) else {
            return incoming;
        };
        let Some((start, end)) = incoming else { return Some((outgoing_start, outgoing_end)) };
        Some((
            Vec2d { x: start.x.min(outgoing_start.x), y: start.y.min(outgoing_start.y) },
            Vec2d { x: end.x.max(outgoing_end.x), y: end.y.max(outgoing_end.y) },
        ))
    }
}

struct ScopedPage { widget: AnyWidget, context: RouteChildContext, key: Key }
impl aimer_widget::PortableWidget for ScopedPage {}
impl Widget for ScopedPage {
    fn key(&self) -> Option<Key> { Some(self.key.clone()) }
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = self.context.with_page(ctx, |ctx| self.widget.to_element(ctx));
        PageElement { child, context: self.context, key: self.key }.boxed()
    }
}
struct PageElement { child: AnyElement, context: RouteChildContext, key: Key }
impl VisitorElement for PageElement {
    fn debug_name(&self) -> &'static str { "RouteContent" }
    fn reconciliation_key(&self) -> Option<&Key> { Some(&self.key) }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) { visitor(self.child.as_ref()); }
}
impl Drawable for PageElement {
    fn can_paint_local_v2(&self, _: &BuildContext) -> bool { true }
    fn update(&self, ctx: &BuildContext) { self.scoped(ctx, |ctx| self.child.update(ctx)); }
    fn retained_v2_child_context<'a>(&self, ctx: &BuildContext<'a>, _: &dyn Element) -> Option<BuildContext<'a>> {
        self.scoped(ctx, |ctx| {
            let mut child = ctx.clone();
            child.inherited_states = Rc::new(RefCell::new(ctx.inherited_states.borrow().clone()));
            Some(child)
        })
    }
    fn sync_paint_geometry(&self, ctx: &BuildContext) { self.scoped(ctx, |ctx| self.child.sync_paint_geometry(ctx)); }
}
impl LayoutElement for PageElement {
    fn pos(&self) -> Option<Vec2d> { self.child.pos() }
    fn size(&self) -> Option<Size> { self.child.size() }
    fn layout(&self, ctx: &BuildContext) -> ResolvedSize { self.scoped(ctx, |ctx| self.child.layout(ctx)) }
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize { self.scoped(ctx, |ctx| self.child.computed_size(ctx)) }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize { self.scoped(ctx, |ctx| self.child.content_size(ctx)) }
    fn layer(&self) -> u32 { self.child.layer() }
    fn flex(&self) -> Option<f32> { self.child.flex() }
    fn get_size_from_child(&self) -> Option<Size> { self.child.get_size_from_child() }
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> { self.child.pos_start_end() }
}
impl EventElement for PageElement {}
impl Rebuildable for PageElement {
    /// Only hands a mark on to its child, so placing it again leaves the
    /// dirty-path index as it was; see [`Rebuildable::forwards_marks_only`].
    #[inline]
    fn forwards_marks_only(&self) -> bool { true }
    fn rebuild_if_dirty(&self, ctx: &BuildContext) { self.scoped(ctx, |ctx| self.child.rebuild_if_dirty(ctx)); }
    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) { self.scoped(ctx, callback); }
}

impl PageElement {
    fn scoped<'a, T>(&self, ctx: &BuildContext<'a>, callback: impl FnOnce(&BuildContext<'a>) -> T) -> T {
        self.context.with_page(ctx, callback)
    }
}

#[cfg(test)]
mod tests;
