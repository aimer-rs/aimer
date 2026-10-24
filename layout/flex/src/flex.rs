pub(crate) mod children_source;
#[doc(hidden)]
pub mod flex_child;
pub(crate) mod flex_layout;
pub mod flex_list;
pub mod layout_transition;
#[cfg(test)]
mod lazy_tests {
    //! Tests for the viewport-proportional behaviour of [`RawFlex`].
    //!
    //! A flex container measures and paints through one cached main-axis table, so
    //! the work a frame does is meant to follow the viewport rather than the child
    //! count. These tests pin that with a hundred-thousand-child column: the probe
    //! children count every measure and every paint they receive, so a regression to
    //! an `O(children)` pass shows up as a number instead of a slowdown.

    use std::cell::Cell;
    use std::rc::Rc;

    use std::cell::RefCell;

    use aimer_events::element::ElementEvent;

    use std::time::Instant;

    use aimer_widget::base::{BuildContext, ResolvedSize, Vec2d};
    use aimer_widget::{
        AnyElement, Drawable, Element, EventElement, EventTreeRole, LayoutElement, Rebuildable,
        VisitorElement,
    };

    use aimer_widget::{EventDispatcher, EventResult, Widget};

    use crate::flex::raw_flex::RawFlex;
    use crate::flex::test_support::{
        CountingChild, CountingSource, KeptChild, ResizingChild, VolatileChild,
        dummy_build_context, replace_a_generated_subtree,
    };
    use crate::flex::{Column, FlexDirection, OverflowBehavior};

    const CHILD_COUNT: usize = 100_000;
    const CHILD_HEIGHT: f32 = 80.0;
    const VIEWPORT: f32 = 600.0;

    struct HitTestChild {
        bounds: (Vec2d, Vec2d),
    }

    impl HitTestChild {
        fn boxed(bounds: (Vec2d, Vec2d)) -> AnyElement {
            Self { bounds }.boxed()
        }
    }

