use js_sys::Array;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    GpuBindGroupDescriptor, GpuBindGroupLayoutDescriptor, GpuBufferDescriptor,
    GpuCommandEncoderDescriptor, GpuExtent3dDict, GpuPipelineLayoutDescriptor,
    GpuRenderPassDescriptor, GpuRenderPipelineDescriptor, GpuSamplerDescriptor,
    GpuShaderModuleDescriptor, GpuTexelCopyBufferInfo, GpuTexelCopyBufferLayout,
    GpuTexelCopyTextureInfo, GpuTextureDescriptor, GpuTextureFormat,
};

use crate::backend::{
    AddressMode, BackendError, BindGroupEntry, BindGroupLayoutEntry, BindingResource, BindingType,
    BlendFactor, BlendOperation, BufferDescriptor, BufferUsage, CompareFunction, DepthStencilState,
    Extent3d, Face, FilterMode, FragmentState, FrontFace, GpuBackend, GpuLimits, IndexFormat,
    LoadOp, MultisampleState, Operations, PolygonMode, PrimitiveState, PrimitiveTopology,
    RenderPassColorAttachment, RenderPassDepthStencilAttachment, RenderPassDescriptor,
    RenderPipelineDescriptor, SamplerBindingType, SamplerDescriptor, ShaderStage, StencilFaceState,
    StencilOperation, StoreOp, TexelCopyBufferInfo, TexelCopyTextureInfo, TextureAspect,
    TextureDescriptor, TextureDimension, TextureSampleType, TextureUsage, TextureViewDimension,
    VertexFormat, VertexState, VertexStepMode, WriteTextureDescriptor,
};

use super::WebGpuBackend;
use super::pipeline::{
    WebGpuBindGroup, WebGpuBindGroupLayout, WebGpuBuffer, WebGpuCommandEncoder,
    WebGpuPipelineLayout, WebGpuRenderPass, WebGpuRenderPipeline, WebGpuSampler,
    WebGpuShaderModule, WebGpuTexture, WebGpuTextureView,
};
use super::util::{
    dictionary, js_error, object, push_value, set, set_array, set_bool, set_f64, set_i32, set_ref,
    set_str, set_u32,
};

const BUFFER_MAP_READ: u32 = 0x0001;
const BUFFER_MAP_WRITE: u32 = 0x0002;
const BUFFER_COPY_SRC: u32 = 0x0004;
const BUFFER_COPY_DST: u32 = 0x0008;
const BUFFER_INDEX: u32 = 0x0010;
const BUFFER_VERTEX: u32 = 0x0020;
const BUFFER_UNIFORM: u32 = 0x0040;
const BUFFER_STORAGE: u32 = 0x0080;
const BUFFER_INDIRECT: u32 = 0x0100;

const TEXTURE_COPY_SRC: u32 = 0x01;
const TEXTURE_COPY_DST: u32 = 0x02;
const TEXTURE_BINDING: u32 = 0x04;
const TEXTURE_STORAGE_BINDING: u32 = 0x08;
const TEXTURE_RENDER_ATTACHMENT: u32 = 0x10;

const SHADER_STAGE_VERTEX: u32 = 0x1;
const SHADER_STAGE_FRAGMENT: u32 = 0x2;
const SHADER_STAGE_COMPUTE: u32 = 0x4;

impl GpuBackend for WebGpuBackend {
    type Buffer = WebGpuBuffer;
    type Texture = WebGpuTexture;
    type TextureView = WebGpuTextureView;
    type BindGroupLayout = WebGpuBindGroupLayout;
    type BindGroup = WebGpuBindGroup;
    type PipelineLayout = WebGpuPipelineLayout;
    type RenderPipeline = WebGpuRenderPipeline;
    type Sampler = WebGpuSampler;
    type ShaderModule = WebGpuShaderModule;
    type CommandEncoder = WebGpuCommandEncoder;
    type TextureFormat = GpuTextureFormat;
    type RenderPass<'a> = WebGpuRenderPass<'a>;

    fn r8_unorm_format() -> Self::TextureFormat {
        GpuTextureFormat::R8unorm
    }

