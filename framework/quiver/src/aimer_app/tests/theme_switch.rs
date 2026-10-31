//! Changing the theme reaches every widget that reads it, and does not rebuild
//! the world to do it.
//!
//! Each probe records the color it last built with, so a test can tell a widget
//! that followed the theme from one that kept the color it was born with.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_animation::{curves, ImplicitAnimatedBuilder};
use aimer_rubick::RcProjectExt;
use aimer_style::{AnimatedTheme, Theme, ThemeData, ThemeMode};
use aimer_widget::{StateUpdater, StatelessElement};

const PLAIN: usize = 4;
const INSIDE: usize = 300;

type Seen = Rc<RefCell<Vec<Option<Color>>>>;

struct Probe {
    index: usize,
    seen: Seen,
}

impl Widget for Probe {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatelessElement::from_builder(
            ctx,
            move |ctx| {
                let color = ThemeData::of(ctx).on_surface_color;
                self.seen.borrow_mut()[self.index] = Some(color);
                if self.index % 2 == 1 {
                    // A label whose text is a shared reference, as the sidebar's
                    // category names are.
                    let label = aimer_rubick::Shared::new(format!("row {}", self.index));
                    return aimer_text::Text::new(label.project(|text| text.as_str()))
                        .text_style(aimer_style::TextStyle::new().font_size(14).color(color))
                        .to_element(ctx);
                }
                SizedBox::new()
                    .width(Dimension::Px(100.0))
                    .height(Dimension::Px(20.0))
                    .color(color)
                    .to_element(ctx)
            },
            None,
            "ThemeProbe",
        )
        .boxed()
    }
}

impl aimer_widget::PortableWidget for Probe {}

struct RootState {
    mode: ThemeMode,
    seen: Seen,
    updater: Rc<RefCell<Option<StateUpdater<RootState>>>>,
}

struct Root {
    seen: Seen,
    updater: Rc<RefCell<Option<StateUpdater<RootState>>>>,
}

impl StatefulWidget for Root {
    type State = RootState;

    fn create_state(self) -> RootState {
        RootState {
            mode: ThemeMode::Light,
            seen: self.seen,
            updater: self.updater,
        }
    }
}

impl State<Root> for RootState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let seen = self.seen.clone();
        AnimatedTheme::new()
            .mode(self.mode)
            .duration(Duration::from_millis(120))
            .curve(curves::EASE_IN_OUT)
            .child(Sidebar { seen })
    }
}

impl Widget for Root {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ThemeRoot", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Root {}

/// The shape of RustForum's sidebar: some rows outside any animation, and a
/// scrolled list of rows inside an implicitly animated container.
struct Sidebar {
    seen: Seen,
}

impl Widget for Sidebar {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatelessElement::from_builder(
            ctx,
            move |ctx| {
                let theme = ThemeData::of(ctx);
                let seen = self.seen.clone();
                sidebar_content(seen, theme.background_color).to_element(ctx)
            },
            None,
            "ThemeSidebar",
        )
        .boxed()
    }
}

impl aimer_widget::PortableWidget for Sidebar {}

fn sidebar_content(seen: Seen, background: Color) -> impl Widget {
    {
        aimer_container::Container::new()
            .color(background)
            .child(ImplicitAnimatedBuilder::new(
                0.0f32,
                Duration::from_millis(120),
                curves::EASE_IN_OUT,
                move |_| {
                    let probe = |index: usize| {
                        Probe {
                            index,
                            seen: seen.clone(),
                        }
                        .boxed()
                    };
                    aimer_container::Resizable::new()
                        .height(f32::MAX)
                        .width(250.0)
                        .min_width(200.0)
                        .max_width(500.0)
                        .direction(aimer_container::Direction::RIGHT)
                        .child(
                            Column::new().children(vec![
                                Column::new()
                                    .children((0..PLAIN).map(probe).collect::<Vec<_>>())
                                    .boxed(),
                                aimer_flex::Expanded::new()
                                    .child(
                                        Scrollable::new().child(
                                            aimer_collapsible::CollapsibleList::new()
                                                .animation_millis(300)
                                                .header(
                                                    aimer_collapsible::ListHeader::new().child(
                                                        SizedBox::new()
                                                            .width(Dimension::Px(50.0))
                                                            .height(Dimension::Px(20.0)),
                                                    ),
                                                )
                                                .body(
                                                    aimer_collapsible::ListBody::new().child(
                                                        Column::new()
                                                            .list(PLAIN..PLAIN + INSIDE)
                                                            .builder({
                                                                let seen = seen.clone();
                                                                move |index: &usize| Probe {
                                                                    index: *index,
                                                                    seen: seen.clone(),
                                                                }
                                                            }),
                                                    ),
                                                ),
                                        ),
                                    )
                                    .boxed(),
                            ]),
                        )
                        .boxed()
                },
            ))
    }
}

