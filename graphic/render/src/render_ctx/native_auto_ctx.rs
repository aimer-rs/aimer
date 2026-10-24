use aimer_cupid::AntiAlias;
use aimer_cupid::canvas::CupidCanvas;
use aimer_cupid::damage_region::DamageSet;
use aimer_cupid::frame::{Frame, FramePacket};
use winit::dpi::PhysicalSize;
use std::sync::Arc;
use winit::window::Window;

use crate::render_ctx::{
    NativeBackendChoice, OpenGlApi, PresentOutcome, VulkanApi, choose_native_backend,
};

enum ActiveBackend {
    Vulkan(VulkanApi),
    OpenGl(OpenGlApi),
}

/// Linux and Android presenter that tries Vulkan first, then OpenGL if initialization fails.
pub struct NativeAutoApi {
    antialiasing: AntiAlias,
    active: Option<ActiveBackend>,
}

impl Default for NativeAutoApi {
    fn default() -> Self {
        Self::new(AntiAlias::default())
    }
}

impl NativeAutoApi {
    /// Creates an uninitialized auto-selecting native renderer.
    pub fn new(antialiasing: AntiAlias) -> Self {
        Self {
            antialiasing,
            active: None,
        }
    }

    #[inline]
    pub fn is_ready(&self) -> bool {
        self.active.as_ref().is_some_and(|active| match active {
            ActiveBackend::Vulkan(api) => api.is_ready(),
            ActiveBackend::OpenGl(api) => api.is_ready(),
        })
    }

    #[inline]
    pub fn is_offloaded(&self) -> bool {
        false
    }

    /// Initializes Vulkan first and falls back to OpenGL on failure.
    pub fn initialize(&mut self, window: &Arc<Window>, size: PhysicalSize<u32>) {
        if self.is_ready() {
            self.resize(size);
            return;
        }

        let mut vulkan = VulkanApi::new(self.antialiasing);
        let mut opengl = OpenGlApi::new(self.antialiasing);
        let mut vulkan_error = None;
        let mut opengl_error = None;
        let choice = choose_native_backend(
            || match vulkan.try_initialize(window, size) {
                Ok(()) => true,
                Err(error) => {
                    vulkan_error = Some(error.to_string());
                    false
                }
            },
            || match opengl.try_initialize(window, size) {
                Ok(()) => true,
                Err(error) => {
                    opengl_error = Some(error.to_string());
                    false
                }
            },
        );
        match choice {
            Some(NativeBackendChoice::Vulkan) => self.active = Some(ActiveBackend::Vulkan(vulkan)),
            Some(NativeBackendChoice::OpenGl) => self.active = Some(ActiveBackend::OpenGl(opengl)),
            None => panic!(
                "initialize native rendering failed: Vulkan: {}; OpenGL: {}",
                vulkan_error.as_deref().unwrap_or("no error detail"),
                opengl_error.as_deref().unwrap_or("no error detail"),
            ),
        }
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        match &mut self.active {
            Some(ActiveBackend::Vulkan(api)) => api.resize(size),
            Some(ActiveBackend::OpenGl(api)) => api.resize(size),
            None => {}
        }
    }

    pub fn render_frame_packet(
        &mut self,
        draw_fn: impl FnOnce(&aimer_cupid::canvas::CupidCanvas, u32, u32) -> (f32, DamageSet),
    ) -> PresentOutcome {
        match &mut self.active {
            Some(ActiveBackend::Vulkan(api)) => api.render_frame_packet(draw_fn),
            Some(ActiveBackend::OpenGl(api)) => api.render_frame_packet(draw_fn),
            None => PresentOutcome::Dropped,
        }
    }

    /// Records and presents a full-damage frame using the selected backend.
    pub fn render_frame(&mut self, draw: impl FnOnce(&CupidCanvas, u32, u32)) -> PresentOutcome {
        self.render_frame_packet(|canvas, width, height| {
            draw(canvas, width, height);
            (1.0, DamageSet::full(width, height))
        })
    }

    /// Records a full-damage frame without presenting it.
    pub fn build_frame(&mut self, draw: impl FnOnce(&CupidCanvas, u32, u32)) -> Option<Frame> {
        self.build_frame_packet(|canvas, width, height| {
            draw(canvas, width, height);
            (1.0, DamageSet::full(width, height))
        })
        .map(FramePacket::into_frame)
    }

    /// Records a damage-carrying packet without presenting it.
    pub fn build_frame_packet(
        &mut self,
        draw: impl FnOnce(&CupidCanvas, u32, u32) -> (f32, DamageSet),
    ) -> Option<FramePacket> {
        match &mut self.active {
            Some(ActiveBackend::Vulkan(api)) => api.build_frame_packet(draw),
            Some(ActiveBackend::OpenGl(api)) => api.build_frame_packet(draw),
            None => None,
        }
    }

    /// Presents a full-damage frame using the selected backend.
    pub fn present(&mut self, frame: Frame) -> PresentOutcome {
        match &mut self.active {
            Some(ActiveBackend::Vulkan(api)) => api.present(frame),
            Some(ActiveBackend::OpenGl(api)) => api.present(frame),
            None => PresentOutcome::Dropped,
        }
    }

    pub(crate) fn worker_max_dimension(&self) -> u32 {
        match &self.active {
            Some(ActiveBackend::Vulkan(api)) => api.worker_max_dimension(),
            Some(ActiveBackend::OpenGl(api)) => api.worker_max_dimension(),
            None => u32::MAX,
        }
    }

    pub(crate) fn present_worker_packet(&mut self, packet: &FramePacket) -> bool {
        match &mut self.active {
            Some(ActiveBackend::Vulkan(api)) => api.present_worker_packet(packet),
            Some(ActiveBackend::OpenGl(api)) => api.present_worker_packet(packet),
            None => false,
        }
    }

    pub fn present_packet(&mut self, packet: FramePacket) -> PresentOutcome {
        match &mut self.active {
            Some(ActiveBackend::Vulkan(api)) => api.present_packet(packet),
            Some(ActiveBackend::OpenGl(api)) => api.present_packet(packet),
            None => PresentOutcome::Dropped,
        }
    }
}
