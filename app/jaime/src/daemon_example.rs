//! A lightweight daemon running next to the UI.
//!
//! A daemon is the counterpart of a full application: no window, no renderer,
//! only a scheduler for timers and jobs, a graceful shutdown with cleanup hooks,
//! and login autostart. This page starts one inside the showcase to show what it
//! offers:
//!
//! - **Start the daemon** brings up the scheduler and a repeating timer whose
//!   ticks are counted on the page.
//! - **Run a one-shot job** queues work 1.5 seconds ahead; it runs on the
//!   daemon's own thread.
//! - **Stop the daemon** shuts it down: the quit hook registered at start runs,
//!   then the scheduler stops.
//! - **Enable / Disable autostart** adds or removes the login entry of this
//!   platform (a launchd agent, an XDG entry or a registry value).
//!
//! The daemon's jobs run off the UI thread, so they only touch the thread-safe
//! log and counter below and then ask the event loop to redraw the page through
//! [`aimer::tray::post`], whose handler runs on the UI thread. No tray is shown
//! and no signal handler is installed: the showcase already owns both.

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use aimer::daemon::{Autostart, Running, TaskHandle};
use aimer::input::button::Button;
use aimer::style::*;
use aimer::tray::{self, TrayEvent, TrayId, TrayItemId, TraySubscription};
use aimer::*;

/// How many log lines the page keeps.
const LOG_LINES: usize = 8;

/// The login entry this page adds and removes.
const AUTOSTART_ID: &str = "com.example.jaime.daemon-demo";

/// How often the repeating timer fires.
const TICK_PERIOD: Duration = Duration::from_secs(1);

/// How far ahead the one-shot job is queued.
const JOB_DELAY: Duration = Duration::from_millis(1500);

/// The newest log lines, oldest first. Daemon jobs write it from their own
/// thread.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Ticks of the repeating timer since the daemon started.
static TICKS: AtomicU32 = AtomicU32::new(0);
/// The raw [`TrayId`] of the subscription that redraws the page.
static WAKE_ID: AtomicU32 = AtomicU32::new(0);

/// A started daemon and its repeating timer.
struct Active {
    running: Running,
    ticker: TaskHandle,
}

aimer_tls::thread_local! {
    /// The daemon, if one is running. It stays on the thread that started it.
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
    /// The page that is showing the log, if one is.
    static PAGE: std::cell::Cell<Option<StateUpdater<DaemonExampleState>>> = const { std::cell::Cell::new(None) };
    /// Keeps the redraw handler registered for the life of the thread.
    static WAKER: RefCell<Option<TraySubscription>> = const { RefCell::new(None) };
}

/// Appends `entry` to `log`, dropping the oldest line past [`LOG_LINES`].
fn push_capped(log: &mut Vec<String>, entry: String) {
    if log.len() == LOG_LINES {
        log.remove(0);
    }
    log.push(entry);
}

/// Appends `entry` to the log and asks the UI thread to redraw. Callable from
/// any thread.
fn record(entry: impl Into<String>) {
    push_capped(&mut LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner()), entry.into());
    wake();
}

/// Asks the event loop, from any thread, to redraw the page.
fn wake() {
    let id = WAKE_ID.load(Ordering::Relaxed);
    if id != 0 {
        tray::post(TrayId::from_raw(id), TrayEvent::MenuItem(TrayItemId(0)));
    }
}

/// Redraws the page, if one is showing. A page that has gone away rejects the
/// update, which is the right outcome. UI thread only.
fn refresh_page() {
    if let Some(page) = PAGE.get() {
        let _ = page.try_set_state(|_| {});
    }
}

