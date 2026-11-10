#![doc = include_str!("../README.md")]

mod backdrop;
mod content;
mod exit_policy;
mod id;
mod new_window;
mod request;
mod window_widget;

pub mod window_attr;

pub use aimer_platform::window_backdrop::WindowMaterial;
pub use backdrop::{WindowBackdrop, WindowBackdropState};
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
