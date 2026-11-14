//! Copying a finished frame out to the CPU, for [`aimer_platform::capture`].
//!
//! Every native renderer does it the same way, through the backend trait: the
//! copy is encoded on the queue right after the frame is drawn and before it is
//! presented, so it is in order behind the drawing and reads this frame; and the
//! bytes are read only after the frame has been handed on, so taking a picture
//! never delays showing one. The renderers differ only in where the frame's
//! texture comes from and whether it is blue-first, which the caller says.

use aimer_cupid::backend::{
    BufferDescriptor, BufferUsage, Extent3d, GpuBackend, Origin3d, TexelCopyBufferInfo, TexelCopyBufferLayout,
    TexelCopyTextureInfo, TextureAspect,
};
use aimer_platform::capture;

/// A copy of a frame on its way to the CPU.
pub(super) struct Readback<B: GpuBackend> {
    buffer: B::Buffer,
    bytes_per_row: u32,
    size: u64,
}

/// Takes the request for a picture that is waiting, if any, to answer with the
/// frame being drawn now. A renderer that cannot copy this frame out refuses it
/// instead, so the asker is told at once and is not left waiting for a timeout.
///
/// Called only once a frame can really be drawn: a request that finds no frame to
/// copy stays waiting for one that can.
pub(super) fn take_request(can_capture: bool) -> Option<capture::Pending> {
    if !capture::is_pending() {
        return None;
    }
    if can_capture {
        capture::take_pending()
    } else {
        capture::refuse(capture::Error::Unsupported);
        None
    }
}

/// The row stride of a copy of a `width`-pixel row, padded to 256 bytes, which
/// every backend accepts.
pub(super) fn padded_row_bytes(width: u32) -> u32 {
    (width * 4).div_ceil(256) * 256
}

/// Encodes a copy of `texture` into a buffer the CPU can read, and submits it.
pub(super) fn begin<B: GpuBackend>(backend: &B, texture: &B::Texture, width: u32, height: u32) -> Readback<B> {
    let bytes_per_row = padded_row_bytes(width);
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

/// Waits for the copy, packs its rows, and answers everyone who asked. `bgra` is
/// whether the surface stores blue first, as Apple's do and most swap chains do.
pub(super) fn finish<B: GpuBackend>(
    backend: &B,
    pending: capture::Pending,
    readback: &Readback<B>,
    (width, height): (u32, u32),
    scale: f32,
    bgra: bool,
) {
    let bytes = backend.read_buffer(&readback.buffer, readback.size);
    pending.complete(capture::Frame {
        width,
        height,
        scale,
        bgra,
        pixels: capture::unpad_rows(&bytes, width, height, readback.bytes_per_row),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_padded_to_the_256_bytes_every_backend_accepts() {
        assert_eq!(padded_row_bytes(1), 256);
        assert_eq!(padded_row_bytes(64), 256, "64 pixels are exactly 256 bytes");
        assert_eq!(padded_row_bytes(65), 512);
        assert_eq!(padded_row_bytes(1120), 4608);
    }
}
