//! What the page repaints when the bar over it changes its backdrop filter.
//!
//! A post page is a `Stack` of a bar pinned to the top and a long scrollable
//! column. Scrolling past a threshold flips the bar from transparent to a
//! backdrop filter. Held by the screen, that flag rebuilds the whole `Stack`,
//! re-records every visible row and damages the target; held by the bar, only
//! the bar is rebuilt and, without a custom-pipeline draw, only its rectangle is
//! damaged.

use super::*;

use std::cell::RefCell;

use aimer_cupid::damage_region::DamageSet;
use aimer_cupid::draw_cmd_v2::RenderOp;
use aimer_decoration::{BackdropFilter, Decoration, DecorationBox};
use aimer_widget::StateUpdater;

const ROWS: usize = 60;

fn bar_widget(scrolled: bool, filter: bool) -> impl Widget {
    let decoration = if scrolled && filter {
        BackdropFilter::new()
            .blur_radius(8.0)
            .saturation(0.65)
            .contrast(0.9)
            .notype()
    } else {
        Color::Transparent.notype()
    };
    DecorationBox::new().decoration(decoration).child(
        SizedBox::new()
            .width(Dimension::Px(300.0))
            .height(Dimension::Px(40.0))
            .color(if scrolled {
                Color::Rgb(40, 30, 30).with_alpha(0.3)
            } else {
                Color::Transparent
            }),
    )
}

struct BarState {
    filter: bool,
    scrolled: bool,
    updater: Rc<RefCell<Option<StateUpdater<BarState>>>>,
}

struct Bar {
    filter: bool,
    updater: Rc<RefCell<Option<StateUpdater<BarState>>>>,
}

impl StatefulWidget for Bar {
    type State = BarState;

    fn create_state(self) -> BarState {
        BarState {
            filter: self.filter,
            scrolled: false,
            updater: self.updater,
        }
    }
}

impl State<Bar> for BarState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        bar_widget(self.scrolled, self.filter)
    }
}

impl Widget for Bar {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "PostBar", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Bar {}

/// The page with its flag held by the bar alone.
struct IsolatedScene {
    filter: bool,
    updater: Rc<RefCell<Option<StateUpdater<BarState>>>>,
}

impl Widget for IsolatedScene {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let rows: Vec<aimer_widget::AnyWidget> = (0..ROWS)
            .map(|index| {
                SizedBox::new()
                    .width(Dimension::Px(280.0))
                    .height(Dimension::Px(20.0))
                    .color(Color::Rgb(10 + index as u8, 90, 160))
                    .boxed()
            })
            .collect();
        aimer_space::Stack::new().children(vec![
            aimer_space::Positioned::new().layer(3).top(0).box_child(Bar {
                filter: self.filter,
                updater: self.updater.clone(),
            }),
            Scrollable::new()
                .child(Column::new().children(rows))
                .boxed(),
        ])
        .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for IsolatedScene {}

fn headless<W: Widget + 'static>(widget: W) -> HeadlessAimerApp<ModalHost<W>> {
    let mut app = AimerApp::start_headless_with(
        widget,
        HeadlessOptions {
            size: PhysicalSize::new(300, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    app
}

fn snapshot<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Vec<(RenderNodeId, u64)> {
    let tree = app.app.render_tree().clone();
    tree.render_all()
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => {
                let revision = tree.draw_list_revision(item.element).unwrap();
                Some((item.element, revision))
            }
            _ => None,
        })
        .collect()
}

/// Flips the bar both ways and reports what each frame did.
fn measure<W: Widget + 'static>(
    app: &mut HeadlessAimerApp<W>,
    flip: &dyn Fn(bool),
) -> Vec<Flip> {
    let mut reports = Vec::new();
    for to in [true, false] {
        let before = snapshot(app);
        let full_syncs = app.app.render_tree.sync_counts.full;
        flip(to);
        app.pump_frames(1);
        let after = snapshot(app);
        let damage = app.last_frame_result.as_ref().unwrap().1.clone();
        let rerecorded = after
            .iter()
            .filter(|(element, revision)| {
                before
                    .iter()
                    .find(|(old, _)| old == element)
                    .is_none_or(|(_, old)| old != revision)
            })
            .count();
        reports.push(Flip {
            rerecorded,
            full_syncs: app.app.render_tree.sync_counts.full - full_syncs,
            damage,
        });
    }
    reports
}

/// What one frame after a flip did.
struct Flip {
    rerecorded: usize,
    full_syncs: u64,
    damage: DamageSet,
}

#[test]
fn a_bar_that_owns_its_flag_leaves_the_rows_alone() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    for filter in [false, true] {
        let updater = Rc::new(RefCell::new(None));
        let mut app = headless(IsolatedScene {
            filter,
            updater: updater.clone(),
        });
        let flips = measure(&mut app, &|to| {
            updater
                .borrow()
                .expect("initialised")
                .set_state(move |state| state.scrolled = to);
        });
        for flip in &flips {
            // The bar, its decoration box and its fill; a row is never one.
            assert!(
                flip.rerecorded <= 4,
                "filter={filter}: {} draws were re-recorded for a change to the bar",
                flip.rerecorded
            );
            assert_eq!(flip.full_syncs, 0, "filter={filter}: the render tree was rebuilt");
            if !filter {
                // A backdrop filter is a custom-pipeline draw, which the renderer
                // cannot composite over a retained target, so that frame (and the
                // one after it) legitimately covers the target.
                assert!(!flip.damage.is_full(), "the whole target was damaged");
                assert!(
                    flip.damage
                        .regions()
                        .iter()
                        .all(|region| region.y + region.height <= 40),
                    "damage reached below the bar: {:?}",
                    flip.damage.regions()
                );
            }
        }
    }
}

