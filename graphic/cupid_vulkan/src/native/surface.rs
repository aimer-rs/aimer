use std::sync::{Arc, Mutex};

use crate::backend::TextureUsage;
use crate::vulkan_surface_format::{SurfaceFormatClass, select_surface_format};
use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use super::{
    InstanceOwner, VulkanBackend, VulkanCore, VulkanError, VulkanShared, VulkanTexture,
    VulkanTextureInner, VulkanTextureView, VulkanTextureViewInner,
};

const FRAMES_IN_FLIGHT: usize = 2;

pub(super) struct VulkanSurfaceOwner {
    pub(super) owner: Arc<InstanceOwner>,
    pub(super) loader: ash::khr::surface::Instance,
    pub(super) raw: vk::SurfaceKHR,
}

impl Drop for VulkanSurfaceOwner {
    fn drop(&mut self) {
        unsafe { self.loader.destroy_surface(self.raw, None) };
    }
}

struct FrameSync {
    acquire: vk::Semaphore,
    fence: vk::Fence,
    in_flight: bool,
}

/// Swapchain-backed window surface for a native Vulkan backend.
pub struct VulkanSurface<W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static> {
    _window: Arc<W>,
    shared: Arc<VulkanShared>,
    owner: VulkanSurfaceOwner,
    swapchain_loader: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    format: vk::Format,
    extent: vk::Extent2D,
    requested_size: (u32, u32),
    needs_recreate: bool,
    images: Vec<VulkanTextureView>,
    render_finished: Vec<vk::Semaphore>,
    frames: Vec<FrameSync>,
    current_frame: usize,
}

/// A swapchain image acquired for one Cupid render frame.
pub struct VulkanSurfaceFrame<'a, W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static> {
    surface: &'a mut VulkanSurface<W>,
    image_index: u32,
    frame_index: usize,
    view: VulkanTextureView,
    completed: bool,
}

pub(super) fn create_windowed<W>(
    window: Arc<W>,
    initial_size: (u32, u32),
) -> Result<(VulkanBackend, VulkanSurface<W>), VulkanError>
where
    W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static,
{
    let display = window
        .display_handle()
        .map_err(|error| VulkanError::new("get display handle", error.to_string()))?
        .as_raw();
    let raw_window = window
        .window_handle()
        .map_err(|error| VulkanError::new("get window handle", error.to_string()))?
        .as_raw();
    let required_extensions =
        ash_window::enumerate_required_extensions(display).map_err(|error| {
            VulkanError::new("enumerate Vulkan surface extensions", format!("{error:?}"))
        })?;
    let owner = super::create_instance_owner(required_extensions, true)?;
    let surface_owner = create_surface_owner(owner.clone(), display, raw_window)?;
    let (core, graphics_family, present_family) = VulkanCore::create(owner, Some(&surface_owner))?;
    let present_family = present_family.expect("windowed device has a present queue");
    let shared = VulkanShared::new(core, graphics_family, Some(present_family));
    let surface = VulkanSurface::new(window, shared.clone(), surface_owner, initial_size)?;
    Ok((VulkanBackend::from_shared(shared), surface))
}

pub(super) fn create_for_backend<W>(
    shared: Arc<VulkanShared>,
    window: Arc<W>,
    initial_size: (u32, u32),
) -> Result<VulkanSurface<W>, VulkanError>
where
    W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static,
{
    let present_family = shared.present_family.ok_or_else(|| {
        VulkanError::new(
            "create Vulkan surface",
            "the backend was initialized headless",
        )
    })?;
    let display = window
        .display_handle()
        .map_err(|error| VulkanError::new("get display handle", error.to_string()))?
        .as_raw();
    let raw_window = window
        .window_handle()
        .map_err(|error| VulkanError::new("get window handle", error.to_string()))?
        .as_raw();
    let owner = shared.core._owner.clone();
    let surface_owner = create_surface_owner(owner, display, raw_window)?;
    let supports_present = unsafe {
        surface_owner.loader.get_physical_device_surface_support(
            shared.core.physical_device,
            present_family,
            surface_owner.raw,
        )
    }
    .map_err(|error| {
        VulkanError::new(
            "query Vulkan surface presentation support",
            format!("{error:?}"),
        )
    })?;
    if !supports_present {
        return Err(VulkanError::new(
            "create Vulkan surface",
            "the selected present queue does not support this window surface",
        ));
    }
    VulkanSurface::new(window, shared, surface_owner, initial_size)
}

