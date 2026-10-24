//! Try `aimer_passkey` against a relying party running on this machine.
//!
//! Start the server in one terminal:
//!
//! ```bash
//! cargo run -p jaime --features passkey-server --bin passkey_server
//! ```
//!
//! then register a passkey and sign in with it from this page. The server issues
//! the challenge and verifies what the platform returns (client data, relying
//! party hash, user presence, the ES256 signature), so a green line here means
//! the whole ceremony worked, not only that the system sheet appeared.
//!
//! The platform decides whether a given relying party id is usable:
//!
//! * **Windows** signs the origin `https://<rp id>` and checks nothing else, so
//!   the default `localhost` works as is.
//! * **macOS and iOS** need a signed app with the Associated Domains entitlement
//!   and a domain whose `apple-app-site-association` lists it; `localhost` can
//!   never qualify. Start the server with `--rp-id your.domain --apple-app-id
//!   TEAMID.bundle.id` and reach it through an HTTPS tunnel on that domain.
//! * **Android** needs the domain's `assetlinks.json` to list the app; the
//!   server serves it with `--android-package` and `--android-fingerprint`.
//!   The emulator reaches the host at `10.0.2.2`.
//!
//! Until that is set up the page still shows how far the ceremony got and the
//! typed error that stopped it.

mod base64url;
mod flow;
mod protocol;

use std::future::Future;
use std::pin::Pin;

use aimer::input::TextEditingController;
use aimer::input::button::Button;
use aimer::input::input::{InputType, TextField};
use aimer::style::{FontWeight, LayoutSpacing, Spacing, TextAlign, TextStyle, Theme, ThemeData};
use aimer::*;

use self::flow::Operation;

/// Where the server listens by default. The Android emulator cannot see the
/// host's loopback, only the alias `10.0.2.2`.
const DEFAULT_SERVER: &str = if cfg!(target_os = "android") {
    "http://10.0.2.2:8787"
} else {
    "http://127.0.0.1:8787"
};
/// Lines of the log kept on screen. A ceremony writes about six.
const MAX_LOG_LINES: usize = 24;

/// Builds the passkey page without starting an application.
pub fn passkey_example() -> impl Widget {
    PasskeyExample::new()
}

/// Starts the passkey page as a standalone Jaime application.
pub fn start_passkey_example() {
    AimerApp::start(crate::theme::provide(PasskeyExample::new()));
}

#[derive(StatefulWidget)]
struct PasskeyExample;

impl PasskeyExample {
    #[inline]
    fn new() -> Self {
        Self
    }
}

struct PasskeyExampleState {
    server: TextEditingController,
    username: TextEditingController,
    log: Vec<String>,
    busy: bool,
    updater: StateUpdater<Self>,
}

impl PasskeyExampleState {
    fn push(&mut self, line: String) {
        if self.log.len() == MAX_LOG_LINES {
            self.log.remove(0);
        }
        self.log.push(line);
    }
}

impl StatefulWidget for PasskeyExample {
    type State = PasskeyExampleState;

    fn create_state(self) -> Self::State {
        PasskeyExampleState {
            server: TextEditingController::with_text(DEFAULT_SERVER),
            username: TextEditingController::with_text("alice@example.com"),
            log: vec!["Start the server, then press Check setup.".to_owned()],
            busy: false,
            updater: StateUpdater::new(),
        }
    }
}

/// The handler for one button press.
///
/// A button takes its closure on the first press, so this is built afresh on
/// every build; marking the page busy rebuilds it, which re-arms the buttons.
fn operation_callback(
    operation: Operation,
    server: String,
    username: String,
    updater: StateUpdater<PasskeyExampleState>,
) -> impl FnOnce() -> Pin<Box<dyn Future<Output = ()> + 'static>> {
    move || {
        Box::pin(async move {
            if updater.read_state().busy {
                return;
            }
            updater.set_state(|state| {
                state.busy = true;
                state.log.clear();
            });

            let log = |line: String| updater.set_state(move |state| state.push(line));
            let line = match flow::run(operation, &server, &username, &log).await {
                Ok(done) => format!("✓ {done}"),
                Err(failure) => format!("✗ {failure}"),
            };
            updater.set_state(move |state| {
                state.push(line);
                state.busy = false;
            });
        })
    }
}

