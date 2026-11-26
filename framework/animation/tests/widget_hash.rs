//! The animation widgets hash everything their element is built from. A
//! controller is a shared handle, so it is compared by identity: the same
//! controller is the same animation, and no other is.

use std::time::Duration;

use aimer_animation::layout::AnimatedLayout;
use aimer_animation::{
    Animated, AnimatedPaint, AnimatedSwitcher, AnimationController, AnimationEffect, FadeTransition,
    MorphStyle, MorphTransition, RotationTransition, ScaleTransition, SharedElement, SlideTransition,
    curves,
};
use aimer_widget::base::BuildContext;
use aimer_widget::testing::assert_hashes_distinct;
use aimer_widget::{AnyElement, ContentHash, ErrorWidget, Widget};

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

fn controller() -> AnimationController {
    AnimationController::new(Duration::from_millis(100), curves::LINEAR)
}

#[test]
fn transitions_hash_their_controller_by_identity_and_their_child() {
    let c = controller();
    let fade = |c: &AnimationController, child: ErrorWidget| hash_of(FadeTransition::new(c.clone(), child));
    assert_eq!(fade(&c, leaf()), fade(&c, leaf()));
    assert_hashes_distinct(fade(&c, leaf()), &[fade(&controller(), leaf()), fade(&c, ErrorWidget::new("y"))]);

    let scale = |c: &AnimationController| hash_of(ScaleTransition::new(c.clone(), leaf()));
    assert_eq!(scale(&c), scale(&c));
    assert_ne!(scale(&c), scale(&controller()));
    // The same controller under another transition is another widget.
    assert_ne!(fade(&c, leaf()), scale(&c));

    assert_eq!(hash_of(FadeTransition::new(c.clone(), Unhashable)), None);
}

#[test]
fn slide_transition_hashes_its_offset() {
    let c = controller();
    let slide = |offset| hash_of(SlideTransition::new(c.clone(), offset, leaf()));
    assert_eq!(slide((1.0, 2.0)), slide((1.0, 2.0)));
    assert_hashes_distinct(slide((1.0, 2.0)), &[slide((2.0, 2.0)), slide((1.0, 3.0))]);
}

#[test]
fn rotation_transition_hashes_its_turn_range() {
    let c = controller();
    let spin = |begin, end| hash_of(RotationTransition::new(c.clone(), leaf()).turn_range(begin, end));
    assert_eq!(spin(0.0, 1.0), spin(0.0, 1.0));
    assert_hashes_distinct(spin(0.0, 1.0), &[spin(0.5, 1.0), spin(0.0, 2.0)]);
}

#[test]
fn animated_hashes_controller_effect_and_child() {
    let c = controller();
    let effect = AnimationEffect::Opacity { from: 0.0, to: 1.0 };
    let animated = |c: &AnimationController, effect, child: ErrorWidget| hash_of(Animated::new(c.clone(), effect, child));
    assert_eq!(animated(&c, effect, leaf()), animated(&c, effect, leaf()));
    assert_hashes_distinct(
        animated(&c, effect, leaf()),
        &[
            animated(&controller(), effect, leaf()),
            animated(&c, AnimationEffect::Opacity { from: 0.0, to: 0.5 }, leaf()),
            animated(&c, AnimationEffect::Scale { from: 0.0, to: 1.0 }, leaf()),
            animated(&c, AnimationEffect::SlideX { from: 0.0, to: 1.0 }, leaf()),
            animated(&c, AnimationEffect::SlideY { from: 0.0, to: 1.0 }, leaf()),
            animated(&c, AnimationEffect::Rotate { from: 0.0, to: 1.0 }, leaf()),
            animated(&c, AnimationEffect::Translate { from_x: 0.0, from_y: 0.0, to_x: 1.0, to_y: 1.0 }, leaf()),
            animated(&c, effect, ErrorWidget::new("y")),
        ],
    );
    assert_eq!(hash_of(Animated::new(c, effect, Unhashable)), None);
}

