//! A page growing out of the item that opened it: standalone and routed.
//!
//! The first two previews wrap their grid items in `SharedElement`; the page
//! grows out of the tapped item and, on Back, shrinks into it again. The third
//! does the same for the rows of a feed, which are as wide as the page: there
//! the page cannot shrink to the item's width, so it is revealed through a
//! window that grows out of the row instead (`reveal`).
//!
//! Each preview shows a different way to set the motion up. The routed grid
//! keeps the defaults. The standalone grid runs a linear timeline with a curve
//! for each element. The rows play it in three stages (`staged`) and tune every
//! step: how long it takes, how much the row scales first, how the window grows
//! last, how far the page behind dims, how much everything blurs while the row
//! turns into the page, and how round the window's corners are.

use std::rc::Rc;
use std::time::Duration;

use aimer::animation::{curves, SharedElement};
use aimer::input::button::Button;
use aimer::router::{ContainerTransform, ContainerTransition, Navigator, NavigatorController, Router};
use aimer::style::*;
use aimer::*;

use crate::theme;

const TILE: f32 = 150.0;
const PREVIEW_WIDTH: f32 = 332.0;
const PREVIEW_HEIGHT: f32 = 400.0;
const ROW_HEIGHT: f32 = 72.0;
/// How long a row takes to open and to close again.
const ROW_DURATION: Duration = Duration::from_millis(600);

fn album_color(id: u32) -> Color {
    match id % 4 {
        0 => Color::Rgb(36, 62, 91),
        1 => Color::Rgb(77, 44, 88),
        2 => Color::Rgb(27, 78, 67),
        _ => Color::Rgb(94, 61, 32),
    }
}

/// The tag of the tile a standalone album grows out of.
fn standalone_tag(id: u32) -> Key {
    Key::from(format!("standalone-album-{id}"))
}

/// The tag of the tile a routed album grows out of.
fn routed_tag(route: &AlbumRoute) -> Option<Key> {
    match route {
        AlbumRoute::Grid => None,
        AlbumRoute::Album(id) => Some(Key::from(format!("routed-album-{id}"))),
    }
}

/// The tag of the row a revealed page grows out of.
fn row_tag(route: &RowRoute) -> Option<Key> {
    match route {
        RowRoute::Feed => None,
        RowRoute::Row(id) => Some(Key::from(format!("routed-row-{id}"))),
    }
}

fn label(text: String, size: u32, color: Color) -> AnyWidget {
    Text::new(text)
        .text_style(TextStyle::new().font_size(size).font_weight(FontWeight::Bold).color(color))
        .boxed()
}

/// One album tile. `tag` makes it the thing its page grows out of.
///
/// The tile is given as a function so the transition can draw a copy of it and
/// lead the motion with that: scaled up first, then dissolved into the page.
fn tile(id: u32, tag: Key, on_press: impl Fn() + 'static, white: Color) -> AnyWidget {
    let on_press = Rc::new(on_press);
    SharedElement::new().tag(tag).child_with(move || {
        let on_press = Rc::clone(&on_press);
        Button::new()
            .on_press(move || on_press())
            .decoration(BoxDecoration::new().background_color(album_color(id)).border_radius(12))
            .child(
                Container::new()
                    .width(Dimension::Px(TILE))
                    .height(Dimension::Px(TILE))
                    .padding(LayoutSpacing::all(Spacing::Px(12)))
                    .child(label(format!("Album {id}"), 18, white)),
            )
    }).boxed()
}

/// One row of the feed: as wide as the page, so the page is revealed from it.
fn feed_row(id: u32, on_press: impl Fn() + 'static, white: Color) -> AnyWidget {
    let on_press = Rc::new(on_press);
    let tag = row_tag(&RowRoute::Row(id)).expect("rows have tags");
    SharedElement::new().tag(tag).child_with(move || {
        let on_press = Rc::clone(&on_press);
        Button::new()
            .on_press(move || on_press())
            .decoration(BoxDecoration::new().background_color(album_color(id)).border_radius(12))
            // A button is as big as its child, so the row asks for the whole
            // width of its slot here.
            .child(
                Container::new()
                    .width(Dimension::Percent(100.0))
                    .height(Dimension::Px(ROW_HEIGHT))
                    .padding(LayoutSpacing::all(Spacing::Px(12)))
                    .child(label(format!("Row {id}"), 18, white)),
            )
    }).boxed()
}

