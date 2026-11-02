use std::panic::Location;
use std::sync::Arc;

use aimer_decoration::Decoration;
use aimer_macro::PortableWidget;
use aimer_style::{LayoutSpacing, TextAlign, TextStyle};
use aimer_widget::base::{BuildContext, Color, Colors};
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{PortableBuildContext, PortableBuildError, SourceFingerprint};
use aimer_widget::{
    AnyElement, FocusNode, Key, State, StateUpdater, StatefulElement, StatefulWidget, Widget,
};

use crate::TextEditingController;
use crate::input_field::caret::CaretContext;
use crate::input_field::chrome::TextFieldVisualState;
use crate::input_field::raw_fields::{ExpandDirection, InputType, TextFieldCallback};
use crate::input_field::{TextField, TextFieldState};

/// A multiline text input widget.
///
/// A text area starts with room for three lines, grows without a line limit,
/// and keeps the shared text-field editing, focus, decoration, and callback
/// behavior. Pressing Return inserts a line break; submission remains available
/// through the platform's modified Return action.
#[derive(PortableWidget)]
#[portable_widget(
    id = "aimer_input::TextArea",
    validate = validate_portable_text_area
)]
pub struct TextArea {
    // The embedded field owns the controller, focus, styling, and callback
    // state. Only the independently representable expansion policy is lowered.
    #[portable_skip]
    field: TextField,
    #[portable_skip]
    min_lines: usize,
    #[portable_skip]
    max_lines: Option<usize>,
    /// Fixed-width guest descriptors for the native line-count configuration.
    ///
    /// The native API remains `usize`-based, but the guest contract uses `u32`
    /// so its representation is identical on every supported target.
    min_lines_wire: u32,
    max_lines_wire: Option<u32>,
    #[portable_optional]
    expand: bool,
}

#[cfg(feature = "portable-guest")]
fn validate_portable_text_area(
    area: &TextArea,
    _ctx: &PortableBuildContext,
    source: SourceFingerprint,
) -> Result<(), PortableBuildError> {
    if let Some(property) = area.field.native_only_property() {
        return Err(PortableBuildError::UnsupportedProperty {
            widget: "TextArea",
            property,
            source,
        });
    }
    let expected_min = u32::try_from(area.min_lines.max(1))
        .map_err(|_| PortableBuildError::InvalidPropertyValue { rust_type: "usize" })?;
    if area.min_lines_wire != expected_min {
        return Err(PortableBuildError::UnsupportedProperty {
            widget: "TextArea",
            property: "min_lines",
            source,
        });
    }

    let expected_max = match area.max_lines {
        None => None,
        Some(value) => Some(
            u32::try_from(value)
                .map_err(|_| PortableBuildError::InvalidPropertyValue { rust_type: "usize" })?,
        ),
    };
    if area.max_lines_wire != expected_max {
        return Err(PortableBuildError::UnsupportedProperty {
            widget: "TextArea",
            property: "max_lines",
            source,
        });
    }
    Ok(())
}

impl TextArea {
    /// Creates an empty, enabled, editable multiline field.
    ///
    /// The field reserves at least three lines, has no maximum line count, and
    /// sizes itself to its content rather than expanding vertically.
    #[inline]
    pub fn new() -> Self {
        Self {
            field: TextField::new(),
            min_lines: 3,
            max_lines: None,
            min_lines_wire: 3,
            max_lines_wire: None,
            expand: false,
        }
    }

    /// Uses `controller` as the area's shared text and editing-history owner.
    ///
    /// Clones of the controller observe the same text, selection, undo stack,
    /// and redo stack.
    #[inline]
    pub fn controller(mut self, controller: TextEditingController) -> Self {
        self.field = self.field.controller(controller);
        self
    }

    /// Attaches the handle used for imperative focus control.
    ///
    /// Retain a clone of `focus_node` to request or release focus after the area
    /// is mounted. The node remains attached across rebuilds.
    #[inline]
    pub fn focus_node(mut self, focus_node: FocusNode) -> Self {
        self.field = self.field.focus_node(focus_node);
        self
    }

