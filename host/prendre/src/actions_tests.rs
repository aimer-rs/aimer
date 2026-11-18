use super::*;
use crate::Command;
use crate::model::{KeyAction, Modifiers, NamedKey, PointerButton, TouchPhase};

fn lines(commands: &[Command]) -> Vec<String> {
    commands.iter().map(ToString::to_string).collect()
}

#[test]
fn a_click_glides_to_the_point_then_presses_and_releases_there() {
    assert_eq!(
        lines(&click(Some((10.0, 20.0)), PointerButton::Primary, 1).unwrap()),
        ["glide 10 20", "down 10 20 primary", "up 10 20 primary"]
    );
    assert_eq!(
        lines(&click(Some((1.0, 2.0)), PointerButton::Secondary, 1).unwrap())[1],
        "down 1 2 secondary"
    );
}

#[test]
fn a_double_and_a_triple_click_are_that_many_presses_after_one_glide() {
    for count in [2u8, 3] {
        let commands = click(Some((5.0, 5.0)), PointerButton::Primary, count).unwrap();
        assert_eq!(commands.len(), 1 + 2 * usize::from(count));
        assert!(matches!(commands[0], Command::PointerGlide { .. }));
        let downs = commands.iter().filter(|c| matches!(c, Command::PointerDown { .. })).count();
        assert_eq!(downs, usize::from(count));
    }
    assert!(click(Some((0.0, 0.0)), PointerButton::Primary, 0).is_err(), "no clicks is not a click");
    assert!(click(Some((0.0, 0.0)), PointerButton::Primary, 4).is_err(), "more than a triple is a mistake");
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
        [Command::TrackpadScroll { dx, dy, .. }] => (*dx, *dy),
        other => panic!("{other:?}"),
    };
    assert_eq!(sign(Direction::Down), (0.0, -120.0), "three steps of 40 pixels");
    assert_eq!(sign(Direction::Up), (0.0, 120.0));
    assert_eq!(sign(Direction::Right), (-120.0, 0.0));
    assert_eq!(sign(Direction::Left), (120.0, 0.0));
}

#[test]
fn a_scroll_with_a_position_glides_there_and_waits_for_the_mouse_before_scrolling() {
    assert_eq!(
        lines(&scroll(Some((40.0, 50.0)), Direction::Down, 1.0).unwrap()),
        ["glide 40 50", "settle", "trackpad 0 -40"]
    );
}

