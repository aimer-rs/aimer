//! A linear alpha ramp applied to glyphs as they are drawn.
//!
//! A fade (an opacity gradient over a child) is normally drawn by rendering the
//! child and the ramp into two offscreen targets and compositing them, which
//! costs several render passes. When the child is only text, each glyph can take
//! the ramp's alpha where it is drawn instead, and nothing is drawn offscreen:
//! [`AlphaRamp`] is that ramp, in the pixels of the target the glyphs go to.
//!
//! It is the same function the material gradient shader evaluates for the alpha
//! channel, so a text fade drawn either way agrees to within rounding.

/// Most stops a ramp carries. A ramp with more is left to the offscreen path.
pub const MAX_RAMP_STOPS: usize = 4;

/// A linear alpha ramp over a rectangle, in target pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlphaRamp {
    /// The ramp's axis, `[x0, y0, x1, y1]`: alpha is the first stop's at `x0, y0`
    /// and the last's at `x1, y1`.
    pub line: [f32; 4],
    /// Where the ramp is defined, `[x, y, width, height]`. Nothing outside it is
    /// drawn, as nothing outside the quad a ramp is drawn on is.
    pub rect: [f32; 4],
    /// Stop positions along the axis, non-decreasing. Unused stops repeat the last.
    pub offsets: [f32; MAX_RAMP_STOPS],
    /// The alpha at each stop. Unused stops repeat the last.
    pub alphas: [f32; MAX_RAMP_STOPS],
}

impl AlphaRamp {
    /// A ramp over `rect` along `line` through `stops`, each `(offset, alpha)`.
    ///
    /// Returns `None` for what a ramp cannot say: no stops, more than
    /// [`MAX_RAMP_STOPS`], offsets that go backwards, or anything not finite.
    pub fn new(line: [f32; 4], rect: [f32; 4], stops: &[(f32, f32)]) -> Option<Self> {
        if stops.is_empty() || stops.len() > MAX_RAMP_STOPS {
            return None;
        }
        let finite = line.iter().chain(&rect).all(|value| value.is_finite())
            && stops
                .iter()
                .all(|(offset, alpha)| offset.is_finite() && alpha.is_finite());
        let ordered = stops.windows(2).all(|pair| pair[0].0 <= pair[1].0);
        if !finite || !ordered {
            return None;
        }
        let last = *stops.last().expect("a ramp has a stop");
        let mut offsets = [last.0; MAX_RAMP_STOPS];
        let mut alphas = [last.1.clamp(0.0, 1.0); MAX_RAMP_STOPS];
        for (index, (offset, alpha)) in stops.iter().enumerate() {
            offsets[index] = *offset;
            alphas[index] = alpha.clamp(0.0, 1.0);
        }
        Some(Self {
            line,
            rect,
            offsets,
            alphas,
        })
    }

    /// The same ramp with every alpha scaled by `opacity`, for a group that is
    /// itself drawn at less than full opacity.
    pub fn scaled(mut self, opacity: f32) -> Self {
        let opacity = opacity.clamp(0.0, 1.0);
        for alpha in &mut self.alphas {
            *alpha *= opacity;
        }
        self
    }

    /// The alpha the ramp gives the pixel at `(x, y)`.
    ///
    /// This is the reference the shader is written to match: before the first
    /// stop and after the last the alpha is held, outside [`Self::rect`] it is
    /// zero, and inside it the outermost pixels are softened as the edge of the
    /// quad a ramp is drawn on is (see [`Self::edge_coverage`]).
    pub fn alpha_at(&self, x: f32, y: f32) -> f32 {
        let [rx, ry, width, height] = self.rect;
        if x < rx || y < ry || x >= rx + width || y >= ry + height {
            return 0.0;
        }
        let along = self.along(x, y);
        self.edge_coverage(x, y) * along
    }

