use std::cell::Cell;
use std::rc::Rc;

use aimer::input::button::Button;
use aimer::macros::StatefulWidget;
use aimer::style::*;
use aimer::*;

const SIDEBAR_WIDTH: f32 = 252.0;
/// Space above the menu window's content, which sits in an ordinary titled
/// window.
const MENU_TOP_INSET: u32 = 28;

/// What the music window plays: title, artist, length, and the color of its
/// artwork.
const TRACKS: [(&str, &str, &str, Color); 6] = [
    (
        "How We Got Here",
        "Rocket Hour",
        "3:42",
        Color::Rgb(238, 68, 72),
    ),
    (
        "Side One, Track One",
        "The Creative Hour",
        "4:08",
        Color::Rgb(205, 211, 92),
    ),
    (
        "Shout-Out to the Liars",
        "Five on Fridays",
        "2:57",
        Color::Rgb(226, 92, 42),
    ),
    (
        "The Dog Days Are Over",
        "Time Crisis",
        "3:15",
        Color::Rgb(82, 96, 82),
    ),
    ("Episode 742", "Soulection", "5:31", Color::Rgb(131, 75, 39)),
    (
        "Open Country",
        "Aimer Radio",
        "3:49",
        Color::Rgb(231, 159, 31),
    ),
];

const STATIONS: [(&str, &str, Color); 6] = [
    ("01", "Aimer One", Color::Rgb(238, 68, 72)),
    ("HITS", "Daily Hits", Color::Rgb(52, 110, 235)),
    ("FOLK", "Open Country", Color::Rgb(231, 159, 31)),
    ("UNO", "Música Uno", Color::Rgb(207, 56, 126)),
    ("CLUB", "Aimer Club", Color::Rgb(39, 39, 42)),
    ("CHILL", "Chill", Color::Rgb(65, 145, 238)),
];

/// Starts the window showcase on its own: a small window with one button that
/// launches the music app.
///
/// Closing this window closes every window the music app opened.
pub fn start_window_example() {
    AimerApp::new()
        .window(
            WindowAttr::new()
                .title("Window Showcase")
                .inner_size(460, 300)
                .min_inner_size(320, 200),
        )
        .exit_policy(ExitPolicy::MainWindowClosed)
        .child(crate::theme::provide(WindowShowcase::new()))
        .run();
}

/// The window showcase: one button, **Launch Music**.
///
/// Launching opens the music main menu in a window of its own, and clicking the
/// player bar at the bottom of that window opens the music player in a third.
/// Launching again while the main menu is open brings it to the front rather
/// than opening a second one.
#[derive(StatefulWidget)]
pub struct WindowShowcase {}

impl WindowShowcase {
    /// Creates the window showcase root widget.
    #[inline]
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for WindowShowcase {
    fn default() -> Self {
        Self::new()
    }
}

/// Retained state for [`WindowShowcase`].
pub struct WindowShowcaseState {
    /// The main menu window, once it has been launched.
    menu: Rc<Cell<Option<WindowId>>>,
}

impl StatefulWidget for WindowShowcase {
    type State = WindowShowcaseState;

    fn create_state(self) -> Self::State {
        WindowShowcaseState {
            menu: Rc::new(Cell::new(None)),
        }
    }
}

impl State<WindowShowcase> for WindowShowcaseState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let menu = self.menu.clone();

        Container::new()
            .color(app_theme.background_color)
            .child(
                Column::new()
                    .vertical_alignment(BoxAlignment::Center)
                    .horizontal_alignment(BoxAlignment::Center)
                    .children([pill_button("Launch Music", app_theme, true, move || {
                        launch_music(&menu)
                    })]),
            )
    }
}

/// Opens the music main menu, or brings it forward when it is already open.
fn launch_music(menu: &Cell<Option<WindowId>>) {
    if let Some(window) = menu.get()
        && window.is_open()
    {
        window.focus();
        return;
    }
    menu.set(Some(
        NewWindow::new()
            .attr(
                WindowAttr::new()
                    .title("Aimer Music")
                    .inner_size(1240, 780)
                    .min_inner_size(900, 620),
            )
            .builder(|_ctx| crate::theme::provide(MusicMainMenu::new()))
            .open(),
    ));
}

/// What the player is doing, as both windows show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Playback {
    /// Index into [`TRACKS`].
    track: usize,
    playing: bool,
}

impl Playback {
    fn next(&mut self) {
        self.track = (self.track + 1) % TRACKS.len();
    }

    fn previous(&mut self) {
        self.track = (self.track + TRACKS.len() - 1) % TRACKS.len();
    }

