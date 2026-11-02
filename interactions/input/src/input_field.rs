pub mod caret;
pub(crate) mod chrome;
pub mod context_menu;
#[cfg(test)]
mod controller {
    use crate::TextEditingController;

    /// A controller for managing and interacting with a text field's content.
    ///
    /// `TextFieldController` provides a mechanism to safely share
    /// and mutate a text field's state across different parts of an application.
    /// It provides interior mutability to modify the text even
    /// when the `TextFieldController` instance is immutable.
    ///
    /// Includes an undo/redo stack so every mutation can be reversed.
    ///
    /// # Offsets
    ///
    /// Every offset in this API counts **grapheme clusters** — what a reader calls
    /// a character — not `char`s and not bytes. A cluster can span several code
    /// points: `"👨‍👩‍👧"` is five `char`s joined by zero-width joiners and `"é"` may be
    /// `e` plus a combining accent, yet each is a single offset step. This is the
    /// same unit the field uses to measure and draw text, so the caret always
    /// lands where the edit does.
    ///
    /// ```rust
    /// use aimer_input::input::TextFieldController;
    ///
    /// let controller = TextFieldController::with_initial("👨‍👩‍👧b");
    /// assert_eq!(controller.grapheme_count(), 2);
    /// controller.insert_str("a", 1);
    /// assert_eq!(controller.text(), "👨‍👩‍👧ab");
    /// ```
    ///
    /// # Example
    /// ```rust
    /// use aimer_input::input::TextFieldController;
    ///
    /// let controller = TextFieldController::with_initial("Initial text");
    /// assert_eq!(controller.text(), "Initial text");
    /// ```
    pub(crate) struct TextFieldController {
        inner: TextEditingController,
    }

    impl Clone for TextFieldController {
        fn clone(&self) -> Self {
            Self {
                inner: self.inner.clone(),
            }
        }
    }

    impl From<TextFieldController> for TextEditingController {
        fn from(controller: TextFieldController) -> Self {
            controller.inner
        }
    }

    impl Default for TextFieldController {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TextFieldController {
        /// Creates a new instance of the TextFieldController with empty text.
        pub fn new() -> Self {
            Self {
                inner: TextEditingController::new(),
            }
        }

        /// Creates a new instance with the given initial text.
        pub fn with_initial(text: impl Into<String>) -> Self {
            Self {
                inner: TextEditingController::with_text(text),
            }
        }

        /// Returns a shared reference to the text stored within the current
        /// instance.
        pub fn text(&self) -> String {
            self.inner.text()
        }

        /// Consumes the content of the `text` field, returning its value while also
        /// clearing it.
        pub fn take(&self) -> String {
            self.inner.take()
        }

        /// Sets the text content of the object.
        pub fn set_text(&self, text: String) {
            self.inner.set_text(text);
        }

        /// Inserts a single character at the given grapheme offset.
        ///
        /// # Safety
        /// Be careful about the index out of bounds or invalid utf-8 char.
        pub unsafe fn insert_char(&self, ch: impl Into<char>, offset: usize) {
            unsafe { self.inner.insert_char(ch, offset) };
        }

        /// Deletes the grapheme cluster at `offset` and returns the removed text.
        ///
        /// A whole cluster goes at once, so one backspace removes an entire family
        /// emoji or an accented letter together with its combining mark instead of
        /// leaving a mangled remainder behind. An offset past the end removes
        /// nothing and returns an empty string.
        ///
        /// # Example
        /// ```rust
        /// use aimer_input::input::TextFieldController;
        ///
        /// let controller = TextFieldController::with_initial("a👨‍👩‍👧b");
        /// assert_eq!(controller.delete_grapheme(1), "👨‍👩‍👧");
        /// assert_eq!(controller.text(), "ab");
        /// ```
        pub fn delete_grapheme(&self, offset: usize) -> String {
            self.inner.delete_grapheme(offset)
        }

        /// Clears the internal text buffer.
        pub fn clear(&self) {
            self.inner.clear();
        }

        /// Returns the number of grapheme clusters in the text.
        ///
        /// This is the length of the text in the same unit its offsets use, so it
        /// doubles as the last valid cursor position.
        pub fn grapheme_count(&self) -> usize {
            self.inner.grapheme_count()
        }

        /// Returns the substring between two grapheme offsets.
        ///
        /// An inverted range yields an empty string rather than panicking.
        pub fn get_range(&self, start: usize, end: usize) -> String {
            self.inner.get_range(start, end)
        }

        /// Deletes the grapheme clusters in the range `[start, end)` and returns
        /// the removed text.
        pub fn delete_range(&self, start: usize, end: usize) -> String {
            self.inner.delete_range(start, end)
        }

        /// Inserts a string at the given grapheme offset.
        pub fn insert_str(&self, text: &str, offset: usize) {
            self.inner.insert_str(text, offset);
        }

        // ── Undo / Redo ──────────────────────────────────────────────────

        /// Revert to the previous text state. Returns `true` if an undo was
        /// performed.
        pub fn undo(&self) -> bool {
            self.inner.undo()
        }

