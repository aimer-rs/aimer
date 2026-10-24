pub mod core;
pub mod input;
pub mod physics;
pub mod platform;
pub mod rendering;
pub mod state;

// Keep the original module paths available while the implementation is
// organized by responsibility below `scrollable/`.
pub use core::raw_scroll;
pub use input::{device_contact, handle_scroll, scroll_frame};
pub use physics::{constants, overscroll_source, scroll_behavior, spring, velocity_history};
pub use platform::recovery_end;
#[cfg(any(target_arch = "wasm32", test))]
pub use platform::{web_overscroll, web_recovery_end};
pub use rendering::{cache_extent, draw_scroll, scroll_bar};
pub use state::{controller, scroll_storage};

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;

use crate::scrollable::raw_scroll::RawScrollableContainer;
pub use crate::scrollable::scroll_bar::*;
use crate::scrollable::scroll_bar::{reserved_viewport, track_width};
use aimer_attribute::position::Vec2d;
#[allow(unused)]
use aimer_macro::key;
use aimer_utils::callback::Callback;
use aimer_widget::InteractionBounds;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, Element, Key, RequiredChild, State, StateUpdater,
    StatefulElement, StatefulWidget, Widget,
};
use controller::ScrollState;
pub use controller::{DragMode, ScrollController};
pub use overscroll_source::{OverscrollSource, OverscrollSources};
pub use scroll_behavior::{ScrollAxis, ScrollBehavior};

/// Bounded, versioned configuration preserved when a Scrollable crosses the
/// portable guest/host boundary.
#[derive(Clone, Copy, Debug, PartialEq, aimer_macro::PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_scroll::scrollable::ScrollPortableConfig",
    max_encoded_bytes = 128
)]
pub struct ScrollPortableConfig {
    /// The axis on which content may overflow.
    pub axis: ScrollAxis,
    /// Whether the vertical scrollbar is enabled.
    pub vertical_scroll_bar: bool,
    /// Whether the horizontal scrollbar is enabled.
    pub horizontal_scroll_bar: bool,
    /// Whether scrollbars overlay the content or reserve layout space.
    pub scroll_bar_placement: ScrollBarPlacement,
}

impl Default for ScrollPortableConfig {
    fn default() -> Self {
        Self {
            axis: ScrollAxis::Vertical,
            vertical_scroll_bar: true,
            horizontal_scroll_bar: true,
            scroll_bar_placement: ScrollBarPlacement::Floating,
        }
    }
}

#[inline]
fn resolved_parent_extent(min: f32, max: f32, parent: f32) -> f32 {
    let extent = if max.is_finite() && max < f32::MAX {
        max
    } else if parent.is_finite() && parent < f32::MAX {
        parent.max(min)
    } else {
        min
    };

    extent.clamp(min, max)
}

/// A single-child viewport that scrolls overflowing content along one axis.
///
/// The child receives an unbounded constraint on the selected [`ScrollAxis`]
/// and the viewport clips and translates that content according to
/// [`ScrollBehavior`]. The default axis is vertical, both scroll bars are
/// enabled, teardown persistence is disabled, and no external
/// [`ScrollController`] is attached. Scrollbars float over the viewport by
/// default; call [`Scrollable::scroll_bar_placement`] to reserve inline space
/// instead. Call [`Scrollable::key`] to preserve the offset across a full
/// teardown and later recreation.
///
/// Attach a child with [`Scrollable::child`] to retain its concrete type, or
/// with [`Scrollable::box_child`] when branches need a shared erased type.
///
/// On the web target the bouncy edges of [`ScrollBehavior`] are off for the
/// wheel only; [`Scrollable::web_overscroll`] takes the bitmap that decides
/// which input devices bounce there.
///
/// # Example
///
/// ```rust
/// use aimer_container::SizedBox;
/// use aimer_flex::Column;
/// use aimer_scroll::{ScrollAxis, Scrollable};
///
/// let viewport = Scrollable::new().axis(ScrollAxis::Vertical)
///                                 .vertical_scroll_bar(None)
///                                 .child(Column::new().children([SizedBox::new().height(200),
///                                                                SizedBox::new().height(200)]));
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(
    id = "aimer_scroll::scrollable::Scrollable",
    schema_only,
    manual_lowering
)]
pub struct Scrollable<W = RequiredChild> {
    /// The scrolling content, kept as a builder because the viewport rebuilds
    /// itself on every offset change and needs the same content each time.
    #[portable_child]
    pub child: ChildBuilder,
    #[portable_skip]
    pub scroll_behavior: ScrollBehavior,
    #[portable_skip]
    pub key_scroll_strength: f32,
    #[portable_skip]
    pub axis: ScrollAxis,
    /// Whether enabled scrollbars float over the content or reserve layout
    /// space beside it. Defaults to [`ScrollBarPlacement::Floating`].
    #[portable_skip]
    pub scroll_bar_placement: ScrollBarPlacement,
    #[portable_skip]
    pub vertical_scroll_bar: Option<ScrollBar>,
    #[portable_skip]
    pub horizontal_scroll_bar: Option<ScrollBar>,
    config: ScrollPortableConfig,
    /// Identity used by live-state reconciliation and, after [`Scrollable::key`]
    /// is called, by `PageStorage`-style teardown persistence.
    ///
    /// A default scrollable does not read or write offset storage; rebuilds and
    /// resizes still preserve its live position through reconciliation.
    #[portable_skip]
    pub key: Key,
    #[portable_skip]
    remember_scroll_offset: bool,
    /// Optional app-held [`ScrollController`] for programmatic control. When
    /// `Some`, the app can read the live position and drive it with
    /// [`ScrollController::jump_to`] / [`ScrollController::animate_to`]; the
    /// controller shares this scrollable's state and survives rebuilds. `None`
    /// keeps the zero-cost default (internally managed) behavior.
    #[portable_skip]
    pub controller: Option<ScrollController>,
    /// Which input devices may rubber-band a bouncy edge on the web target.
    /// Defaults to [`OverscrollSources::WEB_DEFAULT`] and is ignored
    /// everywhere else. See [`Scrollable::web_overscroll`].
    #[portable_skip]
    pub web_overscroll: OverscrollSources,
    /// Records which child type completed the builder without storing it.
    ///
    /// The content itself is erased into [`ChildBuilder`], but the parameter
    /// has to survive so that a viewport without content stays
    /// `Scrollable<RequiredChild>` — a type that is deliberately not a
    /// [`Widget`].
    #[portable_skip]
    marker: PhantomData<W>,
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "illumos",
))]
#[aimer_widget::portable::__linkme::distributed_slice(
    aimer_widget::portable::materializer::PORTABLE_NATIVE_WIDGET_SCHEMAS
)]
#[linkme(crate = aimer_widget::portable::__linkme)]
#[allow(non_upper_case_globals)]
static __AIMER_PORTABLE_NATIVE_SCHEMA_FOR_SCROLLABLE:
    aimer_widget::portable::__anteros::PortableWidgetSchemaMetadata<'static> =
    <Scrollable as aimer_widget::portable::PortableWidgetSchema>::SCHEMA;

