use std::borrow::Cow;
use std::collections::BTreeMap;
use std::rc::Rc;

use js_sys::{Float32Array, Uint8Array, Uint16Array, Uint32Array};
use web_sys::WebGl2RenderingContext as Gl;

use crate::backend::*;

use super::pipeline::{
    self, WebGl2BoundResource, WebGl2Buffer, WebGl2BufferInner, WebGl2CommandEncoder,
    WebGl2RenderPass, WebGl2Sampler, WebGl2SamplerInner, WebGl2ShaderModule, WebGl2Texture,
    WebGl2TextureFormat, WebGl2TextureInner, WebGl2TextureView, WebGl2TextureViewInner,
    WebGl2ViewTarget, backend_error,
};
use super::{WebGl2Backend, WebGl2Shared};

impl GpuBackend for WebGl2Backend {
    type Buffer = WebGl2Buffer;
    type Texture = WebGl2Texture;
    type TextureView = WebGl2TextureView;
    type BindGroupLayout = super::pipeline::WebGl2BindGroupLayout;
    type BindGroup = super::pipeline::WebGl2BindGroup;
    type PipelineLayout = super::pipeline::WebGl2PipelineLayout;
    type RenderPipeline = super::pipeline::WebGl2RenderPipeline;
    type Sampler = WebGl2Sampler;
    type ShaderModule = WebGl2ShaderModule;
    type CommandEncoder = WebGl2CommandEncoder;
    type TextureFormat = WebGl2TextureFormat;
    type RenderPass<'a>
        = WebGl2RenderPass<'a>
    where
        Self: 'a;

    fn r8_unorm_format() -> Self::TextureFormat {
        WebGl2TextureFormat::R8Unorm
    }

    fn rgba8_unorm_format() -> Self::TextureFormat {
        WebGl2TextureFormat::Rgba8Unorm
    }

