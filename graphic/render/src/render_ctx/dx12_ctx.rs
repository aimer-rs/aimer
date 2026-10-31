#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
pub mod render_ctx {
    use aimer_cupid::AntiAlias;
    use aimer_cupid::backend::GpuBackend;
    use aimer_cupid::backend::dx12::{Dx12Backend, Dx12Surface, Dx12TextureFormat};
    use aimer_cupid::canvas::CupidCanvas;
    use aimer_cupid::damage_region::DamageSet;
    use aimer_cupid::frame::{Frame, FramePacket, FrameRenderMetadata};
    use aimer_cupid::renderer::RendererImpl;
    use aimer_utils::info;
    use raw_window_handle::RawWindowHandle;
    use std::ffi::c_void;
    use aimer_time::Instant;
    use windows::Win32::Foundation::HWND;
    use winit::dpi::PhysicalSize;
    use winit::platform::windows::WindowExtWindows;
    use std::sync::Arc;
    use winit::window::Window;

    use aimer_frame_stats::{FramePhase, PhaseTimer};
    use crate::render_ctx::PresentOutcome;

    // The flip-model swap chain uses UNORM storage and an sRGB render-target view.
    const SURFACE_FORMAT: Dx12TextureFormat = Dx12TextureFormat::Bgra8UnormSrgb;

    /// Quiver's inline Windows presenter backed by a Direct3D 12 device.
    pub struct Dx12Api {
        backend: Option<Dx12Backend>,
        surface: Option<Dx12Surface>,
        renderer: Option<RendererImpl<Dx12Backend>>,
        canvas: Option<CupidCanvas>,
        window: Option<Arc<Window>>,
        surface_size: SurfaceSize,
        antialiasing: AntiAlias,
        surface_identity: u64,
        renderer_generation: u64,
        context_generation: u64,
        resource_generation: u64,
        startup_started: Option<Instant>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    struct SurfaceSize {
        size: PhysicalSize<u32>,
        max_dimension: u32,
    }

    impl SurfaceSize {
        fn new(size: PhysicalSize<u32>, max_dimension: u32) -> Self {
            Self {
                size: Self::clamp(size, max_dimension),
                max_dimension,
            }
        }

        fn clamp(size: PhysicalSize<u32>, max_dimension: u32) -> PhysicalSize<u32> {
            PhysicalSize::new(
                size.width.min(max_dimension),
                size.height.min(max_dimension),
            )
        }

        fn resize(&mut self, size: PhysicalSize<u32>) -> bool {
            if size.width == 0 || size.height == 0 {
                return false;
            }
            let size = Self::clamp(size, self.max_dimension);
            let changed = self.size != size;
            self.size = size;
            changed
        }

        fn width(&self) -> u32 {
            self.size.width
        }
        fn height(&self) -> u32 {
            self.size.height
        }
    }

    impl Default for Dx12Api {
        fn default() -> Self {
            Self::new(AntiAlias::default())
        }
    }

    impl Dx12Api {
        /// Creates an uninitialized Windows rendering context.
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
                startup_started: None,
            }
        }

        /// Returns true after device, swap chain, and renderer initialization.
        #[inline]
        pub fn is_ready(&self) -> bool {
            self.backend.is_some() && self.surface.is_some() && self.renderer.is_some()
        }

        /// Direct3D 12 frames are rendered inline on the event-loop thread.
        #[inline]
        pub fn is_offloaded(&self) -> bool {
            false
        }

