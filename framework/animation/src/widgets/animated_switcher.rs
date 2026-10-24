use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::panic::Location;
use std::time::Duration;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_widget::base::*;
use aimer_widget::{
    AnyElement, ChildBuilder, Drawable, Element, EventElement, EventResult, Key, LayoutElement,
    Rebuildable, State, StateUpdater, StatefulElement, StatefulWidget, StatelessElement,
    VisitorElement, Widget,
};

use crate::control::controller::AnimationController;
use crate::primitives::curve::{AnyCurve, Curve};
use crate::primitives::time::AnimInstant;

fn request_next_frame() {
    aimer_events::window::request_animation_frame();
}

/// A widget that cross-fades between its old and new child when the child
/// changes.
///
/// When the `child` field is updated (via rebuild), the switcher fades out the
/// old child and fades in the new one over the specified `duration`.
/// Child widgets should provide distinct keys; use
/// [`AnimatedSwitcher::child_key`] when the child type does not expose one
/// itself. The first child appears immediately. Rebuilding with the same child
/// identity updates the child without starting a transition.
///
/// # Example
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::{AnimatedSwitcher, curves};
/// use aimer_widget::ErrorWidget;
///
/// let switcher =
///     AnimatedSwitcher::new(Duration::from_millis(300),
///                           curves::LINEAR,
///                           ErrorWidget::new("Current page")).child_key("current-page");
/// ```
pub struct AnimatedSwitcher<T: Widget + 'static> {
    /// The subtree to show, kept as a builder because a cross-fade rebuilds it
    /// once per frame, for as long as the fade lasts.
    pub child: ChildBuilder,
    pub duration: Duration,
    pub curve: AnyCurve,
    /// Optional separate curve for the outgoing child. Defaults to `curve`.
    pub switch_out_curve: Option<AnyCurve>,
    transition_key: Option<Key>,
    widget_key: Option<Key>,
    /// The child's type, which the builder erases.
    ///
    /// The switcher's state is bound to it, so switchers that show different
    /// kinds of child stay distinct types and never reconcile onto each
    /// other's state.
    marker: PhantomData<T>,
}

impl<T: Widget + 'static> AnimatedSwitcher<T> {
    /// Creates a switcher with the incoming `curve` and transition `duration`.
    ///
    /// The outgoing child uses the same curve unless
    /// [`with_switch_out_curve`](Self::with_switch_out_curve) is called. The
    /// child's own [`Widget::key`] is used as its identity by default; the
    /// builder the child is stored in reports that key as its own.
    #[inline]
    pub fn new(duration: Duration, curve: impl Curve, child: T) -> Self {
        Self {
            child: ChildBuilder::from_widget(child),
            duration,
            curve: curve.into_any(),
            switch_out_curve: None,
            transition_key: None,
            widget_key: None,
            marker: PhantomData,
        }
    }

    /// Uses a separate curve for fading out the outgoing child.
    ///
    /// If omitted, the incoming [`curve`](Self::curve) is used for both
    /// directions.
    #[inline]
    pub fn with_switch_out_curve(mut self, curve: impl Curve) -> Self {
        self.switch_out_curve = Some(curve.into_any());
        self
    }

    /// Sets the child identity used to decide whether a transition is needed.
    /// The identity is also attached to the rendered child slot, which keeps
    /// the incoming and outgoing children distinct while both are present.
    /// This is useful when the child widget itself does not expose a key.
    #[inline]
    #[track_caller]
    pub fn child_key(mut self, key: impl Into<Key>) -> Self {
        let caller = Location::caller();
        self.transition_key = Some(key.into().with_location(caller));
        self
    }

    /// Sets the identity of the switcher itself for widget reconciliation.
    ///
    /// This is independent of [`child_key`](Self::child_key), which controls
    /// whether the children cross-fade.
    #[inline]
    #[track_caller]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        let caller = Location::caller();
        self.widget_key = Some(key.into().with_location(caller));
        self
    }
}

