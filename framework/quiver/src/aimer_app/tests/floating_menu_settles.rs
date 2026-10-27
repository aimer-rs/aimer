//! A floating menu that has finished entering is drawn exactly where it is laid
//! out: nothing of it is left scaled, shifted or translucent.
//!
//! The menu enters by scaling up from a slightly smaller size and fading in. The
//! panel behind the rows and the rows themselves are separate render nodes, so a
//! node that keeps the entry transform after the others have let go draws its
//! surface at the wrong size and place under rows that are right.

use super::*;

use std::time::Duration;

use aimer_animation::curves;
use aimer_cupid::draw_cmd_v2::RenderOp;
use aimer_cupid::utilities::Mat3;
use aimer_input::button::Button;
use aimer_modal::{Anchor, AnchorHandle, Floating, FloatingAlign, FloatingSide, ModalAnimation};
use aimer_style::{BoxDecoration, BoxShadow};

struct Screen {
    anchor: AnchorHandle,
}

impl Widget for Screen {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        // A chip pinned to the bottom, like the user button of a sidebar.
        Column::new()
            .children(vec![
                aimer_flex::Expanded::new().child(SizedBox::new()).boxed(),
                Anchor::new()
                    .handle(self.anchor)
                    .child(
                        Button::new()
                            .decoration(
                                BoxDecoration::new().background_color(Color::Rgb(60, 40, 40)),
                            )
                            .child(
                                SizedBox::new()
                                    .width(Dimension::Px(180.0))
                                    .height(Dimension::Px(44.0)),
                            ),
                    )
                    .boxed(),
            ])
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for Screen {}

fn panel() -> impl Widget {
    aimer_container::Container::new()
        .width(Dimension::Px(160.0))
        .height(Dimension::Px(130.0))
        .padding(aimer_style::LayoutSpacing::all(8))
        .box_decoration(
            BoxDecoration::new()
                .background_color(Color::Rgb(38, 26, 23))
                .border_radius(12)
                .box_shadow(vec![
                    BoxShadow::new()
                        .color(Color::BLACK.with_opacity(70))
                        .blur(18.0)
                        .offset_y(8.0),
                ]),
        )
        .child(
            Column::new().children(
                (0..3)
                    .map(|_| {
                        Button::new()
                            .decoration(BoxDecoration::new().background_color(Color::Transparent))
                            // A button is as big as its child; the rows span the
                            // panel's 144 px of content because the child asks to.
                            .child(
                                SizedBox::new()
                                    .width(Dimension::Percent(100.0))
                                    .height(Dimension::Px(36.0)),
                            )
                            .boxed()
                    })
                    .collect::<Vec<_>>(),
            ),
        )
}

#[test]
fn a_floating_menu_that_has_entered_keeps_no_entry_transform() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let anchor = AnchorHandle::new();
    let mut app = AimerApp::start_headless_with(
        Screen {
            anchor: anchor.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(400, 400),
            scale_factor: 2.0,
        },
    );
    app.pump_frames(3);

    let _handle = Floating::new()
        .anchor(anchor)
        .side(FloatingSide::Bottom)
        .align(FloatingAlign::Start)
        .gap(6.0)
        .animation(
            ModalAnimation::new()
                .enter_duration(Duration::from_millis(160))
                .exit_duration(Duration::from_millis(120))
                .enter_curve(curves::EASE_OUT)
                .exit_curve(curves::EASE_IN)
                .content_scale_from(0.94),
        )
        .child(panel())
        .show();
    let end = std::time::Instant::now() + Duration::from_millis(600);
    while std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }

    let operations = app.app.render_tree().render_all();
    let moved: Vec<String> = operations
        .iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) if item.transform != Mat3::identity() => Some(format!(
                "node {:?} transform {:?} bounds {:?}",
                item.element, item.transform, item.bounds
            )),
            _ => None,
        })
        .collect();
    assert!(
        moved.is_empty(),
        "nodes keep the entry transform after the menu settled: {moved:#?}"
    );
    let translucent = operations.iter().any(|operation| {
        matches!(
            operation,
            RenderOp::BeginOpacityGroup { opacity, .. } if *opacity < 1.0
        )
    });
    assert!(
        !translucent,
        "the menu is still drawn translucent after it settled"
    );
}

/// The world rectangle each filled box of the menu is drawn at.
fn filled_boxes<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Vec<(f32, f32, f32, f32)> {
    let mut boxes = Vec::new();
    for operation in app.app.render_tree().render_all() {
        let RenderOp::Draw(item) = operation else {
            continue;
        };
        let snapshot = item.snapshot();
        for command in snapshot.commands.iter() {
            if let aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { rect, .. } = command {
                let (x, y) = item
                    .transform
                    .transform_point(item.origin.0 + rect.x, item.origin.1 + rect.y);
                boxes.push((x, y, rect.width, rect.height));
            }
        }
    }
    boxes
}

#[test]
fn a_floating_menus_rows_are_drawn_inside_its_panel() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let anchor = AnchorHandle::new();
    let mut app = AimerApp::start_headless_with(
        Screen {
            anchor: anchor.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(400, 400),
            scale_factor: 2.0,
        },
    );
    app.pump_frames(3);

    let _handle = Floating::new()
        .anchor(anchor)
        .side(FloatingSide::Bottom)
        .align(FloatingAlign::Start)
        .gap(6.0)
        .animation(
            ModalAnimation::new()
                .enter_duration(Duration::from_millis(160))
                .content_scale_from(0.94),
        )
        .child(panel())
        .show();
    let end = std::time::Instant::now() + Duration::from_millis(600);
    while std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }

    let boxes = filled_boxes(&app);
    let panel = boxes.iter().find(|(_, _, width, height)| {
        (*width - 160.0).abs() < 1.0 && (*height - 130.0).abs() < 1.0
    });
    let panel = panel.unwrap_or_else(|| panic!("the panel is drawn: {boxes:?}"));
    let rows: Vec<_> = boxes
        .iter()
        .filter(|(_, _, width, height)| {
            (*width - 144.0).abs() < 1.0 && (*height - 36.0).abs() < 1.0
        })
        .collect();
    assert_eq!(rows.len(), 3, "three rows are drawn: {boxes:?}");
    for (x, y, width, height) in rows {
        assert!(
            *x >= panel.0 - 0.5
                && *y >= panel.1 - 0.5
                && x + width <= panel.0 + panel.2 + 0.5
                && y + height <= panel.1 + panel.3 + 0.5,
            "the row {:?} is drawn outside its panel {:?}",
            (x, y, width, height),
            panel
        );
    }
}
