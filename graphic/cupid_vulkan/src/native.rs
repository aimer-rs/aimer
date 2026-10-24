//! Native Vulkan backend for Cupid's generic renderer.
//!
//! The backend owns its Vulkan instance and device. Use [`VulkanBackend::new_headless`]
//! for offscreen rendering or [`VulkanBackend::new_windowed`] to create a surface
//! that can acquire and present frames.
//!
//! # Safety invariants
//!
//! Every Vulkan object retains the `Arc<VulkanCore>` that owns its device and
//! instance. Command buffers retain every resource they reference until their
//! submission fence signals. Queue submission and presentation calls are
//! serialized with the corresponding queue mutex. Host-visible memory pointers
//! are used only while their memory block remains mapped, and allocation bounds
//! are checked before host copies.

mod allocator;
mod commands;
mod pipeline;
mod surface;
mod trait_impl;

pub use commands::{VulkanCommandEncoder, VulkanRenderPass};
pub use surface::{VulkanSurface, VulkanSurfaceFrame};

use std::ffi::{CStr, CString, c_void};
use std::sync::{Arc, Mutex};

use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::backend::*;

use self::allocator::{Allocation, MemoryAllocator, ResourceClass};

/// Error returned while creating or using Cupid's Vulkan context or surface.
#[derive(Clone, Debug)]
pub struct VulkanError {
    operation: &'static str,
    message: String,
}

impl VulkanError {
    fn new(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for VulkanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed: {}", self.operation, self.message)
    }
}

impl std::error::Error for VulkanError {}

/// Shared Vulkan instance, selected physical device, and logical device.
pub(super) struct VulkanCore {
    _owner: Arc<InstanceOwner>,
    pub(super) device: ash::Device,
    pub(super) physical_device: vk::PhysicalDevice,
    pub(super) properties: vk::PhysicalDeviceProperties,
    pub(super) memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub(super) enabled_features: vk::PhysicalDeviceFeatures,
}

struct InstanceOwner {
    entry: ash::Entry,
    instance: ash::Instance,
    debug_utils: Option<ash::ext::debug_utils::Instance>,
    debug_messenger: vk::DebugUtilsMessengerEXT,
    surface_maintenance1_enabled: bool,
}

pub(super) struct VulkanShared {
    pub(super) core: Arc<VulkanCore>,
    pub(super) graphics_queue: vk::Queue,
    pub(super) present_queue: Option<vk::Queue>,
    pub(super) graphics_family: u32,
    pub(super) present_family: Option<u32>,
    pub(in crate::native) allocator: MemoryAllocator,
    pub(super) queue_lock: Mutex<()>,
    pub(super) present_lock: Mutex<()>,
    pub(in crate::native) in_flight: Mutex<Vec<commands::InFlight>>,
}

/// Native Vulkan implementation of [`GpuBackend`].
pub struct VulkanBackend {
    pub(super) shared: Arc<VulkanShared>,
    pending_buffers: Mutex<Vec<commands::PendingBufferUpload>>,
    pending_textures: Mutex<Vec<commands::PendingTextureUpload>>,
}

/// A Vulkan buffer owned by Cupid.
#[doc(hidden)]
#[derive(Clone)]
pub struct VulkanBuffer(Arc<VulkanBufferInner>);

struct VulkanBufferInner {
    core: Arc<VulkanCore>,
    raw: vk::Buffer,
    allocation: Allocation,
    size: u64,
}

/// A Vulkan image owned by Cupid.
#[doc(hidden)]
#[derive(Clone)]
pub struct VulkanTexture(pub(super) Arc<VulkanTextureInner>);

pub(super) struct VulkanTextureInner {
    pub(super) core: Arc<VulkanCore>,
    pub(super) raw: vk::Image,
    pub(in crate::native) allocation: Option<Allocation>,
    pub(super) size: (u32, u32, u32),
    pub(super) format: vk::Format,
    pub(super) dimension: TextureDimension,
    pub(super) sample_count: u32,
    pub(super) usage: Vec<TextureUsage>,
    pub(super) layout: Mutex<vk::ImageLayout>,
    pub(super) presentable: bool,
}

