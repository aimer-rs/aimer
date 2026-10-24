use aimer_attribute::size::{ResolvedSize, Size};
use aimer_macro::Rebuildable;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventTreeRole, LayoutElement,
    RequiredChild, VisitorElement, Widget,
};

/// A flex child that fills the remaining main-axis space inside a flex
/// container (`Row`, `Column`, `Flex`), mirroring Flutter's `Expanded` widget.
///
/// The `flex` factor controls how the free space of the flex container is
/// shared between the expanding children:
///
/// - In a `Row` with a single `Expanded`, the child fills the whole width.
/// - In a `Row` with two `Expanded` children (both `flex = 1`), each child gets
///   half of the width.
/// - In a `Row` with two `Expanded` children of `flex = 1` and `flex = 2`, the
///   first child gets `1 / (1 + 2)` and the second `2 / (1 + 2)` of the free
///   space.
///
/// Attach a child with [`Expanded::child`] to retain its concrete type, or with
/// [`Expanded::box_child`] when branches need to return the same erased type.
///
/// # Example
///
/// ```rust
/// use aimer_container::SizedBox;
/// use aimer_flex::{Expanded, Row};
///
/// let row = Row::new().children(vec![Expanded::new().child(SizedBox::new()),
///                                    Expanded::new().flex(2.0).child(SizedBox::new()),]);
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_flex::flex::Expanded")]
pub struct Expanded<W = RequiredChild> {
    /// The flex factor: the child's share of the free main-axis space is
    /// `flex / sum_of_all_flex_factors`. Defaults to `1.0`.
    flex: f32,
    /// The widget that expands to fill the assigned space.
    #[portable_child]
    child: W,
}

impl Default for Expanded {
    fn default() -> Self {
        Self::new()
    }
}

impl Expanded {
    /// Creates an expanding child with a flex factor of `1.0`.
    ///
    /// Finish the builder with [`Expanded::child`] or [`Expanded::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            flex: 1.0,
            child: RequiredChild,
        }
    }

    /// Sets this child's weight when a flex parent distributes remaining space.
    ///
    /// The default is `1.0`. At element construction, negative values are
    /// clamped to `0.0`; a zero-weight child receives no share of the remaining
    /// main-axis space.
    #[inline]
    pub fn flex(mut self, flex: f32) -> Self {
        self.flex = flex;
        self
    }
    /// Attaches the required child and makes the builder a valid [`Widget`].
    ///
    /// This terminal operation preserves the child's concrete type. Use
    /// [`Expanded::box_child`] instead when branch type erasure is needed.
    #[inline]
    pub fn child<W: Widget + 'static>(self, child: W) -> Expanded<W> {
        Expanded {
            child,
            flex: self.flex,
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    ///
    /// This is equivalent to calling [`Expanded::child`] followed by
    /// [`Widget::boxed`]. Use it when different branches must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget + 'static> Widget for Expanded<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawExpanded {
            child: self.child.to_element(ctx),
            flex: self.flex.max(0.0),
            debug_name: "Expanded",
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Expanded"
    }
}

/// Lower level element backing [`Expanded`].
///
/// It carries a `flex` factor that its flex parent (`RawFlex`) reads through
/// [`LayoutElement::flex`] to distribute the remaining main-axis space. On
/// layout it delegates intrinsic measurement to its child; the flex parent
/// applies the allocated extent on its own main axis.
#[derive(Rebuildable)]
pub struct RawExpanded<E: Element> {
    pub(crate) child: E,
    pub(crate) flex: f32,
    pub(crate) debug_name: &'static str,
}

impl<E: Element> EventElement for RawExpanded<E> {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl<E: Element> RawExpanded<E> {
    /// Creates the low-level element used by flex layout tests and adapters.
    #[doc(hidden)]
    #[inline]
    pub fn new(child: E, flex: f32, debug_name: &'static str) -> Self {
        Self {
            child,
            flex: flex.max(0.0),
            debug_name,
        }
    }
}

impl<E: Element> Drawable for RawExpanded<E> {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    #[inline]
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }
}

impl<E: Element> VisitorElement for RawExpanded<E> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }

    fn debug_name(&self) -> &'static str {
        self.debug_name
    }
}

impl<E: Element> LayoutElement for RawExpanded<E> {
    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn flex(&self) -> Option<f32> {
        Some(self.flex)
    }

