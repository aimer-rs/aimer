//! Keyboard shortcuts: local, menu and global.
//!
//! One page shows the three kinds side by side, and logs what fired:
//!
//! - **Local.** A [`Shortcuts`] widget around the page binds `Cmd/Ctrl+K`,
//!   `Cmd/Ctrl+S` and `Cmd/Ctrl+Shift+L`. Focus an editor box and the scope
//!   around *it* is asked first: editor B binds its own `Cmd/Ctrl+S`, which
//!   wins over the page's while B has focus and falls through to the page's
//!   when A does.
//! - **Menu.** The `Shortcuts` menu is installed by [`install_app_shortcuts`]
//!   before the app starts, so macOS shows a native menu with the shortcut
//!   beside each item. Everywhere else the same chords run as window-wide
//!   shortcuts.
//! - **Global.** `Cmd/Ctrl+Shift+Alt+G` is claimed system-wide and fires while
//!   another application has focus. Where the platform has no such facility
//!   the page says why.
//!
//! The log lives outside the widget on purpose: the menu and the global
//! shortcut belong to the application, not to this page, so their actions
//! cannot hold the page's state. They write to the log and nudge whichever
//! page is showing it.

use std::cell::{Cell, RefCell};

use aimer::input::button::Button;
use aimer::shortcut::KeyChord;
use aimer::shortcut::global::GlobalShortcut;
use aimer::shortcut::menu::{AppMenu, Menu, set_app_menu};
use aimer::style::*;
use aimer::*;

/// How many log lines the page keeps.
const LOG_LINES: usize = 8;

/// The system-wide chord.
const GLOBAL_CHORD: &str = "CmdOrCtrl+Shift+Alt+G";

aimer_tls::thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static GLOBAL_STATUS: RefCell<String> = const { RefCell::new(String::new()) };
    /// The page that is showing the log, if one is.
    static PAGE: Cell<Option<StateUpdater<ShortcutsExampleState>>> = const { Cell::new(None) };
}

/// Appends `entry` to the log and redraws the page showing it.
fn record(entry: &str) {
    LOG.with_borrow_mut(|log| {
        if log.len() == LOG_LINES {
            log.remove(0);
        }
        log.push(entry.to_owned());
    });
    // A page that has gone away rejects the update, which is the right outcome.
    if let Some(page) = PAGE.get() {
        let _ = page.try_set_state(|_| {});
    }
}

/// Empties the log.
fn clear_log() {
    LOG.with_borrow_mut(Vec::clear);
    if let Some(page) = PAGE.get() {
        let _ = page.try_set_state(|_| {});
    }
}

/// How a chord is written for the person looking at it: `⇧⌘K` on macOS,
/// `Ctrl+Shift+K` elsewhere.
fn label(chord: &str) -> String {
    match KeyChord::parse(chord) {
        Ok(chord) if cfg!(target_os = "macos") => chord.label_macos(),
        Ok(chord) => chord.to_string(),
        Err(_) => chord.to_owned(),
    }
}

/// Installs what belongs to the whole application: the `Shortcuts` menu and
/// the global shortcut.
///
/// Call it on the UI thread before the app runs, so the native menu bar is
/// built from the menu.
pub fn install_app_shortcuts() {
    set_app_menu(AppMenu::new().menu(
        Menu::new("Shortcuts")
            .item("Ping from the menu", "CmdOrCtrl+Shift+M", || record("menu: Ping"))
            .item("Say hello", "CmdOrCtrl+Shift+U", || record("menu: Hello"))
            .separator()
            .item("Clear the log", "CmdOrCtrl+Shift+L", clear_log),
    ));

    let status = match GlobalShortcut::register(
        KeyChord::parse(GLOBAL_CHORD).expect("the global chord is a valid literal"),
        || record("global: summoned"),
    ) {
        Ok(held) => {
            // The shortcut lasts as long as the app does.
            std::mem::forget(held);
            format!("Registered: {}", label(GLOBAL_CHORD))
        }
        Err(why) => format!("Unavailable: {why}"),
    };
    GLOBAL_STATUS.with_borrow_mut(|slot| *slot = status);
}

/// The shortcuts page.
#[derive(StatefulWidget)]
pub struct ShortcutsExample;

