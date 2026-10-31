use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use aimer_attribute::dimension::Dimension;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd_v2::Rect;
use aimer_cupid::utilities::Color as V2Color;
use aimer_input::gesture::GestureEvent;
use aimer_input::gesture::gesture_detector::GestureDetector;
use aimer_input::mouse_region::MouseRegion;
use aimer_widget::base::{BuildContext, Color, Colors};
use aimer_widget::portable::__anteros::{WidgetDocumentView, WidgetNodeView};
use aimer_widget::portable::PortableMaterializeError;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement, Rebuildable, State,
    StateUpdater, StatefulElement, StatefulWidget, VisitorElement, Widget,
};

use crate::ScrollAxis;
use super::thumb_layout::{
    Fade, MOBILE, growth_log_line, rest_strip_width, thumb_cross_layout, touch_strip_width, widened_strip,
};
use crate::scrollable::constants::{
    SCROLLBAR_HIDE_DURATION_MS, SCROLLBAR_IDLE_HOLD_MS, SCROLLBAR_SHOW_DURATION_MS,
    SCROLLBAR_THUMB_GROW_MS, SCROLLBAR_TRACK_FADE_MS,
};
use crate::scrollable::controller::{DragMode, ScrollState};

/// Controls whether a scroll bar overlays the content or occupies layout
/// space beside it.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, aimer_macro::PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_scroll::scrollable::ScrollBarPlacement",
    max_encoded_bytes = 16
)]
pub enum ScrollBarPlacement {
    /// Draws the scroll bar over the content at the viewport edge.
    #[default]
    #[portable_value(tag = 0)]
    Floating,
    /// Reserves the scroll bar's width or height outside the content viewport.
    #[portable_value(tag = 1)]
    Inline,
}

impl ScrollBarPlacement {
    #[inline]
    pub(crate) const fn reserves_space(self) -> bool {
        matches!(self, Self::Inline)
    }

    #[inline]
    pub(crate) fn cross_offset(self, viewport_extent: f32, bar_extent: f32) -> f32 {
        match self {
            Self::Floating => (viewport_extent - bar_extent.max(0.0)).max(0.0),
            Self::Inline => viewport_extent,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ScrollTrack {
    pub width: Dimension,
    pub color: Colors,
    pub hover_color: Colors,
}

impl Default for ScrollTrack {
    fn default() -> Self {
        Self {
            width: Dimension::Auto,
            color: Colors::Transparent,
            // The track shows up with the bar: hovering the strip lights it, and
            // a transparent default would make that invisible.
            hover_color: Colors::Rgba(128, 128, 128, 40),
        }
    }
}

#[derive(Clone, Copy)]
pub struct ScrollThumb {
    pub width: Dimension,
    pub radius: Dimension,
    pub color: Colors,
    pub hover_color: Colors,
    pub active_color: Colors,
}

impl Default for ScrollThumb {
    fn default() -> Self {
        Self {
            width: Dimension::Auto,
            radius: Dimension::Px(4.0),
            color: Colors::Rgba(150, 150, 150, 150),
            hover_color: Colors::Rgba(100, 100, 100, 200),
            active_color: Colors::Rgba(80, 80, 80, 255),
        }
    }
}

#[derive(Clone, Copy)]
pub struct ScrollButton {
    pub width: Dimension,
    pub height: Dimension,

    pub color: Colors,
    pub hover_color: Colors,
    pub active_color: Colors,
}

#[derive(Clone, Default, aimer_macro::PortableWidget)]
#[portable_widget(
    id = "aimer_scroll::scrollable::ScrollBar",
    materializer = materialize_portable_scroll_bar
)]
pub struct ScrollBar {
    #[portable_skip]
    pub track: ScrollTrack,
    #[portable_skip]
    pub thumb: ScrollThumb,
    #[portable_skip]
    pub up_button: Option<ScrollButton>,
    #[portable_skip]
    pub down_button: Option<ScrollButton>,
    #[portable_skip]
    ctrl: Option<Rc<ScrollState>>,
    #[portable_skip]
    axis: ScrollAxis,
}

fn materialize_portable_scroll_bar(
    _document: &WidgetDocumentView<'_>,
    _node: WidgetNodeView<'_>,
    children: Vec<AnyWidget>,
) -> Result<AnyWidget, PortableMaterializeError> {
    if !children.is_empty() {
        return Err(PortableMaterializeError::InvalidChildCount {
            expected: 0,
            actual: children.len(),
        });
    }
    Ok(ScrollBar::default().boxed())
}

impl ScrollBar {
    #[inline]
    pub(crate) fn for_scrollable(mut self, ctrl: Rc<ScrollState>, axis: ScrollAxis) -> Self {
        self.ctrl = Some(ctrl);
        self.axis = axis;
        self
    }
}

/// Persistent state for a [`ScrollBar`].
///
/// The runtime opacity belongs to this state rather than to the scrollable's
/// paint pass, so rebuilding the parent does not reset a scrollbar midway
/// through its show or hide transition.
pub struct ScrollBarState {
    config: ScrollBar,
    runtime: Rc<ScrollBarRuntime>,
}

impl StatefulWidget for ScrollBar {
    type State = ScrollBarState;

