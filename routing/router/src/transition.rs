//! Page transitions selected once per navigation and sampled without rebuilding pages.
//!
//! A Navigator has no fallback effect until configured with `.transition(...)`.
//! Manual routes override [`Route::navigation_transition`] to return
//! [`TransitionChoice::custom`], [`TransitionChoice::Disabled`], or
//! [`TransitionChoice::Inherit`]. Generated routes support the same policy:
//!
//! ```text
//! #[route("/profile/{name}")]
//! #[transition(ZoomTransition::new())]
//! #[view_level(2)]
//! Profile { name: String },
//! ```
//!
//! `#[transition(none)]` disables a route's animation. Unspecified transitions
//! inherit the host fallback, and unspecified view levels are zero. Push,
//! replacement, clear, and branch changes select the destination's policy;
//! pop selects the departing page's policy. Custom effects receive the real
//! source, destination, and action and are responsible for any reversal.
//!
//! Shell frames stay stationary: use `Shell::new(frame, builder).route(child)`
//! and configure its fallback separately. `StatefulShell::transition` handles
//! visible branch changes. Routes generated with `#[shell]` automatically
//! declare the persistent boundary; manual routes override [`Route::shell_key`].
//!
//! The first page and zero-duration effects appear immediately. Navigating
//! during an animation snaps it to completion and starts the latest change,
//! retaining at most two pages. Only the incoming page receives input and
//! focus. Portable guests keep immediate swaps.
//!
//! # Swipe back, system back and links
//!
//! A [`SlideTransition`] fallback is what makes a [`Navigator`](crate::Navigator)
//! feel like a phone: pages slide in from the right and a pop slides the
//! departing page away. Given any pop effect, the navigator also lets a finger
//! drive it: a touch that travels clearly rightward drags the page across the one
//! beneath, and lifting it past halfway, or flicking, pops. The effect is the
//! departing route's own, so [`ZoomTransition`] and [`ContainerTransition`]
//! become swipeable too, and a route that returns [`TransitionChoice::Disabled`]
//! opts out. A navigator with no effect pops instantly and leaves every touch to
//! the page.
//!
//! The swipe may begin [anywhere](crate::SwipeBackArea::Anywhere) on the page, as
//! in the iOS Files app, or only at the [edge](crate::SwipeBackArea::Edge); set it
//! with [`Navigator::swipe_back_area`](crate::Navigator::swipe_back_area). A touch
//! at the left edge always wins. Elsewhere the page gets the touch at once, so
//! taps and scrolls are untouched, and the navigator only watches it: a touch
//! that is not clearly rightward is the page's for good, and one that a child
//! took is too. A child takes a touch by claiming its pointer, which scroll views
//! and gesture detectors with a drag handler do, so a carousel that is scrolling
//! or a slider that is dragging keeps its gesture. A widget that handles pointer
//! events itself should call [`claim_pointer`](aimer_widget::claim_pointer) when
//! it starts a drag. When the swipe takes the touch the page is told it was
//! cancelled, so a button that was pressed lets go.
//!
//! Routes that share a [`Route::shell_key`] swipe too: the shell frame (a tab
//! bar, say) stays put and only the shell's outlet moves, using the effect set
//! with [`RoutedShell::transition`](crate::RoutedShell::transition) or the child route's own. The outlet, not the
//! frame around it, is where a swipe may begin, so a drag that starts on a tab bar
//! pops nothing. The navigator cannot know in advance whether the outlet has an
//! effect, so a swipe in a shell with none begins and is declined, and that touch
//! does nothing.
//!
//! A [`StatefulShell`](crate::StatefulShell) swipes back within its active
//! branch, with the effect set by [`StatefulShell::transition`](crate::StatefulShell::transition)
//! or the departing route's own. Only that branch pops; the others keep their
//! stacks. Its outlet owns the edge touch, since there is no navigator around it,
//! and the system's back pops the active branch before any enclosing navigator.
//!
//! The same gesture is driven by the system's back: the navigator answers
//! `aimer_events::back` requests (Android's Back key, and Android 14's
//! predictive back, which reports progress). It declines at its root, so the
//! platform may leave the app. Links the platform opens the app with
//! (`aimer_events::link`) are parsed with [`Route::parse`], so `jaime://topic/42`
//! and `https://example.com/topic/42` both open `/topic/42` above the first
//! page; [`NavigatorController::open_url`](crate::NavigatorController::open_url)
//! does the same on demand.
//!
//! # Custom effects
//!
//! ```
//! use std::time::Duration;
//! use aimer_router::{curves, NavigationContext, NavigationTransition,
//!     NavigationTransitionFrame, PaintPresentation, ResolvedNavigationTransition, Route};
//!
//! struct FadeIn;
//! impl<R: Route> NavigationTransition<R> for FadeIn {
//!     fn resolve(&self, _: &NavigationContext<'_, '_, R>) -> Option<ResolvedNavigationTransition> {
//!         ResolvedNavigationTransition::new(Duration::from_millis(200), curves::EASE_OUT, |p, _| {
//!             NavigationTransitionFrame {
//!                 outgoing: PaintPresentation::IDENTITY_OPAQUE,
//!                 incoming: PaintPresentation { opacity: p, ..PaintPresentation::IDENTITY_OPAQUE },
//!             }
//!         })
//!     }
//! }
//! ```

