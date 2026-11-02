use std::cell::RefCell;
use std::rc::Rc;

use aimer_attribute::{Bounds, ResolvedSize};
use aimer_events::element::{ElementEvent, KeyAction, NamedKey};
use aimer_events::pointer::{PointerButton, PointerSource};
use aimer_rubick::Shared;
use aimer_style::{LineHeight, TextAlign, TextOverflow, TextShadow, TextStyle, TextTransform};
use aimer_utils::AnimInstant;
use aimer_utils::callback::{Callback, CallbackExecutor};
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, FocusNode, LayoutElement,
    PointerKey, RawFocusable, VisitorElement, Widget,
};

use crate::TextSource;
use crate::paragraph::{Paragraph, PreparedLayout, geometry};
use crate::selection::SelectionPoint;
use crate::selection::TextHitRegion;
use crate::selection::cursor::HoverCursor;
use crate::selection::selectable::{Selectable, SelectionBinding, SelectionScope, TextGeometry};
use crate::selection::session::{SelectionSession, SelectionSlot};
use crate::selection::touch_hold::{TouchHold, TouchHoldGate, enter_hold, press_touch};
use crate::selection::ui;
use crate::text_span::TextSpan;

/// Callback invoked with the target of an activated linked [`TextSpan`].
pub type LinkCallback = Callback<Rc<str>, ()>;

pub(crate) const DEFAULT_SELECTION_COLOR: Color = Color::Rgba(51, 153, 255, 96);

/// Displays a tree of styled [`TextSpan`] values with optional links and
/// selection.
///
/// A span's style is resolved over the widget's base [`TextStyle`]. The widget
/// defaults to the style's overflow mode, default alignment, no link callback,
/// and disabled selection. Wrapping lays text onto multiple lines; ellipsis
/// truncates the first line to the available width. Selectable text supports
/// pointer selection and the platform select-all and copy shortcuts.
/// Transformation, letter spacing, word spacing, decoration, and glyph shadow
/// are run-level style values: the base [`TextStyle`] is inherited and
/// [`SpanStyle`] can override each value for a nested span. Line height and
/// first-line indentation are paragraph values shared by every span. A text
/// transform can change the number of rendered graphemes, but source ranges
/// remain tied to the original span text for selection and links.
///
/// # Example
///
/// ```
/// use aimer_text::RichText;
/// use aimer_text::text_span::TextSpan;
///
/// let text = RichText::new(
///     TextSpan::new("Read ").child(TextSpan::new("the guide").link("/guide")),
/// )
/// .on_link(|target| println!("open {target}"))
/// .selectable()
/// .wrapped();
/// ```
pub struct RichText {
    span: Shared<TextSpan>,
    text_style: TextStyle,
    overflow: Option<TextOverflow>,
    text_align: TextAlign,
    line_height: LineHeight,
    text_indent: f32,
    on_link: LinkCallback,
    link_hover_color: Option<Color>,
    selectable: bool,
    selection_color: Option<Color>,
}

impl RichText {
    /// Creates rich text rooted at `span` with default base style and
    /// interaction settings.
    #[inline]
    pub fn new(span: TextSpan) -> Self {
        Self::from_shared_parts(
            Shared::new(span),
            TextStyle::default(),
            None,
            TextAlign::default(),
        )
    }

    fn from_shared_parts(
        span: Shared<TextSpan>,
        text_style: TextStyle,
        overflow: Option<TextOverflow>,
        text_align: TextAlign,
    ) -> Self {
        Self {
            span,
            text_style,
            overflow,
            text_align,
            line_height: LineHeight::default(),
            text_indent: 0.0,
            on_link: LinkCallback::default(),
            link_hover_color: None,
            selectable: false,
            selection_color: None,
        }
    }

    /// Replaces the base style inherited by spans that do not override
    /// individual attributes.
    #[inline]
    pub fn text_style(mut self, text_style: TextStyle) -> Self {
        self.text_style = text_style;
        self
    }

    /// Sets the Unicode transformation inherited by spans without an
    /// explicit override.
    #[inline]
    pub fn text_transform(mut self, text_transform: TextTransform) -> Self {
        self.text_style.text_transform = text_transform;
        self
    }

    /// Sets the additional advance between adjacent rendered graphemes.
    #[inline]
    pub fn letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.text_style.letter_spacing = letter_spacing;
        self
    }

    /// Sets the additional advance at whitespace boundaries.
    #[inline]
    pub fn word_spacing(mut self, word_spacing: f32) -> Self {
        self.text_style.word_spacing = word_spacing;
        self
    }

    /// Adds one glyph shadow inherited by spans without an explicit override.
    #[inline]
    pub fn text_shadow(mut self, text_shadow: TextShadow) -> Self {
        self.text_style.text_shadow = Some(text_shadow);
        self
    }

    /// Removes the glyph shadow inherited by spans without an explicit
    /// override.
    #[inline]
    pub fn without_text_shadow(mut self) -> Self {
        self.text_style.text_shadow = None;
        self
    }

    /// Sets the alignment of each laid-out line within the available width.
    #[inline]
    pub fn text_align(mut self, text_align: TextAlign) -> Self {
        self.text_align = text_align;
        self
    }

    /// Sets the distance between adjacent paragraph baselines.
    #[inline]
    pub fn line_height(mut self, line_height: LineHeight) -> Self {
        self.line_height = line_height;
        self
    }

    /// Sets the first-line paragraph indent in logical pixels.
    ///
    /// A negative value is a hanging indent: the first line starts before the
    /// paragraph's normal origin while subsequent lines use the normal origin.
    #[inline]
    pub fn text_indent(mut self, text_indent: f32) -> Self {
        self.text_indent = text_indent;
        self
    }

    /// Overrides overflow behavior independently of the base style.
    #[inline]
    pub fn text_overflow(mut self, text_overflow: TextOverflow) -> Self {
        self.overflow = Some(text_overflow);
        self
    }

    fn resolved_overflow(&self) -> TextOverflow {
        self.overflow.unwrap_or(self.text_style.text_overflow)
    }

    /// Configures spans to wrap onto additional lines when width is
    /// constrained.
    #[inline]
    pub fn wrapped(self) -> Self {
        self.text_overflow(TextOverflow::Wrap)
    }

    /// Configures overflowing content to truncate the first line with an
    /// ellipsis.
    #[inline]
    pub fn ellipsis(self) -> Self {
        self.text_overflow(TextOverflow::Ellipsis)
    }

    /// Sets the callback invoked after a primary click completes on a linked
    /// span.
    ///
    /// The callback receives the link target stored by [`TextSpan::link`].
    /// Dragging to select text suppresses link activation.
    #[inline]
    pub fn on_link(mut self, on_link: impl Into<LinkCallback>) -> Self {
        self.on_link = on_link.into();
        self
    }

    /// Changes linked text to `color` while the mouse pointer is over it.
    pub const fn link_hover_color(mut self, color: Color) -> Self {
        self.link_hover_color = Some(color);
        self
    }

    /// Enables pointer selection plus select-all and copy keyboard shortcuts.
    pub const fn selectable(mut self) -> Self {
        self.selectable = true;
        self
    }

    /// Replaces the highlight color used for selected text.
    ///
    /// This does not by itself enable selection; call [`RichText::selectable`]
    /// as well.
    pub const fn selection_color(mut self, color: Color) -> Self {
        self.selection_color = Some(color);
        self
    }
}

impl Widget for RichText {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let spans = self.span.flatten(&self.text_style);
        let scope = ctx.get_state::<SelectionScope>();
        let selectable = self.selectable || scope.is_some();
        let plain_text: Rc<str> = if selectable {
            spans
                .iter()
                .map(|span| span.text.as_ref())
                .collect::<String>()
                .into()
        } else {
            Rc::from("")
        };
        let binding = SelectionBinding::new(
            ctx,
            Rc::clone(&plain_text),
            self.selection_color.unwrap_or(DEFAULT_SELECTION_COLOR),
        );
        let selection_color = self
            .selection_color
            .unwrap_or_else(|| binding.session.selection_color());
        RawRichText {
            paragraph: Paragraph::with_layout(
                spans,
                self.text_align,
                self.resolved_overflow(),
                self.line_height,
                self.text_indent,
            ),
            plain_text,
            on_link: self.on_link.clone(),
            link_hover_color: self.link_hover_color,
            selectable,
            selection_color,
            binding: RefCell::new(binding),
            link_regions: RefCell::new(Vec::new()),
            pressed_link: RefCell::new(None),
            hovered_link: RefCell::new(None),
            hover_cursor: HoverCursor::new(),
            touch_hold: TouchHoldGate::new(),
            focus_node: FocusNode::new(),
        }
        .focus_target()
    }
}