    /// Sets the prompt drawn when the area is empty and focused.
    ///
    /// The string is reference counted so rebuilding the area does not copy its
    /// contents.
    #[inline]
    pub fn prompt(mut self, prompt: impl Into<Arc<str>>) -> Self {
        self.field = self.field.prompt(prompt);
        self
    }

    /// Sets the hint drawn when the area is empty and not showing its prompt.
    ///
    /// The string is reference counted so rebuilding the area does not copy its
    /// contents.
    #[inline]
    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        self.field = self.field.hint(hint);
        self
    }

    /// Replaces the style used to lay out and paint the hint.
    #[inline]
    pub fn hint_style(mut self, hint_style: TextStyle) -> Self {
        self.field = self.field.hint_style(hint_style);
        self
    }

    /// Replaces the style used to lay out and paint entered text.
    #[inline]
    pub fn text_style(mut self, text_style: TextStyle) -> Self {
        self.field = self.field.text_style(text_style);
        self
    }

    /// Replaces the style used to lay out and paint the focused empty prompt.
    #[inline]
    pub fn prompt_style(mut self, prompt_style: TextStyle) -> Self {
        self.field = self.field.prompt_style(prompt_style);
        self
    }

    /// Sets the alignment of text within the area's content region.
    #[inline]
    pub fn text_align(mut self, text_align: TextAlign) -> Self {
        self.field = self.field.text_align(text_align);
        self
    }

    /// Sets whether a newly created area starts focused.
    ///
    /// This initializes focus when the widget becomes an element; it does not
    /// request focus from an area that is already mounted.
    #[inline]
    pub fn auto_focus(mut self, auto_focus: bool) -> Self {
        self.field = self.field.auto_focus(auto_focus);
        self
    }

    /// Sets the minimum number of visible text lines.
    ///
    /// Values below one resolve to one when the field state is created. If the
    /// configured maximum is smaller, that maximum resolves to this minimum.
    #[inline]
    pub fn min_lines(mut self, min_lines: usize) -> Self {
        self.min_lines = min_lines;
        self.min_lines_wire = u32::try_from(min_lines.max(1)).unwrap_or(u32::MAX);
        self
    }

    /// Sets or removes the maximum number of visible text lines.
    ///
    /// Passing an integer sets a finite maximum, while `None` allows the area to
    /// grow without a line limit. A finite value below [`Self::min_lines`] is
    /// raised to the resolved minimum when the field state is created.
    #[inline]
    pub fn max_lines(mut self, max_lines: impl Into<Option<usize>>) -> Self {
        let max_lines = max_lines.into();
        self.max_lines_wire = max_lines.map(|value| u32::try_from(value).unwrap_or(u32::MAX));
        self.max_lines = max_lines;
        self
    }

    /// Applies the bounded guest minimum-line descriptor while materializing a
    /// fresh area.
    #[doc(hidden)]
    #[inline]
    fn min_lines_wire(mut self, min_lines: u32) -> Self {
        self.min_lines = min_lines as usize;
        self.min_lines_wire = min_lines;
        self
    }

    /// Applies the bounded guest maximum-line descriptor while materializing a
    /// fresh area.
    #[doc(hidden)]
    #[inline]
    fn max_lines_wire(mut self, max_lines: u32) -> Self {
        self.max_lines = Some(max_lines as usize);
        self.max_lines_wire = Some(max_lines);
        self
    }

    /// Sets whether the area fills the available vertical extent.
    ///
    /// When `false`, its height follows the resolved line constraints and text
    /// content. When `true`, it expands vertically to its parent's constraint.
    #[inline]
    pub fn expand(mut self, expand: bool) -> Self {
        self.expand = expand;
        self
    }

    /// Sets the optional maximum input length in Unicode scalar values.
    ///
    /// `None` removes the limit. User input beyond a finite limit is not
    /// inserted.
    #[inline]
    pub fn max_length(mut self, max_length: Option<usize>) -> Self {
        self.field = self.field.max_length(max_length);
        self
    }

    /// Enables or disables focus, editing, selection, and input callbacks.
    ///
    /// A disabled area uses its configured disabled decoration when one is
    /// available.
    #[inline]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.field = self.field.enable(enabled);
        self
    }

    /// Sets whether keyboard and input-method edits are blocked.
    ///
    /// A read-only area still supports focus, selection, copying, and cursor
    /// navigation.
    #[inline]
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.field = self.field.read_only(read_only);
        self
    }

    /// Replaces the normal area decoration.
    ///
    /// Accepts any [`Decoration`], with the same rules as
    /// [`TextField::decoration`]. A [`builder`](Self::builder) overrides every
    /// decoration.
    #[inline]
    pub fn decoration<D: Decoration + 'static>(mut self, decoration: D) -> Self {
        self.field = self.field.decoration(decoration);
        self
    }

    /// Sets the decoration used while an enabled, unfocused area is hovered.
    #[inline]
    pub fn hover_decoration<D: Decoration + 'static>(mut self, hover_decoration: D) -> Self {
        self.field = self.field.hover_decoration(hover_decoration);
        self
    }

    /// Sets the decoration used while the enabled area is focused.
    ///
    /// Focus decoration takes precedence over hover decoration.
    #[inline]
    pub fn focus_decoration<D: Decoration + 'static>(mut self, focus_decoration: D) -> Self {
        self.field = self.field.focus_decoration(focus_decoration);
        self
    }

    /// Sets the decoration used while the area is disabled.
    ///
    /// Disabled decoration takes precedence over focus and hover decorations.
    #[inline]
    pub fn disabled_decoration<D: Decoration + 'static>(mut self, disabled_decoration: D) -> Self {
        self.field = self.field.disabled_decoration(disabled_decoration);
        self
    }

    /// Builds the frame around the editing surface from the area's state.
    ///
    /// Works exactly like [`TextField::builder`]: the closure returns the whole
    /// appearance and places the live editing surface with
    /// [`TextFieldVisualState::input`]. **Every decoration set on this area is
    /// ignored once a builder is set**, and the editing surface paints no
    /// background, border or shadow of its own; `padding`, line limits and
    /// every other setting still apply.
    #[inline]
    pub fn builder<W: Widget + 'static>(
        mut self,
        build: impl Fn(TextFieldVisualState) -> W + 'static,
    ) -> Self {
        self.field = self.field.builder(build);
        self
    }

    /// Sets the color painted behind selected text.
    #[inline]
    pub fn selection_color(mut self, selection_color: impl Into<Color>) -> Self {
        self.field = self.field.selection_color(selection_color);
        self
    }

    /// Sets the color of the builtin insertion cursor.
    ///
    /// This has no effect on a custom caret builder. Custom carets receive the
    /// live [`CaretContext`] and choose their own visual style.
    #[inline]
    pub fn cursor_color(mut self, cursor_color: Colors) -> Self {
        self.field = self.field.cursor_color(cursor_color);
        self
    }

    /// Replaces the builtin insertion cursor with a custom caret widget.
    ///
    /// The builder runs once when the area's visual caret element is mounted.
    /// The returned widget is retained, while its [`CaretContext`] continues to
    /// report live geometry and focus state as the area changes.
    #[inline]
    pub fn caret<F, W>(mut self, builder: F) -> Self
    where
        F: Fn(CaretContext) -> W + 'static,
        W: Widget + 'static,
    {
        self.field = self.field.caret(builder);
        self
    }

    /// Restores the builtin insertion cursor after a custom builder was set.
    #[inline]
    pub fn builtin_caret(mut self) -> Self {
        self.field = self.field.builtin_caret();
        self
    }

    /// Sets the callback invoked after a user edit changes the text.
    ///
    /// The callback receives the complete updated string. Programmatic
    /// controller mutations do not dispatch this callback.
    #[inline]
    pub fn on_changed(mut self, on_changed: impl Into<TextFieldCallback>) -> Self {
        self.field = self.field.on_changed(on_changed);
        self
    }

    /// Sets the callback invoked when the user submits the area.
    ///
    /// Plain Return inserts a newline. Submission is available through the
    /// platform's modified Return action and receives the complete string.
    #[inline]
    pub fn on_submitted(mut self, on_submitted: impl Into<TextFieldCallback>) -> Self {
        self.field = self.field.on_submitted(on_submitted);
        self
    }

    /// Sets the callback invoked when the area gains focus.
    ///
    /// The callback receives the complete current string.
    #[inline]
    pub fn on_focus(mut self, on_focus: impl Into<TextFieldCallback>) -> Self {
        self.field = self.field.on_focus(on_focus);
        self
    }

    /// Sets the callback invoked when the area loses focus.
    ///
    /// The callback receives the complete current string.
    #[inline]
    pub fn on_blur(mut self, on_blur: impl Into<TextFieldCallback>) -> Self {
        self.field = self.field.on_blur(on_blur);
        self
    }

    /// Sets the spacing between the area decoration and its text content.
    #[inline]
    pub fn padding(mut self, padding: impl Into<LayoutSpacing>) -> Self {
        self.field = self.field.padding(padding);
        self
    }

    /// Sets the identity of this area for widget reconciliation.
    ///
    /// A keyed area keeps its state, caret timeline, focus, and selection when
    /// the surrounding widget list is reordered.
    #[inline]
    #[track_caller]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        let caller = Location::caller();
        self.field = self.field.key(key.into().with_location(caller));
        self
    }

    #[inline]
    fn config(&self) -> crate::input_field::raw_fields::RawFieldConfig {
        let mut config = self.field.config();
        config.input_type = InputType::Text;
        config.min_lines = Some(self.min_lines.max(1));
        config.max_lines = self
            .max_lines
            .map(|max_lines| max_lines.max(config.min_lines.unwrap_or(1)));
        config.expand = if self.expand {
            ExpandDirection::Vertical
        } else {
            ExpandDirection::None
        };
        config
    }
}