fn start() -> (
    HeadlessAimerApp<impl Widget + 'static>,
    Seen,
    Rc<RefCell<Option<StateUpdater<RootState>>>>,
) {
    let seen: Seen = Rc::new(RefCell::new(vec![None; PLAIN + INSIDE]));
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Root {
            seen: seen.clone(),
            updater: updater.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(300, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    (app, seen, updater)
}

fn run_for<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, millis: u64) {
    let end = aimer_time::Instant::now() + Duration::from_millis(millis);
    while aimer_time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }
}

#[test]
fn a_theme_change_reaches_every_widget_that_reads_the_theme() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, seen, updater) = start();
    let light = ThemeData::light().on_surface_color;
    let dark = ThemeData::dark().on_surface_color;
    assert!(
        seen.borrow().iter().all(|color| *color == Some(light)),
        "the probes start light"
    );

    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.mode = ThemeMode::Dark);
    run_for(&mut app, 500);

    let stale: Vec<usize> = seen
        .borrow()
        .iter()
        .enumerate()
        .filter(|(_, color)| **color != Some(dark))
        .map(|(index, _)| index)
        .collect();
    assert!(
        stale.is_empty(),
        "these widgets kept the old theme's color: {stale:?}"
    );
}

#[test]
fn a_theme_change_settles_and_the_window_goes_idle() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, _seen, updater) = start();

    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.mode = ThemeMode::Dark);
    run_for(&mut app, 600);

    assert_eq!(
        app.pump_frames(10),
        0,
        "the window is still drawing frames after the theme settled"
    );
}

/// The fill colors the probes' render nodes actually hold.
fn painted_probe_colors<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Vec<String> {
    fn walk<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        element: &dyn Element,
        out: &mut Vec<String>,
    ) {
        if let Some(node) = app.app.render_node_for_element(element.id())
            && let Ok(snapshot) = app.app.render_tree().draw_list_snapshot(node)
        {
            for command in snapshot.commands.iter() {
                match command {
                    aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { color, .. }
                    | aimer_cupid::draw_cmd_v2::DrawCommand::DrawText { color, .. } => {
                        out.push(format!("{color:?}"));
                    }
                    _ => {}
                }
            }
        }
        element.visit_children(&mut |child| walk(app, child, out));
    }
    let mut out = Vec::new();
    walk(
        app,
        app.app.widget_root.as_ref().unwrap().as_ref(),
        &mut out,
    );
    out
}

#[test]
fn a_theme_change_repaints_every_widget_that_reads_the_theme() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, _seen, updater) = start();
    let light: aimer_cupid::utilities::Color = ThemeData::light().on_surface_color.into();
    let dark: aimer_cupid::utilities::Color = ThemeData::dark().on_surface_color.into();
    let (light, dark) = (format!("{light:?}"), format!("{dark:?}"));
    let before = painted_probe_colors(&app);
    assert!(
        before.iter().filter(|color| **color == light).count() >= PLAIN,
        "probes start light: {before:?}"
    );

    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.mode = ThemeMode::Dark);
    run_for(&mut app, 500);

    let after = painted_probe_colors(&app);
    let stale = after.iter().filter(|color| **color == light).count();
    assert_eq!(
        stale,
        0,
        "{stale} of {} painted probes still show the old theme: {after:?}",
        after.len()
    );
}

#[test]
fn a_theme_change_does_not_search_the_whole_tree_for_every_widget_it_touches() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, _seen, updater) = start();
    let before = aimer_widget::structural_search_count();

    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.mode = ThemeMode::Dark);
    let mut frames = 0;
    let end = aimer_time::Instant::now() + Duration::from_millis(400);
    while aimer_time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        frames += app.pump_frames(1);
    }

    let searches = aimer_widget::structural_search_count() - before;
    assert!(frames > 0);
    assert!(
        searches <= 2 * frames as u64,
        "{searches} searches of the whole tree in {frames} frames of a theme change"
    );
}

/// The same sidebar as a stateful widget that rebuilds itself when the pointer
/// reaches its resize edge, as RustForum's does.
struct StatefulSidebarState {
    resizable: bool,
    zone_log: Rc<RefCell<Vec<bool>>>,
    seen: Seen,
    updater: Rc<RefCell<Option<StateUpdater<StatefulSidebarState>>>>,
}