use std::rc::Rc;
use std::time::Duration;

pub use aimer_animation::{AnyCurve, Curve, PaintPresentation, curves};
use aimer_animation::{SharedContent, StageSpans, last_shared_element};
use aimer_canvas::Mat3;
use aimer_container::SizedBox;
use aimer_decoration::{BackdropFilter, DecorationBox};
use aimer_attribute::Dimension;
use aimer_widget::{AnyWidget, Key, Widget};
use aimer_widget::base::{BuildContext, ResolvedSize};

use crate::Route;
use crate::container_transform::ItemMotion;

/// The operation that changed the visible route.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NavigationAction {
    /// Append a route to a stack.
    #[default]
    Push,
    /// Return to the previous route, reversing the departing page's effect.
    Pop,
    /// Replace the stack with a route.
    Replace,
    /// Return to the initial route.
    Clear,
    /// Switch the active stateful-shell branch.
    BranchSwitch,
    /// Browser history navigation whose direction is unknown.
    History,
}

/// Effective routes and context supplied to a transition selector.
pub struct NavigationContext<'a, 'frame, R: Route> {
    /// The page being left, after redirects.
    pub from: &'a R,
    /// The destination, after redirects.
    pub to: &'a R,
    /// The operation that requested navigation.
    pub action: NavigationAction,
    /// The host's inherited state, available while resolving the effect.
    pub build_context: &'a BuildContext<'frame>,
}

/// Resolves an effect once per visible navigation.
///
/// Implementations may inspect routes and inherited state. Return `None` for
/// an immediate swap. The resolved sampler runs on the rendering thread;
/// it must be finite, allocation-free, and must not build widgets.
pub trait NavigationTransition<R: Route>: 'static {
    /// Selects an effect for these effective routes and this action.
    fn resolve(&self, navigation: &NavigationContext<'_, '_, R>) -> Option<ResolvedNavigationTransition>;
}

/// Per-route policy. An absent fallback also means an immediate swap.
pub enum TransitionChoice<R: Route> {
    /// Use the host's configured fallback.
    Inherit,
    /// Disable animation, overriding the host fallback.
    Disabled,
    /// Select an application or built-in effect.
    Custom(Rc<dyn NavigationTransition<R>>),
}

impl<R: Route> TransitionChoice<R> {
    /// Stores a transition as the route's explicit selection.
    #[inline]
    pub fn custom(transition: impl NavigationTransition<R>) -> Self {
        Self::Custom(Rc::new(transition))
    }
}

/// Presentation of the two retained pages at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavigationTransitionFrame {
    /// Presentation of the departing page, painted behind the incoming page.
    pub outgoing: PaintPresentation,
    /// Presentation of the arriving page.
    pub incoming: PaintPresentation,
}

impl NavigationTransitionFrame {
    pub(crate) const FINISHED: Self = Self {
        outgoing: PaintPresentation { transform: Mat3::identity(), opacity: 0.0, corner_radius: 0.0, reveal: None },
        incoming: PaintPresentation::IDENTITY_OPAQUE,
    };
}

/// One of the two pages of a [`NavigationTransitionFrame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionPage {
    /// The departing page.
    Outgoing,
    /// The arriving page.
    Incoming,
}

/// A copy of an item that a transition draws above both pages.
///
/// The host lays it out at `size` with its top-left at the host's origin, and
/// presents it with [`AnchoredFrame::anchor`]. The copy is inert, and the host
/// drops it when the transition ends.
#[derive(Clone)]
pub struct AnchorContent {
    content: SharedContent,
    size: ResolvedSize,
}

impl AnchorContent {
    /// Describes a copy built by `content` and laid out at `size` logical pixels.
    pub fn new(content: SharedContent, size: ResolvedSize) -> Self { Self { content, size } }
    /// The size the copy is laid out at, in logical pixels.
    pub fn size(&self) -> ResolvedSize { self.size }
    /// Builds a new copy of the item.
    pub fn build(&self) -> AnyWidget { self.content.build() }
}

/// [`NavigationTransitionFrame`] plus the presentation of an
/// [`AnchorContent`] layer above it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnchoredFrame {
    /// The two pages.
    pub frame: NavigationTransitionFrame,
    /// The copy drawn above both.
    pub anchor: PaintPresentation,
}

impl AnchoredFrame {
    const HIDDEN_ANCHOR: PaintPresentation =
        PaintPresentation { opacity: 0.0, ..PaintPresentation::IDENTITY_OPAQUE };

