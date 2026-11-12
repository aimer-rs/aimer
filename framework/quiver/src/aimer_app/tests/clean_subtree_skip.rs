//! A frame requested by scrolling alone leaves a subtree alone when walking it
//! again could change nothing, and draws exactly what a full walk would have.
//!
//! The left side of the window is a static column of leaves; the right side is
//! a list that scrolls. Every leaf counts the `update`s it receives, so the
//! tests can tell a leaf that was walked from one that was left alone.

use super::*;

use aimer_flex::Row;

const WIDTH: u32 = 400;
const HEIGHT: u32 = 300;
const LEFT: f32 = 100.0;
const LEAF_HEIGHT: f32 = 30.0;
const LEAVES: usize = 8;
const ROWS: usize = 10;
const TALL: usize = 40;
const ROW_HEIGHT: f32 = 50.0;

/// A leaf that draws a fill and counts its updates. `reusable` is whether its
/// update depends on nothing but the context it is given.
struct Leaf {
    updates: Rc<Cell<usize>>,
    shade: u8,
    reusable: bool,
    /// Promises a repeatable walk but publishes a different rectangle on every
    /// update, which is what verify mode has to catch.
    lies: bool,
    width: f32,
    height: f32,
}

impl Widget for Leaf {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        LeafElement {
            updates: self.updates,
            shade: self.shade,
            reusable: self.reusable,
            lies: self.lies,
            width: self.width,
            height: self.height,
        }
        .boxed()
    }
}

struct LeafElement {
    updates: Rc<Cell<usize>>,
    shade: u8,
    reusable: bool,
    lies: bool,
    width: f32,
    height: f32,
}

impl VisitorElement for LeafElement {
    fn debug_name(&self) -> &'static str {
        "CleanSkipLeaf"
    }
}

impl EventElement for LeafElement {}
impl Rebuildable for LeafElement {}

impl LayoutElement for LeafElement {
    fn size(&self) -> Option<Size> {
        Some(Size::new(Dimension::Px(self.width), Dimension::Px(self.height)))
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.lies.then(|| {
            let moved = self.updates.get() as f32;
            (Vec2d { x: moved, y: 0.0 }, Vec2d { x: moved + 1.0, y: 1.0 })
        })
    }
}

impl Drawable for LeafElement {
    fn update(&self, _ctx: &BuildContext) {
        self.updates.set(self.updates.get() + 1);
    }

    fn reusable_walk(&self) -> bool {
        self.reusable
    }

    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint(&self, ctx: &BuildContext) {
        let canvas = Canvas::of(ctx);
        canvas.fill_rect(
            Rect::new(0.0, 0.0, self.width, self.height),
            [self.shade, 255 - self.shade, 128, 255],
        );
        canvas.finish();
    }
}

struct Counters {
    /// The static column whose leaves may be left alone.
    pure: Vec<Rc<Cell<usize>>>,
    /// A second static column whose leaves may not (their update is not repeatable).
    impure: Vec<Rc<Cell<usize>>>,
    /// The rows of the list that scrolls.
    rows: Vec<Rc<Cell<usize>>>,
    /// A column of repeatable leaves inside the list that is taller than the
    /// window: rows are walked only while they are in view, so a scroll brings new
    /// ones in and they must be walked as they arrive.
    moving: Vec<Rc<Cell<usize>>>,
}

impl Counters {
    fn new() -> Self {
        let counters = |count| (0..count).map(|_| Rc::new(Cell::new(0))).collect();
        Self {
            pure: counters(LEAVES),
            impure: counters(LEAVES),
            rows: counters(ROWS),
            moving: counters(TALL),
        }
    }
}

fn counts(cells: &[Rc<Cell<usize>>]) -> Vec<usize> {
    cells.iter().map(|cell| cell.get()).collect()
}

struct Window {
    controller: ScrollController,
    counters: Rc<Counters>,
    /// Makes one repeatable leaf of the static column break its promise.
    lie: bool,
}