    fn rgba8_unorm_format() -> Self::TextureFormat {
        GpuTextureFormat::Rgba8unorm
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
        let code = std::str::from_utf8(source).map_err(|error| {
            backend_error(
                "create_shader_module",
                format!("{label} is not UTF-8 WGSL: {error}"),
            )
        })?;
        let descriptor: GpuShaderModuleDescriptor = dictionary();
        set_str(descriptor.unchecked_ref(), "code", code);
        set_str(descriptor.unchecked_ref(), "label", label);
        let raw = self.shared.device.create_shader_module(&descriptor);
        Ok(WebGpuShaderModule(raw))
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> Self::Buffer {
        assert!(desc.size > 0, "WebGPU buffers must have a non-zero size");
        assert_eq!(
            desc.size % 4,
            0,
            "WebGPU buffer size must be a multiple of four"
        );
        assert!(
            buffer_usage(&desc.usage) != 0,
            "WebGPU buffers require at least one usage flag"
        );
        let descriptor: GpuBufferDescriptor = dictionary();
        if let Some(label) = &desc.label {
            set_str(descriptor.unchecked_ref(), "label", label);
        }
        set_f64(descriptor.unchecked_ref(), "size", desc.size as f64);
        set_u32(
            descriptor.unchecked_ref(),
            "usage",
            buffer_usage(&desc.usage),
        );
        let raw = self
            .shared
            .device
            .create_buffer(&descriptor)
            .unwrap_or_else(|error| panic!("create WebGPU buffer: {error:?}"));
        WebGpuBuffer {
            raw,
            size: desc.size,
        }
    }

    fn create_texture(&self, desc: &TextureDescriptor<Self::TextureFormat>) -> Self::Texture {
        let descriptor: GpuTextureDescriptor = dictionary();
        if let Some(label) = &desc.label {
            set_str(descriptor.unchecked_ref(), "label", label);
        }
        let size = object();
        set_u32(&size, "width", desc.size.0);
        set_u32(&size, "height", desc.size.1);
        set_u32(&size, "depthOrArrayLayers", desc.size.2);
        set_ref(descriptor.unchecked_ref(), "size", &size);
        set_u32(
            descriptor.unchecked_ref(),
            "mipLevelCount",
            desc.mip_level_count,
        );
        set_u32(descriptor.unchecked_ref(), "sampleCount", desc.sample_count);
        set_str(
            descriptor.unchecked_ref(),
            "dimension",
            texture_dimension(desc.dimension),
        );
        let format: JsValue = desc.format.into();
        set(descriptor.unchecked_ref(), "format", &format);
        set_u32(
            descriptor.unchecked_ref(),
            "usage",
            texture_usage(&desc.usage),
        );
        let raw = self
            .shared
            .device
            .create_texture(&descriptor)
            .unwrap_or_else(|error| panic!("create WebGPU texture: {error:?}"));
        WebGpuTexture::new(raw)
    }

    fn create_texture_view(&self, texture: &Self::Texture, label: &str) -> Self::TextureView {
        let raw = self.shared.create_texture_view(texture);
        if !label.is_empty() {
            raw.set_label(label);
        }
        WebGpuTextureView::texture(texture.clone(), raw)
    }

    fn create_sampler(&self, desc: &SamplerDescriptor) -> Self::Sampler {
        let descriptor: GpuSamplerDescriptor = dictionary();
        if let Some(label) = &desc.label {
            set_str(descriptor.unchecked_ref(), "label", label);
        }
        set_str(
            descriptor.unchecked_ref(),
            "addressModeU",
            address_mode(desc.address_mode_u),
        );
        set_str(
            descriptor.unchecked_ref(),
            "addressModeV",
            address_mode(desc.address_mode_v),
        );
        set_str(
            descriptor.unchecked_ref(),
            "addressModeW",
            address_mode(desc.address_mode_w),
        );
        set_str(
            descriptor.unchecked_ref(),
            "magFilter",
            filter_mode(desc.mag_filter),
        );
        set_str(
            descriptor.unchecked_ref(),
            "minFilter",
            filter_mode(desc.min_filter),
        );
        set_str(
            descriptor.unchecked_ref(),
            "mipmapFilter",
            filter_mode(desc.mipmap_filter),
        );
        set_f64(
            descriptor.unchecked_ref(),
            "lodMinClamp",
            f64::from(desc.lod_min_clamp),
        );
        set_f64(
            descriptor.unchecked_ref(),
            "lodMaxClamp",
            f64::from(desc.lod_max_clamp),
        );
        if let Some(compare) = desc.compare {
            set_str(
                descriptor.unchecked_ref(),
                "compare",
                compare_function(compare),
            );
        }
        set_u32(
            descriptor.unchecked_ref(),
            "maxAnisotropy",
            u32::from(desc.max_anisotropy.max(1)),
        );
        WebGpuSampler(
            self.shared
                .device
                .create_sampler_with_descriptor(&descriptor),
        )
    }

    fn create_bind_group_layout(&self, entries: &[BindGroupLayoutEntry]) -> Self::BindGroupLayout {
        let descriptor: GpuBindGroupLayoutDescriptor = dictionary();
        let raw_entries = Array::new();
        for entry in entries {
            let raw_entry = object();
            set_u32(&raw_entry, "binding", entry.binding);
            set_u32(&raw_entry, "visibility", shader_stages(&entry.visibility));
            set_bind_group_layout_type(&raw_entry, &entry.ty);
            if let Some(count) = entry.count {
                set_u32(&raw_entry, "count", count.get());
            }
            push_value(&raw_entries, raw_entry.as_ref());
        }
        set_array(descriptor.unchecked_ref(), "entries", &raw_entries);
        let raw = self
            .shared
            .device
            .create_bind_group_layout(&descriptor)
            .unwrap_or_else(|error| panic!("create WebGPU bind group layout: {error:?}"));
        WebGpuBindGroupLayout(raw)
    }

    fn create_bind_group(
        &self,
        layout: &Self::BindGroupLayout,
        entries: &[BindGroupEntry<Self>],
    ) -> Self::BindGroup {
        let descriptor: GpuBindGroupDescriptor = dictionary();
        set_ref(descriptor.unchecked_ref(), "layout", &layout.0);
        let raw_entries = Array::new();
        for entry in entries {
            let raw_entry = object();
            set_u32(&raw_entry, "binding", entry.binding);
            match &entry.resource {
                BindingResource::Buffer(buffer) => {
                    set_ref(&raw_entry, "resource", &buffer.raw);
                }
                BindingResource::BufferRange(buffer, offset, size) => {
                    let binding = object();
                    set_ref(&binding, "buffer", &buffer.raw);
                    set_f64(&binding, "offset", *offset as f64);
                    set_f64(&binding, "size", *size as f64);
                    set_ref(&raw_entry, "resource", &binding);
                }
                BindingResource::TextureView(view) => {
                    let raw_view = self.view_object(view);
                    set_ref(&raw_entry, "resource", &raw_view);
                }
                BindingResource::Sampler(sampler) => {
                    set_ref(&raw_entry, "resource", &sampler.0);
                }
            }
            push_value(&raw_entries, raw_entry.as_ref());
        }
        set_array(descriptor.unchecked_ref(), "entries", &raw_entries);
        WebGpuBindGroup(self.shared.device.create_bind_group(&descriptor))
    }

    fn create_pipeline_layout(&self, layouts: &[&Self::BindGroupLayout]) -> Self::PipelineLayout {
        let descriptor: GpuPipelineLayoutDescriptor = dictionary();
        let layout_values = Array::new();
        for layout in layouts {
            layout_values.push(layout.0.as_ref());
        }
        set_array(
            descriptor.unchecked_ref(),
            "bindGroupLayouts",
            &layout_values,
        );
        WebGpuPipelineLayout(self.shared.device.create_pipeline_layout(&descriptor))
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
        validate_primitive_state(&desc.primitive)?;
        let descriptor: GpuRenderPipelineDescriptor = dictionary();
        if let Some(label) = &desc.label {
            set_str(descriptor.unchecked_ref(), "label", label);
        }
        match desc.layout {
            Some(layout) => set_ref(descriptor.unchecked_ref(), "layout", &layout.0),
            None => set_str(descriptor.unchecked_ref(), "layout", "auto"),
        }
        let vertex = vertex_state(&desc.vertex);
        set_ref(descriptor.unchecked_ref(), "vertex", &vertex);
        if let Some(fragment) = &desc.fragment {
            let fragment = fragment_state(fragment);
            set_ref(descriptor.unchecked_ref(), "fragment", &fragment);
        }
        let primitive = primitive_state(&desc.primitive);
        set_ref(descriptor.unchecked_ref(), "primitive", &primitive);
        if let Some(depth_stencil) = &desc.depth_stencil {
            let state = depth_stencil_state(depth_stencil);
            set_ref(descriptor.unchecked_ref(), "depthStencil", &state);
        }
        let multisample = multisample_state(desc.multisample);
        set_ref(descriptor.unchecked_ref(), "multisample", &multisample);
        let raw = self
            .shared
            .device
            .create_render_pipeline(&descriptor)
            .map_err(|error| backend_error("create_render_pipeline", js_error(error)))?;
        Ok(WebGpuRenderPipeline(raw))
    }

    fn write_buffer(&self, buffer: &Self::Buffer, offset: u64, data: &[u8]) {
        assert!(
            offset
                .checked_add(data.len() as u64)
                .is_some_and(|end| end <= buffer.size)
        );
        assert_eq!(
            offset % 4,
            0,
            "WebGPU write_buffer offset must be aligned to four bytes"
        );
        assert_eq!(
            data.len() % 4,
            0,
            "WebGPU write_buffer size must be a multiple of four"
        );
        self.shared
            .queue
            .write_buffer_with_f64_and_u8_slice(&buffer.raw, offset as f64, data)
            .unwrap_or_else(|error| panic!("write WebGPU buffer: {error:?}"));
    }

    fn write_texture(&self, desc: &WriteTextureDescriptor<Self>) {
        let destination: GpuTexelCopyTextureInfo = texture_copy_info(
            &desc.texture.0.raw,
            desc.mip_level,
            desc.origin,
            desc.aspect,
        );
        let data_layout: GpuTexelCopyBufferLayout = buffer_copy_layout(desc.buffer_layout);
        let size = extent_dict(desc.extent);
        self.shared
            .queue
            .write_texture_with_u8_slice_and_gpu_extent_3d_dict(
                &destination,
                desc.data,
                &data_layout,
                &size,
            )
            .unwrap_or_else(|error| panic!("write WebGPU texture: {error:?}"));
    }

    fn create_command_encoder(&self, label: &str) -> Self::CommandEncoder {
        let descriptor: GpuCommandEncoderDescriptor = dictionary();
        set_str(descriptor.unchecked_ref(), "label", label);
        let raw = self
            .shared
            .device
            .create_command_encoder_with_descriptor(&descriptor);
        WebGpuCommandEncoder::new(raw)
    }

    fn begin_render_pass<'a>(
        &self,
        encoder: &'a mut Self::CommandEncoder,
        desc: &RenderPassDescriptor<Self>,
    ) -> Self::RenderPass<'a> {
        let descriptor: GpuRenderPassDescriptor = dictionary();
        if let Some(label) = &desc.label {
            set_str(descriptor.unchecked_ref(), "label", label);
        }
        let color_attachments = Array::new();
        for attachment in desc.color_attachments {
            let view = encoder.texture_view(attachment.view);
            let color = color_attachment(attachment, &view, encoder);
            push_value(&color_attachments, color.as_ref());
        }
        set_array(
            descriptor.unchecked_ref(),
            "colorAttachments",
            &color_attachments,
        );
        if let Some(depth_stencil) = &desc.depth_stencil_attachment {
            let attachment = depth_stencil_attachment(depth_stencil, encoder);
            set_ref(
                descriptor.unchecked_ref(),
                "depthStencilAttachment",
                &attachment,
            );
        }
        let raw = encoder
            .raw
            .begin_render_pass(&descriptor)
            .unwrap_or_else(|error| panic!("begin WebGPU render pass: {error:?}"));
        WebGpuRenderPass::new(raw)
    }

    fn copy_buffer_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyBufferInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        let source = buffer_copy_info(src);
        let destination =
            texture_copy_info(&dst.texture.0.raw, dst.mip_level, dst.origin, dst.aspect);
        let size = extent_dict(extent);
        encoder
            .raw
            .copy_buffer_to_texture_with_gpu_extent_3d_dict(&source, &destination, &size)
            .unwrap_or_else(|error| panic!("copy WebGPU buffer to texture: {error:?}"));
    }

    fn copy_texture_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        let source = texture_copy_info(&src.texture.0.raw, src.mip_level, src.origin, src.aspect);
        let destination =
            texture_copy_info(&dst.texture.0.raw, dst.mip_level, dst.origin, dst.aspect);
        let size = extent_dict(extent);
        encoder
            .raw
            .copy_texture_to_texture_with_gpu_extent_3d_dict(&source, &destination, &size)
            .unwrap_or_else(|error| panic!("copy WebGPU texture: {error:?}"));
    }

    fn copy_texture_to_buffer(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyBufferInfo<Self>,
        extent: Extent3d,
    ) {
        let source = texture_copy_info(&src.texture.0.raw, src.mip_level, src.origin, src.aspect);
        let destination = buffer_copy_info(dst);
        let size = extent_dict(extent);
        encoder
            .raw
            .copy_texture_to_buffer_with_gpu_extent_3d_dict(&source, &destination, &size)
            .unwrap_or_else(|error| panic!("copy WebGPU texture to buffer: {error:?}"));
    }

    fn read_buffer(&self, _buffer: &Self::Buffer, _size: u64) -> Vec<u8> {
        panic!("WebGPU buffer readback is asynchronous; use WebGpuBackend::read_buffer_async")
    }

    fn submit(&self, encoder: Self::CommandEncoder) {
        let command_buffer = encoder.raw.finish();
        self.shared.queue.submit(&[command_buffer]);
        // Keeping the acquired canvas texture alive through queue submission lets
        // the browser present the current frame after its render work completes.
        drop(encoder.surface);
    }

    fn limits(&self) -> GpuLimits {
        self.shared.limits
    }

    fn format_is_srgb(format: Self::TextureFormat) -> bool {
        let format: JsValue = format.into();
        format
            .as_string()
            .is_some_and(|value| value.ends_with("-srgb"))
    }
}

