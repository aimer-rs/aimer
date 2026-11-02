use std::sync::Arc;

use ash::vk;

use super::commands::{self, VulkanCommandEncoder, VulkanRenderPass};
use super::*;

impl Drop for VulkanBufferInner {
    fn drop(&mut self) {
        unsafe { self.core.device.destroy_buffer(self.raw, None) };
    }
}

impl Drop for VulkanTextureInner {
    fn drop(&mut self) {
        if self.allocation.is_some() {
            unsafe { self.core.device.destroy_image(self.raw, None) };
        }
    }
}

impl Drop for VulkanTextureViewInner {
    fn drop(&mut self) {
        unsafe { self.core.device.destroy_image_view(self.raw, None) };
    }
}

impl Drop for VulkanSamplerInner {
    fn drop(&mut self) {
        unsafe { self.core.device.destroy_sampler(self.raw, None) };
    }
}

impl Drop for VulkanBindGroupLayoutInner {
    fn drop(&mut self) {
        unsafe {
            self.core
                .device
                .destroy_descriptor_set_layout(self.raw, None)
        };
    }
}

impl Drop for VulkanBindGroupInner {
    fn drop(&mut self) {
        unsafe { self.core.device.destroy_descriptor_pool(self.pool, None) };
    }
}

impl Drop for VulkanPipelineLayoutInner {
    fn drop(&mut self) {
        unsafe { self.core.device.destroy_pipeline_layout(self.raw, None) };
    }
}

impl Drop for VulkanRenderPipelineInner {
    fn drop(&mut self) {
        unsafe { self.core.device.destroy_pipeline(self.raw, None) };
    }
}

impl Drop for VulkanShaderModuleInner {
    fn drop(&mut self) {
        unsafe { self.core.device.destroy_shader_module(self.raw, None) };
    }
}

impl GpuBackend for VulkanBackend {
    type Buffer = VulkanBuffer;
    type Texture = VulkanTexture;
    type TextureView = VulkanTextureView;
    type BindGroupLayout = VulkanBindGroupLayout;
    type BindGroup = VulkanBindGroup;
    type PipelineLayout = VulkanPipelineLayout;
    type RenderPipeline = VulkanRenderPipeline;
    type Sampler = VulkanSampler;
    type ShaderModule = VulkanShaderModule;
    type CommandEncoder = VulkanCommandEncoder;
    type TextureFormat = vk::Format;
    type RenderPass<'a> = VulkanRenderPass<'a>;

    fn r8_unorm_format() -> Self::TextureFormat {
        vk::Format::R8_UNORM
    }

    fn rgba8_unorm_format() -> Self::TextureFormat {
        vk::Format::R8G8B8A8_UNORM
    }

