use std::borrow::Cow;
use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};

use super::frame_upload::FrameUpload;
use crate::utilities::TextureId;

fn constrained_texture_size(width: u32, height: u32, max_dimension: u32) -> (u32, u32) {
    if width <= max_dimension && height <= max_dimension {
        return (width, height);
    }

    if width >= height {
        (
            max_dimension,
            ((height as u64 * max_dimension as u64) / width as u64).max(1) as u32,
        )
    } else {
        (
            ((width as u64 * max_dimension as u64) / height as u64).max(1) as u32,
            max_dimension,
        )
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn resize_rgba8_nearest(
    source_width: u32,
    source_height: u32,
    data: &[u8],
    target_width: u32,
    target_height: u32,
) -> Vec<u8> {
    let mut resized = vec![0; target_width as usize * target_height as usize * 4];
    let source_offsets = (0..target_width as usize)
        .map(|target_x| target_x * source_width as usize / target_width as usize * 4)
        .collect::<Vec<_>>();
    for (target_y, row) in resized
        .chunks_exact_mut(target_width as usize * 4)
        .enumerate()
    {
        let source_y = target_y * source_height as usize / target_height as usize;
        let source_row_offset = source_y * source_width as usize * 4;
        for (pixel, source_x_offset) in row.chunks_exact_mut(4).zip(&source_offsets) {
            let source_offset = source_row_offset + source_x_offset;
            pixel.copy_from_slice(&data[source_offset..source_offset + 4]);
        }
    }
    resized
}

fn constrain_rgba8(
    width: u32,
    height: u32,
    data: &[u8],
    max_dimension: u32,
) -> (u32, u32, Cow<'_, [u8]>) {
    let expected_len = (width as u64)
        .checked_mul(height as u64)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok());
    if width == 0 || height == 0 || max_dimension == 0 || expected_len != Some(data.len()) {
        return (1, 1, Cow::Owned(vec![0; 4]));
    }

    let (target_width, target_height) = constrained_texture_size(width, height, max_dimension);
    if (target_width, target_height) == (width, height) {
        return (width, height, Cow::Borrowed(data));
    }

    #[cfg(target_arch = "wasm32")]
    let resized = resize_rgba8_nearest(width, height, data, target_width, target_height);
    #[cfg(not(target_arch = "wasm32"))]
    let resized = {
        let source = image::RgbaImage::from_raw(width, height, data.to_vec())
            .expect("validated RGBA image dimensions must match the data length");
        image::imageops::resize(
            &source,
            target_width,
            target_height,
            image::imageops::FilterType::Lanczos3,
        )
        .into_raw()
    };
    (target_width, target_height, Cow::Owned(resized))
}

const fn image_mip_level_count() -> u32 {
    1
}

pub(crate) struct InstanceBufferPolicy {
    initial_capacity: usize,
    capacity: usize,
    underused_frames: u16,
}

impl InstanceBufferPolicy {
    pub(crate) const SHRINK_AFTER_FRAMES: u16 = 120;

    pub(crate) const fn new(initial_capacity: usize) -> Self {
        Self {
            initial_capacity,
            capacity: initial_capacity,
            underused_frames: 0,
        }
    }

    pub(crate) const fn capacity(&self) -> usize {
        self.capacity
    }

    pub(crate) fn record_usage(&mut self, used: usize) {
        let required = self.initial_capacity.max(used.next_power_of_two());
        if required > self.capacity {
            self.capacity = required;
            self.underused_frames = 0;
        } else if required <= self.capacity / 4 {
            self.underused_frames = self.underused_frames.saturating_add(1);
            if self.underused_frames >= Self::SHRINK_AFTER_FRAMES {
                self.capacity = required;
                self.underused_frames = 0;
            }
        } else {
            self.underused_frames = 0;
        }
    }

    pub(crate) fn grow_to_fit(&mut self, required: usize) {
        if required > self.capacity {
            self.capacity = required.next_power_of_two();
            self.underused_frames = 0;
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ImageInstance {
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub uv_offset: [f32; 2],
    pub uv_scale: [f32; 2],
    /// Clip rect: [x, y, width, height]. If width <= 0, no clip is applied.
    pub clip_rect: [f32; 4],
    /// Border radius for the clip rect: [top-left, top-right, bottom-right,
    /// bottom-left].
    pub clip_border_radius: [f32; 4],
    pub alpha: f32,
    /// Whether the sampled source already contains premultiplied color.
    /// Ordinary decoded images are straight-alpha; renderer-owned retained
    /// layers are premultiplied render targets.
    pub source_premultiplied: f32,
}

impl ImageInstance {
    #[cfg(feature = "wgpu")]
    const ATTRIBS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
        0 => Float32x2,
        1 => Float32x2,
        2 => Float32x2,
        3 => Float32x2,
        4 => Float32x4,
        5 => Float32x4,
        6 => Float32,
        7 => Float32,
    ];

    #[cfg(feature = "wgpu")]
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<ImageInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBS,
        }
    }

    const GENERIC_ATTRIBUTES: [crate::backend::VertexAttribute; 8] = [
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, position) as u64,
            shader_location: 0,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, size) as u64,
            shader_location: 1,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, uv_offset) as u64,
            shader_location: 2,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x2,
            offset: std::mem::offset_of!(Self, uv_scale) as u64,
            shader_location: 3,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, clip_rect) as u64,
            shader_location: 4,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32x4,
            offset: std::mem::offset_of!(Self, clip_border_radius) as u64,
            shader_location: 5,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32,
            offset: std::mem::offset_of!(Self, alpha) as u64,
            shader_location: 6,
        },
        crate::backend::VertexAttribute {
            format: crate::backend::VertexFormat::Float32,
            offset: std::mem::offset_of!(Self, source_premultiplied) as u64,
            shader_location: 7,
        },
    ];
}

