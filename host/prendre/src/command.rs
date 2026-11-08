use aimer_attribute::position::Vec2d;
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey, ScrollDeltaKind, TouchPhase};
use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
use aimer_platform::AimerNativePlatformEvent;

/// One synthetic input event that can be sent to a running application.
///
/// Positions and deltas are in **logical pixels** in window coordinates, the
/// same space the platform bridges report in. Every float in a command that
/// came from [`Command::parse`](crate::Command::parse) is finite.
///
/// # Examples
///
/// ```
/// use aimer_prendre::Command;
///
/// let click: Vec<Command> = ["down 10 20", "up 10 20"]
///     .iter()
///     .map(|line| line.parse().expect("valid command"))
///     .collect();
/// assert_eq!(click.len(), 2);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// The mouse moved to `(x, y)`.
    PointerMove { x: f32, y: f32 },
    /// `button` went down at `(x, y)`.
    PointerDown { x: f32, y: f32, button: PointerButton },
    /// `button` came up at `(x, y)`.
    PointerUp { x: f32, y: f32, button: PointerButton },
    /// The mouse left the window.
    PointerLeave,
    /// A pixel-precise scroll by `(dx, dy)`.
    Scroll { dx: f32, dy: f32 },
    /// A named key changed state.
    Key {
        key: NamedKey,
        action: KeyAction,
        modifiers: Modifiers,
    },
    /// Text committed as one edit, exactly like an IME commit.
    Text(String),
}

impl Command {
    /// Converts this command into the event the application loop already
    /// understands, so injected input travels the same path as real input.
    pub fn into_platform_event(self) -> AimerNativePlatformEvent {
        let pointer = |x, y, button| PointerInfo::new(Vec2d { x, y }, PointerSource::Mouse, 0, button);
        let native = |event| AimerNativePlatformEvent::NativeInput {
            event,
            modifiers: Modifiers::default(),
        };
        match self {
            Self::PointerMove { x, y } => {
                native(ElementEvent::PointerMove(pointer(x, y, PointerButton::Primary)))
            }
            Self::PointerDown { x, y, button } => native(ElementEvent::PointerDown(pointer(x, y, button))),
            Self::PointerUp { x, y, button } => native(ElementEvent::PointerUp(pointer(x, y, button))),
            Self::PointerLeave => native(ElementEvent::PointerExited(PointerSource::Mouse, 0)),
            Self::Scroll { dx, dy } => native(ElementEvent::Scroll {
                delta: Vec2d { x: dx, y: dy },
                phase: TouchPhase::Moved,
                kind: ScrollDeltaKind::Pixel,
                is_direct_manipulation: false,
            }),
            Self::Key {
                key,
                action,
                modifiers,
            } => AimerNativePlatformEvent::NativeInput {
                event: ElementEvent::KeyInput {
                    key,
                    action,
                    modifiers: modifiers.clone(),
                },
                modifiers,
            },
            Self::Text(text) => AimerNativePlatformEvent::InsertText(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native(command: Command) -> (ElementEvent, Modifiers) {
        match command.into_platform_event() {
            AimerNativePlatformEvent::NativeInput { event, modifiers } => (event, modifiers),
            other => panic!("expected NativeInput, got {other:?}"),
        }
    }

    #[test]
    fn pointer_commands_become_mouse_events_at_the_same_position() {
        let (down, _) = native(Command::PointerDown { x: 3.0, y: 4.0, button: PointerButton::Secondary });
        let ElementEvent::PointerDown(info) = down else { panic!("not a press: {down:?}") };
        assert_eq!((info.pos.x, info.pos.y), (3.0, 4.0));
        assert_eq!((info.source, info.id, info.button), (PointerSource::Mouse, 0, PointerButton::Secondary));

        let (moved, _) = native(Command::PointerMove { x: 1.0, y: 2.0 });
        assert!(matches!(moved, ElementEvent::PointerMove(i) if i.button == PointerButton::Primary));
        assert!(matches!(native(Command::PointerLeave).0, ElementEvent::PointerExited(PointerSource::Mouse, 0)));
    }

    #[test]
    fn scroll_is_a_pixel_delta_the_user_does_not_hold() {
        let (event, _) = native(Command::Scroll { dx: 1.0, dy: -9.0 });
        let ElementEvent::Scroll { delta, kind, is_direct_manipulation, .. } = event else {
            panic!("not a scroll: {event:?}")
        };
        assert_eq!((delta.x, delta.y), (1.0, -9.0));
        assert_eq!(kind, ScrollDeltaKind::Pixel);
        assert!(!is_direct_manipulation);
    }

    #[test]
    fn key_modifiers_travel_with_the_event_and_beside_it() {
        let modifiers = Modifiers { ctrl: true, ..Modifiers::default() };
        let (event, outer) = native(Command::Key {
            key: NamedKey::Enter,
            action: KeyAction::Pressed,
            modifiers: modifiers.clone(),
        });
        assert_eq!(outer, modifiers);
        assert!(matches!(event, ElementEvent::KeyInput { modifiers: m, .. } if m == modifiers));
    }

    #[test]
    fn text_is_inserted_as_a_single_edit() {
        let event = Command::Text("你好".into()).into_platform_event();
        assert!(matches!(event, AimerNativePlatformEvent::InsertText(t) if t == "你好"));
    }
}
