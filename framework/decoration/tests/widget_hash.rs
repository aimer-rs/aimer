//! Decorations report a content hash of what they paint, and the boxes that wear
//! them hash that together with their child, so a part dressed in them can be
//! recognised as unchanged.

use aimer_color::prelude::Color;
use aimer_container::SizedBox;
use aimer_decoration::{
    AnyDecor, BackdropFilter, BorderRadius, BorderSlice, BorderStyle, BoxBorder, BoxDecoration, BoxOutline,
    BoxShadow, Decoration, DecorationBox, ForegroundDecorationBox, Gradient, LightEdge, MaskedDecorationBox,
    Magnification, MultiDecorationBox, OpacityGradient, OpacityMaskBox,
};
use aimer_widget::base::BuildContext;
use aimer_widget::testing::assert_hashes_distinct;
use aimer_widget::{AnyElement, ContentHash, ErrorWidget, Widget};

fn hash_of(widget: impl Widget + 'static) -> Option<ContentHash> {
    Widget::hash(&widget)
}

fn of(decoration: &impl Decoration) -> Option<ContentHash> {
    decoration.content_hash()
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

fn solid() -> BorderSlice {
    BorderSlice::new().style(BorderStyle::Solid).stroke(2.0).color(Color::RED)
}

#[test]
fn a_color_hashes_its_value() {
    assert_eq!(of(&Color::RED), of(&Color::RED));
    assert_hashes_distinct(of(&Color::RED), &[of(&Color::BLUE), of(&Color::Rgba(255, 0, 0, 128))]);
}

#[test]
fn border_and_outline_hash_every_side() {
    assert_eq!(of(&BoxBorder::all(solid())), of(&BoxBorder::all(solid())));
    assert_hashes_distinct(
        of(&BoxBorder::all(solid())),
        &[
            of(&BoxBorder::new().left(solid())),
            of(&BoxBorder::new().top(solid())),
            of(&BoxBorder::all(solid().stroke(3.0))),
            of(&BoxBorder::all(solid().color(Color::BLUE))),
            of(&BoxBorder::all(solid().style(BorderStyle::Dashed))),
        ],
    );
    // Border and outline are different decorations over the same data.
    assert_ne!(of(&BoxBorder::all(solid())), of(&BoxOutline::all(solid())));
    assert_hashes_distinct(of(&BoxOutline::all(solid())), &[of(&BoxOutline::new().left(solid()))]);
}

#[test]
fn radius_and_shadow_hash_every_field() {
    assert_hashes_distinct(of(&BorderRadius::all(4.0)), &[of(&BorderRadius::all(5.0))]);
    let shadow = || BoxShadow::new().offset_x(1.0).blur(2.0);
    assert_eq!(of(&shadow()), of(&shadow()));
    assert_hashes_distinct(
        of(&shadow()),
        &[
            of(&shadow().offset_x(2.0)),
            of(&shadow().offset_y(2.0)),
            of(&shadow().blur(3.0)),
            of(&shadow().spread(3.0)),
            of(&shadow().color(Color::BLUE)),
            of(&shadow().inset(true)),
        ],
    );
}

#[test]
fn box_decoration_hashes_everything_it_paints() {
    let base = || BoxDecoration::new().border_radius(BorderRadius::all(4.0));
    assert_eq!(of(&base()), of(&base()));
    assert_hashes_distinct(
        of(&base()),
        &[
            of(&base().border(BoxBorder::all(solid()))),
            of(&base().outline(BoxOutline::all(solid()))),
            of(&BoxDecoration::new().border_radius(BorderRadius::all(5.0))),
            of(&base().add_shadow(BoxShadow::new())),
            of(&base().background_color(Color::RED)),
        ],
    );
}

#[test]
fn wrappers_forward_the_hash_of_what_they_hold() {
    let decoration = || BoxDecoration::new().background_color(Color::RED);
    assert_eq!(of(&Box::new(decoration())), of(&decoration()));
    assert_eq!(of(&std::rc::Rc::new(decoration())), of(&decoration()));
    assert_eq!(of(&AnyDecor::new(decoration())), of(&decoration()));
}

#[test]
fn effects_hash_their_parameters() {
    assert_hashes_distinct(
        of(&BackdropFilter::new().blur_radius(4.0)),
        &[of(&BackdropFilter::new().blur_radius(5.0)), of(&BackdropFilter::new().blur_radius(4.0).confined(true)), of(&BackdropFilter::new().blur_radius(4.0).saturation(2.0))],
    );
    assert_hashes_distinct(
        of(&Gradient::linear().colors([Color::RED, Color::BLUE])),
        &[
            of(&Gradient::linear().colors([Color::RED, Color::GREEN])),
            of(&Gradient::radial().colors([Color::RED, Color::BLUE])),
            of(&Gradient::linear().colors([Color::RED, Color::BLUE]).angle(45.0)),
        ],
    );
    assert_hashes_distinct(of(&OpacityGradient::linear()), &[of(&OpacityGradient::linear().angle(45.0))]);
    assert_hashes_distinct(of(&LightEdge::new()), &[of(&LightEdge::new().width(3.0))]);
    assert_hashes_distinct(of(&Magnification::new()), &[of(&Magnification::new().zoom(2.0))]);
}

#[test]
fn a_decoration_without_a_hash_stays_without_one() {
    struct Plain;
    impl Decoration for Plain {
        fn paint(&self, _: &aimer_canvas::Canvas, _: aimer_attribute::Vec2d, _: aimer_attribute::ResolvedSize, _: f32) {}
    }
    assert_eq!(of(&Plain), None);
    assert_eq!(hash_of(DecorationBox::new().decoration(Plain).child(leaf())), None);
}

#[test]
fn every_decoration_box_hashes_its_decoration_and_child() {
    let d = || BoxDecoration::new().background_color(Color::RED);
    let e = || BoxDecoration::new().background_color(Color::BLUE);
    let variants: [(&str, Box<dyn Fn(BoxDecoration, ErrorWidget) -> Option<ContentHash>>); 4] = [
        ("DecorationBox", Box::new(|d, c| hash_of(DecorationBox::new().decoration(d).child(c)))),
        ("ForegroundDecorationBox", Box::new(|d, c| hash_of(ForegroundDecorationBox::new().decoration(d).child(c)))),
        ("MaskedDecorationBox", Box::new(|d, c| hash_of(MaskedDecorationBox::new().decoration(d).child(c)))),
        ("OpacityMaskBox", Box::new(|d, c| hash_of(OpacityMaskBox::new().decoration(d).child(c)))),
    ];
    for (name, build) in &variants {
        assert_eq!(build(d(), leaf()), build(d(), leaf()), "{name}");
        assert_hashes_distinct(build(d(), leaf()), &[build(e(), leaf()), build(d(), ErrorWidget::new("y"))]);
    }
    // The same decoration and child under another box type is another widget.
    let hashes: Vec<_> = variants.iter().map(|(_, build)| build(d(), leaf())).collect();
    assert_hashes_distinct(hashes[0], &hashes[1..]);
    assert_eq!(hash_of(DecorationBox::new().decoration(d()).child(Unhashable)), None);
}

#[test]
fn multi_decoration_box_hashes_placement_decoration_and_child() {
    let d = || BoxDecoration::new().background_color(Color::RED);
    let multi = || MultiDecorationBox::new().decoration(d());
    assert_eq!(hash_of(multi().child(leaf())), hash_of(multi().child(leaf())));
    assert_hashes_distinct(
        hash_of(multi().child(leaf())),
        &[
            hash_of(MultiDecorationBox::new().foreground().decoration(d()).child(leaf())),
            hash_of(MultiDecorationBox::new().masked().decoration(d()).child(leaf())),
            hash_of(MultiDecorationBox::new().decoration(BoxDecoration::new().background_color(Color::BLUE)).child(leaf())),
            hash_of(multi().child(ErrorWidget::new("y"))),
        ],
    );
}
