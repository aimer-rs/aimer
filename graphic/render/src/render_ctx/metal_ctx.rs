#[cfg(all(
    not(target_arch = "wasm32"),
    any(target_os = "macos", target_os = "ios")
))]
pub mod render_ctx {
    use aimer_cupid::AntiAlias;
    use aimer_cupid::backend::GpuBackend;
    use aimer_cupid::backend::metal::{MetalBackend, MetalSurface};
    use aimer_cupid::canvas::CupidCanvas;
    use aimer_cupid::damage_region::DamageSet;
    use aimer_cupid::frame::{Frame, FramePacket, FrameRenderMetadata};
    use aimer_cupid::renderer::RendererImpl;
    #[cfg(target_os = "macos")]
    use objc2_app_kit::NSView;
    #[cfg(any(target_os = "ios", test))]
    use objc2_core_foundation::CGRect;
    use objc2_core_foundation::CGSize;
    use objc2_metal::MTLPixelFormat;
    use objc2_quartz_core::CAMetalLayer;
    #[cfg(any(target_os = "ios", test))]
    use objc2_quartz_core::CATransaction;
    #[cfg(target_os = "ios")]
    use objc2_ui_kit::UIView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::dpi::PhysicalSize;
    use std::sync::Arc;
    use winit::window::Window;

    use aimer_frame_stats::{FramePhase, PhaseTimer};
    use crate::render_ctx::PresentOutcome;

    const SURFACE_FORMAT: MTLPixelFormat = MTLPixelFormat::BGRA8Unorm_sRGB;

    /// The Quiver render context backed by a native Apple `CAMetalLayer`.
    ///
    /// This context owns the Metal backend, surface, and backend-generic Cupid
    /// renderer. Metal is selected on macOS by the native dependency feature or
    /// by explicitly enabling Quiver's `metal` feature.
    /// The presenter forwards packet damage to Cupid's retained-target
    /// renderer, which keeps its full-repaint fallback when reuse is unsafe.
    pub struct MetalApi {
        backend: Option<MetalBackend>,
        surface: Option<MetalSurface>,
        renderer: Option<RendererImpl<MetalBackend>>,
        canvas: Option<CupidCanvas>,
        window: Option<Arc<Window>>,
        surface_size: SurfaceSize,
        antialiasing: AntiAlias,
        surface_identity: u64,
        renderer_generation: u64,
        context_generation: u64,
        resource_generation: u64,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    struct SurfaceSize {
        size: PhysicalSize<u32>,
        max_dimension: u32,
    }

    impl SurfaceSize {
        #[inline]
        fn new(size: PhysicalSize<u32>, max_dimension: u32) -> Self {
            Self {
                size: Self::clamp(size, max_dimension),
                max_dimension,
            }
        }

        #[inline]
        fn clamp(size: PhysicalSize<u32>, max_dimension: u32) -> PhysicalSize<u32> {
            PhysicalSize::new(
                size.width.min(max_dimension),
                size.height.min(max_dimension),
            )
        }

        /// Mirror a resize request, returning whether the backing size changed.
        #[inline]
        fn resize(&mut self, size: PhysicalSize<u32>) -> bool {
            if size.width == 0 || size.height == 0 {
                return false;
            }
            let backing = Self::clamp(size, self.max_dimension);
            let changed = backing != self.size;
            self.size = backing;
            changed
        }

        #[inline]
        fn width(&self) -> u32 {
            self.size.width
        }

        #[inline]
        fn height(&self) -> u32 {
            self.size.height
        }
    }

    impl Default for MetalApi {
        #[inline]
        fn default() -> Self {
            Self::new(AntiAlias::default())
        }
    }

    impl MetalApi {
        /// Creates an uninitialized context with the requested antialiasing.
        #[inline]
        pub fn new(antialiasing: AntiAlias) -> Self {
            Self {
                backend: None,
                surface: None,
                renderer: None,
                canvas: None,
                window: None,
                surface_size: SurfaceSize::default(),
                antialiasing,
                surface_identity: 1,
                renderer_generation: 1,
                context_generation: 1,
                resource_generation: 0,
            }
        }

