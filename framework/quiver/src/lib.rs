extern crate aimer_widget;

#[cfg(target_os = "android")]
mod android_storage;
#[cfg(not(target_arch = "wasm32"))]
mod debug_input;
#[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
mod attach_overlay;
#[cfg(target_os = "android")]
pub use android_storage::register_android_app;
#[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
pub(crate) use aimer_platform::shutdown_signals;
pub(crate) use aimer_platform::{ffi_utils, link_inbox};
mod first_frame;
mod frame_wake;
mod window_locals;
#[cfg(any(target_os = "ios", test))]
mod ios_input;
pub use aimer_render::RenderMode;
#[cfg(test)]
mod render_mode_tests;

#[macro_use]
pub mod aimer_app;
pub mod handler;
pub use aimer_app::{AimerApp, HeadlessAimerApp, HeadlessOptions};
pub use aimer_cupid::AntiAlias;
pub use first_frame::{FIRST_FRAME_RENDERED_EVENT, set_first_frame_rendered_callback};
#[cfg(target_os = "ios")]
mod ios_screen {
    pub use crate::ffi_utils::ios_screen::attach_window_to_active_scene;
}

mod adapter_detail;
pub use aimer_frame_stats as frame_stats;
/// The native application menu, and the shortcuts macOS routes through it.
pub use aimer_platform::menu;
/// Entering the application's async runtime while Venus polls a task. Native
/// only: the browser has one task queue and no runtime to be inside of.
#[cfg(not(target_arch = "wasm32"))]
pub use aimer_platform::poll_context;
#[cfg(target_arch = "wasm32")]
pub(crate) use aimer_platform::browser_poll_context;
/// Off-thread rasterization. Native only: the browser has no thread the WebGPU
/// objects could move to, so the web backend presents inline.
#[cfg(not(target_arch = "wasm32"))]
pub use aimer_render::raster;
pub use aimer_render::render_ctx;
pub(crate) use aimer_platform::{system_appearance, system_safe_area};
pub use aimer_window::window_attr;
pub use aimer_window::{ExitPolicy, NewWindow, Window, WindowAttr, WindowId};
/// Opening, controlling and closing windows.
pub use aimer_window as windows;
#[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
mod desktop_host;

pub use winit;