impl WebGpuBackend {
    fn view_object(&self, view: &WebGpuTextureView) -> web_sys::GpuTextureView {
        match &view.0.target {
            super::pipeline::WebGpuViewTarget::Texture { raw, .. } => raw.clone(),
            super::pipeline::WebGpuViewTarget::Surface(_) => {
                panic!(
                    "a WebGPU canvas texture view is only available while encoding a render pass"
                )
            }
        }
    }
}

fn backend_error(operation: &'static str, message: impl Into<String>) -> BackendError {
    BackendError {
        operation,
        message: message.into(),
    }
}

fn buffer_usage(usages: &[BufferUsage]) -> u32 {
    usages.iter().fold(0, |flags, usage| {
        flags
            | match usage {
                BufferUsage::Vertex => BUFFER_VERTEX,
                BufferUsage::Index => BUFFER_INDEX,
                BufferUsage::Uniform => BUFFER_UNIFORM,
                BufferUsage::Storage | BufferUsage::ReadOnlyStorage => BUFFER_STORAGE,
                BufferUsage::Indirect => BUFFER_INDIRECT,
                BufferUsage::CopySrc => BUFFER_COPY_SRC,
                BufferUsage::CopyDst => BUFFER_COPY_DST,
                BufferUsage::MapRead => BUFFER_MAP_READ,
                BufferUsage::MapWrite => BUFFER_MAP_WRITE,
            }
    })
}

