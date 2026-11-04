//! Pixels produced outside the framework, drawn without copying.
//!
//! A video decoder, a camera or another GPU producer hands the renderer frames
//! it already holds on the GPU. The element records one
//! [`DrawCommand::DrawExternalTexture`](crate::draw_cmd::DrawCommand::DrawExternalTexture);
//! at render time the renderer asks the [`ExternalTextureSource`] for its
//! newest [`NativeFrame`] and samples that texture directly. Nothing is
//! uploaded, and the CPU never sees the pixels.

use std::any::Any;
use std::ffi::c_void;
use std::fmt;
use std::ptr::NonNull;
use std::sync::Arc;

/// A platform GPU texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeTexture {
    /// An `id<MTLTexture>` in a pixel format the image pipeline samples
    /// (`BGRA8Unorm` for video). Not retained by this value; see
    /// [`NativeFrame::new`].
    Metal(NonNull<c_void>),
}

/// One frame of an external producer: a GPU texture and what keeps it alive.
///
/// The renderer holds the frame until the GPU has finished sampling it, so the
/// producer cannot recycle the memory under a frame still being drawn.
pub struct NativeFrame {
    texture: NativeTexture,
    width: u32,
    height: u32,
    /// Owns whatever backs `texture` (a decoder's pixel buffer, for example).
    _keep_alive: Arc<dyn Any + Send + Sync>,
}

// SAFETY: the only non-`Send`/`Sync` field is the texture handle. GPU texture
// objects (`MTLTexture`) may be used from any thread, `NativeFrame::new`'s
// contract keeps the object alive for as long as the frame exists, and the
// frame never mutates it.
unsafe impl Send for NativeFrame {}
unsafe impl Sync for NativeFrame {}

impl NativeFrame {
    /// Wraps a platform texture of `width` x `height` pixels.
    ///
    /// # Safety
    ///
    /// `texture` must be a valid texture object of that size, and it must stay
    /// valid, and its contents unchanged, for as long as `keep_alive` lives.
    #[inline]
    pub unsafe fn new(
        texture: NativeTexture,
        width: u32,
        height: u32,
        keep_alive: Arc<dyn Any + Send + Sync>,
    ) -> Self {
        Self {
            texture,
            width,
            height,
            _keep_alive: keep_alive,
        }
    }

    /// The platform texture.
    #[inline]
    pub fn texture(&self) -> NativeTexture {
        self.texture
    }

    /// Width in pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }
}

impl fmt::Debug for NativeFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFrame")
            .field("texture", &self.texture)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Something that produces frames on the GPU, such as a video player.
///
/// The renderer calls [`current_frame`](Self::current_frame) while it encodes a
/// frame, possibly on the raster thread, so it must be cheap and must not
/// block: return the newest frame already decoded, or `None`.
pub trait ExternalTextureSource: Send + Sync + 'static {
    /// The frame to show now, or `None` when there is nothing to show yet.
    fn current_frame(&self) -> Option<NativeFrame>;
}

/// An [`ExternalTextureSource`] as a draw command holds it.
///
/// Two values are equal when they share a source and a `generation`. The
/// render tree repaints a node only when its commands change, so the producer
/// bumps the generation for every new frame; recording the same generation
/// again costs nothing.
#[derive(Clone)]
pub struct ExternalTexture {
    source: Arc<dyn ExternalTextureSource>,
    generation: u64,
}

impl ExternalTexture {
    /// Draws `source` as of its frame `generation`.
    #[inline]
    pub fn new(source: Arc<dyn ExternalTextureSource>, generation: u64) -> Self {
        Self { source, generation }
    }

    /// The producer.
    #[inline]
    pub fn source(&self) -> &Arc<dyn ExternalTextureSource> {
        &self.source
    }

    /// The producer's frame counter when this was recorded.
    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The frame to draw now; see [`ExternalTextureSource::current_frame`].
    #[inline]
    pub fn current_frame(&self) -> Option<NativeFrame> {
        self.source.current_frame()
    }
}

impl PartialEq for ExternalTexture {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source) && self.generation == other.generation
    }
}

impl fmt::Debug for ExternalTexture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalTexture")
            .field("source", &Arc::as_ptr(&self.source).cast::<()>())
            .field("generation", &self.generation)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed;
    impl ExternalTextureSource for Fixed {
        fn current_frame(&self) -> Option<NativeFrame> {
            None
        }
    }

    #[test]
    fn equal_means_the_same_source_at_the_same_generation() {
        let source: Arc<dyn ExternalTextureSource> = Arc::new(Fixed);
        let other: Arc<dyn ExternalTextureSource> = Arc::new(Fixed);
        let a = ExternalTexture::new(source.clone(), 3);
        assert_eq!(a, ExternalTexture::new(source.clone(), 3));
        assert_ne!(a, ExternalTexture::new(source, 4), "a new frame repaints");
        assert_ne!(a, ExternalTexture::new(other, 3), "another source repaints");
    }

    #[test]
    fn a_frame_keeps_its_owner_alive_until_dropped() {
        let owner = Arc::new(7_u32);
        let texture = NativeTexture::Metal(NonNull::dangling());
        // SAFETY: the test never dereferences the texture handle.
        let frame = unsafe { NativeFrame::new(texture, 4, 2, owner.clone()) };
        assert_eq!(Arc::strong_count(&owner), 2);
        assert_eq!((frame.width(), frame.height(), frame.texture()), (4, 2, texture));
        drop(frame);
        assert_eq!(Arc::strong_count(&owner), 1);
    }

    #[test]
    fn frames_and_textures_cross_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<NativeFrame>();
        send_sync::<ExternalTexture>();
    }
}
