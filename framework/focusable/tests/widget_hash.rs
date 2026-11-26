//! FocusScope and Focusable hash everything their element is built from.
//! Callbacks and focus nodes are compared by identity, since a closure cannot be
//! compared by what it does.

use aimer_focus::{FocusBehavior, FocusNode};
use aimer_focusable::{FocusScope, Focusable, Shortcuts};
use aimer_utils::callback::VoidCallback;
use aimer_widget_core::base::BuildContext;
use aimer_widget_core::testing::assert_hashes_distinct;
use aimer_widget_core::{AnyElement, ContentHash, ErrorWidget, Widget};

fn hash_of(widget: impl Widget + 'static) -> Option<ContentHash> {
    Widget::hash(&widget)
}

/// A widget that cannot be hashed, as one holding a closure would be.
struct Unhashable;

impl Widget for Unhashable {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        unreachable!("the hash tests never build elements")
    }
}

fn leaf() -> ErrorWidget {
    ErrorWidget::new("x")
}

#[test]
fn focus_scope_hashes_its_flag_and_child() {
    assert_eq!(hash_of(FocusScope::new().child(leaf())), hash_of(FocusScope::new().child(leaf())));
    assert_hashes_distinct(
        hash_of(FocusScope::new().child(leaf())),
        &[
            hash_of(FocusScope::new().traps(false).child(leaf())),
            hash_of(FocusScope::new().child(ErrorWidget::new("y"))),
        ],
    );
    assert_eq!(hash_of(FocusScope::new().child(Unhashable)), None);
}

#[test]
fn focusable_hashes_behavior_and_child() {
    assert_eq!(hash_of(Focusable::new().child(leaf())), hash_of(Focusable::new().child(leaf())));
    assert_hashes_distinct(
        hash_of(Focusable::new().child(leaf())),
        &[
            hash_of(Focusable::new().behavior(FocusBehavior::Auto).child(leaf())),
            hash_of(Focusable::new().key("k").child(leaf())),
            hash_of(Focusable::new().child(ErrorWidget::new("y"))),
        ],
    );
    assert_eq!(hash_of(Focusable::new().child(Unhashable)), None);
}

/// The node is the identity focus belongs to, so the same node is unchanged and
/// another one is a change.
#[test]
fn focusable_hashes_its_node_by_identity() {
    let node = FocusNode::new();
    let with = |node: FocusNode| hash_of(Focusable::new().node(node).child(leaf()));
    assert_eq!(with(node.clone()), with(node.clone()));
    assert_hashes_distinct(with(node.clone()), &[with(FocusNode::new()), hash_of(Focusable::new().child(leaf()))]);
    drop(node);
}

#[test]
fn focusable_hashes_its_callbacks_by_identity() {
    let callback = VoidCallback::from(|| {});
    let on_focus = |callback: VoidCallback| hash_of(Focusable::new().on_focus(callback).child(leaf()));
    let on_lost = |callback: VoidCallback| hash_of(Focusable::new().on_focus_lost(callback).child(leaf()));
    assert_eq!(on_focus(callback.clone()), on_focus(callback.clone()));
    // The same callback in another slot is another configuration.
    assert_ne!(on_focus(callback.clone()), on_lost(callback.clone()));
    assert_hashes_distinct(on_focus(callback.clone()), &[on_focus(VoidCallback::from(|| {}))]);
    drop(callback);
}

/// Shortcuts are built from closures made at every build, which can never match
/// a previous build's, so they report no hash.
#[test]
fn shortcuts_have_no_hash() {
    assert_eq!(hash_of(Shortcuts::new().bind_str("Ctrl+S", || {}).child(leaf())), None);
}
