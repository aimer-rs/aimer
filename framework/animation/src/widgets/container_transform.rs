//! The geometry of a container transform: a whole page growing out of an item.
//!
//! Tapping an item in a grid makes the destination page start scaled down
//! inside that item, with rounded corners, and grow to fill the viewport while
//! the page behind it dims. Going back plays the same motion in reverse into
//! the same item. This module is only the geometry; hosts such as the router
//! apply it to retained pages.

use std::ops::Range;

use aimer_attribute::size::ResolvedSize;
use aimer_canvas::Mat3;

use super::animated_paint::PaintPresentation;
use crate::primitives::curve::{AnyCurve, Curve};

/// A rectangle in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ElementBounds {
    /// Creates bounds from the top-left corner and size.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Whether the bounds are finite and enclose a positive area.
    pub fn is_valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite())
            && self.width > 0.0
            && self.height > 0.0
    }
}

/// Presentations of the layers of a container transform at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContainerTransformFrame {
    /// The page that grows out of the item (or shrinks back into it).
    pub page: PaintPresentation,
    /// The page behind it, which dims while the page is on top.
    pub backdrop: PaintPresentation,
    /// A copy of the item itself, drawn above everything else.
    ///
    /// It leads the motion: the item scales up to the page's card first, then
    /// dissolves into the page. Fully transparent unless the spec is
    /// [`anchored`](ContainerTransformSpec::anchored). The copy is laid out at
    /// the item's own size with its top-left at the host's origin.
    pub anchor: PaintPresentation,
}

/// Describes a page growing out of one item.
///
/// The page is scaled uniformly so its width matches the item's, which keeps
/// its own proportions: the card is as tall as the page is, and may overlap the
/// items below the one it grew from. Scale, position and corner radius then
/// interpolate to the full viewport.
///
/// That suits a tile much narrower than the viewport, but an item as wide as
/// the viewport, such as a row of a feed, leaves nothing to scale: the page
/// would only slide in. [`reveal`](Self::reveal) is the other way to grow: the
/// page is never scaled, and a rounded window onto it starts as the item's
/// bounds and expands to the viewport.
///
/// With an [`anchor`](Self::anchored) the first part of the motion belongs to
/// the item rather than the page. For the first quarter of the timeline (the
/// [`handoff`](Self::handoff)) a copy of the item scales up to the card's size
/// while the page stays hidden; around the handoff it dissolves into the page,
/// which then carries on to fill the viewport. Collapsing plays the same thing
/// in reverse, so the page shrinks, dissolves into the item, and the item
/// scales back into its slot.
///
/// `sample` takes the progress of the *expansion*: `0.0` is the page sitting on
/// the item and `1.0` is the page filling the viewport. Play it backwards to
/// collapse.
///
/// # Example
///
/// ```rust
/// use aimer_animation::{ContainerTransformSpec, ElementBounds};
/// use aimer_attribute::size::ResolvedSize;
///
/// let viewport = ResolvedSize { width: 400.0, height: 800.0 };
/// let item = ElementBounds::new(20.0, 120.0, 160.0, 160.0);
/// let spec = ContainerTransformSpec::new(item, viewport).unwrap();
///
/// // On the item at first, filling the screen at the end.
/// let start = spec.sample(0.0).page;
/// assert_eq!(start.transform.transform_point(0.0, 0.0), (20.0, 120.0));
/// assert_eq!(spec.sample(1.0).page.transform.transform_point(0.0, 0.0), (0.0, 0.0));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerTransformSpec {
    source: ElementBounds,
    viewport: ResolvedSize,
    source_radius: f32,
    page_radius: f32,
    scrim: f32,
    anchored: bool,
    reveal: bool,
    fit: bool,
    staged: bool,
    lift: f32,
    stage_overlap: f32,
    stage_spans: Option<StageSpans>,
    handoff: f32,
    blend: f32,
    anchor_curve: AnyCurve,
    page_curve: AnyCurve,
}

/// Where the page is fully opaque, as a fraction of the expansion, when there is
/// no anchor to hand off from. Fading in early hides the moment the page
/// replaces the item it grows from.
const FADE_IN_END: f32 = 0.25;

/// Where the item hands over to the page, as a fraction of the expansion.
const DEFAULT_HANDOFF: f32 = 0.25;

/// How much bigger the item's copy gets in the first stage of a
/// [staged](ContainerTransformSpec::staged) motion.
const DEFAULT_LIFT: f32 = 1.06;

/// The most the stages of a [staged](ContainerTransformSpec::staged) motion may
/// overlap. At `1.0` they would all be one stage, with nothing left to stage.
const MAX_STAGE_OVERLAP: f32 = 0.9;

/// How much of the expansion the item-to-page dissolve spans, centred on the
/// handoff.
const DEFAULT_BLEND: f32 = 0.1;

/// When the stages of a [staged](ContainerTransformSpec::staged) motion start,
/// for stages that overlap by `overlap`: the gap between the starts of two
/// stages, and the reciprocal of a stage's length, both as fractions of the
/// timeline.
///
/// Three stages of length `l` starting `l * (1 - overlap)` apart fill the
/// timeline exactly when `l = 1 / (3 - 2 * overlap)`. Without overlap that is a
/// third each, computed as `1 / 3` and `3`, as a motion with no overlap always
/// has been.
fn stage_timing(overlap: f32) -> (f32, f32) {
    let overlap = if overlap.is_finite() { overlap.clamp(0.0, MAX_STAGE_OVERLAP) } else { 0.0 };
    let inverse_length = 3.0 - 2.0 * overlap;
    ((1.0 - overlap) / inverse_length, inverse_length)
}

/// The blur at `stage`, from `0.0` to `1.0` of the way through the span it
/// swells in, with the middle `hold` of the span held at the peak.
///
/// Without a hold it is a single swell. With one it rises and falls over what
/// the hold leaves, each on a smoothstep so the blur is never stepped into or
/// out of.
pub(super) fn blur_swell(stage: f32, hold: f32) -> f32 {
    if hold <= 0.0 {
        return (stage * std::f32::consts::PI).sin().max(0.0);
    }
    let ramp = (1.0 - hold) / 2.0;
    let edge = (stage.min(1.0 - stage) / ramp).clamp(0.0, 1.0);
    edge * edge * (3.0 - 2.0 * edge)
}

/// The shortest a stage may be, as a fraction of the timeline. A stage of no
/// length would divide by zero where it is sampled.
const MIN_STAGE_SPAN: f32 = 1e-3;

/// The most of the turn the blur may be held through. At `1.0` it would have no
/// time to rise or to fall, and would step.
pub(super) const MAX_BLUR_HOLD: f32 = 0.95;

/// Where on the timeline each stage of a [staged](ContainerTransformSpec::staged)
/// motion runs.
///
/// The stages are the **lift** (the item's copy scales), the **turn** (the item
/// becomes the page, and a host can blur) and the **grow** (the window opens to
/// the viewport). Each is a span of the timeline, as fractions in `0.0..=1.0`,
/// and the spans are free: they may be long or short, and overlap or leave gaps.
/// Each stage's curve runs over its own span.
///
/// # Examples
///
/// ```
/// use aimer_animation::StageSpans;
///
/// // A long lift, with the turn at the end of it, then the window.
/// let spans = StageSpans::new(0.0..0.55, 0.4..0.55, 0.55..1.0);
/// assert_eq!(spans.turn(), 0.4..0.55);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageSpans {
    /// `(start, end)` of the lift, turn and grow, in that order.
    spans: [(f32, f32); 3],
    /// `(start, end)` of the dimming of the page behind.
    dim: (f32, f32),
    /// The fraction of the turn through which the blur is held at its peak.
    blur_hold: f32,
}

impl StageSpans {
    /// The three spans of a motion whose stages wait for each other: a third of
    /// the timeline each.
    const THIRDS: [(f32, f32); 3] = [(0.0, 1.0 / 3.0), (1.0 / 3.0, 2.0 / 3.0), (2.0 / 3.0, 1.0)];

    /// Places the lift, the turn and the grow on the timeline.
    ///
    /// A span with a bound that is not finite is replaced by that stage's third
    /// of the timeline. Finite bounds are clamped into `0.0..=1.0`, and a span
    /// that ends before it starts, or is shorter than a thousandth of the
    /// timeline, is made that long.
    pub fn new(lift: Range<f32>, turn: Range<f32>, grow: Range<f32>) -> Self {
        Self {
            spans: [
                Self::span(lift, Self::THIRDS[0]),
                Self::span(turn, Self::THIRDS[1]),
                Self::span(grow, Self::THIRDS[2]),
            ],
            dim: (0.0, 1.0),
            blur_hold: 0.0,
        }
    }

    /// Holds the blur at its peak through the middle of the [turn](Self::turn),
    /// instead of letting it swell to a peak and straight back.
    ///
    /// `fraction` is how much of the turn the blur stays at its peak, in
    /// `0.0..=0.95`: at `0.5` it rises over the first quarter of the turn, holds
    /// through the middle half, and falls over the last quarter. The default,
    /// `0.0`, is the single swell. A value that is not finite is ignored.
    pub fn blur_held(mut self, fraction: f32) -> Self {
        if fraction.is_finite() {
            self.blur_hold = fraction.clamp(0.0, MAX_BLUR_HOLD);
        }
        self
    }

