#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputType {
    #[default]
    Text,
    Number,
    Obscure,
    Password,
    Email,
    Tel,
    Url,
    Search,
    Date,
    Time,
    DateTimeLocal,
    Month,
    Week,
    Hidden,
    Reset,
    Submit,
    Image,
    File,
}

impl InputType {
    /// Returns whether the field should hide committed and composing text.
    #[inline]
    pub const fn is_obscured(self) -> bool {
        matches!(self, Self::Obscure | Self::Password)
    }

    /// Returns the conventional browser input type for this hint.
    ///
    /// The native mobile bridges currently accept a smaller numeric kind and
    /// deliberately fall back to their text configuration for the other
    /// values. This method is a hint only; it does not validate field content.
    #[inline]
    pub const fn html_type(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Obscure | Self::Password => "password",
            Self::Number => "number",
            Self::Email => "email",
            Self::Tel => "tel",
            Self::Url => "url",
            Self::Search => "search",
            Self::Date => "date",
            Self::Time => "time",
            Self::DateTimeLocal => "datetime-local",
            Self::Month => "month",
            Self::Week => "week",
            Self::Hidden => "hidden",
            Self::Reset => "reset",
            Self::Submit => "submit",
            Self::Image => "image",
            Self::File => "file",
        }
    }

    /// Returns the bounded kind understood by the existing mobile bridges.
    ///
    /// `Number` retains the numeric keyboard hint and both obscured variants
    /// retain secure entry. All other kinds use the bridge's text fallback;
    /// their actual validation remains the form layer's responsibility.
    #[inline]
    pub const fn native_input_kind(self) -> i32 {
        match self {
            Self::Number => 1,
            Self::Obscure | Self::Password => 2,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod input_type_tests {
    use super::InputType;

    #[test]
    fn backlog_types_are_distinct_hints_with_explicit_browser_names() {
        assert_eq!(InputType::Email.html_type(), "email");
        assert_eq!(InputType::DateTimeLocal.html_type(), "datetime-local");
        assert_eq!(InputType::File.html_type(), "file");
        assert_eq!(InputType::Obscure.html_type(), "password");
        assert_eq!(InputType::Number.native_input_kind(), 1);
        assert_eq!(InputType::Password.native_input_kind(), 2);
        assert_eq!(InputType::Obscure.native_input_kind(), 2);
    }

    #[test]
    fn unsupported_native_hints_fall_back_to_text_without_implying_validation() {
        assert_eq!(InputType::Email.native_input_kind(), 0);
        assert_eq!(InputType::Date.native_input_kind(), 0);
        assert_eq!(InputType::File.native_input_kind(), 0);
        assert!(!InputType::Email.is_obscured());
    }
}

#[allow(dead_code)]
pub struct Cursor {
    offset: Cell<usize>,
    /// Selection anchor (the end that doesn't move). `None` means no selection.
    selection_anchor: Cell<Option<usize>>,
    /// Blink timeline shared with the owning field state, so the phase outlives
    /// the element it is painted from.
    blink: CaretBlink,
}

impl Cursor {
    /// Creates a cursor with a blink timeline of its own.
    pub fn new(_color: Colors) -> Self {
        Self::with_blink(_color, CaretBlink::new())
    }

    /// Creates a cursor that blinks on `blink`.
    ///
    /// A field built by [`TextField`] passes the timeline owned by its state,
    /// which is what keeps the caret rhythm continuous across rebuilds. The
    /// color argument remains accepted for source compatibility; visual color
    /// belongs to the field's caret widget rather than to editing state.
    ///
    /// [`TextField`]: crate::input_field::TextField
    pub fn with_blink(_color: Colors, blink: CaretBlink) -> Self {
        Self {
            offset: Cell::new(0),
            selection_anchor: Cell::new(None),
            blink,
        }
    }

    /// Returns the blink timeline this cursor is painted from.
    #[inline]
    pub fn blink(&self) -> &CaretBlink {
        &self.blink
    }

    pub fn offset(&self) -> usize {
        self.offset.get()
    }

    pub fn set_offset(&self, offset: usize) {
        self.offset.set(offset);
    }

    /// Returns whether the caret is opaque at the current blink phase.
    pub fn is_visible(&self) -> bool {
        self.blink.is_visible()
    }

    /// Restart the blink timeline so the caret is solid again.
    fn reset_blink(&self) {
        self.blink.reset();
    }

    /// Returns the selection anchor, if any.
    pub fn selection_anchor(&self) -> Option<usize> {
        self.selection_anchor.get()
    }

    /// Set the selection anchor.
    pub fn set_selection_anchor(&self, anchor: Option<usize>) {
        self.selection_anchor.set(anchor);
    }

    /// Returns the ordered (start, end) of the current selection, or `None`.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection_anchor().and_then(|anchor| {
            let offset = self.offset();
            let range = if anchor <= offset {
                (anchor, offset)
            } else {
                (offset, anchor)
            };
            (range.0 != range.1).then_some(range)
        })
    }

    /// Clear the selection without moving the cursor.
    pub fn clear_selection(&self) {
        self.set_selection_anchor(None);
    }
}

/// Offered every key press a focused field receives before the field acts on
/// it. Returning `true` consumes the key, so the field leaves it alone; a
/// combobox uses this to move its highlight with the arrow keys.
pub(crate) type KeyFilter = Rc<dyn Fn(&NamedKey, &Modifiers) -> bool>;

/// Offered the text a focused field is about to insert because it was typed, before the field inserts it. Returning `true`
/// says the text was dealt with, by edits made through the field's controller, and the field leaves it alone. A code
/// editor closes a bracket this way, and steps over a closing one that is already there.
pub(crate) type TextFilter = Rc<dyn Fn(&str) -> bool>;

/// Says what a new line starts with, given the text of the line before the caret.
pub(crate) type LineBreakPrefix = Rc<dyn Fn(&str) -> Option<String>>;

/// Tells where the parts of a field are that something else is drawn over, from the width and the height of the
/// field: each as `(left, top, width, height)` in logical pixels from the field's top-left corner. The pointer over
/// one is not over the text, and the field shows an arrow there and not the text cursor, as it does over an icon in the
/// margin.
pub type PointerHoles = std::rc::Rc<dyn Fn(f32, f32) -> Vec<(f32, f32, f32, f32)>>;

/// What a field calls when it is force-clicked: the trackpad is pressed down hard on the text, which a macOS Force Touch
/// trackpad reports as a deep stage. It is given the byte of the text under the pointer.
pub type ForceClick = std::rc::Rc<dyn Fn(usize)>;

/// What a field calls when it is force-clicked and the app wants to know where the pointer was, as well as the byte of the
/// text under it: what a card that opens beside the name hangs from. The spot is a point, in the field's own coordinates.
pub type ForceClickAt = std::rc::Rc<dyn Fn(usize, crate::input_field::line_labels::FieldSpot)>;

/// What a field calls when it is clicked with the platform's jump key held (see `Modifiers::command`). It is given the
/// byte of the text under the pointer, as a [`ForceClick`] is.
pub type CommandClick = ForceClick;

/// What a field calls when it is clicked with the platform's jump key held and the app wants to know where the pointer was, as
/// well as the byte of the name under it: what a card that opens beside the name hangs from, as a [`ForceClickAt`] is.
pub type CommandClickAt = ForceClickAt;

/// Which bytes are something to jump from, when the app knows better than the field: given the byte under the pointer, the
/// bytes of the name it is in, or `None` when it is in nothing to jump from, such as a comment or a string. A field without
/// one takes a run of letters, digits and underscores for a name.
pub type LinkRange = std::rc::Rc<dyn Fn(usize) -> Option<std::ops::Range<usize>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExpandDirection {
    Horizontal,
    Vertical,
    Both,
    #[default]
    None,
}
/// Everything a [`RawTextField`] needs that is configuration rather than
/// runtime state.
///
/// [`TextField`] keeps one of these in its state and hands a clone to every
/// element it builds, so the widget configuration and the caret timeline travel
/// separately: the configuration is replaced on reconciliation while the
/// timeline keeps running.
///
/// [`TextField`]: crate::input_field::TextField
#[derive(Clone)]
pub(crate) struct RawFieldConfig {
    pub input_type: InputType,
    pub controller: TextEditingController,
    pub prompt: Arc<str>,
    pub hint: Arc<str>,
    pub hint_style: TextStyle,
    pub text_style: TextStyle,
    pub prompt_style: TextStyle,
    pub text_align: TextAlign,
    /// Whether the field asks for the keyboard as soon as it is mounted.
    ///
    /// Read by the field's state to pick the [`FocusBehavior`] of the focus
    /// region it wraps the element in, never by the element itself.
    ///
    /// [`FocusBehavior`]: aimer_widget::FocusBehavior
    pub auto_focus: bool,
    pub max_lines: Option<usize>,
    pub min_lines: Option<usize>,
    pub max_length: Option<usize>,
    pub enable: bool,
    pub expand: ExpandDirection,
    pub decoration: BoxDecoration,
    pub hover_decoration: Option<BoxDecoration>,
    pub focus_decoration: Option<BoxDecoration>,
    pub disabled_decoration: Option<BoxDecoration>,
    pub selection_color: Color,
    /// How round the corners of a selection are, in logical pixels; see [`TextField::selection_radius`].
    pub selection_radius: f32,
    /// The builtin caret's colour; `None` follows the text colour.
    pub cursor_color: Option<Colors>,
    /// Whether the caret glides to a moved cursor instead of jumping to it.
    pub smooth_caret: bool,
    pub caret_builder: Option<CaretBuilder>,
    /// Styles ranges of the displayed text; see [`TextHighlighter`].
    pub highlighter: Option<TextHighlighter>,
    /// Shows ranges of the displayed text as widgets; see [`TextEmbedder`].
    pub embedder: Option<TextEmbedder>,
    pub on_changed: TextFieldCallback,
    pub on_submitted: TextFieldCallback,
    pub on_focus: TextFieldCallback,
    pub on_blur: TextFieldCallback,
    pub read_only: bool,
    pub key_filter: Option<KeyFilter>,
    /// What is offered typed text before it is inserted; see [`TextFilter`].
    pub text_filter: Option<TextFilter>,
    /// What is boxed for the caret; see [`TextField::caret_marks`].
    pub caret_marks: Option<CaretMarks>,
    /// What is told of the name the pointer rests on; see [`TextField::on_hover`].
    pub hover: Option<TextHover>,
    /// What a new line starts with, given the line up to the caret; see [`TextArea::line_break_prefix`].
    pub line_break_prefix: Option<LineBreakPrefix>,
    /// How many spaces the Tab key inserts, for a field that takes it instead of moving focus.
    pub tab_spaces: Option<usize>,
    /// Whether the text is cut off at the padding of the field, above and below: `true` unless asked; see
    /// [`TextArea::clip_to_padding`].
    pub clip_to_padding: bool,
    /// Whether long lines wrap to the width of the field. A field that does not wrap scrolls
    /// sideways to its caret instead. Only a multi-line field has the choice: a single-line one
    /// never wraps.
    pub wrap: bool,
    /// The line numbers in the margin, if the field has them.
    pub line_numbers: Option<LineNumbers>,
    /// The clickable icons before the numbers, if the field has them.
    pub gutter_markers: Option<GutterMarkers>,
    /// The parts of the field that something else is drawn over and is pressed like a button, if there are any;
    /// see [`PointerHoles`].
    pub pointer_holes: Option<PointerHoles>,
    /// What is called when the text is force-clicked, if anything is; see [`ForceClick`].
    pub on_force_click: Option<ForceClick>,
    /// What is called when the text is force-clicked, told where the pointer was too; see [`ForceClickAt`]. Called with
    /// [`on_force_click`](Self::on_force_click), if there is one.
    pub on_force_click_at: Option<ForceClickAt>,
    /// What is called when the text is clicked with the jump key held, if anything is; see [`CommandClick`].
    pub on_command_click: Option<CommandClick>,
    /// What is asked what to add to the context menu before it is raised, if anything is; see
    /// [`ContextMenuExtender`](crate::input_field::context_menu::ContextMenuExtender).
    pub context_menu_extras: Option<crate::input_field::context_menu::ContextMenuExtender>,
    /// The look of the context menu of a desktop, which is a list; a menu that a finger asks for keeps its own.
    pub context_menu_style: Option<aimer_ctxmenu::ContextMenuStyle>,
    /// What is called when the text is clicked with the jump key held, told where the pointer was too; see [`CommandClickAt`]. Called with
    /// [`on_command_click`](Self::on_command_click), if there is one, and the name is a link while the key is held for either.
    pub on_command_click_at: Option<CommandClickAt>,
    /// What says which bytes are links while the jump key is held, if the app has a say; see [`LinkRange`].
    pub link_range: Option<LinkRange>,
    /// What the mark of a range that [`TextEditingController::locate`](crate::TextEditingController::locate) shows is drawn
    /// in. Without one it is the color of the caret, or else of the text.
    pub locate_color: Option<Color>,
    /// The bars beside some lines, if the field has them.
    pub line_bars: Option<LineBars>,
    /// The labels after the end of some lines, if the field has them.
    pub line_labels: Option<crate::input_field::line_labels::LineLabels>,
    /// The notes drawn inside lines, if the field has them.
    pub inlays: Option<crate::input_field::inlay::Inlays>,
    /// The lines drawn under ranges of the text, if the field has them.
    pub underlines: Option<crate::input_field::underline::Underlines>,
    /// The color of the band behind the line the caret is on, if the field draws one.
    pub current_line: Option<Color>,
    pub padding: LayoutSpacing,
    /// The frame placed around the field, when it has one.
    ///
    /// Read by the field's state, never by the element. A field with chrome is
    /// mounted without a decoration of its own.
    pub chrome: Option<Chrome>,
    /// Shared with the state, which owns it, so that the element can report
    /// hover, focus and emptiness to the chrome built around it.
    pub visual: Option<Rc<VisualFlags>>,
}

/// The widget that mounts a [`RawTextField`] element.
///
/// The element owns the interaction state that only makes sense while mounted
/// (focus, hover, selection drag, IME composition), so it must be produced from
/// a widget rather than stored in one. This wrapper is that widget: it carries
/// the configuration and the caret timeline of the field that built it.
pub(crate) struct RawTextFieldWidget {
    config: RawFieldConfig,
    caret: CaretBlink,
    caret_context: CaretContext,
    caret_slot: Rc<CaretSlot>,
    focus_node: FocusNode,
    caret_scope: Option<ScopeId>,
}

