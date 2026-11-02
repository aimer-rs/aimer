//! The native Vulkan backend for the Cupid rendering engine.
//!
//! The backend owns its Vulkan instance and device. Use
//! [`VulkanBackend::new_headless`] for offscreen rendering or
//! [`VulkanBackend::new_windowed`] to create a surface that can acquire and
//! present frames.
//!
//! `ash` builds only for Linux and Android, so the backend is compiled only
//! there. The descriptor and swapchain-format policies are plain logic that
//! every host can test, so they are compiled everywhere.

// `crate::backend::..` is how this code has always named the contract.
pub(crate) use aimer_cupid_gpu as backend;

pub(crate) mod vulkan_descriptor_kind;
pub(crate) mod vulkan_surface_format;

#[cfg(any(target_os = "linux", target_os = "android"))]
mod native;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub use native::*;
