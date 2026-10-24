//! Native Direct3D 12 implementation of Cupid's experimental GPU backend.
//!
//! Built-in and custom pipeline sources are HLSL. The crate is empty on every
//! target but Windows; `aimer_cupid` selects it with its `native` feature.

#![cfg(target_os = "windows")]

// `crate::backend::..` is how this code has always named the contract.
pub(crate) use aimer_cupid_gpu as backend;

mod commands;
mod pipeline;
mod resource;

#[doc(hidden)]
pub use commands::{Dx12CommandEncoder, Dx12RenderPass};
#[doc(hidden)]
pub use pipeline::{
    Dx12BindGroup, Dx12BindGroupLayout, Dx12PipelineLayout, Dx12RenderPipeline, Dx12ShaderModule,
};

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_PRIORITY_NORMAL,
    D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
    D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, D3D12_FENCE_FLAG_NONE, D3D12_HEAP_FLAG_NONE,
    D3D12_HEAP_PROPERTIES, D3D12_RENDER_TARGET_VIEW_DESC, D3D12_RENDER_TARGET_VIEW_DESC_0,
    D3D12_RESOURCE_DESC, D3D12_RESOURCE_STATES, D3D12_RTV_DIMENSION_TEXTURE2D, D3D12_TEX2D_RTV,
    D3D12CreateDevice, ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_ALPHA_MODE_IGNORE;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    IDXGIFactory2, IDXGISwapChain3,
};
use windows::core::{IUnknown, Interface};

use crate::backend::{
    AddressMode, BackendError, BindGroupEntry, BindGroupLayoutEntry, BufferDescriptor,
    BuiltinShader, CompareFunction, Extent3d, FilterMode, GpuBackend, GpuLimits, Origin3d,
    RenderPassDescriptor, RenderPipelineDescriptor, SamplerDescriptor, TexelCopyBufferInfo,
    TexelCopyTextureInfo, TextureDescriptor, TextureDimension, TextureUsage,
    WriteTextureDescriptor,
};

use resource::{DescriptorLease, DescriptorPool, create_texture_resource};
pub use resource::{Dx12Buffer, Dx12Sampler, Dx12Texture, Dx12TextureFormat, Dx12TextureView};

const RESOURCE_DESCRIPTOR_CAPACITY: u32 = 65_536;
const SAMPLER_DESCRIPTOR_CAPACITY: u32 = 2_048;
const RTV_DESCRIPTOR_CAPACITY: u32 = 4_096;
const COPY_ROW_ALIGNMENT: u32 = 256;

struct PendingBufferUpload {
    source: ID3D12Resource,
    destination: Dx12Buffer,
    destination_offset: u64,
    size: u64,
}

struct PendingTextureUpload {
    source: ID3D12Resource,
    destination: Dx12Texture,
    mip_level: u32,
    origin: Origin3d,
    extent: Extent3d,
    row_pitch: u32,
    image_pitch: u32,
}

enum PendingUpload {
    Buffer(PendingBufferUpload),
    Texture(PendingTextureUpload),
}

struct RetiredBatch {
    fence_value: u64,
    _resources: Vec<ID3D12Resource>,
    _descriptors: Vec<Arc<DescriptorLease>>,
    _objects: Vec<IUnknown>,
}

struct BackendShared {
    pub device: ID3D12Device,
    pub queue: ID3D12CommandQueue,
    pub fence: ID3D12Fence,
    resource_heap: Arc<DescriptorPool>,
    sampler_heap: Arc<DescriptorPool>,
    rtv_heap: Arc<DescriptorPool>,
    pub next_fence: AtomicU64,
    pub last_fence: AtomicU64,
    submit_lock: Mutex<()>,
    pending_uploads: Mutex<Vec<PendingUpload>>,
    retired: Mutex<Vec<RetiredBatch>>,
}

/// Direct3D 12 device, queue, resource allocators, and generic Cupid backend.
pub struct Dx12Backend {
    shared: Arc<BackendShared>,
}

