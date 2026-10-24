use super::*;

use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect};
use aimer_cupid::material::{MaterialKind, MaterialRequest};
use aimer::quiver::winit::dpi::PhysicalPosition;
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use aimer::{
    Drawable, Element, EventElement, HeadlessOptions, LayoutElement, Rebuildable, VisitorElement,
};
use aimer_widget::components::element::ElementId;
use aimer_widget::testing::RecordedPaint;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Snapshot {
    paints: Vec<RecordedPaint>,
    sliders: Vec<Rect>,
    text_ids: Vec<ElementId>,
}

struct Captured(Rc<RefCell<Snapshot>>);
impl aimer::PortableWidget for Captured {}
impl Widget for Captured {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let example = EXAMPLES
            .iter()
            .copied()
            .find(|example| example.key() == "light-edge")
            .expect("Light edge must be selectable in Jaime");
        CapturedElement {
            child: build_example(example, theme::app_theme()).to_element(ctx),
            snapshot: self.0,
        }
        .boxed()
    }
}

struct CapturedElement {
    child: AnyElement,
    snapshot: Rc<RefCell<Snapshot>>,
}
impl VisitorElement for CapturedElement {
    fn debug_name(&self) -> &'static str {
        "CapturedLightEdge"
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
        let paints = aimer_widget::testing::record_retained_paint(self.child.as_ref(), ctx);
        let mut sliders = Vec::new();
        let mut text_ids = Vec::new();
        collect_elements(self.child.as_ref(), &mut sliders, &mut text_ids);
        self.snapshot
            .replace(Snapshot {
                paints,
                sliders,
                text_ids,
            });
    }
}

fn collect_elements(element: &dyn Element, sliders: &mut Vec<Rect>, text_ids: &mut Vec<ElementId>) {
    if element.debug_name() == "RawSlider" {
        let (start, end) = element
            .pos_start_end()
            .expect("visible slider interaction bounds");
        sliders.push(Rect::new(
            start.x,
            start.y,
            end.x - start.x,
            end.y - start.y,
        ));
    }
    if element.debug_name() == "RawTextWidget" {
        text_ids.push(element.id());
    }
    element.visit_children(&mut |child| collect_elements(child, sliders, text_ids));
}

fn edge(snapshot: &Snapshot) -> Option<MaterialRequest> {
    snapshot
        .paints
        .iter()
        .flat_map(|paint| {
            paint
                .commands
                .iter()
        })
        .find_map(|command| match command {
            DrawCommand::DrawCustom { data, .. } => MaterialRequest::decode(data)
                .ok()
                .filter(|request| request.kind == MaterialKind::LightEdge),
            _ => None,
        })
}