    pub(crate) const FINISHED: Self =
        Self { frame: NavigationTransitionFrame::FINISHED, anchor: Self::HIDDEN_ANCHOR };
}

/// Timing and a viewport-dependent sampler for one navigation.
pub struct ResolvedNavigationTransition {
    pub(crate) duration: Duration,
    pub(crate) curve: AnyCurve,
    pub(crate) outgoing_on_top: bool,
    anchor: Option<AnchorContent>,
    /// Builds a layer from eased progress that sits between the two pages.
    veil: Option<Rc<dyn Fn(f32) -> AnyWidget>>,
    /// Whether the veil is painted above everything instead of between the pages.
    veil_above: bool,
    /// The page whose presentation the veil is shown with, if it is not to fill the host.
    veil_follows: Option<TransitionPage>,
    /// Receives eased progress, the host's logical size, and the host's
    /// top-left corner in the window in logical pixels.
    sampler: Box<dyn Fn(f32, ResolvedSize, (f32, f32)) -> AnchoredFrame>,
}

impl ResolvedNavigationTransition {
    /// Creates a custom effect. Invalid easing control points return `None`.
    /// A zero duration is valid and tells the host to swap immediately.
    pub fn new(
        duration: Duration,
        curve: impl Curve,
        sampler: impl Fn(f32, ResolvedSize) -> NavigationTransitionFrame + 'static,
    ) -> Option<Self> {
        Self::positioned(duration, curve, move |progress, size, _| sampler(progress, size))
    }

    /// Creates a custom effect that also needs to know where the host is in
    /// the window.
    ///
    /// The sampler's third argument is the host's top-left corner in logical
    /// pixels from the window's top-left. Presentation transforms are relative
    /// to the host, so an effect that targets something measured in the window,
    /// such as a [`SharedElement`](aimer_animation::SharedElement), subtracts it.
    /// Plain [`sample`](Self::sample) reports `(0.0, 0.0)`.
    pub fn positioned(
        duration: Duration,
        curve: impl Curve,
        sampler: impl Fn(f32, ResolvedSize, (f32, f32)) -> NavigationTransitionFrame + 'static,
    ) -> Option<Self> {
        Self::build(duration, curve.into_any(), None, move |progress, size, origin| AnchoredFrame {
            frame: sampler(progress, size, origin),
            anchor: AnchoredFrame::HIDDEN_ANCHOR,
        })
    }

    /// Creates an effect that also draws a copy of an item above both pages.
    ///
    /// The host builds `anchor` once, lays it out at its size with its top-left
    /// at the host's origin, and presents it with the sampler's
    /// [`AnchoredFrame::anchor`] every frame. It is inert and removed when the
    /// transition ends. Otherwise this is [`positioned`](Self::positioned).
    pub fn anchored(
        duration: Duration,
        curve: impl Curve,
        anchor: AnchorContent,
        sampler: impl Fn(f32, ResolvedSize, (f32, f32)) -> AnchoredFrame + 'static,
    ) -> Option<Self> {
        Self::build(duration, curve.into_any(), Some(anchor), sampler)
    }

    fn build(
        duration: Duration,
        curve: AnyCurve,
        anchor: Option<AnchorContent>,
        sampler: impl Fn(f32, ResolvedSize, (f32, f32)) -> AnchoredFrame + 'static,
    ) -> Option<Self> {
        if !curve.is_valid() { return None; }
        Some(Self {
            duration, curve, outgoing_on_top: false, anchor, veil: None, veil_above: false,
            veil_follows: None, sampler: Box::new(sampler),
        })
    }

    /// Puts a layer between the two pages while the effect runs, such as a blur
    /// of the page underneath.
    ///
    /// `build` is called with the eased progress, `0.0` to `1.0`, on every frame
    /// and returns the layer, which fills the host. It sits above the lower page
    /// and below the upper one whichever is which, takes no input, and is
    /// dropped when the effect ends. A finger-driven swipe shows no layer, since
    /// no clock drives it.
    pub fn with_veil(mut self, build: impl Fn(f32) -> AnyWidget + 'static) -> Self {
        self.veil = Some(Rc::new(build));
        self
    }

    /// Whether the effect puts a layer between the pages; see
    /// [`with_veil`](Self::with_veil).
    pub fn has_veil(&self) -> bool { self.veil.is_some() }

    /// Like [`with_veil`](Self::with_veil), but the layer is painted above both
    /// pages and the copy of the item, so it blurs all of them.
    pub fn with_veil_above(mut self, build: impl Fn(f32) -> AnyWidget + 'static) -> Self {
        self.veil = Some(Rc::new(build));
        self.veil_above = true;
        self
    }