impl Widget for Window {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let lie = self.lie;
        let column = |cells: &[Rc<Cell<usize>>], reusable: bool| {
            Column::new()
                .children(
                    cells
                        .iter()
                        .enumerate()
                        .map(|(index, updates)| {
                            Leaf {
                                updates: updates.clone(),
                                shade: 30 + ((index * 20) % 200) as u8,
                                reusable,
                                lies: lie && reusable && index == 3,
                                width: LEFT / 2.0,
                                height: LEAF_HEIGHT,
                            }
                            .boxed()
                        })
                        .collect::<Vec<_>>(),
                )
                .boxed()
        };
        let mut rows = self
            .counters
            .rows
            .iter()
            .enumerate()
            .map(|(index, updates)| {
                Leaf {
                    updates: updates.clone(),
                    shade: 20 + ((index * 7) % 200) as u8,
                    reusable: false,
                    lies: false,
                    width: WIDTH as f32 - LEFT,
                    height: ROW_HEIGHT,
                }
                .boxed()
            })
            .collect::<Vec<_>>();
        // The tall repeatable list comes after the rows, below the first screen.
        rows.push(column(&self.counters.moving, true));
        Row::new()
            .children(vec![
                SizedBox::new()
                    .width(Dimension::Px(LEFT / 2.0))
                    .height(Dimension::Px(HEIGHT as f32))
                    .box_child(column(&self.counters.pure, true)),
                SizedBox::new()
                    .width(Dimension::Px(LEFT / 2.0))
                    .height(Dimension::Px(HEIGHT as f32))
                    .box_child(column(&self.counters.impure, false)),
                SizedBox::new()
                    .width(Dimension::Px(WIDTH as f32 - LEFT))
                    .height(Dimension::Px(HEIGHT as f32))
                    .box_child(
                        Scrollable::new()
                            .controller(self.controller)
                            .vertical_scroll_bar(None)
                            .horizontal_scroll_bar(None)
                            .child(Column::new().children(rows)),
                    ),
            ])
            .to_element(ctx)
    }
}

struct Scene {
    app: HeadlessAimerApp<aimer_modal::ModalHost<Window>>,
    controller: ScrollController,
    counters: Rc<Counters>,
}

impl Scene {
    /// A settled window with the cursor over the list. `skip` selects whether
    /// a scroll-only frame may leave a static subtree alone.
    fn new(skip: bool) -> Self {
        Self::build(skip, false)
    }

    /// A scene in which one leaf of the static column breaks its promise.
    fn lying() -> Self {
        Self::build(true, true)
    }

