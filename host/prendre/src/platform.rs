//! Turns a [`Command`] into the event the application loop already understands.

use aimer_attribute::position::Vec2d;
use aimer_events::element as ev;
use aimer_events::pointer as ptr;
use aimer_platform::AimerNativePlatformEvent;
use aimer_platform::attachment::{MarkerPhase, SyntheticPointer};
use aimer_platform::menu::MenuShortcut;
use aimer_platform::system_back::BackSignal;

use crate::command::{BackStep, EditShortcut};
use crate::model::{KeyAction, Modifiers, NamedKey, PointerButton, ScrollDeltaKind, TouchPhase};
use crate::Command;

// Every match below lists each variant, so a variant added to either side is a
// compile error here instead of an input that is quietly mistranslated.

impl From<Modifiers> for ev::Modifiers {
    fn from(m: Modifiers) -> Self {
        Self { ctrl: m.ctrl, shift: m.shift, alt: m.alt, meta: m.meta }
    }
}

impl From<NamedKey> for ev::NamedKey {
    fn from(key: NamedKey) -> Self {
        match key {
            NamedKey::Backspace => Self::Backspace,
            NamedKey::Delete => Self::Delete,
            NamedKey::ArrowUp => Self::ArrowUp,
            NamedKey::ArrowDown => Self::ArrowDown,
            NamedKey::ArrowLeft => Self::ArrowLeft,
            NamedKey::ArrowRight => Self::ArrowRight,
            NamedKey::PageUp => Self::PageUp,
            NamedKey::PageDown => Self::PageDown,
            NamedKey::Home => Self::Home,
            NamedKey::End => Self::End,
            NamedKey::Enter => Self::Enter,
            NamedKey::Escape => Self::Escape,
            NamedKey::Tab => Self::Tab,
            NamedKey::Other(name) => Self::Other(name),
        }
    }
}

impl From<KeyAction> for ev::KeyAction {
    fn from(action: KeyAction) -> Self {
        match action {
            KeyAction::Pressed => Self::Pressed,
            KeyAction::Released => Self::Released,
            KeyAction::Repeat => Self::Repeat,
        }
    }
}

impl From<PointerButton> for ptr::PointerButton {
    fn from(button: PointerButton) -> Self {
        match button {
            PointerButton::Primary => Self::Primary,
            PointerButton::Secondary => Self::Secondary,
            PointerButton::Middle => Self::Middle,
            PointerButton::Other(code) => Self::Other(code),
        }
    }
}

impl From<ScrollDeltaKind> for ev::ScrollDeltaKind {
    fn from(kind: ScrollDeltaKind) -> Self {
        match kind {
            ScrollDeltaKind::Pixel => Self::Pixel,
            ScrollDeltaKind::Line => Self::Line,
        }
    }
}

impl From<TouchPhase> for ev::TouchPhase {
    fn from(phase: TouchPhase) -> Self {
        match phase {
            TouchPhase::Started => Self::Started,
            TouchPhase::Moved => Self::Moved,
            TouchPhase::Ended => Self::Ended,
            TouchPhase::Cancelled => Self::Cancelled,
        }
    }
}

impl EditShortcut {
    fn shortcut(self) -> MenuShortcut {
        let (letter, shift) = match self {
            Self::Undo => ("z", false),
            Self::Redo => ("z", true),
            Self::Cut => ("x", false),
            Self::Copy => ("c", false),
            Self::Paste => ("v", false),
            Self::SelectAll => ("a", false),
        };
        MenuShortcut { letter, shift }
    }
}

impl BackStep {
    fn signal(self) -> BackSignal {
        match self {
            Self::Started => BackSignal::Started,
            Self::Progressed(p) => BackSignal::Progressed(p),
            Self::Invoked => BackSignal::Invoked,
            Self::Cancelled => BackSignal::Cancelled,
        }
    }
}

