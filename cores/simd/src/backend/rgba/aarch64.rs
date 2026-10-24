use core::arch::aarch64::float32x4_t;
use core::arch::asm;

/// Returns the number of complete pixels processed by NEON.
#[inline]
pub(crate) fn composite(bitmap: &mut [u8], coverage: &[u8], color: [u8; 4]) -> usize {
    let end = (bitmap.len() / 4).min(coverage.len()) / 4 * 4;
    if end == 0 {
        return 0;
    }
    let source_r: float32x4_t;
    let source_g: float32x4_t;
    let source_b: float32x4_t;
    let source_a: float32x4_t;
    let scale: float32x4_t;
    let round: float32x4_t;
    let one: float32x4_t;
    // SAFETY: NEON is baseline. These conversions only produce integer-valued
    // constants in 0..=255. Direct instructions avoid intrinsic wrapper calls
    // in Debug, measured to dominate an otherwise identical implementation.
    unsafe {
        asm!(
            "dup {source_r:v}.4s, {r:w}", "ucvtf {source_r:v}.4s, {source_r:v}.4s",
            "dup {source_g:v}.4s, {g:w}", "ucvtf {source_g:v}.4s, {source_g:v}.4s",
            "dup {source_b:v}.4s, {b:w}", "ucvtf {source_b:v}.4s, {source_b:v}.4s",
            "dup {source_a:v}.4s, {a:w}", "ucvtf {source_a:v}.4s, {source_a:v}.4s",
            "movi {scale:v}.4s, #255", "ucvtf {scale:v}.4s, {scale:v}.4s",
            "movi {round:v}.4s, #127", "ucvtf {round:v}.4s, {round:v}.4s",
            "movi {one:v}.4s, #1", "ucvtf {one:v}.4s, {one:v}.4s",
            r = in(reg) u32::from(color[0]), g = in(reg) u32::from(color[1]),
            b = in(reg) u32::from(color[2]), a = in(reg) u32::from(color[3]),
            source_r = out(vreg) source_r, source_g = out(vreg) source_g,
            source_b = out(vreg) source_b, source_a = out(vreg) source_a,
            scale = out(vreg) scale, round = out(vreg) round, one = out(vreg) one,
            options(nomem, nostack),
        );
    }
    let mut index = 0;
    while index < end {
        // SAFETY: index covers four coverage bytes and sixteen destination
        // bytes within their respective slices. READ_UNALIGNED and LDR/STR
        // permit byte alignment. The destination's exclusive borrow prevents
        // aliasing with coverage. All registers are declared, and floating
        // status changes mean the assembly must not be marked pure.
        let counts = unsafe { coverage.as_ptr().add(index).cast::<u32>().read_unaligned() };
        if counts != 0 {
            let pointer = unsafe { bitmap.as_mut_ptr().add(index * 4) };
            unsafe {
                asm!(
                    "ldr {original:q}, [{pointer}]",
                    "dup {samples:v}.4s, {counts:w}",
                    "ushll {samples:v}.8h, {samples:v}.8b, #0",
                    "ushll {samples:v}.4s, {samples:v}.4h, #0",
                    "ucvtf {samples:v}.4s, {samples:v}.4s",
                    "fmul {alpha:v}.4s, {samples:v}.4s, {source_a:v}.4s",
                    "fadd {alpha:v}.4s, {alpha:v}.4s, {round:v}.4s",
                    "fdiv {alpha:v}.4s, {alpha:v}.4s, {scale:v}.4s",
                    "fcvtzu {alpha:v}.4s, {alpha:v}.4s",
                    "ucvtf {alpha:v}.4s, {alpha:v}.4s",
                    "fcmgt {active:v}.4s, {alpha:v}.4s, #0.0",
                    "fsub {inverse:v}.4s, {scale:v}.4s, {alpha:v}.4s",
                    "ushr {factor:v}.4s, {original:v}.4s, #24",
                    "ucvtf {factor:v}.4s, {factor:v}.4s",
                    "fmul {factor:v}.4s, {factor:v}.4s, {inverse:v}.4s",
                    "fadd {output_alpha:v}.4s, {factor:v}.4s, {round:v}.4s",
                    "fdiv {output_alpha:v}.4s, {output_alpha:v}.4s, {scale:v}.4s",
                    "fcvtzu {output_alpha:v}.4s, {output_alpha:v}.4s",
                    "ucvtf {output_alpha:v}.4s, {output_alpha:v}.4s",
                    "fadd {output_alpha:v}.4s, {alpha:v}.4s, {output_alpha:v}.4s",
                    "fcvtzu {packed:v}.4s, {output_alpha:v}.4s",
                    "ushr {alpha_round:v}.4s, {packed:v}.4s, #1",
                    "ucvtf {alpha_round:v}.4s, {alpha_round:v}.4s",
                    "shl {packed:v}.4s, {packed:v}.4s, #24",
                    "fmax {output_alpha:v}.4s, {output_alpha:v}.4s, {one:v}.4s",
                    "movi {byte_mask:v}.4s, #255",
                    "mov {channel:v}.16b, {original:v}.16b",
                    "and {channel:v}.16b, {channel:v}.16b, {byte_mask:v}.16b",
                    "ucvtf {channel:v}.4s, {channel:v}.4s",
                    "fmul {channel:v}.4s, {channel:v}.4s, {factor:v}.4s",
                    "fadd {channel:v}.4s, {channel:v}.4s, {round:v}.4s",
                    "fdiv {channel:v}.4s, {channel:v}.4s, {scale:v}.4s",
                    "fcvtzu {channel:v}.4s, {channel:v}.4s",
                    "ucvtf {channel:v}.4s, {channel:v}.4s",
                    "fmul {contribution:v}.4s, {source_r:v}.4s, {alpha:v}.4s",
                    "fadd {channel:v}.4s, {channel:v}.4s, {contribution:v}.4s",
                    "fadd {channel:v}.4s, {channel:v}.4s, {alpha_round:v}.4s",
                    "fdiv {channel:v}.4s, {channel:v}.4s, {output_alpha:v}.4s",
                    "fcvtzu {channel:v}.4s, {channel:v}.4s",
                    "and {channel:v}.16b, {channel:v}.16b, {byte_mask:v}.16b",
                    "orr {packed:v}.16b, {packed:v}.16b, {channel:v}.16b",
                    "ushr {channel:v}.4s, {original:v}.4s, #8",
                    "and {channel:v}.16b, {channel:v}.16b, {byte_mask:v}.16b",
                    "ucvtf {channel:v}.4s, {channel:v}.4s",
                    "fmul {channel:v}.4s, {channel:v}.4s, {factor:v}.4s",
                    "fadd {channel:v}.4s, {channel:v}.4s, {round:v}.4s",
                    "fdiv {channel:v}.4s, {channel:v}.4s, {scale:v}.4s",
                    "fcvtzu {channel:v}.4s, {channel:v}.4s",
                    "ucvtf {channel:v}.4s, {channel:v}.4s",
                    "fmul {contribution:v}.4s, {source_g:v}.4s, {alpha:v}.4s",
                    "fadd {channel:v}.4s, {channel:v}.4s, {contribution:v}.4s",
                    "fadd {channel:v}.4s, {channel:v}.4s, {alpha_round:v}.4s",
                    "fdiv {channel:v}.4s, {channel:v}.4s, {output_alpha:v}.4s",
                    "fcvtzu {channel:v}.4s, {channel:v}.4s",
                    "and {channel:v}.16b, {channel:v}.16b, {byte_mask:v}.16b",
                    "shl {channel:v}.4s, {channel:v}.4s, #8",
                    "orr {packed:v}.16b, {packed:v}.16b, {channel:v}.16b",
                    "ushr {channel:v}.4s, {original:v}.4s, #16",
                    "and {channel:v}.16b, {channel:v}.16b, {byte_mask:v}.16b",
                    "ucvtf {channel:v}.4s, {channel:v}.4s",
                    "fmul {channel:v}.4s, {channel:v}.4s, {factor:v}.4s",
                    "fadd {channel:v}.4s, {channel:v}.4s, {round:v}.4s",
                    "fdiv {channel:v}.4s, {channel:v}.4s, {scale:v}.4s",
                    "fcvtzu {channel:v}.4s, {channel:v}.4s",
                    "ucvtf {channel:v}.4s, {channel:v}.4s",
                    "fmul {contribution:v}.4s, {source_b:v}.4s, {alpha:v}.4s",
                    "fadd {channel:v}.4s, {channel:v}.4s, {contribution:v}.4s",
                    "fadd {channel:v}.4s, {channel:v}.4s, {alpha_round:v}.4s",
                    "fdiv {channel:v}.4s, {channel:v}.4s, {output_alpha:v}.4s",
                    "fcvtzu {channel:v}.4s, {channel:v}.4s",
                    "and {channel:v}.16b, {channel:v}.16b, {byte_mask:v}.16b",
                    "shl {channel:v}.4s, {channel:v}.4s, #16",
                    "orr {packed:v}.16b, {packed:v}.16b, {channel:v}.16b",
                    "bsl {active:v}.16b, {packed:v}.16b, {original:v}.16b",
                    "str {active:q}, [{pointer}]",
                    pointer = in(reg) pointer, counts = in(reg) counts,
                    source_r = in(vreg) source_r, source_g = in(vreg) source_g,
                    source_b = in(vreg) source_b, source_a = in(vreg) source_a,
                    scale = in(vreg) scale, round = in(vreg) round, one = in(vreg) one,
                    original = out(vreg) _,
                    samples = out(vreg) _,
                    alpha = out(vreg) _,
                    active = out(vreg) _,
                    inverse = out(vreg) _,
                    factor = out(vreg) _,
                    output_alpha = out(vreg) _,
                    packed = out(vreg) _,
                    alpha_round = out(vreg) _,
                    byte_mask = out(vreg) _,
                    channel = out(vreg) _,
                    contribution = out(vreg) _,
                    options(nostack),
                );
            }
        }
        index += 4;
    }
    end
}
