use std::sync::Arc;

use aimer_cupid::backend::opengl::{OpenGlBackend, OpenGlSurface};
use aimer_cupid::canvas::CupidCanvas;
use aimer_cupid::renderer::RendererImpl;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
compile_error!("the `cupid-opengl` demo currently targets Windows and Linux");

#[path = "bin_support/demo_content.rs"]
mod demo_content;

use demo_content::record_demo_frame;

struct OpenGlDemo {
    backend: Option<OpenGlBackend>,
    surface: Option<OpenGlSurface>,
    renderer: Option<RendererImpl<OpenGlBackend>>,
    canvas: CupidCanvas,
    window: Option<Arc<Window>>,
}

impl OpenGlDemo {
    fn new() -> Self {
        Self {
            backend: None,
            surface: None,
            renderer: None,
            canvas: CupidCanvas::new(),
            window: None,
        }
    }

    fn draw(&mut self) {
        let (Some(backend), Some(surface), Some(renderer)) = (
            self.backend.as_ref(),
            self.surface.as_mut(),
            self.renderer.as_mut(),
        ) else {
            return;
        };
        let is_srgb = surface.is_srgb();
        let frame = match surface.try_acquire() {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(error) => panic!("acquire OpenGL frame: {error}"),
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
        frame.present().expect("present the OpenGL frame");
    }
}

impl ApplicationHandler for OpenGlDemo {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Aimer Cupid — OpenGL")
                        .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
                        .with_visible(false),
                )
                .expect("create OpenGL demo window"),
        );
        let size = window.inner_size();
        let (backend, surface) =
            OpenGlBackend::new_windowed(window.clone(), (size.width, size.height))
                .expect("create OpenGL backend and surface");
        let renderer = RendererImpl::new(&backend, surface.format());

        self.renderer = Some(renderer);
        self.backend = Some(backend);
        self.surface = Some(surface);
        self.window = Some(window);
        self.draw();
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
                if size.width == 0 || size.height == 0 {
                    return;
                }
                if let Some(surface) = &mut self.surface {
                    surface
                        .resize((size.width, size.height))
                        .expect("resize the OpenGL surface");
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("create the OpenGL demo event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = OpenGlDemo::new();
    event_loop.run_app(&mut app).expect("run OpenGL demo");
}
