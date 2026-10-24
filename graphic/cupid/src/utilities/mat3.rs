/// A 3x3 column-major matrix for 2D transforms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3 {
    pub cols: [[f32; 3]; 3],
}

impl Mat3 {
    pub const fn identity() -> Self {
        Self {
            cols: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    /// Whether this matrix is exactly the identity.
    ///
    /// Compares component-wise, so `-0.0` counts as zero and any `NaN` makes
    /// the answer `false`.
    #[inline]
    pub fn is_identity(&self) -> bool {
        // Spelled out: comparing the nested arrays goes through the generic
        // slice-equality machinery, which is several calls per row in an
        // unoptimized build and this runs for every node of every frame.
        let [x, y, w] = &self.cols;
        x[0] == 1.0
            && x[1] == 0.0
            && x[2] == 0.0
            && y[0] == 0.0
            && y[1] == 1.0
            && y[2] == 0.0
            && w[0] == 0.0
            && w[1] == 0.0
            && w[2] == 1.0
    }

    pub const fn translate(tx: f32, ty: f32) -> Self {
        Self {
            cols: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [tx, ty, 1.0]],
        }
    }

    /// Multiply self * other (apply self after other).
    ///
    /// Written out without loops: this runs for every node on every frame, and
    /// in an unoptimized build the nested range iterators and bounds checks
    /// cost more than the arithmetic. Each element is still evaluated as
    /// `a0*b0 + a1*b1 + a2*b2`, left to right, so results are bit-identical to
    /// the loop form.
    #[inline]
    pub fn mul(&self, other: &Mat3) -> Mat3 {
        let [a0, a1, a2] = self.cols;
        let [b0, b1, b2] = other.cols;
        Mat3 {
            cols: [
                [
                    a0[0] * b0[0] + a1[0] * b0[1] + a2[0] * b0[2],
                    a0[1] * b0[0] + a1[1] * b0[1] + a2[1] * b0[2],
                    a0[2] * b0[0] + a1[2] * b0[1] + a2[2] * b0[2],
                ],
                [
                    a0[0] * b1[0] + a1[0] * b1[1] + a2[0] * b1[2],
                    a0[1] * b1[0] + a1[1] * b1[1] + a2[1] * b1[2],
                    a0[2] * b1[0] + a1[2] * b1[1] + a2[2] * b1[2],
                ],
                [
                    a0[0] * b2[0] + a1[0] * b2[1] + a2[0] * b2[2],
                    a0[1] * b2[0] + a1[1] * b2[1] + a2[1] * b2[2],
                    a0[2] * b2[0] + a1[2] * b2[1] + a2[2] * b2[2],
                ],
            ],
        }
    }

    pub fn scale(sx: f32, sy: f32) -> Self {
        Self {
            cols: [[sx, 0.0, 0.0], [0.0, sy, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    pub fn rotate(radians: f32) -> Self {
        let c = radians.cos();
        let s = radians.sin();
        Self {
            cols: [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    /// Transform a 2D point by this matrix.
    pub const fn transform_point(&self, x: f32, y: f32) -> (f32, f32) {
        let c = &self.cols;
        (
            c[0][0] * x + c[1][0] * y + c[2][0],
            c[0][1] * x + c[1][1] * y + c[2][1],
        )
    }

    /// Inverts this affine transform at one point.
    ///
    /// Returns `None` when the linear portion is singular or non-finite. Text
    /// interaction uses this to map pointer coordinates back into the local
    /// paragraph space without assuming that the canvas only translated.
    pub fn inverse_transform_point(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let a = self.cols[0][0];
        let b = self.cols[1][0];
        let c = self.cols[0][1];
        let d = self.cols[1][1];
        let determinant = a * d - b * c;
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            return None;
        }

        let translated_x = x - self.cols[2][0];
        let translated_y = y - self.cols[2][1];
        let inverse_x = (d * translated_x - b * translated_y) / determinant;
        let inverse_y = (-c * translated_x + a * translated_y) / determinant;
        (inverse_x.is_finite() && inverse_y.is_finite()).then_some((inverse_x, inverse_y))
    }

    /// Align the transform origin to physical pixel boundaries without
    /// changing its scale or rotation.
    pub fn pixel_aligned(mut self) -> Self {
        self.cols[2][0] = self.cols[2][0].round();
        self.cols[2][1] = self.cols[2][1].round();
        self
    }
}

impl Default for Mat3 {
    fn default() -> Self {
        Self::identity()
    }
}

#[cfg(test)]
mod tests {
    use super::Mat3;

    #[test]
    fn pixel_alignment_rounds_only_translation() {
        let transform = Mat3::translate(10.49, -4.51).mul(&Mat3::scale(1.5, 2.0));
        let moved_one_pixel = Mat3::translate(11.49, -3.51).mul(&Mat3::scale(1.5, 2.0));

        let aligned = transform.pixel_aligned();
        let aligned_move = moved_one_pixel.pixel_aligned();

        assert_eq!(aligned.cols[2], [10.0, -5.0, 1.0]);
        assert_eq!(aligned_move.cols[2], [11.0, -4.0, 1.0]);
        assert_eq!(aligned.cols[0], transform.cols[0]);
        assert_eq!(aligned.cols[1], transform.cols[1]);
    }

    #[test]
    fn pixel_alignment_keeps_integer_translation_stable() {
        let transform = Mat3::translate(12.0, -7.0);

        assert_eq!(transform.pixel_aligned().cols, transform.cols);
    }

    #[test]
    fn inverse_transform_point_round_trips_scaled_rotated_translation() {
        let transform = Mat3::translate(20.0, 30.0)
            .mul(&Mat3::rotate(0.37))
            .mul(&Mat3::scale(2.0, 3.0));
        let local = (4.0, -5.0);
        let world = transform.transform_point(local.0, local.1);

        let recovered = transform
            .inverse_transform_point(world.0, world.1)
            .expect("the affine transform is invertible");
        assert!((recovered.0 - local.0).abs() < 0.00001);
        assert!((recovered.1 - local.1).abs() < 0.00001);
    }

    /// The straightforward loop definition `mul` must reproduce exactly.
    fn mul_by_loops(a: &Mat3, b: &Mat3) -> Mat3 {
        let (a, b) = (&a.cols, &b.cols);
        let mut out = [[0.0f32; 3]; 3];
        for c in 0..3 {
            for r in 0..3 {
                out[c][r] = a[0][r] * b[c][0] + a[1][r] * b[c][1] + a[2][r] * b[c][2];
            }
        }
        Mat3 { cols: out }
    }

    #[test]
    fn mul_reproduces_the_loop_definition_bit_for_bit() {
        // Deterministic spread of magnitudes and signs, including values whose
        // products round, so any change in evaluation order would show.
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let unit = (state >> 40) as f32 / (1u64 << 24) as f32;
            (unit - 0.5) * 2000.0 * (1.0 + (state >> 60) as f32)
        };
        for _ in 0..2_000 {
            let mut cols = [[0.0f32; 3]; 3];
            let mut other = [[0.0f32; 3]; 3];
            for c in 0..3 {
                for r in 0..3 {
                    cols[c][r] = next();
                    other[c][r] = next();
                }
            }
            let (a, b) = (Mat3 { cols }, Mat3 { cols: other });
            let expected = mul_by_loops(&a, &b);
            let actual = a.mul(&b);
            for c in 0..3 {
                for r in 0..3 {
                    assert_eq!(
                        actual.cols[c][r].to_bits(),
                        expected.cols[c][r].to_bits(),
                        "column {c}, row {r}"
                    );
                }
            }
        }
    }

    #[test]
    fn mul_keeps_non_finite_values_like_the_loop_definition() {
        let a = Mat3 {
            cols: [[f32::INFINITY, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let b = Mat3::identity();
        let (actual, expected) = (a.mul(&b), mul_by_loops(&a, &b));
        for c in 0..3 {
            for r in 0..3 {
                assert_eq!(actual.cols[c][r].to_bits(), expected.cols[c][r].to_bits());
            }
        }
    }

    #[test]
    fn inverse_transform_point_rejects_singular_transforms() {
        assert!(
            Mat3::scale(0.0, 2.0)
                .inverse_transform_point(1.0, 1.0)
                .is_none()
        );
    }

    #[test]
    fn identity_is_recognised_and_every_other_cell_breaks_it() {
        assert!(Mat3::identity().is_identity());
        assert!(Mat3::translate(0.0, 0.0).is_identity());
        assert!(Mat3::scale(1.0, 1.0).is_identity());
        for col in 0..3 {
            for row in 0..3 {
                let mut matrix = Mat3::identity();
                matrix.cols[col][row] += 0.5;
                assert!(!matrix.is_identity(), "cell [{col}][{row}] was ignored");
            }
        }
        assert!(!Mat3::translate(1.0, 0.0).is_identity());
        assert!(!Mat3::scale(2.0, 1.0).is_identity());
    }

    #[test]
    fn identity_treats_negative_zero_as_zero_and_nan_as_different() {
        let mut matrix = Mat3::identity();
        matrix.cols[0][1] = -0.0;
        matrix.cols[2][0] = -0.0;
        assert!(matrix.is_identity());

        matrix.cols[1][1] = f32::NAN;
        assert!(!matrix.is_identity());
    }
}