        /// Re-apply a previously undone text state. Returns `true` if a redo was
        /// performed.
        pub fn redo(&self) -> bool {
            self.inner.redo()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_new_is_empty() {
            let c = TextFieldController::new();
            assert_eq!(c.text(), "");
            assert_eq!(c.grapheme_count(), 0);
        }

        #[test]
        fn test_with_initial() {
            let c = TextFieldController::with_initial("hello");
            assert_eq!(c.text(), "hello");
            assert_eq!(c.grapheme_count(), 5);
        }

        #[test]
        fn test_set_text() {
            let c = TextFieldController::new();
            c.set_text("world".to_string());
            assert_eq!(c.text(), "world");
        }

        #[test]
        fn test_insert_char_ascii() {
            let c = TextFieldController::with_initial("hello");
            unsafe {
                c.insert_char('!', 5);
            }
            assert_eq!(c.text(), "hello!");
        }

        #[test]
        fn test_insert_char_middle() {
            let c = TextFieldController::with_initial("hlo");
            unsafe {
                c.insert_char('e', 1);
            }
            assert_eq!(c.text(), "helo");
        }

        #[test]
        fn test_insert_char_unicode() {
            let c = TextFieldController::with_initial("helo");
            unsafe {
                c.insert_char('🌟', 2);
            }
            assert_eq!(c.text(), "he🌟lo");
        }

        #[test]
        fn test_delete_grapheme() {
            let c = TextFieldController::with_initial("hello");
            c.delete_grapheme(1); // remove 'e'
            assert_eq!(c.text(), "hllo");
        }

        #[test]
        fn test_grapheme_count_unicode() {
            let c = TextFieldController::with_initial("he🌟lo");
            assert_eq!(c.grapheme_count(), 5);
        }

        #[test]
        fn test_get_range() {
            let c = TextFieldController::with_initial("hello world");
            assert_eq!(c.get_range(0, 5), "hello");
            assert_eq!(c.get_range(6, 11), "world");
        }

        #[test]
        fn test_delete_range() {
            let c = TextFieldController::with_initial("hello world");
            let removed = c.delete_range(5, 11);
            assert_eq!(removed, " world");
            assert_eq!(c.text(), "hello");
        }

        #[test]
        fn test_insert_str() {
            let c = TextFieldController::with_initial("hlo");
            c.insert_str("el", 1);
            assert_eq!(c.text(), "hello");
        }

        #[test]
        fn test_insert_str_unicode() {
            let c = TextFieldController::with_initial("hlo");
            c.insert_str("é🌟", 1);
            assert_eq!(c.text(), "hé🌟lo");
        }

        #[test]
        fn test_take() {
            let c = TextFieldController::with_initial("hello");
            let taken = c.take();
            assert_eq!(taken, "hello");
            assert_eq!(c.text(), "");
        }

        #[test]
        fn test_clear() {
            let c = TextFieldController::with_initial("hello");
            c.clear();
            assert_eq!(c.text(), "");
        }

        #[test]
        fn test_clone_shares_state() {
            let c1 = TextFieldController::with_initial("shared");
            let c2 = c1.clone();
            c2.set_text("modified".to_string());
            assert_eq!(c1.text(), "modified");
        }

        #[test]
        fn test_undo_basic() {
            let c = TextFieldController::with_initial("hello");
            c.set_text("world".to_string());
            assert_eq!(c.text(), "world");
            assert!(c.undo());
            assert_eq!(c.text(), "hello");
        }

        #[test]
        fn test_undo_empty_stack() {
            let c = TextFieldController::new();
            assert!(!c.undo());
            assert_eq!(c.text(), "");
        }

        #[test]
        fn test_redo_basic() {
            let c = TextFieldController::with_initial("hello");
            c.set_text("world".to_string());
            assert!(c.undo());
            assert_eq!(c.text(), "hello");
            assert!(c.redo());
            assert_eq!(c.text(), "world");
        }

        #[test]
        fn test_redo_empty_stack() {
            let c = TextFieldController::new();
            assert!(!c.redo());
        }

        #[test]
        fn test_undo_insert_char() {
            let c = TextFieldController::with_initial("hl");
            unsafe {
                c.insert_char('e', 1);
            }
            assert_eq!(c.text(), "hel");
            assert!(c.undo());
            assert_eq!(c.text(), "hl");
        }

        #[test]
        fn test_undo_delete_range() {
            let c = TextFieldController::with_initial("hello world");
            c.delete_range(5, 11);
            assert_eq!(c.text(), "hello");
            assert!(c.undo());
            assert_eq!(c.text(), "hello world");
        }

        #[test]
        fn test_new_mutation_invalidates_redo() {
            let c = TextFieldController::with_initial("a");
            c.set_text("b".to_string());
            c.set_text("c".to_string());
            assert!(c.undo()); // back to "b"
            assert!(c.undo()); // back to "a"
            c.set_text("d".to_string()); // new edit — redo stack should clear
            assert!(!c.redo()); // nothing to redo
            assert_eq!(c.text(), "d");
        }

        // ── Grapheme offsets ────────────────────────────────────────────

        /// A single family emoji: three code points joined by zero-width joiners,
        /// five `char`s and eighteen bytes, but one grapheme cluster.
        const FAMILY: &str = "👨‍👩‍👧";

        /// `e` followed by a combining acute accent — two code points, one
        /// grapheme cluster.
        const COMBINING_E: &str = "e\u{301}";

        #[test]
        fn test_grapheme_count_counts_clusters_not_chars() {
            assert_eq!(
                TextFieldController::with_initial(FAMILY).grapheme_count(),
                1
            );
            assert_eq!(
                TextFieldController::with_initial(COMBINING_E).grapheme_count(),
                1
            );
            // Six code points, three extended clusters: the virama binds "स्ते"
            // together.
            assert_eq!(
                TextFieldController::with_initial("नमस्ते").grapheme_count(),
                3
            );
            assert_eq!(
                TextFieldController::with_initial(format!("{FAMILY}a")).grapheme_count(),
                2
            );
        }

        #[test]
        fn test_insert_str_at_grapheme_offset() {
            let c = TextFieldController::with_initial(format!("{FAMILY}b"));
            c.insert_str("a", 1);
            assert_eq!(c.text(), format!("{FAMILY}ab"));
        }

        #[test]
        fn test_insert_str_past_the_end_appends() {
            let c = TextFieldController::with_initial(FAMILY);
            c.insert_str("!", 99);
            assert_eq!(c.text(), format!("{FAMILY}!"));
        }

        #[test]
        fn test_insert_char_at_grapheme_offset() {
            let c = TextFieldController::with_initial(FAMILY);
            unsafe {
                c.insert_char('\n', 1);
            }
            assert_eq!(c.text(), format!("{FAMILY}\n"));
        }

        #[test]
        fn test_delete_grapheme_removes_whole_cluster() {
            let c = TextFieldController::with_initial(format!("a{FAMILY}b"));
            let removed = c.delete_grapheme(1);
            assert_eq!(removed, FAMILY);
            assert_eq!(c.text(), "ab");
        }

        #[test]
        fn test_delete_grapheme_removes_combining_mark_with_its_base() {
            let c = TextFieldController::with_initial(format!("{COMBINING_E}x"));
            c.delete_grapheme(0);
            assert_eq!(c.text(), "x");
        }

        #[test]
        fn test_delete_grapheme_out_of_bounds() {
            let c = TextFieldController::with_initial("hi");
            assert_eq!(c.delete_grapheme(99), "");
            assert_eq!(c.text(), "hi");
        }

        #[test]
        fn test_delete_range_uses_grapheme_offsets() {
            let c = TextFieldController::with_initial(format!("a{FAMILY}b"));
            let removed = c.delete_range(1, 2);
            assert_eq!(removed, FAMILY);
            assert_eq!(c.text(), "ab");
        }

        #[test]
        fn test_get_range_uses_grapheme_offsets() {
            let c = TextFieldController::with_initial(format!("{COMBINING_E}x"));
            assert_eq!(c.get_range(0, 1), COMBINING_E);
            assert_eq!(c.get_range(1, 2), "x");
        }

        #[test]
        fn test_get_range_clamps_an_inverted_range() {
            let c = TextFieldController::with_initial("hello");
            assert_eq!(c.get_range(4, 1), "");
        }

        #[test]
        fn test_undo_delete_grapheme() {
            let c = TextFieldController::with_initial(format!("a{FAMILY}"));
            c.delete_grapheme(1);
            assert_eq!(c.text(), "a");
            assert!(c.undo());
            assert_eq!(c.text(), format!("a{FAMILY}"));
        }

        #[test]
        fn test_undo_multiple_steps() {
            let c = TextFieldController::new();
            c.set_text("a".to_string());
            c.set_text("ab".to_string());
            c.set_text("abc".to_string());
            assert!(c.undo());
            assert_eq!(c.text(), "ab");
            assert!(c.undo());
            assert_eq!(c.text(), "a");
            assert!(c.undo());
            assert_eq!(c.text(), "");
            assert!(!c.undo()); // nothing left
        }
    }
}
pub mod raw_fields;

use std::cell::{Cell, RefCell};
use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;

use aimer_decoration::Decoration;
use aimer_macro::PortableWidget;
use aimer_style::{BoxDecoration, LayoutSpacing, TextAlign, TextStyle};
use aimer_venus::{ScopeId, TaskScope, Venus};
use aimer_widget::base::{BuildContext, Color, Colors};
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{PortableBuildContext, PortableBuildError, SourceFingerprint};
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, FocusBehavior, FocusNode, Focusable, Key, State,
    StateUpdater, StatefulElement, StatefulWidget, Widget,
};

use crate::TextEditingController;
use aimer_events::element::{Modifiers, NamedKey};
use crate::input_field::caret::{CaretBlink, CaretBuilder, CaretContext};
use crate::input_field::chrome::{Chrome, TextFieldVisualState, VisualBuilder, VisualFlags};
use crate::input_field::raw_fields::{
    CaretSlot, ExpandDirection, InputType, KeyFilter, RawFieldConfig, RawTextFieldWidget,
    TextFieldCallback,
};

#[allow(dead_code)]
///
/// A configurable `TextField` widget struct that provides input capabilities
/// with an array of customizable properties for text input, styling, behavior,
/// and event handling.
///
/// # Fields
///
/// * `controller` - The `TextEditingController` instance used to control the
///   field and observe immutable editing values.
///
/// * `input_type` - Specifies the type of input allowed (e.g., text, number,
///   password). Defaults to a default implementation of `InputType`.
///
/// * `prompt` - The text prompt displayed when the `TextField` is empty. This
///   field can be initialized using types that implement `Into<String>`.
///
/// * `hint` - Hint text displayed within the `TextField` to provide user
///   guidance. Can be initialized using types implementing `Into<String>`.
///
/// * `hint_style` - Styling applied to the hint text. Defaults to a `TextStyle`
///   implementation.
///
/// * `text_style` - Styling applied to the user-inputted text. Defaults to a
///   `TextStyle` implementation.
///
/// * `prompt_style` - Styling for the text prompt. Defaults to a `TextStyle`
///   implementation.
///
/// * `text_align` - The alignment of the text within the `TextField`. Defaults
///   to a default implementation of `TextAlign`.
///
/// * `auto_focus` - Boolean indicating if the field should be automatically
///   focused upon rendering. Defaults to `false`.
///
/// * `max_length` - An optional maximum number of characters allowed in the
///   input. Defaults to `None`.
///
/// * `enable` - Indicates whether the `TextField` is enabled for interaction.
///   Defaults to `true`.
///
/// * `decoration` - The default decoration applied to the `TextField`. Defaults
///   to `BoxDecoration`.
///
/// * `hover_decoration` - The decoration applied to the `TextField` when
///   hovered. Defaults to `None`.
///
/// * `focus_decoration` - The decoration applied to the `TextField` when it
///   gains focus. Defaults to `None`.
///
/// * `disabled_decoration` - The decoration applied to the `TextField` when it
///   is disabled. Defaults to `None`.
///
/// * `cursor_color` - Color of the builtin insertion cursor and IME underline.
///   It does not style a custom caret. Defaults to a default `Colors`
///   implementation.
///
/// * `caret` - A non-portable factory for a retained custom caret. The factory
///   receives live geometry, focus, composition, and blink state through
///   `CaretContext`.
///
/// * `on_changed` - Callback triggered when the input text changes. Accepts a
///   `TextFieldCallback` which is wrapped with an `AsyncTextFieldCallback`.
///
/// * `on_submitted` - Callback triggered when the user submits the input (e.g.,
///   pressing Enter). Accepts a `TextFieldCallback` which is wrapped with an
///   `AsyncTextFieldCallback`.
///
/// * `on_focus` - Callback triggered when the field gains focus. Accepts a
///   `TextFieldCallback` which is wrapped with an `AsyncTextFieldCallback`.
///
/// * `on_blur` - Callback triggered when the field loses focus. Accepts a
///   `TextFieldCallback` which is wrapped with an `AsyncTextFieldCallback`.
///
/// * `read_only` - When `true`, text cannot be modified via keyboard input.
///   Selection, copy, and cursor movement still work. Defaults to `false`.
///
/// Decorations are selected in disabled, focused, hovered, then normal
/// priority. The field starts empty and enabled with a white background, four
/// logical pixels of padding, and no line or length
/// limits. [`TextField::auto_focus`] controls the initial focus state when the
/// element is created.
///
/// # Example
///
/// ```
/// use aimer_input::{TextEditingController, input::{InputType, TextField}};
///
/// let controller = TextEditingController::with_text("hello");
/// let field = TextField::new().controller(controller)
///                             .input_type(InputType::Text)
///                             .hint("Message")
///                             .max_length(Some(200))
///                             .on_changed(|text| println!("changed to {text}"));
/// ```
#[derive(PortableWidget)]
#[portable_widget(
    id = "aimer_input::TextField",
    validate = validate_portable_text_field
)]
pub struct TextField {
    // Controllers, native text styles, and payload-bearing callbacks do not
    // have a portable wire contract yet. Keep them in the native state path
    // while deriving the bounded scalar and decoration properties below.
    #[portable_skip]
    controller: TextEditingController,
    #[portable_skip]
    pub input_type: InputType,
    #[portable_skip]
    pub prompt: Arc<str>,
    #[portable_skip]
    pub hint: Arc<str>,
    #[portable_skip]
    pub hint_style: TextStyle,
    #[portable_skip]
    pub text_style: TextStyle,
    #[portable_skip]
    pub prompt_style: TextStyle,
    #[portable_skip]
    pub text_align: TextAlign,
    #[portable_skip]
    focus_node: Option<FocusNode>,
    pub auto_focus: bool,
    #[portable_skip]
    pub max_length: Option<usize>,
    /// The fixed-width guest descriptor for [`Self::max_length`].
    ///
    /// `usize` is deliberately not put on the wire: its width differs between
    /// targets. The descriptor is kept in sync by the builder and is checked by
    /// the guest validator before lowering, so a direct mutation of the public
    /// native field cannot silently become an unbounded or stale value.
    max_length_wire: Option<u32>,
    pub enable: bool,
    pub decoration: BoxDecoration,
    pub hover_decoration: Option<BoxDecoration>,
    pub focus_decoration: Option<BoxDecoration>,
    pub disabled_decoration: Option<BoxDecoration>,
    /// Decorations that are not a [`BoxDecoration`], in normal, hover, focus,
    /// disabled order. A slot filled here takes precedence over the matching
    /// public field above.
    #[portable_skip]
    custom_decorations: [Option<Rc<dyn Decoration>>; 4],
    /// The caller-built frame; see [`TextField::builder`].
    #[portable_skip]
    visual_builder: Option<VisualBuilder>,
    pub selection_color: Color,
    #[portable_skip]
    pub cursor_color: Colors,
    #[portable_skip]
    pub caret_builder: Option<CaretBuilder>,
    #[portable_skip]
    pub on_changed: TextFieldCallback,
    #[portable_skip]
    pub on_submitted: TextFieldCallback,
    #[portable_skip]
    pub on_focus: TextFieldCallback,
    #[portable_skip]
    pub on_blur: TextFieldCallback,
    pub read_only: bool,
    #[portable_skip]
    key_filter: Option<KeyFilter>,
    pub padding: LayoutSpacing,
    #[portable_skip]
    widget_key: Option<Key>,
}

