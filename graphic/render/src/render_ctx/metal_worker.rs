//! Metal encoding on the worker, with layer attachment and transactions on the UI.
use aimer_cupid::AntiAlias;
use aimer_cupid::backend::GpuBackend;
use aimer_cupid::backend::metal::{
    MetalBackend, MetalDrawableSource, MetalSurface, ScheduledMetalDrawable,
};
use aimer_cupid::frame::FramePacket;
use aimer_cupid::renderer::RendererImpl;
use objc2_core_foundation::CGSize;
use objc2_metal::MTLPixelFormat;
use objc2_quartz_core::{CAMetalLayer, CATransaction};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::dpi::PhysicalSize;
use std::sync::Arc;
use winit::window::Window;

use super::native_ctx::BackendOutput;
use aimer_frame_stats::{FramePhase, PhaseTimer};
use crate::raster::WorkerPresenter;

const FORMAT: MTLPixelFormat = MTLPixelFormat::BGRA8Unorm_sRGB;

pub(super) struct MetalHost {
    surface: MetalSurface,
    window: Arc<Window>,
    size: PhysicalSize<u32>,
    scale: f64,
}

pub(super) struct MetalSeed {
    backend: MetalBackend,
    source: MetalDrawableSource,
    antialiasing: AntiAlias,
}

pub(super) struct MetalPresenter {
    backend: MetalBackend,
    source: MetalDrawableSource,
    renderer: RendererImpl<MetalBackend>,
}

impl MetalHost {
    pub(super) fn new(
        window: &Arc<Window>,
        size: PhysicalSize<u32>,
        antialiasing: AntiAlias,
    ) -> (Self, MetalSeed, u32) {
        assert!(
            objc2::MainThreadMarker::new().is_some(),
            "Metal layer attachment requires the main thread"
        );
        let backend = MetalBackend::new().expect("create the system Metal device and queue");
        let layer = CAMetalLayer::new();
        let handle = window
            .window_handle()
            .expect("Apple window handle")
            .as_raw();
        #[cfg(target_os = "macos")]
        {
            let RawWindowHandle::AppKit(handle) = handle else {
                panic!("expected AppKit handle")
            };
            // SAFETY: winit owns this view for the retained window's lifetime;
            // this method is restricted to the event-loop/main thread.
            let view = unsafe { &*handle.ns_view.as_ptr().cast::<objc2_app_kit::NSView>() };
            view.setWantsLayer(true);
            view.setLayer(Some(&layer));
        }
        #[cfg(target_os = "ios")]
        {
            let RawWindowHandle::UiKit(handle) = handle else {
                panic!("expected UIKit handle")
            };
            // SAFETY: same lifetime and main-thread invariants as the AppKit branch.
            let view = unsafe { &*handle.ui_view.as_ptr().cast::<objc2_ui_kit::UIView>() };
            layer.setFrame(view.bounds());
            layer.setContentsScale(view.contentScaleFactor());
            view.layer().addSublayer(&layer);
        }
        let surface = MetalSurface::new(&backend, layer, FORMAT);
        surface.layer().setPresentsWithTransaction(true);
        surface.layer().setAllowsNextDrawableTimeout(true);
        let max_dimension = backend.limits().max_texture_dimension_2d;
        let source = surface.drawable_source();
        let mut host = Self {
            surface,
            window: window.clone(),
            size: PhysicalSize::new(0, 0),
            scale: 0.0,
        };
        host.prepare(PhysicalSize::new(
            size.width.min(max_dimension),
            size.height.min(max_dimension),
        ));
        (
            host,
            MetalSeed {
                backend,
                source,
                antialiasing,
            },
            max_dimension,
        )
    }

    pub(super) fn prepare(&mut self, size: PhysicalSize<u32>) {
        let scale = self.window.scale_factor();
        if self.size == size && self.scale == scale {
            return;
        }
        self.size = size;
        self.scale = scale;
        let _transaction = Transaction::new();
        #[cfg(target_os = "macos")]
        self.surface.layer().setContentsScale(scale);
        keep_previous_image_unscaled(self.surface.layer());
        self.surface
            .layer()
            .setDrawableSize(CGSize::new(f64::from(size.width), f64::from(size.height)));
    }

    pub(super) fn present(&self, drawable: ScheduledMetalDrawable) -> bool {
        if drawable.size() != (self.size.width, self.size.height) {
            return false;
        }
        #[cfg(target_os = "ios")]
        let view = {
            let RawWindowHandle::UiKit(handle) =
                self.window.window_handle().expect("UIKit window").as_raw()
            else {
                return false;
            };
            // SAFETY: the host and this transaction only run on the main thread.
            unsafe { &*handle.ui_view.as_ptr().cast::<objc2_ui_kit::UIView>() }
        };
        #[cfg(target_os = "ios")]
        {
            let bounds = view.bounds().size;
            let scale = view.contentScaleFactor();
            if !scale.is_finite()
                || scale <= 0.0
                || !bounds.width.is_finite()
                || !bounds.height.is_finite()
                || winit::dpi::LogicalSize::new(bounds.width, bounds.height).to_physical(scale)
                    != self.size
            {
                return false;
            }
        }
        let present = PhaseTimer::start();
        let transaction = Transaction::new();
        #[cfg(target_os = "ios")]
        {
            self.surface.layer().setFrame(view.bounds());
            self.surface
                .layer()
                .setContentsScale(view.contentScaleFactor());
        }
        drawable.present();
        drop(transaction);
        present.finish(FramePhase::Present);
        true
    }
}

