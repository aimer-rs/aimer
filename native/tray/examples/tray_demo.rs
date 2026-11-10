//! Shows a tray icon with a small menu until "Quit" is chosen.
//!
//! `cargo run -p aimer_tray --example tray_demo`

use aimer_tray::{Tray, TrayEvent, TrayIcon, TrayItemId, TrayMenu};

const HELLO: TrayItemId = TrayItemId(1);
const QUIT: TrayItemId = TrayItemId(2);

/// A 64x64 blue disc.
fn icon() -> TrayIcon {
    let size = 64u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - 31.5, y as f32 - 31.5);
            let inside = dx * dx + dy * dy <= 30.0 * 30.0;
            rgba.extend_from_slice(&if inside { [40, 120, 255, 255] } else { [0, 0, 0, 0] });
        }
    }
    TrayIcon::from_rgba(size, size, rgba).expect("the buffer is size * size * 4 bytes")
}

fn main() {
    let tray = Tray::new()
        .icon(icon())
        .tooltip("Aimer tray demo")
        .menu(TrayMenu::new().item(HELLO, "Say hello").separator().item(QUIT, "Quit"))
        .on_event(|event| {
            println!("tray event: {event:?}");
            if event == TrayEvent::MenuItem(QUIT) {
                std::process::exit(0);
            }
        });
    run(tray);
}

#[cfg(target_os = "macos")]
fn run(tray: Tray) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let mtm = MainThreadMarker::new().expect("main runs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon: the tray is the whole interface.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let _handle = tray.build().expect("the status item appears");
    app.run();
}

#[cfg(not(target_os = "macos"))]
fn run(tray: Tray) {
    let _handle = tray.build().expect("the tray icon appears");
    // Windows and Linux report events from the tray's own thread.
    std::thread::park();
}
