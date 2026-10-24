use std::cell::{Cell, UnsafeCell};
use std::marker::PhantomData;
use std::panic::Location;
use std::time::Duration;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_canvas::MorphParams;
use aimer_events::element::ElementEvent;
use aimer_widget::base::*;
use aimer_widget::{
    AnyElement, ChildBuilder, Drawable, Element, EventElement, EventResult, Key, LayoutElement,
    Rebuildable, State, StateUpdater, StatefulElement, StatefulWidget, StatelessElement,
    VisitorElement, Widget,
};

pub use aimer_canvas::MorphStyle;

use crate::control::controller::AnimationController;
use crate::primitives::curve::{AnyCurve, Curve};
use crate::primitives::time::AnimInstant;

// ---------------------------------------------------------------------------
// MorphTransition widget
// ---------------------------------------------------------------------------

/// How a [`MorphTransition`] blends its outgoing and incoming content.
///
/// These are the knobs of the shader behind the transition; each one matters
/// to some [`MorphStyle`]s only. Values are sanitized when the transition
/// runs, so an out-of-range or non-finite value never reaches the GPU.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MorphLook {
    style: MorphStyle,
    direction: [f32; 2],
    softness: f32,
    amplitude: f32,
    frequency: f32,
    seed: f32,
    /// `None` leaves it to the style: [`MorphStyle::Blur`] blurs by
    /// [`DEFAULT_BLUR`], [`MorphStyle::Goo`] melts over [`DEFAULT_GOO_REACH`],
    /// every other style stays sharp.
    blur: Option<f32>,
}

/// The blur, in logical pixels, a [`MorphStyle::Blur`] reaches mid-way unless
/// told otherwise.
const DEFAULT_BLUR: f32 = 10.0;

/// How far, in logical pixels, a [`MorphStyle::Goo`] reaches at its widest unless
/// told otherwise. It decides how wide a gap the shapes can bridge.
const DEFAULT_GOO_REACH: f32 = 14.0;

impl MorphLook {
    #[cfg(test)]
    fn with_style(mut self, style: MorphStyle) -> Self {
        self.style = style;
        self
    }

    /// The parameters of the frame at `progress`.
    ///
    /// A field the shader cannot use falls back to its default instead of
    /// failing the whole frame.
    fn params(&self, progress: f32) -> MorphParams {
        let defaults = MorphParams::new(self.style, 0.0);
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        MorphParams {
            style: self.style,
            progress: finite(progress, 0.0).clamp(0.0, 1.0),
            direction: [
                finite(self.direction[0], defaults.direction[0]),
                finite(self.direction[1], defaults.direction[1]),
            ],
            softness: finite(self.softness, defaults.softness).clamp(0.0, 1.0),
            amplitude: finite(self.amplitude, defaults.amplitude).max(0.0),
            frequency: finite(self.frequency, defaults.frequency).max(0.0),
            seed: finite(self.seed, defaults.seed),
            blur: finite(
                self.blur.unwrap_or(match self.style {
                    MorphStyle::Blur => DEFAULT_BLUR,
                    MorphStyle::Goo => DEFAULT_GOO_REACH,
                    _ => 0.0,
                }),
                0.0,
            )
            .clamp(0.0, MorphParams::MAX_BLUR),
        }
    }
}