    fn create_state(self) -> Self::State {
        ScrollBarState {
            config: self,
            runtime: Rc::new(ScrollBarRuntime::default()),
        }
    }
}

impl Widget for ScrollBar {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ScrollBar", None)
            .0
            .boxed()
    }
}

impl State<ScrollBar> for ScrollBarState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        self.config = new.config;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        // The region covers exactly the strip this bar is laid out as, so
        // hovering it means the pointer is over the bar and not merely over
        // the content it floats above. A standalone bar has no scroll state to
        // report to and keeps a private cell.
        let pointer = self
            .config
            .ctrl
            .as_ref()
            .map(|ctrl| ctrl.bar.pointer.clone())
            .unwrap_or_default();
        // Presses and drags on the strip are recognised by a detector, and what
        // they mean for the bar is decided by the scroll state.
        let ctrl = self.config.ctrl.clone();
        MouseRegion::new().current_state(pointer).child(
            GestureDetector::new()
                .on_gesture(move |event: GestureEvent| {
                    if let Some(ctrl) = &ctrl {
                        ctrl.on_bar_gesture(event);
                    }
                })
                .child(RawScrollBar {
                    config: self.config.clone(),
                    ctrl: self.config.ctrl.clone(),
                    runtime: self.runtime.clone(),
                }),
        )
    }
}

#[derive(Default)]
struct ScrollBarRuntime {
    alpha: Cell<f32>,
    last_activity: Cell<Option<aimer_utils::AnimInstant>>,
    last_offset: Cell<Option<Vec2d>>,
    showing: Cell<bool>,
    /// Whether something held the bar the last time it was asked: a pointer on
    /// it, or a drag. The idle delay starts when this lets go.
    held: Cell<bool>,
    transition_start: Cell<Option<aimer_utils::AnimInstant>>,
    transition_from: Cell<f32>,
    /// How far the track has faded from its resting color toward its hover
    /// color, `0.0..=1.0`.
    track: Fade,
    /// How far the thumb has grown toward its held width, `0.0..=1.0`.
    grab: Fade,
    /// What the growth was when it was last logged, so only changes are.
    grab_logged: Cell<f32>,
    v2_paint_snapshot: Cell<Option<ScrollBarPaint>>,
}

impl ScrollBarRuntime {
    /// Advances the bar to `now` and returns its opacity.
    ///
    /// `active` is a scroll in progress. `held` is something that keeps the bar
    /// for as long as it lasts, a pointer resting on it or a drag: the bar
    /// stays fully shown, and the idle delay before it fades only starts when
    /// it lets go.
    fn update(
        &self,
        offset: Vec2d,
        active: bool,
        held: bool,
        now: aimer_utils::AnimInstant,
    ) -> f32 {
        let moved = self
            .last_offset
            .replace(Some(offset))
            .is_some_and(|previous| {
                (previous.x - offset.x).abs() > 0.01 || (previous.y - offset.y).abs() > 0.01
            });

        let was_held = self.held.replace(held);
        let activity = active || moved || held;
        if activity || was_held {
            self.last_activity.set(Some(now));
        }

        let mut alpha = self.advance_transition(now);
        if activity {
            if !self.showing.get() {
                self.begin_transition(now, true, alpha);
                alpha = self.advance_transition(now);
            }
        } else if self.showing.get()
            && self.last_activity.get().is_some_and(|last| {
                now.duration_since(last) > Duration::from_millis(SCROLLBAR_IDLE_HOLD_MS)
            })
        {
            self.begin_transition(now, false, alpha);
            alpha = self.advance_transition(now);
        }

        self.alpha.set(alpha);
        alpha
    }

    /// Advances the track fade to `now` and returns how far toward its hover
    /// color the track is.
    ///
    /// The fade moves at a fixed rate toward `hovered`, so a pointer that
    /// leaves halfway through turns it around from where it is instead of
    /// restarting it.
    ///
    /// Time only counts toward the fade when the previous call left it
    /// unfinished. A settled track is not painted again until something changes,
    /// so the gap before the frame that changes it is rest, not fade time;
    /// counting it would jump the track straight to its destination. That frame
    /// starts the fade, and the ones that follow advance it.
    fn track_mix(&self, hovered: bool, now: aimer_utils::AnimInstant) -> f32 {
        self.track.advance(hovered, now, SCROLLBAR_TRACK_FADE_MS)
    }

    /// Whether the track is still between its resting and hover colors, which
    /// only a frame can finish.
    fn track_fading(&self, hovered: bool) -> bool {
        self.track.unfinished(hovered)
    }

    /// Advances the thumb's growth to `now` and returns how far it has grown:
    /// `0.0` at its resting width, `1.0` at its held width.
    fn grab_mix(&self, held: bool, now: aimer_utils::AnimInstant) -> f32 {
        self.grab.advance(held, now, SCROLLBAR_THUMB_GROW_MS)
    }

    /// Logs the thumb's growth for this frame if it changed since the last line.
    fn log_growth(&self, grabbed: bool, width_dp: f32) {
        let mix = self.grab.value();
        let previous = self.grab_logged.replace(mix);
        if let Some(line) = growth_log_line(grabbed, mix, previous, width_dp) {
            thumb_log!("{line}");
        }
    }

    /// Whether the thumb is still growing or shrinking, which only a frame can
    /// finish.
    fn grab_changing(&self, held: bool) -> bool {
        self.grab.unfinished(held)
    }

