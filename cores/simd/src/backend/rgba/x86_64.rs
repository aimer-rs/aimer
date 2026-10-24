use core::arch::asm;

/// Returns the number of complete pixels processed by SSE2.
#[inline]
pub(crate) fn composite(bitmap: &mut [u8], coverage: &[u8], color: [u8; 4]) -> usize {
    let end = (bitmap.len() / 4).min(coverage.len()) / 4 * 4;
    if end == 0 {
        return 0;
    }
    let color = [
        f32::from(color[0]),
        f32::from(color[1]),
        f32::from(color[2]),
        f32::from(color[3]),
    ];
    let constants = [255.0_f32, 127.0, 1.0, 0.0];
    let mut index = 0;
    while index < end {
        // SAFETY: index covers four coverage bytes and sixteen destination
        // bytes within their respective slices. Unaligned loads/stores permit
        // byte alignment. Color and constants each supply sixteen readable
        // bytes for the duration of the assembly. The exclusive destination
        // borrow prevents aliasing. SSE2 is baseline, all modified registers
        // are declared, and floating status changes forbid marking this pure.
        let counts = unsafe { coverage.as_ptr().add(index).cast::<u32>().read_unaligned() };
        if counts != 0 {
            let pointer = unsafe { bitmap.as_mut_ptr().add(index * 4) };
            unsafe {
                asm!(
                    "movups xmm0, xmmword ptr [{pointer}]",
                    "movups xmm12, xmmword ptr [{color}]",
                    "movups xmm10, xmmword ptr [{constants}]",
                    "movaps xmm11, xmm10",
                    "shufps xmm11, xmm11, 85",
                    "movaps xmm13, xmm10",
                    "shufps xmm13, xmm13, 170",
                    "shufps xmm10, xmm10, 0",
                    "pxor xmm14, xmm14",
                    "movd xmm1, {counts:e}",
                    "punpcklbw xmm1, xmm14",
                    "punpcklwd xmm1, xmm14",
                    "cvtdq2ps xmm1, xmm1",
                    "movaps xmm8, xmm12",
                    "shufps xmm8, xmm8, 255",
                    "mulps xmm1, xmm8",
                    "addps xmm1, xmm11",
                    "divps xmm1, xmm10",
                    "cvttps2dq xmm1, xmm1",
                    "cvtdq2ps xmm1, xmm1",
                    "movaps xmm5, xmm1",
                    "cmpps xmm5, xmm14, 6",
                    "movaps xmm8, xmm10",
                    "subps xmm8, xmm1",
                    "movdqa xmm2, xmm0",
                    "psrld xmm2, 24",
                    "cvtdq2ps xmm2, xmm2",
                    "mulps xmm2, xmm8",
                    "movaps xmm3, xmm2",
                    "addps xmm3, xmm11",
                    "divps xmm3, xmm10",
                    "cvttps2dq xmm3, xmm3",
                    "cvtdq2ps xmm3, xmm3",
                    "addps xmm3, xmm1",
                    "cvttps2dq xmm4, xmm3",
                    "movdqa xmm9, xmm4",
                    "pslld xmm9, 24",
                    "psrld xmm4, 1",
                    "cvtdq2ps xmm4, xmm4",
                    "maxps xmm3, xmm13",
                    "pcmpeqd xmm6, xmm6",
                    "psrld xmm6, 24",
                    "movdqa xmm7, xmm0",
                    "pand xmm7, xmm6",
                    "cvtdq2ps xmm7, xmm7",
                    "mulps xmm7, xmm2",
                    "addps xmm7, xmm11",
                    "divps xmm7, xmm10",
                    "cvttps2dq xmm7, xmm7",
                    "cvtdq2ps xmm7, xmm7",
                    "movaps xmm8, xmm12",
                    "shufps xmm8, xmm8, 0",
                    "mulps xmm8, xmm1",
                    "addps xmm7, xmm8",
                    "addps xmm7, xmm4",
                    "divps xmm7, xmm3",
                    "cvttps2dq xmm7, xmm7",
                    "pand xmm7, xmm6",
                    "por xmm9, xmm7",
                    "movdqa xmm7, xmm0",
                    "psrld xmm7, 8",
                    "pand xmm7, xmm6",
                    "cvtdq2ps xmm7, xmm7",
                    "mulps xmm7, xmm2",
                    "addps xmm7, xmm11",
                    "divps xmm7, xmm10",
                    "cvttps2dq xmm7, xmm7",
                    "cvtdq2ps xmm7, xmm7",
                    "movaps xmm8, xmm12",
                    "shufps xmm8, xmm8, 85",
                    "mulps xmm8, xmm1",
                    "addps xmm7, xmm8",
                    "addps xmm7, xmm4",
                    "divps xmm7, xmm3",
                    "cvttps2dq xmm7, xmm7",
                    "pand xmm7, xmm6",
                    "pslld xmm7, 8",
                    "por xmm9, xmm7",
                    "movdqa xmm7, xmm0",
                    "psrld xmm7, 16",
                    "pand xmm7, xmm6",
                    "cvtdq2ps xmm7, xmm7",
                    "mulps xmm7, xmm2",
                    "addps xmm7, xmm11",
                    "divps xmm7, xmm10",
                    "cvttps2dq xmm7, xmm7",
                    "cvtdq2ps xmm7, xmm7",
                    "movaps xmm8, xmm12",
                    "shufps xmm8, xmm8, 170",
                    "mulps xmm8, xmm1",
                    "addps xmm7, xmm8",
                    "addps xmm7, xmm4",
                    "divps xmm7, xmm3",
                    "cvttps2dq xmm7, xmm7",
                    "pand xmm7, xmm6",
                    "pslld xmm7, 16",
                    "por xmm9, xmm7",
                    "pand xmm9, xmm5",
                    "pandn xmm5, xmm0",
                    "por xmm9, xmm5",
                    "movups xmmword ptr [{pointer}], xmm9",

                    pointer = in(reg) pointer, counts = in(reg) counts,
                    color = in(reg) color.as_ptr(), constants = in(reg) constants.as_ptr(),
                    out("xmm0") _,
                    out("xmm1") _,
                    out("xmm2") _,
                    out("xmm3") _,
                    out("xmm4") _,
                    out("xmm5") _,
                    out("xmm6") _,
                    out("xmm7") _,
                    out("xmm8") _,
                    out("xmm9") _,
                    out("xmm10") _,
                    out("xmm11") _,
                    out("xmm12") _,
                    out("xmm13") _,
                    out("xmm14") _,
                    options(nostack),
                );
            }
        }
        index += 4;
    }
    end
}