/// Morphs between the old and new content when the child changes.
///
/// `MorphTransition` keeps the outgoing child alive for `duration` and draws
/// both children into offscreen textures. A shader then reads the two
/// textures together and blends them according to a [`MorphStyle`]:
///
/// - [`Dissolve`](MorphStyle::Dissolve): pixels switch from old to new as a
///   noise threshold sweeps past them.
/// - [`Wipe`](MorphStyle::Wipe): a soft edge travels in a direction and
///   reveals the new content behind it.
/// - [`Liquid`](MorphStyle::Liquid): both pictures are pushed along a flowing
///   field while they cross-fade, so shapes melt into one another.
/// - [`Bounds`](MorphStyle::Bounds): the two content boxes are interpolated
///   in position and size and both pictures are stretched into the moving box.
/// - [`Slide`](MorphStyle::Slide): the old content is pushed out along a
///   direction while the new content enters from the opposite side, both
///   opaque and clipped to the transition, like a page turning in a
///   navigation bar.
/// - [`Goo`](MorphStyle::Goo): the pictures' silhouettes melt into one another
///   like drops of liquid, so a shape can bud out of another, join it through a
///   neck and part from it, as one pill divides into a pill and a circle.
/// - [`Blur`](MorphStyle::Blur): both pictures stay in place and trade focus,
///   the old one smudging away as the new one sharpens, the way an iOS toolbar
///   swaps its buttons. Use it when the two pictures share one frame.
///
/// Any style can also blur, with [`MorphTransition::blur`].
///
/// Because the blend works on what the children actually paint, it applies to
/// any content, whatever it draws, without describing its shape or color.
///
/// A morph starts when the effective child key changes. The child's own
/// [`Widget::key`] is the default identity; use [`MorphTransition::child_key`]
/// when the child type does not expose one. The initial child is shown
/// without animation.
///
/// Each morph frame costs two offscreen targets and one extra draw. A morph
/// whose children are larger than the renderer's offscreen budget falls back
/// to showing the pictures stacked rather than failing.
///
/// # Example
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::{curves, MorphStyle, MorphTransition};
/// use aimer_widget::ErrorWidget;
///
/// let transition = MorphTransition::new(Duration::from_millis(400),
///                                       curves::EASE_IN_OUT,
///                                       ErrorWidget::new("Profile card")).style(MorphStyle::Wipe)
///                                                                        .direction(1.0, 0.0)
///                                                                        .child_key("profile-card");
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_animation::MorphTransition", schema_only)]
pub struct MorphTransition<T: Widget + 'static> {
    /// The morphing content, kept as a builder because a cross-fade places the
    /// same content again on every frame of the transition.
    #[portable_child]
    pub child: ChildBuilder,
    #[portable_skip]
    pub duration: Duration,
    #[portable_skip]
    pub curve: AnyCurve,
    #[portable_skip]
    look: MorphLook,
    #[portable_skip]
    transition_key: Option<Key>,
    #[portable_skip]
    widget_key: Option<Key>,
    /// Records which child type completed this transition without storing it,
    /// so one `State` impl stays paired with one child type.
    #[portable_skip]
    marker: PhantomData<T>,
}

impl<T: Widget> MorphTransition<T> {
    /// Creates a morph transition around `child`.
    ///
    /// The initial child is displayed without animation. Later rebuilds start
    /// a morph when the effective child key changes. The look defaults to a
    /// [`MorphStyle::Dissolve`].
    pub fn new(duration: Duration, curve: impl Curve, child: T) -> Self {
        Self {
            child: ChildBuilder::from_widget(child),
            duration,
            curve: curve.into_any(),
            look: MorphLook {
                style: MorphStyle::Dissolve,
                direction: [1.0, 0.0],
                softness: 0.25,
                amplitude: 0.08,
                frequency: 14.0,
                seed: 0.0,
                blur: None,
            },
            transition_key: None,
            widget_key: None,
            marker: PhantomData,
        }
    }

    /// Chooses the effect that blends the outgoing and incoming content.
    #[inline]
    pub fn style(mut self, style: MorphStyle) -> Self {
        self.look.style = style;
        self
    }

    /// Sets the direction a [`MorphStyle::Wipe`] or [`MorphStyle::Slide`]
    /// travels, in screen space.
    ///
    /// `y` grows downward, so `(0.0, 1.0)` wipes or pushes from top to bottom.
    /// For a slide the old content leaves along the direction and the new
    /// content enters from the opposite side, so `(1.0, 0.0)` is a push to the
    /// right that brings the new content in from the left edge. The vector need
    /// not be normalized; a zero vector means rightward.
    #[inline]
    pub fn direction(mut self, x: f32, y: f32) -> Self {
        self.look.direction = [x, y];
        self
    }

    /// Sets the width of the soft edge, as a fraction of the morph.
    ///
    /// Applies to [`MorphStyle::Dissolve`] and [`MorphStyle::Wipe`]. Values
    /// are clamped to `0.0..=1.0`; the default is `0.25`.
    #[inline]
    pub fn softness(mut self, softness: f32) -> Self {
        self.look.softness = softness;
        self
    }

    /// Sets how far [`MorphStyle::Liquid`] displaces the content, as a
    /// fraction of the morph's smaller side. The default is `0.08`.
    #[inline]
    pub fn amplitude(mut self, amplitude: f32) -> Self {
        self.look.amplitude = amplitude;
        self
    }

    /// Sets how many noise cells span the morph's smaller side.
    ///
    /// Applies to [`MorphStyle::Dissolve`] and [`MorphStyle::Liquid`]; the
    /// default is `14.0`. Fewer cells give coarser blobs.
    #[inline]
    pub fn frequency(mut self, frequency: f32) -> Self {
        self.look.frequency = frequency;
        self
    }