        /// Returns true after the Metal device, surface, and renderer are ready.
        #[inline]
        pub fn is_ready(&self) -> bool {
            self.backend.is_some() && self.surface.is_some() && self.renderer.is_some()
        }

        /// Metal frames are presented inline on the window's event-loop thread.
        #[inline]
        pub fn is_offloaded(&self) -> bool {
            false
        }

        /// Creates the Metal device, attaches a `CAMetalLayer`, and initializes
        /// Cupid's generic renderer. Surface setup runs on the main thread.
        pub fn initialize(&mut self, window: &Arc<Window>, size: PhysicalSize<u32>) {
            if self.is_ready() {
                self.window = Some(window.clone());
                self.resize(size);
                return;
            }

            let backend = MetalBackend::new().expect("create the system Metal device and queue");
            let raw_handle = window
                .window_handle()
                .expect("get the Apple window handle")
                .as_raw();
            #[cfg(target_os = "macos")]
            let view = {
                let RawWindowHandle::AppKit(appkit_handle) = raw_handle else {
                    panic!("winit returned a non-AppKit window handle on macOS");
                };
                // SAFETY: Winit owns this NSView for the lifetime of `window`.
                // The context keeps the window alive while the Metal layer is in use.
                unsafe { &*appkit_handle.ns_view.as_ptr().cast::<NSView>() }
            };
            #[cfg(target_os = "ios")]
            let view = {
                let RawWindowHandle::UiKit(uikit_handle) = raw_handle else {
                    panic!("winit returned a non-UIKit window handle on iOS");
                };
                // SAFETY: Winit owns this UIView for the lifetime of `window`.
                // The context keeps the window alive while the Metal layer is in use.
                unsafe { &*uikit_handle.ui_view.as_ptr().cast::<UIView>() }
            };
            let layer = CAMetalLayer::new();
            #[cfg(target_os = "macos")]
            view.setWantsLayer(true);
            #[cfg(target_os = "macos")]
            view.setLayer(Some(&layer));
            #[cfg(target_os = "ios")]
            {
                // Establish initial geometry before acquisition. There is no
                // previous drawable to preserve until the first presentation.
                layer.setFrame(view.bounds());
                layer.setContentsScale(view.contentScaleFactor());
                view.layer().addSublayer(&layer);
            }

            let surface = MetalSurface::new(&backend, layer, SURFACE_FORMAT);
            let max_dimension = backend.limits().max_texture_dimension_2d;
            let surface_size = SurfaceSize::new(size, max_dimension);
            #[cfg(target_os = "macos")]
            configure_surface(&surface, window, surface_size.size);
            #[cfg(target_os = "macos")]
            aimer_platform::ffi_utils::macos_surface::enable_transactional_surface_presentation(window);
            let renderer =
                RendererImpl::with_antialiasing(&backend, SURFACE_FORMAT, self.antialiasing);

            self.backend = Some(backend);
            self.surface = Some(surface);
            self.renderer = Some(renderer);
            self.canvas = Some(CupidCanvas::new());
            self.window = Some(window.clone());
            self.surface_size = surface_size;
        }

        /// Resizes the drawable layer to the current physical window dimensions.
        ///
        /// On iOS the layer update is applied with the next presentation so
        /// UIKit's geometry and the matching drawable commit together.
        pub fn resize(&mut self, size: PhysicalSize<u32>) {
            if size.width == 0 || size.height == 0 {
                return;
            }

            if self.surface_size.resize(size) {
                self.resource_generation = self.resource_generation.wrapping_add(1);
            }

            #[cfg(target_os = "macos")]
            if let (Some(surface), Some(window)) = (&self.surface, &self.window) {
                configure_surface(surface, window, self.surface_size.size);
            }
        }

