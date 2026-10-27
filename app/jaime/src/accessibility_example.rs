//! A small, platform-neutral semantics example for the Jaime showcase.
//!
//! W17 registers this module in the shared showcase and exposes its model
//! through the umbrella crate. Keeping the example independent of a platform
//! adapter makes it useful on native and browser targets alike.

use aimer::accessibility::{
    Bounds, CheckedState, NodeId, Role, SemanticAction, SemanticNode, SemanticSnapshot,
    SemanticTree, ValueRange,
};

use aimer::{AnyElement, BuildContext, Column, Container, Text, Widget};

/// Builds a settings subtree with a merged label, a range value, and an
/// actionable switch.
pub fn settings_semantics_example() -> SemanticSnapshot {
    let volume = ValueRange::new(0.0, 100.0, 75.0)
        .expect("the example range is finite and ordered")
        .with_step(5.0)
        .expect("the example step is positive");
    let root = SemanticNode::new(NodeId::new(1), Role::Group)
        .with_name("Settings")
        .with_bounds(
            Bounds::new(0.0, 0.0, 320.0, 240.0)
                .expect("the example bounds are finite and non-negative"),
        )
        .with_child(
            SemanticNode::new(NodeId::new(2), Role::Switch)
                .with_name("Wi-Fi")
                .checked(CheckedState::Checked)
                .focusable(true)
                .with_action(SemanticAction::Activate),
        )
        .with_child(
            SemanticNode::new(NodeId::new(3), Role::Slider)
                .with_name("Volume")
                .with_value("75 percent")
                .with_value_range(volume)
                .focusable(true)
                .with_action(SemanticAction::Increment)
                .with_action(SemanticAction::Decrement),
        );
    SemanticTree::new(root)
        .snapshot()
        .expect("the example uses unique semantic node identities")
}

/// A small showcase page that renders the canonical semantic snapshot.
pub struct AccessibilityExample;

impl Widget for AccessibilityExample {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let snapshot = settings_semantics_example();
        Container::new()
            .child(Column::new().children(vec![
                    Text::new("Accessibility semantics title").boxed(),
                    Text::new(format!("Published nodes: {}", snapshot.len())).boxed(),
                    Text::new(snapshot.canonical_string()).wrapped().boxed(),
                ]))
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "AccessibilityExample"
    }
}

impl aimer::PortableWidget for AccessibilityExample {}

/// Builds the accessibility semantics showcase without starting an app.
pub fn accessibility_example() -> impl Widget {
    AccessibilityExample
}