    fn build(skip: bool, lie: bool) -> Self {
        aimer_widget::set_clean_subtree_skip(skip);
        aimer_widget::set_clean_skip_blocker_probe(true);
        aimer_widget::reset_clean_subtree_skip_stats();
        let controller = ScrollController::new();
        let counters = Rc::new(Counters::new());
        let mut app = AimerApp::start_headless_with(
            Window {
                controller: controller.clone(),
                counters: counters.clone(),
                lie,
            },
            HeadlessOptions {
                size: PhysicalSize::new(WIDTH, HEIGHT),
                scale_factor: 1.0,
            },
        );
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(300.0, 150.0),
        });
        for _ in 0..6 {
            app.render_frame();
        }
        Self {
            app,
            controller,
            counters,
        }
    }

    /// Queues one wheel notch and draws the frames that deliver it, calling
    /// `after_frame` with the counts taken before and after each one.
    fn scroll(&mut self, delta: f64, mut after_frame: impl FnMut(&Scene, &Counts, &Counts)) {
        self.app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, delta)),
            phase: TouchPhase::Moved,
        });
        let mut frames = 0;
        while self.app.app.scroll_smoother.is_active() || frames == 0 {
            let before = self.counts();
            self.app.render_frame();
            let after = self.counts();
            after_frame(self, &before, &after);
            frames += 1;
            assert!(frames < 120, "the scroll never settled");
        }
    }

    fn counts(&self) -> Counts {
        Counts {
            pure: counts(&self.counters.pure),
            impure: counts(&self.counters.impure),
            rows: counts(&self.counters.rows),
            moving: counts(&self.counters.moving),
        }
    }

    /// Everything the frame would draw, as comparable text.
    fn plan(&self) -> Vec<String> {
        self.app
            .app
            .render_tree()
            .render_all()
            .into_iter()
            .map(|operation| match operation {
                aimer_cupid::draw_cmd::RenderOp::Draw(item) => format!(
                    "{:?} {:?} {:?} {:?} {:?}",
                    item.element,
                    item.bounds,
                    item.origin,
                    item.clip,
                    item.snapshot().commands
                ),
                aimer_cupid::draw_cmd::RenderOp::BeginOpacityGroup {
                    element,
                    bounds,
                    opacity,
                    clip,
                    masked,
                    ..
                } => format!("begin {element:?} {bounds:?} {opacity} {clip:?} {masked}"),
                aimer_cupid::draw_cmd::RenderOp::EndOpacityGroup { element } => {
                    format!("end {element:?}")
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Counts {
    pure: Vec<usize>,
    impure: Vec<usize>,
    rows: Vec<usize>,
    moving: Vec<usize>,
}

#[test]
fn a_scroll_only_frame_leaves_a_static_subtree_alone() {
    let mut scene = Scene::new(true);
    let mut scrolled_frames = 0;
    let mut static_walks = 0;

    scene.scroll(-60.0, |_, before, after| {
        if before.rows == after.rows {
            return; // the frame did not reach the list: nothing scrolled
        }
        scrolled_frames += 1;
        static_walks += after.pure.iter().sum::<usize>() - before.pure.iter().sum::<usize>();
    });

    assert!(scrolled_frames > 0, "the list never scrolled");
    assert_eq!(
        static_walks, 0,
        "a static subtree was walked on {scrolled_frames} scroll-only frames; stats {:?}; blockers {:?}",
        aimer_widget::clean_subtree_skip_stats(),
        aimer_widget::clean_skip_blockers()
    );
    assert!(
        aimer_widget::clean_subtree_skip_stats().skips > 0,
        "nothing was skipped"
    );
}

#[test]
fn with_the_switch_off_a_static_subtree_is_walked_on_every_frame() {
    let mut scene = Scene::new(false);
    let mut scrolled_frames = 0;
    let mut static_walks = 0;

    scene.scroll(-60.0, |_, before, after| {
        if before.rows == after.rows {
            return;
        }
        scrolled_frames += 1;
        static_walks += after.pure.iter().sum::<usize>() - before.pure.iter().sum::<usize>();
    });

    assert!(scrolled_frames > 0, "the list never scrolled");
    assert_eq!(
        static_walks,
        scrolled_frames * LEAVES,
        "every leaf is walked once per frame when nothing is skipped"
    );
    assert_eq!(aimer_widget::clean_subtree_skip_stats().skips, 0);
}

#[test]
fn an_element_whose_update_is_not_repeatable_is_still_walked() {
    let mut scene = Scene::new(true);
    let mut scrolled_frames = 0;
    let mut impure_walks = 0;
    let mut pure_walks = 0;

    scene.scroll(-60.0, |_, before, after| {
        if before.rows == after.rows {
            return;
        }
        scrolled_frames += 1;
        impure_walks += after.impure.iter().sum::<usize>() - before.impure.iter().sum::<usize>();
        pure_walks += after.pure.iter().sum::<usize>() - before.pure.iter().sum::<usize>();
    });

    assert!(scrolled_frames > 0, "the list never scrolled");
    assert_eq!(
        impure_walks,
        scrolled_frames * LEAVES,
        "a leaf that did not opt in must be walked on every frame"
    );
    assert_eq!(pure_walks, 0, "its pure sibling is still left alone");
}

#[test]
fn a_resize_walks_the_subtree_once_and_the_next_scroll_leaves_it_alone_again() {
    let mut scene = Scene::new(true);
    scene.scroll(-40.0, |_, _, _| {});

    let before_resize = scene.counts();
    scene.app.send_window_event(WindowEvent::Resized(PhysicalSize::new(WIDTH + 40, HEIGHT)));
    scene.app.render_frame();
    let after_resize = scene.counts();
    assert!(
        after_resize.pure.iter().sum::<usize>() > before_resize.pure.iter().sum::<usize>(),
        "the resize changed the window's size but the static subtree was not walked"
    );

    // The resize frame recorded what it walked, so scrolling skips again.
    let mut static_walks = 0;
    let mut scrolled_frames = 0;
    scene.scroll(-40.0, |_, before, after| {
        if before.rows == after.rows {
            return;
        }
        scrolled_frames += 1;
        static_walks += after.pure.iter().sum::<usize>() - before.pure.iter().sum::<usize>();
    });
    assert!(scrolled_frames > 0, "the list never scrolled");
    assert_eq!(static_walks, 0);
}

/// The frames a scene shows with the skip on are the frames it shows with it off.
fn run(skip: bool) -> Vec<Vec<String>> {
    let mut scene = Scene::new(skip);
    let mut plans = vec![scene.plan()];
    for delta in [-50.0, -30.0, 70.0] {
        scene.scroll(delta, |_, _, _| {});
        plans.push(scene.plan());
    }
    // The pointer visits the static side and leaves it, then the list scrolls.
    for (x, y) in [(20.0, 40.0), (20.0, 200.0), (300.0, 150.0)] {
        scene.app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        });
        scene.app.render_frame();
        scene.app.render_frame();
        plans.push(scene.plan());
    }
    scene.scroll(-60.0, |_, _, _| {});
    plans.push(scene.plan());
    scene
        .app
        .send_window_event(WindowEvent::Resized(PhysicalSize::new(WIDTH + 40, HEIGHT)));
    scene.app.render_frame();
    plans.push(scene.plan());
    scene.scroll(-80.0, |_, _, _| {});
    plans.push(scene.plan());
    plans
}

#[test]
fn the_frames_with_the_skip_match_the_frames_without() {
    let without = run(false);
    let with = run(true);
    assert_eq!(without.len(), with.len());
    for (step, (expected, actual)) in without.iter().zip(&with).enumerate() {
        assert_eq!(expected, actual, "the frame after step {step} differs");
    }
    assert!(
        aimer_widget::clean_subtree_skip_stats().skips > 0,
        "the run with the skip on never skipped anything, so it proves nothing"
    );
}

/// The scroll smoother runs on the clock, so two runs differ by a hundredth of a
/// pixel where it settles. Rounds every number with a fraction to a whole pixel;
/// the scroll distances in the tests settle far from a rounding boundary.
fn quantize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut number = String::new();
    let flush = |number: &mut String, out: &mut String| {
        if number.contains('.') && let Ok(value) = number.parse::<f64>() {
            out.push_str(&format!("{value:.0}"));
        } else {
            out.push_str(number);
        }
        number.clear();
    };
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == '.' || (ch == '-' && number.is_empty()) {
            number.push(ch);
        } else {
            flush(&mut number, &mut out);
            out.push(ch);
        }
    }
    flush(&mut number, &mut out);
    out
}

