use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_events::pointer::PointerButton;
use aimer_modal::{
    Anchor, AnchorHandle, Floating, FloatingAlign, FloatingSide, ModalController, OverflowPolicy,
};
use aimer_style::ThemeTokens;
use aimer_widget::InteractionBounds;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, FocusNode, LayoutElement,
    PortableWidget, Rebuildable, State, StateUpdater, StatefulElement, StatefulWidget,
    VisitorElement, Widget,
};

use super::widget_helpers::{color_sliders, format_rgba};
use super::{
    ColorChannel, ColorError, ColorKey, ColorPicker, ColorPickerSemantics, ColorSelectionCallback,
    Hsva, PickerOutcome,
};
use super::{paint, theme};
use crate::{CancelReason, PICKER_FIELD_HEIGHT, PICKER_FOOTER_HEIGHT};

/// A retained keyboard-accessible color picker with swatch support.
///
/// The trigger stays at field height while the swatches and channel sliders
/// are presented through the application modal host.
#[derive(Clone)]
pub struct ColorPickerView {
    picker: ColorPicker,
    width: f32,
    height: f32,
    on_selection: Option<ColorSelectionCallback>,
}

impl Default for ColorPickerView {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorPickerView {
    /// Creates a closed opaque-black color picker with alpha editing enabled.
    #[inline]
    pub fn new() -> Self {
        Self {
            picker: ColorPicker::new(
                Hsva::try_new(0, 0, 0, 100).expect("default color is valid"),
                true,
            ),
            width: 280.0,
            height: 260.0,
            on_selection: None,
        }
    }

    /// Replaces the color-picker model.
    #[inline]
    pub fn picker(mut self, picker: ColorPicker) -> Self {
        self.picker = picker;
        self
    }

    /// Sets the logical width of the color picker.
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        if width.is_finite() && width >= 0.0 {
            self.width = width;
        }
        self
    }

    /// Sets the logical height of the color picker.
    #[inline]
    pub fn height(mut self, height: f32) -> Self {
        if height.is_finite() && height >= 0.0 {
            self.height = height;
        }
        self
    }

    /// Registers a callback invoked after a color draft is confirmed.
    #[inline]
    pub fn on_selection<F>(mut self, callback: F) -> Self
    where
        F: Fn(Hsva) + 'static,
    {
        self.on_selection = Some(Rc::new(callback));
        self
    }
}

pub(super) struct ColorPickerRuntime {
    pub(super) picker: RefCell<ColorPicker>,
    pub(super) focus_node: FocusNode,
    pub(super) focused: Cell<bool>,
    pub(super) channel: Cell<ColorChannel>,
    pub(super) updater: RefCell<Option<StateUpdater<ColorPickerViewState>>>,
    pub(super) anchor: AnchorHandle,
    pub(super) overlay_active: Cell<bool>,
}

/// Retained state for [`ColorPickerView`].
pub struct ColorPickerViewState {
    model: ColorPickerView,
    runtime: Rc<ColorPickerRuntime>,
}

impl ColorPickerViewState {
    /// Returns the last confirmed color.
    #[inline]
    pub fn value(&self) -> Hsva {
        self.runtime.picker.borrow().value()
    }

    /// Returns whether the color editor is open.
    #[inline]
    pub fn is_open(&self) -> bool {
        self.runtime.picker.borrow().is_open()
    }

    /// Returns the keyboard-active channel.
    #[inline]
    pub fn active_channel(&self) -> ColorChannel {
        self.runtime.channel.get()
    }

    /// Returns semantic state for the retained color-picker model.
    #[inline]
    pub fn semantics(&self) -> ColorPickerSemantics {
        self.runtime.picker.borrow().semantics()
    }
}

impl StatefulWidget for ColorPickerView {
    type State = ColorPickerViewState;

    fn create_state(self) -> Self::State {
        let runtime = Rc::new(ColorPickerRuntime {
            picker: RefCell::new(self.picker.clone()),
            focus_node: FocusNode::new(),
            focused: Cell::new(false),
            channel: Cell::new(ColorChannel::Hue),
            updater: RefCell::new(None),
            anchor: AnchorHandle::new(),
            overlay_active: Cell::new(false),
        });
        ColorPickerViewState {
            model: self,
            runtime,
        }
    }
}

impl State<ColorPickerView> for ColorPickerViewState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.runtime.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, new: Self) {
        if self.model.picker != new.model.picker {
            *self.runtime.picker.borrow_mut() = new.model.picker.clone();
        }
        self.model = new.model;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        ColorPickerSurface {
            runtime: Rc::clone(&self.runtime),
            width: self.model.width,
            height: self.model.height,
            on_selection: self.model.on_selection.clone(),
            tokens: theme::tokens(ctx),
            popup: false,
        }
    }
}