#[derive(Clone)]
struct LinkRegion {
    target: TextSource,
    bounds: Bounds,
}

/// The laid-out element produced by [`RichText`].
///
/// This low-level exported type participates directly in layout, drawing,
/// links, and selection. Prefer constructing [`RichText`], which resolves the
/// span tree and initializes its interaction state correctly.
pub struct RawRichText {
    paragraph: Paragraph,
    plain_text: Rc<str>,
    on_link: LinkCallback,
    link_hover_color: Option<Color>,
    selectable: bool,
    selection_color: Color,
    binding: RefCell<SelectionBinding>,
    link_regions: RefCell<Vec<LinkRegion>>,
    pressed_link: RefCell<Option<TextSource>>,
    hovered_link: RefCell<Option<TextSource>>,
    hover_cursor: HoverCursor,
    /// Keeps a finger from selecting until it has rested; a mouse never waits.
    touch_hold: TouchHoldGate,
    /// The keyboard focus of a text that owns its selection.
    ///
    /// Inside a [`SelectionArea`](crate::SelectionArea) the region is the one
    /// that holds the focus, and this node is never attached to anything.
    focus_node: FocusNode,
}

struct LocalV2RichTextPaint {
    layout: Rc<PreparedLayout>,
    scale: f32,
    width: f32,
    height: f32,
}

impl RawRichText {
    /// The shared geometry this element writes while drawing.
    #[inline]
    fn geometry(&self) -> Rc<TextGeometry> {
        Rc::clone(&self.binding.borrow().geometry)
    }

    /// The session this element takes part in.
    #[inline]
    fn session(&self) -> Rc<SelectionSession> {
        Rc::clone(&self.binding.borrow().session)
    }

    /// This element's registration inside the session.
    #[inline]
    fn slot(&self) -> Rc<SelectionSlot> {
        Rc::clone(&self.binding.borrow().slot)
    }

    fn local_v2_paint_data(&self, ctx: &BuildContext) -> Option<LocalV2RichTextPaint> {
        if !self.paragraph.supports_retained_v2_rich_text() {
            return None;
        }
        let scale = ctx.scale;
        let width = self.paragraph.available_width(ctx);
        let height = ctx.parent_size.height;
        if !scale.is_finite()
            || scale <= 0.0
            || !width.is_finite()
            || !height.is_finite()
            || width < 0.0
            || height < 0.0
            || width > 1_000_000.0
            || height > 1_000_000.0
        {
            return None;
        }
        let layout = self.paragraph.layout_for_paint(ctx);
        if !layout.size.width.is_finite()
            || !layout.size.height.is_finite()
            || layout.size.width < 0.0
            || layout.size.height < 0.0
            || layout.size.width > 1_000_000.0
            || layout.size.height > 1_000_000.0
            || layout.line_heights.iter().any(|height| !height.is_finite())
            || layout.fragments.iter().any(|fragment| {
                !fragment.x.is_finite()
                    || !fragment.baseline.is_finite()
                    || !fragment.width.is_finite()
                    || !fragment.height.is_finite()
                    || fragment.width < 0.0
                    || fragment.height < 0.0
            })
        {
            return None;
        }
        Some(LocalV2RichTextPaint {
            layout,
            scale,
            width,
            height,
        })
    }

    fn retained_v2_hovered_link(
        &self,
        ctx: &BuildContext,
        data: &LocalV2RichTextPaint,
    ) -> Option<TextSource> {
        let (canvas_x, canvas_y) = ctx.canvas.get_transform_translation();
        let origin = self.geometry().origin(canvas_x, canvas_y, data.scale);
        self.paragraph.link_at_point(
            &data.layout,
            ctx.cursor_pos,
            aimer_attribute::Vec2d {
                x: origin.x * data.scale,
                y: origin.y * data.scale,
            },
            data.scale,
        )
    }

    /// Returns accessibility geometry from the last painted frame.
    ///
    /// The snapshot is built from the same source-aware Aimer layout used by
    /// selection, links, and painting. It is `None` before the element has
    /// painted or when the frame supplied an invalid affine transform.
    pub fn accessibility_snapshot(&self) -> Option<crate::TextAccessibilitySnapshot> {
        let binding = self.binding.borrow();
        binding
            .geometry
            .accessibility_snapshot(binding.slot.selected_range())
    }

    /// Makes this element the keyboard focus target of the selection it owns.
    ///
    /// Only the owner of a session has a keyboard to give: inside a region the
    /// focus is the region's, and attaching here would take it away from the
    /// element that can actually answer for the whole selection — so a
    /// participant is returned as it is, one element and nothing around it.
    ///
    /// A text that owns its selection is focusable exactly while it holds one:
    /// the keyboard belongs to the selection, and holding it is how such a text
    /// learns of a press it is never offered, since routing hit-tests and a
    /// press on another widget goes there and nowhere else. A selection begins
    /// and ends under the finger without anything rebuilding the paragraph,
    /// which is why that condition is a gate on the attachment rather than a
    /// [behavior](aimer_widget::FocusBehavior). The notification travels back
    /// down, so this element still answers [`ElementEvent::FocusLost`] itself.
    fn focus_target(self) -> AnyElement {
        let text = self.attached();
        if !(text.selectable && text.owns_session()) {
            return text.boxed();
        }
        let session = text.session();
        let node = text.focus_node.clone();
        RawFocusable::new(node, text.boxed())
            .focusable_when(move || session.is_focused())
            .boxed()
    }

    /// Hands this element's focus node to the session it owns, and returns it.
    ///
    /// The session asks for the focus when a selection starts and gives it up
    /// when the selection is dropped, so it needs the very node the tree offers
    /// as this text's target.
    fn attached(self) -> Self {
        if self.selectable && self.owns_session() {
            self.session().attach_focus_node(&self.focus_node);
        }
        self
    }

    /// Reports whether the element created its own session, which is the case
    /// outside a selection region.
    #[inline]
    fn owns_session(&self) -> bool {
        self.binding.borrow().owns_session
    }

    fn link_at(&self, x: f32, y: f32) -> Option<TextSource> {
        self.link_regions
            .borrow()
            .iter()
            .find(|region| {
                let b = region.bounds;
                b.x <= x && x <= b.x + b.width && b.y <= y && y <= b.y + b.height
            })
            .map(|region| region.target.clone())
    }

    fn set_hovered_link(&self, hovered_link: Option<TextSource>) {
        if *self.hovered_link.borrow() != hovered_link {
            *self.hovered_link.borrow_mut() = hovered_link;
            self.geometry().window().request_redraw();
        }
    }

    /// Fires the link callback for `target`.
    ///
    /// The paragraph holds no runtime handle, so an async callback goes to
    /// whichever runtime the frame is being built on.
    #[inline]
    fn execute_link(&self, target: TextSource) {
        self.on_link.execute(target.to_rc());
    }
}

impl VisitorElement for RawRichText {
    fn debug_name(&self) -> &'static str {
        "RawRichText"
    }
}

impl aimer_widget::Rebuildable for RawRichText {
    #[inline]
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Keeps the selection alive across a rebuild.
    ///
    /// A rebuilt paragraph is a brand-new element with a brand-new
    /// registration, so a selection anchored in the element being replaced would
    /// otherwise be orphaned. Adopting the registration — and pushing the new
    /// text into it, which clamps live endpoints — keeps a selection that spans
    /// this widget intact while its content changes.
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|value| value.downcast_ref::<Self>())
        else {
            return;
        };
        let adopted = {
            let old_binding = old.binding.borrow();
            if old_binding.owns_session != self.binding.borrow().owns_session {
                return;
            }
            old_binding.adopt(Rc::clone(&self.plain_text))
        };
        *self.binding.borrow_mut() = adopted;
        // The adopted session was pointed at the focus of the element being
        // replaced, which is about to be dropped; a live selection would stop
        // hearing about outside presses without this.
        if self.selectable && self.owns_session() {
            self.session().attach_focus_node(&self.focus_node);
        }
    }
}

