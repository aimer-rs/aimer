//! Native Metal backend.
//!
//! Enabled by the `metal` feature, which implies the `GpuBackend` gate and
//! resolves only on Apple targets, where `objc2-metal` is declared as a
//! target dependency.
//!
//! # Shader source
//!
//! [`GpuBackend::create_shader_module`] receives an opaque byte blob and each
//! backend interprets it in its own native language: `WgpuBackend` reads WGSL,
//! this backend reads MSL. A pipeline that runs on both must supply the source
//! matching its backend.
//!
//! # Thread safety
//!
//! Metal documents device, queue, buffer, texture, sampler, library and
//! pipeline-state objects as usable from any thread, so the resource newtypes
//! are `Send + Sync`, matching the assertion `wgpu`'s own Metal backend makes
//! for the same objects. A recording render pass is deliberately *not*: a
//! command encoder belongs to the thread recording into it.
//!
//! # Deferred
//!
//! Buffers are always allocated `MTLStorageModeShared`. GPU-only buffers would
//! prefer `Private` and CPU-written ones prefer write-combined caching; both
//! are allocation-policy refinements, and tuning this path is out of scope for
//! the backend's introduction.

use std::ops::Range;
#[cfg(feature = "frame-stats")]
use std::ptr::NonNull;
#[cfg(feature = "frame-stats")]
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(feature = "frame-stats")]
use std::time::Duration;

#[cfg(feature = "frame-stats")]
use block2::RcBlock;
use objc2::ffi::NSUInteger;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLBlitCommandEncoder, MTLBlitOption, MTLBuffer,
    MTLClearColor, MTLColorWriteMask, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLCompareFunction, MTLCompileOptions, MTLCreateSystemDefaultDevice, MTLDevice, MTLFunction,
    MTLIndexType, MTLLanguageVersion, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat,
    MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPassColorAttachmentDescriptor,
    MTLRenderPassDescriptor, MTLRenderPipelineColorAttachmentDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResource, MTLResourceOptions,
    MTLSamplerAddressMode, MTLSamplerBorderColor, MTLSamplerDescriptor, MTLSamplerMinMagFilter,
    MTLSamplerMipFilter, MTLSamplerState, MTLScissorRect, MTLSize, MTLStorageMode, MTLStoreAction,
    MTLTexture, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage, MTLVertexDescriptor,
    MTLVertexFormat, MTLVertexStepFunction,
};
#[cfg(test)]
use objc2_quartz_core::CAMetalLayer;

use crate::backend::{
    AddressMode, BackendError, BindGroupEntry, BindGroupLayoutEntry, BindingResource, BindingType,
    BufferDescriptor, BuiltinShader, ColorTargetState, Extent3d, FilterMode, GpuBackend, GpuLimits,
    GpuRenderPass, IndexFormat, LoadOp, Origin3d, RenderPassColorAttachment,
    RenderPassDepthStencilAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
    SamplerDescriptor, ShaderStage, StoreOp, TexelCopyBufferInfo, TexelCopyTextureInfo,
    TextureDescriptor, TextureDimension, TextureUsage, VertexBufferLayout, VertexFormat,
    VertexStepMode, WriteTextureDescriptor,
};

pub mod binding;
mod presentation;
use presentation::MetalQueueState;
pub use presentation::threaded::{MetalDrawableSource, ScheduledMetalDrawable};
pub use presentation::{MetalSurface, MetalSurfaceFrame};

// objc2-metal emits the `Metal` framework link itself, but documents that the
// symbols `MTLCreateSystemDefaultDevice` resolves through live in
// CoreGraphics, which the consumer must link.
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {}

/// The MSL version every library is compiled against.
///
/// 2.1 covers everything the ported shaders use and is accepted by every
/// toolchain this crate runs on. Metal accepts older dialects than the
/// installed one, so a fixed value is safe; raising it is what needs care.
const MSL_LANGUAGE_VERSION: MTLLanguageVersion = MTLLanguageVersion::Version2_1;

// ── Backend handle ────────────────────────────────────────────────────────

/// A [`GpuBackend`] driving a native Metal device and command queue.
pub struct MetalBackend {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    queue_state: Mutex<MetalQueueState>,
    #[cfg(feature = "frame-stats")]
    active_gpu_frame: Mutex<Option<Arc<GpuFrameCapture>>>,
}

#[cfg(feature = "frame-stats")]
#[derive(Debug)]
struct GpuFrameCapture {
    report: fn(Duration),
    state: Mutex<GpuFrameCaptureState>,
}

#[cfg(feature = "frame-stats")]
#[derive(Debug, Default)]
struct GpuFrameCaptureState {
    pending_submissions: usize,
    encoding_finished: bool,
    invalid_timestamp: bool,
    has_interval: bool,
    gpu_start_seconds: f64,
    gpu_end_seconds: f64,
    reported: bool,
}

#[cfg(feature = "frame-stats")]
impl GpuFrameCapture {
    fn new(report: fn(Duration)) -> Self {
        Self {
            report,
            state: Mutex::new(GpuFrameCaptureState::default()),
        }
    }

    fn register_submission(&self) {
        let mut state = self
            .state
            .lock()
            .expect("Metal GPU frame timing state is not poisoned");
        state.pending_submissions = state.pending_submissions.saturating_add(1);
    }

    fn complete_submission(&self, gpu_start_seconds: f64, gpu_end_seconds: f64) {
        let duration = {
            let mut state = self
                .state
                .lock()
                .expect("Metal GPU frame timing state is not poisoned");
            if gpu_start_seconds.is_finite()
                && gpu_end_seconds.is_finite()
                && gpu_start_seconds > 0.0
                && gpu_end_seconds > gpu_start_seconds
            {
                if state.has_interval {
                    state.gpu_start_seconds = state.gpu_start_seconds.min(gpu_start_seconds);
                    state.gpu_end_seconds = state.gpu_end_seconds.max(gpu_end_seconds);
                } else {
                    state.gpu_start_seconds = gpu_start_seconds;
                    state.gpu_end_seconds = gpu_end_seconds;
                    state.has_interval = true;
                }
            } else {
                state.invalid_timestamp = true;
            }
            state.pending_submissions = state.pending_submissions.saturating_sub(1);
            ready_gpu_duration(&mut state)
        };
        if let Some(duration) = duration {
            (self.report)(duration);
        }
    }

    fn finish_encoding(&self) {
        let duration = {
            let mut state = self
                .state
                .lock()
                .expect("Metal GPU frame timing state is not poisoned");
            state.encoding_finished = true;
            ready_gpu_duration(&mut state)
        };
        if let Some(duration) = duration {
            (self.report)(duration);
        }
    }
}

#[cfg(feature = "frame-stats")]
fn ready_gpu_duration(state: &mut GpuFrameCaptureState) -> Option<Duration> {
    if !state.encoding_finished || state.pending_submissions != 0 || state.reported {
        return None;
    }
    state.reported = true;
    if state.invalid_timestamp || !state.has_interval {
        return None;
    }
    let seconds = state.gpu_end_seconds - state.gpu_start_seconds;
    (seconds.is_finite() && seconds > 0.0).then(|| Duration::from_secs_f64(seconds))
}

enum PendingUpload {
    Buffer {
        source: MetalBuffer,
        destination: MetalBuffer,
        destination_offset: u64,
        size: u64,
    },
    Texture {
        source: MetalBuffer,
        destination: MetalTexture,
        mip_level: u32,
        origin: Origin3d,
        extent: Extent3d,
        bytes_per_row: u32,
        bytes_per_image: u32,
    },
}

impl MetalBackend {
    /// Creates a backend over the default system Metal device.
    ///
    /// Returns `None` when the machine exposes no Metal device, which is the
    /// normal result on a headless macOS host with no GPU driver. Callers
    /// should read that as "this backend is unavailable here", not as a
    /// renderer fault.
    pub fn new() -> Option<Self> {
        let device = MTLCreateSystemDefaultDevice()?;
        let queue = device.newCommandQueue()?;
        Some(Self {
            device,
            queue,
            queue_state: Mutex::new(MetalQueueState::default()),
            #[cfg(feature = "frame-stats")]
            active_gpu_frame: Mutex::new(None),
        })
    }

    /// Starts collecting GPU intervals for command buffers submitted until
    /// [`Self::end_gpu_frame_timing`] is called. `report` runs once after every
    /// submission in the captured frame has completed with valid timestamps.
    #[cfg(feature = "frame-stats")]
    #[doc(hidden)]
    pub fn begin_gpu_frame_timing(&self, report: fn(Duration)) {
        let capture = Arc::new(GpuFrameCapture::new(report));
        let previous = self
            .active_gpu_frame
            .lock()
            .expect("Metal active GPU frame state is not poisoned")
            .replace(capture);
        if let Some(previous) = previous {
            previous.finish_encoding();
        }
    }

    /// Closes the current GPU frame capture; its timestamp is reported after
    /// all command buffers submitted by the renderer have completed.
    #[cfg(feature = "frame-stats")]
    #[doc(hidden)]
    pub fn end_gpu_frame_timing(&self) {
        let capture = self
            .active_gpu_frame
            .lock()
            .expect("Metal active GPU frame state is not poisoned")
            .take();
        if let Some(capture) = capture {
            capture.finish_encoding();
        }
    }

    /// Copies queued CPU uploads into GPU resources before the next submitted
    /// render or readback command buffer. Staging buffers keep CPU writes away
    /// from resources that an earlier in-flight frame may still be reading.
    fn commit_pending_uploads(&self, pending: &mut Vec<PendingUpload>) {
        if pending.is_empty() {
            return;
        }

        let command_buffer = self
            .queue
            .commandBuffer()
            .expect("the command queue to hand out an upload command buffer");
        let blit = command_buffer
            .blitCommandEncoder()
            .expect("the upload command buffer to hand out a blit encoder");
        Self::encode_pending_uploads(&blit, pending);
        blit.endEncoding();
        command_buffer.commit();
    }

    /// Records staged uploads in order on a command buffer's blit encoder.
    fn encode_pending_uploads(
        blit: &Retained<ProtocolObject<dyn MTLBlitCommandEncoder>>,
        pending: &mut Vec<PendingUpload>,
    ) {
        let uploads = std::mem::take(pending);

        for upload in &uploads {
            match upload {
                PendingUpload::Buffer {
                    source,
                    destination,
                    destination_offset,
                    size,
                } => unsafe {
                    blit.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
                        &source.0,
                        0,
                        &destination.0,
                        *destination_offset as NSUInteger,
                        *size as NSUInteger,
                    );
                },
                PendingUpload::Texture {
                    source,
                    destination,
                    mip_level,
                    origin,
                    extent,
                    bytes_per_row,
                    bytes_per_image,
                } => {
                    let is_3d = destination.0.textureType() == MTLTextureType::Type3D;
                    unsafe {
                        blit.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                            &source.0,
                            0,
                            *bytes_per_row as NSUInteger,
                            *bytes_per_image as NSUInteger,
                            mtl_size(*extent),
                            &destination.0,
                            if is_3d { 0 } else { origin.z as NSUInteger },
                            *mip_level as NSUInteger,
                            MTLOrigin {
                                x: origin.x as NSUInteger,
                                y: origin.y as NSUInteger,
                                z: if is_3d { origin.z as NSUInteger } else { 0 },
                            },
                        );
                    }
                }
            }
        }
        // Metal command buffers retain resources referenced by their encoders
        // until execution completes, so the staging buffers remain alive.
    }

    /// Creates a shared staging buffer and copies `bytes` into it on the CPU.
    fn staging_buffer(&self, bytes: &[u8], label: &str) -> MetalBuffer {
        let length = NSUInteger::try_from(bytes.len())
            .expect("upload size to fit an NSUInteger on this platform");
        let buffer = self
            .device
            .newBufferWithLength_options(length, MTLResourceOptions::StorageModeShared)
            .unwrap_or_else(|| {
                panic!(
                    "the device refused to create a {} byte upload buffer",
                    bytes.len()
                )
            });
        buffer.setLabel(Some(&NSString::from_str(label)));
        // SAFETY: `contents` is valid for a shared buffer, and `bytes.len()` is
        // exactly the allocation length. The CPU owns the staging allocation
        // until its copy is queued on the Metal command queue.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                buffer.contents().cast::<u8>().as_ptr(),
                bytes.len(),
            );
        }
        MetalBuffer(buffer)
    }

    /// Compile MSL source, reporting the compiler's own diagnostics rather than
    /// only the fact that it failed.
    fn compile_msl(
        &self,
        source: &str,
        label: &str,
    ) -> Result<Retained<ProtocolObject<dyn MTLLibrary>>, BackendError> {
        let options = MTLCompileOptions::new();
        options.setLanguageVersion(MSL_LANGUAGE_VERSION);
        // Preserve-invariance keeps Metal from re-associating float arithmetic,
        // which is what lets these pixels be compared against the wgpu
        // reference render. It exists on macOS 11 and later, and this backend
        // only claims to support that range.
        options.setPreserveInvariance(true);
        self.device
            .newLibraryWithSource_options_error(&NSString::from_str(source), Some(&options))
            .map_err(|error| BackendError {
                operation: "create_shader_module",
                message: format!("{label}: {}", error.localizedDescription()),
            })
    }
}

