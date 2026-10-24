//! A page opening out of an item by the item's window growing to fill the host
//! while the item dissolves into the page inside it.

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use aimer_animation::{
    AnyCurve, Curve, ElementBounds, ExpandSpec, curves, last_shared_element, last_shared_element_bounds,
};
use aimer_widget::Key;
use aimer_widget::base::ResolvedSize;

use crate::Route;
use crate::transition::{
    AnchorContent, AnchoredFrame, NavigationAction, NavigationContext, NavigationTransition,
    NavigationTransitionFrame, ResolvedNavigationTransition, TransitionPage, ZoomTransition,
    blur_veil,
};

/// Opens the destination page out of the item that opened it: the item's window
/// grows to fill the host, and while it grows the item dissolves into the page.
///
/// Wrap the item in an [`aimer_animation::SharedElement`] (with
/// [`child_with`](aimer_animation::SharedElement::child_with), so that the
/// transition can draw a copy of it) and give this transition a `tag` function
/// that names the item a route is opened from. A push starts with a window on
/// the item, with the item's corners, and grows it to the whole host. A pop plays
/// the same motion backwards, shrinking the departing page into the item and
/// painting it above the page that returns.
///
/// A copy of the item is stretched to the window as it grows, so that it is the
/// item itself that expands, and dissolves into the page inside it. The copy's
/// contents stretch with it, so it is best dissolved while it is still close to
/// the item's shape, or behind a [`blur`](Self::blur), which blurs the window
/// and nothing outside it.
///
/// The window and the dissolve each run on a span of the timeline of their own
/// ([`grow`](Self::grow) and [`dissolve`](Self::dissolve)), so they overlap as
/// much as you like: the item can start to go while the window is still
/// opening, and be gone before it has finished. The page is opaque beneath the
/// item's copy from the first frame, so the page behind never shows through the
/// window, and it darkens in step with the window covering it
/// ([`scrim`](Self::scrim)). The page is revealed rather than scaled, so an item
/// as wide as the host, such as a row of a feed, has something to grow into; with
/// [`stretch`](Self::stretch) it is scaled to fill the window instead, as the item's
/// copy is.
/// Unlike [`ContainerTransition`](crate::ContainerTransition), nothing here
/// waits for anything.
///
/// The timeline is linear by default, so that the curves of the window and of
/// the dissolve are the only easing; see [`curve`](Self::curve).
///
/// When the route has no tag, or nothing is registered under it, the effect is
/// a [`ZoomTransition`].
///
/// ```
/// use aimer_router::{ExpandTransition, Route};
/// use aimer_widget::Key;
///
/// fn post_transition<R: Route>(post_tag: fn(&R) -> Option<Key>) -> ExpandTransition<R> {
///     // The window opens over the whole transition; the card dissolves into the
///     // post from a fifth of the way to seven tenths.
///     ExpandTransition::new().tag(post_tag).dissolve(0.2..0.7)
/// }
/// ```
pub struct ExpandTransition<R> {
    tag: Option<Rc<dyn Fn(&R) -> Option<Key>>>,
    duration: Duration,
    curve: AnyCurve,
    /// The motion, described once; it is pointed at the item's geometry on every
    /// frame with [`ExpandSpec::retargeted`].
    motion: ExpandSpec,
    blur: f32,
}

impl<R> Default for ExpandTransition<R> {
    fn default() -> Self { Self::new() }
}

