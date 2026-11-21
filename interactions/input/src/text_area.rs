use std::panic::Location;
use std::rc::Rc;
use std::sync::Arc;

use aimer_decoration::Decoration;
use aimer_style::{LayoutSpacing, TextAlign, TextStyle};
use aimer_widget::base::{BuildContext, Color, Colors};
use aimer_widget::{
    AnyElement, FocusNode, Key, State, StateView, StateUpdater, StatefulElement, Widget,
};

use crate::TextEditingController;
use crate::input_field::caret::CaretContext;
use crate::input_field::chrome::TextFieldVisualState;
use crate::input_field::gutter::{GutterMarkers, LineBars, LineNumbers};
use crate::input_field::highlight::TextHighlighter;
use crate::input_field::raw_fields::{CommandClick, ExpandDirection, ForceClick, LinkRange, InputType, PointerHoles, TextFieldCallback};
use crate::input_field::{TextField, TextFieldState};

/// A multiline text input widget.
///
/// A text area starts with room for three lines, grows without a line limit,
/// and keeps the shared text-field editing, focus, decoration, and callback
/// behavior. Pressing Return inserts a line break; submission remains available
/// through the platform's modified Return action.
pub struct TextArea {
    // The embedded field owns the controller, focus, styling, and callback
    // state. Only the independently representable expansion policy is lowered.
    field: TextField,
    min_lines: usize,
    max_lines: Option<usize>,
    expand: bool,
    tab_spaces: Option<usize>,
    wrap: bool,
    clip_to_padding: bool,
    line_numbers: Option<LineNumbers>,
    gutter_markers: Option<GutterMarkers>,
    pointer_holes: Option<PointerHoles>,
    on_force_click: Option<ForceClick>,
    on_force_click_at: Option<crate::input_field::raw_fields::ForceClickAt>,
    on_command_click: Option<CommandClick>,
    on_command_click_at: Option<crate::input_field::raw_fields::CommandClickAt>,
    context_menu_extras: Option<crate::input_field::context_menu::ContextMenuExtender>,
    context_menu_style: Option<aimer_ctxmenu::ContextMenuStyle>,
    link_range: Option<LinkRange>,
    locate_color: Option<Color>,
    line_bars: Option<LineBars>,
    line_labels: Option<crate::input_field::line_labels::LineLabels>,
    inlays: Option<crate::input_field::inlay::Inlays>,
    underlines: Option<crate::input_field::underline::Underlines>,
    line_break_prefix: Option<Rc<dyn Fn(&str) -> Option<String>>>,
    current_line: Option<Color>,
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
            expand: false,
            tab_spaces: None,
            wrap: true,
            clip_to_padding: true,
            line_numbers: None,
            gutter_markers: None,
            pointer_holes: None,
            on_force_click: None,
            on_force_click_at: None,
            on_command_click: None,
            on_command_click_at: None,
            context_menu_extras: None,
            context_menu_style: None,
            link_range: None,
            locate_color: None,
            line_bars: None,
            line_labels: None,
            inlays: None,
            underlines: None,
            line_break_prefix: None,
            current_line: None,
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
        self.max_lines = max_lines;
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

    /// Makes the Tab key insert `spaces` spaces at the caret, in place of a selection, instead of
    /// moving focus to the next field.
    ///
    /// `None`, the default, leaves Tab to focus traversal. Shift+Tab is never taken, so a
    /// text area that indents with Tab can still be left from the keyboard. Does nothing for
    /// a read-only area.
    #[inline]
    pub fn tab_spaces(mut self, spaces: impl Into<Option<usize>>) -> Self {
        self.tab_spaces = spaces.into();
        self
    }

    /// Chooses whether the text is cut off at the padding of the area, above and below. The default is `true`: the padding is a band
    /// that the text does not enter.
    ///
    /// With `false` the padding above and below is a margin of the text and not a band beside it: the first line starts below
    /// the top padding and the last ends above the bottom one, as before, but a text that is scrolled shows through them, and
    /// is cut only at the edge of the area. That is what a code editor wants of the room it leaves round the text, which is not to
    /// take a strip of the view from it. The sides are not changed: a margin of line numbers is beside the text, and text that is
    /// scrolled sideways does not go over it.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextArea;
    ///
    /// let editor = TextArea::new().clip_to_padding(false);
    /// ```
    #[inline]
    pub fn clip_to_padding(mut self, clip: bool) -> Self {
        self.clip_to_padding = clip;
        self
    }