    /// Whether the bar is waiting out its idle delay, which only a frame can
    /// end: nothing else will happen to wake it.
    fn counting_down(&self) -> bool {
        self.showing.get() && !self.held.get()
    }

    fn begin_transition(&self, now: aimer_utils::AnimInstant, showing: bool, from: f32) {
        self.showing.set(showing);
        self.transition_from.set(from);
        self.transition_start.set(Some(now));
    }

    fn advance_transition(&self, now: aimer_utils::AnimInstant) -> f32 {
        let Some(start) = self.transition_start.get() else {
            return self.alpha.get();
        };

        let duration = if self.showing.get() {
            Duration::from_millis(SCROLLBAR_SHOW_DURATION_MS)
        } else {
            Duration::from_millis(SCROLLBAR_HIDE_DURATION_MS)
        };
        let progress =
            (now.duration_since(start).as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
        let from = self.transition_from.get();
        let target = if self.showing.get() { 1.0 } else { 0.0 };
        let alpha = from + (target - from) * progress;

        if progress >= 1.0 {
            self.transition_start.set(None);
            self.alpha.set(if self.showing.get() { 1.0 } else { 0.0 });
        }
        alpha
    }

    fn hide(&self) {
        self.alpha.set(0.0);
        self.last_activity.set(None);
        self.showing.set(false);
        self.held.set(false);
        self.transition_start.set(None);
        self.transition_from.set(0.0);
        self.grab.reset();
    }
}

pub(crate) fn reserved_viewport(
    axis: ScrollAxis,
    width: f32,
    height: f32,
    bar_extent: f32,
    placement: ScrollBarPlacement,
) -> (f32, f32) {
    let bar_extent = if placement.reserves_space() {
        bar_extent.max(0.0)
    } else {
        0.0
    };
    match axis {
        ScrollAxis::Vertical => ((width - bar_extent).max(0.0), height),
        ScrollAxis::Horizontal => (width, (height - bar_extent).max(0.0)),
    }
}

pub(crate) fn track_width(
    scroll_bar: &ScrollBar,
    cross_extent: f32,
    scale: f32,
    placement: ScrollBarPlacement,
) -> f32 {
    match scroll_bar.track.width {
        Dimension::Px(value) => value * scale,
        Dimension::Percent(percent) => cross_extent * (percent / 100.0),
        Dimension::Auto => {
            if MOBILE {
                // The room a held thumb grows into (see `thumb_layout`).
                touch_strip_width(placement, scale)
            } else {
                12.0 * scale
            }
        }
    }
}

struct RawScrollBar {
    config: ScrollBar,
    ctrl: Option<Rc<ScrollState>>,
    runtime: Rc<ScrollBarRuntime>,
}

impl Widget for RawScrollBar {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }
}

impl aimer_widget::PortableWidget for RawScrollBar {}

impl EventElement for RawScrollBar {}
impl Rebuildable for RawScrollBar {}

impl VisitorElement for RawScrollBar {
    fn debug_name(&self) -> &'static str {
        "RawScrollBar"
    }
}

impl LayoutElement for RawScrollBar {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize {
            width: ctx.box_constraint.max_width,
            height: ctx.box_constraint.max_height,
        }
    }
}

impl Drawable for RawScrollBar {
    fn update(&self, ctx: &BuildContext) {
        let Some(ctrl) = self.ctrl.as_ref() else {
            return;
        };
        // The bar is painted by `paint_local_v2`. Preparing its geometry here
        // publishes the thumb and track metrics hit testing reads.
        let paint = prepare_scrollbar_paint(ctx, ctrl, &self.config, self.runtime.as_ref());
        if let Some(paint) = paint {
            // Once per frame, here: painting prepares the same geometry again.
            self.runtime
                .log_growth(ctrl.thumb_armed(), paint.thumb_size.width / ctx.scale);
        }
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.ctrl
            .as_ref()
            .is_none_or(|ctrl| scrollbar_geometry(ctx, ctrl, &self.config, 0.0).is_ok())
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = Canvas::of(ctx);
        let paint = self.ctrl.as_ref().and_then(|ctrl| {
            prepare_scrollbar_paint(ctx, ctrl, &self.config, self.runtime.as_ref())
        });
        if let Some(paint) = paint {
            paint_scrollbar_local_v2(&canvas, ctx.scale, paint);
        }
        canvas.finish();
        self.runtime.v2_paint_snapshot.set(paint);
    }

    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        let paint = self.ctrl.as_ref().and_then(|ctrl| {
            prepare_scrollbar_paint(ctx, ctrl, &self.config, self.runtime.as_ref())
        });
        self.runtime.v2_paint_snapshot.get() != paint
    }
}

#[derive(Clone, Copy, PartialEq)]
struct ScrollBarButtonPaint {
    pos: Vec2d,
    size: ResolvedSize,
    color: Color,
}

#[derive(Clone, Copy, PartialEq)]
struct ScrollBarPaint {
    alpha: f32,
    track_width: f32,
    track_length: f32,
    track_color: Color,
    up_button: Option<ScrollBarButtonPaint>,
    down_button: Option<ScrollBarButtonPaint>,
    thumb_pos: Vec2d,
    thumb_size: ResolvedSize,
    thumb_radius: f32,
    thumb_color: Color,
    hit_rect: (f32, f32, f32, f32),
    multiplier: f32,
    is_vertical: bool,
}

