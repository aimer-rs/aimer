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
    pub cursor_color: Colors,
    pub caret_builder: Option<CaretBuilder>,
    pub on_changed: TextFieldCallback,
    pub on_submitted: TextFieldCallback,
    pub on_focus: TextFieldCallback,
    pub on_blur: TextFieldCallback,
    pub read_only: bool,
    pub key_filter: Option<KeyFilter>,
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

impl Widget for RawTextFieldWidget {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let cursor_color = self.config.cursor_color;
        let caret = self.caret_slot.build(
            self.config.caret_builder.as_ref(),
            self.caret_context.clone(),
            cursor_color,
            ctx,
        );
        let selection = SelectionParticipant::from_context(
            ctx,
            Rc::from(self.config.controller.text()),
        );
        let field = RawTextField::with_caret_context(
            self.config,
            self.caret,
            self.caret_context,
            self.focus_node,
            selection,
            self.caret_scope,
        );

        RawTextFieldHost::new(field, caret).boxed()
    }
}

impl aimer_widget::PortableWidget for RawTextFieldWidget {}

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
    /// Color supplied to the builtin caret and composition underlines.
    pub caret_color: Colors,
    pub decoration: BoxDecoration,
    pub hover_decoration: Option<BoxDecoration>,
    pub focus_decoration: Option<BoxDecoration>,
    pub disabled_decoration: Option<BoxDecoration>,
    pub selection_color: Color,
    pub focus_node: FocusNode,
    pub focused: Cell<bool>,
    pub hovered: Cell<bool>,
    pub(super) retained_v2_paint_state: Cell<Option<RetainedFieldPaintState>>,
    pub cached_bounds: aimer_widget::InteractionBounds,
    pub on_changed: TextFieldCallback,
    pub on_submitted: TextFieldCallback,
    pub on_focus: TextFieldCallback,
    pub on_blur: TextFieldCallback,
    pub read_only: bool,
    pub key_filter: Option<KeyFilter>,
    /// Told when hover, focus or emptiness changes, so the chrome built around
    /// this field is rebuilt from them.
    visual: Option<Rc<VisualFlags>>,
    pub mouse_held: Cell<Option<PointerKey>>,
    pub last_click_time: Cell<AnimInstant>,
    pub last_click_pos: Cell<Option<Vec2d>>,
    pub click_count: Cell<u8>,
    pub pending_click: Cell<Option<Vec2d>>,
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
    /// The open clipboard menu, or `None` while none is showing.
    ///
    /// The menu is a modal: the host places it above every clip and dismisses
    /// it on an outside press, so all the field keeps is the handle that closes
    /// it.
    pub(crate) menu: RefCell<Option<ModalHandle>>,
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
    /// The verbs of the open menu, in the order it draws them.
    menu_actions: Rc<RefCell<Vec<FieldAction>>>,
    /// The verb the open menu was just told to run.
    chosen_action: Rc<Cell<Option<FieldAction>>>,
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

        self.focused.set(old.focused.get());
        self.observed_revision.set(old.observed_revision.get());
        self.native_session.set(old.native_session.get());
        self.native_base_revision.set(old.native_base_revision.get());
        self.native_mirror_revision.set(old.native_mirror_revision.get());
        self.cursor.set_offset(old.cursor.offset());
        self.cursor.set_selection_anchor(old.cursor.selection_anchor());
        self.mouse_held.set(old.mouse_held.get());
        self.last_click_time.set(old.last_click_time.get());
        self.last_click_pos.set(old.last_click_pos.get());
        self.click_count.set(old.click_count.get());
        self.pending_click.set(old.pending_click.get());
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
            selection.adopt(Rc::from(self.controller.text()))
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
        let controller_attachment = config
            .controller
            .attach(|_, _| aimer_events::window::request_animation_frame());
        let observed_revision = config.controller.revision();
        let cursor = Cursor::with_blink(config.cursor_color, caret.clone());
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
            focus_node,
            focused: Cell::new(false),
            hovered: Cell::new(false),
            retained_v2_paint_state: Cell::new(None),
            cached_bounds: aimer_widget::InteractionBounds::new(),
            on_changed: config.on_changed,
            on_submitted: config.on_submitted,
            on_focus: config.on_focus,
            on_blur: config.on_blur,
            read_only: config.read_only,
            key_filter: config.key_filter,
            visual: config.visual,
            mouse_held: Cell::new(None),
            last_click_time: Cell::new(AnimInstant::now()),
            last_click_pos: Cell::new(None),
            click_count: Cell::new(0),
            pending_click: Cell::new(None),
            scroll_x: Cell::new(0.0),
            scroll_y: Cell::new(0.0),
            scroll_y_extent: Cell::new(0.0),
            reveal_caret: Cell::new(true),
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
            menu: RefCell::new(None),
            menu_shape: Cell::new(None),
            touch_hold: TouchHold::new(),
            selection: RefCell::new(selection),
            pending_menu: Cell::new(None),
            menu_origin: Cell::new(None),
            menu_actions: Rc::new(RefCell::new(Vec::new())),
            chosen_action: Rc::new(Cell::new(None)),
            #[cfg(test)]
            test_clock: Cell::new(None),
        }
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
        self.caret_context.publish(
            CaretGeometry {
                x: (origin_x + x) / scale,
                y: (origin_y + y) / scale,
                width: width / scale,
                height: height / scale,
            },
            self.cursor.offset(),
            self.is_focused(),
            !self.is_composing(),
            self.is_composing(),
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
            self.reveal_caret.set(true);
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
        let selection_state = self.selection.borrow();
        let Some(selection) = selection_state.as_ref() else {
            return;
        };
        let text = self.controller.text();
        let Some(range) = selection.selected_range() else {
            return;
        };
        let anchor = grapheme_count(&text[..range.start.min(text.len())]);
        let focus = grapheme_count(&text[..range.end.min(text.len())]);
        self.cursor.set_selection_anchor((anchor != focus).then_some(anchor));
        self.cursor.set_offset(focus);
        self.controller.set_selection_graphemes(anchor, focus);
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
        self.on_changed.call(&self.controller.text());
        true
    }


    fn delete_backward(&self) -> bool {
        let (anchor, focus) = self.cursor_selection();
        if !self.controller.delete_backward_graphemes(anchor, focus) {
            return false;
        }
        self.sync_cursor_from_controller();
        self.on_changed.call(&self.controller.text());
        true
    }

    fn delete_forward(&self) -> bool {
        let (anchor, focus) = self.cursor_selection();
        if !self.controller.delete_forward_graphemes(anchor, focus) {
            return false;
        }
        self.sync_cursor_from_controller();
        self.on_changed.call(&self.controller.text());
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
            self.on_changed.call(&self.controller.text());
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
                self.controller.value().text().is_empty(),
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
            cache: LayoutCache::new(),
            _typeface: Cell::new(None),
        }
    }

    // ── Word / line selection helpers ────────────────────────────────

    /// Select the word at the given grapheme offset using Unicode word
    /// boundaries.
    fn select_word_at(&self, grapheme_offset: usize) {
        use unicode_segmentation::UnicodeSegmentation;
        let text = self.controller.text();
        if text.is_empty() {
            return;
        }

        // Word boundaries are reported in bytes, so the click offset is
        // converted into byte space once and the matching bounds are converted
        // back into grapheme offsets. Counting `char`s here would land inside a
        // cluster and select the wrong word.
        let byte_offset = text
            .grapheme_indices(true)
            .nth(grapheme_offset)
            .map(|(byte, _)| byte)
            .unwrap_or(text.len());

        for (start, segment) in text.split_word_bound_indices() {
            let end = start + segment.len();
            if byte_offset >= start && byte_offset < end {
                self.cursor
                    .set_selection_anchor(Some(text[..start].graphemes(true).count()));
                self.cursor.set_offset(text[..end].graphemes(true).count());
                return;
            }
        }
    }

    /// Select the line (between newline characters) containing the given
    /// grapheme offset.
    fn select_line_at(&self, grapheme_offset: usize) {
        let text = self.controller.text();
        if text.is_empty() {
            return;
        }

        let graphemes = grapheme_slices(&text);
        let mut line_start = grapheme_offset.min(graphemes.len());
        let mut line_end = line_start;

        while line_start > 0 && graphemes[line_start - 1] != "\n" {
            line_start -= 1;
        }
        while line_end < graphemes.len() && graphemes[line_end] != "\n" {
            line_end += 1;
        }

        self.cursor.set_selection_anchor(Some(line_start));
        self.cursor.set_offset(line_end);
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
    fn display_text_with_sources(&self) -> DisplayText {
        let value = self.controller.value();
        let source = value.text();
        let mut display = String::with_capacity(source.len());
        let mut source_ranges = Vec::new();
        let obscure = self.input_type.is_obscured();
        let append = |display: &mut String,
                      source_ranges: &mut Vec<std::ops::Range<usize>>,
                      segment: &str,
                      source_start: usize| {
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
            append(&mut display, &mut source_ranges, source, 0);
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
        self.max_lines != Some(1)
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

    fn editable_geometry(
        &self,
        ctx: &BuildContext,
        content_width: f32,
    ) -> Rc<EditableGeometry> {
        let font_size = self.scaled_font_size(&self.text_style, ctx.scale);
        let style = self.field_text_style();
        let display = self.display_text_with_sources();
        let language = self.controller.input_language();
        let wrap = self.wraps_text();
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
                let text_widget = self.build_text_widget(
                    display.display.as_ref(),
                    &style,
                    TextAlign::TopLeft,
                );
                let interaction = text_widget.interaction_layout(&layout_ctx);
                EditableGeometry::from_interaction(
                    display.display,
                    display.source_ranges,
                    (*interaction).clone(),
                    content_width,
                    self.horizontal_text_align(),
                )
            },
        );
        ctx.canvas.set_text_language(outer_language);
        geometry
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
