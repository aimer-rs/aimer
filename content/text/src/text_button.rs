use std::cell::{Cell, RefCell};
use std::time::Duration;

use aimer_events::element::ElementEvent;
use aimer_events::pointer::PointerSource;
use aimer_events::window::request_animation_frame;
use aimer_macro::PortableWidget;
use aimer_style::{TextOverflow, TextStyle};
use aimer_utils::AnimInstant;
use aimer_utils::callback::{CallbackExecutor, VoidCallback};
use aimer_widget::InteractionBounds;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, EventTreeRole, LayoutCache,
    LayoutElement, Rebuildable, VisitorElement, Widget,
};

use crate::RawTextWidget;
use crate::text_source::TextSource;

/// A label-sized text control that responds to primary presses.
///
/// The control lays out exactly like its text and has no container or padding.
/// Its normal, hover, and disabled styles each default to
/// [`TextStyle::default`]; explicit color builders override the color of the
/// corresponding style. Wrapped labels shrink to their intrinsic width when
/// space is available and use the available width only when wrapping is
/// necessary. For wrapped labels, each line is hit-tested only across its
/// rendered width, so blank space after a short line is not interactive. A
/// press fires on pointer-up only when pointer-down and pointer-up both occur
/// inside the label. Disabled controls neither hover nor invoke callbacks.
///
/// # Example
///
/// ```
/// use aimer_text::TextButton;
/// use aimer_widget::base::Color;
///
/// let button = TextButton::new("Learn more").color(Color::BLUE)
///                                           .on_press(|| println!("open"));
/// ```
#[derive(Clone, PortableWidget)]
#[portable_widget(id = "aimer_text::TextButton", schema_only)]
pub struct TextButton {
    disabled: bool,
    label: TextSource,
    color: Option<Color>,
    hover_color: Option<Color>,
    disabled_color: Option<Color>,
    #[portable_optional]
    style: TextStyle,
    #[portable_optional]
    hover_style: TextStyle,
    #[portable_optional]
    disabled_style: TextStyle,
    #[portable_callback(async)]
    on_press: VoidCallback,
    #[portable_callback(async)]
    on_double_press: VoidCallback,
}

impl TextButton {
    /// Conventional text color available to callers.
    pub const TEXT_COLOR: Color = Color::BLUE;
    /// Conventional hover color available to callers.
    pub const HOVER_COLOR: Color = Color::BLUE.lighten(0.6);
    /// Conventional disabled color available to callers.
    pub const DISABLED_COLOR: Color = Color::GRAY;

    /// Creates an enabled text button with `label`, default styles, and no-op
    /// callbacks.
    ///
    /// The color constants are not applied automatically; configure them with
    /// the color builders.
    #[inline]
    pub fn new(label: impl Into<TextSource>) -> Self {
        Self {
            disabled: false,
            label: label.into(),
            color: None,
            hover_color: None,
            disabled_color: None,
            style: TextStyle::default(),
            hover_style: TextStyle::default(),
            disabled_style: TextStyle::default(),
            on_press: VoidCallback::default(),
            on_double_press: VoidCallback::default(),
        }
    }

    /// Sets whether pointer interaction and hover styling are disabled.
    #[inline]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Overrides the normal style's text color.
    #[inline]
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Overrides the hover style's text color.
    #[inline]
    pub fn hover_color(mut self, color: impl Into<Color>) -> Self {
        self.hover_color = Some(color.into());
        self
    }

    /// Overrides the disabled style's text color.
    #[inline]
    pub fn disabled_color(mut self, color: impl Into<Color>) -> Self {
        self.disabled_color = Some(color.into());
        self
    }

    /// Replaces the style used while enabled and not hovered.
    #[inline]
    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Replaces the style used while the mouse is over an enabled button.
    #[inline]
    pub fn hover_style(mut self, style: TextStyle) -> Self {
        self.hover_style = style;
        self
    }

    /// Replaces the style used while disabled.
    #[inline]
    pub fn disabled_style(mut self, style: TextStyle) -> Self {
        self.disabled_style = style;
        self
    }

    /// Sets the callback invoked for every completed primary press.
    ///
    /// Both the first and second presses of a double press invoke this
    /// callback.
    #[inline]
    pub fn on_press(mut self, callback: impl Into<VoidCallback>) -> Self {
        self.on_press = callback.into();
        self
    }