    /// Shows the veil with the presentation of `page` instead of filling the host.
    ///
    /// The veil is moved, clipped and rounded exactly as `page` is, at full
    /// opacity whatever the page's own is. A blur then covers only where that page
    /// is shown, such as the growing window of an
    /// [`ExpandTransition`](crate::ExpandTransition), and leaves the rest of the
    /// host sharp. A veil the effect does not point at a page fills the host.
    pub fn veil_follows(mut self, page: TransitionPage) -> Self {
        self.veil_follows = Some(page);
        self
    }

    /// The page the veil follows, if it follows one; see
    /// [`veil_follows`](Self::veil_follows).
    pub fn veil_page(&self) -> Option<TransitionPage> { self.veil_follows }

    /// Whether the veil is painted above everything rather than between the pages.
    pub(crate) fn veil_above(&self) -> bool { self.veil_above }

    /// The layer built between the pages, if the effect has one.
    pub(crate) fn veil(&self) -> Option<&Rc<dyn Fn(f32) -> AnyWidget>> { self.veil.as_ref() }

    /// The item copy this effect draws above both pages, if it has one.
    pub fn anchor(&self) -> Option<&AnchorContent> { self.anchor.as_ref() }

    /// Paints the departing page above the arriving one instead of below it.
    ///
    /// Effects where the departing page is the one that moves, such as a page
    /// shrinking back into the item it grew from, need this to stay visible.
    pub fn outgoing_on_top(mut self, on_top: bool) -> Self {
        self.outgoing_on_top = on_top;
        self
    }

    /// Whether the departing page is painted above the arriving one.
    pub fn is_outgoing_on_top(&self) -> bool { self.outgoing_on_top }

    /// Returns the configured playback duration.
    pub fn duration(&self) -> Duration { self.duration }

    /// Returns the configured easing curve.
    pub fn curve(&self) -> AnyCurve { self.curve.clone() }

    /// Samples already-eased progress, clamped to `0..=1`, for a viewport in
    /// logical pixels (physical layout dimensions divided by display scale).
    /// Returns `None` for non-finite input, invalid viewport sizes, or invalid
    /// sampled presentation, including a negative or non-finite corner radius.
    /// Opacity is clamped to `0..=1`.
    pub fn sample(&self, progress: f32, size: ResolvedSize) -> Option<NavigationTransitionFrame> {
        self.sample_at(progress, size, (0.0, 0.0))
    }

    /// Like [`sample`](Self::sample), for a host whose top-left corner is at
    /// `origin` in the window. See [`positioned`](Self::positioned).
    pub fn sample_at(&self, progress: f32, size: ResolvedSize, origin: (f32, f32)) -> Option<NavigationTransitionFrame> {
        self.sample_anchored_at(progress, size, origin).map(|sampled| sampled.frame)
    }

    /// Like [`sample_at`](Self::sample_at), and also returns the presentation
    /// of the [anchor](Self::anchor) layer.
    pub fn sample_anchored_at(&self, progress: f32, size: ResolvedSize, origin: (f32, f32)) -> Option<AnchoredFrame> {
        if !progress.is_finite() || !size.width.is_finite() || !size.height.is_finite()
            || size.width < 0.0 || size.height < 0.0
            || !origin.0.is_finite() || !origin.1.is_finite() { return None; }
        let mut sampled = (self.sampler)(progress.clamp(0.0, 1.0), size, origin);
        for presentation in [&mut sampled.frame.outgoing, &mut sampled.frame.incoming, &mut sampled.anchor] {
            if !presentation.is_finite() { return None; }
            presentation.opacity = presentation.opacity.clamp(0.0, 1.0);
        }
        Some(sampled)
    }
}

/// Cross-fades the departing and arriving pages over 300 ms by default.
#[derive(Clone, Debug)]
pub struct CrossFadeTransition {
    duration: Duration,
    curve: AnyCurve,
}

impl Default for CrossFadeTransition {
    fn default() -> Self { Self::new() }
}

impl CrossFadeTransition {
    /// Creates a 300 ms transition with [`curves::EASE_OUT`].
    #[inline]
    pub fn new() -> Self { Self { duration: Duration::from_millis(300), curve: curves::EASE_OUT.into_any() } }
    /// Sets the duration. Zero disables animation for this navigation.
    #[inline]
    pub fn duration(mut self, duration: Duration) -> Self { self.duration = duration; self }
    /// Sets easing. Non-finite Bézier control points disable this effect.
    #[inline]
    pub fn curve(mut self, curve: impl Curve) -> Self { self.curve = curve.into_any(); self }
}

impl<R: Route> NavigationTransition<R> for CrossFadeTransition {
    fn resolve(&self, _: &NavigationContext<'_, '_, R>) -> Option<ResolvedNavigationTransition> {
        ResolvedNavigationTransition::new(self.duration, self.curve.clone(), |p, _| NavigationTransitionFrame {
            outgoing: PaintPresentation { transform: Mat3::identity(), opacity: 1.0 - p, corner_radius: 0.0, reveal: None },
            incoming: PaintPresentation { transform: Mat3::identity(), opacity: p, corner_radius: 0.0, reveal: None },
        })
    }
}