impl<T: Widget + 'static> StatefulWidget for AnimatedSwitcher<T> {
    type State = AnimatedSwitcherState<T>;

    fn create_state(self) -> Self::State {
        let in_controller = AnimationController::new(self.duration, self.curve.clone());
        in_controller.set_value(1.0);
        let switch_out_curve = self.switch_out_curve.unwrap_or_else(|| self.curve.clone());
        AnimatedSwitcherState {
            current_child: self.child.clone(),
            old_child: None,
            old_child_key: None,
            child_key: self.transition_key.clone().or_else(|| self.child.key()),
            duration: self.duration,
            curve: self.curve,
            switch_out_curve: switch_out_curve.clone(),
            in_controller,
            out_controller: AnimationController::new(self.duration, switch_out_curve),
            updater: StateUpdater::empty(),
            marker: PhantomData,
        }
    }
}

impl<T: Widget + 'static> Widget for AnimatedSwitcher<T> {
    fn key(&self) -> Option<Key> {
        self.widget_key.clone()
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let __key = Widget::key(&self);
        StatefulElement::new_with_name(self, ctx, "AnimatedSwitcher", __key)
            .0
            .boxed()
    }
}

impl<T: Widget + 'static> aimer_widget::PortableWidget for AnimatedSwitcher<T> {
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        ctx: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<aimer_widget::portable::PortableNodeId, aimer_widget::portable::PortableBuildError>
    {
        self.child.into_portable_node(ctx, source.child(0))
    }
}

#[doc(hidden)]
pub struct AnimatedSwitcherState<T: Widget + 'static> {
    current_child: ChildBuilder,
    old_child: Option<ChildBuilder>,
    old_child_key: Option<Key>,
    child_key: Option<Key>,
    duration: Duration,
    curve: AnyCurve,
    switch_out_curve: AnyCurve,
    in_controller: AnimationController,
    out_controller: AnimationController,
    updater: StateUpdater<Self>,
    marker: PhantomData<T>,
}

impl<T: Widget + 'static> State<AnimatedSwitcher<T>> for AnimatedSwitcherState<T> {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.duration = new.duration;
        self.curve = new.curve;
        self.switch_out_curve = new.switch_out_curve;
        self.in_controller.set_duration(self.duration);
        self.in_controller.set_curve(self.curve.clone());
        self.out_controller.set_duration(self.duration);
        self.out_controller.set_curve(self.switch_out_curve.clone());

        if self.child_key != new.child_key {
            self.old_child = Some(self.current_child.clone());
            self.old_child_key = self.child_key.clone();
            self.current_child = new.current_child;
            self.child_key = new.child_key;
            self.in_controller.reset();
            self.out_controller.reset();
            self.in_controller.forward_from_first_tick();
            self.out_controller.forward_from_first_tick();
            request_next_frame();
        } else {
            self.current_child = new.current_child;
        }
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        AnimatedSwitcherFrame {
            current_child: self.current_child.clone(),
            current_child_key: self.child_key.clone(),
            old_child: if self.out_controller.is_animating() {
                self.old_child.clone()
            } else {
                None
            },
            old_child_key: if self.out_controller.is_animating() {
                self.old_child_key.clone()
            } else {
                None
            },
            in_controller: self.in_controller.clone(),
            out_controller: self.out_controller.clone(),
        }
    }
}

struct AnimatedSwitcherFrame {
    current_child: ChildBuilder,
    current_child_key: Option<Key>,
    old_child: Option<ChildBuilder>,
    old_child_key: Option<Key>,
    in_controller: AnimationController,
    out_controller: AnimationController,
}

impl Widget for AnimatedSwitcherFrame {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let Self {
            current_child,
            current_child_key,
            old_child,
            old_child_key,
            in_controller,
            out_controller,
        } = self;

