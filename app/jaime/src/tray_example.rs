//! A system tray icon whose events arrive in the event loop.
//!
//! The page has two buttons: one puts an icon in the menu bar (macOS), the
//! notification area (Windows) or the status notifier host (Linux), the other
//! takes it away. The icon's menu shows each thing the tray can change at
//! runtime:
//!
//! - **Notifications** is a check item; choosing it ticks or unticks it, which
//!   rebuilds the menu.
//! - **Next icon color** swaps the picture.
//! - Every click on the icon bumps a counter shown in the tooltip, and
//!   **Reset the click count** puts it back.
//! - **Remove the tray icon** drops the tray from inside its own handler.
//!
//! The tray reports from a thread of its own on Windows and Linux, but
//! [`aimer::tray::install`] hands every event to the event loop first, so the
//! handler below runs on the UI thread and writes to the log and the page like
//! any other callback.
//!
//! The decisions live in [`update`], a plain function from the settings and an
//! event to what has to change, so they are tested without a tray.

use std::cell::{Cell, RefCell};

use aimer::input::button::Button;
use aimer::style::*;
use aimer::svg::SvgColor;
use aimer::tray::{self, AppTray, Tray, TrayButton, TrayEvent, TrayIcon, TrayItemId, TrayMenu};
use aimer::*;

/// How many log lines the page keeps.
const LOG_LINES: usize = 8;

const SAY_HELLO: TrayItemId = TrayItemId(1);
const NOTIFICATIONS: TrayItemId = TrayItemId(2);
const NEXT_COLOR: TrayItemId = TrayItemId(3);
const RESET_CLICKS: TrayItemId = TrayItemId(4);
const ABOUT: TrayItemId = TrayItemId(5);
const REMOVE: TrayItemId = TrayItemId(6);

/// The colors the icon cycles through.
const PALETTE: [[u8; 3]; 4] = [[40, 120, 255], [235, 90, 90], [60, 190, 120], [240, 180, 40]];

/// What the menu and the icon are built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Settings {
    notifications: bool,
    /// An index into [`PALETTE`].
    color: usize,
    clicks: u32,
}

const INITIAL: Settings = Settings { notifications: true, color: 0, clicks: 0 };

/// What an event asks the tray and the page to do.
#[derive(Default, Debug, PartialEq, Eq)]
struct Outcome {
    log: Option<String>,
    icon: bool,
    tooltip: bool,
    menu: bool,
    remove: bool,
}

aimer_tls::thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static STATUS: RefCell<String> = const { RefCell::new(String::new()) };
    static SETTINGS: Cell<Settings> = const { Cell::new(INITIAL) };
    /// The tray on screen, if there is one. Dropping it removes the icon.
    static TRAY: RefCell<Option<AppTray>> = const { RefCell::new(None) };
    /// The page that is showing the log, if one is.
    static PAGE: Cell<Option<StateUpdater<TrayExampleState>>> = const { Cell::new(None) };
}

/// Appends `entry` to the log and redraws the page showing it.
fn record(entry: &str) {
    LOG.with_borrow_mut(|log| {
        if log.len() == LOG_LINES {
            log.remove(0);
        }
        log.push(entry.to_owned());
    });
    refresh_page();
}

/// Redraws the page, if one is showing. A page that has gone away rejects the
/// update, which is the right outcome.
fn refresh_page() {
    if let Some(page) = PAGE.get() {
        let _ = page.try_set_state(|_| {});
    }
}

fn set_status(status: impl Into<String>) {
    STATUS.with_borrow_mut(|slot| *slot = status.into());
    refresh_page();
}