impl Default for Scrollable {
    fn default() -> Self {
        Self::new()
    }
}

impl Scrollable {
    /// Creates a vertical scrollable with default scroll bars and no
    /// controller.
    ///
    /// Finish the builder with [`Scrollable::child`] or
    /// [`Scrollable::box_child`].
    #[inline]
    #[track_caller]
    pub fn new() -> Self {
        let caller = std::panic::Location::caller();
        Self {
            child: ChildBuilder::required(),
            scroll_behavior: ScrollBehavior::default(),
            axis: ScrollAxis::default(),
            scroll_bar_placement: ScrollBarPlacement::default(),
            vertical_scroll_bar: Some(ScrollBar::default()),
            horizontal_scroll_bar: Some(ScrollBar::default()),
            key: Key::Value(caller.to_string()).with_location(caller),
            remember_scroll_offset: false,
            key_scroll_strength: 50f32,
            controller: None,
            web_overscroll: OverscrollSources::WEB_DEFAULT,
            config: ScrollPortableConfig::default(),
            marker: PhantomData,
        }
    }

    /// Creates a scrollable with its required child already attached.
    ///
    /// This is equivalent to [`Scrollable::new`] followed by
    /// [`Scrollable::child`]: it uses the default vertical axis and behavior,
    /// enables both default floating scroll bars, does not persist its offset
    /// across a teardown, and has no external controller.
    #[inline]
    #[track_caller]
    pub fn with_child<W: Widget + 'static>(child: W) -> Scrollable<W> {
        let caller = std::panic::Location::caller();
        Scrollable {
            child: ChildBuilder::from_widget(child),
            scroll_behavior: ScrollBehavior::default(),
            axis: ScrollAxis::default(),
            scroll_bar_placement: ScrollBarPlacement::default(),
            vertical_scroll_bar: Some(ScrollBar::default()),
            horizontal_scroll_bar: Some(ScrollBar::default()),
            key: Key::Value(caller.to_string()).with_location(caller),
            remember_scroll_offset: false,
            controller: None,
            key_scroll_strength: 50f32,
            web_overscroll: OverscrollSources::WEB_DEFAULT,
            config: ScrollPortableConfig::default(),
            marker: PhantomData,
        }
    }

    /// Replaces the scrolling physics and initial offset configuration.
    ///
    /// The default is [`ScrollBehavior::default`]. Offsets are interpreted in
    /// logical pixels and scaled for the current build context.
    #[inline]
    pub fn scroll_behavior(mut self, scroll_behavior: ScrollBehavior) -> Self {
        self.scroll_behavior = scroll_behavior;
        self
    }

    /// Sets the single axis along which content may overflow and scroll.
    ///
    /// The default is [`ScrollAxis::Vertical`]. The child is made unbounded
    /// only on this axis and remains constrained on the cross axis.
    #[inline]
    pub fn axis(mut self, axis: ScrollAxis) -> Self {
        self.axis = axis;
        self.config.axis = axis;
        self
    }

    /// Sets whether enabled scrollbars float over the content or are laid out
    /// inline beside it.
    #[inline]
    pub fn scroll_bar_placement(mut self, placement: ScrollBarPlacement) -> Self {
        self.scroll_bar_placement = placement;
        self.config.scroll_bar_placement = placement;
        self
    }

    /// Replaces the vertical scroll-bar configuration.
    ///
    /// A default bar is enabled initially. Pass `None` to hide it. This setting
    /// controls presentation and does not change [`Scrollable::axis`].
    #[inline]
    pub fn vertical_scroll_bar(mut self, scroll_bar: Option<ScrollBar>) -> Self {
        self.config.vertical_scroll_bar = scroll_bar.is_some();
        self.vertical_scroll_bar = scroll_bar;
        self
    }

    /// Replaces the horizontal scroll-bar configuration.
    ///
    /// A default bar is enabled initially. Pass `None` to hide it. This setting
    /// controls presentation and does not change [`Scrollable::axis`].
    #[inline]
    pub fn horizontal_scroll_bar(mut self, scroll_bar: Option<ScrollBar>) -> Self {
        self.config.horizontal_scroll_bar = scroll_bar.is_some();
        self.horizontal_scroll_bar = scroll_bar;
        self
    }

    /// Replaces the identity used to persist and restore the logical offset.
    ///
    /// Default scrollables preserve their live position during ordinary
    /// rebuilds but start from their declared initial offset after a full
    /// teardown. Supplying a stable key opts into storing the live offset and
    /// restoring it when the viewport is later recreated.
    #[inline]
    #[track_caller]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        let location = std::panic::Location::caller();
        self.key = key.into().with_location(location);
        self.remember_scroll_offset = true;
        self
    }

    /// Chooses which input devices may rubber-band a bouncy edge on the web
    /// target.
    ///
    /// A browser reports a finger landing and lifting exactly like a native
    /// platform does, so touch scrolling bounces there normally. Its `wheel`
    /// stream reports neither: it appends a momentum tail of its own after the
    /// user has let go, and an edge fed by that stream stretches on deltas
    /// nobody is producing. Only the wheel is therefore clamped by default —
    /// [`OverscrollSources::WEB_DEFAULT`].
    ///
    /// The bitmap only removes the bounce; it never adds one. A behavior that
    /// is not [`bouncy`](ScrollBehavior::bouncy) stays rigid on every device,
    /// and native targets ignore this setting entirely.
    ///
    /// # Example
    ///
    /// ```rust
    /// use aimer_container::SizedBox;
    /// use aimer_scroll::{OverscrollSources, Scrollable};
    ///
    /// // Rubber-band edges in the browser for every device, wheel included.
    /// let viewport = Scrollable::new().web_overscroll(OverscrollSources::ALL)
    ///                                 .child(SizedBox::new().height(2000));
    /// ```
    ///
    /// Narrow it instead of widening it — here nothing but a finger bounces:
    ///
    /// ```rust
    /// use aimer_container::SizedBox;
    /// use aimer_scroll::{OverscrollSource, OverscrollSources, Scrollable};
    ///
    /// let touch_only = OverscrollSources::NONE.with(OverscrollSource::Touch);
    /// let viewport = Scrollable::new().web_overscroll(touch_only)
    ///                                 .child(SizedBox::new().height(2000));
    /// ```
    #[inline]
    pub fn web_overscroll(mut self, web_overscroll: impl Into<OverscrollSources>) -> Self {
        self.web_overscroll = web_overscroll.into();
        self
    }

    /// Attaches an application-owned controller for reading or driving offset.
    ///
    /// By default no controller is attached. The supplied controller replaces
    /// any previous one and its state survives widget rebuilds.
    #[inline]
    pub fn controller(mut self, controller: ScrollController) -> Self {
        self.controller = Some(controller);
        self
    }

    /// Attaches the required child and completes this builder.
    ///
    /// This terminal operation preserves all scrolling configuration and the
    /// child's concrete type. Use [`Scrollable::box_child`] when different
    /// branches need one erased return type.
    #[inline]
    pub fn child<W: Widget + 'static>(self, child: W) -> Scrollable<W> {
        Scrollable {
            key_scroll_strength: self.key_scroll_strength,
            child: ChildBuilder::from_widget(child),
            scroll_behavior: self.scroll_behavior,
            axis: self.axis,
            scroll_bar_placement: self.scroll_bar_placement,
            key: self.key,
            remember_scroll_offset: self.remember_scroll_offset,
            controller: self.controller,
            vertical_scroll_bar: self.vertical_scroll_bar.clone(),
            horizontal_scroll_bar: self.horizontal_scroll_bar.clone(),
            web_overscroll: self.web_overscroll,
            config: self.config,
            marker: PhantomData,
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    ///
    /// This is equivalent to calling [`Scrollable::child`] followed by
    /// [`Widget::boxed`]. Use it when different branches must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

/// Persistent state for a [`Scrollable`].
///
/// This type separates immutable widget configuration from the live scroll
/// engine. A parent rebuild refreshes the child, behavior, axis, bars, key, and
/// controller through [`State::adopt_config_from`] while preserving the current
/// offset and any in-flight drag or animation state.
pub struct ScrollableState<W: Widget + 'static> {
    child: ChildBuilder,
    scroll_behavior: ScrollBehavior,
    key_scroll_strength: f32,
    axis: ScrollAxis,
    scroll_bar_placement: ScrollBarPlacement,
    vertical_scroll_bar: Option<ScrollBar>,
    horizontal_scroll_bar: Option<ScrollBar>,
    key: Key,
    remember_scroll_offset: bool,
    controller: Option<ScrollController>,
    web_overscroll: OverscrollSources,
    scroll_state: RefCell<Option<Rc<ScrollState>>>,
    refresh_scroll_state: Cell<bool>,
    /// Keeps one state type per child type, exactly as the previous typed
    /// child field did, so a viewport whose content type changes starts from a
    /// fresh scroll engine instead of adopting another viewport's state.
    marker: PhantomData<W>,
}