    fn rect_shader_source(&self) -> &'static [u8] {
        crate::backend::glslpack::RECT
    }

    fn builtin_shader_source(&self, shader: BuiltinShader) -> &'static [u8] {
        match shader {
            BuiltinShader::Image => crate::backend::glslpack::IMAGE,
            BuiltinShader::Text => crate::backend::glslpack::TEXT,
            BuiltinShader::TextColor => {
                crate::backend::glslpack::TEXT_COLOR
            }
            BuiltinShader::TextDecoration => {
                crate::backend::glslpack::TEXT_DECORATION
            }
            BuiltinShader::Svg => crate::backend::glslpack::SVG,
            BuiltinShader::FrameComposite => {
                crate::backend::glslpack::FRAME_COMPOSITE
            }
            BuiltinShader::Material => {
                crate::backend::glslpack::MATERIAL
            }
            BuiltinShader::Morph => {
                crate::backend::glslpack::MORPH
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
        WebGl2ShaderModule::create(&self.shared, source, label)
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> Self::Buffer {
        let size =
            i32::try_from(desc.size).expect("WebGL2 buffer exceeds the signed address limit");
        let usage = if desc
            .usage
            .iter()
            .any(|usage| matches!(usage, BufferUsage::CopyDst | BufferUsage::MapWrite))
        {
            Gl::DYNAMIC_DRAW
        } else {
            Gl::STATIC_DRAW
        };
        let mut targets = Vec::new();
        for usage in &desc.usage {
            let target = match usage {
                BufferUsage::Vertex => Some(Gl::ARRAY_BUFFER),
                BufferUsage::Index => Some(Gl::ELEMENT_ARRAY_BUFFER),
                BufferUsage::Uniform => Some(Gl::UNIFORM_BUFFER),
                BufferUsage::CopySrc | BufferUsage::MapRead => Some(Gl::COPY_READ_BUFFER),
                BufferUsage::CopyDst | BufferUsage::MapWrite => Some(Gl::COPY_WRITE_BUFFER),
                BufferUsage::Storage | BufferUsage::ReadOnlyStorage | BufferUsage::Indirect => None,
            };
            if let Some(target) = target {
                if !targets.contains(&target) {
                    targets.push(target);
                }
            }
        }
        if targets.is_empty() {
            targets.push(Gl::COPY_WRITE_BUFFER);
        }

        // WebGL fixes a buffer object's binding target on first bind, so one
        // logical Cupid buffer needs a mirror for every target in its usage.
        let mut objects = Vec::with_capacity(targets.len());
        for target in targets {
            let object = self
                .shared
                .gl
                .create_buffer()
                .expect("WebGL2 createBuffer returned null");
            self.shared.gl.bind_buffer(target, Some(&object));
            self.shared.gl.buffer_data_with_i32(target, size, usage);
            self.shared.gl.bind_buffer(target, None);
            objects.push((target, object));
        }
        WebGl2Buffer(Rc::new(WebGl2BufferInner {
            shared: self.shared.clone(),
            objects,
            size: desc.size,
        }))
    }

    fn create_texture(&self, desc: &TextureDescriptor<Self::TextureFormat>) -> Self::Texture {
        assert!(
            desc.size.0 > 0 && desc.size.1 > 0 && desc.size.2 > 0,
            "WebGL2 texture extent must be non-zero"
        );
        assert_eq!(
            desc.sample_count, 1,
            "WebGL2 backend currently supports single-sample textures only"
        );
        assert_ne!(
            desc.mip_level_count, 0,
            "WebGL2 texture must have at least one mip level"
        );
        let target = match desc.dimension {
            TextureDimension::D1 => panic!("WebGL2 does not support one-dimensional textures"),
            TextureDimension::D2 if desc.size.2 > 1 => Gl::TEXTURE_2D_ARRAY,
            TextureDimension::D2 => Gl::TEXTURE_2D,
            TextureDimension::D3 => Gl::TEXTURE_3D,
        };
        let (internal_format, _, _, _) = format_info(desc.format);
        let object = self
            .shared
            .gl
            .create_texture()
            .expect("WebGL2 createTexture returned null");
        self.shared.gl.bind_texture(target, Some(&object));
        if target == Gl::TEXTURE_2D {
            self.shared.gl.tex_storage_2d(
                target,
                desc.mip_level_count as i32,
                internal_format,
                desc.size.0 as i32,
                desc.size.1 as i32,
            );
        } else {
            self.shared.gl.tex_storage_3d(
                target,
                desc.mip_level_count as i32,
                internal_format,
                desc.size.0 as i32,
                desc.size.1 as i32,
                desc.size.2 as i32,
            );
        }
        self.shared
            .gl
            .tex_parameteri(target, Gl::TEXTURE_BASE_LEVEL, 0);
        self.shared.gl.tex_parameteri(
            target,
            Gl::TEXTURE_MAX_LEVEL,
            desc.mip_level_count.saturating_sub(1) as i32,
        );
        self.shared.gl.bind_texture(target, None);
        WebGl2Texture(Rc::new(WebGl2TextureInner {
            shared: self.shared.clone(),
            id: self.shared.allocate_id(),
            object,
            target,
            size: desc.size,
            mip_level_count: desc.mip_level_count,
            format: desc.format,
        }))
    }

    fn create_texture_view(&self, texture: &Self::Texture, _label: &str) -> Self::TextureView {
        let layer = (texture.0.size.2 > 1).then_some(0);
        WebGl2TextureView(Rc::new(WebGl2TextureViewInner {
            target: WebGl2ViewTarget::Texture {
                texture: texture.clone(),
                level: 0,
                layer,
            },
        }))
    }

    fn create_sampler(&self, desc: &SamplerDescriptor) -> Self::Sampler {
        let object = self
            .shared
            .gl
            .create_sampler()
            .expect("WebGL2 createSampler returned null");
        let gl = &self.shared.gl;
        gl.sampler_parameteri(
            &object,
            Gl::TEXTURE_WRAP_S,
            address_mode(desc.address_mode_u) as i32,
        );
        gl.sampler_parameteri(
            &object,
            Gl::TEXTURE_WRAP_T,
            address_mode(desc.address_mode_v) as i32,
        );
        gl.sampler_parameteri(
            &object,
            Gl::TEXTURE_WRAP_R,
            address_mode(desc.address_mode_w) as i32,
        );
        gl.sampler_parameteri(
            &object,
            Gl::TEXTURE_MAG_FILTER,
            filter_mode(desc.mag_filter) as i32,
        );
        gl.sampler_parameteri(
            &object,
            Gl::TEXTURE_MIN_FILTER,
            sampler_filter(desc.min_filter, desc.mipmap_filter) as i32,
        );
        gl.sampler_parameterf(&object, Gl::TEXTURE_MIN_LOD, desc.lod_min_clamp);
        gl.sampler_parameterf(&object, Gl::TEXTURE_MAX_LOD, desc.lod_max_clamp);
        if let Some(compare) = desc.compare {
            gl.sampler_parameteri(
                &object,
                Gl::TEXTURE_COMPARE_MODE,
                Gl::COMPARE_REF_TO_TEXTURE as i32,
            );
            gl.sampler_parameteri(
                &object,
                Gl::TEXTURE_COMPARE_FUNC,
                compare_function(compare) as i32,
            );
        }
        WebGl2Sampler(Rc::new(WebGl2SamplerInner {
            shared: self.shared.clone(),
            object,
        }))
    }

    fn create_bind_group_layout(&self, entries: &[BindGroupLayoutEntry]) -> Self::BindGroupLayout {
        assert!(
            entries
                .iter()
                .all(|entry| entry.count.is_none_or(|count| count.get() == 1)),
            "WebGL2 backend does not support binding arrays",
        );
        super::pipeline::WebGl2BindGroupLayout(Rc::new(
            super::pipeline::WebGl2BindGroupLayoutInner {
                entries: entries.to_vec(),
            },
        ))
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
                .expect("WebGL2 bind-group resource must be present in its layout");
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
                "WebGL2 bind-group resource does not match its layout"
            );
            let resource = match &entry.resource {
                BindingResource::Buffer(buffer) => WebGl2BoundResource::Buffer((*buffer).clone()),
                BindingResource::BufferRange(buffer, offset, size) => {
                    WebGl2BoundResource::BufferRange((*buffer).clone(), *offset, *size)
                }
                BindingResource::TextureView(view) => {
                    WebGl2BoundResource::TextureView((*view).clone())
                }
                BindingResource::Sampler(sampler) => {
                    WebGl2BoundResource::Sampler((*sampler).clone())
                }
            };
            resources.insert(entry.binding, resource);
        }
        super::pipeline::WebGl2BindGroup(Rc::new(super::pipeline::WebGl2BindGroupInner {
            layout: layout.clone(),
            resources,
        }))
    }

    fn create_pipeline_layout(&self, layouts: &[&Self::BindGroupLayout]) -> Self::PipelineLayout {
        super::pipeline::WebGl2PipelineLayout(Rc::new(super::pipeline::WebGl2PipelineLayoutInner {
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
        write_buffer_immediate(&self.shared, buffer, offset, data);
    }

    fn write_texture(&self, desc: &WriteTextureDescriptor<Self>) {
        upload_texture_bytes(
            &self.shared,
            &desc.texture.0,
            desc.mip_level,
            desc.origin,
            desc.extent,
            desc.buffer_layout,
            desc.data,
            desc.aspect,
        )
        .unwrap_or_else(|error| panic!("{} failed: {}", error.operation, error.message));
    }

    fn create_command_encoder(&self, _label: &str) -> Self::CommandEncoder {
        WebGl2CommandEncoder {
            shared: self.shared.clone(),
        }
    }

    fn begin_render_pass<'a>(
        &self,
        encoder: &'a mut Self::CommandEncoder,
        desc: &RenderPassDescriptor<Self>,
    ) -> Self::RenderPass<'a> {
        pipeline::begin_render_pass(encoder, desc)
            .unwrap_or_else(|error| panic!("{} failed: {}", error.operation, error.message))
    }

    fn copy_buffer_to_texture(
        &self,
        _encoder: &mut Self::CommandEncoder,
        src: &TexelCopyBufferInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        let texture = &dst.texture.0;
        let (_, _, _, texel_bytes) = format_info(texture.format);
        let bytes_per_row = src
            .layout
            .bytes_per_row
            .unwrap_or(extent.width.saturating_mul(texel_bytes as u32));
        let rows_per_image = src.layout.rows_per_image.unwrap_or(extent.height);
        let length = (bytes_per_row as u64)
            .saturating_mul(rows_per_image as u64)
            .saturating_mul(extent.depth_or_array_layers as u64);
        let bytes = read_buffer_range(&self.shared, &src.buffer.0, src.layout.offset, length);
        let layout = TexelCopyBufferLayout {
            offset: 0,
            ..src.layout
        };
        upload_texture_bytes(
            &self.shared,
            texture,
            dst.mip_level,
            dst.origin,
            extent,
            layout,
            &bytes,
            dst.aspect,
        )
        .unwrap_or_else(|error| panic!("{} failed: {}", error.operation, error.message));
    }

    fn copy_texture_to_texture(
        &self,
        _encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        let source_view = texture_view(&src.texture.0, src.mip_level, src.origin.z);
        let destination_view = texture_view(&dst.texture.0, dst.mip_level, dst.origin.z);
        let src_framebuffer =
            pipeline::framebuffer_for_view(&self.shared, &source_view, Gl::COLOR_ATTACHMENT0)
                .unwrap_or_else(|error| panic!("{} failed: {}", error.operation, error.message))
                .expect("texture copy source cannot be the canvas surface");
        let dst_framebuffer =
            pipeline::framebuffer_for_view(&self.shared, &destination_view, Gl::COLOR_ATTACHMENT0)
                .unwrap_or_else(|error| panic!("{} failed: {}", error.operation, error.message))
                .expect("texture copy destination cannot be the canvas surface");
        let source_size = source_view.size();
        let destination_size = destination_view.size();
        let src_rect = framebuffer_copy_rect(src.origin, extent, source_size);
        let dst_rect = framebuffer_copy_rect(dst.origin, extent, destination_size);
        let gl = &self.shared.gl;
        gl.bind_framebuffer(Gl::READ_FRAMEBUFFER, Some(&src_framebuffer));
        gl.read_buffer(Gl::COLOR_ATTACHMENT0);
        gl.bind_framebuffer(Gl::DRAW_FRAMEBUFFER, Some(&dst_framebuffer));
        pipeline::set_draw_buffers(gl, &[Gl::COLOR_ATTACHMENT0]);
        gl.blit_framebuffer(
            src_rect[0],
            src_rect[1],
            src_rect[2],
            src_rect[3],
            dst_rect[0],
            dst_rect[1],
            dst_rect[2],
            dst_rect[3],
            Gl::COLOR_BUFFER_BIT,
            Gl::NEAREST,
        );
    }

    fn copy_texture_to_buffer(
        &self,
        _encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyBufferInfo<Self>,
        extent: Extent3d,
    ) {
        let texture = &src.texture.0;
        let (_, external_format, external_type, texel_bytes) = format_info(texture.format);
        let row_bytes = extent.width as usize * texel_bytes;
        let bytes_per_row = dst.layout.bytes_per_row.unwrap_or(row_bytes as u32) as usize;
        let rows_per_image = dst.layout.rows_per_image.unwrap_or(extent.height) as usize;
        assert!(
            bytes_per_row >= row_bytes,
            "WebGL2 readback row is too short"
        );
        assert!(
            rows_per_image >= extent.height as usize,
            "WebGL2 readback image pitch is too short"
        );
        let output_len = bytes_per_row
            .saturating_mul(rows_per_image)
            .saturating_mul(extent.depth_or_array_layers as usize);
        let mut output = vec![0; output_len];
        for layer in 0..extent.depth_or_array_layers {
            let view = texture_view(&src.texture.0, src.mip_level, src.origin.z + layer);
            let framebuffer =
                pipeline::framebuffer_for_view(&self.shared, &view, Gl::COLOR_ATTACHMENT0)
                    .unwrap_or_else(|error| panic!("{} failed: {}", error.operation, error.message))
                    .expect("texture readback source cannot be the canvas surface");
            let pixels = Uint8Array::new_with_length(
                u32::try_from(row_bytes.saturating_mul(extent.height as usize))
                    .expect("WebGL2 readback exceeds typed-array size"),
            );
            self.shared
                .gl
                .bind_framebuffer(Gl::FRAMEBUFFER, Some(&framebuffer));
            self.shared.gl.read_buffer(Gl::COLOR_ATTACHMENT0);
            self.shared.gl.pixel_storei(Gl::PACK_ALIGNMENT, 1);
            self.shared
                .gl
                .read_pixels_with_opt_array_buffer_view(
                    src.origin.x as i32,
                    view.size().1.saturating_sub(src.origin.y + extent.height) as i32,
                    extent.width as i32,
                    extent.height as i32,
                    external_format,
                    external_type,
                    Some(pixels.as_ref()),
                )
                .expect("WebGL2 readPixels failed");
            let pixel_bytes = pixels.to_vec();
            for row in 0..extent.height as usize {
                let source_start = row * row_bytes;
                let destination_start =
                    layer as usize * bytes_per_row * rows_per_image + row * bytes_per_row;
                output[destination_start..destination_start + row_bytes]
                    .copy_from_slice(&pixel_bytes[source_start..source_start + row_bytes]);
            }
        }
        let view = unsafe { uint8_view(&output) };
        self.shared.gl.bind_buffer(
            Gl::COPY_WRITE_BUFFER,
            Some(dst.buffer.0.object_for_target(Gl::COPY_WRITE_BUFFER)),
        );
        self.shared
            .gl
            .buffer_sub_data_with_i32_and_array_buffer_view(
                Gl::COPY_WRITE_BUFFER,
                i32::try_from(dst.layout.offset)
                    .expect("WebGL2 readback buffer offset exceeds signed range"),
                view.as_ref(),
            );
        self.shared.gl.bind_buffer(Gl::COPY_WRITE_BUFFER, None);
    }

    fn read_buffer(&self, buffer: &Self::Buffer, size: u64) -> Vec<u8> {
        assert!(
            size <= buffer.0.size,
            "WebGL2 buffer read exceeds allocation"
        );
        read_buffer_range(&self.shared, &buffer.0, 0, size)
    }

    fn submit(&self, encoder: Self::CommandEncoder) {
        // WebGL commands were issued directly to the context while the pass was open.
        drop(encoder);
    }

    fn limits(&self) -> GpuLimits {
        self.shared.limits
    }

    fn format_is_srgb(format: Self::TextureFormat) -> bool {
        matches!(
            format,
            WebGl2TextureFormat::Rgba8UnormSrgb | WebGl2TextureFormat::Bgra8UnormSrgb
        )
    }
}

