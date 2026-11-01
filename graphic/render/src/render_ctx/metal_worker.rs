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
use std::sync::atomic::{AtomicBool, Ordering};
use winit::window::Window;

use super::native_ctx::BackendOutput;
use super::present_gate::{PresentGate, PresentMode};
use aimer_frame_stats::{FramePhase, PhaseTimer};
use crate::raster::WorkerPresenter;

const FORMAT: MTLPixelFormat = MTLPixelFormat::BGRA8Unorm_sRGB;

pub(super) struct MetalHost {
    surface: MetalSurface,
    window: Arc<Window>,
    size: PhysicalSize<u32>,
    scale: f64,
    /// Whether the raster thread may present on its own (see [`PresentGate`]).
    gate: PresentGate,
    /// The gate's answer for the raster thread: `true` while frames go direct.
    /// Written only here, so the layer's `presentsWithTransaction` and this
    /// flag change together, in the order the worker relies on.
    direct: Arc<AtomicBool>,
}

pub(super) struct MetalSeed {
    backend: MetalBackend,
    source: MetalDrawableSource,
    antialiasing: AntiAlias,
    direct: Arc<AtomicBool>,
}

pub(super) struct MetalPresenter {
    backend: MetalBackend,
    source: MetalDrawableSource,
    renderer: RendererImpl<MetalBackend>,
    direct: Arc<AtomicBool>,
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
        let direct = Arc::new(AtomicBool::new(false));
        // `AIMER_TRANSACTIONAL_PRESENT` keeps every frame on the transaction
        // path, for a device where direct presentation misbehaves.
        let allowed = PresentGate::platform_allows_direct()
            && std::env::var_os("AIMER_TRANSACTIONAL_PRESENT").is_none();
        let mut host = Self {
            surface,
            window: window.clone(),
            size: PhysicalSize::new(0, 0),
            scale: 0.0,
            gate: PresentGate::new(allowed),
            direct: direct.clone(),
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
                direct,
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
        // The layer's geometry is about to change, so the frames that follow
        // must reach the screen together with it.
        if self.gate.geometry_changed().is_some() {
            self.direct.store(false, Ordering::Release);
            self.surface.layer().setPresentsWithTransaction(true);
        }
        let _transaction = Transaction::new();
        #[cfg(target_os = "macos")]
        self.surface.layer().setContentsScale(scale);
        keep_previous_image_unscaled(self.surface.layer());
        self.surface
            .layer()
            .setDrawableSize(CGSize::new(f64::from(size.width), f64::from(size.height)));
    }

    pub(super) fn present(&mut self, drawable: ScheduledMetalDrawable) -> bool {
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
        aimer_utils::frame_trace("present_begin", 0.0, 0.0);
        let transaction = Transaction::new();
        #[cfg(target_os = "ios")]
        {
            self.surface.layer().setFrame(view.bounds());
            self.surface
                .layer()
                .setContentsScale(view.contentScaleFactor());
        }
        drawable.present();
        aimer_utils::frame_trace("present_called", 0.0, 0.0);
        drop(transaction);
        aimer_utils::frame_trace("present_committed", 0.0, 0.0);
        present.finish(FramePhase::Present);
        if self.gate.transactional_frame_presented() == Some(PresentMode::Direct) {
            // The layer first stops waiting for a transaction, then the worker
            // is told: a frame is never encoded for direct presentation while
            // the layer would still hold it for a transaction.
            self.surface.layer().setPresentsWithTransaction(false);
            self.direct.store(true, Ordering::Release);
        }
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
            direct: self.direct,
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
            // Read once: the mode this frame is acquired, encoded and presented in.
            let direct = self.direct.load(Ordering::Acquire);
            let acquire = PhaseTimer::start();
            let drawable = if direct {
                self.source.next_direct_drawable()
            } else {
                self.source.next_drawable()
            };
            acquire.finish_acquire();
            aimer_utils::frame_trace("raster_acquired", 0.0, 0.0);
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
            aimer_utils::frame_trace("raster_encoded", 0.0, 0.0);
            if direct {
                // No wait for the GPU and no main-thread transaction.
                drawable.present(&self.backend);
                aimer_utils::frame_trace("raster_presented", 0.0, 0.0);
                return BackendOutput::Presented(true);
            }
            let scheduled = drawable.schedule_transactional_presentation(&self.backend);
            aimer_utils::frame_trace("raster_scheduled", 0.0, 0.0);
            BackendOutput::Metal(scheduled)
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
