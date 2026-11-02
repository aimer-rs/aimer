//! Stateful widget adapters for range controls.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_cupid::draw_cmd_v2::Rect;
use aimer_events::element::{ElementEvent, KeyAction, NamedKey};
use aimer_events::pointer::PointerButton;
use aimer_widget::InteractionBounds;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, ChildBuilder, Drawable, Element, EventElement, EventResult, EventTreeRole,
    FocusNode, LayoutElement, PointerKey, PortableWidget, Rebuildable, State, StateUpdater,
    StatefulElement, StatefulWidget, VisitorElement, Widget,
};

use super::visuals::{SliderTrack, SliderVisualState};
use super::{RangeSlider, RangeThumb, RangeValue, Slider, SliderKey, SliderThumb, SliderTrail};

struct SliderRuntime<T: RangeValue> {
    current: Cell<T>,
    active_pointer: Cell<Option<PointerKey>>,
    pressed: Cell<bool>,
    hovered: Cell<bool>,
    focused: Cell<bool>,
    focus_node: FocusNode,
    last_proposed: Cell<Option<T>>,
    updater: Cell<StateUpdater<SliderState<T>>>,
    default_visuals: RefCell<SliderDefaultVisuals>,
}

struct SliderDefaultVisuals {
    track: ChildBuilder,
    trail: ChildBuilder,
    thumb: ChildBuilder,
    trail_style: DefaultVisualStyle,
    thumb_style: DefaultVisualStyle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DefaultVisualStyle {
    Normal,
    Pressed,
    Focused,
    Disabled,
}

fn trail_visual_style(state: SliderVisualState) -> DefaultVisualStyle {
    if state.disabled {
        DefaultVisualStyle::Disabled
    } else if state.pressed {
        DefaultVisualStyle::Pressed
    } else {
        DefaultVisualStyle::Normal
    }
}

fn thumb_visual_style(state: SliderVisualState) -> DefaultVisualStyle {
    if state.disabled {
        DefaultVisualStyle::Disabled
    } else if state.focused {
        DefaultVisualStyle::Focused
    } else {
        DefaultVisualStyle::Normal
    }
}

impl<T: RangeValue> SliderRuntime<T> {
    fn new(current: T) -> Self {
        let default_visual_state = SliderVisualState::default();
        Self {
            current: Cell::new(current),
            active_pointer: Cell::new(None),
            pressed: Cell::new(false),
            hovered: Cell::new(false),
            focused: Cell::new(false),
            focus_node: FocusNode::new(),
            last_proposed: Cell::new(None),
            updater: Cell::new(StateUpdater::empty()),
            default_visuals: RefCell::new(SliderDefaultVisuals {
                track: ChildBuilder::from_widget(SliderTrack::new()),
                trail: ChildBuilder::from_widget(SliderTrail::for_state(default_visual_state)),
                thumb: ChildBuilder::from_widget(SliderThumb::for_state(default_visual_state)),
                trail_style: DefaultVisualStyle::Normal,
                thumb_style: DefaultVisualStyle::Normal,
            }),
        }
    }

    fn default_visuals(
        &self,
        state: SliderVisualState,
    ) -> (ChildBuilder, ChildBuilder, ChildBuilder) {
        let trail_style = trail_visual_style(state);
        let thumb_style = thumb_visual_style(state);
        let mut visuals = self.default_visuals.borrow_mut();
        if visuals.trail_style != trail_style {
            visuals.trail = ChildBuilder::from_widget(SliderTrail::for_state(state));
            visuals.trail_style = trail_style;
        }
        if visuals.thumb_style != thumb_style {
            visuals.thumb = ChildBuilder::from_widget(SliderThumb::for_state(state));
            visuals.thumb_style = thumb_style;
        }
        (
            visuals.track.clone(),
            visuals.trail.clone(),
            visuals.thumb.clone(),
        )
    }

    fn request_visual_rebuild(&self) {
        self.updater.get().set_state(|state| {
            state.visual_revision = state.visual_revision.wrapping_add(1);
        });
    }
}

struct RangeSliderRuntime<T: RangeValue> {
    lower: Cell<T>,
    upper: Cell<T>,
    active_pointer: Cell<Option<PointerKey>>,
    active_thumb: Cell<Option<RangeThumb>>,
    pressed: Cell<bool>,
    hovered: Cell<bool>,
    focused: Cell<bool>,
    focus_node: FocusNode,
    last_proposed: Cell<Option<(T, T)>>,
    updater: Cell<StateUpdater<RangeSliderState<T>>>,
    default_visuals: RefCell<RangeSliderDefaultVisuals>,
}

struct RangeSliderDefaultVisuals {
    track: ChildBuilder,
    trail: ChildBuilder,
    lower_thumb: ChildBuilder,
    upper_thumb: ChildBuilder,
    trail_style: DefaultVisualStyle,
    thumb_style: DefaultVisualStyle,
}

impl<T: RangeValue> RangeSliderRuntime<T> {
    fn new(lower: T, upper: T) -> Self {
        let default_visual_state = SliderVisualState::default();
        Self {
            lower: Cell::new(lower),
            upper: Cell::new(upper),
            active_pointer: Cell::new(None),
            active_thumb: Cell::new(None),
            pressed: Cell::new(false),
            hovered: Cell::new(false),
            focused: Cell::new(false),
            focus_node: FocusNode::new(),
            last_proposed: Cell::new(None),
            updater: Cell::new(StateUpdater::empty()),
            default_visuals: RefCell::new(RangeSliderDefaultVisuals {
                track: ChildBuilder::from_widget(SliderTrack::new()),
                trail: ChildBuilder::from_widget(SliderTrail::for_state(default_visual_state)),
                lower_thumb: ChildBuilder::from_widget(SliderThumb::for_state(
                    default_visual_state,
                )),
                upper_thumb: ChildBuilder::from_widget(SliderThumb::for_state(
                    default_visual_state,
                )),
                trail_style: DefaultVisualStyle::Normal,
                thumb_style: DefaultVisualStyle::Normal,
            }),
        }
    }

