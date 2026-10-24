//! A list whose section and rows all read an application-defined theme follows
//! it when the theme changes, whichever direction.
//!
//! The shape is a sidebar's category list: a section that reads the theme for
//! its header and builds a lazily materialised list of rows that read it too.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_animation::curves;
use aimer_rubick::RcProjectExt;
use aimer_style::{AnimatedTheme, Theme, ThemeMode};
use aimer_widget::{StateUpdater, StatelessElement};

#[derive(Clone, Copy, Debug, PartialEq, aimer_macro::Theme)]
struct TestTheme {
    tone: u8,
    accent: u8,
}

impl TestTheme {
    fn light() -> Self {
        Self {
            tone: 30,
            accent: 200,
        }
    }

    fn dark() -> Self {
        Self {
            tone: 235,
            accent: 220,
        }
    }

    fn text(&self) -> Color {
        Color::Gray8(self.tone)
    }
}

const ROWS: usize = 9;

type Seen = Rc<RefCell<Vec<Option<u8>>>>;

struct Row_ {
    index: usize,
    seen: Seen,
}

impl Widget for Row_ {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatelessElement::from_builder(
            ctx,
            move |ctx| {
                let theme = TestTheme::of(ctx);
                self.seen.borrow_mut()[self.index] = Some(theme.tone);
                let label = aimer_rubick::Shared::new(format!("row {}", self.index));
                // A row like a category button: a button around an icon and a label.
                aimer_input::button::Button::new()
                    .hover_decoration(
                        aimer_style::BoxDecoration::new()
                            .border_radius(8)
                            .background_color(Color::Rgb(200, 100, 80)),
                    )
                    .decoration(
                        aimer_style::BoxDecoration::new()
                            .border_radius(8)
                            .background_color(Color::Transparent),
                    )
                    .on_press(|| {})
                    .child(
                        aimer_container::Container::new()
                            .padding(aimer_style::LayoutSpacing::vertical(8))
                            .child(
                                Row::new().children(vec![
                                    SizedBox::new().width(4).boxed(),
                                    aimer_text::Text::new(label.project(|text| text.as_str()))
                                        .text_style(
                                            aimer_style::TextStyle::new()
                                                .font_size(16)
                                                .color(theme.text()),
                                        )
                                        .boxed(),
                                ]),
                            ),
                    )
                    .to_element(ctx)
            },
            None,
            "ThemeRow",
        )
        .boxed()
    }
}

impl aimer_widget::PortableWidget for Row_ {}

struct Section {
    seen: Seen,
}

impl Widget for Section {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatelessElement::from_builder(
            ctx,
            move |ctx| {
                let theme = TestTheme::of(ctx);
                let seen = self.seen.clone();
                aimer_collapsible::CollapsibleList::new()
                    .animation_millis(300)
                    .header(
                        aimer_collapsible::ListHeader::new().child(
                            aimer_text::Text::new("Categories").text_style(
                                aimer_style::TextStyle::new()
                                    .font_size(20)
                                    .color(theme.text()),
                            ),
                        ),
                    )
                    .body(
                        aimer_collapsible::ListBody::new().child(
                            Column::new()
                                .list(0..ROWS)
                                .builder(move |index: &usize| Row_ {
                                    index: *index,
                                    seen: seen.clone(),
                                }),
                        ),
                    )
                    .to_element(ctx)
            },
            None,
            "ThemeSection",
        )
        .boxed()
    }
}

impl aimer_widget::PortableWidget for Section {}

struct ShellState {
    split: bool,
    mode: ThemeMode,
    seen: Seen,
    updater: Rc<RefCell<Option<StateUpdater<ShellState>>>>,
}

struct Shell {
    split: bool,
    seen: Seen,
    updater: Rc<RefCell<Option<StateUpdater<ShellState>>>>,
}

impl StatefulWidget for Shell {
    type State = ShellState;

    fn create_state(self) -> ShellState {
        ShellState {
            split: self.split,
            mode: ThemeMode::Dark,
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
            .adaptive(TestTheme::light(), TestTheme::dark())
            .mode(self.mode)
            .duration(Duration::from_millis(150))
            .curve(curves::EASE_IN_OUT)
            .child(Row::new().children(vec![
                SizedBox::new()
                    .width(if self.split { Dimension::Px(80.0) } else { Dimension::Px(150.0) })
                    .child(Scrollable::new().child(Section { seen: self.seen.clone() }))
                    .boxed(),
                SizedBox::new().width(Dimension::Px(70.0)).height(Dimension::Px(400.0)).boxed(),
            ]))
    }
}

impl Widget for Shell {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "CustomThemeShell", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Shell {}

fn run_for<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, millis: u64) {
    let end = std::time::Instant::now() + Duration::from_millis(millis);
    while std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }
}

