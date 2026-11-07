use std::cell::Cell;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_utils::AnimInstant;
use aimer_widget::Key;

use crate::scrollable::overscroll_source::OverscrollSources;
use crate::scrollable::scroll_behavior::ScrollBehavior;
use crate::scrollable::{ScrollAxis, ScrollBarPlacement};

pub use super::bar::BarState;
pub use super::drag_velocity::DragVelocityState;
pub use super::fling::FlingState;
pub use super::overscroll::OverscrollState;
pub use super::scroll_controller::ScrollController;
pub use super::session::SessionState;

/// Minimum logical-pixel movement between two frames for `on_scroll` to fire.
/// Collapses idle frames and sub-pixel jitter to no-ops so the per-frame
/// callback only reports genuine scrolling.
pub(crate) const SCROLL_NOTIFY_EPSILON: f32 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragMode {
    None = 0,
    Content = 1,
    VerticalScrollbar = 2,
    HorizontalScrollbar = 3,
    Pending = 4,
}

impl ScrollState {
    /// Keep scroll state on the axis this engine owns.
    #[inline]
    pub(crate) fn axis_only(axis: ScrollAxis, value: Vec2d) -> Vec2d {
        match axis {
            ScrollAxis::Vertical => Vec2d { x: 0.0, y: value.y },
            ScrollAxis::Horizontal => Vec2d { x: value.x, y: 0.0 },
        }
    }

    pub fn offset(&self) -> Vec2d {
        Self::axis_only(self.axis, self.scroll_offset.get())
    }

    /// Stores a new internal offset and records only actual position changes.
    ///
    /// Tests and state adoption may write the cell directly because they are
    /// establishing a fixture or transferring existing state. Runtime scroll
    /// paths use this boundary so Phase 0 can distinguish an offset update
    /// from a frame that merely redraws an unchanged scrollable.
    #[inline]
    pub(crate) fn set_scroll_offset(&self, offset: Vec2d) {
        let offset = Self::axis_only(self.axis, offset);
        let changed = self.scroll_offset.get() != offset;
        self.scroll_offset.set(offset);
        if changed {
            aimer_widget::record_scroll_offset_update();
            #[cfg(target_arch = "wasm32")]
            // Scrolling changes the transform of an arbitrary child subtree.
            // Its old and new footprints are not available until that subtree
            // is painted, so a browser frame must not preserve a partial
            // compositor target across this event.
            aimer_widget::mark_paint_damage_full();
        }
    }

    /// Request the display-synchronized frame that consumes the latest scroll
    /// state. Keeping this on the live state makes scroll input and momentum
    /// use the same platform-safe scheduling path.
    #[inline]
    pub(crate) fn request_animation_frame(&self) {
        // The only thing this frame has to do is apply the next scroll step, so
        // it says so: the frame loop may then leave alone what the offset does
        // not move.
        aimer_events::window::request_scroll_frame();
    }
}