struct TextureEntry<B: crate::backend::GpuBackend = crate::backend::DefaultGpuBackend> {
    bind_group: B::BindGroup,
    #[allow(dead_code)]
    texture: B::Texture,
    width: u32,
    height: u32,
    bytes: u64,
    last_used_frame: u64,
    evictable: bool,
    retained_revision: Option<u64>,
}

const IMAGE_TEXTURE_CACHE_BUDGET_BYTES: u64 = 128 * 1024 * 1024;
const IMAGE_TEXTURE_IDLE_FRAMES: u64 = 120;

#[derive(Clone, Copy)]
struct TextureCacheEntryInfo {
    id: TextureId,
    bytes: u64,
    last_used_frame: u64,
    evictable: bool,
}

fn select_texture_evictions(
    current_frame: u64,
    budget_bytes: u64,
    idle_frames: u64,
    total_bytes: u64,
    mut entries: Vec<TextureCacheEntryInfo>,
) -> Vec<TextureId> {
    entries.retain(|entry| entry.evictable && entry.last_used_frame < current_frame);
    entries.sort_unstable_by_key(|entry| entry.last_used_frame);

    let mut remaining_bytes = total_bytes;
    let mut evictions = Vec::new();
    for entry in entries {
        let idle = current_frame.saturating_sub(entry.last_used_frame);
        if idle >= idle_frames || remaining_bytes > budget_bytes {
            remaining_bytes = remaining_bytes.saturating_sub(entry.bytes);
            evictions.push(entry.id);
        }
    }
    evictions
}

pub struct ImagePipeline<B: crate::backend::GpuBackend = crate::backend::DefaultGpuBackend> {
    pipeline: B::RenderPipeline,
    /// Draws a texture's alpha as a mask over the destination; made on first
    /// use because only masked groups need it.
    mask_pipeline: Option<B::RenderPipeline>,
    shader: B::ShaderModule,
    pipeline_layout: B::PipelineLayout,
    format: B::TextureFormat,
    antialiasing: crate::AntiAlias,
    viewport_buffer: B::Buffer,
    viewport_bind_group: B::BindGroup,
    texture_bind_group_layout: B::BindGroupLayout,
    sampler: B::Sampler,
    textures: HashMap<TextureId, TextureEntry<B>>,
    next_id: TextureId,
    instance_buffer: B::Buffer,
    instance_policy: InstanceBufferPolicy,
    frame_instance_offset: usize,
    frame_instances: Vec<ImageInstance>,
    upload: FrameUpload<ImageInstance>,
    immediate_uploads: bool,
    last_viewport: Option<(u32, u32, bool)>,
    frame_index: u64,
}

// ── Backend-agnostic generic path ──────────────────────────────────────────
//
// Pipeline resources and uploads use the selected backend's associated types.
impl<B: crate::backend::GpuBackend> ImagePipeline<B> {
    const INITIAL_CAPACITY: usize = 64;

