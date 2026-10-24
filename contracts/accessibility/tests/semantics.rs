use std::time::Duration;

use aimer_accessibility::{
    AccessibilityPreferences, ActionHandler, ActionRequest, Announcement, AnnouncementKind,
    AnnouncementPort, Bounds, CheckedState, Color, FocusTraversalSource, NodeId,
    NoopAnnouncementPort, PreferenceAdapter, Role, SemanticAction, SemanticNode, SemanticTree,
    TouchTargetPolicy, ValueRange, contrast_ratio, validate_contrast, validate_touch_target,
};

#[test]
fn semantic_tree_applies_merge_exclude_leaf_and_has_stable_output() {
    let root = SemanticNode::new(NodeId::new(1), Role::Group)
        .with_name("Settings")
        .with_description("Device settings")
        .with_bounds(Bounds::new(0.0, 0.0, 320.0, 240.0).unwrap())
        .selected(true)
        .checked(CheckedState::Checked)
        .expanded(true)
        .busy(true)
        .with_child(
            SemanticNode::new(NodeId::new(2), Role::Group)
                .merge()
                .with_child(SemanticNode::new(NodeId::new(3), Role::Text).with_name("Wi-Fi"))
                .with_child(SemanticNode::new(NodeId::new(4), Role::Text).with_name("On")),
        )
        .with_child(
            SemanticNode::new(NodeId::new(5), Role::Presentation)
                .exclude()
                .with_child(SemanticNode::new(NodeId::new(6), Role::Button).with_name("Noise")),
        )
        .with_child(
            SemanticNode::new(NodeId::new(7), Role::Group)
                .leaf()
                .with_name("Advanced")
                .with_value("collapsed")
                .with_value_range(ValueRange::new(0.0, 1.0, 0.0).unwrap())
                .enabled(false)
                .with_child(SemanticNode::new(NodeId::new(8), Role::Text).with_name("Hidden")),
        );

    let first = SemanticTree::new(root.clone()).snapshot().unwrap();
    let second = SemanticTree::new(root).snapshot().unwrap();

    let ids: Vec<_> = first.traverse().map(|node| node.id()).collect();
    assert_eq!(ids, vec![NodeId::new(1), NodeId::new(2), NodeId::new(7)]);
    assert_eq!(first.node(NodeId::new(2)).unwrap().name(), Some("Wi-Fi On"));
    assert!(first.node(NodeId::new(6)).is_none());
    assert!(first.node(NodeId::new(7)).unwrap().children().is_empty());
    assert_eq!(first.root().state().selected(), Some(true));
    assert_eq!(first.root().state().checked(), Some(CheckedState::Checked));
    assert_eq!(first.root().state().expanded(), Some(true));
    assert!(first.root().state().busy());
    assert_eq!(first.root().description(), Some("Device settings"));
    assert_eq!(first.root().bounds().unwrap().width(), 320.0);
    assert_eq!(
        first.node(NodeId::new(7)).unwrap().value(),
        Some("collapsed")
    );
    assert_eq!(
        first
            .node(NodeId::new(7))
            .unwrap()
            .value_range()
            .unwrap()
            .current(),
        0.0
    );
    assert!(!first.node(NodeId::new(7)).unwrap().state().enabled());
    assert_eq!(first.canonical_string(), second.canonical_string());
}

#[test]
fn actions_dispatch_and_focus_projection_preserve_host_order() {
    let snapshot = SemanticTree::new(
        SemanticNode::new(NodeId::new(1), Role::Group)
            .with_child(
                SemanticNode::new(NodeId::new(2), Role::Button)
                    .with_name("Save")
                    .focusable(true)
                    .with_action(SemanticAction::Activate),
            )
            .with_child(SemanticNode::new(NodeId::new(3), Role::Text).focusable(true))
            .with_child(SemanticNode::new(NodeId::new(4), Role::Button).focusable(false)),
    )
    .snapshot()
    .unwrap();

    let mut actions = RecordingActions::default();
    snapshot
        .dispatch_action(NodeId::new(2), &SemanticAction::Activate, &mut actions)
        .unwrap();
    assert_eq!(
        actions.requests,
        vec![ActionRequest::new(NodeId::new(2), SemanticAction::Activate)]
    );

    let focus = HostFocusOrder {
        ordered: vec![
            NodeId::new(4),
            NodeId::new(3),
            NodeId::new(2),
            NodeId::new(99),
        ],
    };
    assert_eq!(
        snapshot.focus_order(&focus),
        vec![NodeId::new(3), NodeId::new(2)]
    );
}

