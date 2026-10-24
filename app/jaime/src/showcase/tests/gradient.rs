use super::*;

use aimer_cupid::draw_cmd_v2::{COVERAGE_MASK_PIPELINE_NAME, DrawCommand};
use aimer_cupid::material::{GradientShape, MaterialKind, MaterialRequest};
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::{
    Drawable, Element, EventElement, HeadlessOptions, LayoutElement, Rebuildable, VisitorElement,
};
use aimer_widget::testing::RecordedPaint;
use std::cell::RefCell;
use std::rc::Rc;

struct Captured(Rc<RefCell<Vec<RecordedPaint>>>);
impl aimer::PortableWidget for Captured {}
impl Widget for Captured {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let example = EXAMPLES
            .iter()
            .copied()
            .find(|example| example.key() == "gradient")
            .expect("Gradients must be selectable in Jaime");
        CapturedElement {
            child: build_example(example, theme::app_theme()).to_element(ctx),
            paints: self.0,
        }
        .boxed()
    }
}

struct CapturedElement {
    child: AnyElement,
    paints: Rc<RefCell<Vec<RecordedPaint>>>,
}
impl VisitorElement for CapturedElement {
    fn debug_name(&self) -> &'static str {
        "CapturedGradient"
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
        self.paints
            .replace(aimer_widget::testing::record_retained_paint(self.child.as_ref(), ctx));
    }
}

fn materials(paint: &RecordedPaint) -> Vec<MaterialRequest> {
    paint
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawCustom { data, .. } => MaterialRequest::decode(data).ok(),
            _ => None,
        })
        .collect()
}

#[test]
fn gradient_page_paints_each_shape_and_places_decorations_behind_over_and_through() {
    for scale in [1.0, 2.0] {
        let paints: Rc<RefCell<Vec<RecordedPaint>>> = Rc::default();
        let mut app = AimerApp::start_headless_with(
            theme::provide(Captured(paints.clone())),
            HeadlessOptions {
                size: PhysicalSize::new((900.0 * scale) as u32, (900.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        let paints = paints.borrow();

        let shapes: Vec<GradientShape> = paints
            .iter()
            .filter(|paint| paint.element == "DecorationBox")
            .flat_map(materials)
            .filter(|request| request.kind == MaterialKind::Gradient)
            .map(|request| request.gradient.shape)
            .collect();
        assert_eq!(
            shapes,
            // The three cards, then the colour wheel behind the glass text.
            [
                GradientShape::Linear,
                GradientShape::Radial,
                GradientShape::Conic,
                GradientShape::Conic
            ],
            "scale {scale}"
        );

        let edges = |element: &str| {
            paints
                .iter()
                .filter(|paint| paint.element == element)
                .flat_map(materials)
                .filter(|request| request.kind == MaterialKind::LightEdge)
                .count()
        };
        assert_eq!(edges("DecorationBox"), 1, "the edge behind the card");
        assert_eq!(edges("ForegroundDecorationLayer"), 1, "the edge over the card");

        let masked: Vec<_> = paints
            .iter()
            .filter(|paint| paint.element == "MaskedDecorationBox")
            .collect();
        assert_eq!(masked.len(), 2, "a rainbow and a glass caption, scale {scale}");
        for paint in &masked {
            assert!(
                matches!(paint.commands.first(), Some(DrawCommand::DrawCustom { pipeline_name, .. })
                    if pipeline_name.as_ref() == COVERAGE_MASK_PIPELINE_NAME),
                "the mask marker heads every masked list"
            );
        }
        let kinds: Vec<_> = masked
            .iter()
            .flat_map(|paint| materials(paint))
            .map(|request| (request.kind, request.gradient.stop_count))
            .collect();
        assert_eq!(
            kinds,
            [(MaterialKind::Gradient, 7), (MaterialKind::BackdropFilter, 0)],
            "a seven-stop rainbow, then a backdrop filter"
        );
    }
}