        AnimatedSwitcherElement {
            current_child: keyed_child_element(current_child, current_child_key, ctx),
            old_child: UnsafeCell::new(
                old_child.map(|child| keyed_child_element(child, old_child_key, ctx)),
            ),
            in_controller,
            out_controller,
        }
        .boxed()
    }
}

fn keyed_child_element(child: ChildBuilder, key: Option<Key>, ctx: &BuildContext) -> AnyElement {
    let child = child.build(ctx);
    // The switcher's current and outgoing children are siblings during a
    // transition. Give those slots the same identity used by the transition
    // controller so reconciliation cannot match them positionally.
    match key {
        Some(key) => StatelessElement::wrapper(child, Some(key), "AnimatedSwitcherChild").boxed(),
        None => child,
    }
}

impl aimer_widget::PortableWidget for AnimatedSwitcherFrame {}

struct AnimatedSwitcherElement {
    current_child: AnyElement,
    old_child: UnsafeCell<Option<AnyElement>>,
    in_controller: AnimationController,
    out_controller: AnimationController,
}

unsafe impl Send for AnimatedSwitcherElement {}
unsafe impl Sync for AnimatedSwitcherElement {}

impl Drawable for AnimatedSwitcherElement {
    fn update(&self, ctx: &BuildContext) {
        // The crossfade itself is the children's presentation opacity, applied
        // by `sync_local_v2_state`; this only advances the controllers.
        let now = AnimInstant::now();
        self.in_controller.tick(now);
        self.out_controller.tick(now);
        self.draw_local_v2_compatibility(ctx);
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        // The crossfade is each child node's presentation opacity, so the tree
        // damages whatever the child paints, inside its layout box or not. It
        // needs no promise about the children's paint extent.
        let now = AnimInstant::now();
        let in_value = self.in_controller.tick(now);
        let out_value = self.out_controller.tick(now);
        if !in_value.is_finite() || !out_value.is_finite() {
            return false;
        }

        let has_old_child = unsafe { (&*self.old_child.get()).is_some() };
        if has_old_child
            && !ctx.set_local_v2_child_presentation_at(
                0,
                aimer_canvas::Mat3::identity(),
                (1.0 - out_value).clamp(0.0, 1.0),
            )
        {
            return false;
        }
        ctx.set_local_v2_child_presentation_at(
            usize::from(has_old_child),
            aimer_canvas::Mat3::identity(),
            in_value.clamp(0.0, 1.0),
        )
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        if let Some(old) = unsafe { &*self.old_child.get() } {
            old.update(ctx);
        }
        self.current_child.update(ctx);

        let active = self.in_controller.is_animating() || self.out_controller.is_animating();
        if active {
            request_next_frame();
        } else if self.out_controller.value() >= 1.0
            && unsafe { (&*self.old_child.get()).is_some() }
        {
            unsafe { *self.old_child.get() = None };
            // The render tree still holds the outgoing child's nodes: tell it
            // the structure changed and give it a frame to resync, or the old
            // picture stays painted behind the new one.
            aimer_widget::notify_hosted_element_tree_changed();
            request_next_frame();
        }
    }
}

impl VisitorElement for AnimatedSwitcherElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.current_child.as_ref());
        if let Some(old) = unsafe { &*self.old_child.get() } {
            visitor(old.as_ref());
        }
    }

    fn visit_retained_v2_children<'a>(&'a self, visitor: &mut dyn FnMut(usize, &'a dyn Element)) {
        let mut index = 0;
        if let Some(old) = unsafe { &*self.old_child.get() } {
            visitor(index, old.as_ref());
            index += 1;
        }
        visitor(index, self.current_child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedSwitcherElement"
    }
}

impl EventElement for AnimatedSwitcherElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.current_child.on_event(event)
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.current_child.as_ref());
    }
}