    fn select(&mut self, track: usize) {
        self.track = track.min(TRACKS.len() - 1);
        self.playing = true;
    }

    /// How far into the track the player is, for the progress bar.
    fn progress(&self) -> f32 {
        [0.38, 0.12, 0.67, 0.25, 0.81, 0.5][self.track % 6]
    }
}

/// The music main menu: the radio library, with the player bar at the bottom.
///
/// Clicking the player bar opens the music player in a window of its own. The
/// player is declared with [`Window`] and shown while this page's state says so,
/// and playback lives here, so the two windows are one piece of state seen
/// twice: choosing a track or pressing play in the player changes what the bar
/// shows, and the other way round.
#[derive(StatefulWidget)]
struct MusicMainMenu {}

impl MusicMainMenu {
    #[inline]
    fn new() -> Self {
        Self {}
    }
}

/// Retained state for [`MusicMainMenu`].
struct MusicMenuState {
    player_open: bool,
    playback: Playback,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for MusicMainMenu {
    type State = MusicMenuState;

    fn create_state(self) -> Self::State {
        MusicMenuState {
            player_open: false,
            playback: Playback {
                track: 0,
                playing: true,
            },
            updater: StateUpdater::empty(),
        }
    }
}

impl State<MusicMainMenu> for MusicMenuState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let updater = self.updater;
        let playback = self.playback;

        Container::new()
            .color(app_theme.background_color)
            .child(
                Row::new().children([
                    sidebar(app_theme),
                    Expanded::new()
                        .child(library(app_theme, playback, updater))
                        .boxed(),
                    // Takes no room: it declares the player window, which is on
                    // screen while `player_open` is true. The player is built
                    // again whenever this page is, so it follows `playback`.
                    Window::new()
                        .attr(
                            WindowAttr::new()
                                .title("Now Playing")
                                .inner_size(400, 660)
                                .min_inner_size(340, 520),
                        )
                        .open(self.player_open)
                        .on_close(move || updater.set_state(|state| state.player_open = false))
                        .builder(move |_ctx| {
                            crate::theme::provide(MusicPlayer { playback, updater })
                        })
                        .boxed(),
                ]),
            )
    }
}

fn sidebar(app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .width(Dimension::Px(SIDEBAR_WIDTH))
        .height(Dimension::Percent(100.0))
        .padding(
            LayoutSpacing::new()
                .top(MENU_TOP_INSET)
                .right(16)
                .bottom(18)
                .left(16),
        )
        .color(app_theme.surface_color)
        .child(Column::new().children(vec![
            navigation_item("M", "Search", false, app_theme),
            navigation_item("⌂", "Home", false, app_theme),
            navigation_item("▦", "New", false, app_theme),
            navigation_item("◉", "Radio", true, app_theme),
            section_label("Library", app_theme),
            navigation_item("◷", "Recently Added", false, app_theme),
            navigation_item("♬", "Artists", false, app_theme),
            navigation_item("▣", "Albums", false, app_theme),
            navigation_item("♪", "Songs", false, app_theme),
            section_label("Playlists", app_theme),
            navigation_item("▦", "All Playlists", false, app_theme),
            navigation_item("☆", "Favourite Songs", false, app_theme),
            Expanded::new().child(ZeroSizedBox).boxed(),
            profile(app_theme),
        ]))
        .boxed()
}

fn navigation_item(
    icon: &'static str,
    label: &'static str,
    selected: bool,
    app_theme: ThemeData,
) -> AnyWidget {
    let accent = app_theme.primary_color;
    let background = if selected {
        crate::theme::raised_surface(&app_theme)
    } else {
        Color::Transparent
    };

    Container::new()
        .height(Dimension::Px(42.0))
        .padding(
            LayoutSpacing::new()
                .right(12)
                .left(12),
        )
        .box_decoration(
            BoxDecoration::new()
                .background_color(background)
                .border_radius(10),
        )
        .child(
            Row::new()
                .vertical_alignment(BoxAlignment::Center)
                .gaps(LayoutSpacing::all(Spacing::Px(13)))
                .children([
                    Container::new()
                        .width(Dimension::Px(25.0))
                        .child(
                            Text::new(icon)
                                .text_align(TextAlign::MidCenter)
                                .text_style(
                                    TextStyle::new()
                                        .font_size(22)
                                        .color(accent),
                                ),
                        )
                        .boxed(),
                    Text::new(label)
                        .text_style(
                            TextStyle::new()
                                .font_size(17)
                                .font_weight(if selected {
                                    FontWeight::Bold
                                } else {
                                    FontWeight::Value(500)
                                })
                                .color(app_theme.on_surface_color),
                        )
                        .boxed(),
                ]),
        )
        .boxed()
}

