use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLCommandBuffer, MTLCommandQueue, MTLDrawable, MTLPixelFormat, MTLTexture};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};

use super::{MetalBackend, MetalTexture, MetalTextureView, PendingUpload};

pub(super) mod threaded;

#[derive(Default)]
pub(super) struct MetalQueueState {
    pub(super) uploads: Vec<PendingUpload>,
    pub(super) last_submission: Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>>,
}

/// A configured `CAMetalLayer` connected to a [`MetalBackend`].
pub struct MetalSurface {
    layer: Retained<CAMetalLayer>,
    format: MTLPixelFormat,
}

/// One drawable acquired from a [`MetalSurface`].
///
/// Keep the frame alive until after rendering, then call [`Self::present`].
pub struct MetalSurfaceFrame {
    drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>,
    texture: MetalTexture,
    view: MetalTextureView,
    presents_with_transaction: bool,
}

impl MetalSurface {
    /// Connects an existing layer to `backend` and selects its pixel format.
    ///
    /// Framebuffer-only mode is disabled because custom pipelines may copy the
    /// presented texture to capture backdrop pixels. Presentation is scheduled
    /// on a command buffer, so Core Animation transaction synchronization is
    /// disabled on the layer.
    pub fn new(
        backend: &MetalBackend,
        layer: Retained<CAMetalLayer>,
        format: MTLPixelFormat,
    ) -> Self {
        layer.setDevice(Some(&backend.device));
        layer.setPixelFormat(format);
        layer.setFramebufferOnly(false);
        layer.setPresentsWithTransaction(false);
        Self { layer, format }
    }

    /// Returns the layer's configured pixel format.
    #[inline]
    pub fn format(&self) -> MTLPixelFormat {
        self.format
    }

    /// Borrows the native layer for platform-specific configuration.
    #[inline]
    pub fn layer(&self) -> &CAMetalLayer {
        &self.layer
    }

    /// Acquires the next drawable, returning `None` when the layer is not ready.
    pub fn next_drawable(&self) -> Option<MetalSurfaceFrame> {
        let drawable = self.layer.nextDrawable()?;
        let texture = MetalTexture(drawable.texture());
        let view = MetalTextureView(texture.0.clone());
        Some(MetalSurfaceFrame {
            drawable,
            texture,
            view,
            presents_with_transaction: self.layer.presentsWithTransaction(),
        })
    }
}

impl MetalSurfaceFrame {
    /// Returns the drawable texture dimensions in pixels.
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        (
            self.texture.0.width() as u32,
            self.texture.0.height() as u32,
        )
    }

    /// Borrows the backend texture for source-texture rendering and copies.
    #[inline]
    pub fn texture(&self) -> &MetalTexture {
        &self.texture
    }

    /// Borrows the render-target view for `RendererImpl::render`.
    #[inline]
    pub fn view(&self) -> &MetalTextureView {
        &self.view
    }

    /// Presents after earlier submissions to `backend`'s command queue, such
    /// as the command buffer submitted by `RendererImpl::render`. Transactional
    /// layers wait for the most recent rendering submission to be scheduled
    /// before calling `present()` in the caller's Core Animation transaction.
    /// Ordinary layers schedule presentation on a separate command buffer.
    /// Render and present a frame without interleaving other frames on the same
    /// backend; the native window presenter does this on its event-loop thread.
    pub fn present(self, backend: &MetalBackend) {
        let metal_drawable: &ProtocolObject<dyn CAMetalDrawable> = &self.drawable;
        let drawable = <ProtocolObject<dyn CAMetalDrawable> as AsRef<
            ProtocolObject<dyn MTLDrawable>,
        >>::as_ref(metal_drawable);
        if self.presents_with_transaction {
            let _render_submission = backend.wait_for_render_submission();
            drawable.present();
        } else {
            let command_buffer = backend
                .queue
                .commandBuffer()
                .expect("the command queue to hand out a presentation command buffer");
            command_buffer.presentDrawable(drawable);
            command_buffer.commit();
        }
    }
}

impl MetalBackend {
    fn wait_for_render_submission(&self) -> Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>> {
        // An empty buffer need not carry the drawable's rendering dependency.
        // Wait on the actual producer before joining UIKit's CA transaction.
        // Release the upload/submit lock before waiting on the GPU scheduler.
        let command_buffer = self
            .queue_state
            .lock()
            .expect("Metal upload queue mutex is not poisoned")
            .last_submission
            .take()?;
        command_buffer.waitUntilScheduled();
        Some(command_buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        GpuBackend, LoadOp, RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
        TextureDescriptor, TextureDimension, TextureUsage,
    };
    use objc2_metal::MTLCommandBufferStatus;

    #[test]
    fn transactional_presentation_waits_on_the_actual_render_submission() {
        let backend = MetalBackend::new().expect("native Metal regression test requires a GPU");
        let texture = backend.create_texture(&TextureDescriptor {
            label: Some("presentation regression target".to_string()),
            size: (8, 8, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: MTLPixelFormat::BGRA8Unorm,
            usage: vec![TextureUsage::RenderAttachment],
        });
        let view = backend.create_texture_view(&texture, "presentation regression view");
        for _ in 0..2 {
            let mut encoder = backend.create_command_encoder("actual frame rendering");
            backend.begin_render_pass(
                &mut encoder,
                &RenderPassDescriptor {
                    label: None,
                    color_attachments: &[RenderPassColorAttachment {
                        view: &view,
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
            let waited = backend
                .wait_for_render_submission()
                .expect("render submission");
            assert_eq!(
                Retained::as_ptr(&waited),
                Retained::as_ptr(&rendered),
                "presentation must synchronize with the rendering buffer, not an empty barrier"
            );
            assert!(matches!(
                waited.status(),
                MTLCommandBufferStatus::Scheduled | MTLCommandBufferStatus::Completed
            ));
        }
        assert!(
            backend.wait_for_render_submission().is_none(),
            "do not reuse an older frame's submission"
        );
    }
}