        /// Records and presents a full-damage frame.
        pub fn render_frame(
            &mut self,
            draw_fn: impl FnOnce(&CupidCanvas, u32, u32),
        ) -> PresentOutcome {
            self.render_frame_packet(|canvas, width, height| {
                draw_fn(canvas, width, height);
                (1.0, DamageSet::full(width, height))
            })
        }

        /// Records and presents a frame with damage metadata from the widget walk.
        pub fn render_frame_packet(
            &mut self,
            draw_fn: impl FnOnce(&CupidCanvas, u32, u32) -> (f32, DamageSet),
        ) -> PresentOutcome {
            let cpu_frame = PhaseTimer::start();
            match self.build_frame_packet(draw_fn) {
                Some(packet) => self.present_packet_timed(packet, Some(cpu_frame)),
                None => PresentOutcome::Dropped,
            }
        }

        /// Records a frame without acquiring a Metal drawable.
        pub fn build_frame(
            &mut self,
            draw_fn: impl FnOnce(&CupidCanvas, u32, u32),
        ) -> Option<Frame> {
            self.build_frame_packet(|canvas, width, height| {
                draw_fn(canvas, width, height);
                (1.0, DamageSet::full(width, height))
            })
            .map(FramePacket::into_frame)
        }

        /// Records a frame packet without acquiring a Metal drawable.
        pub fn build_frame_packet(
            &mut self,
            draw_fn: impl FnOnce(&CupidCanvas, u32, u32) -> (f32, DamageSet),
        ) -> Option<FramePacket> {
            if !self.is_ready() {
                return None;
            }
            let canvas = self.canvas.as_ref()?;
            let width = self.surface_size.width();
            let height = self.surface_size.height();

            let build = PhaseTimer::start();
            canvas.begin_frame();
            let (scale, damage) = draw_fn(canvas, width, height);
            let render_plan = canvas.take_retained_render_plan();
            let damage = if damage.target_size() == (width, height) {
                damage
            } else {
                DamageSet::full(width, height)
            };
            let draw_list = canvas.take_draw_list();
            let frame = Frame::new(draw_list, width, height);
            let metadata = FrameRenderMetadata::new(
                scale,
                self.surface_identity,
                self.renderer_generation,
                self.context_generation,
                self.resource_generation,
                damage,
            );
            build.finish(FramePhase::Build);

            Some(FramePacket::with_render_plan(
                frame,
                metadata,
                None,
                render_plan,
            ))
        }

        /// Presents a frame and returns its draw-list storage to the canvas.
        pub fn present(&mut self, frame: Frame) -> PresentOutcome {
            let metadata = FrameRenderMetadata::new(
                1.0,
                self.surface_identity,
                self.renderer_generation,
                self.context_generation,
                self.resource_generation,
                DamageSet::full(frame.width, frame.height),
            );
            self.present_packet(FramePacket::new(frame, metadata))
        }

        /// Presents a frame packet and returns its draw-list storage to the canvas.
        pub fn present_packet(&mut self, packet: FramePacket) -> PresentOutcome {
            self.present_packet_timed(packet, None)
        }

        fn present_packet_timed(
            &mut self,
            packet: FramePacket,
            cpu_frame: Option<PhaseTimer>,
        ) -> PresentOutcome {
            let presented = self.present_inner(&packet, cpu_frame);
            if !presented {
                if let (Some(backend), Some(renderer)) = (&self.backend, &mut self.renderer) {
                    renderer.discard_frame(backend, &packet);
                }
            }
            if let Some(canvas) = &self.canvas {
                canvas.recycle_draw_list(packet.into_frame().into_draw_list());
            }
            PresentOutcome::from_presented(presented)
        }