    /// An `Expanded` has no intrinsic main-axis size of its own — it is sized
    /// by its flex parent — so it must not report a fixed size to
    /// ancestors.
    fn get_size_from_child(&self) -> Option<Size> {
        None
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
}
/// Distribute `remaining` main-axis space across children according to their
/// flex `weights`.
///
/// `weights[i]` is the flex factor of child `i`, or `0.0` for a non-flex
/// (regular) child. The returned vector has the same length: each flex child
/// receives `remaining * weight / total_weight`, and every non-flex child
/// receives `0.0`. When no child is flexible (all weights `<= 0`) the result is
/// all zeros.
#[cfg(test)]
pub(crate) fn distribute_flex_space(remaining: f32, weights: &[f32]) -> Vec<f32> {
    let mut shares = weights.to_vec();
    distribute_flex_space_in_place(remaining, &mut shares);
    for share in &mut shares {
        if share.is_sign_negative() {
            *share = 0.0;
        }
    }
    shares
}

/// Replaces positive entries in `weights` with their allocated shares.
///
/// Negative entries are retained as non-flex sentinels. A zero entry is
/// encoded as negative zero so callers that need to distinguish a zero-weight
/// flex child from a regular child can do so without another allocation.
#[inline]
pub(crate) fn distribute_flex_space_in_place(remaining: f32, weights: &mut [f32]) {
    debug_assert!(remaining >= 0.0);
    let total = positive_weight_sum_scalar(weights);
    if total <= 0.0 {
        mark_non_flex_weights(weights);
        return;
    }
    distribute_positive_weight_shares(remaining, total, weights);
}

#[cfg(test)]
#[inline]
pub(crate) fn distribute_flex_space_in_place_scalar_reference(remaining: f32, weights: &mut [f32]) {
    debug_assert!(remaining >= 0.0);
    let total = positive_weight_sum_scalar(weights);
    if total <= 0.0 {
        mark_non_flex_weights(weights);
        return;
    }
    distribute_positive_weight_shares_scalar(remaining, total, weights);
}

#[inline]
fn positive_weight_sum_scalar(weights: &[f32]) -> f32 {
    let mut total = 0.0;
    for &weight in weights {
        if weight > 0.0 {
            total += weight;
        }
    }
    total
}

#[inline]
fn mark_non_flex_weights(weights: &mut [f32]) {
    for weight in weights {
        if *weight == 0.0 || weight.is_nan() {
            *weight = -0.0;
        }
    }
}

#[cfg(all(
    test,
    not(feature = "force-scalar"),
    not(debug_assertions),
    target_arch = "aarch64"
))]
const SELECTED_DISTRIBUTION_KERNEL: &str = "aarch64-neon";

#[cfg(all(
    test,
    not(feature = "force-scalar"),
    not(debug_assertions),
    target_arch = "x86_64"
))]
const SELECTED_DISTRIBUTION_KERNEL: &str = "x86_64-sse2";

#[cfg(all(
    test,
    any(
        feature = "force-scalar",
        debug_assertions,
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    )
))]
const SELECTED_DISTRIBUTION_KERNEL: &str = "scalar";

#[cfg(all(
    not(feature = "force-scalar"),
    not(debug_assertions),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
#[inline]
fn distribute_positive_weight_shares(remaining: f32, total: f32, weights: &mut [f32]) {
    distribute_positive_weight_shares_simd(remaining, total, weights);
}

#[cfg(any(
    feature = "force-scalar",
    debug_assertions,
    not(any(target_arch = "aarch64", target_arch = "x86_64"))
))]
#[inline]
fn distribute_positive_weight_shares(remaining: f32, total: f32, weights: &mut [f32]) {
    distribute_positive_weight_shares_scalar(remaining, total, weights);
}

#[inline]
fn distribute_positive_weight_shares_scalar(remaining: f32, total: f32, weights: &mut [f32]) {
    for weight in weights {
        if *weight > 0.0 {
            *weight = remaining * (*weight / total);
        } else if *weight == 0.0 || weight.is_nan() {
            *weight = -0.0;
        }
    }
}