impl StatefulWidget for TextField {
    type State = TextFieldState;

    fn create_state(self) -> Self::State {
        self.create_state_with_config(self.config())
    }
}

impl Widget for TextField {
    fn key(&self) -> Option<Key> {
        self.widget_key.clone()
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let __key = Widget::key(&self);
        StatefulElement::new_with_name(self, ctx, "TextField", __key)
            .0
            .boxed()
    }
}

#[cfg(feature = "portable-guest")]
fn validate_portable_text_field(
    field: &TextField,
    _ctx: &PortableBuildContext,
    source: SourceFingerprint,
) -> Result<(), PortableBuildError> {
    let expected = match field.max_length {
        None => None,
        Some(value) => Some(
            u32::try_from(value)
                .map_err(|_| PortableBuildError::InvalidPropertyValue { rust_type: "usize" })?,
        ),
    };
    if field.max_length_wire != expected {
        return Err(PortableBuildError::UnsupportedProperty {
            widget: "TextField",
            property: "max_length",
            source,
        });
    }
    if let Some(property) = field.native_only_property() {
        return Err(PortableBuildError::UnsupportedProperty {
            widget: "TextField",
            property,
            source,
        });
    }
    Ok(())
}

/// The mounted state shared by [`TextField`] and [`TextArea`](crate::TextArea).
///
/// The state owns the caret blink timeline, so the caret keeps its rhythm when
/// the field is rebuilt with a new configuration — a rebuild replaces the
/// configuration through [`State::adopt_config_from`] and leaves the timeline
/// untouched. On native targets, the mounted state owns the scope of the delayed
/// wake that advances the timeline only at a visibility transition.
#[doc(hidden)]
pub struct TextFieldState {
    config: RawFieldConfig,
    caret: CaretBlink,
    /// Presentation state shared by every raw field candidate and its caret.
    caret_context: CaretContext,
    /// Retains the caret element while the raw field widget is rebuilt.
    caret_slot: Rc<CaretSlot>,
    /// Owns delayed native caret wakes for the mounted field.
    caret_scope: RefCell<Option<TaskScope>>,
    focus_node: FocusNode,
    provided_focus_node: bool,
    updater: StateUpdater<Self>,
    /// What the chrome around the field is built from, when it has chrome.
    visual: Rc<VisualFlags>,
}

