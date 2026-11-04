//! Direct browser WebGPU backend implemented with `web-sys`.
//!
//! The crate is empty on every target but wasm32; `aimer_cupid` selects it with
//! its `web` feature.

#![cfg(target_arch = "wasm32")]

// `crate::backend::..` is how this code has always named the contract.
pub(crate) use aimer_cupid_gpu as backend;

mod pipeline;
mod trait_impl;
mod util;
#[cfg(test)]
mod video_tests;

pub use pipeline::{
    WebGpuBindGroup, WebGpuBindGroupLayout, WebGpuBuffer, WebGpuCommandEncoder,
    WebGpuPipelineLayout, WebGpuRenderPass, WebGpuRenderPipeline, WebGpuSampler,
    WebGpuShaderModule, WebGpuTexture, WebGpuTextureView,
};

use std::rc::Rc;

use js_sys::{Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Gpu, GpuAdapter, GpuCanvasAlphaMode, GpuCanvasConfiguration, GpuCanvasContext, GpuDevice,
    GpuQueue, GpuTextureFormat, HtmlCanvasElement,
};

use crate::backend::GpuLimits;

const GPU_MAP_MODE_READ: u32 = 0x1;

/// An error returned while requesting a browser WebGPU adapter or creating its device.
#[derive(Clone, Debug)]
pub struct WebGpuError {
    operation: &'static str,
    message: String,
    can_fallback_to_webgl: bool,
}

impl WebGpuError {
    fn new(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
            can_fallback_to_webgl: true,
        }
    }

    fn terminal(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
            can_fallback_to_webgl: false,
        }
    }

    /// Whether initialization failed before this canvas acquired a WebGPU context.
    #[inline]
    pub fn can_fallback_to_webgl(&self) -> bool {
        self.can_fallback_to_webgl
    }
}

impl std::fmt::Display for WebGpuError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} failed: {}", self.operation, self.message)
    }
}

impl std::error::Error for WebGpuError {}

/// A direct browser WebGPU implementation of Cupid's generic GPU backend.
pub struct WebGpuBackend {
    pub(crate) shared: Rc<WebGpuShared>,
    /// The texture each video element is copied into, by registration id.
    videos: std::cell::RefCell<std::collections::HashMap<u32, VideoTexture>>,
}

/// A video element's texture and the picture last copied into it.
struct VideoTexture {
    texture: WebGpuTexture,
    size: (u32, u32),
    serial: u64,
}

pub(crate) struct WebGpuShared {
    pub(crate) device: GpuDevice,
    pub(crate) queue: GpuQueue,
    pub(crate) context: GpuCanvasContext,
    pub(crate) canvas: HtmlCanvasElement,
    pub(crate) format: WebGpuTextureFormat,
    pub(crate) limits: GpuLimits,
}

/// A texture format understood by this browser WebGPU backend.
pub type WebGpuTextureFormat = GpuTextureFormat;

