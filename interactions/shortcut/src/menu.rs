//! The application menu: titled items, each optionally bound to a chord.
//!
//! A menu is *data*. Describe it once with [`AppMenu`] and hand it to
//! [`set_app_menu`]; where the platform has a native menu bar (macOS) the
//! host builds one from it, with the shortcut shown beside each item and the
//! operating system routing the accelerator. Where it does not, the very same
//! chords still run the very same actions as window-wide shortcuts, so an
//! application describes its commands once and gets them everywhere.
//!
//! # Chords the native menu bar already owns
//!
//! On macOS the framework's built-in menus (`Aimer`, `Edit`, `Window`, ...)
//! claim their accelerators before a window sees any key, and those items win
//! over yours: `Cmd+H` and `Cmd+Opt+H` (Hide), `Cmd+Q`, `Cmd+W`, `Cmd+M`, and
//! the editing keys `Cmd+Z`, `Cmd+X`, `Cmd+C`, `Cmd+V`, `Cmd+A`. Choose other
//! chords for your own items. In practice a chord that collides is simply never
//! delivered to the item: `Cmd+Shift+H` did not reach one in the Jaime showcase.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::scope::{LiveToken, ShortcutAction};
use crate::{Activation, KeyChord};

static NEXT_ITEM: AtomicU32 = AtomicU32::new(1);

/// Identifies one menu item for as long as the process runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuItemId(u32);

impl MenuItemId {
    /// The id as a number, for crossing into a native menu.
    #[inline]
    pub const fn as_raw(self) -> u32 {
        self.0
    }

    /// The id a native menu handed back.
    #[inline]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// One activatable entry of a [`Menu`].
pub struct MenuItem {
    id: MenuItemId,
    title: String,
    chord: Option<KeyChord>,
    action: ShortcutAction,
}

impl MenuItem {
    /// The identity a native menu reports back when the item is chosen.
    #[inline]
    pub fn id(&self) -> MenuItemId {
        self.id
    }

    /// The label shown in the menu.
    #[inline]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The shortcut shown beside the label, and the chord that runs the item.
    #[inline]
    pub fn chord(&self) -> Option<KeyChord> {
        self.chord
    }
}

/// An entry of a [`Menu`].
pub enum MenuEntry {
    Item(MenuItem),
    Separator,
}

/// A titled list of items: `File`, `View`.
///
/// # Examples
///
/// ```
/// use aimer_shortcut::menu::Menu;
///
/// let file = Menu::new("File")
///     .item("New", "CmdOrCtrl+N", || println!("new"))
///     .separator()
///     .item("Quit", "CmdOrCtrl+Q", || println!("quit"));
/// assert_eq!(file.entries().len(), 3);
/// ```
pub struct Menu {
    title: String,
    entries: Vec<MenuEntry>,
}

impl Menu {
    /// Creates an empty menu.
    #[inline]
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), entries: Vec::new() }
    }

    /// Adds an item bound to the chord written as text.
    ///
    /// # Panics
    ///
    /// Panics if `chord` does not parse; it is a literal in the source. Use
    /// [`Menu::item_with`] for text that comes from elsewhere.
    #[inline]
    pub fn item(self, title: impl Into<String>, chord: &str, action: impl Fn() + 'static) -> Self {
        let chord = KeyChord::parse(chord).expect("`item` needs a valid key chord");
        self.item_with(title, Some(chord), action)
    }

    /// Adds an item with an optional, already parsed chord.
    pub fn item_with(
        mut self,
        title: impl Into<String>,
        chord: Option<KeyChord>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.entries.push(MenuEntry::Item(MenuItem {
            id: MenuItemId(NEXT_ITEM.fetch_add(1, Ordering::Relaxed)),
            title: title.into(),
            chord,
            action: Rc::new(action),
        }));
        self
    }

    /// Adds a separator line.
    #[inline]
    pub fn separator(mut self) -> Self {
        self.entries.push(MenuEntry::Separator);
        self
    }

    /// The label of the menu in the menu bar.
    #[inline]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The items and separators, in order.
    #[inline]
    pub fn entries(&self) -> &[MenuEntry] {
        &self.entries
    }
}

/// The menus of the application, left to right.
#[derive(Default)]
pub struct AppMenu {
    menus: Vec<Menu>,
}

impl AppMenu {
    /// Creates an application menu with no menus of its own.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a menu.
    #[inline]
    pub fn menu(mut self, menu: Menu) -> Self {
        self.menus.push(menu);
        self
    }

    /// The menus, left to right.
    #[inline]
    pub fn menus(&self) -> &[Menu] {
        &self.menus
    }
}

/// The installed menu, and the count that tells the dispatcher it exists.
struct Installed {
    menu: AppMenu,
    _live: LiveToken,
}

aimer_tls::thread_local! {
    static INSTALLED: RefCell<Option<Installed>> = const { RefCell::new(None) };
}

/// Makes `menu` the application's menu, replacing any earlier one.
///
/// Call it on the UI thread before the application starts, or at any later
/// time to swap the menu. A native host reads it with [`with_app_menu`] when it
/// builds the menu bar.
pub fn set_app_menu(menu: AppMenu) {
    let installed = Installed { menu, _live: LiveToken::new() };
    // The old menu is dropped after the borrow ends: its actions may own
    // things whose drop reaches back here.
    let old = INSTALLED.with(|slot| slot.borrow_mut().replace(installed));
    drop(old);
}

/// Removes the application menu.
pub fn clear_app_menu() {
    let old = INSTALLED.with(|slot| slot.borrow_mut().take());
    drop(old);
}

/// Reads the installed menu, if there is one.
pub fn with_app_menu<R>(read: impl FnOnce(Option<&AppMenu>) -> R) -> R {
    INSTALLED.with(|slot| read(slot.borrow().as_ref().map(|installed| &installed.menu)))
}

/// Runs the item a native menu reported as chosen. Returns whether there was
/// such an item.
pub fn activate_item(id: MenuItemId) -> bool {
    let action = find_action(|item| item.id == id);
    // The borrow is released first: an action may replace the menu.
    match action {
        Some(action) => {
            action();
            true
        }
        None => false,
    }
}

/// Runs the item bound to `chord`, as a window-wide shortcut.
///
/// Where a native menu bar owns the accelerators the chord never reaches this;
/// everywhere else this is what keeps the menu's shortcuts working.
pub fn activate_chord(chord: &KeyChord, repeating: bool) -> Activation {
    let action = find_action(|item| item.chord.as_ref() == Some(chord));
    match action {
        None => Activation::Unbound,
        Some(_) if repeating => Activation::Held,
        Some(action) => {
            action();
            Activation::Ran
        }
    }
}

/// The action of the first item `matches`, cloned out so no borrow of the
/// installed menu is held while it runs.
fn find_action(mut matches: impl FnMut(&MenuItem) -> bool) -> Option<ShortcutAction> {
    with_app_menu(|menu| {
        menu?.menus.iter().flat_map(|menu| menu.entries.iter()).find_map(|entry| match entry {
            MenuEntry::Item(item) if matches(item) => Some(Rc::clone(&item.action)),
            _ => None,
        })
    })
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod menu_tests;
