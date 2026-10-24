//! A field's frame — a builder, or a decoration that is not a `BoxDecoration` —
//! follows its state while the editing surface inside it stays mounted.

use super::text_field_caret::{caret_x, click, type_text};
use super::*;

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderOp};
use aimer_decoration::Decoration;
use aimer_input::input::TextFieldVisualState;
use aimer_style::BoxDecoration;
use aimer_widget::AnyWidget;
use aimer_widget::base::Colors;

const NORMAL: (u8, u8, u8) = (10, 200, 30);
const FOCUSED: (u8, u8, u8) = (200, 30, 10);

fn headless(widget: impl Widget + 'static) -> HeadlessAimerApp<impl Widget + 'static> {
    let mut app = AimerApp::start_headless_with(
        Column::new().children([widget.boxed()]),
        HeadlessOptions {
            size: PhysicalSize::new(400, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    app
}

/// The bounds of whatever painted a rectangle of exactly this color.
fn bounds_of(
    app: &HeadlessAimerApp<impl Widget + 'static>,
    color: (u8, u8, u8),
) -> Option<aimer_cupid::draw_cmd_v2::Rect> {
    let tree = app.app.render_tree();
    tree.render_all().into_iter().find_map(|op| {
        let RenderOp::Draw(item) = op else { return None };
        let paint = tree.draw_list_snapshot(item.element).ok()?;
        paint
            .commands
            .iter()
            .any(|command| {
                matches!(command, V2DrawCommand::FillRect { color: c, .. }
                    if (c.r, c.g, c.b) == color)
            })
            .then_some(item.bounds)
    })
}

/// Whether anything painted a rectangle of exactly this color.
fn paints(app: &HeadlessAimerApp<impl Widget + 'static>, color: (u8, u8, u8)) -> bool {
    bounds_of(app, color).is_some()
}

/// A decoration that is not a `BoxDecoration`: one filled rectangle.
struct Swatch((u8, u8, u8));

impl Decoration for Swatch {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        canvas.fill_color_rect(origin, size, Color::Rgb(self.0.0, self.0.1, self.0.2));
    }
}

type Seen = Rc<RefCell<Vec<(bool, bool, bool)>>>;

/// A builder that records `(is_hovered, is_focused, is_empty)` each time it runs.
fn recording(seen: &Seen) -> impl Fn(TextFieldVisualState) -> AnyWidget + 'static {
    let seen = seen.clone();
    move |state| {
        seen.borrow_mut()
            .push((state.is_hovered, state.is_focused, state.is_empty));
        state.input().boxed()
    }
}

#[test]
fn a_builder_sees_focus_and_emptiness_and_keeps_the_editor_mounted() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let seen = Seen::default();
    let mut app = headless(
        TextField::new()
            .cursor_color(Colors::Magenta)
            .builder(recording(&seen)),
    );
    assert_eq!(seen.borrow().last(), Some(&(false, false, true)));

    click(&mut app, 100.0, 12.0);
    app.pump_frames(3);
    assert!(seen.borrow().last().unwrap().1, "focus reaches the builder");

    type_text(&mut app, "hello");
    app.pump_frames(3);
    assert!(!seen.borrow().last().unwrap().2, "typing reaches the builder");
    let after_hello = caret_x(&app);
    type_text(&mut app, " and more");
    app.pump_frames(3);
    assert!(
        caret_x(&app) > after_hello + 20.0,
        "the editor kept its caret across the chrome's rebuilds"
    );
}

#[test]
fn a_builder_follows_the_text_changed_through_the_controller() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let seen = Seen::default();
    let controller = TextEditingController::with_text("kept");
    let mut app = headless(
        TextField::new()
            .controller(controller.clone())
            .builder(recording(&seen)),
    );
    assert!(!seen.borrow().last().unwrap().2);

    controller.clear();
    app.pump_frames(3);
    assert!(seen.borrow().last().unwrap().2, "an external clear empties the field");
}