fn section_label(label: &'static str, app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .height(Dimension::Px(34.0))
        .padding(
            LayoutSpacing::new()
                .top(12)
                .left(8),
        )
        .child(
            Text::new(label).text_style(
                TextStyle::new()
                    .font_size(13)
                    .font_weight(FontWeight::Bold)
                    .color(crate::theme::muted_text(&app_theme)),
            ),
        )
        .boxed()
}

fn profile(app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .height(Dimension::Px(48.0))
        .child(
            Row::new()
                .vertical_alignment(BoxAlignment::Center)
                .gaps(LayoutSpacing::all(Spacing::Px(12)))
                .children([
                    Container::new()
                        .width(Dimension::Px(38.0))
                        .height(Dimension::Px(38.0))
                        .box_decoration(
                            BoxDecoration::new()
                                .background_color(app_theme.primary_color)
                                .border_radius(19),
                        )
                        .child(
                            Text::new("AI")
                                .text_align(TextAlign::MidCenter)
                                .text_style(
                                    TextStyle::new()
                                        .font_size(15)
                                        .color(app_theme.on_primary_color),
                                ),
                        )
                        .boxed(),
                    Text::new("Aimer Listener")
                        .text_style(
                            TextStyle::new()
                                .font_size(16)
                                .font_weight(FontWeight::Bold)
                                .color(app_theme.on_surface_color),
                        )
                        .boxed(),
                ]),
        )
        .boxed()
}

fn library(
    app_theme: ThemeData,
    playback: Playback,
    updater: StateUpdater<MusicMenuState>,
) -> AnyWidget {
    Container::new()
        .height(Dimension::Percent(100.0))
        .padding(
            LayoutSpacing::new()
                .top(MENU_TOP_INSET)
                .right(28)
                .bottom(18)
                .left(32),
        )
        .color(app_theme.background_color)
        .child(
            Column::new().children([
                Expanded::new()
                    .child(Scrollable::new().child(library_content(app_theme)))
                    .boxed(),
                SizedBox::new()
                    .height(14)
                    .boxed(),
                player_bar(app_theme, playback, updater),
            ]),
        )
        .boxed()
}

fn library_content(app_theme: ThemeData) -> AnyWidget {
    Column::new()
        .horizontal_alignment(BoxAlignment::Start)
        .children([
            Container::new()
                .height(Dimension::Px(66.0))
                .child(
                    Text::new("Radio")
                        .text_align(TextAlign::TopLeft)
                        .text_style(
                            TextStyle::new()
                                .font_size(38)
                                .font_weight(FontWeight::Bold)
                                .color(app_theme.on_background_color),
                        ),
                )
                .boxed(),
            Container::new()
                .height(Dimension::Px(166.0))
                .child(
                    Scrollable::new()
                        .axis(ScrollAxis::Horizontal)
                        .child(
                            Row::new()
                                .gaps(LayoutSpacing::all(Spacing::Px(18)))
                                .children(
                                    STATIONS
                                        .map(|station| station_card(station, app_theme).boxed()),
                                ),
                        ),
                )
                .boxed(),
            section_title("Latest Radio Episodes  ›", app_theme),
            episode_row(
                (
                    "TIME CRISIS",
                    "The Dog Days Are Over",
                    Color::Rgb(82, 96, 82),
                ),
                (
                    "THE CREATIVE HOUR",
                    "Side One, Track One",
                    Color::Rgb(205, 211, 92),
                ),
                app_theme,
            ),
            SizedBox::new()
                .height(14)
                .boxed(),
            episode_row(
                (
                    "FIVE ON FRIDAYS",
                    "Shout-Out to the Liars",
                    Color::Rgb(226, 92, 42),
                ),
                ("SOULECTION", "Episode 742", Color::Rgb(131, 75, 39)),
                app_theme,
            ),
            SizedBox::new()
                .height(20)
                .boxed(),
        ])
        .boxed()
}

fn station_card(
    (mark, name, accent): (&'static str, &'static str, Color),
    app_theme: ThemeData,
) -> impl Widget {
    Container::new()
        .width(Dimension::Px(132.0))
        .height(Dimension::Px(150.0))
        .child(
            Column::new().children([
                Container::new()
                    .width(Dimension::Px(132.0))
                    .height(Dimension::Px(124.0))
                    .box_decoration(
                        BoxDecoration::new()
                            .background_color(app_theme.surface_color)
                            .border_radius(22),
                    )
                    .child(
                        Text::new(mark)
                            .text_align(TextAlign::MidCenter)
                            .text_style(
                                TextStyle::new()
                                    .font_size(if mark.len() > 2 { 27 } else { 52 })
                                    .font_weight(FontWeight::Bold)
                                    .color(accent),
                            ),
                    )
                    .boxed(),
                Container::new()
                    .height(Dimension::Px(26.0))
                    .child(
                        Text::new(name)
                            .text_align(TextAlign::MidCenter)
                            .text_style(
                                TextStyle::new()
                                    .font_size(12)
                                    .font_weight(FontWeight::Value(500))
                                    .color(crate::theme::muted_text(&app_theme)),
                            ),
                    )
                    .boxed(),
            ]),
        )
}