#[cfg(all(
    not(feature = "force-scalar"),
    not(debug_assertions),
    target_arch = "aarch64"
))]
#[inline]
fn distribute_positive_weight_shares_simd(remaining: f32, total: f32, weights: &mut [f32]) {
    use std::arch::aarch64::{
        float32x4_t, vbslq_f32, vceqq_f32, vcgtq_f32, vdivq_f32, vdupq_n_f32, vld1q_f32, vmulq_f32,
        vmvnq_u32, vorrq_u32, vst1q_f32,
    };

    let remaining_value = remaining;
    let total_value = total;
    let mut chunks = weights.chunks_exact_mut(4);
    let remaining = unsafe { vdupq_n_f32(remaining_value) };
    let total = unsafe { vdupq_n_f32(total_value) };
    let zero = unsafe { vdupq_n_f32(0.0) };
    let negative_zero = unsafe { vdupq_n_f32(-0.0) };

    for chunk in &mut chunks {
        // SAFETY: chunks_exact_mut guarantees four initialized f32 values and
        // vld1q/vst1q accept unaligned pointers. The masks only select values
        // from this same four-lane chunk, so no bounds or aliasing invariant is
        // added beyond the mutable slice borrow.
        unsafe {
            let values: float32x4_t = vld1q_f32(chunk.as_ptr());
            let positive = vcgtq_f32(values, zero);
            let zero_or_nan = vorrq_u32(
                vceqq_f32(values, zero),
                vmvnq_u32(vceqq_f32(values, values)),
            );
            let shares = vmulq_f32(remaining, vdivq_f32(values, total));
            let non_positive = vbslq_f32(zero_or_nan, negative_zero, values);
            let result = vbslq_f32(positive, shares, non_positive);
            vst1q_f32(chunk.as_mut_ptr(), result);
        }
    }

    distribute_positive_weight_shares_scalar(remaining_value, total_value, chunks.into_remainder());
}

#[cfg(all(
    not(feature = "force-scalar"),
    not(debug_assertions),
    target_arch = "x86_64"
))]
#[inline]
fn distribute_positive_weight_shares_simd(remaining: f32, total: f32, weights: &mut [f32]) {
    use std::arch::x86_64::{
        _mm_and_ps, _mm_andnot_ps, _mm_cmpeq_ps, _mm_cmpgt_ps, _mm_cmpunord_ps, _mm_div_ps,
        _mm_loadu_ps, _mm_mul_ps, _mm_or_ps, _mm_set1_ps, _mm_setzero_ps, _mm_storeu_ps,
    };

    let remaining_value = remaining;
    let total_value = total;
    let mut chunks = weights.chunks_exact_mut(4);
    let remaining = unsafe { _mm_set1_ps(remaining_value) };
    let total = unsafe { _mm_set1_ps(total_value) };
    let zero = unsafe { _mm_setzero_ps() };
    let negative_zero = unsafe { _mm_set1_ps(-0.0) };

    for chunk in &mut chunks {
        // SAFETY: chunks_exact_mut guarantees four initialized f32 values and
        // loadu/storeu accept unaligned pointers. The masks only select values
        // from this same four-lane chunk, so no bounds or aliasing invariant is
        // added beyond the mutable slice borrow.
        unsafe {
            let values = _mm_loadu_ps(chunk.as_ptr());
            let positive = _mm_cmpgt_ps(values, zero);
            let zero_or_nan =
                _mm_or_ps(_mm_cmpeq_ps(values, zero), _mm_cmpunord_ps(values, values));
            let shares = _mm_mul_ps(remaining, _mm_div_ps(values, total));
            let non_positive = _mm_or_ps(
                _mm_and_ps(zero_or_nan, negative_zero),
                _mm_andnot_ps(zero_or_nan, values),
            );
            let result = _mm_or_ps(
                _mm_and_ps(positive, shares),
                _mm_andnot_ps(positive, non_positive),
            );
            _mm_storeu_ps(chunk.as_mut_ptr(), result);
        }
    }

    distribute_positive_weight_shares_scalar(remaining_value, total_value, chunks.into_remainder());
}

#[cfg(test)]
mod in_place_tests {
    use super::{distribute_flex_space_in_place, distribute_flex_space_in_place_scalar_reference};

    #[test]
    fn in_place_distribution_preserves_non_flex_and_zero_weight_markers() {
        let mut weights = [1.0, -1.0, 0.0, 2.0];

        distribute_flex_space_in_place(300.0, &mut weights);

        assert_eq!(weights[0], 100.0);
        assert_eq!(weights[1], -1.0);
        assert_eq!(weights[2], 0.0);
        assert!(weights[2].is_sign_negative());
        assert_eq!(weights[3], 200.0);
    }

