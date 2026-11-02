//! The renderer-facing pipeline that draws one registered effect.

use std::any::Any;
use std::ops::Range;
use std::sync::Arc;

use aimer_cupid_gpu::*;
use aimer_pipeline_core::{
    AntiAlias, CommandContext, CustomPipelineGeneric, RenderContextGeneric, multisample_state,
};

use crate::EffectSpec;
use crate::effect::decode_payload;
use crate::header::{HEADER_SIZE, Header};

/// More effect draws than this in one frame are dropped.
const MAX_REQUESTS_PER_FRAME: usize = 1024;

/// A captured backdrop larger than this many pixels is not taken; the effect
/// then sees a transparent texture instead of risking the allocation.
const MAX_BACKDROP_PIXELS: u64 = 16 * 1024 * 1024;

/// Width and height of the texture an effect samples when no backdrop exists.
const NEUTRAL_EXTENT: u32 = 1;

/// One draw, resolved to target pixels when the renderer reached it.
struct Request {
    rect: [f32; 4],
    clip_rect: [f32; 4],
    clip_radii: [f32; 4],
    opacity: f32,
    /// Where its parameter bytes sit in the frame's shared arena.
    params: Range<usize>,
}

/// Bytes between the starts of consecutive uniform slots.
///
/// A dynamic offset must be a multiple of the backend's alignment, so each
/// command's `HEADER_SIZE + params` bytes are rounded up to it.
pub(crate) fn uniform_stride(params_size: usize, alignment: u32) -> u64 {
    let alignment = u64::from(alignment.max(1));
    ((HEADER_SIZE + params_size) as u64).div_ceil(alignment) * alignment
}

/// The rectangle to copy from the frame for a backdrop, clamped to the target.
///
/// Returns `(x, y, width, height)` in whole pixels, or `None` when the rect
/// misses the target or is empty.
pub(crate) fn backdrop_copy(rect: [f32; 4], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let x0 = rect[0].floor().max(0.0);
    let y0 = rect[1].floor().max(0.0);
    let x1 = (rect[0] + rect[2]).ceil().min(width as f32);
    let y1 = (rect[1] + rect[3]).ceil().min(height as f32);
    (x1 > x0 && y1 > y0).then(|| (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
}

enum State<B: GpuBackend> {
    /// Nothing is built; the first frame with a command builds it.
    Unbuilt,
    Ready(Gpu<B>),
    /// Building failed (reported once); every command is dropped.
    Failed,
}

struct Gpu<B: GpuBackend> {
    format: B::TextureFormat,
    pipeline: B::RenderPipeline,
    bind_group_layout: B::BindGroupLayout,
    bind_group: B::BindGroup,
    uniform_buffer: B::Buffer,
    uniform_capacity: usize,
    backdrop: Option<Backdrop<B>>,
}

struct Backdrop<B: GpuBackend> {
    sampler: B::Sampler,
    texture: B::Texture,
    view: B::TextureView,
    extent: (u32, u32),
    format: B::TextureFormat,
    /// Whether the texture holds a captured frame rather than the neutral texel.
    captured: bool,
}

/// Draws one registered [`ShaderEffect`](crate::ShaderEffect) for a renderer.
///
/// The renderer builds one per registered effect and per backend. It compiles
/// nothing until an effect is first drawn, so an effect an app registers but
/// never shows costs nothing.
pub struct EffectPipeline<B: GpuBackend> {
    spec: Arc<EffectSpec>,
    antialiasing: AntiAlias,
    state: State<B>,
    requests: Vec<Request>,
    /// Parameter bytes of every request this frame, back to back.
    arena: Vec<u8>,
    upload: Vec<u8>,
    stride: u64,
    /// Target size of the frame being prepared, for the backdrop copy bounds.
    target: (u32, u32),
}

impl<B: GpuBackend> EffectPipeline<B> {
    /// Creates the pipeline for `spec`. No GPU resource exists until it draws.
    pub fn new(spec: Arc<EffectSpec>, antialiasing: AntiAlias) -> Self {
        Self {
            spec,
            antialiasing,
            state: State::Unbuilt,
            requests: Vec::new(),
            arena: Vec::new(),
            upload: Vec::new(),
            stride: 0,
            target: (0, 0),
        }
    }

    fn fail(&mut self, message: &str) {
        eprintln!("aimer effect `{}`: {message}; the effect will not draw", self.spec.name());
        self.state = State::Failed;
    }

    fn samples_backdrop(&self) -> bool {
        self.spec.program().samples_backdrop()
    }

    /// Builds the pipeline, bind group layout and buffers for `format`.
    fn build(&self, ctx: &RenderContextGeneric<'_, B>) -> Result<Gpu<B>, String> {
        let backend = ctx.backend;
        let language = backend
            .runtime_shader_language()
            .ok_or("this backend does not take shaders supplied at run time")?;
        let source = self.spec.program().module_source(language)?;
        let label = format!("effect {}", self.spec.name());
        let module = backend
            .try_create_shader_module(&source, &label)
            .map_err(|error| error.to_string())?;

        let mut entries = vec![BindGroupLayoutEntry {
            binding: 0,
            visibility: vec![ShaderStage::Vertex, ShaderStage::Fragment],
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: None,
            },
            count: None,
        }];
        if self.samples_backdrop() {
            entries.push(BindGroupLayoutEntry {
                binding: 1,
                visibility: vec![ShaderStage::Fragment],
                ty: BindingType::Texture {
                    multisampled: false,
                    view_dimension: TextureViewDimension::D2,
                    sample_type: TextureSampleType::Float { filterable: true },
                },
                count: None,
            });
            entries.push(BindGroupLayoutEntry {
                binding: 2,
                visibility: vec![ShaderStage::Fragment],
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            });
        }
        let bind_group_layout = backend.create_bind_group_layout(&entries);
        let pipeline_layout = backend.create_pipeline_layout(&[&bind_group_layout]);
        let pipeline = backend
            .try_create_render_pipeline(&RenderPipelineDescriptor {
                label: Some(label.clone()),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &module,
                    entry_point: "vs_main",
                    buffers: &[],
                },
                fragment: Some(FragmentState {
                    module: &module,
                    entry_point: "fs_main",
                    targets: &[Some(ColorTargetState {
                        format: ctx.format,
                        blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: ColorWriteMask::ALL,
                    })],
                }),
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: multisample_state(self.antialiasing),
            })
            .map_err(|error| error.to_string())?;

        let stride = uniform_stride(
            self.spec.program().params_size(),
            backend.limits().min_uniform_buffer_offset_alignment,
        );
        let uniform_capacity = 16;
        let uniform_buffer = uniform_buffer(backend, stride, uniform_capacity);
        let backdrop = self.samples_backdrop().then(|| neutral_backdrop(backend));
        let bind_group = bind_group(backend, &bind_group_layout, &uniform_buffer, stride, backdrop.as_ref());
        Ok(Gpu {
            format: ctx.format,
            pipeline,
            bind_group_layout,
            bind_group,
            uniform_buffer,
            uniform_capacity,
            backdrop,
        })
    }
}

