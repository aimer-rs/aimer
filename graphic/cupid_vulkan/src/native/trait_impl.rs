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
        include_bytes!("../../pipeline/shaders/vulkan/rect.spv")
    }

    fn builtin_shader_source(&self, shader: BuiltinShader) -> &'static [u8] {
        match shader {
            BuiltinShader::Image => {
                #[cfg(target_os = "android")]
                {
                    include_bytes!("../../pipeline/shaders/vulkan/image.android.spv")
                }
                #[cfg(not(target_os = "android"))]
                {
                    include_bytes!("../../pipeline/shaders/vulkan/image.spv")
                }
            }
            BuiltinShader::Text => include_bytes!("../../pipeline/shaders/vulkan/text.spv"),
            BuiltinShader::TextColor => {
                include_bytes!("../../pipeline/shaders/vulkan/text_color.spv")
            }
            BuiltinShader::TextDecoration => {
                include_bytes!("../../pipeline/shaders/vulkan/text_decoration.spv")
            }
            BuiltinShader::Svg => {
                #[cfg(target_os = "android")]
                {
                    include_bytes!("../../pipeline/shaders/vulkan/svg.android.spv")
                }
                #[cfg(not(target_os = "android"))]
                {
                    include_bytes!("../../pipeline/shaders/vulkan/svg.spv")
                }
            }
            BuiltinShader::FrameComposite => {
                include_bytes!("../../pipeline/shaders/vulkan/frame_composite.spv")
            }
            BuiltinShader::Material => include_bytes!("../../pipeline/shaders/vulkan/material.spv"),
            BuiltinShader::Morph => include_bytes!("../../pipeline/shaders/vulkan/morph.spv"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage_region::DamageSet;
    use crate::draw_cmd::DrawList;
    use crate::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderFrame, RenderNodeId, RenderTree};
    use crate::frame::{Frame, FramePacket, FrameRenderMetadata};
    use crate::renderer::RendererImpl;
    use crate::utilities::{Color, Rect, Vec2d};
    use std::sync::Arc;

    #[test]
    fn generic_renderer_draws_and_reads_back_with_native_vulkan() {
        const SIZE: u32 = 32;
        let Ok(backend) = VulkanBackend::new_headless() else {
            eprintln!("skipping: no Vulkan graphics device is available");
            return;
        };
        let mut renderer =
            RendererImpl::<VulkanBackend>::new(&backend, VulkanBackend::rgba8_unorm_format());
        let mut draw = DrawList::new();
        draw.fill_rect(
            Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
            Color::red(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        let target = backend.create_texture(&TextureDescriptor {
            label: Some("Cupid Vulkan readback test target".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: VulkanBackend::rgba8_unorm_format(),
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        let view = backend.create_texture_view(&target, "Cupid Vulkan test view");
        renderer.render(&backend, &view, SIZE, SIZE, false, &draw);

        let pixels = read_target(&backend, &target, SIZE, SIZE);
        let center = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        let colored_pixels = pixels
            .chunks_exact(4)
            .filter(|pixel| pixel.iter().any(|channel| *channel != 0))
            .count();
        assert_eq!(
            &pixels[center..center + 4],
            &[255, 0, 0, 255],
            "non-clear pixel count: {colored_pixels}; corner: {:?}",
            &pixels[..4]
        );
    }

    #[test]
    fn cropped_opacity_groups_composite_nested_damage_at_target_edges_on_native_vulkan() {
        const SIZE: u32 = 80;
        let Ok(backend) = VulkanBackend::new_headless() else {
            eprintln!("skipping: no Vulkan graphics device is available");
            return;
        };
        let mut renderer =
            RendererImpl::<VulkanBackend>::new(&backend, VulkanBackend::rgba8_unorm_format());
        let target = create_target(&backend, SIZE, SIZE);
        let view = backend.create_texture_view(&target, "Cupid Vulkan opacity-group target");
        let tree = RenderTree::new();
        let root = tree
            .add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32))
            .unwrap();
        tree.set_clip(root, Some(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)))
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
            yellow,
            Rect::new(0.0, 0.0, 32.0, 32.0),
            Color::rgba8(255, 255, 0, 255),
        );

        let metadata = || FrameRenderMetadata::new(1.0, 401, 1, 1, 1, DamageSet::new(SIZE, SIZE));
        let first_packet = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)],
                operations: tree.render_all(),
            },
            metadata(),
        )
        .expect("lower Vulkan opacity groups");
        renderer.render_packet(&backend, &view, &first_packet, false);
        let first_pixels = read_target(&backend, &target, SIZE, SIZE);
        assert_pixel_near(&first_pixels, SIZE, 20, 20, [0, 64, 0, 64]);
        assert_pixel_near(&first_pixels, SIZE, 70, 70, [64, 64, 0, 64]);
        assert_eq!(pixel(&first_pixels, SIZE, 2, 2), [0, 0, 0, 0]);
        let _ = tree.take_damage();

        tree.set_opacity(edge_group, 0.25).unwrap();
        let edge_damage = tree.take_damage();
        assert!(
            edge_damage.iter().any(|rect| {
                (rect.x + rect.width - SIZE as f32).abs() < f32::EPSILON
                    && (rect.y + rect.height - SIZE as f32).abs() < f32::EPSILON
            }),
            "edge-group damage reaches both target edges: {edge_damage:?}"
        );
        let update_packet = FramePacket::from_v2_direct_with_frame(
            RenderFrame {
                damage: edge_damage,
                operations: tree.render_all(),
            },
            metadata(),
            Frame::new(DrawList::new(), SIZE, SIZE),
        )
        .expect("lower Vulkan edge-opacity update");
        renderer.render_packet(&backend, &view, &update_packet, false);
        let updated_pixels = read_target(&backend, &target, SIZE, SIZE);
        assert_pixel_near(&updated_pixels, SIZE, 70, 70, [32, 32, 0, 32]);
        assert_pixel_near(&updated_pixels, SIZE, 20, 20, [0, 64, 0, 64]);
        assert_eq!(pixel(&updated_pixels, SIZE, 2, 2), [0, 0, 0, 0]);
    }

    fn record_v2_fill(tree: &RenderTree, node: RenderNodeId, rect: Rect, color: Color) {
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

    fn pixel(data: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * width + x) * 4) as usize;
        data[offset..offset + 4].try_into().expect("RGBA pixel")
    }

    fn assert_pixel_near(data: &[u8], width: u32, x: u32, y: u32, expected: [u8; 4]) {
        let actual = pixel(data, width, x, y);
        for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
            assert!(
                actual.abs_diff(expected) <= 2,
                "pixel ({x}, {y}) channel {channel}: expected {expected}, got {actual}"
            );
        }
    }

    #[test]
    fn generic_renderer_uploads_and_draws_image_and_text() {
        const WIDTH: u32 = 96;
        const HEIGHT: u32 = 64;
        let Ok(backend) = VulkanBackend::new_headless() else {
            eprintln!("skipping: no Vulkan graphics device is available");
            return;
        };
        let mut renderer =
            RendererImpl::<VulkanBackend>::new(&backend, VulkanBackend::rgba8_unorm_format());
        let mut draw = DrawList::new();
        let blue = [0, 0, 255, 255];
        let texture = draw.load_image(&blue.repeat(4), 2, 2);
        draw.draw_image(Rect::new(0.0, 0.0, 24.0, 24.0), texture);
        draw.draw_text(
            Vec2d::new(32.0, 40.0),
            Arc::from("A"),
            22.0,
            Color::white(),
            400,
        );
        let svg_scene = Arc::new(crate::svg::SvgScene {
            viewport: crate::svg::SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            nodes: Arc::new([crate::svg::SvgNode {
                node_id: crate::svg::SvgNodeId(0),
                svg_id: None,
                classes: Arc::new([]),
                element: crate::svg::SvgElementKind::Path,
                is_definition: false,
                definition_owner: None,
                clip_path: None,
                mask: None,
                filter: None,
                parent: None,
                children: Arc::new([]),
                transform: crate::svg::SvgTransform::default(),
                opacity: 1.0,
                geometry: Some(0),
                fill_rule: crate::svg::SvgFillRule::NonZero,
                fill: Some(crate::svg::SvgFill {
                    color: crate::svg::SvgColor::rgba8(255, 0, 0, 255),
                    rule: crate::svg::SvgFillRule::NonZero,
                }),
                stroke: None,
                fill_paint: None,
                stroke_paint: None,
                paint_order: crate::svg::SvgPaintOrder::FillAndStroke,
                visible: true,
            }]),
            geometries: Arc::new([crate::svg::SvgGeometry {
                commands: Arc::new([
                    crate::svg::SvgPathCommand::MoveTo { x: 0.0, y: 0.0 },
                    crate::svg::SvgPathCommand::LineTo { x: 10.0, y: 0.0 },
                    crate::svg::SvgPathCommand::LineTo { x: 10.0, y: 10.0 },
                    crate::svg::SvgPathCommand::LineTo { x: 0.0, y: 10.0 },
                    crate::svg::SvgPathCommand::Close,
                ]),
            }]),
            resources: Arc::new(crate::svg::SvgResourceGraph::empty()),
        });
        draw.draw_svg(svg_scene, Rect::new(64.0, 0.0, 24.0, 24.0), Arc::new([]));
        let target = create_target(&backend, WIDTH, HEIGHT);
        let view = backend.create_texture_view(&target, "Cupid Vulkan image and text target");
        renderer.render(&backend, &view, WIDTH, HEIGHT, false, &draw);
        let pixels = read_target(&backend, &target, WIDTH, HEIGHT);
        let image_center = ((12 * WIDTH + 12) * 4) as usize;
        assert_eq!(&pixels[image_center..image_center + 4], &[0, 0, 255, 255]);
        let svg_center = ((12 * WIDTH + 76) * 4) as usize;
        assert_eq!(&pixels[svg_center..svg_center + 4], &[255, 0, 0, 255]);
        let text_has_pixels = (30..HEIGHT).any(|y| {
            (28..WIDTH).any(|x| {
                let offset = ((y * WIDTH + x) * 4) as usize;
                pixels[offset + 3] != 0
            })
        });
        assert!(text_has_pixels, "text pipeline produced no covered pixels");
    }

    #[test]
    fn material_pipeline_renders_through_native_vulkan() {
        use crate::custom_pipeline::CustomPipelineGeneric;
        use crate::pipeline::material::{MaterialKind, MaterialPipeline, MaterialRequest};
        use std::any::Any;

        const SIZE: u32 = 32;
        let Ok(backend) = VulkanBackend::new_headless() else {
            eprintln!("skipping: no Vulkan graphics device is available");
            return;
        };
        let mut pipeline = MaterialPipeline::new(
            &backend,
            VulkanBackend::rgba8_unorm_format(),
            crate::AntiAlias::Analytic,
        );
        let request =
            MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, SIZE as f32, SIZE as f32]);
        pipeline.begin_frame();
        pipeline.prepare_command(&request as &(dyn Any + Send));
        let context = crate::custom_pipeline::RenderContextGeneric {
            backend: &backend,
            width: SIZE,
            height: SIZE,
            is_srgb: false,
            format: VulkanBackend::rgba8_unorm_format(),
            sample_count: 1,
            source_texture: None,
        };
        pipeline.prepare(&context);
        let target = create_target(&backend, SIZE, SIZE);
        let view = backend.create_texture_view(&target, "Cupid Vulkan material target");
        let mut encoder = backend.create_command_encoder("Cupid Vulkan material render");
        {
            let attachments = [RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear([0.0, 0.0, 0.0, 1.0]),
                    store: StoreOp::Store,
                },
            }];
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &RenderPassDescriptor {
                    label: Some("Cupid Vulkan material pass".to_string()),
                    color_attachments: &attachments,
                    depth_stencil_attachment: None,
                },
            );
            pipeline.render_command(Some(0), &mut pass);
        }
        backend.submit(encoder);
        let pixels = read_target(&backend, &target, SIZE, SIZE);
        let center = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        assert_ne!(&pixels[center..center + 4], &[0, 0, 0, 255]);
    }

    fn create_target(backend: &VulkanBackend, width: u32, height: u32) -> VulkanTexture {
        backend.create_texture(&TextureDescriptor {
            label: Some("Cupid Vulkan readback test target".to_string()),
            size: (width, height, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: VulkanBackend::rgba8_unorm_format(),
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        })
    }

    fn read_target(
        backend: &VulkanBackend,
        target: &VulkanTexture,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let row_bytes = width * 4;
        let readback = backend.create_buffer(&BufferDescriptor {
            label: Some("Cupid Vulkan readback test buffer".to_string()),
            size: (row_bytes * height) as u64,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let mut encoder = backend.create_command_encoder("Cupid Vulkan readback test");
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
                    rows_per_image: Some(height),
                },
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);
        backend.read_buffer(&readback, (row_bytes * height) as u64)
    }
}