    /// Sets the callback additionally invoked when two presses finish within
    /// 500 milliseconds.
    #[inline]
    pub fn on_double_press(mut self, callback: impl Into<VoidCallback>) -> Self {
        self.on_double_press = callback.into();
        self
    }
}

impl Widget for TextButton {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawTextButton {
            widget: self.clone(),
            hovered: Cell::new(false),
            interaction: RefCell::new(ButtonInteraction::default()),
            last_tap: Cell::new(None),
            bounds: TextHitBounds::default(),
        }
        .boxed()
    }
}

#[derive(Debug, Default)]
struct ButtonInteraction {
    armed: bool,
}

#[derive(Debug, Eq, PartialEq)]
enum ButtonAction {
    None,
    Press,
}

impl ButtonInteraction {
    fn pointer_down(&mut self, inside: bool, disabled: bool) -> ButtonAction {
        self.armed = inside && !disabled;
        ButtonAction::None
    }

    fn pointer_up(&mut self, inside: bool, disabled: bool) -> ButtonAction {
        let pressed = self.armed && inside && !disabled;
        self.armed = false;
        if pressed {
            ButtonAction::Press
        } else {
            ButtonAction::None
        }
    }

    fn cancel(&mut self) {
        self.armed = false;
    }
}

struct RawTextButton {
    widget: TextButton,
    hovered: Cell<bool>,
    interaction: RefCell<ButtonInteraction>,
    last_tap: Cell<Option<AnimInstant>>,
    bounds: TextHitBounds,
}

/// The rectangle of each text line a button hit-tests, in logical pixels.
///
/// The lines are relative to the button's origin, which comes from the canvas
/// until the render tree supplies it and from the tree afterwards.
#[derive(Debug, Default)]
struct TextHitBounds {
    /// `(width, offset below the first line, height)` of every line.
    lines: RefCell<Vec<(f32, f32, f32)>>,
    /// The whole text block: its origin, the widest line, and the full height.
    block: InteractionBounds,
}

impl TextHitBounds {
    fn save(
        &self,
        scale: f32,
        x: f32,
        y: f32,
        line_widths: &[f32],
        line_height: f32,
        total_height: f32,
    ) {
        let mut lines = self.lines.borrow_mut();
        lines.clear();
        let mut widest: f32 = 0.0;
        for (index, width) in line_widths.iter().copied().enumerate() {
            let offset_y = index as f32 * line_height;
            let height = line_height.min((total_height - offset_y).max(0.0));
            lines.push((width / scale, offset_y / scale, height / scale));
            widest = widest.max(width);
        }
        self.block.save(scale, x, y, widest, total_height);
    }

    fn is_inside(&self, x: f32, y: f32) -> bool {
        let Some(origin) = self.block.get_bounds() else {
            return false;
        };
        self.lines.borrow().iter().any(|(width, offset, height)| {
            origin.x <= x
                && x <= origin.x + width
                && origin.y + offset <= y
                && y <= origin.y + offset + height
        })
    }
}

impl RawTextButton {
    const DOUBLE_TAP_INTERVAL: Duration = Duration::from_millis(500);
    /// Largest normal-versus-hover label size difference, in device pixels,
    /// that still shares one render node's bounds.
    const SUBPIXEL_SIZE_TOLERANCE: f32 = 1.0;

    fn active_style_for(&self, hovered: bool) -> TextStyle {
        let (mut style, color) = if self.widget.disabled {
            (self.widget.disabled_style, self.widget.disabled_color)
        } else if hovered {
            (self.widget.hover_style, self.widget.hover_color)
        } else {
            (self.widget.style, self.widget.color)
        };
        if let Some(color) = color {
            style.color = color;
        }
        style
    }

    #[cfg(test)]
    fn text_element(&self) -> RawTextWidget {
        self.text_element_for(self.hovered.get())
    }

    fn text_element_for(&self, hovered: bool) -> RawTextWidget {
        RawTextWidget {
            text: self.widget.label.clone(),
            text_style: self.active_style_for(hovered),
            text_align: Default::default(),
            line_height: Default::default(),
            text_indent: 0.0,
            cache: LayoutCache::new(),
            _typeface: Cell::new(None),
        }
    }

