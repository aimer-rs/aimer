use super::*;

use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect};
use aimer_cupid::material::{MaterialKind, MaterialRequest};
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::{
    Drawable, Element, EventElement, HeadlessOptions, LayoutElement, Rebuildable, VisitorElement,
};
use std::cell::RefCell;
use std::rc::Rc;

type Paints = Rc<RefCell<Vec<(Rect, Vec<DrawCommand>)>>>;

struct Captured {
    example: ExampleId,
    paints: Paints,
}
impl Widget for Captured {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        CapturedElement {
            child: build_example(self.example, theme::app_theme()).to_element(ctx),
            paints: self.paints,
        }
        .boxed()
    }
}
impl aimer::PortableWidget for Captured {}

struct CapturedElement {
    child: AnyElement,
    paints: Paints,
}
impl VisitorElement for CapturedElement {
    fn debug_name(&self) -> &'static str {
        "CapturedBackdropFilter"
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
        let recorded = aimer_widget::testing::record_retained_paint(self.child.as_ref(), ctx);
        let mut bounds = Vec::new();
        collect_bounds(self.child.as_ref(), ctx, &mut bounds);
        assert_eq!(recorded.len(), bounds.len());
        self.paints.replace(
            bounds
                .into_iter()
                .zip(recorded)
                .map(|(bounds, paint)| {
                    (
                        bounds,
                        paint
                            .commands
                            .to_vec(),
                    )
                })
                .collect(),
        );
    }
}

// Follow retained child contexts so paint-order assertions also check actual
// layout overlap, including Positioned offsets and the device scale.
fn collect_bounds(element: &dyn Element, ctx: &BuildContext, bounds: &mut Vec<Rect>) {
    let size = element
        .retained_v2_bounds(ctx)
        .unwrap_or_else(|| element.content_size(ctx));
    let position = element
        .pos()
        .unwrap_or_default();
    let origin = Vec2d {
        x: ctx.parent_pos.x + position.x,
        y: ctx.parent_pos.y + position.y,
    };
    if element.can_paint_local_v2(ctx) {
        bounds.push(Rect::new(
            origin.x / ctx.scale,
            origin.y / ctx.scale,
            size.width / ctx.scale,
            size.height / ctx.scale,
        ));
    }
    let mut base = ctx.clone();
    base.parent_size = size;
    base.parent_pos = origin;
    let mut child_sizes = Vec::new();
    element.visit_retained_v2_children(&mut |_, child| child_sizes.push(child.content_size(ctx)));
    if child_sizes.as_slice() != [size] {
        base.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width,
            max_height: size.height,
        };
    } else {
        base.parent_size = ctx.parent_size;
    }
    element.visit_retained_v2_children(&mut |index, child| {
        let child_ctx = element
            .retained_v2_child_context_at(ctx, child, index)
            .unwrap_or_else(|| base.clone());
        collect_bounds(child, &child_ctx, bounds);
    });
}

#[test]
fn backdrop_filter_page_paints_overlapping_text_before_the_filter() {
    let example = EXAMPLES
        .iter()
        .copied()
        .find(|example| example.key() == "backdrop-filter")
        .expect("the backdrop filter page must be selectable in Jaime");
    for scale in [1.0, 2.0] {
        let paints: Paints = Rc::default();
        let mut app = AimerApp::start_headless_with(
            theme::provide(Captured {
                example,
                paints: paints.clone(),
            }),
            HeadlessOptions {
                size: PhysicalSize::new((900.0 * scale) as u32, (700.0 * scale) as u32),
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
        let draws = paints.borrow();
        let (filter_index, (bounds, _)) = draws
            .iter()
            .enumerate()
            .find(|(_, (_, commands))| {
                commands
                    .iter()
                    .any(|command| match command {
                        DrawCommand::DrawCustom { data, .. } => {
                            MaterialRequest::decode(data.as_ref()).is_ok_and(|request| {
                                request.kind == MaterialKind::BackdropFilter
                                    && request.blur_radius > 0.0
                            })
                        }
                        _ => false,
                    })
            })
            .expect("a visible box must record a backdrop filter");
        assert!(bounds.width >= 300.0 && bounds.height >= 180.0);
        assert!(
            draws[..filter_index]
                .iter()
                .any(|(b, commands)| {
                    b.x < bounds.x + bounds.width
                        && b.x + b.width > bounds.x
                        && b.y < bounds.y + bounds.height
                        && b.y + b.height > bounds.y
                        && commands
                            .iter()
                            .any(|command| {
                                matches!(command,
                    DrawCommand::DrawText { text, .. } if text.contains("AIMER"))
                            })
                }),
            "large backdrop text must paint underneath the box"
        );
        assert!(
            draws[filter_index + 1..]
                .iter()
                .any(|(b, commands)| {
                    b.x >= bounds.x
                        && b.y >= bounds.y
                        && b.x + b.width <= bounds.x + bounds.width + 0.1
                        && b.y + b.height <= bounds.y + bounds.height + 0.1
                        && commands
                            .iter()
                            .any(|command| {
                                matches!(command,
                    DrawCommand::DrawText { text, .. } if text.as_ref() == "This text stays sharp")
                            })
                }),
            "the box's label must paint after its backdrop filter"
        );
    }
}