fn resolve_button_extent(
    button: &ScrollButton,
    is_vertical: bool,
    track_width: f32,
    track_length: f32,
    scale: f32,
) -> f32 {
    let dimension = if is_vertical {
        button.height
    } else {
        button.width
    };
    match dimension {
        Dimension::Px(value) => value * scale,
        Dimension::Percent(percent) => track_length * (percent / 100.0),
        Dimension::Auto => track_width,
    }
}

/// `grab_mix` is how far the thumb has grown toward its held width, `0.0..=1.0`.
fn scrollbar_geometry(
    ctx: &BuildContext,
    ctrl: &ScrollState,
    scroll_bar: &ScrollBar,
    grab_mix: f32,
) -> Result<Option<ScrollBarPaint>, ()> {
    let scale = ctx.scale;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(());
    }
    let (viewport_w, viewport_h) = ctrl.cached_viewport.get();
    let offset = ctrl.visual_offset(ctrl.scroll_offset.get());
    let is_vertical = matches!(ctrl.axis, ScrollAxis::Vertical);
    let track_width = track_width(
        scroll_bar,
        if is_vertical { viewport_w } else { viewport_h },
        scale,
        ctrl.scroll_bar_placement,
    );
    let content_size = ctrl.cached_content_size.get();
    let (track_length, content_extent, scroll_pos) = if is_vertical {
        (viewport_h, content_size.height, -offset.y)
    } else {
        (viewport_w, content_size.width, -offset.x)
    };
    if [
        viewport_w,
        viewport_h,
        track_width,
        track_length,
        content_extent,
        scroll_pos,
    ]
    .iter()
    .any(|value| !value.is_finite())
        || viewport_w < 0.0
        || viewport_h < 0.0
        || track_width < 0.0
        || track_length < 0.0
        || content_extent < 0.0
    {
        return Err(());
    }
    if content_extent <= track_length + 0.01 {
        return Ok(None);
    }

    // On a touch device the strip is wider than the thumb is at rest, so the
    // resting thumb is sized from the strip it would have had, and only a held
    // thumb uses the extra room.
    let widened = widened_strip(MOBILE, scroll_bar, ctrl.scroll_bar_placement);
    let sizing_strip = rest_strip_width(widened, track_width, scale);
    let rest_thumb_width = match scroll_bar.thumb.width {
        Dimension::Px(value) => value * scale,
        Dimension::Percent(percent) => sizing_strip * (percent / 100.0),
        Dimension::Auto => (sizing_strip * 0.6).max(4.0),
    };
    let (thumb_width, thumb_cross_offset) =
        thumb_cross_layout(track_width, rest_thumb_width, grab_mix, widened, scale);
    let button_extent = (
        scroll_bar
            .up_button
            .as_ref()
            .map(|button| {
                resolve_button_extent(button, is_vertical, track_width, track_length, scale)
            })
            .unwrap_or(0.0),
        scroll_bar
            .down_button
            .as_ref()
            .map(|button| {
                resolve_button_extent(button, is_vertical, track_width, track_length, scale)
            })
            .unwrap_or(0.0),
    );
    let thumb_radius = match scroll_bar.thumb.radius {
        Dimension::Px(value) => value * scale,
        Dimension::Percent(percent) => thumb_width * (percent / 100.0),
        Dimension::Auto => thumb_width / 2.0,
    };
    if [thumb_width, button_extent.0, button_extent.1, thumb_radius]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(());
    }

    let usable_track = (track_length - button_extent.0 - button_extent.1).max(0.0);
    let thumb_ratio = (track_length / content_extent).min(1.0);
    let thumb_length = (usable_track * thumb_ratio).max(20.0 * scale);
    let max_thumb_move = (usable_track - thumb_length).max(0.0);
    let max_scroll = (content_extent - track_length).max(0.0);
    let multiplier = if max_thumb_move > 0.0 {
        max_scroll / max_thumb_move
    } else {
        0.0
    };
    let scroll_ratio = if max_scroll > 0.0 {
        scroll_pos.clamp(0.0, max_scroll) / max_scroll
    } else {
        0.0
    };
    let thumb_offset = button_extent.0 + scroll_ratio * max_thumb_move;
    let cross_offset = ctrl.scroll_bar_placement.cross_offset(
        if is_vertical { viewport_w } else { viewport_h },
        track_width,
    );
    let is_active = if is_vertical {
        ctrl.drag_mode.get() == DragMode::VerticalScrollbar
    } else {
        ctrl.drag_mode.get() == DragMode::HorizontalScrollbar
    };
    let is_hover = ctrl.cursor_pos.get().is_some_and(|cursor| {
        if is_vertical {
            ctrl.hit_test_v_thumb(cursor)
        } else {
            ctrl.hit_test_h_thumb(cursor)
        }
    });
    // The resting track. Hover is blended in by `prepare_scrollbar_paint`, which
    // owns the fade.
    let track_color: Color = scroll_bar.track.color.into();
    let thumb_color: Color = if is_active {
        scroll_bar.thumb.active_color.into()
    } else if is_hover {
        scroll_bar.thumb.hover_color.into()
    } else {
        scroll_bar.thumb.color.into()
    };
    let (thumb_pos, thumb_size, hit_rect) = if is_vertical {
        (
            Vec2d {
                x: thumb_cross_offset,
                y: thumb_offset,
            },
            ResolvedSize {
                width: thumb_width,
                height: thumb_length,
            },
            (
                cross_offset + thumb_cross_offset,
                thumb_offset,
                thumb_width,
                thumb_length,
            ),
        )
    } else {
        (
            Vec2d {
                x: thumb_offset,
                y: thumb_cross_offset,
            },
            ResolvedSize {
                width: thumb_length,
                height: thumb_width,
            },
            (
                thumb_offset,
                cross_offset + thumb_cross_offset,
                thumb_length,
                thumb_width,
            ),
        )
    };
    let button_paint = |button: Option<&ScrollButton>, start: bool| {
        button.map(|button| {
            let extent = if start {
                button_extent.0
            } else {
                button_extent.1
            };
            let (pos, size) = if is_vertical {
                (
                    if start {
                        Vec2d::ZERO
                    } else {
                        Vec2d {
                            x: 0.0,
                            y: track_length - extent,
                        }
                    },
                    ResolvedSize {
                        width: track_width,
                        height: extent,
                    },
                )
            } else {
                (
                    if start {
                        Vec2d::ZERO
                    } else {
                        Vec2d {
                            x: track_length - extent,
                            y: 0.0,
                        }
                    },
                    ResolvedSize {
                        width: extent,
                        height: track_width,
                    },
                )
            };
            ScrollBarButtonPaint {
                pos,
                size,
                color: button.color.into(),
            }
        })
    };
    let up_button = button_paint(scroll_bar.up_button.as_ref(), true);
    let down_button = button_paint(scroll_bar.down_button.as_ref(), false);
    let geometry_values = [
        usable_track,
        thumb_ratio,
        thumb_length,
        max_thumb_move,
        max_scroll,
        multiplier,
        scroll_ratio,
        thumb_offset,
        thumb_cross_offset,
        cross_offset,
        thumb_pos.x,
        thumb_pos.y,
        thumb_size.width,
        thumb_size.height,
        hit_rect.0,
        hit_rect.1,
        hit_rect.2,
        hit_rect.3,
    ];
    if geometry_values
        .iter()
        .any(|value| !value.is_finite() || !(value / scale).is_finite())
        || [up_button, down_button]
            .into_iter()
            .flatten()
            .flat_map(|button| {
                [
                    button.pos.x,
                    button.pos.y,
                    button.size.width,
                    button.size.height,
                ]
            })
            .any(|value| !value.is_finite() || !(value / scale).is_finite())
    {
        return Err(());
    }

    Ok(Some(ScrollBarPaint {
        alpha: 1.0,
        track_width,
        track_length,
        track_color,
        up_button,
        down_button,
        thumb_pos,
        thumb_size,
        thumb_radius,
        thumb_color,
        hit_rect,
        multiplier,
        is_vertical,
    }))
}