impl Dx12Backend {
    /// Creates the system hardware adapter's D3D12 device and direct queue.
    ///
    /// This constructor does not silently fall back to WARP. It returns the
    /// system error if Windows cannot create a feature-level 11.0 device.
    pub fn new() -> Result<Self, windows::core::Error> {
        let mut device = None;
        unsafe {
            D3D12CreateDevice::<_, ID3D12Device>(
                None::<&IUnknown>,
                D3D_FEATURE_LEVEL_11_0,
                &mut device,
            )?;
        }
        let device = device.expect("D3D12CreateDevice succeeded without a device");
        let queue_desc = D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            Priority: D3D12_COMMAND_QUEUE_PRIORITY_NORMAL.0,
            Flags: Default::default(),
            NodeMask: 0,
        };
        let queue = unsafe { device.CreateCommandQueue::<ID3D12CommandQueue>(&queue_desc)? };
        let fence = unsafe { device.CreateFence::<ID3D12Fence>(0, D3D12_FENCE_FLAG_NONE)? };
        let shared = BackendShared {
            resource_heap: DescriptorPool::new(
                &device,
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                RESOURCE_DESCRIPTOR_CAPACITY,
                true,
            )?,
            sampler_heap: DescriptorPool::new(
                &device,
                D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
                SAMPLER_DESCRIPTOR_CAPACITY,
                true,
            )?,
            rtv_heap: DescriptorPool::new(
                &device,
                D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                RTV_DESCRIPTOR_CAPACITY,
                false,
            )?,
            device,
            queue,
            fence,
            next_fence: AtomicU64::new(1),
            last_fence: AtomicU64::new(0),
            submit_lock: Mutex::new(()),
            pending_uploads: Mutex::new(Vec::new()),
            retired: Mutex::new(Vec::new()),
        };
        Ok(Self {
            shared: Arc::new(shared),
        })
    }

    /// Blocks until all commands submitted before this call have completed.
    pub fn wait_for_gpu(&self) {
        let submission = self
            .shared
            .submit_lock
            .lock()
            .expect("D3D12 submission mutex is not poisoned");
        let target = self.shared.last_fence.load(Ordering::Acquire);
        drop(submission);
        while unsafe { self.shared.fence.GetCompletedValue() } < target {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.collect_retired();
    }

    pub(crate) fn collect_retired(&self) {
        let completed = unsafe { self.shared.fence.GetCompletedValue() };
        self.shared
            .retired
            .lock()
            .expect("D3D12 retired resource mutex is not poisoned")
            .retain(|batch| batch.fence_value > completed);
    }

    fn texture_view(
        &self,
        texture: &Dx12Texture,
        rtv_format: Option<Dx12TextureFormat>,
    ) -> Dx12TextureView {
        let rtv = texture
            .usage
            .contains(&TextureUsage::RenderAttachment)
            .then(|| self.shared.rtv_heap.allocate());
        if let Some(rtv) = &rtv {
            if let Some(format) = rtv_format {
                let desc = D3D12_RENDER_TARGET_VIEW_DESC {
                    Format: format.dxgi(),
                    ViewDimension: D3D12_RTV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D12_RENDER_TARGET_VIEW_DESC_0 {
                        Texture2D: D3D12_TEX2D_RTV {
                            MipSlice: 0,
                            PlaneSlice: 0,
                        },
                    },
                };
                unsafe {
                    self.shared.device.CreateRenderTargetView(
                        &texture.resource,
                        Some(&desc as *const _),
                        rtv.pool.cpu_handle(rtv.index),
                    );
                }
            } else {
                unsafe {
                    self.shared.device.CreateRenderTargetView(
                        &texture.resource,
                        None,
                        rtv.pool.cpu_handle(rtv.index),
                    );
                }
            }
        }
        let srv = texture
            .usage
            .contains(&TextureUsage::TextureBinding)
            .then(|| self.shared.resource_heap.allocate());
        if let Some(srv) = &srv {
            unsafe {
                self.shared.device.CreateShaderResourceView(
                    &texture.resource,
                    None,
                    srv.pool.cpu_handle(srv.index),
                );
            }
        }
        let uav = texture
            .usage
            .contains(&TextureUsage::StorageBinding)
            .then(|| self.shared.resource_heap.allocate());
        if let Some(uav) = &uav {
            unsafe {
                self.shared.device.CreateUnorderedAccessView(
                    &texture.resource,
                    None::<&ID3D12Resource>,
                    None,
                    uav.pool.cpu_handle(uav.index),
                );
            }
        }
        Dx12TextureView {
            texture: texture.clone(),
            rtv,
            srv,
            uav,
        }
    }
}

