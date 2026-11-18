//! Scanning byte slices: where the first interesting byte is, and how many there are.
//!
//! Text is bytes, and a text scan asks a few questions of them over and over: is it all ASCII,
//! where is the next line feed, where does the first byte that needs special handling start. A
//! byte loop answers one byte at a time. These kernels compare sixteen at once and read the
//! answer off a bitmask, with a scalar loop for the last fifteen bytes. On AArch64 they use
//! NEON and on x86_64 SSE2, and everywhere else, or with the `force-scalar` feature, they are
//! the scalar loop alone; the answers are the same on every backend.
//!
//! Nothing is allocated, and no alignment is needed: the slices may start anywhere.

use crate::simd_traits::{Simd, SimdCompare, SimdEq, SimdMask};

#[cfg(all(not(feature = "force-scalar"), target_arch = "aarch64"))]
type Backend = crate::backend::Neon;
#[cfg(all(not(feature = "force-scalar"), target_arch = "x86_64"))]
type Backend = crate::backend::Sse2;

/// Bytes a vector holds.
#[cfg(all(
    not(feature = "force-scalar"),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
const LANES: usize = 16;

/// Bytes tested for a match with one branch: four vectors.
#[cfg(all(
    not(feature = "force-scalar"),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
const WIDE: usize = 4 * LANES;

/// The index of the first byte of `bytes` that `block_mask` marks, found sixty-four bytes at a
/// time while that many remain, then sixteen, then one.
///
/// `block_mask` and `scalar` must agree: lane `i` of the mask for a block is true exactly when
/// `scalar` is true of byte `i` of it. The masks of four vectors are ORed and tested together,
/// so the common case, no match in sixty-four bytes, costs one test and not four; only a block
/// with a match is looked at lane by lane.
#[cfg(all(
    not(feature = "force-scalar"),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
#[inline(always)]
fn find_first_with<K: Simd<u8, LANES> + SimdMask<u8, LANES>>(
    bytes: &[u8],
    block_mask: impl Fn(<K as Simd<u8, LANES>>::Vector) -> <K as Simd<u8, LANES>>::Mask,
    scalar: impl Fn(u8) -> bool,
) -> Option<usize> {
    let load = |index: usize| {
        // SAFETY: every caller has checked index + 16 <= bytes.len(), so the block is sixteen
        // valid, initialized bytes; a [u8; 16] needs no more alignment than a byte.
        let block = unsafe { &*bytes.as_ptr().add(index).cast::<[u8; LANES]>() };
        block_mask(<K as Simd<u8, LANES>>::load(block))
    };
    let mut index = 0;
    let wide_end = bytes.len() / WIDE * WIDE;
    while index < wide_end {
        let masks = [load(index), load(index + LANES), load(index + 2 * LANES), load(index + 3 * LANES)];
        let any = <K as SimdMask<u8, LANES>>::any(<K as SimdMask<u8, LANES>>::or(
            <K as SimdMask<u8, LANES>>::or(masks[0], masks[1]),
            <K as SimdMask<u8, LANES>>::or(masks[2], masks[3]),
        ));
        if any {
            for (block, mask) in masks.into_iter().enumerate() {
                let bits = <K as SimdMask<u8, LANES>>::bitmask(mask);
                if bits != 0 {
                    return Some(index + block * LANES + bits.trailing_zeros() as usize);
                }
            }
        }
        index += WIDE;
    }
    let vector_end = bytes.len() / LANES * LANES;
    while index < vector_end {
        let bits = <K as SimdMask<u8, LANES>>::bitmask(load(index));
        if bits != 0 {
            return Some(index + bits.trailing_zeros() as usize);
        }
        index += LANES;
    }
    bytes[index..].iter().position(|&byte| scalar(byte)).map(|found| index + found)
}