/// Scales pages around the viewport center while cross-fading them.
/// Pops play the departing page's effect backwards.
#[derive(Clone, Debug)]
pub struct ZoomTransition(CrossFadeTransition);

/// Slides between explicit route levels and cross-fades at equal levels.
/// Increasing levels enter from the right; decreasing levels enter from the
/// left. This direction is independent of the stack operation.
#[derive(Clone, Debug)]
pub struct ViewLevelTransitions(CrossFadeTransition);

macro_rules! timed_transition {
    ($name:ident) => {
        impl Default for $name { fn default() -> Self { Self::new() } }
        impl $name {
            /// Creates a 300 ms transition with [`curves::EASE_OUT`].
            #[inline]
            pub fn new() -> Self { Self(CrossFadeTransition::new()) }
            /// Sets the duration. Zero performs an immediate swap.
            #[inline]
            pub fn duration(mut self, duration: Duration) -> Self { self.0.duration = duration; self }
            /// Sets easing. Non-finite Bézier control points disable the effect.
            #[inline]
            pub fn curve(mut self, curve: impl Curve) -> Self { self.0.curve = curve.into_any(); self }
        }
    };
}

timed_transition!(ZoomTransition);
timed_transition!(ViewLevelTransitions);

impl<R: Route> NavigationTransition<R> for ZoomTransition {
    fn resolve(&self, navigation: &NavigationContext<'_, '_, R>) -> Option<ResolvedNavigationTransition> {
        let reverse = navigation.action == NavigationAction::Pop;
        ResolvedNavigationTransition::new(self.0.duration, self.0.curve.clone(), move |p, size| {
            let p = if reverse { 1.0 - p } else { p };
            let centered = |scale| Mat3::translate(size.width * 0.5, size.height * 0.5)
                .mul(&Mat3::scale(scale, scale))
                .mul(&Mat3::translate(-size.width * 0.5, -size.height * 0.5));
            let outgoing = PaintPresentation { transform: centered(1.0 + 0.08 * p), opacity: 1.0 - p, corner_radius: 0.0, reveal: None };
            let incoming = PaintPresentation { transform: centered(0.92 + 0.08 * p), opacity: p, corner_radius: 0.0, reveal: None };
            if reverse { NavigationTransitionFrame { outgoing: incoming, incoming: outgoing } }
            else { NavigationTransitionFrame { outgoing, incoming } }
        })
    }
}

impl<R: Route> NavigationTransition<R> for ViewLevelTransitions {
    fn resolve(&self, navigation: &NavigationContext<'_, '_, R>) -> Option<ResolvedNavigationTransition> {
        use std::cmp::Ordering;
        let sign = match navigation.to.view_level().cmp(&navigation.from.view_level()) {
            Ordering::Equal => return self.0.resolve(navigation),
            Ordering::Greater => 1.0,
            Ordering::Less => -1.0,
        };
        ResolvedNavigationTransition::new(self.0.duration, self.0.curve.clone(), move |p, size| {
            NavigationTransitionFrame {
                outgoing: PaintPresentation { transform: Mat3::translate(-sign * p * size.width, 0.0), opacity: 1.0, corner_radius: 0.0, reveal: None },
                incoming: PaintPresentation { transform: Mat3::translate(sign * (1.0 - p) * size.width, 0.0), opacity: 1.0, corner_radius: 0.0, reveal: None },
            }
        })
    }
}

/// The mobile stack effect: a page slides in from the right edge over a page
/// that drifts a little way left, and a pop reverses it.
///
/// On a pop the departing page is the one that moves, so it is painted in front
/// and carried off to the right while the page it uncovers settles into place.
/// That is also the effect an edge swipe drives: the swipe feeds the finger's
/// travel in as progress, so the page follows the finger exactly and the release
/// plays the rest.
///
/// The page beneath moves [`PARALLAX`](Self::PARALLAX) of the distance the page
/// above does. There is no scrim or edge shadow yet; the pages are painted as
/// they are.
#[derive(Clone, Debug)]
pub struct SlideTransition(CrossFadeTransition);

impl Default for SlideTransition {
    fn default() -> Self { Self::new() }
}

impl SlideTransition {
    /// How far the page beneath travels, as a fraction of the page above's travel.
    pub const PARALLAX: f32 = 0.3;

    /// Creates a 350 ms transition with [`curves::EASE_OUT`].
    #[inline]
    pub fn new() -> Self {
        Self(CrossFadeTransition::new().duration(Duration::from_millis(350)))
    }
    /// Sets the duration. Zero performs an immediate swap.
    #[inline]
    pub fn duration(mut self, duration: Duration) -> Self { self.0.duration = duration; self }
    /// Sets easing. Non-finite Bézier control points disable the effect.
    #[inline]
    pub fn curve(mut self, curve: impl Curve) -> Self { self.0.curve = curve.into_any(); self }
}

