use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey, ScrollDeltaKind};
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_modal::{
    Anchor, AnchorHandle, Floating, FloatingAlign, FloatingSide, ModalController, OverflowPolicy,
};
use aimer_style::ThemeTokens;
use aimer_widget::InteractionBounds;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, FocusNode, LayoutElement, PointerKey,
    PortableWidget, Rebuildable, State, StateUpdater, StatefulElement, StatefulWidget,
    VisitorElement, Widget,
};

use crate::{PICKER_FIELD_HEIGHT, PICKER_FOOTER_HEIGHT, PickerOutcome, TimeOfDay, TimePicker};

use super::time_wheel::{
    self, TimeWheelDrag, column_at, row_at, scroll_steps, time_from_drag, time_from_row,
    time_from_scroll_steps,
};
use super::widget_helpers::{default_time_picker, format_time};
use super::{paint, theme};

/// A callback invoked after a standalone time picker confirms a value.
pub type TimeSelectionCallback = Rc<dyn Fn(TimeOfDay)>;

/// A retained keyboard- and pointer-accessible standalone time picker.
///
/// The trigger remains compact while the scrollable wheel is presented in an
/// anchored overlay owned by the application's modal host.
#[derive(Clone)]
pub struct TimePickerView {
    picker: TimePicker,
    width: f32,
    height: f32,
    use_24_hours: bool,
    on_selection: Option<TimeSelectionCallback>,
}

impl Default for TimePickerView {
    fn default() -> Self {
        Self::new()
    }
}

impl TimePickerView {
    /// Creates a closed picker initialized to midnight and using a 12-hour
    /// wheel. Call [`Self::use_24_hours`] to choose a 24-hour wheel.
    #[inline]
    pub fn new() -> Self {
        Self {
            picker: default_time_picker(),
            width: 320.0,
            height: 300.0,
            use_24_hours: false,
            on_selection: None,
        }
    }

    /// Replaces the time-picker model.
    #[inline]
    pub fn picker(mut self, picker: TimePicker) -> Self {
        self.picker = picker;
        self
    }

    /// Sets the logical width of the trigger and popup.
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        if width.is_finite() && width >= 0.0 {
            self.width = width;
        }
        self
    }

    /// Sets the total logical height reserved for the popup, including its
    /// wheel and action footer.
    #[inline]
    pub fn height(mut self, height: f32) -> Self {
        if height.is_finite() && height >= 0.0 {
            self.height = height;
        }
        self
    }

    /// Chooses between a 24-hour wheel and a 12-hour wheel with one AM/PM
    /// selector.
    #[inline]
    pub fn use_24_hours(mut self, use_24_hours: bool) -> Self {
        self.use_24_hours = use_24_hours;
        self
    }

    /// Registers a callback invoked after a time draft is confirmed.
    #[inline]
    pub fn on_selection<F>(mut self, callback: F) -> Self
    where
        F: Fn(TimeOfDay) + 'static,
    {
        self.on_selection = Some(Rc::new(callback));
        self
    }
}

struct TimePickerRuntime {
    picker: RefCell<TimePicker>,
    focus_node: FocusNode,
    focused: Cell<bool>,
    active_column: Cell<usize>,
    drag: RefCell<Option<TimeWheelDrag>>,
    scroll_remainder: Cell<f32>,
    anchor: AnchorHandle,
    overlay_active: Cell<bool>,
}

/// Retained state for [`TimePickerView`].
pub struct TimePickerViewState {
    model: TimePickerView,
    runtime: Rc<TimePickerRuntime>,
}

impl TimePickerViewState {
    /// Returns the last confirmed time.
    #[inline]
    pub fn value(&self) -> TimeOfDay {
        self.runtime.picker.borrow().value()
    }

    /// Returns whether the time editor is open.
    #[inline]
    pub fn is_open(&self) -> bool {
        self.runtime.picker.borrow().is_open()
    }

