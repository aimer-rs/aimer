use std::sync::Arc;
use std::time::{Duration, Instant};

use aimer_cupid::canvas::CupidCanvas;
use aimer_cupid::renderer::RendererImpl;
use aimer_cupid::{VulkanBackend, VulkanSurface};
#[path = "bin_support/demo_content.rs"]
mod demo_content;
use demo_content::record_demo_frame;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
compile_error!("the `cupid-vulkan` demo currently targets Windows and Linux");

struct VulkanDemo {
    backend: Option<VulkanBackend>,
    surface: Option<VulkanSurface<Window>>,
    renderer: Option<RendererImpl<VulkanBackend>>,
    canvas: CupidCanvas,
    window: Option<Arc<Window>>,
    retry_redraw_at: Option<Instant>,
}

impl VulkanDemo {
    fn new() -> Self {
        Self {
            backend: None,
            surface: None,
            renderer: None,
            canvas: CupidCanvas::new(),
            window: None,
            retry_redraw_at: None,
        }
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(backend), Some(surface), Some(renderer)) = (
            self.backend.as_ref(),
            self.surface.as_mut(),
            self.renderer.as_mut(),
        ) else {
            return;
        };
        let is_srgb = surface.is_srgb();
        let is_suspended = surface.is_suspended();
        let frame = match surface.try_acquire() {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                if !is_suspended {
                    let retry_at = Instant::now() + Duration::from_millis(4);
                    self.retry_redraw_at = Some(retry_at);
                    event_loop.set_control_flow(ControlFlow::WaitUntil(retry_at));
                }
                return;
            }
            Err(error) => panic!("acquire Vulkan frame: {error}"),
        };
        let (width, height) = frame.size();
        if width == 0 || height == 0 {
            return;
        }
        record_demo_frame(&self.canvas, width as f32, height as f32);
        renderer.render(
            backend,
            frame.view(),
            width,
            height,
            is_srgb,
            &self.canvas.draw_list(),
        );
        frame.present().expect("present the Vulkan frame");
        self.retry_redraw_at = None;
        event_loop.set_control_flow(ControlFlow::Wait);
        if surface.is_resize_pending() {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

impl ApplicationHandler for VulkanDemo {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Aimer Cupid — Vulkan")
                        .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
                        .with_visible(false),
                )
                .expect("create Vulkan demo window"),
        );
        let size = window.inner_size();
        let (backend, surface) =
            VulkanBackend::new_windowed(window.clone(), (size.width, size.height))
                .expect("create Vulkan backend and surface");
        let format = surface.format();
        self.renderer = Some(RendererImpl::new(&backend, format));
        self.backend = Some(backend);
        self.surface = Some(surface);
        self.window = Some(window);
        self.draw(event_loop);
        if let Some(window) = &self.window {
            window.set_visible(true);
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(surface) = &mut self.surface {
                    surface.resize((size.width, size.height));
                }
                if size.width > 0 && size.height > 0 {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(retry_at) = self.retry_redraw_at {
            if Instant::now() >= retry_at {
                self.retry_redraw_at = None;
                event_loop.set_control_flow(ControlFlow::Wait);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(retry_at));
            }
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("create the Vulkan demo event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = VulkanDemo::new();
    event_loop.run_app(&mut app).expect("run Vulkan demo");
}
