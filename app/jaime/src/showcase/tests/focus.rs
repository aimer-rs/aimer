//! The FocusNode example, mounted inside the real showcase shell, gives a box
//! the keyboard when it is clicked.

use super::*;
use std::cell::RefCell;
use std::rc::Rc;

use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use aimer::{
    AnyElement, Drawable, Element, EventElement, HeadlessOptions, LayoutElement, Rebuildable,
    ResolvedSize, Vec2d, VisitorElement,
};

type Targets = Rc<RefCell<Vec<(FocusNode, Vec2d, Vec2d)>>>;

struct Captured {
    targets: Targets,
}

impl Widget for Captured {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let state = ExampleShowcaseState::with_selected(ExampleId::FocusNode);
        CapturedElement {
            child: state
                .build(ctx)
                .to_element(ctx),
            targets: self.targets,
        }
        .boxed()
    }
}
impl aimer::PortableWidget for Captured {}

struct CapturedElement {
    child: AnyElement,
    targets: Targets,
}
impl VisitorElement for CapturedElement {
    fn debug_name(&self) -> &'static str {
        "CapturedFocus"
    }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}
impl EventElement for CapturedElement {}
impl Rebuildable for CapturedElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child
            .rebuild_if_dirty(ctx);
    }
}
impl LayoutElement for CapturedElement {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child
            .computed_size(ctx)
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child
            .content_size(ctx)
    }
}
impl Drawable for CapturedElement {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }
    fn paint_local_v2(&self, ctx: &BuildContext) {
        aimer::canvas::Canvas::of(ctx).finish();
    }
    fn update(&self, ctx: &BuildContext) {
        self.child
            .update(ctx);
        let mut found = Vec::new();
        collect(self.child.as_ref(), &mut found);
        self.targets
            .replace(found);
    }
}

fn collect(element: &dyn Element, found: &mut Vec<(FocusNode, Vec2d, Vec2d)>) {
    if let (Some(node), Some((start, end))) = (element.focus_node(), element.pos_start_end()) {
        found.push((node.clone(), start, end));
    }
    element.visit_children(&mut |child| collect(child, found));
}

fn click<W: Widget + 'static>(
    app: &mut aimer::quiver::aimer_app::HeadlessAimerApp<W>,
    x: f64,
    y: f64,
) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    app.render_frame();
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
        app.render_frame();
    }
}

#[test]
fn clicking_a_focus_example_box_focuses_it_inside_the_showcase() {
    for scale in [1.0, 2.0] {
        let targets: Targets = Rc::default();
        let mut app = AimerApp::start_headless_with(
            theme::provide(Captured {
                targets: targets.clone(),
            }),
            HeadlessOptions {
                size: PhysicalSize::new((1280.0 * scale) as u32, (800.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        let found = targets
            .borrow()
            .clone();
        assert!(found.len() >= 2, "the example mounts two focus boxes");

        for (index, (node, start, end)) in found
            .iter()
            .enumerate()
        {
            let (x, y) = (
                (start.x + end.x) as f64 / 2.0,
                (start.y + end.y) as f64 / 2.0,
            );
            click(&mut app, x * scale, y * scale);
            app.pump_frames(3);
            assert!(
                node.has_focus(),
                "scale {scale}: box {index} must own the keyboard"
            );
            for (other, (node, _, _)) in found
                .iter()
                .enumerate()
            {
                assert_eq!(
                    node.has_focus(),
                    other == index,
                    "only the clicked box owns it"
                );
            }
        }
    }
}
