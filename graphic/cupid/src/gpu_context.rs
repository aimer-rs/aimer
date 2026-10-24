#[cfg(target_arch = "wasm32")]
use aimer_utils::warn;
use aimer_utils::{SyncFuture, debug, error, info};
use wgpu::{
    Device, Instance, Limits, Queue, Surface, SurfaceColorSpace, SurfaceConfiguration,
    SurfaceTexture, TextureFormat,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

/// Shrink `size` to fit within `max_dimension` on both axes, preserving the
/// aspect ratio.
///
/// The swap chain cannot be configured larger than the adapter's maximum 2D
/// texture dimension, so an oversized window is rendered to a proportionally
/// smaller surface instead of failing. Sizes that already fit are returned
/// unchanged.
///
/// This is the single source of truth for the backing size: a caller that keeps
/// its own copy of the surface size — for example because the [`GpuContext`]
/// itself has been moved to a raster thread — must run the requested size
/// through this function to stay in agreement with [`GpuContext::resize`].
///
/// # Examples
///
/// ```
/// use aimer_cupid::gpu_context::render_dimensions;
/// use winit::dpi::PhysicalSize;
///
/// assert_eq!(
///     render_dimensions(PhysicalSize::new(3072, 1728), 2048),
///     PhysicalSize::new(2048, 1152)
/// );
/// assert_eq!(
///     render_dimensions(PhysicalSize::new(800, 600), 2048),
///     PhysicalSize::new(800, 600)
/// );
/// ```
pub fn render_dimensions(size: PhysicalSize<u32>, max_dimension: u32) -> PhysicalSize<u32> {
    if size.width <= max_dimension && size.height <= max_dimension {
        return size;
    }

    if size.width >= size.height {
        PhysicalSize::new(
            max_dimension,
            ((size.height as u64 * max_dimension as u64) / size.width as u64).max(1) as u32,
        )
    } else {
        PhysicalSize::new(
            ((size.width as u64 * max_dimension as u64) / size.height as u64).max(1) as u32,
            max_dimension,
        )
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn wasm_gpu_backends() -> [wgpu::Backends; 2] {
    [wgpu::Backends::BROWSER_WEBGPU, wgpu::Backends::GL]
}

#[cfg(any(target_arch = "wasm32", test))]
fn browser_required_limits(adapter_limits: Limits) -> Limits {
    Limits::downlevel_webgl2_defaults().using_resolution(adapter_limits)
}

async fn create_gpu<'w>(
    window: impl Into<wgpu::SurfaceTarget<'w>>,
    size: PhysicalSize<u32>,
    backends: wgpu::Backends,
) -> Result<(Device, Queue, Surface<'w>, wgpu::Adapter), String> {
    #[cfg(not(target_os = "android"))]
    let _ = size;
    debug!("GPU backends: {:?}", backends);

    let instance = Instance::new(wgpu::InstanceDescriptor {
        backends,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });

    debug!("GPU instance: {:?}", instance);

    let surface = instance
        .create_surface(window)
        .map_err(|err| format!("failed to create surface: {err}"))?;

    debug!("Surface: {:?}", surface);

    let adapter = match instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: true,
        })
        .await
    {
        Ok(adapter) => adapter,
        Err(first_err) => instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: true,
                apply_limit_buckets: true,
            })
            .await
            .map_err(|second_err| {
                format!(
                    "failed to find a suitable adapter: {first_err}; fallback adapter: {second_err}"
                )
            })?,
    };

    info!("Creating the gpu device");

    #[cfg(target_os = "android")]
    let resolution = Limits {
        max_texture_dimension_1d: size.width,
        max_texture_dimension_2d: size.height,
        max_texture_dimension_3d: 256,
        ..Limits::default()
    };

    #[cfg(target_os = "android")]
    let limit = Limits::downlevel_webgl2_defaults().using_resolution(resolution);
    #[cfg(target_os = "ios")]
    let limit = adapter.limits();
    #[cfg(not(any(target_os = "android", target_os = "ios", target_arch = "wasm32")))]
    let limit = Limits::default();
    #[cfg(target_arch = "wasm32")]
    let limit = browser_required_limits(adapter.limits());

    let mut features = wgpu::Features::default();
    if adapter.features().contains(wgpu::Features::PIPELINE_CACHE) {
        features |= wgpu::Features::PIPELINE_CACHE;
    }

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("cupid gpu renderer device"),
            required_features: features,
            required_limits: limit,
            ..Default::default()
        })
        .await
        .map_err(|err| format!("failed to create device: {err}"))?;

    Ok((device, queue, surface, adapter))
}

