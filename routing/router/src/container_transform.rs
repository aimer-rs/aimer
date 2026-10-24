//! A page that grows out of an item and shrinks back into it, without a router.

use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Duration;

use aimer_animation::{
    AnimatedPaint, AnimationController, AnimationStatus, AnyCurve, ContainerTransformSpec, Curve,
    ElementBounds, curves, PaintPresentation, SharedContent, StageSpans, StatusListener, last_shared_element,
    last_shared_element_bounds,
};
use aimer_attribute::Dimension;
use aimer_container::SizedBox;
use aimer_space::Stack;
use aimer_widget::base::{BuildContext, ResolvedSize};
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, Key, PortableWidget, RequiredChild, State, StateUpdater,
    StatefulElement, StatefulWidget, Widget,
};

/// How a page grows out of one tagged item: shared by the router transition and
/// the standalone widget, so the two cannot drift apart.
#[derive(Clone)]
pub(crate) struct ItemMotion {
    pub tag: Key,
    /// Where the item was when the motion was resolved, for when it cannot be
    /// found any more.
    pub resolved: Option<ElementBounds>,
    /// Whether a copy of the item leads the motion. The copy was laid out at the
    /// size the item had when `resolved`.
    pub anchored: bool,
    /// Whether the page is revealed through a window rather than scaled; see
    /// [`ContainerTransformSpec::reveal`].
    pub reveal: bool,
    /// Whether the page is scaled to fit inside the item; see
    /// [`ContainerTransformSpec::fit`].
    pub fit: bool,
    /// Whether the motion plays as three stages; see
    /// [`ContainerTransformSpec::staged`].
    pub staged: bool,
    /// How much the item's copy scales in a staged motion's first stage.
    pub lift: Option<f32>,
    /// How much the stages of a staged motion overlap.
    pub stage_overlap: f32,
    /// Where each stage of a staged motion runs, if placed by hand.
    pub stage_spans: Option<StageSpans>,
    pub source_radius: f32,
    pub page_radius: f32,
    pub scrim: f32,
    /// Easing of the item copy's travel to the card; see
    /// [`ContainerTransformSpec::anchor_curve`].
    pub anchor_curve: AnyCurve,
    /// Easing of the page's growth; see [`ContainerTransformSpec::page_curve`].
    pub page_curve: AnyCurve,
}

impl ItemMotion {
    /// The geometry for a host `size` big whose top-left is at `origin` in the
    /// window, or `None` if the item is nowhere to be found.
    ///
    /// The item is looked up now rather than when the motion was resolved: a
    /// grid that scrolled, or was rebuilt after going back, has moved it since.
    pub fn spec(&self, size: ResolvedSize, origin: (f32, f32)) -> Option<ContainerTransformSpec> {
        let item = last_shared_element_bounds(&self.tag).or(self.resolved)?;
        let (width, height) = match (self.anchored, self.resolved) {
            (true, Some(resolved)) => (resolved.width, resolved.height),
            _ => (item.width, item.height),
        };
        let item = ElementBounds::new(item.x - origin.0, item.y - origin.1, width, height);
        let spec = ContainerTransformSpec::new(item, size)?
            .source_radius(self.source_radius)
            .page_radius(self.page_radius)
            .scrim(self.scrim)
            .anchored(self.anchored)
            .reveal(self.reveal)
            .fit(self.fit)
            .staged(self.staged)
            .lift(self.lift.unwrap_or(f32::NAN))
            .stage_overlap(self.stage_overlap)
            .anchor_curve(self.anchor_curve.clone())
            .page_curve(self.page_curve.clone());
        Some(match self.stage_spans {
            Some(spans) => spec.stage_spans(spans),
            None => spec,
        })
    }
}

/// The page being shown over the base, and which item it grows from.
#[derive(Clone)]
struct OpenPage {
    tag: Key,
    child: ChildBuilder,
}

