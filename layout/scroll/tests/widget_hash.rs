//! Scrollable hashes everything its element is built from. Its controller is a
//! shared handle, so it is compared by identity.

use aimer_attribute::Dimension;
use aimer_widget::base::Colors;
use aimer_container::SizedBox;
use aimer_scroll::{
    OverscrollSources, ScrollAxis, ScrollBar, ScrollBarPlacement, ScrollBehavior, ScrollController, Scrollable,
};
use aimer_widget::base::BuildContext;
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

fn leaf(width: f32) -> SizedBox {
    SizedBox::new().width(width)
}

/// One call site, so every scrollable here has the same default key; a key
/// taken from the caller's location would otherwise tell them apart.
fn scrollable(configure: impl FnOnce(Scrollable) -> Scrollable, child: impl Widget + 'static) -> Option<ContentHash> {
    hash_of(configure(Scrollable::new()).child(child))
}

#[test]
fn scrollable_hashes_every_setting_and_its_child() {
    let base = || scrollable(|s| s, leaf(1.0));
    assert_eq!(base(), base());
    assert_hashes_distinct(
        base(),
        &[
            scrollable(|s| s.axis(ScrollAxis::Horizontal), leaf(1.0)),
            scrollable(|s| s.reverse(true), leaf(1.0)),
            scrollable(|s| s.scroll_bar_placement(ScrollBarPlacement::Inline), leaf(1.0)),
            scrollable(|s| s.scroll_behavior(ScrollBehavior::no_bounce()), leaf(1.0)),
            scrollable(|s| s.web_overscroll(OverscrollSources::ALL), leaf(1.0)),
            scrollable(|s| s.vertical_scroll_bar(None), leaf(1.0)),
            scrollable(|s| s.horizontal_scroll_bar(None), leaf(1.0)),
            scrollable(|s| s.key("k"), leaf(1.0)),
            scrollable(|s| s, leaf(2.0)),
        ],
    );
    assert_eq!(scrollable(|s| s, Unhashable), None);
}

#[test]
fn scroll_bars_hash_their_appearance() {
    let bar = |f: &dyn Fn(&mut ScrollBar)| {
        let mut bar = ScrollBar::default();
        f(&mut bar);
        scrollable(|s| s.vertical_scroll_bar(Some(bar.clone())), leaf(1.0))
    };
    assert_eq!(bar(&|_| {}), bar(&|_| {}));
    assert_hashes_distinct(
        bar(&|_| {}),
        &[
            bar(&|b| b.track.width = Dimension::Px(9.0)),
            bar(&|b| b.track.hover_color = Colors::Red),
            bar(&|b| b.thumb.width = Dimension::Px(9.0)),
            bar(&|b| b.thumb.radius = Dimension::Px(1.0)),
            bar(&|b| b.thumb.color = Colors::Blue),
            bar(&|b| b.thumb.active_color = Colors::Blue),
        ],
    );
}

#[test]
fn scrollable_hashes_its_controller_by_identity() {
    let controller = ScrollController::default();
    let with = |controller: &ScrollController| scrollable(|s| s.controller(controller.clone()), leaf(1.0));
    assert_eq!(with(&controller), with(&controller));
    assert_hashes_distinct(with(&controller), &[with(&ScrollController::default()), scrollable(|s| s, leaf(1.0))]);
}

/// A bar is a widget too, and hashes what it looks like.
#[test]
fn a_scroll_bar_widget_hashes_its_appearance() {
    let mut wide = ScrollBar::default();
    wide.thumb.width = Dimension::Px(9.0);
    assert_eq!(hash_of(ScrollBar::default()), hash_of(ScrollBar::default()));
    assert_hashes_distinct(hash_of(ScrollBar::default()), &[hash_of(wide)]);
}