#[test]
fn a_scroll_without_a_position_scrolls_where_the_mouse_already_is() {
    assert_eq!(lines(&scroll(None, Direction::Up, 2.0).unwrap()), ["trackpad 0 80"]);
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
    assert!(click(Some((f32::NAN, 0.0)), PointerButton::Primary, 1).is_err());
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

#[test]
fn typing_is_the_paced_way_to_enter_text_and_type_text_the_instant_one() {
    assert_eq!(typing("hi"), Command::Typing { text: "hi".into(), millis: None });
    assert_eq!(type_text("hi"), Command::Text("hi".into()));
}

#[test]
fn a_click_without_a_point_presses_and_releases_where_the_mouse_already_is() {
    let commands = click(None, PointerButton::Primary, 1).unwrap();
    assert_eq!(lines(&commands), ["down primary", "up primary"], "no glide: it is already there");
    assert_eq!(lines(&click(None, PointerButton::Secondary, 2).unwrap()), ["down secondary", "up secondary", "down secondary", "up secondary"]);
    assert!(click(None, PointerButton::Primary, 0).is_err());
    assert!(click(None, PointerButton::Primary, 4).is_err());
}

#[test]
fn a_resize_is_one_command_that_drags_the_edge_by_the_pointers_travel() {
    use crate::model::WindowEdge;
    assert_eq!(lines(&resize(WindowEdge::West, -80.0, 0.0, None).unwrap()), ["resize w -80 0"]);
    assert_eq!(lines(&resize(WindowEdge::SouthEast, 40.0, 25.0, Some(600)).unwrap()), ["resize se 40 25 600"]);
}

#[test]
fn a_resize_that_cannot_mean_what_it_says_is_an_error() {
    use crate::model::WindowEdge;
    assert!(resize(WindowEdge::East, 10.0, 5.0, None).is_err(), "an east edge does not move vertically");
    assert!(resize(WindowEdge::North, 5.0, 10.0, None).is_err(), "a north edge does not move horizontally");
    assert!(resize(WindowEdge::East, f32::NAN, 0.0, None).is_err());
    assert!(resize(WindowEdge::East, 1.0, 0.0, Some(0)).is_err());
    assert!(resize(WindowEdge::East, 1.0, 0.0, Some(3001)).is_err());
}

#[test]
fn a_window_action_is_one_command() {
    use crate::model::WindowAction;
    assert_eq!(lines(&window(WindowAction::Minimize)), ["window minimize"]);
}


// ── Modifier keys held through a gesture ────────────────────────────────────────

fn held(ctrl: bool, shift: bool, alt: bool, meta: bool) -> Modifiers {
    Modifiers { ctrl, shift, alt, meta }
}

#[test]
fn a_click_holds_its_modifiers_through_every_step_of_it() {
    let shift = held(false, true, false, false);
    assert_eq!(
        lines(&click_with(Some((10.0, 20.0)), PointerButton::Primary, 1, shift).unwrap()),
        ["glide 10 20 mods=shift", "down 10 20 primary mods=shift", "up 10 20 primary mods=shift"]
    );
    assert_eq!(
        lines(&click_with(None, PointerButton::Secondary, 2, held(true, false, false, true)).unwrap()),
        [
            "down secondary mods=ctrl+meta",
            "up secondary mods=ctrl+meta",
            "down secondary mods=ctrl+meta",
            "up secondary mods=ctrl+meta"
        ]
    );
    // Without any, nothing is added to the line: it is the click there always was.
    assert_eq!(
        lines(&click_with(Some((10.0, 20.0)), PointerButton::Primary, 1, Modifiers::default()).unwrap()),
        lines(&click(Some((10.0, 20.0)), PointerButton::Primary, 1).unwrap())
    );
}

#[test]
fn all_four_modifiers_can_be_held_at_once() {
    let all = held(true, true, true, true);
    let commands = click_with(Some((1.0, 2.0)), PointerButton::Primary, 1, all).unwrap();
    assert_eq!(lines(&commands)[1], "down 1 2 primary mods=ctrl+shift+alt+meta");
    // And what is built is what the wire says: each line reads back as the command it came from.
    for command in &commands {
        assert_eq!(&command.to_string().parse::<Command>().unwrap(), command);
    }
}

#[test]
fn drag_move_and_scroll_hold_their_modifiers_too() {
    let alt = held(false, false, true, false);
    assert_eq!(
        lines(&drag_with((10.0, 10.0), (110.0, 60.0), alt).unwrap()),
        ["glide 10 10 mods=alt", "down 10 10 primary mods=alt", "glide 110 60 mods=alt", "up 110 60 primary mods=alt"]
    );
    assert_eq!(lines(&mouse_move_with(120.0, 48.0, alt).unwrap()), ["glide 120 48 mods=alt"]);
    let ctrl_shift = held(true, true, false, false);
    assert_eq!(lines(&scroll_with(None, Direction::Up, 2.0, ctrl_shift).unwrap()), ["trackpad 0 80 mods=ctrl+shift"]);
    let with_point = lines(&scroll_with(Some((40.0, 50.0)), Direction::Down, 1.0, ctrl_shift).unwrap());
    assert_eq!(with_point[0], "glide 40 50 mods=ctrl+shift");
    assert!(with_point.last().unwrap().ends_with("mods=ctrl+shift"));
}

#[test]
fn modifiers_are_read_from_one_name_or_several_in_any_case_and_order() {
    let parse = |text: &str| text.parse::<Modifiers>().unwrap();
    assert_eq!(parse("shift"), held(false, true, false, false));
    assert_eq!(parse("Ctrl+Shift"), held(true, true, false, false));
    assert_eq!(parse("shift+ctrl"), parse("ctrl+shift"), "the order does not matter");
    assert_eq!(parse("cmd,option"), held(false, false, true, true));
    assert_eq!(parse(" control + super + win "), held(true, false, false, true));
    assert_eq!(parse("shift+shift"), parse("shift"), "naming a key twice is naming it once");
    for none in ["", "-", "none", "NONE", "  "] {
        assert_eq!(parse(none), Modifiers::default(), "{none:?}");
    }
}

#[test]
fn something_that_is_not_a_modifier_is_refused_by_name() {
    for bad in ["hyper", "shift+hyper", "shift+", "+shift", "ctrl++alt", "a"] {
        assert!(bad.parse::<Modifiers>().is_err(), "{bad:?}");
    }
    let message = "shift+hyper".parse::<Modifiers>().unwrap_err().to_string();
    assert!(message.contains("hyper") && message.contains("ctrl, shift, alt or meta"), "{message}");
}

#[test]
fn a_list_of_names_holds_every_key_it_names() {
    assert_eq!(modifiers_of::<&str>(&[]).unwrap(), Modifiers::default());
    assert_eq!(modifiers_of(&["shift"]).unwrap(), held(false, true, false, false));
    assert_eq!(modifiers_of(&["shift", "ctrl"]).unwrap(), held(true, true, false, false));
    assert_eq!(modifiers_of(&["ctrl+alt", "meta", "shift".to_owned().as_str()]).unwrap(), held(true, true, true, true));
    assert!(modifiers_of(&["shift", "hyper"]).is_err());
}