impl Command {
    /// Converts this command into the event the application loop already
    /// understands, so injected input travels the same path as real input.
    ///
    /// Returns `None` for [`Command::Link`], which is queued in the platform's link
    /// inbox and has no event of its own, for [`Command::Settle`] and
    /// [`Command::Screenshot`], which are a wait and a question, not events, and
    /// for [`Command::PointerDownHere`] and [`Command::PointerUpHere`], whose
    /// position only the server knows. [`Command::deliver`] handles every command.
    pub fn into_platform_event(self) -> Option<AimerNativePlatformEvent> {
        let pointer = |x, y, button: PointerButton| {
            ptr::PointerInfo::new(Vec2d { x, y }, ptr::PointerSource::Mouse, 0, button.into())
        };
        let native = |event, modifiers: Modifiers| AimerNativePlatformEvent::NativeInput {
            event,
            modifiers: modifiers.into(),
        };
        let plain = |event| native(event, Modifiers::default());
        let at = |x, y| Vec2d { x, y };
        Some(match self {
            // Delivered whole there is no time to glide in: it is the move to the end.
            Self::PointerMove { x, y, modifiers } | Self::PointerGlide { x, y, modifiers, .. } => {
                native(ev::ElementEvent::PointerMove(pointer(x, y, PointerButton::Primary)), modifiers)
            }
            Self::PointerDown { x, y, button, modifiers } => {
                native(ev::ElementEvent::PointerDown(pointer(x, y, button)), modifiers)
            }
            Self::PointerUp { x, y, button, modifiers } => {
                native(ev::ElementEvent::PointerUp(pointer(x, y, button)), modifiers)
            }
            Self::PointerLeave => plain(ev::ElementEvent::PointerExited(ptr::PointerSource::Mouse, 0)),
            Self::Scroll { dx, dy, kind, phase, modifiers } => native(
                ev::ElementEvent::Scroll {
                    delta: at(dx, dy),
                    phase: phase.into(),
                    kind: kind.into(),
                    is_direct_manipulation: false,
                },
                modifiers,
            ),
            // Delivered whole there is no time to swipe in: it is the distance at once.
            Self::TrackpadScroll { dx, dy, modifiers, .. } => native(
                ev::ElementEvent::Scroll {
                    delta: at(dx, dy),
                    phase: ev::TouchPhase::Moved,
                    kind: ev::ScrollDeltaKind::Pixel,
                    is_direct_manipulation: false,
                },
                modifiers,
            ),
            Self::Touch { phase, id, x, y } => {
                let contact = ptr::PointerInfo::touch(at(x, y), id);
                plain(match phase {
                    TouchPhase::Started => ev::ElementEvent::PointerDown(contact),
                    TouchPhase::Moved => ev::ElementEvent::PointerMove(contact),
                    TouchPhase::Ended => ev::ElementEvent::PointerUp(contact),
                    TouchPhase::Cancelled => ev::ElementEvent::Cancel,
                })
            }
            Self::TouchCancel => plain(ev::ElementEvent::Cancel),
            Self::Pinch { x, y, delta, phase } => {
                plain(ev::ElementEvent::Pinch { pos: at(x, y), delta, phase: phase.into() })
            }
            Self::Rotate { x, y, delta, phase } => {
                plain(ev::ElementEvent::Rotate { pos: at(x, y), delta, phase: phase.into() })
            }
            Self::Pan { x, y, dx, dy, phase } => {
                plain(ev::ElementEvent::Pan { pos: at(x, y), delta: at(dx, dy), phase: phase.into() })
            }
            Self::SmartZoom { x, y } => plain(ev::ElementEvent::SmartZoom { pos: at(x, y) }),
            Self::Key { key, action, modifiers } => AimerNativePlatformEvent::NativeInput {
                event: ev::ElementEvent::KeyInput {
                    key: key.into(),
                    action: action.into(),
                    modifiers: modifiers.into(),
                },
                modifiers: modifiers.into(),
            },
            Self::Text(text) => AimerNativePlatformEvent::InsertText(text),
            // Delivered whole there is no time to type in: it is the text at once.
            Self::Typing { text, .. } => AimerNativePlatformEvent::InsertText(text),
            Self::Preedit { text, cursor } => AimerNativePlatformEvent::SetPreedit { text, cursor },
            Self::SoftBackspace => AimerNativePlatformEvent::ForceBackspace,
            Self::Back(step) => AimerNativePlatformEvent::Back(step.signal()),
            Self::Link(_) | Self::Settle | Self::Screenshot { .. } => return None,
            // Where the mouse is is the server's to say; without it there is no event.
            Self::PointerDownHere { .. } | Self::PointerUpHere { .. } => return None,
            Self::Menu(shortcut) => AimerNativePlatformEvent::MenuShortcut(shortcut.shortcut()),
            Self::FileHover(path) => plain(ev::ElementEvent::HoveredFile { path, pos: None }),
            Self::FileHoverCancel => plain(ev::ElementEvent::HoveredFileCancelled),
            Self::FileDrop(path) => plain(ev::ElementEvent::DroppedFile { path, pos: None }),
            Self::Quit => AimerNativePlatformEvent::ShutdownRequested,
        })
    }

