use std::path::PathBuf;

use crate::model::{KeyAction, Modifiers, NamedKey, PointerButton, ScrollDeltaKind, TouchPhase};

use super::*;
use crate::command::{BackStep, EditShortcut};

fn ctrl() -> Modifiers {
    Modifiers { ctrl: true, ..Modifiers::default() }
}

fn roundtrip(command: Command) {
    let line = command.to_string();
    assert!(!line.contains('\n') && !line.contains('\r'), "one command is one line: {line:?}");
    assert_eq!(line.parse::<Command>(), Ok(command), "line was {line:?}");
}

#[test]
fn every_command_survives_a_round_trip() {
    let none = Modifiers::default();
    roundtrip(Command::PointerMove { x: 1.5, y: -2.25, modifiers: none.clone() });
    roundtrip(Command::PointerMove { x: 1.0, y: 2.0, modifiers: ctrl() });
    roundtrip(Command::PointerDown { x: 10.0, y: 20.0, button: PointerButton::Secondary, modifiers: none.clone() });
    roundtrip(Command::PointerUp { x: 0.1, y: 0.2, button: PointerButton::Other(7), modifiers: ctrl() });
    roundtrip(Command::PointerUp { x: 0.0, y: 0.0, button: PointerButton::Middle, modifiers: none.clone() });
    roundtrip(Command::PointerDownHere { button: PointerButton::Primary, modifiers: none.clone() });
    roundtrip(Command::PointerUpHere { button: PointerButton::Other(7), modifiers: ctrl() });
    roundtrip(Command::PointerLeave);
    roundtrip(Command::Settle);
    roundtrip(Command::Typing { text: "hello, world".into(), millis: None });
    roundtrip(Command::Typing { text: "héllo 世界\nsecond \\ ".into(), millis: Some(900) });
    roundtrip(Command::Typing { text: String::new(), millis: None });
    roundtrip(Command::Screenshot { width: None, jpeg: None });
    roundtrip(Command::Screenshot { width: Some(1280), jpeg: None });
    roundtrip(Command::Screenshot { width: Some(0), jpeg: Some(75) });
    roundtrip(Command::TrackpadScroll { dx: 0.0, dy: -120.5, millis: None, modifiers: none });
    roundtrip(Command::TrackpadScroll { dx: 40.0, dy: 8.0, millis: Some(600), modifiers: ctrl() });
    roundtrip(Command::PointerGlide { x: 10.0, y: -4.5, millis: None, modifiers: none });
    roundtrip(Command::PointerGlide { x: 1.0, y: 2.0, millis: Some(350), modifiers: ctrl() });
    roundtrip(Command::Scroll {
        dx: 0.0,
        dy: -120.5,
        kind: ScrollDeltaKind::Pixel,
        phase: TouchPhase::Moved,
        modifiers: none.clone(),
    });
    roundtrip(Command::Scroll {
        dx: 1.0,
        dy: -3.0,
        kind: ScrollDeltaKind::Line,
        phase: TouchPhase::Started,
        modifiers: ctrl(),
    });
    for phase in [TouchPhase::Started, TouchPhase::Moved, TouchPhase::Ended] {
        roundtrip(Command::Touch { phase, id: 3, x: 4.5, y: 6.0 });
    }
    roundtrip(Command::TouchCancel);
    for phase in [TouchPhase::Started, TouchPhase::Moved, TouchPhase::Ended, TouchPhase::Cancelled] {
        roundtrip(Command::Pinch { x: 1.0, y: 2.0, delta: -0.25, phase });
        roundtrip(Command::Rotate { x: 1.0, y: 2.0, delta: 1.5, phase });
        roundtrip(Command::Pan { x: 1.0, y: 2.0, dx: -3.0, dy: 4.0, phase });
    }
    roundtrip(Command::SmartZoom { x: 7.0, y: 8.0 });
    roundtrip(Command::Key {
        key: NamedKey::PageDown,
        action: KeyAction::Repeat,
        modifiers: Modifiers { ctrl: true, shift: false, alt: true, meta: false },
    });
    roundtrip(Command::Key {
        key: NamedKey::Other("F5 key\\x".into()),
        action: KeyAction::Released,
        modifiers: none,
    });
    roundtrip(Command::Text(String::new()));
    roundtrip(Command::Text("héllo 世界\nsecond\r line \\ done ".into()));
    roundtrip(Command::Preedit { text: "ni hao".into(), cursor: Some((0, 2)) });
    roundtrip(Command::Preedit { text: "你好".into(), cursor: Some((3, 6)) });
    roundtrip(Command::Preedit { text: "x".into(), cursor: None });
    roundtrip(Command::Preedit { text: String::new(), cursor: None });
    roundtrip(Command::SoftBackspace);
    for signal in [BackStep::Started, BackStep::Progressed(0.5), BackStep::Invoked, BackStep::Cancelled] {
        roundtrip(Command::Back(signal));
    }
    roundtrip(Command::Link("aimer://item/7?a=b c".into()));
    for shortcut in [
        EditShortcut::Undo,
        EditShortcut::Redo,
        EditShortcut::Cut,
        EditShortcut::Copy,
        EditShortcut::Paste,
        EditShortcut::SelectAll,
    ] {
        roundtrip(Command::Menu(shortcut));
    }
    roundtrip(Command::FileHover(PathBuf::from("/tmp/a b\\c.png")));
    roundtrip(Command::FileHoverCancel);
    roundtrip(Command::FileDrop(PathBuf::from("/tmp/a.png")));
    roundtrip(Command::Quit);
}