pub(super) fn write_buffer_immediate(
    shared: &WebGl2Shared,
    buffer: &WebGl2Buffer,
    offset: u64,
    data: &[u8],
) {
    assert!(
        offset
            .checked_add(data.len() as u64)
            .is_some_and(|end| end <= buffer.0.size),
        "WebGL2 buffer write is out of bounds",
    );
    if data.is_empty() {
        return;
    }
    let offset = i32::try_from(offset).expect("WebGL2 buffer offset exceeds signed range");
    let view = unsafe { uint8_view(data) };
    for (target, object) in &buffer.0.objects {
        shared.gl.bind_buffer(*target, Some(object));
        shared
            .gl
            .buffer_sub_data_with_i32_and_array_buffer_view(*target, offset, view.as_ref());
        shared.gl.bind_buffer(*target, None);
    }
}

fn read_buffer_range(
    shared: &Rc<WebGl2Shared>,
    buffer: &WebGl2BufferInner,
    offset: u64,
    size: u64,
) -> Vec<u8> {
    assert!(
        offset
            .checked_add(size)
            .is_some_and(|end| end <= buffer.size)
    );
    let size_u32 = u32::try_from(size).expect("WebGL2 buffer read exceeds wasm typed-array size");
    let offset_i32 = i32::try_from(offset).expect("WebGL2 buffer offset exceeds signed range");
    let bytes = Uint8Array::new_with_length(size_u32);
    shared.gl.bind_buffer(
        Gl::COPY_READ_BUFFER,
        Some(buffer.object_for_target(Gl::COPY_READ_BUFFER)),
    );
    shared
        .gl
        .get_buffer_sub_data_with_i32_and_array_buffer_view(
            Gl::COPY_READ_BUFFER,
            offset_i32,
            bytes.as_ref(),
        );
    shared.gl.bind_buffer(Gl::COPY_READ_BUFFER, None);
    bytes.to_vec()
}

