use super::*;

use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect};
use aimer_cupid::material::{MagnificationStyle as Style, MaterialKind, MaterialRequest};
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::{
    Drawable, Element, EventElement, HeadlessOptions, LayoutElement, Rebuildable, VisitorElement,
};
use std::cell::RefCell;
use std::rc::Rc;

use aimer::quiver::winit::dpi::PhysicalPosition;
use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use aimer_widget::components::element::ElementId;

#[derive(Default)]
struct Snapshot {
    draws: Vec<(ElementId, Rect, Vec<DrawCommand>)>,
    sliders: Vec<Rect>,
    targets: Vec<(String, Rect)>,
}
type Paints = Rc<RefCell<Snapshot>>;

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
        "CapturedMagnification"
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
        let mut sliders = Vec::new();
        collect_bounds(self.child.as_ref(), ctx, &mut bounds, &mut sliders);
        assert_eq!(recorded.len(), bounds.len());
        let draws = bounds
            .into_iter()
            .zip(recorded)
            .map(|((id, bounds), paint)| {
                (
                    id,
                    bounds,
                    paint
                        .commands
                        .to_vec(),
                )
            })
            .collect::<Vec<_>>();
        let mut targets = Vec::new();
        collect_targets(self.child.as_ref(), &draws, None, &mut targets);
        self.paints
            .replace(Snapshot {
                draws,
                sliders,
                targets,
            });
    }
}

// Label painters are smaller than their control and can have a different
// local origin. Click the nearest gesture ancestor's actual interaction bounds.
fn collect_targets(
    element: &dyn Element,
    draws: &[(ElementId, Rect, Vec<DrawCommand>)],
    target: Option<Rect>,
    targets: &mut Vec<(String, Rect)>,
) {
    let target = if element.debug_name() == "GestureDetector" {
        element
            .pos_start_end()
            .map(|(start, end)| Rect::new(start.x, start.y, end.x - start.x, end.y - start.y))
    } else {
        target
    };
    if let Some(target) = target {
        if let Some((_, _, commands)) = draws
            .iter()
            .find(|(id, _, _)| *id == element.id())
        {
            for command in commands {
                if let DrawCommand::DrawText { text, .. } = command {
                    targets.push((text.to_string(), target));
                }
            }
        }
    }
    element.visit_children(&mut |child| collect_targets(child, draws, target, targets));
}

// Follow retained child contexts so paint-order assertions also check actual
// layout overlap, including Positioned offsets and the device scale.
fn collect_bounds(
    element: &dyn Element,
    ctx: &BuildContext,
    bounds: &mut Vec<(ElementId, Rect)>,
    sliders: &mut Vec<Rect>,
) {
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
    let rect = Rect::new(
        origin.x / ctx.scale,
        origin.y / ctx.scale,
        size.width / ctx.scale,
        size.height / ctx.scale,
    );
    if element.debug_name() == "RawSlider" {
        let (start, end) = element
            .pos_start_end()
            .expect("slider interaction bounds");
        sliders.push(Rect::new(
            start.x,
            start.y,
            end.x - start.x,
            end.y - start.y,
        ));
    }
    if element.can_paint_local_v2(ctx) {
        bounds.push((element.id(), rect));
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
        collect_bounds(child, &child_ctx, bounds, sliders);
    });
}