/// The live scroll engine behind one [`Scrollable`](crate::Scrollable).
///
/// Configuration and the offset itself live here; each interaction concern
/// owns a small state group next to the methods that drive it, so overscroll,
/// momentum, drag velocity, the scroll bar and the scroll session can each be
/// read in one file.
pub struct ScrollState {
    pub(crate) scroll_behavior: ScrollBehavior,
    /// Which input devices this target lets stretch a bouncy edge.
    ///
    /// Resolved once when the engine is built (see
    /// [`Scrollable::overscroll_sources`](crate::Scrollable::overscroll_sources))
    /// and never mutated, so reading it costs a load and a mask.
    pub(crate) overscroll_sources: OverscrollSources,
    pub(crate) axis: ScrollAxis,
    /// Whether the viewport is anchored to the content end; see
    /// [`Scrollable::reverse`](crate::Scrollable::reverse).
    pub(crate) reverse: bool,
    /// Whether the scrollbars overlap the viewport or reserve its edge.
    pub(crate) scroll_bar_placement: ScrollBarPlacement,
    pub(crate) scroll_offset: Cell<Vec2d>,
    /// Identity used for `PageStorage`-style offset persistence when
    /// `remember_scroll_offset` is enabled.
    pub(crate) storage_key: Key,
    /// Whether `storage_key` participates in teardown persistence.
    pub(crate) remember_scroll_offset: bool,
    pub(crate) last_pointer_pos: Cell<Option<Vec2d>>,
    pub(crate) drag_mode: Cell<DragMode>,
    pub(crate) cached_max_scroll: Cell<Vec2d>,
    pub(crate) cached_min_scroll: Cell<Vec2d>,
    pub(crate) pointer_velocity: Cell<Vec2d>,
    pub(crate) last_event_time: Cell<Option<AnimInstant>>,
    pub(crate) last_frame_time: Cell<Option<AnimInstant>>,
    pub(crate) v_scroll_multiplier: Cell<f32>,
    pub(crate) h_scroll_multiplier: Cell<f32>,
    pub(crate) last_scale: Cell<f32>,
    pub(crate) speed_multiplier: f32,
    pub(crate) cursor_pos: Cell<Option<Vec2d>>,
    pub(crate) cached_viewport: Cell<(f32, f32)>,
    pub(crate) key_scroll_strength: f32,
    /// Latest content size observed by the scroll container.
    pub(crate) cached_content_size: Cell<ResolvedSize>,
    /// Whether `cached_content_size` is initialized and can be used as a
    /// previous-frame extent hint while new content is verified.
    pub(crate) cached_content_size_valid: Cell<bool>,
    /// Primary touch finger ID that owns this scrollable, or `None` for mouse.
    /// Set on first PointerDown, cleared on PointerUp/Cancel.
    pub(crate) active_touch_id: Cell<Option<u64>>,
    /// Visual offset the previous frame was drawn with, or `None` before the
    /// first draw.
    ///
    /// Its delta is how far the viewport travelled in one frame, which is what
    /// biases the cache extent toward the direction of travel (see
    /// [`cache_window`](crate::scrollable::cache_extent::cache_window)). Taken
    /// from the drawn offset rather than from
    /// [`pointer_velocity`](Self::pointer_velocity) because every way the
    /// viewport moves — drag, wheel, momentum, bézier fling, spring-back,
    /// `animate_to` — ends up there, while only some of them go through a
    /// velocity.
    pub(crate) last_drawn_offset: Cell<Option<Vec2d>>,
    /// Overscroll holds, the per-gesture stretch peak and device contact.
    pub(crate) overscroll: OverscrollState,
    /// Release fling, programmatic animation and spring-back velocity.
    pub(crate) fling: FlingState,
    /// Drag velocity sampling that feeds the release fling.
    pub(crate) drag_velocity: DragVelocityState,
    /// Scroll bar geometry, hover and drag anchor.
    pub(crate) bar: BarState,
    /// Scroll session latch and the app-facing scroll callbacks.
    pub(crate) session: SessionState,
}

impl ScrollState {
    /// Adopt the live interaction and animation state from a previous scroll
    /// engine.
    ///
    /// Used during reconciliation: when a parent rebuild produces a fresh
    /// scrollable, the newly built controller copies the offset from the old
    /// one so the viewport stays where the user left it instead of snapping
    /// to the top. It also carries the last content extent when the scale is
    /// unchanged, allowing a newly measured prediction to be verified before
    /// it affects the preserved offset.
    pub(crate) fn adopt_scroll_state(&self, prev: &ScrollState) {
        self.scroll_offset.set(self.adopted_offset(prev));
        self.pointer_velocity
            .set(Self::axis_only(self.axis, prev.pointer_velocity.get()));
        self.last_pointer_pos.set(prev.last_pointer_pos.get());
        self.drag_mode.set(prev.drag_mode.get());
        self.last_event_time.set(prev.last_event_time.get());
        self.last_frame_time.set(prev.last_frame_time.get());
        self.cursor_pos.set(prev.cursor_pos.get());
        self.active_touch_id.set(prev.active_touch_id.get());
        self.bar.thumb_hovered.set(prev.bar.thumb_hovered.get());
        self.fling.adopt_from(&prev.fling, self.axis);
        self.drag_velocity.adopt_from(&prev.drag_velocity, self.axis);
        self.session.adopt_from(&prev.session, self.axis);
        self.overscroll.adopt_from(&prev.overscroll, self.axis);
        self.last_drawn_offset.set(
            prev.last_drawn_offset
                .get()
                .map(|offset| Self::axis_only(self.axis, offset)),
        );
        if prev.last_scale.get() == self.last_scale.get() {
            self.cached_content_size.set(prev.cached_content_size.get());
            self.cached_content_size_valid
                .set(prev.cached_content_size_valid.get());
        }
    }