struct StatefulSidebar {
    seen: Seen,
    zone_log: Rc<RefCell<Vec<bool>>>,
    updater: Rc<RefCell<Option<StateUpdater<StatefulSidebarState>>>>,
}

impl StatefulWidget for StatefulSidebar {
    type State = StatefulSidebarState;

    fn create_state(self) -> StatefulSidebarState {
        StatefulSidebarState {
            resizable: false,
            zone_log: self.zone_log,
            seen: self.seen,
            updater: self.updater,
        }
    }
}

impl State<StatefulSidebar> for StatefulSidebarState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::of(ctx);
        let seen = self.seen.clone();
        self.zone_log.borrow_mut().push(self.resizable);
        let zone_updater = self.updater.borrow().expect("initialised");
        let background = theme.background_color;
        ImplicitAnimatedBuilder::new(
            if self.resizable { 1.0f32 } else { 0.0 },
            Duration::from_millis(120),
            curves::EASE_IN_OUT,
            move |delta| {
                let probe = |index: usize| {
                    Probe {
                        index,
                        seen: seen.clone(),
                    }
                    .boxed()
                };
                let seen_rows = seen.clone();
                let stroke = 1.0 + 2.0 * *delta;
                aimer_container::Container::new()
                    .box_decoration(
                        aimer_style::BoxDecoration::new()
                            .background_color(background)
                            .outline(
                                aimer_style::BoxOutline::new().right(
                                    aimer_style::BorderSlice::new()
                                        .color(Color::Rgb(255, 99, 71))
                                        .style(aimer_style::BorderStyle::Solid)
                                        .stroke(stroke),
                                ),
                            ),
                    )
                    .child(
                        aimer_container::Resizable::new()
                            .height(f32::MAX)
                            .width(250.0)
                            .handle_thickness(10.0)
                            .handle_outset(6.0)
                            .direction(aimer_container::Direction::RIGHT)
                            .on_resize_zone(move |zone: aimer_container::Direction| {
                                zone_updater.set_state(move |state| {
                                    state.resizable = zone != aimer_container::Direction::NONE;
                                });
                            })
                            .child(
                                Column::new().children(vec![
                                    Column::new()
                                        .children((0..PLAIN).map(probe).collect::<Vec<_>>())
                                        .boxed(),
                                    aimer_flex::Expanded::new()
                                        .child(
                                            Scrollable::new().child(
                                                aimer_collapsible::CollapsibleList::new()
                                                    .animation_millis(300)
                                                    .header(
                                                        aimer_collapsible::ListHeader::new().child(
                                                            SizedBox::new()
                                                                .width(Dimension::Px(50.0))
                                                                .height(Dimension::Px(20.0)),
                                                        ),
                                                    )
                                                    .body(
                                                        aimer_collapsible::ListBody::new().child(
                                                            Column::new()
                                                                .list(PLAIN..PLAIN + INSIDE)
                                                                .builder(move |index: &usize| {
                                                                    Probe {
                                                                        index: *index,
                                                                        seen: seen_rows.clone(),
                                                                    }
                                                                }),
                                                        ),
                                                    ),
                                            ),
                                        )
                                        .boxed(),
                                ]),
                            ),
                    )
            },
        )
    }
}

impl Widget for StatefulSidebar {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "StatefulSidebar", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for StatefulSidebar {}

struct ShellState {
    mode: ThemeMode,
    zone_log: Rc<RefCell<Vec<bool>>>,
    seen: Seen,
    updater: Rc<RefCell<Option<StateUpdater<ShellState>>>>,
}

struct Shell {
    seen: Seen,
    zone_log: Rc<RefCell<Vec<bool>>>,
    updater: Rc<RefCell<Option<StateUpdater<ShellState>>>>,
}

impl StatefulWidget for Shell {
    type State = ShellState;