impl ShortcutsExample {
    /// Creates the page.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ShortcutsExample {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// The retained side of the page: the two editors' focus nodes, and which of
/// them owns the keyboard.
pub struct ShortcutsExampleState {
    editor_a: FocusNode,
    editor_b: FocusNode,
    focused: Option<&'static str>,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for ShortcutsExample {
    type State = ShortcutsExampleState;

    fn create_state(self) -> ShortcutsExampleState {
        ShortcutsExampleState {
            editor_a: FocusNode::new(),
            editor_b: FocusNode::new(),
            focused: None,
            updater: StateUpdater::empty(),
        }
    }
}

impl ShortcutsExampleState {
    /// One editor box: focusable, and lit while it owns the keyboard.
    fn editor(&self, name: &'static str, node: &FocusNode) -> AnyWidget {
        let app_theme = crate::theme::app_theme();
        let focused = self.focused == Some(name);
        let updater = self.updater;
        Focusable::new()
            .node(node.clone())
            .on_focus_change(move |gained| {
                // The box that lost focus reports first, so recording `None`
                // here can never erase the one that gained it.
                updater.set_state(move |state| state.focused = gained.then_some(name));
            })
            .box_child(
                Container::new()
                    .width(Dimension::Px(220.0))
                    .height(Dimension::Px(110.0))
                    .box_decoration(
                        BoxDecoration::new()
                            .background_color(app_theme.surface_color)
                            .border_radius((12, 12, 12, 12))
                            .border(BoxBorder::all(
                                BorderSlice::new()
                                    .style(BorderStyle::Solid)
                                    .stroke(Stroke::Px(3.0))
                                    .color(if focused {
                                        app_theme.primary_color
                                    } else {
                                        Color::Transparent
                                    }),
                            )),
                    )
                    .box_child(
                        Text::new(name)
                            .text_align(TextAlign::MidCenter)
                            .text_style(
                                TextStyle::new().font_size(18).color(app_theme.on_surface_color),
                            ),
                    ),
            )
    }

    /// A line of text in the page's body style.
    fn line(text: impl Into<String>, color: Color, size: u32) -> AnyWidget {
        Text::new(text.into())
            .text_style(TextStyle::new().font_size(size).color(color))
            .boxed()
    }

    /// A heading above one group of the page.
    fn heading(text: &'static str, app_theme: &ThemeData) -> AnyWidget {
        Text::new(text)
            .text_style(
                TextStyle::new()
                    .font_size(18)
                    .font_weight(FontWeight::Bold)
                    .color(app_theme.on_background_color),
            )
            .boxed()
    }

    /// A button that clears the log.
    fn clear_button(app_theme: ThemeData) -> AnyWidget {
        Container::new()
            .width(Dimension::Px(140.0))
            .height(Dimension::Px(40.0))
            .box_child(
                Button::new()
                    .on_press(clear_log)
                    .decoration(
                        BoxDecoration::new()
                            .background_color(app_theme.primary_color)
                            .border_radius((8, 8, 8, 8)),
                    )
                    .child(
                        Text::new("Clear the log")
                            .text_align(TextAlign::MidCenter)
                            .text_style(
                                TextStyle::new().font_size(14).color(app_theme.on_primary_color),
                            ),
                    ),
            )
    }
}

impl State<ShortcutsExample> for ShortcutsExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
        PAGE.set(Some(updater));
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let muted = crate::theme::muted_text(&app_theme);
        let body = app_theme.on_background_color;
        let focused = self.focused.unwrap_or("nothing");

        let log: Vec<AnyWidget> = LOG.with_borrow(|log| {
            if log.is_empty() {
                vec![Self::line("Nothing yet. Press a shortcut.", muted, 14)]
            } else {
                log.iter().rev().map(|entry| Self::line(entry.as_str(), body, 14)).collect()
            }
        });
        let global_status = GLOBAL_STATUS.with_borrow(|status| status.clone());

