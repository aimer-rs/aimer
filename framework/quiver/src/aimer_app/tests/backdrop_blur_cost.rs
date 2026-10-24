//! What a backdrop blur costs the GPU, by radius.
//!
//! A tab bar or a back button that reveals itself over a page blurs the page
//! behind it with a radius that starts at 60 logical pixels and falls to nothing.
//! On a 3x phone that is a 180 pixel kernel over a region 360 pixels larger than
//! the button in each direction, so the cost of the blur has to be known as a
//! function of its radius before anything is said about making it cheaper.
//!
//! The measurement is `#[ignore]`d: it is wall-clock time on whatever GPU the
//! machine has. Run it with
//! `cargo nextest run -p aimer_quiver --features wgpu -E 'test(backdrop_blur_cost)' --run-ignored only --no-capture`.

use super::*;

use aimer_decoration::{BackdropFilter, BorderRadius, Decoration, DecorationBox};

/// A 3x phone: 390 x 844 logical pixels.
const WIDTH: u32 = 1170;
const HEIGHT: u32 = 2532;
const SCALE: f64 = 3.0;
const FRAMES: usize = 40;

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
            label: Some("Aimer backdrop blur cost"),
            ..Default::default()
        }))
        .ok()?;
    Some(Gpu { device, queue })
}

fn read(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let unpadded = WIDTH * 4;
    let padded = unpadded.div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Aimer backdrop blur cost readback"),
        size: u64::from(padded) * u64::from(HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(HEIGHT) },
        },
        wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
    );
    gpu.queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.expect("map readback"));
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("wait for readback");
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

fn target(gpu: &Gpu) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = gpu.device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Aimer backdrop blur cost target"),
            size: wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
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

/// A page of stripes behind a 370 x 64 pill, blurred by `radius`, drawn the way
/// `ButtonReveal` draws the tab bar's backdrop.
fn scene(radius: f32) -> impl Widget {
    let rows: Vec<aimer_widget::AnyWidget> = (0..28u8)
        .map(|i| {
            SizedBox::new()
                .width(Dimension::Px(390.0))
                .height(Dimension::Px(30.0))
                .color(Color::Rgb(30 + (i % 2) * 180, 90 + i * 4, 220 - i * 6))
                .boxed()
        })
        .collect();
    let pill = if radius > 0.0 {
        DecorationBox::new()
            .decoration([BorderRadius::from(32.0).notype(), BackdropFilter::new().blur_radius(radius).notype()])
            .box_child(SizedBox::new().width(Dimension::Px(370.0)).height(Dimension::Px(64.0)))
    } else {
        DecorationBox::new()
            .decoration(BorderRadius::from(32.0).notype())
            .box_child(SizedBox::new().width(Dimension::Px(370.0)).height(Dimension::Px(64.0)))
    };
    aimer_space::Stack::new().children(vec![
        Column::new().children(rows).boxed(),
        aimer_space::Positioned::new().layer(3).left(10).top(700).box_child(pill).boxed(),
    ])
}

