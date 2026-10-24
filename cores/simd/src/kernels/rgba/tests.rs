#[test]
fn color_composite_preserves_source_over_order_and_transparent_pixels() {
    let mut bitmap = [0; 20];
    super::composite_rgba8(&mut bitmap, &[255; 5], [255, 0, 0, 255]);
    super::composite_rgba8(&mut bitmap, &[128; 5], [0, 255, 0, 255]);
    assert_eq!(bitmap, [127, 128, 0, 255].repeat(5).as_slice());
    let mut transparent = [17, 33, 91, 0].repeat(5);
    super::composite_rgba8(&mut transparent, &[0; 5], [255; 4]);
    assert_eq!(transparent, [17, 33, 91, 0].repeat(5));
}

// Frozen pre-SIMD Cupid implementation: this is the compatibility oracle,
// including the intentional no-write behavior when effective alpha is zero.
fn reference(bitmap: &mut [u8], coverage: &[u8], color: [u8; 4]) {
    for (pixel, coverage) in bitmap.chunks_exact_mut(4).zip(coverage) {
        let source_alpha = (u32::from(*coverage) * u32::from(color[3]) + 127) / 255;
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

#[test]
fn color_composite_matches_all_alpha_pairs_and_coverage_products() {
    let coverage: Vec<u8> = (0..65536).map(|i| (i / 256) as u8).collect();
    let original: Vec<u8> = (0..65536)
        .flat_map(|i| {
            let a = i as u8;
            [(i / 256) as u8, a.wrapping_mul(73), a.wrapping_add(127), a]
        })
        .collect();
    for alpha in 0_u8..=255 {
        let color = [alpha, 255 - alpha, alpha.wrapping_mul(151), alpha];
        let mut expected = original.clone();
        reference(&mut expected, &coverage, color);
        let mut actual = original.clone();
        super::composite_rgba8(&mut actual, &coverage, color);
        assert_bytes(&actual, &expected, &format!("source alpha {alpha}"));
    }
    // Coverage 255 exposes every effective source/destination alpha pair.
    let full_coverage = vec![255; 65536];
    for color in [[0, 0, 0, 255], [255; 4], [17, 129, 231, 255]] {
        for source_alpha in 0..=255 {
            let color = [color[0], color[1], color[2], source_alpha];
            let mut actual = original.clone();
            let mut expected = original.clone();
            reference(&mut expected, &full_coverage, color);
            super::composite_rgba8(&mut actual, &full_coverage, color);
            assert_bytes(&actual, &expected, &format!("color {color:?}"));
        }
    }
}

#[test]
fn color_composite_preserves_unaligned_buffers_tails_and_shorter_inputs() {
    for bytes in 0..=83 {
        for count in 0..=23 {
            for offset in 0..16 {
                let mut actual: Vec<u8> = (0..bytes + 16).map(|i| (i * 73 + 19) as u8).collect();
                let mut expected = actual.clone();
                let storage: Vec<u8> = (0..count + 16).map(|i| (i * 37) as u8).collect();
                let coverage = &storage[offset..offset + count];
                for color in [
                    [29, 177, 255, 137],
                    [255, 13, 7, 0],
                    [255; 4],
                    [23, 17, 91, 1],
                ] {
                    reference(&mut expected[offset..offset + bytes], coverage, color);
                    super::composite_rgba8(&mut actual[offset..offset + bytes], coverage, color);
                    assert_eq!(
                        actual, expected,
                        "bytes {bytes}, mask {count}, offset {offset}"
                    );
                }
            }
        }
    }
}

#[test]
fn color_composite_mixed_zero_lanes_and_repeated_layers_match_reference() {
    let original: Vec<u8> = (0..4099)
        .flat_map(|i| {
            [
                (i * 71) as u8,
                (i * 137) as u8,
                (i * 29) as u8,
                (i * 47) as u8,
            ]
        })
        .collect();
    let coverage: Vec<u8> = (0..4099)
        .map(|i| match i % 4 {
            0 => 0,
            1 => 1,
            2 => 128,
            _ => 255,
        })
        .collect();
    let mut actual = original.clone();
    let mut expected = original;
    for alpha in [1, 127, 128, 254, 255, 0] {
        let color = [alpha, 255 - alpha, 127, alpha];
        reference(&mut expected, &coverage, color);
        super::composite_rgba8(&mut actual, &coverage, color);
        assert_eq!(actual, expected);
    }
}

#[test]
fn bounded_float_division_reproduces_integer_quotients() {
    // Exhaust the complete range used by destination-channel division.
    for numerator in 0..=255_u32.pow(3) + 127 {
        assert_eq!((numerator as f32 / 255.0) as u32, numerator / 255);
    }
    // Final output division has a smaller numerator and a variable divisor.
    for alpha in 1..=255_u32 {
        for numerator in 0..=255 * alpha + 127 + alpha / 2 {
            assert_eq!((numerator as f32 / alpha as f32) as u32, numerator / alpha);
        }
    }
}

fn assert_bytes(actual: &[u8], expected: &[u8], context: &str) {
    if let Some(index) = actual.iter().zip(expected).position(|(a, b)| a != b) {
        let pixel = index / 4 * 4;
        panic!(
            "{context}, pixel {}: actual {:?}, expected {:?}",
            pixel / 4,
            &actual[pixel..pixel + 4],
            &expected[pixel..pixel + 4]
        );
    }
    assert_eq!(actual.len(), expected.len());
}

#[test]
fn color_composite_preserves_byte_truncation_after_rounding() {
    let mut bitmap = [43, 128, 255, 128].repeat(4);
    super::composite_rgba8(&mut bitmap, &[43; 4], [3, 252, 197, 3]);
    assert_eq!(bitmap, [43, 129, 0, 128].repeat(4));
}

#[test]
#[ignore = "manual cross-platform color-composite kernel benchmark"]
fn profile_color_composite_kernel() {
    use std::{hint::black_box, time::Instant};
    let coverage: Vec<u8> = (0..4096).map(|i| (i * 73 + 17) as u8).collect();
    let initial: Vec<u8> = (0..4096)
        .flat_map(|i| {
            [
                (i * 71) as u8,
                (i * 137) as u8,
                (i * 29) as u8,
                (i * 47) as u8,
            ]
        })
        .collect();
    let mut bitmap = initial.clone();
    for (name, kernel) in [
        ("scalar", reference as fn(&mut [u8], &[u8], [u8; 4])),
        ("SIMD", super::composite_rgba8),
    ] {
        let mut samples = Vec::new();
        for _ in 0..21 {
            let start = Instant::now();
            for _ in 0..100 {
                bitmap.copy_from_slice(&initial);
                kernel(
                    black_box(&mut bitmap),
                    black_box(&coverage),
                    black_box([37, 177, 251, 193]),
                );
                black_box(&bitmap);
            }
            samples.push(start.elapsed().as_secs_f64() * 1e6 / 100.0);
        }
        samples.sort_by(f64::total_cmp);
        println!("{name} dense4096 median={:.3} us", samples[10]);
    }
}