impl Widget for ColorPickerView {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ColorPickerView", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "ColorPickerView"
    }
}

impl PortableWidget for ColorPickerView {}

struct ColorPickerSurface {
    runtime: Rc<ColorPickerRuntime>,
    width: f32,
    height: f32,
    on_selection: Option<ColorSelectionCallback>,
    tokens: ThemeTokens,
    popup: bool,
}

impl Widget for ColorPickerSurface {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let sliders = color_sliders(&self.runtime, self.width, ctx);
        let raw = RawColorPicker {
            runtime: self.runtime,
            width: self.width,
            height: self.height,
            on_selection: self.on_selection,
            tokens: self.tokens,
            sliders,
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
        "RawColorPicker"
    }
}

impl PortableWidget for ColorPickerSurface {}

struct RawColorPicker {
    runtime: Rc<ColorPickerRuntime>,
    width: f32,
    height: f32,
    on_selection: Option<ColorSelectionCallback>,
    tokens: ThemeTokens,
    sliders: Vec<AnyElement>,
    bounds: InteractionBounds,
    popup: bool,
}

impl RawColorPicker {
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

    fn request_rebuild(&self) {
        if let Some(updater) = self.runtime.updater.borrow().as_ref() {
            updater.set_state(|_| {});
        }
    }

    fn open(&self) -> EventResult {
        self.runtime.picker.borrow_mut().open();
        self.runtime.overlay_active.set(true);
        self.request_rebuild();
        if !self.popup {
            Floating::new()
                .anchor(self.runtime.anchor.clone())
                .side(FloatingSide::Bottom)
                .align(FloatingAlign::Start)
                .gap(4.0)
                .overflow(OverflowPolicy::Flip)
                .child(ColorPickerSurface {
                    runtime: Rc::clone(&self.runtime),
                    width: self.width,
                    height: self.height,
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
                if has_overlay {
                    ModalController::dismiss_top();
                }
                if let Some(callback) = self.on_selection.as_ref() {
                    callback(value);
                }
                self.request_rebuild();
                EventResult::consumed().with_redraw()
            }
            _ => EventResult::consumed(),
        }
    }

    fn cancel(&self, reason: CancelReason) -> EventResult {
        self.finish_cancel(reason, true)
    }

    fn cancel_from_host(&self) -> EventResult {
        self.finish_cancel(CancelReason::OutsideClick, false)
    }

    fn finish_cancel(&self, reason: CancelReason, dismiss_overlay: bool) -> EventResult {
        if self.runtime.picker.borrow_mut().cancel(reason).is_ok() {
            let has_overlay = self.runtime.overlay_active.replace(false);
            if dismiss_overlay && has_overlay {
                ModalController::dismiss_top();
            }
            self.request_rebuild();
            EventResult::consumed().with_redraw()
        } else {
            EventResult::consumed()
        }
    }

    fn handle_key(&self, key: &NamedKey, modifiers: &Modifiers) -> EventResult {
        if !self.runtime.picker.borrow().is_open() {
            if matches!(key, NamedKey::Enter) {
                return self.open();
            }
            return EventResult::ignored();
        }
        if matches!(key, NamedKey::Escape) {
            return self.cancel(CancelReason::Escape);
        }
        if matches!(key, NamedKey::Tab) {
            let channels = [
                ColorChannel::Hue,
                ColorChannel::Saturation,
                ColorChannel::Value,
                ColorChannel::Alpha,
            ];
            let index = channels
                .iter()
                .position(|channel| *channel == self.runtime.channel.get())
                .unwrap_or(0);
            let next = if modifiers.shift {
                index.checked_sub(1).unwrap_or(channels.len() - 1)
            } else {
                (index + 1) % channels.len()
            };
            self.runtime.channel.set(channels[next]);
            return EventResult::consumed().with_redraw();
        }
        if matches!(key, NamedKey::Enter) {
            return self.commit();
        }
        let color_key = match key {
            NamedKey::ArrowLeft | NamedKey::ArrowDown => Some(ColorKey::Decrease),
            NamedKey::ArrowRight | NamedKey::ArrowUp => Some(ColorKey::Increase),
            NamedKey::Home => Some(ColorKey::Home),
            NamedKey::End => Some(ColorKey::End),
            _ => None,
        };
        if let Some(color_key) = color_key {
            let result = self
                .runtime
                .picker
                .borrow_mut()
                .handle_key(self.runtime.channel.get(), color_key);
            return match result {
                Ok(()) => {
                    self.request_rebuild();
                    EventResult::consumed().with_redraw()
                }
                Err(ColorError::AlphaDisabled) => EventResult::consumed(),
                Err(_) => EventResult::ignored(),
            };
        }
        EventResult::ignored()
    }
}

impl VisitorElement for RawColorPicker {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn aimer_widget::Element)) {
        if self.popup && self.runtime.picker.borrow().is_open() {
            for slider in &self.sliders {
                visitor(slider.as_ref());
            }
        }
    }

    fn debug_name(&self) -> &'static str {
        "RawColorPicker"
    }
}

