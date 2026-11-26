//! The single-child layout widgets hash everything that shapes their element,
//! their child's hash included, so a retained part built from them can be
//! recognised as unchanged.

use aimer_container::{AspectRatio, Center, Margin, Opacity, Padding, RatioOption, Scalable, SizedBox, ZeroSizedBox};
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

/// Every hash in `variants` exists, differs from `base`, and differs from every
/// other variant.
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
fn zero_sized_box_is_hashable() {
    assert!(hash_of(ZeroSizedBox).is_some());
    assert_eq!(hash_of(ZeroSizedBox), hash_of(ZeroSizedBox));
}

#[test]
fn padding_hashes_its_spacing_and_child() {
    assert_all_distinct(
        hash_of(Padding::new().padding(LayoutSpacing::all(4)).child(leaf(1.0))),
        vec![
            hash_of(Padding::new().padding(LayoutSpacing::all(5)).child(leaf(1.0))),
            hash_of(Padding::new().padding(LayoutSpacing::all(4)).child(leaf(2.0))),
            hash_of(Padding::new().padding(LayoutSpacing::new().top(4)).child(leaf(1.0))),
        ],
    );
}

#[test]
fn padding_and_margin_over_the_same_data_differ() {
    let spacing = LayoutSpacing::all(4);
    assert_ne!(
        hash_of(Padding::new().padding(spacing).child(leaf(1.0))),
        hash_of(Margin::new().margin(spacing).child(leaf(1.0)))
    );
}

#[test]
fn margin_hashes_its_spacing_and_child() {
    assert_all_distinct(
        hash_of(Margin::new().margin(LayoutSpacing::all(4)).child(leaf(1.0))),
        vec![
            hash_of(Margin::new().margin(LayoutSpacing::all(5)).child(leaf(1.0))),
            hash_of(Margin::new().margin(LayoutSpacing::all(4)).child(leaf(2.0))),
        ],
    );
}

#[test]
fn center_hashes_its_child() {
    assert_all_distinct(hash_of(Center::new().child(leaf(1.0))), vec![hash_of(Center::new().child(leaf(2.0)))]);
}

#[test]
fn opacity_hashes_its_value_and_child() {
    assert_all_distinct(
        hash_of(Opacity::new().opacity(0.5).child(leaf(1.0))),
        vec![
            hash_of(Opacity::new().opacity(0.6).child(leaf(1.0))),
            hash_of(Opacity::new().opacity(0.5).child(leaf(2.0))),
        ],
    );
}

#[test]
fn scalable_hashes_its_scale_and_child() {
    assert_all_distinct(
        hash_of(Scalable::new().scale(2.0).child(leaf(1.0))),
        vec![
            hash_of(Scalable::new().scale(3.0).child(leaf(1.0))),
            hash_of(Scalable::new().scale(2.0).child(leaf(2.0))),
        ],
    );
}

#[test]
fn aspect_ratio_hashes_its_ratio_axis_and_child() {
    assert_all_distinct(
        hash_of(AspectRatio::new().aspect_ratio(1.5).child(leaf(1.0))),
        vec![
            hash_of(AspectRatio::new().aspect_ratio(2.0).child(leaf(1.0))),
            hash_of(AspectRatio::new().aspect_ratio(1.5).ratio_option(RatioOption::Height).child(leaf(1.0))),
            hash_of(AspectRatio::new().aspect_ratio(1.5).child(leaf(2.0))),
        ],
    );
}

/// One child without a hash is enough to leave every ancestor without one.
#[test]
fn an_unhashable_child_leaves_every_wrapper_unhashable() {
    assert_eq!(hash_of(Padding::new().padding(LayoutSpacing::all(4)).child(Unhashable)), None);
    assert_eq!(hash_of(Margin::new().margin(LayoutSpacing::all(4)).child(Unhashable)), None);
    assert_eq!(hash_of(Center::new().child(Unhashable)), None);
    assert_eq!(hash_of(Opacity::new().opacity(0.5).child(Unhashable)), None);
    assert_eq!(hash_of(Scalable::new().scale(2.0).child(Unhashable)), None);
    assert_eq!(hash_of(AspectRatio::new().child(Unhashable)), None);
}

/// The same child under different wrapper types must not agree.
#[test]
fn wrappers_of_one_child_do_not_collide() {
    assert_all_distinct(
        hash_of(Center::new().child(leaf(1.0))),
        vec![
            hash_of(Opacity::new().opacity(1.0).child(leaf(1.0))),
            hash_of(Scalable::new().scale(1.0).child(leaf(1.0))),
            hash_of(AspectRatio::new().aspect_ratio(1.0).child(leaf(1.0))),
        ],
    );
}