impl WebGpuBackend {
    /// A view of the picture a registered `<video>` element shows as
    /// `serial`, copying it into the element's texture first when that is not
    /// the picture already there.
    ///
    /// The copy is queued on the GPU (`copyExternalImageToTexture`), so the
    /// pixels never reach the CPU. WebGPU's zero-copy `importExternalTexture`
    /// yields a texture type the image pipeline cannot sample, which is why
    /// this is a GPU-side copy. Returns `None` when `id` names no video
    /// element, the picture has no size yet, or the browser refuses the copy
    /// (a video the page may not read, for example).
    pub(crate) fn import_video(
        &self,
        id: u32,
        serial: u64,
        width: u32,
        height: u32,
    ) -> Option<WebGpuTextureView> {
        use crate::backend::GpuBackend;
        use crate::util::{dictionary, object, set, set_bool, set_ref, set_str, set_u32};

        if width == 0 || height == 0 {
            return None;
        }
        let element = aimer_cupid_gpu::web_video::lookup(id)?
            .downcast::<web_sys::HtmlVideoElement>()
            .ok()?;
        let mut videos = self.videos.borrow_mut();
        // Registrations that ended: their textures are only held here.
        if videos.len() > 1 {
            videos.retain(|id, _| aimer_cupid_gpu::web_video::lookup(*id).is_some());
        }
        let size = (width, height);
        if videos.get(&id).is_none_or(|video| video.size != size) {
            let texture = self.create_texture(&crate::backend::TextureDescriptor {
                label: Some("video".into()),
                size: (width, height, 1),
                mip_level_count: 1,
                sample_count: 1,
                dimension: crate::backend::TextureDimension::D2,
                format: GpuTextureFormat::Rgba8unorm,
                // `copyExternalImageToTexture` needs the destination to be a
                // render attachment as well as a copy target.
                usage: vec![
                    crate::backend::TextureUsage::TextureBinding,
                    crate::backend::TextureUsage::CopyDst,
                    crate::backend::TextureUsage::RenderAttachment,
                    // Lets a screenshot or a test read the picture back.
                    crate::backend::TextureUsage::CopySrc,
                ],
            });
            videos.insert(id, VideoTexture { texture, size, serial: u64::MAX });
        }
        let video = videos.get_mut(&id)?;
        if video.serial != serial {
            let source = object();
            set_ref(&source, "source", &*element);
            set_bool(&source, "flipY", false);
            let destination = object();
            set_ref(&destination, "texture", &video.texture.0.raw);
            set_bool(&destination, "premultipliedAlpha", false);
            set_str(&destination, "colorSpace", "srgb");
            let extent = dictionary::<js_sys::Object>();
            set_u32(&extent, "width", width);
            set_u32(&extent, "height", height);
            set(&extent, "depthOrArrayLayers", &JsValue::from_f64(1.0));
            let queue: &JsValue = self.shared.queue.as_ref();
            let method = Reflect::get(queue, &JsValue::from_str("copyExternalImageToTexture")).ok()?;
            let method: js_sys::Function = method.dyn_into().ok()?;
            method
                .call3(queue, source.as_ref(), destination.as_ref(), extent.as_ref())
                .ok()?;
            video.serial = serial;
        }
        Some(self.create_texture_view(&video.texture, "video"))
    }

    /// Requests a browser WebGPU adapter and device, then configures the canvas.
    pub async fn new(canvas: HtmlCanvasElement) -> Result<Self, WebGpuError> {
        let navigator = web_sys::window()
            .ok_or_else(|| WebGpuError::new("get browser window", "window is unavailable"))?
            .navigator();
        let gpu_value = Reflect::get(navigator.as_ref(), &JsValue::from_str("gpu"))
            .map_err(|error| WebGpuError::new("get navigator.gpu", js_error(error)))?;
        if gpu_value.is_null_or_undefined() {
            return Err(WebGpuError::new(
                "get navigator.gpu",
                "this browser does not expose the WebGPU API",
            ));
        }
        let gpu = gpu_value
            .dyn_into::<Gpu>()
            .map_err(|error| WebGpuError::new("cast navigator.gpu", js_error(error)))?;

        let adapter_promise: Promise = gpu.request_adapter().unchecked_into();
        let adapter_value = JsFuture::from(adapter_promise)
            .await
            .map_err(|error| WebGpuError::new("request WebGPU adapter", js_error(error)))?;
        if adapter_value.is_null_or_undefined() {
            return Err(WebGpuError::new(
                "request WebGPU adapter",
                "the browser returned no compatible adapter",
            ));
        }
        let adapter = adapter_value
            .dyn_into::<GpuAdapter>()
            .map_err(|error| WebGpuError::new("cast WebGPU adapter", js_error(error)))?;

        let device_promise: Promise = adapter.request_device().unchecked_into();
        let device_value = JsFuture::from(device_promise)
            .await
            .map_err(|error| WebGpuError::new("request WebGPU device", js_error(error)))?;
        let device = device_value
            .dyn_into::<GpuDevice>()
            .map_err(|error| WebGpuError::new("cast WebGPU device", js_error(error)))?;
        let queue = device.queue();
        let format = gpu.get_preferred_canvas_format();

        let context = canvas
            .get_context("webgpu")
            .map_err(|error| WebGpuError::new("get WebGPU canvas context", js_error(error)))?
            .ok_or_else(|| {
                WebGpuError::new("get WebGPU canvas context", "canvas returned no context")
            })?
            .dyn_into::<GpuCanvasContext>()
            .map_err(|error| {
                WebGpuError::terminal("cast WebGPU canvas context", js_error(error.into()))
            })?;
        let configuration = GpuCanvasConfiguration::new(&device, format);
        configuration.set_alpha_mode(GpuCanvasAlphaMode::Premultiplied);
        context
            .configure(&configuration)
            .map_err(|error| WebGpuError::terminal("configure WebGPU canvas", js_error(error)))?;

        let limits = device.limits();
        let shared = Rc::new(WebGpuShared {
            device,
            queue,
            context,
            canvas,
            format,
            limits: GpuLimits {
                max_texture_dimension_2d: limits.max_texture_dimension_2d(),
                min_uniform_buffer_offset_alignment: limits.min_uniform_buffer_offset_alignment(),
            },
        });

        Ok(Self {
            shared,
            videos: Default::default(),
        })
    }