/// A tall list of repeatable leaves inside the scrolling list is walked only for
/// the rows in view. A scroll brings new rows into view, and leaving the list
/// alone in a frame that did so would leave them blank.
#[test]
fn rows_that_scroll_into_view_inside_a_repeatable_list_are_drawn() {
    let run = |skip: bool| {
        let mut scene = Scene::new(skip);
        let plan = |scene: &Scene| scene.plan().iter().map(|line| quantize(line)).collect::<Vec<_>>();
        let mut plans = vec![plan(&scene)];
        let mut offsets = Vec::new();
        // Stops short of the end of the list: its extent is an estimate that painting
        // refines, so where a scroll clamps depends on the clock.
        for delta in [-200.0, -250.0, -300.0, 150.0, -100.0, -140.0] {
            scene.scroll(delta, |_, _, _| {});
            plans.push(plan(&scene));
            offsets.push(scene.controller.offset().y);
        }
        (plans, offsets, scene.counts())
    };

    let (without, offsets_without, walked_without) = run(false);
    let (with, offsets, walked) = run(true);

    assert!(
        offsets_without.iter().zip(&offsets).all(|(left, right)| (left - right).abs() < 0.5),
        "the two runs did not scroll alike: {offsets_without:?} {offsets:?}"
    );
    assert!(
        offsets.windows(2).any(|pair| pair[0] != pair[1]),
        "the list never scrolled"
    );
    assert!(
        walked.moving.iter().filter(|count| **count > 0).count() > LEAVES,
        "no new rows of the tall list came into view, so the scene proves nothing"
    );
    for (step, (expected, actual)) in without.iter().zip(&with).enumerate() {
        assert_eq!(expected, actual, "the frame after step {step} differs");
    }
    // The same rows were walked at least once either way.
    assert_eq!(
        walked_without.moving.iter().map(|count| *count > 0).collect::<Vec<_>>(),
        walked.moving.iter().map(|count| *count > 0).collect::<Vec<_>>(),
        "a different set of rows of the tall list was ever walked"
    );
}