#[test]
fn animated_layout_hashes_its_configuration_and_child() {
    let layout = || AnimatedLayout::new();
    assert_eq!(hash_of(layout().child(leaf())), hash_of(layout().child(leaf())));
    assert_hashes_distinct(
        hash_of(layout().child(leaf())),
        &[
            hash_of(layout().duration(Duration::from_millis(10)).child(leaf())),
            hash_of(layout().curve(curves::EASE_IN).child(leaf())),
            hash_of(layout().enabled(false).child(leaf())),
            hash_of(layout().reduced_motion(true).child(leaf())),
            hash_of(layout().child(ErrorWidget::new("y"))),
        ],
    );
}

#[test]
fn animated_paint_hashes_its_stock_effects_and_not_its_closures() {
    let c = controller();
    assert_eq!(hash_of(AnimatedPaint::new(c.clone(), leaf())), hash_of(AnimatedPaint::new(c.clone(), leaf())));
    assert_hashes_distinct(
        hash_of(AnimatedPaint::new(c.clone(), leaf())),
        &[
            hash_of(AnimatedPaint::new(c.clone(), leaf()).opacity()),
            hash_of(AnimatedPaint::new(c.clone(), leaf()).inert()),
            hash_of(AnimatedPaint::new(controller(), leaf())),
        ],
    );
    // A closure made at build time can never equal the one before it.
    assert_eq!(hash_of(AnimatedPaint::new(c, leaf()).effect(|_| aimer_animation::PaintPresentation::IDENTITY_OPAQUE)), None);
}

#[test]
fn shared_element_hashes_its_tag_and_child() {
    let shared = |tag: &'static str| hash_of(SharedElement::new().tag(tag).child(leaf()));
    assert_eq!(shared("a"), shared("a"));
    assert_hashes_distinct(
        shared("a"),
        &[shared("b"), hash_of(SharedElement::new().child(leaf())), hash_of(SharedElement::new().tag("a").child(ErrorWidget::new("y")))],
    );
}

#[test]
fn morph_transition_hashes_every_setting_and_its_child() {
    let morph = || MorphTransition::new(Duration::from_millis(100), curves::LINEAR, leaf());
    assert_eq!(hash_of(morph()), hash_of(morph()));
    assert_hashes_distinct(
        hash_of(morph()),
        &[
            hash_of(MorphTransition::new(Duration::from_millis(200), curves::LINEAR, leaf())),
            hash_of(MorphTransition::new(Duration::from_millis(100), curves::EASE_IN, leaf())),
            hash_of(MorphTransition::new(Duration::from_millis(100), curves::LINEAR, ErrorWidget::new("y"))),
            hash_of(morph().style(MorphStyle::Wipe)),
            hash_of(morph().direction(0.0, 1.0)),
            hash_of(morph().softness(0.9)),
            hash_of(morph().amplitude(2.0)),
            hash_of(morph().frequency(3.0)),
            hash_of(morph().seed(4.0)),
            hash_of(morph().blur(5.0)),
            hash_of(morph().child_key("c")),
            hash_of(morph().key("k")),
        ],
    );
    assert_eq!(hash_of(MorphTransition::new(Duration::from_millis(100), curves::LINEAR, Unhashable)), None);
}

#[test]
fn animated_switcher_hashes_every_setting_and_its_child() {
    let switcher = || AnimatedSwitcher::new(Duration::from_millis(100), curves::LINEAR, leaf());
    assert_eq!(hash_of(switcher()), hash_of(switcher()));
    assert_hashes_distinct(
        hash_of(switcher()),
        &[
            hash_of(AnimatedSwitcher::new(Duration::from_millis(200), curves::LINEAR, leaf())),
            hash_of(AnimatedSwitcher::new(Duration::from_millis(100), curves::EASE_IN, leaf())),
            hash_of(AnimatedSwitcher::new(Duration::from_millis(100), curves::LINEAR, ErrorWidget::new("y"))),
            hash_of(switcher().with_switch_out_curve(curves::EASE_OUT)),
            hash_of(switcher().child_key("c")),
            hash_of(switcher().key("k")),
        ],
    );
}
