//! Gaining focus repaints the box that reports it, the way the Jaime focus
//! example draws its border.

use super::*;

use aimer_style::{BorderSlice, BorderStyle, BoxBorder, BoxDecoration, Stroke};
use aimer_widget::Focusable;
use winit::event::ElementState;

fn decoration(focused: bool) -> BoxDecoration {
    BoxDecoration::new()
        .background_color(Color::WHITE)
        .border(BoxBorder::all(
            BorderSlice::new()
                .style(BorderStyle::Solid)
                .stroke(Stroke::Px(3.0))
                .color(if focused {
                    Color::BLUE
                } else {
                    Color::Transparent
                }),
        ))
}

struct Boxes {
    node: FocusNode,
}

struct BoxesState {
    node: FocusNode,
    focused: bool,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for Boxes {
    type State = BoxesState;

    fn create_state(self) -> BoxesState {
        BoxesState {
            node: self.node,
            focused: false,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<Boxes> for BoxesState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let updater = self.updater;
        Column::new().children([
            Focusable::new()
                .node(self.node.clone())
                .on_focus_change(move |gained| updater.set_state(move |s| s.focused = gained))
                .box_child(
                    aimer_container::Container::new()
                        .width(aimer_attribute::Dimension::Px(120.0))
                        .height(aimer_attribute::Dimension::Px(60.0))
                        .box_decoration(decoration(self.focused))
                        .box_child(SizedBox::new().width(1.0).height(1.0)),
                ),
            SizedBox::new().width(100.0).height(100.0).boxed(),
        ])
    }
}

impl Widget for Boxes {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Boxes", None)
            .0
            .boxed()
    }
}
impl aimer_widget::PortableWidget for Boxes {}

fn containers<W: Widget + 'static>(
    element: &dyn Element,
    app: &HeadlessAimerApp<W>,
    found: &mut Vec<aimer_cupid::draw_cmd_v2::RenderNodeId>,
) {
    if element.debug_name() == "Container"
        && let Some(node) = app.app.render_node_for_element(element.id())
    {
        found.push(node);
    }
    element.visit_children(&mut |child| containers(child, app, found));
}

#[test]
fn gaining_focus_repaints_the_box_decoration() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let node = FocusNode::new();
    let mut app = AimerApp::start_headless_with(
        Boxes { node: node.clone() },
        HeadlessOptions {
            size: PhysicalSize::new(300, 300),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    let revisions = |app: &HeadlessAimerApp<_>| {
        let mut nodes = Vec::new();
        containers(
            app.app.widget_root.as_ref().unwrap().as_ref(),
            app,
            &mut nodes,
        );
        nodes
            .iter()
            .map(|n| app.app.render_tree().draw_list_revision(*n).unwrap())
            .collect::<Vec<_>>()
    };
    let before = revisions(&app);
    assert!(
        !before.is_empty(),
        "the decorated container has a render node"
    );

    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(20.0, 20.0),
    });
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: winit::event::MouseButton::Left,
        });
    }
    app.pump_frames(5);

    assert!(node.has_focus());
    let after = revisions(&app);
    assert_ne!(
        before, after,
        "focus must re-record the decoration: {before:?} vs {after:?}"
    );
}
