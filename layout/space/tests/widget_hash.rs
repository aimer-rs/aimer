//! Align, Positioned and Stack hash everything that shapes their element, their
//! children's hashes included, so a retained part built from them can be
//! recognised as unchanged.

use aimer_container::SizedBox;
use aimer_space::{Align, Alignment, Position, Positioned, Stack, StackDirection, Transform};
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, ContentHash, Widget};

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

fn assert_all_distinct(base: Option<ContentHash>, variants: Vec<Option<ContentHash>>) {
    assert!(base.is_some(), "the base widget has no hash");
    for (index, hash) in variants.iter().enumerate() {
        assert!(hash.is_some(), "variant {index} has no hash");
        assert_ne!(*hash, base, "variant {index} left the hash unchanged");
    }
    for first in 0..variants.len() {
        for second in first + 1..variants.len() {
            assert_ne!(variants[first], variants[second], "variants {first} and {second} collide");
        }
    }
}

fn leaf(width: f32) -> SizedBox {
    SizedBox::new().width(width)
}

#[test]
fn align_hashes_alignment_layer_and_child() {
    assert_all_distinct(
        hash_of(Align::new().alignment(Alignment::TopLeft).layer(1).child(leaf(1.0))),
        vec![
            hash_of(Align::new().alignment(Alignment::BotRight).layer(1).child(leaf(1.0))),
            hash_of(Align::new().alignment(Alignment::TopLeft).layer(2).child(leaf(1.0))),
            hash_of(Align::new().alignment(Alignment::TopLeft).layer(1).child(leaf(2.0))),
        ],
    );
}

#[test]
fn positioned_hashes_every_field() {
    let base = || Positioned::new().left(1.0).top(2.0);
    assert_all_distinct(
        hash_of(base().child(leaf(1.0))),
        vec![
            hash_of(base().left(9.0).child(leaf(1.0))),
            hash_of(base().top(9.0).child(leaf(1.0))),
            hash_of(base().right(9.0).child(leaf(1.0))),
            hash_of(base().bottom(9.0).child(leaf(1.0))),
            hash_of(base().position(Position::Absolute).child(leaf(1.0))),
            hash_of(base().transform(Transform::Translate(1.0, 2.0)).child(leaf(1.0))),
            hash_of(base().transform(Transform::Scale(1.0, 2.0)).child(leaf(1.0))),
            hash_of(base().transform(Transform::Rotate(1.0)).child(leaf(1.0))),
            hash_of(base().layer(3).child(leaf(1.0))),
            hash_of(base().child(leaf(2.0))),
        ],
    );
}

/// The same number under another transform is another transform.
#[test]
fn transforms_with_equal_numbers_differ() {
    let with = |transform| hash_of(Positioned::new().transform(transform).child(leaf(1.0)));
    assert_all_distinct(
        with(Transform::TranslateX(2.0)),
        vec![with(Transform::TranslateY(2.0)), with(Transform::ScaleX(2.0)), with(Transform::ScaleY(2.0)), with(Transform::Rotate(2.0))],
    );
}

#[test]
fn stack_hashes_its_children_in_order_and_its_direction() {
    let stack = |widths: &[f32]| {
        widths.iter().fold(Stack::new(), |stack, width| stack.add_child(leaf(*width)))
    };
    assert_all_distinct(
        hash_of(stack(&[1.0, 2.0])),
        vec![
            hash_of(stack(&[2.0, 1.0])),
            hash_of(stack(&[1.0])),
            hash_of(stack(&[1.0, 2.0, 3.0])),
            hash_of(stack(&[1.0, 3.0])),
            hash_of(stack(&[1.0, 2.0]).direction(StackDirection::Reverse)),
        ],
    );
}

#[test]
fn an_empty_stack_is_hashable() {
    assert!(hash_of(Stack::new()).is_some());
    assert_ne!(hash_of(Stack::new()), hash_of(Stack::new().direction(StackDirection::Reverse)));
}

#[test]
fn an_unhashable_child_leaves_every_wrapper_unhashable() {
    assert_eq!(hash_of(Align::new().child(Unhashable)), None);
    assert_eq!(hash_of(Positioned::new().child(Unhashable)), None);
    assert_eq!(hash_of(Stack::new().add_child(leaf(1.0)).add_child(Unhashable)), None);
}