fn upload_texture_bytes(
    shared: &Rc<WebGl2Shared>,
    texture: &WebGl2TextureInner,
    mip_level: u32,
    origin: Origin3d,
    extent: Extent3d,
    layout: TexelCopyBufferLayout,
    data: &[u8],
    aspect: TextureAspect,
) -> Result<(), BackendError> {
    if extent.width == 0 || extent.height == 0 || extent.depth_or_array_layers == 0 {
        return Ok(());
    }
    if mip_level >= texture.mip_level_count {
        return Err(backend_error(
            "write_texture",
            "mip level exceeds the texture",
        ));
    }
    let (internal, format, ty, texel_bytes) = format_info(texture.format);
    let _ = internal;
    if matches!(
        texture.format,
        WebGl2TextureFormat::Depth16Unorm
            | WebGl2TextureFormat::Depth24Plus
            | WebGl2TextureFormat::Depth32Float
            | WebGl2TextureFormat::Depth24PlusStencil8
    ) && aspect == TextureAspect::StencilOnly
    {
        return Err(backend_error(
            "write_texture",
            "stencil-only upload is unsupported",
        ));
    }
    let row_bytes = extent.width as usize * texel_bytes;
    let bytes_per_row = layout
        .bytes_per_row
        .map_or(row_bytes, |value| value as usize);
    let rows_per_image = layout
        .rows_per_image
        .map_or(extent.height as usize, |value| value as usize);
    if bytes_per_row < row_bytes || rows_per_image < extent.height as usize {
        return Err(backend_error(
            "write_texture",
            "source row layout is smaller than the extent",
        ));
    }
    let first = usize::try_from(layout.offset)
        .map_err(|_| backend_error("write_texture", "source offset exceeds addressable memory"))?;
    let layer_stride = bytes_per_row
        .checked_mul(rows_per_image)
        .ok_or_else(|| backend_error("write_texture", "source layout overflows"))?;
    let upload_size = row_bytes
        .checked_mul(extent.height as usize)
        .and_then(|size| size.checked_mul(extent.depth_or_array_layers as usize))
        .ok_or_else(|| backend_error("write_texture", "upload extent overflows"))?;
    let is_bgra = matches!(
        texture.format,
        WebGl2TextureFormat::Bgra8Unorm | WebGl2TextureFormat::Bgra8UnormSrgb
    );
    let tightly_packed = first == 0
        && bytes_per_row == row_bytes
        && rows_per_image == extent.height as usize
        && !is_bgra;
    let packed = if tightly_packed {
        let source = data.get(..upload_size).ok_or_else(|| {
            backend_error("write_texture", "source data is shorter than its layout")
        })?;
        Cow::Borrowed(source)
    } else {
        let mut packed = Vec::with_capacity(upload_size);
        for layer in 0..extent.depth_or_array_layers as usize {
            for row in 0..extent.height as usize {
                let source_start =
                    first
                        .checked_add(layer.checked_mul(layer_stride).ok_or_else(|| {
                            backend_error("write_texture", "source layout overflows")
                        })?)
                        .and_then(|offset| offset.checked_add(row * bytes_per_row))
                        .ok_or_else(|| backend_error("write_texture", "source layout overflows"))?;
                let source_end = source_start
                    .checked_add(row_bytes)
                    .ok_or_else(|| backend_error("write_texture", "source layout overflows"))?;
                let source = data.get(source_start..source_end).ok_or_else(|| {
                    backend_error("write_texture", "source data is shorter than its layout")
                })?;
                if is_bgra {
                    for pixel in source.chunks_exact(4) {
                        packed.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                    }
                } else {
                    packed.extend_from_slice(source);
                }
            }
        }
        Cow::Owned(packed)
    };

    let x = i32::try_from(origin.x)
        .map_err(|_| backend_error("write_texture", "x origin exceeds signed range"))?;
    let y = i32::try_from(origin.y)
        .map_err(|_| backend_error("write_texture", "y origin exceeds signed range"))?;
    let z = i32::try_from(origin.z)
        .map_err(|_| backend_error("write_texture", "z origin exceeds signed range"))?;
    let width = extent.width as i32;
    let height = extent.height as i32;
    let depth = extent.depth_or_array_layers as i32;
    let gl = &shared.gl;
    gl.bind_texture(texture.target, Some(&texture.object));
    gl.pixel_storei(Gl::UNPACK_ALIGNMENT, 1);
    gl.pixel_storei(Gl::UNPACK_ROW_LENGTH, 0);
    gl.pixel_storei(Gl::UNPACK_SKIP_ROWS, 0);
    gl.pixel_storei(Gl::UNPACK_SKIP_PIXELS, 0);
    let result = upload_typed_pixels(
        gl,
        texture.target,
        mip_level as i32,
        x,
        y,
        z,
        width,
        height,
        depth,
        format,
        ty,
        packed.as_ref(),
    );
    gl.bind_texture(texture.target, None);
    result.map_err(|error| backend_error("write_texture", js_error(error)))
}