/// A Vulkan image view.
#[doc(hidden)]
#[derive(Clone)]
pub struct VulkanTextureView(pub(super) Arc<VulkanTextureViewInner>);

pub(super) struct VulkanTextureViewInner {
    pub(super) core: Arc<VulkanCore>,
    pub(super) raw: vk::ImageView,
    pub(super) texture: VulkanTexture,
}

/// A Vulkan sampler.
#[doc(hidden)]
#[derive(Clone)]
pub struct VulkanSampler(pub(super) Arc<VulkanSamplerInner>);

pub(super) struct VulkanSamplerInner {
    pub(super) core: Arc<VulkanCore>,
    pub(super) raw: vk::Sampler,
}

/// A Vulkan descriptor set layout.
#[doc(hidden)]
#[derive(Clone)]
pub struct VulkanBindGroupLayout(pub(super) Arc<VulkanBindGroupLayoutInner>);

pub(super) struct VulkanBindGroupLayoutInner {
    pub(super) core: Arc<VulkanCore>,
    pub(super) raw: vk::DescriptorSetLayout,
    pub(super) entries: Vec<BindGroupLayoutEntry>,
}

/// A Vulkan descriptor set and the resources it binds.
#[doc(hidden)]
#[derive(Clone)]
pub struct VulkanBindGroup(pub(super) Arc<VulkanBindGroupInner>);

pub(super) struct VulkanBindGroupInner {
    pub(super) core: Arc<VulkanCore>,
    pub(super) pool: vk::DescriptorPool,
    pub(super) raw: vk::DescriptorSet,
    pub(super) _layout: VulkanBindGroupLayout,
    pub(super) _buffers: Vec<VulkanBuffer>,
    pub(super) _textures: Vec<VulkanTextureView>,
    pub(super) _samplers: Vec<VulkanSampler>,
}

/// A Vulkan pipeline layout.
#[doc(hidden)]
#[derive(Clone)]
pub struct VulkanPipelineLayout(pub(super) Arc<VulkanPipelineLayoutInner>);

pub(super) struct VulkanPipelineLayoutInner {
    pub(super) core: Arc<VulkanCore>,
    pub(super) raw: vk::PipelineLayout,
    pub(super) _layouts: Vec<VulkanBindGroupLayout>,
}

/// A Vulkan graphics pipeline.
#[doc(hidden)]
#[derive(Clone)]
pub struct VulkanRenderPipeline(pub(super) Arc<VulkanRenderPipelineInner>);

pub(super) struct VulkanRenderPipelineInner {
    pub(super) core: Arc<VulkanCore>,
    pub(super) raw: vk::Pipeline,
    pub(super) layout: VulkanPipelineLayout,
    pub(in crate::native) _compatible_render_pass: Arc<pipeline::VulkanRenderPassObject>,
}

/// A Vulkan shader module containing SPIR-V.
#[doc(hidden)]
#[derive(Clone)]
pub struct VulkanShaderModule(pub(super) Arc<VulkanShaderModuleInner>);

pub(super) struct VulkanShaderModuleInner {
    pub(super) core: Arc<VulkanCore>,
    pub(super) raw: vk::ShaderModule,
}

impl VulkanBackend {
    /// Creates a Vulkan backend for offscreen rendering.
    ///
    /// In debug builds, the Khronos validation layer is enabled automatically
    /// when installed. Set `AIMER_CUPID_VULKAN_VALIDATION=1` to require it and
    /// turn validation warnings and errors into stderr diagnostics.
    pub fn new_headless() -> Result<Self, VulkanError> {
        let owner = create_instance_owner(&[], false)?;
        let (core, graphics_family, _) = VulkanCore::create(owner, None)?;
        Ok(Self::from_shared(VulkanShared::new(
            core,
            graphics_family,
            None,
        )))
    }