/// Decides what `event` changes. Pure: it only edits `settings`.
fn update(settings: &mut Settings, event: TrayEvent) -> Outcome {
    match event {
        TrayEvent::Click(button) => {
            settings.clicks = settings.clicks.saturating_add(1);
            Outcome { log: Some(format!("click: {button:?}")), tooltip: true, ..Outcome::default() }
        }
        TrayEvent::DoubleClick(button) => {
            Outcome { log: Some(format!("double click: {button:?}")), ..Outcome::default() }
        }
        TrayEvent::MenuItem(SAY_HELLO) => {
            Outcome { log: Some("menu: Say hello".into()), ..Outcome::default() }
        }
        TrayEvent::MenuItem(NOTIFICATIONS) => {
            settings.notifications = !settings.notifications;
            let state = if settings.notifications { "on" } else { "off" };
            Outcome { log: Some(format!("menu: Notifications {state}")), menu: true, ..Outcome::default() }
        }
        TrayEvent::MenuItem(NEXT_COLOR) => {
            settings.color = (settings.color + 1) % PALETTE.len();
            Outcome { log: Some("menu: Next icon color".into()), icon: true, ..Outcome::default() }
        }
        TrayEvent::MenuItem(RESET_CLICKS) => {
            settings.clicks = 0;
            Outcome { log: Some("menu: Reset the click count".into()), tooltip: true, ..Outcome::default() }
        }
        TrayEvent::MenuItem(REMOVE) => {
            Outcome { log: Some("menu: Remove the tray icon".into()), remove: true, ..Outcome::default() }
        }
        TrayEvent::MenuItem(_) => Outcome::default(),
    }
}

/// The Lucide bell, 64 pixels square, in the palette color `color`.
fn icon(color: usize) -> TrayIcon {
    let [r, g, b] = PALETTE[color % PALETTE.len()];
    aimer::lucide_tray_icon(Icons::BELL, 64, SvgColor::rgba8(r, g, b, 255))
        .expect("a rasterized icon is a valid RGBA buffer")
}

fn tooltip(clicks: u32) -> String {
    format!("Aimer tray example: {clicks} clicks")
}

fn menu(settings: &Settings) -> TrayMenu {
    TrayMenu::new()
        .item(SAY_HELLO, "Say hello")
        .check_item(NOTIFICATIONS, "Notifications", settings.notifications)
        .item(NEXT_COLOR, "Next icon color")
        .item(RESET_CLICKS, "Reset the click count")
        .separator()
        .submenu("More", TrayMenu::new().disabled_item(ABOUT, "Aimer tray example"))
        .separator()
        .item(REMOVE, "Remove the tray icon")
}

/// Runs on the UI thread for every click and chosen item of the tray.
fn on_tray_event(event: TrayEvent) {
    let mut settings = SETTINGS.get();
    let outcome = update(&mut settings, event);
    SETTINGS.set(settings);
    if let Some(entry) = &outcome.log {
        record(entry);
    }
    if outcome.remove {
        hide_tray();
        return;
    }
    TRAY.with_borrow_mut(|slot| {
        let Some(tray) = slot else { return };
        if outcome.icon {
            tray.set_icon(&icon(settings.color));
        }
        if outcome.tooltip {
            tray.set_tooltip(&tooltip(settings.clicks));
        }
        if outcome.menu {
            tray.set_menu(Some(&menu(&settings)));
        }
    });
}

/// Puts the icon on screen, unless it already is.
fn show_tray() {
    if TRAY.with_borrow(Option::is_some) {
        return;
    }
    SETTINGS.set(INITIAL);
    let tray = Tray::new().icon(icon(INITIAL.color)).tooltip(tooltip(INITIAL.clicks)).menu(menu(&INITIAL));
    match tray::install(tray, on_tray_event) {
        Ok(installed) => {
            TRAY.with_borrow_mut(|slot| *slot = Some(installed));
            set_status("The tray icon is on screen.");
        }
        Err(why) => set_status(format!("Unavailable: {why}")),
    }
}

/// Takes the icon away. Safe inside a tray handler: the loop released its
/// hold on the handler before running it.
fn hide_tray() {
    let removed = TRAY.with_borrow_mut(Option::take);
    if removed.is_some() {
        set_status("The tray icon was removed.");
    }
    drop(removed);
}

/// The tray page.
#[derive(StatefulWidget)]
pub struct TrayExample;