    impl VisitorElement for HitTestChild {
        fn debug_name(&self) -> &'static str {
            "HitTestChild"
        }
    }

    impl EventElement for HitTestChild {}
    impl Rebuildable for HitTestChild {}
    impl Drawable for HitTestChild {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl LayoutElement for HitTestChild {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: 100.0,
                height: 20.0,
            }
        }

        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some(self.bounds)
        }
    }

    struct CountingHitTestChild {
        bounds: (Vec2d, Vec2d),
        pos_start_end_calls: Rc<Cell<usize>>,
    }

    impl VisitorElement for CountingHitTestChild {
        fn debug_name(&self) -> &'static str {
            "CountingHitTestChild"
        }
    }

    impl EventElement for CountingHitTestChild {}
    impl Rebuildable for CountingHitTestChild {}
    impl Drawable for CountingHitTestChild {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl LayoutElement for CountingHitTestChild {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: 100.0,
                height: 10.0,
            }
        }

        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            self.pos_start_end_calls
                .set(self.pos_start_end_calls.get() + 1);
            Some(self.bounds)
        }
    }

    fn tall_column(measured: &Rc<Cell<usize>>, drawn: &Rc<Cell<usize>>) -> RawFlex {
        let children = (0..CHILD_COUNT)
            .map(|_| CountingChild::boxed_new(200.0, CHILD_HEIGHT, measured, drawn))
            .collect();
        RawFlex::new(FlexDirection::Column, children, "Column")
    }

    #[test]
    fn expanded_is_transparent_to_event_routing() {
        let expanded = crate::flex::flex_child::RawExpanded::new(
            HitTestChild {
                bounds: (Vec2d::ZERO, Vec2d { x: 100.0, y: 20.0 }),
            },
            1.0,
            "Expanded",
        );

        assert_eq!(expanded.event_tree_role(), EventTreeRole::Transparent);
    }

    /// A `Column` under a viewport must paint only the children intersecting it,
    /// and must not re-measure the whole list to find them.
    #[test]
    fn draw_only_touches_the_visible_children() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let column = tall_column(&measured, &drawn);
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));

        // The first pass has to size the list once to know the scroll extent.
        column.computed_size(&ctx);
        measured.set(0);
        drawn.set(0);

        column.update(&ctx);

        let visible = (VIEWPORT / CHILD_HEIGHT).ceil() as usize + 1;
        assert!(
            drawn.get() <= visible,
            "painted {} children for a {VIEWPORT}px viewport",
            drawn.get()
        );
        assert!(
            measured.get() <= visible,
            "measured {} children while painting {} of them",
            measured.get(),
            drawn.get()
        );
    }

    /// Scrolling only changes the offset, so a later frame must stay cheap and
    /// paint the slice that the offset exposes.
    #[test]
    fn scrolled_draw_stays_proportional_to_the_viewport() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let column = tall_column(&measured, &drawn);
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));
        column.update(&ctx);

        // Emulate a `Scrollable` that has scrolled 4_000 children down.
        let offset = 4_000.0 * CHILD_HEIGHT;
        let scrolled = dummy_build_context(200.0, VIEWPORT, Some((0.0, offset, 200.0, VIEWPORT)));
        measured.set(0);
        drawn.set(0);
        column.update(&scrolled);

        let visible = (VIEWPORT / CHILD_HEIGHT).ceil() as usize + 1;
        assert!(
            drawn.get() > 0 && drawn.get() <= visible,
            "painted {} children after scrolling",
            drawn.get()
        );
        assert!(
            measured.get() <= visible,
            "measured {} children after scrolling",
            measured.get()
        );
    }

    /// Hit testing must only consider the children of the last painted frame;
    /// nothing else can be under the pointer.
    #[test]
    fn hit_testing_visits_only_the_painted_children() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let column = tall_column(&measured, &drawn);
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));
        column.update(&ctx);

        let mut hit_tested = 0;
        column.hit_test_children(&mut |_| hit_tested += 1);

        assert_eq!(hit_tested, drawn.get());
    }

    #[test]
    fn position_aware_hit_testing_prunes_offscreen_flex_children() {
        let column = RawFlex::new(
            FlexDirection::Column,
            vec![
                HitTestChild::boxed((Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 100.0, y: 20.0 })),
                HitTestChild::boxed((Vec2d { x: 0.0, y: 20.0 }, Vec2d { x: 100.0, y: 40.0 })),
                HitTestChild::boxed((Vec2d { x: 0.0, y: 40.0 }, Vec2d { x: 100.0, y: 60.0 })),
            ],
            "Column",
        );
        let ctx = dummy_build_context(100.0, 100.0, Some((0.0, 0.0, 100.0, 100.0)));
        column.update(&ctx);

        let pos = Vec2d { x: 50.0, y: 30.0 };
        let mut forward = 0;
        column.hit_test_children_at(pos, &mut |_| forward += 1);
        assert_eq!(forward, 1);

        let mut reversed = 0;
        column.hit_test_children_at_reversed(pos, &mut |_| reversed += 1);
        assert_eq!(reversed, 1);
    }

    #[test]
    fn raw_flex_declares_an_indexed_hit_test_boundary() {
        let column = RawFlex::new(FlexDirection::Column, Vec::new(), "Column");

        assert_eq!(
            column.event_tree_role(),
            EventTreeRole::IndexedHitTestBoundary
        );
    }

    struct DispatchProbe {
        id: usize,
        role: EventTreeRole,
        events: Rc<RefCell<Vec<usize>>>,
    }

    impl VisitorElement for DispatchProbe {
        fn debug_name(&self) -> &'static str {
            "DispatchProbe"
        }
    }

    impl EventElement for DispatchProbe {
        fn event_tree_role(&self) -> EventTreeRole {
            self.role
        }

        fn on_event(&self, _event: &ElementEvent) -> EventResult {
            self.events.borrow_mut().push(self.id);
            EventResult::ignored()
        }
    }

    impl Drawable for DispatchProbe {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl LayoutElement for DispatchProbe {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: 100.0,
                height: 10.0,
            }
        }

        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            let top = self.id as f32 * 10.0;
            Some((
                Vec2d { x: 0.0, y: top },
                Vec2d {
                    x: 100.0,
                    y: top + 10.0,
                },
            ))
        }
    }

    impl Rebuildable for DispatchProbe {}

    struct DispatchProbeWidget {
        id: usize,
        role: EventTreeRole,
        events: Rc<RefCell<Vec<usize>>>,
        wrapper_depth: usize,
    }

    impl Widget for DispatchProbeWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            wrap_dispatch_probe(
                DispatchProbe {
                    id: self.id,
                    role: self.role,
                    events: self.events,
                }
                .boxed(),
                self.wrapper_depth,
            )
        }
    }

    impl aimer_widget::PortableWidget for DispatchProbeWidget {}

    struct TransparentDispatchWrapper {
        child: AnyElement,
    }

    impl VisitorElement for TransparentDispatchWrapper {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }

        fn debug_name(&self) -> &'static str {
            "TransparentDispatchWrapper"
        }
    }

    impl EventElement for TransparentDispatchWrapper {
        fn event_tree_role(&self) -> EventTreeRole {
            EventTreeRole::Transparent
        }

        fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }
    }

    impl Drawable for TransparentDispatchWrapper {
        fn update(&self, ctx: &BuildContext) {
            self.child.update(ctx);
        }
    }

    impl LayoutElement for TransparentDispatchWrapper {
        fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
            self.child.computed_size(ctx)
        }

        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            self.child.pos_start_end()
        }
    }

    impl Rebuildable for TransparentDispatchWrapper {}

    fn wrap_dispatch_probe(mut child: AnyElement, depth: usize) -> AnyElement {
        for _ in 0..depth {
            child = TransparentDispatchWrapper { child }.boxed();
        }
        child
    }

    fn build_dispatch_fixture(
        child_count: usize,
        wrapper_depth: usize,
        boundary_child: Option<usize>,
        viewport_height: f32,
    ) -> (
        AnyElement,
        EventDispatcher,
        Rc<RefCell<Vec<usize>>>,
        Vec2d,
        ElementEvent,
        std::time::Duration,
    ) {
        let started = Instant::now();
        let events = Rc::new(RefCell::new(Vec::new()));
        let ctx = dummy_build_context(
            100.0,
            viewport_height,
            Some((0.0, 0.0, 100.0, viewport_height)),
        );
        let children: Vec<DispatchProbeWidget> = (0..child_count)
            .map(|id| {
                let role = if boundary_child == Some(id) {
                    EventTreeRole::IndexedHitTestBoundary
                } else {
                    EventTreeRole::IndexedTarget
                };
                DispatchProbeWidget {
                    id,
                    role,
                    events: events.clone(),
                    wrapper_depth,
                }
            })
            .collect();
        let column = Column::new().children(children).to_element(&ctx);
        column.update(&ctx);

        let pos = Vec2d { x: 50.0, y: 5.0 };
        let event = ElementEvent::DragOver {
            pos,
            source: aimer_events::pointer::PointerSource::Mouse,
            id: 7,
        };
        let mut dispatcher = EventDispatcher::new();
        let _ = dispatcher.dispatch(column.as_ref(), pos, &event);
        events.borrow_mut().clear();
        (column, dispatcher, events, pos, event, started.elapsed())
    }

    fn dispatch_through_test_column(add_boundary_child: bool) -> (Vec<usize>, Vec<usize>) {
        let (column, mut dispatcher, events, pos, event, _) =
            build_dispatch_fixture(5, 0, add_boundary_child.then_some(4), 15.0);
        let _ = dispatcher.dispatch(column.as_ref(), pos, &event);
        let pointer_hits = std::mem::take(&mut *events.borrow_mut());

        let _ = aimer_widget::broadcast_event(column.as_ref(), &ElementEvent::Cancel);
        let broadcast_hits = std::mem::take(&mut *events.borrow_mut());

        (pointer_hits, broadcast_hits)
    }

    #[test]
    fn indexed_flex_dispatch_matches_boundary_child_and_broadcast_views() {
        let indexed = dispatch_through_test_column(false);
        let general = dispatch_through_test_column(true);

        assert_eq!(indexed, general);
        assert!(
            !indexed.0.contains(&4),
            "an unpainted child received a pointer event"
        );
        assert_eq!(indexed.1, [4, 3, 2, 1, 0]);
    }

    #[test]
    #[ignore = "manual debug-profile comparison of warmed production RawFlex routes"]
    fn compare_warmed_direct_and_boundary_indexed_flex_dispatch() {
        const CHILD_COUNT: usize = 10_000;
        const WRAPPER_DEPTH: usize = 6;
        const ROUTES: usize = 2_000;
        const VIEWPORT_HEIGHT: f32 = 40.0;

        // Warm the headless canvas and allocator before measuring either path.
        let _ = build_dispatch_fixture(8, 1, Some(7), VIEWPORT_HEIGHT);
        let (boundary_root, mut boundary_dispatcher, boundary_events, pos, event, boundary_setup) =
            build_dispatch_fixture(
                CHILD_COUNT,
                WRAPPER_DEPTH,
                Some(CHILD_COUNT - 1),
                VIEWPORT_HEIGHT,
            );
        let (direct_root, mut direct_dispatcher, direct_events, _, _, indexed_setup) =
            build_dispatch_fixture(CHILD_COUNT, WRAPPER_DEPTH, None, VIEWPORT_HEIGHT);

        let mut direct_samples = Vec::new();
        for _ in 0..ROUTES {
            direct_events.borrow_mut().clear();
            let result = direct_dispatcher.dispatch(direct_root.as_ref(), pos, &event);
            assert_eq!(result, EventResult::ignored());
            direct_samples.push(std::mem::take(&mut *direct_events.borrow_mut()));
        }
        let mut boundary_samples = Vec::new();
        for _ in 0..ROUTES {
            boundary_events.borrow_mut().clear();
            let result = boundary_dispatcher.dispatch(boundary_root.as_ref(), pos, &event);
            assert_eq!(result, EventResult::ignored());
            boundary_samples.push(std::mem::take(&mut *boundary_events.borrow_mut()));
        }
        assert_eq!(direct_samples, boundary_samples);

        let direct_elapsed = {
            let start = Instant::now();
            for _ in 0..ROUTES {
                direct_events.borrow_mut().clear();
                let _ = direct_dispatcher.dispatch(direct_root.as_ref(), pos, &event);
            }
            start.elapsed()
        };
        let boundary_elapsed = {
            let start = Instant::now();
            for _ in 0..ROUTES {
                boundary_events.borrow_mut().clear();
                let _ = boundary_dispatcher.dispatch(boundary_root.as_ref(), pos, &event);
            }
            start.elapsed()
        };

        eprintln!(
            "RawFlex {CHILD_COUNT} children, {WRAPPER_DEPTH} transparent wrappers/item, {ROUTES} warmed routes: setup boundary={boundary_setup:?}, direct={indexed_setup:?}; route direct={:.2} us, boundary={:.2} us",
            direct_elapsed.as_secs_f64() * 1_000_000.0 / ROUTES as f64,
            boundary_elapsed.as_secs_f64() * 1_000_000.0 / ROUTES as f64,
        );
    }

    #[test]
    fn wide_flex_hit_testing_rejects_non_intersecting_children_before_bounds_lookup() {
        const CHILD_COUNT: usize = 128;
        let pos_start_end_calls = Rc::new(Cell::new(0));
        let mut column = RawFlex::new(
            FlexDirection::Column,
            (0..CHILD_COUNT)
                .map(|index| {
                    CountingHitTestChild {
                        bounds: (
                            Vec2d {
                                x: 0.0,
                                y: index as f32 * 10.0,
                            },
                            Vec2d {
                                x: 100.0,
                                y: index as f32 * 10.0 + 10.0,
                            },
                        ),
                        pos_start_end_calls: Rc::clone(&pos_start_end_calls),
                    }
                    .boxed()
                })
                .collect(),
            "Column",
        );
        let ctx = dummy_build_context(100.0, CHILD_COUNT as f32 * 10.0, None);
        column.overflow_behavior = OverflowBehavior::Visible;
        column.update(&ctx);
        pos_start_end_calls.set(0);

        let mut hits = 0;
        column.hit_test_children_at_reversed(Vec2d { x: 50.0, y: 1005.0 }, &mut |_| hits += 1);

        assert_eq!(hits, 1);
        assert!(
            pos_start_end_calls.get() < CHILD_COUNT,
            "wide flex queried {} of {} child bounds",
            pos_start_end_calls.get(),
            CHILD_COUNT
        );
    }

    /// A hidden root flex has a finite painting box even when no ancestor
    /// supplied a visible rectangle. Its child window should therefore stay
    /// proportional to that box instead of retaining every eager child.
    #[test]
    fn hidden_flex_without_visible_rect_culls_children_to_its_bounds() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let column = tall_column(&measured, &drawn);
        let ctx = dummy_build_context(200.0, VIEWPORT, None);

        column.update(&ctx);

        let visible = (VIEWPORT / CHILD_HEIGHT).ceil() as usize + 1;
        assert!(
            drawn.get() > 0 && drawn.get() <= visible,
            "painted {} children for a {VIEWPORT}px hidden flex",
            drawn.get()
        );

        let mut hit_tested = 0;
        column.hit_test_children(&mut |_| hit_tested += 1);
        assert_eq!(hit_tested, drawn.get());
    }

    /// An unrelated generated subtree advances the global generation, but it
    /// must not force a stable child list to measure every child again.
    #[test]
    fn stable_children_reuse_their_measured_layout_after_unrelated_rebuild() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let column = RawFlex::new(
            FlexDirection::Column,
            (0..256)
                .map(|_| CountingChild::boxed_new(200.0, CHILD_HEIGHT, &measured, &drawn))
                .collect(),
            "Column",
        );
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));

        assert_eq!(column.computed_size(&ctx).height, 256.0 * CHILD_HEIGHT);
        measured.set(0);
        replace_a_generated_subtree(&ctx);

        assert_eq!(column.computed_size(&ctx).height, 256.0 * CHILD_HEIGHT);
        assert_eq!(measured.get(), 0);
    }

    /// One child that cannot promise a stable size must not make every stable
    /// sibling measure again when an unrelated subtree is replaced: only the
    /// unstable child is asked, and a size that did not change leaves the
    /// table in place.
    #[test]
    fn only_the_unstable_child_is_remeasured_after_unrelated_rebuild() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let volatile_measured = Rc::new(Cell::new(0));
        let volatile_height = Rc::new(Cell::new(30.0));
        let mut children = vec![VolatileChild::boxed_new(
            &volatile_height,
            &volatile_measured,
        )];
        children.extend(
            (0..256).map(|_| CountingChild::boxed_new(200.0, CHILD_HEIGHT, &measured, &drawn)),
        );
        let column = RawFlex::new(FlexDirection::Column, children, "Column");
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));
        let total = 30.0 + 256.0 * CHILD_HEIGHT;

        assert_eq!(column.computed_size(&ctx).height, total);
        measured.set(0);
        volatile_measured.set(0);
        replace_a_generated_subtree(&ctx);

        assert_eq!(column.computed_size(&ctx).height, total);
        assert_eq!(measured.get(), 0, "stable children were measured again");
        assert_eq!(
            volatile_measured.get(),
            1,
            "the unstable child must be asked once"
        );
    }

    /// The shortcut must not outlive the size it was checked against: when the
    /// unstable child changes, the table is rebuilt and the siblings move.
    #[test]
    fn a_resized_unstable_child_rebuilds_the_table_after_unrelated_rebuild() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let volatile_measured = Rc::new(Cell::new(0));
        let volatile_height = Rc::new(Cell::new(30.0));
        let mut children = vec![VolatileChild::boxed_new(
            &volatile_height,
            &volatile_measured,
        )];
        children.extend(
            (0..16).map(|_| CountingChild::boxed_new(200.0, CHILD_HEIGHT, &measured, &drawn)),
        );
        let column = RawFlex::new(FlexDirection::Column, children, "Column");
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));
        assert_eq!(
            column.computed_size(&ctx).height,
            30.0 + 16.0 * CHILD_HEIGHT
        );

        volatile_height.set(80.0);
        replace_a_generated_subtree(&ctx);

        assert_eq!(
            column.computed_size(&ctx).height,
            80.0 + 16.0 * CHILD_HEIGHT
        );
    }

    /// Once the index of dirty paths is ready, the rebuild pass reaches the
    /// children it leads to by position: every one of them, and none of the
    /// rest, however long the list.
    #[test]
    fn the_rebuild_pass_enters_only_the_children_dirty_paths_lead_to() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let first_rebuilt = Rc::new(Cell::new(0));
        let second_rebuilt = Rc::new(Cell::new(0));
        let mut children = vec![KeptChild::boxed_new(&first_rebuilt)];
        children.extend(
            (0..256).map(|_| CountingChild::boxed_new(200.0, CHILD_HEIGHT, &measured, &drawn)),
        );
        children.push(KeptChild::boxed_new(&second_rebuilt));
        let root = RawFlex::new(FlexDirection::Column, children, "Column").boxed();
        let ctx = dummy_build_context(200.0, VIEWPORT, None);

        // The first pass walks everything and registers the kept children.
        root.rebuild_if_dirty(&ctx);
        assert_eq!((first_rebuilt.get(), second_rebuilt.get()), (1, 1));

        let entered = aimer_widget::rebuild_visit_count();
        root.rebuild_if_dirty(&ctx);
        let entered = aimer_widget::rebuild_visit_count() - entered;

        assert_eq!(
            (first_rebuilt.get(), second_rebuilt.get()),
            (2, 2),
            "both kept children must be rebuilt again"
        );
        assert!(
            entered <= 3,
            "the pass entered {entered} boundaries for 2 kept children"
        );
    }

    /// Which children changed is recorded as it happens, so revalidating a
    /// long list after an unrelated rebuild must not look at every child: only
    /// the ones that cannot vouch for themselves are asked.
    #[test]
    fn revalidation_does_not_look_at_children_that_did_not_change() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let volatile_measured = Rc::new(Cell::new(0));
        let volatile_height = Rc::new(Cell::new(30.0));
        let asked = Rc::new(Cell::new(0));
        let mut children = vec![VolatileChild::boxed_new(
            &volatile_height,
            &volatile_measured,
        )];
        children.extend(
            (0..256).map(|_| CountingChild::boxed_new(200.0, CHILD_HEIGHT, &measured, &drawn)),
        );
        let mut column = RawFlex::new(FlexDirection::Column, Vec::new(), "Column");
        column.children = Box::new(CountingSource::new(children, &asked));
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));
        let total = 30.0 + 256.0 * CHILD_HEIGHT;
        assert_eq!(column.computed_size(&ctx).height, total);

        replace_a_generated_subtree(&ctx);
        asked.set(0);
        assert_eq!(column.computed_size(&ctx).height, total);

        assert!(
            asked.get() <= 4,
            "{} of 257 children were looked at",
            asked.get()
        );
        assert_eq!(
            measured.get(),
            256,
            "stable children are only measured by the first pass"
        );
    }

    /// A stable child whose subtree was replaced has to be asked again, even
    /// though nothing else about the list changed.
    #[test]
    fn a_stable_child_with_a_new_generation_is_remeasured() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let volatile_measured = Rc::new(Cell::new(0));
        let volatile_height = Rc::new(Cell::new(30.0));
        let asked = Rc::new(Cell::new(0));
        let mut children = vec![VolatileChild::boxed_new(
            &volatile_height,
            &volatile_measured,
        )];
        children.extend(
            (0..64).map(|_| CountingChild::boxed_new(200.0, CHILD_HEIGHT, &measured, &drawn)),
        );
        let mut column = RawFlex::new(FlexDirection::Column, Vec::new(), "Column");
        column.children = Box::new(CountingSource::new(children, &asked));
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));
        column.computed_size(&ctx);
        measured.set(0);

        replace_a_generated_subtree(&ctx);
        let replaced = column.children.get(10).expect("the child exists");
        replaced.set_subtree_generation(aimer_widget::element_tree_generation());
        column.computed_size(&ctx);

        assert_eq!(
            measured.get(),
            1,
            "only the child with a new generation is asked again"
        );
    }

    /// The log of changes is bounded. A container that fell behind it cannot
    /// know which children changed, so it must check all of them.
    #[test]
    fn a_log_that_overflowed_makes_revalidation_check_every_child() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let volatile_measured = Rc::new(Cell::new(0));
        let volatile_height = Rc::new(Cell::new(30.0));
        let asked = Rc::new(Cell::new(0));
        let mut children = vec![VolatileChild::boxed_new(
            &volatile_height,
            &volatile_measured,
        )];
        children.extend(
            (0..64).map(|_| CountingChild::boxed_new(200.0, CHILD_HEIGHT, &measured, &drawn)),
        );
        let mut column = RawFlex::new(FlexDirection::Column, Vec::new(), "Column");
        column.children = Box::new(CountingSource::new(children, &asked));
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));
        column.computed_size(&ctx);
        measured.set(0);

        replace_a_generated_subtree(&ctx);
        let first = column.children.get(10).expect("the child exists");
        let second = column.children.get(20).expect("the child exists");
        second.set_subtree_generation(1_000_000);
        // Far more changes than the log keeps; the earlier ones are forgotten.
        for generation in 0..5_000u64 {
            first.set_subtree_generation(generation + 10);
        }
        column.computed_size(&ctx);

        assert_eq!(
            measured.get(),
            2,
            "both changed children must be found without the log"
        );
    }

    /// Visible overflow is an explicit request to keep children outside the
    /// flex box in the painting and hit-testing set.
    #[test]
    fn visible_flex_without_visible_rect_keeps_overflowing_children() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let mut column = tall_column(&measured, &drawn);
        column.overflow_behavior = OverflowBehavior::Visible;
        let ctx = dummy_build_context(200.0, VIEWPORT, None);

        column.update(&ctx);

        assert_eq!(drawn.get(), CHILD_COUNT);
        let mut hit_tested = 0;
        column.hit_test_children(&mut |_| hit_tested += 1);
        assert_eq!(hit_tested, CHILD_COUNT);
    }

    /// Focus and broadcast delivery must still reach every child, painted or
    /// not, otherwise an off-screen input field would stop receiving keys.
    #[test]
    fn event_children_still_visits_the_whole_list() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let column = tall_column(&measured, &drawn);
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));
        column.update(&ctx);

        let mut visited = 0;
        column.event_children(&mut |_| visited += 1);

        assert_eq!(visited, CHILD_COUNT);
    }

    /// Before the first frame nothing is known to be off-screen, so hit testing
    /// must fall back to the whole list.
    #[test]
    fn hit_testing_before_the_first_frame_visits_everything() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let column = tall_column(&measured, &drawn);

        let mut hit_tested = 0;
        column.hit_test_children(&mut |_| hit_tested += 1);

        assert_eq!(hit_tested, CHILD_COUNT);
    }

    /// A child that resizes itself between frames — an implicitly animated
    /// container does exactly that inside its own `draw` — must still push its
    /// siblings, even though the cached table was measured before the change.
    #[test]
    fn a_resized_child_moves_its_siblings_on_the_next_frame() {
        let height = Rc::new(Cell::new(20.0));
        let first_at = Rc::new(Cell::new((0.0, 0.0)));
        let second_at = Rc::new(Cell::new((0.0, 0.0)));
        let column = RawFlex::new(
            FlexDirection::Column,
            vec![
                ResizingChild::boxed_new(&height, &first_at),
                ResizingChild::boxed_new(&Rc::new(Cell::new(20.0)), &second_at),
            ],
            "Column",
        );
        let ctx = dummy_build_context(200.0, 600.0, Some((0.0, 0.0, 200.0, 600.0)));

        column.update(&ctx);
        assert_eq!(second_at.get().1, 20.0);

        height.set(50.0);
        column.update(&ctx);

        assert_eq!(first_at.get().1, 0.0);
        assert_eq!(second_at.get().1, 50.0);
    }

    /// Replacing the children below a container must retire the cached table.
    ///
    /// A list that grew — an `AsyncBuilder` swapping a spinner for the rows it
    /// waited on — is measured by a `Scrollable` through the container above it. If
    /// that container answers from a table measured before the swap, the scroll
    /// view is told the content still fits and the page will not scroll.
    #[test]
    fn a_replaced_child_list_is_measured_again() {
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));

        // The frame that only had the loading state to show.
        let height = Rc::new(Cell::new(CHILD_HEIGHT));
        let placement = Rc::new(Cell::new((0.0, 0.0)));
        let column = RawFlex::new(
            FlexDirection::Column,
            vec![ResizingChild::boxed_new(&height, &placement)],
            "Column",
        );
        assert_eq!(column.computed_size(&ctx).height, CHILD_HEIGHT);

        // The frame the completed future produced: taller content under the very
        // same container.
        height.set(CHILD_HEIGHT * 20.0);
        replace_a_generated_subtree(&ctx);

        assert_eq!(
            column.computed_size(&ctx).height,
            CHILD_HEIGHT * 20.0,
            "the container reported the extent it had before the rebuild"
        );
    }

    /// A table measured from rows that did not survive the rebuild describes
    /// nothing and must not be trusted, however well the two containers match.
    ///
    /// This is the `Column` an `AsyncBuilder` rebuilds when the data arrives: same
    /// direction, same gaps, same number of children — and completely different
    /// children. Trusting the table there is a page that paints its content and
    /// refuses to scroll, because the `Scrollable` above is told the old extent.
    #[test]
    fn an_adopted_table_is_not_trusted_when_the_rows_were_rebuilt() {
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));
        let placement = Rc::new(Cell::new((0.0, 0.0)));

        // The container that measured the loading state.
        let short = Rc::new(Cell::new(CHILD_HEIGHT));
        let old = RawFlex::new(
            FlexDirection::Column,
            vec![
                ResizingChild::boxed_new(&short, &placement),
                ResizingChild::boxed_new(&short, &placement),
            ],
            "Column",
        );
        assert_eq!(old.computed_size(&ctx).height, CHILD_HEIGHT * 2.0);

        // The container the completed request produced: the same shape, rows built
        // from scratch out of the data that arrived.
        let tall = Rc::new(Cell::new(CHILD_HEIGHT * 20.0));
        let new = RawFlex::new(
            FlexDirection::Column,
            vec![
                ResizingChild::boxed_new(&tall, &placement),
                ResizingChild::boxed_new(&tall, &placement),
            ],
            "Column",
        );
        new.adopt_runtime_state_from(&old as &dyn Element);
        replace_a_generated_subtree(&ctx);

        assert_eq!(
            new.computed_size(&ctx).height,
            CHILD_HEIGHT * 40.0,
            "the rebuilt container answered from the table its predecessor measured"
        );
    }

    /// The total main-axis extent must survive the lazy path: 100_000 children
    /// of 80px are 8_000_000px tall, which `f32` cannot accumulate exactly.
    #[test]
    fn computed_size_reports_the_full_extent() {
        let measured = Rc::new(Cell::new(0));
        let drawn = Rc::new(Cell::new(0));
        let column = tall_column(&measured, &drawn);
        let ctx = dummy_build_context(200.0, VIEWPORT, Some((0.0, 0.0, 200.0, VIEWPORT)));

        let size = column.computed_size(&ctx);

        assert_eq!(size.width, 200.0);
        assert_eq!(size.height, CHILD_COUNT as f32 * CHILD_HEIGHT);
    }
}
#[doc(hidden)]
pub mod raw_flex;
pub mod row_column;
#[cfg(test)]
mod layout_cache_tests;
#[cfg(test)]
mod saturation_tests;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod wrap_layout;
#[cfg(test)]
mod wrap_retained_tests;