    /// Creates a Vulkan backend and presentation surface for a window.
    ///
    /// The surface retains `window` so the native handles remain valid until
    /// the Vulkan surface is destroyed.
    pub fn new_windowed<W>(
        window: Arc<W>,
        initial_size: (u32, u32),
    ) -> Result<(Self, VulkanSurface<W>), VulkanError>
    where
        W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static,
    {
        surface::create_windowed(window, initial_size)
    }

    /// Creates another surface for this window-compatible Vulkan device.
    ///
    /// This lets Android and other hosts recreate their window surface across
    /// native-window lifecycle changes without rebuilding the device. The
    /// backend must have been initialized with [`VulkanBackend::new_windowed`].
    pub fn create_surface<W>(
        &self,
        window: Arc<W>,
        initial_size: (u32, u32),
    ) -> Result<VulkanSurface<W>, VulkanError>
    where
        W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static,
    {
        surface::create_for_backend(self.shared.clone(), window, initial_size)
    }

    fn from_shared(shared: Arc<VulkanShared>) -> Self {
        Self {
            shared,
            pending_buffers: Mutex::new(Vec::new()),
            pending_textures: Mutex::new(Vec::new()),
        }
    }

    /// Returns the selected physical-device name.
    pub fn device_name(&self) -> String {
        unsafe { CStr::from_ptr(self.shared.core.properties.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    /// Creates a buffer with the requested Vulkan usage.
    #[doc(hidden)]
    pub fn create_buffer_inner(&self, desc: &BufferDescriptor) -> VulkanBuffer {
        let mut usage = buffer_usage_flags(&desc.usage);
        // Cupid's queue-level writes use staging copies into the destination.
        usage |= vk::BufferUsageFlags::TRANSFER_DST;
        let create_info = vk::BufferCreateInfo::default()
            .size(desc.size.max(4))
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let raw = unsafe {
            self.shared
                .core
                .device
                .create_buffer(&create_info, None)
                .expect("create Vulkan buffer")
        };
        let requirements = unsafe { self.shared.core.device.get_buffer_memory_requirements(raw) };
        let (required, preferred) = if desc.usage.contains(&BufferUsage::MapRead)
            || desc.usage.contains(&BufferUsage::MapWrite)
        {
            (
                vk::MemoryPropertyFlags::HOST_VISIBLE,
                vk::MemoryPropertyFlags::HOST_COHERENT | vk::MemoryPropertyFlags::HOST_CACHED,
            )
        } else {
            (
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )
        };
        let allocation = self.shared.allocator.allocate(
            requirements,
            required,
            preferred,
            ResourceClass::Linear,
        );
        unsafe {
            self.shared
                .core
                .device
                .bind_buffer_memory(raw, allocation.memory(), allocation.offset())
                .expect("bind Vulkan buffer memory");
        }
        VulkanBuffer(Arc::new(VulkanBufferInner {
            core: self.shared.core.clone(),
            raw,
            allocation,
            size: desc.size.max(4),
        }))
    }

    fn create_upload_buffer(&self, data: &[u8]) -> VulkanBuffer {
        let buffer = self.create_buffer_inner(&BufferDescriptor {
            label: Some("Cupid Vulkan staging upload".to_string()),
            size: data.len().max(4) as u64,
            usage: vec![BufferUsage::CopySrc, BufferUsage::MapWrite],
        });
        buffer.0.allocation.write(0, data);
        buffer
    }

    fn create_texture_inner(&self, desc: &TextureDescriptor<vk::Format>) -> VulkanTexture {
        let image_type = match desc.dimension {
            TextureDimension::D1 => vk::ImageType::TYPE_1D,
            TextureDimension::D2 => vk::ImageType::TYPE_2D,
            TextureDimension::D3 => vk::ImageType::TYPE_3D,
        };
        let create_info = vk::ImageCreateInfo::default()
            .image_type(image_type)
            .format(desc.format)
            .extent(vk::Extent3D {
                width: desc.size.0.max(1),
                height: desc.size.1.max(1),
                depth: desc.size.2.max(1),
            })
            .mip_levels(desc.mip_level_count.max(1))
            .array_layers(if desc.dimension == TextureDimension::D3 {
                1
            } else {
                desc.size.2.max(1)
            })
            .samples(sample_count(desc.sample_count))
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(texture_usage_flags(&desc.usage, desc.format))
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let raw = unsafe {
            self.shared
                .core
                .device
                .create_image(&create_info, None)
                .expect("create Vulkan image")
        };
        let requirements = unsafe { self.shared.core.device.get_image_memory_requirements(raw) };
        let allocation = self.shared.allocator.allocate(
            requirements,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            ResourceClass::OptimalImage,
        );
        unsafe {
            self.shared
                .core
                .device
                .bind_image_memory(raw, allocation.memory(), allocation.offset())
                .expect("bind Vulkan image memory");
        }
        VulkanTexture(Arc::new(VulkanTextureInner {
            core: self.shared.core.clone(),
            raw,
            allocation: Some(allocation),
            size: desc.size,
            format: desc.format,
            dimension: desc.dimension,
            sample_count: desc.sample_count.max(1),
            usage: desc.usage.clone(),
            layout: Mutex::new(vk::ImageLayout::UNDEFINED),
            presentable: false,
        }))
    }
}

impl VulkanCore {
    fn create(
        owner: Arc<InstanceOwner>,
        surface: Option<&surface::VulkanSurfaceOwner>,
    ) -> Result<(Arc<Self>, u32, Option<u32>), VulkanError> {
        let instance = &owner.instance;
        let properties2_loader = owner.surface_maintenance1_enabled.then(|| {
            ash::khr::get_physical_device_properties2::Instance::new(&owner.entry, instance)
        });
        let physical_devices = unsafe { instance.enumerate_physical_devices() }
            .map_err(|error| VulkanError::new("enumerate Vulkan devices", format!("{error:?}")))?;
        let mut selected = None;
        for physical_device in physical_devices {
            let properties = unsafe { instance.get_physical_device_properties(physical_device) };
            let families =
                unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
            let graphics = families.iter().position(|family| {
                family.queue_count > 0 && family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
            });
            let Some(graphics) = graphics else { continue };
            let present = if let Some(surface) = surface {
                (0..families.len()).find(|index| {
                    families[*index].queue_count > 0
                        && unsafe {
                            surface
                                .loader
                                .get_physical_device_surface_support(
                                    physical_device,
                                    *index as u32,
                                    surface.raw,
                                )
                                .unwrap_or(false)
                        }
                })
            } else {
                None
            };
            if surface.is_some() && present.is_none() {
                continue;
            }
            let mut swapchain_maintenance1_enabled = false;
            if let Some(surface) = surface {
                let extensions =
                    unsafe { instance.enumerate_device_extension_properties(physical_device) }
                        .map_err(|error| {
                            VulkanError::new(
                                "enumerate Vulkan device extensions",
                                format!("{error:?}"),
                            )
                        })?;
                let has_swapchain = extensions.iter().any(|property| unsafe {
                    CStr::from_ptr(property.extension_name.as_ptr()) == ash::khr::swapchain::NAME
                });
                if !has_swapchain {
                    let _ = surface;
                    continue;
                }
                let has_swapchain_maintenance1 = owner.surface_maintenance1_enabled
                    && extensions.iter().any(|property| unsafe {
                        CStr::from_ptr(property.extension_name.as_ptr())
                            == ash::ext::swapchain_maintenance1::NAME
                    });
                if has_swapchain_maintenance1 {
                    let mut maintenance_features =
                        vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default();
                    let mut features =
                        vk::PhysicalDeviceFeatures2::default().push_next(&mut maintenance_features);
                    unsafe {
                        properties2_loader
                            .as_ref()
                            .expect("surface maintenance requires physical-device properties2")
                            .get_physical_device_features2(physical_device, &mut features)
                    };
                    swapchain_maintenance1_enabled =
                        maintenance_features.swapchain_maintenance1 == vk::TRUE;
                }
            }
            let score = match properties.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 4,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 3,
                vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
                vk::PhysicalDeviceType::CPU => 1,
                _ => 0,
            };
            if selected.as_ref().is_none_or(|candidate: &SelectedDevice| {
                score > candidate.score
                    || (score == candidate.score
                        && swapchain_maintenance1_enabled
                        && !candidate.swapchain_maintenance1_enabled)
            }) {
                selected = Some(SelectedDevice {
                    physical_device,
                    graphics_family: graphics as u32,
                    present_family: present.map(|index| index as u32),
                    properties,
                    score,
                    swapchain_maintenance1_enabled,
                });
            }
        }
        let selected = selected.ok_or_else(|| {
            VulkanError::new(
                "select Vulkan device",
                if surface.is_some() {
                    "no device supports graphics, presentation, and VK_KHR_swapchain"
                } else {
                    "no Vulkan graphics device is available"
                },
            )
        })?;
        let mut unique_families = vec![selected.graphics_family];
        if let Some(present) = selected.present_family
            && !unique_families.contains(&present)
        {
            unique_families.push(present);
        }
        let priorities = [1.0_f32];
        let queue_infos: Vec<_> = unique_families
            .iter()
            .map(|family| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(*family)
                    .queue_priorities(&priorities)
            })
            .collect();
        let available_features =
            unsafe { instance.get_physical_device_features(selected.physical_device) };
        let enabled_features = vk::PhysicalDeviceFeatures::default()
            .sampler_anisotropy(available_features.sampler_anisotropy == vk::TRUE)
            .depth_clamp(available_features.depth_clamp == vk::TRUE)
            .fill_mode_non_solid(available_features.fill_mode_non_solid == vk::TRUE);
        let mut extension_names = if surface.is_some() {
            vec![ash::khr::swapchain::NAME.as_ptr()]
        } else {
            Vec::new()
        };
        if selected.swapchain_maintenance1_enabled {
            extension_names.push(ash::ext::swapchain_maintenance1::NAME.as_ptr());
        }
        let mut maintenance_features =
            vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default()
                .swapchain_maintenance1(selected.swapchain_maintenance1_enabled);
        let mut device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&extension_names)
            .enabled_features(&enabled_features);
        if selected.swapchain_maintenance1_enabled {
            device_info = device_info.push_next(&mut maintenance_features);
        }
        let device =
            unsafe { instance.create_device(selected.physical_device, &device_info, None) }
                .map_err(|error| VulkanError::new("create Vulkan device", format!("{error:?}")))?;
        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(selected.physical_device) };
        Ok((
            Arc::new(Self {
                _owner: owner,
                device,
                physical_device: selected.physical_device,
                properties: selected.properties,
                memory_properties,
                enabled_features,
            }),
            selected.graphics_family,
            selected.present_family,
        ))
    }
}