/// The color of every piece of text the render tree holds.
fn painted_text_colors<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Vec<String> {
    fn walk<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        element: &dyn Element,
        out: &mut Vec<String>,
    ) {
        if let Some(node) = app.app.render_node_for_element(element.id())
            && let Ok(snapshot) = app.app.render_tree().draw_list_snapshot(node)
        {
            for command in snapshot.commands.iter() {
                if let aimer_cupid::draw_cmd_v2::DrawCommand::DrawText { color, .. } = command {
                    out.push(format!("{color:?}"));
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

fn tone_color(tone: u8) -> String {
    let color: aimer_cupid::utilities::Color = Color::Gray8(tone).into();
    format!("{color:?}")
}

fn switch_and_check(from: ThemeMode, to: ThemeMode, expected: u8) {
    let seen: Seen = Rc::new(RefCell::new(vec![None; ROWS]));
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Shell {
            split: false,
            seen: seen.clone(),
            updater: updater.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(300, 500),
            scale_factor: 2.0,
        },
    );
    app.pump_frames(3);
    if from != ThemeMode::Dark {
        updater
            .borrow()
            .expect("initialised")
            .set_state(move |state| state.mode = from);
        run_for(&mut app, 400);
    }
    updater
        .borrow()
        .expect("initialised")
        .set_state(move |state| state.mode = to);
    run_for(&mut app, 500);

    let stale: Vec<usize> = seen
        .borrow()
        .iter()
        .enumerate()
        .filter(|(_, tone)| **tone != Some(expected))
        .map(|(index, _)| index)
        .collect();
    assert!(
        stale.is_empty(),
        "rows kept the previous theme's tone after {from:?} -> {to:?}: {stale:?} of {:?}",
        seen.borrow()
    );

    let previous = if expected == TestTheme::light().tone {
        TestTheme::dark().tone
    } else {
        TestTheme::light().tone
    };
    let (old, new) = (tone_color(previous), tone_color(expected));
    let painted = painted_text_colors(&app);
    let stale_text = painted.iter().filter(|color| **color == old).count();
    assert!(
        painted.iter().any(|color| *color == new),
        "nothing was painted in the new theme's text color: {painted:?}"
    );
    assert_eq!(
        stale_text,
        0,
        "{stale_text} of {} painted texts still have the previous theme's color: {painted:?}",
        painted.len()
    );
}

#[test]
fn a_custom_theme_change_from_dark_to_light_reaches_every_row() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    switch_and_check(ThemeMode::Dark, ThemeMode::Light, TestTheme::light().tone);
}

#[test]
fn a_custom_theme_change_from_light_to_dark_reaches_every_row() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    switch_and_check(ThemeMode::Light, ThemeMode::Dark, TestTheme::dark().tone);
}

fn painted_hover_fills<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> usize {
    fn walk<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        element: &dyn Element,
        hover: &str,
        out: &mut usize,
    ) {
        if let Some(node) = app.app.render_node_for_element(element.id())
            && let Ok(snapshot) = app.app.render_tree().draw_list_snapshot(node)
        {
            for command in snapshot.commands.iter() {
                if let aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { color, .. } = command
                    && format!("{color:?}") == hover
                {
                    *out += 1;
                }
            }
        }
        element.visit_children(&mut |child| walk(app, child, hover, out));
    }
    let hover: aimer_cupid::utilities::Color = Color::Rgb(200, 100, 80).into();
    let mut out = 0;
    walk(
        app,
        app.app.widget_root.as_ref().unwrap().as_ref(),
        &format!("{hover:?}"),
        &mut out,
    );
    out
}

fn move_to<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
}

/// Sweeps the pointer down the rows and off them, one move per frame.
fn sweep_rows_and_leave(switch_first: bool) {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let seen: Seen = Rc::new(RefCell::new(vec![None; ROWS]));
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Shell {
            split: true,
            seen,
            updater: updater.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(300, 1000),
            scale_factor: 2.0,
        },
    );
    move_to(&mut app, -10.0, -10.0);
    app.pump_frames(3);
    if switch_first {
        updater
            .borrow()
            .expect("initialised")
            .set_state(|state| state.mode = ThemeMode::Light);
        run_for(&mut app, 500);
    }
    let mut y = 60.0;
    let mut most_hovered = 0;
    while y < 800.0 {
        move_to(&mut app, 100.0, y);
        run_for(&mut app, 16);
        most_hovered = most_hovered.max(painted_hover_fills(&app));
        y += 12.0;
    }
    eprintln!("most rows painted hovered at once during the sweep: {most_hovered}");
    assert!(most_hovered > 0, "the pointer never hovered a row");
    move_to(&mut app, 290.0, 950.0);
    run_for(&mut app, 400);
    assert_eq!(
        painted_hover_fills(&app),
        0,
        "rows are still painted hovered after the pointer left them"
    );
}

/// Sweeps down the rows, then leaves the list sideways in one move.
#[test]
fn rows_stop_being_hovered_when_the_pointer_leaves_the_list_sideways() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let seen: Seen = Rc::new(RefCell::new(vec![None; ROWS]));
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Shell {
            split: true,
            seen,
            updater,
        },
        HeadlessOptions {
            size: PhysicalSize::new(300, 1000),
            scale_factor: 2.0,
        },
    );
    move_to(&mut app, -10.0, -10.0);
    app.pump_frames(3);
    let mut y = 60.0;
    let mut most_hovered = 0;
    while y < 500.0 {
        move_to(&mut app, 60.0, y);
        run_for(&mut app, 16);
        most_hovered = most_hovered.max(painted_hover_fills(&app));
        y += 12.0;
    }
    assert!(most_hovered > 0, "the pointer never hovered a row");
    for x in [150.0, 190.0, 240.0] {
        move_to(&mut app, x, y);
        run_for(&mut app, 16);
    }
    run_for(&mut app, 300);
    assert_eq!(
        painted_hover_fills(&app),
        0,
        "rows are still painted hovered after the pointer left the list"
    );
}

#[test]
fn rows_swept_by_the_pointer_do_not_stay_hovered() {
    sweep_rows_and_leave(false);
}

#[test]
fn rows_swept_by_the_pointer_do_not_stay_hovered_after_a_theme_change() {
    sweep_rows_and_leave(true);
}
