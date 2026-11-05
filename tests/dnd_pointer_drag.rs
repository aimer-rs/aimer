//! A card is picked up with a real mouse drag and dropped on a target.
//!
//! These drive window events through a headless app, so the press, the moves
//! and the release all go through the event dispatcher and the retained tree —
//! the path `Draggable` unit tests, which call `on_event` on the element
//! directly, never touch.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer::dnd::{DragSession, DragTarget, DragTargetState, Draggable};
use aimer::quiver::aimer_app::HeadlessAimerApp;
use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use aimer::style::*;
use aimer::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CardId(u32);

/// What the application observed, shared with the widgets under test.
#[derive(Default)]
struct Seen {
    accepted: RefCell<Vec<CardId>>,
    completed: RefCell<Vec<bool>>,
    hovered: Cell<bool>,
}

/// A 100x100 card at the top-left and a 100x100 target to its right.
fn board(seen: Rc<Seen>) -> impl Widget {
    let completed = seen.clone();
    let accepted = seen.clone();
    let hover = seen;

    let card = Draggable::new()
        .data(CardId(7))
        .feedback(|| Container::new().width(100).height(100).color(Color::Rgba(200, 60, 60, 255)).child(ZeroSizedBox))
        .on_drag_completed(move |taken| completed.completed.borrow_mut().push(taken))
        .child(Container::new().width(100).height(100).color(Color::Rgba(60, 60, 200, 255)).child(ZeroSizedBox));

    let target = DragTarget::<CardId>::new()
        .on_accept(move |id| accepted.accepted.borrow_mut().push(id))
        .child(move |state: DragTargetState| {
            hover.hovered.set(state.is_hovered);
            Container::new().width(100).height(100).color(Color::Rgba(60, 200, 60, 255)).child(ZeroSizedBox)
        });

    Row::new().children(vec![card.boxed(), target.boxed()])
}

fn move_to(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
}

fn button(app: &mut HeadlessAimerApp<impl Widget + 'static>, state: ElementState) {
    app.send_window_event(WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state,
        button: MouseButton::Left,
    });
}