/// Keeps texture-copy rows at the same coordinates used by WebGL uploads and
/// sampling. Rebasing `y` against the destination height moves atlas contents
/// when a smaller texture is copied into a larger one.
fn framebuffer_copy_rect(origin: Origin3d, extent: Extent3d, texture_size: (u32, u32)) -> [i32; 4] {
    let right = origin
        .x
        .checked_add(extent.width)
        .expect("WebGL2 texture copy x range overflowed");
    let bottom = origin
        .y
        .checked_add(extent.height)
        .expect("WebGL2 texture copy y range overflowed");
    assert!(
        right <= texture_size.0 && bottom <= texture_size.1,
        "WebGL2 texture copy region exceeds its attachment"
    );
    [
        i32::try_from(origin.x).expect("WebGL2 texture copy x origin exceeds signed range"),
        i32::try_from(origin.y).expect("WebGL2 texture copy y origin exceeds signed range"),
        i32::try_from(right).expect("WebGL2 texture copy x end exceeds signed range"),
        i32::try_from(bottom).expect("WebGL2 texture copy y end exceeds signed range"),
    ]
}

#[allow(clippy::too_many_arguments)]
fn upload_typed_pixels(
    gl: &Gl,
    target: u32,
    level: i32,
    x: i32,
    y: i32,
    z: i32,
    width: i32,
    height: i32,
    depth: i32,
    format: u32,
    ty: u32,
    bytes: &[u8],
) -> Result<(), wasm_bindgen::JsValue> {
    match ty {
        Gl::UNSIGNED_BYTE => {
            let pixels = unsafe { uint8_view(bytes) };
            submit_texture_pixels(
                gl,
                target,
                level,
                x,
                y,
                z,
                width,
                height,
                depth,
                format,
                ty,
                pixels.as_ref(),
            )
        }
        Gl::HALF_FLOAT | Gl::UNSIGNED_SHORT => {
            let words: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|chunk| u16::from_ne_bytes([chunk[0], chunk[1]]))
                .collect();
            // SAFETY: `words` remains alive until the synchronous WebGL upload returns.
            let pixels = unsafe { Uint16Array::view(&words) };
            submit_texture_pixels(
                gl,
                target,
                level,
                x,
                y,
                z,
                width,
                height,
                depth,
                format,
                ty,
                pixels.as_ref(),
            )
        }
        Gl::UNSIGNED_INT | Gl::UNSIGNED_INT_24_8 => {
            let words: Vec<u32> = bytes
                .chunks_exact(4)
                .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();
            // SAFETY: `words` remains alive until the synchronous WebGL upload returns.
            let pixels = unsafe { Uint32Array::view(&words) };
            submit_texture_pixels(
                gl,
                target,
                level,
                x,
                y,
                z,
                width,
                height,
                depth,
                format,
                ty,
                pixels.as_ref(),
            )
        }
        Gl::FLOAT => {
            let words: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();
            // SAFETY: `words` remains alive until the synchronous WebGL upload returns.
            let pixels = unsafe { Float32Array::view(&words) };
            submit_texture_pixels(
                gl,
                target,
                level,
                x,
                y,
                z,
                width,
                height,
                depth,
                format,
                ty,
                pixels.as_ref(),
            )
        }
        _ => Err(wasm_bindgen::JsValue::from_str(
            "unsupported WebGL2 pixel type",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn submit_texture_pixels(
    gl: &Gl,
    target: u32,
    level: i32,
    x: i32,
    y: i32,
    z: i32,
    width: i32,
    height: i32,
    depth: i32,
    format: u32,
    ty: u32,
    pixels: &js_sys::Object,
) -> Result<(), wasm_bindgen::JsValue> {
    if target == Gl::TEXTURE_2D {
        gl.tex_sub_image_2d_with_i32_and_i32_and_u32_and_type_and_opt_array_buffer_view(
            target,
            level,
            x,
            y,
            width,
            height,
            format,
            ty,
            Some(pixels),
        )
    } else {
        gl.tex_sub_image_3d_with_opt_array_buffer_view(
            target,
            level,
            x,
            y,
            z,
            width,
            height,
            depth,
            format,
            ty,
            Some(pixels),
        )
    }
}

fn texture_view(texture: &Rc<WebGl2TextureInner>, level: u32, layer: u32) -> WebGl2TextureView {
    WebGl2TextureView(Rc::new(WebGl2TextureViewInner {
        target: WebGl2ViewTarget::Texture {
            texture: WebGl2Texture(texture.clone()),
            level,
            layer: (texture.size.2 > 1).then_some(layer),
        },
    }))
}

fn format_info(format: WebGl2TextureFormat) -> (u32, u32, u32, usize) {
    match format {
        WebGl2TextureFormat::R8Unorm => (Gl::R8, Gl::RED, Gl::UNSIGNED_BYTE, 1),
        WebGl2TextureFormat::Rg8Unorm => (Gl::RG8, Gl::RG, Gl::UNSIGNED_BYTE, 2),
        WebGl2TextureFormat::Rgba8Unorm | WebGl2TextureFormat::Bgra8Unorm => {
            (Gl::RGBA8, Gl::RGBA, Gl::UNSIGNED_BYTE, 4)
        }
        WebGl2TextureFormat::Rgba8UnormSrgb | WebGl2TextureFormat::Bgra8UnormSrgb => {
            (Gl::SRGB8_ALPHA8, Gl::RGBA, Gl::UNSIGNED_BYTE, 4)
        }
        WebGl2TextureFormat::Rgba16Float => (Gl::RGBA16F, Gl::RGBA, Gl::HALF_FLOAT, 8),
        WebGl2TextureFormat::Depth16Unorm => (
            Gl::DEPTH_COMPONENT16,
            Gl::DEPTH_COMPONENT,
            Gl::UNSIGNED_SHORT,
            2,
        ),
        WebGl2TextureFormat::Depth24Plus => (
            Gl::DEPTH_COMPONENT24,
            Gl::DEPTH_COMPONENT,
            Gl::UNSIGNED_INT,
            4,
        ),
        WebGl2TextureFormat::Depth24PlusStencil8 => (
            Gl::DEPTH24_STENCIL8,
            Gl::DEPTH_STENCIL,
            Gl::UNSIGNED_INT_24_8,
            4,
        ),
        WebGl2TextureFormat::Depth32Float => {
            (Gl::DEPTH_COMPONENT32F, Gl::DEPTH_COMPONENT, Gl::FLOAT, 4)
        }
    }
}

fn filter_mode(filter: FilterMode) -> u32 {
    match filter {
        FilterMode::Nearest => Gl::NEAREST,
        FilterMode::Linear => Gl::LINEAR,
    }
}

fn sampler_filter(filter: FilterMode, mip: FilterMode) -> u32 {
    match (filter, mip) {
        (FilterMode::Nearest, FilterMode::Nearest) => Gl::NEAREST_MIPMAP_NEAREST,
        (FilterMode::Linear, FilterMode::Nearest) => Gl::LINEAR_MIPMAP_NEAREST,
        (FilterMode::Nearest, FilterMode::Linear) => Gl::NEAREST_MIPMAP_LINEAR,
        (FilterMode::Linear, FilterMode::Linear) => Gl::LINEAR_MIPMAP_LINEAR,
    }
}

fn address_mode(mode: AddressMode) -> u32 {
    match mode {
        AddressMode::ClampToEdge => Gl::CLAMP_TO_EDGE,
        AddressMode::Repeat => Gl::REPEAT,
        AddressMode::MirrorRepeat => Gl::MIRRORED_REPEAT,
    }
}

fn compare_function(compare: CompareFunction) -> u32 {
    match compare {
        CompareFunction::Never => Gl::NEVER,
        CompareFunction::Less => Gl::LESS,
        CompareFunction::Equal => Gl::EQUAL,
        CompareFunction::LessEqual => Gl::LEQUAL,
        CompareFunction::Greater => Gl::GREATER,
        CompareFunction::NotEqual => Gl::NOTEQUAL,
        CompareFunction::GreaterEqual => Gl::GEQUAL,
        CompareFunction::Always => Gl::ALWAYS,
    }
}

fn js_error(value: wasm_bindgen::JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}

unsafe fn uint8_view(data: &[u8]) -> Uint8Array {
    // WebGL consumes this view synchronously; it is dropped before Rust can grow
    // wasm memory or otherwise invalidate the borrowed linear-memory range.
    unsafe { Uint8Array::view(data) }
}

#[cfg(test)]
mod tests {
    use super::framebuffer_copy_rect;
    use crate::backend::{Extent3d, Origin3d};

    #[test]
    fn atlas_copy_keeps_old_rows_anchored_when_the_destination_grows() {
        let extent = Extent3d {
            width: 512,
            height: 512,
            depth_or_array_layers: 1,
        };

        let source = framebuffer_copy_rect(Origin3d::ZERO, extent, (512, 512));
        let destination = framebuffer_copy_rect(Origin3d::ZERO, extent, (1024, 1024));

        assert_eq!(source, [0, 0, 512, 512]);
        assert_eq!(destination, source);
    }
}
