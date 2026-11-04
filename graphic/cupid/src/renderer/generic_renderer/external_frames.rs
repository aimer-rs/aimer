//! Frames of external GPU producers (video), drawn without copying.
//!
//! A `DrawExternalTexture` command asks its producer for the current frame,
//! wraps that frame's texture as a backend view and draws it through the image
//! pipeline like a retained layer. The frame object is then kept alive here for
//! the next few renders: the producer may recycle a frame's memory once it is
//! released, and the GPU can still be sampling it from a frame in flight.

use crate::backend::{GpuBackend, NativeTextureHandle};
use crate::external_texture::{ExternalTexture, NativeFrame, NativeTexture};

/// Renders a frame is kept alive for. A presenting surface has at most three
/// drawables in flight (`CAMetalLayer`'s maximum); one more is margin.
const KEPT_RENDERS: usize = 4;

/// The backend handle for `frame`'s texture, or `None` for a kind of texture
/// this build does not know.
fn handle_of(frame: &NativeFrame) -> Option<NativeTextureHandle> {
    Some(match frame.texture() {
        NativeTexture::Metal(pointer) => NativeTextureHandle::Metal(pointer),
        NativeTexture::WebVideo { id, serial } => NativeTextureHandle::WebVideo {
            id,
            serial,
            width: frame.width(),
            height: frame.height(),
        },
        #[allow(unreachable_patterns)]
        _ => return None,
    })
}

/// Imported frames, one slot per render, reused round-robin.
pub(super) struct ExternalFrames {
    slots: [Vec<NativeFrame>; KEPT_RENDERS],
    current: usize,
}

impl ExternalFrames {
    pub(super) fn new() -> Self {
        Self {
            slots: Default::default(),
            current: 0,
        }
    }

    /// Starts a render: releases the frames of the render `KEPT_RENDERS` ago,
    /// which the GPU has finished with, and reuses their slot.
    pub(super) fn begin_render(&mut self, frame_index: u64) {
        self.current = (frame_index % KEPT_RENDERS as u64) as usize;
        // `clear` keeps the capacity, so a steady video allocates nothing.
        self.slots[self.current].clear();
    }

    /// Imports `texture`'s current frame for this render, returning a view the
    /// image pipeline can sample, or `None` when the producer has no frame or
    /// the backend cannot import its kind of texture.
    pub(super) fn import<B: GpuBackend>(
        &mut self,
        backend: &B,
        texture: &ExternalTexture,
    ) -> Option<B::TextureView> {
        let frame = texture.current_frame()?;
        let handle = handle_of(&frame)?;
        // SAFETY: `NativeFrame::new`'s contract keeps the texture valid while
        // `frame` lives, and `frame` is kept in this render's slot until
        // `KEPT_RENDERS` renders later, after the GPU has sampled it.
        let view = unsafe { backend.import_native_texture(handle) }?;
        self.hold(frame);
        Some(view)
    }

    /// Keeps `frame` alive through this render and the next few.
    fn hold(&mut self, frame: NativeFrame) {
        self.slots[self.current].push(frame);
    }

    /// Frames held across all slots; for tests.
    #[cfg(test)]
    pub(super) fn held(&self) -> usize {
        self.slots.iter().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::external_texture::ExternalTextureSource;

    /// Hands out a new frame on every call and counts the live ones.
    struct Counting {
        live: Arc<AtomicU64>,
    }

    struct Alive(Arc<AtomicU64>);
    impl Drop for Alive {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl ExternalTextureSource for Counting {
        fn current_frame(&self) -> Option<NativeFrame> {
            self.live.fetch_add(1, Ordering::SeqCst);
            let owner = Arc::new(Alive(self.live.clone()));
            // SAFETY: the tests never dereference the texture handle.
            Some(unsafe { NativeFrame::new(NativeTexture::Metal(NonNull::dangling()), 16, 9, owner) })
        }
    }

    fn hold_current(frames: &mut ExternalFrames, texture: &ExternalTexture) {
        frames.hold(texture.current_frame().expect("the source always has a frame"));
    }

    #[test]
    fn a_web_video_frame_names_its_element_picture_and_size() {
        let texture = NativeTexture::WebVideo { id: 3, serial: 9 };
        // SAFETY: the handle is a pair of numbers; nothing is dereferenced.
        let frame = unsafe { NativeFrame::new(texture, 1280, 720, Arc::new(())) };
        assert_eq!(
            handle_of(&frame),
            Some(NativeTextureHandle::WebVideo { id: 3, serial: 9, width: 1280, height: 720 })
        );
    }

    #[test]
    fn a_metal_frame_keeps_its_pointer() {
        let pointer = NonNull::dangling();
        // SAFETY: the pointer is never dereferenced.
        let frame = unsafe { NativeFrame::new(NativeTexture::Metal(pointer), 16, 9, Arc::new(())) };
        assert_eq!(handle_of(&frame), Some(NativeTextureHandle::Metal(pointer)));
    }

    #[test]
    fn a_frame_is_kept_until_its_slot_comes_round_again() {
        let live = Arc::new(AtomicU64::new(0));
        let texture = ExternalTexture::new(Arc::new(Counting { live: live.clone() }), 0);
        let mut frames = ExternalFrames::new();
        for render in 0..KEPT_RENDERS as u64 {
            frames.begin_render(render);
            hold_current(&mut frames, &texture);
        }
        assert_eq!(live.load(Ordering::SeqCst), KEPT_RENDERS as u64);
        assert_eq!(frames.held(), KEPT_RENDERS);

        // The next render reuses the oldest slot and releases only its frame.
        frames.begin_render(KEPT_RENDERS as u64);
        assert_eq!(live.load(Ordering::SeqCst), KEPT_RENDERS as u64 - 1);
        assert_eq!(frames.held(), KEPT_RENDERS - 1);
    }
}
