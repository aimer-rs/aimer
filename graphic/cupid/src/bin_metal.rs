use aimer_cupid::backend::metal::{MetalBackend, MetalSurface};
use aimer_cupid::canvas::CupidCanvas;
use aimer_cupid::renderer::RendererImpl;
#[path = "bin_support/demo_content.rs"]
mod demo_content;
use demo_content::record_demo_frame;
use objc2_app_kit::NSView;
use objc2_core_foundation::CGSize;
use objc2_metal::MTLPixelFormat;
use objc2_quartz_core::CAMetalLayer;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

#[cfg(not(target_os = "macos"))]
compile_error!("the `cupid-metal` demo currently requires macOS");

const SURFACE_FORMAT: MTLPixelFormat = MTLPixelFormat::BGRA8Unorm_sRGB;

struct MetalDemo {
    backend: Option<MetalBackend>,
    surface: Option<MetalSurface>,
    renderer: Option<RendererImpl<MetalBackend>>,
    canvas: CupidCanvas,
    window: Option<Window>,
}

impl MetalDemo {
    fn new() -> Self {
        Self {
            backend: None,
            surface: None,
            renderer: None,
            canvas: CupidCanvas::new(),
            window: None,
        }
    }

    fn resize_surface(&self, width: u32, height: u32, scale: f64) {
        let Some(surface) = &self.surface else {
            return;
        };
        surface.layer().setContentsScale(scale);
        surface
            .layer()
            .setDrawableSize(CGSize::new(f64::from(width), f64::from(height)));
    }

    fn draw(&mut self) {
        let (Some(backend), Some(surface), Some(renderer)) = (
            self.backend.as_ref(),
            self.surface.as_ref(),
            self.renderer.as_mut(),
        ) else {
            return;
        };
        let Some(frame) = surface.next_drawable() else {
            return;
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
            true,
            &self.canvas.draw_list(),
        );
        frame.present(backend);
    }
}

impl ApplicationHandler for MetalDemo {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Aimer Cupid — Metal")
                    .with_inner_size(winit::dpi::LogicalSize::new(800, 600)),
            )
            .expect("create Metal demo window");
        let backend = MetalBackend::new().expect("create the system Metal device and queue");
        let raw_handle = window
            .window_handle()
            .expect("get the macOS window handle")
            .as_raw();
        let RawWindowHandle::AppKit(appkit_handle) = raw_handle else {
            panic!("winit returned a non-AppKit window handle on macOS");
        };

        // SAFETY: Winit owns this NSView for the lifetime of `window`. We keep
        // `window` in `self` as long as the Metal layer and renderer are used.
        let view = unsafe { &*appkit_handle.ns_view.as_ptr().cast::<NSView>() };
        let layer = CAMetalLayer::new();
        view.setWantsLayer(true);
        view.setLayer(Some(&layer));
        let surface = MetalSurface::new(&backend, layer, SURFACE_FORMAT);
        let size = window.inner_size();
        surface.layer().setContentsScale(window.scale_factor());
        surface
            .layer()
            .setDrawableSize(CGSize::new(f64::from(size.width), f64::from(size.height)));

        self.renderer = Some(RendererImpl::new(&backend, SURFACE_FORMAT));
        self.backend = Some(backend);
        self.surface = Some(surface);
        self.window = Some(window);
        if let Some(window) = &self.window {
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
                let scale = self.window.as_ref().map_or(1.0, Window::scale_factor);
                self.resize_surface(size.width, size.height, scale);
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
    let event_loop = EventLoop::new().expect("create the macOS event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = MetalDemo::new();
    event_loop.run_app(&mut app).expect("run Metal demo");
}
