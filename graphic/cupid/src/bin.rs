#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::sync::OnceLock;

#[path = "bin_support/demo_content.rs"]
mod demo_content;
use demo_content::{
    COLOR_GLYPH_SHOWCASE, SOUTHEAST_ASIAN_FONT_SIZE, SOUTHEAST_ASIAN_TEXT, WARM_FONT_SIZES,
    WELCOME_TEXT, record_demo_frame,
};

#[allow(unused_imports)]
use aimer_cupid::backend::wgpu::WgpuBackend;
#[allow(unused_imports)]
use aimer_cupid::canvas::CupidCanvas;
#[allow(unused_imports)]
use aimer_cupid::gpu_context::GpuContext;
#[allow(unused_imports)]
use aimer_cupid::renderer::Renderer;
#[allow(unused_imports)]
use aimer_utils::{ExecTimes, debug};
#[allow(unused_imports)]
use winit::application::ApplicationHandler;
#[allow(unused_imports)]
use winit::event::{ElementState, WindowEvent};
#[allow(unused_imports)]
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
#[allow(unused_imports)]
use winit::window::{Window, WindowId};

static MY_EVENT_PROXY: OnceLock<EventLoopProxy<MyWindowEvent>> = OnceLock::new();

const RENDER_BACKEND_NAME: &str = "WGPU";
pub fn time_consume(func: impl FnOnce()) {
    let start = aimer_utils::AnimInstant::now();
    func();
    let elapsed = start.elapsed();
    println!("Time elapsed: {} ms", elapsed.as_millis());
}
#[cfg(not(target_arch = "wasm32"))]
struct App<'w> {
    gpu: Option<GpuContext<'w>>,
    backend: Option<WgpuBackend>,
    renderer: Option<Renderer>,
    canvas: CupidCanvas,
    window: Option<Window>,
    texture_id: Option<u32>,
}
#[allow(dead_code)]
enum MyWindowEvent {
    FirstFrame,
}
#[cfg(not(target_arch = "wasm32"))]
impl<'w> App<'w> {
    fn new() -> Self {
        Self {
            gpu: None,
            backend: None,
            renderer: None,
            canvas: CupidCanvas::new(),
            window: None,
            texture_id: None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<'w> ApplicationHandler<MyWindowEvent> for App<'w> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let title = RENDER_BACKEND_NAME;
        let attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(800, 600));
        #[cfg(target_os = "macos")]
        let attrs = {
            use winit::platform::macos::WindowAttributesExtMacOS;

            attrs
                .with_decorations(true)
                .with_titlebar_hidden(false)
                .with_titlebar_transparent(false)
                .with_title_hidden(false)
                .with_titlebar_buttons_hidden(false)
                .with_fullsize_content_view(false)
        };
        let window = event_loop.create_window(attrs).unwrap();
        // window.set_min_inner_size(Some(winit::dpi::LogicalSize::new(1500, 700)));
        window.set_title(title);

        let size = window.inner_size();

        // SAFETY: We store the window in self and the GpuContext borrows it.
        // The window outlives the GpuContext because we drop gpu before window.
        let window_ref: &'w Window = unsafe { &*(&window as *const Window) };
        let gpu = GpuContext::initialize(window_ref, size);
        let backend = gpu.backend();

        debug!("Initializing GPU context and loading test image");
        let mut img_renderer = Renderer::new(&backend, gpu.format);
        debug!("Initialized GPU context");

        // AOT-style warm-up: move the expensive text shaping/rasterization off
        // the first visible frame and behind init, so startup is "fast like a
        // compiled language" instead of paying the cold-cache 27–86 ms stall on
        // first paint.
        //
        // Level 2 — pre-rasterize the common ASCII glyph set at the font sizes
        // the app uses, filling the glyph atlas so even brand-new strings only
        // pay shaping (never glyph rasterization).
        img_renderer.warm_glyph_set(&backend, &WARM_FONT_SIZES);
        // Level 1 — pre-shape and lay out the known static text at the size and
        // wrapping width it is drawn with, so it renders from the warm cache on
        // the very first frame. The wrap width mirrors the draw call below
        // (`inner_size().width - 60.0`).
        img_renderer.warm_text(&backend, WELCOME_TEXT, 44.0, size.width as f32 - 60.0);
        img_renderer.warm_text(
            &backend,
            SOUTHEAST_ASIAN_TEXT,
            SOUTHEAST_ASIAN_FONT_SIZE,
            size.width as f32 - 60.0,
        );
        img_renderer.warm_text(&backend, COLOR_GLYPH_SHOWCASE, 22.0, 0.0);
        debug!("Text warm-up complete");
        let image_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("image.png");
        debug!("Loading test image from {}", image_path.display());
        let img = image::open(&image_path)
            .unwrap_or_else(|e| panic!("Failed to load {}: {e}", image_path.display()))
            .into_rgba8();
        let (img_w, img_h) = img.dimensions();
        let tex_id = img_renderer.upload_image(&backend, img_w, img_h, img.as_raw());
        debug!("Uploaded image to GPU");

        self.texture_id = Some(tex_id);
        self.backend = Some(backend);
        debug!("Test image uploaded");
        self.renderer = Some(img_renderer);
        debug!("Renderer initialized");
        self.gpu = Some(gpu);
        debug!("GPU context initialized");
        self.window = Some(window);
        debug!("Window initialized");
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
        debug!("App resumed");
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: MyWindowEvent) {
        match event {
            MyWindowEvent::FirstFrame => {
                self.window.as_ref().unwrap().request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(new_size);
                    // Ensure a frame is painted after the surface is reconfigured.
                    // On macOS, the initial request_redraw() in resumed() can be
                    // silently dropped if the window isn't fully on-screen yet.
                    // The first Resized event is the reliable signal that the
                    // window is visible and the surface is ready.
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }

            WindowEvent::MouseInput { state, .. } => {
                if ElementState::Pressed == state
                    && let Some(window) = self.window.as_ref()
                {
                    window.request_redraw();
                }
            }

            WindowEvent::RedrawRequested => {
                let gpu = match &self.gpu {
                    Some(g) => g,
                    None => return,
                };
                let renderer = match &mut self.renderer {
                    Some(r) => r,
                    None => return,
                };

                let frame = match gpu.begin_frame() {
                    wgpu::CurrentSurfaceTexture::Success(f)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                    _ => return,
                };

                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                let width = gpu.width();
                let height = gpu.height();

                // Build draw commands using CupidCanvas.
                record_demo_frame(&self.canvas, width as f32, height as f32);

                // Draw a blue background rect
                // self.canvas
                //     .fill_rect(20.0, 20.0, 300.0, 200.0, Color::new(0.2, 0.4, 0.8, 1.0),
                // [10.0; 4]);
                //
                // // Draw a red rect
                // self.canvas
                //     .fill_rect(50.0, 50.0, 150.0, 80.0, Color::red(), [20.0; 4]);
                //
                // // Draw a green rounded rect
                // self.canvas
                //     .fill_rect(200.0, 100.0, 180.0, 120.0, Color::green(), [20.0; 4]);
                //
                // // Draw a rect with border
                // self.canvas.fill_rect_with_border(
                //     420.0, 300.0, 160.0, 100.0,
                //     Color::red(),
                //     [12.0; 4],
                //     3.0,
                //     Color::new(0.2, 0.2, 0.8, 1.0),
                // );
                //
                // // Draw a border-only rect (transparent fill)
                // self.canvas.fill_rect_with_border(
                //     420.0, 420.0, 460.0, 480.0,
                //     Color::blue(),
                //     [28.0; 4],
                //     0.0,
                //     Color::red(),
                // );
                //
                // // Test clipping
                // self.canvas.set_clip(50.0, 400.0, 200.0, 100.0);
                // self.canvas.fill_rect(
                //     30.0, 380.0, 300.0, 150.0,
                //     Color::red(),
                //     [0.0; 4],
                // );
                //
                // // Test save/translate/restore
                // self.canvas.save();
                // self.canvas.translate(400.0, 50.0);
                // self.canvas.fill_rect(
                //     0.0, 0.0, 500.0, 450.0,
                //     Color::new(0.8, 0.2, 0.8, 1.0).set_alpha(128),
                //     [5.0; 4],
                // );
                // self.canvas.restore();

                // Draw text
                // self.canvas.draw_text(30.0, 250.0, "Hello from Cupid!", 32.0,
                // Color::black());
                //
                // self.canvas
                //     .draw_text(30.0, 300.0, "Wgpu-powered UI render engine", 20.0,
                // Color::black());

                // Mixed CJK + color emoji line — verifies fixes A (no first-frame
                // stall on CJK) and B/C (AppleColorEmoji renders alongside CJK).
                // self.canvas.draw_text(30.0, 340.0, "អរគុណ 你哈皮  With State 你好 きみなと
                // 👉", 44.0, Color::black()); self.canvas
                //     .draw_text(30.0, 740.0, "هَمْزَة عَلَى الأَلِفْ	", 44.0, Color::black());
                // Draw test image if available
                // if let Some(tex_id) = self.texture_id {
                //     self.canvas.draw_image(500.0, 200.0, 300.0, 300.0, tex_id);
                // }

                ExecTimes::print_time(|| {
                    renderer.render(
                        self.backend.as_ref().unwrap(),
                        &view,
                        width,
                        height,
                        gpu.is_srgb,
                        &self.canvas.draw_list(),
                    )
                });

                gpu.end_frame(frame);
                #[cfg(debug_assertions)]
                {
                    debug!(
                        "#############################>Time Consume<#######################################"
                    );
                    ExecTimes::cost_grouping();
                    debug!(
                        "##################################################################################"
                    )
                }
            }
            _ => {}
        }
    }

    // fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
    //     if self.frame_count > 0 {
    //         self.frame_count -= 1;
    //         debug!("Render Frame: {}", self.frame_count);
    //         self.window.as_ref().unwrap().request_redraw();
    //     }
    // }
}

fn main() {
    let event_loop = EventLoop::<MyWindowEvent>::with_user_event()
        .build()
        .expect("Failed to create event loop");

    MY_EVENT_PROXY.set(event_loop.create_proxy()).ok();
    event_loop.set_control_flow(ControlFlow::Wait);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = App::new();
        event_loop.run_app(&mut app).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::record_demo_frame;
    use aimer_cupid::canvas::CupidCanvas;
    use aimer_cupid::draw_cmd::DrawCommand;

    #[test]
    fn demo_records_an_opaque_background_before_text() {
        let canvas = CupidCanvas::new();

        record_demo_frame(&canvas, 800.0, 600.0);

        let draw_list = canvas.draw_list();
        assert!(matches!(
            draw_list.commands().first(),
            Some(DrawCommand::FillRect { color, .. })
                if color.a > 0 && color.r > 0 && color.g > 0 && color.b > 0
        ));
        assert!(draw_list.commands().iter().any(|command| matches!(
            command,
            DrawCommand::DrawText {
                draw_glyphs: true,
                ..
            }
        )));
        assert!(draw_list.commands().iter().any(|command| matches!(
            command,
            DrawCommand::PushClip { rect, .. }
                if rect.x >= 0.0 && rect.y >= 0.0 && rect.width <= 800.0 && rect.height <= 600.0
        )));
        assert!(matches!(
            draw_list.commands().last(),
            Some(DrawCommand::PopClip)
        ));
    }

    #[test]
    fn demo_records_color_and_multiple_weight_samples() {
        let canvas = CupidCanvas::new();

        record_demo_frame(&canvas, 800.0, 600.0);

        let draw_list = canvas.draw_list();
        let draw_texts = draw_list
            .commands()
            .iter()
            .filter_map(|command| match command {
                DrawCommand::DrawText {
                    text,
                    color,
                    font_weight,
                    ..
                } => Some((text.as_ref(), *color, *font_weight)),
                _ => None,
            })
            .collect::<Vec<_>>();

        for weight in [200, 300, 400, 500, 600, 700, 800] {
            assert!(
                draw_texts
                    .iter()
                    .any(|(_, _, recorded)| *recorded == weight),
                "the demo should record a {weight} weight sample"
            );
        }
        assert!(
            draw_texts.iter().any(|(text, _, _)| text.contains("😀")),
            "the demo should include a color-glyph probe"
        );
        assert!(
            draw_texts.iter().any(|(_, color, _)| {
                (color.r > 0 && color.g == 0 && color.b == 0)
                    || (color.g > 0 && color.r == 0 && color.b == 0)
                    || (color.b > 0 && color.r == 0 && color.g == 0)
            }),
            "the demo should include colored text samples"
        );
    }
}