struct SelectedDevice {
    physical_device: vk::PhysicalDevice,
    graphics_family: u32,
    present_family: Option<u32>,
    properties: vk::PhysicalDeviceProperties,
    score: i32,
    swapchain_maintenance1_enabled: bool,
}

impl Drop for VulkanCore {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
        }
    }
}

impl Drop for InstanceOwner {
    fn drop(&mut self) {
        unsafe {
            if let Some(debug_utils) = &self.debug_utils {
                if self.debug_messenger != vk::DebugUtilsMessengerEXT::null() {
                    debug_utils.destroy_debug_utils_messenger(self.debug_messenger, None);
                }
            }
            self.instance.destroy_instance(None);
        }
    }
}

impl VulkanShared {
    fn new(core: Arc<VulkanCore>, graphics_family: u32, present_family: Option<u32>) -> Arc<Self> {
        let graphics_queue = unsafe { core.device.get_device_queue(graphics_family, 0) };
        let present_queue =
            present_family.map(|family| unsafe { core.device.get_device_queue(family, 0) });
        let allocator = MemoryAllocator::new(core.clone());
        Arc::new(Self {
            core,
            graphics_queue,
            present_queue,
            graphics_family,
            present_family,
            allocator,
            queue_lock: Mutex::new(()),
            present_lock: Mutex::new(()),
            in_flight: Mutex::new(Vec::new()),
        })
    }
}

