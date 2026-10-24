//! Direct browser WebGL2 backend used by Cupid's generic renderer.
//!
//! The crate is empty on every target but wasm32; `aimer_cupid` selects it with
//! its `web` feature.

#![cfg(target_arch = "wasm32")]

// `crate::backend::..` is how this code has always named the contract.
pub(crate) use aimer_cupid_gpu as backend;

mod pipeline;
mod trait_impl;

pub use pipeline::{
    WebGl2BindGroup, WebGl2BindGroupLayout, WebGl2Buffer, WebGl2CommandEncoder,
    WebGl2PipelineLayout, WebGl2RenderPass, WebGl2RenderPipeline, WebGl2Sampler,
    WebGl2ShaderModule, WebGl2Texture, WebGl2TextureFormat, WebGl2TextureView,
};

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext, WebGlFramebuffer};

use crate::backend::GpuLimits;

use self::pipeline::FramebufferKey;

/// An error returned while creating a browser WebGL2 context or resource.
#[derive(Clone, Debug)]
pub struct WebGl2Error {
    operation: &'static str,
    message: String,
}

impl WebGl2Error {
    fn new(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for WebGl2Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} failed: {}", self.operation, self.message)
    }
}

impl std::error::Error for WebGl2Error {}

/// A direct WebGL2 implementation of Cupid's generic GPU backend.
pub struct WebGl2Backend {
    pub(crate) shared: Rc<WebGl2Shared>,
}

pub(crate) struct WebGl2Shared {
    pub(crate) gl: WebGl2RenderingContext,
    pub(crate) canvas: HtmlCanvasElement,
    pub(crate) limits: GpuLimits,
    pub(crate) max_texture_units: u32,
    pub(crate) max_vertex_attributes: u32,
    pub(crate) max_uniform_buffer_bindings: u32,
    pub(crate) next_id: Cell<u32>,
    framebuffer_cache: RefCell<HashMap<FramebufferKey, WebGlFramebuffer>>,
}

impl WebGl2Backend {
    /// Creates a WebGL2 backend from an existing browser canvas.
    pub fn new(canvas: HtmlCanvasElement) -> Result<Self, WebGl2Error> {
        let context = canvas
            .get_context("webgl2")
            .map_err(|error| WebGl2Error::new("get WebGL2 context", js_error(error)))?
            .ok_or_else(|| {
                WebGl2Error::new("get WebGL2 context", "browser returned no WebGL2 context")
            })?
            .dyn_into::<WebGl2RenderingContext>()
            .map_err(|error| WebGl2Error::new("cast WebGL2 context", js_error(error.into())))?;

        let max_texture_dimension_2d =
            parameter_u32(&context, WebGl2RenderingContext::MAX_TEXTURE_SIZE, 2048);
        let min_uniform_buffer_offset_alignment = parameter_u32(
            &context,
            WebGl2RenderingContext::UNIFORM_BUFFER_OFFSET_ALIGNMENT,
            256,
        );
        let shared = Rc::new(WebGl2Shared {
            max_texture_units: parameter_u32(
                &context,
                WebGl2RenderingContext::MAX_COMBINED_TEXTURE_IMAGE_UNITS,
                32,
            ),
            max_vertex_attributes: parameter_u32(
                &context,
                WebGl2RenderingContext::MAX_VERTEX_ATTRIBS,
                16,
            ),
            max_uniform_buffer_bindings: parameter_u32(
                &context,
                WebGl2RenderingContext::MAX_UNIFORM_BUFFER_BINDINGS,
                24,
            ),
            gl: context,
            canvas,
            limits: GpuLimits {
                max_texture_dimension_2d,
                min_uniform_buffer_offset_alignment,
            },
            next_id: Cell::new(1),
            framebuffer_cache: RefCell::new(HashMap::new()),
        });
        Ok(Self { shared })
    }

    /// Returns a view of the canvas drawing buffer for rendering and presentation.
    #[inline]
    pub fn surface_view(&self) -> WebGl2TextureView {
        WebGl2TextureView::surface(self.shared.clone())
    }

    /// Resizes the browser drawing buffer to physical pixel dimensions.
    pub fn resize(&self, width: u32, height: u32) {
        self.shared.canvas.set_width(width);
        self.shared.canvas.set_height(height);
    }

    /// Flushes queued WebGL commands so the browser can present the canvas.
    #[inline]
    pub fn present(&self) {
        self.shared.gl.flush();
    }
}

impl WebGl2Shared {
    pub(crate) fn allocate_id(&self) -> u32 {
        let id = self.next_id.get();
        self.next_id
            .set(id.checked_add(1).expect("WebGL resource id overflow"));
        id
    }

    pub(crate) fn invalidate_framebuffers_for(&self, texture: u32) {
        let mut cache = self.framebuffer_cache.borrow_mut();
        cache.retain(|key, framebuffer| {
            let keep = !key
                .attachments
                .iter()
                .any(|attachment| attachment.1 == texture);
            if !keep {
                self.gl.delete_framebuffer(Some(framebuffer));
            }
            keep
        });
    }
}

impl Drop for WebGl2Shared {
    fn drop(&mut self) {
        for framebuffer in self.framebuffer_cache.get_mut().values() {
            self.gl.delete_framebuffer(Some(framebuffer));
        }
    }
}

fn parameter_u32(context: &WebGl2RenderingContext, parameter: u32, fallback: u32) -> u32 {
    context
        .get_parameter(parameter)
        .ok()
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite() && *value >= 1.0)
        .map(|value| value as u32)
        .unwrap_or(fallback)
}

fn js_error(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}
