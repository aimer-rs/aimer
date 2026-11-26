//! A theme reports a content hash, so the widgets that carry one can too.
//!
//! `#[derive(Theme)]` builds the hash from the fields: floats, integers, colors
//! and anything that is `Hash` or implements `HashContent` count. A theme with a
//! field that offers neither has no hash, which is the safe answer.

use std::time::Duration;

use aimer_animation::{Animatable, curves};
use aimer_color::prelude::Color;
use aimer_style::{AnimatedTheme, Theme, ThemeData, ThemeMode, ThemeTokens};
use aimer_widget::base::BuildContext;
use aimer_widget::testing::assert_hashes_distinct;
use aimer_widget::{AnyElement, ContentHash, ErrorWidget, Widget};

#[derive(Clone, Debug, PartialEq, Theme)]
struct AppTheme {
    accent: f32,
    radius: i32,
    color: Color,
    pair: (f32, f32),
}

fn app(accent: f32, radius: i32, color: Color) -> AppTheme {
    AppTheme { accent, radius, color, pair: (1.0, 2.0) }
}

#[test]
fn a_derived_theme_hashes_every_field() {
    let base = || app(1.0, 2, Color::RED);
    assert!(base().content_hash().is_some());
    assert_eq!(base().content_hash(), base().content_hash());
    assert_hashes_distinct(
        base().content_hash(),
        &[
            app(1.5, 2, Color::RED).content_hash(),
            app(1.0, 3, Color::RED).content_hash(),
            app(1.0, 2, Color::BLUE).content_hash(),
            AppTheme { pair: (2.0, 1.0), ..base() }.content_hash(),
        ],
    );
}

/// A field that is only `Hash` still counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, aimer_macro::Animatable)]
#[animatable(discrete)]
enum Density {
    Compact,
    Roomy,
}

#[derive(Clone, Debug, PartialEq, Theme)]
struct WithHashField {
    density: Density,
    scale: f32,
}

#[test]
fn a_field_that_is_hash_counts() {
    let theme = |density| WithHashField { density, scale: 1.0 };
    assert!(theme(Density::Compact).content_hash().is_some());
    assert_hashes_distinct(theme(Density::Compact).content_hash(), &[theme(Density::Roomy).content_hash()]);
}

/// A field that can be neither hashed by content nor by `Hash` leaves the theme
/// without a hash, since leaving it out would hide a change.
#[derive(Clone, Debug, PartialEq, aimer_macro::Animatable)]
struct Opaque(f32);

#[derive(Clone, Debug, PartialEq, Theme)]
struct WithOpaqueField {
    scale: f32,
    opaque: Opaque,
}

#[test]
fn a_field_that_cannot_be_hashed_leaves_the_theme_without_a_hash() {
    assert_eq!(WithOpaqueField { scale: 1.0, opaque: Opaque(1.0) }.content_hash(), None);
}

#[test]
fn the_built_in_themes_hash_their_data() {
    assert_hashes_distinct(ThemeData::light().content_hash(), &[ThemeData::dark().content_hash()]);
    assert_hashes_distinct(ThemeTokens::light().content_hash(), &[ThemeTokens::dark().content_hash()]);
    assert_eq!(ThemeData::light().content_hash(), ThemeData::light().content_hash());
    assert_eq!(ThemeTokens::light().content_hash(), ThemeTokens::light().content_hash());
}

/// A theme that says nothing has no hash.
#[test]
fn a_theme_that_does_not_say_has_none() {
    #[derive(Clone, PartialEq)]
    struct Plain(f32);
    impl Animatable for Plain {
        fn lerp(&self, other: &Self, t: f32) -> Self {
            Plain(self.0.lerp(&other.0, t))
        }
    }
    impl Theme for Plain {}
    assert_eq!(Plain(1.0).content_hash(), None);
}

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
fn animated_theme_hashes_its_themes_settings_and_child() {
    let theme = || AnimatedTheme::new().data(app(1.0, 2, Color::RED));
    assert!(hash_of(theme().child(leaf())).is_some());
    assert_eq!(hash_of(theme().child(leaf())), hash_of(theme().child(leaf())));
    assert_hashes_distinct(
        hash_of(theme().child(leaf())),
        &[
            hash_of(AnimatedTheme::new().data(app(2.0, 2, Color::RED)).child(leaf())),
            hash_of(theme().dark(app(1.0, 2, Color::BLUE)).child(leaf())),
            hash_of(theme().mode(ThemeMode::Dark).child(leaf())),
            hash_of(theme().duration(Duration::from_millis(5)).child(leaf())),
            hash_of(theme().curve(curves::EASE_IN).child(leaf())),
            hash_of(theme().child(ErrorWidget::new("y"))),
        ],
    );
    assert_eq!(hash_of(theme().child(Unhashable)), None);
}

#[test]
fn animated_theme_over_an_unhashable_theme_has_no_hash() {
    let theme = WithOpaqueField { scale: 1.0, opaque: Opaque(1.0) };
    assert_eq!(hash_of(AnimatedTheme::new().data(theme).child(leaf())), None);
}

#[test]
fn the_default_theme_is_hashable() {
    assert!(hash_of(AnimatedTheme::new().child(leaf())).is_some());
    let adaptive = |light: ThemeData, dark: ThemeData| hash_of(AnimatedTheme::new().adaptive(light, dark).child(leaf()));
    assert_hashes_distinct(
        adaptive(ThemeData::light(), ThemeData::dark()),
        &[adaptive(ThemeData::dark(), ThemeData::light())],
    );
}