/// Grows a page out of a tagged item over a base page, and shrinks it back.
///
/// The child is the base: usually a grid or list whose items are wrapped in
/// [`aimer_animation::SharedElement`]. Give [`page`](ContainerTransform::page)
/// the tag of the item that was activated and the page to show; the page grows
/// out of that item, rounded, while the base dims behind it. Passing `None`
/// again shrinks the page back into the item, and drops it once it has
/// finished. The base stays mounted throughout, so its scroll position and
/// state survive. The page should fill the area this widget occupies.
///
/// This is the same effect [`ContainerTransition`](crate::ContainerTransition)
/// gives route changes, for places that have no router.
///
/// # Example
///
/// ```
/// use aimer_router::ContainerTransform;
/// use aimer_widget::{AnyWidget, ErrorWidget, Key, Widget};
///
/// fn screen(selected: Option<u32>) -> impl Widget {
///     ContainerTransform::new()
///         .page(selected.map(|id| (Key::from(format!("album-{id}")), ErrorWidget::new("album").boxed())))
///         .child(ErrorWidget::new("album grid"))
/// }
/// ```
pub struct ContainerTransform<W = RequiredChild> {
    base: W,
    page: Option<(Key, AnyWidget)>,
    duration: Duration,
    curve: AnyCurve,
    source_radius: f32,
    page_radius: f32,
    scrim: f32,
    reveal: bool,
    staged: bool,
    lift: Option<f32>,
    stage_overlap: f32,
    stage_spans: Option<StageSpans>,
    anchor_curve: AnyCurve,
    page_curve: AnyCurve,
}

impl Default for ContainerTransform {
    fn default() -> Self {
        Self::new()
    }
}

impl ContainerTransform {
    /// Creates a 400 ms transition with [`curves::EASE_IN_OUT`] and no page open.
    ///
    /// Finish it with [`ContainerTransform::child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            base: RequiredChild,
            page: None,
            duration: Duration::from_millis(400),
            curve: curves::EASE_IN_OUT.into_any(),
            source_radius: 12.0,
            page_radius: 0.0,
            scrim: 0.35,
            reveal: false,
            staged: false,
            lift: None,
            stage_overlap: 0.0,
            stage_spans: None,
            anchor_curve: AnyCurve::default(),
            page_curve: AnyCurve::default(),
        }
    }
}

impl<W> ContainerTransform<W> {
    /// Sets the page to show: the tag of the item it grows from, and the page.
    /// `None` closes it.
    #[inline]
    pub fn page(mut self, page: Option<(Key, AnyWidget)>) -> Self {
        self.page = page;
        self
    }

    /// Sets the duration of opening and closing. Zero swaps immediately.
    #[inline]
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Sets the easing of opening and closing.
    #[inline]
    pub fn curve(mut self, curve: impl Curve) -> Self {
        self.curve = curve.into_any();
        self
    }

    /// Sets the page's corner radius while it sits on the item. The default is
    /// 12 logical pixels.
    #[inline]
    pub fn source_radius(mut self, radius: f32) -> Self {
        self.source_radius = radius;
        self
    }

    /// Sets the page's corner radius once it fills the area. The default is
    /// zero; see [`ContainerTransformSpec::page_radius`].
    #[inline]
    pub fn page_radius(mut self, radius: f32) -> Self {
        self.page_radius = radius;
        self
    }

    /// Sets how much the base fades while the page is on top, in `0.0..=1.0`.
    /// The default is 0.35.
    #[inline]
    pub fn scrim(mut self, scrim: f32) -> Self {
        self.scrim = scrim;
        self
    }

    /// Reveals the page through a window that grows out of the item, instead of
    /// scaling the page to the item's width.
    ///
    /// Use it for an item as wide as the area, such as a row of a feed, where a
    /// scaled page would only slide in. See [`ContainerTransformSpec::reveal`].
    #[inline]
    pub fn reveal(mut self, reveal: bool) -> Self {
        self.reveal = reveal;
        self
    }

