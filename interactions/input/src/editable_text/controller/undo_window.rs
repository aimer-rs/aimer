use std::time::Duration;

use aimer_animation::AnimInstant;

use super::TextEditingController;

/// A controller whose clock is held, so that when each edit is made is what a test says.
struct Clocked {
    controller: TextEditingController,
    start: AnimInstant,
}

impl Clocked {
    fn new(text: &str, window: Duration) -> Self {
        let controller = TextEditingController::with_text(text);
        controller.set_undo_merge_window(window);
        Self { controller, start: AnimInstant::now() }
    }

    /// Types `typed` at the end of the text, `millis` after the start.
    fn type_at(&self, millis: u64, typed: &str) {
        self.controller.core.test_now.set(Some(self.start + Duration::from_millis(millis)));
        let end = self.controller.value().len();
        assert!(self.controller.replace_range(end..end, typed));
    }

    fn text(&self) -> String {
        self.controller.value().text().to_string()
    }
}

const SECOND: Duration = Duration::from_secs(1);

#[test]
fn every_edit_is_an_undo_of_its_own_unless_a_window_is_asked_for() {
    let typing = Clocked::new("", Duration::ZERO);
    for (millis, typed) in [(0, "a"), (10, "b"), (20, "c")] {
        typing.type_at(millis, typed);
    }
    for expected in ["ab", "a", ""] {
        assert!(typing.controller.undo());
        assert_eq!(typing.text(), expected);
    }
    assert!(!typing.controller.undo());
}

#[test]
fn edits_that_follow_each_other_within_the_window_are_one_undo() {
    let typing = Clocked::new("fn ", SECOND);
    for (millis, typed) in [(0, "m"), (300, "a"), (600, "i"), (900, "n")] {
        typing.type_at(millis, typed);
    }
    assert_eq!(typing.text(), "fn main");
    assert!(typing.controller.undo());
    assert_eq!(typing.text(), "fn ", "all of it, as it was before the first");
    assert!(!typing.controller.undo(), "and there is nothing else");
    assert!(typing.controller.redo());
    assert_eq!(typing.text(), "fn main", "and one redo gives it back");
}

#[test]
fn the_window_is_from_the_edit_before_and_not_from_the_first_of_them() {
    let typing = Clocked::new("", SECOND);
    // Each is less than a second from the one before, and the last is two seconds from the first.
    for (millis, typed) in [(0, "a"), (800, "b"), (1600, "c")] {
        typing.type_at(millis, typed);
    }
    assert!(typing.controller.undo());
    assert_eq!(typing.text(), "");
    assert!(!typing.controller.undo());
}

#[test]
fn a_pause_longer_than_the_window_ends_the_undo_and_the_next_edit_begins_another() {
    let typing = Clocked::new("", SECOND);
    for (millis, typed) in [(0, "a"), (200, "b"), (1500, "c"), (1700, "d")] {
        typing.type_at(millis, typed);
    }
    assert_eq!(typing.text(), "abcd");
    assert!(typing.controller.undo());
    assert_eq!(typing.text(), "ab", "what was typed after the pause");
    assert!(typing.controller.undo());
    assert_eq!(typing.text(), "", "and what was typed before it");
    assert!(!typing.controller.undo());
    // The window is the window's, to the edge.
    let edge = Clocked::new("", SECOND);
    edge.type_at(0, "a");
    edge.type_at(1000, "b");
    edge.type_at(2001, "c");
    assert!(edge.controller.undo());
    assert_eq!(edge.text(), "ab", "a gap of exactly the window is within it, and a little more is not");
}

#[test]
fn an_undo_ends_the_burst_so_that_what_is_typed_next_is_not_taken_into_what_was_undone() {
    let typing = Clocked::new("", SECOND);
    typing.type_at(0, "a");
    typing.type_at(100, "b");
    assert!(typing.controller.undo());
    assert_eq!(typing.text(), "");
    typing.type_at(150, "x");
    typing.type_at(200, "y");
    assert_eq!(typing.text(), "xy");
    assert!(typing.controller.undo());
    assert_eq!(typing.text(), "", "the new burst is one undo of its own");
    assert!(!typing.controller.undo(), "nothing was lost and nothing was doubled");
    assert!(typing.controller.redo());
    assert_eq!(typing.text(), "xy", "the redo that was there is the new one");
}

#[test]
fn a_text_set_from_outside_is_never_taken_into_the_typing_around_it() {
    use super::TextEditingValue;
    let typing = Clocked::new("", SECOND);
    typing.type_at(0, "a");
    typing.controller.core.test_now.set(Some(typing.start + Duration::from_millis(10)));
    typing.controller.set_value(TextEditingValue::with_text("zz"));
    typing.type_at(20, "b");
    assert_eq!(typing.text(), "zzb");
    for expected in ["zz", "a", ""] {
        assert!(typing.controller.undo());
        assert_eq!(typing.text(), expected);
    }
}

#[test]
fn an_edit_of_several_ranges_and_a_move_of_the_caret_are_within_the_window_like_any_other() {
    let typing = Clocked::new("a", SECOND);
    typing.type_at(0, "b");
    typing.controller.core.test_now.set(Some(typing.start + Duration::from_millis(100)));
    typing.controller.select_bytes(0, 0);
    typing.controller.replace_ranges(&[(0..0, "<"), (2..2, ">")], 0);
    assert_eq!(typing.text(), "<ab>");
    assert!(typing.controller.undo());
    assert_eq!(typing.text(), "a", "the time is all that parts one undo from the next");
    assert!(!typing.controller.undo());
}

#[test]
fn a_window_that_is_asked_for_again_is_the_one_that_counts_from_then_on() {
    let typing = Clocked::new("", SECOND);
    typing.type_at(0, "a");
    typing.controller.set_undo_merge_window(Duration::ZERO);
    typing.type_at(10, "b");
    typing.type_at(20, "c");
    for expected in ["ab", "a", ""] {
        assert!(typing.controller.undo());
        assert_eq!(typing.text(), expected);
    }
}
