use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::MTLDrawable;
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};

use super::{MetalBackend, MetalSurface, MetalSurfaceFrame, MetalTexture, MetalTextureView};

/// An acquisition-only handle to a Metal layer configured by the UI thread.
///
/// Layer geometry and attachment remain the UI owner's responsibility. This
/// handle exposes no AppKit/UIKit objects or layer mutation methods.
#[doc(hidden)]
pub struct MetalDrawableSource {
    layer: Retained<CAMetalLayer>,
}

// SAFETY: CAMetalLayer supports drawable acquisition from a rendering thread.
// The retained reference keeps the layer alive; only acquisition is exposed,
// while the UI owns geometry/configuration. No command encoder crosses threads.
unsafe impl Send for MetalDrawableSource {}

impl MetalSurface {
    /// Retains an acquisition-only handle for a dedicated rendering thread.
    #[doc(hidden)]
    pub fn drawable_source(&self) -> MetalDrawableSource {
        MetalDrawableSource {
            layer: self.layer.clone(),
        }
    }
}

impl MetalDrawableSource {
    /// Acquires a drawable. May wait for the layer's bounded drawable pool.
    pub fn next_drawable(&self) -> Option<MetalSurfaceFrame> {
        let drawable = self.layer.nextDrawable()?;
        let texture = MetalTexture(drawable.texture());
        let view = MetalTextureView(texture.0.clone());
        Some(MetalSurfaceFrame {
            drawable,
            texture,
            view,
            presents_with_transaction: true,
        })
    }
}

/// Asks Metal to call `observer` with the host time `drawable` reaches the
/// screen, in seconds on the `CACurrentMediaTime` clock (0 if it was skipped).
///
/// The observer runs on a Metal thread after the frame is shown, so it must be
/// cheap and may not touch UI state. Register it before presenting.
fn observe_drawable_presentation(
    drawable: &ProtocolObject<dyn MTLDrawable>,
    observer: impl Fn(f64) + 'static,
) {
    let handler = RcBlock::new(move |shown: NonNull<ProtocolObject<dyn MTLDrawable>>| {
        // SAFETY: Metal passes the live drawable the handler was added to.
        observer(unsafe { shown.as_ref() }.presentedTime());
    });
    // SAFETY: `handler` is a valid block, and Metal retains it until called.
    unsafe { drawable.addPresentedHandler(RcBlock::as_ptr(&handler)) };
}

impl MetalSurfaceFrame {
    /// Reports when this frame reaches the screen. See
    /// [`observe_drawable_presentation`]'s contract; call before presenting.
    #[doc(hidden)]
    pub fn observe_presentation(&self, observer: impl Fn(f64) + 'static) {
        let metal_drawable: &ProtocolObject<dyn CAMetalDrawable> = &self.drawable;
        observe_drawable_presentation(metal_drawable.as_ref(), observer);
    }
}

impl MetalDrawableSource {
    /// Acquires a drawable the rendering thread presents by itself.
    ///
    /// Call [`MetalSurfaceFrame::present`] on the returned frame once it is
    /// rendered: it schedules the presentation on the command queue and returns
    /// at once, with no wait for the GPU and no main-thread transaction. That is
    /// only right while the layer's size and scale are not changing, because the
    /// frame then reaches the screen on its own schedule instead of together with
    /// the layer's geometry. The layer must not present with a transaction.
    #[doc(hidden)]
    pub fn next_direct_drawable(&self) -> Option<MetalSurfaceFrame> {
        let drawable = self.layer.nextDrawable()?;
        let texture = MetalTexture(drawable.texture());
        let view = MetalTextureView(texture.0.clone());
        Some(MetalSurfaceFrame {
            drawable,
            texture,
            view,
            presents_with_transaction: false,
        })
    }
}