/// A list of hover regions that slides under a pointer that has not moved. Each
/// region compares the pointer with where it is, so what its walk does depends
/// on where the content is in the window, not only on the context it is given.
struct HoverList {
    controller: ScrollController,
    hovered: Rc<Cell<i32>>,
    transitions: Rc<Cell<usize>>,
    /// How many boxes each region wraps its content in: a region is only worth
    /// leaving alone once its subtree has a few elements.
    depth: usize,
}

/// A 200 by 60 box in `depth` more of the same.
fn nested_box(depth: usize) -> aimer_widget::AnyWidget {
    let mut widget = SizedBox::new().width(Dimension::Px(200.0)).height(Dimension::Px(60.0)).boxed();
    for _ in 0..depth {
        widget = SizedBox::new()
            .width(Dimension::Px(200.0))
            .height(Dimension::Px(60.0))
            .child(widget)
            .boxed();
    }
    widget
}

impl Widget for HoverList {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        use aimer_input::mouse_region::MouseRegion;
        let rows = (0..16)
            .map(|_| {
                let (enter, exit) = (
                    (self.hovered.clone(), self.transitions.clone()),
                    (self.hovered.clone(), self.transitions.clone()),
                );
                MouseRegion::new()
                    .on_hover_enter(move || {
                        enter.0.set(enter.0.get() + 1);
                        enter.1.set(enter.1.get() + 1);
                    })
                    .on_hover_exit(move || {
                        exit.0.set(exit.0.get() - 1);
                        exit.1.set(exit.1.get() + 1);
                    })
                    .child(nested_box(self.depth))
                    .boxed()
            })
            .collect::<Vec<_>>();
        SizedBox::new()
            .width(Dimension::Px(200.0))
            .height(Dimension::Px(300.0))
            .child(
                Scrollable::new()
                    .controller(self.controller)
                    .vertical_scroll_bar(None)
                    .horizontal_scroll_bar(None)
                    .child(Column::new().children(rows)),
            )
            .to_element(ctx)
    }
}

/// Slides a list of hover regions under a pointer that does not move, with the skip
/// on or off, and returns what hover did frame by frame.
fn slide_under_the_pointer(skip: bool, depth: usize) -> (Vec<(i32, usize)>, aimer_widget::CleanSkipStats) {
    aimer_widget::set_clean_subtree_skip(skip);
    aimer_widget::reset_clean_subtree_skip_stats();
    let (hovered, transitions) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let controller = ScrollController::new();
    let mut app = AimerApp::start_headless_with(
        HoverList {
            controller: controller.clone(),
            hovered: hovered.clone(),
            transitions: transitions.clone(),
            depth,
        },
        HeadlessOptions {
            size: PhysicalSize::new(200, 300),
            scale_factor: 1.0,
        },
    );
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(100.0, 150.0),
    });
    for _ in 0..6 {
        app.render_frame();
    }
    let mut trace = vec![(hovered.get(), transitions.get())];
    for delta in [-45.0, -60.0, -75.0, 90.0, -130.0, -45.0, -60.0, 75.0, -90.0] {
        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, delta)),
            phase: TouchPhase::Moved,
        });
        let mut frames = 0;
        while app.app.scroll_smoother.is_active() || frames == 0 {
            app.render_frame();
            trace.push((hovered.get(), transitions.get()));
            frames += 1;
            assert!(frames < 120, "the scroll never settled");
        }
    }
    (trace, aimer_widget::clean_subtree_skip_stats())
}

/// Content that slides under a pointer that did not move changes which region it
/// is over, and hover follows exactly as it does without the skip.
#[test]
fn hover_follows_content_that_slides_under_a_pointer_that_did_not_move() {
    let (without, _) = slide_under_the_pointer(false, 0);
    let (with, stats) = slide_under_the_pointer(true, 0);

    assert!(
        without.last().expect("a trace").1 > without[0].1 + 2,
        "the content never slid under the pointer, so the scene proves nothing: {without:?}"
    );
    assert_eq!(without, with, "hover differs when subtrees are left alone; stats {stats:?}");
    // The enter of the region that arrives under the pointer is delivered before
    // the exit of the one that leaves, so two are hovered for a moment, never more.
    assert!(with.iter().all(|(hovered, _)| (0..=2).contains(hovered)), "{with:?}");
}