        let page = Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Start)
                    .gaps(LayoutSpacing::new().bottom(12))
                    .children(vec![
                        Self::heading("Local: this page", &app_theme),
                        Self::line(format!("{}  command palette", label("CmdOrCtrl+K")), body, 15),
                        Self::line(format!("{}  save (the page's)", label("CmdOrCtrl+S")), body, 15),
                        Self::line(
                            format!("{}  clear the log", label("CmdOrCtrl+Shift+L")),
                            body,
                            15,
                        ),
                        Self::line(
                            "Focus an editor. Editor B overrides save with its own; A falls back to the page's.",
                            muted,
                            14,
                        ),
                        Row::new()
                            .gaps(LayoutSpacing::new().right(48))
                            .children(vec![
                                Row::new()
                                    .gaps(LayoutSpacing::new().right(24))
                                    .children(vec![
                                        self.editor("Editor A", &self.editor_a),
                                        // The scope sits around B alone, so it is
                                        // asked only while B has focus.
                                        Shortcuts::new()
                                            .bind_str("CmdOrCtrl+S", || {
                                                record("local: save in editor B")
                                            })
                                            .box_child(self.editor("Editor B", &self.editor_b)),
                                    ])
                                    .boxed(),
                                // Beside the editors, so a press and its effect
                                // are in view together.
                                Column::new()
                                    .horizontal_alignment(BoxAlignment::Start)
                                    .gaps(LayoutSpacing::new().bottom(4))
                                    .children(
                                        std::iter::once(Self::heading("Log", &app_theme))
                                            .chain(log)
                                            .collect::<Vec<_>>(),
                                    )
                                    .boxed(),
                            ])
                            .boxed(),
                        Self::line(format!("Focused: {focused}"), body, 15),
                        Self::heading("Menu", &app_theme),
                        Self::line(
                            format!(
                                "Shortcuts > Ping ({}), Say hello ({}), Clear ({})",
                                label("CmdOrCtrl+Shift+M"),
                                label("CmdOrCtrl+Shift+U"),
                                label("CmdOrCtrl+Shift+L"),
                            ),
                            body,
                            15,
                        ),
                        Self::heading("Global", &app_theme),
                        Self::line(global_status, body, 15),
                        Self::line("Fires while another application has focus.", muted, 14),
                        Self::clear_button(app_theme),
                    ]),
            );

        // Window-wide, so these work wherever focus is on this page. The editor
        // scopes above are asked first, which is why B's save wins over this one.
        Shortcuts::new()
            .window(true)
            .bind_str("CmdOrCtrl+K", || record("local: command palette"))
            .bind_str("CmdOrCtrl+S", || record("local: save (page)"))
            .bind_str("CmdOrCtrl+Shift+L", clear_log)
            .child(page)
    }
}

#[cfg(test)]
mod tests {
    use aimer::events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
    use aimer::quiver::aimer_app::{AimerApp, AimerNativePlatformEvent};

    use super::*;

    fn log() -> Vec<String> {
        LOG.with_borrow(Clone::clone)
    }

    fn press(app: &mut aimer::quiver::aimer_app::HeadlessAimerApp<impl Widget + 'static>, key: &str, shift: bool) {
        // `CmdOrCtrl` is Command on macOS and Control elsewhere.
        let primary = cfg!(target_os = "macos");
        let modifiers = Modifiers { ctrl: !primary, meta: primary, shift, ..Modifiers::default() };
        app.send_user_event(AimerNativePlatformEvent::NativeInput {
            event: ElementEvent::KeyInput {
                key: NamedKey::Other(key.into()),
                action: KeyAction::Pressed,
                modifiers: modifiers.clone(),
            },
            modifiers,
        });
        app.render_frame();
    }

    #[test]
    fn the_log_keeps_only_the_latest_lines_in_order() {
        for n in 0..LOG_LINES + 3 {
            record(&format!("entry {n}"));
        }
        let log = log();
        assert_eq!(log.len(), LOG_LINES);
        assert_eq!(log.first().map(String::as_str), Some("entry 3"));
        assert_eq!(log.last().map(String::as_str), Some(format!("entry {}", LOG_LINES + 2).as_str()));
    }

    #[test]
    fn labels_follow_the_platform_and_unparsable_text_is_left_alone() {
        assert!(label("CmdOrCtrl+K").ends_with('K'));
        assert_eq!(label("nonsense+"), "nonsense+");
    }

    #[test]
    fn page_shortcuts_run_and_the_clear_chord_empties_the_log() {
        LOG.with_borrow_mut(Vec::clear);
        let mut app = AimerApp::start_headless(crate::theme::provide(ShortcutsExample::new().boxed()));
        app.render_frame();

        press(&mut app, "k", false);
        press(&mut app, "s", false);
        assert_eq!(log(), ["local: command palette", "local: save (page)"]);

        press(&mut app, "l", true);
        assert!(log().is_empty(), "Ctrl+Shift+L clears the log");
    }
}