impl State<PasskeyExample> for PasskeyExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        let server = self.server.value().text().to_owned();
        let username = self.username.value().text().to_owned();
        let press = |operation| operation_callback(operation, server.clone(), username.clone(), self.updater);

        Scrollable::new()
            .vertical_scroll_bar(None)
            .axis(ScrollAxis::Vertical)
            .child(
                Container::new()
                    .color(theme.background_color)
                    .padding(LayoutSpacing::all(Spacing::Px(32)))
                    .child(
                        Column::new()
                            .gaps(LayoutSpacing::all(Spacing::Px(12)))
                            .children([
                                Text::new("Passkeys")
                                    .text_style(
                                        TextStyle::new()
                                            .font_size(28)
                                            .font_weight(FontWeight::Bold)
                                            .color(theme.on_background_color),
                                    )
                                    .boxed(),
                                Text::new(
                                    "Create and use a passkey against the local server \
                                     (cargo run -p jaime --features passkey-server --bin passkey_server).",
                                )
                                .wrapped()
                                .text_style(
                                    TextStyle::new()
                                        .font_size(16)
                                        .color(crate::theme::muted_text(&theme)),
                                )
                                .boxed(),
                                field("Server", &self.server, "http://127.0.0.1:8787", &theme),
                                field("Account name", &self.username, "alice@example.com", &theme),
                                Row::new()
                                    .gaps(LayoutSpacing::all(Spacing::Px(12)))
                                    .children([
                                        action("Check setup", &theme, self.busy, press(Operation::Check)),
                                        action("Register passkey", &theme, self.busy, press(Operation::Register)),
                                        action("Sign in", &theme, self.busy, press(Operation::SignIn)),
                                    ])
                                    .boxed(),
                                Column::new()
                                    .children(
                                        self.log
                                            .iter()
                                            .map(|line| {
                                                Text::new(line.clone())
                                                    .wrapped()
                                                    .text_style(
                                                        TextStyle::new()
                                                            .font_size(15)
                                                            .color(theme.on_background_color),
                                                    )
                                                    .boxed()
                                            })
                                            .collect::<Vec<_>>(),
                                    )
                                    .boxed(),
                            ]),
                    ),
            )
    }
}

/// A captioned single-line field.
fn field(
    caption: &'static str,
    controller: &TextEditingController,
    hint: &'static str,
    theme: &ThemeData,
) -> AnyWidget {
    Column::new()
        .gaps(LayoutSpacing::all(Spacing::Px(4)))
        .children([
            Text::new(caption)
                .text_style(
                    TextStyle::new()
                        .font_size(14)
                        .color(crate::theme::muted_text(theme)),
                )
                .boxed(),
            TextField::new()
                .controller(controller.clone())
                .input_type(InputType::Text)
                .hint(hint)
                .hint_style(
                    TextStyle::new()
                        .font_size(16)
                        .color(crate::theme::muted_text(theme)),
                )
                .text_style(TextStyle::new().font_size(16).color(theme.on_surface_color))
                .decoration(crate::theme::input_decoration(theme))
                .hover_decoration(crate::theme::input_hover_decoration(theme))
                .focus_decoration(crate::theme::input_focus_decoration(theme))
                .cursor_color(crate::theme::input_cursor_color(theme))
                .selection_color(theme.primary_color.with_alpha(0.28))
                .padding(LayoutSpacing::all(Spacing::Px(12)))
                .boxed(),
        ])
        .boxed()
}

/// A filled button that is inert while an operation is running.
fn action<F, Fut>(label: &'static str, theme: &ThemeData, busy: bool, on_press: F) -> AnyWidget
where
    F: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    Container::new()
        .color(if busy {
            crate::theme::muted_text(theme)
        } else {
            theme.primary_color
        })
        .padding(LayoutSpacing::all(Spacing::Px(12)))
        .box_child(
            Button::new()
                .disabled(busy)
                .on_press_async(on_press)
                .box_child(
                    Text::new(label)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .color(theme.on_background_color)
                                .font_weight(FontWeight::Bold),
                        ),
                ),
        )
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_builds_and_shows_its_prompt_without_touching_the_network() {
        let mut app = AimerApp::start_headless(crate::theme::provide(PasskeyExample::new()));

        app.pump_frames(3);
    }

    #[test]
    fn the_log_keeps_only_the_most_recent_lines() {
        let mut state = PasskeyExample::new().create_state();

        for n in 0..MAX_LOG_LINES + 5 {
            state.push(format!("line {n}"));
        }

        assert_eq!(state.log.len(), MAX_LOG_LINES);
        assert_eq!(state.log.last().unwrap(), &format!("line {}", MAX_LOG_LINES + 4));
        assert_eq!(state.log.first().unwrap(), "line 5");
    }

    #[test]
    fn registering_a_callback_is_safe_before_dispatch() {
        let _callback = operation_callback(
            Operation::Check,
            DEFAULT_SERVER.to_owned(),
            String::new(),
            StateUpdater::empty(),
        );
    }
}