    fn default_visuals(
        &self,
        state: SliderVisualState,
    ) -> (ChildBuilder, ChildBuilder, ChildBuilder, ChildBuilder) {
        let trail_style = trail_visual_style(state);
        let thumb_style = thumb_visual_style(state);
        let mut visuals = self.default_visuals.borrow_mut();
        if visuals.trail_style != trail_style {
            visuals.trail = ChildBuilder::from_widget(SliderTrail::for_state(state));
            visuals.trail_style = trail_style;
        }
        if visuals.thumb_style != thumb_style {
            visuals.lower_thumb = ChildBuilder::from_widget(SliderThumb::for_state(state));
            visuals.upper_thumb = ChildBuilder::from_widget(SliderThumb::for_state(state));
            visuals.thumb_style = thumb_style;
        }
        (
            visuals.track.clone(),
            visuals.trail.clone(),
            visuals.lower_thumb.clone(),
            visuals.upper_thumb.clone(),
        )
    }

    fn request_visual_rebuild(&self) {
        self.updater.get().set_state(|state| {
            state.visual_revision = state.visual_revision.wrapping_add(1);
        });
    }
}

/// Retained runtime state for a [`Slider`] widget.
pub struct SliderState<T: RangeValue = f64> {
    model: Slider<T>,
    runtime: Rc<SliderRuntime<T>>,
    // Runtime-only pressed/focus changes still rebuild the visual leaf widgets.
    visual_revision: u64,
}

impl<T: RangeValue> SliderState<T> {
    /// Returns the current value held by the widget, including pointer and
    /// keyboard changes made since the last parent rebuild.
    #[inline]
    pub fn current_value(&self) -> T {
        self.runtime.current.get()
    }

    /// Returns whether a pointer is currently dragging this slider.
    #[inline]
    pub fn is_pressed(&self) -> bool {
        self.runtime.pressed.get()
    }

    /// Returns whether this slider currently owns keyboard focus.
    #[inline]
    pub fn is_focused(&self) -> bool {
        self.runtime.focused.get()
    }

    /// Returns whether this slider currently ignores user input.
    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.model.is_disabled()
    }

    /// Returns semantic range metadata for the current widget value.
    #[inline]
    pub fn semantics(&self) -> super::RangeSemantics {
        let mut model = self.model.clone();
        let _ = model.set_value(self.current_value());
        model.semantics()
    }
}

impl<T: RangeValue> StatefulWidget for Slider<T> {
    type State = SliderState<T>;

    fn create_state(self) -> Self::State {
        let current = self.current_value();
        SliderState {
            model: self,
            runtime: Rc::new(SliderRuntime::new(current)),
            visual_revision: 0,
        }
    }
}

impl<T: RangeValue> State<Slider<T>> for SliderState<T> {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.runtime.updater.set(updater);
    }

    fn adopt_config_from(&mut self, new: Self) {
        let old_model = &self.model;
        let value_changed = old_model.current_value() != new.model.current_value();
        let domain_changed = old_model.range_bounds() != new.model.range_bounds()
            || old_model.step_value() != new.model.step_value()
            || old_model.reversed_bounds_policy_value() != new.model.reversed_bounds_policy_value();
        self.model = new.model;
        if value_changed {
            self.runtime.current.set(self.model.current_value());
        } else if domain_changed {
            let current = self.runtime.current.get();
            let current = self
                .model
                .canonical_value(current)
                .unwrap_or(self.model.current_value());
            self.runtime.current.set(current);
        }
        self.runtime.last_proposed.set(None);
        if self.model.is_disabled() {
            self.runtime.active_pointer.set(None);
            self.runtime.pressed.set(false);
            self.runtime.hovered.set(false);
            self.runtime.focused.set(false);
        }
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let visual_state = SliderVisualState {
            disabled: self.model.is_disabled(),
            pressed: self.runtime.pressed.get(),
            focused: self.runtime.focused.get(),
        };
        let default_visuals = self.runtime.default_visuals(visual_state);
        SliderSurface {
            model: self.model.clone(),
            runtime: Rc::clone(&self.runtime),
            track: self
                .model
                .track_child()
                .unwrap_or_else(|| default_visuals.0),
            trail: self
                .model
                .trail_child()
                .unwrap_or_else(|| default_visuals.1),
            thumb: self
                .model
                .thumb_child()
                .unwrap_or_else(|| default_visuals.2),
        }
    }
}

impl<T: RangeValue> Widget for Slider<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Slider", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Slider"
    }
}

impl<T: RangeValue> PortableWidget for Slider<T> {}

struct SliderSurface<T: RangeValue> {
    model: Slider<T>,
    runtime: Rc<SliderRuntime<T>>,
    track: ChildBuilder,
    trail: ChildBuilder,
    thumb: ChildBuilder,
}

impl<T: RangeValue> Widget for SliderSurface<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let track_geometry = Rc::new(Cell::new(SliderVisualGeometry::default()));
        let trail_geometry = Rc::new(Cell::new(SliderVisualGeometry::default()));
        let thumb_geometry = Rc::new(Cell::new(SliderVisualGeometry::default()));
        let track = Element::boxed(RawSliderVisualSlot {
            child: self.track.build(ctx),
            geometry: Rc::clone(&track_geometry),
        });
        let trail = Element::boxed(RawSliderVisualSlot {
            child: self.trail.build(ctx),
            geometry: Rc::clone(&trail_geometry),
        });
        let thumb = Element::boxed(RawSliderVisualSlot {
            child: self.thumb.build(ctx),
            geometry: Rc::clone(&thumb_geometry),
        });
        Element::boxed(RawSlider {
            model: self.model,
            runtime: self.runtime,
            track,
            trail,
            thumb,
            bounds: InteractionBounds::new(),
            visual_inset: Cell::new(0.0),
            track_geometry,
            trail_geometry,
            thumb_geometry,
        })
    }

    fn debug_name(&self) -> &'static str {
        "RawSlider"
    }
}