fn create_instance_owner(
    required_extensions: &[*const std::ffi::c_char],
    enable_windowed_present_scaling: bool,
) -> Result<Arc<InstanceOwner>, VulkanError> {
    let entry = unsafe { ash::Entry::load() }
        .map_err(|error| VulkanError::new("load Vulkan loader", error.to_string()))?;
    let validation_requested =
        cfg!(debug_assertions) || std::env::var_os("AIMER_CUPID_VULKAN_VALIDATION").is_some();
    let validation_layer_name = c"VK_LAYER_KHRONOS_validation";
    let available_layers = unsafe { entry.enumerate_instance_layer_properties() }
        .map_err(|error| VulkanError::new("enumerate Vulkan layers", format!("{error:?}")))?;
    let validation_available = available_layers
        .iter()
        .any(|layer| unsafe { CStr::from_ptr(layer.layer_name.as_ptr()) == validation_layer_name });
    if std::env::var_os("AIMER_CUPID_VULKAN_VALIDATION").is_some() && !validation_available {
        return Err(VulkanError::new(
            "enable Vulkan validation",
            "AIMER_CUPID_VULKAN_VALIDATION is set but VK_LAYER_KHRONOS_validation is unavailable",
        ));
    }
    let available_extensions = unsafe { entry.enumerate_instance_extension_properties(None) }
        .map_err(|error| {
            VulkanError::new("enumerate Vulkan instance extensions", format!("{error:?}"))
        })?;
    let has_instance_extension = |name: &CStr| {
        available_extensions
            .iter()
            .any(|extension| unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) == name })
    };
    let debug_utils_available = available_extensions.iter().any(|extension| unsafe {
        CStr::from_ptr(extension.extension_name.as_ptr()) == ash::ext::debug_utils::NAME
    });
    let enable_validation = validation_requested && validation_available && debug_utils_available;
    let surface_maintenance1_enabled = enable_windowed_present_scaling
        && has_instance_extension(ash::khr::get_physical_device_properties2::NAME)
        && has_instance_extension(ash::khr::get_surface_capabilities2::NAME)
        && has_instance_extension(ash::ext::surface_maintenance1::NAME);
    if std::env::var_os("AIMER_CUPID_VULKAN_VALIDATION").is_some() && !debug_utils_available {
        return Err(VulkanError::new(
            "enable Vulkan validation",
            "VK_EXT_debug_utils is unavailable",
        ));
    }
    let mut extensions = required_extensions.to_vec();
    if enable_validation && !extensions.contains(&ash::ext::debug_utils::NAME.as_ptr()) {
        extensions.push(ash::ext::debug_utils::NAME.as_ptr());
    }
    if surface_maintenance1_enabled {
        extensions.push(ash::khr::get_physical_device_properties2::NAME.as_ptr());
        extensions.push(ash::khr::get_surface_capabilities2::NAME.as_ptr());
        extensions.push(ash::ext::surface_maintenance1::NAME.as_ptr());
    }
    let layers = if enable_validation {
        vec![validation_layer_name.as_ptr()]
    } else {
        Vec::new()
    };
    let app_name = CString::new("Cupid").expect("static app name has no NUL");
    let app_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_0);
    let mut debug_create = vk::DebugUtilsMessengerCreateInfoEXT::default()
        .message_severity(
            vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
        )
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .pfn_user_callback(Some(validation_callback));
    let mut instance_info = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_extension_names(&extensions)
        .enabled_layer_names(&layers);
    if enable_validation {
        instance_info = instance_info.push_next(&mut debug_create);
    }
    let instance = unsafe { entry.create_instance(&instance_info, None) }
        .map_err(|error| VulkanError::new("create Vulkan instance", format!("{error:?}")))?;
    let debug_utils =
        enable_validation.then(|| ash::ext::debug_utils::Instance::new(&entry, &instance));
    let debug_messenger = if let Some(debug_utils) = &debug_utils {
        match unsafe { debug_utils.create_debug_utils_messenger(&debug_create, None) } {
            Ok(messenger) => messenger,
            Err(error) => {
                unsafe { instance.destroy_instance(None) };
                return Err(VulkanError::new(
                    "create Vulkan validation messenger",
                    format!("{error:?}"),
                ));
            }
        }
    } else {
        vk::DebugUtilsMessengerEXT::null()
    };
    Ok(Arc::new(InstanceOwner {
        entry,
        instance,
        debug_utils,
        debug_messenger,
        surface_maintenance1_enabled,
    }))
}

