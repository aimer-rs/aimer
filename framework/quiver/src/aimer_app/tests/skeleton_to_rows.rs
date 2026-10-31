//! A list that shows pulsing placeholders and is then replaced by button rows
//! paints those rows at rest, not hovered.
//!
//! The shape is a sidebar's category list while its data streams in: a
//! collapsible list whose body is a lazily built column of animated skeleton
//! rows, replaced by a column of buttons once the data arrives.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_animation::{AnimatedBuilder, AnimationController, curves};
use aimer_rubick::RcProjectExt;
use aimer_widget::{StateUpdater, StatelessElement};

const ROWS: usize = 9;

struct SkeletonRow;

impl Widget for SkeletonRow {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let controller = AnimationController::new(Duration::from_millis(1000), curves::EASE_IN_OUT);
        controller.set_repeat(true);
        controller.set_auto_reverse(true);
        controller.forward_from_first_tick();
        AnimatedBuilder::new(controller, move |value| {
            let bar = Color::Rgb(90, 50, 40).with_alpha(0.12 + 0.18 * value);
            aimer_container::Container::new()
                .padding(aimer_style::LayoutSpacing::vertical(12))
                .child(
                    Row::new().children(vec![
                        SizedBox::new().width(4).boxed(),
                        aimer_container::Container::new()
                            .width(Dimension::Px(12.0))
                            .height(Dimension::Px(12.0))
                            .box_decoration(
                                aimer_style::BoxDecoration::new()
                                    .background_color(bar)
                                    .border_radius(6),
                            )
                            .child(aimer_container::ZeroSizedBox)
                            .boxed(),
                    ]),
                )
        })
        .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for SkeletonRow {}

struct ButtonRow {
    index: usize,
}

impl Widget for ButtonRow {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatelessElement::from_builder(
            ctx,
            move |ctx| {
                let label = aimer_rubick::Shared::new(format!("row {}", self.index));
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
                            .child(Row::new().children(vec![
                                SizedBox::new().width(4).boxed(),
                                aimer_text::Text::new(label.project(|text| text.as_str()))
                                    .text_style(aimer_style::TextStyle::new().font_size(16))
                                    .boxed(),
                            ])),
                    )
                    .to_element(ctx)
            },
            None,
            "SkeletonTestRow",
        )
        .boxed()
    }
}

impl aimer_widget::PortableWidget for ButtonRow {}

struct ShellState {
    loaded: bool,
    updater: Rc<RefCell<Option<StateUpdater<ShellState>>>>,
}

struct Shell {
    updater: Rc<RefCell<Option<StateUpdater<ShellState>>>>,
}

impl StatefulWidget for Shell {
    type State = ShellState;

    fn create_state(self) -> ShellState {
        ShellState {
            loaded: false,
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
        let body = if self.loaded {
            Column::new()
                .gaps(aimer_style::LayoutSpacing::all(2))
                .list(0..ROWS)
                .builder(|index: &usize| ButtonRow { index: *index })
                .boxed()
        } else {
            Column::new()
                .gaps(aimer_style::LayoutSpacing::all(2))
                .list(0..ROWS)
                .builder(|_: &usize| SkeletonRow)
                .boxed()
        };
        Scrollable::new().child(
            aimer_collapsible::CollapsibleList::new()
                .animation_millis(300)
                .header(
                    aimer_collapsible::ListHeader::new().child(aimer_text::Text::new("Categories")),
                )
                .body(aimer_collapsible::ListBody::new().child(body)),
        )
    }
}

impl Widget for Shell {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "SkeletonShell", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Shell {}

fn run_for<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, millis: u64) {
    let end = aimer_time::Instant::now() + Duration::from_millis(millis);
    while aimer_time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }
}

fn painted_fills<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Vec<String> {
    fn walk<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        element: &dyn Element,
        out: &mut Vec<String>,
    ) {
        if let Some(node) = app.app.render_node_for_element(element.id())
            && let Ok(snapshot) = app.app.render_tree().draw_list_snapshot(node)
        {
            for command in snapshot.commands.iter() {
                if let aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { color, .. } = command {
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

#[test]
fn rows_that_replace_placeholders_are_painted_at_rest() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Shell {
            updater: updater.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(300, 1000),
            scale_factor: 2.0,
        },
    );
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(-10.0, -10.0),
    });
    run_for(&mut app, 500);
    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.loaded = true);
    run_for(&mut app, 800);

    let hover: aimer_cupid::utilities::Color = Color::Rgb(200, 100, 80).into();
    let fills = painted_fills(&app);
    let hovered = fills
        .iter()
        .filter(|color| **color == format!("{hover:?}"))
        .count();
    assert_eq!(
        hovered, 0,
        "{hovered} rows are painted hovered at rest: {fills:?}"
    );
}