pub struct GpuContext<'w> {
    pub device: Device,
    pub queue: Queue,
    pub surface: Surface<'w>,
    pub config: SurfaceConfiguration,
    pub format: TextureFormat,
    pub is_srgb: bool,
    viewport_size: PhysicalSize<u32>,
}

/// Headless / offscreen GPU pair used when a windowed [`GpuContext`] is not
/// needed. Available with the `wgpu` feature so tests and non-surface
/// consumers can build a [`crate::backend::wgpu::WgpuBackend`] without going
/// through winit.
pub struct GpuDevice {
    pub device: Device,
    pub queue: Queue,
}

impl GpuDevice {
    /// Wraps an existing device/queue pair.
    #[inline]
    pub fn new(device: Device, queue: Queue) -> Self {
        Self { device, queue }
    }

    /// Builds a [`crate::backend::wgpu::WgpuBackend`] over this device/queue.
    ///
    /// `Device` and `Queue` are cheap to clone (they are reference-counted
    /// handles), so this does not transfer ownership away from `self`.
    #[inline]
    pub fn backend(&self) -> crate::backend::wgpu::WgpuBackend {
        crate::backend::wgpu::WgpuBackend::new(self.device.clone(), self.queue.clone())
    }

    /// Borrows the underlying wgpu device.
    #[inline]
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Borrows the underlying wgpu queue.
    #[inline]
    pub fn queue(&self) -> &Queue {
        &self.queue
    }
}

impl<'w> GpuContext<'w> {
    /// Synchronous initializer for non-wasm targets.
    #[cfg(not(target_arch = "wasm32"))]
    ///
    /// `window` is anything wgpu can make a surface of: a borrowed window, or an
    /// owning one such as `Arc<Window>`, which keeps the window alive for as
    /// long as the context uses its surface and frees it with the last owner.
    pub fn initialize<W>(window: W, size: PhysicalSize<u32>) -> Self
    where
        W: Into<wgpu::SurfaceTarget<'w>> + Clone,
    {
        Self::initialize_async(window, size).block()
    }

    /// Async initializer usable on all targets (required on wasm where blocking
    /// is not allowed).
    pub async fn initialize_async<W>(window: W, size: PhysicalSize<u32>) -> Self
    where
        W: Into<wgpu::SurfaceTarget<'w>> + Clone,
    {
        #[cfg(target_arch = "wasm32")]
        let (device, queue, surface, adapter) = {
            let mut failures = Vec::new();
            let mut initialized = None;
            for backend in wasm_gpu_backends() {
                match create_gpu(window.clone(), size, backend).await {
                    Ok(gpu) => {
                        initialized = Some(gpu);
                        break;
                    }
                    Err(err) => {
                        warn!(
                            "GPU initialization with {:?} failed, Trying other Backend",
                            backend
                        );
                        failures.push(format!("{backend:?}: {err}"));
                    }
                }
            }

            initialized.unwrap_or_else(|| {
                panic!(
                    "Failed to initialize WebGPU or WebGL: {}",
                    failures.join("; ")
                )
            })
        };

        #[cfg(not(target_arch = "wasm32"))]
        let backends = {
            #[cfg(target_os = "android")]
            {
                // wgpu::Backends::GL | wgpu::Backends::VULKAN
                wgpu::Backends::GL
            }
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            {
                wgpu::Backends::METAL
            }
            #[cfg(target_os = "windows")]
            {
                wgpu::Backends::DX12
            }
            #[cfg(target_os = "linux")]
            {
                wgpu::Backends::VULKAN
            }
        };

        #[cfg(not(target_arch = "wasm32"))]
        let (device, queue, surface, adapter) = create_gpu(window, size, backends)
            .await
            .unwrap_or_else(|err| {
                error!("Failed to initialize GPU: {}", err);
                std::process::exit(1);
            });

        debug!(
            "Current Activated Backend: {:?}",
            adapter.get_info().backend
        );

        let caps = surface.get_capabilities(&adapter);

        let selected_format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let is_srgb = selected_format.is_srgb();

        let max_dim = device.limits().max_texture_dimension_2d;

        let present_mode = wgpu::PresentMode::Fifo;

        let backing_size = render_dimensions(size, max_dim);
        let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            usage |= wgpu::TextureUsages::COPY_SRC;
        }
        let config = SurfaceConfiguration {
            usage,
            format: selected_format,
            width: backing_size.width.max(1),
            height: backing_size.height.max(1),
            present_mode,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);