        /// Creates a hardware D3D12 device, attaches a DXGI swap chain, and
        /// initializes Cupid's generic renderer.
        ///
        /// Runs on either the window thread (inline mode) or the raster worker.
        pub fn initialize(&mut self, window: &Arc<Window>, size: PhysicalSize<u32>) {
            if self.is_ready() {
                self.window = Some(window.clone());
                self.resize(size);
                return;
            }
            let startup_started = Instant::now();
            let backend_started = Instant::now();
            let backend =
                Dx12Backend::new().expect("create the system Direct3D 12 device and queue");
            // info!(
            //     "DX12 startup stage: device, command queue, and descriptor heaps {:.2} ms",
            //     backend_started.elapsed().as_secs_f64() * 1000.0
            // );
            // winit refuses `window_handle()` off the event-loop thread, but the
            // raster worker initializes this presenter on its own thread.
            // SAFETY: the HWND only reaches `CreateSwapChainForHwnd`, which DXGI
            // allows from any thread, and the `'static` window outlives it.
            let raw_handle = unsafe { window.window_handle_any_thread() }
                .expect("get the Windows window handle")
                .as_raw();
            let RawWindowHandle::Win32(win32_handle) = raw_handle else {
                panic!("winit returned a non-Win32 window handle on Windows");
            };
            let hwnd = HWND(win32_handle.hwnd.get() as *mut c_void);
            let max_dimension = backend.limits().max_texture_dimension_2d;
            let surface_size = SurfaceSize::new(size, max_dimension);
            let surface_started = Instant::now();
            let surface = Dx12Surface::new(
                &backend,
                hwnd,
                surface_size.width(),
                surface_size.height(),
                SURFACE_FORMAT,
            )
            .expect("create the DXGI window swap chain");
            // info!(
            //     "DX12 startup stage: swap chain and back buffers {:.2} ms",
            //     surface_started.elapsed().as_secs_f64() * 1000.0
            // );
            let renderer_started = Instant::now();
            let mut renderer =
                RendererImpl::with_antialiasing(&backend, SURFACE_FORMAT, self.antialiasing);
            renderer.enable_first_render_timing();
            // info!(
            //     "DX12 startup stage: built-in renderer pipelines (HLSL compile and PSO creation) {:.2} ms",
            //     renderer_started.elapsed().as_secs_f64() * 1000.0
            // );
            self.backend = Some(backend);
            self.surface = Some(surface);
            self.renderer = Some(renderer);
            self.canvas = Some(CupidCanvas::new());
            self.window = Some(window.clone());
            self.surface_size = surface_size;
            info!(
                "DX12 startup stage: render context initialization {:.2} ms",
                startup_started.elapsed().as_secs_f64() * 1000.0
            );
            self.startup_started = Some(startup_started);
        }