impl TextFieldState {
    /// Returns the raw configuration for state-construction tests.
    #[cfg(test)]
    #[inline]
    pub(crate) fn config(&self) -> &RawFieldConfig {
        &self.config
    }

    /// Attaches the updater supplied by the stateful element.
    #[inline]
    pub(crate) fn init(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    /// Replaces widget configuration while retaining mounted editing state.
    #[inline]
    pub(crate) fn adopt(&mut self, new: Self) {
        self.config = new.config;
        self.config.visual = self.config.chrome.is_some().then(|| self.visual.clone());
        if new.provided_focus_node {
            self.focus_node = new.focus_node;
        }
        self.provided_focus_node = new.provided_focus_node;
    }

    /// Builds the raw field widget from the current shared state.
    ///
    /// Only [`TextFieldState::focusable_field`] mounts this: the raw element is
    /// never a focus target on its own, so nothing outside builds it bare.
    #[inline]
    fn raw_widget(&self) -> RawTextFieldWidget {
        RawTextFieldWidget::new(
            self.config.clone(),
            self.caret.clone(),
            self.caret_context.clone(),
            self.caret_slot.clone(),
            self.focus_node.clone(),
            self.caret_scope_id(),
        )
    }

    /// Lazily creates the scope after the state is mounted in the app runtime.
    #[inline]
    fn caret_scope_id(&self) -> Option<ScopeId> {
        let mut scope = self.caret_scope.borrow_mut();
        if scope.is_none() {
            if let Some(venus) = Venus::current() {
                scope.replace(venus.scope());
            }
        }
        scope.as_ref().map(TaskScope::id)
    }

    /// Reads how the field takes part in focus off its configuration.
    ///
    /// A disabled field is not a target at all — not by a press, not by `Tab`,
    /// and a press on it does not take focus from whoever holds it — which is
    /// [`FocusBehavior::Ignore`]. An enabled field asking to start focused is
    /// [`FocusBehavior::Auto`], and an ordinary enabled field is focused when it
    /// is pressed.
    ///
    /// Both inputs come from the widget configuration, so they only change when
    /// the field is rebuilt with a new one: a static behavior says all of this,
    /// and no [`Focusable::focusable_when`] gate is needed to re-ask per
    /// traversal.
    #[inline]
    fn focus_behavior(&self) -> FocusBehavior {
        match (self.config.enable, self.config.auto_focus) {
            (false, _) => FocusBehavior::Ignore,
            (true, true) => FocusBehavior::Auto,
            (true, false) => FocusBehavior::OnPress,
        }
    }

    /// Builds the field as a focus target of the standard focus mechanism.
    ///
    /// The state keeps owning the node and only lends it to the region, so the
    /// handle handed out by [`TextField::focus_node`] still drives this field:
    /// `request_focus()` and `unfocus()` on it move the keyboard exactly as they
    /// did while the field reported the node itself.
    ///
    /// The region wraps the raw field, which paints its own decoration and
    /// padding, so the area a press has to land in to focus the field is exactly
    /// the area the field covers.
    #[inline]
    pub(crate) fn focusable_field(&self) -> Focusable<RawTextFieldWidget> {
        Focusable::new()
            .node(self.focus_node.clone())
            .behavior(self.focus_behavior())
            .child(self.raw_widget())
    }

    /// Builds everything this field mounts: the focusable editing surface,
    /// inside whatever chrome the field was configured with.
    pub(crate) fn build_field(&self) -> AnyWidget {
        let Some(chrome) = &self.config.chrome else {
            return aimer_text::SelectionArea::without_focus()
                .selection_color(self.config.selection_color)
                .child(self.focusable_field())
                .boxed();
        };
        chrome.mount(
            self.bare_input(),
            self.visual.clone(),
            self.config.enable,
            self.config.read_only,
        )
    }

    /// The editing surface of a field with chrome: the same focusable field as
    /// [`TextFieldState::focusable_field`], but painting no decoration of its
    /// own, and rebuilt in place whenever the chrome around it is.
    fn bare_input(&self) -> ChildBuilder {
        let mut config = self.config.clone();
        config.decoration = BoxDecoration::default();
        config.hover_decoration = None;
        config.focus_decoration = None;
        config.disabled_decoration = None;
        let caret = self.caret.clone();
        let caret_context = self.caret_context.clone();
        let caret_slot = self.caret_slot.clone();
        let focus_node = self.focus_node.clone();
        let behavior = self.focus_behavior();
        let scope = self.caret_scope_id();
        ChildBuilder::new(move || {
            let raw = RawTextFieldWidget::new(
                config.clone(),
                caret.clone(),
                caret_context.clone(),
                caret_slot.clone(),
                focus_node.clone(),
                scope,
            );
            aimer_text::SelectionArea::without_focus()
                .selection_color(config.selection_color)
                .child(
                    Focusable::new()
                        .node(focus_node.clone())
                        .behavior(behavior)
                        .child(raw),
                )
                .boxed()
        })
    }

    /// Returns the caret blink timeline this field paints from.
    #[inline]
    pub fn caret(&self) -> &CaretBlink {
        &self.caret
    }
}

impl State<TextField> for TextFieldState {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.init(updater);
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.adopt(new);
    }

    fn build(&self, _: &BuildContext) -> impl Widget {
        self.build_field()
    }
}

impl TextField {
    /// Creates an empty, enabled, editable field with default styling and no-op
    /// callbacks.
    pub fn new() -> Self {
        Self {
            controller: TextEditingController::default(),
            input_type: InputType::default(),
            prompt: Arc::default(),
            hint: Arc::default(),
            hint_style: TextStyle::default(),
            text_style: TextStyle::default(),
            prompt_style: TextStyle::default(),
            text_align: TextAlign::default(),
            focus_node: None,
            auto_focus: false,
            max_length: None,
            max_length_wire: None,
            enable: true,
            decoration: BoxDecoration {
                background_color: Cell::new(Some(Colors::White.into())),
                ..Default::default()
            },
            hover_decoration: None,
            focus_decoration: None,
            disabled_decoration: None,
            custom_decorations: [None, None, None, None],
            visual_builder: None,
            selection_color: Color::Rgba(66, 133, 244, 100),
            cursor_color: Colors::default(),
            caret_builder: None,
            on_changed: TextFieldCallback::default(),
            on_submitted: TextFieldCallback::default(),
            on_focus: TextFieldCallback::default(),
            on_blur: TextFieldCallback::default(),
            read_only: false,
            key_filter: None,
            padding: LayoutSpacing::default(),
            widget_key: None,
        }
    }