    /// Offsets the noise, so two morphs of the same size need not look alike.
    #[inline]
    pub fn seed(mut self, seed: f32) -> Self {
        self.look.seed = seed;
        self
    }

    /// Sets the blur, in logical pixels, the pictures trade through; for
    /// [`MorphStyle::Goo`] it is how far the shapes reach to join each other.
    ///
    /// The outgoing picture blurs more as it leaves and the incoming one less as
    /// it arrives, so each is sharp while it is the only one showing. Any style
    /// honors it; [`MorphStyle::Blur`] uses `10.0` unless told otherwise and the
    /// others stay sharp. Values are clamped to `0.0..=128.0`.
    #[inline]
    pub fn blur(mut self, radius: f32) -> Self {
        self.look.blur = Some(radius);
        self
    }

    /// Sets an explicit child identity used to detect content changes.
    ///
    /// This overrides the key reported by the child widget.
    #[inline]
    #[track_caller]
    pub fn child_key(mut self, key: impl Into<Key>) -> Self {
        let caller = Location::caller();
        self.transition_key = Some(key.into().with_location(caller));
        self
    }

    /// Sets the identity of the transition widget for reconciliation.
    ///
    /// This does not determine whether a morph starts; use
    /// [`child_key`](Self::child_key) for that purpose.
    #[track_caller]
    #[inline]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        let caller = Location::caller();
        self.widget_key = Some(key.into().with_location(caller));
        self
    }
}

impl<T: Widget + 'static> StatefulWidget for MorphTransition<T> {
    type State = MorphTransitionState<T>;

    fn create_state(self) -> Self::State {
        MorphTransitionState {
            child_key: self.transition_key.or_else(|| Widget::key(&self.child)),
            current_child: self.child,
            old_child: None,
            old_child_key: None,
            duration: self.duration,
            controller: AnimationController::new(self.duration, self.curve.clone()),
            curve: self.curve,
            look: self.look,
            updater: StateUpdater::empty(),
            marker: PhantomData,
        }
    }
}

impl<T: Widget + 'static> Widget for MorphTransition<T> {
    fn key(&self) -> Option<Key> {
        self.widget_key.clone()
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let __key = Widget::key(&self);
        StatefulElement::new_with_name(self, ctx, "MorphTransition", __key)
            .0
            .boxed()
    }
}

#[doc(hidden)]
pub struct MorphTransitionState<T: Widget + 'static> {
    current_child: ChildBuilder,
    old_child: Option<ChildBuilder>,
    child_key: Option<Key>,
    old_child_key: Option<Key>,
    duration: Duration,
    curve: AnyCurve,
    look: MorphLook,
    controller: AnimationController,
    updater: StateUpdater<Self>,
    marker: PhantomData<T>,
}

impl<T: Widget + 'static> State<MorphTransition<T>> for MorphTransitionState<T> {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.duration = new.duration;
        self.curve = new.curve;
        self.look = new.look;
        self.controller.set_duration(self.duration);
        self.controller.set_curve(self.curve.clone());

        if self.child_key != new.child_key {
            self.old_child = Some(self.current_child.clone());
            self.old_child_key = self.child_key.clone();
            self.current_child = new.current_child;
            self.child_key = new.child_key;
            self.controller.reset();
            self.controller.forward();
        } else {
            self.current_child = new.current_child;
        }
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let morphing = self.controller.is_animating();
        MorphTransitionFrame {
            current_child: self.current_child.clone(),
            current_child_key: self.child_key.clone(),
            old_child: if morphing {
                self.old_child.clone()
            } else {
                None
            },
            old_child_key: if morphing {
                self.old_child_key.clone()
            } else {
                None
            },
            look: self.look,
            controller: self.controller.clone(),
        }
    }
}

struct MorphTransitionFrame {
    current_child: ChildBuilder,
    current_child_key: Option<Key>,
    old_child: Option<ChildBuilder>,
    old_child_key: Option<Key>,
    look: MorphLook,
    controller: AnimationController,
}

/// Builds one of the morph's two children under the identity the transition
/// tracks it by. The current and outgoing children are siblings while a morph
/// runs; without distinct identities reconciliation matches them positionally,
/// the render tree is handed the same element twice, and its structure sync
/// fails for the whole window.
fn keyed_child_element(child: ChildBuilder, key: Option<Key>, ctx: &BuildContext) -> AnyElement {
    let child = child.build(ctx);
    match key {
        Some(key) => StatelessElement::wrapper(child, Some(key), "MorphTransitionChild").boxed(),
        None => child,
    }
}