    /// Returns semantic state for the retained time picker model.
    #[inline]
    pub fn semantics(&self) -> super::super::TimePickerSemantics {
        self.runtime.picker.borrow().semantics()
    }

    /// Returns the wheel column currently receiving keyboard and scroll input.
    #[inline]
    pub fn active_column(&self) -> usize {
        self.runtime.active_column.get()
    }
}

impl StatefulWidget for TimePickerView {
    type State = TimePickerViewState;

    fn create_state(self) -> Self::State {
        let runtime = Rc::new(TimePickerRuntime {
            picker: RefCell::new(self.picker.clone()),
            focus_node: FocusNode::new(),
            focused: Cell::new(false),
            active_column: Cell::new(0),
            drag: RefCell::new(None),
            scroll_remainder: Cell::new(0.0),
            anchor: AnchorHandle::new(),
            overlay_active: Cell::new(false),
        });
        TimePickerViewState {
            model: self,
            runtime,
        }
    }
}

impl State<TimePickerView> for TimePickerViewState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        if self.model.picker != new.model.picker {
            *self.runtime.picker.borrow_mut() = new.model.picker.clone();
        }
        self.model = new.model;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        TimePickerSurface {
            runtime: Rc::clone(&self.runtime),
            width: self.model.width,
            height: self.model.height,
            use_24_hours: self.model.use_24_hours,
            on_selection: self.model.on_selection.clone(),
            tokens: theme::tokens(ctx),
            popup: false,
        }
    }
}

impl Widget for TimePickerView {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "TimePickerView", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "TimePickerView"
    }
}

impl PortableWidget for TimePickerView {}

struct TimePickerSurface {
    runtime: Rc<TimePickerRuntime>,
    width: f32,
    height: f32,
    use_24_hours: bool,
    on_selection: Option<TimeSelectionCallback>,
    tokens: ThemeTokens,
    popup: bool,
}

impl Widget for TimePickerSurface {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let raw = RawTimePicker {
            runtime: self.runtime,
            width: self.width,
            height: self.height,
            use_24_hours: self.use_24_hours,
            on_selection: self.on_selection,
            tokens: self.tokens,
            bounds: InteractionBounds::new(),
            popup: self.popup,
        };
        if self.popup {
            raw.to_element(ctx)
        } else {
            Anchor::new()
                .handle(raw.runtime.anchor.clone())
                .child(raw)
                .to_element(ctx)
        }
    }

    fn debug_name(&self) -> &'static str {
        "RawTimePicker"
    }
}

impl PortableWidget for TimePickerSurface {}

struct RawTimePicker {
    runtime: Rc<TimePickerRuntime>,
    width: f32,
    height: f32,
    use_24_hours: bool,
    on_selection: Option<TimeSelectionCallback>,
    tokens: ThemeTokens,
    bounds: InteractionBounds,
    popup: bool,
}

impl RawTimePicker {
    fn layout_height(&self) -> f32 {
        if self.popup {
            self.height
        } else {
            PICKER_FIELD_HEIGHT
        }
    }

    fn hit_test(&self, x: f32, y: f32) -> bool {
        self.bounds
            .get_bounds()
            .is_some_and(|bounds| bounds.width > 0.0 && bounds.height > 0.0)
            && self.bounds.is_inside(x, y)
    }

    fn open(&self) -> EventResult {
        self.runtime.picker.borrow_mut().open();
        self.runtime.active_column.set(0);
        self.runtime.drag.replace(None);
        self.runtime.scroll_remainder.set(0.0);
        self.runtime.overlay_active.set(true);
        if !self.popup {
            Floating::new()
                .anchor(self.runtime.anchor.clone())
                .side(FloatingSide::Bottom)
                .align(FloatingAlign::Start)
                .gap(4.0)
                .overflow(OverflowPolicy::Flip)
                .child(TimePickerSurface {
                    runtime: Rc::clone(&self.runtime),
                    width: self.width,
                    height: self.height,
                    use_24_hours: self.use_24_hours,
                    on_selection: self.on_selection.clone(),
                    tokens: self.tokens,
                    popup: true,
                })
                .show();
        }
        EventResult::consumed().with_redraw()
    }