    #[test]
    fn optimized_distribution_matches_scalar_reference() {
        let cases = [
            (128.0, vec![]),
            (128.0, vec![1.0]),
            (128.0, vec![1.0, -1.0, 0.0]),
            (4096.0, vec![1.0, 2.0, 3.0, -1.0, 0.0, -0.0, f32::NAN]),
            (300.0, vec![0.0, -1.0, -0.0, f32::NAN]),
            (0.0, vec![1.0, 2.0, -1.0, 0.0]),
            (1.0, vec![0.25, 0.5, 0.75, 1.0, -2.0, 0.0]),
            (f32::MAX, vec![f32::MAX, 1.0, -1.0]),
            (1.0, vec![f32::MIN_POSITIVE, f32::MAX, f32::MAX, 0.0, -1.0]),
        ];

        for (remaining, input) in cases {
            let mut expected = input.clone();
            let mut actual = input;
            distribute_flex_space_in_place_scalar_reference(remaining, &mut expected);
            distribute_flex_space_in_place(remaining, &mut actual);

            for (expected, actual) in expected.iter().zip(actual) {
                assert_eq!(expected.is_sign_negative(), actual.is_sign_negative());
                if expected.is_nan()
                    || actual.is_nan()
                    || !expected.is_finite()
                    || !actual.is_finite()
                {
                    assert_eq!(expected.to_bits(), actual.to_bits());
                } else {
                    let expected_ordered = if expected.is_sign_negative() {
                        !expected.to_bits()
                    } else {
                        expected.to_bits() | (1 << 31)
                    };
                    let actual_ordered = if actual.is_sign_negative() {
                        !actual.to_bits()
                    } else {
                        actual.to_bits() | (1 << 31)
                    };
                    let ulps = expected_ordered.abs_diff(actual_ordered);
                    assert!(
                        ulps <= 4,
                        "scalar {expected:?} differs from optimized {actual:?} by {ulps} ULPs"
                    );
                }
            }
        }
    }

    #[test]
    fn selected_dispatch_matches_build_configuration() {
        let expected = if cfg!(all(
            not(feature = "force-scalar"),
            not(debug_assertions),
            target_arch = "aarch64"
        )) {
            "aarch64-neon"
        } else if cfg!(all(
            not(feature = "force-scalar"),
            not(debug_assertions),
            target_arch = "x86_64"
        )) {
            "x86_64-sse2"
        } else {
            "scalar"
        };

        assert_eq!(super::SELECTED_DISTRIBUTION_KERNEL, expected);
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::Instant;

    use super::*;

    #[test]
    fn single_flex_child_fills_all_remaining_space() {
        // A single Expanded in a Row: it fills the whole main axis.
        let shares = distribute_flex_space(300.0, &[1.0]);
        assert_eq!(shares, vec![300.0]);
    }

    #[test]
    fn two_equal_flex_children_split_evenly() {
        // Two Expanded children, both flex = 1 => each gets parent / 2.
        let shares = distribute_flex_space(300.0, &[1.0, 1.0]);
        assert_eq!(shares, vec![150.0, 150.0]);
    }

    #[test]
    fn weighted_flex_children_split_proportionally() {
        // flex = 1 and flex = 2 => 1/3 and 2/3 of the free space.
        let shares = distribute_flex_space(300.0, &[1.0, 2.0]);
        assert_eq!(shares, vec![100.0, 200.0]);
    }

    #[test]
    fn non_flex_children_receive_no_space() {
        // A sized (non-flex) child in the middle gets nothing; the flex
        // children share everything.
        let shares = distribute_flex_space(300.0, &[1.0, 0.0, 2.0]);
        assert_eq!(shares, vec![100.0, 0.0, 200.0]);
    }

    #[test]
    fn no_flex_children_yields_zeros() {
        let shares = distribute_flex_space(300.0, &[0.0, 0.0]);
        assert_eq!(shares, vec![0.0, 0.0]);
    }

    #[test]
    #[ignore = "manual numeric-kernel profile"]
    fn profile_distribute_flex_space() {
        const MEASURED: usize = 256;
        const WARMUP: usize = 32;
        const ROUNDS: usize = 31;

        let cases = [
            ("zero-weights-32", vec![0.0; 32]),
            (
                "sparse-weights-256",
                (0..256)
                    .map(|index| {
                        if index % 8 == 0 {
                            (index % 5 + 1) as f32
                        } else {
                            -1.0
                        }
                    })
                    .collect(),
            ),
            ("uniform-weights-2048", vec![1.0; 2048]),
            (
                "weighted-weights-2048",
                (0..2048)
                    .map(|index| {
                        if index % 3 == 0 {
                            (index % 17 + 1) as f32
                        } else if index % 5 == 0 {
                            -2.0
                        } else {
                            0.0
                        }
                    })
                    .collect(),
            ),
        ];

        for (name, weights) in cases {
            let mut samples = Vec::with_capacity(ROUNDS);
            let mut checksum = 0.0;
            for _ in 0..ROUNDS {
                for _ in 0..WARMUP {
                    let shares = black_box(distribute_flex_space(
                        black_box(4096.0),
                        black_box(&weights),
                    ));
                    checksum = black_box(checksum + shares[0]);
                }

                let start = Instant::now();
                for _ in 0..MEASURED {
                    let shares = black_box(distribute_flex_space(
                        black_box(4096.0),
                        black_box(&weights),
                    ));
                    checksum = black_box(checksum + shares[weights.len() / 2]);
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED as f64);
            }

            samples.sort_by(f64::total_cmp);
            let p50 = samples[ROUNDS / 2];
            let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
            println!("{name}: p50 {p50:.3} us, p95 {p95:.3} us");
            assert!(checksum.is_finite());
        }
    }

    fn measure_distribution_variant<F>(template: &[f32], mut kernel: F) -> (f64, f64)
    where
        F: FnMut(f32, &mut [f32]),
    {
        const MEASURED: usize = 256;
        const WARMUP: usize = 32;
        const ROUNDS: usize = 31;

        let mut samples = Vec::with_capacity(ROUNDS);
        let mut checksum = 0.0;
        for _ in 0..ROUNDS {
            let mut inputs: Vec<Vec<f32>> =
                (0..WARMUP + MEASURED).map(|_| template.to_vec()).collect();
            let mut inputs = inputs.iter_mut();

            for _ in 0..WARMUP {
                let weights = black_box(inputs.next().expect("warmup input"));
                kernel(4096.0, weights);
                black_box(&*weights);
                checksum = black_box(checksum + weights[0]);
            }

            let start = Instant::now();
            for _ in 0..MEASURED {
                let weights = black_box(inputs.next().expect("measured input"));
                kernel(4096.0, weights);
                black_box(&*weights);
                checksum = black_box(checksum + weights[weights.len() / 2]);
            }
            samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED as f64);
        }

        assert!(checksum.is_finite());
        samples.sort_by(f64::total_cmp);
        (
            samples[ROUNDS / 2],
            samples[(ROUNDS * 95).div_ceil(100) - 1],
        )
    }

