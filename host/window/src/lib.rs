//! Windows: opening them, controlling them, and deciding when the application
//! ends with them.
//!
//! A desktop application is not limited to the window it starts with. There are
//! two ways to ask for another, and they are the same window underneath:
//!
//! - the [`Window`] widget declares a window as part of a widget tree. It is
//!   shown while the widget is mounted and `open` is true, follows the widget's
//!   properties, and is closed when the widget goes away;
//! - [`NewWindow`] opens one from a callback, and gives back a [`WindowId`] to
//!   control it with.
//!
//! Every window has a widget tree of its own and shares the application's
//! runtime, its tasks and — unless asked not to — the providers in scope where
//! it was opened. Whether closing windows ends the application is the
//! [`ExitPolicy`], and [`quit`] ends it whatever that says.
//!
//! Windows exist only on desktop platforms. Elsewhere the application has one
//! surface, and a request for another is never answered.

mod content;
mod exit_policy;
mod id;
mod new_window;
mod request;
mod window_widget;

pub mod window_attr;

pub use exit_policy::ExitPolicy;
pub use id::WindowId;
pub use new_window::{ContentBuilder, NewWindow, RequiredBuilder};
pub use window_attr::WindowAttr;
pub use window_widget::Window;

/// What the event loop that hosts windows uses. Not for applications.
#[doc(hidden)]
pub mod host {
    pub use crate::exit_policy::AfterClose;
    pub use crate::request::{
        OpenRequest, Request, WindowCommand, drain, mark_closed, mark_open,
    };
}

/// Asks the application to end, whatever its [`ExitPolicy`] says.
///
/// Safe to call from any thread.
pub fn quit() {
    if let Some(proxy) = aimer_platform::EVENT_PROXY.get() {
        let _ = proxy.send_event(aimer_platform::AimerNativePlatformEvent::ShutdownRequested);
    }
}