// pub use raw_flex::RawFlex;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_widget::base::BuildContext;
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{PortableBuildContext, PortableBuildError, PortableEncodeProperty};
use aimer_widget::portable::{
    PortableMaterializeError, PortableMaterializeProperty, PortableProperty,
    PortablePropertyConversion, PortablePropertyReflection,
};
pub use flex_child::Expanded;
pub use flex_list::{FlexList, ListFlex};
pub use layout_transition::{FlexGeometrySnapshot, FlexItemGeometry, FlexSnapshotError};

#[cfg(test)]
mod portable_materializer_tests {
    use aimer_widget::portable::PortableNativeWidget;

    use super::Expanded;

    #[test]
    fn expanded_exposes_a_native_materializer_for_hot_reload_hosts() {
        fn assert_materializer<T: PortableNativeWidget>() {}

        assert_materializer::<Expanded>();
    }
}
pub use raw_flex::Flex;
pub use row_column::{Column, Row};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    Column,
    #[default]
    Inherit,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum BoxAlignment {
    #[default]
    Start,
    Center,
    End,
}

/// Controls how children are placed along a flex container's main axis.
///
/// `Start`, `Center`, and `End` place the group without changing the spacing
/// between children. The remaining variants distribute positive free space
/// between and around the children. When the children use all available space
/// or overflow it, every variant falls back to the closest position possible.
///
/// # Examples
///
/// ```rust
/// use aimer_flex::JustifyContent;
///
/// let start = JustifyContent::Start;
/// let centered = JustifyContent::Center;
/// let end = JustifyContent::End;
/// let between = JustifyContent::SpaceBetween;
/// let around = JustifyContent::SpaceAround;
/// let evenly = JustifyContent::SpaceEvenly;
/// ```
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum JustifyContent {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum OverflowBehavior {
    #[default]
    Hidden,
    Wrap,
    Visible,
}

impl PortableProperty for BoxAlignment {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        aimer_widget::portable::__anteros::PropertyValueKind::I64,
        PortablePropertyConversion::SignedInteger {
            minimum: 0,
            maximum: 2,
        },
    );
}