impl<W: Widget + 'static> StatefulWidget for Scrollable<W> {
    type State = ScrollableState<W>;

    fn create_state(self) -> Self::State {
        ScrollableState {
            key_scroll_strength: self.key_scroll_strength,
            child: self.child.clone(),
            scroll_behavior: self.scroll_behavior,
            axis: self.axis,
            scroll_bar_placement: self.scroll_bar_placement,
            vertical_scroll_bar: self.vertical_scroll_bar.clone(),
            horizontal_scroll_bar: self.horizontal_scroll_bar.clone(),
            key: self.key.clone(),
            remember_scroll_offset: self.remember_scroll_offset,
            controller: self.controller.clone(),
            web_overscroll: self.web_overscroll,
            scroll_state: RefCell::new(None),
            refresh_scroll_state: Cell::new(false),
            marker: PhantomData,
        }
    }
}

impl<W: Widget + 'static> Widget for Scrollable<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Scrollable", None)
            .0
            .boxed()
    }
}

impl<W: Widget + 'static> aimer_widget::PortableWidget for Scrollable<W> {
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        ctx: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<aimer_widget::portable::PortableNodeId, aimer_widget::portable::PortableBuildError>
    {
        let property_id = aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_scroll::scrollable::Scrollable:config",
        );
        let config = ctx.encode_property_named(
            property_id,
            "aimer.property:aimer_scroll::scrollable::Scrollable:config",
            source.child(aimer_widget::portable::__anteros::stable_schema_hash64(
                "aimer.source:aimer_scroll::scrollable::Scrollable:config",
            )),
            self.config,
        )?;
        let child = self.child.into_portable_node(
            ctx,
            source.child(aimer_widget::portable::__anteros::stable_schema_hash64(
                "aimer.source:aimer_scroll::scrollable::Scrollable:child",
            )),
        )?;
        let schema = <Self as aimer_widget::portable::PortableWidgetSchema>::SCHEMA;
        ctx.push_node(
            schema.widget().id(),
            schema.widget().min_version(),
            None,
            source,
            &[aimer_widget::portable::__anteros::WidgetProperty::new(
                property_id,
                config,
            )],
            &[child],
        )
    }
}