fn grid(tile_for: impl Fn(u32) -> AnyWidget) -> impl Widget {
    let row = |first: u32| {
        Row::new()
            .gaps(LayoutSpacing::new().right(12))
            .children([tile_for(first), tile_for(first + 1)])
            .boxed()
    };
    Container::new()
        .padding(LayoutSpacing::all(Spacing::Px(8)))
        .child(Column::new().gaps(LayoutSpacing::new().bottom(12)).children([row(0), row(2)]))
}

/// The page an album opens: full size, so it can grow from the tile to fill
/// the preview.
fn album_page(id: u32, close: impl Fn() + 'static, white: Color) -> AnyWidget {
    Container::new()
        .color(album_color(id))
        .padding(LayoutSpacing::all(Spacing::Px(24)))
        .child(
            Column::new()
                .gaps(LayoutSpacing::new().bottom(16))
                .children([
                    label(format!("Album {id}"), 28, white),
                    Text::new("Back shrinks this page into the tile it grew from.")
                        .text_style(TextStyle::new().font_size(15).color(white))
                        .boxed(),
                    Button::new()
                        .on_press(close)
                        .decoration(BoxDecoration::new().background_color(white.with_alpha(0.2)).border_radius(8))
                        .child(
                            Container::new()
                                .padding(LayoutSpacing::all(Spacing::Px(10)))
                                .child(Text::new("Back").text_style(TextStyle::new().font_size(15).color(white))),
                        )
                        .boxed(),
                ]),
        )
        .boxed()
}

fn preview(title: &'static str, note: &'static str, body: impl Widget + 'static, theme: ThemeData) -> AnyWidget {
    Column::new()
        .gaps(LayoutSpacing::new().bottom(8))
        .children([
            label(title.to_owned(), 18, theme.on_background_color),
            Text::new(note)
                .text_style(TextStyle::new().font_size(14).color(theme::muted_text(&theme)))
                .boxed(),
            SizedBox::new()
                .width(PREVIEW_WIDTH as u32)
                .height(PREVIEW_HEIGHT as u32)
                .color(theme::recessed_surface(&theme))
                .child(body)
                .boxed(),
        ])
        .boxed()
}

/// How a row opens, in three stages with every step of the motion set.
///
/// The timeline is linear, and each stage is a third of it. First the row
/// scales up a little (`lift`), easing out. Then it turns into the page, which
/// fades in on it while everything goes out of focus and back (`blur`). Last the
/// page scales to fit the screen as a window that grows from the row, quickly at
/// first and settling slowly. The feed behind dims a little, and the window
/// keeps the row's 12px corners until it fills the page, which is square.
/// Going back plays the stages in the opposite order.
fn row_transition() -> ContainerTransition<RowRoute> {
    ContainerTransition::new()
        .tag(row_tag)
        .staged(true)
        .lift(1.06)
        .duration(ROW_DURATION)
        .curve(curves::LINEAR)
        .anchor_curve(curves::EASE_OUT)
        .page_curve(curves::FAST_OUT_SLOW_IN)
        .scrim(0.25)
        .blur(10.0)
        .source_radius(12.0)
        .page_radius(0.0)
}

/// Builds the container-transform example without starting an application.
pub fn container_transform_example() -> impl Widget {
    ContainerTransformExample
}
#[derive(StatelessWidget)]
struct ContainerTransformExample;

impl StatelessWidget for ContainerTransformExample {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        Container::new()
            .color(theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(28)))
            .child(
                Column::new()
                    .gaps(LayoutSpacing::new().bottom(20))
                    .children([
                        label("Container transform".to_owned(), 26, theme.on_background_color),
                        Row::new()
                            .gaps(LayoutSpacing::new().right(28))
                            .children([
                                preview(
                                    "Standalone",
                                    "ContainerTransform: no router involved.",
                                    StandalonePreview,
                                    theme,
                                ),
                                preview(
                                    "Routed",
                                    "ContainerTransition on a Navigator.",
                                    Navigator::new(AlbumRoute::Grid, |route| route.boxed())
                                        .transition(ContainerTransition::new().tag(routed_tag)),
                                    theme,
                                ),
                                preview(
                                    "Rows",
                                    "reveal: a window grows out of a full-width row.",
                                    Navigator::new(RowRoute::Feed, |route| route.boxed())
                                        .transition(row_transition()),
                                    theme,
                                ),
                            ])
                            .boxed(),
                    ]),
            )
    }
}

