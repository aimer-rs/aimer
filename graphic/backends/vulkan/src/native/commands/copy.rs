use ash::vk;

use crate::backend::{
    Extent3d, TexelCopyBufferInfo, TexelCopyTextureInfo, TextureAspect, TextureUsage,
};

use super::{VulkanBackend, VulkanCommandEncoder};

pub fn copy_buffer_to_texture(
    encoder: &mut VulkanCommandEncoder,
    src: &TexelCopyBufferInfo<VulkanBackend>,
    dst: &TexelCopyTextureInfo<VulkanBackend>,
    extent: Extent3d,
) {
    let texel_size = texel_size(dst.texture.0.format) as u32;
    let bytes_per_row = src.layout.bytes_per_row.unwrap_or(0);
    assert!(
        bytes_per_row == 0 || bytes_per_row % texel_size == 0,
        "Vulkan texture row stride must contain whole texels"
    );
    assert!(
        src.layout.offset % 4 == 0,
        "Vulkan buffer-to-image offset must be four-byte aligned"
    );
    let buffer_row_length = if bytes_per_row == 0 {
        0
    } else {
        bytes_per_row / texel_size
    };
    let image_height = src.layout.rows_per_image.unwrap_or(0);
    let is_3d = dst.texture.0.dimension == crate::backend::TextureDimension::D3;
    let region = vk::BufferImageCopy::default()
        .buffer_offset(src.layout.offset)
        .buffer_row_length(buffer_row_length)
        .buffer_image_height(image_height)
        .image_subresource(vk::ImageSubresourceLayers {
            aspect_mask: texture_aspect(dst.texture.0.format, dst.aspect),
            mip_level: dst.mip_level,
            base_array_layer: if is_3d { 0 } else { dst.origin.z },
            layer_count: if is_3d {
                1
            } else {
                extent.depth_or_array_layers
            },
        })
        .image_offset(vk::Offset3D {
            x: dst.origin.x as i32,
            y: dst.origin.y as i32,
            z: if is_3d { dst.origin.z as i32 } else { 0 },
        })
        .image_extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: if is_3d {
                extent.depth_or_array_layers
            } else {
                1
            },
        });
    encoder.image_barrier(dst.texture, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
    encoder.buffer_barrier(
        src.buffer,
        vk::AccessFlags::MEMORY_WRITE | vk::AccessFlags::HOST_WRITE,
        vk::AccessFlags::TRANSFER_READ,
        vk::PipelineStageFlags::ALL_COMMANDS | vk::PipelineStageFlags::HOST,
        vk::PipelineStageFlags::TRANSFER,
    );
    unsafe {
        encoder.shared.core.device.cmd_copy_buffer_to_image(
            encoder.raw,
            src.buffer.0.raw,
            dst.texture.0.raw,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[region],
        );
    }
    let final_layout = if dst.texture.0.presentable {
        vk::ImageLayout::PRESENT_SRC_KHR
    } else if dst.texture.0.usage.contains(&TextureUsage::TextureBinding) {
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
    } else {
        vk::ImageLayout::GENERAL
    };
    encoder.image_barrier(dst.texture, final_layout);
    encoder.retain((*src.buffer).clone());
    encoder.retain((*dst.texture).clone());
}

