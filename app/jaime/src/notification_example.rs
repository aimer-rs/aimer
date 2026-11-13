//! System notifications whose answers arrive in the event loop.
//!
//! The buttons exercise what [`aimer::notification`] offers: ask for
//! permission, show a notification at once or after five seconds, close them,
//! and ask for a push token. Clicking a notification, choosing its **Snooze**
//! button, swiping it away and every permission answer are written to the log.
//!
//! The platform reports from threads of its own, but
//! [`aimer::notification::install`] hands every event to the event loop first,
//! so the handler below runs on the UI thread and writes to the log and the page
//! like any other callback.
//!
//! macOS shows notifications only for an application run from an app bundle; a
//! bare executable is told so in the status line. Android shows them but does
//! not report clicks; desktop Windows shows a balloon without buttons.
//!
//! What each event means is decided by [`describe`], a plain function, so it is
//! tested without a notification service.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use aimer::input::button::Button;
use aimer::notification::{
    self, AppNotifier, Notification, NotificationAction, NotificationActionId, NotificationEvent,
    NotificationId, Permission,
};
use aimer::style::*;
use aimer::*;

/// How many log lines the page keeps.
const LOG_LINES: usize = 8;

/// How long the delayed notification waits.
const DELAY: Duration = Duration::from_secs(5);

const NOW: NotificationId = NotificationId(1);
const LATER: NotificationId = NotificationId(2);
const SNOOZE: NotificationActionId = NotificationActionId(1);

aimer_tls::thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static STATUS: RefCell<String> = const { RefCell::new(String::new()) };
    static PERMISSION: Cell<Option<Permission>> = const { Cell::new(None) };
    /// The connection to the notification service, once there is one.
    static NOTIFIER: RefCell<Option<AppNotifier>> = const { RefCell::new(None) };
    /// The page that is showing the log, if one is.
    static PAGE: Cell<Option<StateUpdater<NotificationExampleState>>> = const { Cell::new(None) };
}

/// The log line for `event`.
fn describe(event: &NotificationEvent) -> String {
    match event {
        NotificationEvent::Clicked(id) => format!("clicked: #{}", id.0),
        NotificationEvent::Action { notification, action } => {
            let name = if *action == SNOOZE { "Snooze".to_owned() } else { format!("#{}", action.0) };
            format!("button {name} of #{}", notification.0)
        }
        NotificationEvent::Dismissed(id) => format!("dismissed: #{}", id.0),
        NotificationEvent::Permission(permission) => format!("permission: {}", permission_name(*permission)),
        NotificationEvent::PushToken(token) => format!("push token: {}", abbreviate(token)),
        NotificationEvent::PushReceived(message) => {
            format!("push: {}", message.title.as_deref().or(message.body.as_deref()).unwrap_or("(no text)"))
        }
        NotificationEvent::PushFailed(why) => format!("push failed: {why}"),
    }
}

fn permission_name(permission: Permission) -> &'static str {
    match permission {
        Permission::NotDetermined => "not asked yet",
        Permission::Granted => "granted",
        Permission::Denied => "denied",
    }
}

/// A long token cut to a line's worth, on a character boundary.
fn abbreviate(token: &str) -> String {
    const KEEP: usize = 16;
    match token.char_indices().nth(KEEP) {
        Some((end, _)) => format!("{}…", &token[..end]),
        None => token.to_owned(),
    }
}

/// The notification the **Show now** button sends.
fn immediate() -> Notification {
    Notification::new(NOW)
        .title("Jaime")
        .body("Hello from the notification example.")
        .action(NotificationAction::new(SNOOZE, "Snooze"))
}

/// The notification the **Show in 5 seconds** button sends.
fn delayed() -> Notification {
    Notification::new(LATER).title("Aimer").subtitle("Scheduled").body("Five seconds have passed.").delay(DELAY)
}

/// Appends `entry` to the log and redraws the page showing it.
fn record(entry: &str) {
    LOG.with_borrow_mut(|log| {
        if log.len() == LOG_LINES {
            log.remove(0);
        }
        log.push(entry.to_owned());
    });
    refresh_page();
}

/// Redraws the page, if one is showing. A page that has gone away rejects the
/// update, which is the right outcome.
fn refresh_page() {
    if let Some(page) = PAGE.get() {
        let _ = page.try_set_state(|_| {});
    }
}