impl<R: Route> NavigationTransition<R> for SlideTransition {
    fn resolve(&self, navigation: &NavigationContext<'_, '_, R>) -> Option<ResolvedNavigationTransition> {
        let slide = |x: f32| PaintPresentation { transform: Mat3::translate(x, 0.0), opacity: 1.0, corner_radius: 0.0, reveal: None };
        if navigation.action == NavigationAction::Pop {
            ResolvedNavigationTransition::new(self.0.duration, self.0.curve.clone(), move |p, size| {
                NavigationTransitionFrame {
                    outgoing: slide(p * size.width),
                    incoming: slide(-Self::PARALLAX * size.width * (1.0 - p)),
                }
            }).map(|effect| effect.outgoing_on_top(true))
        } else {
            ResolvedNavigationTransition::new(self.0.duration, self.0.curve.clone(), move |p, size| {
                NavigationTransitionFrame {
                    outgoing: slide(-Self::PARALLAX * size.width * p),
                    incoming: slide((1.0 - p) * size.width),
                }
            })
        }
    }
}

/// Grows the destination page out of the item that opened it.
///
/// Wrap the item in an [`aimer_animation::SharedElement`] and give this
/// transition a `tag` function that names the item a route is opened from. A
/// push starts the page scaled down on that item, with rounded corners, and
/// grows it to fill the host while the page behind dims. A pop plays it in
/// reverse, shrinking the departing page into the same item and painting it
/// above the page that returns. The item is looked up on every frame, so it
/// follows a grid that scrolls meanwhile.
///
/// When the route has no tag, or nothing is registered under it, the effect is
/// a [`ZoomTransition`].
///
/// ```
/// use aimer_router::{ContainerTransition, Route};
/// use aimer_widget::Key;
///
/// fn album_transition<R: Route>(album_tag: fn(&R) -> Option<Key>) -> ContainerTransition<R> {
///     ContainerTransition::new().tag(album_tag)
/// }
/// ```
pub struct ContainerTransition<R> {
    tag: Option<Rc<dyn Fn(&R) -> Option<Key>>>,
    timing: CrossFadeTransition,
    source_radius: f32,
    page_radius: f32,
    scrim: f32,
    reveal: bool,
    fit: bool,
    staged: bool,
    lift: Option<f32>,
    stage_overlap: f32,
    stage_spans: Option<StageSpans>,
    blur: f32,
    anchor_curve: AnyCurve,
    page_curve: AnyCurve,
}

impl<R> Default for ContainerTransition<R> {
    fn default() -> Self { Self::new() }
}