fn texture_usage(usages: &[TextureUsage]) -> u32 {
    usages.iter().fold(0, |flags, usage| {
        flags
            | match usage {
                TextureUsage::TextureBinding => TEXTURE_BINDING,
                TextureUsage::StorageBinding => TEXTURE_STORAGE_BINDING,
                TextureUsage::RenderAttachment => TEXTURE_RENDER_ATTACHMENT,
                TextureUsage::CopySrc => TEXTURE_COPY_SRC,
                TextureUsage::CopyDst => TEXTURE_COPY_DST,
            }
    })
}

fn shader_stages(stages: &[ShaderStage]) -> u32 {
    stages.iter().fold(0, |flags, stage| {
        flags
            | match stage {
                ShaderStage::Vertex => SHADER_STAGE_VERTEX,
                ShaderStage::Fragment => SHADER_STAGE_FRAGMENT,
                ShaderStage::Compute => SHADER_STAGE_COMPUTE,
            }
    })
}

fn set_bind_group_layout_type(entry: &js_sys::Object, ty: &BindingType) {
    match ty {
        BindingType::Buffer {
            ty,
            has_dynamic_offset,
            min_binding_size,
        } => {
            let buffer = object();
            set_str(&buffer, "type", buffer_binding_type(*ty));
            set_bool(&buffer, "hasDynamicOffset", *has_dynamic_offset);
            if let Some(size) = min_binding_size {
                set_f64(&buffer, "minBindingSize", *size as f64);
            }
            set_ref(entry, "buffer", &buffer);
        }
        BindingType::Texture {
            multisampled,
            view_dimension: dimension,
            sample_type,
        } => {
            let texture = object();
            set_str(&texture, "sampleType", texture_sample_type(*sample_type));
            set_str(&texture, "viewDimension", view_dimension(*dimension));
            set_bool(&texture, "multisampled", *multisampled);
            set_ref(entry, "texture", &texture);
        }
        BindingType::Sampler(ty) => {
            let sampler = object();
            set_str(&sampler, "type", sampler_binding_type(*ty));
            set_ref(entry, "sampler", &sampler);
        }
        BindingType::StorageTexture { .. } => {
            unimplemented!("storage texture layouts are not used by Cupid's browser renderer")
        }
    }
}