/// Registers the handler that turns [`wake`] into a redraw, once per thread.
fn ensure_waker() {
    WAKER.with_borrow_mut(|waker| {
        if waker.is_none() {
            let subscription = tray::subscribe(|_| refresh_page());
            WAKE_ID.store(subscription.id().as_raw(), Ordering::Relaxed);
            *waker = Some(subscription);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn start_daemon() {
    use aimer::daemon::Daemon;

    if ACTIVE.with_borrow(Option::is_some) {
        record("The daemon is already running.");
        return;
    }
    // No tray and no signals: the showcase owns the event loop. The Dock icon
    // stays because this process is a full application.
    let daemon = Daemon::new().handle_signals(false).show_in_dock(true);
    match daemon.start() {
        Ok(running) => {
            let handle = running.handle();
            TICKS.store(0, Ordering::Relaxed);
            handle.on_quit(|| record("Quit hook ran: state saved."));
            let ticker = handle.every(TICK_PERIOD, || {
                TICKS.fetch_add(1, Ordering::Relaxed);
                wake();
            });
            ACTIVE.with_borrow_mut(|active| *active = Some(Active { running, ticker }));
            record("The daemon started.");
        }
        Err(error) => record(format!("The daemon could not start: {error}")),
    }
}

#[cfg(target_arch = "wasm32")]
fn start_daemon() {
    record("The web has no threads for the daemon's scheduler.");
}

fn stop_daemon() {
    let Some(active) = ACTIVE.with_borrow_mut(Option::take) else {
        record("The daemon is not running.");
        return;
    };
    active.ticker.cancel();
    // Dropping runs the quit hooks, then stops the scheduler.
    drop(active.running);
    record("The daemon stopped.");
}

/// Stops the daemon because the event loop is ending. Runs the quit hooks and
/// the scheduler stop like [`stop_daemon`], but logs nothing and wakes nobody:
/// no page is left to redraw. UI thread only.
pub fn stop_on_exit() {
    let Some(active) = ACTIVE.with_borrow_mut(Option::take) else { return };
    active.ticker.cancel();
    drop(active.running);
}

fn run_job() {
    let handle = ACTIVE.with_borrow(|active| active.as_ref().map(|active| active.running.handle()));
    match handle {
        Some(handle) => {
            handle.after(JOB_DELAY, || record("The one-shot job ran on the daemon's thread."));
            record("A one-shot job is queued for 1.5 seconds ahead.");
        }
        None => record("Start the daemon first."),
    }
}

/// The login entry for this app.
fn autostart() -> Autostart {
    Autostart::new(AUTOSTART_ID)
}

fn enable_autostart() {
    match autostart().enable() {
        Ok(()) => record("Autostart enabled: this app starts at login."),
        Err(error) => record(format!("Autostart failed: {error}")),
    }
}

fn disable_autostart() {
    match autostart().disable() {
        Ok(()) => record("Autostart disabled."),
        Err(error) => record(format!("Autostart failed: {error}")),
    }
}

/// The daemon page.
#[derive(StatefulWidget)]
pub struct DaemonExample;

impl DaemonExample {
    /// Creates the page.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Default for DaemonExample {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// The retained side of the page. The log and the daemon outlive the page, so
/// all it keeps is a way to ask for a redraw.
pub struct DaemonExampleState {
    updater: StateUpdater<Self>,
}

impl StatefulWidget for DaemonExample {
    type State = DaemonExampleState;

    fn create_state(self) -> DaemonExampleState {
        DaemonExampleState { updater: StateUpdater::empty() }
    }
}

impl DaemonExampleState {
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

impl State<DaemonExample> for DaemonExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
        PAGE.set(Some(updater));
        ensure_waker();
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let muted = crate::theme::muted_text(&app_theme);
        let body = app_theme.on_background_color;

        let log: Vec<AnyWidget> = {
            let log = LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if log.is_empty() {
                vec![Self::line("Nothing yet. Start the daemon.", muted, 14)]
            } else {
                log.iter().rev().map(|entry| Self::line(entry.as_str(), body, 14)).collect()
            }
        };
        let running = ACTIVE.with_borrow(Option::is_some);
        let status = if running {
            format!("The daemon is running. Ticks: {}", TICKS.load(Ordering::Relaxed))
        } else {
            "The daemon is stopped.".to_owned()
        };

        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Start)
                    .gaps(LayoutSpacing::new().bottom(12))
                    .children(vec![
                        Self::heading("Daemon", &app_theme),
                        Self::line(status, body, 15),
                        Row::new()
                            .gaps(LayoutSpacing::new().right(16))
                            .children(vec![
                                Self::button("Start the daemon", &app_theme, start_daemon),
                                Self::button("Stop the daemon", &app_theme, stop_daemon),
                                Self::button("Run a one-shot job", &app_theme, run_job),
                            ])
                            .boxed(),
                        Row::new()
                            .gaps(LayoutSpacing::new().right(16))
                            .children(vec![
                                Self::button("Enable autostart", &app_theme, enable_autostart),
                                Self::button("Disable autostart", &app_theme, disable_autostart),
                            ])
                            .boxed(),
                        Self::line(
                            "The timer ticks once a second on the daemon's own thread. Stopping runs the quit hook \
                             first, then ends the scheduler.",
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
                            "Autostart is a launchd agent on macOS, an XDG entry on Linux and a registry value on \
                             Windows; mobile and web have no login items. A standalone daemon with a tray icon and \
                             no window is the `daemon_demo` example of the aimer_daemon crate.",
                            muted,
                            14,
                        ),
                    ]),
            )
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::time::Instant;

    use super::*;

    fn log() -> Vec<String> {
        LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Waits, bounded, until the log satisfies `done`.
    fn wait_for(done: impl Fn(&[String]) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if done(&log()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn the_log_keeps_only_the_newest_lines() {
        let mut lines = Vec::new();
        for n in 0..LOG_LINES + 3 {
            push_capped(&mut lines, n.to_string());
        }
        assert_eq!(lines.len(), LOG_LINES);
        assert_eq!(lines.first().map(String::as_str), Some("3"));
        assert_eq!(lines.last().map(String::as_str), Some("10"));
    }

    #[test]
    fn the_autostart_id_is_a_valid_entry_name() {
        // `is_enabled` validates the id before touching the system, and a valid
        // id never fails with `InvalidInput`.
        let error = autostart().is_enabled().err();
        assert!(error.is_none_or(|e| e.kind() != std::io::ErrorKind::InvalidInput));
    }

    #[test]
    fn the_daemon_ticks_runs_jobs_and_runs_the_quit_hook_on_stop() {
        start_daemon();
        assert!(ACTIVE.with_borrow(Option::is_some), "the daemon starts without a tray off the main thread");

        run_job();
        assert!(
            wait_for(|log| log.iter().any(|line| line.starts_with("The one-shot job ran"))),
            "the job runs on the scheduler thread"
        );
        assert!(wait_for(|_| TICKS.load(Ordering::Relaxed) >= 1), "the repeating timer fires");

        stop_daemon();
        let log = log();
        let hook = log.iter().position(|line| line.starts_with("Quit hook ran")).expect("the hook ran");
        let stopped = log.iter().position(|line| line == "The daemon stopped.").expect("stop was logged");
        assert!(hook < stopped, "hooks run before stop returns: {log:?}");
        assert!(ACTIVE.with_borrow(Option::is_none));

        run_job();
        assert_eq!(self::log().last().map(String::as_str), Some("Start the daemon first."));
    }
}