    /// Chooses whether long lines wrap to the width of the area. The default is `true`.
    ///
    /// An area that does not wrap shows each line on one row, however long, and scrolls sideways
    /// to keep the caret in view, which is what a code editor does. The horizontal scroll follows
    /// the caret, the trackpad and shift with the wheel; the lines are not cut or abbreviated.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextArea;
    ///
    /// let code = TextArea::new().wrap(false);
    /// # let _ = code;
    /// ```
    #[inline]
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Shows line numbers in a margin to the left of the text, or hides them with `None`.
    ///
    /// The margin takes room from the text, as wide as the largest number needs, and the number of the
    /// line the caret is on stands out. Clicking in it puts the caret at the start of the line.
    #[inline]
    pub fn line_numbers(mut self, numbers: impl Into<Option<LineNumbers>>) -> Self {
        self.line_numbers = numbers.into();
        self
    }

    /// Sets what is called when the text is force-clicked: the trackpad is pressed down hard on it, which a macOS Force
    /// Touch trackpad reports as a deep stage. It is called once for each deep press, with the byte of the text under the
    /// pointer, a frame after the press, when the layout can say which it is. A press on something drawn over the text,
    /// such as the menu of a caret widget or an icon in the margin, is not a press on the text.
    #[inline]
    pub fn on_force_click(mut self, on_force_click: impl Fn(usize) + 'static) -> Self {
        self.on_force_click = Some(std::rc::Rc::new(on_force_click));
        self
    }

    /// Like [`on_force_click`](Self::on_force_click), and told where the pointer was as well, as a point of the field:
    /// for what opens beside the name that was pressed. Both are called, if both are set.
    #[inline]
    pub fn on_force_click_at(mut self, on_force_click_at: impl Fn(usize, crate::input_field::line_labels::FieldSpot) + 'static) -> Self {
        self.on_force_click_at = Some(std::rc::Rc::new(on_force_click_at));
        self
    }

    /// Sets what is called when the text is clicked with the platform's jump key held: Command on Apple systems and
    /// Control elsewhere (see `Modifiers::command`). It is called once for each click, with the byte of the text under
    /// the pointer, a frame after the press, when the layout can say which it is. The click moves nothing: it does not
    /// place the caret or change the selection. A press on something drawn over the text, such as the menu of a caret
    /// widget or an icon in the margin, is not a press on the text.
    #[inline]
    pub fn on_command_click(mut self, on_command_click: impl Fn(usize) + 'static) -> Self {
        self.on_command_click = Some(std::rc::Rc::new(on_command_click));
        self
    }

    /// Like [`on_command_click`](Self::on_command_click), and told where the pointer was as well, as a point of the field: for what
    /// opens beside the name that was clicked, as [`on_force_click_at`](Self::on_force_click_at) is. Both are called, if both are set,
    /// and the name under the pointer is a link while the jump key is held if either is.
    #[inline]
    pub fn on_command_click_at(mut self, on_command_click_at: impl Fn(usize, crate::input_field::line_labels::FieldSpot) + 'static) -> Self {
        self.on_command_click_at = Some(std::rc::Rc::new(on_command_click_at));
        self
    }

    /// Sets what is asked what to add to the context menu, each time one is raised by a right click or a long press: rows that go before the
    /// clipboard verbs the field offers, and rows that go after them, each of which does what it was given when it is chosen. The callback is
    /// told what is selected and where the pointer was, which is what an editor decides its rows by, such as whether the name under it can be
    /// renamed.
    ///
    /// Without it the menu is only the clipboard verbs, and with it a field that has nothing to cut, copy or paste still has a menu.
    ///
    /// A row can show more than words: a [`hint`](aimer_ctxmenu::ContextMenuItem::hint) and an [`icon`](aimer_ctxmenu::ContextMenuItem::icon), a
    /// line before it with [`separated`](aimer_ctxmenu::ContextMenuItem::separated), or a whole widget with
    /// [`ContextMenuItem::widget`](aimer_ctxmenu::ContextMenuItem::widget), which takes its own events: a strip of buttons, a toggle, a header.
    #[inline]
    pub fn context_menu_items(
        mut self,
        items: impl Fn(&crate::input_field::context_menu::ContextMenuRequest) -> crate::input_field::context_menu::ContextMenuExtras + 'static,
    ) -> Self {
        self.context_menu_extras = Some(std::rc::Rc::new(items));
        self
    }