    fn create_state(self) -> ShellState {
        ShellState {
            mode: ThemeMode::Light,
            zone_log: self.zone_log,
            seen: self.seen,
            updater: self.updater,
        }
    }
}

impl State<Shell> for ShellState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        AnimatedTheme::new()
            .mode(self.mode)
            .duration(Duration::from_millis(120))
            .curve(curves::EASE_IN_OUT)
            .child(
                Row::new().children(vec![
                    StatefulSidebar {
                        seen: self.seen.clone(),
                        zone_log: self.zone_log.clone(),
                        updater: Rc::new(RefCell::new(None)),
                    }
                    .boxed(),
                    // The content beside the sidebar: a scroll view of hover rows.
                    aimer_flex::Expanded::new()
                        .child(
                            Scrollable::new().child(
                                Column::new().children(
                                    (0..6)
                                        .map(|_| {
                                            aimer_input::mouse_region::MouseRegion::new().box_child(
                                                SizedBox::new()
                                                    .height(Dimension::Px(80.0))
                                                    .color(Color::Rgb(30, 24, 22)),
                                            )
                                        })
                                        .collect::<Vec<_>>(),
                                ),
                            ),
                        )
                        .boxed(),
                ]),
            )
    }
}

impl Widget for Shell {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ThemeShell", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Shell {}

#[test]
fn a_stateful_sidebar_that_rebuilt_for_its_resize_edge_still_follows_the_theme() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let seen: Seen = Rc::new(RefCell::new(vec![None; PLAIN + INSIDE]));
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Shell {
            seen: seen.clone(),
            zone_log: Rc::new(RefCell::new(Vec::new())),
            updater: updater.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(500, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    // The pointer visits the resize edge and leaves it, rebuilding the sidebar
    // twice, then the theme changes.
    for x in [250.0, 100.0] {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, 200.0),
        });
        run_for(&mut app, 300);
    }

    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.mode = ThemeMode::Dark);
    run_for(&mut app, 500);

    let dark = ThemeData::dark().on_surface_color;
    let stale: Vec<usize> = seen
        .borrow()
        .iter()
        .enumerate()
        .filter(|(_, color)| **color != Some(dark))
        .map(|(index, _)| index)
        .collect();
    assert!(
        stale.is_empty(),
        "these widgets kept the old theme's color: {stale:?}"
    );
}

fn shell_with_log() -> (
    HeadlessAimerApp<impl Widget + 'static>,
    Rc<RefCell<Vec<bool>>>,
) {
    let seen: Seen = Rc::new(RefCell::new(vec![None; PLAIN + INSIDE]));
    let zone_log = Rc::new(RefCell::new(Vec::new()));
    let mut app = AimerApp::start_headless_with(
        Shell {
            seen,
            zone_log: zone_log.clone(),
            updater: Rc::new(RefCell::new(None)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(600, 400),
            scale_factor: 1.0,
        },
    );
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(500.0, 200.0),
    });
    app.pump_frames(3);
    (app, zone_log)
}

fn glide<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, 200.0),
    });
    run_for(app, 40);
}

#[test]
fn the_sidebar_edge_reports_a_zone_in_both_halves_of_its_band_and_clears_when_left() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Pointer arrives from the content side, crosses the edge and returns.
    for rest_at in [250.0 + 4.0, 250.0 - 4.0, 250.0 + 5.0, 250.0 - 9.0] {
        let (mut app, log) = shell_with_log();
        for x in [490.0, 400.0, 300.0, 270.0, 258.0, rest_at] {
            glide(&mut app, x);
        }
        run_for(&mut app, 400);
        assert_eq!(
            log.borrow().last().copied(),
            Some(true),
            "resting at {rest_at} did not report the zone: {:?}",
            log.borrow()
        );

        for x in [270.0, 300.0, 400.0, 490.0] {
            glide(&mut app, x);
        }
        run_for(&mut app, 400);
        assert_eq!(
            log.borrow().last().copied(),
            Some(false),
            "the zone stayed reported after the pointer left it ({rest_at}): {:?}",
            log.borrow()
        );
    }
}

#[test]
fn a_sidebar_that_animates_for_its_zone_is_told_when_the_pointer_leaves_during_the_animation() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // The pointer reaches the resize edge, which starts the sidebar's animation,
    // and leaves for the content pane while that animation is still running.
    for (wait_ms, target_x) in [(8, 400.0), (24, 400.0), (50, 300.0), (8, 270.0)] {
        let (mut app, log) = shell_with_log();
        for x in [490.0, 400.0, 300.0, 270.0, 250.0 - 3.0] {
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(x, 200.0),
            });
            app.pump_frames(1);
        }
        run_for(&mut app, wait_ms);
        assert_eq!(
            log.borrow().last().copied(),
            Some(true),
            "entering the edge must be reported: {:?}",
            log.borrow()
        );

        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(target_x, 200.0),
        });
        run_for(&mut app, 500);

        assert_eq!(
            log.borrow().last().copied(),
            Some(false),
            "the zone stayed reported after leaving to x={target_x} {wait_ms} ms into the animation: {:?}",
            log.borrow()
        );
    }
}