    #[inline]
    const fn get_source() -> &'static str {
        #[cfg(target_os = "android")]
        {
            concat!(
                include_str!("./shaders/wgsl/android_color.wgsl"),
                include_str!("./shaders/wgsl/image.wgsl")
            )
        }
        #[cfg(not(target_os = "android"))]
        {
            concat!(
                include_str!("./shaders/wgsl/color.wgsl"),
                include_str!("./shaders/wgsl/image.wgsl")
            )
        }
    }

    #[inline]
    pub fn has_texture(&self, id: TextureId) -> bool {
        self.textures.contains_key(&id)
    }

    #[inline]
    pub fn remove_texture(&mut self, id: TextureId) -> bool {
        self.textures.remove(&id).is_some()
    }

    #[inline]
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    pub fn texture_bytes(&self) -> u64 {
        self.textures.values().map(|entry| entry.bytes).sum()
    }

    pub(crate) fn eviction_candidates(&self) -> Vec<TextureId> {
        let entries = self
            .textures
            .iter()
            .map(|(&id, entry)| TextureCacheEntryInfo {
                id,
                bytes: entry.bytes,
                last_used_frame: entry.last_used_frame,
                evictable: entry.evictable,
            })
            .collect();
        select_texture_evictions(
            self.frame_index,
            IMAGE_TEXTURE_CACHE_BUDGET_BYTES,
            IMAGE_TEXTURE_IDLE_FRAMES,
            self.texture_bytes(),
            entries,
        )
    }

    pub fn instance_buffer_bytes(&self) -> u64 {
        (self.instance_policy.capacity() * size_of::<ImageInstance>()) as u64
    }

    /// Creates the image pipeline through the selected backend.
    ///
    /// The pipeline uses the backend's GPU operations while keeping image
    /// handling independent of the concrete graphics API.
    pub fn new(backend: &B, format: B::TextureFormat, antialiasing: crate::AntiAlias) -> Self {
        use crate::backend::*;

        let shader = backend.create_shader_module(
            backend.builtin_shader_source(BuiltinShader::Image),
            "image shader",
        );

        let viewport_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("image viewport uniform".to_string()),
            size: 16,
            usage: vec![BufferUsage::Uniform, BufferUsage::CopyDst],
        });

        let viewport_layout = backend.create_bind_group_layout(&[BindGroupLayoutEntry {
            binding: 0,
            visibility: vec![ShaderStage::Vertex, ShaderStage::Fragment],
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }]);

        let viewport_bind_group = backend.create_bind_group(
            &viewport_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(&viewport_buffer),
            }],
        );

        let texture_bind_group_layout = backend.create_bind_group_layout(&[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: vec![ShaderStage::Fragment],
                ty: BindingType::Texture {
                    multisampled: false,
                    view_dimension: TextureViewDimension::D2,
                    sample_type: TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: vec![ShaderStage::Fragment],
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ]);

        let pipeline_layout =
            backend.create_pipeline_layout(&[&viewport_layout, &texture_bind_group_layout]);

        let pipeline = Self::create_blended_pipeline(
            backend,
            &shader,
            &pipeline_layout,
            format,
            antialiasing,
            BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            "image pipeline",
        );

        let sampler = backend.create_sampler(&SamplerDescriptor {
            label: Some("image sampler".to_string()),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            lod_min_clamp: 0.0,
            lod_max_clamp: 32.0,
            compare: None,
            max_anisotropy: 4,
        });

        let instance_buffer = backend.create_buffer(&BufferDescriptor {
            label: Some("image instance buffer".to_string()),
            size: (Self::INITIAL_CAPACITY * size_of::<ImageInstance>()) as u64,
            usage: vec![BufferUsage::Vertex, BufferUsage::CopyDst],
        });

        Self {
            pipeline,
            mask_pipeline: None,
            shader,
            pipeline_layout,
            format,
            antialiasing,
            viewport_buffer,
            viewport_bind_group,
            texture_bind_group_layout,
            sampler,
            textures: HashMap::new(),
            next_id: 1,
            instance_buffer,
            instance_policy: InstanceBufferPolicy::new(Self::INITIAL_CAPACITY),
            frame_instance_offset: 0,
            frame_instances: Vec::new(),
            upload: FrameUpload::new(),
            immediate_uploads: false,
            last_viewport: None,
            frame_index: 0,
        }
    }

    fn create_blended_pipeline(
        backend: &B,
        shader: &B::ShaderModule,
        pipeline_layout: &B::PipelineLayout,
        format: B::TextureFormat,
        antialiasing: crate::AntiAlias,
        blend: crate::backend::BlendState,
        label: &str,
    ) -> B::RenderPipeline {
        use crate::backend::*;
        let vertex_buffers = [Some(VertexBufferLayout {
            array_stride: size_of::<ImageInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: &ImageInstance::GENERIC_ATTRIBUTES,
        })];
        backend.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(label.to_string()),
            layout: Some(pipeline_layout),
            vertex: VertexState {
                module: shader,
                entry_point: "vs_main",
                buffers: &vertex_buffers,
            },
            fragment: Some(FragmentState {
                module: shader,
                entry_point: "fs_main",
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(blend),
                    write_mask: ColorWriteMask::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: crate::pipeline::multisample_state(antialiasing),
        })
    }

    /// Starts a frame through the selected backend.
    ///
    /// Equivalent to [`ImagePipeline::begin_frame`] but routes all GPU
    /// operations through the backend trait.
    pub fn begin_frame(
        &mut self,
        backend: &B,
        total_instances: usize,
        width: u32,
        height: u32,
        is_srgb: bool,
    ) {
        self.frame_index = self.frame_index.saturating_add(1);
        self.frame_instance_offset = 0;
        self.frame_instances.clear();
        self.immediate_uploads = false;
        let previous_capacity = self.instance_policy.capacity();
        self.instance_policy.record_usage(total_instances);
        if self.instance_policy.capacity() != previous_capacity {
            self.instance_buffer = backend.create_buffer(&crate::backend::BufferDescriptor {
                label: Some("image instance buffer (resized)".to_string()),
                size: (self.instance_policy.capacity() * size_of::<ImageInstance>()) as u64,
                usage: vec![
                    crate::backend::BufferUsage::Vertex,
                    crate::backend::BufferUsage::CopyDst,
                ],
            });
            self.upload.invalidate();
        }

        #[cfg(target_os = "android")]
        let is_srgb_f32 = 2.0_f32;
        #[cfg(not(target_os = "android"))]
        let is_srgb_f32 = if is_srgb { 1.0_f32 } else { 0.0 };
        if self.last_viewport != Some((width, height, is_srgb)) {
            self.last_viewport = Some((width, height, is_srgb));
            backend.write_buffer(
                &self.viewport_buffer,
                0,
                bytemuck::cast_slice(&[width as f32, height as f32, is_srgb_f32, 0.0]),
            );
        }
    }

    /// Upload RGBA8 image data through the backend and return a TextureId.
    ///
    /// Equivalent to [`ImagePipeline::upload_image`] but routes all GPU
    /// operations through the backend trait.
    pub fn upload_image(&mut self, backend: &B, width: u32, height: u32, data: &[u8]) -> TextureId {
        let id = self.next_id;
        self.next_id += 1;
        self.upload_image_with_id_internal_generic(backend, id, width, height, data, false, None);
        id
    }

    /// Upload RGBA8 image data only if the texture ID does not already exist.
    ///
    /// Equivalent to [`ImagePipeline::upload_if_absent`] but routes all GPU
    /// operations through the backend trait.
    pub fn upload_if_absent(
        &mut self,
        backend: &B,
        id: TextureId,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> bool {
        use std::collections::hash_map::Entry;
        match self.textures.entry(id) {
            Entry::Occupied(_) => false,
            Entry::Vacant(vacant) => {
                let (width, height, data) = constrain_rgba8(
                    width,
                    height,
                    data,
                    backend.limits().max_texture_dimension_2d,
                );
                let texture = backend.create_texture(&crate::backend::TextureDescriptor {
                    label: Some("uploaded image".to_string()),
                    size: (width, height, 1),
                    mip_level_count: image_mip_level_count(),
                    sample_count: 1,
                    dimension: crate::backend::TextureDimension::D2,
                    format: B::rgba8_unorm_format(),
                    usage: vec![
                        crate::backend::TextureUsage::TextureBinding,
                        crate::backend::TextureUsage::CopyDst,
                    ],
                });
                upload_rgba8(backend, &texture, width, height, data.as_ref());

                let view = backend.create_texture_view(&texture, "uploaded image view");
                let bind_group = backend.create_bind_group(
                    &self.texture_bind_group_layout,
                    &[
                        crate::backend::BindGroupEntry {
                            binding: 0,
                            resource: crate::backend::BindingResource::TextureView(&view),
                        },
                        crate::backend::BindGroupEntry {
                            binding: 1,
                            resource: crate::backend::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                );

                vacant.insert(TextureEntry {
                    bind_group,
                    texture,
                    width,
                    height,
                    bytes: width as u64 * height as u64 * 4,
                    last_used_frame: self.frame_index,
                    evictable: true,
                    retained_revision: None,
                });
                true
            }
        }
    }

    /// Upload RGBA8 image data with an explicit TextureId through the backend.
    ///
    /// Equivalent to [`ImagePipeline::upload_image_with_id`] but routes all
    /// GPU operations through the backend trait.
    pub fn upload_image_with_id(
        &mut self,
        backend: &B,
        id: TextureId,
        width: u32,
        height: u32,
        data: &[u8],
    ) {
        self.upload_image_with_id_internal_generic(backend, id, width, height, data, false, None);
    }

    /// Uploads retained RGBA8 pixels only when this texture ID lacks the given
    /// resource revision.
    pub fn upload_retained_image_with_id(
        &mut self,
        backend: &B,
        id: TextureId,
        revision: u64,
        width: u32,
        height: u32,
        data: &[u8],
    ) {
        if let Some(entry) = self.textures.get_mut(&id)
            && entry.retained_revision == Some(revision)
        {
            entry.last_used_frame = self.frame_index;
            entry.evictable = false;
            return;
        }
        self.upload_image_with_id_internal_generic(
            backend,
            id,
            width,
            height,
            data,
            false,
            Some(revision),
        );
    }

    /// Internal upload helper using the selected backend.
    fn upload_image_with_id_internal_generic(
        &mut self,
        backend: &B,
        id: TextureId,
        width: u32,
        height: u32,
        data: &[u8],
        evictable: bool,
        retained_revision: Option<u64>,
    ) {
        let (width, height, data) = constrain_rgba8(
            width,
            height,
            data,
            backend.limits().max_texture_dimension_2d,
        );
        // In-place update if the texture exists and dimensions match.
        if let Some(entry) = self.textures.get_mut(&id) {
            if entry.width == width && entry.height == height {
                if retained_revision.is_some() && entry.retained_revision == retained_revision {
                    entry.last_used_frame = self.frame_index;
                    entry.evictable = evictable;
                    return;
                }
                upload_rgba8(backend, &entry.texture, width, height, data.as_ref());
                entry.last_used_frame = self.frame_index;
                entry.evictable = evictable;
                entry.retained_revision = retained_revision;
                return;
            }
        }

        let texture = backend.create_texture(&crate::backend::TextureDescriptor {
            label: Some("uploaded image".to_string()),
            size: (width, height, 1),
            mip_level_count: image_mip_level_count(),
            sample_count: 1,
            dimension: crate::backend::TextureDimension::D2,
            format: B::rgba8_unorm_format(),
            usage: vec![
                crate::backend::TextureUsage::TextureBinding,
                crate::backend::TextureUsage::CopyDst,
            ],
        });
        upload_rgba8(backend, &texture, width, height, data.as_ref());

        let view = backend.create_texture_view(&texture, "uploaded image view");
        let bind_group = backend.create_bind_group(
            &self.texture_bind_group_layout,
            &[
                crate::backend::BindGroupEntry {
                    binding: 0,
                    resource: crate::backend::BindingResource::TextureView(&view),
                },
                crate::backend::BindGroupEntry {
                    binding: 1,
                    resource: crate::backend::BindingResource::Sampler(&self.sampler),
                },
            ],
        );

        self.textures.insert(
            id,
            TextureEntry {
                bind_group,
                texture,
                width,
                height,
                bytes: width as u64 * height as u64 * 4,
                last_used_frame: self.frame_index,
                evictable,
                retained_revision,
            },
        );
    }

    /// Creates an image bind group for a renderer-owned texture view using the
    /// selected backend.
    ///
    /// Equivalent to [`ImagePipeline::create_external_bind_group`] but routes
    /// bind-group creation through the backend trait.
    pub fn create_external_bind_group(&self, backend: &B, view: &B::TextureView) -> B::BindGroup {
        backend.create_bind_group(
            &self.texture_bind_group_layout,
            &[
                crate::backend::BindGroupEntry {
                    binding: 0,
                    resource: crate::backend::BindingResource::TextureView(view),
                },
                crate::backend::BindGroupEntry {
                    binding: 1,
                    resource: crate::backend::BindingResource::Sampler(&self.sampler),
                },
            ],
        )
    }

    /// Uploads frame instance data through the selected backend.
    pub fn end_frame(&mut self, backend: &B) {
        if self.immediate_uploads {
            self.upload.mark_uploaded(&self.frame_instances);
        } else {
            self.upload
                .upload(backend, &self.instance_buffer, &self.frame_instances);
        }
    }

    pub fn draw_batch<'a>(
        &mut self,
        backend: &B,
        pass: &mut B::RenderPass<'a>,
        texture_id: TextureId,
        instances: &[ImageInstance],
    ) where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        let Some(bind_group) = self.textures.get_mut(&texture_id).map(|entry| {
            entry.last_used_frame = self.frame_index;
            entry.bind_group.clone()
        }) else {
            return;
        };
        self.draw_batch_with_bind_group_generic(backend, pass, &bind_group, instances, false);
    }

    pub(crate) fn draw_external_batch<'a>(
        &mut self,
        backend: &B,
        pass: &mut B::RenderPass<'a>,
        bind_group: &B::BindGroup,
        instances: &[ImageInstance],
    ) where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        self.draw_batch_with_bind_group_generic(backend, pass, bind_group, instances, false);
    }

    /// Draws a renderer-owned texture as a mask: the destination keeps its
    /// pixels only where the texture has alpha, scaled by it.
    pub(crate) fn draw_external_mask_batch<'a>(
        &mut self,
        backend: &B,
        pass: &mut B::RenderPass<'a>,
        bind_group: &B::BindGroup,
        instances: &[ImageInstance],
    ) where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        self.draw_batch_with_bind_group_generic(backend, pass, bind_group, instances, true);
    }

    fn draw_batch_with_bind_group_generic<'a>(
        &mut self,
        backend: &B,
        pass: &mut B::RenderPass<'a>,
        bind_group: &B::BindGroup,
        instances: &[ImageInstance],
        mask: bool,
    ) where
        B::RenderPass<'a>: crate::backend::GpuRenderPass<B>,
    {
        use crate::backend::{BufferDescriptor, BufferUsage, GpuRenderPass};
        if instances.is_empty() {
            return;
        }

        let end = self.frame_instance_offset + instances.len();
        if end > self.instance_policy.capacity() {
            if !self.immediate_uploads && !self.frame_instances.is_empty() {
                backend.write_buffer(
                    &self.instance_buffer,
                    0,
                    bytemuck::cast_slice(&self.frame_instances),
                );
            }
            self.instance_policy.grow_to_fit(end);
            self.instance_buffer = backend.create_buffer(&BufferDescriptor {
                label: Some("image instance buffer (resized)".to_string()),
                size: (self.instance_policy.capacity() * size_of::<ImageInstance>()) as u64,
                usage: vec![BufferUsage::Vertex, BufferUsage::CopyDst],
            });
            self.upload.invalidate();
        }

        let byte_offset = (self.frame_instance_offset * size_of::<ImageInstance>()) as u64;
        self.frame_instances.extend_from_slice(instances);
        self.immediate_uploads |= pass.write_buffer_before_draw(
            &self.instance_buffer,
            byte_offset,
            bytemuck::cast_slice(instances),
        );
        if mask && self.mask_pipeline.is_none() {
            self.mask_pipeline = Some(Self::create_blended_pipeline(
                backend,
                &self.shader,
                &self.pipeline_layout,
                self.format,
                self.antialiasing,
                crate::backend::BlendState::DESTINATION_IN,
                "image mask pipeline",
            ));
        }
        pass.set_pipeline(match (&self.mask_pipeline, mask) {
            (Some(mask_pipeline), true) => mask_pipeline,
            _ => &self.pipeline,
        });
        pass.set_bind_group(0, &self.viewport_bind_group, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.set_vertex_buffer(0, &self.instance_buffer, byte_offset);
        pass.draw(0..6, 0..instances.len() as u32);
        self.frame_instance_offset = end;
    }
}

