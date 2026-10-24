//! A frame that holds a glass panel never loses the page behind it.
//!
//! The renderer keeps the last composed frame and repaints only what changed
//! into it, but a frame with a custom-pipeline draw such as a glass panel cannot
//! use that target: it is drawn from scratch from the frame's render plan. That
//! plan is culled to the frame's damage, so when the damage is small it holds a
//! handful of nodes, and drawing it from scratch leaves everything else blank.
//! A frame has to be a complete one whenever the frame before it or the frame
//! itself needs the renderer to draw from scratch.

use super::*;

use std::cell::RefCell;

use aimer_container::Glass;
use aimer_widget::StateUpdater;

const WIDTH: u32 = 200;
const HEIGHT: u32 = 160;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Option<Gpu> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = runtime
        .block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: true,
        }))
        .ok()?;
    let (device, queue) = runtime
        .block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Aimer bypass target test"),
            ..Default::default()
        }))
        .ok()?;
    Some(Gpu { device, queue })
}

fn target(gpu: &Gpu) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Aimer bypass target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn read(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let unpadded = WIDTH * 4;
    let padded = unpadded.div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Aimer bypass readback"),
        size: u64::from(padded) * u64::from(HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.expect("map readback"));
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for readback");
    let mapped = slice.get_mapped_range().expect("mapped range");
    let mut pixels = Vec::with_capacity((unpadded * HEIGHT) as usize);
    for row in 0..HEIGHT as usize {
        let start = row * padded as usize;
        pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    drop(mapped);
    buffer.unmap();
    pixels
}

fn differing_pixels(left: &[u8], right: &[u8]) -> usize {
    left.chunks(4)
        .zip(right.chunks(4))
        .filter(|(a, b)| a != b)
        .count()
}

fn hover<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
}

struct BarState {
    hovered: bool,
    updater: Rc<RefCell<Option<StateUpdater<BarState>>>>,
}

struct BarScene {
    updater: Rc<RefCell<Option<StateUpdater<BarState>>>>,
}

impl StatefulWidget for BarScene {
    type State = BarState;

    fn create_state(self) -> BarState {
        BarState {
            hovered: false,
            updater: self.updater,
        }
    }
}

impl State<BarScene> for BarState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let updater = self.updater.borrow().expect("initialised");
        let opacity = if self.hovered { 1.0f32 } else { 0.4 };
        let rows: Vec<aimer_widget::AnyWidget> = (0..6u8)
            .map(|i| {
                SizedBox::new()
                    .width(Dimension::Px(200.0))
                    .height(Dimension::Px(60.0))
                    .color(Color::Rgb(40 + i * 30, 80, 200 - i * 30))
                    .boxed()
            })
            .collect();
        aimer_space::Stack::new().children(vec![
            aimer_space::Positioned::new().layer(3).top(0).box_child(
                aimer_input::mouse_region::MouseRegion::new()
                    .on_hover_enter(move || updater.set_state(|state| state.hovered = true))
                    .on_hover_exit(move || updater.set_state(|state| state.hovered = false))
                    .box_child(aimer_animation::ImplicitAnimatedBuilder::new(
                        opacity,
                        std::time::Duration::from_millis(220),
                        aimer_animation::curves::EASE_IN_OUT,
                        move |_opacity| {
                            Glass::new().child(
                                SizedBox::new()
                                    .width(Dimension::Px(200.0))
                                    .height(Dimension::Px(30.0)),
                            )
                        },
                    )),
            ),
            Scrollable::new()
                .child(Column::new().children(rows))
                .boxed(),
        ])
    }
}

impl Widget for BarScene {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "BarScene", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for BarScene {}

#[test]
fn a_glass_bar_over_a_page_never_blanks_the_page() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let Some(gpu) = gpu() else {
        assert!(
            std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_none(),
            "no Metal device"
        );
        eprintln!("skipping: Metal device unavailable");
        return;
    };
    let backend = aimer_cupid::WgpuBackend::new(gpu.device.clone(), gpu.queue.clone());
    let mut renderer =
        aimer_cupid::renderer::Renderer::new(&backend, wgpu::TextureFormat::Rgba8Unorm);
    let (texture, view) = target(&gpu);
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        BarScene { updater },
        HeadlessOptions {
            size: PhysicalSize::new(WIDTH, HEIGHT),
            scale_factor: 1.0,
        },
    );
    hover(&mut app, -10.0, -10.0);

    // The third row of the page, below the bar, is a flat color.
    let page = (120 * WIDTH as usize + 100) * 4;
    let mut painted = None;
    for frame in 0..50 {
        // The cursor enters the bar, leaves it, and moves over the page.
        match frame {
            5 => hover(&mut app, 100.0, 10.0),
            25 => hover(&mut app, 100.0, 120.0),
            35 => hover(&mut app, 100.0, 125.0),
            _ => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(12));
        let (_, packet) = direct_headless_frame_packet(&mut app);
        renderer.render_packet(&backend, &view, &packet, false);
        let shown = read(&gpu, &texture);
        let color = [
            shown[page],
            shown[page + 1],
            shown[page + 2],
            shown[page + 3],
        ];
        let expected = *painted.get_or_insert(color);
        assert_ne!(expected[3], 0, "the first frame must paint the page");
        assert_eq!(
            color, expected,
            "frame {frame} shows {color:?} where the page is {expected:?}"
        );
    }
}
