use super::*;
use crate::Command;
use crate::model::{KeyAction, Modifiers, NamedKey, PointerButton, ScrollDeltaKind, TouchPhase};

fn lines(commands: &[Command]) -> Vec<String> {
    commands.iter().map(ToString::to_string).collect()
}

#[test]
fn a_click_glides_to_the_point_then_presses_and_releases_there() {
    assert_eq!(
        lines(&click(10.0, 20.0, PointerButton::Primary, 1).unwrap()),
        ["glide 10 20", "down 10 20 primary", "up 10 20 primary"]
    );
    assert_eq!(
        lines(&click(1.0, 2.0, PointerButton::Secondary, 1).unwrap())[1],
        "down 1 2 secondary"
    );
}

#[test]
fn a_double_and_a_triple_click_are_that_many_presses_after_one_glide() {
    for count in [2u8, 3] {
        let commands = click(5.0, 5.0, PointerButton::Primary, count).unwrap();
        assert_eq!(commands.len(), 1 + 2 * usize::from(count));
        assert!(matches!(commands[0], Command::PointerGlide { .. }));
        let downs = commands.iter().filter(|c| matches!(c, Command::PointerDown { .. })).count();
        assert_eq!(downs, usize::from(count));
    }
    assert!(click(0.0, 0.0, PointerButton::Primary, 0).is_err(), "no clicks is not a click");
    assert!(click(0.0, 0.0, PointerButton::Primary, 4).is_err(), "more than a triple is a mistake");
}

#[test]
fn a_drag_glides_to_the_start_presses_glides_to_the_end_and_releases() {
    assert_eq!(
        lines(&drag((10.0, 10.0), (110.0, 60.0)).unwrap()),
        ["glide 10 10", "down 10 10 primary", "glide 110 60", "up 110 60 primary"]
    );
}

#[test]
fn scrolling_down_makes_the_content_move_up() {
    // The offset of a scrolled-down list is negative, so the delta is too.
    let sign = |direction| match scroll(None, direction, 3.0).unwrap().as_slice() {
        [Command::Scroll { dx, dy, kind: ScrollDeltaKind::Line, .. }] => (*dx, *dy),
        other => panic!("{other:?}"),
    };
    assert_eq!(sign(Direction::Down), (0.0, -3.0));
    assert_eq!(sign(Direction::Up), (0.0, 3.0));
    assert_eq!(sign(Direction::Right), (-3.0, 0.0));
    assert_eq!(sign(Direction::Left), (3.0, 0.0));
}

#[test]
fn a_scroll_with_a_position_glides_there_and_waits_for_the_mouse_before_scrolling() {
    assert_eq!(
        lines(&scroll(Some((40.0, 50.0)), Direction::Down, 1.0).unwrap()),
        ["glide 40 50", "settle", "scroll 0 -1 kind=line"]
    );
}

#[test]
fn a_scroll_without_a_position_scrolls_where_the_mouse_already_is() {
    assert_eq!(lines(&scroll(None, Direction::Up, 2.0).unwrap()), ["scroll 0 2 kind=line"]);
}

#[test]
fn a_scroll_amount_must_be_sensible() {
    assert!(scroll(None, Direction::Down, 0.0).is_err());
    assert!(scroll(None, Direction::Down, -1.0).is_err());
    assert!(scroll(None, Direction::Down, 10_001.0).is_err());
    assert!(scroll(None, Direction::Down, f32::NAN).is_err());
}

#[test]
fn directions_are_read_by_name() {
    assert_eq!("down".parse(), Ok(Direction::Down));
    assert_eq!("UP".parse(), Ok(Direction::Up));
    assert!("sideways".parse::<Direction>().is_err());
}

#[test]
fn a_tap_is_a_finger_down_and_up_in_place() {
    assert_eq!(lines(&tap(7.0, 8.0).unwrap()), ["touch start 1 7 8", "touch end 1 7 8"]);
}

#[test]
fn a_swipe_is_one_finger_along_a_path() {
    let commands = swipe((0.0, 100.0), (0.0, 0.0)).unwrap();
    assert!(matches!(commands[0], Command::Touch { phase: TouchPhase::Started, id: 1, y: 100.0, .. }));
    assert!(matches!(commands.last(), Some(Command::Touch { phase: TouchPhase::Ended, id: 1, y: 0.0, .. })));
    let moves = commands.iter().filter(|c| matches!(c, Command::Touch { phase: TouchPhase::Moved, .. })).count();
    assert!(moves >= 3, "a flick needs intermediate samples");
    assert!(commands.iter().all(|c| matches!(c, Command::Touch { id: 1, .. })));
}

#[test]
fn a_pinch_changes_the_scale_by_the_asked_factor() {
    let commands = pinch(50.0, 60.0, 1.5).unwrap();
    let total: f32 = commands
        .iter()
        .map(|c| match c {
            Command::Pinch { delta, .. } => *delta,
            other => panic!("{other:?}"),
        })
        .sum();
    assert!((total - 0.5).abs() < 1e-4, "1.5x is a 0.5 change in total, got {total}");
    assert!(matches!(commands.first(), Some(Command::Pinch { phase: TouchPhase::Started, .. })));
    assert!(matches!(commands.last(), Some(Command::Pinch { phase: TouchPhase::Ended, .. })));

    let shrink: f32 = pinch(0.0, 0.0, 0.5)
        .unwrap()
        .iter()
        .map(|c| if let Command::Pinch { delta, .. } = c { *delta } else { 0.0 })
        .sum();
    assert!((shrink + 0.5).abs() < 1e-4);
    assert!(pinch(0.0, 0.0, 0.0).is_err());
    assert!(pinch(0.0, 0.0, -1.0).is_err());
}