impl TrayExample {
    /// Creates the page.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Default for TrayExample {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// The retained side of the page. The log and the tray outlive the page, so
/// all it keeps is a way to ask for a redraw.
pub struct TrayExampleState {
    /// Kept for the helper methods that call back into the state.
    updater: StateUpdater<Self>,
}

impl TrayExampleState {
    /// A line of text in the page's body style.
    fn line(text: impl Into<String>, color: Color, size: u32) -> AnyWidget {
        Text::new(text.into()).text_style(TextStyle::new().font_size(size).color(color)).boxed()
    }

    /// A heading above one group of the page.
    fn heading(text: &'static str, app_theme: &ThemeData) -> AnyWidget {
        Text::new(text)
            .text_style(
                TextStyle::new().font_size(18).font_weight(FontWeight::Bold).color(app_theme.on_background_color),
            )
            .boxed()
    }

    fn button(label: &'static str, app_theme: &ThemeData, on_press: fn()) -> AnyWidget {
        Container::new()
            .width(Dimension::Px(180.0))
            .height(Dimension::Px(40.0))
            .box_child(
                Button::new()
                    .on_press(on_press)
                    .decoration(
                        BoxDecoration::new().background_color(app_theme.primary_color).border_radius((8, 8, 8, 8)),
                    )
                    .child(
                        Text::new(label)
                            .text_align(TextAlign::MidCenter)
                            .text_style(TextStyle::new().font_size(14).color(app_theme.on_primary_color)),
                    ),
            )
            .boxed()
    }
}

impl State<TrayExample> for TrayExampleState {
    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn create(widget: TrayExample) -> Self {
        TrayExampleState { updater: StateUpdater::empty() }
    }


    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        PAGE.set(Some(this.updater()));
        let app_theme = ThemeData::copied(ctx);
        let muted = crate::theme::muted_text(&app_theme);
        let body = app_theme.on_background_color;

        let log: Vec<AnyWidget> = LOG.with_borrow(|log| {
            if log.is_empty() {
                vec![Self::line("Nothing yet. Click the icon or choose a menu item.", muted, 14)]
            } else {
                log.iter().rev().map(|entry| Self::line(entry.as_str(), body, 14)).collect()
            }
        });
        let status = STATUS.with_borrow(|status| {
            if status.is_empty() { "The tray icon is not shown.".to_owned() } else { status.clone() }
        });
        let settings = SETTINGS.get();

        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Start)
                    .gaps(LayoutSpacing::new().bottom(12))
                    .children(vec![
                        Self::heading("Tray icon", &app_theme),
                        Self::line(status, body, 15),
                        Row::new()
                            .gaps(LayoutSpacing::new().right(16))
                            .children(vec![
                                Self::button("Show the tray icon", &app_theme, show_tray),
                                Self::button("Remove the tray icon", &app_theme, hide_tray),
                            ])
                            .boxed(),
                        Self::line(
                            format!(
                                "Clicks: {}   Notifications: {}   Color: {}",
                                settings.clicks,
                                if settings.notifications { "on" } else { "off" },
                                settings.color + 1,
                            ),
                            body,
                            15,
                        ),
                        Self::line(
                            "Click the icon, or open its menu: Say hello, Notifications, Next icon color, \
                             Reset the click count, More, Remove.",
                            muted,
                            14,
                        ),
                        Self::heading("Log", &app_theme),
                        Column::new()
                            .horizontal_alignment(BoxAlignment::Start)
                            .gaps(LayoutSpacing::new().bottom(4))
                            .children(log)
                            .boxed(),
                        Self::line(
                            "macOS shows it in the menu bar and opens the menu on any click, so only chosen items are \
                             reported there. Windows uses the notification area. Linux needs a StatusNotifier host. \
                             Mobile and web have no tray.",
                            muted,
                            14,
                        ),
                    ]),
            )
    }
}

#[cfg(test)]
mod tests {
    use aimer::quiver::aimer_app::{AimerApp, AimerNativePlatformEvent};

    use super::*;

    fn run(settings: &mut Settings, event: TrayEvent) -> Outcome {
        update(settings, event)
    }

    #[test]
    fn a_click_is_counted_and_refreshes_the_tooltip() {
        let mut settings = INITIAL;
        let outcome = run(&mut settings, TrayEvent::Click(TrayButton::Left));
        assert_eq!(settings.clicks, 1);
        assert!(outcome.tooltip && !outcome.icon && !outcome.menu);
        assert_eq!(outcome.log.as_deref(), Some("click: Left"));
    }