    /// Plays the motion as three equal stages: the item scales, turns into the
    /// page, and the page scales to fit the area. Off by default; implies
    /// [`reveal`](Self::reveal). See [`ContainerTransformSpec::staged`].
    ///
    /// This widget has no layer to blur through, so the middle stage is a plain
    /// dissolve; a [`ContainerTransition`](crate::ContainerTransition) blurs.
    #[inline]
    pub fn staged(mut self, staged: bool) -> Self {
        self.staged = staged;
        self
    }

    /// Sets how much the item's copy scales in a [staged](Self::staged) motion's
    /// first stage. The default is `1.06`.
    #[inline]
    pub fn lift(mut self, lift: f32) -> Self {
        self.lift = Some(lift);
        self
    }

    /// Lets the stages of a [staged](Self::staged) motion run into each other,
    /// by a fraction of a stage in `0.0..=0.9`. The default is `0.0`, one stage
    /// after the other. See [`ContainerTransformSpec::stage_overlap`].
    #[inline]
    pub fn stage_overlap(mut self, overlap: f32) -> Self {
        self.stage_overlap = overlap;
        self
    }

    /// Places each stage of a [staged](Self::staged) motion on the timeline
    /// yourself, replacing [`stage_overlap`](Self::stage_overlap). See
    /// [`StageSpans`].
    #[inline]
    pub fn stage_spans(mut self, spans: StageSpans) -> Self {
        self.stage_spans = Some(spans);
        self
    }

    /// Sets the easing of the item copy's travel from its slot to the page's
    /// card, over the copy's own phase. The default is linear.
    ///
    /// This composes with [`curve`](Self::curve), so use `.curve(curves::LINEAR)`
    /// when the element curves should be the only easing. Going back plays the
    /// same curve over reversed time. See [`ContainerTransformSpec::anchor_curve`].
    #[inline]
    pub fn anchor_curve(mut self, curve: impl Curve) -> Self {
        self.anchor_curve = curve.into_any();
        self
    }

    /// Sets the easing of the page's growth (scale, position and corner radius).
    /// The default is linear. Composes with [`curve`](Self::curve) like
    /// [`anchor_curve`](Self::anchor_curve). See
    /// [`ContainerTransformSpec::page_curve`].
    #[inline]
    pub fn page_curve(mut self, curve: impl Curve) -> Self {
        self.page_curve = curve.into_any();
        self
    }

    /// Attaches the base page and completes this builder.
    #[inline]
    pub fn child<C: Widget + 'static>(self, base: C) -> ContainerTransform<C> {
        ContainerTransform {
            base,
            page: self.page,
            duration: self.duration,
            curve: self.curve,
            source_radius: self.source_radius,
            page_radius: self.page_radius,
            scrim: self.scrim,
            reveal: self.reveal,
            staged: self.staged,
            lift: self.lift,
            stage_overlap: self.stage_overlap,
            stage_spans: self.stage_spans,
            anchor_curve: self.anchor_curve,
            page_curve: self.page_curve,
        }
    }
}

impl<W: Widget + 'static> PortableWidget for ContainerTransform<W> {}

impl<W: Widget + 'static> Widget for ContainerTransform<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ContainerTransform", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "ContainerTransform"
    }
}