fn prepare_scrollbar_paint(
    ctx: &BuildContext,
    ctrl: &ScrollState,
    scroll_bar: &ScrollBar,
    runtime: &ScrollBarRuntime,
) -> Option<ScrollBarPaint> {
    let now = aimer_utils::AnimInstant::now();
    // A finger that has taken the thumb makes it grow, and letting go shrinks it
    // back; both are frames' work.
    let grabbed = ctrl.thumb_armed();
    let grab_mix = runtime.grab_mix(grabbed, now);
    if runtime.grab_changing(grabbed) {
        ctrl.request_animation_frame();
    }
    let paint = match scrollbar_geometry(ctx, ctrl, scroll_bar, grab_mix) {
        Ok(Some(paint)) => paint,
        Ok(None) => {
            runtime.hide();
            ctrl.bar.opacity.set(0.0);
            if matches!(ctrl.axis, ScrollAxis::Vertical) {
                ctrl.bar.v_thumb_rect.set(None);
            } else {
                ctrl.bar.h_thumb_rect.set(None);
            }
            return None;
        }
        Err(()) => return None,
    };
    // The geometry is published whether or not the bar is showing: a pointer
    // reaching for a hidden bar has to find it where it is, and that is what
    // shows it.
    if paint.is_vertical {
        ctrl.bar.v_track_width.set(paint.track_width);
        ctrl.v_scroll_multiplier.set(paint.multiplier);
        ctrl.bar.v_thumb_rect.set(Some(paint.hit_rect));
    } else {
        ctrl.bar.h_track_width.set(paint.track_width);
        ctrl.h_scroll_multiplier.set(paint.multiplier);
        ctrl.bar.h_thumb_rect.set(Some(paint.hit_rect));
    }

    let dragging = matches!(
        ctrl.drag_mode.get(),
        DragMode::VerticalScrollbar | DragMode::HorizontalScrollbar
    );
    let alpha = runtime.update(
        ctrl.scroll_offset.get(),
        ctrl.session.active.get(),
        dragging || ctrl.scroll_bar_hovered() || ctrl.thumb_holding(),
        aimer_utils::AnimInstant::now(),
    );
    ctrl.bar.opacity.set(alpha);
    // The strip lights the track whether or not the pointer is on the thumb, and
    // a thumb drag keeps it lit however far the pointer strays from the strip.
    let track_hovered = dragging || ctrl.scroll_bar_hovered();
    // Nothing is painted for the track on a touch device, so there is no fade to
    // advance and no frame to ask for.
    let track_mix = if MOBILE {
        0.0
    } else {
        runtime.track_mix(track_hovered, aimer_utils::AnimInstant::now())
    };
    let track_color = track_paint_color(MOBILE, &scroll_bar.track, track_mix);
    if !MOBILE && runtime.track_fading(track_hovered) {
        ctrl.request_animation_frame();
    }
    // A transition, and the idle delay before the fade, end only on a frame
    // that nothing else is going to ask for.
    if runtime.transition_start.get().is_some() || runtime.counting_down() {
        ctrl.request_animation_frame();
    }
    if alpha <= 0.0 {
        return None;
    }

    Some(ScrollBarPaint {
        alpha,
        track_color,
        ..paint
    })
}

