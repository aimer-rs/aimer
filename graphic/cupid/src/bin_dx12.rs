use std::ffi::c_void;

use aimer_cupid::backend::dx12::{Dx12Backend, Dx12Surface, Dx12TextureFormat};
use aimer_cupid::canvas::CupidCanvas;
use aimer_cupid::renderer::RendererImpl;
#[path = "bin_support/demo_content.rs"]
mod demo_content;
use demo_content::record_demo_frame;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

#[cfg(not(target_os = "windows"))]
compile_error!("the `cupid-dx12` demo currently requires Windows");

const SURFACE_FORMAT: Dx12TextureFormat = Dx12TextureFormat::Bgra8UnormSrgb;

struct Dx12Demo {
    backend: Option<Dx12Backend>,
    surface: Option<Dx12Surface>,
    renderer: Option<RendererImpl<Dx12Backend>>,
    canvas: CupidCanvas,
    window: Option<Window>,
}

impl Dx12Demo {
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
        let frame = match surface.try_acquire(backend) {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return;
            }
            Err(error) => panic!("resize the DirectX 12 swap chain: {error}"),
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
            SURFACE_FORMAT.is_srgb(),
            &self.canvas.draw_list(),
        );
        frame.present().expect("present the Direct3D 12 frame");
    }
}

impl ApplicationHandler for Dx12Demo {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Aimer Cupid — DirectX 12")
                    .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
                    .with_visible(false),
            )
            .expect("create DirectX 12 demo window");
        let backend = Dx12Backend::new().expect("create the system Direct3D 12 device and queue");
        let raw_handle = window
            .window_handle()
            .expect("get the Windows window handle")
            .as_raw();
        let RawWindowHandle::Win32(win32_handle) = raw_handle else {
            panic!("winit returned a non-Win32 window handle on Windows");
        };
        let hwnd = HWND(win32_handle.hwnd.get() as *mut c_void);
        let size = window.inner_size();
        let surface = Dx12Surface::new(&backend, hwnd, size.width, size.height, SURFACE_FORMAT)
            .expect("create the DirectX 12 swap chain");

        self.renderer = Some(RendererImpl::new(&backend, SURFACE_FORMAT));
        self.backend = Some(backend);
        self.surface = Some(surface);
        self.window = Some(window);
        // Renderer and first-frame setup are synchronous. Keep the native
        // window hidden until its swap chain contains real demo content.
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
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                if let Some(surface) = &mut self.surface {
                    surface.request_resize(size.width, size.height);
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
    let event_loop = EventLoop::new().expect("create the Windows event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = Dx12Demo::new();
    event_loop.run_app(&mut app).expect("run DirectX 12 demo");
}
