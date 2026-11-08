//! Input a driver injects (`aimer_prendre`) takes the same route as the
//! platform's own: touch contacts and file drags, which arrive as window events
//! on a real device, must also be accepted as `NativeInput`.

use super::*;
use aimer_events::element::{Modifiers, TouchPhase};
use aimer_events::pointer::PointerSource;
use std::path::PathBuf;

struct Recorder(Arc<Mutex<Vec<ElementEvent>>>);
impl Widget for Recorder {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RecorderElement(self.0).boxed()
    }
}
struct RecorderElement(Arc<Mutex<Vec<ElementEvent>>>);
impl Drawable for RecorderElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for RecorderElement {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some((Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }))
    }
}
impl Rebuildable for RecorderElement {}
impl VisitorElement for RecorderElement {
    fn debug_name(&self) -> &'static str {
        "RecorderElement"
    }
}
impl EventElement for RecorderElement {
    fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
        self.0.lock().unwrap().push(event.clone());
        aimer_widget::EventResult::consumed()
    }
}

fn native(event: ElementEvent) -> AimerNativePlatformEvent {
    AimerNativePlatformEvent::NativeInput { event, modifiers: Modifiers::default() }
}

fn start() -> (HeadlessAimerApp<ModalHost<Recorder>>, Arc<Mutex<Vec<ElementEvent>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut app = AimerApp::start_headless_with(
        Recorder(events.clone()),
        HeadlessOptions { size: PhysicalSize::new(800, 600), scale_factor: 2.0 },
    );
    app.render_frame();
    (app, events)
}

#[test]
fn a_touch_contact_reaches_the_tree_with_its_finger_id() {
    let (mut app, events) = start();
    let at = Vec2d { x: 20.0, y: 30.0 };
    app.send_user_event(native(ElementEvent::PointerDown(PointerInfo::touch(at, 7))));
    app.send_user_event(native(ElementEvent::PointerMove(PointerInfo::touch(at.x_plus(5.0), 7))));
    app.send_user_event(native(ElementEvent::PointerUp(PointerInfo::touch(at.x_plus(5.0), 7))));

    assert!(app.app.touch_pointer, "a touch contact marks the pointer as touch");
    let events = events.lock().unwrap();
    let seen: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ElementEvent::PointerDown(p) => Some(("down", *p)),
            ElementEvent::PointerMove(p) => Some(("move", *p)),
            ElementEvent::PointerUp(p) => Some(("up", *p)),
            _ => None,
        })
        .collect();
    assert_eq!(seen.len(), 3, "{events:?}");
    for (_, p) in &seen {
        assert_eq!((p.source, p.id), (PointerSource::Touch, 7));
    }
    assert_eq!(seen[2].1.pos, Vec2d { x: 25.0, y: 30.0 });
}

#[test]
fn a_cancelled_touch_cancels_what_the_finger_started() {
    let (mut app, events) = start();
    let at = Vec2d { x: 20.0, y: 30.0 };
    app.send_user_event(native(ElementEvent::PointerDown(PointerInfo::touch(at, 1))));
    app.send_user_event(native(ElementEvent::Cancel));
    // Cancel is delivered to the tree that was pressed, not swallowed.
    assert!(events.lock().unwrap().iter().any(|e| matches!(e, ElementEvent::Cancel)));
}

#[test]
fn a_file_drag_enters_moves_and_drops() {
    let (mut app, events) = start();
    let path = PathBuf::from("/tmp/a.png");
    app.send_user_event(native(ElementEvent::PointerMove(PointerInfo::mouse(
        Vec2d { x: 10.0, y: 10.0 },
        PointerButton::Primary,
    ))));
    app.send_user_event(native(ElementEvent::HoveredFile { path: path.clone(), pos: None }));
    app.send_user_event(native(ElementEvent::PointerMove(PointerInfo::mouse(
        Vec2d { x: 40.0, y: 40.0 },
        PointerButton::Primary,
    ))));
    app.send_user_event(native(ElementEvent::DroppedFile { path: path.clone(), pos: None }));

    let events = events.lock().unwrap();
    assert!(events.iter().any(|e| matches!(e, ElementEvent::HoveredFile { path: p, .. } if *p == path)));
    assert!(
        events.iter().any(|e| matches!(e, ElementEvent::HoveredFileMoved { paths, pos }
            if paths.as_ref() == [path.clone()] && pos.x == 40.0)),
        "moving while dragging reports the batch: {events:?}"
    );
    assert!(events.iter().any(|e| matches!(e, ElementEvent::DroppedFile { path: p, .. } if *p == path)));
    assert!(!app.app.file_drag.is_active(), "a drop ends the drag");
}

#[test]
fn a_cancelled_file_drag_ends_the_drag() {
    let (mut app, events) = start();
    app.send_user_event(native(ElementEvent::HoveredFile {
        path: PathBuf::from("/tmp/a.png"),
        pos: None,
    }));
    assert!(app.app.file_drag.is_active());
    app.send_user_event(native(ElementEvent::HoveredFileCancelled));
    assert!(!app.app.file_drag.is_active());
    assert!(events.lock().unwrap().iter().any(|e| matches!(e, ElementEvent::HoveredFileCancelled)));
}

trait XPlus {
    fn x_plus(self, dx: f32) -> Self;
}
impl XPlus for Vec2d {
    fn x_plus(self, dx: f32) -> Self {
        Vec2d { x: self.x + dx, ..self }
    }
}

#[allow(dead_code)]
fn _phase_is_reexported(_: TouchPhase) {}