#[derive(StatefulWidget)]
struct StandalonePreview;

struct StandaloneState {
    selected: Option<u32>,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for StandalonePreview {
    type State = StandaloneState;

    fn create_state(self) -> Self::State {
        StandaloneState { selected: None, updater: StateUpdater::empty() }
    }
}

impl State<StandalonePreview> for StandaloneState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        let white = theme.on_primary_color;
        let updater = self.updater;
        let open = move |id: u32| move || updater.set_state(move |state| state.selected = Some(id));
        let close = move || updater.set_state(|state| state.selected = None);

        // A linear timeline with an element curve each: the item copy eases out
        // as it travels to the card, then the page eases in and out as it grows.
        ContainerTransform::new()
            .curve(curves::LINEAR)
            .anchor_curve(curves::EASE_OUT)
            .page_curve(curves::EASE_IN_OUT)
            .page(self.selected.map(|id| (standalone_tag(id), album_page(id, close, white))))
            .child(grid(move |id| tile(id, standalone_tag(id), open(id), white)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Router)]
enum AlbumRoute {
    #[route("/")]
    Grid,
    #[route("/album/{}")]
    Album(u32),
}

impl Router for AlbumRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        match *self {
            Self::Grid => RoutedGrid.boxed(),
            Self::Album(id) => RoutedAlbum { id }.boxed(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Router)]
enum RowRoute {
    #[route("/")]
    Feed,
    #[route("/row/{}")]
    Row(u32),
}

impl Router for RowRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        match *self {
            Self::Feed => RoutedFeed.boxed(),
            Self::Row(id) => RoutedRow { id }.boxed(),
        }
    }
}

#[derive(StatelessWidget)]
struct RoutedFeed;

impl StatelessWidget for RoutedFeed {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        let navigator = NavigatorController::<RowRoute>::of(ctx);
        let white = theme.on_primary_color;
        Container::new()
            .padding(LayoutSpacing::all(Spacing::Px(8)))
            .child(
                Column::new()
                    .gaps(LayoutSpacing::new().bottom(12))
                    .children([0u32, 1, 2, 3].map(|id| {
                        let navigator = navigator.clone();
                        feed_row(id, move || navigator.push(RowRoute::Row(id)), white)
                    })),
            )
    }
}

#[derive(StatelessWidget)]
struct RoutedRow {
    id: u32,
}

impl StatelessWidget for RoutedRow {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        let navigator = NavigatorController::<RowRoute>::of(ctx);
        album_page(self.id, move || navigator.pop(), theme.on_primary_color)
    }
}

#[derive(StatelessWidget)]
struct RoutedGrid;

impl StatelessWidget for RoutedGrid {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        let navigator = NavigatorController::<AlbumRoute>::of(ctx);
        let white = theme.on_primary_color;
        grid(move |id| {
            let navigator = navigator.clone();
            let route = AlbumRoute::Album(id);
            tile(id, routed_tag(&route).expect("albums have tags"), move || navigator.push(route), white)
        })
    }
}

#[derive(StatelessWidget)]
struct RoutedAlbum {
    id: u32,
}

impl StatelessWidget for RoutedAlbum {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        let navigator = NavigatorController::<AlbumRoute>::of(ctx);
        album_page(self.id, move || navigator.pop(), theme.on_primary_color)
    }
}

#[cfg(test)]
mod tests {
    use aimer::animation::{ElementBounds, last_shared_element_bounds, shared_element_bounds};
    use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
    use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    use aimer::{AimerApp, HeadlessOptions};

    use super::*;

    const WINDOW: (u32, u32) = (1500, 700);