impl<R> ExpandTransition<R> {
    /// Creates a 450 ms transition on a linear timeline, with no tag.
    ///
    /// Without a [`tag`](Self::tag) function it only ever zooms.
    #[inline]
    pub fn new() -> Self {
        // The geometry is a placeholder: it is replaced by the item's on every
        // frame, and only the settings are kept.
        let motion = ExpandSpec::new(ElementBounds::new(0.0, 0.0, 1.0, 1.0), ResolvedSize { width: 1.0, height: 1.0 })
            .expect("a unit item in a unit host has an area");
        Self {
            tag: None,
            duration: Duration::from_millis(450),
            curve: curves::LINEAR.into_any(),
            motion,
            blur: 0.0,
        }
    }
    /// Names the item a route is opened from, or `None` for a route that has no
    /// item. A push asks about the destination, a pop about the departing page.
    #[inline]
    pub fn tag(mut self, tag: impl Fn(&R) -> Option<Key> + 'static) -> Self {
        self.tag = Some(Rc::new(tag));
        self
    }
    /// Sets the duration. Zero performs an immediate swap.
    #[inline]
    pub fn duration(mut self, duration: Duration) -> Self { self.duration = duration; self }
    /// Sets the easing of the whole timeline, which the window's and the
    /// dissolve's own curves are applied on top of. The default is linear, so
    /// that those are the only easing. An invalid curve (a non-finite Bézier
    /// control point) disables the effect.
    #[inline]
    pub fn curve(mut self, curve: impl Curve) -> Self { self.curve = curve.into_any(); self }
    /// Sets the corner radius of the window while it sits on the item. The
    /// default is 12 logical pixels. See [`ExpandSpec::source_radius`].
    #[inline]
    pub fn source_radius(mut self, radius: f32) -> Self { self.motion = self.motion.source_radius(radius); self }
    /// Sets the corner radius of the window once it fills the host. The default
    /// is zero. See [`ExpandSpec::page_radius`].
    #[inline]
    pub fn page_radius(mut self, radius: f32) -> Self { self.motion = self.motion.page_radius(radius); self }
    /// Sets how dark the page behind is when the window has filled the host, in
    /// `0.0..=1.0`. The default is `0.35`. It darkens along the window's growth.
    /// See [`ExpandSpec::scrim`].
    #[inline]
    pub fn scrim(mut self, scrim: f32) -> Self { self.motion = self.motion.scrim(scrim); self }
    /// Places the window's growth on the timeline, as fractions in `0.0..=1.0`.
    /// The default is all of it. See [`ExpandSpec::grow`].
    #[inline]
    pub fn grow(mut self, span: Range<f32>) -> Self { self.motion = self.motion.grow(span); self }
    /// Places the item's dissolve into the page on the timeline, as fractions in
    /// `0.0..=1.0`. The default is `0.2..0.7`. See [`ExpandSpec::dissolve`].
    #[inline]
    pub fn dissolve(mut self, span: Range<f32>) -> Self { self.motion = self.motion.dissolve(span); self }
    /// Has the page fade in as the item fades out, instead of lying opaque beneath
    /// it. Off by default. The page then rises on the dissolve's curve exactly as
    /// far as the item falls, so a little of the page behind shows through the
    /// window at the crossing; it is dimmed, and a [`blur`](Self::blur) hides most
    /// of it. See [`ExpandSpec::crossfade`].
    #[inline]
    pub fn crossfade(mut self, crossfade: bool) -> Self { self.motion = self.motion.crossfade(crossfade); self }
    /// Has the page scaled to fill the window, as the copy of the item is, instead
    /// of revealed through it. Off by default.
    ///
    /// The whole page is shown at the window's size and moved to its corner, so
    /// the page is squeezed into the item's shape and stretches back to its own as
    /// the window grows. It is only presented that way: the page is laid out for
    /// the host throughout, so nothing about its layout changes. See
    /// [`ExpandSpec::stretch`].
    #[inline]
    pub fn stretch(mut self, stretch: bool) -> Self { self.motion = self.motion.stretch(stretch); self }
    /// Sets the easing of the window's growth over its span. The default is
    /// linear. See [`ExpandSpec::grow_curve`].
    #[inline]
    pub fn grow_curve(mut self, curve: impl Curve) -> Self { self.motion = self.motion.grow_curve(curve); self }
    /// Sets the easing of the item's dissolve over its span. The default is
    /// linear. See [`ExpandSpec::dissolve_curve`].
    #[inline]
    pub fn dissolve_curve(mut self, curve: impl Curve) -> Self { self.motion = self.motion.dissolve_curve(curve); self }
    /// Blurs the window while the item dissolves into the page, up to `radius`
    /// logical pixels. The default is `0.0`, no blur.
    ///
    /// The blur swells through the [dissolve](Self::dissolve), sharp before it
    /// and after, over the item's copy and the page inside the window. It is
    /// clipped to the window and its corners as it grows, so the rest of the
    /// screen stays sharp. It is clamped to the renderer's maximum, 96 pixels.
    #[inline]
    pub fn blur(mut self, radius: f32) -> Self {
        self.blur = if radius.is_finite() { radius.max(0.0) } else { 0.0 };
        self
    }
    /// Holds the blur at its peak through the middle of the dissolve, by
    /// `fraction` of it in `0.0..=0.95`, instead of a single swell. See
    /// [`ExpandSpec::blur_held`].
    #[inline]
    pub fn blur_held(mut self, fraction: f32) -> Self { self.motion = self.motion.blur_held(fraction); self }

