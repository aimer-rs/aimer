//! The geometry of a page that opens out of the item it was opened from by that
//! item's window growing to fill the host, while the item dissolves into the
//! page inside it. Like [`ContainerTransformSpec`](super::ContainerTransformSpec)
//! this module is only the geometry; hosts such as the router apply it to
//! retained pages.

use std::ops::Range;

use aimer_attribute::size::ResolvedSize;
use aimer_canvas::Mat3;

use super::animated_paint::PaintPresentation;
use super::container_transform::{
    ContainerTransformFrame, ElementBounds, MAX_BLUR_HOLD, StageSpans, blur_swell, sanitize, valid_or,
};
use crate::primitives::curve::{AnyCurve, Curve};

/// Describes an item's window growing to fill the host while the item turns
/// into the page.
///
/// Two things happen at once, each on a span of the timeline of its own:
///
/// * **The window grows** ([`grow`](Self::grow)). It starts as the item's bounds,
///   with the item's corners, and ends as the whole host, with the page's. The
///   page is revealed through it rather than scaled, so an item as wide as the
///   host, such as a row of a feed, has something to grow into. With
///   [`stretch`](Self::stretch) the page is instead scaled to fill the window, as
///   the copy of the item is, which squeezes the whole page into the item's shape.
/// * **The item dissolves into the page** ([`dissolve`](Self::dissolve)). A copy
///   of the item is stretched to the window, so that it is the item that grows
///   and its box is always the window's, and fades out over the page, which is
///   opaque beneath it from the first frame so that what is behind never shows
///   through the window; or, with a [`crossfade`](Self::crossfade), the page
///   rises as the copy falls. The stretch distorts the copy's contents, so keep
///   the dissolve short of the window's last stretch, or blur through it.
///
/// The spans are free, so the item can start dissolving while the window is
/// still opening and be gone before it has finished: nothing waits for anything.
/// The page behind darkens in step with the window covering it, and a host can
/// blur everything through the dissolve; see [`blur`](Self::blur).
///
/// # Examples
///
/// ```
/// use aimer_animation::{ElementBounds, ExpandSpec};
/// use aimer_attribute::size::ResolvedSize;
///
/// let row = ElementBounds::new(0.0, 260.0, 400.0, 160.0);
/// let spec = ExpandSpec::new(row, ResolvedSize { width: 400.0, height: 640.0 })
///     .unwrap()
///     .anchored(true);
///
/// // On the row at first, filling the host at the end, and dissolving between.
/// // The copy of the row is stretched to the window, so it grows with it.
/// assert_eq!(spec.sample(0.0).page.reveal, Some(ElementBounds::new(0.0, 0.0, 400.0, 160.0)));
/// assert_eq!(spec.sample(1.0).page.reveal, None);
/// assert!((spec.sample(0.45).anchor.opacity - 0.5).abs() < 1e-5);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct ExpandSpec {
    source: ElementBounds,
    viewport: ResolvedSize,
    source_radius: f32,
    page_radius: f32,
    scrim: f32,
    anchored: bool,
    crossfade: bool,
    stretch: bool,
    grow: (f32, f32),
    dissolve: (f32, f32),
    blur_hold: f32,
    grow_curve: AnyCurve,
    dissolve_curve: AnyCurve,
}

/// The whole timeline: the window grows from the start to the end.
const DEFAULT_GROW: (f32, f32) = (0.0, 1.0);

/// The middle of the timeline: the window is already opening when the item
/// starts to go, and is not full when the item is gone.
const DEFAULT_DISSOLVE: (f32, f32) = (0.2, 0.7);

impl ExpandSpec {
    /// Describes `source` growing to fill `viewport`.
    ///
    /// Both are in the same logical-pixel space, with `source` measured from the
    /// viewport's top-left corner. Returns `None` if either has no positive,
    /// finite area.
    pub fn new(source: ElementBounds, viewport: ResolvedSize) -> Option<Self> {
        let viewport_ok = viewport.width.is_finite()
            && viewport.height.is_finite()
            && viewport.width > 0.0
            && viewport.height > 0.0;
        (source.is_valid() && viewport_ok).then_some(Self {
            source,
            viewport,
            source_radius: 12.0,
            page_radius: 0.0,
            scrim: 0.35,
            anchored: false,
            crossfade: false,
            stretch: false,
            grow: DEFAULT_GROW,
            dissolve: DEFAULT_DISSOLVE,
            blur_hold: 0.0,
            grow_curve: AnyCurve::default(),
            dissolve_curve: AnyCurve::default(),
        })
    }