impl Rebuildable for AnimatedSwitcherElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.current_child.rebuild_if_dirty(ctx);
        if let Some(old) = unsafe { &*self.old_child.get() } {
            old.rebuild_if_dirty(ctx);
        }
    }
}

impl LayoutElement for AnimatedSwitcherElement {
    fn pos(&self) -> Option<Vec2d> {
        self.current_child.pos()
    }

    fn size(&self) -> Option<Size> {
        self.current_child.size()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.current_child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.current_child.content_size(ctx)
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.current_child.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.current_child.invalidate_layout();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::curve::curves;

    use crate::widgets::test_frame_requester;

    struct TestWidget(&'static str);

    impl Widget for TestWidget {
        fn key(&self) -> Option<Key> {
            Some(Key::Value(self.0.to_owned()))
        }

        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            panic!("not needed for state lifecycle tests")
        }
    }

    impl aimer_widget::PortableWidget for TestWidget {}

    fn state(key: &'static str) -> AnimatedSwitcherState<TestWidget> {
        AnimatedSwitcher::new(Duration::from_millis(100), curves::LINEAR, TestWidget(key))
            .create_state()
    }

        #[test]
    fn a_custom_curve_is_stored_for_both_directions() {
        use crate::primitives::curve::Curve;

        struct Halfway;
        impl Curve for Halfway {
            fn transform(&self, t: f32) -> f32 {
                t * 0.5
            }
        }

        let switcher = AnimatedSwitcher::new(Duration::from_millis(100), Halfway, TestWidget("a"))
            .with_switch_out_curve(curves::LINEAR);

        assert_eq!(switcher.curve.transform(1.0), 0.5);
        assert_eq!(switcher.switch_out_curve.as_ref().unwrap().transform(1.0), 1.0);
    }

    #[test]
    fn initial_child_is_shown_without_starting_a_transition() {
        let current = state("initial");

        assert_eq!(current.in_controller.value(), 1.0);
        assert!(!current.in_controller.is_animating());
        assert!(current.old_child.is_none());
    }

    #[test]
    fn changed_key_preserves_outgoing_child_and_starts_both_transitions() {
        test_frame_requester::install();
        test_frame_requester::reset();
        let mut current = state("first");

        current.adopt_config_from(state("second"));

        assert!(current.old_child.is_some());
        assert_eq!(current.child_key, Some(Key::Value("second".to_owned())));
        assert!(current.in_controller.is_animating());
        assert!(current.out_controller.is_animating());
        assert_eq!(
            test_frame_requester::count(),
            1,
            "transition startup must schedule its first frame"
        );
        std::thread::sleep(Duration::from_millis(10));

        assert_eq!(current.in_controller.tick(AnimInstant::now()), 0.0);
        assert_eq!(current.out_controller.tick(AnimInstant::now()), 0.0);
        assert!(current.in_controller.is_animating());
        assert!(current.out_controller.is_animating());
    }

    #[test]
    fn unchanged_key_updates_without_a_transition() {
        let mut current = state("same");

        current.adopt_config_from(state("same"));

        assert!(current.old_child.is_none());
        assert!(!current.out_controller.is_animating());
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn portable_lowering_is_transparent_to_the_current_child() {
        use aimer_widget::portable::__anteros::{Version, WIDGET_TEXT, WidgetDocumentView};
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
            StableId128,
        };
        use aimer_widget::{ErrorWidget, PortableWidget};

        struct PortableLeaf;

        impl Widget for PortableLeaf {
            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                ErrorWidget::new("portable leaf").to_element(ctx)
            }
        }

        impl PortableWidget for PortableLeaf {
            fn to_portable_node(
                self,
                ctx: &mut PortableBuildContext,
                source: SourceFingerprint,
            ) -> Result<
                aimer_widget::portable::PortableNodeId,
                aimer_widget::portable::PortableBuildError,
            > {
                ctx.push_node(WIDGET_TEXT, Version::new(1, 0), None, source, &[], &[])
            }
        }

        let mut context = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let root = AnimatedSwitcher::new(Duration::from_millis(100), curves::LINEAR, PortableLeaf)
            .to_portable_node(
                &mut context,
                SourceFingerprint::new(StableId128::from_bytes([9; 16])),
            )
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();

        assert_eq!(
            view.node(view.root_node()).unwrap().widget_type(),
            WIDGET_TEXT
        );
        assert_eq!(view.node_count(), 1);
    }

