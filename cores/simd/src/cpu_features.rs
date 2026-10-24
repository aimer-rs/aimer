//! Runtime CPU feature detection for SIMD backend selection.

/// CPU features that the current process can use for SIMD kernels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuFeatures {
    /// x86 SSE2 support. This is a baseline feature on x86_64.
    pub sse2: bool,
    /// AArch64 Advanced SIMD (NEON) support. This is a baseline feature on
    /// AArch64.
    pub neon: bool,
    /// x86 AVX2 support, including operating-system vector-state support.
    pub avx2: bool,
    /// AArch64 Scalable Vector Extension support reported by the platform.
    ///
    /// The standard library currently reports most AArch64 runtime features
    /// only on Linux; this field is false on platforms where detection is not
    /// available.
    pub sve: bool,
}

impl CpuFeatures {
    /// Detects SIMD features available to the current process.
    ///
    /// Store the result when selecting a backend during initialization rather
    /// than querying it inside a per-vector loop.
    pub fn detect() -> Self {
        Self {
            sse2: detect_sse2(),
            neon: detect_neon(),
            avx2: detect_avx2(),
            sve: detect_sve(),
        }
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
fn detect_sse2() -> bool {
    std::is_x86_feature_detected!("sse2")
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
#[inline]
fn detect_sse2() -> bool {
    false
}

#[cfg(target_arch = "aarch64")]
#[inline]
fn detect_neon() -> bool {
    true
}

#[cfg(not(target_arch = "aarch64"))]
#[inline]
fn detect_neon() -> bool {
    false
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
fn detect_avx2() -> bool {
    std::is_x86_feature_detected!("avx2")
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
#[inline]
fn detect_avx2() -> bool {
    false
}

#[cfg(target_arch = "aarch64")]
#[inline]
fn detect_sve() -> bool {
    std::arch::is_aarch64_feature_detected!("sve")
}

#[cfg(not(target_arch = "aarch64"))]
#[inline]
fn detect_sve() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::CpuFeatures;

    #[test]
    fn reports_baseline_features_for_the_current_architecture() {
        let features = CpuFeatures::detect();

        #[cfg(target_arch = "aarch64")]
        {
            assert!(features.neon);
            assert!(!features.sse2);
        }

        #[cfg(target_arch = "x86_64")]
        {
            assert!(features.sse2);
            assert!(!features.neon);
        }

        let _optional_features = (features.avx2, features.sve);
    }
}