/// A Metal buffer.
#[derive(Clone)]
pub struct MetalBuffer(Retained<ProtocolObject<dyn MTLBuffer>>);

/// A Metal texture.
#[derive(Clone)]
pub struct MetalTexture(Retained<ProtocolObject<dyn MTLTexture>>);

/// A Metal texture view.
///
/// Metal binds a render-pass attachment as a texture plus a level and slice, so
/// a view adds nothing here. The type exists because the trait separates the two
/// for backends where they are genuinely distinct.
#[derive(Clone)]
pub struct MetalTextureView(Retained<ProtocolObject<dyn MTLTexture>>);

/// A Metal sampler state.
#[derive(Clone)]
pub struct MetalSampler(Retained<ProtocolObject<dyn MTLSamplerState>>);

/// A compiled Metal function library.
#[derive(Clone)]
pub struct MetalShaderModule(Retained<ProtocolObject<dyn MTLLibrary>>);

/// A compiled pipeline, plus the primitive class to draw it with.
///
/// Metal takes the topology as a draw argument rather than binding it into the
/// pipeline object, and the trait's `draw` carries no topology, so it is stored
/// here to be replayed at draw time.
#[derive(Clone)]
pub struct MetalRenderPipeline {
    state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    topology: MTLPrimitiveType,
}

/// Which of Metal's three per-stage argument tables a binding occupies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArgumentKind {
    Buffer,
    Texture,
    Sampler,
}

/// A recorded binding slot within a layout.
#[derive(Clone, Debug)]
struct LayoutEntry {
    binding: u32,
    kind: ArgumentKind,
    vertex: bool,
    fragment: bool,
    dynamic_offset: bool,
}

/// A bind group layout, kept as the plan a [`MetalBindGroup`] is resolved
/// against.
#[derive(Clone, Debug)]
pub struct MetalBindGroupLayout {
    entries: Vec<LayoutEntry>,
}

/// A pipeline layout.
///
/// Metal has no pipeline layout object, and this one carries no state because
/// it cannot: a group's index is what selects its argument indices, and
/// `create_bind_group` is never told which index it will later be bound at, so
/// there is nothing to precompute here. The type exists to receive the one
/// check the backend can make — that the group count fits the index rule — and
/// to satisfy the trait's signature.
#[derive(Clone, Copy, Debug)]
pub struct MetalPipelineLayout;

/// One resolved resource in a bind group.
#[derive(Clone)]
enum ResolvedArgument {
    Buffer {
        buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
        offset: u64,
        dynamic_offset: bool,
        binding: u32,
        vertex: bool,
        fragment: bool,
    },
    Texture {
        texture: Retained<ProtocolObject<dyn MTLTexture>>,
        binding: u32,
        fragment: bool,
    },
    Sampler {
        sampler: Retained<ProtocolObject<dyn MTLSamplerState>>,
        binding: u32,
        vertex: bool,
        fragment: bool,
    },
}

/// A bind group: resources resolved to the Metal objects they wrap.
///
/// The wrapped handles are retained, so a bound resource cannot be freed while
/// a group still refers to it.
#[derive(Clone)]
pub struct MetalBindGroup {
    entries: Vec<ResolvedArgument>,
}

/// A Metal command encoder: a command buffer plus the blit encoder lazily
/// opened on it.
///
/// Metal issues a distinct encoder object per pass type where wgpu offers one
/// mutable encoder, so the blit encoder is opened on demand and closed before
/// any render pass begins or the buffer is submitted.
pub struct MetalCommandEncoder {
    command_buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    blit: Option<Retained<ProtocolObject<dyn MTLBlitCommandEncoder>>>,
}

impl MetalCommandEncoder {
    /// Returns the open blit encoder, opening one if needed.
    fn blit_encoder(&mut self) -> &Retained<ProtocolObject<dyn MTLBlitCommandEncoder>> {
        if self.blit.is_none() {
            let opened = self
                .command_buffer
                .blitCommandEncoder()
                .expect("the command queue to hand out a blit encoder");
            self.blit = Some(opened);
        }
        self.blit.as_ref().expect("just opened")
    }

    /// Close the blit encoder if one is open.
    fn end_blit(&mut self) {
        if let Some(blit) = self.blit.take() {
            blit.endEncoding();
        }
    }
}

/// A recording Metal render pass.
///
/// Ends its Metal encoder when dropped. That is what makes the trait's
/// "submit consumes the encoder" rule safe: a live pass borrows the encoder
/// mutably for its whole lifetime, so it cannot still be recording when its
/// command buffer is submitted.
pub struct MetalRenderPass<'a> {
    encoder: Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>,
    /// Metal has no index bind point, so `set_index_buffer` records state for
    /// `draw_indexed` to fold into its own arguments.
    index: Option<(Retained<ProtocolObject<dyn MTLBuffer>>, MTLIndexType, u64)>,
    topology: MTLPrimitiveType,
    _encoder: std::marker::PhantomData<&'a mut MetalCommandEncoder>,
}

impl Drop for MetalRenderPass<'_> {
    fn drop(&mut self) {
        self.encoder.endEncoding();
    }
}

// Safety: see the module note. Each newtype is a reference-counted handle to a
// Metal object that Apple documents as thread-safe, with no additional
// invariants maintained in Rust.
unsafe impl Send for MetalBackend {}
unsafe impl Sync for MetalBackend {}
unsafe impl Send for MetalBuffer {}
unsafe impl Sync for MetalBuffer {}
unsafe impl Send for MetalTexture {}
unsafe impl Sync for MetalTexture {}
unsafe impl Send for MetalTextureView {}
unsafe impl Sync for MetalTextureView {}
unsafe impl Send for MetalSampler {}
unsafe impl Sync for MetalSampler {}
unsafe impl Send for MetalShaderModule {}
unsafe impl Sync for MetalShaderModule {}
unsafe impl Send for MetalRenderPipeline {}
unsafe impl Sync for MetalRenderPipeline {}
unsafe impl Send for MetalBindGroup {}
unsafe impl Sync for MetalBindGroup {}
unsafe impl Send for MetalBindGroupLayout {}
unsafe impl Sync for MetalBindGroupLayout {}
unsafe impl Send for MetalPipelineLayout {}
unsafe impl Sync for MetalPipelineLayout {}

// ── GpuBackend ────────────────────────────────────────────────────────────

impl GpuBackend for MetalBackend {
    type Buffer = MetalBuffer;
    type Texture = MetalTexture;
    type TextureView = MetalTextureView;
    type BindGroupLayout = MetalBindGroupLayout;
    type BindGroup = MetalBindGroup;
    type PipelineLayout = MetalPipelineLayout;
    type RenderPipeline = MetalRenderPipeline;
    type Sampler = MetalSampler;
    type ShaderModule = MetalShaderModule;
    type CommandEncoder = MetalCommandEncoder;
    type TextureFormat = MTLPixelFormat;

    fn r8_unorm_format() -> Self::TextureFormat {
        MTLPixelFormat::R8Unorm
    }

    fn rgba8_unorm_format() -> Self::TextureFormat {
        MTLPixelFormat::RGBA8Unorm
    }

    type RenderPass<'a> = MetalRenderPass<'a>;