    /// The geometry for a host `size` big whose top-left is at `origin` in the
    /// window, or `None` if the item is nowhere to be found.
    ///
    /// The item is looked up now rather than when the motion was resolved: a
    /// grid that scrolled, or was rebuilt after going back, has moved it since.
    fn spec_for(
        motion: &ExpandSpec,
        tag: &Key,
        resolved: ElementBounds,
        anchored: bool,
        size: ResolvedSize,
        origin: (f32, f32),
    ) -> Option<ExpandSpec> {
        let item = last_shared_element_bounds(tag).unwrap_or(resolved);
        // A copy of the item is laid out at the size the item had when the
        // motion was resolved, wherever the item is now.
        let (width, height) = if anchored { (resolved.width, resolved.height) } else { (item.width, item.height) };
        let item = ElementBounds::new(item.x - origin.0, item.y - origin.1, width, height);
        Some(motion.retargeted(item, size)?.anchored(anchored))
    }

    /// `effect` with the blur over the window, if one was asked for.
    fn veiled(&self, effect: ResolvedNavigationTransition, reverse: bool) -> ResolvedNavigationTransition {
        let blur = self.blur;
        if blur <= 0.0 { return effect; }
        let motion = self.motion.clone();
        // Over everything, so the copy of the item and the page, which cross over
        // in the dissolve, are both in it; and shown as the page is, so that it
        // covers the window they are in and nothing else. The page grows on a
        // push and is the departing one on a pop.
        effect
            .with_veil_above(move |progress| {
                let expansion = if reverse { 1.0 - progress } else { progress };
                blur_veil(blur * motion.blur(expansion))
            })
            .veil_follows(if reverse { TransitionPage::Outgoing } else { TransitionPage::Incoming })
    }

    /// A zoom with the blur, for a route with nothing to grow from.
    fn zoomed(&self, effect: ResolvedNavigationTransition, reverse: bool) -> ResolvedNavigationTransition {
        let blur = self.blur;
        if blur <= 0.0 { return effect; }
        // The blur leads the page, easing out: a page that grows quickly covers
        // most of what is behind it within the first few frames.
        effect.with_veil(move |progress| {
            let expansion = if reverse { 1.0 - progress } else { progress }.clamp(0.0, 1.0);
            blur_veil(blur * (1.0 - (1.0 - expansion).powi(3)))
        })
    }
}

impl<R: Route> NavigationTransition<R> for ExpandTransition<R> {
    fn resolve(&self, navigation: &NavigationContext<'_, '_, R>) -> Option<ResolvedNavigationTransition> {
        let reverse = navigation.action == NavigationAction::Pop;
        // Same owner rule as the route's own policy: the destination opens an
        // item, and the departing page is what closes back into it.
        let owner = if reverse { navigation.from } else { navigation.to };
        let tag = self.tag.as_ref().and_then(|tag| tag(owner));
        // The item may not be measurable yet when going back, but it must be
        // known now for the effect to be this one rather than a zoom.
        let snapshot = tag.as_ref().and_then(last_shared_element);
        let (Some(tag), Some(snapshot)) = (tag.clone(), snapshot) else {
            match &tag {
                Some(tag) => aimer_utils::warn!(
                    "expand transition: nothing is registered under {tag:?} ({owner}), falling back to a zoom",
                    owner = if reverse { "going back" } else { "opening" },
                ),
                None => aimer_utils::debug!("expand transition: the route has no item, zooming"),
            }
            return ZoomTransition::new()
                .duration(self.duration)
                .curve(self.curve.clone())
                .resolve(navigation)
                .map(|effect| self.zoomed(effect, reverse));
        };
        let resolved = snapshot.bounds;
        let anchored = snapshot.content.is_some();
        let motion = self.motion.clone();
        let sample = move |p: f32, size: ResolvedSize, origin: (f32, f32)| {
            let Some(spec) = Self::spec_for(&motion, &tag, resolved, anchored, size, origin) else {
                return AnchoredFrame::FINISHED;
            };
            let frame = spec.sample(if reverse { 1.0 - p } else { p });
            // A pop plays the expansion backwards: the page that grew is now the
            // departing one, and the page behind it is the one returning.
            let pages = if reverse { NavigationTransitionFrame { outgoing: frame.page, incoming: frame.backdrop } }
                else { NavigationTransitionFrame { outgoing: frame.backdrop, incoming: frame.page } };
            AnchoredFrame { frame: pages, anchor: frame.anchor }
        };
        let effect = match snapshot.content {
            Some(content) => ResolvedNavigationTransition::anchored(
                self.duration, self.curve.clone(),
                AnchorContent::new(content, ResolvedSize { width: resolved.width, height: resolved.height }),
                sample),
            None => ResolvedNavigationTransition::positioned(
                self.duration, self.curve.clone(),
                move |p, size, origin| sample(p, size, origin).frame),
        };
        effect.map(|effect| self.veiled(effect.outgoing_on_top(reverse), reverse))
    }
}