impl Widget for MorphTransitionFrame {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let current_child = keyed_child_element(self.current_child, self.current_child_key, ctx);
        let old_child = self
            .old_child
            .map(|child| keyed_child_element(child, self.old_child_key, ctx));
        let morphing = old_child.is_some();

        MorphTransitionElement {
            current_child: SyncChild::new(current_child),
            old_child: SyncChild(UnsafeCell::new(old_child)),
            controller: self.controller,
            window: ctx.window.clone(),
            look: self.look,
            progress: Cell::new(0.0),
            recorded: Cell::new(None),
            morph_state: Cell::new(if morphing {
                MorphState::Morphing
            } else {
                MorphState::Idle
            }),
        }
        .boxed()
    }
}

impl aimer_widget::PortableWidget for MorphTransitionFrame {}

/// The current morph animation state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum MorphState {
    /// No animation running; only the current child exists in the render tree.
    Idle,
    /// Blending the outgoing child into the current one.
    Morphing,
}

/// Unsafe wrapper for single-threaded mutable access to an element.
/// Safety: the rendering pipeline is single-threaded.
struct SyncChild(UnsafeCell<Option<AnyElement>>);
unsafe impl Send for SyncChild {}
unsafe impl Sync for SyncChild {}

impl SyncChild {
    fn new(element: AnyElement) -> Self {
        Self(UnsafeCell::new(Some(element)))
    }

    /// # Safety
    /// Must only be called from the single rendering thread.
    unsafe fn get(&self) -> Option<&dyn Element> {
        unsafe { (*self.0.get()).as_ref().map(|b| b.as_ref()) }
    }

    /// Take the element out, leaving `None` in its place.
    /// # Safety
    /// Must only be called from the single rendering thread.
    unsafe fn take(&self) -> Option<AnyElement> {
        unsafe { (*self.0.get()).take() }
    }
}

// ---------------------------------------------------------------------------
// MorphTransitionElement
// ---------------------------------------------------------------------------

/// The bits of the last recorded marker, so an unchanged frame records nothing.
type MarkerSignature = [u32; 8];

struct MorphTransitionElement {
    current_child: SyncChild,
    old_child: SyncChild,
    controller: AnimationController,
    window: WindowHandle,
    look: MorphLook,
    /// The eased progress settled by the latest `sync_local_v2_state`.
    progress: Cell<f32>,
    /// The marker the node's list currently holds; `None` while it holds none.
    recorded: Cell<Option<MarkerSignature>>,
    morph_state: Cell<MorphState>,
}

// Safety: rendering pipeline is single-threaded
unsafe impl Send for MorphTransitionElement {}
unsafe impl Sync for MorphTransitionElement {}

impl MorphTransitionElement {
    /// The marker this node should hold now, or `None` while it is idle.
    fn marker(&self) -> Option<MorphParams> {
        (self.morph_state.get() == MorphState::Morphing)
            .then(|| self.look.params(self.progress.get()))
    }

    fn signature(params: &MorphParams) -> MarkerSignature {
        let encoded = params.encode();
        let mut bits = [0u32; 8];
        for (word, bytes) in bits.iter_mut().zip(encoded.chunks_exact(4)) {
            *word = u32::from_le_bytes(bytes.try_into().expect("four bytes per word"));
        }
        bits
    }
}

impl Drawable for MorphTransitionElement {
    fn update(&self, ctx: &BuildContext) {
        self.controller.tick(AnimInstant::now());
        if self.controller.is_animating() || self.morph_state.get() != MorphState::Idle {
            aimer_widget::mark_paint_damage_full();
        }
        // The morph itself is a shader over the children's render nodes (see
        // `paint_local_v2`); this walks them and settles the state.
        self.draw_local_v2_compatibility(ctx);
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        // While idle the node holds no list at all, which is what drops the
        // marker once the morph has finished.
        let Some(params) = self.marker() else {
            self.recorded.set(None);
            return;
        };
        let canvas = aimer_canvas::Canvas::of(ctx);
        canvas.morph_children(&params);
        canvas.finish();
        self.recorded.set(Some(Self::signature(&params)));
    }