        /// Queues a DXGI buffer resize to the current physical client size.
        ///
        /// The swap chain applies the latest requested size when its previous
        /// GPU work has completed, so this event handler never waits for the GPU.
        pub fn resize(&mut self, size: PhysicalSize<u32>) {
            if size.width == 0 || size.height == 0 {
                return;
            }
            let changed = self.surface_size.resize(size);
            if changed {
                self.resource_generation = self.resource_generation.wrapping_add(1);
            }
            if let Some(surface) = &mut self.surface {
                surface.request_resize(self.surface_size.width(), self.surface_size.height());
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
            let startup_frame = self.startup_started.is_some();
            let build_started = startup_frame.then(Instant::now);
            let packet = self.build_frame_packet(draw_fn);
            // if let Some(build_started) = build_started {
            //     info!(
            //         "DX12 first-frame stage: widget build, layout, and draw-list recording {:.2} ms",
            //         build_started.elapsed().as_secs_f64() * 1000.0
            //     );
            // }

            let Some(packet) = packet else {
                // if startup_frame {
                //     info!("DX12 first-frame attempt: no frame packet was produced");
                // }
                return PresentOutcome::Dropped;
            };

            let present_started = startup_frame.then(Instant::now);
            let outcome = self.present_packet(packet);
            // if let Some(present_started) = present_started {
            //     info!(
            //         "DX12 first-frame stage: render and present path {:.2} ms (presented={})",
            //         present_started.elapsed().as_secs_f64() * 1000.0,
            //         outcome.is_presented()
            //     );
            // }
            outcome
        }

        /// Records a frame without acquiring a swap-chain buffer.
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

        /// Records a frame packet without acquiring a swap-chain buffer.
        pub fn build_frame_packet(
            &mut self,
            draw_fn: impl FnOnce(&CupidCanvas, u32, u32) -> (f32, DamageSet),
        ) -> Option<FramePacket> {
            if !self.is_ready() {
                return None;
            }
            let canvas = self.canvas.as_ref()?;
            let (width, height) = (self.surface_size.width(), self.surface_size.height());
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

        /// Presents a frame packet and recycles its draw-list storage.
        pub fn present_packet(&mut self, packet: FramePacket) -> PresentOutcome {
            let presented = self.present_worker_packet(&packet);
            if let Some(canvas) = &self.canvas {
                canvas.recycle_draw_list(packet.into_frame().into_draw_list());
            }
            let outcome = PresentOutcome::from_presented(presented);
            // if self.startup_started.is_some() && outcome.is_presented() {
            //     if let Some(started) = self.startup_started.take() {
            //         info!(
            //             "DX12 startup total: context initialization to first presented frame completion {:.2} ms",
            //             started.elapsed().as_secs_f64() * 1000.0
            //         );
            //     }
            // }
            outcome
        }

        /// Presents a frame and recycles its draw-list storage.
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

        pub(crate) fn worker_max_dimension(&self) -> u32 {
            self.backend.as_ref().map_or(u32::MAX, |backend| {
                backend.limits().max_texture_dimension_2d
            })
        }

        pub(crate) fn present_worker_packet(&mut self, packet: &FramePacket) -> bool {
            let presented = self.present_inner(packet);
            if !presented {
                if let (Some(backend), Some(renderer)) = (&self.backend, &mut self.renderer) {
                    renderer.discard_frame(backend, packet);
                }
            }
            presented
        }

        fn present_inner(&mut self, packet: &FramePacket) -> bool {
            let (Some(backend), Some(surface), Some(renderer)) = (
                self.backend.as_ref(),
                self.surface.as_mut(),
                self.renderer.as_mut(),
            ) else {
                return false;
            };
            let frame = packet.frame();
            let startup_timing = self.startup_started.is_some();
            let acquire_started = startup_timing.then(Instant::now);
            let acquire = PhaseTimer::start();
            let acquired = surface.try_acquire(backend);
            acquire.finish_acquire();
            // if let Some(acquire_started) = acquire_started {
            //     info!(
            //         "DX12 first-frame stage: swap-chain back-buffer acquisition {:.2} ms",
            //         acquire_started.elapsed().as_secs_f64() * 1000.0
            //     );
            // }
            let drawable = match acquired {
                Ok(Some(drawable)) => drawable,
                Ok(None) => return false,
                Err(error) => panic!("resize the DXGI swap chain: {error}"),
            };
            let (width, height) = drawable.size();
            if width == 0 || height == 0 {
                return false;
            }
            let encode = PhaseTimer::start();
            let encode_started = startup_timing.then(Instant::now);
            if (width, height) == (frame.width, frame.height) {
                renderer.render_packet(backend, drawable.view(), packet, SURFACE_FORMAT.is_srgb());
            } else {
                renderer.render_packet_at_size(
                    backend,
                    drawable.view(),
                    packet,
                    width,
                    height,
                    SURFACE_FORMAT.is_srgb(),
                );
            }
            // if let Some(encode_started) = encode_started {
            //     info!(
            //         "DX12 first-frame stage: renderer preparation, command encoding, and queue submission {:.2} ms (CPU elapsed)",
            //         encode_started.elapsed().as_secs_f64() * 1000.0
            //     );
            // }
            encode.finish(FramePhase::Encode);
            let present = PhaseTimer::start();
            let present_started = startup_timing.then(Instant::now);
            let success = drawable.present().is_ok();
            // if let Some(present_started) = present_started {
            //    info!(
            //        "DX12 first-frame stage: DXGI present call {:.2} ms (success={success})",
            //        present_started.elapsed().as_secs_f64() * 1000.0
            //    );
            //}
            present.finish(FramePhase::Present);
            success
        }
    }
}
