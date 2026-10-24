use super::*;
use aimer_events::element::{KeyAction, Modifiers, NamedKey};
use aimer_events::pointer::PointerSource;

struct InputRecorder(Arc<Mutex<Vec<ElementEvent>>>);
impl aimer_widget::PortableWidget for InputRecorder {}
impl Widget for InputRecorder {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        InputElement(self.0).boxed()
    }
}
struct InputElement(Arc<Mutex<Vec<ElementEvent>>>);
impl Drawable for InputElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for InputElement {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some((Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }))
    }
}
impl Rebuildable for InputElement {}
impl VisitorElement for InputElement {
    fn debug_name(&self) -> &'static str {
        "InputElement"
    }
}
impl EventElement for InputElement {
    fn on_event(&self, event: &ElementEvent) -> aimer_widget::EventResult {
        self.0.lock().unwrap().push(event.clone());
        aimer_widget::EventResult::consumed()
    }
}

fn native(event: ElementEvent) -> AimerNativePlatformEvent {
    AimerNativePlatformEvent::NativeInput {
        event,
        modifiers: Modifiers::default(),
    }
}

#[test]
fn ios_mouse_input_keeps_logical_coordinates_buttons_and_mouse_identity() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut app = AimerApp::start_headless_with(
        InputRecorder(events.clone()),
        HeadlessOptions {
            size: PhysicalSize::new(800, 600),
            scale_factor: 2.0,
        },
    );
    app.render_frame();
    app.app.touch_pointer = true;
    let pointer = PointerInfo::mouse(Vec2d { x: 20.25, y: 30.5 }, PointerButton::Secondary);
    app.send_user_event(native(ElementEvent::PointerMove(pointer)));
    app.send_user_event(native(ElementEvent::PointerDown(pointer)));
    app.send_user_event(native(ElementEvent::PointerMove(
        pointer.at(Vec2d { x: 40.0, y: 50.0 }),
    )));
    app.send_user_event(native(ElementEvent::PointerUp(
        pointer.at(Vec2d { x: 40.25, y: 50.25 }),
    )));
    assert!(!app.app.touch_pointer);
    assert_eq!(app.app.cursor_pos, Vec2d { x: 40.25, y: 50.25 });
    let events = events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ElementEvent::PointerDown(p)
        if *p == pointer && p.source == PointerSource::Mouse))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ElementEvent::PointerMove(p)
        if p.pos.x == 40.0 && p.button == PointerButton::Secondary))
    );
    assert!(events.iter().any(|e| matches!(e, ElementEvent::PointerUp(p)
        if p.pos.x == 40.25 && p.button == PointerButton::Secondary)));
}

#[test]
fn ios_hardware_key_actions_and_modifiers_do_not_commit_text_twice() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut app = AimerApp::start_headless(InputRecorder(events.clone()));
    app.render_frame();
    app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };
    let modifiers = Modifiers {
        meta: true,
        shift: true,
        ..Default::default()
    };
    for action in [KeyAction::Pressed, KeyAction::Repeat, KeyAction::Released] {
        app.send_user_event(AimerNativePlatformEvent::NativeInput {
            event: ElementEvent::KeyInput {
                key: NamedKey::Other("c".into()),
                action,
                modifiers: modifiers.clone(),
            },
            modifiers: modifiers.clone(),
        });
    }
    let events = events.lock().unwrap();
    let actions: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ElementEvent::KeyInput {
                action,
                modifiers: m,
                ..
            } => {
                assert_eq!(m, &modifiers);
                Some(action.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        actions,
        [KeyAction::Pressed, KeyAction::Repeat, KeyAction::Released]
    );
    assert!(!events.iter().any(|e| matches!(
        e,
        ElementEvent::CharInput { .. } | ElementEvent::TextInput { .. }
    )));
    assert_eq!(app.app.current_modifiers, modifiers);
}

