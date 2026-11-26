//! A slider reports a content hash of what it is configured with, so a part that
//! holds one can be retained while nothing changed. Its callbacks are compared by
//! identity: the same shared callback is unchanged, and a closure written again is
//! a change, so callers that want a slider to be retained keep one callback and
//! hand it over each build.

use std::rc::Rc;

use aimer_range::{RangeChangeCallback, ReversedBoundsPolicy, Slider, SliderThumb, SliderTrail};
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::testing::assert_hashes_distinct;
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

fn base() -> Slider {
    Slider::new().range(0.0..100.0).step(5.0).value(20.0)
}

#[test]
fn a_slider_hashes_everything_it_is_configured_with() {
    assert!(hash_of(base()).is_some());
    assert_eq!(hash_of(base()), hash_of(base()));
    assert_hashes_distinct(
        hash_of(base()),
        &[
            hash_of(base().range(0.0..200.0)),
            hash_of(base().range(10.0..100.0)),
            hash_of(base().step(10.0)),
            hash_of(base().value(25.0)),
            hash_of(base().reversed_bounds_policy(ReversedBoundsPolicy::Normalize)),
            hash_of(base().disabled(true)),
            hash_of(base().width(300.0)),
            hash_of(base().height(40.0)),
            hash_of(base().thumb(SliderThumb::new())),
            hash_of(base().trail(SliderTrail::new())),
        ],
    );
}

#[test]
fn the_visual_parts_hash_their_settings() {
    let with_thumb = |thumb: SliderThumb| hash_of(base().thumb(thumb));
    assert_eq!(with_thumb(SliderThumb::new()), with_thumb(SliderThumb::new()));
    assert_hashes_distinct(
        with_thumb(SliderThumb::new()),
        &[
            with_thumb(SliderThumb::new().size(30.0)),
            with_thumb(SliderThumb::new().radius(3.0)),
            with_thumb(SliderThumb::new().color(Color::RED)),
            with_thumb(SliderThumb::new().focused_color(Color::RED)),
            with_thumb(SliderThumb::new().disabled_color(Color::RED)),
        ],
    );
    let with_trail = |trail: SliderTrail| hash_of(base().trail(trail));
    assert_eq!(with_trail(SliderTrail::new()), with_trail(SliderTrail::new()));
    assert_hashes_distinct(
        with_trail(SliderTrail::new()),
        &[
            with_trail(SliderTrail::new().width(50.0)),
            with_trail(SliderTrail::new().height(9.0)),
            with_trail(SliderTrail::new().radius(1.0)),
            with_trail(SliderTrail::new().color(Color::RED)),
            with_trail(SliderTrail::new().pressed_color(Color::RED)),
            with_trail(SliderTrail::new().disabled_color(Color::RED)),
        ],
    );
}

#[test]
fn a_custom_visual_without_a_hash_leaves_the_slider_without_one() {
    assert_eq!(hash_of(base().thumb(Unhashable)), None);
    assert_eq!(hash_of(base().track(Unhashable)), None);
    assert_eq!(hash_of(base().trail(Unhashable)), None);
}

/// A shared callback is one object: handing it over again changes nothing, and a
/// different one is a change.
#[test]
fn a_shared_callback_is_compared_by_identity() {
    let callback: RangeChangeCallback = Rc::new(|_value: f64| {});
    let with = |callback: &RangeChangeCallback| hash_of(base().on_change_shared(Rc::clone(callback)));
    assert_eq!(with(&callback), with(&callback));
    let other: RangeChangeCallback = Rc::new(|_value: f64| {});
    assert_hashes_distinct(with(&callback), &[with(&other), hash_of(base())]);
    // Another slot is another configuration.
    assert_ne!(
        hash_of(base().on_change_shared(Rc::clone(&callback))),
        hash_of(base().on_change_end_shared(Rc::clone(&callback)))
    );
}

/// A closure written again at every build is a new callback each time, which is
/// why the shared form exists.
#[test]
fn a_closure_written_again_is_a_change() {
    // Both stay alive while they are hashed, as a retained part keeps the old one
    // alive: a freed callback's address can be handed to the next one.
    let first = base().on_change(|_value: f64| {});
    let second = base().on_change(|_value: f64| {});
    assert!(Widget::hash(&first).is_some());
    assert_ne!(Widget::hash(&first), Widget::hash(&second));
}

#[test]
fn a_slider_of_another_value_type_hashes_apart() {
    let float = hash_of(Slider::<f64>::new().range(0.0..10.0).step(1.0).value(2.0));
    let integer = hash_of(Slider::<i32>::new().range(0..10).step(1).value(2));
    assert!(float.is_some() && integer.is_some());
    assert_ne!(float, integer);
}
