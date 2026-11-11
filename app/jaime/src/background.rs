//! Keeps Jaime alive after its last window closes.
//!
//! The app runs with [`ExitPolicy::Never`](aimer::ExitPolicy::Never), so closing
//! the window no longer ends the event loop. What replaces the window as the
//! app's presence is a tray icon with two items:
//!
//! - **Open window** brings the showcase window forward, or opens one when
//!   none exists, which is how the user gets back after closing it. A click on
//!   the Dock icon does the same when no window is showing.
//! - **Quit** asks the loop to end through `windows::quit`.
//!
//! There is one showcase window at a time: asking for it while it is open
//! focuses it, and un-minimizes it, instead of opening a second. It remembers
//! its position, size and maximized state under [`WINDOW_NAME`], so the one
//! that replaces a closed window, and the one of the next run, appear where the
//! user left it.
//!
//! Whichever way the loop ends, [`Lifecycle::Exit`] stops the daemon started on
//! the Daemon page, so its quit hooks run and its scheduler thread ends instead
//! of being cut off with the process.
//!
//! [`install`] runs in the app's `setup` hook, on the main thread and after the
//! native application exists. The returned [`Background`] is kept by the app
//! until shutdown; dropping it removes the icon.

use aimer::reopen::{self, ReopenSubscription};
use aimer::tray::{self, AppTray, Tray, TrayButton, TrayEvent, TrayItemId, TrayMenu};
use aimer::style::AnimatedTheme;
use aimer::venus::{Lifecycle, LifecycleSubscription, on_lifecycle};
use aimer::*;
use std::cell::Cell;

use crate::test_animation::is_mobile;

/// The name the showcase window's position and size are remembered under.
pub const WINDOW_NAME: &str = "main";

const OPEN: TrayItemId = TrayItemId(1);
const QUIT: TrayItemId = TrayItemId(2);

/// What a tray event asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    OpenWindow,
    Quit,
    Nothing,
}

/// Maps a tray event to what it asks for. Pure, so it is tested without a tray.
fn action(event: TrayEvent) -> Action {
    match event {
        TrayEvent::MenuItem(OPEN) | TrayEvent::DoubleClick(TrayButton::Left) => Action::OpenWindow,
        TrayEvent::MenuItem(QUIT) => Action::Quit,
        _ => Action::Nothing,
    }
}

thread_local! {
    /// The window this module opened last, which may since have closed.
    static OPENED: Cell<Option<WindowId>> = const { Cell::new(None) };
}

/// The first of `known` that is open. Pure, so it is tested without windows.
fn open_among(known: &[WindowId], is_open: impl Fn(WindowId) -> bool) -> Option<WindowId> {
    known.iter().copied().find(|id| is_open(*id))
}

/// Shows the showcase window: the one that is open, brought forward, or a new
/// one when it has been closed.
fn show_window() {
    let known = [WindowId::MAIN, OPENED.get().unwrap_or(WindowId::MAIN)];
    if let Some(id) = open_among(&known, WindowId::is_open) {
        id.set_minimized(false);
        id.focus();
        return;
    }
    let id = NewWindow::new()
        .attr(WindowAttr::new().title("Jaime").inner_size(1280, 800).remember(WINDOW_NAME))
        .builder(|_ctx| AnimatedTheme::new().data(crate::theme::app_theme()).child(crate::showcase::ExampleShowcase::new()))
        .open();
    OPENED.set(Some(id));
}

/// A click on the Dock icon. With a window on screen it has nothing to add; a
/// window that is open but hidden or minimized is not on screen, so it is
/// brought forward rather than a second one opened.
fn on_reopen(has_visible_windows: bool) {
    if !has_visible_windows {
        show_window();
    }
}

fn on_tray_event(event: TrayEvent) {
    match action(event) {
        Action::OpenWindow => show_window(),
        Action::Quit => aimer::aimer_quiver::windows::quit(),
        Action::Nothing => {}
    }
}

fn menu() -> TrayMenu {
    TrayMenu::new().item(OPEN, "Open window").separator().item(QUIT, "Quit Jaime")
}

/// What keeps the app present without a window. Dropping it removes the tray
/// icon and the exit hook.
pub struct Background {
    /// `None` where no tray can be shown; the app then still stays alive, and
    /// ends only through a signal or `quit`.
    _tray: Option<AppTray>,
    _exit: LifecycleSubscription,
    _reopen: ReopenSubscription,
}

/// Puts the tray icon on screen and stops the daemon when the loop exits. Call
/// it on the UI thread, once the native application exists.
pub fn install() -> Background {
    let icon = aimer::lucide_tray_icon(Icons::BELL, 64, aimer::svg::SvgColor::rgba8(40, 120, 255, 255))
        .expect("a rasterized icon is a valid RGBA buffer");
    let tray = match tray::install(Tray::new().icon(icon).tooltip("Jaime").menu(menu()), on_tray_event) {
        Ok(installed) => Some(installed),
        Err(why) => {
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            eprintln!("jaime: no tray icon, closing the window will not leave a way back in: {why}");
            None
        }
    };
    let exit = on_lifecycle(|event| {
        if event == Lifecycle::Exit {
            crate::daemon_example::stop_on_exit();
        }
    });
    Background { _tray: tray, _exit: exit, _reopen: reopen::subscribe(on_reopen) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_item_and_a_double_click_open_a_window() {
        assert_eq!(action(TrayEvent::MenuItem(OPEN)), Action::OpenWindow);
        assert_eq!(action(TrayEvent::DoubleClick(TrayButton::Left)), Action::OpenWindow);
    }

    #[test]
    fn an_open_window_is_reused_and_none_open_means_a_new_one() {
        let main = WindowId::MAIN;
        assert_eq!(open_among(&[main, main], |_| true), Some(main));
        assert_eq!(open_among(&[main, main], |_| false), None);
        assert_eq!(open_among(&[], |_| true), None);
    }

    #[test]
    fn the_quit_item_quits() {
        assert_eq!(action(TrayEvent::MenuItem(QUIT)), Action::Quit);
    }

    #[test]
    fn everything_else_is_ignored() {
        assert_eq!(action(TrayEvent::Click(TrayButton::Left)), Action::Nothing);
        assert_eq!(action(TrayEvent::DoubleClick(TrayButton::Right)), Action::Nothing);
        assert_eq!(action(TrayEvent::MenuItem(TrayItemId(99))), Action::Nothing);
    }
}
