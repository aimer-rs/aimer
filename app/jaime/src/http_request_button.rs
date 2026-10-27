//! A button that fetches <https://example.com> from a plain Venus task.
//!
//! The point is what the handler *does not* do. It does not reach for a Tokio
//! handle, it does not wrap the request in anything, and it does not have to be
//! [`Send`]: it spawns an ordinary microtask that captures a `StateUpdater`
//! from the element tree and awaits `reqwest` while holding it.
//!
//! That works because the application installs a
//! [`PollContext`](aimer::venus::PollContext) on its runtime at startup, so
//! every poll Venus performs happens inside the async runtime driving the
//! sockets. The request's TLS handshake and decoding stay on that runtime's own
//! threads; only the completion comes back here, into the microtask phase — so
//! the answer is visible to the very next build rather than a frame later.
//!
//! ```bash
//! cargo run --example http_request_button
//! ```

use std::future::Future;
use std::pin::Pin;

use aimer::input::button::Button;
use aimer::style::{FontWeight, LayoutSpacing, TextAlign, TextStyle, Theme, ThemeData};
use aimer::*;

/// The URL the button asks for: small, stable, and made for exactly this.
const EXAMPLE_URL: &str = "https://example.com";

/// What the screen has to say about the request, and the only state there is.
enum Request {
    /// Nothing has been asked for yet.
    Idle,
    /// A request is in flight; pressing again would be a second one.
    InFlight,
    /// The response arrived, summarised by its status and size.
    Answered { status: u16, bytes: usize },
    /// The request failed, and the reason is worth showing.
    Failed(String),
}

/// The line under the button, for every state a request can be in.
///
/// Split out from `build` because it is the only part of this example with an
/// answer that can be wrong, and therefore the only part worth a test.
fn status_label(request: &Request) -> String {
    match request {
        Request::Idle => format!("Press to GET {EXAMPLE_URL}"),
        Request::InFlight => "Requesting…".to_owned(),
        Request::Answered { status, bytes } => format!("HTTP {status} — {bytes} bytes"),
        Request::Failed(reason) => format!("Failed: {reason}"),
    }
}

/// Performs the request and reduces it to what the screen shows.
///
/// A free `async fn` rather than an inline block: the handler stays about
/// *when* the work runs, and this stays about what the work is.
async fn fetch_example() -> Request {
    let response = match reqwest::get(EXAMPLE_URL).await {
        Ok(response) => response,
        Err(error) => return Request::Failed(error.to_string()),
    };

    let status = response
        .status()
        .as_u16();
    match response
        .text()
        .await
    {
        Ok(body) => Request::Answered {
            status,
            bytes: body.len(),
        },
        Err(error) => Request::Failed(error.to_string()),
    }
}

fn request_callback(
    updater: StateUpdater<HttpRequestButtonState>,
) -> impl FnOnce() -> Pin<Box<dyn Future<Output = ()> + 'static>> {
    move || {
        Box::pin(async move {
            if matches!(
                updater
                    .read_state()
                    .request,
                Request::InFlight
            ) {
                return;
            }
            updater.set_state(|state| state.request = Request::InFlight);
            let answered = updater;

            let request = fetch_example().await;
            answered.set_state(move |state| state.request = request);
        })
    }
}

#[derive(StatefulWidget)]
struct HttpRequestButton;

impl HttpRequestButton {
    /// Creates the screen, in its idle state.
    #[inline]
    fn new() -> Self {
        Self
    }
}

/// Builds the HTTP request showcase without starting an application.
pub fn http_request_button_example() -> impl Widget {
    HttpRequestButton::new()
}

struct HttpRequestButtonState {
    request: Request,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for HttpRequestButton {
    type State = HttpRequestButtonState;

    fn create_state(self) -> Self::State {
        HttpRequestButtonState {
            request: Request::Idle,
            updater: StateUpdater::new(),
        }
    }
}

impl State<HttpRequestButton> for HttpRequestButtonState {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);

        Container::new()
            .color(theme.background_color)
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Center)
                    .vertical_alignment(BoxAlignment::Center)
                    .children([
                        Container::new()
                            .color(theme.primary_color)
                            .width(200)
                            .padding(LayoutSpacing::all(12))
                            .box_child(
                                Button::new()
                                    .on_press_async(request_callback(self.updater))
                                    .box_child(
                                        Text::new("Fetch example.com")
                                            .text_align(TextAlign::MidCenter)
                                            .text_style(
                                                TextStyle::new()
                                                    .color(theme.on_background_color)
                                                    .font_weight(FontWeight::Bold),
                                            ),
                                    ),
                            ),
                        SizedBox::new()
                            .height(16)
                            .boxed(),
                        Text::new(status_label(&self.request))
                            .text_style(TextStyle::new().color(theme.on_background_color))
                            .boxed(),
                    ]),
            )
    }
}

pub fn start_http_request_button() {
    AimerApp::start(crate::theme::provide(HttpRequestButton::new()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answered_request_reports_its_status_and_size() {
        assert_eq!(
            status_label(&Request::Answered {
                status: 200,
                bytes: 1256,
            }),
            "HTTP 200 — 1256 bytes"
        );
    }

    #[test]
    fn a_failed_request_shows_the_reason_rather_than_hiding_it() {
        assert_eq!(
            status_label(&Request::Failed("dns error".to_owned())),
            "Failed: dns error"
        );
    }

    #[test]
    fn async_button_builds_without_starting_the_request() {
        let mut app = AimerApp::start_headless(HttpRequestButton::new());

        app.pump_frames(3);
    }

    #[test]
    fn registering_the_async_callback_is_safe_before_dispatch() {
        let _callback = request_callback(StateUpdater::empty());
    }
}