fn section_title(title: &'static str, app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .height(Dimension::Px(50.0))
        .child(
            Text::new(title)
                .text_align(TextAlign::MidLeft)
                .text_style(
                    TextStyle::new()
                        .font_size(20)
                        .font_weight(FontWeight::Bold)
                        .color(app_theme.on_background_color),
                ),
        )
        .boxed()
}

fn episode_row(
    left: (&'static str, &'static str, Color),
    right: (&'static str, &'static str, Color),
    app_theme: ThemeData,
) -> AnyWidget {
    Row::new()
        .gaps(LayoutSpacing::all(Spacing::Px(24)))
        .children([
            Expanded::new()
                .child(episode(left, app_theme))
                .boxed(),
            Expanded::new()
                .child(episode(right, app_theme))
                .boxed(),
        ])
        .boxed()
}

fn episode(
    (show, title, accent): (&'static str, &'static str, Color),
    app_theme: ThemeData,
) -> AnyWidget {
    Container::new()
        .height(Dimension::Px(116.0))
        .child(
            Row::new()
                .vertical_alignment(BoxAlignment::Center)
                .gaps(LayoutSpacing::all(Spacing::Px(14)))
                .children([
                    Container::new()
                        .width(Dimension::Px(106.0))
                        .height(Dimension::Px(106.0))
                        .box_decoration(
                            BoxDecoration::new()
                                .background_color(accent)
                                .border_radius(8),
                        )
                        .child(
                            Text::new("ON AIR")
                                .text_align(TextAlign::MidCenter)
                                .text_style(
                                    TextStyle::new()
                                        .font_size(15)
                                        .font_weight(FontWeight::Bold)
                                        .color(app_theme.on_primary_color),
                                ),
                        )
                        .boxed(),
                    Expanded::new()
                        .child(
                            Column::new()
                                .vertical_alignment(BoxAlignment::Center)
                                .children([
                                    Text::new(show)
                                        .text_style(
                                            TextStyle::new()
                                                .font_size(12)
                                                .font_weight(FontWeight::Bold)
                                                .color(crate::theme::muted_text(&app_theme)),
                                        )
                                        .boxed(),
                                    SizedBox::new()
                                        .height(5)
                                        .boxed(),
                                    Text::new(title)
                                        .text_style(
                                            TextStyle::new()
                                                .font_size(16)
                                                .font_weight(FontWeight::Value(500))
                                                .color(app_theme.on_background_color),
                                        )
                                        .boxed(),
                                    SizedBox::new()
                                        .height(5)
                                        .boxed(),
                                    Text::new("A new mix for your library.")
                                        .text_style(
                                            TextStyle::new()
                                                .font_size(13)
                                                .color(crate::theme::muted_text(&app_theme)),
                                        )
                                        .boxed(),
                                ]),
                        )
                        .boxed(),
                    Text::new("•••")
                        .text_style(
                            TextStyle::new()
                                .font_size(15)
                                .color(crate::theme::muted_text(&app_theme)),
                        )
                        .boxed(),
                ]),
        )
        .boxed()
}