    /// Sets the look of the context menu that a right click raises: its panel, labels, rows, hints and lines, as a
    /// [`ContextMenuStyle`](aimer_ctxmenu::ContextMenuStyle) styles any menu. It is the look of the list a desktop gets; the pill that a long
    /// press raises on a touch screen keeps the look that suits a finger.
    #[inline]
    pub fn context_menu_style(mut self, style: aimer_ctxmenu::ContextMenuStyle) -> Self {
        self.context_menu_style = Some(style);
        self
    }

    /// Sets the color of the mark that [`TextEditingController::locate`](crate::TextEditingController::locate) draws over the
    /// range it shows: an outline that closes on the range and fades, with the same color faintly inside it. Without one it
    /// is the color of the caret, or else of the text.
    #[inline]
    pub fn locate_color(mut self, locate_color: impl Into<Color>) -> Self {
        self.locate_color = Some(locate_color.into());
        self
    }

    /// Sets which bytes are something to jump from, for [`Self::on_command_click`]: given the byte under the pointer, the
    /// bytes of the name it is in, or `None` when it is in nothing to jump from, such as a comment or a string. The
    /// range is what is underlined while the jump key is held, and a command click on a byte that has none is not
    /// reported. Without it, a run of letters, digits and underscores is a name. It is asked a frame after the
    /// pointer moved or the key changed, with the byte under the pointer, so it should be quick.
    #[inline]
    pub fn link_range(mut self, link_range: impl Fn(usize) -> Option<std::ops::Range<usize>> + 'static) -> Self {
        self.link_range = Some(std::rc::Rc::new(link_range));
        self
    }

    /// Says where something else is drawn over the field and is pressed like a button, such as a badge floating at its
    /// corner: from the field's width and height, the rectangles `(left, top, width, height)` it covers. The pointer over
    /// one shows an arrow and not the text cursor.
    #[inline]
    pub fn pointer_holes(mut self, holes: impl Into<Option<PointerHoles>>) -> Self {
        self.pointer_holes = holes.into();
        self
    }

    /// Puts a column of clickable icons before the line numbers; see [`GutterMarkers`]. Does nothing
    /// without [`line_numbers`](Self::line_numbers), whose margin it is part of.
    #[inline]
    pub fn gutter_markers(mut self, markers: impl Into<Option<GutterMarkers>>) -> Self {
        self.gutter_markers = markers.into();
        self
    }

    /// Makes Return start the new line with what `prefix` says: it is given the text of the line before
    /// the caret, and returns what the new line begins with, or `None` for nothing. A code editor
    /// continues a comment (`/// `) or a list this way.
    #[inline]
    pub fn line_break_prefix(mut self, prefix: impl Fn(&str) -> Option<String> + 'static) -> Self {
        self.line_break_prefix = Some(Rc::new(prefix));
        self
    }

    /// Lets `filter` take a key press before the area does, and keep it from the area by answering
    /// `true`; see [`TextField::on_key`]. A code editor refuses a key that would change text it keeps
    /// from being edited this way.
    #[inline]
    pub fn on_key(
        mut self,
        filter: impl Fn(&aimer_events::element::NamedKey, &aimer_events::element::Modifiers) -> bool + 'static,
    ) -> Self {
        self.field = self.field.on_key(filter);
        self
    }

    /// Offers `filter` the text that was typed before the area inserts it, and keeps it from the area by answering
    /// `true`; see [`TextField::on_text_input`]. A code editor closes a bracket this way.
    #[inline]
    pub fn on_text_input(mut self, filter: impl Fn(&str) -> bool + 'static) -> Self {
        self.field = self.field.on_text_input(filter);
        self
    }

    /// Draws a bar down the left of the lines `bars` gives one, behind the text; see [`LineBars`].
    #[inline]
    pub fn line_bars(mut self, bars: impl Into<Option<LineBars>>) -> Self {
        self.line_bars = bars.into();
        self
    }

    /// Draws the notes of `inlays` inside the lines, such as the type of a name that a language server knows; see [`Inlays`].
    /// The text is not changed: the notes are room in a line, and are not edited, selected or copied.
    ///
    /// A note is not drawn at the start of a line or at the end of the text, and not in a field that hides its text.
    #[inline]
    pub fn inlays(mut self, inlays: impl Into<Option<crate::input_field::inlay::Inlays>>) -> Self {
        self.inlays = inlays.into();
        self
    }