impl<T: RangeValue> PortableWidget for SliderSurface<T> {}

#[derive(Clone, Copy, Default)]
struct SliderVisualGeometry {
    position: Vec2d,
    clip: Option<Rect>,
}

/// Where a [`RawSlider`] puts its visuals. Offsets are device pixels; the
/// trail's clip is logical, in its geometry.
struct SliderPlacement {
    track: SliderVisualGeometry,
    trail: SliderVisualGeometry,
    thumb: SliderVisualGeometry,
}

/// [`SliderPlacement`] for a [`RawRangeSlider`], which has two thumbs.
struct RangePlacement {
    track: SliderVisualGeometry,
    trail: SliderVisualGeometry,
    lower: SliderVisualGeometry,
    upper: SliderVisualGeometry,
}

struct RawSliderVisualSlot {
    child: AnyElement,
    geometry: Rc<Cell<SliderVisualGeometry>>,
}

impl VisitorElement for RawSliderVisualSlot {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "SliderVisualSlot"
    }
}

impl EventElement for RawSliderVisualSlot {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl Rebuildable for RawSliderVisualSlot {
    fn is_carry_state(&self) -> bool {
        self.child.is_carry_state()
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.child.with_rebuild_context(ctx, callback);
    }
}

impl LayoutElement for RawSliderVisualSlot {
    fn pos(&self) -> Option<Vec2d> {
        Some(self.geometry.get().position)
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.layout(ctx)
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.event_tree_bounds()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
}

impl Drawable for RawSliderVisualSlot {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn retained_clip(&self, _ctx: &BuildContext) -> Option<Rect> {
        self.geometry.get().clip
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }
}

struct RawSlider<T: RangeValue> {
    model: Slider<T>,
    runtime: Rc<SliderRuntime<T>>,
    track: AnyElement,
    trail: AnyElement,
    thumb: AnyElement,
    bounds: InteractionBounds,
    /// Logical inset reserved for half of the visual thumb at each endpoint.
    visual_inset: Cell<f32>,
    track_geometry: Rc<Cell<SliderVisualGeometry>>,
    trail_geometry: Rc<Cell<SliderVisualGeometry>>,
    thumb_geometry: Rc<Cell<SliderVisualGeometry>>,
}

impl<T: RangeValue> RawSlider<T> {
    fn hit_test(&self, x: f32, y: f32) -> bool {
        self.bounds
            .get_bounds()
            .is_some_and(|bounds| bounds.width > 0.0 && bounds.height > 0.0)
            && self.bounds.is_inside(x, y)
    }

    fn track_position(&self, x: f32) -> Option<T> {
        let bounds = self.bounds.get_bounds()?;
        // Map pointer coordinates over the same inset track used for drawing,
        // so pressing an endpoint thumb keeps the value at the endpoint.
        let inset = self.visual_inset.get().max(0.0).min(bounds.width / 2.0);
        let track_width = (bounds.width - inset * 2.0).max(0.0);
        self.model
            .value_at_position(
                f64::from((x - bounds.x - inset).clamp(0.0, track_width)),
                f64::from(track_width),
            )
            .ok()
    }

    fn propose(&self, value: T) {
        if self.runtime.current.get() == value {
            return;
        }
        self.runtime.current.set(value);
        if self.runtime.last_proposed.replace(Some(value)) == Some(value) {
            return;
        }
        if let Some(callback) = self.model.on_change.as_ref() {
            callback(value);
        }
    }

    fn propose_at(&self, x: f32) {
        if let Some(value) = self.track_position(x)
            && let Ok(value) = self.model.canonical_value(value)
        {
            self.propose(value);
        }
    }

    fn key_action(key: &NamedKey) -> Option<SliderKey> {
        match key {
            NamedKey::ArrowLeft => Some(SliderKey::ArrowLeft),
            NamedKey::ArrowRight => Some(SliderKey::ArrowRight),
            NamedKey::ArrowUp => Some(SliderKey::ArrowUp),
            NamedKey::ArrowDown => Some(SliderKey::ArrowDown),
            NamedKey::Home => Some(SliderKey::Home),
            NamedKey::End => Some(SliderKey::End),
            NamedKey::PageUp => Some(SliderKey::PageUp),
            NamedKey::PageDown => Some(SliderKey::PageDown),
            _ => None,
        }
    }

    fn handle_key(&self, key: &NamedKey) -> EventResult {
        if self.model.is_disabled() {
            return EventResult::ignored();
        }
        let Some(key) = Self::key_action(key) else {
            return EventResult::ignored();
        };
        let mut candidate = self.model.clone();
        if candidate.set_value(self.runtime.current.get()).is_err() {
            return EventResult::ignored();
        }
        let Ok(changed) = candidate.handle_key(key) else {
            return EventResult::ignored();
        };
        if changed {
            self.propose(candidate.current_value());
        }
        EventResult::consumed().with_redraw()
    }

    fn position_px(&self, ctx: &BuildContext, size: ResolvedSize, inset: f32) -> f32 {
        let width = size.width.max(0.0);
        // Endpoint centers are inset by half the thumb width so the visual
        // thumb remains inside the slider bounds at both ends.
        let inset = inset.max(0.0).min(width / 2.0);
        let track_width = (width - inset * 2.0).max(0.0);
        let logical_width = (track_width / ctx.scale).max(0.0);
        let position = self
            .model
            .position_for_value(self.runtime.current.get(), logical_width as f64)
            .unwrap_or(0.0) as f32
            * ctx.scale;
        (inset + position).clamp(inset, width - inset)
    }

