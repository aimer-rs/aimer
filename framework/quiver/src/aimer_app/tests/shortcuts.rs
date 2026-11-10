use std::cell::Cell;
use std::rc::Rc;

use super::*;
use aimer_events::element::{KeyAction, Modifiers, NamedKey};
use aimer_shortcut::global::GlobalShortcutId;
use aimer_shortcut::menu::{AppMenu, Menu, MenuEntry, set_app_menu};
use aimer_shortcut::{ChordMods, Key, KeyChord};
use aimer_widget::Shortcuts;

struct Blank;
impl Widget for Blank {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        BlankElement.boxed()
    }
}
struct BlankElement;
impl Drawable for BlankElement {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for BlankElement {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some((Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }))
    }
}
impl Rebuildable for BlankElement {}
impl VisitorElement for BlankElement {
    fn debug_name(&self) -> &'static str {
        "BlankElement"
    }
}
impl EventElement for BlankElement {}

fn ctrl_k() -> KeyChord {
    KeyChord::new(ChordMods::CTRL, Key::Char('k'))
}

fn press_ctrl<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, letter: &str) {
    let modifiers = Modifiers { ctrl: true, ..Modifiers::default() };
    app.send_user_event(AimerNativePlatformEvent::NativeInput {
        event: ElementEvent::KeyInput {
            key: NamedKey::Other(letter.into()),
            action: KeyAction::Pressed,
            modifiers: modifiers.clone(),
        },
        modifiers,
    });
}

#[test]
fn a_window_shortcut_runs_from_a_key_press_that_reaches_the_app() {
    let hits = Rc::new(Cell::new(0));
    let seen = Rc::clone(&hits);
    let mut app = AimerApp::start_headless(
        Shortcuts::new()
            .window(true)
            .bind(ctrl_k(), move || seen.set(seen.get() + 1))
            .child(Blank),
    );
    app.render_frame();

    press_ctrl(&mut app, "k");
    press_ctrl(&mut app, "j");

    assert_eq!(hits.get(), 1, "only the bound chord runs the action");
}

#[test]
fn a_menu_item_chosen_in_the_native_menu_runs_in_the_app() {
    let hits = Rc::new(Cell::new(0));
    let seen = Rc::clone(&hits);
    let menu = Menu::new("File").item_with("Save", Some(ctrl_k()), move || seen.set(seen.get() + 1));
    let MenuEntry::Item(item) = &menu.entries()[0] else { panic!("an item") };
    let id = item.id();
    set_app_menu(AppMenu::new().menu(menu));
    let mut app = AimerApp::start_headless(Blank);
    app.render_frame();

    app.send_user_event(AimerNativePlatformEvent::MenuItem(id));
    assert_eq!(hits.get(), 1);

    // With no native menu bar the same item answers its chord.
    press_ctrl(&mut app, "k");
    assert_eq!(hits.get(), 2);
}

#[test]
fn a_global_shortcut_that_nobody_holds_is_ignored() {
    let mut app = AimerApp::start_headless(Blank);
    app.render_frame();

    app.send_user_event(AimerNativePlatformEvent::GlobalShortcut(GlobalShortcutId::from_raw(7)));
}

#[test]
fn a_tray_event_runs_its_handler_in_the_app_until_it_is_unsubscribed() {
    use aimer_platform::tray::{self, TrayButton, TrayEvent, TrayItemId};

    let seen = Rc::new(Cell::new(None));
    let sink = Rc::clone(&seen);
    let subscription = tray::subscribe(move |event| sink.set(Some(event)));
    let id = subscription.id();
    let mut app = AimerApp::start_headless(Blank);
    app.render_frame();

    app.send_user_event(AimerNativePlatformEvent::Tray { id, event: TrayEvent::MenuItem(TrayItemId(3)) });
    assert_eq!(seen.get(), Some(TrayEvent::MenuItem(TrayItemId(3))));

    drop(subscription);
    seen.set(None);
    app.send_user_event(AimerNativePlatformEvent::Tray { id, event: TrayEvent::Click(TrayButton::Left) });
    assert_eq!(seen.get(), None, "a tray nobody holds is ignored");
}