impl Default for TextArea {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl StatefulWidget for TextArea {
    type State = TextFieldState;

    #[inline]
    fn create_state(self) -> Self::State {
        self.field.create_state_with_config(self.config())
    }
}

impl State<TextArea> for TextFieldState {
    #[inline]
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.init(updater);
    }

    #[inline]
    fn adopt_config_from(&mut self, new: Self) {
        self.adopt(new);
    }

    #[inline]
    fn build(&self, _: &BuildContext) -> impl Widget {
        self.build_field()
    }
}

impl Widget for TextArea {
    #[inline]
    fn key(&self) -> Option<Key> {
        Widget::key(&self.field)
    }

    #[inline]
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let __key = Widget::key(&self);
        StatefulElement::new_with_name(self, ctx, "TextArea", __key)
            .0
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    use aimer_attribute::Vec2d;
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_style::{BoxDecoration, LayoutSpacing, TextAlign, TextStyle};
    use aimer_widget::StatefulWidget;
    use aimer_widget::base::{Color, Colors};
    use aimer_widget::{AnyElement, Drawable, EventDispatcher, FocusNode, Key, Widget};

    use crate::input::{DefaultCaret, TextArea, TextEditingController};
    use crate::input_field::raw_fields::test_support::dummy_build_context;
    use crate::input_field::raw_fields::{ExpandDirection, InputType};

