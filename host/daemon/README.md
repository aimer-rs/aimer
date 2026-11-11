# aimer_daemon

A lightweight background process for Aimer. Where a full Aimer application owns
a window, a renderer, a layout engine and a widget tree, a daemon is only what a
resident helper needs:

- **[`Daemon`]** describes the process and [`Daemon::run`] blocks until it is
  asked to quit; [`Daemon::start`] brings it up without blocking, for hosts that
  own the thread.
- **[`DaemonHandle`]** is the cloneable, thread-safe remote: [`quit`], delayed
  and repeating jobs ([`after`], [`every`], [`spawn`]) and shutdown hooks
  ([`on_quit`]).
- **[`Tray`]** (from `aimer_tray`) is the optional interface: a menu-bar or
  notification-area icon with a menu.
- **[`Autostart`]** registers the program to start at login.

It depends on `aimer_tray` and the operating system, and on nothing that draws:
no GPU backend, no text shaping, no windowing library. An idle daemon sleeps on
a condition variable; it never polls.

```no_run
use std::time::Duration;
use aimer_daemon::{Daemon, Tray, TrayEvent, TrayIcon, TrayItemId, TrayMenu};

const QUIT: TrayItemId = TrayItemId(1);

let daemon = Daemon::new();
let handle = daemon.handle();
handle.every(Duration::from_secs(60), || println!("sync"));
handle.on_quit(|| println!("saving"));

let icon = TrayIcon::from_rgba(2, 2, [40, 120, 255, 255].repeat(4)).unwrap();
let quit = handle.clone();
daemon
    .tray(
        Tray::new()
            .icon(icon)
            .tooltip("My daemon")
            .menu(TrayMenu::new().item(QUIT, "Quit"))
            .on_event(move |event| {
                if event == TrayEvent::MenuItem(QUIT) {
                    quit.quit();
                }
            }),
    )
    .run()
    .unwrap();
```

Availability: macOS, Windows, Linux

## Lifecycle

A quit can come from a [`DaemonHandle`], a tray menu item or the operating
system (`SIGTERM`, `SIGINT` and `SIGHUP` on Unix; Ctrl-C, Ctrl-Break and console
close on Windows). Whichever comes first, shutdown runs once, in this order:

1. [`Running::wait`] returns.
2. Quit hooks run in the order they were registered; a panicking hook does not
   skip the rest.
3. The scheduler drops its queued jobs and its thread is joined.
4. The tray icon is removed.

A second Ctrl-C or `SIGTERM` during a graceful quit terminates the process the
usual way, so a stuck hook can never trap the user.

## Timers and jobs

All jobs run one at a time on a single scheduler thread. That keeps the daemon
at one extra thread, but a long job delays the others: hand heavy work to a
thread of its own. A repeating job that overruns its period skips the missed
ticks rather than firing them in a burst, and a job that panics is dropped
without taking the scheduler down.

## Living on after the last window closes

A windowed app can keep a daemon running while it has no window, without
becoming a separate process. By default closing the last window ends the app
(`ExitPolicy::LastWindowClosed`), and the process dies with the scheduler thread
in it. To stay resident instead:

1. Build the app with `AimerApp::exit_policy(ExitPolicy::Never)`. Closing windows
   then leaves the event loop alone; it ends only on a quit request or a signal.
2. Give the user a way back and a way out: a tray icon (`aimer::tray`) whose menu
   has *Open* (`NewWindow::new()...open()`) and *Quit* (`windows::quit()`).
   Without it nothing is on screen and the process can only be killed. On macOS
   a click on the Dock icon is the other way back: subscribe with
   `aimer::reopen::subscribe`, which hears it on the UI thread with whether a
   window is showing, and open one when none is. When the app has one main
   window, check `WindowId::is_open` first and `focus` (after
   `set_minimized(false)`) the window that exists instead of opening a second.
