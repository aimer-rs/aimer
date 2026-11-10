//! The daemon: its builder, the handle other threads hold, and the running state.

use std::fmt;
use std::marker::PhantomData;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use aimer_tray::{Tray, TrayError, TrayHandle};

use crate::scheduler::{Scheduler, TaskHandle};
use crate::{run_loop, signals};

type QuitHook = Box<dyn FnOnce() + Send>;

/// Why a daemon could not start or wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonError {
    /// The tray icon could not be shown.
    Tray(TrayError),
    /// macOS only: a daemon with a tray must be started, and waited on, from
    /// the main thread, which is where the system event loop lives.
    NotMainThread,
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tray(error) => write!(f, "the daemon's tray failed: {error}"),
            Self::NotMainThread => f.write_str("the daemon must run on the main thread"),
        }
    }
}

impl std::error::Error for DaemonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Tray(error) => Some(error),
            Self::NotMainThread => None,
        }
    }
}

impl From<TrayError> for DaemonError {
    #[inline]
    fn from(error: TrayError) -> Self {
        Self::Tray(error)
    }
}

/// State shared by the builder, every handle and the running daemon.
pub(crate) struct Shared {
    quit: Mutex<bool>,
    quit_signal: Condvar,
    /// `None` once the hooks have run, so a late [`DaemonHandle::on_quit`]
    /// runs at once instead of being lost.
    hooks: Mutex<Option<Vec<QuitHook>>>,
    finished: AtomicBool,
    scheduler: Scheduler,
}

impl Shared {
    fn new() -> Self {
        Self {
            quit: Mutex::new(false),
            quit_signal: Condvar::new(),
            hooks: Mutex::new(Some(Vec::new())),
            finished: AtomicBool::new(false),
            scheduler: Scheduler::start(),
        }
    }

    pub(crate) fn is_quitting(&self) -> bool {
        *self.quit.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Returns `true` for the call that actually flipped the flag.
    fn request_quit(&self) -> bool {
        let mut quit = self.quit.lock().unwrap_or_else(|p| p.into_inner());
        let first = !*quit;
        *quit = true;
        drop(quit);
        if first {
            self.quit_signal.notify_all();
        }
        first
    }

    pub(crate) fn wait_for_quit(&self) {
        let mut quit = self.quit.lock().unwrap_or_else(|p| p.into_inner());
        while !*quit {
            quit = self.quit_signal.wait(quit).unwrap_or_else(|p| p.into_inner());
        }
    }

    /// Runs the shutdown sequence once: hooks in registration order, then the
    /// scheduler. Later calls do nothing.
    fn finish(&self) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        self.request_quit();
        signals::release();
        let hooks = self.hooks.lock().unwrap_or_else(|p| p.into_inner()).take();
        for hook in hooks.into_iter().flatten() {
            // One failing hook must not skip the rest of the cleanup.
            let _ = panic::catch_unwind(AssertUnwindSafe(hook));
        }
        self.scheduler.stop();
    }
}

/// A cloneable, thread-safe way to steer a daemon: ask it to quit, schedule
/// work on it, register cleanup.
///
/// Handles are cheap to clone and may be moved into tray callbacks and jobs.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use aimer_daemon::Daemon;
///
/// let daemon = Daemon::new();
/// let handle = daemon.handle();
/// handle.after(Duration::from_secs(3600), || println!("an hour later"));
/// handle.on_quit(|| println!("saving"));
/// handle.quit();
/// assert!(handle.is_quitting());
/// ```
#[derive(Clone)]
pub struct DaemonHandle {
    shared: Arc<Shared>,
}

impl DaemonHandle {
    /// Asks the daemon to stop. Returns immediately; [`Running::wait`] returns
    /// and the quit hooks run on the thread that was waiting.
    ///
    /// Callable from any thread, any number of times.
    pub fn quit(&self) {
        if self.shared.request_quit() {
            run_loop::wake();
        }
    }

    /// Whether [`DaemonHandle::quit`] was called, by you or by a signal.
    #[inline]
    pub fn is_quitting(&self) -> bool {
        self.shared.is_quitting()
    }

    /// Runs `job` once, as soon as the scheduler thread is free.
    ///
    /// Jobs share one thread and run one at a time, so a long job delays the
    /// others; hand heavy work to a thread of its own. A job that panics is
    /// dropped and the scheduler carries on.
    #[inline]
    pub fn spawn(&self, job: impl FnOnce() + Send + 'static) -> TaskHandle {
        self.after(Duration::ZERO, job)
    }

    /// Runs `job` once, `delay` from now.
    pub fn after(&self, delay: Duration, job: impl FnOnce() + Send + 'static) -> TaskHandle {
        let mut job = Some(job);
        self.shared.scheduler.schedule(
            delay,
            None,
            Box::new(move || {
                if let Some(job) = job.take() {
                    job();
                }
            }),
        )
    }