    /// Sets the identity of this field for widget reconciliation.
    ///
    /// A keyed field keeps its state — and therefore its caret timeline, focus,
    /// and selection — when the surrounding widget list is reordered.
    #[inline]
    #[track_caller]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        let caller = Location::caller();
        self.widget_key = Some(key.into().with_location(caller));
        self
    }

    /// Collects the configuration handed to the element on every build.
    pub(crate) fn config(&self) -> RawFieldConfig {
        RawFieldConfig {
            input_type: self.input_type,
            controller: self.controller.clone(),
            prompt: self.prompt.clone(),
            hint: self.hint.clone(),
            hint_style: self.hint_style,
            text_style: self.text_style,
            prompt_style: self.prompt_style,
            text_align: self.text_align,
            auto_focus: self.auto_focus,
            max_lines: Some(1),
            min_lines: Some(1),
            max_length: self.max_length,
            enable: self.enable,
            expand: ExpandDirection::Horizontal,
            decoration: self.decoration.clone(),
            hover_decoration: self.hover_decoration.clone(),
            focus_decoration: self.focus_decoration.clone(),
            disabled_decoration: self.disabled_decoration.clone(),
            selection_color: self.selection_color,
            cursor_color: self.cursor_color,
            caret_builder: self.caret_builder.clone(),
            on_changed: self.on_changed.clone(),
            on_submitted: self.on_submitted.clone(),
            on_focus: self.on_focus.clone(),
            on_blur: self.on_blur.clone(),
            read_only: self.read_only,
            key_filter: self.key_filter.clone(),
            padding: self.padding,
            chrome: self.chrome(),
            visual: None,
        }
    }

    /// Names the first property set that has no portable wire form.
    #[cfg(feature = "portable-guest")]
    pub(crate) fn native_only_property(&self) -> Option<&'static str> {
        if self.visual_builder.is_some() {
            Some("builder")
        } else if self.custom_decorations.iter().any(Option::is_some) {
            Some("decoration")
        } else {
            None
        }
    }

    /// Resolves the frame placed around the editing surface, if any.
    ///
    /// A builder owns the whole appearance and ignores every decoration. Without
    /// one, a decoration that is not a [`BoxDecoration`] moves the decorations
    /// out of the field and around it, because only the field can paint a
    /// `BoxDecoration` together with its text.
    fn chrome(&self) -> Option<Chrome> {
        if let Some(builder) = &self.visual_builder {
            return Some(Chrome::from_builder(builder.clone()));
        }
        if self.custom_decorations.iter().all(Option::is_none) {
            return None;
        }
        let boxed = [
            Some(&self.decoration),
            self.hover_decoration.as_ref(),
            self.focus_decoration.as_ref(),
            self.disabled_decoration.as_ref(),
        ];
        Some(Chrome::from_decorations(std::array::from_fn(|slot| {
            self.custom_decorations[slot].clone().or_else(|| {
                boxed[slot].map(|decoration| Rc::new(decoration.clone()) as Rc<dyn Decoration>)
            })
        })))
    }

    /// Creates shared field state using `config` and this field's focus node.
    #[inline]
    pub(crate) fn create_state_with_config(&self, mut config: RawFieldConfig) -> TextFieldState {
        let caret = CaretBlink::new();
        let visual = Rc::new(VisualFlags::new(
            config.chrome.is_some() && config.controller.value().text().is_empty(),
        ));
        config.visual = config.chrome.is_some().then(|| visual.clone());
        TextFieldState {
            config,
            visual,
            caret_context: CaretContext::new(caret.clone()),
            caret_slot: Rc::new(CaretSlot::new()),
            caret,
            caret_scope: RefCell::new(None),
            focus_node: self.focus_node.clone().unwrap_or_default(),
            provided_focus_node: self.focus_node.is_some(),
            updater: StateUpdater::empty(),
        }
    }

    /// Uses `controller` as the field's shared text and selection-history
    /// owner.
    ///
    /// Clones of the controller observe the same text, undo stack, and redo
    /// stack.
    #[inline]
    pub fn controller(mut self, controller: TextEditingController) -> Self {
        self.controller = controller;
        self
    }

    /// Sets the accepted input mode, including plain text, numeric, and
    /// obscured password input.
    #[inline]
    pub fn input_type(mut self, input_type: InputType) -> Self {
        self.input_type = input_type;
        self
    }

    /// Sets the prompt drawn when the field is empty and focused.
    #[inline]
    pub fn prompt(mut self, prompt: impl Into<Arc<str>>) -> Self {
        self.prompt = prompt.into();
        self
    }

    /// Sets the hint drawn when the field is empty and not showing its prompt.
    #[inline]
    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        self.hint = hint.into();
        self
    }

    /// Replaces the style used to lay out and paint the hint.
    #[inline]
    pub fn hint_style(mut self, hint_style: TextStyle) -> Self {
        self.hint_style = hint_style;
        self
    }

    /// Replaces the style used to lay out and paint entered text.
    #[inline]
    pub fn text_style(mut self, text_style: TextStyle) -> Self {
        self.text_style = text_style;
        self
    }

    /// Replaces the style used to lay out and paint the focused empty prompt.
    #[inline]
    pub fn prompt_style(mut self, prompt_style: TextStyle) -> Self {
        self.prompt_style = prompt_style;
        self
    }

    /// Sets the alignment of text within the field's content area.
    #[inline]
    pub fn text_align(mut self, text_align: TextAlign) -> Self {
        self.text_align = text_align;
        self
    }

    /// Attaches the handle used for imperative focus control.
    ///
    /// Retain a clone of `focus_node` to request or release this field's focus
    /// after it is mounted. The node remains attached across rebuilds.
    #[inline]
    pub fn focus_node(mut self, focus_node: FocusNode) -> Self {
        self.focus_node = Some(focus_node);
        self
    }

    /// Sets whether a newly created field starts focused.
    ///
    /// This initializes focus when the widget becomes an element; it is not an
    /// imperative request to focus an already mounted field.
    #[inline]
    pub fn auto_focus(mut self, auto_focus: bool) -> Self {
        self.auto_focus = auto_focus;
        self
    }

    /// Sets the optional maximum input length in Unicode scalar values.
    ///
    /// `None` removes the limit. Input beyond the limit is not inserted.
    #[inline]
    pub fn max_length(mut self, max_length: Option<usize>) -> Self {
        self.max_length = max_length;
        self.max_length_wire = max_length.map(|value| u32::try_from(value).unwrap_or(u32::MAX));
        self
    }

    /// Applies the bounded guest descriptor while materializing a fresh field.
    #[doc(hidden)]
    #[inline]
    fn max_length_wire(mut self, max_length: u32) -> Self {
        self.max_length = Some(max_length as usize);
        self.max_length_wire = Some(max_length);
        self
    }

    /// Enables or disables focus, editing, selection, and input callbacks.
    ///
    /// A disabled field uses its configured disabled decoration when present.
    #[inline]
    pub fn enable(mut self, enable: bool) -> Self {
        self.enable = enable;
        self
    }

    /// Replaces the normal field decoration.
    ///
    /// Accepts any [`Decoration`], without requiring `Clone`. A
    /// [`BoxDecoration`] is painted by the field together with its text and
    /// keeps its border, outline and shadow geometry. Any other decoration is
    /// painted behind the field instead, and the field's own `padding` still
    /// insets the text inside it. Custom decorations require native rendering;
    /// portable lowering reports an unsupported property.
    ///
    /// A [`builder`](Self::builder) overrides every decoration.
    #[inline]
    pub fn decoration<D: Decoration + 'static>(mut self, decoration: D) -> Self {
        let (decoration, custom) = chrome::split(decoration);
        self.decoration = decoration.unwrap_or_default();
        self.custom_decorations[0] = custom;
        self
    }

    /// Sets the decoration used while an enabled, unfocused field is hovered.
    ///
    /// Accepts any [`Decoration`]; see [`decoration`](Self::decoration).
    #[inline]
    pub fn hover_decoration<D: Decoration + 'static>(mut self, hover_decoration: D) -> Self {
        let (decoration, custom) = chrome::split(hover_decoration);
        self.hover_decoration = decoration;
        self.custom_decorations[1] = custom;
        self
    }

    /// Sets the decoration used while the enabled field is focused.
    ///
    /// Focus decoration takes precedence over hover decoration. Accepts any
    /// [`Decoration`]; see [`decoration`](Self::decoration).
    #[inline]
    pub fn focus_decoration<D: Decoration + 'static>(mut self, focus_decoration: D) -> Self {
        let (decoration, custom) = chrome::split(focus_decoration);
        self.focus_decoration = decoration;
        self.custom_decorations[2] = custom;
        self
    }

    /// Sets the decoration used while the field is disabled.
    ///
    /// Disabled decoration takes precedence over focus and hover decorations.
    /// Accepts any [`Decoration`]; see [`decoration`](Self::decoration).
    #[inline]
    pub fn disabled_decoration<D: Decoration + 'static>(mut self, disabled_decoration: D) -> Self {
        let (decoration, custom) = chrome::split(disabled_decoration);
        self.disabled_decoration = decoration;
        self.custom_decorations[3] = custom;
        self
    }

    /// Builds the frame around the editing surface from the field's state.
    ///
    /// `build` receives a [`TextFieldVisualState`] and returns the whole
    /// appearance of the field: a label, icons, a counter, an error line, a
    /// custom border. It places the live editing surface with
    /// [`TextFieldVisualState::input`], which is the text, caret, selection and
    /// hint, and nothing else.
    ///
    /// **A builder owns the appearance, so every decoration set on this field
    /// is ignored**: `decoration`, `hover_decoration`, `focus_decoration` and
    /// `disabled_decoration` do nothing once a builder is set, and the editing
    /// surface paints no background, border or shadow of its own. `padding`,
    /// `text_style`, `controller`, callbacks, focus and every other setting
    /// still apply.
    ///
    /// The closure runs when the field is built and again whenever hover,
    /// focus or emptiness changes; only the returned subtree is rebuilt, and
    /// the editing surface inside it keeps its selection, composition and
    /// scroll. Keep [`input`](TextFieldVisualState::input) at the same position
    /// in the tree from one state to the next — wrapping it differently for
    /// focused and unfocused states would mount it again. Only the editing
    /// surface takes focus on a press; give the chrome a
    /// [`focus_node`](Self::focus_node) handle if a press elsewhere in it should
    /// focus the field. A builder requires native rendering; portable lowering
    /// reports an unsupported property.
    ///
    /// ```
    /// use aimer_input::input::TextField;
    /// use aimer_style::{BoxDecoration, LayoutSpacing};
    /// use aimer_container::Container;
    /// use aimer_widget::Widget;
    ///
    /// let field = TextField::new().builder(|state| {
    ///     let border = if state.is_focused { 2.0 } else { 1.0 };
    ///     Container::new()
    ///         .padding(LayoutSpacing::all(border as u32))
    ///         .box_child(state.input())
    /// });
    /// ```
    #[inline]
    pub fn builder<W: Widget + 'static>(
        mut self,
        build: impl Fn(TextFieldVisualState) -> W + 'static,
    ) -> Self {
        self.visual_builder = Some(Rc::new(move |state| build(state).boxed()));
        self
    }

    /// Sets the color painted behind selected text.
    #[inline]
    pub fn selection_color(mut self, selection_color: impl Into<Color>) -> Self {
        self.selection_color = selection_color.into();
        self
    }

    /// Sets the color of the builtin insertion cursor.
    ///
    /// This has no effect on a custom caret builder. Custom carets receive the
    /// live [`CaretContext`] and choose their own visual style.
    #[inline]
    pub fn cursor_color(mut self, cursor_color: Colors) -> Self {
        self.cursor_color = cursor_color;
        self
    }

    /// Replaces the builtin insertion cursor with a custom caret widget.
    ///
    /// The builder runs once when the field's visual caret element is mounted.
    /// The returned widget is retained, while its [`CaretContext`] continues to
    /// report live geometry and focus state as the field changes.
    #[inline]
    pub fn caret<F, W>(mut self, builder: F) -> Self
    where
        F: Fn(CaretContext) -> W + 'static,
        W: Widget + 'static,
    {
        self.caret_builder = Some(CaretBuilder::new(builder));
        self
    }

    /// Restores the builtin insertion cursor after a custom builder was set.
    #[inline]
    pub fn builtin_caret(mut self) -> Self {
        self.caret_builder = None;
        self
    }

    /// Sets the callback invoked after a user edit changes the text.
    ///
    /// The callback receives the complete updated string. Programmatic
    /// controller mutations do not themselves dispatch widget callbacks.
    #[inline]
    pub fn on_changed(mut self, on_changed: impl Into<TextFieldCallback>) -> Self {
        self.on_changed = on_changed.into();
        self
    }

    /// Sets the callback invoked when the user submits the field.
    ///
    /// The callback receives the current complete string.
    #[inline]
    pub fn on_submitted(mut self, on_submitted: impl Into<TextFieldCallback>) -> Self {
        self.on_submitted = on_submitted.into();
        self
    }

    /// Sets the callback invoked when the field gains focus.
    ///
    /// The callback receives the current complete string.
    #[inline]
    pub fn on_focus(mut self, on_focus: impl Into<TextFieldCallback>) -> Self {
        self.on_focus = on_focus.into();
        self
    }

    /// Sets the callback invoked when the field loses focus.
    ///
    /// The callback receives the current complete string.
    #[inline]
    pub fn on_blur(mut self, on_blur: impl Into<TextFieldCallback>) -> Self {
        self.on_blur = on_blur.into();
        self
    }

    /// Sets whether user editing is blocked while focus, selection, copy, and
    /// navigation remain.
    #[inline]
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Offers every key press to `filter` before the field acts on it.
    ///
    /// The filter runs only while the field is focused, for presses and
    /// repeats but not releases. Returning `true` consumes the key: the field
    /// does nothing with it and the event goes no further. Returning `false`
    /// leaves the field's own handling untouched, so a filter that claims only
    /// a few keys changes nothing else.
    ///
    /// A field consumes the arrow keys, `Home`, `End`, and `Escape` itself, so
    /// nothing outside it can react to them; this is how a list that is driven
    /// while typing — a combobox, a command palette — takes them first.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_events::element::NamedKey;
    /// use aimer_input::input::TextField;
    ///
    /// // Claim ArrowDown for a suggestion list; the caret still moves on the rest.
    /// let field = TextField::new().on_key(|key, _modifiers| matches!(key, NamedKey::ArrowDown));
    /// ```
    #[inline]
    pub fn on_key(mut self, filter: impl Fn(&NamedKey, &Modifiers) -> bool + 'static) -> Self {
        self.key_filter = Some(Rc::new(filter));
        self
    }

    /// Sets the spacing between the field decoration and its text content.
    #[inline]
    pub fn padding(mut self, padding: impl Into<LayoutSpacing>) -> Self {
        self.padding = padding.into();
        self
    }
}