    fn rect_shader_source(&self) -> &'static [u8] {
        include_str!("../pipeline/shaders/metal/rect.metal").as_bytes()
    }

    fn builtin_shader_source(&self, shader: BuiltinShader) -> &'static [u8] {
        match shader {
            BuiltinShader::Image => concat!(
                include_str!("../pipeline/shaders/metal/color.metal"),
                include_str!("../pipeline/shaders/metal/image.metal")
            )
            .as_bytes(),
            BuiltinShader::Text => concat!(
                include_str!("../pipeline/shaders/metal/text_common.metal"),
                include_str!("../pipeline/shaders/metal/text.metal")
            )
            .as_bytes(),
            BuiltinShader::TextColor => concat!(
                include_str!("../pipeline/shaders/metal/text_common.metal"),
                include_str!("../pipeline/shaders/metal/text_color.metal")
            )
            .as_bytes(),
            BuiltinShader::TextDecoration => concat!(
                include_str!("../pipeline/shaders/metal/text_common.metal"),
                include_str!("../pipeline/shaders/metal/text_decoration.metal")
            )
            .as_bytes(),
            BuiltinShader::Svg => concat!(
                include_str!("../pipeline/shaders/metal/color.metal"),
                include_str!("../pipeline/shaders/metal/svg.metal")
            )
            .as_bytes(),
            BuiltinShader::FrameComposite => {
                include_str!("../pipeline/frame_composite.metal").as_bytes()
            }
            BuiltinShader::Material => {
                include_str!("../pipeline/shaders/metal/material.metal").as_bytes()
            }
            BuiltinShader::Morph => {
                include_str!("../pipeline/shaders/metal/morph.metal").as_bytes()
            }
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
        let text = std::str::from_utf8(source).map_err(|error| BackendError {
            operation: "create_shader_module",
            message: format!("{label} is not valid UTF-8 MSL: {error}"),
        })?;
        Ok(MetalShaderModule(self.compile_msl(text, label)?))
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> Self::Buffer {
        let length = NSUInteger::try_from(desc.size)
            .expect("buffer size to fit an NSUInteger on this platform");
        let buffer = self
            .device
            .newBufferWithLength_options(length, MTLResourceOptions::StorageModeShared)
            .unwrap_or_else(|| {
                panic!(
                    "the device refused to create a {} byte buffer ({:?})",
                    desc.size, desc.label
                )
            });
        if let Some(label) = &desc.label {
            buffer.setLabel(Some(&NSString::from_str(label)));
        }
        MetalBuffer(buffer)
    }

    fn create_texture(&self, desc: &TextureDescriptor<Self::TextureFormat>) -> Self::Texture {
        let descriptor = MTLTextureDescriptor::new();
        descriptor.setTextureType(match desc.dimension {
            TextureDimension::D1 => MTLTextureType::Type1D,
            TextureDimension::D2 => {
                if desc.sample_count > 1 {
                    MTLTextureType::Type2DMultisample
                } else {
                    MTLTextureType::Type2D
                }
            }
            TextureDimension::D3 => MTLTextureType::Type3D,
        });
        descriptor.setPixelFormat(desc.format);
        unsafe {
            descriptor.setWidth(desc.size.0 as NSUInteger);
            descriptor.setHeight(desc.size.1 as NSUInteger);
            descriptor.setDepth(desc.size.2.max(1) as NSUInteger);
            descriptor.setMipmapLevelCount(desc.mip_level_count.max(1) as NSUInteger);
            descriptor.setSampleCount(desc.sample_count.max(1) as NSUInteger);
        }
        descriptor.setUsage(texture_usage(&desc.usage));
        descriptor.setStorageMode(if needs_cpu_access(&desc.usage) {
            MTLStorageMode::Shared
        } else {
            MTLStorageMode::Private
        });
        let texture = self
            .device
            .newTextureWithDescriptor(&descriptor)
            .unwrap_or_else(|| {
                panic!(
                    "the device refused to create a {:?} texture of {}x{}",
                    desc.format, desc.size.0, desc.size.1
                )
            });
        // The label lives on the resource, not on the descriptor used to make it.
        if let Some(label) = &desc.label {
            texture.setLabel(Some(&NSString::from_str(label)));
        }
        MetalTexture(texture)
    }

    fn create_texture_view(&self, texture: &Self::Texture, _label: &str) -> Self::TextureView {
        MetalTextureView(texture.0.clone())
    }

    fn create_sampler(&self, desc: &SamplerDescriptor) -> Self::Sampler {
        let descriptor = MTLSamplerDescriptor::new();
        descriptor.setMinFilter(map_filter(desc.min_filter));
        descriptor.setMagFilter(map_filter(desc.mag_filter));
        descriptor.setMipFilter(match desc.mipmap_filter {
            FilterMode::Nearest => MTLSamplerMipFilter::Nearest,
            FilterMode::Linear => MTLSamplerMipFilter::Linear,
        });
        // Metal names the axes S, T and R where the portable descriptors use
        // U, V and W.
        descriptor.setSAddressMode(map_address_mode(desc.address_mode_u));
        descriptor.setTAddressMode(map_address_mode(desc.address_mode_v));
        descriptor.setRAddressMode(map_address_mode(desc.address_mode_w));
        descriptor.setLodMinClamp(desc.lod_min_clamp);
        descriptor.setLodMaxClamp(desc.lod_max_clamp);
        descriptor.setMaxAnisotropy(desc.max_anisotropy as NSUInteger);
        descriptor.setCompareFunction(match desc.compare {
            Some(compare) => map_compare_function(compare),
            None => MTLCompareFunction::Always,
        });
        descriptor.setBorderColor(MTLSamplerBorderColor::TransparentBlack);
        if let Some(label) = &desc.label {
            descriptor.setLabel(Some(&NSString::from_str(label)));
        }
        let sampler = self
            .device
            .newSamplerStateWithDescriptor(&descriptor)
            .unwrap_or_else(|| panic!("the device refused to create the sampler {:?}", desc.label));
        MetalSampler(sampler)
    }

    fn create_bind_group_layout(&self, entries: &[BindGroupLayoutEntry]) -> Self::BindGroupLayout {
        MetalBindGroupLayout {
            entries: entries
                .iter()
                .map(|entry| LayoutEntry {
                    binding: entry.binding,
                    kind: match entry.ty {
                        BindingType::Buffer { .. } => ArgumentKind::Buffer,
                        BindingType::Texture { .. } | BindingType::StorageTexture { .. } => {
                            ArgumentKind::Texture
                        }
                        BindingType::Sampler(_) => ArgumentKind::Sampler,
                    },
                    vertex: visible_to(entry, ShaderStage::Vertex),
                    fragment: visible_to(entry, ShaderStage::Fragment),
                    dynamic_offset: matches!(
                        entry.ty,
                        BindingType::Buffer {
                            has_dynamic_offset: true,
                            ..
                        }
                    ),
                })
                .collect(),
        }
    }

    fn create_bind_group(
        &self,
        layout: &Self::BindGroupLayout,
        entries: &[BindGroupEntry<Self>],
    ) -> Self::BindGroup {
        let mut resolved = Vec::with_capacity(entries.len());
        for entry in entries {
            let plan = layout
                .entries
                .iter()
                .find(|plan| plan.binding == entry.binding)
                .unwrap_or_else(|| {
                    panic!(
                        "binding {} is not declared by the layout it was used with",
                        entry.binding
                    )
                });
            let argument = match (&entry.resource, plan.kind) {
                (BindingResource::Buffer(buffer), ArgumentKind::Buffer) => {
                    ResolvedArgument::Buffer {
                        buffer: buffer.0.clone(),
                        offset: 0,
                        dynamic_offset: plan.dynamic_offset,
                        binding: entry.binding,
                        vertex: plan.vertex,
                        fragment: plan.fragment,
                    }
                }
                (BindingResource::BufferRange(buffer, offset, _size), ArgumentKind::Buffer) => {
                    ResolvedArgument::Buffer {
                        buffer: buffer.0.clone(),
                        offset: *offset,
                        dynamic_offset: plan.dynamic_offset,
                        binding: entry.binding,
                        vertex: plan.vertex,
                        fragment: plan.fragment,
                    }
                }
                (BindingResource::TextureView(view), ArgumentKind::Texture) => {
                    ResolvedArgument::Texture {
                        texture: view.0.clone(),
                        binding: entry.binding,
                        fragment: plan.fragment,
                    }
                }
                (BindingResource::Sampler(sampler), ArgumentKind::Sampler) => {
                    ResolvedArgument::Sampler {
                        sampler: sampler.0.clone(),
                        binding: entry.binding,
                        vertex: plan.vertex,
                        fragment: plan.fragment,
                    }
                }
                (_, kind) => panic!(
                    "bind group entry {} does not match the layout's {:?} slot",
                    entry.binding, kind
                ),
            };
            resolved.push(argument);
        }
        MetalBindGroup { entries: resolved }
    }

    fn create_pipeline_layout(&self, layouts: &[&Self::BindGroupLayout]) -> Self::PipelineLayout {
        assert!(
            layouts.len() <= binding::MAX_BIND_GROUPS as usize,
            "{} bind groups exceeds the {} the argument index rule can address",
            layouts.len(),
            binding::MAX_BIND_GROUPS
        );
        MetalPipelineLayout
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
        let descriptor = MTLRenderPipelineDescriptor::new();
        if let Some(label) = &desc.label {
            descriptor.setLabel(Some(&NSString::from_str(label)));
        }

        let vertex_function = self.entry_point(&desc.vertex)?;
        descriptor.setVertexFunction(Some(&vertex_function));

        if let Some(fragment) = &desc.fragment {
            let fragment_function =
                self.entry_point_for(&fragment.module.0, fragment.entry_point)?;
            descriptor.setFragmentFunction(Some(&fragment_function));
            for (index, target) in fragment.targets.iter().enumerate() {
                let Some(target) = target else { continue };
                let attachment = unsafe {
                    descriptor
                        .colorAttachments()
                        .objectAtIndexedSubscript(index)
                };
                configure_color_target(&attachment, target);
            }
        }

        if let Some(vertex_descriptor) = build_vertex_descriptor(desc.vertex.buffers) {
            descriptor.setVertexDescriptor(Some(&vertex_descriptor));
        }

        descriptor.setRasterSampleCount(desc.multisample.count.max(1) as NSUInteger);
        descriptor.setAlphaToCoverageEnabled(desc.multisample.alpha_to_coverage_enabled);

        let state = self
            .device
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .map_err(|error| BackendError {
                operation: "create_render_pipeline",
                message: format!("{}: {}", desc.label.as_deref().unwrap_or("pipeline"), error),
            })?;

        Ok(MetalRenderPipeline {
            state,
            topology: map_topology(desc.primitive.topology),
        })
    }

    fn write_buffer(&self, buffer: &Self::Buffer, offset: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let start = offset as usize;
        let length = buffer.0.length();
        assert!(
            start.saturating_add(data.len()) <= length,
            "write of {} bytes at offset {start} overruns a {length} byte Metal buffer",
            data.len()
        );
        let mut queue_state = self
            .queue_state
            .lock()
            .expect("Metal upload queue mutex is not poisoned");
        let source = self.staging_buffer(data, "Metal buffer upload");
        queue_state.uploads.push(PendingUpload::Buffer {
            source,
            destination: buffer.clone(),
            destination_offset: offset,
            size: data.len() as u64,
        });
    }

    fn write_texture(&self, desc: &WriteTextureDescriptor<Self>) {
        assert!(
            !desc.data.is_empty(),
            "write_texture needs at least one byte of pixel data"
        );
        assert_eq!(desc.aspect, crate::backend::TextureAspect::All);
        if desc.extent.width == 0
            || desc.extent.height == 0
            || desc.extent.depth_or_array_layers == 0
        {
            return;
        }

        let bytes_per_pixel = bytes_per_pixel(desc.texture.0.pixelFormat()) as usize;
        let row_bytes = (desc.extent.width as usize)
            .checked_mul(bytes_per_pixel)
            .expect("texture upload row size overflowed");
        let source_bytes_per_row = desc
            .buffer_layout
            .bytes_per_row
            .map(|bytes_per_row| bytes_per_row as usize)
            .unwrap_or(row_bytes);
        assert!(
            source_bytes_per_row >= row_bytes,
            "texture upload row stride is smaller than the copied row"
        );
        let source_rows_per_image = desc
            .buffer_layout
            .rows_per_image
            .map(|rows| rows as usize)
            .unwrap_or(desc.extent.height as usize);
        assert!(
            source_rows_per_image >= desc.extent.height as usize,
            "texture upload image stride is smaller than the copied image"
        );

        let destination_bytes_per_row = align_texture_upload_row(row_bytes);
        let destination_bytes_per_image = destination_bytes_per_row
            .checked_mul(desc.extent.height as usize)
            .expect("texture upload image size overflowed");
        let staging_size = destination_bytes_per_image
            .checked_mul(desc.extent.depth_or_array_layers as usize)
            .expect("texture upload size overflowed");
        let source_image_stride = source_bytes_per_row
            .checked_mul(source_rows_per_image)
            .expect("source texture image stride overflowed");
        let source_offset = usize::try_from(desc.buffer_layout.offset)
            .expect("texture upload offset to fit usize on this platform");
        let last_source_byte = source_offset
            .checked_add(
                (desc.extent.depth_or_array_layers.saturating_sub(1) as usize)
                    .checked_mul(source_image_stride)
                    .expect("source texture layer offset overflowed"),
            )
            .and_then(|last_layer| {
                last_layer.checked_add(
                    (desc.extent.height.saturating_sub(1) as usize)
                        .checked_mul(source_bytes_per_row)?
                        .checked_add(row_bytes)?,
                )
            })
            .expect("source texture extent overflowed");
        assert!(
            last_source_byte <= desc.data.len(),
            "texture upload data does not contain all rows in the requested extent"
        );

        let mut packed = vec![0; staging_size];
        for layer in 0..desc.extent.depth_or_array_layers as usize {
            for row in 0..desc.extent.height as usize {
                let source_start =
                    source_offset + layer * source_image_stride + row * source_bytes_per_row;
                let destination_start =
                    layer * destination_bytes_per_image + row * destination_bytes_per_row;
                packed[destination_start..destination_start + row_bytes]
                    .copy_from_slice(&desc.data[source_start..source_start + row_bytes]);
            }
        }

        let mut queue_state = self
            .queue_state
            .lock()
            .expect("Metal upload queue mutex is not poisoned");
        let source = self.staging_buffer(&packed, "Metal texture upload");
        queue_state.uploads.push(PendingUpload::Texture {
            source,
            destination: desc.texture.clone(),
            mip_level: desc.mip_level,
            origin: desc.origin,
            extent: desc.extent,
            bytes_per_row: destination_bytes_per_row as u32,
            bytes_per_image: if desc.extent.depth_or_array_layers > 1 {
                destination_bytes_per_image as u32
            } else {
                0
            },
        });
    }

    fn create_command_encoder(&self, label: &str) -> Self::CommandEncoder {
        let command_buffer = self
            .queue
            .commandBuffer()
            .expect("the command queue to hand out a command buffer");
        command_buffer.setLabel(Some(&NSString::from_str(label)));
        let mut encoder = MetalCommandEncoder {
            command_buffer,
            blit: None,
        };
        let mut queue_state = self
            .queue_state
            .lock()
            .expect("Metal upload queue mutex is not poisoned");
        if !queue_state.uploads.is_empty() {
            let blit = encoder.blit_encoder();
            Self::encode_pending_uploads(blit, &mut queue_state.uploads);
        }
        encoder
    }

    fn begin_render_pass<'a>(
        &self,
        encoder: &'a mut Self::CommandEncoder,
        desc: &RenderPassDescriptor<Self>,
    ) -> Self::RenderPass<'a> {
        // Metal permits a single open encoder per command buffer.
        encoder.end_blit();

        let pass_descriptor = MTLRenderPassDescriptor::new();
        for (index, attachment) in desc.color_attachments.iter().enumerate() {
            let slot = unsafe {
                pass_descriptor
                    .colorAttachments()
                    .objectAtIndexedSubscript(index)
            };
            configure_color_attachment(&slot, attachment);
        }
        if let Some(depth) = &desc.depth_stencil_attachment {
            configure_depth_attachment(&pass_descriptor, depth);
        }

        let recording = encoder
            .command_buffer
            .renderCommandEncoderWithDescriptor(&pass_descriptor)
            .expect("the command buffer to hand out a render encoder");

        MetalRenderPass {
            encoder: recording,
            index: None,
            topology: MTLPrimitiveType::Triangle,
            _encoder: std::marker::PhantomData,
        }
    }

    fn copy_buffer_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyBufferInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        let bytes_per_row = src.layout.bytes_per_row.unwrap_or(0) as NSUInteger;
        let bytes_per_image = if extent.depth_or_array_layers > 1 {
            bytes_per_row * src.layout.rows_per_image.unwrap_or(0) as NSUInteger
        } else {
            0
        };
        let blit = encoder.blit_encoder();
        unsafe {
            blit.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                &src.buffer.0,
                src.layout.offset as NSUInteger,
                bytes_per_row,
                bytes_per_image,
                mtl_size(extent),
                &dst.texture.0,
                dst.origin.z as NSUInteger,
                dst.mip_level as NSUInteger,
                mtl_origin(dst.origin),
            );
        }
    }

    fn copy_texture_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        let blit = encoder.blit_encoder();
        unsafe {
            blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                &src.texture.0,
                src.origin.z as NSUInteger,
                src.mip_level as NSUInteger,
                mtl_origin(src.origin),
                mtl_size(extent),
                &dst.texture.0,
                dst.origin.z as NSUInteger,
                dst.mip_level as NSUInteger,
                mtl_origin(dst.origin),
            );
        }
    }

    fn copy_texture_to_buffer(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyBufferInfo<Self>,
        extent: Extent3d,
    ) {
        let bytes_per_row = dst.layout.bytes_per_row.unwrap_or(0) as NSUInteger;
        let blit = encoder.blit_encoder();
        unsafe {
            blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage_options(
                &src.texture.0,
                src.origin.z as NSUInteger,
                src.mip_level as NSUInteger,
                mtl_origin(src.origin),
                mtl_size(extent),
                &dst.buffer.0,
                dst.layout.offset as NSUInteger,
                bytes_per_row,
                bytes_per_row * dst.layout.rows_per_image.unwrap_or(0) as NSUInteger,
                MTLBlitOption::None,
            );
        }
    }

    fn read_buffer(&self, buffer: &Self::Buffer, size: u64) -> Vec<u8> {
        // Command buffers issued by one queue retire in submission order, so
        // committing an empty buffer and waiting for it returns only after
        // everything already submitted has finished. That keeps this correct
        // without the backend having to remember its last submission.
        let mut queue_state = self
            .queue_state
            .lock()
            .expect("Metal upload queue mutex is not poisoned");
        self.commit_pending_uploads(&mut queue_state.uploads);
        let fence = self
            .queue
            .commandBuffer()
            .expect("the command queue to hand out a fence command buffer");
        fence.commit();
        drop(queue_state);
        fence.waitUntilCompleted();

        let length = buffer.0.length();
        assert!(
            size as usize <= length,
            "readback of {size} bytes exceeds a {length} byte Metal buffer"
        );
        let mut bytes = vec![0u8; size as usize];
        unsafe {
            std::ptr::copy_nonoverlapping(
                buffer.0.contents().cast::<u8>().as_ptr(),
                bytes.as_mut_ptr(),
                bytes.len(),
            );
        }
        bytes
    }

    fn submit(&self, encoder: Self::CommandEncoder) {
        // A render pass derived from this encoder was dropped before the borrow
        // ended, so its encoder is already closed. The blit encoder is the one
        // this type owns directly.
        let mut queue_state = self
            .queue_state
            .lock()
            .expect("Metal upload queue mutex is not poisoned");
        self.commit_pending_uploads(&mut queue_state.uploads);
        let mut encoder = encoder;
        encoder.end_blit();
        #[cfg(feature = "frame-stats")]
        if let Some(capture) = self
            .active_gpu_frame
            .lock()
            .expect("Metal active GPU frame state is not poisoned")
            .as_ref()
            .cloned()
        {
            capture.register_submission();
            let callback_capture = capture.clone();
            let callback = RcBlock::new(
                move |command_buffer: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                    // SAFETY: Metal invokes the handler with a live command buffer.
                    let command_buffer = unsafe { command_buffer.as_ref() };
                    callback_capture.complete_submission(
                        command_buffer.GPUStartTime(),
                        command_buffer.GPUEndTime(),
                    );
                },
            );
            // SAFETY: `callback` owns a valid block pointer for this call, and
            // Metal retains the completion handler until it has invoked it.
            unsafe {
                encoder
                    .command_buffer
                    .addCompletedHandler(RcBlock::as_ptr(&callback));
            }
        }
        encoder.command_buffer.commit();
        // Reuse the existing queue lock and transfer ownership: no extra lock,
        // allocation or retain is needed to remember this frame's GPU producer.
        queue_state.last_submission = Some(encoder.command_buffer);
    }

    fn limits(&self) -> GpuLimits {
        GpuLimits {
            // Metal's documented 2D ceiling on every part this backend supports.
            max_texture_dimension_2d: 16_384,
            // Metal has no uniform bind point, so bindings are plain device
            // buffers; 256 is the alignment the argument-table conventions here
            // are built around.
            min_uniform_buffer_offset_alignment: 256,
        }
    }

    fn format_is_srgb(format: Self::TextureFormat) -> bool {
        format == MTLPixelFormat::RGBA8Unorm_sRGB || format == MTLPixelFormat::BGRA8Unorm_sRGB
    }
}