    /// Runs `job` every `period`, first after one `period`.
    ///
    /// A run that overruns its slot makes the scheduler skip the missed ticks
    /// instead of firing them in a burst. Stop it with [`TaskHandle::cancel`].
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero, which would spin the scheduler thread.
    pub fn every(&self, period: Duration, job: impl FnMut() + Send + 'static) -> TaskHandle {
        assert!(!period.is_zero(), "a repeating job needs a non-zero period");
        self.shared.scheduler.schedule(period, Some(period), Box::new(job))
    }

    /// Registers cleanup to run once when the daemon shuts down, in the order
    /// registered.
    ///
    /// If the daemon has already shut down, `hook` runs immediately.
    pub fn on_quit(&self, hook: impl FnOnce() + Send + 'static) {
        let mut hooks = self.shared.hooks.lock().unwrap_or_else(|p| p.into_inner());
        match hooks.as_mut() {
            Some(list) => list.push(Box::new(hook)),
            None => {
                drop(hooks);
                hook();
            }
        }
    }
}

impl fmt::Debug for DaemonHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DaemonHandle")
            .field("quitting", &self.is_quitting())
            .finish_non_exhaustive()
    }
}

/// Describes a daemon; [`Daemon::start`] or [`Daemon::run`] brings it up.
///
/// A daemon is the lightweight counterpart of a full Aimer application: no
/// window, no renderer, no widget tree. It has a scheduler for timers and jobs,
/// optionally a tray icon with a menu, and a graceful shutdown that signals and
/// the tray can both trigger.
///
/// The [`DaemonHandle`] exists before the daemon starts, so a tray callback can
/// capture it.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use aimer_daemon::{Daemon, Tray, TrayEvent, TrayIcon, TrayItemId, TrayMenu};
///
/// const QUIT: TrayItemId = TrayItemId(1);
///
/// let daemon = Daemon::new();
/// let handle = daemon.handle();
/// let icon = TrayIcon::from_rgba(2, 2, [40, 120, 255, 255].repeat(4)).unwrap();
///
/// handle.every(Duration::from_secs(60), || println!("tick"));
///
/// let quit = handle.clone();
/// daemon
///     .tray(
///         Tray::new()
///             .icon(icon)
///             .tooltip("My daemon")
///             .menu(TrayMenu::new().item(QUIT, "Quit"))
///             .on_event(move |event| {
///                 if event == TrayEvent::MenuItem(QUIT) {
///                     quit.quit();
///                 }
///             }),
///     )
///     .run()
///     .unwrap();
/// ```
pub struct Daemon {
    shared: Arc<Shared>,
    tray: Option<Tray>,
    handle_signals: bool,
    show_in_dock: bool,
}

impl Daemon {
    /// Creates a daemon that has no tray, handles termination signals and, on
    /// macOS, stays out of the Dock.
    ///
    /// This already starts the scheduler thread, so jobs may be queued through
    /// [`Daemon::handle`] before the daemon starts.
    #[inline]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared::new()),
            tray: None,
            handle_signals: true,
            show_in_dock: false,
        }
    }

    /// The handle for this daemon.
    #[inline]
    pub fn handle(&self) -> DaemonHandle {
        DaemonHandle { shared: Arc::clone(&self.shared) }
    }

    /// Shows `tray` while the daemon runs. Route its events to
    /// [`DaemonHandle::quit`] or your own logic with `Tray::on_event`.
    #[inline]
    pub fn tray(mut self, tray: Tray) -> Self {
        self.tray = Some(tray);
        self
    }

    /// Whether `SIGTERM`/`SIGINT`/`SIGHUP` (Ctrl-C, Ctrl-Break and console
    /// close on Windows) request a graceful quit. The default is `true`.
    #[inline]
    pub fn handle_signals(mut self, handle_signals: bool) -> Self {
        self.handle_signals = handle_signals;
        self
    }

    /// macOS only: keep a Dock icon while running. The default is `false`, the
    /// usual choice for a tray-only process. Ignored elsewhere.
    #[inline]
    pub fn show_in_dock(mut self, show_in_dock: bool) -> Self {
        self.show_in_dock = show_in_dock;
        self
    }

    /// Brings the daemon up without blocking: shows the tray and installs the
    /// signal handling.
    ///
    /// Use this where something else owns the thread, such as a mobile host.
    /// Otherwise [`Daemon::run`] is the shorter way.
    ///
    /// # Errors
    ///
    /// [`DaemonError::NotMainThread`] on macOS when there is a tray and this is
    /// not the main thread, and [`DaemonError::Tray`] if the tray icon cannot
    /// be shown.
    pub fn start(self) -> Result<Running, DaemonError> {
        run_loop::prepare(self.show_in_dock, self.tray.is_some())?;
        let tray = self.tray.map(Tray::build).transpose()?;
        let running = Running {
            shared: Arc::clone(&self.shared),
            tray,
            _not_send: PhantomData,
        };
        if self.handle_signals {
            signals::install(running.handle());
        }
        Ok(running)
    }

    /// Starts the daemon and blocks until it is asked to quit, then runs the
    /// quit hooks and stops the scheduler.
    ///
    /// # Errors
    ///
    /// The errors of [`Daemon::start`].
    pub fn run(self) -> Result<(), DaemonError> {
        self.start()?.wait();
        Ok(())
    }
}

