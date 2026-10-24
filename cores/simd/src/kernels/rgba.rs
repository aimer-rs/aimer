/// Composites a constant straight RGBA8 color through a coverage mask.
///
/// Each complete destination pixel is blended in source-over order, using
/// integer rounding at each stage and byte truncation for the final channels.
/// A zero effective source alpha leaves all four destination bytes untouched.
/// The shorter of the coverage mask and
/// complete pixel count is processed; extra coverage, pixels, and trailing
/// bytes are untouched. No allocation or alignment is required.
///
/// Uses NEON on little-endian AArch64 and SSE2 on x86_64 unless `force-scalar`
/// is enabled. Other targets use scalar arithmetic.
/// Short buffers and partial vectors use scalar arithmetic.
///
/// ```
/// let mut pixel = [255, 0, 0, 255];
/// aimer_simd::kernels::composite_rgba8(&mut pixel, &[128], [0, 255, 0, 255]);
/// assert_eq!(pixel, [127, 128, 0, 255]);
/// ```
#[inline(always)]
pub fn composite_rgba8(bitmap: &mut [u8], coverage: &[u8], color: [u8; 4]) {
    if color[3] == 0 {
        return;
    }
    if bitmap.len() < 16 || coverage.len() < 4 {
        composite_scalar(bitmap, coverage, color);
        return;
    }
    #[cfg(all(
        not(feature = "force-scalar"),
        target_endian = "little",
        any(target_arch = "aarch64", target_arch = "x86_64")
    ))]
    let processed = crate::backend::rgba::composite(bitmap, coverage, color);
    #[cfg(any(
        feature = "force-scalar",
        target_endian = "big",
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    ))]
    let processed = 0;
    composite_scalar(&mut bitmap[processed * 4..], &coverage[processed..], color);
}

#[inline(always)]
fn composite_scalar(bitmap: &mut [u8], coverage: &[u8], color: [u8; 4]) {
    for (pixel, &count) in bitmap.as_chunks_mut::<4>().0.iter_mut().zip(coverage) {
        let source_alpha = (u32::from(count) * u32::from(color[3]) + 127) / 255;
        if source_alpha == 0 {
            continue;
        }
        let destination_alpha = u32::from(pixel[3]);
        let inverse_source_alpha = 255 - source_alpha;
        let output_alpha = source_alpha + (destination_alpha * inverse_source_alpha + 127) / 255;
        let destination_factor = destination_alpha * inverse_source_alpha;
        pixel[0] = ((u32::from(color[0]) * source_alpha
            + (u32::from(pixel[0]) * destination_factor + 127) / 255
            + output_alpha / 2)
            / output_alpha) as u8;
        pixel[1] = ((u32::from(color[1]) * source_alpha
            + (u32::from(pixel[1]) * destination_factor + 127) / 255
            + output_alpha / 2)
            / output_alpha) as u8;
        pixel[2] = ((u32::from(color[2]) * source_alpha
            + (u32::from(pixel[2]) * destination_factor + 127) / 255
            + output_alpha / 2)
            / output_alpha) as u8;
        pixel[3] = output_alpha as u8;
    }
}

#[cfg(test)]
mod tests;