    /// Returns a view of the canvas drawing buffer for rendering and presentation.
    #[inline]
    pub fn surface_view(&self) -> WebGpuTextureView {
        WebGpuTextureView::surface(self.shared.clone())
    }

    /// Returns the format selected by the browser for the canvas surface.
    #[inline]
    pub fn format(&self) -> WebGpuTextureFormat {
        self.shared.format
    }

    /// Returns whether the configured canvas format is sRGB encoded.
    #[inline]
    pub fn is_srgb(&self) -> bool {
        <Self as crate::backend::GpuBackend>::format_is_srgb(self.shared.format)
    }

    /// Maps a `MAP_READ` buffer after prior GPU work completes and copies its
    /// first `size` bytes to host memory.
    pub async fn read_buffer_async(
        &self,
        buffer: &WebGpuBuffer,
        size: u64,
    ) -> Result<Vec<u8>, WebGpuError> {
        if size > buffer.size {
            return Err(WebGpuError::new(
                "read WebGPU buffer",
                format!("requested {size} bytes from a {} byte buffer", buffer.size),
            ));
        }
        if size == 0 {
            return Ok(Vec::new());
        }
        if size % 4 != 0 {
            return Err(WebGpuError::new(
                "read WebGPU buffer",
                "mapped range size must be a multiple of four bytes",
            ));
        }
        let size = u32::try_from(size).map_err(|_| {
            WebGpuError::new(
                "read WebGPU buffer",
                "mapped range exceeds the wasm typed-array limit",
            )
        })?;
        let map_promise: Promise = buffer
            .raw
            .map_async_with_u32_and_u32(GPU_MAP_MODE_READ, 0, size)
            .unchecked_into();
        JsFuture::from(map_promise)
            .await
            .map_err(|error| WebGpuError::new("map WebGPU buffer", js_error(error)))?;
        let mapped = match buffer.raw.get_mapped_range_with_u32_and_u32(0, size) {
            Ok(mapped) => mapped,
            Err(error) => {
                buffer.raw.unmap();
                return Err(WebGpuError::new(
                    "read mapped WebGPU buffer",
                    js_error(error),
                ));
            }
        };
        let bytes = Uint8Array::new(&mapped).to_vec();
        buffer.raw.unmap();
        Ok(bytes)
    }

    /// Resizes the browser canvas drawing buffer in physical pixels.
    pub fn resize(&self, width: u32, height: u32) {
        self.shared.canvas.set_width(width);
        self.shared.canvas.set_height(height);
    }

    /// WebGPU presents automatically when the submitted command buffer finishes.
    #[inline]
    pub fn present(&self) {}
}

impl WebGpuShared {
    pub(crate) fn get_surface_texture(&self) -> Result<web_sys::GpuTexture, JsValue> {
        self.context.get_current_texture()
    }
}

fn js_error(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}
