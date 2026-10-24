use core::arch::asm;

#[inline]
pub(crate) fn unpremultiply(bitmap: &mut [u8]) -> usize {
    let end = bitmap.len() / 16 * 16;
    let constants = [255.0_f32, 1.0, 0.0, 0.0];
    let mut index = 0;
    while index < end {
        // SAFETY: each block addresses sixteen valid exclusive bytes. MOVUPS
        // permits byte alignment. Constants supply sixteen initialized bytes
        // throughout the block. SSE2 is baseline and all clobbered registers
        // are declared. Alpha-zero/255 pixels are preserved; zero divisors are
        // replaced by one. Floating status changes forbid marking this pure.
        unsafe {
            asm!(
                "movups xmm0, xmmword ptr [{pointer}]",
                "movups xmm9, xmmword ptr [{constants}]",
                "movaps xmm8, xmm9",
                "shufps xmm8, xmm8, 85",
                "shufps xmm9, xmm9, 0",
                "pcmpeqd xmm6, xmm6",
                "psrld xmm6, 24",
                "movdqa xmm1, xmm0",
                "psrld xmm1, 24",
                "movdqa xmm5, xmm6",
                "pcmpgtd xmm5, xmm1",
                "movdqa xmm4, xmm1",
                "pxor xmm7, xmm7",
                "pcmpgtd xmm4, xmm7",
                "pand xmm5, xmm4",
                "pmovmskb {any:e}, xmm5",
                "test {any:e}, {any:e}",
                "jz 2f",
                "movdqa xmm3, xmm1",
                "pslld xmm3, 24",
                "movdqa xmm2, xmm1",
                "psrld xmm2, 1",
                "cvtdq2ps xmm2, xmm2",
                "cvtdq2ps xmm1, xmm1",
                "maxps xmm1, xmm8",
                "movdqa xmm4, xmm0",
                "pand xmm4, xmm6",
                "cvtdq2ps xmm4, xmm4",
                "mulps xmm4, xmm9",
                "addps xmm4, xmm2",
                "divps xmm4, xmm1",
                "minps xmm4, xmm9",
                "cvttps2dq xmm4, xmm4",
                "por xmm3, xmm4",
                "movdqa xmm4, xmm0",
                "psrld xmm4, 8",
                "pand xmm4, xmm6",
                "cvtdq2ps xmm4, xmm4",
                "mulps xmm4, xmm9",
                "addps xmm4, xmm2",
                "divps xmm4, xmm1",
                "minps xmm4, xmm9",
                "cvttps2dq xmm4, xmm4",
                "pslld xmm4, 8",
                "por xmm3, xmm4",
                "movdqa xmm4, xmm0",
                "psrld xmm4, 16",
                "pand xmm4, xmm6",
                "cvtdq2ps xmm4, xmm4",
                "mulps xmm4, xmm9",
                "addps xmm4, xmm2",
                "divps xmm4, xmm1",
                "minps xmm4, xmm9",
                "cvttps2dq xmm4, xmm4",
                "pslld xmm4, 16",
                "por xmm3, xmm4",
                "pand xmm3, xmm5",
                "pandn xmm5, xmm0",
                "por xmm3, xmm5",
                "movups xmmword ptr [{pointer}], xmm3",
                "2:",
                pointer = in(reg) bitmap.as_mut_ptr().add(index),
                constants = in(reg) constants.as_ptr(), any = out(reg) _,
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
                options(nostack),
            );
        }
        index += 16;
    }
    end
}