    fn text_layout<'a>(
        &self,
        ctx: &BuildContext<'a>,
    ) -> (
        RawTextWidget,
        BuildContext<'a>,
        aimer_attribute::ResolvedSize,
        Vec<f32>,
        f32,
    ) {
        self.text_layout_for(ctx, self.hovered.get())
    }

    fn text_layout_for<'a>(
        &self,
        ctx: &BuildContext<'a>,
        hovered: bool,
    ) -> (
        RawTextWidget,
        BuildContext<'a>,
        aimer_attribute::ResolvedSize,
        Vec<f32>,
        f32,
    ) {
        let mut intrinsic_text = self.text_element_for(hovered);
        let wraps = matches!(intrinsic_text.text_style.text_overflow, TextOverflow::Wrap);
        intrinsic_text.text_style.text_overflow = TextOverflow::Clip;
        let intrinsic_size = intrinsic_text.computed_size(ctx);

        let mut text_ctx = ctx.clone();
        if wraps {
            let available_width = if ctx.box_constraint.max_width > 0.0 {
                ctx.box_constraint.max_width
            } else {
                ctx.parent_size.width
            };
            let width = intrinsic_size.width.min(available_width);
            text_ctx.box_constraint.max_width = width;
            text_ctx.parent_size.width = width;
        }

        let text = self.text_element_for(hovered);
        // Measuring the label that will be painted also prepares its retained
        // layout (a decorated label lays out as a paragraph), which local v2
        // paint reads from the label's cache.
        let measured = text.computed_size(&text_ctx);
        let size = if wraps { measured } else { intrinsic_size };
        let (line_widths, line_height) = if wraps {
            let font_size = text.font_size(text_ctx.scale);
            let metrics = text_ctx.canvas.measure_text_metrics_styled(
                &text.text,
                font_size,
                text_ctx.parent_size.width,
                text.text_style.font_family,
                text.text_style.font_style,
                text.text_style.font_weight.numeric(),
            );
            let widths = text_ctx.canvas.measure_text_line_widths_styled(
                &text.text,
                font_size,
                text_ctx.parent_size.width,
                text.text_style.font_family,
                text.text_style.font_style,
                text.text_style.font_weight.numeric(),
            );
            (widths, metrics.line_height)
        } else {
            (vec![size.width], size.height)
        };
        (text, text_ctx, size, line_widths, line_height)
    }

    fn local_v2_text<'a>(
        &self,
        ctx: &BuildContext<'a>,
    ) -> Option<(
        RawTextWidget,
        BuildContext<'a>,
        aimer_attribute::ResolvedSize,
    )> {
        let normal = self.text_layout_for(ctx, false);
        if normal.0.text_style.text_shadow.is_some()
            || normal.0.text_style.background_color.is_some()
            || !normal.0.can_paint_local_v2(&normal.1)
        {
            return None;
        }
        let hover = (!self.widget.disabled).then(|| self.text_layout_for(ctx, true));
        let mut sizes_differ = false;
        if let Some((hover_text, hover_ctx, hover_size, _, _)) = &hover {
            // A decoration moves the label onto the paragraph path, which does
            // not round the size up like plain text, so the two states can
            // differ by a fraction of a pixel. Anything larger is a real
            // resize: the node's bounds follow the state the button is in, so
            // the label is painted in that state rather than the cursor's.
            sizes_differ = (normal.2.width - hover_size.width).abs()
                > Self::SUBPIXEL_SIZE_TOLERANCE
                || (normal.2.height - hover_size.height).abs() > Self::SUBPIXEL_SIZE_TOLERANCE;
            if hover_text.text_style.text_shadow.is_some()
                || hover_text.text_style.background_color.is_some()
                || !hover_text.can_paint_local_v2(hover_ctx)
            {
                return None;
            }
        }

        let hovered = !self.widget.disabled
            && if sizes_differ {
                self.hovered.get()
            } else {
                self.bounds.is_inside(ctx.cursor_pos.x, ctx.cursor_pos.y)
            };
        let (text, text_ctx, size, _, _) = if hovered {
            hover.expect("enabled buttons have a hover layout")
        } else {
            normal
        };
        Some((text, text_ctx, size))
    }

    fn hover_style_changes_bounds(&self) -> bool {
        if self.widget.disabled {
            return false;
        }
        let normal = self.active_style_for(false);
        let hover = self.active_style_for(true);
        normal.font_size != hover.font_size
            || normal.font_family != hover.font_family
            || normal.font_style != hover.font_style
            || normal.font_weight != hover.font_weight
            || normal.text_overflow != hover.text_overflow
            || normal.text_decoration != hover.text_decoration
            || normal.text_transform != hover.text_transform
            || normal.letter_spacing != hover.letter_spacing
            || normal.word_spacing != hover.word_spacing
            || normal.text_shadow != hover.text_shadow
    }

    fn save_bounds(
        &self,
        ctx: &BuildContext,
        size: aimer_attribute::ResolvedSize,
        line_widths: &[f32],
        line_height: f32,
    ) {
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds
            .save(ctx.scale, x, y, line_widths, line_height, size.height);
    }

    /// Fires one press callback.
    ///
    /// The button holds no runtime handle, so an async callback goes to
    /// whichever runtime the frame is being built on.
    #[inline]
    fn execute(callback: &VoidCallback) {
        callback.execute(());
    }

    fn press(&self) {
        Self::execute(&self.widget.on_press);
        let now = AnimInstant::now();
        if self
            .last_tap
            .get()
            .is_some_and(|last| now.duration_since(last) <= Self::DOUBLE_TAP_INTERVAL)
        {
            Self::execute(&self.widget.on_double_press);
            self.last_tap.set(None);
        } else {
            self.last_tap.set(Some(now));
        }
    }

    fn set_hovered(&self, hovered: bool) -> bool {
        let changed = self.hovered.replace(hovered) != hovered;
        if changed {
            if self.hover_style_changes_bounds() {
                aimer_widget::mark_paint_damage_full();
            }
            request_animation_frame();
        }
        changed
    }
}