#[test]
fn a_builder_overrides_every_decoration() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let seen = Seen::default();
    let app = headless(
        TextField::new()
            .decoration(BoxDecoration::new().background_color(Color::Rgb(NORMAL.0, NORMAL.1, NORMAL.2)))
            .focus_decoration(Swatch(FOCUSED))
            .builder(recording(&seen)),
    );
    assert!(!paints(&app, NORMAL), "the builder owns the appearance");
    assert!(!paints(&app, FOCUSED));
}

#[test]
fn a_custom_decoration_follows_the_field_state() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = headless(
        TextField::new()
            .decoration(Swatch(NORMAL))
            .focus_decoration(Swatch(FOCUSED)),
    );
    assert!(paints(&app, NORMAL));
    assert!(!paints(&app, FOCUSED));
    let frame = bounds_of(&app, NORMAL).unwrap();
    assert_eq!(frame.width, 400.0, "the frame fills the width the field would");
    assert!(frame.height > 10.0 && frame.height < 60.0, "and wraps one line: {frame:?}");

    click(&mut app, 100.0, 12.0);
    app.pump_frames(3);
    assert!(paints(&app, FOCUSED), "focus selects the focus decoration");
    assert!(!paints(&app, NORMAL));
}

#[test]
fn a_box_decoration_is_still_painted_by_the_field() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let app = headless(TextField::new().decoration(
        BoxDecoration::new().background_color(Color::Rgb(NORMAL.0, NORMAL.1, NORMAL.2)),
    ));
    assert!(paints(&app, NORMAL));
}

#[test]
fn a_text_area_takes_a_builder() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let seen = Seen::default();
    let mut app = headless(TextArea::new().builder(recording(&seen)));
    click(&mut app, 100.0, 12.0);
    app.pump_frames(3);
    assert!(seen.borrow().last().unwrap().1);
}

fn move_pointer(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    app.pump_frames(3);
}

#[test]
fn a_builder_sees_the_pointer_arrive_and_leave() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let seen = Seen::default();
    let mut app = headless(TextField::new().builder(recording(&seen)));
    assert!(!seen.borrow().last().unwrap().0);

    move_pointer(&mut app, 100.0, 12.0);
    assert!(seen.borrow().last().unwrap().0, "hover reaches the builder");

    move_pointer(&mut app, 100.0, 150.0);
    assert!(
        !seen.borrow().last().unwrap().0,
        "leaving the field clears the hover the builder was shown"
    );
}

/// A parent whose rebuilds rebuild the field's widget, the way a theme or a
/// page change does, while the mounted field stays.
struct Rebuilding {
    seen: Seen,
    updater: Rc<Cell<Option<StateUpdater<RebuildingState>>>>,
}

struct RebuildingState {
    seen: Seen,
    handle: Rc<Cell<Option<StateUpdater<RebuildingState>>>>,
}

impl StatefulWidget for Rebuilding {
    type State = RebuildingState;

    fn create_state(self) -> RebuildingState {
        RebuildingState {
            seen: self.seen,
            handle: self.updater,
        }
    }
}

impl State<Rebuilding> for RebuildingState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.handle.set(Some(updater));
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        TextField::new().builder(recording(&self.seen))
    }
}

impl Widget for Rebuilding {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Rebuilding", None)
            .0
            .boxed()
    }
}
impl aimer_widget::PortableWidget for Rebuilding {}

#[test]
fn a_builder_keeps_hearing_about_the_field_after_its_parent_rebuilds() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let seen = Seen::default();
    let handle = Rc::new(Cell::new(None));
    let mut app = headless(Rebuilding {
        seen: seen.clone(),
        updater: handle.clone(),
    });
    for _ in 0..3 {
        handle.get().expect("the parent mounted").set_state(|_| {});
        app.pump_frames(3);
    }

    click(&mut app, 100.0, 12.0);
    app.pump_frames(3);
    assert!(
        seen.borrow().last().unwrap().1,
        "focus still reaches the builder once the field was rebuilt in place"
    );
}