fn buffer_binding_type(ty: crate::backend::BufferBindingType) -> &'static str {
    match ty {
        crate::backend::BufferBindingType::Uniform => "uniform",
        crate::backend::BufferBindingType::Storage => "storage",
        crate::backend::BufferBindingType::ReadOnlyStorage => "read-only-storage",
    }
}

fn texture_sample_type(ty: TextureSampleType) -> &'static str {
    match ty {
        TextureSampleType::Float { filterable: true } => "float",
        TextureSampleType::Float { filterable: false } => "unfilterable-float",
        TextureSampleType::Depth => "depth",
        TextureSampleType::Uint => "uint",
        TextureSampleType::Sint => "sint",
    }
}

fn sampler_binding_type(ty: SamplerBindingType) -> &'static str {
    match ty {
        SamplerBindingType::Filtering => "filtering",
        SamplerBindingType::NonFiltering => "non-filtering",
        SamplerBindingType::Comparison => "comparison",
    }
}

fn view_dimension(dimension: TextureViewDimension) -> &'static str {
    match dimension {
        TextureViewDimension::D1 => "1d",
        TextureViewDimension::D2 => "2d",
        TextureViewDimension::D2Array => "2d-array",
        TextureViewDimension::Cube => "cube",
        TextureViewDimension::CubeArray => "cube-array",
        TextureViewDimension::D3 => "3d",
    }
}

fn texture_dimension(dimension: TextureDimension) -> &'static str {
    match dimension {
        TextureDimension::D1 => "1d",
        TextureDimension::D2 => "2d",
        TextureDimension::D3 => "3d",
    }
}

fn address_mode(mode: AddressMode) -> &'static str {
    match mode {
        AddressMode::ClampToEdge => "clamp-to-edge",
        AddressMode::Repeat => "repeat",
        AddressMode::MirrorRepeat => "mirror-repeat",
    }
}

fn filter_mode(mode: FilterMode) -> &'static str {
    match mode {
        FilterMode::Nearest => "nearest",
        FilterMode::Linear => "linear",
    }
}

fn compare_function(function: CompareFunction) -> &'static str {
    match function {
        CompareFunction::Never => "never",
        CompareFunction::Less => "less",
        CompareFunction::Equal => "equal",
        CompareFunction::LessEqual => "less-equal",
        CompareFunction::Greater => "greater",
        CompareFunction::NotEqual => "not-equal",
        CompareFunction::GreaterEqual => "greater-equal",
        CompareFunction::Always => "always",
    }
}

