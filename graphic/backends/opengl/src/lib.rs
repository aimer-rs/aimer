//! Direct OpenGL backend for native Windows/Linux windows and Android GLES 3.
//!
//! The crate is empty on every target but Linux and Android; `aimer_cupid`
//! selects it with its `native` feature.

#![cfg(any(target_os = "linux", target_os = "android"))]

// `crate::backend::..` is how this code has always named the contract.
pub(crate) use aimer_cupid_gpu as backend;

mod context;
mod gl;
mod pipeline;
mod surface;
mod trait_impl;

pub use pipeline::{
    OpenGlBindGroup, OpenGlBindGroupLayout, OpenGlCommandEncoder, OpenGlPipelineLayout,
    OpenGlRenderPass, OpenGlRenderPipeline, OpenGlShaderModule,
};
pub use surface::{OpenGlSurface, OpenGlSurfaceFrame};

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CStr;
use std::rc::Rc;
use std::sync::Arc;

use crate::backend::GpuLimits;

use self::context::NativeContext;
use self::gl::{GLuint, GlFns};

/// Texture formats supported by Cupid's desktop OpenGL renderer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OpenGlTextureFormat {
    R8Unorm,
    Rg8Unorm,
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    Rgba16Float,
    Depth16Unorm,
    Depth24Plus,
    Depth24PlusStencil8,
    Depth32Float,
}

/// An error returned while creating an OpenGL context or compiling a shader.
#[derive(Clone, Debug)]
pub struct OpenGlError {
    operation: &'static str,
    message: String,
}

impl OpenGlError {
    pub(crate) fn new(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for OpenGlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed: {}", self.operation, self.message)
    }
}

impl std::error::Error for OpenGlError {}

/// Native OpenGL device/context wrapper used by Cupid's generic renderer.
pub struct OpenGlBackend {
    pub(crate) shared: Rc<OpenGlShared>,
}

pub(crate) struct OpenGlShared {
    context: NativeContext,
    pub(crate) gl: GlFns,
    pub(crate) limits: GpuLimits,
    pub(crate) max_texture_units: u32,
    pub(crate) max_vertex_attributes: u32,
    pub(crate) max_uniform_buffer_bindings: u32,
    pub(crate) framebuffer_cache: RefCell<HashMap<FramebufferKey, GLuint>>,
    pub(crate) state: GlStateCache,
}