impl EventElement for RawRichText {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        let hovered_link = match event {
            ElementEvent::PointerDown(info)
            | ElementEvent::PointerUp(info)
            | ElementEvent::PointerMove(info)
                if info.source == PointerSource::Mouse =>
            {
                self.link_at(info.x(), info.y())
            }
            ElementEvent::PointerExited(PointerSource::Mouse, _) | ElementEvent::Cancel => None,
            _ => self.hovered_link.borrow().clone(),
        };
        self.set_hovered_link(hovered_link.clone());

        // The selection's knobs and callout are painted over this text, so a
        // press that grabbed one must never reach the paragraph underneath.
        if self.selectable
            && let Some(result) = ui::intercept(&self.session(), event)
        {
            return result;
        }

        let cursor_claimed = if self.selectable || !self.link_regions.borrow().is_empty() {
            let geometry = self.geometry();
            // A press is a finger resting on a glyph, so one whose paragraph has
            // since slid is no longer a hold and must not be judged as one
            // below.
            self.touch_hold
                .forget_if_content_moved(geometry.painted_origin());
            let over_glyphs = event
                .pointer()
                .is_some_and(|info| geometry.hits_glyph(info.x(), info.y()));
            self.hover_cursor.apply(
                geometry.window(),
                event,
                self.selectable,
                hovered_link.is_some(),
                over_glyphs,
            )
        } else {
            false
        };

        match event {
            ElementEvent::PointerDown(info) => {
                let pos = info.pos;
                let pointer = PointerKey::new(info.source, info.id);
                if self.selectable && info.button == PointerButton::Secondary {
                    return ui::open_context_menu(
                        &self.session(),
                        &self.slot(),
                        &self.geometry(),
                        pos,
                        pointer,
                    )
                    .into();
                }
                let target = self.link_at(pos.x, pos.y);
                *self.pressed_link.borrow_mut() = target;
                if self.selectable {
                    let geometry = self.geometry();
                    // The offset lookup snaps to the nearest glyph, so a press
                    // that never touched this text — the tree broadcasts the
                    // presses nobody took — must be told apart by its bounds
                    // first, or it would start a selection from a click far
                    // away instead of dismissing the one on screen.
                    let inside = geometry.contains_point(pos.x, pos.y);
                    if inside && let Some(offset) = geometry.offset_at(pos.x, pos.y) {
                        if info.source == PointerSource::Touch {
                            // A finger means a scroll as often as a selection,
                            // so the press is only remembered until the hold has
                            // been earned — together with where this paragraph
                            // sat when it landed, the one evidence a later frame
                            // has that the page has moved on.
                            return press_touch(
                                &self.session(),
                                &self.touch_hold,
                                pointer,
                                offset,
                                pos,
                                geometry.painted_origin(),
                            );
                        }
                        self.session()
                            .begin(SelectionPoint::new(self.slot(), offset), pointer);
                        return EventResult::consumed().with_pointer_capture(pointer);
                    }
                    self.touch_hold.clear();
                    let session = self.session();
                    if session.active_pointer() != Some(pointer) {
                        session.clear();
                    }
                }
                self.pressed_link.borrow().is_some()
            }
            ElementEvent::PointerMove(info) if self.selectable => {
                let pos = info.pos;
                let pointer = PointerKey::new(info.source, info.id);
                match self.touch_hold.poll(pointer, pos, AnimInstant::now()) {
                    TouchHold::Entered(offset) => {
                        enter_hold(&self.session(), &self.slot(), offset, pointer);
                        // A hold that selects must not also follow the link it
                        // rested on.
                        self.pressed_link.borrow_mut().take();
                        return EventResult::consumed().with_pointer_capture(pointer);
                    }
                    // Still resting, or gone off scrolling: either way the
                    // gesture is not this element's yet.
                    TouchHold::Waiting | TouchHold::Abandoned => return false.into(),
                    TouchHold::Idle => {}
                }
                let session = self.session();
                if session.active_pointer() != Some(pointer) {
                    return cursor_claimed.into();
                }
                if session.extend_to_position(pos.x, pos.y, pointer) && session.was_dragged() {
                    self.pressed_link.borrow_mut().take();
                }
                return EventResult::consumed();
            }
            ElementEvent::PointerUp(info) => {
                let pos = info.pos;
                let pointer = PointerKey::new(info.source, info.id);
                if self.touch_hold.release_was_stationary(pointer, pos) {
                    let session = self.session();
                    session.end(pointer);
                    ui::offer_menu_after_gesture(&session, info.source);
                    self.pressed_link.borrow_mut().take();
                    return EventResult::consumed();
                }
                if let TouchHold::Entered(offset) =
                    self.touch_hold.poll(pointer, pos, AnimInstant::now())
                {
                    // Held still and let go: the word stays selected, and the
                    // link underneath is not followed.
                    let session = self.session();
                    enter_hold(&session, &self.slot(), offset, pointer);
                    session.end(pointer);
                    ui::offer_menu_after_gesture(&session, info.source);
                    self.pressed_link.borrow_mut().take();
                    return EventResult::consumed();
                }
                self.touch_hold.clear();
                let session = self.session();
                let selection_owned = self.selectable && session.active_pointer() == Some(pointer);
                let dragged = if selection_owned {
                    session.extend_to_position(pos.x, pos.y, pointer);
                    let dragged = session.was_dragged();
                    session.end(pointer);
                    ui::offer_menu_after_gesture(&session, info.source);
                    dragged
                } else {
                    false
                };
                if dragged {
                    self.pressed_link.borrow_mut().take();
                    return EventResult::consumed().with_pointer_release(pointer);
                }
                let pressed = self.pressed_link.borrow_mut().take();
                let released = self.link_at(pos.x, pos.y);
                if let (Some(pressed), Some(released)) = (pressed, released)
                    && pressed == released
                {
                    self.execute_link(released);
                    let result = EventResult::consumed();
                    return if selection_owned {
                        result.with_pointer_release(pointer)
                    } else {
                        result
                    };
                }
                let result = EventResult::from(false);
                return if selection_owned {
                    result.with_pointer_release(pointer)
                } else {
                    result
                };
            }
            // Something else took the keyboard, which is what a press anywhere
            // outside this text amounts to.
            ElementEvent::FocusLost if self.selectable && self.owns_session() => {
                self.session().blur();
                false
            }
            ElementEvent::PointerExited(_, _) | ElementEvent::Cancel => {
                self.pressed_link.borrow_mut().take();
                self.touch_hold.clear();

                if matches!(event, ElementEvent::Cancel) {
                    self.session().cancel();
                }
                false
            }
            ElementEvent::KeyInput {
                key: NamedKey::Other(key),
                action,
                modifiers,
            } if self.selectable
                && self.owns_session()
                && self.session().is_focused()
                && matches!(action, KeyAction::Pressed | KeyAction::Repeat)
                && (modifiers.ctrl || modifiers.meta) =>
            {
                match key.as_str() {
                    "a" => {
                        self.session().select_all();
                        true
                    }
                    "c" => {
                        let text = self.session().selected_text();
                        if text.is_empty() {
                            return false.into();
                        }
                        let _ = aimer_clipboard::clipboard::set_text(&text);
                        true
                    }
                    _ => false,
                }
            }
            _ => cursor_claimed,
        }
        .into()
    }
}

impl LayoutElement for RawRichText {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.paragraph.prepare_for_paint(ctx).size
    }

    fn invalidate_layout(&self) {
        self.paragraph.invalidate();
    }

    /// The box this text painted, grown to cover the knobs of a selection it
    /// owns.
    ///
    /// A knob is drawn outside the line it marks, and routing hit-tests, so a
    /// text that claimed only its glyphs would grow knobs no press could reach.
    /// Inside a region the region claims them instead — it encloses every
    /// participant, and the knobs of a selection spanning several of them are
    /// nobody's in particular.
    fn pos_start_end(&self) -> Option<(aimer_attribute::Vec2d, aimer_attribute::Vec2d)> {
        let bounds = self.geometry().pos_start_end();
        if !(self.selectable && self.owns_session()) {
            return bounds;
        }
        ui::hit_bounds_with_handles(&self.session(), bounds)
    }
}