mod container {
    use aimer_color::prelude::Color;
    use aimer_container::Container;
    use aimer_decoration::{BorderRadius, BoxDecoration};
    use aimer_style::LayoutSpacing;

    use super::*;

    #[test]
    fn container_hashes_every_field() {
        let base = || Container::new().width(10.0).height(20.0);
        assert_eq!(hash_of(base().child(leaf(1.0))), hash_of(base().child(leaf(1.0))));
        assert_all_distinct(
            hash_of(base().child(leaf(1.0))),
            vec![
                hash_of(Container::new().width(11.0).height(20.0).child(leaf(1.0))),
                hash_of(Container::new().width(10.0).height(21.0).child(leaf(1.0))),
                hash_of(base().padding(LayoutSpacing::all(4)).child(leaf(1.0))),
                hash_of(base().margin(LayoutSpacing::all(4)).child(leaf(1.0))),
                hash_of(base().color(Color::RED).child(leaf(1.0))),
                hash_of(base().fit_content().child(leaf(1.0))),
                hash_of(base().box_decoration(BoxDecoration::new().border_radius(BorderRadius::all(4.0))).child(leaf(1.0))),
                hash_of(base().child(leaf(2.0))),
            ],
        );
        assert_eq!(hash_of(base().child(Unhashable)), None);
    }

    /// Padding and margin are different things, even with the same spacing.
    #[test]
    fn padding_and_margin_differ() {
        let spacing = LayoutSpacing::all(4);
        assert_all_distinct(
            hash_of(Container::new().padding(spacing).child(leaf(1.0))),
            vec![hash_of(Container::new().margin(spacing).child(leaf(1.0)))],
        );
    }

    #[test]
    fn a_container_over_a_custom_decoration_hashes_what_it_reports() {
        use aimer_decoration::Decoration;
        struct Plain;
        impl Decoration for Plain {
            fn paint(&self, _: &aimer_canvas::Canvas, _: aimer_attribute::Vec2d, _: aimer_attribute::ResolvedSize, _: f32) {}
        }
        // A decoration that reports no hash leaves the container without one.
        assert_eq!(hash_of(Container::new().decoration(Plain).child(leaf(1.0))), None);
        assert!(hash_of(Container::new().decoration(Color::RED).child(leaf(1.0))).is_some());
    }
}

mod resizable {
    use aimer_attribute::ResolvedSize;
    use aimer_container::{Direction, Resizable};
    use aimer_utils::callback::Callback;

    use super::*;

    #[test]
    fn resizable_hashes_every_field() {
        let base = || Resizable::new().width(10.0).height(20.0);
        assert_eq!(hash_of(base().child(leaf(1.0))), hash_of(base().child(leaf(1.0))));
        assert_all_distinct(
            hash_of(base().child(leaf(1.0))),
            vec![
                hash_of(Resizable::new().width(11.0).height(20.0).child(leaf(1.0))),
                hash_of(Resizable::new().width(10.0).height(21.0).child(leaf(1.0))),
                hash_of(base().min_width(1.0).child(leaf(1.0))),
                hash_of(base().min_height(1.0).child(leaf(1.0))),
                hash_of(base().max_width(99.0).child(leaf(1.0))),
                hash_of(base().max_height(99.0).child(leaf(1.0))),
                hash_of(base().handle_thickness(9.0).child(leaf(1.0))),
                hash_of(base().handle_outset(3.0).child(leaf(1.0))),
                hash_of(base().direction(Direction::LEFT).child(leaf(1.0))),
                hash_of(base().child(leaf(2.0))),
            ],
        );
        assert_eq!(hash_of(base().child(Unhashable)), None);
    }

    /// Callbacks are compared by identity.
    #[test]
    fn resizable_hashes_its_callbacks_by_identity() {
        let callback: Callback<ResolvedSize> = Callback::from(|_: ResolvedSize| {});
        let with = |callback: Callback<ResolvedSize>| hash_of(Resizable::new().on_resize(callback).child(leaf(1.0)));
        assert_eq!(with(callback.clone()), with(callback.clone()));
        assert_all_distinct(with(callback.clone()), vec![with(Callback::from(|_: ResolvedSize| {}))]);
        drop(callback);
    }
}

