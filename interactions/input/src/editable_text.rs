mod controller;
mod editing;
mod layout;
mod platform;
mod value;

pub(crate) use controller::ControllerAttachment;
pub use controller::TextEditingController;
pub(crate) use layout::{
    DisplayText, EditableGeometry, EditableGeometryCache, EditableGeometryKey, vertical_target,
    wrap_visual_lines,
};
pub(crate) use platform::adapt_native_delta;
#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) use platform::byte_to_utf16;
pub use value::{TextEditingValue, TextRange};

#[cfg(test)]
mod tests {
    use aimer_text::TextSelection;
    use std::cell::Cell;
    use std::rc::Rc;

    use super::{TextEditingController, TextEditingValue, TextRange};

    #[test]
    fn value_normalizes_selection_and_composition_to_grapheme_boundaries() {
        let value = TextEditingValue::new(
            "a👩‍💻b",
            TextSelection::new(4, usize::MAX),
            Some(TextRange::new(2, 8)),
        );

        assert_eq!(value.text(), "a👩‍💻b");
        assert_eq!(value.selection(), TextSelection::new(1, 13));
        assert_eq!(value.composing(), Some(TextRange::new(1, 12)));
    }

    #[test]
    fn programmatic_changes_do_not_mutate_existing_snapshots() {
        let controller = TextEditingController::with_text("hello");
        let snapshot = controller.value();

        controller.set_text("hi👋");

        assert_eq!(snapshot.text(), "hello");
        assert_eq!(controller.value().text(), "hi👋");
        assert_eq!(controller.value().selection(), TextSelection::collapsed(2));
        assert_eq!(controller.revision(), 1);
    }

    #[test]
    fn each_attached_editor_is_notified_once_per_changed_transaction() {
        let controller = TextEditingController::with_text("a");
        let first_count = Rc::new(Cell::new(0));
        let second_count = Rc::new(Cell::new(0));
        let first_listener = first_count.clone();
        let first = controller.attach(move |_, _| first_listener.set(first_listener.get() + 1));
        let second_listener = second_count.clone();
        let _second = controller.attach(move |_, _| second_listener.set(second_listener.get() + 1));

        controller.set_text("b");
        controller.set_text("b");
        drop(first);
        controller.set_text("c");

        assert_eq!(first_count.get(), 1);
        assert_eq!(second_count.get(), 2);
    }

    #[test]
    fn selection_replacement_and_history_restore_complete_values_atomically() {
        let original = TextEditingValue::new(
            "A👩‍💻Z",
            TextSelection::new(1, 12),
            Some(TextRange::new(1, 12)),
        );
        let controller = TextEditingController::new();
        controller.set_value(original.clone());
        let notifications = Rc::new(Cell::new(0));
        let count = notifications.clone();
        let _attachment = controller.attach(move |_, _| count.set(count.get() + 1));

        assert!(controller.replace_selection_graphemes(1, 2, "你", None));
        assert_eq!(controller.value().text(), "A你Z");
        assert_eq!(controller.value().selection(), TextSelection::collapsed(4));
        assert_eq!(notifications.get(), 1);

        assert!(controller.undo());
        assert_eq!(controller.value(), original);
        assert_eq!(notifications.get(), 2);
        assert!(controller.redo());
        assert_eq!(controller.value().text(), "A你Z");
        assert_eq!(notifications.get(), 3);
    }

    #[test]
    fn deletion_never_splits_an_extended_grapheme_cluster() {
        let family = "👨‍👩‍👧‍👦";
        let controller = TextEditingController::with_text(format!("A{family}"));

        assert!(controller.delete_backward_graphemes(2, 2));
        assert_eq!(controller.value().text(), "A");
        assert_eq!(controller.value().selection(), TextSelection::collapsed(1));

        assert!(controller.undo());
        controller.set_value(TextEditingValue::new(
            format!("A{family}"),
            TextSelection::collapsed(1),
            None,
        ));
        assert!(controller.delete_forward_graphemes(1, 1));
        assert_eq!(controller.value().text(), "A");
    }

    #[test]
    fn horizontal_movement_respects_graphemes_and_directional_selection() {
        let controller = TextEditingController::with_text("A👩‍💻B");

        assert!(controller.move_left_graphemes(3, 3, false));
        assert_eq!(controller.value().selection(), TextSelection::collapsed(12));
        assert!(controller.move_left_graphemes(2, 2, false));
        assert_eq!(controller.value().selection(), TextSelection::collapsed(1));
        assert!(controller.move_right_graphemes(1, 1, true));
        assert_eq!(controller.value().selection(), TextSelection::new(1, 12));
        assert!(!controller.undo(), "caret movement is not editing history");
    }

    #[test]
    fn composition_updates_are_provisional_and_commit_as_one_transaction() {
        let original = TextEditingValue::with_text("Say ");
        let controller = TextEditingController::with_text("Say ");

        assert!(controller.update_composing_graphemes(4, 4, "ni"));
        assert_eq!(controller.value().text(), "Say ni");
        assert_eq!(controller.value().composing(), Some(TextRange::new(4, 6)));
        assert!(controller.update_composing_graphemes(4, 4, "nihao"));
        assert_eq!(controller.value().text(), "Say nihao");
        assert!(controller.commit_composing("你好", None));
        assert_eq!(controller.value().text(), "Say 你好");
        assert_eq!(controller.value().composing(), None);

        assert!(controller.undo());
        assert_eq!(controller.value(), original);
        assert!(
            !controller.undo(),
            "preedit updates did not add history entries"
        );
    }

    #[test]
    fn committed_max_length_counts_extended_graphemes() {
        let controller = TextEditingController::with_text("A🇺🇳");

        assert!(controller.commit_composing("👩‍💻BC", Some(4)));

        assert_eq!(controller.value().text(), "A🇺🇳👩‍💻B");
        assert_eq!(controller.value().selection(), TextSelection::collapsed(21));
    }
}
