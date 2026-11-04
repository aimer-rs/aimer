//! Memory from other APIs, sampled without a copy.
//!
//! A video decoder that writes into an `AHardwareBuffer` (Android) hands the
//! renderer the buffer instead of its pixels. The device imports it as the
//! memory of a `VkImage`, so the picture is sampled where the producer wrote
//! it.
//!
//! Every extension this needs is optional. They are enabled together, and only
//! when the device and instance have all of them; otherwise
//! [`ExternalSupport`] says so and an import returns `None`, which draws
//! nothing.

use std::collections::HashMap;
use std::ffi::{CStr, c_char};

use ash::vk;

use super::VulkanTexture;

/// Which kinds of memory the device can import.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ExternalSupport {
    /// `AHardwareBuffer` memory (Android).
    pub(super) android_hardware_buffer: bool,
}

/// The device extensions importing an `AHardwareBuffer` needs: the extension
/// itself and everything it requires.
const ANDROID_HARDWARE_BUFFER: &[&CStr] = &[
    vk::ANDROID_EXTERNAL_MEMORY_ANDROID_HARDWARE_BUFFER_NAME,
    vk::KHR_SAMPLER_YCBCR_CONVERSION_NAME,
    vk::KHR_EXTERNAL_MEMORY_NAME,
    vk::EXT_QUEUE_FAMILY_FOREIGN_NAME,
    vk::KHR_DEDICATED_ALLOCATION_NAME,
    vk::KHR_MAINTENANCE1_NAME,
    vk::KHR_BIND_MEMORY2_NAME,
    vk::KHR_GET_MEMORY_REQUIREMENTS2_NAME,
];

/// The instance extensions the device extensions above depend on.
const INSTANCE: &[&CStr] = &[
    vk::KHR_GET_PHYSICAL_DEVICE_PROPERTIES2_NAME,
    vk::KHR_EXTERNAL_MEMORY_CAPABILITIES_NAME,
];

fn contains(names: &[*const c_char], wanted: &CStr) -> bool {
    // SAFETY: every pointer in the list is a NUL-terminated extension name,
    // either a `'static` constant or one the driver reported.
    names.iter().any(|&name| unsafe { CStr::from_ptr(name) } == wanted)
}

/// Adds the instance extensions importing needs to `extensions`, those not
/// already there, when the loader has all of them. Returns whether they are
/// enabled.
pub(super) fn enable_instance_extensions(entry: &ash::Entry, extensions: &mut Vec<*const c_char>) -> bool {
    if !cfg!(target_os = "android") {
        return false;
    }
    let Ok(available) = (unsafe { entry.enumerate_instance_extension_properties(None) }) else {
        return false;
    };
    let has = |wanted: &CStr| {
        available
            .iter()
            .any(|property| unsafe { CStr::from_ptr(property.extension_name.as_ptr()) } == wanted)
    };
    if !INSTANCE.iter().all(|wanted| has(wanted)) {
        return false;
    }
    for wanted in INSTANCE {
        if !contains(extensions, wanted) {
            extensions.push(wanted.as_ptr());
        }
    }
    true
}

impl ExternalSupport {
    /// Adds the device extensions for what `physical_device` can import to
    /// `extensions`, and says what that is.
    pub(super) fn select(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        instance_extensions_enabled: bool,
        extensions: &mut Vec<*const c_char>,
    ) -> Self {
        if !instance_extensions_enabled || !cfg!(target_os = "android") {
            return Self::default();
        }
        let Ok(available) = (unsafe { instance.enumerate_device_extension_properties(physical_device) }) else {
            return Self::default();
        };
        let has = |wanted: &CStr| {
            available
                .iter()
                .any(|property| unsafe { CStr::from_ptr(property.extension_name.as_ptr()) } == wanted)
        };
        if !ANDROID_HARDWARE_BUFFER.iter().all(|wanted| has(wanted)) {
            return Self::default();
        }
        for wanted in ANDROID_HARDWARE_BUFFER {
            if !contains(extensions, wanted) {
                extensions.push(wanted.as_ptr());
            }
        }
        Self {
            android_hardware_buffer: true,
        }
    }
}

/// Textures made from other APIs' buffers, by producer pool and slot.
#[derive(Default)]
pub(super) struct ImportedTextures {
    map: HashMap<(u32, u32), Imported>,
}

struct Imported {
    texture: VulkanTexture,
    size: (u32, u32),
    /// The picture last acquired from the producer.
    serial: u64,
}

impl ImportedTextures {
    /// Drops what belongs to pools that no longer exist.
    pub(super) fn prune(&mut self) {
        self.map
            .retain(|(pool, _), _| aimer_cupid_gpu::external_pool::is_live(*pool));
    }

    /// Entries held; for tests.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(target_os = "android")]
mod android {
    use std::sync::{Arc, Mutex};

    use ash::vk;

    use super::{Imported, ImportedTextures};
    use crate::backend::{GpuBackend, TextureDimension, TextureUsage};
    use crate::native::{VulkanBackend, VulkanCore, VulkanTexture, VulkanTextureInner, VulkanTextureView};

