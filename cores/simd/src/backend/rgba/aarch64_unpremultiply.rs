use core::arch::aarch64::float32x4_t;
use core::arch::asm;

#[inline]
pub(crate) fn unpremultiply(bitmap: &mut [u8]) -> usize {
    let end = bitmap.len() / 16 * 16;
    if end == 0 {
        return 0;
    }
    let scale: float32x4_t;
    let one: float32x4_t;
    // SAFETY: NEON is baseline; conversion produces exact integer constants.
    unsafe {
        asm!(
            "movi {scale:v}.4s, #255", "ucvtf {scale:v}.4s, {scale:v}.4s",
            "movi {one:v}.4s, #1", "ucvtf {one:v}.4s, {one:v}.4s",
            scale = out(vreg) scale, one = out(vreg) one,
            options(nomem, nostack),
        );
    }
    let mut index = 0;
    while index < end {
        // SAFETY: each block addresses sixteen valid exclusive bytes. LDR/STR
        // accept byte alignment. All clobbered registers are declared. The
        // active mask preserves whole pixels with alpha zero or 255; replacing
        // zero divisors by one prevents invalid arithmetic in inactive lanes.
        // Floating-point status can change, so this block is not marked pure.
        unsafe {
            asm!(
                "ldr {original:q}, [{pointer}]",
                "movi {byte_mask:v}.4s, #255",
                "ushr {alpha:v}.4s, {original:v}.4s, #24",
                "cmhi {active:v}.4s, {byte_mask:v}.4s, {alpha:v}.4s",
                "cmtst {channel:v}.4s, {alpha:v}.4s, {alpha:v}.4s",
                "and {active:v}.16b, {active:v}.16b, {channel:v}.16b",
                "umaxv {channel:s}, {active:v}.4s",
                "fmov {any:w}, {channel:s}",
                "cbz {any:w}, 2f",
                "shl {packed:v}.4s, {alpha:v}.4s, #24",
                "ushr {round:v}.4s, {alpha:v}.4s, #1",
                "ucvtf {round:v}.4s, {round:v}.4s",
                "ucvtf {alpha:v}.4s, {alpha:v}.4s",
                "fmax {alpha:v}.4s, {alpha:v}.4s, {one:v}.4s",
                "mov {channel:v}.16b, {original:v}.16b",
                "and {channel:v}.16b, {channel:v}.16b, {byte_mask:v}.16b",
                "ucvtf {channel:v}.4s, {channel:v}.4s",
                "fmul {channel:v}.4s, {channel:v}.4s, {scale:v}.4s",
                "fadd {channel:v}.4s, {channel:v}.4s, {round:v}.4s",
                "fdiv {channel:v}.4s, {channel:v}.4s, {alpha:v}.4s",
                "fcvtzu {channel:v}.4s, {channel:v}.4s",
                "umin {channel:v}.4s, {channel:v}.4s, {byte_mask:v}.4s",
                "orr {packed:v}.16b, {packed:v}.16b, {channel:v}.16b",
                "ushr {channel:v}.4s, {original:v}.4s, #8",
                "and {channel:v}.16b, {channel:v}.16b, {byte_mask:v}.16b",
                "ucvtf {channel:v}.4s, {channel:v}.4s",
                "fmul {channel:v}.4s, {channel:v}.4s, {scale:v}.4s",
                "fadd {channel:v}.4s, {channel:v}.4s, {round:v}.4s",
                "fdiv {channel:v}.4s, {channel:v}.4s, {alpha:v}.4s",
                "fcvtzu {channel:v}.4s, {channel:v}.4s",
                "umin {channel:v}.4s, {channel:v}.4s, {byte_mask:v}.4s",
                "shl {channel:v}.4s, {channel:v}.4s, #8",
                "orr {packed:v}.16b, {packed:v}.16b, {channel:v}.16b",
                "ushr {channel:v}.4s, {original:v}.4s, #16",
                "and {channel:v}.16b, {channel:v}.16b, {byte_mask:v}.16b",
                "ucvtf {channel:v}.4s, {channel:v}.4s",
                "fmul {channel:v}.4s, {channel:v}.4s, {scale:v}.4s",
                "fadd {channel:v}.4s, {channel:v}.4s, {round:v}.4s",
                "fdiv {channel:v}.4s, {channel:v}.4s, {alpha:v}.4s",
                "fcvtzu {channel:v}.4s, {channel:v}.4s",
                "umin {channel:v}.4s, {channel:v}.4s, {byte_mask:v}.4s",
                "shl {channel:v}.4s, {channel:v}.4s, #16",
                "orr {packed:v}.16b, {packed:v}.16b, {channel:v}.16b",
                "bsl {active:v}.16b, {packed:v}.16b, {original:v}.16b",
                "str {active:q}, [{pointer}]",
                "2:",
                pointer = in(reg) bitmap.as_mut_ptr().add(index),
                scale = in(vreg) scale, one = in(vreg) one,
                original = out(vreg) _, byte_mask = out(vreg) _,
                alpha = out(vreg) _, active = out(vreg) _,
                channel = out(vreg) _, packed = out(vreg) _, round = out(vreg) _,
                any = out(reg) _,
                options(nostack),
            );
        }
        index += 16;
    }
    end
}