unsafe extern "system" fn validation_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user_data: *mut c_void,
) -> vk::Bool32 {
    let message = if callback_data.is_null() || unsafe { (*callback_data).p_message.is_null() } {
        "(Vulkan validation message without text)".to_string()
    } else {
        unsafe { CStr::from_ptr((*callback_data).p_message) }
            .to_string_lossy()
            .into_owned()
    };
    eprintln!("[Vulkan {severity:?} {message_type:?}] {message}");
    vk::FALSE
}

fn buffer_usage_flags(usages: &[BufferUsage]) -> vk::BufferUsageFlags {
    usages
        .iter()
        .fold(vk::BufferUsageFlags::empty(), |flags, usage| {
            flags
                | match usage {
                    BufferUsage::Vertex => vk::BufferUsageFlags::VERTEX_BUFFER,
                    BufferUsage::Index => vk::BufferUsageFlags::INDEX_BUFFER,
                    BufferUsage::Uniform => vk::BufferUsageFlags::UNIFORM_BUFFER,
                    BufferUsage::Storage | BufferUsage::ReadOnlyStorage => {
                        vk::BufferUsageFlags::STORAGE_BUFFER
                    }
                    BufferUsage::Indirect => vk::BufferUsageFlags::INDIRECT_BUFFER,
                    BufferUsage::CopySrc => vk::BufferUsageFlags::TRANSFER_SRC,
                    BufferUsage::CopyDst => vk::BufferUsageFlags::TRANSFER_DST,
                    BufferUsage::MapRead | BufferUsage::MapWrite => vk::BufferUsageFlags::empty(),
                }
        })
}