    #[test]
    fn default_text_area_is_unbounded_multiline_without_expansion() {
        let state = TextArea::new().create_state();
        let config = state.config();

        assert_eq!(config.input_type, InputType::Text);
        assert_eq!(config.min_lines, Some(3));
        assert_eq!(config.max_lines, None);
        assert_eq!(config.expand, ExpandDirection::None);
    }

    #[test]
    fn configured_maximum_is_clamped_to_minimum_when_state_is_created() {
        let state = TextArea::new()
            .min_lines(6)
            .max_lines(2)
            .expand(true)
            .create_state();
        let config = state.config();

        assert_eq!(config.min_lines, Some(6));
        assert_eq!(config.max_lines, Some(6));
        assert_eq!(config.expand, ExpandDirection::Vertical);
    }

    #[test]
    fn a_custom_caret_builder_reaches_the_created_state() {
        let state = TextArea::new()
            .caret(|context| DefaultCaret::new(context, Colors::default()))
            .create_state();

        assert!(state.config().caret_builder.is_some());
    }

    #[test]
    fn shared_field_builders_reach_the_created_state() {
        let controller = TextEditingController::with_text("first\nsecond");
        let area = TextArea::new()
            .controller(controller.clone())
            .focus_node(FocusNode::default())
            .prompt("Prompt")
            .hint("Hint")
            .hint_style(TextStyle::default())
            .text_style(TextStyle::default())
            .prompt_style(TextStyle::default())
            .text_align(TextAlign::default())
            .auto_focus(true)
            .max_length(Some(20))
            .enabled(false)
            .read_only(true)
            .decoration(BoxDecoration::default())
            .hover_decoration(BoxDecoration::default())
            .focus_decoration(BoxDecoration::default())
            .disabled_decoration(BoxDecoration::default())
            .selection_color(Color::Rgba(1, 2, 3, 4))
            .cursor_color(Colors::default())
            .on_changed(|_| {})
            .on_submitted(|_| {})
            .on_focus(|_| {})
            .on_blur(|_| {})
            .padding(LayoutSpacing::default())
            .key("notes");

        let key = Widget::key(&area);
        let state = area.create_state();
        let config = state.config();
        assert_eq!(config.controller.value().text(), "first\nsecond");
        assert_eq!(&*config.prompt, "Prompt");
        assert_eq!(&*config.hint, "Hint");
        assert!(config.auto_focus);
        assert_eq!(config.max_length, Some(20));
        assert!(!config.enable);
        assert!(config.read_only);
        assert_eq!(key, Some(Key::Value("notes".to_owned())));
    }