/// Backend-agnostic RGBA8 texture upload via the GpuBackend trait.
fn upload_rgba8<B: crate::backend::GpuBackend>(
    backend: &B,
    texture: &B::Texture,
    width: u32,
    height: u32,
    data: &[u8],
) {
    backend.write_texture(&crate::backend::WriteTextureDescriptor {
        texture,
        mip_level: 0,
        origin: crate::backend::Origin3d { x: 0, y: 0, z: 0 },
        aspect: crate::backend::TextureAspect::All,
        data,
        buffer_layout: crate::backend::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        extent: crate::backend::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    });
}

/// Helper: convert a wgpu vertex format to the backend-agnostic equivalent
/// for formats used by ImageInstance.
#[cfg(all(test, feature = "wgpu"))]
mod tests {
    use std::hint::black_box;
    use std::time::Instant;

    use super::*;

    #[test]
    fn ui_images_allocate_only_the_base_mip_level() {
        assert_eq!(image_mip_level_count(), 1);
    }

    #[test]
    fn sustained_low_usage_reclaims_peak_instance_capacity() {
        let mut policy = InstanceBufferPolicy::new(64);
        policy.record_usage(4096);
        assert_eq!(policy.capacity(), 4096);

        for _ in 0..InstanceBufferPolicy::SHRINK_AFTER_FRAMES {
            policy.record_usage(32);
        }

        assert_eq!(policy.capacity(), 64);
    }