    /// Works out where the visuals go and publishes it to their slots.
    ///
    /// The render tree reads a slot's position when it syncs, which is before
    /// this slider's `update` runs, so the placement has to be available from
    /// `retained_v2_bounds` as well: a position written only while drawing
    /// reaches the tree one frame late, and a first frame nothing follows left
    /// the visuals at the slider's origin.
    fn place_visuals(&self, ctx: &BuildContext) -> (ResolvedSize, SliderPlacement) {
        let size = self.computed_size(ctx);
        let thumb_inset = self.thumb_inset(ctx, size);
        self.visual_inset
            .set((thumb_inset / ctx.scale.max(f32::EPSILON)).max(0.0));
        let child_ctx = child_context(ctx, size);
        let logical_scale = ctx.scale.max(f32::EPSILON);

        let track_size = self.track.computed_size(&child_ctx);
        let track = SliderVisualGeometry {
            position: Vec2d {
                x: 0.0,
                y: (size.height - track_size.height) / 2.0,
            },
            clip: None,
        };

        let position = self.position_px(ctx, size, thumb_inset);
        let trail_size = self.trail.computed_size(&child_ctx);
        let trail_width = position.clamp(0.0, size.width);
        let trail_height = trail_size.height.max(0.0);
        let trail = SliderVisualGeometry {
            position: Vec2d {
                x: 0.0,
                y: (size.height - trail_size.height) / 2.0,
            },
            clip: Some(Rect::new(
                0.0,
                0.0,
                trail_width / logical_scale,
                trail_height / logical_scale,
            )),
        };

        let thumb_size = self.thumb.computed_size(&child_ctx);
        let thumb = SliderVisualGeometry {
            position: Vec2d {
                x: position - thumb_size.width / 2.0,
                y: (size.height - thumb_size.height) / 2.0,
            },
            clip: None,
        };

        self.track_geometry.set(track);
        self.trail_geometry.set(trail);
        self.thumb_geometry.set(thumb);
        (
            size,
            SliderPlacement {
                track,
                trail,
                thumb,
            },
        )
    }

    fn thumb_inset(&self, ctx: &BuildContext, size: ResolvedSize) -> f32 {
        let child_ctx = child_context(ctx, size);
        (self.thumb.computed_size(&child_ctx).width.max(0.0) / 2.0).min(size.width.max(0.0) / 2.0)
    }
}

impl<T: RangeValue> VisitorElement for RawSlider<T> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.track.as_ref());
        visitor(self.trail.as_ref());
        visitor(self.thumb.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "RawSlider"
    }
}

impl<T: RangeValue> EventElement for RawSlider<T> {
    // The visual slots are composable decorations. The slider itself owns the
    // pointer gesture so an interactive child cannot steal its capture.
    fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn focus_node(&self) -> Option<&FocusNode> {
        (!self.model.is_disabled()).then_some(&self.runtime.focus_node)
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerDown(pointer)
                if pointer.button == PointerButton::Primary
                    && !self.model.is_disabled()
                    && self.hit_test(pointer.pos.x, pointer.pos.y) =>
            {
                let key = PointerKey::new(pointer.source, pointer.id);
                // The thumb follows this finger from the press, so the gesture is
                // the slider's: an ancestor that arbitrates touches must not take
                // a sideways drag of it for something else.
                aimer_widget::claim_pointer(key);
                self.runtime.active_pointer.set(Some(key));
                self.runtime.pressed.set(true);
                self.runtime.request_visual_rebuild();
                self.propose_at(pointer.pos.x);
                EventResult::consumed()
                    .with_pointer_capture(key)
                    .with_redraw()
            }
            ElementEvent::PointerMove(pointer) => {
                if self.model.is_disabled() {
                    return EventResult::ignored();
                }
                let key = PointerKey::new(pointer.source, pointer.id);
                if self.runtime.active_pointer.get() == Some(key) {
                    self.propose_at(pointer.pos.x);
                    EventResult::consumed().with_redraw()
                } else {
                    let inside = self.hit_test(pointer.pos.x, pointer.pos.y);
                    if self.runtime.hovered.replace(inside) != inside {
                        EventResult::redraw()
                    } else {
                        EventResult::ignored()
                    }
                }
            }
            ElementEvent::PointerUp(pointer)
                if self.runtime.active_pointer.get()
                    == Some(PointerKey::new(pointer.source, pointer.id)) =>
            {
                let key = PointerKey::new(pointer.source, pointer.id);
                self.propose_at(pointer.pos.x);
                self.runtime.active_pointer.set(None);
                self.runtime.pressed.set(false);
                self.runtime.request_visual_rebuild();
                EventResult::consumed()
                    .with_pointer_release(key)
                    .with_redraw()
            }
            ElementEvent::PointerExited(_, _) => {
                if self.runtime.hovered.replace(false) {
                    EventResult::redraw()
                } else {
                    EventResult::ignored()
                }
            }
            ElementEvent::FocusGained => {
                self.runtime.focused.set(true);
                self.runtime.request_visual_rebuild();
                EventResult::redraw()
            }
            ElementEvent::FocusLost => {
                self.runtime.focused.set(false);
                self.runtime.request_visual_rebuild();
                EventResult::redraw()
            }
            ElementEvent::KeyInput {
                key,
                action: KeyAction::Pressed | KeyAction::Repeat,
                ..
            } => self.handle_key(key),
            ElementEvent::Cancel => {
                if self.runtime.active_pointer.get().is_some() {
                    self.runtime.active_pointer.set(None);
                    self.runtime.pressed.set(false);
                    self.runtime.request_visual_rebuild();
                    EventResult::consumed().with_redraw()
                } else {
                    EventResult::ignored()
                }
            }
            _ => EventResult::ignored(),
        }
    }
}

