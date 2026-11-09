//! Local, menu and global shortcuts in one window.
//!
//! Each action prints a line, so what fired is visible in the terminal:
//!
//! - `Cmd/Ctrl+K`: a window shortcut, from a [`Shortcuts`] widget.
//! - `File > New Note` (`Cmd/Ctrl+N`): an application-menu item.
//! - `Cmd/Ctrl+Shift+Alt+G`: a global shortcut that fires even while another
//!   application has focus.
//!
//! ```text
//! cargo run -p aimer --example shortcuts
//! ```

use aimer::shortcut::global::GlobalShortcut;
use aimer::shortcut::menu::{AppMenu, Menu, set_app_menu};
use aimer::{AimerApp, Shortcuts, Text};

fn main() {
    set_app_menu(AppMenu::new().menu(
        Menu::new("File")
            .item("New Note", "CmdOrCtrl+N", || println!("menu: New Note"))
            .separator()
            .item("Archive Note", "CmdOrCtrl+Shift+D", || println!("menu: Archive Note")),
    ));

    AimerApp::new()
        .setup(|| {
            // The registry lives on the UI thread, where `setup` runs. The guard
            // is kept for the life of the process; dropping it would release
            // the key.
            match GlobalShortcut::register(
                "CmdOrCtrl+Shift+Alt+G".parse().expect("a valid chord"),
                || println!("global: summoned"),
            ) {
                Ok(held) => std::mem::forget(held),
                Err(why) => println!("global shortcut unavailable: {why}"),
            }
        })
        .child(
            Shortcuts::new()
                .window(true)
                .bind_str("CmdOrCtrl+K", || println!("local: command palette"))
                .child(Text::new("Press Cmd/Ctrl+K, use the File menu, or Cmd/Ctrl+Shift+Alt+G anywhere")),
        )
        .run();
}