    #[test]
    fn image_instance_carries_draw_opacity() {
        let instance = ImageInstance {
            position: [0.0; 2],
            size: [1.0; 2],
            uv_offset: [0.0; 2],
            uv_scale: [1.0; 2],
            clip_rect: [-1.0; 4],
            clip_border_radius: [0.0; 4],
            alpha: 0.35,
            source_premultiplied: 0.0,
        };

        assert_eq!(instance.alpha, 0.35);
    }

    #[test]
    fn texture_eviction_keeps_current_and_explicit_textures() {
        let evictions = select_texture_evictions(
            10,
            100,
            3,
            200,
            vec![
                TextureCacheEntryInfo {
                    id: 1,
                    bytes: 80,
                    last_used_frame: 10,
                    evictable: true,
                },
                TextureCacheEntryInfo {
                    id: 2,
                    bytes: 40,
                    last_used_frame: 8,
                    evictable: true,
                },
                TextureCacheEntryInfo {
                    id: 3,
                    bytes: 30,
                    last_used_frame: 7,
                    evictable: true,
                },
                TextureCacheEntryInfo {
                    id: 4,
                    bytes: 50,
                    last_used_frame: 6,
                    evictable: false,
                },
            ],
        );

        assert_eq!(evictions, vec![3, 2]);
    }

