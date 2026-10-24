#![cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]

use crate::simd_traits::{Simd, SimdAdd, SimdCompare, SimdMul, SimdSelect, SimdSub};

#[cfg(target_arch = "aarch64")]
use super::Neon as Native;
#[cfg(target_arch = "x86_64")]
use super::Sse2 as Native;

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
#[test]
fn native_unsigned_vectors_transfer_add_compare_and_select() {
    macro_rules! verify {
        ($number:ty, $lanes:expr) => {{
            let left: [$number; $lanes] = core::array::from_fn(|i| {
                [
                    0,
                    1,
                    <$number>::MAX / 2,
                    <$number>::MAX / 2 + 1,
                    <$number>::MAX,
                ][i % 5]
            });
            let right: [$number; $lanes] =
                core::array::from_fn(|i| [<$number>::MAX, 1, 1, 0, 0][i % 5]);
            let a = <Native as Simd<$number, $lanes>>::load(&left);
            let b = <Native as Simd<$number, $lanes>>::load(&right);
            let mut output = [0; $lanes];
            <Native as Simd<$number, $lanes>>::store(a, &mut output);
            assert_eq!(output, left);

            let sum = <Native as SimdAdd<$number, $lanes>>::add(a, b);
            <Native as Simd<$number, $lanes>>::store(sum, &mut output);
            assert_eq!(
                output,
                core::array::from_fn(|i| {
                    [
                        <$number>::MAX,
                        2,
                        <$number>::MAX / 2 + 1,
                        <$number>::MAX / 2 + 1,
                        <$number>::MAX,
                    ][i % 5]
                })
            );

            let mask = <Native as SimdCompare<$number, $lanes>>::gt(a, b);
            let yes = <Native as Simd<$number, $lanes>>::splat(42);
            let no = <Native as Simd<$number, $lanes>>::splat(7);
            let selected = <Native as SimdSelect<$number, $lanes>>::select(mask, yes, no);
            <Native as Simd<$number, $lanes>>::store(selected, &mut output);
            assert_eq!(output, core::array::from_fn(|i| [7, 7, 42, 42, 42][i % 5]));

            // Scalar alignment is sufficient: neither array has to be SIMD-aligned.
            let mut storage = [0 as $number; $lanes + 16];
            let offset = (0..16)
                .find(|&i| storage[i..].as_ptr() as usize % 16 != 0)
                .unwrap();
            storage[offset..offset + $lanes].copy_from_slice(&left);
            let input = storage[offset..offset + $lanes].try_into().unwrap();
            let value = <Native as Simd<$number, $lanes>>::load(input);
            let mut target = [9 as $number; $lanes + 16];
            let start = (0..16)
                .find(|&i| target[i..].as_ptr() as usize % 16 != 0)
                .unwrap();
            <Native as Simd<$number, $lanes>>::store(
                value,
                (&mut target[start..start + $lanes]).try_into().unwrap(),
            );
            assert_eq!(&target[start..start + $lanes], &left);
            assert!(
                target[..start]
                    .iter()
                    .chain(&target[start + $lanes..])
                    .all(|&x| x == 9)
            );
        }};
    }

    verify!(u8, 16);
    verify!(u16, 8);
}

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
#[test]
fn native_unsigned_overflow_matches_scalar_in_every_lane() {
    use std::hint::black_box;
    use std::panic::catch_unwind;

    macro_rules! verify {
        ($number:ty, $lanes:expr) => {{
            let add_panics = catch_unwind(|| black_box(<$number>::MAX) + black_box(1)).is_err();
            let sub_panics = catch_unwind(|| black_box(0 as $number) - black_box(1)).is_err();
            let mul_panics = catch_unwind(|| {
                black_box(<$number>::MAX) * black_box(<$number>::MAX)
            }).is_err();

            for lane in 0..$lanes {
                macro_rules! check {
                    ($trait:ident, $method:ident, $left:expr, $right:expr, $expected:expr, $panics:expr) => {{
                        let mut left = [0 as $number; $lanes];
                        let mut right = [0 as $number; $lanes];
                        left[lane] = $left;
                        right[lane] = $right;
                        let result = catch_unwind(|| {
                            let a = <Native as Simd<$number, $lanes>>::load(&left);
                            let b = <Native as Simd<$number, $lanes>>::load(&right);
                            let value = <Native as $trait<$number, $lanes>>::$method(a, b);
                            let mut output = [0; $lanes];
                            <Native as Simd<$number, $lanes>>::store(value, &mut output);
                            output
                        });
                        assert_eq!(result.is_err(), $panics, "{} lane {lane}", stringify!($method));
                        if let Ok(output) = result {
                            let mut expected = [0; $lanes];
                            expected[lane] = $expected;
                            assert_eq!(output, expected);
                        }
                    }};
                }
                check!(SimdAdd, add, <$number>::MAX, 1, 0, add_panics);
                check!(SimdSub, sub, 0, 1, <$number>::MAX, sub_panics);
                check!(SimdMul, mul, <$number>::MAX, <$number>::MAX, 1, mul_panics);
            }
        }};
    }

    verify!(u8, 16);
    verify!(u16, 8);
}

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
#[test]
fn native_u8_operations_match_scalar_for_all_operand_pairs() {
    for left in 0..=u8::MAX {
        let a = <Native as Simd<u8, 16>>::splat(left);
        for right in 0..=u8::MAX {
            let b = <Native as Simd<u8, 16>>::splat(right);
            let mut output = [0; 16];
            let mask = <Native as SimdCompare<u8, 16>>::gt(a, b);
            let selected = <Native as SimdSelect<u8, 16>>::select(
                mask,
                <Native as Simd<u8, 16>>::splat(1),
                <Native as Simd<u8, 16>>::splat(0),
            );
            <Native as Simd<u8, 16>>::store(selected, &mut output);
            assert_eq!(output, [u8::from(left > right); 16]);

            macro_rules! check {
                ($checked:ident, $trait:ident, $method:ident) => {
                    if let Some(expected) = left.$checked(right) {
                        let value = <Native as $trait<u8, 16>>::$method(a, b);
                        <Native as Simd<u8, 16>>::store(value, &mut output);
                        assert_eq!(
                            output,
                            [expected; 16],
                            "{}({left}, {right})",
                            stringify!($method)
                        );
                    }
                };
            }
            check!(checked_add, SimdAdd, add);
            check!(checked_sub, SimdSub, sub);
            check!(checked_mul, SimdMul, mul);
        }
    }
}

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
#[test]
fn native_unsigned_vectors_subtract_and_multiply_at_boundaries() {
    macro_rules! verify {
        ($number:ty, $lanes:expr) => {{
            let left: [$number; $lanes] = core::array::from_fn(|i| {
                [0, <$number>::MAX, <$number>::MAX / 2 + 1, 15, 16, 1, 2, 3][i % 8]
            });
            let factors: [$number; $lanes] = core::array::from_fn(|i| {
                [
                    <$number>::MAX,
                    1,
                    1,
                    15,
                    15,
                    <$number>::MAX,
                    <$number>::MAX / 2,
                    0,
                ][i % 8]
            });
            let subtrahends: [$number; $lanes] =
                core::array::from_fn(|i| [0, <$number>::MAX, 1, 1, 15, 1, 1, 3][i % 8]);
            let a = <Native as Simd<$number, $lanes>>::load(&left);
            let b = <Native as Simd<$number, $lanes>>::load(&factors);
            let c = <Native as Simd<$number, $lanes>>::load(&subtrahends);
            let mut output = [0; $lanes];
            let product = <Native as SimdMul<$number, $lanes>>::mul(a, b);
            <Native as Simd<$number, $lanes>>::store(product, &mut output);
            assert_eq!(
                output,
                core::array::from_fn(|i| {
                    [
                        0,
                        <$number>::MAX,
                        <$number>::MAX / 2 + 1,
                        225,
                        240,
                        <$number>::MAX,
                        <$number>::MAX - 1,
                        0,
                    ][i % 8]
                })
            );
            let difference = <Native as SimdSub<$number, $lanes>>::sub(a, c);
            <Native as Simd<$number, $lanes>>::store(difference, &mut output);
            assert_eq!(
                output,
                core::array::from_fn(|i| { [0, 0, <$number>::MAX / 2, 14, 1, 0, 1, 0][i % 8] })
            );
        }};
    }

    verify!(u8, 16);
    verify!(u16, 8);
}