        Self {
            device,
            queue,
            surface,
            config,
            format: selected_format,
            is_srgb,
            viewport_size: size,
        }
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width > 0 && size.height > 0 {
            let max_dim = self.device.limits().max_texture_dimension_2d;
            let backing_size = render_dimensions(size, max_dim);
            self.config.width = backing_size.width;
            self.config.height = backing_size.height;
            self.viewport_size = size;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// The adapter's maximum 2D texture dimension, which caps the surface size.
    ///
    /// Paired with [`render_dimensions`] this lets a caller predict the backing
    /// size a [`resize`] would produce without holding the context — needed when
    /// the context lives on a raster thread and the UI thread still has to build
    /// frames for the right dimensions.
    ///
    /// [`resize`]: GpuContext::resize
    #[inline]
    pub fn max_texture_dimension(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    pub fn width(&self) -> u32 {
        self.config.width
    }

    pub fn height(&self) -> u32 {
        self.config.height
    }

    pub fn begin_frame(&self) -> wgpu::CurrentSurfaceTexture {
        self.surface.get_current_texture()
    }

    pub fn end_frame(&self, frame: SurfaceTexture) {
        // wgpu 30: presentation moved from `SurfaceTexture::present()` to
        // `Queue::present()`.
        self.queue.present(frame);
    }

    /// Builds a [`crate::backend::wgpu::WgpuBackend`] over this context's
    /// device and queue.
    ///
    /// Use with [`crate::renderer::Renderer::new`]. `Device` and `Queue` are
    /// reference-counted, so cloning them into the backend is cheap and does
    /// not invalidate the context.
    #[inline]
    pub fn backend(&self) -> crate::backend::wgpu::WgpuBackend {
        crate::backend::wgpu::WgpuBackend::new(self.device.clone(), self.queue.clone())
    }

    /// Returns a headless [`GpuDevice`] view of this context's device/queue
    /// pair (surface is not included).
    #[inline]
    pub fn device_pair(&self) -> GpuDevice {
        GpuDevice::new(self.device.clone(), self.queue.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_gpu_backends_try_webgpu_before_webgl() {
        assert_eq!(
            wasm_gpu_backends(),
            [wgpu::Backends::BROWSER_WEBGPU, wgpu::Backends::GL]
        );
    }

    #[test]
    fn browser_limits_preserve_the_adapter_texture_resolution() {
        let adapter_limits = Limits {
            max_texture_dimension_1d: 8192,
            max_texture_dimension_2d: 8192,
            max_texture_dimension_3d: 2048,
            max_texture_array_layers: 512,
            ..Limits::downlevel_webgl2_defaults()
        };

        let limits = browser_required_limits(adapter_limits);

        assert_eq!(limits.max_texture_dimension_1d, 8192);
        assert_eq!(limits.max_texture_dimension_2d, 8192);
        assert_eq!(limits.max_texture_dimension_3d, 2048);
        assert_eq!(limits.max_texture_array_layers, 256);
    }

    #[test]
    fn surface_size_preserves_aspect_ratio_when_width_exceeds_limit() {
        assert_eq!(
            render_dimensions(PhysicalSize::new(3072, 1728), 2048),
            PhysicalSize::new(2048, 1152)
        );
    }

    #[test]
    fn surface_size_preserves_aspect_ratio_when_height_exceeds_limit() {
        assert_eq!(
            render_dimensions(PhysicalSize::new(1728, 3072), 2048),
            PhysicalSize::new(1152, 2048)
        );
    }

    #[test]
    fn surface_size_keeps_dimensions_within_limit() {
        assert_eq!(
            render_dimensions(PhysicalSize::new(2048, 1024), 2048),
            PhysicalSize::new(2048, 1024)
        );
    }

    #[test]
    fn render_dimensions_use_the_limited_surface_size() {
        assert_eq!(
            render_dimensions(PhysicalSize::new(2294, 1290), 2048),
            PhysicalSize::new(2048, 1151)
        );
    }
}
