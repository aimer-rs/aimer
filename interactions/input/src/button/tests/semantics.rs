use aimer_accessibility::{Role, SemanticAction};
use aimer_text::Text;
use aimer_widget::semantics::{collect_semantic_tree, perform_semantic_action};

use super::retained::button_context;
use super::*;

/// The button published for `button`, as a screen reader would see it.
async fn published(button: Button<aimer_widget::AnyWidget>) -> (AnyElement, aimer_accessibility::SemanticSnapshot) {
    let ctx = button_context();
    let root = button.to_element(&ctx);
    root.update(&ctx);
    let snapshot = collect_semantic_tree(root.as_ref(), &mut |_| None)
        .snapshot()
        .unwrap();
    (root, snapshot)
}

fn label(text: &'static str) -> aimer_widget::AnyWidget {
    Text::new(text).boxed()
}

#[tokio::test]
async fn a_button_is_one_named_button_made_of_its_text() {
    let (_root, snapshot) = published(Button::new().on_press(|| {}).child(label("Submit"))).await;

    let nodes: Vec<_> = snapshot.root().children().iter().collect();
    assert_eq!(nodes.len(), 1, "the text must fold into the button, not sit beside it");
    assert_eq!(nodes[0].role(), Role::Button);
    assert_eq!(nodes[0].name(), Some("Submit"));
    assert_eq!(nodes[0].actions(), [SemanticAction::Activate]);
    assert!(nodes[0].children().is_empty());
}

#[tokio::test]
async fn activating_the_published_button_runs_its_press_callback() {
    let presses = Rc::new(Cell::new(0));
    let counter = presses.clone();
    let (root, snapshot) = published(
        Button::new()
            .on_press(move || counter.set(counter.get() + 1))
            .child(label("Submit")),
    )
    .await;
    let id = snapshot.root().children()[0].id();

    assert!(perform_semantic_action(root.as_ref(), id, &SemanticAction::Activate));

    assert_eq!(presses.get(), 1);
}

#[tokio::test]
async fn a_disabled_button_is_dimmed_and_refuses_activation() {
    let presses = Rc::new(Cell::new(0));
    let counter = presses.clone();
    let (root, snapshot) = published(
        Button::new()
            .disabled(true)
            .on_press(move || counter.set(counter.get() + 1))
            .child(label("Submit")),
    )
    .await;
    let button = &snapshot.root().children()[0];

    assert_eq!(button.role(), Role::Button);
    assert!(!button.state().enabled());
    assert!(button.actions().is_empty());
    assert!(!perform_semantic_action(root.as_ref(), button.id(), &SemanticAction::Activate));
    assert_eq!(presses.get(), 0);
}