impl Drop for BackendShared {
    fn drop(&mut self) {
        let target = self.last_fence.load(Ordering::Acquire);
        while unsafe { self.fence.GetCompletedValue() } < target {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// A DXGI flip-model swap chain attached to a Win32 window.
pub struct Dx12Surface {
    swap_chain: IDXGISwapChain3,
    swap_chain_format: Dx12TextureFormat,
    rtv_format: Dx12TextureFormat,
    shared: Arc<BackendShared>,
    buffer_fences: Arc<Mutex<[u64; 3]>>,
    buffers: Vec<Dx12TextureView>,
    width: u32,
    height: u32,
    pending_size: Option<(u32, u32)>,
}

/// An acquired swap-chain buffer and its render-target view.
pub struct Dx12SurfaceFrame {
    pub(crate) texture: Dx12Texture,
    view: Dx12TextureView,
    swap_chain: IDXGISwapChain3,
    shared: Arc<BackendShared>,
    buffer_fences: Arc<Mutex<[u64; 3]>>,
    buffer_index: usize,
}

impl Dx12Surface {
    /// Attaches a flip-discard DXGI swap chain to `hwnd`.
    ///
    /// `format` is the render-target-view format. An sRGB format uses its
    /// matching UNORM format for flip-model swap-chain storage.
    pub fn new(
        backend: &Dx12Backend,
        hwnd: HWND,
        width: u32,
        height: u32,
        format: Dx12TextureFormat,
    ) -> Result<Self, windows::core::Error> {
        let swap_chain_format = format.swap_chain_storage();
        let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width.max(1),
            Height: height.max(1),
            Format: swap_chain_format.dxgi(),
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 3,
            // Keep the previous frame at native size during a resize instead
            // of stretching it to the changing client area while ResizeBuffers
            // waits for the GPU.
            Scaling: DXGI_SCALING_NONE,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };
        let swap_chain1 = unsafe {
            factory.CreateSwapChainForHwnd(
                &backend.shared.queue,
                hwnd,
                &desc,
                None,
                None::<&windows::Win32::Graphics::Dxgi::IDXGIOutput>,
            )?
        };
        let swap_chain: IDXGISwapChain3 = swap_chain1.cast()?;
        let mut surface = Self {
            swap_chain,
            swap_chain_format,
            rtv_format: format,
            shared: backend.shared.clone(),
            buffer_fences: Arc::new(Mutex::new([0; 3])),
            buffers: Vec::with_capacity(3),
            width: width.max(1),
            height: height.max(1),
            pending_size: None,
        };
        surface.rebuild_buffers(backend)?;
        Ok(surface)
    }

    /// Returns the current backbuffer for a render pass.
    pub fn acquire(&self) -> Dx12SurfaceFrame {
        let index = unsafe { self.swap_chain.GetCurrentBackBufferIndex() } as usize;
        let last_fence = self
            .buffer_fences
            .lock()
            .expect("D3D12 surface fence mutex is not poisoned")[index];
        while unsafe { self.shared.fence.GetCompletedValue() } < last_fence {
            std::thread::sleep(Duration::from_millis(1));
        }
        let view = self.buffers[index].clone();
        Dx12SurfaceFrame {
            texture: view.texture.clone(),
            view,
            swap_chain: self.swap_chain.clone(),
            shared: self.shared.clone(),
            buffer_fences: self.buffer_fences.clone(),
            buffer_index: index,
        }
    }

    /// Recreates all swap-chain resources after waiting for their last use.
    pub fn resize(
        &mut self,
        backend: &Dx12Backend,
        width: u32,
        height: u32,
    ) -> Result<(), windows::core::Error> {
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            self.pending_size = None;
            return Ok(());
        }
        backend.wait_for_gpu();
        self.pending_size = None;
        self.resize_buffers(backend, width, height)
    }

    /// Records the latest client size without waiting for the GPU.
    ///
    /// The next [`Self::try_acquire`] applies the size after the queue has
    /// finished using the current swap-chain buffers. Intermediate resize
    /// requests are coalesced.
    pub fn request_resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.pending_size =
            (width != self.width || height != self.height).then_some((width, height));
    }