    /// The offset this engine takes over from `prev`.
    ///
    /// A reverse engine adopts a reverse one by its distance from the end, which
    /// is the one quantity both engines agree on: `prev`'s distance is its offset
    /// plus its range, and this engine's offset is that distance minus *its own*
    /// range. An engine that has not laid out yet has a range of zero, so it
    /// holds the distance as the seed form described in
    /// [`end_anchor`](super::end_anchor); one that has laid out gets the real
    /// offset. Both directions happen on every rebuild — the fresh engine takes
    /// over from the old one, which the surviving element then takes back — so
    /// copying the offset across would turn a seed into an overscroll.
    ///
    /// Taking over a forward engine's position would mean something else
    /// entirely, so it starts at the end instead.
    fn adopted_offset(&self, prev: &ScrollState) -> Vec2d {
        let prev_offset = Self::axis_only(self.axis, prev.scroll_offset.get());
        if !self.reverse {
            return prev_offset;
        }
        if !prev.reverse {
            return Vec2d::ZERO;
        }
        let prev_max = prev.cached_max_scroll.get();
        let max = self.cached_max_scroll.get();
        Self::axis_only(
            self.axis,
            Vec2d {
                x: prev_offset.x + prev_max.x - max.x,
                y: prev_offset.y + prev_max.y - max.y,
            },
        )
    }

    /// Whether the edges rubber-band for the device that is currently
    /// scrolling.

    /// Current scroll position in logical (unscaled) pixels, measured from the
    /// content start (positive = scrolled toward the content end), or from the
    /// content end for a reverse engine.
    ///
    /// Internally the offset is stored scaled and negated (content moved up),
    /// so this converts back to the user-facing convention.
    pub(super) fn logical_offset(&self) -> Vec2d {
        self.logical_position(self.scroll_offset.get())
    }
}

#[cfg(test)]
impl ScrollState {
    /// A default vertical engine parked at `offset`, shared by the tests of
    /// every module in this crate.
    pub(crate) fn for_test_at(offset: Vec2d) -> Self {
        use aimer_macro::key;

        Self {
            key_scroll_strength: crate::scrollable::constants::KEYBOARD_SCROLL_STEP,
            scroll_behavior: ScrollBehavior::default(),
            overscroll_sources: OverscrollSources::ALL,
            axis: ScrollAxis::Vertical,
            reverse: false,
            scroll_bar_placement: ScrollBarPlacement::Floating,
            overscroll: Default::default(),
            fling: Default::default(),
            drag_velocity: Default::default(),
            bar: Default::default(),
            session: Default::default(),
            scroll_offset: Cell::new(offset),
            storage_key: key!(),
            remember_scroll_offset: false,
            last_pointer_pos: Cell::new(None),
            drag_mode: Cell::new(DragMode::None),
            cached_max_scroll: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            cached_min_scroll: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            pointer_velocity: Cell::new(Vec2d { x: 0.0, y: 0.0 }),
            last_event_time: Cell::new(None),
            last_frame_time: Cell::new(None),
            v_scroll_multiplier: Cell::new(0.0),
            h_scroll_multiplier: Cell::new(0.0),
            last_scale: Cell::new(1.0),
            speed_multiplier: 1.0,
            cursor_pos: Cell::new(None),
            cached_viewport: Cell::new((0.0, 0.0)),
            cached_content_size: Cell::new(Default::default()),
            cached_content_size_valid: Cell::new(false),
            active_touch_id: Cell::new(None),
            last_drawn_offset: Cell::new(None),
        }
    }
}
