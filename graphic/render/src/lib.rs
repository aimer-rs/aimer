//! Where a recorded frame turns into pixels.
//!
//! The per-platform surfaces ([`render_ctx`]: Metal, Direct3D 12, Vulkan and
//! OpenGL, WGPU, the browser canvas) and the worker thread that encodes and
//! presents off the UI thread ([`raster`]). The application loop in
//! `aimer_quiver` hands this crate a finished frame and asks for the next
//! surface; nothing here knows about widgets or windows beyond the native
//! handle a surface is created on.

#[cfg(not(target_arch = "wasm32"))]
pub mod raster;
pub mod render_ctx;
mod render_mode;
pub use render_mode::RenderMode;