impl<W: Widget + 'static> StatefulWidget for ContainerTransform<W> {
    type State = ContainerTransformState<W>;

    fn create_state(self) -> Self::State {
        let controller = AnimationController::new(self.duration, self.curve.clone());
        let open = self.page.is_some();
        if open {
            // Already open when first built: show it, don't animate it in.
            controller.set_value(1.0);
        }
        ContainerTransformState {
            base: ChildBuilder::from_widget(self.base),
            page: self
                .page
                .map(|(tag, page)| OpenPage { tag, child: ChildBuilder::from_widget(page) }),
            wanted_open: open,
            controller,
            source_radius: self.source_radius,
            page_radius: self.page_radius,
            scrim: self.scrim,
            reveal: self.reveal,
            staged: self.staged,
            lift: self.lift,
            stage_overlap: self.stage_overlap,
            stage_spans: self.stage_spans,
            anchor_curve: self.anchor_curve,
            page_curve: self.page_curve,
            anchor: None,
            resolved: None,
            show_anchor: false,
            viewport: Rc::new(Cell::new(None)),
            updater: StateUpdater::empty(),
            marker: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct ContainerTransformState<W> {
    base: ChildBuilder,
    /// Kept while open and while collapsing, so it can shrink back.
    page: Option<OpenPage>,
    /// Whether the latest configuration asks for a page.
    wanted_open: bool,
    controller: AnimationController,
    source_radius: f32,
    page_radius: f32,
    scrim: f32,
    reveal: bool,
    staged: bool,
    lift: Option<f32>,
    stage_overlap: f32,
    stage_spans: Option<StageSpans>,
    anchor_curve: AnyCurve,
    page_curve: AnyCurve,
    /// A copy of the item the page grows from, when it can be drawn again.
    anchor: Option<Anchor>,
    /// Where that item was when the page was asked for.
    resolved: Option<ElementBounds>,
    /// Whether the copy is on screen: only while the motion runs.
    show_anchor: bool,
    /// The area the page fills, as last measured by the page layer, which the
    /// copy's layer (laid out at the item's size) cannot measure for itself.
    viewport: Rc<Cell<Option<ResolvedSize>>>,
    updater: StateUpdater<Self>,
    marker: PhantomData<fn() -> W>,
}

/// A copy of the item, and the size it is laid out at.
#[derive(Clone)]
struct Anchor {
    content: SharedContent,
    size: ResolvedSize,
}

impl<W> ContainerTransformState<W> {
    /// Ends a motion. A collapse that completed drops the page, unless it was
    /// asked for again meanwhile; either way the copy of the item, which only
    /// leads a motion that is running, goes.
    fn finish_motion(&mut self, status: AnimationStatus) {
        self.show_anchor = false;
        if status == AnimationStatus::Dismissed && !self.wanted_open {
            self.page = None;
        }
    }

    /// How the page grows out of the item tagged `tag`, with this state's look.
    fn motion(&self, tag: Key) -> ItemMotion {
        ItemMotion {
            tag,
            resolved: self.resolved,
            anchored: self.anchor.is_some(),
            source_radius: self.source_radius,
            page_radius: self.page_radius,
            scrim: self.scrim.clamp(0.0, 1.0),
            reveal: self.reveal,
            fit: false,
            staged: self.staged,
            lift: self.lift,
            stage_overlap: self.stage_overlap,
            stage_spans: self.stage_spans,
            anchor_curve: self.anchor_curve.clone(),
            page_curve: self.page_curve.clone(),
        }
    }

    /// Notes the item the page grows from, and whether a copy of it can lead
    /// the motion.
    fn lead_with_item(&mut self, tag: &Key) {
        let snapshot = last_shared_element(tag);
        self.resolved = snapshot.as_ref().map(|snapshot| snapshot.bounds);
        self.anchor = snapshot.and_then(|snapshot| {
            let content = snapshot.content?;
            Some(Anchor {
                content,
                size: ResolvedSize { width: snapshot.bounds.width, height: snapshot.bounds.height },
            })
        });
        self.show_anchor = self.anchor.is_some();
    }
}

/// Reacts to the end of a motion.
struct MotionListener<W: 'static> {
    updater: StateUpdater<ContainerTransformState<W>>,
}

// Safety: playback, listeners and state all live on the rendering thread. The
// bounds only satisfy `StatusListener`, which is shared with controllers that
// may be handed to other threads; this one never leaves its creator's.
unsafe impl<W> Send for MotionListener<W> {}
unsafe impl<W> Sync for MotionListener<W> {}

impl<W: 'static> StatusListener for MotionListener<W> {
    fn on_status_changed(&self, status: AnimationStatus) {
        if matches!(status, AnimationStatus::Completed | AnimationStatus::Dismissed) {
            self.updater.set_state(move |state| state.finish_motion(status));
        }
    }
}