    impl VulkanBackend {
        /// A view of the picture `serial` in the `AHardwareBuffer` at `buffer`,
        /// importing the buffer the first time and taking ownership of it from
        /// the producer's GL context when the picture is new.
        ///
        /// # Safety
        ///
        /// `buffer` must be a valid `AHardwareBuffer*` of `R8G8B8A8` pixels the
        /// producer has finished writing, and stay valid until the GPU has
        /// finished with the frame.
        pub(in crate::native) unsafe fn import_hardware_buffer(
            &self,
            pool: u32,
            slot: u32,
            buffer: usize,
            serial: u64,
            size: (u32, u32),
        ) -> Option<VulkanTextureView> {
            let core = &self.shared.core;
            if !core.external.android_hardware_buffer || size.0 == 0 || size.1 == 0 {
                return None;
            }
            let (texture, acquire) = {
                let mut imports = self.imports.lock().ok()?;
                let kept = imports.map.get_mut(&(pool, slot)).filter(|kept| kept.size == size);
                if let Some(kept) = kept {
                    let acquire = kept.serial != serial;
                    kept.serial = serial;
                    (kept.texture.clone(), acquire)
                } else {
                    // SAFETY: the caller vouches for `buffer`.
                    let texture = unsafe { create_image(core, buffer as *mut vk::AHardwareBuffer, size) }?;
                    imports.map.insert(
                        (pool, slot),
                        Imported {
                            texture: texture.clone(),
                            size,
                            serial,
                        },
                    );
                    (texture, true)
                }
            };
            if acquire {
                let mut encoder = self.create_command_encoder("acquire external image");
                encoder.acquire_foreign_image(&texture);
                self.submit(encoder);
            }
            Some(self.create_texture_view(&texture, "video"))
        }
    }

    /// A sampled image on the memory of `buffer`.
    ///
    /// # Safety
    ///
    /// `buffer` must be a valid `AHardwareBuffer*`.
    unsafe fn create_image(
        core: &Arc<VulkanCore>,
        buffer: *mut vk::AHardwareBuffer,
        size: (u32, u32),
    ) -> Option<VulkanTexture> {
        let loader = ash::android::external_memory_android_hardware_buffer::Device::new(
            &core._owner.instance,
            &core.device,
        );
        let mut format = vk::AndroidHardwareBufferFormatPropertiesANDROID::default();
        let mut properties = vk::AndroidHardwareBufferPropertiesANDROID::default().push_next(&mut format);
        // SAFETY: a valid buffer and out-structures.
        unsafe { loader.get_android_hardware_buffer_properties(buffer, &mut properties) }.ok()?;
        let (allocation_size, type_bits) = (properties.allocation_size, properties.memory_type_bits);
        // Producers allocate plain RGBA; anything else (an opaque, external
        // format) would need a YCbCr sampler the image pipeline does not have.
        if format.format != vk::Format::R8G8B8A8_UNORM
            || !format.format_features.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
        {
            return None;
        }

        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);
        let info = vk::ImageCreateInfo::default()
            .push_next(&mut external)
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width: size.0,
                height: size.1,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: a valid description.
        let image = unsafe { core.device.create_image(&info, None) }.ok()?;

        let memory_type = (0..core.memory_properties.memory_type_count)
            .filter(|index| type_bits & (1 << index) != 0)
            .max_by_key(|index| {
                core.memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            });
        let Some(memory_type) = memory_type else {
            unsafe { core.device.destroy_image(image, None) };
            return None;
        };
        let mut import = vk::ImportAndroidHardwareBufferInfoANDROID::default();
        import.buffer = buffer;
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let allocate = vk::MemoryAllocateInfo::default()
            .allocation_size(allocation_size)
            .memory_type_index(memory_type)
            .push_next(&mut dedicated)
            .push_next(&mut import);
        // SAFETY: importing takes its own reference on the buffer.
        let memory = match unsafe { core.device.allocate_memory(&allocate, None) } {
            Ok(memory) => memory,
            Err(_) => {
                unsafe { core.device.destroy_image(image, None) };
                return None;
            }
        };
        // SAFETY: the memory was allocated for exactly this image.
        if unsafe { core.device.bind_image_memory(image, memory, 0) }.is_err() {
            unsafe {
                core.device.destroy_image(image, None);
                core.device.free_memory(memory, None);
            }
            return None;
        }
        Some(VulkanTexture(Arc::new(VulkanTextureInner {
            core: core.clone(),
            raw: image,
            allocation: None,
            size: (size.0, size.1, 1),
            format: vk::Format::R8G8B8A8_UNORM,
            dimension: TextureDimension::D2,
            sample_count: 1,
            usage: vec![TextureUsage::TextureBinding],
            layout: Mutex::new(vk::ImageLayout::UNDEFINED),
            presentable: false,
            imported_memory: Some(memory),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_android_set_names_the_extension_and_what_it_requires() {
        assert!(ANDROID_HARDWARE_BUFFER.contains(&vk::ANDROID_EXTERNAL_MEMORY_ANDROID_HARDWARE_BUFFER_NAME));
        // The Vulkan spec's required extensions of the Android one.
        for required in [
            vk::KHR_SAMPLER_YCBCR_CONVERSION_NAME,
            vk::KHR_EXTERNAL_MEMORY_NAME,
            vk::EXT_QUEUE_FAMILY_FOREIGN_NAME,
            vk::KHR_DEDICATED_ALLOCATION_NAME,
        ] {
            assert!(ANDROID_HARDWARE_BUFFER.contains(&required), "{required:?}");
        }
    }

    #[test]
    fn extensions_already_in_a_list_are_recognised() {
        let list = [vk::KHR_MAINTENANCE1_NAME.as_ptr(), vk::KHR_EXTERNAL_MEMORY_NAME.as_ptr()];
        assert!(contains(&list, vk::KHR_EXTERNAL_MEMORY_NAME));
        assert!(!contains(&list, vk::KHR_BIND_MEMORY2_NAME));
    }

    #[test]
    fn nothing_is_supported_by_default() {
        assert!(!ExternalSupport::default().android_hardware_buffer);
    }
}