#[test]
fn dragging_a_card_onto_a_target_delivers_it() {
    DragSession::cancel_any();
    let seen = Rc::new(Seen::default());
    let mut app = AimerApp::start_headless_with(
        board(seen.clone()),
        HeadlessOptions {
            size: PhysicalSize::new(400, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);

    move_to(&mut app, 50.0, 50.0);
    button(&mut app, ElementState::Pressed);
    app.render_frame();

    for step in 1..=8 {
        move_to(&mut app, 50.0 + 12.0 * step as f64, 50.0);
        app.render_frame();
    }
    assert!(DragSession::is_active(), "the card was never picked up");

    move_to(&mut app, 150.0, 50.0);
    app.render_frame();
    assert!(seen.hovered.get(), "the target never saw the drag arrive");

    button(&mut app, ElementState::Released);
    app.pump_frames(2);

    assert_eq!(*seen.accepted.borrow(), vec![CardId(7)]);
    assert_eq!(*seen.completed.borrow(), vec![true]);
    assert!(!DragSession::is_active(), "the session outlived the drop");
}

/// Two columns like the kanban board: the card lives *inside* the first
/// column's `DragTarget`, so the first target the drag reaches is the one that
/// owns the card being carried, and its hover rebuild replaces that card.
fn columns(seen: Rc<Seen>) -> impl Widget {
    let completed = seen.clone();
    let accepted = seen.clone();

    let source = DragTarget::<CardId>::new().child(move |_state: DragTargetState| {
        let completed = completed.clone();
        Container::new()
            .width(200)
            .height(200)
            .child(
                Draggable::new()
                    .data(CardId(7))
                    .feedback(|| {
                        Container::new()
                            .width(100)
                            .height(50)
                            .color(Color::Rgba(200, 60, 60, 255))
                            .child(ZeroSizedBox)
                    })
                    .on_drag_completed(move |taken| completed.completed.borrow_mut().push(taken))
                    .child(
                        Container::new()
                            .width(100)
                            .height(50)
                            .color(Color::Rgba(60, 60, 200, 255))
                            .child(ZeroSizedBox),
                    ),
            )
    });

    let destination = DragTarget::<CardId>::new()
        .on_accept(move |id| accepted.accepted.borrow_mut().push(id))
        .child(|_state: DragTargetState| {
            Container::new()
                .width(200)
                .height(200)
                .color(Color::Rgba(60, 200, 60, 255))
                .child(ZeroSizedBox)
        });

    Row::new().children(vec![source.boxed(), destination.boxed()])
}

#[test]
fn a_card_inside_a_column_can_be_dragged_to_another_column() {
    DragSession::cancel_any();
    let seen = Rc::new(Seen::default());
    let mut app = AimerApp::start_headless_with(
        columns(seen.clone()),
        HeadlessOptions {
            size: PhysicalSize::new(500, 300),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);

    move_to(&mut app, 40.0, 20.0);
    button(&mut app, ElementState::Pressed);
    app.render_frame();

    for step in 1..=8 {
        move_to(&mut app, 40.0 + 30.0 * step as f64, 20.0);
        app.render_frame();
    }
    assert!(DragSession::is_active(), "the card was never picked up");

    button(&mut app, ElementState::Released);
    app.pump_frames(2);

    assert_eq!(*seen.accepted.borrow(), vec![CardId(7)]);
    assert_eq!(*seen.completed.borrow(), vec![true]);
    assert!(!DragSession::is_active(), "the session outlived the drop");
}

/// The same two columns, but under a `Scrollable`, as every page of the
/// showcase is. A scroller is a forwarding boundary: the card's pointer capture
/// is recorded for the boundary rather than for the window, and the moves that
/// follow reach the card without passing through the hover bookkeeping that
/// found it under the pointer before the press. The dispatcher then used to
/// report the card as exited, which `Draggable` took as a lost pointer.
#[test]
fn a_card_under_a_scrollable_survives_the_hover_bookkeeping() {
    DragSession::cancel_any();
    let seen = Rc::new(Seen::default());
    let mut app = AimerApp::start_headless_with(
        Scrollable::new().child(columns(seen.clone())),
        HeadlessOptions {
            size: PhysicalSize::new(500, 300),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);

    // Hover first, as a person does: the card must already be in the hover chain.
    move_to(&mut app, 38.0, 18.0);
    app.render_frame();
    move_to(&mut app, 40.0, 20.0);
    app.render_frame();
    button(&mut app, ElementState::Pressed);
    app.render_frame();

    for step in 1..=8 {
        move_to(&mut app, 40.0 + 30.0 * step as f64, 20.0);
        app.render_frame();
    }
    assert!(DragSession::is_active(), "the drag ended before the button was released");

    button(&mut app, ElementState::Released);
    app.pump_frames(2);

    assert_eq!(*seen.accepted.borrow(), vec![CardId(7)]);
    assert_eq!(*seen.completed.borrow(), vec![true]);
    assert!(!DragSession::is_active(), "the session outlived the drop");
}

/// The top-left corner, in logical window pixels, of the node drawn with the
/// given size, after its own transform. `None` if nothing of that size is drawn.
fn drawn_corner(
    app: &HeadlessAimerApp<impl Widget + 'static>,
    width: f32,
    height: f32,
) -> Option<(f32, f32)> {
    use aimer_cupid::draw_cmd::RenderOp;

    // Parents are drawn before their children, and the hosted follower is the
    // same size as the feedback it carries, so the last match is the feedback
    // itself: the node the presentation is applied to.
    app.render_ops().into_iter().filter_map(|operation| match operation {
        RenderOp::Draw(item) if item.bounds.width == width && item.bounds.height == height => {
            // A presentation that only moves a node is a translation, which sits
            // in the last column of the cumulative transform.
            let [.., translation] = item.transform.cols;
            Some((item.origin.0 + translation[0], item.origin.1 + translation[1]))
        }
        _ => None,
    }).last()
}

/// While a card is carried, the feedback is where the pointer is, on the frame
/// that follows each move, not whenever something else happens to repaint.
///
/// The feedback is a node hosted above the window, and the render tree only
/// asks a node where it is when the tree changes shape. A pointer move changes
/// nothing of the kind, so the card stayed where it was first placed and then
/// jumped each time an unrelated change forced a synchronization.
#[test]
fn the_feedback_is_under_the_pointer_after_every_move() {
    DragSession::cancel_any();
    let seen = Rc::new(Seen::default());
    let completed = seen.clone();
    let card = Draggable::new()
        .data(CardId(7))
        // A size nothing else on screen has, so it can be found among the ops.
        .feedback(|| {
            Container::new()
                .width(61)
                .height(37)
                .color(Color::Rgba(200, 60, 60, 255))
                .child(ZeroSizedBox)
        })
        .on_drag_completed(move |taken| completed.completed.borrow_mut().push(taken))
        .child(
            Container::new()
                .width(100)
                .height(100)
                .color(Color::Rgba(60, 60, 200, 255))
                .child(ZeroSizedBox),
        );
    let mut app = AimerApp::start_headless_with(
        Row::new().children(vec![card.boxed()]),
        HeadlessOptions {
            size: PhysicalSize::new(500, 300),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);

    // Pressed 50 px in from the card's corner, so the feedback's corner should
    // trail the pointer by exactly that.
    move_to(&mut app, 50.0, 50.0);
    button(&mut app, ElementState::Pressed);
    app.render_frame();

    let mut worst: f32 = 0.0;
    for step in 1..=20 {
        let pointer = (50.0 + 15.0 * step as f64, 50.0 + 7.0 * step as f64);
        move_to(&mut app, pointer.0, pointer.1);
        app.render_frame();

        // The feedback is built on the first frame that paints it, so it is
        // allowed to be missing from the first couple of moves.
        let Some((x, y)) = drawn_corner(&app, 61.0, 37.0) else {
            assert!(step <= 3, "step {step}: the feedback is not drawn");
            continue;
        };
        let expected = ((pointer.0 - 50.0) as f32, (pointer.1 - 50.0) as f32);
        worst = worst.max((x - expected.0).abs()).max((y - expected.1).abs());
        assert!(
            (x - expected.0).abs() < 0.5 && (y - expected.1).abs() < 0.5,
            "step {step}: the feedback is at ({x}, {y}), the pointer says ({}, {})",
            expected.0,
            expected.1
        );
    }
    assert!(worst < 0.5);

    button(&mut app, ElementState::Released);
    app.pump_frames(3);
    DragSession::cancel_any();
}

// ---------------------------------------------------------------------------
// Dropping at a position in a list.
// ---------------------------------------------------------------------------

use aimer::dnd::{InsertionList, InsertionState, ListAxis};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NewItem(u32);

/// What the list reported, shared with the callbacks.
#[derive(Default)]
struct Inserted {
    at: RefCell<Vec<(usize, NewItem)>>,
}

/// A 100x30 palette item on the left and, to its right, a list of three 90x40
/// rows stacked from the top. Sizes are chosen so nothing else on screen shares
/// them and each can be found among the drawn nodes.
fn palette_and_list(inserted: Rc<Inserted>, accept: bool) -> impl Widget {
    let palette = Draggable::new()
        .data(NewItem(1))
        .feedback(|| {
            Container::new()
                .width(50)
                .height(20)
                .color(Color::Rgba(200, 60, 60, 255))
                .child(ZeroSizedBox)
        })
        .child(
            Container::new()
                .width(100)
                .height(30)
                .color(Color::Rgba(60, 60, 200, 255))
                .child(ZeroSizedBox),
        );

    let list = InsertionList::<NewItem>::new()
        .axis(ListAxis::Vertical)
        .will_accept(move |_| accept)
        .on_insert(move |index, item| inserted.at.borrow_mut().push((index, item)))
        .indicator(|| {
            Container::new()
                .width(77)
                .height(5)
                .color(Color::Rgba(60, 200, 60, 255))
                .child(ZeroSizedBox)
        })
        .items(|_state: InsertionState| {
            (0..3)
                .map(|_| {
                    Container::new()
                        .width(90)
                        .height(40)
                        .color(Color::Rgba(90, 90, 90, 255))
                        .child(ZeroSizedBox)
                        .boxed()
                })
                .collect::<Vec<_>>()
        });

    Row::new().children(vec![palette.boxed(), list.boxed()])
}

/// Picks the palette item up and carries it over the list at height `y`,
/// returning the app with the button still held.
fn carry_over_list(
    inserted: Rc<Inserted>,
    accept: bool,
    y: f64,
) -> (HeadlessAimerApp<impl Widget + 'static>, f64) {
    DragSession::cancel_any();
    let mut app = AimerApp::start_headless_with(
        palette_and_list(inserted, accept),
        HeadlessOptions {
            size: PhysicalSize::new(500, 300),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    let (list_x, _) = drawn_corner(&app, 90.0, 40.0).expect("the list is drawn");

    move_to(&mut app, 50.0, 15.0);
    button(&mut app, ElementState::Pressed);
    app.render_frame();
    let x = f64::from(list_x) + 45.0;
    for step in 1..=6 {
        let t = f64::from(step) / 6.0;
        move_to(&mut app, 50.0 + (x - 50.0) * t, 15.0 + (y - 15.0) * t);
        app.render_frame();
    }
    (app, x)
}

/// Where a pointer at each height lands among three 40 px rows: before the row
/// whose middle it has not yet passed.
#[test]
fn a_drop_lands_in_the_slot_under_the_pointer() {
    for (y, slot) in [(10.0, 0), (50.0, 1), (80.0, 2), (110.0, 3)] {
        let inserted = Rc::new(Inserted::default());
        let (mut app, _) = carry_over_list(inserted.clone(), true, y);

        button(&mut app, ElementState::Released);
        app.pump_frames(3);

        assert_eq!(
            *inserted.at.borrow(),
            vec![(slot, NewItem(1))],
            "released at y = {y}"
        );
        assert!(!DragSession::is_active());
    }
}

/// The list opens a gap where the item will go, so the user sees the slot
/// before letting go.
#[test]
fn the_list_shows_where_the_item_will_go() {
    let inserted = Rc::new(Inserted::default());
    let (mut app, _) = carry_over_list(inserted.clone(), true, 80.0);
    app.render_frame();

    let (_, indicator_y) = drawn_corner(&app, 77.0, 5.0).expect("the slot is marked");
    assert!(
        (indicator_y - 80.0).abs() < 0.5,
        "the marker sits between the second and third row, not at {indicator_y}"
    );
    // The rows are the only 90x40 nodes; the last of them is the third row.
    let third_row_y = {
        use aimer_cupid::draw_cmd::RenderOp;
        app.render_ops()
            .into_iter()
            .filter_map(|op| match op {
                RenderOp::Draw(item) if item.bounds.width == 90.0 && item.bounds.height == 40.0 => {
                    Some(item.origin.1)
                }
                _ => None,
            })
            .last()
            .expect("three rows")
    };
    assert!(
        third_row_y >= 85.0 - 0.5,
        "the rows after the slot make room for the marker: {third_row_y}"
    );

    button(&mut app, ElementState::Released);
    app.pump_frames(3);
    assert!(
        drawn_corner(&app, 77.0, 5.0).is_none(),
        "the marker goes with the drag"
    );
}

#[test]
fn a_refused_payload_gets_no_slot_and_no_marker() {
    let inserted = Rc::new(Inserted::default());
    let (mut app, _) = carry_over_list(inserted.clone(), false, 80.0);
    app.render_frame();

    assert!(drawn_corner(&app, 77.0, 5.0).is_none(), "nothing is offered");

    button(&mut app, ElementState::Released);
    app.pump_frames(3);
    assert!(inserted.at.borrow().is_empty(), "nothing was inserted");
}

/// A column's border and heading depend on the drag, and wrap the list. Only one
/// target is hovered at a time, so the list hands the drag state to a frame
/// around it instead of asking for a second target.
#[test]
fn the_frame_around_a_list_sees_the_drag() {
    DragSession::cancel_any();
    let inserted = Rc::new(Inserted::default());
    let palette = Draggable::new()
        .data(NewItem(1))
        .child(
            Container::new()
                .width(100)
                .height(30)
                .color(Color::Rgba(60, 60, 200, 255))
                .child(ZeroSizedBox),
        );
    let recorder = inserted.clone();
    let list = InsertionList::<NewItem>::new()
        .on_insert(move |index, item| recorder.at.borrow_mut().push((index, item)))
        .frame(|state: InsertionState, list| {
            // 131 wide while a drag is over it, 130 otherwise.
            Container::new()
                .width(if state.is_hovered { 131 } else { 130 })
                .height(150)
                .child(list)
        })
        .items(|_| {
            (0..3).map(|_| {
                Container::new()
                    .width(90)
                    .height(40)
                    .color(Color::Rgba(90, 90, 90, 255))
                    .child(ZeroSizedBox)
                    .boxed()
            })
        });
    let mut app = AimerApp::start_headless_with(
        Row::new().children(vec![palette.boxed(), list.boxed()]),
        HeadlessOptions {
            size: PhysicalSize::new(500, 300),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    assert!(drawn_corner(&app, 130.0, 150.0).is_some(), "the idle frame is drawn");
    assert!(drawn_corner(&app, 131.0, 150.0).is_none());

    move_to(&mut app, 50.0, 15.0);
    button(&mut app, ElementState::Pressed);
    app.render_frame();
    for step in 1..=6 {
        let t = f64::from(step) / 6.0;
        move_to(&mut app, 50.0 + 100.0 * t, 15.0 + 35.0 * t);
        app.render_frame();
    }
    app.render_frame();

    assert!(
        drawn_corner(&app, 131.0, 150.0).is_some(),
        "the frame was rebuilt with the drag over it"
    );

    button(&mut app, ElementState::Released);
    app.pump_frames(3);
    assert_eq!(*inserted.at.borrow(), vec![(1, NewItem(1))]);
    assert!(drawn_corner(&app, 130.0, 150.0).is_some(), "the frame is idle again");
}

/// Reordering inside the list: the item being carried is itself one of the
/// list's children, and the marker opens a gap in front of it as the pointer
/// moves up past its neighbours.
///
/// Rows are reconciled by position, so a marker that shifts the rows after it
/// would hand the carried row's drag to whichever row now sits in its place.
#[test]
fn an_item_of_the_list_can_be_carried_up_the_list() {
    DragSession::cancel_any();
    let inserted = Rc::new(Inserted::default());
    let completed = Rc::new(RefCell::new(Vec::new()));
    let recorder = inserted.clone();
    let done = completed.clone();

    // Three 90x40 rows, each a draggable carrying its own number.
    let list = InsertionList::<usize>::new()
        .on_insert(move |slot, row| recorder.at.borrow_mut().push((slot, NewItem(row as u32))))
        .items(move |_state: InsertionState| {
            let done = done.clone();
            (0..3usize).map(move |row| {
                let done = done.clone();
                Draggable::new()
                    .data(row)
                    .feedback(|| {
                        Container::new()
                            .width(50)
                            .height(20)
                            .color(Color::Rgba(200, 60, 60, 255))
                            .child(ZeroSizedBox)
                    })
                    .on_drag_completed(move |taken| done.borrow_mut().push((row, taken)))
                    .child(
                        Container::new()
                            .width(90)
                            .height(40)
                            .color(Color::Rgba(90, 90, 90, 255))
                            .child(ZeroSizedBox),
                    )
                    .boxed()
            })
        });
    let mut app = AimerApp::start_headless_with(
        Row::new().children(vec![list.boxed()]),
        HeadlessOptions {
            size: PhysicalSize::new(500, 300),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);

    // Press the third row, then climb past the other two to the top.
    move_to(&mut app, 45.0, 100.0);
    button(&mut app, ElementState::Pressed);
    app.render_frame();
    for y in [90.0, 75.0, 60.0, 45.0, 30.0, 15.0, 8.0] {
        move_to(&mut app, 45.0, y);
        app.render_frame();
        assert!(DragSession::is_active(), "the drag ended at y = {y}");
    }

    button(&mut app, ElementState::Released);
    app.pump_frames(3);

    assert_eq!(*inserted.at.borrow(), vec![(0, NewItem(2))]);
    assert_eq!(*completed.borrow(), vec![(2, true)]);
    assert!(!DragSession::is_active(), "the session outlived the drop");
}

#[derive(Clone, Debug, PartialEq)]
struct Greeting(&'static str);

thread_local! {
    static BANNER_SAW: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// The drag feedback: reads what the application provides.
#[derive(Clone, StatelessWidget)]
struct Banner;

impl StatelessWidget for Banner {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let greeting = ctx
            .try_watch::<Greeting>()
            .map_or("no provider in scope", |greeting| greeting.0);
        BANNER_SAW.with(|saw| saw.borrow_mut().push(greeting));
        Container::new().width(100).height(100).child(ZeroSizedBox)
    }
}

/// The overlay hosts the feedback of a drag, outside of what the application
/// provides. The feedback is still the card's, so it reads and follows the
/// providers the card does.
#[test]
fn the_feedback_of_a_drag_reads_and_follows_the_providers_of_its_card() {
    DragSession::cancel_any();
    BANNER_SAW.with(|saw| saw.borrow_mut().clear());
    let greeting = ProviderHandle::new(Greeting("hello"));
    let card = Draggable::new()
        .data(CardId(7))
        .feedback(|| Banner)
        .child(Container::new().width(100).height(100).child(ZeroSizedBox));
    let mut app = AimerApp::start_headless_with(
        Provider::new()
            .handle(greeting.clone())
            .child(Row::new().children(vec![card.boxed()])),
        HeadlessOptions {
            size: PhysicalSize::new(400, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);

    move_to(&mut app, 50.0, 50.0);
    button(&mut app, ElementState::Pressed);
    app.render_frame();
    for step in 1..=8 {
        move_to(&mut app, 50.0 + 12.0 * step as f64, 50.0);
        app.render_frame();
    }
    assert!(DragSession::is_active(), "the card was never picked up");
    app.pump_frames(2);
    assert_eq!(
        BANNER_SAW.with(|saw| saw.borrow().last().copied()),
        Some("hello"),
        "the feedback should find the provider its card is under"
    );

    greeting.update(|value| *value = Greeting("goodbye"));
    app.pump_frames(3);
    assert_eq!(
        BANNER_SAW.with(|saw| saw.borrow().last().copied()),
        Some("goodbye"),
        "and follow it"
    );

    button(&mut app, ElementState::Released);
    app.pump_frames(3);
    DragSession::cancel_any();
}