impl<W: Widget + 'static> State<ContainerTransform<W>> for ContainerTransformState<W> {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
        self.controller
            .add_status_listener(Rc::new(MotionListener { updater }));
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.base = new.base;
        self.source_radius = new.source_radius;
        self.page_radius = new.page_radius;
        self.scrim = new.scrim;
        self.reveal = new.reveal;
        self.staged = new.staged;
        self.lift = new.lift;
        self.stage_overlap = new.stage_overlap;
        self.stage_spans = new.stage_spans;
        self.anchor_curve = new.anchor_curve;
        self.page_curve = new.page_curve;
        self.controller.set_duration(new.controller.duration());
        self.controller.set_curve(new.controller.curve());

        match (self.wanted_open, new.page) {
            (false, Some(page)) => {
                // Opening, or reopening while still collapsing: continue from
                // wherever the page currently is.
                self.lead_with_item(&page.tag);
                self.page = Some(page);
                self.wanted_open = true;
                self.controller.forward();
            }
            (true, Some(page)) => {
                let moved_to_another_item = self.page.as_ref().is_none_or(|open| open.tag != page.tag);
                if moved_to_another_item {
                    self.lead_with_item(&page.tag);
                }
                self.page = Some(page);
                if moved_to_another_item {
                    self.controller.reset();
                    self.controller.forward();
                }
            }
            (true, None) => {
                self.wanted_open = false;
                // Shrinking back into the item goes through its copy again.
                self.show_anchor = self.anchor.is_some();
                self.controller.reverse();
            }
            (false, None) => {}
        }
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let scrim = self.scrim.clamp(0.0, 1.0);
        let base = AnimatedPaint::new(self.controller.clone(), self.base.clone()).effect(move |progress| {
            // The base dims as the page comes up.
            PaintPresentation { opacity: 1.0 - scrim * progress.clamp(0.0, 1.0), ..PaintPresentation::IDENTITY_OPAQUE }
        });

        let mut layers: Vec<AnyWidget> = vec![base.boxed()];
        if let Some(page) = &self.page {
            let motion = self.motion(page.tag.clone());
            let viewport = Rc::clone(&self.viewport);
            let page_motion = motion.clone();
            layers.push(
                AnimatedPaint::new(self.controller.clone(), page.child.clone())
                    .geometry_effect(move |progress, size, origin| {
                        viewport.set(Some(size));
                        match page_motion.spec(size, origin) {
                            Some(spec) => spec.sample(progress).page,
                            // No item to grow from: fade the page in place.
                            None => PaintPresentation { opacity: progress.clamp(0.0, 1.0), ..PaintPresentation::IDENTITY_OPAQUE },
                        }
                    })
                    .boxed(),
            );
            // The copy of the item goes above everything, while the motion runs.
            if let (true, Some(anchor)) = (self.show_anchor, &self.anchor) {
                let viewport = Rc::clone(&self.viewport);
                layers.push(
                    AnimatedPaint::new(
                        self.controller.clone(),
                        SizedBox::new()
                            .width(Dimension::Px(anchor.size.width))
                            .height(Dimension::Px(anchor.size.height))
                            .child(anchor.content.build()),
                    )
                    .inert()
                    .geometry_effect(move |progress, _, origin| {
                        // The copy is as big as the item, so the area is the page's.
                        let hidden = PaintPresentation { opacity: 0.0, ..PaintPresentation::IDENTITY_OPAQUE };
                        viewport
                            .get()
                            .and_then(|size| motion.spec(size, origin))
                            .map_or(hidden, |spec| spec.sample(progress).anchor)
                    })
                    .boxed(),
                );
            }
        }
        Stack::new().children(layers)
    }
}

#[cfg(test)]
mod tests {
    use aimer_widget::ErrorWidget;

    use super::*;

    fn tag(name: &str) -> Key {
        Key::from(name)
    }

    fn page(name: &str) -> Option<(Key, AnyWidget)> {
        Some((tag(name), ErrorWidget::new("page").boxed()))
    }

    fn state(page: Option<(Key, AnyWidget)>) -> ContainerTransformState<ErrorWidget> {
        ContainerTransform::new().page(page).child(ErrorWidget::new("base")).create_state()
    }