    #[test]
    fn texture_eviction_reclaims_idle_entries_even_under_budget() {
        let evictions = select_texture_evictions(
            20,
            1_000,
            5,
            120,
            vec![
                TextureCacheEntryInfo {
                    id: 1,
                    bytes: 40,
                    last_used_frame: 15,
                    evictable: true,
                },
                TextureCacheEntryInfo {
                    id: 2,
                    bytes: 40,
                    last_used_frame: 14,
                    evictable: true,
                },
                TextureCacheEntryInfo {
                    id: 3,
                    bytes: 40,
                    last_used_frame: 19,
                    evictable: true,
                },
            ],
        );

        assert_eq!(evictions, vec![2, 1]);
    }

    #[test]
    fn texture_size_preserves_aspect_ratio_when_width_exceeds_limit() {
        assert_eq!(constrained_texture_size(2170, 1085, 2048), (2048, 1024));
    }

    #[test]
    fn texture_size_preserves_aspect_ratio_when_height_exceeds_limit() {
        assert_eq!(constrained_texture_size(1085, 2170, 2048), (1024, 2048));
    }

    #[test]
    fn texture_size_keeps_dimensions_within_limit() {
        assert_eq!(constrained_texture_size(2048, 1024, 2048), (2048, 1024));
    }

    #[test]
    fn oversized_rgba8_data_is_resized_to_the_constrained_dimensions() {
        let data = vec![255; 4 * 10 * 5];
        let (width, height, resized) = constrain_rgba8(10, 5, &data, 4);

        assert_eq!((width, height), (4, 2));
        assert_eq!(resized.len(), 4 * 4 * 2);
    }

