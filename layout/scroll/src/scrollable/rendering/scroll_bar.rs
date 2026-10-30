use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use aimer_attribute::dimension::Dimension;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd_v2::Rect;
use aimer_cupid::utilities::Color as V2Color;
use aimer_widget::base::{BuildContext, Color, Colors};
use aimer_widget::portable::__anteros::{WidgetDocumentView, WidgetNodeView};
use aimer_widget::portable::PortableMaterializeError;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement, Rebuildable, State,
    StateUpdater, StatefulElement, StatefulWidget, VisitorElement, Widget,
};

use crate::ScrollAxis;
use crate::scrollable::constants::{
    SCROLLBAR_HIDE_DURATION_MS, SCROLLBAR_IDLE_HOLD_MS, SCROLLBAR_SHOW_DURATION_MS,
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
            hover_color: Colors::Transparent,
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
        RawScrollBar {
            config: self.config.clone(),
            ctrl: self.config.ctrl.clone(),
            runtime: self.runtime.clone(),
        }
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

pub(crate) fn track_width(scroll_bar: &ScrollBar, cross_extent: f32, scale: f32) -> f32 {
    match scroll_bar.track.width {
        Dimension::Px(value) => value * scale,
        Dimension::Percent(percent) => cross_extent * (percent / 100.0),
        Dimension::Auto => {
            #[cfg(any(target_os = "android", target_os = "ios"))]
            {
                6.0 * scale
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
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
        let _ = prepare_scrollbar_paint(ctx, ctrl, &self.config, self.runtime.as_ref());
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.ctrl
            .as_ref()
            .is_none_or(|ctrl| scrollbar_geometry(ctx, ctrl, &self.config).is_ok())
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

fn scrollbar_geometry(
    ctx: &BuildContext,
    ctrl: &ScrollState,
    scroll_bar: &ScrollBar,
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

    let thumb_width = match scroll_bar.thumb.width {
        Dimension::Px(value) => value * scale,
        Dimension::Percent(percent) => track_width * (percent / 100.0),
        Dimension::Auto => (track_width * 0.6).max(4.0),
    };
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
    let thumb_cross_offset = (track_width - thumb_width) / 2.0;
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
    let track_color: Color = if is_hover {
        scroll_bar.track.hover_color.into()
    } else {
        scroll_bar.track.color.into()
    };
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
    let paint = match scrollbar_geometry(ctx, ctrl, scroll_bar) {
        Ok(Some(paint)) => paint,
        Ok(None) => {
            runtime.hide();
            ctrl.scroll_bar_opacity.set(0.0);
            if matches!(ctrl.axis, ScrollAxis::Vertical) {
                ctrl.v_thumb_rect.set(None);
            } else {
                ctrl.h_thumb_rect.set(None);
            }
            return None;
        }
        Err(()) => return None,
    };
    // The geometry is published whether or not the bar is showing: a pointer
    // reaching for a hidden bar has to find it where it is, and that is what
    // shows it.
    if paint.is_vertical {
        ctrl.cached_v_track_width.set(paint.track_width);
        ctrl.v_scroll_multiplier.set(paint.multiplier);
        ctrl.v_thumb_rect.set(Some(paint.hit_rect));
    } else {
        ctrl.cached_h_track_width.set(paint.track_width);
        ctrl.h_scroll_multiplier.set(paint.multiplier);
        ctrl.h_thumb_rect.set(Some(paint.hit_rect));
    }

    let dragging = matches!(
        ctrl.drag_mode.get(),
        DragMode::VerticalScrollbar | DragMode::HorizontalScrollbar
    );
    let alpha = runtime.update(
        ctrl.scroll_offset.get(),
        ctrl.is_scrolling.get() || ctrl.scroll_bar_pointer_moved.replace(false),
        dragging || ctrl.scroll_bar_hovered.get(),
        aimer_utils::AnimInstant::now(),
    );
    ctrl.scroll_bar_opacity.set(alpha);
    // A transition, and the idle delay before the fade, end only on a frame
    // that nothing else is going to ask for.
    if runtime.transition_start.get().is_some() || runtime.counting_down() {
        ctrl.request_animation_frame();
    }
    if alpha <= 0.0 {
        return None;
    }

    Some(ScrollBarPaint { alpha, ..paint })
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
}
