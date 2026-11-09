use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::*;
use crate::{ChordMods, Key};

fn chord(text: &str) -> KeyChord {
    KeyChord::parse_with_primary(text, ChordMods::CTRL).expect(text)
}

/// Records what the registry asks of the operating system.
#[derive(Default)]
struct Fake {
    log: Rc<RefCell<Vec<String>>>,
    refuse: Rc<Cell<bool>>,
}

impl Backend for Fake {
    fn register(&mut self, id: GlobalShortcutId, chord: &KeyChord) -> Result<(), GlobalShortcutError> {
        if self.refuse.get() {
            return Err(GlobalShortcutError::Taken);
        }
        self.log.borrow_mut().push(format!("+{} {chord}", id.as_raw()));
        Ok(())
    }

    fn unregister(&mut self, id: GlobalShortcutId) {
        self.log.borrow_mut().push(format!("-{}", id.as_raw()));
    }
}

type Log = Rc<RefCell<Vec<String>>>;

fn registry() -> (Registry, Log, Rc<Cell<bool>>) {
    let fake = Fake::default();
    let (log, refuse) = (Rc::clone(&fake.log), Rc::clone(&fake.refuse));
    (Registry::new(Box::new(fake)), log, refuse)
}

fn counter() -> (Rc<Cell<u32>>, impl Fn() + 'static) {
    let count = Rc::new(Cell::new(0));
    let seen = Rc::clone(&count);
    (count, move || seen.set(seen.get() + 1))
}

#[test]
fn a_registered_shortcut_runs_when_its_id_fires() {
    let (mut registry, log, _) = registry();
    let (count, action) = counter();

    let id = registry.register(chord("Ctrl+Shift+K"), Rc::new(action)).unwrap();

    assert_eq!(&*log.borrow(), &[format!("+{} Ctrl+Shift+K", id.as_raw())]);
    assert!(registry.fire(id));
    assert!(registry.fire(id));
    assert_eq!(count.get(), 2);
}

#[test]
fn a_chord_can_be_registered_only_once_by_this_application() {
    let (mut registry, log, _) = registry();
    registry.register(chord("Ctrl+K"), Rc::new(|| {})).unwrap();

    let again = registry.register(chord("ctrl+k"), Rc::new(|| {}));

    assert_eq!(again, Err(GlobalShortcutError::AlreadyRegistered));
    assert_eq!(log.borrow().len(), 1, "the operating system was not asked twice");
}

#[test]
fn a_refusal_from_the_operating_system_registers_nothing() {
    let (mut registry, _, refuse) = registry();
    refuse.set(true);
    let (count, action) = counter();

    assert_eq!(registry.register(chord("Ctrl+K"), Rc::new(action)), Err(GlobalShortcutError::Taken));

    refuse.set(false);
    let id = registry.register(chord("Ctrl+K"), Rc::new(|| {})).expect("the chord is free again");
    registry.fire(id);
    assert_eq!(count.get(), 0);
}

#[test]
fn unregistering_releases_the_chord_and_silences_the_id() {
    let (mut registry, log, _) = registry();
    let (count, action) = counter();
    let id = registry.register(chord("Ctrl+K"), Rc::new(action)).unwrap();

    registry.unregister(id);

    assert_eq!(log.borrow().last().map(String::as_str), Some(format!("-{}", id.as_raw()).as_str()));
    assert!(!registry.fire(id));
    assert_eq!(count.get(), 0);
    assert!(registry.register(chord("Ctrl+K"), Rc::new(|| {})).is_ok());
}

#[test]
fn unregistering_twice_or_an_unknown_id_is_harmless() {
    let (mut registry, log, _) = registry();
    let id = registry.register(chord("Ctrl+K"), Rc::new(|| {})).unwrap();
    registry.unregister(id);
    registry.unregister(id);
    registry.unregister(GlobalShortcutId::from_raw(u32::MAX));
    assert_eq!(log.borrow().len(), 2);
}

#[test]
fn a_chord_the_platform_cannot_express_is_rejected_before_asking_it() {
    let (mut registry, log, _) = registry();
    let bare = KeyChord::new(ChordMods::NONE, Key::Char('k'));
    assert_eq!(registry.register(bare, Rc::new(|| {})), Err(GlobalShortcutError::NeedsModifier));
    assert!(log.borrow().is_empty());
}

#[test]
fn an_action_may_unregister_its_own_shortcut() {
    let (registry, _, _) = registry();
    let registry = Rc::new(RefCell::new(registry));
    let slot = Rc::new(Cell::new(None));
    let (inner, cell) = (Rc::clone(&registry), Rc::clone(&slot));
    let id = registry
        .borrow_mut()
        .register(chord("Ctrl+K"), Rc::new(move || {
            if let Some(id) = cell.get() {
                inner.borrow_mut().unregister(id);
            }
        }))
        .unwrap();
    slot.set(Some(id));

    let action = registry.borrow().action(id).expect("registered");
    action();

    assert!(registry.borrow().action(id).is_none());
}
