use super::{ColorRgba, composite_color_layer};

#[test]
fn color_layers_preserve_vector_boundaries_and_source_over_order() {
    let mut bitmap = [0; 19 * 4];
    composite_color_layer(&mut bitmap, &[255; 19], ColorRgba::new(255, 0, 0, 255));
    let coverage: Vec<u8> = (0..19).map(|i| [0, 128, 255][i % 3]).collect();
    composite_color_layer(&mut bitmap, &coverage, ColorRgba::new(0, 255, 0, 255));
    for (i, pixel) in bitmap.chunks_exact(4).enumerate() {
        assert_eq!(
            pixel,
            match i % 3 {
                0 => [255, 0, 0, 255],
                1 => [127, 128, 0, 255],
                _ => [0, 255, 0, 255],
            }
        );
    }
}

// Frozen scalar implementation used before SIMD integration.
fn reference(bitmap: &mut [u8], coverage: &[u8], color: ColorRgba) {
    for (pixel, coverage) in bitmap.chunks_exact_mut(4).zip(coverage) {
        let source_alpha = (u32::from(*coverage) * u32::from(color.alpha) + 127) / 255;
        if source_alpha == 0 {
            continue;
        }
        let destination_alpha = u32::from(pixel[3]);
        let inverse_source_alpha = 255 - source_alpha;
        let output_alpha = source_alpha + (destination_alpha * inverse_source_alpha + 127) / 255;
        let destination_factor = destination_alpha * inverse_source_alpha;
        pixel[0] = ((u32::from(color.red) * source_alpha
            + (u32::from(pixel[0]) * destination_factor + 127) / 255
            + output_alpha / 2)
            / output_alpha) as u8;
        pixel[1] = ((u32::from(color.green) * source_alpha
            + (u32::from(pixel[1]) * destination_factor + 127) / 255
            + output_alpha / 2)
            / output_alpha) as u8;
        pixel[2] = ((u32::from(color.blue) * source_alpha
            + (u32::from(pixel[2]) * destination_factor + 127) / 255
            + output_alpha / 2)
            / output_alpha) as u8;
        pixel[3] = output_alpha as u8;
    }
}

fn measure(
    kernel: fn(&mut [u8], &[u8], ColorRgba),
    initial: &[u8],
    coverage: &[u8],
    color: ColorRgba,
) -> f64 {
    use std::{hint::black_box, time::Instant};
    let mut bitmap = initial.to_vec();
    let mut samples = Vec::with_capacity(21);
    for _ in 0..21 {
        for _ in 0..8 {
            bitmap.copy_from_slice(initial);
            kernel(
                black_box(&mut bitmap),
                black_box(coverage),
                black_box(color),
            );
            black_box(&bitmap);
        }
        let start = Instant::now();
        for _ in 0..64 {
            bitmap.copy_from_slice(initial);
            kernel(
                black_box(&mut bitmap),
                black_box(coverage),
                black_box(color),
            );
            black_box(&bitmap);
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / 64.0);
    }
    samples.sort_by(f64::total_cmp);
    samples[10]
}

#[test]
#[ignore = "manual scalar versus SIMD color-layer benchmark"]
fn profile_color_composite() {
    for pixels in [1, 4, 16, 1024, 4096, 16384] {
        for pattern in ["dense", "sparse", "zero", "first-layer"] {
            let initial: Vec<u8> = (0..pixels)
                .flat_map(|i| {
                    if pattern == "first-layer" {
                        [0; 4]
                    } else {
                        [
                            (i * 71) as u8,
                            (i * 137) as u8,
                            (i * 29) as u8,
                            (i * 47) as u8,
                        ]
                    }
                })
                .collect();
            let coverage: Vec<u8> = (0..pixels)
                .map(|i| match pattern {
                    "zero" => 0,
                    "sparse" if i % 16 < 12 => 0,
                    _ => (i * 73 + 17) as u8,
                })
                .collect();
            let color = ColorRgba::new(37, 177, 251, 193);
            let mut expected = initial.clone();
            let mut actual = initial.clone();
            reference(&mut expected, &coverage, color);
            composite_color_layer(&mut actual, &coverage, color);
            assert_eq!(actual, expected);
            let scalar = measure(reference, &initial, &coverage, color);
            let simd = measure(composite_color_layer, &initial, &coverage, color);
            println!(
                "pixels={pixels:5} {pattern:11}: scalar={scalar:9.3} us SIMD={simd:9.3} us speedup={:.2}x",
                scalar / simd
            );
        }
    }
}