impl<W: Widget + 'static> State<Scrollable<W>> for ScrollableState<W> {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        let controller_changed = match (&self.controller, &new.controller) {
            (_, None) => false,
            (Some(current), Some(new)) => !current.shares_identity_with(new),
            (None, Some(_)) => true,
        };
        let engine_config_changed =
            !same_scroll_behavior(self.scroll_behavior, new.scroll_behavior)
                || !same_scroll_axis(self.axis, new.axis)
                || self.scroll_bar_placement != new.scroll_bar_placement
                || self.web_overscroll != new.web_overscroll
                || self.key != new.key
                || self.remember_scroll_offset != new.remember_scroll_offset
                || controller_changed;

        self.child = new.child;
        self.scroll_behavior = new.scroll_behavior;
        self.axis = new.axis;
        self.scroll_bar_placement = new.scroll_bar_placement;
        self.vertical_scroll_bar = new.vertical_scroll_bar;
        self.horizontal_scroll_bar = new.horizontal_scroll_bar;
        self.key = new.key;
        self.remember_scroll_offset = new.remember_scroll_offset;
        self.web_overscroll = new.web_overscroll;

        if let Some(new_ctrl) = new.controller {
            self.controller = Some(new_ctrl);
        }

        if engine_config_changed {
            self.refresh_scroll_state.set(true);
        }
    }

    #[inline]
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let ctrl = self.live_scroll_state(ctx);

        ScrollableFrame {
            child: self.child.clone(),
            ctrl,
            vertical_scroll_bar: self.vertical_scroll_bar.clone(),
            horizontal_scroll_bar: self.horizontal_scroll_bar.clone(),
        }
    }
}

