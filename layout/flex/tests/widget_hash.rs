//! Column, Row, Flex and Expanded hash everything that shapes their element, their
//! children's hashes included. A list built from a closure has no hash.

use aimer_container::SizedBox;
use aimer_flex::{BoxAlignment, Column, Expanded, Flex, FlexDirection, JustifyContent, OverflowBehavior, Row};
use aimer_style::LayoutSpacing;
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

fn boxes(widths: &[f32]) -> Vec<SizedBox> {
    widths.iter().map(|width| SizedBox::new().width(*width)).collect()
}

#[test]
fn column_hashes_every_setting_and_its_children_in_order() {
    let base = || Column::new();
    assert_all_distinct(
        hash_of(base().children(boxes(&[1.0, 2.0]))),
        vec![
            hash_of(base().children(boxes(&[2.0, 1.0]))),
            hash_of(base().children(boxes(&[1.0]))),
            hash_of(base().children(boxes(&[1.0, 2.0, 3.0]))),
            hash_of(base().vertical_alignment(BoxAlignment::Center).children(boxes(&[1.0, 2.0]))),
            hash_of(base().horizontal_alignment(BoxAlignment::End).children(boxes(&[1.0, 2.0]))),
            hash_of(base().justify_content(JustifyContent::Center).children(boxes(&[1.0, 2.0]))),
            hash_of(base().gaps(LayoutSpacing::all(4)).children(boxes(&[1.0, 2.0]))),
            hash_of(base().overflow(OverflowBehavior::Wrap).children(boxes(&[1.0, 2.0]))),
        ],
    );
}

#[test]
fn row_hashes_every_setting_and_its_children_in_order() {
    let base = || Row::new();
    assert_all_distinct(
        hash_of(base().children(boxes(&[1.0, 2.0]))),
        vec![
            hash_of(base().children(boxes(&[2.0, 1.0]))),
            hash_of(base().children(boxes(&[1.0]))),
            hash_of(base().vertical_alignment(BoxAlignment::Center).children(boxes(&[1.0, 2.0]))),
            hash_of(base().horizontal_alignment(BoxAlignment::End).children(boxes(&[1.0, 2.0]))),
            hash_of(base().justify_content(JustifyContent::Center).children(boxes(&[1.0, 2.0]))),
            hash_of(base().gaps(LayoutSpacing::all(4)).children(boxes(&[1.0, 2.0]))),
            hash_of(base().overflow(OverflowBehavior::Wrap).children(boxes(&[1.0, 2.0]))),
        ],
    );
}

#[test]
fn flex_hashes_its_direction_and_children() {
    let base = || Flex::new().direction(FlexDirection::Row);
    assert_all_distinct(
        hash_of(base().children(boxes(&[1.0, 2.0]))),
        vec![
            hash_of(Flex::new().direction(FlexDirection::Column).children(boxes(&[1.0, 2.0]))),
            hash_of(base().children(boxes(&[1.0, 3.0]))),
            hash_of(base().gaps(LayoutSpacing::all(4)).children(boxes(&[1.0, 2.0]))),
        ],
    );
}

/// The same settings and children under another container type must not agree.
#[test]
fn a_column_a_row_and_a_flex_over_the_same_children_differ() {
    assert_all_distinct(
        hash_of(Column::new().children(boxes(&[1.0, 2.0]))),
        vec![
            hash_of(Row::new().children(boxes(&[1.0, 2.0]))),
            hash_of(Flex::new().children(boxes(&[1.0, 2.0]))),
        ],
    );
}

#[test]
fn expanded_hashes_its_factor_and_child() {
    assert_all_distinct(
        hash_of(Expanded::new().flex(1.0).child(SizedBox::new())),
        vec![
            hash_of(Expanded::new().flex(2.0).child(SizedBox::new())),
            hash_of(Expanded::new().flex(1.0).child(SizedBox::new().width(3.0))),
        ],
    );
}

#[test]
fn empty_containers_are_hashable() {
    assert!(hash_of(Column::new().children(Vec::<SizedBox>::new())).is_some());
    assert!(hash_of(Row::new()).is_some());
    assert!(hash_of(Flex::new()).is_some());
}

#[test]
fn an_unhashable_child_leaves_every_container_unhashable() {
    assert_eq!(hash_of(Column::new().children([Unhashable])), None);
    assert_eq!(hash_of(Row::new().add_child(Unhashable)), None);
    assert_eq!(hash_of(Flex::new().add_child(Unhashable.boxed())), None);
    assert_eq!(hash_of(Expanded::new().child(Unhashable)), None);
}

/// A list's rows come from a closure, which cannot be hashed.
#[test]
fn a_list_built_from_a_closure_has_no_hash() {
    assert_eq!(
        hash_of(Column::new().list(0..3u32).builder(|_: &u32| SizedBox::new())),
        None
    );
}