fn create_surface_owner(
    owner: Arc<InstanceOwner>,
    display: raw_window_handle::RawDisplayHandle,
    window: raw_window_handle::RawWindowHandle,
) -> Result<VulkanSurfaceOwner, VulkanError> {
    let loader = ash::khr::surface::Instance::new(&owner.entry, &owner.instance);
    // SAFETY: the instance was created with the platform surface extensions,
    // and its caller keeps the corresponding window alive through this owner.
    let raw =
        unsafe { ash_window::create_surface(&owner.entry, &owner.instance, display, window, None) }
            .map_err(|error| VulkanError::new("create Vulkan surface", format!("{error:?}")))?;
    Ok(VulkanSurfaceOwner { owner, loader, raw })
}

impl<W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static> VulkanSurface<W> {
    fn new(
        window: Arc<W>,
        shared: Arc<VulkanShared>,
        owner: VulkanSurfaceOwner,
        initial_size: (u32, u32),
    ) -> Result<Self, VulkanError> {
        let swapchain_loader =
            ash::khr::swapchain::Device::new(&owner.owner.instance, &shared.core.device);
        let (swapchain, format, extent, images) = create_swapchain(
            &shared,
            &owner,
            &swapchain_loader,
            initial_size,
            vk::SwapchainKHR::null(),
        )?;
        let render_finished = match create_semaphores(&shared.core, images.len()) {
            Ok(semaphores) => semaphores,
            Err(error) => {
                drop(images);
                unsafe { swapchain_loader.destroy_swapchain(swapchain, None) };
                return Err(error);
            }
        };
        let frames = match create_frame_sync(&shared.core) {
            Ok(frames) => frames,
            Err(error) => {
                destroy_semaphores(&shared.core, render_finished);
                drop(images);
                unsafe { swapchain_loader.destroy_swapchain(swapchain, None) };
                return Err(error);
            }
        };
        Ok(Self {
            _window: window,
            shared,
            owner,
            swapchain_loader,
            swapchain,
            format,
            extent,
            requested_size: initial_size,
            needs_recreate: false,
            images,
            render_finished,
            frames,
            current_frame: 0,
        })
    }

    /// Returns the selected swapchain format.
    #[inline]
    pub fn format(&self) -> vk::Format {
        self.format
    }

    /// Returns whether the selected swapchain format is sRGB.
    #[inline]
    pub fn is_srgb(&self) -> bool {
        is_srgb_format(self.format)
    }

