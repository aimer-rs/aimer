use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer::events::element::ElementEvent;
use aimer::input::mouse_region::MouseRegion;
use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};
use aimer::{
    AimerApp, AnyElement, AnyWidget, BuildContext, Color, Column, Container, Dimension, Drawable,
    Element, EventElement, EventResult, EventTreeRole, HeadlessOptions, LayoutElement, Rebuildable,
    ScrollAxis, ScrollController, Scrollable, SizedBox, State, StateView, StatefulWidget, Vec2d,
    VisitorElement, Widget,
};

struct PointerMoveProbe {
    moves: Rc<Cell<usize>>,
}

impl Widget for PointerMoveProbe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        aimer::Element::boxed(self)
    }
}

impl VisitorElement for PointerMoveProbe {}

impl Element for PointerMoveProbe {
    fn name(&self) -> &'static str {
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

#[derive(Clone, StatefulWidget)]
struct StatefulHoverRow {
    index: usize,
    hovered_rows: Rc<RefCell<Vec<bool>>>,
}

struct StatefulHoverRowState {
    index: usize,
    hovered: bool,
    hovered_rows: Rc<RefCell<Vec<bool>>>,
}

impl State<StatefulHoverRow> for StatefulHoverRowState {
    fn create(widget: StatefulHoverRow) -> Self {
        Self {
            index: widget.index,
            hovered: false,
            hovered_rows: widget.hovered_rows,
        }
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.index = new.index;
        self.hovered_rows = new.hovered_rows;
    }

    fn build(this: StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        let index = this.index;
        let hovered = this.hovered;
        let enter_rows = Rc::clone(&this.hovered_rows);
        let exit_rows = Rc::clone(&this.hovered_rows);
        MouseRegion::new()
            .on_hover_enter(move || {
                enter_rows.borrow_mut()[index] = true;
                this.set_state(|state| state.hovered = true);
            })
            .on_hover_exit(move || {
                exit_rows.borrow_mut()[index] = false;
                this.set_state(|state| state.hovered = false);
            })
            .child(
                Container::new()
                    .width(Dimension::Percent(100.0))
                    .height(24)
                    .color(if hovered {
                        Color::Rgb(80, 80, 80)
                    } else {
                        Color::Transparent
                    })
                    .child(SizedBox::new()),
            )
    }
}

#[test]
fn wheel_scrolling_stateful_hover_rows_exits_previous_rows() {
    let count = 40;
    let hovered_rows = Rc::new(RefCell::new(vec![false; count]));
    let controller = ScrollController::new();
    let rows: Vec<usize> = (0..count).collect();
    let hovered_in = Rc::clone(&hovered_rows);

    let page = Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .controller(controller.clone())
        .vertical_scroll_bar(None)
        .child(Column::new().list(rows).builder(move |&index| {
            StatefulHoverRow {
                index,
                hovered_rows: Rc::clone(&hovered_in),
            }
        }));

    let mut app = AimerApp::start_headless_with(
        page,
        HeadlessOptions {
            size: PhysicalSize::new(400, 400),
            scale_factor: 2.0,
        },
    );
    app.pump_frames(2);

    let device_id = DeviceId::dummy();
    // Place the cursor at logical y = 60.5 (inside row 2, avoiding exact 24px row seams).
    app.send_window_event(WindowEvent::CursorMoved {
        device_id,
        position: PhysicalPosition::new(100.0, 121.0),
    });
    app.pump_frames(2);

    let initial: Vec<usize> = hovered_rows
        .borrow()
        .iter()
        .enumerate()
        .filter_map(|(i, &h)| h.then_some(i))
        .collect();
    assert_eq!(initial, vec![2], "row 2 should start hovered");

    for step in 0..15 {
        let phase = if step == 0 {
            TouchPhase::Started
        } else {
            TouchPhase::Moved
        };
        app.send_window_event(WindowEvent::MouseWheel {
            device_id,
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -48.0)),
            phase,
        });
        app.render_frame();
        let currently_hovered: Vec<usize> = hovered_rows
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(i, &h)| h.then_some(i))
            .collect();
        assert!(
            currently_hovered.len() <= 1,
            "after scroll step {step} (offset_y={}), expected at most 1 hovered row, got {currently_hovered:?}",
            controller.offset().y
        );
    }
    assert!(
        controller.offset().y > 100.0,
        "expected scroll offset to advance, got {}",
        controller.offset().y
    );
}