fn uniform_buffer<B: GpuBackend>(backend: &B, stride: u64, capacity: usize) -> B::Buffer {
    backend.create_buffer(&BufferDescriptor {
        label: Some("effect uniform buffer".to_string()),
        size: stride * capacity as u64,
        usage: vec![BufferUsage::Uniform, BufferUsage::CopyDst],
    })
}

fn bind_group<B: GpuBackend>(
    backend: &B,
    layout: &B::BindGroupLayout,
    uniform_buffer: &B::Buffer,
    stride: u64,
    backdrop: Option<&Backdrop<B>>,
) -> B::BindGroup {
    let mut entries = vec![BindGroupEntry {
        binding: 0,
        resource: BindingResource::BufferRange(uniform_buffer, 0, stride),
    }];
    if let Some(backdrop) = backdrop {
        entries.push(BindGroupEntry {
            binding: 1,
            resource: BindingResource::TextureView(&backdrop.view),
        });
        entries.push(BindGroupEntry {
            binding: 2,
            resource: BindingResource::Sampler(&backdrop.sampler),
        });
    }
    backend.create_bind_group(layout, &entries)
}

fn backdrop_texture<B: GpuBackend>(
    backend: &B,
    format: B::TextureFormat,
    (width, height): (u32, u32),
) -> (B::Texture, B::TextureView) {
    let texture = backend.create_texture(&TextureDescriptor {
        label: Some("effect backdrop".to_string()),
        size: (width, height, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: vec![TextureUsage::TextureBinding, TextureUsage::CopyDst],
    });
    let view = backend.create_texture_view(&texture, "effect backdrop view");
    (texture, view)
}

/// A 1x1 transparent texture, so a backdrop effect always has something bound.
fn neutral_backdrop<B: GpuBackend>(backend: &B) -> Backdrop<B> {
    let format = B::rgba8_unorm_format();
    let extent = (NEUTRAL_EXTENT, NEUTRAL_EXTENT);
    let (texture, view) = backdrop_texture(backend, format, extent);
    backend.write_texture(&WriteTextureDescriptor {
        texture: &texture,
        mip_level: 0,
        origin: Origin3d::ZERO,
        aspect: TextureAspect::All,
        data: &[0, 0, 0, 0],
        buffer_layout: TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        extent: Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    });
    let sampler = backend.create_sampler(&SamplerDescriptor {
        label: Some("effect backdrop sampler".to_string()),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: FilterMode::Nearest,
        lod_min_clamp: 0.0,
        lod_max_clamp: f32::MAX,
        compare: None,
        max_anisotropy: 1,
    });
    Backdrop {
        sampler,
        texture,
        view,
        extent,
        format,
        captured: false,
    }
}

impl<B: GpuBackend> CustomPipelineGeneric<B> for EffectPipeline<B> {
    fn name(&self) -> &str {
        self.spec.name()
    }

    fn begin_frame(&mut self) {
        self.requests.clear();
        self.arena.clear();
    }

    fn prepare_command(&mut self, _data: &(dyn Any + Send)) -> Option<usize> {
        // An effect is meaningless without its placement; the renderer always
        // supplies it through `prepare_command_in`.
        None
    }

    fn prepare_command_in(&mut self, data: &(dyn Any + Send), context: &CommandContext) -> Option<usize> {
        if matches!(self.state, State::Failed) || self.requests.len() >= MAX_REQUESTS_PER_FRAME {
            return None;
        }
        let payload = data.downcast_ref::<Vec<u8>>()?;
        let (rect, params) = decode_payload(payload)?;
        let expected = self.spec.program().params_size();
        if params.len() != expected {
            self.fail(&format!(
                "the command carries {} parameter bytes but the shader declares {expected}",
                params.len()
            ));
            return None;
        }
        let start = self.arena.len();
        self.arena.extend_from_slice(params);
        let (clip_rect, clip_radii) = context.clip.unwrap_or(([0.0, 0.0, -1.0, 0.0], [0.0; 4]));
        self.requests.push(Request {
            rect: context.map_rect(rect),
            clip_rect,
            clip_radii,
            opacity: context.alpha,
            params: start..self.arena.len(),
        });
        Some(self.requests.len() - 1)
    }

    fn prepare(&mut self, ctx: &RenderContextGeneric<'_, B>) {
        if self.requests.is_empty() {
            return;
        }
        self.target = (ctx.width, ctx.height);
        let rebuild = match &self.state {
            State::Failed => return,
            State::Unbuilt => true,
            State::Ready(gpu) => gpu.format != ctx.format,
        };
        if rebuild {
            match self.build(ctx) {
                Ok(gpu) => {
                    self.stride = uniform_stride(
                        self.spec.program().params_size(),
                        ctx.backend.limits().min_uniform_buffer_offset_alignment,
                    );
                    self.state = State::Ready(gpu);
                }
                Err(message) => return self.fail(&message),
            }
        }
        let State::Ready(gpu) = &mut self.state else { return };
        let backend = ctx.backend;
        let stride = self.stride;

        let wants_capture = gpu.backdrop.is_some()
            && ctx.source_texture.is_some()
            && u64::from(ctx.width) * u64::from(ctx.height) <= MAX_BACKDROP_PIXELS
            && ctx.width <= backend.limits().max_texture_dimension_2d
            && ctx.height <= backend.limits().max_texture_dimension_2d;
        let mut layout_changed = false;
        if let Some(backdrop) = &mut gpu.backdrop {
            let (extent, format) = if wants_capture {
                ((ctx.width.max(1), ctx.height.max(1)), ctx.format)
            } else {
                ((NEUTRAL_EXTENT, NEUTRAL_EXTENT), B::rgba8_unorm_format())
            };
            if backdrop.extent != extent || backdrop.format != format {
                (backdrop.texture, backdrop.view) = backdrop_texture(backend, format, extent);
                backdrop.extent = extent;
                backdrop.format = format;
                layout_changed = true;
            }
            backdrop.captured = wants_capture;
        }
        if self.requests.len() > gpu.uniform_capacity {
            gpu.uniform_capacity = self.requests.len().next_power_of_two().min(MAX_REQUESTS_PER_FRAME);
            gpu.uniform_buffer = uniform_buffer(backend, stride, gpu.uniform_capacity);
            layout_changed = true;
        }
        if layout_changed {
            gpu.bind_group = bind_group(
                backend,
                &gpu.bind_group_layout,
                &gpu.uniform_buffer,
                stride,
                gpu.backdrop.as_ref(),
            );
        }

        let stride = stride as usize;
        self.upload.clear();
        self.upload.resize(stride * self.requests.len(), 0);
        for (index, request) in self.requests.iter().enumerate() {
            let slot = &mut self.upload[index * stride..(index + 1) * stride];
            Header {
                target_size: [ctx.width as f32, ctx.height as f32],
                surface_is_srgb: if ctx.is_srgb { 1.0 } else { 0.0 },
                opacity: request.opacity,
                rect: request.rect,
                clip_rect: request.clip_rect,
                clip_radii: request.clip_radii,
            }
            .write_to(slot);
            slot[HEADER_SIZE..HEADER_SIZE + request.params.len()].copy_from_slice(&self.arena[request.params.clone()]);
        }
        backend.write_buffer(&gpu.uniform_buffer, 0, &self.upload);
    }

    fn render<'pass>(&'pass self, _pass: &mut B::RenderPass<'pass>) {}

    fn render_command<'pass>(&'pass self, command_index: Option<usize>, pass: &mut B::RenderPass<'pass>) {
        let State::Ready(gpu) = &self.state else { return };
        let Some(index) = command_index.filter(|index| *index < self.requests.len()) else {
            return;
        };
        pass.set_pipeline(&gpu.pipeline);
        pass.set_bind_group(0, &gpu.bind_group, &[(index as u64 * self.stride) as u32]);
        pass.draw(0..6, 0..1);
    }

    fn needs_backdrop(&self, command_index: Option<usize>) -> bool {
        let State::Ready(gpu) = &self.state else { return false };
        gpu.backdrop.as_ref().is_some_and(|backdrop| backdrop.captured)
            && command_index.is_some_and(|index| index < self.requests.len())
    }

    fn capture_backdrop_command(
        &self,
        command_index: Option<usize>,
        backend: &B,
        encoder: &mut B::CommandEncoder,
        source_texture: &B::Texture,
        source_offset: (u32, u32),
        width: u32,
        height: u32,
    ) {
        let State::Ready(gpu) = &self.state else { return };
        let Some(backdrop) = gpu.backdrop.as_ref().filter(|backdrop| backdrop.captured) else {
            return;
        };
        let Some(request) = command_index.and_then(|index| self.requests.get(index)) else {
            return;
        };
        // The backdrop texture mirrors the target, so a region lands where it
        // came from and the shader can index it by target pixel.
        let (width, height) = (width.min(backdrop.extent.0), height.min(backdrop.extent.1));
        let Some((x, y, w, h)) = backdrop_copy(request.rect, width, height) else {
            return;
        };
        backend.copy_texture_to_texture(
            encoder,
            &TexelCopyTextureInfo {
                texture: source_texture,
                mip_level: 0,
                origin: Origin3d {
                    x: x + source_offset.0,
                    y: y + source_offset.1,
                    z: 0,
                },
                aspect: TextureAspect::All,
            },
            &TexelCopyTextureInfo {
                texture: &backdrop.texture,
                mip_level: 0,
                origin: Origin3d { x, y, z: 0 },
                aspect: TextureAspect::All,
            },
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    fn has_work(&self) -> bool {
        !self.requests.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_holds_the_header_and_parameters_rounded_to_the_alignment() {
        assert_eq!(uniform_stride(16, 256), 256);
        assert_eq!(uniform_stride(16, 16), 80);
        assert_eq!(uniform_stride(0, 16), 64);
        assert_eq!(uniform_stride(192, 256), 256);
        assert_eq!(uniform_stride(193, 256), 512);
        // A backend that reports no alignment is not a divide by zero.
        assert_eq!(uniform_stride(16, 0), 80);
    }

    #[test]
    fn the_backdrop_copy_is_clamped_to_the_target() {
        assert_eq!(backdrop_copy([10.2, 20.7, 30.0, 40.0], 200, 200), Some((10, 20, 31, 41)));
        assert_eq!(backdrop_copy([-5.0, -5.0, 20.0, 20.0], 100, 100), Some((0, 0, 15, 15)));
        assert_eq!(backdrop_copy([90.0, 90.0, 50.0, 50.0], 100, 100), Some((90, 90, 10, 10)));
    }

    #[test]
    fn a_rect_outside_or_empty_copies_nothing() {
        assert_eq!(backdrop_copy([200.0, 0.0, 10.0, 10.0], 100, 100), None);
        assert_eq!(backdrop_copy([-50.0, 0.0, 10.0, 10.0], 100, 100), None);
        assert_eq!(backdrop_copy([10.0, 10.0, 0.0, 10.0], 100, 100), None);
        assert_eq!(backdrop_copy([10.0, 10.0, 10.0, 10.0], 0, 0), None);
    }
}
