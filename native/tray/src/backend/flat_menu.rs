//! A [`TrayMenu`] with a number on every row.
//!
//! The `dbusmenu` protocol names each row by an `i32` and asks for them by
//! number, so the tree is flattened once per menu change: rows get ids in
//! display order starting at 1, and id 0 is the invisible root that holds the
//! top-level rows.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::{TrayItemId, TrayMenu, TrayMenuEntry};

pub(super) enum Kind {
    Item { id: TrayItemId, enabled: bool, checked: Option<bool> },
    Separator,
    Submenu { enabled: bool },
}

pub(super) struct Node {
    pub kind: Kind,
    /// The label with `_` doubled, because `dbusmenu` reads a single `_` as the
    /// marker of the mnemonic letter.
    pub label: String,
    pub children: Vec<i32>,
}

#[derive(Default)]
pub(super) struct FlatMenu {
    nodes: Vec<Node>,
    top: Vec<i32>,
}

impl FlatMenu {
    pub fn from_menu(menu: &TrayMenu) -> Self {
        let mut flat = Self::default();
        flat.top = flat.push_all(menu);
        flat
    }

    fn push_all(&mut self, menu: &TrayMenu) -> Vec<i32> {
        let mut ids = Vec::with_capacity(menu.entries().len());
        for entry in menu.entries() {
            let (kind, label) = match entry {
                TrayMenuEntry::Item(item) => (
                    Kind::Item { id: item.id, enabled: item.enabled, checked: item.checked },
                    escape(&item.title),
                ),
                TrayMenuEntry::Separator => (Kind::Separator, String::new()),
                TrayMenuEntry::Submenu { title, enabled, .. } => (Kind::Submenu { enabled: *enabled }, escape(title)),
            };
            // Reserve the id before the children so ids follow display order.
            let id = self.nodes.len() as i32 + 1;
            self.nodes.push(Node { kind, label, children: Vec::new() });
            if let TrayMenuEntry::Submenu { menu, .. } = entry {
                let children = self.push_all(menu);
                self.nodes[id as usize - 1].children = children;
            }
            ids.push(id);
        }
        ids
    }

    /// The row numbered `id`; `None` for 0 (the root) and unknown ids.
    pub fn node(&self, id: i32) -> Option<&Node> {
        usize::try_from(id).ok()?.checked_sub(1).and_then(|index| self.nodes.get(index))
    }

    /// The ids under `id`, with 0 standing for the root.
    pub fn children_of(&self, id: i32) -> Option<&[i32]> {
        if id == 0 { Some(&self.top) } else { self.node(id).map(|node| node.children.as_slice()) }
    }

    /// The application's id for the row, if the row is an item.
    pub fn item_id(&self, id: i32) -> Option<TrayItemId> {
        match self.node(id)?.kind {
            Kind::Item { id, .. } => Some(id),
            _ => None,
        }
    }
}

fn escape(label: &str) -> String {
    label.replace('_', "__")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FlatMenu {
        let inner = TrayMenu::new().item(TrayItemId(20), "Deep");
        FlatMenu::from_menu(
            &TrayMenu::new()
                .item(TrayItemId(10), "First")
                .submenu("More", inner)
                .separator()
                .item(TrayItemId(30), "snake_case"),
        )
    }

    #[test]
    fn ids_follow_display_order() {
        let flat = sample();
        assert_eq!(flat.children_of(0).unwrap(), [1, 2, 4, 5]);
        assert_eq!(flat.children_of(2).unwrap(), [3]);
        assert_eq!(flat.item_id(3), Some(TrayItemId(20)));
    }

    #[test]
    fn only_items_map_back() {
        let flat = sample();
        assert_eq!(flat.item_id(2), None);
        assert_eq!(flat.item_id(4), None);
        assert_eq!(flat.item_id(0), None);
        assert_eq!(flat.item_id(99), None);
        assert_eq!(flat.item_id(-1), None);
    }

    #[test]
    fn underscores_are_escaped() {
        assert_eq!(sample().node(5).unwrap().label, "snake__case");
    }

    #[test]
    fn unknown_parent_has_no_children() {
        assert!(sample().children_of(42).is_none());
        assert!(FlatMenu::default().children_of(0).unwrap().is_empty());
    }
}
