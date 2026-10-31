use std::time::Duration;

use aimer_animation::curves;
use aimer_canvas::Mat3;
use aimer_widget::ErrorWidget;

use super::*;

#[test]
fn playback_starts_on_first_tick_and_releases_outgoing_content_at_completion() {
    let controller = AnimationController::new(Duration::from_millis(100), curves::LINEAR);
    controller.forward_from_first_tick();
    let effect = ResolvedNavigationTransition::new(Duration::from_millis(100), curves::LINEAR, |p, _| {
        NavigationTransitionFrame {
            incoming: crate::PaintPresentation { transform: Mat3::identity(), opacity: p, corner_radius: 0.0, reveal: None },
            outgoing: crate::PaintPresentation { transform: Mat3::identity(), opacity: 1.0 - p, corner_radius: 0.0, reveal: None },
        }
    }).unwrap();
    let playback = Playback { controller, effect, finished: Cell::new(false), swipe: None,
        outgoing: RefCell::new(Some(ChildBuilder::from_widget(ErrorWidget::new("outgoing")))) };
    let size = ResolvedSize { width: 100.0, height: 100.0 };
    let now = AnimInstant::now();
    assert_eq!(playback.sample(now, size).incoming.opacity, 0.0);
    assert_eq!(playback.sample(now + Duration::from_millis(50), size).incoming.opacity, 0.5);
    assert!(!playback.finished.get());
    assert_eq!(playback.sample(now + Duration::from_millis(100), size), NavigationTransitionFrame::FINISHED);
    assert!(playback.finished.get());
    assert!(!playback.controller.is_animating());
    assert!(playback.outgoing.borrow().is_none());
}

/// An effect whose incoming opacity is its progress, so a test reads progress back.
fn progress_effect(on_top: bool) -> ResolvedNavigationTransition {
    ResolvedNavigationTransition::new(Duration::from_millis(100), curves::LINEAR, |p, _| {
        NavigationTransitionFrame {
            incoming: crate::PaintPresentation { transform: Mat3::identity(), opacity: p, corner_radius: 0.0, reveal: None },
            outgoing: crate::PaintPresentation { transform: Mat3::identity(), opacity: 1.0 - p, corner_radius: 0.0, reveal: None },
        }
    }).unwrap().outgoing_on_top(on_top)
}

fn swipe_playback() -> (Playback, Rc<SwipeDrive>, Rc<Cell<u32>>) {
    let commits = Rc::new(Cell::new(0));
    let counter = commits.clone();
    let drive = SwipeDrive::new(move || counter.set(counter.get() + 1));
    let playback = Playback::swipe(progress_effect(true), drive.clone(),
        ChildBuilder::from_widget(ErrorWidget::new("underlay")));
    (playback, drive, commits)
}

const SIZE: ResolvedSize = ResolvedSize { width: 100.0, height: 100.0 };

#[test]
fn a_swipe_follows_the_finger_and_ignores_the_clock() {
    let (playback, drive, _) = swipe_playback();
    let now = AnimInstant::now();
    drive.drag(0.25);
    assert_eq!(playback.sample(now, SIZE).incoming.opacity, 0.25);
    assert_eq!(playback.sample(now + Duration::from_secs(5), SIZE).incoming.opacity, 0.25);
    drive.drag(0.8);
    assert_eq!(playback.sample(now + Duration::from_secs(5), SIZE).incoming.opacity, 0.8);
    assert!(!playback.finished.get());
    assert!(playback.outgoing.borrow().is_some(), "the page beneath stays while the finger is down");
}

#[test]
fn a_committed_swipe_settles_to_the_end_from_where_the_finger_lifted() {
    let (playback, drive, _) = swipe_playback();
    let now = AnimInstant::now();
    drive.drag(0.5);
    drive.release(true);
    // Half the distance is left, so half the duration: 50 ms.
    assert_eq!(playback.sample(now, SIZE).incoming.opacity, 0.5);
    assert_eq!(playback.sample(now + Duration::from_millis(25), SIZE).incoming.opacity, 0.75);
    assert!(!playback.finished.get());
    assert_eq!(drive.take_settled(), None);

    let end = playback.sample(now + Duration::from_millis(60), SIZE);
    assert_eq!(end.incoming.opacity, 1.0, "it ends at the destination, not at a blank frame");
    assert!(playback.finished.get());
    assert_eq!(drive.take_settled(), Some(true));
    assert_eq!(drive.take_settled(), None, "the outcome is reported once");
    assert_eq!(playback.sample(now + Duration::from_millis(90), SIZE).incoming.opacity, 1.0);
}

#[test]
fn a_cancelled_swipe_returns_to_the_start() {
    let (playback, drive, _) = swipe_playback();
    let now = AnimInstant::now();
    drive.drag(0.4);
    drive.release(false);
    assert_eq!(playback.sample(now, SIZE).incoming.opacity, 0.4);
    assert!((playback.sample(now + Duration::from_millis(20), SIZE).incoming.opacity - 0.2).abs() < 1e-6);
    // 40 ms is the exact boundary and f32 rounding puts it a hair short; 45 is past it.
    assert_eq!(playback.sample(now + Duration::from_millis(45), SIZE).incoming.opacity, 0.0);
    assert!(playback.finished.get());
    assert_eq!(drive.take_settled(), Some(false));
}

#[test]
fn a_swipe_that_finishes_asks_for_the_frame_that_cleans_up_after_it() {
    // The host drops the page beneath on the first frame that sees the swipe
    // finished. A trackpad's fingers lifted long before, so no input is coming to
    // bring that frame about: the swipe has to ask for it.
    let requests = Rc::new(Cell::new(0u32));
    let counted = requests.clone();
    let previous = aimer_events::window::set_thread_redraw_requester(move || counted.set(counted.get() + 1));

    let (playback, drive, _) = swipe_playback();
    let now = AnimInstant::now();
    drive.drag(0.4);
    drive.release(false);
    playback.sample(now, SIZE);
    let while_settling = requests.get();
    playback.sample(now + Duration::from_millis(20), SIZE);
    assert_eq!(requests.get(), while_settling, "nothing is asked for while it is still settling");

    playback.sample(now + Duration::from_millis(500), SIZE);
    assert!(playback.finished.get());
    assert_eq!(requests.get(), while_settling + 1, "one frame, when it finishes");

    playback.sample(now + Duration::from_millis(600), SIZE);
    assert_eq!(requests.get(), while_settling + 1, "and not again");

    aimer_events::window::restore_thread_redraw_requester(previous);
}

#[test]
fn a_swipe_cut_short_by_another_navigation_counts_as_cancelled() {
    let (playback, drive, commits) = swipe_playback();
    drive.drag(0.9);
    playback.finish();
    assert!(playback.finished.get());
    assert_eq!(drive.take_settled(), Some(false));
    assert_eq!(commits.get(), 0);
}

mod swipe_host {
    use super::*;

    fn element(playback: &Rc<Playback>, ctx: &BuildContext) -> AnyElement {
        NavigationFrame {
            child: ChildBuilder::from_widget(ErrorWidget::new("current")),
            playback: Some(Rc::clone(playback)),
            swipe: None,
        }.to_element(ctx)
    }

    /// An effect whose layer between the pages records the progress it is built with.
    fn veiled_effect(seen: &Rc<RefCell<Vec<f32>>>) -> ResolvedNavigationTransition {
        let seen = Rc::clone(seen);
        progress_effect(true).with_veil(move |progress| {
            seen.borrow_mut().push(progress);
            SizedBox::new().boxed()
        })
    }

    #[test]
    fn the_layer_between_the_pages_follows_the_finger_like_the_pages_do() {
        // A blur that builds as a page opens has to build as it is dragged away,
        // and has to be at the same place the page is: the effect's layer is part
        // of the effect, not of its clock.
        let ctx = presentation::context();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let drive = SwipeDrive::new(|| {});
        let playback = Rc::new(Playback::swipe(
            veiled_effect(&seen), drive.clone(), ChildBuilder::from_widget(ErrorWidget::new("underlay")),
        ));
        drive.drag(0.3);
        playback.sample(AnimInstant::now(), SIZE);
        let element = element(&playback, &ctx);
        assert!(!seen.borrow().is_empty(), "the layer is built for a swipe");
        assert_eq!(seen.borrow().last().copied(), Some(0.3), "built where the finger is");

        drive.drag(0.7);
        playback.sample(AnimInstant::now(), SIZE);
        element.rebuild_if_dirty(&ctx);
        assert_eq!(seen.borrow().last().copied(), Some(0.7), "and rebuilt as it moves");

        drive.drag(0.1);
        playback.sample(AnimInstant::now(), SIZE);
        element.rebuild_if_dirty(&ctx);
        assert_eq!(seen.borrow().last().copied(), Some(0.1), "and back again");
    }

