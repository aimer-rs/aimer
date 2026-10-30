//! A scrollable list of rows that differ in height must end where its rows end.
//!
//! A windowed list predicts every row from one probe and corrects the rows it
//! paints, so its total shrinks as it is scrolled. The scroll view has to follow
//! that total even when a container stands between it and the list, or its range
//! outlives the content and the end of the page is blank.

use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::style::LayoutSpacing;
use aimer::{
    AimerApp, Column, Container, HeadlessOptions, ScrollController, Scrollable, SizedBox, Vec2d,
    Widget,
};

const VIEWPORT: f32 = 600.0;
/// Two tall rows first, so the probe row overstates the rest.
const TALL: f32 = 400.0;
const SHORT: f32 = 50.0;
const ROWS: u32 = 40;

fn row_height(index: u32) -> f32 {
    if index < 2 { TALL } else { SHORT }
}

fn list() -> impl Widget {
    Column::new()
        .list((0..ROWS).collect::<Vec<u32>>())
        .builder(|index| SizedBox::new().width(400).height(row_height(*index)))
}

/// Walks the page from the top to the end in half-viewport steps, so that every
/// row is painted once, and returns the range it ends with.
///
/// A windowed list is exact only about the rows it has painted, so anything less
/// than a full walk would be asking it for what it cannot know.
fn walked_max_extent(page: impl Widget + 'static, controller: &ScrollController) -> f32 {
    let mut app = AimerApp::start_headless_with(
        page,
        HeadlessOptions {
            size: PhysicalSize::new(400, VIEWPORT as u32),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    let mut offset = 0.0;
    for _ in 0..400 {
        controller.jump_to(Vec2d { x: 0.0, y: offset });
        app.pump_frames(4);
        let max = controller.max_extent().y;
        if offset >= max {
            break;
        }
        offset = (offset + VIEWPORT / 2.0).min(max);
    }
    // One more pass at the end, in case the last rows moved it.
    controller.jump_to(Vec2d { x: 0.0, y: controller.max_extent().y });
    app.pump_frames(4);
    controller.max_extent().y
}

fn exact_max_extent() -> f32 {
    (0..ROWS).map(row_height).sum::<f32>() - VIEWPORT
}

#[test]
fn a_list_directly_in_a_scrollable_ends_where_its_rows_end() {
    let controller = ScrollController::new();
    let page = Scrollable::new().controller(controller.clone()).child(list());

    assert_eq!(walked_max_extent(page, &controller), exact_max_extent());
}

#[test]
fn a_list_inside_a_container_in_a_scrollable_ends_where_its_rows_end() {
    let controller = ScrollController::new();
    let page = Scrollable::new().controller(controller.clone()).child(
        Container::new()
            .padding(LayoutSpacing::new().right(40))
            .child(list()),
    );

    assert_eq!(walked_max_extent(page, &controller), exact_max_extent());
}

#[test]
fn a_list_below_a_header_in_a_column_in_a_scrollable_ends_where_its_rows_end() {
    let controller = ScrollController::new();
    let page = Scrollable::new().controller(controller.clone()).child(
        Container::new().child(Column::new().children([
            SizedBox::new().height(60).boxed(),
            list().boxed(),
            SizedBox::new().height(10).boxed(),
        ])),
    );

    assert_eq!(
        walked_max_extent(page, &controller),
        exact_max_extent() + 70.0
    );
}
