//! Wgpu backend adapter — delegates every [`GpuBackend`] operation to the
//! `wgpu` crate.
//!
//! This optional backend can be selected explicitly with the `wgpu` feature.
//! Each trait method converts the generic descriptor to its `wgpu` counterpart
//! and calls the corresponding `wgpu::Device` / `wgpu::Queue` / `wgpu::Encoder`
//! method.

use std::ops::Range;

use crate::backend::*;

// ── Backend struct ─────────────────────────────────────────────────────────

/// A [`GpuBackend`] that wraps a wgpu device and queue.
pub struct WgpuBackend {
    pub device: ::wgpu::Device,
    pub queue: ::wgpu::Queue,
    pipeline_cache: Option<::wgpu::PipelineCache>,
}

impl WgpuBackend {
    /// Create a new backend from an already-initialized wgpu device and queue.
    pub fn new(device: ::wgpu::Device, queue: ::wgpu::Queue) -> Self {
        let pipeline_cache = crate::pipeline_cache::create_pipeline_cache(&device);
        Self {
            device,
            queue,
            pipeline_cache,
        }
    }

    /// Returns a reference to the wgpu device.
    pub fn device(&self) -> &::wgpu::Device {
        &self.device
    }

    /// Returns a reference to the wgpu queue.
    pub fn queue(&self) -> &::wgpu::Queue {
        &self.queue
    }
}

// ── GpuRenderPass impl ─────────────────────────────────────────────────────

impl<'a> GpuRenderPass<WgpuBackend> for ::wgpu::RenderPass<'a> {
    fn set_pipeline(&mut self, pipeline: &<WgpuBackend as GpuBackend>::RenderPipeline) {
        self.set_pipeline(pipeline);
    }

    fn set_bind_group(
        &mut self,
        index: u32,
        bind_group: &<WgpuBackend as GpuBackend>::BindGroup,
        dynamic_offsets: &[u32],
    ) {
        self.set_bind_group(index, Some(bind_group), dynamic_offsets);
    }

    fn set_vertex_buffer(
        &mut self,
        slot: u32,
        buffer: &<WgpuBackend as GpuBackend>::Buffer,
        offset: u64,
    ) {
        self.set_vertex_buffer(slot, buffer.slice(offset..));
    }

    fn set_index_buffer(
        &mut self,
        buffer: &<WgpuBackend as GpuBackend>::Buffer,
        index_format: IndexFormat,
        offset: u64,
    ) {
        let wgpu_fmt = match index_format {
            IndexFormat::Uint16 => ::wgpu::IndexFormat::Uint16,
            IndexFormat::Uint32 => ::wgpu::IndexFormat::Uint32,
        };
        self.set_index_buffer(buffer.slice(offset..), wgpu_fmt);
    }

    fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.set_scissor_rect(x, y, width, height);
    }

    fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        self.draw(vertices, instances);
    }

    fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        self.draw_indexed(indices, base_vertex, instances);
    }
}

// ── GpuBackend impl ────────────────────────────────────────────────────────

impl GpuBackend for WgpuBackend {
    type Buffer = ::wgpu::Buffer;
    type Texture = ::wgpu::Texture;
    type TextureView = ::wgpu::TextureView;
    type BindGroupLayout = ::wgpu::BindGroupLayout;
    type BindGroup = ::wgpu::BindGroup;
    type PipelineLayout = ::wgpu::PipelineLayout;
    type RenderPipeline = ::wgpu::RenderPipeline;
    type Sampler = ::wgpu::Sampler;
    type ShaderModule = ::wgpu::ShaderModule;
    type CommandEncoder = ::wgpu::CommandEncoder;
    type TextureFormat = ::wgpu::TextureFormat;

    fn r8_unorm_format() -> Self::TextureFormat {
        ::wgpu::TextureFormat::R8Unorm
    }

    fn rgba8_unorm_format() -> Self::TextureFormat {
        ::wgpu::TextureFormat::Rgba8Unorm
    }

    type RenderPass<'a> = ::wgpu::RenderPass<'a>;

    // ── Resource creation ───────────────────────────────────────────────