impl<T: RangeValue> LayoutElement for RawSlider<T> {
    fn size(&self) -> Option<Size> {
        Some(Size::new(
            self.model.widget_width(),
            self.model.widget_height(),
        ))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let requested = Size::new(self.model.widget_width(), self.model.widget_height()).resolve(
            &ResolvedSize {
                width: ctx.box_constraint.max_width,
                height: ctx.box_constraint.max_height,
            },
            ctx.scale,
        );
        ResolvedSize {
            width: requested
                .width
                .clamp(ctx.box_constraint.min_width, ctx.box_constraint.max_width),
            height: requested
                .height
                .clamp(ctx.box_constraint.min_height, ctx.box_constraint.max_height),
        }
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let (size, placement) = self.place_visuals(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
        let child_ctx = child_context(ctx, size);
        layout_child(&self.track, &child_ctx, placement.track.position);
        layout_child(&self.trail, &child_ctx, placement.trail.position);
        layout_child(&self.thumb, &child_ctx, placement.thumb.position);
        size
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl<T: RangeValue> Drawable for RawSlider<T> {
    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("RawSlider")
    }

    fn update(&self, ctx: &BuildContext) {
        let (size, placement) = self.place_visuals(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
        let child_ctx = child_context(ctx, size);

        let track_height = (4.0 * ctx.scale).min(size.height.max(0.0));
        if size.width <= 0.0 || track_height <= 0.0 {
            return;
        }
        draw_child(&self.track, &child_ctx, placement.track.position);

        draw_child(&self.trail, &child_ctx, placement.trail.position);

        draw_child(&self.thumb, &child_ctx, placement.thumb.position);
    }

    /// Publishes the visuals' placement before the render tree reads their
    /// positions. See [`RawSlider::place_visuals`]; it does not change the
    /// bounds themselves.
    fn retained_v2_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        self.place_visuals(ctx);
        None
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        if !ctx.scale.is_finite() || ctx.scale <= 0.0 {
            return false;
        }
        let size = self.computed_size(ctx);
        let width = size.width / ctx.scale;
        let height = size.height / ctx.scale;
        width.is_finite()
            && height.is_finite()
            && width > 0.0
            && height > 0.0
            && width <= 1_000_000.0
            && height <= 1_000_000.0
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}
}

impl<T: RangeValue> Rebuildable for RawSlider<T> {}
impl<T: RangeValue> PortableWidget for RawSlider<T> {}

impl<T: RangeValue> Widget for RawSlider<T> {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }

    fn debug_name(&self) -> &'static str {
        "RawSlider"
    }
}

/// Retained runtime state for a [`RangeSlider`] widget.
pub struct RangeSliderState<T: RangeValue = f64> {
    model: RangeSlider<T>,
    runtime: Rc<RangeSliderRuntime<T>>,
    // Runtime-only pressed/focus changes still rebuild the visual leaf widgets.
    visual_revision: u64,
}

impl<T: RangeValue> RangeSliderState<T> {
    /// Returns the lower thumb value currently held by the widget.
    #[inline]
    pub fn lower(&self) -> T {
        self.runtime.lower.get()
    }

    /// Returns the upper thumb value currently held by the widget.
    #[inline]
    pub fn upper(&self) -> T {
        self.runtime.upper.get()
    }

    /// Returns both current thumb values as an inclusive range.
    #[inline]
    pub fn current_values(&self) -> std::ops::Range<T> {
        self.lower()..self.upper()
    }

    /// Returns the thumb currently selected for keyboard input.
    #[inline]
    pub fn active_thumb(&self) -> Option<RangeThumb> {
        self.runtime.active_thumb.get()
    }

    /// Returns whether a pointer is currently dragging either thumb.
    #[inline]
    pub fn is_pressed(&self) -> bool {
        self.runtime.pressed.get()
    }

    /// Returns whether this range slider currently owns keyboard focus.
    #[inline]
    pub fn is_focused(&self) -> bool {
        self.runtime.focused.get()
    }

    /// Returns whether this range slider currently ignores user input.
    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.model.is_disabled()
    }

    /// Returns semantic range metadata for the current widget values.
    #[inline]
    pub fn semantics(&self) -> super::RangeSemantics {
        let mut model = self.model.clone();
        let _ = model.set_values(self.lower(), self.upper());
        model.semantics()
    }
}

impl<T: RangeValue> StatefulWidget for RangeSlider<T> {
    type State = RangeSliderState<T>;

    fn create_state(self) -> Self::State {
        let lower = self.lower();
        let upper = self.upper();
        RangeSliderState {
            model: self,
            runtime: Rc::new(RangeSliderRuntime::new(lower, upper)),
            visual_revision: 0,
        }
    }
}

impl<T: RangeValue> State<RangeSlider<T>> for RangeSliderState<T> {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.runtime.updater.set(updater);
    }

    fn adopt_config_from(&mut self, new: Self) {
        let old_model = &self.model;
        let values_changed =
            old_model.lower() != new.model.lower() || old_model.upper() != new.model.upper();
        let domain_changed = old_model.range_bounds() != new.model.range_bounds()
            || old_model.step_value() != new.model.step_value()
            || old_model.reversed_bounds_policy_value() != new.model.reversed_bounds_policy_value();
        self.model = new.model;
        if values_changed {
            self.runtime.lower.set(self.model.lower());
            self.runtime.upper.set(self.model.upper());
        } else if domain_changed {
            let lower = self
                .model
                .canonical_value(super::RangeField::LowerValue, self.runtime.lower.get())
                .unwrap_or(self.model.lower());
            let upper = self
                .model
                .canonical_value(super::RangeField::UpperValue, self.runtime.upper.get())
                .unwrap_or(self.model.upper());
            let (lower, upper) = if lower <= upper {
                (lower, upper)
            } else {
                (upper, lower)
            };
            self.runtime.lower.set(lower);
            self.runtime.upper.set(upper);
        }
        self.runtime.last_proposed.set(None);
        if self.model.is_disabled() {
            self.runtime.active_pointer.set(None);
            self.runtime.active_thumb.set(None);
            self.runtime.pressed.set(false);
            self.runtime.hovered.set(false);
            self.runtime.focused.set(false);
        }
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let visual_state = SliderVisualState {
            disabled: self.model.is_disabled(),
            pressed: self.runtime.pressed.get(),
            focused: self.runtime.focused.get(),
        };
        let default_visuals = self.runtime.default_visuals(visual_state);
        RangeSliderSurface {
            model: self.model.clone(),
            runtime: Rc::clone(&self.runtime),
            track: self
                .model
                .track_child()
                .unwrap_or_else(|| default_visuals.0),
            trail: self
                .model
                .trail_child()
                .unwrap_or_else(|| default_visuals.1),
            lower_thumb: self
                .model
                .lower_thumb_child()
                .unwrap_or_else(|| default_visuals.2),
            upper_thumb: self
                .model
                .upper_thumb_child()
                .unwrap_or_else(|| default_visuals.3),
        }
    }
}