/// Returns the index of the first byte of `haystack` equal to `needle`, or `None`.
///
/// Compares sixteen bytes at a time with NEON on AArch64 and SSE2 on x86_64, and one at a time
/// elsewhere or with the `force-scalar` feature. No allocation or alignment is required.
///
/// # Examples
///
/// ```
/// use aimer_simd::kernels::find_byte;
///
/// let text = b"first line\nsecond line\n";
/// assert_eq!(find_byte(text, b'\n'), Some(10));
/// assert_eq!(find_byte(text, b'#'), None);
/// assert_eq!(find_byte(b"", b'\n'), None);
/// ```
// An `#[inline]` function is compiled in the crate that calls it, at that crate's optimization
// level, so in a debug build the profile that optimizes this crate would not reach it. Not
// inlining it there makes it a function of this crate's own, compiled once, optimized.
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(not(debug_assertions), inline)]
pub fn find_byte(haystack: &[u8], needle: u8) -> Option<usize> {
    #[cfg(all(
        not(feature = "force-scalar"),
        any(target_arch = "aarch64", target_arch = "x86_64")
    ))]
    {
        let target = <Backend as Simd<u8, LANES>>::splat(needle);
        find_first_with::<Backend>(
            haystack,
            |block| <Backend as SimdEq<u8, LANES>>::eq(block, target),
            |byte| byte == needle,
        )
    }
    #[cfg(any(
        feature = "force-scalar",
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    ))]
    {
        haystack.iter().position(|&byte| byte == needle)
    }
}

/// Returns the index of the first byte of `haystack` that is at least `threshold`, or `None`.
///
/// With a threshold of `0x80` this finds the first byte that is not ASCII, which in UTF-8 is
/// the first byte of a multi-byte character; with `0xC0` the first lead byte past ASCII.
///
/// # Examples
///
/// ```
/// use aimer_simd::kernels::find_byte_at_least;
///
/// assert_eq!(find_byte_at_least("café".as_bytes(), 0x80), Some(3));
/// assert_eq!(find_byte_at_least(b"plain", 0x80), None);
/// // A threshold of zero is met by every byte.
/// assert_eq!(find_byte_at_least(b"abc", 0), Some(0));
/// ```
// An `#[inline]` function is compiled in the crate that calls it, at that crate's optimization
// level, so in a debug build the profile that optimizes this crate would not reach it. Not
// inlining it there makes it a function of this crate's own, compiled once, optimized.
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(not(debug_assertions), inline)]
pub fn find_byte_at_least(haystack: &[u8], threshold: u8) -> Option<usize> {
    if threshold == 0 {
        return (!haystack.is_empty()).then_some(0);
    }
    #[cfg(all(
        not(feature = "force-scalar"),
        any(target_arch = "aarch64", target_arch = "x86_64")
    ))]
    {
        // `gt` is the comparison the backends have, so "at least t" is "greater than t - 1".
        let below = <Backend as Simd<u8, LANES>>::splat(threshold - 1);
        find_first_with::<Backend>(
            haystack,
            |block| <Backend as SimdCompare<u8, LANES>>::gt(block, below),
            |byte| byte >= threshold,
        )
    }
    #[cfg(any(
        feature = "force-scalar",
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    ))]
    {
        haystack.iter().position(|&byte| byte >= threshold)
    }
}

/// Returns the index of the first byte of `haystack` outside `low..=high`, or `None`.
///
/// An empty range (`low > high`) contains no byte, so the first byte of a non-empty slice is
/// outside it.
///
/// # Examples
///
/// ```
/// use aimer_simd::kernels::find_byte_outside;
///
/// // The first byte that is not printable ASCII.
/// assert_eq!(find_byte_outside(b"plain text\n", b' ', b'~'), Some(10));
/// assert_eq!(find_byte_outside(b"plain text", b' ', b'~'), None);
/// ```
// An `#[inline]` function is compiled in the crate that calls it, at that crate's optimization
// level, so in a debug build the profile that optimizes this crate would not reach it. Not
// inlining it there makes it a function of this crate's own, compiled once, optimized.
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(not(debug_assertions), inline)]
pub fn find_byte_outside(haystack: &[u8], low: u8, high: u8) -> Option<usize> {
    if low > high {
        return (!haystack.is_empty()).then_some(0);
    }
    #[cfg(all(
        not(feature = "force-scalar"),
        any(target_arch = "aarch64", target_arch = "x86_64")
    ))]
    {
        let low_vector = <Backend as Simd<u8, LANES>>::splat(low);
        let high_vector = <Backend as Simd<u8, LANES>>::splat(high);
        find_first_with::<Backend>(
            haystack,
            |block| {
                // Below the range, or above it.
                <Backend as SimdMask<u8, LANES>>::or(
                    <Backend as SimdCompare<u8, LANES>>::gt(low_vector, block),
                    <Backend as SimdCompare<u8, LANES>>::gt(block, high_vector),
                )
            },
            |byte| byte < low || byte > high,
        )
    }
    #[cfg(any(
        feature = "force-scalar",
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    ))]
    {
        haystack.iter().position(|&byte| byte < low || byte > high)
    }
}