impl<R> ContainerTransition<R> {
    /// Creates a 400 ms transition with [`curves::EASE_IN_OUT`] and no tag.
    ///
    /// Without a [`tag`](Self::tag) function it only ever zooms.
    #[inline]
    pub fn new() -> Self {
        Self {
            tag: None,
            timing: CrossFadeTransition::new().duration(Duration::from_millis(400)).curve(curves::EASE_IN_OUT),
            source_radius: 12.0,
            page_radius: 0.0,
            scrim: 0.35,
            reveal: false,
            fit: false,
            staged: false,
            lift: None,
            stage_overlap: 0.0,
            stage_spans: None,
            blur: 0.0,
            anchor_curve: AnyCurve::default(),
            page_curve: AnyCurve::default(),
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
    pub fn duration(mut self, duration: Duration) -> Self { self.timing = self.timing.duration(duration); self }
    /// Sets easing. Non-finite Bézier control points disable the effect.
    #[inline]
    pub fn curve(mut self, curve: impl Curve) -> Self { self.timing = self.timing.curve(curve); self }
    /// Sets the corner radius of the page while it sits on the item, in logical
    /// pixels as shown. The default is 12.
    #[inline]
    pub fn source_radius(mut self, radius: f32) -> Self { self.source_radius = radius; self }
    /// Sets the corner radius of the page once it fills the host. The default
    /// is zero; see [`aimer_animation::ContainerTransformSpec::page_radius`].
    #[inline]
    pub fn page_radius(mut self, radius: f32) -> Self { self.page_radius = radius; self }
    /// Sets how much the page behind fades, in `0.0..=1.0`. The default is 0.35.
    #[inline]
    pub fn scrim(mut self, scrim: f32) -> Self { self.scrim = scrim; self }
    /// Reveals the page through a window that grows out of the item, instead of
    /// scaling the page to the item's width. Off by default.
    ///
    /// Use it for an item as wide as the host, such as a row of a feed: a
    /// scaled page would have nothing to shrink to and only slide in. The page
    /// keeps its size; a rounded window that starts as the item's bounds expands
    /// to fill the host. See [`aimer_animation::ContainerTransformSpec::reveal`].
    #[inline]
    pub fn reveal(mut self, reveal: bool) -> Self { self.reveal = reveal; self }
    /// Scales the page to fit inside the item, then grows it to the whole host,
    /// as an app opens from its icon. Off by default.
    ///
    /// The page is shrunk, with its proportions, until it fits the item, and
    /// centred on it. The item's copy stays in its slot and dissolves into it.
    /// Use it for an item as wide as the host, such as a row of a feed, where
    /// scaling the page to the item's width would leave nothing to scale. See
    /// [`aimer_animation::ContainerTransformSpec::fit`].
    #[inline]
    pub fn fit(mut self, fit: bool) -> Self { self.fit = fit; self }
    /// Plays the motion as three equal stages, one after the other. Off by
    /// default; implies [`reveal`](Self::reveal).
    ///
    /// 1. The item scales: a copy of it grows by [`lift`](Self::lift).
    /// 2. The item turns into the page: the copy fades out as the page fades in
    ///    on it, and with a [`blur`](Self::blur) everything goes soft through
    ///    this stage and sharp again at the end of it.
    /// 3. The page scales to fit the screen: a window onto it grows from the
    ///    item to the whole host.
    ///
    /// A pop plays them in the opposite order. [`anchor_curve`](Self::anchor_curve)
    /// eases the first stage and [`page_curve`](Self::page_curve) the last, each
    /// over its own third. See [`aimer_animation::ContainerTransformSpec::staged`].
    #[inline]
    pub fn staged(mut self, staged: bool) -> Self { self.staged = staged; self }
    /// Sets how much the item's copy scales in a [staged](Self::staged) motion's
    /// first stage, where `1.0` is not at all. The default is `1.06`.
    #[inline]
    pub fn lift(mut self, lift: f32) -> Self { self.lift = Some(lift); self }
    /// Lets the stages of a [staged](Self::staged) motion run into each other, by
    /// a fraction of a stage in `0.0..=0.9`. The default is `0.0`: each stage
    /// waits for the last, and the item stands still between the lift and the
    /// window.
    ///
    /// At `0.5` the page starts turning in halfway through the lift and the
    /// window starts opening halfway through the turn, so the motion never
    /// stops. The [`blur`](Self::blur) follows the middle stage wherever it
    /// falls. See [`aimer_animation::ContainerTransformSpec::stage_overlap`].
    #[inline]
    pub fn stage_overlap(mut self, overlap: f32) -> Self { self.stage_overlap = overlap; self }
    /// Places each stage of a [staged](Self::staged) motion on the timeline
    /// yourself, replacing [`stage_overlap`](Self::stage_overlap): a long lift, a
    /// short turn, a late window, whatever the motion wants. The
    /// [`blur`](Self::blur) swells through the turn's span, and
    /// [`StageSpans::dimmed_over`] limits the dimming of the page behind to a
    /// span of its own. See [`aimer_animation::StageSpans`].
    #[inline]
    pub fn stage_spans(mut self, spans: StageSpans) -> Self { self.stage_spans = Some(spans); self }
    /// Blurs the page behind while the page opens, up to `radius` logical pixels.
    /// The default is `0.0`, no blur.
    ///
    /// The page behind goes from sharp to `radius` as the page comes up, so it
    /// reads as dissolving into the new page rather than being cut away; going
    /// back brings it into focus again. The blur leads the page, easing out so
    /// that it is most of the way there when the page has come up a third, since
    /// a page that grows fast covers most of what is behind it early. The blur is
    /// clamped to the renderer's maximum, 96 pixels. It sits on top of the
    /// dimming set by [`scrim`](Self::scrim).
    #[inline]
    pub fn blur(mut self, radius: f32) -> Self {
        self.blur = if radius.is_finite() { radius.max(0.0) } else { 0.0 };
        self
    }
    /// Sets the easing of the item copy's travel from its slot to the page's
    /// card, over the copy's own phase. The default is linear. It composes with
    /// [`curve`](Self::curve): use `.curve(curves::LINEAR)` when the element
    /// curves should be the only easing. A pop plays it over reversed time. An
    /// invalid curve is ignored. See [`aimer_animation::ContainerTransformSpec::anchor_curve`].
    #[inline]
    pub fn anchor_curve(mut self, curve: impl Curve) -> Self { self.anchor_curve = curve.into_any(); self }
    /// Sets the easing of the page's growth: its scale, position and corner
    /// radius. The default is linear. Composes with [`curve`](Self::curve) like
    /// [`anchor_curve`](Self::anchor_curve). See [`aimer_animation::ContainerTransformSpec::page_curve`].
    #[inline]
    pub fn page_curve(mut self, curve: impl Curve) -> Self { self.page_curve = curve.into_any(); self }
}

impl<R: Route> NavigationTransition<R> for ContainerTransition<R> {
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
            // Nothing to grow from, so zoom, with the same blur. Say why: a plain
            // zoom where the item was expected means the item was not on screen
            // or not registered when the navigation was resolved.
            match &tag {
                Some(tag) => aimer_utils::warn!(
                    "container transition: nothing is registered under {tag:?} ({owner}), falling back to a zoom",
                    owner = if reverse { "going back" } else { "opening" },
                ),
                None => aimer_utils::debug!("container transition: the route has no item, zooming"),
            }
            return ZoomTransition(self.timing.clone()).resolve(navigation)
                .map(|effect| self.veiled(effect, reverse));
        };
        let resolved = snapshot.bounds;
        let motion = ItemMotion {
            tag,
            resolved: Some(resolved),
            anchored: snapshot.content.is_some(),
            source_radius: self.source_radius,
            page_radius: self.page_radius,
            scrim: self.scrim,
            reveal: self.reveal,
            fit: self.fit,
            staged: self.staged,
            lift: self.lift,
            stage_overlap: self.stage_overlap,
            stage_spans: self.stage_spans,
            anchor_curve: self.anchor_curve.clone(),
            page_curve: self.page_curve.clone(),
        };
        let sample = move |p: f32, size: ResolvedSize, origin: (f32, f32)| {
            let Some(spec) = motion.spec(size, origin) else { return AnchoredFrame::FINISHED };
            let frame = spec.sample(if reverse { 1.0 - p } else { p });
            // A pop plays the expansion backwards: the page that grew is now the
            // departing one, and the page behind it is the one returning.
            let pages = if reverse { NavigationTransitionFrame { outgoing: frame.page, incoming: frame.backdrop } }
                else { NavigationTransitionFrame { outgoing: frame.backdrop, incoming: frame.page } };
            AnchoredFrame { frame: pages, anchor: frame.anchor }
        };
        let effect = match snapshot.content {
            Some(content) => ResolvedNavigationTransition::anchored(
                self.timing.duration, self.timing.curve.clone(),
                AnchorContent::new(content, ResolvedSize { width: resolved.width, height: resolved.height }),
                sample),
            None => ResolvedNavigationTransition::positioned(
                self.timing.duration, self.timing.curve.clone(),
                move |p, size, origin| sample(p, size, origin).frame),
        };
        effect.map(|effect| self.veiled(effect.outgoing_on_top(reverse), reverse))
    }
}