    fn click(app: &mut aimer::HeadlessAimerApp<impl Widget + 'static>, at: (f32, f32)) {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(at.0 as f64, at.1 as f64),
        });
        for state in [ElementState::Pressed, ElementState::Released] {
            app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: MouseButton::Left,
            });
        }
    }

    /// Pumps frames until the app stops asking for more, returning how many it
    /// rendered. An animation shows up as a run of frames.
    fn settle(app: &mut aimer::HeadlessAimerApp<impl Widget + 'static>) -> usize {
        let mut frames = 0;
        for _ in 0..200 {
            std::thread::sleep(Duration::from_millis(10));
            if app.pump_frames(1) == 0 {
                break;
            }
            frames += 1;
        }
        frames
    }

    fn app() -> aimer::HeadlessAimerApp<impl Widget + 'static> {
        let mut app = AimerApp::start_headless_with(
            theme::provide(container_transform_example()),
            HeadlessOptions { size: PhysicalSize::new(WINDOW.0, WINDOW.1), scale_factor: 1.0 },
        );
        app.pump_frames(6);
        app
    }

    /// Every element paints through the retained path and the render tree syncs:
    /// the same rule the showcase holds every page to, here for a frame taken
    /// while the copy of the item is on screen.
    fn assert_retained_paint(app: &aimer::HeadlessAimerApp<impl Widget + 'static>, when: &str) {
        let census = app.paint_source_census().expect("a mounted page has a root");
        assert!(census.declined_paint.is_empty(), "{when}: declined {:?}", census.declined_paint.iter().map(|e| e.debug_name).collect::<Vec<_>>());
        assert!(census.drawn_unmapped.is_empty(), "{when}: unmapped {:?}", census.drawn_unmapped.iter().map(|e| e.debug_name).collect::<Vec<_>>());
        assert!(census.invalid_bounds.is_empty(), "{when}: invalid bounds {:?}", census.invalid_bounds.iter().map(|e| e.debug_name).collect::<Vec<_>>());
        assert!(census.sync_error.is_none(), "{when}: {:?}", census.sync_error);
    }

    fn center(bounds: ElementBounds) -> (f32, f32) {
        (bounds.x + bounds.width / 2.0, bounds.y + bounds.height / 2.0)
    }

    fn near(actual: f32, expected: f32, what: &str) {
        assert!((actual - expected).abs() < 1.5, "{what}: {actual} != {expected}");
    }

    #[test]
    fn tiles_are_registered_where_they_are_laid_out() {
        let _app = app();

        for tag in [standalone_tag as fn(u32) -> Key, |id| routed_tag(&AlbumRoute::Album(id)).unwrap()] {
            let [a, b, c, d] = [0, 1, 2, 3].map(|id| {
                shared_element_bounds(&tag(id)).unwrap_or_else(|| panic!("tile {id} is not registered"))
            });
            for tile in [a, b, c, d] {
                near(tile.width, TILE, "tile width");
                near(tile.height, TILE, "tile height");
                assert!(tile.x >= 0.0 && tile.y >= 0.0, "{tile:?}");
                assert!(tile.x + tile.width <= WINDOW.0 as f32 && tile.y + tile.height <= WINDOW.1 as f32, "{tile:?}");
            }
            // A 2x2 grid with a 12px gap.
            near(b.x - a.x, TILE + 12.0, "column pitch");
            near(c.y - a.y, TILE + 12.0, "row pitch");
            near(b.y, a.y, "same row");
            near(c.x, a.x, "same column");
            assert!(d.x > c.x && d.y > b.y);
        }
    }

    #[test]
    fn tapping_a_standalone_tile_plays_an_animation_that_then_settles() {
        let mut app = app();
        let tile = shared_element_bounds(&standalone_tag(3)).expect("tile 3 is registered");

        click(&mut app, center(tile));
        // Mid-motion the page and the copy of the tile are both on screen.
        app.pump_frames(3);
        assert_retained_paint(&app, "standalone, mid-motion");
        let opening = settle(&mut app);

        assert!(opening > 5, "opening should animate over several frames, got {opening}");
        // Nothing keeps requesting frames once the page is open.
        assert_eq!(app.pump_frames(1), 0);
    }

    #[test]
    fn a_routed_push_leaves_the_tapped_tile_remembered_for_the_way_back() {
        let mut app = app();
        let tag = routed_tag(&AlbumRoute::Album(2)).unwrap();
        let tile = shared_element_bounds(&tag).expect("tile 2 is registered");

        click(&mut app, center(tile));
        app.pump_frames(3);
        assert_retained_paint(&app, "routed, mid-motion");
        let pushing = settle(&mut app);

        assert!(pushing > 5, "the push should animate over several frames, got {pushing}");
        // The grid page is gone once the push finishes, but Back still needs to
        // know where this tile was.
        let remembered = last_shared_element_bounds(&tag).expect("the tile was remembered");
        near(remembered.x, tile.x, "remembered x");
        near(remembered.y, tile.y, "remembered y");
    }

    #[test]
    fn a_routed_round_trip_puts_the_tile_back_in_its_slot() {
        let mut app = app();
        let tag = routed_tag(&AlbumRoute::Album(1)).unwrap();
        let tile = shared_element_bounds(&tag).expect("tile 1 is registered");
        // The preview starts 8px (the grid's padding) up and left of tile 0.
        let origin = shared_element_bounds(&routed_tag(&AlbumRoute::Album(0)).unwrap()).unwrap();
        let (left, top) = (origin.x - 8.0, origin.y - 8.0);

        click(&mut app, center(tile));
        assert!(settle(&mut app) > 5, "the push animates");

        // Find the Back button of the album page, which fills the preview: it is
        // under the title and the caption, inside the page's 24px padding.
        let mut went_back = false;
        for step in 0..30 {
            let at = (left + 24.0 + 24.0, top + 60.0 + step as f32 * 8.0);
            click(&mut app, at);
            app.pump_frames(3);
            if settle(&mut app) > 5 {
                went_back = true;
                break;
            }
        }
        assert!(went_back, "pressing Back should play the pop");

        // The grid is rebuilt, and its tile is exactly where it was.
        app.pump_frames(3);
        let back = shared_element_bounds(&tag).expect("the grid registered its tile again");
        near(back.x, tile.x, "x after the round trip");
        near(back.y, tile.y, "y after the round trip");
        assert_retained_paint(&app, "after the round trip");
    }

    #[test]
    fn tiles_can_be_drawn_again_so_the_transition_can_lead_with_them() {
        let _app = app();

        for id in 0..4 {
            let snapshot = aimer::animation::last_shared_element(&standalone_tag(id)).expect("registered");
            assert!(snapshot.content.is_some(), "tile {id} should be repeatable");
        }
    }

    #[test]
    fn a_click_that_misses_every_tile_animates_nothing() {
        // The control for the tests above: without it, a run of frames after a
        // tap could be a button's press feedback rather than the page opening.
        let mut app = app();

        click(&mut app, (2.0, 2.0));

        assert!(settle(&mut app) <= 3);
    }

    #[test]
    fn standalone_and_routed_tiles_use_different_tags() {
        // Both previews live on one page, so a shared tag would make the
        // registry answer with whichever grid synchronized last.
        for id in 0..4 {
            assert_ne!(standalone_tag(id), routed_tag(&AlbumRoute::Album(id)).unwrap());
        }
        assert!(routed_tag(&AlbumRoute::Grid).is_none());
    }

    #[test]
    fn rows_are_registered_as_wide_as_the_page_they_open() {
        let _app = app();

        let rows = [0, 1, 2, 3].map(|id| {
            shared_element_bounds(&row_tag(&RowRoute::Row(id)).unwrap())
                .unwrap_or_else(|| panic!("row {id} is not registered"))
        });
        for row in rows {
            near(row.height, ROW_HEIGHT, "row height");
            // The grid's 8px of padding either side of a 332px preview.
            near(row.width, PREVIEW_WIDTH - 16.0, "row width");
        }
        near(rows[1].y - rows[0].y, ROW_HEIGHT + 12.0, "row pitch");
    }

    #[test]
    fn tapping_a_row_reveals_the_page_and_going_back_returns_to_the_row() {
        let mut app = app();
        let tag = row_tag(&RowRoute::Row(2)).unwrap();
        let row = shared_element_bounds(&tag).expect("row 2 is registered");

        click(&mut app, center(row));
        app.pump_frames(3);
        assert_retained_paint(&app, "rows, mid-motion");
        assert!(settle(&mut app) > 5, "the push animates");

        // The list is gone once the push finishes, but Back still knows the row.
        let remembered = last_shared_element_bounds(&tag).expect("the row was remembered");
        near(remembered.x, row.x, "remembered x");
        near(remembered.y, row.y, "remembered y");
    }

    #[test]
    fn albums_have_distinct_colors_within_a_grid() {
        let colors: Vec<_> = (0..4).map(album_color).collect();
        for (index, color) in colors.iter().enumerate() {
            assert!(!colors[index + 1..].contains(color));
        }
    }
}