impl Default for TextField {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use aimer_animation::AnimInstant;

    use super::*;
    use crate::input::DefaultCaret;
    use crate::input_field::raw_fields::test_support::dummy_build_context;

    const HALF: Duration = Duration::from_millis(500);

    #[test]
    fn explicit_key_sets_reconciliation_identity() {
        let field = TextField::new().key("login-field");

        assert_eq!(
            Widget::key(&field),
            Some(Key::Value("login-field".to_owned()))
        );
    }

    #[test]
    fn created_state_carries_the_widget_configuration() {
        let controller = TextEditingController::with_text("hello");
        let state = TextField::new()
            .controller(controller.clone())
            .hint("Message")
            .max_length(Some(7))
            .read_only(true)
            .create_state();

        assert_eq!(&*state.config.hint, "Message");
        assert_eq!(state.config.max_length, Some(7));
        assert!(state.config.read_only);
        assert_eq!(state.config.controller.value().text(), "hello");
    }

    #[test]
    fn text_field_is_always_single_line_and_number_is_only_an_input_hint() {
        let state = TextField::new()
            .input_type(InputType::Number)
            .create_state();

        assert_eq!(state.config.input_type, InputType::Number);
        assert_eq!(state.config.min_lines, Some(1));
        assert_eq!(state.config.max_lines, Some(1));
        assert_eq!(state.config.expand, ExpandDirection::Horizontal);
    }