#[test]
fn light_edge_page_records_a_visible_rim_and_five_controls() {
    for scale in [1.0, 2.0] {
        let snapshot: Rc<RefCell<Snapshot>> = Rc::default();
        let mut app = AimerApp::start_headless_with(
            theme::provide(Captured(snapshot.clone())),
            HeadlessOptions {
                size: PhysicalSize::new((900.0 * scale) as u32, (800.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        let census = app
            .paint_source_census()
            .unwrap();
        assert!(
            census
                .declined_paint
                .is_empty()
        );
        assert!(
            census
                .drawn_unmapped
                .is_empty()
        );
        assert!(
            census
                .invalid_bounds
                .is_empty()
        );
        assert!(
            census
                .sync_error
                .is_none()
        );
        let snapshot = snapshot.borrow();
        let edge = edge(&snapshot).expect("the preview must record a light edge");
        assert_eq!(edge.border_width, 3.0);
        assert_eq!(edge.specular_highlight, 0.8);
        assert_eq!(edge.edge_lighting, 0.35);
        assert_eq!(edge.light_angle, 225.0);
        assert_eq!(edge.corner_radii, [24.0; 4]);
        assert_eq!(
            snapshot
                .sliders
                .len(),
            5
        );
        let commands: Vec<_> = snapshot
            .paints
            .iter()
            .flat_map(|paint| {
                paint
                    .commands
                    .iter()
            })
            .collect();
        let rim_index = commands
            .iter()
            .position(|command| {
                matches!(command,
            DrawCommand::DrawCustom { data, .. } if MaterialRequest::decode(data)
                .is_ok_and(|request| request.kind == MaterialKind::LightEdge))
            })
            .unwrap();
        assert!(commands[..rim_index].iter().any(|command| matches!(command,
            DrawCommand::FillRect { color, .. } if color.r == 24 && color.g == 42 && color.b == 76)),
            "the surface must paint before the rim");
        assert!(
            commands[rim_index + 1..]
                .iter()
                .any(|command| matches!(command,
            DrawCommand::DrawText { text, .. } if text.as_ref() == "Light edge"))
        );
        assert!(
            !commands[rim_index + 1..]
                .iter()
                .any(|command| matches!(command,
            DrawCommand::FillRect { color, .. } if color.a != 0)),
            "the child must not cover the rim"
        );
    }
}

#[test]
fn light_edge_controls_update_the_rim_and_retain_preview_text() {
    for scale in [1.0, 2.0] {
        let snapshot: Rc<RefCell<Snapshot>> = Rc::default();
        let mut app = AimerApp::start_headless_with(
            theme::provide(Captured(snapshot.clone())),
            HeadlessOptions {
                size: PhysicalSize::new((900.0 * scale) as u32, (800.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        // The two preview cards are last in the page and each owns two text
        // elements. Controls rebuild their value labels; preview text stays retained.
        let preview_ids: Vec<_> = snapshot
            .borrow()
            .text_ids
            .iter()
            .rev()
            .take(4)
            .copied()
            .collect();
        assert_eq!(preview_ids.len(), 4);
        for (index, fraction) in [
            (0, 0.75),
            (1, 0.75),
            (2, 0.5),
            (3, 0.75),
            (4, 0.0),
            (4, 0.5),
            (4, 1.0),
            (1, 0.0),
            (1, 0.5),
            (2, 0.0),
        ] {
            let bounds = snapshot
                .borrow()
                .sliders[index];
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(
                    (bounds.x + bounds.width * fraction) as f64 * scale,
                    (bounds.y + bounds.height * 0.5) as f64 * scale,
                ),
            });
            app.render_frame();
            for state in [ElementState::Pressed, ElementState::Released] {
                app.send_window_event(WindowEvent::MouseInput {
                    device_id: DeviceId::dummy(),
                    state,
                    button: MouseButton::Left,
                });
                app.pump_frames(2);
            }
            let snapshot = snapshot.borrow();
            if fraction == 0.0 && (index == 1 || index == 2) {
                assert!(
                    edge(&snapshot).is_none(),
                    "zero width or strength must remove the rim"
                );
            } else {
                let edge = edge(&snapshot).expect("visible rim after control change");
                match index {
                    0 => assert!(
                        edge.light_angle > 240.0 && edge.light_angle < 315.0,
                        "direction: {edge:?}"
                    ),
                    1 => assert!(edge.border_width > 6.0, "width: {edge:?}"),
                    2 => assert!(
                        (edge.specular_highlight - 0.5).abs() < 0.1,
                        "strength: {edge:?}"
                    ),
                    3 => assert!(edge.corner_radii[0] > 50.0, "radius: {edge:?}"),
                    _ => assert!(
                        (edge.edge_lighting - fraction).abs() < 0.1,
                        "reflection: {edge:?}"
                    ),
                }
            }
            assert_eq!(
                snapshot
                    .text_ids
                    .iter()
                    .rev()
                    .take(4)
                    .copied()
                    .collect::<Vec<_>>(),
                preview_ids,
                "slider changes must retain the preview text elements"
            );
        }
    }
}