#[test]
fn announcements_are_bounded_and_noop_delivery_is_deterministic() {
    let announcement =
        Announcement::try_new(AnnouncementKind::ValidationError, "Name is required").unwrap();
    let mut port = RecordingAnnouncements::default();
    announcement.deliver_to(&mut port);
    assert_eq!(port.messages, vec!["Name is required"]);

    let mut noop = NoopAnnouncementPort;
    announcement.deliver_to(&mut noop);

    assert!(Announcement::try_new(AnnouncementKind::Status, "   ").is_err());
    assert!(Announcement::try_new(AnnouncementKind::Loading, "x".repeat(513)).is_err());
    assert!(Announcement::try_new(AnnouncementKind::Status, "bad\0text").is_err());
}

#[test]
fn preference_adapter_maps_platform_inputs_without_global_state() {
    let preferences = AccessibilityPreferences::from_adapter(&FakePreferences {
        reduced_motion: true,
        text_scale: 1.5,
        high_contrast: true,
        non_color_cues: true,
    })
    .unwrap();

    assert!(preferences.reduced_motion());
    assert!(preferences.high_contrast());
    assert!(preferences.non_color_cues());
    assert_eq!(preferences.scaled_text_size(10.0).unwrap(), 15.0);
    assert_eq!(
        preferences.motion_duration(Duration::from_millis(200)),
        Duration::ZERO
    );
    assert!(
        AccessibilityPreferences::from_adapter(&FakePreferences {
            text_scale: f32::NAN,
            ..FakePreferences::default()
        })
        .is_err()
    );
}

#[test]
fn touch_and_contrast_helpers_enforce_their_boundaries() {
    let policy = TouchTargetPolicy::default();
    assert!(validate_touch_target(Bounds::new(0.0, 0.0, 44.0, 44.0).unwrap(), policy).is_ok());
    assert!(validate_touch_target(Bounds::new(0.0, 0.0, 43.99, 44.0).unwrap(), policy).is_err());

    let black = Color::new(0.0, 0.0, 0.0).unwrap();
    let white = Color::new(1.0, 1.0, 1.0).unwrap();
    assert!((contrast_ratio(black, white).unwrap() - 21.0).abs() < 0.00001);
    assert!(validate_contrast(black, white, 4.5).is_ok());
    assert!(validate_contrast(black, white, 21.01).is_err());
    assert!(Color::new(1.0, f32::INFINITY, 0.0).is_err());
}

#[test]
fn value_ranges_reject_non_finite_and_reversed_values() {
    assert!(
        ValueRange::new(0.0, 100.0, 50.0)
            .unwrap()
            .with_step(5.0)
            .is_ok()
    );
    assert!(ValueRange::new(100.0, 0.0, 50.0).is_err());
    assert!(ValueRange::new(0.0, 100.0, f32::NAN).is_err());
    assert!(
        ValueRange::new(0.0, 100.0, 50.0)
            .unwrap()
            .with_step(0.0)
            .is_err()
    );
}

#[derive(Default)]
struct RecordingActions {
    requests: Vec<ActionRequest>,
}

impl ActionHandler for RecordingActions {
    fn handle(&mut self, request: ActionRequest) {
        self.requests.push(request);
    }
}

struct HostFocusOrder {
    ordered: Vec<NodeId>,
}

impl FocusTraversalSource for HostFocusOrder {
    fn ordered_nodes(&self) -> &[NodeId] {
        &self.ordered
    }
}

#[derive(Default)]
struct RecordingAnnouncements {
    messages: Vec<String>,
}

impl AnnouncementPort for RecordingAnnouncements {
    fn announce(&mut self, announcement: &Announcement) {
        self.messages.push(announcement.text().to_owned());
    }
}

#[derive(Default)]
struct FakePreferences {
    reduced_motion: bool,
    text_scale: f32,
    high_contrast: bool,
    non_color_cues: bool,
}

impl PreferenceAdapter for FakePreferences {
    fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    fn text_scale(&self) -> f32 {
        if self.text_scale == 0.0 {
            1.0
        } else {
            self.text_scale
        }
    }

    fn high_contrast(&self) -> bool {
        self.high_contrast
    }

    fn non_color_cues(&self) -> bool {
        self.non_color_cues
    }
}