fn stencil_operation(operation: StencilOperation) -> &'static str {
    match operation {
        StencilOperation::Keep => "keep",
        StencilOperation::Zero => "zero",
        StencilOperation::Replace => "replace",
        StencilOperation::IncrementClamp => "increment-clamp",
        StencilOperation::DecrementClamp => "decrement-clamp",
        StencilOperation::Invert => "invert",
        StencilOperation::IncrementWrap => "increment-wrap",
        StencilOperation::DecrementWrap => "decrement-wrap",
    }
}

fn texture_aspect(aspect: TextureAspect) -> &'static str {
    match aspect {
        TextureAspect::All => "all",
        TextureAspect::DepthOnly => "depth-only",
        TextureAspect::StencilOnly => "stencil-only",
    }
}

fn vertex_format(format: VertexFormat) -> &'static str {
    match format {
        VertexFormat::Uint8x2 => "uint8x2",
        VertexFormat::Uint8x4 => "uint8x4",
        VertexFormat::Sint8x2 => "sint8x2",
        VertexFormat::Sint8x4 => "sint8x4",
        VertexFormat::Unorm8x2 => "unorm8x2",
        VertexFormat::Unorm8x4 => "unorm8x4",
        VertexFormat::Snorm8x2 => "snorm8x2",
        VertexFormat::Snorm8x4 => "snorm8x4",
        VertexFormat::Uint16x2 => "uint16x2",
        VertexFormat::Uint16x4 => "uint16x4",
        VertexFormat::Sint16x2 => "sint16x2",
        VertexFormat::Sint16x4 => "sint16x4",
        VertexFormat::Unorm16x2 => "unorm16x2",
        VertexFormat::Unorm16x4 => "unorm16x4",
        VertexFormat::Snorm16x2 => "snorm16x2",
        VertexFormat::Snorm16x4 => "snorm16x4",
        VertexFormat::Float16x2 => "float16x2",
        VertexFormat::Float16x4 => "float16x4",
        VertexFormat::Float32 => "float32",
        VertexFormat::Float32x2 => "float32x2",
        VertexFormat::Float32x3 => "float32x3",
        VertexFormat::Float32x4 => "float32x4",
        VertexFormat::Uint32 => "uint32",
        VertexFormat::Uint32x2 => "uint32x2",
        VertexFormat::Uint32x3 => "uint32x3",
        VertexFormat::Uint32x4 => "uint32x4",
        VertexFormat::Sint32 => "sint32",
        VertexFormat::Sint32x2 => "sint32x2",
        VertexFormat::Sint32x3 => "sint32x3",
        VertexFormat::Sint32x4 => "sint32x4",
    }
}

fn vertex_step_mode(mode: VertexStepMode) -> &'static str {
    match mode {
        VertexStepMode::Vertex => "vertex",
        VertexStepMode::Instance => "instance",
    }
}

fn blend_factor(factor: BlendFactor) -> &'static str {
    match factor {
        BlendFactor::Zero => "zero",
        BlendFactor::One => "one",
        BlendFactor::Src => "src",
        BlendFactor::OneMinusSrc => "one-minus-src",
        BlendFactor::SrcAlpha => "src-alpha",
        BlendFactor::OneMinusSrcAlpha => "one-minus-src-alpha",
        BlendFactor::Dst => "dst",
        BlendFactor::OneMinusDst => "one-minus-dst",
        BlendFactor::DstAlpha => "dst-alpha",
        BlendFactor::OneMinusDstAlpha => "one-minus-dst-alpha",
        BlendFactor::SrcAlphaSaturated => "src-alpha-saturated",
        BlendFactor::Constant => "constant",
        BlendFactor::OneMinusConstant => "one-minus-constant",
    }
}

fn blend_operation(operation: BlendOperation) -> &'static str {
    match operation {
        BlendOperation::Add => "add",
        BlendOperation::Subtract => "subtract",
        BlendOperation::ReverseSubtract => "reverse-subtract",
        BlendOperation::Min => "min",
        BlendOperation::Max => "max",
    }
}

fn primitive_topology(topology: PrimitiveTopology) -> &'static str {
    match topology {
        PrimitiveTopology::PointList => "point-list",
        PrimitiveTopology::LineList => "line-list",
        PrimitiveTopology::LineStrip => "line-strip",
        PrimitiveTopology::TriangleList => "triangle-list",
        PrimitiveTopology::TriangleStrip => "triangle-strip",
    }
}

fn index_format(format: IndexFormat) -> &'static str {
    match format {
        IndexFormat::Uint16 => "uint16",
        IndexFormat::Uint32 => "uint32",
    }
}

fn validate_primitive_state(state: &PrimitiveState) -> Result<(), BackendError> {
    if state.polygon_mode != PolygonMode::Fill {
        return Err(backend_error(
            "create_render_pipeline",
            "WebGPU supports filled polygons only",
        ));
    }
    if state.conservative {
        return Err(backend_error(
            "create_render_pipeline",
            "conservative rasterization is not supported by the browser WebGPU backend",
        ));
    }
    if state.unclipped_depth {
        return Err(backend_error(
            "create_render_pipeline",
            "unclipped depth requires an optional WebGPU device feature",
        ));
    }
    Ok(())
}