impl MetalBackend {
    /// Fetch the vertex entry point named by `state`.
    fn entry_point(
        &self,
        state: &crate::backend::VertexState<'_, Self>,
    ) -> Result<Retained<ProtocolObject<dyn MTLFunction>>, BackendError> {
        self.entry_point_for(&state.module.0, state.entry_point)
    }

    /// Fetch a named entry point from a compiled library.
    fn entry_point_for(
        &self,
        library: &Retained<ProtocolObject<dyn MTLLibrary>>,
        name: &str,
    ) -> Result<Retained<ProtocolObject<dyn MTLFunction>>, BackendError> {
        library
            .newFunctionWithName(&NSString::from_str(name))
            .ok_or_else(|| BackendError {
                operation: "create_render_pipeline",
                message: format!("the Metal library exports no entry point named `{name}`"),
            })
    }
}

// ── GpuRenderPass ──────────────────────────────────────────────────────────

impl GpuRenderPass<MetalBackend> for MetalRenderPass<'_> {
    fn set_pipeline(&mut self, pipeline: &MetalRenderPipeline) {
        self.encoder.setRenderPipelineState(&pipeline.state);
        self.topology = pipeline.topology;
    }

    fn set_bind_group(&mut self, index: u32, bind_group: &MetalBindGroup, dynamic_offsets: &[u32]) {
        let encoder = &self.encoder;
        let mut next_offset = 0usize;
        for entry in &bind_group.entries {
            match entry {
                ResolvedArgument::Buffer {
                    buffer,
                    offset,
                    dynamic_offset,
                    binding,
                    vertex,
                    fragment,
                } => {
                    let mut base = *offset;
                    if *dynamic_offset {
                        let extra =
                            dynamic_offsets
                                .get(next_offset)
                                .copied()
                                .unwrap_or_else(|| {
                                    panic!(
                                        "group {index} binding {binding} needs a dynamic offset, \
                                         but only {} were supplied",
                                        dynamic_offsets.len()
                                    )
                                });
                        next_offset += 1;
                        base += extra as u64;
                    }
                    let slot = binding::argument_index(index, *binding) as NSUInteger;
                    let argument: &ProtocolObject<dyn MTLBuffer> = buffer;
                    if *vertex {
                        unsafe {
                            encoder.setVertexBuffer_offset_atIndex(
                                Some(argument),
                                base as NSUInteger,
                                slot,
                            )
                        };
                    }
                    if *fragment {
                        unsafe {
                            encoder.setFragmentBuffer_offset_atIndex(
                                Some(argument),
                                base as NSUInteger,
                                slot,
                            )
                        };
                    }
                }
                ResolvedArgument::Texture {
                    texture,
                    binding,
                    fragment,
                } => {
                    let slot = binding::argument_index(index, *binding) as NSUInteger;
                    let argument: &ProtocolObject<dyn MTLTexture> = texture;
                    if *fragment {
                        unsafe { encoder.setFragmentTexture_atIndex(Some(argument), slot) };
                    }
                }
                ResolvedArgument::Sampler {
                    sampler,
                    binding,
                    vertex,
                    fragment,
                } => {
                    let slot = binding::argument_index(index, *binding) as NSUInteger;
                    let argument: &ProtocolObject<dyn MTLSamplerState> = sampler;
                    if *vertex {
                        unsafe { encoder.setVertexSamplerState_atIndex(Some(argument), slot) };
                    }
                    if *fragment {
                        unsafe { encoder.setFragmentSamplerState_atIndex(Some(argument), slot) };
                    }
                }
            }
        }
    }

    fn set_vertex_buffer(&mut self, slot: u32, buffer: &MetalBuffer, offset: u64) {
        unsafe {
            self.encoder.setVertexBuffer_offset_atIndex(
                Some(&buffer.0),
                offset as NSUInteger,
                binding::vertex_buffer_index(slot) as NSUInteger,
            )
        };
    }

    fn set_index_buffer(&mut self, buffer: &MetalBuffer, index_format: IndexFormat, offset: u64) {
        let index_type = match index_format {
            IndexFormat::Uint16 => MTLIndexType::UInt16,
            IndexFormat::Uint32 => MTLIndexType::UInt32,
        };
        self.index = Some((buffer.0.clone(), index_type, offset));
    }

    fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.encoder.setScissorRect(MTLScissorRect {
            x: x as NSUInteger,
            y: y as NSUInteger,
            width: width as NSUInteger,
            height: height as NSUInteger,
        });
    }

    fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        let count = (vertices.end - vertices.start) as NSUInteger;
        let instance_count = (instances.end - instances.start) as NSUInteger;
        let base_instance = instances.start as NSUInteger;
        let (encoder, topology, start) =
            (&self.encoder, self.topology, vertices.start as NSUInteger);
        unsafe {
            if base_instance != 0 {
                encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                    topology,
                    start,
                    count,
                    instance_count,
                    base_instance,
                );
            } else if instance_count != 1 {
                encoder.drawPrimitives_vertexStart_vertexCount_instanceCount(
                    topology,
                    start,
                    count,
                    instance_count,
                );
            } else {
                encoder.drawPrimitives_vertexStart_vertexCount(topology, start, count);
            }
        }
    }

    fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        let (buffer, index_type, offset) = self
            .index
            .as_ref()
            .expect("set_index_buffer before draw_indexed");
        let encoder = &self.encoder;
        let topology = self.topology;
        unsafe {
            encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset_instanceCount_baseVertex_baseInstance(
                topology,
                (indices.end - indices.start) as NSUInteger,
                *index_type,
                buffer,
                (offset + indices.start as u64) as NSUInteger,
                (instances.end - instances.start) as NSUInteger,
                base_vertex as _,
                instances.start as NSUInteger,
            );
        }
    }
}

// ── Conversion helpers ────────────────────────────────────────────────────

