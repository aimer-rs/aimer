use std::collections::BTreeMap;
use std::ffi::c_void;
use std::ptr::null;
use std::rc::Rc;

use crate::backend::*;

use super::gl::{self, GLsizeiptr};
use super::pipeline::{
    self, OpenGlBindGroupInner, OpenGlBindGroupLayoutInner, OpenGlBoundResource,
    OpenGlCommandEncoder, OpenGlPipelineLayoutInner, OpenGlRenderPass,
};
use super::{
    OpenGlBackend, OpenGlBindGroup, OpenGlBindGroupLayout, OpenGlBuffer, OpenGlBufferInner,
    OpenGlPipelineLayout, OpenGlSampler, OpenGlSamplerInner, OpenGlShaderModule, OpenGlShared,
    OpenGlTexture, OpenGlTextureFormat, OpenGlTextureInner, OpenGlTextureView,
    OpenGlTextureViewInner, OpenGlViewTarget,
};

impl GpuBackend for OpenGlBackend {
    type Buffer = OpenGlBuffer;
    type Texture = OpenGlTexture;
    type TextureView = OpenGlTextureView;
    type BindGroupLayout = OpenGlBindGroupLayout;
    type BindGroup = OpenGlBindGroup;
    type PipelineLayout = OpenGlPipelineLayout;
    type RenderPipeline = pipeline::OpenGlRenderPipeline;
    type Sampler = OpenGlSampler;
    type ShaderModule = OpenGlShaderModule;
    type CommandEncoder = OpenGlCommandEncoder;
    type TextureFormat = OpenGlTextureFormat;
    type RenderPass<'a>
        = OpenGlRenderPass<'a>
    where
        Self: 'a;

    fn r8_unorm_format() -> Self::TextureFormat {
        OpenGlTextureFormat::R8Unorm
    }

    fn rgba8_unorm_format() -> Self::TextureFormat {
        OpenGlTextureFormat::Rgba8Unorm
    }