    /// What a parent rebuild does: build the new configuration's state and
    /// hand it to the live one.
    fn rebuild(live: &mut ContainerTransformState<ErrorWidget>, page: Option<(Key, AnyWidget)>) {
        live.adopt_config_from(state(page));
    }

    #[test]
    fn it_starts_closed_and_idle() {
        let state = state(None);

        assert!(state.page.is_none());
        assert_eq!(state.controller.status(), AnimationStatus::Dismissed);
        assert_eq!(state.controller.value(), 0.0);
    }

    #[test]
    fn a_page_given_at_creation_is_shown_without_animating() {
        let state = state(page("a"));

        assert!(state.page.is_some());
        assert_eq!(state.controller.value(), 1.0);
        assert!(!state.controller.is_animating());
    }

    #[test]
    fn a_new_page_opens_by_playing_forward() {
        let mut live = state(None);

        rebuild(&mut live, page("a"));

        assert_eq!(live.page.as_ref().map(|open| open.tag.clone()), Some(tag("a")));
        assert_eq!(live.controller.status(), AnimationStatus::Forward);
    }

    #[test]
    fn closing_plays_backward_and_keeps_the_page_until_it_has_shrunk() {
        let mut live = state(page("a"));

        rebuild(&mut live, None);

        assert_eq!(live.controller.status(), AnimationStatus::Reverse);
        assert!(live.page.is_some(), "the page must stay mounted while it shrinks");
        live.finish_motion(AnimationStatus::Dismissed);
        assert!(live.page.is_none());
    }

    #[test]
    fn finishing_a_collapse_does_not_drop_a_page_that_was_asked_for_again() {
        let mut live = state(page("a"));
        rebuild(&mut live, None);

        // Reopened while collapsing, then the stale "dismissed" arrives.
        rebuild(&mut live, page("a"));
        live.finish_motion(AnimationStatus::Dismissed);

        assert!(live.page.is_some());
        assert_eq!(live.controller.status(), AnimationStatus::Forward);
    }

    #[test]
    fn rebuilding_with_the_same_page_does_not_restart_the_animation() {
        let mut live = state(page("a"));

        rebuild(&mut live, page("a"));

        assert_eq!(live.controller.value(), 1.0);
        assert!(!live.controller.is_animating());
    }

    #[test]
    fn moving_to_another_item_restarts_from_the_item() {
        let mut live = state(page("a"));

        rebuild(&mut live, page("b"));

        assert_eq!(live.page.as_ref().map(|open| open.tag.clone()), Some(tag("b")));
        assert_eq!(live.controller.value(), 0.0);
        assert_eq!(live.controller.status(), AnimationStatus::Forward);
    }

    #[test]
    fn timing_and_look_follow_the_latest_configuration() {
        let mut live = state(None);
        let next = ContainerTransform::new()
            .duration(Duration::from_millis(900))
            .curve(curves::LINEAR)
            .source_radius(30.0)
            .page_radius(5.0)
            .scrim(0.8)
            .child(ErrorWidget::new("base"))
            .create_state();

        live.adopt_config_from(next);

        assert_eq!(live.controller.duration(), Duration::from_millis(900));
        assert_eq!(live.controller.curve(), curves::LINEAR.into_any());
        assert_eq!((live.source_radius, live.page_radius, live.scrim), (30.0, 5.0, 0.8));
    }

    #[test]
    fn the_element_curves_follow_the_latest_configuration() {
        let mut live = state(None);
        let next = ContainerTransform::new()
            .anchor_curve(curves::EASE_OUT)
            .page_curve(curves::EASE_IN)
            .child(ErrorWidget::new("base"))
            .create_state();

        live.adopt_config_from(next);

        assert_eq!((live.anchor_curve.clone(), live.page_curve.clone()), (curves::EASE_OUT.into_any(), curves::EASE_IN.into_any()));
    }

    #[test]
    fn the_element_curves_reach_the_motion() {
        let mut live = state(None);
        live.adopt_config_from(
            ContainerTransform::new().anchor_curve(curves::EASE_OUT).page_curve(curves::EASE_IN).child(ErrorWidget::new("base")).create_state(),
        );

        let motion = live.motion(tag("curved"));

        assert_eq!((motion.anchor_curve, motion.page_curve), (curves::EASE_OUT.into_any(), curves::EASE_IN.into_any()));
    }

