//! A frame requested by scrolling alone leaves prepared rows that sit outside
//! the viewport alone, and draws exactly what a full revisit would have drawn.
//!
//! The rows count every `update` they receive, so the tests can tell a row
//! that was revisited from one that was left alone.

use super::*;

const ROWS: usize = 40;
const ROW_HEIGHT: f32 = 50.0;
const VIEWPORT_WIDTH: u32 = 100;
const VIEWPORT_HEIGHT: u32 = 300;

struct CountingRow {
    updates: Rc<Cell<usize>>,
    index: usize,
}

impl Widget for CountingRow {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        CountingRowElement {
            updates: self.updates,
            index: self.index,
        }
        .boxed()
    }
}

impl aimer_widget::PortableWidget for CountingRow {}

struct CountingRowElement {
    updates: Rc<Cell<usize>>,
    index: usize,
}

impl VisitorElement for CountingRowElement {
    fn debug_name(&self) -> &'static str {
        "CountingRow"
    }
}

impl EventElement for CountingRowElement {}
impl Rebuildable for CountingRowElement {}

impl LayoutElement for CountingRowElement {
    fn size(&self) -> Option<Size> {
        Some(Size::new(
            Dimension::Px(VIEWPORT_WIDTH as f32),
            Dimension::Px(ROW_HEIGHT),
        ))
    }
}

impl Drawable for CountingRowElement {
    fn update(&self, _ctx: &BuildContext) {
        self.updates.set(self.updates.get() + 1);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let shade = 40 + (self.index % 7) as u8 * 30;
        let canvas = Canvas::of(ctx);
        canvas.fill_rect(
            Rect::new(0.0, 0.0, VIEWPORT_WIDTH as f32, ROW_HEIGHT),
            [shade, 255 - shade, 128, 255],
        );
        canvas.finish();
    }
}

struct CountingList {
    controller: ScrollController,
    updates: Vec<Rc<Cell<usize>>>,
}

impl Widget for CountingList {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let rows = self
            .updates
            .iter()
            .enumerate()
            .map(|(index, updates)| {
                CountingRow {
                    updates: updates.clone(),
                    index,
                }
                .boxed()
            })
            .collect::<Vec<_>>();
        SizedBox::new()
            .width(Dimension::Px(VIEWPORT_WIDTH as f32))
            .height(Dimension::Px(VIEWPORT_HEIGHT as f32))
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

impl aimer_widget::PortableWidget for CountingList {}

struct Scene {
    app: HeadlessAimerApp<aimer_modal::ModalHost<CountingList>>,
    controller: ScrollController,
    updates: Vec<Rc<Cell<usize>>>,
}

impl Scene {
    /// A settled list with the cursor over it. `skip` selects whether prepared
    /// off-screen rows may be left alone on scroll-only frames.
    fn new(skip: bool) -> Self {
        aimer_widget::set_settled_offscreen_skip(skip);
        let controller = ScrollController::new();
        let updates = (0..ROWS).map(|_| Rc::new(Cell::new(0))).collect::<Vec<_>>();
        let mut app = AimerApp::start_headless_with(
            CountingList {
                controller: controller.clone(),
                updates: updates.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
                scale_factor: 1.0,
            },
        );
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(50.0, 150.0),
        });
        for _ in 0..6 {
            app.render_frame();
        }
        Self {
            app,
            controller,
            updates,
        }
    }

    fn counts(&self) -> Vec<usize> {
        self.updates.iter().map(|count| count.get()).collect()
    }

    fn offset(&self) -> f32 {
        self.controller.offset().y
    }

    /// Queues one wheel notch of `delta` pixels and draws the frames that
    /// deliver it, calling `after_frame` with the counts taken before and
    /// after each one.
    fn scroll(&mut self, delta: f64, mut after_frame: impl FnMut(&Scene, &[usize], &[usize])) {
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

    /// Whether row `index` overlaps the viewport widened by `slack` pixels.
    fn near_viewport(&self, index: usize, slack: f32) -> bool {
        let top = index as f32 * ROW_HEIGHT;
        let bottom = top + ROW_HEIGHT;
        let offset = self.offset();
        bottom > offset - slack && top < offset + VIEWPORT_HEIGHT as f32 + slack
    }

    /// Everything the frame would draw, as comparable text.
    fn plan(&self) -> Vec<String> {
        self.app
            .app
            .render_tree()
            .render_all()
            .into_iter()
            .map(|operation| match operation {
                aimer_cupid::draw_cmd_v2::RenderOp::Draw(item) => format!(
                    "{:?} {:?} {:?} {:?} {:?}",
                    item.element,
                    item.bounds,
                    item.origin,
                    item.clip,
                    item.snapshot().commands
                ),
                aimer_cupid::draw_cmd_v2::RenderOp::BeginOpacityGroup {
                    element,
                    bounds,
                    opacity,
                    clip,
                    masked,
                    ..
                } => format!("begin {element:?} {bounds:?} {opacity} {clip:?} {masked}"),
                aimer_cupid::draw_cmd_v2::RenderOp::EndOpacityGroup { element } => {
                    format!("end {element:?}")
                }
            })
            .collect()
    }
}

