//! Retained widget adapters for the platform-neutral picker models.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::{CALENDAR_HEADER, CALENDAR_WEEKDAYS, PICKER_FIELD_HEIGHT, PICKER_FOOTER_HEIGHT};
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

use super::{
    Calendar, CalendarNavigation, CalendarSemantics, ColorChannel, ColorError, ColorKey,
    ColorPicker, ColorPickerSemantics, Date, DateBounds, DatePickerSemantics, DateSelection,
    DateTime, DateTimePicker, DateTimePickerPolicy, DateTimePickerSemantics, Hsva, PickerOutcome,
    TimeOfDay, TimeZonePolicy,
};
use super::{paint, theme};
mod color;
mod time;
mod time_wheel;
mod widget_helpers;
pub use color::{ColorPickerView, ColorPickerViewState};
pub use time::{TimePickerView, TimePickerViewState, TimeSelectionCallback};
use time_wheel::{
    TimeWheelDrag, column_at, row_at, scroll_steps, time_from_drag, time_from_row,
    time_from_scroll_steps,
};
use widget_helpers::{
    calendar_date_at, calendar_navigation, default_date, default_datetime_picker, format_datetime,
    format_time, popup_height, selection_label,
};

const COLOR_CHANNEL_FIRST_Y: f32 = 116.0;
const COLOR_CHANNEL_ROW_HEIGHT: f32 = 30.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DateTimeField {
    Date,
    Hour,
    Minute,
    Second,
    Period,
}

impl DateTimeField {
    #[inline]
    fn is_date(self) -> bool {
        matches!(self, Self::Date)
    }

    #[inline]
    fn time_column(self) -> usize {
        match self {
            Self::Date | Self::Hour => 0,
            Self::Minute => 1,
            Self::Second => 2,
            Self::Period => 3,
        }
    }
}

/// A callback invoked after a calendar selection changes.
pub type CalendarSelectionCallback = Rc<dyn Fn(DateSelection)>;

/// A callback invoked after a date picker confirms a selection.
pub type DateSelectionCallback = Rc<dyn Fn(DateSelection)>;

/// A callback invoked after a date-time picker confirms a value.
pub type DateTimeSelectionCallback = Rc<dyn Fn(Option<DateTime>)>;

/// A callback invoked after a color picker confirms a color.
pub type ColorSelectionCallback = Rc<dyn Fn(Hsva)>;

/// A retained, keyboard-accessible calendar grid.
///
/// `CalendarView` is the visual adapter for [`Calendar`]. The model remains
/// independent of rendering and locale formatting, while this widget provides
/// the standard pointer and keyboard navigation path. The retained widget state
/// owns focus and selection; selection callbacks are notifications for an
/// application that wants to observe confirmed changes.
#[derive(Clone)]
pub struct CalendarView {
    calendar: Calendar,
    width: f32,
    height: f32,
    on_selection: Option<CalendarSelectionCallback>,
}
impl Default for CalendarView {
    fn default() -> Self {
        Self::new()
    }
}
impl CalendarView {
    /// Creates an unbounded calendar focused on the Gregorian epoch.
    #[inline]
    pub fn new() -> Self {
        Self {
            calendar: Calendar::new(default_date()),
            width: 320.0,
            height: 300.0,
            on_selection: None,
        }
    }

    /// Replaces the calendar model used by this view.
    #[inline]
    pub fn calendar(mut self, calendar: Calendar) -> Self {
        self.calendar = calendar;
        self
    }

    /// Sets the logical width of the calendar.
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        if width.is_finite() && width >= 0.0 {
            self.width = width;
        }
        self
    }

    /// Sets the logical height of the calendar.
    #[inline]
    pub fn height(mut self, height: f32) -> Self {
        if height.is_finite() && height >= 0.0 {
            self.height = height;
        }
        self
    }

    /// Registers a callback for a newly selected date or range.
    #[inline]
    pub fn on_selection<F>(mut self, callback: F) -> Self
    where
        F: Fn(DateSelection) + 'static,
    {
        self.on_selection = Some(Rc::new(callback));
        self
    }
}

struct CalendarRuntime {
    calendar: RefCell<Calendar>,
    focus_node: FocusNode,
    focused: Cell<bool>,
    hovered: Cell<bool>,
}
impl CalendarRuntime {
    fn new(calendar: Calendar) -> Self {
        Self {
            calendar: RefCell::new(calendar),
            focus_node: FocusNode::new(),
            focused: Cell::new(false),
            hovered: Cell::new(false),
        }
    }
}

/// Retained state for [`CalendarView`].
pub struct CalendarViewState {
    model: CalendarView,
    runtime: Rc<CalendarRuntime>,
}
impl CalendarViewState {
    /// Returns the currently focused date.
    #[inline]
    pub fn focused_date(&self) -> Date {
        self.runtime.calendar.borrow().focused_date()
    }

    /// Returns the current date or range selection.
    #[inline]
    pub fn selection(&self) -> DateSelection {
        self.runtime.calendar.borrow().selection()
    }

    /// Returns whether the view currently owns keyboard focus.
    #[inline]
    pub fn is_focused(&self) -> bool {
        self.runtime.focused.get()
    }