impl VisitorElement for RawTextButton {
    fn debug_name(&self) -> &'static str {
        "TextButton"
    }
}

impl EventElement for RawTextButton {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn semantics(
        &self,
        id: aimer_accessibility::NodeId,
    ) -> Option<aimer_accessibility::SemanticNode> {
        use aimer_accessibility::{Role, SemanticAction, SemanticNode};

        let node = SemanticNode::new(id, Role::Button)
            .with_name(self.widget.label.as_str())
            .enabled(!self.widget.disabled);
        Some(if self.widget.disabled {
            node
        } else {
            node.with_action(SemanticAction::Activate)
        })
    }

    fn perform_semantic_action(&self, action: &aimer_accessibility::SemanticAction) -> bool {
        if self.widget.disabled || *action != aimer_accessibility::SemanticAction::Activate {
            return false;
        }
        self.press();
        true
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerMove(info) if info.source == PointerSource::Mouse => {
                let changed = self.set_hovered(
                    self.bounds.is_inside(info.x(), info.y()) && !self.widget.disabled,
                );
                if changed {
                    EventResult::redraw()
                } else {
                    EventResult::ignored()
                }
            }
            ElementEvent::PointerExited(PointerSource::Mouse, _) => {
                let changed = self.set_hovered(false);
                self.interaction.borrow_mut().cancel();
                if changed {
                    EventResult::redraw()
                } else {
                    EventResult::ignored()
                }
            }
            ElementEvent::PointerDown(info) => {
                let inside = self.bounds.is_inside(info.x(), info.y());
                self.interaction
                    .borrow_mut()
                    .pointer_down(inside, self.widget.disabled);
                EventResult::from(inside && !self.widget.disabled)
            }
            ElementEvent::PointerUp(info) => {
                let action = self.interaction.borrow_mut().pointer_up(
                    self.bounds.is_inside(info.x(), info.y()),
                    self.widget.disabled,
                );
                if action == ButtonAction::Press {
                    self.press();
                    EventResult::consumed()
                } else {
                    EventResult::ignored()
                }
            }
            ElementEvent::Cancel => {
                self.interaction.borrow_mut().cancel();
                EventResult::ignored()
            }
            _ => EventResult::ignored(),
        }
    }
}

impl LayoutElement for RawTextButton {
    fn layout(&self, ctx: &BuildContext) -> aimer_attribute::ResolvedSize {
        let (_, _, size, line_widths, line_height) = self.text_layout(ctx);
        self.save_bounds(ctx, size, &line_widths, line_height);
        size
    }

    fn computed_size(&self, ctx: &BuildContext) -> aimer_attribute::ResolvedSize {
        self.text_layout(ctx).2
    }
}

impl Drawable for RawTextButton {
    /// The button hit-tests its text block: the widest line by the full height.
    fn retained_v2_interaction_size(
        &self,
        ctx: &BuildContext,
    ) -> Option<aimer_attribute::ResolvedSize> {
        let (_, _, size, line_widths, _) = self.text_layout(ctx);
        Some(aimer_attribute::ResolvedSize {
            width: line_widths.iter().copied().fold(0.0, f32::max),
            height: size.height,
        })
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.block.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.block.disagreement("TextButton")
    }

