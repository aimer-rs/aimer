//! Pluggable GPU backends for the Cupid rendering engine.
//!
//! The contract (`GpuBackend`, `GpuRenderPass` and the shared descriptors) lives
//! in `aimer_cupid_gpu` and is re-exported here, so every path under
//! `aimer_cupid::backend` is unchanged. Each concrete backend is its own
//! crate, selected by target and Cargo feature below.
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`wgpu`] | `WgpuBackend` — delegates every operation to the `wgpu` crate |
//! | `metal` | `MetalBackend` — native Metal via `objc2-metal` |
//! | `dx12` | `Dx12Backend` — native D3D12 with hand-authored HLSL |
//! | `vulkan` | `VulkanBackend` — native Vulkan with SPIR-V shaders |

pub use aimer_cupid_gpu::*;


// WGPU is an opt-in compatibility backend.
#[cfg(feature = "wgpu")]
pub use aimer_cupid_wgpu as wgpu;

// Native backend modules are selected from the target, not individual Cargo
// backend features. Linux and Android compile both APIs for init-time fallback.
#[cfg(all(
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "macos", target_os = "ios")
))]
pub use aimer_cupid_metal as metal;

#[cfg(all(feature = "native", not(feature = "wgpu"), target_os = "windows"))]
pub use aimer_cupid_dx12 as dx12;

#[cfg(all(
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "linux", target_os = "android")
))]
pub use aimer_cupid_vulkan as vulkan;

#[cfg(all(
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "linux", target_os = "android")
))]
pub use aimer_cupid_opengl as opengl;
#[cfg(all(feature = "web", target_arch = "wasm32", not(feature = "wgpu")))]
pub use aimer_cupid_webgl as webgl;
#[cfg(all(feature = "web", target_arch = "wasm32", not(feature = "wgpu")))]
pub use aimer_cupid_webgpu as webgpu;

/// Backend selected for generic type defaults on this target.
#[cfg(feature = "wgpu")]
pub type DefaultGpuBackend = wgpu::WgpuBackend;

#[cfg(all(
    not(feature = "wgpu"),
    feature = "native",
    any(target_os = "macos", target_os = "ios")
))]
pub type DefaultGpuBackend = metal::MetalBackend;

#[cfg(all(not(feature = "wgpu"), feature = "native", target_os = "windows"))]
pub type DefaultGpuBackend = dx12::Dx12Backend;

#[cfg(all(
    not(feature = "wgpu"),
    feature = "native",
    any(target_os = "linux", target_os = "android")
))]
pub type DefaultGpuBackend = vulkan::VulkanBackend;

#[cfg(all(not(feature = "wgpu"), feature = "web", target_arch = "wasm32"))]
pub type DefaultGpuBackend = webgpu::WebGpuBackend;

#[cfg(all(
    not(feature = "wgpu"),
    not(any(
        all(
            feature = "native",
            any(
                target_os = "macos",
                target_os = "ios",
                target_os = "windows",
                target_os = "linux",
                target_os = "android"
            )
        ),
        all(feature = "web", target_arch = "wasm32")
    ))
))]
compile_error!("enable the `native` or `web` dependency group for this target, or select `wgpu`");
