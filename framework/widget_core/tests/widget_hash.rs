//! The widgets `aimer_widget_core` defines hash everything their element is built
//! from, so a part made of them can be recognised as unchanged.

use aimer_accessibility::Role;
use aimer_widget_core::base::BuildContext;
use aimer_widget_core::testing::assert_hashes_distinct;
use aimer_widget_core::{
    AnyElement, ChildBuilder, ContentHash, ErrorWidget, NamedWidget, OverflowIndicator,
    RepaintBoundary, Semantics, Widget,
};

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

#[test]
fn error_widget_hashes_its_message() {
    assert!(hash_of(ErrorWidget::new("a")).is_some());
    assert_eq!(hash_of(ErrorWidget::new("a")), hash_of(ErrorWidget::new("a")));
    assert_hashes_distinct(hash_of(ErrorWidget::new("a")), &[hash_of(ErrorWidget::new("b"))]);
}

#[test]
fn overflow_indicator_hashes_label_clip_and_child() {
    let base = || OverflowIndicator::new(ErrorWidget::new("x"));
    assert_eq!(hash_of(base()), hash_of(base()));
    assert_hashes_distinct(
        hash_of(base()),
        &[
            hash_of(base().label("l")),
            hash_of(base().clip(false)),
            hash_of(OverflowIndicator::new(ErrorWidget::new("y"))),
        ],
    );
    assert_eq!(hash_of(OverflowIndicator::new(Unhashable)), None);
}

#[test]
fn repaint_boundary_hashes_its_child_by_content() {
    let boundary = |message| RepaintBoundary::new().child(ErrorWidget::new(message));
    assert!(hash_of(boundary("a")).is_some());
    assert_eq!(hash_of(boundary("a")), hash_of(boundary("a")));
    assert_hashes_distinct(hash_of(boundary("a")), &[hash_of(boundary("b"))]);
    assert_eq!(hash_of(RepaintBoundary::new().child(Unhashable)), None);
}

#[test]
fn semantics_hashes_every_field() {
    let base = || Semantics::new().label("l").child(ErrorWidget::new("x"));
    assert_eq!(hash_of(base()), hash_of(base()));
    assert_hashes_distinct(
        hash_of(base()),
        &[
            hash_of(Semantics::new().label("m").child(ErrorWidget::new("x"))),
            hash_of(Semantics::new().label("l").value("v").child(ErrorWidget::new("x"))),
            hash_of(Semantics::new().label("l").role(Role::Button).child(ErrorWidget::new("x"))),
            hash_of(Semantics::new().label("l").enabled(false).child(ErrorWidget::new("x"))),
            hash_of(Semantics::new().label("l").merges(false).child(ErrorWidget::new("x"))),
            hash_of(Semantics::new().label("l").child(ErrorWidget::new("y"))),
        ],
    );
}

/// A callback is compared by identity: the same one is unchanged, another one is
/// a change even if it is written the same.
#[test]
fn semantics_hashes_its_callback_by_identity() {
    use aimer_utils::callback::VoidCallback;
    let callback = VoidCallback::from(|| {});
    let with = |callback: VoidCallback| {
        hash_of(Semantics::new().on_activate(callback).child(ErrorWidget::new("x")))
    };
    assert_eq!(with(callback.clone()), with(callback.clone()));
    // `callback` stays alive, as the old part's element keeps it alive in an app:
    // a freed callback's address can be handed to the next one.
    assert_hashes_distinct(with(callback.clone()), &[with(VoidCallback::from(|| {}))]);
    drop(callback);
}

#[test]
fn named_widget_hashes_its_name_and_inner_widget() {
    let named = |name, message| NamedWidget::new(ErrorWidget::new(message).boxed(), name);
    assert_eq!(hash_of(named("n", "a")), hash_of(named("n", "a")));
    assert_hashes_distinct(hash_of(named("n", "a")), &[hash_of(named("m", "a")), hash_of(named("n", "b"))]);
    assert_eq!(hash_of(NamedWidget::new(Unhashable.boxed(), "n")), None);
}

/// A child nobody has built yet is the widget it will build, so equal widgets
/// give equal hashes however many times they were wrapped.
#[test]
fn an_unbuilt_child_builder_hashes_by_the_widget_inside() {
    let builder = |message| ChildBuilder::from_widget(ErrorWidget::new(message));
    assert!(hash_of(builder("a")).is_some());
    assert_eq!(hash_of(builder("a")), hash_of(builder("a")));
    assert_hashes_distinct(hash_of(builder("a")), &[hash_of(builder("b"))]);
    assert_eq!(hash_of(ChildBuilder::from_widget(Unhashable)), None);
}

/// A closure is asked again at every build, so the only thing that identifies the
/// child is the closure itself.
#[test]
fn a_closure_child_builder_hashes_by_identity() {
    let first = ChildBuilder::new(|| ErrorWidget::new("a").boxed());
    let second = ChildBuilder::new(|| ErrorWidget::new("a").boxed());
    assert!(Widget::hash(&first).is_some());
    assert_eq!(Widget::hash(&first), Widget::hash(&first));
    assert_ne!(Widget::hash(&first), Widget::hash(&second));
}