impl EventElement for RawColorPicker {
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
                        if self.runtime.picker.borrow().is_open() {
                            self.cancel(CancelReason::OutsideClick)
                        } else {
                            self.open()
                        }
                    } else if !self.runtime.picker.borrow().is_open() {
                        EventResult::consumed()
                    } else if y >= bounds.height - PICKER_FOOTER_HEIGHT {
                        if x < bounds.width / 2.0 {
                            self.cancel(CancelReason::OutsideClick)
                        } else {
                            self.commit()
                        }
                    } else if (82.0..106.0).contains(&y) {
                        let swatch_index = (x / 32.0).floor() as usize;
                        let swatches = self.runtime.picker.borrow().swatches().to_vec();
                        if let Some(swatch) = swatches.get(swatch_index) {
                            let selected = self
                                .runtime
                                .picker
                                .borrow_mut()
                                .select_swatch(swatch.id())
                                .is_ok();
                            if selected {
                                self.request_rebuild();
                            }
                            EventResult::consumed().with_redraw()
                        } else {
                            EventResult::ignored()
                        }
                    } else {
                        EventResult::ignored()
                    }
                } else if y < PICKER_FIELD_HEIGHT {
                    if self.runtime.picker.borrow().is_open() {
                        self.cancel(CancelReason::OutsideClick)
                    } else {
                        self.open()
                    }
                } else {
                    EventResult::consumed()
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

impl LayoutElement for RawColorPicker {
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
        if self.popup && self.runtime.picker.borrow().is_open() {
            for (index, slider) in self.sliders.iter().enumerate() {
                super::layout_child(slider, ctx, super::color_slider_offset(index, ctx.scale));
            }
        }
        size
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl Drawable for RawColorPicker {
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
        self.bounds.disagreement("RawColorPicker")
    }

    fn update(&self, ctx: &BuildContext) {
        // The surface is painted by `paint_local_v2`; this only publishes where
        // it sits and visits the retained children.
        let size = self.computed_size(ctx);
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
        if self.popup && self.runtime.picker.borrow().is_open() {
            for (index, slider) in self.sliders.iter().enumerate() {
                super::draw_child(slider, ctx, super::color_slider_offset(index, ctx.scale));
            }
        }
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
                paint::draw_picker_field(
                    painter,
                    "Color",
                    format_rgba(value.to_rgba()),
                    size.width,
                    picker.is_open(),
                    self.runtime.focused.get(),
                    &self.tokens,
                );
                paint::draw_color_picker(
                    painter,
                    &picker,
                    size.width,
                    size.height,
                    self.runtime.channel.get(),
                    &self.tokens,
                );
                paint::draw_overlay_border(painter, size.width, size.height, &self.tokens);
            } else {
                paint::draw_picker_field(
                    painter,
                    "Color",
                    format_rgba(value.to_rgba()),
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

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        _child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        if !(self.popup && self.runtime.picker.borrow().is_open()) {
            return None;
        }
        let offset = super::color_slider_offset(child_index, ctx.scale);
        let mut child_ctx = ctx.clone();
        child_ctx.parent_pos.x += offset.x;
        child_ctx.parent_pos.y += offset.y;
        Some(child_ctx)
    }

    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        let child_ctx = self.retained_v2_child_context_at(ctx, child, child_index)?;
        let offset = super::color_slider_offset(child_index, ctx.scale);
        let position = child.pos().unwrap_or_default();
        let size = child
            .retained_v2_bounds(&child_ctx)
            .unwrap_or_else(|| child.content_size(&child_ctx));
        Some((
            aimer_cupid::draw_cmd_v2::Rect::new(
                (offset.x + position.x) / ctx.scale,
                (offset.y + position.y) / ctx.scale,
                size.width / ctx.scale,
                size.height / ctx.scale,
            ),
            child.retained_clip(&child_ctx),
        ))
    }
}

impl Rebuildable for RawColorPicker {}
impl PortableWidget for RawColorPicker {}

impl Widget for RawColorPicker {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }

    fn debug_name(&self) -> &'static str {
        "RawColorPicker"
    }
}