        fn present_inner(&mut self, packet: &FramePacket, cpu_frame: Option<PhaseTimer>) -> bool {
            let (Some(backend), Some(surface), Some(renderer)) = (
                self.backend.as_ref(),
                self.surface.as_ref(),
                self.renderer.as_mut(),
            ) else {
                return false;
            };
            let frame = packet.frame();
            #[cfg(target_os = "ios")]
            let _surface_update = {
                let Some(window) = self.window.as_ref() else {
                    return false;
                };
                let Some(update) = configure_uikit_surface(
                    surface,
                    window,
                    PhysicalSize::new(frame.width, frame.height),
                ) else {
                    return false;
                };
                update
            };
            let acquire = PhaseTimer::start();
            let acquired = surface.next_drawable();
            acquire.finish_acquire();
            let Some(drawable) = acquired else {
                return false;
            };
            let (width, height) = drawable.size();
            if width == 0 || height == 0 {
                return false;
            }
            #[cfg(target_os = "ios")]
            if !_surface_update.matches_drawable((width, height)) {
                return false;
            }

            let encode = PhaseTimer::start();
            #[cfg(feature = "frame-stats")]
            backend.begin_gpu_frame_timing(aimer_frame_stats::record_gpu_frame_time);
            if (width, height) == (frame.width, frame.height) {
                renderer.render_packet(backend, drawable.view(), packet, true);
            } else {
                // A resize can race frame recording. Damage belongs to the
                // packet's target dimensions, so redraw its complete retained
                // plan against the drawable's current size.
                renderer.render_packet_at_size(
                    backend,
                    drawable.view(),
                    packet,
                    width,
                    height,
                    true,
                );
            }
            encode.finish(FramePhase::Encode);
            #[cfg(feature = "frame-stats")]
            {
                backend.end_gpu_frame_timing();
            }
            if let Some(cpu_frame) = cpu_frame {
                cpu_frame.finish_cpu_frame();
            }

            let present = PhaseTimer::start();
            #[cfg(target_os = "ios")]
            _surface_update.apply_geometry();
            drawable.present(backend);
            #[cfg(target_os = "ios")]
            drop(_surface_update);
            present.finish(FramePhase::Present);
            true
        }
    }

    impl Drop for MetalApi {
        fn drop(&mut self) {
            let Some(surface) = &self.surface else { return };
            #[cfg(target_os = "macos")]
            {
                let Some(window) = self.window.as_ref() else { return };
                let Ok(handle) = window.window_handle() else {
                    return;
                };
                let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                    return;
                };
                // SAFETY: the inline context owns the layer on its UI thread;
                // winit keeps the view alive for the retained window's lifetime.
                let view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
                let owned: &objc2_quartz_core::CALayer = surface.layer();
                if view
                    .layer()
                    .is_some_and(|layer| std::ptr::eq(&*layer, owned))
                {
                    view.setLayer(None);
                    view.setWantsLayer(false);
                }
            }
            #[cfg(target_os = "ios")]
            surface.layer().removeFromSuperlayer();
        }
    }