/// The same with regions big enough to be left alone: the skip applies to them
/// while they move, and a region that moves under the pointer is still walked.
#[test]
fn hover_follows_regions_that_are_left_alone_while_they_slide_under_the_pointer() {
    let (without, _) = slide_under_the_pointer(false, 10);
    let (with, stats) = slide_under_the_pointer(true, 10);

    assert!(
        without.last().expect("a trace").1 > without[0].1 + 2,
        "the content never slid under the pointer, so the scene proves nothing: {without:?}"
    );
    assert!(stats.skips > 0, "no region was left alone, so the scene proves nothing: {stats:?}");
    assert_eq!(without, with, "hover differs when subtrees are left alone; stats {stats:?}");
}

/// A leaf that counts its updates and, if it `reads_place`, asks the canvas where
/// it is drawn in every one of them.
struct PlaceLeaf {
    updates: Rc<Cell<usize>>,
    reads_place: bool,
}

impl Widget for PlaceLeaf {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        PlaceLeafElement { updates: self.updates, reads_place: self.reads_place }.boxed()
    }
}

struct PlaceLeafElement {
    updates: Rc<Cell<usize>>,
    reads_place: bool,
}

impl VisitorElement for PlaceLeafElement {
    fn debug_name(&self) -> &'static str {
        "CleanSkipPlaceLeaf"
    }
}

impl EventElement for PlaceLeafElement {}
impl Rebuildable for PlaceLeafElement {}

impl LayoutElement for PlaceLeafElement {
    fn size(&self) -> Option<Size> {
        Some(Size::new(Dimension::Px(200.0), Dimension::Px(60.0)))
    }
}

impl Drawable for PlaceLeafElement {
    fn update(&self, ctx: &BuildContext) {
        self.updates.set(self.updates.get() + 1);
        if self.reads_place {
            let _ = ctx.canvas.get_transform_translation();
        }
    }

    fn reusable_walk(&self) -> bool {
        true
    }

    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint(&self, ctx: &BuildContext) {
        Canvas::of(ctx).finish();
    }
}

/// A list of rows that scroll, each a few boxes deep around a leaf.
struct PlaceList {
    updates: Rc<Cell<usize>>,
    reads_place: bool,
}

impl Widget for PlaceList {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let rows = (0..30)
            .map(|_| {
                let mut row = PlaceLeaf {
                    updates: self.updates.clone(),
                    reads_place: self.reads_place,
                }
                .boxed();
                for _ in 0..10 {
                    row = SizedBox::new()
                        .width(Dimension::Px(200.0))
                        .height(Dimension::Px(60.0))
                        .child(row)
                        .boxed();
                }
                row
            })
            .collect::<Vec<_>>();
        SizedBox::new()
            .width(Dimension::Px(200.0))
            .height(Dimension::Px(300.0))
            .child(
                Scrollable::new()
                    .vertical_scroll_bar(None)
                    .horizontal_scroll_bar(None)
                    .child(Column::new().children(rows)),
            )
            .to_element(ctx)
    }
}

/// How many times the leaves were updated by a script of scrolling.
fn updates_while_scrolling(skip: bool, reads_place: bool) -> (usize, aimer_widget::CleanSkipStats) {
    aimer_widget::set_clean_subtree_skip(skip);
    aimer_widget::reset_clean_subtree_skip_stats();
    let updates = Rc::new(Cell::new(0));
    let mut app = AimerApp::start_headless_with(
        PlaceList { updates: updates.clone(), reads_place },
        HeadlessOptions { size: PhysicalSize::new(200, 300), scale_factor: 1.0 },
    );
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(100.0, 150.0),
    });
    for _ in 0..6 {
        app.render_frame();
    }
    let before = updates.get();
    for delta in [-45.0, -60.0, -75.0, 90.0, -130.0, -45.0] {
        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, delta)),
            phase: TouchPhase::Moved,
        });
        let mut frames = 0;
        while app.app.scroll_smoother.is_active() || frames == 0 {
            app.render_frame();
            frames += 1;
            assert!(frames < 120, "the scroll never settled");
        }
    }
    (updates.get() - before, aimer_widget::clean_subtree_skip_stats())
}