fn vertex_state(state: &VertexState<'_, WebGpuBackend>) -> js_sys::Object {
    let vertex = object();
    set_ref(&vertex, "module", &state.module.0);
    set_str(&vertex, "entryPoint", state.entry_point);
    let buffers = Array::new();
    for layout in state.buffers {
        let Some(layout) = layout else {
            push_value(&buffers, &JsValue::NULL);
            continue;
        };
        let raw_layout = object();
        set_f64(&raw_layout, "arrayStride", layout.array_stride as f64);
        set_str(&raw_layout, "stepMode", vertex_step_mode(layout.step_mode));
        let attributes = Array::new();
        for attribute in layout.attributes {
            let raw_attribute = object();
            set_str(&raw_attribute, "format", vertex_format(attribute.format));
            set_f64(&raw_attribute, "offset", attribute.offset as f64);
            set_u32(&raw_attribute, "shaderLocation", attribute.shader_location);
            push_value(&attributes, raw_attribute.as_ref());
        }
        set_array(&raw_layout, "attributes", &attributes);
        push_value(&buffers, raw_layout.as_ref());
    }
    set_array(&vertex, "buffers", &buffers);
    vertex
}

fn fragment_state(state: &FragmentState<'_, WebGpuBackend>) -> js_sys::Object {
    let fragment = object();
    set_ref(&fragment, "module", &state.module.0);
    set_str(&fragment, "entryPoint", state.entry_point);
    let targets = Array::new();
    for target in state.targets {
        if let Some(target) = target {
            let raw_target = object();
            let format: JsValue = target.format.into();
            set(&raw_target, "format", &format);
            if let Some(blend) = &target.blend {
                let raw_blend = object();
                set_ref(&raw_blend, "color", &blend_component(&blend.color));
                set_ref(&raw_blend, "alpha", &blend_component(&blend.alpha));
                set_ref(&raw_target, "blend", &raw_blend);
            }
            let write_mask = u32::from(target.write_mask.red)
                | (u32::from(target.write_mask.green) << 1)
                | (u32::from(target.write_mask.blue) << 2)
                | (u32::from(target.write_mask.alpha) << 3);
            set_u32(&raw_target, "writeMask", write_mask);
            push_value(&targets, raw_target.as_ref());
        } else {
            push_value(&targets, &JsValue::NULL);
        }
    }
    set_array(&fragment, "targets", &targets);
    fragment
}

fn blend_component(component: &crate::backend::BlendComponent) -> js_sys::Object {
    let raw = object();
    set_str(&raw, "srcFactor", blend_factor(component.src_factor));
    set_str(&raw, "dstFactor", blend_factor(component.dst_factor));
    set_str(&raw, "operation", blend_operation(component.operation));
    raw
}

fn primitive_state(state: &PrimitiveState) -> js_sys::Object {
    let raw = object();
    set_str(&raw, "topology", primitive_topology(state.topology));
    if let Some(format) = state.strip_index_format {
        set_str(&raw, "stripIndexFormat", index_format(format));
    }
    set_str(
        &raw,
        "frontFace",
        match state.front_face {
            FrontFace::Ccw => "ccw",
            FrontFace::Cw => "cw",
        },
    );
    if let Some(face) = state.cull_mode {
        set_str(
            &raw,
            "cullMode",
            match face {
                Face::Front => "front",
                Face::Back => "back",
            },
        );
    }
    set_bool(&raw, "unclippedDepth", state.unclipped_depth);
    raw
}

fn depth_stencil_state(state: &DepthStencilState<GpuTextureFormat>) -> js_sys::Object {
    let raw = object();
    let format: JsValue = state.format.into();
    set(&raw, "format", &format);
    set_bool(&raw, "depthWriteEnabled", state.depth_write_enabled);
    set_str(&raw, "depthCompare", compare_function(state.depth_compare));
    set_ref(
        &raw,
        "stencilFront",
        &stencil_face_state(state.stencil.front),
    );
    set_ref(&raw, "stencilBack", &stencil_face_state(state.stencil.back));
    set_u32(&raw, "stencilReadMask", state.stencil.read_mask);
    set_u32(&raw, "stencilWriteMask", state.stencil.write_mask);
    set_i32(&raw, "depthBias", state.bias.constant);
    set_f64(
        &raw,
        "depthBiasSlopeScale",
        f64::from(state.bias.slope_scale),
    );
    set_f64(&raw, "depthBiasClamp", f64::from(state.bias.clamp));
    raw
}

fn stencil_face_state(state: StencilFaceState) -> js_sys::Object {
    let raw = object();
    set_str(&raw, "compare", compare_function(state.compare));
    set_str(&raw, "failOp", stencil_operation(state.fail_op));
    set_str(&raw, "depthFailOp", stencil_operation(state.depth_fail_op));
    set_str(&raw, "passOp", stencil_operation(state.pass_op));
    raw
}