#[test]
fn a_scroll_only_frame_leaves_settled_rows_outside_the_viewport_alone() {
    let mut scene = Scene::new(true);
    // The window prepared around the viewport reaches past it, so some rows
    // outside the viewport have been visited and are now settled.
    let prepared = scene.counts();
    let margin_rows = (0..ROWS)
        .filter(|&index| prepared[index] > 0 && !scene.near_viewport(index, 0.0))
        .count();
    assert!(
        margin_rows > 0,
        "the scene prepared no rows outside the viewport"
    );

    let mut checked_frames = 0;
    let mut revisited_margin = 0;
    let mut revisited_visible = 0;
    scene.scroll(-60.0, |scene, before, after| {
        if before == after {
            return;
        }
        checked_frames += 1;
        for index in 0..ROWS {
            let revisited = after[index] > before[index];
            if scene.near_viewport(index, 0.0) {
                revisited_visible += usize::from(revisited);
            } else if before[index] > 0 && !scene.near_viewport(index, 2.0 * ROW_HEIGHT) {
                // Prepared earlier, off screen now and a full row clear of the
                // edge, so the offset cannot have brought it into view.
                revisited_margin += usize::from(revisited);
            }
        }
    });

    assert!(
        checked_frames > 1,
        "the scroll drew {checked_frames} frames"
    );
    assert!(
        revisited_visible > 0,
        "visible rows must keep being updated"
    );
    assert_eq!(
        revisited_margin, 0,
        "prepared rows outside the viewport were visited on a scroll-only frame"
    );
    aimer_widget::set_settled_offscreen_skip(true);
}

#[test]
fn without_the_skip_the_same_scroll_visits_the_prepared_rows_every_frame() {
    let mut scene = Scene::new(false);
    let mut revisited_margin = 0;
    scene.scroll(-60.0, |scene, before, after| {
        for index in 0..ROWS {
            if before[index] > 0
                && !scene.near_viewport(index, 2.0 * ROW_HEIGHT)
                && after[index] > before[index]
            {
                revisited_margin += 1;
            }
        }
    });

    // The control for the test above: it only means something if rows really
    // were being revisited before.
    assert!(revisited_margin > 0);
    aimer_widget::set_settled_offscreen_skip(true);
}