    #[test]
    fn a_swipe_released_to_the_end_leaves_the_layer_at_the_end() {
        let ctx = presentation::context();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let drive = SwipeDrive::new(|| {});
        let playback = Rc::new(Playback::swipe(
            veiled_effect(&seen), drive.clone(), ChildBuilder::from_widget(ErrorWidget::new("underlay")),
        ));
        let element = element(&playback, &ctx);
        let now = AnimInstant::now();
        drive.drag(0.5);
        drive.release(true);
        playback.sample(now, SIZE);
        element.rebuild_if_dirty(&ctx);
        playback.sample(now + Duration::from_millis(200), SIZE);
        element.rebuild_if_dirty(&ctx);

        assert_eq!(seen.borrow().last().copied(), Some(1.0));
    }

    use aimer_attribute::Vec2d;
    use aimer_widget::{Drawable, EventElement, LayoutElement, PortableWidget, Rebuildable, VisitorElement};

    /// A page 200 wide that has been presented `offset` further along, as a page is
    /// that is sliding away, and then `shift` more, as the page it uncovers is.
    struct Sliding { offset: Rc<Cell<f32>>, shift: f32 }
    impl PortableWidget for Sliding {}
    impl Widget for Sliding {
        fn to_element(self, _: &BuildContext) -> AnyElement { aimer_widget::Element::boxed(self) }
    }
    impl VisitorElement for Sliding { fn debug_name(&self) -> &'static str { "Sliding" } }
    impl Drawable for Sliding { fn update(&self, _: &BuildContext) {} }
    impl Rebuildable for Sliding {}
    impl EventElement for Sliding {}
    impl LayoutElement for Sliding {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            let left = self.offset.get() + self.shift;
            Some((Vec2d { x: left, y: 0.0 }, Vec2d { x: left + 200.0, y: 100.0 }))
        }
    }

    #[test]
    fn the_host_covers_both_pages_so_the_swipe_is_not_left_behind_by_the_page_sliding_away() {
        // Scroll events are routed by where the cursor is. The cursor does not move
        // with the page, so as the departing page slides off to the right it leaves
        // the cursor behind, and a host that covered only that page would stop being
        // reached, and the swipe would stay where it was.
        let ctx = presentation::context();
        let offset = Rc::new(Cell::new(0.0));
        let drive = SwipeDrive::new(|| {});
        let playback = Rc::new(Playback::swipe(
            progress_effect(true),
            drive,
            ChildBuilder::from_widget(Sliding { offset: offset.clone(), shift: -200.0 }),
        ));
        let element = NavigationFrame {
            child: ChildBuilder::from_widget(Sliding { offset: offset.clone(), shift: 0.0 }),
            playback: Some(playback),
            swipe: None,
        }.to_element(&ctx);

        for slid in [0.0, 50.0, 120.0, 199.0, 200.0] {
            offset.set(slid);
            let (start, end) = element.pos_start_end().expect("the host has an area");
            // The host's own 0..200, which the cursor was over, is covered throughout.
            assert!(start.x <= 0.0 && end.x >= 200.0, "slid {slid}: covers {}..{}", start.x, end.x);
            assert!(start.y <= 0.0 && end.y >= 100.0);
        }
    }

    #[test]
    fn the_host_covers_just_its_page_when_there_is_only_one() {
        let ctx = presentation::context();
        let element = NavigationFrame {
            child: ChildBuilder::from_widget(Sliding { offset: Rc::new(Cell::new(30.0)), shift: 0.0 }),
            playback: None,
            swipe: None,
        }.to_element(&ctx);

        let (start, end) = element.pos_start_end().unwrap();

        assert_eq!((start.x, end.x), (30.0, 230.0));
    }

    #[test]
    fn the_page_under_the_finger_is_the_departing_one_and_stays_in_front_when_the_effect_says_so() {
        let ctx = presentation::context();
        let (playback, drive, _) = swipe_playback();
        drive.drag(0.25);
        let element = element(&Rc::new(playback), &ctx);

        // Slot 0 is the current page. Presented as the departing page, and an
        // effect that keeps the departing page on top paints slot 0 last.
        let mut order = Vec::new();
        element.visit_retained_v2_children(&mut |index, _| order.push(index));
        assert_eq!(order, [1, 0]);

        let (tree, root, [first, second]) = presentation::tree(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(presentation::sync(&element, &tree, root, &ctx));
        // Painted first is the page beneath (progress-as-opacity: incoming = p).
        assert_eq!(tree.presentation(first).unwrap().1, 0.25);
        assert_eq!(tree.presentation(second).unwrap().1, 0.75);
    }

    #[test]
    fn only_the_page_under_the_finger_takes_input() {
        let ctx = presentation::context();
        let (playback, _, _) = swipe_playback();
        let element = element(&Rc::new(playback), &ctx);
        let mut events = 0;
        element.event_children(&mut |_| events += 1);
        assert_eq!(events, 1);
    }

    #[test]
    fn a_committed_swipe_pops_once_and_keeps_both_pages_until_the_rebuild() {
        let ctx = presentation::context();
        let (playback, drive, commits) = swipe_playback();
        let playback = Rc::new(playback);
        let element = element(&playback, &ctx);
        drive.drag(0.6);
        drive.release(true);
        let now = AnimInstant::now();
        playback.sample(now, SIZE);
        playback.sample(now + Duration::from_secs(1), SIZE);

        element.update(&ctx);
        element.update(&ctx);

        assert_eq!(commits.get(), 1, "the pop is requested exactly once");
        let mut remaining = Vec::new();
        element.visit_retained_v2_children(&mut |index, _| remaining.push(index));
        assert_eq!(remaining.len(), 2, "removing the page beneath now would flash an empty frame");
    }

    #[test]
    fn a_cancelled_swipe_drops_the_page_beneath_without_popping() {
        let ctx = presentation::context();
        let (playback, drive, commits) = swipe_playback();
        let playback = Rc::new(playback);
        let element = element(&playback, &ctx);
        drive.drag(0.2);
        drive.release(false);
        let now = AnimInstant::now();
        playback.sample(now, SIZE);
        playback.sample(now + Duration::from_secs(1), SIZE);

        element.update(&ctx);

        assert_eq!(commits.get(), 0);
        let mut remaining = Vec::new();
        element.visit_retained_v2_children(&mut |index, _| remaining.push(index));
        assert_eq!(remaining, [0]);
    }
}

mod swipe_touch {
    use std::cell::RefCell;

    use aimer_attribute::Vec2d;
    use aimer_events::element::{ElementEvent, ScrollDeltaKind, TouchPhase};
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::{Drawable, EventDispatcher, EventElement, LayoutElement, PortableWidget, Rebuildable, VisitorElement};

    use super::*;
    use crate::SwipeBack;
    use crate::swipe_back::SwipeLink;

    type Log = Rc<RefCell<Vec<&'static str>>>;

    /// A page that records what reaches it, 200 wide.
    struct Probe(Log);
    impl PortableWidget for Probe {}
    impl Widget for Probe {
        fn to_element(self, _: &BuildContext) -> AnyElement { aimer_widget::Element::boxed(self) }
    }
    impl VisitorElement for Probe { fn debug_name(&self) -> &'static str { "Probe" } }
    impl Drawable for Probe { fn update(&self, _: &BuildContext) {} }
    impl Rebuildable for Probe {}
    impl LayoutElement for Probe {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> { Some((Vec2d::ZERO, Vec2d { x: 200.0, y: 100.0 })) }
    }
    impl EventElement for Probe {
        fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
            let name = match event {
                ElementEvent::PointerDown(_) => "down",
                ElementEvent::PointerMove(_) => "move",
                ElementEvent::PointerUp(_) => "up",
                // A page that scrolls only up and down has no use for a scroll that
                // goes sideways: it sees it, and leaves it.
                ElementEvent::Scroll { .. } => {
                    self.0.borrow_mut().push("scroll");
                    return aimer_widget::EventResult::ignored();
                }
                _ => return aimer_widget::EventResult::ignored(),
            };
            self.0.borrow_mut().push(name);
            aimer_widget::EventResult::consumed()
        }
    }