    /// Acquires a frame, applying any queued resize once prior GPU work is done.
    ///
    /// Returns `Ok(None)` while a frame using the old buffers is still in
    /// flight. Callers can defer their next redraw until the queue becomes idle.
    pub fn try_acquire(
        &mut self,
        backend: &Dx12Backend,
    ) -> Result<Option<Dx12SurfaceFrame>, windows::core::Error> {
        if let Some((width, height)) = self.pending_size {
            if width == self.width && height == self.height {
                self.pending_size = None;
            } else {
                let shared = Arc::clone(&self.shared);
                // Prevent another command list from referencing these buffers
                // between the idle check and ResizeBuffers.
                let _submission = shared
                    .submit_lock
                    .lock()
                    .expect("D3D12 submission mutex is not poisoned");
                let target = shared.last_fence.load(Ordering::Acquire);
                if unsafe { shared.fence.GetCompletedValue() } < target {
                    return Ok(None);
                }
                backend.collect_retired();
                self.resize_buffers(backend, width, height)?;
                self.pending_size = None;
            }
        }
        Ok(Some(self.acquire()))
    }

    fn resize_buffers(
        &mut self,
        backend: &Dx12Backend,
        width: u32,
        height: u32,
    ) -> Result<(), windows::core::Error> {
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            return Ok(());
        }
        self.buffers.clear();
        unsafe {
            self.swap_chain.ResizeBuffers(
                3,
                width,
                height,
                self.swap_chain_format.dxgi(),
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        self.width = width;
        self.height = height;
        *self
            .buffer_fences
            .lock()
            .expect("D3D12 surface fence mutex is not poisoned") = [0; 3];
        self.rebuild_buffers(backend)
    }

    fn rebuild_buffers(&mut self, backend: &Dx12Backend) -> Result<(), windows::core::Error> {
        use windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_STATE_PRESENT;
        for index in 0..3 {
            let resource: ID3D12Resource = unsafe { self.swap_chain.GetBuffer(index)? };
            let state: D3D12_RESOURCE_STATES = D3D12_RESOURCE_STATE_PRESENT;
            let texture = Dx12Texture {
                resource,
                size: (self.width, self.height, 1),
                format: self.swap_chain_format,
                usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
                state: Arc::new(Mutex::new(state)),
            };
            // The UNORM swap-chain buffer gets an sRGB RTV when requested so
            // output conversion and fixed-function blending use linear RGB.
            self.buffers
                .push(backend.texture_view(&texture, Some(self.rtv_format)));
        }
        Ok(())
    }
}

impl Dx12SurfaceFrame {
    /// Returns the render-target view and its physical dimensions.
    #[inline]
    pub fn view(&self) -> &Dx12TextureView {
        &self.view
    }

    #[inline]
    pub fn size(&self) -> (u32, u32) {
        (self.texture.size.0, self.texture.size.1)
    }

