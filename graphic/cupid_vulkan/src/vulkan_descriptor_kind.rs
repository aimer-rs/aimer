//! Descriptor-type policy for the native Vulkan backend.
//!
//! Kept free of `ash` so the mapping is testable on every host, although only
//! Linux and Android consume it.

use crate::backend::{BindingType, BufferBindingType};

/// The Vulkan descriptor family a bind-group entry must be created as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
pub(crate) enum DescriptorKind {
    UniformBuffer,
    UniformBufferDynamic,
    StorageBuffer,
    StorageBufferDynamic,
    SampledImage,
    Sampler,
    StorageImage,
}

/// Classifies a binding.
///
/// A buffer that declares `has_dynamic_offset` has to be created as a *dynamic*
/// descriptor. Vulkan ignores the offsets passed to `vkCmdBindDescriptorSets`
/// for a plain one, so every draw would read the uniform at offset zero; the
/// material pipeline, which selects a request's uniform by offset, then drew
/// each of its commands with the first command's data.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
pub(crate) fn descriptor_kind(ty: &BindingType) -> DescriptorKind {
    match ty {
        BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            ..
        } => DescriptorKind::UniformBuffer,
        BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: true,
            ..
        } => DescriptorKind::UniformBufferDynamic,
        BindingType::Buffer {
            has_dynamic_offset: false,
            ..
        } => DescriptorKind::StorageBuffer,
        BindingType::Buffer {
            has_dynamic_offset: true,
            ..
        } => DescriptorKind::StorageBufferDynamic,
        BindingType::Texture { .. } => DescriptorKind::SampledImage,
        BindingType::Sampler(_) => DescriptorKind::Sampler,
        BindingType::StorageTexture { .. } => DescriptorKind::StorageImage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{SamplerBindingType, TextureSampleType, TextureViewDimension};

    fn buffer(ty: BufferBindingType, has_dynamic_offset: bool) -> BindingType {
        BindingType::Buffer {
            ty,
            has_dynamic_offset,
            min_binding_size: None,
        }
    }

    #[test]
    fn a_uniform_buffer_with_a_dynamic_offset_is_a_dynamic_descriptor() {
        assert_eq!(
            descriptor_kind(&buffer(BufferBindingType::Uniform, true)),
            DescriptorKind::UniformBufferDynamic
        );
    }

    #[test]
    fn a_uniform_buffer_without_one_stays_static() {
        assert_eq!(
            descriptor_kind(&buffer(BufferBindingType::Uniform, false)),
            DescriptorKind::UniformBuffer
        );
    }

    #[test]
    fn storage_buffers_follow_the_same_rule() {
        for ty in [BufferBindingType::Storage, BufferBindingType::ReadOnlyStorage] {
            assert_eq!(
                descriptor_kind(&buffer(ty, false)),
                DescriptorKind::StorageBuffer
            );
            assert_eq!(
                descriptor_kind(&buffer(ty, true)),
                DescriptorKind::StorageBufferDynamic
            );
        }
    }

    #[test]
    fn images_and_samplers_are_unaffected() {
        let texture = BindingType::Texture {
            multisampled: false,
            view_dimension: TextureViewDimension::D2,
            sample_type: TextureSampleType::Float { filterable: true },
        };
        assert_eq!(descriptor_kind(&texture), DescriptorKind::SampledImage);
        assert_eq!(
            descriptor_kind(&BindingType::Sampler(SamplerBindingType::Filtering)),
            DescriptorKind::Sampler
        );
    }
}