fn player(app_theme: ThemeData, playback: Playback) -> AnyWidget {
    let (title, artist, _, _) = TRACKS[playback.track];
    Container::new()
        // The bar is a button, which is only as wide as its child, so the bar
        // asks for the whole width of the library it sits in.
        .width(Dimension::Percent(100.0))
        .height(Dimension::Px(78.0))
        .padding(
            LayoutSpacing::new()
                .right(22)
                .left(22),
        )
        .box_decoration(
            BoxDecoration::new()
                .background_color(crate::theme::raised_surface(&app_theme))
                .border_radius(30)
                .border(BoxBorder::all(
                    BorderSlice::new()
                        .style(BorderStyle::Solid)
                        .stroke(Stroke::Px(1.0))
                        .color(crate::theme::divider(&app_theme)),
                )),
        )
        .child(
            Row::new()
                .vertical_alignment(BoxAlignment::Center)
                .gaps(LayoutSpacing::all(Spacing::Px(18)))
                .children([
                    Text::new(if playback.playing {
                        "↶  ◀  ⏸"
                    } else {
                        "↶  ◀  ▶"
                    })
                    .text_style(
                        TextStyle::new()
                            .font_size(19)
                            .font_weight(FontWeight::Bold)
                            .color(app_theme.on_surface_color),
                    )
                    .boxed(),
                    Expanded::new()
                        .child(
                            Column::new()
                                .vertical_alignment(BoxAlignment::Center)
                                .children([
                                    Text::new(title.to_uppercase())
                                        .text_style(
                                            TextStyle::new()
                                                .font_size(13)
                                                .font_weight(FontWeight::Bold)
                                                .color(crate::theme::muted_text(&app_theme)),
                                        )
                                        .boxed(),
                                    Text::new(format!("{artist}  ·  Aimer Radio"))
                                        .text_style(
                                            TextStyle::new()
                                                .font_size(14)
                                                .color(app_theme.on_surface_color),
                                        )
                                        .boxed(),
                                ]),
                        )
                        .boxed(),
                    Text::new("☷   ◉   🔊")
                        .text_style(
                            TextStyle::new()
                                .font_size(18)
                                .color(app_theme.on_surface_color),
                        )
                        .boxed(),
                ]),
        )
        .boxed()
}

/// The player bar as a button: pressing it opens the music player window.
fn player_bar(
    app_theme: ThemeData,
    playback: Playback,
    updater: StateUpdater<MusicMenuState>,
) -> AnyWidget {
    Button::new()
        .on_press(move || updater.set_state(|state| state.player_open = true))
        .decoration(
            BoxDecoration::new()
                .background_color(Color::Transparent)
                .border_radius(30),
        )
        .hover_decoration(
            BoxDecoration::new()
                .background_color(app_theme.primary_color.with_alpha(0.10))
                .border_radius(30),
        )
        .child(player(app_theme, playback))
        .boxed()
}

/// A rounded text button: filled with the accent color when `primary`.
fn pill_button(
    label: &'static str,
    app_theme: ThemeData,
    primary: bool,
    on_press: impl Fn() + 'static,
) -> AnyWidget {
    let background = if primary {
        app_theme.primary_color
    } else {
        crate::theme::raised_surface(&app_theme)
    };
    let foreground = if primary {
        app_theme.on_primary_color
    } else {
        app_theme.on_surface_color
    };

    // The button is as big as its child, so the pill's height and its room
    // either side of the label belong to the child.
    Button::new()
        .on_press(on_press)
        .decoration(
            BoxDecoration::new()
                .background_color(background)
                .border_radius(18),
        )
        .hover_decoration(
            BoxDecoration::new()
                .background_color(background.lighten(0.08))
                .border_radius(18),
        )
        .press_decoration(
            BoxDecoration::new()
                .background_color(background.darken(0.08))
                .border_radius(18),
        )
        .child(
            Container::new()
                .height(Dimension::Px(36.0))
                .padding(
                    LayoutSpacing::new()
                        .left(18)
                        .right(18),
                )
                .child(
                    Text::new(label)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(14)
                                .font_weight(FontWeight::Bold)
                                .color(foreground),
                        ),
                ),
        )
        .notype()
}

/// The contents of the music window: what is playing, the transport controls,
/// and the queue.
///
/// Every control changes the radio window's state, which is where the playback
/// lives, so the radio window's player bar follows the player.
#[derive(Clone, StatelessWidget)]
struct MusicPlayer {
    playback: Playback,
    updater: StateUpdater<MusicMenuState>,
}

impl StatelessWidget for MusicPlayer {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let playback = self.playback;
        let updater = self.updater;
        let (title, artist, length, accent) = TRACKS[playback.track];

        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(26))
            .child(
                Column::new().children([
                    Container::new()
                        .height(Dimension::Px(250.0))
                        .box_decoration(
                            BoxDecoration::new()
                                .background_color(accent)
                                .border_radius(28),
                        )
                        .child(
                            Text::new("♪")
                                .text_align(TextAlign::MidCenter)
                                .text_style(
                                    TextStyle::new()
                                        .font_size(96)
                                        .font_weight(FontWeight::Bold)
                                        .color(app_theme.on_primary_color),
                                ),
                        )
                        .boxed(),
                    SizedBox::new()
                        .height(22)
                        .boxed(),
                    Text::new(title)
                        .text_style(
                            TextStyle::new()
                                .font_size(24)
                                .font_weight(FontWeight::Bold)
                                .color(app_theme.on_background_color),
                        )
                        .boxed(),
                    SizedBox::new()
                        .height(4)
                        .boxed(),
                    Text::new(artist)
                        .text_style(
                            TextStyle::new()
                                .font_size(15)
                                .color(crate::theme::muted_text(&app_theme)),
                        )
                        .boxed(),
                    SizedBox::new()
                        .height(18)
                        .boxed(),
                    progress_bar(app_theme, playback.progress(), length),
                    SizedBox::new()
                        .height(14)
                        .boxed(),
                    transport(app_theme, playback, updater),
                    SizedBox::new()
                        .height(20)
                        .boxed(),
                    Text::new("Up Next")
                        .text_style(
                            TextStyle::new()
                                .font_size(13)
                                .font_weight(FontWeight::Bold)
                                .color(crate::theme::muted_text(&app_theme)),
                        )
                        .boxed(),
                    SizedBox::new()
                        .height(6)
                        .boxed(),
                    Expanded::new()
                        .child(Scrollable::new().child(queue(app_theme, playback, updater)))
                        .boxed(),
                ]),
            )
    }
}