#[derive(Default)]
pub(crate) struct GlStateCache {
    program: Cell<Option<GLuint>>,
    vertex_array: Cell<Option<GLuint>>,
    read_framebuffer: Cell<Option<GLuint>>,
    draw_framebuffer: Cell<Option<GLuint>>,
    viewport: Cell<Option<(i32, i32, i32, i32)>>,
    scissor: Cell<Option<(i32, i32, i32, i32)>>,
    active_texture: Cell<Option<u32>>,
    textures: RefCell<HashMap<(u32, u32), GLuint>>,
    samplers: RefCell<HashMap<u32, GLuint>>,
    uniform_buffers: RefCell<HashMap<u32, (GLuint, isize, isize)>>,
    capabilities: RefCell<HashMap<u32, bool>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FramebufferKey {
    pub(crate) attachments: Vec<(u32, u32, u32, i32)>,
    pub(crate) color_count: usize,
}

#[derive(Clone)]
pub struct OpenGlBuffer(pub(crate) Rc<OpenGlBufferInner>);

pub(crate) struct OpenGlBufferInner {
    pub(crate) shared: Rc<OpenGlShared>,
    pub(crate) raw: GLuint,
    pub(crate) size: u64,
}

impl Drop for OpenGlBufferInner {
    fn drop(&mut self) {
        if self.shared.context.make_current().is_ok() {
            self.shared.invalidate_buffer(self.raw);
            unsafe { (self.shared.gl.delete_buffers)(1, &self.raw) };
        }
    }
}

#[derive(Clone)]
pub struct OpenGlTexture(pub(crate) Rc<OpenGlTextureInner>);

pub(crate) struct OpenGlTextureInner {
    pub(crate) shared: Rc<OpenGlShared>,
    pub(crate) raw: GLuint,
    pub(crate) target: u32,
    pub(crate) size: (u32, u32, u32),
    pub(crate) format: OpenGlTextureFormat,
}

impl Drop for OpenGlTextureInner {
    fn drop(&mut self) {
        if self.shared.context.make_current().is_ok() {
            self.shared.invalidate_framebuffers_for(self.raw);
            unsafe { (self.shared.gl.delete_textures)(1, &self.raw) };
        }
    }
}

#[derive(Clone)]
pub struct OpenGlTextureView(pub(crate) Rc<OpenGlTextureViewInner>);

pub(crate) struct OpenGlTextureViewInner {
    pub(crate) target: OpenGlViewTarget,
}

pub(crate) enum OpenGlViewTarget {
    Surface {
        size: Cell<(u32, u32)>,
    },
    Texture {
        texture: OpenGlTexture,
        level: u32,
        layer: Option<u32>,
    },
}

impl OpenGlTextureView {
    pub(crate) fn gl_view_size(&self) -> (u32, u32) {
        match &self.0.target {
            OpenGlViewTarget::Surface { size } => size.get(),
            OpenGlViewTarget::Texture { texture, level, .. } => (
                (texture.0.size.0 >> *level).max(1),
                (texture.0.size.1 >> *level).max(1),
            ),
        }
    }
}

#[derive(Clone)]
pub struct OpenGlSampler(pub(crate) Rc<OpenGlSamplerInner>);

pub(crate) struct OpenGlSamplerInner {
    pub(crate) shared: Rc<OpenGlShared>,
    pub(crate) raw: GLuint,
}

impl Drop for OpenGlSamplerInner {
    fn drop(&mut self) {
        if self.shared.context.make_current().is_ok() {
            self.shared.invalidate_sampler(self.raw);
            unsafe { (self.shared.gl.delete_samplers)(1, &self.raw) };
        }
    }
}

impl OpenGlShared {
    pub(crate) fn use_program(&self, program: GLuint) {
        if self.state.program.replace(Some(program)) != Some(program) {
            unsafe { (self.gl.use_program)(program) };
        }
    }

    pub(crate) fn bind_vertex_array(&self, array: GLuint) {
        if self.state.vertex_array.replace(Some(array)) != Some(array) {
            unsafe { (self.gl.bind_vertex_array)(array) };
        }
    }

    pub(crate) fn bind_framebuffer(&self, target: u32, framebuffer: GLuint) {
        let draw_changed = target != gl::READ_FRAMEBUFFER
            && self.state.draw_framebuffer.replace(Some(framebuffer)) != Some(framebuffer);
        let read_changed = target != gl::DRAW_FRAMEBUFFER
            && self.state.read_framebuffer.replace(Some(framebuffer)) != Some(framebuffer);
        if draw_changed || read_changed {
            unsafe { (self.gl.bind_framebuffer)(target, framebuffer) };
        }
    }

    pub(crate) fn set_viewport(&self, x: i32, y: i32, width: i32, height: i32) {
        let viewport = (x, y, width, height);
        if self.state.viewport.replace(Some(viewport)) != Some(viewport) {
            unsafe { (self.gl.viewport)(x, y, width, height) };
        }
    }

    pub(crate) fn set_scissor(&self, x: i32, y: i32, width: i32, height: i32) {
        let scissor = (x, y, width, height);
        if self.state.scissor.replace(Some(scissor)) != Some(scissor) {
            unsafe { (self.gl.scissor)(x, y, width, height) };
        }
    }

    pub(crate) fn set_capability(&self, capability: u32, enabled: bool) {
        let changed = self
            .state
            .capabilities
            .borrow_mut()
            .insert(capability, enabled)
            != Some(enabled);
        if changed {
            unsafe {
                if enabled {
                    (self.gl.enable)(capability)
                } else {
                    (self.gl.disable)(capability)
                }
            };
        }
    }

    pub(crate) fn active_texture(&self, unit: u32) {
        if self.state.active_texture.replace(Some(unit)) != Some(unit) {
            unsafe { (self.gl.active_texture)(gl::TEXTURE0 + unit) };
        }
    }