    /// A page that uses every scroll it is given, as a horizontal scroller that can
    /// still move does.
    struct Scroller(Log);
    impl PortableWidget for Scroller {}
    impl Widget for Scroller {
        fn to_element(self, _: &BuildContext) -> AnyElement { aimer_widget::Element::boxed(self) }
    }
    impl VisitorElement for Scroller { fn debug_name(&self) -> &'static str { "Scroller" } }
    impl Drawable for Scroller { fn update(&self, _: &BuildContext) {} }
    impl Rebuildable for Scroller {}
    impl LayoutElement for Scroller {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> { Some((Vec2d::ZERO, Vec2d { x: 200.0, y: 100.0 })) }
    }
    impl EventElement for Scroller {
        fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
            if !matches!(event, ElementEvent::Scroll { .. }) { return aimer_widget::EventResult::ignored(); }
            self.0.borrow_mut().push("scroll");
            aimer_events::pointer::note_scroll_used();
            aimer_widget::EventResult::consumed()
        }
    }

    /// A page like an opaque container, which absorbs any scroll nothing in front of
    /// it used, so that it cannot fall through to a scroller behind it. It does not
    /// redraw: nothing moved.
    struct Occluder(Log);
    impl PortableWidget for Occluder {}
    impl Widget for Occluder {
        fn to_element(self, _: &BuildContext) -> AnyElement { aimer_widget::Element::boxed(self) }
    }
    impl VisitorElement for Occluder { fn debug_name(&self) -> &'static str { "Occluder" } }
    impl Drawable for Occluder { fn update(&self, _: &BuildContext) {} }
    impl Rebuildable for Occluder {}
    impl LayoutElement for Occluder {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> { Some((Vec2d::ZERO, Vec2d { x: 200.0, y: 100.0 })) }
    }
    impl EventElement for Occluder {
        fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
            if !matches!(event, ElementEvent::Scroll { .. }) { return aimer_widget::EventResult::ignored(); }
            self.0.borrow_mut().push("scroll");
            aimer_widget::EventResult::consumed()
        }
    }

    /// A page that scrolls once and then has nothing to scroll: a list that reached
    /// its end.
    struct UsesOnce(Log, Cell<bool>);
    impl PortableWidget for UsesOnce {}
    impl Widget for UsesOnce {
        fn to_element(self, _: &BuildContext) -> AnyElement { aimer_widget::Element::boxed(self) }
    }
    impl VisitorElement for UsesOnce { fn debug_name(&self) -> &'static str { "UsesOnce" } }
    impl Drawable for UsesOnce { fn update(&self, _: &BuildContext) {} }
    impl Rebuildable for UsesOnce {}
    impl LayoutElement for UsesOnce {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> { Some((Vec2d::ZERO, Vec2d { x: 200.0, y: 100.0 })) }
    }
    impl EventElement for UsesOnce {
        fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
            if !matches!(event, ElementEvent::Scroll { .. }) { return aimer_widget::EventResult::ignored(); }
            self.0.borrow_mut().push("scroll");
            if self.1.replace(true) {
                aimer_widget::EventResult::ignored()
            } else {
                aimer_events::pointer::note_scroll_used();
                aimer_widget::EventResult::consumed()
            }
        }
    }

    /// A page like a button: it captures on press, so it must be told when the
    /// touch is taken from it. `consume_moves` makes it a draggable that claims the
    /// pointer when it moves.
    struct Pressable { log: Log, consume_moves: bool }
    impl PortableWidget for Pressable {}
    impl Widget for Pressable {
        fn to_element(self, _: &BuildContext) -> AnyElement { aimer_widget::Element::boxed(self) }
    }
    impl VisitorElement for Pressable { fn debug_name(&self) -> &'static str { "Pressable" } }
    impl Drawable for Pressable { fn update(&self, _: &BuildContext) {} }
    impl Rebuildable for Pressable {}
    impl LayoutElement for Pressable {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> { Some((Vec2d::ZERO, Vec2d { x: 200.0, y: 100.0 })) }
    }
    impl EventElement for Pressable {
        fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
            use aimer_widget::{EventResult, PointerKey};
            match event {
                ElementEvent::PointerDown(p) => {
                    self.log.borrow_mut().push("down");
                    EventResult::consumed().with_pointer_capture(PointerKey::new(p.source, p.id))
                }
                ElementEvent::PointerMove(p) => {
                    self.log.borrow_mut().push("move");
                    if self.consume_moves {
                        // A draggable: it starts a drag of its own, and says so.
                        aimer_widget::claim_pointer(PointerKey::new(p.source, p.id));
                        EventResult::consumed().with_redraw()
                    } else {
                        EventResult::consumed()
                    }
                }
                ElementEvent::PointerUp(p) => {
                    self.log.borrow_mut().push("up");
                    EventResult::consumed().with_pointer_release(PointerKey::new(p.source, p.id))
                }
                ElementEvent::Cancel => {
                    self.log.borrow_mut().push("cancel");
                    EventResult::consumed()
                }
                _ => EventResult::ignored(),
            }
        }
    }

    struct Rig {
        element: AnyElement,
        dispatcher: EventDispatcher,
        log: Log,
        clock: Rc<Cell<f64>>,
        drive: Rc<RefCell<Option<Rc<SwipeDrive>>>>,
        begun: Rc<Cell<u32>>,
        link: Rc<SwipeLink>,
    }

    /// A host over a [`Probe`], whose `begin` hands out a drive when `allow`.
    fn rig(allow: bool) -> Rig {
        rig_over(allow, |log| ChildBuilder::from_widget(Probe(log)))
    }

    /// A host over a [`Pressable`] page.
    fn pressable_rig(allow: bool, consume_moves: bool) -> Rig {
        rig_over(allow, move |log| ChildBuilder::from_widget(Pressable { log, consume_moves }))
    }

    fn rig_over(allow: bool, page: impl FnOnce(Log) -> ChildBuilder) -> Rig {
        let ctx = presentation::context();
        let log = Log::default();
        let clock = Rc::new(Cell::new(0.0));
        let drive = Rc::new(RefCell::new(None));
        let begun = Rc::new(Cell::new(0));
        let (slot, count) = (drive.clone(), begun.clone());
        let link = {
            let clock = clock.clone();
            SwipeLink::new(
                move || {
                    count.set(count.get() + 1);
                    allow.then(|| {
                        let d = SwipeDrive::new(|| {});
                        *slot.borrow_mut() = Some(d.clone());
                        SwipeBack(d)
                    })
                },
                move || clock.get(),
            )
        };
        link.set_enabled(true);
        let element = NavigationFrame {
            child: page(log.clone()),
            playback: None,
            swipe: Some(link.clone()),
        }.to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();
        // A tree is indexed before its root has an identity, and again once it
        // has one. Real trees have it from the start, so settle that first: an
        // identity that changed under a held capture would drop the capture.
        for _ in 0..2 {
            let away = Vec2d { x: 900.0, y: 900.0 };
            let _ = dispatcher.dispatch(element.as_ref(), away,
                &ElementEvent::PointerMove(PointerInfo::mouse(away, PointerButton::Primary)));
        }
        Rig { element, dispatcher, log, clock, drive, begun, link }
    }

    fn touch(x: f32, y: f32) -> PointerInfo {
        PointerInfo::new(Vec2d { x, y }, PointerSource::Touch, 1, PointerButton::Primary)
    }

