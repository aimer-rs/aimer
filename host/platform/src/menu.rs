//! The native macOS menu bar and the shortcuts it owns.
//!
//! macOS hands a `Cmd`-modified key press to the menu bar *before* the window
//! sees it: `NSApplication` asks the main menu to `performKeyEquivalent:` and,
//! when an item matches, the key event is consumed there and never reaches the
//! window's view. An `Edit` menu holding the standard `Cut` / `Copy` / `Paste` /
//! `Select All` items therefore swallows `Cmd`+`X`/`C`/`V`/`A` before any widget
//! can answer them — which is why those shortcuts used to do nothing on macOS
//! while their `Ctrl` counterparts worked everywhere.
//!
//! So the items are ours rather than Cocoa's: each carries an id and its native
//! accelerator, and an activation — by the menu or by its shortcut — is
//! translated back into the very same [`ElementEvent::KeyInput`] a `Ctrl`
//! shortcut produces, then dispatched into the widget tree. The menu keeps its
//! native look and its shortcut labels, and one code path answers the shortcut
//! on every platform.

use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};

/// The id prefix every item this module owns is registered under.
const EDIT_PREFIX: &str = "aimer.edit.";

/// One editing shortcut, as the widget tree understands it.
///
/// The letter is the one `WindowEventHandler` in `aimer_quiver`
/// puts in [`NamedKey::Other`] for a `Ctrl`/`Cmd` shortcut, so a menu
/// activation and a key press are indistinguishable to a widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuShortcut {
    /// The shortcut's letter, lowercase.
    pub letter: &'static str,
    /// Whether the shortcut is the shifted variant, as `Cmd`+`Shift`+`Z` is.
    pub shift: bool,
}

impl MenuShortcut {
    /// The event this shortcut delivers to the widget tree.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_platform::menu::MenuShortcut;
    /// use aimer_events::element::ElementEvent;
    ///
    /// let event = MenuShortcut { letter: "c", shift: false }.to_event();
    /// assert!(matches!(event, ElementEvent::KeyInput { .. }));
    /// ```
    #[inline]
    pub fn to_event(self) -> ElementEvent {
        ElementEvent::KeyInput {
            key: NamedKey::Other(self.letter.to_owned()),
            action: KeyAction::Pressed,
            modifiers: Modifiers {
                ctrl: false,
                shift: self.shift,
                alt: false,
                meta: true,
            },
        }
    }
}

/// The shortcut an activated menu item stands for, or `None` for an item this
/// module does not own.
///
/// # Examples
///
/// ```
/// use aimer_platform::menu::shortcut_for_menu_id;
///
/// assert_eq!(shortcut_for_menu_id("aimer.edit.copy").unwrap().letter, "c");
/// assert!(shortcut_for_menu_id("aimer.edit.redo").unwrap().shift);
/// assert!(shortcut_for_menu_id("aimer.help").is_none());
/// ```
pub fn shortcut_for_menu_id(id: &str) -> Option<MenuShortcut> {
    let action = id.strip_prefix(EDIT_PREFIX)?;
    let (letter, shift) = match action {
        "undo" => ("z", false),
        "redo" => ("z", true),
        "cut" => ("x", false),
        "copy" => ("c", false),
        "paste" => ("v", false),
        "select_all" => ("a", false),
        _ => return None,
    };
    Some(MenuShortcut { letter, shift })
}