    pub(crate) fn bind_texture(&self, unit: u32, target: u32, texture: GLuint) {
        self.active_texture(unit);
        let changed = self
            .state
            .textures
            .borrow_mut()
            .insert((unit, target), texture)
            != Some(texture);
        if changed {
            unsafe { (self.gl.bind_texture)(target, texture) };
        }
    }

    pub(crate) fn bind_sampler(&self, unit: u32, sampler: GLuint) {
        let changed = self.state.samplers.borrow_mut().insert(unit, sampler) != Some(sampler);
        if changed {
            unsafe { (self.gl.bind_sampler)(unit, sampler) };
        }
    }

    pub(crate) fn bind_uniform_buffer(
        &self,
        point: u32,
        buffer: GLuint,
        offset: isize,
        size: isize,
    ) {
        let binding = (buffer, offset, size);
        let changed = self
            .state
            .uniform_buffers
            .borrow_mut()
            .insert(point, binding)
            != Some(binding);
        if changed {
            unsafe { (self.gl.bind_buffer_range)(gl::UNIFORM_BUFFER, point, buffer, offset, size) };
        }
    }

    pub(crate) fn invalidate_program(&self, program: GLuint) {
        if self.state.program.get() == Some(program) {
            self.state.program.set(None);
        }
    }

    pub(crate) fn invalidate_vertex_array(&self, array: GLuint) {
        if self.state.vertex_array.get() == Some(array) {
            self.state.vertex_array.set(None);
        }
    }

    pub(crate) fn invalidate_framebuffer(&self, framebuffer: GLuint) {
        if self.state.read_framebuffer.get() == Some(framebuffer) {
            self.state.read_framebuffer.set(None);
        }
        if self.state.draw_framebuffer.get() == Some(framebuffer) {
            self.state.draw_framebuffer.set(None);
        }
    }

    pub(crate) fn invalidate_buffer(&self, buffer: GLuint) {
        self.state
            .uniform_buffers
            .borrow_mut()
            .retain(|_, (bound, _, _)| *bound != buffer);
    }

    pub(crate) fn invalidate_sampler(&self, sampler: GLuint) {
        self.state
            .samplers
            .borrow_mut()
            .retain(|_, bound| *bound != sampler);
    }

    pub(crate) fn invalidate_texture(&self, texture: GLuint) {
        self.state
            .textures
            .borrow_mut()
            .retain(|_, bound| *bound != texture);
    }

    pub(crate) fn invalidate_framebuffers_for(&self, texture: GLuint) {
        let mut cache = self.framebuffer_cache.borrow_mut();
        let stale: Vec<_> = cache
            .iter()
            .filter_map(|(key, framebuffer)| {
                key.attachments
                    .iter()
                    .any(|attachment| attachment.1 == texture)
                    .then_some(*framebuffer)
            })
            .collect();
        cache.retain(|key, _| {
            !key.attachments
                .iter()
                .any(|attachment| attachment.1 == texture)
        });
        for framebuffer in stale {
            if self.state.read_framebuffer.get() == Some(framebuffer) {
                self.state.read_framebuffer.set(None);
            }
            if self.state.draw_framebuffer.get() == Some(framebuffer) {
                self.state.draw_framebuffer.set(None);
            }
            unsafe { (self.gl.delete_framebuffers)(1, &framebuffer) };
        }
        drop(cache);
        self.invalidate_texture(texture);
    }
}

impl Drop for OpenGlShared {
    fn drop(&mut self) {
        if self.context.make_current().is_err() {
            return;
        }
        for framebuffer in self.framebuffer_cache.get_mut().values().copied() {
            unsafe { (self.gl.delete_framebuffers)(1, &framebuffer) };
        }
    }
}

impl OpenGlBackend {
    /// Returns the oldest pending `glGetError` code, or `0` when the driver has
    /// nothing to report.
    ///
    /// A diagnostic for tests and tooling that render through this backend and
    /// want to know whether the driver objected. Reading the code clears it.
    ///
    /// # Errors
    ///
    /// Fails if the backend's context cannot be made current on this thread.
    #[doc(hidden)]
    pub fn pending_gl_error(&self) -> Result<u32, OpenGlError> {
        self.shared
            .context
            .make_current()
            .map_err(|message| OpenGlError::new("make current", message))?;
        // SAFETY: the context was just made current on this thread, which is
        // what `glGetError` requires.
        Ok(unsafe { (self.shared.gl.get_error)() })
    }

