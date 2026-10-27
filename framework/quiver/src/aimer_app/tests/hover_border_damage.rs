//! A decorated container repaints everything it paints.
//!
//! A card with a border, a padding or a margin paints its background and border
//! over its whole box, but its render node is only as large as its content. The
//! damage for a repaint, a hover for instance, comes from the node, so the
//! strips of the border beyond it were left as they were: the card being
//! hovered kept the old color on its right and bottom edges, and the card the
//! cursor had just left kept the hover color there.
use super::*;

use aimer_style::{BorderSlice, BorderStyle, BoxBorder, BoxDecoration};

struct Cards;

impl Widget for Cards {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let card = || {
            aimer_container::Container::new()
                .margin(aimer_style::LayoutSpacing::all(12))
                .child(
                    aimer_container::Container::new()
                        .height(160)
                        .width(Dimension::Auto)
                        .child(
                            aimer_input::button::Button::new()
                                .decoration(
                                    BoxDecoration::new()
                                        .background_color(Color::Rgb(30, 20, 20))
                                        .border_radius(8)
                                        .border(BoxBorder::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Dotted)
                                                .stroke(2)
                                                .color(Color::Rgb(60, 40, 40)),
                                        )),
                                )
                                .hover_decoration(
                                    BoxDecoration::new()
                                        .background_color(Color::Rgb(60, 30, 25))
                                        .border_radius(8)
                                        .border(BoxBorder::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .stroke(2)
                                                .color(Color::Rgb(240, 100, 70)),
                                        )),
                                )
                                // A button is as big as its child; the card spans
                                // the width of its slot because the child asks to.
                                .child(
                                    SizedBox::new()
                                        .width(Dimension::Percent(100.0))
                                        .height(Dimension::Px(100.0)),
                                ),
                        ),
                )
                .boxed()
        };
        Column::new()
            .children(vec![card(), card(), card(), card()])
            .to_element(ctx)
    }
}
impl aimer_widget::PortableWidget for Cards {}

fn decoration_nodes<W: Widget + 'static>(
    root: &dyn Element,
    app: &HeadlessAimerApp<W>,
    out: &mut Vec<aimer_cupid::draw_cmd_v2::RenderNodeId>,
) {
    if root.debug_name() == "Container" {
        if let Some(node) = app.app.render_node_for_element(root.id()) {
            out.push(node);
        }
    }
    root.visit_children(&mut |child| decoration_nodes(child, app, out));
}

/// The part of the window a node's own fill covers, in logical pixels.
fn painted_fill<W: Widget + 'static>(
    app: &HeadlessAimerApp<W>,
    node: aimer_cupid::draw_cmd_v2::RenderNodeId,
) -> aimer_cupid::utilities::Rect {
    let bounds = app.app.render_tree().element_bounds(node).unwrap();
    let snapshot = app.app.render_tree().draw_list_snapshot(node).unwrap();
    let fill = snapshot
        .commands
        .iter()
        .find_map(|command| match command {
            aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("the decorated container records a fill");
    aimer_cupid::utilities::Rect {
        x: bounds.x + fill.x,
        y: bounds.y + fill.y,
        width: fill.width,
        height: fill.height,
    }
}

fn covers(
    damage: &aimer_cupid::damage_region::DamageSet,
    rect: aimer_cupid::utilities::Rect,
    scale: f32,
) -> bool {
    let (left, top) = ((rect.x * scale).floor(), (rect.y * scale).floor());
    let (right, bottom) = (
        ((rect.x + rect.width) * scale).ceil(),
        ((rect.y + rect.height) * scale).ceil(),
    );
    damage.is_full()
        || damage.regions().iter().any(|region| {
            (region.x as f32) <= left
                && (region.y as f32) <= top
                && (region.x + region.width) as f32 >= right
                && (region.y + region.height) as f32 >= bottom
        })
}

#[test]
fn a_hovered_card_repaints_its_whole_border() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    for (width, scale) in [(1003u32, 2.0f32), (1001, 2.0), (1000, 1.0)] {
        let mut app = AimerApp::start_headless_with(
            Cards,
            HeadlessOptions {
                size: PhysicalSize::new(width, 1400),
                scale_factor: f64::from(scale),
            },
        );
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(-10.0, -10.0),
        });
        app.pump_frames(3);
        let _ = direct_headless_frame_packet(&mut app);
        let root = app.app.widget_root.as_ref().unwrap();
        let mut nodes = Vec::new();
        decoration_nodes(root.as_ref(), &app, &mut nodes);
        // Per card: the margin box, the fixed-height box, the decorated box.
        let decorated: Vec<_> = nodes.chunks(3).map(|card| card[2]).collect();

        for (index, node) in decorated.iter().enumerate().take(3) {
            let painted = painted_fill(&app, *node);
            let y = (painted.y + painted.height / 2.0) * scale;
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(f64::from(300.0 * scale), f64::from(y)),
            });
            let (_, packet) = direct_headless_frame_packet(&mut app);
            assert!(
                covers(packet.metadata().damage(), painted, scale),
                "hovering card {index} at width {width}, scale {scale}: damage {:?} misses the painted box {painted:?}",
                packet.metadata().damage().regions(),
            );
        }
        // Leaving a card repaints it as well.
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(-10.0, -10.0),
        });
        let (_, packet) = direct_headless_frame_packet(&mut app);
        let left = painted_fill(&app, decorated[2]);
        assert!(
            covers(packet.metadata().damage(), left, scale),
            "leaving card 2 at width {width}, scale {scale}: damage {:?} misses {left:?}",
            packet.metadata().damage().regions(),
        );
    }
}

struct Padded;

impl Widget for Padded {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        aimer_container::Container::new()
            .margin(aimer_style::LayoutSpacing::all(12))
            .padding(aimer_style::LayoutSpacing::all(20))
            .height(160)
            .box_decoration(
                BoxDecoration::new()
                    .background_color(Color::Rgb(30, 20, 20))
                    .border_radius(8)
                    .border(BoxBorder::all(
                        BorderSlice::new()
                            .style(BorderStyle::Solid)
                            .stroke(2)
                            .color(Color::Rgb(240, 100, 70)),
                    )),
            )
            .child(
                SizedBox::new()
                    .width(Dimension::Px(50.0))
                    .height(Dimension::Px(50.0)),
            )
            .to_element(ctx)
    }
}
impl aimer_widget::PortableWidget for Padded {}

#[test]
fn a_decorated_node_covers_what_it_paints() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless_with(
        Padded,
        HeadlessOptions {
            size: PhysicalSize::new(1000, 600),
            scale_factor: 2.0,
        },
    );
    app.pump_frames(3);
    let root = app.app.widget_root.as_ref().unwrap();
    let mut nodes = Vec::new();
    decoration_nodes(root.as_ref(), &app, &mut nodes);
    let node = nodes[0];

    let painted = painted_fill(&app, node);
    let bounds = app.app.render_tree().element_bounds(node).unwrap();
    assert!(
        bounds.x <= painted.x
            && bounds.y <= painted.y
            && bounds.x + bounds.width >= painted.x + painted.width
            && bounds.y + bounds.height >= painted.y + painted.height,
        "the node {bounds:?} does not contain the box it paints {painted:?}"
    );
}