#[test]
fn the_original_lines_keep_their_meaning() {
    let none = Modifiers::default();
    assert_eq!(
        "down 3 4".parse(),
        Ok(Command::PointerDown { x: 3.0, y: 4.0, button: PointerButton::Primary, modifiers: none.clone() })
    );
    assert_eq!("move 1 2".parse(), Ok(Command::PointerMove { x: 1.0, y: 2.0, modifiers: none.clone() }));
    assert_eq!(
        "scroll 0 -9".parse(),
        Ok(Command::Scroll {
            dx: 0.0,
            dy: -9.0,
            kind: ScrollDeltaKind::Pixel,
            phase: TouchPhase::Moved,
            modifiers: none,
        })
    );
}

#[test]
fn a_glide_takes_an_optional_duration_in_milliseconds() {
    let none = Modifiers::default();
    assert_eq!("glide 5 6".parse(), Ok(Command::PointerGlide { x: 5.0, y: 6.0, millis: None, modifiers: none }));
    assert_eq!("glide 5 6 250".parse(), Ok(Command::PointerGlide { x: 5.0, y: 6.0, millis: Some(250), modifiers: none }));
    assert_eq!(
        "glide 5 6 250 mods=ctrl".parse(),
        Ok(Command::PointerGlide { x: 5.0, y: 6.0, millis: Some(250), modifiers: ctrl() })
    );
    assert_eq!("glide 5 6 mods=ctrl".parse(), Ok(Command::PointerGlide { x: 5.0, y: 6.0, millis: None, modifiers: ctrl() }));
}

#[test]
fn a_screenshot_takes_an_optional_width_and_jpeg_quality_in_any_order() {
    assert_eq!("screenshot".parse(), Ok(Command::Screenshot { width: None, jpeg: None }));
    assert_eq!("screenshot width=800".parse(), Ok(Command::Screenshot { width: Some(800), jpeg: None }));
    assert_eq!(
        "screenshot jpeg=70 width=640".parse(),
        Ok(Command::Screenshot { width: Some(640), jpeg: Some(70) })
    );
    let bad = |line: &str| line.parse::<Command>().unwrap_err();
    assert_eq!(bad("screenshot width=wide"), ParseError::Invalid("width"));
    assert_eq!(bad("screenshot width=100000"), ParseError::Invalid("width"));
    assert_eq!(bad("screenshot jpeg=0"), ParseError::Invalid("jpeg"));
    assert_eq!(bad("screenshot jpeg=101"), ParseError::Invalid("jpeg"));
    assert_eq!(bad("screenshot colour=red"), ParseError::Invalid("option"));
    assert_eq!(bad("screenshot width=1 width=2"), ParseError::Invalid("option"));
    assert_eq!(bad("screenshot 800"), ParseError::Invalid("option"));
}

#[test]
fn a_trackpad_scroll_takes_an_optional_duration_like_a_glide() {
    let none = Modifiers::default();
    assert_eq!(
        "trackpad 0 -300".parse(),
        Ok(Command::TrackpadScroll { dx: 0.0, dy: -300.0, millis: None, modifiers: none })
    );
    assert_eq!(
        "trackpad 0 -300 800 mods=ctrl".parse(),
        Ok(Command::TrackpadScroll { dx: 0.0, dy: -300.0, millis: Some(800), modifiers: ctrl() })
    );
    let bad = |line: &str| line.parse::<Command>().unwrap_err();
    assert_eq!(bad("trackpad 1"), ParseError::Missing("dy"));
    assert_eq!(bad("trackpad 1 2 0"), ParseError::Invalid("duration"));
    assert_eq!(bad("trackpad 1 2 3001"), ParseError::Invalid("duration"));
    assert!("trackpad NaN 2".parse::<Command>().is_err());
    assert!("trackpad 1 inf".parse::<Command>().is_err());
}

