//! A collapsed list keeps showing its collapsed chevron when the screen around
//! it rebuilds.
//!
//! The shape is a sidebar section: a collapsible list inside a stateful screen
//! that rebuilds whenever the pointer crosses the panel's grab band.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_widget::StateUpdater;
use winit::event::{ElementState, MouseButton};

/// A stateless section that loads its content, as a sidebar's category list
/// does: the same list is built for the waiting and for the loaded snapshot.
struct Section {
    title: String,
}

impl Widget for Section {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        aimer_widget::StatelessElement::from_builder(
            ctx,
            move |ctx| {
                let title = self.title.clone();
                aimer_widget::AsyncBuilder::new()
                    .key("categories")
                    .future(|| async { Ok::<_, String>(4usize) })
                    .child(move |snapshot| {
                        let rows = match snapshot {
                            aimer_widget::AsyncSnapshot::Data(rows) => *rows,
                            _ => 2,
                        };
                        aimer_collapsible::CollapsibleList::new()
                            .animation_millis(120)
                            .header(
                                aimer_collapsible::ListHeader::new()
                                    .child(aimer_text::Text::new(title.clone())),
                            )
                            .body(
                                aimer_collapsible::ListBody::new().child(
                                    Column::new().children(
                                        (0..rows)
                                            .map(|_| {
                                                SizedBox::new().height(Dimension::Px(30.0)).boxed()
                                            })
                                            .collect::<Vec<_>>(),
                                    ),
                                ),
                            )
                            .boxed()
                    })
                    .to_element(ctx)
            },
            None,
            "TestSection",
        )
        .boxed()
    }
}

impl aimer_widget::PortableWidget for Section {}

struct ScreenState {
    rebuilds: u32,
    animate: bool,
    updater: Rc<RefCell<Option<StateUpdater<ScreenState>>>>,
}

struct Screen {
    updater: Rc<RefCell<Option<StateUpdater<ScreenState>>>>,
}

impl StatefulWidget for Screen {
    type State = ScreenState;

    fn create_state(self) -> ScreenState {
        ScreenState {
            rebuilds: 0,
            animate: false,
            updater: self.updater,
        }
    }
}

impl State<Screen> for ScreenState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        // Each rebuild hands the list a new header, as a screen rebuilding does.
        let title = format!("Categories {}", self.rebuilds % 2);
        // The panel around the list animates while its edge is being grabbed, and
        // rebuilds its whole content on every frame of that.
        aimer_animation::ImplicitAnimatedBuilder::new(
            if self.animate { 1.0f32 } else { 0.0 },
            Duration::from_millis(300),
            aimer_animation::curves::EASE_IN_OUT,
            move |value| {
                let title = title.clone();
                aimer_container::Container::new()
                    .padding(aimer_style::LayoutSpacing::all(
                        ((*value * 4.0) as i32).max(0),
                    ))
                    .child(Column::new().children(vec![
                        aimer_flex::Expanded::new()
                            .child(Column::new().children(vec![
                                SizedBox::new().height(36).boxed(),
                                Section { title }.boxed(),
                            ]))
                            .boxed(),
                        SizedBox::new().height(12).boxed(),
                    ]))
            },
        )
    }
}

impl Widget for Screen {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ChevronScreen", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Screen {}

fn run_for<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, millis: u64) {
    let end = aimer_time::Instant::now() + Duration::from_millis(millis);
    while aimer_time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }
}

/// The turn the chevron is drawn at, as the compositor holds it.
fn chevron_turn<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Option<f32> {
    fn walk<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        element: &dyn Element,
        out: &mut Option<f32>,
    ) {
        if element.debug_name() == "RotationTransitionElement"
            && let Some(node) = app.app.render_node_for_element(element.id())
            && let Ok((matrix, _)) = app.app.render_tree().compositor_animation(node)
        {
            // The sine of the turn sits in the first column's second row.
            *out = Some(matrix.cols[0][1].atan2(matrix.cols[0][0]) / std::f32::consts::TAU);
        }
        element.visit_children(&mut |child| walk(app, child, out));
    }
    let mut out = None;
    walk(
        app,
        app.app.widget_root.as_ref().unwrap().as_ref(),
        &mut out,
    );
    out
}

fn click_at<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
        app.pump_frames(1);
    }
}

#[test]
fn a_collapsed_list_keeps_its_chevron_turned_when_the_screen_rebuilds() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Screen {
            updater: updater.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(300, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    let open = chevron_turn(&app).expect("the chevron is on screen");
    assert!(
        open.abs() < 0.01,
        "an expanded list shows the chevron un-turned, got {open}"
    );

    click_at(&mut app, 40.0, 56.0);
    run_for(&mut app, 400);
    let collapsed = chevron_turn(&app).expect("the chevron is still on screen");
    assert!(
        (collapsed + 0.25).abs() < 0.01,
        "a collapsed list shows the chevron turned a quarter, got {collapsed}"
    );

    for _ in 0..3 {
        updater
            .borrow()
            .expect("initialised")
            .set_state(|state| state.rebuilds += 1);
        run_for(&mut app, 120);
        let after = chevron_turn(&app).expect("the chevron is still on screen");
        assert!(
            (after + 0.25).abs() < 0.01,
            "the screen rebuilt and the collapsed list's chevron went back to {after} of a turn"
        );
    }
}

/// The element id of the chevron's transition, to see whether it is replaced.
fn chevron_id<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Option<u64> {
    fn walk(element: &dyn Element, out: &mut Option<u64>) {
        if element.debug_name() == "RotationTransitionElement" {
            *out = element.element_id().map(|id| id.get());
        }
        element.visit_children(&mut |child| walk(child, out));
    }
    let mut out = None;
    walk(app.app.widget_root.as_ref().unwrap().as_ref(), &mut out);
    out
}

#[test]
fn a_panel_that_animates_around_a_collapsed_list_keeps_the_chevron_turned_on_every_frame() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Screen {
            updater: updater.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(300, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    click_at(&mut app, 40.0, 56.0);
    run_for(&mut app, 400);
    let settled = chevron_turn(&app).expect("the chevron is on screen");
    assert!(
        (settled + 0.25).abs() < 0.01,
        "a collapsed list shows the chevron turned, got {settled}"
    );
    let id_before = chevron_id(&app);

    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.animate = true);
    let mut wrong = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    let end = aimer_time::Instant::now() + Duration::from_millis(500);
    while aimer_time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(16));
        app.pump_frames(1);
        if let Some(turn) = chevron_turn(&app)
            && (turn + 0.25).abs() > 0.01
        {
            wrong.push(turn);
        }
        ids.extend(chevron_id(&app));
    }
    assert_eq!(
        ids.len(),
        1,
        "the chevron's element was replaced {} times: {ids:?} (before: {id_before:?})",
        ids.len()
    );
    assert!(
        wrong.is_empty(),
        "the collapsed chevron was drawn at {} wrong angles during the panel's animation: {wrong:?}",
        wrong.len()
    );
}