    impl Rig {
        fn send(&mut self, event: ElementEvent) {
            let pos = event.get_pointer_pos().unwrap_or_default();
            let _ = self.dispatcher.dispatch(self.element.as_ref(), pos, &event);
        }
        fn down(&mut self, x: f32, y: f32) { self.send(ElementEvent::PointerDown(touch(x, y))); }
        fn mv(&mut self, x: f32, y: f32, at: f64) { self.clock.set(at); self.send(ElementEvent::PointerMove(touch(x, y))); }
        fn up(&mut self, x: f32, y: f32, at: f64) { self.clock.set(at); self.send(ElementEvent::PointerUp(touch(x, y))); }
        fn seen(&self) -> Vec<&'static str> { self.log.borrow().clone() }
        /// One scroll event of a trackpad gesture, over the middle of the page.
        fn scroll(&mut self, dx: f32, dy: f32, phase: TouchPhase, direct: bool, at: f64) -> aimer_widget::EventResult {
            self.clock.set(at);
            let event = ElementEvent::Scroll {
                delta: Vec2d { x: dx, y: dy },
                phase,
                kind: ScrollDeltaKind::Pixel,
                is_direct_manipulation: direct,
            };
            self.dispatcher.dispatch(self.element.as_ref(), Vec2d { x: 100.0, y: 50.0 }, &event)
        }
        /// A scroll in `kind`, which a notched wheel and the unclassified opening
        /// pulses of a trackpad gesture both report as lines.
        fn scroll_as(&mut self, kind: ScrollDeltaKind, dx: f32, phase: TouchPhase, direct: bool, at: f64) -> aimer_widget::EventResult {
            self.clock.set(at);
            let event = ElementEvent::Scroll {
                delta: Vec2d { x: dx, y: 0.0 },
                phase,
                kind,
                is_direct_manipulation: direct,
            };
            self.dispatcher.dispatch(self.element.as_ref(), Vec2d { x: 100.0, y: 50.0 }, &event)
        }
        /// A trackpad gesture that has begun: fingers down and moved right.
        fn trackpad_swipe(&mut self) {
            let _ = self.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);
        }
        fn finger(&self) -> f32 { self.drive.borrow().as_ref().expect("a swipe began").finger.get() }
    }

    /// A host over an [`Occluder`] page.
    fn occluder_rig(allow: bool) -> Rig {
        rig_over(allow, |log| ChildBuilder::from_widget(Occluder(log)))
    }

    #[test]
    fn a_page_that_only_absorbs_the_scroll_does_not_keep_it() {
        // Every page root that has a background is one of these. If it kept the
        // scroll, no swipe could ever begin on a real page.
        let mut rig = occluder_rig(true);

        let result = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);