    /// Presents the acquired buffer after Cupid has submitted its render pass.
    pub fn present(self) -> Result<(), windows::core::Error> {
        let fence_value = self.shared.last_fence.load(Ordering::Acquire);
        let result = unsafe { self.swap_chain.Present(1, Default::default()).ok() };
        self.buffer_fences
            .lock()
            .expect("D3D12 surface fence mutex is not poisoned")[self.buffer_index] = fence_value;
        result
    }
}

impl GpuBackend for Dx12Backend {
    type Buffer = Dx12Buffer;
    type Texture = Dx12Texture;
    type TextureView = Dx12TextureView;
    type BindGroupLayout = pipeline::Dx12BindGroupLayout;
    type BindGroup = pipeline::Dx12BindGroup;
    type PipelineLayout = pipeline::Dx12PipelineLayout;
    type RenderPipeline = pipeline::Dx12RenderPipeline;
    type Sampler = Dx12Sampler;
    type ShaderModule = pipeline::Dx12ShaderModule;
    type CommandEncoder = commands::Dx12CommandEncoder;
    type TextureFormat = Dx12TextureFormat;
    type RenderPass<'a> = commands::Dx12RenderPass<'a>;

    fn r8_unorm_format() -> Self::TextureFormat {
        Dx12TextureFormat::R8Unorm
    }

    fn rgba8_unorm_format() -> Self::TextureFormat {
        Dx12TextureFormat::Rgba8Unorm
    }

    fn rect_shader_source(&self) -> &'static [u8] {
        include_str!("../shaders/dx12/rect.hlsl").as_bytes()
    }