fn texture_usage_flags(usages: &[TextureUsage], format: vk::Format) -> vk::ImageUsageFlags {
    usages
        .iter()
        .fold(vk::ImageUsageFlags::empty(), |flags, usage| {
            flags
                | match usage {
                    TextureUsage::TextureBinding => vk::ImageUsageFlags::SAMPLED,
                    TextureUsage::StorageBinding => vk::ImageUsageFlags::STORAGE,
                    TextureUsage::RenderAttachment => {
                        if image_aspect(format)
                            .intersects(vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL)
                        {
                            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT
                        } else {
                            vk::ImageUsageFlags::COLOR_ATTACHMENT
                        }
                    }
                    TextureUsage::CopySrc => vk::ImageUsageFlags::TRANSFER_SRC,
                    TextureUsage::CopyDst => vk::ImageUsageFlags::TRANSFER_DST,
                }
        })
}

fn sample_count(count: u32) -> vk::SampleCountFlags {
    match count {
        1 => vk::SampleCountFlags::TYPE_1,
        2 => vk::SampleCountFlags::TYPE_2,
        4 => vk::SampleCountFlags::TYPE_4,
        8 => vk::SampleCountFlags::TYPE_8,
        16 => vk::SampleCountFlags::TYPE_16,
        32 => vk::SampleCountFlags::TYPE_32,
        64 => vk::SampleCountFlags::TYPE_64,
        _ => panic!("unsupported Vulkan sample count {count}"),
    }
}

fn image_aspect(format: vk::Format) -> vk::ImageAspectFlags {
    match format {
        vk::Format::D16_UNORM | vk::Format::X8_D24_UNORM_PACK32 | vk::Format::D32_SFLOAT => {
            vk::ImageAspectFlags::DEPTH
        }
        vk::Format::S8_UINT => vk::ImageAspectFlags::STENCIL,
        vk::Format::D16_UNORM_S8_UINT
        | vk::Format::D24_UNORM_S8_UINT
        | vk::Format::D32_SFLOAT_S8_UINT => {
            vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
        }
        _ => vk::ImageAspectFlags::COLOR,
    }
}
