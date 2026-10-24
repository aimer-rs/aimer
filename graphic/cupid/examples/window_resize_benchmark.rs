//! Measures CPU encoding of full window repaints at changing iPad-sized bounds.
//! Run `cargo run -p aimer_cupid --example window_resize_benchmark` on macOS.

#[cfg(all(target_os = "macos", feature = "native"))]
fn main() {
    use aimer_cupid::backend::metal::MetalBackend;
    use aimer_cupid::backend::*;
    use aimer_cupid::damage_region::DamageSet;
    use aimer_cupid::draw_cmd::DrawList;
    use aimer_cupid::frame::{Frame, FramePacket, FrameRenderMetadata};
    use aimer_cupid::renderer::RendererImpl;
    use objc2_metal::MTLPixelFormat;
    use std::time::Instant;

    let backend = MetalBackend::new().expect("benchmark requires a Metal device");
    let format = MTLPixelFormat::RGBA8Unorm;
    let mut renderer = RendererImpl::new(&backend, format);
    let fence = backend.create_buffer(&BufferDescriptor {
        label: None,
        size: 4,
        usage: vec![BufferUsage::MapRead],
    });
    let mut times = Vec::new();
    for index in 0..65 {
        let width = 1536 + index * 11;
        let height = 1152 + index * 7;
        let texture = backend.create_texture(&TextureDescriptor {
            label: None,
            size: (width, height, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: vec![TextureUsage::RenderAttachment],
        });
        let view = backend.create_texture_view(&texture, "resize benchmark");
        let packet = FramePacket::new(
            Frame::new(DrawList::new(), width, height),
            FrameRenderMetadata::new(2.0, 1, 1, 1, 1, DamageSet::full(width, height)),
        );
        let start = Instant::now();
        renderer.render_packet(&backend, &view, &packet, false);
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        // Limit in-flight work without including GPU completion in CPU time.
        backend.read_buffer(&fence, 0);
        if index != 0 {
            times.push(elapsed);
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "frames={} mean={:.3}ms median={:.3}ms p95={:.3}ms",
        times.len(),
        times.iter().sum::<f64>() / times.len() as f64,
        times[times.len() / 2],
        times[(times.len() * 95 / 100).min(times.len() - 1)]
    );
}

#[cfg(not(all(target_os = "macos", feature = "native")))]
fn main() {
    eprintln!("This benchmark requires macOS with the native Metal backend.");
}