#[test]
fn a_glide_that_cannot_be_walked_is_rejected() {
    let bad = |line: &str| line.parse::<Command>().unwrap_err();
    assert_eq!(bad("glide 1"), ParseError::Missing("y"));
    assert_eq!(bad("glide 1 2 0"), ParseError::Invalid("duration"));
    assert_eq!(bad("glide 1 2 3001"), ParseError::Invalid("duration"));
    assert_eq!(bad("glide 1 2 soon"), ParseError::Invalid("duration"));
    assert_eq!(bad("glide 1 2 -5"), ParseError::Invalid("duration"));
    assert!("glide NaN 2".parse::<Command>().is_err());
}

#[test]
fn options_follow_the_positional_arguments_in_any_order() {
    assert_eq!(
        "down 1 2 secondary mods=ctrl+shift".parse(),
        Ok(Command::PointerDown {
            x: 1.0,
            y: 2.0,
            button: PointerButton::Secondary,
            modifiers: Modifiers { ctrl: true, shift: true, ..Modifiers::default() },
        })
    );
    assert_eq!(
        "scroll 0 -2 phase=start kind=line mods=ctrl".parse(),
        Ok(Command::Scroll {
            dx: 0.0,
            dy: -2.0,
            kind: ScrollDeltaKind::Line,
            phase: TouchPhase::Started,
            modifiers: ctrl(),
        })
    );
}

#[test]
fn malformed_lines_are_rejected_with_a_reason() {
    let bad = |line: &str| line.parse::<Command>().unwrap_err();
    assert_eq!(bad(""), ParseError::Empty);
    assert_eq!(bad("   "), ParseError::Empty);
    assert_eq!(bad("teleport 1 2"), ParseError::UnknownCommand);
    assert_eq!(bad("move 1"), ParseError::Missing("y"));
    assert_eq!(bad("move"), ParseError::Missing("x"));
    assert_eq!(bad("move 1 2 3"), ParseError::Invalid("option"));
    assert_eq!(bad("move one 2"), ParseError::Invalid("x"));
    assert_eq!(bad("down 1 2 left"), ParseError::Invalid("button"));
    assert_eq!(bad("key press - hyper"), ParseError::Invalid("key"));
    assert_eq!(bad("key tap - enter"), ParseError::Invalid("action"));
    assert_eq!(bad("key press ctrl+fn enter"), ParseError::Invalid("modifiers"));
    assert_eq!(bad("key press - other:"), ParseError::Invalid("key"));
    assert_eq!(bad("text bad\\q"), ParseError::Invalid("escape"));
    assert_eq!(bad("text trailing\\"), ParseError::Invalid("escape"));
    assert_eq!(bad("text"), ParseError::Missing("text"));
    assert_eq!(bad("leave now"), ParseError::TooMany);
    // Options that do not belong to the command, or are said twice.
    assert_eq!(bad("move 1 2 kind=line"), ParseError::Invalid("option"));
    assert_eq!(bad("down 1 2 phase=start"), ParseError::Invalid("option"));
    assert_eq!(bad("move 1 2 mods=ctrl mods=alt"), ParseError::Invalid("option"));
    assert_eq!(bad("scroll 1 2 kind=page"), ParseError::Invalid("kind"));
    assert_eq!(bad("scroll 1 2 phase=began"), ParseError::Invalid("phase"));
    assert_eq!(bad("move 1 2 mods=hyper"), ParseError::Invalid("modifiers"));
    // The new commands.
    assert_eq!(bad("touch start 1 2"), ParseError::Missing("y"));
    assert_eq!(bad("touch cancel 1"), ParseError::TooMany);
    assert_eq!(bad("touch drag 1 2 3"), ParseError::Invalid("phase"));
    assert_eq!(bad("touch start x 2 3"), ParseError::Invalid("id"));
    assert_eq!(bad("pinch 1 2 0.1"), ParseError::Missing("phase"));
    assert_eq!(bad("pan 1 2 3 4"), ParseError::Missing("phase"));
    assert_eq!(bad("smartzoom 1"), ParseError::Missing("y"));
    assert_eq!(bad("back"), ParseError::Missing("signal"));
    assert_eq!(bad("back swipe"), ParseError::Invalid("signal"));
    assert_eq!(bad("back progress"), ParseError::Missing("progress"));
    assert_eq!(bad("back progress 1.5"), ParseError::Invalid("progress"));
    assert_eq!(bad("back progress -0.1"), ParseError::Invalid("progress"));
    assert_eq!(bad("menu"), ParseError::Missing("shortcut"));
    assert_eq!(bad("menu print"), ParseError::Invalid("shortcut"));
    assert_eq!(bad("link"), ParseError::Missing("url"));
    assert_eq!(bad("link "), ParseError::Invalid("url"));
    assert_eq!(bad("file-hover"), ParseError::Missing("path"));
    assert_eq!(bad("file-drop "), ParseError::Invalid("path"));
    assert_eq!(bad("file-hover-cancel now"), ParseError::TooMany);
    assert_eq!(bad("quit now"), ParseError::TooMany);
    assert_eq!(bad("settle now"), ParseError::TooMany);
    assert_eq!(bad("soft-backspace 1"), ParseError::TooMany);
}