impl<W: Widget + 'static> ScrollableState<W> {
    #[inline]
    fn live_scroll_state(&self, ctx: &BuildContext) -> Rc<ScrollState> {
        let mut live = self.scroll_state.borrow_mut();
        let scale_changed = live
            .as_ref()
            .is_some_and(|state| state.last_scale.get() != ctx.scale);
        let refresh_scroll_state = self.refresh_scroll_state.replace(false);
        if live.is_none() || scale_changed || refresh_scroll_state {
            let next = self.create_scroll_state(ctx);
            if let Some(previous) = live.as_ref() {
                // Adopting the previous engine's live offset keeps the viewport
                // in place across a rebuild or scale change, but a storage-key
                // switch means this viewport now shows a *different* list:
                // `create_scroll_state` has already seeded it from that key's
                // saved offset, so carrying the previous list's position across
                // would share one offset between the two. Only adopt the live
                // interaction state when the key is unchanged.
                if previous.storage_key == self.key
                    && previous.remember_scroll_offset == self.remember_scroll_offset
                {
                    next.adopt_scroll_state(previous);
                }
            }
            *live = Some(next);
        }

        let state = live
            .as_ref()
            .expect("scroll state is initialized before the frame is built")
            .clone();

        // Offered on every build, not only when the engine is created: a rebuild
        // can produce a state that is then dropped in favour of the one the
        // surviving element carried, and the controller must end up on whichever
        // engine actually draws this frame. Re-offering the bound engine is a
        // pointer comparison.
        if let Some(controller) = &self.controller {
            controller.attach(state.clone());
        }

        state
    }
    #[inline]
    fn create_scroll_state(&self, ctx: &BuildContext) -> Rc<ScrollState> {
        // Seed an explicitly keyed viewport from its stored position after a
        // full teardown; an unkeyed viewport always uses the declared behavior.
        // Stored offsets are logical (unscaled), so re-apply `ctx.scale` here
        // just like the declared offset below.
        let stored_offset = self
            .remember_scroll_offset
            .then(|| scroll_storage::read_offset(&self.key))
            .flatten();
        let mut initial_offset = stored_offset
            .map(|logical| Vec2d {
                x: logical.x * ctx.scale,
                y: logical.y * ctx.scale,
            })
            .unwrap_or(Vec2d {
                x: self.scroll_behavior.scroll_offset.x * ctx.scale,
                y: self.scroll_behavior.scroll_offset.y * ctx.scale,
            });

        // If an app-supplied controller is already attached (i.e. this is a
        // rebuild), it is the source of truth for the live position — seed the
        // fresh state from it so the viewport stays put. Its `offset()` is
        // logical (positive toward the content end); convert to the internal
        // scaled/negated convention.
        if let Some(ctrl) = &self.controller
            && ctrl.is_attached()
        {
            let logical = ctrl.offset();
            initial_offset = Vec2d {
                x: -logical.x * ctx.scale,
                y: -logical.y * ctx.scale,
            };
        }
        let initial_offset = ScrollState::axis_only(self.axis, initial_offset);
        let initial_velocity = ScrollState::axis_only(
            self.axis,
            Vec2d {
                x: self.scroll_behavior.velocity.x * ctx.scale,
                y: self.scroll_behavior.velocity.y * ctx.scale,
            },
        );

        // Which devices the target trusts with a rubber band is fixed for the
        // life of the engine; only the device currently scrolling changes.
        let overscroll_sources =
            resolved_overscroll_sources(self.web_overscroll, cfg!(target_arch = "wasm32"));

        Rc::new(ScrollState {
            key_scroll_strength: self.key_scroll_strength,
            speed_multiplier: ctx.scale,
            scroll_offset: Cell::new(initial_offset),
            storage_key: self.key.clone(),
            remember_scroll_offset: self.remember_scroll_offset,
            last_pointer_pos: Cell::new(None),
            drag_mode: Cell::new(DragMode::None),
            cached_max_scroll: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            cached_min_scroll: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            pointer_velocity: Cell::new(initial_velocity),
            last_event_time: Cell::new(None),
            last_frame_time: Cell::new(None),
            v_thumb_rect: Cell::new(None),
            h_thumb_rect: Cell::new(None),
            thumb_hovered: Cell::new(false),
            v_scroll_multiplier: Cell::new(0.0),
            h_scroll_multiplier: Cell::new(0.0),
            last_scale: Cell::new(ctx.scale),
            scroll_behavior: self.scroll_behavior,
            overscroll_sources,
            overscroll_source: Cell::new(OverscrollSource::Wheel),
            axis: self.axis,
            scroll_bar_placement: self.scroll_bar_placement,
            cursor_pos: Cell::new(None),
            velocity_history: RefCell::new(None),
            cached_viewport: Cell::new((0.0, 0.0)),
            cached_v_track_width: Cell::new(0.0),
            cached_h_track_width: Cell::new(0.0),
            cached_content_size: Cell::new(Default::default()),
            cached_content_size_valid: Cell::new(false),
            fling_start_time: Cell::new(None),
            fling_start_offset: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            fling_target_offset: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            fling_duration: Cell::new(0.0),
            anim_curve: RefCell::new(None),
            active_touch_id: Cell::new(None),
            spring_velocity: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            momentum_start_time: Cell::new(None),
            vel_accum: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            vel_sample_time: Cell::new(None),
            is_scrolling: Cell::new(false),
            #[cfg(debug_assertions)]
            session_input_events: Cell::new(0),
            #[cfg(debug_assertions)]
            session_draw_frames: Cell::new(0),
            #[cfg(debug_assertions)]
            session_moved_frames: Cell::new(0),
            // Left empty here; `live_scroll_state` re-shares any
            // app-registered
            // scroll-lifecycle callbacks when it attaches the controller.
            on_scroll_start: RefCell::new(Callback::default()),
            on_scroll_end: RefCell::new(Callback::default()),
            on_scroll: RefCell::new(Callback::default()),
            last_reported_offset: Cell::new(None),
            last_drawn_offset: Cell::new(None),
            overscroll_hold: Cell::new(None),
            direct_overscroll_hold: Cell::new(false),
            highest_overscroll_offset: Default::default(),
            overscroll_peak_at: Cell::new(None),
            device_contact: Cell::new(false),
            #[cfg(target_arch = "wasm32")]
            web_overscroll_decay: web_overscroll::WebOverscrollDecay::new(),
            #[cfg(target_arch = "wasm32")]
            web_recovery_end: web_recovery_end::WebRecoveryEnd::new(),
        })
    }
}

/// The devices the scroll engine is actually built to rubber-band for.
///
/// `web_overscroll` is the [`Scrollable::web_overscroll`] bitmap and
/// `target_web` whether the build runs in a browser. Only a browser narrows
/// the set: every native platform reports the end of every gesture, so nothing
/// there needs to be held back.
///
/// [`ScrollBehavior::bouncy`] is left untouched — resistance, recovery and
/// friction stay exactly as declared, so a device that keeps its bounce keeps
/// the tuning the app asked for.
#[inline]
const fn resolved_overscroll_sources(
    web_overscroll: OverscrollSources,
    target_web: bool,
) -> OverscrollSources {
    if target_web {
        web_overscroll
    } else {
        OverscrollSources::ALL
    }
}

#[inline]
fn same_scroll_axis(current: ScrollAxis, new: ScrollAxis) -> bool {
    matches!(
        (current, new),
        (ScrollAxis::Vertical, ScrollAxis::Vertical)
            | (ScrollAxis::Horizontal, ScrollAxis::Horizontal)
    )
}

#[inline]
fn same_scroll_behavior(current: ScrollBehavior, new: ScrollBehavior) -> bool {
    current.max_scroll.x == new.max_scroll.x
        && current.max_scroll.y == new.max_scroll.y
        && current.min_scroll.x == new.min_scroll.x
        && current.min_scroll.y == new.min_scroll.y
        && current.velocity.x == new.velocity.x
        && current.velocity.y == new.velocity.y
        && current.scroll_offset.x == new.scroll_offset.x
        && current.scroll_offset.y == new.scroll_offset.y
        && current.bouncy == new.bouncy
        && current.bouncy_resistance == new.bouncy_resistance
        && current.bouncy_recovery == new.bouncy_recovery
        && current.friction == new.friction
}

