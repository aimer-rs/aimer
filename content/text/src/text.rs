pub mod raw_text;
pub mod selectable_text;

use std::cell::Cell;

use aimer_style::{LineHeight, TextAlign, TextOverflow, TextShadow, TextStyle, TextTransform};
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, Element, LayoutCache, Widget};

use crate::selection::selectable::SelectionScope;
use crate::text::raw_text::RawTextWidget;
use crate::text::selectable_text::RawSelectableText;
use crate::text_source::TextSource;

/// The highlight color a selectable `Text` falls back to when it somehow sits
/// outside a region; regions always supply their own.
const DEFAULT_SELECTION_COLOR: aimer_widget::base::Color =
    aimer_widget::base::Color::Rgba(51, 153, 255, 96);

/// Displays a single run of styled text.
///
/// Text uses [`TextStyle::default`] and [`TextAlign::default`] unless replaced.
/// Overflow behavior comes from the active style; use [`Text::wrapped`] or
/// [`Text::ellipsis`] for the common modes. Unlike [`crate::RichText`], this
/// widget does not provide spans, links, or selection. Run-level properties
/// such as transformation, spacing, decoration, and glyph shadow belong to
/// [`TextStyle`]; the forwarding builders below only update that canonical
/// style. Line height and first-line indentation belong to the paragraph and
/// affect layout, wrapping, hit geometry, and painting together.
///
/// # Example
///
/// ```
/// use aimer_style::{TextAlign, TextStyle};
/// use aimer_text::Text;
///
/// let title = Text::new("Aimer")
///                 .text_align(TextAlign::MidCenter)
///                 .text_style(TextStyle::default())
///                 .wrapped();
/// ```
#[allow(dead_code)]
pub struct Text {
    text: TextSource,
    text_align: TextAlign,
    text_style: TextStyle,
    line_height: LineHeight,
    text_indent: f32,
}

impl Text {
    /// Creates text containing `text` with default style and alignment.
    ///
    /// Passing a `&'static str` literal stores it directly with no
    /// allocation; passing a `String` or `Rc<str>` behaves as before. See
    /// [`TextSource`] for details.
    #[inline]
    pub fn new(text: impl Into<TextSource>) -> Self {
        Self {
            text: text.into(),
            text_align: TextAlign::default(),
            text_style: TextStyle::default(),
            line_height: LineHeight::default(),
            text_indent: 0.0,
        }
    }

    /// Replaces the displayed string while preserving style and alignment.
    #[inline]
    pub fn text(mut self, text: impl Into<TextSource>) -> Self {
        self.text = text.into();
        self
    }

    /// Sets how laid-out text is aligned within its available width.
    #[inline]
    pub fn text_align(mut self, text_align: TextAlign) -> Self {
        self.text_align = text_align;
        self
    }

    /// Replaces the complete style used for shaping, layout, and painting.
    ///
    /// This includes font attributes, color, decoration, and overflow behavior.
    #[inline]
    pub fn text_style(mut self, text_style: TextStyle) -> Self {
        self.text_style = text_style;
        self
    }

    /// Sets the Unicode transformation applied before shaping.
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

    /// Adds one glyph shadow behind the text.
    #[inline]
    pub fn text_shadow(mut self, text_shadow: TextShadow) -> Self {
        self.text_style.text_shadow = Some(text_shadow);
        self
    }