    /// The same motion, with every setting kept, played over other geometry:
    /// `source` growing to fill `viewport`. Returns `None` if either has no
    /// positive, finite area.
    ///
    /// A host that follows an item that scrolls, or a host that is resized, asks
    /// for a spec every frame; this saves it from describing the motion again
    /// each time.
    pub fn retargeted(&self, source: ElementBounds, viewport: ResolvedSize) -> Option<Self> {
        let mut spec = Self::new(source, viewport)?;
        spec.source_radius = self.source_radius;
        spec.page_radius = self.page_radius;
        spec.scrim = self.scrim;
        spec.anchored = self.anchored;
        spec.crossfade = self.crossfade;
        spec.stretch = self.stretch;
        spec.grow = self.grow;
        spec.dissolve = self.dissolve;
        spec.blur_hold = self.blur_hold;
        spec.grow_curve = self.grow_curve.clone();
        spec.dissolve_curve = self.dissolve_curve.clone();
        Some(spec)
    }

    /// Sets the corner radius of the window while it sits on the item. The
    /// default is 12 logical pixels. Negative or non-finite values count as zero.
    pub fn source_radius(mut self, radius: f32) -> Self {
        self.source_radius = sanitize(radius);
        self
    }

    /// Sets the corner radius of the window once it fills the host. The default
    /// is zero. Negative or non-finite values count as zero.
    pub fn page_radius(mut self, radius: f32) -> Self {
        self.page_radius = sanitize(radius);
        self
    }

    /// Sets how dark the page behind is when the window has filled the host, as
    /// a fraction in `0.0..=1.0`. The default is `0.35`. A value that is not
    /// finite is ignored.
    ///
    /// It darkens along the window's growth, so the page behind is shaded as fast
    /// as the window covers it.
    pub fn scrim(mut self, scrim: f32) -> Self {
        if scrim.is_finite() {
            self.scrim = scrim.clamp(0.0, 1.0);
        }
        self
    }

    /// Whether a copy of the item is drawn above the page and dissolved away.
    ///
    /// With a copy the page is opaque beneath it from the start. Without one
    /// there is nothing to dissolve from, so the page fades in over the
    /// [dissolve](Self::dissolve) instead. Off by default.
    pub fn anchored(mut self, anchored: bool) -> Self {
        self.anchored = anchored;
        self
    }

    /// Has the page fade in as the item fades out, instead of lying opaque beneath
    /// it. Off by default.
    ///
    /// The page rises on the [dissolve](Self::dissolve)'s curve exactly as far as
    /// the item falls, so the two cross at the middle of it, as in an ordinary
    /// crossfade. Both are part-transparent there, so a little of the page behind
    /// shows through the window; the page behind is dimmed and a host can blur it,
    /// which hides most of that. Without a copy of the item there is nothing to
    /// cross, and the page fades in either way.
    pub fn crossfade(mut self, crossfade: bool) -> Self {
        self.crossfade = crossfade;
        self
    }

    /// Has the page scaled to fill the window instead of revealed through it. Off
    /// by default.
    ///
    /// The whole page is scaled, on each axis, by the window's fraction of the
    /// host and moved to the window's corner, exactly as the copy of the item is.
    /// Nothing about the page's own layout changes: it is a presentation of the
    /// page as laid out for the host. The page is squeezed into the item's shape,
    /// so a row much wider than it is tall flattens it; it is undistorted again
    /// once the window fills the host. The page's corner radius is that of the
    /// window as shown.
    pub fn stretch(mut self, stretch: bool) -> Self {
        self.stretch = stretch;
        self
    }

    /// Places the window's growth on the timeline, as fractions in `0.0..=1.0`.
    /// The default is all of it, `0.0..1.0`.
    ///
    /// Before the span the window is the item; after it, the host. A span with a
    /// bound that is not finite is the default, bounds are clamped into range,
    /// and a span that ends before it starts, or is shorter than a thousandth of
    /// the timeline, is made that long.
    pub fn grow(mut self, span: Range<f32>) -> Self {
        self.grow = StageSpans::span(span, DEFAULT_GROW);
        self
    }

