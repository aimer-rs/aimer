//! A tray daemon with a repeating timer, until "Quit" is chosen or the process is
//! sent Ctrl-C / `SIGTERM`.
//!
//! `cargo run -p aimer_daemon --example daemon_demo`

use std::time::Duration;

use aimer_daemon::{Daemon, Tray, TrayEvent, TrayIcon, TrayItemId, TrayMenu};

const HELLO: TrayItemId = TrayItemId(1);
const QUIT: TrayItemId = TrayItemId(2);

/// A 64x64 green disc.
fn icon() -> TrayIcon {
    let size = 64u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - 31.5, y as f32 - 31.5);
            let inside = dx * dx + dy * dy <= 30.0 * 30.0;
            rgba.extend_from_slice(&if inside { [60, 190, 90, 255] } else { [0, 0, 0, 0] });
        }
    }
    TrayIcon::from_rgba(size, size, rgba).expect("the buffer is size * size * 4 bytes")
}

fn main() {
    let daemon = Daemon::new();
    let handle = daemon.handle();

    handle.every(Duration::from_secs(2), || println!("tick"));
    handle.on_quit(|| println!("quit hook: saving state"));

    let on_event = handle.clone();
    let tray = Tray::new()
        .icon(icon())
        .tooltip("Aimer daemon demo")
        .menu(TrayMenu::new().item(HELLO, "Say hello").separator().item(QUIT, "Quit"))
        .on_event(move |event| match event {
            TrayEvent::MenuItem(HELLO) => println!("hello"),
            TrayEvent::MenuItem(QUIT) => on_event.quit(),
            _ => {}
        });

    daemon.tray(tray).run().expect("the daemon starts");
    println!("daemon stopped");
}