struct ScrollableFrame {
    child: ChildBuilder,
    ctrl: Rc<ScrollState>,
    vertical_scroll_bar: Option<ScrollBar>,
    horizontal_scroll_bar: Option<ScrollBar>,
}

impl Widget for ScrollableFrame {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let full_width = resolved_parent_extent(
            ctx.box_constraint.min_width,
            ctx.box_constraint.max_width,
            ctx.parent_size.width,
        );
        let full_height = resolved_parent_extent(
            ctx.box_constraint.min_height,
            ctx.box_constraint.max_height,
            ctx.parent_size.height,
        );
        let vertical_bar_width = if matches!(self.ctrl.axis, ScrollAxis::Vertical) {
            self.vertical_scroll_bar
                .as_ref()
                .map(|bar| track_width(bar, full_width, ctx.scale))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let horizontal_bar_height = if matches!(self.ctrl.axis, ScrollAxis::Horizontal) {
            self.horizontal_scroll_bar
                .as_ref()
                .map(|bar| track_width(bar, full_height, ctx.scale))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let (viewport_w, viewport_h) = reserved_viewport(
            self.ctrl.axis,
            full_width,
            full_height,
            if matches!(self.ctrl.axis, ScrollAxis::Vertical) {
                vertical_bar_width
            } else {
                horizontal_bar_height
            },
            self.ctrl.scroll_bar_placement,
        );

        let mut child_ctx = ctx.clone();
        child_ctx.box_constraint.min_width = child_ctx.box_constraint.min_width.min(viewport_w);
        child_ctx.box_constraint.min_height = child_ctx.box_constraint.min_height.min(viewport_h);
        child_ctx.box_constraint.max_width = viewport_w;
        child_ctx.box_constraint.max_height = viewport_h;
        child_ctx.parent_size = aimer_attribute::size::ResolvedSize {
            width: viewport_w,
            height: viewport_h,
        };
        match self.ctrl.axis {
            ScrollAxis::Vertical => child_ctx.box_constraint.max_height = f32::MAX,
            ScrollAxis::Horizontal => child_ctx.box_constraint.max_width = f32::MAX,
        }
        let child = self.child.build(&child_ctx);
        let vertical_scroll_bar = self.vertical_scroll_bar.map(|bar| {
            bar.for_scrollable(self.ctrl.clone(), ScrollAxis::Vertical)
                .to_element(ctx)
        });
        let horizontal_scroll_bar = self.horizontal_scroll_bar.map(|bar| {
            bar.for_scrollable(self.ctrl.clone(), ScrollAxis::Horizontal)
                .to_element(ctx)
        });

        RawScrollableContainer {
            child,
            ctrl: self.ctrl.clone(),
            vertical_scroll_bar,
            horizontal_scroll_bar,
            viewport_w,
            viewport_h,
            vertical_bar_width,
            horizontal_bar_height,
            bounds: {
                let bounds = InteractionBounds::new();
                bounds.save(
                    1.0,
                    child_ctx.parent_pos.x,
                    child_ctx.parent_pos.y,
                    0.0,
                    0.0,
                );
                bounds
            },
            event_dispatcher: RefCell::new(aimer_widget::EventDispatcher::new()),
            layout_cache: Default::default(),
        }
        .boxed()
    }
}

impl aimer_widget::PortableWidget for ScrollableFrame {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::{
        BuildContext, OverscrollSource, OverscrollSources, ScrollAxis, ScrollBarPlacement,
        Scrollable, resolved_overscroll_sources, resolved_parent_extent, scroll_storage,
    };
    use crate::controller::ScrollState;
    use aimer_attribute::position::Vec2d;
    use aimer_attribute::size::ResolvedSize;
    use aimer_widget::base::WindowHandle;
    use aimer_widget::{AnyElement, ErrorWidget, Key, State, StatefulWidget, Widget};

    #[test]
    fn i_want_to_know_size_of_it() {
        // eprintln!("size of ScrollableFrame: {}", size_of::<ScrollableFrame>());
        eprintln!("size of Scrollable: {}", size_of::<Scrollable>());
        // eprintln!("size of ScrollableFrame::State: {}", size_of::<ScrollableFrame::State>());
        eprintln!("Size of ScrollState: {}", size_of::<ScrollState>());
    }

    fn assert_stateful_widget<W: StatefulWidget>() {}

    #[test]
    fn a_bounded_parent_uses_its_maximum_extent() {
        assert_eq!(resolved_parent_extent(0.0, 320.0, 240.0), 320.0);
    }

    #[test]
    fn an_unbounded_parent_uses_its_resolved_size() {
        assert_eq!(resolved_parent_extent(0.0, f32::MAX, 240.0), 240.0);
    }

    #[test]
    fn an_unbounded_parent_still_honors_the_minimum_extent() {
        assert_eq!(resolved_parent_extent(320.0, f32::MAX, 240.0), 320.0);
    }

    #[test]
    fn two_unbounded_extents_fall_back_to_the_minimum() {
        assert_eq!(resolved_parent_extent(0.0, f32::MAX, f32::MAX), 0.0);
    }

    #[test]
    fn scrollable_defaults_to_floating_scroll_bars() {
        let scrollable = Scrollable::new();

        assert_eq!(
            scrollable.scroll_bar_placement,
            ScrollBarPlacement::Floating
        );
        assert_eq!(
            scrollable.config.scroll_bar_placement,
            ScrollBarPlacement::Floating
        );
    }