    /// Returns semantic state for the retained calendar model.
    #[inline]
    pub fn semantics(&self) -> CalendarSemantics {
        self.runtime.calendar.borrow().semantics()
    }
}
impl StatefulWidget for CalendarView {
    type State = CalendarViewState;

    fn create_state(self) -> Self::State {
        let runtime = Rc::new(CalendarRuntime::new(self.calendar.clone()));
        CalendarViewState {
            model: self,
            runtime,
        }
    }
}
impl State<CalendarView> for CalendarViewState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        if self.model.calendar != new.model.calendar {
            *self.runtime.calendar.borrow_mut() = new.model.calendar.clone();
        }
        self.model = new.model;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        CalendarSurface {
            runtime: Rc::clone(&self.runtime),
            width: self.model.width,
            height: self.model.height,
            on_selection: self.model.on_selection.clone(),
            tokens: theme::tokens(ctx),
        }
    }
}
impl Widget for CalendarView {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "CalendarView", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "CalendarView"
    }
}
impl PortableWidget for CalendarView {}

struct CalendarSurface {
    runtime: Rc<CalendarRuntime>,
    width: f32,
    height: f32,
    on_selection: Option<CalendarSelectionCallback>,
    tokens: ThemeTokens,
}
impl Widget for CalendarSurface {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(RawCalendar {
            runtime: self.runtime,
            width: self.width,
            height: self.height,
            on_selection: self.on_selection,
            tokens: self.tokens,
            bounds: InteractionBounds::new(),
        })
    }

    fn debug_name(&self) -> &'static str {
        "RawCalendar"
    }
}
impl PortableWidget for CalendarSurface {}

struct RawCalendar {
    runtime: Rc<CalendarRuntime>,
    width: f32,
    height: f32,
    on_selection: Option<CalendarSelectionCallback>,
    tokens: ThemeTokens,
    bounds: InteractionBounds,
}
impl RawCalendar {
    fn hit_test(&self, x: f32, y: f32) -> bool {
        self.bounds
            .get_bounds()
            .is_some_and(|bounds| bounds.width > 0.0 && bounds.height > 0.0)
            && self.bounds.is_inside(x, y)
    }

    fn select_focused(&self) -> EventResult {
        let selection = {
            let mut calendar = self.runtime.calendar.borrow_mut();
            let focused = calendar.focused_date();
            match calendar.select(focused) {
                Ok(selection) => selection,
                Err(_) => return EventResult::ignored(),
            }
        };
        if let Some(callback) = self.on_selection.as_ref() {
            callback(selection);
        }
        EventResult::consumed().with_redraw()
    }

    fn navigate(&self, navigation: CalendarNavigation) -> EventResult {
        if self.runtime.calendar.borrow_mut().navigate(navigation) {
            EventResult::consumed().with_redraw()
        } else {
            EventResult::consumed()
        }
    }

    fn select_at(&self, x: f32, y: f32) -> EventResult {
        let Some(bounds) = self.bounds.get_bounds() else {
            return EventResult::ignored();
        };
        let date = {
            let calendar = self.runtime.calendar.borrow();
            calendar_date_at(
                &calendar,
                Vec2d::default(),
                bounds.width,
                bounds.height,
                x,
                y,
            )
        };
        let Some(date) = date else {
            return EventResult::ignored();
        };
        let selection = {
            let mut calendar = self.runtime.calendar.borrow_mut();
            match calendar.select(date) {
                Ok(selection) => selection,
                Err(_) => return EventResult::consumed(),
            }
        };
        if let Some(callback) = self.on_selection.as_ref() {
            callback(selection);
        }
        EventResult::consumed().with_redraw()
    }

    fn handle_key(&self, key: &NamedKey, modifiers: &Modifiers) -> EventResult {
        if let Some(navigation) = calendar_navigation(key, modifiers) {
            return self.navigate(navigation);
        }
        if matches!(key, NamedKey::Enter) {
            return self.select_focused();
        }
        EventResult::ignored()
    }
}
impl VisitorElement for RawCalendar {
    fn debug_name(&self) -> &'static str {
        "RawCalendar"
    }
}
impl EventElement for RawCalendar {
    fn focus_node(&self) -> Option<&FocusNode> {
        Some(&self.runtime.focus_node)
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerDown(pointer)
                if pointer.button == PointerButton::Primary
                    && self.hit_test(pointer.pos.x, pointer.pos.y) =>
            {
                self.runtime.focused.set(true);
                let bounds = self.bounds.get_bounds().unwrap_or_default();
                self.select_at(pointer.pos.x - bounds.x, pointer.pos.y - bounds.y)
            }
            ElementEvent::PointerMove(pointer) => {
                let inside = self.hit_test(pointer.pos.x, pointer.pos.y);
                if self.runtime.hovered.replace(inside) != inside {
                    EventResult::redraw()
                } else {
                    EventResult::ignored()
                }
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
                EventResult::redraw()
            }
            ElementEvent::FocusLost => {
                self.runtime.focused.set(false);
                EventResult::redraw()
            }
            ElementEvent::KeyInput {
                key,
                action: KeyAction::Pressed | KeyAction::Repeat,
                modifiers,
            } => self.handle_key(key, modifiers),
            _ => EventResult::ignored(),
        }
    }
}
impl LayoutElement for RawCalendar {
    fn size(&self) -> Option<Size> {
        Some(Size::new(self.width, self.height))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let requested = Size::new(self.width, self.height).resolve(
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
impl Drawable for RawCalendar {
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
        self.bounds.disagreement("RawCalendar")
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
        let calendar = self.runtime.calendar.borrow();
        paint::record_local_v2(ctx, |painter| {
            paint::draw_calendar(
                painter,
                &calendar,
                Vec2d::default(),
                size.width,
                size.height,
                self.runtime.focused.get(),
                &self.tokens,
            );
        });
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
    }
}
impl Rebuildable for RawCalendar {}
impl PortableWidget for RawCalendar {}
impl Widget for RawCalendar {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }

    fn debug_name(&self) -> &'static str {
        "RawCalendar"
    }
}