    fn create_shader_module(&self, source: &[u8], label: &str) -> Self::ShaderModule {
        self.device
            .create_shader_module(::wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: ::wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                    std::str::from_utf8(source).expect("shader source must be valid UTF-8"),
                )),
            })
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> Self::Buffer {
        self.device.create_buffer(&::wgpu::BufferDescriptor {
            label: desc.label.as_deref(),
            size: desc.size,
            usage: buffer_usages(&desc.usage),
            mapped_at_creation: false,
        })
    }

    fn create_texture(&self, desc: &TextureDescriptor<Self::TextureFormat>) -> Self::Texture {
        self.device.create_texture(&::wgpu::TextureDescriptor {
            label: desc.label.as_deref(),
            size: ::wgpu::Extent3d {
                width: desc.size.0,
                height: desc.size.1,
                depth_or_array_layers: desc.size.2,
            },
            mip_level_count: desc.mip_level_count,
            sample_count: desc.sample_count,
            dimension: match desc.dimension {
                TextureDimension::D1 => ::wgpu::TextureDimension::D1,
                TextureDimension::D2 => ::wgpu::TextureDimension::D2,
                TextureDimension::D3 => ::wgpu::TextureDimension::D3,
            },
            format: desc.format,
            usage: texture_usages(&desc.usage),
            view_formats: &[],
        })
    }

    fn create_texture_view(&self, texture: &Self::Texture, label: &str) -> Self::TextureView {
        texture.create_view(&::wgpu::TextureViewDescriptor {
            label: Some(label),
            ..Default::default()
        })
    }

    fn create_sampler(&self, desc: &SamplerDescriptor) -> Self::Sampler {
        self.device.create_sampler(&::wgpu::SamplerDescriptor {
            label: desc.label.as_deref(),
            address_mode_u: address_mode(desc.address_mode_u),
            address_mode_v: address_mode(desc.address_mode_v),
            address_mode_w: address_mode(desc.address_mode_w),
            mag_filter: filter_mode(desc.mag_filter),
            min_filter: filter_mode(desc.min_filter),
            mipmap_filter: mipmap_filter_mode(desc.mipmap_filter),
            lod_min_clamp: desc.lod_min_clamp,
            lod_max_clamp: desc.lod_max_clamp,
            compare: desc.compare.map(compare_function),
            anisotropy_clamp: if desc.max_anisotropy > 1 {
                desc.max_anisotropy
            } else {
                1
            },
            border_color: None,
        })
    }

    fn create_bind_group_layout(&self, entries: &[BindGroupLayoutEntry]) -> Self::BindGroupLayout {
        let wgpu_entries: Vec<::wgpu::BindGroupLayoutEntry> = entries
            .iter()
            .map(|e| ::wgpu::BindGroupLayoutEntry {
                binding: e.binding,
                visibility: shader_stages(&e.visibility),
                ty: binding_type(&e.ty),
                count: e.count,
            })
            .collect();
        self.device
            .create_bind_group_layout(&::wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &wgpu_entries,
            })
    }

    fn create_bind_group(
        &self,
        layout: &Self::BindGroupLayout,
        entries: &[BindGroupEntry<Self>],
    ) -> Self::BindGroup {
        let wgpu_entries: Vec<::wgpu::BindGroupEntry<'_>> = entries
            .iter()
            .map(|e| ::wgpu::BindGroupEntry {
                binding: e.binding,
                resource: binding_resource(&e.resource),
            })
            .collect();
        self.device.create_bind_group(&::wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &wgpu_entries,
        })
    }

    fn create_pipeline_layout(&self, layouts: &[&Self::BindGroupLayout]) -> Self::PipelineLayout {
        let wgpu_layouts: Vec<Option<&::wgpu::BindGroupLayout>> =
            layouts.iter().map(|l| Some(*l)).collect();
        self.device
            .create_pipeline_layout(&::wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &wgpu_layouts,
                immediate_size: 0,
            })
    }

    fn create_render_pipeline(
        &self,
        desc: &RenderPipelineDescriptor<Self>,
    ) -> Self::RenderPipeline {
        // Pre-compute vertex buffer layouts with separately-allocated attribute
        // storage so the references live long enough for the pipeline creation.
        let attr_storage: Vec<Vec<::wgpu::VertexAttribute>> = desc
            .vertex
            .buffers
            .iter()
            .map(|layout| {
                layout
                    .as_ref()
                    .map(|layout| {
                        layout
                            .attributes
                            .iter()
                            .map(|attr| ::wgpu::VertexAttribute {
                                format: vertex_format(attr.format),
                                offset: attr.offset,
                                shader_location: attr.shader_location,
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();

        let vertex_layouts: Vec<Option<::wgpu::VertexBufferLayout<'_>>> = desc
            .vertex
            .buffers
            .iter()
            .enumerate()
            .map(|(i, layout)| {
                layout.as_ref().map(|layout| ::wgpu::VertexBufferLayout {
                    array_stride: layout.array_stride,
                    step_mode: match layout.step_mode {
                        VertexStepMode::Vertex => ::wgpu::VertexStepMode::Vertex,
                        VertexStepMode::Instance => ::wgpu::VertexStepMode::Instance,
                    },
                    attributes: &attr_storage[i],
                })
            })
            .collect();

        let vertex = ::wgpu::VertexState {
            module: desc.vertex.module,
            entry_point: Some(desc.vertex.entry_point),
            buffers: &vertex_layouts,
            compilation_options: Default::default(),
        };

        // Compute color target states first so the Vec lives long enough for
        // the FragmentState to reference it.
        let color_state_storage: Vec<Option<::wgpu::ColorTargetState>> = desc
            .fragment
            .as_ref()
            .map(|f| color_target_states(f.targets))
            .unwrap_or_default();

        let fragment = desc.fragment.as_ref().map(|f| ::wgpu::FragmentState {
            module: f.module,
            entry_point: Some(f.entry_point),
            targets: &color_state_storage,
            compilation_options: Default::default(),
        });

        self.device
            .create_render_pipeline(&::wgpu::RenderPipelineDescriptor {
                label: desc.label.as_deref(),
                layout: desc.layout,
                vertex,
                fragment,
                primitive: primitive_state(&desc.primitive),
                depth_stencil: desc.depth_stencil.as_ref().map(depth_stencil_state),
                multisample: ::wgpu::MultisampleState {
                    count: desc.multisample.count,
                    mask: desc.multisample.mask,
                    alpha_to_coverage_enabled: desc.multisample.alpha_to_coverage_enabled,
                },
                multiview_mask: None,
                cache: self.pipeline_cache.as_ref(),
            })
    }

    // ── Data upload ─────────────────────────────────────────────────────

    fn write_buffer(&self, buffer: &Self::Buffer, offset: u64, data: &[u8]) {
        self.queue.write_buffer(buffer, offset, data);
    }

    fn write_texture(&self, desc: &WriteTextureDescriptor<Self>) {
        self.queue.write_texture(
            ::wgpu::TexelCopyTextureInfo {
                texture: desc.texture,
                mip_level: desc.mip_level,
                origin: ::wgpu::Origin3d {
                    x: desc.origin.x,
                    y: desc.origin.y,
                    z: desc.origin.z,
                },
                aspect: match desc.aspect {
                    TextureAspect::All => ::wgpu::TextureAspect::All,
                    TextureAspect::DepthOnly => ::wgpu::TextureAspect::DepthOnly,
                    TextureAspect::StencilOnly => ::wgpu::TextureAspect::StencilOnly,
                },
            },
            desc.data,
            ::wgpu::TexelCopyBufferLayout {
                offset: desc.buffer_layout.offset,
                bytes_per_row: desc.buffer_layout.bytes_per_row,
                rows_per_image: desc.buffer_layout.rows_per_image,
            },
            ::wgpu::Extent3d {
                width: desc.extent.width,
                height: desc.extent.height,
                depth_or_array_layers: desc.extent.depth_or_array_layers,
            },
        );
    }

    // ── Command encoding ────────────────────────────────────────────────

    fn create_command_encoder(&self, label: &str) -> Self::CommandEncoder {
        self.device
            .create_command_encoder(&::wgpu::CommandEncoderDescriptor { label: Some(label) })
    }

    fn begin_render_pass<'a>(
        &self,
        encoder: &'a mut Self::CommandEncoder,
        desc: &RenderPassDescriptor<Self>,
    ) -> Self::RenderPass<'a> {
        let color_attachments: Vec<Option<::wgpu::RenderPassColorAttachment<'_>>> = desc
            .color_attachments
            .iter()
            .map(|ca| {
                Some(::wgpu::RenderPassColorAttachment {
                    view: ca.view,
                    resolve_target: ca.resolve_target,
                    ops: ::wgpu::Operations {
                        load: match ca.ops.load {
                            LoadOp::Load => ::wgpu::LoadOp::Load,
                            LoadOp::Clear(rgba) => ::wgpu::LoadOp::Clear(::wgpu::Color {
                                r: rgba[0],
                                g: rgba[1],
                                b: rgba[2],
                                a: rgba[3],
                            }),
                        },
                        store: match ca.ops.store {
                            StoreOp::Store => ::wgpu::StoreOp::Store,
                            StoreOp::Discard => ::wgpu::StoreOp::Discard,
                        },
                    },
                    depth_slice: None,
                })
            })
            .collect();

        let depth_stencil = desc.depth_stencil_attachment.as_ref().map(|ds| {
            ::wgpu::RenderPassDepthStencilAttachment {
                view: ds.view,
                depth_ops: ds.depth_ops.map(|ops| ::wgpu::Operations {
                    load: match ops.load {
                        LoadOp::Load => ::wgpu::LoadOp::Load,
                        LoadOp::Clear(depth) => ::wgpu::LoadOp::Clear(depth),
                    },
                    store: match ops.store {
                        StoreOp::Store => ::wgpu::StoreOp::Store,
                        StoreOp::Discard => ::wgpu::StoreOp::Discard,
                    },
                }),
                stencil_ops: ds.stencil_ops.map(|ops| ::wgpu::Operations {
                    load: match ops.load {
                        LoadOp::Load => ::wgpu::LoadOp::Load,
                        LoadOp::Clear(stencil) => ::wgpu::LoadOp::Clear(stencil),
                    },
                    store: match ops.store {
                        StoreOp::Store => ::wgpu::StoreOp::Store,
                        StoreOp::Discard => ::wgpu::StoreOp::Discard,
                    },
                }),
            }
        });

        encoder.begin_render_pass(&::wgpu::RenderPassDescriptor {
            label: desc.label.as_deref(),
            color_attachments: &color_attachments,
            depth_stencil_attachment: depth_stencil,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    // ── Copy operations ─────────────────────────────────────────────────

    fn copy_buffer_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyBufferInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        encoder.copy_buffer_to_texture(
            ::wgpu::TexelCopyBufferInfo {
                buffer: src.buffer,
                layout: ::wgpu::TexelCopyBufferLayout {
                    offset: src.layout.offset,
                    bytes_per_row: src.layout.bytes_per_row,
                    rows_per_image: src.layout.rows_per_image,
                },
            },
            ::wgpu::TexelCopyTextureInfo {
                texture: dst.texture,
                mip_level: dst.mip_level,
                origin: ::wgpu::Origin3d {
                    x: dst.origin.x,
                    y: dst.origin.y,
                    z: dst.origin.z,
                },
                aspect: match dst.aspect {
                    TextureAspect::All => ::wgpu::TextureAspect::All,
                    TextureAspect::DepthOnly => ::wgpu::TextureAspect::DepthOnly,
                    TextureAspect::StencilOnly => ::wgpu::TextureAspect::StencilOnly,
                },
            },
            ::wgpu::Extent3d {
                width: extent.width,
                height: extent.height,
                depth_or_array_layers: extent.depth_or_array_layers,
            },
        );
    }

    fn copy_texture_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        encoder.copy_texture_to_texture(
            ::wgpu::TexelCopyTextureInfo {
                texture: src.texture,
                mip_level: src.mip_level,
                origin: ::wgpu::Origin3d {
                    x: src.origin.x,
                    y: src.origin.y,
                    z: src.origin.z,
                },
                aspect: match src.aspect {
                    TextureAspect::All => ::wgpu::TextureAspect::All,
                    TextureAspect::DepthOnly => ::wgpu::TextureAspect::DepthOnly,
                    TextureAspect::StencilOnly => ::wgpu::TextureAspect::StencilOnly,
                },
            },
            ::wgpu::TexelCopyTextureInfo {
                texture: dst.texture,
                mip_level: dst.mip_level,
                origin: ::wgpu::Origin3d {
                    x: dst.origin.x,
                    y: dst.origin.y,
                    z: dst.origin.z,
                },
                aspect: match dst.aspect {
                    TextureAspect::All => ::wgpu::TextureAspect::All,
                    TextureAspect::DepthOnly => ::wgpu::TextureAspect::DepthOnly,
                    TextureAspect::StencilOnly => ::wgpu::TextureAspect::StencilOnly,
                },
            },
            ::wgpu::Extent3d {
                width: extent.width,
                height: extent.height,
                depth_or_array_layers: extent.depth_or_array_layers,
            },
        );
    }

    // ── Submission ──────────────────────────────────────────────────────

    fn copy_texture_to_buffer(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyBufferInfo<Self>,
        extent: Extent3d,
    ) {
        encoder.copy_texture_to_buffer(
            ::wgpu::TexelCopyTextureInfo {
                texture: src.texture,
                mip_level: src.mip_level,
                origin: ::wgpu::Origin3d {
                    x: src.origin.x,
                    y: src.origin.y,
                    z: src.origin.z,
                },
                aspect: match src.aspect {
                    TextureAspect::All => ::wgpu::TextureAspect::All,
                    TextureAspect::DepthOnly => ::wgpu::TextureAspect::DepthOnly,
                    TextureAspect::StencilOnly => ::wgpu::TextureAspect::StencilOnly,
                },
            },
            ::wgpu::TexelCopyBufferInfo {
                buffer: dst.buffer,
                layout: ::wgpu::TexelCopyBufferLayout {
                    offset: dst.layout.offset,
                    bytes_per_row: dst.layout.bytes_per_row,
                    rows_per_image: dst.layout.rows_per_image,
                },
            },
            ::wgpu::Extent3d {
                width: extent.width,
                height: extent.height,
                depth_or_array_layers: extent.depth_or_array_layers,
            },
        );
    }

    fn read_buffer(&self, buffer: &Self::Buffer, size: u64) -> Vec<u8> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let slice = buffer.slice(0..size);
        slice.map_async(::wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(::wgpu::PollType::wait_indefinitely())
            .expect("wgpu device to finish the readback map");
        receiver
            .recv()
            .expect("map callback to run during poll")
            .expect("readback buffer to map for reading");
        let bytes = slice
            .get_mapped_range()
            .expect("mapped readback range")
            .to_vec();
        buffer.unmap();
        bytes
    }

    fn submit(&self, encoder: Self::CommandEncoder) {
        self.queue.submit([encoder.finish()]);
    }

    fn save_pipeline_cache(&self) {
        if let Some(cache) = &self.pipeline_cache {
            crate::pipeline_cache::save_pipeline_cache(cache);
        }
    }

    // ── Queries ─────────────────────────────────────────────────────────

    fn limits(&self) -> GpuLimits {
        let l = self.device.limits();
        GpuLimits {
            max_texture_dimension_2d: l.max_texture_dimension_2d,
            min_uniform_buffer_offset_alignment: l.min_uniform_buffer_offset_alignment,
        }
    }

    fn format_is_srgb(format: Self::TextureFormat) -> bool {
        format.is_srgb()
    }
}

// ── Conversion helpers ─────────────────────────────────────────────────────

fn address_mode(mode: AddressMode) -> ::wgpu::AddressMode {
    match mode {
        AddressMode::ClampToEdge => ::wgpu::AddressMode::ClampToEdge,
        AddressMode::Repeat => ::wgpu::AddressMode::Repeat,
        AddressMode::MirrorRepeat => ::wgpu::AddressMode::MirrorRepeat,
    }
}

fn filter_mode(mode: FilterMode) -> ::wgpu::FilterMode {
    match mode {
        FilterMode::Nearest => ::wgpu::FilterMode::Nearest,
        FilterMode::Linear => ::wgpu::FilterMode::Linear,
    }
}

fn mipmap_filter_mode(mode: FilterMode) -> ::wgpu::MipmapFilterMode {
    match mode {
        FilterMode::Nearest => ::wgpu::MipmapFilterMode::Nearest,
        FilterMode::Linear => ::wgpu::MipmapFilterMode::Linear,
    }
}

fn compare_function(cf: CompareFunction) -> ::wgpu::CompareFunction {
    match cf {
        CompareFunction::Never => ::wgpu::CompareFunction::Never,
        CompareFunction::Less => ::wgpu::CompareFunction::Less,
        CompareFunction::Equal => ::wgpu::CompareFunction::Equal,
        CompareFunction::LessEqual => ::wgpu::CompareFunction::LessEqual,
        CompareFunction::Greater => ::wgpu::CompareFunction::Greater,
        CompareFunction::NotEqual => ::wgpu::CompareFunction::NotEqual,
        CompareFunction::GreaterEqual => ::wgpu::CompareFunction::GreaterEqual,
        CompareFunction::Always => ::wgpu::CompareFunction::Always,
    }
}

fn binding_type(ty: &BindingType) -> ::wgpu::BindingType {
    match ty {
        BindingType::Buffer {
            ty,
            has_dynamic_offset,
            min_binding_size,
        } => ::wgpu::BindingType::Buffer {
            ty: buffer_binding_type(*ty),
            has_dynamic_offset: *has_dynamic_offset,
            min_binding_size: min_binding_size.and_then(|s| std::num::NonZeroU64::new(s)),
        },
        BindingType::Texture {
            multisampled,
            view_dimension,
            sample_type,
        } => ::wgpu::BindingType::Texture {
            multisampled: *multisampled,
            view_dimension: texture_view_dimension(*view_dimension),
            sample_type: texture_sample_type(*sample_type),
        },
        BindingType::Sampler(binding_type) => {
            ::wgpu::BindingType::Sampler(sampler_binding_type(*binding_type))
        }
        BindingType::StorageTexture { .. } => {
            // Storage textures are not used by current pipelines.
            unimplemented!("StorageTexture binding type is not yet supported by WgpuBackend")
        }
    }
}

fn buffer_binding_type(ty: BufferBindingType) -> ::wgpu::BufferBindingType {
    match ty {
        BufferBindingType::Uniform => ::wgpu::BufferBindingType::Uniform,
        BufferBindingType::Storage => ::wgpu::BufferBindingType::Storage { read_only: false },
        BufferBindingType::ReadOnlyStorage => {
            ::wgpu::BufferBindingType::Storage { read_only: true }
        }
    }
}

fn texture_view_dimension(dim: TextureViewDimension) -> ::wgpu::TextureViewDimension {
    match dim {
        TextureViewDimension::D1 => ::wgpu::TextureViewDimension::D1,
        TextureViewDimension::D2 => ::wgpu::TextureViewDimension::D2,
        TextureViewDimension::D2Array => ::wgpu::TextureViewDimension::D2Array,
        TextureViewDimension::Cube => ::wgpu::TextureViewDimension::Cube,
        TextureViewDimension::CubeArray => ::wgpu::TextureViewDimension::CubeArray,
        TextureViewDimension::D3 => ::wgpu::TextureViewDimension::D3,
    }
}

fn texture_sample_type(sample: TextureSampleType) -> ::wgpu::TextureSampleType {
    match sample {
        TextureSampleType::Float { filterable } => ::wgpu::TextureSampleType::Float { filterable },
        TextureSampleType::Depth => ::wgpu::TextureSampleType::Depth,
        TextureSampleType::Uint => ::wgpu::TextureSampleType::Uint,
        TextureSampleType::Sint => ::wgpu::TextureSampleType::Sint,
    }
}

fn sampler_binding_type(ty: SamplerBindingType) -> ::wgpu::SamplerBindingType {
    match ty {
        SamplerBindingType::Filtering => ::wgpu::SamplerBindingType::Filtering,
        SamplerBindingType::NonFiltering => ::wgpu::SamplerBindingType::NonFiltering,
        SamplerBindingType::Comparison => ::wgpu::SamplerBindingType::Comparison,
    }
}

fn buffer_usages(usages: &[BufferUsage]) -> ::wgpu::BufferUsages {
    usages
        .iter()
        .fold(::wgpu::BufferUsages::empty(), |bits, u| {
            bits | match u {
                BufferUsage::Vertex => ::wgpu::BufferUsages::VERTEX,
                BufferUsage::Index => ::wgpu::BufferUsages::INDEX,
                BufferUsage::Uniform => ::wgpu::BufferUsages::UNIFORM,
                BufferUsage::Storage => ::wgpu::BufferUsages::STORAGE,
                BufferUsage::ReadOnlyStorage => ::wgpu::BufferUsages::STORAGE,
                BufferUsage::Indirect => ::wgpu::BufferUsages::INDIRECT,
                BufferUsage::CopySrc => ::wgpu::BufferUsages::COPY_SRC,
                BufferUsage::CopyDst => ::wgpu::BufferUsages::COPY_DST,
                BufferUsage::MapRead => ::wgpu::BufferUsages::MAP_READ,
                BufferUsage::MapWrite => ::wgpu::BufferUsages::MAP_WRITE,
            }
        })
}

fn texture_usages(usages: &[TextureUsage]) -> ::wgpu::TextureUsages {
    usages
        .iter()
        .fold(::wgpu::TextureUsages::empty(), |bits, u| {
            bits | match u {
                TextureUsage::TextureBinding => ::wgpu::TextureUsages::TEXTURE_BINDING,
                TextureUsage::StorageBinding => ::wgpu::TextureUsages::STORAGE_BINDING,
                TextureUsage::RenderAttachment => ::wgpu::TextureUsages::RENDER_ATTACHMENT,
                TextureUsage::CopySrc => ::wgpu::TextureUsages::COPY_SRC,
                TextureUsage::CopyDst => ::wgpu::TextureUsages::COPY_DST,
            }
        })
}

fn shader_stages(stages: &[ShaderStage]) -> ::wgpu::ShaderStages {
    stages
        .iter()
        .fold(::wgpu::ShaderStages::empty(), |bits, s| {
            bits | match s {
                ShaderStage::Vertex => ::wgpu::ShaderStages::VERTEX,
                ShaderStage::Fragment => ::wgpu::ShaderStages::FRAGMENT,
                ShaderStage::Compute => ::wgpu::ShaderStages::COMPUTE,
            }
        })
}

fn color_write_mask(mask: ColorWriteMask) -> ::wgpu::ColorWrites {
    let mut bits = ::wgpu::ColorWrites::empty();
    if mask.red {
        bits |= ::wgpu::ColorWrites::RED;
    }
    if mask.green {
        bits |= ::wgpu::ColorWrites::GREEN;
    }
    if mask.blue {
        bits |= ::wgpu::ColorWrites::BLUE;
    }
    if mask.alpha {
        bits |= ::wgpu::ColorWrites::ALPHA;
    }
    bits
}

fn binding_resource<'a>(
    resource: &'a BindingResource<'a, WgpuBackend>,
) -> ::wgpu::BindingResource<'a> {
    match resource {
        BindingResource::Buffer(buffer) => buffer.as_entire_binding(),
        BindingResource::BufferRange(buffer, offset, size) => {
            ::wgpu::BindingResource::Buffer(::wgpu::BufferBinding {
                buffer: *buffer,
                offset: *offset,
                size: std::num::NonZeroU64::new(*size),
            })
        }
        BindingResource::TextureView(view) => ::wgpu::BindingResource::TextureView(*view),
        BindingResource::Sampler(sampler) => ::wgpu::BindingResource::Sampler(*sampler),
    }
}

fn vertex_format(format: VertexFormat) -> ::wgpu::VertexFormat {
    match format {
        VertexFormat::Uint8x2 => ::wgpu::VertexFormat::Uint8x2,
        VertexFormat::Uint8x4 => ::wgpu::VertexFormat::Uint8x4,
        VertexFormat::Sint8x2 => ::wgpu::VertexFormat::Sint8x2,
        VertexFormat::Sint8x4 => ::wgpu::VertexFormat::Sint8x4,
        VertexFormat::Unorm8x2 => ::wgpu::VertexFormat::Unorm8x2,
        VertexFormat::Unorm8x4 => ::wgpu::VertexFormat::Unorm8x4,
        VertexFormat::Snorm8x2 => ::wgpu::VertexFormat::Snorm8x2,
        VertexFormat::Snorm8x4 => ::wgpu::VertexFormat::Snorm8x4,
        VertexFormat::Uint16x2 => ::wgpu::VertexFormat::Uint16x2,
        VertexFormat::Uint16x4 => ::wgpu::VertexFormat::Uint16x4,
        VertexFormat::Sint16x2 => ::wgpu::VertexFormat::Sint16x2,
        VertexFormat::Sint16x4 => ::wgpu::VertexFormat::Sint16x4,
        VertexFormat::Unorm16x2 => ::wgpu::VertexFormat::Unorm16x2,
        VertexFormat::Unorm16x4 => ::wgpu::VertexFormat::Unorm16x4,
        VertexFormat::Snorm16x2 => ::wgpu::VertexFormat::Snorm16x2,
        VertexFormat::Snorm16x4 => ::wgpu::VertexFormat::Snorm16x4,
        VertexFormat::Float16x2 => ::wgpu::VertexFormat::Float16x2,
        VertexFormat::Float16x4 => ::wgpu::VertexFormat::Float16x4,
        VertexFormat::Float32 => ::wgpu::VertexFormat::Float32,
        VertexFormat::Float32x2 => ::wgpu::VertexFormat::Float32x2,
        VertexFormat::Float32x3 => ::wgpu::VertexFormat::Float32x3,
        VertexFormat::Float32x4 => ::wgpu::VertexFormat::Float32x4,
        VertexFormat::Uint32 => ::wgpu::VertexFormat::Uint32,
        VertexFormat::Uint32x2 => ::wgpu::VertexFormat::Uint32x2,
        VertexFormat::Uint32x3 => ::wgpu::VertexFormat::Uint32x3,
        VertexFormat::Uint32x4 => ::wgpu::VertexFormat::Uint32x4,
        VertexFormat::Sint32 => ::wgpu::VertexFormat::Sint32,
        VertexFormat::Sint32x2 => ::wgpu::VertexFormat::Sint32x2,
        VertexFormat::Sint32x3 => ::wgpu::VertexFormat::Sint32x3,
        VertexFormat::Sint32x4 => ::wgpu::VertexFormat::Sint32x4,
    }
}

fn color_target_states(
    targets: &[Option<ColorTargetState<::wgpu::TextureFormat>>],
) -> Vec<Option<::wgpu::ColorTargetState>> {
    targets
        .iter()
        .map(|target| {
            target.as_ref().map(|t| ::wgpu::ColorTargetState {
                format: t.format,
                blend: t.blend.as_ref().map(|b| ::wgpu::BlendState {
                    color: ::wgpu::BlendComponent {
                        src_factor: blend_factor(b.color.src_factor),
                        dst_factor: blend_factor(b.color.dst_factor),
                        operation: blend_operation(b.color.operation),
                    },
                    alpha: ::wgpu::BlendComponent {
                        src_factor: blend_factor(b.alpha.src_factor),
                        dst_factor: blend_factor(b.alpha.dst_factor),
                        operation: blend_operation(b.alpha.operation),
                    },
                }),
                write_mask: color_write_mask(t.write_mask),
            })
        })
        .collect()
}

fn blend_factor(factor: BlendFactor) -> ::wgpu::BlendFactor {
    match factor {
        BlendFactor::Zero => ::wgpu::BlendFactor::Zero,
        BlendFactor::One => ::wgpu::BlendFactor::One,
        BlendFactor::Src => ::wgpu::BlendFactor::Src,
        BlendFactor::OneMinusSrc => ::wgpu::BlendFactor::OneMinusSrc,
        BlendFactor::SrcAlpha => ::wgpu::BlendFactor::SrcAlpha,
        BlendFactor::OneMinusSrcAlpha => ::wgpu::BlendFactor::OneMinusSrcAlpha,
        BlendFactor::Dst => ::wgpu::BlendFactor::Dst,
        BlendFactor::OneMinusDst => ::wgpu::BlendFactor::OneMinusDst,
        BlendFactor::DstAlpha => ::wgpu::BlendFactor::DstAlpha,
        BlendFactor::OneMinusDstAlpha => ::wgpu::BlendFactor::OneMinusDstAlpha,
        BlendFactor::SrcAlphaSaturated => ::wgpu::BlendFactor::SrcAlphaSaturated,
        BlendFactor::Constant => ::wgpu::BlendFactor::Constant,
        BlendFactor::OneMinusConstant => ::wgpu::BlendFactor::OneMinusConstant,
    }
}

fn blend_operation(op: BlendOperation) -> ::wgpu::BlendOperation {
    match op {
        BlendOperation::Add => ::wgpu::BlendOperation::Add,
        BlendOperation::Subtract => ::wgpu::BlendOperation::Subtract,
        BlendOperation::ReverseSubtract => ::wgpu::BlendOperation::ReverseSubtract,
        BlendOperation::Min => ::wgpu::BlendOperation::Min,
        BlendOperation::Max => ::wgpu::BlendOperation::Max,
    }
}

fn primitive_state(state: &PrimitiveState) -> ::wgpu::PrimitiveState {
    ::wgpu::PrimitiveState {
        topology: match state.topology {
            PrimitiveTopology::PointList => ::wgpu::PrimitiveTopology::PointList,
            PrimitiveTopology::LineList => ::wgpu::PrimitiveTopology::LineList,
            PrimitiveTopology::LineStrip => ::wgpu::PrimitiveTopology::LineStrip,
            PrimitiveTopology::TriangleList => ::wgpu::PrimitiveTopology::TriangleList,
            PrimitiveTopology::TriangleStrip => ::wgpu::PrimitiveTopology::TriangleStrip,
        },
        strip_index_format: match state.strip_index_format {
            Some(IndexFormat::Uint16) => Some(::wgpu::IndexFormat::Uint16),
            Some(IndexFormat::Uint32) => Some(::wgpu::IndexFormat::Uint32),
            None => None,
        },
        front_face: match state.front_face {
            FrontFace::Ccw => ::wgpu::FrontFace::Ccw,
            FrontFace::Cw => ::wgpu::FrontFace::Cw,
        },
        cull_mode: match state.cull_mode {
            Some(Face::Front) => Some(::wgpu::Face::Front),
            Some(Face::Back) => Some(::wgpu::Face::Back),
            None => None,
        },
        unclipped_depth: state.unclipped_depth,
        polygon_mode: match state.polygon_mode {
            PolygonMode::Fill => ::wgpu::PolygonMode::Fill,
            PolygonMode::Line => ::wgpu::PolygonMode::Line,
            PolygonMode::Point => ::wgpu::PolygonMode::Point,
        },
        conservative: state.conservative,
    }
}

fn depth_stencil_state(
    state: &DepthStencilState<::wgpu::TextureFormat>,
) -> ::wgpu::DepthStencilState {
    ::wgpu::DepthStencilState {
        format: state.format,
        depth_write_enabled: Some(state.depth_write_enabled),
        depth_compare: Some(compare_function(state.depth_compare)),
        stencil: ::wgpu::StencilState {
            front: ::wgpu::StencilFaceState {
                compare: compare_function(state.stencil.front.compare),
                fail_op: stencil_op(state.stencil.front.fail_op),
                depth_fail_op: stencil_op(state.stencil.front.depth_fail_op),
                pass_op: stencil_op(state.stencil.front.pass_op),
            },
            back: ::wgpu::StencilFaceState {
                compare: compare_function(state.stencil.back.compare),
                fail_op: stencil_op(state.stencil.back.fail_op),
                depth_fail_op: stencil_op(state.stencil.back.depth_fail_op),
                pass_op: stencil_op(state.stencil.back.pass_op),
            },
            read_mask: state.stencil.read_mask,
            write_mask: state.stencil.write_mask,
        },
        bias: ::wgpu::DepthBiasState {
            constant: state.bias.constant,
            slope_scale: state.bias.slope_scale,
            clamp: state.bias.clamp,
        },
    }
}

fn stencil_op(op: StencilOperation) -> ::wgpu::StencilOperation {
    match op {
        StencilOperation::Keep => ::wgpu::StencilOperation::Keep,
        StencilOperation::Zero => ::wgpu::StencilOperation::Zero,
        StencilOperation::Replace => ::wgpu::StencilOperation::Replace,
        StencilOperation::IncrementClamp => ::wgpu::StencilOperation::IncrementClamp,
        StencilOperation::DecrementClamp => ::wgpu::StencilOperation::DecrementClamp,
        StencilOperation::Invert => ::wgpu::StencilOperation::Invert,
        StencilOperation::IncrementWrap => ::wgpu::StencilOperation::IncrementWrap,
        StencilOperation::DecrementWrap => ::wgpu::StencilOperation::DecrementWrap,
    }
}