/// The color the track is painted with, `mix` of the way from its resting color
/// to its hover color.
///
/// A touch device shows no track: with no pointer to hover it, the track only
/// lit up under a held thumb, as a strip the finger was already covering. The
/// thumb alone is the bar there, whatever colors the track was given.
fn track_paint_color(mobile: bool, track: &ScrollTrack, mix: f32) -> Color {
    if mobile {
        Colors::Rgba(0, 0, 0, 0).into()
    } else {
        blend_colors(track.color, track.hover_color, mix)
    }
}

/// Blends `from` into `to` by `t`, weighting each color by its alpha so a fade
/// out of transparent takes on the target's hue instead of passing through
/// black.
fn blend_colors(from: Colors, to: Colors, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let (fr, fg, fb, fa) = Color::from(from).to_rgba();
    let (tr, tg, tb, ta) = Color::from(to).to_rgba();
    let from_weight = f32::from(fa) * (1.0 - t);
    let to_weight = f32::from(ta) * t;
    let alpha = from_weight + to_weight;
    if alpha <= 0.0 {
        return Colors::Rgba(0, 0, 0, 0).into();
    }
    let channel = |from: u8, to: u8| {
        ((f32::from(from) * from_weight + f32::from(to) * to_weight) / alpha).round() as u8
    };
    Colors::Rgba(
        channel(fr, tr),
        channel(fg, tg),
        channel(fb, tb),
        alpha.round() as u8,
    )
    .into()
}

fn local_rect(pos: Vec2d, size: ResolvedSize, scale: f32) -> Rect {
    Rect::new(
        pos.x / scale,
        pos.y / scale,
        size.width / scale,
        size.height / scale,
    )
}