/// The page behind the pill blurred by a CPU Gaussian of `radius` logical pixels
/// (three sigma is its radius in device pixels), down the column `x` through rows
/// `top..bottom`. The stripes are constant along x, so only rows matter. Samples
/// past the capture's edge repeat its last row, as the shader clamps them.
fn reference_column(page: &[u8], x: usize, rows: std::ops::Range<usize>, radius: f32) -> Vec<[f32; 3]> {
    // The shader's support is the blur radius in device pixels, three sigma.
    let support = (radius * SCALE as f32).ceil() as i64;
    let sigma = support as f32 / 3.0;
    let capture_top = (rows.start as i64 - support).max(0);
    let capture_bottom = (rows.end as i64 + support).min(HEIGHT as i64);
    let weights: Vec<f32> = (-support..=support).map(|d| (-(d * d) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let total: f32 = weights.iter().sum();
    rows.map(|y| {
        let mut sum = [0.0f32; 3];
        for (i, weight) in weights.iter().enumerate() {
            let row = (y as i64 + i as i64 - support).clamp(capture_top, capture_bottom - 1) as usize;
            for channel in 0..3 {
                sum[channel] += weight * page[(row * WIDTH as usize + x) * 4 + channel] as f32;
            }
        }
        sum.map(|value| value / total)
    })
    .collect()
}

/// The largest difference, in levels of an 8 bit channel, between the GPU's blur
/// and a CPU Gaussian, over the middle of the pill.
fn blur_error(gpu: &Gpu, backend: &aimer_cupid::WgpuBackend, radius: f32) -> f32 {
    let (texture, view) = target(gpu);
    let mut page = None;
    let mut blurred = None;
    for (blur, output) in [(0.0, &mut page), (radius, &mut blurred)] {
        let mut renderer = aimer_cupid::renderer::Renderer::new(backend, wgpu::TextureFormat::Rgba8Unorm);
        let mut app = AimerApp::start_headless_with(
            scene(blur),
            HeadlessOptions { size: PhysicalSize::new(WIDTH, HEIGHT), scale_factor: SCALE },
        );
        for _ in 0..3 {
            let (_, packet) = direct_headless_frame_packet(&mut app);
            renderer.render_packet(backend, &view, &packet, false);
        }
        *output = Some(read(gpu, &texture));
    }
    let (page, blurred) = (page.unwrap(), blurred.unwrap());
    // The pill is 1110 x 192 device pixels at (30, 2100); stay off its round ends
    // and its top and bottom edge.
    let rows = 2106..2286;
    let mut worst = 0.0f32;
    for x in [300usize, 600, 900] {
        let expected = reference_column(&page, x, rows.clone(), radius);
        for (offset, y) in rows.clone().enumerate() {
            for channel in 0..3 {
                let shown = blurred[(y * WIDTH as usize + x) * 4 + channel] as f32;
                worst = worst.max((shown - expected[offset][channel]).abs());
            }
        }
    }
    worst
}

#[test]
fn a_backdrop_blur_matches_a_cpu_gaussian_at_every_radius() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let Some(gpu) = gpu() else {
        assert!(std::env::var_os("AIMER_REQUIRE_GPU_E2E").is_none(), "no Metal device");
        eprintln!("skipping: Metal device unavailable");
        return;
    };
    let backend = aimer_cupid::WgpuBackend::new(gpu.device.clone(), gpu.queue.clone());
    // 6 is below any reduction; 10, 20 and 40 logical pixels are 30, 60 and 120
    // device pixels of radius, which reduce the working resolution by 2, 4 and 8.
    for radius in [6.0f32, 10.0, 20.0, 40.0, 60.0] {
        let error = blur_error(&gpu, &backend, radius);
        println!("blur {radius:>4.1}: worst difference from a CPU Gaussian {error:.2} levels");
        assert!(error <= 2.0, "blur {radius} is {error:.2} levels from a CPU Gaussian");
    }
}

#[test]
#[ignore = "wall-clock GPU measurement"]
fn profile_backdrop_blur_cost() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let Some(gpu) = gpu() else {
        eprintln!("skipping: Metal device unavailable");
        return;
    };
    let backend = aimer_cupid::WgpuBackend::new(gpu.device.clone(), gpu.queue.clone());
    let (texture, view) = target(&gpu);

    for radius in [0.0f32, 6.0, 20.0, 40.0, 60.0, 96.0] {
        // A renderer of its own per radius: one renderer keeps what it drew for the
        // scene before.
        let mut renderer = aimer_cupid::renderer::Renderer::new(&backend, wgpu::TextureFormat::Rgba8Unorm);
        let mut app = AimerApp::start_headless_with(
            scene(radius),
            HeadlessOptions { size: PhysicalSize::new(WIDTH, HEIGHT), scale_factor: SCALE },
        );
        // Warm the pipelines and textures.
        for _ in 0..3 {
            let (_, packet) = direct_headless_frame_packet(&mut app);
            renderer.render_packet(&backend, &view, &packet, false);
        }
        let mut samples = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            let (_, packet) = direct_headless_frame_packet(&mut app);
            let started = std::time::Instant::now();
            renderer.render_packet(&backend, &view, &packet, false);
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("wait for the GPU");
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        println!(
            "blur {radius:>5.1} logical px: p50 {:.2} ms  p95 {:.2} ms  min {:.2} ms",
            samples[FRAMES / 2],
            samples[FRAMES * 95 / 100],
            samples[0]
        );
    }
}