    fn builtin_shader_source(&self, shader: BuiltinShader) -> &'static [u8] {
        match shader {
            BuiltinShader::Image => include_str!("../shaders/dx12/image.hlsl").as_bytes(),
            BuiltinShader::Text => include_str!("../shaders/dx12/text.hlsl").as_bytes(),
            BuiltinShader::TextColor => {
                include_str!("../shaders/dx12/text_color.hlsl").as_bytes()
            }
            BuiltinShader::TextDecoration => {
                include_str!("../shaders/dx12/text_decoration.hlsl").as_bytes()
            }
            BuiltinShader::Svg => include_str!("../shaders/dx12/svg.hlsl").as_bytes(),
            BuiltinShader::FrameComposite => {
                include_str!("../shaders/dx12/frame_composite.hlsl").as_bytes()
            }
            BuiltinShader::Material => {
                include_str!("../shaders/dx12/material.hlsl").as_bytes()
            }
            BuiltinShader::Morph => include_str!("../shaders/dx12/morph.hlsl").as_bytes(),
        }
    }

    fn create_shader_module(&self, source: &[u8], label: &str) -> Self::ShaderModule {
        self.try_create_shader_module(source, label)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn try_create_shader_module(
        &self,
        source: &[u8],
        label: &str,
    ) -> Result<Self::ShaderModule, BackendError> {
        let source = std::str::from_utf8(source).map_err(|error| BackendError {
            operation: "create_shader_module",
            message: format!("{label} is not valid UTF-8 HLSL: {error}"),
        })?;
        Ok(pipeline::Dx12ShaderModule {
            source: Arc::from(source),
            label: label.to_string(),
            compiled: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        })
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> Self::Buffer {
        let readback = desc.usage.contains(&crate::backend::BufferUsage::MapRead);
        let upload = !readback
            && desc.usage.contains(&crate::backend::BufferUsage::MapWrite)
            && !desc.usage.contains(&crate::backend::BufferUsage::CopyDst);
        Dx12Buffer::new(
            &self.shared,
            desc.size,
            desc.usage.clone(),
            upload,
            readback,
        )
    }

    fn create_texture(&self, desc: &TextureDescriptor<Self::TextureFormat>) -> Self::Texture {
        assert_eq!(
            desc.dimension,
            TextureDimension::D2,
            "D3D12 Cupid textures currently require D2"
        );
        assert_eq!(
            desc.sample_count, 1,
            "multisampled D3D12 textures are not supported yet"
        );
        assert!(
            desc.size.0 > 0 && desc.size.1 > 0,
            "D3D12 texture dimensions must be nonzero"
        );
        create_texture_resource(
            &self.shared,
            desc.size,
            desc.mip_level_count,
            desc.sample_count,
            desc.format,
            desc.usage.clone(),
        )
    }

    fn create_texture_view(&self, texture: &Self::Texture, _label: &str) -> Self::TextureView {
        self.texture_view(texture, None)
    }

    fn create_sampler(&self, desc: &SamplerDescriptor) -> Self::Sampler {
        let lease = self.shared.sampler_heap.allocate();
        let sampler = windows::Win32::Graphics::Direct3D12::D3D12_SAMPLER_DESC {
            Filter: sampler_filter(desc),
            AddressU: sampler_address(desc.address_mode_u),
            AddressV: sampler_address(desc.address_mode_v),
            AddressW: sampler_address(desc.address_mode_w),
            MipLODBias: 0.0,
            MaxAnisotropy: desc.max_anisotropy.max(1) as u32,
            ComparisonFunc: desc.compare.map_or(
                windows::Win32::Graphics::Direct3D12::D3D12_COMPARISON_FUNC_ALWAYS,
                sampler_compare,
            ),
            BorderColor: [0.0, 0.0, 0.0, 0.0],
            MinLOD: desc.lod_min_clamp,
            MaxLOD: desc.lod_max_clamp,
        };
        unsafe {
            self.shared
                .device
                .CreateSampler(&sampler, lease.pool.cpu_handle(lease.index));
        }
        Dx12Sampler { descriptor: lease }
    }

    fn create_bind_group_layout(&self, entries: &[BindGroupLayoutEntry]) -> Self::BindGroupLayout {
        pipeline::bind_group_layout(entries)
    }

    fn create_bind_group(
        &self,
        layout: &Self::BindGroupLayout,
        entries: &[BindGroupEntry<Self>],
    ) -> Self::BindGroup {
        pipeline::bind_group(self, layout, entries)
    }

    fn create_pipeline_layout(&self, layouts: &[&Self::BindGroupLayout]) -> Self::PipelineLayout {
        self.make_pipeline_layout(layouts)
    }

    fn create_render_pipeline(
        &self,
        desc: &RenderPipelineDescriptor<Self>,
    ) -> Self::RenderPipeline {
        self.try_create_render_pipeline(desc)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn try_create_render_pipeline(
        &self,
        desc: &RenderPipelineDescriptor<Self>,
    ) -> Result<Self::RenderPipeline, BackendError> {
        self.make_pipeline(desc)
    }

    fn write_buffer(&self, buffer: &Self::Buffer, offset: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        assert!(
            offset
                .checked_add(data.len() as u64)
                .is_some_and(|end| end <= buffer.size),
            "D3D12 buffer upload exceeds its allocation"
        );
        if buffer.upload_heap {
            unsafe {
                let mut mapped = std::ptr::null_mut();
                buffer
                    .resource
                    .Map(0, None, Some(&mut mapped))
                    .expect("map D3D12 upload buffer");
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    mapped.cast::<u8>().add(offset as usize),
                    data.len(),
                );
                buffer.resource.Unmap(0, None);
            }
            return;
        }
        let source = resource::create_upload_buffer(&self.shared, data);
        self.shared
            .pending_uploads
            .lock()
            .expect("D3D12 upload queue mutex is not poisoned")
            .push(PendingUpload::Buffer(PendingBufferUpload {
                source,
                destination: buffer.clone(),
                destination_offset: offset,
                size: data.len() as u64,
            }));
    }

    fn write_texture(&self, desc: &WriteTextureDescriptor<Self>) {
        assert_eq!(desc.aspect, crate::backend::TextureAspect::All);
        if desc.extent.width == 0
            || desc.extent.height == 0
            || desc.extent.depth_or_array_layers == 0
        {
            return;
        }
        assert!(desc.data.len() > desc.buffer_layout.offset as usize);
        let pixel_size = desc.texture.format.bytes_per_pixel();
        let copied_row_size = (desc.extent.width as usize)
            .checked_mul(pixel_size)
            .expect("D3D12 texture row size overflowed");
        let source_row_pitch = desc
            .buffer_layout
            .bytes_per_row
            .map_or(copied_row_size, |pitch| pitch as usize);
        assert!(source_row_pitch >= copied_row_size);
        let source_rows = desc
            .buffer_layout
            .rows_per_image
            .map_or(desc.extent.height as usize, |rows| rows as usize);
        assert!(source_rows >= desc.extent.height as usize);
        let source_image_pitch = source_row_pitch
            .checked_mul(source_rows)
            .expect("D3D12 source image pitch overflowed");
        let row_pitch = align_copy_row(copied_row_size);
        let image_pitch = row_pitch
            .checked_mul(desc.extent.height)
            .expect("D3D12 upload image pitch overflowed");
        let staging_size = image_pitch
            .checked_mul(desc.extent.depth_or_array_layers)
            .expect("D3D12 upload size overflowed") as usize;
        let source_offset = usize::try_from(desc.buffer_layout.offset)
            .expect("D3D12 texture offset must fit address space");
        let mut packed = vec![0u8; staging_size];
        for layer in 0..desc.extent.depth_or_array_layers as usize {
            for row in 0..desc.extent.height as usize {
                let src = source_offset + layer * source_image_pitch + row * source_row_pitch;
                let dst = layer * image_pitch as usize + row * row_pitch as usize;
                let end = src
                    .checked_add(copied_row_size)
                    .expect("D3D12 source row overflowed");
                assert!(
                    end <= desc.data.len(),
                    "texture upload data does not contain all copied rows"
                );
                packed[dst..dst + copied_row_size].copy_from_slice(&desc.data[src..end]);
            }
        }
        let source = resource::create_upload_buffer(&self.shared, &packed);
        self.shared
            .pending_uploads
            .lock()
            .expect("D3D12 upload queue mutex is not poisoned")
            .push(PendingUpload::Texture(PendingTextureUpload {
                source,
                destination: desc.texture.clone(),
                mip_level: desc.mip_level,
                origin: desc.origin,
                extent: desc.extent,
                row_pitch,
                image_pitch,
            }));
    }

    fn create_command_encoder(&self, label: &str) -> Self::CommandEncoder {
        self.make_command_encoder(label)
    }

    fn begin_render_pass<'a>(
        &self,
        encoder: &'a mut Self::CommandEncoder,
        desc: &RenderPassDescriptor<Self>,
    ) -> Self::RenderPass<'a> {
        self.make_render_pass(encoder, desc)
    }

    fn copy_buffer_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyBufferInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        self.encode_copy_buffer_to_texture(encoder, src, dst, extent)
    }

    fn copy_texture_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        self.encode_copy_texture_to_texture(encoder, src, dst, extent)
    }

    fn copy_texture_to_buffer(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyBufferInfo<Self>,
        extent: Extent3d,
    ) {
        self.encode_copy_texture_to_buffer(encoder, src, dst, extent)
    }

    fn read_buffer(&self, buffer: &Self::Buffer, size: u64) -> Vec<u8> {
        assert!(
            size <= buffer.size,
            "D3D12 readback exceeds its buffer allocation"
        );
        assert!(buffer.usage.contains(&crate::backend::BufferUsage::MapRead));
        self.wait_for_gpu();
        let mut result = vec![0u8; size as usize];
        unsafe {
            let range = windows::Win32::Graphics::Direct3D12::D3D12_RANGE {
                Begin: 0,
                End: size as usize,
            };
            let mut mapped = std::ptr::null_mut();
            buffer
                .resource
                .Map(0, Some(&range), Some(&mut mapped))
                .expect("map D3D12 readback buffer");
            std::ptr::copy_nonoverlapping(mapped.cast::<u8>(), result.as_mut_ptr(), result.len());
            buffer.resource.Unmap(0, None);
        }
        result
    }

    fn submit(&self, encoder: Self::CommandEncoder) {
        encoder.submit();
        self.collect_retired();
    }

    fn limits(&self) -> GpuLimits {
        GpuLimits {
            max_texture_dimension_2d: 16_384,
            min_uniform_buffer_offset_alignment: 256,
        }
    }

    fn format_is_srgb(format: Self::TextureFormat) -> bool {
        format.is_srgb()
    }
}