impl RawTextFieldWidget {
    /// Creates the widget for a field configured by `config` blinking on
    /// `caret` and sharing `caret_context` with its retained caret child.
    #[inline]
    pub(crate) fn new(
        config: RawFieldConfig,
        caret: CaretBlink,
        caret_context: CaretContext,
        caret_slot: Rc<CaretSlot>,
        focus_node: FocusNode,
        caret_scope: Option<ScopeId>,
    ) -> Self {
        Self {
            config,
            caret,
            caret_context,
            caret_slot,
            focus_node,
            caret_scope,
        }
    }
}

impl RawFieldConfig {
    /// The colour of the builtin caret and of composition underlines.
    ///
    /// Without a configured colour this is the text colour. The text is drawn
    /// in a colour that reads against whatever it sits on, so a caret that
    /// follows it does as well, light on a dark page and dark on a light one,
    /// where a fixed colour disappears on one of them.
    #[inline]
    pub fn caret_color(&self) -> Color {
        self.cursor_color
            .map_or(self.text_style.color, Into::into)
    }
}

impl Widget for RawTextFieldWidget {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let cursor_color = self.config.caret_color();
        let smooth_caret = self.config.smooth_caret;
        let caret = self.caret_slot.build(
            self.config.caret_builder.as_ref(),
            self.caret_context.clone(),
            cursor_color,
            ctx,
        );
        let selection = SelectionParticipant::from_context(
            ctx,
            crate::editable_text::selection_source(&self.config.controller),
        );
        let field = RawTextField::with_caret_context(
            self.config,
            self.caret,
            self.caret_context,
            self.focus_node,
            selection,
            self.caret_scope,
        )
        .with_embeds(Rc::clone(&self.caret_slot.embeds));

        RawTextFieldHost::new(field, caret).smooth(smooth_caret).boxed()
    }
}