    /// Dims the page behind only over `span` of the timeline, instead of over all
    /// of it, and along the page's curve, so it darkens in step with the window
    /// it is being covered by.
    ///
    /// That is how a page that fills the screen can seem to shade what it leaves
    /// behind: set the span to the [grow](Self::grow) and the page behind stays
    /// bright while the item dissolves, then darkens as the page takes over.
    /// The amount it reaches is the [scrim](ContainerTransformSpec::scrim).
    /// A span with a bound that is not finite is the whole timeline.
    pub fn dimmed_over(mut self, span: Range<f32>) -> Self {
        self.dim = Self::span(span, (0.0, 1.0));
        self
    }

    /// The span over which the page behind dims.
    pub fn dim(&self) -> Range<f32> {
        self.dim.0..self.dim.1
    }

    /// How far through the dimming the timeline `p` is, from `0.0` to `1.0`.
    fn dim_progress(&self, p: f32) -> f32 {
        ((p - self.dim.0) / (self.dim.1 - self.dim.0)).clamp(0.0, 1.0)
    }

    pub(super) fn span(range: Range<f32>, fallback: (f32, f32)) -> (f32, f32) {
        if !(range.start.is_finite() && range.end.is_finite()) {
            return fallback;
        }
        let start = range.start.clamp(0.0, 1.0);
        let end = range.end.clamp(0.0, 1.0).max(start + MIN_STAGE_SPAN).min(1.0);
        (start.min(end - MIN_STAGE_SPAN), end)
    }

    /// The span in which the item's copy scales.
    pub fn lift(&self) -> Range<f32> {
        self.spans[0].0..self.spans[0].1
    }

    /// The span in which the item turns into the page.
    pub fn turn(&self) -> Range<f32> {
        self.spans[1].0..self.spans[1].1
    }

    /// The span in which the window grows to the viewport.
    pub fn grow(&self) -> Range<f32> {
        self.spans[2].0..self.spans[2].1
    }

    /// How blurred a motion with these spans should be at `progress`, from `0.0`
    /// to `1.0` of the peak: nothing outside the [turn](Self::turn), and in it a
    /// swell that peaks halfway through, or one [held](Self::blur_held) at its
    /// peak. Progress is of the expansion, so
    /// collapsing asks with reversed progress.
    pub fn blur(&self, progress: f32) -> f32 {
        let progress = if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 1.0 };
        let (start, end) = self.spans[1];
        let stage = ((progress - start) / (end - start)).clamp(0.0, 1.0);
        blur_swell(stage, self.blur_hold)
    }

    /// Where each stage starts, and the reciprocal of its length.
    fn timing(&self) -> ([f32; 3], [f32; 3]) {
        (self.spans.map(|(start, _)| start), self.spans.map(|(start, end)| 1.0 / (end - start)))
    }
}

