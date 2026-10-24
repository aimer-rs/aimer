use std::sync::{Arc, Mutex};

use windows::Win32::Graphics::Direct3D12::{
    D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_DESCRIPTOR_HEAP_DESC, D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
    D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, D3D12_DESCRIPTOR_HEAP_TYPE,
    D3D12_GPU_DESCRIPTOR_HANDLE, ID3D12DescriptorHeap, ID3D12Device, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R8_UNORM,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
};

use crate::backend::{BufferUsage, TextureUsage};

use super::{BackendShared, create_committed_resource};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// A texture storage format supported by Cupid's Direct3D 12 backend.
pub enum Dx12TextureFormat {
    /// One 8-bit normalized channel.
    R8Unorm,
    /// Four 8-bit normalized channels.
    Rgba8Unorm,
    /// Four 8-bit normalized channels with sRGB sampling and output conversion.
    Rgba8UnormSrgb,
    /// Four 8-bit normalized channels in BGRA order.
    Bgra8Unorm,
    /// Four 8-bit normalized channels in BGRA order with sRGB conversion.
    Bgra8UnormSrgb,
}

impl Dx12TextureFormat {
    #[inline]
    pub fn dxgi(self) -> DXGI_FORMAT {
        match self {
            Self::R8Unorm => DXGI_FORMAT_R8_UNORM,
            Self::Rgba8Unorm => DXGI_FORMAT_R8G8B8A8_UNORM,
            Self::Rgba8UnormSrgb => DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
            Self::Bgra8Unorm => DXGI_FORMAT_B8G8R8A8_UNORM,
            Self::Bgra8UnormSrgb => DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        }
    }

    #[inline]
    pub fn is_srgb(self) -> bool {
        matches!(self, Self::Rgba8UnormSrgb | Self::Bgra8UnormSrgb)
    }

    pub(super) fn swap_chain_storage(self) -> Self {
        match self {
            Self::Rgba8UnormSrgb => Self::Rgba8Unorm,
            Self::Bgra8UnormSrgb => Self::Bgra8Unorm,
            format => format,
        }
    }

    #[inline]
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::R8Unorm => 1,
            Self::Rgba8Unorm | Self::Rgba8UnormSrgb | Self::Bgra8Unorm | Self::Bgra8UnormSrgb => 4,
        }
    }
}

#[derive(Clone)]
/// Opaque Direct3D 12 buffer used by Cupid's backend interface.
#[doc(hidden)]
pub struct Dx12Buffer {
    pub(super) resource: ID3D12Resource,
    pub(super) size: u64,
    pub(super) usage: Vec<BufferUsage>,
    pub(super) state: Arc<Mutex<windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_STATES>>,
    pub(super) upload_heap: bool,
}

#[derive(Clone)]
/// Opaque Direct3D 12 texture used by Cupid's backend interface.
#[doc(hidden)]
pub struct Dx12Texture {
    pub(super) resource: ID3D12Resource,
    pub(super) size: (u32, u32, u32),
    pub(super) format: Dx12TextureFormat,
    pub(super) usage: Vec<TextureUsage>,
    pub(super) state: Arc<Mutex<windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_STATES>>,
}

#[derive(Clone)]
/// Opaque Direct3D 12 texture view used by Cupid's backend interface.
#[doc(hidden)]
pub struct Dx12TextureView {
    pub(super) texture: Dx12Texture,
    pub(super) rtv: Option<Arc<DescriptorLease>>,
    pub(super) srv: Option<Arc<DescriptorLease>>,
    pub(super) uav: Option<Arc<DescriptorLease>>,
}

#[derive(Clone)]
/// Opaque Direct3D 12 sampler used by Cupid's backend interface.
#[doc(hidden)]
pub struct Dx12Sampler {
    pub(super) descriptor: Arc<DescriptorLease>,
}

pub(super) struct DescriptorPool {
    pub heap: ID3D12DescriptorHeap,
    pub stride: u32,
    capacity: u32,
    next: std::sync::atomic::AtomicU32,
    free: Mutex<Vec<u32>>,
}

pub(super) struct DescriptorLease {
    pub pool: Arc<DescriptorPool>,
    pub index: u32,
}

impl Drop for DescriptorLease {
    fn drop(&mut self) {
        self.pool
            .free
            .lock()
            .expect("D3D12 descriptor pool mutex is not poisoned")
            .push(self.index);
    }
}

impl DescriptorPool {
    pub fn new(
        device: &ID3D12Device,
        kind: D3D12_DESCRIPTOR_HEAP_TYPE,
        capacity: u32,
        shader_visible: bool,
    ) -> Result<Arc<Self>, windows::core::Error> {
        let desc = D3D12_DESCRIPTOR_HEAP_DESC {
            Type: kind,
            NumDescriptors: capacity,
            Flags: if shader_visible {
                D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE
            } else {
                D3D12_DESCRIPTOR_HEAP_FLAG_NONE
            },
            NodeMask: 0,
        };
        let heap = unsafe { device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&desc)? };
        let stride = unsafe { device.GetDescriptorHandleIncrementSize(kind) };
        Ok(Arc::new(Self {
            heap,
            stride,
            capacity,
            next: std::sync::atomic::AtomicU32::new(0),
            free: Mutex::new(Vec::new()),
        }))
    }

    pub fn allocate(self: &Arc<Self>) -> Arc<DescriptorLease> {
        let index = self
            .free
            .lock()
            .expect("D3D12 descriptor pool mutex is not poisoned")
            .pop()
            .unwrap_or_else(|| {
                let index = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                assert!(index < self.capacity, "D3D12 descriptor heap is exhausted");
                index
            });
        Arc::new(DescriptorLease {
            pool: self.clone(),
            index,
        })
    }

    pub fn cpu_handle(&self, index: u32) -> D3D12_CPU_DESCRIPTOR_HANDLE {
        let start = unsafe { self.heap.GetCPUDescriptorHandleForHeapStart() };
        D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: start.ptr + index as usize * self.stride as usize,
        }
    }

    pub fn gpu_handle(&self, index: u32) -> D3D12_GPU_DESCRIPTOR_HANDLE {
        let start = unsafe { self.heap.GetGPUDescriptorHandleForHeapStart() };
        D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: start.ptr + index as u64 * self.stride as u64,
        }
    }
}