impl<T: RangeValue> Widget for RangeSlider<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "RangeSlider", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "RangeSlider"
    }
}

impl<T: RangeValue> PortableWidget for RangeSlider<T> {}

struct RangeSliderSurface<T: RangeValue> {
    model: RangeSlider<T>,
    runtime: Rc<RangeSliderRuntime<T>>,
    track: ChildBuilder,
    trail: ChildBuilder,
    lower_thumb: ChildBuilder,
    upper_thumb: ChildBuilder,
}

impl<T: RangeValue> Widget for RangeSliderSurface<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let track_geometry = Rc::new(Cell::new(SliderVisualGeometry::default()));
        let trail_geometry = Rc::new(Cell::new(SliderVisualGeometry::default()));
        let lower_thumb_geometry = Rc::new(Cell::new(SliderVisualGeometry::default()));
        let upper_thumb_geometry = Rc::new(Cell::new(SliderVisualGeometry::default()));
        let track = Element::boxed(RawSliderVisualSlot {
            child: self.track.build(ctx),
            geometry: Rc::clone(&track_geometry),
        });
        let trail = Element::boxed(RawSliderVisualSlot {
            child: self.trail.build(ctx),
            geometry: Rc::clone(&trail_geometry),
        });
        let lower_thumb = Element::boxed(RawSliderVisualSlot {
            child: self.lower_thumb.build(ctx),
            geometry: Rc::clone(&lower_thumb_geometry),
        });
        let upper_thumb = Element::boxed(RawSliderVisualSlot {
            child: self.upper_thumb.build(ctx),
            geometry: Rc::clone(&upper_thumb_geometry),
        });
        Element::boxed(RawRangeSlider {
            model: self.model,
            runtime: self.runtime,
            track,
            trail,
            lower_thumb,
            upper_thumb,
            bounds: InteractionBounds::new(),
            visual_inset: Cell::new(0.0),
            track_geometry,
            trail_geometry,
            lower_thumb_geometry,
            upper_thumb_geometry,
        })
    }

    fn debug_name(&self) -> &'static str {
        "RawRangeSlider"
    }
}

impl<T: RangeValue> PortableWidget for RangeSliderSurface<T> {}

struct RawRangeSlider<T: RangeValue> {
    model: RangeSlider<T>,
    runtime: Rc<RangeSliderRuntime<T>>,
    track: AnyElement,
    trail: AnyElement,
    lower_thumb: AnyElement,
    upper_thumb: AnyElement,
    bounds: InteractionBounds,
    /// Logical inset shared by both visual thumbs at the range endpoints.
    visual_inset: Cell<f32>,
    track_geometry: Rc<Cell<SliderVisualGeometry>>,
    trail_geometry: Rc<Cell<SliderVisualGeometry>>,
    lower_thumb_geometry: Rc<Cell<SliderVisualGeometry>>,
    upper_thumb_geometry: Rc<Cell<SliderVisualGeometry>>,
}

impl<T: RangeValue> RawRangeSlider<T> {
    fn hit_test(&self, x: f32, y: f32) -> bool {
        self.bounds
            .get_bounds()
            .is_some_and(|bounds| bounds.width > 0.0 && bounds.height > 0.0)
            && self.bounds.is_inside(x, y)
    }

    fn track_value(&self, x: f32) -> Option<T> {
        let bounds = self.bounds.get_bounds()?;
        // Keep pointer-to-value conversion aligned with the inset visual
        // track; endpoint thumbs therefore do not jump on press.
        let inset = self.visual_inset.get().max(0.0).min(bounds.width / 2.0);
        let track_width = (bounds.width - inset * 2.0).max(0.0);
        self.model
            .value_at_position(
                f64::from((x - bounds.x - inset).clamp(0.0, track_width)),
                f64::from(track_width),
            )
            .ok()
    }

    fn choose_thumb(&self, x: f32) -> RangeThumb {
        let bounds = self.bounds.get_bounds();
        let inset = bounds
            .map(|bounds| self.visual_inset.get().max(0.0).min(bounds.width / 2.0))
            .unwrap_or(0.0);
        let width = bounds
            .map(|bounds| f64::from((bounds.width - inset * 2.0).max(0.0)))
            .unwrap_or(0.0);
        let position = bounds
            .map(|bounds| f64::from(x - bounds.x))
            .map(|position| (position - f64::from(inset)).clamp(0.0, width))
            .unwrap_or(0.0);
        let lower = self
            .model
            .position_for_value(self.runtime.lower.get(), width)
            .unwrap_or(0.0);
        let upper = self
            .model
            .position_for_value(self.runtime.upper.get(), width)
            .unwrap_or(0.0);
        if (position - lower).abs() <= (position - upper).abs() {
            RangeThumb::Lower
        } else {
            RangeThumb::Upper
        }
    }

    fn propose(&self, values: (T, T)) {
        if self.runtime.lower.get() == values.0 && self.runtime.upper.get() == values.1 {
            return;
        }
        self.runtime.lower.set(values.0);
        self.runtime.upper.set(values.1);
        if self.runtime.last_proposed.replace(Some(values)) == Some(values) {
            return;
        }
        if let Some(callback) = self.model.on_change.as_ref() {
            callback(values);
        }
    }

