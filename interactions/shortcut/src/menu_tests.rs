use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::{ChordMods, Key, scopes_alive};

fn counter() -> (Rc<Cell<u32>>, impl Fn() + 'static) {
    let count = Rc::new(Cell::new(0));
    let seen = Rc::clone(&count);
    (count, move || seen.set(seen.get() + 1))
}

fn ctrl(key: char) -> KeyChord {
    KeyChord::new(ChordMods::CTRL, Key::Char(key))
}

fn item_ids(menu: &Menu) -> Vec<MenuItemId> {
    menu.entries()
        .iter()
        .filter_map(|entry| match entry {
            MenuEntry::Item(item) => Some(item.id()),
            MenuEntry::Separator => None,
        })
        .collect()
}

#[test]
fn a_menu_keeps_its_entries_in_order() {
    let menu = Menu::new("File")
        .item_with("Save", Some(ctrl('s')), || {})
        .separator()
        .item_with("Quit", None, || {});

    assert_eq!(menu.title(), "File");
    assert_eq!(menu.entries().len(), 3);
    assert!(matches!(menu.entries()[1], MenuEntry::Separator));
    let MenuEntry::Item(save) = &menu.entries()[0] else { panic!("first entry is an item") };
    assert_eq!((save.title(), save.chord()), ("Save", Some(ctrl('s'))));
}

#[test]
fn every_item_gets_its_own_id() {
    let menu = Menu::new("File").item_with("A", None, || {}).item_with("B", None, || {});
    let ids = item_ids(&menu);
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn installing_a_menu_makes_items_activatable_by_id_and_by_chord() {
    let (saved, save) = counter();
    let (opened, open) = counter();
    let menu = Menu::new("File")
        .item_with("Save", Some(ctrl('s')), save)
        .item_with("Open", Some(ctrl('o')), open);
    let ids = item_ids(&menu);
    set_app_menu(AppMenu::new().menu(menu));

    assert!(activate_item(ids[1]));
    assert_eq!(opened.get(), 1);
    assert_eq!(activate_chord(&ctrl('s'), false), Activation::Ran);
    assert_eq!(saved.get(), 1);
    assert_eq!(activate_chord(&ctrl('q'), false), Activation::Unbound);
    assert!(!activate_item(MenuItemId::from_raw(u32::MAX)));
}

#[test]
fn a_chord_repeat_is_swallowed_by_the_menu() {
    let (count, action) = counter();
    set_app_menu(AppMenu::new().menu(Menu::new("File").item_with("Save", Some(ctrl('s')), action)));

    assert_eq!(activate_chord(&ctrl('s'), true), Activation::Held);
    assert_eq!(count.get(), 0);
}

#[test]
fn an_installed_menu_counts_as_a_live_shortcut_source() {
    assert!(!scopes_alive());
    set_app_menu(AppMenu::new());
    assert!(scopes_alive());
    clear_app_menu();
    assert!(!scopes_alive());
}

#[test]
fn an_action_may_replace_the_menu_that_ran_it() {
    let menu = Menu::new("File").item_with("Swap", Some(ctrl('w')), || {
        set_app_menu(AppMenu::new());
    });
    set_app_menu(AppMenu::new().menu(menu));

    assert_eq!(activate_chord(&ctrl('w'), false), Activation::Ran);
    assert_eq!(activate_chord(&ctrl('w'), false), Activation::Unbound);
}

#[test]
fn item_str_parses_the_chord_for_the_platform() {
    let menu = Menu::new("File").item("Save", "Ctrl+S", || {});
    let MenuEntry::Item(save) = &menu.entries()[0] else { panic!("an item") };
    assert_eq!(save.chord(), Some(KeyChord::parse("Ctrl+S").unwrap()));
}