fn set_status(status: impl Into<String>) {
    STATUS.with_borrow_mut(|slot| *slot = status.into());
    refresh_page();
}

fn on_event(event: NotificationEvent) {
    if let NotificationEvent::Permission(permission) = &event {
        PERMISSION.set(Some(*permission));
    }
    record(&describe(&event));
}

/// Runs `action` with the notification service, connecting first if need be.
/// A failure is shown in the status line.
fn with_notifier(action: impl FnOnce(&AppNotifier) -> Result<(), notification::NotificationError>) {
    if NOTIFIER.with_borrow(Option::is_none) {
        match notification::install(on_event) {
            Ok(installed) => {
                NOTIFIER.with_borrow_mut(|slot| *slot = Some(installed));
                set_status("Connected to the notification service.");
            }
            Err(why) => {
                set_status(format!("Unavailable: {why}"));
                return;
            }
        }
    }
    let result = NOTIFIER.with_borrow(|slot| slot.as_ref().map(action));
    if let Some(Err(why)) = result {
        set_status(format!("Failed: {why}"));
    }
}

fn ask_permission() {
    with_notifier(|notifier| {
        notifier.request_permission(Box::new(|_| {}));
        Ok(())
    });
}

fn check_permission() {
    with_notifier(|notifier| {
        notifier.permission(Box::new(|_| {}));
        Ok(())
    });
}

fn show_now() {
    with_notifier(|notifier| notifier.show(&immediate()));
}

fn show_later() {
    with_notifier(|notifier| notifier.show(&delayed()));
}

fn close_all() {
    with_notifier(|notifier| {
        notifier.close(NOW)?;
        notifier.close(LATER)
    });
}

fn register_push() {
    with_notifier(|notifier| notifier.register_push());
}

/// The notification page.
#[derive(StatefulWidget)]
pub struct NotificationExample;

impl NotificationExample {
    /// Creates the page.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Default for NotificationExample {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// The retained side of the page. The log and the connection outlive the page,
/// so all it keeps is a way to ask for a redraw.
pub struct NotificationExampleState {
    /// Kept for the helper methods that call back into the state.
    updater: StateUpdater<Self>,
}

impl NotificationExampleState {
    /// A line of text in the page's body style.
    fn line(text: impl Into<String>, color: Color, size: u32) -> AnyWidget {
        Text::new(text.into()).text_style(TextStyle::new().font_size(size).color(color)).boxed()
    }

    /// A heading above one group of the page.
    fn heading(text: &'static str, app_theme: &ThemeData) -> AnyWidget {
        Text::new(text)
            .text_style(
                TextStyle::new().font_size(18).font_weight(FontWeight::Bold).color(app_theme.on_background_color),
            )
            .boxed()
    }

    fn button(label: &'static str, app_theme: &ThemeData, on_press: fn()) -> AnyWidget {
        Container::new()
            .width(Dimension::Px(180.0))
            .height(Dimension::Px(40.0))
            .box_child(
                Button::new()
                    .on_press(on_press)
                    .decoration(
                        BoxDecoration::new().background_color(app_theme.primary_color).border_radius((8, 8, 8, 8)),
                    )
                    .child(
                        Text::new(label)
                            .text_align(TextAlign::MidCenter)
                            .text_style(TextStyle::new().font_size(14).color(app_theme.on_primary_color)),
                    ),
            )
            .boxed()
    }

    fn row(buttons: Vec<AnyWidget>) -> AnyWidget {
        Row::new().gaps(LayoutSpacing::new().right(16)).children(buttons).boxed()
    }
}

impl State<NotificationExample> for NotificationExampleState {
    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn create(widget: NotificationExample) -> Self {
        NotificationExampleState { updater: StateUpdater::empty() }
    }


    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        PAGE.set(Some(this.updater()));
        let app_theme = ThemeData::copied(ctx);
        let muted = crate::theme::muted_text(&app_theme);
        let body = app_theme.on_background_color;