    /// Removes the glyph shadow from the text.
    #[inline]
    pub fn without_text_shadow(mut self) -> Self {
        self.text_style.text_shadow = None;
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
    /// Sets overflow behavior on the current style.
    ///
    /// Prefer configuring [`TextStyle::text_overflow`] before passing the style
    /// to [`Text::text_style`].
    #[deprecated(note = "set TextStyle::text_overflow and pass it to Text::text_style")]
    #[inline]
    pub fn text_overflow(mut self, text_overflow: TextOverflow) -> Self {
        self.text_style.text_overflow = text_overflow;
        self
    }

    /// Configures text to wrap onto additional lines when width is constrained.
    #[allow(deprecated)]
    #[inline]
    pub fn wrapped(self) -> Self {
        self.text_overflow(TextOverflow::Wrap)
    }

    /// Configures overflowing text to be truncated with an ellipsis.
    #[allow(deprecated)]
    #[inline]
    pub fn ellipsis(self) -> Self {
        self.text_overflow(TextOverflow::Ellipsis)
    }
}

impl Widget for Text {
    /// Emits the paragraph-backed selectable element inside a
    /// [`SelectionArea`](crate::SelectionArea) and the plain fast path
    /// everywhere else.
    ///
    /// The lookup is a single `TypeId` probe, so a tree without a region pays
    /// nothing for selection.
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        if ctx.get_state::<SelectionScope>().is_some() {
            return RawSelectableText::new(
                ctx,
                self.text,
                self.text_style,
                self.text_align,
                self.line_height,
                self.text_indent,
                DEFAULT_SELECTION_COLOR,
            )
            .boxed();
        }
        RawTextWidget {
            text: self.text,
            text_style: self.text_style,
            text_align: self.text_align,
            line_height: self.line_height,
            text_indent: self.text_indent,
            cache: LayoutCache::new(),
            _typeface: Cell::new(None),
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_attribute::{ResolvedSize, Vec2d};
    use aimer_canvas::{FrameCanvas, InnerCanvas};
    use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderPaintSource, RenderTree};
    use aimer_style::{LineHeight, TextAlign, TextStyle, TextTransform};
    use aimer_widget::base::{BuildContext, WindowHandle};
    use aimer_widget::{Drawable, LayoutCache, LayoutElement, Rebuildable, Widget};

    use super::{RawTextWidget, Text};
    use crate::selection::selectable::{SelectionCoordinator, SelectionScope};
    use crate::selection::session::SelectionSession;
    use crate::text_source::TextSource;

    /// The debug name of the non-selectable fast path.
    const RAW_TEXT_NAME: &str = "RawTextWidget";

    #[test]
    fn text_builder_keeps_paragraph_values_with_the_widget() {
        let text = Text::new("Aimer")
            .text_style(
                TextStyle::new()
                    .text_transform(TextTransform::Uppercase)
                    .letter_spacing(0.5),
            )
            .line_height(LineHeight::Px(24.0))
            .text_indent(-8.0);

        assert_eq!(text.text_style.text_transform, TextTransform::Uppercase);
        assert_eq!(text.text_style.letter_spacing, 0.5);
        assert_eq!(text.line_height, LineHeight::Px(24.0));
        assert_eq!(text.text_indent, -8.0);
    }