    #[test]
    fn created_state_starts_with_a_visible_caret() {
        let state = TextField::new().create_state();

        assert!(state.caret().is_visible());
        assert_eq!(state.caret().period(), CaretBlink::DEFAULT_PERIOD);
    }

    #[test]
    fn a_custom_caret_builder_is_retained_in_field_configuration() {
        let state = TextField::new()
            .caret(|context| DefaultCaret::new(context, Colors::default()))
            .create_state();

        assert!(state.config.caret_builder.is_some());
    }

    #[test]
    fn builtin_caret_restores_the_default_configuration() {
        let state = TextField::new()
            .caret(|context| DefaultCaret::new(context, Colors::default()))
            .builtin_caret()
            .create_state();

        assert!(state.config.caret_builder.is_none());
    }

    #[test]
    fn custom_caret_builder_mounts_one_retained_widget() {
        let builds = Rc::new(Cell::new(0));
        let seen = builds.clone();
        let state = TextField::new()
            .caret(move |context| {
                seen.set(seen.get() + 1);
                DefaultCaret::new(context, Colors::default())
            })
            .create_state();
        let context = dummy_build_context(200.0, 40.0);

        let _first = Widget::to_element(state.focusable_field(), &context);
        let _second = Widget::to_element(state.focusable_field(), &context);

        assert_eq!(builds.get(), 1);
    }

    #[test]
    fn adopting_a_configuration_keeps_the_caret_phase() {
        let mut state = TextField::new().hint("before").create_state();
        let start = AnimInstant::now();
        state.caret().tick(start);
        state.caret().tick(start + HALF);
        assert!(!state.caret().is_visible());

        let rebuilt = TextField::new().hint("after").create_state();
        State::<TextField>::adopt_config_from(&mut state, rebuilt);

        assert_eq!(&*state.config.hint, "after");
        assert!(!state.caret().is_visible());
    }

    #[test]
    fn size_of_text_field() {
        eprintln!("Size Of TextField: {}", size_of::<TextField>())
    }

    #[test]
    fn adopting_a_configuration_does_not_restart_the_blink() {
        let mut state = TextField::new().create_state();
        let start = AnimInstant::now();
        state.caret().tick(start);

        State::<TextField>::adopt_config_from(&mut state, TextField::new().create_state());

        assert!(state.caret().tick(start + HALF));
        assert!(!state.caret().is_visible());
    }
}

#[cfg(test)]
mod focus_tests {
    //! What focus has to do for a field, asserted on the whole widget.
    //!
    //! A field is focused by a press, focused and blurred by code through the
    //! node an application supplies, and claims the keyboard on arrival when it
    //! autofocuses; while it owns focus, typed text and committed phrases reach
    //! its controller. None of that depends on *which* element of the field
    //! offers the focus node, so these tests drive the built widget the way the
    //! framework does — build the element, let the dispatcher resolve focus,
    //! read the controller — rather than poking the raw element, and they hold
    //! whether the offer is made by the field itself or by an enclosing
    //! [`Focusable`] region.
    //!
    //! [`Focusable`]: aimer_widget::Focusable

    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_attribute::position::Vec2d;
    use aimer_events::element::{ElementEvent, KeyAction, Modifiers};
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_style::LayoutSpacing;
    use aimer_widget::{
        AnyElement, Drawable, Element, EventDispatcher, FocusNode, Focusable, Rebuildable,
        StatefulElement, VisitorElement,
    };

    use super::*;
    use crate::input::DefaultCaret;
    use crate::input_field::raw_fields::test_support::dummy_build_context;

    /// The extent the field is built and drawn at.
    const FIELD_WIDTH: f32 = 200.0;
    const FIELD_HEIGHT: f32 = 40.0;

    /// A point the drawn field covers.
    const INSIDE: Vec2d = Vec2d { x: 20.0, y: 12.0 };

    /// Builds `field` and draws it once, so the element has the bounds a press
    /// is resolved against.
    ///
    /// Nothing reaches a GPU; the draw exists because the field caches its
    /// hit-test rectangle while painting, which is where a real frame puts it.
    fn drawn(field: TextField) -> (AnyElement, BuildContext<'static>) {
        let ctx = dummy_build_context(FIELD_WIDTH, FIELD_HEIGHT);
        let element = field.to_element(&ctx);
        element.update(&ctx);
        (element, ctx)
    }

    /// Presses at `pos`, the way a mouse would.
    fn press(dispatcher: &mut EventDispatcher, root: &AnyElement, pos: Vec2d) {
        let _ = dispatcher.dispatch(
            root.as_ref(),
            pos,
            &ElementEvent::PointerDown(PointerInfo::mouse(pos, PointerButton::Primary)),
        );
    }

    /// Types `text` the way a platform input method commits a phrase.
    fn commit(dispatcher: &mut EventDispatcher, root: &AnyElement, text: &str) {
        let _ = dispatcher.dispatch(
            root.as_ref(),
            INSIDE,
            &ElementEvent::TextInput {
                text: text.to_owned(),
                action: KeyAction::Pressed,
                modifiers: Modifiers::default(),
            },
        );
    }

    /// Pumps one frame of focus resolution without touching the field.
    ///
    /// Focus is settled while an event is dispatched, so a node that asked for
    /// focus — or a region that autofocuses — only becomes the owner once
    /// something is delivered.
    fn settle(dispatcher: &mut EventDispatcher, root: &AnyElement) {
        let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
    }

    /// Names every element of `root` that offers itself as a focus target.
    fn focus_targets(root: &AnyElement) -> Vec<&'static str> {
        fn walk(element: &dyn Element, names: &mut Vec<&'static str>) {
            if element.focus_node().is_some() {
                names.push(VisitorElement::debug_name(element));
            }
            element.focus_children(&mut |child| walk(child, names));
        }