    fn local_v2_paint_needs_recording(&self, _ctx: &BuildContext) -> bool {
        self.recorded.get() != self.marker().as_ref().map(Self::signature)
    }

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        // Both children are presented through their own render nodes and the
        // shader reads them there, so the tree damages whatever they paint; no
        // promise about the paint extent of either is needed.
        let progress = self.controller.tick(AnimInstant::now());
        if !progress.is_finite() {
            return false;
        }
        self.progress.set(progress.clamp(0.0, 1.0));

        let identity = aimer_canvas::Mat3::identity();
        if self.morph_state.get() == MorphState::Idle {
            return ctx.set_local_v2_child_presentation_at(0, identity, 1.0);
        }
        if unsafe { self.old_child.get() }.is_none() {
            return false;
        }
        // The shader owns the blend, so neither child carries a presentation of
        // its own; this also clears one left by an earlier frame.
        ctx.set_local_v2_child_presentation_at(0, identity, 1.0)
            && ctx.set_local_v2_child_presentation_at(1, identity, 1.0)
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        unsafe {
            if self.morph_state.get() == MorphState::Morphing
                && let Some(old) = self.old_child.get()
            {
                old.update(ctx);
            }
            if let Some(child) = self.current_child.get() {
                child.update(ctx);
            }
        }

        if self.controller.is_animating() {
            self.window.request_redraw();
        } else if self.morph_state.get() == MorphState::Morphing {
            let _ = unsafe { self.old_child.take() };
            self.morph_state.set(MorphState::Idle);
            // The render tree still holds the outgoing child's nodes: tell it
            // the structure changed and give it a frame to resync.
            aimer_widget::notify_hosted_element_tree_changed();
            self.window.request_redraw();
        }
    }
}

impl VisitorElement for MorphTransitionElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        unsafe {
            if let Some(child) = self.current_child.get() {
                visitor(child);
            }
            if let Some(old) = self.old_child.get() {
                visitor(old);
            }
        }
    }

    /// The outgoing child comes first: the renderer reads the first child of a
    /// morph as the picture it blends away from.
    fn visit_retained_v2_children<'a>(&'a self, visitor: &mut dyn FnMut(usize, &'a dyn Element)) {
        let mut index = 0;
        if self.morph_state.get() == MorphState::Morphing
            && let Some(old) = unsafe { self.old_child.get() }
        {
            visitor(index, old);
            index += 1;
        }
        if let Some(current) = unsafe { self.current_child.get() } {
            visitor(index, current);
        }
    }

    fn debug_name(&self) -> &'static str {
        "MorphTransitionElement"
    }
}

impl EventElement for MorphTransitionElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        unsafe {
            self.current_child
                .get()
                .map(|c| c.on_event(event))
                .unwrap_or_else(EventResult::ignored)
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        unsafe {
            if let Some(child) = self.current_child.get() {
                visitor(child);
            }
        }
    }
}

impl Rebuildable for MorphTransitionElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        unsafe {
            if let Some(child) = self.current_child.get() {
                child.rebuild_if_dirty(ctx);
            }
            if let Some(old) = self.old_child.get() {
                old.rebuild_if_dirty(ctx);
            }
        }
    }
}

impl LayoutElement for MorphTransitionElement {
    fn pos(&self) -> Option<Vec2d> {
        unsafe { self.current_child.get().and_then(|c| c.pos()) }
    }

    fn size(&self) -> Option<Size> {
        unsafe { self.current_child.get().and_then(|c| c.size()) }
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        unsafe {
            self.current_child
                .get()
                .map(|c| c.computed_size(ctx))
                .unwrap_or(ResolvedSize {
                    width: 0.0,
                    height: 0.0,
                })
        }
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        unsafe {
            self.current_child
                .get()
                .map(|c| c.content_size(ctx))
                .unwrap_or(ResolvedSize {
                    width: 0.0,
                    height: 0.0,
                })
        }
    }

    fn get_size_from_child(&self) -> Option<Size> {
        unsafe {
            self.current_child
                .get()
                .and_then(|c| c.get_size_from_child())
        }
    }