fn align_copy_row(bytes: usize) -> u32 {
    let alignment = COPY_ROW_ALIGNMENT as usize;
    u32::try_from((bytes + alignment - 1) & !(alignment - 1))
        .expect("D3D12 texture row pitch exceeds u32")
}

fn sampler_filter(desc: &SamplerDescriptor) -> windows::Win32::Graphics::Direct3D12::D3D12_FILTER {
    use windows::Win32::Graphics::Direct3D12::*;
    match (desc.min_filter, desc.mag_filter, desc.mipmap_filter) {
        (FilterMode::Nearest, FilterMode::Nearest, FilterMode::Nearest) => {
            D3D12_FILTER_MIN_MAG_MIP_POINT
        }
        (FilterMode::Nearest, FilterMode::Nearest, FilterMode::Linear) => {
            D3D12_FILTER_MIN_MAG_POINT_MIP_LINEAR
        }
        (FilterMode::Nearest, FilterMode::Linear, FilterMode::Nearest) => {
            D3D12_FILTER_MIN_POINT_MAG_LINEAR_MIP_POINT
        }
        (FilterMode::Nearest, FilterMode::Linear, FilterMode::Linear) => {
            D3D12_FILTER_MIN_POINT_MAG_MIP_LINEAR
        }
        (FilterMode::Linear, FilterMode::Nearest, FilterMode::Nearest) => {
            D3D12_FILTER_MIN_LINEAR_MAG_MIP_POINT
        }
        (FilterMode::Linear, FilterMode::Nearest, FilterMode::Linear) => {
            D3D12_FILTER_MIN_LINEAR_MAG_POINT_MIP_LINEAR
        }
        (FilterMode::Linear, FilterMode::Linear, FilterMode::Nearest) => {
            D3D12_FILTER_MIN_MAG_LINEAR_MIP_POINT
        }
        (FilterMode::Linear, FilterMode::Linear, FilterMode::Linear) => {
            D3D12_FILTER_MIN_MAG_MIP_LINEAR
        }
    }
}