impl Drop for MetalHost {
    fn drop(&mut self) {
        // The worker is stopped before this UI-owned host is removed.
        #[cfg(target_os = "macos")]
        {
            let RawWindowHandle::AppKit(handle) =
                self.window.window_handle().expect("AppKit window").as_raw()
            else {
                return;
            };
            // SAFETY: the window retains this view and the host is UI-owned.
            let view = unsafe { &*handle.ns_view.as_ptr().cast::<objc2_app_kit::NSView>() };
            let owned: &objc2_quartz_core::CALayer = self.surface.layer();
            if view
                .layer()
                .is_some_and(|layer| std::ptr::eq(&*layer, owned))
            {
                view.setLayer(None);
                view.setWantsLayer(false);
            }
        }
        #[cfg(target_os = "ios")]
        self.surface.layer().removeFromSuperlayer();
    }
}

struct Transaction(std::marker::PhantomData<*mut ()>);
impl Transaction {
    fn new() -> Self {
        assert!(
            objc2::MainThreadMarker::new().is_some(),
            "Core Animation transaction requires the main thread"
        );
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        Self(std::marker::PhantomData)
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        CATransaction::commit();
    }
}

impl MetalSeed {
    pub(super) fn create(self) -> Result<MetalPresenter, String> {
        let renderer = RendererImpl::with_antialiasing(&self.backend, FORMAT, self.antialiasing);
        Ok(MetalPresenter {
            backend: self.backend,
            source: self.source,
            renderer,
        })
    }
}

impl WorkerPresenter for MetalPresenter {
    type Output = BackendOutput;
    fn resize(&mut self, _: PhysicalSize<u32>) {}
    fn max_texture_dimension(&self) -> u32 {
        self.backend.limits().max_texture_dimension_2d
    }
    fn render(&mut self, packet: &FramePacket) -> BackendOutput {
        objc2::rc::autoreleasepool(|_| {
            let acquire = PhaseTimer::start();
            let drawable = self.source.next_drawable();
            acquire.finish_acquire();
            let Some(drawable) = drawable else {
                self.renderer.discard_frame(&self.backend, packet);
                return BackendOutput::Presented(false);
            };
            if drawable.size() != (packet.frame().width, packet.frame().height) {
                self.renderer.discard_frame(&self.backend, packet);
                return BackendOutput::Presented(false);
            }
            let encode = PhaseTimer::start();
            #[cfg(feature = "frame-stats")]
            self.backend
                .begin_gpu_frame_timing(aimer_frame_stats::record_gpu_frame_time);
            self.renderer
                .render_packet(&self.backend, drawable.view(), packet, true);
            encode.finish(FramePhase::Encode);
            #[cfg(feature = "frame-stats")]
            self.backend.end_gpu_frame_timing();
            BackendOutput::Metal(drawable.schedule_transactional_presentation(&self.backend))
        })
    }
}

/// Keeps the last presented image at its own pixel scale while the layer is resized.
///
/// AppKit and UIKit resize the layer immediately, but the worker presents a
/// replacement drawable of the new size only afterwards. With the default
/// `resize` gravity the compositor stretches the old image across the new
/// bounds in between, which is visible as content scaling during a live resize.
///
/// The image is pinned to the *on-screen* top-left corner, the origin the UI
/// lays out from. AppKit's layer is not flipped, so there the constant for that
/// corner is `kCAGravityBottomLeft`: `kCAGravityTopLeft` pinned the image to the
/// bottom edge, which pushed it down (or cut off its top) while the height
/// changed, as seen in a live-resize screen recording.
fn keep_previous_image_unscaled(layer: &CAMetalLayer) {
    // SAFETY: QuartzCore exports these immutable gravity constants.
    #[cfg(target_os = "macos")]
    layer.setContentsGravity(unsafe { objc2_quartz_core::kCAGravityBottomLeft });
    #[cfg(not(target_os = "macos"))]
    layer.setContentsGravity(unsafe { objc2_quartz_core::kCAGravityTopLeft });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_does_not_stretch_the_previous_image() {
        let layer = CAMetalLayer::new();
        // SAFETY: QuartzCore exports these immutable gravity constants.
        let (top_left_on_screen, wrong_edge) = unsafe {
            if cfg!(target_os = "macos") {
                (
                    objc2_quartz_core::kCAGravityBottomLeft,
                    objc2_quartz_core::kCAGravityTopLeft,
                )
            } else {
                (
                    objc2_quartz_core::kCAGravityTopLeft,
                    objc2_quartz_core::kCAGravityBottomLeft,
                )
            }
        };
        assert_ne!(&*layer.contentsGravity(), top_left_on_screen);
        keep_previous_image_unscaled(&layer);
        assert_eq!(&*layer.contentsGravity(), top_left_on_screen);
        assert_ne!(&*layer.contentsGravity(), wrong_edge);
    }
}