    fn propose_at(&self, x: f32, thumb: RangeThumb) {
        let Some(value) = self.track_value(x) else {
            return;
        };
        let mut candidate = self.model.clone();
        if candidate
            .set_values(self.runtime.lower.get(), self.runtime.upper.get())
            .is_err()
        {
            return;
        }
        let Ok(changed) = (match thumb {
            RangeThumb::Lower => candidate.set_lower(value),
            RangeThumb::Upper => candidate.set_upper(value),
        }) else {
            return;
        };
        if changed {
            self.propose((candidate.lower(), candidate.upper()));
        }
    }

    fn key_action(key: &NamedKey) -> Option<SliderKey> {
        RawSlider::<T>::key_action(key)
    }

    fn position_px(&self, ctx: &BuildContext, size: ResolvedSize, value: T, inset: f32) -> f32 {
        let width = size.width.max(0.0);
        // Both range thumbs share one inset, keeping their value mapping
        // aligned even when custom lower and upper thumbs have different
        // widths.
        let inset = inset.max(0.0).min(width / 2.0);
        let track_width = (width - inset * 2.0).max(0.0);
        let logical_width = (track_width / ctx.scale).max(0.0);
        let position = self
            .model
            .position_for_value(value, logical_width as f64)
            .unwrap_or(0.0) as f32
            * ctx.scale;
        (inset + position).clamp(inset, width - inset)
    }

    /// Works out where the visuals go and publishes it to their slots. See
    /// [`RawSlider::place_visuals`] for why this also runs while the render
    /// tree syncs.
    fn place_visuals(&self, ctx: &BuildContext) -> (ResolvedSize, RangePlacement) {
        let size = self.computed_size(ctx);
        let thumb_inset = self.thumb_inset(ctx, size);
        self.visual_inset
            .set((thumb_inset / ctx.scale.max(f32::EPSILON)).max(0.0));
        let child_ctx = child_context(ctx, size);
        let logical_scale = ctx.scale.max(f32::EPSILON);

        let track_size = self.track.computed_size(&child_ctx);
        let track = SliderVisualGeometry {
            position: Vec2d {
                x: 0.0,
                y: (size.height - track_size.height) / 2.0,
            },
            clip: None,
        };

        let lower = self.position_px(ctx, size, self.runtime.lower.get(), thumb_inset);
        let upper = self.position_px(ctx, size, self.runtime.upper.get(), thumb_inset);
        let trail_size = self.trail.computed_size(&child_ctx);
        let clip_x = lower.min(upper);
        let clip_width = (upper - lower).abs().min(size.width.max(0.0));
        let trail_height = trail_size.height.max(0.0);
        let trail = SliderVisualGeometry {
            position: Vec2d {
                x: 0.0,
                y: (size.height - trail_size.height) / 2.0,
            },
            clip: Some(Rect::new(
                clip_x / logical_scale,
                0.0,
                clip_width / logical_scale,
                trail_height / logical_scale,
            )),
        };

        let lower_size = self.lower_thumb.computed_size(&child_ctx);
        let lower_geometry = SliderVisualGeometry {
            position: Vec2d {
                x: lower - lower_size.width / 2.0,
                y: (size.height - lower_size.height) / 2.0,
            },
            clip: None,
        };
        let upper_size = self.upper_thumb.computed_size(&child_ctx);
        let upper_geometry = SliderVisualGeometry {
            position: Vec2d {
                x: upper - upper_size.width / 2.0,
                y: (size.height - upper_size.height) / 2.0,
            },
            clip: None,
        };

        self.track_geometry.set(track);
        self.trail_geometry.set(trail);
        self.lower_thumb_geometry.set(lower_geometry);
        self.upper_thumb_geometry.set(upper_geometry);
        (
            size,
            RangePlacement {
                track,
                trail,
                lower: lower_geometry,
                upper: upper_geometry,
            },
        )
    }

    fn thumb_inset(&self, ctx: &BuildContext, size: ResolvedSize) -> f32 {
        let child_ctx = child_context(ctx, size);
        let lower = self.lower_thumb.computed_size(&child_ctx).width / 2.0;
        let upper = self.upper_thumb.computed_size(&child_ctx).width / 2.0;
        lower.max(upper).max(0.0).min(size.width.max(0.0) / 2.0)
    }
}

impl<T: RangeValue> VisitorElement for RawRangeSlider<T> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.track.as_ref());
        visitor(self.trail.as_ref());
        visitor(self.lower_thumb.as_ref());
        visitor(self.upper_thumb.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "RawRangeSlider"
    }
}

