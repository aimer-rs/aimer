//! A list of pulsing placeholders keeps animating with no input at all.
//!
//! The shape is a sidebar section while its data loads: an async builder that
//! stays waiting, a collapsible list, and a lazily built column of rows that
//! each pulse on a controller of their own.

use super::*;

use std::time::Duration;

use aimer_animation::{AnimatedBuilder, AnimationController, curves};

const ROWS: usize = 9;

struct SkeletonRow;

impl Widget for SkeletonRow {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let controller = AnimationController::new(Duration::from_millis(1000), curves::EASE_IN_OUT);
        controller.set_repeat(true);
        controller.set_auto_reverse(true);
        controller.forward_from_first_tick();
        AnimatedBuilder::new(controller, move |value| {
            let bar = Color::Rgb(255, 146, 110).with_alpha(0.12 + 0.18 * value);
            aimer_container::Container::new()
                .padding(aimer_style::LayoutSpacing::vertical(12))
                .child(
                    Row::new().children(vec![
                        SizedBox::new().width(4).boxed(),
                        aimer_container::Container::new()
                            .width(Dimension::Px(110.0))
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

struct Screen;

impl Widget for Screen {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        Scrollable::new()
            .child(
                aimer_widget::AsyncBuilder::new()
                    .key("categories")
                    // Never answers, so the section stays in its waiting state.
                    .future(|| async {
                        std::future::pending::<()>().await;
                        Ok::<_, String>(())
                    })
                    .child(|_snapshot| {
                        aimer_collapsible::CollapsibleList::new()
                            .animation_millis(120)
                            .header(
                                aimer_collapsible::ListHeader::new()
                                    .child(aimer_text::Text::new("Categories")),
                            )
                            .body(
                                aimer_collapsible::ListBody::new().child(
                                    Column::new()
                                        .gaps(aimer_style::LayoutSpacing::all(2))
                                        .list(0..ROWS)
                                        .builder(|_: &usize| SkeletonRow),
                                ),
                            )
                            .boxed()
                    }),
            )
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for Screen {}

/// The distinct translucent fills the render tree holds for the pulsing bars.
fn bar_fills<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Vec<String> {
    fn walk<W: Widget + 'static>(
        app: &HeadlessAimerApp<W>,
        element: &dyn Element,
        out: &mut Vec<String>,
    ) {
        if let Some(node) = app.app.render_node_for_element(element.id())
            && let Ok(snapshot) = app.app.render_tree().draw_list_snapshot(node)
        {
            for command in snapshot.commands.iter() {
                if let aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { color, rect, .. } = command
                    && rect.width > 100.0
                    && rect.height < 14.0
                {
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

/// One placeholder row alone: the pulsing bar is several levels below the
/// builder, and a frame that revisits only the builder has to repaint all of it.
struct LoneRow;

impl Widget for LoneRow {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        SkeletonRow.to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for LoneRow {}

#[test]
fn a_placeholder_nested_below_its_builder_keeps_pulsing() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless_with(
        LoneRow,
        HeadlessOptions {
            size: PhysicalSize::new(300, 600),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..60 {
        std::thread::sleep(Duration::from_millis(16));
        app.pump_frames(1);
        seen.extend(bar_fills(&app));
    }
    assert!(
        seen.len() > 5,
        "the bar was painted with only {} distinct fills over a second: {seen:?}",
        seen.len()
    );
}

#[test]
fn placeholders_keep_pulsing_with_no_input() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless_with(
        Screen,
        HeadlessOptions {
            size: PhysicalSize::new(300, 600),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);

    let mut stalled = 0;
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..60 {
        std::thread::sleep(Duration::from_millis(16));
        if app.pump_frames(1) == 0 {
            stalled += 1;
        }
        seen.extend(bar_fills(&app));
    }
    assert_eq!(
        stalled, 0,
        "{stalled} of 60 frames were never asked for while the placeholders pulsed"
    );
    assert!(
        seen.len() > 5,
        "the bars were painted with only {} distinct fills over a second: {seen:?}",
        seen.len()
    );
}