impl PortableProperty for JustifyContent {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        aimer_widget::portable::__anteros::PropertyValueKind::I64,
        PortablePropertyConversion::SignedInteger {
            minimum: 0,
            maximum: 5,
        },
    );
}

impl PortableProperty for OverflowBehavior {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        aimer_widget::portable::__anteros::PropertyValueKind::I64,
        PortablePropertyConversion::SignedInteger {
            minimum: 0,
            maximum: 2,
        },
    );
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for BoxAlignment {
    #[inline]
    fn encode_property(
        self,
        _context: &mut PortableBuildContext,
    ) -> Result<aimer_widget::portable::__anteros::PropertyValue, PortableBuildError> {
        Ok(aimer_widget::portable::__anteros::PropertyValue::I64(
            match self {
                Self::Start => 0,
                Self::Center => 1,
                Self::End => 2,
            },
        ))
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for JustifyContent {
    #[inline]
    fn encode_property(
        self,
        _context: &mut PortableBuildContext,
    ) -> Result<aimer_widget::portable::__anteros::PropertyValue, PortableBuildError> {
        Ok(aimer_widget::portable::__anteros::PropertyValue::I64(
            match self {
                Self::Start => 0,
                Self::Center => 1,
                Self::End => 2,
                Self::SpaceBetween => 3,
                Self::SpaceAround => 4,
                Self::SpaceEvenly => 5,
            },
        ))
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for OverflowBehavior {
    #[inline]
    fn encode_property(
        self,
        _context: &mut PortableBuildContext,
    ) -> Result<aimer_widget::portable::__anteros::PropertyValue, PortableBuildError> {
        Ok(aimer_widget::portable::__anteros::PropertyValue::I64(
            match self {
                Self::Hidden => 0,
                Self::Wrap => 1,
                Self::Visible => 2,
            },
        ))
    }
}

impl PortableMaterializeProperty for BoxAlignment {
    fn from_awir(
        _document: &aimer_widget::portable::__anteros::WidgetDocumentView<'_>,
        property: aimer_widget::portable::__anteros::PropertyId,
        value: aimer_widget::portable::__anteros::PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        match value {
            aimer_widget::portable::__anteros::PropertyValue::I64(0) => Ok(Self::Start),
            aimer_widget::portable::__anteros::PropertyValue::I64(1) => Ok(Self::Center),
            aimer_widget::portable::__anteros::PropertyValue::I64(2) => Ok(Self::End),
            aimer_widget::portable::__anteros::PropertyValue::I64(_) => {
                Err(PortableMaterializeError::InvalidPropertyValue { property })
            }
            _ => Err(PortableMaterializeError::InvalidPropertyType { property }),
        }
    }
}

impl PortableMaterializeProperty for JustifyContent {
    fn from_awir(
        _document: &aimer_widget::portable::__anteros::WidgetDocumentView<'_>,
        property: aimer_widget::portable::__anteros::PropertyId,
        value: aimer_widget::portable::__anteros::PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        match value {
            aimer_widget::portable::__anteros::PropertyValue::I64(0) => Ok(Self::Start),
            aimer_widget::portable::__anteros::PropertyValue::I64(1) => Ok(Self::Center),
            aimer_widget::portable::__anteros::PropertyValue::I64(2) => Ok(Self::End),
            aimer_widget::portable::__anteros::PropertyValue::I64(3) => Ok(Self::SpaceBetween),
            aimer_widget::portable::__anteros::PropertyValue::I64(4) => Ok(Self::SpaceAround),
            aimer_widget::portable::__anteros::PropertyValue::I64(5) => Ok(Self::SpaceEvenly),
            aimer_widget::portable::__anteros::PropertyValue::I64(_) => {
                Err(PortableMaterializeError::InvalidPropertyValue { property })
            }
            _ => Err(PortableMaterializeError::InvalidPropertyType { property }),
        }
    }
}

impl PortableMaterializeProperty for OverflowBehavior {
    fn from_awir(
        _document: &aimer_widget::portable::__anteros::WidgetDocumentView<'_>,
        property: aimer_widget::portable::__anteros::PropertyId,
        value: aimer_widget::portable::__anteros::PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        match value {
            aimer_widget::portable::__anteros::PropertyValue::I64(0) => Ok(Self::Hidden),
            aimer_widget::portable::__anteros::PropertyValue::I64(1) => Ok(Self::Wrap),
            aimer_widget::portable::__anteros::PropertyValue::I64(2) => Ok(Self::Visible),
            aimer_widget::portable::__anteros::PropertyValue::I64(_) => {
                Err(PortableMaterializeError::InvalidPropertyValue { property })
            }
            _ => Err(PortableMaterializeError::InvalidPropertyType { property }),
        }
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_layout_tests {
    use aimer_widget::base::BuildContext;
    use aimer_widget::portable::__anteros::{
        PropertyValue, Version, WIDGET_SIZED_BOX, WidgetDocumentView,
    };
    use aimer_widget::portable::{
        PortableBuildContext, PortableLimits, PortableWidgetLimits, PortableWidgetSchema,
        SourceFingerprint, StableId128,
    };
    use aimer_widget::{AnyElement, ErrorWidget, PortableWidget, Widget};

    use super::{Expanded, Flex, FlexDirection, ListFlex};

    struct Leaf;

    impl Widget for Leaf {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            ErrorWidget::new("portable leaf").to_element(ctx)
        }
    }

    impl PortableWidget for Leaf {
        fn to_portable_node(
            self,
            ctx: &mut PortableBuildContext,
            source: SourceFingerprint,
        ) -> Result<
            aimer_widget::portable::PortableNodeId,
            aimer_widget::portable::PortableBuildError,
        > {
            ctx.push_node(WIDGET_SIZED_BOX, Version::new(1, 0), None, source, &[], &[])
        }
    }

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(32, 32, 32, 32, 1_024, 8_192),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap()
    }

    fn source() -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_bytes([0x21; 16]))
    }

    fn assert_lowered<W>(widget: W, expected_children: usize, expected_properties: usize)
    where
        W: Widget + PortableWidget + PortableWidgetSchema,
    {
        let mut ctx = context();
        let root = widget.to_portable_node(&mut ctx, source()).unwrap();
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        assert_eq!(node.widget_type(), W::SCHEMA.widget().id());
        assert_eq!(node.properties().count(), expected_properties);
        assert_eq!(node.children().count(), expected_children);
    }

    #[test]
    fn expanded_lowers_flex_property_and_required_child() {
        let mut ctx = context();
        let root = Expanded::new()
            .flex(2.5)
            .child(Leaf)
            .to_portable_node(&mut ctx, source())
            .unwrap();
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        assert_eq!(
            node.widget_type(),
            <Expanded<Leaf> as PortableWidgetSchema>::SCHEMA
                .widget()
                .id()
        );
        assert_eq!(
            node.properties().next().unwrap().value(),
            PropertyValue::F64(2.5)
        );
        assert_eq!(node.children().count(), 1);
    }

    #[test]
    fn flex_lowers_collection_children_without_native_construction() {
        assert_lowered(
            Flex::new().direction(FlexDirection::Row).children([Leaf]),
            1,
            0,
        );
    }

    #[test]
    fn list_flex_lowers_data_builder_children() {
        let list: ListFlex<u8, _> = super::Column::new().list([1_u8, 2]).builder(|_| Leaf);
        assert_lowered(list, 2, 0);
    }
}