/// Rows that scroll are the same rows in another place. Those that never asked
/// where they are drawn are left alone while they move; those that did are walked,
/// since their walk may depend on the answer.
#[test]
fn rows_that_never_ask_where_they_are_drawn_are_left_alone_while_they_move() {
    let (always, _) = updates_while_scrolling(false, false);
    let (blind, blind_stats) = updates_while_scrolling(true, false);
    let (reading, reading_stats) = updates_while_scrolling(true, true);

    assert!(always > 0);
    assert!(blind_stats.skips > 0, "nothing was left alone: {blind_stats:?}");
    assert!(
        blind < always,
        "rows that only moved were walked as often as without the skip: {blind} against {always}"
    );
    // A row that read its place is still left alone while it holds still, between
    // one wheel step and the next, so it is walked less than without the skip; but
    // not while it moves, so it is walked more than a row that never asked.
    assert!(
        blind < reading,
        "rows that read where they are drawn were left alone as often as rows that did not: \
         {reading} against {blind} (stats {reading_stats:?})"
    );
    assert!(reading <= always, "{reading} updates with the skip against {always} without");
}

/// A subtree that never asks for the pointer does not depend on it, so a pointer
/// that moves over (or away from) the window leaves it alone.
#[test]
fn a_subtree_that_never_reads_the_pointer_is_left_alone_while_it_moves() {
    let mut scene = Scene::new(true);
    let before = scene.counts();

    for (x, y) in [(20.0, 40.0), (20.0, 200.0), (60.0, 120.0), (300.0, 150.0), (350.0, 250.0)] {
        scene.app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        });
        scene.app.render_frame();
        scene.app.render_frame();
    }
    let after = scene.counts();

    assert_eq!(
        after.pure, before.pure,
        "a subtree that does not read the pointer was walked as the pointer moved"
    );
    assert!(
        after.impure.iter().sum::<usize>() > before.impure.iter().sum::<usize>(),
        "the leaves that did not opt in must still be walked"
    );
    assert!(aimer_widget::clean_subtree_skip_stats().skips > 0);
}

/// Verify mode walks a subtree it would have skipped and compares what the walk
/// leaves in the retained tree with what was there before. Elements that keep
/// their promise leave it unchanged, in every frame of a scroll, a hover and a
/// resize.
#[test]
fn verify_mode_finds_nothing_wrong_with_elements_that_keep_their_promise() {
    let mut scene = Scene::new(true);
    aimer_widget::set_clean_skip_verify(true);
    scene.scroll(-60.0, |_, _, _| {});
    for (x, y) in [(20.0, 40.0), (20.0, 200.0), (300.0, 150.0)] {
        scene.app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        });
        scene.app.render_frame();
        scene.app.render_frame();
    }
    scene.app.send_window_event(WindowEvent::Resized(PhysicalSize::new(WIDTH + 40, HEIGHT)));
    scene.app.render_frame();
    scene.scroll(-40.0, |_, _, _| {});
    aimer_widget::set_clean_skip_verify(false);

    let stats = aimer_widget::clean_subtree_skip_stats();
    assert!(stats.verified > 0, "nothing was verified, so the run proves nothing: {stats:?}");
    assert_eq!(aimer_widget::clean_skip_violations(), Vec::new());
}

/// An element that promises a repeatable walk and then changes what it publishes
/// on every walk is named, with what changed.
#[test]
fn verify_mode_names_an_element_that_breaks_its_promise() {
    let mut scene = Scene::lying();
    aimer_widget::set_clean_skip_verify(true);
    scene.scroll(-60.0, |_, _, _| {});
    aimer_widget::set_clean_skip_verify(false);

    let violations = aimer_widget::clean_skip_violations();
    assert!(!violations.is_empty(), "a walk that changed what it published went unnoticed");
    assert!(
        violations.iter().any(|violation| violation.detail.contains("CleanSkipLeaf")),
        "the violation does not name the element: {violations:?}"
    );
}

/// Once a steady subtree has been asked what the retained tree shows of it, the
/// tree is not asked again for as long as its geometry holds still.
#[test]
fn a_steady_subtree_is_not_asked_what_the_tree_shows_while_the_tree_holds_still() {
    let mut scene = Scene::new(true);
    for _ in 0..8 {
        scene.app.render_frame();
    }
    let asked = aimer_widget::clean_subtree_skip_stats().tree_queries;
    let skips = aimer_widget::clean_subtree_skip_stats().skips;

    for _ in 0..16 {
        scene.app.render_frame();
    }
    let stats = aimer_widget::clean_subtree_skip_stats();

    assert!(stats.skips > skips, "nothing was skipped, so the run proves nothing");
    assert_eq!(
        stats.tree_queries, asked,
        "the tree was asked again although nothing in it moved"
    );
}