    /// Creates an OpenGL 3.3 Core backend and an attached window surface.
    pub fn new_windowed<W>(
        window: Arc<W>,
        size: (u32, u32),
    ) -> Result<(Self, OpenGlSurface), OpenGlError>
    where
        W: raw_window_handle::HasDisplayHandle
            + raw_window_handle::HasWindowHandle
            + Send
            + Sync
            + 'static,
    {
        let context = NativeContext::new(window, size)
            .map_err(|error| OpenGlError::new("create OpenGL context", error))?;
        let shared = Self::shared_from_context(context)?;
        let surface = OpenGlSurface::new(shared.clone(), size);
        Ok((Self { shared }, surface))
    }

    /// Creates a headless OpenGL 3.3 Core backend for offscreen rendering.
    ///
    /// This is intended for GPU tests and readback workflows. On Linux it uses
    /// an EGL pbuffer; on Windows it uses a hidden WGL window.
    pub fn new_headless() -> Result<Self, OpenGlError> {
        let context = NativeContext::new_headless()
            .map_err(|error| OpenGlError::new("create headless OpenGL context", error))?;
        Ok(Self {
            shared: Self::shared_from_context(context)?,
        })
    }

    fn shared_from_context(context: NativeContext) -> Result<Rc<OpenGlShared>, OpenGlError> {
        context
            .make_current()
            .map_err(|error| OpenGlError::new("activate OpenGL context", error))?;
        let gl = context
            .load_gl()
            .map_err(|error| OpenGlError::new("load OpenGL entry points", error))?;
        let mut gl_major = 0;
        let mut gl_minor = 0;
        let mut max_texture_dimension_2d = 0;
        let mut min_uniform_buffer_offset_alignment = 0;
        let mut max_texture_units = 0;
        let mut max_vertex_attributes = 0;
        let mut max_uniform_buffer_bindings = 0;
        unsafe {
            (gl.get_integerv)(gl::MAJOR_VERSION, &mut gl_major);
            (gl.get_integerv)(gl::MINOR_VERSION, &mut gl_minor);
            (gl.get_integerv)(gl::MAX_TEXTURE_SIZE, &mut max_texture_dimension_2d);
            (gl.get_integerv)(
                gl::UNIFORM_BUFFER_OFFSET_ALIGNMENT,
                &mut min_uniform_buffer_offset_alignment,
            );
            (gl.get_integerv)(gl::MAX_TEXTURE_IMAGE_UNITS, &mut max_texture_units);
            (gl.get_integerv)(gl::MAX_VERTEX_ATTRIBS, &mut max_vertex_attributes);
            (gl.get_integerv)(
                gl::MAX_UNIFORM_BUFFER_BINDINGS,
                &mut max_uniform_buffer_bindings,
            );
        }
        if (gl_major, gl_minor) < (3, 3) {
            let version =
                unsafe { CStr::from_ptr((gl.get_string)(gl::VERSION).cast()) }.to_string_lossy();
            return Err(OpenGlError::new(
                "validate OpenGL version",
                format!("OpenGL 3.3 Core is required; driver reports {version}"),
            ));
        }
        let gl_error = unsafe { (gl.get_error)() };
        if gl_error != gl::NO_ERROR {
            return Err(OpenGlError::new(
                "query OpenGL limits",
                format!("OpenGL reported error {gl_error:#x}"),
            ));
        }
        let shared = Rc::new(OpenGlShared {
            context,
            gl,
            limits: GpuLimits {
                max_texture_dimension_2d: max_texture_dimension_2d.max(1) as u32,
                min_uniform_buffer_offset_alignment: min_uniform_buffer_offset_alignment.max(1)
                    as u32,
            },
            max_texture_units: max_texture_units.max(1) as u32,
            max_vertex_attributes: max_vertex_attributes.max(1) as u32,
            max_uniform_buffer_bindings: max_uniform_buffer_bindings.max(1) as u32,
            framebuffer_cache: RefCell::new(HashMap::new()),
            state: GlStateCache::default(),
        });
        Ok(shared)
    }
}