        assert!(result.is_consumed());
        assert_eq!(rig.begun.get(), 1, "the swipe began though the page absorbed the scroll");
        assert!((rig.finger() - 0.25).abs() < 1e-4, "{}", rig.finger());
    }

    #[test]
    fn once_the_swipe_has_begun_the_page_is_not_scrolled_by_the_rest_of_the_gesture() {
        let mut rig = occluder_rig(true);
        let _ = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);
        let before = rig.seen().len();

        for step in 1..4 {
            let _ = rig.scroll(10.0, 0.0, TouchPhase::Moved, true, step as f64 * 0.02);
        }

        assert_eq!(rig.seen().len(), before, "the page sees nothing more of this gesture");
        assert!((rig.finger() - 0.5).abs() < 1e-4, "while the swipe follows it: {}", rig.finger());
    }

    #[test]
    fn a_gesture_the_page_used_stays_the_pages_even_when_it_runs_out_of_things_to_scroll() {
        // The page scrolled on the first frame, so this is a scroll, and sliding
        // sideways later in it must not turn it into a back swipe.
        let mut rig = rig_over(true, |log| ChildBuilder::from_widget(UsesOnce(log, Cell::new(false))));

        let _ = rig.scroll(0.0, 30.0, TouchPhase::Started, true, 0.0);
        for step in 1..6 {
            let result = rig.scroll(40.0, 0.0, TouchPhase::Moved, true, step as f64 * 0.02);
            assert!(!result.is_consumed());
        }

        assert_eq!(rig.begun.get(), 0);
    }

    #[test]
    fn a_host_over_an_absorbing_page_still_hands_the_page_a_gesture_that_is_not_a_swipe() {
        let mut rig = occluder_rig(true);

        let result = rig.scroll(2.0, 40.0, TouchPhase::Started, true, 0.0);

        assert!(result.is_consumed(), "the page's own absorbing stands");
        assert_eq!(rig.begun.get(), 0);
        assert_eq!(rig.seen(), ["scroll"]);
    }

    /// A host over a [`Scroller`] page.
    fn scroller_rig(allow: bool) -> Rig {
        rig_over(allow, |log| ChildBuilder::from_widget(Scroller(log)))
    }

    #[test]
    fn a_rightward_two_finger_swipe_pops_the_page_and_follows_the_fingers() {
        let mut rig = rig(true);

        let result = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);
        assert!(result.is_consumed(), "the swipe takes the gesture");
        assert_eq!(rig.begun.get(), 1);
        // The page is 200 wide, so a full pop takes 120 px of travel: 30 is a quarter.
        assert!((rig.finger() - 0.25).abs() < 1e-4, "{}", rig.finger());

        let _ = rig.scroll(30.0, 0.0, TouchPhase::Moved, true, 0.02);
        assert!((rig.finger() - 0.5).abs() < 1e-4, "{}", rig.finger());
        let _ = rig.scroll(-60.0, 0.0, TouchPhase::Moved, true, 0.04);
        assert!(rig.finger() < 1e-4, "it comes back as the fingers do: {}", rig.finger());
        assert_eq!(rig.begun.get(), 1, "one swipe, begun once");
    }

    #[test]
    fn lifting_the_fingers_far_enough_completes_the_pop_and_short_of_it_cancels() {
        let mut rig = rig(true);
        rig.trackpad_swipe();
        let _ = rig.scroll(40.0, 0.0, TouchPhase::Moved, true, 1.0);
        let _ = rig.scroll(0.0, 0.0, TouchPhase::Ended, true, 2.0);
        assert_eq!(rig.drive.borrow().as_ref().unwrap().release.get(), Some(true));

        let mut rig = self::rig(true);
        rig.trackpad_swipe();
        let _ = rig.scroll(0.0, 0.0, TouchPhase::Ended, true, 2.0);
        assert_eq!(rig.drive.borrow().as_ref().unwrap().release.get(), Some(false));
    }

    #[test]
    fn the_glide_after_the_fingers_lift_belongs_to_the_swipe_and_not_to_anything_beyond() {
        let mut rig = rig(true);
        rig.trackpad_swipe();
        let _ = rig.scroll(40.0, 0.0, TouchPhase::Moved, true, 1.0);
        let _ = rig.scroll(0.0, 0.0, TouchPhase::Ended, true, 1.1);

        // The glide, with no fingers on the surface, is swallowed for a moment...
        assert!(rig.scroll(6.0, 0.0, TouchPhase::Moved, false, 1.2).is_consumed());
        // ...until it goes quiet, which the host's frames notice...
        rig.clock.set(1.6);
        rig.link.poll_trackpad(200.0);
        // ...and a wheel long afterwards is nobody's business here.
        assert!(!rig.scroll(6.0, 0.0, TouchPhase::Moved, false, 4.0).is_consumed());
    }

    /// A page-width swipe, 120 px of travel from end to end, that has lifted at 0.42
    /// of the way and at 250 px a second: slow enough to be no flick, fast enough
    /// for a glide to follow.
    fn lifted_with_a_glide_to_come() -> Rig {
        let mut rig = occluder_rig(true);
        let _ = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);
        let _ = rig.scroll(10.0, 0.0, TouchPhase::Moved, true, 0.50);
        let _ = rig.scroll(10.0, 0.0, TouchPhase::Moved, true, 0.55);
        let lift = rig.scroll(0.0, 0.0, TouchPhase::Ended, true, 0.58);
        assert!(lift.is_consumed());
        assert!(rig.drive.borrow().as_ref().unwrap().release.get().is_none(), "not yet let go: a glide is to come");
        rig
    }

    #[test]
    fn the_glide_after_the_lift_carries_the_swipe_on_and_decides_where_it_ends() {
        let mut rig = lifted_with_a_glide_to_come();
        let at_lift = rig.finger();
        assert!(at_lift < 0.5, "short of half way when the fingers lifted: {at_lift}");

        let _ = rig.scroll(6.0, 0.0, TouchPhase::Moved, false, 0.60);
        let _ = rig.scroll(6.0, 0.0, TouchPhase::Moved, false, 0.62);
        assert!(rig.finger() > 0.5, "the glide carried it past half way: {}", rig.finger());
        assert!(rig.drive.borrow().as_ref().unwrap().release.get().is_none(), "still going");

        let _ = rig.scroll(0.0, 0.0, TouchPhase::Ended, false, 0.64);
        assert_eq!(rig.drive.borrow().as_ref().unwrap().release.get(), Some(true), "and completes");
    }

    #[test]
    fn a_glide_that_never_reports_its_end_is_ended_when_the_host_asks() {
        let mut rig = lifted_with_a_glide_to_come();
        let _ = rig.scroll(12.0, 0.0, TouchPhase::Moved, false, 0.60);

        // The frame loop asks between events. Nothing is due yet...
        rig.clock.set(0.64);
        rig.link.poll_trackpad(200.0);
        assert!(rig.drive.borrow().as_ref().unwrap().release.get().is_none());
        // ...and after a quiet moment the swipe is let go, where the glide left it.
        rig.clock.set(0.80);
        rig.link.poll_trackpad(200.0);
        assert_eq!(rig.drive.borrow().as_ref().unwrap().release.get(), Some(true));
    }

    #[test]
    fn asking_when_nothing_is_gliding_does_nothing() {
        let mut rig = rig(true);
        rig.link.poll_trackpad(200.0);
        let _ = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);

        rig.clock.set(5.0);
        rig.link.poll_trackpad(200.0);

        assert!(rig.drive.borrow().as_ref().unwrap().release.get().is_none(), "fingers still down");
    }

    #[test]
    fn a_line_scroll_in_the_middle_of_a_swipe_does_not_end_it() {
        // The smoother reports the first pulses of a trackpad gesture as lines,
        // between the pixel steps that follow. They are no part of the swipe, and
        // no reason to let go of it.
        let mut rig = occluder_rig(true);
        let _ = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);
        assert!((rig.finger() - 0.25).abs() < 1e-4);

        let line = rig.scroll_as(ScrollDeltaKind::Line, 0.4, TouchPhase::Moved, true, 0.01);
        let _ = rig.scroll(30.0, 0.0, TouchPhase::Moved, true, 0.02);

        assert!(line.is_consumed(), "the host holds the gesture, so the page is not scrolled by it");
        assert_eq!(rig.begun.get(), 1, "one swipe throughout");
        assert!((rig.finger() - 0.5).abs() < 1e-4, "and it went on: {}", rig.finger());
        assert!(rig.drive.borrow().as_ref().unwrap().release.get().is_none(), "never let go");
    }

    #[test]
    fn a_swipe_whose_end_is_reported_as_lines_still_ends() {
        // After a pause of a quarter of a second the smoother no longer knows the
        // gesture it is part of, and the fingers lifting arrives as the end of a
        // wheel. The fingers lifted all the same.
        let mut rig = occluder_rig(true);
        let _ = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);
        let _ = rig.scroll(50.0, 0.0, TouchPhase::Moved, true, 0.5);

        let end = rig.scroll_as(ScrollDeltaKind::Line, 0.0, TouchPhase::Ended, false, 2.0);

        assert!(end.is_consumed());
        assert_eq!(rig.drive.borrow().as_ref().unwrap().release.get(), Some(true), "far enough, so it pops");

        let mut rig = self::rig(true);
        rig.trackpad_swipe();
        let _ = rig.scroll_as(ScrollDeltaKind::Line, 0.0, TouchPhase::Ended, true, 2.0);
        assert_eq!(rig.drive.borrow().as_ref().unwrap().release.get(), Some(false), "and short of it, it does not");
    }

    #[test]
    fn a_line_scroll_with_no_swipe_going_is_the_pages() {
        let mut rig = rig(true);

        let result = rig.scroll_as(ScrollDeltaKind::Line, 20.0, TouchPhase::Moved, false, 0.0);

        assert!(!result.is_consumed());
        assert_eq!(rig.seen(), ["scroll"], "the page has it, as it would have");
        assert_eq!(rig.begun.get(), 0);
    }

    #[test]
    fn a_vertical_scroll_is_left_to_the_page_throughout() {
        let mut rig = rig(true);

        let started = rig.scroll(2.0, 30.0, TouchPhase::Started, true, 0.0);
        let moved = rig.scroll(40.0, 20.0, TouchPhase::Moved, true, 0.1);

        assert!(!started.is_consumed() && !moved.is_consumed());
        assert_eq!(rig.begun.get(), 0);
        assert_eq!(rig.seen(), ["scroll", "scroll"], "the page saw both and kept scrolling");
    }

    #[test]
    fn a_page_that_uses_the_scroll_keeps_it() {
        // A horizontal scroller that can still move takes the event, so it never
        // reaches the host and no swipe begins.
        let mut rig = scroller_rig(true);

        let result = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);

        assert!(result.is_consumed());
        assert_eq!(rig.begun.get(), 0);
        assert_eq!(rig.seen(), ["scroll"]);
    }

    #[test]
    fn a_mouse_wheel_is_not_a_swipe() {
        let mut rig = rig(true);

        for step in 0..10 {
            let result = rig.scroll(40.0, 0.0, TouchPhase::Moved, false, step as f64 * 0.01);
            assert!(!result.is_consumed());
        }
        assert_eq!(rig.begun.get(), 0);
    }

    #[test]
    fn with_nowhere_to_go_back_to_a_swipe_does_not_begin() {
        let mut rig = rig(true);
        rig.link.set_enabled(false);

        let result = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);

        assert!(!result.is_consumed());
        assert_eq!(rig.begun.get(), 0);
    }

    #[test]
    fn a_swipe_that_cannot_begin_is_asked_for_once_and_leaves_the_gesture_to_the_page() {
        let mut rig = rig(false);

        let first = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.0);
        let more = rig.scroll(30.0, 0.0, TouchPhase::Moved, true, 0.02);

        assert!(!first.is_consumed() && !more.is_consumed());
        assert_eq!(rig.begun.get(), 1, "declined once, not on every event");
        assert_eq!(rig.seen(), ["scroll", "scroll"]);
    }

    #[test]
    fn a_touch_swipe_in_progress_is_not_disturbed_by_a_trackpad() {
        let mut rig = rig(true);
        rig.down(5.0, 50.0);
        rig.mv(60.0, 50.0, 0.1);
        assert_eq!(rig.begun.get(), 1, "the touch swipe began");

        let result = rig.scroll(30.0, 0.0, TouchPhase::Started, true, 0.2);

        assert!(!result.is_consumed());
        assert_eq!(rig.begun.get(), 1, "and the trackpad did not begin a second one");
    }

    #[test]
    fn a_press_the_swipe_does_not_want_reaches_the_page_untouched() {
        let mut rig = rig(true);
        rig.down(50.0, 50.0);
        assert_eq!(rig.seen(), ["down"], "outside the edge zone");

        let mut rig = self::rig(true);
        let mouse = PointerInfo::mouse(Vec2d { x: 5.0, y: 50.0 }, PointerButton::Primary);
        rig.send(ElementEvent::PointerDown(mouse));
        assert_eq!(rig.seen(), ["down"], "a cursor is not a finger");
    }

    #[test]
    fn a_touch_in_the_edge_zone_is_held_back_and_a_tap_is_replayed_whole() {
        let mut rig = rig(true);
        rig.down(5.0, 50.0);
        assert_eq!(rig.seen(), Vec::<&str>::new(), "the page does not see the press yet");
        rig.up(5.0, 50.0, 0.05);
        assert_eq!(rig.seen(), ["down", "up"], "a tap is still a tap, only later");
    }

    #[test]
    fn a_vertical_drag_from_the_edge_goes_to_the_page_from_the_press_on() {
        let mut rig = rig(true);
        rig.down(5.0, 50.0);
        rig.mv(6.0, 80.0, 0.02);
        rig.mv(6.0, 90.0, 0.04);
        rig.up(6.0, 90.0, 0.06);
        assert_eq!(rig.seen(), ["down", "move", "move", "up"]);
        assert_eq!(rig.begun.get(), 0);
    }

    #[test]
    fn a_rightward_drag_from_the_edge_moves_the_page_and_the_page_never_sees_it() {
        let mut rig = rig(true);
        rig.down(5.0, 50.0);
        rig.mv(30.0, 52.0, 0.02);
        assert_eq!(rig.begun.get(), 1, "the swipe starts exactly once");
        rig.mv(130.0, 52.0, 0.04);
        assert!((rig.finger() - 0.5).abs() < 1e-6, "progress is measured from where the swipe began: (130-30)/200");
        rig.mv(150.0, 52.0, 0.06);
        assert!((rig.finger() - 0.6).abs() < 1e-6);
        assert_eq!(rig.seen(), Vec::<&str>::new());
        assert_eq!(rig.begun.get(), 1);
    }

    #[test]
    fn lifting_past_halfway_commits_and_short_of_it_cancels() {
        for (to, commit) in [(150.0, true), (70.0, false)] {
            let mut rig = rig(true);
            rig.down(5.0, 50.0);
            rig.mv(30.0, 52.0, 0.02);
            rig.mv(to, 52.0, 1.0);
            rig.up(to, 52.0, 2.0);
            let drive = rig.drive.borrow().clone().unwrap();
            assert_eq!(drive.release.get(), Some(commit), "released at x = {to}");
            assert_eq!(rig.seen(), Vec::<&str>::new());
        }
    }

    #[test]
    fn a_swipe_that_cannot_begin_is_handed_to_the_page_instead() {
        let mut rig = rig(false);
        rig.down(5.0, 50.0);
        rig.mv(30.0, 52.0, 0.02);
        rig.mv(60.0, 52.0, 0.04);
        rig.up(60.0, 52.0, 0.06);
        assert_eq!(rig.begun.get(), 1);
        assert_eq!(rig.seen(), ["down", "move", "move", "up"]);
    }

    #[test]
    fn a_cancelled_touch_cancels_the_swipe_and_nothing_is_stuck_afterwards() {
        let mut rig = rig(true);
        rig.down(5.0, 50.0);
        rig.mv(30.0, 52.0, 0.02);
        rig.mv(100.0, 52.0, 0.04);
        rig.send(ElementEvent::Cancel);
        assert_eq!(rig.drive.borrow().as_ref().unwrap().release.get(), Some(false));
        // The next touch is an ordinary one again.
        rig.down(80.0, 50.0);
        assert_eq!(rig.seen(), ["down"]);
    }

    #[test]
    fn a_press_in_the_middle_reaches_the_page_at_once_and_a_tap_is_an_ordinary_tap() {
        let mut rig = rig(true);
        rig.down(100.0, 50.0);
        assert_eq!(rig.seen(), ["down"], "nothing is held back away from the edge");
        rig.up(100.0, 50.0, 0.05);
        assert_eq!(rig.seen(), ["down", "up"]);
        rig.down(100.0, 50.0);
        assert_eq!(rig.seen(), ["down", "up", "down"], "and the next touch is as ordinary");
    }

    #[test]
    fn a_vertical_drag_in_the_middle_is_the_pages_throughout() {
        let mut rig = pressable_rig(true, false);
        rig.down(100.0, 50.0);
        rig.mv(103.0, 80.0, 0.02);
        rig.mv(104.0, 95.0, 0.04);
        rig.up(104.0, 95.0, 0.06);
        assert_eq!(rig.seen(), ["down", "move", "move", "up"]);
        assert_eq!(rig.begun.get(), 0);
    }

    #[test]
    fn a_rightward_drag_in_the_middle_takes_the_touch_and_tells_the_page_its_press_is_off() {
        let mut rig = pressable_rig(true, false);
        rig.down(60.0, 50.0);
        rig.mv(85.0, 52.0, 0.02);
        assert_eq!(rig.begun.get(), 1, "a swipe began");
        assert_eq!(rig.seen(), ["down", "move", "cancel"], "the button that was pressed lets go");
        rig.mv(145.0, 52.0, 0.04);
        assert!((rig.finger() - 0.3).abs() < 1e-6, "progress counts from where the swipe began: (145-85)/200");
        rig.up(145.0, 52.0, 2.0);
        assert_eq!(rig.seen(), ["down", "move", "cancel"], "the page sees nothing more of this touch");
        assert!(rig.drive.borrow().as_ref().unwrap().release.get().is_some(), "and the swipe was released");
    }

    #[test]
    fn a_page_that_starts_a_drag_of_its_own_keeps_the_touch() {
        // A slider, or a carousel that is scrolling: it claims the pointer.
        let mut rig = pressable_rig(true, true);
        rig.down(60.0, 50.0);
        rig.mv(85.0, 52.0, 0.02);
        rig.mv(120.0, 52.0, 0.04);
        rig.up(120.0, 52.0, 0.06);
        assert_eq!(rig.begun.get(), 0);
        assert_eq!(rig.seen(), ["down", "move", "move", "up"]);
    }

    #[test]
    fn a_pointer_a_child_claimed_stays_with_the_child() {
        use aimer_widget::{PointerKey, claim_pointer, release_pointer};
        let mut rig = pressable_rig(true, false);
        rig.down(60.0, 50.0);
        let finger = PointerKey::new(PointerSource::Touch, 1);
        claim_pointer(finger); // a text selection that started dragging, say
        rig.mv(85.0, 52.0, 0.02);
        rig.mv(120.0, 52.0, 0.04);
        release_pointer(finger);
        assert_eq!(rig.begun.get(), 0);
        assert_eq!(rig.seen(), ["down", "move", "move"]);
    }

    #[test]
    fn a_swipe_that_cannot_begin_leaves_the_page_the_whole_touch() {
        let mut rig = pressable_rig(false, false);
        rig.down(60.0, 50.0);
        rig.mv(85.0, 52.0, 0.02);
        rig.mv(120.0, 52.0, 0.04);
        rig.up(120.0, 52.0, 0.06);
        assert_eq!(rig.begun.get(), 1);
        assert_eq!(rig.seen(), ["down", "move", "move", "up"], "no second press, no cancel");
    }

    #[test]
    fn a_second_finger_is_routed_to_the_page_as_usual() {
        let mut rig = rig(true);
        rig.down(60.0, 50.0);
        let second = PointerInfo::new(Vec2d { x: 150.0, y: 50.0 }, PointerSource::Touch, 2, PointerButton::Primary);
        rig.send(ElementEvent::PointerDown(second));
        assert_eq!(rig.seen(), ["down", "down"]);
    }

    #[test]
    fn edge_only_leaves_the_middle_of_the_page_alone() {
        let mut rig = pressable_rig(true, false);
        let NavigationFrame { swipe: Some(link), .. } = (NavigationFrame {
            child: ChildBuilder::from_widget(Probe(Log::default())), playback: None,
            swipe: Some(SwipeLink::new(|| None, || 0.0)),
        }) else { unreachable!() };
        drop(link);
        // Rebuild the rig's link as edge-only through the public setter.
        let ctx = presentation::context();
        let log = Log::default();
        let link = SwipeLink::new(|| None, || 0.0);
        link.set_enabled(true);
        link.set_anywhere(false);
        let element = NavigationFrame {
            child: ChildBuilder::from_widget(Pressable { log: log.clone(), consume_moves: false }),
            playback: None, swipe: Some(link),
        }.to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();
        let mut send = |event: ElementEvent| {
            let pos = event.get_pointer_pos().unwrap_or_default();
            let _ = dispatcher.dispatch(element.as_ref(), pos, &event);
        };
        send(ElementEvent::PointerDown(touch(100.0, 50.0)));
        send(ElementEvent::PointerMove(touch(160.0, 52.0)));
        send(ElementEvent::PointerUp(touch(160.0, 52.0)));
        assert_eq!(*log.borrow(), ["down", "move", "up"], "no swipe, and no cancel");
        let _ = &mut rig;
    }

    #[test]
    fn a_system_cancel_of_a_watched_touch_reaches_the_page_that_holds_it() {
        let mut rig = pressable_rig(true, false);
        rig.down(100.0, 50.0);
        rig.send(ElementEvent::Cancel);
        assert_eq!(rig.seen(), ["down", "cancel"]);
        rig.down(100.0, 50.0);
        assert_eq!(rig.seen(), ["down", "cancel", "down"], "and the next touch is ordinary again");
    }

    /// What the host costs a touch that is not a swipe: a vertical drag the page
    /// keeps, routed as usual (link off) and forwarded by the host (link on).
    /// Run with `--run-ignored only` in a debug build, where the cost matters
    /// most; it asserts nothing, it reports.
    #[test]
    #[ignore = "profile harness: prints per-move costs"]
    fn profile_the_cost_of_forwarding_a_touch_the_page_keeps() {
        const MOVES: u32 = 20_000;
        for (label, enabled) in [("link off", false), ("link on ", true)] {
            let mut rig = rig(true);
            rig.link.set_enabled(enabled);
            rig.down(100.0, 10.0);
            let started = aimer_time::Instant::now();
            for n in 0..MOVES {
                rig.send(ElementEvent::PointerMove(touch(100.0 + (n % 3) as f32, 10.0 + (n % 80) as f32)));
            }
            let per_move = started.elapsed().as_secs_f64() * 1e6 / MOVES as f64;
            eprintln!("{label}: {per_move:.2} us per move");
        }
    }

    #[test]
    fn a_link_that_is_not_enabled_never_holds_anything_back() {
        let ctx = presentation::context();
        let log = Log::default();
        // Enabled only by the navigator, and only when there is somewhere to go back to.
        let link = SwipeLink::new(|| None, || 0.0);
        let element = NavigationFrame {
            child: ChildBuilder::from_widget(Probe(log.clone())),
            playback: None,
            swipe: Some(link),
        }
        .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();
        let down = ElementEvent::PointerDown(touch(5.0, 50.0));
        let _ = dispatcher.dispatch(element.as_ref(), Vec2d { x: 5.0, y: 50.0 }, &down);
        assert_eq!(*log.borrow(), ["down"], "a navigator with nothing to go back to leaves its edge to the page");
    }
}