impl<R> ContainerTransition<R> {
    /// `effect` with the blur of the page behind, if one was asked for.
    fn veiled(&self, effect: ResolvedNavigationTransition, reverse: bool) -> ResolvedNavigationTransition {
        let blur = self.blur;
        if blur <= 0.0 { return effect; }
        if self.staged {
            let (overlap, spans) = (self.stage_overlap, self.stage_spans);
            // Over everything, so the copy of the item and the page, which cross
            // over in the middle stage, are both in it.
            return effect.with_veil_above(move |progress| {
                let expansion = if reverse { 1.0 - progress } else { progress };
                blur_veil(blur * match spans {
                    Some(spans) => spans.blur(expansion),
                    None => aimer_animation::ContainerTransformSpec::staged_blur_with(expansion, overlap),
                })
            });
        }
        // The blur follows how far the page has come up, which a pop plays
        // backwards. It leads the page: a page that grows quickly covers most of
        // what is behind it within the first few frames, so a blur that only
        // arrived with the page would be gone before it could be seen.
        effect.with_veil(move |progress| {
            let expansion = if reverse { 1.0 - progress } else { progress }.clamp(0.0, 1.0);
            blur_veil(blur * (1.0 - (1.0 - expansion).powi(3)))
        })
    }
}

/// A layer that blurs whatever is painted beneath it by `radius`.
pub(crate) fn blur_veil(radius: f32) -> AnyWidget {
    DecorationBox::new()
        .decoration(BackdropFilter::new().blur_radius(radius))
        .child(SizedBox::new().width(Dimension::Percent(100.0)).height(Dimension::Percent(100.0)))
        .boxed()
}

/// Selects an effect at runtime from the routes, action, and inherited state.
/// The callback runs once per visible navigation, never per animation tick.
pub struct DynamicTransitions<F> { selector: F }

impl<F> DynamicTransitions<F> {
    /// Creates a runtime selector. Return `None` to swap immediately.
    #[inline]
    pub fn new(selector: F) -> Self { Self { selector } }
}

impl<R, F> NavigationTransition<R> for DynamicTransitions<F>
where
    R: Route,
    F: Fn(&NavigationContext<'_, '_, R>) -> Option<ResolvedNavigationTransition> + 'static,
{
    fn resolve(&self, navigation: &NavigationContext<'_, '_, R>) -> Option<ResolvedNavigationTransition> {
        (self.selector)(navigation)
    }
}