#[test]
fn a_preedit_range_must_fall_inside_the_text_on_character_boundaries() {
    assert_eq!(bad_cursor("preedit 0,9 ab"), ParseError::Invalid("cursor"));
    assert_eq!(bad_cursor("preedit 2,1 ab"), ParseError::Invalid("cursor"));
    // `你` is three bytes: offset 1 is inside it.
    assert_eq!(bad_cursor("preedit 0,1 你"), ParseError::Invalid("cursor"));
    assert_eq!(bad_cursor("preedit a,b ab"), ParseError::Invalid("cursor"));
    assert_eq!("preedit 0,3 你".parse(), Ok(Command::Preedit { text: "你".into(), cursor: Some((0, 3)) }));
    assert_eq!("preedit -".parse(), Ok(Command::Preedit { text: String::new(), cursor: None }));
}

fn bad_cursor(line: &str) -> ParseError {
    line.parse::<Command>().unwrap_err()
}

#[test]
fn non_finite_numbers_never_reach_the_application() {
    for line in [
        "move NaN 1",
        "move 1 inf",
        "move -inf 0",
        "scroll 1 NaN",
        "down inf 1",
        "touch start 1 NaN 2",
        "pinch 1 2 NaN start",
        "rotate inf 2 0.1 start",
        "pan 1 2 NaN 1 start",
        "smartzoom 1 inf",
        "back progress NaN",
    ] {
        assert!(line.parse::<Command>().is_err(), "{line} must be rejected");
    }
}

#[test]
fn typing_takes_a_time_or_a_dash_and_then_the_text_as_it_is() {
    assert_eq!("typing - hello world".parse(), Ok(Command::Typing { text: "hello world".into(), millis: None }));
    assert_eq!("typing 900 12 apples".parse(), Ok(Command::Typing { text: "12 apples".into(), millis: Some(900) }));
    let bad = |line: &str| line.parse::<Command>().unwrap_err();
    assert_eq!(bad("typing"), ParseError::Missing("duration"));
    assert_eq!(bad("typing 12apples"), ParseError::Invalid("duration"));
    assert_eq!(bad("typing 0 x"), ParseError::Invalid("duration"));
    assert_eq!(bad("typing 3001 x"), ParseError::Invalid("duration"));
    assert_eq!(bad("typing 500"), ParseError::Missing("text"));
}

#[test]
fn a_press_or_release_without_a_point_is_where_the_mouse_is() {
    let none = Modifiers::default();
    assert_eq!("down".parse(), Ok(Command::PointerDownHere { button: PointerButton::Primary, modifiers: none.clone() }));
    assert_eq!("up secondary".parse(), Ok(Command::PointerUpHere { button: PointerButton::Secondary, modifiers: none }));
    assert_eq!(
        "down middle mods=ctrl".parse(),
        Ok(Command::PointerDownHere { button: PointerButton::Middle, modifiers: ctrl() })
    );
    assert_eq!(
        "down mods=ctrl".parse(),
        Ok(Command::PointerDownHere { button: PointerButton::Primary, modifiers: ctrl() })
    );
    // A point is both numbers or neither.
    let bad = |line: &str| line.parse::<Command>().unwrap_err();
    assert_eq!(bad("down 5"), ParseError::Missing("y"));
    assert_eq!(bad("up 5 nope"), ParseError::Invalid("y"));
}