    // ─── Both children are *built* again on every frame of a transition ────
    //
    // A cross-fade paints two subtrees, and the switcher rebuilds itself on
    // each frame of it, so each of those frames has to reach both the outgoing
    // and the incoming child again. Holding the children as values would let
    // the first frame consume them; the retained child the switcher stores
    // instead keeps them reachable, and reaches the *same* element every time
    // rather than building another one — which is why these tests count
    // children in the tree instead of counting builds.
    mod reaches_both_children {
        use std::cell::Cell;
        use std::rc::Rc;

        use aimer_attribute::size::ResolvedSize;
        use aimer_widget::ErrorWidget;
        use aimer_widget::base::WindowHandle;

        use super::*;

        /// A child that counts how many times it was asked for an element.
        struct Probe {
            key: &'static str,
            builds: Rc<Cell<usize>>,
        }

        impl Widget for Probe {
            fn key(&self) -> Option<Key> {
                Some(Key::Static(self.key))
            }

            fn to_element(self, ctx: &BuildContext) -> AnyElement {
                self.builds.set(self.builds.get() + 1);
                ErrorWidget::new("probe").to_element(ctx)
            }

            fn debug_name(&self) -> &'static str {
                "Probe"
            }
        }

        impl aimer_widget::PortableWidget for Probe {}