        let mut names = Vec::new();
        walk(root.as_ref(), &mut names);
        names
    }

    /// A field is one focus target, offered by one element of it.
    ///
    /// The same node offered at two depths would be gathered twice in a single
    /// traversal, and every rule that reads tree order — `Tab` above all — would
    /// then have to guess which of the two it meant. The element that makes the
    /// offer is named here so that moving the offer stays a deliberate change:
    /// wrapping the field in [`Focusable`] turns this into `["Focusable"]`, and
    /// the field itself must stop offering the node in the same breath.
    ///
    /// [`Focusable`]: aimer_widget::Focusable
    #[test]
    fn the_field_is_offered_exactly_once_as_a_focus_target() {
        let (element, _ctx) = drawn(TextField::new());

        assert_eq!(focus_targets(&element), ["Focusable"]);
    }

    /// A press on the field focuses it, and what is typed afterwards reaches
    /// the controller.
    ///
    /// This is the pair a field lives by, and the press half is the fragile
    /// one: the field consumes the press that lands on it, so whatever offers
    /// its focus node has to be reached by that same press.
    #[test]
    fn a_press_focuses_the_field_and_typed_text_reaches_its_controller() {
        let controller = TextEditingController::new();
        let node = FocusNode::new();
        let (element, _ctx) = drawn(
            TextField::new()
                .controller(controller.clone())
                .focus_node(node.clone()),
        );
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &element, INSIDE);
        assert!(
            node.has_focus(),
            "a press must focus the field it landed on"
        );

        commit(&mut dispatcher, &element, "你好");

        assert_eq!(controller.text(), "你好");
    }

    #[test]
    fn public_caret_context_follows_focus_and_layout() {
        let node = FocusNode::new();
        let context = Rc::new(std::cell::RefCell::new(None));
        let captured = Rc::clone(&context);
        let (element, build_context) = drawn(
            TextField::new()
                .padding(LayoutSpacing::all(4))
                .focus_node(node.clone())
                .caret(move |caret_context| {
                    *captured.borrow_mut() = Some(caret_context.clone());
                    DefaultCaret::new(caret_context, Colors::default())
                }),
        );
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &element, INSIDE);
        element.update(&build_context);

        let context = context
            .borrow()
            .as_ref()
            .expect("the caret builder must receive a context")
            .clone();
        assert!(context.is_focused());
        assert!(context.is_available());
        assert_eq!(context.offset(), 0);
        assert_eq!(context.geometry().x, 4.0);
        assert!(context.geometry().height > 0.0);
    }

    #[test]
    fn appending_text_before_a_rebuild_keeps_the_caret_at_the_new_end() {
        let controller = TextEditingController::with_text("prefix");
        let node = FocusNode::new();
        let append_position = Vec2d { x: 190.0, y: 12.0 };
        let context = Rc::new(std::cell::RefCell::new(None));
        let captured = Rc::clone(&context);
        let widget = TextField::new()
            .controller(controller.clone())
            .focus_node(node.clone())
            .caret(move |caret_context| {
                *captured.borrow_mut() = Some(caret_context.clone());
                DefaultCaret::new(caret_context, Colors::default())
            });
        let build_context = dummy_build_context(FIELD_WIDTH, FIELD_HEIGHT);
        let (element, updater) =
            StatefulElement::new_with_name(widget, &build_context, "TextField", None);
        let element = element.boxed();
        element.update(&build_context);
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &element, append_position);
        let _ = dispatcher.dispatch(
            element.as_ref(),
            append_position,
            &ElementEvent::TextInput {
                text: "S".to_owned(),
                action: KeyAction::Pressed,
                modifiers: Modifiers::default(),
            },
        );
        updater.set_state(|_| {});
        element.rebuild_if_dirty(&build_context);
        element.update(&build_context);

        let context = context
            .borrow()
            .as_ref()
            .expect("the caret builder must receive a context")
            .clone();
        assert_eq!(controller.selection_graphemes(), (7, 7));
        assert_eq!(context.offset(), 7);
    }

    #[test]
    fn custom_caret_subtrees_are_not_focus_targets() {
        let field_node = FocusNode::new();
        let caret_node = FocusNode::new();
        let (element, _context) = drawn(TextField::new().focus_node(field_node).caret(
            move |context| {
                Focusable::new()
                    .node(caret_node.clone())
                    .child(DefaultCaret::new(context, Colors::default()))
            },
        ));

        assert_eq!(focus_targets(&element), ["Focusable"]);
    }

    /// Pressing a field that already holds focus changes nothing about its
    /// focus.
    ///
    /// Every click inside a field is a caret placement, and there are many of
    /// them in a row while text is being edited. If such a click blurred the
    /// field before focusing it again, each one would report a focus edge that
    /// did not happen — `on_blur` then `on_focus` — and, worse, the blur tears
    /// down the platform input method mid-composition, so a phrase being
    /// composed would be dropped by a click meant to move the caret. So the
    /// second press must leave the counts where the first one put them.
    #[test]
    fn pressing_a_field_that_already_holds_focus_reports_no_further_edge() {
        let node = FocusNode::new();
        let focuses = Rc::new(Cell::new(0));
        let blurs = Rc::new(Cell::new(0));
        let (element, _ctx) = drawn(
            TextField::new()
                .focus_node(node.clone())
                .on_focus({
                    let focuses = Rc::clone(&focuses);
                    move |_: String| focuses.set(focuses.get() + 1)
                })
                .on_blur({
                    let blurs = Rc::clone(&blurs);
                    move |_: String| blurs.set(blurs.get() + 1)
                }),
        );
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &element, INSIDE);
        assert_eq!(
            (focuses.get(), blurs.get()),
            (1, 0),
            "the first press focuses"
        );

        press(&mut dispatcher, &element, INSIDE);
        press(&mut dispatcher, &element, INSIDE);

        assert!(node.has_focus(), "the field keeps the focus it was given");
        assert_eq!((focuses.get(), blurs.get()), (1, 0));
    }

    /// A disabled field is not a target at all: pressing it neither focuses it
    /// nor takes focus from whoever holds it.
    #[test]
    fn a_disabled_field_is_not_a_focus_target() {
        let node = FocusNode::new();
        let (element, _ctx) = drawn(TextField::new().enable(false).focus_node(node.clone()));
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &element, INSIDE);

        assert!(!node.has_focus());
        assert!(focus_targets(&element).is_empty());
    }

    /// An autofocusing field claims the keyboard as soon as it is in the tree,
    /// without anybody pointing at it.
    #[test]
    fn an_auto_focus_field_claims_focus_when_it_appears() {
        let node = FocusNode::new();
        let (element, _ctx) = drawn(TextField::new().auto_focus(true).focus_node(node.clone()));
        let mut dispatcher = EventDispatcher::new();

        settle(&mut dispatcher, &element);

        assert!(node.has_focus());
    }

    /// The node stays the application's to drive: [`TextField::focus_node`]
    /// hands out a handle, and focusing or blurring it moves the keyboard
    /// without anything being pressed.
    ///
    /// Each edge is also reported exactly once. The callbacks fire on the focus
    /// notifications the field is delivered, so a notification repeated while
    /// nothing changed — a frame that resolves focus again, an offer made twice
    /// — would call them again.
    #[test]
    fn a_supplied_node_focuses_and_blurs_the_field_and_reports_each_edge_once() {
        let controller = TextEditingController::new();
        let node = FocusNode::new();
        let focuses = Rc::new(Cell::new(0));
        let blurs = Rc::new(Cell::new(0));
        let (element, _ctx) = drawn(
            TextField::new()
                .controller(controller.clone())
                .focus_node(node.clone())
                .on_focus({
                    let focuses = Rc::clone(&focuses);
                    move |_: String| focuses.set(focuses.get() + 1)
                })
                .on_blur({
                    let blurs = Rc::clone(&blurs);
                    move |_: String| blurs.set(blurs.get() + 1)
                }),
        );
        let mut dispatcher = EventDispatcher::new();

        node.request_focus();
        settle(&mut dispatcher, &element);
        settle(&mut dispatcher, &element);
        commit(&mut dispatcher, &element, "hi");
        node.unfocus();
        settle(&mut dispatcher, &element);
        settle(&mut dispatcher, &element);

        assert_eq!(controller.text(), "hi");
        assert_eq!((focuses.get(), blurs.get()), (1, 1));
        assert!(!node.has_focus());
    }

    /// Drags the mouse across a built field from `from` to `to` and reports the
    /// selection it leaves, in graphemes.
    fn drag_selection(text: &str, from: f32, to: f32) -> (usize, usize) {
        let controller = TextEditingController::with_text(text);
        let (element, ctx) = drawn(TextField::new().controller(controller.clone()));
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
        element.update(&ctx);
        controller.selection_graphemes()
    }

    /// Dragging selects through the field's shared selection area.
    ///
    /// The field measures its text with the system's font, so where a given
    /// pixel falls among the characters differs from one platform to the next.
    /// The drags below are therefore chosen to mean the same thing in any font:
    /// from the very left edge, which is before the first character, to a point
    /// past the end of the text, and to a point inside it.
    #[test]
    fn a_built_text_field_uses_the_shared_selection_area_for_dragging() {
        const TEXT: &str = "hello world";
        let length = TEXT.chars().count();

        // Everything: the whole text lies between the two ends of the drag.
        assert_eq!(
            drag_selection(TEXT, 1.0, FIELD_WIDTH - 10.0),
            (0, length)
        );

        // Part of it: the drag stops inside the text, so the selection starts
        // at the beginning, is not empty, and does not reach the end.
        let (start, end) = drag_selection(TEXT, 1.0, 30.0);
        assert_eq!(start, 0);
        assert!(
            0 < end && end < length,
            "a drag that ends inside the text selected {start}..{end} of {length}"
        );
    }
}