3. Start the daemon with `Daemon::start`, not `run`, and turn off what the app
   already owns: `handle_signals(false)` because the app handles signals, and
   `show_in_dock(true)` on macOS if the app keeps its Dock icon. Keep the
   returned [`Running`] alive for as long as the daemon should run.
4. Drop that [`Running`] when the loop ends. Quit hooks and the scheduler stop
   run only in its `Drop`, and the daemon does not subscribe to the app's
   lifecycle on its own. Listen for `Lifecycle::Exit`
   (`aimer::venus::on_lifecycle`) and drop it there, so hooks run and the
   scheduler thread is joined instead of being cut off with the process.

```rust,ignore
AimerApp::new()
    .exit_policy(ExitPolicy::Never)
    .setup(|| {
        // Runs on the main thread once the native application exists; what it
        // returns lives until shutdown.
        let running = Daemon::new().handle_signals(false).start().unwrap();
        running.handle().every(Duration::from_secs(60), sync);
        let tray = tray::install(menu_with_open_and_quit(), on_tray_event);
        let exit = on_lifecycle(|event| {
            if event == Lifecycle::Exit {
                // Drop the `Running` here: hooks run, the scheduler is joined.
            }
        });
        (running, tray, exit)
    })
    .child(root)
    .run();
```

The Jaime showcase does exactly this in `app/jaime/src/background.rs` (tray,
`ExitPolicy::Never`, exit hook) and `app/jaime/src/daemon_example.rs` (the
daemon itself).

Keep in mind:

- **Quit.** With `ExitPolicy::Never` the only ways out are `windows::quit()` and
  a process signal. Always offer a Quit item.
- **Autostart.** `Autostart::new(id).enable()` starts the app at login, so the
  resident app is there from boot.
- **Linux.** Desktops without a tray (some GNOME setups) leave no visible way
  back in once the window is closed.
- **macOS.** The Dock icon stays after the last window closes, and clicking it
  does nothing unless the app subscribes to `aimer::reopen`. A menu-bar-only
  app also needs the accessory activation policy, which a daemon with a tray
  sets for you (see below) and a windowed app does not.
- **Window position and size.** Give the window a name with
  `WindowAttr::remember("main")`, on the app's first window and on the ones it
  opens later, and it comes back where and as big as it was left: position,
  size, maximized and full screen. It is saved when the window closes and when
  the app quits, in `window-state.txt` in the app's data directory, so it also
  survives a restart. A saved position that is on no current screen is ignored,
  and a maximized or full-screen window does not overwrite the size it will
  return to.
- **Window styling.** What the app installs with `MacosWindowing` applies to
  every window it opens, so a window opened from the tray or the Dock looks like
  the first one. Only its `on_created` callback runs once, for the first.
- **Mobile and web.** `Never` is not background execution. iOS and Android
  suspend or kill the app; a foreground service or background mode is needed.
  The web has no background process at all.

## Platforms

| Platform | Run loop                                         | Autostart                         |
| -------- | ------------------------------------------------ | --------------------------------- |
| macOS    | `NSApplication` on the main thread (with a tray) | launchd agent                     |
| Windows  | waits on a condition variable                    | `HKCU\...\Run` registry value     |
| Linux    | waits on a condition variable                    | XDG autostart `.desktop` entry    |
| Mobile   | [`Daemon::start`]; the host owns the thread      | unsupported (the OS launches apps) |

On macOS a daemon with a tray must start and wait on the main thread, and by
default stays out of the Dock ([`Daemon::show_in_dock`]). A daemon without a
tray runs on any thread. On iOS and Android there is no tray, so
[`Daemon::start`] with a [`Tray`] reports [`TrayError::Unsupported`]; use the
scheduler and hooks only.

[`quit`]: DaemonHandle::quit
[`after`]: DaemonHandle::after
[`every`]: DaemonHandle::every
[`spawn`]: DaemonHandle::spawn
[`on_quit`]: DaemonHandle::on_quit
[`TrayError::Unsupported`]: TrayError::Unsupported