    fn rect_shader_source(&self) -> &'static [u8] {
        include_bytes!("../../pipeline/shaders/opengl/rect.glslpack")
    }

    fn builtin_shader_source(&self, shader: BuiltinShader) -> &'static [u8] {
        match shader {
            BuiltinShader::Image => include_bytes!("../../pipeline/shaders/opengl/image.glslpack"),
            BuiltinShader::Text => include_bytes!("../../pipeline/shaders/opengl/text.glslpack"),
            BuiltinShader::TextColor => {
                include_bytes!("../../pipeline/shaders/opengl/text_color.glslpack")
            }
            BuiltinShader::TextDecoration => {
                include_bytes!("../../pipeline/shaders/opengl/text_decoration.glslpack")
            }
            BuiltinShader::Svg => include_bytes!("../../pipeline/shaders/opengl/svg.glslpack"),
            BuiltinShader::FrameComposite => {
                include_bytes!("../../pipeline/shaders/opengl/frame_composite.glslpack")
            }
            BuiltinShader::Material => {
                include_bytes!("../../pipeline/shaders/opengl/material.glslpack")
            }
            BuiltinShader::Morph => {
                include_bytes!("../../pipeline/shaders/opengl/morph.glslpack")
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
        OpenGlShaderModule::create(&self.shared, source, label)
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> Self::Buffer {
        let size =
            GLsizeiptr::try_from(desc.size).expect("OpenGL buffer size exceeds addressable range");
        let mut raw = 0;
        let _ = self.shared.context.make_current();
        unsafe {
            (self.shared.gl.gen_buffers)(1, &mut raw);
            (self.shared.gl.bind_buffer)(gl::COPY_WRITE_BUFFER, raw);
            (self.shared.gl.buffer_data)(
                gl::COPY_WRITE_BUFFER,
                size,
                null(),
                buffer_usage(&desc.usage),
            );
            (self.shared.gl.bind_buffer)(gl::COPY_WRITE_BUFFER, 0);
        }
        assert_ne!(raw, 0, "glGenBuffers returned zero");
        OpenGlBuffer(std::rc::Rc::new(OpenGlBufferInner {
            shared: self.shared.clone(),
            raw,
            size: desc.size,
        }))
    }

    fn create_texture(&self, desc: &TextureDescriptor<Self::TextureFormat>) -> Self::Texture {
        assert!(
            desc.size.0 > 0 && desc.size.1 > 0 && desc.size.2 > 0,
            "OpenGL texture extent must be non-zero"
        );
        let target = if desc.sample_count > 1 {
            assert!(
                desc.dimension == TextureDimension::D2 && desc.size.2 == 1,
                "OpenGL multisampled textures must be 2D"
            );
            gl::TEXTURE_2D_MULTISAMPLE
        } else {
            pipeline::texture_target(desc.dimension, desc.size)
        };
        let (internal_format, external_format, external_type, _) =
            pipeline::format_info(desc.format);
        let mut raw = 0;
        let _ = self.shared.context.make_current();
        unsafe {
            (self.shared.gl.gen_textures)(1, &mut raw);
            self.shared.bind_texture(0, target, raw);
            if desc.sample_count > 1 {
                (self.shared.gl.tex_image_2d_multisample)(
                    target,
                    desc.sample_count as i32,
                    internal_format,
                    desc.size.0 as i32,
                    desc.size.1 as i32,
                    gl::TRUE,
                );
            } else {
                allocate_texture_levels(
                    &self.shared,
                    target,
                    internal_format,
                    external_format,
                    external_type,
                    desc,
                );
                (self.shared.gl.tex_parameter_i)(target, gl::TEXTURE_BASE_LEVEL, 0);
                (self.shared.gl.tex_parameter_i)(
                    target,
                    gl::TEXTURE_MAX_LEVEL,
                    desc.mip_level_count.saturating_sub(1) as i32,
                );
            }
            self.shared.bind_texture(0, target, 0);
        }
        assert_ne!(raw, 0, "glGenTextures returned zero");
        OpenGlTexture(std::rc::Rc::new(OpenGlTextureInner {
            shared: self.shared.clone(),
            raw,
            target,
            size: desc.size,
            format: desc.format,
        }))
    }

    fn create_texture_view(&self, texture: &Self::Texture, _label: &str) -> Self::TextureView {
        let layer = if texture.0.size.2 > 1 { Some(0) } else { None };
        OpenGlTextureView(std::rc::Rc::new(OpenGlTextureViewInner {
            target: OpenGlViewTarget::Texture {
                texture: texture.clone(),
                level: 0,
                layer,
            },
        }))
    }

    fn create_sampler(&self, desc: &SamplerDescriptor) -> Self::Sampler {
        let mut raw = 0;
        let _ = self.shared.context.make_current();
        unsafe {
            (self.shared.gl.gen_samplers)(1, &mut raw);
            (self.shared.gl.sampler_parameter_i)(
                raw,
                gl::TEXTURE_WRAP_S,
                pipeline::address_mode(desc.address_mode_u) as i32,
            );
            (self.shared.gl.sampler_parameter_i)(
                raw,
                gl::TEXTURE_WRAP_T,
                pipeline::address_mode(desc.address_mode_v) as i32,
            );
            (self.shared.gl.sampler_parameter_i)(
                raw,
                gl::TEXTURE_WRAP_R,
                pipeline::address_mode(desc.address_mode_w) as i32,
            );
            let mag = match desc.mag_filter {
                FilterMode::Nearest => gl::NEAREST,
                FilterMode::Linear => gl::LINEAR,
            };
            (self.shared.gl.sampler_parameter_i)(raw, gl::TEXTURE_MAG_FILTER, mag as i32);
            (self.shared.gl.sampler_parameter_i)(
                raw,
                gl::TEXTURE_MIN_FILTER,
                pipeline::sampler_filter(desc.min_filter, desc.mipmap_filter) as i32,
            );
            (self.shared.gl.sampler_parameter_f)(raw, gl::TEXTURE_MIN_LOD, desc.lod_min_clamp);
            (self.shared.gl.sampler_parameter_f)(raw, gl::TEXTURE_MAX_LOD, desc.lod_max_clamp);
            if let Some(compare) = desc.compare {
                (self.shared.gl.sampler_parameter_i)(
                    raw,
                    gl::TEXTURE_COMPARE_MODE,
                    gl::COMPARE_REF_TO_TEXTURE as i32,
                );
                (self.shared.gl.sampler_parameter_i)(
                    raw,
                    gl::TEXTURE_COMPARE_FUNC,
                    pipeline::compare_function(compare) as i32,
                );
            }
        }
        assert_ne!(raw, 0, "glGenSamplers returned zero");
        OpenGlSampler(std::rc::Rc::new(OpenGlSamplerInner {
            shared: self.shared.clone(),
            raw,
        }))
    }

    fn create_bind_group_layout(&self, entries: &[BindGroupLayoutEntry]) -> Self::BindGroupLayout {
        assert!(
            entries
                .iter()
                .all(|entry| entry.count.is_none_or(|count| count.get() == 1)),
            "OpenGL 3.3 backend does not support binding arrays"
        );
        OpenGlBindGroupLayout(std::rc::Rc::new(OpenGlBindGroupLayoutInner {
            entries: entries.to_vec(),
        }))
    }

    fn create_bind_group(
        &self,
        layout: &Self::BindGroupLayout,
        entries: &[BindGroupEntry<Self>],
    ) -> Self::BindGroup {
        let mut resources = BTreeMap::new();
        for entry in entries {
            let layout_entry = layout
                .0
                .entries
                .iter()
                .find(|item| item.binding == entry.binding)
                .expect("OpenGL bind-group resource must be present in its layout");
            let compatible = matches!(
                (&layout_entry.ty, &entry.resource),
                (
                    BindingType::Buffer { .. },
                    BindingResource::Buffer(_) | BindingResource::BufferRange(_, _, _)
                ) | (
                    BindingType::Texture { .. } | BindingType::StorageTexture { .. },
                    BindingResource::TextureView(_)
                ) | (BindingType::Sampler(_), BindingResource::Sampler(_))
            );
            assert!(
                compatible,
                "OpenGL bind-group resource does not match its layout"
            );
            let resource = match &entry.resource {
                BindingResource::Buffer(buffer) => OpenGlBoundResource::Buffer((*buffer).clone()),
                BindingResource::BufferRange(buffer, offset, size) => {
                    OpenGlBoundResource::BufferRange((*buffer).clone(), *offset, *size)
                }
                BindingResource::TextureView(view) => {
                    OpenGlBoundResource::TextureView((*view).clone())
                }
                BindingResource::Sampler(sampler) => {
                    OpenGlBoundResource::Sampler((*sampler).clone())
                }
            };
            resources.insert(entry.binding, resource);
        }
        OpenGlBindGroup(std::rc::Rc::new(OpenGlBindGroupInner {
            layout: layout.clone(),
            resources,
        }))
    }

    fn create_pipeline_layout(&self, layouts: &[&Self::BindGroupLayout]) -> Self::PipelineLayout {
        OpenGlPipelineLayout(std::rc::Rc::new(OpenGlPipelineLayoutInner {
            groups: layouts.iter().map(|layout| (*layout).clone()).collect(),
        }))
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
        pipeline::create_render_pipeline(&self.shared, desc)
    }

    fn write_buffer(&self, buffer: &Self::Buffer, offset: u64, data: &[u8]) {
        assert!(
            offset
                .checked_add(data.len() as u64)
                .is_some_and(|end| end <= buffer.0.size),
            "OpenGL buffer write is out of bounds"
        );
        if data.is_empty() {
            return;
        }
        let _ = self.shared.context.make_current();
        unsafe {
            (self.shared.gl.bind_buffer)(gl::COPY_WRITE_BUFFER, buffer.0.raw);
            (self.shared.gl.buffer_sub_data)(
                gl::COPY_WRITE_BUFFER,
                offset as isize,
                data.len() as isize,
                data.as_ptr().cast(),
            );
            (self.shared.gl.bind_buffer)(gl::COPY_WRITE_BUFFER, 0);
        }
    }

    fn write_texture(&self, desc: &WriteTextureDescriptor<Self>) {
        if desc.extent.width == 0
            || desc.extent.height == 0
            || desc.extent.depth_or_array_layers == 0
        {
            return;
        }
        let texture = &desc.texture.0;
        let (_, format, ty, texel_bytes) = pipeline::format_info(texture.format);
        upload_texture_bytes(
            &self.shared,
            texture.raw,
            texture.target,
            desc.mip_level,
            desc.origin,
            desc.extent,
            desc.buffer_layout.offset,
            desc.buffer_layout.bytes_per_row,
            desc.buffer_layout.rows_per_image,
            desc.data,
            format,
            ty,
            texel_bytes,
        );
    }

    fn create_command_encoder(&self, _label: &str) -> Self::CommandEncoder {
        self.shared
            .context
            .make_current()
            .expect("activate OpenGL context");
        OpenGlCommandEncoder {
            shared: self.shared.clone(),
            commands: Vec::new(),
        }
    }

    fn begin_render_pass<'a>(
        &self,
        encoder: &'a mut Self::CommandEncoder,
        desc: &RenderPassDescriptor<Self>,
    ) -> Self::RenderPass<'a> {
        let shared = encoder.shared.clone();
        pipeline::begin_render_pass(&shared, encoder, desc)
            .unwrap_or_else(|error| panic!("{} failed: {}", error.operation, error.message))
    }

    fn copy_buffer_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyBufferInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        encoder
            .commands
            .push(pipeline::OpenGlCommand::CopyBufferToTexture {
                buffer: src.buffer.clone(),
                texture: dst.texture.clone(),
                mip_level: dst.mip_level,
                origin: dst.origin,
                aspect: dst.aspect,
                layout: src.layout,
                extent,
            });
    }

    fn copy_texture_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        encoder
            .commands
            .push(pipeline::OpenGlCommand::CopyTextureToTexture {
                source: src.texture.clone(),
                source_mip: src.mip_level,
                source_origin: src.origin,
                source_aspect: src.aspect,
                destination: dst.texture.clone(),
                destination_mip: dst.mip_level,
                destination_origin: dst.origin,
                destination_aspect: dst.aspect,
                extent,
            });
    }

    fn copy_texture_to_buffer(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyBufferInfo<Self>,
        extent: Extent3d,
    ) {
        encoder
            .commands
            .push(pipeline::OpenGlCommand::CopyTextureToBuffer {
                texture: src.texture.clone(),
                mip_level: src.mip_level,
                origin: src.origin,
                aspect: src.aspect,
                buffer: dst.buffer.clone(),
                layout: dst.layout,
                extent,
            });
    }

    fn read_buffer(&self, buffer: &Self::Buffer, size: u64) -> Vec<u8> {
        assert!(
            size <= buffer.0.size,
            "OpenGL buffer read exceeds its allocation"
        );
        let mut bytes = vec![0; size as usize];
        if size == 0 {
            return bytes;
        }
        self.shared
            .context
            .make_current()
            .expect("activate OpenGL context");
        unsafe {
            (self.shared.gl.bind_buffer)(gl::COPY_READ_BUFFER, buffer.0.raw);
            (self.shared.gl.get_buffer_sub_data)(
                gl::COPY_READ_BUFFER,
                0,
                size as isize,
                bytes.as_mut_ptr().cast(),
            );
            (self.shared.gl.bind_buffer)(gl::COPY_READ_BUFFER, 0);
        }
        bytes
    }

    fn submit(&self, encoder: Self::CommandEncoder) {
        encoder
            .shared
            .context
            .make_current()
            .expect("activate OpenGL context");
        pipeline::submit_encoder(&encoder.shared, encoder.commands);
    }

    fn limits(&self) -> GpuLimits {
        self.shared.limits
    }

    fn format_is_srgb(format: Self::TextureFormat) -> bool {
        matches!(
            format,
            OpenGlTextureFormat::Rgba8UnormSrgb | OpenGlTextureFormat::Bgra8UnormSrgb
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw_cmd::DrawList;
    use crate::renderer::RendererImpl;
    use crate::utilities::{Color, Rect};

    #[test]
    fn generic_renderer_renders_and_reads_back_with_headless_opengl() {
        const SIZE: u32 = 32;
        let backend = match OpenGlBackend::new_headless() {
            Ok(backend) => backend,
            Err(error) => {
                eprintln!("skipping: a headless OpenGL 3.3 context is unavailable: {error}");
                return;
            }
        };
        let mut renderer =
            RendererImpl::<OpenGlBackend>::new(&backend, OpenGlBackend::rgba8_unorm_format());
        let mut draw = DrawList::new();
        draw.fill_rect(
            Rect::new(0.0, 0.0, SIZE as f32, (SIZE / 2) as f32),
            Color::red(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        draw.fill_rect(
            Rect::new(0.0, (SIZE / 2) as f32, SIZE as f32, (SIZE / 2) as f32),
            Color::blue(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        let target = backend.create_texture(&TextureDescriptor {
            label: Some("Cupid OpenGL readback test target".to_owned()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: OpenGlBackend::rgba8_unorm_format(),
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        let view = backend.create_texture_view(&target, "Cupid OpenGL test view");
        renderer.render(&backend, &view, SIZE, SIZE, false, &draw);
        let gl_error = unsafe { (backend.shared.gl.get_error)() };
        assert_eq!(gl_error, 0, "OpenGL reported an error after rendering");

        let row_bytes = SIZE * 4;
        let readback = backend.create_buffer(&BufferDescriptor {
            label: Some("Cupid OpenGL readback test buffer".to_owned()),
            size: (row_bytes * SIZE) as u64,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let mut encoder = backend.create_command_encoder("Cupid OpenGL readback test");
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
                    bytes_per_row: Some(row_bytes),
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
        let pixels = backend.read_buffer(&readback, (row_bytes * SIZE) as u64);
        let colored_pixels = pixels
            .chunks_exact(4)
            .filter(|pixel| pixel.iter().any(|channel| *channel != 0))
            .count();
        assert_eq!(
            &pixels[..4],
            &[0, 0, 255, 255],
            "the bottom logical band should stay at the bottom"
        );
        let top = ((SIZE - 1) * row_bytes) as usize;
        assert_eq!(
            &pixels[top..top + 4],
            &[255, 0, 0, 255],
            "the top logical band should stay at the top"
        );
        assert_eq!(colored_pixels, (SIZE * SIZE) as usize);
    }
}

fn buffer_usage(usage: &[BufferUsage]) -> u32 {
    if usage.contains(&BufferUsage::MapWrite) || usage.contains(&BufferUsage::CopyDst) {
        gl::DYNAMIC_DRAW
    } else if usage.contains(&BufferUsage::MapRead) || usage.contains(&BufferUsage::CopySrc) {
        gl::STREAM_DRAW
    } else {
        gl::STATIC_DRAW
    }
}

fn allocate_texture_levels(
    shared: &OpenGlShared,
    target: u32,
    internal_format: u32,
    external_format: u32,
    external_type: u32,
    desc: &TextureDescriptor<OpenGlTextureFormat>,
) {
    for level in 0..desc.mip_level_count {
        let width = (desc.size.0 >> level).max(1) as i32;
        let height = (desc.size.1 >> level).max(1) as i32;
        let depth = (desc.size.2 >> level).max(1) as i32;
        unsafe {
            match desc.dimension {
                TextureDimension::D1 if desc.size.2 > 1 => (shared.gl.tex_image_2d)(
                    target,
                    level as i32,
                    internal_format as i32,
                    width,
                    depth,
                    0,
                    external_format,
                    external_type,
                    null(),
                ),
                TextureDimension::D1 => (shared.gl.tex_image_1d)(
                    target,
                    level as i32,
                    internal_format as i32,
                    width,
                    0,
                    external_format,
                    external_type,
                    null(),
                ),
                TextureDimension::D2 if desc.size.2 > 1 => (shared.gl.tex_image_3d)(
                    target,
                    level as i32,
                    internal_format as i32,
                    width,
                    height,
                    depth,
                    0,
                    external_format,
                    external_type,
                    null(),
                ),
                TextureDimension::D2 => (shared.gl.tex_image_2d)(
                    target,
                    level as i32,
                    internal_format as i32,
                    width,
                    height,
                    0,
                    external_format,
                    external_type,
                    null(),
                ),
                TextureDimension::D3 => (shared.gl.tex_image_3d)(
                    target,
                    level as i32,
                    internal_format as i32,
                    width,
                    height,
                    depth,
                    0,
                    external_format,
                    external_type,
                    null(),
                ),
            }
        }
    }
}

fn upload_texture_bytes(
    shared: &OpenGlShared,
    texture: u32,
    target: u32,
    mip_level: u32,
    origin: Origin3d,
    extent: Extent3d,
    offset: u64,
    bytes_per_row: Option<u32>,
    rows_per_image: Option<u32>,
    data: &[u8],
    format: u32,
    ty: u32,
    texel_bytes: usize,
) {
    let tight_row = extent.width as usize * texel_bytes;
    let row_bytes = bytes_per_row.map_or(tight_row, |bytes| bytes as usize);
    let rows = rows_per_image.map_or(extent.height as usize, |rows| rows as usize);
    assert!(
        row_bytes >= tight_row && rows >= extent.height as usize,
        "OpenGL upload row layout is too small"
    );
    let offset = usize::try_from(offset).expect("OpenGL upload offset exceeds addressable range");
    let required = if extent.depth_or_array_layers == 0 {
        offset
    } else {
        offset
            + (extent.depth_or_array_layers as usize - 1) * row_bytes * rows
            + (extent.height as usize - 1) * row_bytes
            + tight_row
    };
    assert!(
        required <= data.len(),
        "OpenGL texture upload data is shorter than its layout"
    );
    let row_length = if row_bytes % texel_bytes == 0 {
        (row_bytes / texel_bytes) as i32
    } else {
        0
    };
    let use_tight = row_length == 0;
    let packed;
    let pointer = if use_tight {
        let length = tight_row * extent.height as usize * extent.depth_or_array_layers as usize;
        packed = repack_rows(
            data,
            offset,
            row_bytes,
            rows,
            tight_row,
            extent.height as usize,
            extent.depth_or_array_layers as usize,
        );
        assert_eq!(packed.len(), length);
        packed.as_ptr()
    } else {
        unsafe { data.as_ptr().add(offset) }
    };
    let _ = shared.context.make_current();
    unsafe {
        (shared.gl.bind_buffer)(gl::PIXEL_UNPACK_BUFFER, 0);
        shared.bind_texture(0, target, texture);
        (shared.gl.pixel_store_i)(gl::UNPACK_ALIGNMENT, 1);
        (shared.gl.pixel_store_i)(
            gl::UNPACK_ROW_LENGTH,
            if use_tight { 0 } else { row_length },
        );
        (shared.gl.pixel_store_i)(
            gl::UNPACK_IMAGE_HEIGHT,
            if use_tight { 0 } else { rows as i32 },
        );
        match target {
            gl::TEXTURE_1D => (shared.gl.tex_sub_image_1d)(
                target,
                mip_level as i32,
                origin.x as i32,
                extent.width as i32,
                format,
                ty,
                pointer.cast(),
            ),
            gl::TEXTURE_1D_ARRAY => (shared.gl.tex_sub_image_2d)(
                target,
                mip_level as i32,
                origin.x as i32,
                origin.z as i32,
                extent.width as i32,
                extent.depth_or_array_layers as i32,
                format,
                ty,
                pointer.cast(),
            ),
            gl::TEXTURE_2D => (shared.gl.tex_sub_image_2d)(
                target,
                mip_level as i32,
                origin.x as i32,
                origin.y as i32,
                extent.width as i32,
                extent.height as i32,
                format,
                ty,
                pointer.cast(),
            ),
            gl::TEXTURE_2D_ARRAY | gl::TEXTURE_3D => (shared.gl.tex_sub_image_3d)(
                target,
                mip_level as i32,
                origin.x as i32,
                origin.y as i32,
                origin.z as i32,
                extent.width as i32,
                extent.height as i32,
                extent.depth_or_array_layers as i32,
                format,
                ty,
                pointer.cast(),
            ),
            _ => panic!("unsupported OpenGL texture target {target:#x} for upload"),
        }
        (shared.gl.pixel_store_i)(gl::UNPACK_ROW_LENGTH, 0);
        (shared.gl.pixel_store_i)(gl::UNPACK_IMAGE_HEIGHT, 0);
        shared.bind_texture(0, target, 0);
    }
}

fn repack_rows(
    data: &[u8],
    offset: usize,
    row_bytes: usize,
    rows_per_image: usize,
    tight_row: usize,
    height: usize,
    layers: usize,
) -> Vec<u8> {
    let mut packed = Vec::with_capacity(tight_row * height * layers);
    for layer in 0..layers {
        for row in 0..height {
            let start = offset + layer * row_bytes * rows_per_image + row * row_bytes;
            packed.extend_from_slice(&data[start..start + tight_row]);
        }
    }
    packed
}

pub(super) fn execute_copy_buffer_to_texture(
    shared: &Rc<OpenGlShared>,
    buffer: &OpenGlBuffer,
    texture: &OpenGlTexture,
    mip_level: u32,
    origin: Origin3d,
    aspect: TextureAspect,
    layout: TexelCopyBufferLayout,
    extent: Extent3d,
) {
    let backend = OpenGlBackend {
        shared: shared.clone(),
    };
    let source = TexelCopyBufferInfo { buffer, layout };
    let destination = TexelCopyTextureInfo {
        texture,
        mip_level,
        origin,
        aspect,
    };
    let (_, format, ty, texel_bytes) = pipeline::format_info(texture.0.format);
    copy_buffer_to_texture(
        &backend,
        &source,
        &destination,
        extent,
        format,
        ty,
        texel_bytes,
    );
}

pub(super) fn execute_copy_texture_to_texture(
    shared: &Rc<OpenGlShared>,
    source: &OpenGlTexture,
    source_mip: u32,
    source_origin: Origin3d,
    source_aspect: TextureAspect,
    destination: &OpenGlTexture,
    destination_mip: u32,
    destination_origin: Origin3d,
    destination_aspect: TextureAspect,
    extent: Extent3d,
) {
    let backend = OpenGlBackend {
        shared: shared.clone(),
    };
    let source_info = TexelCopyTextureInfo {
        texture: source,
        mip_level: source_mip,
        origin: source_origin,
        aspect: source_aspect,
    };
    let destination_info = TexelCopyTextureInfo {
        texture: destination,
        mip_level: destination_mip,
        origin: destination_origin,
        aspect: destination_aspect,
    };
    copy_texture_to_texture(&backend, &source_info, &destination_info, extent);
}

pub(super) fn execute_copy_texture_to_buffer(
    shared: &Rc<OpenGlShared>,
    texture: &OpenGlTexture,
    mip_level: u32,
    origin: Origin3d,
    aspect: TextureAspect,
    buffer: &OpenGlBuffer,
    layout: TexelCopyBufferLayout,
    extent: Extent3d,
) {
    let backend = OpenGlBackend {
        shared: shared.clone(),
    };
    let source = TexelCopyTextureInfo {
        texture,
        mip_level,
        origin,
        aspect,
    };
    let destination = TexelCopyBufferInfo { buffer, layout };
    copy_texture_to_buffer(&backend, &source, &destination, extent);
}

fn copy_buffer_to_texture(
    backend: &OpenGlBackend,
    src: &TexelCopyBufferInfo<'_, OpenGlBackend>,
    dst: &TexelCopyTextureInfo<'_, OpenGlBackend>,
    extent: Extent3d,
    format: u32,
    ty: u32,
    texel_bytes: usize,
) {
    let texture = &dst.texture.0;
    let layout = src.layout;
    let row_bytes = layout
        .bytes_per_row
        .unwrap_or(extent.width * texel_bytes as u32) as usize;
    if row_bytes % texel_bytes != 0 {
        let needed = layout.offset
            + (row_bytes * extent.height as usize * extent.depth_or_array_layers as usize) as u64;
        let bytes = backend.read_buffer(src.buffer, needed);
        let desc = WriteTextureDescriptor {
            texture: dst.texture,
            mip_level: dst.mip_level,
            origin: dst.origin,
            aspect: dst.aspect,
            data: &bytes,
            buffer_layout: TexelCopyBufferLayout {
                offset: layout.offset,
                bytes_per_row: layout.bytes_per_row,
                rows_per_image: layout.rows_per_image,
            },
            extent,
        };
        backend.write_texture(&desc);
        return;
    }
    let row_length = (row_bytes / texel_bytes) as i32;
    let rows = layout.rows_per_image.unwrap_or(extent.height) as i32;
    let _ = backend.shared.context.make_current();
    unsafe {
        backend.shared.bind_texture(0, texture.target, texture.raw);
        (backend.shared.gl.bind_buffer)(gl::PIXEL_UNPACK_BUFFER, src.buffer.0.raw);
        (backend.shared.gl.pixel_store_i)(gl::UNPACK_ALIGNMENT, 1);
        (backend.shared.gl.pixel_store_i)(gl::UNPACK_ROW_LENGTH, row_length);
        (backend.shared.gl.pixel_store_i)(gl::UNPACK_IMAGE_HEIGHT, rows);
        let pointer = layout.offset as usize as *const c_void;
        match texture.target {
            gl::TEXTURE_1D => (backend.shared.gl.tex_sub_image_1d)(
                texture.target,
                dst.mip_level as i32,
                dst.origin.x as i32,
                extent.width as i32,
                format,
                ty,
                pointer,
            ),
            gl::TEXTURE_1D_ARRAY => (backend.shared.gl.tex_sub_image_2d)(
                texture.target,
                dst.mip_level as i32,
                dst.origin.x as i32,
                dst.origin.z as i32,
                extent.width as i32,
                extent.depth_or_array_layers as i32,
                format,
                ty,
                pointer,
            ),
            gl::TEXTURE_2D => (backend.shared.gl.tex_sub_image_2d)(
                texture.target,
                dst.mip_level as i32,
                dst.origin.x as i32,
                dst.origin.y as i32,
                extent.width as i32,
                extent.height as i32,
                format,
                ty,
                pointer,
            ),
            gl::TEXTURE_2D_ARRAY | gl::TEXTURE_3D => (backend.shared.gl.tex_sub_image_3d)(
                texture.target,
                dst.mip_level as i32,
                dst.origin.x as i32,
                dst.origin.y as i32,
                dst.origin.z as i32,
                extent.width as i32,
                extent.height as i32,
                extent.depth_or_array_layers as i32,
                format,
                ty,
                pointer,
            ),
            _ => panic!("unsupported OpenGL texture target for buffer upload"),
        }
        (backend.shared.gl.pixel_store_i)(gl::UNPACK_ROW_LENGTH, 0);
        (backend.shared.gl.pixel_store_i)(gl::UNPACK_IMAGE_HEIGHT, 0);
        (backend.shared.gl.bind_buffer)(gl::PIXEL_UNPACK_BUFFER, 0);
        backend.shared.bind_texture(0, texture.target, 0);
    }
}

fn copy_texture_to_texture(
    backend: &OpenGlBackend,
    src: &TexelCopyTextureInfo<'_, OpenGlBackend>,
    dst: &TexelCopyTextureInfo<'_, OpenGlBackend>,
    extent: Extent3d,
) {
    let mask = copy_aspect_mask(src.texture.0.format, src.aspect);
    let attachment = copy_attachment(src.texture.0.format, src.aspect);
    for layer in 0..extent.depth_or_array_layers {
        let src_view = pipeline::subresource_view(
            &backend.shared,
            src.texture,
            src.mip_level,
            src.origin.z + layer,
        );
        let dst_view = pipeline::subresource_view(
            &backend.shared,
            dst.texture,
            dst.mip_level,
            dst.origin.z + layer,
        );
        let src_fbo = pipeline::framebuffer_for_view(&backend.shared, &src_view, attachment)
            .expect("create OpenGL source framebuffer");
        let dst_fbo = pipeline::framebuffer_for_view(&backend.shared, &dst_view, attachment)
            .expect("create OpenGL destination framebuffer");
        backend
            .shared
            .bind_framebuffer(gl::READ_FRAMEBUFFER, src_fbo.framebuffer);
        backend
            .shared
            .bind_framebuffer(gl::DRAW_FRAMEBUFFER, dst_fbo.framebuffer);
        unsafe {
            if mask & gl::COLOR_BUFFER_BIT != 0 {
                (backend.shared.gl.read_buffer)(attachment);
            }
            (backend.shared.gl.blit_framebuffer)(
                src.origin.x as i32,
                src.origin.y as i32,
                (src.origin.x + extent.width) as i32,
                (src.origin.y + extent.height) as i32,
                dst.origin.x as i32,
                dst.origin.y as i32,
                (dst.origin.x + extent.width) as i32,
                (dst.origin.y + extent.height) as i32,
                mask,
                gl::NEAREST,
            );
        }
    }
}

fn copy_texture_to_buffer(
    backend: &OpenGlBackend,
    src: &TexelCopyTextureInfo<'_, OpenGlBackend>,
    dst: &TexelCopyBufferInfo<'_, OpenGlBackend>,
    extent: Extent3d,
) {
    let (_, format, ty, texel_bytes) = pipeline::format_info(src.texture.0.format);
    let row_bytes = dst
        .layout
        .bytes_per_row
        .unwrap_or(extent.width * texel_bytes as u32) as usize;
    assert_eq!(
        row_bytes % texel_bytes,
        0,
        "OpenGL readback row stride must be a whole number of texels"
    );
    let row_count = dst.layout.rows_per_image.unwrap_or(extent.height) as u64;
    let mask = copy_aspect_mask(src.texture.0.format, src.aspect);
    let attachment = copy_attachment(src.texture.0.format, src.aspect);
    for layer in 0..extent.depth_or_array_layers {
        let view = pipeline::subresource_view(
            &backend.shared,
            src.texture,
            src.mip_level,
            src.origin.z + layer,
        );
        let framebuffer = pipeline::framebuffer_for_view(&backend.shared, &view, attachment)
            .expect("create OpenGL readback framebuffer");
        backend
            .shared
            .bind_framebuffer(gl::READ_FRAMEBUFFER, framebuffer.framebuffer);
        unsafe {
            if mask & gl::COLOR_BUFFER_BIT != 0 {
                (backend.shared.gl.read_buffer)(attachment);
            }
            (backend.shared.gl.bind_buffer)(gl::PIXEL_PACK_BUFFER, dst.buffer.0.raw);
            (backend.shared.gl.pixel_store_i)(gl::PACK_ALIGNMENT, 1);
            (backend.shared.gl.pixel_store_i)(
                gl::PACK_ROW_LENGTH,
                (row_bytes / texel_bytes) as i32,
            );
            let offset = dst.layout.offset + layer as u64 * row_bytes as u64 * row_count;
            (backend.shared.gl.read_pixels)(
                src.origin.x as i32,
                src.origin.y as i32,
                extent.width as i32,
                extent.height as i32,
                format,
                ty,
                offset as usize as *mut c_void,
            );
            (backend.shared.gl.pixel_store_i)(gl::PACK_ROW_LENGTH, 0);
            (backend.shared.gl.bind_buffer)(gl::PIXEL_PACK_BUFFER, 0);
        }
    }
    backend.shared.bind_framebuffer(gl::READ_FRAMEBUFFER, 0);
}

fn copy_attachment(format: OpenGlTextureFormat, aspect: TextureAspect) -> u32 {
    match aspect {
        TextureAspect::DepthOnly => gl::DEPTH_ATTACHMENT,
        TextureAspect::StencilOnly => gl::STENCIL_ATTACHMENT,
        TextureAspect::All => match format {
            OpenGlTextureFormat::Depth24PlusStencil8 => gl::DEPTH_STENCIL_ATTACHMENT,
            OpenGlTextureFormat::Depth16Unorm
            | OpenGlTextureFormat::Depth24Plus
            | OpenGlTextureFormat::Depth32Float => gl::DEPTH_ATTACHMENT,
            _ => gl::COLOR_ATTACHMENT0,
        },
    }
}

fn copy_aspect_mask(format: OpenGlTextureFormat, aspect: TextureAspect) -> u32 {
    match aspect {
        TextureAspect::DepthOnly => gl::DEPTH_BUFFER_BIT,
        TextureAspect::StencilOnly => gl::STENCIL_BUFFER_BIT,
        TextureAspect::All => match format {
            OpenGlTextureFormat::Depth24PlusStencil8 => {
                gl::DEPTH_BUFFER_BIT | gl::STENCIL_BUFFER_BIT
            }
            OpenGlTextureFormat::Depth16Unorm
            | OpenGlTextureFormat::Depth24Plus
            | OpenGlTextureFormat::Depth32Float => gl::DEPTH_BUFFER_BIT,
            _ => gl::COLOR_BUFFER_BIT,
        },
    }
}