    fn commit(&self) -> EventResult {
        match self.runtime.picker.borrow_mut().confirm() {
            Ok(PickerOutcome::Confirmed(value)) => {
                let has_overlay = self.runtime.overlay_active.replace(false);
                self.runtime.drag.replace(None);
                if has_overlay {
                    ModalController::dismiss_top();
                }
                if let Some(callback) = self.on_selection.as_ref() {
                    callback(value);
                }
                EventResult::consumed().with_redraw()
            }
            _ => EventResult::consumed(),
        }
    }

    fn cancel(&self, reason: super::super::CancelReason) -> EventResult {
        self.finish_cancel(reason, true)
    }

    fn cancel_from_host(&self) -> EventResult {
        self.finish_cancel(super::super::CancelReason::OutsideClick, false)
    }

    fn finish_cancel(
        &self,
        reason: super::super::CancelReason,
        dismiss_overlay: bool,
    ) -> EventResult {
        if self.runtime.picker.borrow_mut().cancel(reason).is_ok() {
            let has_overlay = self.runtime.overlay_active.replace(false);
            self.runtime.drag.replace(None);
            if dismiss_overlay && has_overlay {
                ModalController::dismiss_top();
            }
            EventResult::consumed().with_redraw()
        } else {
            EventResult::consumed()
        }
    }

    fn set_time(&self, time: TimeOfDay) -> EventResult {
        if self.runtime.picker.borrow_mut().set_time(time).is_ok() {
            EventResult::consumed().with_redraw()
        } else {
            EventResult::consumed()
        }
    }

    fn begin_drag(
        &self,
        pointer: &aimer_events::pointer::PointerInfo,
        x: f32,
        y: f32,
    ) -> EventResult {
        let Some(bounds) = self.bounds.get_bounds() else {
            return EventResult::ignored();
        };
        let Some(row) = row_at(y, bounds.height, 0.0) else {
            return EventResult::ignored();
        };
        let Some(column) = column_at(x, bounds.width, self.use_24_hours) else {
            return EventResult::ignored();
        };
        let selected = {
            let mut picker = self.runtime.picker.borrow_mut();
            let current = picker.draft();
            let Some(next) = time_from_row(current, column, row, self.use_24_hours) else {
                return EventResult::ignored();
            };
            if picker.set_time(next).is_err() {
                return EventResult::consumed();
            }
            next
        };
        let key = PointerKey::new(pointer.source, pointer.id);
        self.runtime.active_column.set(column);
        self.runtime.drag.replace(Some(TimeWheelDrag {
            pointer: key,
            column,
            start_y: y,
            start_time: selected,
        }));
        EventResult::consumed()
            .with_pointer_capture(key)
            .with_redraw()
    }

    fn update_drag(&self, pointer: &aimer_events::pointer::PointerInfo, y: f32) -> EventResult {
        let key = PointerKey::new(pointer.source, pointer.id);
        let Some(drag) = self.runtime.drag.borrow().as_ref().copied() else {
            return EventResult::ignored();
        };
        if drag.pointer != key {
            return EventResult::ignored();
        }
        let Some(next) = time_from_drag(
            drag.start_time,
            drag.column,
            drag.start_y,
            y,
            self.use_24_hours,
        ) else {
            return EventResult::consumed();
        };
        self.set_time(next)
    }

    fn update_hover_column(&self, pointer: &PointerInfo) -> EventResult {
        let Some(bounds) = self.bounds.get_bounds() else {
            return EventResult::ignored();
        };
        let x = pointer.pos.x - bounds.x;
        let y = pointer.pos.y - bounds.y;
        if row_at(y, bounds.height, 0.0).is_none() {
            return EventResult::ignored();
        }
        let Some(column) = column_at(x, bounds.width, self.use_24_hours) else {
            return EventResult::ignored();
        };
        if self.runtime.active_column.replace(column) != column {
            EventResult::redraw()
        } else {
            EventResult::ignored()
        }
    }