/// A drawable whose actual producer submission has already been scheduled.
///
/// The UI can present it in a Core Animation transaction without a GPU wait.
#[doc(hidden)]
pub struct ScheduledMetalDrawable {
    drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>,
    size: (u32, u32),
}

// SAFETY: Metal drawables are retained GPU objects. Exclusive ownership moves
// from the renderer to the UI, and present() enforces main-thread affinity.
unsafe impl Send for ScheduledMetalDrawable {}

impl MetalSurfaceFrame {
    /// Waits for this frame's rendering submission on its rendering thread.
    ///
    /// Render and call this method without interleaving another frame on the
    /// same backend. The encoder remains local to that thread throughout.
    #[doc(hidden)]
    pub fn schedule_transactional_presentation(
        self,
        backend: &MetalBackend,
    ) -> ScheduledMetalDrawable {
        let size = self.size();
        let _submission = backend.wait_for_render_submission();
        ScheduledMetalDrawable {
            drawable: self.drawable,
            size,
        }
    }
}

impl ScheduledMetalDrawable {
    /// Reports when this frame reaches the screen; call before [`Self::present`].
    #[doc(hidden)]
    pub fn observe_presentation(&self, observer: impl Fn(f64) + 'static) {
        let metal_drawable: &ProtocolObject<dyn CAMetalDrawable> = &self.drawable;
        observe_drawable_presentation(metal_drawable.as_ref(), observer);
    }

    /// Returns the acquired drawable dimensions for viewport validation.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Presents in the current main-thread Core Animation transaction.
    ///
    /// Panics off the main thread. The caller must establish the transaction
    /// and validate that the drawable still matches the window's viewport.
    pub fn present(self) {
        assert!(
            objc2::MainThreadMarker::new().is_some(),
            "transactional presentation requires the main thread"
        );
        let metal_drawable: &ProtocolObject<dyn CAMetalDrawable> = &self.drawable;
        let drawable: &ProtocolObject<dyn MTLDrawable> = metal_drawable.as_ref();
        drawable.present();
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::CGSize;
    use objc2_quartz_core::CAMetalLayer;

    use super::*;
    use crate::backend::{
        GpuBackend, LoadOp, RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
    };
    use objc2_metal::MTLPixelFormat;

    /// A direct presentation must not wait for the GPU to schedule the frame:
    /// that wait is what ties the rendering thread to the transaction path.
    /// Waiting consumes the recorded submission, so a direct present that
    /// leaves it in place did not wait.
    #[test]
    fn a_direct_present_does_not_wait_for_the_render_submission() {
        let backend = MetalBackend::new().expect("native Metal regression test requires a GPU");
        let layer = CAMetalLayer::new();
        let surface = MetalSurface::new(&backend, layer, MTLPixelFormat::BGRA8Unorm);
        surface
            .layer()
            .setDrawableSize(CGSize::new(16.0, 16.0));
        let source = surface.drawable_source();

        let frame = source
            .next_direct_drawable()
            .expect("a detached layer hands out a drawable");
        assert!(!frame.presents_with_transaction);
        let mut encoder = backend.create_command_encoder("direct presentation regression");
        backend.begin_render_pass(
            &mut encoder,
            &RenderPassDescriptor {
                label: None,
                color_attachments: &[RenderPassColorAttachment {
                    view: frame.view(),
                    resolve_target: None,
                    ops: crate::backend::Operations {
                        load: LoadOp::Clear([0.0; 4]),
                        store: StoreOp::Store,
                    },
                }],
                depth_stencil_attachment: None,
            },
        );
        let rendered = encoder.command_buffer.clone();
        backend.submit(encoder);

        // Registering an observer must not change how the frame is presented.
        frame.observe_presentation(|_| {});
        frame.present(&backend);

        let still_recorded = backend
            .wait_for_render_submission()
            .expect("a direct present leaves the submission for whoever waits next");
        assert_eq!(
            Retained::as_ptr(&still_recorded),
            Retained::as_ptr(&rendered)
        );
    }
}