mod presentation {
    use std::sync::OnceLock;

    use aimer_attribute::BoxConstraint;
    use aimer_cupid::draw_cmd_v2::{Rect, RenderNodeId, RenderTree};
    use aimer_widget::base::WindowHandle;

    use super::*;
    use crate::{AnchorContent, AnchoredFrame, PaintPresentation};

    pub(super) fn context() -> BuildContext<'static> {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
        });
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        let mut context = BuildContext::new(
            aimer_canvas::FrameCanvas::new(inner),
            ResolvedSize { width: 100.0, height: 100.0 },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            runtime.handle().clone(),
        );
        context.box_constraint = BoxConstraint { min_width: 0.0, min_height: 0.0, max_width: 100.0, max_height: 100.0 };
        context
    }

    fn opacity(value: f32) -> PaintPresentation {
        PaintPresentation { opacity: value, ..PaintPresentation::IDENTITY_OPAQUE }
    }

    /// A host mid-transition with an outgoing page whose effect was built by `effect`.
    fn host(effect: ResolvedNavigationTransition, context: &BuildContext) -> AnyElement {
        host_with_playback(effect, context).0
    }

    /// Like [`host`], also returning the playback so a test can end it.
    fn host_with_playback(effect: ResolvedNavigationTransition, context: &BuildContext) -> (AnyElement, Rc<Playback>) {
        let controller = AnimationController::new(Duration::from_millis(100), curves::LINEAR);
        controller.forward_from_first_tick();
        let playback = Rc::new(Playback {
            controller,
            effect,
            finished: Cell::new(false),
            swipe: None,
            outgoing: RefCell::new(Some(ChildBuilder::from_widget(ErrorWidget::new("outgoing")))),
        });
        let element = NavigationFrame {
            child: ChildBuilder::from_widget(ErrorWidget::new("incoming")),
            playback: Some(Rc::clone(&playback)),
            swipe: None,
        }
        .to_element(context);
        (element, playback)
    }

    fn fixed_effect(on_top: bool) -> ResolvedNavigationTransition {
        ResolvedNavigationTransition::new(Duration::from_millis(100), curves::LINEAR, |_, _| {
            NavigationTransitionFrame { outgoing: opacity(0.25), incoming: opacity(0.75) }
        })
        .unwrap()
        .outgoing_on_top(on_top)
    }

    /// A host node with the two page nodes under it, as retained sync builds it.
    pub(super) fn tree(host_bounds: Rect) -> (RenderTree, RenderNodeId, [RenderNodeId; 2]) {
        let tree = RenderTree::new();
        let root = tree.add_root(host_bounds).unwrap();
        let first = tree.add_child(root, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let second = tree.add_child(root, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        (tree, root, [first, second])
    }

    pub(super) fn sync(element: &AnyElement, tree: &RenderTree, root: RenderNodeId, context: &BuildContext) -> bool {
        let retained = tree.context(root).unwrap();
        context.with_local_v2_paint_context(retained, |context| element.sync_local_v2_state(context))
    }

    #[test]
    fn the_departing_page_is_painted_below_by_default_and_above_on_request() {
        let context = context();
        for (on_top, expected) in [(false, [1, 0]), (true, [0, 1])] {
            let element = host(fixed_effect(on_top), &context);
            let mut visited = Vec::new();

            element.visit_retained_v2_children(&mut |index, _| visited.push(index));

            // Structural slot 0 is the arriving page, 1 the departing one;
            // later in the order is painted on top.
            assert_eq!(visited, expected, "outgoing_on_top = {on_top}");
        }
    }

    #[test]
    fn each_page_gets_its_own_presentation_in_either_paint_order() {
        let context = context();
        // (outgoing_on_top, opacity of the first painted node, of the second)
        for (on_top, first, second) in [(false, 0.25, 0.75), (true, 0.75, 0.25)] {
            let element = host(fixed_effect(on_top), &context);
            let (tree, root, [a, b]) = tree(Rect::new(0.0, 0.0, 100.0, 100.0));

            assert!(sync(&element, &tree, root, &context));

            assert_eq!(tree.presentation(a).unwrap().1, first, "outgoing_on_top = {on_top}");
            assert_eq!(tree.presentation(b).unwrap().1, second, "outgoing_on_top = {on_top}");
        }
    }

    #[test]
    fn the_sampler_is_told_where_the_host_is_in_the_window() {
        let context = context();
        let seen = Rc::new(Cell::new((f32::NAN, f32::NAN)));
        let recorded = seen.clone();
        let effect = ResolvedNavigationTransition::positioned(
            Duration::from_millis(100),
            curves::LINEAR,
            move |_, _, origin| {
                recorded.set(origin);
                NavigationTransitionFrame::FINISHED
            },
        )
        .unwrap();
        let element = host(effect, &context);
        let (tree, root, _) = tree(Rect::new(30.0, 40.0, 100.0, 100.0));

        sync(&element, &tree, root, &context);

        assert_eq!(seen.get(), (30.0, 40.0));
    }

    fn anchored_effect(on_top: bool) -> ResolvedNavigationTransition {
        let content = aimer_animation::SharedContent::new(|| ErrorWidget::new("copy").boxed());
        ResolvedNavigationTransition::anchored(
            Duration::from_millis(100),
            curves::LINEAR,
            AnchorContent::new(content, ResolvedSize { width: 40.0, height: 30.0 }),
            |_, _, _| AnchoredFrame {
                frame: NavigationTransitionFrame { outgoing: opacity(0.25), incoming: opacity(0.75) },
                anchor: opacity(0.5),
            },
        )
        .unwrap()
        .outgoing_on_top(on_top)
    }

    #[test]
    fn the_anchor_is_painted_above_both_pages_either_way() {
        let context = context();
        for (on_top, expected) in [(false, [1, 0, 2]), (true, [0, 1, 2])] {
            let element = host(anchored_effect(on_top), &context);
            let mut visited = Vec::new();

            element.visit_retained_v2_children(&mut |index, _| visited.push(index));

            // Structural slot 2 is the anchor copy, always painted last.
            assert_eq!(visited, expected, "outgoing_on_top = {on_top}");
        }
    }

    #[test]
    fn the_anchor_gets_its_own_presentation() {
        let context = context();
        for (on_top, expected) in [(false, [0.25, 0.75, 0.5]), (true, [0.75, 0.25, 0.5])] {
            let element = host(anchored_effect(on_top), &context);
            let tree = RenderTree::new();
            let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
            let nodes: Vec<_> = (0..3)
                .map(|_| tree.add_child(root, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap())
                .collect();

            assert!(sync(&element, &tree, root, &context));

            let opacities: Vec<_> = nodes.iter().map(|node| tree.presentation(*node).unwrap().1).collect();
            assert_eq!(opacities, expected, "outgoing_on_top = {on_top}");
        }
    }

    /// A growing window on the arriving page: moved by (10, 20), showing 40 x 30
    /// of it with rounded corners, and not yet opaque.
    fn window() -> PaintPresentation {
        PaintPresentation {
            transform: Mat3::translate(10.0, 20.0),
            opacity: 0.75,
            corner_radius: 8.0,
            reveal: Some(aimer_animation::ElementBounds::new(0.0, 0.0, 40.0, 30.0)),
        }
    }

    fn veiled_window_effect(
        above: bool,
        follows: Option<crate::TransitionPage>,
        outgoing_on_top: bool,
    ) -> ResolvedNavigationTransition {
        let effect = ResolvedNavigationTransition::new(Duration::from_millis(100), curves::LINEAR, |_, _| {
            NavigationTransitionFrame { outgoing: opacity(0.25), incoming: window() }
        })
        .unwrap()
        .outgoing_on_top(outgoing_on_top);
        let effect = if above {
            effect.with_veil_above(|_| SizedBox::new().boxed())
        } else {
            effect.with_veil(|_| SizedBox::new().boxed())
        };
        match follows {
            Some(page) => effect.veil_follows(page),
            None => effect,
        }
    }

    /// The nodes of a host with an outgoing page, an incoming page and a veil,
    /// in retained paint order, and the render tree that owns them.
    fn veiled_tree() -> (RenderTree, RenderNodeId, Vec<RenderNodeId>) {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let nodes = (0..3)
            .map(|_| tree.add_child(root, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap())
            .collect();
        (tree, root, nodes)
    }

    fn clip_of(tree: &RenderTree, node: RenderNodeId) -> (Option<Rect>, [f32; 4]) {
        tree.render_all()
            .into_iter()
            .find_map(|operation| match operation {
                aimer_cupid::draw_cmd_v2::RenderOp::Draw(item) if item.element == node => {
                    Some((item.clip, item.clip_radius))
                }
                _ => None,
            })
            .expect("the node is drawn")
    }

    #[test]
    fn a_veil_above_follows_the_arriving_page_window_at_full_strength() {
        let context = context();
        let element = host(veiled_window_effect(true, Some(crate::TransitionPage::Incoming), false), &context);
        let (tree, root, nodes) = veiled_tree();

        assert!(sync(&element, &tree, root, &context));

        // Paint order: the departing page, the arriving page, then the veil above.
        let veil = nodes[2];
        let (transform, opacity) = tree.presentation(veil).unwrap();
        assert_eq!(transform, Mat3::translate(10.0, 20.0), "moved with the window");
        assert_eq!(opacity, 1.0, "the page's own opacity does not weaken the blur");
        assert_eq!(
            clip_of(&tree, veil),
            (Some(Rect::new(10.0, 20.0, 40.0, 30.0)), [8.0; 4]),
            "clipped to the window, with its corners"
        );
        assert_eq!(tree.presentation(nodes[1]).unwrap().1, 0.75, "the page keeps its own opacity");
    }

    #[test]
    fn a_veil_can_follow_the_departing_page_instead() {
        let context = context();
        let element = host(veiled_window_effect(true, Some(crate::TransitionPage::Outgoing), false), &context);
        let (tree, root, nodes) = veiled_tree();

        assert!(sync(&element, &tree, root, &context));

        let veil = nodes[2];
        assert_eq!(tree.presentation(veil).unwrap(), (Mat3::identity(), 1.0));
        assert_eq!(clip_of(&tree, veil), (None, [0.0; 4]), "the departing page here has no window to follow");
    }

    #[test]
    fn a_stretched_page_is_clipped_to_the_rectangle_it_is_shown_in_with_its_corners_as_shown() {
        let context = context();
        let stretched = PaintPresentation {
            transform: Mat3::translate(10.0, 20.0).mul(&Mat3::scale(0.4, 0.3)),
            opacity: 1.0,
            corner_radius: 8.0,
            reveal: None,
        };
        let effect = ResolvedNavigationTransition::new(Duration::from_millis(100), curves::LINEAR, move |_, _| {
            NavigationTransitionFrame { outgoing: opacity(0.25), incoming: stretched }
        })
        .unwrap();
        let element = host(effect, &context);
        let (tree, root, nodes) = veiled_tree();

        assert!(sync(&element, &tree, root, &context));

        // The 100 x 100 page, scaled to 40 x 30 and moved to (10, 20): that is where it
        // is shown, and where it is clipped, whatever it is laid out as.
        let (clip, radius) = clip_of(&tree, nodes[1]);
        let clip = clip.expect("the stretched page is clipped to where it is shown");
        assert!((clip.x - 10.0).abs() < 1e-3 && (clip.y - 20.0).abs() < 1e-3, "{clip:?}");
        assert!((clip.width - 40.0).abs() < 1e-3 && (clip.height - 30.0).abs() < 1e-3, "{clip:?}");
        assert!(radius.iter().all(|corner| (corner - 8.0).abs() < 0.5), "corners as shown: {radius:?}");
    }

    #[test]
    fn a_veil_between_the_pages_follows_a_page_too() {
        let context = context();
        let element = host(veiled_window_effect(false, Some(crate::TransitionPage::Incoming), false), &context);
        let (tree, root, nodes) = veiled_tree();

        assert!(sync(&element, &tree, root, &context));

        // Paint order: the departing page, the veil, the arriving page.
        assert_eq!(tree.presentation(nodes[0]).unwrap().1, 0.25);
        assert_eq!(tree.presentation(nodes[1]).unwrap().0, Mat3::translate(10.0, 20.0));
        assert_eq!(clip_of(&tree, nodes[1]).0, Some(Rect::new(10.0, 20.0, 40.0, 30.0)));
        assert_eq!(tree.presentation(nodes[2]).unwrap().1, 0.75);
    }

    #[test]
    fn a_veil_the_effect_leaves_alone_still_fills_the_host() {
        let context = context();
        let element = host(veiled_window_effect(true, None, false), &context);
        let (tree, root, nodes) = veiled_tree();

        assert!(sync(&element, &tree, root, &context));

        assert_eq!(tree.presentation(nodes[2]).unwrap(), (Mat3::identity(), 1.0));
        assert_eq!(clip_of(&tree, nodes[2]).0, None);
    }

    #[test]
    fn the_veil_ordinal_follows_the_anchor_copy_when_there_is_one() {
        let context = context();
        let content = aimer_animation::SharedContent::new(|| ErrorWidget::new("copy").boxed());
        let effect = ResolvedNavigationTransition::anchored(
            Duration::from_millis(100),
            curves::LINEAR,
            AnchorContent::new(content, ResolvedSize { width: 40.0, height: 30.0 }),
            |_, _, _| AnchoredFrame {
                frame: NavigationTransitionFrame { outgoing: opacity(0.25), incoming: window() },
                anchor: opacity(0.5),
            },
        )
        .unwrap()
        .with_veil_above(|_| SizedBox::new().boxed())
        .veil_follows(crate::TransitionPage::Incoming);
        let element = host(effect, &context);
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let nodes: Vec<_> = (0..4)
            .map(|_| tree.add_child(root, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap())
            .collect();

        assert!(sync(&element, &tree, root, &context));

        // Painted: departing page, arriving page, the item's copy, the veil.
        assert_eq!(tree.presentation(nodes[2]).unwrap().1, 0.5, "the copy keeps its own presentation");
        assert_eq!(tree.presentation(nodes[3]).unwrap(), (Mat3::translate(10.0, 20.0), 1.0));
    }

    #[test]
    fn the_anchor_takes_no_input_and_is_dropped_when_the_transition_ends() {
        let context = context();
        let (element, playback) = host_with_playback(anchored_effect(false), &context);
        let mut events = 0;
        element.event_children(&mut |_| events += 1);
        assert_eq!(events, 1, "only the arriving page takes input");
        let mut focusable = 0;
        element.focus_children(&mut |_| focusable += 1);
        assert_eq!(focusable, 1, "and only the arriving page takes focus");
        let mut structural = 0;
        element.structural_children(&mut |_| structural += 1);
        assert_eq!(structural, 3, "all three still take part in reconciliation");
        let mut visited = 0;
        element.visit_retained_v2_children(&mut |_, _| visited += 1);
        assert_eq!(visited, 3);

        // The transition ends: the next frame removes both the page that left and the copy.
        playback.finish();
        element.update(&context);

        let mut remaining = Vec::new();
        element.visit_retained_v2_children(&mut |index, _| remaining.push(index));
        assert_eq!(remaining, [0]);
    }
}