    fn end_drag(&self, pointer: &aimer_events::pointer::PointerInfo, y: f32) -> EventResult {
        let key = PointerKey::new(pointer.source, pointer.id);
        let Some(drag) = self.runtime.drag.borrow().as_ref().copied() else {
            return EventResult::ignored();
        };
        if drag.pointer != key {
            return EventResult::ignored();
        }
        let result = if let Some(next) = time_from_drag(
            drag.start_time,
            drag.column,
            drag.start_y,
            y,
            self.use_24_hours,
        ) {
            self.set_time(next)
        } else {
            EventResult::consumed()
        };
        self.runtime.drag.replace(None);
        result.with_pointer_release(key)
    }

    fn handle_scroll(&self, delta_y: f32, kind: ScrollDeltaKind) -> EventResult {
        if !self.runtime.picker.borrow().is_open() {
            return EventResult::ignored();
        }
        let steps = scroll_steps(delta_y, kind, &self.runtime.scroll_remainder);
        if steps == 0 {
            return EventResult::consumed();
        }
        let column = self.runtime.active_column.get();
        let next = {
            let picker = self.runtime.picker.borrow();
            time_from_scroll_steps(picker.draft(), column, steps, self.use_24_hours)
        };
        next.map_or_else(|| EventResult::consumed(), |time| self.set_time(time))
    }

    fn move_column(&self, backwards: bool) -> EventResult {
        let count = time_wheel::column_count(self.use_24_hours);
        let current = self.runtime.active_column.get().min(count - 1);
        let next = if backwards {
            current.checked_sub(1).unwrap_or(count - 1)
        } else {
            (current + 1) % count
        };
        self.runtime.active_column.set(next);
        EventResult::consumed().with_redraw()
    }

    fn adjust_time(&self, increase: bool) -> EventResult {
        let column = self.runtime.active_column.get();
        let steps = if increase { 1 } else { -1 };
        let next = {
            let picker = self.runtime.picker.borrow();
            time_from_scroll_steps(picker.draft(), column, steps, self.use_24_hours)
        };
        next.map_or_else(|| EventResult::ignored(), |time| self.set_time(time))
    }

    fn handle_key(&self, key: &NamedKey, modifiers: &Modifiers) -> EventResult {
        if !self.runtime.picker.borrow().is_open() {
            return if matches!(key, NamedKey::Enter) {
                self.open()
            } else {
                EventResult::ignored()
            };
        }
        if matches!(key, NamedKey::Escape) {
            return self.cancel(super::super::CancelReason::Escape);
        }
        if matches!(key, NamedKey::Tab)
            || (modifiers.ctrl && matches!(key, NamedKey::ArrowLeft | NamedKey::ArrowRight))
        {
            return self.move_column(
                modifiers.shift || (modifiers.ctrl && matches!(key, NamedKey::ArrowLeft)),
            );
        }
        match key {
            NamedKey::ArrowUp | NamedKey::ArrowRight => self.adjust_time(true),
            NamedKey::ArrowDown | NamedKey::ArrowLeft => self.adjust_time(false),
            NamedKey::Enter => self.commit(),
            _ => EventResult::ignored(),
        }
    }
}

impl VisitorElement for RawTimePicker {
    fn debug_name(&self) -> &'static str {
        "RawTimePicker"
    }
}

impl EventElement for RawTimePicker {
    fn focus_node(&self) -> Option<&FocusNode> {
        Some(&self.runtime.focus_node)
    }

