//! The menu a tray icon opens.

/// Names one menu item in the [`TrayEvent::MenuItem`](crate::TrayEvent::MenuItem)
/// that choosing it produces.
///
/// The number is yours: pick any, and keep them unique inside one menu. Keep it
/// below `i32::MAX`: Windows reports the chosen command as a signed number, so
/// an item with a larger id is left out of the menu there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrayItemId(pub u32);

/// A chooseable entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayMenuItem {
    pub id: TrayItemId,
    pub title: String,
    pub enabled: bool,
    /// `Some` makes the item a check item, ticked when `true`.
    pub checked: Option<bool>,
}

/// One row of a [`TrayMenu`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayMenuEntry {
    Item(TrayMenuItem),
    Separator,
    Submenu { title: String, enabled: bool, menu: TrayMenu },
}

/// A list of items, separators and submenus.
///
/// # Examples
///
/// ```
/// use aimer_tray::{TrayItemId, TrayMenu};
///
/// const SHOW: TrayItemId = TrayItemId(1);
/// const QUIT: TrayItemId = TrayItemId(2);
///
/// let menu = TrayMenu::new()
///     .item(SHOW, "Show window")
///     .separator()
///     .item(QUIT, "Quit");
/// assert_eq!(menu.entries().len(), 3);
/// assert!(menu.find(QUIT).is_some());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayMenu {
    entries: Vec<TrayMenuEntry>,
}

impl TrayMenu {
    /// Creates an empty menu.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an enabled item.
    #[inline]
    pub fn item(mut self, id: TrayItemId, title: impl Into<String>) -> Self {
        self.entries.push(TrayMenuEntry::Item(TrayMenuItem {
            id,
            title: title.into(),
            enabled: true,
            checked: None,
        }));
        self
    }

    /// Adds an item that is shown but cannot be chosen.
    #[inline]
    pub fn disabled_item(mut self, id: TrayItemId, title: impl Into<String>) -> Self {
        self.entries.push(TrayMenuEntry::Item(TrayMenuItem {
            id,
            title: title.into(),
            enabled: false,
            checked: None,
        }));
        self
    }

    /// Adds an item with a check mark that is ticked when `checked` is `true`.
    #[inline]
    pub fn check_item(mut self, id: TrayItemId, title: impl Into<String>, checked: bool) -> Self {
        self.entries.push(TrayMenuEntry::Item(TrayMenuItem {
            id,
            title: title.into(),
            enabled: true,
            checked: Some(checked),
        }));
        self
    }

    /// Adds a line between groups of items.
    #[inline]
    pub fn separator(mut self) -> Self {
        self.entries.push(TrayMenuEntry::Separator);
        self
    }

    /// Adds a submenu.
    #[inline]
    pub fn submenu(mut self, title: impl Into<String>, menu: TrayMenu) -> Self {
        self.entries.push(TrayMenuEntry::Submenu { title: title.into(), enabled: true, menu });
        self
    }

    /// The rows, top to bottom.
    #[inline]
    pub fn entries(&self) -> &[TrayMenuEntry] {
        &self.entries
    }

    /// Whether the menu has no rows.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The item named `id`, searching submenus too.
    pub fn find(&self, id: TrayItemId) -> Option<&TrayMenuItem> {
        self.entries.iter().find_map(|entry| match entry {
            TrayMenuEntry::Item(item) if item.id == id => Some(item),
            TrayMenuEntry::Submenu { menu, .. } => menu.find(id),
            _ => None,
        })
    }

    /// Calls `visit` with every item, submenus included, in display order.
    pub fn for_each_item(&self, visit: &mut impl FnMut(&TrayMenuItem)) {
        for entry in &self.entries {
            match entry {
                TrayMenuEntry::Item(item) => visit(item),
                TrayMenuEntry::Submenu { menu, .. } => menu.for_each_item(visit),
                TrayMenuEntry::Separator => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_searches_submenus() {
        let inner = TrayMenu::new().item(TrayItemId(7), "Deep");
        let menu = TrayMenu::new().item(TrayItemId(1), "Top").submenu("More", inner);
        assert_eq!(menu.find(TrayItemId(7)).unwrap().title, "Deep");
        assert!(menu.find(TrayItemId(9)).is_none());
    }

    #[test]
    fn builders_set_state() {
        let menu = TrayMenu::new()
            .disabled_item(TrayItemId(1), "Off")
            .check_item(TrayItemId(2), "Ticked", true);
        assert!(!menu.find(TrayItemId(1)).unwrap().enabled);
        assert_eq!(menu.find(TrayItemId(2)).unwrap().checked, Some(true));
    }

    #[test]
    fn for_each_visits_in_display_order() {
        let inner = TrayMenu::new().item(TrayItemId(2), "b");
        let menu = TrayMenu::new().item(TrayItemId(1), "a").separator().submenu("s", inner).item(TrayItemId(3), "c");
        let mut seen = Vec::new();
        menu.for_each_item(&mut |item| seen.push(item.id.0));
        assert_eq!(seen, [1, 2, 3]);
    }
}