    /// Places the item's dissolve into the page on the timeline, as fractions in
    /// `0.0..=1.0`. The default is `0.2..0.7`. Sanitized like
    /// [`grow`](Self::grow).
    pub fn dissolve(mut self, span: Range<f32>) -> Self {
        self.dissolve = StageSpans::span(span, DEFAULT_DISSOLVE);
        self
    }

    /// Holds the blur at its peak through the middle of the
    /// [dissolve](Self::dissolve), by `fraction` of it in `0.0..=0.95`, instead
    /// of a single swell. The default is `0.0`. A value that is not finite is
    /// ignored.
    pub fn blur_held(mut self, fraction: f32) -> Self {
        if fraction.is_finite() {
            self.blur_hold = fraction.clamp(0.0, MAX_BLUR_HOLD);
        }
        self
    }

    /// Sets the easing of the window's growth over its [span](Self::grow). The
    /// default is linear. An invalid curve (a non-finite Bézier control point) is
    /// ignored.
    pub fn grow_curve(mut self, curve: impl Curve) -> Self {
        self.grow_curve = valid_or(curve, self.grow_curve);
        self
    }

    /// Sets the easing of the item's dissolve over its [span](Self::dissolve).
    /// The default is linear. An invalid curve is ignored.
    pub fn dissolve_curve(mut self, curve: impl Curve) -> Self {
        self.dissolve_curve = valid_or(curve, self.dissolve_curve);
        self
    }