#[test]
fn magnification_page_paints_overlapping_text_before_the_lens() {
    let example = EXAMPLES
        .iter()
        .copied()
        .find(|example| example.key() == "magnification")
        .expect("the magnification page must be selectable in Jaime");
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
        let snapshot = paints.borrow();
        let draws = &snapshot.draws;
        let (filter_index, (_, bounds, _)) = draws
            .iter()
            .enumerate()
            .find(|(_, (_, _, commands))| {
                commands
                    .iter()
                    .any(|command| match command {
                        DrawCommand::DrawCustom { data, .. } => {
                            MaterialRequest::decode(data.as_ref()).is_ok_and(|request| {
                                request.kind == MaterialKind::Magnification
                                    && request.magnification == 1.5
                            })
                        }
                        _ => false,
                    })
            })
            .expect("a visible box must record a magnification lens");
        assert!(bounds.width >= 300.0 && bounds.height >= 180.0);
        assert!(
            draws[..filter_index]
                .iter()
                .any(|(_, b, commands)| {
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
                .any(|(_, b, commands)| {
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
            "the box's label must paint after its lens"
        );
    }
}

fn recorded_lens(snapshot: &Snapshot) -> MaterialRequest {
    snapshot
        .draws
        .iter()
        .flat_map(|(_, _, commands)| commands)
        .find_map(|command| match command {
            DrawCommand::DrawCustom { data, .. } => MaterialRequest::decode(data)
                .ok()
                .filter(|request| request.kind == MaterialKind::Magnification),
            _ => None,
        })
        .expect("visible lens")
}

fn text_ids(snapshot: &Snapshot) -> Vec<ElementId> {
    snapshot
        .draws
        .iter()
        .filter_map(|(id, _, commands)| {
            commands
                .iter()
                .any(|command| {
                    matches!(command, DrawCommand::DrawText { text, .. }
            if text.contains("AIMER") || text.as_ref() == "This text stays sharp")
                })
                .then_some(*id)
        })
        .collect()
}

#[test]
fn magnification_sliders_update_the_lens_and_retain_static_text_elements() {
    for scale in [1.0, 2.0] {
        let paints: Paints = Rc::default();
        let mut app = AimerApp::start_headless_with(
            theme::provide(Captured {
                example: ExampleId::Magnification,
                paints: paints.clone(),
            }),
            HeadlessOptions {
                size: PhysicalSize::new((900.0 * scale) as u32, (800.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        let before = text_ids(&paints.borrow());
        assert!(before.len() >= 5);
        assert_eq!(recorded_lens(&paints.borrow()).bevel_radius, 16.0);
        for (index, fraction) in [(0, 0.75), (1, 0.75), (2, 0.75)] {
            let sliders = paints
                .borrow()
                .sliders
                .clone();
            assert_eq!(sliders.len(), 3);
            let bounds = sliders[index];
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
            let snapshot = paints.borrow();
            let lens = recorded_lens(&snapshot);
            if index == 0 {
                assert!(
                    lens.magnification > 2.5,
                    "zoom slider must update the material: {lens:?}"
                );
            } else if index == 1 {
                assert!(
                    lens.bevel_radius > 32.0,
                    "thickness slider must update the material: {lens:?}"
                );
            } else {
                assert!(
                    lens.bulge_strength > 0.5,
                    "bulge slider must update independently: {lens:?}"
                );
            }
            assert_eq!(
                text_ids(&snapshot),
                before,
                "slider rebuilds retain background and foreground text"
            );
        }
    }
}

#[test]
fn magnification_style_controls_combine_effects_axes_and_anchors() {
    // A select's options live in a popup presented by the modal host, which the
    // paint capture below does not see. The popup is anchored under its trigger
    // and its rows are one standard row tall, so an option is clicked where the
    // popup puts it.
    const POPUP_GAP: f32 = 4.0;
    let row_height = aimer::style::ThemeTokens::light()
        .density
        .target_size(32.0);
    for scale in [1.0, 2.0] {
        let paints: Paints = Rc::default();
        let mut app = AimerApp::start_headless_with(
            theme::provide(Captured {
                example: ExampleId::Magnification,
                paints: paints.clone(),
            }),
            HeadlessOptions {
                size: PhysicalSize::new((900.0 * scale) as u32, (900.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        let before = text_ids(&paints.borrow());
        let mut expected = Style::DEFAULT;
        let mut last_trigger: Option<Rect> = None;
        // (control, bits to clear, bits to set, popup row when it is an option)
        for (label, remove, add, popup_row) in [
            ("Uniform zoom", Style::UNIFORM, Style::NONE, None),
            ("Bulge", Style::NONE, Style::BULGE, None),
            ("Edge refraction", Style::REFRACTION, Style::NONE, None),
            ("Both axes", Style::NONE, Style::NONE, None),
            ("Horizontal", Style::NONE, Style::HORIZONTAL, Some(1)),
            ("Center X", Style::NONE, Style::NONE, None),
            ("Left", Style::NONE, Style::LEFT, Some(1)),
            ("Center Y", Style::NONE, Style::NONE, None),
            ("Top", Style::NONE, Style::TOP, Some(1)),
        ] {
            let bounds = match popup_row {
                Some(row) => {
                    let trigger = last_trigger.expect("an option follows its trigger");
                    Rect::new(
                        trigger.x,
                        trigger.y + trigger.height + POPUP_GAP + row as f32 * row_height,
                        trigger.width,
                        row_height,
                    )
                }
                None => paints
                    .borrow()
                    .targets
                    .iter()
                    .find_map(|(text, bounds)| (text == label).then_some(*bounds))
                    .unwrap_or_else(|| panic!("visible control: {label}")),
            };
            last_trigger = Some(bounds);
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(
                    (bounds.x + bounds.width * 0.5) as f64 * scale,
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
            expected = (expected & !remove) | add;
            let snapshot = paints.borrow();
            assert_eq!(
                recorded_lens(&snapshot).magnification_style,
                expected,
                "control {label}"
            );
            assert_eq!(
                text_ids(&snapshot),
                before,
                "style changes retain static text"
            );
        }
    }
}