fn progress_bar(app_theme: ThemeData, fraction: f32, length: &'static str) -> AnyWidget {
    Column::new()
        .children([
            Container::new()
                .height(Dimension::Px(5.0))
                .box_decoration(
                    BoxDecoration::new()
                        .background_color(crate::theme::recessed_surface(&app_theme))
                        .border_radius(3),
                )
                .child(
                    Container::new()
                        .width(Dimension::Percent(fraction * 100.0))
                        .height(Dimension::Px(5.0))
                        .box_decoration(
                            BoxDecoration::new()
                                .background_color(app_theme.primary_color)
                                .border_radius(3),
                        )
                        .child(ZeroSizedBox),
                )
                .boxed(),
            SizedBox::new()
                .height(6)
                .boxed(),
            Row::new()
                .children([
                    Expanded::new()
                        .child(
                            Text::new("0:00").text_style(
                                TextStyle::new()
                                    .font_size(12)
                                    .color(crate::theme::muted_text(&app_theme)),
                            ),
                        )
                        .boxed(),
                    Text::new(length)
                        .text_style(
                            TextStyle::new()
                                .font_size(12)
                                .color(crate::theme::muted_text(&app_theme)),
                        )
                        .boxed(),
                ])
                .boxed(),
        ])
        .boxed()
}

fn transport(
    app_theme: ThemeData,
    playback: Playback,
    updater: StateUpdater<MusicMenuState>,
) -> AnyWidget {
    Row::new()
        .gaps(LayoutSpacing::all(Spacing::Px(18)))
        .horizontal_alignment(BoxAlignment::Center)
        .children([
            round_button("◀◀", 48.0, false, app_theme, move || {
                updater.set_state(|state| {
                    state
                        .playback
                        .previous()
                })
            }),
            round_button(
                if playback.playing { "⏸" } else { "▶" },
                64.0,
                true,
                app_theme,
                move || {
                    updater.set_state(|state| {
                        state
                            .playback
                            .playing = !state
                            .playback
                            .playing
                    })
                },
            ),
            round_button("▶▶", 48.0, false, app_theme, move || {
                updater.set_state(|state| {
                    state
                        .playback
                        .next()
                })
            }),
        ])
        .boxed()
}

fn round_button(
    glyph: &'static str,
    size: f32,
    primary: bool,
    app_theme: ThemeData,
    on_press: impl Fn() + 'static,
) -> AnyWidget {
    let background = if primary {
        app_theme.primary_color
    } else {
        crate::theme::raised_surface(&app_theme)
    };
    let foreground = if primary {
        app_theme.on_primary_color
    } else {
        app_theme.on_surface_color
    };
    let radius = (size / 2.0) as i32;

    Button::new()
        .on_press(on_press)
        .decoration(
            BoxDecoration::new()
                .background_color(background)
                .border_radius(radius),
        )
        .hover_decoration(
            BoxDecoration::new()
                .background_color(background.lighten(0.08))
                .border_radius(radius),
        )
        .press_decoration(
            BoxDecoration::new()
                .background_color(background.darken(0.08))
                .border_radius(radius),
        )
        .child(
            Container::new()
                .width(Dimension::Px(size))
                .height(Dimension::Px(size))
                .child(
                    Text::new(glyph)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(if primary { 24 } else { 17 })
                                .font_weight(FontWeight::Bold)
                                .color(foreground),
                        ),
                ),
        )
        .boxed()
}

fn queue(
    app_theme: ThemeData,
    playback: Playback,
    updater: StateUpdater<MusicMenuState>,
) -> AnyWidget {
    Column::new()
        .children(
            TRACKS
                .iter()
                .enumerate()
                .map(|(index, track)| {
                    queue_row(index, *track, index == playback.track, app_theme, updater)
                })
                .collect::<Vec<_>>(),
        )
        .boxed()
}

