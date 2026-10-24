use super::*;

use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use aimer::{
    Drawable, Element, EventElement, HeadlessOptions, LayoutElement, Rebuildable, VisitorElement,
};
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect};
use aimer_cupid::material::{MaterialKind, MaterialRequest};
use aimer_widget::testing::RecordedPaint;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Snapshot {
    paints: Vec<RecordedPaint>,
    buttons: Vec<Rect>,
    /// In build order: blur, then magnification.
    sliders: Vec<Rect>,
}

struct Captured(Rc<RefCell<Snapshot>>);
impl aimer::PortableWidget for Captured {}
impl Widget for Captured {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let example = EXAMPLES
            .iter()
            .copied()
            .find(|example| example.key() == "foreground-decoration")
            .expect("Foreground decoration must be selectable in Jaime");
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
        "CapturedForegroundDecoration"
    }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}
impl EventElement for CapturedElement {}
impl Rebuildable for CapturedElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}
impl LayoutElement for CapturedElement {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
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
        self.child.update(ctx);
        let paints = aimer_widget::testing::record_retained_paint(self.child.as_ref(), ctx);
        let mut buttons = Vec::new();
        let mut sliders = Vec::new();
        collect_controls(self.child.as_ref(), &mut buttons, &mut sliders);
        self.snapshot.replace(Snapshot {
            paints,
            buttons,
            sliders,
        });
    }
}

fn collect_controls(element: &dyn Element, buttons: &mut Vec<Rect>, sliders: &mut Vec<Rect>) {
    let target = match element.debug_name() {
        "GestureDetector" => Some(&mut *buttons),
        "RawSlider" => Some(&mut *sliders),
        _ => None,
    };
    if let Some(target) = target {
        let (start, end) = element
            .pos_start_end()
            .expect("a visible control has interaction bounds");
        target.push(Rect::new(
            start.x,
            start.y,
            end.x - start.x,
            end.y - start.y,
        ));
    }
    element.visit_children(&mut |child| collect_controls(child, buttons, sliders));
}

/// The materials the foreground layer records, in paint order.
fn foreground_requests(snapshot: &Snapshot) -> Vec<MaterialRequest> {
    snapshot
        .paints
        .iter()
        .filter(|paint| paint.element == "ForegroundDecorationLayer")
        .flat_map(|paint| paint.commands.iter())
        .filter_map(|command| match command {
            DrawCommand::DrawCustom { data, .. } => MaterialRequest::decode(data).ok(),
            _ => None,
        })
        .collect()
}

fn foreground_kinds(snapshot: &Snapshot) -> Vec<MaterialKind> {
    foreground_requests(snapshot)
        .into_iter()
        .map(|request| request.kind)
        .collect()
}

fn click(
    app: &mut aimer::HeadlessAimerApp<impl Widget + 'static>,
    bounds: Rect,
    fraction: f32,
    scale: f64,
) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(
            f64::from(bounds.x + bounds.width * fraction) * scale,
            f64::from(bounds.y + bounds.height * 0.5) * scale,
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
    app.pump_frames(3);
}

#[test]
fn the_button_shows_and_hides_the_decoration_painted_over_the_card() {
    use MaterialKind::{BackdropFilter, Gradient, LightEdge, Magnification};
    for scale in [1.0_f64, 2.0] {
        let snapshot: Rc<RefCell<Snapshot>> = Rc::default();
        let mut app = AimerApp::start_headless_with(
            theme::provide(Captured(snapshot.clone())),
            HeadlessOptions {
                size: PhysicalSize::new((900.0 * scale) as u32, (800.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);

        let shown = [Magnification, BackdropFilter, Gradient, LightEdge];
        assert_eq!(foreground_kinds(&snapshot.borrow()), shown, "starts shown");
        {
            let snapshot = snapshot.borrow();
            let position = |element: &str| {
                snapshot.paints.iter().position(|paint| paint.element == element)
            };
            assert!(
                position("ForegroundDecorationLayer") > position("DecorationBox"),
                "the decoration paints after the card it covers"
            );
            assert_eq!((snapshot.buttons.len(), snapshot.sliders.len()), (1, 2));
        }

        let button = snapshot.borrow().buttons[0];
        click(&mut app, button, 0.25, scale);
        assert_eq!(foreground_kinds(&snapshot.borrow()), [], "hidden after one press, scale {scale}");
        // A different spot keeps this a second tap, not a double tap.
        click(&mut app, button, 0.75, scale);
        assert_eq!(foreground_kinds(&snapshot.borrow()), shown, "shown again after a second press");
    }
}

#[test]
fn the_sliders_add_remove_and_resize_the_blur_and_the_lens() {
    use MaterialKind::{BackdropFilter, Gradient, LightEdge, Magnification};
    for scale in [1.0_f64, 2.0] {
        let snapshot: Rc<RefCell<Snapshot>> = Rc::default();
        let mut app = AimerApp::start_headless_with(
            theme::provide(Captured(snapshot.clone())),
            HeadlessOptions {
                size: PhysicalSize::new((900.0 * scale) as u32, (800.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        let [blur, zoom] = snapshot.borrow().sliders[..] else {
            panic!("a blur slider and a magnification slider");
        };
        let kinds = |snapshot: &Rc<RefCell<Snapshot>>| foreground_kinds(&snapshot.borrow());

        // Both start "slightly" on: a small blur and a gentle lens.
        let initial = foreground_requests(&snapshot.borrow());
        assert!(initial[0].magnification > 1.0 && initial[0].magnification < 1.5);
        assert!(initial[1].blur_radius > 0.0 && initial[1].blur_radius <= 4.0);

        click(&mut app, blur, 0.0, scale);
        assert_eq!(kinds(&snapshot), [Magnification, Gradient, LightEdge], "no blur at its minimum");
        click(&mut app, zoom, 0.0, scale);
        assert_eq!(kinds(&snapshot), [Gradient, LightEdge], "no lens at one times");

        click(&mut app, blur, 1.0, scale);
        let requests = foreground_requests(&snapshot.borrow());
        assert_eq!(
            requests.iter().map(|request| request.kind).collect::<Vec<_>>(),
            [BackdropFilter, Gradient, LightEdge]
        );
        assert_eq!(requests[0].blur_radius, 24.0, "the slider's maximum, in logical pixels");

        click(&mut app, zoom, 1.0, scale);
        let requests = foreground_requests(&snapshot.borrow());
        assert_eq!(
            requests.iter().map(|request| request.kind).collect::<Vec<_>>(),
            [Magnification, BackdropFilter, Gradient, LightEdge],
            "the lens paints before the blur, so the blur softens its enlargement"
        );
        assert_eq!(requests[0].magnification, 2.5);
    }
}
