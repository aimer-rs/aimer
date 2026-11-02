//! What a text field tells assistive technology about itself.

use aimer_accessibility::{NodeId, Role, SemanticAction, SemanticNode};

use super::RawTextField;

impl RawTextField {
    /// The node a screen reader sees for this field.
    ///
    /// The name is the field's hint, falling back to its prompt: the visible
    /// text a sighted person reads inside an empty field. A label drawn
    /// *beside* the field is a separate element the field cannot see.
    ///
    /// An obscured field publishes one bullet per character and never its text,
    /// so a screen reader does not read a password aloud. The node is a leaf:
    /// nothing the field draws inside itself is exposed separately.
    pub(crate) fn semantic_node(&self, id: NodeId) -> SemanticNode {
        let name = [&self.hint, &self.prompt]
            .into_iter()
            .find(|text| !text.is_empty());
        let value = if self.input_type.is_obscured() {
            "\u{2022}".repeat(self.controller.grapheme_count())
        } else {
            self.controller.text()
        };
        let mut node = SemanticNode::new(id, Role::TextField)
            .with_value(value)
            .enabled(self.enable)
            .focusable(self.enable)
            .leaf();
        if let Some(name) = name {
            node = node.with_name(&**name);
        }
        if self.enable {
            node = node.with_action(SemanticAction::Activate);
        }
        node
    }

    /// Activating a field gives it the keyboard.
    pub(crate) fn perform_semantic_action(&self, action: &SemanticAction) -> bool {
        if !self.enable || *action != SemanticAction::Activate {
            return false;
        }
        self.focus_node.request_focus();
        true
    }
}

#[cfg(test)]
mod tests {
    use aimer_accessibility::{NodeId, Role, SemanticAction, SemanticBehavior};
    use aimer_widget::FocusNode;

    use super::super::test_support::field_config;
    use super::RawTextField;
    use crate::input_field::caret::CaretBlink;
    use crate::TextEditingController;
    use crate::input::InputType;

    fn field(text: &str, configure: impl FnOnce(&mut RawTextField)) -> RawTextField {
        let controller = TextEditingController::with_initial(text);
        let mut field =
            RawTextField::new(field_config(controller), CaretBlink::new(), FocusNode::new());
        configure(&mut field);
        field
    }

    #[test]
    fn a_field_is_a_leaf_text_field_holding_its_text() {
        let field = field("ada@example.com", |field| field.hint = "Email".into());

        let node = field.semantic_node(NodeId::new(9));

        assert_eq!(node.id(), NodeId::new(9));
        assert_eq!(node.role(), Role::TextField);
        assert_eq!(node.name(), Some("Email"));
        assert_eq!(node.value(), Some("ada@example.com"));
        assert_eq!(node.behavior(), SemanticBehavior::Leaf);
        assert!(node.is_focusable());
        assert!(node.state().enabled());
    }

    #[test]
    fn the_prompt_names_a_field_that_has_no_hint() {
        let field = field("", |field| field.prompt = "Search".into());

        assert_eq!(field.semantic_node(NodeId::new(1)).name(), Some("Search"));
    }

    #[test]
    fn a_field_with_neither_hint_nor_prompt_is_unnamed() {
        let field = field("", |_| {});

        assert_eq!(field.semantic_node(NodeId::new(1)).name(), None);
    }

    #[test]
    fn a_password_is_never_published() {
        let field = field("hunter2", |field| field.input_type = InputType::Password);

        let node = field.semantic_node(NodeId::new(1));

        assert_eq!(node.value(), Some("\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}"));
        assert!(!node.value().unwrap().contains("hunter2"));
    }

    #[test]
    fn a_disabled_field_is_dimmed_and_cannot_be_activated() {
        let field = field("", |field| field.enable = false);

        let node = field.semantic_node(NodeId::new(1));

        assert!(!node.state().enabled());
        assert!(!node.is_focusable());
        assert!(node.actions().is_empty());
        assert!(!field.perform_semantic_action(&SemanticAction::Activate));
    }

    #[test]
    fn activating_an_enabled_field_handles_the_request() {
        let field = field("", |_| {});

        assert!(field.perform_semantic_action(&SemanticAction::Activate));
        assert!(!field.perform_semantic_action(&SemanticAction::Dismiss));
    }
}