        fn context() -> BuildContext<'static> {
            let canvas = {
                let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
                aimer_canvas::FrameCanvas::new(inner)
            };
            BuildContext::new(
                canvas,
                ResolvedSize::default(),
                1.0,
                Default::default(),
                Default::default(),
                WindowHandle::headless(Default::default(), 1.0),
                tokio::runtime::Handle::current(),
            )
        }

        fn switcher(key: &'static str, builds: &Rc<Cell<usize>>) -> AnimatedSwitcher<Probe> {
            switcher_lasting(Duration::from_millis(100), key, builds)
        }

        fn switcher_lasting(
            duration: Duration,
            key: &'static str,
            builds: &Rc<Cell<usize>>,
        ) -> AnimatedSwitcher<Probe> {
            AnimatedSwitcher::new(
                duration,
                curves::LINEAR,
                Probe {
                    key,
                    builds: Rc::clone(builds),
                },
            )
        }

        /// Returns how many probes a tree reaches.
        ///
        /// A probe is counted where the walk runs out of children: a retained
        /// child is reached through a proxy that reports the child's own name,
        /// so only the leaf itself is the child, and its holders are the path
        /// to it.
        fn probe_count(element: &dyn aimer_widget::Element) -> usize {
            let mut children = 0;
            let mut probes = 0;
            element.visit_children(&mut |child| {
                children += 1;
                probes += probe_count(child);
            });
            if children == 0 {
                return usize::from(element.debug_name() == "ErrorWidget");
            }
            probes
        }

        #[tokio::test]
        async fn every_frame_of_a_transition_reaches_the_outgoing_and_incoming_child() {
            test_frame_requester::install();
            test_frame_requester::reset();
            let ctx = context();
            let outgoing = Rc::new(Cell::new(0));
            let incoming = Rc::new(Cell::new(0));

            let mut current = switcher("home", &outgoing).create_state();
            current.adopt_config_from(switcher("docs", &incoming).create_state());

            assert!(
                current.out_controller.is_animating(),
                "a changed child key has to start the cross-fade"
            );

            for frame in 1..=3 {
                let tree = current.build(&ctx).to_element(&ctx);

                assert_eq!(
                    probe_count(tree.as_ref()),
                    2,
                    "frame {frame} of the cross-fade must reach both children"
                );
                assert_eq!(
                    (outgoing.get(), incoming.get()),
                    (1, 1),
                    "frame {frame} must reuse those children, not rebuild them"
                );
            }
        }

        #[tokio::test]
        async fn transition_children_keep_their_child_identity_when_both_are_present() {
            test_frame_requester::install();
            test_frame_requester::reset();
            let ctx = context();

            let mut current = AnimatedSwitcher::new(
                Duration::from_millis(100),
                curves::LINEAR,
                ErrorWidget::new("home"),
            )
            .child_key("home")
            .create_state();
            current.adopt_config_from(
                AnimatedSwitcher::new(
                    Duration::from_millis(100),
                    curves::LINEAR,
                    ErrorWidget::new("docs"),
                )
                .child_key("docs")
                .create_state(),
            );

            let tree = current.build(&ctx).to_element(&ctx);
            let mut child_keys = Vec::new();
            tree.visit_children(&mut |child| {
                child_keys.push(child.reconciliation_key().cloned());
            });

            assert_eq!(
                child_keys,
                vec![
                    Some(Key::Value("docs".to_owned())),
                    Some(Key::Value("home".to_owned()))
                ],
                "incoming and outgoing children must remain distinguishable during a cross-fade"
            );
        }

        #[tokio::test]
        async fn a_finished_transition_stops_reaching_the_outgoing_child() {
            test_frame_requester::install();
            test_frame_requester::reset();
            let ctx = context();
            let outgoing = Rc::new(Cell::new(0));
            let incoming = Rc::new(Cell::new(0));

            let quick = Duration::from_millis(1);
            let mut current = switcher_lasting(quick, "home", &outgoing).create_state();
            current.adopt_config_from(switcher_lasting(quick, "docs", &incoming).create_state());

            // Run the fade to its end: the first tick starts the clock, the
            // second one arrives after the duration has elapsed.
            current.out_controller.tick(AnimInstant::now());
            std::thread::sleep(Duration::from_millis(10));
            current.out_controller.tick(AnimInstant::now());
            assert!(!current.out_controller.is_animating());

            let tree = current.build(&ctx).to_element(&ctx);

            assert_eq!(
                probe_count(tree.as_ref()),
                1,
                "once the fade is over the outgoing child leaves the tree"
            );
            assert_eq!(
                (outgoing.get(), incoming.get()),
                (0, 1),
                "a child that never reached a frame is never built, and one that                  did is built once"
            );
        }
    }

    // ─── End-to-end draw test: a keyed switcher across a "route" change ────
    //
    // Reproduces the website router: a top-level stateful widget rebuilds a
    // keyed `AnimatedSwitcher` whose child changes with the active route. On a
    // route change the switcher's live state must be carried across the parent
    // rebuild (via keyed reconciliation) so `adopt_config_from` observes the
    // new child key and starts a cross-fade. During that fade BOTH the outgoing
    // and incoming children must be painted; if only the new child is painted,
    // the switch is instant (the reported bug).
    mod draw_transition {
        use std::cell::RefCell;
        use std::rc::Rc;

        use aimer_widget::StatefulElement;
        use aimer_widget::base::BuildContext;

        use super::*;

        /// A leaf that records the label it renders each time it is drawn, so a
        /// test can see exactly which children reached the screen this frame.
        struct RecordingLeaf {
            label: &'static str,
            drawn: Rc<RefCell<Vec<&'static str>>>,
        }
        impl VisitorElement for RecordingLeaf {
            fn debug_name(&self) -> &'static str {
                "RecordingLeaf"
            }
        }
        impl Drawable for RecordingLeaf {
            fn update(&self, _ctx: &BuildContext) {
                self.drawn.borrow_mut().push(self.label);
            }
        }
        impl EventElement for RecordingLeaf {}
        impl LayoutElement for RecordingLeaf {}
        impl Rebuildable for RecordingLeaf {}

        /// Widget wrapper around `RecordingLeaf` so it can be the switcher
        /// child.
        struct RecordingPage {
            label: &'static str,
            drawn: Rc<RefCell<Vec<&'static str>>>,
        }
        impl Widget for RecordingPage {
            fn to_element(self, _ctx: &BuildContext) -> AnyElement {
                RecordingLeaf {
                    label: self.label,
                    drawn: self.drawn.clone(),
                }
                .boxed()
            }
            fn debug_name(&self) -> &'static str {
                "RecordingPage"
            }
        }

        impl aimer_widget::PortableWidget for RecordingPage {}

        fn route_label(route: usize) -> &'static str {
            if route == 0 { "home" } else { "docs" }
        }

        /// A "router": rebuilding it with a new `route` swaps the switcher
        /// child.
        struct RouterMock {
            drawn: Rc<RefCell<Vec<&'static str>>>,
        }
        struct RouterMockState {
            route: usize,
            drawn: Rc<RefCell<Vec<&'static str>>>,
            updater: StateUpdater<Self>,
        }
        impl StatefulWidget for RouterMock {
            type State = RouterMockState;
            fn create_state(self) -> Self::State {
                RouterMockState {
                    route: 0,
                    drawn: self.drawn.clone(),
                    updater: StateUpdater::new(),
                }
            }
        }
        impl State<RouterMock> for RouterMockState {
            fn init_state(&mut self, updater: StateUpdater<Self>) {
                self.updater = updater;
            }
            fn build(&self, _ctx: &BuildContext) -> impl Widget {
                let label = route_label(self.route);
                AnimatedSwitcher::new(
                    Duration::from_millis(50),
                    curves::LINEAR,
                    RecordingPage {
                        label,
                        drawn: self.drawn.clone(),
                    }
                    .boxed(),
                )
                .child_key(label)
                .key("route-switcher")
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        fn dummy_async_handle() -> tokio::runtime::Handle {
            static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> =
                std::sync::OnceLock::new();
            let runtime = RUNTIME.get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
            });
            let _guard = runtime.enter();
            tokio::runtime::Handle::current()
        }

        fn dummy_build_context() -> BuildContext<'static> {
            let canvas = {
                let leaked: &'static aimer_canvas::InnerCanvas =
                    Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
                aimer_canvas::FrameCanvas::new(leaked)
            };
            BuildContext::new(
                canvas,
                Default::default(),
                1.0,
                Default::default(),
                Default::default(),
                WindowHandle::headless(Default::default(), 1.0),
                #[cfg(not(target_arch = "wasm32"))]
                dummy_async_handle(),
            )
        }

        #[test]
        fn switching_route_cross_fades_instead_of_switching_instantly() {
            let ctx = dummy_build_context();
            let drawn: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));

            let (router, updater) = StatefulElement::new_with_name(
                RouterMock {
                    drawn: drawn.clone(),
                },
                &ctx,
                "Router",
                None,
            );

            // Initial frame: only the "home" page is painted.
            drawn.borrow_mut().clear();
            router.update(&ctx);
            assert_eq!(
                *drawn.borrow(),
                ["home"],
                "initial frame should paint only the home page"
            );

            // Navigate home -> docs, then draw the very next frame.
            updater.set_state(|s| s.route = 1);
            router.rebuild_if_dirty(&ctx);
            drawn.borrow_mut().clear();
            router.update(&ctx);

            let painted = drawn.borrow().clone();
            assert!(
                painted.contains(&"home") && painted.contains(&"docs"),
                "a cross-fade must paint BOTH the outgoing (home) and incoming (docs) pages on the \
                 first frame after navigation; instead only {painted:?} was painted (instant switch)"
            );
        }
    }
}
