use std::cell::Cell;
use std::rc::Rc;

use super::gl;
use super::{
    OpenGlError, OpenGlShared, OpenGlTextureFormat, OpenGlTextureView, OpenGlTextureViewInner,
    OpenGlViewTarget,
};

/// A window's OpenGL default framebuffer and its drawable dimensions.
pub struct OpenGlSurface {
    shared: Rc<OpenGlShared>,
    size: (u32, u32),
    view: OpenGlTextureView,
}

/// An acquired OpenGL default-framebuffer drawable.
pub struct OpenGlSurfaceFrame<'a> {
    surface: &'a mut OpenGlSurface,
    view: OpenGlTextureView,
    completed: bool,
}

impl OpenGlSurface {
    pub(super) fn new(shared: Rc<OpenGlShared>, size: (u32, u32)) -> Self {
        let view = OpenGlTextureView(Rc::new(OpenGlTextureViewInner {
            target: OpenGlViewTarget::Surface {
                size: Cell::new(size),
            },
        }));
        Self { shared, size, view }
    }

    /// Updates the drawable extent after a native window resize.
    pub fn resize(&mut self, size: (u32, u32)) -> Result<(), OpenGlError> {
        self.size = size;
        if let OpenGlViewTarget::Surface { size: view_size } = &self.view.0.target {
            view_size.set(size);
        }
        self.shared
            .context
            .resize(size)
            .map_err(|error| OpenGlError::new("resize OpenGL surface", error))
    }

    /// Returns the current drawable size in physical pixels.
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Returns the color format used for the default framebuffer.
    #[inline]
    pub fn format(&self) -> OpenGlTextureFormat {
        OpenGlTextureFormat::Rgba8Unorm
    }

    /// Returns whether the default framebuffer uses sRGB encoding.
    #[inline]
    pub fn is_srgb(&self) -> bool {
        false
    }

    /// Acquires the default framebuffer for a frame.
    pub fn try_acquire(&mut self) -> Result<Option<OpenGlSurfaceFrame<'_>>, OpenGlError> {
        if self.size.0 == 0 || self.size.1 == 0 {
            return Ok(None);
        }
        self.shared
            .context
            .make_current()
            .map_err(|error| OpenGlError::new("activate OpenGL drawable", error))?;
        self.shared.bind_framebuffer(gl::FRAMEBUFFER, 0);
        self.shared
            .set_viewport(0, 0, self.size.0 as i32, self.size.1 as i32);
        let view = self.view.clone();
        Ok(Some(OpenGlSurfaceFrame {
            surface: self,
            view,
            completed: false,
        }))
    }
}

impl OpenGlSurfaceFrame<'_> {
    /// Returns the default framebuffer view used by Cupid's renderer.
    #[inline]
    pub fn view(&self) -> &OpenGlTextureView {
        &self.view
    }

    /// Returns the acquired drawable's physical dimensions.
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        self.surface.size
    }

    /// Presents the rendered frame.
    pub fn present(mut self) -> Result<(), OpenGlError> {
        let result = self
            .surface
            .shared
            .context
            .swap_buffers()
            .map_err(|error| OpenGlError::new("present OpenGL frame", error));
        self.completed = true;
        result
    }
}

impl Drop for OpenGlSurfaceFrame<'_> {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.surface.shared.context.swap_buffers();
        }
    }
}