impl<T: RangeValue> EventElement for RawRangeSlider<T> {
    fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn focus_node(&self) -> Option<&FocusNode> {
        (!self.model.is_disabled()).then_some(&self.runtime.focus_node)
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerDown(pointer)
                if pointer.button == PointerButton::Primary
                    && !self.model.is_disabled()
                    && self.hit_test(pointer.pos.x, pointer.pos.y) =>
            {
                let key = PointerKey::new(pointer.source, pointer.id);
                let thumb = self.choose_thumb(pointer.pos.x);
                // See `RawSlider`: the thumb follows this finger from the press.
                aimer_widget::claim_pointer(key);
                self.runtime.active_pointer.set(Some(key));
                self.runtime.active_thumb.set(Some(thumb));
                self.runtime.pressed.set(true);
                self.runtime.request_visual_rebuild();
                self.propose_at(pointer.pos.x, thumb);
                EventResult::consumed()
                    .with_pointer_capture(key)
                    .with_redraw()
            }
            ElementEvent::PointerMove(pointer) => {
                if self.model.is_disabled() {
                    return EventResult::ignored();
                }
                let key = PointerKey::new(pointer.source, pointer.id);
                if self.runtime.active_pointer.get() == Some(key) {
                    if let Some(thumb) = self.runtime.active_thumb.get() {
                        self.propose_at(pointer.pos.x, thumb);
                    }
                    EventResult::consumed().with_redraw()
                } else {
                    let inside = self.hit_test(pointer.pos.x, pointer.pos.y);
                    if self.runtime.hovered.replace(inside) != inside {
                        EventResult::redraw()
                    } else {
                        EventResult::ignored()
                    }
                }
            }
            ElementEvent::PointerUp(pointer)
                if self.runtime.active_pointer.get()
                    == Some(PointerKey::new(pointer.source, pointer.id)) =>
            {
                let key = PointerKey::new(pointer.source, pointer.id);
                if let Some(thumb) = self.runtime.active_thumb.get() {
                    self.propose_at(pointer.pos.x, thumb);
                }
                self.runtime.active_pointer.set(None);
                self.runtime.active_thumb.set(None);
                self.runtime.pressed.set(false);
                self.runtime.request_visual_rebuild();
                EventResult::consumed()
                    .with_pointer_release(key)
                    .with_redraw()
            }
            ElementEvent::PointerExited(_, _) => {
                if self.runtime.hovered.replace(false) {
                    EventResult::redraw()
                } else {
                    EventResult::ignored()
                }
            }
            ElementEvent::FocusGained => {
                self.runtime.focused.set(true);
                self.runtime.request_visual_rebuild();
                EventResult::redraw()
            }
            ElementEvent::FocusLost => {
                self.runtime.focused.set(false);
                self.runtime.request_visual_rebuild();
                EventResult::redraw()
            }
            ElementEvent::KeyInput {
                key,
                action: KeyAction::Pressed | KeyAction::Repeat,
                ..
            } => {
                if self.model.is_disabled() {
                    return EventResult::ignored();
                }
                let Some(key) = Self::key_action(key) else {
                    return EventResult::ignored();
                };
                let thumb = self.runtime.active_thumb.get().unwrap_or(RangeThumb::Lower);
                let mut candidate = self.model.clone();
                if candidate
                    .set_values(self.runtime.lower.get(), self.runtime.upper.get())
                    .is_err()
                {
                    return EventResult::ignored();
                }
                let Ok(changed) = candidate.handle_key(thumb, key) else {
                    return EventResult::ignored();
                };
                if changed {
                    self.propose((candidate.lower(), candidate.upper()));
                }
                EventResult::consumed().with_redraw()
            }
            ElementEvent::Cancel => {
                if self.runtime.active_pointer.get().is_some() {
                    self.runtime.active_pointer.set(None);
                    self.runtime.active_thumb.set(None);
                    self.runtime.pressed.set(false);
                    self.runtime.request_visual_rebuild();
                    EventResult::consumed().with_redraw()
                } else {
                    EventResult::ignored()
                }
            }
            _ => EventResult::ignored(),
        }
    }
}

impl<T: RangeValue> LayoutElement for RawRangeSlider<T> {
    fn size(&self) -> Option<Size> {
        Some(Size::new(
            self.model.widget_width(),
            self.model.widget_height(),
        ))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let requested = Size::new(self.model.widget_width(), self.model.widget_height()).resolve(
            &ResolvedSize {
                width: ctx.box_constraint.max_width,
                height: ctx.box_constraint.max_height,
            },
            ctx.scale,
        );
        ResolvedSize {
            width: requested
                .width
                .clamp(ctx.box_constraint.min_width, ctx.box_constraint.max_width),
            height: requested
                .height
                .clamp(ctx.box_constraint.min_height, ctx.box_constraint.max_height),
        }
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let (size, placement) = self.place_visuals(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
        let child_ctx = child_context(ctx, size);
        layout_child(&self.track, &child_ctx, placement.track.position);
        layout_child(&self.trail, &child_ctx, placement.trail.position);
        layout_child(&self.lower_thumb, &child_ctx, placement.lower.position);
        layout_child(&self.upper_thumb, &child_ctx, placement.upper.position);
        size
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl<T: RangeValue> Drawable for RawRangeSlider<T> {
    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("RawRangeSlider")
    }

    fn update(&self, ctx: &BuildContext) {
        let (size, placement) = self.place_visuals(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
        let child_ctx = child_context(ctx, size);
        let track_height = (4.0 * ctx.scale).min(size.height.max(0.0));
        if size.width <= 0.0 || track_height <= 0.0 {
            return;
        }
        draw_child(&self.track, &child_ctx, placement.track.position);

        draw_child(&self.trail, &child_ctx, placement.trail.position);

        draw_child(&self.lower_thumb, &child_ctx, placement.lower.position);
        draw_child(&self.upper_thumb, &child_ctx, placement.upper.position);
    }

    /// Publishes the visuals' placement before the render tree reads their
    /// positions. See [`RawSlider::place_visuals`].
    fn retained_v2_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        self.place_visuals(ctx);
        None
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        if !ctx.scale.is_finite() || ctx.scale <= 0.0 {
            return false;
        }
        let size = self.computed_size(ctx);
        let width = size.width / ctx.scale;
        let height = size.height / ctx.scale;
        width.is_finite()
            && height.is_finite()
            && width > 0.0
            && height > 0.0
            && width <= 1_000_000.0
            && height <= 1_000_000.0
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}
}

impl<T: RangeValue> Rebuildable for RawRangeSlider<T> {}
impl<T: RangeValue> PortableWidget for RawRangeSlider<T> {}

impl<T: RangeValue> Widget for RawRangeSlider<T> {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }

    fn debug_name(&self) -> &'static str {
        "RawRangeSlider"
    }
}

fn layout_child(child: &AnyElement, ctx: &BuildContext, offset: Vec2d) {
    ctx.canvas.save();
    ctx.canvas.translate(offset);
    child.layout(ctx);
    ctx.canvas.restore();
}

fn child_context<'a>(ctx: &BuildContext<'a>, size: ResolvedSize) -> BuildContext<'a> {
    BuildContext {
        parent_size: size,
        box_constraint: BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width.max(0.0),
            max_height: size.height.max(0.0),
        },
        ..ctx.clone()
    }
}

fn draw_child(child: &AnyElement, ctx: &BuildContext, offset: Vec2d) {
    ctx.canvas.save();
    ctx.canvas.translate(offset);
    child.update(ctx);
    ctx.canvas.restore();
}