    /// How much of a pixel inside [`Self::rect`] the rectangle covers: 1 well
    /// inside, and less on the pixels at the edge, which a quad drawn through the
    /// material pipeline antialiases one pixel wide. A fade drawn in a group is
    /// seen through that quad, so a ramp matches it only if it softens the edge
    /// the same way.
    pub fn edge_coverage(&self, x: f32, y: f32) -> f32 {
        let [rx, ry, width, height] = self.rect;
        let (half_width, half_height) = (width * 0.5, height * 0.5);
        let qx = (x - (rx + half_width)).abs() - half_width;
        let qy = (y - (ry + half_height)).abs() - half_height;
        // The distance to the rectangle, negative inside it.
        let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0);
        // `fwidth` of that distance is 1 along a straight edge.
        1.0 - smoothstep(-1.0, 1.0, distance)
    }

    /// The ramp's alpha at `(x, y)` along its axis, without the rectangle's edge.
    fn along(&self, x: f32, y: f32) -> f32 {
        let axis = [self.line[2] - self.line[0], self.line[3] - self.line[1]];
        let along = (x - self.line[0]) * axis[0] + (y - self.line[1]) * axis[1];
        let t = (along / (axis[0] * axis[0] + axis[1] * axis[1]).max(0.000_001)).clamp(0.0, 1.0);

        if t <= self.offsets[0] {
            return self.alphas[0];
        }
        for index in 1..MAX_RAMP_STOPS {
            if t <= self.offsets[index] {
                let span = (self.offsets[index] - self.offsets[index - 1]).max(0.000_001);
                let f = ((t - self.offsets[index - 1]) / span).clamp(0.0, 1.0);
                return self.alphas[index - 1] + (self.alphas[index] - self.alphas[index - 1]) * f;
            }
        }
        self.alphas[MAX_RAMP_STOPS - 1]
    }
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ramp of a title that fades out over the last quarter of 200 pixels, as
    /// `faded_line` draws it.
    fn title_fade() -> AlphaRamp {
        AlphaRamp::new(
            [0.0, 0.0, 200.0, 0.0],
            [0.0, 0.0, 200.0, 20.0],
            &[(0.0, 1.0), (0.74, 1.0), (1.0, 0.0)],
        )
        .expect("three stops make a ramp")
    }

    fn near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1e-4, "{actual} vs {expected}");
    }

    #[test]
    fn the_title_is_opaque_until_the_fade_begins() {
        let ramp = title_fade();
        near(ramp.alpha_at(3.0, 10.0), 1.0);
        near(ramp.alpha_at(100.0, 10.0), 1.0);
        near(ramp.alpha_at(147.0, 10.0), 1.0);
    }

    #[test]
    fn the_title_fades_linearly_to_nothing_at_the_edge() {
        let ramp = title_fade();
        // Halfway through the fade, 0.74 + 0.13 = 0.87 of 200 pixels.
        near(ramp.alpha_at(174.0, 10.0), 0.5);
        assert!(ramp.alpha_at(199.99, 10.0) < 0.001, "all but gone at the edge");
    }

    #[test]
    fn nothing_is_drawn_outside_the_rect() {
        let ramp = title_fade();
        near(ramp.alpha_at(-0.1, 10.0), 0.0);
        near(ramp.alpha_at(100.0, -0.1), 0.0);
        near(ramp.alpha_at(100.0, 20.0), 0.0);
        near(ramp.alpha_at(200.0, 10.0), 0.0);
    }

    #[test]
    fn the_outermost_pixels_are_softened_as_the_edge_of_a_quad_is() {
        let ramp = title_fade();
        // A pixel center half a pixel inside the edge: 1 - smoothstep(-1, 1, -0.5).
        near(ramp.alpha_at(50.0, 0.5), 0.84375);
        near(ramp.alpha_at(0.5, 10.0), 0.84375);
        // At a corner the nearer edge decides, not both.
        near(ramp.alpha_at(0.5, 0.5), 0.84375);
        // A pixel further in is untouched.
        near(ramp.alpha_at(50.0, 1.5), 1.0);
    }

    #[test]
    fn the_alpha_is_held_before_the_first_stop_and_after_the_last() {
        let ramp = AlphaRamp::new(
            [50.0, 0.0, 150.0, 0.0],
            [0.0, 0.0, 200.0, 20.0],
            &[(0.0, 0.25), (1.0, 0.75)],
        )
        .unwrap();
        near(ramp.alpha_at(10.0, 5.0), 0.25);
        near(ramp.alpha_at(100.0, 5.0), 0.5);
        near(ramp.alpha_at(190.0, 5.0), 0.75);
    }

    #[test]
    fn a_ramp_runs_along_its_axis_whatever_its_direction() {
        // Down the rect, from opaque at the top to clear at the bottom.
        let ramp = AlphaRamp::new(
            [0.0, 0.0, 0.0, 100.0],
            [0.0, 0.0, 40.0, 100.0],
            &[(0.0, 1.0), (1.0, 0.0)],
        )
        .unwrap();
        near(ramp.alpha_at(20.0, 25.0), 0.75);
        near(ramp.alpha_at(3.0, 25.0), 0.75);
    }

    #[test]
    fn the_group_opacity_scales_every_stop() {
        near(title_fade().scaled(0.5).alpha_at(100.0, 10.0), 0.5);
        near(title_fade().scaled(0.5).alpha_at(174.0, 10.0), 0.25);
    }

    #[test]
    fn a_ramp_with_unused_stops_repeats_the_last() {
        let ramp = title_fade();
        assert_eq!(ramp.offsets[3], 1.0);
        assert_eq!(ramp.alphas[3], 0.0);
    }

    #[test]
    fn what_a_ramp_cannot_say_is_refused() {
        let line = [0.0, 0.0, 1.0, 0.0];
        let rect = [0.0, 0.0, 1.0, 1.0];
        assert!(AlphaRamp::new(line, rect, &[]).is_none(), "no stops");
        assert!(
            AlphaRamp::new(line, rect, &[(0.0, 1.0); MAX_RAMP_STOPS + 1]).is_none(),
            "too many stops"
        );
        assert!(
            AlphaRamp::new(line, rect, &[(0.5, 1.0), (0.2, 0.0)]).is_none(),
            "offsets going backwards"
        );
        assert!(
            AlphaRamp::new(line, rect, &[(0.0, f32::NAN)]).is_none(),
            "a stop that is not a number"
        );
    }

    #[test]
    fn a_ramp_with_no_length_holds_its_first_alpha() {
        let ramp = AlphaRamp::new(
            [5.0, 5.0, 5.0, 5.0],
            [0.0, 0.0, 10.0, 10.0],
            &[(0.0, 0.5), (1.0, 0.0)],
        )
        .unwrap();
        near(ramp.alpha_at(5.0, 5.0), 0.5);
    }
}