    mod anchor {
        use std::sync::OnceLock;

        use aimer_attribute::BoxConstraint;
        use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};
        use aimer_widget::base::WindowHandle;
        use aimer_widget::ErrorWidget;

        use super::*;

        fn context() -> BuildContext<'static> {
            static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
            let runtime = RUNTIME.get_or_init(|| {
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
            });
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            let mut context = BuildContext::new(
                aimer_canvas::FrameCanvas::new(inner),
                aimer_attribute::ResolvedSize { width: 100.0, height: 100.0 },
                1.0,
                Default::default(),
                Default::default(),
                WindowHandle::headless(Default::default(), 1.0),
                runtime.handle().clone(),
            );
            context.box_constraint = BoxConstraint { min_width: 0.0, min_height: 0.0, max_width: 100.0, max_height: 100.0 };
            context
        }

        /// A tile on screen at `bounds`, repeatable or not. Keep it alive.
        fn tile(tag: &str, repeatable: bool, bounds: Rect, context: &BuildContext) -> (AnyElement, RenderTree) {
            let element = if repeatable {
                aimer_animation::SharedElement::new().tag(tag).child_with(|| ErrorWidget::new("tile")).to_element(context)
            } else {
                aimer_animation::SharedElement::new().tag(tag).child(ErrorWidget::new("tile")).to_element(context)
            };
            let tree = RenderTree::new();
            let node = tree.add_root(bounds).unwrap();
            let retained = tree.context(node).unwrap();
            context.with_local_v2_paint_context(retained, |context| element.sync_local_v2_state(context));
            (element, tree)
        }

        #[test]
        fn opening_from_a_repeatable_item_leads_with_a_copy_of_it_until_the_motion_ends() {
            let context = context();
            let _tile = tile("anchored-a", true, Rect::new(10.0, 20.0, 30.0, 40.0), &context);
            let mut live = state(None);

            rebuild(&mut live, page("anchored-a"));

            let anchor = live.anchor.as_ref().expect("the item can be drawn again");
            assert_eq!((anchor.size.width, anchor.size.height), (30.0, 40.0));
            assert!(live.show_anchor);
            live.finish_motion(AnimationStatus::Completed);
            assert!(!live.show_anchor, "the copy is only needed while the motion runs");
            assert!(live.page.is_some(), "an open page stays open");
        }

        #[test]
        fn an_item_that_cannot_be_repeated_has_no_copy() {
            let context = context();
            let _tile = tile("anchored-b", false, Rect::new(10.0, 20.0, 30.0, 40.0), &context);
            let mut live = state(None);

            rebuild(&mut live, page("anchored-b"));

            assert!(live.anchor.is_none());
            assert!(!live.show_anchor);
        }

        #[test]
        fn closing_brings_the_copy_back_and_dismissal_drops_the_page() {
            let context = context();
            let _tile = tile("anchored-c", true, Rect::new(10.0, 20.0, 30.0, 40.0), &context);
            let mut live = state(None);
            rebuild(&mut live, page("anchored-c"));
            live.finish_motion(AnimationStatus::Completed);

            rebuild(&mut live, None);

            assert!(live.show_anchor, "the page shrinks into the item through its copy");
            live.finish_motion(AnimationStatus::Dismissed);
            assert!(!live.show_anchor);
            assert!(live.page.is_none());
        }

        #[test]
        fn a_completed_open_does_not_drop_the_page_and_a_stale_dismissal_does_not_either() {
            let context = context();
            let _tile = tile("anchored-d", true, Rect::new(10.0, 20.0, 30.0, 40.0), &context);
            let mut live = state(None);
            rebuild(&mut live, page("anchored-d"));

            live.finish_motion(AnimationStatus::Dismissed);

            assert!(live.page.is_some(), "the page was asked for again");
        }
    }
}
