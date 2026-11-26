//! Grid hashes its tracks, gaps, alignment, overflow and every item's placement
//! and child, so a part made of it can be recognised as unchanged.

use aimer_container::SizedBox;
use aimer_grid::{Grid, GridAlignment, GridItem, GridOverflow, GridTrack};
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

fn cell(width: f32) -> GridItem<SizedBox> {
    GridItem::new(SizedBox::new().width(width))
}

fn base() -> Grid<SizedBox> {
    Grid::new()
        .columns([GridTrack::Px(10.0), GridTrack::Fr(1.0)])
        .rows([GridTrack::Auto])
        .children([cell(1.0), cell(2.0)])
}

#[test]
fn grid_hashes_every_setting() {
    assert_eq!(hash_of(base()), hash_of(base()));
    let rebuilt = |grid: Grid<SizedBox>| grid;
    let with = |f: &dyn Fn(Grid<SizedBox>) -> Grid<SizedBox>| hash_of(rebuilt(f(Grid::new()
        .columns([GridTrack::Px(10.0), GridTrack::Fr(1.0)])
        .rows([GridTrack::Auto])
        .children([cell(1.0), cell(2.0)]))));
    assert_hashes_distinct(
        hash_of(base()),
        &[
            with(&|g| g.column_gap(4.0)),
            with(&|g| g.row_gap(4.0)),
            with(&|g| g.horizontal_alignment(GridAlignment::Center)),
            with(&|g| g.vertical_alignment(GridAlignment::End)),
            with(&|g| g.overflow(GridOverflow::Visible)),
            hash_of(Grid::new().columns([GridTrack::Px(11.0), GridTrack::Fr(1.0)]).rows([GridTrack::Auto]).children([cell(1.0), cell(2.0)])),
            hash_of(Grid::new().columns([GridTrack::Px(10.0), GridTrack::Fr(2.0)]).rows([GridTrack::Auto]).children([cell(1.0), cell(2.0)])),
            hash_of(Grid::new().columns([GridTrack::Px(10.0), GridTrack::Auto]).rows([GridTrack::Auto]).children([cell(1.0), cell(2.0)])),
            hash_of(Grid::new().columns([GridTrack::Px(10.0), GridTrack::Fr(1.0)]).rows([GridTrack::Px(5.0)]).children([cell(1.0), cell(2.0)])),
        ],
    );
}

/// A track's number under another unit is another track.
#[test]
fn tracks_with_equal_numbers_differ() {
    let with = |track| hash_of(Grid::new().columns([track]).children([cell(1.0)]));
    assert_hashes_distinct(with(GridTrack::Px(1.0)), &[with(GridTrack::Fr(1.0))]);
}

#[test]
fn grid_hashes_its_items_in_order_with_their_placement() {
    let grid = |items: Vec<GridItem<SizedBox>>| hash_of(Grid::new().columns([GridTrack::Fr(1.0)]).children(items));
    assert_hashes_distinct(
        grid(vec![cell(1.0), cell(2.0)]),
        &[
            grid(vec![cell(2.0), cell(1.0)]),
            grid(vec![cell(1.0)]),
            grid(vec![cell(1.0), cell(2.0).row(1)]),
            grid(vec![cell(1.0), cell(2.0).column(0)]),
            grid(vec![cell(1.0), cell(2.0).row_span(2)]),
            grid(vec![cell(1.0), cell(2.0).column_span(2)]),
            grid(vec![cell(1.0), cell(2.0).horizontal_alignment(GridAlignment::End)]),
            grid(vec![cell(1.0), cell(2.0).vertical_alignment(GridAlignment::End)]),
            grid(vec![cell(1.0), cell(3.0)]),
        ],
    );
}

#[test]
fn an_unhashable_item_leaves_the_grid_unhashable() {
    assert_eq!(hash_of(Grid::new().columns([GridTrack::Fr(1.0)]).children([GridItem::new(Unhashable)])), None);
}