    #[test]
    fn inline_scroll_bar_placement_is_retained_by_state() {
        let state = Scrollable::new()
            .scroll_bar_placement(ScrollBarPlacement::Inline)
            .child(ErrorWidget::new("content"))
            .create_state();

        assert_eq!(state.scroll_bar_placement, ScrollBarPlacement::Inline);
    }

    #[tokio::test]
    async fn an_unbounded_parent_constraint_uses_the_parent_size_for_the_viewport() {
        let mut ctx = context();
        ctx.parent_size = ResolvedSize {
            width: 320.0,
            height: 180.0,
        };
        ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 320.0,
            max_height: f32::MAX,
        };

        let state = Scrollable::new()
            .vertical_scroll_bar(None)
            .horizontal_scroll_bar(None)
            .child(ErrorWidget::new("content"))
            .create_state();
        let element = state.build(&ctx).to_element(&ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 320.0,
                height: 180.0,
            }
        );
    }

    #[tokio::test]
    async fn floating_scrollbars_leave_the_content_viewport_unshrunk() {
        let mut ctx = context();
        ctx.parent_size = ResolvedSize {
            width: 320.0,
            height: 180.0,
        };
        ctx.box_constraint.max_width = 320.0;
        ctx.box_constraint.max_height = 180.0;

        let floating_width = Rc::new(Cell::new(0.0));
        Scrollable::new()
            .child(ConstraintProbe {
                max_width: floating_width.clone(),
            })
            .create_state()
            .build(&ctx)
            .to_element(&ctx);
        assert_eq!(floating_width.get(), 320.0);

        let inline_width = Rc::new(Cell::new(0.0));
        Scrollable::new()
            .scroll_bar_placement(ScrollBarPlacement::Inline)
            .child(ConstraintProbe {
                max_width: inline_width.clone(),
            })
            .create_state()
            .build(&ctx)
            .to_element(&ctx);
        assert_eq!(inline_width.get(), 308.0);
    }

    /// Content that reports how often it was asked for an element.
    struct Probe {
        builds: Rc<Cell<usize>>,
    }

    impl Widget for Probe {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            self.builds.set(self.builds.get() + 1);
            ErrorWidget::new("probe").to_element(ctx)
        }

        fn debug_name(&self) -> &'static str {
            "Probe"
        }
    }

    impl aimer_widget::PortableWidget for Probe {}

    struct ConstraintProbe {
        max_width: Rc<Cell<f32>>,
    }

    impl Widget for ConstraintProbe {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            self.max_width.set(ctx.box_constraint.max_width);
            ErrorWidget::new("constraint probe").to_element(ctx)
        }

        fn debug_name(&self) -> &'static str {
            "ConstraintProbe"
        }
    }

    impl aimer_widget::PortableWidget for ConstraintProbe {}

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

    /// A viewport rebuilds itself on every offset change, and each of those
    /// rebuilds has to reach the scrolling content again — reusing its element,
    /// so a nested scroll offset or text selection survives a scroll frame.
    #[tokio::test]
    async fn every_rebuild_reaches_the_same_content() {
        let builds = Rc::new(Cell::new(0));
        let state = Scrollable::new()
            .child(Probe {
                builds: Rc::clone(&builds),
            })
            .create_state();
        let ctx = context();

        state.build(&ctx).to_element(&ctx);
        state.build(&ctx).to_element(&ctx);

        assert_eq!(
            builds.get(),
            1,
            "scrolling must reuse the content, not rebuild it"
        );
    }

    #[test]
    fn a_native_viewport_bounces_for_every_device() {
        let narrowed = OverscrollSources::NONE.with(OverscrollSource::Touch);

        assert_eq!(
            resolved_overscroll_sources(narrowed, false),
            OverscrollSources::ALL,
            "a native platform reports every gesture, so nothing is held back"
        );
    }

    #[test]
    fn the_browser_clamps_only_the_devices_the_bitmap_leaves_out() {
        let resolved = resolved_overscroll_sources(OverscrollSources::WEB_DEFAULT, true);

        assert!(
            !resolved.contains(OverscrollSource::Wheel),
            "a browser wheel stream never reports the end of its gesture"
        );
        assert!(
            resolved.contains(OverscrollSource::Touch),
            "a touch screen scrolls normally in a browser"
        );
        assert_eq!(
            resolved_overscroll_sources(OverscrollSources::ALL, true),
            OverscrollSources::ALL,
            "an app may hand the wheel its bounce back"
        );
    }

    #[test]
    fn only_the_wheel_is_clamped_on_the_web_by_default() {
        assert_eq!(
            Scrollable::new().web_overscroll,
            OverscrollSources::WEB_DEFAULT
        );
        assert_eq!(
            Scrollable::new()
                .web_overscroll(OverscrollSources::ALL)
                .web_overscroll,
            OverscrollSources::ALL
        );
        assert_eq!(
            Scrollable::new()
                .web_overscroll(OverscrollSource::Touch)
                .web_overscroll,
            OverscrollSources::from(OverscrollSource::Touch),
            "a single source names the set that holds just it"
        );
    }

    #[test]
    fn changing_the_web_bitmap_rebuilds_the_scroll_engine() {
        let current = Scrollable::new().child(ErrorWidget::new("current"));
        let next = Scrollable::new()
            .web_overscroll(OverscrollSources::ALL)
            .child(ErrorWidget::new("next"));
        let mut state = current.create_state();

        state.adopt_config_from(next.create_state());

        assert_eq!(state.web_overscroll, OverscrollSources::ALL);
        assert!(state.refresh_scroll_state.get());
    }

    #[test]
    fn scrollable_uses_the_standard_stateful_widget_lifecycle() {
        assert_stateful_widget::<Scrollable<ErrorWidget>>();
    }

    #[test]
    fn state_adopts_configuration_from_parent_rebuild() {
        let current = Scrollable::new().child(ErrorWidget::new("current"));
        let next = Scrollable::new()
            .axis(ScrollAxis::Horizontal)
            .scroll_bar_placement(ScrollBarPlacement::Inline)
            .vertical_scroll_bar(None)
            .child(ErrorWidget::new("next"));
        let mut state = current.create_state();
        let next_state = next.create_state();

        state.adopt_config_from(next_state);

        assert!(matches!(state.axis, ScrollAxis::Horizontal));
        assert_eq!(state.scroll_bar_placement, ScrollBarPlacement::Inline);
        assert!(state.vertical_scroll_bar.is_none());
        assert!(state.refresh_scroll_state.get());
    }

    #[test]
    fn equivalent_parent_rebuild_keeps_the_live_scroll_engine() {
        // `Scrollable::new` keys by call site, so pin an explicit key to model
        // a parent rebuild that produces the *same* scrollable.
        let current = Scrollable::new()
            .key("equivalent")
            .child(ErrorWidget::new("current"));
        let next = Scrollable::new()
            .key("equivalent")
            .child(ErrorWidget::new("next"));
        let mut state = current.create_state();
        let next_state = next.create_state();

        state.adopt_config_from(next_state);

        assert!(!state.refresh_scroll_state.get());
    }

    // A storage-key switch (e.g. swapping between two category lists) must seed
    // the fresh engine from the new key's saved offset rather than adopting the
    // previous list's live position — otherwise every list shares one offset.
    #[tokio::test]
    async fn a_storage_key_switch_seeds_from_the_new_keys_saved_offset() {
        let ctx = context();

        // List A is live and has been scrolled down.
        let mut state = Scrollable::new()
            .key("list-a")
            .child(ErrorWidget::new("a"))
            .create_state();
        state.build(&ctx).to_element(&ctx);
        state
            .scroll_state
            .borrow()
            .as_ref()
            .unwrap()
            .scroll_offset
            .set(Vec2d { x: 0.0, y: 300.0 });

        // List B has its own remembered position.
        scroll_storage::save_offset(&Key::Value("list-b".into()), Vec2d { x: 0.0, y: 120.0 });

        // Swap to list B.
        state.adopt_config_from(
            Scrollable::new()
                .key("list-b")
                .child(ErrorWidget::new("b"))
                .create_state(),
        );
        state.build(&ctx).to_element(&ctx);

        let engine = state.scroll_state.borrow();
        let engine = engine.as_ref().unwrap();
        assert_eq!(
            engine.scroll_offset.get().y,
            120.0,
            "list B must restore its own offset, not list A's 300"
        );
    }

    fn default_scrollable() -> Scrollable<ErrorWidget> {
        Scrollable::new().child(ErrorWidget::new("content"))
    }

    #[tokio::test]
    async fn a_default_scrollable_does_not_restore_an_old_offset_after_teardown() {
        let ctx = context();
        let first = default_scrollable().create_state();
        scroll_storage::save_offset(&first.key, Vec2d { x: 0.0, y: -48.0 });

        let recreated = default_scrollable().create_state();
        recreated.build(&ctx).to_element(&ctx);

        let engine = recreated.scroll_state.borrow();
        assert_eq!(
            engine.as_ref().unwrap().scroll_offset.get(),
            Vec2d::ZERO,
            "an unkeyed viewport must open at the declared initial offset"
        );
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_layout_tests {
    use aimer_widget::base::BuildContext;
    use aimer_widget::portable::__anteros::{Version, WIDGET_SIZED_BOX, WidgetDocumentView};
    use aimer_widget::portable::{
        PortableBuildContext, PortableLimits, PortableWidgetLimits, PortableWidgetSchema,
        SourceFingerprint, StableId128,
    };
    use aimer_widget::{AnyElement, ErrorWidget, PortableWidget, Widget};

    use super::{ScrollBar, Scrollable};

    struct Leaf;

    impl Widget for Leaf {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            ErrorWidget::new("portable leaf").to_element(ctx)
        }
    }

    impl PortableWidget for Leaf {
        fn to_portable_node(
            self,
            ctx: &mut PortableBuildContext,
            source: SourceFingerprint,
        ) -> Result<
            aimer_widget::portable::PortableNodeId,
            aimer_widget::portable::PortableBuildError,
        > {
            ctx.push_node(WIDGET_SIZED_BOX, Version::new(1, 0), None, source, &[], &[])
        }
    }

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(32, 32, 32, 32, 1_024, 8_192).with_max_blob_bytes(64),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap()
    }

    fn source() -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_bytes([0x23; 16]))
    }

    #[test]
    fn scrollable_lowers_its_retained_child() {
        let mut ctx = context();
        let root = Scrollable::new()
            .vertical_scroll_bar(None)
            .horizontal_scroll_bar(None)
            .child(Leaf)
            .to_portable_node(&mut ctx, source())
            .unwrap();
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        assert_eq!(
            node.widget_type(),
            <Scrollable<Leaf> as PortableWidgetSchema>::SCHEMA
                .widget()
                .id()
        );
        assert_eq!(node.children().collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn scrollbar_lowers_as_a_leaf_portable_widget() {
        let mut ctx = context();
        let root = ScrollBar::default()
            .to_portable_node(&mut ctx, source())
            .unwrap();
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        assert_eq!(
            node.widget_type(),
            <ScrollBar as PortableWidgetSchema>::SCHEMA.widget().id()
        );
        assert_eq!(node.properties().count(), 0);
        assert_eq!(node.children().count(), 0);
    }
}
