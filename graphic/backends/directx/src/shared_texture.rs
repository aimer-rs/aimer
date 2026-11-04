//! Textures another Direct3D device produced, opened by shared handle.
//!
//! A video decoder (Media Foundation) writes its pictures into Direct3D 11
//! textures it shares by NT handle, and signals a shared fence once each one is
//! written. The renderer opens the handles on its own device, makes its queue
//! wait on the fence and samples the textures as they are: nothing is copied.
//!
//! What was opened is kept under (`pool`, `slot`), which names one texture for
//! the life of the process, so a playing video opens each of its few textures
//! once. When a video's pool is gone (see `aimer_cupid_gpu::external_pool`) its
//! textures are dropped.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D12::{
    D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATES, ID3D12Device, ID3D12Fence, ID3D12Resource,
};
use windows::core::Interface;

use crate::backend::TextureUsage;
use crate::resource::{Dx12Texture, Dx12TextureFormat};

/// What has been opened, by producer pool and slot.
#[derive(Default)]
pub(crate) struct SharedImports {
    textures: HashMap<(u32, u32), Imported>,
    fences: HashMap<u32, ImportedFence>,
}

struct Imported {
    texture: Dx12Texture,
    /// The handle it was opened from, to notice a changed texture.
    handle: usize,
    size: (u32, u32),
}

struct ImportedFence {
    fence: ID3D12Fence,
    handle: usize,
}

/// Opens the NT handle `value` on `device` as an `T`.
///
/// # Safety
///
/// `value` must be an open NT handle to a shared D3D12-compatible resource or
/// fence, and stay open for the call.
unsafe fn open<T: Interface>(device: &ID3D12Device, value: usize) -> Option<T> {
    let mut opened: Option<T> = None;
    // SAFETY: the caller vouches for the handle; `opened` receives a +1 reference.
    unsafe { device.OpenSharedHandle(HANDLE(value as *mut c_void), &mut opened) }.ok()?;
    opened
}

impl SharedImports {
    /// The texture of `slot` in `pool`, opening it from `handle` the first time.
    ///
    /// Returns `None` when the device cannot open the handle (a texture from
    /// another adapter, for example).
    pub(crate) fn texture(
        &mut self,
        device: &ID3D12Device,
        pool: u32,
        slot: u32,
        texture_handle: usize,
        size: (u32, u32),
    ) -> Option<Dx12Texture> {
        if let Some(entry) = self.textures.get(&(pool, slot))
            && entry.handle == texture_handle
            && entry.size == size
        {
            return Some(entry.texture.clone());
        }
        // SAFETY: the producer keeps `texture_handle` open until the GPU has
        // finished with the frame that names it (see `NativeTexture::Dxgi`).
        let resource: ID3D12Resource = unsafe { open(device, texture_handle) }?;
        let texture = Dx12Texture {
            resource,
            size: (size.0, size.1, 1),
            format: Dx12TextureFormat::Bgra8Unorm,
            usage: vec![TextureUsage::TextureBinding],
            state: Arc::new(Mutex::new(D3D12_RESOURCE_STATE_COMMON as D3D12_RESOURCE_STATES)),
        };
        self.textures.insert(
            (pool, slot),
            Imported {
                texture: texture.clone(),
                handle: texture_handle,
                size,
            },
        );
        Some(texture)
    }

    /// The shared fence of `pool`, opening it from `fence_handle` the first time.
    pub(crate) fn fence(&mut self, device: &ID3D12Device, pool: u32, fence_handle: usize) -> Option<ID3D12Fence> {
        if let Some(entry) = self.fences.get(&pool)
            && entry.handle == fence_handle
        {
            return Some(entry.fence.clone());
        }
        // SAFETY: as for the texture: the producer keeps the handle open.
        let fence: ID3D12Fence = unsafe { open(device, fence_handle) }?;
        self.fences.insert(
            pool,
            ImportedFence {
                fence: fence.clone(),
                handle: fence_handle,
            },
        );
        Some(fence)
    }

    /// Drops what belongs to pools that no longer exist.
    pub(crate) fn prune(&mut self) {
        self.textures.retain(|(pool, _), _| aimer_cupid_gpu::external_pool::is_live(*pool));
        self.fences.retain(|pool, _| aimer_cupid_gpu::external_pool::is_live(*pool));
    }
}