    fn rect_shader_source(&self) -> &'static [u8] {
        include_bytes!("../../shaders/vulkan/rect.spv")
    }

    fn builtin_shader_source(&self, shader: BuiltinShader) -> &'static [u8] {
        match shader {
            BuiltinShader::Image => {
                #[cfg(target_os = "android")]
                {
                    include_bytes!("../../shaders/vulkan/image.android.spv")
                }
                #[cfg(not(target_os = "android"))]
                {
                    include_bytes!("../../shaders/vulkan/image.spv")
                }
            }
            BuiltinShader::Text => include_bytes!("../../shaders/vulkan/text.spv"),
            BuiltinShader::TextColor => {
                include_bytes!("../../shaders/vulkan/text_color.spv")
            }
            BuiltinShader::TextDecoration => {
                include_bytes!("../../shaders/vulkan/text_decoration.spv")
            }
            BuiltinShader::Svg => {
                #[cfg(target_os = "android")]
                {
                    include_bytes!("../../shaders/vulkan/svg.android.spv")
                }
                #[cfg(not(target_os = "android"))]
                {
                    include_bytes!("../../shaders/vulkan/svg.spv")
                }
            }
            BuiltinShader::FrameComposite => {
                include_bytes!("../../shaders/vulkan/frame_composite.spv")
            }
            BuiltinShader::Material => include_bytes!("../../shaders/vulkan/material.spv"),
            BuiltinShader::Morph => include_bytes!("../../shaders/vulkan/morph.spv"),
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
        if source.len() < 20 || source.len() % 4 != 0 {
            return Err(BackendError {
                operation: "create_shader_module",
                message: format!("{label} is not a complete SPIR-V module"),
            });
        }
        let words: Vec<u32> = source
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        if words.first().copied() != Some(0x0723_0203) {
            return Err(BackendError {
                operation: "create_shader_module",
                message: format!("{label} has an invalid SPIR-V magic number"),
            });
        }
        let create_info = vk::ShaderModuleCreateInfo::default().code(&words);
        let raw = unsafe {
            self.shared
                .core
                .device
                .create_shader_module(&create_info, None)
        }
        .map_err(|error| BackendError {
            operation: "create_shader_module",
            message: format!("{label}: {error:?}"),
        })?;
        Ok(VulkanShaderModule(Arc::new(VulkanShaderModuleInner {
            core: self.shared.core.clone(),
            raw,
        })))
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> Self::Buffer {
        self.create_buffer_inner(desc)
    }

    fn create_texture(&self, desc: &TextureDescriptor<Self::TextureFormat>) -> Self::Texture {
        self.create_texture_inner(desc)
    }

    fn create_texture_view(&self, texture: &Self::Texture, _label: &str) -> Self::TextureView {
        let view_type = match texture.0.dimension {
            TextureDimension::D1 if texture.0.size.2 > 1 => vk::ImageViewType::TYPE_1D_ARRAY,
            TextureDimension::D1 => vk::ImageViewType::TYPE_1D,
            TextureDimension::D2 if texture.0.size.2 > 1 => vk::ImageViewType::TYPE_2D_ARRAY,
            TextureDimension::D2 => vk::ImageViewType::TYPE_2D,
            TextureDimension::D3 => vk::ImageViewType::TYPE_3D,
        };
        let create_info = vk::ImageViewCreateInfo::default()
            .image(texture.0.raw)
            .view_type(view_type)
            .format(texture.0.format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: image_aspect(texture.0.format),
                base_mip_level: 0,
                level_count: vk::REMAINING_MIP_LEVELS,
                base_array_layer: 0,
                layer_count: if texture.0.dimension == TextureDimension::D3 {
                    1
                } else {
                    vk::REMAINING_ARRAY_LAYERS
                },
            });
        let raw = unsafe {
            self.shared
                .core
                .device
                .create_image_view(&create_info, None)
                .expect("create Vulkan image view")
        };
        VulkanTextureView(Arc::new(VulkanTextureViewInner {
            core: self.shared.core.clone(),
            raw,
            texture: texture.clone(),
        }))
    }

    fn create_sampler(&self, desc: &SamplerDescriptor) -> Self::Sampler {
        pipeline::create_sampler(&self.shared.core, desc)
    }

    fn create_bind_group_layout(&self, entries: &[BindGroupLayoutEntry]) -> Self::BindGroupLayout {
        pipeline::create_bind_group_layout(&self.shared.core, entries)
    }

    fn create_bind_group(
        &self,
        layout: &Self::BindGroupLayout,
        entries: &[BindGroupEntry<Self>],
    ) -> Self::BindGroup {
        pipeline::create_bind_group(&self.shared.core, layout, entries)
    }

    fn create_pipeline_layout(&self, layouts: &[&Self::BindGroupLayout]) -> Self::PipelineLayout {
        pipeline::create_pipeline_layout(&self.shared.core, layouts)
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
        pipeline::create_render_pipeline(&self.shared.core, desc)
    }

    fn write_buffer(&self, buffer: &Self::Buffer, offset: u64, data: &[u8]) {
        assert!(
            offset.saturating_add(data.len() as u64) <= buffer.0.size,
            "Vulkan buffer write is out of bounds"
        );
        if data.is_empty() {
            return;
        }
        let staging = self.create_upload_buffer(data);
        self.pending_buffers
            .lock()
            .expect("Vulkan pending upload mutex is not poisoned")
            .push(commands::PendingBufferUpload {
                staging,
                destination: (*buffer).clone(),
                offset,
            });
    }

    fn write_texture(&self, desc: &WriteTextureDescriptor<Self>) {
        if desc.extent.width == 0
            || desc.extent.height == 0
            || desc.extent.depth_or_array_layers == 0
        {
            return;
        }
        let texel_size = texel_size(desc.texture.0.format);
        let tight_row = desc.extent.width as usize * texel_size;
        let source_stride = desc
            .buffer_layout
            .bytes_per_row
            .map(|value| value as usize)
            .unwrap_or(tight_row);
        assert!(
            source_stride >= tight_row,
            "Vulkan texture upload row is shorter than its extent"
        );
        let input_rows = desc
            .buffer_layout
            .rows_per_image
            .map(|value| value as usize)
            .unwrap_or(desc.extent.height as usize);
        assert!(
            input_rows >= desc.extent.height as usize,
            "Vulkan texture upload image height is shorter than its extent"
        );
        let row_stride = align_up(source_stride as u64, texel_size.max(4) as u64) as usize;
        let image_stride = row_stride.saturating_mul(input_rows);
        let mut bytes =
            vec![0; image_stride.saturating_mul(desc.extent.depth_or_array_layers as usize)];
        for layer in 0..desc.extent.depth_or_array_layers as usize {
            for row in 0..desc.extent.height as usize {
                let src_offset = desc.buffer_layout.offset as usize
                    + layer * input_rows * source_stride
                    + row * source_stride;
                let dst_offset = layer * image_stride + row * row_stride;
                let src = desc
                    .data
                    .get(src_offset..src_offset.saturating_add(tight_row))
                    .expect("Vulkan texture upload data is shorter than its declared layout");
                bytes[dst_offset..dst_offset + tight_row].copy_from_slice(src);
            }
        }
        let staging = self.create_upload_buffer(&bytes);
        let row_length =
            u32::try_from(row_stride / texel_size).expect("Vulkan texture row length fits u32");
        self.pending_textures
            .lock()
            .expect("Vulkan pending upload mutex is not poisoned")
            .push(commands::PendingTextureUpload {
                staging,
                destination: (*desc.texture).clone(),
                mip_level: desc.mip_level,
                origin: desc.origin,
                aspect: desc.aspect,
                extent: desc.extent,
                row_length,
                image_height: input_rows as u32,
            });
    }

    fn create_command_encoder(&self, label: &str) -> Self::CommandEncoder {
        let buffers = std::mem::take(
            &mut *self
                .pending_buffers
                .lock()
                .expect("Vulkan pending upload mutex is not poisoned"),
        );
        let textures = std::mem::take(
            &mut *self
                .pending_textures
                .lock()
                .expect("Vulkan pending upload mutex is not poisoned"),
        );
        VulkanCommandEncoder::new(self.shared.clone(), label, buffers, textures)
    }

    fn begin_render_pass<'a>(
        &self,
        encoder: &'a mut Self::CommandEncoder,
        desc: &RenderPassDescriptor<Self>,
    ) -> Self::RenderPass<'a> {
        commands::begin_render_pass(encoder, desc)
    }

    fn copy_buffer_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyBufferInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        commands::copy_buffer_to_texture(encoder, src, dst, extent)
    }

    fn copy_texture_to_texture(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyTextureInfo<Self>,
        extent: Extent3d,
    ) {
        commands::copy_texture_to_texture(encoder, src, dst, extent)
    }

    fn copy_texture_to_buffer(
        &self,
        encoder: &mut Self::CommandEncoder,
        src: &TexelCopyTextureInfo<Self>,
        dst: &TexelCopyBufferInfo<Self>,
        extent: Extent3d,
    ) {
        commands::copy_texture_to_buffer(encoder, src, dst, extent)
    }

    fn read_buffer(&self, buffer: &Self::Buffer, size: u64) -> Vec<u8> {
        self.flush_pending_uploads();
        unsafe {
            self.shared
                .core
                .device
                .device_wait_idle()
                .expect("wait for Vulkan readback")
        };
        commands::collect_all(&self.shared);
        buffer.0.allocation.read(size.min(buffer.0.size))
    }

    fn submit(&self, encoder: Self::CommandEncoder) {
        let buffers = std::mem::take(
            &mut *self
                .pending_buffers
                .lock()
                .expect("Vulkan pending upload mutex is not poisoned"),
        );
        let textures = std::mem::take(
            &mut *self
                .pending_textures
                .lock()
                .expect("Vulkan pending upload mutex is not poisoned"),
        );
        if buffers.is_empty() && textures.is_empty() {
            encoder.submit();
        } else {
            let upload_encoder = VulkanCommandEncoder::new(
                self.shared.clone(),
                "Cupid Vulkan late uploads",
                buffers,
                textures,
            );
            encoder.submit_with_uploads(upload_encoder);
        }
    }

    fn limits(&self) -> GpuLimits {
        GpuLimits {
            max_texture_dimension_2d: self.shared.core.properties.limits.max_image_dimension2_d,
            min_uniform_buffer_offset_alignment: self
                .shared
                .core
                .properties
                .limits
                .min_uniform_buffer_offset_alignment
                as u32,
        }
    }

    fn format_is_srgb(format: Self::TextureFormat) -> bool {
        surface::is_srgb_format(format)
    }
}

impl VulkanBackend {
    fn flush_pending_uploads(&self) {
        let buffers = std::mem::take(
            &mut *self
                .pending_buffers
                .lock()
                .expect("Vulkan pending upload mutex is not poisoned"),
        );
        let textures = std::mem::take(
            &mut *self
                .pending_textures
                .lock()
                .expect("Vulkan pending upload mutex is not poisoned"),
        );
        if buffers.is_empty() && textures.is_empty() {
            return;
        }
        VulkanCommandEncoder::new(
            self.shared.clone(),
            "Cupid Vulkan readback uploads",
            buffers,
            textures,
        )
        .submit();
    }
}

fn texel_size(format: vk::Format) -> usize {
    match format {
        vk::Format::R8_UNORM | vk::Format::R8_UINT | vk::Format::R8_SINT => 1,
        vk::Format::R8G8_UNORM | vk::Format::R8G8_UINT | vk::Format::R8G8_SINT => 2,
        _ => 4,
    }
}

fn align_up(value: u64, alignment: u64) -> u64 {
    let alignment = alignment.max(1);
    value.saturating_add(alignment - 1) / alignment * alignment
}