    fn autofocus(&self) -> bool {
        self.popup
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if self.popup && matches!(event, ElementEvent::Cancel) {
            return self.cancel_from_host();
        }
        match event {
            ElementEvent::PointerDown(pointer)
                if pointer.button == PointerButton::Primary
                    && self.hit_test(pointer.pos.x, pointer.pos.y) =>
            {
                self.runtime.focused.set(true);
                let bounds = self.bounds.get_bounds().unwrap_or_default();
                let x = pointer.pos.x - bounds.x;
                let y = pointer.pos.y - bounds.y;
                if self.popup {
                    if y >= bounds.height - PICKER_FOOTER_HEIGHT {
                        if x < bounds.width / 2.0 {
                            self.cancel(super::super::CancelReason::OutsideClick)
                        } else {
                            self.commit()
                        }
                    } else if !self.runtime.picker.borrow().is_open() {
                        EventResult::consumed()
                    } else {
                        self.begin_drag(pointer, x, y)
                    }
                } else if y < PICKER_FIELD_HEIGHT {
                    if self.runtime.picker.borrow().is_open() {
                        self.cancel(super::super::CancelReason::OutsideClick)
                    } else {
                        self.open()
                    }
                } else {
                    EventResult::consumed()
                }
            }
            ElementEvent::PointerMove(pointer) => {
                let bounds = self.bounds.get_bounds().unwrap_or_default();
                let result = self.update_drag(pointer, pointer.pos.y - bounds.y);
                if result.is_consumed() {
                    result
                } else {
                    self.update_hover_column(pointer)
                }
            }
            ElementEvent::PointerUp(pointer) => {
                let bounds = self.bounds.get_bounds().unwrap_or_default();
                self.end_drag(pointer, pointer.pos.y - bounds.y)
            }
            ElementEvent::Scroll { delta, kind, .. } if self.popup => {
                self.handle_scroll(delta.y, *kind)
            }
            ElementEvent::FocusGained => {
                self.runtime.focused.set(true);
                EventResult::redraw()
            }
            ElementEvent::FocusLost => {
                self.runtime.focused.set(false);
                EventResult::redraw()
            }
            ElementEvent::Cancel if !self.runtime.overlay_active.get() => self.cancel_from_host(),
            ElementEvent::KeyInput {
                key,
                action: KeyAction::Pressed | KeyAction::Repeat,
                modifiers,
            } => self.handle_key(key, modifiers),
            _ => EventResult::ignored(),
        }
    }
}

impl LayoutElement for RawTimePicker {
    fn size(&self) -> Option<Size> {
        Some(Size::new(self.width, self.layout_height()))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let requested = Size::new(self.width, self.layout_height()).resolve(
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
        let size = self.computed_size(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
        size
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl Drawable for RawTimePicker {
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
        self.bounds.disagreement("RawTimePicker")
    }

    fn update(&self, ctx: &BuildContext) {
        // The surface is painted by `paint_local_v2`; this only publishes where
        // it sits.
        let size = self.computed_size(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite() && ctx.scale > 0.0
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let picker = self.runtime.picker.borrow();
        let value = picker.draft();
        paint::record_local_v2(ctx, |painter| {
            if self.popup {
                paint::draw_time_picker(
                    painter,
                    value,
                    size.width,
                    size.height,
                    0.0,
                    self.runtime.active_column.get(),
                    self.use_24_hours,
                    &self.tokens,
                );
                paint::draw_done_footer(
                    painter,
                    size.width,
                    size.height,
                    self.runtime.focused.get(),
                    &self.tokens,
                );
                paint::draw_overlay_border(painter, size.width, size.height, &self.tokens);
            } else {
                paint::draw_picker_field(
                    painter,
                    "Time",
                    format_time(value, self.use_24_hours),
                    size.width,
                    picker.is_open(),
                    self.runtime.focused.get(),
                    &self.tokens,
                );
            }
        });
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
    }
}

impl Rebuildable for RawTimePicker {}
impl PortableWidget for RawTimePicker {}

impl Widget for RawTimePicker {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }

    fn debug_name(&self) -> &'static str {
        "RawTimePicker"
    }
}
