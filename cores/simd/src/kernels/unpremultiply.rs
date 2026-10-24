/// Converts premultiplied RGBA8 pixels to straight alpha in place.
///
/// Each RGB channel becomes `(channel * 255 + alpha / 2) / alpha`, clamped
/// to 255. Alpha bytes are unchanged. Fully transparent and opaque pixels
/// retain all their bytes, including hidden RGB in transparent pixels.
/// Channels greater than alpha are accepted and saturated. Incomplete trailing
/// pixels are untouched. No allocation or vector alignment is required.
///
/// Uses NEON on little-endian AArch64 and SSE2 on x86_64 unless `force-scalar`
/// is enabled. Other targets, short buffers, and partial vectors use scalar
/// arithmetic.
///
/// ```
/// let mut pixel = [64, 32, 16, 128];
/// aimer_simd::kernels::unpremultiply_rgba8(&mut pixel);
/// assert_eq!(pixel, [128, 64, 32, 128]);
/// ```
#[inline(always)]
pub fn unpremultiply_rgba8(bitmap: &mut [u8]) {
    if bitmap.len() < 16 {
        unpremultiply_scalar(bitmap);
        return;
    }
    #[cfg(all(
        not(feature = "force-scalar"),
        target_endian = "little",
        any(target_arch = "aarch64", target_arch = "x86_64")
    ))]
    let processed = crate::backend::rgba::unpremultiply(bitmap);
    #[cfg(any(
        feature = "force-scalar",
        target_endian = "big",
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    ))]
    let processed = 0;
    unpremultiply_scalar(&mut bitmap[processed..]);
}

#[inline(always)]
fn unpremultiply_scalar(bitmap: &mut [u8]) {
    for pixel in bitmap.as_chunks_mut::<4>().0.iter_mut() {
        let alpha = pixel[3];
        if alpha == 0 || alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * 255 + u16::from(alpha) / 2) / u16::from(alpha))
                .min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unpremultiply_preserves_alpha_rounding_saturation_and_inactive_pixels() {
        let mut bitmap = [
            64, 32, 16, 128, 8, 9, 17, 17, 17, 99, 231, 0, 1, 128, 255, 255, 1, 1, 1, 1, 255, 13,
            7, 1, 1, 0, 2, 2, 0, 0, 0, 127,
        ];
        super::unpremultiply_rgba8(&mut bitmap);
        assert_eq!(
            bitmap,
            [
                128, 64, 32, 128, 120, 135, 255, 17, 17, 99, 231, 0, 1, 128, 255, 255, 255, 255,
                255, 1, 255, 255, 255, 1, 128, 0, 255, 2, 0, 0, 0, 127
            ]
        );
    }

    // Frozen pre-SIMD Core Text implementation used as the compatibility oracle.
    fn reference(bitmap: &mut [u8]) {
        for pixel in bitmap.chunks_exact_mut(4) {
            let alpha = pixel[3];
            if alpha == 0 || alpha == u8::MAX {
                continue;
            }
            for channel in &mut pixel[..3] {
                *channel =
                    ((*channel as u16 * 255 + alpha as u16 / 2) / alpha as u16).min(255) as u8;
            }
        }
    }

    #[test]
    fn unpremultiply_matches_every_channel_and_alpha_pair() {
        let original: Vec<u8> = (0..=255_u8)
            .flat_map(|alpha| {
                (0..=255_u8).flat_map(move |channel| {
                    [channel, channel.wrapping_mul(73), 255 - channel, alpha]
                })
            })
            .collect();
        let mut expected = original.clone();
        reference(&mut expected);
        for offset in 0..16 {
            let mut actual = vec![99; original.len() + 16];
            actual[offset..offset + original.len()].copy_from_slice(&original);
            super::unpremultiply_rgba8(&mut actual[offset..offset + original.len()]);
            assert_eq!(&actual[offset..offset + original.len()], expected);
            assert!(
                actual[..offset]
                    .iter()
                    .chain(&actual[offset + original.len()..])
                    .all(|&v| v == 99)
            );
        }
    }

    #[test]
    fn unpremultiply_preserves_unaligned_buffers_and_incomplete_pixels() {
        for bytes in 0..=83 {
            for offset in 0..16 {
                let mut actual: Vec<u8> = (0..bytes + 16).map(|i| (i * 73 + 19) as u8).collect();
                let mut expected = actual.clone();
                reference(&mut expected[offset..offset + bytes]);
                super::unpremultiply_rgba8(&mut actual[offset..offset + bytes]);
                assert_eq!(actual, expected, "length {bytes}, offset {offset}");
            }
        }
    }

    #[test]
    fn unpremultiply_preserves_all_transparent_and_opaque_block_patterns() {
        for mask in 0..16 {
            let mut bitmap: Vec<u8> = (0..4)
                .flat_map(|i| [17, 99, 231, if mask & (1 << i) == 0 { 0 } else { 255 }])
                .collect();
            let expected = bitmap.clone();
            super::unpremultiply_rgba8(&mut bitmap);
            assert_eq!(bitmap, expected);
        }
    }

    fn measure(kernel: fn(&mut [u8]), initial: &[u8]) -> f64 {
        use std::{hint::black_box, time::Instant};
        let mut bitmap = initial.to_vec();
        let mut samples = Vec::with_capacity(21);
        for _ in 0..21 {
            for _ in 0..8 {
                bitmap.copy_from_slice(initial);
                kernel(black_box(&mut bitmap));
                black_box(&bitmap);
            }
            let start = Instant::now();
            for _ in 0..64 {
                bitmap.copy_from_slice(initial);
                kernel(black_box(&mut bitmap));
                black_box(&bitmap);
            }
            samples.push(start.elapsed().as_secs_f64() * 1e6 / 64.0);
        }
        samples.sort_by(f64::total_cmp);
        samples[10]
    }

    #[test]
    #[ignore = "manual scalar versus SIMD unpremultiplication benchmark"]
    fn profile_unpremultiply_kernel() {
        for pixels in [1_usize, 4, 16, 1024, 4096, 16384] {
            for pattern in ["mixed", "edges", "transparent", "opaque"] {
                let initial: Vec<u8> = (0..pixels)
                    .flat_map(|i| {
                        let alpha = match pattern {
                            "transparent" => 0,
                            "opaque" => 255,
                            "edges" => (i % 254 + 1) as u8,
                            _ if i % 16 < 6 => 0,
                            _ if i % 16 < 12 => 255,
                            _ => (i % 254 + 1) as u8,
                        };
                        let channel = |factor| ((i * factor + 17) % (usize::from(alpha) + 1)) as u8;
                        [channel(71), channel(137), channel(29), alpha]
                    })
                    .collect();
                let mut expected = initial.clone();
                let mut actual = initial.clone();
                reference(&mut expected);
                super::unpremultiply_rgba8(&mut actual);
                assert_eq!(actual, expected);
                let scalar = measure(reference, &initial);
                let simd = measure(super::unpremultiply_rgba8, &initial);
                println!(
                    "pixels={pixels:5} {pattern:11}: scalar={scalar:9.3} us SIMD={simd:9.3} us speedup={:.2}x",
                    scalar / simd
                );
            }
        }
    }
}