    fn invalidate_layout(&self) {
        unsafe {
            if let Some(child) = self.current_child.get() {
                child.invalidate_layout();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::curve::curves;

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

    fn state(key: &'static str) -> MorphTransitionState<TestWidget> {
        MorphTransition::new(Duration::from_millis(100), curves::LINEAR, TestWidget(key))
            .create_state()
    }

    #[test]
    fn changed_key_preserves_old_child_and_starts_morph() {
        let mut current = state("small");

        current.adopt_config_from(state("large"));

        assert!(current.old_child.is_some());
        assert_eq!(current.old_child_key, Some(Key::Value("small".to_owned())));
        assert_eq!(current.child_key, Some(Key::Value("large".to_owned())));
        assert!(current.controller.is_animating());
    }

    #[test]
    fn unchanged_key_does_not_start_morph() {
        let mut current = state("card");

        current.adopt_config_from(state("card"));

        assert!(current.old_child.is_none());
        assert!(!current.controller.is_animating());
    }

    #[test]
    fn a_new_look_applies_without_starting_a_morph() {
        let mut current = state("card");
        let restyled = MorphTransition::new(
            Duration::from_millis(100),
            curves::LINEAR,
            TestWidget("card"),
        )
        .style(MorphStyle::Liquid)
        .amplitude(0.2)
        .create_state();

        current.adopt_config_from(restyled);

        assert_eq!(current.look.style, MorphStyle::Liquid);
        assert_eq!(current.look.amplitude, 0.2);
        assert!(current.old_child.is_none());
        assert!(!current.controller.is_animating());
    }

    #[test]
    fn the_look_maps_onto_the_shader_parameters() {
        let look = MorphTransition::new(
            Duration::from_millis(100),
            curves::LINEAR,
            TestWidget("card"),
        )
        .style(MorphStyle::Wipe)
        .direction(0.0, 1.0)
        .softness(0.5)
        .amplitude(0.1)
        .frequency(9.0)
        .seed(3.0)
        .blur(4.0)
        .look;

        let params = look.params(0.4);

        assert_eq!(params.style, MorphStyle::Wipe);
        assert_eq!(params.progress, 0.4);
        assert_eq!(params.direction, [0.0, 1.0]);
        assert_eq!(params.softness, 0.5);
        assert_eq!(params.amplitude, 0.1);
        assert_eq!(params.frequency, 9.0);
        assert_eq!(params.seed, 3.0);
        assert_eq!(params.blur, 4.0);
    }

    #[test]
    fn only_the_blur_style_blurs_unless_asked() {
        let look = |style| state("card").look.with_style(style);

        assert_eq!(look(MorphStyle::Blur).params(0.5).blur, DEFAULT_BLUR);
        assert_eq!(look(MorphStyle::Goo).params(0.5).blur, DEFAULT_GOO_REACH);
        for style in [
            MorphStyle::Dissolve,
            MorphStyle::Wipe,
            MorphStyle::Liquid,
            MorphStyle::Bounds,
            MorphStyle::Slide,
        ] {
            assert_eq!(look(style).params(0.5).blur, 0.0, "{style:?}");
        }

        let mut asked = look(MorphStyle::Wipe);
        asked.blur = Some(6.0);
        assert_eq!(asked.params(0.5).blur, 6.0, "any style blurs when asked");
        asked.blur = Some(10_000.0);
        assert_eq!(asked.params(0.5).blur, MorphParams::MAX_BLUR);
        asked.blur = Some(-3.0);
        assert_eq!(asked.params(0.5).blur, 0.0);
    }

    #[test]
    fn unusable_values_never_reach_the_shader() {
        let look = MorphLook {
            style: MorphStyle::Liquid,
            direction: [f32::NAN, f32::INFINITY],
            softness: 4.0,
            amplitude: -1.0,
            frequency: f32::NAN,
            seed: f32::NEG_INFINITY,
            blur: Some(f32::NAN),
        };

        let params = look.params(f32::NAN);

        // The renderer rejects a marker it cannot decode, which would drop the
        // morph for the frame; every sanitized value must decode.
        assert_eq!(MorphParams::decode(&params.encode()), Some(params));
        assert_eq!(params.progress, 0.0);
        assert_eq!(params.softness, 1.0);
        assert_eq!(params.amplitude, 0.0);
    }

    #[test]
    fn progress_is_clamped_into_the_unit_range() {
        let look = state("card").look;

        assert_eq!(look.params(-0.5).progress, 0.0);
        assert_eq!(look.params(1.5).progress, 1.0);
    }

    #[test]
    fn an_unchanged_marker_has_the_same_signature() {
        let look = state("card").look;

        assert_eq!(
            MorphTransitionElement::signature(&look.params(0.5)),
            MorphTransitionElement::signature(&look.params(0.5))
        );
        assert_ne!(
            MorphTransitionElement::signature(&look.params(0.5)),
            MorphTransitionElement::signature(&look.params(0.6))
        );
    }
}
