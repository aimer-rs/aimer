//! One region per cursor.
//!
//! Every tile is a [`MouseRegion`] that asks for its own cursor icon. The icon
//! changes when the pointer enters the tile and goes back to the default when it
//! leaves, and the line above the grid names the tile the pointer is over.
//!
//! A region sets its icon once, on entering, and claims every move over it, so the
//! application does not reset the cursor to the default one move later: the shape
//! holds for as long as the pointer stays inside, however much it moves.
//!
//! The tiles are the cursors a driven app's virtual mouse can draw as well (see
//! `aimer_prendre`), which makes this page the place to see them change.

use aimer::input::mouse_region::MouseRegion;
use aimer::quiver::winit::window::CursorIcon;
use aimer::style::*;
use aimer::{AimerApp, *};

/// How wide a tile is drawn, in logical pixels.
const TILE_WIDTH: f32 = 150.0;
/// How tall a tile is drawn, in logical pixels.
const TILE_HEIGHT: f32 = 72.0;
/// How many tiles stand in a row.
const TILES_PER_ROW: usize = 4;
/// The border is always this thick and only its colour changes, so hovering a tile
/// cannot move its text.
const BORDER_STROKE: f32 = 2.0;

/// A tile: the cursor it asks for, and the name painted in it.
pub(crate) struct Tile {
    name: &'static str,
    icon: CursorIcon,
}

/// The tiles, in reading order: pointing and text, then dragging, then the states a
/// program is in, then the resize arrows.
pub(crate) const TILES: [Tile; 16] = [
    Tile { name: "Pointer", icon: CursorIcon::Pointer },
    Tile { name: "Text", icon: CursorIcon::Text },
    Tile { name: "Crosshair", icon: CursorIcon::Crosshair },
    Tile { name: "Help", icon: CursorIcon::Help },
    Tile { name: "Grab", icon: CursorIcon::Grab },
    Tile { name: "Grabbing", icon: CursorIcon::Grabbing },
    Tile { name: "Move", icon: CursorIcon::Move },
    Tile { name: "Zoom in", icon: CursorIcon::ZoomIn },
    Tile { name: "Wait", icon: CursorIcon::Wait },
    Tile { name: "Progress", icon: CursorIcon::Progress },
    Tile { name: "Not allowed", icon: CursorIcon::NotAllowed },
    Tile { name: "Zoom out", icon: CursorIcon::ZoomOut },
    Tile { name: "Resize ↔", icon: CursorIcon::EwResize },
    Tile { name: "Resize ↕", icon: CursorIcon::NsResize },
    Tile { name: "Resize ⤢", icon: CursorIcon::NeswResize },
    Tile { name: "Resize ⤡", icon: CursorIcon::NwseResize },
];

/// What the readout above the grid says while the pointer is over `hovered`.
fn readout(hovered: Option<&str>) -> String {
    match hovered {
        Some(name) => format!("Cursor: {name}"),
        None => "Cursor: default".to_owned(),
    }
}

/// A grid of regions that each change the cursor.
#[derive(StatefulWidget)]
pub struct CursorsExample;

impl CursorsExample {
    /// Creates the showcase.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CursorsExample {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// The retained side of the example: the tile the pointer is over.
pub struct CursorsExampleState {
    updater: StateUpdater<Self>,
    hovered: Option<&'static str>,
}

impl CursorsExampleState {
    /// One tile: a region asking for the tile's cursor around a decorated box.
    fn tile(&self, tile: &'static Tile) -> AnyWidget {
        let app_theme = crate::theme::app_theme();
        let hovered = self.hovered == Some(tile.name);
        let (enter, exit, name) = (self.updater, self.updater, tile.name);
        let border = if hovered { app_theme.primary_color } else { Color::Transparent };
        MouseRegion::new()
            .cursor(tile.icon)
            .on_hover_enter(move || enter.set_state(move |state| state.hovered = Some(name)))
            // Leaving one tile can be reported after entering the next, so only the
            // tile that is named gives up the name.
            .on_hover_exit(move || {
                exit.set_state(move |state| {
                    if state.hovered == Some(name) {
                        state.hovered = None;
                    }
                })
            })
            .box_child(
                Container::new()
                    .width(Dimension::Px(TILE_WIDTH))
                    .height(Dimension::Px(TILE_HEIGHT))
                    .box_decoration(
                        BoxDecoration::new()
                            .background_color(if hovered {
                                crate::theme::raised_surface(&app_theme)
                            } else {
                                app_theme.surface_color
                            })
                            .border_radius((12, 12, 12, 12))
                            .border(BoxBorder::all(
                                BorderSlice::new()
                                    .style(BorderStyle::Solid)
                                    .stroke(Stroke::Px(BORDER_STROKE))
                                    .color(border),
                            )),
                    )
                    .box_child(
                        Text::new(tile.name)
                            .text_align(TextAlign::MidCenter)
                            .text_style(TextStyle::new().font_size(16).color(app_theme.on_surface_color)),
                    ),
            )
    }
}

impl State<CursorsExample> for CursorsExampleState {
    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn create(_widget: CursorsExample) -> Self {
        CursorsExampleState { updater: StateUpdater::empty(), hovered: None }
    }