fn queue_row(
    index: usize,
    (title, artist, length, accent): (&'static str, &'static str, &'static str, Color),
    selected: bool,
    app_theme: ThemeData,
    updater: StateUpdater<MusicMenuState>,
) -> AnyWidget {
    let background = if selected {
        crate::theme::raised_surface(&app_theme)
    } else {
        Color::Transparent
    };

    Button::new()
        .on_press(move || {
            updater.set_state(move |state| {
                state
                    .playback
                    .select(index)
            })
        })
        .decoration(
            BoxDecoration::new()
                .background_color(background)
                .border_radius(12),
        )
        .hover_decoration(
            BoxDecoration::new()
                .background_color(crate::theme::raised_surface(&app_theme))
                .border_radius(12),
        )
        .child(
            Container::new()
                .height(Dimension::Px(56.0))
                .padding(
                    LayoutSpacing::new()
                        .left(10)
                        .right(12),
                )
                .child(
                    Row::new()
                        .vertical_alignment(BoxAlignment::Center)
                        .gaps(LayoutSpacing::all(Spacing::Px(12)))
                        .children([
                            Container::new()
                                .width(Dimension::Px(38.0))
                                .height(Dimension::Px(38.0))
                                .box_decoration(
                                    BoxDecoration::new()
                                        .background_color(accent)
                                        .border_radius(8),
                                )
                                .child(ZeroSizedBox)
                                .boxed(),
                            Expanded::new()
                                .child(
                                    Column::new()
                                        .vertical_alignment(BoxAlignment::Center)
                                        .children([
                                            Text::new(title)
                                                .text_style(
                                                    TextStyle::new()
                                                        .font_size(14)
                                                        .font_weight(if selected {
                                                            FontWeight::Bold
                                                        } else {
                                                            FontWeight::Value(500)
                                                        })
                                                        .color(app_theme.on_background_color),
                                                )
                                                .boxed(),
                                            Text::new(artist)
                                                .text_style(
                                                    TextStyle::new()
                                                        .font_size(12)
                                                        .color(crate::theme::muted_text(
                                                            &app_theme,
                                                        )),
                                                )
                                                .boxed(),
                                        ]),
                                )
                                .boxed(),
                            Text::new(length)
                                .text_style(
                                    TextStyle::new()
                                        .font_size(12)
                                        .color(crate::theme::muted_text(&app_theme)),
                                )
                                .boxed(),
                        ]),
                ),
        )
        .boxed()
}

#[cfg(test)]
mod tests {
    use aimer::quiver::windows::host::{OpenRequest, Request, WindowCommand, drain, mark_closed, mark_open};
    use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
    use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    use aimer::{AimerApp, HeadlessAimerApp, HeadlessOptions, Widget, WindowAttr, WindowId};

    use super::{MusicMainMenu, Playback, TRACKS, WindowShowcase};

    #[test]
    fn window_showcase_is_constructible_as_a_widget() {
        fn assert_widget(_: impl Widget) {}

        assert_widget(WindowShowcase::new());
    }

    #[test]
    fn next_and_previous_wrap_around_the_queue() {
        let mut playback = Playback {
            track: TRACKS.len() - 1,
            playing: false,
        };

        playback.next();
        assert_eq!(playback.track, 0);
        playback.previous();
        assert_eq!(playback.track, TRACKS.len() - 1);
    }

    #[test]
    fn choosing_a_track_plays_it_and_stays_inside_the_queue() {
        let mut playback = Playback {
            track: 0,
            playing: false,
        };

        playback.select(3);
        assert_eq!(playback, Playback { track: 3, playing: true });

        playback.select(usize::MAX);
        assert_eq!(playback.track, TRACKS.len() - 1);
    }

    #[test]
    fn every_track_has_a_progress_inside_the_bar() {
        for track in 0..TRACKS.len() {
            let progress = Playback { track, playing: true }.progress();

            assert!((0.0..=1.0).contains(&progress), "track {track}: {progress}");
        }
    }

    /// The windows the page has asked for since the queue was last read.
    fn opened() -> Vec<OpenRequest> {
        drain()
            .into_iter()
            .filter_map(|request| match request {
                Request::Open(open) => Some(open),
                Request::Command(..) => None,
            })
            .collect()
    }

    fn commands() -> Vec<(WindowId, WindowCommand)> {
        drain()
            .into_iter()
            .filter_map(|request| match request {
                Request::Command(id, command) => Some((id, command)),
                Request::Open(_) => None,
            })
            .collect()
    }

    /// A headless window of the size the music app opens at.
    fn window(root: impl Widget + 'static) -> HeadlessAimerApp<impl Widget + 'static> {
        drain();
        let mut app = AimerApp::start_headless_with(
            crate::theme::provide(root),
            HeadlessOptions {
                size: PhysicalSize::new(1240, 780),
                scale_factor: 1.0,
            },
        );
        app.pump_frames(8);
        app
    }

    /// Presses and releases the primary button at `(x, y)`.
    fn click(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        });
        app.pump_frames(2);
        for state in [ElementState::Pressed, ElementState::Released] {
            app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: MouseButton::Left,
            });
            app.pump_frames(2);
        }
        app.pump_frames(8);
    }

    /// The Launch Music button: the only thing on the page, in its middle.
    const LAUNCH: (f64, f64) = (620.0, 390.0);

    /// The player bar: the pill along the bottom of the main menu.
    const PLAYER_BAR: (f64, f64) = (748.0, 723.0);

    fn menu_attr() -> WindowAttr {
        WindowAttr::new()
            .title("Aimer Music")
            .inner_size(1240, 780)
            .min_inner_size(900, 620)
    }

    #[test]
    fn nothing_is_opened_until_music_is_launched() {
        let _app = window(WindowShowcase::new());

        assert!(drain().is_empty());
    }

    #[test]
    fn launch_music_opens_the_main_menu() {
        let mut app = window(WindowShowcase::new());

        click(&mut app, LAUNCH.0, LAUNCH.1);

        let requests = opened();
        assert_eq!(requests.len(), 1, "Launch Music did not open a window");
        assert_eq!(requests[0].attr, menu_attr());
    }

    #[test]
    fn launching_again_brings_the_open_menu_forward() {
        let mut app = window(WindowShowcase::new());
        click(&mut app, LAUNCH.0, LAUNCH.1);
        let menu = opened().remove(0).id;
        // The event loop has created it.
        mark_open(menu);

        click(&mut app, LAUNCH.0, LAUNCH.1);

        let requests = drain();
        assert!(
            requests.iter().all(|request| matches!(request, Request::Command(..))),
            "a second main menu was opened"
        );
        let commands: Vec<_> = requests
            .into_iter()
            .filter_map(|request| match request {
                Request::Command(id, command) => Some((id, command)),
                Request::Open(_) => None,
            })
            .collect();
        assert_eq!(commands, [(menu, WindowCommand::Focus)]);
    }

    #[test]
    fn launching_after_the_menu_was_closed_opens_a_new_one() {
        let mut app = window(WindowShowcase::new());
        click(&mut app, LAUNCH.0, LAUNCH.1);
        let menu = opened().remove(0).id;
        mark_open(menu);
        mark_closed(menu);

        click(&mut app, LAUNCH.0, LAUNCH.1);

        assert_eq!(opened().len(), 1);
    }

    #[test]
    fn the_main_menu_does_not_open_the_player_by_itself() {
        let _app = window(MusicMainMenu::new());

        assert!(drain().is_empty());
    }

    #[test]
    fn clicking_the_player_bar_opens_the_player() {
        let mut app = window(MusicMainMenu::new());

        click(&mut app, PLAYER_BAR.0, PLAYER_BAR.1);

        let requests = opened();
        assert_eq!(requests.len(), 1, "the player bar did not open the player");
        assert_eq!(
            requests[0].attr,
            WindowAttr::new()
                .title("Now Playing")
                .inner_size(400, 660)
                .min_inner_size(340, 520)
        );
    }

    #[test]
    fn the_player_stays_one_window_when_the_bar_is_clicked_again() {
        let mut app = window(MusicMainMenu::new());
        click(&mut app, PLAYER_BAR.0, PLAYER_BAR.1);
        assert_eq!(opened().len(), 1);

        click(&mut app, PLAYER_BAR.0, PLAYER_BAR.1);

        assert!(opened().is_empty(), "a second player was opened");
        assert!(commands().is_empty(), "the open player was closed");
    }

    #[test]
    fn closing_the_player_lets_the_bar_open_it_again() {
        let mut app = window(MusicMainMenu::new());
        click(&mut app, PLAYER_BAR.0, PLAYER_BAR.1);
        let player = opened().remove(0);

        // The user closes the player window; the event loop reports it.
        (player.on_closed.expect("the event loop is told when it closes"))();
        app.pump_frames(8);
        click(&mut app, PLAYER_BAR.0, PLAYER_BAR.1);

        assert_eq!(opened().len(), 1, "the player did not open again");
    }
}