    /// Where an injected mouse or finger is, and what it just did, for commands
    /// that place one. The debug overlay draws it: an injected event moves no
    /// real cursor, so without this nothing would show where the driver is.
    ///
    /// A touch cancel names no finger, because the framework cancels every
    /// press: it is reported as a `Left` touch and clears them all.
    pub fn synthetic_pointer(&self) -> Option<SyntheticPointer> {
        let at = |touch, id, phase, x, y| Some(SyntheticPointer { touch, id, phase, pos: Vec2d { x, y } });
        match *self {
            Self::PointerMove { x, y, .. } | Self::PointerGlide { x, y, .. } => at(false, 0, MarkerPhase::Moved, x, y),
            Self::PointerDown { x, y, .. } => at(false, 0, MarkerPhase::Pressed, x, y),
            Self::PointerUp { x, y, .. } => at(false, 0, MarkerPhase::Released, x, y),
            Self::PointerLeave => at(false, 0, MarkerPhase::Left, 0.0, 0.0),
            Self::Touch { phase, id, x, y } => {
                let phase = match phase {
                    TouchPhase::Started => MarkerPhase::Pressed,
                    TouchPhase::Moved => MarkerPhase::Moved,
                    TouchPhase::Ended => MarkerPhase::Released,
                    TouchPhase::Cancelled => MarkerPhase::Left,
                };
                at(true, id, phase, x, y)
            }
            Self::TouchCancel => at(true, 0, MarkerPhase::Left, 0.0, 0.0),
            _ => None,
        }
    }