fn visible_to(entry: &BindGroupLayoutEntry, stage: ShaderStage) -> bool {
    entry.visibility.iter().any(|visible| *visible == stage)
}

fn needs_cpu_access(usages: &[TextureUsage]) -> bool {
    usages.contains(&TextureUsage::CopySrc) || usages.contains(&TextureUsage::CopyDst)
}

fn texture_usage(usages: &[TextureUsage]) -> MTLTextureUsage {
    let mut result = MTLTextureUsage::Unknown;
    result.set(
        MTLTextureUsage::RenderTarget,
        usages.contains(&TextureUsage::RenderAttachment),
    );
    result.set(
        MTLTextureUsage::ShaderRead,
        usages.contains(&TextureUsage::TextureBinding) || usages.contains(&TextureUsage::CopySrc),
    );
    result.set(
        MTLTextureUsage::ShaderWrite,
        usages.contains(&TextureUsage::StorageBinding),
    );
    result
}

/// Bytes per texel for the formats this backend allocates.
///
/// Depth and packed formats are not read back or replaced through
/// `write_texture`, so a missing entry is an explicit bug rather than a silent
/// stride of zero.
fn bytes_per_pixel(format: MTLPixelFormat) -> u32 {
    match format {
        MTLPixelFormat::R8Unorm | MTLPixelFormat::A8Unorm => 1,
        MTLPixelFormat::RG8Unorm => 2,
        MTLPixelFormat::RGBA8Unorm
        | MTLPixelFormat::RGBA8Unorm_sRGB
        | MTLPixelFormat::BGRA8Unorm
        | MTLPixelFormat::BGRA8Unorm_sRGB => 4,
        other => unimplemented!("no byte width known for Metal pixel format {other:?}"),
    }
}

fn map_filter(filter: FilterMode) -> MTLSamplerMinMagFilter {
    match filter {
        FilterMode::Nearest => MTLSamplerMinMagFilter::Nearest,
        FilterMode::Linear => MTLSamplerMinMagFilter::Linear,
    }
}

fn map_address_mode(mode: AddressMode) -> MTLSamplerAddressMode {
    match mode {
        AddressMode::ClampToEdge => MTLSamplerAddressMode::ClampToEdge,
        AddressMode::Repeat => MTLSamplerAddressMode::Repeat,
        AddressMode::MirrorRepeat => MTLSamplerAddressMode::MirrorRepeat,
    }
}

fn map_compare_function(compare: crate::backend::CompareFunction) -> MTLCompareFunction {
    use crate::backend::CompareFunction as Cmp;
    match compare {
        Cmp::Never => MTLCompareFunction::Never,
        Cmp::Less => MTLCompareFunction::Less,
        Cmp::Equal => MTLCompareFunction::Equal,
        Cmp::LessEqual => MTLCompareFunction::LessEqual,
        Cmp::Greater => MTLCompareFunction::Greater,
        Cmp::NotEqual => MTLCompareFunction::NotEqual,
        Cmp::GreaterEqual => MTLCompareFunction::GreaterEqual,
        Cmp::Always => MTLCompareFunction::Always,
    }
}

fn mtl_origin(origin: Origin3d) -> MTLOrigin {
    MTLOrigin {
        x: origin.x as NSUInteger,
        y: origin.y as NSUInteger,
        z: origin.z as NSUInteger,
    }
}

fn mtl_size(extent: Extent3d) -> MTLSize {
    MTLSize {
        width: extent.width as NSUInteger,
        height: extent.height as NSUInteger,
        depth: extent.depth_or_array_layers.max(1) as NSUInteger,
    }
}

#[inline]
fn align_texture_upload_row(row_bytes: usize) -> usize {
    row_bytes
        .checked_add(255)
        .expect("texture upload row size overflowed")
        & !255
}

fn map_topology(topology: crate::backend::PrimitiveTopology) -> MTLPrimitiveType {
    use crate::backend::PrimitiveTopology as Topology;
    match topology {
        Topology::PointList => MTLPrimitiveType::Point,
        Topology::LineList => MTLPrimitiveType::Line,
        Topology::LineStrip => MTLPrimitiveType::LineStrip,
        Topology::TriangleList => MTLPrimitiveType::Triangle,
        Topology::TriangleStrip => MTLPrimitiveType::TriangleStrip,
    }
}

fn map_blend_operation(operation: crate::backend::BlendOperation) -> MTLBlendOperation {
    use crate::backend::BlendOperation as Op;
    match operation {
        Op::Add => MTLBlendOperation::Add,
        Op::Subtract => MTLBlendOperation::Subtract,
        Op::ReverseSubtract => MTLBlendOperation::ReverseSubtract,
        Op::Min => MTLBlendOperation::Min,
        Op::Max => MTLBlendOperation::Max,
    }
}

fn map_blend_factor(factor: crate::backend::BlendFactor) -> MTLBlendFactor {
    use crate::backend::BlendFactor as F;
    match factor {
        F::Zero => MTLBlendFactor::Zero,
        F::One => MTLBlendFactor::One,
        F::Src => MTLBlendFactor::SourceColor,
        F::OneMinusSrc => MTLBlendFactor::OneMinusSourceColor,
        F::SrcAlpha => MTLBlendFactor::SourceAlpha,
        F::OneMinusSrcAlpha => MTLBlendFactor::OneMinusSourceAlpha,
        F::Dst => MTLBlendFactor::DestinationColor,
        F::OneMinusDst => MTLBlendFactor::OneMinusDestinationColor,
        F::DstAlpha => MTLBlendFactor::DestinationAlpha,
        F::OneMinusDstAlpha => MTLBlendFactor::OneMinusDestinationAlpha,
        F::SrcAlphaSaturated => MTLBlendFactor::SourceAlphaSaturated,
        F::Constant => MTLBlendFactor::BlendColor,
        F::OneMinusConstant => MTLBlendFactor::OneMinusBlendColor,
    }
}

fn configure_color_target(
    attachment: &MTLRenderPipelineColorAttachmentDescriptor,
    target: &ColorTargetState<MTLPixelFormat>,
) {
    attachment.setPixelFormat(target.format);
    attachment.setWriteMask(
        if target.write_mask == crate::backend::ColorWriteMask::NONE {
            MTLColorWriteMask::None
        } else {
            MTLColorWriteMask::All
        },
    );
    let Some(blend) = &target.blend else {
        attachment.setBlendingEnabled(false);
        return;
    };
    attachment.setBlendingEnabled(true);
    attachment.setRgbBlendOperation(map_blend_operation(blend.color.operation));
    attachment.setSourceRGBBlendFactor(map_blend_factor(blend.color.src_factor));
    attachment.setDestinationRGBBlendFactor(map_blend_factor(blend.color.dst_factor));
    attachment.setAlphaBlendOperation(map_blend_operation(blend.alpha.operation));
    attachment.setSourceAlphaBlendFactor(map_blend_factor(blend.alpha.src_factor));
    attachment.setDestinationAlphaBlendFactor(map_blend_factor(blend.alpha.dst_factor));
}

fn configure_color_attachment(
    attachment: &MTLRenderPassColorAttachmentDescriptor,
    source: &RenderPassColorAttachment<'_, MetalBackend>,
) {
    attachment.setTexture(Some(&source.view.0));
    if let Some(resolve) = source.resolve_target {
        attachment.setResolveTexture(Some(&resolve.0));
    }
    match source.ops.load {
        LoadOp::Load => attachment.setLoadAction(MTLLoadAction::Load),
        LoadOp::Clear([red, green, blue, alpha]) => {
            attachment.setClearColor(MTLClearColor {
                red,
                green,
                blue,
                alpha,
            });
            attachment.setLoadAction(MTLLoadAction::Clear);
        }
    }
    attachment.setStoreAction(match (source.resolve_target.is_some(), source.ops.store) {
        (true, StoreOp::Store) => MTLStoreAction::StoreAndMultisampleResolve,
        (true, StoreOp::Discard) => MTLStoreAction::MultisampleResolve,
        (false, StoreOp::Store) => MTLStoreAction::Store,
        (false, StoreOp::Discard) => MTLStoreAction::DontCare,
    });
}

/// Fill in Metal's depth and stencil attachments for one portable
/// depth-stencil attachment.
///
/// Metal keeps depth and stencil as two separate attachment descriptors sharing
/// one texture, where the portable model has one attachment carrying both sets
/// of operations. Either side may be absent, in which case its descriptor is
/// left untouched and defaults to `DontCare`.
fn configure_depth_attachment(
    pass: &MTLRenderPassDescriptor,
    source: &RenderPassDepthStencilAttachment<'_, MetalBackend>,
) {
    if let Some(ops) = &source.depth_ops {
        let depth = pass.depthAttachment();
        depth.setTexture(Some(&source.view.0));
        match ops.load {
            LoadOp::Load => depth.setLoadAction(MTLLoadAction::Load),
            LoadOp::Clear(value) => {
                depth.setClearDepth(value as f64);
                depth.setLoadAction(MTLLoadAction::Clear);
            }
        }
        depth.setStoreAction(match ops.store {
            StoreOp::Store => MTLStoreAction::Store,
            StoreOp::Discard => MTLStoreAction::DontCare,
        });
    }
    if let Some(ops) = &source.stencil_ops {
        let stencil = pass.stencilAttachment();
        stencil.setTexture(Some(&source.view.0));
        match ops.load {
            LoadOp::Load => stencil.setLoadAction(MTLLoadAction::Load),
            LoadOp::Clear(value) => {
                stencil.setClearStencil(value);
                stencil.setLoadAction(MTLLoadAction::Clear);
            }
        }
        stencil.setStoreAction(match ops.store {
            StoreOp::Store => MTLStoreAction::Store,
            StoreOp::Discard => MTLStoreAction::DontCare,
        });
    }
}