    #[cfg(target_os = "ios")]
    fn configure_uikit_surface<'a>(
        surface: &'a MetalSurface,
        window: &Window,
        size: PhysicalSize<u32>,
    ) -> Option<SurfaceTransaction<'a>> {
        let raw_handle = window
            .window_handle()
            .expect("get the iOS window handle")
            .as_raw();
        let RawWindowHandle::UiKit(uikit_handle) = raw_handle else {
            panic!("winit returned a non-UIKit window handle on iOS");
        };
        // SAFETY: Winit owns this UIView for the window's lifetime, and the
        // inline Metal presenter runs on winit's main event-loop thread.
        let view = unsafe { &*uikit_handle.ui_view.as_ptr().cast::<UIView>() };
        configure_uikit_layer(
            surface.layer(),
            view.bounds(),
            view.contentScaleFactor(),
            size,
        )
    }

    #[cfg(target_os = "macos")]
    fn configure_surface(surface: &MetalSurface, window: &Window, size: PhysicalSize<u32>) {
        surface.layer().setContentsScale(window.scale_factor());
        surface
            .layer()
            .setDrawableSize(CGSize::new(f64::from(size.width), f64::from(size.height)));
    }

    // The guard spans layer configuration, drawable acquisition, encoding and
    // direct presentation. Early returns and unwinding still close the transaction.
    #[cfg(any(target_os = "ios", test))]
    #[must_use = "keep the transaction alive until the drawable has been presented"]
    struct SurfaceTransaction<'a> {
        layer: &'a CAMetalLayer,
        bounds: CGRect,
        scale: f64,
        size: PhysicalSize<u32>,
        // Core Animation transactions are thread-local; commit on the same thread.
        _thread: std::marker::PhantomData<*mut ()>,
    }

    #[cfg(any(target_os = "ios", test))]
    impl SurfaceTransaction<'_> {
        fn matches_drawable(&self, size: (u32, u32)) -> bool {
            size == (self.size.width, self.size.height)
        }

        fn apply_geometry(&self) {
            self.layer.setFrame(self.bounds);
            self.layer.setContentsScale(self.scale);
        }
    }

    #[cfg(any(target_os = "ios", test))]
    impl Drop for SurfaceTransaction<'_> {
        fn drop(&mut self) {
            CATransaction::commit();
        }
    }

    #[cfg(any(target_os = "ios", test))]
    fn configure_uikit_layer(
        layer: &CAMetalLayer,
        bounds: CGRect,
        scale: f64,
        size: PhysicalSize<u32>,
    ) -> Option<SurfaceTransaction<'_>> {
        let viewport = bounds.size;
        if !viewport.width.is_finite()
            || viewport.width <= 0.0
            || !viewport.height.is_finite()
            || viewport.height <= 0.0
            || !scale.is_finite()
            || scale <= 0.0
            || winit::dpi::LogicalSize::new(viewport.width, viewport.height).to_physical(scale)
                != size
        {
            return None;
        }
        // A bare sublayer implicitly animates its geometry unless actions are
        // disabled. Keep it at the UIView's current bounds throughout resize.
        CATransaction::begin();
        let transaction = SurfaceTransaction {
            layer,
            bounds,
            scale,
            size,
            _thread: std::marker::PhantomData,
        };
        CATransaction::setDisableActions(true);
        // UIKit commits its window geometry through Core Animation. Present
        // the drawable in that same transaction, otherwise the compositor can
        // stretch an old drawable into the new bounds (or clip a new one into
        // the old bounds). MetalSurfaceFrame already waits for scheduling and
        // calls drawable.present() directly for transactional layers.
        layer.setPresentsWithTransaction(true);
        // Until the replacement drawable arrives, retain the previous image's
        // pixel scale instead of stretching it into the resized layer bounds.
        // SAFETY: QuartzCore exports this immutable gravity constant.
        layer.setContentsGravity(unsafe { objc2_quartz_core::kCAGravityTopLeft });
        // Request the replacement texture first. The visible geometry changes
        // only after acquiring and rendering a drawable of exactly this size.
        layer.setDrawableSize(CGSize::new(f64::from(size.width), f64::from(size.height)));
        Some(transaction)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use objc2_core_foundation::CGPoint;
        use objc2_quartz_core::kCAGravityTopLeft;

        #[test]
        fn uikit_resize_does_not_commit_new_bounds_without_a_matching_drawable() {
            let layer = CAMetalLayer::new();
            let previous = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 80.0));
            layer.setFrame(previous);
            let next = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(140.0, 100.0));
            // Acquiring a drawable can fail during resize. Merely requesting
            // a new drawable must not resize the image still on screen.
            drop(configure_uikit_layer(
                &layer,
                next,
                2.0,
                PhysicalSize::new(280, 200),
            ));
            assert_eq!(layer.frame(), previous);
        }

        #[test]
        fn uikit_resize_rejects_a_frame_recorded_before_the_view_changed_size() {
            let layer = CAMetalLayer::new();
            let previous_drawable_size = layer.drawableSize();
            let previous = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(100.0, 80.0));
            layer.setFrame(previous);
            let next = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(140.0, 100.0));
            drop(configure_uikit_layer(
                &layer,
                next,
                2.0,
                PhysicalSize::new(200, 160),
            ));
            assert_eq!(layer.frame(), previous);
            assert_eq!(layer.drawableSize(), previous_drawable_size);
        }

        #[test]
        fn uikit_resize_accepts_only_the_requested_drawable_dimensions() {
            let layer = CAMetalLayer::new();
            let bounds = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(140.0, 100.0));
            let update = configure_uikit_layer(&layer, bounds, 2.0, PhysicalSize::new(280, 200))
                .expect("matching viewport");
            assert!(update.matches_drawable((280, 200)));
            for stale in [(200, 160), (280, 160), (200, 200), (0, 0)] {
                assert!(!update.matches_drawable(stale));
            }
        }

        #[test]
        fn uikit_resize_rejects_invalid_viewports_without_changing_the_layer() {
            let layer = CAMetalLayer::new();
            let previous = layer.frame();
            for (width, height, scale) in [
                (0.0, 100.0, 2.0),
                (140.0, 0.0, 2.0),
                (-140.0, 100.0, 2.0),
                (f64::NAN, 100.0, 2.0),
                (140.0, f64::INFINITY, 2.0),
                (140.0, 100.0, 0.0),
                (140.0, 100.0, f64::NAN),
                (140.0, 100.0, f64::INFINITY),
            ] {
                let bounds = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height));
                assert!(
                    configure_uikit_layer(&layer, bounds, scale, PhysicalSize::new(280, 200))
                        .is_none()
                );
                assert_eq!(layer.frame(), previous);
            }
        }

        #[test]
        fn uikit_resize_preserves_the_previous_drawables_pixel_scale() {
            let layer = CAMetalLayer::new();
            for (width, height) in [(800.0, 600.0), (1000.0, 700.0), (500.0, 400.0)] {
                let _update = configure_uikit_layer(
                    &layer,
                    CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height)),
                    2.0,
                    PhysicalSize::new((width * 2.0) as u32, (height * 2.0) as u32),
                )
                .expect("matching viewport");
                // SAFETY: QuartzCore exports this immutable gravity constant.
                assert_eq!(&*layer.contentsGravity(), unsafe { kCAGravityTopLeft });
            }
        }

        #[test]
        fn uikit_resize_keeps_its_transaction_open_until_presentation() {
            let layer = CAMetalLayer::new();
            CATransaction::begin();
            CATransaction::setDisableActions(false);
            let update = configure_uikit_layer(
                &layer,
                CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(512.0, 384.0)),
                2.0,
                PhysicalSize::new(1024, 768),
            )
            .expect("matching viewport");
            let open_during_presentation = CATransaction::disableActions();
            drop(update);
            let restored_after_presentation = !CATransaction::disableActions();
            CATransaction::commit();

            assert!(
                open_during_presentation,
                "geometry must not commit before presentation"
            );
            assert!(
                restored_after_presentation,
                "the caller's transaction must be restored"
            );
        }

        #[test]
        fn uikit_resize_synchronizes_geometry_and_metal_presentation() {
            let layer = CAMetalLayer::new();
            // MetalSurface starts with asynchronous command-buffer presentation.
            layer.setPresentsWithTransaction(false);
            for (width, height, scale) in [
                (1024.0, 768.0, 2.0),
                (512.0, 384.0, 2.0),
                (900.0, 700.0, 2.0),
                (700.0, 900.0, 3.0),
            ] {
                let bounds = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height));
                let size = PhysicalSize::new((width * scale) as u32, (height * scale) as u32);
                let update =
                    configure_uikit_layer(&layer, bounds, scale, size).expect("matching viewport");
                update.apply_geometry();

                assert!(
                    layer.presentsWithTransaction(),
                    "resize must present with UIKit's transaction"
                );
                assert_eq!(layer.frame(), bounds);
                assert_eq!(layer.contentsScale(), scale);
                assert_eq!(
                    layer.drawableSize(),
                    CGSize::new(size.width.into(), size.height.into())
                );
            }
        }
    }
}