pub fn copy_texture_to_texture(
    encoder: &mut VulkanCommandEncoder,
    src: &TexelCopyTextureInfo<VulkanBackend>,
    dst: &TexelCopyTextureInfo<VulkanBackend>,
    extent: Extent3d,
) {
    let source_is_3d = src.texture.0.dimension == crate::backend::TextureDimension::D3;
    let destination_is_3d = dst.texture.0.dimension == crate::backend::TextureDimension::D3;
    let region = vk::ImageCopy::default()
        .src_subresource(vk::ImageSubresourceLayers {
            aspect_mask: texture_aspect(src.texture.0.format, src.aspect),
            mip_level: src.mip_level,
            base_array_layer: if source_is_3d { 0 } else { src.origin.z },
            layer_count: if source_is_3d {
                1
            } else {
                extent.depth_or_array_layers
            },
        })
        .src_offset(vk::Offset3D {
            x: src.origin.x as i32,
            y: src.origin.y as i32,
            z: if source_is_3d { src.origin.z as i32 } else { 0 },
        })
        .dst_subresource(vk::ImageSubresourceLayers {
            aspect_mask: texture_aspect(dst.texture.0.format, dst.aspect),
            mip_level: dst.mip_level,
            base_array_layer: if destination_is_3d { 0 } else { dst.origin.z },
            layer_count: if destination_is_3d {
                1
            } else {
                extent.depth_or_array_layers
            },
        })
        .dst_offset(vk::Offset3D {
            x: dst.origin.x as i32,
            y: dst.origin.y as i32,
            z: if destination_is_3d {
                dst.origin.z as i32
            } else {
                0
            },
        })
        .extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: if source_is_3d || destination_is_3d {
                extent.depth_or_array_layers
            } else {
                1
            },
        });
    encoder.image_barrier(src.texture, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
    encoder.image_barrier(dst.texture, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
    unsafe {
        encoder.shared.core.device.cmd_copy_image(
            encoder.raw,
            src.texture.0.raw,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            dst.texture.0.raw,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[region],
        );
    }
    encoder.retain((*src.texture).clone());
    encoder.retain((*dst.texture).clone());
}

pub fn copy_texture_to_buffer(
    encoder: &mut VulkanCommandEncoder,
    src: &TexelCopyTextureInfo<VulkanBackend>,
    dst: &TexelCopyBufferInfo<VulkanBackend>,
    extent: Extent3d,
) {
    let texel_size = texel_size(src.texture.0.format) as u32;
    let bytes_per_row = dst.layout.bytes_per_row.unwrap_or(0);
    assert!(
        bytes_per_row == 0 || bytes_per_row % texel_size == 0,
        "Vulkan texture row stride must contain whole texels"
    );
    assert!(
        dst.layout.offset % 4 == 0,
        "Vulkan image-to-buffer offset must be four-byte aligned"
    );
    let is_3d = src.texture.0.dimension == crate::backend::TextureDimension::D3;
    let region = vk::BufferImageCopy::default()
        .buffer_offset(dst.layout.offset)
        .buffer_row_length(if bytes_per_row == 0 {
            0
        } else {
            bytes_per_row / texel_size
        })
        .buffer_image_height(dst.layout.rows_per_image.unwrap_or(0))
        .image_subresource(vk::ImageSubresourceLayers {
            aspect_mask: texture_aspect(src.texture.0.format, src.aspect),
            mip_level: src.mip_level,
            base_array_layer: if is_3d { 0 } else { src.origin.z },
            layer_count: if is_3d {
                1
            } else {
                extent.depth_or_array_layers
            },
        })
        .image_offset(vk::Offset3D {
            x: src.origin.x as i32,
            y: src.origin.y as i32,
            z: if is_3d { src.origin.z as i32 } else { 0 },
        })
        .image_extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: if is_3d {
                extent.depth_or_array_layers
            } else {
                1
            },
        });
    encoder.image_barrier(src.texture, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
    unsafe {
        encoder.shared.core.device.cmd_copy_image_to_buffer(
            encoder.raw,
            src.texture.0.raw,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            dst.buffer.0.raw,
            &[region],
        );
    }
    encoder.buffer_barrier(
        dst.buffer,
        vk::AccessFlags::TRANSFER_WRITE,
        vk::AccessFlags::HOST_READ,
        vk::PipelineStageFlags::TRANSFER,
        vk::PipelineStageFlags::HOST,
    );
    encoder.retain((*src.texture).clone());
    encoder.retain((*dst.buffer).clone());
}

fn texture_aspect(format: vk::Format, requested: TextureAspect) -> vk::ImageAspectFlags {
    match requested {
        TextureAspect::DepthOnly => vk::ImageAspectFlags::DEPTH,
        TextureAspect::StencilOnly => vk::ImageAspectFlags::STENCIL,
        TextureAspect::All => super::super::image_aspect(format),
    }
}

fn texel_size(format: vk::Format) -> usize {
    match format {
        vk::Format::R8_UNORM | vk::Format::R8_UINT | vk::Format::R8_SINT => 1,
        vk::Format::R8G8_UNORM | vk::Format::R8G8_UINT | vk::Format::R8G8_SINT => 2,
        _ => 4,
    }
}