/// A retained date-picker field backed by [`super::DatePicker`].
///
/// Opening the field presents its calendar as an anchored overlay through the
/// application modal host. The field remains compact while the popup is open.
#[derive(Clone)]
pub struct DatePickerView {
    picker: super::DatePicker,
    width: f32,
    height: f32,
    on_selection: Option<DateSelectionCallback>,
}
impl Default for DatePickerView {
    fn default() -> Self {
        Self::new()
    }
}
impl DatePickerView {
    /// Creates an unbounded picker field with no initial date.
    #[inline]
    pub fn new() -> Self {
        Self {
            picker: super::DatePicker::new(None),
            width: 320.0,
            height: 360.0,
            on_selection: None,
        }
    }

    /// Replaces the date-picker model.
    #[inline]
    pub fn picker(mut self, picker: super::DatePicker) -> Self {
        self.picker = picker;
        self
    }

    /// Sets the logical width of the field and its popup.
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        if width.is_finite() && width >= 0.0 {
            self.width = width;
        }
        self
    }

    /// Sets the total logical height reserved for the popup, including its
    /// calendar and action footer.
    #[inline]
    pub fn height(mut self, height: f32) -> Self {
        if height.is_finite() && height >= 0.0 {
            self.height = height;
        }
        self
    }

    /// Registers a callback invoked only after the picker confirms a draft.
    #[inline]
    pub fn on_selection<F>(mut self, callback: F) -> Self
    where
        F: Fn(DateSelection) + 'static,
    {
        self.on_selection = Some(Rc::new(callback));
        self
    }
}

struct DatePickerRuntime {
    picker: RefCell<super::DatePicker>,
    focus_node: FocusNode,
    focused: Cell<bool>,
    anchor: AnchorHandle,
    overlay_active: Cell<bool>,
}

/// Retained state for [`DatePickerView`].
pub struct DatePickerViewState {
    model: DatePickerView,
    runtime: Rc<DatePickerRuntime>,
}
impl DatePickerViewState {
    /// Returns the last confirmed selection.
    #[inline]
    pub fn selection(&self) -> DateSelection {
        self.runtime.picker.borrow().selection()
    }

    /// Returns whether the anchored calendar overlay is open.
    #[inline]
    pub fn is_open(&self) -> bool {
        self.runtime.picker.borrow().is_open()
    }

    /// Returns semantic state for the retained date-picker model.
    #[inline]
    pub fn semantics(&self) -> DatePickerSemantics {
        self.runtime.picker.borrow().semantics()
    }
}
impl StatefulWidget for DatePickerView {
    type State = DatePickerViewState;

    fn create_state(self) -> Self::State {
        let runtime = Rc::new(DatePickerRuntime {
            picker: RefCell::new(self.picker.clone()),
            focus_node: FocusNode::new(),
            focused: Cell::new(false),
            anchor: AnchorHandle::new(),
            overlay_active: Cell::new(false),
        });
        DatePickerViewState {
            model: self,
            runtime,
        }
    }
}
impl State<DatePickerView> for DatePickerViewState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        if self.model.picker != new.model.picker {
            *self.runtime.picker.borrow_mut() = new.model.picker.clone();
        }
        self.model = new.model;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        DatePickerSurface {
            runtime: Rc::clone(&self.runtime),
            width: self.model.width,
            height: self.model.height,
            on_selection: self.model.on_selection.clone(),
            tokens: theme::tokens(ctx),
            popup: false,
        }
    }
}
impl Widget for DatePickerView {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "DatePickerView", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "DatePickerView"
    }
}
impl PortableWidget for DatePickerView {}

struct DatePickerSurface {
    runtime: Rc<DatePickerRuntime>,
    width: f32,
    height: f32,
    on_selection: Option<DateSelectionCallback>,
    tokens: ThemeTokens,
    popup: bool,
}
impl Widget for DatePickerSurface {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let raw = RawDatePicker {
            runtime: self.runtime,
            width: self.width,
            height: self.height,
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
        "RawDatePicker"
    }
}
impl PortableWidget for DatePickerSurface {}