fn multisample_state(state: MultisampleState) -> js_sys::Object {
    let raw = object();
    set_u32(&raw, "count", state.count);
    set_u32(&raw, "mask", state.mask as u32);
    set_bool(
        &raw,
        "alphaToCoverageEnabled",
        state.alpha_to_coverage_enabled,
    );
    raw
}

fn color_attachment(
    attachment: &RenderPassColorAttachment<'_, WebGpuBackend>,
    view: &web_sys::GpuTextureView,
    encoder: &mut WebGpuCommandEncoder,
) -> js_sys::Object {
    let raw = object();
    set_ref(&raw, "view", view);
    if let Some(resolve_target) = attachment.resolve_target {
        let resolve_target = encoder.texture_view(resolve_target);
        set_ref(&raw, "resolveTarget", &resolve_target);
    }
    match attachment.ops.load {
        LoadOp::Load => set_str(&raw, "loadOp", "load"),
        LoadOp::Clear(color) => {
            set_str(&raw, "loadOp", "clear");
            let clear_value = object();
            set_f64(&clear_value, "r", color[0]);
            set_f64(&clear_value, "g", color[1]);
            set_f64(&clear_value, "b", color[2]);
            set_f64(&clear_value, "a", color[3]);
            set_ref(&raw, "clearValue", &clear_value);
        }
    }
    set_str(
        &raw,
        "storeOp",
        match attachment.ops.store {
            StoreOp::Store => "store",
            StoreOp::Discard => "discard",
        },
    );
    raw
}

fn depth_stencil_attachment(
    attachment: &RenderPassDepthStencilAttachment<'_, WebGpuBackend>,
    encoder: &mut WebGpuCommandEncoder,
) -> js_sys::Object {
    let raw = object();
    let view = encoder.texture_view(attachment.view);
    set_ref(&raw, "view", &view);
    if let Some(operations) = attachment.depth_ops {
        set_attachment_operations(&raw, "depth", operations);
        set_bool(&raw, "depthReadOnly", false);
    }
    if let Some(operations) = attachment.stencil_ops {
        set_attachment_operations(&raw, "stencil", operations);
        set_bool(&raw, "stencilReadOnly", false);
    }
    raw
}

fn set_attachment_operations<T: Copy + Into<f64>>(
    descriptor: &js_sys::Object,
    prefix: &str,
    operations: Operations<T>,
) {
    match operations.load {
        LoadOp::Load => set_str(descriptor, &format!("{prefix}LoadOp"), "load"),
        LoadOp::Clear(value) => {
            set_str(descriptor, &format!("{prefix}LoadOp"), "clear");
            set_f64(descriptor, &format!("{prefix}ClearValue"), value.into());
        }
    }
    set_str(
        descriptor,
        &format!("{prefix}StoreOp"),
        match operations.store {
            StoreOp::Store => "store",
            StoreOp::Discard => "discard",
        },
    );
}

fn buffer_copy_layout(layout: crate::backend::TexelCopyBufferLayout) -> GpuTexelCopyBufferLayout {
    let raw: GpuTexelCopyBufferLayout = dictionary();
    set_f64(raw.unchecked_ref(), "offset", layout.offset as f64);
    if let Some(bytes_per_row) = layout.bytes_per_row {
        set_u32(raw.unchecked_ref(), "bytesPerRow", bytes_per_row);
    }
    if let Some(rows_per_image) = layout.rows_per_image {
        set_u32(raw.unchecked_ref(), "rowsPerImage", rows_per_image);
    }
    raw
}

fn buffer_copy_info(info: &TexelCopyBufferInfo<WebGpuBackend>) -> GpuTexelCopyBufferInfo {
    let raw: GpuTexelCopyBufferInfo = dictionary();
    set_ref(raw.unchecked_ref(), "buffer", &info.buffer.raw);
    let layout = buffer_copy_layout(info.layout);
    set_ref(raw.unchecked_ref(), "layout", &layout);
    raw
}

fn texture_copy_info(
    texture: &web_sys::GpuTexture,
    mip_level: u32,
    origin: crate::backend::Origin3d,
    aspect: TextureAspect,
) -> GpuTexelCopyTextureInfo {
    let raw: GpuTexelCopyTextureInfo = dictionary();
    set_ref(raw.unchecked_ref(), "texture", texture);
    set_u32(raw.unchecked_ref(), "mipLevel", mip_level);
    let raw_origin = object();
    set_u32(&raw_origin, "x", origin.x);
    set_u32(&raw_origin, "y", origin.y);
    set_u32(&raw_origin, "z", origin.z);
    set_ref(raw.unchecked_ref(), "origin", &raw_origin);
    set_str(raw.unchecked_ref(), "aspect", texture_aspect(aspect));
    raw
}

fn extent_dict(extent: Extent3d) -> GpuExtent3dDict {
    let raw: GpuExtent3dDict = dictionary();
    set_u32(raw.unchecked_ref(), "width", extent.width);
    set_u32(raw.unchecked_ref(), "height", extent.height);
    set_u32(
        raw.unchecked_ref(),
        "depthOrArrayLayers",
        extent.depth_or_array_layers,
    );
    raw
}