    fn update(&self, ctx: &BuildContext) {
        let (text, text_ctx, size, line_widths, line_height) = self.text_layout(ctx);
        self.save_bounds(ctx, size, &line_widths, line_height);
        let hover_changed = !self.widget.disabled
            && self.set_hovered(self.bounds.is_inside(ctx.cursor_pos.x, ctx.cursor_pos.y));
        if hover_changed {
            // The first layout above is needed to establish the transformed
            // hit bounds. Re-resolve the text after syncing hover so a cursor
            // that is already over a link is painted with its hover style in
            // this frame instead of waiting for a second repaint.
            let (text, text_ctx, size, line_widths, line_height) = self.text_layout(ctx);
            self.save_bounds(ctx, size, &line_widths, line_height);
            text.update(&text_ctx);
            return;
        }
        text.update(&text_ctx);
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.local_v2_text(ctx).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let (text, text_ctx, _) = self
            .local_v2_text(ctx)
            .expect("TextButton v2 support must be checked before painting");
        text.paint_local_v2(&text_ctx);
    }
}

impl Rebuildable for RawTextButton {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};

    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_style::TextDecoration;
    use aimer_widget::base::WindowHandle;

    use super::*;

    fn context(max_width: f32) -> BuildContext<'static> {
        context_with_canvas(max_width).0
    }

    fn context_with_canvas(max_width: f32) -> (BuildContext<'static>, aimer_canvas::InnerCanvas) {
        let canvas = aimer_canvas::InnerCanvas::new();
        let inner = Box::leak(Box::new(canvas.clone()));
        let mut ctx = BuildContext::new(
            aimer_canvas::FrameCanvas::new(inner),
            ResolvedSize {
                width: max_width,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = BoxConstraint::new().max_width(max_width).max_height(100.0);
        (ctx, canvas)
    }

    /// Records `element`'s own retained list, as the frame loop does for a node.
    fn record_local_v2(
        element: &impl Drawable,
        ctx: &BuildContext<'_>,
    ) -> std::sync::Arc<[aimer_cupid::draw_cmd_v2::DrawCommand]> {
        let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
        let root = tree
            .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 400.0, 300.0))
            .unwrap();
        let node_context = tree.context(root).unwrap();
        ctx.with_local_v2_paint_context(node_context, |ctx| {
            assert!(element.can_paint_local_v2(ctx));
            element.paint_local_v2(ctx);
        });
        tree.draw_list_snapshot(root).unwrap().commands
    }

    fn raw_button(widget: TextButton) -> RawTextButton {
        RawTextButton {
            widget,
            hovered: Cell::new(false),
            interaction: RefCell::new(ButtonInteraction::default()),
            last_tap: Cell::new(None),
            bounds: TextHitBounds::default(),
        }
    }

    #[test]
    fn text_button_publishes_a_named_activatable_button() {
        use aimer_accessibility::{NodeId, Role, SemanticAction};

        let button = raw_button(TextButton::new("Open"));

        let node = button.semantics(NodeId::new(7)).expect("a button describes itself");

        assert_eq!(node.id(), NodeId::new(7));
        assert_eq!(node.role(), Role::Button);
        assert_eq!(node.name(), Some("Open"));
        assert_eq!(node.actions(), [SemanticAction::Activate]);
        assert!(node.state().enabled());
    }

    #[test]
    fn disabled_text_button_publishes_no_action_and_refuses_activation() {
        use aimer_accessibility::{NodeId, SemanticAction};

        let button = raw_button(TextButton::new("Open").disabled(true));

        let node = button.semantics(NodeId::new(7)).unwrap();

        assert!(node.actions().is_empty());
        assert!(!node.state().enabled());
        assert!(!button.perform_semantic_action(&SemanticAction::Activate));
    }

    #[test]
    fn activating_the_text_button_runs_its_press_callback() {
        use aimer_accessibility::SemanticAction;

        let pressed = Rc::new(Cell::new(0));
        let counter = pressed.clone();
        let button = raw_button(
            TextButton::new("Open")
                .on_press(VoidCallback::from(move || counter.set(counter.get() + 1))),
        );

        assert!(button.perform_semantic_action(&SemanticAction::Activate));
        assert!(!button.perform_semantic_action(&SemanticAction::Dismiss));
        assert_eq!(pressed.get(), 1);
    }

    #[test]
    fn text_button_opts_into_indexed_event_routing() {
        let button = raw_button(TextButton::new("Open"));

        assert_eq!(button.event_tree_role(), EventTreeRole::IndexedTarget);
    }

    #[test]
    fn press_requires_down_and_up_inside_the_text_bounds() {
        let mut state = ButtonInteraction::default();

        assert_eq!(state.pointer_down(true, false), ButtonAction::None);
        assert_eq!(state.pointer_up(true, false), ButtonAction::Press);

        state.pointer_down(true, false);
        assert_eq!(state.pointer_up(false, false), ButtonAction::None);
    }

    #[test]
    fn disabled_button_never_arms_or_presses() {
        let mut state = ButtonInteraction::default();

        state.pointer_down(true, true);

        assert_eq!(state.pointer_up(true, true), ButtonAction::None);
    }

    #[test]
    fn synchronous_press_callback_is_executed() {
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let callback = VoidCallback::from(move || observed.set(observed.get() + 1));

        RawTextButton::execute(&callback);

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn wrapped_text_button_shrink_wraps_label_and_hitbox() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let ctx = context(300.0);
        let button = raw_button(
            TextButton::new("Open").style(TextStyle::default().text_overflow(TextOverflow::Wrap)),
        );
        let mut intrinsic_text = button.text_element();
        intrinsic_text.text_style.text_overflow = TextOverflow::Clip;
        let intrinsic = intrinsic_text.computed_size(&ctx);

        let size = button.layout(&ctx);

        assert_eq!(size, intrinsic);
        assert!(
            !button
                .on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
                    Vec2d {
                        x: intrinsic.width + 1.0,
                        y: intrinsic.height / 2.0
                    },
                    PointerButton::Primary,
                )))
                .is_consumed()
        );
    }

    #[test]
    fn wrapped_text_button_respects_narrow_available_width() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let ctx = context(30.0);
        let button = raw_button(
            TextButton::new("A label that wraps")
                .style(TextStyle::default().text_overflow(TextOverflow::Wrap)),
        );

        let size = button.layout(&ctx);

        assert_eq!(size.width, 30.0);
        assert!(size.height > 17.0);
        assert!(
            !button
                .on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
                    Vec2d {
                        x: 31.0,
                        y: size.height / 2.0
                    },
                    PointerButton::Primary,
                )))
                .is_consumed()
        );
    }

    #[test]
    fn wrapped_text_button_excludes_empty_space_after_short_final_line() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let ctx = context(45.0);
        let button = raw_button(
            TextButton::new("MMMM i").style(TextStyle::default().text_overflow(TextOverflow::Wrap)),
        );

        let size = button.layout(&ctx);

        assert!(size.height > 17.0);
        assert!(
            button
                .on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
                    Vec2d {
                        x: 1.0,
                        y: size.height - 1.0
                    },
                    PointerButton::Primary,
                )))
                .is_consumed()
        );
        assert!(
            !button
                .on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
                    Vec2d {
                        x: size.width - 1.0,
                        y: size.height - 1.0
                    },
                    PointerButton::Primary,
                )))
                .is_consumed()
        );
    }

    #[test]
    fn wrapped_underlined_text_button_draws_each_line_decoration() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (mut ctx, _canvas) = context_with_canvas(45.0);
        ctx.cursor_pos = Vec2d { x: -1.0, y: -1.0 };
        let button = raw_button(
            TextButton::new("MMMM i").style(
                TextStyle::default()
                    .text_overflow(TextOverflow::Wrap)
                    .text_decoration(TextDecoration::Underline),
            ),
        );

        // Layout establishes the hit bounds and the shaped paragraph the
        // retained paint reads, as the frame's tree sync does.
        button.layout(&ctx);
        let commands = record_local_v2(&button, &ctx);
        let decorations = commands
            .iter()
            .filter_map(|command| match command {
                aimer_cupid::draw_cmd_v2::DrawCommand::DrawTextDecoration { rect, .. } => {
                    Some(rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(decorations.len(), 2);
        assert!(decorations[1].y > decorations[0].y);
        assert!(decorations[1].width < decorations[0].width);
    }

    #[test]
    fn text_button_draws_hover_style_when_cursor_is_already_over_the_label() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (mut ctx, _canvas) = context_with_canvas(300.0);
        ctx.cursor_pos = Vec2d { x: 1.0, y: 1.0 };
        let button = raw_button(
            TextButton::new("Open")
                .hover_style(TextStyle::default().text_decoration(TextDecoration::Underline)),
        );

        button.layout(&ctx);
        assert!(record_local_v2(&button, &ctx).iter().any(|command| {
            matches!(
                command,
                aimer_cupid::draw_cmd_v2::DrawCommand::DrawTextDecoration { .. }
            )
        }));
    }

    #[test]
    fn text_button_hover_transition_requests_redraw_without_full_damage() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let ctx = context(300.0);
        let button = raw_button(TextButton::new("Open"));
        button.layout(&ctx);

        aimer_widget::begin_paint_frame(300, 100);
        assert!(aimer_widget::take_paint_frame_damage(300, 100).is_full());
        aimer_widget::begin_paint_frame(300, 100);
        assert!(aimer_widget::take_paint_frame_damage(300, 100).is_empty());

        let result = button.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(
            Vec2d { x: 1.0, y: 1.0 },
            PointerButton::Primary,
        )));

        assert!(result.needs_redraw());
        assert!(!result.is_consumed());
        aimer_widget::begin_paint_frame(300, 100);
        assert!(aimer_widget::take_paint_frame_damage(300, 100).is_empty());
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn text_button_publishes_its_derived_properties_and_async_callbacks() {
        use aimer_widget::portable::PortableWidgetSchema;

        let schema = <super::TextButton as PortableWidgetSchema>::SCHEMA;

        assert_eq!(
            schema.widget().canonical_name(),
            "aimer.widget:aimer_text::TextButton"
        );
        assert_eq!(
            schema
                .properties()
                .iter()
                .map(|property| property.canonical_name())
                .collect::<Vec<_>>(),
            vec![
                "aimer.property:aimer_text::TextButton:disabled",
                "aimer.property:aimer_text::TextButton:label",
                "aimer.property:aimer_text::TextButton:color",
                "aimer.property:aimer_text::TextButton:hover_color",
                "aimer.property:aimer_text::TextButton:disabled_color",
                "aimer.property:aimer_text::TextButton:style",
                "aimer.property:aimer_text::TextButton:hover_style",
                "aimer.property:aimer_text::TextButton:disabled_style",
            ]
        );
        assert_eq!(
            schema
                .callbacks()
                .iter()
                .map(|callback| callback.canonical_name())
                .collect::<Vec<_>>(),
            vec![
                "aimer.event:aimer_text::TextButton:on_press",
                "aimer.event:aimer_text::TextButton:on_double_press",
            ]
        );
        assert!(
            schema
                .callbacks()
                .iter()
                .all(|callback| callback.async_schema().is_some())
        );
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn text_button_lowering_retains_properties_and_callback_routes() {
        use std::cell::Cell;

        use aimer_anteros::WidgetDocumentView;
        use aimer_widget::PortableWidget;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, PortableWidgetSchema,
            SourceFingerprint, StableId128,
        };

        let calls = Rc::new(Cell::new(0_u8));
        let press_calls = Rc::clone(&calls);
        let double_press_calls = Rc::clone(&calls);
        let mut context = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 4_096)
                .with_max_callbacks(4)
                .with_max_blob_bytes(128),
            PortableLimits::new(8, 16, 64, 128, 4_096),
        )
        .unwrap();
        let source = SourceFingerprint::new(StableId128::from_bytes([4; 16]));
        let root = super::TextButton::new("Open")
            .disabled(true)
            .color(Color::Rgba(1, 2, 3, 4))
            .hover_color(Color::Rgba(5, 6, 7, 8))
            .disabled_color(Color::Rgba(9, 10, 11, 12))
            .style(TextStyle::default().font_size(14))
            .hover_style(TextStyle::default().font_size(15))
            .disabled_style(TextStyle::default().font_size(16))
            .on_press(move || press_calls.set(press_calls.get() | 1))
            .on_double_press(move || double_press_calls.set(double_press_calls.get() | 2))
            .to_portable_node(&mut context, source)
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        let schema = <super::TextButton as PortableWidgetSchema>::SCHEMA;
        let mut expected = schema
            .properties()
            .iter()
            .map(|property| property.id())
            .collect::<Vec<_>>();
        expected.sort_unstable();
        let mut actual = node
            .properties()
            .map(|property| property.property_id())
            .collect::<Vec<_>>();
        actual.sort_unstable();
        let callbacks = node.callbacks().collect::<Vec<_>>();

        assert_eq!(actual, expected);
        assert_eq!(callbacks.len(), 2);
        for callback in callbacks {
            context
                .callback_registry()
                .dispatch(
                    aimer_widget::portable::StableId128::from_bytes(
                        *callback.callback_id().as_bytes(),
                    ),
                    &mut context,
                )
                .unwrap();
        }
        assert_eq!(calls.get(), 3);
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn text_button_async_callback_uses_the_reflected_async_route() {
        use aimer_anteros::WidgetDocumentView;
        use aimer_widget::PortableWidget;
        use aimer_widget::portable::{
            PortableBuildContext, PortableCallbackStart, PortableLimits, PortableWidgetLimits,
            SourceFingerprint, StableId128,
        };

        let mut context = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 4_096)
                .with_max_callbacks(4)
                .with_max_blob_bytes(128),
            PortableLimits::new(8, 16, 64, 128, 4_096),
        )
        .unwrap();
        let source = SourceFingerprint::new(StableId128::from_bytes([6; 16]));
        let root = super::TextButton::new("Open")
            .on_press(VoidCallback::from_async(|| async {}))
            .to_portable_node(&mut context, source)
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let binding = view
            .node(root.index())
            .unwrap()
            .callbacks()
            .next()
            .expect("the configured async callback is reflected");
        let callback_id =
            aimer_widget::portable::StableId128::from_bytes(*binding.callback_id().as_bytes());

        assert!(binding.is_async());
        assert!(matches!(
            context
                .callback_registry()
                .dispatch_start(callback_id, &mut context)
                .unwrap(),
            PortableCallbackStart::Started { .. }
        ));
        context.run_async_microtasks();
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn text_button_portable_lowering_encodes_non_default_styles() {
        use aimer_widget::PortableWidget;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
            StableId128,
        };

        let mut context = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 4_096)
                .with_max_callbacks(4)
                .with_max_blob_bytes(128),
            PortableLimits::new(8, 16, 64, 128, 4_096),
        )
        .unwrap();
        let source = SourceFingerprint::new(StableId128::from_bytes([7; 16]));
        let root = TextButton::new("Open")
            .style(TextStyle::default().font_size(18))
            .to_portable_node(&mut context, source)
            .unwrap();
        context.finish_document(root).unwrap();
    }

    fn underline_on_hover_button() -> RawTextButton {
        raw_button(
            TextButton::new("Tab")
                .style(TextStyle::new().font_size(20))
                .hover_style(
                    TextStyle::new()
                        .font_size(20)
                        .text_decoration(TextDecoration::Underline),
                ),
        )
    }

    #[test]
    fn a_button_that_underlines_on_hover_paints_locally() {
        // The decoration moves the label onto the paragraph path, whose size is
        // not rounded up like plain text: normal and hover differ by a
        // fraction of a pixel, which must not push the button to a legacy
        // island.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let mut ctx = context(300.0);
        ctx.cursor_pos = Vec2d { x: -1.0, y: -1.0 };
        let button = underline_on_hover_button();

        assert!(button.can_paint_local_v2(&ctx));
    }

    #[test]
    fn a_hovered_underlined_button_paints_locally() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let ctx = context(300.0);
        let button = underline_on_hover_button();
        button.layout(&ctx);
        let mut hovered = ctx.clone();
        hovered.cursor_pos = Vec2d { x: 2.0, y: 2.0 };

        assert!(button.can_paint_local_v2(&hovered));
    }

    #[test]
    fn a_hover_style_that_resizes_the_label_paints_the_state_the_button_is_in() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let ctx = context(300.0);
        let button = raw_button(
            TextButton::new("Tab")
                .style(TextStyle::new().font_size(20))
                .hover_style(TextStyle::new().font_size(28)),
        );
        button.layout(&ctx);

        let font_size = |button: &RawTextButton| {
            record_local_v2(button, &ctx)
                .iter()
                .find_map(|command| match command {
                    aimer_cupid::draw_cmd_v2::DrawCommand::DrawText { font_size, .. } => {
                        Some(*font_size)
                    }
                    _ => None,
                })
        };
        // The node's bounds follow the state, so the label is painted in it
        // rather than left blank because the two states differ in size.
        assert_eq!(font_size(&button), Some(20.0));
        button.set_hovered(true);
        assert_eq!(font_size(&button), Some(28.0));
    }
}
