// A scalar overflow probe preserves Rust's configured overflow-check behavior,
// including -C overflow-checks overrides. The unused arithmetic and its SIMD
// predicate can be eliminated when checks are disabled; debug_assert! would
// incorrectly tie this contract to debug assertions instead of overflow checks.
#[inline(always)]
pub(super) fn check_add_overflow(overflow: bool) {
    let _ = u8::MAX + u8::from(overflow);
}

#[inline(always)]
pub(super) fn check_sub_overflow(overflow: bool) {
    let _ = 0_u8 - u8::from(overflow);
}

#[inline(always)]
pub(super) fn check_mul_overflow(overflow: bool) {
    let _ = u8::MAX * (1 + u8::from(overflow));
}