    #[test]
    fn integer_nearest_resize_maps_destination_pixels_to_source_pixels() {
        let data = vec![1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255];

        assert_eq!(
            resize_rgba8_nearest(4, 1, &data, 2, 1),
            vec![1, 0, 0, 255, 3, 0, 0, 255],
        );
        assert_eq!(
            resize_rgba8_nearest(1, 4, &data, 1, 2),
            vec![1, 0, 0, 255, 3, 0, 0, 255],
        );
    }

    #[test]
    fn malformed_rgba8_data_uses_a_transparent_placeholder() {
        let (width, height, data) = constrain_rgba8(10, 5, &[255; 4], 4);

        assert_eq!((width, height), (1, 1));
        assert_eq!(data.as_ref(), &[0; 4]);
    }

    #[test]
    fn unaligned_rgba8_data_can_use_the_borrowed_path() {
        let mut storage = vec![0; 17];
        storage[1..].fill(255);

        let (width, height, data) = constrain_rgba8(2, 2, &storage[1..], 4);

        assert_eq!((width, height), (2, 2));
        assert!(matches!(&data, Cow::Borrowed(_)));
        assert_eq!(data.as_ref(), &[255; 16]);
    }

    fn patterned_rgba8(width: u32, height: u32) -> Vec<u8> {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for index in 0..width as usize * height as usize {
            data.extend_from_slice(&[
                index as u8,
                index.wrapping_mul(3) as u8,
                index.wrapping_mul(7) as u8,
                255,
            ]);
        }
        data
    }