impl Default for Daemon {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Daemon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Daemon").finish_non_exhaustive()
    }
}

/// A started daemon. Dropping it shuts the daemon down.
///
/// It stays on the thread that started it: the tray handle it owns is
/// main-thread bound on macOS.
pub struct Running {
    shared: Arc<Shared>,
    tray: Option<TrayHandle>,
    _not_send: PhantomData<*const ()>,
}

impl Running {
    /// A handle for steering the daemon from anywhere.
    #[inline]
    pub fn handle(&self) -> DaemonHandle {
        DaemonHandle { shared: Arc::clone(&self.shared) }
    }

    /// The live tray icon, to change its picture, tooltip or menu; `None` if the
    /// daemon was built without one.
    #[inline]
    pub fn tray_mut(&mut self) -> Option<&mut TrayHandle> {
        self.tray.as_mut()
    }

    /// Blocks until the daemon is asked to quit, then runs the quit hooks, stops
    /// the scheduler and removes the tray icon.
    ///
    /// On macOS, on the main thread, this runs the system event loop, which is
    /// what keeps the tray icon alive; call it there when there is a tray.
    pub fn wait(self) {
        run_loop::block_until_quit(&self.shared);
        // Dropping `self` finishes the shutdown.
    }

    /// Shuts the daemon down now, without waiting for a request.
    #[inline]
    pub fn shutdown(self) {
        drop(self);
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // Hooks and the scheduler stop before the tray icon is removed, which
        // happens when the fields drop after this body.
        self.shared.finish();
    }
}

impl fmt::Debug for Running {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Running")
            .field("has_tray", &self.tray.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn quit_is_idempotent_and_visible_on_every_handle() {
        let daemon = Daemon::new();
        let a = daemon.handle();
        let b = a.clone();
        assert!(!a.is_quitting());
        a.quit();
        b.quit();
        assert!(a.is_quitting() && b.is_quitting());
    }

    #[test]
    fn hooks_run_once_in_registration_order() {
        let daemon = Daemon::new().handle_signals(false);
        let handle = daemon.handle();
        let order = Arc::new(Mutex::new(Vec::new()));
        for n in 1..=3 {
            let order = Arc::clone(&order);
            handle.on_quit(move || order.lock().unwrap().push(n));
        }
        let running = daemon.start().unwrap();
        handle.quit();
        running.wait();
        assert_eq!(*order.lock().unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn a_panicking_hook_does_not_skip_the_rest() {
        let daemon = Daemon::new().handle_signals(false);
        let handle = daemon.handle();
        let ran = Arc::new(AtomicUsize::new(0));
        handle.on_quit(|| panic!("boom"));
        let counter = Arc::clone(&ran);
        handle.on_quit(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        daemon.start().unwrap().shutdown();
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_hook_added_after_shutdown_runs_immediately() {
        let daemon = Daemon::new().handle_signals(false);
        let handle = daemon.handle();
        daemon.start().unwrap().shutdown();
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        handle.on_quit(move || flag.store(true, Ordering::SeqCst));
        assert!(ran.load(Ordering::SeqCst));
    }

    #[test]
    fn wait_returns_when_another_thread_quits() {
        let daemon = Daemon::new().handle_signals(false);
        let handle = daemon.handle();
        let running = daemon.start().unwrap();
        let quitter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            handle.quit();
        });
        running.wait();
        quitter.join().unwrap();
    }

    #[test]
    fn wait_returns_at_once_after_an_earlier_quit() {
        let daemon = Daemon::new().handle_signals(false);
        daemon.handle().quit();
        daemon.start().unwrap().wait();
    }

    #[test]
    fn scheduled_jobs_run_and_stop_with_the_daemon() {
        let daemon = Daemon::new().handle_signals(false);
        let handle = daemon.handle();
        let (tx, rx) = mpsc::channel();
        let quitter = handle.clone();
        handle.after(Duration::from_millis(10), move || {
            tx.send(()).unwrap();
            quitter.quit();
        });
        daemon.run().unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // After shutdown the scheduler refuses new work.
        assert!(handle.spawn(|| {}).is_cancelled());
    }

    #[test]
    #[should_panic(expected = "non-zero period")]
    fn a_zero_period_is_rejected() {
        Daemon::new().handle().every(Duration::ZERO, || {});
    }

    #[test]
    fn a_daemon_without_a_tray_reports_none() {
        let daemon = Daemon::new().handle_signals(false);
        let mut running = daemon.start().unwrap();
        assert!(running.tray_mut().is_none());
    }
}
