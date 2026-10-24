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