    #[test]
    #[ignore = "manual numeric-kernel profile"]
    fn profile_distribute_flex_space_variants() {
        let cases = [
            ("zero-weights-32", vec![0.0; 32]),
            (
                "sparse-weights-256",
                (0..256)
                    .map(|index| {
                        if index % 8 == 0 {
                            (index % 5 + 1) as f32
                        } else {
                            -1.0
                        }
                    })
                    .collect(),
            ),
            ("uniform-weights-2048", vec![1.0; 2048]),
            (
                "weighted-weights-2048",
                (0..2048)
                    .map(|index| {
                        if index % 3 == 0 {
                            (index % 17 + 1) as f32
                        } else if index % 5 == 0 {
                            -2.0
                        } else {
                            0.0
                        }
                    })
                    .collect(),
            ),
        ];

        for (case_index, (name, template)) in cases.into_iter().enumerate() {
            let (scalar_p50, scalar_p95, optimized_p50, optimized_p95) = if case_index % 2 == 0 {
                let (scalar_p50, scalar_p95) = measure_distribution_variant(
                    &template,
                    distribute_flex_space_in_place_scalar_reference,
                );
                let (optimized_p50, optimized_p95) =
                    measure_distribution_variant(&template, distribute_flex_space_in_place);
                (scalar_p50, scalar_p95, optimized_p50, optimized_p95)
            } else {
                let (optimized_p50, optimized_p95) =
                    measure_distribution_variant(&template, distribute_flex_space_in_place);
                let (scalar_p50, scalar_p95) = measure_distribution_variant(
                    &template,
                    distribute_flex_space_in_place_scalar_reference,
                );
                (scalar_p50, scalar_p95, optimized_p50, optimized_p95)
            };
            println!(
                "{name}: scalar {scalar_p50:.3}/{scalar_p95:.3} us, optimized {optimized_p50:.3}/{optimized_p95:.3} us"
            );
        }
    }
}