#[test]
fn typing_is_one_text_command() {
    assert_eq!(type_text("héllo\nworld"), Command::Text("héllo\nworld".into()));
}

fn key_commands(spec: &str) -> Vec<Command> {
    key(spec).unwrap_or_else(|e| panic!("{spec}: {e}"))
}

#[test]
fn a_named_key_is_pressed_then_released() {
    for (spec, named) in [
        ("Return", NamedKey::Enter),
        ("enter", NamedKey::Enter),
        ("Escape", NamedKey::Escape),
        ("esc", NamedKey::Escape),
        ("Tab", NamedKey::Tab),
        ("BackSpace", NamedKey::Backspace),
        ("Delete", NamedKey::Delete),
        ("Up", NamedKey::ArrowUp),
        ("Down", NamedKey::ArrowDown),
        ("Left", NamedKey::ArrowLeft),
        ("Right", NamedKey::ArrowRight),
        ("Page_Up", NamedKey::PageUp),
        ("pagedown", NamedKey::PageDown),
        ("Home", NamedKey::Home),
        ("End", NamedKey::End),
    ] {
        assert_eq!(
            key_commands(spec),
            [
                Command::Key { key: named.clone(), action: KeyAction::Pressed, modifiers: Modifiers::default() },
                Command::Key { key: named, action: KeyAction::Released, modifiers: Modifiers::default() },
            ],
            "{spec}"
        );
    }
}

#[test]
fn modifiers_are_held_around_a_shortcut_letter() {
    let held = Modifiers { ctrl: true, shift: true, ..Modifiers::default() };
    assert_eq!(
        key_commands("ctrl+shift+A"),
        [
            Command::Key { key: NamedKey::Other("a".into()), action: KeyAction::Pressed, modifiers: held },
            Command::Key { key: NamedKey::Other("a".into()), action: KeyAction::Released, modifiers: held },
        ]
    );
    let meta = |spec| match &key_commands(spec)[0] {
        Command::Key { modifiers, .. } => *modifiers,
        other => panic!("{other:?}"),
    };
    assert!(meta("cmd+c").meta && meta("command+c").meta && meta("super+c").meta);
    assert!(meta("option+c").alt && meta("alt+c").alt);
    assert!(meta("control+c").ctrl);
}

#[test]
fn a_plain_character_is_typed_not_a_key() {
    assert_eq!(key_commands("a"), [Command::Text("a".into())]);
    assert_eq!(key_commands("Z"), [Command::Text("Z".into())]);
    assert_eq!(key_commands("space"), [Command::Text(" ".into())]);
    assert_eq!(key_commands("5"), [Command::Text("5".into())]);
}

#[test]
fn function_and_other_keys_use_the_name_the_platform_gives_them() {
    let other = |spec| match &key_commands(spec)[0] {
        Command::Key { key: NamedKey::Other(name), .. } => name.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(other("f5"), "F5");
    assert_eq!(other("F12"), "F12");
    assert_eq!(other("ctrl+space"), "Space");
    // Shift alone does not make a shortcut: a shifted space is still a space.
    assert_eq!(key_commands("shift+space"), [Command::Text(" ".into())]);
    assert_eq!(key_commands("shift+a"), [Command::Text("A".into())]);
    assert_eq!(other("Insert"), "Insert");
}

#[test]
fn a_key_spec_that_says_nothing_is_rejected() {
    for spec in ["", "  ", "ctrl+", "+a", "ctrl++a", "ctrl+shift"] {
        assert!(key(spec).is_err(), "{spec:?} must be rejected");
    }
}

#[test]
fn non_finite_and_absurd_coordinates_are_rejected() {
    assert!(click(f32::NAN, 0.0, PointerButton::Primary, 1).is_err());
    assert!(drag((0.0, f32::INFINITY), (1.0, 1.0)).is_err());
    assert!(tap(0.0, f32::NEG_INFINITY).is_err());
    assert!(swipe((f32::NAN, 0.0), (1.0, 1.0)).is_err());
    assert!(scroll(Some((f32::NAN, 0.0)), Direction::Up, 1.0).is_err());
    assert!(pinch(f32::NAN, 0.0, 1.5).is_err());
}

#[test]
fn a_zero_length_drag_or_swipe_is_still_a_valid_gesture() {
    assert!(drag((5.0, 5.0), (5.0, 5.0)).is_ok());
    assert!(swipe((5.0, 5.0), (5.0, 5.0)).is_ok());
}

#[test]
fn moving_the_mouse_glides_there_and_a_raw_move_is_left_to_the_wire() {
    assert_eq!(lines(&mouse_move(120.0, 48.0).unwrap()), ["glide 120 48"]);
    assert!(mouse_move(f32::NAN, 0.0).is_err());
}