impl ContainerTransformSpec {
    /// Describes a page growing from `source` to fill `viewport`.
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
            reveal: false,
            fit: false,
            staged: false,
            lift: DEFAULT_LIFT,
            stage_overlap: 0.0,
            stage_spans: None,
            handoff: DEFAULT_HANDOFF,
            blend: DEFAULT_BLEND,
            anchor_curve: AnyCurve::default(),
            page_curve: AnyCurve::default(),
        })
    }

    /// Sets the corner radius of the page while it sits on the item, as shown.
    /// The default is 12 logical pixels. Negative or non-finite values are
    /// treated as zero.
    pub fn source_radius(mut self, radius: f32) -> Self {
        self.source_radius = sanitize(radius);
        self
    }

    /// Sets the corner radius of the page once it fills the viewport. The
    /// default is zero. A non-zero radius ends with square corners when the
    /// host removes the clip, so give the page the same radius itself.
    pub fn page_radius(mut self, radius: f32) -> Self {
        self.page_radius = sanitize(radius);
        self
    }

    /// Sets how much the page behind fades by the time the page is on top, in
    /// `0.0..=1.0`. The default is `0.35`.
    pub fn scrim(mut self, scrim: f32) -> Self {
        self.scrim = if scrim.is_finite() {
            scrim.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self
    }

    /// Draws a copy of the item over the pages, leading the motion.
    ///
    /// Without it the page alone grows out of the item, fading in over the first
    /// quarter. Enable it only when the item can actually be drawn again; see
    /// [`ContainerTransformFrame::anchor`].
    pub fn anchored(mut self, anchored: bool) -> Self {
        self.anchored = anchored;
        self
    }

    /// Grows the page by revealing it instead of scaling it.
    ///
    /// The page keeps its size. What shows of it is a window that starts as the
    /// item's bounds, with the page's top-left corner on the item's, and expands
    /// to fill the viewport while the page slides to the origin. Use it for an
    /// item that is as wide as the viewport, or any item that should not squeeze
    /// the page into its width. A page wider than the item is cropped at first.
    ///
    /// With an [`anchor`](Self::anchored) the copy of the item stays in its slot,
    /// above a page that is opaque from the start, and fades out over the
    /// [`handoff`](Self::handoff): the window already starts on the item, so
    /// the copy has nowhere to travel.
    pub fn reveal(mut self, reveal: bool) -> Self {
        self.reveal = reveal;
        self
    }

    /// Scales the page to fit inside the item, instead of to the item's width.
    ///
    /// The page keeps its proportions and is scaled about the item's centre: the
    /// part of the page under the item stays under it, and the rest grows out
    /// from there until the page fills the viewport, as an app does when it opens
    /// from its icon. Use it for an item that is as wide as the viewport, where
    /// scaling to its width would leave nothing to scale. With an
    /// [`anchor`](Self::anchored) the page is up almost at once and the copy of
    /// the item stays in its slot at its own size, fading out over the
    /// [`handoff`](Self::handoff). Ignored by a [`reveal`](Self::reveal)ed or
    /// [staged](Self::staged) motion.
    pub fn fit(mut self, fit: bool) -> Self {
        self.fit = fit;
        self
    }

    /// Plays the motion as three equal stages, one after the other.
    ///
    /// 1. **The item scales** (the first third): its copy grows by
    ///    [`lift`](Self::lift) about its centre, and the page is not there yet.
    /// 2. **The item turns into the page** (the middle third): the copy fades out
    ///    as the page, still no bigger than the item, fades in on it. A host can
    ///    blur everything through this stage; see
    ///    [`staged_blur`](Self::staged_blur).
    /// 3. **The page scales to fit the screen** (the last third): the window
    ///    onto it grows from the item's bounds to the viewport.
    ///
    /// The page is revealed rather than scaled, as with [`reveal`](Self::reveal),
    /// which this implies, and an [`anchor`](Self::anchored) is what the first
    /// stage scales; without one the first stage is still. Collapsing plays the
    /// stages in the opposite order. [`anchor_curve`](Self::anchor_curve) eases
    /// the first stage and [`page_curve`](Self::page_curve) the last; both run
    /// over their own stage, and the [`handoff`](Self::handoff) and
    /// [`blend`](Self::blend) are not used. The stages can run into each other
    /// instead of waiting; see [`stage_overlap`](Self::stage_overlap).
    pub fn staged(mut self, staged: bool) -> Self {
        self.staged = staged;
        self
    }

    /// Sets how much the item's copy scales in the first stage of a
    /// [staged](Self::staged) motion, where `1.0` is not at all. The default is
    /// `1.06`. A value that is not finite and positive is ignored.
    pub fn lift(mut self, lift: f32) -> Self {
        if lift.is_finite() && lift > 0.0 {
            self.lift = lift;
        }
        self
    }

    /// Lets the stages of a [staged](Self::staged) motion run into each other.
    ///
    /// `overlap` is the fraction of a stage that the next one starts early, in
    /// `0.0..=0.9`. At `0.0`, the default, each stage waits for the last, and the
    /// item stands still between the lift and the window. At `0.5` the page
    /// starts turning in halfway through the lift and the window starts opening
    /// halfway through the turn, so the motion never stops: the stages are each
    /// half the timeline, starting a quarter apart. Each stage keeps its curve
    /// over its own, longer, span. A non-finite value is ignored and a finite one
    /// is clamped into range.
    pub fn stage_overlap(mut self, overlap: f32) -> Self {
        if overlap.is_finite() {
            self.stage_overlap = overlap.clamp(0.0, MAX_STAGE_OVERLAP);
        }
        self
    }

    /// Places each stage of a [staged](Self::staged) motion on the timeline
    /// yourself; see [`StageSpans`]. This replaces
    /// [`stage_overlap`](Self::stage_overlap), which is then ignored. A host that
    /// blurs follows [`StageSpans::blur`] instead of
    /// [`staged_blur`](Self::staged_blur).
    pub fn stage_spans(mut self, spans: StageSpans) -> Self {
        self.stage_spans = Some(spans);
        self
    }

    /// How blurred a [staged](Self::staged) motion should be at `progress`, from
    /// `0.0` to `1.0` of the peak: nothing in the first and last stage, and in
    /// the middle stage, where the item turns into the page, a swell that peaks
    /// halfway through it. Progress is of the expansion, so collapsing asks with
    /// reversed progress.
    pub fn staged_blur(progress: f32) -> f32 {
        Self::staged_blur_with(progress, 0.0)
    }

    /// [`staged_blur`](Self::staged_blur) for stages that
    /// [overlap](Self::stage_overlap) by `overlap`: the swell spans the middle
    /// stage wherever that falls. An overlap that is not finite counts as none.
    pub fn staged_blur_with(progress: f32, overlap: f32) -> f32 {
        let progress = if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 1.0 };
        let (stride, inverse_length) = stage_timing(overlap);
        let stage = ((progress - stride) * inverse_length).clamp(0.0, 1.0);
        (stage * std::f32::consts::PI).sin().max(0.0)
    }

    /// Sets where the item hands over to the page, as a fraction of the
    /// expansion in `0.0..=1.0`. The default is `0.25`. A non-finite value is
    /// ignored.
    pub fn handoff(mut self, handoff: f32) -> Self {
        self.handoff = if handoff.is_finite() { handoff.clamp(0.0, 1.0) } else { DEFAULT_HANDOFF };
        self
    }

    /// Sets how much of the expansion the item-to-page dissolve spans, centred
    /// on the handoff, in `0.0..=1.0`. Zero swaps them at the handoff. The
    /// default is `0.1`. A non-finite value is ignored.
    pub fn blend(mut self, blend: f32) -> Self {
        self.blend = if blend.is_finite() { blend.clamp(0.0, 1.0) } else { DEFAULT_BLEND };
        self
    }

    /// Sets the easing of the item copy's travel from its slot to the card.
    ///
    /// The curve runs over the copy's own phase, from the start to the
    /// [`handoff`](Self::handoff): `0.0` is the copy on the item and `1.0` is the
    /// copy on the card. With [`reveal`](Self::reveal) the copy does not travel,
    /// so the curve shapes its fade out instead: `0.0` is fully shown and `1.0`
    /// is gone. It only reshapes the travel; the fade between copy and
    /// page keeps following the timeline, and the copy still lands exactly on the
    /// card at the handoff, whatever the curve. The default is
    /// [`curves::LINEAR`](crate::curves::LINEAR), and it does nothing without an
    /// [`anchor`](Self::anchored).
    ///
    /// This composes with the easing the host already applies to the timeline,
    /// so pair it with a linear timeline when the element curves should be the
    /// only easing. An invalid curve (a non-finite Bézier control point) is
    /// ignored.
    pub fn anchor_curve(mut self, curve: impl Curve) -> Self {
        self.anchor_curve = valid_or(curve, self.anchor_curve);
        self
    }

    /// Sets the easing of the page's growth.
    ///
    /// The curve runs over the whole expansion and reshapes the page card's
    /// scale, position and corner radius. The page's fade, the dissolve and the
    /// dimming of the page behind keep following the timeline. The item copy
    /// follows the card, so it picks up this curve after the handoff. The
    /// default is [`curves::LINEAR`](crate::curves::LINEAR). A curve that overshoots makes the page
    /// overshoot too, but never gives it a negative corner radius. An invalid
    /// curve is ignored.
    ///
    /// Collapsing runs the same curve over reversed time, so the exit mirrors
    /// the entrance.
    pub fn page_curve(mut self, curve: impl Curve) -> Self {
        self.page_curve = valid_or(curve, self.page_curve);
        self
    }

    /// Samples the expansion at `progress`.
    ///
    /// Progress is clamped to `0.0..=1.0`; a non-finite value is treated as
    /// settled, the same as `1.0`.
    pub fn sample(&self, progress: f32) -> ContainerTransformFrame {
        let p = if progress.is_finite() {
            progress.clamp(0.0, 1.0)
        } else {
            1.0
        };
        // The page card follows its own curve; everything timed (fades,
        // dimming) follows the timeline itself.
        let q = self.page_curve.transform(p);
        let lerp = |from: f32, to: f32| from + (to - from) * q;

        // Where the page starts: scaled to the item's width at its corner, or, to
        // fit, scaled to fit inside the item about the item's centre.
        let centre = (self.source.x + self.source.width / 2.0, self.source.y + self.source.height / 2.0);
        let (start_scale, start_x, start_y) = if self.fit {
            let fit = (self.source.width / self.viewport.width).min(self.source.height / self.viewport.height);
            (fit, centre.0 * (1.0 - fit), centre.1 * (1.0 - fit))
        } else {
            (self.source.width / self.viewport.width, self.source.x, self.source.y)
        };
        let scale = lerp(start_scale, 1.0);
        let (page_x, page_y) = (lerp(start_x, 0.0), lerp(start_y, 0.0));
        let transform = Mat3::translate(page_x, page_y).mul(&Mat3::scale(scale, scale));
        // The page behind dims with the timeline, unless the stages are placed by
        // hand and the dimming with them.
        let dim = match &self.stage_spans {
            Some(spans) if self.staged => self.page_curve.transform(spans.dim_progress(p)).clamp(0.0, 1.0),
            _ => p,
        };
        let backdrop = PaintPresentation {
            opacity: 1.0 - self.scrim * dim,
            ..PaintPresentation::IDENTITY_OPAQUE
        };
        // A curve that overshoots must not carry the radius below zero: a
        // negative radius is declined by the host and the page would blink out.
        let corner_radius = lerp(self.source_radius, self.page_radius).max(0.0);

        if self.staged {
            return self.sample_staged(p, backdrop);
        }
        if self.reveal {
            return self.sample_reveal(p, q, backdrop, corner_radius);
        }

        if !self.anchored {
            return ContainerTransformFrame {
                page: PaintPresentation {
                    transform,
                    opacity: (p / FADE_IN_END).min(1.0),
                    corner_radius,
                    reveal: None,
                },
                backdrop,
                anchor: PaintPresentation { opacity: 0.0, ..PaintPresentation::IDENTITY_OPAQUE },
            };
        }

        if self.fit {
            // The page is up almost at once, under the copy of the item, which stays
            // in its slot at its own size and fades out over the handoff: scaled up
            // with the page, it would swamp the small page it hands over to.
            let handed_over = if self.handoff > 0.0 {
                self.anchor_curve.transform((p / self.handoff).min(1.0)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            return ContainerTransformFrame {
                page: PaintPresentation {
                    transform,
                    opacity: (handed_over * 2.0).min(1.0),
                    corner_radius,
                    reveal: None,
                },
                backdrop,
                anchor: PaintPresentation {
                    transform: Mat3::translate(self.source.x, self.source.y),
                    opacity: 1.0 - handed_over,
                    corner_radius: 0.0,
                    reveal: None,
                },
            };
        }

        // How far through the dissolve: 0 before it, 1 after it.
        let half = self.blend / 2.0;
        let dissolve = if self.blend > 0.0 {
            ((p - (self.handoff - half)) / self.blend).clamp(0.0, 1.0)
        } else if p >= self.handoff {
            1.0
        } else {
            0.0
        };
        // How far the item has travelled to the card: it arrives at the handoff
        // and then keeps pace with the card.
        let lead = if self.handoff > 0.0 { self.anchor_curve.transform((p / self.handoff).min(1.0)) } else { 1.0 };
        // The item scaled to the card's width, which keeps the item's own
        // proportions, anchored at the card's corner.
        let card_scale = scale * self.viewport.width / self.source.width;
        let (anchor_scale, anchor_x, anchor_y) = (
            1.0 + (card_scale - 1.0) * lead,
            self.source.x + (page_x - self.source.x) * lead,
            self.source.y + (page_y - self.source.y) * lead,
        );

        ContainerTransformFrame {
            page: PaintPresentation {
                transform,
                // Fully in by the middle of the dissolve: the item above it is
                // still half there, so nothing behind shows through.
                opacity: (dissolve * 2.0).min(1.0),
                corner_radius,
                reveal: None,
            },
            backdrop,
            anchor: PaintPresentation {
                transform: Mat3::translate(anchor_x, anchor_y).mul(&Mat3::scale(anchor_scale, anchor_scale)),
                opacity: 1.0 - dissolve,
                corner_radius: 0.0,
                reveal: None,
            },
        }
    }

    /// The frame of a [staged](Self::staged) motion at timeline `p`.
    fn sample_staged(&self, p: f32, backdrop: PaintPresentation) -> ContainerTransformFrame {
        // How far through each stage, which is a third of the timeline unless the
        // stages overlap or are placed by hand.
        // A stage that is all but done is done: the stages are not exact in
        // floating point, and the end must land exactly.
        let (starts, inverse_lengths) = match &self.stage_spans {
            Some(spans) => spans.timing(),
            None => {
                let (stride, inverse_length) = stage_timing(self.stage_overlap);
                ([0.0, stride, 2.0 * stride], [inverse_length; 3])
            }
        };
        let stage = |index: usize| {
            let done = ((p - starts[index]) * inverse_lengths[index]).clamp(0.0, 1.0);
            if done > 0.999_99 { 1.0 } else { done }
        };
        let (scaling, turning, growing) = (stage(0), stage(1), stage(2));

        // The window grows only in the last stage, on the page's own curve.
        let q = self.page_curve.transform(growing);
        let lerp = |from: f32, to: f32| from + (to - from) * q;
        let window = ElementBounds::new(
            0.0,
            0.0,
            lerp(self.source.width, self.viewport.width).max(0.0),
            lerp(self.source.height, self.viewport.height).max(0.0),
        );
        // A window that has grown to the whole page clips nothing.
        let reveal = (q < 1.0).then_some(window);
        // Corners go from the item's to the page's as the window grows, and never
        // below zero, which a host declines.
        let corner_radius = lerp(self.source_radius, self.page_radius).max(0.0);

        // The item's copy scales about its centre in the first stage and holds
        // that size while it fades out in the second.
        let lift = 1.0 + (self.lift - 1.0) * self.anchor_curve.transform(scaling);
        // The copy is laid out at the item's size with its corner at the origin,
        // so it is moved onto the item and scaled about the item's centre.
        let anchor_transform = Mat3::translate(
            self.source.x + self.source.width / 2.0 * (1.0 - lift),
            self.source.y + self.source.height / 2.0 * (1.0 - lift),
        )
        .mul(&Mat3::scale(lift, lift));

        ContainerTransformFrame {
            page: PaintPresentation {
                transform: Mat3::translate(lerp(self.source.x, 0.0), lerp(self.source.y, 0.0)),
                // In by the middle of the second stage, under the copy that is
                // still fading out, so what is behind never shows through it.
                opacity: (turning * 2.0).min(1.0),
                corner_radius,
                reveal,
            },
            backdrop,
            anchor: PaintPresentation {
                transform: anchor_transform,
                opacity: if self.anchored { 1.0 - turning } else { 0.0 },
                corner_radius: 0.0,
                reveal: None,
            },
        }
    }

    /// The frame of a page that is revealed rather than scaled; see
    /// [`reveal`](Self::reveal). `p` is the timeline and `q` the page's own
    /// curve of it.
    fn sample_reveal(
        &self,
        p: f32,
        q: f32,
        backdrop: PaintPresentation,
        corner_radius: f32,
    ) -> ContainerTransformFrame {
        let lerp = |from: f32, to: f32| from + (to - from) * q;
        let window = ElementBounds::new(
            0.0,
            0.0,
            lerp(self.source.width, self.viewport.width).max(0.0),
            lerp(self.source.height, self.viewport.height).max(0.0),
        );
        let page_transform = Mat3::translate(lerp(self.source.x, 0.0), lerp(self.source.y, 0.0));
        // Once the window has grown to the whole page it clips nothing, so the
        // page is left unclipped, as it is when the host stops presenting it.
        let reveal = (q < 1.0).then_some(window);

        // Without a copy of the item the page fades in over it. With one the
        // page is opaque from the first frame, so the item underneath, which
        // stays on screen behind the pages, never shows through it, and the copy
        // above it fades out over the handoff: the item hands over to the page
        // in one clean dissolve, never three layers of text.
        let (page_opacity, anchor_opacity) = if self.anchored {
            let handed_over = if self.handoff > 0.0 {
                self.anchor_curve.transform((p / self.handoff).min(1.0))
            } else {
                1.0
            };
            (1.0, (1.0 - handed_over).clamp(0.0, 1.0))
        } else {
            ((p / FADE_IN_END).min(1.0), 0.0)
        };

        ContainerTransformFrame {
            page: PaintPresentation {
                transform: page_transform,
                opacity: page_opacity,
                corner_radius,
                reveal,
            },
            backdrop,
            // The window starts on the item, so the copy has nowhere to travel:
            // it stays put, as big as the item, and fades.
            anchor: PaintPresentation {
                transform: Mat3::translate(self.source.x, self.source.y),
                opacity: anchor_opacity,
                corner_radius: 0.0,
                reveal: None,
            },
        }
    }
}

/// `curve` unless it has a non-finite control point, in which case `fallback`.
pub(super) fn valid_or(curve: impl Curve, fallback: AnyCurve) -> AnyCurve {
    if curve.is_valid() { curve.into_any() } else { fallback }
}

pub(super) fn sanitize(radius: f32) -> f32 {
    if radius.is_finite() {
        radius.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::curve::curves;

    const VIEWPORT: ResolvedSize = ResolvedSize {
        width: 400.0,
        height: 800.0,
    };

    fn spec() -> ContainerTransformSpec {
        ContainerTransformSpec::new(ElementBounds::new(20.0, 120.0, 160.0, 160.0), VIEWPORT)
            .unwrap()
    }

    fn point(frame: &ContainerTransformFrame, x: f32, y: f32) -> (f32, f32) {
        frame.page.transform.transform_point(x, y)
    }

    #[test]
    fn at_the_start_the_page_sits_on_the_item_at_the_items_width() {
        let frame = spec().sample(0.0);

        assert_eq!(point(&frame, 0.0, 0.0), (20.0, 120.0));
        // The page's right edge lands on the item's right edge.
        assert_eq!(point(&frame, VIEWPORT.width, 0.0), (180.0, 120.0));
        // The page keeps its proportions, so it is taller than the item.
        assert_eq!(point(&frame, 0.0, VIEWPORT.height), (20.0, 120.0 + 320.0));
        assert_eq!(frame.page.corner_radius, 12.0);
        assert_eq!(frame.page.opacity, 0.0);
    }

    #[test]
    fn at_the_end_the_page_fills_the_viewport_unchanged() {
        let frame = spec().sample(1.0);

        assert_eq!(frame.page.transform, Mat3::identity());
        assert_eq!(frame.page.corner_radius, 0.0);
        assert_eq!(frame.page.opacity, 1.0);
        assert_eq!(frame.backdrop.opacity, 1.0 - 0.35);
    }

    #[test]
    fn halfway_is_halfway_in_scale_position_and_radius() {
        let frame = spec().sample(0.5);

        // Scale goes 0.4 -> 1.0, position (20, 120) -> (0, 0).
        assert_eq!(point(&frame, 0.0, 0.0), (10.0, 60.0));
        let right = point(&frame, VIEWPORT.width, 0.0);
        assert!((right.0 - (10.0 + 0.7 * 400.0)).abs() < 1e-3, "{right:?}");
        assert_eq!(right.1, 60.0);
        assert_eq!(frame.page.corner_radius, 6.0);
        assert_eq!(frame.page.opacity, 1.0);
    }

    #[test]
    fn the_page_grows_and_its_corners_straighten_monotonically() {
        let spec = spec().source_radius(24.0).page_radius(4.0);
        let mut last_width = f32::NEG_INFINITY;
        let mut last_radius = f32::INFINITY;

        for step in 0..=20 {
            let frame = spec.sample(step as f32 / 20.0);
            let width = point(&frame, VIEWPORT.width, 0.0).0 - point(&frame, 0.0, 0.0).0;
            assert!(width >= last_width, "step {step}");
            assert!(frame.page.corner_radius <= last_radius, "step {step}");
            last_width = width;
            last_radius = frame.page.corner_radius;
        }
        assert_eq!(spec.sample(1.0).page.corner_radius, 4.0);
    }

    #[test]
    fn the_page_fades_in_early_and_the_backdrop_dims_steadily() {
        let spec = spec().scrim(0.5);

        assert_eq!(spec.sample(0.0).page.opacity, 0.0);
        assert_eq!(spec.sample(0.125).page.opacity, 0.5);
        assert_eq!(spec.sample(0.25).page.opacity, 1.0);
        assert_eq!(spec.sample(0.0).backdrop.opacity, 1.0);
        assert_eq!(spec.sample(0.5).backdrop.opacity, 0.75);
        assert_eq!(spec.sample(1.0).backdrop.opacity, 0.5);
    }

    /// A row as wide as the viewport, scaled to fit inside itself.
    fn fit_spec() -> ContainerTransformSpec {
        ContainerTransformSpec::new(ElementBounds::new(0.0, 260.0, 400.0, 160.0), VIEWPORT)
            .unwrap()
            .anchored(true)
            .fit(true)
    }

    #[test]
    fn a_fitted_page_grows_about_the_items_centre_and_ends_filling_the_viewport() {
        let start = fit_spec().sample(0.0);
        // 160 / 800 = 0.2: the page is 80 x 160, scaled about the row's centre.
        assert_eq!(point(&start, 200.0, 340.0), (200.0, 340.0));
        assert_eq!(point(&start, 0.0, 0.0), (160.0, 272.0));
        assert_eq!(point(&start, VIEWPORT.width, VIEWPORT.height), (240.0, 432.0));

        // The point under the item's centre never moves, whatever the progress.
        for step in 0..=10 {
            let frame = fit_spec().sample(step as f32 / 10.0);
            let centre = point(&frame, 200.0, 340.0);
            assert!((centre.0 - 200.0).abs() < 1e-3 && (centre.1 - 340.0).abs() < 1e-3, "step {step}");
        }

        let end = fit_spec().sample(1.0);
        assert_eq!(end.page.transform, Mat3::identity());
        assert_eq!(end.page.opacity, 1.0);
    }

    #[test]
    fn a_fitted_page_is_up_early_as_the_items_copy_fades() {
        let spec = fit_spec();
        assert_eq!(spec.sample(0.0).page.opacity, 0.0);
        // Half way through the handoff, the page is fully in and the copy half out.
        assert_eq!(spec.sample(0.125).page.opacity, 1.0);
        assert_eq!(spec.sample(0.125).anchor.opacity, 0.5);
    }

    #[test]
    fn a_fitted_items_copy_stays_in_its_slot_and_dissolves() {
        let spec = fit_spec();
        let mut last = 1.0;
        for step in 0..=10 {
            let frame = spec.sample(step as f32 / 10.0);
            assert_eq!(frame.anchor.transform, Mat3::translate(0.0, 260.0), "step {step}");
            assert!(frame.anchor.opacity <= last, "step {step}");
            last = frame.anchor.opacity;
        }
        assert_eq!(spec.sample(0.0).anchor.opacity, 1.0);
        assert_eq!(spec.sample(1.0).anchor.opacity, 0.0);
    }

    /// A row as wide as the viewport, a third of the way down.
    fn row_spec() -> ContainerTransformSpec {
        ContainerTransformSpec::new(ElementBounds::new(0.0, 260.0, 400.0, 160.0), VIEWPORT)
            .unwrap()
            .reveal(true)
    }

    #[test]
    fn a_revealed_page_is_never_scaled() {
        for step in 0..=20 {
            let frame = row_spec().sample(step as f32 / 20.0);
            let (dx, dy) = point(&frame, 1.0, 1.0);
            let (ox, oy) = point(&frame, 0.0, 0.0);
            assert_eq!((dx - ox, dy - oy), (1.0, 1.0), "step {step}");
        }
    }

    #[test]
    fn a_revealed_page_starts_as_a_window_on_the_item() {
        let frame = row_spec().sample(0.0);

        // The page's corner is on the item's, and only an item-sized window of
        // it shows.
        assert_eq!(point(&frame, 0.0, 0.0), (0.0, 260.0));
        assert_eq!(frame.page.reveal, Some(ElementBounds::new(0.0, 0.0, 400.0, 160.0)));
        assert_eq!(frame.page.corner_radius, 12.0);
    }

    #[test]
    fn a_revealed_page_ends_filling_the_viewport_unchanged() {
        let frame = row_spec().sample(1.0);

        assert_eq!(frame.page.transform, Mat3::identity());
        // The window has grown to the whole page, which needs no clip.
        assert_eq!(frame.page.reveal, None);
        assert_eq!(frame.page.corner_radius, 0.0);
        assert_eq!(frame.page.opacity, 1.0);
    }

    #[test]
    fn the_window_grows_in_every_direction_as_the_page_rises() {
        let mut last = (f32::INFINITY, f32::NEG_INFINITY);
        for step in 0..=20 {
            let frame = row_spec().sample(step as f32 / 20.0);
            let window = frame.page.reveal.unwrap_or(ElementBounds::new(0.0, 0.0, 400.0, VIEWPORT.height));
            let top = point(&frame, 0.0, 0.0).1;
            let bottom = top + window.height;
            assert!(top <= last.0 && bottom >= last.1, "step {step}");
            last = (top, bottom);
        }
        assert_eq!(last, (0.0, VIEWPORT.height));
    }

    #[test]
    fn the_copy_of_the_item_stays_in_its_slot_and_dissolves_into_a_revealed_page() {
        let spec = row_spec().anchored(true);

        let start = spec.sample(0.0);
        assert_eq!(start.anchor.transform.transform_point(0.0, 0.0), (0.0, 260.0));
        // The page is opaque underneath from the first frame, so the item
        // behind the pages never shows through it.
        assert_eq!((start.anchor.opacity, start.page.opacity), (1.0, 1.0));
        let quarter = spec.sample(0.125);
        assert_eq!((quarter.anchor.opacity, quarter.page.opacity), (0.5, 1.0));
        let middle = spec.sample(0.5);
        assert_eq!(middle.anchor.transform.transform_point(0.0, 0.0), (0.0, 260.0));
        assert_eq!((middle.anchor.opacity, middle.page.opacity), (0.0, 1.0));
        assert_eq!(
            middle.anchor.transform.transform_point(1.0, 1.0),
            (1.0, 261.0),
            "the copy is not scaled"
        );
    }

    #[test]
    fn the_anchor_curve_shapes_the_fade_of_a_revealed_pages_copy_and_not_its_ends() {
        let linear = row_spec().anchored(true);
        let eased = row_spec().anchored(true).anchor_curve(curves::EASE_OUT);

        // An ease-out has done more than half its work by the middle.
        assert!(eased.sample(0.125).anchor.opacity < linear.sample(0.125).anchor.opacity);
        assert_eq!(eased.sample(0.0).anchor.opacity, 1.0);
        assert_eq!(eased.sample(0.25).anchor.opacity, 0.0);
        // The page is under the copy from the start whatever the curve.
        assert_eq!(eased.sample(0.0).page.opacity, 1.0);
    }

    /// The row of [`row_spec`], played as three stages with a copy of the item.
    fn staged_spec() -> ContainerTransformSpec {
        row_spec().anchored(true).staged(true)
    }

    #[test]
    fn in_the_first_third_only_the_item_scales_and_the_page_is_not_there() {
        let spec = staged_spec();

        let start = spec.sample(0.0);
        let end = spec.sample(1.0 / 3.0);

        // The copy starts as the item and ends lifted about its centre.
        assert_eq!(start.anchor.transform.transform_point(0.0, 0.0), (0.0, 260.0));
        let corner = end.anchor.transform.transform_point(0.0, 0.0);
        assert!((corner.0 - -12.0).abs() < 1e-3 && (corner.1 - 260.0 + 4.8).abs() < 1e-3, "{corner:?}");
        let far = end.anchor.transform.transform_point(400.0, 160.0);
        assert!((far.0 - 412.0).abs() < 1e-3 && (far.1 - (420.0 + 4.8)).abs() < 1e-3, "{far:?}");
        assert_eq!((start.anchor.opacity, end.anchor.opacity), (1.0, 1.0));
        assert_eq!((start.page.opacity, end.page.opacity), (0.0, 0.0));
        // The page's window has not started to grow.
        assert_eq!(end.page.reveal, Some(ElementBounds::new(0.0, 0.0, 400.0, 160.0)));
    }

    #[test]
    fn in_the_middle_third_the_item_turns_into_the_page_without_either_moving() {
        let spec = staged_spec();

        let start = spec.sample(1.0 / 3.0);
        let middle = spec.sample(0.5);
        let end = spec.sample(2.0 / 3.0);

        assert_eq!((start.anchor.opacity, start.page.opacity), (1.0, 0.0));
        assert!((middle.anchor.opacity - 0.5).abs() < 1e-5 && (middle.page.opacity - 1.0).abs() < 1e-5);
        assert!(end.anchor.opacity.abs() < 1e-5 && (end.page.opacity - 1.0).abs() < 1e-5);
        // The copy holds its size, and the page its place and size, throughout.
        for frame in [start, middle, end] {
            assert_eq!(frame.anchor.transform, start.anchor.transform);
            assert_eq!(frame.page.transform, start.page.transform);
            assert_eq!(frame.page.reveal, Some(ElementBounds::new(0.0, 0.0, 400.0, 160.0)));
        }
    }

    #[test]
    fn in_the_last_third_the_page_grows_to_fill_the_screen() {
        let spec = staged_spec();

        let start = spec.sample(2.0 / 3.0);
        let middle = spec.sample(5.0 / 6.0);
        let end = spec.sample(1.0);

        assert_eq!(start.page.reveal, Some(ElementBounds::new(0.0, 0.0, 400.0, 160.0)));
        assert_eq!(point(&start, 0.0, 0.0), (0.0, 260.0));
        // Halfway through the stage the window is halfway to the viewport.
        let window = middle.page.reveal.unwrap();
        assert!((window.width - 400.0).abs() < 1e-3 && (window.height - 480.0).abs() < 1e-2, "{window:?}");
        let corner = point(&middle, 0.0, 0.0);
        assert!(corner.0.abs() < 1e-3 && (corner.1 - 130.0).abs() < 1e-2, "{corner:?}");
        assert_eq!(end.page.transform, Mat3::identity());
        assert_eq!(end.page.reveal, None);
        assert_eq!(end.page.corner_radius, 0.0);
        assert_eq!(end.anchor.opacity, 0.0);
    }

    #[test]
    fn each_stage_runs_its_own_curve_over_its_own_third() {
        let eased = staged_spec().anchor_curve(curves::EASE_OUT).page_curve(curves::EASE_IN);
        let linear = staged_spec();

        // The first stage's curve eases the lift, and only the lift.
        let lifted = |spec: &ContainerTransformSpec| spec.sample(1.0 / 6.0).anchor.transform.transform_point(400.0, 0.0).0;
        assert!(lifted(&eased) > lifted(&linear));
        assert_eq!(eased.sample(1.0 / 3.0).anchor.transform, linear.sample(1.0 / 3.0).anchor.transform);
        // The last stage's curve eases the window, and only the window.
        let height = |spec: &ContainerTransformSpec| spec.sample(5.0 / 6.0).page.reveal.unwrap().height;
        assert!(height(&eased) < height(&linear));
        assert_eq!(eased.sample(0.5).page, linear.sample(0.5).page);
    }

    #[test]
    fn the_lift_is_configurable_and_a_bad_one_is_ignored() {
        let end = |spec: ContainerTransformSpec| spec.sample(1.0 / 3.0).anchor.transform.transform_point(400.0, 0.0).0;

        assert_eq!(end(staged_spec().lift(1.0)), 400.0);
        assert!(end(staged_spec().lift(1.2)) > end(staged_spec()));
        for bad in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert_eq!(end(staged_spec().lift(bad)), end(staged_spec()), "{bad}");
        }
    }

    #[test]
    fn the_blur_swells_in_the_middle_third_and_is_gone_either_side() {
        let blur = ContainerTransformSpec::staged_blur;

        assert_eq!(blur(0.0), 0.0);
        assert_eq!(blur(1.0 / 3.0), 0.0);
        assert!((blur(0.5) - 1.0).abs() < 1e-6);
        assert!(blur(0.4) > 0.0 && blur(0.4) < blur(0.5));
        assert!(blur(2.0 / 3.0).abs() < 1e-6);
        assert_eq!(blur(1.0), 0.0);
        assert_eq!(blur(f32::NAN), 0.0);
    }

    #[test]
    fn overlapping_stages_run_into_each_other() {
        // Half overlap: each stage is half the timeline, starting a quarter after
        // the last, so 0..0.5 lifts, 0.25..0.75 turns and 0.5..1 grows.
        let spec = staged_spec().stage_overlap(0.5);
        let corner = |frame: &ContainerTransformFrame| frame.anchor.transform.transform_point(400.0, 0.0).0;

        // The page starts turning in while the item is still being lifted.
        let early = spec.sample(0.375);
        assert!(early.page.opacity > 0.0 && early.anchor.opacity < 1.0);
        assert!(corner(&early) > 400.0 && corner(&early) < corner(&spec.sample(0.5)));
        // The window starts to open while the item is still turning into the page.
        let late = spec.sample(0.625);
        assert!(late.anchor.opacity > 0.0 && late.anchor.opacity < 1.0);
        assert!(late.page.reveal.unwrap().height > 160.0);
        // Nothing waits: the lift is done where the window starts, and the copy is
        // gone where the window is done.
        assert!((corner(&spec.sample(0.5)) - corner(&spec.sample(1.0))).abs() < 1e-3);
        assert_eq!(spec.sample(0.5).page.reveal, Some(ElementBounds::new(0.0, 0.0, 400.0, 160.0)));
        assert!(spec.sample(0.75).anchor.opacity.abs() < 1e-5);
    }

    #[test]
    fn overlapping_stages_still_start_and_end_exactly() {
        for overlap in [0.25, 0.5, 0.9] {
            let spec = staged_spec().stage_overlap(overlap);
            let (start, end) = (spec.sample(0.0), spec.sample(1.0));

            assert_eq!(start.anchor.transform, staged_spec().sample(0.0).anchor.transform, "{overlap}");
            assert_eq!((start.anchor.opacity, start.page.opacity), (1.0, 0.0), "{overlap}");
            assert_eq!(end.page.transform, Mat3::identity(), "{overlap}");
            assert_eq!((end.page.reveal, end.page.corner_radius, end.anchor.opacity), (None, 0.0, 0.0), "{overlap}");
        }
    }

    #[test]
    fn no_overlap_is_three_stages_one_after_the_other() {
        for step in 0..=30 {
            let p = step as f32 / 30.0;
            assert_eq!(staged_spec().stage_overlap(0.0).sample(p), staged_spec().sample(p), "{p}");
        }
    }

    #[test]
    fn a_bad_overlap_is_ignored_or_kept_in_range() {
        for bad in [f32::NAN, f32::INFINITY, -1.0] {
            assert_eq!(staged_spec().stage_overlap(bad).sample(0.4), staged_spec().sample(0.4), "{bad}");
        }
        // Everything at once is as far as it goes; it must still be showable.
        for overlap in [1.0, 5.0] {
            for step in 0..=60 {
                let frame = staged_spec().stage_overlap(overlap).sample(step as f32 / 60.0);
                assert!(frame.page.is_finite() && frame.anchor.is_finite(), "{overlap} {step}");
            }
        }
    }

    #[test]
    fn the_blur_follows_the_middle_stage_when_stages_overlap() {
        let blur = |progress| ContainerTransformSpec::staged_blur_with(progress, 0.5);

        assert_eq!(blur(0.0), 0.0);
        assert!(blur(0.25).abs() < 1e-6);
        assert!(blur(0.375) > 0.0 && blur(0.375) < blur(0.5));
        assert!((blur(0.5) - 1.0).abs() < 1e-6);
        assert!(blur(0.75).abs() < 1e-6);
        assert_eq!(blur(1.0), 0.0);
        assert_eq!(blur(f32::NAN), 0.0);
        for progress in [0.0, 0.2, 0.5, 0.9, 1.0] {
            assert_eq!(ContainerTransformSpec::staged_blur_with(progress, 0.0), ContainerTransformSpec::staged_blur(progress));
        }
    }

    /// A long lift, a short turn at the end of it, then the window.
    fn long_lift() -> StageSpans {
        StageSpans::new(0.0..0.55, 0.4..0.55, 0.55..1.0)
    }

    #[test]
    fn stage_spans_place_each_stage_where_they_say() {
        let spec = staged_spec().stage_spans(long_lift());
        let corner = |frame: &ContainerTransformFrame| frame.anchor.transform.transform_point(400.0, 0.0).0;

        // The lift is under way for most of the first half, and the page is not.
        let early = spec.sample(0.2);
        assert!(corner(&early) > 400.0 && corner(&early) < corner(&spec.sample(0.55)));
        assert_eq!((early.anchor.opacity, early.page.opacity), (1.0, 0.0));
        // The turn is only 0.4..0.55: the copy is whole at its start, half there
        // in the middle, and gone at its end, with the page in by then.
        let start = spec.sample(0.4);
        assert_eq!((start.anchor.opacity, start.page.opacity), (1.0, 0.0));
        assert!((spec.sample(0.475).anchor.opacity - 0.5).abs() < 1e-4);
        let end = spec.sample(0.55);
        assert!(end.anchor.opacity.abs() < 1e-5 && (end.page.opacity - 1.0).abs() < 1e-5);
        // The lift finishes as the turn does, and the window waits for both.
        assert!((corner(&end) - corner(&spec.sample(1.0))).abs() < 1e-3);
        assert_eq!(end.page.reveal, Some(ElementBounds::new(0.0, 0.0, 400.0, 160.0)));
        // The window then has the last 0.45 to itself: halfway is 0.775.
        let window = spec.sample(0.775).page.reveal.unwrap();
        assert!((window.height - 480.0).abs() < 1e-2, "{window:?}");
        assert_eq!(spec.sample(1.0).page.reveal, None);
    }

    #[test]
    fn stage_spans_still_start_and_end_exactly() {
        let spec = staged_spec().stage_spans(long_lift());
        let (start, end) = (spec.sample(0.0), spec.sample(1.0));

        assert_eq!(start.anchor.transform, staged_spec().sample(0.0).anchor.transform);
        assert_eq!((start.anchor.opacity, start.page.opacity), (1.0, 0.0));
        assert_eq!(end.page.transform, Mat3::identity());
        assert_eq!((end.page.reveal, end.page.corner_radius, end.anchor.opacity), (None, 0.0, 0.0));
    }

    #[test]
    fn stage_spans_win_over_the_overlap() {
        let both = staged_spec().stage_overlap(0.5).stage_spans(long_lift());
        let spans = staged_spec().stage_spans(long_lift());

        for step in 0..=30 {
            let p = step as f32 / 30.0;
            assert_eq!(both.sample(p), spans.sample(p), "{p}");
        }
    }

    #[test]
    fn bad_stage_spans_are_made_showable() {
        let nan = f32::NAN;
        for spans in [
            StageSpans::new(0.5..0.5, 0.7..0.2, 1.0..1.0),
            StageSpans::new(nan..nan, f32::INFINITY..0.0, -3.0..9.0),
            StageSpans::new(0.0..0.0, 0.0..0.0, 0.0..0.0),
        ] {
            for step in 0..=60 {
                let p = step as f32 / 60.0;
                let frame = staged_spec().stage_spans(spans).sample(p);
                assert!(frame.page.is_finite() && frame.anchor.is_finite(), "{spans:?} {p}");
                assert!(spans.blur(p).is_finite(), "{spans:?} {p}");
            }
            assert_eq!(staged_spec().stage_spans(spans).sample(1.0).page.reveal, None, "{spans:?}");
        }
    }

    #[test]
    fn the_blur_swells_through_the_turn_span_only() {
        let spans = long_lift();

        assert_eq!(spans.blur(0.0), 0.0);
        assert!(spans.blur(0.4).abs() < 1e-6);
        assert!(spans.blur(0.44) > 0.0 && spans.blur(0.44) < spans.blur(0.475));
        assert!((spans.blur(0.475) - 1.0).abs() < 1e-5);
        assert!(spans.blur(0.55).abs() < 1e-5);
        assert_eq!(spans.blur(1.0), 0.0);
        assert_eq!(spans.blur(f32::NAN), 0.0);
    }

    #[test]
    fn the_page_behind_dims_over_its_own_span_along_the_pages_growth() {
        let dimmed = staged_spec()
            .scrim(0.5)
            .page_curve(curves::EASE_OUT)
            .stage_spans(long_lift().dimmed_over(0.7..1.0));
        let dim = |p: f32| 1.0 - dimmed.sample(p).backdrop.opacity;

        // Untouched until the span begins, then fully dimmed where it ends.
        assert_eq!(dim(0.0), 0.0);
        assert_eq!(dim(0.5), 0.0);
        assert_eq!(dim(0.7), 0.0);
        assert!((dim(1.0) - 0.5).abs() < 1e-6);
        // It rises in step with the page's curve, not linearly: a page that
        // grows fast darkens what it leaves uncovered just as fast.
        let linear = 0.5 * (0.85 - 0.7) / 0.3;
        assert!(dim(0.85) > linear + 0.05, "{} vs {linear}", dim(0.85));
        let mut last = 0.0;
        for step in 0..=60 {
            let now = dim(step as f32 / 60.0);
            assert!(now >= last - 1e-6, "step {step}");
            last = now;
        }
    }

    #[test]
    fn without_a_dim_span_the_page_behind_dims_over_the_whole_timeline() {
        let spec = staged_spec().scrim(0.5);
        let placed = staged_spec().scrim(0.5).stage_spans(long_lift());

        for step in 0..=10 {
            let p = step as f32 / 10.0;
            assert!((spec.sample(p).backdrop.opacity - (1.0 - 0.5 * p)).abs() < 1e-6, "{p}");
            assert!((placed.sample(p).backdrop.opacity - (1.0 - 0.5 * p)).abs() < 1e-6, "{p}");
        }
    }

    #[test]
    fn a_bad_dim_span_falls_back_to_the_whole_timeline() {
        let bad = long_lift().dimmed_over(f32::NAN..0.5);

        assert_eq!(bad.dim(), 0.0..1.0);
        assert_eq!(long_lift().dimmed_over(0.9..0.1).dim().start, 0.9);
        for step in 0..=20 {
            let frame = staged_spec().stage_spans(long_lift().dimmed_over(0.5..0.5)).sample(step as f32 / 20.0);
            assert!(frame.backdrop.is_finite(), "{step}");
        }
    }

    #[test]
    fn a_held_blur_stays_at_its_peak_through_the_middle_of_the_turn() {
        // The turn is 0.4..0.55; holding half of it leaves a quarter to rise in
        // and a quarter to fall out of, so the peak is held from 0.4375 to 0.5125.
        let held = long_lift().blur_held(0.5);

        assert!(held.blur(0.4).abs() < 1e-6 && held.blur(0.55).abs() < 1e-5);
        assert!(held.blur(0.41) > 0.0 && held.blur(0.41) < 1.0);
        for p in [0.4375, 0.45, 0.475, 0.5, 0.5125] {
            assert!((held.blur(p) - 1.0).abs() < 1e-4, "{p}: {}", held.blur(p));
        }
        assert!(held.blur(0.54) > 0.0 && held.blur(0.54) < 1.0);
        // It rises and falls without a step, and is never beyond the peak.
        let mut last = 0.0;
        for step in 0..=75 {
            let p = 0.4 + step as f32 * 0.002;
            let now = held.blur(p);
            assert!((0.0..=1.0 + 1e-6).contains(&now), "{p}: {now}");
            if p < 0.4375 {
                assert!(now >= last - 1e-6, "{p}");
            }
            assert!((now - last).abs() < 0.2, "{p}: {last} -> {now}");
            last = now;
        }
        // Held for longer than the swell would be at the same points.
        assert!(held.blur(0.43) > long_lift().blur(0.43));
    }

    #[test]
    fn a_blur_that_is_not_held_is_the_swell_it_always_was() {
        let plain = long_lift();

        for step in 0..=20 {
            let p = step as f32 / 20.0;
            assert_eq!(plain.blur_held(0.0).blur(p), plain.blur(p), "{p}");
        }
        assert!((plain.blur(0.475) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_bad_hold_is_ignored_or_kept_in_range() {
        let plain = long_lift();

        for bad in [f32::NAN, f32::INFINITY, -1.0] {
            assert_eq!(plain.blur_held(bad), plain, "{bad}");
        }
        // Holding all of it would leave nothing to rise or fall in.
        for hold in [1.0, 7.0] {
            for step in 0..=40 {
                let blur = plain.blur_held(hold).blur(step as f32 / 40.0);
                assert!(blur.is_finite() && (0.0..=1.0 + 1e-6).contains(&blur), "{hold}");
            }
        }
    }

    #[test]
    fn every_staged_presentation_can_be_shown() {
        for anchored in [false, true] {
            for step in 0..=60 {
                let frame = row_spec().anchored(anchored).staged(true).sample(step as f32 / 60.0);
                assert!(frame.page.is_finite() && frame.anchor.is_finite(), "{anchored} {step}");
            }
        }
    }

    #[test]
    fn every_revealed_presentation_can_be_shown() {
        for anchored in [false, true] {
            for step in 0..=50 {
                let frame = row_spec().anchored(anchored).sample(step as f32 / 50.0);
                assert!(frame.page.is_finite() && frame.anchor.is_finite(), "{anchored} {step}");
            }
        }
    }

    #[test]
    fn a_curve_that_overshoots_never_gives_the_window_a_negative_size() {
        // An undershoot carries the interpolation below zero, where a window
        // between the item's size and the viewport's would go negative.
        let spec = row_spec().page_curve(curves::cubic_bezier(0.3, -3.0, 0.6, 1.0));
        for step in 0..=50 {
            // At the end there is no window, which is all of the page.
            if let Some(window) = spec.sample(step as f32 / 50.0).page.reveal {
                assert!(window.width >= 0.0 && window.height >= 0.0, "step {step}");
            }
        }
    }

    #[test]
    fn every_sampled_presentation_can_be_shown() {
        for step in 0..=50 {
            let frame = spec().sample(step as f32 / 50.0);
            assert!(frame.page.is_finite() && frame.backdrop.is_finite());
        }
    }

    #[test]
    fn out_of_range_progress_clamps_and_non_finite_progress_settles() {
        let spec = spec();

        assert_eq!(spec.sample(-3.0), spec.sample(0.0));
        assert_eq!(spec.sample(9.0), spec.sample(1.0));
        assert_eq!(spec.sample(f32::NAN), spec.sample(1.0));
        assert_eq!(spec.sample(f32::NEG_INFINITY), spec.sample(1.0));
    }

    #[test]
    fn invalid_geometry_is_rejected() {
        let good = ElementBounds::new(0.0, 0.0, 10.0, 10.0);
        for source in [
            ElementBounds::new(0.0, 0.0, 0.0, 10.0),
            ElementBounds::new(0.0, 0.0, 10.0, -1.0),
            ElementBounds::new(f32::NAN, 0.0, 10.0, 10.0),
            ElementBounds::new(0.0, 0.0, f32::INFINITY, 10.0),
        ] {
            assert!(ContainerTransformSpec::new(source, VIEWPORT).is_none());
        }
        for viewport in [
            ResolvedSize {
                width: 0.0,
                height: 800.0,
            },
            ResolvedSize {
                width: 400.0,
                height: f32::NAN,
            },
        ] {
            assert!(ContainerTransformSpec::new(good, viewport).is_none());
        }
    }

    #[test]
    fn radii_and_scrim_are_sanitized() {
        let spec = spec()
            .source_radius(-5.0)
            .page_radius(f32::NAN)
            .scrim(f32::NAN);

        let frame = spec.sample(0.0);
        assert_eq!(frame.page.corner_radius, 0.0);
        assert_eq!(spec.sample(1.0).page.corner_radius, 0.0);
        assert_eq!(spec.sample(1.0).backdrop.opacity, 1.0);
        assert_eq!(self::spec().scrim(7.0).sample(1.0).backdrop.opacity, 0.0);
    }

    /// The spec with an anchor (the item drawn as an overlay) and the default
    /// 25% handoff dissolved over 10% of the timeline.
    fn anchored() -> ContainerTransformSpec {
        spec().anchored(true)
    }

    fn corner_of(presentation: &PaintPresentation, x: f32, y: f32) -> (f32, f32) {
        presentation.transform.transform_point(x, y)
    }

    #[test]
    fn at_the_start_the_anchor_is_the_item_itself_and_the_page_is_hidden() {
        let frame = anchored().sample(0.0);

        // The item is 160x160 at (20, 120): the overlay copy lies exactly on it.
        assert_eq!(corner_of(&frame.anchor, 0.0, 0.0), (20.0, 120.0));
        assert_eq!(corner_of(&frame.anchor, 160.0, 160.0), (180.0, 280.0));
        assert_eq!(frame.anchor.opacity, 1.0);
        assert_eq!(frame.page.opacity, 0.0);
    }

    #[test]
    fn at_the_handoff_the_anchor_has_grown_to_the_pages_card() {
        let frame = anchored().sample(0.25);

        // The anchor's top-left and width are the card's: the page is scaled to
        // the same width and anchored at the same corner.
        let anchor_left = corner_of(&frame.anchor, 0.0, 0.0);
        let anchor_right = corner_of(&frame.anchor, 160.0, 0.0);
        let card_left = corner_of(&frame.page, 0.0, 0.0);
        let card_right = corner_of(&frame.page, VIEWPORT.width, 0.0);
        assert!((anchor_left.0 - card_left.0).abs() < 1e-3 && (anchor_left.1 - card_left.1).abs() < 1e-3);
        assert!((anchor_right.0 - card_right.0).abs() < 1e-3, "{anchor_right:?} vs {card_right:?}");
        // The anchor is a uniform scale, so its height follows its width.
        let scale = (anchor_right.0 - anchor_left.0) / 160.0;
        let anchor_bottom = corner_of(&frame.anchor, 0.0, 160.0);
        assert!((anchor_bottom.1 - anchor_left.1 - 160.0 * scale).abs() < 1e-3);
    }

    #[test]
    fn the_handoff_dissolves_the_anchor_into_the_page() {
        let spec = anchored();
        // Default: handoff at 0.25, dissolve over 0.1, so 0.20 to 0.30.
        let opacities = |p: f32| {
            let frame = spec.sample(p);
            (frame.anchor.opacity, frame.page.opacity)
        };

        assert_eq!(opacities(0.0), (1.0, 0.0));
        assert_eq!(opacities(0.19), (1.0, 0.0));
        let (anchor, page) = opacities(0.25);
        assert!((anchor - 0.5).abs() < 1e-3);
        // The page is fully in by the middle of the dissolve, so the two never
        // add up to less than the page and nothing shows through.
        assert!((page - 1.0).abs() < 1e-3, "{page}");
        let (anchor, page) = opacities(0.30);
        assert!(anchor.abs() < 1e-3 && (page - 1.0).abs() < 1e-3);
        assert_eq!(opacities(1.0), (0.0, 1.0));
    }

    #[test]
    fn after_the_handoff_the_anchor_stays_with_the_card() {
        let spec = anchored();

        for step in 25..=100 {
            let frame = spec.sample(step as f32 / 100.0);
            let anchor = corner_of(&frame.anchor, 0.0, 0.0);
            let card = corner_of(&frame.page, 0.0, 0.0);
            assert!((anchor.0 - card.0).abs() < 1e-3 && (anchor.1 - card.1).abs() < 1e-3, "step {step}");
        }
    }

    #[test]
    fn the_anchor_grows_monotonically_during_its_phase() {
        let spec = anchored();
        let mut last = 0.0;

        for step in 0..=25 {
            let frame = spec.sample(step as f32 / 100.0);
            let width = corner_of(&frame.anchor, 160.0, 0.0).0 - corner_of(&frame.anchor, 0.0, 0.0).0;
            assert!(width >= last, "step {step}");
            last = width;
        }
        assert!(last > 160.0);
    }

    #[test]
    fn the_handoff_and_dissolve_are_configurable_and_sanitized() {
        let late = anchored().handoff(0.5).blend(0.2);
        assert_eq!(late.sample(0.39).anchor.opacity, 1.0);
        assert!((late.sample(0.5).anchor.opacity - 0.5).abs() < 1e-3);
        assert_eq!(late.sample(0.61).anchor.opacity, 0.0);

        // A zero-width dissolve is a hard swap at the handoff.
        let hard = anchored().blend(0.0);
        assert_eq!(hard.sample(0.24).anchor.opacity, 1.0);
        assert_eq!(hard.sample(0.25).anchor.opacity, 0.0);
        assert_eq!(hard.sample(0.25).page.opacity, 1.0);

        // Garbage falls back to the defaults instead of poisoning the frame.
        let garbage = anchored().handoff(f32::NAN).blend(f32::INFINITY);
        assert_eq!(garbage.sample(0.0), anchored().sample(0.0));
        assert!(garbage.sample(0.4).anchor.is_finite());
        assert!(anchored().handoff(7.0).sample(0.5).anchor.is_finite());
    }

    #[test]
    fn without_an_anchor_the_page_fades_in_by_itself_as_before() {
        let frame = spec().sample(0.125);

        assert_eq!(frame.anchor.opacity, 0.0);
        assert_eq!(frame.page.opacity, 0.5);
    }

    #[test]
    fn every_anchored_presentation_can_be_shown() {
        for step in 0..=50 {
            let frame = anchored().sample(step as f32 / 50.0);
            assert!(frame.page.is_finite() && frame.backdrop.is_finite() && frame.anchor.is_finite());
        }
    }

    mod element_curves {
        use super::*;

        /// The overshooting curve used to check that nothing becomes invalid.
        const OVERSHOOT: curves::CubicBezier = curves::cubic_bezier(0.3, 1.8, 0.6, 1.0);

        fn all_curves() -> [AnyCurve; 5] {
            [
                curves::LINEAR.into_any(),
                curves::EASE_IN.into_any(),
                curves::EASE_OUT.into_any(),
                curves::BOUNCE_OUT.into_any(),
                OVERSHOOT.into_any(),
            ]
        }

        #[test]
        fn the_default_curves_change_nothing() {
            let plain = anchored();
            let explicit = anchored().anchor_curve(curves::LINEAR).page_curve(curves::LINEAR);

            for step in 0..=20 {
                let p = step as f32 / 20.0;
                assert_eq!(plain.sample(p), explicit.sample(p));
            }
        }

        #[test]
        fn the_page_curve_reshapes_the_pages_growth_but_not_its_fade() {
            let eased = spec().page_curve(curves::EASE_IN);
            let linear = spec();

            let at_half = eased.sample(0.5);

            // The page is where a linear page would be at EaseIn(0.5) = 0.125.
            let reference = linear.sample(curves::EASE_IN.transform(0.5));
            assert_eq!(at_half.page.transform, reference.page.transform);
            assert_eq!(at_half.page.corner_radius, reference.page.corner_radius);
            // Its fade-in and the dimming behind it keep following the timeline.
            assert_eq!(at_half.page.opacity, linear.sample(0.5).page.opacity);
            assert_eq!(at_half.backdrop, linear.sample(0.5).backdrop);
        }

        #[test]
        fn the_anchor_curve_reshapes_the_copys_travel() {
            let eased = anchored().anchor_curve(curves::EASE_IN);
            let at = 0.125; // halfway through the 0 to 25% phase

            let frame = eased.sample(at);

            // Halfway through its phase, EaseIn has covered 12.5% of the way to the card.
            let card = anchored().sample(at).page.transform.transform_point(0.0, 0.0);
            let expected = (20.0 + (card.0 - 20.0) * 0.125, 120.0 + (card.1 - 120.0) * 0.125);
            let got = frame.anchor.transform.transform_point(0.0, 0.0);
            assert!((got.0 - expected.0).abs() < 1e-3 && (got.1 - expected.1).abs() < 1e-3, "{got:?} vs {expected:?}");
            // The fade is untouched.
            assert_eq!(frame.anchor.opacity, anchored().sample(at).anchor.opacity);
        }

        #[test]
        fn the_copy_still_lands_on_the_card_at_the_handoff_with_any_curves() {
            for anchor in all_curves() {
                for page in all_curves() {
                    let frame = anchored().anchor_curve(anchor.clone()).page_curve(page.clone()).sample(0.25);

                    let copy = frame.anchor.transform.transform_point(0.0, 0.0);
                    let card = frame.page.transform.transform_point(0.0, 0.0);
                    assert!((copy.0 - card.0).abs() < 1e-2 && (copy.1 - card.1).abs() < 1e-2,
                        "{anchor:?}/{page:?}: {copy:?} vs {card:?}");
                    let copy_right = frame.anchor.transform.transform_point(160.0, 0.0).0;
                    let card_right = frame.page.transform.transform_point(VIEWPORT.width, 0.0).0;
                    assert!((copy_right - card_right).abs() < 1e-2, "{anchor:?}/{page:?}");
                }
            }
        }

        #[test]
        fn the_copy_keeps_pace_with_the_card_after_the_handoff_whatever_the_curves() {
            let spec = anchored().anchor_curve(curves::EASE_IN).page_curve(OVERSHOOT);

            for step in 25..=100 {
                let frame = spec.sample(step as f32 / 100.0);
                let copy = frame.anchor.transform.transform_point(0.0, 0.0);
                let card = frame.page.transform.transform_point(0.0, 0.0);
                assert!((copy.0 - card.0).abs() < 1e-2 && (copy.1 - card.1).abs() < 1e-2, "step {step}");
            }
        }

        #[test]
        fn overshooting_curves_never_make_a_presentation_invalid() {
            // An overshoot would push the corner radius negative, and a frame with
            // a negative radius is declined by the host: the page would blink out.
            for curve in all_curves() {
                let spec = anchored().anchor_curve(curve.clone()).page_curve(curve.clone()).source_radius(12.0).page_radius(0.0);
                for step in 0..=100 {
                    let frame = spec.sample(step as f32 / 100.0);
                    assert!(frame.page.is_finite() && frame.anchor.is_finite() && frame.backdrop.is_finite(),
                        "{curve:?} at step {step}");
                    assert!(frame.page.corner_radius >= 0.0, "{curve:?} at step {step}");
                }
            }
        }

        #[test]
        fn the_ends_are_unchanged_by_curves() {
            let spec = anchored().anchor_curve(curves::EASE_OUT).page_curve(curves::EASE_IN);

            assert_eq!(spec.sample(0.0).anchor.transform, anchored().sample(0.0).anchor.transform);
            assert_eq!(spec.sample(1.0).page.transform, Mat3::identity());
            assert_eq!(spec.sample(1.0).page.corner_radius, 0.0);
        }

        #[test]
        fn an_invalid_curve_is_ignored() {
            let bad = curves::cubic_bezier(f32::NAN, 0.0, 1.0, 1.0);
            let spec = anchored().anchor_curve(bad).page_curve(bad);

            for step in 0..=20 {
                let p = step as f32 / 20.0;
                assert_eq!(spec.sample(p), anchored().sample(p));
            }
        }

        #[test]
        fn the_anchor_curve_does_nothing_without_an_anchor() {
            let eased = spec().anchor_curve(curves::EASE_IN);

            for step in 0..=10 {
                let p = step as f32 / 10.0;
                assert_eq!(eased.sample(p), spec().sample(p));
            }
        }
    }
}
