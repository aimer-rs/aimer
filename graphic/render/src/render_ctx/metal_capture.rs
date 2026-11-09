//! Copying a finished Metal frame out to the CPU, for [`aimer_platform::capture`].
//!
//! Two paths present Metal frames — the inline one in `metal_ctx` and the raster
//! worker's in `metal_worker` — and both do it the same way: the copy is encoded
//! on the queue right after the frame is drawn and before it is presented, so it
//! is in order behind the drawing and reads this frame; and the bytes are read
//! only after the frame has been handed on, so taking a picture never delays
//! showing one.

use aimer_cupid::backend::metal::MetalBackend;
use aimer_cupid::backend::{
    BufferDescriptor, BufferUsage, Extent3d, GpuBackend, Origin3d, TexelCopyBufferInfo, TexelCopyBufferLayout,
    TexelCopyTextureInfo, TextureAspect,
};
use aimer_platform::capture;

/// A copy of a frame on its way to the CPU.
pub(super) struct Readback {
    buffer: <MetalBackend as GpuBackend>::Buffer,
    bytes_per_row: u32,
    size: u64,
}

/// Encodes a copy of `texture` into a buffer the CPU can read. Rows are padded to
/// 256 bytes, which every backend accepts.
pub(super) fn begin(
    backend: &MetalBackend,
    texture: &<MetalBackend as GpuBackend>::Texture,
    width: u32,
    height: u32,
) -> Readback {
    let bytes_per_row = (width * 4).div_ceil(256) * 256;
    let size = u64::from(bytes_per_row) * u64::from(height);
    let buffer = backend.create_buffer(&BufferDescriptor {
        label: Some("window capture".to_string()),
        size,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("window capture");
    backend.copy_texture_to_buffer(
        &mut encoder,
        &TexelCopyTextureInfo { texture, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
        &TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        Extent3d { width, height, depth_or_array_layers: 1 },
    );
    backend.submit(encoder);
    Readback { buffer, bytes_per_row, size }
}

/// Waits for the copy, packs its rows, and answers everyone who asked. The
/// surface is BGRA on both Metal paths.
pub(super) fn finish(
    backend: &MetalBackend,
    pending: capture::Pending,
    readback: &Readback,
    (width, height): (u32, u32),
    scale: f32,
) {
    let bytes = backend.read_buffer(&readback.buffer, readback.size);
    pending.complete(capture::Frame {
        width,
        height,
        scale,
        bgra: true,
        pixels: capture::unpad_rows(&bytes, width, height, readback.bytes_per_row),
    });
}
