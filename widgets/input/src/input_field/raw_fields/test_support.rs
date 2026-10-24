
use std::sync::Arc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::size::ResolvedSize;
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_events::element::{ElementEvent, KeyAction, Modifiers};
use aimer_style::{BoxDecoration, LayoutSpacing, Spacing, TextAlign, TextStyle};
use aimer_widget::base::{BuildContext, Color, Colors, WindowHandle};
use aimer_widget::{EventElement, FocusNode};

use super::{ExpandDirection, InputType, RawFieldConfig, RawTextField, TextFieldCallback};
use crate::TextEditingController;
use crate::input_field::caret::CaretBlink;

/// Builds the configuration of a focused, editable single-line field around
/// `controller`.
pub(super) fn field_config(controller: impl Into<TextEditingController>) -> RawFieldConfig {
    RawFieldConfig {
        input_type: InputType::Text,
        controller: controller.into(),
        prompt: Arc::from(""),
        hint: Arc::from(""),
        hint_style: TextStyle::default(),
        text_style: TextStyle::default(),
        prompt_style: TextStyle::default(),
        text_align: TextAlign::default(),
        auto_focus: true,
        max_lines: None,
        min_lines: None,
        max_length: None,
        enable: true,
        expand: ExpandDirection::default(),
        decoration: BoxDecoration::default(),
        hover_decoration: None,
        focus_decoration: None,
        disabled_decoration: None,
        selection_color: Color::Rgba(66, 133, 244, 100),
        cursor_color: Colors::default(),
        caret_builder: None,
        on_changed: TextFieldCallback::default(),
        on_submitted: TextFieldCallback::default(),
        on_focus: TextFieldCallback::default(),
        on_blur: TextFieldCallback::default(),
        read_only: false,
        key_filter: None,
        padding: LayoutSpacing::all(Spacing::Px(4)),
        chrome: None,
        visual: None,
    }
}

/// Builds a focused, editable single-line field around `controller`.
pub(super) fn focused_field(controller: impl Into<TextEditingController>) -> RawTextField {
    let field = RawTextField::new(
        field_config(controller),
        CaretBlink::new(),
        FocusNode::new(),
    );
    let _ = field.on_event(&ElementEvent::FocusGained);
    field
}

/// Builds a focused field that blinks on `caret`.
pub(super) fn focused_field_with_caret(
    controller: impl Into<TextEditingController>,
    caret: CaretBlink,
) -> RawTextField {
    let field = RawTextField::new(field_config(controller), caret, FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field
}

/// A text payload delivered as one batched edit, like an IME commit.
pub(super) fn commit(text: &str) -> ElementEvent {
    ElementEvent::TextInput {
        text: text.to_owned(),
        action: KeyAction::Pressed,
        modifiers: Modifiers::default(),
    }
}

/// Builds a focused, editable field limited to one line, the way
/// [`crate::TextField`] configures its element.
///
/// The shared configuration leaves the line count open, which is the
/// multiline shape a [`crate::TextArea`] builds; a single-line field takes
/// a different painting path, so it needs its own fixture.
pub(super) fn focused_single_line_field(
    controller: impl Into<TextEditingController>,
) -> RawTextField {
    let mut config = field_config(controller);
    config.min_lines = Some(1);
    config.max_lines = Some(1);
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field
}

/// Builds a focused, editable field that reserves `min_lines` lines and
/// grows without a line limit, the way [`crate::TextArea`] configures its
/// element.
pub(super) fn focused_multiline_field(
    controller: impl Into<TextEditingController>,
    min_lines: usize,
) -> RawTextField {
    let mut config = field_config(controller);
    config.min_lines = Some(min_lines);
    config.max_lines = None;
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field
}

/// Builds a headless [`BuildContext`] constrained to `width` x `height`.
///
/// Drawing a field needs a canvas for text measurement and a window handle
/// for the platform calls it makes while focused. Nothing reaches a GPU:
/// the canvas measures with the font backend and records the draw calls.
pub(crate) fn dummy_build_context(width: f32, height: f32) -> BuildContext<'static> {
    let canvas = {
        let leaked: &'static InnerCanvas = Box::leak(Box::new(InnerCanvas::new()));
        FrameCanvas::new(leaked)
    };

    let mut context = BuildContext::new(
        canvas,
        ResolvedSize { width, height },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(
            winit::dpi::PhysicalSize::new(width.max(1.0) as u32, height.max(1.0) as u32),
            1.0,
        ),
        #[cfg(not(target_arch = "wasm32"))]
        dummy_async_handle(),
    );
    context.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: width,
        max_height: height,
    };
    context
}

/// The runtime handle a headless [`BuildContext`] carries.
#[cfg(not(target_arch = "wasm32"))]
fn dummy_async_handle() -> tokio::runtime::Handle {
    use std::sync::OnceLock;

    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
    });
    let _guard = runtime.enter();
    tokio::runtime::Handle::current()
}

/// A key press with no modifiers held.
pub(super) fn key(key: aimer_events::element::NamedKey) -> ElementEvent {
    ElementEvent::KeyInput {
        key,
        action: KeyAction::Pressed,
        modifiers: Modifiers::default(),
    }
}