    /// How blurred the host should make everything at timeline `progress`, from
    /// `0.0` to `1.0` of the peak: nothing outside the [dissolve](Self::dissolve)
    /// and, inside it, a swell that peaks halfway through (or one
    /// [held](Self::blur_held) at its peak). Progress is of the expansion, so
    /// collapsing asks with reversed progress.
    pub fn blur(&self, progress: f32) -> f32 {
        let p = if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 1.0 };
        blur_swell(Self::through(self.dissolve, p), self.blur_hold)
    }

    /// How far through `span` the timeline `p` is, from `0.0` to `1.0`.
    ///
    /// A span that is all but done is done: the end must land exactly, and the
    /// division does not always.
    fn through((start, end): (f32, f32), p: f32) -> f32 {
        let done = ((p - start) / (end - start)).clamp(0.0, 1.0);
        if done > 0.999_99 { 1.0 } else { done }
    }

    /// The frame at timeline `progress`, `0.0` (the item) to `1.0` (the host).
    /// Progress outside that range is clamped, and one that is not a number
    /// counts as done.
    pub fn sample(&self, progress: f32) -> ContainerTransformFrame {
        let p = if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 1.0 };

        // The window grows on its own span and curve; the page behind darkens
        // with it, so it is shaded exactly as fast as it is covered.
        let q = self.grow_curve.transform(Self::through(self.grow, p));
        let lerp = |from: f32, to: f32| from + (to - from) * q;
        let window = ElementBounds::new(
            0.0,
            0.0,
            lerp(self.source.width, self.viewport.width).max(0.0),
            lerp(self.source.height, self.viewport.height).max(0.0),
        );
        // A window that has grown to the whole host clips nothing.
        let reveal = (q < 1.0 && !self.stretch).then_some(window);
        // A curve that overshoots must not carry the radius below zero: a
        // negative radius is declined by the host and the page would blink out.
        let corner_radius = lerp(self.source_radius, self.page_radius).max(0.0);
        let backdrop = PaintPresentation {
            opacity: 1.0 - self.scrim * q.clamp(0.0, 1.0),
            ..PaintPresentation::IDENTITY_OPAQUE
        };

        let dissolved = self.dissolve_curve.transform(Self::through(self.dissolve, p)).clamp(0.0, 1.0);
        // With a copy the page is opaque from the first frame, so what is behind
        // never shows through the window and the copy fades out over it, unless
        // the two cross, with the page rising as the copy falls. Without a copy
        // the page fades in over the dissolve.
        let (page_opacity, anchor_opacity) = match (self.anchored, self.crossfade) {
            (true, false) => (1.0, 1.0 - dissolved),
            (true, true) => (dissolved, 1.0 - dissolved),
            (false, _) => (dissolved, 0.0),
        };

        let position = Mat3::translate(lerp(self.source.x, 0.0), lerp(self.source.y, 0.0));
        // The page is as large as the host: scaled by how much of it the window is,
        // and then moved to the window's corner, it fills the window, like the copy.
        let page_transform = if self.stretch {
            position.mul(&Mat3::scale(
                window.width / self.viewport.width,
                window.height / self.viewport.height,
            ))
        } else {
            position
        };
        ContainerTransformFrame {
            page: PaintPresentation {
                transform: page_transform,
                opacity: page_opacity,
                corner_radius,
                reveal,
            },
            backdrop,
            // The copy is laid out at the item's size with its corner at the
            // origin, so it is stretched to the window's size and moved to the
            // window's corner: its box is the window, and it is the item that
            // grows. It is gone, dissolved into the page, before it is far from
            // the item's shape.
            anchor: PaintPresentation {
                transform: position.mul(&Mat3::scale(
                    window.width / self.source.width,
                    window.height / self.source.height,
                )),
                opacity: anchor_opacity,
                corner_radius: 0.0,
                reveal: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use aimer_attribute::size::ResolvedSize;
    use aimer_canvas::Mat3;

    use super::*;
    use crate::primitives::curve::curves;
    use crate::widgets::container_transform::ElementBounds;

    const VIEWPORT: ResolvedSize = ResolvedSize { width: 400.0, height: 640.0 };

    /// A row as wide as the viewport, a little way down it, with a copy of it.
    fn spec() -> ExpandSpec {
        ExpandSpec::new(ElementBounds::new(0.0, 260.0, 400.0, 160.0), VIEWPORT).unwrap().anchored(true)
    }

    fn height(spec: &ExpandSpec, p: f32) -> f32 {
        spec.sample(p).page.reveal.map_or(VIEWPORT.height, |window| window.height)
    }

    #[test]
    fn the_window_grows_from_the_item_to_the_viewport() {
        let spec = spec();
        let start = spec.sample(0.0);
        let end = spec.sample(1.0);

        assert_eq!(start.page.reveal, Some(ElementBounds::new(0.0, 0.0, 400.0, 160.0)));
        assert_eq!(start.page.transform.transform_point(0.0, 0.0), (0.0, 260.0));
        assert_eq!(start.page.corner_radius, 12.0);
        assert_eq!(end.page.reveal, None);
        assert_eq!(end.page.transform, Mat3::identity());
        assert_eq!(end.page.corner_radius, 0.0);
        // Halfway on a linear curve it is halfway there, its corner on the way up.
        let middle = spec.sample(0.5);
        assert!((height(&spec, 0.5) - 400.0).abs() < 1e-3);
        assert_eq!(middle.page.transform.transform_point(0.0, 0.0), (0.0, 130.0));
        assert!((middle.page.corner_radius - 6.0).abs() < 1e-5);
        // And never smaller on the way.
        let mut last = 0.0;
        for step in 0..=40 {
            let now = height(&spec, step as f32 / 40.0);
            assert!(now >= last - 1e-4, "step {step}");
            last = now;
        }
    }

    #[test]
    fn the_grow_span_places_the_growth_on_the_timeline() {
        let spec = spec().grow(0.2..0.8);

        assert_eq!(height(&spec, 0.0), 160.0);
        assert_eq!(height(&spec, 0.2), 160.0);
        assert!((height(&spec, 0.5) - 400.0).abs() < 1e-3);
        assert_eq!(spec.sample(0.8).page.reveal, None);
        assert_eq!(spec.sample(0.9).page.reveal, None);
    }

    #[test]
    fn the_item_dissolves_into_the_page_while_the_window_is_growing() {
        let spec = spec();

        // The window is on its way from the start; the item has not begun to go.
        assert!(height(&spec, 0.1) > 160.0);
        assert_eq!(spec.sample(0.1).anchor.opacity, 1.0);
        // Half way through the dissolve, the item is half gone and the window
        // is still opening.
        assert!((spec.sample(0.45).anchor.opacity - 0.5).abs() < 1e-5);
        assert!(height(&spec, 0.45) > height(&spec, 0.2) && height(&spec, 0.45) < VIEWPORT.height);
        // The item is gone before the window is full.
        assert!(spec.sample(0.7).anchor.opacity.abs() < 1e-5);
        assert!(height(&spec, 0.7) < VIEWPORT.height);
    }

    #[test]
    fn the_item_stretches_with_the_window_so_it_is_the_window_that_grows() {
        let spec = spec();

        for step in 0..=20 {
            let p = step as f32 / 20.0;
            let frame = spec.sample(p);
            let window = frame.page.reveal.unwrap_or(ElementBounds::new(0.0, 0.0, VIEWPORT.width, VIEWPORT.height));
            // The copy is laid out at the item's size with its corner at the
            // origin. Wherever the window is, the copy's corners are its corners.
            let (left, top) = frame.page.transform.transform_point(0.0, 0.0);
            let (right, bottom) = frame.anchor.transform.transform_point(400.0, 160.0);
            let (x, y) = frame.anchor.transform.transform_point(0.0, 0.0);
            assert!((x - left).abs() < 1e-3 && (y - top).abs() < 1e-3, "{p}: {x},{y} vs {left},{top}");
            assert!((right - (left + window.width)).abs() < 1e-3, "{p}: {right}");
            assert!((bottom - (top + window.height)).abs() < 1e-3, "{p}: {bottom}");
        }
        // It starts as the item, and halfway it is half way to the host.
        assert_eq!(spec.sample(0.0).anchor.transform.transform_point(400.0, 160.0), (400.0, 420.0));
        let (right, bottom) = spec.sample(0.5).anchor.transform.transform_point(400.0, 160.0);
        assert!((right - 400.0).abs() < 1e-3 && (bottom - 530.0).abs() < 1e-3, "{right},{bottom}");
    }

    #[test]
    fn the_page_is_opaque_from_the_first_frame_so_the_page_behind_never_shows_through() {
        for step in 0..=20 {
            assert_eq!(spec().sample(step as f32 / 20.0).page.opacity, 1.0, "{step}");
        }
    }

    #[test]
    fn a_crossfade_has_the_page_rise_exactly_as_the_item_falls() {
        let spec = spec().crossfade(true);

        // The page is not there until the dissolve starts, and is whole when it
        // ends; the item is the other way about.
        assert_eq!((spec.sample(0.0).page.opacity, spec.sample(0.2).page.opacity), (0.0, 0.0));
        assert_eq!((spec.sample(0.7).page.opacity, spec.sample(1.0).page.opacity), (1.0, 1.0));
        assert!((spec.sample(0.45).page.opacity - 0.5).abs() < 1e-5);
        // At every moment one is what the other is not.
        for step in 0..=40 {
            let frame = spec.sample(step as f32 / 40.0);
            assert!((frame.page.opacity + frame.anchor.opacity - 1.0).abs() < 1e-5, "{step}");
        }
    }

    #[test]
    fn a_crossfade_follows_the_dissolves_curve_in_both_directions() {
        let linear = spec().crossfade(true);
        let eased = spec().crossfade(true).dissolve_curve(curves::EASE_IN_OUT);

        // An S-curve is slow at both ends and quick in the middle, for the page
        // coming in and the item going out alike.
        assert!(eased.sample(0.3).page.opacity < linear.sample(0.3).page.opacity);
        assert!(eased.sample(0.6).page.opacity > linear.sample(0.6).page.opacity);
        assert!(eased.sample(0.3).anchor.opacity > linear.sample(0.3).anchor.opacity);
        assert!((eased.sample(0.45).page.opacity - 0.5).abs() < 1e-3);
        for step in 0..=40 {
            let frame = eased.sample(step as f32 / 40.0);
            assert!((frame.page.opacity + frame.anchor.opacity - 1.0).abs() < 1e-5, "{step}");
        }
    }

    #[test]
    fn a_crossfade_changes_nothing_but_the_pages_opacity() {
        let (plain, crossfade) = (spec(), spec().crossfade(true));

        for step in 0..=20 {
            let p = step as f32 / 20.0;
            let (plain, crossfade) = (plain.sample(p), crossfade.sample(p));
            assert_eq!(plain.anchor, crossfade.anchor, "{p}");
            assert_eq!(plain.backdrop, crossfade.backdrop, "{p}");
            assert_eq!(plain.page.transform, crossfade.page.transform, "{p}");
            assert_eq!(plain.page.reveal, crossfade.page.reveal, "{p}");
            assert_eq!(plain.page.corner_radius, crossfade.page.corner_radius, "{p}");
        }
        // Without one, the page is opaque from the first frame, as it always was.
        assert_eq!(plain.sample(0.0).page.opacity, 1.0);
    }

    #[test]
    fn a_crossfade_ends_exactly_and_is_ignored_without_a_copy() {
        for anchored in [false, true] {
            let spec = spec().anchored(anchored).crossfade(true).dissolve(0.3..0.6);
            assert_eq!(spec.sample(1.0).page.opacity, 1.0, "{anchored}");
            for step in 0..=60 {
                let frame = spec.sample(step as f32 / 60.0);
                assert!(frame.page.is_finite() && frame.anchor.is_finite(), "{anchored} {step}");
            }
        }
        // With no copy there is nothing to fall, and the page fades in as ever.
        let bare = ExpandSpec::new(ElementBounds::new(0.0, 260.0, 400.0, 160.0), VIEWPORT).unwrap();
        for step in 0..=20 {
            let p = step as f32 / 20.0;
            assert_eq!(bare.clone().crossfade(true).sample(p), bare.sample(p), "{p}");
        }
    }

    #[test]
    fn without_a_copy_of_the_item_the_page_fades_in_over_the_dissolve() {
        let spec = ExpandSpec::new(ElementBounds::new(0.0, 260.0, 400.0, 160.0), VIEWPORT).unwrap();

        assert_eq!(spec.sample(0.1).page.opacity, 0.0);
        assert!((spec.sample(0.45).page.opacity - 0.5).abs() < 1e-5);
        assert_eq!(spec.sample(0.9).page.opacity, 1.0);
        assert_eq!(spec.sample(0.45).anchor.opacity, 0.0);
    }

    #[test]
    fn the_page_behind_darkens_as_the_window_covers_it() {
        let spec = spec().scrim(0.5).grow_curve(curves::EASE_OUT);
        let dim = |p: f32| 1.0 - spec.sample(p).backdrop.opacity;

        assert_eq!(dim(0.0), 0.0);
        assert!((dim(1.0) - 0.5).abs() < 1e-6);
        // It follows the window, not the clock: a window that opens fast darkens
        // what it leaves uncovered just as fast.
        assert!(dim(0.25) > 0.5 * 0.25 + 0.02);
        let mut last = 0.0;
        for step in 0..=40 {
            let now = dim(step as f32 / 40.0);
            assert!(now >= last - 1e-6, "step {step}");
            last = now;
        }
    }

    #[test]
    fn each_curve_eases_only_its_own_element() {
        let linear = spec();
        let eased = spec().grow_curve(curves::EASE_OUT).dissolve_curve(curves::EASE_IN);

        // The window is further along on an ease-out, the item more still there
        // on an ease-in.
        assert!(height(&eased, 0.3) > height(&linear, 0.3));
        assert!(eased.sample(0.45).anchor.opacity > linear.sample(0.45).anchor.opacity);
        // The window's curve moves the item's box with the window, and leaves the
        // item's dissolve alone.
        let only_grow = spec().grow_curve(curves::EASE_OUT);
        assert_ne!(only_grow.sample(0.45).anchor.transform, linear.sample(0.45).anchor.transform);
        assert_eq!(only_grow.sample(0.45).anchor.opacity, linear.sample(0.45).anchor.opacity);
        // The dissolve's curve leaves the page and the item's box alone.
        let only_dissolve = spec().dissolve_curve(curves::EASE_IN);
        assert_eq!(only_dissolve.sample(0.3).page, linear.sample(0.3).page);
        assert_eq!(only_dissolve.sample(0.3).anchor.transform, linear.sample(0.3).anchor.transform);
    }

    #[test]
    fn the_blur_swells_through_the_dissolve_and_can_be_held() {
        let swell = spec();
        let held = spec().blur_held(0.5);

        // 0.2..0.7: nothing outside it, the peak in the middle.
        for spec in [&swell, &held] {
            assert_eq!(spec.blur(0.0), 0.0);
            assert!(spec.blur(0.2).abs() < 1e-6 && spec.blur(0.7).abs() < 1e-5);
            assert_eq!(spec.blur(1.0), 0.0);
            assert!((spec.blur(0.45) - 1.0).abs() < 1e-4);
            assert_eq!(spec.blur(f32::NAN), 0.0);
        }
        // Held, it is at its peak from a quarter to three quarters of the way.
        assert!((held.blur(0.35) - 1.0).abs() < 1e-4 && (held.blur(0.55) - 1.0).abs() < 1e-4);
        assert!(swell.blur(0.35) < 0.95);
    }

    #[test]
    fn it_ends_exactly_and_every_frame_can_be_shown() {
        for anchored in [false, true] {
            let spec = spec().anchored(anchored).grow(0.1..0.9).dissolve(0.3..0.6).blur_held(0.4);
            let end = spec.sample(1.0);

            assert_eq!((end.page.reveal, end.page.corner_radius, end.anchor.opacity), (None, 0.0, 0.0));
            assert_eq!(end.page.transform, Mat3::identity());
            assert_eq!(end.page.opacity, 1.0);
            for step in 0..=60 {
                let frame = spec.sample(step as f32 / 60.0);
                assert!(frame.page.is_finite() && frame.anchor.is_finite() && frame.backdrop.is_finite(), "{step}");
            }
        }
    }

    #[test]
    fn the_same_motion_can_be_pointed_at_other_geometry() {
        let tuned = spec().scrim(0.6).grow(0.1..0.9).dissolve(0.3..0.5).blur_held(0.4).source_radius(20.0).crossfade(true);
        let moved = ElementBounds::new(10.0, 40.0, 300.0, 100.0);
        let other = ResolvedSize { width: 320.0, height: 500.0 };

        let retargeted = tuned.retargeted(moved, other).unwrap();

        // Everything about the motion is kept; only where it happens changes.
        assert_eq!(retargeted, ExpandSpec::new(moved, other).unwrap().scrim(0.6).grow(0.1..0.9).dissolve(0.3..0.5)
            .blur_held(0.4).source_radius(20.0).crossfade(true).anchored(true));
        assert_eq!(retargeted.sample(0.0).page.reveal, Some(ElementBounds::new(0.0, 0.0, 300.0, 100.0)));
        assert_eq!(retargeted.sample(1.0).page.reveal, None);
        // An item or a viewport with no area is still nothing to describe.
        assert!(tuned.retargeted(ElementBounds::new(0.0, 0.0, 0.0, 5.0), other).is_none());
        assert!(tuned.retargeted(moved, ResolvedSize { width: 0.0, height: 5.0 }).is_none());
    }

    #[test]
    fn bad_input_is_made_showable() {
        let valid = ElementBounds::new(0.0, 260.0, 400.0, 160.0);

        // No item or no viewport to speak of: nothing to describe.
        assert!(ExpandSpec::new(ElementBounds::new(0.0, 0.0, 0.0, 10.0), VIEWPORT).is_none());
        assert!(ExpandSpec::new(valid, ResolvedSize { width: 0.0, height: 100.0 }).is_none());
        assert!(ExpandSpec::new(valid, ResolvedSize { width: f32::NAN, height: 100.0 }).is_none());
        // Spans that are reversed, empty or not numbers still give a frame.
        let nan = f32::NAN;
        for spec in [
            spec().grow(0.5..0.5).dissolve(0.9..0.1),
            spec().grow(nan..nan).dissolve(f32::INFINITY..0.0),
            spec().grow(-3.0..9.0).dissolve(0.0..0.0).scrim(nan).source_radius(-5.0).page_radius(nan),
        ] {
            for step in 0..=40 {
                let p = step as f32 / 40.0;
                let frame = spec.sample(p);
                assert!(frame.page.is_finite() && frame.anchor.is_finite() && frame.backdrop.is_finite(), "{p}");
                assert!(spec.blur(p).is_finite());
            }
            assert_eq!(spec.sample(1.0).page.reveal, None);
            // Progress that is not a number counts as done.
            assert_eq!(spec.sample(nan).page.reveal, None);
            assert_eq!(spec.sample(7.0).page.reveal, None);
            assert!(spec.sample(-1.0).page.reveal.is_some());
        }
        // A scrim is a fraction.
        assert_eq!(spec().scrim(7.0).sample(1.0).backdrop.opacity, 0.0);
    }

    fn close(actual: Mat3, expected: Mat3) {
        for (a, e) in actual.cols.iter().flatten().zip(expected.cols.iter().flatten()) {
            assert!((a - e).abs() < 1e-4, "{actual:?} vs {expected:?}");
        }
    }

    #[test]
    fn a_stretched_page_is_scaled_into_the_window_instead_of_revealed() {
        let spec = spec().stretch(true);

        // On the item: the whole page squeezed into the row's 400 x 160 and moved
        // onto it, with nothing clipped away but its corners.
        let start = spec.sample(0.0).page;
        assert_eq!(start.reveal, None);
        close(start.transform, Mat3::translate(0.0, 260.0).mul(&Mat3::scale(1.0, 160.0 / 640.0)));
        assert_eq!(start.corner_radius, 12.0);

        // Halfway the window is halfway between the row and the viewport.
        let middle = spec.sample(0.5).page;
        assert_eq!(middle.reveal, None);
        close(middle.transform, Mat3::translate(0.0, 130.0).mul(&Mat3::scale(1.0, 0.625)));

        // Filling the host it is the page as it is: no scale, no offset.
        let end = spec.sample(1.0).page;
        assert_eq!(end.reveal, None);
        close(end.transform, Mat3::identity());
        assert_eq!(end.corner_radius, 0.0);
    }

    #[test]
    fn a_page_is_revealed_unless_it_is_asked_to_stretch() {
        assert_eq!(ExpandSpec::new(ElementBounds::new(0.0, 0.0, 1.0, 1.0), VIEWPORT), ExpandSpec::new(ElementBounds::new(0.0, 0.0, 1.0, 1.0), VIEWPORT).map(|spec| spec.stretch(false)));
        let revealed = spec().sample(0.0).page;
        assert_eq!(revealed.reveal, Some(ElementBounds::new(0.0, 0.0, 400.0, 160.0)));
        close(revealed.transform, Mat3::translate(0.0, 260.0));
    }

    #[test]
    fn stretching_the_page_changes_nothing_else_about_the_motion() {
        let (revealed, stretched) = (spec(), spec().stretch(true));
        for p in [0.0, 0.1, 0.3, 0.5, 0.8, 1.0] {
            let (a, b) = (revealed.sample(p), stretched.sample(p));
            assert_eq!(a.anchor, b.anchor, "the item's copy at {p}");
            assert_eq!(a.backdrop, b.backdrop, "the page behind at {p}");
            assert_eq!(a.page.opacity, b.page.opacity, "the page's opacity at {p}");
            assert_eq!(a.page.corner_radius, b.page.corner_radius, "the corners at {p}");
        }
    }

    #[test]
    fn a_stretched_page_scales_every_axis_by_the_windows_fraction_of_the_host() {
        // An item that is neither as wide nor as tall as the host: both axes scale.
        let spec = ExpandSpec::new(ElementBounds::new(50.0, 100.0, 100.0, 80.0), VIEWPORT)
            .unwrap()
            .stretch(true);
        let start = spec.sample(0.0).page;
        close(start.transform, Mat3::translate(50.0, 100.0).mul(&Mat3::scale(0.25, 80.0 / 640.0)));
        // A curve that overshoots must not turn the page inside out.
        let overshoot = spec.grow_curve(curves::cubic_bezier(0.34, 1.56, 0.64, 1.0));
        for p in [0.0, 0.2, 0.5, 0.9, 1.0] {
            let transform = overshoot.sample(p).page.transform;
            assert!(transform.cols.iter().flatten().all(|value| value.is_finite()), "finite at {p}");
        }
    }

    #[test]
    fn retargeting_keeps_the_stretch() {
        let moved = spec()
            .stretch(true)
            .retargeted(ElementBounds::new(0.0, 100.0, 400.0, 160.0), VIEWPORT)
            .unwrap();
        assert_eq!(moved.sample(0.0).page.reveal, None);
        close(moved.sample(0.0).page.transform, Mat3::translate(0.0, 100.0).mul(&Mat3::scale(1.0, 0.25)));
    }
}
