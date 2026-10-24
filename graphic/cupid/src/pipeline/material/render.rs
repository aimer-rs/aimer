use std::{any::Any, mem::size_of};

use bytemuck::{Pod, Zeroable};

use super::{
    MATERIAL_PIPELINE_NAME, MAX_INTERMEDIATE_DIMENSION, MAX_INTERMEDIATE_PIXELS,
    MagnificationStyle, MaterialKind, MaterialRenderPath, MaterialRequest, plan_material,
};

mod blur;
mod downsample;

use downsample::{blur_downsample, reduced_extent};

#[cfg(test)]
mod light_edge_tests;

#[cfg(test)]
mod blur_reduction_tests;

#[cfg(test)]
mod gradient_tests;

const INITIAL_REQUEST_CAPACITY: usize = 16;
const MAX_REQUESTS_PER_FRAME: usize = 512;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MaterialUniform {
    bounds: [f32; 4],
    tint: [f32; 4],
    border_color: [f32; 4],
    shadow: [f32; 4],
    // Magnification uses z/w for axes; LightEdge uses them for light direction.
    effect: [f32; 4],
    light: [f32; 4],
    detail: [f32; 4],
    radii: [f32; 4],
    clip_rect: [f32; 4],
    clip_radii: [f32; 4],
    viewport: [f32; 4],
    backdrop_rect: [f32; 4],
    // x: blob amount, y: blob seed, z: magnification, w: tip pull
    // Magnification: x/y physical anchor, z uniform zoom, w bulge strength.
    liquid: [f32; 4],
    // x: chromatic aberration, y: bevel radius/depth, z/w: Gaussian recurrence
    // Magnification reuses z for its corner-bounded bevel width.
    liquid2: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
struct BackdropRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl MaterialUniform {
    fn from_request(
        request: MaterialRequest,
        width: u32,
        height: u32,
        is_srgb: bool,
        backdrop_region: Option<BackdropRegion>,
    ) -> Self {
        let request = request.normalized();
        let (clip_rect, clip_radii) = request
            .clip
            .map(|clip| (clip.rect, clip.corner_radii))
            .unwrap_or(([0.0, 0.0, -1.0, 0.0], [0.0; 4]));
        let mut uniform = Self {
            bounds: request.bounds,
            tint: request.tint,
            border_color: request.border_color,
            shadow: [
                request.shadow_color[0],
                request.shadow_color[1],
                request.shadow_color[2],
                request.shadow_color[3],
            ],
            effect: {
                let [z, w] = if request.kind == MaterialKind::Magnification {
                    request.magnification_style.axes()
                } else if request.kind == MaterialKind::LightEdge {
                    let (sin, cos) = request.light_angle.to_radians().sin_cos();
                    [cos, sin]
                } else {
                    [request.effective_phase(), request.distortion_strength]
                };
                [request.kind as u8 as f32, request.opacity, z, w]
            },
            light: [
                request.saturation,
                request.brightness,
                request.contrast,
                request.edge_lighting,
            ],
            detail: [
                request.specular_highlight,
                request.interaction,
                blur_radius_pixels(&request),
                if request.kind == MaterialKind::LightEdge {
                    (request.border_width * request_scale(&request))
                        .min(request.bounds[2].min(request.bounds[3]) * 0.5)
                } else {
                    request.border_width
                },
            ],
            radii: if request.kind == MaterialKind::LightEdge {
                let scale = request_scale(&request);
                request.corner_radii.map(|radius| {
                    (radius * scale).min(request.bounds[2].min(request.bounds[3]) * 0.5)
                })
            } else {
                request.corner_radii
            },
            clip_rect,
            clip_radii,
            // `w` tells the shader whether the bound texture contains the
            // current frame or only the neutral fallback texel.
            viewport: [
                width as f32,
                height as f32,
                if is_srgb { 1.0 } else { 0.0 },
                if backdrop_region.is_some() { 1.0 } else { 0.0 },
            ],
            backdrop_rect: backdrop_region.map_or([0.0; 4], |region| {
                [
                    region.x as f32,
                    region.y as f32,
                    region.width as f32,
                    region.height as f32,
                ]
            }),
            liquid: if request.kind == MaterialKind::Magnification {
                let style = request.magnification_style;
                let anchor = style.anchor();
                [
                    request.bounds[0] + request.bounds[2] * anchor[0],
                    request.bounds[1] + request.bounds[3] * anchor[1],
                    if style.contains(MagnificationStyle::UNIFORM) {
                        request.magnification
                    } else {
                        1.0
                    },
                    if style.contains(MagnificationStyle::BULGE) {
                        request.bulge_strength
                    } else {
                        0.0
                    },
                ]
            } else {
                [
                    request.blob_amount,
                    request.blob_seed,
                    request.magnification,
                    request.tip_pull,
                ]
            },
            liquid2: {
                if request.kind == MaterialKind::Magnification {
                    let depth = lens_thickness_pixels(&request);
                    [
                        request.chromatic_aberration,
                        depth,
                        lens_bevel_width_pixels(&request, depth),
                        0.0,
                    ]
                } else {
                    let sigma = (blur_radius_pixels(&request) / 3.0).max(0.001);
                    let ratio = if request.kind == MaterialKind::BackdropFilter {
                        (-0.5 / (sigma * sigma)).exp()
                    } else {
                        0.0
                    };
                    [
                        request.chromatic_aberration,
                        request.bevel_radius,
                        ratio,
                        ratio * ratio,
                    ]
                }
            },
        };
        if request.kind == MaterialKind::Gradient {
            uniform.apply_gradient(&request.gradient);
        }
        if request.kind == MaterialKind::BackdropFilter && request.blur_radius > 0.0 {
            uniform.set_vertical_blur(&request);
        }
        uniform
    }

    /// Stores a gradient ramp in the slots a gradient never otherwise reads,
    /// so analytic ramps fit the uniform every material shares.
    ///
    /// | field          | gradient use                                       |
    /// |----------------|----------------------------------------------------|
    /// | `tint`         | geometry in pixels from the bounds origin          |
    /// | `light`        | offsets of stops 0..4                              |
    /// | `detail`       | offsets of stops 4..8                              |
    /// | `border_color` | packed RGBA8 colors of stops 0..4, as `f32` bits   |
    /// | `shadow`       | packed RGBA8 colors of stops 4..8, as `f32` bits   |
    /// | `liquid`       | x: shape, y: stop count                            |
    ///
    /// The packed colors are raw bit patterns that the shader reads back with
    /// a bit cast; nothing does float arithmetic on them.
    fn apply_gradient(&mut self, gradient: &super::MaterialGradient) {
        let bits = |colors: &[u32]| -> [f32; 4] {
            [0, 1, 2, 3].map(|index| f32::from_bits(colors[index]))
        };
        self.effect[2] = 0.0;
        self.effect[3] = 0.0;
        self.tint = gradient.geometry;
        self.light = [0, 1, 2, 3].map(|index| gradient.offsets[index]);
        self.detail = [4, 5, 6, 7].map(|index| gradient.offsets[index]);
        self.border_color = bits(&gradient.colors[..4]);
        self.shadow = bits(&gradient.colors[4..]);
        self.liquid = [
            gradient.shape as u8 as f32,
            f32::from(gradient.stop_count),
            0.0,
            0.0,
        ];
    }
}

/// Cupid-owned procedural material renderer.
///
/// The stage captures only the material's expanded bounds at each ordered
/// material boundary, then samples that bounded texture for frosted Glass blur
/// or Liquid refraction. This works for both analytic and multisampled
/// rendering. An unsupported or over-budget request keeps a small neutral
/// texture bound and retains the analytic surface treatment.
/// Plain backdrop filters reuse a horizontal Gaussian intermediate, then blur
/// vertically in the replacement pass. They leave the target unchanged
/// when capture is unavailable.
/// Magnification samples the capture directly with configurable zoom, bulge,
/// and bounded rounded-edge refraction, without a Gaussian intermediate.
/// LightEdge draws an analytic inward highlight without capturing the backdrop.
pub struct MaterialPipeline<B: crate::backend::GpuBackend = crate::backend::DefaultGpuBackend> {
    pipeline: B::RenderPipeline,
    // A filtered capture already contains the backdrop's alpha and must replace it.
    filter_pipeline: Option<B::RenderPipeline>,
    blur: Option<blur::BackdropBlur<B>>,
    antialiasing: crate::AntiAlias,
    has_filter: bool,
    bind_group_layout: B::BindGroupLayout,
    bind_group: B::BindGroup,
    uniform_buffer: B::Buffer,
    uniform_stride: u64,
    uniform_capacity: usize,
    backdrop_texture: B::Texture,
    _backdrop_view: B::TextureView,
    backdrop_sampler: B::Sampler,
    backdrop_width: u32,
    backdrop_height: u32,
    backdrop_format: B::TextureFormat,
    backdrop_available: bool,
    max_capture_dimension: u32,
    requests: Vec<MaterialRequest>,
    backdrop_regions: Vec<Option<BackdropRegion>>,
    upload: Vec<u8>,
}

// ── Backend-generic pipeline implementation ─────────────────────────────────

impl<B: crate::backend::GpuBackend> MaterialPipeline<B> {
    /// Creates the pipeline and its GPU resources through the selected
    /// [`GpuBackend`].
    pub fn new(backend: &B, format: B::TextureFormat, antialiasing: crate::AntiAlias) -> Self {
        use crate::backend::*;

        let bind_group_layout = backend.create_bind_group_layout(&[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: vec![ShaderStage::Vertex, ShaderStage::Fragment],
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: vec![ShaderStage::Fragment],
                ty: BindingType::Texture {
                    multisampled: false,
                    view_dimension: TextureViewDimension::D2,
                    sample_type: TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 3,
                visibility: vec![ShaderStage::Fragment],
                ty: BindingType::Texture {
                    multisampled: false,
                    view_dimension: TextureViewDimension::D2,
                    sample_type: TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: vec![ShaderStage::Fragment],
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ]);

        let backdrop_texture = backend.create_texture(&TextureDescriptor {
            label: Some("material neutral backdrop".to_string()),
            size: (1, 1, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: B::rgba8_unorm_format(),
            usage: vec![TextureUsage::TextureBinding, TextureUsage::CopyDst],
        });
        let backdrop_view =
            backend.create_texture_view(&backdrop_texture, "material neutral backdrop view");
        let backdrop_sampler = backend.create_sampler(&SamplerDescriptor {
            label: Some("material backdrop sampler".to_string()),
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

        let uniform_stride = aligned_uniform_stride_generic(backend);
        let uniform_capacity = INITIAL_REQUEST_CAPACITY;
        let uniform_buffer =
            create_uniform_buffer_generic(backend, uniform_stride, uniform_capacity);
        let bind_group = create_bind_group(
            backend,
            &bind_group_layout,
            &uniform_buffer,
            uniform_stride,
            &backdrop_view,
            &backdrop_sampler,
            &backdrop_view,
        );

        let pipeline =
            create_material_pipeline(backend, &bind_group_layout, format, antialiasing, false);

        Self {
            pipeline,
            filter_pipeline: None,
            blur: None,
            antialiasing,
            has_filter: false,
            bind_group_layout,
            bind_group,
            uniform_buffer,
            uniform_stride,
            uniform_capacity,
            backdrop_texture,
            _backdrop_view: backdrop_view,
            backdrop_sampler,
            backdrop_width: 1,
            backdrop_height: 1,
            backdrop_format: B::rgba8_unorm_format(),
            backdrop_available: false,
            max_capture_dimension: backend.limits().max_texture_dimension_2d,
            requests: Vec::with_capacity(INITIAL_REQUEST_CAPACITY),
            backdrop_regions: Vec::with_capacity(INITIAL_REQUEST_CAPACITY),
            upload: Vec::new(),
        }
    }

    /// Backend-driven equivalent of [`ensure_backdrop_texture`].
    fn ensure_backdrop_texture_generic(
        &mut self,
        backend: &B,
        format: B::TextureFormat,
        width: u32,
        height: u32,
        available: bool,
    ) {
        use crate::backend::*;

        let (width, height, format) = if available {
            (width.max(1), height.max(1), format)
        } else {
            (1, 1, B::rgba8_unorm_format())
        };
        if self.backdrop_width == width
            && self.backdrop_height == height
            && self.backdrop_format == format
        {
            return;
        }

        let texture = backend.create_texture(&TextureDescriptor {
            label: Some(
                if available {
                    "material captured backdrop"
                } else {
                    "material neutral backdrop"
                }
                .to_string(),
            ),
            size: (width, height, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: vec![TextureUsage::TextureBinding, TextureUsage::CopyDst],
        });
        let view = backend.create_texture_view(&texture, "material backdrop view");
        self.bind_group = create_bind_group(
            backend,
            &self.bind_group_layout,
            &self.uniform_buffer,
            self.uniform_stride,
            &view,
            &self.backdrop_sampler,
            self.blur.as_ref().map_or(&view, |blur| &blur.view),
        );
        self.backdrop_texture = texture;
        self._backdrop_view = view;
        self.backdrop_width = width;
        self.backdrop_height = height;
        self.backdrop_format = format;
    }

    /// Backend-driven equivalent of [`ensure_uniform_capacity`].
    fn ensure_uniform_capacity_generic(&mut self, backend: &B, required: usize) {
        if required <= self.uniform_capacity {
            return;
        }
        let capacity = required.next_power_of_two().min(MAX_REQUESTS_PER_FRAME);
        self.uniform_capacity = capacity;
        self.uniform_buffer = create_uniform_buffer_generic(backend, self.uniform_stride, capacity);
        self.bind_group = create_bind_group(
            backend,
            &self.bind_group_layout,
            &self.uniform_buffer,
            self.uniform_stride,
            &self._backdrop_view,
            &self.backdrop_sampler,
            self.blur
                .as_ref()
                .map_or(&self._backdrop_view, |blur| &blur.view),
        );
    }

    /// Builds the pipeline a backdrop filter draws with, ahead of the first one.
    ///
    /// Left alone, the first frame with a filter in it builds the pipeline, and
    /// that compiles the material shader in the middle of whatever animation
    /// brought the filter in. Building it here moves the same work to a frame
    /// nobody is watching.
    pub(crate) fn prewarm(&mut self, backend: &B, format: B::TextureFormat) {
        if self.filter_pipeline.is_none() {
            self.filter_pipeline = Some(create_material_pipeline(
                backend,
                &self.bind_group_layout,
                format,
                self.antialiasing,
                true,
            ));
        }
    }

    /// Equivalent to [`CustomPipeline::prepare`] but routes GPU operations
    /// through the [`GpuBackend`] trait.
    pub fn prepare(&mut self, ctx: &crate::custom_pipeline::RenderContextGeneric<'_, B>) {
        if self.requests.is_empty() {
            return;
        }
        if self.has_filter && self.filter_pipeline.is_none() {
            self.filter_pipeline = Some(create_material_pipeline(
                ctx.backend,
                &self.bind_group_layout,
                ctx.format,
                self.antialiasing,
                true,
            ));
        }
        let mut capture_extent = [1, 1];
        self.backdrop_regions
            .extend(self.requests.iter().copied().map(|request| {
                ctx.source_texture
                    .and_then(|_| {
                        backdrop_region(request, ctx.width, ctx.height, self.max_capture_dimension)
                    })
                    .and_then(|region| {
                        include_backdrop_region(
                            region,
                            &mut capture_extent,
                            self.max_capture_dimension,
                        )
                    })
            }));
        self.backdrop_available = self.backdrop_regions.iter().any(Option::is_some);
        let [backdrop_width, backdrop_height] = capture_extent;
        let capture_changed = self.backdrop_width != backdrop_width
            || self.backdrop_height != backdrop_height
            || self.backdrop_format != ctx.format;
        let uniform_changed = self.requests.len() > self.uniform_capacity;
        self.ensure_backdrop_texture_generic(
            ctx.backend,
            ctx.format,
            backdrop_width,
            backdrop_height,
            self.backdrop_available,
        );
        self.ensure_uniform_capacity_generic(ctx.backend, self.requests.len());
        let needs_blur =
            self.requests
                .iter()
                .zip(&self.backdrop_regions)
                .any(|(request, region)| {
                    request.kind == MaterialKind::BackdropFilter
                        && request.blur_radius > 0.0
                        && region.is_some()
                });
        if needs_blur {
            let new_blur = self.blur.is_none();
            let mut blur = self
                .blur
                .take()
                .unwrap_or_else(|| blur::BackdropBlur::new(self, ctx.backend));
            let texture_changed = blur.prepare(self, ctx.backend, capture_changed);
            if new_blur || texture_changed || capture_changed || uniform_changed {
                self.bind_group = create_bind_group(
                    ctx.backend,
                    &self.bind_group_layout,
                    &self.uniform_buffer,
                    self.uniform_stride,
                    &self._backdrop_view,
                    &self.backdrop_sampler,
                    &blur.view,
                );
            }
            self.blur = Some(blur);
        }

        let stride = self.uniform_stride as usize;
        self.upload
            .resize(stride.saturating_mul(self.requests.len()), 0);
        for (index, request) in self.requests.iter().copied().enumerate() {
            let uniform = MaterialUniform::from_request(
                request,
                ctx.width,
                ctx.height,
                ctx.is_srgb,
                self.backdrop_regions[index],
            );
            let start = index * stride;
            let end = start + size_of::<MaterialUniform>();
            self.upload[start..end].copy_from_slice(bytemuck::bytes_of(&uniform));
        }
        ctx.backend
            .write_buffer(&self.uniform_buffer, 0, &self.upload);

        if !self.backdrop_available
            && self
                .requests
                .iter()
                .any(|request| request.kind.samples_backdrop())
        {
            ctx.backend
                .write_texture(&crate::backend::WriteTextureDescriptor {
                    texture: &self.backdrop_texture,
                    mip_level: 0,
                    origin: crate::backend::Origin3d::ZERO,
                    aspect: crate::backend::TextureAspect::All,
                    data: &[18, 28, 44, 255],
                    buffer_layout: crate::backend::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    extent: crate::backend::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                });
        }
    }

    /// Backend-driven equivalent of [`CustomPipeline::capture_backdrop_command`].
    ///
    /// Routes the texture-to-texture copy through the backend trait so it stays
    /// compatible with the generic path.
    pub fn capture_backdrop_command_generic(
        &self,
        command_index: Option<usize>,
        backend: &B,
        encoder: &mut B::CommandEncoder,
        source_texture: &B::Texture,
        source_offset: (u32, u32),
        _width: u32,
        _height: u32,
    ) {
        use crate::backend::{Extent3d, Origin3d, TexelCopyTextureInfo, TextureAspect};
        let Some(region) = command_index
            .and_then(|index| self.backdrop_regions.get(index))
            .copied()
            .flatten()
        else {
            return;
        };
        let (x, y) = backdrop_source_origin(region, source_offset);
        backend.copy_texture_to_texture(
            encoder,
            &TexelCopyTextureInfo {
                texture: source_texture,
                mip_level: 0,
                origin: Origin3d { x, y, z: 0 },
                aspect: TextureAspect::All,
            },
            &TexelCopyTextureInfo {
                texture: &self.backdrop_texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            Extent3d {
                width: region.width,
                height: region.height,
                depth_or_array_layers: 1,
            },
        );
        if let Some(index) = command_index {
            let request = self.requests[index];
            if request.kind == MaterialKind::BackdropFilter && request.blur_radius > 0.0 {
                if let Some(blur) = &self.blur {
                    blur.encode(index, backend, encoder);
                }
            }
        }
    }

    fn request_from_payload_generic(data: &(dyn Any + Send)) -> Option<MaterialRequest> {
        data.downcast_ref::<MaterialRequest>().copied().or_else(|| {
            data.downcast_ref::<Vec<u8>>()
                .and_then(|bytes| MaterialRequest::decode(bytes).ok())
        })
    }

    fn render_command_generic<'pass>(
        &'pass self,
        command_index: Option<usize>,
        pass: &mut B::RenderPass<'pass>,
    ) where
        B::RenderPass<'pass>: crate::backend::GpuRenderPass<B>,
    {
        use crate::backend::GpuRenderPass;
        let Some(index) = command_index.filter(|index| *index < self.requests.len()) else {
            return;
        };
        let pipeline = if replaces_backdrop(self.requests[index].kind) {
            let Some(pipeline) = &self.filter_pipeline else {
                return;
            };
            pipeline
        } else {
            &self.pipeline
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(
            0,
            &self.bind_group,
            &[(index as u32).saturating_mul(self.uniform_stride as u32)],
        );
        pass.draw(0..6, 0..1);
    }
}

impl<B: crate::backend::GpuBackend> crate::custom_pipeline::CustomPipelineGeneric<B>
    for MaterialPipeline<B>
{
    fn name(&self) -> &str {
        MATERIAL_PIPELINE_NAME
    }

    fn prepare(&mut self, ctx: &crate::custom_pipeline::RenderContextGeneric<'_, B>) {
        self.prepare(ctx);
    }

    fn begin_frame(&mut self) {
        self.has_filter = false;
        self.requests.clear();
        self.backdrop_regions.clear();
        self.upload.clear();
    }

    fn prepare_command(&mut self, data: &(dyn Any + Send)) -> Option<usize> {
        let request = Self::request_from_payload_generic(data)?;
        let mut capabilities = super::MaterialCapabilities::default();
        if replaces_backdrop(request.kind) {
            capabilities.max_surface_dimension = self.max_capture_dimension;
        }
        if plan_material(request, capabilities).path != MaterialRenderPath::Gpu
            || self.requests.len() >= MAX_REQUESTS_PER_FRAME
        {
            return None;
        }
        let index = self.requests.len();
        self.has_filter |= replaces_backdrop(request.kind);
        self.requests.push(request);
        Some(index)
    }

    fn render<'pass>(&'pass self, _pass: &mut B::RenderPass<'pass>) {}

    fn render_command<'pass>(
        &'pass self,
        command_index: Option<usize>,
        pass: &mut B::RenderPass<'pass>,
    ) {
        self.render_command_generic(command_index, pass);
    }

    fn needs_backdrop(&self, command_index: Option<usize>) -> bool {
        self.backdrop_available
            && command_index.is_some_and(|index| {
                self.backdrop_regions
                    .get(index)
                    .is_some_and(Option::is_some)
            })
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
        self.capture_backdrop_command_generic(
            command_index,
            backend,
            encoder,
            source_texture,
            source_offset,
            width,
            height,
        );
    }

    fn has_work(&self) -> bool {
        !self.requests.is_empty()
    }
}

fn create_material_pipeline<B: crate::backend::GpuBackend>(
    backend: &B,
    bind_group_layout: &B::BindGroupLayout,
    format: B::TextureFormat,
    antialiasing: crate::AntiAlias,
    replace_backdrop: bool,
) -> B::RenderPipeline {
    use crate::backend::*;
    let shader = backend.create_shader_module(
        backend.builtin_shader_source(BuiltinShader::Material),
        "material shader",
    );
    let pipeline_layout = backend.create_pipeline_layout(&[bind_group_layout]);
    backend.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some(
            if replace_backdrop {
                "backdrop filter pipeline"
            } else {
                "material pipeline"
            }
            .to_string(),
        ),
        layout: Some(&pipeline_layout),
        vertex: VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[],
        },
        fragment: Some(FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[Some(ColorTargetState {
                format,
                blend: if replace_backdrop {
                    None
                } else {
                    Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING)
                },
                write_mask: ColorWriteMask::ALL,
            })],
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: crate::pipeline::multisample_state(antialiasing),
    })
}

fn backdrop_fits_budget(width: u32, height: u32, max_dimension: u32) -> bool {
    width > 0
        && height > 0
        && width <= max_dimension
        && height <= max_dimension
        && u64::from(width)
            .checked_mul(u64::from(height))
            .is_some_and(|pixels| pixels <= MAX_INTERMEDIATE_PIXELS)
}

fn include_backdrop_region(
    region: BackdropRegion,
    extent: &mut [u32; 2],
    max_dimension: u32,
) -> Option<BackdropRegion> {
    let width = extent[0].max(region.width);
    let height = extent[1].max(region.height);
    // Ordered captures share one texture. Individually valid wide and tall
    // regions must not combine into an allocation beyond the pixel budget.
    if !backdrop_fits_budget(width, height, max_dimension) {
        return None;
    }
    *extent = [width, height];
    Some(region)
}

#[inline]
fn replaces_backdrop(kind: MaterialKind) -> bool {
    matches!(
        kind,
        MaterialKind::BackdropFilter | MaterialKind::Magnification
    )
}

#[inline]
fn lens_thickness_pixels(request: &MaterialRequest) -> f32 {
    if !request
        .magnification_style
        .contains(MagnificationStyle::REFRACTION)
    {
        return 0.0;
    }
    // Keep thickness logical through bridge normalization so display scaling
    // cannot clamp it back to 512 device pixels before the geometry limit.
    let m = &request.transform;
    let scale_x = (m[0] * m[0] + m[1] * m[1]).sqrt();
    let scale_y = (m[3] * m[3] + m[4] * m[4]).sqrt();
    (request.bevel_radius * (scale_x + scale_y) * 0.5)
        .min(request.bounds[2].min(request.bounds[3]) * 0.5)
}

#[inline]
fn lens_bevel_width_pixels(request: &MaterialRequest, depth: f32) -> f32 {
    // An inset beyond a rounded corner's radius crosses the SDF's medial
    // axis, where the nearest-edge normal jumps. Flatten the bevel before
    // that axis while retaining optical depth for Snell displacement.
    // Zero radii describe deliberately sharp corners and impose no limit.
    request
        .corner_radii
        .iter()
        .copied()
        .filter(|radius| *radius > 0.0)
        .fold(depth, f32::min)
}

#[inline]
fn blur_radius_pixels(request: &MaterialRequest) -> f32 {
    if request.kind != MaterialKind::BackdropFilter {
        return request.blur_radius;
    }
    // Bounds are resolved to device pixels, but the filter radius remains
    // logical so normalization cannot cap a scaled radius back to 96 pixels.
    let m = &request.transform;
    let scale_x = (m[0] * m[0] + m[1] * m[1]).sqrt();
    let scale_y = (m[3] * m[3] + m[4] * m[4]).sqrt();
    request.blur_radius * (scale_x + scale_y) * 0.5
}

#[inline]
fn request_scale(request: &MaterialRequest) -> f32 {
    let m = &request.transform;
    ((m[0] * m[0] + m[1] * m[1]).sqrt() + (m[3] * m[3] + m[4] * m[4]).sqrt()) * 0.5
}

/// Where in the source texture a target-local backdrop `region` is read from.
///
/// Inside an offscreen opacity group the region is relative to the group's
/// bounds, which sit at `source_offset` within the whole-frame source.
#[inline]
fn backdrop_source_origin(region: BackdropRegion, source_offset: (u32, u32)) -> (u32, u32) {
    (
        region.x.saturating_add(source_offset.0),
        region.y.saturating_add(source_offset.1),
    )
}

fn backdrop_region(
    request: MaterialRequest,
    viewport_width: u32,
    viewport_height: u32,
    max_dimension: u32,
) -> Option<BackdropRegion> {
    if !request.kind.samples_backdrop() || viewport_width == 0 || viewport_height == 0 {
        return None;
    }
    let request = request.normalized();
    if request.kind == MaterialKind::BackdropFilter
        && blur_radius_pixels(&request) > MAX_INTERMEDIATE_DIMENSION as f32
    {
        return None;
    }
    let blur_expansion = blur_radius_pixels(&request).ceil().max(1.0);
    // Keep the ripple term in sync with the summed amplitudes in
    // `liquid_warp`: x = 0.006 + 0.004, y = 0.0055 + 0.0035. The bevel term
    // covers `liquid_bevel_refract_offset`, whose displacement is bounded by
    // roughly its fake depth (`bevel_radius * (1 + magnification)`),
    // strongest at the steepest part of the bevel; the margin here is
    // generous on purpose so refracted samples never clamp against the
    // capture edge, and an unreasonably large combination simply falls back
    // through the existing intermediate-budget check below.
    let (expansion_x, expansion_y) = if request.kind == MaterialKind::Magnification {
        let margin = (2.0 * lens_thickness_pixels(&request)).ceil() + 1.0;
        (margin, margin)
    } else if request.kind == MaterialKind::Liquid {
        let ripple_x = (request.bounds[2] * 0.010 * request.distortion_strength).ceil();
        let ripple_y = (request.bounds[3] * 0.009 * request.distortion_strength).ceil();
        let bevel_expansion = (request.bevel_radius * (1.0 + request.magnification) * 2.5).ceil();
        (
            blur_expansion.max(ripple_x).max(bevel_expansion),
            blur_expansion.max(ripple_y).max(bevel_expansion),
        )
    } else {
        (blur_expansion, blur_expansion)
    };
    let left = (request.bounds[0] - expansion_x)
        .floor()
        .clamp(0.0, viewport_width as f32) as u32;
    let top = (request.bounds[1] - expansion_y)
        .floor()
        .clamp(0.0, viewport_height as f32) as u32;
    let right = (request.bounds[0] + request.bounds[2] + expansion_x)
        .ceil()
        .clamp(0.0, viewport_width as f32) as u32;
    let bottom = (request.bounds[1] + request.bounds[3] + expansion_y)
        .ceil()
        .clamp(0.0, viewport_height as f32) as u32;
    let width = right.saturating_sub(left);
    let height = bottom.saturating_sub(top);
    // Replacement effects use full-resolution captures. Wide, shallow bars
    // can exceed 2,048 device pixels while staying within the pixel budget.
    let max_dimension = if replaces_backdrop(request.kind) {
        max_dimension
    } else {
        max_dimension.min(MAX_INTERMEDIATE_DIMENSION)
    };
    backdrop_fits_budget(width, height, max_dimension).then_some(BackdropRegion {
        x: left,
        y: top,
        width,
        height,
    })
}

// ── Generic helper functions ────────────────────────────────────────────────

fn aligned_uniform_stride_generic<B: crate::backend::GpuBackend>(backend: &B) -> u64 {
    let alignment = u64::from(backend.limits().min_uniform_buffer_offset_alignment.max(1));
    let size = size_of::<MaterialUniform>() as u64;
    size.div_ceil(alignment) * alignment
}

fn create_uniform_buffer_generic<B: crate::backend::GpuBackend>(
    backend: &B,
    stride: u64,
    capacity: usize,
) -> B::Buffer {
    use crate::backend::*;
    backend.create_buffer(&BufferDescriptor {
        label: Some("material uniform buffer".to_string()),
        size: stride * capacity as u64,
        usage: vec![BufferUsage::Uniform, BufferUsage::CopyDst],
    })
}

fn create_bind_group<B: crate::backend::GpuBackend>(
    backend: &B,
    layout: &B::BindGroupLayout,
    uniform_buffer: &B::Buffer,
    uniform_stride: u64,
    backdrop_view: &B::TextureView,
    backdrop_sampler: &B::Sampler,
    horizontal_blur_view: &B::TextureView,
) -> B::BindGroup
where
    B: crate::backend::GpuBackend,
{
    use crate::backend::*;
    backend.create_bind_group(
        layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::BufferRange(uniform_buffer, 0, uniform_stride),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(backdrop_view),
            },
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::TextureView(horizontal_blur_view),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Sampler(backdrop_sampler),
            },
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::{MaterialUniform, backdrop_fits_budget, backdrop_region};
    use crate::pipeline::material::{MaterialKind, MaterialRequest};

    #[test]
    fn uniform_layout_fits_one_aligned_dynamic_slot() {
        assert_eq!(size_of::<MaterialUniform>(), 224);
        assert!(size_of::<MaterialUniform>() <= 256);
        assert_eq!(size_of::<MaterialUniform>() % 16, 0);
    }

    #[test]
    fn gaussian_support_is_bounded_even_at_extreme_device_scales() {
        let mut request =
            MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 64.0, 64.0]);
        request.blur_radius = 96.0;
        request.transform[0] = 24.0;
        request.transform[4] = 24.0;
        assert!(backdrop_region(request, 64, 64, 2_048).is_none());
    }

    #[test]
    fn backdrop_capture_obeys_the_frame_budget() {
        assert!(backdrop_fits_budget(1_024, 1_024, 2_048));
        assert!(backdrop_fits_budget(2_048, 2_048, 2_048));
        assert!(!backdrop_fits_budget(2_048, 2_049, 2_048));
        assert!(!backdrop_fits_budget(2_049, 1, 2_048));
    }

    #[test]
    fn backdrop_capture_is_local_to_the_material_on_a_large_viewport() {
        let request = MaterialRequest::new(MaterialKind::Glass, [32.0, 16.0, 64.0, 32.0]);
        let region = backdrop_region(request, 4_096, 2_160, 2_048).expect("local capture region");

        assert_eq!((region.x, region.y), (16, 0));
        assert_eq!((region.width, region.height), (96, 64));
    }

    #[test]
    fn liquid_capture_includes_the_bounded_refraction_envelope() {
        let mut request =
            MaterialRequest::new(MaterialKind::Liquid, [100.0, 100.0, 1_000.0, 1_000.0]);
        request.blur_radius = 0.0;
        request.distortion_strength = 1.0;
        request.magnification = 0.5;
        request.bevel_radius = 40.0;
        let region = backdrop_region(request, 2_000, 2_000, 2_048).expect("liquid capture region");

        // The bevel term (`40 * 1.5 * 2.5 = 150`) dominates the ripple term
        // (`1_000 * 0.010 = 10`), so the captured region grows to cover the
        // bevel refraction's reach rather than clamping its samples.
        assert_eq!((region.x, region.y), (0, 0));
        assert_eq!((region.width, region.height), (1_250, 1_250));
    }
}

#[cfg(test)]
mod magnification_tests;

#[cfg(test)]
mod backdrop_tests;