// ---- the page as RustForum builds it: a controller-driven bar ----------------

const SCROLLED_AT: f32 = 50.0;

struct ScrollBarState {
    controller: ScrollController,
    updater: StateUpdater<Self>,
    scrolled: bool,
}

struct ScrollBar {
    controller: ScrollController,
}

impl ScrollBarState {
    /// Points the controller's single callback at this state.
    ///
    /// `init_state` runs when a widget is turned into an element, which every
    /// rebuild of the parent does; reconciliation then keeps the OLD live state
    /// and drops the new one. A callback registered only in `init_state` is left
    /// pointing at the dropped state after the first parent rebuild, and the
    /// next scroll panics on its stale updater. The state that survives must
    /// register again from `adopt_config_from`.
    fn register_scroll_hook(&self) {
        let updater = self.updater;
        self.controller.on_scroll(move |offset: Vec2d| {
            let scrolled = offset.y > SCROLLED_AT;
            if scrolled != updater.read_state().scrolled {
                updater.set_state(move |s| s.scrolled = scrolled)
            }
        });
    }
}

impl StatefulWidget for ScrollBar {
    type State = ScrollBarState;

    fn create_state(self) -> ScrollBarState {
        ScrollBarState {
            controller: self.controller,
            updater: StateUpdater::new(),
            scrolled: false,
        }
    }
}

impl State<ScrollBar> for ScrollBarState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
        self.register_scroll_hook();
    }

    fn adopt_config_from(&mut self, _new: Self) {
        self.register_scroll_hook();
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        bar_widget(self.scrolled, true)
    }
}

impl Widget for ScrollBar {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ScrollBar", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for ScrollBar {}

struct HostState {
    show: bool,
    controller: ScrollController,
    updater: Rc<RefCell<Option<StateUpdater<HostState>>>>,
}

struct Host {
    controller: ScrollController,
    updater: Rc<RefCell<Option<StateUpdater<HostState>>>>,
}

impl StatefulWidget for Host {
    type State = HostState;

    fn create_state(self) -> HostState {
        HostState {
            show: true,
            controller: self.controller,
            updater: self.updater,
        }
    }
}

impl State<Host> for HostState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        if !self.show {
            return SizedBox::new().boxed();
        }
        let rows: Vec<aimer_widget::AnyWidget> = (0..ROWS)
            .map(|index| {
                SizedBox::new()
                    .width(Dimension::Px(280.0))
                    .height(Dimension::Px(20.0))
                    .color(Color::Rgb(10 + index as u8, 90, 160))
                    .boxed()
            })
            .collect();
        aimer_space::Stack::new()
            .children(vec![
                aimer_space::Positioned::new().layer(3).top(0).box_child(ScrollBar {
                    controller: self.controller.clone(),
                }),
                Scrollable::new()
                    .controller(self.controller.clone())
                    .child(Column::new().children(rows))
                    .boxed(),
            ])
            .boxed()
    }
}

impl Widget for Host {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Host", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Host {}

fn wheel(app: &mut HeadlessAimerApp<ModalHost<Host>>, delta: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(100.0, 200.0),
    });
    app.pump_frames(2);
    app.send_window_event(WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, delta)),
        phase: TouchPhase::Moved,
    });
}

/// Draws frames until the smoothed wheel step has been delivered.
fn settle(app: &mut HeadlessAimerApp<ModalHost<Host>>) {
    let mut frames = 0;
    while app.app.scroll_smoother.is_active() || frames == 0 {
        app.render_frame();
        frames += 1;
        assert!(frames < 400, "the smoother never finished");
    }
    app.pump_frames(3);
}

fn host() -> (
    HeadlessAimerApp<ModalHost<Host>>,
    Rc<RefCell<Option<StateUpdater<HostState>>>>,
    ScrollController,
) {
    let updater = Rc::new(RefCell::new(None));
    let controller = ScrollController::new();
    let app = headless(Host {
        controller: controller.clone(),
        updater: updater.clone(),
    });
    (app, updater, controller)
}

fn filter_is_drawn(app: &HeadlessAimerApp<ModalHost<Host>>) -> bool {
    app.app.render_tree().render_all().iter().any(|operation| match operation {
        RenderOp::Draw(item) => item
            .snapshot()
            .commands
            .iter()
            .any(|command| matches!(command, aimer_cupid::draw_cmd_v2::DrawCommand::DrawCustom { .. })),
        _ => false,
    })
}

#[test]
fn scrolling_then_rebuilding_the_page_keeps_the_bar_hook_valid() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, updater, controller) = host();
    assert!(!filter_is_drawn(&app));
    wheel(&mut app, -300.0);
    settle(&mut app);
    assert!(
        controller.offset().y > SCROLLED_AT,
        "the wheel did not scroll: {:?}",
        controller.offset()
    );
    assert!(filter_is_drawn(&app), "scrolling past the threshold turns the filter on");
    // The post's counts arrive and rebuild the page, replacing the bar's element.
    updater.borrow().unwrap().set_state(|_| {});
    app.pump_frames(3);
    wheel(&mut app, 300.0);
    settle(&mut app);
    assert!(
        !filter_is_drawn(&app),
        "the bar still follows the scroll after the page rebuilt"
    );
}

#[test]
fn leaving_the_page_while_it_still_scrolls_keeps_the_bar_hook_valid() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, updater, _) = host();
    wheel(&mut app, -300.0);
    app.render_frame();
    updater.borrow().unwrap().set_state(|s| s.show = false);
    app.pump_frames(40);
}