#[test]
fn ios_mouse_hover_at_the_last_touch_position_changes_input_source() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut app = AimerApp::start_headless(InputRecorder(events.clone()));
    app.render_frame();
    app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };
    app.app.touch_pointer = true;
    app.send_user_event(native(ElementEvent::PointerMove(PointerInfo::mouse(
        app.app.cursor_pos,
        PointerButton::Primary,
    ))));
    assert!(!app.app.touch_pointer);
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, ElementEvent::PointerMove(p)
        if p.source == PointerSource::Mouse))
    );
}

#[test]
fn ios_pointer_cancellation_does_not_turn_into_a_click_release() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut app = AimerApp::start_headless(InputRecorder(events.clone()));
    app.render_frame();
    app.send_user_event(native(ElementEvent::PointerDown(PointerInfo::mouse(
        Vec2d { x: 20.0, y: 20.0 },
        PointerButton::Primary,
    ))));
    events.lock().unwrap().clear();
    app.send_user_event(native(ElementEvent::Cancel));
    assert_eq!(app.app.pressed_button, None);
    let events = events.lock().unwrap();
    assert!(events.iter().any(|e| matches!(e, ElementEvent::Cancel)));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ElementEvent::PointerUp(_)))
    );
}

#[test]
fn ios_scroll_reaches_the_existing_frame_synchronized_scroll_path() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut app = AimerApp::start_headless_with(
        ScrollRecordingWidget {
            events: events.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(800, 600),
            scale_factor: 3.0,
        },
    );
    app.render_frame();
    app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };
    for (phase, y) in [
        (TouchPhase::Started, 0.0),
        (TouchPhase::Moved, -12.0),
        (TouchPhase::Ended, 0.0),
    ] {
        app.send_user_event(native(ElementEvent::Scroll {
            delta: Vec2d { x: 0.0, y },
            phase,
            kind: ScrollDeltaKind::Pixel,
            is_direct_manipulation: true,
        }));
    }
    assert!(app.app.scroll_smoother.is_active());
    app.render_frame();
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|(delta, kind, _)| delta.y < 0.0 && *kind == ScrollDeltaKind::Pixel)
    );
}

#[test]
fn ios_hardware_keys_edit_the_focused_field_even_when_the_mouse_is_elsewhere() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let controller = TextEditingController::with_text("abc");
    let focus = FocusNode::new();
    let mut app = AimerApp::start_headless_with(
        RetainedTextInputWidget {
            controller: controller.clone(),
            focus_node: focus.clone(),
            caret_context: Rc::new(RefCell::new(None)),
            multiline: false,
        },
        HeadlessOptions {
            size: PhysicalSize::new(320, 120),
            scale_factor: 1.0,
        },
    );
    app.render_frame();
    focus_headless_field(&mut app, &focus);
    app.app.cursor_pos = Vec2d { x: 300.0, y: 110.0 };
    for key in [NamedKey::End, NamedKey::ArrowLeft, NamedKey::Backspace] {
        app.send_user_event(native(ElementEvent::KeyInput {
            key,
            action: KeyAction::Pressed,
            modifiers: Modifiers::default(),
        }));
    }
    assert_eq!(controller.value().text(), "ac");
    // The raw key reports the physical action; only the native text commit
    // inserts its character. This also covers a non-ASCII layout character.
    app.send_user_event(native(ElementEvent::KeyInput {
        key: NamedKey::Other("β".into()),
        action: KeyAction::Pressed,
        modifiers: Modifiers::default(),
    }));
    assert_eq!(controller.value().text(), "ac");
    app.send_user_event(AimerNativePlatformEvent::InsertText("β".into()));
    assert_eq!(controller.value().text(), "aβc");
    let modifiers = Modifiers {
        meta: true,
        ..Default::default()
    };
    app.send_user_event(AimerNativePlatformEvent::NativeInput {
        event: ElementEvent::KeyInput {
            key: NamedKey::Other("a".into()),
            action: KeyAction::Pressed,
            modifiers: modifiers.clone(),
        },
        modifiers,
    });
    assert_eq!(controller.value().selection().range(), 0.."aβc".len());
}