/// Whether every byte of `haystack` is ASCII (below `0x80`). The empty slice is.
///
/// # Examples
///
/// ```
/// use aimer_simd::kernels::is_ascii;
///
/// assert!(is_ascii(b"plain text"));
/// assert!(!is_ascii("café".as_bytes()));
/// assert!(is_ascii(b""));
/// ```
// An `#[inline]` function is compiled in the crate that calls it, at that crate's optimization
// level, so in a debug build the profile that optimizes this crate would not reach it. Not
// inlining it there makes it a function of this crate's own, compiled once, optimized.
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(not(debug_assertions), inline)]
pub fn is_ascii(haystack: &[u8]) -> bool {
    find_byte_at_least(haystack, 0x80).is_none()
}

/// Counts the bytes of `haystack` equal to `needle`.
///
/// # Examples
///
/// ```
/// use aimer_simd::kernels::count_byte;
///
/// assert_eq!(count_byte(b"a\nb\n\nc", b'\n'), 3);
/// assert_eq!(count_byte(b"", b'\n'), 0);
/// ```
// An `#[inline]` function is compiled in the crate that calls it, at that crate's optimization
// level, so in a debug build the profile that optimizes this crate would not reach it. Not
// inlining it there makes it a function of this crate's own, compiled once, optimized.
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(not(debug_assertions), inline)]
pub fn count_byte(haystack: &[u8], needle: u8) -> usize {
    #[cfg(all(
        not(feature = "force-scalar"),
        any(target_arch = "aarch64", target_arch = "x86_64")
    ))]
    {
        let target = <Backend as Simd<u8, LANES>>::splat(needle);
        let vector_end = haystack.len() / LANES * LANES;
        let mut count = 0;
        let mut index = 0;
        while index < vector_end {
            // SAFETY: index + 16 <= vector_end <= haystack.len(), so the block is sixteen
            // valid, initialized bytes; a [u8; 16] needs no more alignment than a byte.
            let block = unsafe { &*haystack.as_ptr().add(index).cast::<[u8; LANES]>() };
            let bits = <Backend as SimdMask<u8, LANES>>::bitmask(<Backend as SimdEq<u8, LANES>>::eq(
                <Backend as Simd<u8, LANES>>::load(block),
                target,
            ));
            count += bits.count_ones() as usize;
            index += LANES;
        }
        count + haystack[index..].iter().filter(|&&byte| byte == needle).count()
    }
    #[cfg(any(
        feature = "force-scalar",
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    ))]
    {
        haystack.iter().filter(|&&byte| byte == needle).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator, so a failure reproduces.
    struct Random(u64);

    impl Random {
        fn byte(&mut self) -> u8 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 24) as u8
        }
    }

    fn random_bytes(random: &mut Random, len: usize) -> Vec<u8> {
        (0..len).map(|_| random.byte()).collect()
    }

    /// The results of every kernel against the plain loop it stands for, for `bytes`.
    #[track_caller]
    fn assert_all_agree(bytes: &[u8], needle: u8, threshold: u8, low: u8, high: u8) {
        let what = format!("len {} needle {needle:#x} threshold {threshold:#x} range {low:#x}..={high:#x}", bytes.len());
        assert_eq!(find_byte(bytes, needle), bytes.iter().position(|&b| b == needle), "find_byte, {what}");
        assert_eq!(
            find_byte_at_least(bytes, threshold),
            bytes.iter().position(|&b| b >= threshold),
            "find_byte_at_least, {what}"
        );
        assert_eq!(
            find_byte_outside(bytes, low, high),
            bytes.iter().position(|&b| b < low || b > high),
            "find_byte_outside, {what}"
        );
        assert_eq!(is_ascii(bytes), bytes.is_ascii(), "is_ascii, {what}");
        assert_eq!(count_byte(bytes, needle), bytes.iter().filter(|&&b| b == needle).count(), "count_byte, {what}");
    }

    #[test]
    fn empty_input_has_nothing_to_find() {
        assert_eq!(find_byte(&[], 0), None);
        assert_eq!(find_byte_at_least(&[], 0), None);
        assert_eq!(find_byte_at_least(&[], 0x80), None);
        assert_eq!(find_byte_outside(&[], 5, 4), None);
        assert_eq!(find_byte_outside(&[], 0, 255), None);
        assert!(is_ascii(&[]));
        assert_eq!(count_byte(&[], 0), 0);
    }

    /// The one byte that is different, at every position of every length through several
    /// vectors and a tail: it is found at exactly that position, and nothing else is.
    #[test]
    fn a_single_match_is_found_at_every_position_of_every_length() {
        for len in 0..=70 {
            for at in 0..len {
                let mut bytes = vec![b'a'; len];
                bytes[at] = b'\n';
                assert_eq!(find_byte(&bytes, b'\n'), Some(at), "len {len}");
                assert_eq!(count_byte(&bytes, b'\n'), 1, "len {len}");

                let mut text = vec![b'a'; len];
                text[at] = 0xC3;
                assert_eq!(find_byte_at_least(&text, 0x80), Some(at), "len {len}");
                assert!(!is_ascii(&text), "len {len}");

                let mut printable = vec![b'q'; len];
                printable[at] = b'\t';
                assert_eq!(find_byte_outside(&printable, b' ', b'~'), Some(at), "len {len}");
                printable[at] = 0xFF;
                assert_eq!(find_byte_outside(&printable, b' ', b'~'), Some(at), "len {len}");
            }
            assert_eq!(find_byte(&vec![b'a'; len], b'\n'), None);
            assert!(is_ascii(&vec![b'a'; len]));
            assert_eq!(find_byte_outside(&vec![b'q'; len], b' ', b'~'), None);
        }
    }

    /// The first of several matches is the answer, and all of them are counted.
    #[test]
    fn the_first_of_many_matches_is_the_one_found() {
        let mut bytes = vec![b'a'; 100];
        for at in [37, 38, 63, 64, 99] {
            bytes[at] = b'\n';
        }
        assert_eq!(find_byte(&bytes, b'\n'), Some(37));
        assert_eq!(count_byte(&bytes, b'\n'), 5);
    }

    #[test]
    fn random_bytes_agree_with_the_plain_loops_whatever_the_alignment() {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        let backing = random_bytes(&mut random, 600);
        for len in (0..=80).chain([127, 128, 129, 255, 256, 257, 500]) {
            for offset in 0..20 {
                let bytes = &backing[offset..offset + len];
                for _ in 0..6 {
                    let (a, b) = (random.byte(), random.byte());
                    assert_all_agree(bytes, random.byte(), random.byte(), a.min(b), a.max(b));
                }
            }
        }
    }

    #[test]
    fn every_threshold_needle_and_range_edge_agrees_on_text_like_bytes() {
        // Mostly ASCII with a few lead bytes, as text is, so the matches are rare and a mistake
        // at a boundary value shows.
        let mut random = Random(0xDEAD_BEEF_CAFE_F00D);
        let mut bytes: Vec<u8> = (0..300).map(|_| b' ' + random.byte() % 95).collect();
        bytes[150] = 0xD6;
        bytes[299] = 0x80;
        for threshold in [0, 1, 0x20, 0x7F, 0x80, 0xBF, 0xC0, 0xD5, 0xD6, 0xD7, 0xFE, 0xFF] {
            for needle in [0, b'\n', b' ', b'~', 0x7F, 0x80, 0xD6, 0xFF] {
                for (low, high) in [(b' ', b'~'), (0, 0), (255, 255), (0, 255), (5, 4), (0x7F, 0x80), (b'a', b'a')] {
                    assert_all_agree(&bytes, needle, threshold, low, high);
                }
            }
        }
        assert_eq!(find_byte_at_least(&bytes, 0xD6), Some(150));
        assert_eq!(find_byte_at_least(&bytes, 0x80), Some(150));
        assert_eq!(find_byte_at_least(&bytes, 0xD7), None);
    }

    /// The same scan with the unsigned comparison the backends build from signed ones: bytes
    /// on either side of 0x7F and 0x80 are where a signed compare goes wrong.
    #[test]
    fn bytes_around_the_sign_boundary_compare_as_unsigned() {
        let bytes: Vec<u8> = (0..=255u8).collect();
        for threshold in 1..=255u8 {
            assert_eq!(find_byte_at_least(&bytes, threshold), Some(usize::from(threshold)));
        }
        assert_eq!(find_byte_at_least(&bytes, 0x80), Some(0x80));
        assert_eq!(find_byte_outside(&bytes, 0x10, 0xEF), Some(0));
        assert_eq!(find_byte_outside(&bytes[0x10..], 0x10, 0xEF), Some(0xF0 - 0x10));
        for needle in 0..=255u8 {
            assert_eq!(find_byte(&bytes, needle), Some(usize::from(needle)));
            assert_eq!(count_byte(&bytes, needle), 1);
        }
    }

    /// What the kernels cost against the loops they replace, in whatever profile the tests are
    /// built in (a debug build, where a byte loop is slowest):
    ///
    /// ```text
    /// cargo nextest run -p aimer_simd --run-ignored only -E 'test(bench_scanning)' --no-capture
    /// ```
    #[test]
    #[ignore = "a timing, not a check"]
    fn bench_scanning() {
        use std::hint::black_box;
        use std::time::Instant;

        let mut text: Vec<u8> = (0..64 * 1024).map(|index| b' ' + (index % 95) as u8).collect();
        let last = text.len() - 1;
        text[last] = 0xC3; // the only non-ASCII byte, at the very end: a full scan
        let time = |what: &str, mut run: Box<dyn FnMut() -> usize>| {
            let mut best = std::time::Duration::MAX;
            for _ in 0..30 {
                let started = Instant::now();
                black_box(run());
                best = best.min(started.elapsed());
            }
            println!("{what:<34} {best:>10.2?}  ({:.2} GB/s)", 64.0 * 1024.0 / best.as_secs_f64() / 1e9);
        };
        let text = &text;
        time("find non-ASCII: iter().position", Box::new(move || black_box(text).iter().position(|&b| b >= 0x80).unwrap_or(0)));
        time("find non-ASCII: find_byte_at_least", Box::new(move || find_byte_at_least(black_box(text), 0x80).unwrap_or(0)));
        time("is_ascii: std", Box::new(move || usize::from(black_box(text).is_ascii())));
        time("is_ascii: kernel", Box::new(move || usize::from(is_ascii(black_box(text)))));
        time("find b'~' (last)", Box::new(move || black_box(text).iter().position(|&b| b == 0xC3).unwrap_or(0)));
        time("find b'~' (last): kernel", Box::new(move || find_byte(black_box(text), 0xC3).unwrap_or(0)));
        time("count '\\n': filter().count()", Box::new(move || black_box(text).iter().filter(|&&b| b == b'~').count()));
        time("count '\\n': kernel", Box::new(move || count_byte(black_box(text), b'~')));
        time("outside ' '..='~': position", Box::new(move || black_box(text).iter().position(|&b| !(b' '..=b'~').contains(&b)).unwrap_or(0)));
        time("outside ' '..='~': kernel", Box::new(move || find_byte_outside(black_box(text), b' ', b'~').unwrap_or(0)));
    }
}