struct RawDatePicker {
    runtime: Rc<DatePickerRuntime>,
    width: f32,
    height: f32,
    on_selection: Option<DateSelectionCallback>,
    tokens: ThemeTokens,
    bounds: InteractionBounds,
    popup: bool,
}
impl RawDatePicker {
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
        self.runtime.overlay_active.set(true);
        if !self.popup {
            Floating::new()
                .anchor(self.runtime.anchor.clone())
                .side(FloatingSide::Bottom)
                .align(FloatingAlign::Start)
                .gap(4.0)
                .overflow(OverflowPolicy::Flip)
                .child(DatePickerSurface {
                    runtime: Rc::clone(&self.runtime),
                    width: self.width,
                    height: popup_height(self.height),
                    on_selection: self.on_selection.clone(),
                    tokens: self.tokens,
                    popup: true,
                })
                .show();
        }
        EventResult::consumed().with_redraw()
    }

    fn commit(&self) -> EventResult {
        let outcome = self.runtime.picker.borrow_mut().confirm();
        match outcome {
            Ok(PickerOutcome::Confirmed(selection)) => {
                let has_overlay = self.runtime.overlay_active.replace(false);
                if has_overlay {
                    ModalController::dismiss_top();
                }
                if let Some(callback) = self.on_selection.as_ref() {
                    callback(selection);
                }
                EventResult::consumed().with_redraw()
            }
            _ => EventResult::consumed(),
        }
    }

    fn cancel(&self, reason: super::CancelReason) -> EventResult {
        self.finish_cancel(reason, true)
    }

    fn cancel_from_host(&self) -> EventResult {
        self.finish_cancel(super::CancelReason::OutsideClick, false)
    }

    fn finish_cancel(&self, reason: super::CancelReason, dismiss_overlay: bool) -> EventResult {
        if self.runtime.picker.borrow_mut().cancel(reason).is_ok() {
            let has_overlay = self.runtime.overlay_active.replace(false);
            if dismiss_overlay && has_overlay {
                ModalController::dismiss_top();
            }
            EventResult::consumed().with_redraw()
        } else {
            EventResult::consumed()
        }
    }

    fn select_at(&self, x: f32, y: f32) -> EventResult {
        let Some(bounds) = self.bounds.get_bounds() else {
            return EventResult::ignored();
        };
        let calendar_height = bounds.height - PICKER_FOOTER_HEIGHT;
        let origin_y = if self.popup { 0.0 } else { PICKER_FIELD_HEIGHT };
        let date = {
            let picker = self.runtime.picker.borrow();
            calendar_date_at(
                picker.calendar(),
                Vec2d {
                    x: 0.0,
                    y: origin_y,
                },
                bounds.width,
                calendar_height,
                x,
                y,
            )
        };
        let Some(date) = date else {
            return EventResult::ignored();
        };
        if self.runtime.picker.borrow_mut().select(date).is_ok() {
            EventResult::consumed().with_redraw()
        } else {
            EventResult::consumed()
        }
    }

    fn handle_key(&self, key: &NamedKey, modifiers: &Modifiers) -> EventResult {
        let is_open = self.runtime.picker.borrow().is_open();
        if !is_open {
            if matches!(key, NamedKey::Enter) {
                return self.open();
            }
            return EventResult::ignored();
        }
        if matches!(key, NamedKey::Escape) {
            return self.cancel(super::CancelReason::Escape);
        }
        if let Some(navigation) = calendar_navigation(key, modifiers) {
            let changed = self
                .runtime
                .picker
                .borrow_mut()
                .navigate(navigation)
                .unwrap_or(false);
            return if changed {
                EventResult::consumed().with_redraw()
            } else {
                EventResult::consumed()
            };
        }
        if matches!(key, NamedKey::Enter) {
            let focused = self.runtime.picker.borrow().calendar().focused_date();
            let _ = self.runtime.picker.borrow_mut().select(focused);
            return self.commit();
        }
        EventResult::ignored()
    }
}
impl VisitorElement for RawDatePicker {
    fn debug_name(&self) -> &'static str {
        "RawDatePicker"
    }
}
impl EventElement for RawDatePicker {
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
                            self.cancel(super::CancelReason::OutsideClick)
                        } else {
                            self.commit()
                        }
                    } else {
                        self.select_at(x, y)
                    }
                } else if y < PICKER_FIELD_HEIGHT {
                    if self.runtime.picker.borrow().is_open() {
                        self.cancel(super::CancelReason::OutsideClick)
                    } else {
                        self.open()
                    }
                } else if !self.runtime.picker.borrow().is_open() {
                    EventResult::consumed()
                } else if y >= bounds.height - PICKER_FOOTER_HEIGHT {
                    if x < bounds.width / 2.0 {
                        self.cancel(super::CancelReason::OutsideClick)
                    } else {
                        self.commit()
                    }
                } else {
                    self.select_at(x, y)
                }
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
impl LayoutElement for RawDatePicker {
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
impl Drawable for RawDatePicker {
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
        self.bounds.disagreement("RawDatePicker")
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
        paint::record_local_v2(ctx, |painter| {
            if self.popup {
                paint::draw_calendar(
                    painter,
                    picker.calendar(),
                    Vec2d::default(),
                    size.width,
                    (size.height - PICKER_FOOTER_HEIGHT * ctx.scale).max(0.0),
                    self.runtime.focused.get(),
                    &self.tokens,
                );
                paint::draw_footer(
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
                    "Date",
                    selection_label(if picker.is_open() {
                        picker.draft()
                    } else {
                        picker.selection()
                    }),
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
impl Rebuildable for RawDatePicker {}
impl PortableWidget for RawDatePicker {}
impl Widget for RawDatePicker {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }

    fn debug_name(&self) -> &'static str {
        "RawDatePicker"
    }
}

/// A retained date-time picker backed by [`super::DateTimePicker`].
///
/// The trigger stays at field height while the calendar or time editor is
/// presented through the application modal host.
#[derive(Clone)]
pub struct DateTimePickerView {
    picker: DateTimePicker,
    width: f32,
    height: f32,
    use_24_hours: bool,
    on_selection: Option<DateTimeSelectionCallback>,
}
impl Default for DateTimePickerView {
    fn default() -> Self {
        Self::new()
    }
}
impl DateTimePickerView {
    /// Creates an unbounded UTC date-time picker field.
    #[inline]
    pub fn new() -> Self {
        Self {
            picker: default_datetime_picker(),
            width: 320.0,
            height: 360.0,
            use_24_hours: false,
            on_selection: None,
        }
    }

    /// Replaces the date-time picker model and its explicit timezone policy.
    #[inline]
    pub fn picker(mut self, picker: DateTimePicker) -> Self {
        self.picker = picker;
        self
    }

    /// Sets the logical width of the picker field and its popup.
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        if width.is_finite() && width >= 0.0 {
            self.width = width;
        }
        self
    }

    /// Sets the total logical height reserved for the popup, including its
    /// editor and action footer.
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

    /// Registers a callback invoked after a date-time draft is confirmed.
    #[inline]
    pub fn on_selection<F>(mut self, callback: F) -> Self
    where
        F: Fn(Option<DateTime>) + 'static,
    {
        self.on_selection = Some(Rc::new(callback));
        self
    }
}

struct DateTimePickerRuntime {
    picker: RefCell<DateTimePicker>,
    focus_node: FocusNode,
    focused: Cell<bool>,
    field: Cell<DateTimeField>,
    drag: RefCell<Option<TimeWheelDrag>>,
    scroll_remainder: Cell<f32>,
    anchor: AnchorHandle,
    overlay_active: Cell<bool>,
}

/// Retained state for [`DateTimePickerView`].
pub struct DateTimePickerViewState {
    model: DateTimePickerView,
    runtime: Rc<DateTimePickerRuntime>,
}
impl DateTimePickerViewState {
    /// Returns the last confirmed date-time.
    #[inline]
    pub fn value(&self) -> Option<DateTime> {
        self.runtime.picker.borrow().value()
    }

    /// Returns whether the date-time editor is open.
    #[inline]
    pub fn is_open(&self) -> bool {
        self.runtime.picker.borrow().is_open()
    }

    /// Returns semantic state for the retained date-time picker model.
    #[inline]
    pub fn semantics(&self) -> DateTimePickerSemantics {
        self.runtime.picker.borrow().semantics()
    }
}
impl StatefulWidget for DateTimePickerView {
    type State = DateTimePickerViewState;

    fn create_state(self) -> Self::State {
        let runtime = Rc::new(DateTimePickerRuntime {
            picker: RefCell::new(self.picker.clone()),
            focus_node: FocusNode::new(),
            focused: Cell::new(false),
            field: Cell::new(DateTimeField::Date),
            drag: RefCell::new(None),
            scroll_remainder: Cell::new(0.0),
            anchor: AnchorHandle::new(),
            overlay_active: Cell::new(false),
        });
        DateTimePickerViewState {
            model: self,
            runtime,
        }
    }
}
impl State<DateTimePickerView> for DateTimePickerViewState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        if self.model.picker != new.model.picker {
            *self.runtime.picker.borrow_mut() = new.model.picker.clone();
        }
        self.model = new.model;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        DateTimePickerSurface {
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
impl Widget for DateTimePickerView {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "DateTimePickerView", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "DateTimePickerView"
    }
}

impl PortableWidget for DateTimePickerView {}

struct DateTimePickerSurface {
    runtime: Rc<DateTimePickerRuntime>,
    width: f32,
    height: f32,
    use_24_hours: bool,
    on_selection: Option<DateTimeSelectionCallback>,
    tokens: ThemeTokens,
    popup: bool,
}

impl Widget for DateTimePickerSurface {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let raw = RawDateTimePicker {
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
        "RawDateTimePicker"
    }
}

impl PortableWidget for DateTimePickerSurface {}

struct RawDateTimePicker {
    runtime: Rc<DateTimePickerRuntime>,
    width: f32,
    height: f32,
    use_24_hours: bool,
    on_selection: Option<DateTimeSelectionCallback>,
    tokens: ThemeTokens,
    bounds: InteractionBounds,
    popup: bool,
}

impl RawDateTimePicker {
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

    fn calendar(&self) -> Calendar {
        let picker = self.runtime.picker.borrow();
        let current = picker.draft().expect("date-time picker always has a value");
        let bounds = picker.policy().bounds();
        let date_bounds = DateBounds::new(
            bounds.min().map(|value| value.date()),
            bounds.max().map(|value| value.date()),
        )
        .unwrap_or_else(|_| DateBounds::unbounded());
        Calendar::try_new(
            current.date(),
            date_bounds,
            super::DateSelectionMode::Single,
        )
        .map(|calendar| {
            let mut calendar =
                calendar.with_disabled_dates(picker.disabled_dates().iter().copied());
            let _ = calendar.select(current.date());
            calendar
        })
        .unwrap_or_else(|_| Calendar::new(current.date()))
    }

    fn open(&self) -> EventResult {
        self.runtime.picker.borrow_mut().open();
        self.runtime.field.set(DateTimeField::Date);
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
                .child(DateTimePickerSurface {
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

    fn cancel(&self, reason: super::CancelReason) -> EventResult {
        self.finish_cancel(reason, true)
    }

    fn cancel_from_host(&self) -> EventResult {
        self.finish_cancel(super::CancelReason::OutsideClick, false)
    }

    fn finish_cancel(&self, reason: super::CancelReason, dismiss_overlay: bool) -> EventResult {
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

    fn move_field(&self, backwards: bool) -> EventResult {
        let fields = [
            DateTimeField::Date,
            DateTimeField::Hour,
            DateTimeField::Minute,
            DateTimeField::Second,
            DateTimeField::Period,
        ];
        let max_index = if self.use_24_hours { 3 } else { 4 };
        let index = fields
            .iter()
            .position(|field| *field == self.runtime.field.get())
            .unwrap_or(0)
            .min(max_index);
        let next = if backwards {
            if index == 0 { max_index } else { index - 1 }
        } else if index >= max_index {
            0
        } else {
            index + 1
        };
        self.runtime.field.set(fields[next]);
        EventResult::consumed().with_redraw()
    }

    fn adjust_time(&self, increase: bool, to_boundary: Option<bool>) -> EventResult {
        let field = self.runtime.field.get();
        if field == DateTimeField::Period {
            if self.use_24_hours {
                return EventResult::ignored();
            }
            let steps = if increase { 1 } else { -1 };
            let next = {
                let picker = self.runtime.picker.borrow();
                picker
                    .draft()
                    .and_then(|current| time_from_scroll_steps(current.time(), 3, steps, false))
            };
            let Some(next) = next else {
                return EventResult::consumed();
            };
            return if self.runtime.picker.borrow_mut().set_time(next).is_ok() {
                EventResult::consumed().with_redraw()
            } else {
                EventResult::consumed()
            };
        }
        let result = {
            let mut picker = self.runtime.picker.borrow_mut();
            let Some(current) = picker.draft() else {
                return EventResult::ignored();
            };
            let time = current.time();
            let (value, maximum) = match field {
                DateTimeField::Date => return EventResult::ignored(),
                DateTimeField::Hour => (i16::from(time.hour()), 23),
                DateTimeField::Minute => (i16::from(time.minute()), 59),
                DateTimeField::Second => (i16::from(time.second()), 59),
                DateTimeField::Period => return EventResult::ignored(),
            };
            let next = match to_boundary {
                Some(true) => maximum,
                Some(false) => 0,
                None if increase => value.saturating_add(1).min(maximum),
                None => value.saturating_sub(1).max(0),
            } as u8;
            let next_time = match field {
                DateTimeField::Date => Ok(time),
                DateTimeField::Hour => {
                    TimeOfDay::try_new(next, time.minute(), time.second(), time.nanosecond())
                }
                DateTimeField::Minute => {
                    TimeOfDay::try_new(time.hour(), next, time.second(), time.nanosecond())
                }
                DateTimeField::Second => {
                    TimeOfDay::try_new(time.hour(), time.minute(), next, time.nanosecond())
                }
                DateTimeField::Period => return EventResult::ignored(),
            };
            match next_time {
                Ok(next_time) => picker.set_time(next_time),
                Err(_) => return EventResult::ignored(),
            }
        };
        if result.is_ok() {
            EventResult::consumed().with_redraw()
        } else {
            EventResult::consumed()
        }
    }

    fn select_section(&self, date: bool) -> EventResult {
        self.runtime.field.set(if date {
            DateTimeField::Date
        } else {
            match self.runtime.field.get() {
                DateTimeField::Minute => DateTimeField::Minute,
                DateTimeField::Second => DateTimeField::Second,
                DateTimeField::Period => DateTimeField::Period,
                DateTimeField::Date | DateTimeField::Hour => DateTimeField::Hour,
            }
        });
        EventResult::consumed().with_redraw()
    }

    fn begin_time_drag(&self, pointer: &PointerInfo, x: f32, y: f32) -> EventResult {
        let Some(bounds) = self.bounds.get_bounds() else {
            return EventResult::ignored();
        };
        let Some(row) = row_at(y, bounds.height, PICKER_FIELD_HEIGHT) else {
            return EventResult::ignored();
        };
        let Some(column) = column_at(x, bounds.width, self.use_24_hours) else {
            return EventResult::ignored();
        };
        let selected = {
            let mut picker = self.runtime.picker.borrow_mut();
            let Some(current) = picker.draft() else {
                return EventResult::ignored();
            };
            let Some(next) = time_from_row(current.time(), column, row, self.use_24_hours) else {
                return EventResult::ignored();
            };
            if picker.set_time(next).is_err() {
                return EventResult::consumed();
            }
            next
        };
        self.runtime.field.set(match column {
            0 => DateTimeField::Hour,
            1 => DateTimeField::Minute,
            2 => DateTimeField::Second,
            _ => DateTimeField::Period,
        });
        let key = PointerKey::new(pointer.source, pointer.id);
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

    fn update_time_drag(&self, pointer: &PointerInfo, y: f32) -> EventResult {
        let key = PointerKey::new(pointer.source, pointer.id);
        let Some(drag) = self.runtime.drag.borrow().as_ref().copied() else {
            return EventResult::ignored();
        };
        if drag.pointer != key {
            return EventResult::ignored();
        }
        let Some(next_time) = time_from_drag(
            drag.start_time,
            drag.column,
            drag.start_y,
            y,
            self.use_24_hours,
        ) else {
            return EventResult::consumed();
        };
        if self.runtime.picker.borrow_mut().set_time(next_time).is_ok() {
            EventResult::consumed().with_redraw()
        } else {
            EventResult::consumed()
        }
    }

    fn update_time_hover(&self, pointer: &PointerInfo) -> EventResult {
        if self.runtime.field.get().is_date() {
            return EventResult::ignored();
        }
        let Some(bounds) = self.bounds.get_bounds() else {
            return EventResult::ignored();
        };
        let x = pointer.pos.x - bounds.x;
        let y = pointer.pos.y - bounds.y;
        if row_at(y, bounds.height, PICKER_FIELD_HEIGHT).is_none() {
            return EventResult::ignored();
        }
        let Some(column) = column_at(x, bounds.width, self.use_24_hours) else {
            return EventResult::ignored();
        };
        let field = match column {
            0 => DateTimeField::Hour,
            1 => DateTimeField::Minute,
            2 => DateTimeField::Second,
            _ => DateTimeField::Period,
        };
        if self.runtime.field.replace(field) != field {
            EventResult::redraw()
        } else {
            EventResult::ignored()
        }
    }

    fn end_time_drag(&self, pointer: &PointerInfo, y: f32) -> EventResult {
        let key = PointerKey::new(pointer.source, pointer.id);
        let Some(drag) = self.runtime.drag.borrow().as_ref().copied() else {
            return EventResult::ignored();
        };
        if drag.pointer != key {
            return EventResult::ignored();
        }
        let result = self.update_time_drag(pointer, y);
        self.runtime.drag.replace(None);
        result.with_pointer_release(key)
    }

    fn handle_time_scroll(&self, delta_y: f32, kind: ScrollDeltaKind) -> EventResult {
        if !self.runtime.picker.borrow().is_open() {
            return EventResult::ignored();
        }
        if self.runtime.field.get().is_date() {
            self.runtime.field.set(DateTimeField::Hour);
        }
        let steps = scroll_steps(delta_y, kind, &self.runtime.scroll_remainder);
        if steps == 0 {
            return EventResult::consumed();
        }
        let column = self.runtime.field.get().time_column();
        let next = {
            let picker = self.runtime.picker.borrow();
            let Some(current) = picker.draft() else {
                return EventResult::consumed();
            };
            time_from_scroll_steps(current.time(), column, steps, self.use_24_hours)
        };
        let Some(next) = next else {
            return EventResult::consumed();
        };
        if self.runtime.picker.borrow_mut().set_time(next).is_ok() {
            EventResult::consumed().with_redraw()
        } else {
            EventResult::consumed()
        }
    }

    fn handle_key(&self, key: &NamedKey, modifiers: &Modifiers) -> EventResult {
        if !self.runtime.picker.borrow().is_open() {
            if matches!(key, NamedKey::Enter) {
                self.runtime.field.set(DateTimeField::Date);
                return self.open();
            }
            return EventResult::ignored();
        }
        if matches!(key, NamedKey::Escape) {
            return self.cancel(super::CancelReason::Escape);
        }
        if matches!(key, NamedKey::Tab) {
            return self.move_field(modifiers.shift);
        }
        if modifiers.ctrl && matches!(key, NamedKey::ArrowLeft | NamedKey::ArrowRight) {
            return self.move_field(matches!(key, NamedKey::ArrowLeft));
        }
        if self.runtime.field.get() == DateTimeField::Date {
            if let Some(navigation) = calendar_navigation(key, modifiers) {
                return if self
                    .runtime
                    .picker
                    .borrow_mut()
                    .navigate(navigation)
                    .unwrap_or(false)
                {
                    EventResult::consumed().with_redraw()
                } else {
                    EventResult::consumed()
                };
            }
        } else {
            match key {
                NamedKey::ArrowRight | NamedKey::ArrowUp => {
                    return self.adjust_time(true, None);
                }
                NamedKey::ArrowLeft | NamedKey::ArrowDown => {
                    return self.adjust_time(false, None);
                }
                NamedKey::Home => return self.adjust_time(false, Some(false)),
                NamedKey::End => return self.adjust_time(true, Some(true)),
                _ => {}
            }
        }
        if matches!(key, NamedKey::Enter) {
            return self.commit();
        }
        EventResult::ignored()
    }
}

impl VisitorElement for RawDateTimePicker {
    fn debug_name(&self) -> &'static str {
        "RawDateTimePicker"
    }
}

impl EventElement for RawDateTimePicker {
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
                    if y < PICKER_FIELD_HEIGHT {
                        self.select_section(x < bounds.width / 2.0)
                    } else if !self.runtime.picker.borrow().is_open() {
                        EventResult::consumed()
                    } else if y >= bounds.height - PICKER_FOOTER_HEIGHT {
                        if x < bounds.width / 2.0 {
                            self.cancel(super::CancelReason::OutsideClick)
                        } else {
                            self.commit()
                        }
                    } else if self.runtime.field.get() == DateTimeField::Date {
                        let calendar = self.calendar();
                        let date = calendar_date_at(
                            &calendar,
                            Vec2d {
                                x: 0.0,
                                y: PICKER_FIELD_HEIGHT,
                            },
                            bounds.width,
                            bounds.height - PICKER_FIELD_HEIGHT - PICKER_FOOTER_HEIGHT,
                            x,
                            y,
                        );
                        if let Some(date) = date {
                            if self.runtime.picker.borrow_mut().set_date(date).is_ok() {
                                EventResult::consumed().with_redraw()
                            } else {
                                EventResult::consumed()
                            }
                        } else {
                            EventResult::ignored()
                        }
                    } else {
                        self.begin_time_drag(pointer, x, y)
                    }
                } else {
                    if y < PICKER_FIELD_HEIGHT {
                        if self.runtime.picker.borrow().is_open() {
                            self.cancel(super::CancelReason::OutsideClick)
                        } else {
                            self.open()
                        }
                    } else {
                        EventResult::consumed()
                    }
                }
            }
            ElementEvent::PointerMove(pointer) => {
                let bounds = self.bounds.get_bounds().unwrap_or_default();
                let result = self.update_time_drag(pointer, pointer.pos.y - bounds.y);
                if result.is_consumed() {
                    result
                } else {
                    self.update_time_hover(pointer)
                }
            }
            ElementEvent::PointerUp(pointer) => {
                let bounds = self.bounds.get_bounds().unwrap_or_default();
                self.end_time_drag(pointer, pointer.pos.y - bounds.y)
            }
            ElementEvent::Scroll { delta, kind, .. } if self.popup => {
                self.handle_time_scroll(delta.y, *kind)
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

impl LayoutElement for RawDateTimePicker {
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

impl Drawable for RawDateTimePicker {
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
        self.bounds.disagreement("RawDateTimePicker")
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
        let value = picker.draft().expect("date-time picker always has a value");
        let field = self.runtime.field.get();
        let (label, display) = if field.is_date() {
            ("Date", format_datetime(value))
        } else {
            ("Time", format_time(value.time(), self.use_24_hours))
        };
        paint::record_local_v2(ctx, |painter| {
            if self.popup {
                paint::draw_segmented_picker_header(
                    painter,
                    size.width,
                    field.is_date(),
                    &self.tokens,
                );
                if field.is_date() {
                    let calendar = self.calendar();
                    paint::draw_calendar(
                        painter,
                        &calendar,
                        Vec2d {
                            x: 0.0,
                            y: PICKER_FIELD_HEIGHT * ctx.scale,
                        },
                        size.width,
                        (size.height - (PICKER_FIELD_HEIGHT + PICKER_FOOTER_HEIGHT) * ctx.scale)
                            .max(0.0),
                        self.runtime.focused.get(),
                        &self.tokens,
                    );
                } else {
                    paint::draw_time_picker(
                        painter,
                        value.time(),
                        size.width,
                        size.height,
                        PICKER_FIELD_HEIGHT,
                        field.time_column(),
                        self.use_24_hours,
                        &self.tokens,
                    );
                }
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
                    label,
                    display,
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

impl Rebuildable for RawDateTimePicker {}
impl PortableWidget for RawDateTimePicker {}

impl Widget for RawDateTimePicker {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }

    fn debug_name(&self) -> &'static str {
        "RawDateTimePicker"
    }
}

fn color_slider_offset(index: usize, scale: f32) -> Vec2d {
    Vec2d {
        x: 24.0 * scale,
        y: (COLOR_CHANNEL_FIRST_Y - COLOR_CHANNEL_ROW_HEIGHT / 2.0
            + index as f32 * COLOR_CHANNEL_ROW_HEIGHT)
            * scale,
    }
}

fn layout_child(child: &AnyElement, ctx: &BuildContext, offset: Vec2d) {
    ctx.canvas.save();
    ctx.canvas.translate(offset);
    child.layout(ctx);
    ctx.canvas.restore();
}

fn draw_child(child: &AnyElement, ctx: &BuildContext, offset: Vec2d) {
    ctx.canvas.save();
    ctx.canvas.translate(offset);
    child.update(ctx);
    ctx.canvas.restore();
}