    /// Draws the lines of `underlines` under ranges of the text, such as under what a language server says is an error. The text is
    /// not changed: a line is not edited, selected or copied. A line is straight, a little above the bottom of the text's line, with
    /// rounded ends, and is drawn a line of text at a time under a range that has more than one.
    #[inline]
    pub fn underlines(mut self, underlines: impl Into<Option<crate::input_field::underline::Underlines>>) -> Self {
        self.underlines = underlines.into();
        self
    }

    /// Puts a label after the end of the lines `labels` gives one, which can have buttons in it; see [`LineLabels`].
    #[inline]
    pub fn line_labels(mut self, labels: impl Into<Option<crate::input_field::line_labels::LineLabels>>) -> Self {
        self.line_labels = labels.into();
        self
    }

    /// Draws a band of `color` behind the line the caret is on, across the margin and the text, as a
    /// code editor marks the current line. `None`, the default, draws none.
    #[inline]
    pub fn current_line(mut self, color: impl Into<Option<Color>>) -> Self {
        self.current_line = color.into();
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

    /// Tells `hover` of the name the pointer rests on, and takes it back when the pointer goes; see [`TextField::on_hover`].
    #[inline]
    pub fn on_hover(mut self, hover: crate::input_field::hover::TextHover) -> Self {
        self.field = self.field.on_hover(hover);
        self
    }

    /// Boxes what `marks` says of where the caret is, and marks it for a moment when it changes; see [`TextField::caret_marks`].
    #[inline]
    pub fn caret_marks(mut self, marks: crate::input_field::caret_marks::CaretMarks) -> Self {
        self.field = self.field.caret_marks(marks);
        self
    }

    /// Rounds the corners of the selection, and joins its lines into one shape; see [`TextField::selection_radius`].
    #[inline]
    pub fn selection_radius(mut self, radius: f32) -> Self {
        self.field = self.field.selection_radius(radius);
        self
    }

    /// Sets the color of the builtin insertion cursor.
    ///
    /// Without it the cursor takes the text color, which already contrasts with
    /// the background behind the text: light on a dark page, dark on a light one.
    ///
    /// This has no effect on a custom caret builder. Custom carets receive the
    /// live [`CaretContext`] and choose their own visual style.
    #[inline]
    pub fn cursor_color(mut self, cursor_color: Colors) -> Self {
        self.field = self.field.cursor_color(cursor_color);
        self
    }

    /// Sets whether the caret glides to where the cursor moved, as in modern
    /// editors, or jumps there. The default is `true`.
    ///
    /// See [`TextField::smooth_caret`](crate::input::TextField::smooth_caret).
    #[inline]
    pub fn smooth_caret(mut self, smooth_caret: bool) -> Self {
        self.field = self.field.smooth_caret(smooth_caret);
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

    /// Styles ranges of the text, such as code tokens or `@mentions`.
    ///
    /// The text stays a plain string, so editing, selection and input methods
    /// behave as without a highlighter. Create the [`TextHighlighter`] once and
    /// clone it into each build; see its documentation for what the function
    /// receives and how its ranges are repaired.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_input::TextArea;
    /// use aimer_input::input::{SpanStyle, StyledRange, TextHighlighter};
    /// use aimer_widget::base::Color;
    ///
    /// // Marks every `TODO` in a note.
    /// let todos = TextHighlighter::new(|text| {
    ///     let style = SpanStyle::new().color(Color::Rgba(220, 60, 60, 255));
    ///     text.match_indices("TODO")
    ///         .map(|(start, word)| StyledRange::new(start..start + word.len(), style))
    ///         .collect()
    /// });
    ///
    /// let area = TextArea::new().highlighter(todos);
    /// # let _ = area;
    /// ```
    #[inline]
    pub fn highlighter(mut self, highlighter: TextHighlighter) -> Self {
        self.field = self.field.highlighter(highlighter);
        self
    }

    /// Shows ranges of the text as widgets, such as the table or code block of a
    /// markdown note; see [`TextField::embedder`](crate::input::TextField::embedder).
    #[inline]
    pub fn embedder(mut self, embedder: crate::input::TextEmbedder) -> Self {
        self.field = self.field.embedder(embedder);
        self
    }

    /// Removes the embedder, so every range is drawn as text again.
    #[inline]
    pub fn no_embedder(mut self) -> Self {
        self.field = self.field.no_embedder();
        self
    }

    /// Removes the highlighter, so the text is drawn in one style again.
    #[inline]
    pub fn no_highlighter(mut self) -> Self {
        self.field = self.field.no_highlighter();
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
        config.tab_spaces = self.tab_spaces;
        config.wrap = self.wrap;
        config.clip_to_padding = self.clip_to_padding;
        config.line_numbers = self.line_numbers;
        config.gutter_markers = self.gutter_markers.clone();
        config.pointer_holes = self.pointer_holes.clone();
        config.on_force_click = self.on_force_click.clone();
        config.on_force_click_at = self.on_force_click_at.clone();
        config.on_command_click = self.on_command_click.clone();
        config.on_command_click_at = self.on_command_click_at.clone();
        config.context_menu_extras = self.context_menu_extras.clone();
        config.context_menu_style = self.context_menu_style.clone();
        config.link_range = self.link_range.clone();
        config.locate_color = self.locate_color;
        config.line_bars = self.line_bars.clone();
        config.line_labels = self.line_labels.clone();
        config.inlays = self.inlays.clone();
        config.underlines = self.underlines.clone();
        config.line_break_prefix = self.line_break_prefix.clone();
        config.current_line = self.current_line;
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

impl State<TextArea> for TextFieldState {
    fn create(widget: TextArea) -> Self {
        widget.field.create_state_with_config(widget.config())
    }

    #[inline]
    fn adopt_config_from(&mut self, new: Self) {
        self.adopt(new);
    }

    #[inline]
    fn build(this: StateView<Self>, _: &BuildContext) -> impl Widget {
        this.build_field()
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
        StatefulElement::new_with_name::<Self, TextFieldState>(self, ctx, "TextArea", __key)
            .0
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    use aimer_widget::CreateState;
    use aimer_attribute::Vec2d;
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_style::{BoxDecoration, LayoutSpacing, TextAlign, TextStyle};
    use crate::input_field::TextFieldState;
    use crate::input_field::highlight::TextHighlighter;
    use aimer_widget::base::{Color, Colors};
    use aimer_widget::{AnyElement, Drawable, EventDispatcher, FocusNode, Key, Widget};

    use crate::input::{DefaultCaret, TextArea, TextEditingController};
    use crate::input_field::raw_fields::test_support::dummy_build_context;
    use crate::input_field::raw_fields::{ExpandDirection, InputType};

    #[test]
    fn default_text_area_is_unbounded_multiline_without_expansion() {
        let state = TextArea::new().create_state::<TextFieldState>();
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
            .create_state::<TextFieldState>();
        let config = state.config();

        assert_eq!(config.min_lines, Some(6));
        assert_eq!(config.max_lines, Some(6));
        assert_eq!(config.expand, ExpandDirection::Vertical);
    }

    #[test]
    fn the_tab_setting_reaches_the_created_state() {
        let state = TextArea::new().create_state::<TextFieldState>();
        assert_eq!(state.config().tab_spaces, None, "Tab moves focus unless asked");

        let state = TextArea::new().tab_spaces(4).create_state::<TextFieldState>();
        assert_eq!(state.config().tab_spaces, Some(4));
        let state = TextArea::new().tab_spaces(4).tab_spaces(None).create_state::<TextFieldState>();
        assert_eq!(state.config().tab_spaces, None);
    }

    #[test]
    fn the_text_is_clipped_to_the_padding_unless_asked_not_to() {
        let state = TextArea::new().create_state::<TextFieldState>();
        assert!(state.config().clip_to_padding, "the padding is where the text is cut, as it always was");

        let state = TextArea::new().clip_to_padding(false).create_state::<TextFieldState>();
        assert!(!state.config().clip_to_padding);
        let state = TextArea::new().clip_to_padding(false).clip_to_padding(true).create_state::<TextFieldState>();
        assert!(state.config().clip_to_padding);
    }

    #[test]
    fn a_key_filter_reaches_the_created_state() {
        use aimer_events::element::{Modifiers, NamedKey};

        let state = TextArea::new()
            .on_key(|key, _| matches!(key, NamedKey::Backspace))
            .create_state::<TextFieldState>();

        let filter = state.config().key_filter.as_ref().expect("it was set").clone();
        assert!(filter(&NamedKey::Backspace, &Modifiers::default()));
        assert!(!filter(&NamedKey::Delete, &Modifiers::default()));
        assert!(TextArea::new().create_state::<TextFieldState>().config().key_filter.is_none());
    }

    #[test]
    fn a_custom_caret_builder_reaches_the_created_state() {
        let state = TextArea::new()
            .caret(|context| DefaultCaret::new(context, Colors::default()))
            .create_state::<TextFieldState>();

        assert!(state.config().caret_builder.is_some());
    }

    #[test]
    fn a_highlighter_reaches_the_raw_field_configuration() {
        let highlighter = TextHighlighter::new(|_| Vec::new());
        let state = TextArea::new()
            .highlighter(highlighter.clone())
            .create_state::<TextFieldState>();

        let configured = state.config().highlighter.as_ref().expect("it was set");
        assert_eq!(configured.id(), highlighter.id());
        let cleared = TextArea::new()
            .highlighter(highlighter)
            .no_highlighter()
            .create_state::<TextFieldState>();
        assert!(cleared.config().highlighter.is_none());
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
        let state = area.create_state::<TextFieldState>();
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

    /// Presses at `start`, moves through every point of `path`, releases at the
    /// last one, and reports the selection it leaves, in graphemes.
    fn drag_along(text: &str, start: Vec2d, path: &[Vec2d]) -> (usize, usize) {
        let controller = TextEditingController::with_text(text);
        let ctx = dummy_build_context(200.0, 80.0);
        let element: AnyElement = TextArea::new()
            .controller(controller.clone())
            .to_element(&ctx);
        element.update(&ctx);
        let mut dispatcher = EventDispatcher::new();

        let _ = dispatcher.dispatch(
            element.as_ref(),
            start,
            &ElementEvent::PointerDown(PointerInfo::mouse(start, PointerButton::Primary)),
        );
        element.update(&ctx);
        for point in path {
            let _ = dispatcher.dispatch(
                element.as_ref(),
                *point,
                &ElementEvent::PointerMove(PointerInfo::mouse(*point, PointerButton::Primary)),
            );
            element.update(&ctx);
        }
        let end = *path.last().expect("a drag moves at least once");
        let _ = dispatcher.dispatch(
            element.as_ref(),
            end,
            &ElementEvent::PointerUp(PointerInfo::mouse(end, PointerButton::Primary)),
        );
        controller.selection_graphemes()
    }

    /// A selection drag keeps following the pointer after it leaves the field.
    ///
    /// The press captures the pointer, so the field keeps hearing about it
    /// wherever it goes; a position past the field, below it or to its side,
    /// stands for the nearest edge of the text. The drag below ends far below
    /// and to the right of a two-line field, which is the end of the text.
    #[test]
    fn a_drag_that_leaves_the_field_keeps_extending_the_selection() {
        const TEXT: &str = "first line\nsecond line";
        let length = TEXT.chars().count();
        let inside = Vec2d { x: 40.0, y: 12.0 };
        let outside = Vec2d { x: 900.0, y: 600.0 };

        let selection = drag_along(TEXT, Vec2d { x: 1.0, y: 12.0 }, &[inside, outside]);

        assert_eq!(selection, (0, length), "the selection stopped at the field's edge");
    }

    /// The same drag, sent straight out of the field with no move in between.
    #[test]
    fn a_drag_that_starts_leaving_at_once_still_reaches_the_end() {
        const TEXT: &str = "first line\nsecond line";
        let length = TEXT.chars().count();

        let selection = drag_along(
            TEXT,
            Vec2d { x: 1.0, y: 12.0 },
            &[Vec2d { x: 900.0, y: 600.0 }],
        );

        assert_eq!(selection, (0, length));
    }

    /// Dragging upward out of the field reaches the start of the text.
    #[test]
    fn a_drag_that_leaves_upward_reaches_the_start() {
        const TEXT: &str = "first line\nsecond line";
        let length = TEXT.chars().count();

        // The selection keeps its direction: it is anchored where the press was and
        // its focus, where the caret goes, is the start the drag reached.
        let (anchor, focus) = drag_along(
            TEXT,
            Vec2d { x: 150.0, y: 12.0 },
            &[Vec2d { x: 60.0, y: 12.0 }, Vec2d { x: -400.0, y: -300.0 }],
        );

        assert_eq!(focus, 0, "the selection stopped at the field's edge");
        assert!(anchor > 0 && anchor <= length, "selected {focus}..{anchor}");
    }
}
