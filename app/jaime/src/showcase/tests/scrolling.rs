use super::*;
use std::cell::RefCell;
use std::rc::Rc;

use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};
use aimer::{
    AnyElement, Drawable, Element, EventElement, HeadlessOptions, LayoutElement, Rebuildable,
    ResolvedSize, Vec2d, VisitorElement,
};

struct CapturedShowcase {
    selected: ExampleId,
    positions: Rc<RefCell<Vec<Vec2d>>>,
}

impl Widget for CapturedShowcase {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let state = ExampleShowcaseState::with_selected(self.selected);
        CapturedShowcaseElement {
            child: state
                .build(ctx)
                .to_element(ctx),
            positions: self.positions,
        }
        .boxed()
    }
}
impl aimer::PortableWidget for CapturedShowcase {}

struct CapturedShowcaseElement {
    child: AnyElement,
    positions: Rc<RefCell<Vec<Vec2d>>>,
}
impl VisitorElement for CapturedShowcaseElement {
    fn debug_name(&self) -> &'static str {
        "CapturedShowcase"
    }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}
impl EventElement for CapturedShowcaseElement {}
impl Rebuildable for CapturedShowcaseElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child
            .rebuild_if_dirty(ctx);
    }
}
impl LayoutElement for CapturedShowcaseElement {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child
            .computed_size(ctx)
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child
            .content_size(ctx)
    }
}
impl Drawable for CapturedShowcaseElement {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }
    fn paint_local_v2(&self, ctx: &BuildContext) {
        aimer::canvas::Canvas::of(ctx).finish();
    }
    fn update(&self, ctx: &BuildContext) {
        self.child
            .update(ctx);
        self.positions
            .replace(scroll_positions(self.child.as_ref()));
    }
}

fn scroll_positions(root: &dyn Element) -> Vec<Vec2d> {
    fn first_position(element: &dyn Element) -> Option<Vec2d> {
        if let Some(bounds) = element.pos_start_end() {
            return Some(bounds.0);
        }
        let mut position = None;
        element.visit_children(&mut |child| {
            if position.is_none() {
                position = first_position(child);
            }
        });
        position
    }
    fn visit(element: &dyn Element, positions: &mut Vec<Vec2d>) {
        if element.debug_name() == "RawScrollableContainer" {
            let mut child_position = None;
            element.visit_children(&mut |child| {
                if child_position.is_none() {
                    child_position = first_position(child);
                }
            });
            positions.push(child_position.expect("scroll content has bounds"));
        }
        element.visit_children(&mut |child| visit(child, positions));
    }
    let mut positions = Vec::new();
    visit(root, &mut positions);
    positions
}

#[test]
fn wheel_reaches_the_right_panel_page_scrollable() {
    for scale_factor in [1.0, 2.0] {
        check_panel_scrolling(scale_factor);
    }
}

fn check_panel_scrolling(scale_factor: f64) {
    let positions = Rc::new(RefCell::new(Vec::new()));
    let mut app = AimerApp::start_headless_with(
        theme::provide(CapturedShowcase {
            selected: ExampleId::ChoiceControls,
            positions: positions.clone(),
        }),
        HeadlessOptions {
            size: PhysicalSize::new(1280, 800),
            scale_factor,
        },
    );
    app.render_frame();
    let before = positions
        .borrow()
        .clone();
    let right = before
        .iter()
        .position(|content| content.x > SIDEBAR_WIDTH)
        .unwrap();
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(900.0, 400.0),
    });
    app.render_frame();
    for _ in 0..4 {
        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -60.0)),
            phase: TouchPhase::Moved,
        });
        app.render_frame();
    }
    let after = positions
        .borrow()
        .clone();
    assert!(
        after[right].y < before[right].y,
        "right-panel content must scroll: before={before:?}, after={after:?}"
    );
    assert_eq!(
        after[0], before[0],
        "scrolling the right panel must leave the sidebar fixed"
    );

    app.send_window_event(WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 0.0)),
        phase: TouchPhase::Cancelled,
    });
    app.render_frame();
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(120.0, 400.0),
    });
    app.render_frame();
    // The wheel can carry the right panel past its end, and its spring back
    // runs on the wall clock. How far it goes depends on the host's fonts and
    // how slowly frames render, so let it come to rest: what is checked next is
    // the sidebar's wheel input, not the tail of the previous gesture.
    let mut frames = 0;
    while app.take_redraw_request() && frames < 600 {
        app.render_frame();
        frames += 1;
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(frames < 600, "the right panel never came to rest");
    let before = positions
        .borrow()
        .clone();
    for _ in 0..4 {
        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -60.0)),
            phase: TouchPhase::Moved,
        });
        app.render_frame();
    }
    let after = positions
        .borrow()
        .clone();
    assert!(
        after[0].y < before[0].y,
        "the sidebar must continue scrolling: before={before:?}, after={after:?}"
    );
    assert_eq!(
        after[right], before[right],
        "scrolling the sidebar must leave the right panel fixed"
    );
}