    fn context<'a>(
        canvas: FrameCanvas<'a>,
        runtime: &'a tokio::runtime::Runtime,
    ) -> BuildContext<'a> {
        context_with_scale(canvas, runtime, 1.0)
    }

    fn context_with_scale<'a>(
        canvas: FrameCanvas<'a>,
        runtime: &'a tokio::runtime::Runtime,
        scale: f32,
    ) -> BuildContext<'a> {
        BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0 * scale,
                height: 100.0 * scale,
            },
            scale,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(
                winit::dpi::PhysicalSize::new((200.0 * scale) as u32, (100.0 * scale) as u32),
                f64::from(scale),
            ),
            runtime.handle().clone(),
        )
    }

    #[test]
    fn text_outside_a_region_stays_on_the_non_selectable_fast_path() {
        let inner = InnerCanvas::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let ctx = context(FrameCanvas::new(&inner), &runtime);

        let element = Text::new("plain").to_element(&ctx);

        assert_eq!(element.debug_name(), RAW_TEXT_NAME);
    }

    #[test]
    fn plain_text_records_a_local_v2_run_and_repaints_without_dirtifying_its_sibling() {
        let inner = InnerCanvas::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let ctx = context_with_scale(FrameCanvas::new(&inner), &runtime, 2.0);
        let scale = ctx.scale;
        let raw_text = |text| RawTextWidget {
            text: TextSource::Static(text),
            text_style: TextStyle::default(),
            text_align: TextAlign::TopLeft,
            line_height: LineHeight::Normal,
            text_indent: 0.0,
            cache: LayoutCache::new(),
            _typeface: Cell::new(None),
        };
        let original = raw_text("before");
        let sibling = raw_text("sibling");
        let original_size = original.computed_size(&ctx);
        let sibling_size = sibling.computed_size(&ctx);
        let original_bounds = Rect::new(
            0.0,
            0.0,
            original_size.width / scale,
            original_size.height / scale,
        );
        let tree = RenderTree::new();
        let text_node = tree.add_root(original_bounds).unwrap();
        let sibling_node = tree
            .add_root(Rect::new(
                original_bounds.width + 20.0,
                0.0,
                sibling_size.width / scale,
                sibling_size.height / scale,
            ))
            .unwrap();

        let local_context = tree.context(text_node).unwrap();
        assert!(original.can_paint_local_v2(&ctx));
        ctx.with_local_v2_paint_context(local_context, |ctx| original.paint_local_v2(ctx));
        tree.set_paint_source(text_node, RenderPaintSource::LocalV2)
            .unwrap();
        let sibling_context = tree.context(sibling_node).unwrap();
        assert!(sibling.can_paint_local_v2(&ctx));
        ctx.with_local_v2_paint_context(sibling_context, |ctx| sibling.paint_local_v2(ctx));
        tree.set_paint_source(sibling_node, RenderPaintSource::LocalV2)
            .unwrap();

        let first_recording = tree.draw_list_snapshot(text_node).unwrap();
        assert_eq!(first_recording.revision, 1);
        assert_eq!(
            tree.paint_source(text_node).unwrap(),
            RenderPaintSource::LocalV2
        );
        assert!(matches!(
            first_recording.commands.as_ref(),
            [DrawCommand::DrawText { text, font_size, .. }]
                if text.as_ref() == "before" && *font_size == 13.0
        ));

        tree.take_damage();
        tree.invalidate_paint(text_node).unwrap();
        let updated = raw_text("after");
        let updated_context = tree.context(text_node).unwrap();
        assert!(updated.can_paint_local_v2(&ctx));
        ctx.with_local_v2_paint_context(updated_context, |ctx| updated.paint_local_v2(ctx));

        let updated_recording = tree.draw_list_snapshot(text_node).unwrap();
        assert_eq!(updated_recording.revision, 2);
        assert!(matches!(
            updated_recording.commands.as_ref(),
            [DrawCommand::DrawText { text, .. }] if text.as_ref() == "after"
        ));
        assert_eq!(tree.draw_list_revision(sibling_node).unwrap(), 1);
        assert_eq!(tree.take_damage(), vec![original_bounds]);
    }

    #[test]
    fn text_inside_a_region_becomes_selectable() {
        let inner = InnerCanvas::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let ctx = context(FrameCanvas::new(&inner), &runtime);
        let session = SelectionSession::new(
            ctx.window.clone(),
            Rc::new(SelectionCoordinator::default()),
            super::DEFAULT_SELECTION_COLOR,
        );

        let element = ctx.with_state(SelectionScope(Rc::clone(&session)), |ctx| {
            Text::new("selectable").to_element(ctx)
        });

        assert_eq!(element.debug_name(), "SelectableText");
        session.select_all();
        assert_eq!(session.selected_text(), "selectable");
    }

    #[test]
    fn toggling_a_region_around_a_text_swaps_the_element_without_panicking() {
        let inner = InnerCanvas::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let ctx = context(FrameCanvas::new(&inner), &runtime);
        let session = SelectionSession::new(
            ctx.window.clone(),
            Rc::new(SelectionCoordinator::default()),
            super::DEFAULT_SELECTION_COLOR,
        );

        let inside = ctx.with_state(SelectionScope(Rc::clone(&session)), |ctx| {
            Text::new("toggled").to_element(ctx)
        });
        let outside = Text::new("toggled").to_element(&ctx);
        outside.adopt_runtime_state_from(inside.as_ref());

        assert_eq!(outside.debug_name(), RAW_TEXT_NAME);
    }
}
