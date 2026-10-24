//! Working resolution of a large backdrop blur.
//!
//! A Gaussian whose sigma is dozens of pixels is a very smooth function, so
//! blurring the capture at full resolution spends nearly all of its samples on
//! detail the result throws away. The horizontal pass therefore averages
//! `factor` capture rows into each row of a smaller intermediate, and the
//! vertical pass reads that intermediate with a kernel `factor` times narrower
//! and lets bilinear filtering put it back at full size. The reduction keeps
//! the working sigma at four texels or more, where the filter's own
//! interpolation error stays under one level of an 8 bit channel.

/// Smallest sigma, in texels of the working textures, a reduced blur may have.
const MIN_WORKING_SIGMA: f32 = 4.0;
/// The largest reduction. It bounds the rows the horizontal pass averages.
const MAX_DOWNSAMPLE: u32 = 8;

/// How many capture pixels one texel of the blur's working textures covers, for
/// a blur of `radius_px` device pixels (three sigma): `1`, `2`, `4` or `8`.
///
/// A blur that is already narrow, or a radius that is not a number, is not
/// reduced, so it keeps taking exactly the path it always took.
pub(super) fn blur_downsample(radius_px: f32) -> u32 {
    if !radius_px.is_finite() || radius_px <= 0.0 {
        return 1;
    }
    let sigma = radius_px / 3.0;
    let mut factor = 1;
    while factor < MAX_DOWNSAMPLE && sigma / (factor * 2) as f32 >= MIN_WORKING_SIGMA {
        factor *= 2;
    }
    factor
}

/// The support (in texels) and the recurrence of the Gaussian weights for a blur
/// of `radius_px` texels: each weight is `ratio` times the one before, and the
/// ratio itself shrinks by `ratio_squared` per step.
pub(super) fn gaussian_terms(radius_px: f32) -> (f32, f32, f32) {
    let sigma = (radius_px / 3.0).max(0.001);
    let ratio = (-0.5 / (sigma * sigma)).exp();
    (radius_px, ratio, ratio * ratio)
}

/// The size of the working texture for a capture of `width` x `height` pixels
/// reduced by `factor`. It never collapses to nothing.
pub(super) fn reduced_extent(width: u32, height: u32, factor: u32) -> (u32, u32) {
    let factor = factor.max(1);
    (width.div_ceil(factor).max(1), height.div_ceil(factor).max(1))
}

use super::{MaterialUniform, blur_radius_pixels};
use crate::pipeline::material::{MaterialKind, MaterialRequest};

impl MaterialUniform {
    /// Sets the Gaussian terms of the final, vertical pass of a backdrop blur.
    ///
    /// `liquid.x` carries the reduction (`1` when there is none) for the shader:
    /// a backdrop filter has no use for the blob amount it otherwise holds. The
    /// kernel is expressed in texels of the reduced intermediate, so it is
    /// `factor` times narrower than the blur is on screen.
    pub(super) fn set_vertical_blur(&mut self, request: &MaterialRequest) {
        debug_assert_eq!(request.kind, MaterialKind::BackdropFilter);
        let radius = blur_radius_pixels(request);
        let factor = blur_downsample(radius);
        let (support, ratio, ratio_squared) = gaussian_terms(radius / factor as f32);
        self.liquid[0] = factor as f32;
        self.detail[2] = support;
        self.liquid2[2] = ratio;
        self.liquid2[3] = ratio_squared;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_blurs_keep_the_exact_full_resolution_path() {
        // sigma = radius / 3: a 6 logical pixel blur on a 3x phone is 18 pixels.
        assert_eq!(blur_downsample(0.0), 1);
        assert_eq!(blur_downsample(18.0), 1);
        assert_eq!(blur_downsample(23.9), 1);
    }

    #[test]
    fn the_reduction_grows_with_sigma_and_keeps_the_working_sigma_at_least_four() {
        for (radius, expected) in [(24.0, 2), (47.9, 2), (48.0, 4), (95.9, 4), (96.0, 8), (288.0, 8)] {
            let factor = blur_downsample(radius);
            assert_eq!(factor, expected, "radius {radius}");
            assert!(radius / 3.0 / factor as f32 >= 3.99, "radius {radius} works at sigma {}", radius / 3.0 / factor as f32);
        }
    }

    #[test]
    fn a_nonfinite_radius_is_not_reduced() {
        assert_eq!(blur_downsample(f32::NAN), 1);
        assert_eq!(blur_downsample(f32::INFINITY), 1);
        assert_eq!(blur_downsample(-5.0), 1);
    }

    #[test]
    fn the_recurrence_follows_sigma_and_the_support_follows_the_radius() {
        let (support, ratio, ratio2) = gaussian_terms(30.0);
        assert_eq!(support, 30.0);
        // sigma = 10: each weight is exp(-0.5 / sigma^2) times the one before.
        assert!((ratio - (-0.5f32 / 100.0).exp()).abs() < 1e-6);
        assert!((ratio2 - ratio * ratio).abs() < 1e-7);
    }

    #[test]
    fn a_reduced_blur_has_the_same_footprint_in_fewer_texels() {
        let (support, _, _) = gaussian_terms(288.0 / 8.0);
        assert_eq!(support, 36.0);
        assert_eq!(reduced_extent(1470, 552, 8), (184, 69));
        assert_eq!(reduced_extent(1470, 552, 1), (1470, 552));
        assert_eq!(reduced_extent(1, 1, 8), (1, 1));
    }
}