impl Dx12Buffer {
    pub(super) fn new(
        shared: &BackendShared,
        size: u64,
        usage: Vec<BufferUsage>,
        upload_heap: bool,
        readback_heap: bool,
    ) -> Self {
        use windows::Win32::Graphics::Direct3D12::{
            D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT, D3D12_HEAP_TYPE_READBACK,
            D3D12_HEAP_TYPE_UPLOAD, D3D12_RESOURCE_DIMENSION_BUFFER, D3D12_RESOURCE_FLAG_NONE,
            D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_GENERIC_READ, D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER,
            D3D12_RESOURCE_STATES, D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        };
        use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC};

        let heap_type = if upload_heap {
            D3D12_HEAP_TYPE_UPLOAD
        } else if readback_heap {
            D3D12_HEAP_TYPE_READBACK
        } else {
            D3D12_HEAP_TYPE_DEFAULT
        };
        let properties = D3D12_HEAP_PROPERTIES {
            Type: heap_type,
            CPUPageProperty: Default::default(),
            MemoryPoolPreference: Default::default(),
            CreationNodeMask: 1,
            VisibleNodeMask: 1,
        };
        let desc = windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Alignment: 0,
            Width: size.max(1),
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_UNKNOWN,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            Flags: D3D12_RESOURCE_FLAG_NONE,
        };
        let streamed_vertex_buffer = usage.contains(&BufferUsage::Vertex)
            && usage.contains(&BufferUsage::CopyDst)
            && !usage.contains(&BufferUsage::Index)
            && !usage.contains(&BufferUsage::ReadOnlyStorage)
            && !usage.contains(&BufferUsage::Storage);
        let initial = if upload_heap {
            D3D12_RESOURCE_STATE_GENERIC_READ
        } else if readback_heap {
            D3D12_RESOURCE_STATE_COPY_DEST
        } else if streamed_vertex_buffer {
            // Keep dynamic instance buffers in their draw state from creation.
            // Their queued copies can then run before an already-recorded draw
            // list without making that list's first barrier out of date.
            D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER
        } else {
            D3D12_RESOURCE_STATE_COMMON
        };
        let resource = create_committed_resource(&shared.device, &properties, &desc, initial)
            .expect("create D3D12 buffer resource");
        let _ = (D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATE_COPY_DEST);
        let state: D3D12_RESOURCE_STATES = initial;
        Self {
            resource,
            size,
            usage,
            state: Arc::new(Mutex::new(state)),
            upload_heap,
        }
    }
}

pub(super) fn create_texture_resource(
    shared: &BackendShared,
    size: (u32, u32, u32),
    mip_levels: u32,
    sample_count: u32,
    format: Dx12TextureFormat,
    usage: Vec<TextureUsage>,
) -> Dx12Texture {
    use windows::Win32::Graphics::Direct3D12::{
        D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
        D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_STATE_COMMON, D3D12_RESOURCE_STATES,
        D3D12_TEXTURE_LAYOUT_UNKNOWN,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;

    assert!(
        size.2 <= u16::MAX as u32,
        "D3D12 array size exceeds the native limit"
    );
    assert!(
        mip_levels <= u16::MAX as u32,
        "D3D12 mip count exceeds the native limit"
    );
    let render_target = usage.contains(&TextureUsage::RenderAttachment);
    let props = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_DEFAULT,
        CPUPageProperty: Default::default(),
        MemoryPoolPreference: Default::default(),
        CreationNodeMask: 1,
        VisibleNodeMask: 1,
    };
    let mut flags = D3D12_RESOURCE_FLAG_NONE;
    if render_target {
        flags |= D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    }
    if usage.contains(&TextureUsage::StorageBinding) {
        flags |= D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;
    }
    let desc = windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: size.0 as u64,
        Height: size.1,
        DepthOrArraySize: size.2.max(1) as u16,
        MipLevels: mip_levels.max(1) as u16,
        Format: format.dxgi(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: sample_count.max(1),
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: flags,
    };
    let resource =
        create_committed_resource(&shared.device, &props, &desc, D3D12_RESOURCE_STATE_COMMON)
            .expect("create D3D12 texture resource");
    let state: D3D12_RESOURCE_STATES = D3D12_RESOURCE_STATE_COMMON;
    Dx12Texture {
        resource,
        size,
        format,
        usage,
        state: Arc::new(Mutex::new(state)),
    }
}

pub(super) fn create_upload_buffer(shared: &BackendShared, data: &[u8]) -> ID3D12Resource {
    use windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_STATE_GENERIC_READ;
    let buffer = Dx12Buffer::new(shared, data.len() as u64, Vec::new(), true, false);
    unsafe {
        let mut mapped = std::ptr::null_mut();
        buffer
            .resource
            .Map(0, None, Some(&mut mapped))
            .expect("map D3D12 upload buffer");
        std::ptr::copy_nonoverlapping(data.as_ptr(), mapped.cast::<u8>(), data.len());
        buffer.resource.Unmap(0, None);
    }
    let _ = D3D12_RESOURCE_STATE_GENERIC_READ;
    buffer.resource
}