#[test]
fn a_full_frame_visits_the_whole_prepared_window_again() {
    let mut scene = Scene::new(true);
    scene.scroll(-60.0, |_, _, _| {});
    let before = scene.counts();

    scene.app.app.request_full_redraw();
    scene.app.render_frame();
    let after = scene.counts();

    // The margin ahead of a scroll shrinks back once the viewport rests, so
    // only the base margin around the viewport is certain to be revisited.
    let window = (0..ROWS)
        .filter(|&index| scene.near_viewport(index, 100.0))
        .collect::<Vec<_>>();
    assert!(window.len() > 6);
    for index in window {
        assert!(
            after[index] > before[index],
            "row {index} was not visited by the full frame"
        );
    }
    aimer_widget::set_settled_offscreen_skip(true);
}

#[test]
fn rows_are_prepared_before_they_scroll_into_the_viewport() {
    let mut scene = Scene::new(true);
    // Rows already drawn before the script count as prepared and visible at
    // frame zero.
    let mut first_updated = scene
        .counts()
        .iter()
        .map(|&count| (count > 0).then_some(0usize))
        .collect::<Vec<_>>();
    let mut first_visible = (0..ROWS)
        .map(|index| {
            (first_updated[index].is_some() && scene.near_viewport(index, 0.0)).then_some(0usize)
        })
        .collect::<Vec<_>>();
    let mut frame = 0;
    for _ in 0..6 {
        scene.scroll(-80.0, |scene, before, after| {
            frame += 1;
            for index in 0..ROWS {
                if first_updated[index].is_none() && after[index] > 0 {
                    first_updated[index] = Some(frame);
                }
                if first_visible[index].is_none()
                    && scene.near_viewport(index, 0.0)
                    && after[index] > before[index]
                {
                    first_visible[index] = Some(frame);
                }
            }
        });
    }

    let mut entered = 0;
    for index in 0..ROWS {
        if let (Some(updated), Some(visible)) = (first_updated[index], first_visible[index]) {
            assert!(
                updated <= visible,
                "row {index} was first updated after it was visible"
            );
            if visible > 0 {
                entered += 1;
                assert!(
                    updated < visible,
                    "row {index} was first updated in the frame it appeared"
                );
            }
        }
    }
    assert!(entered >= 5, "only {entered} rows scrolled into view");
    aimer_widget::set_settled_offscreen_skip(true);
}

#[test]
fn skipping_draws_exactly_what_a_full_revisit_draws() {
    let script = [-70.0, -45.0, 20.0, -130.0, -35.0, 90.0, -220.0, 15.0];
    let mut with_skip = Scene::new(true);
    let mut without_skip = Scene::new(false);
    aimer_widget::set_settled_offscreen_skip(true);

    for delta in script {
        // Deliver a fixed sequence at the smoother's output boundary. Using
        // two wall-clock-driven smoothers rounds each scene's offsets
        // differently and can change device classification under load.
        // Exact matching inputs let every frame's plan be compared exactly.
        for frame in 0..8 {
            let event = ElementEvent::Scroll {
                delta: Vec2d { x: 0.0, y: delta / 8.0 },
                phase: match frame {
                    0 => TouchPhase::Started,
                    7 => TouchPhase::Ended,
                    _ => TouchPhase::Moved,
                },
                kind: aimer_events::element::ScrollDeltaKind::Line,
                is_direct_manipulation: false,
            };
            for (skip, scene) in [(true, &mut with_skip), (false, &mut without_skip)] {
                aimer_widget::set_settled_offscreen_skip(skip);
                crate::aimer_app::request_scroll_frame(|| {
                    let result = scene.app.app.dispatch_element_event(
                        Vec2d { x: 50.0, y: 150.0 },
                        &event,
                    );
                    assert!(result.needs_redraw(), "the scroll step must move the list");
                });
                scene.app.render_frame();
            }

            assert_eq!(
                with_skip.offset(), without_skip.offset(),
                "offsets differ at frame {frame} after {delta}"
            );
            assert_eq!(
                with_skip.plan(), without_skip.plan(),
                "plans differ at frame {frame} after {delta}"
            );
        }
    }
    aimer_widget::set_settled_offscreen_skip(true);
}