mod materials {
    use aimer_color::prelude::Color;
    use aimer_container::{Glass, GlassMaterial, Liquid, LiquidMaterial, MaterialMotionPolicy};
    use aimer_decoration::{BoxDecoration, Decoration};

    use super::*;

    #[test]
    fn glass_hashes_its_material_decoration_and_child() {
        let glass = || Glass::new().blur_radius(8.0);
        assert_eq!(hash_of(glass().child(leaf(1.0))), hash_of(glass().child(leaf(1.0))));
        assert_all_distinct(
            hash_of(glass().child(leaf(1.0))),
            vec![
                hash_of(Glass::new().blur_radius(9.0).child(leaf(1.0))),
                hash_of(glass().tint(Color::RED).child(leaf(1.0))),
                hash_of(glass().opacity(0.5).child(leaf(1.0))),
                hash_of(glass().saturation(2.0).child(leaf(1.0))),
                hash_of(glass().brightness(2.0).child(leaf(1.0))),
                hash_of(glass().contrast(2.0).child(leaf(1.0))),
                hash_of(glass().border_width(2.0).child(leaf(1.0))),
                hash_of(glass().border_color(Color::RED).child(leaf(1.0))),
                hash_of(glass().shadow_color(Color::RED).child(leaf(1.0))),
                hash_of(glass().shadow_blur(2.0).child(leaf(1.0))),
                hash_of(glass().elevation(2.0).child(leaf(1.0))),
                hash_of(glass().edge_lighting(0.5).child(leaf(1.0))),
                hash_of(glass().specular_highlight(0.5).child(leaf(1.0))),
                hash_of(glass().quality(1).child(leaf(1.0))),
                hash_of(glass().motion_policy(MaterialMotionPolicy::Reduced).child(leaf(1.0))),
                hash_of(glass().decoration(BoxDecoration::new()).child(leaf(1.0))),
                hash_of(glass().child(leaf(2.0))),
            ],
        );
        assert_eq!(hash_of(glass().child(Unhashable)), None);
        assert_eq!(
            hash_of(glass().material(GlassMaterial::new().blur_radius(8.0)).child(leaf(1.0))),
            hash_of(glass().child(leaf(1.0)))
        );
    }

    #[test]
    fn glass_over_a_decoration_without_a_hash_has_none() {
        struct Plain;
        impl Decoration for Plain {
            fn paint(&self, _: &aimer_canvas::Canvas, _: aimer_attribute::Vec2d, _: aimer_attribute::ResolvedSize, _: f32) {}
        }
        assert_eq!(hash_of(Glass::new().decoration(Plain).child(leaf(1.0))), None);
    }

    #[test]
    fn liquid_hashes_its_material_decoration_and_child() {
        let liquid = || Liquid::new().distortion_strength(0.5);
        assert_eq!(hash_of(liquid().child(leaf(1.0))), hash_of(liquid().child(leaf(1.0))));
        assert_all_distinct(
            hash_of(liquid().child(leaf(1.0))),
            vec![
                hash_of(Liquid::new().distortion_strength(0.6).child(leaf(1.0))),
                hash_of(liquid().blur_radius(9.0).child(leaf(1.0))),
                hash_of(liquid().tint(Color::RED).child(leaf(1.0))),
                hash_of(liquid().edge_lighting(0.5).child(leaf(1.0))),
                hash_of(liquid().specular_highlight(0.5).child(leaf(1.0))),
                hash_of(liquid().animation_speed(2.0).child(leaf(1.0))),
                hash_of(liquid().animation_time(2.0).child(leaf(1.0))),
                hash_of(liquid().interaction(0.5).child(leaf(1.0))),
                hash_of(liquid().blob_amount(0.5).child(leaf(1.0))),
                hash_of(liquid().blob_seed(2.0).child(leaf(1.0))),
                hash_of(liquid().magnification(0.5).child(leaf(1.0))),
                hash_of(liquid().tip_pull(0.5).child(leaf(1.0))),
                hash_of(liquid().chromatic_aberration(0.5).child(leaf(1.0))),
                hash_of(liquid().bevel_radius(2.0).child(leaf(1.0))),
                hash_of(liquid().decoration(BoxDecoration::new()).child(leaf(1.0))),
                hash_of(liquid().child(leaf(2.0))),
            ],
        );
        assert_eq!(hash_of(liquid().child(Unhashable)), None);
        // Glass and Liquid over the same child are different widgets.
        assert_ne!(hash_of(Glass::new().child(leaf(1.0))), hash_of(Liquid::new().child(leaf(1.0))));
        let _ = LiquidMaterial::new();
    }
}