        let log: Vec<AnyWidget> = LOG.with_borrow(|log| {
            if log.is_empty() {
                vec![Self::line("Nothing yet. Show a notification and click it.", muted, 14)]
            } else {
                log.iter().rev().map(|entry| Self::line(entry.as_str(), body, 14)).collect()
            }
        });
        let status = STATUS.with_borrow(|status| {
            if status.is_empty() { "Not connected yet. Press any button.".to_owned() } else { status.clone() }
        });
        let permission = PERMISSION.get().map_or("unknown", permission_name);

        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Start)
                    .gaps(LayoutSpacing::new().bottom(12))
                    .children(vec![
                        Self::heading("Notifications", &app_theme),
                        Self::line(status, body, 15),
                        Self::line(format!("Permission: {permission}"), body, 15),
                        Self::row(vec![
                            Self::button("Ask permission", &app_theme, ask_permission),
                            Self::button("Check permission", &app_theme, check_permission),
                        ]),
                        Self::row(vec![
                            Self::button("Show now", &app_theme, show_now),
                            Self::button("Show in 5 seconds", &app_theme, show_later),
                            Self::button("Close both", &app_theme, close_all),
                        ]),
                        Self::row(vec![Self::button("Register for push", &app_theme, register_push)]),
                        Self::heading("Log", &app_theme),
                        Column::new()
                            .horizontal_alignment(BoxAlignment::Start)
                            .gaps(LayoutSpacing::new().bottom(4))
                            .children(log)
                            .boxed(),
                        Self::line(
                            "macOS needs the app to run from a bundle. Android shows notifications but reports no \
                             clicks, Windows shows a balloon without buttons, and Linux needs a notification daemon. \
                             Push needs launcher glue to report the token; desktop has none.",
                            muted,
                            14,
                        ),
                    ]),
            )
    }
}

#[cfg(test)]
mod tests {
    use aimer::notification::PushMessage;
    use aimer::quiver::aimer_app::{AimerApp, AimerNativePlatformEvent};

    use super::*;

    #[test]
    fn every_event_has_a_readable_line() {
        assert_eq!(describe(&NotificationEvent::Clicked(NOW)), "clicked: #1");
        assert_eq!(describe(&NotificationEvent::Dismissed(LATER)), "dismissed: #2");
        assert_eq!(
            describe(&NotificationEvent::Action { notification: NOW, action: SNOOZE }),
            "button Snooze of #1"
        );
        assert_eq!(
            describe(&NotificationEvent::Action { notification: NOW, action: NotificationActionId(9) }),
            "button #9 of #1"
        );
        assert_eq!(describe(&NotificationEvent::Permission(Permission::Granted)), "permission: granted");
        assert_eq!(describe(&NotificationEvent::PushFailed("no".into())), "push failed: no");
    }

    #[test]
    fn a_push_message_shows_its_title_then_its_body() {
        let titled = PushMessage { title: Some("T".into()), body: Some("B".into()), ..PushMessage::default() };
        assert_eq!(describe(&NotificationEvent::PushReceived(titled)), "push: T");
        let untitled = PushMessage { body: Some("B".into()), ..PushMessage::default() };
        assert_eq!(describe(&NotificationEvent::PushReceived(untitled)), "push: B");
        assert_eq!(describe(&NotificationEvent::PushReceived(PushMessage::default())), "push: (no text)");
    }

    #[test]
    fn a_long_token_is_cut_on_a_character_boundary() {
        assert_eq!(abbreviate("short"), "short");
        assert_eq!(abbreviate(&"a".repeat(16)), "a".repeat(16));
        assert_eq!(abbreviate(&"a".repeat(17)), format!("{}…", "a".repeat(16)));
        let wide = "é".repeat(20);
        assert_eq!(abbreviate(&wide), format!("{}…", "é".repeat(16)));
    }

    #[test]
    fn the_samples_are_distinct_and_the_delayed_one_waits() {
        assert_ne!(immediate().id(), delayed().id());
        assert_eq!(delayed(), delayed());
        assert_ne!(immediate(), delayed());
    }

    #[test]
    fn a_notification_event_in_the_loop_reaches_the_page_log() {
        LOG.with_borrow_mut(Vec::clear);
        PERMISSION.set(None);
        let subscription = notification::subscribe(on_event);
        let mut app = AimerApp::start_headless(crate::theme::provide(NotificationExample::new().boxed()));
        app.render_frame();

        app.send_user_event(AimerNativePlatformEvent::Notification {
            id: subscription.id(),
            event: NotificationEvent::Clicked(NOW),
        });
        app.send_user_event(AimerNativePlatformEvent::Notification {
            id: subscription.id(),
            event: NotificationEvent::Permission(Permission::Denied),
        });

        assert_eq!(LOG.with_borrow(Clone::clone), ["clicked: #1", "permission: denied"]);
        assert_eq!(PERMISSION.get(), Some(Permission::Denied));
    }
}