    fn consume_image_result(result: (u32, u32, Cow<'_, [u8]>), checksum: &mut u64) {
        let (width, height, data) = black_box(result);
        let data = black_box(data);
        let first = data.as_ref().first().copied().unwrap_or_default();
        let last = data.as_ref().last().copied().unwrap_or_default();
        *checksum = black_box(
            checksum
                .wrapping_add(width as u64)
                .wrapping_add(height as u64)
                .wrapping_add(data.len() as u64)
                .wrapping_add(first as u64)
                .wrapping_add(last as u64),
        );
    }

    #[test]
    #[ignore = "manual image/bulk-data profile"]
    fn profile_image_bulk_operations() {
        const ROUNDS: usize = 7;

        let cases = [
            ("invalid-placeholder", 0, 0, 256, 4_096, 512),
            ("borrowed-256x256", 256, 256, 1_024, 4_096, 512),
            ("lanczos-256x128-to-128x64", 256, 128, 128, 16, 4),
            ("lanczos-512x256-to-128x64", 512, 256, 128, 8, 2),
            ("lanczos-1024x512-to-256x128", 1_024, 512, 256, 4, 1),
        ];
        let mut checksum = 0u64;

        for (name, width, height, max_dimension, measured, warmup) in cases {
            let data = patterned_rgba8(width, height);
            let mut samples = Vec::with_capacity(ROUNDS);
            for _ in 0..ROUNDS {
                for _ in 0..warmup {
                    consume_image_result(
                        constrain_rgba8(width, height, &data, max_dimension),
                        &mut checksum,
                    );
                }

                let start = Instant::now();
                for _ in 0..measured {
                    consume_image_result(
                        constrain_rgba8(width, height, &data, max_dimension),
                        &mut checksum,
                    );
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / measured as f64);
            }

            samples.sort_by(f64::total_cmp);
            let p50 = samples[ROUNDS / 2];
            let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
            println!("{name}: p50 {p50:.3} us, p95 {p95:.3} us");
        }

        let source = patterned_rgba8(513, 257);
        let mut samples = Vec::with_capacity(ROUNDS);
        for _ in 0..ROUNDS {
            for _ in 0..32 {
                consume_image_result(
                    (
                        129,
                        65,
                        Cow::Owned(resize_rgba8_nearest(513, 257, &source, 129, 65)),
                    ),
                    &mut checksum,
                );
            }

            let start = Instant::now();
            for _ in 0..256 {
                consume_image_result(
                    (
                        129,
                        65,
                        Cow::Owned(resize_rgba8_nearest(513, 257, &source, 129, 65)),
                    ),
                    &mut checksum,
                );
            }
            samples.push(start.elapsed().as_secs_f64() * 1e6 / 256.0);
        }
        samples.sort_by(f64::total_cmp);
        let p50 = samples[ROUNDS / 2];
        let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
        println!("nearest-odd-513x257-to-129x65: p50 {p50:.3} us, p95 {p95:.3} us");

        assert_ne!(checksum, 0);
    }
}
