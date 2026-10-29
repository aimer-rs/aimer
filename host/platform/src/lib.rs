//! What the application loop needs from the platform underneath it.
//!
//! The event the native bridges send to the loop, the system's appearance and
//! safe area, the application menu, links and shutdown signals posted from
//! outside, and the per-platform glue (UIKit, AppKit, Android, the browser) that
//! reaches them. Nothing here knows about windows or frames: the loop in
//! `aimer_quiver` and the renderers in `aimer_render` both sit on top of it.

mod event;
pub use event::AimerNativePlatformEvent;
#[cfg(target_os = "android")]
pub use event::ANDROID_APP;
pub use event::EVENT_PROXY;

/// Handing the application's semantic tree to the platform's assistive
/// technology, and taking its action requests back.
pub mod accessibility;
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub mod browser_poll_context;
#[doc(hidden)]
pub mod ffi_utils;
#[doc(hidden)]
pub mod link_inbox;
/// The native application menu, and the shortcuts macOS routes through it.
pub mod menu;
/// Entering the application's async runtime while Venus polls a task. Native
/// only: the browser has one task queue and no runtime to be inside of.
#[cfg(not(target_arch = "wasm32"))]
pub mod poll_context;
#[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
#[doc(hidden)]
pub mod shutdown_signals;
pub mod system_back;
/// Where the platform's light / dark appearance comes from.
#[doc(hidden)]
pub mod system_appearance;
/// Where the region the platform reserves in the window comes from.
#[doc(hidden)]
pub mod system_safe_area;