fn sampler_address(
    mode: AddressMode,
) -> windows::Win32::Graphics::Direct3D12::D3D12_TEXTURE_ADDRESS_MODE {
    use windows::Win32::Graphics::Direct3D12::*;
    match mode {
        AddressMode::ClampToEdge => D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        AddressMode::Repeat => D3D12_TEXTURE_ADDRESS_MODE_WRAP,
        AddressMode::MirrorRepeat => D3D12_TEXTURE_ADDRESS_MODE_MIRROR,
    }
}

fn sampler_compare(
    compare: CompareFunction,
) -> windows::Win32::Graphics::Direct3D12::D3D12_COMPARISON_FUNC {
    use windows::Win32::Graphics::Direct3D12::*;
    match compare {
        CompareFunction::Never => D3D12_COMPARISON_FUNC_NEVER,
        CompareFunction::Less => D3D12_COMPARISON_FUNC_LESS,
        CompareFunction::Equal => D3D12_COMPARISON_FUNC_EQUAL,
        CompareFunction::LessEqual => D3D12_COMPARISON_FUNC_LESS_EQUAL,
        CompareFunction::Greater => D3D12_COMPARISON_FUNC_GREATER,
        CompareFunction::NotEqual => D3D12_COMPARISON_FUNC_NOT_EQUAL,
        CompareFunction::GreaterEqual => D3D12_COMPARISON_FUNC_GREATER_EQUAL,
        CompareFunction::Always => D3D12_COMPARISON_FUNC_ALWAYS,
    }
}

pub(crate) fn create_committed_resource<T: Interface>(
    device: &ID3D12Device,
    properties: &D3D12_HEAP_PROPERTIES,
    desc: &D3D12_RESOURCE_DESC,
    initial_state: D3D12_RESOURCE_STATES,
) -> Result<T, windows::core::Error> {
    let mut resource = None;
    unsafe {
        device.CreateCommittedResource(
            properties,
            D3D12_HEAP_FLAG_NONE,
            desc,
            initial_state,
            None,
            &mut resource,
        )?;
    }
    Ok(resource.expect("CreateCommittedResource succeeded without a resource"))
}
