use std::cell::Cell;
use std::rc::Rc;

use aimer::events::element::ElementEvent;
use aimer::input::mouse_region::MouseRegion;
use aimer::quiver::winit::dpi::PhysicalPosition;
use aimer::quiver::winit::event::{DeviceId, WindowEvent};
use aimer::{
    AimerApp, AnyElement, AnyWidget, BuildContext, Column, Drawable, EventElement, EventResult,
    EventTreeRole, LayoutElement, Rebuildable, ScrollAxis, ScrollController, Scrollable, SizedBox,
    Vec2d, VisitorElement, Widget,
};

struct PointerMoveProbe {
    moves: Rc<Cell<usize>>,
}

impl Widget for PointerMoveProbe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        aimer::Element::boxed(self)
    }
}

impl VisitorElement for PointerMoveProbe {
    fn debug_name(&self) -> &'static str {
        "PointerMoveProbe"
    }
}

impl EventElement for PointerMoveProbe {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if matches!(event, ElementEvent::PointerMove(_)) {
            self.moves.set(self.moves.get() + 1);
        }
        EventResult::ignored()
    }
}

impl LayoutElement for PointerMoveProbe {}
impl Drawable for PointerMoveProbe {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for PointerMoveProbe {}

#[test]
fn scrolling_reconciles_stationary_mouse_hover_through_the_child_dispatcher() {
    let moves = Rc::new(Cell::new(0));
    let enters = Rc::new(Cell::new(0));
    let exits = Rc::new(Cell::new(0));
    let controller = ScrollController::new();
    let rows = (0..10)
        .map(|_| {
            let enters = enters.clone();
            let exits = exits.clone();
            MouseRegion::new()
                .on_hover_enter(move || enters.set(enters.get() + 1))
                .on_hover_exit(move || exits.set(exits.get() + 1))
                .child(
                    SizedBox::new()
                        .width(200)
                        .height(40)
                        .child(PointerMoveProbe {
                            moves: moves.clone(),
                        }),
                )
                .boxed()
        })
        .collect::<Vec<AnyWidget>>();
    let page = SizedBox::new().width(200).height(120).child(
        Scrollable::new()
            .axis(ScrollAxis::Vertical)
            .controller(controller.clone())
            .child(Column::new().children(rows)),
    );
    let mut app = AimerApp::start_headless(page);
    app.render_frame();
    assert!(
        controller.max_extent().y > 0.0,
        "the page must be scrollable"
    );

    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(20.0, 20.0),
    });
    app.render_frame();
    let moves_before_scroll = moves.get();
    assert!(moves_before_scroll > 0);
    assert_eq!(enters.get(), 1);

    controller.jump_to(Vec2d { x: 0.0, y: 24.0 });
    app.render_frame();

    assert_ne!(
        controller.offset().y,
        0.0,
        "the wheel must move the content"
    );
    assert!(
        moves.get() > moves_before_scroll,
        "scroll movement should reconcile the child pointer hit path"
    );
    assert_eq!(enters.get(), 2);
    assert_eq!(exits.get(), 1);
}
