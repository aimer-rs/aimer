#[cfg(not(target_arch = "wasm32"))]
pub mod render_ctx {
    use aimer_cupid::AntiAlias;
    use aimer_cupid::backend::wgpu::WgpuBackend;
    use aimer_cupid::canvas::CupidCanvas;
    use aimer_cupid::compositor::CompositorScene;
    use aimer_cupid::damage_region::DamageSet;
    use aimer_cupid::frame::{Frame, FramePacket, FrameRenderMetadata};
    use aimer_cupid::gpu_context::{GpuContext, render_dimensions};
    use aimer_cupid::renderer::Renderer;
    use winit::dpi::PhysicalSize;
    use std::sync::Arc;
    use winit::window::Window;

    use aimer_frame_stats::{FramePhase, PhaseTimer};
    use crate::raster::FramePresenter;
    use crate::raster::{FrameWorker, WorkerPresenter};
    use crate::render_ctx::PresentOutcome;
    use crate::render_ctx::native_ctx::{BackendOutput, wake_event_loop};

    /// The UI thread's copy of the surface dimensions.
    ///
    /// The widget walk needs the size of the surface it is painting for, but the
    /// [`GpuContext`] that knows it may have been moved to the raster thread. So
    /// the size is mirrored here and kept in agreement with the context by
    /// running every requested size through [`render_dimensions`], exactly as
    /// [`GpuContext::resize`] does.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    struct SurfaceSize {
        size: PhysicalSize<u32>,
        max_dimension: u32,
    }

    impl SurfaceSize {
        #[inline]
        fn new(size: PhysicalSize<u32>, max_dimension: u32) -> Self {
            Self {
                size,
                max_dimension,
            }
        }

        /// Mirror a resize request, returning whether the backing size changed.
        ///
        /// A zero-sized request is ignored, matching [`GpuContext::resize`]:
        /// minimising a window must not reconfigure the surface to nothing.
        #[inline]
        fn resize(&mut self, size: PhysicalSize<u32>) -> bool {
            if size.width == 0 || size.height == 0 {
                return false;
            }
            let backing = render_dimensions(size, self.max_dimension);
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

    /// Whether deferred off-screen text preparation should keep the window
    /// rendering after this frame.
    ///
    /// Off-screen preparation is opportunistic: the next real frame (scroll,
    /// animation, input, or another state change) prepares whatever has become
    /// visible. Treating the deferred tail as its own frame source keeps a
    /// settled page in a render loop while it prepares content the user cannot
    /// see, which is especially expensive on native debug Metal builds.
    #[inline]
    fn should_request_text_preparation_frame(_has_postponed_text_preparation: bool) -> bool {
        false
    }

    /// Everything downstream of the widget walk: the GPU context and the
    /// renderer. This owner is constructed on whichever thread uses it;
    /// custom pipelines do not need to implement `Send`.
    struct GpuPresenter {
        gpu: GpuContext<'static>,
        backend: WgpuBackend,
        renderer: Renderer<WgpuBackend>,
    }

    impl Drop for GpuPresenter {
        fn drop(&mut self) {
            self.renderer.save_pipeline_cache(&self.backend);
        }
    }

    impl GpuPresenter {
        /// The size the surface is currently configured for.
        #[inline]
        fn surface_size(&self) -> PhysicalSize<u32> {
            PhysicalSize::new(self.gpu.width(), self.gpu.height())
        }
    }

    impl FramePresenter for GpuPresenter {
        fn resize(&mut self, size: PhysicalSize<u32>) {
            self.gpu.resize(size);
        }

        fn present(&mut self, frame: &Frame) -> bool {
            self.present_inner(frame, None)
        }

        fn present_packet(&mut self, packet: &FramePacket) -> bool {
            self.present_inner(packet.frame(), Some(packet))
        }
    }

    impl GpuPresenter {
        fn present_inner(&mut self, frame: &Frame, packet: Option<&FramePacket>) -> bool {
            let acquire = PhaseTimer::start();
            let surface = match self.gpu.begin_frame() {
                wgpu::CurrentSurfaceTexture::Success(texture)
                | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
                // Nothing was encoded, so nothing is worth timing: an outdated
                // surface would otherwise show up as a suspiciously cheap frame.
                _ => return false,
            };

            acquire.finish_acquire();
            let encode = PhaseTimer::start();
            let view = surface.texture.create_view(&Default::default());

            if self
                .gpu
                .config
                .usage
                .contains(wgpu::TextureUsages::COPY_SRC)
            {
                if let Some(packet) = packet {
                    self.renderer.render_packet_with_source_texture(
                        &self.backend,
                        &view,
                        &surface.texture,
                        packet,
                        self.gpu.is_srgb,
                    );
                } else {
                    self.renderer.render_with_source_texture(
                        &self.backend,
                        &view,
                        Some(&surface.texture),
                        frame.width,
                        frame.height,
                        self.gpu.is_srgb,
                        &frame.draw_list,
                    );
                }
            } else if let Some(packet) = packet {
                self.renderer
                    .render_packet(&self.backend, &view, packet, self.gpu.is_srgb);
            } else {
                self.renderer.render(
                    &self.backend,
                    &view,
                    frame.width,
                    frame.height,
                    self.gpu.is_srgb,
                    &frame.draw_list,
                );
            }
            encode.finish(FramePhase::Encode);

            // Deferred off-screen text is advisory and must not keep a settled
            // page repainting. A later scroll or other real frame will prepare
            // the text that has become visible.
            if should_request_text_preparation_frame(self.renderer.has_postponed_text_preparation())
            {
                aimer_events::window::request_animation_frame();
            }

            let present = PhaseTimer::start();
            self.gpu.end_frame(surface);
            present.finish(FramePhase::Present);

            true
        }
    }

    // Surface creation stays on the UI thread; pipeline construction and all
    // subsequent surface use stay with the worker, without requiring Renderer: Send.
    pub(in crate::render_ctx) fn start_worker(
        window: &Arc<Window>,
        size: PhysicalSize<u32>,
        antialiasing: AntiAlias,
    ) -> FrameWorker<BackendOutput> {
        let gpu = GpuContext::initialize(window.clone(), size);
        // WGPU presents on the worker, outside UI Core Animation transactions.
        FrameWorker::spawn(
            move || {
                let backend = gpu.backend();
                let renderer = Renderer::with_antialiasing(&backend, gpu.format, antialiasing);
                Ok(GpuPresenter {
                    gpu,
                    backend,
                    renderer,
                })
            },
            wake_event_loop,
        )
    }

    impl WorkerPresenter for GpuPresenter {
        type Output = BackendOutput;
        fn resize(&mut self, size: PhysicalSize<u32>) {
            FramePresenter::resize(self, size);
        }
        fn max_texture_dimension(&self) -> u32 {
            self.gpu.max_texture_dimension()
        }
        fn render(&mut self, packet: &FramePacket) -> BackendOutput {
            let presented = FramePresenter::present_packet(self, packet);
            if !presented {
                self.renderer.discard_frame(&self.backend, packet);
            }
            BackendOutput::Presented(presented)
        }
    }

    pub struct WgpuApi {
        presentation: Option<GpuPresenter>,
        canvas: Option<CupidCanvas>,
        surface_size: SurfaceSize,
        antialiasing: AntiAlias,
        surface_identity: u64,
        renderer_generation: u64,
        context_generation: u64,
        resource_generation: u64,
    }

    impl Default for WgpuApi {
        #[inline]
        fn default() -> Self {
            Self::new(AntiAlias::default())
        }
    }

    impl WgpuApi {
        #[inline]
        pub fn new(antialiasing: AntiAlias) -> Self {
            Self {
                presentation: None,
                canvas: None,
                surface_size: SurfaceSize::default(),
                antialiasing,
                surface_identity: 1,
                renderer_generation: 1,
                context_generation: 1,
                resource_generation: 0,
            }
        }

        /// Returns true when the GPU context has been initialized and is
        /// usable.
        #[inline]
        pub fn is_ready(&self) -> bool {
            self.presentation.is_some()
        }

        /// Whether frames are handed to the raster thread instead of being
        /// presented inline.
        ///
        /// This explicit adapter is inline; the application context selects runtime offloading.
        #[inline]
        pub fn is_offloaded(&self) -> bool {
            false
        }

        /// Create the GPU context, the renderer and the canvas.
        ///
        /// Everything here runs on the main thread on purpose. Surface creation
        /// and, on macOS, `enable_transactional_surface_presentation` are AppKit
        /// calls, and AppKit is main-thread-only; only the encode and present
        /// steps are eligible to move to the raster thread afterwards.
        pub fn initialize(&mut self, window: &Arc<Window>, size: PhysicalSize<u32>) {
            if self.presentation.is_some() {
                self.resize(size);
                return;
            }

            let gpu = GpuContext::initialize(window.clone(), size);
            #[cfg(target_os = "macos")]
            aimer_platform::ffi_utils::macos_surface::enable_transactional_surface_presentation(window);
            let canvas = CupidCanvas::new();
            let backend = gpu.backend();
            let renderer = Renderer::with_antialiasing(&backend, gpu.format, self.antialiasing);

            let presenter = GpuPresenter {
                gpu,
                backend,
                renderer,
            };
            self.surface_size = SurfaceSize::new(
                presenter.surface_size(),
                presenter.gpu.max_texture_dimension(),
            );

            self.presentation = Some(presenter);
            self.canvas = Some(canvas);
        }

        pub fn resize(&mut self, size: PhysicalSize<u32>) {
            if !self.surface_size.resize(size) {
                return;
            }
            self.resource_generation = self.resource_generation.wrapping_add(1);

            if let Some(presenter) = &mut self.presentation {
                FramePresenter::resize(presenter, size);
            }
        }

        /// Record a frame by calling `draw_fn` with the canvas and the current
        /// surface dimensions, then present it.
        ///
        /// Equivalent to [`build_frame`] followed by [`present`].
        ///
        /// [`build_frame`]: WgpuApi::build_frame
        /// [`present`]: WgpuApi::present
        pub fn render_frame(
            &mut self,
            draw_fn: impl FnOnce(&CupidCanvas, u32, u32),
        ) -> PresentOutcome {
            match self.build_frame_packet(|canvas, width, height| {
                draw_fn(canvas, width, height);
                (1.0, DamageSet::full(width, height))
            }) {
                Some(packet) => self.present_packet(packet),
                None => PresentOutcome::Dropped,
            }
        }

        /// Record a frame with the widget damage contract and present it.
        pub fn render_frame_packet(
            &mut self,
            draw_fn: impl FnOnce(&CupidCanvas, u32, u32) -> (f32, DamageSet),
        ) -> PresentOutcome {
            match self.build_frame_packet(draw_fn) {
                Some(packet) => self.present_packet(packet),
                None => PresentOutcome::Dropped,
            }
        }

        /// Record a frame without touching the swap chain.
        ///
        /// The widget walk runs here, entirely on the CPU. Keeping it out of
        /// [`present`] means no surface texture is held while the tree is being
        /// built and laid out, which is what shrinks the window in which a vsync
        /// deadline can be missed. The returned [`Frame`] is `Send`, so it can
        /// also be handed to a raster thread instead of presented inline.
        ///
        /// Returns `None` before the GPU context has been initialized.
        ///
        /// [`present`]: WgpuApi::present
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

        /// Record a frame and retain the damage information returned by the
        /// widget walk.
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
            let scene = render_plan.is_none().then(|| {
                CompositorScene::from_draw_list(
                    &frame.draw_list,
                    width,
                    height,
                    metadata.damage().clone(),
                )
            });
            build.finish(FramePhase::Build);

            Some(FramePacket::with_render_plan(
                frame,
                metadata,
                scene,
                render_plan,
            ))
        }

        /// Presents a recorded frame inline and recycles its command buffer.
        /// A dropped frame requires a redraw; the application context provides
        /// the worker-backed path when runtime threading is selected.
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

        /// Presents a metadata-carrying frame packet inline.
        pub fn present_packet(&mut self, packet: FramePacket) -> PresentOutcome {
            let Some(presenter) = &mut self.presentation else {
                return PresentOutcome::Dropped;
            };
            let presented = FramePresenter::present_packet(presenter, &packet);
            if !presented {
                presenter
                    .renderer
                    .discard_frame(&presenter.backend, &packet);
            }
            if let Some(canvas) = &self.canvas {
                canvas.recycle_draw_list(packet.into_frame().into_draw_list());
            }
            PresentOutcome::from_presented(presented)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_surface_owner_can_cross_to_the_factory() {
            const fn assert_send<T: Send>() {}
            assert_send::<GpuContext<'static>>();
        }

        #[test]
        fn the_mirrored_size_matches_what_the_context_would_configure() {
            let mut surface = SurfaceSize::new(PhysicalSize::new(800, 600), 2048);

            assert!(surface.resize(PhysicalSize::new(3072, 1728)));

            assert_eq!(
                PhysicalSize::new(surface.width(), surface.height()),
                render_dimensions(PhysicalSize::new(3072, 1728), 2048)
            );
        }

        #[test]
        fn a_zero_sized_resize_is_ignored() {
            let mut surface = SurfaceSize::new(PhysicalSize::new(800, 600), 2048);

            assert!(!surface.resize(PhysicalSize::new(0, 600)));
            assert!(!surface.resize(PhysicalSize::new(800, 0)));

            assert_eq!(surface.width(), 800);
            assert_eq!(surface.height(), 600);
        }

        #[test]
        fn resizing_to_the_same_size_reports_no_change() {
            let mut surface = SurfaceSize::new(PhysicalSize::new(800, 600), 2048);

            assert!(!surface.resize(PhysicalSize::new(800, 600)));
            assert!(surface.resize(PhysicalSize::new(801, 600)));
        }

        #[test]
        fn a_context_less_api_is_not_ready_and_drops_frames() {
            let mut api = WgpuApi::new(AntiAlias::default());

            assert!(!api.is_ready());
            assert!(!api.is_offloaded());
            assert!(api.build_frame(|_, _, _| unreachable!()).is_none());
            assert_eq!(
                api.render_frame(|_, _, _| unreachable!()),
                PresentOutcome::Dropped
            );
        }

        #[test]
        fn postponed_off_screen_text_does_not_schedule_an_idle_frame() {
            assert!(!should_request_text_preparation_frame(true));
        }
    }
}