impl Drawable for RawRichText {
    /// The paragraph's interaction rectangle is its laid-out box. The render
    /// tree supplies where that box is now, so geometry painted earlier follows
    /// a scroll.
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.paragraph.prepare_for_paint(ctx).size)
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.geometry().bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.geometry().bounds.disagreement("RawRichText")
    }

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        self.slot().stamp();
        self.sync_paint_geometry(ctx);
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let origin = self.geometry().origin(abs_x, abs_y, ctx.scale);
        if let Some((pointer, offset)) = self.touch_hold.poll_stationary(AnimInstant::now(), origin)
        {
            enter_hold(&self.session(), &self.slot(), offset, pointer);
            self.pressed_link.borrow_mut().take();
        }
        self.set_hovered_link(self.link_at(ctx.cursor_pos.x, ctx.cursor_pos.y));
        true
    }

    fn draw_local_v2_compatibility(&self, _ctx: &BuildContext) {
        if self.selectable && self.owns_session() {
            let session = self.session();
            ui::track_menu(&session);
            ui::track_handles(&session);
        }
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        let data = self.local_v2_paint_data(ctx)?;
        let outsets = self
            .paragraph
            .retained_v2_paint_outsets(&data.layout, data.scale);
        outsets
            .iter()
            .any(|outset| *outset > 0.0)
            .then_some(outsets)
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.local_v2_paint_data(ctx).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let data = self
            .local_v2_paint_data(ctx)
            .expect("RawRichText v2 support must be checked before painting");
        let hovered_link = self.retained_v2_hovered_link(ctx, &data);
        let outsets = self
            .paragraph
            .retained_v2_paint_outsets(&data.layout, data.scale);
        let origin_offset = aimer_attribute::Vec2d {
            x: outsets[0],
            y: outsets[1],
        };
        let canvas = aimer_canvas::Canvas::of(ctx);
        if self.paragraph.needs_clip() {
            canvas.push_clip(
                aimer_cupid::utilities::Rect::new(
                    origin_offset.x,
                    origin_offset.y,
                    data.width / data.scale,
                    data.height / data.scale,
                ),
                [0.0; 4],
            );
        }
        self.paragraph.record_retained_v2_backgrounds(
            &canvas,
            &data.layout,
            data.scale,
            origin_offset,
        );
        let selected_range = self
            .selectable
            .then(|| self.slot().selected_range().unwrap_or(0..0));
        if let Some(selection) = selected_range.as_ref() {
            let (red, green, blue, alpha) = self.selection_color.to_rgba();
            let draw_rect = |x: f32, y: f32, width: f32, height: f32| {
                canvas.fill_rect(
                    aimer_cupid::utilities::Rect::new(
                        x / data.scale + origin_offset.x,
                        y / data.scale + origin_offset.y,
                        width / data.scale,
                        height / data.scale,
                    ),
                    [red, green, blue, alpha],
                );
            };
            if let Some(layout) = data.layout.aimer_interaction.as_ref() {
                for rect in layout.selection_rects(selection.clone()) {
                    draw_rect(rect.x, rect.y, rect.width, rect.height);
                }
            } else {
                for rect in
                    geometry::selection_runs(&data.layout, selection.clone(), ctx.visible_rect)
                {
                    draw_rect(rect.x, rect.y, rect.width, rect.height);
                }
            }
        }
        self.paragraph.record_retained_v2_foreground(
            &canvas,
            &data.layout,
            data.scale,
            origin_offset,
            hovered_link.as_ref(),
            self.link_hover_color,
        );
        if self.paragraph.needs_clip() {
            canvas.pop_clip();
        }
        canvas.finish();
        let geometry = self.geometry();
        *geometry.retained_v2_link_hover.borrow_mut() = hovered_link;
        *geometry.retained_v2_selection.borrow_mut() = selected_range;
    }

    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        let Some(data) = self.local_v2_paint_data(ctx) else {
            return false;
        };
        let geometry = self.geometry();
        let current_selection = self
            .selectable
            .then(|| self.slot().selected_range().unwrap_or(0..0));
        if current_selection != *geometry.retained_v2_selection.borrow() {
            return true;
        }
        if self.link_hover_color.is_none() {
            return false;
        }
        self.retained_v2_hovered_link(ctx, &data).as_deref()
            != geometry.retained_v2_link_hover.borrow().as_deref()
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let Some(data) = self.local_v2_paint_data(ctx) else {
            return;
        };
        let transform = ctx.canvas.get_transform();
        let geometry = self.geometry();
        geometry.save_painted_bounds(
            data.scale,
            transform,
            data.layout.size.width,
            data.layout.size.height,
        );
        geometry.regions.borrow_mut().clear();
        geometry.set_shared_interaction_layout(
            data.layout.aimer_interaction.clone(),
            transform,
            data.scale,
        );
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        if self.selectable {
            if let Some(shared) = data.layout.aimer_interaction.as_ref() {
                let mut regions = geometry.regions.borrow_mut();
                for cluster in &shared.clusters {
                    let left = cluster.start_x.min(cluster.end_x);
                    let right = cluster.start_x.max(cluster.end_x);
                    let is_hard_break = shared
                        .text
                        .get(cluster.text_range.clone())
                        .is_some_and(|text| text == "\n" || text == "\r\n");
                    regions.push(TextHitRegion::new(
                        if is_hard_break {
                            cluster.text_range.start..cluster.text_range.start
                        } else {
                            cluster.text_range.clone()
                        },
                        Bounds::new(
                            (abs_x + left) / data.scale,
                            (abs_y + cluster.y) / data.scale,
                            if is_hard_break {
                                (shared.metrics.width - left).max(data.scale) / data.scale
                            } else {
                                (right - left) / data.scale
                            },
                            cluster.height / data.scale,
                        ),
                    ));
                }
            } else {
                geometry::hit_regions(
                    &data.layout,
                    abs_x,
                    abs_y,
                    data.scale,
                    ctx.visible_rect,
                    &mut geometry.regions.borrow_mut(),
                );
            }
        }
        let mut link_regions = self.link_regions.borrow_mut();
        link_regions.clear();
        self.paragraph
            .visit_link_fragments(&data.layout, ctx.visible_rect, |target, fragment| {
                link_regions.push(LinkRegion {
                    target: target.clone(),
                    bounds: Bounds::new(
                        (abs_x + fragment.x) / data.scale,
                        (abs_y + fragment.baseline - fragment.ascent) / data.scale,
                        fragment.width / data.scale,
                        fragment.height / data.scale,
                    ),
                });
            });
    }

    fn update(&self, ctx: &BuildContext) {
        // The text itself is painted by `paint_local_v2`. This traversal keeps
        // the geometry the interaction reads (painted bounds, hit and link
        // regions) current and drives the gesture state around it.
        self.slot().stamp();
        self.paragraph.prepare_for_paint(ctx);
        self.sync_paint_geometry(ctx);

        // Where this frame paints tells a resting finger from a page moving
        // under one, so the hold is polled once the origin is known.
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let origin = self.geometry().origin(abs_x, abs_y, ctx.scale);
        if let Some((pointer, offset)) = self.touch_hold.poll_stationary(AnimInstant::now(), origin)
        {
            enter_hold(&self.session(), &self.slot(), offset, pointer);
            self.pressed_link.borrow_mut().take();
        }

        self.set_hovered_link(self.link_at(ctx.cursor_pos.x, ctx.cursor_pos.y));

        // Inside a region the furniture is kept by the region, on behalf of
        // every participant. A standalone text has no region to do it. Neither
        // the knobs nor the callout are painted into this canvas: both go
        // through the modal host's overlay, clear of every clip.
        if self.selectable && self.owns_session() {
            let session = self.session();
            ui::track_menu(&session);
            ui::track_handles(&session);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Behaviour tests for [`super::RichText`] and [`super::RawRichText`].

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use aimer_attribute::{Bounds, Vec2d};
    use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_style::{LineHeight, TextAlign, TextOverflow, TextStyle, TextTransform};
    use aimer_widget::base::{Color, WindowHandle};
    use aimer_widget::{EventElement, FocusNode, PointerKey};

    use super::{DEFAULT_SELECTION_COLOR, LinkCallback, LinkRegion, RawRichText, SelectionBinding};
    use crate::TextSource;
    use crate::paragraph::Paragraph;
    use crate::selection::TextHitRegion;
    use crate::selection::selectable::{SelectionCoordinator, TextGeometry};
    use crate::selection::session::SelectionSession;
    use crate::text_span::{ResolvedTextSpan, layout_resolved_spans};

    /// What `text` records in its own retained list, after the `update` that
    /// establishes its layout and geometry.
    fn recorded_v2(
        text: &RawRichText,
        ctx: &aimer_widget::base::BuildContext<'_>,
    ) -> Vec<aimer_cupid::draw_cmd_v2::DrawCommand> {
        use aimer_widget::Drawable;
        let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
        let root = tree
            .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 400.0, 400.0))
            .unwrap();
        let node_context = tree.context(root).unwrap();
        ctx.with_local_v2_paint_context(node_context, |ctx| {
            assert!(text.can_paint_local_v2(ctx), "the text paints locally");
            text.paint_local_v2(ctx);
        });
        tree.draw_list_snapshot(root).unwrap().commands.to_vec()
    }

    #[test]
    fn rich_text_keeps_paragraph_values_with_the_widget() {
        let rich_text = super::RichText::new(crate::TextSpan::new("Aimer"))
            .text_style(TextStyle::new().text_transform(TextTransform::Lowercase))
            .line_height(LineHeight::Factor(1.5))
            .text_indent(12.0);

        assert_eq!(
            rich_text.text_style.text_transform,
            TextTransform::Lowercase
        );
        assert_eq!(rich_text.line_height, LineHeight::Factor(1.5));
        assert_eq!(rich_text.text_indent, 12.0);
    }

    /// Builds a standalone registration: its own session with a single slot,
    /// which is what a `RichText` outside a selection region gets.
    fn standalone_binding(
        window: &WindowHandle,
        coordinator: Rc<SelectionCoordinator>,
        text: Rc<str>,
    ) -> RefCell<SelectionBinding> {
        let session = SelectionSession::new(window.clone(), coordinator, DEFAULT_SELECTION_COLOR);
        let geometry = Rc::new(TextGeometry::new(window.clone()));
        let slot = session.register(text, Rc::downgrade(&geometry) as _);
        slot.stamp();
        RefCell::new(SelectionBinding {
            geometry,
            session,
            slot,
            owns_session: true,
        })
    }

    fn selectable_raw_text(on_link: LinkCallback) -> RawRichText {
        selectable_raw_text_with_coordinator(on_link, Rc::new(SelectionCoordinator::default()))
    }

    fn selectable_raw_text_with_coordinator(
        on_link: LinkCallback,
        selection_coordinator: Rc<SelectionCoordinator>,
    ) -> RawRichText {
        raw_text_with(
            on_link,
            selection_coordinator,
            Rc::from("élink"),
            vec![TextHitRegion::new(0..6, Bounds::new(0.0, 0.0, 20.0, 10.0))],
            Bounds::new(0.0, 0.0, 20.0, 10.0),
        )
    }

    /// Builds an element that behaves as if it had already painted `regions`
    /// inside `bounds`, which is what pointer handling and the session's
    /// geometric hit test read.
    fn raw_text_with(
        on_link: LinkCallback,
        selection_coordinator: Rc<SelectionCoordinator>,
        plain_text: Rc<str>,
        regions: Vec<TextHitRegion>,
        bounds: Bounds,
    ) -> RawRichText {
        let window = WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 100), 1.0);
        let text = RawRichText {
            paragraph: Paragraph::new(
                vec![ResolvedTextSpan::plain(
                    Rc::clone(&plain_text),
                    TextStyle::default(),
                )],
                TextAlign::TopLeft,
                TextOverflow::Clip,
            ),
            plain_text: Rc::clone(&plain_text),
            on_link,
            link_hover_color: Some(Color::Hex(0x388BFD)),
            selectable: true,
            selection_color: DEFAULT_SELECTION_COLOR,
            binding: standalone_binding(&window, selection_coordinator, plain_text),
            link_regions: RefCell::new(vec![LinkRegion {
                target: TextSource::from("https://aimer.dev"),
                bounds: Bounds::new(0.0, 0.0, 20.0, 10.0),
            }]),
            pressed_link: RefCell::new(None),
            hovered_link: RefCell::new(None),
            hover_cursor: crate::selection::cursor::HoverCursor::new(),
            touch_hold: crate::selection::touch_hold::TouchHoldGate::new(),
            focus_node: FocusNode::new(),
        };
        let geometry = text.geometry();
        *geometry.regions.borrow_mut() = regions;
        geometry
            .bounds
            .save(1.0, bounds.x, bounds.y, bounds.width, bounds.height);
        text.attached()
    }

    /// A primary mouse press at an absolute logical position.
    fn mouse_press(x: f32, y: f32) -> PointerInfo {
        PointerInfo::new(
            Vec2d { x, y },
            PointerSource::Mouse,
            0,
            PointerButton::Primary,
        )
    }

    /// The range of the element's own text that is selected.
    fn selected(text: &RawRichText) -> Option<std::ops::Range<usize>> {
        text.slot().selected_range()
    }

    #[test]
    fn rich_text_selection_is_opt_in() {
        let plain = super::RichText::new(crate::TextSpan::new("plain"));
        let selectable = super::RichText::new(crate::TextSpan::new("selectable")).selectable();

        assert!(!plain.selectable);
        assert!(selectable.selectable);
    }

    #[test]
    fn rich_text_selection_color_is_customizable() {
        let color = Color::Rgba(255, 0, 128, 64);
        let text = super::RichText::new(crate::TextSpan::new("selectable"))
            .selectable()
            .selection_color(color);

        assert_eq!(text.selection_color, Some(color));
    }

    #[test]
    fn explicit_overflow_override_is_independent_of_builder_order() {
        let before_style = super::RichText::new(crate::TextSpan::new("before"))
            .text_overflow(TextOverflow::Wrap)
            .text_style(TextStyle::new().font_size(20));
        let after_style = super::RichText::new(crate::TextSpan::new("after"))
            .text_style(TextStyle::new().font_size(20))
            .text_overflow(TextOverflow::Wrap);

        assert!(matches!(
            before_style.resolved_overflow(),
            TextOverflow::Wrap
        ));
        assert!(matches!(
            after_style.resolved_overflow(),
            TextOverflow::Wrap
        ));
    }

    // #[test]
    // fn hovering_interactive_text_claims_the_cursor_event() {
    //     let text = selectable_raw_text(LinkCallback::default());
    //
    //     assert!(text.on_event(&ElementEvent::PointerMove(
    //         Vec2d { x: 1.0, y: 5.0 },
    //         PointerSource::Mouse,
    //         0,
    //     )).is_consumed());
    // }

    #[test]
    fn moving_into_and_out_of_a_link_updates_hover_and_requests_redraw() {
        let text = selectable_raw_text(LinkCallback::default());

        let _ = text.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(
            Vec2d { x: 1.0, y: 5.0 },
            PointerButton::Primary,
        )));
        assert_eq!(
            text.hovered_link.borrow().as_deref(),
            Some("https://aimer.dev")
        );
        assert!(text.geometry().window().take_redraw_request());

        let _ = text.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(
            Vec2d { x: 50.0, y: 50.0 },
            PointerButton::Primary,
        )));
        assert!(text.hovered_link.borrow().is_none());
        assert!(text.geometry().window().take_redraw_request());
    }

    #[test]
    fn moving_within_a_link_keeps_the_link_hovered() {
        let text = selectable_raw_text(LinkCallback::default());

        for x in 1..20 {
            let _ = text.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(
                Vec2d {
                    x: x as f32,
                    y: 5.0,
                },
                PointerButton::Primary,
            )));

            assert_eq!(
                text.hovered_link.borrow().as_deref(),
                Some("https://aimer.dev")
            );
        }
    }

    #[test]
    fn select_all_shortcut_selects_the_visible_text_after_focus() {
        let text = selectable_raw_text(LinkCallback::default());
        let _ = text.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
            Vec2d { x: 1.0, y: 5.0 },
            PointerButton::Primary,
        )));

        let handled = text.on_event(&ElementEvent::KeyInput {
            key: NamedKey::Other("a".into()),
            action: KeyAction::Pressed,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        });

        assert!(handled.is_consumed());
        assert_eq!(selected(&text), Some(0..6));
    }

    #[test]
    fn a_press_outside_a_standalone_text_clears_its_selection() {
        let text = selectable_raw_text(LinkCallback::default());
        let _ = text.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
            Vec2d { x: 1.0, y: 5.0 },
            PointerButton::Primary,
        )));
        let _ = text.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(
            Vec2d { x: 1.0, y: 5.0 },
            PointerButton::Primary,
        )));
        text.session().select_all();
        assert_eq!(selected(&text), Some(0..6));

        // Broadcast to every element is how the tree reports a press nobody took.
        let _ = text.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
            Vec2d { x: 500.0, y: 900.0 },
            PointerButton::Primary,
        )));

        assert_eq!(selected(&text), None);
        assert!(!text.session().is_focused());
    }

    #[test]
    fn selecting_second_text_clears_first_selection_focus_and_capture() {
        let coordinator = Rc::new(SelectionCoordinator::default());
        let first =
            selectable_raw_text_with_coordinator(LinkCallback::default(), coordinator.clone());
        let second = selectable_raw_text_with_coordinator(LinkCallback::default(), coordinator);

        let _ = first.on_event(&ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 1.0, y: 5.0 },
            PointerSource::Mouse,
            7,
            PointerButton::Primary,
        )));
        first.session().select_all();
        assert!(first.session().is_focused());
        assert_eq!(selected(&first), Some(0..6));
        let _ = first.geometry().window().take_redraw_request();

        let second_result = second.on_event(&ElementEvent::PointerDown(PointerInfo::new(
            Vec2d { x: 1.0, y: 5.0 },
            PointerSource::Mouse,
            8,
            PointerButton::Primary,
        )));

        assert_eq!(selected(&first), None);
        assert!(!first.session().is_focused());
        assert_eq!(first.session().active_pointer(), None);
        assert!(first.geometry().window().take_redraw_request());
        assert!(second.session().is_focused());
        let second_pointer = PointerKey::new(PointerSource::Mouse, 8);
        assert_eq!(second.session().active_pointer(), Some(second_pointer));
        assert_eq!(
            second_result.capture_request(),
            aimer_widget::CaptureRequest::Capture(second_pointer)
        );
    }

    #[test]
    fn coordinator_does_not_retain_a_dropped_session() {
        let coordinator = Rc::new(SelectionCoordinator::default());
        let session = SelectionSession::new(
            WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 100), 1.0),
            coordinator.clone(),
            DEFAULT_SELECTION_COLOR,
        );
        let weak_session = Rc::downgrade(&session);
        session.claim();

        drop(session);

        assert!(weak_session.upgrade().is_none());
        assert!(coordinator.current().is_none());
    }

    #[test]
    fn dragging_a_link_selects_text_without_activating_the_link() {
        let activations = Rc::new(Cell::new(0));
        let text = selectable_raw_text(LinkCallback::from({
            let activations = activations.clone();
            move |_| activations.set(activations.get() + 1)
        }));

        let down_result = text.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
            Vec2d { x: 1.0, y: 5.0 },
            PointerButton::Primary,
        )));
        let pointer = PointerKey::new(PointerSource::Mouse, 0);
        assert_eq!(
            down_result.capture_request(),
            aimer_widget::CaptureRequest::Capture(pointer)
        );
        assert_eq!(selected(&text), Some(0..0));
        let _ = text.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(
            Vec2d { x: 19.0, y: 5.0 },
            PointerButton::Primary,
        )));
        let up_result = text.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(
            Vec2d { x: 19.0, y: 5.0 },
            PointerButton::Primary,
        )));

        assert_eq!(selected(&text), Some(0..6));
        assert_eq!(
            up_result.capture_request(),
            aimer_widget::CaptureRequest::Release(pointer)
        );
        assert_eq!(activations.get(), 0);
    }

    #[test]
    fn dragging_below_short_final_line_selects_complete_text() {
        let text = raw_text_with(
            LinkCallback::default(),
            Rc::new(SelectionCoordinator::default()),
            Rc::from("long\n}"),
            vec![
                TextHitRegion::new(0..1, Bounds::new(10.0, 20.0, 100.0, 10.0)),
                TextHitRegion::new(5..6, Bounds::new(10.0, 30.0, 10.0, 10.0)),
            ],
            Bounds::new(10.0, 20.0, 100.0, 20.0),
        );

        let _ = text.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
            Vec2d { x: 10.0, y: 25.0 },
            PointerButton::Primary,
        )));
        let _ = text.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(
            Vec2d { x: 200.0, y: 50.0 },
            PointerButton::Primary,
        )));
        let _ = text.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(
            Vec2d { x: 200.0, y: 50.0 },
            PointerButton::Primary,
        )));

        let selection = selected(&text).expect("the drag selects the whole text");
        assert_eq!(selection, 0..text.plain_text.len());
        assert_eq!(text.session().selected_text(), text.plain_text.as_ref());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn selection_highlight_starts_at_the_text_line_top() {
        use aimer_attribute::ResolvedSize;
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_cupid::draw_cmd_v2::DrawCommand;
        use aimer_widget::Drawable;
        use aimer_widget::base::BuildContext;

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let selection_color = Color::Rgba(255, 0, 128, 64);
        let text = RawRichText {
            paragraph: Paragraph::new(
                vec![ResolvedTextSpan::plain(
                    Rc::from("selected"),
                    TextStyle::new().font_size(24),
                )],
                TextAlign::TopLeft,
                TextOverflow::Wrap,
            ),
            plain_text: Rc::from("selected"),
            on_link: LinkCallback::default(),
            link_hover_color: None,
            selectable: true,
            selection_color,
            binding: standalone_binding(
                &context.window,
                Rc::new(SelectionCoordinator::default()),
                Rc::from("selected"),
            ),
            link_regions: RefCell::new(Vec::new()),
            pressed_link: RefCell::new(None),
            hovered_link: RefCell::new(None),
            hover_cursor: crate::selection::cursor::HoverCursor::new(),
            touch_hold: crate::selection::touch_hold::TouchHoldGate::new(),
            focus_node: FocusNode::new(),
        };
        text.session().select_all();
        let layout = text.paragraph.prepare(&context);
        // The highlight covers the whole line box, which starts half the
        // leading above the ascent line.
        let fragment = &layout.fragments[0];
        let leading = (layout.line_heights[0] - fragment.height).max(0.0);
        let expected_top = fragment.baseline - fragment.ascent - leading * 0.5;

        text.update(&context);

        let (selection_top, rendered_color) = recorded_v2(&text, &context)
            .as_slice()
            .iter()
            .find_map(|command| match command {
                DrawCommand::FillRect { rect, color, .. } => Some((rect.y, *color)),
                _ => None,
            })
            .unwrap();
        let expected_color: aimer_cupid::utilities::Color = selection_color.into();
        assert!(
            (selection_top - expected_top).abs() < 1e-3,
            "selection starts at {selection_top}, expected {expected_top}"
        );
        assert_eq!(rendered_color.to_array(), expected_color.to_array());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn selection_highlight_connects_across_adjacent_spans() {
        use aimer_attribute::ResolvedSize;
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_cupid::draw_cmd_v2::DrawCommand;
        use aimer_widget::Drawable;
        use aimer_widget::base::BuildContext;

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let text = RawRichText {
            paragraph: Paragraph::new(
                vec![
                    ResolvedTextSpan::plain(Rc::from("normal "), TextStyle::new().font_size(20)),
                    ResolvedTextSpan::plain(
                        Rc::from("italic"),
                        TextStyle::new()
                            .font_size(20)
                            .font_style(aimer_style::FontStyle::Italic),
                    ),
                ],
                TextAlign::TopLeft,
                TextOverflow::Wrap,
            ),
            plain_text: Rc::from("normal italic"),
            on_link: LinkCallback::default(),
            link_hover_color: None,
            selectable: true,
            selection_color: DEFAULT_SELECTION_COLOR,
            binding: standalone_binding(
                &context.window,
                Rc::new(SelectionCoordinator::default()),
                Rc::from("normal italic"),
            ),
            link_regions: RefCell::new(Vec::new()),
            pressed_link: RefCell::new(None),
            hovered_link: RefCell::new(None),
            hover_cursor: crate::selection::cursor::HoverCursor::new(),
            touch_hold: crate::selection::touch_hold::TouchHoldGate::new(),
            focus_node: FocusNode::new(),
        };
        text.session().select_all();

        text.update(&context);

        let highlight_count = recorded_v2(&text, &context)
            .as_slice()
            .iter()
            .filter(|command| matches!(command, DrawCommand::FillRect { .. }))
            .count();
        assert_eq!(highlight_count, 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn selection_highlights_touch_between_wrapped_lines() {
        use aimer_attribute::{BoxConstraint, ResolvedSize};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_cupid::draw_cmd_v2::DrawCommand;
        use aimer_widget::Drawable;
        use aimer_widget::base::BuildContext;

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 70.0,
                height: 200.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(70, 200), 1.0),
            runtime.handle().clone(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 70.0,
            max_height: 200.0,
        };
        let text = RawRichText {
            paragraph: Paragraph::new(
                vec![ResolvedTextSpan::plain(
                    Rc::from("first second third"),
                    TextStyle::new().font_size(24),
                )],
                TextAlign::TopLeft,
                TextOverflow::Wrap,
            ),
            plain_text: Rc::from("first second third"),
            on_link: LinkCallback::default(),
            link_hover_color: None,
            selectable: true,
            selection_color: DEFAULT_SELECTION_COLOR,
            binding: standalone_binding(
                &context.window,
                Rc::new(SelectionCoordinator::default()),
                Rc::from("first second third"),
            ),
            link_regions: RefCell::new(Vec::new()),
            pressed_link: RefCell::new(None),
            hovered_link: RefCell::new(None),
            hover_cursor: crate::selection::cursor::HoverCursor::new(),
            touch_hold: crate::selection::touch_hold::TouchHoldGate::new(),
            focus_node: FocusNode::new(),
        };
        text.session().select_all();

        text.update(&context);

        let highlights = recorded_v2(&text, &context)
            .as_slice()
            .iter()
            .filter_map(|command| match command {
                DrawCommand::FillRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(highlights.len() > 1);
        for adjacent in highlights.windows(2) {
            assert!((adjacent[0].y + adjacent[0].height - adjacent[1].y).abs() < 0.01);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn explicit_newlines_have_stable_hit_targets_and_connected_highlights() {
        use aimer_attribute::{BoxConstraint, ResolvedSize};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_cupid::draw_cmd_v2::DrawCommand;
        use aimer_widget::Drawable;
        use aimer_widget::base::BuildContext;

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 200.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 200), 1.0),
            runtime.handle().clone(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 200.0,
            max_height: 200.0,
        };
        let text = RawRichText {
            paragraph: Paragraph::new(
                vec![
                    ResolvedTextSpan::plain(Rc::from("first\n"), TextStyle::new().font_size(20)),
                    ResolvedTextSpan::plain(Rc::from("\n"), TextStyle::new().font_size(20)),
                    ResolvedTextSpan::plain(Rc::from("third"), TextStyle::new().font_size(20)),
                ],
                TextAlign::TopLeft,
                TextOverflow::Wrap,
            ),
            plain_text: Rc::from("first\n\nthird"),
            on_link: LinkCallback::default(),
            link_hover_color: None,
            selectable: true,
            selection_color: DEFAULT_SELECTION_COLOR,
            binding: standalone_binding(
                &context.window,
                Rc::new(SelectionCoordinator::default()),
                Rc::from("first\n\nthird"),
            ),
            link_regions: RefCell::new(Vec::new()),
            pressed_link: RefCell::new(None),
            hovered_link: RefCell::new(None),
            hover_cursor: crate::selection::cursor::HoverCursor::new(),
            touch_hold: crate::selection::touch_hold::TouchHoldGate::new(),
            focus_node: FocusNode::new(),
        };
        text.session().select_all();

        let layout = text.paragraph.prepare(&context);
        assert_eq!(layout.line_breaks.len(), 2);
        assert_eq!(layout.line_breaks[0].source_range, 5..6);
        assert_eq!(layout.line_breaks[1].source_range, 6..7);
        assert_eq!(
            layout.line_breaks[0].x + layout.line_breaks[0].hit_width,
            layout.size.width
        );
        assert_eq!(layout.line_breaks[1].hit_width, layout.size.width);
        assert_eq!(layout.line_breaks[0].selection_width, 1.0);
        assert_eq!(layout.line_breaks[1].selection_width, 1.0);
        assert!(layout.line_breaks[1].height > 0.0);
        assert!(
            (layout.line_breaks[0].y + layout.line_breaks[0].height - layout.line_breaks[1].y)
                .abs()
                < 0.01
        );

        text.update(&context);

        let geometry = text.geometry();
        let regions = geometry.regions.borrow();
        assert!(regions.iter().any(|region| region.source_range == (5..5)));
        assert!(regions.iter().any(|region| region.source_range == (6..6)));
        assert_eq!(
            crate::selection::text_offset_at(
                &regions,
                199.0,
                layout.line_breaks[0].y + layout.line_breaks[0].height / 2.0,
            ),
            Some(5),
        );
        assert_eq!(
            crate::selection::text_offset_at(
                &regions,
                199.0,
                layout.line_breaks[1].y + layout.line_breaks[1].height / 2.0,
            ),
            Some(6),
        );
        let highlights = recorded_v2(&text, &context)
            .as_slice()
            .iter()
            .filter_map(|command| match command {
                DrawCommand::FillRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(highlights.len(), 3);
        assert_eq!(highlights[0].width, layout.fragments[0].width + 1.0);
        assert_eq!(highlights[1].width, 1.0);
        for adjacent in highlights.windows(2) {
            assert!((adjacent[0].y + adjacent[0].height - adjacent[1].y).abs() < 0.01);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn italic_span_enables_synthetic_italic_for_its_draw() {
        use aimer_attribute::ResolvedSize;
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_cupid::draw_cmd_v2::DrawCommand;
        use aimer_widget::Drawable;
        use aimer_widget::base::BuildContext;

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let link_target = TextSource::from("https://aimer.dev");
        let text = RawRichText {
            paragraph: Paragraph::new(
                vec![
                    ResolvedTextSpan::plain(
                        Rc::from("italic"),
                        TextStyle::new()
                            .font_size(20)
                            .font_style(aimer_style::FontStyle::Italic),
                    ),
                    ResolvedTextSpan {
                        text: TextSource::from("link"),
                        style: TextStyle::default(),
                        link: Some(link_target.clone()),
                    },
                    ResolvedTextSpan::plain(Rc::from("tail"), TextStyle::default()),
                ],
                TextAlign::TopLeft,
                TextOverflow::Clip,
            ),
            plain_text: Rc::from("italiclinktail"),
            on_link: LinkCallback::default(),
            link_hover_color: None,
            selectable: false,
            selection_color: DEFAULT_SELECTION_COLOR,
            binding: standalone_binding(
                &context.window,
                Rc::new(SelectionCoordinator::default()),
                Rc::from("italic"),
            ),
            link_regions: RefCell::new(Vec::new()),
            pressed_link: RefCell::new(None),
            hovered_link: RefCell::new(None),
            hover_cursor: crate::selection::cursor::HoverCursor::new(),
            touch_hold: crate::selection::touch_hold::TouchHoldGate::new(),
            focus_node: FocusNode::new(),
        };

        text.update(&context);

        let commands = recorded_v2(&text, &context);
        let draw_index = commands
            .iter()
            .position(|command| matches!(command, DrawCommand::DrawText { .. }))
            .unwrap();
        assert!(matches!(
            commands[draw_index - 1],
            DrawCommand::SetItalic { italic: true }
        ));
        assert!(matches!(
            commands[draw_index + 1],
            DrawCommand::SetItalic { italic: false }
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn backgrounds_draw_before_text_without_changing_size_or_link_regions() {
        use std::cell::RefCell;

        use aimer_attribute::{ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_cupid::draw_cmd_v2::DrawCommand;
        use aimer_style::{TextAlign, TextOverflow};
        use aimer_widget::Drawable;
        use aimer_widget::base::{BuildContext, WindowHandle};

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Vec2d { x: 1.0, y: 5.0 },
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            runtime.handle().clone(),
        );
        let highlighted_span = ResolvedTextSpan {
            text: TextSource::from("linked"),
            style: TextStyle::new().background_color(aimer_widget::base::Color::RED),
            link: Some(TextSource::from("https://aimer.dev")),
        };
        let highlighted = RawRichText {
            paragraph: Paragraph::new(
                vec![highlighted_span.clone()],
                TextAlign::TopLeft,
                TextOverflow::Clip,
            ),
            plain_text: Rc::from("linked"),
            on_link: LinkCallback::default(),
            link_hover_color: None,
            selectable: false,
            selection_color: DEFAULT_SELECTION_COLOR,
            binding: standalone_binding(
                &context.window,
                Rc::new(SelectionCoordinator::default()),
                Rc::from("linked"),
            ),
            link_regions: RefCell::new(Vec::new()),
            pressed_link: RefCell::new(None),
            hovered_link: RefCell::new(None),
            hover_cursor: crate::selection::cursor::HoverCursor::new(),
            touch_hold: crate::selection::touch_hold::TouchHoldGate::new(),
            focus_node: FocusNode::new(),
        };
        let plain = RawRichText {
            paragraph: Paragraph::new(
                vec![ResolvedTextSpan {
                    style: TextStyle {
                        background_color: None,
                        ..highlighted_span.style
                    },
                    ..highlighted_span
                }],
                TextAlign::TopLeft,
                TextOverflow::Clip,
            ),
            plain_text: Rc::from("linked"),
            on_link: LinkCallback::default(),
            link_hover_color: None,
            selectable: false,
            selection_color: DEFAULT_SELECTION_COLOR,
            binding: standalone_binding(
                &context.window,
                Rc::new(SelectionCoordinator::default()),
                Rc::from("linked"),
            ),
            link_regions: RefCell::new(Vec::new()),
            pressed_link: RefCell::new(None),
            hovered_link: RefCell::new(None),
            hover_cursor: crate::selection::cursor::HoverCursor::new(),
            touch_hold: crate::selection::touch_hold::TouchHoldGate::new(),
            focus_node: FocusNode::new(),
        };

        assert_eq!(
            highlighted.paragraph.prepare(&context).size,
            plain.paragraph.prepare(&context).size
        );
        highlighted.update(&context);
        assert_eq!(
            highlighted.hovered_link.borrow().as_deref(),
            Some("https://aimer.dev")
        );

        let commands = recorded_v2(&highlighted, &context);
        let background_index = commands
            .iter()
            .position(|command| matches!(command, DrawCommand::FillRect { .. }))
            .unwrap();
        let text_index = commands
            .iter()
            .position(|command| matches!(command, DrawCommand::DrawText { .. }))
            .unwrap();
        assert!(background_index < text_index);
        assert_eq!(highlighted.link_regions.borrow().len(), 1);
    }

    #[test]
    fn wrapping_uses_one_cursor_across_span_boundaries() {
        let style = TextStyle::new().font_size(10);
        let spans = vec![
            ResolvedTextSpan::plain(Rc::from("abc"), style),
            ResolvedTextSpan::plain(Rc::from("def"), style),
        ];

        let layout =
            layout_resolved_spans(&spans, 20.0, |text, _| text.chars().count() as f32 * 5.0);

        assert_eq!(layout.line_count, 2);
        assert_eq!(layout.fragments[0].line, 0);
        assert_eq!(layout.fragments[1].line, 0);
        assert_eq!(layout.fragments[1].x, 15.0);
        assert_eq!(layout.fragments[2].line, 1);
        assert_eq!(layout.fragments[2].x, 0.0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn wrapping_uses_parent_width_when_constraint_is_unbounded() {
        use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_style::{TextAlign, TextOverflow};
        use aimer_widget::base::{BuildContext, WindowHandle};

        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 20.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(20, 100), 1.0),
            runtime.handle().clone(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: f32::MAX,
            max_height: f32::MAX,
        };
        let rich_text = RawRichText {
            paragraph: Paragraph::new(
                vec![ResolvedTextSpan::plain(
                    Rc::from("abcdef"),
                    TextStyle::new().font_size(10),
                )],
                TextAlign::TopLeft,
                TextOverflow::Wrap,
            ),
            plain_text: Rc::from("abcdef"),
            on_link: LinkCallback::default(),
            link_hover_color: None,
            selectable: false,
            selection_color: DEFAULT_SELECTION_COLOR,
            binding: standalone_binding(
                &context.window,
                Rc::new(SelectionCoordinator::default()),
                Rc::from("abcdef"),
            ),
            link_regions: RefCell::new(Vec::new()),
            pressed_link: RefCell::new(None),
            hovered_link: RefCell::new(None),
            hover_cursor: crate::selection::cursor::HoverCursor::new(),
            touch_hold: crate::selection::touch_hold::TouchHoldGate::new(),
            focus_node: FocusNode::new(),
        };

        assert_eq!(rich_text.paragraph.available_width(&context), 20.0);
        let first_layout = rich_text.paragraph.prepare(&context);
        let cached_layout = rich_text.paragraph.prepare(&context);
        assert_eq!(first_layout.size.width, 20.0);
        assert!(Rc::ptr_eq(&first_layout, &cached_layout));

        context.parent_size.width = 40.0;
        let resized_layout = rich_text.paragraph.prepare(&context);
        assert_eq!(resized_layout.size.width, 40.0);
        assert!(!Rc::ptr_eq(&first_layout, &resized_layout));
    }

    /// A text that owns its session owns the keyboard that goes with it, which is
    /// what tells it about a press it never sees — one that landed on another
    /// widget entirely.
    #[test]
    fn a_standalone_selectable_text_holds_the_focus_while_it_holds_a_selection() {
        let text = selectable_raw_text(LinkCallback::default());
        let session = text.session();

        assert!(!session.is_focused(), "nothing is selected yet");

        let _ = text.on_event(&ElementEvent::PointerDown(mouse_press(2.0, 5.0)));
        let _ = text.on_event(&ElementEvent::PointerMove(mouse_press(18.0, 5.0)));

        assert!(selected(&text).is_some());
        assert!(session.is_focused());
    }

    /// The keyboard such a text holds is held for it by the standard focus
    /// attachment, which is asked afresh every time targets are gathered — a
    /// selection begins and ends under the finger, with nothing rebuilding the
    /// paragraph in between.
    #[test]
    fn a_standalone_selectable_text_is_offered_as_a_target_only_while_it_has_a_selection() {
        let text = selectable_raw_text(LinkCallback::default());
        let session = text.session();
        let target = text.focus_target();

        assert!(target.focus_node().is_none(), "nothing is selected yet");

        session.select_all();

        assert!(target.focus_node().is_some());
    }

    #[test]
    fn a_standalone_selectable_text_drops_its_selection_when_it_loses_the_focus() {
        let text = selectable_raw_text(LinkCallback::default());
        let _ = text.on_event(&ElementEvent::PointerDown(mouse_press(2.0, 5.0)));
        let _ = text.on_event(&ElementEvent::PointerMove(mouse_press(18.0, 5.0)));
        assert!(selected(&text).is_some());

        let _ = text.on_event(&ElementEvent::FocusLost);

        assert_eq!(selected(&text), None);
        assert!(!text.session().is_focused());
    }

    /// Inside a region the selection, and therefore the keyboard, belongs to the
    /// region: a participant that took the focus for itself would keep the region
    /// from ever getting it.
    #[test]
    fn a_participant_of_a_region_leaves_the_focus_to_the_region() {
        let window = WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 100), 1.0);
        let session = SelectionSession::new(
            window.clone(),
            Rc::new(SelectionCoordinator::default()),
            DEFAULT_SELECTION_COLOR,
        );
        let text = selectable_raw_text(LinkCallback::default());
        let plain = text.plain_text.clone();
        let geometry = text.geometry();
        let slot = session.register(Rc::clone(&plain), Rc::downgrade(&geometry) as _);
        slot.stamp();
        session.begin_frame();
        *text.binding.borrow_mut() = SelectionBinding {
            geometry,
            session,
            slot,
            owns_session: false,
        };

        let _ = text.on_event(&ElementEvent::PointerDown(mouse_press(2.0, 5.0)));
        let _ = text.on_event(&ElementEvent::PointerMove(mouse_press(18.0, 5.0)));

        assert!(selected(&text).is_some());
        assert!(
            text.focus_target().focus_node().is_none(),
            "a participant is not wrapped in a focus attachment at all"
        );
    }

    // Keep the region cases included here without pushing this source past the
    // repository's 2,000-line limit.
    mod region {
        include!("rich_text/tests/region.rs");
    }
}
