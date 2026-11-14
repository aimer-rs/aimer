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

    /// Reads back what has been drawn into this frame, for a screenshot: `width *
    /// height * 4` bytes of RGBA, **bottom row first**, as OpenGL stores the
    /// default framebuffer.
    ///
    /// Call it after Cupid's render pass and before [`present`](Self::present): once
    /// the buffers are swapped the back buffer holds nothing defined. It waits for
    /// the GPU to finish the frame, so it is for a rare picture, not for every frame.
    pub fn read_pixels(&self) -> Vec<u8> {
        let (width, height) = self.surface.size;
        let shared = &self.surface.shared;
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        shared.bind_framebuffer(gl::READ_FRAMEBUFFER, 0);
        unsafe {
            // A pixel pack buffer left bound would turn the destination into an offset.
            (shared.gl.bind_buffer)(gl::PIXEL_PACK_BUFFER, 0);
            (shared.gl.pixel_store_i)(gl::PACK_ALIGNMENT, 1);
            (shared.gl.read_pixels)(
                0,
                0,
                width as i32,
                height as i32,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                pixels.as_mut_ptr().cast(),
            );
        }
        pixels
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