    #[test]
    fn the_click_count_does_not_overflow() {
        let mut settings = Settings { clicks: u32::MAX, ..INITIAL };
        run(&mut settings, TrayEvent::Click(TrayButton::Right));
        assert_eq!(settings.clicks, u32::MAX);
    }

    #[test]
    fn notifications_toggle_and_rebuild_the_menu() {
        let mut settings = INITIAL;
        let outcome = run(&mut settings, TrayEvent::MenuItem(NOTIFICATIONS));
        assert!(!settings.notifications && outcome.menu);
        assert_eq!(menu(&settings).find(NOTIFICATIONS).unwrap().checked, Some(false));

        run(&mut settings, TrayEvent::MenuItem(NOTIFICATIONS));
        assert_eq!(menu(&settings).find(NOTIFICATIONS).unwrap().checked, Some(true));
    }

    #[test]
    fn the_color_cycles_through_the_palette_and_wraps() {
        let mut settings = INITIAL;
        for step in 1..=PALETTE.len() {
            let outcome = run(&mut settings, TrayEvent::MenuItem(NEXT_COLOR));
            assert!(outcome.icon);
            assert_eq!(settings.color, step % PALETTE.len());
        }
        assert_eq!(settings.color, 0, "a full cycle comes back to the start");
    }

    #[test]
    fn reset_zeroes_the_count_and_remove_asks_to_leave() {
        let mut settings = Settings { clicks: 5, ..INITIAL };
        assert!(run(&mut settings, TrayEvent::MenuItem(RESET_CLICKS)).tooltip);
        assert_eq!(settings.clicks, 0);
        assert!(run(&mut settings, TrayEvent::MenuItem(REMOVE)).remove);
    }

    #[test]
    fn unknown_items_and_the_disabled_one_change_nothing() {
        let mut settings = INITIAL;
        assert_eq!(run(&mut settings, TrayEvent::MenuItem(TrayItemId(99))), Outcome::default());
        assert_eq!(settings, INITIAL);
    }

    #[test]
    fn every_menu_item_is_one_the_handler_knows() {
        let mut seen = Vec::new();
        menu(&INITIAL).for_each_item(&mut |item| seen.push(item.id));
        assert_eq!(seen, [SAY_HELLO, NOTIFICATIONS, NEXT_COLOR, RESET_CLICKS, ABOUT, REMOVE]);
        // The disabled item can never be chosen, so it needs no behaviour.
        assert!(!menu(&INITIAL).find(ABOUT).unwrap().enabled);
    }

    #[test]
    fn icons_are_the_lucide_bell_in_the_palette_color() {
        for color in 0..PALETTE.len() {
            let icon = icon(color);
            assert_eq!((icon.width(), icon.height()), (64, 64));
            assert!(icon.rgba().chunks(4).all(|px| px[..3] == PALETTE[color]), "every pixel is the palette color");
            assert!(icon.rgba().chunks(4).any(|px| px[3] == 255), "the bell has solid ink");
            assert_eq!(icon.rgba()[3], 0, "the corner is transparent");
        }
    }

    #[test]
    fn a_tray_event_in_the_loop_reaches_the_page_log() {
        LOG.with_borrow_mut(Vec::clear);
        SETTINGS.set(INITIAL);
        let subscription = tray::subscribe(on_tray_event);
        let mut app = AimerApp::start_headless(crate::theme::provide(TrayExample::new().boxed()));
        app.render_frame();

        app.send_user_event(AimerNativePlatformEvent::Tray {
            id: subscription.id(),
            event: TrayEvent::Click(TrayButton::Left),
        });
        app.send_user_event(AimerNativePlatformEvent::Tray {
            id: subscription.id(),
            event: TrayEvent::MenuItem(SAY_HELLO),
        });

        assert_eq!(LOG.with_borrow(Clone::clone), ["click: Left", "menu: Say hello"]);
        assert_eq!(SETTINGS.get().clicks, 1);
    }
}