    /// Hands this command to the running application's event loop.
    ///
    /// Returns `false` when the loop does not exist yet or has gone, so a driver
    /// can retry; the command is dropped in that case, a link included.
    pub fn deliver(self) -> bool {
        let Some(proxy) = aimer_platform::EVENT_PROXY.get() else {
            return false;
        };
        // Ahead of the input itself, so the loop learns it is not a person's.
        if let Some(marker) = self.synthetic_pointer() {
            let _ = proxy.send_event(AimerNativePlatformEvent::SyntheticPointer(marker));
        }
        match self {
            // Nothing is gliding when a command is delivered on its own.
            Self::Settle => true,
            // A picture is the reply to a connection; delivered alone it has none to go to.
            Self::Screenshot { .. } => false,
            // There is no mouse to ask where it is.
            Self::PointerDownHere { .. } | Self::PointerUpHere { .. } => false,
            // Queued, then the loop is woken to deliver it on its own thread.
            Self::Link(url) => {
                aimer_platform::link_inbox::post_link(url);
                true
            }
            other => {
                let event = other.into_platform_event().expect("only a link, a settle and a screenshot have no event");
                proxy.send_event(event).is_ok()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use aimer_events::element as ev;
    use aimer_events::pointer as ptr;

    use super::*;
    use crate::model::{KeyAction, Modifiers, NamedKey, PointerButton, ScrollDeltaKind, TouchPhase};

    fn event(command: Command) -> AimerNativePlatformEvent {
        command.into_platform_event().expect("this command has an event")
    }

    fn native(command: Command) -> (ev::ElementEvent, ev::Modifiers) {
        match event(command) {
            AimerNativePlatformEvent::NativeInput { event, modifiers } => (event, modifiers),
            other => panic!("expected NativeInput, got {other:?}"),
        }
    }

    fn none() -> Modifiers {
        Modifiers::default()
    }

    #[test]
    fn pointer_commands_become_mouse_events_at_the_same_position() {
        let (down, _) = native(Command::PointerDown {
            x: 3.0,
            y: 4.0,
            button: PointerButton::Secondary,
            modifiers: none(),
        });
        let ev::ElementEvent::PointerDown(info) = down else { panic!("not a press: {down:?}") };
        assert_eq!((info.pos.x, info.pos.y), (3.0, 4.0));
        assert_eq!((info.source, info.id, info.button), (ptr::PointerSource::Mouse, 0, ptr::PointerButton::Secondary));

        let (moved, _) = native(Command::PointerMove { x: 1.0, y: 2.0, modifiers: none() });
        assert!(matches!(moved, ev::ElementEvent::PointerMove(i) if i.button == ptr::PointerButton::Primary));
        assert!(matches!(native(Command::PointerLeave).0, ev::ElementEvent::PointerExited(ptr::PointerSource::Mouse, 0)));
    }

    #[test]
    fn pointer_modifiers_travel_beside_the_event() {
        let held = Modifiers { ctrl: true, ..none() };
        let (_, outer) = native(Command::PointerDown {
            x: 0.0,
            y: 0.0,
            button: PointerButton::Primary,
            modifiers: held.clone(),
        });
        assert_eq!(outer, ev::Modifiers::from(held));
    }

    #[test]
    fn scroll_keeps_its_kind_and_phase_and_is_never_held() {
        let (event, _) = native(Command::Scroll {
            dx: 1.0,
            dy: -9.0,
            kind: ScrollDeltaKind::Line,
            phase: TouchPhase::Ended,
            modifiers: none(),
        });
        let ev::ElementEvent::Scroll { delta, kind, phase, is_direct_manipulation } = event else {
            panic!("not a scroll: {event:?}")
        };
        assert_eq!((delta.x, delta.y), (1.0, -9.0));
        assert_eq!((kind, phase), (ev::ScrollDeltaKind::Line, ev::TouchPhase::Ended));
        assert!(!is_direct_manipulation);
    }

    #[test]
    fn a_touch_is_a_finger_with_its_own_id() {
        for (phase, check) in [
            (TouchPhase::Started, 0),
            (TouchPhase::Moved, 1),
            (TouchPhase::Ended, 2),
        ] {
            let (event, _) = native(Command::Touch { phase, id: 5, x: 8.0, y: 9.0 });
            let info = match (check, event) {
                (0, ev::ElementEvent::PointerDown(i)) | (1, ev::ElementEvent::PointerMove(i)) | (2, ev::ElementEvent::PointerUp(i)) => i,
                (_, other) => panic!("wrong event for {phase:?}: {other:?}"),
            };
            assert_eq!((info.source, info.id), (ptr::PointerSource::Touch, 5));
            assert_eq!((info.pos.x, info.pos.y), (8.0, 9.0));
        }
        assert!(matches!(native(Command::TouchCancel).0, ev::ElementEvent::Cancel));
    }

    #[test]
    fn gestures_keep_their_anchor_delta_and_phase() {
        let (pinch, _) = native(Command::Pinch { x: 1.0, y: 2.0, delta: 0.25, phase: TouchPhase::Moved });
        assert!(matches!(pinch, ev::ElementEvent::Pinch { pos, delta, phase: ev::TouchPhase::Moved }
            if (pos.x, pos.y, delta) == (1.0, 2.0, 0.25)));
        let (rotate, _) = native(Command::Rotate { x: 1.0, y: 2.0, delta: -1.0, phase: TouchPhase::Started });
        assert!(matches!(rotate, ev::ElementEvent::Rotate { delta, phase: ev::TouchPhase::Started, .. } if delta == -1.0));
        let (pan, _) = native(Command::Pan { x: 1.0, y: 2.0, dx: 3.0, dy: 4.0, phase: TouchPhase::Ended });
        assert!(matches!(pan, ev::ElementEvent::Pan { delta, phase: ev::TouchPhase::Ended, .. }
            if (delta.x, delta.y) == (3.0, 4.0)));
        let (zoom, _) = native(Command::SmartZoom { x: 6.0, y: 7.0 });
        assert!(matches!(zoom, ev::ElementEvent::SmartZoom { pos } if (pos.x, pos.y) == (6.0, 7.0)));
    }

    #[test]
    fn key_modifiers_travel_with_the_event_and_beside_it() {
        let modifiers = Modifiers { ctrl: true, ..Modifiers::default() };
        let (event, outer) = native(Command::Key {
            key: NamedKey::Enter,
            action: KeyAction::Pressed,
            modifiers: modifiers.clone(),
        });
        assert_eq!(outer, ev::Modifiers::from(modifiers));
        assert!(matches!(event, ev::ElementEvent::KeyInput { modifiers: m, .. } if m == ev::Modifiers::from(modifiers)));
    }

    #[test]
    fn text_is_inserted_as_a_single_edit() {
        let event = event(Command::Text("你好".into()));
        assert!(matches!(event, AimerNativePlatformEvent::InsertText(t) if t == "你好"));
    }

    #[test]
    fn preedit_and_soft_backspace_use_the_soft_keyboard_events() {
        let preedit = event(Command::Preedit { text: "ni".into(), cursor: Some((0, 2)) });
        assert!(matches!(preedit, AimerNativePlatformEvent::SetPreedit { text, cursor: Some((0, 2)) } if text == "ni"));
        assert!(matches!(event(Command::SoftBackspace), AimerNativePlatformEvent::ForceBackspace));
    }

    #[test]
    fn back_menu_and_quit_become_platform_events() {
        assert!(matches!(
            event(Command::Back(BackStep::Progressed(0.5))),
            AimerNativePlatformEvent::Back(BackSignal::Progressed(p)) if p == 0.5
        ));
        let redo = event(Command::Menu(EditShortcut::Redo));
        assert!(matches!(redo, AimerNativePlatformEvent::MenuShortcut(s) if s.letter == "z" && s.shift));
        let copy = event(Command::Menu(EditShortcut::Copy));
        assert!(matches!(copy, AimerNativePlatformEvent::MenuShortcut(s) if s.letter == "c" && !s.shift));
        assert!(matches!(event(Command::Quit), AimerNativePlatformEvent::ShutdownRequested));
    }

    #[test]
    fn file_drags_carry_their_path_and_no_position() {
        let path = PathBuf::from("/tmp/a.png");
        let (hover, _) = native(Command::FileHover(path.clone()));
        assert!(matches!(hover, ev::ElementEvent::HoveredFile { path: p, pos: None } if p == path));
        let (drop, _) = native(Command::FileDrop(path.clone()));
        assert!(matches!(drop, ev::ElementEvent::DroppedFile { path: p, pos: None } if p == path));
        assert!(matches!(native(Command::FileHoverCancel).0, ev::ElementEvent::HoveredFileCancelled));
    }

    fn marker(command: Command) -> Option<SyntheticPointer> {
        command.synthetic_pointer()
    }

    #[test]
    fn pointer_commands_say_where_the_mouse_is_and_what_it_did() {
        let held = Modifiers::default();
        let moved = marker(Command::PointerMove { x: 3.0, y: 4.0, modifiers: held }).expect("a marker");
        assert_eq!((moved.touch, moved.id, moved.phase), (false, 0, MarkerPhase::Moved));
        assert_eq!((moved.pos.x, moved.pos.y), (3.0, 4.0));
        let down = marker(Command::PointerDown { x: 1.0, y: 2.0, button: PointerButton::Secondary, modifiers: held });
        assert_eq!(down.map(|m| m.phase), Some(MarkerPhase::Pressed));
        let up = marker(Command::PointerUp { x: 1.0, y: 2.0, button: PointerButton::Primary, modifiers: held });
        assert_eq!(up.map(|m| m.phase), Some(MarkerPhase::Released));
        assert_eq!(marker(Command::PointerLeave).map(|m| (m.touch, m.phase)), Some((false, MarkerPhase::Left)));
    }

    #[test]
    fn touch_commands_say_which_finger_and_what_it_did() {
        for (phase, expected) in [
            (TouchPhase::Started, MarkerPhase::Pressed),
            (TouchPhase::Moved, MarkerPhase::Moved),
            (TouchPhase::Ended, MarkerPhase::Released),
        ] {
            let m = marker(Command::Touch { phase, id: 7, x: 5.0, y: 6.0 }).expect("a marker");
            assert_eq!((m.touch, m.id, m.phase), (true, 7, expected));
            assert_eq!((m.pos.x, m.pos.y), (5.0, 6.0));
        }
        assert_eq!(marker(Command::TouchCancel).map(|m| (m.touch, m.phase)), Some((true, MarkerPhase::Left)));
    }

    #[test]
    fn a_glide_arrives_as_a_move_to_its_target_when_delivered_whole() {
        let glide = Command::PointerGlide { x: 7.0, y: 8.0, millis: Some(200), modifiers: Modifiers::default() };
        let m = marker(glide.clone()).expect("a marker at the target");
        assert_eq!((m.phase, m.pos.x, m.pos.y), (MarkerPhase::Moved, 7.0, 8.0));
        // `deliver` has no time to spend, so a glide handed to it jumps.
        let (event, _) = native(glide);
        assert!(matches!(event, ev::ElementEvent::PointerMove(i) if (i.pos.x, i.pos.y) == (7.0, 8.0)));
    }

    #[test]
    fn commands_that_place_no_pointer_have_no_marker() {
        for command in [
            Command::Text("a".into()),
            Command::Quit,
            Command::SoftBackspace,
            Command::Link("aimer://x".into()),
            Command::FileHoverCancel,
            Command::Scroll {
                dx: 0.0,
                dy: 1.0,
                kind: ScrollDeltaKind::Line,
                phase: TouchPhase::Moved,
                modifiers: Modifiers::default(),
            },
        ] {
            assert_eq!(marker(command.clone()), None, "{command:?}");
        }
    }

    #[test]
    fn a_trackpad_scroll_delivered_whole_is_one_pixel_scroll_and_places_no_pointer() {
        let swipe = Command::TrackpadScroll { dx: 0.0, dy: -90.0, millis: Some(500), modifiers: Modifiers::default() };
        assert_eq!(swipe.synthetic_pointer(), None);
        let (event, _) = native(swipe);
        let ev::ElementEvent::Scroll { delta, kind, .. } = event else { panic!("{event:?}") };
        assert_eq!((delta.x, delta.y), (0.0, -90.0));
        assert_eq!(kind, ev::ScrollDeltaKind::Pixel);
    }

    #[test]
    fn typing_delivered_whole_is_one_commit_of_all_the_text_and_places_no_pointer() {
        let typing = Command::Typing { text: "hello".into(), millis: Some(500) };
        assert_eq!(typing.synthetic_pointer(), None);
        assert!(matches!(typing.into_platform_event(), Some(AimerNativePlatformEvent::InsertText(text)) if text == "hello"));
    }

    #[test]
    fn a_press_without_a_point_is_resolved_by_the_server_not_delivered_to_the_app() {
        let down = Command::PointerDownHere { button: PointerButton::Primary, modifiers: Modifiers::default() };
        assert!(down.clone().into_platform_event().is_none(), "nothing here knows where the mouse is");
        assert_eq!(down.synthetic_pointer(), None);
        assert!(!down.deliver(), "there is no event to send");
    }

    #[test]
    fn a_screenshot_is_answered_by_the_server_not_delivered_to_the_app() {
        let shot = Command::Screenshot { width: None, jpeg: None };
        assert!(shot.clone().into_platform_event().is_none());
        assert_eq!(shot.synthetic_pointer(), None);
        assert!(!Command::Screenshot { width: None, jpeg: None }.deliver(), "there is no one to send the picture to");
    }

    #[test]
    fn a_link_and_a_settle_have_no_event_of_their_own() {
        assert!(Command::Link("aimer://x".into()).into_platform_event().is_none());
        assert!(Command::Settle.into_platform_event().is_none());
        assert_eq!(Command::Settle.synthetic_pointer(), None);
    }

    #[test]
    fn nothing_is_delivered_before_the_loop_exists() {
        // No event loop is running in a unit test.
        assert!(!Command::Quit.deliver());
        assert!(!Command::Link("aimer://x".into()).deliver());
    }
}

#[cfg(test)]
mod mapping_tests {
    use aimer_events::element as ev;
    use aimer_events::pointer as ptr;

    use crate::model::{KeyAction, Modifiers, NamedKey, PointerButton, ScrollDeltaKind, TouchPhase};

    #[test]
    fn every_named_key_maps_to_the_framework_key_of_the_same_name() {
        let table = [
            (NamedKey::Backspace, ev::NamedKey::Backspace),
            (NamedKey::Delete, ev::NamedKey::Delete),
            (NamedKey::ArrowUp, ev::NamedKey::ArrowUp),
            (NamedKey::ArrowDown, ev::NamedKey::ArrowDown),
            (NamedKey::ArrowLeft, ev::NamedKey::ArrowLeft),
            (NamedKey::ArrowRight, ev::NamedKey::ArrowRight),
            (NamedKey::PageUp, ev::NamedKey::PageUp),
            (NamedKey::PageDown, ev::NamedKey::PageDown),
            (NamedKey::Home, ev::NamedKey::Home),
            (NamedKey::End, ev::NamedKey::End),
            (NamedKey::Enter, ev::NamedKey::Enter),
            (NamedKey::Escape, ev::NamedKey::Escape),
            (NamedKey::Tab, ev::NamedKey::Tab),
            (NamedKey::Other("c".into()), ev::NamedKey::Other("c".into())),
        ];
        for (ours, theirs) in table {
            assert_eq!(ev::NamedKey::from(ours.clone()), theirs, "{ours:?}");
        }
    }

    #[test]
    fn buttons_actions_kinds_and_phases_keep_their_meaning() {
        for (ours, theirs) in [
            (PointerButton::Primary, ptr::PointerButton::Primary),
            (PointerButton::Secondary, ptr::PointerButton::Secondary),
            (PointerButton::Middle, ptr::PointerButton::Middle),
            (PointerButton::Other(9), ptr::PointerButton::Other(9)),
        ] {
            assert_eq!(ptr::PointerButton::from(ours), theirs);
        }
        for (ours, theirs) in [
            (KeyAction::Pressed, ev::KeyAction::Pressed),
            (KeyAction::Released, ev::KeyAction::Released),
            (KeyAction::Repeat, ev::KeyAction::Repeat),
        ] {
            assert_eq!(ev::KeyAction::from(ours), theirs);
        }
        assert_eq!(ev::ScrollDeltaKind::from(ScrollDeltaKind::Pixel), ev::ScrollDeltaKind::Pixel);
        assert_eq!(ev::ScrollDeltaKind::from(ScrollDeltaKind::Line), ev::ScrollDeltaKind::Line);
        for (ours, theirs) in [
            (TouchPhase::Started, ev::TouchPhase::Started),
            (TouchPhase::Moved, ev::TouchPhase::Moved),
            (TouchPhase::Ended, ev::TouchPhase::Ended),
            (TouchPhase::Cancelled, ev::TouchPhase::Cancelled),
        ] {
            assert_eq!(ev::TouchPhase::from(ours), theirs);
        }
    }

    #[test]
    fn each_modifier_lands_on_its_own_flag() {
        for (ours, theirs) in [
            (Modifiers { ctrl: true, ..Default::default() }, ev::Modifiers { ctrl: true, ..Default::default() }),
            (Modifiers { shift: true, ..Default::default() }, ev::Modifiers { shift: true, ..Default::default() }),
            (Modifiers { alt: true, ..Default::default() }, ev::Modifiers { alt: true, ..Default::default() }),
            (Modifiers { meta: true, ..Default::default() }, ev::Modifiers { meta: true, ..Default::default() }),
            (Modifiers::default(), ev::Modifiers::default()),
        ] {
            assert_eq!(ev::Modifiers::from(ours), theirs);
        }
    }
}