fn paint_scrollbar_local_v2(canvas: &Canvas, scale: f32, paint: ScrollBarPaint) {
    let track_size = if paint.is_vertical {
        ResolvedSize {
            width: paint.track_width,
            height: paint.track_length,
        }
    } else {
        ResolvedSize {
            width: paint.track_length,
            height: paint.track_width,
        }
    };
    canvas.set_alpha(paint.alpha);
    let (track_red, track_green, track_blue, track_alpha) = paint.track_color.to_rgba();
    canvas.fill_rect(
        local_rect(Vec2d::ZERO, track_size, scale),
        [track_red, track_green, track_blue, track_alpha],
    );
    for button in [paint.up_button, paint.down_button].into_iter().flatten() {
        let (red, green, blue, alpha) = button.color.to_rgba();
        canvas.fill_rect(
            local_rect(button.pos, button.size, scale),
            [red, green, blue, alpha],
        );
    }
    let (thumb_red, thumb_green, thumb_blue, thumb_alpha) = paint.thumb_color.to_rgba();
    canvas.fill_rect_styled(
        local_rect(paint.thumb_pos, paint.thumb_size, scale),
        V2Color::rgba8(thumb_red, thumb_green, thumb_blue, thumb_alpha),
        [paint.thumb_radius / scale; 4],
        [0.0; 4],
        V2Color::transparent(),
        [0.0; 4],
        V2Color::transparent(),
    );
    canvas.restore_alpha();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepts_stateful_widget<W: StatefulWidget>() {}

    #[test]
    fn scrollbar_is_a_stateful_widget() {
        accepts_stateful_widget::<ScrollBar>();
        let _ = ScrollBar::default().create_state();
    }

    #[test]
    fn scrolling_scrollbar_fades_in_instead_of_jumping_to_full_opacity() {
        let runtime = ScrollBarRuntime::default();
        let start = aimer_utils::AnimInstant::now();

        assert_eq!(runtime.update(Vec2d::ZERO, true, false, start), 0.0);

        let halfway = runtime.update(
            Vec2d::ZERO,
            true,
            false,
            start + Duration::from_millis(SCROLLBAR_SHOW_DURATION_MS / 2),
        );
        assert!(halfway > 0.0 && halfway < 1.0);
        assert_eq!(
            runtime.update(
                Vec2d::ZERO,
                true,
                false,
                start + Duration::from_millis(SCROLLBAR_SHOW_DURATION_MS),
            ),
            1.0
        );
    }

    #[test]
    fn an_idle_scrollbar_fades_out_after_the_idle_delay() {
        let runtime = ScrollBarRuntime::default();
        let start = aimer_utils::AnimInstant::now();
        let fully_visible = start + Duration::from_millis(SCROLLBAR_SHOW_DURATION_MS);

        assert_eq!(runtime.update(Vec2d::ZERO, true, false, start), 0.0);
        assert_eq!(runtime.update(Vec2d::ZERO, true, false, fully_visible), 1.0);

        // Short of the delay it is still there, however long a fade would take.
        let almost = fully_visible + Duration::from_millis(SCROLLBAR_IDLE_HOLD_MS - 1);
        assert_eq!(runtime.update(Vec2d::ZERO, false, false, almost), 1.0);

        let fade_start = fully_visible + Duration::from_millis(SCROLLBAR_IDLE_HOLD_MS + 1);
        assert_eq!(runtime.update(Vec2d::ZERO, false, false, fade_start), 1.0);
        assert_eq!(
            runtime.update(
                Vec2d::ZERO,
                false,
                false,
                fade_start + Duration::from_millis(SCROLLBAR_HIDE_DURATION_MS / 2),
            ),
            0.5
        );
        assert_eq!(
            runtime.update(
                Vec2d::ZERO,
                false,
                false,
                fade_start + Duration::from_millis(SCROLLBAR_HIDE_DURATION_MS),
            ),
            0.0
        );
    }

    /// A pointer resting on the bar keeps it for as long as it rests there, and
    /// the idle delay counts from when it lets go, not from when it arrived.
    #[test]
    fn a_held_scrollbar_waits_out_the_idle_delay_from_the_release() {
        let runtime = ScrollBarRuntime::default();
        let start = aimer_utils::AnimInstant::now();
        let shown = start + Duration::from_millis(SCROLLBAR_SHOW_DURATION_MS);
        runtime.update(Vec2d::ZERO, false, true, start);
        assert_eq!(runtime.update(Vec2d::ZERO, false, true, shown), 1.0);

        // Held for far longer than the delay, with no frame in between.
        let released = shown + Duration::from_millis(10 * SCROLLBAR_IDLE_HOLD_MS);
        assert_eq!(runtime.update(Vec2d::ZERO, false, false, released), 1.0);
        assert!(runtime.counting_down());

        let almost = released + Duration::from_millis(SCROLLBAR_IDLE_HOLD_MS - 1);
        assert_eq!(runtime.update(Vec2d::ZERO, false, false, almost), 1.0);
        let gone = released + Duration::from_millis(SCROLLBAR_IDLE_HOLD_MS + SCROLLBAR_HIDE_DURATION_MS + 2);
        runtime.update(Vec2d::ZERO, false, false, released + Duration::from_millis(SCROLLBAR_IDLE_HOLD_MS + 1));
        assert_eq!(runtime.update(Vec2d::ZERO, false, false, gone), 0.0);
    }

    #[test]
    fn vertical_scrollbar_reserves_width_from_the_content_viewport() {
        assert_eq!(
            reserved_viewport(
                ScrollAxis::Vertical,
                320.0,
                500.0,
                12.0,
                ScrollBarPlacement::Inline,
            ),
            (308.0, 500.0)
        );
    }

    #[test]
    fn horizontal_scrollbar_reserves_height_from_the_content_viewport() {
        assert_eq!(
            reserved_viewport(
                ScrollAxis::Horizontal,
                320.0,
                500.0,
                12.0,
                ScrollBarPlacement::Inline,
            ),
            (320.0, 488.0)
        );
    }

    #[test]
    fn floating_scrollbars_do_not_reserve_viewport_space() {
        assert_eq!(
            reserved_viewport(
                ScrollAxis::Vertical,
                320.0,
                500.0,
                12.0,
                ScrollBarPlacement::Floating,
            ),
            (320.0, 500.0)
        );
        assert_eq!(
            reserved_viewport(
                ScrollAxis::Horizontal,
                320.0,
                500.0,
                12.0,
                ScrollBarPlacement::Floating,
            ),
            (320.0, 500.0)
        );
    }

    fn geometry_ctx() -> BuildContext<'static> {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        BuildContext::new(
            aimer_canvas::FrameCanvas::new(inner),
            ResolvedSize {
                width: 100.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            aimer_widget::base::WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        )
    }

    /// A held thumb is painted wider than a resting one, from the same edge, and
    /// the geometry the finger hit-tests against grows with it.
    #[tokio::test]
    async fn a_held_thumb_is_painted_wider_and_hit_tested_wider() {
        let ctx = geometry_ctx();
        let ctrl = ScrollState::for_test_at(Vec2d::ZERO);
        ctrl.cached_viewport.set((100.0, 100.0));
        ctrl.cached_content_size.set(ResolvedSize {
            width: 100.0,
            height: 400.0,
        });
        let bar = ScrollBar::default();

        let resting = scrollbar_geometry(&ctx, &ctrl, &bar, 0.0).unwrap().unwrap();
        let held = scrollbar_geometry(&ctx, &ctrl, &bar, 1.0).unwrap().unwrap();

        assert!(held.thumb_size.width > resting.thumb_size.width);
        assert_eq!(held.thumb_size.height, resting.thumb_size.height, "only the width grows");
        assert!(held.hit_rect.2 > resting.hit_rect.2, "and so does the target");
        assert!(
            ((held.hit_rect.0 + held.hit_rect.2) - (resting.hit_rect.0 + resting.hit_rect.2)).abs()
                < 1e-3,
            "from the edge against the viewport"
        );
    }

    /// A phone shows no track: the thumb alone is the bar, whatever colors the
    /// track was given and however far a drag has lit it.
    #[test]
    fn a_touch_device_paints_no_track() {
        let track = ScrollTrack {
            color: Colors::Rgba(10, 20, 30, 255),
            hover_color: Colors::Rgba(200, 200, 200, 255),
            ..ScrollTrack::default()
        };

        for mix in [0.0, 0.5, 1.0] {
            let (_, _, _, alpha) = track_paint_color(true, &track, mix).to_rgba();
            assert_eq!(alpha, 0, "mix {mix}");
        }
        let (_, _, _, alpha) = track_paint_color(false, &track, 0.0).to_rgba();
        assert_eq!(alpha, 255, "other targets still paint their track");
    }

    /// The track fades in rather than popping: hovering starts it at nothing and
    /// it reaches full strength only after the fade duration.
    #[test]
    fn the_track_fades_in_over_the_fade_duration() {
        let runtime = ScrollBarRuntime::default();
        let start = aimer_utils::AnimInstant::now();
        let fade = Duration::from_millis(SCROLLBAR_TRACK_FADE_MS);

        assert_eq!(runtime.track_mix(false, start), 0.0);
        assert_eq!(runtime.track_mix(true, start), 0.0, "the fade begins at nothing");
        let halfway = runtime.track_mix(true, start + fade / 2);
        assert!((halfway - 0.5).abs() < 0.01, "halfway through: {halfway}");
        assert_eq!(runtime.track_mix(true, start + fade), 1.0);
        assert_eq!(
            runtime.track_mix(true, start + fade * 3),
            1.0,
            "it stays at full strength"
        );
    }

    /// Nothing paints while the strip is held, so the first frame after the
    /// pointer leaves comes long after the previous one. That gap is not fade
    /// time: the track must start fading out from full strength, not vanish.
    #[test]
    fn the_track_fades_out_after_a_long_rest() {
        let runtime = ScrollBarRuntime::default();
        let start = aimer_utils::AnimInstant::now();
        let fade = Duration::from_millis(SCROLLBAR_TRACK_FADE_MS);
        runtime.track_mix(true, start);
        assert_eq!(runtime.track_mix(true, start + fade), 1.0);

        let left = start + Duration::from_secs(5);
        assert_eq!(
            runtime.track_mix(false, left),
            1.0,
            "the fade out begins at full strength"
        );
        let leaving = runtime.track_mix(false, left + fade / 2);
        assert!((leaving - 0.5).abs() < 0.01, "halfway out: {leaving}");
        assert_eq!(runtime.track_mix(false, left + fade * 2), 0.0);
    }

    /// The same gap precedes the first frame of a hover.
    #[test]
    fn the_track_fades_in_after_a_long_rest() {
        let runtime = ScrollBarRuntime::default();
        let start = aimer_utils::AnimInstant::now();
        let fade = Duration::from_millis(SCROLLBAR_TRACK_FADE_MS);
        runtime.track_mix(false, start);

        let entered = start + Duration::from_secs(5);
        assert_eq!(runtime.track_mix(true, entered), 0.0);
        let halfway = runtime.track_mix(true, entered + fade / 2);
        assert!((halfway - 0.5).abs() < 0.01, "halfway in: {halfway}");
    }

    #[test]
    fn track_colors_blend_without_darkening_through_transparent() {
        let from = Colors::Transparent;
        let to = Colors::Rgba(128, 128, 128, 40);
        assert_eq!(blend_colors(from, to, 0.0).to_rgba(), (0, 0, 0, 0));
        assert_eq!(blend_colors(from, to, 1.0).to_rgba(), (128, 128, 128, 40));
        let mid = blend_colors(from, to, 0.5).to_rgba();
        assert_eq!((mid.0, mid.1, mid.2), (128, 128, 128), "the hue is the target's");
        assert!((19..=21).contains(&mid.3), "alpha is halfway: {}", mid.3);
    }

    /// Wiring: a bar painted for the first time while hovered has not faded in
    /// yet, so its track is still the resting one.
    #[tokio::test]
    async fn a_freshly_hovered_track_has_not_popped_in() {
        let ctrl = Rc::new(ScrollState::for_test_at(Vec2d::ZERO));
        ctrl.cached_viewport.set((100.0, 100.0));
        ctrl.cached_content_size.set(ResolvedSize {
            width: 100.0,
            height: 400.0,
        });
        ctrl.bar.pointer
            .set(aimer_input::mouse_region::PointerState::Inside);
        let mut bar = ScrollBar::default();
        bar.track.hover_color = Colors::Rgba(10, 20, 30, 40);
        let ctx = geometry_ctx();
        let runtime = ScrollBarRuntime::default();
        // Already fully showing, so the paint is not skipped for being invisible.
        runtime.showing.set(true);
        runtime.alpha.set(1.0);

        let paint = prepare_scrollbar_paint(&ctx, &ctrl, &bar, &runtime).unwrap();
        assert_ne!(paint.track_color, Color::from(bar.track.hover_color));
    }

    #[test]
    fn the_default_track_has_a_visible_hover_color() {
        let hover: Color = ScrollTrack::default().hover_color.into();
        assert!(hover.to_rgba().3 > 0, "a transparent hover color shows no track");
    }
}