/// What the layout depends on from the embeds: who asked, and each box's key,
/// place and size.
fn embed_signature(embedder: Option<&TextEmbedder>, boxes: &[EmbedBox]) -> u64 {
    use std::hash::{Hash, Hasher};

    if embedder.is_none() && boxes.is_empty() {
        return 0;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    embedder_id(embedder).hash(&mut hasher);
    for embed in boxes {
        embed.key.hash(&mut hasher);
        embed.range.hash(&mut hasher);
        embed.width.to_bits().hash(&mut hasher);
        embed.height.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// The whole text as a string, made when it is asked for and not before: a memo that holds the
/// answer never asks, so a long text is not turned into a string by a frame that changed nothing.
/// The shortest paste, in bytes, that goes into the text as the clipboard's own bytes: a shorter one
/// is copied in, which costs less than the chunks that share it.
const SHARED_PASTE_MIN: usize = 64 * 1024;

type Whole<'a> = &'a dyn Fn() -> std::borrow::Cow<'a, str>;

/// The highlighter's ranges with a box in place of every embed. A highlighted range that reaches into
/// an embed is cut at it, and what lies outside keeps its style: the embed draws the text inside. A
/// range that is cut in two is two ranges, one on each side, so a comment that runs through a rendered
/// block is still a comment before it and after it.
fn with_embed_boxes(
    ranges: Option<Rc<[aimer_text::StyledRange]>>,
    boxes: &[EmbedBox],
    scale: f32,
) -> Option<Rc<[aimer_text::StyledRange]>> {
    if boxes.is_empty() {
        return ranges;
    }
    let scale = if scale > 0.0 { scale } else { 1.0 };
    // In order, so that the ones a range reaches into are found by where it starts. They do not
    // overlap: embeds that do were left out when they were made.
    let mut order: Vec<&Range<usize>> = boxes.iter().map(|embed| &embed.range).collect();
    order.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<aimer_text::StyledRange> = Vec::new();
    for styled in ranges.iter().flat_map(|ranges| ranges.iter()) {
        let mut from = styled.range.start;
        // The first embed that ends after the range starts.
        let first = order.partition_point(|embed| embed.end <= styled.range.start);
        let mut cut = false;
        for embed in &order[first..] {
            if embed.start >= styled.range.end {
                break;
            }
            cut = true;
            if from < embed.start {
                merged.push(aimer_text::StyledRange::new(from..embed.start, styled.style.clone()));
            }
            from = from.max(embed.end);
        }
        if !cut {
            merged.push(styled.clone());
        } else if from < styled.range.end {
            merged.push(aimer_text::StyledRange::new(from..styled.range.end, styled.style.clone()));
        }
    }
    for embed in boxes {
        // A hair under the full width, so rounding never wraps the box away
        // from a line of its own.
        let width = (embed.width / scale - 0.01).max(0.0);
        merged.push(aimer_text::StyledRange::new(
            embed.range.clone(),
            aimer_text::SpanStyle::new()
                .inline_box(aimer_text::InlineBox::new(width, embed.height / scale)),
        ));
    }
    Some(merged.into())
}

/// What a [`HighlightContext`] was built from: the text revision, the cursor
/// offset, the selection anchor, and where the drag in progress began, which
/// decides what the context counts as active.
type HighlightKey = (u64, usize, Option<usize>, Option<usize>);
/// What the display text depends on: the controller's revision, which every change
/// to its value (a composition included) advances, and whether the field obscures.
type DisplayKey = (u64, bool);

/// What a pending context-menu request is anchored to.
///
/// A right-click pins the menu to the click, the way every desktop menu does.
/// A hold has no cursor to pin to and covers whatever it touches, so its pill
/// floats clear of the whole field.
#[derive(Clone, Copy)]
enum MenuOrigin {
    /// A hold: the pill floats above the field.
    Hold,
    /// A secondary click: the list opens at the click.
    Click(Vec2d),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RetainedFieldPaintState {
    controller_revision: u64,
    cursor_offset: usize,
    selection_anchor: Option<usize>,
    preedit_cursor: Option<(usize, usize)>,
    scroll_x_bits: u32,
    scroll_y_bits: u32,
    input_language: Option<aimer_cupid::font::TextLanguage>,
    hovered: bool,
    focused: bool,
    composing: bool,
    highlighter: u64,
    link: Option<(usize, usize, u64)>,
    /// How far along the mark of a located range is, in thousandths: it changes with every frame while there is one.
    pulse: Option<u32>,
    /// The same for the mark of the boxes round what the caret is next to. What they are round follows the controller's revision
    /// and the focus, which are here already.
    marks_pulse: Option<u32>,
    /// The window a windowed text was laid out for, and the version of the heights it was placed by.
    window: Option<((usize, usize), u64)>,
    /// Which button of a label after a line the pointer is on, if one: it is drawn in another color.
    label_hover: Option<usize>,
    /// Which set of lines under the text there is: zero for none.
    underlines: u64,
}

#[allow(dead_code)]
pub(crate) struct RawTextField {
    pub input_type: InputType,
    pub controller: TextEditingController,
    pub controller_attachment: ControllerAttachment,
    pub observed_revision: Cell<u64>,
    pub native_session: Cell<u64>,
    /// Floor of the native delta acceptance window: the controller revision
    /// carried by the last snapshot pushed to the platform text editor.
    ///
    /// The editor bases its deltas on that snapshot and keeps reporting
    /// against it while further snapshots are in flight, so a delta is not
    /// stale merely because the controller has moved past its revision — it
    /// is stale only when it predates the last pushed snapshot.
    pub native_base_revision: Cell<u64>,
    /// The controller revision whose value the platform text editor is known
    /// to mirror.
    ///
    /// Deltas carry offsets into the editor's own buffer; they only apply
    /// while that buffer and the controller text are the same, which holds
    /// exactly when every revision since the last push was produced by the
    /// editor's own deltas.
    pub native_mirror_revision: Cell<u64>,
    pub geometry_cache: EditableGeometryCache,
    /// Styles ranges of the displayed text; ignored by a secure field.
    pub highlighter: Option<TextHighlighter>,
    pub highlights: HighlightCache,
    /// Shows ranges of the text as widgets; see [`TextEmbedder`].
    pub embedder: Option<TextEmbedder>,
    /// The embedded widgets, shared with the state that owns them so a rebuilt
    /// field finds the same ones.
    pub(crate) embeds: Rc<EmbedStore>,
    /// The last embeds the embedder answered with, and what they were asked for.
    embed_memo: RefCell<Option<(HighlightKey, Rc<[TextEmbed]>)>>,
    /// The embeds of the bytes a windowed embedder was last asked about, with what they were for.
    window_embed_memo: RefCell<Option<(HighlightKey, Range<usize>, bool, Rc<[TextEmbed]>)>>,
    pub display_widget: RetainedDisplay,
    pub prompt: Arc<str>,
    pub hint: Arc<str>,
    pub hint_style: TextStyle,
    pub text_style: TextStyle,
    pub prompt_style: TextStyle,
    pub text_align: TextAlign,
    pub max_lines: Option<usize>,
    pub min_lines: Option<usize>,
    pub max_length: Option<usize>,
    pub enable: bool,
    pub expand: ExpandDirection,
    pub cursor: Cursor,
    /// Color supplied to the builtin caret and composition underlines; `None`
    /// follows the text colour.
    pub caret_color: Option<Colors>,
    pub decoration: BoxDecoration,
    pub hover_decoration: Option<BoxDecoration>,
    pub focus_decoration: Option<BoxDecoration>,
    pub disabled_decoration: Option<BoxDecoration>,
    pub selection_color: Color,
    /// How round the corners of a selection are, in logical pixels; see [`TextField::selection_radius`].
    pub selection_radius: f32,
    pub focus_node: FocusNode,
    pub focused: Cell<bool>,
    pub hovered: Cell<bool>,
    /// Taken while the field has its I-beam up; see [`aimer_utils::cursor::CursorClaim`].
    pub cursor_claim: aimer_utils::cursor::CursorClaim,
    pub(super) retained_v2_paint_state: Cell<Option<RetainedFieldPaintState>>,
    pub cached_bounds: aimer_widget::InteractionBounds,
    pub on_changed: TextFieldCallback,
    pub on_submitted: TextFieldCallback,
    pub on_focus: TextFieldCallback,
    pub on_blur: TextFieldCallback,
    pub read_only: bool,
    pub key_filter: Option<KeyFilter>,
    /// What is offered typed text before it is inserted; see [`TextFilter`].
    pub text_filter: Option<TextFilter>,
    /// What is boxed for the caret; see [`RawFieldConfig::caret_marks`].
    pub caret_marks: Option<CaretMarks>,
    /// What is told of the name the pointer rests on; see [`RawFieldConfig::hover`].
    pub hover: Option<TextHover>,
    /// The name the pointer is on and is being waited on for, and since when.
    pub hover_candidate: RefCell<Option<(Range<usize>, AnimInstant)>>,
    /// The name that was told of, and the revision of the text it was told for.
    pub hover_shown: RefCell<Option<(Range<usize>, u64)>>,
    /// Whether nothing is told until the pointer moves: a press, a key or a scroll ended what was.
    pub hover_blocked: Cell<bool>,
    /// Whether the name under the pointer is to be found again by the next frame.
    pub pending_hover: Cell<bool>,
    /// What the marks were last asked for and said, so that a frame that is not a new text or caret asks nothing.
    pub marks_asked: RefCell<Option<MarksAsked>>,
    /// The ranges that were boxed the last time a field was drawn, and when they were first: what a mark is made of.
    pub marks_seen: RefCell<Vec<Range<usize>>>,
    pub marks_began: Cell<Option<AnimInstant>>,
    /// What a new line starts with; see [`RawFieldConfig::line_break_prefix`].
    pub line_break_prefix: Option<LineBreakPrefix>,
    /// How many spaces the Tab key inserts; `None` leaves Tab to move focus.
    pub tab_spaces: Option<usize>,
    /// Whether long lines wrap; see [`RawFieldConfig::wrap`].
    pub wrap: bool,
    /// Whether the text is cut off at the padding; see [`RawFieldConfig::clip_to_padding`].
    pub clip_to_padding: bool,
    /// The line numbers in the margin; see [`RawFieldConfig::line_numbers`].
    pub line_numbers: Option<LineNumbers>,
    /// The icons before the numbers; see [`RawFieldConfig::gutter_markers`].
    pub gutter_markers: Option<GutterMarkers>,
    /// The parts of the field that something else is over; see [`RawFieldConfig::pointer_holes`].
    pub pointer_holes: Option<PointerHoles>,
    /// What is called when the text is force-clicked; see [`RawFieldConfig::on_force_click`].
    pub on_force_click: Option<ForceClick>,
    /// What is called when the text is force-clicked, with where; see [`RawFieldConfig::on_force_click_at`].
    pub on_force_click_at: Option<ForceClickAt>,
    /// What is called when the text is command-clicked; see [`RawFieldConfig::on_command_click`].
    pub on_command_click: Option<CommandClick>,
    /// What is called when the text is command-clicked, with where; see [`RawFieldConfig::on_command_click_at`].
    pub on_command_click_at: Option<CommandClickAt>,
    /// What is asked what to add to the context menu; see [`RawFieldConfig::context_menu_extras`].
    pub context_menu_extras: Option<crate::input_field::context_menu::ContextMenuExtender>,
    /// The look of the context menu of a desktop; see [`RawFieldConfig::context_menu_style`].
    pub context_menu_style: Option<aimer_ctxmenu::ContextMenuStyle>,
    /// What says which bytes are links; see [`RawFieldConfig::link_range`].
    pub link_range: Option<LinkRange>,
    /// What the mark of a located range is drawn in, if the app has a color for it; see [`RawFieldConfig::locate_color`].
    pub locate_color: Option<Color>,
    /// The range that something pointed at and is being marked: its bytes, and when the mark began; see [`RawTextField::pulse_phase`].
    pub pulse: Cell<Option<(usize, usize, AnimInstant)>>,
    /// The range that was asked to be shown, taken from the controller and waiting for the layout to put it in view.
    pub locating: Cell<Option<(usize, usize)>>,
    /// The last place the pointer was inside the field, for what depends on the pointer and a key and not on a move: the
    /// name under it is a link when the jump key goes down.
    pub hover_pos: Cell<Option<Vec2d>>,
    /// The last place the pointer was inside the field, wherever it was: what a scroll, which does not say where it is,
    /// is over.
    pub pointer_pos: Cell<Option<Vec2d>>,
    /// Whether the name under the pointer has to be found again by the next layout.
    pub pending_link: Cell<bool>,
    /// The name that is shown as a link: where its bytes start and end, and the text revision they are of.
    pub link: Cell<Option<(usize, usize, u64)>>,
    /// Where the text was command-clicked and the byte of it is not known yet, as `pending_force` is for a force click.
    pub pending_command: Cell<Option<Vec2d>>,
    /// Where the text was force-clicked and the byte of it is not known yet: the layout finds it, as it does for a click.
    pub pending_force: Cell<Option<Vec2d>>,
    /// How deep the press that is going on has got, so that a deep press is one force click and not one for each report.
    pub force_stage: Cell<aimer_events::element::PressureStage>,
    /// The bars beside lines; see [`RawFieldConfig::line_bars`].
    pub line_bars: Option<LineBars>,
    /// The labels after lines; see [`RawFieldConfig::line_labels`].
    pub line_labels: Option<crate::input_field::line_labels::LineLabels>,
    /// The notes drawn inside lines; see [`RawFieldConfig::inlays`].
    pub inlays: Option<crate::input_field::inlay::Inlays>,
    /// The lines under ranges of the text; see [`RawFieldConfig::underlines`].
    pub underlines: Option<crate::input_field::underline::Underlines>,
    /// Where the notes go, for the text and the font they were placed for: see [`Self::placed_inlays`].
    pub(super) inlay_memo: RefCell<Option<InlayMemo>>,
    /// Where each button of a label was drawn the last time, in logical pixels from the field's outer origin.
    pub label_hits: RefCell<Vec<LabelHit>>,
    /// Where each icon was drawn the last time, in logical pixels from the field's outer origin, and
    /// what pressing it does: what a press is judged by.
    pub marker_hits: RefCell<Vec<MarkerHit>>,
    /// Whether the press that has not been let go yet was on an icon, which keeps the release from the
    /// field.
    pub marker_pressed: Cell<bool>,
    /// Whether a press that began on what hangs from the caret has not been released.
    pub popup_pressed: Cell<bool>,
    /// The band behind the caret's line; see [`RawFieldConfig::current_line`].
    pub current_line: Option<Color>,
    /// Told when hover, focus or emptiness changes, so the chrome built around
    /// this field is rebuilt from them.
    visual: Option<Rc<VisualFlags>>,
    pub mouse_held: Cell<Option<PointerKey>>,
    /// Where the press of the drag in progress landed, as a grapheme offset, once
    /// it has been resolved against a layout. The cursor cannot say: a selection
    /// area reports the drag as an ordered range, which puts the cursor's anchor
    /// at the head when the drag goes backwards.
    pub drag_origin: Cell<Option<usize>>,
    /// The press of the drag in progress, while it has not yet been resolved to an
    /// offset: the position and the layout it was aimed at. Positions resolve when
    /// a frame draws, and moves that arrive before then replace the one queued
    /// click, so the press is set aside to stay the anchor.
    pub pending_press: RefCell<Option<(Vec2d, Option<Rc<EditableGeometry>>)>>,
    pub last_click_time: Cell<AnimInstant>,
    pub last_click_pos: Cell<Option<Vec2d>>,
    pub click_count: Cell<u8>,
    /// The anchor (a grapheme offset) a press made with shift held extends the selection from, while
    /// the press waits to be placed: it extends the selection to where it lands. Kept here and not
    /// only in the cursor, because a selection area writes its own state over the cursor's in the
    /// frame between, and a caret with nothing selected has no anchor to write back. Taken when the
    /// press is placed.
    pub shift_press: Cell<Option<usize>>,
    pub pending_click: Cell<Option<Vec2d>>,
    /// The last [`HighlightContext`], with the state it was built from, so the
    /// many geometry requests of one frame walk the text for it once.
    highlight_context_memo: RefCell<Option<(HighlightKey, HighlightContext)>>,
    /// The last [`DisplayText`], with the revision and obscuring it was built for.
    /// Building it walks every grapheme of the text, and the geometry requests of
    /// a frame ask for it before the geometry cache can answer, so without this
    /// a frame costs a pass over the whole text however little has changed.
    display_memo: RefCell<Option<(DisplayKey, Rc<DisplayText>)>>,
    /// The layout the user was looking at when `pending_click` arrived. A click
    /// is hit-tested against it, not against the layout of the frame that
    /// applies it: placing the cursor can reflow the text (a heading that grows
    /// when the cursor leaves it), and the same pixel would then name another line.
    pub pending_click_geometry: RefCell<Option<Rc<EditableGeometry>>>,
    /// How many bytes of text make the field lay out a window of it; see
    /// [`windowing`](super::windowing).
    pub(super) window_threshold: usize,
    /// The size of the box the text shows in, in physical pixels, as the last frame laid it
    /// out: what a caret has to be inside to be seen.
    pub(super) caret_viewport: Cell<Option<(f32, f32)>>,
    /// Where each hard line of the text is and how tall: the windowed layout's index.
    pub(super) line_index: RefCell<Option<Rc<crate::editable_text::LineIndex>>>,
    /// The hard lines of the window the geometry was last laid out for.
    pub(super) window_lines: Cell<Option<(usize, usize)>>,
    /// The text a windowed field lays out while a composition is in progress, with the
    /// controller revision it was made for.
    pub(super) composition_display_memo: RefCell<Option<(u64, TextSnapshot)>>,
    /// The line at the top of the viewport and how far into it, with the scroll they
    /// were taken at: what keeps the text where it is when the estimates above it are
    /// corrected.
    pub(super) window_anchor: Cell<Option<(usize, f32, f32)>>,
    pub scroll_x: Cell<f32>,
    pub scroll_y: Cell<f32>,
    pub scroll_y_extent: Cell<f32>,
    pub reveal_caret: Cell<bool>,
    pub preedit_text: Cell<String>,
    pub preedit_cursor: Cell<Option<(usize, usize)>>,
    pub ime_enabled: Cell<bool>,
    /// Whether a keyboard that was held back is still wanted.
    ///
    /// Shared with the timer that raises it, so that losing focus, or a rebuilt
    /// field that took this one's place and then lost it, cancels the raise.
    #[cfg(target_os = "ios")]
    keyboard_pending: RefCell<Rc<Cell<bool>>>,
    pub ime_cursor_area: Cell<Option<ImeCaretArea>>,
    pub padding: LayoutSpacing,
    /// Shared presentation state consumed by the retained caret child.
    pub(crate) caret_context: CaretContext,
    /// Scope used to wake the next native caret visibility transition.
    pub(crate) caret_scope: Option<ScopeId>,
    /// Physical canvas offset from the field origin to its text content.
    pub(crate) caret_origin: Cell<(f32, f32)>,
    /// The shape the open menu was raised in.
    pub(crate) menu_shape: Cell<Option<ContextMenuShape>>,
    /// Watches a finger for the hold that raises that menu.
    pub touch_hold: TouchHold,
    /// The shared selection participant installed by the enclosing area.
    pub(crate) selection: RefCell<Option<SelectionParticipant>>,
    /// A menu asked for by an event and raised by the next frame, once the
    /// deferred click has been resolved into a caret offset.
    pending_menu: Cell<Option<MenuOrigin>>,
    /// What the open menu is anchored to, so a verb that reshapes the
    /// selection can re-offer it in the same place.
    menu_origin: Cell<Option<MenuOrigin>>,
    /// What the open menu and this field share; a field made again takes it from the one before.
    menu_shared: RefCell<Rc<menu::MenuShared>>,
    /// Where a right-click was, waiting for the layout to say which byte is under it.
    menu_hit_at: Cell<Option<Vec2d>>,
    /// The byte that was under the pointer when the menu was asked for, once the layout has said.
    menu_hit: Cell<Option<usize>>,
    /// A fixed instant for tests, so a hold's five hundred milliseconds are
    /// exercised by handing in a time rather than by sleeping — the same way
    /// `gesture::recognize::tap` keeps its thresholds testable.
    #[cfg(test)]
    test_clock: Cell<Option<AnimInstant>>,
}

impl Rebuildable for RawTextField {
    #[inline]
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Carries interaction state a freshly built field has no way to relearn.
    ///
    /// Reconciliation replaces this element with a brand-new one on every
    /// ancestor rebuild — and typing itself is one, through `on_changed`'s
    /// `set_state`. [`RawTextField::new`] always starts unfocused, with the
    /// caret at the controller's current selection and no IME session, on the
    /// documented assumption that the enclosing focus region will deliver a
    /// fresh [`ElementEvent::FocusGained`] to teach it otherwise. That
    /// assumption only holds for a field that is *becoming* focused. When the
    /// field's `FocusNode` is already the frame's owner, ownership never
    /// changes, so the framework never re-fires the event — and without this,
    /// the field would silently stop accepting input after its very first
    /// rebuild: `focused` resets to `false`, and every key, character and IME
    /// handler gates on it before doing anything.
    ///
    /// [`ElementEvent::FocusGained`]: aimer_events::element::ElementEvent::FocusGained
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|value| value.downcast_ref::<Self>())
        else {
            return;
        };

        self.highlights.adopt(&old.highlights);
        self.adopt_menu_from(old);
        self.focused.set(old.focused.get());
        self.observed_revision.set(old.observed_revision.get());
        self.native_session.set(old.native_session.get());
        self.native_base_revision.set(old.native_base_revision.get());
        self.native_mirror_revision.set(old.native_mirror_revision.get());
        self.cursor.set_offset(old.cursor.offset());
        self.cursor.set_selection_anchor(old.cursor.selection_anchor());
        self.mouse_held.set(old.mouse_held.get());
        self.drag_origin.set(old.drag_origin.get());
        *self.pending_press.borrow_mut() = old.pending_press.borrow_mut().take();
        self.last_click_time.set(old.last_click_time.get());
        self.last_click_pos.set(old.last_click_pos.get());
        self.click_count.set(old.click_count.get());
        self.pending_click.set(old.pending_click.get());
        // A force click or a command click that the layout has not placed yet is waited for across the rebuild, or it is
        // lost: the layout is the frame after the press. The deep press that is going on stays one press, and the name
        // under the pointer stays a link until the pointer or the key says otherwise.
        self.pending_force.set(old.pending_force.get());
        self.pending_command.set(old.pending_command.get());
        self.force_stage.set(old.force_stage.get());
        self.hover_pos.set(old.hover_pos.get());
        self.pointer_pos.set(old.pointer_pos.get());
        // Copied, and not taken: a field is adopted from more than once while the pointer goes over what holds it, and one that was
        // left with nothing would show the same marks as new, and mark them again.
        self.marks_seen.replace(old.marks_seen.borrow().clone());
        self.marks_began.set(old.marks_began.get());
        // Copied, like the marks: what was told of is still told of, and is not told again by the field that replaces this one.
        self.hover_candidate.replace(old.hover_candidate.borrow().clone());
        self.hover_shown.replace(old.hover_shown.borrow().clone());
        self.hover_blocked.set(old.hover_blocked.get());
        self.pending_hover.set(old.pending_hover.get());
        self.pending_link.set(old.pending_link.get());
        self.link.set(old.link.get());
        self.pulse.set(old.pulse.get());
        self.locating.set(old.locating.get());
        *self.pending_click_geometry.borrow_mut() = old.pending_click_geometry.borrow_mut().take();
        *self.line_index.borrow_mut() = old.line_index.borrow().clone();
        self.window_lines.set(old.window_lines.get());
        self.window_anchor.set(old.window_anchor.get());
        self.scroll_x.set(old.scroll_x.get());
        self.scroll_y.set(old.scroll_y.get());
        self.scroll_y_extent.set(old.scroll_y_extent.get());
        self.reveal_caret.set(old.reveal_caret.get());
        self.ime_enabled.set(old.ime_enabled.get());
        #[cfg(target_os = "ios")]
        self.keyboard_pending
            .replace(Rc::clone(&old.keyboard_pending.borrow()));
        self.preedit_cursor.set(old.preedit_cursor.get());
        self.ime_cursor_area.set(old.ime_cursor_area.get());
        let selection = old.selection.borrow().as_ref().map(|selection| {
            selection.adopt(crate::editable_text::selection_source(&self.controller))
        });
        *self.selection.borrow_mut() = selection;

        // `String` is not `Copy`, so this one field is moved rather than
        // copied. Guard against the double-visit some stateful ancestors
        // perform (once eagerly materializing their adopted child, once more
        // walking the candidate tree): a first visit may have already moved a
        // live composition into `self`, and a second visit's now-drained
        // `old` must not erase it.
        let live = self.preedit_text.take();
        let incoming = old.preedit_text.take();
        self.preedit_text.set(if live.is_empty() { incoming } else { live });
    }
}

/// Caret rectangle reported to the platform input method, in logical window
/// coordinates.
///
/// The input method places its candidate window relative to this rectangle, so
/// it has to follow the caret as text is typed or scrolled.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ImeCaretArea {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl RawTextField {
    /// Smallest caret movement, in logical pixels, that is worth reporting to
    /// the platform input method.
    const IME_AREA_EPSILON: f32 = 0.1;

    /// Logical width of the caret rectangle reported to the platform input
    /// method.
    const IME_CARET_WIDTH: f32 = 1.0;

    /// Builds the element for `config`, painting its caret from `caret`.
    ///
    /// All runtime state starts empty, focus included: the element learns it is
    /// focused from the [`ElementEvent::FocusGained`] the enclosing focus region
    /// delivers, which is the same notification an autofocused field receives on
    /// its first frame.
    ///
    /// [`ElementEvent::FocusGained`]: aimer_events::element::ElementEvent::FocusGained
    pub(crate) fn new(
        config: RawFieldConfig,
        caret: CaretBlink,
        focus_node: FocusNode,
    ) -> Self {
        let caret_context = CaretContext::new(caret.clone());
        Self::with_caret_context(config, caret, caret_context, focus_node, None, None)
    }

    /// Builds a field using presentation state owned by its mounted widget.
    ///
    /// The shared context is what lets a retained custom caret continue to
    /// observe the replacement field after configuration reconciliation.
    pub(crate) fn with_caret_context(
        config: RawFieldConfig,
        caret: CaretBlink,
        caret_context: CaretContext,
        focus_node: FocusNode,
        selection: Option<SelectionParticipant>,
        caret_scope: Option<ScopeId>,
    ) -> Self {
        // A field that indents with Tab takes the key from focus traversal while it has focus.
        // Only a multiline, editable one: a single line has nothing to indent.
        focus_node.set_claims_tab(
            config.tab_spaces.is_some() && config.max_lines != Some(1) && !config.read_only,
        );
        let controller_attachment = config
            .controller
            .attach(|_, _| aimer_events::window::request_animation_frame());
        let observed_revision = config.controller.revision();
        let saved_scroll = config.controller.scroll_offset();
        let cursor = Cursor::with_blink(config.cursor_color.unwrap_or_default(), caret.clone());
        let (anchor, focus) = config.controller.selection_graphemes();
        cursor.set_offset(focus);
        cursor.set_selection_anchor((anchor != focus).then_some(anchor));
        Self {
            input_type: config.input_type,
            controller: config.controller,
            controller_attachment,
            observed_revision: Cell::new(observed_revision),
            native_session: Cell::new(0),
            native_base_revision: Cell::new(0),
            native_mirror_revision: Cell::new(0),
            geometry_cache: EditableGeometryCache::default(),
            highlighter: config.highlighter,
            highlights: HighlightCache::default(),
            embedder: config.embedder,
            embeds: Rc::new(EmbedStore::default()),
            embed_memo: RefCell::new(None),
            window_embed_memo: RefCell::new(None),
            display_widget: RetainedDisplay::default(),
            prompt: config.prompt,
            hint: config.hint,
            hint_style: config.hint_style,
            text_style: config.text_style,
            prompt_style: config.prompt_style,
            text_align: config.text_align,
            max_lines: config.max_lines,
            min_lines: config.min_lines,
            max_length: config.max_length,
            enable: config.enable,
            expand: config.expand,
            cursor,
            caret_color: config.cursor_color,
            decoration: config.decoration,
            hover_decoration: config.hover_decoration,
            focus_decoration: config.focus_decoration,
            disabled_decoration: config.disabled_decoration,
            selection_color: config.selection_color,
            selection_radius: config.selection_radius,
            focus_node,
            focused: Cell::new(false),
            hovered: Cell::new(false),
            cursor_claim: aimer_utils::cursor::CursorClaim::new(),
            retained_v2_paint_state: Cell::new(None),
            cached_bounds: aimer_widget::InteractionBounds::new(),
            on_changed: config.on_changed,
            on_submitted: config.on_submitted,
            on_focus: config.on_focus,
            on_blur: config.on_blur,
            read_only: config.read_only,
            key_filter: config.key_filter,
            text_filter: config.text_filter,
            caret_marks: config.caret_marks,
            hover: config.hover,
            hover_candidate: RefCell::new(None),
            hover_shown: RefCell::new(None),
            hover_blocked: Cell::new(false),
            pending_hover: Cell::new(false),
            marks_asked: RefCell::new(None),
            marks_seen: RefCell::new(Vec::new()),
            marks_began: Cell::new(None),
            line_break_prefix: config.line_break_prefix,
            tab_spaces: config.tab_spaces,
            wrap: config.wrap,
            clip_to_padding: config.clip_to_padding,
            line_numbers: config.line_numbers,
            gutter_markers: config.gutter_markers,
            pointer_holes: config.pointer_holes,
            on_force_click: config.on_force_click,
            on_force_click_at: config.on_force_click_at,
            on_command_click: config.on_command_click,
            on_command_click_at: config.on_command_click_at,
            context_menu_extras: config.context_menu_extras,
            context_menu_style: config.context_menu_style,
            link_range: config.link_range,
            locate_color: config.locate_color,
            pulse: Cell::new(None),
            locating: Cell::new(None),
            pending_command: Cell::new(None),
            hover_pos: Cell::new(None),
            pointer_pos: Cell::new(None),
            pending_link: Cell::new(false),
            link: Cell::new(None),
            pending_force: Cell::new(None),
            force_stage: Cell::new(aimer_events::element::PressureStage::None),
            line_bars: config.line_bars,
            line_labels: config.line_labels,
            inlays: config.inlays,
            underlines: config.underlines,
            inlay_memo: RefCell::new(None),
            label_hits: RefCell::new(Vec::new()),
            marker_hits: RefCell::new(Vec::new()),
            marker_pressed: Cell::new(false),
            popup_pressed: Cell::new(false),
            current_line: config.current_line,
            visual: config.visual,
            mouse_held: Cell::new(None),
            drag_origin: Cell::new(None),
            pending_press: RefCell::new(None),
            last_click_time: Cell::new(AnimInstant::now()),
            last_click_pos: Cell::new(None),
            click_count: Cell::new(0),
            shift_press: Cell::new(None),
            pending_click: Cell::new(None),
            highlight_context_memo: RefCell::new(None),
            display_memo: RefCell::new(None),
            pending_click_geometry: RefCell::new(None),
            window_threshold: crate::editable_text::WINDOWED_MIN_BYTES,
            caret_viewport: Cell::new(None),
            line_index: RefCell::new(None),
            window_lines: Cell::new(None),
            composition_display_memo: RefCell::new(None),
            window_anchor: Cell::new(None),
            // Where the text was left scrolled to, by the view of it before this one: and then the caret is not to be shown, which
            // would take it from there.
            scroll_x: Cell::new(saved_scroll.0),
            scroll_y: Cell::new(saved_scroll.1),
            scroll_y_extent: Cell::new(0.0),
            reveal_caret: Cell::new(saved_scroll == (0.0, 0.0)),
            preedit_text: Cell::new(String::new()),
            preedit_cursor: Cell::new(None),
            ime_enabled: Cell::new(false),
            #[cfg(target_os = "ios")]
            keyboard_pending: RefCell::new(Rc::new(Cell::new(false))),
            ime_cursor_area: Cell::new(None),
            padding: config.padding,
            caret_context,
            caret_scope,
            caret_origin: Cell::new((0.0, 0.0)),
            menu_shape: Cell::new(None),
            touch_hold: TouchHold::new(),
            selection: RefCell::new(selection),
            pending_menu: Cell::new(None),
            menu_origin: Cell::new(None),
            menu_shared: RefCell::new(Rc::default()),
            menu_hit_at: Cell::new(None),
            menu_hit: Cell::new(None),
            #[cfg(test)]
            test_clock: Cell::new(None),
        }
    }

    /// Whether the window position `pos` is over an embedded widget.
    ///
    /// Judged by the field's own bounds and the rectangles it published, not by
    /// what the widgets recorded, which a widget that has not been drawn yet has
    /// not.
    pub(crate) fn press_popup(&self, pos: Vec2d) -> bool {
        let Some(origin) = self.cached_bounds.get_bounds() else {
            return false;
        };
        self.caret_context.press_popup(pos.x - origin.x, pos.y - origin.y)
    }

    /// Tells what hangs from the caret that the pointer moved to `pos`, in the coordinates of the window.
    pub(crate) fn hover_popup(&self, pos: Vec2d) {
        if let Some(origin) = self.cached_bounds.get_bounds() {
            self.caret_context.hover_popup(pos.x - origin.x, pos.y - origin.y);
        }
    }

    /// Scrolls what hangs from the caret, if the pointer is on it. Whether it scrolled.
    pub(crate) fn scroll_popup(&self, delta_y: f32) -> bool {
        let (Some(pos), Some(origin)) = (self.pointer_pos.get(), self.cached_bounds.get_bounds()) else {
            return false;
        };
        self.caret_context.scroll_popup(pos.x - origin.x, pos.y - origin.y, delta_y)
    }

    /// Whether `pos`, in the coordinates of the window, is on what hangs from the caret.
    pub(crate) fn hole_at(&self, pos: Vec2d) -> bool {
        let (Some(holes), Some(bounds)) = (&self.pointer_holes, self.cached_bounds.get_bounds()) else {
            return false;
        };
        let (x, y) = (pos.x - bounds.x, pos.y - bounds.y);
        holes(bounds.width, bounds.height).into_iter().any(|(left, top, width, height)| x >= left && x < left + width && y >= top && y < top + height)
    }

    /// Whether `pos`, in the coordinates of the window, is on what hangs from the caret.
    pub(crate) fn popup_at(&self, pos: Vec2d) -> bool {
        // A press is told in the coordinates of the window, and what hangs from the caret in those of the field.
        let Some(origin) = self.cached_bounds.get_bounds() else {
            return false;
        };
        self.caret_context.popup_at(pos.x - origin.x, pos.y - origin.y)
    }

    /// Whether `pos`, in the coordinates of the window, is on an embedded widget.
    pub(crate) fn embed_at(&self, pos: Vec2d) -> bool {
        let Some(origin) = self.cached_bounds.get_bounds() else {
            return false;
        };
        self.embeds.published_contains(pos.x - origin.x, pos.y - origin.y)
    }

    /// Puts the cursor into the source of the embed under `pos`, on the line that
    /// matches how far down the embed the point is, so the user can edit it.
    ///
    /// An embed shows while the cursor is outside its range and gives way to the
    /// source when it is inside, so moving the cursor there is how a click edits.
    /// Returns whether there was an embed to enter.
    pub(crate) fn edit_embed_at(&self, pos: Vec2d) -> bool {
        let Some(origin) = self.cached_bounds.get_bounds() else {
            return false;
        };
        let (x, y) = (pos.x - origin.x, pos.y - origin.y);
        let Some(embed) = self.embeds.published_at(x, y) else {
            return false;
        };
        let Some(geometry) = self.geometry_cache.latest() else {
            return false;
        };
        let Some(range) = geometry.embeds.iter().find(|b| b.key == embed.key).map(|b| b.range.clone()) else {
            return false;
        };
        let source = &geometry.display[range.clone()];
        let mut lines = vec![range.start];
        lines.extend(source.match_indices('\n').map(|(newline, _)| range.start + newline + 1));
        let fraction = if embed.height > 0.0 { ((y - embed.y) / embed.height).clamp(0.0, 0.999) } else { 0.0 };
        let byte = lines[(fraction * lines.len() as f32) as usize];
        let grapheme = geometry.window.as_ref().map_or(0, |window| window.grapheme_base)
            + geometry.display[..byte].graphemes(true).count();
        self.cursor.set_offset(grapheme);
        self.cursor.set_selection_anchor(None);
        self.sync_selection_to_controller();
        self.reveal_caret.set(true);
        self.cursor.reset_blink();
        true
    }

    /// Tells the host where the embedded widgets' boxes were laid out, in logical
    /// pixels from the field's outer origin, the way the caret's rectangle is.
    /// An embed is part of the text, so it goes with it when the field scrolls
    /// sideways, `scroll_x` physical pixels, and is clipped to the box of the text.
    fn publish_embeds(
        &self,
        geometry: &EditableGeometry,
        multiline: bool,
        base_y: f32,
        scroll_x: f32,
        scroll_y: f32,
        scale: f32,
    ) {
        let rects = if multiline { geometry.embed_rects() } else { Vec::new() };
        if rects.is_empty() && !self.embeds.has_published() {
            return;
        }
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let (origin_x, origin_y) = self.caret_origin.get();
        let (view_width, view_height) = self.caret_viewport.get().unwrap_or((0.0, 0.0));
        let clip = (origin_x / scale, origin_y / scale, view_width / scale, view_height / scale);
        self.embeds.publish(
            rects
                .into_iter()
                .map(|rect| PublishedEmbed {
                    key: rect.key,
                    x: (origin_x + rect.x - scroll_x) / scale,
                    y: (origin_y + base_y + rect.top - scroll_y) / scale,
                    width: rect.width / scale,
                    height: rect.height / scale,
                    clip,
                })
                .collect(),
        );
    }

    /// Makes the field show its embedded widgets from `embeds`, the store its
    /// state keeps across rebuilds.
    #[inline]
    pub(crate) fn with_embeds(mut self, embeds: Rc<EmbedStore>) -> Self {
        self.embeds = embeds;
        self
    }

    /// Returns the live context consumed by the visual caret child.
    #[inline]
    pub(crate) fn caret_context(&self) -> CaretContext {
        self.caret_context.clone()
    }

    /// Publishes the field-local caret rectangle in logical pixels.
    #[inline]
    pub(crate) fn publish_caret(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        scale: f32,
    ) {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let (origin_x, origin_y) = self.caret_origin.get();
        // A caret on a line the field has scrolled away from is not on the field: the caret
        // is drawn by a child that is not clipped to the text's box, so it would be drawn
        // where the line would be, outside.
        let in_view = self.caret_viewport.get().is_none_or(|(view_width, view_height)| {
            y + height > 0.0 && y < view_height && x + width > 0.0 && x < view_width
        });
        self.caret_context.publish(
            CaretGeometry {
                x: (origin_x + x) / scale,
                y: (origin_y + y) / scale,
                width: width / scale,
                height: height / scale,
            },
            self.cursor.offset(),
            self.is_focused(),
            in_view && !self.is_composing(),
            self.is_composing(),
        );
        // Where the text ends on the field, which is where what hangs from the caret has to stay inside.
        self.caret_context.set_extent(
            self.caret_viewport.get().map(|(view_width, view_height)| ((origin_x + view_width) / scale, (origin_y + view_height) / scale)),
        );
    }

    /// Publishes the insertion point inside an active IME composition.
    #[inline]
    pub(crate) fn publish_composition_caret(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        scale: f32,
    ) {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let (origin_x, origin_y) = self.caret_origin.get();
        self.caret_context.publish(
            CaretGeometry {
                x: (origin_x + x) / scale,
                y: (origin_y + y) / scale,
                width: width / scale,
                height: height / scale,
            },
            self.cursor.offset(),
            self.is_focused(),
            true,
            true,
        );
    }

    /// The instant gestures are reckoned against.
    #[inline]
    fn now(&self) -> AnimInstant {
        #[cfg(test)]
        if let Some(now) = self.test_clock.get() {
            return now;
        }
        AnimInstant::now()
    }

    fn scaled_font_size(&self, style: &TextStyle, scale: f32) -> f32 {
        let fs = if style.font_size == 0 {
            14.0
        } else {
            style.font_size as f32
        };
        fs * scale
    }

    fn is_focused(&self) -> bool {
        self.focused.get()
    }

    /// Focuses or blurs the field and brings platform text input in sync.
    ///
    /// Gaining focus is what enables the input method — not the click that
    /// usually causes it — so a field focused by [`auto_focus`] or by code can
    /// compose immediately. Losing focus abandons any composition.
    ///
    /// [`auto_focus`]: crate::input_field::text_field::TextField::auto_focus
    fn set_focused(&self, focused: bool) {
        let was_focused = self.focused.replace(focused);
        self.sync_visual();
        if focused {
            // The view is not scrolled to the caret here. A field is told it has lost and regained
            // focus whenever its window is deactivated and activated again (full screen, switching
            // apps), and each time that would throw away where the user had scrolled to. What does
            // scroll to the caret asks for it: a click, a key, an edit, and the first layout.
            self.enable_platform_ime();
            return;
        }
        self.cursor.blink().cancel_scheduled_toggle();
        if was_focused {
            self.clear_preedit();
        }
        self.disable_platform_ime();
    }

    /// Turns platform text input on for this field.
    ///
    /// Idempotent: the platform is only told on the transition into the enabled
    /// state, because raising the keyboard and allowing IME are window-server
    /// round trips.
    fn enable_platform_ime(&self) {
        if self.ime_enabled.replace(true) {
            return;
        }
        #[cfg(target_os = "ios")]
        self.raise_ios_keyboard();
        #[cfg(target_os = "android")]
        android_keyboard::show_keyboard();
        #[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
        if let Some(w) = get_window() {
            w.set_ime_allowed(true);
        }
        #[cfg(target_arch = "wasm32")]
        wasm_request_keyboard(true, self.input_type);
    }

    /// Raises the iOS keyboard, now for a user's touch and after
    /// [`KEYBOARD_SETTLE_DELAY`] for a focus the frame settled by itself.
    ///
    /// The delayed raise runs on the field's caret scope, so it dies with the
    /// field, and only if the keyboard is still wanted when the timer fires.
    #[cfg(target_os = "ios")]
    fn raise_ios_keyboard(&self) {
        let delay = keyboard_delay(aimer_widget::focus_is_settling_between_events());
        let (Some(delay), Some(scope), Some(venus)) =
            (delay, self.caret_scope, aimer_venus::Venus::current())
        else {
            ios_keyboard::show_keyboard();
            return;
        };
        let pending = Rc::clone(&self.keyboard_pending.borrow());
        pending.set(true);
        venus.spawn_in(scope, async move {
            tokio::time::sleep(delay).await;
            if pending.replace(false) {
                ios_keyboard::show_keyboard();
            }
        });
    }

    /// Turns platform text input off and forgets the reported caret area.
    fn disable_platform_ime(&self) {
        self.ime_cursor_area.set(None);
        if !self.ime_enabled.replace(false) {
            return;
        }
        #[cfg(target_os = "ios")]
        {
            self.keyboard_pending.borrow().set(false);
            ios_keyboard::dismiss_keyboard();
        }
        #[cfg(target_os = "android")]
        android_keyboard::dismiss_keyboard();
        #[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
        if let Some(w) = get_window() {
            w.set_ime_allowed(false);
        }
        #[cfg(target_arch = "wasm32")]
        wasm_request_keyboard(false, self.input_type);
    }

    /// Reports `caret` to the platform so the candidate window follows the
    /// insertion point.
    ///
    /// The rectangle is submitted only once it has moved by at least
    /// [`Self::IME_AREA_EPSILON`] logical pixels: the field draws every blink
    /// frame, and telling the window server about an unchanged area on each of
    /// them is pure overhead.
    fn update_ime_cursor_area(&self, caret: ImeCaretArea) {
        // In the browser the caret is reported by moving the hidden input the
        // composition happens in, and that element is positioned against the
        // viewport rather than against the canvas. Converting before the
        // comparison also makes a scrolled or resized page resubmit the area,
        // because the canvas origin is part of what is being compared.
        #[cfg(target_arch = "wasm32")]
        let caret = ime_overlay_rect(caret, wasm_canvas_origin());

        if !self.ime_cursor_area_moved(caret) {
            return;
        }
        self.ime_cursor_area.set(Some(caret));
        #[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
        if let Some(w) = get_window() {
            use winit::dpi::{LogicalPosition, LogicalSize};
            w.set_ime_cursor_area(
                LogicalPosition::new(caret.x as f64, caret.y as f64),
                LogicalSize::new(caret.width.max(1.0) as f64, caret.height.max(1.0) as f64),
            );
        }
        #[cfg(target_arch = "wasm32")]
        wasm_place_ime_input(caret);
    }

    /// Returns whether `caret` differs from the last reported caret area.
    fn ime_cursor_area_moved(&self, caret: ImeCaretArea) -> bool {
        match self.ime_cursor_area.get() {
            Some(previous) => {
                (previous.x - caret.x).abs() > Self::IME_AREA_EPSILON
                    || (previous.y - caret.y).abs() > Self::IME_AREA_EPSILON
                    || (previous.width - caret.width).abs() > Self::IME_AREA_EPSILON
                    || (previous.height - caret.height).abs() > Self::IME_AREA_EPSILON
            }
            None => true,
        }
    }

    /// Converts a caret drawn at content-local canvas coordinates into the
    /// logical window rectangle the platform expects and reports it.
    fn publish_ime_caret(
        &self,
        local_x: f32,
        local_y: f32,
        height: f32,
        content_origin: (f32, f32),
        scale: f32,
    ) {
        if !self.is_focused() {
            return;
        }
        let scale = if scale > 0.0 { scale } else { 1.0 };
        self.update_ime_cursor_area(ImeCaretArea {
            x: (content_origin.0 + local_x) / scale,
            y: (content_origin.1 + local_y) / scale,
            width: Self::IME_CARET_WIDTH,
            height: height / scale,
        });
    }

    /// Mirrors the controller's selection into the canvas cursor.
    ///
    /// Unlike [`Self::sync_cursor_from_controller`] this does not push the
    /// state to the platform text editor: a change the editor itself reported
    /// must not be echoed back, or the echo races the user's next keystroke
    /// and resets the editor's buffer underneath it.
    fn sync_cursor_presentation(&self) {
        let (anchor, focus) = self.controller.selection_graphemes();
        self.cursor.set_offset(focus);
        self.cursor
            .set_selection_anchor((anchor != focus).then_some(anchor));
        self.reveal_caret.set(true);
        self.observed_revision.set(self.controller.revision());
        self.sync_selection_to_area();
        self.sync_visual();
    }

    fn sync_cursor_from_controller(&self) {
        self.sync_cursor_presentation();
        self.sync_platform_text_state();
    }

    /// Advances the sync epoch and re-anchors the platform text editor.
    ///
    /// A native delta that was consumed without producing a transaction — a
    /// no-op report, a Return key that submits instead of editing, offsets
    /// that no longer map — leaves the editor holding a buffer or a revision
    /// the controller did not adopt. Bumping the revision makes the refreshed
    /// snapshot outrank whatever revision the editor advanced speculatively,
    /// so the push is guaranteed to land and both sides converge again.
    fn rebase_native_editor(&self) {
        self.controller.bump_revision();
        self.observed_revision.set(self.controller.revision());
        self.sync_platform_text_state();
    }

    fn cursor_selection(&self) -> (usize, usize) {
        let focus = self.cursor.offset();
        (self.cursor.selection_anchor().unwrap_or(focus), focus)
    }

    /// Mirrors the field cursor into the shared byte-based selection session.
    fn sync_selection_to_area(&self) {
        // A windowed field has no layout of the lines outside its window to give an area:
        // it does not take part, and does not copy its text to say so.
        if self.windowed_display().is_some() {
            return;
        }
        let selection_state = self.selection.borrow();
        let Some(selection) = selection_state.as_ref() else {
            return;
        };
        let text = self.controller.text();
        selection.set_text(Rc::from(text.as_str()));
        if selection.active_pointer().is_some() {
            return;
        }
        let (anchor, focus) = self.cursor_selection();
        selection.set_range(
            grapheme_byte_offset(&text, anchor),
            grapheme_byte_offset(&text, focus),
        );
    }

    /// Mirrors a shared selection handle or cross-widget drag into the field.
    fn sync_cursor_from_area(&self) {
        if self.windowed_display().is_some() {
            return;
        }
        let selection_state = self.selection.borrow();
        let Some(selection) = selection_state.as_ref() else {
            return;
        };
        let text = self.controller.text();
        // Not `selected_range`: that one is ordered, and the caret belongs where the
        // pointer is, which for a drag that goes backwards is the start.
        let Some((anchor, focus)) = selection.selected_endpoints() else {
            return;
        };
        let anchor = grapheme_count(&text[..anchor.min(text.len())]);
        let focus = grapheme_count(&text[..focus.min(text.len())]);
        let anchor = (anchor != focus).then_some(anchor);
        // Asked on every layout pass, and the area usually says what the field already has. Then
        // nothing is done, and above all the view is not taken back to the caret: it would undo
        // where the user scrolled to each time the window is resized or activated again.
        if self.cursor.offset() == focus && self.cursor.selection_anchor() == anchor {
            return;
        }
        self.cursor.set_selection_anchor(anchor);
        self.cursor.set_offset(focus);
        self.controller.set_selection_graphemes(anchor.unwrap_or(focus), focus);
        self.observed_revision.set(self.controller.revision());
        self.reveal_caret.set(true);
    }

    /// Mirrors a canvas-driven cursor or selection change into the retained
    /// editing value and the platform editor.
    ///
    /// Pointer hit-testing and the clipboard menu resolve their offsets from
    /// the painted geometry, so they update [`Cursor`] directly. The controller
    /// remains the source of truth for native editor deltas and survives field
    /// rebuilds; leaving it stale makes the next native edit use the old range.
    fn sync_selection_to_controller(&self) {
        let (anchor, focus) = self.cursor_selection();
        if self.controller.set_selection_graphemes(anchor, focus) {
            self.observed_revision.set(self.controller.revision());
            self.sync_platform_text_state();
        }
        self.sync_selection_to_area();
    }

    fn replace_cursor_selection(&self, text: &str, max_length: Option<usize>) -> bool {
        let (anchor, focus) = self.cursor_selection();
        if !self
            .controller
            .replace_selection_graphemes(anchor, focus, text, max_length)
        {
            return false;
        }
        self.sync_cursor_from_controller();
        self.cursor.reset_blink();
        self.clear_preedit();
        self.on_changed.call_with(|| self.controller.text());
        true
    }


    fn delete_backward(&self) -> bool {
        let (anchor, focus) = self.cursor_selection();
        if !self.controller.delete_backward_graphemes(anchor, focus) {
            return false;
        }
        self.sync_cursor_from_controller();
        self.on_changed.call_with(|| self.controller.text());
        true
    }

    fn delete_forward(&self) -> bool {
        let (anchor, focus) = self.cursor_selection();
        if !self.controller.delete_forward_graphemes(anchor, focus) {
            return false;
        }
        self.sync_cursor_from_controller();
        self.on_changed.call_with(|| self.controller.text());
        true
    }

    fn move_left(&self, extend: bool) {
        let (anchor, focus) = self.cursor_selection();
        if self
            .controller
            .move_left_graphemes(anchor, focus, extend)
        {
            self.sync_cursor_from_controller();
        }
    }

    fn move_right(&self, extend: bool) {
        let (anchor, focus) = self.cursor_selection();
        if self
            .controller
            .move_right_graphemes(anchor, focus, extend)
        {
            self.sync_cursor_from_controller();
        }
    }


    /// Inserts `text` at the cursor as a single edit.
    ///
    /// The current selection, if any, is replaced first. A `max_length` limit
    /// truncates the payload instead of rejecting it, so a committed phrase
    /// still inserts as much as fits. The whole payload produces one undo
    /// entry, one cursor advance, and one `on_changed` call, which keeps undo
    /// granularity at the phrase the user actually typed.
    ///
    /// Returns whether the text changed.
    /// Puts the graphemes `start..end` on the clipboard, from the chunks they are in: a selection of
    /// megabytes is not made into a `String` on the way.
    fn copy_range(&self, start: usize, end: usize) {
        let value = self.controller.value();
        let from = value.byte_of_grapheme(start);
        let to = value.byte_of_grapheme(start.max(end));
        clipboard_write_pieces(&value.pieces(from..to));
    }

    /// [`insert_text`](Self::insert_text) for a text read from the clipboard. A long one goes into
    /// the field as the bytes the clipboard delivered, shared with the text and not copied; the
    /// rest, and every field that must change what is put in (a limit on its length, one line), take
    /// the text as a `str`.
    fn insert_shared(&self, text: &aimer_clipboard::clipboard::ClipboardText) -> bool {
        let whole: &str = (**text).as_ref();
        if whole.len() < SHARED_PASTE_MIN
            || self.max_length.is_some()
            || self.max_lines == Some(1)
            || self.is_composing()
            || self.read_only
        {
            return self.insert_text(whole);
        }
        let snapshot = TextSnapshot::from_shared(std::sync::Arc::clone(text));
        let (anchor, focus) = self.cursor_selection();
        if !self.controller.replace_selection_graphemes_snapshot(anchor, focus, &snapshot) {
            return false;
        }
        self.sync_cursor_from_controller();
        self.cursor.reset_blink();
        self.clear_preedit();
        self.on_changed.call_with(|| self.controller.text());
        true
    }

    /// Offers typed `text` to the text filter, if there is one. Whether it dealt with the text, which it did by editing
    /// through the controller: the caret of the field follows. What is being composed is not offered, as it is not
    /// text yet.
    fn filter_text(&self, text: &str) -> bool {
        let Some(filter) = &self.text_filter else {
            return false;
        };
        if self.is_composing() || !filter(text) {
            return false;
        }
        self.sync_cursor_from_controller();
        self.cursor.reset_blink();
        self.clear_preedit();
        self.on_changed.call_with(|| self.controller.text());
        true
    }

    fn insert_text(&self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let text = if self.max_lines == Some(1) {
            normalize_single_line(text)
        } else {
            Cow::Borrowed(text)
        };
        if self.is_composing() {
            if !self.controller.commit_composing(&text, self.max_length) {
                return false;
            }
            self.sync_cursor_from_controller();
            self.cursor.reset_blink();
            self.clear_preedit_presentation();
            self.on_changed.call_with(|| self.controller.text());
            return true;
        }
        self.replace_cursor_selection(&text, self.max_length)
    }

    fn is_hovered(&self) -> bool {
        self.hovered.get()
    }

    fn set_hovered(&self, hovered: bool) {
        self.hovered.set(hovered);
        self.sync_visual();
    }

    /// Reports hover, focus and emptiness to the chrome around this field.
    ///
    /// Cheap when nothing changed, and a no-op for a field without chrome.
    #[inline]
    fn sync_visual(&self) {
        if let Some(visual) = &self.visual {
            visual.sync(
                self.is_hovered(),
                self.is_focused(),
                self.controller.value().is_empty(),
            );
        }
    }

    fn active_decoration(&self) -> &BoxDecoration {
        if let Some(ref s) = self.disabled_decoration
            && !self.enable
        {
            return s;
        }

        if let Some(ref s) = self.focus_decoration
            && self.is_focused()
        {
            return s;
        }

        if let Some(ref s) = self.hover_decoration
            && self.is_hovered()
        {
            return s;
        }

        &self.decoration
    }


    fn outline_strokes(&self, box_width: f32, box_height: f32, scale: f32) -> (f32, f32, f32, f32) {
        self.active_decoration()
            .outline
            .strokes(box_width, box_height, scale)
    }


    /// The highlighter that applies to what the field displays: none for a
    /// secure field, whose bullets are not the text a highlighter was written for.
    #[inline]
    fn effective_highlighter(&self) -> Option<&TextHighlighter> {
        self.highlighter
            .as_ref()
            .filter(|_| !self.input_type.is_obscured())
    }

    /// Builds the widget that lays out and paints the field's displayed text,
    /// with the styled ranges `geometry` was shaped with.
    ///
    /// Highlighted text is laid out as a paragraph, which is expensive to
    /// prepare, so its widget is retained and shared by every caller of the
    /// frame. Plain text gets a fresh widget, as it always did.
    fn build_display_widget(
        &self,
        geometry: &Rc<EditableGeometry>,
        style: &TextStyle,
        align: TextAlign,
    ) -> Rc<RawTextWidget> {
        let build = || {
            let mut widget = self.build_text_widget(geometry.display.as_ref(), style, align);
            widget.ranges = geometry.ranges.clone();
            widget
        };
        // The widget the geometry was measured with has laid the text out already.
        if let Some(widget) = geometry.display_widget_for(style, align) {
            return widget;
        }
        if geometry.ranges.is_none() {
            return Rc::new(build());
        }
        self.display_widget.resolve(geometry, style, align, build)
    }

    fn build_text_widget(
        &'_ self,
        text: &str,
        style: &TextStyle,
        align: TextAlign,
    ) -> RawTextWidget {
        RawTextWidget {
            text: TextSource::Shared(Rc::from(text)),
            text_style: *style,
            text_align: align,
            line_height: Default::default(),
            text_indent: 0.0,
            ranges: None,
            cache: LayoutCache::new(),
            _typeface: Cell::new(None),
        }
    }

    // ── Word / line selection helpers ────────────────────────────────

    /// Select the word at the given grapheme offset using Unicode word
    /// boundaries.
    fn select_word_at(&self, grapheme_offset: usize) {
        use unicode_segmentation::UnicodeSegmentation;
        let value = self.controller.value();
        if value.is_empty() {
            return;
        }

        // Word boundaries are reported in bytes, so the click offset is
        // converted into byte space once and the matching bounds are converted
        // back into grapheme offsets. Counting `char`s here would land inside a
        // cluster and select the wrong word.
        let byte_offset = value.byte_of_grapheme(grapheme_offset);

        // The words are looked for in the text around the click: all of a short text, and a few
        // kilobytes either side of the click in a long one, since a word is not longer than that.
        const REACH: usize = 4096;
        let (from, to) = if value.len() <= 2 * REACH {
            (0, value.len())
        } else {
            (
                value.floor_to_boundary(byte_offset.saturating_sub(REACH)),
                value.ceil_to_boundary((byte_offset + REACH).min(value.len())),
            )
        };
        let window = value.slice(from..to);
        for (start, segment) in window.split_word_bound_indices() {
            let (start, end) = (from + start, from + start + segment.len());
            if byte_offset >= start && byte_offset < end {
                self.cursor.set_selection_anchor(Some(value.grapheme_of_byte(start)));
                self.cursor.set_offset(value.grapheme_of_byte(end));
                return;
            }
        }
    }

    /// Select the line (between newline characters) containing the given
    /// grapheme offset.
    fn select_line_at(&self, grapheme_offset: usize) {
        let value = self.controller.value();
        if value.is_empty() {
            return;
        }
        // The hard line the offset is on, from the rope: no walk over the text.
        let snapshot = value.snapshot();
        let byte = value.byte_of_grapheme(grapheme_offset.min(value.grapheme_count()));
        let mut line = snapshot.line_range(snapshot.byte_to_line(byte));
        // Without its line break: a line feed, or a carriage return and a line feed.
        if line.end > line.start && snapshot.byte_at(line.end - 1) == Some(b'\n') {
            line.end -= 1;
            if line.end > line.start && snapshot.byte_at(line.end - 1) == Some(b'\r') {
                line.end -= 1;
            }
        }
        self.cursor.set_selection_anchor(Some(value.grapheme_of_byte(line.start)));
        self.cursor.set_offset(value.grapheme_of_byte(line.end));
    }

    /// Adjust `scroll_x` so the cursor is visible within `content_width`.
    ///
    /// While an input method is composing, the end of the composition is the
    /// point the user is looking at, so the composition width is included;
    /// otherwise a long phrase would grow out of the clipped viewport.
    fn ensure_cursor_visible(
        &self,
        content_width: f32,
        geometry: &EditableGeometry,
        composition_width: f32,
    ) {
        let cursor_x = geometry
            .caret_geometry(self.cursor.offset())
            .map_or(0.0, |caret| caret.x);
        let composition_end = cursor_x + composition_width.max(0.0);
        let max_scroll = (geometry.text_width.max(composition_end) - content_width).max(0.0);
        let scroll = self.scroll_x.get().min(max_scroll);
        self.scroll_x.set(scroll);

        if cursor_x < scroll {
            self.scroll_x.set(cursor_x.max(0.0).min(max_scroll));
        } else if composition_end > scroll + content_width {
            self.scroll_x
                .set((composition_end - content_width).max(0.0).min(max_scroll));
        }
    }

    /// Builds the text presented to the renderer and preserves its source
    /// range for every displayed grapheme.
    ///
    /// An obscured field shows one bullet per grapheme cluster, so a family
    /// emoji hides behind a single dot and the bullets stay in step with the
    /// cursor offsets used for hit testing and caret placement. An active IME
    /// preedit is omitted from the committed presentation, so the mapping also
    /// skips that source range rather than shifting all following offsets.
    ///
    /// Built once per revision of the text and shared after that: see
    /// [`display_memo`](Self::display_memo).
    fn display_text_with_sources(&self) -> Rc<DisplayText> {
        let key = (self.controller.revision(), self.input_type.is_obscured());
        if let Some((memo_key, display)) = self.display_memo.borrow().as_ref()
            && *memo_key == key
        {
            return Rc::clone(display);
        }
        let display = Rc::new(self.build_display_text());
        *self.display_memo.borrow_mut() = Some((key, Rc::clone(&display)));
        display
    }

    fn build_display_text(&self) -> DisplayText {
        let value = self.controller.value();
        let source = value.text();
        let mut display = String::with_capacity(source.len());
        let mut source_ranges = Vec::new();
        let obscure = self.input_type.is_obscured();
        let append = |display: &mut String,
                      source_ranges: &mut Vec<std::ops::Range<usize>>,
                      segment: &str,
                      source_start: usize| {
            if !obscure && crate::editable_text::is_one_byte_graphemes(segment) {
                // Every byte is a grapheme: no need to walk the text to find them.
                display.push_str(segment);
                source_ranges.extend(
                    (source_start..source_start + segment.len()).map(|byte| byte..byte + 1),
                );
                return;
            }
            for (offset, grapheme) in segment.grapheme_indices(true) {
                if obscure {
                    display.push('\u{2022}');
                } else {
                    display.push_str(grapheme);
                }
                source_ranges.push(
                    source_start + offset..source_start + offset + grapheme.len(),
                );
            }
        };

        if let Some(composing) = value.composing() {
            append(
                &mut display,
                &mut source_ranges,
                &source[..composing.start()],
                0,
            );
            append(
                &mut display,
                &mut source_ranges,
                &source[composing.end()..],
                composing.end(),
            );
        } else {
            append(&mut display, &mut source_ranges, &source, 0);
        }

        DisplayText {
            display: Arc::from(display),
            source_ranges,
        }
    }

    /// The text as it is drawn.
    #[inline]
    fn display_text(&self) -> String {
        self.display_text_with_sources().display.as_ref().to_owned()
    }

    /// Selects the field's effective wrapping behavior. A text area owns line
    /// wrapping regardless of the general text style's default overflow mode;
    /// a single-line field keeps one unwrapped line and clips it through the
    /// field viewport.
    #[inline]
    fn wraps_text(&self) -> bool {
        self.max_lines != Some(1) && self.wrap
    }

    /// Whether the text scrolls up and down: it is a field of more than one line.
    #[inline]
    fn scrolls_vertically(&self) -> bool {
        self.max_lines != Some(1)
    }

    /// Whether the text scrolls sideways: its lines do not wrap, so a line can be wider than the
    /// field. A single-line field always does; a multi-line one when it was asked not to wrap.
    #[inline]
    fn scrolls_horizontally(&self) -> bool {
        !self.wraps_text()
    }

    /// How far the text is scrolled sideways, in physical pixels: none for a field that wraps.
    #[inline]
    fn scroll_x_offset(&self) -> f32 {
        if self.scrolls_horizontally() { self.scroll_x.get() } else { 0.0 }
    }

    /// Makes the text style used by the committed field painter. Alignment is
    /// supplied separately as a horizontal-only value so vertical placement
    /// remains under the field's scroll/reveal logic.
    #[inline]
    fn field_text_style(&self) -> TextStyle {
        let mut style = self.text_style;
        style.text_overflow = if self.wraps_text() {
            TextOverflow::Wrap
        } else {
            TextOverflow::Clip
        };
        style
    }

    /// Returns the horizontal part of the configured field alignment.
    #[inline]
    fn horizontal_text_align(&self) -> TextAlign {
        match self.text_align {
            TextAlign::TopLeft | TextAlign::MidLeft | TextAlign::BotLeft => TextAlign::TopLeft,
            TextAlign::TopCenter | TextAlign::MidCenter | TextAlign::BotCenter => {
                TextAlign::TopCenter
            }
            TextAlign::TopRight | TextAlign::MidRight | TextAlign::BotRight => {
                TextAlign::TopRight
            }
        }
    }

    /// Queues the pointer position of a drag that is under way. If the press is
    /// still waiting for a frame, it is set aside first, since this position
    /// replaces it in the queue and the drag must stay anchored at the press.
    #[inline]
    fn queue_drag_position(&self, position: Vec2d) {
        if self.drag_origin.get().is_none() && self.pending_press.borrow().is_none() {
            if let Some(press) = self.pending_click.get() {
                *self.pending_press.borrow_mut() =
                    Some((press, self.pending_click_geometry.borrow().clone()));
            }
        }
        self.queue_click(position);
    }

    /// Turns the press set aside by [`queue_drag_position`](Self::queue_drag_position)
    /// into the origin and anchor of the drag. The arguments are the frame's
    /// measurements, the same ones a queued click is resolved with.
    /// Tells what the text was force-clicked at, if it was since the last layout: the byte under the pointer, found with the
    /// same measurements a queued click is resolved with.
    fn resolve_force_click(
        &self,
        geometry: &Rc<EditableGeometry>,
        scale: f32,
        (abs_x, abs_y): (f32, f32),
        (pad_left, pad_top): (f32, f32),
        base_y: f32,
        is_multiline: bool,
    ) {
        for (pending, callback, with_spot) in [
            (&self.pending_force, &self.on_force_click, &self.on_force_click_at),
            (&self.pending_command, &self.on_command_click, &self.on_command_click_at),
        ] {
            let Some(pos) = pending.take() else {
                continue;
            };
            let x = pos.x * scale - abs_x - pad_left + self.scroll_x_offset();
            let y = pos.y * scale - abs_y - pad_top - base_y + if is_multiline { self.scroll_y.get() } else { 0.0 };
            // What is below the last line says nothing of the text, and a pointer there is not on it.
            let Some(grapheme) = geometry.hit_test(x, y) else {
                continue;
            };
            // The callbacks are given a byte of the text and are made on a copy of the handle, so that they may do what they
            // like with the field.
            let (callback, with_spot) = (callback.clone(), with_spot.clone());
            if callback.is_none() && with_spot.is_none() {
                continue;
            }
            let byte = self.controller.value().byte_of_grapheme(grapheme);
            // A command click is one on a name, as a link is: the app's when it says which are, and a word where it does
            // not. It is the byte of the name that is reported, wherever in the name the pointer was. A force click is on the
            // byte, whatever it is.
            let byte = if std::ptr::eq(pending, &self.pending_command) {
                let Some(name) = self.link_at(geometry, (x, y), byte) else {
                    continue;
                };
                name.start
            } else {
                byte
            };
            if let Some(with_spot) = with_spot {
                let bounds = self.cached_bounds.get_bounds();
                let origin = bounds.map_or((0.0, 0.0), |bounds| (bounds.x, bounds.y));
                with_spot(
                    byte,
                    crate::input_field::line_labels::FieldSpot {
                        x: pos.x - origin.0,
                        y: pos.y - origin.1,
                        width: 0.0,
                        height: 0.0,
                        extent: bounds.map(|bounds| (bounds.width, bounds.height)),
                    },
                );
            }
            if let Some(callback) = callback {
                callback(byte);
            }
        }
    }

    /// Finds the byte of the text under the place a right click was, once the layout can say, for the app that is asked what to add to the menu.
    /// Only what is under the pointer is found: the caret and the selection are not moved by it.
    #[allow(clippy::too_many_arguments)]
    fn resolve_menu_hit(
        &self,
        geometry: &Rc<EditableGeometry>,
        scale: f32,
        (abs_x, abs_y): (f32, f32),
        (pad_left, pad_top): (f32, f32),
        base_y: f32,
        is_multiline: bool,
    ) {
        let Some(pos) = self.menu_hit_at.take() else {
            return;
        };
        let x = pos.x * scale - abs_x - pad_left + self.scroll_x_offset();
        let y = pos.y * scale - abs_y - pad_top - base_y + if is_multiline { self.scroll_y.get() } else { 0.0 };
        // What is below the last line says nothing of the text, and a pointer there is not on it.
        let byte = geometry.hit_test(x, y).map(|grapheme| self.controller.value().byte_of_grapheme(grapheme));
        self.menu_hit.set(byte);
    }

    /// Whether anything is to be told of a click with the jump key held, so that the name under the pointer is a link while it is.
    pub(super) fn command_click_wanted(&self) -> bool {
        self.on_command_click.is_some() || self.on_command_click_at.is_some()
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_pending_press(
        &self,
        geometry: &Rc<EditableGeometry>,
        scale: f32,
        (abs_x, abs_y): (f32, f32),
        (pad_left, pad_top): (f32, f32),
        base_y: f32,
        is_multiline: bool,
    ) {
        let Some((press, seen)) = self.pending_press.borrow_mut().take() else {
            return;
        };
        let x = press.x * scale - abs_x - pad_left + self.scroll_x_offset();
        let y = press.y * scale - abs_y - pad_top - base_y + if is_multiline { self.scroll_y.get() } else { 0.0 };
        let seen = seen.unwrap_or_else(|| Rc::clone(geometry));
        let offset = seen.hit_test(x, y).unwrap_or_else(|| seen.grapheme_count());
        self.drag_origin.set(Some(offset));
        if self.mouse_held.get().is_some() && self.cursor.selection_anchor().is_none() {
            self.cursor.set_selection_anchor(Some(offset));
        }
    }

    /// Queues a pointer position to become a cursor offset when the next frame
    /// can measure, remembering the layout it was aimed at.
    #[inline]
    fn queue_click(&self, position: Vec2d) {
        self.pending_click.set(Some(position));
        *self.pending_click_geometry.borrow_mut() = self.geometry_cache.latest();
    }

    /// Where the cursor is in `display`, for a highlighter that styles by it.
    ///
    /// A field without focus has no cursor, so nothing is active and every line
    /// shows its rendered form.
    fn highlight_context(&self, display: Whole<'_>, index: Option<&LineIndex>) -> HighlightContext {
        if !self.is_focused() {
            return HighlightContext::default();
        }
        let dragging = self.drag_key();
        let key = (
            self.controller.revision(),
            self.cursor.offset(),
            self.cursor.selection_anchor(),
            dragging,
        );
        if let Some((memo_key, context)) = self.highlight_context_memo.borrow().as_ref()
            && *memo_key == key
        {
            return context.clone();
        }
        let context = self.build_highlight_context(display, index);
        *self.highlight_context_memo.borrow_mut() = Some((key, context.clone()));
        context
    }

    /// What a drag in progress contributes to a memo key: `None` with no button
    /// held, otherwise where the press landed (`usize::MAX` until it has resolved).
    #[inline]
    fn drag_key(&self) -> Option<usize> {
        self.mouse_held
            .get()
            .map(|_| self.drag_origin.get().unwrap_or(usize::MAX))
    }

    fn build_highlight_context(&self, display: Whole<'_>, index: Option<&LineIndex>) -> HighlightContext {
        let offset = self.cursor.offset();
        // While the button is held the head sweeps over whatever is under the
        // pointer, and an embed it enters would give way to its source, which moves
        // the layout under the pointer and sends the head somewhere else, again
        // and again for as long as the button stays down. A drag therefore decides
        // what shows by where it began; the selection it makes counts once it ends.
        let (from, to) = if self.mouse_held.get().is_some() {
            // The press, not the cursor's anchor: a selection area reports a
            // backwards drag as an ordered range, which leaves the anchor at the
            // head, inside the embed the drag is passing over.
            let origin = self.drag_origin.get().or(self.cursor.selection_anchor()).unwrap_or(offset);
            (origin, origin)
        } else {
            self.cursor.selection_range().unwrap_or((offset, offset))
        };
        // The index finds a grapheme's byte without a walk over the text from its start, and the
        // lines the cursor is on from its line counts: the text is not read, nor copied.
        if let Some(index) = index {
            return HighlightContext::in_snapshot(index.text(), index.byte_of_grapheme(from)..index.byte_of_grapheme(to));
        }
        let display = display();
        let byte = |grapheme: usize| match index {
            Some(index) => index.byte_of_grapheme(grapheme),
            None => display
                .grapheme_indices(true)
                .nth(grapheme)
                .map_or(display.len(), |(byte, _)| byte),
        };
        HighlightContext::new(&display, byte(from)..byte(to))
    }

    fn editable_geometry(
        &self,
        ctx: &BuildContext,
        content_width: f32,
    ) -> Rc<EditableGeometry> {
        if let Some(text) = self.windowed_display() {
            return self.windowed_geometry(ctx, content_width, text);
        }
        let font_size = self.scaled_font_size(&self.text_style, ctx.scale);
        let style = self.field_text_style();
        let display = self.display_text_with_sources();
        let language = self.controller.input_language();
        let wrap = self.wraps_text();
        let embedder = self.effective_embedder();
        let context = match self.effective_highlighter() {
            Some(highlighter) if highlighter.depends_on_selection() => {
                self.highlight_context(&|| std::borrow::Cow::Borrowed(&*display.display), None)
            }
            _ if embedder.is_some() => self.highlight_context(&|| std::borrow::Cow::Borrowed(&*display.display), None),
            _ => HighlightContext::default(),
        };
        let active = context.active_lines();
        let embed_boxes = self.embed_boxes(&|| std::borrow::Cow::Borrowed(&*display.display), &context, content_width, ctx);
        let embed_signature = embed_signature(embedder, &embed_boxes) ^ self.inlay_signature();
        let outer_language = ctx.canvas.text_language();
        ctx.canvas.set_text_language(language);
        let geometry = self.geometry_cache.resolve(
            EditableGeometryKey {
                revision: self.controller.revision(),
                font_size_bits: font_size.to_bits(),
                width_bits: content_width.to_bits(),
                obscure: self.input_type.is_obscured(),
                wrap,
                font_family: style.font_family,
                font_style: style.font_style,
                font_weight: style.font_weight.numeric(),
                text_transform: style.text_transform,
                letter_spacing_bits: style.letter_spacing.to_bits(),
                word_spacing_bits: style.word_spacing.to_bits(),
                language,
                highlighter: highlighter_id(self.effective_highlighter()),
                active_start: active.as_ref().map_or(usize::MAX, |lines| lines.start),
                active_end: active.as_ref().map_or(usize::MAX, |lines| lines.end),
                embeds: embed_signature,
                window: (0, 0),
            },
            || {
                let mut layout_ctx = ctx.clone();
                layout_ctx.parent_size = ResolvedSize {
                    width: content_width,
                    height: ctx.parent_size.height,
                };
                layout_ctx.box_constraint = aimer_attribute::BoxConstraint {
                    min_width: 0.0,
                    min_height: 0.0,
                    max_width: content_width,
                    max_height: ctx.box_constraint.max_height,
                };
                let ranges = self.highlights.resolve_in(
                    self.effective_highlighter(),
                    &display.display,
                    &context,
                );
                let ranges = with_embed_boxes(ranges, &embed_boxes, ctx.scale);
                let placed = self.placed_inlays(ctx, &display.display, 0, &embed_boxes);
                let ranges = inlay::with_gaps(ranges, &placed, 0, ctx.scale);
                let mut text_widget = self.build_text_widget(
                    display.display.as_ref(),
                    &style,
                    TextAlign::TopLeft,
                );
                text_widget.ranges = ranges.clone();
                let text_widget = Rc::new(text_widget);
                let interaction = text_widget.interaction_layout(&layout_ctx);
                let geometry = EditableGeometry::from_interaction(
                    Arc::clone(&display.display),
                    display.source_ranges.clone(),
                    (*interaction).clone(),
                    content_width,
                    self.horizontal_text_align(),
                )
                .with_ranges(ranges)
                .with_embeds(embed_boxes.clone())
                .with_inlays(placed, ctx.scale);
                // Styled text keeps the widget it was measured with for painting. Plain
                // text paints with a fresh one, as it always has.
                if geometry.ranges.is_some() {
                    geometry.with_display_widget(style, TextAlign::TopLeft, text_widget)
                } else {
                    geometry
                }
            },
        );
        ctx.canvas.set_text_language(outer_language);
        geometry
    }

    /// The embedder that applies to what the field displays: none for a secure
    /// field, whose bullets are not the text an embedder was written for.
    #[inline]
    fn effective_embedder(&self) -> Option<&TextEmbedder> {
        self.embedder.as_ref().filter(|_| !self.input_type.is_obscured())
    }

    /// The embeds for `display` with the cursor where it is, asked of the embedder
    /// once per text and cursor like the highlight context is.
    fn current_embeds(&self, display: Whole<'_>, context: &HighlightContext) -> Rc<[TextEmbed]> {
        let Some(embedder) = self.effective_embedder() else {
            return Rc::from(Vec::new());
        };
        let dragging = self.drag_key();
        let key = (
            self.controller.revision(),
            self.cursor.offset(),
            self.cursor.selection_anchor(),
            dragging,
        );
        if let Some((memo_key, embeds)) = self.embed_memo.borrow().as_ref()
            && *memo_key == key
            && self.is_focused() == context.selection().is_some()
        {
            return Rc::clone(embeds);
        }
        let embeds: Rc<[TextEmbed]> = embedder.embeds(&display(), context).into();
        *self.embed_memo.borrow_mut() = Some((key, Rc::clone(&embeds)));
        embeds
    }

    /// Reserves a box for each embed: the full width of the field, and the height
    /// the embed asks for, in physical pixels. Embeds that are out of the text, cut
    /// a character, or overlap an earlier one are left out.
    fn embed_boxes(
        &self,
        display: Whole<'_>,
        context: &HighlightContext,
        content_width: f32,
        ctx: &BuildContext,
    ) -> Vec<EmbedBox> {
        let embeds = self.current_embeds(display, context);
        let display = display();
        let mut boxes: Vec<EmbedBox> = Vec::with_capacity(embeds.len());
        let mut sorted: Vec<&TextEmbed> = embeds.iter().collect();
        sorted.sort_by_key(|embed| embed.range().start);
        let mut end = 0;
        for embed in sorted {
            let range = embed.range();
            if range.start >= range.end
                || range.start < end
                || range.end > display.len()
                || !display.is_char_boundary(range.start)
                || !display.is_char_boundary(range.end)
            {
                continue;
            }
            end = range.end;
            boxes.push(EmbedBox {
                key: embed.key(),
                range,
                width: content_width,
                height: self.embeds.height(embed, content_width, ctx),
            });
        }
        boxes
    }

    /// Count the number of lines in the text (newlines + 1).
    fn line_count(&self) -> usize {
        self.controller
            .text()
            .chars()
            .filter(|&c| c == '\n')
            .count()
            + 1
    }
}

#[cfg(test)]
mod grapheme_tests {
    //! Every offset the field exposes — cursor, selection, hit testing —
    //! counts grapheme clusters, so editing and drawing agree on
    //! multi-code-point text. These are the scripts an input method exists
    //! for, which is why they get their own suite.

    use aimer_events::element::{KeyAction, Modifiers, NamedKey};
    use aimer_widget::EventElement;

    use super::test_support::{commit, focused_field, key};
    use super::{ElementEvent, InputType};
    use crate::input_field::controller::TextFieldController;

    /// A single family emoji: five `char`s joined by zero-width joiners, one
    /// grapheme cluster.
    const FAMILY: &str = "👨‍👩‍👧";

    /// A shortcut press with the control modifier held.
    fn shortcut(letter: &str) -> ElementEvent {
        ElementEvent::KeyInput {
            key: NamedKey::Other(letter.to_owned()),
            action: KeyAction::Pressed,
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
        }
    }

    #[test]
    fn backspace_removes_a_whole_grapheme_cluster() {
        let controller = TextFieldController::with_initial(format!("a{FAMILY}"));
        let field = focused_field(controller.clone());
        field.cursor.set_offset(2);

        assert!(field.on_event(&key(NamedKey::Backspace)).is_consumed());

        assert_eq!(controller.text(), "a");
        assert_eq!(field.cursor.offset(), 1);
    }

    #[test]
    fn delete_removes_a_whole_grapheme_cluster() {
        let controller = TextFieldController::with_initial(format!("{FAMILY}a"));
        let field = focused_field(controller.clone());
        field.cursor.set_offset(0);

        assert!(field.on_event(&key(NamedKey::Delete)).is_consumed());

        assert_eq!(controller.text(), "a");
        assert_eq!(field.cursor.offset(), 0);
    }

    #[test]
    fn horizontal_movement_steps_over_whole_clusters() {
        let field = focused_field(TextFieldController::with_initial(format!("{FAMILY}a")));
        field.cursor.set_offset(0);

        let _ = field.on_event(&key(NamedKey::ArrowRight));
        assert_eq!(field.cursor.offset(), 1);
        let _ = field.on_event(&key(NamedKey::ArrowRight));
        assert_eq!(field.cursor.offset(), 2);
        // Already at the end: the offset must not run past the cluster count.
        let _ = field.on_event(&key(NamedKey::ArrowRight));
        assert_eq!(field.cursor.offset(), 2);

        let _ = field.on_event(&key(NamedKey::ArrowLeft));
        assert_eq!(field.cursor.offset(), 1);
    }

    #[test]
    fn end_and_select_all_reach_the_last_cluster() {
        let field = focused_field(TextFieldController::with_initial(format!("{FAMILY}a")));
        field.cursor.set_offset(0);

        let _ = field.on_event(&key(NamedKey::End));
        assert_eq!(field.cursor.offset(), 2);

        let _ = field.on_event(&shortcut("a"));
        assert_eq!(field.cursor.selection_anchor(), Some(0));
        assert_eq!(field.cursor.offset(), 2);
    }

    #[test]
    fn word_selection_uses_grapheme_offsets() {
        let field = focused_field(TextFieldController::with_initial(format!("{FAMILY} word")));

        // Clusters: family(0) space(1) w(2) o(3) r(4) d(5).
        field.select_word_at(2);

        assert_eq!(field.cursor.selection_anchor(), Some(2));
        assert_eq!(field.cursor.offset(), 6);
    }

    #[test]
    fn line_selection_uses_grapheme_offsets() {
        let field = focused_field(TextFieldController::with_initial(format!("{FAMILY}a\nbc")));

        // Clusters: family(0) a(1) newline(2) b(3) c(4).
        field.select_line_at(4);

        assert_eq!(field.cursor.selection_anchor(), Some(3));
        assert_eq!(field.cursor.offset(), 5);
    }

    #[test]
    fn a_key_filter_that_consumes_a_key_keeps_it_from_the_field() {
        let mut field = focused_field(TextFieldController::with_initial("hello"));
        field.cursor.set_offset(3);
        field.key_filter = Some(std::rc::Rc::new(|key, _| matches!(key, NamedKey::ArrowLeft)));

        assert!(field.on_event(&key(NamedKey::ArrowLeft)).is_consumed());
        assert_eq!(field.cursor.offset(), 3, "the filter claimed ArrowLeft");

        assert!(field.on_event(&key(NamedKey::ArrowRight)).is_consumed());
        assert_eq!(field.cursor.offset(), 4, "a key the filter declines is the field's own");
    }

    #[test]
    fn a_key_filter_does_not_see_releases() {
        let mut field = focused_field(TextFieldController::with_initial("hello"));
        let seen = std::rc::Rc::new(std::cell::Cell::new(0_u32));
        let observed = std::rc::Rc::clone(&seen);
        field.key_filter = Some(std::rc::Rc::new(move |_, _| {
            observed.set(observed.get() + 1);
            false
        }));

        let _ = field.on_event(&ElementEvent::KeyInput {
            key: NamedKey::ArrowLeft,
            action: KeyAction::Released,
            modifiers: Modifiers::default(),
        });
        assert_eq!(seen.get(), 0);

        let _ = field.on_event(&key(NamedKey::ArrowLeft));
        assert_eq!(seen.get(), 1);
    }

    fn typed(ch: char) -> ElementEvent {
        ElementEvent::CharInput { ch, action: KeyAction::Pressed, modifiers: Modifiers::default() }
    }

    #[test]
    fn a_text_filter_that_deals_with_the_text_keeps_it_from_the_field_and_the_field_follows_its_edit() {
        let controller = crate::TextEditingController::with_text("ab");
        let mut field = focused_field(controller.clone());
        controller.select_bytes(1, 1);
        let edited = controller.clone();
        field.text_filter = Some(std::rc::Rc::new(move |text| {
            if text != "(" {
                return false;
            }
            // A pair, with the caret between its two: one edit and then the caret.
            let range = edited.value().selection().range();
            let start = range.start;
            edited.replace_ranges(&[(range, "()")], 0);
            edited.select_bytes(start + 1, start + 1);
            true
        }));

        assert!(field.on_event(&typed('(')).is_consumed());
        assert_eq!(controller.text(), "a()b", "the pair, and not the key as well");
        assert_eq!(field.cursor.offset(), 2, "the caret is where the filter put it, between the two");

        // One undo takes the pair back.
        assert!(controller.undo());
        assert_eq!(controller.text(), "ab");
    }

    #[test]
    fn a_text_filter_that_declines_leaves_the_text_to_the_field_and_is_told_what_was_typed() {
        let mut field = focused_field(TextFieldController::with_initial(""));
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = std::rc::Rc::clone(&seen);
        field.text_filter = Some(std::rc::Rc::new(move |text| {
            log.borrow_mut().push(text.to_owned());
            false
        }));

        let _ = field.on_event(&typed('x'));
        let _ = field.on_event(&ElementEvent::TextInput { text: "\u{4f60}\u{597d}".to_owned(), action: KeyAction::Pressed, modifiers: Modifiers::default() });
        assert_eq!(field.controller.text(), "x\u{4f60}\u{597d}");
        assert_eq!(*seen.borrow(), ["x", "\u{4f60}\u{597d}"]);
    }

    #[test]
    fn a_text_filter_is_not_offered_a_release_and_a_field_that_cannot_be_edited_is_not_offered_anything() {
        let mut field = focused_field(TextFieldController::with_initial(""));
        let seen = std::rc::Rc::new(std::cell::Cell::new(0_u32));
        let counted = std::rc::Rc::clone(&seen);
        field.text_filter = Some(std::rc::Rc::new(move |_| {
            counted.set(counted.get() + 1);
            true
        }));

        let _ = field.on_event(&ElementEvent::CharInput { ch: 'x', action: KeyAction::Released, modifiers: Modifiers::default() });
        assert_eq!(seen.get(), 0, "a release types nothing");
        field.read_only = true;
        let _ = field.on_event(&typed('x'));
        assert_eq!(seen.get(), 0, "a read-only field takes no text, so it is not asked");
    }

    #[test]
    fn a_field_without_a_text_filter_inserts_what_is_typed_as_before() {
        let mut field = focused_field(TextFieldController::with_initial(""));
        let _ = field.on_event(&typed('('));
        assert_eq!(field.controller.text(), "(");
    }

    #[test]
    fn a_field_without_a_key_filter_handles_keys_as_before() {
        let mut field = focused_field(TextFieldController::with_initial("hello"));
        field.cursor.set_offset(3);
        assert!(field.on_event(&key(NamedKey::ArrowLeft)).is_consumed());
        assert_eq!(field.cursor.offset(), 2);
    }

    #[test]
    fn vertical_movement_walks_lines_in_graphemes() {
        // Clusters: family(0) newline(1) a(2) b(3) c(4) d(5). The first line is
        // a single cluster made of five `char`s, so counting `char`s misplaces
        // both the current column and the target line.
        let controller = TextFieldController::with_initial(format!("{FAMILY}\nabcd"));
        let mut field = focused_field(controller);
        field.max_lines = Some(2);
        field.cursor.set_offset(6);

        // Column 4 of the second line clamps to the end of the one-cluster
        // first line.
        let _ = field.on_event(&key(NamedKey::ArrowUp));
        assert_eq!(field.cursor.offset(), 1);

        // Back down into the second line, keeping the clamped column.
        let _ = field.on_event(&key(NamedKey::ArrowDown));
        assert_eq!(field.cursor.offset(), 3);
    }

    #[test]
    fn max_length_counts_clusters_not_chars() {
        let controller = TextFieldController::with_initial("a");
        let mut field = focused_field(controller.clone());
        field.max_length = Some(2);
        field.cursor.set_offset(1);

        assert!(field.on_event(&commit(&format!("{FAMILY}{FAMILY}"))).is_consumed());

        assert_eq!(controller.text(), format!("a{FAMILY}"));
        assert_eq!(field.cursor.offset(), 2);
    }

    #[test]
    fn single_line_commit_normalizes_all_line_separators_to_spaces() {
        let controller = TextFieldController::new();
        let mut field = focused_field(controller.clone());
        field.max_lines = Some(1);

        assert!(field.on_event(&commit("hello\r\n你好\rworld\nagain")).is_consumed());

        assert_eq!(controller.text(), "hello 你好 world again");
        assert_eq!(field.cursor.offset(), 20);
    }

    #[test]
    fn obscured_text_shows_one_bullet_per_cluster() {
        let mut field = focused_field(TextFieldController::with_initial(format!("{FAMILY}a")));
        field.input_type = InputType::Obscure;

        assert_eq!(field.display_text(), "••");
    }

    #[test]
    fn the_display_text_is_built_once_per_revision() {
        use std::rc::Rc;

        let controller = TextFieldController::with_initial("hello");
        let mut field = focused_field(controller.clone());

        let first = field.display_text_with_sources();
        assert!(
            Rc::ptr_eq(&first, &field.display_text_with_sources()),
            "asked again with the text unchanged, the same display text answers"
        );

        controller.set_text("hello world".to_string());
        let edited = field.display_text_with_sources();
        assert!(!Rc::ptr_eq(&first, &edited), "an edit builds it again");
        assert_eq!(&*edited.display, "hello world");
        assert_eq!(edited.source_ranges.len(), "hello world".len());

        field.input_type = InputType::Obscure;
        let obscured = field.display_text_with_sources();
        assert!(!Rc::ptr_eq(&edited, &obscured), "obscuring changes what is shown");
        assert_eq!(&*obscured.display, "•••••••••••");
    }

    /// What each displayed grapheme stands for, built one grapheme at a time.
    fn source_ranges_by_grapheme(text: &str) -> Vec<std::ops::Range<usize>> {
        use unicode_segmentation::UnicodeSegmentation;
        text.grapheme_indices(true)
            .map(|(start, grapheme)| start..start + grapheme.len())
            .collect()
    }

    #[test]
    fn ascii_display_text_maps_each_grapheme_to_its_source_range() {
        // Plain ASCII is a grapheme per byte, which the builder does not walk the text
        // to find out; a carriage return before a line feed is the exception.
        for text in ["", "hello", "tab\there\nline two\n\nend", "crlf\r\nline\r\n", "\r\n", "lone\rcr"] {
            let field = focused_field(TextFieldController::with_initial(text));
            let display = field.display_text_with_sources();
            assert_eq!(&*display.display, text);
            assert_eq!(display.source_ranges, source_ranges_by_grapheme(text), "{text:?}");
        }
    }

    #[test]
    fn ascii_display_text_around_a_composition_keeps_source_offsets() {
        let controller = crate::TextEditingController::with_text("abcd\r\nef");
        assert!(controller.update_composing_graphemes(2, 2, "XY"));
        assert_eq!(controller.value().composing().map(|range| (range.start(), range.end())), Some((2, 4)));
        let field = focused_field(controller);
        let display = field.display_text_with_sources();

        assert_eq!(&*display.display, "abcd\r\nef");
        let mut expected = source_ranges_by_grapheme("ab");
        expected.extend(
            source_ranges_by_grapheme("cd\r\nef")
                .into_iter()
                .map(|range| range.start + 4..range.end + 4),
        );
        assert_eq!(display.source_ranges, expected);
    }

    #[test]
    fn obscure_fields_do_not_copy_or_cut_selected_text() {
        let controller = TextFieldController::with_initial("secret");
        let mut field = focused_field(controller.clone());
        field.input_type = InputType::Obscure;
        field.cursor.set_selection_anchor(Some(0));
        field.cursor.set_offset(6);
        let cut = ElementEvent::KeyInput {
            key: NamedKey::Other("x".into()),
            action: KeyAction::Pressed,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        };

        assert!(field.on_event(&cut).is_consumed());

        assert_eq!(controller.text(), "secret");
        assert_eq!(field.cursor.selection_range(), Some((0, 6)));
    }

    #[test]
    fn obscure_preedit_masks_each_composing_grapheme_and_remaps_its_clause() {
        let preedit = format!("{FAMILY}a");

        let (display, cursor) = super::presentation_preedit(
            InputType::Obscure,
            &preedit,
            Some((FAMILY.len(), preedit.len())),
        );

        assert_eq!(display, "••");
        assert_eq!(cursor, Some((3, 6)));
    }

    #[test]
    fn truncation_counts_clusters_not_chars() {
        assert_eq!(super::truncate_graphemes("你好世界", 2), "你好");
        assert_eq!(super::truncate_graphemes("你好", 9), "你好");
        assert_eq!(super::truncate_graphemes("你好", 0), "");
        // Never splits a cluster: one unit of room takes the whole family.
        assert_eq!(
            super::truncate_graphemes(&format!("{FAMILY}{FAMILY}"), 1),
            FAMILY
        );
    }
}

#[cfg(test)]
mod caret_blink_tests {
    use std::time::Duration;

    use aimer_animation::AnimInstant;
    use aimer_events::element::ElementEvent;
    use aimer_events::text_editing::{NativeTextRange, TextEditingDelta};
    use aimer_widget::EventElement;

    use super::test_support::{commit, focused_field_with_caret, key};
    use crate::TextEditingController;
    use crate::input_field::caret::CaretBlink;
    use crate::input_field::controller::TextFieldController;

    const HALF: Duration = Duration::from_millis(500);

    /// Advances `caret` to the middle of its hidden half.
    fn hide(caret: &CaretBlink) {
        let start = AnimInstant::now();
        caret.tick(start);
        caret.tick(start + HALF);
    }

    #[test]
    fn caret_visibility_follows_the_shared_timeline() {
        let caret = CaretBlink::new();
        let field = focused_field_with_caret(TextFieldController::new(), caret.clone());

        assert!(field.cursor.is_visible());

        hide(&caret);

        assert!(!field.cursor.is_visible());
    }

    #[test]
    fn typing_keeps_the_caret_solid() {
        let caret = CaretBlink::new();
        let field = focused_field_with_caret(TextFieldController::new(), caret.clone());
        hide(&caret);

        assert!(field.on_event(&commit("a")).is_consumed());

        assert!(field.cursor.is_visible());
    }

    #[test]
    fn a_native_delta_keeps_the_caret_solid() {
        let controller = TextEditingController::new();
        let caret = CaretBlink::new();
        let field = focused_field_with_caret(controller.clone(), caret.clone());
        hide(&caret);

        // On a phone every keystroke arrives as a delta from the hidden
        // platform editor — it is typing, and typing keeps the caret solid.
        let delta = ElementEvent::TextEditingDelta(TextEditingDelta {
            session_id: field.native_session.get(),
            revision: controller.revision(),
            replacement: NativeTextRange::new(0, 0),
            replacement_text: "你".into(),
            selection: NativeTextRange::new(1, 1),
            composing: None,
        });
        assert!(field.on_event(&delta).is_consumed());

        assert!(field.cursor.is_visible());
    }

    #[test]
    fn moving_the_caret_keeps_it_solid() {
        let controller = TextFieldController::with_initial("hello");
        let caret = CaretBlink::new();
        let field = focused_field_with_caret(controller, caret.clone());
        field.cursor.set_offset(5);
        hide(&caret);

        assert!(
            field
                .on_event(&key(aimer_events::element::NamedKey::ArrowLeft))
                .is_consumed()
        );

        assert!(field.cursor.is_visible());
        assert_eq!(field.cursor.offset(), 4);
    }
}