    /// Drags the mouse across a built text area, along its first line, from
    /// `from` to `to`, and reports the selection it leaves, in graphemes.
    fn drag_selection(text: &str, from: f32, to: f32) -> (usize, usize) {
        let controller = TextEditingController::with_text(text);
        let ctx = dummy_build_context(200.0, 80.0);
        let element: AnyElement = TextArea::new()
            .controller(controller.clone())
            .to_element(&ctx);
        element.update(&ctx);
        let mut dispatcher = EventDispatcher::new();
        let start = Vec2d { x: from, y: 12.0 };
        let end = Vec2d { x: to, y: 12.0 };

        assert!(
            dispatcher
                .dispatch(
                    element.as_ref(),
                    start,
                    &ElementEvent::PointerDown(PointerInfo::mouse(start, PointerButton::Primary)),
                )
                .is_consumed()
        );
        element.update(&ctx);
        assert!(
            dispatcher
                .dispatch(
                    element.as_ref(),
                    end,
                    &ElementEvent::PointerMove(PointerInfo::mouse(end, PointerButton::Primary)),
                )
                .is_consumed()
        );
        element.update(&ctx);
        assert!(
            dispatcher
                .dispatch(
                    element.as_ref(),
                    end,
                    &ElementEvent::PointerUp(PointerInfo::mouse(end, PointerButton::Primary)),
                )
                .is_consumed()
        );

        controller.selection_graphemes()
    }

    /// Dragging selects through the shared selection area.
    ///
    /// The area measures its text with the system's font, so where a given pixel
    /// falls among the characters differs from one platform to the next. The
    /// drags below mean the same thing in any font: from the very left edge to a
    /// point past the end of the text, and to a point inside it.
    #[test]
    fn a_built_text_area_uses_the_shared_selection_area_for_dragging() {
        const TEXT: &str = "hello world";
        let length = TEXT.chars().count();

        assert_eq!(drag_selection(TEXT, 1.0, 190.0), (0, length));

        let (start, end) = drag_selection(TEXT, 1.0, 30.0);
        assert_eq!(start, 0);
        assert!(
            0 < end && end < length,
            "a drag that ends inside the text selected {start}..{end} of {length}"
        );
    }
}
