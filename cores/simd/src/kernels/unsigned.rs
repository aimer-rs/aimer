use super::{LengthMismatch, add_with};

macro_rules! unsigned_add {
    ($name:ident, $number:ty, $lanes:expr) => {
        /// Adds equal-length unsigned slices with the target's SIMD backend.
        ///
        /// Uses NEON on AArch64, SSE2 on x86_64, and scalar addition elsewhere
        /// or with the `force-scalar` feature. Partial vectors use scalar
        /// addition. Scalar alignment is sufficient for all buffers.
        ///
        /// Returns [`LengthMismatch`] without writing output if lengths differ.
        ///
        /// # Panics
        ///
        /// Panics on integer overflow when Rust overflow checks are enabled.
        /// When checks are disabled, each lane wraps. An overflow panic may
        /// leave preceding output elements written.
        #[inline]
        pub fn $name(
            left: &[$number],
            right: &[$number],
            output: &mut [$number],
        ) -> Result<(), LengthMismatch> {
            #[cfg(all(not(feature = "force-scalar"), target_arch = "aarch64"))]
            {
                add_with::<$number, crate::backend::Neon, $lanes>(left, right, output)
            }
            #[cfg(all(not(feature = "force-scalar"), target_arch = "x86_64"))]
            {
                add_with::<$number, crate::backend::Sse2, $lanes>(left, right, output)
            }
            #[cfg(any(
                feature = "force-scalar",
                not(any(target_arch = "aarch64", target_arch = "x86_64"))
            ))]
            {
                add_with::<$number, crate::backend::Scalar, 1>(left, right, output)
            }
        }
    };
}

unsigned_add!(add_u8, u8, 16);
unsigned_add!(add_u16, u16, 8);

#[cfg(test)]
mod tests {
    use super::{add_u8, add_u16};

    #[test]
    fn unsigned_add_handles_empty_vectors_tails_and_scalar_alignment() {
        macro_rules! verify {
            ($number:ty, $lanes:expr, $add:ident) => {
                for len in 0..=4 * $lanes + 1 {
                    for offset in 0..16 {
                        let mut left = vec![0 as $number; len + 16];
                        let right = vec![3 as $number; len + 16];
                        let mut output = vec![99 as $number; len + 16];
                        for (i, value) in left[offset..offset + len].iter_mut().enumerate() {
                            *value = (i * 29 % (<$number>::MAX as usize - 3)) as $number;
                        }
                        $add(
                            &left[offset..offset + len],
                            &right[offset..offset + len],
                            &mut output[offset..offset + len],
                        )
                        .unwrap();
                        let expected: Vec<$number> = left[offset..offset + len]
                            .iter()
                            .map(|x| x.checked_add(3).unwrap())
                            .collect();
                        assert_eq!(&output[offset..offset + len], expected);
                        assert!(
                            output[..offset]
                                .iter()
                                .chain(&output[offset + len..])
                                .all(|&x| x == 99)
                        );
                    }
                }
            };
        }
        verify!(u8, 16, add_u8);
        verify!(u16, 8, add_u16);
    }

    #[test]
    fn unsigned_add_rejects_each_length_mismatch_without_writing() {
        macro_rules! verify {
            ($number:ty, $add:ident) => {
                for (left_len, right_len, output_len) in [(1, 2, 2), (2, 1, 2), (2, 2, 1)] {
                    let left = vec![1 as $number; left_len];
                    let right = vec![2 as $number; right_len];
                    let mut output = vec![99 as $number; output_len];
                    assert_eq!(
                        $add(&left, &right, &mut output),
                        Err(super::LengthMismatch {
                            left: left_len,
                            right: right_len,
                            output: output_len,
                        })
                    );
                    assert_eq!(output, vec![99; output_len]);
                }
            };
        }
        verify!(u8, add_u8);
        verify!(u16, add_u16);
    }

    #[test]
    fn unsigned_add_overflow_matches_scalar_for_vectors_and_tails() {
        use std::hint::black_box;
        use std::panic::{AssertUnwindSafe, catch_unwind};

        macro_rules! verify {
            ($number:ty, $lanes:expr, $add:ident) => {{
                let panics = catch_unwind(|| black_box(<$number>::MAX) + black_box(1)).is_err();
                for len in [1, $lanes, $lanes + 1, $lanes * 4 + 1] {
                    for index in 0..len {
                        let mut left = vec![0 as $number; len];
                        let mut right = vec![0 as $number; len];
                        let mut output = vec![99 as $number; len];
                        left[index] = <$number>::MAX;
                        right[index] = 1;
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            $add(&left, &right, &mut output).unwrap();
                        }));
                        assert_eq!(result.is_err(), panics, "length {len}, index {index}");
                        if result.is_ok() {
                            assert_eq!(output, vec![0; len]);
                        }
                    }
                }
            }};
        }
        verify!(u8, 16, add_u8);
        verify!(u16, 8, add_u16);
    }

    #[test]
    #[ignore = "manual scalar versus unsigned SIMD kernel benchmark"]
    fn profile_unsigned_add_scalar_vs_simd() {
        use std::hint::black_box;
        use std::time::Instant;

        fn measure(mut run: impl FnMut()) -> f64 {
            const ROUNDS: usize = 31;
            const ITERATIONS: usize = 128;
            for _ in 0..16 {
                run();
            }
            let mut samples = [0.0; ROUNDS];
            for sample in &mut samples {
                let start = Instant::now();
                for _ in 0..ITERATIONS {
                    run();
                }
                *sample = start.elapsed().as_secs_f64() * 1e9 / ITERATIONS as f64;
            }
            samples.sort_by(f64::total_cmp);
            samples[ROUNDS / 2]
        }

        macro_rules! measure_type {
            ($number:ty, $add:ident) => {
                for len in [0, 1, 8, 16, 32, 64, 256, 1024, 4096] {
                    let left = vec![100 as $number; len];
                    let right = vec![3 as $number; len];
                    let mut scalar = vec![0 as $number; len];
                    let mut simd = vec![0 as $number; len];
                    let scalar_ns = measure(|| {
                        let left = black_box(left.as_slice());
                        let right = black_box(right.as_slice());
                        let output = black_box(scalar.as_mut_slice());
                        assert_eq!(left.len(), right.len());
                        assert_eq!(left.len(), output.len());
                        for ((a, b), out) in left.iter().zip(right).zip(output) { *out = *a + *b; }
                        black_box(&scalar);
                    });
                    let simd_ns = measure(|| {
                        $add(black_box(&left), black_box(&right), black_box(&mut simd)).unwrap();
                        black_box(&simd);
                    });
                    assert_eq!(scalar, simd);
                    println!("{} len={len:4}: scalar={scalar_ns:.2} ns SIMD={simd_ns:.2} ns speedup={:.2}x",
                        stringify!($number), scalar_ns / simd_ns);
                }
            };
        }
        measure_type!(u8, add_u8);
        measure_type!(u16, add_u16);
    }
}