    fn build(this: StateView<Self>, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let rows = TILES
            .chunks(TILES_PER_ROW)
            .map(|row| {
                Row::new()
                    .gaps(LayoutSpacing::new().right(16))
                    .children(row.iter().map(|tile| this.tile(tile)).collect::<Vec<_>>())
                    .boxed()
            })
            .collect::<Vec<_>>();
        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Start)
                    .gaps(LayoutSpacing::new().bottom(16))
                    .children(
                        [
                            Text::new("Each tile is a MouseRegion with a cursor of its own: it changes on entering and goes back on leaving.")
                                .text_style(TextStyle::new().font_size(16).color(crate::theme::muted_text(&app_theme)))
                                .boxed(),
                            Text::new(readout(this.hovered))
                                .text_style(TextStyle::new().font_size(16).color(app_theme.on_background_color))
                                .boxed(),
                        ]
                        .into_iter()
                        .chain(rows),
                    ),
            )
    }
}

/// Starts the cursors showcase on its own.
pub fn start_cursors_example() {
    AimerApp::start(crate::theme::provide(CursorsExample::new().boxed()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
    use aimer::quiver::winit::event::{DeviceId, WindowEvent};

    use super::*;

    /// The window the page is laid out in: wide enough for the grid.
    const WINDOW: (u32, u32) = (800, 640);

    type HeadlessApp<W> = aimer::quiver::aimer_app::HeadlessAimerApp<W>;

    fn hover(app: &mut HeadlessApp<impl Widget + 'static>, x: f64, y: f64) -> CursorIcon {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        });
        app.render_frame();
        app.cursor_icon()
    }

    /// The page in a window of the showcase's size, after its first frame.
    fn page() -> HeadlessApp<impl Widget + 'static> {
        let mut app = AimerApp::start_headless_with(
            crate::theme::provide(CursorsExample::new().boxed()),
            HeadlessOptions { size: PhysicalSize::new(WINDOW.0, WINDOW.1), scale_factor: 1.0 },
        );
        app.pump_frames(3);
        app
    }

    /// Where on the page each icon first showed, found by sweeping the pointer over
    /// the whole grid: the page's own layout is not repeated here.
    fn sweep(app: &mut HeadlessApp<impl Widget + 'static>) -> HashMap<CursorIcon, (f64, f64)> {
        let mut seen = HashMap::new();
        for y in (0..WINDOW.1).step_by(20) {
            for x in (0..WINDOW.0).step_by(20) {
                let icon = hover(app, x as f64 + 1.0, y as f64 + 1.0);
                seen.entry(icon).or_insert((x as f64 + 1.0, y as f64 + 1.0));
            }
        }
        seen
    }

    #[test]
    fn every_tile_changes_the_cursor_to_its_own_icon() {
        let mut app = page();
        let seen = sweep(&mut app);
        for tile in &TILES {
            assert!(seen.contains_key(&tile.icon), "no tile showed {} ({:?}); saw {seen:?}", tile.name, tile.icon);
        }
    }

    #[test]
    fn the_icon_holds_while_the_pointer_moves_inside_a_tile() {
        let mut app = page();
        let seen = sweep(&mut app);
        for tile in &TILES {
            let (x, y) = seen[&tile.icon];
            // Back to the default so the next move enters the tile afresh.
            hover(&mut app, 2.0, 2.0);
            assert_eq!(hover(&mut app, x, y), tile.icon, "entering {}", tile.name);
            assert_eq!(hover(&mut app, x + 3.0, y + 2.0), tile.icon, "a second move inside {}", tile.name);
            assert_eq!(hover(&mut app, x + 6.0, y + 4.0), tile.icon, "a third move inside {}", tile.name);
        }
    }

    #[test]
    fn leaving_every_tile_gives_the_default_cursor_back() {
        let mut app = page();
        let seen = sweep(&mut app);
        let (x, y) = seen[&CursorIcon::Pointer];
        assert_eq!(hover(&mut app, x, y), CursorIcon::Pointer);
        assert_eq!(hover(&mut app, 790.0, 630.0), CursorIcon::Default);
    }

    #[test]
    fn each_tile_asks_for_a_different_icon() {
        let mut icons: Vec<_> = TILES.iter().map(|tile| tile.icon).collect();
        icons.sort_by_key(|icon| *icon as usize);
        icons.dedup();
        assert_eq!(icons.len(), TILES.len());
        assert_eq!(TILES.len() % TILES_PER_ROW, 0, "the grid has no short row");
    }

    #[test]
    fn the_readout_names_the_tile_or_the_default() {
        assert_eq!(readout(Some("Pointer")), "Cursor: Pointer");
        assert_eq!(readout(None), "Cursor: default");
    }
}