    /// Returns the current swapchain dimensions in physical pixels.
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        (self.extent.width, self.extent.height)
    }

    /// Returns whether the last resize request suspended swapchain acquisition.
    #[inline]
    pub fn is_suspended(&self) -> bool {
        self.requested_size.0 == 0 || self.requested_size.1 == 0
    }

    /// Requests swapchain recreation for the new physical window size.
    ///
    /// A zero-sized request suspends acquisition until a non-zero size arrives.
    /// The latest non-zero size is applied on the next available frame. Frames
    /// are rendered using the recreated swapchain's exact physical extent.
    pub fn resize(&mut self, size: (u32, u32)) {
        self.requested_size = size;
        if size.0 != 0 && size.1 != 0 {
            self.needs_recreate = true;
        } else {
            self.needs_recreate = false;
        }
    }

    /// Returns whether a queued resize still needs a redraw to rebuild.
    /// Callers should request another redraw while this returns `true`.
    #[inline]
    pub fn is_resize_pending(&self) -> bool {
        self.needs_recreate && !self.is_suspended()
    }

    /// Acquires a swapchain image. Returns `Ok(None)` while minimized, while
    /// prior frame work is still in flight during a resize, or after an
    /// out-of-date result requires a later redraw.
    pub fn try_acquire(&mut self) -> Result<Option<VulkanSurfaceFrame<'_, W>>, VulkanError> {
        if self.requested_size.0 == 0 || self.requested_size.1 == 0 {
            return Ok(None);
        }
        if self.needs_recreate {
            if !self.frame_work_complete()? {
                return Ok(None);
            }
            self.recreate()?;
        }
        let frame_index = self.current_frame;
        let frame = &mut self.frames[frame_index];
        if frame.in_flight {
            unsafe {
                self.shared
                    .core
                    .device
                    .wait_for_fences(&[frame.fence], true, u64::MAX)
                    .map_err(|error| {
                        VulkanError::new("wait for Vulkan frame", format!("{error:?}"))
                    })?;
            }
            frame.in_flight = false;
        }

        let acquired = unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                frame.acquire,
                vk::Fence::null(),
            )
        };
        let (image_index, suboptimal) = match acquired {
            Ok(acquired) => acquired,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.needs_recreate = true;
                if self.frame_work_complete()? {
                    self.recreate()?;
                }
                return Ok(None);
            }
            Err(error) => {
                return Err(VulkanError::new(
                    "acquire Vulkan swapchain image",
                    format!("{error:?}"),
                ));
            }
        };

        // This wait-only queue operation places WSI's acquire dependency before
        // every command buffer RendererImpl submits later on the graphics queue.
        let wait_semaphores = [frame.acquire];
        let wait_stages = [vk::PipelineStageFlags::ALL_COMMANDS];
        let wait = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages);
        {
            let _queue = self
                .shared
                .queue_lock
                .lock()
                .expect("Vulkan graphics queue mutex is not poisoned");
            unsafe {
                self.shared
                    .core
                    .device
                    .queue_submit(self.shared.graphics_queue, &[wait], vk::Fence::null())
                    .map_err(|error| {
                        VulkanError::new("wait for Vulkan acquire semaphore", format!("{error:?}"))
                    })?;
            }
        }
        if suboptimal {
            self.needs_recreate = true;
        }
        let view = self.images[image_index as usize].clone();
        Ok(Some(VulkanSurfaceFrame {
            surface: self,
            image_index,
            frame_index,
            view,
            completed: false,
        }))
    }

    fn frame_work_complete(&self) -> Result<bool, VulkanError> {
        // Poll before recreation so a live resize does not block the event loop
        // waiting for work submitted for the previous swapchain.
        for frame in &self.frames {
            if frame.in_flight {
                let complete = unsafe { self.shared.core.device.get_fence_status(frame.fence) }
                    .map_err(|error| {
                        VulkanError::new(
                            "check Vulkan frame before swapchain recreation",
                            format!("{error:?}"),
                        )
                    })?;
                if !complete {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn recreate(&mut self) -> Result<(), VulkanError> {
        if self.requested_size.0 == 0 || self.requested_size.1 == 0 {
            return Ok(());
        }
        // `frame_work_complete` has already waited for this surface's graphics
        // submissions. Only presentation can still reference the old swapchain,
        // so avoid stalling unrelated device work during interactive resizing.
        let present_queue = self
            .shared
            .present_queue
            .expect("windowed backend has a present queue");
        {
            let _queue = self
                .shared
                .present_lock
                .lock()
                .expect("Vulkan present queue mutex is not poisoned");
            unsafe {
                self.shared
                    .core
                    .device
                    .queue_wait_idle(present_queue)
                    .map_err(|error| {
                        VulkanError::new(
                            "wait for Vulkan presentations before swapchain recreation",
                            format!("{error:?}"),
                        )
                    })?;
            }
        }
        super::commands::collect_finished(&self.shared);
        let (new_swapchain, format, extent, images) = create_swapchain(
            &self.shared,
            &self.owner,
            &self.swapchain_loader,
            self.requested_size,
            self.swapchain,
        )?;
        let render_finished = match create_semaphores(&self.shared.core, images.len()) {
            Ok(semaphores) => semaphores,
            Err(error) => {
                drop(images);
                unsafe { self.swapchain_loader.destroy_swapchain(new_swapchain, None) };
                return Err(error);
            }
        };
        self.images = images;
        for semaphore in self.render_finished.drain(..) {
            unsafe { self.shared.core.device.destroy_semaphore(semaphore, None) };
        }
        self.render_finished = render_finished;
        unsafe {
            self.swapchain_loader
                .destroy_swapchain(self.swapchain, None)
        };
        self.swapchain = new_swapchain;
        self.format = format;
        self.extent = extent;
        self.needs_recreate = false;
        Ok(())
    }

    fn finish_frame(&mut self, frame_index: usize, image_index: u32) -> Result<(), VulkanError> {
        let signal_semaphore = self.render_finished[image_index as usize];
        let signal_semaphores = [signal_semaphore];
        let signal = vk::SubmitInfo::default().signal_semaphores(&signal_semaphores);
        let frame_fence = self.frames[frame_index].fence;
        unsafe {
            self.shared
                .core
                .device
                .reset_fences(&[frame_fence])
                .map_err(|error| {
                    VulkanError::new("reset Vulkan frame fence", format!("{error:?}"))
                })?;
        }
        {
            let _queue = self
                .shared
                .queue_lock
                .lock()
                .expect("Vulkan graphics queue mutex is not poisoned");
            unsafe {
                self.shared
                    .core
                    .device
                    .queue_submit(self.shared.graphics_queue, &[signal], frame_fence)
                    .map_err(|error| {
                        VulkanError::new("signal Vulkan present semaphore", format!("{error:?}"))
                    })?;
            }
        }
        self.frames[frame_index].in_flight = true;
        let swapchains = [self.swapchain];
        let image_indices = [image_index];
        let waits = [signal_semaphore];
        let present = vk::PresentInfoKHR::default()
            .wait_semaphores(&waits)
            .swapchains(&swapchains)
            .image_indices(&image_indices);
        let present_queue = self
            .shared
            .present_queue
            .expect("windowed backend has a present queue");
        let present_result = {
            let _queue = self
                .shared
                .present_lock
                .lock()
                .expect("Vulkan present queue mutex is not poisoned");
            unsafe { self.swapchain_loader.queue_present(present_queue, &present) }
        };
        self.current_frame = (frame_index + 1) % self.frames.len();
        match present_result {
            Ok(suboptimal) => {
                self.needs_recreate |= suboptimal;
                Ok(())
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.needs_recreate = true;
                Ok(())
            }
            Err(error) => Err(VulkanError::new(
                "present Vulkan swapchain image",
                format!("{error:?}"),
            )),
        }
    }
}

impl<'a, W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static> VulkanSurfaceFrame<'a, W> {
    /// Returns the acquired render target view.
    #[inline]
    pub fn view(&self) -> &VulkanTextureView {
        &self.view
    }

    /// Returns the acquired image dimensions.
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        self.surface.size()
    }

    /// Presents this frame after prior graphics queue submissions complete.
    pub fn present(mut self) -> Result<(), VulkanError> {
        let result = self
            .surface
            .finish_frame(self.frame_index, self.image_index);
        self.completed = true;
        result
    }
}

impl<W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static> Drop
    for VulkanSurfaceFrame<'_, W>
{
    fn drop(&mut self) {
        if !self.completed {
            let _ = self
                .surface
                .finish_frame(self.frame_index, self.image_index);
        }
    }
}

impl<W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static> Drop for VulkanSurface<W> {
    fn drop(&mut self) {
        unsafe {
            let _ = self.shared.core.device.device_wait_idle();
            self.images.clear();
            for semaphore in self.render_finished.drain(..) {
                self.shared.core.device.destroy_semaphore(semaphore, None);
            }
            for frame in self.frames.drain(..) {
                self.shared
                    .core
                    .device
                    .destroy_semaphore(frame.acquire, None);
                self.shared.core.device.destroy_fence(frame.fence, None);
            }
            self.swapchain_loader
                .destroy_swapchain(self.swapchain, None);
        }
    }
}

fn create_swapchain(
    shared: &Arc<VulkanShared>,
    surface: &VulkanSurfaceOwner,
    loader: &ash::khr::swapchain::Device,
    requested_size: (u32, u32),
    old_swapchain: vk::SwapchainKHR,
) -> Result<
    (
        vk::SwapchainKHR,
        vk::Format,
        vk::Extent2D,
        Vec<VulkanTextureView>,
    ),
    VulkanError,
> {
    let capabilities = unsafe {
        surface
            .loader
            .get_physical_device_surface_capabilities(shared.core.physical_device, surface.raw)
    }
    .map_err(|error| VulkanError::new("query Vulkan surface capabilities", format!("{error:?}")))?;
    if !capabilities
        .supported_usage_flags
        .contains(vk::ImageUsageFlags::COLOR_ATTACHMENT)
    {
        return Err(VulkanError::new(
            "create Vulkan swapchain",
            "surface images do not support color attachments",
        ));
    }
    let formats = unsafe {
        surface
            .loader
            .get_physical_device_surface_formats(shared.core.physical_device, surface.raw)
    }
    .map_err(|error| VulkanError::new("query Vulkan surface formats", format!("{error:?}")))?;
    let selected_format = select_surface_format(
        &formats,
        !cfg!(target_os = "android"),
        classify_surface_format,
    )
    .ok_or_else(|| {
        VulkanError::new(
            "select Vulkan surface format",
            "surface exposes no color formats",
        )
    })?;
    let extent = if capabilities.current_extent.width != u32::MAX {
        capabilities.current_extent
    } else {
        vk::Extent2D {
            width: requested_size.0.clamp(
                capabilities.min_image_extent.width,
                capabilities.max_image_extent.width,
            ),
            height: requested_size.1.clamp(
                capabilities.min_image_extent.height,
                capabilities.max_image_extent.height,
            ),
        }
    };
    let desired_count = capabilities
        .min_image_count
        .saturating_add(1)
        .max(FRAMES_IN_FLIGHT as u32);
    let image_count = if capabilities.max_image_count == 0 {
        desired_count
    } else {
        desired_count.min(capabilities.max_image_count)
    };
    let mut queue_families = vec![shared.graphics_family];
    if let Some(present) = shared.present_family
        && present != shared.graphics_family
    {
        queue_families.push(present);
    }
    let (sharing_mode, queue_family_indices) = if queue_families.len() > 1 {
        (vk::SharingMode::CONCURRENT, queue_families.as_slice())
    } else {
        (vk::SharingMode::EXCLUSIVE, &[][..])
    };
    let mut create_info = vk::SwapchainCreateInfoKHR::default()
        .surface(surface.raw)
        .min_image_count(image_count)
        .image_format(selected_format.format)
        .image_color_space(selected_format.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .image_sharing_mode(sharing_mode)
        .queue_family_indices(queue_family_indices)
        .pre_transform(capabilities.current_transform)
        .composite_alpha(select_composite_alpha(
            capabilities.supported_composite_alpha,
        ))
        .present_mode(vk::PresentModeKHR::FIFO)
        .clipped(true)
        .old_swapchain(old_swapchain);
    let swapchain = unsafe { loader.create_swapchain(&create_info, None) }
        .map_err(|error| VulkanError::new("create Vulkan swapchain", format!("{error:?}")))?;
    let raw_images = match unsafe { loader.get_swapchain_images(swapchain) } {
        Ok(images) => images,
        Err(error) => {
            unsafe { loader.destroy_swapchain(swapchain, None) };
            return Err(VulkanError::new(
                "get Vulkan swapchain images",
                format!("{error:?}"),
            ));
        }
    };
    let mut images = Vec::with_capacity(raw_images.len());
    for raw_image in raw_images {
        let texture = VulkanTexture(Arc::new(VulkanTextureInner {
            core: shared.core.clone(),
            raw: raw_image,
            allocation: None,
            size: (extent.width, extent.height, 1),
            format: selected_format.format,
            dimension: super::TextureDimension::D2,
            sample_count: 1,
            usage: vec![TextureUsage::RenderAttachment],
            layout: Mutex::new(vk::ImageLayout::UNDEFINED),
            presentable: true,
        }));
        let view_info = vk::ImageViewCreateInfo::default()
            .image(raw_image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(selected_format.format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let raw_view = match unsafe { shared.core.device.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(error) => {
                drop(images);
                unsafe { loader.destroy_swapchain(swapchain, None) };
                return Err(VulkanError::new(
                    "create Vulkan swapchain image view",
                    format!("{error:?}"),
                ));
            }
        };
        images.push(VulkanTextureView(Arc::new(VulkanTextureViewInner {
            core: shared.core.clone(),
            raw: raw_view,
            texture,
        })));
    }
    Ok((swapchain, selected_format.format, extent, images))
}

fn create_semaphores(core: &VulkanCore, count: usize) -> Result<Vec<vk::Semaphore>, VulkanError> {
    let mut semaphores = Vec::with_capacity(count);
    for _ in 0..count {
        match unsafe {
            core.device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
        } {
            Ok(semaphore) => semaphores.push(semaphore),
            Err(error) => {
                destroy_semaphores(core, semaphores);
                return Err(VulkanError::new(
                    "create Vulkan semaphore",
                    format!("{error:?}"),
                ));
            }
        }
    }
    Ok(semaphores)
}

fn create_frame_sync(core: &VulkanCore) -> Result<Vec<FrameSync>, VulkanError> {
    let mut frames = Vec::with_capacity(FRAMES_IN_FLIGHT);
    for _ in 0..FRAMES_IN_FLIGHT {
        let acquire = match unsafe {
            core.device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
        } {
            Ok(semaphore) => semaphore,
            Err(error) => {
                destroy_frame_sync(core, frames);
                return Err(VulkanError::new(
                    "create Vulkan acquire semaphore",
                    format!("{error:?}"),
                ));
            }
        };
        let fence = match unsafe {
            core.device.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            )
        } {
            Ok(fence) => fence,
            Err(error) => {
                unsafe { core.device.destroy_semaphore(acquire, None) };
                destroy_frame_sync(core, frames);
                return Err(VulkanError::new(
                    "create Vulkan frame fence",
                    format!("{error:?}"),
                ));
            }
        };
        frames.push(FrameSync {
            acquire,
            fence,
            in_flight: false,
        });
    }
    Ok(frames)
}

fn destroy_semaphores(core: &VulkanCore, semaphores: Vec<vk::Semaphore>) {
    unsafe {
        for semaphore in semaphores {
            core.device.destroy_semaphore(semaphore, None);
        }
    }
}

fn destroy_frame_sync(core: &VulkanCore, frames: Vec<FrameSync>) {
    unsafe {
        for frame in frames {
            core.device.destroy_semaphore(frame.acquire, None);
            core.device.destroy_fence(frame.fence, None);
        }
    }
}

fn select_composite_alpha(flags: vk::CompositeAlphaFlagsKHR) -> vk::CompositeAlphaFlagsKHR {
    [
        vk::CompositeAlphaFlagsKHR::OPAQUE,
        vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::INHERIT,
    ]
    .into_iter()
    .find(|candidate| flags.contains(*candidate))
    .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE)
}

fn classify_surface_format(format: vk::SurfaceFormatKHR) -> SurfaceFormatClass {
    if format.color_space != vk::ColorSpaceKHR::SRGB_NONLINEAR {
        return SurfaceFormatClass::Other;
    }
    match format.format {
        vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB => SurfaceFormatClass::Srgb8,
        vk::Format::B8G8R8A8_UNORM | vk::Format::R8G8B8A8_UNORM => SurfaceFormatClass::Unorm8,
        _ => SurfaceFormatClass::Other,
    }
}

pub(super) fn is_srgb_format(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::R8_SRGB
            | vk::Format::R8G8_SRGB
            | vk::Format::R8G8B8_SRGB
            | vk::Format::B8G8R8_SRGB
            | vk::Format::R8G8B8A8_SRGB
            | vk::Format::B8G8R8A8_SRGB
            | vk::Format::A8B8G8R8_SRGB_PACK32
            | vk::Format::BC1_RGB_SRGB_BLOCK
            | vk::Format::BC1_RGBA_SRGB_BLOCK
            | vk::Format::BC2_SRGB_BLOCK
            | vk::Format::BC3_SRGB_BLOCK
    )
}
