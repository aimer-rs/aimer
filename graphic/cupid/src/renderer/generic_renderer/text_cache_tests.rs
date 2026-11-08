//! The text caches are bounded by bytes, not only by entry count.
//!
//! A frame of changing text (a clock, a counter, a streamed reply, a
//! diagnostics overlay) shapes a string no later frame will ask for again. The
//! caches are keyed by the whole string, so without a byte bound they keep
//! every one of them until an entry count that is far too high for strings of
//! this length.

use super::*;
use crate::backend::metal::MetalBackend;
use crate::backend::*;
use crate::damage_region::DamageSet;
use crate::draw_cmd::Vec2d;
use crate::frame::Frame;
use crate::utilities::Color;
use objc2_metal::MTLPixelFormat;
use std::sync::Arc;

const FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA8Unorm;
const WIDTH: u32 = 640;
const HEIGHT: u32 = 64;

fn target(backend: &MetalBackend) -> <MetalBackend as GpuBackend>::Texture {
    backend.create_texture(&TextureDescriptor {
        label: None,
        size: (WIDTH, HEIGHT, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FORMAT,
        usage: vec![TextureUsage::RenderAttachment],
    })
}

fn packet(draw: DrawList) -> FramePacket {
    FramePacket::new(
        Frame::new(draw, WIDTH, HEIGHT),
        FrameRenderMetadata::new(1.0, 1, 1, 1, 1, DamageSet::full(WIDTH, HEIGHT)),
    )
}

/// A long single line that is unique to `frame`.
fn unique_line(frame: usize) -> String {
    format!(
        "tick {frame:05}: {}",
        "the quick brown fox jumps over the lazy dog ".repeat(9)
    )
}

#[test]
fn a_stream_of_unique_strings_cannot_grow_the_text_caches_without_bound() {
    let backend = MetalBackend::new().expect("native GPU regression requires Metal");
    let mut renderer = Renderer::new(&backend, FORMAT);
    let texture = target(&backend);
    let view = backend.create_texture_view(&texture, "text cache bound");

    let mut largest = (0_usize, 0_usize);
    let mut one_entry = 0_usize;
    for frame in 0..400 {
        let mut draw = DrawList::new();
        draw.draw_text(
            Vec2d { x: 8.0, y: 8.0 },
            Arc::from(unique_line(frame).as_str()),
            14.0,
            Color::rgba8(255, 255, 255, 255),
            400,
        );
        renderer.render_packet(&backend, &view, &packet(draw), false);

        let stats = renderer.memory_stats();
        largest.0 = largest.0.max(stats.text_shaping_cache_bytes);
        largest.1 = largest.1.max(stats.text_layout_cache_bytes);
        if frame == 0 {
            one_entry = stats.text_shaping_cache_bytes;
        }
    }

    assert!(
        one_entry > 0,
        "the shaping cache must account for what it holds"
    );
    let shaping_limit = TextPipelineV2::<MetalBackend>::SHAPING_CACHE_BYTE_BUDGET + one_entry * 2;
    let layout_limit = TextPipelineV2::<MetalBackend>::LAYOUT_CACHE_BYTE_BUDGET + one_entry;
    assert!(
        largest.0 <= shaping_limit,
        "400 unique strings left {} KiB of shaping results (budget {} KiB)",
        largest.0 / 1024,
        TextPipelineV2::<MetalBackend>::SHAPING_CACHE_BYTE_BUDGET / 1024
    );
    assert!(
        largest.1 <= layout_limit,
        "400 unique strings left {} KiB of layouts (budget {} KiB)",
        largest.1 / 1024,
        TextPipelineV2::<MetalBackend>::LAYOUT_CACHE_BYTE_BUDGET / 1024
    );
}