/// Builds the application menu and starts forwarding its editing shortcuts.
///
/// The returned [`muda::Menu`] owns the native objects and must be kept alive
/// for as long as the application runs.
#[cfg(target_os = "macos")]
pub fn install_macos_menu() -> muda::Menu {
    use muda::accelerator::{Accelerator, Code, Modifiers as Accel};
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

    use crate::event::{AimerNativePlatformEvent, EVENT_PROXY};

    /// `Cmd` alone, the modifier every editing shortcut on macOS is built on.
    const CMD: Accel = Accel::META;

    let menu = Menu::new();

    let app_menu = Submenu::new("Aimer", true);
    app_menu
        .append_items(&[
            &PredefinedMenuItem::about(None, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ])
        .unwrap();

    let file_menu = Submenu::new("File", true);
    file_menu
        .append_items(&[
            &MenuItem::new("New", true, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(None),
        ])
        .unwrap();

    // Ours, not Cocoa's: a predefined item would send `copy:` to the first
    // responder — which is the window's view, and answers to none of it — while
    // still eating the key press on its way in.
    let edit_menu = Submenu::new("Edit", true);
    edit_menu
        .append_items(&[
            &edit_item("undo", "Undo", Accelerator::new(Some(CMD), Code::KeyZ)),
            &edit_item(
                "redo",
                "Redo",
                Accelerator::new(Some(CMD | Accel::SHIFT), Code::KeyZ),
            ),
            &PredefinedMenuItem::separator(),
            &edit_item("cut", "Cut", Accelerator::new(Some(CMD), Code::KeyX)),
            &edit_item("copy", "Copy", Accelerator::new(Some(CMD), Code::KeyC)),
            &edit_item("paste", "Paste", Accelerator::new(Some(CMD), Code::KeyV)),
            &edit_item(
                "select_all",
                "Select All",
                Accelerator::new(Some(CMD), Code::KeyA),
            ),
        ])
        .unwrap();

    let view_menu = Submenu::new("View", true);
    view_menu
        .append_items(&[&PredefinedMenuItem::fullscreen(None)])
        .unwrap();

    let window_menu = Submenu::new("Window", true);
    window_menu
        .append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(None),
        ])
        .unwrap();

    let help_menu = Submenu::new("Help", true);
    help_menu
        .append_items(&[&MenuItem::new("Aimer Help", true, None)])
        .unwrap();

    menu.append_items(&[
        &app_menu,
        &file_menu,
        &edit_menu,
        &view_menu,
        &window_menu,
        &help_menu,
    ])
    .unwrap();

    menu.init_for_nsapp();

    // The handler runs on the main thread, inside AppKit's own dispatch of the
    // menu command, so the work is handed to the event loop instead of being
    // done here: it is the loop that owns the widget tree.
    MenuEvent::set_event_handler(Some(|event: MenuEvent| {
        let Some(shortcut) = shortcut_for_menu_id(event.id.as_ref()) else {
            return;
        };
        let Some(proxy) = EVENT_PROXY.get() else {
            return;
        };
        let _ = proxy.send_event(AimerNativePlatformEvent::MenuShortcut(shortcut));
    }));

    menu
}

/// One `Edit` item, owned by Aimer and reachable by both click and shortcut.
#[cfg(target_os = "macos")]
fn edit_item(
    action: &str,
    title: &str,
    accelerator: muda::accelerator::Accelerator,
) -> muda::MenuItem {
    muda::MenuItem::with_id(
        format!("{EDIT_PREFIX}{action}"),
        title,
        true,
        Some(accelerator),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_edit_item_maps_to_the_letter_the_widgets_expect() {
        for (id, letter, shift) in [
            ("aimer.edit.undo", "z", false),
            ("aimer.edit.redo", "z", true),
            ("aimer.edit.cut", "x", false),
            ("aimer.edit.copy", "c", false),
            ("aimer.edit.paste", "v", false),
            ("aimer.edit.select_all", "a", false),
        ] {
            let shortcut = shortcut_for_menu_id(id).expect(id);
            assert_eq!(shortcut, MenuShortcut { letter, shift }, "{id}");
        }
    }

    #[test]
    fn an_item_we_do_not_own_has_no_shortcut() {
        assert!(shortcut_for_menu_id("aimer.edit.compose").is_none());
        assert!(shortcut_for_menu_id("File").is_none());
        assert!(shortcut_for_menu_id("").is_none());
    }

    #[test]
    fn the_event_looks_like_a_cmd_shortcut_press() {
        let event = MenuShortcut {
            letter: "a",
            shift: false,
        }
        .to_event();
        match event {
            ElementEvent::KeyInput {
                key,
                action,
                modifiers,
            } => {
                assert_eq!(key, NamedKey::Other("a".into()));
                assert_eq!(action, KeyAction::Pressed);
                assert!(modifiers.meta);
                assert!(!modifiers.ctrl);
                assert!(!modifiers.shift);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn the_shifted_variant_carries_shift() {
        let event = MenuShortcut {
            letter: "z",
            shift: true,
        }
        .to_event();
        match event {
            ElementEvent::KeyInput { modifiers, .. } => {
                assert!(modifiers.shift);
                assert!(modifiers.meta);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