/// Describe `buffers` to Metal's fixed-function vertex fetch.
///
/// Returns `None` when no stream is declared, which leaves Metal taking vertex
/// data from the function arguments instead.
fn build_vertex_descriptor(
    buffers: &[Option<VertexBufferLayout<'_>>],
) -> Option<Retained<MTLVertexDescriptor>> {
    if buffers.iter().all(Option::is_none) {
        return None;
    }
    let descriptor = MTLVertexDescriptor::new();
    for (slot, layout) in buffers.iter().enumerate() {
        let Some(layout) = layout else { continue };
        let buffer_index = binding::vertex_buffer_index(slot as u32);
        let buffer_descriptor = unsafe {
            descriptor
                .layouts()
                .objectAtIndexedSubscript(buffer_index as NSUInteger)
        };
        unsafe {
            buffer_descriptor.setStride(layout.array_stride as NSUInteger);
        }
        buffer_descriptor.setStepFunction(match layout.step_mode {
            VertexStepMode::Vertex => MTLVertexStepFunction::PerVertex,
            VertexStepMode::Instance => MTLVertexStepFunction::PerInstance,
        });
        for attribute in layout.attributes {
            let attribute_descriptor = unsafe {
                descriptor
                    .attributes()
                    .objectAtIndexedSubscript(attribute.shader_location as NSUInteger)
            };
            attribute_descriptor.setFormat(map_vertex_format(attribute.format));
            unsafe {
                attribute_descriptor.setBufferIndex(buffer_index as NSUInteger);
                attribute_descriptor.setOffset(attribute.offset as NSUInteger);
            }
        }
    }
    Some(descriptor)
}

fn map_vertex_format(format: VertexFormat) -> MTLVertexFormat {
    match format {
        VertexFormat::Uint8x2 => MTLVertexFormat::UChar2,
        VertexFormat::Uint8x4 => MTLVertexFormat::UChar4,
        VertexFormat::Sint8x2 => MTLVertexFormat::Char2,
        VertexFormat::Sint8x4 => MTLVertexFormat::Char4,
        VertexFormat::Unorm8x2 => MTLVertexFormat::UChar2Normalized,
        VertexFormat::Unorm8x4 => MTLVertexFormat::UChar4Normalized,
        VertexFormat::Snorm8x2 => MTLVertexFormat::Char2Normalized,
        VertexFormat::Snorm8x4 => MTLVertexFormat::Char4Normalized,
        VertexFormat::Uint16x2 => MTLVertexFormat::UShort2,
        VertexFormat::Uint16x4 => MTLVertexFormat::UShort4,
        VertexFormat::Sint16x2 => MTLVertexFormat::Short2,
        VertexFormat::Sint16x4 => MTLVertexFormat::Short4,
        VertexFormat::Unorm16x2 => MTLVertexFormat::UShort2Normalized,
        VertexFormat::Unorm16x4 => MTLVertexFormat::UShort4Normalized,
        VertexFormat::Snorm16x2 => MTLVertexFormat::Short2Normalized,
        VertexFormat::Snorm16x4 => MTLVertexFormat::Short4Normalized,
        VertexFormat::Float16x2 => MTLVertexFormat::Half2,
        VertexFormat::Float16x4 => MTLVertexFormat::Half4,
        VertexFormat::Float32 => MTLVertexFormat::Float,
        VertexFormat::Float32x2 => MTLVertexFormat::Float2,
        VertexFormat::Float32x3 => MTLVertexFormat::Float3,
        VertexFormat::Float32x4 => MTLVertexFormat::Float4,
        VertexFormat::Uint32 => MTLVertexFormat::UInt,
        VertexFormat::Uint32x2 => MTLVertexFormat::UInt2,
        VertexFormat::Uint32x3 => MTLVertexFormat::UInt3,
        VertexFormat::Uint32x4 => MTLVertexFormat::UInt4,
        VertexFormat::Sint32 => MTLVertexFormat::Int,
        VertexFormat::Sint32x2 => MTLVertexFormat::Int2,
        VertexFormat::Sint32x3 => MTLVertexFormat::Int3,
        VertexFormat::Sint32x4 => MTLVertexFormat::Int4,
    }
}

#[cfg(test)]
mod tests {
    use std::mem::{offset_of, size_of};

    use super::*;
    use crate::backend::{
        BindGroupEntry, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
        BufferBindingType, BufferUsage, ColorWriteMask, Operations, PrimitiveState,
        RenderPassColorAttachment, RenderPassDescriptor, TexelCopyBufferLayout, TextureAspect,
        VertexAttribute, VertexState,
    };
    use crate::damage_region::DamageSet;
    use crate::draw_cmd::DrawList;
    use crate::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderFrame, RenderTree};
    use crate::frame::{Frame, FramePacket, FrameRenderMetadata};
    use crate::rect_pipeline::RectInstance;
    use crate::renderer::RendererImpl;
    use crate::utilities::Rgba8;
    use crate::utilities::{Color, Rect};

    const SIZE: u32 = 64;
    const FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA8Unorm;
    const ROW_BYTES: u32 = SIZE * 4;
    const FRAME_BYTES: u64 = (ROW_BYTES * SIZE) as u64;

    #[test]
    fn surface_uses_command_buffer_compatible_presentation() {
        let Some(backend) = MetalBackend::new() else {
            return;
        };
        let layer = CAMetalLayer::new();
        layer.setPresentsWithTransaction(true);

        let surface = MetalSurface::new(&backend, layer, MTLPixelFormat::BGRA8Unorm_sRGB);

        assert!(!surface.layer().presentsWithTransaction());
    }

    /// A non-clipped, unbordered, unshadowed rect filled with `color`.
    fn instance(x: f32, y: f32, width: f32, height: f32, color: Rgba8) -> RectInstance {
        RectInstance {
            position: [x, y],
            size: [width, height],
            color,
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Rgba8::TRANSPARENT,
            outline_width: [0.0; 4],
            outline_color: Rgba8::TRANSPARENT,
            // A negative width tells the shader that no clip is active.
            clip_rect: [-1.0, -1.0, -1.0, -1.0],
            clip_border_radius: [0.0; 4],
            shadow_params: [0.0; 4],
            shadow_color: Rgba8::TRANSPARENT,
            shadow_flags: [0.0; 4],
        }
    }

    /// The vertex attributes of [`RectInstance`], read from the struct itself.
    ///
    /// Offsets come from `offset_of!` rather than literals so this cannot drift
    /// from the layout the CPU side actually writes.
    fn rect_attributes() -> Vec<VertexAttribute> {
        let float2 = VertexFormat::Float32x2;
        let float4 = VertexFormat::Float32x4;
        let unorm4 = VertexFormat::Unorm8x4;
        let field = |offset: usize, format: VertexFormat, location: u32| VertexAttribute {
            format,
            offset: offset as u64,
            shader_location: location,
        };
        vec![
            field(offset_of!(RectInstance, position), float2, 0),
            field(offset_of!(RectInstance, size), float2, 1),
            field(offset_of!(RectInstance, color), unorm4, 2),
            field(offset_of!(RectInstance, border_radius), float4, 3),
            field(offset_of!(RectInstance, border_width), float4, 4),
            field(offset_of!(RectInstance, border_color), unorm4, 5),
            field(offset_of!(RectInstance, outline_width), float4, 6),
            field(offset_of!(RectInstance, outline_color), unorm4, 7),
            field(offset_of!(RectInstance, clip_rect), float4, 8),
            field(offset_of!(RectInstance, clip_border_radius), float4, 9),
            field(offset_of!(RectInstance, shadow_params), float4, 10),
            field(offset_of!(RectInstance, shadow_color), unorm4, 11),
            field(offset_of!(RectInstance, shadow_flags), float4, 12),
        ]
    }

    fn pixel(data: &[u8], x: u32, y: u32) -> [u8; 4] {
        let offset = (y as usize * ROW_BYTES as usize) + (x as usize * 4);
        [
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]
    }

    fn pixel_with_stride(data: &[u8], row_bytes: u32, x: u32, y: u32) -> [u8; 4] {
        let offset = (y as usize * row_bytes as usize) + (x as usize * 4);
        [
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]
    }

    fn assert_pixel_near(data: &[u8], row_bytes: u32, x: u32, y: u32, expected: [u8; 4]) {
        let actual = pixel_with_stride(data, row_bytes, x, y);
        for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
            assert!(
                actual.abs_diff(expected) <= 2,
                "pixel ({x}, {y}) channel {channel}: expected {expected}, got {actual}"
            );
        }
    }

    fn record_v2_fill(
        tree: &RenderTree,
        node: crate::draw_cmd_v2::RenderNodeId,
        rect: Rect,
        color: Color,
    ) {
        tree.context(node)
            .unwrap()
            .begin_recording()
            .unwrap()
            .commit(vec![V2DrawCommand::FillRect {
                rect,
                color,
                border_radius: [0.0; 4],
                border_width: [0.0; 4],
                border_color: Color::transparent(),
                outline_width: [0.0; 4],
                outline_color: Color::transparent(),
            }])
            .unwrap();
    }

    /// Construction compiles every built-in pipeline shader for Metal.
    #[test]
    fn generic_renderer_initializes_all_builtin_pipelines_on_metal() {
        let Some(backend) = MetalBackend::new() else {
            eprintln!("skipping: this host exposes no Metal device");
            return;
        };

        let _renderer = RendererImpl::<MetalBackend>::new(&backend, FORMAT);
    }

    /// Draw the non-rectangle built-ins through their Metal pipelines and
    /// verify image, multiple text requests, SVG, material and composite pixels.
    #[test]
    fn generic_renderer_renders_all_builtin_pipelines_on_metal() {
        let Some(backend) = MetalBackend::new() else {
            eprintln!("skipping: this host exposes no Metal device");
            return;
        };

        use std::sync::Arc;

        use crate::pipeline::material::{MATERIAL_PIPELINE_NAME, MaterialKind, MaterialRequest};
        use crate::svg::{
            SvgElementKind, SvgFill, SvgFillRule, SvgGeometry, SvgNode, SvgNodeId, SvgPaintOrder,
            SvgPathCommand, SvgScene, SvgTransform, SvgViewport,
        };
        use crate::utilities::Vec2d;

        let mut renderer = RendererImpl::<MetalBackend>::new(&backend, FORMAT);
        let mut draw = DrawList::new();
        let blue = [0, 0, 255, 255];
        let texture_id = draw.load_image(&[blue; 4].concat(), 2, 2);
        draw.draw_image(Rect::new(0.0, 0.0, 24.0, 24.0), texture_id);
        draw.draw_text(
            Vec2d::new(32.0, 24.0),
            Arc::from("A"),
            18.0,
            Color::white(),
            400,
        );
        draw.draw_text(
            Vec2d::new(2.0, 36.0),
            Arc::from("B"),
            18.0,
            Color::white(),
            400,
        );
        draw.draw_text_decoration(Rect::new(8.0, 56.0, 48.0, 4.0), Color::white(), 0, 2.0, 8.0);

        let scene = Arc::new(SvgScene {
            viewport: SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            nodes: Arc::new([
                SvgNode {
                    node_id: SvgNodeId(0),
                    svg_id: None,
                    classes: Arc::new([]),
                    element: SvgElementKind::Path,
                    is_definition: false,
                    definition_owner: None,
                    clip_path: None,
                    mask: None,
                    filter: None,
                    parent: None,
                    children: Arc::new([]),
                    transform: SvgTransform::default(),
                    opacity: 1.0,
                    geometry: Some(0),
                    fill_rule: SvgFillRule::NonZero,
                    fill: Some(SvgFill {
                        color: crate::svg::SvgColor::rgba8(255, 0, 0, 255),
                        rule: SvgFillRule::NonZero,
                    }),
                    stroke: None,
                    fill_paint: None,
                    stroke_paint: None,
                    paint_order: SvgPaintOrder::FillAndStroke,
                    visible: true,
                },
                SvgNode {
                    node_id: SvgNodeId(1),
                    svg_id: None,
                    classes: Arc::new([]),
                    element: SvgElementKind::Path,
                    is_definition: false,
                    definition_owner: None,
                    clip_path: None,
                    mask: None,
                    filter: None,
                    parent: None,
                    children: Arc::new([]),
                    transform: SvgTransform {
                        tx: 5.0,
                        ..SvgTransform::default()
                    },
                    opacity: 1.0,
                    geometry: Some(0),
                    fill_rule: SvgFillRule::NonZero,
                    fill: Some(SvgFill {
                        color: crate::svg::SvgColor::rgba8(0, 255, 0, 255),
                        rule: SvgFillRule::NonZero,
                    }),
                    stroke: None,
                    fill_paint: None,
                    stroke_paint: None,
                    paint_order: SvgPaintOrder::FillAndStroke,
                    visible: true,
                },
            ]),
            geometries: Arc::new([SvgGeometry {
                commands: Arc::new([
                    SvgPathCommand::MoveTo { x: 0.0, y: 0.0 },
                    SvgPathCommand::LineTo { x: 10.0, y: 0.0 },
                    SvgPathCommand::LineTo { x: 10.0, y: 10.0 },
                    SvgPathCommand::LineTo { x: 0.0, y: 10.0 },
                    SvgPathCommand::Close,
                ]),
            }]),
            resources: Arc::new(crate::svg::SvgResourceGraph::empty()),
        });
        draw.draw_svg(scene, Rect::new(32.0, 0.0, 20.0, 20.0), Arc::from([]));
        draw.draw_custom(
            MATERIAL_PIPELINE_NAME,
            MaterialRequest::new(MaterialKind::Glass, [24.0, 32.0, 32.0, 20.0]),
        );

        let target = backend.create_texture(&TextureDescriptor {
            label: Some("all Metal built-ins target".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: FORMAT,
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        let view = backend.create_texture_view(&target, "all Metal built-ins view");
        renderer.render(&backend, &view, SIZE, SIZE, false, &draw);

        let readback = backend.create_buffer(&BufferDescriptor {
            label: Some("all Metal built-ins readback".to_string()),
            size: FRAME_BYTES,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let mut encoder = backend.create_command_encoder("all Metal built-ins readback");
        backend.copy_texture_to_buffer(
            &mut encoder,
            &TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW_BYTES),
                    rows_per_image: Some(SIZE),
                },
            },
            Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);

        let pixels = backend.read_buffer(&readback, FRAME_BYTES);
        assert_eq!(pixel(&pixels, 12, 12), [0, 0, 255, 255], "image");
        assert_eq!(pixel(&pixels, 35, 10), [255, 0, 0, 255], "first SVG path");
        assert_eq!(pixel(&pixels, 45, 10), [0, 255, 0, 255], "second SVG path");
        assert_eq!(
            pixel(&pixels, 55, 10),
            [0, 255, 0, 255],
            "second SVG path transform"
        );
        assert_eq!(pixel(&pixels, 32, 58), [255, 255, 255, 255], "decoration");
        assert!(
            (32..SIZE).any(|x| {
                (16..32).any(|y| {
                    let [red, green, blue, alpha] = pixel(&pixels, x, y);
                    alpha > 0 && red > 0 && green > 0 && blue > 0
                })
            }),
            "text pipeline should render antialiased glyph pixels"
        );
        assert!(
            (2..20).any(|x| (24..36).any(|y| pixel(&pixels, x, y)[3] > 0)),
            "a later text request should draw from its own instance range"
        );
        assert!(pixel(&pixels, 40, 40)[3] > 0, "material + frame composite");
    }

    /// The analytic antialiasing fringe carries a per-vertex coverage that the
    /// Metal SVG shader must multiply into the fragment. Ignoring it draws the
    /// fringe at full opacity: edges grow by a pixel and stay hard, which shows
    /// as sawtooth corners on curved icons.
    #[test]
    fn svg_edge_fringe_is_partially_covered_on_metal() {
        let Some(backend) = MetalBackend::new() else {
            eprintln!("skipping: this host exposes no Metal device");
            return;
        };

        use std::sync::Arc;

        use crate::svg::{
            SvgElementKind, SvgFill, SvgFillRule, SvgGeometry, SvgNode, SvgNodeId, SvgPaintOrder,
            SvgPathCommand, SvgScene, SvgTransform, SvgViewport,
        };

        let mut renderer = RendererImpl::<MetalBackend>::new(&backend, FORMAT);
        let mut draw = DrawList::new();
        // The right edge sits a quarter pixel into pixel column 10, so the
        // fringe quad covers that column's centre at a coverage of 0.75.
        let scene = Arc::new(SvgScene {
            viewport: SvgViewport {
                width: 16.0,
                height: 16.0,
            },
            nodes: Arc::new([SvgNode {
                node_id: SvgNodeId(0),
                svg_id: None,
                classes: Arc::new([]),
                element: SvgElementKind::Path,
                is_definition: false,
                definition_owner: None,
                clip_path: None,
                mask: None,
                filter: None,
                parent: None,
                children: Arc::new([]),
                transform: SvgTransform::default(),
                opacity: 1.0,
                geometry: Some(0),
                fill_rule: SvgFillRule::NonZero,
                fill: Some(SvgFill {
                    color: crate::svg::SvgColor::rgba8(255, 0, 0, 255),
                    rule: SvgFillRule::NonZero,
                }),
                stroke: None,
                fill_paint: None,
                stroke_paint: None,
                paint_order: SvgPaintOrder::FillAndStroke,
                visible: true,
            }]),
            geometries: Arc::new([SvgGeometry {
                commands: Arc::new([
                    SvgPathCommand::MoveTo { x: 2.0, y: 2.0 },
                    SvgPathCommand::LineTo { x: 10.25, y: 2.0 },
                    SvgPathCommand::LineTo { x: 10.25, y: 14.0 },
                    SvgPathCommand::LineTo { x: 2.0, y: 14.0 },
                    SvgPathCommand::Close,
                ]),
            }]),
            resources: Arc::new(crate::svg::SvgResourceGraph::empty()),
        });
        draw.draw_svg(scene, Rect::new(0.0, 0.0, 16.0, 16.0), Arc::from([]));

        let target = backend.create_texture(&TextureDescriptor {
            label: Some("Metal SVG fringe target".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: FORMAT,
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        let view = backend.create_texture_view(&target, "Metal SVG fringe view");
        renderer.render(&backend, &view, SIZE, SIZE, false, &draw);

        let readback = backend.create_buffer(&BufferDescriptor {
            label: Some("Metal SVG fringe readback".to_string()),
            size: FRAME_BYTES,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let mut encoder = backend.create_command_encoder("Metal SVG fringe readback");
        backend.copy_texture_to_buffer(
            &mut encoder,
            &TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW_BYTES),
                    rows_per_image: Some(SIZE),
                },
            },
            Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);

        let pixels = backend.read_buffer(&readback, FRAME_BYTES);
        assert_eq!(pixel(&pixels, 6, 8), [255, 0, 0, 255], "interior is opaque");
        let fringe = pixel(&pixels, 10, 8)[3];
        assert!(
            (100..=230).contains(&fringe),
            "fringe pixel should be partially covered, got alpha {fringe}"
        );
        assert_eq!(pixel(&pixels, 12, 8)[3], 0, "beyond the fringe is empty");
    }

    /// Render a DrawList through the generic renderer on Metal and verify the
    /// rect's pixels through a buffer readback.
    #[test]
    fn generic_renderer_renders_rect_through_metal() {
        let Some(backend) = MetalBackend::new() else {
            eprintln!("skipping: this host exposes no Metal device");
            return;
        };

        let mut renderer = RendererImpl::<MetalBackend>::new_rect_only(&backend, FORMAT);
        let mut draw_list = DrawList::new();
        draw_list.fill_rect(
            Rect::new(8.0, 8.0, 48.0, 48.0),
            Color::red(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );

        let target = backend.create_texture(&TextureDescriptor {
            label: Some("generic Metal renderer target".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: FORMAT,
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        let view = backend.create_texture_view(&target, "generic Metal renderer view");
        renderer.render(&backend, &view, SIZE, SIZE, false, &draw_list);

        let readback = backend.create_buffer(&BufferDescriptor {
            label: Some("generic Metal renderer readback".to_string()),
            size: FRAME_BYTES,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let mut encoder = backend.create_command_encoder("generic Metal renderer readback");
        backend.copy_texture_to_buffer(
            &mut encoder,
            &TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW_BYTES),
                    rows_per_image: Some(SIZE),
                },
            },
            Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);

        let pixels = backend.read_buffer(&readback, FRAME_BYTES);
        assert_eq!(pixel(&pixels, 32, 32), [255, 0, 0, 255]);
        assert_eq!(pixel(&pixels, 1, 1), [0, 0, 0, 0]);
    }

    /// Render two rects on a transparent 64x64 target and read the pixels back.
    ///
    /// The placement is deliberate. Red occupies the upper band and green the
    /// lower one, with a gap between them and margins at the edges, so a single
    /// pass proves four things at once:
    ///
    /// * a filled pixel anywhere means the vertex descriptor, the argument
    ///   indices and the MSL all agree;
    /// * *which* band is red rules out a vertical flip, which a full-cover rect
    ///   would hide completely;
    /// * the clear leaving pixels transparent proves the load action reached the
    ///   attachment;
    /// * a green pixel in the last rows means the readback row pitch is right,
    ///   which a centre-only check cannot tell.
    #[test]
    fn two_bands_render_and_read_back_through_metal() {
        let Some(backend) = MetalBackend::new() else {
            eprintln!("skipping: this host exposes no Metal device");
            return;
        };

        // A shader that fails to compile here is the most likely first failure
        // of a hand-written MSL port, so carry the compiler's own words.
        let msl = include_str!("../pipeline/shaders/metal/rect.metal");
        let module = backend
            .try_create_shader_module(msl.as_bytes(), "rect shader")
            .expect("rect.metal must compile as MSL");

        let viewport_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("metal test viewport".to_string()),
            size: 16,
            usage: vec![BufferUsage::Uniform, BufferUsage::CopyDst],
        });
        let layout = backend.create_bind_group_layout(&[BindGroupLayoutEntry {
            binding: 0,
            visibility: vec![ShaderStage::Vertex, ShaderStage::Fragment],
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }]);
        let bind_group = backend.create_bind_group(
            &layout,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(&viewport_buffer),
            }],
        );
        let pipeline_layout = backend.create_pipeline_layout(&[&layout]);

        let attributes = rect_attributes();
        let vertex_buffers = [Some(VertexBufferLayout {
            array_stride: size_of::<RectInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &attributes,
        })];
        let targets = [Some(ColorTargetState {
            format: FORMAT,
            blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: ColorWriteMask::ALL,
        })];

        let pipeline = backend
            .try_create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("metal test rect".to_string()),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &module,
                    entry_point: "vs_main",
                    buffers: &vertex_buffers,
                },
                fragment: Some(crate::backend::FragmentState {
                    module: &module,
                    entry_point: "fs_main",
                    targets: &targets,
                }),
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: crate::backend::MultisampleState::default(),
            })
            .expect("the rect pipeline must build on Metal");

        // `surface_is_srgb` stays 0.0 to match the non-sRGB reference target the
        // wgpu tests render into.
        backend.write_buffer(
            &viewport_buffer,
            0,
            bytemuck::cast_slice(&[SIZE as f32, SIZE as f32, 0.0f32, 0.0f32]),
        );

        let red = Rgba8::new(255, 0, 0, 255);
        let green = Rgba8::new(0, 255, 0, 255);
        let instances = [
            instance(4.0, 4.0, 56.0, 24.0, red),
            instance(4.0, 36.0, 56.0, 24.0, green),
        ];
        let instance_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("metal test instances".to_string()),
            size: (instances.len() * size_of::<RectInstance>()) as u64,
            usage: vec![BufferUsage::Vertex],
        });
        backend.write_buffer(&instance_buffer, 0, bytemuck::cast_slice(&instances));

        let target = backend.create_texture(&TextureDescriptor {
            label: Some("metal test target".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: FORMAT,
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        let view = backend.create_texture_view(&target, "metal test view");

        let mut encoder = backend.create_command_encoder("metal test frame");
        {
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &RenderPassDescriptor {
                    label: Some("metal test pass".to_string()),
                    color_attachments: &[RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear([0.0, 0.0, 0.0, 0.0]),
                            store: StoreOp::Store,
                        },
                    }],
                    depth_stencil_attachment: None,
                },
            );
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_vertex_buffer(0, &instance_buffer, 0);
            // Six vertices generated in the shader, one instance per band.
            pass.draw(0..6, 0..2);
        }

        let readback = backend.create_buffer(&BufferDescriptor {
            label: Some("metal test readback".to_string()),
            size: FRAME_BYTES,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        backend.copy_texture_to_buffer(
            &mut encoder,
            &TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW_BYTES),
                    rows_per_image: Some(SIZE),
                },
            },
            Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);

        let pixels = backend.read_buffer(&readback, FRAME_BYTES);
        assert_eq!(pixels.len(), FRAME_BYTES as usize);

        // Upper band: the red instance.
        assert_eq!(pixel(&pixels, 32, 16), [255, 0, 0, 255], "upper band");
        assert_eq!(pixel(&pixels, 8, 8), [255, 0, 0, 255], "upper band corner");
        // Lower band: the green instance. Getting these two the wrong way round
        // is what a silent vertical flip would look like.
        assert_eq!(pixel(&pixels, 32, 48), [0, 255, 0, 255], "lower band");
        assert_eq!(
            pixel(&pixels, 8, 56),
            [0, 255, 0, 255],
            "lower band, last rows: a wrong readback pitch lands here"
        );
        // Gap and margins stayed cleared.
        assert_eq!(pixel(&pixels, 32, 32), [0, 0, 0, 0], "gap between bands");
        assert_eq!(pixel(&pixels, 1, 1), [0, 0, 0, 0], "outside both bands");
    }

    #[test]
    fn queued_texture_upload_honors_offset_and_padded_rows() {
        let Some(backend) = MetalBackend::new() else {
            eprintln!("skipping: this host exposes no Metal device");
            return;
        };

        const WIDTH: u32 = 5;
        const HEIGHT: u32 = 3;
        const SOURCE_STRIDE: usize = 8;
        const SOURCE_OFFSET: usize = 2;
        const READBACK_STRIDE: u32 = 256;
        let mut source = vec![0xee; SOURCE_OFFSET + SOURCE_STRIDE * HEIGHT as usize];
        for row in 0..HEIGHT as usize {
            for column in 0..WIDTH as usize {
                source[SOURCE_OFFSET + row * SOURCE_STRIDE + column] =
                    (row * 16 + column + 1) as u8;
            }
        }

        let texture = backend.create_texture(&TextureDescriptor {
            label: Some("offset texture upload target".to_string()),
            size: (WIDTH, HEIGHT, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: MTLPixelFormat::R8Unorm,
            usage: vec![
                TextureUsage::TextureBinding,
                TextureUsage::CopyDst,
                TextureUsage::CopySrc,
            ],
        });
        backend.write_texture(&WriteTextureDescriptor {
            texture: &texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
            data: &source,
            buffer_layout: TexelCopyBufferLayout {
                offset: SOURCE_OFFSET as u64,
                bytes_per_row: Some(SOURCE_STRIDE as u32),
                rows_per_image: Some(HEIGHT),
            },
            extent: Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        });

        let readback = backend.create_buffer(&BufferDescriptor {
            label: Some("offset texture upload readback".to_string()),
            size: (READBACK_STRIDE * HEIGHT) as u64,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let mut encoder = backend.create_command_encoder("offset texture upload readback");
        backend.copy_texture_to_buffer(
            &mut encoder,
            &TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(READBACK_STRIDE),
                    rows_per_image: Some(HEIGHT),
                },
            },
            Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);

        let pixels = backend.read_buffer(&readback, (READBACK_STRIDE * HEIGHT) as u64);
        for row in 0..HEIGHT as usize {
            let start = row * READBACK_STRIDE as usize;
            let expected = (0..WIDTH as usize)
                .map(|column| (row * 16 + column + 1) as u8)
                .collect::<Vec<_>>();
            assert_eq!(&pixels[start..start + WIDTH as usize], expected);
        }
    }

    /// The `Unorm8x4` colors must arrive normalized, not as raw 0..255 bytes.
    ///
    /// This is the specific thing that would break if the vertex descriptor were
    /// dropped in favour of reading `RectInstance` as a plain MSL struct: Metal's
    /// own alignment for `float4` does not reproduce the Rust layout, and pulling
    /// bytes would hand the shader 255.0 instead of 1.0.
    #[test]
    fn unorm_vertex_attributes_reach_the_shader_normalized() {
        let Some(backend) = MetalBackend::new() else {
            eprintln!("skipping: this host exposes no Metal device");
            return;
        };
        let msl = include_str!("../pipeline/shaders/metal/rect.metal");
        let module = backend
            .try_create_shader_module(msl.as_bytes(), "rect shader")
            .expect("rect.metal must compile as MSL");

        // Half-intensity grey: 128/255 of full.
        let grey = Rgba8::new(128, 128, 128, 255);
        let viewport_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("unorm viewport".to_string()),
            size: 16,
            usage: vec![BufferUsage::Uniform, BufferUsage::CopyDst],
        });
        backend.write_buffer(
            &viewport_buffer,
            0,
            bytemuck::cast_slice(&[SIZE as f32, SIZE as f32, 0.0f32, 0.0f32]),
        );
        let layout = backend.create_bind_group_layout(&[BindGroupLayoutEntry {
            binding: 0,
            visibility: vec![ShaderStage::Vertex, ShaderStage::Fragment],
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }]);
        let bind_group = backend.create_bind_group(
            &layout,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(&viewport_buffer),
            }],
        );
        let pipeline_layout = backend.create_pipeline_layout(&[&layout]);
        let attributes = rect_attributes();
        let vertex_buffers = [Some(VertexBufferLayout {
            array_stride: size_of::<RectInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &attributes,
        })];
        let targets = [Some(ColorTargetState {
            format: FORMAT,
            blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: ColorWriteMask::ALL,
        })];
        let pipeline = backend
            .try_create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("unorm rect".to_string()),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &module,
                    entry_point: "vs_main",
                    buffers: &vertex_buffers,
                },
                fragment: Some(crate::backend::FragmentState {
                    module: &module,
                    entry_point: "fs_main",
                    targets: &targets,
                }),
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: crate::backend::MultisampleState::default(),
            })
            .expect("the rect pipeline must build on Metal");

        let instances = [instance(0.0, 0.0, SIZE as f32, SIZE as f32, grey)];
        let instance_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("unorm instances".to_string()),
            size: (size_of::<RectInstance>()) as u64,
            usage: vec![BufferUsage::Vertex],
        });
        backend.write_buffer(&instance_buffer, 0, bytemuck::cast_slice(&instances));
        let target = backend.create_texture(&TextureDescriptor {
            label: Some("unorm target".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: FORMAT,
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        let view = backend.create_texture_view(&target, "unorm view");

        let mut encoder = backend.create_command_encoder("unorm frame");
        {
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &RenderPassDescriptor {
                    label: Some("unorm pass".to_string()),
                    color_attachments: &[RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear([0.0, 0.0, 0.0, 0.0]),
                            store: StoreOp::Store,
                        },
                    }],
                    depth_stencil_attachment: None,
                },
            );
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_vertex_buffer(0, &instance_buffer, 0);
            pass.draw(0..6, 0..1);
        }
        let readback = backend.create_buffer(&BufferDescriptor {
            label: Some("unorm readback".to_string()),
            size: FRAME_BYTES,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        backend.copy_texture_to_buffer(
            &mut encoder,
            &TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW_BYTES),
                    rows_per_image: Some(SIZE),
                },
            },
            Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);
        let pixels = backend.read_buffer(&readback, FRAME_BYTES);

        let centre = pixel(&pixels, 32, 32);
        assert_eq!(centre[3], 255, "alpha must stay opaque: {centre:?}");
        assert!(
            (118..=138).contains(&centre[0]),
            "128/255 unorm should land near mid grey after the sRGB round trip, got {centre:?}"
        );
        assert_eq!(centre[0], centre[1]);
        assert_eq!(centre[1], centre[2]);
    }

    #[test]
    fn cropped_opacity_groups_render_with_msaa_through_native_metal() {
        let Some(backend) = MetalBackend::new() else {
            eprintln!("skipping: this host exposes no Metal device");
            return;
        };

        const TARGET_SIZE: u32 = 80;
        let format = <MetalBackend as GpuBackend>::rgba8_unorm_format();
        let target = backend.create_texture(&TextureDescriptor {
            label: Some("native Metal cropped opacity target".to_string()),
            size: (TARGET_SIZE, TARGET_SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        let view = backend.create_texture_view(&target, "native Metal cropped opacity view");
        let mut renderer = RendererImpl::<MetalBackend>::with_antialiasing(
            &backend,
            format,
            crate::AntiAlias::Msaa4x,
        );

        let tree = RenderTree::new();
        let root = tree
            .add_root(Rect::new(0.0, 0.0, TARGET_SIZE as f32, TARGET_SIZE as f32))
            .unwrap();
        tree.set_clip(
            root,
            Some(Rect::new(0.0, 0.0, TARGET_SIZE as f32, TARGET_SIZE as f32)),
        )
        .unwrap();
        tree.set_opacity(root, 0.5).unwrap();
        let nested = tree
            .add_child(root, Rect::new(8.0, 8.0, 56.0, 56.0))
            .unwrap();
        tree.set_clip(nested, Some(Rect::new(0.0, 0.0, 40.0, 40.0)))
            .unwrap();
        tree.set_opacity(nested, 0.5).unwrap();
        let green = tree
            .add_child(nested, Rect::new(8.0, 8.0, 32.0, 32.0))
            .unwrap();
        let blue = tree
            .add_child(nested, Rect::new(24.0, 24.0, 40.0, 40.0))
            .unwrap();
        let edge_group = tree
            .add_child(root, Rect::new(48.0, 48.0, 32.0, 32.0))
            .unwrap();
        tree.set_clip(edge_group, Some(Rect::new(0.0, 0.0, 32.0, 32.0)))
            .unwrap();
        tree.set_opacity(edge_group, 0.5).unwrap();
        let yellow = tree
            .add_child(edge_group, Rect::new(0.0, 0.0, 32.0, 32.0))
            .unwrap();
        record_v2_fill(
            &tree,
            green,
            Rect::new(0.0, 0.0, 32.0, 32.0),
            Color::rgba8(0, 255, 0, 255),
        );
        record_v2_fill(
            &tree,
            blue,
            Rect::new(0.0, 0.0, 40.0, 40.0),
            Color::rgba8(0, 0, 255, 255),
        );
        record_v2_fill(
            &tree,
            yellow,
            Rect::new(0.0, 0.0, 32.0, 32.0),
            Color::rgba8(255, 255, 0, 255),
        );

        let metadata = || {
            FrameRenderMetadata::new(1.0, 301, 1, 1, 1, DamageSet::new(TARGET_SIZE, TARGET_SIZE))
        };
        let first_packet = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, TARGET_SIZE as f32, TARGET_SIZE as f32)],
                operations: tree.render_all(),
            },
            metadata(),
        )
        .expect("lower native Metal opacity groups");
        let first_plan = first_packet.render_plan().unwrap();
        let revisions = [green, blue, yellow].map(|node| {
            first_plan
                .local_v2_revision(node)
                .expect("retained child list")
        });
        renderer.render_packet(&backend, &view, &first_packet, false);

        let readback_row_bytes = (TARGET_SIZE * 4).div_ceil(256) * 256;
        let readback_size = u64::from(readback_row_bytes) * u64::from(TARGET_SIZE);
        let readback = backend.create_buffer(&BufferDescriptor {
            label: Some("native Metal cropped opacity readback".to_string()),
            size: readback_size,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let read_pixels = || {
            let mut encoder =
                backend.create_command_encoder("native Metal cropped opacity readback");
            backend.copy_texture_to_buffer(
                &mut encoder,
                &TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                &TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(readback_row_bytes),
                        rows_per_image: Some(TARGET_SIZE),
                    },
                },
                Extent3d {
                    width: TARGET_SIZE,
                    height: TARGET_SIZE,
                    depth_or_array_layers: 1,
                },
            );
            backend.submit(encoder);
            backend.read_buffer(&readback, readback_size)
        };

        let first_pixels = read_pixels();
        assert_pixel_near(&first_pixels, readback_row_bytes, 20, 20, [0, 64, 0, 64]);
        assert_pixel_near(&first_pixels, readback_row_bytes, 40, 40, [0, 0, 64, 64]);
        assert_pixel_near(&first_pixels, readback_row_bytes, 70, 70, [64, 64, 0, 64]);
        let _ = tree.take_damage();

        tree.set_opacity(nested, 0.25).unwrap();
        let update_packet = FramePacket::from_v2_direct_with_frame(
            RenderFrame {
                damage: tree.take_damage(),
                operations: tree.render_all(),
            },
            metadata(),
            Frame::new(DrawList::new(), TARGET_SIZE, TARGET_SIZE),
        )
        .expect("lower native Metal MSAA opacity update");
        for (node, revision) in [green, blue, yellow].into_iter().zip(revisions) {
            assert_eq!(
                update_packet.render_plan().unwrap().local_v2_revision(node),
                Some(revision)
            );
        }
        renderer.render_packet(&backend, &view, &update_packet, false);
        let updated_pixels = read_pixels();
        assert_pixel_near(&updated_pixels, readback_row_bytes, 20, 20, [0, 32, 0, 32]);
        assert_pixel_near(&updated_pixels, readback_row_bytes, 40, 40, [0, 0, 32, 32]);
        assert_pixel_near(&updated_pixels, readback_row_bytes, 70, 70, [64, 64, 0, 64]);
        assert_eq!(
            pixel_with_stride(&updated_pixels, readback_row_bytes, 2, 2),
            [0, 0, 0, 0]
        );
    }
}
